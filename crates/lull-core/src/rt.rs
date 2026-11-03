// SPDX-License-Identifier: AGPL-3.0-or-later
//! The few runtime primitives the engines need: spawning, sleeping and
//! timeouts, on tokio (`native`) or the browser event loop (`web`).

use std::future::Future;
use std::time::Duration;

/// `Send` on native targets, nothing in the browser (where futures and
/// browser handles are single-threaded).
#[cfg(feature = "native")]
pub trait MaybeSend: Send {}
#[cfg(feature = "native")]
impl<T: Send + ?Sized> MaybeSend for T {}

/// `Sync` on native targets, nothing in the browser.
#[cfg(feature = "native")]
pub trait MaybeSync: Sync {}
#[cfg(feature = "native")]
impl<T: Sync + ?Sized> MaybeSync for T {}

/// `Send` on native targets, nothing in the browser.
#[cfg(not(feature = "native"))]
pub trait MaybeSend {}
#[cfg(not(feature = "native"))]
impl<T: ?Sized> MaybeSend for T {}

/// `Sync` on native targets, nothing in the browser.
#[cfg(not(feature = "native"))]
pub trait MaybeSync {}
#[cfg(not(feature = "native"))]
impl<T: ?Sized> MaybeSync for T {}

/// A boxed future that is `Send` where tasks may move between threads.
#[cfg(feature = "native")]
pub(crate) type BoxFuture<T> = std::pin::Pin<Box<dyn Future<Output = T> + Send>>;
/// A boxed future that is `Send` where tasks may move between threads.
#[cfg(not(feature = "native"))]
pub(crate) type BoxFuture<T> = std::pin::Pin<Box<dyn Future<Output = T>>>;

/// Spawns a background task on the current runtime.
#[cfg(feature = "native")]
pub(crate) fn spawn<F>(fut: F)
where
    F: Future<Output = ()> + Send + 'static,
{
    tokio::spawn(fut);
}

/// Spawns a background task on the browser event loop.
#[cfg(not(feature = "native"))]
pub(crate) fn spawn<F>(fut: F)
where
    F: Future<Output = ()> + 'static,
{
    wasm_bindgen_futures::spawn_local(fut);
}

/// Sleeps for `d`.
#[cfg(feature = "native")]
pub(crate) async fn sleep(d: Duration) {
    tokio::time::sleep(d).await;
}

/// Sleeps for `d`.
#[cfg(not(feature = "native"))]
pub(crate) async fn sleep(d: Duration) {
    gloo_timers::future::sleep(d).await;
}

/// Runs `fut` for at most `d`; `None` on timeout.
pub(crate) async fn timeout<F: Future>(d: Duration, fut: F) -> Option<F::Output> {
    tokio::select! {
        v = fut => Some(v),
        () = sleep(d) => None,
    }
}
