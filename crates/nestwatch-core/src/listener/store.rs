// SPDX-License-Identifier: AGPL-3.0-or-later
//! The known station list (protocol.md §2.3, product.md FR-NET-3).

use std::fmt;

use minicbor::{Decode, Encode};
use nestwatch_crypto::PublicKey;
use nestwatch_proto::Endpoint;

use crate::persist;
use crate::storage::{Storage, StorageError};
use crate::types::ListenMode;

/// Storage key of the known station list.
pub const KEY_STATIONS: &str = "listener-stations";
/// Storage key of the listener's own settings.
pub const KEY_LISTENER: &str = "listener-settings";

/// Remembered addresses per station at most.
const MAX_ENDPOINTS: usize = 16;

/// How to reach a station.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Route {
    /// TCP to an endpoint (protocol.md §3.2). Native only.
    Tcp(Endpoint),
    /// A `ws://` or `wss://` URL of the station's `/nestwatch/1` (protocol.md §3.3).
    WebSocket(String),
}

impl Route {
    /// Parses `ws://…`, `wss://…` or `host:port`.
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        let lower = s.to_ascii_lowercase();
        if lower.starts_with("ws://") || lower.starts_with("wss://") {
            Some(Self::WebSocket(s.to_owned()))
        } else {
            Endpoint::parse(s).ok().map(Self::Tcp)
        }
    }

    /// The `ws(s)://host:port/nestwatch/1` URL for a station web address
    /// (`http(s)://host:port/...`).
    pub fn websocket_for_origin(url: &str) -> Option<Self> {
        let (scheme, rest) = url.split_once("://")?;
        let ws = match scheme.to_ascii_lowercase().as_str() {
            "http" => "ws",
            "https" => "wss",
            _ => return None,
        };
        let authority = rest
            .split(['/', '#', '?'])
            .next()
            .filter(|a| !a.is_empty())?;
        Some(Self::WebSocket(format!(
            "{ws}://{authority}{}",
            nestwatch_proto::endpoint::WEBSOCKET_PATH
        )))
    }
}

impl fmt::Display for Route {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Tcp(e) => write!(f, "{e}"),
            Self::WebSocket(u) => f.write_str(u),
        }
    }
}

/// Per-station sound activation settings (FR-AU-2, FR-AU-4).
#[derive(Debug, Clone, Copy, PartialEq, Encode, Decode)]
pub struct ListenSettings {
    /// Continuous or sound activation.
    #[n(0)]
    pub mode: ListenMode,
    /// Threshold in dBFS (`-90..0`).
    #[n(1)]
    pub threshold_dbfs: f32,
    /// Hold time in ms.
    #[n(2)]
    pub hold_ms: u32,
    /// Pre-roll in ms.
    #[n(3)]
    pub preroll_ms: u32,
}

impl Default for ListenSettings {
    /// FR-AU-4: activated, -50 dBFS, 4 s hold, 500 ms pre-roll.
    fn default() -> Self {
        Self {
            mode: ListenMode::Activated,
            threshold_dbfs: -50.0,
            hold_ms: 4_000,
            preroll_ms: 500,
        }
    }
}

impl ListenSettings {
    /// Continuous audio.
    pub fn continuous() -> Self {
        Self {
            mode: ListenMode::Continuous,
            ..Self::default()
        }
    }

    /// Sound activation with the given parameters.
    pub fn activated(threshold_dbfs: f32, hold_ms: u32, preroll_ms: u32) -> Self {
        Self {
            mode: ListenMode::Activated,
            threshold_dbfs,
            hold_ms,
            preroll_ms,
        }
    }

    pub(crate) fn to_wire(self) -> nestwatch_proto::control::Listen {
        nestwatch_proto::control::Listen {
            mode: self.mode.to_wire(),
            threshold: if self.threshold_dbfs.is_nan() {
                -50.0
            } else {
                self.threshold_dbfs.clamp(-90.0, 0.0)
            },
            hold_ms: self.hold_ms.min(nestwatch_proto::control::MAX_HOLD_MS),
            preroll_ms: self
                .preroll_ms
                .min(nestwatch_proto::control::MAX_PREROLL_MS),
        }
    }
}

