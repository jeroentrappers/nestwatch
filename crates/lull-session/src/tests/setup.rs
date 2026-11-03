// SPDX-License-Identifier: AGPL-3.0-or-later
//! Setup order of protocol.md §7.2, checked against hand-driven peers.

use lull_proto::control::{CloseReason, Control, Hello, Role};
use lull_proto::record::Record;

use super::harness::*;
use crate::{ListenerEvent, ProtocolError, SessionError, StationEvent};

fn ctl(c: Control) -> Record {
    Record::Control(c)
}

/// The listener fails with `err` and sends a `protocol-error` close.
fn assert_listener_protocol_error(
    l: &mut crate::ListenerSession,
    peer: &mut RawPeer,
    err: ProtocolError,
) {
    let events = events_l(l);
    assert_eq!(
        events.last(),
        Some(&ListenerEvent::Failed(SessionError::Protocol(err))),
        "{events:?}"
    );
    let sent = peer.open_all(&drain_l(l, 0));
    assert!(
        matches!(
            sent.as_slice(),
            [Record::Control(Control::Close {
                reason: CloseReason::ProtocolError,
                detail: Some(_)
            })]
        ),
        "{sent:?}"
    );
    assert!(l.is_finished());
}

#[test]
fn listener_requires_station_hello_first_section_7_2() {
    let (mut l, mut peer) = listener_vs_raw_station(false);
    l.handle_bytes(0, &peer.seal(&ctl(Control::AudioConfig { codec: 2 })));
    assert_listener_protocol_error(
        &mut l,
        &mut peer,
        ProtocolError::Unexpected {
            expected: "hello",
            got: "audio-config".into(),
        },
    );
}

#[test]
fn listener_rejects_ping_before_hello_section_7_2() {
    let (mut l, mut peer) = listener_vs_raw_station(false);
    l.handle_bytes(0, &peer.seal(&Record::Ping(1)));
    assert_listener_protocol_error(
        &mut l,
        &mut peer,
        ProtocolError::Unexpected {
            expected: "hello",
            got: "record 0x04".into(),
        },
    );
}

#[test]
fn listener_rejects_hello_with_wrong_role() {
    let (mut l, mut peer) = listener_vs_raw_station(false);
    let hello = Hello {
        role: Role::Listener,
        ..station_hello()
    };
    l.handle_bytes(0, &peer.seal(&ctl(Control::Hello(hello))));
    assert_listener_protocol_error(&mut l, &mut peer, ProtocolError::InvalidHello);
}

#[test]
fn pairing_requires_paired_before_audio_config_section_7_2() {
    let (mut l, mut peer) = listener_vs_raw_station(true);
    let mut bytes = peer.seal(&ctl(Control::Hello(station_hello())));
    bytes.extend(peer.seal(&ctl(Control::AudioConfig { codec: 2 })));
    l.handle_bytes(0, &bytes);
    assert_listener_protocol_error(
        &mut l,
        &mut peer,
        ProtocolError::Unexpected {
            expected: "paired",
            got: "audio-config".into(),
        },
    );
}

#[test]
fn trusted_mode_rejects_paired() {
    let (mut l, mut peer) = listener_vs_raw_station(false);
    let mut bytes = peer.seal(&ctl(Control::Hello(station_hello())));
    bytes.extend(peer.seal(&ctl(Control::Paired {
        station_id: station_proto_id(),
    })));
    l.handle_bytes(0, &bytes);
    assert_listener_protocol_error(
        &mut l,
        &mut peer,
        ProtocolError::Unexpected {
            expected: "audio-config",
            got: "paired".into(),
        },
    );
}

#[test]
fn status_before_audio_config_is_an_error_section_7_2() {
    let (mut l, mut peer) = listener_vs_raw_station(false);
    let mut bytes = peer.seal(&ctl(Control::Hello(station_hello())));
    bytes.extend(peer.seal(&ctl(Control::Status(status()))));
    l.handle_bytes(0, &bytes);
    assert_listener_protocol_error(
        &mut l,
        &mut peer,
        ProtocolError::Unexpected {
            expected: "audio-config",
            got: "status".into(),
        },
    );
}

#[test]
fn paired_must_name_the_authenticated_station_section_7_10() {
    let (mut l, mut peer) = listener_vs_raw_station(true);
    let mut bytes = peer.seal(&ctl(Control::Hello(station_hello())));
    let wrong = lull_proto::DeviceId::parse(other_key().device_id().as_str()).unwrap();
    bytes.extend(peer.seal(&ctl(Control::Paired { station_id: wrong })));
    l.handle_bytes(0, &bytes);
    assert_listener_protocol_error(&mut l, &mut peer, ProtocolError::StationIdMismatch);
}

