//! Client-side scheduling for an authenticated SyncPlay room.
//!
//! An adapter owns the authenticated connection, media authorization, decoder,
//! and clock reads. The schedule never carries media credentials or grants
//! access. Use a new local connection epoch after reconnect or account change.
#![no_std]
#![forbid(unsafe_code)]

/// Signed wall times and monotonic times use milliseconds.
/// Media positions use the server's 100-nanosecond ticks.
const TICKS_PER_MILLISECOND: u64 = 10_000;
const MAX_NETWORK_DELAY: u64 = 2_000;
const MAX_CLOCK_AGE: u64 = 30_000;
const MAX_FUTURE: u64 = 10_000;
const MAX_LATE: u64 = 30_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClockExchange {
    pub local_sent: u64,
    pub server_received: i64,
    pub server_sent: i64,
    pub local_received: u64,
}

/// Server UTC corresponding to the midpoint of the local monotonic interval.
/// Half the network round trip bounds unknown path asymmetry in this sample.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RoomClock {
    local_midpoint: u64,
    server_midpoint: i64,
    sampled_at: u64,
    network_delay: u64,
}

impl RoomClock {
    pub fn measure(sample: ClockExchange) -> Option<Self> {
        let elapsed = sample.local_received.checked_sub(sample.local_sent)?;
        let work = u64::try_from(sample.server_sent.checked_sub(sample.server_received)?).ok()?;
        let network_delay = elapsed.checked_sub(work)?;
        if elapsed > MAX_NETWORK_DELAY {
            return None;
        }
        Some(Self {
            local_midpoint: sample.local_sent.checked_add(elapsed / 2)?,
            server_midpoint: sample
                .server_received
                .checked_add(i64::try_from(work / 2).ok()?)?,
            sampled_at: sample.local_received,
            network_delay,
        })
    }

    pub fn uncertainty_ms(self) -> u64 {
        self.network_delay.div_ceil(2).saturating_add(1)
    }

    pub fn server_time(self, local_now: u64) -> Option<i64> {
        if local_now.checked_sub(self.sampled_at)? > MAX_CLOCK_AGE {
            return None;
        }
        let offset = i64::try_from(local_now.checked_sub(self.local_midpoint)?).ok()?;
        self.server_midpoint.checked_add(offset)
    }
}

/// The adapter parses the server UUID and rejects the nil identifier.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Id([u8; 16]);

impl Id {
    pub fn new(value: [u8; 16]) -> Option<Self> {
        (value != [0; 16]).then_some(Self(value))
    }

    pub fn bytes(self) -> [u8; 16] {
        self.0
    }
}

/// Captured when opening a connection to a room. Never reuse an epoch between
/// account changes or reconnects, even when the server room ID is unchanged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Membership {
    pub epoch: u64,
    pub group: Id,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QueueItem {
    pub item: Id,
    pub playlist_item: Id,
    pub start_ticks: u64,
}

