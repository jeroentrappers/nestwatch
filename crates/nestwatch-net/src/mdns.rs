// SPDX-License-Identifier: AGPL-3.0-or-later
//! DNS-SD over mDNS for `_nestwatch._tcp.local.` (protocol.md §3.4), on the
//! pure-Rust `mdns-sd` responder.
//!
//! mDNS data is never trusted for authentication; it only tells a listener
//! where a station with a given `id` might be.
//!
//! On Android the responder is not built: `mdns-sd` lists interfaces with
//! `getifaddrs`, which Android only has from API 24 (the minimum is 23), and
//! the platform's NSD advertises and browses instead (platforms.md §2.2).
//! [`Advertiser::start`] and [`Browser::start`] then fail; the data types
//! stay so apps can report NSD results.

#[cfg(not(target_os = "android"))]
use std::collections::HashMap;
use std::net::IpAddr;
#[cfg(not(target_os = "android"))]
use std::time::Duration;

#[cfg(not(target_os = "android"))]
use nestwatch_proto::DeviceId;
use nestwatch_proto::StationTxt;
#[cfg(not(target_os = "android"))]
use nestwatch_proto::discovery::SERVICE_TYPE;
#[cfg(not(target_os = "android"))]
use mdns_sd::{Receiver, ResolvedService, ServiceDaemon, ServiceEvent, ServiceInfo};

use crate::NetError;

/// Longest DNS label, the limit for an instance name (RFC 6763 §4.1.1).
#[cfg(not(target_os = "android"))]
const MAX_INSTANCE_LEN: usize = 63;
/// Addresses kept per station at most.
#[cfg(not(target_os = "android"))]
const MAX_ADDRS: usize = 16;
/// How long [`Advertiser::stop`] waits for the goodbye packets.
#[cfg(not(target_os = "android"))]
const STOP_TIMEOUT: Duration = Duration::from_secs(2);

#[cfg(not(target_os = "android"))]
fn mdns_err(e: impl std::fmt::Display) -> NetError {
    NetError::Mdns(e.to_string())
}

/// Truncates a display name to a valid instance label at a char boundary;
/// an empty name becomes `Nestwatch`.
#[cfg(not(target_os = "android"))]
fn instance_name(name: &str) -> String {
    let name = name.trim();
    if name.is_empty() {
        return "Nestwatch".to_owned();
    }
    let mut end = name.len().min(MAX_INSTANCE_LEN);
    while !name.is_char_boundary(end) {
        end -= 1;
    }
    name[..end].to_owned()
}

/// The `.local.` host name used in the SRV record. It is derived from the
/// device ID so it does not reveal the computer's own name.
#[cfg(not(target_os = "android"))]
fn host_name(id: &DeviceId) -> String {
    format!("nestwatch-{id}.local.")
}

/// What a station advertises.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Advertisement {
    /// Instance name: the station's display name (§3.4); at most 63 bytes
    /// are used, and the responder renames on conflict (`Name (2)`).
    pub name: String,
    /// TCP port of the station listener (§3.2).
    pub port: u16,
    /// TXT data (§3.4).
    pub txt: StationTxt,
}

/// Advertises a station while it runs; [`stop`](Self::stop) or dropping
/// withdraws it.
#[cfg(not(target_os = "android"))]
pub struct Advertiser {
    daemon: ServiceDaemon,
    ad: Advertisement,
    fullname: String,
    stopped: bool,
}

#[cfg(not(target_os = "android"))]
fn service_info(ad: &Advertisement) -> Result<ServiceInfo, NetError> {
    let props = ad.txt.to_pairs();
    ServiceInfo::new(
        SERVICE_TYPE,
        &instance_name(&ad.name),
        &host_name(&ad.txt.id),
        (),
        ad.port,
        &props[..],
    )
    .map(ServiceInfo::enable_addr_auto)
    .map_err(mdns_err)
}

#[cfg(not(target_os = "android"))]
impl std::fmt::Debug for Advertiser {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Advertiser")
            .field("ad", &self.ad)
            .field("fullname", &self.fullname)
            .finish_non_exhaustive()
    }
}

#[cfg(not(target_os = "android"))]
impl Advertiser {
    /// Starts a responder and registers the service on all interfaces.
    pub fn start(ad: Advertisement) -> Result<Self, NetError> {
        let daemon = ServiceDaemon::new().map_err(mdns_err)?;
        let info = service_info(&ad)?;
        let fullname = info.get_fullname().to_owned();
        daemon.register(info).map_err(mdns_err)?;
        Ok(Self {
            daemon,
            ad,
            fullname,
            stopped: false,
        })
    }

