// SPDX-License-Identifier: AGPL-3.0-or-later
//! Battery state through `starship-battery` (IOKit on macOS, the power
//! API on Windows).

use starship_battery::State;
use starship_battery::units::ratio::ratio;

use super::{BatteryInfo, BatteryStatus, ratio_to_percent};
use crate::PlatformError;

/// The starship-battery backend.
#[derive(Debug, Clone, Copy, Default)]
pub struct StarshipBattery;

fn err(e: starship_battery::Error) -> PlatformError {
    PlatformError::Unavailable(format!("battery: {e}"))
}

impl BatteryInfo for StarshipBattery {
    fn backend(&self) -> &'static str {
        "starship-battery"
    }

    fn status(&self) -> Result<Option<BatteryStatus>, PlatformError> {
        let manager = starship_battery::Manager::new().map_err(err)?;
        let mut energy = 0.0f64;
        let mut full = 0.0f64;
        let mut charging = false;
        let mut found = false;
        for battery in manager.batteries().map_err(err)? {
            let b = battery.map_err(err)?;
            found = true;
            let ratio_now = f64::from(b.state_of_charge().get::<ratio>());
            let full_j = f64::from(b.energy_full().value);
            if full_j > 0.0 {
                energy += ratio_now * full_j;
                full += full_j;
            } else {
                energy += ratio_now;
                full += 1.0;
            }
            charging |= matches!(b.state(), State::Charging | State::Full | State::Paused);
        }
        Ok(found.then(|| BatteryStatus {
            percent: ratio_to_percent(if full > 0.0 { energy / full } else { 0.0 }),
            charging,
        }))
    }
}
