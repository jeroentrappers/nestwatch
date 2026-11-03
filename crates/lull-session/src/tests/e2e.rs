// SPDX-License-Identifier: AGPL-3.0-or-later
//! All three modes end to end over both transport flavours.

use lull_proto::control::{CloseReason, Control, StationStatus};
use lull_proto::preface::Mode;

use super::harness::*;
use crate::{Flavor, ListenerEvent, ListenerMode, StationConfig, StationEvent};

fn mode_of(m: &ListenerMode) -> Mode {
    m.wire_mode()
}

/// Every mode × flavour × chunking reaches `Connected` on both sides with
/// the §7.2 setup order and the right pairing outcome.
fn check_mode(flavor: Flavor, mode: ListenerMode, chunk: usize, deliver_chunk: usize) {
    let pairing = mode_of(&mode).is_pairing();
    let policy = if pairing {
        TestPolicy::pairing()
    } else {
        TestPolicy::trusting()
    };
    let mut net = Net::new(flavor, mode, policy);
    net.chunk = chunk;
    net.deliver_chunk = deliver_chunk;
    net.delay = 3;
    net.connect();

    let station_id = station_key().device_id();
    let mut expected = Vec::new();
    if pairing {
        expected.push(ListenerEvent::Paired {
            station_key: *station_key().public_key(),
            station_id,
        });
    }
    expected.push(ListenerEvent::Connected {
        station: station_hello(),
        audio_codec: 2,
        status: status(),
    });
    assert_eq!(net.l_events, expected, "{flavor:?} {chunk}/{deliver_chunk}");

    assert_eq!(
        net.s_events,
        vec![StationEvent::Connected {
            listener_key: *listener_key().public_key(),
            listener: listener_hello(),
            audio_codec: 2,
            paired: pairing,
        }]
    );
    let paired = &net.s.policy().paired;
    if pairing {
        assert_eq!(
            paired,
            &vec![(*listener_key().public_key(), "Parents' phone".to_owned())]
        );
    } else {
        assert!(paired.is_empty());
    }
    // FR-PAIR-5 / §5.5: both sides show the same number in pairing modes.
    assert_eq!(net.l.verification_number(), net.s.verification_number());
    assert_eq!(net.l.verification_number().is_some(), pairing);
    assert_eq!(net.l.station_key(), Some(station_key().public_key()));
    assert_eq!(net.s.listener_key(), Some(listener_key().public_key()));
    assert_eq!(net.s.mode(), Some(mode_of(net.l.mode())));
}

#[test]
fn modes_trusted_invite_code_end_to_end_tcp_and_websocket() {
    for flavor in [Flavor::Tcp, Flavor::WebSocket] {
        for mode in [trusted_mode(), invite_mode(), code_mode()] {
            check_mode(flavor, mode, 64 * 1024, usize::MAX);
        }
    }
}

#[test]
fn tcp_byte_by_byte_and_odd_chunks_section_3_1() {
    for mode in [trusted_mode(), invite_mode(), code_mode()] {
        // Sent one byte at a time.
        check_mode(Flavor::Tcp, mode.clone(), 1, usize::MAX);
        // Sent in big chunks, delivered one byte at a time.
        check_mode(Flavor::Tcp, mode.clone(), 1 << 20, 1);
        // Odd sizes that straddle the preface and length prefixes.
        check_mode(Flavor::Tcp, mode, 7, 5);
    }
}

#[test]
fn verification_number_differs_between_pairings_fr_pair_5() {
    let mut a = Net::new(Flavor::Tcp, invite_mode(), TestPolicy::pairing());
    a.connect();
    let mut b = Net::new(Flavor::Tcp, invite_mode(), TestPolicy::pairing());
    b.connect();
    // Fresh ephemeral keys give a fresh handshake hash.
    assert_ne!(a.l.verification_number(), b.l.verification_number());
}

#[test]
fn reconnect_after_pairing_uses_trusted_mode_fr_pair_4() {
    let mut net = Net::new(Flavor::Tcp, code_mode(), TestPolicy::pairing());
    net.connect();
    let station = match &net.l_events[0] {
        ListenerEvent::Paired { station_key, .. } => *station_key,
        other => panic!("expected Paired, got {other:?}"),
    };
    // The station's trust store now holds the listener (TestPolicy::paired).
    let mut policy = TestPolicy::closed();
    policy.trusted = net.s.policy().trusted.clone();
    let l = listener(Flavor::WebSocket, ListenerMode::Trusted { station }, 0);
    let s = station_with(
        StationConfig::new(Flavor::WebSocket, station_hello()),
        policy,
        0,
    );
    let mut again = Net::with(Flavor::WebSocket, l, s);
    again.connect();
    assert!(matches!(
        again.s_events.as_slice(),
        [StationEvent::Connected { paired: false, .. }]
    ));
}

