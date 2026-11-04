// SPDX-License-Identifier: AGPL-3.0-or-later
//! The HTTP side (rendezvous.md §6, §8): `/nwr/1` WebSocket upgrades,
//! `/healthz`, and the static directories, on a plain local port behind the
//! host's TLS-terminating reverse proxy.

use std::convert::Infallible;
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Empty, Full};
use hyper::body::Incoming;
use hyper::header::{self, HeaderMap, HeaderValue};
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::{TokioIo, TokioTimer};
use nestwatch_relay::RENDEZVOUS_PATH;
use tokio::net::TcpListener;
use tokio::sync::{Semaphore, watch};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::handshake::derive_accept_key;
use tokio_tungstenite::tungstenite::protocol::{Role, WebSocketConfig};

use crate::files;
use crate::hub::{Client, Hub};
use crate::tasks::Tasks;

/// Response body type.
pub(crate) type Body = BoxBody<Bytes, io::Error>;
type Resp = Response<Body>;

/// Time allowed for request headers.
const HEADER_TIMEOUT: Duration = Duration::from_secs(10);
/// Time connections get to finish when the server stops.
const DRAIN_TIMEOUT: Duration = Duration::from_secs(5);

/// CSP of the browser listener (FR-RV-5, PRIV-1): everything from this
/// origin only; the rendezvous WebSocket is same-origin (`connect-src
/// 'self'`, OPEN-QUESTIONS RELAY-10).
pub const APP_CSP: &str = "default-src 'none'; script-src 'self' 'wasm-unsafe-eval'; \
    style-src 'self'; img-src 'self' data: blob:; font-src 'self'; media-src 'self' blob:; \
    connect-src 'self'; worker-src 'self' blob:; manifest-src 'self'; base-uri 'none'; \
    form-action 'none'; frame-ancestors 'none'";
/// CSP of the landing page and downloads: same-origin resources, no
/// connections, no framing.
pub const SITE_CSP: &str = "default-src 'self'; img-src 'self' data:; object-src 'none'; \
    connect-src 'none'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";

fn full(b: impl Into<Bytes>) -> Body {
    Full::new(b.into()).map_err(|never| match never {}).boxed()
}

fn empty() -> Body {
    Empty::new().map_err(|never| match never {}).boxed()
}

/// The WebSocket configuration of every `/nwr/1` connection.
pub(crate) fn ws_config(max: usize) -> WebSocketConfig {
    WebSocketConfig::default()
        .max_message_size(Some(max))
        .max_frame_size(Some(max))
        .write_buffer_size(0)
        .max_write_buffer_size(4 * max.max(1024))
}

/// Where a static request goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Area {
    Site,
    App,
    Downloads,
}

