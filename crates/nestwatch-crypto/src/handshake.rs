// SPDX-License-Identifier: AGPL-3.0-or-later
//! Noise handshakes for the three NWP/1 modes (protocol.md §4, §5).
//!
//! The listener is the [`Initiator`], the station the [`Responder`]. Both
//! are step machines: feed each received handshake message to
//! `read_message`, send what the returned [`Step`] asks for, and switch to
//! the [`Transport`] on [`Step::Complete`]. Any error, or a rejected peer,
//! means: close the connection without sending anything further.

use snow::{Builder, HandshakeState};

use crate::{DeviceId, DeviceKey, HandshakeError, Psk, PublicKey, Transport};

/// Length of the prologue: `"nestwatch/1"` plus two 6-byte prefaces.
pub const PROLOGUE_LEN: usize = 23;

const PROLOGUE_TAG: &[u8; 11] = b"nestwatch/1";
/// Largest Noise message (Noise spec §3).
const MAX_NOISE_MESSAGE: usize = 65535;

/// The handshake mode selected by the preface (protocol.md §4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Mode {
    /// Reconnect of a paired listener: `IK`.
    Trusted,
    /// First pairing with an invitation: `IKpsk2`.
    Invite,
    /// First pairing with a pairing code: `XXpsk3`.
    Code,
}

impl Mode {
    /// The full Noise protocol name.
    pub const fn protocol_name(self) -> &'static str {
        match self {
            Mode::Trusted => "Noise_IK_25519_ChaChaPoly_BLAKE2s",
            Mode::Invite => "Noise_IKpsk2_25519_ChaChaPoly_BLAKE2s",
            Mode::Code => "Noise_XXpsk3_25519_ChaChaPoly_BLAKE2s",
        }
    }

    /// Whether this mode pairs devices (and so yields a verification number).
    pub const fn is_pairing(self) -> bool {
        !matches!(self, Mode::Trusted)
    }

    const fn psk_location(self) -> Option<u8> {
        match self {
            Mode::Trusted => None,
            Mode::Invite => Some(2),
            Mode::Code => Some(3),
        }
    }
}

/// `"nestwatch/1" || listener_preface || station_preface` (protocol.md §4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Prologue([u8; PROLOGUE_LEN]);

impl Prologue {
    /// Builds the prologue from the two prefaces exactly as sent on the wire.
    pub fn new(listener_preface: [u8; 6], station_preface: [u8; 6]) -> Self {
        let mut p = [0u8; PROLOGUE_LEN];
        let tag = PROLOGUE_TAG.len();
        p[..tag].copy_from_slice(PROLOGUE_TAG);
        p[tag..tag + 6].copy_from_slice(&listener_preface);
        p[tag + 6..].copy_from_slice(&station_preface);
        Self(p)
    }

    /// The 23 prologue bytes.
    pub const fn as_bytes(&self) -> &[u8; PROLOGUE_LEN] {
        &self.0
    }
}

/// What the caller must do next.
#[derive(Debug)]
#[must_use]
pub enum Step {
    /// Send this handshake message and feed the peer's reply to
    /// `read_message`.
    Send(Vec<u8>),
    /// Responder in `trusted` mode only: the listener presented this static
    /// key. Call [`Responder::accept`] if it is in the trusted listener list,
    /// otherwise [`Responder::reject`] and close silently.
    Authorize(PublicKey),
    /// The handshake is complete.
    Complete(Completion),
}

/// A finished handshake.
#[derive(Debug)]
#[must_use]
pub struct Completion {
    /// A last handshake message to send before any transport message.
    pub message: Option<Vec<u8>>,
    /// The transport state for records.
    pub transport: Transport,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// Initiator: the first message has not been written yet.
    Start,
    /// Waiting for the peer's next message.
    Reading,
    /// Responder in `trusted` mode: waiting for accept or reject.
    Authorizing,
    /// Finished or failed.
    Done,
}

struct Machine {
    state: Option<HandshakeState>,
    mode: Mode,
    phase: Phase,
}

impl core::fmt::Debug for Machine {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Handshake")
            .field("mode", &self.mode)
            .field("phase", &self.phase)
            .finish_non_exhaustive()
    }
}

