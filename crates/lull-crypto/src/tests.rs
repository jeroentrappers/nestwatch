// SPDX-License-Identifier: AGPL-3.0-or-later
//! Handshake and transport tests across initiator and responder.

use std::sync::OnceLock;

use crate::*;

const LISTENER_PREFACE: [u8; 6] = *b"LULL\x01\x01";
const STATION_PREFACE: [u8; 6] = *b"LULL\x01\x00";

fn prologue() -> Prologue {
    Prologue::new(LISTENER_PREFACE, STATION_PREFACE)
}

struct Keys {
    listener: DeviceKey,
    station: DeviceKey,
}

fn keys() -> Keys {
    Keys {
        listener: DeviceKey::from_secret_bytes(&[0x11; 32]),
        station: DeviceKey::from_secret_bytes(&[0x22; 32]),
    }
}

fn code() -> PairingCode {
    PairingCode::parse("7K3QD-9XW2M").unwrap()
}

fn code_psk() -> &'static Psk {
    static PSK: OnceLock<Psk> = OnceLock::new();
    PSK.get_or_init(|| code().psk(&keys().station.device_id()))
}

fn invite_psk(k: &Keys) -> Psk {
    InviteToken::from_bytes(&[0x33; 16]).psk(k.station.public_key())
}

fn pair(mode: Mode, k: &Keys) -> (Initiator, Responder) {
    let p = prologue();
    match mode {
        Mode::Trusted => (
            Initiator::trusted(&k.listener, k.station.public_key(), &p).unwrap(),
            Responder::trusted(&k.station, &p).unwrap(),
        ),
        Mode::Invite => {
            let psk = invite_psk(k);
            (
                Initiator::invite(&k.listener, k.station.public_key(), &psk, &p).unwrap(),
                Responder::invite(&k.station, &psk, &p).unwrap(),
            )
        }
        Mode::Code => (
            Initiator::code(&k.listener, &k.station.device_id(), code_psk(), &p).unwrap(),
            Responder::code(&k.station, code_psk(), &p).unwrap(),
        ),
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Failure {
    Initiator(HandshakeError),
    Responder(HandshakeError),
    Rejected,
}

/// Runs a handshake, applying `tamper` to the `n`th message (0-based) and
/// deciding trusted-mode membership with `trusted`.
fn run(
    initiator: &mut Initiator,
    responder: &mut Responder,
    trusted: impl Fn(&PublicKey) -> bool,
    mut tamper: impl FnMut(usize, &mut Vec<u8>),
) -> Result<(Transport, Transport), Failure> {
    let mut index = 0;
    let mut to_responder = initiator.start().map_err(Failure::Initiator)?;
    loop {
        tamper(index, &mut to_responder);
        index += 1;
        let mut step = responder
            .read_message(&to_responder)
            .map_err(Failure::Responder)?;
        if let Step::Authorize(key) = &step {
            if !trusted(key) {
                return Err(Failure::Rejected);
            }
            step = responder.accept().map_err(Failure::Responder)?;
        }
        let mut to_initiator = match step {
            Step::Send(m) => m,
            Step::Complete(c) => {
                let mut m = c.message.expect("responder completes by sending");
                tamper(index, &mut m);
                return match initiator.read_message(&m).map_err(Failure::Initiator)? {
                    Step::Complete(i) => {
                        assert!(i.message.is_none());
                        Ok((i.transport, c.transport))
                    }
                    other => panic!("unexpected {other:?}"),
                };
            }
            Step::Authorize(_) => panic!("authorize twice"),
        };
        tamper(index, &mut to_initiator);
        index += 1;
        match initiator
            .read_message(&to_initiator)
            .map_err(Failure::Initiator)?
        {
            Step::Send(m) => to_responder = m,
            Step::Complete(i) => {
                let mut m = i.message.expect("initiator completes by sending");
                tamper(index, &mut m);
                return match responder.read_message(&m).map_err(Failure::Responder)? {
                    Step::Complete(c) => {
                        assert!(c.message.is_none());
                        Ok((i.transport, c.transport))
                    }
                    other => panic!("unexpected {other:?}"),
                };
            }
            Step::Authorize(_) => panic!("initiator never authorizes"),
        }
    }
}

fn handshake(mode: Mode) -> (Transport, Transport) {
    let k = keys();
    let (mut i, mut r) = pair(mode, &k);
    run(&mut i, &mut r, |_| true, |_, _| {}).unwrap()
}

const MODES: [Mode; 3] = [Mode::Trusted, Mode::Invite, Mode::Code];

fn message_count(mode: Mode) -> usize {
    if mode == Mode::Code { 3 } else { 2 }
}

#[test]
fn all_modes_complete_and_agree() {
    let k = keys();
    for mode in MODES {
        let (listener, station) = handshake(mode);
        assert_eq!(listener.mode(), mode);
        assert_eq!(listener.remote_static(), k.station.public_key());
        assert_eq!(station.remote_static(), k.listener.public_key());
        assert_eq!(listener.handshake_hash(), station.handshake_hash());
        assert_eq!(
            listener.verification_number(),
            station.verification_number()
        );
        assert_eq!(listener.verification_number().is_some(), mode.is_pairing());
    }
}

#[test]
fn handshakes_use_fresh_ephemerals() {
    for mode in MODES {
        let (a, _) = handshake(mode);
        let (b, _) = handshake(mode);
        assert_ne!(a.handshake_hash(), b.handshake_hash(), "{mode:?}");
    }
}

#[test]
fn trusted_responder_sees_listener_key_and_rejects_silently() {
    let k = keys();
    let (mut i, mut r) = pair(Mode::Trusted, &k);
    let m1 = i.start().unwrap();
    match r.read_message(&m1).unwrap() {
        Step::Authorize(key) => assert_eq!(&key, k.listener.public_key()),
        other => panic!("unexpected {other:?}"),
    }
    // Rejection produces nothing to send; the caller just closes.
    r.reject();

    let (mut i, mut r) = pair(Mode::Trusted, &k);
    let unknown = DeviceKey::from_secret_bytes(&[0x44; 32]);
    let mut i2 = Initiator::trusted(&unknown, k.station.public_key(), &prologue()).unwrap();
    let mut r2 = Responder::trusted(&k.station, &prologue()).unwrap();
    let allow = |key: &PublicKey| key == k.listener.public_key();
    assert!(run(&mut i, &mut r, allow, |_, _| {}).is_ok());
    assert_eq!(
        run(&mut i2, &mut r2, allow, |_, _| {}).unwrap_err(),
        Failure::Rejected
    );
}

#[test]
fn accept_only_after_authorize() {
    let k = keys();
    let (_, mut r) = pair(Mode::Trusted, &k);
    assert_eq!(r.accept().unwrap_err(), HandshakeError::InvalidState);
    let (mut i, mut r) = pair(Mode::Invite, &k);
    let m1 = i.start().unwrap();
    assert!(matches!(r.read_message(&m1).unwrap(), Step::Complete(_)));
    assert_eq!(r.accept().unwrap_err(), HandshakeError::InvalidState);
}

#[test]
fn trusted_with_wrong_station_key_fails() {
    let k = keys();
    let other = DeviceKey::from_secret_bytes(&[0x55; 32]);
    let mut i = Initiator::trusted(&k.listener, other.public_key(), &prologue()).unwrap();
    let mut r = Responder::trusted(&k.station, &prologue()).unwrap();
    assert_eq!(
        run(&mut i, &mut r, |_| true, |_, _| {}).unwrap_err(),
        Failure::Responder(HandshakeError::Rejected)
    );
}

#[test]
fn invite_with_wrong_psk_fails_at_listener() {
    let k = keys();
    let p = prologue();
    let wrong = InviteToken::from_bytes(&[0x34; 16]).psk(k.station.public_key());
    let mut i = Initiator::invite(&k.listener, k.station.public_key(), &wrong, &p).unwrap();
    let mut r = Responder::invite(&k.station, &invite_psk(&k), &p).unwrap();
    assert_eq!(
        run(&mut i, &mut r, |_| true, |_, _| {}).unwrap_err(),
        Failure::Initiator(HandshakeError::Rejected)
    );
}

#[test]
fn invite_station_only_trusts_after_first_transport_message() {
    // With a wrong PSK the station completes IKpsk2 but cannot decrypt
    // anything the listener sends, so it never adds the listener.
    let k = keys();
    let p = prologue();
    let wrong = InviteToken::from_bytes(&[0x34; 16]).psk(k.station.public_key());
    let mut r = Responder::invite(&k.station, &invite_psk(&k), &p).unwrap();
    let mut raw = raw_builder(Mode::Invite, &k.listener, &p)
        .remote_public_key(k.station.public_key().as_bytes())
        .unwrap()
        .psk(2, wrong.as_bytes())
        .unwrap()
        .build_initiator()
        .unwrap();
    let mut buf = [0u8; 1024];
    let n = raw.write_message(&[], &mut buf).unwrap();
    let Step::Complete(c) = r.read_message(&buf[..n]).unwrap() else {
        panic!("station completes after one message");
    };
    let mut station = c.transport;
    // The attacker cannot finish the handshake, so its best guess is noise.
    assert_eq!(
        station.decrypt(&[0u8; 32]).unwrap_err(),
        TransportError::Decrypt
    );
}

#[test]
fn code_with_wrong_code_fails_at_station() {
    let k = keys();
    let p = prologue();
    let wrong = PairingCode::parse("7K3QD-9XW2N")
        .unwrap()
        .psk(&k.station.device_id());
    let mut i = Initiator::code(&k.listener, &k.station.device_id(), &wrong, &p).unwrap();
    let mut r = Responder::code(&k.station, code_psk(), &p).unwrap();
    assert_eq!(
        run(&mut i, &mut r, |_| true, |_, _| {}).unwrap_err(),
        Failure::Responder(HandshakeError::Rejected)
    );
}

#[test]
fn code_with_wrong_device_id_fails_before_third_message() {
    let k = keys();
    let p = prologue();
    let fake = DeviceKey::from_secret_bytes(&[0x66; 32]);
    // The listener expects the real station's ID but talks to `fake`, which
    // even knows the code.
    let mut i = Initiator::code(&k.listener, &k.station.device_id(), code_psk(), &p).unwrap();
    let mut r = Responder::code(&fake, code_psk(), &p).unwrap();
    let m1 = i.start().unwrap();
    let Step::Send(m2) = r.read_message(&m1).unwrap() else {
        panic!("expected second message");
    };
    assert_eq!(
        i.read_message(&m2).unwrap_err(),
        HandshakeError::DeviceIdMismatch
    );
    assert_eq!(
        i.read_message(&m2).unwrap_err(),
        HandshakeError::InvalidState
    );
}

#[test]
fn prologue_mismatch_fails_in_every_mode() {
    let k = keys();
    for mode in MODES {
        let (mut i, _) = pair(mode, &k);
        let p = Prologue::new(LISTENER_PREFACE, *b"LULL\x01\x03");
        let mut r = match mode {
            Mode::Trusted => Responder::trusted(&k.station, &p),
            Mode::Invite => Responder::invite(&k.station, &invite_psk(&k), &p),
            Mode::Code => Responder::code(&k.station, code_psk(), &p),
        }
        .unwrap();
        let err = run(&mut i, &mut r, |_| true, |_, _| {}).unwrap_err();
        // Every mode authenticates the first message's (empty) payload
        // under a key bound to the prologue: IK through `es`, XXpsk3
        // through the PSK modifier's `e` key mixing.
        let expected = Failure::Responder(HandshakeError::Rejected);
        assert_eq!(err, expected, "{mode:?}");
    }
}

#[test]
fn tampering_with_any_handshake_message_fails() {
    let k = keys();
    for mode in MODES {
        for target in 0..message_count(mode) {
            for position in [0usize, 31, usize::MAX] {
                let (mut i, mut r) = pair(mode, &k);
                let result = run(
                    &mut i,
                    &mut r,
                    |_| true,
                    |n, m| {
                        if n == target {
                            let at = position.min(m.len() - 1);
                            m[at] ^= 0x01;
                        }
                    },
                );
                assert!(result.is_err(), "{mode:?} message {target} byte {position}");
            }
            let (mut i, mut r) = pair(mode, &k);
            let truncated = run(
                &mut i,
                &mut r,
                |_| true,
                |n, m| {
                    if n == target {
                        m.pop();
                    }
                },
            );
            assert!(truncated.is_err(), "{mode:?} truncated message {target}");
        }
    }
}

/// A plain snow handshake, to play a peer that breaks LWP/1 rules.
fn raw_builder<'a>(mode: Mode, local: &'a DeviceKey, p: &'a Prologue) -> snow::Builder<'a> {
    snow::Builder::new(mode.protocol_name().parse().unwrap())
        .local_private_key(local.secret())
        .unwrap()
        .prologue(p.as_bytes())
        .unwrap()
}

#[test]
fn non_empty_handshake_payloads_are_rejected() {
    let k = keys();
    let p = prologue();
    let mut buf = [0u8; 1024];

    // A listener sending a payload in XXpsk3's first message.
    let mut raw = raw_builder(Mode::Code, &k.listener, &p)
        .psk(3, code_psk().as_bytes())
        .unwrap()
        .build_initiator()
        .unwrap();
    let n = raw.write_message(b"x", &mut buf).unwrap();
    let (_, mut r) = pair(Mode::Code, &k);
    assert_eq!(
        r.read_message(&buf[..n]).unwrap_err(),
        HandshakeError::NonEmptyPayload
    );

    // A peer writing an encrypted payload with valid keys.
    let mut raw = raw_builder(Mode::Trusted, &k.listener, &p)
        .remote_public_key(k.station.public_key().as_bytes())
        .unwrap()
        .build_initiator()
        .unwrap();
    let n = raw.write_message(b"hello", &mut buf).unwrap();
    let mut r = Responder::trusted(&k.station, &prologue()).unwrap();
    assert_eq!(
        r.read_message(&buf[..n]).unwrap_err(),
        HandshakeError::NonEmptyPayload
    );

    // And a station answering with a payload.
    let mut i = Initiator::trusted(&k.listener, k.station.public_key(), &prologue()).unwrap();
    let mut raw = raw_builder(Mode::Trusted, &k.station, &p)
        .build_responder()
        .unwrap();
    let m1 = i.start().unwrap();
    raw.read_message(&m1, &mut buf).unwrap();
    let n = raw.write_message(b"x", &mut buf).unwrap();
    assert_eq!(
        i.read_message(&buf[..n]).unwrap_err(),
        HandshakeError::NonEmptyPayload
    );
}

#[test]
fn step_order_is_enforced() {
    let k = keys();
    let (mut i, mut r) = pair(Mode::Code, &k);
    assert_eq!(
        i.read_message(&[0; 32]).unwrap_err(),
        HandshakeError::InvalidState
    );
    let (mut i, _) = pair(Mode::Code, &k);
    i.start().unwrap();
    assert_eq!(i.start().unwrap_err(), HandshakeError::InvalidState);
    // A failed handshake stays failed.
    assert_eq!(
        r.read_message(&[0; 3]).unwrap_err(),
        HandshakeError::Rejected
    );
    assert_eq!(
        r.read_message(&[0; 32]).unwrap_err(),
        HandshakeError::InvalidState
    );
}

#[test]
fn oversized_handshake_message_is_rejected() {
    let k = keys();
    let (_, mut r) = pair(Mode::Trusted, &k);
    assert_eq!(
        r.read_message(&vec![0; MAX_MESSAGE_LEN + 1]).unwrap_err(),
        HandshakeError::MessageTooLarge
    );
}

#[test]
fn transport_round_trips_both_directions() {
    for mode in MODES {
        let (mut listener, mut station) = handshake(mode);
        for n in 0..5u8 {
            let record = vec![n; usize::from(n) * 100];
            let c = listener.encrypt(&record).unwrap();
            assert_eq!(c.len(), record.len() + 16);
            assert_eq!(station.decrypt(&c).unwrap(), record);
            let c = station.encrypt(&record).unwrap();
            assert_eq!(listener.decrypt(&c).unwrap(), record);
        }
        assert_eq!(listener.sending_nonce(), 5);
        assert_eq!(station.receiving_nonce(), 5);
    }
}

#[test]
fn transport_size_limits() {
    let (mut listener, mut station) = handshake(Mode::Trusted);
    let max = listener.encrypt(&vec![7; MAX_PLAINTEXT_LEN]).unwrap();
    assert_eq!(max.len(), MAX_MESSAGE_LEN);
    assert_eq!(station.decrypt(&max).unwrap().len(), MAX_PLAINTEXT_LEN);
    assert_eq!(
        listener
            .encrypt(&vec![7; MAX_PLAINTEXT_LEN + 1])
            .unwrap_err(),
        TransportError::MessageTooLarge
    );
    // A too-large plaintext does not consume a nonce.
    assert_eq!(listener.sending_nonce(), 1);
    assert_eq!(
        station.decrypt(&vec![0; MAX_MESSAGE_LEN + 1]).unwrap_err(),
        TransportError::MessageTooLarge
    );
}

#[test]
fn transport_detects_tampering_and_then_stays_closed() {
    let (mut listener, mut station) = handshake(Mode::Invite);
    let mut c = listener.encrypt(b"\x01record").unwrap();
    c[3] ^= 0x80;
    assert_eq!(station.decrypt(&c).unwrap_err(), TransportError::Decrypt);
    c[3] ^= 0x80;
    assert_eq!(station.decrypt(&c).unwrap_err(), TransportError::Closed);
    assert_eq!(station.encrypt(b"x").unwrap_err(), TransportError::Closed);
}

#[test]
fn transport_detects_replay_reorder_and_truncation() {
    let (mut listener, mut station) = handshake(Mode::Trusted);
    let a = listener.encrypt(b"a").unwrap();
    station.decrypt(&a).unwrap();
    assert_eq!(station.decrypt(&a).unwrap_err(), TransportError::Decrypt);

    let (mut listener, mut station) = handshake(Mode::Trusted);
    let _a = listener.encrypt(b"a").unwrap();
    let b = listener.encrypt(b"b").unwrap();
    assert_eq!(station.decrypt(&b).unwrap_err(), TransportError::Decrypt);

    let (mut listener, mut station) = handshake(Mode::Trusted);
    let a = listener.encrypt(b"abc").unwrap();
    assert_eq!(
        station.decrypt(&a[..a.len() - 1]).unwrap_err(),
        TransportError::Decrypt
    );
    let (_, mut station) = handshake(Mode::Trusted);
    assert_eq!(station.decrypt(&[]).unwrap_err(), TransportError::Decrypt);
}

#[test]
fn transport_messages_are_bound_to_direction() {
    let (mut listener, _) = handshake(Mode::Trusted);
    let c = listener.encrypt(b"loop").unwrap();
    assert_eq!(listener.decrypt(&c).unwrap_err(), TransportError::Decrypt);
}

#[test]
fn nonce_limit_is_enforced_in_both_directions() {
    let (mut listener, mut station) = handshake(Mode::Code);
    listener.set_nonces_for_test(NONCE_LIMIT - 1, 0);
    station.set_nonces_for_test(0, NONCE_LIMIT - 1);
    // The last permitted nonce, 2^32 - 1, still works.
    let c = listener.encrypt(b"last").unwrap();
    assert_eq!(station.decrypt(&c).unwrap(), b"last");
    assert_eq!(listener.sending_nonce(), NONCE_LIMIT);
    assert_eq!(station.receiving_nonce(), NONCE_LIMIT);
    assert_eq!(
        listener.encrypt(b"more").unwrap_err(),
        TransportError::NonceExhausted
    );
    assert_eq!(
        station.decrypt(&c).unwrap_err(),
        TransportError::NonceExhausted
    );
    // The other directions are unaffected.
    let c = station.encrypt(b"reply").unwrap();
    assert_eq!(listener.decrypt(&c).unwrap(), b"reply");
}

#[test]
fn debug_output_contains_no_secrets() {
    let k = keys();
    let (i, r) = pair(Mode::Invite, &k);
    let (t, _) = handshake(Mode::Invite);
    let text = format!("{i:?} {r:?} {t:?} {:?}", invite_psk(&k));
    assert!(!text.contains("1111"), "{text}");
    let hash_hex: String = t
        .handshake_hash()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert!(!text.contains(&hash_hex[..16]));
}