/// Accepts HTTP connections until `shutdown`, then lets open requests
/// finish for a few seconds.
pub(crate) async fn serve(
    listener: TcpListener,
    hub: Arc<Hub>,
    tasks: Tasks,
    mut shutdown: watch::Receiver<bool>,
) {
    let limit = Arc::new(Semaphore::new(hub.cfg.limits.max_connections.max(1)));
    loop {
        let permit = tokio::select! {
            p = limit.clone().acquire_owned() => match p { Ok(p) => p, Err(_) => return },
            () = crate::tasks::stopped(&mut shutdown) => return,
        };
        let accepted = tokio::select! {
            a = listener.accept() => a,
            () = crate::tasks::stopped(&mut shutdown) => return,
        };
        let (stream, peer) = match accepted {
            Ok(x) => x,
            Err(_) => {
                // Typically EMFILE; back off instead of spinning.
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
        };
        let _ = stream.set_nodelay(true);
        let hub = hub.clone();
        let conn_tasks = tasks.clone();
        let mut stop = shutdown.clone();
        tasks.spawn(async move {
            let _permit = permit;
            let svc = hyper::service::service_fn(move |req| {
                let hub = hub.clone();
                let tasks = conn_tasks.clone();
                async move { Ok::<_, Infallible>(handle(req, peer, &hub, &tasks).await) }
            });
            let conn = hyper::server::conn::http1::Builder::new()
                .timer(TokioTimer::new())
                .header_read_timeout(HEADER_TIMEOUT)
                .serve_connection(TokioIo::new(stream), svc)
                .with_upgrades();
            tokio::pin!(conn);
            tokio::select! {
                _ = conn.as_mut() => {}
                () = crate::tasks::stopped(&mut stop) => {
                    conn.as_mut().graceful_shutdown();
                    let _ = tokio::time::timeout(DRAIN_TIMEOUT, conn).await;
                }
            }
        });
    }
}

fn last_forwarded(headers: &HeaderMap) -> Option<IpAddr> {
    if let Some(v) = headers.get("x-real-ip").and_then(|v| v.to_str().ok())
        && let Ok(ip) = v.trim().parse()
    {
        return Some(ip);
    }
    // The last entry is the one our own proxy appended.
    let v = headers.get_all("x-forwarded-for").iter().next_back()?;
    v.to_str().ok()?.rsplit(',').next()?.trim().parse().ok()
}

/// The client address: the TCP peer, or the proxy's forwarded address when
/// the peer is a trusted proxy (`trust_proxy`, `trusted_proxies`).
fn client(hub: &Hub, headers: &HeaderMap, peer: SocketAddr) -> Client {
    let peer = SocketAddr::new(peer.ip().to_canonical(), peer.port());
    if hub.cfg.trust_proxy
        && hub
            .cfg
            .trusted_proxies
            .iter()
            .any(|n| n.contains(&peer.ip()))
        && let Some(ip) = last_forwarded(headers)
    {
        let ip = ip.to_canonical();
        return Client {
            ip,
            observed: ip.to_string(),
        };
    }
    Client {
        ip: peer.ip(),
        observed: peer.to_string(),
    }
}

/// The `Host` header if it only holds host-name, address and port bytes.
fn request_host(headers: &HeaderMap) -> Option<&str> {
    let h = headers.get(header::HOST)?.to_str().ok()?;
    let ok = !h.is_empty()
        && h.len() <= 262
        && h.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-._:[]".contains(&b));
    ok.then_some(h)
}

/// Whether `Origin` (if any) names the request's `Host` (NET-4 rule).
fn origin_ok(headers: &HeaderMap) -> bool {
    let Some(origin) = headers.get(header::ORIGIN) else {
        return true;
    };
    let (Ok(origin), Some(host)) = (origin.to_str(), request_host(headers)) else {
        return false;
    };
    origin
        .split_once("://")
        .is_some_and(|(_, rest)| rest.eq_ignore_ascii_case(host))
}

fn secure(mut r: Resp, area: Option<Area>) -> Resp {
    let h = r.headers_mut();
    let csp = if area == Some(Area::App) {
        APP_CSP
    } else {
        SITE_CSP
    };
    h.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(csp),
    );
    let permissions = if area == Some(Area::App) {
        "camera=(), geolocation=(), microphone=(self), payment=(), usb=()"
    } else {
        "camera=(), geolocation=(), microphone=(), payment=(), usb=()"
    };
    let fixed = [
        (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        (header::X_FRAME_OPTIONS, "DENY"),
        (header::REFERRER_POLICY, "no-referrer"),
    ];
    for (k, v) in fixed {
        h.insert(k, HeaderValue::from_static(v));
    }
    let named = [
        ("cross-origin-opener-policy", "same-origin"),
        ("cross-origin-resource-policy", "same-origin"),
        ("permissions-policy", permissions),
    ];
    for (k, v) in named {
        h.insert(k, HeaderValue::from_static(v));
    }
    r
}

fn simple(status: StatusCode) -> Resp {
    let mut r = Response::new(full(status.canonical_reason().unwrap_or("")));
    *r.status_mut() = status;
    r.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    r.headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    secure(r, None)
}

async fn handle(req: Request<Incoming>, peer: SocketAddr, hub: &Arc<Hub>, tasks: &Tasks) -> Resp {
    let path = req.uri().path().to_owned();
    if path == RENDEZVOUS_PATH {
        return upgrade(req, peer, hub, tasks);
    }
    if req.method() != Method::GET && req.method() != Method::HEAD {
        let mut r = simple(StatusCode::METHOD_NOT_ALLOWED);
        r.headers_mut()
            .insert(header::ALLOW, HeaderValue::from_static("GET, HEAD"));
        return r;
    }
    if path == "/healthz" {
        let body = serde_json::json!({
            "status": "ok",
            "version": env!("CARGO_PKG_VERSION"),
            "counters": hub.metrics.snapshot(),
        });
        let mut r = Response::new(full(body.to_string()));
        let h = r.headers_mut();
        h.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );
        h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
        return secure(r, None);
    }
    let cfg = &hub.cfg;
    // Without a landing page, the front door is the browser app.
    let app_only = path == "/" && cfg.site_dir.is_none() && cfg.app_dir.is_some();
    if path == "/app" || app_only {
        let status = if app_only {
            StatusCode::FOUND
        } else {
            StatusCode::MOVED_PERMANENTLY
        };
        let mut r = simple(status);
        r.headers_mut()
            .insert(header::LOCATION, HeaderValue::from_static("/app/"));
        return r;
    }
    let (area, root, rest) = if let Some(rest) = path.strip_prefix("/app/") {
        (Area::App, cfg.app_dir.as_deref(), rest)
    } else if let Some(rest) = path.strip_prefix("/downloads/") {
        (Area::Downloads, cfg.downloads_dir.as_deref(), rest)
    } else {
        (Area::Site, cfg.site_dir.as_deref(), path.as_str())
    };
    let head = req.method() == Method::HEAD;
    match root {
        Some(root) => static_file(&req, root, rest, area, head).await,
        None => simple(StatusCode::NOT_FOUND),
    }
}

