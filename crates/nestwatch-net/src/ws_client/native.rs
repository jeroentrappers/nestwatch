// SPDX-License-Identifier: AGPL-3.0-or-later
//! Native WebSocket client on tokio-tungstenite.

use std::sync::Arc;
use std::time::Duration;

use nestwatch_proto::Endpoint;
use rustls::RootCertStore;
use rustls::pki_types::{CertificateDer, ServerName};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_rustls::TlsConnector;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

use crate::dial::dial_tcp;
use crate::link::{server_config, ws_link};
use crate::web::StationCert;
use crate::{Link, LinkInfo, LinkKind, NetError};

/// Options for [`connect_with`].
#[derive(Debug, Clone)]
pub struct WsClientConfig {
    /// Time allowed for TCP, TLS and the WebSocket handshake together.
    pub timeout: Duration,
    /// Certificates trusted for `wss://`, typically the station's own
    /// self-signed certificate ([`StationCert::cert_der`]).
    pub trusted: Vec<CertificateDer<'static>>,
}

impl Default for WsClientConfig {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(10),
            trusted: Vec::new(),
        }
    }
}

impl WsClientConfig {
    /// Trusts `cert` for `wss://`.
    pub fn trust(mut self, cert: &StationCert) -> Self {
        self.trusted.push(cert.cert_der().clone());
        self
    }
}

/// Opens a `ws://` or `wss://` URL with `config` and returns a
/// [`Messages`](crate::LinkKind::Messages) link.
pub async fn connect_with(url: &str, config: &WsClientConfig) -> Result<Link, NetError> {
    tokio::time::timeout(config.timeout, connect_inner(url, config))
        .await
        .map_err(|_| NetError::Timeout)?
}

async fn connect_inner(url: &str, config: &WsClientConfig) -> Result<Link, NetError> {
    let bad = |m: &str| NetError::InvalidUrl(format!("{url}: {m}"));
    let request = url.into_client_request().map_err(|e| bad(&e.to_string()))?;
    let uri = request.uri();
    let tls = match uri.scheme_str() {
        Some("ws") => false,
        Some("wss") => true,
        _ => return Err(bad("scheme must be ws or wss")),
    };
    let host = uri.host().ok_or_else(|| bad("no host"))?.to_owned();
    let port = uri.port_u16().unwrap_or(if tls { 443 } else { 80 });
    let endpoint = Endpoint::parse(&format!("{host}:{port}")).map_err(|e| bad(&e.to_string()))?;
    let stream = dial_tcp(&endpoint, config.timeout).await?;
    let info = LinkInfo {
        kind: LinkKind::Messages,
        peer: stream.peer_addr().ok(),
        local: stream.local_addr().ok(),
        tls,
    };
    if !tls {
        return handshake(request, stream, info).await;
    }
    let mut roots = RootCertStore::empty();
    for c in &config.trusted {
        roots
            .add(c.clone())
            .map_err(|e| NetError::Tls(e.to_string()))?;
    }
    let tls_config = rustls::ClientConfig::builder_with_provider(crate::web::provider())
        .with_safe_default_protocol_versions()
        .map_err(|e| NetError::Tls(e.to_string()))?
        .with_root_certificates(roots)
        .with_no_client_auth();
    let name = host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_owned();
    let name = ServerName::try_from(name).map_err(|e| bad(&e.to_string()))?;
    let stream = TlsConnector::from(Arc::new(tls_config))
        .connect(name, stream)
        .await
        .map_err(|e| NetError::Tls(e.to_string()))?;
    handshake(request, stream, info).await
}

async fn handshake<S>(
    request: tokio_tungstenite::tungstenite::handshake::client::Request,
    stream: S,
    info: LinkInfo,
) -> Result<Link, NetError>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (ws, _resp) =
        tokio_tungstenite::client_async_with_config(request, stream, Some(server_config()))
            .await
            .map_err(|e| NetError::WebSocket(e.to_string()))?;
    Ok(ws_link(ws, info))
}
