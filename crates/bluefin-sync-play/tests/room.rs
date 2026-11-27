use bluefin_sync_play::{
    Action, ClockExchange, Command, CommandStatus, Id, MediaLoad, Membership, QueueChange,
    QueueItem, Ready, Room, RoomClock,
};

fn id(tag: u8) -> Id {
    Id::new([tag; 16]).unwrap()
}

fn membership(epoch: u64) -> Membership {
    Membership {
        epoch,
        group: id(1),
    }
}

fn item(occurrence: u8) -> QueueItem {
    QueueItem {
        item: id(2),
        playlist_item: id(occurrence),
        start_ticks: 0,
    }
}

fn clock() -> RoomClock {
    RoomClock::measure(ClockExchange {
        local_sent: 1000,
        server_received: 50_010,
        server_sent: 50_014,
        local_received: 1024,
    })
    .unwrap()
}

fn command(action: Action) -> Command {
    Command {
        group: id(1),
        playlist_item: id(3),
        emitted_at: 500_240_000,
        execute_at_ms: 50_100,
        action,
    }
}

fn load(change: QueueChange) -> MediaLoad {
    let QueueChange::Load(ticket) = change else {
        panic!("expected a new media load")
    };
    ticket
}

fn room_with_queue() -> (Room, MediaLoad) {
    let mut room = Room::default();
    assert!(room.joined(membership(1)));
    assert!(room.update_clock(membership(1), clock()));
    let ticket = load(room.update_queue(membership(1), 500_000_000, Some(item(3))));
    (room, ticket)
}

#[test]
fn rejects_aged_backward_and_impossible_clock_samples() {
    let sample = ClockExchange {
        local_sent: 1000,
        server_received: 50_010,
        server_sent: 50_014,
        local_received: 1024,
    };
    let clock = RoomClock::measure(sample).unwrap();
    assert_eq!(clock.uncertainty_ms(), 11);
    assert_eq!(clock.server_time(1024), Some(50_024));
    assert_eq!(clock.server_time(31_025), None);
    assert_eq!(clock.server_time(1023), None);
    assert!(
        RoomClock::measure(ClockExchange {
            local_received: 999,
            ..sample
        })
        .is_none()
    );
    assert!(
        RoomClock::measure(ClockExchange {
            server_sent: 50_040,
            ..sample
        })
        .is_none()
    );
    assert!(
        RoomClock::measure(ClockExchange {
            server_sent: 50_009,
            ..sample
        })
        .is_none()
    );
    assert!(
        RoomClock::measure(ClockExchange {
            local_received: 3_001,
            ..sample
        })
        .is_none()
    );
}

#[test]
fn resume_waits_for_own_media_and_adds_elapsed_group_time() {
    let (mut room, ticket) = room_with_queue();
    let resume = command(Action::Resume {
        position_ticks: 20_000_000,
    });
    assert_eq!(
        room.schedule(membership(1), resume, 1024),
        CommandStatus::Scheduled
    );
    assert_eq!(room.take_due(membership(1), 1110), None);
    assert!(room.item_ready(ticket));
    assert_eq!(room.take_due(membership(1), 1099), None);
    assert_eq!(
        room.take_due(membership(1), 1110),
        Some(Action::Resume {
            position_ticks: 20_100_000
        })
    );
    assert_eq!(room.take_due(membership(1), 1111), None);
    assert_eq!(
        room.schedule(membership(1), resume, 1111),
        CommandStatus::Stale
    );
}

#[test]
fn newer_pause_replaces_pending_resume_and_ignores_foreign_group() {
    let (mut room, ticket) = room_with_queue();
    assert!(room.item_ready(ticket));
    let resume = command(Action::Resume {
        position_ticks: 20_000_000,
    });
    let pause = Command {
        emitted_at: resume.emitted_at + 1,
        ..command(Action::Pause {
            position_ticks: 22_000_000,
        })
    };
    assert_eq!(
        room.schedule(membership(1), resume, 1024),
        CommandStatus::Scheduled
    );
    assert_eq!(
        room.schedule(
            membership(1),
            Command {
                group: id(4),
                ..pause
            },
            1024
        ),
        CommandStatus::Stale
    );
    assert_eq!(
        room.schedule(membership(1), pause, 1024),
        CommandStatus::Scheduled
    );
    assert_eq!(
        room.schedule(membership(1), resume, 1024),
        CommandStatus::Stale
    );
    assert_eq!(room.take_due(membership(1), 1200), Some(pause.action));
}

#[test]
fn reconnect_and_new_playlist_occurrence_fence_stale_callbacks() {
    let (mut room, ticket) = room_with_queue();
    assert!(room.item_ready(ticket));
    let other = load(room.update_queue(membership(1), 500_100_000, Some(item(4))));
    assert_eq!(
        room.update_queue(membership(1), 500_000_000, Some(item(3))),
        QueueChange::Unchanged
    );
    assert_eq!(room.report(ticket, 1024, 60, false), None);
    assert!(room.item_ready(other));
    assert_eq!(
        room.report(other, 1024, 60, false),
        Some(Ready {
            playlist_item: id(4),
            position_ticks: 60,
            when_ms: 50_024,
            is_playing: false
        })
    );
    room.retire(membership(1));
    assert!(!room.joined(membership(1)));
    assert!(room.joined(membership(2)));
    room.retire(membership(1));
    assert!(room.update_clock(membership(2), clock()));
    let reconnected = load(room.update_queue(membership(2), 500_100_001, Some(item(4))));
    assert!(!room.item_ready(other));
    assert_eq!(room.report(reconnected, 1024, 60, false), None);
}

#[test]
fn refuses_outside_clock_window_and_stopped_queue_cancels_pending_command() {
    let (mut room, ticket) = room_with_queue();
    assert!(room.item_ready(ticket));
    let valid = command(Action::Seek {
        position_ticks: 40_000_000,
    });
    let future = Command {
        execute_at_ms: 65_050,
        ..valid
    };
    assert_eq!(
        room.schedule(membership(1), future, 1024),
        CommandStatus::OutsideClockWindow
    );
    assert_eq!(
        room.schedule(membership(1), valid, 1024),
        CommandStatus::Scheduled
    );
    assert_eq!(room.take_due(membership(1), 31_025), None);
    assert_eq!(
        room.update_queue(membership(1), 500_250_000, None),
        QueueChange::Clear
    );
    assert_eq!(room.take_due(membership(1), 1100), None);
}

#[test]
fn same_item_queue_refresh_preserves_current_decoder_and_position_grant() {
    let (mut room, ticket) = room_with_queue();
    assert!(room.item_ready(ticket));
    let announcement = QueueItem {
        start_ticks: 80_000_000,
        ..item(3)
    };
    assert_eq!(
        room.update_queue(membership(1), 500_010_000, Some(announcement)),
        QueueChange::Unchanged
    );
    assert!(room.report(ticket, 1024, 60, false).is_some());
    assert!(Id::new([0; 16]).is_none());
}

#[test]
fn returning_to_a_queue_item_requires_a_fresh_authorization_callback() {
    let (mut room, first) = room_with_queue();
    let _ = room.update_queue(membership(1), 500_030_000, Some(item(4)));
    let again = load(room.update_queue(membership(1), 500_060_000, Some(item(3))));
    assert!(!room.item_ready(first));
    assert!(room.item_ready(again));
}
