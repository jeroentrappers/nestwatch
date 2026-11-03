// SPDX-License-Identifier: AGPL-3.0-or-later
//! Sessions, pairing and the per-station reconnect loop (FR-LS-1,
//! FR-LS-5, FR-NET-3, FR-PAIR-4), including the relay route and the move
//! from a relayed to a direct path (rendezvous.md FR-RV-2, FR-RV-3, §5.2).

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use nestwatch_media::jitter::JitterBuffer;
use nestwatch_media::{AudioFrame, CodecId, FrameFlags};
use nestwatch_net::Link;
use nestwatch_proto::control::{CloseReason, Control, StationStatus, VideoStateKind};
use nestwatch_proto::preface::Status;
use nestwatch_session::{
    ListenerConfig as SessConfig, ListenerEvent as SessEvent, ListenerMode, ListenerSession,
    SessionError,
};
use tokio::sync::{mpsc, oneshot};

use super::relay;
use super::{
    Alert, Cmd, ConnStatus, Inner, KnownStation, ListenerError, ListenerEvent, NetPath, PairResult,
    Route, Shared, StationSettings, TalkView, VideoFrameEvent, VideoView, effective_video,
    talk_view, video_start,
};
use crate::pump;
use crate::rt;
use crate::types::ListenMode;

/// How one session attempt ended.
#[derive(Debug, Default)]
pub(crate) struct Outcome {
    dial_error: Option<String>,
    live: bool,
    close: Option<CloseReason>,
    error: Option<SessionError>,
    /// The station, known from the start or learned from `paired`.
    station_id: Option<String>,
    /// Reconnect loop generation started by a pairing.
    pairing_gen: Option<u64>,
}

type Reply = oneshot::Sender<Result<PairResult, ListenerError>>;

#[cfg(feature = "native")]
use super::relay::Punch;

/// Nothing to punch with in the browser.
#[cfg(not(feature = "native"))]
pub(crate) enum Punch {}

/// An opened link and what is known about its path.
pub(crate) struct Dialed {
    pub link: Link,
    pub path: NetPath,
    /// Hole punching material of a relayed link (native).
    pub punch: Option<Punch>,
}

async fn dial(shared: &Shared, route: &Route) -> Result<Dialed, String> {
    if let Route::Relay(r) = route {
        return relay::dial(shared, r).await;
    }
    let link = dial_direct(shared, route).await?;
    let path = net_path(link.info().peer);
    Ok(Dialed {
        link,
        path,
        punch: None,
    })
}

async fn dial_direct(shared: &Shared, route: &Route) -> Result<Link, String> {
    let timeout = shared.cfg.dial_timeout;
    match route {
        Route::Relay(_) => Err("not a direct route".into()),
        #[cfg(feature = "native")]
        Route::Tcp(e) => nestwatch_net::dial::dial(e, timeout)
            .await
            .map_err(|e| e.to_string()),
        #[cfg(not(feature = "native"))]
        Route::Tcp(_) => Err("TCP is not available in the browser".into()),
        Route::WebSocket(url) => {
            #[cfg(feature = "native")]
            {
                let cfg = nestwatch_net::ws_client::WsClientConfig {
                    timeout,
                    ..Default::default()
                };
                nestwatch_net::ws_client::connect_with(url, &cfg)
                    .await
                    .map_err(|e| e.to_string())
            }
            #[cfg(not(feature = "native"))]
            {
                match rt::timeout(timeout, nestwatch_net::ws_client::connect(url)).await {
                    Some(r) => r.map_err(|e| e.to_string()),
                    None => Err("timed out".into()),
                }
            }
        }
    }
}

/// Home network or VPN from the peer address (protocol.md §3.5 ranges,
/// CORE-11).
fn net_path(peer: Option<SocketAddr>) -> NetPath {
    match peer {
        Some(p) if nestwatch_proto::endpoint::is_vpn_address(p.ip()) => NetPath::Vpn,
        _ => NetPath::HomeNetwork,
    }
}

