// SPDX-License-Identifier: AGPL-3.0-or-later
//! Sleep inhibition while a station runs (platforms.md §2.4).
//!
//! [`PowerGuard::inhibit_sleep`] returns a [`SleepInhibition`]; the system
//! may sleep again once it is dropped. The display is allowed to turn off.

use std::fmt;

use crate::PlatformError;

#[cfg(feature = "power")]
pub mod keepawake_backend;
#[cfg(target_os = "linux")]
pub mod systemd_inhibit;

/// An active sleep inhibition, released on drop.
pub struct SleepInhibition {
    backend: &'static str,
    _inner: Box<dyn Send>,
}

impl SleepInhibition {
    /// Wraps a backend handle whose drop releases the inhibition.
    pub fn new(backend: &'static str, handle: impl Send + 'static) -> Self {
        Self {
            backend,
            _inner: Box::new(handle),
        }
    }

    /// Backend that holds the inhibition.
    pub fn backend(&self) -> &'static str {
        self.backend
    }
}

impl fmt::Debug for SleepInhibition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SleepInhibition({})", self.backend)
    }
}

/// Prevents system sleep.
pub trait PowerGuard: Send + Sync {
    /// Backend name for diagnostics.
    fn backend(&self) -> &'static str;
    /// Inhibits idle and explicit sleep until the result is dropped.
    /// `reason` is shown by OS tools listing inhibitors.
    fn inhibit_sleep(&self, reason: &str) -> Result<SleepInhibition, PlatformError>;
}

/// Tries several guards in order and uses the first that succeeds.
pub struct FallbackPowerGuard {
    guards: Vec<Box<dyn PowerGuard>>,
}

impl fmt::Debug for FallbackPowerGuard {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list()
            .entries(self.guards.iter().map(|g| g.backend()))
            .finish()
    }
}

impl FallbackPowerGuard {
    /// Uses `guards` in order.
    pub fn new(guards: Vec<Box<dyn PowerGuard>>) -> Self {
        Self { guards }
    }
}

impl PowerGuard for FallbackPowerGuard {
    fn backend(&self) -> &'static str {
        self.guards.first().map_or("none", |g| g.backend())
    }

    fn inhibit_sleep(&self, reason: &str) -> Result<SleepInhibition, PlatformError> {
        let mut last = PlatformError::Unsupported("no sleep inhibitor available");
        for g in &self.guards {
            match g.inhibit_sleep(reason) {
                Ok(i) => return Ok(i),
                Err(e) => {
                    log::debug!("sleep inhibitor {} failed: {e}", g.backend());
                    last = e;
                }
            }
        }
        Err(last)
    }
}

/// Always fails: the platform cannot keep the system awake. Callers warn
/// the user (product.md FR-ST-5).
#[derive(Debug, Clone, Copy, Default)]
pub struct NoPowerGuard;

impl PowerGuard for NoPowerGuard {
    fn backend(&self) -> &'static str {
        "none"
    }

    fn inhibit_sleep(&self, _: &str) -> Result<SleepInhibition, PlatformError> {
        Err(PlatformError::Unsupported("no sleep inhibitor available"))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use super::*;

    struct Flag(Arc<AtomicBool>);
    impl Drop for Flag {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    struct Fake(Arc<AtomicBool>);
    impl PowerGuard for Fake {
        fn backend(&self) -> &'static str {
            "fake"
        }
        fn inhibit_sleep(&self, _: &str) -> Result<SleepInhibition, PlatformError> {
            Ok(SleepInhibition::new("fake", Flag(Arc::clone(&self.0))))
        }
    }

    #[test]
    fn released_on_drop_and_fallback_order() {
        let released = Arc::new(AtomicBool::new(false));
        let g = FallbackPowerGuard::new(vec![
            Box::new(NoPowerGuard),
            Box::new(Fake(Arc::clone(&released))),
        ]);
        let i = g.inhibit_sleep("test").unwrap();
        assert_eq!(i.backend(), "fake");
        assert!(!released.load(Ordering::SeqCst));
        drop(i);
        assert!(released.load(Ordering::SeqCst));
        assert!(FallbackPowerGuard::new(vec![]).inhibit_sleep("x").is_err());
    }
}
