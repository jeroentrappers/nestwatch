// SPDX-License-Identifier: AGPL-3.0-or-later
//! The pairing window and pairing rate limits (product.md FR-PAIR-1..3,
//! protocol.md §4, §5.2, §5.3, security.md §4). Pure logic on caller time.

use std::collections::HashMap;
use std::net::IpAddr;

use lull_crypto::{InviteToken, Psk};

/// Why a pairing window ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PairingEnd {
    /// The user closed it.
    Closed,
    /// Its time ran out (10 minutes by default).
    Expired,
    /// The maximum number of pairings happened (8).
    Full,
    /// Too many failed code attempts (5).
    CodeFailures,
    /// The station stopped.
    Shutdown,
}

/// Limits of one window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct WindowLimits {
    pub duration_ms: u64,
    pub max_pairings: u32,
    pub max_code_failures: u32,
}

/// An open pairing window. The code PSK is derived once when it opens
/// (lull-session `StationPolicy::code_psk`), never per connection.
#[derive(Debug)]
pub(crate) struct PairingWindow {
    pub token: InviteToken,
    pub psk: Psk,
    pub opened_ms: u64,
    pub limits: WindowLimits,
    pub pairings: u32,
    pub code_failures: u32,
}

impl PairingWindow {
    pub fn new(token: InviteToken, psk: Psk, now: u64, limits: WindowLimits) -> Self {
        Self {
            token,
            psk,
            opened_ms: now,
            limits,
            pairings: 0,
            code_failures: 0,
        }
    }

    /// Monotonic time at which the window expires.
    pub fn closes_at(&self) -> u64 {
        self.opened_ms + self.limits.duration_ms
    }

    /// Whether the window is over at `now`, and why.
    pub fn ended(&self, now: u64) -> Option<PairingEnd> {
        if self.pairings >= self.limits.max_pairings {
            Some(PairingEnd::Full)
        } else if self.code_failures >= self.limits.max_code_failures {
            Some(PairingEnd::CodeFailures)
        } else if now >= self.closes_at() {
            Some(PairingEnd::Expired)
        } else {
            None
        }
    }

    pub fn record_pairing(&mut self) {
        self.pairings += 1;
    }

    pub fn record_code_failure(&mut self) {
        self.code_failures += 1;
    }
}

/// Failed pairing handshakes from one address that trigger `rate-limited`.
pub(crate) const RATE_LIMIT_FAILURES: usize = 3;
/// Window and back-off of the rate limit (protocol.md §4: retry after 30 s).
pub(crate) const RATE_LIMIT_MS: u64 = 30_000;

/// Per-address rate limit for pairing modes (CORE-3): an address with
/// [`RATE_LIMIT_FAILURES`] failed pairing handshakes within the last 30 s
/// is answered `rate-limited` to `invite` and `code` prefaces. `trusted`
/// mode is never rate limited, so the answer reveals nothing about keys.
#[derive(Debug, Default)]
pub(crate) struct RateLimiter {
    failures: HashMap<IpAddr, Vec<u64>>,
}

impl RateLimiter {
    pub fn record_failure(&mut self, ip: IpAddr, now: u64) {
        let list = self.failures.entry(ip).or_default();
        list.push(now);
        self.prune(now);
    }

    pub fn is_limited(&mut self, ip: IpAddr, now: u64) -> bool {
        self.prune(now);
        self.failures
            .get(&ip)
            .is_some_and(|l| l.len() >= RATE_LIMIT_FAILURES)
    }

    fn prune(&mut self, now: u64) {
        for l in self.failures.values_mut() {
            l.retain(|&t| now.saturating_sub(t) < RATE_LIMIT_MS);
        }
        self.failures.retain(|_, l| !l.is_empty());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(now: u64) -> PairingWindow {
        let token = InviteToken::from_bytes(&[1; 16]);
        // A cheap stand-in for the Argon2id PSK; the window only stores it.
        let psk = token.psk(lull_crypto::DeviceKey::from_secret_bytes(&[2; 32]).public_key());
        let limits = WindowLimits {
            duration_ms: 600_000,
            max_pairings: 8,
            max_code_failures: 5,
        };
        PairingWindow::new(token, psk, now, limits)
    }

    #[test]
    fn fr_pair_3_window_ends_after_ten_minutes() {
        let w = window(1_000);
        assert_eq!(w.ended(600_999), None);
        assert_eq!(w.ended(601_000), Some(PairingEnd::Expired));
    }

    #[test]
    fn fr_pair_3_window_ends_after_eight_pairings() {
        let mut w = window(0);
        for _ in 0..7 {
            w.record_pairing();
        }
        assert_eq!(w.ended(1), None);
        w.record_pairing();
        assert_eq!(w.ended(1), Some(PairingEnd::Full));
    }

    #[test]
    fn protocol_5_3_window_ends_after_five_code_failures() {
        let mut w = window(0);
        for _ in 0..4 {
            w.record_code_failure();
        }
        assert_eq!(w.ended(1), None);
        w.record_code_failure();
        assert_eq!(w.ended(1), Some(PairingEnd::CodeFailures));
    }

    #[test]
    fn protocol_4_rate_limit_per_address_for_30_s() {
        let mut r = RateLimiter::default();
        let a: IpAddr = "192.0.2.1".parse().unwrap();
        let b: IpAddr = "192.0.2.2".parse().unwrap();
        r.record_failure(a, 0);
        r.record_failure(a, 1_000);
        assert!(!r.is_limited(a, 1_000));
        r.record_failure(a, 2_000);
        assert!(r.is_limited(a, 2_000));
        assert!(!r.is_limited(b, 2_000));
        assert!(r.is_limited(a, 29_999));
        assert!(!r.is_limited(a, 30_000));
    }
}
