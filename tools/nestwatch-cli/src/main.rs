// SPDX-License-Identifier: AGPL-3.0-or-later
//! `nestwatch`: a headless Nestwatch station and listener on nestwatch-core
//! (`specs/architecture.md` §2, tools/nestwatch-cli).
//!
//! Device audio arrives with nestwatch-platform; until then audio comes from a
//! test tone, a WAV file or raw PCM on stdin, and received audio can be
//! written to a WAV file.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use clap::{Parser, Subcommand};
use nestwatch_core::listener::{
    ConnStatus, ListenSettings, ListenerConfig, ListenerEngine, ListenerEvent, NetPath, Route,
    VideoRequest,
};
use nestwatch_core::station::{
    AssetSource, BindScope, ListenConfig, RelayStatus, StationConfig, StationEngine, StationEvent,
    WebConfig, WebOptions,
};
use nestwatch_core::{DirStorage, Endpoint, Storage, SystemClock, grouped_id};

#[derive(Parser, Debug)]
#[command(
    name = "nestwatch",
    version,
    about = "Headless Nestwatch station and listener"
)]
struct Cli {
    /// Directory for the device key, trust store and settings
    /// (default: the per-user data directory).
    #[arg(long, global = true)]
    data: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Run a station.
    Station(StationArgs),
    /// Pair with and follow stations.
    Listen(ListenArgs),
    /// List stored stations (listener role) and trusted listeners (station role).
    Stations,
}

#[derive(Parser, Debug)]
struct StationArgs {
    /// Display name (kept after the first run).
    #[arg(long)]
    name: Option<String>,
    /// Open pairing mode and print the invitation, web address, QR code and code.
    #[arg(long)]
    pair: bool,
    /// Send a 440 Hz test tone at -20 dBFS.
    #[arg(long, conflicts_with_all = ["wav", "stdin_pcm"])]
    tone: bool,
    /// Send a WAV file, looped.
    #[arg(long, value_name = "FILE", conflicts_with = "stdin_pcm")]
    wav: Option<PathBuf>,
    /// Send raw signed 16-bit little-endian mono PCM from stdin at RATE Hz.
    #[arg(long, value_name = "RATE")]
    stdin_pcm: Option<u32>,
    /// TCP port (default 47100 with fallbacks; 0 picks a free port).
    #[arg(long)]
    port: Option<u16>,
    /// HTTP port of the browser app (default 47180; 0 picks a free port).
    #[arg(long)]
    http_port: Option<u16>,
    /// HTTPS port of the browser app (default 47181; 0 picks a free port).
    #[arg(long)]
    https_port: Option<u16>,
    /// Do not serve the browser app.
    #[arg(long)]
    no_web: bool,
    /// Directory with the browser listener app.
    #[arg(long, value_name = "DIR")]
    web_dir: Option<PathBuf>,
    /// Do not advertise over mDNS.
    #[arg(long)]
    no_mdns: bool,
    /// Listen on 127.0.0.1 / ::1 only, on free ports unless given (tests).
    #[arg(long)]
    loopback: bool,
    /// Endpoint to put into invitations instead of the local addresses.
    #[arg(long = "endpoint", value_name = "HOST:PORT")]
    endpoints: Vec<String>,
    /// Stop after this many seconds.
    #[arg(long, value_name = "SECS")]
    duration: Option<u64>,
    /// Answer camera requests with a synthetic moving test pattern (tests;
    /// the CLI has no camera otherwise).
    #[arg(long)]
    test_camera: bool,
    /// Make the station reachable through this relay server (e.g.
    /// https://nestwatch.appmire.be or a self-hosted origin); kept for later
    /// runs.
    #[arg(long, value_name = "ORIGIN", conflicts_with = "no_relay")]
    relay: Option<String>,
    /// Turn the relay off (kept for later runs).
    #[arg(long)]
    no_relay: bool,
    /// Make a new relay mailbox; listeners need a new invitation.
    #[arg(long)]
    reset_relay_identity: bool,
    /// Put no direct endpoints into invitations (reachable through the
    /// relay only).
    #[arg(long, conflicts_with = "endpoints")]
    no_endpoints: bool,
    /// STUN port of the relay server instead of 3478.
    #[arg(long, value_name = "PORT")]
    relay_stun_port: Option<u16>,
}

