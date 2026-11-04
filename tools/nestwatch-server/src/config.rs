// SPDX-License-Identifier: AGPL-3.0-or-later
//! Server configuration (rendezvous.md §4.3 limits, §8 deployment). Read
//! from TOML; every field has a default, and the command line and
//! environment override it (see `main.rs`).

use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::PathBuf;
use std::time::Duration;

use ipnet::IpNet;
use serde::Deserialize;

/// Everything the server needs.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Local HTTP address behind the reverse proxy (TLS terminates there).
    pub http: SocketAddr,
    /// UDP address of the STUN service (§5.1); `None` disables it.
    pub stun: Option<SocketAddr>,
    /// Directory of the persistent state (secret hashes, transfer counters).
    pub state_dir: PathBuf,
    /// Landing page, served at `/`.
    pub site_dir: Option<PathBuf>,
    /// Browser listener app (built from `apps/web`), served at `/app/`.
    pub app_dir: Option<PathBuf>,
    /// Downloads, served at `/downloads/`.
    pub downloads_dir: Option<PathBuf>,
    /// Take the client address from `X-Forwarded-For` / `X-Real-IP` when
    /// the TCP peer is one of `trusted_proxies` (the reverse proxy).
    pub trust_proxy: bool,
    /// Peers whose forwarded headers are believed: loopback by default; add
    /// the proxy's network when it runs elsewhere, e.g. in another container.
    pub trusted_proxies: Vec<IpNet>,
    /// Reject WebSocket upgrades whose `Origin` names another host than
    /// `Host` (browsers on other sites); native clients send no `Origin`.
    pub check_origin: bool,
    /// Seconds between aggregate counter log lines; 0 disables them.
    pub log_interval: f64,
    /// Limits of §4.3.
    pub limits: Limits,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            http: SocketAddr::from(([127, 0, 0, 1], 8740)),
            stun: Some(SocketAddr::from(([0, 0, 0, 0], 3478))),
            state_dir: PathBuf::from("state"),
            site_dir: None,
            app_dir: None,
            downloads_dir: None,
            trust_proxy: true,
            trusted_proxies: vec![
                IpNet::new_assert(Ipv4Addr::LOCALHOST.into(), 8).trunc(),
                IpNet::new_assert(Ipv6Addr::LOCALHOST.into(), 128),
            ],
            check_origin: true,
            log_interval: 300.0,
            limits: Limits::default(),
        }
    }
}

/// Abuse and capacity limits (rendezvous.md §4.3). Durations are seconds
/// (fractions allowed), rates are per minute unless stated.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Limits {
    /// Concurrent relay pipes (and pending sessions) per mailbox.
    pub pipes_per_mailbox: usize,
    /// Concurrent relay pipes (and pending sessions) per listener address.
    pub pipes_per_ip: usize,
    /// Relay rate per pipe and direction in bits per second.
    pub pipe_rate_bps: u64,
    /// Monthly transfer budget per mailbox in bytes (both directions).
    pub monthly_bytes_per_mailbox: u64,
    /// A pipe without any message for this long is closed.
    pub idle_timeout: f64,
    /// Pipes are closed after this long; sessions reconnect.
    pub pipe_lifetime: f64,
    /// A session offered in `incoming` must be accepted within this.
    pub session_timeout: f64,
    /// A mailbox stays registered this long after its control connection
    /// ends; `connect`s meanwhile wait for the station to come back.
    pub registration_grace: f64,
    /// A control connection without any frame (pings included) for this
    /// long is dropped.
    pub control_idle_timeout: f64,
    /// Time allowed for the first NWR/1 message on a new connection.
    pub first_message_timeout: f64,
    /// `register` requests per address and minute (burst of the same size).
    pub register_per_minute: u32,
    /// `connect` requests per address and minute (burst of the same size).
    pub connect_per_minute: u32,
    /// STUN requests per address and second.
    pub stun_per_second: u32,
    /// Largest WebSocket message and frame (bytes); larger ones end the
    /// connection.
    pub max_message_bytes: usize,
    /// Concurrent TCP connections at most.
    pub max_connections: usize,
    /// Concurrent WebSocket connections per address.
    pub max_connections_per_ip: usize,
    /// Days after which a mailbox that never registered again is forgotten.
    pub mailbox_retention_days: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            pipes_per_mailbox: 16,
            pipes_per_ip: 4,
            pipe_rate_bps: 2_000_000,
            monthly_bytes_per_mailbox: 50 * 1024 * 1024 * 1024,
            idle_timeout: 60.0,
            pipe_lifetime: 24.0 * 3600.0,
            session_timeout: 30.0,
            registration_grace: 60.0,
            control_idle_timeout: 60.0,
            first_message_timeout: 10.0,
            register_per_minute: 10,
            connect_per_minute: 30,
            stun_per_second: 20,
            max_message_bytes: 64 * 1024,
            max_connections: 4096,
            max_connections_per_ip: 32,
            mailbox_retention_days: 400,
        }
    }
}

/// Seconds as a [`Duration`]; negative or invalid values become zero.
pub(crate) fn secs(s: f64) -> Duration {
    Duration::try_from_secs_f64(s).unwrap_or(Duration::ZERO)
}

impl Config {
    /// Parses TOML.
    pub fn from_toml(text: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toml_overrides_defaults() {
        let c = Config::from_toml(
            r#"
            http = "127.0.0.1:9000"
            state_dir = "/var/lib/nestwatch-server"
            app_dir = "/srv/app"
            [limits]
            pipes_per_ip = 2
            idle_timeout = 5
            session_timeout = 0.5
            "#,
        )
        .unwrap();
        assert_eq!(c.http.port(), 9000);
        assert_eq!(c.stun.unwrap().port(), 3478);
        assert_eq!(c.limits.pipes_per_ip, 2);
        assert_eq!(c.limits.pipes_per_mailbox, 16);
        assert_eq!(secs(c.limits.idle_timeout), Duration::from_secs(5));
        assert_eq!(secs(c.limits.session_timeout), Duration::from_millis(500));
        assert!(Config::from_toml("bogus = 1").is_err());
        assert!(Config::from_toml("[limits]\nbogus = 1").is_err());
    }

    #[test]
    fn defaults_follow_section_4_3() {
        let l = Limits::default();
        assert_eq!((l.pipes_per_mailbox, l.pipes_per_ip), (16, 4));
        assert_eq!(l.pipe_rate_bps, 2_000_000);
        assert_eq!(secs(l.idle_timeout), Duration::from_secs(60));
        assert_eq!(secs(l.pipe_lifetime), Duration::from_secs(86_400));
        assert_eq!(secs(l.session_timeout), Duration::from_secs(30));
        assert_eq!(secs(-1.0), Duration::ZERO);
    }

    #[test]
    fn trusted_proxies_default_to_loopback() {
        let trusted = |c: &Config, ip: &str| {
            let ip: std::net::IpAddr = ip.parse().unwrap();
            c.trusted_proxies.iter().any(|n| n.contains(&ip))
        };
        let c = Config::default();
        assert!(trusted(&c, "127.0.0.1") && trusted(&c, "::1"));
        assert!(!trusted(&c, "172.18.0.3"));
        let c = Config::from_toml(r#"trusted_proxies = ["172.16.0.0/12"]"#).unwrap();
        assert!(trusted(&c, "172.18.0.3") && !trusted(&c, "127.0.0.1"));
    }
}
