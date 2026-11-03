// SPDX-License-Identifier: AGPL-3.0-or-later
//! Web server: static files, WebSocket links and HTTPS (protocol.md §3.3).

mod common;

use std::net::Ipv4Addr;
use std::sync::Arc;

use common::{STEP, http_request};
use futures_util::{SinkExt, StreamExt};
use lull_net::web::{AssetSource, StationCert, WebConfig, WebServer};
use lull_net::ws_client::{WsClientConfig, connect, connect_with};
use lull_net::{BindScope, Link, LinkKind, ListenConfig, MAX_MESSAGE_LEN, NetError};
use lull_proto::Host;
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

fn loopback_any() -> Option<ListenConfig> {
    Some(ListenConfig {
        port: 0,
        attempts: 1,
        scope: BindScope::Loopback,
    })
}

fn assets() -> AssetSource {
    AssetSource::memory([
        ("/index.html", &b"<!doctype html><title>Lull</title>"[..]),
        ("app.js", &b"console.log(1)"[..]),
        ("pkg/app_bg.wasm", &b"\0asm\x01\0\0\0"[..]),
        ("style.css", &b"body{}"[..]),
    ])
}

async fn http_server() -> (WebServer, u16) {
    let cfg = WebConfig {
        http: loopback_any(),
        https: None,
        ..WebConfig::default()
    };
    let server = WebServer::start(cfg, assets(), None).await.unwrap();
    let port = server.http_port().unwrap();
    assert_eq!(server.https_port(), None);
    (server, port)
}

async fn get(port: u16, method: &str, path: &str) -> common::HttpResponse {
    let s = TcpStream::connect((Ipv4Addr::LOCALHOST, port))
        .await
        .unwrap();
    http_request(s, method, path, &format!("127.0.0.1:{port}")).await
}

#[tokio::test]
async fn static_files_and_security_headers() {
    let (_server, port) = http_server().await;

    let r = get(port, "GET", "/").await;
    assert_eq!(r.status, 200);
    assert_eq!(r.body, b"<!doctype html><title>Lull</title>");
    assert_eq!(r.header("content-type"), Some("text/html; charset=utf-8"));
    let csp = r.header("content-security-policy").unwrap();
    assert!(csp.starts_with("default-src 'none'; script-src 'self' 'wasm-unsafe-eval';"));
    assert!(csp.contains(&format!(
        "connect-src 'self' ws://127.0.0.1:{port} wss://127.0.0.1:{port};"
    )));
    assert!(csp.contains("frame-ancestors 'none'"));
    assert!(!csp.contains('*') && !csp.contains("http"));
    assert_eq!(r.header("x-content-type-options"), Some("nosniff"));
    assert_eq!(r.header("x-frame-options"), Some("DENY"));
    assert_eq!(r.header("referrer-policy"), Some("no-referrer"));
    assert_eq!(r.header("cross-origin-opener-policy"), Some("same-origin"));

    let r = get(port, "GET", "/pkg/app_bg.wasm").await;
    assert_eq!(
        (r.status, r.header("content-type")),
        (200, Some("application/wasm"))
    );
    let r = get(port, "GET", "/app.js").await;
    assert_eq!(
        r.header("content-type"),
        Some("text/javascript; charset=utf-8")
    );
    let r = get(port, "HEAD", "/style.css").await;
    assert_eq!((r.status, r.header("content-length")), (200, Some("6")));
    assert!(r.body.is_empty());

    // App routes fall back to index.html; missing files with an extension do not.
    let r = get(port, "GET", "/listen/station").await;
    assert_eq!((r.status, r.body.len()), (200, 34));
    let r = get(port, "GET", "/missing.js").await;
    assert_eq!(r.status, 404);
    assert_eq!(r.header("x-content-type-options"), Some("nosniff"));
    assert!(r.header("content-security-policy").is_some());
    for bad in ["/../index.html", "/%2e%2e/index.html", "/a\\b"] {
        assert_eq!(get(port, "GET", bad).await.status, 404, "{bad}");
    }
    let r = get(port, "POST", "/").await;
    assert_eq!((r.status, r.header("allow")), (405, Some("GET, HEAD")));
    // A plain GET of the WebSocket path is refused.
    assert_eq!(get(port, "GET", "/lull/1").await.status, 426);
}

