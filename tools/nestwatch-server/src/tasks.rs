// SPDX-License-Identifier: AGPL-3.0-or-later
//! Tracks spawned connection tasks so shutdown can wait for them.

use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use tokio::sync::Notify;

#[derive(Debug, Default)]
struct Inner {
    active: AtomicUsize,
    idle: Notify,
}

/// A cloneable set of running tasks.
#[derive(Debug, Clone, Default)]
pub(crate) struct Tasks(Arc<Inner>);

struct Guard(Arc<Inner>);

impl Drop for Guard {
    fn drop(&mut self) {
        if self.0.active.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.0.idle.notify_waiters();
        }
    }
}

impl Tasks {
    /// Spawns `f` on the current runtime and counts it until it ends.
    pub(crate) fn spawn<F>(&self, f: F)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        self.0.active.fetch_add(1, Ordering::AcqRel);
        let guard = Guard(self.0.clone());
        tokio::spawn(async move {
            let _guard = guard;
            f.await;
        });
    }

    /// Waits until no task runs, at most `limit`.
    pub(crate) async fn wait_idle(&self, limit: Duration) {
        let _ = tokio::time::timeout(limit, async {
            loop {
                let notified = self.0.idle.notified();
                if self.0.active.load(Ordering::Acquire) == 0 {
                    return;
                }
                notified.await;
            }
        })
        .await;
    }
}

/// Resolves once `stop` is set. (`wait_for`'s guard is not `Send`, so it
/// must not live in a `select!` output.)
pub(crate) async fn stopped(stop: &mut tokio::sync::watch::Receiver<bool>) {
    let _ = stop.wait_for(|s| *s).await;
}