    /// The registered full name (`Name._nestwatch._tcp.local.`) before any
    /// conflict renaming.
    pub fn fullname(&self) -> &str {
        &self.fullname
    }

    /// Current advertisement.
    pub fn advertisement(&self) -> &Advertisement {
        &self.ad
    }

    /// Replaces the TXT data, e.g. when pairing mode opens or closes, and
    /// re-announces the service.
    pub fn set_txt(&mut self, txt: StationTxt) -> Result<(), NetError> {
        let mut ad = self.ad.clone();
        ad.txt = txt;
        self.daemon.register(service_info(&ad)?).map_err(mdns_err)?;
        self.ad = ad;
        Ok(())
    }

    /// Withdraws the service (goodbye packets) and stops the responder.
    pub async fn stop(mut self) -> Result<(), NetError> {
        self.stopped = true;
        let rx = self.daemon.unregister(&self.fullname).map_err(mdns_err)?;
        let _ = tokio::time::timeout(STOP_TIMEOUT, rx.recv_async()).await;
        let _ = self.daemon.shutdown();
        Ok(())
    }
}

#[cfg(not(target_os = "android"))]
impl Drop for Advertiser {
    fn drop(&mut self) {
        if !self.stopped {
            let _ = self.daemon.unregister(&self.fullname);
            let _ = self.daemon.shutdown();
        }
    }
}

/// A station seen on the network.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredStation {
    /// Full DNS-SD name, the key for updates and removal.
    pub fullname: String,
    /// Instance name (display name), unescaped.
    pub name: String,
    /// SRV target host, e.g. `nestwatch-….local.`.
    pub host: String,
    /// Resolved addresses, IPv4 first, never empty; IPv6 link-local
    /// addresses are left out because endpoints cannot carry a zone
    /// (protocol.md §3.5).
    pub addrs: Vec<IpAddr>,
    /// TCP port of the station listener.
    pub port: u16,
    /// Parsed TXT data.
    pub txt: StationTxt,
}

/// A change in the set of discovered stations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiscoveryEvent {
    /// A station appeared.
    Found(DiscoveredStation),
    /// A known station's TXT, addresses or port changed.
    Updated(DiscoveredStation),
    /// A station went away (or its record became malformed).
    Lost {
        /// Full DNS-SD name as in [`DiscoveredStation::fullname`].
        fullname: String,
    },
}

/// Browses for stations.
#[cfg(not(target_os = "android"))]
pub struct Browser {
    daemon: ServiceDaemon,
    rx: Receiver<ServiceEvent>,
    known: HashMap<String, DiscoveredStation>,
}

#[cfg(not(target_os = "android"))]
fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(n) = chars.next() {
                out.push(n);
            }
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(not(target_os = "android"))]
fn is_link_local_v6(ip: &IpAddr) -> bool {
    matches!(ip, IpAddr::V6(a) if (a.segments()[0] & 0xffc0) == 0xfe80)
}

/// Converts a resolved record; malformed TXT data yields `None`.
#[cfg(not(target_os = "android"))]
fn parse(r: &ResolvedService) -> Option<DiscoveredStation> {
    let txt =
        StationTxt::from_pairs(r.get_properties().iter().map(|p| (p.key(), p.val_str()))).ok()?;
    let mut addrs: Vec<IpAddr> = r
        .get_addresses()
        .iter()
        .map(|a| a.to_ip_addr())
        .filter(|a| !is_link_local_v6(a))
        .collect();
    addrs.sort_by_key(|a| (a.is_ipv6(), *a));
    let suffix = format!(".{SERVICE_TYPE}");
    let fullname = r.get_fullname().to_owned();
    let name = unescape(fullname.strip_suffix(&suffix).unwrap_or(&fullname));
    Some(DiscoveredStation {
        fullname,
        name,
        host: r.get_hostname().to_owned(),
        addrs,
        port: r.get_port(),
        txt,
    })
}

#[cfg(not(target_os = "android"))]
impl std::fmt::Debug for Browser {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Browser")
            .field("known", &self.known.keys())
            .finish_non_exhaustive()
    }
}

