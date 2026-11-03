// SPDX-License-Identifier: AGPL-3.0-or-later
//! The `Backend` abstraction between the components and the engines
//! (architecture.md §5): state arrives in [`Ui`] signals, intents are
//! plain method calls.
//!
//! Implementations: [`crate::adapter::EngineBackend`] over the nestwatch-core
//! engines (native desktop: station and listener; browser: listener) and
//! [`crate::demo::DemoBackend`] with made-up data for design previews.

use std::collections::BTreeMap;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;

use dioxus::prelude::*;

use crate::model::{
    Capabilities, Frame, Gain, LevelHistory, ListenerModel, Mode, Notice, NoticeKind, PairOutcome,
    Prefs, StationRun, Theme, UiError,
};

/// A boxed, single-threaded future (UI tasks run on the UI thread).
pub type LocalFuture<T> = Pin<Box<dyn Future<Output = T>>>;

/// State the components render. All fields are signals; backends write
/// them from tasks spawned in [`Backend::attach`].
#[derive(Clone, Copy, PartialEq)]
pub struct Ui {
    /// Listener state.
    pub listener: Signal<ListenerModel>,
    /// Station state.
    pub station: Signal<StationRun>,
    /// Recent levels per station (calibration chart).
    pub history: Signal<BTreeMap<String, LevelHistory>>,
    /// Newest camera frame per station.
    pub frames: Signal<BTreeMap<String, Frame>>,
    /// Notices (alerts, pairings).
    pub notices: Signal<Vec<Notice>>,
    /// Listeners that paired in the open window, with their verification
    /// numbers (FR-PAIR-5).
    pub just_paired: Signal<Vec<(String, String)>>,
    /// Audio output waits for a user gesture (browser rule, platforms.md §2.5).
    pub audio_locked: Signal<bool>,
    /// Notifications can be enabled with a gesture (browser).
    pub notifications_askable: Signal<bool>,
    next_notice: Signal<u64>,
}

impl Ui {
    /// Creates the signals in the current component scope; call from a
    /// hook in the root component. (No `Default`: signals belong to the
    /// scope that creates them.)
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self {
            listener: Signal::new(ListenerModel::default()),
            station: Signal::new(StationRun::Stopped),
            history: Signal::new(BTreeMap::new()),
            frames: Signal::new(BTreeMap::new()),
            notices: Signal::new(Vec::new()),
            just_paired: Signal::new(Vec::new()),
            audio_locked: Signal::new(false),
            notifications_askable: Signal::new(false),
            next_notice: Signal::new(1),
        }
    }

    /// Adds a notice; returns its id. At most 6 are kept.
    pub fn notify(mut self, kind: NoticeKind) -> u64 {
        let id = *self.next_notice.peek();
        self.next_notice.set(id + 1);
        let mut n = self.notices.write();
        n.push(Notice { id, kind });
        let excess = n.len().saturating_sub(6);
        n.drain(..excess);
        id
    }

    /// Removes a notice.
    pub fn dismiss(mut self, id: u64) {
        self.notices.write().retain(|n| n.id != id);
    }
}

impl fmt::Debug for Ui {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Ui").finish_non_exhaustive()
    }
}

/// Everything the UI can ask for. Station methods default to
/// "unsupported" for listener-only backends.
pub trait Backend {
    /// What this backend can do.
    fn capabilities(&self) -> Capabilities;
    /// Starts feeding `ui`; called once by the root component.
    fn attach(&self, ui: Ui);
    /// Version string for About.
    fn version(&self) -> String {
        env!("CARGO_PKG_VERSION").to_owned()
    }

    // ----- host ---------------------------------------------------------------

    /// Loads UI preferences.
    fn load_prefs(&self) -> Prefs;
    /// Stores UI preferences.
    fn save_prefs(&self, prefs: &Prefs);
    /// Sets `data-theme` on the root element.
    fn apply_theme(&self, theme: Theme);
    /// Sleeps (UI timers).
    fn sleep(&self, ms: u64) -> LocalFuture<()>;
    /// Copies to the clipboard; `false` if unavailable.
    fn copy_text(&self, _text: &str) -> bool {
        false
    }
    /// Unlocks audio output after a user gesture (browser).
    fn unlock_audio(&self) {}
    /// Asks for notification permission after a user gesture (browser).
    fn request_notifications(&self) {}

    // ----- listener -----------------------------------------------------------

