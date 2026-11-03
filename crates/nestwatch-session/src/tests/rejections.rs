// SPDX-License-Identifier: AGPL-3.0-or-later
//! Preface statuses, handshake rejections, deadlines and the pairing trust
//! rule (protocol.md §4, §5).

use nestwatch_crypto::{HandshakeError, Initiator, Prologue, TransportError};
use nestwatch_proto::preface::{ListenerPreface, Mode, PrefaceError, StationPreface, Status};

use super::harness::*;
use crate::{Flavor, ListenerEvent, ListenerMode, SessionError, StationConfig, TimeoutKind};

fn refused(flavor: Flavor, mode: ListenerMode, policy: TestPolicy, busy: bool, status: Status) {
    let mut config = StationConfig::new(flavor, station_hello());
    config.busy = busy;
    let s = station_with(config, policy, 0);
    let mut net = Net::with(flavor, listener(flavor, mode, 0), s);
    net.run_for(100);
    assert_eq!(net.l_failed(), Some(&SessionError::Refused(status)));
    assert_eq!(net.s_failed(), Some(&SessionError::Refused(status)));
    // Only the 6-byte answer went out, then both sides are done.
    assert_eq!(net.s_sent, StationPreface::new(status).encode());
    assert!(net.l.is_finished() && net.s.is_finished());
    assert!(net.s.policy().paired.is_empty());
}

#[test]
fn pairing_closed_for_invite_and_code_section_4_fr_pair_3() {
    for flavor in [Flavor::Tcp, Flavor::WebSocket] {
        for mode in [invite_mode(), code_mode()] {
            refused(
                flavor,
                mode,
                TestPolicy::trusting(),
                false,
                Status::PairingClosed,
            );
        }
    }
    // Trusted reconnects are unaffected by a closed pairing window.
    let mut net = Net::new(Flavor::Tcp, trusted_mode(), TestPolicy::trusting());
    net.connect();
}

#[test]
fn busy_answers_busy_in_every_mode_section_9() {
    for mode in [trusted_mode(), invite_mode(), code_mode()] {
        refused(Flavor::Tcp, mode, TestPolicy::pairing(), true, Status::Busy);
    }
}

#[test]
fn rate_limited_section_4() {
    let mut policy = TestPolicy::pairing();
    policy.rate_limited = true;
    refused(
        Flavor::WebSocket,
        code_mode(),
        policy,
        false,
        Status::RateLimited,
    );
}

#[test]
fn unsupported_version_and_unknown_mode_section_4_proto_1() {
    for preface in [*b"NEST\x02\x01", *b"NEST\x01\x09"] {
        let mut s = station(Flavor::Tcp, TestPolicy::pairing(), 0);
        s.handle_bytes(10, &preface);
        assert_eq!(drain_s(&mut s, 10), b"NEST\x01\x01");
        assert_eq!(
            events_s(&mut s),
            vec![crate::StationEvent::Failed(SessionError::Refused(
                Status::UnsupportedVersion
            ))]
        );
        assert!(s.is_finished());
    }
    // The listener side of the same answer.
    let mut l = listener(Flavor::Tcp, trusted_mode(), 0);
    drain_l(&mut l, 0);
    l.handle_bytes(5, b"NEST\x01\x01");
    assert_eq!(
        events_l(&mut l),
        vec![ListenerEvent::Failed(SessionError::Refused(
            Status::UnsupportedVersion
        ))]
    );
    // A station speaking another version.
    let mut l = listener(Flavor::Tcp, trusted_mode(), 0);
    l.handle_bytes(5, b"NEST\x02\x00");
    assert_eq!(
        events_l(&mut l),
        vec![ListenerEvent::Failed(SessionError::Preface(
            PrefaceError::UnsupportedVersion(2)
        ))]
    );
}

#[test]
fn non_nestwatch_preface_gets_no_answer() {
    let mut s = station(Flavor::Tcp, TestPolicy::pairing(), 0);
    s.handle_bytes(0, b"GET / HTTP/1.1\r\n");
    assert!(drain_s(&mut s, 0).is_empty());
    assert_eq!(
        events_s(&mut s),
        vec![crate::StationEvent::Failed(SessionError::Preface(
            PrefaceError::BadMagic
        ))]
    );
    // WebSocket: the preface must be one 6-byte message (§3.3).
    let mut s = station(Flavor::WebSocket, TestPolicy::pairing(), 0);
    s.handle_ws_message(0, nestwatch_proto::frame::WsKind::Binary, b"NEST\x01\x01\x00");
    assert!(drain_s(&mut s, 0).is_empty());
    assert!(matches!(
        events_s(&mut s).as_slice(),
        [crate::StationEvent::Failed(SessionError::Protocol(
            crate::ProtocolError::PrefaceLength
        ))]
    ));
}

