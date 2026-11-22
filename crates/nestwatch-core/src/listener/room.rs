// SPDX-License-Identifier: AGPL-3.0-or-later
//! Room status, wake-ups, the escalating alarm and "Not the baby"
//! (product.md FR-AL-5..7, §3.11, OPEN-QUESTIONS SL-1). Pure logic on caller
//! time: every input carries the monotonic and the wall-clock time.

/// A point in time on both clocks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct At {
    /// Monotonic milliseconds.
    pub mono: u64,
    /// Unix milliseconds.
    pub unix: u64,
}

/// Room status shown on the station tile (FR-SL-3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RoomView {
    /// The station is not live: nothing is known.
    #[default]
    Unknown,
    /// No wake-up; quiet since this Unix time.
    Quiet {
        /// Unix ms.
        since_unix_ms: u64,
    },
    /// A wake-up that started at this Unix time is going on.
    Awake {
        /// Unix ms.
        since_unix_ms: u64,
    },
}

/// Alarm state of a station on this listener (FR-AL-5, FR-AL-6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AlarmView {
    /// Nothing to escalate.
    #[default]
    Off,
    /// A sound alert nobody acknowledged yet; escalates if the room stays
    /// loud.
    Pending {
        /// When the sound alert was raised (Unix ms).
        since_unix_ms: u64,
    },
    /// The alarm rings: apps play it until this is no longer the state.
    Ringing {
        /// When it started ringing (Unix ms).
        since_unix_ms: u64,
    },
}

/// One wake-up of the night log (FR-SL-4).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WakeUp {
    /// First loud moment (Unix ms).
    pub start_unix_ms: u64,
    /// Last loud moment (Unix ms); `None` while it goes on.
    pub end_unix_ms: Option<u64>,
    /// Total loud time in ms.
    pub loud_ms: u64,
    /// Loudest reported level (dBFS), if any level was reported.
    pub peak_dbfs: Option<f32>,
    /// It raised a sound alert.
    pub alerted: bool,
    /// It escalated to the alarm.
    pub alarmed: bool,
    /// Marked "Not the baby" (FR-AL-7).
    pub not_the_baby: bool,
}

/// Room tunables.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RoomLimits {
    /// Loud time that makes a wake-up (FR-SL-1: 10 s).
    pub awake_loud_ms: u64,
    /// Quiet time that ends one (FR-SL-2: 2 min).
    pub quiet_gap_ms: u64,
    /// "Still loud" for escalation (FR-AL-5: 15 s).
    pub loud_recent_ms: u64,
    /// An outage longer than this restarts "quiet" (FR-SL-3: 60 s).
    pub outage_reset_ms: u64,
    /// "Not the baby" pause (FR-AL-7: 10 min).
    pub pause_ms: u64,
    /// Loud time counted per gap between reports at most (SL-1: 1 s).
    pub max_gap_ms: u64,
}

impl Default for RoomLimits {
    fn default() -> Self {
        Self {
            awake_loud_ms: 10_000,
            quiet_gap_ms: 120_000,
            loud_recent_ms: 15_000,
            outage_reset_ms: 60_000,
            pause_ms: 600_000,
            max_gap_ms: 1_000,
        }
    }
}

/// What a call changed, for the engine to act on.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct RoomOutput {
    /// The view changed: publish.
    pub changed: bool,
    /// The alarm started ringing: raise the alarm alert.
    pub ringing: bool,
    /// A wake-up ended (or was marked): store it in the log.
    pub finished: Option<WakeUp>,
}