fn apply(s: &mut ListenerSession, cmd: Cmd) {
    // Errors mean the session is not (or no longer) set up; drop the command.
    let _ = match cmd {
        Cmd::Control(c) => s.send_control(c),
        Cmd::TalkAudio(h, p) => s.send_talk_audio(h, &p),
        Cmd::Close(reason) => {
            s.close(reason, None);
            Ok(())
        }
    };
}

struct Ctx<'a> {
    shared: &'a Arc<Shared>,
    serial: u64,
    /// The stored route this session uses (none for a punched path).
    route: Option<Route>,
    /// Shown as the station's route.
    label: String,
    path: NetPath,
    /// Hole punching material of a relayed session.
    #[cfg_attr(not(feature = "native"), allow(dead_code))]
    punch: Option<Punch>,
    /// For a direct session: the relayed session it replaces (§5.2).
    replaces: Option<u64>,
    tx: mpsc::UnboundedSender<Cmd>,
    reply: Option<Reply>,
    remember: Vec<Route>,
    verification: String,
    out: Outcome,
}

/// One connection attempt over `route`, run until the session ends.
async fn attempt(
    shared: &Arc<Shared>,
    route: Route,
    mode: ListenerMode,
    station_id: Option<String>,
    reply: Option<Reply>,
    remember: Vec<Route>,
) -> Outcome {
    let dialed = match dial(shared, &route).await {
        Ok(l) => l,
        Err(e) => {
            if let Some(r) = reply {
                let _ = r.send(Err(ListenerError::Unreachable(format!("{route}: {e}"))));
            }
            return Outcome {
                dial_error: Some(e),
                station_id,
                ..Outcome::default()
            };
        }
    };
    let label = route.to_string();
    run_session(
        shared,
        dialed,
        Some(route),
        label,
        mode,
        station_id,
        reply,
        remember,
        None,
    )
    .await
}

/// Runs one NWP/1 listener session over an opened link until it ends.
#[allow(clippy::too_many_arguments)]
pub(super) async fn run_session(
    shared: &Arc<Shared>,
    dialed: Dialed,
    route: Option<Route>,
    label: String,
    mode: ListenerMode,
    station_id: Option<String>,
    reply: Option<Reply>,
    remember: Vec<Route>,
    replaces: Option<u64>,
) -> Outcome {
    let Dialed { link, path, punch } = dialed;
    let flavor = crate::types::Transport::from(link.kind()).flavor();
    let hello = {
        let g = shared.lock();
        shared.hello(&g)
    };
    let cfg = SessConfig {
        flavor,
        mode,
        hello,
    };
    let mut session = match ListenerSession::new(cfg, &shared.key, shared.now()) {
        Ok(s) => s,
        Err(e) => {
            if let Some(r) = reply {
                let _ = r.send(Err(ListenerError::Invalid(e.to_string())));
            }
            return Outcome {
                station_id,
                ..Outcome::default()
            };
        }
    };
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mut ctx = Ctx {
        shared,
        serial: shared.next_session(),
        route,
        label,
        path,
        punch,
        replaces,
        tx,
        reply,
        remember,
        verification: String::new(),
        out: Outcome {
            station_id,
            ..Outcome::default()
        },
    };
    pump::drive(&mut session, link, &*shared.clock, &mut rx, apply, |s| {
        while let Some(ev) = s.poll_event() {
            ctx.on_event(s, ev);
        }
    })
    .await;
    ctx.finish()
}

