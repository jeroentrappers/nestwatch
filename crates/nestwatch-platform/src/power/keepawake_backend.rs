// SPDX-License-Identifier: AGPL-3.0-or-later
//! Sleep inhibition through `keepawake`: logind inhibitor locks on Linux,
//! `IOPMAssertion` on macOS, `SetThreadExecutionState` on Windows.
//!
//! `SetThreadExecutionState` is per thread, so each inhibition is held by a
//! dedicated thread that lives until the inhibition is dropped.

use std::sync::mpsc;

use super::{PowerGuard, SleepInhibition};
use crate::{APP_ID, APP_NAME, PlatformError};

/// The keepawake backend.
#[derive(Debug, Clone, Copy, Default)]
pub struct KeepAwakeGuard;

/// Dropping the sender ends the holder thread, which releases the lock.
struct Holder(#[allow(dead_code)] mpsc::Sender<()>);

impl PowerGuard for KeepAwakeGuard {
    fn backend(&self) -> &'static str {
        "keepawake"
    }

    fn inhibit_sleep(&self, reason: &str) -> Result<SleepInhibition, PlatformError> {
        let reason = reason.to_owned();
        let (ready_tx, ready_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel::<()>();
        std::thread::Builder::new()
            .name("nestwatch-keepawake".into())
            .spawn(move || {
                let awake = keepawake::Builder::default()
                    .display(false)
                    .idle(true)
                    .sleep(true)
                    .reason(reason)
                    .app_name(APP_NAME)
                    .app_reverse_domain(APP_ID)
                    .create();
                match awake {
                    Ok(awake) => {
                        let _ = ready_tx.send(Ok(()));
                        let _ = release_rx.recv();
                        drop(awake);
                    }
                    Err(e) => {
                        let _ = ready_tx.send(Err(PlatformError::from_message(e.to_string())));
                    }
                }
            })
            .map_err(PlatformError::Io)?;
        ready_rx
            .recv()
            .map_err(|_| PlatformError::Backend("keepawake thread exited".into()))??;
        Ok(SleepInhibition::new("keepawake", Holder(release_tx)))
    }
}
