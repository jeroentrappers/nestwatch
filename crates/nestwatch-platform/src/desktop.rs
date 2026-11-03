// SPDX-License-Identifier: AGPL-3.0-or-later
//! Runtime selection of desktop backends.

use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;

use crate::audio::unavailable::NoAudio;
use crate::battery::NoBattery;
use crate::camera::unavailable::NoCamera;
use crate::keystore::file::FileKeyStore;
use crate::notify::LogNotifier;
use crate::power::NoPowerGuard;
use crate::{
    AudioInput, AudioOutput, BatteryInfo, Camera, KeyStore, Notifier, PlatformError, PowerGuard,
};

/// Overrides for backend selection (diagnostics, tests, packaging).
#[derive(Debug, Clone, Default)]
pub struct DesktopOptions {
    /// Use only the audio backend with this name (`"pulse"`, `"cpal"`).
    pub audio: Option<String>,
    /// Use only the camera backend with this name (`"nokhwa"`, `"ffmpeg"`).
    pub camera: Option<String>,
    /// Use only the key store with this name (`"os"`, `"sealed"`, `"file"`).
    pub key_store: Option<String>,
    /// Directory for file-based secrets instead of the per-user default.
    pub key_dir: Option<PathBuf>,
}

/// What was chosen for one component, and why the others were not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    /// Component name (`"audio"`, `"camera"`, ...).
    pub component: &'static str,
    /// Chosen backend, `"none"` when nothing was usable.
    pub chosen: &'static str,
    /// Backends tried before it, with the reason each was skipped.
    pub rejected: Vec<(&'static str, String)>,
}

/// Selections for every component.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SelectionReport {
    /// One entry per component.
    pub selections: Vec<Selection>,
}

impl SelectionReport {
    /// The chosen backend for `component`.
    pub fn chosen(&self, component: &str) -> Option<&'static str> {
        self.selections
            .iter()
            .find(|s| s.component == component)
            .map(|s| s.chosen)
    }
}

impl fmt::Display for SelectionReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for s in &self.selections {
            write!(f, "{:<10} {}", s.component, s.chosen)?;
            for (name, why) in &s.rejected {
                write!(f, " [skipped {name}: {why}]")?;
            }
            writeln!(f)?;
        }
        Ok(())
    }
}

/// A backend candidate: a name and a constructor that probes availability.
pub(crate) type Candidate<'a, T> = (
    &'static str,
    Box<dyn FnOnce() -> Result<T, PlatformError> + 'a>,
);

/// Tries candidates in order (only `forced` when given) and returns the
/// first that constructs.
pub(crate) fn select<T>(
    component: &'static str,
    candidates: Vec<Candidate<'_, T>>,
    forced: Option<&str>,
) -> (Option<T>, Selection) {
    let mut rejected = Vec::new();
    for (name, make) in candidates {
        if forced.is_some_and(|f| f != name) {
            rejected.push((name, "not requested".to_owned()));
            continue;
        }
        match make() {
            Ok(v) => {
                return (
                    Some(v),
                    Selection {
                        component,
                        chosen: name,
                        rejected,
                    },
                );
            }
            Err(e) => rejected.push((name, e.to_string())),
        }
    }
    (
        None,
        Selection {
            component,
            chosen: "none",
            rejected,
        },
    )
}

type AudioPair = (Arc<dyn AudioInput>, Arc<dyn AudioOutput>);

/// The best available implementation of every trait on this desktop.
#[derive(Clone)]
pub struct DesktopPlatform {
    /// Microphone.
    pub audio_input: Arc<dyn AudioInput>,
    /// Speaker.
    pub audio_output: Arc<dyn AudioOutput>,
    /// Camera.
    pub camera: Arc<dyn Camera>,
    /// Secret storage.
    pub key_store: Arc<dyn KeyStore>,
    /// Sleep inhibition.
    pub power: Arc<dyn PowerGuard>,
    /// Notifications.
    pub notifier: Arc<dyn Notifier>,
    /// Battery.
    pub battery: Arc<dyn BatteryInfo>,
    /// What was picked and why.
    pub report: SelectionReport,
}

impl fmt::Debug for DesktopPlatform {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DesktopPlatform")
            .field("report", &self.report)
            .finish_non_exhaustive()
    }
}