impl Ctx<'_> {
    fn on_event(&mut self, s: &mut ListenerSession, ev: SessEvent) {
        let shared = self.shared;
        match ev {
            SessEvent::Paired {
                station_key,
                station_id,
            } => {
                let id = station_id.as_str().to_owned();
                let name = s
                    .station_hello()
                    .map(|h| h.name.clone())
                    .unwrap_or_default();
                let mut known = KnownStation {
                    key: station_key,
                    id: id.clone(),
                    name: name.clone(),
                    endpoints: Vec::new(),
                    settings: StationSettings::default(),
                    needs_pairing: false,
                    paired_unix_ms: shared.clock.unix_ms(),
                    relay: None,
                };
                if let Some(r) = &self.route {
                    known.note_endpoint(r, None);
                }
                for r in &self.remember {
                    known.note_endpoint(r, None);
                }
                self.verification = s
                    .verification_number()
                    .map(|v| v.to_string())
                    .unwrap_or_default();
                let mut g = shared.lock();
                shared.upsert_station(&mut g, known);
                self.out.pairing_gen = shared.start_follow(&mut g, &id).ok();
                shared.save(&g);
                self.out.station_id = Some(id.clone());
                shared.emit_after_publish(
                    &mut g,
                    ListenerEvent::Paired {
                        station_id: id,
                        name,
                        verification: self.verification.clone(),
                    },
                );
                shared.publish(&mut g);
            }
            SessEvent::Connected {
                station,
                audio_codec,
                status,
            } => {
                let Some(id) = self.out.station_id.clone() else {
                    return;
                };
                if let Some(old) = self.replaces {
                    self.switch_path(s, &id, old, station, audio_codec, &status);
                    return;
                }
                let unix = shared.clock.unix_ms();
                let mut g = shared.lock();
                let Some(rt) = g.stations.get_mut(&id) else {
                    s.close(CloseReason::Shutdown, None);
                    return;
                };
                rt.cmd = Some((self.serial, self.tx.clone()));
                rt.status = ConnStatus::Live;
                rt.path = Some(self.path);
                rt.route = Some(self.label.clone());
                rt.draining = None;
                if let Some(r) = &self.route {
                    rt.known.note_endpoint(r, Some(unix));
                }
                if !station.name.is_empty() {
                    rt.known.name = station.name.clone();
                }
                rt.caps = station.caps.clone();
                rt.codec = Some(CodecId(audio_codec));
                rt.jitter = JitterBuffer::new(CodecId(audio_codec)).ok();
                rt.talk = TalkView::Idle;
                // protocol.md §7.4: `listen` right after setup.
                let _ = s.send_control(Control::Listen(rt.known.settings.listen.to_wire()));
                if let Some(req) = rt.video_req {
                    rt.video = VideoView::Requested;
                    let _ = s.send_control(video_start(effective_video(req, rt.path)));
                }
                if let Some((camera, night)) = rt.video_options {
                    let _ = s.send_control(Control::VideoOptions { camera, night });
                }
                #[cfg(feature = "native")]
                if let Some(p) = self.punch.take() {
                    rt::spawn(relay::upgrade(
                        shared.clone(),
                        id.clone(),
                        rt.known.key,
                        self.serial,
                        p,
                    ));
                }
                let reachable = rt.alerts.on_up();
                let name = rt.known.name.clone();
                self.out.live = true;
                apply_status(shared, &mut g, &id, &status);
                shared.apply_mixer(&mut g, &id);
                shared.save(&g);
                shared.emit_after_publish(
                    &mut g,
                    ListenerEvent::Connected {
                        station_id: id.clone(),
                    },
                );
                if reachable {
                    shared.alert(&g, &id, Alert::Reachable);
                }
                // The caller learns of the pairing only once the published
                // state already shows the live station.
                shared.publish(&mut g);
                if let Some(r) = self.reply.take() {
                    let _ = r.send(Ok(PairResult {
                        station_id: id,
                        name,
                        verification: self.verification.clone(),
                    }));
                }
            }
            SessEvent::Control(c) => self.on_control(c),
            SessEvent::Audio { header, payload } => {
                let Some(id) = &self.out.station_id else {
                    return;
                };
                let now = shared.now();
                let mut g = shared.lock();
                let Some(rt) = g.stations.get_mut(id) else {
                    return;
                };
                rt.audio_frames += 1;
                if header.flags.preroll() {
                    rt.preroll_frames += 1;
                }
                if let Some(j) = rt.jitter.as_mut() {
                    j.push(
                        AudioFrame {
                            codec: CodecId(header.codec),
                            flags: FrameFlags(header.flags.0),
                            seq: header.seq,
                            timestamp: header.timestamp,
                            payload,
                        },
                        now,
                    );
                }
            }
            SessEvent::Video(frame) => {
                let Some(id) = &self.out.station_id else {
                    return;
                };
                let rgba = if shared.cfg.decode_video {
                    nestwatch_media::video::decode_to_rgba(&frame.data)
                        .ok()
                        .map(Arc::new)
                } else {
                    None
                };
                let _ = shared.video.send(VideoFrameEvent {
                    station_id: id.clone(),
                    frame_id: frame.frame_id,
                    width: frame.width,
                    height: frame.height,
                    jpeg: Arc::new(frame.data),
                    rgba,
                });
            }
            SessEvent::Rtt(ms) => {
                let Some(id) = &self.out.station_id else {
                    return;
                };
                let mut g = shared.lock();
                if let Some(rt) = g.stations.get_mut(id) {
                    rt.rtt_ms = Some(ms);
                }
            }
            SessEvent::Closed { reason, .. } => self.out.close = Some(reason),
            SessEvent::Failed(e) => self.out.error = Some(e),
        }
    }

    /// A direct session replacing the relayed session `old` is set up
    /// (rendezvous.md §5.2 item 4): it takes over the station, the relayed
    /// session is closed with `replaced`, and the relayed audio already
    /// buffered keeps playing until the direct stream decodes its first
    /// frame.
    fn switch_path(
        &mut self,
        s: &mut ListenerSession,
        id: &str,
        old: u64,
        station: nestwatch_proto::control::Hello,
        audio_codec: u8,
        status: &StationStatus,
    ) {
        let shared = self.shared;
        let mut g = shared.lock();
        let Some(rt) = g.stations.get_mut(id) else {
            s.close(CloseReason::Shutdown, None);
            return;
        };
        match &rt.cmd {
            Some((serial, tx)) if *serial == old => {
                let _ = tx.send(Cmd::Close(CloseReason::Replaced));
                rt.draining = rt.jitter.take();
            }
            // The relayed session already ended: just take over.
            None if rt.following => {}
            _ => {
                s.close(CloseReason::Shutdown, None);
                return;
            }
        }
        let was_live = rt.status == ConnStatus::Live;
        if let Some(r) = &self.route {
            rt.known.note_endpoint(r, Some(shared.clock.unix_ms()));
        }
        rt.cmd = Some((self.serial, self.tx.clone()));
        rt.status = ConnStatus::Live;
        rt.path = Some(self.path);
        rt.route = Some(self.label.clone());
        rt.caps = station.caps.clone();
        if !station.name.is_empty() {
            rt.known.name = station.name.clone();
        }
        rt.codec = Some(CodecId(audio_codec));
        rt.jitter = JitterBuffer::new(CodecId(audio_codec)).ok();
        let _ = s.send_control(Control::Listen(rt.known.settings.listen.to_wire()));
        if let Some(req) = rt.video_req {
            rt.video = VideoView::Requested;
            let _ = s.send_control(video_start(effective_video(req, rt.path)));
        }
        if let Some((camera, night)) = rt.video_options {
            let _ = s.send_control(Control::VideoOptions { camera, night });
        }
        // Talk-back belongs to a session: ask again on the new one.
        if matches!(rt.talk, TalkView::Granted | TalkView::Requested) {
            let _ = s.send_control(Control::TalkRequest);
            rt.talk = TalkView::Requested;
        }
        self.out.live = true;
        let reachable = !was_live && rt.alerts.on_up();
        apply_status(shared, &mut g, id, status);
        shared.apply_mixer(&mut g, id);
        if !was_live {
            shared.emit_after_publish(
                &mut g,
                ListenerEvent::Connected {
                    station_id: id.to_owned(),
                },
            );
        }
        if reachable {
            shared.alert(&g, id, Alert::Reachable);
        }
        shared.publish(&mut g);
    }

    fn on_control(&mut self, c: Control) {
        let shared = self.shared;
        let Some(id) = self.out.station_id.clone() else {
            return;
        };
        let now = shared.now();
        let limits = shared.limits();
        let mut g = shared.lock();
        let Some(rt) = g.stations.get_mut(&id) else {
            return;
        };
        match c {
            Control::Level { dbfs, open } => {
                rt.level = Some(dbfs);
                rt.open = open;
                let s = rt.known.settings.listen;
                let sound = s.mode == ListenMode::Continuous
                    && rt.alerts.on_level(dbfs, s.threshold_dbfs, now, &limits);
                if sound {
                    shared.alert(&g, &id, Alert::SoundStarted);
                }
                shared.touch(&mut g);
            }
            Control::Activity { open } => {
                rt.open = open;
                let sound = rt.known.settings.listen.mode == ListenMode::Activated
                    && rt.alerts.on_activity(open, now, &limits);
                if sound {
                    shared.alert(&g, &id, Alert::SoundStarted);
                }
                shared.publish(&mut g);
            }
            Control::Status(st) => {
                apply_status(shared, &mut g, &id, &st);
                shared.publish(&mut g);
            }
            Control::VideoState(v) => {
                rt.video = match (v.state, v.w, v.h, v.fps) {
                    (VideoStateKind::Streaming, Some(w), Some(h), Some(fps)) => {
                        VideoView::Streaming { w, h, fps }
                    }
                    (VideoStateKind::Streaming, ..) => VideoView::Requested,
                    (VideoStateKind::Stopped, ..) if rt.video_req.is_none() => VideoView::Off,
                    (state, ..) => VideoView::State(state),
                };
                shared.publish(&mut g);
            }
            Control::TalkState { state, by } => {
                let view = talk_view(state, by);
                rt.talk = view.clone();
                // FR-TB-2: own playback of this station is muted while talking.
                shared.apply_mixer(&mut g, &id);
                shared.emit_after_publish(
                    &mut g,
                    ListenerEvent::Talk {
                        station_id: id.clone(),
                        state: view.clone(),
                    },
                );
                if view == TalkView::Granted {
                    shared.start_talk_capture(&mut g, &id);
                } else {
                    shared.stop_talk_capture(&mut g, &id);
                }
                shared.publish(&mut g);
            }
            Control::AudioConfig { codec } if rt.codec != Some(CodecId(codec)) => {
                rt.codec = Some(CodecId(codec));
                rt.jitter = JitterBuffer::new(CodecId(codec)).ok();
            }
            _ => {}
        }
    }

    /// Detaches the session from its station and answers a pending pairing.
    fn finish(mut self) -> Outcome {
        let shared = self.shared;
        let reason = describe(&self.out);
        if let Some(id) = self.out.station_id.clone() {
            let now = shared.now();
            let mut g = shared.lock();
            let ours = g
                .stations
                .get(&id)
                .is_some_and(|rt| rt.cmd.as_ref().is_some_and(|(s, _)| *s == self.serial));
            let handover = matches!(self.out.close, Some(CloseReason::Replaced))
                && self.out.live
                && g.stations.get(&id).is_some_and(|rt| rt.upgrading);
            if ours
                && handover
                && let Some(rt) = g.stations.get_mut(&id)
            {
                // The station already switched to the direct session that is
                // still finishing its setup here (§5.2 item 4): stay live and
                // keep the buffered audio playing.
                rt.cmd = None;
                rt.draining = rt.jitter.take();
                shared.publish(&mut g);
            } else if ours {
                detach(shared, &mut g, &id, now, reason.clone());
            }
        }
        if let Some(r) = self.reply.take() {
            let err = match &self.out.error {
                Some(SessionError::Refused(st)) => ListenerError::Refused(status_name(*st).into()),
                _ => ListenerError::Rejected(reason),
            };
            let _ = r.send(Err(err));
        }
        self.out
    }
}

