// SPDX-License-Identifier: AGPL-3.0-or-later
//! Keep-alive and liveness (protocol.md §7.9, NFR-REL-1).

use lull_proto::control::{CloseReason, Control};
use lull_proto::record::Record;

use super::harness::*;
use crate::{Flavor, ListenerEvent, SessionError, StationEvent, TimeoutKind};

fn connected_raw() -> (crate::ListenerSession, RawPeer) {
    let (mut l, mut peer) = listener_vs_raw_station(false);
    raw_station_setup(&mut l, &mut peer, false);
    events_l(&mut l);
    (l, peer)
}

#[test]
fn ping_after_5_s_without_sending_and_rtt_section_7_9() {
    let (mut l, mut peer) = connected_raw();
    // The listener last sent (its hello) at t = 0.
    assert_eq!(l.poll_timeout(), Some(5_000));
    l.handle_timeout(4_999);
    assert!(drain_l(&mut l, 4_999).is_empty());
    l.handle_timeout(5_000);
    assert_eq!(
        peer.open_all(&drain_l(&mut l, 5_000)),
        vec![Record::Ping(5_000)]
    );
    // A PONG for something never sent is ignored; the real one gives RTT.
    l.handle_bytes(5_050, &peer.seal(&Record::Pong(1)));
    l.handle_bytes(5_120, &peer.seal(&Record::Pong(5_000)));
    assert_eq!(events_l(&mut l), vec![ListenerEvent::Rtt(120)]);
    // Next PING 5 s after the last send.
    assert_eq!(l.poll_timeout(), Some(10_000));
}

#[test]
fn sending_anything_postpones_the_ping_section_7_9() {
    let (mut l, mut peer) = connected_raw();
    l.handle_bytes(3_000, &peer.seal(&Record::Ping(3_000)));
    // The PONG counts as sending.
    assert_eq!(
        peer.open_all(&drain_l(&mut l, 3_000)),
        vec![Record::Pong(3_000)]
    );
    assert_eq!(l.poll_timeout(), Some(8_000));
    l.send_control(Control::VideoStop).unwrap();
    drain_l(&mut l, 7_000);
    assert_eq!(l.poll_timeout(), Some(12_000));
}

#[test]
fn pong_answers_ping_immediately_section_7_9() {
    let (mut l, mut peer) = connected_raw();
    l.handle_bytes(100, &peer.seal(&Record::Ping(0xDEAD_BEEF)));
    assert_eq!(
        peer.open_all(&drain_l(&mut l, 100)),
        vec![Record::Pong(0xDEAD_BEEF)]
    );
}

#[test]
fn nothing_received_for_15_s_closes_nfr_rel_1() {
    let (mut l, mut peer) = connected_raw();
    let mut sent = Vec::new();
    for t in [5_000, 10_000] {
        l.handle_timeout(t);
        sent.extend(peer.open_all(&drain_l(&mut l, t)));
    }
    assert_eq!(sent, vec![Record::Ping(5_000), Record::Ping(10_000)]);
    assert_eq!(l.poll_timeout(), Some(15_000));
    l.handle_timeout(14_999);
    assert!(events_l(&mut l).is_empty());
    l.handle_timeout(15_000);
    assert_eq!(
        events_l(&mut l),
        vec![ListenerEvent::Failed(SessionError::Timeout(
            TimeoutKind::Idle
        ))]
    );
    assert_eq!(
        peer.open_all(&drain_l(&mut l, 15_000)),
        vec![Record::Control(Control::Close {
            reason: CloseReason::Idle,
            detail: None
        })]
    );
    assert!(l.is_finished());
}

#[test]
fn receiving_anything_resets_the_idle_timer_section_7_9() {
    let (mut l, mut peer) = connected_raw();
    l.handle_bytes(9_000, &peer.seal(&Record::Pong(42)));
    l.handle_timeout(15_000);
    assert!(events_l(&mut l).is_empty());
    l.handle_timeout(24_000);
    assert_eq!(
        events_l(&mut l),
        vec![ListenerEvent::Failed(SessionError::Timeout(
            TimeoutKind::Idle
        ))]
    );
}

#[test]
fn idle_session_stays_up_on_keepalives_alone_section_7_9() {
    for flavor in [Flavor::Tcp, Flavor::WebSocket] {
        let mut net = Net::new(flavor, trusted_mode(), TestPolicy::trusting());
        net.delay = 40;
        net.connect();
        net.run_for(60_000);
        assert!(net.l.is_connected() && net.s.is_connected());
        let rtts: Vec<_> = net
            .l_events
            .iter()
            .filter_map(|e| match e {
                ListenerEvent::Rtt(ms) => Some(*ms),
                _ => None,
            })
            .collect();
        assert!(rtts.len() >= 10, "{rtts:?}");
        assert!(rtts.iter().all(|&ms| ms == 80));
        assert!(net.l_failed().is_none() && net.s_failed().is_none());
    }
}

#[test]
fn dead_link_detected_on_both_sides_within_15_s_nfr_rel_1() {
    let mut net = Net::new(Flavor::Tcp, trusted_mode(), TestPolicy::trusting());
    net.delay = 10;
    net.connect();
    net.run_for(2_000);
    let cut = net.now;
    net.link_up = false;
    net.run_for(15_000);
    // Both detect it within 15 s of the cut.
    assert_eq!(
        net.l_failed(),
        Some(&SessionError::Timeout(TimeoutKind::Idle))
    );
    assert_eq!(
        net.s_failed(),
        Some(&SessionError::Timeout(TimeoutKind::Idle))
    );
    assert_eq!(net.now, cut + 15_000);
    assert!(net.l.is_finished() && net.s.is_finished());
    assert!(
        !net.s_events
            .iter()
            .any(|e| matches!(e, StationEvent::Closed { .. }))
    );
}
