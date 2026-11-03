// SPDX-License-Identifier: AGPL-3.0-or-later
//! Platform glue for Nestwatch (architecture.md §1, platforms.md §2).
//!
//! `nestwatch-core` only talks to the traits defined here; each OS provides an
//! implementation. This crate also ships the desktop implementations for
//! Linux, macOS and Windows and a [`DesktopPlatform`] bundle that picks the
//! best available backend for each trait at runtime.
//!
//! | Trait | Purpose |
//! |-------|---------|
//! | [`AudioInput`] | microphone capture, interleaved `f32` / `i16` at the device rate |
//! | [`AudioOutput`] | playback, pulling 16 kHz mono frames or device-rate `f32` |
//! | [`Camera`] | camera listing and frame capture |
//! | [`KeyStore`] | named secret blobs (security.md §5) |
//! | [`PowerGuard`] | system sleep inhibition while a station runs |
//! | [`Notifier`] | system notifications for alerts |
//! | [`BatteryInfo`] | battery percent and charging state |
//!
//! All handles are `Send`; streams and sleep inhibitions stop or release
//! when dropped. Nothing here keeps global state.
//!
//! # Features
//!
//! | Feature | Default | Backend | Extra build needs |
//! |---------|---------|---------|-------------------|
//! | `cpal` | always on macOS / Windows | cpal audio | Linux: ALSA headers (Debian `libasound2-dev`, `pkg-config`) |
//! | `camera` | yes | nokhwa | macOS: Xcode command line tools |
//! | `keyring` | yes | Secret Service / Keychain / Credential Manager | none (pure-Rust D-Bus) |
//! | `notify` | yes | notify-rust | macOS: Xcode command line tools |
//! | `power` | yes | keepawake | none |
//! | `battery` | yes | starship-battery | none |
//!
//! Always built on Linux: the `parec`/`pacat` sound-server backend
//! (runtime: `pulseaudio-utils`), the `ffmpeg` camera fallback (runtime:
//! `ffmpeg`), the `systemd-inhibit` sleep inhibitor, the sysfs battery
//! reader and the file key store.

pub mod audio;
pub mod battery;
pub mod camera;
mod desktop;
mod error;
pub mod keystore;
pub mod notify;
pub mod power;
mod process;

pub use audio::{
    AudioDevice, AudioInput, AudioOutput, AudioStream, CaptureCallback, CaptureRequest,
    PlaybackRequest, PlaybackSource, SampleType, Samples, StreamFormat, StreamStats,
};
pub use battery::{BatteryInfo, BatteryStatus};
pub use camera::{
    Camera, CameraInfo, CameraMode, CameraRequest, CameraStream, Facing, FrameCallback, PixelData,
    VideoFrame,
};
pub use desktop::{DesktopOptions, DesktopPlatform, Selection, SelectionReport};
pub use error::PlatformError;
pub use keystore::{KeyStore, Secret};
pub use notify::{Notification, Notifier, Urgency};
pub use power::{PowerGuard, SleepInhibition};

/// Application name used for client, stream and inhibitor names.
pub const APP_NAME: &str = "Nestwatch";
/// Reverse-DNS application id.
pub const APP_ID: &str = "org.nestwatch.Nestwatch";
