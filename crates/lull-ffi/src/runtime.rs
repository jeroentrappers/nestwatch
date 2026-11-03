// SPDX-License-Identifier: AGPL-3.0-or-later
//! The crate's own tokio runtime and helpers to run engine futures on it
//! from foreign async calls.

use std::future::Future;
use std::sync::OnceLock;
use std::time::Duration;

use tokio::runtime::Runtime;

/// Shared runtime for all engines of the process.
pub(crate) fn rt() -> &'static Runtime {
    static RT: OnceLock<Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(3)
            .thread_name("lull-rt")
            .enable_all()
            .build()
            .expect("tokio runtime")
    })
}

/// Runs `fut` on the runtime and awaits its output from any executor (the
/// foreign language's: UniFFI polls exported futures from there).
pub(crate) async fn on_rt<F>(fut: F) -> F::Output
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    match rt().spawn(fut).await {
        Ok(v) => v,
        Err(e) => std::panic::resume_unwind(e.into_panic()),
    }
}

/// Minimum time between two state snapshots sent to an observer (≤ 10 Hz,
/// architecture.md §5).
pub(crate) const STATE_INTERVAL: Duration = Duration::from_millis(100);

/// Heartbeat for countdowns (pairing window).
pub(crate) const HEARTBEAT: Duration = Duration::from_secs(1);