#[test]
fn preface_deadline_5_s_section_4() {
    let mut s = station(Flavor::Tcp, TestPolicy::pairing(), 1_000);
    assert_eq!(s.poll_timeout(), Some(6_000));
    s.handle_timeout(5_999);
    assert!(events_s(&mut s).is_empty());
    s.handle_timeout(6_000);
    assert_eq!(
        events_s(&mut s),
        vec![crate::StationEvent::Failed(SessionError::Timeout(
            TimeoutKind::Preface
        ))]
    );
    assert!(drain_s(&mut s, 6_000).is_empty());
}

#[test]
fn handshake_deadline_15_s_section_4() {
    // Preface arrives, then the listener goes quiet but keeps the
    // connection alive with stray bytes of an unfinished frame.
    let mut s = station(Flavor::Tcp, TestPolicy::pairing(), 0);
    s.handle_bytes(1_000, &ListenerPreface::new(Mode::Code).encode());
    s.handle_bytes(1_000, &[0xff, 0xff]);
    for t in (2_000..15_000).step_by(2_000) {
        s.handle_bytes(t, &[0]);
        s.handle_timeout(t);
    }
    assert!(events_s(&mut s).is_empty());
    assert_eq!(s.poll_timeout(), Some(15_000));
    s.handle_timeout(15_000);
    assert_eq!(
        events_s(&mut s),
        vec![crate::StationEvent::Failed(SessionError::Timeout(
            TimeoutKind::Setup
        ))]
    );
}

#[test]
fn unknown_trusted_listener_rejected_silently_section_5_1() {
    let mut net = Net::new(Flavor::Tcp, trusted_mode(), TestPolicy::closed());
    net.run_for(100);
    assert_eq!(net.s_failed(), Some(&SessionError::Untrusted));
    assert!(net.s.is_finished());
    // Nothing distinguishes the rejection: the station sent only its `ok`
    // preface, exactly as for a garbage first handshake message.
    assert_eq!(net.s_sent, StationPreface::new(Status::Ok).encode());

    let mut s = station(Flavor::Tcp, TestPolicy::trusting(), 0);
    s.handle_bytes(0, &ListenerPreface::new(Mode::Trusted).encode());
    let answer = drain_s(&mut s, 0);
    s.handle_bytes(0, &frame(&[0x42; 96]));
    assert_eq!(answer, net.s_sent);
    assert!(drain_s(&mut s, 0).is_empty());
    assert!(s.is_finished());

    // The listener just sees the connection go away.
    net.disconnect();
    assert_eq!(net.l_failed(), Some(&SessionError::ConnectionLost));
}

#[test]
fn wrong_invite_token_section_5_2() {
    let mode = ListenerMode::Invite {
        station: *station_key().public_key(),
        token: nestwatch_crypto::InviteToken::from_bytes(&[0x01; 16]),
    };
    let mut net = Net::new(Flavor::Tcp, mode, TestPolicy::pairing());
    net.run_for(100);
    assert_eq!(
        net.l_failed(),
        Some(&SessionError::Handshake(HandshakeError::Rejected))
    );
    assert!(net.l.is_finished());
    // The station completed IKpsk2 but never trusts the listener.
    net.disconnect();
    assert_eq!(net.s_failed(), Some(&SessionError::ConnectionLost));
    assert!(net.s.policy().paired.is_empty());
    assert_eq!(net.s.policy().code_failures, 0);
}

#[test]
fn wrong_code_counts_a_failed_attempt_section_5_3() {
    let mode = ListenerMode::Code {
        station_id: station_key().device_id(),
        psk: wrong_psk(),
    };
    let mut net = Net::new(Flavor::WebSocket, mode, TestPolicy::pairing());
    net.run_for(100);
    assert_eq!(
        net.s_failed(),
        Some(&SessionError::Handshake(HandshakeError::Rejected))
    );
    assert_eq!(net.s.policy().code_failures, 1);
    assert!(net.s.policy().paired.is_empty());
    // The listener finished XXpsk3 locally and waits for a hello that never
    // comes; the station closed silently.
    assert!(!net.l.is_connected());
    net.disconnect();
    assert_eq!(net.l_failed(), Some(&SessionError::ConnectionLost));
}

#[test]
fn code_mode_wrong_station_id_aborts_before_message_3_crypto_1() {
    let mode = ListenerMode::Code {
        station_id: other_key().device_id(),
        psk: code_psk(),
    };
    let mut net = Net::new(Flavor::Tcp, mode, TestPolicy::pairing());
    net.run_for(100);
    assert_eq!(
        net.l_failed(),
        Some(&SessionError::Handshake(HandshakeError::DeviceIdMismatch))
    );
    net.disconnect();
    // The station never got a PSK-bound message to count.
    assert_eq!(net.s.policy().code_failures, 0);
}