#[tokio::test]
async fn directory_source() {
    let root = std::env::temp_dir().join(format!("lull-net-assets-{}", std::process::id()));
    let outside = root.with_extension("secret");
    std::fs::create_dir_all(root.join("sub")).unwrap();
    std::fs::write(root.join("index.html"), "home").unwrap();
    std::fs::write(root.join("sub/data.json"), "{}").unwrap();
    std::fs::write(&outside, "secret").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside, root.join("link.txt")).unwrap();

    let cfg = WebConfig {
        http: loopback_any(),
        https: None,
        ..WebConfig::default()
    };
    let server = WebServer::start(cfg, AssetSource::Dir(root.clone()), None)
        .await
        .unwrap();
    let port = server.http_port().unwrap();
    let r = get(port, "GET", "/sub/data.json").await;
    assert_eq!(
        (r.status, r.header("content-type")),
        (200, Some("application/json"))
    );
    assert_eq!(get(port, "GET", "/").await.body, b"home");
    #[cfg(unix)]
    assert_eq!(get(port, "GET", "/link.txt").await.status, 404);
    drop(server);
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_file(&outside);
}

async fn ws_pair(server: &mut WebServer, url: &str, cfg: &WsClientConfig) -> (Link, Link) {
    let (client, accepted) = tokio::join!(connect_with(url, cfg), async {
        tokio::time::timeout(STEP, server.accept()).await.unwrap()
    });
    (client.unwrap(), accepted.unwrap())
}

#[tokio::test]
async fn websocket_round_trip() {
    let (mut server, port) = http_server().await;
    let url = format!("ws://127.0.0.1:{port}/lull/1");
    let (mut client, mut station) = ws_pair(&mut server, &url, &WsClientConfig::default()).await;
    assert_eq!(client.kind(), LinkKind::Messages);
    assert_eq!(station.kind(), LinkKind::Messages);
    assert_eq!(station.info().peer, client.info().local);
    assert!(!station.info().tls);

    // Message boundaries are kept, including the largest Noise message.
    client.send(b"LULL\x01\x01").await.unwrap();
    client.send(&[]).await.unwrap();
    let max = vec![0x5a; MAX_MESSAGE_LEN];
    client.send(&max).await.unwrap();
    assert_eq!(station.recv().await.unwrap().unwrap(), b"LULL\x01\x01");
    assert_eq!(station.recv().await.unwrap().unwrap(), b"");
    assert_eq!(station.recv().await.unwrap().unwrap(), max);
    station.send(b"LULL\x01\x00").await.unwrap();
    assert_eq!(client.recv().await.unwrap().unwrap(), b"LULL\x01\x00");

    assert!(matches!(
        client.send(&vec![0; MAX_MESSAGE_LEN + 1]).await,
        Err(NetError::MessageTooLarge(_))
    ));

    // Split halves and a clean close.
    let (mut r, mut w) = station.split();
    w.send(b"bye").await.unwrap();
    assert_eq!(client.recv().await.unwrap().unwrap(), b"bye");
    client.close().await.unwrap();
    assert_eq!(r.recv().await.unwrap(), None);

    // The plain `connect` helper works for ws:// too.
    let (c, s) = tokio::join!(connect(&url), server.accept());
    c.unwrap();
    s.unwrap();
}

/// A raw tungstenite client, to send what the Lull client never sends.
async fn raw_client(
    port: u16,
    origin: Option<&str>,
) -> Result<tokio_tungstenite::WebSocketStream<TcpStream>, tokio_tungstenite::tungstenite::Error> {
    let mut req = format!("ws://127.0.0.1:{port}/lull/1")
        .into_client_request()
        .unwrap();
    if let Some(o) = origin {
        req.headers_mut().insert("origin", o.parse().unwrap());
    }
    let tcp = TcpStream::connect((Ipv4Addr::LOCALHOST, port))
        .await
        .unwrap();
    tokio_tungstenite::client_async(req, tcp)
        .await
        .map(|(ws, _)| ws)
}

