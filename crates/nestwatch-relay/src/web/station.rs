// SPDX-License-Identifier: AGPL-3.0-or-later
//! The browser station's control connection (rendezvous.md §4.1, FR-RV-7)
//! and the accept connections it opens for each listener (§4.2), over the
//! browser WebSocket.
//!
//! Same behaviour as the native registration, except for the keep-alive:
//! a page cannot send WebSocket pings, so it sends `{t: "ping"}` every
//! `ping_interval` once registered (OPEN-QUESTIONS WEBST-3). Servers that
//! know the message answer `pong`; once one did on a connection, silence for
//! `dead_after` makes the station reconnect. Older servers ignore `ping` as
//! an unknown type, which still keeps their idle timer from expiring.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use nestwatch_net::{Link, LinkReader, LinkWriter, NetError};
use tokio::sync::{mpsc, oneshot, watch};

use crate::candidate::Candidate;
use crate::exchange::{self, PeerInfo, with_timeout};
use crate::msg::{ErrorCode, Message, SessionId};
use crate::registration::{
    End, Incoming, REQUEST_QUEUE, RegistrationConfig, RegistrationStatus, SESSION_VALIDITY, end_for,
};
use crate::{MailboxIdentity, RelayConfig, RelayError};

fn now_ms() -> f64 {
    js_sys::Date::now()
}

async fn sleep(d: Duration) {
    gloo_timers::future::sleep(d).await;
}

async fn open(relay: &RelayConfig) -> Result<Link, RelayError> {
    let url = relay.server.ws_url().to_owned();
    with_timeout(relay.timeout, async move {
        Ok(nestwatch_net::ws_client::connect(&url).await?)
    })
    .await
}

/// A listener asks to connect (`incoming`, §4.2). Accept it within 30 s.
#[derive(Debug)]
pub struct IncomingRequest {
    session: SessionId,
    peer: PeerInfo,
    relay: Arc<RelayConfig>,
    received_ms: f64,
}

impl IncomingRequest {
    /// The listener's candidates and observed address.
    pub fn peer(&self) -> &PeerInfo {
        &self.peer
    }

    /// Whether the 30 s session validity has passed.
    pub fn is_expired(&self) -> bool {
        now_ms() - self.received_ms >= SESSION_VALIDITY.as_millis() as f64
    }

    /// Opens the accept connection and sends `accept` with `candidates`
    /// (none for a browser, which cannot punch). The returned link is the
    /// relay pipe, ready for an NWP/1 station session.
    pub async fn accept(self, candidates: &[Candidate]) -> Result<Incoming, RelayError> {
        if self.is_expired() {
            return Err(RelayError::Server(ErrorCode::Session));
        }
        let mut link = open(&self.relay).await?;
        exchange::station_accept(&mut link, self.session, candidates, self.relay.timeout).await?;
        Ok(Incoming {
            link,
            peer: self.peer,
        })
    }
}

/// Keeps a browser station registered at a relay server (rendezvous.md
/// §4.1): one control connection with keep-alive and reconnect, and one
/// [`IncomingRequest`] per listener. Dropping it unregisters.
#[derive(Debug)]
pub struct StationRegistration {
    status: watch::Receiver<RegistrationStatus>,
    requests: mpsc::Receiver<IncomingRequest>,
    candidates: Arc<Mutex<Vec<Candidate>>>,
    relay: Arc<RelayConfig>,
    stop: watch::Sender<bool>,
    done: Option<oneshot::Receiver<()>>,
}

impl Drop for StationRegistration {
    fn drop(&mut self) {
        let _ = self.stop.send(true);
    }
}

