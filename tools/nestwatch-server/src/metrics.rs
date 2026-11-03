// SPDX-License-Identifier: AGPL-3.0-or-later
//! Aggregate counters (rendezvous.md §6): the only thing the server logs or
//! reports. No identifiers, addresses or contents.

use std::sync::atomic::{AtomicU64, Ordering};

use serde::Serialize;

/// Live counters.
#[derive(Debug, Default)]
pub struct Metrics {
    pub(crate) registrations: AtomicU64,
    pub(crate) registrations_refused: AtomicU64,
    pub(crate) connects: AtomicU64,
    pub(crate) connects_refused: AtomicU64,
    pub(crate) pipes: AtomicU64,
    pub(crate) pipes_active: AtomicU64,
    pub(crate) mailboxes_online: AtomicU64,
    pub(crate) bytes_relayed: AtomicU64,
    pub(crate) rate_limited: AtomicU64,
    pub(crate) sessions_expired: AtomicU64,
    pub(crate) protocol_errors: AtomicU64,
    pub(crate) stun_requests: AtomicU64,
}

/// A copy of the counters, as served at `/healthz`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Snapshot {
    /// Successful registrations since start.
    pub registrations_total: u64,
    /// Refused registrations (taken, rate-limited, version, ...).
    pub registrations_refused_total: u64,
    /// `connect` requests since start.
    pub connects_total: u64,
    /// Refused `connect`s (offline, limits, rate, ...).
    pub connects_refused_total: u64,
    /// Relay pipes opened since start.
    pub pipes_total: u64,
    /// Relay pipes open now.
    pub pipes_active: u64,
    /// Mailboxes with a live control connection now.
    pub mailboxes_online: u64,
    /// Bytes forwarded through pipes since start.
    pub bytes_relayed_total: u64,
    /// Requests refused by rate limits since start.
    pub rate_limited_total: u64,
    /// Sessions the station did not accept in time.
    pub sessions_expired_total: u64,
    /// Connections closed for malformed NWR/1.
    pub protocol_errors_total: u64,
    /// STUN Binding requests answered.
    pub stun_requests_total: u64,
}

pub(crate) fn inc(c: &AtomicU64) {
    c.fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn dec(c: &AtomicU64) {
    let mut v = c.load(Ordering::Relaxed);
    while v > 0 {
        match c.compare_exchange_weak(v, v - 1, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return,
            Err(now) => v = now,
        }
    }
}

impl Metrics {
    /// Reads every counter.
    pub fn snapshot(&self) -> Snapshot {
        let r = |c: &AtomicU64| c.load(Ordering::Relaxed);
        Snapshot {
            registrations_total: r(&self.registrations),
            registrations_refused_total: r(&self.registrations_refused),
            connects_total: r(&self.connects),
            connects_refused_total: r(&self.connects_refused),
            pipes_total: r(&self.pipes),
            pipes_active: r(&self.pipes_active),
            mailboxes_online: r(&self.mailboxes_online),
            bytes_relayed_total: r(&self.bytes_relayed),
            rate_limited_total: r(&self.rate_limited),
            sessions_expired_total: r(&self.sessions_expired),
            protocol_errors_total: r(&self.protocol_errors),
            stun_requests_total: r(&self.stun_requests),
        }
    }
}

impl std::fmt::Display for Snapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "online={} pipes={} pipes_total={} registrations={} (refused {}) connects={} (refused {}) \
             bytes={} rate_limited={} expired={} protocol_errors={} stun={}",
            self.mailboxes_online,
            self.pipes_active,
            self.pipes_total,
            self.registrations_total,
            self.registrations_refused_total,
            self.connects_total,
            self.connects_refused_total,
            self.bytes_relayed_total,
            self.rate_limited_total,
            self.sessions_expired_total,
            self.protocol_errors_total,
            self.stun_requests_total,
        )
    }
}