impl DesktopPlatform {
    /// Probes the system and picks a backend for each trait. Never fails:
    /// components without a usable backend get one that reports why.
    /// Probing does not stream from devices or trigger permission
    /// prompts.
    pub fn detect(options: &DesktopOptions) -> Self {
        let mut report = SelectionReport::default();

        let (audio, sel) = select("audio", audio_candidates(), options.audio.as_deref());
        let (audio_input, audio_output) = audio.unwrap_or_else(|| {
            let none = Arc::new(NoAudio(no_backend(&sel)));
            (
                none.clone() as Arc<dyn AudioInput>,
                none as Arc<dyn AudioOutput>,
            )
        });
        report.selections.push(sel);

        let (camera, sel) = select("camera", camera_candidates(), options.camera.as_deref());
        let camera = camera.unwrap_or_else(|| Arc::new(NoCamera(no_backend(&sel))));
        report.selections.push(sel);

        let (key_store, sel) = select(
            "key-store",
            key_store_candidates(options.key_dir.as_ref()),
            options.key_store.as_deref(),
        );
        let key_store = key_store.unwrap_or_else(|| {
            log::warn!("no persistent key store; secrets live in memory only");
            Arc::new(crate::keystore::MemoryKeyStore::new())
        });
        report.selections.push(sel);

        let (power, sel) = select("power", power_candidates(), None);
        let power = power.unwrap_or_else(|| Arc::new(NoPowerGuard));
        report.selections.push(sel);

        let (notifier, sel) = select("notify", notifier_candidates(), None);
        let notifier = notifier.unwrap_or_else(|| Arc::new(LogNotifier));
        report.selections.push(sel);

        let (battery, sel) = select("battery", battery_candidates(), None);
        let battery = battery.unwrap_or_else(|| Arc::new(NoBattery));
        report.selections.push(sel);

        Self {
            audio_input,
            audio_output,
            camera,
            key_store,
            power,
            notifier,
            battery,
            report,
        }
    }
}

fn no_backend(sel: &Selection) -> String {
    if sel.rejected.is_empty() {
        format!("no {} backend in this build", sel.component)
    } else {
        let why: Vec<String> = sel
            .rejected
            .iter()
            .map(|(n, w)| format!("{n}: {w}"))
            .collect();
        format!(
            "no {} backend available ({})",
            sel.component,
            why.join("; ")
        )
    }
}

// Which candidates exist depends on the target and features.
#[allow(clippy::vec_init_then_push)]
fn audio_candidates() -> Vec<Candidate<'static, AudioPair>> {
    #[allow(unused_mut)]
    let mut c: Vec<Candidate<'static, AudioPair>> = Vec::new();
    #[cfg(target_os = "linux")]
    c.push((
        "pulse",
        Box::new(|| {
            let pa = Arc::new(crate::audio::pulse::PulseAudio::detect()?);
            Ok((
                pa.clone() as Arc<dyn AudioInput>,
                pa as Arc<dyn AudioOutput>,
            ))
        }),
    ));
    #[cfg(any(feature = "cpal", not(target_os = "linux")))]
    c.push((
        "cpal",
        Box::new(|| {
            let a = Arc::new(crate::audio::cpal_backend::CpalAudio::detect()?);
            Ok((a.clone() as Arc<dyn AudioInput>, a as Arc<dyn AudioOutput>))
        }),
    ));
    c
}

// Which candidates exist depends on the target and features.
#[allow(clippy::vec_init_then_push)]
fn camera_candidates() -> Vec<Candidate<'static, Arc<dyn Camera>>> {
    #[allow(unused_mut)]
    let mut c: Vec<Candidate<'static, Arc<dyn Camera>>> = Vec::new();
    #[cfg(feature = "camera")]
    c.push((
        "nokhwa",
        Box::new(|| {
            let cam = crate::camera::nokhwa_backend::NokhwaCamera::new();
            // On Linux listing needs no permission; elsewhere listing would
            // prompt, so availability is assumed (PRIV-4).
            #[cfg(target_os = "linux")]
            if cam.cameras()?.is_empty() {
                return Err(PlatformError::Unavailable("no cameras listed".into()));
            }
            Ok(Arc::new(cam) as Arc<dyn Camera>)
        }),
    ));
    #[cfg(target_os = "linux")]
    c.push((
        "ffmpeg",
        Box::new(
            || Ok(Arc::new(crate::camera::ffmpeg::FfmpegCamera::detect()?) as Arc<dyn Camera>),
        ),
    ));
    c
}

