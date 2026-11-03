// SPDX-License-Identifier: AGPL-3.0-or-later
//! Machinery shared by both roles: transport flavours (protocol.md §3),
//! the record queues and scheduler (§8), keep-alive (§7.9), deadlines (§4)
//! and the nonce limit (§5.6).

use std::collections::VecDeque;

use nestwatch_crypto::{NONCE_LIMIT, Transport};
use nestwatch_proto::control::{CloseReason, Control};
use nestwatch_proto::frame::{self, FrameDecoder, WsKind};
use nestwatch_proto::preface::PREFACE_LEN;
use nestwatch_proto::record::{AudioHeader, Record};

use crate::error::{ProtocolError, SendError, SessionError, TimeoutKind};
use crate::{
    AUDIO_QUEUE_FRAMES, CONTROL_STALL_MS, IDLE_TIMEOUT_MS, NONCE_LIMIT_REASON, PING_INTERVAL_MS,
};

/// The byte transport carrying the session (protocol.md §3.2, §3.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Flavor {
    /// A byte stream: the 6 preface bytes, then `u16` length-prefixed Noise
    /// messages. Feed with `handle_bytes`; `poll_transmit` returns stream
    /// bytes that may split frames anywhere.
    Tcp,
    /// Binary messages: the preface as one message, then one Noise message
    /// per message. Feed with `handle_ws_message`; `poll_transmit` returns
    /// one whole message per call.
    WebSocket,
}

/// Counters for diagnostics and media adaptation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Stats {
    /// AUDIO frames dropped from a full queue (protocol.md §8).
    pub audio_dropped: u64,
    /// Queued VIDEO frames replaced by a newer one (protocol.md §8).
    pub video_replaced: u64,
    /// Received AUDIO frames discarded for a non-increasing `seq` (§6.1).
    pub audio_stale: u64,
    /// Received VIDEO frames discarded as inconsistent (PROTO-9).
    pub video_discarded: u64,
    /// Received records or control messages ignored as unknown (§6, §7.1).
    pub ignored: u64,
}

/// One unit of input after transport framing.
#[derive(Debug)]
pub(crate) enum Input {
    Preface([u8; PREFACE_LEN]),
    Noise(Vec<u8>),
}

