// SPDX-License-Identifier: AGPL-3.0-or-later
//! Lull browser listener (platforms.md §2.5), served by every station
//! (protocol.md §3.3) and built to a self-contained static bundle
//! (security.md PRIV-1): see `apps/web/README.md`.

// The demo build (design previews) skips the engine boot.
#![cfg_attr(
    feature = "demo",
    allow(dead_code, unused_imports, unreachable_code, clippy::needless_return)
)]

#[cfg(target_arch = "wasm32")]
mod app;
#[cfg(target_arch = "wasm32")]
mod audio;
#[cfg(target_arch = "wasm32")]
mod host;
#[cfg(target_arch = "wasm32")]
mod storage;

#[cfg(target_arch = "wasm32")]
fn main() {
    app::run();
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    eprintln!(
        "lull-web is the browser listener; build it for wasm32 with apps/web/build.sh \
         (see apps/web/README.md)."
    );
    std::process::exit(2);
}