fn file_store(dir: Option<&PathBuf>) -> Result<FileKeyStore, PlatformError> {
    match dir {
        Some(d) => FileKeyStore::open(d),
        None => FileKeyStore::open_default(),
    }
}

// Which candidates exist depends on the target and features.
#[allow(clippy::vec_init_then_push)]
fn key_store_candidates(dir: Option<&PathBuf>) -> Vec<Candidate<'_, Arc<dyn KeyStore>>> {
    #[allow(unused_mut)]
    let mut c: Vec<Candidate<'_, Arc<dyn KeyStore>>> = Vec::new();
    // Windows: the Credential Manager limits blob sizes, so it only holds
    // the wrapping key of a sealed file store (security.md §5 asks for a
    // DPAPI-protected file there).
    #[cfg(all(feature = "keyring", target_os = "windows"))]
    c.push((
        "sealed",
        Box::new(move || {
            let keys = crate::keystore::os::OsKeyStore::open()?;
            let files = file_store(dir)?;
            Ok(Arc::new(crate::keystore::sealed::SealedKeyStore::new(
                Box::new(keys),
                files,
            )) as Arc<dyn KeyStore>)
        }),
    ));
    // Elsewhere the OS store holds the secrets, layered over the file store
    // so secrets written while it was unavailable are still found.
    #[cfg(all(feature = "keyring", not(target_os = "windows")))]
    c.push((
        "os",
        Box::new(move || {
            let os = crate::keystore::os::OsKeyStore::open()?;
            let files = file_store(dir)?;
            Ok(Arc::new(crate::keystore::LayeredKeyStore::new(
                Box::new(os),
                Box::new(files),
            )) as Arc<dyn KeyStore>)
        }),
    ));
    c.push((
        "file",
        Box::new(move || Ok(Arc::new(file_store(dir)?) as Arc<dyn KeyStore>)),
    ));
    c
}

// Which candidates exist depends on the target and features.
#[allow(clippy::vec_init_then_push)]
fn power_candidates() -> Vec<Candidate<'static, Arc<dyn PowerGuard>>> {
    #[allow(unused_mut)]
    let mut guards: Vec<Box<dyn PowerGuard>> = Vec::new();
    #[cfg(feature = "power")]
    guards.push(Box::new(crate::power::keepawake_backend::KeepAwakeGuard));
    #[cfg(target_os = "linux")]
    if let Ok(s) = crate::power::systemd_inhibit::SystemdInhibit::detect() {
        guards.push(Box::new(s));
    }
    #[cfg(all(feature = "power", target_os = "linux"))]
    const NAME: &str = "keepawake, then systemd-inhibit";
    #[cfg(all(feature = "power", not(target_os = "linux")))]
    const NAME: &str = "keepawake";
    #[cfg(all(not(feature = "power"), target_os = "linux"))]
    const NAME: &str = "systemd-inhibit";
    #[cfg(all(not(feature = "power"), not(target_os = "linux")))]
    const NAME: &str = "none";
    vec![(
        NAME,
        Box::new(move || {
            if guards.is_empty() {
                return Err(PlatformError::Unsupported(
                    "no sleep inhibitor in this build",
                ));
            }
            Ok(Arc::new(crate::power::FallbackPowerGuard::new(guards)) as Arc<dyn PowerGuard>)
        }),
    )]
}

// Which candidates exist depends on the target and features.
#[allow(clippy::vec_init_then_push)]
fn notifier_candidates() -> Vec<Candidate<'static, Arc<dyn Notifier>>> {
    #[allow(unused_mut)]
    let mut c: Vec<Candidate<'static, Arc<dyn Notifier>>> = Vec::new();
    #[cfg(feature = "notify")]
    c.push((
        "notify-rust",
        Box::new(|| {
            Ok(
                Arc::new(crate::notify::notify_rust_backend::DesktopNotifier::detect()?)
                    as Arc<dyn Notifier>,
            )
        }),
    ));
    c.push((
        "log",
        Box::new(|| Ok(Arc::new(LogNotifier) as Arc<dyn Notifier>)),
    ));
    c
}