#[tokio::test]
async fn text_and_oversized_messages_are_errors() {
    let (mut server, port) = http_server().await;

    let mut raw = raw_client(port, None).await.unwrap();
    let mut station = server.accept().await.unwrap();
    raw.send(Message::Ping(vec![1].into())).await.unwrap();
    raw.send(Message::text("hello")).await.unwrap();
    assert!(matches!(station.recv().await, Err(NetError::TextMessage)));
    // Pings were answered without surfacing.
    let pong = tokio::time::timeout(STEP, raw.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(matches!(pong, Message::Pong(_)));

    let mut raw = raw_client(port, None).await.unwrap();
    let mut station = server.accept().await.unwrap();
    raw.send(Message::binary(vec![0; 70_000])).await.unwrap();
    assert!(station.recv().await.is_err());
}

#[tokio::test]
async fn cross_origin_upgrades_are_refused() {
    let (mut server, port) = http_server().await;
    assert!(raw_client(port, Some("http://evil.example")).await.is_err());
    assert!(raw_client(port, Some("null")).await.is_err());
    let ok = raw_client(port, Some(&format!("http://127.0.0.1:{port}"))).await;
    assert!(ok.is_ok());
    server.accept().await.unwrap();
}

fn test_cert() -> StationCert {
    StationCert::generate(&[
        Host::Dns("localhost".into()),
        Host::Ipv4(Ipv4Addr::LOCALHOST),
        Host::parse("[::1]").unwrap(),
    ])
    .unwrap()
}

#[tokio::test]
async fn https_with_self_signed_certificate() {
    let cert = test_cert();
    let cfg = WebConfig {
        http: loopback_any(),
        https: loopback_any(),
        ..WebConfig::default()
    };
    let mut server = WebServer::start(cfg, assets(), Some(&cert)).await.unwrap();
    let port = server.https_port().unwrap();
    assert_ne!(Some(port), server.http_port());

    // WSS with the station certificate trusted.
    let trusting = WsClientConfig::default().trust(&cert);
    for host in ["127.0.0.1", "localhost"] {
        let url = format!("wss://{host}:{port}/lull/1");
        let (mut client, mut station) = ws_pair(&mut server, &url, &trusting).await;
        assert!(client.info().tls && station.info().tls);
        client.send(b"secure").await.unwrap();
        assert_eq!(station.recv().await.unwrap().unwrap(), b"secure");
    }

    // Without trust the handshake fails.
    let url = format!("wss://127.0.0.1:{port}/lull/1");
    assert!(matches!(
        connect_with(&url, &WsClientConfig::default()).await,
        Err(NetError::Tls(_))
    ));
    // A different certificate is not accepted either.
    let other = WsClientConfig::default().trust(&test_cert());
    assert!(connect_with(&url, &other).await.is_err());

    // Static files over HTTPS.
    let mut roots = rustls::RootCertStore::empty();
    roots.add(cert.cert_der().clone()).unwrap();
    let tls = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_root_certificates(roots)
    .with_no_client_auth();
    let tcp = TcpStream::connect((Ipv4Addr::LOCALHOST, port))
        .await
        .unwrap();
    let name = rustls::pki_types::ServerName::try_from("127.0.0.1").unwrap();
    let s = tokio_rustls::TlsConnector::from(Arc::new(tls))
        .connect(name, tcp)
        .await
        .unwrap();
    let r = http_request(s, "GET", "/app.js", &format!("127.0.0.1:{port}")).await;
    assert_eq!((r.status, r.body.as_slice()), (200, &b"console.log(1)"[..]));
    assert_eq!(r.header("x-content-type-options"), Some("nosniff"));
}

#[tokio::test]
async fn https_requires_a_certificate() {
    let cfg = WebConfig {
        http: None,
        https: loopback_any(),
        ..WebConfig::default()
    };
    assert!(matches!(
        WebServer::start(cfg, assets(), None).await,
        Err(NetError::Certificate(_))
    ));
}

#[tokio::test]
async fn http_and_https_fall_back_upward() {
    let blocker = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let base = blocker.local_addr().unwrap().port();
    let cert = test_cert();
    let cfg = WebConfig {
        http: Some(ListenConfig::new(base).loopback()),
        https: Some(ListenConfig::new(base + 1).loopback()),
        ..WebConfig::default()
    };
    let server = WebServer::start(cfg, assets(), Some(&cert)).await.unwrap();
    let (http, https) = (server.http_port().unwrap(), server.https_port().unwrap());
    // HTTP takes base + 1 (or higher), HTTPS moves past it.
    assert!(
        http > base && https > http && https < base + 11,
        "{http} {https}"
    );
}
