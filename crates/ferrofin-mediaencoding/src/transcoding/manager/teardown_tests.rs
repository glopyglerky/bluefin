//! Real process accounting, independently of the retained-input FD budget.
use super::*;
use crate::TokioSegmentTranscoder;
use crate::encoding_helper::{BaseEncodingJobOptions, TranscodeDisplayNames};
use ferrofin_model::{dlna::SubtitleDeliveryMethod, dto::MediaSourceInfo};
use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicBool, Ordering},
};

fn encoder() -> &'static Path {
    static ENCODER: OnceLock<tempfile::TempDir> = OnceLock::new();
    ENCODER.get_or_init(|| {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("child.rs");
        std::fs::write(&source, r#"use std::{io::Read, path::Path, time::Duration};
fn main() {
    let args: Vec<_> = std::env::args().collect();
    let root = Path::new(&args[1]);
    let session = &args[2];
    std::fs::write(root.join(format!("{session}.pid")), std::process::id().to_string()).unwrap();
    std::fs::write(root.join(format!("{session}.m3u8")), b"actual child output").unwrap();
    let mut quit = [0;2];
    let _ = std::io::stdin().read_exact(&mut quit);
    std::fs::write(root.join(format!("{session}.stopping")), b"stop received").unwrap();
    while !root.join(format!("{session}.exit")).exists() { std::thread::sleep(Duration::from_millis(5)); }
}
"#).unwrap();
        assert!(std::process::Command::new("rustc").args(["--edition=2024", "-C", "debuginfo=0"])
            .arg(&source).arg("-o").arg(dir.path().join("child")).status().unwrap().success());
        dir
    }).path()
}

fn state(output: &Path, session: &str, device: &str) -> EncodingJobInfo {
    EncodingJobInfo {
        display: TranscodeDisplayNames::default(),
        base_request: BaseEncodingJobOptions::default(),
        video_stream: None,
        audio_stream: None,
        subtitle_stream: None,
        media_source: MediaSourceInfo::default(),
        output_video_codec: Some("h264".into()),
        output_audio_codec: None,
        output_video_bitrate: None,
        output_audio_bitrate: None,
        output_audio_channels: None,
        output_container: None,
        output_video_sync: None,
        output_file_path: output.to_string_lossy().into_owned(),
        input_container: None,
        is_input_video: true,
        subtitle_delivery_method: SubtitleDeliveryMethod::Encode,
        trimmed_subtitle_path: None,
        run_time_ticks: None,
        transcoding_type: TranscodingJobType::Hls,
        supported_video_codecs: vec![],
        supported_audio_codecs: vec![],
        supported_subtitle_codecs: vec![],
        segment_length_secs: 6,
        wait_for_path: None,
        segment_container: Some("ts".into()),
        play_session_id: Some(session.into()),
        device_id: Some(device.into()),
    }
}

async fn start(
    manager: &TranscodeManagerImpl<NoopSessionReporter>,
    transcoder: &dyn SegmentTranscoder,
    root: &Path,
    session: &str,
    device: &str,
    user: u128,
) -> Result<TranscodingJobHandle, StartFfmpegError> {
    let output = root.join(format!("{session}.m3u8"));
    let state = state(&output, session, device);
    manager
        .start_ffmpeg(
            transcoder,
            StartFfMpegRequest {
                program: encoder().join("child").to_str().unwrap(),
                state: &state,
                output_path: &output,
                arguments: vec![root.to_string_lossy().into_owned(), session.into()],
                log_dir: root.join("logs"),
                working_dir: None,
                env: vec![],
                hardware_acceleration_type: HardwareAccelerationType::none,
                user_id: Some(uuid::Uuid::from_u128(user)),
            },
        )
        .await
}

