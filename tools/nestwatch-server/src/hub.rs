// SPDX-License-Identifier: AGPL-3.0-or-later
//! The rendezvous and relay (rendezvous.md §4): registrations, pending
//! sessions, and the relay pipes. State lives in memory except what the
//! [`Store`] persists.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use hyper::upgrade::Upgraded;
use hyper_util::rt::TokioIo;
use nestwatch_relay::msg::{DecodeError, SessionId};
use nestwatch_relay::{ErrorCode, MailboxId, Message, secret_hash};
use tokio::sync::{mpsc, oneshot, watch};
use tokio::time::Instant;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message as WsMessage;

use crate::config::{Config, secs};
use crate::limits::{ByteBucket, Kind, RateLimiter, ip_key};
use crate::metrics::{Metrics, dec, inc};
use crate::store::{Claim, Store};

/// The server side of one WebSocket.
pub(crate) type ServerWs = WebSocketStream<TokioIo<Upgraded>>;

/// Time allowed to send a final message and close a WebSocket.
const CLOSE_TIMEOUT: Duration = Duration::from_secs(2);
/// Queue of messages for one control connection.
const CONTROL_QUEUE: usize = 64;

/// Who is on the other end of a connection.
#[derive(Debug, Clone)]
pub(crate) struct Client {
    /// Address used for limits (proxy-aware).
    pub ip: IpAddr,
    /// Text for `observed` (`ip:port`, or `ip` behind a proxy).
    pub observed: String,
}

enum ControlCmd {
    Send(Message),
    Replaced,
}

struct ControlRef {
    id: u64,
    tx: mpsc::Sender<ControlCmd>,
}

#[derive(Default)]
struct Mailbox {
    control: Option<ControlRef>,
    offline_since: Option<Instant>,
    /// Pipes plus pending sessions.
    pipes: usize,
}

struct Pending {
    mailbox: MailboxId,
    created: Instant,
    tx: oneshot::Sender<StationSide>,
    /// The `incoming` to deliver when the station (re-)registers.
    offer: Option<Message>,
}

struct StationSide {
    ws: ServerWs,
    candidates: Vec<String>,
    observed: String,
}

#[derive(Default)]
struct State {
    mailboxes: HashMap<MailboxId, Mailbox>,
    sessions: HashMap<SessionId, Pending>,
    ip_pipes: HashMap<IpAddr, usize>,
    ip_conns: HashMap<IpAddr, usize>,
    rate: RateLimiter,
}

/// Shared server state.
pub(crate) struct Hub {
    pub(crate) cfg: Arc<Config>,
    pub(crate) store: Store,
    pub(crate) metrics: Metrics,
    state: Mutex<State>,
    shutdown: watch::Receiver<bool>,
    next_conn: AtomicU64,
}

/// Decrements a per-address WebSocket connection count when dropped.
pub(crate) struct ConnSlot {
    hub: Arc<Hub>,
    ip: IpAddr,
}

impl Drop for ConnSlot {
    fn drop(&mut self) {
        let mut g = self.hub.lock();
        dec_map(&mut g.ip_conns, self.ip);
    }
}

/// Releases a pipe slot (mailbox and listener address) when dropped.
struct PipeSlot {
    hub: Arc<Hub>,
    mailbox: MailboxId,
    ip: IpAddr,
}

impl Drop for PipeSlot {
    fn drop(&mut self) {
        let mut g = self.hub.lock();
        if let Some(m) = g.mailboxes.get_mut(&self.mailbox) {
            m.pipes = m.pipes.saturating_sub(1);
        }
        dec_map(&mut g.ip_pipes, self.ip);
    }
}

fn dec_map(map: &mut HashMap<IpAddr, usize>, ip: IpAddr) {
    if let Some(n) = map.get_mut(&ip) {
        *n -= 1;
        if *n == 0 {
            map.remove(&ip);
        }
    }
}

