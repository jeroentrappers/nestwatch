// SPDX-License-Identifier: AGPL-3.0-or-later
//! UniFFI bindings exposing the Lull engines to Kotlin and Swift
//! (`specs/architecture.md` §2, §5).
//!
//! The API is object based:
//!
//! - [`Station`] wraps `lull_core::station::StationEngine` and is started
//!   with [`start_station`]; [`Listener`] wraps
//!   `lull_core::listener::ListenerEngine` and is created with
//!   [`Listener::start`].
//! - State arrives as immutable records through foreign-implemented
//!   observers ([`StationObserver`], [`ListenerObserver`]) at a bounded
//!   rate: at most 10 snapshots per second, plus a 1 s heartbeat so
//!   countdowns (pairing window) advance. Events and video frames arrive
//!   through the same observers as they happen.
//! - Intents are plain method calls; slow ones (starting, pairing,
//!   stopping) are `async` (Kotlin `suspend`, Swift `async`).
//! - Audio crosses the boundary as little-endian PCM bytes
//!   (`push_capture_*`, `pull_playback`, `pull_talkback`), the layout
//!   `AudioRecord`/`AudioTrack` and `AVAudioPCMBuffer` use natively. Camera
//!   frames are RGBA, RGB or I420 with a rotation ([`CameraImage`]); video
//!   frames out are JPEG bytes.
//! - Secrets and settings go through the foreign [`Storage`] (Android
//!   Keystore or iOS Keychain backed; security.md §5).
//!
//! Bindings are generated in library mode by the `uniffi-bindgen` binary
//! (feature `bindgen`): `crates/lull-ffi/generate-bindings.sh` writes Kotlin
//! and Swift into `target/uniffi/`, and the Android Gradle build generates
//! its Kotlin copy itself. `uniffi.toml` sets the Kotlin package
//! `org.lull.ffi` and the Swift module `LullFFI`.
//!
//! The crate runs its own multi-threaded tokio runtime. Observer methods
//! are called on its threads and must return quickly (post to the UI
//! thread); they may call engine methods.

uniffi::setup_scaffolding!();

mod error;
mod listener;
mod media;
mod runtime;
mod station;
mod storage;
mod util;

pub use error::{LullError, StorageFailure};
pub use listener::*;
pub use media::{CameraImage, ImageFormat};
pub use station::*;
pub use storage::Storage;
pub use util::*;