impl Machine {
    fn new(
        initiator: bool,
        mode: Mode,
        local: &DeviceKey,
        remote: Option<&PublicKey>,
        psk: Option<&Psk>,
        prologue: &Prologue,
        fixed_ephemeral: Option<&[u8; 32]>,
    ) -> Result<Self, HandshakeError> {
        let params = mode
            .protocol_name()
            .parse()
            .map_err(|_| HandshakeError::InvalidState)?;
        let init = |_| HandshakeError::InvalidState;
        let mut builder = Builder::new(params)
            .local_private_key(local.secret())
            .map_err(init)?
            .prologue(prologue.as_bytes())
            .map_err(init)?;
        if let Some(remote) = remote {
            builder = builder.remote_public_key(remote.as_bytes()).map_err(init)?;
        }
        if let (Some(location), Some(psk)) = (mode.psk_location(), psk) {
            builder = builder.psk(location, psk.as_bytes()).map_err(init)?;
        }
        if let Some(e) = fixed_ephemeral {
            builder = builder.fixed_ephemeral_key_for_testing_only(e);
        }
        let state = if initiator {
            builder.build_initiator()
        } else {
            builder.build_responder()
        }
        .map_err(init)?;
        Ok(Self {
            state: Some(state),
            mode,
            phase: if initiator {
                Phase::Start
            } else {
                Phase::Reading
            },
        })
    }

    fn fail(&mut self, err: HandshakeError) -> HandshakeError {
        self.state = None;
        self.phase = Phase::Done;
        err
    }

    fn expect(&mut self, phase: Phase) -> Result<(), HandshakeError> {
        if self.phase == phase && self.state.is_some() {
            Ok(())
        } else {
            Err(self.fail(HandshakeError::InvalidState))
        }
    }

    fn state(&mut self) -> &mut HandshakeState {
        self.state.as_mut().expect("checked by expect()")
    }

    fn write(&mut self) -> Result<Vec<u8>, HandshakeError> {
        let mut buf = vec![0u8; MAX_NOISE_MESSAGE];
        match self.state().write_message(&[], &mut buf) {
            Ok(n) => {
                buf.truncate(n);
                Ok(buf)
            }
            Err(snow::Error::Rng) => Err(self.fail(HandshakeError::Rng)),
            Err(_) => Err(self.fail(HandshakeError::InvalidState)),
        }
    }

    fn read(&mut self, message: &[u8]) -> Result<(), HandshakeError> {
        if message.len() > MAX_NOISE_MESSAGE {
            return Err(self.fail(HandshakeError::MessageTooLarge));
        }
        let mut payload = vec![0u8; MAX_NOISE_MESSAGE];
        match self.state().read_message(message, &mut payload) {
            Ok(0) => Ok(()),
            Ok(_) => Err(self.fail(HandshakeError::NonEmptyPayload)),
            Err(_) => Err(self.fail(HandshakeError::Rejected)),
        }
    }

    fn remote_static(&mut self) -> Result<PublicKey, HandshakeError> {
        match self.state().get_remote_static().map(PublicKey::from_slice) {
            Some(Ok(key)) => Ok(key),
            _ => Err(self.fail(HandshakeError::InvalidState)),
        }
    }

    fn is_finished(&mut self) -> bool {
        self.state().is_handshake_finished()
    }

    fn finish(&mut self, message: Option<Vec<u8>>) -> Result<Step, HandshakeError> {
        let remote = self.remote_static()?;
        let state = self.state.take().expect("checked by expect()");
        self.phase = Phase::Done;
        let hash: [u8; 32] = state
            .get_handshake_hash()
            .try_into()
            .map_err(|_| HandshakeError::InvalidState)?;
        let transport = state
            .into_transport_mode()
            .map_err(|_| HandshakeError::InvalidState)?;
        Ok(Step::Complete(Completion {
            message,
            transport: Transport::new(transport, self.mode, remote, hash),
        }))
    }
}

/// The listener's side of a handshake (Noise initiator).
#[derive(Debug)]
pub struct Initiator {
    machine: Machine,
    expected_station: Option<DeviceId>,
}

impl Initiator {
    /// Mode `trusted` (`IK`): reconnect to a known station.
    pub fn trusted(
        local: &DeviceKey,
        station: &PublicKey,
        prologue: &Prologue,
    ) -> Result<Self, HandshakeError> {
        Self::build(
            Mode::Trusted,
            local,
            Some(station),
            None,
            None,
            prologue,
            None,
        )
    }

    /// Mode `invite` (`IKpsk2`): `psk` is [`crate::InviteToken::psk`] of the
    /// invitation's token and station key.
    pub fn invite(
        local: &DeviceKey,
        station: &PublicKey,
        psk: &Psk,
        prologue: &Prologue,
    ) -> Result<Self, HandshakeError> {
        Self::build(
            Mode::Invite,
            local,
            Some(station),
            Some(psk),
            None,
            prologue,
            None,
        )
    }

    /// Mode `code` (`XXpsk3`): `psk` is [`crate::PairingCode::psk`] for
    /// `station_id`, the ID from the station's discovery record. The
    /// station's static key must hash to `station_id`.
    pub fn code(
        local: &DeviceKey,
        station_id: &DeviceId,
        psk: &Psk,
        prologue: &Prologue,
    ) -> Result<Self, HandshakeError> {
        Self::build(
            Mode::Code,
            local,
            None,
            Some(psk),
            Some(*station_id),
            prologue,
            None,
        )
    }

