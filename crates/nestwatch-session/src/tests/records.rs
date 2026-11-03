// SPDX-License-Identifier: AGPL-3.0-or-later
//! Record rules (protocol.md §6), talk-back (§7.8), close (§7.12) and the
//! nonce limit (§5.6).

use nestwatch_crypto::TransportError;
use nestwatch_proto::control::{CloseReason, Control, TalkStatus};
use nestwatch_proto::frame::WsKind;
use nestwatch_proto::record::{AudioFlags, AudioHeader, AudioStream, Record};

use super::harness::*;
use crate::{Flavor, ListenerEvent, ProtocolError, SendError, SessionError, StationEvent};

pub fn audio(stream: AudioStream, seq: u32) -> AudioHeader {
    AudioHeader {
        stream,
        codec: 2,
        flags: AudioFlags::default(),
        seq,
        timestamp: seq.wrapping_mul(960),
    }
}

fn connected_raw() -> (crate::ListenerSession, RawPeer) {
    let (mut l, mut peer) = listener_vs_raw_station(false);
    raw_station_setup(&mut l, &mut peer, false);
    events_l(&mut l);
    (l, peer)
}

fn assert_protocol_close(l: &mut crate::ListenerSession, peer: &mut RawPeer) {
    let events = events_l(l);
    assert!(
        matches!(
            events.last(),
            Some(ListenerEvent::Failed(SessionError::Protocol(
                ProtocolError::MalformedRecord(_)
            )))
        ),
        "{events:?}"
    );
    let sent = peer.open_all(&drain_l(l, 0));
    assert!(matches!(
        sent.last(),
        Some(Record::Control(Control::Close {
            reason: CloseReason::ProtocolError,
            ..
        }))
    ));
    assert!(l.is_finished());
}

#[test]
fn malformed_bodies_end_with_protocol_error_section_6() {
    let cases: [&[u8]; 6] = [
        &[0x02, 0x00, 0x02],                         // AUDIO header truncated
        &[0x02, 0x05, 0x02, 0, 0, 0, 0, 0, 0, 0, 0], // unknown stream (PROTO-8)
        &[0x04, 1, 2, 3],                            // PING not 8 bytes
        &[0x01, 0xff],                               // CBOR break outside indefinite item
        &[0x01, 0xa1, 0x61, b't', 0x65, b'l', b'e', b'v', b'e', b'l'], // level without keys
        &[],                                         // empty record
    ];
    for plain in cases {
        let (mut l, mut peer) = connected_raw();
        l.handle_bytes(0, &peer.seal_plain(plain));
        assert_protocol_close(&mut l, &mut peer);
    }
    // Missing required key (§7.1).
    let (mut l, mut peer) = connected_raw();
    // {"t": "audio-config"} without "codec".
    let mut plain = vec![0x01, 0xa1, 0x61, b't', 0x6c];
    plain.extend_from_slice(b"audio-config");
    l.handle_bytes(0, &peer.seal_plain(&plain));
    assert_protocol_close(&mut l, &mut peer);
}

#[test]
fn unknown_record_types_and_messages_are_ignored_section_6_section_7_1() {
    let (mut l, mut peer) = connected_raw();
    let mut bytes = peer.seal(&Record::Unknown {
        record_type: 0x42,
        body: vec![9; 10],
    });
    bytes.extend(peer.seal(&Record::Control(Control::Unknown {
        t: "lullaby".into(),
    })));
    // A station→listener message sent the wrong way is ignored too.
    bytes.extend(peer.seal(&Record::Control(Control::TalkRequest)));
    bytes.extend(peer.seal(&Record::Audio {
        header: audio(AudioStream::Station, 0),
        payload: vec![1],
    }));
    l.handle_bytes(0, &bytes);
    assert_eq!(l.stats().ignored, 3);
    assert!(l.is_connected());
    assert_eq!(
        events_l(&mut l),
        vec![ListenerEvent::Audio {
            header: audio(AudioStream::Station, 0),
            payload: vec![1]
        }]
    );
}

#[test]
fn audio_seq_not_newer_is_discarded_section_6_1() {
    let (mut l, mut peer) = connected_raw();
    let mut bytes = Vec::new();
    for seq in [0, 1, 1, 0, 5, 3, 6, 6] {
        bytes.extend(peer.seal(&Record::Audio {
            header: audio(AudioStream::Station, seq),
            payload: vec![seq as u8],
        }));
    }
    l.handle_bytes(0, &bytes);
    let seqs: Vec<u32> = events_l(&mut l)
        .into_iter()
        .filter_map(|e| match e {
            ListenerEvent::Audio { header, .. } => Some(header.seq),
            _ => None,
        })
        .collect();
    // Gaps (2..4) are fine; repeats and older frames are dropped.
    assert_eq!(seqs, vec![0, 1, 5, 6]);
    assert_eq!(l.stats().audio_stale, 4);
}

