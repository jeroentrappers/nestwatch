// SPDX-License-Identifier: AGPL-3.0-or-later
//! `nestwatch-server`: the rendezvous and relay server (rendezvous.md §8).
//!
//! Configuration comes from defaults, then an optional TOML file
//! (`--config`), then environment variables, then command-line flags.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use nestwatch_server::{Config, Server};

/// Nestwatch rendezvous and relay server (NWR/1). Serves the rendezvous
/// WebSocket at /nwr/1, STUN on UDP, the landing page, the browser listener
/// at /app/ and downloads at /downloads/. Run it behind a reverse proxy that
/// terminates TLS.
#[derive(Debug, Parser)]
#[command(name = "nestwatch-server", version, about, long_about = None)]
struct Args {
    /// TOML configuration file.
    #[arg(short, long, env = "NESTWATCH_SERVER_CONFIG")]
    config: Option<PathBuf>,
    /// Local HTTP address [default: 127.0.0.1:8740].
    #[arg(long, env = "NESTWATCH_SERVER_HTTP")]
    http: Option<SocketAddr>,
    /// STUN UDP address [default: 0.0.0.0:3478].
    #[arg(long, env = "NESTWATCH_SERVER_STUN", conflicts_with = "no_stun")]
    stun: Option<SocketAddr>,
    /// Disable the STUN service.
    #[arg(long)]
    no_stun: bool,
    /// State directory (mailbox secret hashes, transfer counters).
    #[arg(long, env = "NESTWATCH_SERVER_STATE_DIR")]
    state_dir: Option<PathBuf>,
    /// Landing page directory, served at /.
    #[arg(long, env = "NESTWATCH_SERVER_SITE_DIR")]
    site_dir: Option<PathBuf>,
    /// Browser listener app directory, served at /app/.
    #[arg(long, env = "NESTWATCH_SERVER_APP_DIR")]
    app_dir: Option<PathBuf>,
    /// Downloads directory, served at /downloads/.
    #[arg(long, env = "NESTWATCH_SERVER_DOWNLOADS_DIR")]
    downloads_dir: Option<PathBuf>,
    /// Trust X-Forwarded-For / X-Real-IP from a loopback proxy.
    #[arg(long, env = "NESTWATCH_SERVER_TRUST_PROXY")]
    trust_proxy: Option<bool>,
    /// Reject WebSocket upgrades from other web origins.
    #[arg(long, env = "NESTWATCH_SERVER_CHECK_ORIGIN")]
    check_origin: Option<bool>,
    /// Seconds between aggregate counter log lines (0: never).
    #[arg(long, env = "NESTWATCH_SERVER_LOG_INTERVAL")]
    log_interval: Option<f64>,
    /// Log filter (tracing EnvFilter syntax).
    #[arg(long, env = "NESTWATCH_SERVER_LOG", default_value = "info")]
    log: String,
    /// Concurrent pipes per mailbox [default: 16].
    #[arg(long, env = "NESTWATCH_SERVER_PIPES_PER_MAILBOX")]
    pipes_per_mailbox: Option<usize>,
    /// Concurrent pipes per listener address [default: 4].
    #[arg(long, env = "NESTWATCH_SERVER_PIPES_PER_IP")]
    pipes_per_ip: Option<usize>,
    /// Relay rate per pipe and direction, bits per second [default: 2000000].
    #[arg(long, env = "NESTWATCH_SERVER_PIPE_RATE_BPS")]
    pipe_rate_bps: Option<u64>,
    /// Monthly transfer budget per mailbox in bytes [default: 50 GiB].
    #[arg(long, env = "NESTWATCH_SERVER_MONTHLY_BYTES")]
    monthly_bytes: Option<u64>,
    /// Pipe idle timeout in seconds [default: 60].
    #[arg(long, env = "NESTWATCH_SERVER_IDLE_TIMEOUT")]
    idle_timeout: Option<f64>,
    /// Pipe lifetime in seconds [default: 86400].
    #[arg(long, env = "NESTWATCH_SERVER_PIPE_LIFETIME")]
    pipe_lifetime: Option<f64>,
    /// Session validity for the station's accept, seconds [default: 30].
    #[arg(long, env = "NESTWATCH_SERVER_SESSION_TIMEOUT")]
    session_timeout: Option<f64>,
    /// `register` requests per address and minute [default: 10].
    #[arg(long, env = "NESTWATCH_SERVER_REGISTER_PER_MINUTE")]
    register_per_minute: Option<u32>,
    /// `connect` requests per address and minute [default: 30].
    #[arg(long, env = "NESTWATCH_SERVER_CONNECT_PER_MINUTE")]
    connect_per_minute: Option<u32>,
    /// STUN requests per address and second [default: 20].
    #[arg(long, env = "NESTWATCH_SERVER_STUN_PER_SECOND")]
    stun_per_second: Option<u32>,
    /// Concurrent TCP connections [default: 4096].
    #[arg(long, env = "NESTWATCH_SERVER_MAX_CONNECTIONS")]
    max_connections: Option<usize>,
    /// Concurrent WebSocket connections per address [default: 32].
    #[arg(long, env = "NESTWATCH_SERVER_MAX_CONNECTIONS_PER_IP")]
    max_connections_per_ip: Option<usize>,
}