#[cfg(not(target_os = "android"))]
impl Browser {
    /// Starts a responder and browses `_nestwatch._tcp.local.`.
    pub fn start() -> Result<Self, NetError> {
        let daemon = ServiceDaemon::new().map_err(mdns_err)?;
        let rx = daemon.browse(SERVICE_TYPE).map_err(mdns_err)?;
        Ok(Self {
            daemon,
            rx,
            known: HashMap::new(),
        })
    }

    /// Next discovery event; `None` once the responder has stopped.
    /// Malformed records are skipped, repeated identical resolutions are
    /// not reported, and addresses learned from successive resolutions
    /// (often one interface at a time) are merged. Cancel-safe.
    pub async fn next(&mut self) -> Option<DiscoveryEvent> {
        loop {
            let ev = self.rx.recv_async().await.ok()?;
            match ev {
                ServiceEvent::ServiceResolved(r) => match parse(&r) {
                    // Not dialable yet (only link-local IPv6 so far); wait
                    // for the next resolution.
                    Some(s) if s.addrs.is_empty() => {}
                    Some(mut s) => match self.known.get_mut(&s.fullname) {
                        None => {
                            self.known.insert(s.fullname.clone(), s.clone());
                            return Some(DiscoveryEvent::Found(s));
                        }
                        Some(prev) => {
                            for a in &prev.addrs {
                                if !s.addrs.contains(a) && s.addrs.len() < MAX_ADDRS {
                                    s.addrs.push(*a);
                                }
                            }
                            s.addrs.sort_by_key(|a| (a.is_ipv6(), *a));
                            if *prev != s {
                                *prev = s.clone();
                                return Some(DiscoveryEvent::Updated(s));
                            }
                        }
                    },
                    None => {
                        if self.known.remove(r.get_fullname()).is_some() {
                            return Some(DiscoveryEvent::Lost {
                                fullname: r.get_fullname().to_owned(),
                            });
                        }
                    }
                },
                ServiceEvent::ServiceRemoved(_, fullname)
                    if self.known.remove(&fullname).is_some() =>
                {
                    return Some(DiscoveryEvent::Lost { fullname });
                }
                _ => {}
            }
        }
    }

    /// Stops browsing and the responder.
    pub fn stop(self) {}
}

#[cfg(not(target_os = "android"))]
impl Drop for Browser {
    fn drop(&mut self) {
        let _ = self.daemon.stop_browse(SERVICE_TYPE);
        let _ = self.daemon.shutdown();
    }
}

#[cfg(all(test, not(target_os = "android")))]
mod tests {
    use super::*;

    #[test]
    fn instance_names() {
        assert_eq!(instance_name("  "), "Nestwatch");
        assert_eq!(instance_name("Nursery"), "Nursery");
        let long = "é".repeat(40);
        let got = instance_name(&long);
        assert!(got.len() <= 63 && got.chars().all(|c| c == 'é'));
        assert_eq!(unescape(r"Baby\.room \\ 1"), r"Baby.room \ 1");
    }
}

#[cfg(target_os = "android")]
fn unsupported() -> NetError {
    NetError::Mdns("mDNS goes through the platform's NSD on Android".into())
}

/// Advertises a station (not available on Android; see the module docs).
#[cfg(target_os = "android")]
#[derive(Debug)]
pub struct Advertiser {
    ad: Advertisement,
}

#[cfg(target_os = "android")]
impl Advertiser {
    /// Always fails on Android.
    pub fn start(_ad: Advertisement) -> Result<Self, NetError> {
        Err(unsupported())
    }

    /// The full DNS-SD name.
    pub fn fullname(&self) -> &str {
        ""
    }

    /// What is advertised.
    pub fn advertisement(&self) -> &Advertisement {
        &self.ad
    }

    /// Always fails on Android.
    pub fn set_txt(&mut self, _txt: StationTxt) -> Result<(), NetError> {
        Err(unsupported())
    }

    /// Withdraws the advertisement.
    pub async fn stop(self) -> Result<(), NetError> {
        Ok(())
    }
}

/// Browses for stations (not available on Android; see the module docs).
#[cfg(target_os = "android")]
#[derive(Debug)]
pub struct Browser {
    _private: (),
}

#[cfg(target_os = "android")]
impl Browser {
    /// Always fails on Android.
    pub fn start() -> Result<Self, NetError> {
        Err(unsupported())
    }

    /// Never yields on Android.
    pub async fn next(&mut self) -> Option<DiscoveryEvent> {
        None
    }

    /// Stops browsing.
    pub fn stop(self) {}
}
