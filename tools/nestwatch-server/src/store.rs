// SPDX-License-Identifier: AGPL-3.0-or-later
//! Persistent state (rendezvous.md §6): per mailbox only the secret hash
//! (TOFU, §3.2) and the monthly transfer counter. Everything else is in
//! memory.
//!
//! The file is keyed by `SHA-256("nestwatch-server/1 mailbox" || id)`, so it
//! never contains mailbox IDs themselves (OPEN-QUESTIONS RELAY-9). It is
//! written atomically (temporary file, fsync, rename, fsync of the
//! directory), immediately when a mailbox is first claimed and otherwise
//! when [`Store::flush`] runs.

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use nestwatch_relay::MailboxId;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// File name inside the state directory.
pub const STATE_FILE: &str = "mailboxes.json";
const FORMAT_VERSION: u32 = 1;
const KEY_DOMAIN: &[u8] = b"nestwatch-server/1 mailbox";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct Entry {
    /// Hex `SHA-256(mailbox_secret)`.
    secret_hash: String,
    /// Days since the Unix epoch of the last registration.
    last_seen_day: u64,
    /// `YYYY-MM` of `bytes`.
    month: String,
    /// Bytes relayed for this mailbox in `month`.
    bytes: u64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Data {
    version: u32,
    mailboxes: BTreeMap<String, Entry>,
}

/// Result of [`Store::check_or_claim`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Claim {
    /// The secret matches the stored hash.
    Known,
    /// First registration: the hash is now stored.
    Claimed,
    /// A different secret owns this mailbox.
    Taken,
}

/// The persistent mailbox table.
#[derive(Debug)]
pub struct Store {
    path: PathBuf,
    inner: Mutex<(Data, bool)>,
    retention_days: u64,
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn key(id: &MailboxId) -> String {
    let mut h = Sha256::new();
    h.update(KEY_DOMAIN);
    h.update(id.as_bytes());
    hex(&h.finalize())
}

/// Days since the Unix epoch and `YYYY-MM`, in UTC.
pub(crate) fn today() -> (u64, String) {
    let now = time::OffsetDateTime::now_utc();
    let day = (now.unix_timestamp().max(0) as u64) / 86_400;
    (
        day,
        format!("{:04}-{:02}", now.year(), u8::from(now.month())),
    )
}

fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Writes `bytes` to `path` atomically.
fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    let tmp = path.with_extension("json.tmp");
    {
        let mut opts = fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut f = opts.open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)?;
    #[cfg(unix)]
    if let Ok(d) = fs::File::open(dir) {
        let _ = d.sync_all();
    }
    #[cfg(not(unix))]
    let _ = dir;
    Ok(())
}