impl StationRegistration {
    /// Starts registering on the browser event loop.
    pub fn start(config: RegistrationConfig, identity: MailboxIdentity) -> Self {
        let (status_tx, status) = watch::channel(RegistrationStatus::Connecting);
        let (req_tx, requests) = mpsc::channel(REQUEST_QUEUE);
        let (stop, stop_rx) = watch::channel(false);
        let (done_tx, done) = oneshot::channel();
        let relay = Arc::new(config.relay.clone());
        let candidates = Arc::new(Mutex::new(config.candidates.clone()));
        let r = relay.clone();
        wasm_bindgen_futures::spawn_local(async move {
            control_loop(config, identity, r, status_tx, req_tx, stop_rx).await;
            let _ = done_tx.send(());
        });
        Self {
            status,
            requests,
            candidates,
            relay,
            stop,
            done: Some(done),
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

    /// The next listener request. `None` once stopped. Cancel-safe.
    pub async fn next_request(&mut self) -> Option<IncomingRequest> {
        self.requests.recv().await
    }

    /// The next listener, accepted with the configured candidates. Failed
    /// accepts are skipped. `None` once stopped.
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
        if let Some(d) = self.done.take() {
            let _ = d.await;
        }
    }
}

/// Resolves once `stop` is set.
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
        if *stop.borrow() {
            let _ = status.send(RegistrationStatus::Stopped);
            return;
        }
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
        let (error, delay) = match end {
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
            error,
            retry_in: delay,
        });
        tokio::select! {
            () = sleep(delay) => {}
            () = stopped(&mut stop) => {
                let _ = status.send(RegistrationStatus::Stopped);
                return;
            }
        }
    }
}

async fn send(w: &mut LinkWriter, m: &Message) -> Result<(), RelayError> {
    w.send(&m.encode()?).await?;
    Ok(())
}

/// Time until `at` (ms on the [`now_ms`] clock), between zero and a day
/// (browser timers take 32-bit milliseconds; disabled `select!` branches
/// still create their timer).
fn until(at: f64) -> Duration {
    Duration::from_millis((at - now_ms()).clamp(0.0, 86_400_000.0) as u64)
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
        r = open(relay) => r,
        () = stopped(stop) => return End::Stop,
    };
    let (mut reader, mut writer): (LinkReader, LinkWriter) = match opened {
        Ok(link) => link.split(),
        Err(e) => return End::Retry(e.to_string()),
    };
    let register = Message::Register {
        mailbox: identity.id(),
        secret: identity.secret(),
    };
    if let Err(e) = send(&mut writer, &register).await {
        return End::Retry(e.to_string());
    }
    let ping_ms = config.ping_interval.as_millis() as f64;
    let mut registered = false;
    let mut pong_seen = false;
    let register_deadline = now_ms() + relay.timeout.as_millis() as f64;
    let mut last_rx = now_ms();
    let mut next_ping = f64::INFINITY;
    let end = loop {
        let dead_at = if !registered {
            register_deadline
        } else if pong_seen {
            last_rx + config.dead_after.as_millis() as f64
        } else {
            f64::INFINITY
        };
        let ping_due = next_ping.is_finite();
        let dead_due = dead_at.is_finite();
        let received = tokio::select! {
            r = reader.recv() => r,
            () = sleep(until(next_ping)), if ping_due => {
                next_ping = now_ms() + ping_ms;
                if send(&mut writer, &Message::Ping).await.is_err() {
                    break End::Retry("control connection lost".into());
                }
                continue;
            }
            () = sleep(until(dead_at)), if dead_due => {
                break End::Retry(if registered { "keep-alive timed out" } else { "no answer to register" }.into());
            }
            () = stopped(stop) => break End::Stop,
        };
        last_rx = now_ms();
        let bytes = match received {
            Ok(Some(b)) => b,
            Ok(None) => break End::Retry("control connection closed".into()),
            Err(NetError::TextMessage) => break End::Retry("text message from server".into()),
            Err(e) => break End::Retry(e.to_string()),
        };
        let msg = match Message::decode(&bytes) {
            Ok(m) => m,
            Err(e) => break End::Retry(format!("bad message from server: {e}")),
        };
        match msg {
            Message::Registered { observed } if !registered => {
                registered = true;
                *backoff = config.backoff_min;
                next_ping = now_ms() + ping_ms;
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
                    received_ms: now_ms(),
                };
                // A full queue drops the request; the listener retries.
                let _ = requests.try_send(req);
            }
            Message::Pong => pong_seen = true,
            Message::Error { code } => break end_for(code),
            Message::Unknown { .. } => {}
            other => {
                break End::Retry(format!(
                    "unexpected {:?} on the control connection",
                    other.message_type()
                ));
            }
        }
    };
    let _ = writer.close().await;
    end
}
