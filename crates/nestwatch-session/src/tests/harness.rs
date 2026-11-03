// SPDX-License-Identifier: AGPL-3.0-or-later
//! Deterministic in-memory harness: a listener and a station wired through a
//! simulated link with virtual time, chunking, delay and loss.

use std::collections::VecDeque;
use std::sync::OnceLock;

use nestwatch_crypto::{
    DeviceKey, Initiator, InviteToken, PairingCode, Prologue, Psk, PublicKey, Responder, Step,
    Transport,
};
use nestwatch_proto::control::{Control, Hello, Role, StationStatus, caps};
use nestwatch_proto::frame::{FrameDecoder, WsKind, encode_frame};
use nestwatch_proto::preface::{ListenerPreface, Mode, StationPreface, Status};
use nestwatch_proto::record::{AUDIO_CODEC_ADPCM, AUDIO_CODEC_OPUS, Record, VIDEO_CODEC_JPEG};

use crate::{
    Flavor, ListenerConfig, ListenerEvent, ListenerMode, ListenerSession, StationConfig,
    StationEvent, StationPolicy, StationSession,
};

pub const CODE: &str = "7KQ2M-X9ZRT";

pub fn station_key() -> DeviceKey {
    DeviceKey::from_secret_bytes(&[0x51; 32])
}

pub fn listener_key() -> DeviceKey {
    DeviceKey::from_secret_bytes(&[0x1e; 32])
}

pub fn other_key() -> DeviceKey {
    DeviceKey::from_secret_bytes(&[0x99; 32])
}

pub fn token() -> InviteToken {
    InviteToken::from_bytes(&[0x7a; 16])
}

/// A PSK no honest peer uses (cheap: derived like an invite PSK).
pub fn wrong_psk() -> Psk {
    InviteToken::from_bytes(&[0x00; 16]).psk(station_key().public_key())
}

/// The Argon2id code PSK, derived once per test binary (§5.3).
pub fn code_psk() -> Psk {
    static PSK: OnceLock<Psk> = OnceLock::new();
    PSK.get_or_init(|| {
        PairingCode::parse(CODE)
            .unwrap()
            .psk(&station_key().device_id())
    })
    .clone()
}

pub fn listener_hello() -> Hello {
    Hello {
        name: "Parents' phone".into(),
        role: Role::Listener,
        app: "nestwatch-test/0".into(),
        caps: vec![caps::AUDIO.into(), caps::TALK.into(), caps::VIDEO.into()],
        audio: vec![AUDIO_CODEC_OPUS, AUDIO_CODEC_ADPCM],
        video: vec![VIDEO_CODEC_JPEG],
    }
}

pub fn station_hello() -> Hello {
    Hello {
        name: "Nursery".into(),
        role: Role::Station,
        app: "nestwatch-test/0".into(),
        caps: vec![caps::AUDIO.into(), caps::TALK.into(), caps::VIDEO.into()],
        audio: vec![AUDIO_CODEC_ADPCM],
        video: vec![VIDEO_CODEC_JPEG],
    }
}

pub fn status() -> StationStatus {
    StationStatus {
        name: "Nursery".into(),
        battery: Some(80),
        charging: Some(true),
        listeners: 1,
        camera: false,
        talker: None,
    }
}

/// Records every policy call.
#[derive(Debug)]
pub struct TestPolicy {
    pub trusted: Vec<PublicKey>,
    pub rate_limited: bool,
    pub invite: Option<InviteToken>,
    pub code: Option<Psk>,
    pub status: StationStatus,
    pub paired: Vec<(PublicKey, String)>,
    pub code_failures: u32,
}

impl TestPolicy {
    /// Trusts the test listener; pairing closed.
    pub fn trusting() -> Self {
        Self {
            trusted: vec![*listener_key().public_key()],
            ..Self::closed()
        }
    }

    /// Trusts nobody; pairing closed.
    pub fn closed() -> Self {
        Self {
            trusted: Vec::new(),
            rate_limited: false,
            invite: None,
            code: None,
            status: status(),
            paired: Vec::new(),
            code_failures: 0,
        }
    }

    /// Pairing open with the test token and code.
    pub fn pairing() -> Self {
        Self {
            invite: Some(token()),
            code: Some(code_psk()),
            ..Self::closed()
        }
    }
}