/// A load ticket identifies a specific occurrence on this connection.
/// A delayed decoder callback cannot reuse an earlier ticket.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MediaLoad {
    pub item: QueueItem,
    membership: Membership,
    revision: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QueueChange {
    Unchanged,
    Clear,
    Load(MediaLoad),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Pause { position_ticks: u64 },
    Resume { position_ticks: u64 },
    Seek { position_ticks: u64 },
    Stop,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Command {
    pub group: Id,
    pub playlist_item: Id,
    /// The wire timestamp's original ordering precision, in 100-ns ticks.
    pub emitted_at: i64,
    pub execute_at_ms: i64,
    pub action: Action,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ready {
    pub playlist_item: Id,
    pub position_ticks: u64,
    pub when_ms: i64,
    pub is_playing: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandStatus {
    Scheduled,
    Stale,
    OutsideClockWindow,
}

#[derive(Clone, Copy)]
struct Playback {
    load: MediaLoad,
    authorized: bool,
    command: Option<Command>,
    delivered: bool,
}

#[derive(Clone, Copy)]
struct ActiveRoom {
    membership: Membership,
    clock: Option<RoomClock>,
    queue_updated_at: Option<i64>,
    queue_revision: u64,
    playback: Option<Playback>,
}

/// One independently authenticated room. Feed server events in delivery order.
/// Socket loss must call `retire` before reconnecting or reusing the player.
#[derive(Default)]
pub struct Room {
    highest_epoch: u64,
    room: Option<ActiveRoom>,
}

impl Room {
    /// Call only after the server confirms actual room admission.
    pub fn joined(&mut self, membership: Membership) -> bool {
        if membership.epoch <= self.highest_epoch {
            return false;
        }
        self.highest_epoch = membership.epoch;
        self.room = Some(ActiveRoom {
            membership,
            clock: None,
            queue_updated_at: None,
            queue_revision: 0,
            playback: None,
        });
        true
    }

    pub fn retire(&mut self, membership: Membership) {
        if self.active(membership).is_some() {
            self.room = None;
        }
    }

    fn active(&mut self, membership: Membership) -> Option<&mut ActiveRoom> {
        self.room
            .as_mut()
            .filter(|room| room.membership == membership)
    }

    pub fn update_clock(&mut self, membership: Membership, clock: RoomClock) -> bool {
        let Some(room) = self.active(membership) else {
            return false;
        };
        if room
            .clock
            .is_some_and(|old| old.sampled_at > clock.sampled_at)
        {
            return false;
        }
        room.clock = Some(clock);
        true
    }

    /// Every change of occurrence requires fresh, independent media access.
    pub fn update_queue(
        &mut self,
        membership: Membership,
        updated_at: i64,
        item: Option<QueueItem>,
    ) -> QueueChange {
        let Some(room) = self.active(membership) else {
            return QueueChange::Unchanged;
        };
        if room
            .queue_updated_at
            .is_some_and(|previous| updated_at < previous)
        {
            return QueueChange::Unchanged;
        }
        room.queue_updated_at = Some(updated_at);
        let identity = |item: QueueItem| (item.item, item.playlist_item);
        if room.playback.map(|current| identity(current.load.item)) == item.map(identity) {
            return QueueChange::Unchanged;
        }
        let Some(revision) = room.queue_revision.checked_add(1) else {
            self.retire(membership);
            return QueueChange::Clear;
        };
        room.queue_revision = revision;
        room.playback = item.map(|item| Playback {
            load: MediaLoad {
                item,
                membership,
                revision,
            },
            authorized: false,
            command: None,
            delivered: false,
        });
        room.playback.map_or(QueueChange::Clear, |playback| {
            QueueChange::Load(playback.load)
        })
    }

    /// Call only when this account's authorized exact item is ready to seek.
    pub fn item_ready(&mut self, load: MediaLoad) -> bool {
        let Some(playback) = self
            .active(load.membership)
            .and_then(|room| room.playback.as_mut())
        else {
            return false;
        };
        if playback.load != load {
            return false;
        }
        playback.authorized = true;
        true
    }

    pub fn schedule(
        &mut self,
        membership: Membership,
        command: Command,
        local_now: u64,
    ) -> CommandStatus {
        let Some(room) = self.active(membership) else {
            return CommandStatus::Stale;
        };
        let Some(playback) = room.playback.as_mut() else {
            return CommandStatus::Stale;
        };
        if command.group != membership.group
            || command.playlist_item != playback.load.item.playlist_item
            || room
                .queue_updated_at
                .is_some_and(|at| command.emitted_at < at)
            || playback.command.is_some_and(|previous| {
                previous == command || previous.emitted_at > command.emitted_at
            })
        {
            return CommandStatus::Stale;
        }
        let Some(now) = room.clock.and_then(|clock| clock.server_time(local_now)) else {
            return CommandStatus::OutsideClockWindow;
        };
        let allowed = if command.execute_at_ms >= now {
            command.execute_at_ms.abs_diff(now) <= MAX_FUTURE
        } else {
            command.execute_at_ms.abs_diff(now) <= MAX_LATE
        };
        if !allowed {
            return CommandStatus::OutsideClockWindow;
        }
        playback.command = Some(command);
        playback.delivered = false;
        CommandStatus::Scheduled
    }

    /// Take the newest due action once. Call from the current player's
    /// monotonic timer. A late resume advances its starting media position.
    pub fn take_due(&mut self, membership: Membership, local_now: u64) -> Option<Action> {
        let room = self.active(membership)?;
        let playback = room.playback.as_mut()?;
        if !playback.authorized || playback.delivered {
            return None;
        }
        let now = room.clock?.server_time(local_now)?;
        let command = playback.command?;
        if now < command.execute_at_ms {
            return None;
        }
        playback.delivered = true;
        let late = now.abs_diff(command.execute_at_ms);
        if late > MAX_LATE {
            return None;
        }
        match command.action {
            Action::Resume { position_ticks } => Some(Action::Resume {
                position_ticks: position_ticks
                    .checked_add(late.checked_mul(TICKS_PER_MILLISECOND)?)?,
            }),
            action => Some(action),
        }
    }

    /// Build a Ready or Buffering report only for the current authorized item.
    pub fn report(
        &mut self,
        load: MediaLoad,
        local_now: u64,
        position_ticks: u64,
        is_playing: bool,
    ) -> Option<Ready> {
        let room = self.active(load.membership)?;
        let playback = room
            .playback
            .filter(|current| current.authorized && current.load == load)?;
        Some(Ready {
            playlist_item: playback.load.item.playlist_item,
            position_ticks,
            when_ms: room.clock?.server_time(local_now)?,
            is_playing,
        })
    }
}
