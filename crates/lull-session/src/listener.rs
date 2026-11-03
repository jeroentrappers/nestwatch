// SPDX-License-Identifier: AGPL-3.0-or-later
//! The listener's session: Noise initiator (protocol.md §4, §5, §7.2).

use lull_crypto::{
    DeviceId, DeviceKey, HandshakeError, Initiator, InviteToken, PairingCode, Prologue, Psk,
    PublicKey, Step, VerificationNumber,
};
use lull_proto::control::{CloseReason, Control, Hello, Role, StationStatus, TalkStatus};
use lull_proto::frame::WsKind;
use lull_proto::preface::{ListenerPreface, Mode, PREFACE_LEN, StationPreface, Status};
use lull_proto::record::{AudioHeader, AudioStream, Record};
use lull_proto::video::{Reassembler, VideoFrame};

use crate::common::{
    self as common, CommonEvent, Core, Flavor, Input, Machine, Stats, check_local_hello, describe,
    listener_to_station, station_to_listener,
};
use crate::error::{ConfigError, ProtocolError, SendError, SessionError};

/// How the listener authenticates the station (protocol.md §4, §5).
#[derive(Debug, Clone)]
pub enum ListenerMode {
    /// Reconnect to a known station (`IK`, §5.1).
    Trusted {
        /// The station's static key from the known station list.
        station: PublicKey,
    },
    /// First pairing with an invitation (`IKpsk2`, §5.2).
    Invite {
        /// `s` of the invitation.
        station: PublicKey,
        /// `t` of the invitation.
        token: InviteToken,
    },
    /// First pairing with a pairing code (`XXpsk3`, §5.3).
    Code {
        /// `id` of the station's discovery record; the station key must
        /// hash to it.
        station_id: DeviceId,
        /// [`PairingCode::psk`] for `station_id`.
        psk: Psk,
    },
}

impl ListenerMode {
    /// Mode `code` from the code the user typed. Runs Argon2id, which is
    /// deliberately slow (§5.3); call it off the UI thread.
    pub fn code(code: &PairingCode, station_id: DeviceId) -> Self {
        Self::Code {
            station_id,
            psk: code.psk(&station_id),
        }
    }

    /// The preface mode byte for this mode.
    pub fn wire_mode(&self) -> Mode {
        match self {
            Self::Trusted { .. } => Mode::Trusted,
            Self::Invite { .. } => Mode::Invite,
            Self::Code { .. } => Mode::Code,
        }
    }
}

/// Everything needed to start a listener session.
#[derive(Debug, Clone)]
pub struct ListenerConfig {
    /// Transport flavour.
    pub flavor: Flavor,
    /// Handshake mode and station credentials.
    pub mode: ListenerMode,
    /// The listener's `hello` (role `listener`, codec 2 listed).
    pub hello: Hello,
}

/// Events of a [`ListenerSession`].
#[derive(Debug, Clone, PartialEq)]
pub enum ListenerEvent {
    /// Pairing modes: the station sent `paired` (§7.10); store it as a
    /// known station. Comes before [`ListenerEvent::Connected`].
    Paired {
        /// The station's authenticated static key.
        station_key: PublicKey,
        /// Its device ID, checked against the key.
        station_id: DeviceId,
    },
    /// Setup finished (§7.2): commands are accepted from now on.
    Connected {
        /// The station's `hello`.
        station: Hello,
        /// Codec of both audio streams (§7.3).
        audio_codec: u8,
        /// The first `status` (§7.6).
        status: StationStatus,
    },
    /// A station→listener control message after setup. `talk-state` also
    /// updates [`ListenerSession::talk_granted`].
    Control(Control),
    /// AUDIO stream 0 with a `seq` newer than any before (§6.1).
    Audio {
        /// Record header.
        header: AudioHeader,
        /// One codec frame.
        payload: Vec<u8>,
    },
    /// A complete, newest video frame (§6.2).
    Video(VideoFrame),
    /// Round-trip time of a PING in ms (§7.9).
    Rtt(u64),
    /// Orderly end (§7.12): `by_peer` if the station sent `close`.
    Closed {
        /// Close reason.
        reason: CloseReason,
        /// Optional detail.
        detail: Option<String>,
        /// Whether the station closed.
        by_peer: bool,
    },
    /// Any other end of the session.
    Failed(SessionError),
}

