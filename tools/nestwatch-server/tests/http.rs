// SPDX-License-Identifier: AGPL-3.0-or-later
//! The HTTP side (rendezvous.md §6, §8): static directories with MIME
//! types, CSP and caching headers; `/healthz` with aggregate counters only;
//! graceful shutdown.

mod common;

use std::collections::HashMap;
use std::path::Path;

use common::*;
use nestwatch_relay::MailboxIdentity;
use nestwatch_server::{APP_CSP, SITE_CSP};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

struct Resp {
    status: u16,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

async fn get(srv: &TestServer, method: &str, path: &str, extra: &str) -> Resp {
    let mut s = tokio::net::TcpStream::connect(srv.server.http_addr())
        .await
        .unwrap();
    let req = format!(
        "{method} {path} HTTP/1.1\r\nHost: relay.example\r\n{extra}Connection: close\r\n\r\n"
    );
    s.write_all(req.as_bytes()).await.unwrap();
    let mut raw = Vec::new();
    s.read_to_end(&mut raw).await.unwrap();
    let split = raw.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    let head = String::from_utf8(raw[..split].to_vec()).unwrap();
    let mut lines = head.lines();
    let status = lines
        .next()
        .unwrap()
        .split(' ')
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    let headers = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_owned()))
        .collect();
    Resp {
        status,
        headers,
        body: raw[split + 4..].to_vec(),
    }
}

fn write(dir: &Path, rel: &str, body: &[u8]) {
    let p = dir.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, body).unwrap();
}

#[tokio::test]
async fn static_directories_and_headers() {
    let site = tempfile::tempdir().unwrap();
    let app = tempfile::tempdir().unwrap();
    let downloads = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    write(site.path(), "index.html", b"<h1>Nestwatch</h1>");
    write(site.path(), "style.css", b"h1{}");
    write(app.path(), "index.html", b"<title>listener</title>");
    write(app.path(), "assets/app.wasm", b"\0asm");
    write(app.path(), "assets/app.js", b"export {}");
    write(downloads.path(), "nestwatch.apk", b"PK");
    write(outside.path(), "secret.txt", b"nope");
    #[cfg(unix)]
    std::os::unix::fs::symlink(
        outside.path().join("secret.txt"),
        site.path().join("leak.txt"),
    )
    .unwrap();
    let (s, a, d) = (
        site.path().to_owned(),
        app.path().to_owned(),
        downloads.path().to_owned(),
    );
    let srv = TestServer::start(move |c| {
        c.site_dir = Some(s);
        c.app_dir = Some(a);
        c.downloads_dir = Some(d);
    })
    .await;

    let r = get(&srv, "GET", "/", "").await;
    assert_eq!(r.status, 200);
    assert_eq!(r.body, b"<h1>Nestwatch</h1>");
    assert_eq!(r.headers["content-type"], "text/html; charset=utf-8");
    assert_eq!(r.headers["content-security-policy"], SITE_CSP);
    assert_eq!(r.headers["cache-control"], "public, max-age=300");
    assert_eq!(r.headers["x-content-type-options"], "nosniff");
    assert_eq!(r.headers["x-frame-options"], "DENY");
    assert_eq!(r.headers["referrer-policy"], "no-referrer");
    assert_eq!(
        get(&srv, "GET", "/style.css", "").await.headers["content-type"],
        "text/css; charset=utf-8"
    );

    // The app: strict CSP (connect-src 'self'), no-cache, SPA routes.
    let r = get(&srv, "GET", "/app/", "").await;
    assert_eq!(r.status, 200);
    assert_eq!(r.body, b"<title>listener</title>");
    assert_eq!(r.headers["content-security-policy"], APP_CSP);
    assert!(APP_CSP.contains("connect-src 'self';"));
    assert_eq!(r.headers["cache-control"], "no-cache");
    assert!(r.headers["permissions-policy"].contains("microphone=(self)"));
    let wasm = get(&srv, "GET", "/app/assets/app.wasm", "").await;
    assert_eq!(wasm.headers["content-type"], "application/wasm");
    assert_eq!(wasm.body, b"\0asm");
    assert_eq!(
        get(&srv, "GET", "/app/assets/app.js", "").await.headers["content-type"],
        "text/javascript; charset=utf-8"
    );
    let route = get(&srv, "GET", "/app/station/abc", "").await;
    assert_eq!(
        (route.status, route.body.as_slice()),
        (200, &b"<title>listener</title>"[..])
    );
    assert_eq!(get(&srv, "GET", "/app/missing.js", "").await.status, 404);
    let redirect = get(&srv, "GET", "/app", "").await;
    assert_eq!(redirect.status, 301);
    assert_eq!(redirect.headers["location"], "/app/");

    // Conditional requests.
    let etag = wasm.headers["etag"].clone();
    let r = get(
        &srv,
        "GET",
        "/app/assets/app.wasm",
        &format!("If-None-Match: {etag}\r\n"),
    )
    .await;
    assert_eq!(r.status, 304);
    assert!(r.body.is_empty());

    // Downloads.
    let r = get(&srv, "GET", "/downloads/nestwatch.apk", "").await;
    assert_eq!(r.status, 200);
    assert_eq!(
        r.headers["content-type"],
        "application/vnd.android.package-archive"
    );
    assert_eq!(r.headers["cache-control"], "public, max-age=3600");
    let r = get(&srv, "HEAD", "/downloads/nestwatch.apk", "").await;
    assert_eq!((r.status, r.headers["content-length"].as_str()), (200, "2"));
    assert!(r.body.is_empty());

    // Nothing outside the directories.
    for bad in [
        "/../secret.txt",
        "/%2e%2e/secret.txt",
        "/leak.txt",
        "/downloads/../index.html",
        "/.hidden",
        "/nope.html",
    ] {
        assert_eq!(get(&srv, "GET", bad, "").await.status, 404, "{bad}");
    }
    assert_eq!(
        get(&srv, "POST", "/", "Content-Length: 0\r\n").await.status,
        405
    );
    // A plain GET of the rendezvous path is not an upgrade.
    assert_eq!(get(&srv, "GET", "/nwr/1", "").await.status, 426);
    srv.stop().await;
}

