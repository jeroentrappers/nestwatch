// SPDX-License-Identifier: AGPL-3.0-or-later
//! The station side of the relay (rendezvous.md §2 FR-RV-1/2, §3.2, §4.1,
//! §4.2, §5): the persisted mailbox identity, the registration that runs
//! alongside the TCP and WebSocket servers, and, per listener request, a
//! relayed session plus an attempt at the direct QUIC path.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use nestwatch_proto::RelayRef;
use nestwatch_relay::direct::{PunchRole, punch_direct};
use nestwatch_relay::{
    Candidate, IncomingRequest, MailboxIdentity, RegistrationConfig, RegistrationStatus,
    RelayConfig, RelayServer, StationRegistration,
};
use tokio::sync::watch;

use super::{Inner, Shared, StationError, session, stopped};
use crate::storage::{Storage, StorageError};
use crate::types::Transport;

pub use nestwatch_relay::DEFAULT_SERVER;

/// Storage key of the mailbox identity (OPEN-QUESTIONS RELAY-2: the 53-byte
/// `MailboxIdentity` form, kept with the other key material).
pub const KEY_RELAY_IDENTITY: &str = "relay-mailbox";

/// Relay tunables of a station (rendezvous.md §4, §5). The server itself
/// and whether the relay is on are user settings
/// ([`StationSettings`](super::StationSettings)).
#[derive(Debug, Clone)]
pub struct RelayOptions {
    /// STUN port instead of 3478 (tests, unusual deployments; §5.1).
    pub stun_port: Option<u16>,
    /// Local address of each per-request punch socket (§5.2).
    pub punch_bind: SocketAddr,
    /// Offer and accept direct paths (§5). Off: relayed only.
    pub direct: bool,
    /// Time allowed per server request (WebSocket open, answers).
    pub timeout: Duration,
    /// Time allowed for the STUN Binding (§5.1).
    pub stun_timeout: Duration,
    /// First registration retry delay; doubles up to `backoff_max`.
    pub backoff_min: Duration,
    /// Longest registration retry delay.
    pub backoff_max: Duration,
}

impl Default for RelayOptions {
    fn default() -> Self {
        Self {
            stun_port: None,
            punch_bind: SocketAddr::from(([0, 0, 0, 0], 0)),
            direct: true,
            timeout: Duration::from_secs(10),
            stun_timeout: Duration::from_millis(1500),
            backoff_min: Duration::from_secs(1),
            backoff_max: Duration::from_secs(60),
        }
    }
}

/// Registration state for the station UI (FR-RV-1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelayStatus {
    /// The relay setting is off.
    Off,
    /// Opening the control connection.
    Connecting,
    /// Registered: listeners can reach the station through the server.
    Online {
        /// The station's public address as the server sees it.
        observed: String,
    },
    /// The last attempt failed.
    Error {
        /// What went wrong, for display.
        message: String,
        /// The server refused for good (`mailbox-taken`, `version`); only a
        /// new relay identity or another server helps.
        permanent: bool,
    },
}

/// The running registration task.
pub(crate) struct RelayTask {
    stop: watch::Sender<bool>,
}

impl Drop for RelayTask {
    fn drop(&mut self) {
        let _ = self.stop.send(true);
    }
}

/// Loads the stored mailbox identity.
pub(crate) fn load_identity(
    storage: &dyn Storage,
) -> Result<Option<MailboxIdentity>, StorageError> {
    storage
        .get(KEY_RELAY_IDENTITY)?
        .map(|b| {
            MailboxIdentity::from_bytes(&b).map_err(|e| StorageError::Corrupt {
                key: KEY_RELAY_IDENTITY.into(),
                why: e.to_string(),
            })
        })
        .transpose()
}

fn save_identity(storage: &dyn Storage, id: &MailboxIdentity) -> Result<(), StorageError> {
    storage.put(KEY_RELAY_IDENTITY, &id.to_bytes())
}

/// Validates a server address (FR-RV-6) and returns its normalised origin.
pub(crate) fn normalize_server(origin: &str) -> Result<String, StationError> {
    RelayServer::parse(origin)
        .map(|s| s.origin().to_owned())
        .map_err(|e| StationError::Invalid(e.to_string()))
}

impl Shared {
    /// The relay parameters for invitations while the relay is on (§3.2).
    pub(crate) fn relay_ref(&self, g: &Inner) -> Option<RelayRef> {
        if !g.settings.relay_enabled {
            return None;
        }
        let id = g.relay_identity.as_ref()?;
        Some(RelayRef {
            server: g.settings.relay_server.clone(),
            mailbox: *id.id().as_bytes(),
        })
    }

    /// Makes the registration match the settings: stops a running one and,
    /// while the relay is on, starts a new one with the current server and
    /// identity (created and stored on first use, §3.2).
    pub(crate) fn sync_relay(self: &Arc<Self>, g: &mut Inner) -> Result<(), StationError> {
        g.relay_task = None;
        g.relay_generation += 1;
        if !g.running || !g.settings.relay_enabled {
            g.relay_status = RelayStatus::Off;
            return Ok(());
        }
        let identity = match &g.relay_identity {
            Some(i) => i.clone(),
            None => {
                let i = MailboxIdentity::generate().map_err(|_| StationError::Rng)?;
                save_identity(&*self.storage, &i)?;
                g.relay_identity = Some(i.clone());
                i
            }
        };
        let server = RelayServer::parse(&g.settings.relay_server)
            .map_err(|e| StationError::Invalid(e.to_string()))?;
        let (stop, stop_rx) = watch::channel(false);
        g.relay_task = Some(RelayTask { stop });
        g.relay_status = RelayStatus::Connecting;
        tokio::spawn(run(
            self.clone(),
            server,
            identity,
            stop_rx,
            g.relay_generation,
        ));
        Ok(())
    }

