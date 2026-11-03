// SPDX-License-Identifier: AGPL-3.0-or-later
//! Shared Dioxus components and screens of the Nestwatch desktop app and the
//! browser listener (architecture.md §2, design/README.md §5).
//!
//! - [`NestwatchApp`] is the root component; it renders every screen from the
//!   [`backend::Ui`] signals and sends intents to a [`backend::Backend`].
//! - [`adapter::EngineBackend`] (features `native` / `web`) runs the
//!   components against the nestwatch-core engines; [`demo::DemoBackend`] against
//!   made-up data for design previews.
//! - [`NESTWATCH_CSS`] is `design/nestwatch.css` verbatim; [`APP_CSS`] holds the few
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
pub use components::app::NestwatchApp;

/// The design system stylesheet, verbatim (`design/nestwatch.css`).
pub const NESTWATCH_CSS: &str = include_str!("../../../design/nestwatch.css");
/// App-only additions on top of the design system.
pub const APP_CSS: &str = include_str!("../app.css");
/// The brand mark (`design/logo.svg`).
pub const LOGO_SVG: &str = include_str!("../../../design/logo.svg");