impl StationPolicy for TestPolicy {
    fn is_trusted(&mut self, listener: &PublicKey) -> bool {
        self.trusted.contains(listener)
    }
    fn is_rate_limited(&mut self, _mode: Mode) -> bool {
        self.rate_limited
    }
    fn invite_token(&mut self) -> Option<InviteToken> {
        self.invite.clone()
    }
    fn code_psk(&mut self) -> Option<Psk> {
        self.code.clone()
    }
    fn status(&mut self) -> StationStatus {
        self.status.clone()
    }
    fn paired(&mut self, listener: &PublicKey, name: &str) {
        self.paired.push((*listener, name.to_owned()));
        self.trusted.push(*listener);
    }
    fn code_attempt_failed(&mut self) {
        self.code_failures += 1;
    }
}

pub fn trusted_mode() -> ListenerMode {
    ListenerMode::Trusted {
        station: *station_key().public_key(),
    }
}

pub fn invite_mode() -> ListenerMode {
    ListenerMode::Invite {
        station: *station_key().public_key(),
        token: token(),
    }
}

pub fn code_mode() -> ListenerMode {
    ListenerMode::Code {
        station_id: station_key().device_id(),
        psk: code_psk(),
    }
}

pub fn listener(flavor: Flavor, mode: ListenerMode, now: u64) -> ListenerSession {
    let config = ListenerConfig {
        flavor,
        mode,
        hello: listener_hello(),
    };
    ListenerSession::new(config, &listener_key(), now).unwrap()
}

pub fn station(flavor: Flavor, policy: TestPolicy, now: u64) -> StationSession<TestPolicy> {
    station_with(StationConfig::new(flavor, station_hello()), policy, now)
}

pub fn station_with(
    config: StationConfig,
    policy: TestPolicy,
    now: u64,
) -> StationSession<TestPolicy> {
    StationSession::new(config, &station_key(), policy, now).unwrap()
}

/// A simulated connection between one listener and one station.
#[derive(Debug)]
pub struct Net {
    pub now: u64,
    pub flavor: Flavor,
    pub l: ListenerSession,
    pub s: StationSession<TestPolicy>,
    /// Largest `poll_transmit` request (socket write size).
    pub chunk: usize,
    /// Split each delivered TCP chunk further into pieces of this size.
    pub deliver_chunk: usize,
    /// One-way delay in ms.
    pub delay: u64,
    /// When false, everything sent is silently lost.
    pub link_up: bool,
    /// When false, nothing is pulled from the sessions (blocked sockets).
    pub writable: bool,
    to_s: VecDeque<(u64, Vec<u8>)>,
    to_l: VecDeque<(u64, Vec<u8>)>,
    pub l_events: Vec<ListenerEvent>,
    pub s_events: Vec<StationEvent>,
    /// Every byte the station handed out, in order.
    pub s_sent: Vec<u8>,
}

impl Net {
    pub fn new(flavor: Flavor, mode: ListenerMode, policy: TestPolicy) -> Self {
        let s = station(flavor, policy, 0);
        Self::with(flavor, listener(flavor, mode, 0), s)
    }

    pub fn with(flavor: Flavor, l: ListenerSession, s: StationSession<TestPolicy>) -> Self {
        Self {
            now: 0,
            flavor,
            l,
            s,
            chunk: 64 * 1024,
            deliver_chunk: usize::MAX,
            delay: 0,
            link_up: true,
            writable: true,
            to_s: VecDeque::new(),
            to_l: VecDeque::new(),
            l_events: Vec::new(),
            s_events: Vec::new(),
            s_sent: Vec::new(),
        }
    }

    /// Moves everything that can move at the current time.
    pub fn pump(&mut self) {
        loop {
            let mut progress = false;
            if self.writable {
                while let Some(b) = self.l.poll_transmit(self.now, self.chunk) {
                    if self.link_up {
                        self.to_s.push_back((self.now + self.delay, b));
                    }
                    progress = true;
                }
                while let Some(b) = self.s.poll_transmit(self.now, self.chunk) {
                    self.s_sent.extend_from_slice(&b);
                    if self.link_up {
                        self.to_l.push_back((self.now + self.delay, b));
                    }
                    progress = true;
                }
            }
            while self.to_s.front().is_some_and(|(t, _)| *t <= self.now) {
                let (_, b) = self.to_s.pop_front().unwrap();
                match self.flavor {
                    Flavor::Tcp => {
                        for piece in b.chunks(self.deliver_chunk) {
                            self.s.handle_bytes(self.now, piece);
                        }
                    }
                    Flavor::WebSocket => self.s.handle_ws_message(self.now, WsKind::Binary, &b),
                }
                progress = true;
            }
            while self.to_l.front().is_some_and(|(t, _)| *t <= self.now) {
                let (_, b) = self.to_l.pop_front().unwrap();
                match self.flavor {
                    Flavor::Tcp => {
                        for piece in b.chunks(self.deliver_chunk) {
                            self.l.handle_bytes(self.now, piece);
                        }
                    }
                    Flavor::WebSocket => self.l.handle_ws_message(self.now, WsKind::Binary, &b),
                }
                progress = true;
            }
            while let Some(e) = self.l.poll_event() {
                self.l_events.push(e);
                progress = true;
            }
            while let Some(e) = self.s.poll_event() {
                self.s_events.push(e);
                progress = true;
            }
            if !progress {
                break;
            }
        }
    }

