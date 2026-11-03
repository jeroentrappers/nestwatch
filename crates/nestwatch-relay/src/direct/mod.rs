// SPDX-License-Identifier: AGPL-3.0-or-later
//! Hole punching and the direct QUIC transport (rendezvous.md §5.2).
//!
//! Both sides use one UDP socket for everything: the STUN Binding that
//! found their server-reflexive address ([`crate::stun::stun_binding`]), the
//! probes that open the NAT mappings, and QUIC. The station is the QUIC
//! server, the listener the client; one bidirectional stream carries the
//! NWP/1 byte stream framed exactly as over TCP (protocol.md §3.1), exposed
//! as a [`Stream`](nestwatch_net::LinkKind::Stream) [`Link`].
//!
//! # Security
//!
//! QUIC's TLS here authenticates **nothing**: the station presents a
//! throw-away self-signed certificate generated per call, and the listener
//! accepts any certificate. That is deliberate (§5.2 item 2). The devices
//! authenticate each other with the NWP/1 Noise handshake that runs inside
//! the stream and binds the session to their static keys; a party that
//! intercepts the QUIC connection can only fail that handshake. Never use
//! this transport for anything that is not itself authenticated end to end.

mod socket;

use std::collections::HashSet;
use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use futures_util::StreamExt;
use futures_util::stream::FuturesUnordered;
use nestwatch_net::{Link, LinkInfo, LinkKind, StreamLink};
use quinn::crypto::rustls::{QuicClientConfig, QuicServerConfig};
use quinn::{Connection, Endpoint, EndpointConfig, RecvStream, SendStream, TransportConfig};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::CryptoProvider;
use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::UdpSocket;
use tokio::time::Instant;

use crate::candidate::Candidate;
use socket::{DemuxSocket, ProbeKind};

/// ALPN of the direct transport.
pub const ALPN: &[u8] = b"nwp/1";
/// Server name the listener puts in its QUIC ClientHello (not verified).
const SERVER_NAME: &str = "nestwatch.invalid";

/// Which end of the direct path this is (§5.2: the station is the QUIC
/// server).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PunchRole {
    /// QUIC server; accepts the listener's connection and stream.
    Station,
    /// QUIC client; connects to the first candidate that answers a probe.
    Listener,
}

/// Timing of [`punch_direct_with`] (defaults from §5.2).
#[derive(Debug, Clone)]
pub struct PunchConfig {
    /// Probe interval (100 ms).
    pub probe_interval: Duration,
    /// How long to probe (5 s).
    pub probe_duration: Duration,
    /// Time allowed after probing ends for a QUIC handshake (and, on the
    /// station, for the listener's stream to appear).
    pub handshake_timeout: Duration,
    /// QUIC keep-alive interval, so NAT mappings stay open.
    pub keep_alive: Duration,
    /// QUIC idle timeout.
    pub idle_timeout: Duration,
}

impl Default for PunchConfig {
    fn default() -> Self {
        Self {
            probe_interval: Duration::from_millis(100),
            probe_duration: Duration::from_secs(5),
            handshake_timeout: Duration::from_secs(5),
            keep_alive: Duration::from_secs(5),
            idle_timeout: Duration::from_secs(30),
        }
    }
}

/// Why no direct path formed.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum PunchError {
    /// The peer sent no `udp:` candidate reachable from this socket.
    #[error("no usable UDP candidates")]
    NoCandidates,
    /// No probe came back and no QUIC connection completed in time.
    #[error("hole punching timed out")]
    Timeout,
    /// Socket or QUIC setup failure.
    #[error("direct path setup failed: {0}")]
    Setup(String),
    /// The QUIC connection failed after it was established.
    #[error("QUIC connection failed: {0}")]
    Connection(String),
}

fn setup<E: std::fmt::Display>(e: E) -> PunchError {
    PunchError::Setup(e.to_string())
}

/// Binds a UDP socket for STUN, probes and QUIC. `0.0.0.0:0` is the usual
/// choice (IPv4 only); `[::]:0` gives a dual-stack socket where available.
pub async fn bind_punch_socket(addr: SocketAddr) -> io::Result<UdpSocket> {
    UdpSocket::bind(addr).await
}

/// Punches through to `peer` and returns the direct link, or `None` if no
/// path formed in time (the session then stays relayed, §5.2 item 5). See
/// [`punch_direct_with`].
pub async fn punch_direct(socket: UdpSocket, peer: &[Candidate], role: PunchRole) -> Option<Link> {
    punch_direct_with(socket, peer, role, &PunchConfig::default())
        .await
        .ok()
}

