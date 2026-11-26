//! In-process native playback. No browser companion or external media process.
use crate::{
    bridge::Bridge,
    mpv::{Command, Decoder, Frame, Snapshot, StartupError},
    playback::Plan,
    protocol::{Result, Session},
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
pub struct Player {
    bridge: Bridge,
    decoder: Decoder,
}
impl Player {
    pub fn start(
        session: Session,
        plan: Plan,
        peer: Option<(String, std::net::SocketAddr)>,
    ) -> Result<Self> {
        Self::start_with_cancellation(session, plan, peer, Arc::new(AtomicBool::new(false)))
            .map_err(|error| error.message)
    }
    pub fn start_with_cancellation(
        session: Session,
        plan: Plan,
        peer: Option<(String, std::net::SocketAddr)>,
        canceled: Arc<AtomicBool>,
    ) -> std::result::Result<Self, StartupError> {
        if canceled.load(Ordering::Acquire) {
            return Err(StartupError {
                message: "Playback preparation canceled".into(),
                retired: true,
            });
        }
        let bridge =
            Bridge::start(session, plan.clone(), peer).map_err(|message| StartupError {
                message,
                retired: false,
            })?;
        let decoder = Decoder::start_with_cancellation(
            bridge.url.clone(),
            (plan.selection.position - plan.offset).max(0.0),
            std::env::var_os("BLUEFIN_TEST_NULL_AUDIO").is_some(),
            plan.hls,
            canceled,
        );
        let decoder = match decoder {
            Ok(decoder) => decoder,
            Err(mut error) => {
                bridge.stop();
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(12);
                while !bridge.stopped() && std::time::Instant::now() < deadline {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                error.retired &= bridge.cleanup_receipt().is_ok();
                return Err(error);
            }
        };
        Ok(Self { bridge, decoder })
    }
    pub fn command(&self, command: Command) {
        self.decoder.command(command);
    }
    pub fn snapshot(&self) -> Snapshot {
        self.decoder.snapshot()
    }
    pub fn frame(&self) -> Option<Frame> {
        self.decoder.frame()
    }
    pub fn stop(&self) {
        self.bridge.stop();
        self.decoder.stop();
    }
    pub fn cleanup_receipt(&self) -> Result<()> {
        if !self.decoder.snapshot().stopped {
            return Err("Native decoder retirement is still pending".into());
        }
        self.bridge.cleanup_receipt()
    }
    pub fn stopped(&self) -> bool {
        self.bridge.stopped() && self.decoder.snapshot().stopped
    }
}
impl Drop for Player {
    fn drop(&mut self) {
        self.stop();
    }
}