#[derive(Parser, Debug)]
struct ListenArgs {
    /// Pair with an invitation (`nestwatch://invite?...` or the station's web address).
    #[arg(long, value_name = "URI", conflicts_with = "code")]
    invite: Option<String>,
    /// Pair with a pairing code.
    #[arg(long)]
    code: Option<String>,
    /// Station ID for `--code` (otherwise found over mDNS).
    #[arg(long, value_name = "ID")]
    station_id: Option<String>,
    /// Address to try for pairing (`host:port` or `ws://host:port/nestwatch/1`).
    #[arg(long = "route", value_name = "ROUTE")]
    routes: Vec<String>,
    /// This listener's display name.
    #[arg(long, default_value = "Nestwatch CLI")]
    name: String,
    /// Write the mixed audio of all stations to a 16 kHz mono WAV file.
    #[arg(long, value_name = "FILE")]
    out_wav: Option<PathBuf>,
    /// Ask for continuous audio instead of sound activation.
    #[arg(long)]
    continuous: bool,
    /// Sound activation threshold in dBFS.
    #[arg(long, allow_negative_numbers = true)]
    threshold: Option<f32>,
    /// Do not browse mDNS.
    #[arg(long)]
    no_mdns: bool,
    /// Stop after this many seconds.
    #[arg(long, value_name = "SECS")]
    duration: Option<u64>,
    /// STUN port of relay servers instead of 3478.
    #[arg(long, value_name = "PORT")]
    relay_stun_port: Option<u16>,
    /// Stay on the relay; do not punch a direct path.
    #[arg(long)]
    no_direct: bool,
    /// Open the camera view of every live station and report the frames
    /// received (tests, FR-CAM-1).
    #[arg(long)]
    video: bool,
    /// Hold talk-back to every live station for 10 s with a 440 Hz tone
    /// (tests, FR-TB-1).
    #[arg(long)]
    talk_tone: bool,
}