/// Hole punching and QUIC on `socket` (rendezvous.md §5.2):
///
/// 1. both sides probe every `udp:` candidate of the peer every 100 ms for
///    up to 5 s and answer every probe they receive;
/// 2. the listener starts a QUIC connection to every address a probe or an
///    answer came from (the peer's candidates or its NAT's mapping); the
///    first that completes wins;
/// 3. the station accepts the first QUIC connection and its first
///    bidirectional stream.
///
/// The listener's stream reaches the station only once the listener sends
/// on it, which NWP/1 does at once (the preface). The socket is consumed;
/// the QUIC endpoint lives as long as the returned link.
pub async fn punch_direct_with(
    socket: UdpSocket,
    peer: &[Candidate],
    role: PunchRole,
    config: &PunchConfig,
) -> Result<Link, PunchError> {
    let std_socket = socket.into_std().map_err(setup)?;
    let runtime: Arc<dyn quinn::Runtime> = Arc::new(quinn::TokioRuntime);
    let inner = runtime.wrap_udp_socket(std_socket).map_err(setup)?;
    let (demux, mut probes) = DemuxSocket::new(inner).map_err(setup)?;
    let local = demux.local_addr_canonical();
    let targets: Vec<SocketAddr> = peer
        .iter()
        .filter_map(Candidate::udp)
        .filter(|a| demux.target(*a).is_some())
        .collect();
    if targets.is_empty() {
        return Err(PunchError::NoCandidates);
    }
    let transport = Arc::new(transport_config(config));
    let server = match role {
        PunchRole::Station => Some(server_config(transport.clone())?),
        PunchRole::Listener => None,
    };
    let endpoint = Endpoint::new_with_abstract_socket(
        EndpointConfig::default(),
        server,
        demux.clone() as Arc<dyn quinn::AsyncUdpSocket>,
        runtime,
    )
    .map_err(setup)?;
    let client = match role {
        PunchRole::Listener => Some(client_config(transport)?),
        PunchRole::Station => None,
    };

    let start = Instant::now();
    let probe_until = start + config.probe_duration;
    let give_up = probe_until + config.handshake_timeout;
    let mut tick = tokio::time::interval(config.probe_interval);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut tried: HashSet<SocketAddr> = HashSet::new();
    let mut attempts = FuturesUnordered::new();
    let is_station = role == PunchRole::Station;

    let conn: Connection = {
        let accept = accept_first(&endpoint, config.handshake_timeout);
        tokio::pin!(accept);
        loop {
            tokio::select! {
                _ = tick.tick(), if Instant::now() < probe_until => {
                    for t in &targets {
                        demux.send_probe(*t, ProbeKind::Probe);
                    }
                }
                Some((from, kind)) = probes.recv() => {
                    if kind == ProbeKind::Probe {
                        demux.send_probe(from, ProbeKind::Ack);
                    }
                    if let Some(client) = &client
                        && tried.insert(from)
                        && let Some(to) = demux.target(from)
                        && let Ok(connecting) = endpoint.connect_with(client.clone(), to, SERVER_NAME)
                    {
                        let limit = config.handshake_timeout;
                        attempts.push(async move {
                            tokio::time::timeout(limit, connecting).await
                        });
                    }
                }
                Some(r) = attempts.next(), if !attempts.is_empty() => {
                    if let Ok(Ok(c)) = r {
                        break c;
                    }
                }
                c = &mut accept, if is_station => match c {
                    Some(c) => break c,
                    None => return Err(PunchError::Timeout),
                },
                () = tokio::time::sleep_until(give_up) => return Err(PunchError::Timeout),
            }
        }
    };
    // Losing attempts are dropped (and abandoned) here.
    drop(attempts);

    let (send, recv) = match role {
        PunchRole::Listener => conn
            .open_bi()
            .await
            .map_err(|e| PunchError::Connection(e.to_string()))?,
        PunchRole::Station => tokio::time::timeout(config.handshake_timeout, conn.accept_bi())
            .await
            .map_err(|_| PunchError::Timeout)?
            .map_err(|e| PunchError::Connection(e.to_string()))?,
    };
    let info = LinkInfo {
        kind: LinkKind::Stream,
        peer: Some(canonical(conn.remote_address())),
        local,
        // QUIC encrypts, but authenticates nothing here (see module docs);
        // `tls` stays false so nothing treats this link as authenticated.
        tls: false,
    };
    let holder = Arc::new(Holder {
        _endpoint: endpoint,
        conn,
    });
    let reader = QuicReader {
        stream: recv,
        _holder: holder.clone(),
    };
    let writer = QuicWriter {
        stream: Some(send),
        holder,
    };
    Ok(Link::Stream(StreamLink::from_io(reader, writer, info)))
}

fn canonical(a: SocketAddr) -> SocketAddr {
    SocketAddr::new(a.ip().to_canonical(), a.port())
}

impl DemuxSocket {
    fn local_addr_canonical(&self) -> Option<SocketAddr> {
        quinn::AsyncUdpSocket::local_addr(self).ok().map(canonical)
    }
}