#[derive(Debug, Clone, Copy)]
struct Episode {
    start: At,
    loud_ms: u64,
    peak: Option<f32>,
    confirmed: bool,
    alerted: bool,
    alarmed: bool,
    /// Quiet-since before it was confirmed, restored by "Not the baby".
    quiet_before: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Alarm {
    Off,
    Pending { mono: u64, unix: u64 },
    Ringing { unix: u64 },
}

/// Per-station room state on a listener.
#[derive(Debug, Clone)]
pub(crate) struct Room {
    live: bool,
    quiet_since: Option<u64>,
    down_at: Option<u64>,
    last_loud: Option<At>,
    last_sample: Option<(u64, bool)>,
    episode: Option<Episode>,
    alarm: Alarm,
    acknowledged: bool,
    paused_until: Option<At>,
    /// Start of the last finished wake-up, which "Not the baby" marks when
    /// no wake-up is going on, and the quiet-since before it.
    last_finished: Option<(u64, Option<u64>)>,
}

impl Default for Room {
    fn default() -> Self {
        Self {
            live: false,
            quiet_since: None,
            down_at: None,
            last_loud: None,
            last_sample: None,
            episode: None,
            alarm: Alarm::Off,
            acknowledged: false,
            paused_until: None,
            last_finished: None,
        }
    }
}

impl Room {
    /// The tile's room status.
    pub fn view(&self) -> RoomView {
        match (self.live, self.episode, self.quiet_since) {
            (false, ..) => RoomView::Unknown,
            (
                true,
                Some(Episode {
                    confirmed: true,
                    start,
                    ..
                }),
                _,
            ) => RoomView::Awake {
                since_unix_ms: start.unix,
            },
            (true, _, Some(since)) => RoomView::Quiet {
                since_unix_ms: since,
            },
            (true, _, None) => RoomView::Unknown,
        }
    }

    /// The alarm state.
    pub fn alarm(&self) -> AlarmView {
        match self.alarm {
            Alarm::Off => AlarmView::Off,
            Alarm::Pending { unix, .. } => AlarmView::Pending {
                since_unix_ms: unix,
            },
            Alarm::Ringing { unix } => AlarmView::Ringing {
                since_unix_ms: unix,
            },
        }
    }

    /// Until when sound alerts are paused (Unix ms).
    pub fn paused_until(&self) -> Option<u64> {
        self.paused_until.map(|p| p.unix)
    }

    /// Whether sound alerts are paused at `mono`.
    pub fn paused(&self, mono: u64) -> bool {
        self.paused_until.is_some_and(|p| mono < p.mono)
    }

    /// The wake-up going on, for the night log.
    pub fn ongoing(&self) -> Option<WakeUp> {
        self.episode
            .filter(|e| e.confirmed && self.live)
            .map(|e| Self::entry(&e, None, false))
    }

    fn entry(e: &Episode, end: Option<u64>, not_the_baby: bool) -> WakeUp {
        WakeUp {
            start_unix_ms: e.start.unix,
            end_unix_ms: end,
            loud_ms: e.loud_ms,
            peak_dbfs: e.peak,
            alerted: e.alerted,
            alarmed: e.alarmed,
            not_the_baby,
        }
    }

    /// A live session started.
    pub fn on_up(&mut self, at: At, limits: &RoomLimits) -> RoomOutput {
        let long_outage = self
            .down_at
            .is_none_or(|d| at.mono.saturating_sub(d) > limits.outage_reset_ms);
        if self.quiet_since.is_none() || long_outage {
            self.quiet_since = Some(at.unix);
        }
        self.down_at = None;
        self.live = true;
        self.last_sample = None;
        RoomOutput {
            changed: true,
            ..RoomOutput::default()
        }
    }

    /// The live session ended (FR-SL-2: so does a wake-up).
    pub fn on_down(&mut self, at: At) -> RoomOutput {
        if !self.live {
            return RoomOutput::default();
        }
        let mut out = self.finish();
        self.live = false;
        self.down_at = Some(at.mono);
        self.last_sample = None;
        out.changed = true;
        out
    }

    /// A sound report: `loud` is the sound signal of FR-AL-1 / CORE-5.
    pub fn on_sample(
        &mut self,
        loud: bool,
        dbfs: Option<f32>,
        at: At,
        limits: &RoomLimits,
    ) -> RoomOutput {
        let mut out = RoomOutput::default();
        if !self.live || self.paused(at.mono) {
            self.last_sample = None;
            return out;
        }
        let previous = self.last_sample.replace((at.mono, loud));
        if !loud {
            return out;
        }
        let gap = match previous {
            Some((t, true)) => at.mono.saturating_sub(t).min(limits.max_gap_ms),
            _ => 0,
        };
        self.last_loud = Some(at);
        let e = self.episode.get_or_insert(Episode {
            start: at,
            loud_ms: 0,
            peak: None,
            confirmed: false,
            alerted: false,
            alarmed: false,
            quiet_before: None,
        });
        e.loud_ms += gap;
        if let Some(d) = dbfs {
            e.peak = Some(e.peak.map_or(d, |p| p.max(d)));
        }
        if !e.confirmed && e.loud_ms >= limits.awake_loud_ms {
            e.confirmed = true;
            e.quiet_before = self.quiet_since;
            out.changed = true;
        }
        out
    }