impl Store {
    /// Opens (or creates) the state in `dir`. A corrupt file is an error:
    /// starting empty would let anyone take over every mailbox.
    pub fn open(dir: &Path, retention_days: u64) -> io::Result<Self> {
        fs::create_dir_all(dir)?;
        let path = dir.join(STATE_FILE);
        let mut data = match fs::read(&path) {
            Ok(b) => serde_json::from_slice::<Data>(&b)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => Data {
                version: FORMAT_VERSION,
                mailboxes: BTreeMap::new(),
            },
            Err(e) => return Err(e),
        };
        if data.version != FORMAT_VERSION {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unknown state format {}", data.version),
            ));
        }
        let (day, _) = today();
        let before = data.mailboxes.len();
        data.mailboxes
            .retain(|_, e| day.saturating_sub(e.last_seen_day) <= retention_days);
        let dirty = data.mailboxes.len() != before;
        let store = Self {
            path,
            inner: Mutex::new((data, dirty)),
            retention_days,
        };
        store.flush()?;
        Ok(store)
    }

    /// Number of stored mailboxes.
    pub fn len(&self) -> usize {
        self.inner.lock().expect("not poisoned").0.mailboxes.len()
    }

    /// Whether no mailbox is stored.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// TOFU check (§3.2): compares `secret_hash` with the stored hash, or
    /// stores it on first use and writes the file before returning (a
    /// restart must not let the mailbox be claimed again).
    pub fn check_or_claim(&self, id: &MailboxId, secret_hash: &[u8; 32]) -> io::Result<Claim> {
        let (day, month) = today();
        let k = key(id);
        let mut g = self.inner.lock().expect("not poisoned");
        if let Some(e) = g.0.mailboxes.get_mut(&k) {
            if !ct_eq(e.secret_hash.as_bytes(), hex(secret_hash).as_bytes()) {
                return Ok(Claim::Taken);
            }
            if e.last_seen_day != day {
                e.last_seen_day = day;
                g.1 = true;
            }
            return Ok(Claim::Known);
        }
        g.0.mailboxes.insert(
            k.clone(),
            Entry {
                secret_hash: hex(secret_hash),
                last_seen_day: day,
                month,
                bytes: 0,
            },
        );
        let bytes = serde_json::to_vec_pretty(&g.0).map_err(io::Error::other)?;
        if let Err(e) = write_atomic(&self.path, &bytes) {
            // Not persisted: do not accept the claim.
            g.0.mailboxes.remove(&k);
            return Err(e);
        }
        Ok(Claim::Claimed)
    }

    /// Adds `n` relayed bytes to the mailbox's monthly counter and returns
    /// the new total for the current month.
    pub fn add_transfer(&self, id: &MailboxId, n: u64) -> u64 {
        let (_, month) = today();
        let mut g = self.inner.lock().expect("not poisoned");
        let Some(e) = g.0.mailboxes.get_mut(&key(id)) else {
            return 0;
        };
        if e.month != month {
            e.month = month;
            e.bytes = 0;
        }
        e.bytes = e.bytes.saturating_add(n);
        let total = e.bytes;
        g.1 = true;
        total
    }

    /// Bytes relayed for the mailbox this month.
    pub fn transfer(&self, id: &MailboxId) -> u64 {
        let (_, month) = today();
        let g = self.inner.lock().expect("not poisoned");
        g.0.mailboxes
            .get(&key(id))
            .filter(|e| e.month == month)
            .map_or(0, |e| e.bytes)
    }

    /// Writes the file if anything changed since the last write, dropping
    /// mailboxes unseen for longer than the retention period.
    pub fn flush(&self) -> io::Result<()> {
        let (day, _) = today();
        let mut g = self.inner.lock().expect("not poisoned");
        let before = g.0.mailboxes.len();
        let keep = self.retention_days;
        g.0.mailboxes
            .retain(|_, e| day.saturating_sub(e.last_seen_day) <= keep);
        if g.0.mailboxes.len() != before {
            g.1 = true;
        }
        if !g.1 && self.path.exists() {
            return Ok(());
        }
        let bytes = serde_json::to_vec_pretty(&g.0).map_err(io::Error::other)?;
        write_atomic(&self.path, &bytes)?;
        g.1 = false;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u8) -> MailboxId {
        MailboxId([n; 16])
    }

    #[test]
    fn tofu_survives_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::open(dir.path(), 400).unwrap();
        assert_eq!(s.check_or_claim(&id(1), &[7; 32]).unwrap(), Claim::Claimed);
        assert_eq!(s.check_or_claim(&id(1), &[7; 32]).unwrap(), Claim::Known);
        assert_eq!(s.check_or_claim(&id(1), &[8; 32]).unwrap(), Claim::Taken);
        drop(s);
        // Claimed hashes are on disk before the claim returns.
        let s = Store::open(dir.path(), 400).unwrap();
        assert_eq!(s.check_or_claim(&id(1), &[8; 32]).unwrap(), Claim::Taken);
        assert_eq!(s.check_or_claim(&id(1), &[7; 32]).unwrap(), Claim::Known);
        assert_eq!(s.check_or_claim(&id(2), &[8; 32]).unwrap(), Claim::Claimed);
        assert_eq!(s.len(), 2);
    }

    #[test]
    fn file_holds_no_mailbox_ids_or_secrets() {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::open(dir.path(), 400).unwrap();
        let mailbox = MailboxId(*b"0123456789abcdef");
        s.check_or_claim(&mailbox, &nestwatch_relay::secret_hash(&[0x41; 32]))
            .unwrap();
        let text = fs::read_to_string(dir.path().join(STATE_FILE)).unwrap();
        assert!(!text.contains("0123456789abcdef"));
        assert!(!text.contains(&hex(mailbox.as_bytes())));
        assert!(!text.contains(&mailbox.to_text()));
        assert!(!text.contains(&hex(&[0x41; 32])));
        assert!(!dir.path().join("mailboxes.json.tmp").exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(dir.path().join(STATE_FILE))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o077, 0);
        }
    }

    #[test]
    fn transfer_counters_persist_on_flush() {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::open(dir.path(), 400).unwrap();
        s.check_or_claim(&id(3), &[1; 32]).unwrap();
        assert_eq!(s.add_transfer(&id(3), 1000), 1000);
        assert_eq!(s.add_transfer(&id(3), 24), 1024);
        assert_eq!(
            s.add_transfer(&id(4), 5),
            0,
            "unknown mailboxes are not counted"
        );
        s.flush().unwrap();
        let s = Store::open(dir.path(), 400).unwrap();
        assert_eq!(s.transfer(&id(3)), 1024);
    }

    #[test]
    fn corrupt_state_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(STATE_FILE), b"{ not json").unwrap();
        assert!(Store::open(dir.path(), 400).is_err());
        fs::write(
            dir.path().join(STATE_FILE),
            br#"{"version": 9, "mailboxes": {}}"#,
        )
        .unwrap();
        assert!(Store::open(dir.path(), 400).is_err());
    }

    #[test]
    fn old_mailboxes_are_forgotten() {
        let dir = tempfile::tempdir().unwrap();
        let (day, month) = today();
        let mut data = Data {
            version: FORMAT_VERSION,
            mailboxes: BTreeMap::new(),
        };
        for (n, age) in [(5u8, 10u64), (6, 500)] {
            data.mailboxes.insert(
                key(&id(n)),
                Entry {
                    secret_hash: hex(&[n; 32]),
                    last_seen_day: day - age,
                    month: month.clone(),
                    bytes: 0,
                },
            );
        }
        fs::write(
            dir.path().join(STATE_FILE),
            serde_json::to_vec(&data).unwrap(),
        )
        .unwrap();
        let s = Store::open(dir.path(), 400).unwrap();
        assert_eq!(s.len(), 1);
        assert_eq!(s.check_or_claim(&id(6), &[9; 32]).unwrap(), Claim::Claimed);
    }
}
