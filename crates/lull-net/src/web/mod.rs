// SPDX-License-Identifier: AGPL-3.0-or-later
//! HTTP and HTTPS server for the browser listener (protocol.md §3.3).
//!
//! Serves the app's static files with strict security headers (PRIV-1: the
//! app is self-contained, nothing is loaded from elsewhere) and accepts
//! WebSocket upgrades at `/lull/1`, which become
//! [`Messages`](crate::LinkKind::Messages) links returned by
//! [`WebServer::accept`].

mod assets;
mod cert;

use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use http_body_util::Full;
use hyper::body::{Bytes, Incoming};
use hyper::header::{self, HeaderMap, HeaderValue};
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::{TokioIo, TokioTimer};
use lull_proto::endpoint::{DEFAULT_HTTP_PORT, DEFAULT_HTTPS_PORT, WEBSOCKET_PATH};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpListener;
use tokio::sync::{Semaphore, mpsc};
use tokio::task::{JoinHandle, JoinSet};
use tokio_rustls::TlsAcceptor;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::handshake::derive_accept_key;
use tokio_tungstenite::tungstenite::protocol::Role;

pub use assets::AssetSource;
pub(crate) use cert::provider;
pub use cert::{StationCert, VALIDITY_DAYS, format_fingerprint};

use crate::link::{canonical, server_config, ws_link};
use crate::listen::{ListenConfig, accept_any, bind_with_fallback};
use crate::{Link, LinkInfo, LinkKind, NetError};

/// Time allowed for a TLS handshake or for request headers to arrive.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// Configuration of the [`WebServer`].
#[derive(Debug, Clone)]
pub struct WebConfig {
    /// Plain HTTP; default port 47180 with fallbacks. `None` disables it.
    pub http: Option<ListenConfig>,
    /// HTTPS; default port 47181 with fallbacks. `None` disables it.
    pub https: Option<ListenConfig>,
    /// `Content-Security-Policy` override. `None` uses [`default_csp`].
    pub csp: Option<String>,
    /// Reject WebSocket upgrades whose `Origin` header names a different
    /// host than the request (cross-site WebSocket hijacking). Requests
    /// without `Origin` (native clients) are accepted.
    pub check_origin: bool,
    /// Concurrent HTTP connections at most; further ones wait.
    pub max_connections: usize,
    /// Upgraded links queued for [`WebServer::accept`] at most.
    pub link_queue: usize,
}

impl Default for WebConfig {
    fn default() -> Self {
        Self {
            http: Some(ListenConfig::new(DEFAULT_HTTP_PORT)),
            https: Some(ListenConfig::new(DEFAULT_HTTPS_PORT)),
            csp: None,
            check_origin: true,
            max_connections: 64,
            link_queue: 16,
        }
    }
}

/// The default `Content-Security-Policy`: everything from the station's own
/// origin only, WebAssembly allowed, no framing, no plugins, no forms.
/// `host` (the request's `Host`) adds explicit `ws:`/`wss:` sources for
/// browsers whose `'self'` does not cover WebSocket schemes.
pub fn default_csp(host: Option<&str>) -> String {
    let ws = host
        .map(|h| format!(" ws://{h} wss://{h}"))
        .unwrap_or_default();
    format!(
        "default-src 'none'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self'; \
         img-src 'self' data: blob:; font-src 'self'; media-src 'self' blob:; \
         connect-src 'self'{ws}; worker-src 'self' blob:; manifest-src 'self'; \
         base-uri 'none'; form-action 'none'; frame-ancestors 'none'"
    )
}

struct Ctx {
    assets: AssetSource,
    csp: Option<String>,
    check_origin: bool,
    links: mpsc::Sender<Link>,
}

/// A running web server; dropping it stops it (links already accepted
/// stay open).
#[derive(Debug)]
pub struct WebServer {
    http_port: Option<u16>,
    https_port: Option<u16>,
    links: mpsc::Receiver<Link>,
    tasks: Vec<JoinHandle<()>>,
}

impl Drop for WebServer {
    fn drop(&mut self) {
        for t in &self.tasks {
            t.abort();
        }
    }
}

impl WebServer {
    /// Binds the configured ports (HTTP first, then HTTPS, each falling back
    /// upward) and starts serving on the current tokio runtime. `cert` is
    /// required when HTTPS is enabled.
    pub async fn start(
        config: WebConfig,
        assets: AssetSource,
        cert: Option<&StationCert>,
    ) -> Result<Self, NetError> {
        let tls = match (&config.https, cert) {
            (Some(_), Some(c)) => Some(TlsAcceptor::from(Arc::new(c.server_config()?))),
            (Some(_), None) => {
                return Err(NetError::Certificate("HTTPS needs a certificate".into()));
            }
            (None, _) => None,
        };
        let (tx, rx) = mpsc::channel(config.link_queue.max(1));
        let ctx = Arc::new(Ctx {
            assets,
            csp: config.csp.clone(),
            check_origin: config.check_origin,
            links: tx,
        });
        let limit = Arc::new(Semaphore::new(config.max_connections.max(1)));
        let mut server = Self {
            http_port: None,
            https_port: None,
            links: rx,
            tasks: Vec::new(),
        };
        if let Some(c) = config.http {
            let (ls, port) = bind_with_fallback(c)?;
            server.http_port = Some(port);
            server
                .tasks
                .push(tokio::spawn(serve(ls, None, ctx.clone(), limit.clone())));
        }
        if let (Some(c), Some(tls)) = (config.https, tls) {
            let (ls, port) = bind_with_fallback(c)?;
            server.https_port = Some(port);
            server
                .tasks
                .push(tokio::spawn(serve(ls, Some(tls), ctx, limit)));
        }
        Ok(server)
    }