#[test]
fn audio_config_must_be_an_offered_codec_section_7_3() {
    let (mut l, mut peer) = listener_vs_raw_station(false);
    let mut bytes = peer.seal(&ctl(Control::Hello(station_hello())));
    bytes.extend(peer.seal(&ctl(Control::AudioConfig { codec: 7 })));
    l.handle_bytes(0, &bytes);
    assert_listener_protocol_error(&mut l, &mut peer, ProtocolError::UnofferedCodec(7));
}

#[test]
fn unknowns_and_pings_allowed_during_setup_section_6_section_10() {
    let (mut l, mut peer) = listener_vs_raw_station(true);
    let mut bytes = peer.seal(&Record::Unknown {
        record_type: 0x7f,
        body: vec![1, 2, 3],
    });
    bytes.extend(peer.seal(&ctl(Control::Hello(station_hello()))));
    bytes.extend(peer.seal(&ctl(Control::Unknown {
        t: "future-thing".into(),
    })));
    bytes.extend(peer.seal(&Record::Ping(77)));
    bytes.extend(peer.seal(&ctl(Control::Paired {
        station_id: station_proto_id(),
    })));
    bytes.extend(peer.seal(&ctl(Control::AudioConfig { codec: 2 })));
    bytes.extend(peer.seal(&ctl(Control::Status(status()))));
    l.handle_bytes(0, &bytes);
    assert!(l.is_connected(), "{:?}", events_l(&mut l));
    assert_eq!(l.stats().ignored, 2);
    let sent = peer.open_all(&drain_l(&mut l, 0));
    assert_eq!(sent, vec![Record::Pong(77)]);
}

#[test]
fn second_hello_is_a_protocol_error() {
    let (mut l, mut peer) = listener_vs_raw_station(false);
    raw_station_setup(&mut l, &mut peer, false);
    events_l(&mut l);
    l.handle_bytes(0, &peer.seal(&ctl(Control::Hello(station_hello()))));
    assert_listener_protocol_error(&mut l, &mut peer, ProtocolError::InvalidHello);
}

#[test]
fn station_requires_listener_hello_first_section_7_2() {
    let (mut s, mut peer) = station_vs_raw_listener();
    s.handle_bytes(0, &peer.seal(&ctl(Control::Listen(Default::default()))));
    assert_eq!(
        events_s(&mut s),
        vec![StationEvent::Failed(SessionError::Protocol(
            ProtocolError::Unexpected {
                expected: "hello",
                got: "listen".into()
            }
        ))]
    );
    let sent = peer.open_all(&drain_s(&mut s, 0));
    assert!(matches!(
        sent.as_slice(),
        [Record::Control(Control::Close {
            reason: CloseReason::ProtocolError,
            ..
        })]
    ));
}

#[test]
fn station_sends_audio_config_then_status_after_hello_section_7_2() {
    let (mut s, mut peer) = station_vs_raw_listener();
    // The station sent only its hello so far.
    assert!(drain_s(&mut s, 0).is_empty());
    s.handle_bytes(0, &peer.seal(&ctl(Control::Hello(listener_hello()))));
    let sent = peer.open_all(&drain_s(&mut s, 0));
    assert_eq!(
        sent,
        vec![
            ctl(Control::AudioConfig { codec: 2 }),
            ctl(Control::Status(status())),
        ]
    );
}

#[test]
fn station_needs_a_common_codec_media_2() {
    let (mut s, mut peer) = station_vs_raw_listener();
    let hello = Hello {
        audio: vec![1],
        ..listener_hello()
    };
    s.handle_bytes(0, &peer.seal(&ctl(Control::Hello(hello))));
    assert_eq!(
        events_s(&mut s),
        vec![StationEvent::Failed(SessionError::Protocol(
            ProtocolError::NoCommonCodec
        ))]
    );
}

#[test]
fn station_rejects_hello_with_station_role() {
    let (mut s, mut peer) = station_vs_raw_listener();
    s.handle_bytes(0, &peer.seal(&ctl(Control::Hello(station_hello()))));
    assert_eq!(
        events_s(&mut s),
        vec![StationEvent::Failed(SessionError::Protocol(
            ProtocolError::InvalidHello
        ))]
    );
}

#[test]
fn invalid_policy_status_fails_setup() {
    let (mut s, mut peer) = station_vs_raw_listener();
    s.policy_mut().status.battery = Some(140);
    s.handle_bytes(0, &peer.seal(&ctl(Control::Hello(listener_hello()))));
    assert!(matches!(
        events_s(&mut s).as_slice(),
        [StationEvent::Failed(SessionError::LocalMessage(_))]
    ));
}
