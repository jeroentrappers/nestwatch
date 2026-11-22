// SPDX-License-Identifier: AGPL-3.0-or-later
//! The night log of wake-ups per station (product.md FR-SL-4, SL-1),
//! stored under `listener-log`.

use std::collections::BTreeMap;

use minicbor::{Decode, Encode};

use super::room::WakeUp;
use crate::persist;
use crate::storage::{Storage, StorageError};

/// Storage key of the night log.
pub const KEY_LOG: &str = "listener-log";

/// Wake-ups kept per station at most (FR-SL-4).
pub(crate) const MAX_ENTRIES: usize = 500;
/// Age of the oldest kept wake-up (FR-SL-4: 30 days).
pub(crate) const MAX_AGE_MS: u64 = 30 * 24 * 60 * 60 * 1000;

#[derive(Debug, Clone, Default, Encode, Decode)]
struct LogRecord {
    #[n(0)]
    stations: Vec<StationLog>,
}

#[derive(Debug, Clone, Encode, Decode)]
struct StationLog {
    #[n(0)]
    id: String,
    #[n(1)]
    entries: Vec<EntryRecord>,
}

#[derive(Debug, Clone, Encode, Decode)]
struct EntryRecord {
    #[n(0)]
    start: u64,
    #[n(1)]
    end: u64,
    #[n(2)]
    loud_ms: u64,
    #[n(3)]
    peak_dbfs: Option<f32>,
    #[n(4)]
    alerted: bool,
    #[n(5)]
    alarmed: bool,
    #[n(6)]
    not_the_baby: bool,
}

impl From<&WakeUp> for EntryRecord {
    fn from(w: &WakeUp) -> Self {
        Self {
            start: w.start_unix_ms,
            end: w.end_unix_ms.unwrap_or(w.start_unix_ms),
            loud_ms: w.loud_ms,
            peak_dbfs: w.peak_dbfs,
            alerted: w.alerted,
            alarmed: w.alarmed,
            not_the_baby: w.not_the_baby,
        }
    }
}

impl From<&EntryRecord> for WakeUp {
    fn from(e: &EntryRecord) -> Self {
        Self {
            start_unix_ms: e.start,
            end_unix_ms: Some(e.end),
            loud_ms: e.loud_ms,
            peak_dbfs: e.peak_dbfs,
            alerted: e.alerted,
            alarmed: e.alarmed,
            not_the_baby: e.not_the_baby,
        }
    }
}

/// The night logs of all stations, oldest first per station.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct NightLog {
    stations: BTreeMap<String, Vec<WakeUp>>,
}

impl NightLog {
    /// Loads the log; a corrupt log is an error (CORE-1).
    pub fn load(storage: &dyn Storage) -> Result<Self, StorageError> {
        let rec = persist::load::<LogRecord>(storage, KEY_LOG)?.unwrap_or_default();
        Ok(Self {
            stations: rec
                .stations
                .iter()
                .map(|s| (s.id.clone(), s.entries.iter().map(WakeUp::from).collect()))
                .collect(),
        })
    }

    /// Stores the log.
    pub fn save(&self, storage: &dyn Storage) -> Result<(), StorageError> {
        let rec = LogRecord {
            stations: self
                .stations
                .iter()
                .filter(|(_, e)| !e.is_empty())
                .map(|(id, e)| StationLog {
                    id: id.clone(),
                    entries: e.iter().map(EntryRecord::from).collect(),
                })
                .collect(),
        };
        persist::save(storage, KEY_LOG, &rec)
    }

    /// Wake-ups of `id`, oldest first.
    pub fn entries(&self, id: &str) -> &[WakeUp] {
        self.stations.get(id).map_or(&[], Vec::as_slice)
    }

    /// Adds a finished wake-up and drops what is too old or too many.
    pub fn push(&mut self, id: &str, w: WakeUp, now_unix: u64) {
        let list = self.stations.entry(id.to_owned()).or_default();
        list.push(w);
        list.sort_by_key(|e| e.start_unix_ms);
        let oldest = now_unix.saturating_sub(MAX_AGE_MS);
        list.retain(|e| e.end_unix_ms.unwrap_or(e.start_unix_ms) >= oldest);
        if list.len() > MAX_ENTRIES {
            let extra = list.len() - MAX_ENTRIES;
            list.drain(..extra);
        }
    }

    /// Marks the wake-up of `id` that started at `start` as "Not the baby".
    pub fn mark_not_the_baby(&mut self, id: &str, start: u64) -> bool {
        let Some(e) = self
            .stations
            .get_mut(id)
            .and_then(|l| l.iter_mut().find(|e| e.start_unix_ms == start))
        else {
            return false;
        };
        e.not_the_baby = true;
        true
    }

    /// Clears the log of `id`.
    pub fn clear(&mut self, id: &str) -> bool {
        self.stations.remove(id).is_some_and(|l| !l.is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::MemoryStorage;

    fn w(start: u64) -> WakeUp {
        WakeUp {
            start_unix_ms: start,
            end_unix_ms: Some(start + 60_000),
            loud_ms: 12_000,
            peak_dbfs: Some(-30.0),
            alerted: true,
            alarmed: false,
            not_the_baby: false,
        }
    }

    #[test]
    fn fr_sl_4_round_trip_mark_clear() {
        let s = MemoryStorage::new();
        let mut log = NightLog::load(&s).unwrap();
        let now = 100 * 24 * 3_600_000;
        log.push("a", w(now - 10_000_000), now);
        log.push("a", w(now - 20_000_000), now);
        log.push("b", w(now - 5_000_000), now);
        assert!(log.mark_not_the_baby("a", now - 10_000_000));
        log.save(&s).unwrap();
        let back = NightLog::load(&s).unwrap();
        assert_eq!(back, log);
        assert_eq!(back.entries("a")[0].start_unix_ms, now - 20_000_000);
        assert!(back.entries("a")[1].not_the_baby);
        let mut back = back;
        assert!(back.clear("a"));
        assert!(back.entries("a").is_empty());
        assert_eq!(back.entries("b").len(), 1);
    }

    #[test]
    fn fr_sl_4_keeps_30_days_and_500_entries() {
        let mut log = NightLog::default();
        let now = 100 * 24 * 3_600_000;
        log.push("a", w(now - MAX_AGE_MS - 120_000), now);
        assert!(log.entries("a").is_empty());
        for i in 0..(MAX_ENTRIES as u64 + 5) {
            log.push("a", w(now - 1_000_000_000 + i * 1_000), now);
        }
        assert_eq!(log.entries("a").len(), MAX_ENTRIES);
        assert_eq!(
            log.entries("a")[0].start_unix_ms,
            now - 1_000_000_000 + 5_000
        );
    }
}
