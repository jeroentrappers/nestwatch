// SPDX-License-Identifier: AGPL-3.0-or-later
//! Linux sleep inhibition through a `systemd-inhibit` child process.
//!
//! The child runs `cat` on a pipe from us, so the lock is released when the
//! inhibition is dropped and also if this process dies.

use std::process::{Child, Command, Stdio};
use std::time::Duration;

use super::{PowerGuard, SleepInhibition};
use crate::process::{find_program, kill};
use crate::{APP_NAME, PlatformError};

/// The `systemd-inhibit` backend.
#[derive(Debug, Clone)]
pub struct SystemdInhibit {
    program: std::path::PathBuf,
}

impl SystemdInhibit {
    /// Finds `systemd-inhibit` on `PATH`.
    pub fn detect() -> Result<Self, PlatformError> {
        find_program("systemd-inhibit")
            .map(|program| Self { program })
            .ok_or_else(|| PlatformError::Unavailable("systemd-inhibit not installed".into()))
    }
}

struct InhibitChild(Child);

impl Drop for InhibitChild {
    fn drop(&mut self) {
        // Closing stdin ends `cat`; kill in case it lingers.
        drop(self.0.stdin.take());
        kill(&mut self.0);
    }
}

impl PowerGuard for SystemdInhibit {
    fn backend(&self) -> &'static str {
        "systemd-inhibit"
    }

    fn inhibit_sleep(&self, reason: &str) -> Result<SleepInhibition, PlatformError> {
        let mut child = Command::new(&self.program)
            .arg("--what=sleep:idle")
            .arg(format!("--who={APP_NAME}"))
            .arg(format!("--why={reason}"))
            .arg("--mode=block")
            .arg("cat")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| PlatformError::Unavailable(format!("systemd-inhibit: {e}")))?;
        // A refused lock makes systemd-inhibit exit at once.
        std::thread::sleep(Duration::from_millis(150));
        if let Ok(Some(status)) = child.try_wait() {
            let mut msg = String::new();
            if let Some(mut e) = child.stderr.take() {
                let _ = std::io::Read::read_to_string(&mut e, &mut msg);
            }
            return Err(PlatformError::from_message(format!(
                "systemd-inhibit exited with {status}: {}",
                msg.trim()
            )));
        }
        Ok(SleepInhibition::new("systemd-inhibit", InhibitChild(child)))
    }
}
