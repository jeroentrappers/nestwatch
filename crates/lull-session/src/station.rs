// SPDX-License-Identifier: AGPL-3.0-or-later
//! The station's session: Noise responder (protocol.md §4, §5, §7.2).

use core::fmt;

use lull_crypto::{
    DeviceKey, HandshakeError, InviteToken, Prologue, Psk, PublicKey, Responder, Step,
    VerificationNumber,
};
use lull_proto::control::{CloseReason, Control, Hello, Role, StationStatus};
use lull_proto::frame::WsKind;
use lull_proto::preface::{
    ListenerPreface, Mode, PREFACE_LEN, PrefaceError, StationPreface, Status,
};
use lull_proto::record::{AudioHeader, AudioStream, Record};
use lull_proto::video::{self, VideoFrame};

use crate::DEFAULT_VIDEO_FRAGMENT_LEN;
use crate::common::{
    self as common, CommonEvent, Core, Flavor, Input, Machine, Stats, check_local_hello, describe,
    listener_to_station, station_to_listener,
};
use crate::error::{ConfigError, ProtocolError, SendError, SessionError};

/// The station engine's answers and notifications for one session.
///
/// Implementations typically wrap shared engine state (trust store, pairing
/// window). Methods are called synchronously from the session's input
/// methods.
pub trait StationPolicy {
    /// Whether `listener` is in the trusted listener list (§5.1).
    fn is_trusted(&mut self, listener: &PublicKey) -> bool;

    /// Whether to answer `rate-limited` (§4) to a listener asking for `mode`.
    fn is_rate_limited(&mut self, mode: Mode) -> bool;

    /// The open invitation's token, or `None` if pairing mode is closed
    /// (§5.2, FR-PAIR-3).
    fn invite_token(&mut self) -> Option<InviteToken>;

    /// The displayed code's PSK ([`lull_crypto::PairingCode::psk`] for this
    /// station's device ID), or `None` if pairing mode is closed (§5.3).
    /// Derive it once per pairing window: Argon2id per connection would let
    /// any LAN host make the station burn 19 MiB and tens of ms per attempt.
    fn code_psk(&mut self) -> Option<Psk>;

    /// The `status` to send at the end of setup (§7.2, §7.6).
    fn status(&mut self) -> StationStatus;

    /// A pairing listener proved the PSK: its first transport message
    /// (`hello`) decrypted (§5.2). Add it to the trusted list now.
    fn paired(&mut self, listener: &PublicKey, name: &str);

    /// A `code` handshake failed at the listener's PSK-bound message (§5.3).
    /// Close pairing mode after 5 of these.
    fn code_attempt_failed(&mut self);
}

/// Everything needed to start a station session.
#[derive(Debug, Clone)]
pub struct StationConfig {
    /// Transport flavour.
    pub flavor: Flavor,
    /// The station's `hello` (role `station`, codec 2 listed).
    pub hello: Hello,
    /// The listener limit is reached: answer `busy` (§4, §9).
    pub busy: bool,
    /// Largest VIDEO fragment payload (clamped by lull-proto to what fits
    /// one record).
    pub video_fragment_len: usize,
}

impl StationConfig {
    /// Defaults: not busy, [`DEFAULT_VIDEO_FRAGMENT_LEN`].
    pub fn new(flavor: Flavor, hello: Hello) -> Self {
        Self {
            flavor,
            hello,
            busy: false,
            video_fragment_len: DEFAULT_VIDEO_FRAGMENT_LEN,
        }
    }
}

/// Events of a [`StationSession`].
#[derive(Debug, Clone, PartialEq)]
pub enum StationEvent {
    /// Setup finished (§7.2): `paired` (if pairing), `audio-config` and
    /// `status` are queued and commands are accepted.
    Connected {
        /// The listener's authenticated static key.
        listener_key: PublicKey,
        /// The listener's `hello`.
        listener: Hello,
        /// Codec chosen for both audio streams (§7.3).
        audio_codec: u8,
        /// Whether the listener was paired in this session.
        paired: bool,
    },
    /// A listener→station control message after setup.
    Control(Control),
    /// AUDIO stream 1 (talk-back) with a `seq` newer than any before
    /// (§6.1). Whether talk is granted is the engine's decision.
    Audio {
        /// Record header.
        header: AudioHeader,
        /// One codec frame.
        payload: Vec<u8>,
    },
    /// Round-trip time of a PING in ms (§7.9).
    Rtt(u64),
    /// A queued video frame was replaced before it was fully sent; adapt
    /// frame rate and quality (§8, media.md §5).
    VideoFrameReplaced {
        /// The replaced frame.
        frame_id: u32,
    },
    /// Orderly end (§7.12): `by_peer` if the listener sent `close`.
    Closed {
        /// Close reason.
        reason: CloseReason,
        /// Optional detail.
        detail: Option<String>,
        /// Whether the listener closed.
        by_peer: bool,
    },
    /// Any other end of the session.
    Failed(SessionError),
}

