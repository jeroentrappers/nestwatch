// SPDX-License-Identifier: AGPL-3.0-or-later
//! Station media: capture → gate → sessions (media.md §1, §3), talk-back
//! (§4.3, FR-TB-1..3) and the on-demand camera (§5, FR-CAM-1..4).

use std::collections::VecDeque;
use std::sync::Arc;

use nestwatch_media::capture::{CaptureChain, GainSetting};
use nestwatch_media::codec::{AudioEncoder, new_encoder};
use nestwatch_media::gate::{AudioRing, GateEvent};
use nestwatch_media::jitter::JitterBuffer;
use nestwatch_media::level::LEVEL_FLOOR_DBFS;
use nestwatch_media::video::adapt::VideoAdaptation;
use nestwatch_media::video::{encode_rgb, encode_rgba, fit_within, night_filter, scale_image};
use nestwatch_media::{AudioFrame, CodecId, FRAME_TICKS, FrameFlags, SILENT_FRAME};
use nestwatch_proto::control::{
    CameraFacing, Control, TalkStatus, VideoStart, VideoState, VideoStateKind,
};
use nestwatch_proto::record::{AudioFlags, AudioHeader, AudioStream, VIDEO_CODEC_JPEG};
use nestwatch_proto::video::VideoFrame;

use super::session::Cmd;
use super::{Inner, Shared, StationEvent};

/// Output size and JPEG quality of one encoding.
type EncodeKey = (u32, u32, u8);

/// Interleaved device samples.
pub(crate) enum Samples<'a> {
    I16(&'a [i16]),
    F32(&'a [f32]),
}

/// One encoded stream and its 2 s ring (media.md §3.1), per codec in use.
pub(crate) struct CodecRing {
    pub codec: CodecId,
    pub ring: AudioRing,
    encoder: Box<dyn AudioEncoder>,
    /// Sessions whose gate reads this ring.
    pub users: usize,
}

/// The capture side of the audio pipeline.
pub(crate) struct Audio {
    chain: Option<(u32, u16, CaptureChain)>,
    gain: GainSetting,
    pub rings: Vec<CodecRing>,
    frames: u64,
    peak: Option<f32>,
    pub last_level: f32,
}

impl Audio {
    pub fn new(gain: GainSetting) -> Self {
        Self {
            chain: None,
            gain,
            rings: Vec::new(),
            frames: 0,
            peak: None,
            last_level: LEVEL_FLOOR_DBFS,
        }
    }

    pub fn set_gain(&mut self, gain: GainSetting) {
        self.gain = gain;
        if let Some((_, _, c)) = &mut self.chain {
            c.set_gain(gain);
        }
    }

    /// Peak frame level since the previous call (protocol.md §7.5).
    pub fn take_level(&mut self) -> f32 {
        let v = self.peak.take().unwrap_or(LEVEL_FLOOR_DBFS);
        self.last_level = v;
        v
    }

    /// The ring for `codec`, created on first use.
    pub fn ring_index(&mut self, codec: CodecId) -> Option<usize> {
        if let Some(i) = self.rings.iter().position(|r| r.codec == codec) {
            return Some(i);
        }
        let encoder = new_encoder(codec).ok()?;
        self.rings.push(CodecRing {
            codec,
            ring: AudioRing::new(),
            encoder,
            users: 0,
        });
        Some(self.rings.len() - 1)
    }
}

/// Talk-back playback state (FR-TB-1, FR-TB-3).
#[derive(Default)]
pub(crate) struct Talk {
    /// Session holding talk-back.
    pub talker: Option<u64>,
    jitter: Option<JitterBuffer>,
    pending: VecDeque<i16>,
}

/// Whether the camera can be used right now (protocol.md §7.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum CameraAvailability {
    /// It can be started.
    #[default]
    Available,
    /// No camera, or the platform forbids it now (iOS background).
    Unavailable,
    /// The OS permission was denied.
    Denied,
}

