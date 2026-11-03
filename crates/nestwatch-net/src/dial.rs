// SPDX-License-Identifier: AGPL-3.0-or-later
//! Connecting to station endpoints over TCP (protocol.md §3.2, §3.5).

use std::io;
use std::net::SocketAddr;
use std::time::Duration;

use futures_util::StreamExt;
use futures_util::stream::FuturesUnordered;
use nestwatch_proto::{Endpoint, Host};
use tokio::net::TcpStream;

use crate::{Link, NetError, StreamLink};

/// Delay before racing the next address (RFC 8305 §5 "Connection Attempt Delay").
const ATTEMPT_DELAY: Duration = Duration::from_millis(250);

/// Resolves an endpoint to socket addresses (DNS or literal).
pub async fn resolve(endpoint: &Endpoint) -> Result<Vec<SocketAddr>, NetError> {
    let addrs: Vec<SocketAddr> = match &endpoint.host {
        Host::Dns(name) => tokio::net::lookup_host((name.as_str(), endpoint.port))
            .await?
            .collect(),
        h => h
            .ip()
            .map(|ip| SocketAddr::new(ip, endpoint.port))
            .into_iter()
            .collect(),
    };
    if addrs.is_empty() {
        return Err(NetError::NoEndpoints);
    }
    Ok(addrs)
}

/// Interleaves address families, starting with the first one returned
/// (RFC 8305 §4).
fn interleave(addrs: Vec<SocketAddr>) -> Vec<SocketAddr> {
    let first_v6 = addrs.first().is_some_and(SocketAddr::is_ipv6);
    let (mut a, mut b): (Vec<_>, Vec<_>) = addrs.into_iter().partition(|s| s.is_ipv6() == first_v6);
    a.reverse();
    b.reverse();
    let mut out = Vec::with_capacity(a.len() + b.len());
    while !a.is_empty() || !b.is_empty() {
        out.extend(a.pop());
        out.extend(b.pop());
    }
    out
}

/// Races connection attempts, starting a new one every 250 ms or as soon
/// as the previous one fails; the first success wins.
async fn connect_addrs(addrs: Vec<SocketAddr>) -> io::Result<TcpStream> {
    let mut queue = interleave(addrs).into_iter();
    let mut pending = FuturesUnordered::new();
    let mut last_err = None;
    loop {
        if pending.is_empty() {
            match queue.next() {
                Some(a) => pending.push(TcpStream::connect(a)),
                None => {
                    return Err(last_err
                        .unwrap_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no address")));
                }
            }
        }
        let more = queue.len() > 0;
        tokio::select! {
            Some(res) = pending.next() => match res {
                Ok(s) => return Ok(s),
                Err(e) => {
                    last_err = Some(e);
                    if let Some(a) = queue.next() {
                        pending.push(TcpStream::connect(a));
                    }
                }
            },
            () = tokio::time::sleep(ATTEMPT_DELAY), if more => {
                if let Some(a) = queue.next() {
                    pending.push(TcpStream::connect(a));
                }
            }
        }
    }
}

/// Opens a TCP connection to `endpoint` within `timeout` (resolution
/// included), trying every resolved address.
pub async fn dial_tcp(endpoint: &Endpoint, timeout: Duration) -> Result<TcpStream, NetError> {
    let attempt = async {
        let addrs = resolve(endpoint).await?;
        Ok::<_, NetError>(connect_addrs(addrs).await?)
    };
    let stream = tokio::time::timeout(timeout, attempt)
        .await
        .map_err(|_| NetError::Timeout)??;
    let _ = stream.set_nodelay(true);
    Ok(stream)
}

/// Connects to `endpoint` and returns a [`Stream`](crate::LinkKind::Stream)
/// link with `TCP_NODELAY` set.
pub async fn dial(endpoint: &Endpoint, timeout: Duration) -> Result<Link, NetError> {
    Ok(Link::Stream(StreamLink::new(
        dial_tcp(endpoint, timeout).await?,
    )))
}

/// Tries `endpoints` in the given order (the caller applies FR-NET-3
/// ordering) with `timeout` per endpoint and returns the first link with
/// the endpoint that worked, or the last error.
pub async fn dial_first(
    endpoints: &[Endpoint],
    timeout: Duration,
) -> Result<(Link, Endpoint), NetError> {
    let mut last = NetError::NoEndpoints;
    for e in endpoints {
        match dial(e, timeout).await {
            Ok(link) => return Ok((link, e.clone())),
            Err(err) => last = err,
        }
    }
    Err(last)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interleaves_families() {
        let s = |t: &str| t.parse::<SocketAddr>().unwrap();
        let got = interleave(vec![
            s("[::1]:1"),
            s("[::2]:1"),
            s("[::3]:1"),
            s("10.0.0.1:1"),
        ]);
        assert_eq!(
            got,
            [s("[::1]:1"), s("10.0.0.1:1"), s("[::2]:1"), s("[::3]:1")]
        );
    }
}