    /// Bound HTTP port, if enabled (advertised as TXT `http`, §3.4).
    pub fn http_port(&self) -> Option<u16> {
        self.http_port
    }

    /// Bound HTTPS port, if enabled (advertised as TXT `https`, §3.4).
    pub fn https_port(&self) -> Option<u16> {
        self.https_port
    }

    /// Next upgraded WebSocket link from either port. Cancel-safe.
    pub async fn accept(&mut self) -> Option<Link> {
        self.links.recv().await
    }
}

async fn serve(
    listeners: Vec<TcpListener>,
    tls: Option<TlsAcceptor>,
    ctx: Arc<Ctx>,
    limit: Arc<Semaphore>,
) {
    // Connection tasks live in this set, so stopping the server stops them.
    let mut conns = JoinSet::new();
    let mut next = 0;
    loop {
        while conns.try_join_next().is_some() {}
        let Ok(permit) = limit.clone().acquire_owned().await else {
            return;
        };
        let (stream, peer) = match accept_any(&listeners, &mut next).await {
            Ok(s) => s,
            Err(_) => {
                // Typically EMFILE; back off instead of spinning.
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
        };
        let _ = stream.set_nodelay(true);
        let local = stream.local_addr().ok().map(canonical);
        let ctx = ctx.clone();
        let tls = tls.clone();
        conns.spawn(async move {
            let _permit = permit;
            let info = LinkInfo {
                kind: LinkKind::Messages,
                peer: Some(canonical(peer)),
                local,
                tls: tls.is_some(),
            };
            match tls {
                None => serve_conn(stream, ctx, info).await,
                Some(acceptor) => {
                    if let Ok(Ok(s)) =
                        tokio::time::timeout(HANDSHAKE_TIMEOUT, acceptor.accept(stream)).await
                    {
                        serve_conn(s, ctx, info).await;
                    }
                }
            }
        });
    }
}

async fn serve_conn<S>(io: S, ctx: Arc<Ctx>, info: LinkInfo)
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let svc = hyper::service::service_fn(move |req| {
        let ctx = ctx.clone();
        let info = info.clone();
        async move { Ok::<_, Infallible>(handle(req, &ctx, info).await) }
    });
    let _ = hyper::server::conn::http1::Builder::new()
        .timer(TokioTimer::new())
        .header_read_timeout(HANDSHAKE_TIMEOUT)
        .serve_connection(TokioIo::new(io), svc)
        .with_upgrades()
        .await;
}

type Resp = Response<Full<Bytes>>;

/// The `Host` header if it only contains host-name, address and port
/// characters (it is echoed into the CSP).
fn request_host(headers: &HeaderMap) -> Option<&str> {
    let h = headers.get(header::HOST)?.to_str().ok()?;
    let ok = !h.is_empty()
        && h.len() <= 262
        && h.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-._:[]".contains(&b));
    ok.then_some(h)
}

fn simple(status: StatusCode, ctx: &Ctx, host: Option<&str>) -> Resp {
    let mut r = Response::new(Full::new(Bytes::from(
        status.canonical_reason().unwrap_or(""),
    )));
    *r.status_mut() = status;
    r.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    secure(r, ctx, host)
}