    /// A sound alert went out (FR-AL-1); with `escalate` it may become the
    /// alarm (FR-AL-5).
    pub fn on_sound_alert(&mut self, escalate: bool, at: At) -> RoomOutput {
        if let Some(e) = self.episode.as_mut() {
            e.alerted = true;
        }
        if escalate && !self.acknowledged && self.alarm == Alarm::Off {
            self.alarm = Alarm::Pending {
                mono: at.mono,
                unix: at.unix,
            };
            return RoomOutput {
                changed: true,
                ..RoomOutput::default()
            };
        }
        RoomOutput::default()
    }

    /// Periodic work: escalation, end of a wake-up, end of the pause.
    pub fn tick(
        &mut self,
        escalate: Option<u64>,
        at: At,
        limits: &RoomLimits,
    ) -> RoomOutput {
        let mut out = RoomOutput::default();
        if self.paused_until.is_some_and(|p| at.mono >= p.mono) {
            self.paused_until = None;
            out.changed = true;
        }
        let since_loud = self.last_loud.map(|l| at.mono.saturating_sub(l.mono));
        if self.episode.is_some() && since_loud.is_none_or(|q| q >= limits.quiet_gap_ms) {
            let finished = self.finish();
            out.finished = finished.finished;
            out.changed = true;
        }
        match (self.alarm, escalate) {
            (Alarm::Pending { .. }, None) => {
                self.alarm = Alarm::Off;
                out.changed = true;
            }
            (Alarm::Pending { mono, .. }, Some(after))
                if at.mono.saturating_sub(mono) >= after
                    && since_loud.is_some_and(|q| q <= limits.loud_recent_ms) =>
            {
                self.alarm = Alarm::Ringing { unix: at.unix };
                if let Some(e) = self.episode.as_mut() {
                    e.alarmed = true;
                }
                out.changed = true;
                out.ringing = true;
            }
            _ => {}
        }
        out
    }

    /// Ends the current wake-up (or drops a sound that never became one),
    /// stops the alarm and re-arms acknowledging.
    fn finish(&mut self) -> RoomOutput {
        let mut out = RoomOutput::default();
        if let Some(e) = self.episode.take() {
            if e.confirmed {
                let end = self.last_loud.map_or(e.start.unix, |l| l.unix);
                self.quiet_since = Some(end);
                self.last_finished = Some((e.start.unix, e.quiet_before));
                out.finished = Some(Self::entry(&e, Some(end), false));
            }
        }
        if self.alarm != Alarm::Off {
            self.alarm = Alarm::Off;
        }
        self.acknowledged = false;
        self.last_sample = None;
        out.changed = true;
        out
    }

    /// "I'm on it" (FR-AL-6).
    pub fn acknowledge(&mut self) -> RoomOutput {
        let changed = self.alarm != Alarm::Off;
        self.alarm = Alarm::Off;
        if self.episode.is_some() {
            self.acknowledged = true;
        }
        RoomOutput {
            changed,
            ..RoomOutput::default()
        }
    }

    /// "Not the baby" (FR-AL-7). Returns the marked wake-up: the one going
    /// on (now finished), or the start of the last finished one to mark in
    /// the log.
    pub fn not_the_baby(&mut self, at: At, limits: &RoomLimits) -> (RoomOutput, Option<u64>) {
        let mut out = self.acknowledge();
        let mut mark = None;
        match self.episode.take() {
            Some(e) if e.confirmed => {
                let end = self.last_loud.map_or(e.start.unix, |l| l.unix);
                self.quiet_since = e.quiet_before.or(self.quiet_since);
                out.finished = Some(Self::entry(&e, Some(end), true));
            }
            Some(_) => {}
            None => {
                if let Some((start, before)) = self.last_finished.take() {
                    self.quiet_since = before.or(self.quiet_since);
                    mark = Some(start);
                }
            }
        }
        self.acknowledged = false;
        self.last_sample = None;
        self.paused_until = Some(At {
            mono: at.mono + limits.pause_ms,
            unix: at.unix + limits.pause_ms,
        });
        out.changed = true;
        (out, mark)
    }