    /// Advances virtual time by `ms`, firing timers and deliveries in order.
    pub fn run_for(&mut self, ms: u64) {
        let target = self.now + ms;
        loop {
            self.pump();
            let next = [
                self.l.poll_timeout(),
                self.s.poll_timeout(),
                self.to_s.front().map(|(t, _)| *t),
                self.to_l.front().map(|(t, _)| *t),
            ]
            .into_iter()
            .flatten()
            .min();
            match next {
                Some(t) if t <= target => {
                    self.now = self.now.max(t);
                    self.l.handle_timeout(self.now);
                    self.s.handle_timeout(self.now);
                }
                _ => break,
            }
        }
        self.now = target;
        self.l.handle_timeout(self.now);
        self.s.handle_timeout(self.now);
        self.pump();
    }

    /// Runs until both sides are connected (or 1 s passed).
    pub fn connect(&mut self) {
        self.run_for(1_000);
        assert!(self.l.is_connected(), "listener: {:?}", self.l_events);
        assert!(self.s.is_connected(), "station: {:?}", self.s_events);
    }

    /// One `poll_transmit(max)` on the station, sent over the link.
    pub fn pull_station_once(&mut self, max: usize) -> usize {
        match self.s.poll_transmit(self.now, max) {
            Some(b) => {
                let n = b.len();
                self.s_sent.extend_from_slice(&b);
                self.to_l.push_back((self.now + self.delay, b));
                n
            }
            None => 0,
        }
    }

    /// The transport broke: both sides learn it.
    pub fn disconnect(&mut self) {
        self.to_s.clear();
        self.to_l.clear();
        self.l.handle_connection_lost(self.now);
        self.s.handle_connection_lost(self.now);
        self.pump();
    }

    pub fn l_failed(&self) -> Option<&crate::SessionError> {
        self.l_events.iter().find_map(|e| match e {
            ListenerEvent::Failed(err) => Some(err),
            _ => None,
        })
    }

    pub fn s_failed(&self) -> Option<&crate::SessionError> {
        self.s_events.iter().find_map(|e| match e {
            StationEvent::Failed(err) => Some(err),
            _ => None,
        })
    }

    pub fn l_audio_seqs(&self) -> Vec<u32> {
        self.l_events
            .iter()
            .filter_map(|e| match e {
                ListenerEvent::Audio { header, .. } => Some(header.seq),
                _ => None,
            })
            .collect()
    }
}

/// Every byte a session currently wants to send.
pub fn drain_l(l: &mut ListenerSession, now: u64) -> Vec<u8> {
    let mut out = Vec::new();
    while let Some(b) = l.poll_transmit(now, usize::MAX) {
        out.extend(b);
    }
    out
}

pub fn drain_s(s: &mut StationSession<TestPolicy>, now: u64) -> Vec<u8> {
    let mut out = Vec::new();
    while let Some(b) = s.poll_transmit(now, usize::MAX) {
        out.extend(b);
    }
    out
}

pub fn frames(bytes: &[u8]) -> Vec<Vec<u8>> {
    let mut d = FrameDecoder::new();
    d.push(bytes);
    let all = d.by_ref().collect::<Result<Vec<_>, _>>().unwrap();
    assert_eq!(d.buffered(), 0, "partial frame left over");
    all
}

pub fn frame(message: &[u8]) -> Vec<u8> {
    encode_frame(message).unwrap()
}

pub fn events_l(l: &mut ListenerSession) -> Vec<ListenerEvent> {
    std::iter::from_fn(|| l.poll_event()).collect()
}

pub fn events_s(s: &mut StationSession<TestPolicy>) -> Vec<StationEvent> {
    std::iter::from_fn(|| s.poll_event()).collect()
}

/// A hand-driven peer holding a Noise transport, for feeding a session
/// records in orders a correct implementation never would (TCP only).
#[derive(Debug)]
pub struct RawPeer {
    pub transport: Transport,
}

