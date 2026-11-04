// SPDX-License-Identifier: AGPL-3.0-or-later
//! Nestwatch desktop host (platforms.md §2.4): runs the station and listener
//! engines on `nestwatch-platform`'s [`DesktopPlatform`] and adapts them to the
//! `nestwatch-ui` [`Host`] port. The Dioxus window lives in the binary
//! (feature `window`); everything here builds without a webview.
//!
//! - Storage: the OS key store (security.md §5, PLATFORM-1..3) as the
//!   engines' [`Storage`]; `NESTWATCH_DATA_DIR` selects a plain directory
//!   instead (tests, several instances on one machine).
//! - Station: microphone → `push_capture`, `pull_talkback` → speaker,
//!   camera started on `CameraWanted` and stopped on `CameraNotWanted`,
//!   battery every 30 s, system sleep inhibited while it runs, the browser
//!   listener bundle served from [`web_dir`].
//! - Listener: `pull_playback` → speaker (plus the alert tone), microphone
//!   for talk-back on `MicrophoneWanted`, system notifications for alerts.

pub mod listener_host;
pub mod station_host;
pub mod storage;

use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use nestwatch_core::{Clock, Storage};
use nestwatch_platform::DesktopPlatform;
use nestwatch_ui::adapter::Host;
use nestwatch_ui::adapter::station::RunningStation;
use nestwatch_ui::model::{Capabilities, Platform, Prefs, Theme};
use nestwatch_ui::{LocalFuture, Ui};

/// Where the browser listener bundle is served from (protocol.md §3.3):
/// `$NESTWATCH_WEB_DIR`, then `web/` next to the executable, then
/// `../share/nestwatch/web` relative to it, then the workspace build output
/// `target/nestwatch-web` (development). `None` serves a placeholder page.
pub fn web_dir() -> Option<PathBuf> {
    web_dir_from(
        std::env::var_os("NESTWATCH_WEB_DIR").map(PathBuf::from),
        std::env::current_exe().ok(),
    )
}

/// [`web_dir`] with explicit inputs.
pub fn web_dir_from(env: Option<PathBuf>, exe: Option<PathBuf>) -> Option<PathBuf> {
    let mut candidates = Vec::new();
    candidates.extend(env);
    if let Some(dir) = exe.as_deref().and_then(|e| e.parent()) {
        candidates.push(dir.join("web"));
        candidates.push(dir.join("../share/nestwatch/web"));
    }
    candidates.push(PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../target/nestwatch-web"
    )));
    candidates
        .into_iter()
        .find(|d| d.join("index.html").is_file())
}

/// Engine storage: `$NESTWATCH_DATA_DIR` as a directory, else the OS key store.
pub fn engine_storage(platform: &DesktopPlatform) -> Arc<dyn Storage> {
    match std::env::var_os("NESTWATCH_DATA_DIR") {
        Some(d) => match nestwatch_core::DirStorage::open(PathBuf::from(d)) {
            Ok(s) => Arc::new(s),
            Err(e) => {
                log::error!("NESTWATCH_DATA_DIR unusable ({e}); using the key store");
                Arc::new(storage::KeyStoreStorage::new(platform.key_store.clone()))
            }
        },
        None => Arc::new(storage::KeyStoreStorage::new(platform.key_store.clone())),
    }
}

/// File with UI preferences (not secrets).
pub fn prefs_path() -> PathBuf {
    if let Some(d) = std::env::var_os("NESTWATCH_DATA_DIR") {
        return PathBuf::from(d).join("ui-prefs.txt");
    }
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("nestwatch")
        .join("ui-prefs.txt")
}

/// The desktop [`Host`].
pub struct DesktopHost {
    /// Devices and OS services.
    pub platform: DesktopPlatform,
    /// Engine storage (shared by both roles, CORE-1).
    pub storage: Arc<dyn Storage>,
    /// Engine clock.
    pub clock: Arc<dyn Clock>,
    /// Station options (ports, mDNS) from the environment.
    pub station: station_host::StationOptions,
    /// Sets `data-theme` in the webview.
    pub theme: Box<dyn Fn(Theme)>,
    /// Copies text to the clipboard through the webview.
    pub clipboard: Box<dyn Fn(&str) -> bool>,
}

impl std::fmt::Debug for DesktopHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DesktopHost")
            .field("platform", &self.platform)
            .finish_non_exhaustive()
    }
}

