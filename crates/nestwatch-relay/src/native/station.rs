// SPDX-License-Identifier: AGPL-3.0-or-later
//! The station's control connection (rendezvous.md §4.1) and the accept
//! connections it opens for each listener (§4.2).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use nestwatch_net::Link;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use tokio::time::Instant;
use tokio_tungstenite::tungstenite::Message as WsMessage;

use super::ws::{self, Ws};
use crate::candidate::Candidate;
use crate::exchange::{self, PeerInfo};
use crate::msg::{ErrorCode, Message, SessionId};
use crate::registration::{
    End, Incoming, REQUEST_QUEUE, RegistrationConfig, RegistrationStatus, SESSION_VALIDITY, end_for,
};
use crate::{MailboxIdentity, RelayConfig, RelayError};

/// A listener asks to connect (`incoming`, §4.2). Accept it within 30 s.
#[derive(Debug)]
pub struct IncomingRequest {
    session: SessionId,
    peer: PeerInfo,
    relay: Arc<RelayConfig>,
    received: Instant,
}

impl IncomingRequest {
    /// The listener's candidates and observed address.
    pub fn peer(&self) -> &PeerInfo {
        &self.peer
    }

    /// Whether the 30 s session validity has passed.
    pub fn is_expired(&self) -> bool {
        self.received.elapsed() >= SESSION_VALIDITY
    }

    /// Opens the accept connection and sends `accept` with `candidates`
    /// (the station's own, typically from the UDP socket it will punch
    /// with). The returned link is the relay pipe, ready for an NWP/1
    /// station session.
    pub async fn accept(self, candidates: &[Candidate]) -> Result<Incoming, RelayError> {
        if self.is_expired() {
            return Err(RelayError::Server(ErrorCode::Session));
        }
        let (ws, info) = ws::open(&self.relay).await?;
        let mut link = Link::from_websocket(ws, info);
        exchange::station_accept(&mut link, self.session, candidates, self.relay.timeout).await?;
        Ok(Incoming {
            link,
            peer: self.peer,
        })
    }
}

/// Keeps a station registered at a relay server (rendezvous.md §4.1): one
/// control connection with keep-alive and reconnect, and one
/// [`IncomingRequest`] per listener. Dropping it unregisters (the server
/// forgets the registration 60 s later).
#[derive(Debug)]
pub struct StationRegistration {
    status: watch::Receiver<RegistrationStatus>,
    requests: mpsc::Receiver<IncomingRequest>,
    candidates: Arc<Mutex<Vec<Candidate>>>,
    relay: Arc<RelayConfig>,
    stop: watch::Sender<bool>,
    task: Option<JoinHandle<()>>,
}

impl Drop for StationRegistration {
    fn drop(&mut self) {
        if let Some(t) = &self.task {
            t.abort();
        }
    }
}

impl StationRegistration {
    /// Starts registering on the current tokio runtime.
    pub fn start(config: RegistrationConfig, identity: MailboxIdentity) -> Self {
        let (status_tx, status) = watch::channel(RegistrationStatus::Connecting);
        let (req_tx, requests) = mpsc::channel(REQUEST_QUEUE);
        let (stop, stop_rx) = watch::channel(false);
        let relay = Arc::new(config.relay.clone());
        let candidates = Arc::new(Mutex::new(config.candidates.clone()));
        let task = tokio::spawn(control_loop(
            config,
            identity,
            relay.clone(),
            status_tx,
            req_tx,
            stop_rx,
        ));
        Self {
            status,
            requests,
            candidates,
            relay,
            stop,
            task: Some(task),
        }
    }

    /// Subscribes to the registration state.
    pub fn status(&self) -> watch::Receiver<RegistrationStatus> {
        self.status.clone()
    }

    /// The server this registration uses.
    pub fn relay(&self) -> &RelayConfig {
        &self.relay
    }

    /// Replaces the candidates [`next`](Self::next) sends.
    pub fn set_candidates(&self, candidates: Vec<Candidate>) {
        *self.candidates.lock().expect("not poisoned") = candidates;
    }

    /// The next listener request; accept it with your own candidates (for
    /// hole punching). `None` once stopped. Cancel-safe.
    pub async fn next_request(&mut self) -> Option<IncomingRequest> {
        self.requests.recv().await
    }

    /// The next listener, accepted with the configured candidates. Failed
    /// accepts (expired session, network) are skipped. `None` once stopped.
    /// Not cancel-safe while an accept is in progress.
    pub async fn next(&mut self) -> Option<Incoming> {
        loop {
            let req = self.requests.recv().await?;
            let candidates = self.candidates.lock().expect("not poisoned").clone();
            if let Ok(i) = req.accept(&candidates).await {
                return Some(i);
            }
        }
    }

    /// Closes the control connection and ends the task.
    pub async fn shutdown(mut self) {
        let _ = self.stop.send(true);
        if let Some(t) = self.task.take() {
            let _ = t.await;
        }
    }
}

