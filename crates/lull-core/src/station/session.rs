// SPDX-License-Identifier: AGPL-3.0-or-later
//! One station session task per connection, and the [`StationPolicy`] that
//! answers it from the engine's shared state (protocol.md §4, §5, §9).

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use lull_crypto::{InviteToken, Psk, PublicKey};
use lull_media::CodecId;
use lull_media::gate::{GateConfig, ListenGate};
use lull_net::Link;
use lull_proto::control::{CloseReason, Control, StationStatus, TalkStatus};
use lull_proto::preface::Mode;
use lull_proto::record::AudioHeader;
use lull_proto::video::VideoFrame;
use lull_session::{SessionError, StationConfig as SessConfig, StationEvent as SessEvent};
use lull_session::{StationPolicy, StationSession};
use tokio::sync::mpsc;

use super::media::Watch;
use super::{MAX_PENDING_SESSIONS, Shared, StationEvent, next_session};
use crate::pump;
use crate::types::{ListenMode, Transport};

/// Commands from the engine to a session task.
#[derive(Debug)]
pub(crate) enum Cmd {
    Audio(AudioHeader, Vec<u8>),
    Control(Control),
    Video(VideoFrame),
    Close(CloseReason, Option<String>),
}

/// The engine's view of one session.
pub(crate) struct Sess {
    tx: mpsc::UnboundedSender<Cmd>,
    pub transport: Transport,
    pub peer: Option<SocketAddr>,
    pub pairing_mode: bool,
    pub key: Option<PublicKey>,
    pub name: Option<String>,
    pub connected: bool,
    pub codec: Option<CodecId>,
    pub ring: Option<usize>,
    pub gate: Option<ListenGate>,
    pub mode: ListenMode,
    pub video: Option<Watch>,
    pub rtt_ms: Option<u64>,
}

impl Sess {
    pub fn send(&self, cmd: Cmd) {
        let _ = self.tx.send(cmd);
    }
}

/// Answers a session from the shared trust list and pairing window.
pub(crate) struct Policy {
    shared: Arc<Shared>,
    id: u64,
    ip: Option<IpAddr>,
}

impl StationPolicy for Policy {
    fn is_trusted(&mut self, listener: &PublicKey) -> bool {
        self.shared.lock().trust.contains(listener)
    }

    fn is_rate_limited(&mut self, mode: Mode) -> bool {
        let now = self.shared.now();
        let mut g = self.shared.lock();
        if let Some(s) = g.sessions.get_mut(&self.id) {
            s.pairing_mode = mode.is_pairing();
        }
        mode.is_pairing() && self.ip.is_some_and(|ip| g.limiter.is_limited(ip, now))
    }

    fn invite_token(&mut self) -> Option<InviteToken> {
        let mut g = self.shared.lock();
        self.shared.check_window(&mut g);
        g.pairing.as_ref().map(|w| w.token.clone())
    }

    fn code_psk(&mut self) -> Option<Psk> {
        let mut g = self.shared.lock();
        self.shared.check_window(&mut g);
        g.pairing.as_ref().map(|w| w.psk.clone())
    }

    fn status(&mut self) -> StationStatus {
        let g = self.shared.lock();
        self.shared.status(&g)
    }

    fn paired(&mut self, listener: &PublicKey, name: &str) {
        let unix = self.shared.clock.unix_ms();
        let mut g = self.shared.lock();
        g.trust.add(*listener, name, unix);
        self.shared.save_trust(&g);
        if let Some(w) = g.pairing.as_mut() {
            w.record_pairing();
        }
        self.shared.check_window(&mut g);
        self.shared.publish(&g);
    }

    fn code_attempt_failed(&mut self) {
        let mut g = self.shared.lock();
        if let Some(w) = g.pairing.as_mut() {
            w.record_code_failure();
        }
        self.shared.check_window(&mut g);
        self.shared.publish(&g);
    }
}

type Session = StationSession<Policy>;

fn apply(s: &mut Session, cmd: Cmd) {
    // Errors mean the session is closing or not set up yet; drop the command.
    let _ = match cmd {
        Cmd::Audio(h, p) => s.send_audio(h, &p),
        Cmd::Control(c) => s.send_control(c),
        Cmd::Video(f) => s.send_video(&f),
        Cmd::Close(reason, detail) => {
            s.close(reason, detail);
            Ok(())
        }
    };
}

