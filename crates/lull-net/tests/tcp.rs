// SPDX-License-Identifier: AGPL-3.0-or-later
//! TCP listener and dialer (protocol.md §3.2, §3.5).

mod common;

use std::net::{Ipv4Addr, SocketAddr};
use std::time::{Duration, Instant};

use common::STEP;
use lull_net::dial::{dial, dial_first};
use lull_net::{BindScope, Link, LinkKind, ListenConfig, NetError, StationListener};
use lull_proto::{Endpoint, Host};

fn loopback_any() -> ListenConfig {
    ListenConfig {
        port: 0,
        attempts: 1,
        scope: BindScope::Loopback,
    }
}

async fn read_exact(link: &mut Link, n: usize) -> Vec<u8> {
    let mut got = Vec::new();
    while got.len() < n {
        let chunk = tokio::time::timeout(STEP, link.recv())
            .await
            .unwrap()
            .unwrap()
            .expect("unexpected end of stream");
        got.extend(chunk);
    }
    got
}

async fn round_trip(listener: &mut StationListener, endpoint: &Endpoint) {
    let (client, server) = tokio::join!(dial(endpoint, STEP), listener.accept());
    let (mut client, mut server) = (client.unwrap(), server.unwrap());
    assert_eq!(client.kind(), LinkKind::Stream);
    assert_eq!(server.kind(), LinkKind::Stream);
    assert_eq!(client.info().peer, server.info().local);
    assert!(!client.info().tls);

    let big: Vec<u8> = (0..200_000u32).map(|i| i as u8).collect();
    let (sent, got) = tokio::join!(client.send(&big), read_exact(&mut server, big.len()));
    sent.unwrap();
    assert_eq!(got, big);

    server.send(b"LULL\x01\x00").await.unwrap();
    assert_eq!(read_exact(&mut client, 6).await, b"LULL\x01\x00");

    client.close().await.unwrap();
    assert_eq!(server.recv().await.unwrap(), None);
}

#[tokio::test]
async fn listen_and_dial_ipv4_ipv6_and_dns() {
    let mut listener = StationListener::bind(loopback_any()).unwrap();
    let port = listener.port();
    assert_ne!(port, 0);
    round_trip(
        &mut listener,
        &Endpoint::new(Host::Ipv4(Ipv4Addr::LOCALHOST), port),
    )
    .await;
    if listener.local_addrs().iter().any(SocketAddr::is_ipv6) {
        round_trip(
            &mut listener,
            &Endpoint::parse(&format!("[::1]:{port}")).unwrap(),
        )
        .await;
    }
    round_trip(
        &mut listener,
        &Endpoint::new(Host::Dns("localhost".into()), port),
    )
    .await;
}

#[tokio::test]
async fn split_halves_work_concurrently() {
    let mut listener = StationListener::bind(loopback_any()).unwrap();
    let ep = Endpoint::new(Host::Ipv4(Ipv4Addr::LOCALHOST), listener.port());
    let (client, server) = tokio::join!(dial(&ep, STEP), listener.accept());
    let (mut r, mut w) = client.unwrap().split();
    let mut server = server.unwrap();
    let echo = tokio::spawn(async move {
        while let Some(chunk) = server.recv().await.unwrap() {
            server.send(&chunk).await.unwrap();
        }
    });
    let writer = tokio::spawn(async move {
        for i in 0..100u8 {
            w.send(&[i; 1000]).await.unwrap();
        }
        w.close().await.unwrap();
    });
    let mut total = 0;
    while let Some(chunk) = r.recv().await.unwrap() {
        total += chunk.len();
    }
    assert_eq!(total, 100_000);
    writer.await.unwrap();
    echo.await.unwrap();
}

#[tokio::test]
async fn all_interfaces_port_fallback() {
    // A foreign socket holds the first port of the range.
    let blocker = std::net::TcpListener::bind((Ipv4Addr::UNSPECIFIED, 0)).unwrap();
    let base = blocker.local_addr().unwrap().port();
    let mut listener = StationListener::bind(ListenConfig::new(base)).unwrap();
    let port = listener.port();
    assert!(port > base && port < base + 10, "{base} -> {port}");
    // A second station on the same range moves further up.
    let second = StationListener::bind(ListenConfig::new(base)).unwrap();
    assert!(second.port() > port && second.port() < base + 10);
    round_trip(
        &mut listener,
        &Endpoint::new(Host::Ipv4(Ipv4Addr::LOCALHOST), port),
    )
    .await;
}

#[tokio::test]
async fn loopback_port_fallback_and_exhaustion() {
    let blocker = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let base = blocker.local_addr().unwrap().port();
    let cfg = ListenConfig::new(base).loopback();
    let listener = StationListener::bind(cfg).unwrap();
    assert!(listener.port() > base && listener.port() < base + 10);

    let only_base = ListenConfig { attempts: 1, ..cfg };
    match StationListener::bind(only_base) {
        Err(NetError::NoPort { first, last, .. }) => assert_eq!((first, last), (base, base)),
        other => panic!("expected NoPort, got {other:?}"),
    }
}

/// A port that refuses connections (bound, then released).
fn dead_port() -> u16 {
    let l = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    l.local_addr().unwrap().port()
}

#[tokio::test]
async fn dial_first_skips_dead_endpoint() {
    let mut listener = StationListener::bind(loopback_any()).unwrap();
    let dead = Endpoint::new(Host::Ipv4(Ipv4Addr::LOCALHOST), dead_port());
    let live = Endpoint::new(Host::Ipv4(Ipv4Addr::LOCALHOST), listener.port());
    let candidates = [dead.clone(), live.clone()];
    let (res, accepted) = tokio::join!(
        dial_first(&candidates, Duration::from_secs(2)),
        listener.accept()
    );
    let (link, used) = res.unwrap();
    assert_eq!(used, live);
    assert_eq!(link.kind(), LinkKind::Stream);
    accepted.unwrap();

    assert!(dial_first(&[dead], Duration::from_secs(2)).await.is_err());
    assert!(matches!(
        dial_first(&[], Duration::from_secs(2)).await,
        Err(NetError::NoEndpoints)
    ));
}

#[tokio::test]
async fn dial_timeout() {
    // A listener with a full accept queue never answers further SYNs.
    let sock = socket2::Socket::new(socket2::Domain::IPV4, socket2::Type::STREAM, None).unwrap();
    sock.bind(&SocketAddr::from((Ipv4Addr::LOCALHOST, 0)).into())
        .unwrap();
    sock.listen(0).unwrap();
    let addr = sock.local_addr().unwrap().as_socket().unwrap();
    let ep = Endpoint::new(Host::Ipv4(Ipv4Addr::LOCALHOST), addr.port());
    let mut held = Vec::new();
    let mut timed_out = false;
    for _ in 0..16 {
        let start = Instant::now();
        match dial(&ep, Duration::from_millis(300)).await {
            Ok(link) => held.push(link),
            Err(NetError::Timeout) => {
                let t = start.elapsed();
                assert!(
                    t >= Duration::from_millis(250) && t < Duration::from_secs(3),
                    "{t:?}"
                );
                timed_out = true;
                break;
            }
            Err(e) => panic!("unexpected error {e}"),
        }
    }
    assert!(timed_out, "accept queue never filled");
}