async fn static_file(
    req: &Request<Incoming>,
    root: &Path,
    rest: &str,
    area: Area,
    head: bool,
) -> Resp {
    let Some(rel) = files::sanitize(rest) else {
        return simple(StatusCode::NOT_FOUND);
    };
    let mut found = files::open(root, &rel).await;
    // Extensionless paths in the app are client-side routes.
    let is_route = !rel.rsplit('/').next().unwrap_or("").contains('.');
    if found.is_none() && area == Area::App && is_route {
        found = files::open(root, "index.html").await;
    }
    let Some(f) = found else {
        return simple(StatusCode::NOT_FOUND);
    };
    let cache = match area {
        // The app must update as soon as it is redeployed.
        Area::App => "no-cache",
        Area::Site => "public, max-age=300",
        Area::Downloads => "public, max-age=3600",
    };
    let not_modified = req
        .headers()
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.split(',').any(|t| t.trim() == f.etag || t.trim() == "*"));
    let (status, body) = if not_modified {
        (StatusCode::NOT_MODIFIED, empty())
    } else if head {
        (StatusCode::OK, empty())
    } else {
        (StatusCode::OK, files::body(f.file))
    };
    let mut r = Response::new(body);
    *r.status_mut() = status;
    let h = r.headers_mut();
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(files::mime_type(&f.rel)),
    );
    if status == StatusCode::OK {
        h.insert(header::CONTENT_LENGTH, HeaderValue::from(f.len));
    }
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    if let Ok(v) = HeaderValue::from_str(&f.etag) {
        h.insert(header::ETAG, v);
    }
    secure(r, Some(area))
}