#[test]
fn trusted_mode_to_wrong_station_fails_handshake() {
    let mode = ListenerMode::Trusted {
        station: *other_key().public_key(),
    };
    let mut net = Net::new(Flavor::Tcp, mode, TestPolicy::trusting());
    net.run_for(100);
    // IK: the station cannot decrypt message 1 made for another key.
    assert_eq!(
        net.s_failed(),
        Some(&SessionError::Handshake(HandshakeError::Rejected))
    );
}

/// §5.2 (as amended): in IKpsk2 the station's handshake completes before
/// the listener proved the PSK. A listener without the PSK completes
/// message 1, gets message 2 and then sends a transport message it could
/// not have encrypted: the station must not trust it.
#[test]
fn invite_listener_without_psk_is_never_trusted_section_5_2() {
    let mut s = station(Flavor::Tcp, TestPolicy::pairing(), 0);
    let lp = ListenerPreface::new(Mode::Invite).encode();
    s.handle_bytes(0, &lp);
    let sp: [u8; 6] = drain_s(&mut s, 0).try_into().unwrap();
    assert_eq!(sp, StationPreface::new(Status::Ok).encode());
    let prologue = Prologue::new(lp, sp);
    let mut attacker = Initiator::invite(
        &other_key(),
        station_key().public_key(),
        &wrong_psk(),
        &prologue,
    )
    .unwrap();
    s.handle_bytes(0, &frame(&attacker.start().unwrap()));
    let out = frames(&drain_s(&mut s, 0));
    // Message 2 and the station's hello.
    assert_eq!(out.len(), 2);
    assert_eq!(
        attacker.read_message(&out[0]).unwrap_err(),
        HandshakeError::Rejected
    );
    // A forged "hello": any bytes the attacker can produce.
    s.handle_bytes(1, &frame(&[0xAA; 64]));
    assert_eq!(
        events_s(&mut s),
        vec![crate::StationEvent::Failed(SessionError::Transport(
            TransportError::Decrypt
        ))]
    );
    assert!(s.policy().paired.is_empty());
    assert!(!s.policy().trusted.contains(other_key().public_key()));
}

/// §5.2: the honest listener becomes trusted only when its `hello`
/// decrypts, not when the handshake completes; a tampered hello never
/// makes it trusted.
#[test]
fn pairing_trust_only_after_first_transport_message_section_5_2() {
    for (mode, tamper) in [
        (invite_mode(), false),
        (code_mode(), false),
        (invite_mode(), true),
        (code_mode(), true),
    ] {
        let mut l = listener(Flavor::Tcp, mode, 0);
        let mut s = station(Flavor::Tcp, TestPolicy::pairing(), 0);
        // Exchange everything except the listener's hello.
        let mut hello_frame = None;
        for _ in 0..4 {
            let from_l = drain_l(&mut l, 0);
            if l.verification_number().is_some() {
                // The handshake is done on the listener side; what it sends
                // now ends with its hello (code mode: message 3 first).
                let mut f = frames(&from_l);
                let hello = f.pop().unwrap();
                for m in f {
                    s.handle_bytes(0, &frame(&m));
                }
                hello_frame = Some(hello);
                break;
            }
            s.handle_bytes(0, &from_l);
            l.handle_bytes(0, &drain_s(&mut s, 0));
        }
        let mut hello = hello_frame.expect("listener finished its handshake");
        assert!(s.verification_number().is_some(), "station handshake done");
        assert!(s.policy().paired.is_empty(), "not trusted before hello");
        assert!(!s.is_connected());
        if tamper {
            hello[20] ^= 1;
        }
        s.handle_bytes(1, &frame(&hello));
        let events = events_s(&mut s);
        if tamper {
            assert!(s.policy().paired.is_empty());
            assert_eq!(
                events,
                vec![crate::StationEvent::Failed(SessionError::Transport(
                    TransportError::Decrypt
                ))]
            );
        } else {
            assert_eq!(s.policy().paired.len(), 1);
            assert!(matches!(
                events.as_slice(),
                [crate::StationEvent::Connected { paired: true, .. }]
            ));
        }
    }
}

#[test]
fn connection_lost_during_handshake() {
    let mut net = Net::new(Flavor::Tcp, invite_mode(), TestPolicy::pairing());
    net.pump();
    net.disconnect();
    assert_eq!(net.l_failed(), Some(&SessionError::ConnectionLost));
    assert_eq!(net.s_failed(), Some(&SessionError::ConnectionLost));
    assert!(net.l.is_finished() && net.s.is_finished());
}