/// Adds the security headers every response carries.
fn secure(mut r: Resp, ctx: &Ctx, host: Option<&str>) -> Resp {
    let csp = ctx.csp.clone().unwrap_or_else(|| default_csp(host));
    let h = r.headers_mut();
    if let Ok(v) = HeaderValue::from_str(&csp) {
        h.insert(header::CONTENT_SECURITY_POLICY, v);
    }
    let fixed = [
        (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        (header::X_FRAME_OPTIONS, "DENY"),
        (header::REFERRER_POLICY, "no-referrer"),
        (header::CACHE_CONTROL, "no-cache"),
    ];
    for (k, v) in fixed {
        h.insert(k, HeaderValue::from_static(v));
    }
    let named = [
        ("cross-origin-opener-policy", "same-origin"),
        ("cross-origin-resource-policy", "same-origin"),
        (
            "permissions-policy",
            "camera=(), geolocation=(), microphone=(self), payment=(), usb=()",
        ),
    ];
    for (k, v) in named {
        h.insert(k, HeaderValue::from_static(v));
    }
    r
}

fn has_token(headers: &HeaderMap, name: header::HeaderName, token: &str) -> bool {
    headers.get_all(name).iter().any(|v| {
        v.to_str()
            .is_ok_and(|s| s.split(',').any(|t| t.trim().eq_ignore_ascii_case(token)))
    })
}

/// Whether `Origin` (if present) names the same host and port as `Host`.
fn origin_ok(headers: &HeaderMap, host: Option<&str>) -> bool {
    let Some(origin) = headers.get(header::ORIGIN) else {
        return true;
    };
    let (Ok(origin), Some(host)) = (origin.to_str(), host) else {
        return false;
    };
    origin
        .split_once("://")
        .is_some_and(|(_, rest)| rest.eq_ignore_ascii_case(host))
}

async fn handle(mut req: Request<Incoming>, ctx: &Arc<Ctx>, info: LinkInfo) -> Resp {
    let host = request_host(req.headers()).map(str::to_owned);
    let host = host.as_deref();
    if req.uri().path() == WEBSOCKET_PATH {
        return upgrade(&mut req, ctx, info, host);
    }
    if req.method() != Method::GET && req.method() != Method::HEAD {
        let mut r = simple(StatusCode::METHOD_NOT_ALLOWED, ctx, host);
        r.headers_mut()
            .insert(header::ALLOW, HeaderValue::from_static("GET, HEAD"));
        return r;
    }
    let Some(rel) = assets::sanitize(req.uri().path()) else {
        return simple(StatusCode::NOT_FOUND, ctx, host);
    };
    let mut found = ctx.assets.load(&rel).await.map(|b| (rel.clone(), b));
    // Paths without an extension are app routes: serve the app.
    let is_route = !rel.rsplit('/').next().unwrap_or("").contains('.');
    if found.is_none() && is_route {
        found = ctx
            .assets
            .load("index.html")
            .await
            .map(|b| ("index.html".to_owned(), b));
    }
    let Some((path, body)) = found else {
        return simple(StatusCode::NOT_FOUND, ctx, host);
    };
    let len = body.len();
    let body = if req.method() == Method::HEAD {
        Bytes::new()
    } else {
        body
    };
    let mut r = Response::new(Full::new(body));
    let h = r.headers_mut();
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(assets::mime_type(&path)),
    );
    h.insert(header::CONTENT_LENGTH, HeaderValue::from(len));
    secure(r, ctx, host)
}

fn upgrade(
    req: &mut Request<Incoming>,
    ctx: &Arc<Ctx>,
    info: LinkInfo,
    host: Option<&str>,
) -> Resp {
    let headers = req.headers();
    if req.method() != Method::GET {
        return simple(StatusCode::METHOD_NOT_ALLOWED, ctx, host);
    }
    if !has_token(headers, header::UPGRADE, "websocket")
        || !has_token(headers, header::CONNECTION, "upgrade")
    {
        let mut r = simple(StatusCode::UPGRADE_REQUIRED, ctx, host);
        r.headers_mut()
            .insert(header::UPGRADE, HeaderValue::from_static("websocket"));
        return r;
    }
    if headers
        .get(header::SEC_WEBSOCKET_VERSION)
        .is_none_or(|v| v.as_bytes() != b"13")
    {
        let mut r = simple(StatusCode::UPGRADE_REQUIRED, ctx, host);
        r.headers_mut().insert(
            header::SEC_WEBSOCKET_VERSION,
            HeaderValue::from_static("13"),
        );
        return r;
    }
    let Some(key) = headers.get(header::SEC_WEBSOCKET_KEY) else {
        return simple(StatusCode::BAD_REQUEST, ctx, host);
    };
    if ctx.check_origin && !origin_ok(headers, host) {
        return simple(StatusCode::FORBIDDEN, ctx, host);
    }
    let accept = derive_accept_key(key.as_bytes());
    let on_upgrade = hyper::upgrade::on(req);
    let links = ctx.links.clone();
    tokio::spawn(async move {
        let Ok(upgraded) = on_upgrade.await else {
            return;
        };
        let ws = WebSocketStream::from_raw_socket(
            TokioIo::new(upgraded),
            Role::Server,
            Some(server_config()),
        )
        .await;
        // If the server is gone the link is dropped, which closes it.
        let _ = links.send(ws_link(ws, info)).await;
    });
    let mut r = Response::new(Full::new(Bytes::new()));
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
    fn origin_matching() {
        let mut h = HeaderMap::new();
        assert!(origin_ok(&h, Some("a:1")));
        h.insert(header::ORIGIN, HeaderValue::from_static("http://A:1"));
        assert!(origin_ok(&h, Some("a:1")));
        assert!(!origin_ok(&h, Some("a:2")));
        assert!(!origin_ok(&h, None));
        h.insert(header::ORIGIN, HeaderValue::from_static("null"));
        assert!(!origin_ok(&h, Some("a:1")));
    }

    #[test]
    fn host_validation() {
        let mut h = HeaderMap::new();
        h.insert(header::HOST, HeaderValue::from_static("[fd7a::1]:47180"));
        assert_eq!(request_host(&h), Some("[fd7a::1]:47180"));
        h.insert(header::HOST, HeaderValue::from_static("x; script-src *"));
        assert_eq!(request_host(&h), None);
    }
}
