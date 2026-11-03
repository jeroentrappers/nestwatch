// SPDX-License-Identifier: AGPL-3.0-or-later
//! Sound-server audio on Linux through `parec` / `pacat` (PulseAudio or
//! PipeWire's pulse server).
//!
//! Going through the user's sound server keeps their default devices,
//! routing and gain, which raw ALSA can bypass. Streams are 16-bit mono at
//! a chosen rate; the server resamples. Playback is paced by the wall clock
//! ([`WallClockPacer`]) because the pipe would otherwise swallow seconds of
//! audio, and the pipe itself is shrunk to bound latency under clock drift.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use super::pacing::WallClockPacer;
use super::{
    AudioDevice, AudioInput, AudioOutput, AudioStream, CaptureCallback, CaptureRequest,
    PlaybackRequest, PlaybackSource, SampleType, Samples, SharedStats, StreamFormat, StreamStats,
    f32_to_i16,
};
use crate::process::{StderrTail, find_program, kill, run_with_timeout};
use crate::{APP_ID, APP_NAME, PlatformError};

const PACTL_TIMEOUT: Duration = Duration::from_secs(3);
/// Audio kept written ahead of the wall clock.
const PLAYBACK_LEAD: Duration = Duration::from_millis(40);
/// Pipe capacity towards `pacat` (one page: 128 ms of 16 kHz mono).
const PIPE_BYTES: usize = 4096;
/// How long `start` waits for a capture stream to deliver or fail.
const START_TIMEOUT: Duration = Duration::from_millis(1500);

/// The `parec` / `pacat` backend.
#[derive(Debug, Clone)]
pub struct PulseAudio {
    pactl: PathBuf,
    parec: PathBuf,
    pacat: PathBuf,
}

impl PulseAudio {
    /// Finds the tools and checks that a sound server answers `pactl info`.
    pub fn detect() -> Result<Self, PlatformError> {
        let tool = |name: &'static str| {
            find_program(name).ok_or(PlatformError::Unavailable(format!("{name} not installed")))
        };
        let me = Self {
            pactl: tool("pactl")?,
            parec: tool("parec")?,
            pacat: tool("pacat")?,
        };
        me.pactl(&["info"])
            .ok_or_else(|| PlatformError::Unavailable("no sound server answers pactl".into()))?;
        Ok(me)
    }

    fn pactl(&self, args: &[&str]) -> Option<String> {
        let mut cmd = Command::new(&self.pactl);
        cmd.args(args).env("LC_ALL", "C");
        run_with_timeout(&mut cmd, PACTL_TIMEOUT).map(|o| String::from_utf8_lossy(&o).into_owned())
    }

    fn list(&self, kind: Kind) -> Result<Vec<AudioDevice>, PlatformError> {
        let listing = self
            .pactl(&["list", kind.list_arg()])
            .ok_or_else(|| PlatformError::Unavailable("pactl list failed".into()))?;
        let default = self
            .pactl(&["info"])
            .and_then(|info| parse_default(&info, kind));
        Ok(parse_devices(&listing, kind, default.as_deref()))
    }

    fn check_device(&self, kind: Kind, device: Option<&str>) -> Result<(), PlatformError> {
        if let Some(id) = device
            && !self.list(kind)?.iter().any(|d| d.id == id)
        {
            return Err(PlatformError::NoSuchDevice(id.to_owned()));
        }
        Ok(())
    }
}

/// Device direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Source,
    Sink,
}

impl Kind {
    fn list_arg(self) -> &'static str {
        match self {
            Self::Source => "sources",
            Self::Sink => "sinks",
        }
    }

    fn header(self) -> &'static str {
        match self {
            Self::Source => "Source #",
            Self::Sink => "Sink #",
        }
    }

    fn default_key(self) -> &'static str {
        match self {
            Self::Source => "Default Source:",
            Self::Sink => "Default Sink:",
        }
    }
}