/// Which alerts a station raises (product.md §3.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub struct AlertToggles {
    /// Sound started (FR-AL-1).
    #[n(0)]
    pub sound: bool,
    /// Unreachable and reachable again (FR-AL-2).
    #[n(1)]
    pub unreachable: bool,
    /// Low battery (FR-AL-3).
    #[n(2)]
    pub battery: bool,
}

impl Default for AlertToggles {
    fn default() -> Self {
        Self {
            sound: true,
            unreachable: true,
            battery: true,
        }
    }
}

/// Per-station settings (FR-LS-1, FR-AU-2, FR-AU-6).
#[derive(Debug, Clone, Copy, PartialEq, Encode, Decode)]
pub struct StationSettings {
    /// Sound activation.
    #[n(0)]
    pub listen: ListenSettings,
    /// Linear playback volume 0..1.
    #[n(1)]
    pub volume: f32,
    /// Muted (alerts keep working, FR-AU-6).
    #[n(2)]
    pub muted: bool,
    /// Alert toggles.
    #[n(3)]
    pub alerts: AlertToggles,
    /// Followed: connected whenever possible.
    #[n(4)]
    pub follow: bool,
}

impl Default for StationSettings {
    fn default() -> Self {
        Self {
            listen: ListenSettings::default(),
            volume: 1.0,
            muted: false,
            alerts: AlertToggles::default(),
            follow: true,
        }
    }
}

/// An address that reached a station, with its last success (FR-NET-3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnownEndpoint {
    /// Where.
    pub route: Route,
    /// Last successful session setup (Unix ms), `None` if never.
    pub last_success_unix_ms: Option<u64>,
}

/// A station this listener is paired with.
#[derive(Debug, Clone, PartialEq)]
pub struct KnownStation {
    /// Static public key.
    pub key: PublicKey,
    /// Device ID.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Known addresses.
    pub endpoints: Vec<KnownEndpoint>,
    /// Settings.
    pub settings: StationSettings,
    /// The station revoked us (protocol.md §7.11).
    pub needs_pairing: bool,
    /// When it was paired (Unix ms).
    pub paired_unix_ms: u64,
}

impl KnownStation {
    /// Adds an endpoint (or refreshes it on `success`).
    pub fn note_endpoint(&mut self, route: &Route, success: Option<u64>) {
        if let Some(e) = self.endpoints.iter_mut().find(|e| &e.route == route) {
            if success.is_some() {
                e.last_success_unix_ms = success;
            }
            return;
        }
        self.endpoints.push(KnownEndpoint {
            route: route.clone(),
            last_success_unix_ms: success,
        });
        if self.endpoints.len() > MAX_ENDPOINTS {
            // Forget the stalest never- or least-recently-successful one.
            if let Some((i, _)) = self
                .endpoints
                .iter()
                .enumerate()
                .min_by_key(|(_, e)| e.last_success_unix_ms)
            {
                self.endpoints.remove(i);
            }
        }
    }

    /// Stored routes, most recently successful first (FR-NET-3); never
    /// successful ones keep their stored order after those.
    pub fn routes_by_success(&self) -> Vec<Route> {
        let mut v: Vec<&KnownEndpoint> = self.endpoints.iter().collect();
        v.sort_by_key(|e| std::cmp::Reverse(e.last_success_unix_ms));
        v.into_iter().map(|e| e.route.clone()).collect()
    }
}

#[derive(Encode, Decode)]
struct EndpointRecord {
    #[n(0)]
    websocket: bool,
    #[n(1)]
    addr: String,
    #[n(2)]
    last_success: Option<u64>,
}

#[derive(Encode, Decode)]
struct StationRecord {
    #[b(0)]
    key: Vec<u8>,
    #[n(1)]
    name: String,
    #[n(2)]
    endpoints: Vec<EndpointRecord>,
    #[n(3)]
    settings: StationSettings,
    #[n(4)]
    needs_pairing: bool,
    #[n(5)]
    paired: u64,
}

#[derive(Encode, Decode)]
struct StationsRecord {
    #[n(0)]
    stations: Vec<StationRecord>,
}

/// The listener's own settings.
#[derive(Debug, Clone, PartialEq, Encode, Decode)]
pub(crate) struct ListenerRecord {
    #[n(0)]
    pub name: String,
}

