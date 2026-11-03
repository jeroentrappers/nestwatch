// SPDX-License-Identifier: AGPL-3.0-or-later
//! Shared helpers: an in-process server on ephemeral loopback ports, and
//! real NWP/1 station and listener sessions (nestwatch-session) driven over
//! any `Link`.
#![allow(dead_code)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use nestwatch_crypto::{DeviceKey, InviteToken, Psk, PublicKey};
use nestwatch_net::{Link, LinkKind};
use nestwatch_proto::control::{CloseReason, Hello, Role, StationStatus, caps};
use nestwatch_proto::preface::Mode;
use nestwatch_proto::record::{AUDIO_CODEC_ADPCM, AudioFlags, AudioHeader, AudioStream};
use nestwatch_relay::{
    MailboxIdentity, RegistrationConfig, RegistrationStatus, RelayConfig, RelayServer,
    StationRegistration,
};
use nestwatch_server::{Config, Server};
use nestwatch_session::{
    Flavor, ListenerConfig, ListenerEvent, ListenerMode, ListenerSession, StationConfig,
    StationEvent, StationPolicy, StationSession,
};
use tokio::sync::watch;

pub const WAIT: Duration = Duration::from_secs(20);

/// A test configuration: loopback ports chosen by the OS, no log lines.
pub fn test_config(state: &std::path::Path) -> Config {
    let mut c = Config {
        http: "127.0.0.1:0".parse().unwrap(),
        stun: Some("127.0.0.1:0".parse().unwrap()),
        state_dir: state.to_owned(),
        log_interval: 0.0,
        ..Config::default()
    };
    // Tests connect many times from 127.0.0.1.
    c.limits.connect_per_minute = 1000;
    c.limits.register_per_minute = 1000;
    c
}

pub struct TestServer {
    pub server: Server,
    pub state: tempfile::TempDir,
}

impl TestServer {
    pub async fn start(tweak: impl FnOnce(&mut Config)) -> Self {
        let state = tempfile::tempdir().unwrap();
        Self::start_in(state, tweak).await
    }

    pub async fn start_in(state: tempfile::TempDir, tweak: impl FnOnce(&mut Config)) -> Self {
        let mut c = test_config(state.path());
        tweak(&mut c);
        let server = Server::start(c).await.expect("server starts");
        Self { server, state }
    }

    pub fn relay(&self) -> RelayConfig {
        let origin = format!("http://{}", self.server.http_addr());
        let mut server = RelayServer::parse(&origin).unwrap();
        if let Some(s) = self.server.stun_addr() {
            server = server.with_stun_port(s.port());
        }
        let mut c = RelayConfig::new(server);
        c.timeout = Duration::from_secs(5);
        c
    }

    pub fn registration(&self, identity: MailboxIdentity) -> StationRegistration {
        self.registration_with(identity, |_| {})
    }

    pub fn registration_with(
        &self,
        identity: MailboxIdentity,
        tweak: impl FnOnce(&mut RegistrationConfig),
    ) -> StationRegistration {
        let mut c = RegistrationConfig::new(self.relay());
        c.backoff_min = Duration::from_millis(50);
        c.backoff_max = Duration::from_millis(200);
        tweak(&mut c);
        StationRegistration::start(c, identity)
    }

    pub async fn stop(self) -> tempfile::TempDir {
        self.server.shutdown().await;
        self.state
    }
}

/// Waits until the registration status satisfies `f`.
pub async fn wait_status(
    reg: &StationRegistration,
    f: impl Fn(&RegistrationStatus) -> bool,
) -> RegistrationStatus {
    let mut rx: watch::Receiver<RegistrationStatus> = reg.status();
    let r = tokio::time::timeout(WAIT, rx.wait_for(|s| f(s)))
        .await
        .expect("status in time")
        .expect("registration alive");
    r.clone()
}