/// The station's live session is gone: reset its runtime state and report
/// the disconnection.
pub(super) fn detach(shared: &Shared, g: &mut Inner, id: &str, now: u64, reason: String) {
    let Some(rt) = g.stations.get_mut(id) else {
        return;
    };
    rt.cmd = None;
    rt.jitter = None;
    rt.draining = None;
    rt.codec = None;
    rt.open = false;
    rt.path = None;
    rt.route = None;
    rt.rtt_ms = None;
    rt.talk = TalkView::Idle;
    rt.video = if rt.video_req.is_some() {
        VideoView::Requested
    } else {
        VideoView::Off
    };
    rt.alerts.on_down(now);
    if rt.following {
        rt.status = ConnStatus::Reconnecting {
            attempt: 1,
            tried: Vec::new(),
        };
    }
    shared.stop_talk_capture(g, id);
    shared.apply_mixer(g, id);
    shared.emit_after_publish(
        g,
        ListenerEvent::Disconnected {
            station_id: id.to_owned(),
            reason,
        },
    );
    shared.publish(g);
}

fn status_name(s: Status) -> &'static str {
    match s {
        Status::Ok => "ok",
        Status::UnsupportedVersion => "unsupported-version",
        Status::PairingClosed => "pairing-closed",
        Status::Busy => "busy",
        Status::RateLimited => "rate-limited",
    }
}