/// Events both roles emit.
#[derive(Debug)]
pub(crate) enum CommonEvent {
    Rtt(u64),
    VideoFrameReplaced {
        frame_id: u32,
    },
    Closed {
        reason: CloseReason,
        detail: Option<String>,
        by_peer: bool,
    },
    Failed(SessionError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Life {
    /// Running normally.
    Open,
    /// The terminal event was emitted; remaining output (ending with a
    /// `close` record) is being flushed.
    Closing,
    /// Nothing more to do.
    Done,
}

#[derive(Debug)]
struct QueuedAudio {
    preroll: bool,
    record: Vec<u8>,
}

#[derive(Debug)]
struct QueuedControl {
    record: Vec<u8>,
    is_close: bool,
}

/// Shared state of one session; `E` is the role's event type.
#[derive(Debug)]
pub(crate) struct Core<E> {
    flavor: Flavor,
    now: u64,
    life: Life,
    // Input.
    preface_seen: bool,
    prebuf: Vec<u8>,
    decoder: FrameDecoder,
    // Output, encrypted and framed, not yet handed to the I/O layer.
    out: Vec<u8>,
    out_pos: usize,
    out_msgs: VecDeque<Vec<u8>>,
    // Plaintext record queues (protocol.md §8).
    transport: Option<Transport>,
    control_q: VecDeque<QueuedControl>,
    audio_q: VecDeque<QueuedAudio>,
    /// Remaining fragments of the frame being sent.
    video_q: VecDeque<Vec<u8>>,
    video_frame: Option<u32>,
    /// Whether a fragment of `video_frame` was already sent.
    video_started: bool,
    /// The one complete frame waiting behind a partly sent one (SESSION-5).
    video_next: Option<(u32, Vec<Vec<u8>>)>,
    // Timing.
    last_rx: u64,
    last_tx: u64,
    preface_deadline: Option<u64>,
    setup_deadline: Option<u64>,
    keepalive: bool,
    control_progress: Option<u64>,
    closing_deadline: Option<u64>,
    ping_outstanding: Option<u64>,
    nonce_limit: u64,
    stats: Stats,
    events: VecDeque<E>,
}

impl<E: From<CommonEvent>> Core<E> {
    pub(crate) fn new(flavor: Flavor, now: u64) -> Self {
        Self {
            flavor,
            now,
            life: Life::Open,
            preface_seen: false,
            prebuf: Vec::new(),
            decoder: FrameDecoder::new(),
            out: Vec::new(),
            out_pos: 0,
            out_msgs: VecDeque::new(),
            transport: None,
            control_q: VecDeque::new(),
            audio_q: VecDeque::new(),
            video_q: VecDeque::new(),
            video_frame: None,
            video_started: false,
            video_next: None,
            last_rx: now,
            last_tx: now,
            preface_deadline: None,
            setup_deadline: Some(now + crate::SETUP_TIMEOUT_MS),
            keepalive: false,
            control_progress: None,
            closing_deadline: None,
            ping_outstanding: None,
            nonce_limit: NONCE_LIMIT,
            stats: Stats::default(),
            events: VecDeque::new(),
        }
    }

    // ----- accessors -------------------------------------------------------

    pub(crate) fn is_open(&self) -> bool {
        self.life == Life::Open
    }

    pub(crate) fn is_finished(&self) -> bool {
        self.life == Life::Done
    }

    pub(crate) fn stats(&self) -> Stats {
        self.stats
    }

    pub(crate) fn stats_mut(&mut self) -> &mut Stats {
        &mut self.stats
    }

    pub(crate) fn set_transport(&mut self, transport: Transport) {
        self.transport = Some(transport);
    }

    pub(crate) fn push_event(&mut self, event: E) {
        self.events.push_back(event);
    }

    pub(crate) fn poll_event(&mut self) -> Option<E> {
        self.events.pop_front()
    }

    pub(crate) fn set_preface_deadline(&mut self, ms: u64) {
        self.preface_deadline = Some(self.now + ms);
    }

    /// Both `hello`s were exchanged: keep-alive starts (protocol.md §7.9).
    pub(crate) fn enable_keepalive(&mut self) {
        self.keepalive = true;
    }

    /// Setup finished: the setup deadline no longer applies.
    pub(crate) fn setup_complete(&mut self) {
        self.setup_deadline = None;
    }

    #[cfg(test)]
    pub(crate) fn set_nonce_limit(&mut self, limit: u64) {
        self.nonce_limit = limit;
    }

    // ----- input -----------------------------------------------------------

    /// Common entry for every input call. Returns whether to process it.
    pub(crate) fn begin_input(&mut self, now: u64, flavor: Flavor, nonempty: bool) -> bool {
        self.advance(now);
        if self.life != Life::Open {
            return false;
        }
        if flavor != self.flavor {
            self.fail(SessionError::FlavorMismatch, false);
            return false;
        }
        if nonempty {
            self.last_rx = self.now;
        }
        true
    }

    /// Appends TCP stream bytes.
    pub(crate) fn tcp_push(&mut self, bytes: &[u8]) {
        if self.preface_seen {
            self.decoder.push(bytes);
        } else {
            self.prebuf.extend_from_slice(bytes);
        }
    }

    /// The next framed TCP input, if complete.
    pub(crate) fn tcp_next(&mut self) -> Result<Option<Input>, ProtocolError> {
        if !self.preface_seen {
            if self.prebuf.len() < PREFACE_LEN {
                return Ok(None);
            }
            self.preface_seen = true;
            let rest = self.prebuf.split_off(PREFACE_LEN);
            let raw: [u8; PREFACE_LEN] = self.prebuf[..].try_into().expect("6 bytes");
            self.prebuf = Vec::new();
            self.decoder.push(&rest);
            return Ok(Some(Input::Preface(raw)));
        }
        Ok(self.decoder.next_message()?.map(Input::Noise))
    }

    /// Converts one WebSocket message into input (protocol.md §3.3).
    pub(crate) fn ws_input(
        &mut self,
        kind: WsKind,
        payload: &[u8],
    ) -> Result<Input, ProtocolError> {
        if kind == WsKind::Text {
            return Err(frame::FrameError::TextMessage.into());
        }
        if !self.preface_seen {
            let raw: [u8; PREFACE_LEN] = payload
                .try_into()
                .map_err(|_| ProtocolError::PrefaceLength)?;
            self.preface_seen = true;
            return Ok(Input::Preface(raw));
        }
        Ok(Input::Noise(
            frame::ws_noise_message(kind, payload)?.to_vec(),
        ))
    }

    /// Decrypts and parses one transport message. On failure the session
    /// has already been terminated.
    pub(crate) fn decrypt(&mut self, message: &[u8]) -> Option<Record> {
        let transport = self.transport.as_mut()?;
        let plain = match transport.decrypt(message) {
            Ok(p) => p,
            Err(e) => {
                // The transport is unusable now; no close can be sent.
                self.fail(SessionError::Transport(e), false);
                return None;
            }
        };
        match Record::decode(&plain) {
            Ok(r) => Some(r),
            Err(e) => {
                self.protocol_error(ProtocolError::MalformedRecord(e));
                None
            }
        }
    }

    /// Runs after each input: graceful close when the receiving nonce is
    /// used up (protocol.md §5.6).
    pub(crate) fn after_input(&mut self) {
        if self.life == Life::Open
            && self
                .transport
                .as_ref()
                .is_some_and(|t| t.receiving_nonce() >= self.nonce_limit)
        {
            self.nonce_close();
        }
    }

    /// Handles PING and PONG; returns other records unchanged.
    pub(crate) fn keepalive_record(&mut self, record: Record) -> Option<Record> {
        match record {
            Record::Ping(t) => {
                self.queue_record(&Record::Pong(t), false);
                None
            }
            Record::Pong(t) => {
                if self.ping_outstanding == Some(t) && t <= self.now {
                    self.ping_outstanding = None;
                    self.events.push_back(CommonEvent::Rtt(self.now - t).into());
                }
                None
            }
            other => Some(other),
        }
    }

    // ----- output ----------------------------------------------------------

    /// Queues raw preface bytes (before any Noise message).
    pub(crate) fn send_preface(&mut self, preface: [u8; PREFACE_LEN]) {
        match self.flavor {
            Flavor::Tcp => self.out.extend_from_slice(&preface),
            Flavor::WebSocket => self.out_msgs.push_back(preface.to_vec()),
        }
    }

    /// Queues one handshake (or transport) Noise message.
    pub(crate) fn send_noise(&mut self, message: Vec<u8>) {
        match self.flavor {
            Flavor::Tcp => {
                frame::encode_frame_into(&message, &mut self.out)
                    .expect("Noise messages are 1..=65535 bytes");
            }
            Flavor::WebSocket => self.out_msgs.push_back(message),
        }
    }

    fn queue_record(&mut self, record: &Record, is_close: bool) {
        let record = record.encode().expect("session-built records are valid");
        self.push_control(record, is_close);
    }

    fn push_control(&mut self, record: Vec<u8>, is_close: bool) {
        if self.control_q.is_empty() {
            self.control_progress = Some(self.now);
        }
        self.control_q.push_back(QueuedControl { record, is_close });
    }

    /// Queues a CONTROL record (highest priority, protocol.md §8).
    pub(crate) fn queue_control(&mut self, control: Control) -> Result<(), SendError> {
        if self.life != Life::Open {
            return Err(SendError::Closed);
        }
        let record = Record::Control(control).encode()?;
        self.push_control(record, false);
        Ok(())
    }

    /// Queues an AUDIO record; drops the oldest frame (pre-roll first,
    /// media.md §3) when 500 ms are already queued (protocol.md §8).
    pub(crate) fn queue_audio(
        &mut self,
        header: AudioHeader,
        payload: &[u8],
    ) -> Result<(), SendError> {
        if self.life != Life::Open {
            return Err(SendError::Closed);
        }
        let record = Record::Audio {
            header,
            payload: payload.to_vec(),
        }
        .encode()?;
        if self.audio_q.len() >= AUDIO_QUEUE_FRAMES {
            let victim = self.audio_q.iter().position(|a| a.preroll).unwrap_or(0);
            self.audio_q.remove(victim);
            self.stats.audio_dropped += 1;
        }
        self.audio_q.push_back(QueuedAudio {
            preroll: header.flags.preroll(),
            record,
        });
        Ok(())
    }

    /// Queues the VIDEO records of one frame (protocol.md §8). At most one
    /// complete frame waits: a newer frame replaces it and the replacement
    /// is reported (media.md §5). A frame whose first fragment already left
    /// is finished first, so a slow link still shows complete frames
    /// (SESSION-5).
    pub(crate) fn queue_video(
        &mut self,
        frame_id: u32,
        records: Vec<Vec<u8>>,
    ) -> Result<(), SendError> {
        if self.life != Life::Open {
            return Err(SendError::Closed);
        }
        let replaced = if self.video_started {
            self.video_next
                .replace((frame_id, records))
                .map(|(old, _)| old)
        } else {
            let old = self.video_frame.replace(frame_id);
            self.video_q = records.into();
            old
        };
        if let Some(old) = replaced {
            self.stats.video_replaced += 1;
            self.events
                .push_back(CommonEvent::VideoFrameReplaced { frame_id: old }.into());
        }
        Ok(())
    }

    fn clear_video(&mut self) {
        self.video_q.clear();
        self.video_frame = None;
        self.video_started = false;
        self.video_next = None;
    }

    fn wire_pending(&self) -> usize {
        match self.flavor {
            Flavor::Tcp => self.out.len() - self.out_pos,
            Flavor::WebSocket => self.out_msgs.len(),
        }
    }

    fn nothing_pending(&self) -> bool {
        self.wire_pending() == 0
            && self.control_q.is_empty()
            && self.audio_q.is_empty()
            && self.video_q.is_empty()
    }

    /// Encrypts the next record by priority onto the wire. Returns `false`
    /// when there is nothing (more) to send.
    fn fill_one(&mut self) -> bool {
        if self.life == Life::Done || self.transport.is_none() {
            return false;
        }
        let (record, is_close) = if let Some(c) = self.control_q.pop_front() {
            self.control_progress = (!self.control_q.is_empty()).then_some(self.now);
            (c.record, c.is_close)
        } else if let Some(a) = self.audio_q.pop_front() {
            (a.record, false)
        } else if let Some(v) = self.video_q.pop_front() {
            self.video_started = true;
            if self.video_q.is_empty() {
                self.video_started = false;
                self.video_frame = None;
                if let Some((id, next)) = self.video_next.take() {
                    self.video_frame = Some(id);
                    self.video_q = next.into();
                }
            }
            (v, false)
        } else {
            return false;
        };
        let transport = self.transport.as_mut().expect("checked above");
        // Keep the last usable nonce for the `close` record (§5.6).
        if !is_close && transport.sending_nonce() + 1 >= self.nonce_limit {
            if self.life == Life::Open {
                self.nonce_close();
            } else {
                // Already closing: send nothing but the close.
                self.control_q.retain(|c| c.is_close);
            }
            return true;
        }
        match transport.encrypt(&record) {
            Ok(message) => self.send_noise(message),
            Err(e) => {
                self.fail(SessionError::Transport(e), false);
                return false;
            }
        }
        if is_close {
            self.control_q.clear();
        }
        true
    }

    /// Hands up to `max` bytes (TCP) or one message (WebSocket) to the I/O
    /// layer.
    pub(crate) fn poll_transmit(&mut self, now: u64, max: usize) -> Option<Vec<u8>> {
        self.advance(now);
        if self.life == Life::Done || max == 0 {
            return None;
        }
        let chunk = match self.flavor {
            Flavor::Tcp => {
                while self.out.len() - self.out_pos < max && self.fill_one() {}
                let avail = self.out.len() - self.out_pos;
                if avail == 0 {
                    None
                } else {
                    let n = avail.min(max);
                    let chunk = self.out[self.out_pos..self.out_pos + n].to_vec();
                    self.out_pos += n;
                    if self.out_pos == self.out.len() {
                        self.out.clear();
                        self.out_pos = 0;
                    }
                    Some(chunk)
                }
            }
            Flavor::WebSocket => {
                while self.out_msgs.is_empty() && self.fill_one() {}
                self.out_msgs.pop_front()
            }
        };
        if chunk.is_some() {
            self.last_tx = self.now;
        }
        self.check_done();
        chunk
    }

    fn check_done(&mut self) {
        if self.life == Life::Closing
            && self.wire_pending() == 0
            && (self.control_q.is_empty() || self.transport.is_none())
        {
            self.finish();
        }
    }

    fn finish(&mut self) {
        self.life = Life::Done;
        self.out.clear();
        self.out_pos = 0;
        self.out_msgs.clear();
        self.control_q.clear();
        self.audio_q.clear();
        self.clear_video();
        self.transport = None;
    }

    // ----- termination -----------------------------------------------------

    /// Emits the terminal event. With `flush`, output already produced (and
    /// a `close` record, if given and the transport is up) is still sent;
    /// otherwise everything is dropped.
    pub(crate) fn terminate(
        &mut self,
        event: CommonEvent,
        close: Option<(CloseReason, Option<String>)>,
        flush: bool,
    ) {
        if self.life != Life::Open {
            return;
        }
        self.events.push_back(event.into());
        self.audio_q.clear();
        self.clear_video();
        if !flush {
            self.finish();
            return;
        }
        if let (Some((reason, detail)), Some(_)) = (close, &self.transport) {
            self.queue_record(&Record::Control(Control::Close { reason, detail }), true);
        }
        self.life = Life::Closing;
        self.closing_deadline = Some(self.now + CONTROL_STALL_MS);
        self.check_done();
    }

    /// Fails the session; with `flush`, pending output is still sent.
    pub(crate) fn fail(&mut self, error: SessionError, flush: bool) {
        self.terminate(CommonEvent::Failed(error), None, flush);
    }

    /// Ends the session with a `protocol-error` close (protocol.md §6, §7.12).
    pub(crate) fn protocol_error(&mut self, error: ProtocolError) {
        let detail = error.to_string();
        self.terminate(
            CommonEvent::Failed(SessionError::Protocol(error)),
            Some((CloseReason::ProtocolError, Some(detail))),
            true,
        );
    }

    /// A local, orderly close (protocol.md §7.12).
    pub(crate) fn close(&mut self, reason: CloseReason, detail: Option<String>) {
        self.terminate(
            CommonEvent::Closed {
                reason: reason.clone(),
                detail: detail.clone(),
                by_peer: false,
            },
            Some((reason, detail)),
            true,
        );
    }

    /// The peer sent `close`: stop without sending anything.
    pub(crate) fn peer_closed(&mut self, reason: CloseReason, detail: Option<String>) {
        self.terminate(
            CommonEvent::Closed {
                reason,
                detail,
                by_peer: true,
            },
            None,
            false,
        );
    }

    fn nonce_close(&mut self) {
        self.close(CloseReason::Other(NONCE_LIMIT_REASON.to_owned()), None);
    }

    pub(crate) fn connection_lost(&mut self, now: u64) {
        self.advance(now);
        match self.life {
            Life::Open => self.fail(SessionError::ConnectionLost, false),
            Life::Closing => self.finish(),
            Life::Done => {}
        }
    }

    // ----- timers ----------------------------------------------------------

    fn advance(&mut self, now: u64) {
        self.now = self.now.max(now);
    }

    fn ping_due(&self) -> Option<u64> {
        (self.keepalive && self.nothing_pending()).then_some(self.last_tx + PING_INTERVAL_MS)
    }

    pub(crate) fn poll_timeout(&self) -> Option<u64> {
        match self.life {
            Life::Done => None,
            Life::Closing => self.closing_deadline,
            Life::Open => [
                self.preface_deadline,
                self.setup_deadline,
                Some(self.last_rx + IDLE_TIMEOUT_MS),
                self.control_progress.map(|t| t + CONTROL_STALL_MS),
                self.ping_due(),
            ]
            .into_iter()
            .flatten()
            .min(),
        }
    }

    pub(crate) fn handle_timeout(&mut self, now: u64) {
        self.advance(now);
        let now = self.now;
        let due = |d: Option<u64>| d.is_some_and(|d| d <= now);
        match self.life {
            Life::Done => {}
            Life::Closing => {
                if due(self.closing_deadline) {
                    self.finish();
                }
            }
            Life::Open => {
                if due(self.preface_deadline) {
                    self.fail(SessionError::Timeout(TimeoutKind::Preface), false);
                } else if due(self.setup_deadline) {
                    self.fail(SessionError::Timeout(TimeoutKind::Setup), false);
                } else if due(self.control_progress.map(|t| t + CONTROL_STALL_MS)) {
                    self.fail(SessionError::ControlStalled, false);
                } else if due(Some(self.last_rx + IDLE_TIMEOUT_MS)) {
                    self.terminate(
                        CommonEvent::Failed(SessionError::Timeout(TimeoutKind::Idle)),
                        Some((CloseReason::Idle, None)),
                        true,
                    );
                } else if due(self.ping_due()) {
                    self.queue_record(&Record::Ping(now), false);
                    self.ping_outstanding = Some(now);
                }
            }
        }
    }

    /// Called when the station preface arrived; no more preface deadline.
    pub(crate) fn preface_received(&mut self) {
        self.preface_deadline = None;
    }
}

/// A role state machine built on [`Core`].
pub(crate) trait Machine {
    type Event: From<CommonEvent>;
    fn core(&mut self) -> &mut Core<Self::Event>;
    /// Processes one framed input while the session is open.
    fn on_input(&mut self, input: Input);
}

/// Feeds TCP stream bytes, in any chunking (protocol.md §3.1, §3.2).
pub(crate) fn feed_tcp<M: Machine>(m: &mut M, now: u64, bytes: &[u8]) {
    if !m.core().begin_input(now, Flavor::Tcp, !bytes.is_empty()) {
        return;
    }
    m.core().tcp_push(bytes);
    while m.core().is_open() {
        match m.core().tcp_next() {
            Ok(Some(input)) => {
                m.on_input(input);
                m.core().after_input();
            }
            Ok(None) => break,
            Err(e) => m.core().protocol_error(e),
        }
    }
}

/// Feeds one WebSocket message (protocol.md §3.3).
pub(crate) fn feed_ws<M: Machine>(m: &mut M, now: u64, kind: WsKind, payload: &[u8]) {
    if !m.core().begin_input(now, Flavor::WebSocket, true) {
        return;
    }
    match m.core().ws_input(kind, payload) {
        Ok(input) => {
            m.on_input(input);
            m.core().after_input();
        }
        Err(e) => m.core().protocol_error(e),
    }
}

/// Checks a local `hello` before a session starts (protocol.md §7.2).
pub(crate) fn check_local_hello(
    hello: &nestwatch_proto::control::Hello,
    role: nestwatch_proto::control::Role,
) -> Result<(), crate::ConfigError> {
    if hello.role != role {
        return Err(crate::ConfigError::WrongRole);
    }
    if !hello
        .audio
        .contains(&nestwatch_proto::record::AUDIO_CODEC_ADPCM)
    {
        return Err(crate::ConfigError::MissingMandatoryCodec);
    }
    Control::Hello(hello.clone()).validate()?;
    Ok(())
}

/// Whether a station may send this message (protocol.md §7).
pub(crate) fn station_to_listener(c: &Control) -> bool {
    matches!(
        c,
        Control::AudioConfig { .. }
            | Control::Level { .. }
            | Control::Activity { .. }
            | Control::Status(_)
            | Control::VideoState(_)
            | Control::TalkState { .. }
            | Control::Paired { .. }
            | Control::Unknown { .. }
    )
}

/// Whether a listener may send this message (protocol.md §7).
pub(crate) fn listener_to_station(c: &Control) -> bool {
    matches!(
        c,
        Control::Listen(_)
            | Control::VideoStart(_)
            | Control::VideoStop
            | Control::VideoOptions { .. }
            | Control::TalkRequest
            | Control::TalkRelease
            | Control::Unpair
            | Control::Unknown { .. }
    )
}

/// Short description of a record for diagnostics.
pub(crate) fn describe(record: &Record) -> String {
    match record {
        Record::Control(c) => c.message_type().to_owned(),
        other => format!("record {:#04x}", other.record_type()),
    }
}
