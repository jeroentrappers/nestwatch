// SPDX-License-Identifier: AGPL-3.0-or-later
//! Web Audio for the browser listener (platforms.md §2.5).
//!
//! - Playback: a 16 kHz `AudioContext` created on the first user gesture.
//!   An `AudioWorklet` (`audio-worklet.js`, `lull-playback`) asks the main
//!   thread for frames at the device's pace; each request pulls 20 ms
//!   frames with `ListenerEngine::pull_playback` (CORE-12). Without
//!   `AudioWorklet` a `ScriptProcessorNode` pulls directly.
//! - Talk-back: `getUserMedia` (secure contexts only) into a separate
//!   context at the device rate, blocks fed to `push_talk_capture`.
//! - Alert sound: two short tones on the playback context.
//!
//! Counters are mirrored to `window.__lullAudio` for diagnostics and tests
//! (headless browsers cannot be heard).

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use js_sys::{Array, Float32Array, Object, Reflect};
use lull_core::listener::ListenerEngine;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::{
    AudioContext, AudioContextOptions, AudioContextState, AudioProcessingEvent, AudioWorkletNode,
    AudioWorkletNodeOptions, MediaStream, MediaStreamConstraints, MessageEvent,
    ScriptProcessorNode,
};

const FRAME: usize = 320;

/// Converts and forwards one block of microphone samples.
type MicFeed = dyn Fn(&[f32]);
const WORKLET_URL: &str = "audio-worklet.js";

#[derive(Default)]
struct Counters {
    pulled: f64,
    non_silent: f64,
    peak: f64,
    underruns: f64,
    played: f64,
    mic_blocks: f64,
    mic_samples: f64,
}

struct Mic {
    stream: MediaStream,
    ctx: AudioContext,
    _node: JsValue,
    _closure: JsValue,
}

#[derive(Default)]
struct Inner {
    ctx: Option<AudioContext>,
    node: Option<JsValue>,
    closures: Vec<JsValue>,
    mic: Option<Mic>,
    unlocked: bool,
}

/// Browser audio for one listener engine.
pub struct Audio {
    engine: ListenerEngine,
    inner: RefCell<Inner>,
    counters: RefCell<Counters>,
    stats: Object,
    mic_gen: Cell<u64>,
    on_unlocked: RefCell<Option<Box<dyn Fn()>>>,
}

impl std::fmt::Debug for Audio {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Audio").finish_non_exhaustive()
    }
}

fn set(o: &Object, k: &str, v: impl Into<JsValue>) {
    let _ = Reflect::set(o, &JsValue::from_str(k), &v.into());
}

fn log(msg: &str) {
    web_sys::console::log_1(&JsValue::from_str(msg));
}

impl Audio {
    /// Prepares audio for `engine`; nothing plays until [`unlock`](Self::unlock).
    pub fn new(engine: ListenerEngine) -> Rc<Self> {
        let stats = Object::new();
        set(&stats, "mode", "locked");
        if let Some(w) = web_sys::window() {
            let _ = Reflect::set(&w, &JsValue::from_str("__lullAudio"), &stats);
        }
        Rc::new(Self {
            engine,
            inner: RefCell::new(Inner::default()),
            counters: RefCell::new(Counters::default()),
            stats,
            mic_gen: Cell::new(0),
            on_unlocked: RefCell::new(None),
        })
    }

    /// Called once playback runs.
    pub fn on_unlocked(&self, f: impl Fn() + 'static) {
        *self.on_unlocked.borrow_mut() = Some(Box::new(f));
        if self.inner.borrow().unlocked {
            self.fire_unlocked();
        }
    }

    fn fire_unlocked(&self) {
        if let Some(f) = &*self.on_unlocked.borrow() {
            f();
        }
    }

    /// Whether playback runs.
    pub fn is_unlocked(&self) -> bool {
        self.inner.borrow().unlocked
    }