fn describe(out: &Outcome) -> String {
    if let Some(r) = &out.close {
        format!("closed: {}", r.as_str())
    } else if let Some(e) = &out.error {
        e.to_string()
    } else if let Some(e) = &out.dial_error {
        e.clone()
    } else {
        "ended".into()
    }
}

/// Applies a `status` (protocol.md §7.6) and the battery alert (FR-AL-3).
fn apply_status(shared: &Shared, g: &mut Inner, id: &str, st: &StationStatus) {
    let limits = shared.limits();
    let Some(rt) = g.stations.get_mut(id) else {
        return;
    };
    if !st.name.is_empty() && rt.known.name != st.name {
        rt.known.name = st.name.clone();
        shared.save(g);
    }
    let Some(rt) = g.stations.get_mut(id) else {
        return;
    };
    rt.battery = st.battery;
    rt.charging = st.charging;
    rt.station_listeners = Some(st.listeners);
    rt.camera_on = st.camera;
    rt.talker = st.talker.clone();
    if let Some(percent) = rt.alerts.on_battery(st.battery, st.charging, &limits) {
        shared.alert(g, id, Alert::BatteryLow { percent });
    }
}

/// Routes to try for `id` (FR-NET-3): the configured WebSocket, fresh
/// mDNS results, then stored routes by last success.
fn candidates(shared: &Shared, g: &Inner, id: &str) -> Vec<Route> {
    let mut out: Vec<Route> = Vec::new();
    if let Some(ws) = &shared.cfg.default_websocket {
        out.push(Route::WebSocket(ws.clone()));
    }
    out.extend(discovered_in(g, id));
    if let Some(rt) = g.stations.get(id) {
        out.extend(rt.known.routes_by_success());
        // FR-NET-3 / FR-RV-2: the relay after every direct route.
        out.extend(rt.known.relay.clone().map(Route::Relay));
    }
    let mut seen = Vec::new();
    out.retain(|r| {
        let keep = usable(shared, r) && !seen.contains(r);
        seen.push(r.clone());
        keep
    });
    out
}