// Which candidates exist depends on the target and features.
#[allow(clippy::vec_init_then_push)]
fn battery_candidates() -> Vec<Candidate<'static, Arc<dyn BatteryInfo>>> {
    #[allow(unused_mut)]
    let mut c: Vec<Candidate<'static, Arc<dyn BatteryInfo>>> = Vec::new();
    #[cfg(target_os = "linux")]
    c.push((
        "sysfs",
        Box::new(|| {
            Ok(Arc::new(crate::battery::sysfs::SysfsBattery::default()) as Arc<dyn BatteryInfo>)
        }),
    ));
    #[cfg(feature = "battery")]
    c.push((
        "starship-battery",
        Box::new(
            || Ok(Arc::new(crate::battery::starship::StarshipBattery) as Arc<dyn BatteryInfo>),
        ),
    ));
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(v: u32) -> Box<dyn FnOnce() -> Result<u32, PlatformError>> {
        Box::new(move || Ok(v))
    }

    fn fail(why: &'static str) -> Box<dyn FnOnce() -> Result<u32, PlatformError>> {
        Box::new(move || Err(PlatformError::Unavailable(why.into())))
    }

    #[test]
    fn picks_first_available_and_records_rejections() {
        let (v, sel) = select(
            "audio",
            vec![("pulse", fail("no server")), ("cpal", ok(2))],
            None,
        );
        assert_eq!(v, Some(2));
        assert_eq!(sel.chosen, "cpal");
        assert_eq!(
            sel.rejected,
            vec![("pulse", "unavailable: no server".to_owned())]
        );
    }

    #[test]
    fn prefers_earlier_candidates() {
        let (v, sel) = select("audio", vec![("pulse", ok(1)), ("cpal", ok(2))], None);
        assert_eq!((v, sel.chosen), (Some(1), "pulse"));
        assert!(sel.rejected.is_empty());
    }

    #[test]
    fn forced_backend_skips_others() {
        let (v, sel) = select(
            "audio",
            vec![("pulse", ok(1)), ("cpal", ok(2))],
            Some("cpal"),
        );
        assert_eq!((v, sel.chosen), (Some(2), "cpal"));
        assert_eq!(sel.rejected[0].0, "pulse");
        let (v, sel) = select("audio", vec![("pulse", ok(1))], Some("jack"));
        assert_eq!((v, sel.chosen), (None, "none"));
    }

    #[test]
    fn nothing_available() {
        let (v, sel) = select::<u32>("camera", vec![], None);
        assert_eq!((v, sel.chosen), (None, "none"));
        assert_eq!(no_backend(&sel), "no camera backend in this build");
        let (_, sel) = select("camera", vec![("ffmpeg", fail("not installed"))], None);
        assert!(no_backend(&sel).contains("ffmpeg: unavailable: not installed"));
    }

    #[test]
    fn report_display_and_lookup() {
        let report = SelectionReport {
            selections: vec![Selection {
                component: "audio",
                chosen: "cpal",
                rejected: vec![("pulse", "unavailable: x".into())],
            }],
        };
        assert_eq!(report.chosen("audio"), Some("cpal"));
        assert_eq!(report.chosen("camera"), None);
        assert_eq!(
            report.to_string(),
            "audio      cpal [skipped pulse: unavailable: x]\n"
        );
    }

    #[test]
    #[ignore = "opens the default camera for one frame"]
    fn captures_one_camera_frame() {
        let p = DesktopPlatform::detect(&DesktopOptions::default());
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let mut stream = p
            .camera
            .start(
                &crate::CameraRequest::default(),
                Box::new(move |f| {
                    let _ = tx.try_send(f);
                }),
            )
            .unwrap();
        let frame = rx
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap()
            .unwrap();
        stream.stop();
        assert!(frame.width > 0 && frame.height > 0);
    }

    #[test]
    fn file_key_store_is_always_a_candidate() {
        let dir = crate::keystore::file::temp_dir("desktop");
        let opts = DesktopOptions {
            key_store: Some("file".into()),
            key_dir: Some(dir.clone()),
            ..DesktopOptions::default()
        };
        let (store, sel) = select(
            "key-store",
            key_store_candidates(opts.key_dir.as_ref()),
            opts.key_store.as_deref(),
        );
        assert_eq!(sel.chosen, "file");
        let store = store.unwrap();
        store.put("x", b"y").unwrap();
        assert!(store.delete("x").unwrap());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