/// What the camera should deliver (media.md §5): the largest requested
/// size and frame rate over all watching listeners.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CameraRequest {
    /// Largest requested width.
    pub max_w: u32,
    /// Largest requested height.
    pub max_h: u32,
    /// Highest requested frame rate.
    pub max_fps: u32,
    /// Camera to use (FR-CAM-4).
    pub facing: CameraFacing,
    /// Night image requested (FR-CAM-4); the engine applies the filter.
    pub night: bool,
}

/// Pixel layout of a [`CameraFrame`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PixelFormat {
    /// 3 bytes per pixel.
    Rgb,
    /// 4 bytes per pixel, alpha ignored.
    Rgba,
}

impl PixelFormat {
    fn channels(self) -> usize {
        match self {
            Self::Rgb => 3,
            Self::Rgba => 4,
        }
    }
}

/// One captured camera image, tightly packed rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CameraFrame {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Layout of `data`.
    pub format: PixelFormat,
    /// Pixels.
    pub data: Vec<u8>,
}

/// Camera aggregation over sessions.
#[derive(Debug)]
pub(crate) struct Camera {
    pub availability: CameraAvailability,
    pub facing: CameraFacing,
    pub night: bool,
    pub wanted: Option<CameraRequest>,
    frame_id: u32,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            availability: CameraAvailability::Available,
            facing: CameraFacing::Back,
            night: false,
            wanted: None,
            frame_id: 0,
        }
    }
}

/// One listener's open camera view.
#[derive(Debug)]
pub(crate) struct Watch {
    pub max_w: u32,
    pub max_h: u32,
    pub max_fps: u32,
    pub adapt: VideoAdaptation,
    announced: Option<(u32, u32, u32)>,
}

fn video_state(state: VideoStateKind) -> Control {
    Control::VideoState(VideoState {
        state,
        codec: None,
        w: None,
        h: None,
        fps: None,
    })
}

impl Shared {
    /// Capture → chain → ring → gates → sessions (media.md §1, §3).
    pub(crate) fn push_capture(&self, samples: Samples<'_>, rate: u32, channels: u16) {
        let mut g = self.lock();
        if !g.running || rate == 0 || channels == 0 {
            return;
        }
        let Inner {
            audio, sessions, ..
        } = &mut *g;
        if !matches!(&audio.chain, Some((r, c, _)) if *r == rate && *c == channels) {
            match CaptureChain::new(rate, channels, audio.gain) {
                Ok(c) => audio.chain = Some((rate, channels, c)),
                Err(_) => return,
            }
        }
        let (_, _, chain) = audio.chain.as_mut().expect("set above");
        match samples {
            Samples::I16(s) => chain.push_i16(s),
            Samples::F32(s) => chain.push_f32(s),
        }
        let mut changed = false;
        while let Some(f) = chain.pop_frame() {
            audio.peak = Some(audio.peak.map_or(f.level_dbfs, |p| p.max(f.level_dbfs)));
            let ts = (audio.frames as u32).wrapping_mul(FRAME_TICKS);
            audio.frames += 1;
            let mut pushed = vec![None; audio.rings.len()];
            for (slot, r) in pushed.iter_mut().zip(audio.rings.iter_mut()) {
                if r.users == 0 {
                    continue;
                }
                if let Ok(payload) = r.encoder.encode(&f.pcm) {
                    *slot = Some(r.ring.push(r.codec, ts, f.level_dbfs, payload));
                }
            }
            for s in sessions.values_mut() {
                let (Some(gate), Some(ri)) = (s.gate.as_mut(), s.ring) else {
                    continue;
                };
                let Some(index) = pushed.get(ri).copied().flatten() else {
                    continue;
                };
                for ev in gate.process(&audio.rings[ri].ring, index) {
                    match ev {
                        GateEvent::Frame(af) => {
                            let header = AudioHeader {
                                stream: AudioStream::Station,
                                codec: af.codec.0,
                                flags: AudioFlags(af.flags.bits()),
                                seq: af.seq,
                                timestamp: af.timestamp,
                            };
                            s.send(Cmd::Audio(header, af.payload));
                        }
                        GateEvent::Activity { open } => {
                            s.send(Cmd::Control(Control::Activity { open }));
                            changed = true;
                        }
                    }
                }
            }
        }
        if changed {
            self.publish(&g);
        }
    }