impl From<CommonEvent> for ListenerEvent {
    fn from(e: CommonEvent) -> Self {
        match e {
            CommonEvent::Rtt(ms) => Self::Rtt(ms),
            CommonEvent::Closed {
                reason,
                detail,
                by_peer,
            } => Self::Closed {
                reason,
                detail,
                by_peer,
            },
            CommonEvent::Failed(err) => Self::Failed(err),
            CommonEvent::VideoFrameReplaced { .. } => {
                unreachable!("listeners never queue video")
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Expect {
    Paired,
    AudioConfig,
    Status,
}

#[derive(Debug)]
enum State {
    AwaitPreface,
    Handshake(Box<Initiator>),
    AwaitHello,
    Setup(Expect),
    Operating,
}

/// A listener's LWP/1 session (Noise initiator). See the crate docs for
/// the driving loop.
#[derive(Debug)]
pub struct ListenerSession {
    core: Core<ListenerEvent>,
    key: DeviceKey,
    mode: ListenerMode,
    hello: Hello,
    preface: [u8; PREFACE_LEN],
    state: State,
    station_key: Option<PublicKey>,
    station_hello: Option<Hello>,
    verification: Option<VerificationNumber>,
    audio_codec: Option<u8>,
    talk_granted: bool,
    last_seq: Option<u32>,
    reassembler: Reassembler,
}

impl ListenerSession {
    /// Starts a session at `now` (ms); the preface is queued for sending.
    pub fn new(config: ListenerConfig, key: &DeviceKey, now: u64) -> Result<Self, ConfigError> {
        check_local_hello(&config.hello, Role::Listener)?;
        let preface = ListenerPreface::new(config.mode.wire_mode()).encode();
        let mut core = Core::new(config.flavor, now);
        core.send_preface(preface);
        Ok(Self {
            core,
            key: DeviceKey::from_secret_bytes(&key.secret_bytes()),
            mode: config.mode,
            hello: config.hello,
            preface,
            state: State::AwaitPreface,
            station_key: None,
            station_hello: None,
            verification: None,
            audio_codec: None,
            talk_granted: false,
            last_seq: None,
            reassembler: Reassembler::new(),
        })
    }

    // ----- I/O driving -----------------------------------------------------

    /// Feeds TCP bytes in any chunking ([`Flavor::Tcp`] only).
    pub fn handle_bytes(&mut self, now: u64, bytes: &[u8]) {
        common::feed_tcp(self, now, bytes);
    }

    /// Feeds one WebSocket message ([`Flavor::WebSocket`] only).
    pub fn handle_ws_message(&mut self, now: u64, kind: WsKind, payload: &[u8]) {
        common::feed_ws(self, now, kind, payload);
    }

    /// The transport ended (EOF or error) without an LWP/1 `close`.
    pub fn handle_connection_lost(&mut self, now: u64) {
        self.core.connection_lost(now);
    }

    /// Next bytes to send: at most `max` stream bytes for TCP, one whole
    /// message for WebSocket. Records are encrypted only when pulled, so
    /// priorities apply at the moment the socket can take data (§8).
    pub fn poll_transmit(&mut self, now: u64, max: usize) -> Option<Vec<u8>> {
        self.core.poll_transmit(now, max)
    }

    /// The next deadline (ms) at which [`handle_timeout`](Self::handle_timeout)
    /// must be called.
    pub fn poll_timeout(&self) -> Option<u64> {
        self.core.poll_timeout()
    }

    /// Processes expired deadlines: keep-alive and timeouts (§4, §7.9, §8).
    pub fn handle_timeout(&mut self, now: u64) {
        self.core.handle_timeout(now);
    }

    /// The next event.
    pub fn poll_event(&mut self) -> Option<ListenerEvent> {
        self.core.poll_event()
    }

    // ----- commands --------------------------------------------------------

    /// Sends a listener→station control message (§7). `hello` and `close`
    /// are managed by the session; use [`close`](Self::close).
    pub fn send_control(&mut self, control: Control) -> Result<(), SendError> {
        self.ready()?;
        if !listener_to_station(&control) {
            return Err(SendError::NotAllowed(control.message_type().to_owned()));
        }
        if control == Control::TalkRelease {
            self.talk_granted = false;
        }
        self.core.queue_control(control)
    }

    /// Queues one AUDIO stream 1 (talk-back) frame; only while
    /// `talk-state` is `granted` (§7.8). `seq` and `timestamp` come from
    /// the capture pipeline.
    pub fn send_talk_audio(
        &mut self,
        header: AudioHeader,
        payload: &[u8],
    ) -> Result<(), SendError> {
        self.ready()?;
        if header.stream != AudioStream::TalkBack {
            return Err(SendError::WrongStream);
        }
        if !self.talk_granted {
            return Err(SendError::TalkNotGranted);
        }
        if Some(header.codec) != self.audio_codec {
            return Err(SendError::WrongCodec(header.codec));
        }
        self.core.queue_audio(header, payload)
    }

    /// Ends the session with `close` (§7.12). Queued control messages are
    /// still sent before it; queued media is dropped.
    pub fn close(&mut self, reason: CloseReason, detail: Option<String>) {
        self.core.close(reason, detail);
    }

    fn ready(&self) -> Result<(), SendError> {
        if !self.core.is_open() {
            Err(SendError::Closed)
        } else if !matches!(self.state, State::Operating) {
            Err(SendError::NotConnected)
        } else {
            Ok(())
        }
    }

    // ----- accessors -------------------------------------------------------

    /// The requested handshake mode.
    pub fn mode(&self) -> &ListenerMode {
        &self.mode
    }

    /// Pairing modes: the verification number (§5.5), once the handshake
    /// completed.
    pub fn verification_number(&self) -> Option<VerificationNumber> {
        self.verification
    }

    /// The station's authenticated static key, once the handshake completed.
    pub fn station_key(&self) -> Option<&PublicKey> {
        self.station_key.as_ref()
    }

    /// The station's `hello`, once received.
    pub fn station_hello(&self) -> Option<&Hello> {
        self.station_hello.as_ref()
    }

    /// The session's audio codec, once `audio-config` arrived.
    pub fn audio_codec(&self) -> Option<u8> {
        self.audio_codec
    }

    /// Whether talk-back is currently granted (§7.8).
    pub fn talk_granted(&self) -> bool {
        self.talk_granted
    }

    /// Whether setup finished and the session is open.
    pub fn is_connected(&self) -> bool {
        self.ready().is_ok()
    }

    /// Whether the session ended and all final bytes were handed out.
    pub fn is_finished(&self) -> bool {
        self.core.is_finished()
    }

    /// Diagnostic counters.
    pub fn stats(&self) -> Stats {
        self.core.stats()
    }

    #[cfg(test)]
    pub(crate) fn set_nonce_limit(&mut self, limit: u64) {
        self.core.set_nonce_limit(limit);
    }

    // ----- protocol --------------------------------------------------------

    fn on_preface(&mut self, raw: [u8; PREFACE_LEN]) {
        let status = match StationPreface::decode(&raw) {
            Ok(p) => p.status,
            Err(e) => return self.core.fail(SessionError::Preface(e), false),
        };
        if status != Status::Ok {
            return self.core.fail(SessionError::Refused(status), false);
        }
        let prologue = Prologue::new(self.preface, raw);
        let initiator = match &self.mode {
            ListenerMode::Trusted { station } => Initiator::trusted(&self.key, station, &prologue),
            ListenerMode::Invite { station, token } => {
                Initiator::invite(&self.key, station, &token.psk(station), &prologue)
            }
            ListenerMode::Code { station_id, psk } => {
                Initiator::code(&self.key, station_id, psk, &prologue)
            }
        };
        let started = initiator.and_then(|mut i| i.start().map(|m| (i, m)));
        match started {
            Ok((initiator, message)) => {
                self.core.send_noise(message);
                self.state = State::Handshake(Box::new(initiator));
            }
            Err(e) => self.core.fail(SessionError::Handshake(e), false),
        }
    }

    fn on_handshake(&mut self, message: &[u8]) {
        let State::Handshake(initiator) = &mut self.state else {
            return;
        };
        match initiator.read_message(message) {
            Ok(Step::Send(reply)) => self.core.send_noise(reply),
            Ok(Step::Complete(done)) => {
                if let Some(last) = done.message {
                    self.core.send_noise(last);
                }
                self.verification = done.transport.verification_number();
                self.station_key = Some(*done.transport.remote_static());
                self.core.set_transport(done.transport);
                self.state = State::AwaitHello;
                // `hello` is the first transport record (§7.2).
                if let Err(e) = self.core.queue_control(Control::Hello(self.hello.clone())) {
                    unreachable!("local hello was validated: {e}");
                }
            }
            Ok(Step::Authorize(_)) => self
                .core
                .fail(SessionError::Handshake(HandshakeError::InvalidState), false),
            Err(e) => self.core.fail(SessionError::Handshake(e), false),
        }
    }

    fn on_record(&mut self, record: Record) {
        // Unknown records and messages are ignored everywhere (§6, §7.1, §10).
        match &record {
            Record::Unknown { .. } | Record::Control(Control::Unknown { .. }) => {
                self.core.stats_mut().ignored += 1;
                return;
            }
            Record::Control(Control::Close { reason, detail }) => {
                return self.core.peer_closed(reason.clone(), detail.clone());
            }
            _ => {}
        }
        match self.state {
            State::AwaitHello => self.on_first_record(record),
            State::Setup(expect) => {
                if let Some(record) = self.core.keepalive_record(record) {
                    self.on_setup_record(expect, record);
                }
            }
            State::Operating => {
                if let Some(record) = self.core.keepalive_record(record) {
                    self.on_operating_record(record);
                }
            }
            State::AwaitPreface | State::Handshake(_) => {}
        }
    }

    fn unexpected(&mut self, expected: &'static str, got: &Record) {
        self.core.protocol_error(ProtocolError::Unexpected {
            expected,
            got: describe(got),
        });
    }

    /// The station's `hello` must come first (§7.2).
    fn on_first_record(&mut self, record: Record) {
        match record {
            Record::Control(Control::Hello(h)) if h.role == Role::Station => {
                self.station_hello = Some(h);
                self.core.enable_keepalive();
                let pairing = self.mode.wire_mode().is_pairing();
                self.state = State::Setup(if pairing {
                    Expect::Paired
                } else {
                    Expect::AudioConfig
                });
            }
            Record::Control(Control::Hello(_)) => {
                self.core.protocol_error(ProtocolError::InvalidHello)
            }
            other => self.unexpected("hello", &other),
        }
    }

    /// After `hello`: `paired` (pairing only), `audio-config`, `status` (§7.2).
    fn on_setup_record(&mut self, expect: Expect, record: Record) {
        match (expect, record) {
            (Expect::Paired, Record::Control(Control::Paired { station_id })) => {
                let key = self.station_key.expect("set with the transport");
                let id = key.device_id();
                if id.as_str() != station_id.as_str() {
                    return self.core.protocol_error(ProtocolError::StationIdMismatch);
                }
                self.core.push_event(ListenerEvent::Paired {
                    station_key: key,
                    station_id: id,
                });
                self.state = State::Setup(Expect::AudioConfig);
            }
            (Expect::AudioConfig, Record::Control(Control::AudioConfig { codec })) => {
                if !self.hello.audio.contains(&codec) {
                    return self
                        .core
                        .protocol_error(ProtocolError::UnofferedCodec(codec));
                }
                self.audio_codec = Some(codec);
                self.state = State::Setup(Expect::Status);
            }
            (Expect::Status, Record::Control(Control::Status(status))) => {
                self.core.setup_complete();
                self.state = State::Operating;
                self.core.push_event(ListenerEvent::Connected {
                    station: self.station_hello.clone().expect("received before setup"),
                    audio_codec: self.audio_codec.expect("received before status"),
                    status,
                });
            }
            (_, Record::Control(Control::Hello(_))) => {
                self.core.protocol_error(ProtocolError::InvalidHello)
            }
            (expect, other) => {
                let expected = match expect {
                    Expect::Paired => "paired",
                    Expect::AudioConfig => "audio-config",
                    Expect::Status => "status",
                };
                self.unexpected(expected, &other);
            }
        }
    }

    fn on_operating_record(&mut self, record: Record) {
        match record {
            Record::Control(Control::Hello(_)) => {
                self.core.protocol_error(ProtocolError::InvalidHello)
            }
            Record::Control(c) if station_to_listener(&c) => {
                if let Control::TalkState { state, .. } = &c {
                    self.talk_granted = *state == TalkStatus::Granted;
                }
                self.core.push_event(ListenerEvent::Control(c));
            }
            Record::Audio { header, payload } if header.stream == AudioStream::Station => {
                // §6.1: discard frames not newer than the last accepted one.
                if self.last_seq.is_some_and(|last| header.seq <= last) {
                    self.core.stats_mut().audio_stale += 1;
                    return;
                }
                self.last_seq = Some(header.seq);
                self.core
                    .push_event(ListenerEvent::Audio { header, payload });
            }
            Record::Video { header, payload } => match self.reassembler.push(&header, &payload) {
                Ok(Some(frame)) => self.core.push_event(ListenerEvent::Video(frame)),
                Ok(None) => {}
                Err(_) => self.core.stats_mut().video_discarded += 1,
            },
            // Wrong-direction messages and streams (SESSION-4).
            _ => self.core.stats_mut().ignored += 1,
        }
    }
}

impl Machine for ListenerSession {
    type Event = ListenerEvent;

    fn core(&mut self) -> &mut Core<ListenerEvent> {
        &mut self.core
    }

    fn on_input(&mut self, input: Input) {
        match input {
            Input::Preface(raw) => self.on_preface(raw),
            Input::Noise(message) => {
                if matches!(self.state, State::Handshake(_)) {
                    self.on_handshake(&message);
                } else if let Some(record) = self.core.decrypt(&message) {
                    self.on_record(record);
                }
            }
        }
    }
}
