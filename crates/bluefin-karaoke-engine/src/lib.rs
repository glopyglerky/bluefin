//! Bounded server-owned source separation. See the integration contract in docs.
mod audio;
mod checkpoint;
mod engine;
mod model;
mod staging;
pub use engine::{Config, Engine, Input, Processed, Provenance, StemMetadata};
pub use staging::{PublishedTask, StagingRoot, Stem};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("unsupported or corrupt official model")]
    InvalidModel,
    #[error("unsupported, oversized, incomplete or corrupt PCM input")]
    InvalidInput,
    #[error("non-finite, oversized or incomplete model output")]
    InvalidOutput,
    #[error("engine already exists in this process; use the existing owner")]
    Busy,
    #[error("invalid trusted runtime bounds")]
    InvalidConfig,
    #[error(
        "set RAYON_NUM_THREADS and CANDLE_NUM_THREADS to Config::cpu_threads before starting the process; existing Candle pools must match"
    )]
    ThreadConfig,
    #[error("staging must be an opened, exclusively server-owned private directory")]
    InvalidStaging,
    #[error("admitted PCM source digest mismatch")]
    SourceHash,
    #[error("operation exceeded its deadline")]
    Deadline,
    #[error("serialization failed")]
    Serialization(#[from] serde_json::Error),
    #[error("{cause}; task cleanup also failed: {cleanup}")]
    Cleanup {
        cause: Box<Error>,
        cleanup: std::io::Error,
    },
    #[error("operation cancelled")]
    Cancelled,
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Platform(#[from] rustix::io::Errno),
    #[error(transparent)]
    Tensor(#[from] candle_core::Error),
}
#[derive(Clone, Default)]
pub struct Cancellation(Arc<AtomicBool>, Option<std::time::Instant>);
impl Cancellation {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }
    pub fn check(&self) -> Result<(), Error> {
        if self.0.load(Ordering::Acquire) {
            Err(Error::Cancelled)
        } else if self.1.is_some_and(|d| std::time::Instant::now() >= d) {
            Err(Error::Deadline)
        } else {
            Ok(())
        }
    }
    fn bounded(&self, duration: std::time::Duration) -> Self {
        Self(self.0.clone(), Some(std::time::Instant::now() + duration))
    }
}
#[cfg(feature = "qualification")]
pub mod qualification;