/// Runs one accepted connection to its end.
pub(crate) async fn run(shared: Arc<Shared>, link: Link) {
    let id = next_session(&shared);
    let info = link.info().clone();
    let transport = Transport::from(link.kind());
    let (tx, mut rx) = mpsc::unbounded_channel();
    let (busy, hello) = {
        let mut g = shared.lock();
        if !g.running {
            return;
        }
        let pending = g.sessions.values().filter(|s| !s.connected).count();
        if pending >= MAX_PENDING_SESSIONS {
            return;
        }
        let connected = g.sessions.values().filter(|s| s.connected).count();
        g.sessions.insert(
            id,
            Sess {
                tx,
                transport,
                peer: info.peer,
                pairing_mode: false,
                key: None,
                name: None,
                connected: false,
                codec: None,
                ring: None,
                gate: None,
                mode: ListenMode::Continuous,
                video: None,
                rtt_ms: None,
            },
        );
        (connected >= shared.cfg.max_listeners, shared.hello(&g))
    };
    let mut cfg = SessConfig::new(transport.flavor(), hello);
    cfg.busy = busy;
    let policy = Policy {
        shared: shared.clone(),
        id,
        ip: info.peer.map(|p| p.ip()),
    };
    let mut reason = String::from("ended");
    match StationSession::new(cfg, &shared.key, policy, shared.now()) {
        Ok(mut session) => {
            pump::drive(&mut session, link, &*shared.clock, &mut rx, apply, |s| {
                while let Some(ev) = s.poll_event() {
                    on_event(&shared, id, s, ev, &mut reason);
                }
            })
            .await;
        }
        Err(e) => reason = e.to_string(),
    }
    ended(&shared, id, reason);
}

fn on_event(shared: &Shared, id: u64, s: &mut Session, ev: SessEvent, reason: &mut String) {
    match ev {
        SessEvent::Connected {
            listener_key,
            listener,
            audio_codec,
            paired,
        } => {
            let unix = shared.clock.unix_ms();
            let mut g = shared.lock();
            g.trust.seen(&listener_key, &listener.name, unix);
            shared.save_trust(&g);
            let listener_id = listener_key.device_id().as_str().to_owned();
            // protocol.md §9: the same listener again replaces the old session.
            let older: Vec<u64> = g
                .sessions
                .iter()
                .filter(|&(&other, o)| other != id && o.key == Some(listener_key))
                .map(|(&other, _)| other)
                .collect();
            for old in older {
                if let Some(o) = g.sessions.get(&old) {
                    o.send(Cmd::Close(CloseReason::Replaced, None));
                }
                shared.emit(StationEvent::SessionReplaced {
                    listener_id: listener_id.clone(),
                    old_session: old,
                });
            }
            let codec = CodecId(audio_codec);
            let ring = g.audio.ring_index(codec);
            let gate = ring.map(|r| {
                g.audio.rings[r].users += 1;
                // Continuous until the first `listen` (protocol.md §7.4).
                ListenGate::new(GateConfig::default(), &g.audio.rings[r].ring)
            });
            if let Some(e) = g.sessions.get_mut(&id) {
                e.key = Some(listener_key);
                e.name = Some(listener.name.clone());
                e.connected = true;
                e.codec = Some(codec);
                e.ring = ring;
                e.gate = gate;
            }
            shared.emit(StationEvent::ListenerConnected {
                session: id,
                listener_id: listener_id.clone(),
                name: listener.name.clone(),
            });
            if paired {
                shared.emit(StationEvent::Paired {
                    listener_id,
                    name: listener.name,
                    verification: s
                        .verification_number()
                        .map(|v| v.to_string())
                        .unwrap_or_default(),
                });
            }
            shared.send_status(&mut g, false);
            shared.publish(&g);
        }
        SessEvent::Control(c) => on_control(shared, id, s, c),
        SessEvent::Audio { header, payload } => {
            let mut g = shared.lock();
            shared.on_talk_audio(&mut g, id, header, payload);
        }
        SessEvent::Rtt(ms) => {
            let mut g = shared.lock();
            if let Some(e) = g.sessions.get_mut(&id) {
                e.rtt_ms = Some(ms);
            }
        }
        SessEvent::VideoFrameReplaced { .. } => {
            let mut g = shared.lock();
            shared.on_video_replaced(&mut g, id);
        }
        SessEvent::Closed {
            reason: r,
            detail,
            by_peer,
        } => {
            *reason = match (by_peer, detail) {
                (true, Some(d)) => format!("closed by listener: {} ({d})", r.as_str()),
                (true, None) => format!("closed by listener: {}", r.as_str()),
                (false, _) => format!("closed: {}", r.as_str()),
            };
        }
        SessEvent::Failed(e) => {
            *reason = e.to_string();
            let now = shared.now();
            let mut g = shared.lock();
            let failed_pairing = g
                .sessions
                .get(&id)
                .is_some_and(|e| e.pairing_mode && !e.connected);
            let auth_failure = matches!(
                e,
                SessionError::Handshake(_) | SessionError::Transport(_) | SessionError::Protocol(_)
            );
            if failed_pairing
                && auth_failure
                && let Some(ip) = g.sessions.get(&id).and_then(|e| e.peer).map(|p| p.ip())
            {
                g.limiter.record_failure(ip, now);
            }
        }
    }
}

