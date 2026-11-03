// SPDX-License-Identifier: AGPL-3.0-or-later
//! WebSocket client (protocol.md §3.3).
//!
//! In the browser this uses `web-sys` and is how the web listener reaches
//! its station. On native targets it uses tokio-tungstenite (for tests and
//! the CLI) and can trust a station's self-signed certificate.

#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(not(target_arch = "wasm32"))]
pub use native::{WsClientConfig, connect_with};

use crate::{Link, NetError};

/// Opens a `ws://` or `wss://` URL and returns a
/// [`Messages`](crate::LinkKind::Messages) link.
///
/// In the browser, TLS trust is the browser's; on native targets this uses
/// [`WsClientConfig::default`] (no trusted certificates, so `wss://` needs
/// [`connect_with`]).
pub async fn connect(url: &str) -> Result<Link, NetError> {
    #[cfg(target_arch = "wasm32")]
    {
        crate::link::open(url).await
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        connect_with(url, &WsClientConfig::default()).await
    }
}
