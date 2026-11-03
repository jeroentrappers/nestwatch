// SPDX-License-Identifier: AGPL-3.0-or-later
//! Hole punching and the direct QUIC transport on loopback (rendezvous.md
//! §5): candidate selection by probes, a byte stream framed like TCP
//! (protocol.md §3.1), and an address-translating middlebox between the
//! two sides.
//!
//! Loopback cannot reproduce real NAT filtering; the forwarder below only
//! models what matters to the protocol: the peers see each other at
//! addresses that are not their local ones, one side cannot reach the other
//! directly, and the path forms only because probes and QUIC replies follow
//! the source address.
#![cfg(feature = "direct")]

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use nestwatch_net::{Link, LinkKind};
use nestwatch_proto::frame::{FrameDecoder, encode_frame};
use nestwatch_relay::Candidate;
use nestwatch_relay::direct::{
    PunchConfig, PunchError, PunchRole, punch_direct, punch_direct_with,
};
use tokio::net::UdpSocket;
use tokio::sync::Mutex;

const WAIT: Duration = Duration::from_secs(20);

async fn socket() -> UdpSocket {
    UdpSocket::bind("127.0.0.1:0").await.unwrap()
}

fn udp(a: SocketAddr) -> Candidate {
    Candidate::Udp(a)
}

/// Unreachable candidates: TEST-NET-1 and a loopback port nobody uses.
async fn bogus() -> Vec<Candidate> {
    let closed = socket().await.local_addr().unwrap();
    vec![
        Candidate::parse("udp:192.0.2.1:40000").unwrap(),
        udp(closed),
        Candidate::parse("tcp:nursery.example:47100").unwrap(),
    ]
}

/// Sends NWP/1-style frames of many sizes from the listener; the station
/// decodes and echoes each message; the listener checks the echo.
async fn exchange_frames(mut listener: Link, mut station: Link) {
    assert_eq!(listener.kind(), LinkKind::Stream);
    assert_eq!(station.kind(), LinkKind::Stream);
    let sizes = [1usize, 6, 100, 1400, 1500, 16_000, 65_535, 3];
    let echo = tokio::spawn(async move {
        let mut dec = FrameDecoder::new();
        let mut n = 0;
        while n < sizes.len() {
            match dec.next_message().unwrap() {
                Some(m) => {
                    station.send(&encode_frame(&m).unwrap()).await.unwrap();
                    n += 1;
                }
                None => dec.push(&station.recv().await.unwrap().expect("open")),
            }
        }
        // End of stream after the listener closes.
        assert_eq!(station.recv().await.unwrap(), None);
        station.close().await.unwrap();
    });
    let mut dec = FrameDecoder::new();
    for (i, len) in sizes.iter().enumerate() {
        let msg: Vec<u8> = (0..*len).map(|j| (i * 31 + j) as u8).collect();
        listener.send(&encode_frame(&msg).unwrap()).await.unwrap();
        let got = loop {
            if let Some(m) = dec.next_message().unwrap() {
                break m;
            }
            dec.push(&listener.recv().await.unwrap().expect("open"));
        };
        assert_eq!(got, msg, "frame {i}");
    }
    listener.close().await.unwrap();
    assert_eq!(listener.recv().await.unwrap(), None);
    tokio::time::timeout(WAIT, echo).await.unwrap().unwrap();
}

#[tokio::test]
async fn punches_to_the_answering_candidate_and_carries_framed_bytes() {
    let (s, l) = (socket().await, socket().await);
    let (s_addr, l_addr) = (s.local_addr().unwrap(), l.local_addr().unwrap());
    let mut s_peer = bogus().await;
    s_peer.push(udp(l_addr));
    let mut l_peer = bogus().await;
    l_peer.insert(1, udp(s_addr));
    let station = tokio::spawn(async move { punch_direct(s, &s_peer, PunchRole::Station).await });
    let mut listener = punch_direct(l, &l_peer, PunchRole::Listener)
        .await
        .expect("direct path");
    // The station sees the stream only once the listener sends.
    listener
        .send(&encode_frame(b"NEST\x01\x01").unwrap())
        .await
        .unwrap();
    let station = tokio::time::timeout(WAIT, station).await.unwrap().unwrap();
    let station = station.expect("station side");
    assert_eq!(listener.info().peer, Some(s_addr));
    assert_eq!(station.info().peer, Some(l_addr));
    assert!(!listener.info().tls, "QUIC TLS authenticates nothing");
    // Consume the first frame, then run the exchange.
    let mut station = station;
    let mut dec = FrameDecoder::new();
    while dec.next_message().unwrap().is_none() {
        dec.push(&station.recv().await.unwrap().unwrap());
    }
    exchange_frames(listener, station).await;
}