    /// "Reset relay identity" (§3.2): a new mailbox; listeners need a new
    /// invitation.
    pub(crate) fn reset_relay_identity(
        self: &Arc<Self>,
        g: &mut Inner,
    ) -> Result<(), StationError> {
        let next = match &g.relay_identity {
            Some(i) => i.reset(),
            None => MailboxIdentity::generate(),
        }
        .map_err(|_| StationError::Rng)?;
        save_identity(&*self.storage, &next)?;
        g.relay_identity = Some(next);
        self.sync_relay(g)
    }

    fn set_relay_status(&self, generation: u64, status: RelayStatus) {
        let mut g = self.lock();
        if g.relay_generation == generation && g.relay_status != status {
            g.relay_status = status;
            self.publish(&g);
        }
    }

    fn relay_config(&self, server: RelayServer) -> RelayConfig {
        let o = &self.cfg.relay;
        let server = match o.stun_port {
            Some(p) => server.with_stun_port(p),
            None => server,
        };
        let mut c = RelayConfig::new(server);
        c.timeout = o.timeout;
        c
    }
}

fn status_of(s: RegistrationStatus) -> Option<RelayStatus> {
    Some(match s {
        RegistrationStatus::Connecting => RelayStatus::Connecting,
        RegistrationStatus::Registered { observed } => RelayStatus::Online { observed },
        RegistrationStatus::Retrying { error, .. } => RelayStatus::Error {
            message: error,
            permanent: false,
        },
        RegistrationStatus::Rejected(code) => RelayStatus::Error {
            message: code.to_string(),
            permanent: true,
        },
        RegistrationStatus::Stopped => return None,
    })
}

/// The registration task: status updates and one task per listener request.
async fn run(
    shared: Arc<Shared>,
    server: RelayServer,
    identity: MailboxIdentity,
    mut stop: watch::Receiver<bool>,
    generation: u64,
) {
    let relay = shared.relay_config(server);
    let mut cfg = RegistrationConfig::new(relay.clone());
    cfg.backoff_min = shared.cfg.relay.backoff_min;
    cfg.backoff_max = shared.cfg.relay.backoff_max;
    let mut reg = StationRegistration::start(cfg, identity);
    let mut status = reg.status();
    let mut shutdown = shared.shutdown.subscribe();
    let relay = Arc::new(relay);
    loop {
        tokio::select! {
            r = reg.next_request() => match r {
                Some(req) => {
                    tokio::spawn(serve(shared.clone(), relay.clone(), req));
                }
                None => break,
            },
            c = status.changed() => {
                if c.is_err() {
                    break;
                }
                let s = status.borrow_and_update().clone();
                if let Some(s) = status_of(s) {
                    shared.set_relay_status(generation, s);
                }
            }
            () = stopped(&mut stop) => break,
            () = stopped(&mut shutdown) => break,
        }
    }
    reg.shutdown().await;
}

/// The listener's address as the server saw it (informational, RELAY-6);
/// used for the session list and the pairing rate limit, since the relay
/// pipe's own peer is the server.
fn observed_addr(observed: Option<&str>) -> Option<SocketAddr> {
    let o = observed?;
    o.parse::<SocketAddr>()
        .ok()
        .or_else(|| o.parse::<IpAddr>().ok().map(|ip| SocketAddr::new(ip, 0)))
}

/// One listener request (§4.2): STUN from a fresh punch socket, `accept`
/// with its server-reflexive and local candidates (plus the station's TCP
/// endpoints), serve the relay pipe as a session, and punch the direct path
/// in parallel (§5.2), serving it as another session once it forms.
async fn serve(shared: Arc<Shared>, relay: Arc<RelayConfig>, req: IncomingRequest) {
    let opts = shared.cfg.relay.clone();
    let wants_direct = opts.direct && req.peer().candidates.iter().any(|c| c.udp().is_some());
    let (socket, mut candidates) = match wants_direct {
        true => crate::punch::punch_socket(opts.punch_bind, &relay.server, opts.stun_timeout)
            .await
            .map_or((None, Vec::new()), |(s, c)| (Some(s), c)),
        false => (None, Vec::new()),
    };
    let endpoints = shared.lock().endpoints.clone();
    candidates.extend(endpoints.into_iter().map(Candidate::Tcp));
    candidates.truncate(crate::punch::MAX_CANDIDATES);
    let Ok(incoming) = req.accept(&candidates).await else {
        return;
    };
    let peer = observed_addr(incoming.peer.observed.as_deref());
    tokio::spawn(session::run_as(
        shared.clone(),
        incoming.link,
        Transport::Relayed,
        peer,
    ));
    if let Some(sock) = socket
        && let Some(link) = punch_direct(sock, &incoming.peer.candidates, PunchRole::Station).await
    {
        let peer = link.info().peer;
        session::run_as(shared, link, Transport::Direct, peer).await;
    }
}