    /// Creates or resumes the playback context. Call from a user gesture
    /// (browser autoplay rule).
    pub fn unlock(self: &Rc<Self>) {
        if let Some(ctx) = &self.inner.borrow().ctx {
            let _ = ctx.resume();
            return;
        }
        let opts = AudioContextOptions::new();
        opts.set_sample_rate(16_000.0);
        let ctx = match AudioContext::new_with_context_options(&opts) {
            Ok(c) => c,
            Err(e) => {
                web_sys::console::error_2(&JsValue::from_str("Lull: no 16 kHz AudioContext"), &e);
                return;
            }
        };
        let _ = ctx.resume();
        set(&self.stats, "contextRate", ctx.sample_rate());
        self.inner.borrow_mut().ctx = Some(ctx.clone());
        let this = self.clone();
        spawn_local(async move {
            let worklet = match ctx.audio_worklet() {
                Ok(w) => match w.add_module(WORKLET_URL) {
                    Ok(p) => JsFuture::from(p).await.is_ok(),
                    Err(_) => false,
                },
                Err(_) => false,
            };
            let ok = if worklet {
                this.start_worklet(&ctx)
            } else {
                false
            };
            if !ok {
                this.start_script_processor(&ctx);
            }
            // Wait for the context to run (resume may need the gesture).
            for _ in 0..200 {
                if ctx.state() == AudioContextState::Running {
                    break;
                }
                gloo_timers::future::sleep(std::time::Duration::from_millis(50)).await;
            }
            let running = ctx.state() == AudioContextState::Running;
            set(
                &this.stats,
                "state",
                if running { "running" } else { "suspended" },
            );
            if running {
                this.inner.borrow_mut().unlocked = true;
                this.fire_unlocked();
            }
        });
    }

    fn pull_frame(&self, out: &mut [f32]) {
        let mut pcm = [0i16; FRAME];
        self.engine.pull_playback(&mut pcm);
        let mut peak = 0i32;
        for (o, s) in out.iter_mut().zip(pcm.iter()) {
            *o = f32::from(*s) / 32768.0;
            peak = peak.max(i32::from(*s).abs());
        }
        let mut c = self.counters.borrow_mut();
        c.pulled += 1.0;
        if peak > 0 {
            c.non_silent += 1.0;
        }
        c.peak = c.peak.max(f64::from(peak) / 32768.0);
    }

    fn publish_counters(&self) {
        let c = self.counters.borrow();
        set(&self.stats, "pulled", c.pulled);
        set(&self.stats, "nonSilent", c.non_silent);
        set(&self.stats, "peak", c.peak);
        set(&self.stats, "underruns", c.underruns);
        set(&self.stats, "played", c.played);
        set(&self.stats, "micBlocks", c.mic_blocks);
        set(&self.stats, "micSamples", c.mic_samples);
        set(&self.stats, "updated", js_sys::Date::now());
    }

