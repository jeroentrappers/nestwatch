// SPDX-License-Identifier: AGPL-3.0-or-later
//! Shared Dioxus components and screens of the Lull desktop app and the
//! browser listener (architecture.md §2, design/README.md §5).
//!
//! - [`LullApp`] is the root component; it renders every screen from the
//!   [`backend::Ui`] signals and sends intents to a [`backend::Backend`].
//! - [`adapter::EngineBackend`] (features `native` / `web`) runs the
//!   components against the lull-core engines; [`demo::DemoBackend`] against
//!   made-up data for design previews.
//! - [`LULL_CSS`] is `design/lull.css` verbatim; [`APP_CSS`] holds the few
//!   app-only additions. Hosts load both (as files on the web, where the
//!   station's CSP forbids inline styles; in the head on desktop).
//! - All English text lives in [`strings`] (NFR-I18N-1).

pub mod backend;
pub mod components;
pub mod demo;
pub mod logic;
pub mod model;
pub mod qr;
pub mod strings;

#[cfg(any(feature = "native", feature = "web"))]
pub mod adapter;

pub use backend::{Backend, BackendHandle, LocalFuture, Ui};
pub use components::app::LullApp;

/// The design system stylesheet, verbatim (`design/lull.css`).
pub const LULL_CSS: &str = include_str!("../../../design/lull.css");
/// App-only additions on top of the design system.
pub const APP_CSS: &str = include_str!("../app.css");
/// The brand mark (`design/logo.svg`).
pub const LOGO_SVG: &str = include_str!("../../../design/logo.svg");