fn data_dir(cli: &Cli) -> PathBuf {
    cli.data.clone().unwrap_or_else(|| {
        dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("nestwatch")
    })
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let storage = match DirStorage::open(data_dir(&cli)) {
        Ok(s) => Arc::new(s) as Arc<dyn Storage>,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    let result = rt.block_on(async {
        match cli.command {
            Command::Station(a) => run_station(a, storage).await,
            Command::Listen(a) => run_listen(a, storage).await,
            Command::Stations => list(&*storage),
        }
    });
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Resolves on Ctrl-C or after `duration`.
async fn stop_signal(duration: Option<u64>) {
    let timer = async {
        match duration {
            Some(s) => tokio::time::sleep(Duration::from_secs(s)).await,
            None => std::future::pending().await,
        }
    };
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        () = timer => {}
    }
}

fn port_config(port: Option<u16>, default: ListenConfig, loopback: bool) -> ListenConfig {
    let any = ListenConfig {
        port: 0,
        attempts: 1,
        scope: BindScope::All,
    };
    let mut c = match port {
        Some(0) => any,
        Some(p) => ListenConfig::new(p),
        None if loopback => any,
        None => default,
    };
    if loopback {
        c.scope = BindScope::Loopback;
    }
    c
}

const PLACEHOLDER_APP: &[u8] = b"<!doctype html><meta charset=utf-8><title>Nestwatch</title>\
<p>This station runs without the browser listener app.</p>";

fn print_qr(text: &str) {
    match qrcode::QrCode::new(text.as_bytes()) {
        Ok(code) => {
            let image = code
                .render::<qrcode::render::unicode::Dense1x2>()
                .quiet_zone(true)
                .build();
            println!("{image}");
        }
        Err(e) => println!("(no QR code: {e})"),
    }
}

async fn run_station(a: StationArgs, storage: Arc<dyn Storage>) -> Result<(), String> {
    let mut cfg = StationConfig::new(a.name.clone().unwrap_or_else(|| "Nestwatch station".into()));
    cfg.tcp = port_config(a.port, ListenConfig::default(), a.loopback);
    cfg.mdns = !a.no_mdns;
    if !a.endpoints.is_empty() {
        let mut eps = Vec::new();
        for e in &a.endpoints {
            eps.push(Endpoint::parse(e).map_err(|err| format!("endpoint {e}: {err}"))?);
        }
        cfg.endpoints = Some(eps);
    }
    if a.no_endpoints {
        cfg.endpoints = Some(Vec::new());
    }
    cfg.relay.stun_port = a.relay_stun_port;
    if a.loopback {
        cfg.relay.punch_bind = std::net::SocketAddr::from(([127, 0, 0, 1], 0));
    }
    if !a.no_web {
        let defaults = WebConfig::default();
        let http = defaults.http.unwrap_or_default();
        let https = defaults.https.unwrap_or_default();
        let assets = match &a.web_dir {
            Some(d) => AssetSource::Dir(d.clone()),
            None => AssetSource::memory([("index.html", PLACEHOLDER_APP)]),
        };
        cfg.web = Some(WebOptions {
            config: WebConfig {
                http: Some(port_config(a.http_port, http, a.loopback)),
                https: Some(port_config(a.https_port, https, a.loopback)),
                ..defaults
            },
            assets,
        });
    }
    let station = StationEngine::start(cfg, storage, Arc::new(SystemClock::new()))
        .await
        .map_err(|e| e.to_string())?;
    if let Some(n) = &a.name
        && station.snapshot().name != *n
    {
        station.rename(n).map_err(|e| e.to_string())?;
    }
    if let Some(origin) = &a.relay {
        station
            .set_relay_server(origin)
            .map_err(|e| e.to_string())?;
        station.set_relay_enabled(true).map_err(|e| e.to_string())?;
    } else if a.no_relay {
        station
            .set_relay_enabled(false)
            .map_err(|e| e.to_string())?;
    }
    if a.reset_relay_identity {
        station.reset_relay_identity().map_err(|e| e.to_string())?;
    }
    let s = station.snapshot();
    println!("station {} id {}", s.name, grouped_id(&s.device_id));
    println!(
        "serving tcp {} http {} https {}",
        s.tcp_port,
        s.http_port.map_or("-".into(), |p| p.to_string()),
        s.https_port.map_or("-".into(), |p| p.to_string())
    );
    if let Some(f) = &s.https_fingerprint {
        println!("https fingerprint {f}");
    }
    let mut events = station.events();
    if a.pair {
        let info = station.open_pairing().await.map_err(|e| e.to_string())?;
        println!("invite: {}", info.invite_uri);
        if let Some(w) = &info.web_url {
            println!("web: {w}");
        }
        if let Some(w) = &info.relay_web_url {
            println!("relay web: {w}");
        }
        println!("code: {}", info.code);
        print_qr(&info.invite_uri);
    }
    let relay = spawn_relay_printer(station.clone());
    let source = spawn_source(&a, station.clone())?;
    let test_camera = a.test_camera;
    let camera_station = station.clone();
    let printer = tokio::spawn(async move {
        let mut camera: Option<tokio::task::JoinHandle<()>> = None;
        loop {
            match events.recv().await {
                Ok(e) => {
                    print_station_event(&e);
                    if test_camera {
                        match &e {
                            StationEvent::CameraWanted(r) => {
                                if let Some(c) = camera.take() {
                                    c.abort();
                                }
                                camera = Some(spawn_test_camera(camera_station.clone(), r.clone()));
                            }
                            StationEvent::CameraNotWanted => {
                                if let Some(c) = camera.take() {
                                    c.abort();
                                }
                            }
                            _ => {}
                        }
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(_) => break,
            }
        }
    });
    stop_signal(a.duration).await;
    source.abort();
    relay.abort();
    station.shutdown().await;
    printer.abort();
    Ok(())
}

/// Prints relay registration changes (rendezvous.md §4.1).
fn spawn_relay_printer(station: StationEngine) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut st = station.state();
        let mut last: Option<RelayStatus> = None;
        loop {
            let (relay, server) = {
                let s = st.borrow_and_update();
                (s.relay.clone(), s.settings.relay_server.clone())
            };
            if last.as_ref() != Some(&relay) {
                match &relay {
                    RelayStatus::Off => println!("relay: off"),
                    RelayStatus::Connecting => println!("relay: connecting to {server}"),
                    RelayStatus::Online { observed } => {
                        println!("relay: online at {server} (seen as {observed})")
                    }
                    RelayStatus::Error { message, permanent } => println!(
                        "relay: {}error: {message}",
                        if *permanent { "permanent " } else { "" }
                    ),
                }
                last = Some(relay);
            }
            if st.changed().await.is_err() {
                break;
            }
        }
    })
}

