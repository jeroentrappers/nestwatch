// SPDX-License-Identifier: AGPL-3.0-or-later
//! Alert decisions per station (product.md §3.5). Pure logic on caller time;
//! alerts do not depend on mute or volume (FR-AL-4, FR-AU-6).

/// Something the user must be told about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Alert {
    /// A quiet station started sending sound (FR-AL-1).
    SoundStarted,
    /// The station has been unreachable for longer than 20 s (FR-AL-2).
    Unreachable,
    /// It is reachable again after an [`Alert::Unreachable`] (FR-AL-2).
    Reachable,
    /// Battery below 15 % while not charging (FR-AL-3).
    BatteryLow {
        /// Reported percent.
        percent: u8,
    },
}

/// Alert limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AlertLimits {
    pub sound_interval_ms: u64,
    pub unreachable_after_ms: u64,
    pub battery_percent: u8,
}

/// Per-station alert state.
#[derive(Debug, Clone, Default)]
pub(crate) struct AlertState {
    last_sound: Option<u64>,
    loud: bool,
    down_since: Option<u64>,
    unreachable_alerted: bool,
    battery_alerted: bool,
}

impl AlertState {
    fn sound(&mut self, now: u64, limits: &AlertLimits) -> bool {
        let due = self
            .last_sound
            .is_none_or(|t| now.saturating_sub(t) >= limits.sound_interval_ms);
        if due {
            self.last_sound = Some(now);
        }
        due
    }

    /// `activity {open}` in sound activation mode (protocol.md §7.5).
    pub fn on_activity(&mut self, open: bool, now: u64, limits: &AlertLimits) -> bool {
        open && self.sound(now, limits)
    }

    /// A `level` in continuous mode, where the station sends no
    /// `activity` (MEDIA-7): crossing the threshold upward counts (CORE-5).
    pub fn on_level(&mut self, dbfs: f32, threshold: f32, now: u64, limits: &AlertLimits) -> bool {
        let loud = dbfs >= threshold;
        let rising = loud && !self.loud;
        self.loud = loud;
        rising && self.sound(now, limits)
    }

    /// The station is not reachable from `now` on (if not already down).
    pub fn on_down(&mut self, now: u64) {
        self.down_since.get_or_insert(now);
        self.loud = false;
    }

    /// A session is live again; `true` if "reachable again" is due.
    pub fn on_up(&mut self) -> bool {
        self.down_since = None;
        std::mem::take(&mut self.unreachable_alerted)
    }

    /// Whether "unreachable" is due now.
    pub fn check_unreachable(&mut self, now: u64, limits: &AlertLimits) -> bool {
        match self.down_since {
            Some(t)
                if !self.unreachable_alerted
                    && now.saturating_sub(t) > limits.unreachable_after_ms =>
            {
                self.unreachable_alerted = true;
                true
            }
            _ => false,
        }
    }

    /// A `status` with battery data; returns the percent if an alert is due.
    /// Re-arms once charging or back at or above the limit.
    pub fn on_battery(
        &mut self,
        battery: Option<u8>,
        charging: Option<bool>,
        limits: &AlertLimits,
    ) -> Option<u8> {
        let low = match (battery, charging) {
            (Some(p), c) => p < limits.battery_percent && c != Some(true),
            _ => false,
        };
        if !low {
            self.battery_alerted = false;
            return None;
        }
        if self.battery_alerted {
            return None;
        }
        self.battery_alerted = true;
        battery
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const L: AlertLimits = AlertLimits {
        sound_interval_ms: 30_000,
        unreachable_after_ms: 20_000,
        battery_percent: 15,
    };

    #[test]
    fn fr_al_1_sound_alert_rate_limited_to_once_per_30_s() {
        let mut a = AlertState::default();
        assert!(a.on_activity(true, 0, &L));
        assert!(!a.on_activity(false, 1_000, &L));
        assert!(!a.on_activity(true, 10_000, &L));
        assert!(a.on_activity(true, 30_000, &L));
    }

    #[test]
    fn fr_al_1_continuous_mode_threshold_crossing() {
        let mut a = AlertState::default();
        assert!(!a.on_level(-60.0, -50.0, 0, &L));
        assert!(a.on_level(-40.0, -50.0, 100, &L));
        assert!(!a.on_level(-40.0, -50.0, 200, &L));
        assert!(!a.on_level(-60.0, -50.0, 300, &L));
        assert!(!a.on_level(-40.0, -50.0, 400, &L), "rate limited");
    }

    #[test]
    fn fr_al_2_unreachable_after_20_s_and_back() {
        let mut a = AlertState::default();
        a.on_down(1_000);
        assert!(!a.check_unreachable(21_000, &L));
        assert!(a.check_unreachable(21_001, &L));
        assert!(!a.check_unreachable(40_000, &L));
        assert!(a.on_up());
        assert!(!a.on_up());
        a.on_down(50_000);
        assert!(!a.on_up(), "short outage: no reachable alert");
    }

    #[test]
    fn fr_al_3_battery_low_once_until_rearmed() {
        let mut a = AlertState::default();
        assert_eq!(a.on_battery(Some(14), Some(false), &L), Some(14));
        assert_eq!(a.on_battery(Some(13), Some(false), &L), None);
        assert_eq!(a.on_battery(Some(13), Some(true), &L), None);
        assert_eq!(a.on_battery(Some(12), None, &L), Some(12));
        assert_eq!(a.on_battery(Some(15), Some(false), &L), None);
        assert_eq!(a.on_battery(None, None, &L), None);
    }
}