    /// Pairs with an invitation link or web address (FR-PAIR-1).
    fn pair_invitation(&self, input: String) -> LocalFuture<Result<PairOutcome, UiError>>;
    /// Pairs with a code (FR-PAIR-2); `address` is an optional manual
    /// route (FR-NET-2).
    fn pair_code(
        &self,
        code: String,
        station_id: String,
        address: Option<String>,
    ) -> LocalFuture<Result<PairOutcome, UiError>>;
    /// Forgets a station (FR-PAIR-6).
    fn forget(&self, station: &str) -> Result<(), UiError>;
    /// Follows or stops following.
    fn set_follow(&self, station: &str, follow: bool) -> Result<(), UiError>;
    /// Adds a manual address (FR-NET-2).
    fn add_address(&self, station: &str, address: &str) -> Result<(), UiError>;
    /// Sets mode, threshold and hold time (FR-AU-2).
    fn set_listen(
        &self,
        station: &str,
        mode: Mode,
        threshold_dbfs: f32,
        hold_ms: u32,
    ) -> Result<(), UiError>;
    /// Volume 0..1 (FR-AU-6).
    fn set_volume(&self, station: &str, volume: f32) -> Result<(), UiError>;
    /// Mute (FR-AU-6).
    fn set_muted(&self, station: &str, muted: bool) -> Result<(), UiError>;
    /// Alert toggles.
    fn set_alerts(
        &self,
        station: &str,
        sound: bool,
        unreachable: bool,
        battery: bool,
    ) -> Result<(), UiError>;
    /// Solo one station or none (FR-LS-2).
    fn set_solo(&self, station: Option<&str>) -> Result<(), UiError>;
    /// Push-to-talk (FR-TB-1).
    fn talk(&self, station: &str, on: bool) -> Result<(), UiError>;
    /// Opens or closes the camera view (FR-CAM-1).
    fn set_camera(&self, station: &str, on: bool) -> Result<(), UiError>;
    /// Camera facing and night image (FR-CAM-4).
    fn set_video_options(&self, station: &str, front: bool, night: bool) -> Result<(), UiError>;

    // ----- station ------------------------------------------------------------

    /// Starts the station.
    fn start_station(&self) -> LocalFuture<Result<(), UiError>> {
        Box::pin(async { Err(UiError::Unsupported) })
    }
    /// Stops the station.
    fn stop_station(&self) -> LocalFuture<()> {
        Box::pin(async {})
    }
    /// Opens pairing mode.
    fn open_pairing(&self) -> LocalFuture<Result<(), UiError>> {
        Box::pin(async { Err(UiError::Unsupported) })
    }
    /// Closes pairing mode.
    fn close_pairing(&self) {}
    /// Renames the station (FR-ST-7).
    fn rename_station(&self, _name: &str) -> Result<(), UiError> {
        Err(UiError::Unsupported)
    }
    /// Revokes a listener (FR-PAIR-6).
    fn revoke(&self, _listener: &str) {}
    /// Camera allowed (FR-ST-8).
    fn set_camera_enabled(&self, _on: bool) {}
    /// Talk-back allowed (FR-ST-8).
    fn set_talk_enabled(&self, _on: bool) {}
    /// Microphone gain (FR-AU-5).
    fn set_gain(&self, _gain: Gain) {}
    /// Talk-back volume (FR-TB-3).
    fn set_talkback_volume(&self, _volume: f32) {}
    /// "Reachable through a relay server" (rendezvous.md FR-RV-1).
    fn set_relay_enabled(&self, _on: bool) -> Result<(), UiError> {
        Err(UiError::Unsupported)
    }
    /// The relay server address (FR-RV-6).
    fn set_relay_server(&self, _origin: &str) -> Result<(), UiError> {
        Err(UiError::Unsupported)
    }
    /// "Reset relay identity" (rendezvous.md §3.2).
    fn reset_relay_identity(&self) -> Result<(), UiError> {
        Err(UiError::Unsupported)
    }
    /// Current Unix time in ms (for "last seen").
    fn unix_ms(&self) -> u64;
}

/// A cloneable, comparable handle for props and context.
#[derive(Clone)]
pub struct BackendHandle(pub Rc<dyn Backend>);

impl BackendHandle {
    /// Wraps a backend.
    pub fn new(b: impl Backend + 'static) -> Self {
        Self(Rc::new(b))
    }
}

impl PartialEq for BackendHandle {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl fmt::Debug for BackendHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("BackendHandle")
    }
}

impl std::ops::Deref for BackendHandle {
    type Target = dyn Backend;
    fn deref(&self) -> &Self::Target {
        &*self.0
    }
}

/// Resolves to whichever of `a` (`Some`) or `b` (`None`) finishes first;
/// `a` is polled first. Both must be cancel safe.
pub async fn race<A, B>(a: A, b: B) -> Option<A::Output>
where
    A: Future,
    B: Future<Output = ()>,
{
    let mut a = std::pin::pin!(a);
    let mut b = std::pin::pin!(b);
    std::future::poll_fn(move |cx| {
        if let std::task::Poll::Ready(v) = a.as_mut().poll(cx) {
            return std::task::Poll::Ready(Some(v));
        }
        if b.as_mut().poll(cx).is_ready() {
            return std::task::Poll::Ready(None);
        }
        std::task::Poll::Pending
    })
    .await
}
