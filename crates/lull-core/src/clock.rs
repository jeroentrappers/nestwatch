// SPDX-License-Identifier: AGPL-3.0-or-later
//! Time sources (`specs/architecture.md` §3: time is passed in).

use std::sync::atomic::{AtomicU64, Ordering};

use crate::rt::{MaybeSend, MaybeSync};

/// Monotonic time for sessions and media, wall-clock time for dates such
/// as "paired" and "last seen".
pub trait Clock: MaybeSend + MaybeSync {
    /// Monotonic milliseconds since an arbitrary origin.
    fn now_ms(&self) -> u64;
    /// Milliseconds since the Unix epoch.
    fn unix_ms(&self) -> u64;
}

/// The real clock.
#[derive(Debug, Clone)]
pub struct SystemClock {
    #[cfg(feature = "native")]
    origin: std::time::Instant,
}

impl SystemClock {
    /// A clock whose monotonic origin is now.
    pub fn new() -> Self {
        Self {
            #[cfg(feature = "native")]
            origin: std::time::Instant::now(),
        }
    }
}

impl Default for SystemClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for SystemClock {
    #[cfg(feature = "native")]
    fn now_ms(&self) -> u64 {
        // Offset so that 0 is never a valid "now" (sessions treat it as a time).
        1_000 + self.origin.elapsed().as_millis() as u64
    }

    #[cfg(not(feature = "native"))]
    fn now_ms(&self) -> u64 {
        // `performance.now()` is not available in every worker scope;
        // `Date.now()` is monotonic enough for 20 ms media scheduling.
        js_sys::Date::now() as u64
    }

    #[cfg(feature = "native")]
    fn unix_ms(&self) -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }

    #[cfg(not(feature = "native"))]
    fn unix_ms(&self) -> u64 {
        js_sys::Date::now() as u64
    }
}

/// A clock that only moves when told to (tests).
#[derive(Debug, Default)]
pub struct ManualClock {
    now: AtomicU64,
    unix_offset: AtomicU64,
}

impl ManualClock {
    /// Starts at `now_ms` with Unix time equal to `unix_ms` at that point.
    pub fn new(now_ms: u64, unix_ms: u64) -> Self {
        Self {
            now: AtomicU64::new(now_ms),
            unix_offset: AtomicU64::new(unix_ms.saturating_sub(now_ms)),
        }
    }

    /// Moves time forward by `ms`.
    pub fn advance(&self, ms: u64) {
        self.now.fetch_add(ms, Ordering::SeqCst);
    }

    /// Sets the monotonic time (never backwards).
    pub fn set(&self, now_ms: u64) {
        self.now.fetch_max(now_ms, Ordering::SeqCst);
    }
}

impl Clock for ManualClock {
    fn now_ms(&self) -> u64 {
        self.now.load(Ordering::SeqCst)
    }

    fn unix_ms(&self) -> u64 {
        self.now_ms() + self.unix_offset.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_clock_moves_only_forward() {
        let c = ManualClock::new(10, 1_000_010);
        assert_eq!((c.now_ms(), c.unix_ms()), (10, 1_000_010));
        c.advance(5);
        c.set(3);
        assert_eq!((c.now_ms(), c.unix_ms()), (15, 1_000_015));
    }

    #[cfg(feature = "native")]
    #[test]
    fn system_clock_is_monotonic() {
        let c = SystemClock::new();
        let a = c.now_ms();
        assert!(c.now_ms() >= a && a >= 1_000);
        assert!(c.unix_ms() > 1_600_000_000_000);
    }
}
