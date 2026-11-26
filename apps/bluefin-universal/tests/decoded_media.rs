//! Opt-in runtime qualification against original fixtures, never personal media.
use bluefin_universal::{
    bridge::Bridge,
    mpv::{Command, Decoder},
    playback::{self, Platform, Selection},
    protocol::{self, Item},
};
use std::time::{Duration, Instant};
fn item(id: u128, kind: &str) -> Item {
    Item {
        id: uuid::Uuid::from_u128(id),
        name: "Original generated fixture".into(),
        kind: kind.into(),
        overview: String::new(),
        start_date: None,
        end_date: None,
        channel_name: None,
        channel_id: None,
    }
}
#[test]
#[ignore = "requires task-owned synthetic fixture server and compatible libmpv runtime"]
fn actual_native_decoded_video_audio_seek_hls_live_and_stop() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let base =
        url::Url::parse(&std::env::var("BLUEFIN_FIXTURE_BASE").expect("task fixture URL")).unwrap();
    let session = rt
        .block_on(protocol::login(base, "fixture".into(), String::new()))
        .unwrap();
    for (id, kind, hls) in [
        (1, "Movie", false),
        (3, "Audio", false),
        (1, "Movie", true),
        (5, "TvChannel", true),
    ] {
        eprintln!("Decoding original fixture: id={id} kind={kind} hls={hls}");
        let selection = Selection {
            audio: if hls && id == 1 { Some(1) } else { None },
            ..Default::default()
        };
        let plan = rt
            .block_on(playback::negotiate(
                &session,
                item(id, kind),
                selection,
                Platform::Native,
            ))
            .unwrap();
        assert_eq!(plan.hls, hls);
        let bridge = Bridge::start(session.clone(), plan.clone(), None).unwrap();
        let decoder = Decoder::start(bridge.url.clone(), 0.0, true, plan.hls).unwrap();
        let start = Instant::now();
        let mut frame = false;
        loop {
            let state = decoder.snapshot();
            assert!(!state.failed, "decode failed: {:?}", state.error);
            if let Some(pixels) = decoder.frame() {
                assert_eq!(pixels.rgba.len(), pixels.size[0] * pixels.size[1] * 4);
                assert!(pixels.size[0] <= 4096 && pixels.size[1] <= 2160);
                frame |= pixels
                    .rgba
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .any(|p| p[0] > 20 || p[1] > 20 || p[2] > 20);
            }
            if state.position > 0.3
                && state.audio_decoded
                && (kind == "Audio" || (state.video_decoded && frame))
            {
                break;
            }
            assert!(
                start.elapsed() < Duration::from_secs(15),
                "no actual decoded time/audio/video progress: pos={} audio={} video={} error={:?}",
                state.position,
                state.audio_decoded,
                state.video_decoded,
                state.error
            );
            std::thread::sleep(Duration::from_millis(40));
        }
        rt.block_on(playback::report(
            &session,
            &plan,
            "started",
            decoder.snapshot().position,
            false,
        ))
        .unwrap();
        if kind == "Movie" {
            decoder.command(Command::Seek(6.0));
            let seek = Instant::now();
            while decoder.snapshot().position < 5.5 {
                assert!(seek.elapsed() < Duration::from_secs(8));
                std::thread::sleep(Duration::from_millis(40));
            }
        }
        rt.block_on(playback::report(
            &session,
            &plan,
            "progress",
            decoder.snapshot().position,
            false,
        ))
        .unwrap();
        let end = Instant::now();
        bridge.stop();
        decoder.stop();
        while !bridge.stopped() || !decoder.snapshot().stopped {
            assert!(
                end.elapsed() < Duration::from_secs(12),
                "stop not acknowledged"
            );
            std::thread::sleep(Duration::from_millis(30));
        }
        bridge.cleanup_receipt().unwrap();
        rt.block_on(playback::report(
            &session,
            &plan,
            "stopped",
            decoder.snapshot().position,
            true,
        ))
        .unwrap();
        rt.block_on(playback::release(&session, &plan)).unwrap();
    }
    rt.block_on(protocol::request(
        &session.base,
        &session.token,
        "POST",
        "Sessions/Logout",
        &[],
        None,
    ))
    .unwrap();
}
#[test]
#[ignore = "requires task-owned synthetic fixture server and compatible libmpv runtime"]
fn actual_native_queue_eof_and_account_revocation() {
    use bluefin_contract::QueueState;
    use bluefin_universal::service::{Command as Control, Service};
    let rt = tokio::runtime::Runtime::new().unwrap();
    let base = url::Url::parse(&std::env::var("BLUEFIN_FIXTURE_BASE").unwrap()).unwrap();
    let session = rt
        .block_on(protocol::login(base, "fixture".into(), String::new()))
        .unwrap();
    let service = Service::start(session.clone(), true, None);
    let queue = QueueState {
        item_ids: vec![uuid::Uuid::from_u128(3), uuid::Uuid::from_u128(6)],
        ..Default::default()
    };
    service.send(Control::Play(
        Box::new(item(3, "Audio")),
        Selection::default(),
        Some((queue, 0)),
    ));
    let start = Instant::now();
    let mut first_session = None;
    while service
        .snapshot()
        .plan
        .is_none_or(|p| p.item.id != uuid::Uuid::from_u128(6))
    {
        if let Some(plan) = service.snapshot().plan
            && plan.item.id == uuid::Uuid::from_u128(3)
        {
            first_session = Some(plan.play_session);
        }
        assert!(
            start.elapsed() < Duration::from_secs(15),
            "queue did not progress: {}",
            service.snapshot().message
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    let evidence = rt
        .block_on(protocol::request(
            &session.base,
            &session.token,
            "GET",
            "evidence",
            &[],
            None,
        ))
        .unwrap();
    let first_session = first_session.expect("first queue plan");
    assert!(
        evidence["reports"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["PlaySessionId"] == first_session
                && r["phase"] == "/fixture/Sessions/Playing"),
        "short audio must report started before EOF"
    );
    service.send(Control::Logout);
    let start = Instant::now();
    while !service.snapshot().finished {
        assert!(start.elapsed() < Duration::from_secs(15));
        std::thread::sleep(Duration::from_millis(50));
    }
    let status = service.snapshot();
    assert!(status.revoked);
    assert!(status.cleanup_acknowledged, "{}", status.message);
    assert!(
        rt.block_on(protocol::request(
            &session.base,
            &session.token,
            "GET",
            "Bluefin/Capabilities",
            &[],
            None
        ))
        .is_err()
    );
}

#[test]
#[ignore = "requires task-owned synthetic fixture server and compatible libmpv runtime"]
fn actual_native_ui_frame_mailbox_is_not_capped_by_reporting_tick() {
    use bluefin_universal::service::{Command as Control, Service};
    let rt = tokio::runtime::Runtime::new().unwrap();
    let base = url::Url::parse(&std::env::var("BLUEFIN_FIXTURE_BASE").unwrap()).unwrap();
    let session = rt
        .block_on(protocol::login(base, "fixture".into(), String::new()))
        .unwrap();
    let service = Service::start(session, true, None);
    service.send(Control::Play(
        Box::new(item(1, "Movie")),
        Selection::default(),
        None,
    ));
    let ready = Instant::now();
    while service.snapshot().position < 0.2 {
        assert!(
            ready.elapsed() < Duration::from_secs(12),
            "{}",
            service.snapshot().message
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    let start = Instant::now();
    let mut frames = 0;
    while start.elapsed() < Duration::from_secs(2) {
        if service.frame().is_some() {
            frames += 1;
        }
        std::thread::sleep(Duration::from_millis(16));
    }
    service.send(Control::Logout);
    let ended = Instant::now();
    while !service.snapshot().finished {
        assert!(ended.elapsed() < Duration::from_secs(15));
        std::thread::sleep(Duration::from_millis(20));
    }
    eprintln!("Actual decoded frames delivered to UI mailbox in two seconds: {frames}");
    assert!(
        frames > 28,
        "UI mailbox yielded only {frames} frames in two seconds"
    );
}
