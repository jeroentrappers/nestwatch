// SPDX-License-Identifier: AGPL-3.0-or-later
//! Rendezvous, hole punching and relay client for Nestwatch (NWR/1,
//! `specs/rendezvous.md`).
//!
//! The relay server never sees content: NWP/1 (protocol.md) runs end to end
//! inside the relay pipe or the direct QUIC stream, and its Noise handshake
//! authenticates both devices. This crate only finds and opens the path:
//!
//! - [`msg`]: the NWR/1 messages (§4) and their CBOR codec;
//! - [`MailboxIdentity`]: the station's mailbox ID and secret (§3.2);
//! - [`stun`]: STUN Binding for the server-reflexive address (§5.1);
//! - [`StationRegistration`]: the station's control connection (§4.1),
//!   yielding one relayed [`nestwatch_net::Link`] per listener, natively and,
//!   with the `web` feature, in the browser (a relay-only station);
//! - [`connect_via_relay`]: the listener side (§4.2), native and, with the
//!   `web` feature, in the browser;
//! - [`direct`] (feature `direct`): UDP hole punching and the direct QUIC
//!   transport (§5.2).
//!
//! Features: `native` (tokio client), `direct` (QUIC, implies `native`;
//! both default), `web` (wasm32 listener and relay-only station over the
//! browser WebSocket).

pub mod candidate;
mod error;
mod exchange;
mod identity;
pub mod msg;
mod server;
pub mod stun;

#[cfg(feature = "direct")]
pub mod direct;
#[cfg(feature = "native")]
mod native;
#[cfg(any(feature = "native", all(feature = "web", target_arch = "wasm32")))]
mod registration;
#[cfg(all(feature = "web", target_arch = "wasm32"))]
mod web;

pub use candidate::{Candidate, candidate_strings, parse_candidates};
pub use error::RelayError;
pub use exchange::PeerInfo;
pub use identity::{IDENTITY_BYTES_LEN, IdentityError, MailboxId, MailboxIdentity, secret_hash};
pub use msg::{ErrorCode, Message};
pub use nestwatch_proto::RelayRef;
pub use server::{DEFAULT_SERVER, RENDEZVOUS_PATH, RelayConfig, RelayServer};

#[cfg(feature = "native")]
pub use native::{IncomingRequest, StationRegistration, connect_via_relay, local_udp_candidates};
#[cfg(any(feature = "native", all(feature = "web", target_arch = "wasm32")))]
pub use registration::{Incoming, RegistrationConfig, RegistrationStatus};
#[cfg(all(feature = "web", target_arch = "wasm32"))]
pub use web::{IncomingRequest, StationRegistration, connect_via_relay};