fn has_token(headers: &HeaderMap, name: header::HeaderName, token: &str) -> bool {
    headers.get_all(name).iter().any(|v| {
        v.to_str()
            .is_ok_and(|s| s.split(',').any(|t| t.trim().eq_ignore_ascii_case(token)))
    })
}

fn upgrade(mut req: Request<Incoming>, peer: SocketAddr, hub: &Arc<Hub>, tasks: &Tasks) -> Resp {
    let headers = req.headers();
    if req.method() != Method::GET {
        return simple(StatusCode::METHOD_NOT_ALLOWED);
    }
    if !has_token(headers, header::UPGRADE, "websocket")
        || !has_token(headers, header::CONNECTION, "upgrade")
        || headers
            .get(header::SEC_WEBSOCKET_VERSION)
            .is_none_or(|v| v.as_bytes() != b"13")
    {
        let mut r = simple(StatusCode::UPGRADE_REQUIRED);
        let h = r.headers_mut();
        h.insert(header::UPGRADE, HeaderValue::from_static("websocket"));
        h.insert(
            header::SEC_WEBSOCKET_VERSION,
            HeaderValue::from_static("13"),
        );
        return r;
    }
    let Some(key) = headers.get(header::SEC_WEBSOCKET_KEY) else {
        return simple(StatusCode::BAD_REQUEST);
    };
    if hub.cfg.check_origin && !origin_ok(headers) {
        return simple(StatusCode::FORBIDDEN);
    }
    let client = client(hub, headers, peer);
    let Some(slot) = hub.conn_slot(client.ip) else {
        return simple(StatusCode::TOO_MANY_REQUESTS);
    };
    let accept = derive_accept_key(key.as_bytes());
    let on_upgrade = hyper::upgrade::on(&mut req);
    let hub = hub.clone();
    let max = hub.cfg.limits.max_message_bytes;
    tasks.spawn(async move {
        let Ok(upgraded) = on_upgrade.await else {
            return;
        };
        let ws = WebSocketStream::from_raw_socket(
            TokioIo::new(upgraded),
            Role::Server,
            Some(ws_config(max)),
        )
        .await;
        hub.serve(ws, client, slot).await;
    });
    let mut r = Response::new(empty());
    *r.status_mut() = StatusCode::SWITCHING_PROTOCOLS;
    let h = r.headers_mut();
    h.insert(header::UPGRADE, HeaderValue::from_static("websocket"));
    h.insert(header::CONNECTION, HeaderValue::from_static("Upgrade"));
    if let Ok(v) = HeaderValue::from_str(&accept) {
        h.insert(header::SEC_WEBSOCKET_ACCEPT, v);
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forwarded_addresses() {
        let mut h = HeaderMap::new();
        assert_eq!(last_forwarded(&h), None);
        h.insert(
            "x-forwarded-for",
            HeaderValue::from_static("10.9.9.9, 203.0.113.4"),
        );
        assert_eq!(last_forwarded(&h), Some("203.0.113.4".parse().unwrap()));
        h.insert("x-real-ip", HeaderValue::from_static("2001:db8::7"));
        assert_eq!(last_forwarded(&h), Some("2001:db8::7".parse().unwrap()));
    }

    #[test]
    fn origin_matching() {
        let mut h = HeaderMap::new();
        assert!(origin_ok(&h));
        h.insert(header::HOST, HeaderValue::from_static("relay.example"));
        h.insert(
            header::ORIGIN,
            HeaderValue::from_static("https://relay.example"),
        );
        assert!(origin_ok(&h));
        h.insert(
            header::ORIGIN,
            HeaderValue::from_static("https://evil.example"),
        );
        assert!(!origin_ok(&h));
    }

    #[test]
    fn app_csp_only_connects_to_self() {
        assert!(APP_CSP.contains("connect-src 'self';"));
        assert!(!APP_CSP.contains("ws:") && !APP_CSP.contains('*'));
        assert!(SITE_CSP.contains("connect-src 'none'"));
    }
}