    /// 16 kHz talk-back audio for the speaker (FR-TB-3).
    pub(crate) fn pull_talkback(&self, out: &mut [i16]) -> bool {
        let now = self.now();
        let mut g = self.lock();
        let volume = g.settings.talkback_volume;
        let talking = g.talk.talker.is_some();
        let talk = &mut g.talk;
        let mut i = 0;
        while i < out.len() {
            if talk.pending.is_empty() {
                let pcm = match talk.jitter.as_mut() {
                    Some(j) => j.pull(now).pcm(),
                    None => SILENT_FRAME,
                };
                talk.pending
                    .extend(pcm.iter().map(|&s| (f32::from(s) * volume).round() as i16));
            }
            while i < out.len() {
                match talk.pending.pop_front() {
                    Some(s) => {
                        out[i] = s;
                        i += 1;
                    }
                    None => break,
                }
            }
        }
        talking
    }

    /// `talk-request` (protocol.md §7.8): one talker at a time (FR-TB-1).
    pub(crate) fn on_talk_request(&self, g: &mut Inner, id: u64) {
        let reply = if !g.settings.talk_enabled {
            Control::TalkState {
                state: TalkStatus::Disabled,
                by: None,
            }
        } else {
            match g.talk.talker {
                Some(t) if t != id => Control::TalkState {
                    state: TalkStatus::Busy,
                    by: g.sessions.get(&t).and_then(|s| s.name.clone()),
                },
                current => {
                    if current.is_none() {
                        let codec = g.sessions.get(&id).and_then(|s| s.codec);
                        g.talk.talker = Some(id);
                        g.talk.jitter = codec.and_then(|c| JitterBuffer::new(c).ok());
                        g.talk.pending.clear();
                        let name = g.sessions.get(&id).and_then(|s| s.name.clone());
                        self.emit(StationEvent::TalkStarted {
                            name: name.unwrap_or_default(),
                        });
                    }
                    Control::TalkState {
                        state: TalkStatus::Granted,
                        by: None,
                    }
                }
            }
        };
        if let Some(s) = g.sessions.get(&id) {
            s.send(Cmd::Control(reply));
        }
        self.send_status(g, false);
        self.publish(g);
    }

    /// Ends talk-back of `id`, answering `reply` if it is still connected.
    pub(crate) fn release_talk(&self, g: &mut Inner, id: u64, reply: TalkStatus) {
        if g.talk.talker != Some(id) {
            return;
        }
        g.talk.talker = None;
        g.talk.jitter = None;
        g.talk.pending.clear();
        self.emit(StationEvent::TalkEnded);
        if let Some(s) = g.sessions.get(&id) {
            s.send(Cmd::Control(Control::TalkState {
                state: reply,
                by: None,
            }));
        }
        self.send_status(g, false);
        self.publish(g);
    }

    /// AUDIO stream 1: only the granted talker is played (SESSION-10).
    pub(crate) fn on_talk_audio(
        &self,
        g: &mut Inner,
        id: u64,
        header: AudioHeader,
        payload: Vec<u8>,
    ) {
        if g.talk.talker != Some(id) {
            return;
        }
        let now = self.now();
        if let Some(j) = g.talk.jitter.as_mut() {
            j.push(
                AudioFrame {
                    codec: CodecId(header.codec),
                    flags: FrameFlags(header.flags.0),
                    seq: header.seq,
                    timestamp: header.timestamp,
                    payload,
                },
                now,
            );
        }
    }