impl From<CommonEvent> for StationEvent {
    fn from(e: CommonEvent) -> Self {
        match e {
            CommonEvent::Rtt(ms) => Self::Rtt(ms),
            CommonEvent::VideoFrameReplaced { frame_id } => Self::VideoFrameReplaced { frame_id },
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
        }
    }
}

#[derive(Debug)]
enum State {
    AwaitPreface,
    Handshake(Box<Responder>),
    AwaitHello,
    Operating,
    Ended,
}

/// A station's LWP/1 session with one listener (Noise responder). See the
/// crate docs for the driving loop.
pub struct StationSession<P> {
    core: Core<StationEvent>,
    key: DeviceKey,
    config: StationConfig,
    policy: P,
    state: State,
    mode: Option<Mode>,
    code_msg2_sent: bool,
    listener_key: Option<PublicKey>,
    listener_hello: Option<Hello>,
    verification: Option<VerificationNumber>,
    audio_codec: Option<u8>,
    last_seq: Option<u32>,
}

impl<P> fmt::Debug for StationSession<P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StationSession")
            .field("state", &self.state)
            .field("mode", &self.mode)
            .field("listener_key", &self.listener_key)
            .field("audio_codec", &self.audio_codec)
            .field("core", &self.core)
            .finish_non_exhaustive()
    }
}

