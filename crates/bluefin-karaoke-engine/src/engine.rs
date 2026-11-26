use crate::{
    Cancellation, Error, PublishedTask, StagingRoot, audio,
    checkpoint::{self, Checkpoint},
    model::Model,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

static ENGINE_OWNED: AtomicBool = AtomicBool::new(false);
struct Lease;
impl Drop for Lease {
    fn drop(&mut self) {
        ENGINE_OWNED.store(false, Ordering::Release);
    }
}

/// Trusted operator configuration, never deserialized from a client request.
#[derive(Clone, Copy, Debug)]
pub struct Config {
    /// Inclusive 1..=300 seconds, also bounded to 52,920,044 input bytes.
    pub max_duration_seconds: u32,
    /// Inclusive 1..=4 Candle workers. Both startup thread environment settings
    /// and actual persistent pools must match; no global environment mutation.
    pub cpu_threads: usize,
    /// Inclusive 1..=3600 seconds. Checked between DSP blocks and neural layers.
    pub task_timeout: Duration,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            max_duration_seconds: 300,
            cpu_threads: 3,
            task_timeout: Duration::from_secs(600),
        }
    }
}
impl Config {
    fn validate(self) -> Result<Self, Error> {
        if !(1..=300).contains(&self.max_duration_seconds)
            || !(1..=4).contains(&self.cpu_threads)
            || self.task_timeout < Duration::from_secs(1)
            || self.task_timeout > Duration::from_secs(3600)
        {
            return Err(Error::InvalidConfig);
        }
        Ok(self)
    }
}
/// Immutable, explicitly preadmitted canonical PCM bytes. The opaque attachment
/// is returned unchanged; constructing this value establishes no authorization.
/// The parent pins the original source generation and actual authenticated
/// authorization, prepares PCM lawfully, and rechecks both before admission.
pub struct Input<'a, A> {
    pub pcm_wav: &'a [u8],
    /// Digest pinned by the server admission step, not supplied by a client.
    pub expected_pcm_sha256: [u8; 32],
    pub attachment: A,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StemMetadata {
    pub sha256: String,
    pub bytes: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Provenance {
    pub processing: String,
    pub engine_version: String,
    pub model_archive_sha256: String,
    pub publisher_commit: String,
    pub runtime: String,
    pub cpu_threads: usize,
    pub input_pcm_sha256: String,
    pub sample_rate: u32,
    pub channels: u16,
    pub frames: u64,
    pub mask_extension: String,
    pub lead_vocal: StemMetadata,
    pub accompaniment: StemMetadata,
    pub elapsed_seconds: f64,
}
pub struct Processed<A> {
    pub attachment: A,
    pub provenance: Provenance,
    pub files: PublishedTask,
}
/// Single process-wide model owner. Use one server queue and `&mut Engine`.
/// The process lease bounds model copies and inference concurrency to one.
pub struct Engine {
    model: Model,
    config: Config,
    _lease: Lease,
}
impl Engine {
    pub fn load_official_archive(
        archive: &[u8],
        config: Config,
        cancel: &Cancellation,
    ) -> Result<Self, Error> {
        let config = config.validate()?;
        cancel.check()?;
        if ENGINE_OWNED
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(Error::Busy);
        }
        let lease = Lease;
        let control = cancel.bounded(config.task_timeout);
        check_thread_environment(config.cpu_threads)?;
        // Candle 0.11 owns persistent Rayon and barrier pools. Verify the actual
        // pools too, including a pool initialized previously by another component.
        if candle_core::utils::with_threadpool(rayon::current_num_threads) != config.cpu_threads
            || candle_core::utils::barrier_pool().n_workers() != config.cpu_threads - 1
        {
            return Err(Error::ThreadConfig);
        }
        let c = Checkpoint::from_official_archive(archive, &control)?;
        let model = candle_core::utils::with_threadpool(|| Model::load(&c))?;
        control.check()?;
        Ok(Self {
            model,
            config,
            _lease: lease,
        })
    }
    pub fn process<A>(
        &mut self,
        input: Input<'_, A>,
        staging: &StagingRoot,
        cancel: &Cancellation,
    ) -> Result<Processed<A>, Error> {
        let start = Instant::now();
        let control = cancel.bounded(self.config.task_timeout);
        control.check()?;
        check_thread_environment(self.config.cpu_threads)?;
        let frames = audio::validate(input.pcm_wav, self.config.max_duration_seconds)?;
        let mut hash = Sha256::new();
        for chunk in input.pcm_wav.chunks(65536) {
            control.check()?;
            hash.update(chunk);
        }
        let digest: [u8; 32] = hash.finalize().into();
        if digest != input.expected_pcm_sha256 {
            return Err(Error::SourceHash);
        }
        let mut task = staging.begin()?;
        let result = (|| {
            let samples = audio::samples(input.pcm_wav);
            control.check()?;
            let [vocals, accompaniment] = candle_core::utils::with_threadpool(|| {
                audio::separate(&self.model, &samples, &control)
            })?;
            drop(samples);
            let vocal_wav = audio::encode(&vocals, &control)?;
            drop(vocals);
            let accompaniment_wav = audio::encode(&accompaniment, &control)?;
            drop(accompaniment);
            for bytes in [&vocal_wav, &accompaniment_wav] {
                if audio::validate(bytes, self.config.max_duration_seconds)? != frames {
                    return Err(Error::InvalidOutput);
                }
            }
            let stem = |bytes: &[u8]| StemMetadata {
                sha256: checkpoint::digest(bytes),
                bytes: bytes.len() as u64,
            };
            let provenance = Provenance {
                processing: "modelSeparatedStems".into(),
                engine_version: env!("CARGO_PKG_VERSION").into(),
                model_archive_sha256: checkpoint::ARCHIVE_SHA256.into(),
                publisher_commit: checkpoint::RELEASE_COMMIT.into(),
                runtime: "candle-core-0.11.1-rustfft-6.4.1-cpu".into(),
                cpu_threads: self.config.cpu_threads,
                input_pcm_sha256: checkpoint::digest(input.pcm_wav),
                sample_rate: audio::RATE as u32,
                channels: 2,
                frames: frames as u64,
                mask_extension: "zeros-above-bin-1023".into(),
                lead_vocal: stem(&vocal_wav),
                accompaniment: stem(&accompaniment_wav),
                elapsed_seconds: start.elapsed().as_secs_f64(),
            };
            task.write("lead-vocal.wav", &vocal_wav, &control)?;
            task.write("accompaniment.wav", &accompaniment_wav, &control)?;
            task.write(
                "provenance.json",
                &serde_json::to_vec_pretty(&provenance)?,
                &control,
            )?;
            task.publish(&control)?;
            Ok(provenance)
        })();
        match result {
            Ok(provenance) => Ok(Processed {
                attachment: input.attachment,
                provenance,
                files: task,
            }),
            Err(e) => Err(task.abort(e)),
        }
    }
}

