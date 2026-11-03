// SPDX-License-Identifier: AGPL-3.0-or-later
//! Rate limiting (rendezvous.md §4.3): token buckets per client address for
//! `register`, `connect` and STUN, and the per-pipe relay rate.
//!
//! Addresses are kept only while their bucket is not full again plus a
//! short grace period (minutes, far below the 24 h of §6), and never logged.
//! IPv6 clients are grouped by /64, since one host usually has a whole
//! prefix.

use std::collections::HashMap;
use std::net::IpAddr;
use std::time::{Duration, Instant};

/// What a bucket limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    /// `register` requests.
    Register,
    /// `connect` requests.
    Connect,
    /// STUN Binding requests.
    Stun,
}

/// Idle buckets are forgotten after this long once full again.
const FORGET_AFTER: Duration = Duration::from_secs(600);

/// The address key: IPv4 as is, IPv6 cut to its /64.
pub fn ip_key(ip: IpAddr) -> IpAddr {
    match ip.to_canonical() {
        IpAddr::V6(v6) => {
            let s = v6.segments();
            IpAddr::from([s[0], s[1], s[2], s[3], 0, 0, 0, 0])
        }
        v4 => v4,
    }
}

#[derive(Debug, Clone, Copy)]
struct Bucket {
    tokens: f64,
    last: Instant,
}

/// Token buckets per address and kind.
#[derive(Debug, Default)]
pub struct RateLimiter {
    buckets: HashMap<(IpAddr, Kind), Bucket>,
}

impl RateLimiter {
    /// Takes one token from the bucket of `(ip, kind)`, which holds `burst`
    /// tokens and refills at `per_second`. Returns whether a token was
    /// available.
    pub fn allow(
        &mut self,
        ip: IpAddr,
        kind: Kind,
        burst: f64,
        per_second: f64,
        now: Instant,
    ) -> bool {
        let b = self.buckets.entry((ip_key(ip), kind)).or_insert(Bucket {
            tokens: burst,
            last: now,
        });
        let elapsed = now.saturating_duration_since(b.last).as_secs_f64();
        b.tokens = (b.tokens + elapsed * per_second).min(burst);
        b.last = now;
        if b.tokens >= 1.0 {
            b.tokens -= 1.0;
            true
        } else {
            false
        }
    }

    /// Forgets buckets idle for a while (their bucket would be full).
    pub fn sweep(&mut self, now: Instant) {
        self.buckets
            .retain(|_, b| now.saturating_duration_since(b.last) < FORGET_AFTER);
    }

    /// Number of tracked addresses (for tests).
    pub fn tracked(&self) -> usize {
        self.buckets.len()
    }
}

/// Byte-rate limiter of one pipe direction: a bucket of one second's worth
/// (at least one maximal message), refilled continuously.
#[derive(Debug)]
pub struct ByteBucket {
    rate: f64,
    capacity: f64,
    tokens: f64,
    last: Instant,
}

impl ByteBucket {
    /// `bits_per_second` rate; `min_burst` bytes must always fit.
    pub fn new(bits_per_second: u64, min_burst: usize, now: Instant) -> Self {
        let rate = (bits_per_second as f64 / 8.0).max(1.0);
        let capacity = rate.max(min_burst as f64);
        Self {
            rate,
            capacity,
            tokens: capacity,
            last: now,
        }
    }

    /// Takes `n` bytes and returns how long to wait before forwarding them
    /// (zero if they fit now). Debt is carried, so the long-run rate holds.
    pub fn take(&mut self, n: usize, now: Instant) -> Duration {
        let elapsed = now.saturating_duration_since(self.last).as_secs_f64();
        self.last = now;
        self.tokens = (self.tokens + elapsed * self.rate).min(self.capacity);
        self.tokens -= n as f64;
        if self.tokens >= 0.0 {
            Duration::ZERO
        } else {
            Duration::from_secs_f64(-self.tokens / self.rate)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buckets_limit_and_refill() {
        let mut r = RateLimiter::default();
        let t0 = Instant::now();
        let ip: IpAddr = "203.0.113.5".parse().unwrap();
        for _ in 0..3 {
            assert!(r.allow(ip, Kind::Connect, 3.0, 0.5, t0));
        }
        assert!(!r.allow(ip, Kind::Connect, 3.0, 0.5, t0));
        // Kinds and addresses are separate.
        assert!(r.allow(ip, Kind::Register, 3.0, 0.5, t0));
        assert!(r.allow("203.0.113.6".parse().unwrap(), Kind::Connect, 3.0, 0.5, t0));
        // 0.5 tokens per second: one more after 2 s.
        let t1 = t0 + Duration::from_secs(2);
        assert!(r.allow(ip, Kind::Connect, 3.0, 0.5, t1));
        assert!(!r.allow(ip, Kind::Connect, 3.0, 0.5, t1));
    }

    #[test]
    fn ipv6_is_grouped_by_prefix() {
        let mut r = RateLimiter::default();
        let t = Instant::now();
        assert!(r.allow(
            "2001:db8:1:2::1".parse().unwrap(),
            Kind::Connect,
            1.0,
            0.0,
            t
        ));
        assert!(!r.allow(
            "2001:db8:1:2::ffff".parse().unwrap(),
            Kind::Connect,
            1.0,
            0.0,
            t
        ));
        assert!(r.allow(
            "2001:db8:1:3::1".parse().unwrap(),
            Kind::Connect,
            1.0,
            0.0,
            t
        ));
        assert_eq!(
            ip_key("::ffff:192.0.2.1".parse().unwrap()),
            "192.0.2.1".parse::<IpAddr>().unwrap()
        );
    }

    #[test]
    fn short_memory() {
        let mut r = RateLimiter::default();
        let t = Instant::now();
        r.allow("192.0.2.1".parse().unwrap(), Kind::Stun, 5.0, 5.0, t);
        r.sweep(t + Duration::from_secs(60));
        assert_eq!(r.tracked(), 1);
        r.sweep(t + FORGET_AFTER);
        assert_eq!(r.tracked(), 0);
    }

    #[test]
    fn byte_bucket_enforces_the_rate() {
        let t0 = Instant::now();
        // 80 kbit/s = 10 000 bytes/s, burst 10 000.
        let mut b = ByteBucket::new(80_000, 1000, t0);
        assert_eq!(b.take(10_000, t0), Duration::ZERO);
        let wait = b.take(5_000, t0);
        assert!((wait.as_secs_f64() - 0.5).abs() < 1e-6, "{wait:?}");
        // After the wait the debt is paid.
        let t1 = t0 + Duration::from_millis(500);
        assert_eq!(b.take(0, t1), Duration::ZERO);
        // A tiny rate still lets one maximal message through.
        let mut slow = ByteBucket::new(8, 65_536, t0);
        assert_eq!(slow.take(65_536, t0), Duration::ZERO);
    }
}