pub async fn registered(reg: &StationRegistration) -> String {
    match wait_status(reg, |s| matches!(s, RegistrationStatus::Registered { .. })).await {
        RegistrationStatus::Registered { observed } => observed,
        _ => unreachable!(),
    }
}

// ----- NWP/1 sessions ------------------------------------------------------

pub fn station_key() -> DeviceKey {
    DeviceKey::from_secret_bytes(&[0x51; 32])
}

pub fn listener_key() -> DeviceKey {
    DeviceKey::from_secret_bytes(&[0x1e; 32])
}

pub fn token() -> InviteToken {
    InviteToken::from_bytes(&[0x7a; 16])
}

fn hello(role: Role, name: &str) -> Hello {
    Hello {
        name: name.into(),
        role,
        app: "nestwatch-server-test/0".into(),
        caps: vec![caps::AUDIO.into()],
        audio: vec![AUDIO_CODEC_ADPCM],
        video: vec![],
    }
}

/// Station policy: a shared trust list and an optional open invitation.
#[derive(Debug, Clone, Default)]
pub struct Trust {
    pub trusted: Arc<Mutex<Vec<PublicKey>>>,
    pub invite: Option<InviteToken>,
}

impl StationPolicy for Trust {
    fn is_trusted(&mut self, l: &PublicKey) -> bool {
        self.trusted.lock().unwrap().contains(l)
    }
    fn is_rate_limited(&mut self, _: Mode) -> bool {
        false
    }
    fn invite_token(&mut self) -> Option<InviteToken> {
        self.invite.clone()
    }
    fn code_psk(&mut self) -> Option<Psk> {
        None
    }
    fn status(&mut self) -> StationStatus {
        StationStatus {
            name: "Nursery".into(),
            battery: None,
            charging: None,
            listeners: 1,
            camera: false,
            talker: None,
        }
    }
    fn paired(&mut self, l: &PublicKey, _: &str) {
        self.trusted.lock().unwrap().push(*l);
    }
    fn code_attempt_failed(&mut self) {}
}

fn now(start: tokio::time::Instant) -> u64 {
    start.elapsed().as_millis() as u64
}

fn flavor(link: &Link) -> Flavor {
    match link.kind() {
        LinkKind::Stream => Flavor::Tcp,
        LinkKind::Messages => Flavor::WebSocket,
    }
}

/// What a station session did.
#[derive(Debug, Default)]
pub struct StationReport {
    pub connected: bool,
    pub paired: bool,
    pub frames_sent: u32,
    pub close: Option<CloseReason>,
    pub failed: Option<String>,
}

/// What a listener session did.
#[derive(Debug, Default)]
pub struct ListenerReport {
    pub paired: bool,
    pub connected: bool,
    pub frames: u32,
    pub first_seq: Option<u32>,
    pub last_seq: Option<u32>,
    pub failed: Option<String>,
}

/// The I/O side of both session types.
trait Drive {
    fn feed(&mut self, now: u64, kind: LinkKind, data: &[u8]);
    fn lost(&mut self, now: u64);
    fn transmit(&mut self, now: u64) -> Option<Vec<u8>>;
    fn timeout(&mut self, now: u64);
    fn finished(&self) -> bool;
}

macro_rules! drive {
    ($t:ty) => {
        impl Drive for $t {
            fn feed(&mut self, now: u64, kind: LinkKind, data: &[u8]) {
                match kind {
                    LinkKind::Stream => self.handle_bytes(now, data),
                    LinkKind::Messages => {
                        self.handle_ws_message(now, nestwatch_proto::frame::WsKind::Binary, data)
                    }
                }
            }
            fn lost(&mut self, now: u64) {
                self.handle_connection_lost(now);
            }
            fn transmit(&mut self, now: u64) -> Option<Vec<u8>> {
                self.poll_transmit(now, 16 * 1024)
            }
            fn timeout(&mut self, now: u64) {
                self.handle_timeout(now);
            }
            fn finished(&self) -> bool {
                self.is_finished()
            }
        }
    };
}