/// Whether this build can use `route`. The browser opens WebSockets only,
/// and the relay only on the relay server's own origin (FR-RV-5).
fn usable(shared: &Shared, route: &Route) -> bool {
    if cfg!(feature = "native") {
        return true;
    }
    match route {
        Route::Tcp(_) => false,
        Route::WebSocket(_) => shared.cfg.relay_origin.is_none(),
        Route::Relay(r) => shared
            .cfg
            .relay_origin
            .as_deref()
            .and_then(nestwatch_proto::invite::normalize_relay_origin)
            .is_some_and(|o| o == r.server),
    }
}

#[cfg(feature = "native")]
fn discovered_in(g: &Inner, id: &str) -> Vec<Route> {
    g.discovered
        .values()
        .filter(|d| d.txt.id.as_str() == id)
        .flat_map(|d| {
            d.addrs
                .iter()
                .map(|ip| Route::Tcp(nestwatch_proto::Endpoint::new(*ip, d.port)))
        })
        .collect()
}

#[cfg(not(feature = "native"))]
fn discovered_in(_g: &Inner, _id: &str) -> Vec<Route> {
    Vec::new()
}

/// Discovered (or configured) routes to a station not known yet.
pub(crate) fn discovered_routes(shared: &Shared, id: &str) -> Vec<Route> {
    let g = shared.lock();
    let mut out = discovered_in(&g, id);
    if let Some(ws) = &shared.cfg.default_websocket {
        out.push(Route::WebSocket(ws.clone()));
    }
    out
}