    /// `video-start` (protocol.md §7.7, media.md §5).
    pub(crate) fn on_video_start(&self, g: &mut Inner, id: u64, v: VideoStart) {
        let refusal = if !g.settings.camera_enabled {
            Some(VideoStateKind::Disabled)
        } else {
            match g.camera.availability {
                CameraAvailability::Unavailable => Some(VideoStateKind::Unavailable),
                CameraAvailability::Denied => Some(VideoStateKind::Denied),
                CameraAvailability::Available if !v.codecs.contains(&VIDEO_CODEC_JPEG) => {
                    Some(VideoStateKind::Unavailable)
                }
                CameraAvailability::Available => None,
            }
        };
        let now = self.now();
        let Some(s) = g.sessions.get_mut(&id) else {
            return;
        };
        if let Some(state) = refusal {
            s.send(Cmd::Control(video_state(state)));
            return;
        }
        let max_fps = v.max_fps.clamp(1, 30);
        s.video = Some(Watch {
            max_w: v.max_w.clamp(16, 4096),
            max_h: v.max_h.clamp(16, 4096),
            max_fps,
            adapt: VideoAdaptation::new(max_fps, now),
            announced: None,
        });
        self.recompute_camera(g);
        self.send_status(g, false);
        self.publish(g);
    }

    /// `video-stop` or the end of a session.
    pub(crate) fn on_video_stop(&self, g: &mut Inner, id: u64) {
        let Some(s) = g.sessions.get_mut(&id) else {
            return;
        };
        if s.video.take().is_some() {
            s.send(Cmd::Control(video_state(VideoStateKind::Stopped)));
            self.recompute_camera(g);
            self.send_status(g, false);
            self.publish(g);
        }
    }

    /// `video-options` (FR-CAM-4).
    pub(crate) fn on_video_options(&self, g: &mut Inner, facing: CameraFacing, night: bool) {
        g.camera.facing = facing;
        g.camera.night = night;
        self.recompute_camera(g);
    }

    /// Ends every camera view with `state` (FR-ST-8, permission loss).
    pub(crate) fn stop_all_video(&self, g: &mut Inner, state: VideoStateKind) {
        for s in g.sessions.values_mut() {
            if s.video.take().is_some() {
                s.send(Cmd::Control(video_state(state)));
            }
        }
        self.recompute_camera(g);
        self.send_status(g, false);
    }

    /// Emits `CameraWanted` / `CameraNotWanted` when the aggregate changes.
    pub(crate) fn recompute_camera(&self, g: &mut Inner) {
        let watches: Vec<_> = g
            .sessions
            .values()
            .filter_map(|s| s.video.as_ref())
            .collect();
        // State is published before the event, so anyone reacting to the
        // event reads a snapshot that already agrees with it.
        if watches.is_empty() {
            if g.camera.wanted.take().is_some() {
                self.publish(g);
                self.emit(StationEvent::CameraNotWanted);
            }
            return;
        }
        let req = CameraRequest {
            max_w: watches.iter().map(|w| w.max_w).max().unwrap_or(640),
            max_h: watches.iter().map(|w| w.max_h).max().unwrap_or(480),
            max_fps: watches.iter().map(|w| w.max_fps).max().unwrap_or(10),
            facing: g.camera.facing,
            night: g.camera.night,
        };
        if g.camera.wanted.as_ref() != Some(&req) {
            g.camera.wanted = Some(req.clone());
            self.publish(g);
            self.emit(StationEvent::CameraWanted(req));
        }
    }

