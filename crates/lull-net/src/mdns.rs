// SPDX-License-Identifier: AGPL-3.0-or-later
//! DNS-SD over mDNS for `_lull._tcp.local.` (protocol.md §3.4), on the
//! pure-Rust `mdns-sd` responder.
//!
//! mDNS data is never trusted for authentication; it only tells a listener
//! where a station with a given `id` might be.

use std::collections::HashMap;
use std::net::IpAddr;
use std::time::Duration;

use lull_proto::discovery::SERVICE_TYPE;
use lull_proto::{DeviceId, StationTxt};
use mdns_sd::{Receiver, ResolvedService, ServiceDaemon, ServiceEvent, ServiceInfo};

use crate::NetError;

/// Longest DNS label, the limit for an instance name (RFC 6763 §4.1.1).
const MAX_INSTANCE_LEN: usize = 63;
/// Addresses kept per station at most.
const MAX_ADDRS: usize = 16;
/// How long [`Advertiser::stop`] waits for the goodbye packets.
const STOP_TIMEOUT: Duration = Duration::from_secs(2);

fn mdns_err(e: impl std::fmt::Display) -> NetError {
    NetError::Mdns(e.to_string())
}

/// Truncates a display name to a valid instance label at a char boundary;
/// an empty name becomes `Lull`.
fn instance_name(name: &str) -> String {
    let name = name.trim();
    if name.is_empty() {
        return "Lull".to_owned();
    }
    let mut end = name.len().min(MAX_INSTANCE_LEN);
    while !name.is_char_boundary(end) {
        end -= 1;
    }
    name[..end].to_owned()
}

/// The `.local.` host name used in the SRV record. It is derived from the
/// device ID so it does not reveal the computer's own name.
fn host_name(id: &DeviceId) -> String {
    format!("lull-{id}.local.")
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
pub struct Advertiser {
    daemon: ServiceDaemon,
    ad: Advertisement,
    fullname: String,
    stopped: bool,
}

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

impl std::fmt::Debug for Advertiser {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Advertiser")
            .field("ad", &self.ad)
            .field("fullname", &self.fullname)
            .finish_non_exhaustive()
    }
}

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

    /// The registered full name (`Name._lull._tcp.local.`) before any
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
    /// SRV target host, e.g. `lull-….local.`.
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
pub struct Browser {
    daemon: ServiceDaemon,
    rx: Receiver<ServiceEvent>,
    known: HashMap<String, DiscoveredStation>,
}

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

fn is_link_local_v6(ip: &IpAddr) -> bool {
    matches!(ip, IpAddr::V6(a) if (a.segments()[0] & 0xffc0) == 0xfe80)
}

/// Converts a resolved record; malformed TXT data yields `None`.
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

impl std::fmt::Debug for Browser {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Browser")
            .field("known", &self.known.keys())
            .finish_non_exhaustive()
    }
}

impl Browser {
    /// Starts a responder and browses `_lull._tcp.local.`.
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

impl Drop for Browser {
    fn drop(&mut self) {
        let _ = self.daemon.stop_browse(SERVICE_TYPE);
        let _ = self.daemon.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instance_names() {
        assert_eq!(instance_name("  "), "Lull");
        assert_eq!(instance_name("Nursery"), "Nursery");
        let long = "é".repeat(40);
        let got = instance_name(&long);
        assert!(got.len() <= 63 && got.chars().all(|c| c == 'é'));
        assert_eq!(unescape(r"Baby\.room \\ 1"), r"Baby.room \ 1");
    }
}