fn random_session() -> SessionId {
    let mut s = [0; 16];
    getrandom::fill(&mut s).expect("system random generator");
    s
}

async fn send(ws: &mut ServerWs, m: &Message) -> bool {
    match m.encode() {
        Ok(b) => ws.send(WsMessage::Binary(b.into())).await.is_ok(),
        Err(_) => false,
    }
}

/// Sends `error` and closes.
async fn refuse(mut ws: ServerWs, code: ErrorCode) {
    let _ = tokio::time::timeout(CLOSE_TIMEOUT, async {
        send(&mut ws, &Message::Error { code }).await;
        let _ = ws.close(None).await;
    })
    .await;
}

/// The next binary message; `None` on close, error, or a text message.
async fn next_binary(ws: &mut ServerWs) -> Option<Vec<u8>> {
    loop {
        match ws.next().await? {
            Ok(WsMessage::Binary(b)) => return Some(b.into()),
            Ok(WsMessage::Ping(_) | WsMessage::Pong(_) | WsMessage::Frame(_)) => {}
            Ok(WsMessage::Text(_) | WsMessage::Close(_)) | Err(_) => return None,
        }
    }
}

impl Hub {
    pub(crate) fn new(cfg: Arc<Config>, store: Store, shutdown: watch::Receiver<bool>) -> Self {
        Self {
            cfg,
            store,
            metrics: Metrics::default(),
            state: Mutex::new(State::default()),
            shutdown,
            next_conn: AtomicU64::new(1),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().expect("not poisoned")
    }

    async fn stopped(&self) {
        let mut s = self.shutdown.clone();
        let _ = s.wait_for(|v| *v).await;
    }

    /// Reserves a WebSocket connection slot for `ip`, or `None` at the
    /// per-address limit.
    pub(crate) fn conn_slot(self: &Arc<Self>, ip: IpAddr) -> Option<ConnSlot> {
        let ip = ip_key(ip);
        let mut g = self.lock();
        let n = g.ip_conns.entry(ip).or_insert(0);
        if *n >= self.cfg.limits.max_connections_per_ip {
            return None;
        }
        *n += 1;
        Some(ConnSlot {
            hub: self.clone(),
            ip,
        })
    }

    fn rate_ok(&self, ip: IpAddr, kind: Kind, per_minute: u32) -> bool {
        let burst = f64::from(per_minute.max(1));
        let ok = self
            .lock()
            .rate
            .allow(ip, kind, burst, burst / 60.0, std::time::Instant::now());
        if !ok {
            inc(&self.metrics.rate_limited);
        }
        ok
    }

    /// STUN rate limit per address (§4.3 spirit: no amplification floods).
    pub(crate) fn stun_allowed(&self, ip: IpAddr) -> bool {
        let per_s = f64::from(self.cfg.limits.stun_per_second.max(1));
        let ok = self
            .lock()
            .rate
            .allow(ip, Kind::Stun, per_s, per_s, std::time::Instant::now());
        if !ok {
            inc(&self.metrics.rate_limited);
        }
        ok
    }

    /// Periodic housekeeping: expired sessions, forgotten mailboxes, rate
    /// limiter memory.
    pub(crate) fn sweep(&self) {
        let session_timeout = secs(self.cfg.limits.session_timeout);
        let grace = secs(self.cfg.limits.registration_grace);
        let mut g = self.lock();
        // Expiries are counted by the waiting listener task.
        g.sessions
            .retain(|_, p| p.created.elapsed() < session_timeout && !p.tx.is_closed());
        g.mailboxes.retain(|_, m| {
            m.control.is_some()
                || m.pipes > 0
                || m.offline_since.is_some_and(|t| t.elapsed() < grace)
        });
        g.rate.sweep(std::time::Instant::now());
    }

    /// Serves one `/nwr/1` WebSocket until it ends.
    pub(crate) async fn serve(self: Arc<Self>, mut ws: ServerWs, client: Client, slot: ConnSlot) {
        let first = tokio::time::timeout(
            secs(self.cfg.limits.first_message_timeout),
            next_binary(&mut ws),
        )
        .await;
        let Ok(Some(bytes)) = first else {
            let _ = tokio::time::timeout(CLOSE_TIMEOUT, ws.close(None)).await;
            return;
        };
        match Message::decode(&bytes) {
            Ok(Message::Register { mailbox, secret }) => {
                self.control(ws, client, mailbox, secret_hash(&secret.0))
                    .await;
            }
            Ok(Message::Connect {
                mailbox,
                candidates,
                ..
            }) => self.listener(ws, client, mailbox, candidates).await,
            Ok(Message::Accept {
                session,
                candidates,
            }) => {
                drop(slot);
                self.accept(ws, client, session, candidates);
            }
            Err(DecodeError::Version(_)) => {
                inc(&self.metrics.registrations_refused);
                refuse(ws, ErrorCode::Version).await;
            }
            Ok(_) | Err(_) => {
                inc(&self.metrics.protocol_errors);
                refuse(ws, ErrorCode::Protocol).await;
            }
        }
    }

    /// A station's control connection (§4.1).
    async fn control(
        self: &Arc<Self>,
        mut ws: ServerWs,
        client: Client,
        mailbox: MailboxId,
        hash: [u8; 32],
    ) {
        if !self.rate_ok(
            client.ip,
            Kind::Register,
            self.cfg.limits.register_per_minute,
        ) {
            inc(&self.metrics.registrations_refused);
            return refuse(ws, ErrorCode::RateLimited).await;
        }
        match self.store.check_or_claim(&mailbox, &hash) {
            Ok(Claim::Known | Claim::Claimed) => {}
            Ok(Claim::Taken) => {
                inc(&self.metrics.registrations_refused);
                return refuse(ws, ErrorCode::MailboxTaken).await;
            }
            Err(e) => {
                tracing::error!("cannot persist mailbox state: {e}");
                inc(&self.metrics.registrations_refused);
                return refuse(ws, ErrorCode::Unavailable).await;
            }
        }
        let id = self.next_conn.fetch_add(1, Ordering::Relaxed);
        let (tx, mut rx) = mpsc::channel(CONTROL_QUEUE);
        let parked: Vec<Message> = {
            let mut g = self.lock();
            let state = &mut *g;
            let m = state.mailboxes.entry(mailbox).or_default();
            match m.control.replace(ControlRef { id, tx }) {
                Some(old) => {
                    let _ = old.tx.try_send(ControlCmd::Replaced);
                }
                None => inc(&self.metrics.mailboxes_online),
            }
            m.offline_since = None;
            state
                .sessions
                .values_mut()
                .filter(|p| p.mailbox == mailbox)
                .filter_map(|p| p.offer.take())
                .collect()
        };
        inc(&self.metrics.registrations);
        let registered = Message::Registered {
            observed: client.observed.clone(),
        };
        let mut ok = send(&mut ws, &registered).await;
        for offer in &parked {
            ok = ok && send(&mut ws, offer).await;
        }
        let idle = secs(self.cfg.limits.control_idle_timeout);
        let mut last_rx = Instant::now();
        while ok {
            tokio::select! {
                frame = ws.next() => {
                    last_rx = Instant::now();
                    match frame {
                        Some(Ok(WsMessage::Ping(_) | WsMessage::Pong(_) | WsMessage::Frame(_))) => {}
                        Some(Ok(WsMessage::Binary(b))) => match Message::decode(&b) {
                            Ok(Message::Unknown { .. }) => {}
                            _ => {
                                inc(&self.metrics.protocol_errors);
                                send(&mut ws, &Message::Error { code: ErrorCode::Protocol }).await;
                                break;
                            }
                        },
                        _ => break,
                    }
                }
                cmd = rx.recv() => match cmd {
                    Some(ControlCmd::Send(m)) => ok = send(&mut ws, &m).await,
                    Some(ControlCmd::Replaced) | None => {
                        send(&mut ws, &Message::Error { code: ErrorCode::Replaced }).await;
                        break;
                    }
                },
                () = tokio::time::sleep_until(last_rx + idle) => break,
                () = self.stopped() => {
                    send(&mut ws, &Message::Error { code: ErrorCode::Unavailable }).await;
                    break;
                }
            }
        }
        {
            let mut g = self.lock();
            if let Some(m) = g.mailboxes.get_mut(&mailbox)
                && m.control.as_ref().is_some_and(|c| c.id == id)
            {
                m.control = None;
                m.offline_since = Some(Instant::now());
                dec(&self.metrics.mailboxes_online);
            }
        }
        let _ = tokio::time::timeout(CLOSE_TIMEOUT, ws.close(None)).await;
    }

    /// A listener's `connect` (§4.2).
    async fn listener(
        self: &Arc<Self>,
        ws: ServerWs,
        client: Client,
        mailbox: MailboxId,
        candidates: Vec<String>,
    ) {
        inc(&self.metrics.connects);
        let refused = |ws, code| {
            inc(&self.metrics.connects_refused);
            refuse(ws, code)
        };
        let limits = &self.cfg.limits;
        if !self.rate_ok(client.ip, Kind::Connect, limits.connect_per_minute) {
            return refused(ws, ErrorCode::RateLimited).await;
        }
        if self.store.transfer(&mailbox) >= limits.monthly_bytes_per_mailbox {
            return refused(ws, ErrorCode::Quota).await;
        }
        let ip = ip_key(client.ip);
        let session = random_session();
        let (tx, rx) = oneshot::channel();
        let grace = secs(limits.registration_grace);
        let outcome = {
            let mut g = self.lock();
            let state = &mut *g;
            let ip_pipes = state.ip_pipes.get(&ip).copied().unwrap_or(0);
            match state.mailboxes.get_mut(&mailbox) {
                None => Err(ErrorCode::Offline),
                Some(m)
                    if m.control.is_none()
                        && m.offline_since.is_none_or(|t| t.elapsed() >= grace) =>
                {
                    Err(ErrorCode::Offline)
                }
                Some(m) if m.pipes >= limits.pipes_per_mailbox => Err(ErrorCode::Limit),
                Some(_) if ip_pipes >= limits.pipes_per_ip => Err(ErrorCode::Limit),
                Some(m) => {
                    m.pipes += 1;
                    *state.ip_pipes.entry(ip).or_insert(0) += 1;
                    let offer = Message::Incoming {
                        session,
                        candidates,
                        observed: client.observed.clone(),
                    };
                    let parked = match &m.control {
                        Some(c) => c.tx.try_send(ControlCmd::Send(offer.clone())).is_err(),
                        None => true,
                    };
                    state.sessions.insert(
                        session,
                        Pending {
                            mailbox,
                            created: Instant::now(),
                            tx,
                            offer: parked.then_some(offer),
                        },
                    );
                    Ok(())
                }
            }
        };
        if let Err(code) = outcome {
            return refused(ws, code).await;
        }
        let _slot = PipeSlot {
            hub: self.clone(),
            mailbox,
            ip,
        };
        let timeout = secs(limits.session_timeout);
        let station = tokio::select! {
            r = tokio::time::timeout(timeout, rx) => r.ok().and_then(Result::ok),
            () = self.stopped() => None,
        };
        let Some(station) = station else {
            self.lock().sessions.remove(&session);
            inc(&self.metrics.sessions_expired);
            return refused(ws, ErrorCode::Offline).await;
        };
        let mut ws = ws;
        let accepted = Message::Accepted {
            candidates: station.candidates,
            observed: station.observed,
        };
        if !send(&mut ws, &accepted).await {
            return;
        }
        self.pipe(ws, station.ws, mailbox).await;
    }

    /// A station's `accept` on a new connection (§4.2): hands the socket
    /// to the waiting listener task, which runs the pipe.
    fn accept(
        self: &Arc<Self>,
        ws: ServerWs,
        client: Client,
        session: SessionId,
        candidates: Vec<String>,
    ) {
        let timeout = secs(self.cfg.limits.session_timeout);
        let pending = self
            .lock()
            .sessions
            .remove(&session)
            .filter(|p| p.created.elapsed() < timeout);
        let side = StationSide {
            ws,
            candidates,
            observed: client.observed,
        };
        let failed = match pending {
            Some(p) => p.tx.send(side).err(),
            None => Some(side),
        };
        if let Some(side) = failed {
            tokio::spawn(refuse(side.ws, ErrorCode::Session));
        }
    }

    /// The relay pipe (§4.2): forwards binary messages unchanged in both
    /// directions until either side closes, the pipe idles, its lifetime
    /// ends, the mailbox's budget is used up, or the server stops.
    async fn pipe(self: &Arc<Self>, listener: ServerWs, station: ServerWs, mailbox: MailboxId) {
        inc(&self.metrics.pipes);
        inc(&self.metrics.pipes_active);
        let start = Instant::now();
        let last = AtomicU64::new(0);
        let (mut l_tx, mut l_rx) = listener.split();
        let (mut s_tx, mut s_rx) = station.split();
        let limits = &self.cfg.limits;
        let idle = secs(limits.idle_timeout);
        {
            let up = self.forward(&mut l_rx, &mut s_tx, &mailbox, &last, start);
            let down = self.forward(&mut s_rx, &mut l_tx, &mailbox, &last, start);
            let idle_watch = async {
                loop {
                    let seen = start + Duration::from_millis(last.load(Ordering::Relaxed));
                    let at = seen + idle;
                    if Instant::now() >= at {
                        return;
                    }
                    tokio::time::sleep_until(at).await;
                }
            };
            tokio::select! {
                () = up => {}
                () = down => {}
                () = idle_watch => {}
                () = tokio::time::sleep(secs(limits.pipe_lifetime)) => {}
                () = self.stopped() => {}
            }
        }
        let _ = tokio::time::timeout(CLOSE_TIMEOUT, async {
            let _ = l_tx.close().await;
            let _ = s_tx.close().await;
        })
        .await;
        dec(&self.metrics.pipes_active);
    }

    async fn forward<R, W>(
        &self,
        rx: &mut R,
        tx: &mut W,
        mailbox: &MailboxId,
        last: &AtomicU64,
        start: Instant,
    ) where
        R: futures_util::Stream<Item = Result<WsMessage, tokio_tungstenite::tungstenite::Error>>
            + Unpin,
        W: futures_util::Sink<WsMessage> + Unpin,
    {
        let limits = &self.cfg.limits;
        let mut bucket = ByteBucket::new(
            limits.pipe_rate_bps,
            limits.max_message_bytes,
            std::time::Instant::now(),
        );
        while let Some(frame) = rx.next().await {
            let data = match frame {
                Ok(WsMessage::Binary(b)) => b,
                Ok(WsMessage::Ping(_) | WsMessage::Pong(_) | WsMessage::Frame(_)) => continue,
                Ok(WsMessage::Text(_)) => {
                    inc(&self.metrics.protocol_errors);
                    return;
                }
                Ok(WsMessage::Close(_)) | Err(_) => return,
            };
            let n = data.len();
            last.store(start.elapsed().as_millis() as u64, Ordering::Relaxed);
            self.metrics
                .bytes_relayed
                .fetch_add(n as u64, Ordering::Relaxed);
            if self.store.add_transfer(mailbox, n as u64) > limits.monthly_bytes_per_mailbox {
                return;
            }
            let wait = bucket.take(n, std::time::Instant::now());
            if !wait.is_zero() {
                tokio::time::sleep(wait).await;
            }
            if tx.send(WsMessage::Binary(data)).await.is_err() {
                return;
            }
        }
    }
}