    /// Scales, filters and encodes a camera frame per watching listener at
    /// its adapted rate and quality (media.md §5). Encoding runs outside
    /// the lock; identical outputs are encoded once.
    pub(crate) fn push_camera_frame(&self, frame: &CameraFrame) {
        let ch = frame.format.channels();
        let (w, h) = (frame.width, frame.height);
        if w == 0 || h == 0 || frame.data.len() < w as usize * h as usize * ch {
            return;
        }
        let now = self.now();
        let (frame_id, night, plan) = {
            let mut g = self.lock();
            if !g.running || g.camera.wanted.is_none() {
                return;
            }
            g.camera.frame_id = g.camera.frame_id.wrapping_add(1);
            let frame_id = g.camera.frame_id;
            let night = g.camera.night;
            let mut plan = Vec::new();
            for (&id, s) in g.sessions.iter_mut() {
                let Some(watch) = s.video.as_mut() else {
                    continue;
                };
                if !watch.adapt.take_frame(now) {
                    continue;
                }
                let (dw, dh) = fit_within(w, h, watch.max_w, watch.max_h);
                let (dw, dh) = (dw.min(u32::from(u16::MAX)), dh.min(u32::from(u16::MAX)));
                plan.push((id, dw, dh, watch.adapt.quality(), watch.adapt.fps()));
            }
            (frame_id, night, plan)
        };
        if plan.is_empty() {
            return;
        }
        let mut scaled: Vec<((u32, u32), Vec<u8>)> = Vec::new();
        let mut encoded: Vec<(EncodeKey, Arc<Vec<u8>>)> = Vec::new();
        let mut out = Vec::new();
        for &(id, dw, dh, q, fps) in &plan {
            let jpeg = if let Some((_, j)) = encoded.iter().find(|(k, _)| *k == (dw, dh, q)) {
                j.clone()
            } else {
                if !scaled.iter().any(|(k, _)| *k == (dw, dh)) {
                    let mut px = if (dw, dh) == (w, h) {
                        frame.data[..w as usize * h as usize * ch].to_vec()
                    } else {
                        scale_image(&frame.data, w, h, ch, dw, dh)
                    };
                    if night {
                        night_filter(&mut px, ch);
                    }
                    scaled.push(((dw, dh), px));
                }
                let px = &scaled
                    .iter()
                    .find(|(k, _)| *k == (dw, dh))
                    .expect("added")
                    .1;
                let res = match frame.format {
                    PixelFormat::Rgb => encode_rgb(px, dw as u16, dh as u16, q),
                    PixelFormat::Rgba => encode_rgba(px, dw as u16, dh as u16, q),
                };
                let Ok(bytes) = res else { continue };
                let j = Arc::new(bytes);
                encoded.push(((dw, dh, q), j.clone()));
                j
            };
            out.push((id, dw, dh, fps, jpeg));
        }
        let mut g = self.lock();
        for (id, dw, dh, fps, jpeg) in out {
            let Some(s) = g.sessions.get_mut(&id) else {
                continue;
            };
            let Some(watch) = s.video.as_mut() else {
                continue;
            };
            if watch.announced != Some((dw, dh, fps)) {
                watch.announced = Some((dw, dh, fps));
                s.send(Cmd::Control(Control::VideoState(VideoState {
                    state: VideoStateKind::Streaming,
                    codec: Some(VIDEO_CODEC_JPEG),
                    w: Some(dw),
                    h: Some(dh),
                    fps: Some(fps),
                })));
            }
            s.send(Cmd::Video(VideoFrame {
                codec: VIDEO_CODEC_JPEG,
                frame_id,
                capture_ms: now as u32,
                width: dw as u16,
                height: dh as u16,
                data: (*jpeg).clone(),
            }));
        }
    }

    /// A queued frame of `id` was replaced: adapt (protocol.md §8, media.md §5).
    pub(crate) fn on_video_replaced(&self, g: &mut Inner, id: u64) {
        let now = self.now();
        if let Some(w) = g.sessions.get_mut(&id).and_then(|s| s.video.as_mut()) {
            w.adapt.on_frame_replaced(now);
        }
    }
}
