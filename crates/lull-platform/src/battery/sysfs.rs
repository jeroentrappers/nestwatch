// SPDX-License-Identifier: AGPL-3.0-or-later
//! Linux battery state from `/sys/class/power_supply`.
//!
//! System batteries are supplies of `type` `Battery` whose `scope` is not
//! `Device` (that marks peripherals such as mice). Several batteries are
//! combined by energy (or charge) where all report it, else by averaging
//! `capacity`. "Charging" means on external power: a non-battery supply
//! (`Mains`, `USB`, ...) is `online`, or a battery reports `Charging`.

use std::fs;
use std::path::{Path, PathBuf};

use super::{BatteryInfo, BatteryStatus, ratio_to_percent};
use crate::PlatformError;

/// The sysfs backend.
#[derive(Debug, Clone)]
pub struct SysfsBattery {
    root: PathBuf,
}

impl Default for SysfsBattery {
    fn default() -> Self {
        Self::new("/sys/class/power_supply")
    }
}

impl SysfsBattery {
    /// Reads supplies under `root` (normally `/sys/class/power_supply`).
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }
}

#[derive(Debug, Default)]
struct Supply {
    kind: String,
    scope: Option<String>,
    status: Option<String>,
    online: Option<bool>,
    capacity: Option<f64>,
    energy: Option<(f64, f64)>,
    charge: Option<(f64, f64)>,
}

fn read(dir: &Path, name: &str) -> Option<String> {
    fs::read_to_string(dir.join(name))
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
}

fn num(dir: &Path, name: &str) -> Option<f64> {
    read(dir, name)?.parse().ok()
}

fn pair(dir: &Path, now: &str, full: &str) -> Option<(f64, f64)> {
    let (n, f) = (num(dir, now)?, num(dir, full)?);
    (f > 0.0).then_some((n, f))
}

fn load(dir: &Path) -> Option<Supply> {
    Some(Supply {
        kind: read(dir, "type")?,
        scope: read(dir, "scope"),
        status: read(dir, "status"),
        online: read(dir, "online").map(|v| v != "0"),
        capacity: num(dir, "capacity"),
        energy: pair(dir, "energy_now", "energy_full"),
        charge: pair(dir, "charge_now", "charge_full"),
    })
}

fn combine(supplies: &[Supply]) -> Option<BatteryStatus> {
    let is_system = |s: &&Supply| s.scope.as_deref() != Some("Device");
    let batteries: Vec<&Supply> = supplies
        .iter()
        .filter(is_system)
        .filter(|s| s.kind == "Battery")
        .collect();
    if batteries.is_empty() {
        return None;
    }
    let sum = |f: fn(&Supply) -> Option<(f64, f64)>| -> Option<f64> {
        let pairs: Option<Vec<(f64, f64)>> = batteries.iter().map(|b| f(b)).collect();
        let (n, full) = pairs?
            .into_iter()
            .fold((0.0, 0.0), |(a, b), (n, f)| (a + n, b + f));
        (full > 0.0).then_some(n / full)
    };
    let ratio = if batteries.len() == 1 && batteries[0].capacity.is_some() {
        // Firmware-reported capacity matches what the desktop shows.
        batteries[0].capacity.map(|c| c / 100.0)
    } else {
        sum(|b| b.energy).or_else(|| sum(|b| b.charge)).or_else(|| {
            let caps: Option<Vec<f64>> = batteries.iter().map(|b| b.capacity).collect();
            let caps = caps?;
            Some(caps.iter().sum::<f64>() / caps.len() as f64 / 100.0)
        })
    }?;
    let external: Vec<&Supply> = supplies
        .iter()
        .filter(is_system)
        .filter(|s| s.kind != "Battery")
        .collect();
    let any_status = |want: &str| batteries.iter().any(|b| b.status.as_deref() == Some(want));
    let charging = external.iter().any(|s| s.online == Some(true))
        || any_status("Charging")
        || (external.is_empty() && any_status("Full"));
    Some(BatteryStatus {
        percent: ratio_to_percent(ratio),
        charging,
    })
}

impl BatteryInfo for SysfsBattery {
    fn backend(&self) -> &'static str {
        "sysfs"
    }

    fn status(&self) -> Result<Option<BatteryStatus>, PlatformError> {
        let entries = match fs::read_dir(&self.root) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        let mut paths: Vec<PathBuf> = entries.filter_map(Result::ok).map(|e| e.path()).collect();
        paths.sort();
        let supplies: Vec<Supply> = paths.iter().filter_map(|p| load(p)).collect();
        Ok(combine(&supplies))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> SysfsBattery {
        SysfsBattery::new(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/power_supply")
                .join(name),
        )
    }

    #[test]
    fn laptop_on_ac_is_charging() {
        let s = fixture("laptop-charging").status().unwrap().unwrap();
        assert_eq!(
            s,
            BatteryStatus {
                percent: 92,
                charging: true
            }
        );
    }

    #[test]
    fn two_batteries_combine_by_energy() {
        // 20 Wh of 50 Wh + 10 Wh of 25 Wh = 30 / 75 = 40 %.
        let s = fixture("two-batteries-discharging")
            .status()
            .unwrap()
            .unwrap();
        assert_eq!(
            s,
            BatteryStatus {
                percent: 40,
                charging: false
            }
        );
    }

    #[test]
    fn held_at_charge_limit_counts_as_charging() {
        let s = fixture("charge-limit").status().unwrap().unwrap();
        assert_eq!(s.percent, 80);
        assert!(s.charging);
    }

    #[test]
    fn peripheral_battery_is_ignored() {
        assert_eq!(fixture("desktop-with-mouse").status().unwrap(), None);
    }

    #[test]
    fn missing_root_means_no_battery() {
        assert_eq!(fixture("does-not-exist").status().unwrap(), None);
    }
}