impl<P: StationPolicy> StationSession<P> {
    /// Starts a session for a connection accepted at `now` (ms).
    pub fn new(
        config: StationConfig,
        key: &DeviceKey,
        policy: P,
        now: u64,
    ) -> Result<Self, ConfigError> {
        check_local_hello(&config.hello, Role::Station)?;
        let mut core = Core::new(config.flavor, now);
        core.set_preface_deadline(crate::PREFACE_TIMEOUT_MS);
        Ok(Self {
            core,
            key: DeviceKey::from_secret_bytes(&key.secret_bytes()),
            config,
            policy,
            state: State::AwaitPreface,
            mode: None,
            code_msg2_sent: false,
            listener_key: None,
            listener_hello: None,
            verification: None,
            audio_codec: None,
            last_seq: None,
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
    /// message for WebSocket (§8).
    pub fn poll_transmit(&mut self, now: u64, max: usize) -> Option<Vec<u8>> {
        self.core.poll_transmit(now, max)
    }

    /// The next deadline (ms) at which [`handle_timeout`](Self::handle_timeout)
    /// must be called.
    pub fn poll_timeout(&self) -> Option<u64> {
        self.core.poll_timeout()
    }

    /// Processes expired deadlines: preface, setup, keep-alive (§4, §7.9, §8).
    pub fn handle_timeout(&mut self, now: u64) {
        self.core.handle_timeout(now);
    }

    /// The next event.
    pub fn poll_event(&mut self) -> Option<StationEvent> {
        self.core.poll_event()
    }

    // ----- commands --------------------------------------------------------

    /// Queues one AUDIO stream 0 frame (§6.1); the queue keeps the newest
    /// 500 ms (§8). `seq` and `timestamp` come from the capture pipeline.
    pub fn send_audio(&mut self, header: AudioHeader, payload: &[u8]) -> Result<(), SendError> {
        self.ready()?;
        if header.stream != AudioStream::Station {
            return Err(SendError::WrongStream);
        }
        if Some(header.codec) != self.audio_codec {
            return Err(SendError::WrongCodec(header.codec));
        }
        self.core.queue_audio(header, payload)
    }

    /// Queues a video frame, fragmented per §6.2. It replaces the unsent
    /// rest of a queued frame, reported as
    /// [`StationEvent::VideoFrameReplaced`] (§8).
    pub fn send_video(&mut self, frame: &VideoFrame) -> Result<(), SendError> {
        self.ready()?;
        let accepted = self
            .listener_hello
            .as_ref()
            .is_some_and(|h| h.video.contains(&frame.codec));
        if !accepted {
            return Err(SendError::VideoCodecNotAccepted(frame.codec));
        }
        let records = video::fragment(frame, self.config.video_fragment_len)?
            .iter()
            .map(Record::encode)
            .collect::<Result<Vec<_>, _>>()?;
        self.core.queue_video(frame.frame_id, records)
    }

    /// Sends a station→listener control message (§7). `hello`, `paired`,
    /// `audio-config` and `close` are managed by the session.
    pub fn send_control(&mut self, control: Control) -> Result<(), SendError> {
        self.ready()?;
        let managed = matches!(
            control,
            Control::Paired { .. } | Control::AudioConfig { .. }
        );
        if managed || !station_to_listener(&control) {
            return Err(SendError::NotAllowed(control.message_type().to_owned()));
        }
        self.core.queue_control(control)
    }

    /// Ends the session with `close` (§7.12), for example `revoked` (§7.11)
    /// or `replaced` (§9).
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

    /// The mode the listener asked for, once its preface arrived.
    pub fn mode(&self) -> Option<Mode> {
        self.mode
    }

    /// The listener's authenticated static key, once the handshake completed.
    pub fn listener_key(&self) -> Option<&PublicKey> {
        self.listener_key.as_ref()
    }

    /// The listener's `hello`, once received.
    pub fn listener_hello(&self) -> Option<&Hello> {
        self.listener_hello.as_ref()
    }

    /// Pairing modes: the verification number (§5.5).
    pub fn verification_number(&self) -> Option<VerificationNumber> {
        self.verification
    }

    /// The session's audio codec, once chosen.
    pub fn audio_codec(&self) -> Option<u8> {
        self.audio_codec
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

    /// The policy.
    pub fn policy(&self) -> &P {
        &self.policy
    }

    /// The policy, mutably.
    pub fn policy_mut(&mut self) -> &mut P {
        &mut self.policy
    }

    #[cfg(test)]
    pub(crate) fn set_nonce_limit(&mut self, limit: u64) {
        self.core.set_nonce_limit(limit);
    }

    // ----- protocol --------------------------------------------------------

    /// Answers a status other than `ok` and closes (§4).
    fn refuse(&mut self, status: Status) {
        self.core.send_preface(StationPreface::new(status).encode());
        self.core.fail(SessionError::Refused(status), true);
    }

    fn on_preface(&mut self, raw: [u8; PREFACE_LEN]) {
        self.core.preface_received();
        let mode = match ListenerPreface::decode(&raw) {
            Ok(p) => p.mode,
            // PROTO-1: an unknown mode is answered like an unknown version.
            Err(PrefaceError::UnsupportedVersion(_) | PrefaceError::UnknownMode(_)) => {
                return self.refuse(Status::UnsupportedVersion);
            }
            // Not LWP at all: close without answering.
            Err(e) => return self.core.fail(SessionError::Preface(e), false),
        };
        self.mode = Some(mode);
        // SESSION-1: busy, then rate-limited, then pairing-closed.
        if self.config.busy {
            return self.refuse(Status::Busy);
        }
        if self.policy.is_rate_limited(mode) {
            return self.refuse(Status::RateLimited);
        }
        let psk = match mode {
            Mode::Trusted => None,
            Mode::Invite => match self.policy.invite_token() {
                Some(token) => Some(token.psk(self.key.public_key())),
                None => return self.refuse(Status::PairingClosed),
            },
            Mode::Code => match self.policy.code_psk() {
                Some(psk) => Some(psk),
                None => return self.refuse(Status::PairingClosed),
            },
        };
        let answer = StationPreface::new(Status::Ok).encode();
        let prologue = Prologue::new(raw, answer);
        let responder = match (mode, &psk) {
            (Mode::Invite, Some(psk)) => Responder::invite(&self.key, psk, &prologue),
            (Mode::Code, Some(psk)) => Responder::code(&self.key, psk, &prologue),
            _ => Responder::trusted(&self.key, &prologue),
        };
        match responder {
            Ok(r) => {
                self.core.send_preface(answer);
                self.state = State::Handshake(Box::new(r));
            }
            Err(e) => self.core.fail(SessionError::Handshake(e), false),
        }
    }

    fn on_handshake(&mut self, message: &[u8]) {
        let State::Handshake(responder) = &mut self.state else {
            return;
        };
        let step = match responder.read_message(message) {
            // §5.1: unknown keys are rejected by closing, indistinguishably
            // from a network failure.
            Ok(Step::Authorize(listener)) => {
                if self.policy.is_trusted(&listener) {
                    responder.accept()
                } else {
                    if let State::Handshake(r) = std::mem::replace(&mut self.state, State::Ended) {
                        r.reject();
                    }
                    return self.core.fail(SessionError::Untrusted, false);
                }
            }
            other => other,
        };
        match step {
            Ok(Step::Send(reply)) => {
                self.core.send_noise(reply);
                self.code_msg2_sent = self.mode == Some(Mode::Code);
            }
            Ok(Step::Complete(done)) => {
                if let Some(last) = done.message {
                    self.core.send_noise(last);
                }
                self.verification = done.transport.verification_number();
                self.listener_key = Some(*done.transport.remote_static());
                self.core.set_transport(done.transport);
                self.state = State::AwaitHello;
                if let Err(e) = self
                    .core
                    .queue_control(Control::Hello(self.config.hello.clone()))
                {
                    unreachable!("local hello was validated: {e}");
                }
            }
            Ok(Step::Authorize(_)) => {
                self.state = State::Ended;
                self.core
                    .fail(SessionError::Handshake(HandshakeError::InvalidState), false);
            }
            Err(e) => {
                self.state = State::Ended;
                if self.code_msg2_sent {
                    self.policy.code_attempt_failed();
                }
                self.core.fail(SessionError::Handshake(e), false);
            }
        }
    }

    fn on_record(&mut self, record: Record) {
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
            State::Operating => {
                if let Some(record) = self.core.keepalive_record(record) {
                    self.on_operating_record(record);
                }
            }
            _ => {}
        }
    }

    /// The listener's `hello` (§7.2). Only now, after the first transport
    /// message decrypted, is a pairing listener trusted (§5.2).
    fn on_first_record(&mut self, record: Record) {
        let hello = match record {
            Record::Control(Control::Hello(h)) if h.role == Role::Listener => h,
            Record::Control(Control::Hello(_)) => {
                return self.core.protocol_error(ProtocolError::InvalidHello);
            }
            other => {
                return self.core.protocol_error(ProtocolError::Unexpected {
                    expected: "hello",
                    got: describe(&other),
                });
            }
        };
        // §7.3: the first listener codec the station supports.
        let Some(codec) = hello
            .audio
            .iter()
            .copied()
            .find(|c| self.config.hello.audio.contains(c))
        else {
            return self.core.protocol_error(ProtocolError::NoCommonCodec);
        };
        let status = Control::Status(self.policy.status());
        if let Err(e) = status.validate() {
            return self.core.fail(SessionError::LocalMessage(e), false);
        }
        let key = self.listener_key.expect("set with the transport");
        let paired = self.mode.is_some_and(Mode::is_pairing);
        let mut setup = Vec::with_capacity(3);
        if paired {
            self.policy.paired(&key, &hello.name);
            let id = lull_proto::DeviceId::parse(self.key.device_id().as_str())
                .expect("lull-crypto produces canonical device IDs");
            setup.push(Control::Paired { station_id: id });
        }
        setup.push(Control::AudioConfig { codec });
        setup.push(status);
        for message in setup {
            if let Err(e) = self.core.queue_control(message) {
                unreachable!("setup messages are valid: {e}");
            }
        }
        self.audio_codec = Some(codec);
        self.listener_hello = Some(hello.clone());
        self.core.enable_keepalive();
        self.core.setup_complete();
        self.state = State::Operating;
        self.core.push_event(StationEvent::Connected {
            listener_key: key,
            listener: hello,
            audio_codec: codec,
            paired,
        });
    }

    fn on_operating_record(&mut self, record: Record) {
        match record {
            Record::Control(Control::Hello(_)) => {
                self.core.protocol_error(ProtocolError::InvalidHello)
            }
            Record::Control(c) if listener_to_station(&c) => {
                self.core.push_event(StationEvent::Control(c));
            }
            Record::Audio { header, payload } if header.stream == AudioStream::TalkBack => {
                // §6.1: discard frames not newer than the last accepted one.
                if self.last_seq.is_some_and(|last| header.seq <= last) {
                    self.core.stats_mut().audio_stale += 1;
                    return;
                }
                self.last_seq = Some(header.seq);
                self.core
                    .push_event(StationEvent::Audio { header, payload });
            }
            // Wrong-direction messages, streams and VIDEO (SESSION-4).
            _ => self.core.stats_mut().ignored += 1,
        }
    }
}

impl<P: StationPolicy> Machine for StationSession<P> {
    type Event = StationEvent;

    fn core(&mut self) -> &mut Core<StationEvent> {
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