/// Mode `code` with the Argon2id PSK derived off the async executor.
pub(crate) async fn code_mode(
    code: nestwatch_crypto::PairingCode,
    id: nestwatch_crypto::DeviceId,
) -> ListenerMode {
    #[cfg(feature = "native")]
    {
        tokio::task::spawn_blocking(move || ListenerMode::code(&code, id))
            .await
            .expect("Argon2id task panicked")
    }
    #[cfg(not(feature = "native"))]
    {
        ListenerMode::code(&code, id)
    }
}

/// Pairs over the first route that answers (FR-PAIR-1, FR-PAIR-2). On
/// success the session stays up and becomes the station's followed
/// session (FR-PAIR-4).
pub(crate) async fn pair(
    shared: &Arc<Shared>,
    mode: ListenerMode,
    routes: Vec<Route>,
) -> Result<PairResult, ListenerError> {
    let mut list: Vec<Route> = Vec::new();
    for r in routes {
        if usable(shared, &r) && !list.contains(&r) {
            list.push(r);
        }
    }
    if !shared.lock().running {
        return Err(ListenerError::Stopped);
    }
    let mut last = ListenerError::Unreachable("no route to the station".into());
    for route in list.clone() {
        let (tx, rx) = oneshot::channel();
        let s = shared.clone();
        let mode = mode.clone();
        let remember = list.clone();
        rt::spawn(async move {
            let out = attempt(&s, route, mode, None, Some(tx), remember).await;
            if let (Some(id), Some(gen_)) = (out.station_id, out.pairing_gen) {
                if matches!(out.close, Some(CloseReason::Revoked)) {
                    needs_pairing(&s, &id, gen_);
                } else {
                    follow_loop(s, id, gen_).await;
                }
            }
        });
        match rx.await {
            Ok(Ok(r)) => return Ok(r),
            Ok(Err(e @ ListenerError::Unreachable(_))) => last = e,
            Ok(Err(e)) => return Err(e),
            Err(_) => {}
        }
    }
    Err(last)
}

fn following(shared: &Shared, id: &str, gen_: u64) -> bool {
    let g = shared.lock();
    g.running
        && g.stations
            .get(id)
            .is_some_and(|r| r.following && r.generation == gen_)
}

fn set_status(shared: &Shared, id: &str, gen_: u64, status: ConnStatus) {
    let mut g = shared.lock();
    if let Some(rt) = g.stations.get_mut(id)
        && rt.generation == gen_
        && rt.status != ConnStatus::Live
        && rt.status != status
    {
        rt.status = status;
        shared.publish(&mut g);
    }
}

fn progress(ever_live: bool, attempt: u32, tried: &[String]) -> ConnStatus {
    let tried = tried.to_vec();
    if !ever_live && attempt <= 1 {
        ConnStatus::Connecting { attempt, tried }
    } else {
        ConnStatus::Reconnecting { attempt, tried }
    }
}

fn backoff(shared: &Shared, attempt: u32) -> Duration {
    let min = shared.cfg.backoff_min;
    let exp = attempt.saturating_sub(1).min(16);
    min.saturating_mul(1u32 << exp).min(shared.cfg.backoff_max)
}