drive!(ListenerSession);
drive!(StationSession<Trust>);

/// Drives a session over `link`: received data in, `poll_transmit` out,
/// timers every 20 ms. `step` runs after every turn.
async fn pump<S: Drive>(s: &mut S, mut link: Link, mut step: impl FnMut(&mut S, u64)) {
    let start = tokio::time::Instant::now();
    let kind = link.kind();
    let mut tick = tokio::time::interval(Duration::from_millis(20));
    loop {
        step(s, now(start));
        while let Some(b) = s.transmit(now(start)) {
            if link.send(&b).await.is_err() {
                s.lost(now(start));
                break;
            }
        }
        if s.finished() {
            break;
        }
        tokio::select! {
            r = link.recv() => match r {
                Ok(Some(b)) => s.feed(now(start), kind, &b),
                Ok(None) | Err(_) => s.lost(now(start)),
            },
            _ = tick.tick() => s.timeout(now(start)),
        }
    }
    step(s, now(start));
    let _ = tokio::time::timeout(Duration::from_secs(2), link.close()).await;
}

/// Runs a station session; once connected it sends `frames` AUDIO frames
/// (one per 20 ms tick), then keeps the session up until the listener
/// closes it.
pub async fn run_station(link: Link, policy: Trust, frames: u32) -> StationReport {
    let cfg = StationConfig::new(flavor(&link), hello(Role::Station, "Nursery"));
    let mut s = StationSession::new(cfg, &station_key(), policy, 0).unwrap();
    let mut report = StationReport::default();
    let mut codec = AUDIO_CODEC_ADPCM;
    pump(&mut s, link, |s, n| {
        while let Some(e) = s.poll_event() {
            match e {
                StationEvent::Connected {
                    audio_codec,
                    paired,
                    ..
                } => {
                    report.connected = true;
                    report.paired = paired;
                    codec = audio_codec;
                }
                StationEvent::Closed { reason, .. } => report.close = Some(reason),
                StationEvent::Failed(e) => report.failed = Some(e.to_string()),
                _ => {}
            }
        }
        if report.connected && report.frames_sent < frames && s.is_connected() {
            let header = AudioHeader {
                stream: AudioStream::Station,
                codec,
                flags: AudioFlags(0),
                seq: report.frames_sent,
                timestamp: (n as u32).wrapping_mul(48),
            };
            if s.send_audio(header, &[0x5a; 80]).is_ok() {
                report.frames_sent += 1;
            }
        }
    })
    .await;
    report
}

/// Runs a listener session until it received `want` AUDIO frames, then
/// closes it with `shutdown`.
pub async fn run_listener(link: Link, mode: ListenerMode, want: u32) -> ListenerReport {
    let cfg = ListenerConfig {
        flavor: flavor(&link),
        mode,
        hello: hello(Role::Listener, "Parents"),
    };
    let mut s = ListenerSession::new(cfg, &listener_key(), 0).unwrap();
    let mut report = ListenerReport::default();
    let mut closing = false;
    pump(&mut s, link, |s, _| {
        while let Some(e) = s.poll_event() {
            match e {
                ListenerEvent::Paired { .. } => report.paired = true,
                ListenerEvent::Connected { .. } => report.connected = true,
                ListenerEvent::Audio { header, .. } => {
                    report.frames += 1;
                    report.first_seq.get_or_insert(header.seq);
                    report.last_seq = Some(header.seq);
                }
                ListenerEvent::Failed(e) => report.failed = Some(e.to_string()),
                _ => {}
            }
        }
        if report.frames >= want && !closing {
            closing = true;
            s.close(CloseReason::Shutdown, None);
        }
    })
    .await;
    report
}

pub fn invite_mode() -> ListenerMode {
    ListenerMode::Invite {
        station: *station_key().public_key(),
        token: token(),
    }
}

pub fn trusted_mode() -> ListenerMode {
    ListenerMode::Trusted {
        station: *station_key().public_key(),
    }
}
