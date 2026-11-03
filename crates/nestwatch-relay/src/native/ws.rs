// SPDX-License-Identifier: AGPL-3.0-or-later
//! Opening the rendezvous WebSocket on native targets: TCP through
//! nestwatch-net's dialer, TLS with the Web PKI roots, tokio-tungstenite.

use std::sync::Arc;

use nestwatch_net::{LinkInfo, LinkKind, NetError, websocket_config};
use nestwatch_proto::Endpoint;
use rustls::RootCertStore;
use rustls::pki_types::ServerName;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_rustls::TlsConnector;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

use crate::{RelayConfig, RelayError};

/// Any byte stream the WebSocket runs on (TCP or TLS over TCP).
pub(crate) trait Io: AsyncRead + AsyncWrite + Unpin + Send + 'static {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send + 'static> Io for T {}

pub(crate) type Ws = WebSocketStream<Box<dyn Io>>;

fn tls_config(config: &RelayConfig) -> Result<rustls::ClientConfig, RelayError> {
    let mut roots = RootCertStore::empty();
    if config.webpki_roots {
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    }
    for c in &config.trusted {
        roots
            .add(c.clone())
            .map_err(|e| NetError::Tls(e.to_string()))?;
    }
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut tls = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| NetError::Tls(e.to_string()))?
        .with_root_certificates(roots)
        .with_no_client_auth();
    tls.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(tls)
}

/// Opens `config.server.ws_url()` within `config.timeout`.
pub(crate) async fn open(config: &RelayConfig) -> Result<(Ws, LinkInfo), RelayError> {
    tokio::time::timeout(config.timeout, open_inner(config))
        .await
        .map_err(|_| RelayError::Timeout)?
}

async fn open_inner(config: &RelayConfig) -> Result<(Ws, LinkInfo), RelayError> {
    let url = config.server.ws_url();
    let bad = |m: String| RelayError::InvalidServer(format!("{url}: {m}"));
    let request = url.into_client_request().map_err(|e| bad(e.to_string()))?;
    let uri = request.uri();
    let tls = uri.scheme_str() == Some("wss");
    let host = uri.host().ok_or_else(|| bad("no host".into()))?.to_owned();
    let port = uri.port_u16().unwrap_or(if tls { 443 } else { 80 });
    let endpoint = Endpoint::parse(&format!("{host}:{port}")).map_err(|e| bad(e.to_string()))?;
    let tcp = nestwatch_net::dial::dial_tcp(&endpoint, config.timeout).await?;
    let info = LinkInfo {
        kind: LinkKind::Messages,
        peer: tcp.peer_addr().ok(),
        local: tcp.local_addr().ok(),
        tls,
    };
    let stream: Box<dyn Io> = if tls {
        let name = host
            .trim_start_matches('[')
            .trim_end_matches(']')
            .to_owned();
        let name = ServerName::try_from(name).map_err(|e| bad(e.to_string()))?;
        let s = TlsConnector::from(Arc::new(tls_config(config)?))
            .connect(name, tcp)
            .await
            .map_err(|e| NetError::Tls(e.to_string()))?;
        Box::new(s)
    } else {
        Box::new(tcp)
    };
    let (ws, _resp) =
        tokio_tungstenite::client_async_with_config(request, stream, Some(websocket_config()))
            .await
            .map_err(|e| NetError::WebSocket(e.to_string()))?;
    Ok((ws, info))
}
