//! Own a captured source until its actual child has exited, including when an
//! async spawn/open/wait future is canceled or unwinds before registration.
use std::{
    ops::{Deref, DerefMut},
    sync::Arc,
};

use ferrofin_traits::library::MediaSourceManager;

/// Redacts remote credentials and opened-input capabilities from diagnostics.
/// Handles descriptor paths embedded in quoted arguments or error messages.
#[must_use]
pub fn redact_inputs(value: &str) -> std::borrow::Cow<'_, str> {
    let remote = ferrofin_util::source_url::redact_urls_in(value);
    let mut cursor = 0;
    let mut shown = String::new();
    for (position, _) in remote.match_indices("/proc/") {
        if position < cursor {
            continue;
        }
        let rest = &remote[position + "/proc/".len()..];
        let Some((process, descriptor)) = rest.split_once("/fd/") else {
            continue;
        };
        if !(matches!(process, "self" | "thread-self")
            || (!process.is_empty() && process.bytes().all(|b| b.is_ascii_digit())))
        {
            continue;
        }
        let digits = descriptor.bytes().take_while(u8::is_ascii_digit).count();
        if digits == 0 {
            continue;
        }
        shown.push_str(&remote[cursor..position]);
        shown.push_str("<opened-input>");
        cursor = position + "/proc/".len() + process.len() + "/fd/".len() + digits;
    }
    if cursor == 0 {
        remote
    } else {
        shown.push_str(&remote[cursor..]);
        std::borrow::Cow::Owned(shown)
    }
}

/// A child plus the source manager owning its opened input. Retention conveys
/// no permission: callers must authorize before every dispatch.
pub struct RetainedInputProcess {
    child: Option<tokio::process::Child>,
    source: Option<InputProcessOwner>,
}

/// A source binding and the runtime's shared bound on retained children and
/// reapers. This lifetime does not authorize an operation on the source.
#[derive(Clone)]
pub struct InputProcessBinding {
    source: Arc<dyn MediaSourceManager>,
    slots: Arc<tokio::sync::Semaphore>,
}

/// Owned before synchronous spawn, until the actual child has exited.
pub struct InputProcessOwner {
    _source: Arc<dyn MediaSourceManager>,
    _slot: tokio::sync::OwnedSemaphorePermit,
}

impl InputProcessBinding {
    /// Binds one source to the existing runtime's shared process budget.
    #[must_use]
    pub fn new(source: Arc<dyn MediaSourceManager>, slots: Arc<tokio::sync::Semaphore>) -> Self {
        Self { source, slots }
    }

    /// Reserves an actual process/reaper slot before any child can start.
    ///
    /// # Errors
    /// Refuses exhaustion immediately, retaining no queued spawn or new pool.
    pub fn try_acquire(&self) -> Result<InputProcessOwner, String> {
        let slot = Arc::clone(&self.slots)
            .try_acquire_owned()
            .map_err(|_| "opened-input process budget exhausted".to_owned())?;
        Ok(InputProcessOwner {
            _source: Arc::clone(&self.source),
            _slot: slot,
        })
    }
}

impl RetainedInputProcess {
    /// Takes ownership immediately after synchronous spawn, before any await.
    #[must_use]
    pub fn new(child: tokio::process::Child, source: Option<InputProcessOwner>) -> Self {
        Self {
            child: Some(child),
            source,
        }
    }
}

impl Deref for RetainedInputProcess {
    type Target = tokio::process::Child;
    fn deref(&self) -> &Self::Target {
        self.child.as_ref().expect("owned process")
    }
}
impl DerefMut for RetainedInputProcess {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.child.as_mut().expect("owned process")
    }
}

impl Drop for RetainedInputProcess {
    fn drop(&mut self) {
        let Some(source) = self.source.take() else {
            return;
        };
        let mut child = self.child.take().expect("owned process");
        if matches!(child.try_wait(), Ok(Some(_))) {
            return;
        }
        // A tokio task may be canceled during runtime shutdown too. The reap
        // must survive it. Only interrupted scoped children take this path;
        // the runtime's shared process permit bounds these cleanup threads too.
        let _ = child.start_kill();
        let pending = Arc::new(std::sync::Mutex::new(Some((child, source))));
        let reaping = Arc::clone(&pending);
        if std::thread::Builder::new()
            .name("media-input-reap".into())
            .spawn(move || {
                let (mut child, source) = reaping
                    .lock()
                    .expect("input reaper lock")
                    .take()
                    .expect("input reaper owner");
                loop {
                    match child.try_wait() {
                        Ok(Some(_)) => break,
                        Ok(None) => std::thread::sleep(std::time::Duration::from_millis(10)),
                        Err(_) => {
                            // Unable to establish exit. Preserve the capability
                            // owner rather than permit numeric descriptor reuse.
                            std::mem::forget((child, source));
                            tracing::error!("retained input reaper could not establish child exit");
                            break;
                        }
                    }
                }
            })
            .is_err()
        {
            // Thread exhaustion must fail closed too. This consumes a bounded
            // pin until process exit and is an explicit resource failure.
            std::mem::forget(pending);
            tracing::error!("retained input reaper could not start");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::redact_inputs;

    #[test]
    fn descriptor_capabilities_are_redacted_in_quoted_arguments_and_errors() {
        for value in [
            "file:/proc/123/fd/4",
            "input '/proc/self/fd/12' failed",
            "-i \"file:/proc/1/fd/99\" -i /proc/2/fd/3",
        ] {
            let shown = redact_inputs(value);
            assert!(!shown.contains("/proc/"));
            assert!(shown.contains("<opened-input>"));
        }
        assert_eq!(redact_inputs("/media/ordinary.mkv"), "/media/ordinary.mkv");
        assert_eq!(
            redact_inputs("https://account:secret@invalid.example/path?token=secret"),
            "https://invalid.example"
        );
    }
}
