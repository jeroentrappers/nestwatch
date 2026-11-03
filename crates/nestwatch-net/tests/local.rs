// SPDX-License-Identifier: AGPL-3.0-or-later
//! Local endpoint enumeration (protocol.md §3.5).

use std::time::Duration;

use nestwatch_net::local::{interface_addrs, station_endpoints, tailscale_dns_name};
use nestwatch_proto::Host;

#[tokio::test]
async fn endpoints_follow_section_3_5() {
    assert!(!interface_addrs().unwrap().is_empty());
    let eps = station_endpoints(47100, Duration::from_secs(2)).await;
    for e in &eps {
        assert_eq!(e.port, 47100);
        if let Some(ip) = e.host.ip() {
            assert!(!ip.is_loopback() && !ip.is_unspecified(), "{e}");
        }
    }
    // A Tailscale name, when present, comes first.
    if let Some(name) = tailscale_dns_name(Duration::from_secs(2)).await {
        assert!(matches!(name, Host::Dns(_)));
        assert_eq!(eps.first().map(|e| &e.host), Some(&name));
    }
}