/// A UDP forwarder in front of `target`: every external sender gets its
/// own inside socket (a mapping), and replies to that socket go back out
/// from the public address, like a NAT with a port forward.
async fn forwarder(target: SocketAddr) -> SocketAddr {
    let public = Arc::new(socket().await);
    let addr = public.local_addr().unwrap();
    let map: Arc<Mutex<HashMap<SocketAddr, Arc<UdpSocket>>>> = Arc::default();
    tokio::spawn(async move {
        let mut buf = vec![0u8; 2048];
        loop {
            let Ok((n, from)) = public.recv_from(&mut buf).await else {
                continue;
            };
            let inside = {
                let mut m = map.lock().await;
                if let Some(s) = m.get(&from) {
                    s.clone()
                } else {
                    let s = Arc::new(socket().await);
                    m.insert(from, s.clone());
                    let (back, public) = (s.clone(), public.clone());
                    tokio::spawn(async move {
                        let mut buf = vec![0u8; 2048];
                        while let Ok((n, src)) = back.recv_from(&mut buf).await {
                            if src == target {
                                let _ = public.send_to(&buf[..n], from).await;
                            }
                        }
                    });
                    s
                }
            };
            let _ = inside.send_to(&buf[..n], target).await;
        }
    });
    addr
}

#[tokio::test]
async fn punches_through_an_address_translating_middlebox() {
    let (s, l) = (socket().await, socket().await);
    let s_local = s.local_addr().unwrap();
    let public = forwarder(s_local).await;
    // The listener only knows the station's public address; the station
    // cannot reach the listener's candidates at all (its NAT filters).
    let l_peer = vec![udp(public), Candidate::parse("udp:192.0.2.7:1").unwrap()];
    let s_peer = vec![Candidate::parse("udp:192.0.2.8:2").unwrap()];
    let station = tokio::spawn(async move { punch_direct(s, &s_peer, PunchRole::Station).await });
    let mut listener = punch_direct(l, &l_peer, PunchRole::Listener)
        .await
        .expect("path through the middlebox");
    assert_eq!(listener.info().peer, Some(public));
    listener
        .send(&encode_frame(b"NEST\x01\x01").unwrap())
        .await
        .unwrap();
    let mut station = tokio::time::timeout(WAIT, station)
        .await
        .unwrap()
        .unwrap()
        .expect("station side");
    // The station sees the forwarder's mapping, not the listener's socket.
    assert_ne!(station.info().peer, listener.info().local);
    let mut dec = FrameDecoder::new();
    while dec.next_message().unwrap().is_none() {
        dec.push(&station.recv().await.unwrap().unwrap());
    }
    exchange_frames(listener, station).await;
}

#[tokio::test]
async fn no_path_ends_in_time() {
    let cfg = PunchConfig {
        probe_duration: Duration::from_millis(300),
        handshake_timeout: Duration::from_millis(300),
        ..PunchConfig::default()
    };
    let t = tokio::time::Instant::now();
    let r = punch_direct_with(socket().await, &bogus().await, PunchRole::Listener, &cfg).await;
    assert!(matches!(r, Err(PunchError::Timeout)), "{r:?}");
    assert!(t.elapsed() < Duration::from_secs(3));
    let r = punch_direct_with(socket().await, &bogus().await, PunchRole::Station, &cfg).await;
    assert!(matches!(r, Err(PunchError::Timeout)), "{r:?}");
    let tcp_only = vec![Candidate::parse("tcp:a.example:1").unwrap()];
    let r = punch_direct_with(socket().await, &tcp_only, PunchRole::Listener, &cfg).await;
    assert!(matches!(r, Err(PunchError::NoCandidates)), "{r:?}");
    // An IPv4 socket cannot use IPv6 candidates.
    let v6 = vec![Candidate::parse("udp:[2001:db8::1]:5").unwrap()];
    let r = punch_direct_with(socket().await, &v6, PunchRole::Listener, &cfg).await;
    assert!(matches!(r, Err(PunchError::NoCandidates)), "{r:?}");
}

#[tokio::test]
async fn stun_then_punch_on_the_same_socket() {
    // A minimal STUN responder built from the crate's codec.
    let stun = socket().await;
    let stun_addr = stun.local_addr().unwrap();
    tokio::spawn(async move {
        let mut buf = [0u8; 1500];
        while let Ok((n, from)) = stun.recv_from(&mut buf).await {
            if let Some(r) = nestwatch_relay::stun::answer(&buf[..n], from) {
                let _ = stun.send_to(&r, from).await;
            }
        }
    });
    let (s, l) = (socket().await, socket().await);
    let t = Duration::from_secs(3);
    let s_srflx = nestwatch_relay::stun::stun_binding(&s, stun_addr, t)
        .await
        .unwrap();
    let l_srflx = nestwatch_relay::stun::stun_binding(&l, stun_addr, t)
        .await
        .unwrap();
    assert_eq!(s_srflx, s.local_addr().unwrap());
    let station =
        tokio::spawn(async move { punch_direct(s, &[udp(l_srflx)], PunchRole::Station).await });
    let mut listener = punch_direct(l, &[udp(s_srflx)], PunchRole::Listener)
        .await
        .unwrap();
    listener.send(b"\x00\x01x").await.unwrap();
    let mut station = station.await.unwrap().unwrap();
    assert_eq!(station.recv().await.unwrap().unwrap(), b"\x00\x01x");
}