/// Resolves once `stop` is set (the `Ref` of `wait_for` is not `Send`).
async fn stopped(stop: &mut watch::Receiver<bool>) {
    let _ = stop.wait_for(|s| *s).await;
}

async fn control_loop(
    config: RegistrationConfig,
    identity: MailboxIdentity,
    relay: Arc<RelayConfig>,
    status: watch::Sender<RegistrationStatus>,
    requests: mpsc::Sender<IncomingRequest>,
    mut stop: watch::Receiver<bool>,
) {
    let mut backoff = config.backoff_min;
    loop {
        let _ = status.send(RegistrationStatus::Connecting);
        let end = session(
            &config,
            &identity,
            &relay,
            &status,
            &requests,
            &mut stop,
            &mut backoff,
        )
        .await;
        let delay = match end {
            End::Stop => {
                let _ = status.send(RegistrationStatus::Stopped);
                return;
            }
            End::Fatal(code) => {
                let _ = status.send(RegistrationStatus::Rejected(code));
                return;
            }
            End::Retry(error) => {
                let d = backoff;
                backoff = (backoff * 2).min(config.backoff_max);
                (error, d)
            }
            End::Slow(error) => (error, config.backoff_max),
        };
        let _ = status.send(RegistrationStatus::Retrying {
            error: delay.0,
            retry_in: delay.1,
        });
        tokio::select! {
            () = tokio::time::sleep(delay.1) => {}
            () = stopped(&mut stop) => {
                let _ = status.send(RegistrationStatus::Stopped);
                return;
            }
        }
    }
}

async fn send(ws: &mut Ws, m: &Message) -> Result<(), RelayError> {
    let bytes = m.encode()?;
    ws.send(WsMessage::Binary(bytes.into()))
        .await
        .map_err(|e| RelayError::Net(nestwatch_net::NetError::WebSocket(e.to_string())))
}

/// One control connection from open to end.
async fn session(
    config: &RegistrationConfig,
    identity: &MailboxIdentity,
    relay: &Arc<RelayConfig>,
    status: &watch::Sender<RegistrationStatus>,
    requests: &mpsc::Sender<IncomingRequest>,
    stop: &mut watch::Receiver<bool>,
    backoff: &mut Duration,
) -> End {
    let opened = tokio::select! {
        r = ws::open(relay) => r,
        () = stopped(stop) => return End::Stop,
    };
    let mut ws = match opened {
        Ok((ws, _)) => ws,
        Err(e) => return End::Retry(e.to_string()),
    };
    let register = Message::Register {
        mailbox: identity.id(),
        secret: identity.secret(),
    };
    if let Err(e) = send(&mut ws, &register).await {
        return End::Retry(e.to_string());
    }
    let mut registered = false;
    let register_deadline = Instant::now() + relay.timeout;
    let mut last_rx = Instant::now();
    let mut ping =
        tokio::time::interval_at(Instant::now() + config.ping_interval, config.ping_interval);
    loop {
        let dead_at = if registered {
            last_rx + config.dead_after
        } else {
            register_deadline
        };
        let frame = tokio::select! {
            f = ws.next() => f,
            _ = ping.tick() => {
                if ws.send(WsMessage::Ping(Vec::new().into())).await.is_err() {
                    return End::Retry("control connection lost".into());
                }
                continue;
            }
            () = tokio::time::sleep_until(dead_at) => {
                return End::Retry(if registered { "keep-alive timed out" } else { "no answer to register" }.into());
            }
            () = stopped(stop) => {
                let _ = ws.close(None).await;
                return End::Stop;
            }
        };
        last_rx = Instant::now();
        let bytes = match frame {
            Some(Ok(WsMessage::Binary(b))) => b,
            Some(Ok(WsMessage::Ping(_) | WsMessage::Pong(_) | WsMessage::Frame(_))) => continue,
            Some(Ok(WsMessage::Text(_))) => return End::Retry("text message from server".into()),
            Some(Ok(WsMessage::Close(_))) | None => {
                return End::Retry("control connection closed".into());
            }
            Some(Err(e)) => return End::Retry(e.to_string()),
        };
        let msg = match Message::decode(&bytes) {
            Ok(m) => m,
            Err(e) => return End::Retry(format!("bad message from server: {e}")),
        };
        match msg {
            Message::Registered { observed } if !registered => {
                registered = true;
                *backoff = config.backoff_min;
                let _ = status.send(RegistrationStatus::Registered { observed });
            }
            Message::Incoming {
                session,
                candidates,
                observed,
            } if registered => {
                let req = IncomingRequest {
                    session,
                    peer: PeerInfo::new(&candidates, observed),
                    relay: relay.clone(),
                    received: Instant::now(),
                };
                // A full queue drops the request; the listener retries.
                let _ = requests.try_send(req);
            }
            Message::Error { code } => return end_for(code),
            Message::Unknown { .. } => {}
            other => {
                return End::Retry(format!(
                    "unexpected {:?} on the control connection",
                    other.message_type()
                ));
            }
        }
    }
}
