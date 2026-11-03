// SPDX-License-Identifier: AGPL-3.0-or-later
//! Battery state for station status (product.md FR-ST-6, protocol.md §7.6).

use crate::PlatformError;

#[cfg(feature = "battery")]
pub mod starship;
#[cfg(target_os = "linux")]
pub mod sysfs;

/// Battery state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BatteryStatus {
    /// Charge, 0..=100.
    pub percent: u8,
    /// On external power (charging, full, or held at a charge limit).
    pub charging: bool,
}

/// Reads the battery.
pub trait BatteryInfo: Send + Sync {
    /// Backend name for diagnostics.
    fn backend(&self) -> &'static str;
    /// Current state; `Ok(None)` on machines without a system battery.
    fn status(&self) -> Result<Option<BatteryStatus>, PlatformError>;
}

/// Reports no battery (mains-only machines, or no backend).
#[derive(Debug, Clone, Copy, Default)]
pub struct NoBattery;

impl BatteryInfo for NoBattery {
    fn backend(&self) -> &'static str {
        "none"
    }

    fn status(&self) -> Result<Option<BatteryStatus>, PlatformError> {
        Ok(None)
    }
}

/// Rounds and clamps a ratio in `0..=1` to a percentage.
pub(crate) fn ratio_to_percent(r: f64) -> u8 {
    (r * 100.0).round().clamp(0.0, 100.0) as u8
}