impl Host for DesktopHost {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            platform: Platform::Desktop,
            station: true,
            discovery: true,
            talk: true,
            relay_only: false,
            relay_server_fixed: false,
        }
    }

    fn attach(&self, _ui: Ui) {}

    fn load_prefs(&self) -> Prefs {
        std::fs::read_to_string(prefs_path())
            .map(|s| Prefs::decode(&s))
            .unwrap_or_default()
    }

    fn save_prefs(&self, prefs: &Prefs) {
        let path = prefs_path();
        if let Some(d) = path.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        if let Err(e) = std::fs::write(&path, prefs.encode()) {
            log::warn!("could not save UI preferences: {e}");
        }
    }

    fn apply_theme(&self, theme: Theme) {
        (self.theme)(theme);
    }

    fn sleep(&self, ms: u64) -> LocalFuture<()> {
        Box::pin(tokio::time::sleep(Duration::from_millis(ms)))
    }

    fn copy_text(&self, text: &str) -> bool {
        (self.clipboard)(text)
    }

    fn start_station(&self) -> LocalFuture<Result<RunningStation, String>> {
        let platform = self.platform.clone();
        let storage = self.storage.clone();
        let clock = self.clock.clone();
        let opts = self.station.clone();
        Box::pin(async move {
            station_host::start(platform, storage, clock, opts)
                .await
                .map_err(|e| e.to_string())
        })
    }
}

/// Builds the UI backend over a started listener engine.
pub fn backend(
    host: DesktopHost,
    listener: nestwatch_core::listener::ListenerEngine,
) -> nestwatch_ui::BackendHandle {
    let clock = host.clock.clone();
    nestwatch_ui::BackendHandle::new(nestwatch_ui::adapter::EngineBackend::new(
        Rc::new(host),
        clock,
        listener,
    ))
}

/// The brand mark as RGBA pixels (window and tray icon): a honey crescent
/// with a lavender wave, drawn like `design/logo.svg`.
pub fn logo_rgba(size: u32) -> Vec<u8> {
    let s = size as f32 / 64.0;
    let mut px = vec![0u8; (size * size * 4) as usize];
    for y in 0..size {
        for x in 0..size {
            // Sample the pixel centre in logo units.
            let (fx, fy) = ((x as f32 + 0.5) / s, (y as f32 + 0.5) / s);
            let in_moon = (fx - 30.0).powi(2) + (fy - 32.0).powi(2) <= 24.0f32.powi(2);
            let in_bite = (fx - 40.0).powi(2) + (fy - 24.0).powi(2) <= 20.0f32.powi(2);
            // The inner wave: a ring segment around (36, 32.5), radius ~6.
            let r = ((fx - 35.0).powi(2) + (fy - 32.5).powi(2)).sqrt();
            let wave = (r - 6.0).abs() <= 1.6 && fx > 38.5;
            let rgba: Option<[u8; 4]> = if wave {
                Some([0xb3, 0xa6, 0xff, 0xff])
            } else if in_moon && !in_bite {
                Some([0xf4, 0xbd, 0x7e, 0xff])
            } else {
                None
            };
            if let Some(c) = rgba {
                let i = ((y * size + x) * 4) as usize;
                px[i..i + 4].copy_from_slice(&c);
            }
        }
    }
    px
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logo_has_honey_and_transparent_pixels() {
        let px = logo_rgba(32);
        assert_eq!(px.len(), 32 * 32 * 4);
        let honey = px
            .chunks(4)
            .filter(|c| c == &[0xf4, 0xbd, 0x7e, 0xff])
            .count();
        let clear = px.chunks(4).filter(|c| c[3] == 0).count();
        assert!(honey > 100 && clear > 300, "{honey} {clear}");
    }

    #[test]
    fn web_dir_prefers_the_environment_then_the_executable() {
        let root = std::env::temp_dir().join(format!("nestwatch-web-dir-{}", std::process::id()));
        let env = root.join("env");
        let exe_web = root.join("bin/web");
        for d in [&env, &exe_web] {
            std::fs::create_dir_all(d).unwrap();
            std::fs::write(d.join("index.html"), "x").unwrap();
        }
        let exe = Some(root.join("bin/nestwatch-desktop"));
        assert_eq!(web_dir_from(Some(env.clone()), exe.clone()), Some(env));
        assert_eq!(web_dir_from(None, exe), Some(exe_web));
        let _ = std::fs::remove_dir_all(root);
    }
}