    pub(crate) fn build(
        mode: Mode,
        local: &DeviceKey,
        station: Option<&PublicKey>,
        psk: Option<&Psk>,
        expected_station: Option<DeviceId>,
        prologue: &Prologue,
        fixed_ephemeral: Option<&[u8; 32]>,
    ) -> Result<Self, HandshakeError> {
        Ok(Self {
            machine: Machine::new(true, mode, local, station, psk, prologue, fixed_ephemeral)?,
            expected_station,
        })
    }

    /// The handshake mode.
    pub fn mode(&self) -> Mode {
        self.machine.mode
    }

    /// Writes the first handshake message. Call once, before `read_message`.
    pub fn start(&mut self) -> Result<Vec<u8>, HandshakeError> {
        let m = &mut self.machine;
        m.expect(Phase::Start)?;
        let message = m.write()?;
        m.phase = Phase::Reading;
        Ok(message)
    }

    /// Processes the station's handshake message.
    pub fn read_message(&mut self, message: &[u8]) -> Result<Step, HandshakeError> {
        let m = &mut self.machine;
        m.expect(Phase::Reading)?;
        m.read(message)?;
        if let Some(expected) = self.expected_station {
            // Checked as soon as the key is known, so a station with the
            // wrong key never receives the PSK-bound third message.
            let station = m.remote_static()?;
            if !expected.matches(&station) {
                return Err(m.fail(HandshakeError::DeviceIdMismatch));
            }
        }
        if m.is_finished() {
            return m.finish(None);
        }
        let reply = m.write()?;
        if m.is_finished() {
            m.finish(Some(reply))
        } else {
            Ok(Step::Send(reply))
        }
    }
}

/// The station's side of a handshake (Noise responder).
#[derive(Debug)]
pub struct Responder {
    machine: Machine,
}

impl Responder {
    /// Mode `trusted` (`IK`). `read_message` returns [`Step::Authorize`]
    /// with the listener's key before anything is sent back.
    pub fn trusted(local: &DeviceKey, prologue: &Prologue) -> Result<Self, HandshakeError> {
        Self::build(Mode::Trusted, local, None, prologue, None)
    }

    /// Mode `invite` (`IKpsk2`): `psk` is [`crate::InviteToken::psk`] of the
    /// open invitation's token and this station's public key.
    ///
    /// `IKpsk2` gives the station no proof that the listener knew the PSK
    /// until the first transport message from the listener decrypts; only
    /// then may the listener be added to the trusted list.
    pub fn invite(
        local: &DeviceKey,
        psk: &Psk,
        prologue: &Prologue,
    ) -> Result<Self, HandshakeError> {
        Self::build(Mode::Invite, local, Some(psk), prologue, None)
    }

    /// Mode `code` (`XXpsk3`): `psk` is [`crate::PairingCode::psk`] of the
    /// displayed code and this station's device ID. A
    /// [`HandshakeError::Rejected`] from the third message counts as a failed
    /// code attempt (protocol.md §5.3).
    pub fn code(local: &DeviceKey, psk: &Psk, prologue: &Prologue) -> Result<Self, HandshakeError> {
        Self::build(Mode::Code, local, Some(psk), prologue, None)
    }

    pub(crate) fn build(
        mode: Mode,
        local: &DeviceKey,
        psk: Option<&Psk>,
        prologue: &Prologue,
        fixed_ephemeral: Option<&[u8; 32]>,
    ) -> Result<Self, HandshakeError> {
        Ok(Self {
            machine: Machine::new(false, mode, local, None, psk, prologue, fixed_ephemeral)?,
        })
    }

    /// The handshake mode.
    pub fn mode(&self) -> Mode {
        self.machine.mode
    }

    /// Processes the listener's handshake message.
    pub fn read_message(&mut self, message: &[u8]) -> Result<Step, HandshakeError> {
        let m = &mut self.machine;
        m.expect(Phase::Reading)?;
        m.read(message)?;
        if m.is_finished() {
            return m.finish(None);
        }
        if m.mode == Mode::Trusted {
            let listener = m.remote_static()?;
            m.phase = Phase::Authorizing;
            return Ok(Step::Authorize(listener));
        }
        self.respond()
    }

    /// Trusted mode: the listener's key is trusted; continue the handshake.
    pub fn accept(&mut self) -> Result<Step, HandshakeError> {
        self.machine.expect(Phase::Authorizing)?;
        self.machine.phase = Phase::Reading;
        self.respond()
    }

    /// Trusted mode: the listener's key is unknown. Drops all handshake
    /// state; the caller closes the connection without sending anything.
    pub fn reject(self) {}

    fn respond(&mut self) -> Result<Step, HandshakeError> {
        let m = &mut self.machine;
        let reply = m.write()?;
        if m.is_finished() {
            m.finish(Some(reply))
        } else {
            Ok(Step::Send(reply))
        }
    }
}
