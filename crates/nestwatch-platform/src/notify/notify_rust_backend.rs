// SPDX-License-Identifier: AGPL-3.0-or-later
//! Notifications through `notify-rust`: the freedesktop notification
//! service over D-Bus (pure Rust) on Linux, Notification Center on macOS,
//! toast notifications on Windows.

#[cfg(not(target_os = "macos"))]
use super::Urgency;
use super::{Notification, Notifier};
use crate::{APP_NAME, PlatformError};

/// The notify-rust backend.
#[derive(Debug, Clone, Copy, Default)]
pub struct DesktopNotifier;

impl DesktopNotifier {
    /// Checks that a notification service is reachable (Linux asks the
    /// D-Bus service for its identity; other platforms always have one).
    pub fn detect() -> Result<Self, PlatformError> {
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            notify_rust::get_server_information()
                .map_err(|e| PlatformError::Unavailable(format!("notification service: {e}")))?;
        }
        Ok(Self)
    }
}

impl Notifier for DesktopNotifier {
    fn backend(&self) -> &'static str {
        "notify-rust"
    }

    fn notify(&self, n: &Notification) -> Result<(), PlatformError> {
        let mut note = notify_rust::Notification::new();
        note.appname(APP_NAME).summary(&n.title).body(&n.body);
        #[cfg(any(all(unix, not(target_os = "macos")), target_os = "windows"))]
        {
            note.urgency(match n.urgency {
                Urgency::Low => notify_rust::Urgency::Low,
                Urgency::Normal => notify_rust::Urgency::Normal,
                Urgency::Critical => notify_rust::Urgency::Critical,
            });
        }
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            note.icon(match n.urgency {
                Urgency::Critical => "dialog-warning",
                _ => "dialog-information",
            });
            if n.urgency == Urgency::Critical {
                note.timeout(notify_rust::Timeout::Never);
            }
        }
        #[cfg(target_os = "macos")]
        let _ = n.urgency;
        match note.show() {
            Ok(_) => Ok(()),
            Err(e) => Err(PlatformError::Unavailable(format!(
                "notification failed: {e}"
            ))),
        }
    }
}