#[test]
fn talk_back_only_while_granted_section_7_8() {
    let mut net = Net::new(Flavor::Tcp, trusted_mode(), TestPolicy::trusting());
    net.connect();
    let frame = |seq| audio(AudioStream::TalkBack, seq);
    assert_eq!(
        net.l.send_talk_audio(frame(0), &[0; 4]),
        Err(SendError::TalkNotGranted)
    );
    net.l.send_control(Control::TalkRequest).unwrap();
    net.run_for(5);
    assert_eq!(
        net.s_events.last(),
        Some(&StationEvent::Control(Control::TalkRequest))
    );
    net.s
        .send_control(Control::TalkState {
            state: TalkStatus::Granted,
            by: None,
        })
        .unwrap();
    net.run_for(5);
    assert!(net.l.talk_granted());
    for seq in [3, 2, 4] {
        net.l.send_talk_audio(frame(seq), &[seq as u8; 4]).unwrap();
    }
    assert_eq!(
        net.l.send_talk_audio(audio(AudioStream::Station, 9), &[0]),
        Err(SendError::WrongStream)
    );
    let mut wrong_codec = frame(10);
    wrong_codec.codec = 1;
    assert_eq!(
        net.l.send_talk_audio(wrong_codec, &[0]),
        Err(SendError::WrongCodec(1))
    );
    net.run_for(5);
    let got: Vec<u32> = net
        .s_events
        .iter()
        .filter_map(|e| match e {
            StationEvent::Audio { header, .. } => Some(header.seq),
            _ => None,
        })
        .collect();
    assert_eq!(got, vec![3, 4]);
    assert_eq!(net.s.stats().audio_stale, 1);
    // Releasing stops talk-back immediately.
    net.l.send_control(Control::TalkRelease).unwrap();
    assert_eq!(
        net.l.send_talk_audio(frame(11), &[0]),
        Err(SendError::TalkNotGranted)
    );
    // So does a station saying someone else talks.
    net.l.send_control(Control::TalkRequest).unwrap();
    net.s
        .send_control(Control::TalkState {
            state: TalkStatus::Granted,
            by: None,
        })
        .unwrap();
    net.run_for(5);
    assert!(net.l.talk_granted());
    net.s
        .send_control(Control::TalkState {
            state: TalkStatus::Busy,
            by: Some("Grandma".into()),
        })
        .unwrap();
    net.run_for(5);
    assert!(!net.l.talk_granted());
}

#[test]
fn station_audio_rules() {
    let mut net = Net::new(Flavor::Tcp, trusted_mode(), TestPolicy::trusting());
    assert_eq!(
        net.s.send_audio(audio(AudioStream::Station, 0), &[0]),
        Err(SendError::NotConnected)
    );
    net.connect();
    assert_eq!(
        net.s.send_audio(audio(AudioStream::TalkBack, 0), &[0]),
        Err(SendError::WrongStream)
    );
    let mut h = audio(AudioStream::Station, 0);
    h.codec = 1;
    assert_eq!(net.s.send_audio(h, &[0]), Err(SendError::WrongCodec(1)));
    net.s
        .send_audio(audio(AudioStream::Station, 0), &[5; 160])
        .unwrap();
    net.run_for(5);
    assert_eq!(net.l_audio_seqs(), vec![0]);
}

#[test]
fn graceful_close_reasons_section_7_12() {
    for (reason, by_station) in [
        (CloseReason::Shutdown, false),
        (CloseReason::Shutdown, true),
        (CloseReason::Revoked, true),
        (CloseReason::Replaced, true),
        (CloseReason::Other("custom".into()), false),
    ] {
        for flavor in [Flavor::Tcp, Flavor::WebSocket] {
            let mut net = Net::new(flavor, trusted_mode(), TestPolicy::trusting());
            net.connect();
            let detail = Some("bye".to_owned());
            if by_station {
                net.s.close(reason.clone(), detail.clone());
            } else {
                net.l.close(reason.clone(), detail.clone());
            }
            net.run_for(5);
            let closer = ListenerEvent::Closed {
                reason: reason.clone(),
                detail: detail.clone(),
                by_peer: by_station,
            };
            assert_eq!(net.l_events.last(), Some(&closer));
            assert_eq!(
                net.s_events.last(),
                Some(&StationEvent::Closed {
                    reason: reason.clone(),
                    detail,
                    by_peer: !by_station,
                })
            );
            assert!(net.l.is_finished() && net.s.is_finished());
            assert_eq!(
                net.l.send_control(Control::VideoStop),
                Err(SendError::Closed)
            );
        }
    }
}

#[test]
fn close_flushes_queued_control_but_drops_media_section_7_12() {
    let mut net = Net::new(Flavor::Tcp, trusted_mode(), TestPolicy::trusting());
    net.connect();
    net.s
        .send_audio(audio(AudioStream::Station, 0), &[1])
        .unwrap();
    net.s
        .send_control(Control::Activity { open: true })
        .unwrap();
    net.s.close(CloseReason::Revoked, None);
    net.run_for(5);
    assert_eq!(
        &net.l_events[1..],
        &[
            ListenerEvent::Control(Control::Activity { open: true }),
            ListenerEvent::Closed {
                reason: CloseReason::Revoked,
                detail: None,
                by_peer: true
            }
        ]
    );
}

