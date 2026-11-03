// SPDX-License-Identifier: AGPL-3.0-or-later
//! Station certificate generation and persistence (protocol.md §3.3).

use nestwatch_net::NetError;
use nestwatch_net::web::StationCert;
use nestwatch_proto::Host;

fn names() -> Vec<Host> {
    vec![
        Host::Dns("nursery.local".into()),
        Host::Dns("nursery.tail1234.ts.net".into()),
        Host::parse("192.168.1.20").unwrap(),
        Host::parse("[fd7a:115c:a1e0::7]").unwrap(),
    ]
}

#[test]
fn fingerprint_format() {
    let cert = StationCert::generate(&names()).unwrap();
    let fp = cert.fingerprint();
    assert_eq!(fp.len(), 32 * 3 - 1);
    let pairs: Vec<&str> = fp.split(':').collect();
    assert_eq!(pairs.len(), 32);
    for p in pairs {
        assert_eq!(p.len(), 2);
        assert!(
            p.bytes()
                .all(|b| b.is_ascii_digit() || (b'A'..=b'F').contains(&b))
        );
    }
    assert_eq!(
        nestwatch_net::web::format_fingerprint(&[0xab; 32]),
        ["AB"; 32].join(":")
    );
    assert!(format!("{cert:?}").contains(&fp));
    assert!(!format!("{cert:?}").contains("PRIVATE"));
}

#[test]
fn pem_round_trip() {
    let cert = StationCert::generate(&names()).unwrap();
    assert!(cert.cert_pem().starts_with("-----BEGIN CERTIFICATE-----"));
    assert!(cert.key_pem().starts_with("-----BEGIN PRIVATE KEY-----"));
    let loaded = StationCert::from_pem(cert.cert_pem(), cert.key_pem()).unwrap();
    assert_eq!(loaded.fingerprint(), cert.fingerprint());
    assert_eq!(loaded.cert_der(), cert.cert_der());

    // Each generation is a new key and certificate.
    let other = StationCert::generate(&names()).unwrap();
    assert_ne!(other.fingerprint(), cert.fingerprint());
    // A key from another certificate is rejected.
    assert!(matches!(
        StationCert::from_pem(cert.cert_pem(), other.key_pem()),
        Err(NetError::Certificate(_))
    ));
    assert!(StationCert::from_pem("garbage", cert.key_pem()).is_err());
    assert!(StationCert::from_pem(cert.cert_pem(), "garbage").is_err());
}

#[test]
fn certificate_contents() {
    let cert = StationCert::generate(&names()).unwrap();
    let der = cert.cert_der().as_ref();
    let has = |needle: &[u8]| der.windows(needle.len()).any(|w| w == needle);
    // ecdsa-with-SHA256 and the P-256 curve OIDs.
    assert!(has(&[
        0x06, 0x08, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x04, 0x03, 0x02
    ]));
    assert!(has(&[
        0x06, 0x08, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07
    ]));
    // SANs: DNS names and the IP addresses as raw octets.
    assert!(has(b"nursery.tail1234.ts.net"));
    assert!(has(&[0x87, 0x04, 192, 168, 1, 20]));
    assert!(has(&[0x87, 0x10, 0xfd, 0x7a, 0x11, 0x5c, 0xa1, 0xe0]));
    // Validity: notAfter is ten years after notBefore (+1 day back-dating).
    let years = generalized_or_utc_years(der);
    assert_eq!(years.len(), 2);
    assert!((10..=11).contains(&(years[1] - years[0])), "{years:?}");
}

/// Extracts the years of the two validity times (UTCTime or GeneralizedTime).
fn generalized_or_utc_years(der: &[u8]) -> Vec<i32> {
    let mut out = Vec::new();
    let mut i = 0;
    while i + 2 < der.len() && out.len() < 2 {
        let (tag, len) = (der[i], der[i + 1] as usize);
        let digits = |s: &[u8]| s.iter().all(u8::is_ascii_digit);
        if tag == 0x17 && len == 13 && der[i + 14] == b'Z' && digits(&der[i + 2..i + 14]) {
            let yy: i32 = std::str::from_utf8(&der[i + 2..i + 4])
                .unwrap()
                .parse()
                .unwrap();
            out.push(if yy < 50 { 2000 + yy } else { 1900 + yy });
            i += 15;
        } else if tag == 0x18 && len == 15 && der[i + 16] == b'Z' && digits(&der[i + 2..i + 16]) {
            out.push(
                std::str::from_utf8(&der[i + 2..i + 6])
                    .unwrap()
                    .parse()
                    .unwrap(),
            );
            i += 17;
        } else {
            i += 1;
        }
    }
    out
}