fn print_station_event(e: &StationEvent) {
    match e {
        StationEvent::PairingOpened => println!("pairing open"),
        StationEvent::PairingClosed { reason } => println!("pairing closed: {reason:?}"),
        StationEvent::Paired {
            name, verification, ..
        } => println!("paired: {name} verification {verification}"),
        StationEvent::ListenerConnected { name, session, .. } => {
            println!("connected: {name} (session {session})")
        }
        StationEvent::ListenerDisconnected {
            listener_id,
            reason,
            session,
        } => println!(
            "disconnected: {} (session {session}): {reason}",
            grouped_id(listener_id)
        ),
        StationEvent::SessionReplaced { listener_id, .. } => {
            println!("replaced: {}", grouped_id(listener_id))
        }
        StationEvent::ListenerRevoked { listener_id } => {
            println!("revoked: {}", grouped_id(listener_id))
        }
        StationEvent::ListenerUnpaired { listener_id } => {
            println!("unpaired: {}", grouped_id(listener_id))
        }
        StationEvent::CameraWanted(r) => {
            println!("camera wanted {}x{} at {} fps", r.max_w, r.max_h, r.max_fps)
        }
        StationEvent::CameraNotWanted => println!("camera not wanted"),
        StationEvent::TalkStarted { name } => println!("talk-back from {name}"),
        StationEvent::TalkEnded => println!("talk-back ended"),
    }
}

/// Pushes a moving RGB test pattern at the requested size and rate.
fn spawn_test_camera(
    station: StationEngine,
    req: nestwatch_core::station::CameraRequest,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let (w, h) = (req.max_w.clamp(16, 640), req.max_h.clamp(16, 480));
        let period = Duration::from_millis(1000 / u64::from(req.max_fps.clamp(1, 30)));
        let mut tick = tokio::time::interval(period);
        let mut t = 0u32;
        loop {
            tick.tick().await;
            let mut data = Vec::with_capacity((w * h * 3) as usize);
            for y in 0..h {
                for x in 0..w {
                    let band = ((x + t * 4) / 40) % 2 == 0;
                    data.push(if band { 200 } else { 40 });
                    data.push(((y * 255) / h) as u8);
                    data.push(((x * 255) / w) as u8);
                }
            }
            station.push_camera_frame(&nestwatch_core::station::CameraFrame {
                width: w,
                height: h,
                format: nestwatch_core::station::PixelFormat::Rgb,
                data,
            });
            t = t.wrapping_add(1);
        }
    })
}

/// Starts the audio source task.
fn spawn_source(
    a: &StationArgs,
    station: StationEngine,
) -> Result<tokio::task::JoinHandle<()>, String> {
    if let Some(path) = &a.wav {
        let (samples, rate, channels) = read_wav(path)?;
        return Ok(tokio::spawn(async move {
            let chunk = (rate as usize / 50) * usize::from(channels);
            let mut pos = 0;
            let mut tick = tokio::time::interval(Duration::from_millis(20));
            loop {
                tick.tick().await;
                let end = (pos + chunk).min(samples.len());
                station.push_capture(&samples[pos..end], rate, channels);
                pos = if end >= samples.len() { 0 } else { end };
            }
        }));
    }
    if let Some(rate) = a.stdin_pcm {
        if rate == 0 {
            return Err("--stdin-pcm needs a rate".into());
        }
        let (tx, mut rx) = tokio::sync::mpsc::channel::<Vec<i16>>(64);
        std::thread::spawn(move || {
            let mut stdin = std::io::stdin().lock();
            let mut buf = vec![0u8; (rate as usize / 50) * 2];
            while stdin.read_exact(&mut buf).is_ok() {
                let s = buf
                    .chunks_exact(2)
                    .map(|b| i16::from_le_bytes([b[0], b[1]]))
                    .collect();
                if tx.blocking_send(s).is_err() {
                    break;
                }
            }
        });
        return Ok(tokio::spawn(async move {
            while let Some(s) = rx.recv().await {
                station.push_capture(&s, rate, 1);
            }
        }));
    }
    let tone = a.tone;
    Ok(tokio::spawn(async move {
        // Without a source the station sends silence, so levels and sound
        // activation behave as in a quiet room.
        let rate = 48_000u32;
        let amp = 32768.0 * 10f32.powf(-20.0 / 20.0) * std::f32::consts::SQRT_2;
        let mut phase = 0f32;
        let step = 2.0 * std::f32::consts::PI * 440.0 / rate as f32;
        let mut tick = tokio::time::interval(Duration::from_millis(20));
        loop {
            tick.tick().await;
            let s: Vec<i16> = (0..rate / 50)
                .map(|_| {
                    let v = if tone { (amp * phase.sin()) as i16 } else { 0 };
                    phase = (phase + step) % (2.0 * std::f32::consts::PI);
                    v
                })
                .collect();
            station.push_capture(&s, rate, 1);
        }
    }))
}

