// SPDX-License-Identifier: AGPL-3.0-or-later
//! The station's self-signed HTTPS certificate (protocol.md §3.3,
//! security.md §7).

use std::fmt;
use std::sync::Arc;

use lull_proto::Host;
use rcgen::{
    CertificateParams, DistinguishedName, DnType, KeyPair, PKCS_ECDSA_P256_SHA256, SanType,
};
use rustls::crypto::CryptoProvider;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::sign::CertifiedKey;
use sha2::{Digest, Sha256};
use time::{Duration, OffsetDateTime};

use crate::NetError;

/// Validity of a generated certificate (protocol.md §3.3: 10 years).
pub const VALIDITY_DAYS: i64 = 3653;

/// Common name of generated certificates.
const COMMON_NAME: &str = "Lull station";

/// A self-signed ECDSA P-256 certificate and its private key.
///
/// Generate it once with [`generate`](Self::generate), store
/// [`cert_pem`](Self::cert_pem) and [`key_pem`](Self::key_pem) in the
/// station's key storage, and reload it with [`from_pem`](Self::from_pem).
#[derive(Clone)]
pub struct StationCert {
    cert_pem: String,
    key_pem: String,
    cert_der: CertificateDer<'static>,
}

impl fmt::Debug for StationCert {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StationCert")
            .field("fingerprint", &self.fingerprint())
            .finish_non_exhaustive()
    }
}

fn cert_err(e: impl fmt::Display) -> NetError {
    NetError::Certificate(e.to_string())
}

pub(crate) fn provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

impl StationCert {
    /// Generates a new certificate valid from now for 10 years, with the
    /// given names and addresses as subject alternative names.
    pub fn generate(names: &[Host]) -> Result<Self, NetError> {
        let mut params = CertificateParams::default();
        let mut dn = DistinguishedName::new();
        dn.push(DnType::CommonName, COMMON_NAME);
        params.distinguished_name = dn;
        for h in names {
            let san = match h {
                Host::Dns(n) => SanType::DnsName(n.clone().try_into().map_err(cert_err)?),
                other => SanType::IpAddress(other.ip().ok_or_else(|| cert_err("bad host"))?),
            };
            if !params.subject_alt_names.contains(&san) {
                params.subject_alt_names.push(san);
            }
        }
        let now = OffsetDateTime::now_utc();
        // Back-date slightly so clients with a skewed clock accept it.
        params.not_before = now - Duration::days(1);
        params.not_after = now + Duration::days(VALIDITY_DAYS);
        let key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).map_err(cert_err)?;
        let cert = params.self_signed(&key).map_err(cert_err)?;
        Ok(Self {
            cert_pem: cert.pem(),
            key_pem: key.serialize_pem(),
            cert_der: cert.der().clone(),
        })
    }

    /// Loads a stored certificate and key, checking that they belong together.
    pub fn from_pem(cert_pem: &str, key_pem: &str) -> Result<Self, NetError> {
        let cert_der = CertificateDer::from_pem_slice(cert_pem.as_bytes()).map_err(cert_err)?;
        let key = PrivateKeyDer::from_pem_slice(key_pem.as_bytes()).map_err(cert_err)?;
        let signing = provider()
            .key_provider
            .load_private_key(key)
            .map_err(cert_err)?;
        CertifiedKey::new(vec![cert_der.clone()], signing)
            .keys_match()
            .map_err(cert_err)?;
        Ok(Self {
            cert_pem: cert_pem.to_owned(),
            key_pem: key_pem.to_owned(),
            cert_der,
        })
    }

    /// The certificate in PEM form, for storage.
    pub fn cert_pem(&self) -> &str {
        &self.cert_pem
    }

    /// The private key in PKCS#8 PEM form, for secure storage only.
    pub fn key_pem(&self) -> &str {
        &self.key_pem
    }

    /// The certificate in DER form (for clients that pin or trust it).
    pub fn cert_der(&self) -> &CertificateDer<'static> {
        &self.cert_der
    }

    /// SHA-256 of the DER certificate.
    pub fn fingerprint_bytes(&self) -> [u8; 32] {
        Sha256::digest(self.cert_der.as_ref()).into()
    }

    /// SHA-256 fingerprint as upper-case hex pairs separated by colons
    /// (`AB:CD:…`, 95 characters), as browsers display it.
    pub fn fingerprint(&self) -> String {
        format_fingerprint(&self.fingerprint_bytes())
    }

    pub(crate) fn server_config(&self) -> Result<rustls::ServerConfig, NetError> {
        let key = PrivateKeyDer::from_pem_slice(self.key_pem.as_bytes()).map_err(cert_err)?;
        let mut cfg = rustls::ServerConfig::builder_with_provider(provider())
            .with_safe_default_protocol_versions()
            .map_err(|e| NetError::Tls(e.to_string()))?
            .with_no_client_auth()
            .with_single_cert(vec![self.cert_der.clone()], key)
            .map_err(|e| NetError::Tls(e.to_string()))?;
        cfg.alpn_protocols = vec![b"http/1.1".to_vec()];
        Ok(cfg)
    }
}

/// Formats 32 bytes as `AB:CD:…`.
pub fn format_fingerprint(bytes: &[u8; 32]) -> String {
    bytes
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}
