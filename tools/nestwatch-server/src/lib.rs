// SPDX-License-Identifier: AGPL-3.0-or-later
//! The Nestwatch rendezvous and relay server (NWR/1, `specs/rendezvous.md`
//! §4, §6, §8) as a library, so tests can run it in-process; `main.rs` is
//! the command-line front end.
//!
//! One process serves, on a local HTTP port behind a TLS-terminating
//! reverse proxy: the `/nwr/1` rendezvous WebSocket with the relay pipes,
//! `/healthz` with aggregate counters, the landing page (`/`), the browser
//! listener (`/app/`) and downloads (`/downloads/`); and on UDP the STUN
//! Binding service. Only mailbox secret hashes and monthly transfer
//! counters are persisted ([`store`]); logs carry aggregate counters only.

pub mod config;
mod files;
mod http;
mod hub;
pub mod limits;
pub mod metrics;
pub mod store;
mod stun;
mod tasks;

use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use tokio::net::{TcpListener, UdpSocket};
use tokio::sync::watch;
use tokio::task::JoinHandle;

pub use config::{Config, Limits};
pub use http::{APP_CSP, SITE_CSP};
pub use metrics::Snapshot;

use crate::hub::Hub;
use crate::tasks::Tasks;

/// How often the state file is flushed when counters changed.
const FLUSH_INTERVAL: Duration = Duration::from_secs(30);
/// Housekeeping interval (expired sessions, forgotten mailboxes).
const SWEEP_INTERVAL: Duration = Duration::from_secs(1);
/// Time open connections get to finish on shutdown.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(5);

/// A running server; [`shutdown`](Self::shutdown) stops it gracefully.
#[derive(Debug)]
pub struct Server {
    http_addr: SocketAddr,
    stun_addr: Option<SocketAddr>,
    hub: Arc<Hub>,
    stop: watch::Sender<bool>,
    tasks: Tasks,
    background: Vec<JoinHandle<()>>,
}

impl std::fmt::Debug for Hub {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Hub").finish_non_exhaustive()
    }
}

impl Server {
    /// Opens the state, binds the HTTP and STUN sockets and starts serving
    /// on the current tokio runtime.
    pub async fn start(config: Config) -> io::Result<Self> {
        let store = store::Store::open(&config.state_dir, config.limits.mailbox_retention_days)?;
        let http = TcpListener::bind(config.http).await?;
        let http_addr = http.local_addr()?;
        let stun_socket = match config.stun {
            Some(a) => Some(UdpSocket::bind(a).await?),
            None => None,
        };
        let stun_addr = stun_socket
            .as_ref()
            .map(UdpSocket::local_addr)
            .transpose()?;
        let (stop, stop_rx) = watch::channel(false);
        let log_interval = config::secs(config.log_interval);
        let hub = Arc::new(Hub::new(Arc::new(config), store, stop_rx.clone()));
        let tasks = Tasks::default();
        let mut background = vec![tokio::spawn(http::serve(
            http,
            hub.clone(),
            tasks.clone(),
            stop_rx.clone(),
        ))];
        if let Some(s) = stun_socket {
            background.push(tokio::spawn(stun::serve(s, hub.clone(), stop_rx.clone())));
        }
        background.push(tokio::spawn(housekeeping(
            hub.clone(),
            log_interval,
            stop_rx,
        )));
        tracing::info!(
            "nestwatch-server {} listening on http://{http_addr}{}",
            env!("CARGO_PKG_VERSION"),
            stun_addr
                .map(|a| format!(", STUN udp://{a}"))
                .unwrap_or_default()
        );
        Ok(Self {
            http_addr,
            stun_addr,
            hub,
            stop,
            tasks,
            background,
        })
    }

    /// The bound HTTP address.
    pub fn http_addr(&self) -> SocketAddr {
        self.http_addr
    }

    /// The bound STUN address, if enabled.
    pub fn stun_addr(&self) -> Option<SocketAddr> {
        self.stun_addr
    }

    /// The aggregate counters.
    pub fn metrics(&self) -> Snapshot {
        self.hub.metrics.snapshot()
    }

    /// Stops accepting, closes rendezvous connections and pipes, waits up
    /// to 5 s for them, and writes the state file.
    pub async fn shutdown(self) {
        let _ = self.stop.send(true);
        self.tasks.wait_idle(SHUTDOWN_GRACE).await;
        for t in self.background {
            let _ = t.await;
        }
        if let Err(e) = self.hub.store.flush() {
            tracing::error!("cannot write state: {e}");
        }
        tracing::info!("stopped; {}", self.hub.metrics.snapshot());
    }
}

async fn housekeeping(hub: Arc<Hub>, log_interval: Duration, mut stop: watch::Receiver<bool>) {
    let mut sweep = tokio::time::interval(SWEEP_INTERVAL);
    let mut flush = tokio::time::interval(FLUSH_INTERVAL);
    let mut log = (!log_interval.is_zero()).then(|| {
        tokio::time::interval_at(tokio::time::Instant::now() + log_interval, log_interval)
    });
    loop {
        tokio::select! {
            _ = sweep.tick() => hub.sweep(),
            _ = flush.tick() => {
                if let Err(e) = hub.store.flush() {
                    tracing::error!("cannot write state: {e}");
                }
            }
            _ = async { log.as_mut().expect("checked").tick().await }, if log.is_some() => {
                tracing::info!("{}", hub.metrics.snapshot());
            }
            () = crate::tasks::stopped(&mut stop) => return,
        }
    }
}