    /// "Resume alerts" (FR-AL-7).
    pub fn resume(&mut self) -> RoomOutput {
        RoomOutput {
            changed: self.paused_until.take().is_some(),
            ..RoomOutput::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const L: RoomLimits = RoomLimits {
        awake_loud_ms: 10_000,
        quiet_gap_ms: 120_000,
        loud_recent_ms: 15_000,
        outage_reset_ms: 60_000,
        pause_ms: 600_000,
        max_gap_ms: 1_000,
    };

    fn at(mono: u64) -> At {
        At {
            mono,
            unix: 1_700_000_000_000 + mono,
        }
    }

    fn u(mono: u64) -> u64 {
        at(mono).unix
    }

    /// Loud reports every 250 ms from `from` for `ms`.
    fn loud(r: &mut Room, from: u64, ms: u64) {
        let mut t = from;
        while t <= from + ms {
            r.on_sample(true, Some(-40.0), at(t), &L);
            t += 250;
        }
    }

    fn live() -> Room {
        let mut r = Room::default();
        r.on_up(at(0), &L);
        r
    }

    #[test]
    fn fr_sl_3_unknown_until_live_then_quiet() {
        let mut r = Room::default();
        assert_eq!(r.view(), RoomView::Unknown);
        r.on_up(at(1_000), &L);
        assert_eq!(r.view(), RoomView::Quiet { since_unix_ms: u(1_000) });
    }

    #[test]
    fn fr_sl_1_short_sound_is_not_a_wake_up() {
        let mut r = live();
        loud(&mut r, 1_000, 5_000);
        r.on_sample(false, Some(-70.0), at(6_250), &L);
        assert_eq!(r.view(), RoomView::Quiet { since_unix_ms: u(0) });
        let out = r.tick(None, at(130_000), &L);
        assert_eq!(out.finished, None);
        assert_eq!(r.view(), RoomView::Quiet { since_unix_ms: u(0) });
    }

    #[test]
    fn fr_sl_1_2_wake_up_counts_after_10_s_and_ends_after_quiet_gap() {
        let mut r = live();
        loud(&mut r, 10_000, 9_000);
        assert!(matches!(r.view(), RoomView::Quiet { .. }));
        loud(&mut r, 19_250, 2_000);
        assert_eq!(r.view(), RoomView::Awake { since_unix_ms: u(10_000) });
        assert!(r.ongoing().is_some());
        assert_eq!(r.tick(None, at(100_000), &L).finished, None);
        let out = r.tick(None, at(21_250 + 120_000), &L);
        let w = out.finished.expect("finished");
        assert_eq!(w.start_unix_ms, u(10_000));
        assert_eq!(w.end_unix_ms, Some(u(21_250)));
        assert!(w.loud_ms >= 10_000);
        assert_eq!(w.peak_dbfs, Some(-40.0));
        assert_eq!(r.view(), RoomView::Quiet { since_unix_ms: u(21_250) });
    }

    #[test]
    fn sl_1_loud_time_counts_at_most_1_s_per_gap() {
        let mut r = live();
        r.on_sample(true, None, at(1_000), &L);
        r.on_sample(true, None, at(60_000), &L);
        assert!(matches!(r.view(), RoomView::Quiet { .. }));
    }

    #[test]
    fn fr_sl_2_session_end_ends_the_wake_up() {
        let mut r = live();
        loud(&mut r, 1_000, 12_000);
        let out = r.on_down(at(20_000));
        assert_eq!(out.finished.unwrap().end_unix_ms, Some(u(13_000)));
        assert_eq!(r.view(), RoomView::Unknown);
        // Short outage keeps "quiet since".
        r.on_up(at(50_000), &L);
        assert_eq!(r.view(), RoomView::Quiet { since_unix_ms: u(13_000) });
        // A long one restarts it.
        r.on_down(at(60_000));
        r.on_up(at(200_000), &L);
        assert_eq!(r.view(), RoomView::Quiet { since_unix_ms: u(200_000) });
    }

    #[test]
    fn fr_al_5_escalates_while_still_loud() {
        let mut r = live();
        loud(&mut r, 1_000, 2_000);
        r.on_sound_alert(true, at(1_000));
        assert_eq!(r.alarm(), AlarmView::Pending { since_unix_ms: u(1_000) });
        loud(&mut r, 3_250, 58_000);
        let out = r.tick(Some(60_000), at(61_000), &L);
        assert!(out.ringing);
        assert_eq!(r.alarm(), AlarmView::Ringing { since_unix_ms: u(61_000) });
        assert!(r.ongoing().unwrap().alarmed);
        // Rings until the room has been quiet for the quiet gap.
        assert!(!r.tick(Some(60_000), at(100_000), &L).ringing);
        assert!(matches!(r.alarm(), AlarmView::Ringing { .. }));
        let out = r.tick(Some(60_000), at(61_250 + 120_000), &L);
        assert!(out.finished.unwrap().alarmed);
        assert_eq!(r.alarm(), AlarmView::Off);
    }

    #[test]
    fn fr_al_5_no_escalation_when_the_room_went_quiet() {
        let mut r = live();
        loud(&mut r, 1_000, 2_000);
        r.on_sound_alert(true, at(1_000));
        assert!(!r.tick(Some(60_000), at(61_000), &L).ringing);
        assert!(matches!(r.alarm(), AlarmView::Pending { .. }));
        // Sound again later: rings then.
        loud(&mut r, 70_000, 1_000);
        assert!(r.tick(Some(60_000), at(71_100), &L).ringing);
    }

    #[test]
    fn fr_al_5_off_when_escalation_disabled() {
        let mut r = live();
        loud(&mut r, 1_000, 2_000);
        r.on_sound_alert(false, at(1_000));
        assert_eq!(r.alarm(), AlarmView::Off);
        r.on_sound_alert(true, at(1_000));
        r.tick(None, at(2_000), &L);
        assert_eq!(r.alarm(), AlarmView::Off, "turned off meanwhile");
    }

    #[test]
    fn fr_al_6_acknowledge_stops_and_prevents_escalation_until_wake_up_ends() {
        let mut r = live();
        loud(&mut r, 1_000, 70_000);
        r.on_sound_alert(true, at(1_000));
        assert!(r.tick(Some(60_000), at(61_000), &L).ringing);
        assert!(r.acknowledge().changed);
        assert_eq!(r.alarm(), AlarmView::Off);
        r.on_sound_alert(true, at(65_000));
        assert_eq!(r.alarm(), AlarmView::Off);
        // Next wake-up escalates again.
        r.tick(Some(60_000), at(71_000 + 120_000), &L);
        loud(&mut r, 300_000, 1_000);
        r.on_sound_alert(true, at(300_000));
        assert!(matches!(r.alarm(), AlarmView::Pending { .. }));
    }

    #[test]
    fn fr_al_7_not_the_baby_restores_quiet_marks_and_pauses() {
        let mut r = live();
        loud(&mut r, 5_000, 70_000);
        r.on_sound_alert(true, at(5_000));
        r.tick(Some(60_000), at(65_000), &L);
        let (out, mark) = r.not_the_baby(at(75_000), &L);
        assert_eq!(mark, None);
        let w = out.finished.unwrap();
        assert!(w.not_the_baby && w.alarmed);
        assert_eq!(r.alarm(), AlarmView::Off);
        assert_eq!(r.view(), RoomView::Quiet { since_unix_ms: u(0) });
        assert_eq!(r.paused_until(), Some(u(675_000)));
        assert!(r.paused(100_000));
        // Sounds during the pause are ignored.
        loud(&mut r, 100_000, 20_000);
        assert_eq!(r.view(), RoomView::Quiet { since_unix_ms: u(0) });
        assert!(r.tick(None, at(675_000), &L).changed);
        assert_eq!(r.paused_until(), None);
    }

    #[test]
    fn fr_al_7_not_the_baby_after_the_wake_up_marks_the_last_one() {
        let mut r = live();
        loud(&mut r, 5_000, 12_000);
        r.tick(None, at(17_000 + 120_000), &L);
        let (out, mark) = r.not_the_baby(at(200_000), &L);
        assert_eq!(out.finished, None);
        assert_eq!(mark, Some(u(5_000)));
        assert_eq!(r.view(), RoomView::Quiet { since_unix_ms: u(0) });
        assert!(r.resume().changed);
        assert!(!r.paused(200_001));
    }
}
