// SPDX-License-Identifier: AGPL-3.0-or-later
//! File-based secret store: one file per secret in a private directory.
//!
//! On Unix the directory is `0700` and files are `0600`. Writes are atomic:
//! a temporary file is written and synced, then renamed over the target and
//! the directory synced. Used on Linux when no Secret Service is available
//! (security.md §5) and as the container for [`super::sealed`] stores.

use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use super::{KeyStore, Secret, validate_name};
use crate::PlatformError;

const SUFFIX: &str = ".secret";

/// A directory of secret files.
#[derive(Debug)]
pub struct FileKeyStore {
    dir: PathBuf,
    counter: AtomicU64,
}

impl FileKeyStore {
    /// Opens (creating if needed) a store in `dir`.
    pub fn open(dir: impl Into<PathBuf>) -> Result<Self, PlatformError> {
        let dir = dir.into();
        create_private_dir(&dir)?;
        Ok(Self {
            dir,
            counter: AtomicU64::new(0),
        })
    }

    /// The per-user default location: `<data dir>/keys`, e.g.
    /// `$XDG_DATA_HOME/lull/keys` on Linux.
    pub fn default_dir() -> Result<PathBuf, PlatformError> {
        directories::ProjectDirs::from("org", "Lull", "Lull")
            .map(|d| d.data_dir().join("keys"))
            .ok_or_else(|| PlatformError::Unavailable("no home directory".into()))
    }

    /// Opens the store at [`Self::default_dir`].
    pub fn open_default() -> Result<Self, PlatformError> {
        Self::open(Self::default_dir()?)
    }

    /// The store directory.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.join(format!("{name}{SUFFIX}"))
    }
}

#[cfg(unix)]
fn create_private_dir(dir: &Path) -> Result<(), PlatformError> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)?;
    let meta = fs::metadata(dir)?;
    if meta.permissions().mode() & 0o077 != 0 {
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn create_private_dir(dir: &Path) -> Result<(), PlatformError> {
    fs::create_dir_all(dir)?;
    Ok(())
}

fn create_private_file(path: &Path) -> std::io::Result<File> {
    let mut opts = OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    opts.open(path)
}

fn sync_dir(dir: &Path) {
    #[cfg(unix)]
    if let Ok(d) = File::open(dir) {
        let _ = d.sync_all();
    }
    #[cfg(not(unix))]
    let _ = dir;
}

impl KeyStore for FileKeyStore {
    fn backend(&self) -> &'static str {
        "file"
    }

    fn get(&self, name: &str) -> Result<Option<Secret>, PlatformError> {
        validate_name(name)?;
        let mut file = match File::open(self.path(name)) {
            Ok(f) => f,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if file.metadata()?.permissions().mode() & 0o077 != 0 {
                log::warn!("secret file {name} is readable by others; restricting to 0600");
                file.set_permissions(fs::Permissions::from_mode(0o600))?;
            }
        }
        let mut buf = zeroize::Zeroizing::new(Vec::new());
        file.read_to_end(&mut buf)?;
        Ok(Some(Secret::new(std::mem::take(&mut *buf))))
    }

    fn put(&self, name: &str, value: &[u8]) -> Result<(), PlatformError> {
        validate_name(name)?;
        let n = self.counter.fetch_add(1, Ordering::Relaxed);
        let tmp = self
            .dir
            .join(format!(".{name}.{}.{n}.tmp", std::process::id()));
        let result = (|| {
            let mut f = create_private_file(&tmp)?;
            f.write_all(value)?;
            f.sync_all()?;
            drop(f);
            fs::rename(&tmp, self.path(name))
        })();
        if let Err(e) = result {
            let _ = fs::remove_file(&tmp);
            return Err(e.into());
        }
        sync_dir(&self.dir);
        Ok(())
    }

    fn delete(&self, name: &str) -> Result<bool, PlatformError> {
        validate_name(name)?;
        match fs::remove_file(self.path(name)) {
            Ok(()) => {
                sync_dir(&self.dir);
                Ok(true)
            }
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(false),
            Err(e) => Err(e.into()),
        }
    }
}

#[cfg(test)]
pub(crate) fn temp_dir(tag: &str) -> PathBuf {
    static N: AtomicU64 = AtomicU64::new(0);
    std::env::temp_dir().join(format!(
        "lull-platform-{tag}-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn behaves_like_a_key_store() {
        let dir = temp_dir("file");
        let store = FileKeyStore::open(&dir).unwrap();
        super::super::tests::exercise(&store);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn persists_across_instances_and_leaves_no_temp_files() {
        let dir = temp_dir("persist");
        FileKeyStore::open(&dir)
            .unwrap()
            .put("device-key", b"k1")
            .unwrap();
        let again = FileKeyStore::open(&dir).unwrap();
        assert_eq!(again.get("device-key").unwrap().unwrap().expose(), b"k1");
        let names: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(names, vec!["device-key.secret".to_owned()]);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn uses_private_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_dir("perm");
        let store = FileKeyStore::open(&dir).unwrap();
        store.put("device-key", b"k").unwrap();
        let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&dir), 0o700);
        assert_eq!(mode(&dir.join("device-key.secret")), 0o600);
        // A loosened file is tightened on read.
        fs::set_permissions(
            dir.join("device-key.secret"),
            fs::Permissions::from_mode(0o644),
        )
        .unwrap();
        store.get("device-key").unwrap();
        assert_eq!(mode(&dir.join("device-key.secret")), 0o600);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn concurrent_writers_never_leave_partial_files() {
        let dir = temp_dir("race");
        let store = std::sync::Arc::new(FileKeyStore::open(&dir).unwrap());
        let threads: Vec<_> = (0..8u8)
            .map(|i| {
                let s = std::sync::Arc::clone(&store);
                std::thread::spawn(move || {
                    for _ in 0..20 {
                        s.put("trust", &[i; 4096]).unwrap();
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        let v = store.get("trust").unwrap().unwrap();
        assert_eq!(v.len(), 4096);
        assert!(v.expose().iter().all(|&b| b == v.expose()[0]));
        fs::remove_dir_all(&dir).unwrap();
    }
}