fn check_thread_environment(threads: usize) -> Result<(), Error> {
    for key in ["RAYON_NUM_THREADS", "CANDLE_NUM_THREADS"] {
        if std::env::var(key)
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            != Some(threads)
        {
            return Err(Error::ThreadConfig);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs::File, os::unix::fs::PermissionsExt};
    #[test]
    fn rejects_runtime_boundaries() {
        for c in [
            Config {
                max_duration_seconds: 0,
                ..Config::default()
            },
            Config {
                max_duration_seconds: 301,
                ..Config::default()
            },
            Config {
                cpu_threads: 0,
                ..Config::default()
            },
            Config {
                cpu_threads: 5,
                ..Config::default()
            },
            Config {
                task_timeout: Duration::from_secs(3601),
                ..Config::default()
            },
        ] {
            assert!(c.validate().is_err());
        }
        let c = Cancellation::default().bounded(Duration::ZERO);
        assert!(matches!(c.check(), Err(Error::Deadline)));
    }
    #[test]
    #[ignore = "official archive + BLUEFIN_MODEL_ARCHIVE and private TMPDIR required"]
    fn official_cpu_inference_failures_cancellation_and_isolation() {
        let archive = std::fs::read(
            std::env::var_os("BLUEFIN_MODEL_ARCHIVE").expect("official archive env required"),
        )
        .unwrap();
        let c = Cancellation::default();
        let mut corrupt = archive.clone();
        corrupt[100] ^= 1;
        assert!(matches!(
            Engine::load_official_archive(&corrupt, Config::default(), &c),
            Err(Error::InvalidModel)
        ));
        assert!(matches!(
            Engine::load_official_archive(&[], Config::default(), &c),
            Err(Error::InvalidModel)
        ));
        let cancelled = Cancellation::default();
        cancelled.cancel();
        assert!(matches!(
            Engine::load_official_archive(&archive, Config::default(), &cancelled),
            Err(Error::Cancelled)
        ));
        let prep_cancel = Cancellation::default();
        let signal = prep_cancel.clone();
        let prep_worker = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(20));
            signal.cancel();
        });
        assert!(matches!(
            Engine::load_official_archive(&archive, Config::default(), &prep_cancel),
            Err(Error::Cancelled)
        ));
        prep_worker.join().unwrap();
        let mut engine = Engine::load_official_archive(
            &archive,
            Config {
                max_duration_seconds: 2,
                ..Config::default()
            },
            &c,
        )
        .unwrap();
        assert!(matches!(
            Engine::load_official_archive(&archive, Config::default(), &c),
            Err(Error::Busy)
        ));
        let root = tempfile::tempdir().unwrap();
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let staging = StagingRoot::from_opened_directory(File::open(root.path()).unwrap()).unwrap();
        let samples: Vec<f32> = (0..audio::RATE * 2)
            .map(|i| {
                ((i / 2) as f32 * 440. * std::f32::consts::TAU / audio::RATE as f32).sin() * 0.1
            })
            .collect();
        let wav = audio::encode(&samples, &c).unwrap();
        let hash = Sha256::digest(&wav).into();
        assert!(matches!(
            engine.process(
                Input {
                    pcm_wav: &wav,
                    expected_pcm_sha256: [0; 32],
                    attachment: ()
                },
                &staging,
                &c
            ),
            Err(Error::SourceHash)
        ));
        for bad in [&wav[..wav.len() - 1], &b"not audio"[..]] {
            assert!(matches!(
                engine.process(
                    Input {
                        pcm_wav: bad,
                        expected_pcm_sha256: hash,
                        attachment: ()
                    },
                    &staging,
                    &c
                ),
                Err(Error::InvalidInput)
            ));
        }
        let mut bad = wav.clone();
        bad[20] = 3;
        bad[44..48].copy_from_slice(&f32::NAN.to_le_bytes());
        assert!(matches!(
            engine.process(
                Input {
                    pcm_wav: &bad,
                    expected_pcm_sha256: hash,
                    attachment: ()
                },
                &staging,
                &c
            ),
            Err(Error::InvalidInput)
        ));
        let long = audio::encode(&vec![0.; audio::RATE * 2 * 3], &c).unwrap();
        assert!(matches!(
            engine.process(
                Input {
                    pcm_wav: &long,
                    expected_pcm_sha256: hash,
                    attachment: ()
                },
                &staging,
                &c
            ),
            Err(Error::InvalidInput)
        ));
        assert!(matches!(
            engine.process(
                Input {
                    pcm_wav: &wav,
                    expected_pcm_sha256: hash,
                    attachment: ()
                },
                &staging,
                &cancelled
            ),
            Err(Error::Cancelled)
        ));
        let cancel = Cancellation::default();
        let other = cancel.clone();
        let cancelling = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(250));
            other.cancel();
        });
        let start = Instant::now();
        let result = engine.process(
            Input {
                pcm_wav: &wav,
                expected_pcm_sha256: hash,
                attachment: (),
            },
            &staging,
            &cancel,
        );
        cancelling.join().unwrap();
        assert!(matches!(result, Err(Error::Cancelled)));
        println!(
            "cancellation elapsed seconds={}",
            start.elapsed().as_secs_f64()
        );
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
        engine.config.task_timeout = Duration::from_secs(1);
        engine.config.max_duration_seconds = 300;
        let timeout_wav = audio::encode(&vec![0.; audio::RATE * 2 * 60], &c).unwrap();
        assert!(matches!(
            engine.process(
                Input {
                    pcm_wav: &timeout_wav,
                    expected_pcm_sha256: Sha256::digest(&timeout_wav).into(),
                    attachment: ()
                },
                &staging,
                &c
            ),
            Err(Error::Deadline)
        ));
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
        engine.config.task_timeout = Config::default().task_timeout;
        engine.config.max_duration_seconds = 2;
        let attachment = Box::new("real parent generation/authority object remains opaque");
        let out = engine
            .process(
                Input {
                    pcm_wav: &wav,
                    expected_pcm_sha256: hash,
                    attachment,
                },
                &staging,
                &c,
            )
            .unwrap();
        assert!(out.files.directory_name().starts_with("processed-"));
        assert_eq!(
            *out.attachment,
            "real parent generation/authority object remains opaque"
        );
        assert_eq!(out.provenance.processing, "modelSeparatedStems");
        for stem in [crate::Stem::LeadVocal, crate::Stem::Accompaniment] {
            use std::io::Read;
            let mut bytes = Vec::new();
            out.files
                .open_stem(stem)
                .unwrap()
                .read_to_end(&mut bytes)
                .unwrap();
            assert_eq!(audio::validate(&bytes, 2).unwrap(), audio::RATE);
            assert!(audio::samples(&bytes).iter().all(|v| v.is_finite()));
        }
        out.files.remove().unwrap();
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
        drop(engine);
        // Releasing the owner on success/failure permits a subsequent real engine.
        drop(Engine::load_official_archive(&archive, Config::default(), &c).unwrap());
    }
}
