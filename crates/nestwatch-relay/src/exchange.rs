// SPDX-License-Identifier: AGPL-3.0-or-later
//! The NWR/1 exchanges that turn a rendezvous WebSocket into a relay pipe
//! (rendezvous.md §4.2), over any [`Link`] so the browser shares them.
// Without a client (codec-only builds) nothing calls these.
#![cfg_attr(
    not(any(feature = "native", all(feature = "web", target_arch = "wasm32"))),
    allow(dead_code)
)]

use std::future::Future;
use std::time::Duration;

use nestwatch_net::Link;

use crate::candidate::{Candidate, candidate_strings, parse_candidates};
use crate::msg::{Message, NONCE_LEN};
use crate::{MailboxId, RelayError};

/// What the rendezvous tells one side about the other.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PeerInfo {
    /// The peer's direct candidates that this version understands (§5).
    pub candidates: Vec<Candidate>,
    /// The peer's address as the server saw it, if the server said
    /// (OPEN-QUESTIONS RELAY-6). Informational only.
    pub observed: Option<String>,
}

impl PeerInfo {
    pub(crate) fn new(candidates: &[String], observed: String) -> Self {
        Self {
            candidates: parse_candidates(candidates),
            observed: (!observed.is_empty()).then_some(observed),
        }
    }
}

/// Runs `f` with a deadline on the current platform.
pub(crate) async fn with_timeout<T>(
    d: Duration,
    f: impl Future<Output = Result<T, RelayError>>,
) -> Result<T, RelayError> {
    #[cfg(feature = "native")]
    {
        tokio::time::timeout(d, f)
            .await
            .map_err(|_| RelayError::Timeout)?
    }
    #[cfg(all(not(feature = "native"), feature = "web"))]
    {
        use futures_util::future::{Either, select};
        let timer = gloo_timers::future::sleep(d);
        let f = std::pin::pin!(f);
        match select(f, timer).await {
            Either::Left((r, _)) => r,
            Either::Right(_) => Err(RelayError::Timeout),
        }
    }
    #[cfg(not(any(feature = "native", feature = "web")))]
    {
        let _ = d;
        f.await
    }
}

pub(crate) async fn send(link: &mut Link, m: &Message) -> Result<(), RelayError> {
    link.send(&m.encode()?).await?;
    Ok(())
}

/// The next known message; unknown types are skipped (§7.1 convention).
pub(crate) async fn recv(link: &mut Link) -> Result<Message, RelayError> {
    loop {
        let Some(bytes) = link.recv().await? else {
            return Err(RelayError::Closed);
        };
        match Message::decode(&bytes)? {
            Message::Unknown { .. } => continue,
            m => return Ok(m),
        }
    }
}

pub(crate) fn random_nonce() -> [u8; NONCE_LEN] {
    let mut n = [0; NONCE_LEN];
    // The server only checks the length (RELAY-5).
    let _ = getrandom::fill(&mut n);
    n
}

/// Listener: `connect`, then wait for `accepted` (§4.2). On success the link
/// is a relay pipe.
pub(crate) async fn listener_connect(
    link: &mut Link,
    mailbox: MailboxId,
    candidates: &[Candidate],
    timeout: Duration,
) -> Result<PeerInfo, RelayError> {
    let connect = Message::Connect {
        mailbox,
        nonce: random_nonce(),
        candidates: candidate_strings(candidates),
    };
    with_timeout(timeout, send(link, &connect)).await?;
    // The station has up to 30 s to accept; allow a little more.
    let wait = timeout.max(Duration::from_secs(35));
    with_timeout(wait, async {
        match recv(link).await? {
            Message::Accepted {
                candidates,
                observed,
            } => Ok(PeerInfo::new(&candidates, observed)),
            Message::Error { code } => Err(RelayError::Server(code)),
            other => Err(RelayError::Protocol(format!(
                "unexpected {:?} before accepted",
                other.message_type()
            ))),
        }
    })
    .await
}

#[cfg(feature = "native")]
/// Station: `accept` on a fresh connection (§4.2). The server does not
/// answer; the link is a relay pipe from now on (if the session had expired
/// the server sends `error` and closes, which the NWP/1 session then sees as
/// a failed preface).
pub(crate) async fn station_accept(
    link: &mut Link,
    session: crate::msg::SessionId,
    candidates: &[Candidate],
    timeout: Duration,
) -> Result<(), RelayError> {
    let accept = Message::Accept {
        session,
        candidates: candidate_strings(candidates),
    };
    with_timeout(timeout, send(link, &accept)).await
}