#[test]
fn audio_config_picks_first_listener_codec_station_supports_section_7_3() {
    for (listener_codecs, station_codecs, expected) in [
        (vec![1, 2], vec![2], 2),
        (vec![2, 1], vec![1, 2], 2),
        (vec![1, 2], vec![2, 1], 1),
        (vec![9, 2], vec![1, 2], 2),
    ] {
        let mut lh = listener_hello();
        lh.audio = listener_codecs;
        let mut sh = station_hello();
        sh.audio = station_codecs;
        let l = crate::ListenerSession::new(
            crate::ListenerConfig {
                flavor: Flavor::Tcp,
                mode: trusted_mode(),
                hello: lh,
            },
            &listener_key(),
            0,
        )
        .unwrap();
        let s = station_with(
            StationConfig::new(Flavor::Tcp, sh),
            TestPolicy::trusting(),
            0,
        );
        let mut net = Net::with(Flavor::Tcp, l, s);
        net.connect();
        assert_eq!(net.l.audio_codec(), Some(expected));
        assert_eq!(net.s.audio_codec(), Some(expected));
    }
}

#[test]
fn control_messages_flow_both_ways_after_setup() {
    let mut net = Net::new(Flavor::WebSocket, trusted_mode(), TestPolicy::trusting());
    net.connect();
    net.l
        .send_control(Control::Listen(Default::default()))
        .unwrap();
    let st = StationStatus {
        listeners: 2,
        ..status()
    };
    net.s.send_control(Control::Status(st.clone())).unwrap();
    net.run_for(10);
    assert_eq!(
        net.s_events.last(),
        Some(&StationEvent::Control(Control::Listen(Default::default())))
    );
    assert_eq!(
        net.l_events.last(),
        Some(&ListenerEvent::Control(Control::Status(st)))
    );
}

#[test]
fn commands_rejected_before_setup_and_in_wrong_direction() {
    let mut l = listener(Flavor::Tcp, trusted_mode(), 0);
    assert_eq!(
        l.send_control(Control::TalkRequest),
        Err(crate::SendError::NotConnected)
    );
    let mut net = Net::new(Flavor::Tcp, trusted_mode(), TestPolicy::trusting());
    net.connect();
    assert!(matches!(
        net.l.send_control(Control::Status(status())),
        Err(crate::SendError::NotAllowed(_))
    ));
    assert!(matches!(
        net.l.send_control(Control::Close {
            reason: CloseReason::Idle,
            detail: None
        }),
        Err(crate::SendError::NotAllowed(_))
    ));
    assert!(matches!(
        net.s.send_control(Control::TalkRequest),
        Err(crate::SendError::NotAllowed(_))
    ));
    assert!(matches!(
        net.s.send_control(Control::AudioConfig { codec: 1 }),
        Err(crate::SendError::NotAllowed(_))
    ));
}

#[test]
fn local_hello_is_validated() {
    let mut h = listener_hello();
    h.audio = vec![1];
    let cfg = crate::ListenerConfig {
        flavor: Flavor::Tcp,
        mode: trusted_mode(),
        hello: h,
    };
    assert_eq!(
        crate::ListenerSession::new(cfg, &listener_key(), 0).unwrap_err(),
        crate::ConfigError::MissingMandatoryCodec
    );
    let cfg = StationConfig::new(Flavor::Tcp, listener_hello());
    assert_eq!(
        crate::StationSession::new(cfg, &station_key(), TestPolicy::closed(), 0).unwrap_err(),
        crate::ConfigError::WrongRole
    );
    let mut h = station_hello();
    h.name = "x".repeat(65);
    let cfg = StationConfig::new(Flavor::Tcp, h);
    assert!(matches!(
        crate::StationSession::new(cfg, &station_key(), TestPolicy::closed(), 0),
        Err(crate::ConfigError::InvalidHello(_))
    ));
}
