// SPDX-License-Identifier: AGPL-3.0-or-later
//! Local interface addresses and the Tailscale name, for the station's
//! endpoint list (protocol.md §3.5).

use std::io;
use std::process::Stdio;
use std::time::Duration;

use lull_proto::endpoint::{InterfaceAddr, order_endpoints};
use lull_proto::{Endpoint, Host};

/// Default time allowed for `tailscale status --json`.
pub const TAILSCALE_TIMEOUT: Duration = Duration::from_secs(2);

/// Every interface address with its interface name, unfiltered
/// ([`order_endpoints`] drops what must not be listed).
///
/// On Android this is unsupported (`getifaddrs` needs API 24); apps pass
/// the interface addresses they get from the platform.
#[cfg(not(target_os = "android"))]
pub fn interface_addrs() -> io::Result<Vec<InterfaceAddr>> {
    Ok(if_addrs::get_if_addrs()?
        .into_iter()
        .map(|i| InterfaceAddr::new(i.name.clone(), i.ip()))
        .collect())
}

/// Every interface address (unsupported on Android, see above).
#[cfg(target_os = "android")]
pub fn interface_addrs() -> io::Result<Vec<InterfaceAddr>> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "interfaces come from the app on Android",
    ))
}

/// Places the Tailscale CLI may be installed besides `PATH`.
const TAILSCALE_BINARIES: [&str; 2] = [
    "tailscale",
    "/Applications/Tailscale.app/Contents/MacOS/Tailscale",
];

/// Extracts the MagicDNS name of this node from `tailscale status --json`
/// output, if Tailscale is running.
pub fn parse_tailscale_status(json: &[u8]) -> Option<Host> {
    let v: serde_json::Value = serde_json::from_slice(json).ok()?;
    if let Some(state) = v.get("BackendState").and_then(|s| s.as_str())
        && state != "Running"
    {
        return None;
    }
    let name = v.get("Self")?.get("DNSName")?.as_str()?;
    match Host::parse(name).ok()? {
        h @ Host::Dns(_) => Some(h),
        _ => None,
    }
}

/// Asks the Tailscale CLI for this node's MagicDNS name. Missing binary,
/// errors and timeouts all yield `None`.
pub async fn tailscale_dns_name(timeout: Duration) -> Option<Host> {
    for bin in TAILSCALE_BINARIES {
        let child = tokio::process::Command::new(bin)
            .args(["status", "--json"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn();
        let Ok(child) = child else { continue };
        let out = tokio::time::timeout(timeout, child.wait_with_output()).await;
        return match out {
            Ok(Ok(o)) if o.status.success() => parse_tailscale_status(&o.stdout),
            _ => None,
        };
    }
    None
}

/// The station's endpoints for `port` in protocol.md §3.5 order: the
/// Tailscale name (if any), VPN addresses, then LAN addresses.
pub async fn station_endpoints(port: u16, tailscale_timeout: Duration) -> Vec<Endpoint> {
    let names: Vec<Host> = tailscale_dns_name(tailscale_timeout)
        .await
        .into_iter()
        .collect();
    let addrs = interface_addrs().unwrap_or_default();
    order_endpoints(&names, &addrs, port)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tailscale_json() {
        let ok = br#"{"BackendState":"Running","Self":{"DNSName":"nursery.tail1234.ts.net.","Online":true}}"#;
        assert_eq!(
            parse_tailscale_status(ok),
            Some(Host::Dns("nursery.tail1234.ts.net".into()))
        );
        let stopped = br#"{"BackendState":"Stopped","Self":{"DNSName":"n.ts.net."}}"#;
        assert_eq!(parse_tailscale_status(stopped), None);
        assert_eq!(parse_tailscale_status(br#"{"Self":{"DNSName":""}}"#), None);
        assert_eq!(parse_tailscale_status(b"not json"), None);
    }

    #[test]
    fn lists_interfaces() {
        // Every host has at least a loopback address.
        assert!(!interface_addrs().unwrap().is_empty());
    }
}