/// Parses `LC_ALL=C pactl list sources|sinks`. Monitor sources (loopbacks
/// of outputs) are not microphones and are skipped.
fn parse_devices(listing: &str, kind: Kind, default: Option<&str>) -> Vec<AudioDevice> {
    let mut out = Vec::new();
    let mut current: Option<(Option<String>, Option<String>)> = None;
    let flush = |cur: Option<(Option<String>, Option<String>)>, out: &mut Vec<AudioDevice>| {
        if let Some((Some(id), name)) = cur {
            if kind == Kind::Source && id.ends_with(".monitor") {
                return;
            }
            out.push(AudioDevice {
                is_default: default == Some(id.as_str()),
                name: name.unwrap_or_else(|| id.clone()),
                id,
            });
        }
    };
    for line in listing.lines() {
        if line.starts_with(kind.header()) {
            flush(current.take(), &mut out);
            current = Some((None, None));
        } else if let Some(cur) = current.as_mut() {
            let t = line.trim();
            if let Some(v) = t.strip_prefix("Name:") {
                cur.0 = Some(v.trim().to_owned());
            } else if let Some(v) = t.strip_prefix("Description:") {
                cur.1 = Some(v.trim().to_owned());
            }
        }
    }
    flush(current, &mut out);
    out
}

fn parse_default(info: &str, kind: Kind) -> Option<String> {
    info.lines()
        .find_map(|l| l.strip_prefix(kind.default_key()))
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty())
}

fn common_args(cmd: &mut Command, rate: u32, channels: u16, stream: &str, latency_ms: u32) {
    cmd.arg("--raw")
        .arg("--format=s16le")
        .arg(format!("--rate={rate}"))
        .arg(format!("--channels={channels}"))
        .arg(format!("--client-name={APP_NAME}"))
        .arg(format!("--stream-name={stream}"))
        .arg(format!("--latency-msec={latency_ms}"))
        .arg(format!("--property=application.id={APP_ID}"))
        .arg(format!("--property=application.name={APP_NAME}"));
}

impl AudioInput for PulseAudio {
    fn backend(&self) -> &'static str {
        "pulse (parec)"
    }

    fn devices(&self) -> Result<Vec<AudioDevice>, PlatformError> {
        self.list(Kind::Source)
    }

    fn start(
        &self,
        request: &CaptureRequest,
        mut callback: CaptureCallback,
    ) -> Result<Box<dyn AudioStream>, PlatformError> {
        self.check_device(Kind::Source, request.device.as_deref())?;
        let format = StreamFormat {
            sample_rate: request.sample_rate.unwrap_or(lull_media::SAMPLE_RATE),
            channels: request.channels.unwrap_or(1).max(1),
            sample_type: SampleType::I16,
        };
        let mut cmd = Command::new(&self.parec);
        common_args(
            &mut cmd,
            format.sample_rate,
            format.channels,
            "Microphone",
            lull_media::FRAME_MS,
        );
        if let Some(dev) = &request.device {
            cmd.arg(format!("--device={dev}"));
        }
        let mut child = cmd
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| PlatformError::Unavailable(format!("cannot start parec: {e}")))?;
        let mut stdout = child.stdout.take().expect("piped stdout");
        let stderr = StderrTail::spawn(child.stderr.take().expect("piped stderr"), |_| true);
        let stats = Arc::new(SharedStats::default());
        let stopping = Arc::new(AtomicBool::new(false));
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(), ()>>();

        // 20 ms blocks.
        let block_samples = (format.sample_rate / 50) as usize * usize::from(format.channels);
        let thread_stats = Arc::clone(&stats);
        let thread_stop = Arc::clone(&stopping);
        let tail = stderr.handle();
        let reader = std::thread::Builder::new()
            .name("lull-parec".into())
            .spawn(move || {
                let mut bytes = vec![0u8; block_samples * 2];
                let mut samples = vec![0i16; block_samples];
                let mut first = true;
                loop {
                    if let Err(e) = stdout.read_exact(&mut bytes) {
                        if !thread_stop.load(Ordering::Relaxed) {
                            std::thread::sleep(Duration::from_millis(50));
                            let why = tail.tail();
                            thread_stats.fail(if why.is_empty() {
                                format!("capture ended: {e}")
                            } else {
                                format!("capture ended: {why}")
                            });
                        }
                        let _ = ready_tx.send(Err(()));
                        return;
                    }
                    for (s, b) in samples.iter_mut().zip(bytes.chunks_exact(2)) {
                        *s = i16::from_le_bytes([b[0], b[1]]);
                    }
                    if first {
                        first = false;
                        let _ = ready_tx.send(Ok(()));
                    }
                    callback(format, Samples::I16(&samples));
                }
            })
            .map_err(PlatformError::Io)?;

        let mut stream = PipeStream {
            child,
            worker: Some(reader),
            stderr,
            stats,
            stopping,
            format,
        };
        match ready_rx.recv_timeout(START_TIMEOUT) {
            Ok(Err(())) => {
                let why = stream.stats().error.unwrap_or_default();
                stream.stop();
                Err(PlatformError::from_message(why))
            }
            // Delivering, or slow to start (a suspended source): keep going.
            _ => Ok(Box::new(stream)),
        }
    }
}