pub(crate) fn load_stations(storage: &dyn Storage) -> Result<Vec<KnownStation>, StorageError> {
    let Some(rec) = persist::load::<StationsRecord>(storage, KEY_STATIONS)? else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for s in rec.stations {
        let key = persist::public_key(KEY_STATIONS, &s.key)?;
        let endpoints = s
            .endpoints
            .into_iter()
            .filter_map(|e| {
                let route = if e.websocket {
                    Route::WebSocket(e.addr)
                } else {
                    Route::Tcp(Endpoint::parse(&e.addr).ok()?)
                };
                Some(KnownEndpoint {
                    route,
                    last_success_unix_ms: e.last_success,
                })
            })
            .collect();
        out.push(KnownStation {
            id: key.device_id().as_str().to_owned(),
            key,
            name: s.name,
            endpoints,
            settings: s.settings,
            needs_pairing: s.needs_pairing,
            paired_unix_ms: s.paired,
        });
    }
    Ok(out)
}

pub(crate) fn save_stations<'a>(
    storage: &dyn Storage,
    stations: impl IntoIterator<Item = &'a KnownStation>,
) -> Result<(), StorageError> {
    let rec = StationsRecord {
        stations: stations
            .into_iter()
            .map(|s| StationRecord {
                key: s.key.as_bytes().to_vec(),
                name: s.name.clone(),
                endpoints: s
                    .endpoints
                    .iter()
                    .map(|e| match &e.route {
                        Route::Tcp(ep) => EndpointRecord {
                            websocket: false,
                            addr: ep.to_string(),
                            last_success: e.last_success_unix_ms,
                        },
                        Route::WebSocket(u) => EndpointRecord {
                            websocket: true,
                            addr: u.clone(),
                            last_success: e.last_success_unix_ms,
                        },
                    })
                    .collect(),
                settings: s.settings,
                needs_pairing: s.needs_pairing,
                paired: s.paired_unix_ms,
            })
            .collect(),
    };
    persist::save(storage, KEY_STATIONS, &rec)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MemoryStorage;

    fn station() -> KnownStation {
        KnownStation {
            key: *nestwatch_crypto::DeviceKey::from_secret_bytes(&[5; 32]).public_key(),
            id: String::new(),
            name: "Nursery".into(),
            endpoints: Vec::new(),
            settings: StationSettings::default(),
            needs_pairing: false,
            paired_unix_ms: 1,
        }
    }

    #[test]
    fn fr_net_3_routes_ordered_by_last_success() {
        let mut s = station();
        let a = Route::parse("192.168.1.2:47100").unwrap();
        let b = Route::parse("nursery.tail1234.ts.net:47100").unwrap();
        let c = Route::parse("ws://192.168.1.2:47180/nestwatch/1").unwrap();
        s.note_endpoint(&a, None);
        s.note_endpoint(&b, Some(10));
        s.note_endpoint(&c, Some(20));
        assert_eq!(s.routes_by_success(), vec![c.clone(), b.clone(), a.clone()]);
        s.note_endpoint(&a, Some(30));
        assert_eq!(s.routes_by_success()[0], a);
    }

    #[test]
    fn known_stations_roundtrip() {
        let storage = MemoryStorage::new();
        let mut s = station();
        s.id = s.key.device_id().as_str().to_owned();
        s.note_endpoint(&Route::parse("[fd7a:115c:a1e0::1]:47100").unwrap(), Some(5));
        s.note_endpoint(&Route::parse("wss://h:47181/nestwatch/1").unwrap(), None);
        s.settings.listen = ListenSettings::activated(-35.5, 2000, 300);
        s.settings.muted = true;
        save_stations(&storage, [&s]).unwrap();
        assert_eq!(load_stations(&storage).unwrap(), vec![s]);
    }

    #[test]
    fn websocket_route_from_web_address() {
        assert_eq!(
            Route::websocket_for_origin("http://192.168.1.20:47180/#nestwatch%3A%2F%2F"),
            Some(Route::WebSocket(
                "ws://192.168.1.20:47180/nestwatch/1".into()
            ))
        );
        assert_eq!(
            Route::websocket_for_origin("https://[fd7a::1]:47181/"),
            Some(Route::WebSocket("wss://[fd7a::1]:47181/nestwatch/1".into()))
        );
        assert_eq!(Route::websocket_for_origin("nestwatch://invite?v=1"), None);
    }
}