/// The station revoked us (protocol.md §7.11): stop and mark it.
fn needs_pairing(shared: &Shared, id: &str, gen_: u64) {
    let mut g = shared.lock();
    let Some(rt) = g.stations.get_mut(id) else {
        return;
    };
    if rt.generation != gen_ {
        return;
    }
    rt.following = false;
    rt.known.needs_pairing = true;
    rt.status = ConnStatus::NeedsPairing;
    shared.save(&g);
    shared.emit_after_publish(
        &mut g,
        ListenerEvent::NeedsPairing {
            station_id: id.to_owned(),
        },
    );
    shared.publish(&mut g);
}

/// Keeps one station connected while it is followed with generation
/// `gen_` (FR-LS-5): tries every candidate route, then backs off.
pub(crate) async fn follow_loop(shared: Arc<Shared>, id: String, gen_: u64) {
    let mut attempt_no: u32 = 0;
    let mut ever_live = false;
    loop {
        let (key, routes, wake) = {
            let g = shared.lock();
            match g.stations.get(&id) {
                Some(rt) if g.running && rt.following && rt.generation == gen_ => {
                    (rt.known.key, candidates(&shared, &g, &id), rt.wake.clone())
                }
                _ => return,
            }
        };
        // A live session from pairing (or another loop) may still run.
        let busy = shared
            .lock()
            .stations
            .get(&id)
            .is_some_and(|rt| rt.cmd.is_some() || rt.upgrading);
        if busy {
            let notified = wake.notified();
            tokio::select! {
                () = rt::sleep(Duration::from_millis(200)) => {}
                () = notified => {}
            }
            continue;
        }
        attempt_no += 1;
        let mut tried = Vec::new();
        set_status(&shared, &id, gen_, progress(ever_live, attempt_no, &tried));
        let mut live = false;
        for route in routes {
            if !following(&shared, &id, gen_) {
                return;
            }
            tried.push(route.to_string());
            set_status(&shared, &id, gen_, progress(ever_live, attempt_no, &tried));
            let mode = ListenerMode::Trusted { station: key };
            let out = attempt(&shared, route, mode, Some(id.clone()), None, Vec::new()).await;
            if matches!(out.close, Some(CloseReason::Revoked)) {
                needs_pairing(&shared, &id, gen_);
                return;
            }
            if out.live {
                live = true;
                ever_live = true;
                attempt_no = 0;
                break;
            }
        }
        if !following(&shared, &id, gen_) {
            return;
        }
        let delay = if live {
            shared.cfg.backoff_min
        } else {
            set_status(
                &shared,
                &id,
                gen_,
                ConnStatus::Reconnecting {
                    attempt: attempt_no,
                    tried,
                },
            );
            backoff(&shared, attempt_no)
        };
        let notified = wake.notified();
        tokio::select! {
            () = rt::sleep(delay) => {}
            () = notified => {}
        }
    }
}

/// Browses mDNS and keeps the discovered station list current
/// (protocol.md §3.4, FR-NET-1).
#[cfg(feature = "native")]
pub(crate) fn spawn_discovery(shared: Arc<Shared>) {
    use nestwatch_net::mdns::{Browser, DiscoveryEvent};
    tokio::spawn(async move {
        let Ok(mut browser) = Browser::start() else {
            return;
        };
        loop {
            let ev = tokio::select! {
                e = browser.next() => e,
                () = rt::sleep(Duration::from_secs(1)) => {
                    if !shared.lock().running {
                        break;
                    }
                    continue;
                }
            };
            let Some(ev) = ev else { break };
            let mut g = shared.lock();
            match ev {
                DiscoveryEvent::Found(d) | DiscoveryEvent::Updated(d) => {
                    if let Some(rt) = g.stations.get(d.txt.id.as_str()) {
                        rt.wake.notify_waiters();
                    }
                    g.discovered.insert(d.fullname.clone(), d);
                }
                DiscoveryEvent::Lost { fullname } => {
                    g.discovered.remove(&fullname);
                }
            }
            shared.publish(&mut g);
        }
    });
}
