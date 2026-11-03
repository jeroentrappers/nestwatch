// SPDX-License-Identifier: AGPL-3.0-or-later
//! System notifications for alerts (product.md FR-AL-1..4).

use crate::PlatformError;

#[cfg(feature = "notify")]
pub mod notify_rust_backend;

/// How urgent an alert is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Urgency {
    /// Informational (e.g. a station is reachable again).
    Low,
    /// Default.
    #[default]
    Normal,
    /// Needs attention now (sound, unreachable station, low battery). Stays
    /// visible until dismissed where the platform supports it.
    Critical,
}

/// A notification to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notification {
    /// Title line.
    pub title: String,
    /// Body text.
    pub body: String,
    /// Urgency.
    pub urgency: Urgency,
}

/// Shows system notifications.
pub trait Notifier: Send + Sync {
    /// Backend name for diagnostics.
    fn backend(&self) -> &'static str;
    /// Shows `notification`.
    fn notify(&self, notification: &Notification) -> Result<(), PlatformError>;
}

/// Fallback that only logs that a notification could not be shown. Title
/// and body are not logged: they name stations (security.md PRIV-2).
#[derive(Debug, Clone, Copy, Default)]
pub struct LogNotifier;

impl Notifier for LogNotifier {
    fn backend(&self) -> &'static str {
        "log"
    }

    fn notify(&self, n: &Notification) -> Result<(), PlatformError> {
        log::warn!(
            "no notification service; {:?} alert not shown as a system notification",
            n.urgency
        );
        Ok(())
    }
}