fn set<T>(slot: &mut T, v: Option<T>) {
    if let Some(v) = v {
        *slot = v;
    }
}

fn config(args: Args) -> Result<Config, String> {
    let mut c = match &args.config {
        Some(p) => {
            let text = std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()))?;
            Config::from_toml(&text).map_err(|e| format!("{}: {e}", p.display()))?
        }
        None => Config::default(),
    };
    set(&mut c.http, args.http);
    if args.no_stun {
        c.stun = None;
    } else if args.stun.is_some() {
        c.stun = args.stun;
    }
    set(&mut c.state_dir, args.state_dir);
    if args.site_dir.is_some() {
        c.site_dir = args.site_dir;
    }
    if args.app_dir.is_some() {
        c.app_dir = args.app_dir;
    }
    if args.downloads_dir.is_some() {
        c.downloads_dir = args.downloads_dir;
    }
    set(&mut c.trust_proxy, args.trust_proxy);
    set(&mut c.check_origin, args.check_origin);
    set(&mut c.log_interval, args.log_interval);
    let l = &mut c.limits;
    set(&mut l.pipes_per_mailbox, args.pipes_per_mailbox);
    set(&mut l.pipes_per_ip, args.pipes_per_ip);
    set(&mut l.pipe_rate_bps, args.pipe_rate_bps);
    set(&mut l.monthly_bytes_per_mailbox, args.monthly_bytes);
    set(&mut l.idle_timeout, args.idle_timeout);
    set(&mut l.pipe_lifetime, args.pipe_lifetime);
    set(&mut l.session_timeout, args.session_timeout);
    set(&mut l.register_per_minute, args.register_per_minute);
    set(&mut l.connect_per_minute, args.connect_per_minute);
    set(&mut l.stun_per_second, args.stun_per_second);
    set(&mut l.max_connections, args.max_connections);
    set(&mut l.max_connections_per_ip, args.max_connections_per_ip);
    Ok(c)
}

async fn terminated() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        match signal(SignalKind::terminate()) {
            Ok(mut term) => {
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {}
                    _ = term.recv() => {}
                }
            }
            Err(_) => {
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

fn main() -> ExitCode {
    let args = Args::parse();
    let filter = tracing_subscriber::EnvFilter::try_new(&args.log)
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stderr()))
        .init();
    let config = match config(args) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("nestwatch-server: {e}");
            return ExitCode::from(2);
        }
    };
    let rt = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("nestwatch-server: cannot start runtime: {e}");
            return ExitCode::FAILURE;
        }
    };
    rt.block_on(async {
        let server = match Server::start(config).await {
            Ok(s) => s,
            Err(e) => {
                tracing::error!("cannot start: {e}");
                return ExitCode::FAILURE;
            }
        };
        terminated().await;
        tracing::info!("shutting down");
        server.shutdown().await;
        ExitCode::SUCCESS
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_override_defaults() {
        let a = Args::parse_from([
            "nestwatch-server",
            "--http",
            "127.0.0.1:9999",
            "--no-stun",
            "--pipes-per-ip",
            "2",
            "--idle-timeout",
            "1.5",
        ]);
        let c = config(a).unwrap();
        assert_eq!(c.http.port(), 9999);
        assert_eq!(c.stun, None);
        assert_eq!(c.limits.pipes_per_ip, 2);
        assert_eq!(c.limits.idle_timeout, 1.5);
        assert_eq!(c.limits.pipes_per_mailbox, 16);
    }

    #[test]
    fn flags_override_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("server.toml");
        std::fs::write(&path, "[limits]\npipes_per_ip = 3\npipes_per_mailbox = 5\n").unwrap();
        let a = Args::parse_from([
            "nestwatch-server",
            "--config",
            path.to_str().unwrap(),
            "--pipes-per-ip",
            "7",
        ]);
        let c = config(a).unwrap();
        assert_eq!(c.limits.pipes_per_ip, 7);
        assert_eq!(c.limits.pipes_per_mailbox, 5);
        let bad = dir.path().join("bad.toml");
        std::fs::write(&bad, "unknown_key = 1\n").unwrap();
        let a = Args::parse_from(["nestwatch-server", "--config", bad.to_str().unwrap()]);
        assert!(config(a).is_err());
    }
}