fn on_control(shared: &Shared, id: u64, s: &mut Session, c: Control) {
    let mut g = shared.lock();
    match c {
        Control::Listen(l) => {
            if let Some(e) = g.sessions.get_mut(&id) {
                e.mode = ListenMode::from_wire(l.mode);
                if let Some(gate) = e.gate.as_mut() {
                    gate.set_config(GateConfig {
                        mode: match e.mode {
                            ListenMode::Continuous => lull_media::gate::ListenMode::Continuous,
                            ListenMode::Activated => lull_media::gate::ListenMode::Activated,
                        },
                        threshold_dbfs: l.threshold,
                        hold_ms: l.hold_ms,
                        preroll_ms: l.preroll_ms,
                    });
                }
            }
            shared.publish(&g);
        }
        Control::VideoStart(v) => shared.on_video_start(&mut g, id, v),
        Control::VideoStop => shared.on_video_stop(&mut g, id),
        Control::VideoOptions { camera, night } => shared.on_video_options(&mut g, camera, night),
        Control::TalkRequest => shared.on_talk_request(&mut g, id),
        Control::TalkRelease => {
            if g.talk.talker == Some(id) {
                shared.release_talk(&mut g, id, TalkStatus::Idle);
            } else if let Some(e) = g.sessions.get(&id) {
                e.send(Cmd::Control(Control::TalkState {
                    state: TalkStatus::Idle,
                    by: None,
                }));
            }
        }
        Control::Unpair => {
            // protocol.md §7.10: forget the listener and close (CORE-6).
            let key = g.sessions.get(&id).and_then(|e| e.key);
            if let Some(key) = key
                && let Some(l) = g.trust.remove(&key)
            {
                shared.save_trust(&g);
                shared.emit(StationEvent::ListenerUnpaired { listener_id: l.id });
            }
            drop(g);
            s.close(CloseReason::Revoked, Some("unpaired".into()));
        }
        _ => {}
    }
}

/// Removes a finished session and updates everything that depended on it.
fn ended(shared: &Shared, id: u64, reason: String) {
    let mut g = shared.lock();
    if g.talk.talker == Some(id) {
        shared.release_talk(&mut g, id, TalkStatus::Idle);
    }
    let Some(e) = g.sessions.remove(&id) else {
        return;
    };
    if let Some(r) = e.ring
        && let Some(ring) = g.audio.rings.get_mut(r)
    {
        ring.users = ring.users.saturating_sub(1);
    }
    if e.video.is_some() {
        shared.recompute_camera(&mut g);
    }
    if let (true, Some(key)) = (e.connected, e.key) {
        shared.emit(StationEvent::ListenerDisconnected {
            session: id,
            listener_id: key.device_id().as_str().to_owned(),
            reason,
        });
        shared.send_status(&mut g, false);
    }
    shared.publish(&g);
}