#[tokio::test]
async fn missing_directories_are_404() {
    let srv = TestServer::start(|_| {}).await;
    for p in ["/", "/app/", "/downloads/x.apk"] {
        assert_eq!(get(&srv, "GET", p, "").await.status, 404, "{p}");
    }
    srv.stop().await;
}

#[tokio::test]
async fn healthz_reports_aggregate_counters_only() {
    let srv = TestServer::start(|_| {}).await;
    let identity = MailboxIdentity::generate().unwrap();
    let id_text = identity.id().to_text();
    let reg = srv.registration(identity);
    registered(&reg).await;
    let r = get(&srv, "GET", "/healthz", "").await;
    assert_eq!(r.status, 200);
    assert_eq!(r.headers["content-type"], "application/json");
    assert_eq!(r.headers["cache-control"], "no-store");
    let v: serde_json::Value = serde_json::from_slice(&r.body).unwrap();
    assert_eq!(v["status"], "ok");
    assert_eq!(v["counters"]["mailboxes_online"], 1);
    assert_eq!(v["counters"]["registrations_total"], 1);
    let text = String::from_utf8(r.body).unwrap();
    assert!(!text.contains(&id_text));
    assert!(!text.contains("127.0.0.1"));
    srv.stop().await;
}

#[tokio::test]
async fn cross_origin_websockets_are_refused() {
    let srv = TestServer::start(|c| c.limits.first_message_timeout = 0.2).await;
    let upgrade = "Upgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Version: 13\r\n\
                   Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n";
    let r = get(
        &srv,
        "GET",
        "/nwr/1",
        &format!("{upgrade}Origin: https://evil.example\r\n"),
    )
    .await;
    assert_eq!(r.status, 403);
    let r = get(
        &srv,
        "GET",
        "/nwr/1",
        &format!("{upgrade}Origin: https://relay.example\r\n"),
    )
    .await;
    assert_eq!(r.status, 101);
    srv.stop().await;
}

#[tokio::test]
async fn shutdown_closes_registrations_and_flushes_state() {
    let srv = TestServer::start(|_| {}).await;
    let reg = srv.registration(MailboxIdentity::generate().unwrap());
    registered(&reg).await;
    let mut status = reg.status();
    let state = srv.stop().await;
    // The station notices and starts retrying.
    tokio::time::timeout(
        WAIT,
        status.wait_for(|s| !matches!(s, nestwatch_relay::RegistrationStatus::Registered { .. })),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(
        state
            .path()
            .join(nestwatch_server::store::STATE_FILE)
            .exists()
    );
}