/// The first incoming connection whose handshake completes (station).
async fn accept_first(endpoint: &Endpoint, limit: Duration) -> Option<Connection> {
    while let Some(incoming) = endpoint.accept().await {
        if let Ok(Ok(c)) = tokio::time::timeout(limit, incoming).await {
            return Some(c);
        }
    }
    None
}

fn transport_config(config: &PunchConfig) -> TransportConfig {
    let mut t = TransportConfig::default();
    t.keep_alive_interval(Some(config.keep_alive));
    if let Ok(idle) = config.idle_timeout.try_into() {
        t.max_idle_timeout(Some(idle));
    }
    t.max_concurrent_bidi_streams(1u8.into());
    t.max_concurrent_uni_streams(0u8.into());
    t
}

fn provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

/// A throw-away self-signed certificate and the QUIC server configuration.
fn server_config(transport: Arc<TransportConfig>) -> Result<quinn::ServerConfig, PunchError> {
    let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).map_err(setup)?;
    let params = rcgen::CertificateParams::new(vec![SERVER_NAME.to_owned()]).map_err(setup)?;
    let cert = params.self_signed(&key).map_err(setup)?;
    let cert_der = CertificateDer::from(cert.der().to_vec());
    let key_der = PrivatePkcs8KeyDer::from(key.serialize_der());
    let mut tls = rustls::ServerConfig::builder_with_provider(provider())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(setup)?
        .with_no_client_auth()
        .with_single_cert(vec![cert_der], key_der.into())
        .map_err(setup)?;
    tls.alpn_protocols = vec![ALPN.to_vec()];
    let crypto = QuicServerConfig::try_from(tls).map_err(setup)?;
    let mut server = quinn::ServerConfig::with_crypto(Arc::new(crypto));
    server.transport_config(transport);
    Ok(server)
}

fn client_config(transport: Arc<TransportConfig>) -> Result<quinn::ClientConfig, PunchError> {
    let provider = provider();
    let mut tls = rustls::ClientConfig::builder_with_provider(provider.clone())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(setup)?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(AnyServerCert(provider)))
        .with_no_client_auth();
    tls.alpn_protocols = vec![ALPN.to_vec()];
    let crypto = QuicClientConfig::try_from(tls).map_err(setup)?;
    let mut client = quinn::ClientConfig::new(Arc::new(crypto));
    client.transport_config(transport);
    Ok(client)
}

/// Accepts any server certificate (see the module's security notes). The
/// TLS 1.3 handshake signature is still checked against the presented key,
/// which keeps the TLS state machine honest but proves no identity.
#[derive(Debug)]
struct AnyServerCert(Arc<CryptoProvider>);

impl ServerCertVerifier for AnyServerCert {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        // QUIC is TLS 1.3 only.
        Err(rustls::Error::PeerIncompatible(
            rustls::PeerIncompatible::Tls12NotOffered,
        ))
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

/// Keeps the endpoint and connection alive as long as either half of the
/// link exists.
#[derive(Debug)]
struct Holder {
    _endpoint: Endpoint,
    conn: Connection,
}

struct QuicReader {
    stream: RecvStream,
    _holder: Arc<Holder>,
}

impl AsyncRead for QuicReader {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_read(cx, buf)
    }
}

struct QuicWriter {
    stream: Option<SendStream>,
    holder: Arc<Holder>,
}

impl QuicWriter {
    fn stream(&mut self) -> io::Result<&mut SendStream> {
        self.stream
            .as_mut()
            .ok_or_else(|| io::Error::from(io::ErrorKind::NotConnected))
    }
}

impl AsyncWrite for QuicWriter {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        AsyncWrite::poll_write(Pin::new(self.stream()?), cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        AsyncWrite::poll_flush(Pin::new(self.stream()?), cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        AsyncWrite::poll_shutdown(Pin::new(self.stream()?), cx)
    }
}

/// Time a dropped writer waits for the peer to receive the last bytes
/// before the connection may close.
const LINGER: Duration = Duration::from_secs(2);

impl Drop for QuicWriter {
    fn drop(&mut self) {
        // Dropping the last connection handle closes the connection at once
        // and may lose bytes still in flight (such as an NWP/1 `close`), so
        // finish the stream and keep the connection until the peer has read
        // it or `LINGER` passes.
        let Some(mut stream) = self.stream.take() else {
            return;
        };
        let _ = stream.finish();
        let holder = self.holder.clone();
        if let Ok(rt) = tokio::runtime::Handle::try_current() {
            rt.spawn(async move {
                let _ = tokio::time::timeout(LINGER, stream.stopped()).await;
                holder.conn.close(0u8.into(), b"done");
            });
        }
    }
}