impl AudioOutput for PulseAudio {
    fn backend(&self) -> &'static str {
        "pulse (pacat)"
    }

    fn devices(&self) -> Result<Vec<AudioDevice>, PlatformError> {
        self.list(Kind::Sink)
    }

    fn start(
        &self,
        request: &PlaybackRequest,
        source: PlaybackSource,
    ) -> Result<Box<dyn AudioStream>, PlatformError> {
        self.check_device(Kind::Sink, request.device.as_deref())?;
        let format = StreamFormat {
            sample_rate: lull_media::SAMPLE_RATE,
            channels: 1,
            sample_type: SampleType::F32,
        };
        let mut cmd = Command::new(&self.pacat);
        cmd.arg("--playback").arg("--verbose");
        common_args(
            &mut cmd,
            format.sample_rate,
            1,
            "Playback",
            request.latency_ms.clamp(20, 500),
        );
        if let Some(dev) = &request.device {
            cmd.arg(format!("--device={dev}"));
        }
        let mut child = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| PlatformError::Unavailable(format!("cannot start pacat: {e}")))?;
        let stdin = child.stdin.take().expect("piped stdin");
        if let Err(e) = rustix::pipe::fcntl_setpipe_size(&stdin, PIPE_BYTES) {
            log::debug!("cannot shrink playback pipe: {e}");
        }
        let stats = Arc::new(SharedStats::default());
        let underruns = Arc::clone(&stats);
        let stderr = StderrTail::spawn(child.stderr.take().expect("piped stderr"), move |line| {
            let lower = line.to_ascii_lowercase();
            if lower.contains("underrun") {
                underruns.underruns.fetch_add(1, Ordering::Relaxed);
            }
            // Keep errors, not the verbose progress chatter.
            !(lower.starts_with("time:")
                || lower.contains("underrun")
                || lower.starts_with("stream started"))
        });
        let stopping = Arc::new(AtomicBool::new(false));
        let thread_stats = Arc::clone(&stats);
        let thread_stop = Arc::clone(&stopping);
        let tail = stderr.handle();
        let writer = std::thread::Builder::new()
            .name("lull-pacat".into())
            .spawn(move || {
                let result = playback_loop(stdin, source, &thread_stats, &thread_stop);
                if let Err(e) = result
                    && !thread_stop.load(Ordering::Relaxed)
                {
                    std::thread::sleep(Duration::from_millis(50));
                    let why = tail.tail();
                    thread_stats.fail(if why.is_empty() {
                        format!("playback ended: {e}")
                    } else {
                        format!("playback ended: {why}")
                    });
                }
            })
            .map_err(PlatformError::Io)?;
        Ok(Box::new(PipeStream {
            child,
            worker: Some(writer),
            stderr,
            stats,
            stopping,
            format,
        }))
    }
}

fn playback_loop(
    mut stdin: ChildStdin,
    mut source: PlaybackSource,
    stats: &SharedStats,
    stop: &AtomicBool,
) -> std::io::Result<()> {
    let frame_time = Duration::from_millis(u64::from(lull_media::FRAME_MS));
    let mut pacer = WallClockPacer::new(Instant::now(), frame_time, PLAYBACK_LEAD);
    let format = StreamFormat {
        sample_rate: lull_media::SAMPLE_RATE,
        channels: 1,
        sample_type: SampleType::F32,
    };
    let mut pcm = lull_media::SILENT_FRAME;
    let mut float = vec![0f32; lull_media::FRAME_SAMPLES];
    let mut bytes = vec![0u8; lull_media::FRAME_SAMPLES * 2];
    while !stop.load(Ordering::Relaxed) {
        let pace = pacer.poll(Instant::now());
        if pace.reanchored {
            log::debug!("playback writer re-anchored to the wall clock");
        }
        for _ in 0..pace.frames {
            match &mut source {
                PlaybackSource::Pcm16k(pull) => {
                    if !pull(&mut pcm) {
                        pcm = lull_media::SILENT_FRAME;
                        stats.starved.fetch_add(1, Ordering::Relaxed);
                    }
                }
                PlaybackSource::Device(fill) => {
                    float.fill(0.0);
                    fill(format, &mut float);
                    for (d, &s) in pcm.iter_mut().zip(&float) {
                        *d = f32_to_i16(s);
                    }
                }
            }
            for (b, s) in bytes.chunks_exact_mut(2).zip(pcm.iter()) {
                b.copy_from_slice(&s.to_le_bytes());
            }
            stdin.write_all(&bytes)?;
        }
        let wait = pace.next.saturating_duration_since(Instant::now());
        std::thread::sleep(wait.clamp(Duration::from_millis(1), frame_time));
    }
    Ok(())
}