    fn start_worklet(self: &Rc<Self>, ctx: &AudioContext) -> bool {
        let opts = AudioWorkletNodeOptions::new();
        opts.set_number_of_inputs(0);
        opts.set_number_of_outputs(1);
        opts.set_output_channel_count(&Array::of1(&JsValue::from(1)));
        let node = match AudioWorkletNode::new_with_options(ctx, "lull-playback", &opts) {
            Ok(n) => n,
            Err(_) => return false,
        };
        let Ok(port) = node.port() else { return false };
        let this = self.clone();
        let port2 = port.clone();
        let closure = Closure::<dyn FnMut(MessageEvent)>::new(move |e: MessageEvent| {
            let data = e.data();
            let need = Reflect::get(&data, &JsValue::from_str("need"))
                .ok()
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0) as usize;
            {
                let mut c = this.counters.borrow_mut();
                if let Some(u) = Reflect::get(&data, &JsValue::from_str("underruns"))
                    .ok()
                    .and_then(|v| v.as_f64())
                {
                    c.underruns = u;
                }
                if let Some(p) = Reflect::get(&data, &JsValue::from_str("played"))
                    .ok()
                    .and_then(|v| v.as_f64())
                {
                    c.played = p;
                }
            }
            let mut buf = [0f32; FRAME];
            for _ in 0..need.min(50) {
                this.pull_frame(&mut buf);
                let arr = Float32Array::new_with_length(FRAME as u32);
                arr.copy_from(&buf);
                let _ = port2.post_message_with_transferable(&arr, &Array::of1(&arr.buffer()));
            }
            this.publish_counters();
        });
        port.set_onmessage(Some(closure.as_ref().unchecked_ref()));
        if node.connect_with_audio_node(&ctx.destination()).is_err() {
            return false;
        }
        set(&self.stats, "mode", "worklet");
        log("Lull: audio playback through AudioWorklet at 16 kHz");
        let mut inner = self.inner.borrow_mut();
        inner.node = Some(node.into());
        inner.closures.push(closure.into_js_value());
        true
    }

    fn start_script_processor(self: &Rc<Self>, ctx: &AudioContext) {
        let node: ScriptProcessorNode = match ctx
            .create_script_processor_with_buffer_size_and_number_of_input_channels_and_number_of_output_channels(
                1024, 0, 1,
            ) {
            Ok(n) => n,
            Err(e) => {
                web_sys::console::error_2(&JsValue::from_str("Lull: no ScriptProcessor"), &e);
                return;
            }
        };
        let this = self.clone();
        // Buffers are powers of two (1024 = 64 ms), frames are 320 samples:
        // carry the rest of a pulled frame over to the next callback.
        let mut carry: std::collections::VecDeque<f32> = std::collections::VecDeque::new();
        let closure =
            Closure::<dyn FnMut(AudioProcessingEvent)>::new(move |e: AudioProcessingEvent| {
                let Ok(out) = e.output_buffer() else { return };
                let len = out.length() as usize;
                let mut frame = [0f32; FRAME];
                while carry.len() < len {
                    this.pull_frame(&mut frame);
                    carry.extend(frame);
                }
                let data: Vec<f32> = carry.drain(..len).collect();
                let _ = out.copy_to_channel(&data, 0);
                this.counters.borrow_mut().played += len as f64;
                this.publish_counters();
            });
        node.set_onaudioprocess(Some(closure.as_ref().unchecked_ref()));
        let _ = node.connect_with_audio_node(&ctx.destination());
        set(&self.stats, "mode", "script-processor");
        log("Lull: audio playback through ScriptProcessorNode (no AudioWorklet)");
        let mut inner = self.inner.borrow_mut();
        inner.node = Some(node.into());
        inner.closures.push(closure.into_js_value());
    }

    /// Plays the alert sound (FR-AL-1), if audio is unlocked.
    pub fn beep(&self) {
        let inner = self.inner.borrow();
        let Some(ctx) = inner.ctx.as_ref().filter(|_| inner.unlocked) else {
            return;
        };
        let t0 = ctx.current_time();
        for (i, freq) in [880.0f32, 660.0].into_iter().enumerate() {
            let (Ok(osc), Ok(gain)) = (ctx.create_oscillator(), ctx.create_gain()) else {
                return;
            };
            let start = t0 + i as f64 * 0.22;
            osc.frequency().set_value(freq);
            let g = gain.gain();
            g.set_value(0.0);
            let _ = g.set_value_at_time(0.0, start);
            let _ = g.linear_ramp_to_value_at_time(0.25, start + 0.02);
            let _ = g.linear_ramp_to_value_at_time(0.0, start + 0.2);
            let _ = osc.connect_with_audio_node(&gain);
            let _ = gain.connect_with_audio_node(&ctx.destination());
            let _ = osc.start_with_when(start);
            let _ = osc.stop_with_when(start + 0.21);
        }
    }

    /// Starts microphone capture for talk-back (secure contexts only).
    pub fn start_mic(self: &Rc<Self>) {
        if self.inner.borrow().mic.is_some() {
            return;
        }
        let Some(window) = web_sys::window() else {
            return;
        };
        if !window.is_secure_context() {
            return;
        }
        let Ok(devices) = window.navigator().media_devices() else {
            return;
        };
        let gen_ = self.mic_gen.get() + 1;
        self.mic_gen.set(gen_);
        let constraints = MediaStreamConstraints::new();
        let audio = Object::new();
        set(&audio, "echoCancellation", true);
        set(&audio, "noiseSuppression", true);
        set(&audio, "channelCount", 1);
        constraints.set_audio(&audio);
        let Ok(promise) = devices.get_user_media_with_constraints(&constraints) else {
            return;
        };
        let this = self.clone();
        spawn_local(async move {
            let stream: MediaStream = match JsFuture::from(promise).await {
                Ok(s) => s.unchecked_into(),
                Err(e) => {
                    web_sys::console::warn_2(
                        &JsValue::from_str("Lull: microphone unavailable"),
                        &e,
                    );
                    return;
                }
            };
            if this.mic_gen.get() != gen_ {
                stop_tracks(&stream);
                return;
            }
            let Ok(ctx) = AudioContext::new() else {
                stop_tracks(&stream);
                return;
            };
            let _ = ctx.resume();
            let Ok(source) = ctx.create_media_stream_source(&stream) else {
                stop_tracks(&stream);
                return;
            };
            let rate = ctx.sample_rate() as u32;
            let worklet = match ctx.audio_worklet() {
                Ok(w) => match w.add_module(WORKLET_URL) {
                    Ok(p) => JsFuture::from(p).await.is_ok(),
                    Err(_) => false,
                },
                Err(_) => false,
            };
            if this.mic_gen.get() != gen_ {
                stop_tracks(&stream);
                let _ = ctx.close();
                return;
            }
            let feed: Rc<MicFeed> = {
                let this = this.clone();
                Rc::new(move |samples: &[f32]| {
                    let pcm: Vec<i16> = samples
                        .iter()
                        .map(|s| (s.clamp(-1.0, 1.0) * 32767.0) as i16)
                        .collect();
                    this.engine.push_talk_capture(&pcm, rate, 1);
                    let mut c = this.counters.borrow_mut();
                    c.mic_blocks += 1.0;
                    c.mic_samples += pcm.len() as f64;
                })
            };
            let mut node_and_closure: Option<(JsValue, JsValue)> = None;
            if worklet && let Ok(node) = AudioWorkletNode::new(&ctx, "lull-capture") {
                if let Ok(port) = node.port() {
                    let this2 = this.clone();
                    let feed = feed.clone();
                    let closure =
                        Closure::<dyn FnMut(MessageEvent)>::new(move |e: MessageEvent| {
                            if let Ok(arr) = e.data().dyn_into::<Float32Array>() {
                                feed(&arr.to_vec());
                                this2.publish_counters();
                            }
                        });
                    port.set_onmessage(Some(closure.as_ref().unchecked_ref()));
                    let _ = source.connect_with_audio_node(&node);
                    let _ = node.connect_with_audio_node(&ctx.destination());
                    node_and_closure = Some((node.into(), closure.into_js_value()));
                }
            }
            if node_and_closure.is_none()
                && let Ok(node) = ctx
                    .create_script_processor_with_buffer_size_and_number_of_input_channels_and_number_of_output_channels(
                        2048, 1, 1,
                    )
            {
                let this2 = this.clone();
                let closure = Closure::<dyn FnMut(AudioProcessingEvent)>::new(move |e: AudioProcessingEvent| {
                    if let Ok(buf) = e.input_buffer()
                        && let Ok(data) = buf.get_channel_data(0)
                    {
                        feed(&data);
                        this2.publish_counters();
                    }
                });
                node.set_onaudioprocess(Some(closure.as_ref().unchecked_ref()));
                let _ = source.connect_with_audio_node(&node);
                let _ = node.connect_with_audio_node(&ctx.destination());
                node_and_closure = Some((node.into(), closure.into_js_value()));
            }
            let Some((node, closure)) = node_and_closure else {
                stop_tracks(&stream);
                let _ = ctx.close();
                return;
            };
            set(&this.stats, "mic", "on");
            this.inner.borrow_mut().mic = Some(Mic {
                stream,
                ctx,
                _node: node,
                _closure: closure,
            });
        });
    }

    /// Stops microphone capture.
    pub fn stop_mic(&self) {
        self.mic_gen.set(self.mic_gen.get() + 1);
        if let Some(m) = self.inner.borrow_mut().mic.take() {
            stop_tracks(&m.stream);
            let _ = m.ctx.close();
            set(&self.stats, "mic", "off");
        }
    }
}

fn stop_tracks(stream: &MediaStream) {
    for t in stream.get_tracks().iter() {
        if let Ok(t) = t.dyn_into::<web_sys::MediaStreamTrack>() {
            t.stop();
        }
    }
}