fn read_wav(path: &Path) -> Result<(Vec<i16>, u32, u16), String> {
    let mut r = hound::WavReader::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let spec = r.spec();
    let samples: Vec<i16> = match (spec.sample_format, spec.bits_per_sample) {
        (hound::SampleFormat::Int, 16) => r.samples::<i16>().filter_map(Result::ok).collect(),
        (hound::SampleFormat::Int, b) if (17..=32).contains(&b) => r
            .samples::<i32>()
            .filter_map(Result::ok)
            .map(|s| (s >> (b - 16)) as i16)
            .collect(),
        (hound::SampleFormat::Int, 8) => r
            .samples::<i8>()
            .filter_map(Result::ok)
            .map(|s| i16::from(s) << 8)
            .collect(),
        (hound::SampleFormat::Float, _) => r
            .samples::<f32>()
            .filter_map(Result::ok)
            .map(|s| (s * 32767.0).clamp(-32768.0, 32767.0) as i16)
            .collect(),
        _ => return Err("unsupported WAV format".into()),
    };
    if samples.is_empty() {
        return Err("empty WAV file".into());
    }
    Ok((samples, spec.sample_rate, spec.channels))
}

fn parse_routes(list: &[String]) -> Result<Vec<Route>, String> {
    list.iter()
        .map(|r| Route::parse(r).ok_or_else(|| format!("invalid route {r}")))
        .collect()
}