/// A `parec` or `pacat` child plus its worker thread.
struct PipeStream {
    child: Child,
    worker: Option<JoinHandle<()>>,
    stderr: StderrTail,
    stats: Arc<SharedStats>,
    stopping: Arc<AtomicBool>,
    format: StreamFormat,
}

impl AudioStream for PipeStream {
    fn format(&self) -> StreamFormat {
        self.format
    }

    fn stats(&self) -> StreamStats {
        let mut s = self.stats.snapshot();
        if s.error.is_none()
            && self.worker.as_ref().is_some_and(|w| w.is_finished())
            && !self.stopping.load(Ordering::Relaxed)
        {
            s.error = Some(self.stderr.tail());
        }
        s
    }

    fn stop(&mut self) {
        self.stopping.store(true, Ordering::Relaxed);
        kill(&mut self.child);
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
        self.stderr.join();
    }
}

impl Drop for PipeStream {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOURCES: &str = "Source #57
\tState: SUSPENDED
\tName: alsa_output.pci-0000_c6_00.6.HiFi__Speaker__sink.monitor
\tDescription: Monitor of Speaker
Source #59
\tState: RUNNING
\tName: alsa_input.usb-mic
\tDescription: USB Microphone
\tProperties:
\t\tdevice.description = \"USB Microphone\"
Source #60
\tName: alsa_input.internal
";

    #[test]
    fn parses_sources_and_skips_monitors() {
        let devs = parse_devices(SOURCES, Kind::Source, Some("alsa_input.internal"));
        assert_eq!(
            devs,
            vec![
                AudioDevice {
                    id: "alsa_input.usb-mic".into(),
                    name: "USB Microphone".into(),
                    is_default: false,
                },
                AudioDevice {
                    id: "alsa_input.internal".into(),
                    name: "alsa_input.internal".into(),
                    is_default: true,
                },
            ]
        );
    }

    #[test]
    fn parses_sinks() {
        let listing = "Sink #57\n\tName: speaker\n\tDescription: Speaker\n";
        let devs = parse_devices(listing, Kind::Sink, Some("speaker"));
        assert_eq!(devs.len(), 1);
        assert!(devs[0].is_default);
        // A sink listing is not mistaken for sources.
        assert!(parse_devices(listing, Kind::Source, None).is_empty());
    }

    #[test]
    fn parses_defaults() {
        let info =
            "Server Name: PulseAudio (on PipeWire 1.4.2)\nDefault Sink: spk\nDefault Source: mic\n";
        assert_eq!(parse_default(info, Kind::Sink).as_deref(), Some("spk"));
        assert_eq!(parse_default(info, Kind::Source).as_deref(), Some("mic"));
        assert_eq!(parse_default("Default Source: \n", Kind::Source), None);
    }

    #[test]
    #[ignore = "needs a running PulseAudio or PipeWire sound server; plays 1 s of silence"]
    fn playback_is_paced_by_the_wall_clock() {
        use std::sync::atomic::AtomicU32;
        let pa = PulseAudio::detect().unwrap();
        let pulled = Arc::new(AtomicU32::new(0));
        let p = Arc::clone(&pulled);
        let source = PlaybackSource::Pcm16k(Box::new(move |f| {
            p.fetch_add(1, Ordering::Relaxed);
            f.fill(0);
            true
        }));
        let mut stream = AudioOutput::start(&pa, &PlaybackRequest::default(), source).unwrap();
        std::thread::sleep(Duration::from_secs(1));
        stream.stop();
        // 50 frames per second plus the lead, not the seconds a pipe holds.
        let n = pulled.load(Ordering::Relaxed);
        assert!((45..=56).contains(&n), "pulled {n} frames in 1 s");
        assert_eq!(stream.stats().error, None);
    }

    #[test]
    #[ignore = "needs a running PulseAudio or PipeWire sound server"]
    fn records_from_default_source() {
        let pa = PulseAudio::detect().unwrap();
        let (cb, rx, _) = super::super::channel_callback(64);
        let stream = AudioInput::start(&pa, &CaptureRequest::default(), cb).unwrap();
        let block = rx.recv_timeout(Duration::from_secs(3)).unwrap();
        assert_eq!(block.samples.len(), 320);
        drop(stream);
    }
}
