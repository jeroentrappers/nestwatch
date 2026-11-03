// SPDX-License-Identifier: AGPL-3.0-or-later
//! Async transports for Lull (`specs/architecture.md` §2, `specs/protocol.md` §3).
//!
//! This crate only moves bytes: it opens, accepts and closes connections,
//! serves the browser listener app and runs mDNS. Framing, the preface,
//! Noise and everything above live in `lull-session`, which consumes the
//! [`Link`]s produced here.
//!
//! On native targets everything runs on tokio. On `wasm32` only the
//! WebSocket client ([`ws_client::connect`]) is available; it is driven by
//! the browser event loop (for example through `wasm-bindgen-futures`).

mod error;
mod link;

pub use error::NetError;
pub use link::{
    Link, LinkInfo, LinkKind, LinkReader, LinkWriter, MAX_MESSAGE_LEN, MessageLink, MessageReader,
    MessageWriter, WS_MAX_MESSAGE_SIZE,
};

#[cfg(not(target_arch = "wasm32"))]
pub mod dial;
#[cfg(not(target_arch = "wasm32"))]
pub mod listen;
#[cfg(not(target_arch = "wasm32"))]
pub mod local;
#[cfg(not(target_arch = "wasm32"))]
pub mod mdns;
#[cfg(not(target_arch = "wasm32"))]
pub mod web;
pub mod ws_client;

#[cfg(not(target_arch = "wasm32"))]
pub use link::{StreamLink, StreamReader, StreamWriter};
#[cfg(not(target_arch = "wasm32"))]
pub use listen::{BindScope, ListenConfig, StationListener};