/// Finds the one station in pairing mode on the local network.
async fn discover_pairing_station(l: &ListenerEngine) -> Result<(String, Vec<Route>), String> {
    let mut st = l.state();
    let wait = async {
        loop {
            let found: Vec<_> = st
                .borrow_and_update()
                .discovered
                .iter()
                .filter(|d| d.pairing)
                .cloned()
                .collect();
            if let [d] = &found[..] {
                return Ok((
                    d.id.clone(),
                    d.endpoints.iter().cloned().map(Route::Tcp).collect(),
                ));
            }
            if found.len() > 1 {
                let names: Vec<_> = found
                    .iter()
                    .map(|d| format!("{} ({})", d.name, grouped_id(&d.id)))
                    .collect();
                return Err(format!(
                    "several stations are pairing, pass --station-id: {}",
                    names.join(", ")
                ));
            }
            if st.changed().await.is_err() {
                return Err("listener stopped".into());
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(10), wait)
        .await
        .map_err(|_| {
            "no station in pairing mode found; pass --station-id and --route".to_string()
        })?
}

async fn run_listen(a: ListenArgs, storage: Arc<dyn Storage>) -> Result<(), String> {
    let mut cfg = ListenerConfig::new(a.name.clone());
    cfg.discovery = !a.no_mdns;
    cfg.relay.stun_port = a.relay_stun_port;
    cfg.relay.direct = !a.no_direct;
    let l = ListenerEngine::start(cfg, storage, Arc::new(SystemClock::new()))
        .map_err(|e| e.to_string())?;
    if l.snapshot().name != a.name {
        l.rename(&a.name).map_err(|e| e.to_string())?;
    }
    let routes = parse_routes(&a.routes)?;
    let mut events = l.events();
    let paired = if let Some(uri) = &a.invite {
        Some(
            l.pair_invitation_via(uri, routes)
                .await
                .map_err(|e| e.to_string())?,
        )
    } else if let Some(code) = &a.code {
        let (id, routes) = match &a.station_id {
            Some(id) => (id.clone(), routes),
            None => discover_pairing_station(&l).await?,
        };
        Some(
            l.pair_code(code, &id, routes)
                .await
                .map_err(|e| e.to_string())?,
        )
    } else {
        None
    };
    if let Some(p) = &paired {
        println!(
            "paired with {} ({}), verification {}",
            p.name,
            grouped_id(&p.station_id),
            p.verification
        );
    }
    let known = l.known_stations();
    if known.is_empty() {
        return Err("no stations: pair with --invite or --code".into());
    }
    let listen = if a.continuous {
        ListenSettings::continuous()
    } else {
        let mut s = ListenSettings::default();
        if let Some(t) = a.threshold {
            s.threshold_dbfs = t;
        }
        s
    };
    for s in &known {
        let _ = l.set_listen(&s.id, listen);
        if !s.needs_pairing {
            let _ = l.follow(&s.id);
        }
    }

    let printer = tokio::spawn(async move {
        loop {
            match events.recv().await {
                Ok(e) => print_listener_event(&e),
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(_) => break,
            }
        }
    });
    let status = {
        let l = l.clone();
        tokio::spawn(async move {
            let mut last: Vec<(String, ConnStatus)> = Vec::new();
            let mut paths: Vec<(String, Option<NetPath>)> = Vec::new();
            let mut tick = tokio::time::interval(Duration::from_secs(1));
            loop {
                tick.tick().await;
                let s = l.snapshot();
                for v in &s.stations {
                    if !last.iter().any(|(id, st)| id == &v.id && *st == v.status) {
                        println!("{}: {}", v.name, describe_status(&v.status));
                    }
                    if v.path.is_some() && !paths.iter().any(|(id, p)| id == &v.id && *p == v.path)
                    {
                        println!(
                            "{}: path {} via {}",
                            v.name,
                            describe_path(v.path),
                            v.route.as_deref().unwrap_or("-")
                        );
                    }
                    if v.status == ConnStatus::Live {
                        println!(
                            "{}: level {:.1} dBFS {} battery {} listeners {} audio frames {}",
                            v.name,
                            v.level_dbfs.unwrap_or(-90.0),
                            if v.open { "open" } else { "quiet" },
                            v.battery.map_or("-".into(), |b| format!("{b}%")),
                            v.station_listeners.unwrap_or(0),
                            v.audio_frames
                        );
                    }
                }
                last = s
                    .stations
                    .iter()
                    .map(|v| (v.id.clone(), v.status.clone()))
                    .collect();
                paths = s.stations.iter().map(|v| (v.id.clone(), v.path)).collect();
            }
        })
    };
    let writer = match &a.out_wav {
        Some(path) => Some(spawn_wav_writer(path, l.clone())?),
        None => None,
    };
    let tester =
        (a.video || a.talk_tone).then(|| spawn_media_tester(l.clone(), a.video, a.talk_tone));
    stop_signal(a.duration).await;
    if let Some(t) = tester {
        t.abort();
    }
    status.abort();
    if let Some((task, stop)) = writer {
        let _ = stop.send(());
        let _ = task.await;
    }
    l.shutdown().await;
    printer.abort();
    Ok(())
}

/// `--video` / `--talk-tone`: once a station is live, open its camera view
/// and hold talk-back with a 440 Hz tone for 10 s; report the frames
/// received every second.
fn spawn_media_tester(l: ListenerEngine, video: bool, talk: bool) -> tokio::task::JoinHandle<()> {
    /// 10 s of 20 ms ticks.
    const TALK_TICKS: u64 = 500;
    tokio::spawn(async move {
        let mut frames = l.video_frames();
        let (mut count, mut size) = (0u64, (0u16, 0u16));
        // Stations served, with the tick their talk-back started.
        let mut started: Vec<(String, u64)> = Vec::new();
        let mut tone = (0..320 * 50)
            .map(|i| {
                let t = i as f32 / 16_000.0;
                ((t * 440.0 * std::f32::consts::TAU).sin() * 8_000.0) as i16
            })
            .cycle();
        let mut tick = tokio::time::interval(Duration::from_millis(20));
        let mut n = 0u64;
        loop {
            tokio::select! {
                f = frames.recv() => match f {
                    Ok(f) => {
                        count += 1;
                        size = (f.width, f.height);
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(k)) => count += k,
                    Err(_) => break,
                },
                _ = tick.tick() => {
                    n += 1;
                    for v in l.snapshot().stations.iter().filter(|v| v.status == ConnStatus::Live) {
                        if !started.iter().any(|(id, _)| id == &v.id) {
                            started.push((v.id.clone(), n));
                            if video {
                                let _ = l.request_video(&v.id, VideoRequest::default());
                            }
                            if talk {
                                let _ = l.talk(&v.id, true);
                            }
                        }
                    }
                    if talk {
                        for (id, at) in &started {
                            if n == at + TALK_TICKS {
                                let _ = l.talk(id, false);
                            }
                        }
                        let block: Vec<i16> = (&mut tone).take(320).collect();
                        l.push_talk_capture(&block, 16_000, 1);
                    }
                    if n % 50 == 0 && video {
                        println!("video: {count} frames, last {}x{}", size.0, size.1);
                    }
                }
            }
        }
    })
}

/// The tile's path label (rendezvous.md FR-RV-4).
fn describe_path(p: Option<NetPath>) -> &'static str {
    match p {
        Some(NetPath::HomeNetwork) => "home network",
        Some(NetPath::Vpn) => "VPN",
        Some(NetPath::Direct) => "direct (internet)",
        Some(NetPath::Relayed) => "relayed",
        None => "-",
    }
}

fn describe_status(s: &ConnStatus) -> String {
    match s {
        ConnStatus::Idle => "not followed".into(),
        ConnStatus::Connecting { attempt, tried } => {
            format!("connecting (attempt {attempt}, tried {})", tried.join(", "))
        }
        ConnStatus::Live => "live".into(),
        ConnStatus::Reconnecting { attempt, tried } => {
            format!(
                "reconnecting (attempt {attempt}, tried {})",
                tried.join(", ")
            )
        }
        ConnStatus::NeedsPairing => "needs pairing".into(),
    }
}

fn print_listener_event(e: &ListenerEvent) {
    match e {
        ListenerEvent::Paired { .. }
        | ListenerEvent::MicrophoneWanted { .. }
        | ListenerEvent::MicrophoneNotWanted => {}
        ListenerEvent::Connected { station_id } => {
            println!("connected: {}", grouped_id(station_id))
        }
        ListenerEvent::Disconnected { station_id, reason } => {
            println!("disconnected: {}: {reason}", grouped_id(station_id))
        }
        ListenerEvent::NeedsPairing { station_id } => {
            println!("needs pairing: {}", grouped_id(station_id))
        }
        ListenerEvent::Alert { station_id, alert } => {
            println!("alert: {}: {alert:?}", grouped_id(station_id))
        }
        ListenerEvent::Talk { station_id, state } => {
            println!("talk: {}: {state:?}", grouped_id(station_id))
        }
        ListenerEvent::Forgotten { station_id } => {
            println!("forgotten: {}", grouped_id(station_id))
        }
    }
}

type WavTask = (
    tokio::task::JoinHandle<()>,
    tokio::sync::oneshot::Sender<()>,
);

/// Pulls 20 ms of playback every 20 ms into a 16 kHz mono WAV file.
fn spawn_wav_writer(path: &Path, l: ListenerEngine) -> Result<WavTask, String> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 16_000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w =
        hound::WavWriter::create(path, spec).map_err(|e| format!("{}: {e}", path.display()))?;
    let (tx, mut rx) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_millis(20));
        let mut buf = [0i16; 320];
        loop {
            tokio::select! {
                _ = tick.tick() => {
                    l.pull_playback(&mut buf);
                    for s in buf {
                        if w.write_sample(s).is_err() {
                            return;
                        }
                    }
                }
                _ = &mut rx => break,
            }
        }
        let _ = w.finalize();
    });
    Ok((task, tx))
}

fn list(storage: &dyn Storage) -> Result<(), String> {
    let key = nestwatch_core::load_or_create_device_key(storage).map_err(|e| e.to_string())?;
    println!("device {}", key.device_id().grouped());
    let stations = nestwatch_core::listener::stored_stations(storage).map_err(|e| e.to_string())?;
    if stations.is_empty() {
        println!("no known stations");
    }
    for s in stations {
        let routes: Vec<String> = s
            .routes_by_success()
            .iter()
            .map(ToString::to_string)
            .chain(s.relay.clone().map(|r| Route::Relay(r).to_string()))
            .collect();
        println!(
            "station {} ({}){}: {}",
            s.name,
            grouped_id(&s.id),
            if s.needs_pairing {
                " needs pairing"
            } else {
                ""
            },
            routes.join(", ")
        );
    }
    let listeners =
        nestwatch_core::station::stored_trusted_listeners(storage).map_err(|e| e.to_string())?;
    for t in listeners {
        println!("trusted listener {} ({})", t.name, grouped_id(&t.id));
    }
    Ok(())
}