impl RawPeer {
    pub fn seal(&mut self, record: &Record) -> Vec<u8> {
        self.seal_plain(&record.encode().unwrap())
    }

    pub fn seal_plain(&mut self, plain: &[u8]) -> Vec<u8> {
        frame(&self.transport.encrypt(plain).unwrap())
    }

    pub fn open_all(&mut self, bytes: &[u8]) -> Vec<Record> {
        frames(bytes)
            .iter()
            .map(|m| Record::decode(&self.transport.decrypt(m).unwrap()).unwrap())
            .collect()
    }
}

/// A listener session talking to a hand-driven station that completed a
/// `trusted` (or, with `pairing`, `invite`) handshake. The listener's
/// `hello` has been consumed; the raw station has sent nothing else.
pub fn listener_vs_raw_station(pairing: bool) -> (ListenerSession, RawPeer) {
    let mode = if pairing {
        invite_mode()
    } else {
        trusted_mode()
    };
    let mut l = listener(Flavor::Tcp, mode, 0);
    let lp: [u8; 6] = drain_l(&mut l, 0).try_into().unwrap();
    assert_eq!(
        lp,
        ListenerPreface::new(if pairing { Mode::Invite } else { Mode::Trusted }).encode()
    );
    let sp = StationPreface::new(Status::Ok).encode();
    l.handle_bytes(0, &sp);
    let msg1 = frames(&drain_l(&mut l, 0)).remove(0);
    let prologue = Prologue::new(lp, sp);
    let key = station_key();
    let mut r = if pairing {
        Responder::invite(&key, &token().psk(key.public_key()), &prologue).unwrap()
    } else {
        Responder::trusted(&key, &prologue).unwrap()
    };
    let mut step = r.read_message(&msg1).unwrap();
    if let Step::Authorize(_) = step {
        step = r.accept().unwrap();
    }
    let Step::Complete(done) = step else {
        panic!("IK completes after message 2")
    };
    l.handle_bytes(0, &frame(&done.message.unwrap()));
    let mut peer = RawPeer {
        transport: done.transport,
    };
    let records = peer.open_all(&drain_l(&mut l, 0));
    assert!(matches!(
        records.as_slice(),
        [Record::Control(Control::Hello(_))]
    ));
    (l, peer)
}

/// Completes setup with the raw station: hello, [paired], audio-config, status.
pub fn raw_station_setup(l: &mut ListenerSession, peer: &mut RawPeer, pairing: bool) {
    let mut bytes = peer.seal(&Record::Control(Control::Hello(station_hello())));
    if pairing {
        let id = nestwatch_proto::DeviceId::parse(station_key().device_id().as_str()).unwrap();
        bytes.extend(peer.seal(&Record::Control(Control::Paired { station_id: id })));
    }
    bytes.extend(peer.seal(&Record::Control(Control::AudioConfig { codec: 2 })));
    bytes.extend(peer.seal(&Record::Control(Control::Status(status()))));
    l.handle_bytes(0, &bytes);
    assert!(l.is_connected(), "{:?}", events_l(l));
}

/// A station session talking to a hand-driven listener after a `trusted`
/// handshake. The station's `hello` has been consumed.
pub fn station_vs_raw_listener() -> (StationSession<TestPolicy>, RawPeer) {
    let mut s = station(Flavor::Tcp, TestPolicy::trusting(), 0);
    let lp = ListenerPreface::new(Mode::Trusted).encode();
    s.handle_bytes(0, &lp);
    let sp: [u8; 6] = drain_s(&mut s, 0).try_into().unwrap();
    let prologue = Prologue::new(lp, sp);
    let key = listener_key();
    let mut i = Initiator::trusted(&key, station_key().public_key(), &prologue).unwrap();
    s.handle_bytes(0, &frame(&i.start().unwrap()));
    let mut out = frames(&drain_s(&mut s, 0));
    let Step::Complete(done) = i.read_message(&out.remove(0)).unwrap() else {
        panic!("IK completes for the initiator after message 2")
    };
    let mut peer = RawPeer {
        transport: done.transport,
    };
    let hello = Record::decode(&peer.transport.decrypt(&out.remove(0)).unwrap()).unwrap();
    assert!(matches!(hello, Record::Control(Control::Hello(_))));
    (s, peer)
}

/// The station's device ID in nestwatch-proto form.
pub fn station_proto_id() -> nestwatch_proto::DeviceId {
    nestwatch_proto::DeviceId::parse(station_key().device_id().as_str()).unwrap()
}