#[test]
fn unpair_is_delivered_to_the_station_section_7_10() {
    let mut net = Net::new(Flavor::WebSocket, trusted_mode(), TestPolicy::trusting());
    net.connect();
    net.l.send_control(Control::Unpair).unwrap();
    net.run_for(5);
    assert_eq!(
        net.s_events.last(),
        Some(&StationEvent::Control(Control::Unpair))
    );
}

#[test]
fn connection_lost_after_setup() {
    let mut net = Net::new(Flavor::Tcp, trusted_mode(), TestPolicy::trusting());
    net.connect();
    net.disconnect();
    assert_eq!(net.l_failed(), Some(&SessionError::ConnectionLost));
    assert_eq!(net.s_failed(), Some(&SessionError::ConnectionLost));
}

#[test]
fn websocket_text_message_is_a_protocol_error_section_3_3() {
    let mut net = Net::new(Flavor::WebSocket, trusted_mode(), TestPolicy::trusting());
    net.connect();
    net.s.handle_ws_message(net.now, WsKind::Text, b"hi");
    net.run_for(5);
    assert!(matches!(
        net.s_failed(),
        Some(SessionError::Protocol(ProtocolError::Framing(_)))
    ));
    assert_eq!(
        net.l_events.last(),
        Some(&ListenerEvent::Closed {
            reason: CloseReason::ProtocolError,
            detail: Some("framing: websocket text message is a protocol error".into()),
            by_peer: true
        })
    );
}

#[test]
fn zero_length_tcp_frame_is_a_protocol_error_section_3_1() {
    let (mut l, mut peer) = connected_raw();
    l.handle_bytes(0, &[0, 0]);
    let events = events_l(&mut l);
    assert!(matches!(
        events.as_slice(),
        [ListenerEvent::Failed(SessionError::Protocol(
            ProtocolError::Framing(_)
        ))]
    ));
    assert!(matches!(
        peer.open_all(&drain_l(&mut l, 0)).as_slice(),
        [Record::Control(Control::Close { .. })]
    ));
}

#[test]
fn tampered_transport_message_fails_without_close_security_3() {
    let (mut l, mut peer) = connected_raw();
    let mut bad = peer.seal(&Record::Ping(1));
    let n = bad.len();
    bad[n - 1] ^= 0x80;
    l.handle_bytes(0, &bad);
    assert_eq!(
        events_l(&mut l),
        vec![ListenerEvent::Failed(SessionError::Transport(
            TransportError::Decrypt
        ))]
    );
    assert!(drain_l(&mut l, 0).is_empty());
}

#[test]
fn flavour_mismatch_fails_the_session() {
    let mut l = listener(Flavor::WebSocket, trusted_mode(), 0);
    l.handle_bytes(0, b"NEST\x01\x00");
    assert_eq!(
        events_l(&mut l),
        vec![ListenerEvent::Failed(SessionError::FlavorMismatch)]
    );
}

/// §5.6: a side closes before its sending nonce runs out; the last nonce
/// carries the `close`.
#[test]
fn sending_nonce_limit_closes_gracefully_section_5_6() {
    let mut net = Net::new(Flavor::Tcp, trusted_mode(), TestPolicy::trusting());
    net.s.set_nonce_limit(10);
    net.connect();
    // Hello, audio-config and status used 3 nonces.
    for seq in 0..20 {
        let _ = net.s.send_audio(audio(AudioStream::Station, seq), &[0]);
    }
    net.run_for(5);
    let reason = CloseReason::Other(crate::NONCE_LIMIT_REASON.into());
    assert_eq!(
        net.s_events.last(),
        Some(&StationEvent::Closed {
            reason: reason.clone(),
            detail: None,
            by_peer: false
        })
    );
    assert_eq!(
        net.l_events.last(),
        Some(&ListenerEvent::Closed {
            reason,
            detail: None,
            by_peer: true
        })
    );
    // 3 setup records + 6 audio frames + the close = 10 messages.
    assert_eq!(net.l_audio_seqs(), (0..6).collect::<Vec<_>>());
}

#[test]
fn receiving_nonce_limit_closes_gracefully_section_5_6() {
    let mut net = Net::new(Flavor::WebSocket, trusted_mode(), TestPolicy::trusting());
    net.l.set_nonce_limit(6);
    net.connect();
    for seq in 0..5 {
        net.s
            .send_audio(audio(AudioStream::Station, seq), &[0])
            .unwrap();
    }
    net.run_for(5);
    // hello, audio-config, status + 3 frames = 6 received, then close.
    assert_eq!(net.l_audio_seqs(), vec![0, 1, 2]);
    assert!(matches!(
        net.s_events.last(),
        Some(StationEvent::Closed { by_peer: true, .. })
    ));
}
