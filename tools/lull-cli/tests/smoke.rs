// SPDX-License-Identifier: AGPL-3.0-or-later
//! CLI smoke test: a station process and a listener process pair by
//! invitation over loopback, the listener follows and records audio
//! (FR-PAIR-1, FR-AU-1, FR-ST-1).

use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

fn lull() -> Command {
    Command::new(env!("CARGO_BIN_EXE_lull"))
}

fn temp_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("lull-cli-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

struct Kill(std::process::Child);

impl Drop for Kill {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn fr_pair_1_station_and_listener_processes_over_loopback() {
    let (sdir, ldir) = (temp_dir("station"), temp_dir("listener"));
    let wav = ldir.join("out.wav");
    let mut station = lull()
        .args(["--data", sdir.to_str().unwrap()])
        .args([
            "station",
            "--name",
            "Smoke",
            "--pair",
            "--tone",
            "--loopback",
            "--no-mdns",
        ])
        .args(["--duration", "30"])
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("station starts");
    let out = station.stdout.take().unwrap();
    let station = Kill(station);
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(out).lines().map_while(Result::ok) {
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    let mut lines = Vec::new();
    let invite = loop {
        let line = rx
            .recv_timeout(Duration::from_secs(30))
            .unwrap_or_else(|_| panic!("no invitation printed: {lines:?}"));
        lines.push(line.clone());
        if let Some(uri) = line.strip_prefix("invite: ") {
            break uri.to_owned();
        }
    };
    assert!(lines.iter().any(|l| l.starts_with("station Smoke id ")));
    let code_seen = loop {
        let line = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        if let Some(c) = line.strip_prefix("code: ") {
            break c.to_owned();
        }
    };
    assert_eq!(code_seen.len(), 11);

    let listen = lull()
        .args(["--data", ldir.to_str().unwrap()])
        .args(["listen", "--invite", &invite, "--name", "Smoke listener"])
        .args(["--continuous", "--no-mdns", "--duration", "3"])
        .args(["--out-wav", wav.to_str().unwrap()])
        .output()
        .expect("listener runs");
    let stdout = String::from_utf8_lossy(&listen.stdout);
    assert!(
        listen.status.success(),
        "listener failed: {stdout}\n{}",
        String::from_utf8_lossy(&listen.stderr)
    );
    assert!(stdout.contains("paired with Smoke"), "{stdout}");
    assert!(stdout.contains("Smoke: live"), "{stdout}");
    assert!(stdout.contains("dBFS open"), "{stdout}");

    // The station logged the pairing with the same verification number.
    let verification = stdout
        .lines()
        .find_map(|l| l.split_once("verification ").map(|(_, v)| v.to_owned()))
        .unwrap();
    let mut station_lines = Vec::new();
    while let Ok(l) = rx.recv_timeout(Duration::from_secs(3)) {
        station_lines.push(l);
        if station_lines.iter().any(|l| l.starts_with("disconnected:")) {
            break;
        }
    }
    assert!(
        station_lines
            .iter()
            .any(|l| l == &format!("paired: Smoke listener verification {verification}")),
        "{station_lines:?}"
    );

    // Received audio: the tone, not silence.
    let mut r = hound::WavReader::open(&wav).unwrap();
    assert_eq!(r.spec().sample_rate, 16_000);
    let samples: Vec<i16> = r.samples::<i16>().map(Result::unwrap).collect();
    assert!(samples.len() >= 16_000 * 2, "{} samples", samples.len());
    let tail = &samples[samples.len() - 8_000..];
    let rms = (tail.iter().map(|&s| f64::from(s).powi(2)).sum::<f64>() / tail.len() as f64).sqrt();
    assert!(rms > 1_000.0, "rms {rms}");

    let list = lull()
        .args(["--data", ldir.to_str().unwrap(), "stations"])
        .output()
        .unwrap();
    let list = String::from_utf8_lossy(&list.stdout);
    assert!(list.contains("station Smoke ("), "{list}");
    let slist = lull()
        .args(["--data", sdir.to_str().unwrap(), "stations"])
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&slist.stdout).contains("trusted listener Smoke listener"));
    drop(station);
    let _ = std::fs::remove_dir_all(sdir);
    let _ = std::fs::remove_dir_all(ldir);
}
