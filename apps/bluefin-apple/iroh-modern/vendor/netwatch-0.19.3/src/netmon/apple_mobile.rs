//! Bounded foreground polling using the existing public interface snapshot.
//! No routing socket, sysctl routing ABI, background service or private binding.
use super::actor::NetworkMessage;
use n0_error::stack_error;
use n0_future::{
    task::AbortOnDropHandle,
    time::{self, Duration},
};
use tokio::sync::mpsc;
#[stack_error(derive, add_meta)]
pub struct Error;
#[derive(Debug)]
pub(super) struct RouteMonitor {
    _handle: AbortOnDropHandle<()>,
}
impl RouteMonitor {
    pub(super) fn new(sender: mpsc::Sender<NetworkMessage>) -> Result<Self, Error> {
        let handle = tokio::spawn(async move {
            let mut interval = time::interval(Duration::from_secs(5));
            interval.set_missed_tick_behavior(time::MissedTickBehavior::Skip);
            loop {
                interval.tick().await;
                if sender.send(NetworkMessage::Change).await.is_err() {
                    break;
                }
            }
        });
        Ok(Self {
            _handle: AbortOnDropHandle::new(handle),
        })
    }
}