async fn until(mut condition: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(10), async {
        while !condition() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
}
fn alive(pid: u32) -> bool {
    Path::new(&format!("/proc/{pid}")).exists()
}
fn pid(root: &Path, session: &str) -> u32 {
    std::fs::read_to_string(root.join(format!("{session}.pid")))
        .unwrap()
        .parse()
        .unwrap()
}

#[rstest::rstest]
#[case::general_same(0, 1, 1, 1)]
#[case::general_other(0, 1, 0, 2)]
#[case::per_user(0, 0, 1, 1)]
#[case::seek(1, 1, 1, 1)]
#[case::first_output_timeout(2, 1, 1, 1)]
#[case::idle_reaper(3, 1, 1, 1)]
#[case::same_device_replacement(4, 1, 1, 1)]
#[case::cancel_stop(5, 1, 1, 1)]
#[tokio::test]
async fn stopping_real_child_reserves_configured_capacity_until_observed_exit(
    #[case] mode: u8,
    #[case] server: u32,
    #[case] per_user: u32,
    #[case] next_user: u128,
) {
    let root = tempfile::tempdir().unwrap();
    let manager = Arc::new(
        TranscodeManagerImpl::new(NoopSessionReporter)
            .with_transcode_limits(TranscodeLimits { server, per_user }),
    );
    let first = start(
        &manager,
        &TokioSegmentTranscoder::new(),
        root.path(),
        "first",
        "device",
        1,
    )
    .await
    .unwrap();
    let first_pid = pid(root.path(), "first");
    let stopping_manager = Arc::clone(&manager);
    let stopping_handle = first.clone();
    let stopping_root = root.path().to_owned();
    let mut stop = tokio::spawn(async move {
        match mode {
            0 | 5 => stopping_manager
                .kill_transcoding_jobs("unrelated", Some("first"), true)
                .await
                .unwrap(),
            1 => {
                stopping_manager
                    .kill_and_remove(&stopping_handle, false, false)
                    .await;
            }
            2 => {
                let _ = stopping_manager
                    .wait_for_first_output_until(
                        &stopping_handle,
                        &stopping_root.join("never.ts"),
                        Instant::now(),
                    )
                    .await;
            }
            3 => {
                stopping_manager.jobs.lock().unwrap()[0].ping_timeout_ms = -1;
                stopping_manager.reap_idle_jobs().await;
            }
            4 => {
                let _ = start(
                    &stopping_manager,
                    &TokioSegmentTranscoder::new(),
                    &stopping_root,
                    "replacement",
                    "device",
                    1,
                )
                .await;
            }
            _ => unreachable!(),
        }
    });
    until(|| root.path().join("first.stopping").exists()).await;
    assert!(alive(first_pid), "actual slow child must still be alive");
    assert_eq!(
        manager.active_job_count(),
        1,
        "stopping reservation remains"
    );
    assert!(
        manager
            .get_transcoding_job_by_path(&first.path, first.job_type)
            .await
            .unwrap()
            .is_none(),
        "stopping process cannot be reused"
    );
    assert!(
        start(
            &manager,
            &TokioSegmentTranscoder::new(),
            root.path(),
            "second",
            "another-device",
            next_user
        )
        .await
        .is_err()
    );
    assert!(
        !root.path().join("second.pid").exists(),
        "cap must reject before actual fork"
    );
    assert!(
        start(
            &manager,
            &TokioSegmentTranscoder::new(),
            root.path(),
            "duplicate",
            "device",
            1
        )
        .await
        .is_err(),
        "a stopping job cannot be evicted again"
    );
    if mode == 5 {
        stop.abort();
        let _ = (&mut stop).await;
        assert!(alive(first_pid));
    }
    std::fs::write(root.path().join("first.exit"), b"exit").unwrap();
    if mode != 5 {
        stop.await.unwrap();
    }
    // A canceled stop is completed by the same shared reaper, not a new pool.
    manager.reap_idle_jobs().await;
    until(|| !alive(first_pid)).await;
    if root.path().join("replacement.pid").exists() {
        std::fs::write(root.path().join("replacement.exit"), b"exit").unwrap();
        manager
            .kill_transcoding_jobs("device", Some("replacement"), true)
            .await
            .unwrap();
    }
    let second = start(
        &manager,
        &TokioSegmentTranscoder::new(),
        root.path(),
        "second",
        "another-device",
        next_user,
    )
    .await
    .unwrap();
    let second_pid = pid(root.path(), "second");
    assert!(alive(second_pid) && !alive(first_pid));
    std::fs::write(root.path().join("second.exit"), b"exit").unwrap();
    manager.kill_and_remove(&second, true, false).await;
    until(|| !alive(second_pid)).await;
    assert_eq!(manager.active_job_count(), 0);
}

struct InterruptedSpawn {
    published: AtomicBool,
    settle: AtomicBool,
    fail_stop: Arc<AtomicBool>,
}
struct FailingStop {
    child: Arc<dyn TranscodeChild>,
    fail: Arc<AtomicBool>,
}
#[async_trait]
impl TranscodeChild for FailingStop {
    fn has_exited(&self) -> bool {
        self.child.has_exited()
    }
    fn exit_code(&self) -> Option<i32> {
        self.child.exit_code()
    }
    async fn wait(&self) -> i32 {
        self.child.wait().await
    }
    async fn kill(&self) -> Result<(), String> {
        if self.fail.swap(false, Ordering::SeqCst) {
            Err("injected signal failure".into())
        } else {
            self.child.kill().await
        }
    }
}
#[async_trait]
impl SegmentTranscoder for InterruptedSpawn {
    async fn start_transcode(&self, _: &SpawnRequest) -> Result<Box<dyn TranscodeChild>, String> {
        unreachable!()
    }
    async fn start_registered(
        &self,
        req: &SpawnRequest,
        register: &(dyn Fn(Arc<dyn TranscodeChild>) + Send + Sync),
    ) -> Result<(), String> {
        TokioSegmentTranscoder::new()
            .start_registered(req, &|child| {
                register(Arc::new(FailingStop {
                    child,
                    fail: Arc::clone(&self.fail_stop),
                }));
                self.published.store(true, Ordering::SeqCst);
            })
            .await?;
        while !self.settle.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        Ok(())
    }
}

#[rstest::rstest]
#[case::canceled(true)]
#[case::returned_after_stop(false)]
#[tokio::test]
async fn canceled_delayed_registration_and_failed_stop_keep_the_actual_child_counted(
    #[case] cancel: bool,
) {
    let root = tempfile::tempdir().unwrap();
    let manager = Arc::new(
        TranscodeManagerImpl::new(NoopSessionReporter).with_transcode_limits(TranscodeLimits {
            server: 1,
            per_user: 1,
        }),
    );
    let transcoder = Arc::new(InterruptedSpawn {
        published: AtomicBool::new(false),
        settle: AtomicBool::new(false),
        fail_stop: Arc::new(AtomicBool::new(true)),
    });
    let task_manager = Arc::clone(&manager);
    let task_transcoder = Arc::clone(&transcoder);
    let task_root = root.path().to_owned();
    let startup = tokio::spawn(async move {
        start(
            &task_manager,
            task_transcoder.as_ref(),
            &task_root,
            "first",
            "device",
            1,
        )
        .await
    });
    until(|| transcoder.published.load(Ordering::SeqCst) && root.path().join("first.pid").exists())
        .await;
    let first_pid = pid(root.path(), "first");
    assert!(alive(first_pid));
    let task_manager = Arc::clone(&manager);
    let stop = tokio::spawn(async move {
        task_manager
            .kill_owned_encoding(uuid::Uuid::from_u128(1), "device", "first")
            .await
    });
    until(|| manager.jobs.lock().unwrap()[0].is_stopping()).await;
    assert!(!stop.is_finished(), "stop joins in-flight registration");
    assert!(
        start(
            &manager,
            &TokioSegmentTranscoder::new(),
            root.path(),
            "second",
            "other",
            2
        )
        .await
        .is_err()
    );
    if cancel {
        startup.abort();
        let _ = startup.await;
    } else {
        transcoder.settle.store(true, Ordering::SeqCst);
        assert!(
            startup.await.unwrap().is_err(),
            "existing output must not reuse a stopping startup"
        );
    }
    assert!(
        stop.await.unwrap().is_err(),
        "injected actual-child signal failure"
    );
    assert!(alive(first_pid));
    assert_eq!(manager.active_job_count(), 1);
    assert!(
        start(
            &manager,
            &TokioSegmentTranscoder::new(),
            root.path(),
            "second",
            "other",
            2
        )
        .await
        .is_err()
    );
    std::fs::write(root.path().join("first.exit"), b"exit").unwrap();
    manager.reap_idle_jobs().await;
    until(|| !alive(first_pid)).await;
    let second = start(
        &manager,
        &TokioSegmentTranscoder::new(),
        root.path(),
        "second",
        "other",
        2,
    )
    .await
    .unwrap();
    std::fs::write(root.path().join("second.exit"), b"exit").unwrap();
    manager.kill_and_remove(&second, true, false).await;
    assert_eq!(manager.active_job_count(), 0);
}
