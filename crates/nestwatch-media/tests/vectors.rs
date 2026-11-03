// SPDX-License-Identifier: AGPL-3.0-or-later
//! Conformance vectors in `vectors/media/` (test-vectors.md §1).
//!
//! `codec2.json`: codec 2 frames for fixed input signals (media.md §2.1) and
//! the §2.2 concealment sequence. `gate.json`: sound activation decisions
//! (media.md §3). The decoder output and gate decisions are normative; the
//! encoded bytes come from the reference encoder.
//!
//! Regenerate with `NESTWATCH_WRITE_VECTORS=1 cargo test -p nestwatch-media --test vectors`.

use std::path::PathBuf;

use nestwatch_media::codec::AudioDecoder;
use nestwatch_media::codec::adpcm::{AdpcmDecoder, decode_frame, encode_frame};
use nestwatch_media::gate::{AudioRing, GateConfig, GateEvent, ListenGate, ListenMode};
use nestwatch_media::{CodecId, FRAME_SAMPLES, PcmFrame};
use serde_json::{Value, json};

fn vectors_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../vectors/media")
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

fn signal(n_frames: usize, f: impl Fn(usize) -> f64) -> Vec<PcmFrame> {
    (0..n_frames)
        .map(|k| {
            let mut fr = [0i16; FRAME_SAMPLES];
            for (i, s) in fr.iter_mut().enumerate() {
                *s = f(k * FRAME_SAMPLES + i).round().clamp(-32768.0, 32767.0) as i16;
            }
            fr
        })
        .collect()
}

fn signals() -> Vec<(&'static str, &'static str, Vec<PcmFrame>)> {
    use std::f64::consts::PI;
    let t = |i: usize| i as f64 / 16_000.0;
    let mut lcg = 12_345u32;
    let noise: Vec<f64> = (0..2 * FRAME_SAMPLES)
        .map(|_| {
            lcg = lcg.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            f64::from((lcg >> 16) as i32 % 16_001 - 8000)
        })
        .collect();
    vec![
        ("silence", "all zero", signal(1, |_| 0.0)),
        (
            "sine-440",
            "10000·sin(2π·440·t), 3 consecutive frames",
            signal(3, |i| 10_000.0 * (2.0 * PI * 440.0 * t(i)).sin()),
        ),
        (
            "sine-3000-loud",
            "20000·sin(2π·3000·t)",
            signal(1, |i| 20_000.0 * (2.0 * PI * 3000.0 * t(i)).sin()),
        ),
        (
            "noise",
            "uniform LCG noise in -8000..8000",
            signal(2, |i| noise[i]),
        ),
        (
            "square-full-scale",
            "250 Hz square at full scale",
            signal(1, |i| if (i / 32) % 2 == 0 { 32767.0 } else { -32768.0 }),
        ),
        (
            "step",
            "0 then 30000 from sample 160, starting at -100",
            signal(1, |i| match i {
                0 => -100.0,
                1..160 => 0.0,
                _ => 30_000.0,
            }),
        ),
        (
            "chirp",
            "linear chirp 100 Hz → 6000 Hz over 2 frames, amplitude 12000",
            signal(2, |i| {
                let tt = t(i);
                let k = (6000.0 - 100.0) / 0.04;
                12_000.0 * (2.0 * PI * (100.0 * tt + 0.5 * k * tt * tt)).sin()
            }),
        ),
    ]
}

fn codec2_vectors() -> Value {
    let cases: Vec<Value> = signals()
        .into_iter()
        .map(|(name, desc, frames)| {
            let frames: Vec<Value> = frames
                .iter()
                .map(|f| {
                    let enc = encode_frame(f);
                    let dec = decode_frame(&enc).unwrap();
                    json!({ "input": f.to_vec(), "encoded": hex(&enc), "decoded": dec.to_vec() })
                })
                .collect();
            json!({ "name": name, "description": desc, "frames": frames })
        })
        .collect();
    // Concealment: one decoded frame, then four lost frames.
    let src = &signals()[1].2[0];
    let enc = encode_frame(src);
    let mut dec = AdpcmDecoder::new();
    dec.decode(&enc).unwrap();
    let conceal: Vec<Value> = (0..4)
        .map(|_| dec.conceal(None).map_or(Value::Null, |f| json!(f.to_vec())))
        .collect();
    json!({
        "description": "Nestwatch codec 2 (IMA ADPCM 16 kHz) frames, media.md §2.1. Each frame: 320 input samples, the 164-byte encoding in hex (reference encoder: predictor = input[0], step_index minimising squared error over samples 1..31, ties to the lowest index), and the normative decoder output.",
        "cases": cases,
        "concealment": {
            "description": "media.md §2.2: after decoding `encoded`, each lost frame repeats the last decoded frame attenuated by 6 dB per repetition (gain 10^(-6n/20), rounded to nearest), at most 3 times; null = silence.",
            "encoded": hex(&enc),
            "outputs": conceal,
        }
    })
}

struct GateCase {
    name: &'static str,
    description: &'static str,
    config: GateConfig,
    /// Ring frames pushed before the session starts.
    before_session: usize,
    levels: Vec<f32>,
    /// (step index, new config) applied before that step.
    changes: Vec<(usize, GateConfig)>,
}

fn config_json(c: &GateConfig) -> Value {
    json!({
        "mode": match c.mode { ListenMode::Continuous => "continuous", ListenMode::Activated => "activated" },
        "threshold": c.threshold_dbfs,
        "hold_ms": c.hold_ms,
        "preroll_ms": c.preroll_ms,
    })
}

fn gate_cases() -> Vec<GateCase> {
    let rep = |l: f32, n: usize| vec![l; n];
    let cat = |parts: Vec<Vec<f32>>| parts.concat();
    vec![
        GateCase {
            name: "quiet-room",
            description: "room below threshold: nothing is sent",
            config: GateConfig::product_default(),
            before_session: 0,
            levels: rep(-62.0, 40),
            changes: vec![],
        },
        GateCase {
            name: "cry-with-preroll",
            description: "defaults (-50 dBFS, 4 s hold, 500 ms pre-roll): 25 pre-roll frames, trigger, activity open, then hold until expiry",
            config: GateConfig::product_default(),
            before_session: 10,
            levels: cat(vec![
                rep(-60.0, 40),
                vec![-30.0, -35.0, -45.0],
                rep(-55.0, 202),
            ]),
            changes: vec![],
        },
        GateCase {
            name: "hold-rearm-and-expiry",
            description: "100 ms hold re-armed by frames at the threshold, then closing",
            config: GateConfig::activated(-40.0, 100, 60),
            before_session: 0,
            levels: cat(vec![
                rep(-70.0, 5),
                vec![-40.0],
                rep(-60.0, 4),
                vec![-39.0],
                rep(-60.0, 7),
                vec![-20.0],
                rep(-60.0, 3),
            ]),
            changes: vec![],
        },
        GateCase {
            name: "continuous",
            description: "continuous mode sends every frame; first frame starts a burst",
            config: GateConfig::default(),
            before_session: 5,
            levels: cat(vec![rep(-80.0, 3), rep(-10.0, 2)]),
            changes: vec![],
        },
        GateCase {
            name: "threshold-change",
            description: "threshold lowered then raised by new listen messages",
            config: GateConfig::activated(-30.0, 0, 40),
            before_session: 0,
            levels: rep(-45.0, 8),
            changes: vec![
                (3, GateConfig::activated(-50.0, 0, 40)),
                (6, GateConfig::activated(-40.0, 0, 40)),
            ],
        },
    ]
}

fn gate_vectors() -> Value {
    let cases: Vec<Value> = gate_cases()
        .iter()
        .map(|c| {
            let mut ring = AudioRing::new();
            for _ in 0..c.before_session {
                ring.push(CodecId::ADPCM, 0, -20.0, vec![]);
            }
            let mut gate = ListenGate::new(c.config, &ring);
            let mut steps = Vec::new();
            for (i, &level) in c.levels.iter().enumerate() {
                let mut step = serde_json::Map::new();
                if let Some((_, cfg)) = c.changes.iter().find(|(at, _)| *at == i) {
                    gate.set_config(*cfg);
                    step.insert("listen".into(), config_json(cfg));
                }
                let idx = ring.push(CodecId::ADPCM, 0, level, vec![]);
                let out: Vec<Value> = gate
                    .process(&ring, idx)
                    .into_iter()
                    .map(|e| match e {
                        GateEvent::Frame(f) => json!({ "frame": f.seq, "flags": f.flags.bits() }),
                        GateEvent::Activity { open } => json!({ "activity": open }),
                    })
                    .collect();
                step.insert("level".into(), json!(level));
                step.insert("out".into(), Value::Array(out));
                steps.push(Value::Object(step));
            }
            json!({
                "name": c.name,
                "description": c.description,
                "listen": config_json(&c.config),
                "frames_before_session": c.before_session,
                "steps": steps,
            })
        })
        .collect();
    json!({
        "description": "Nestwatch sound activation gate decisions, media.md §3. One step per captured 20 ms frame (its level in dBFS); `out` lists what is queued for the listener in order: frames by seq (captured frames since session start) with flags (1 = pre-roll, 2 = first of burst), and activity messages. An optional `listen` applies before the step's frame.",
        "cases": cases,
    })
}

/// Indented JSON where small values and number arrays stay on one line.
fn to_text(v: &Value) -> String {
    fn write(v: &Value, indent: usize, out: &mut String) {
        let compact = serde_json::to_string(v).unwrap();
        let numbers = v.as_array().is_some_and(|a| a.iter().all(Value::is_number));
        if compact.len() <= 100 || numbers {
            out.push_str(&compact);
            return;
        }
        let pad = "  ".repeat(indent + 1);
        match v {
            Value::Array(items) => {
                out.push_str("[\n");
                for (i, item) in items.iter().enumerate() {
                    out.push_str(&pad);
                    write(item, indent + 1, out);
                    out.push_str(if i + 1 < items.len() { ",\n" } else { "\n" });
                }
                out.push_str(&"  ".repeat(indent));
                out.push(']');
            }
            Value::Object(map) => {
                out.push_str("{\n");
                for (i, (k, item)) in map.iter().enumerate() {
                    out.push_str(&pad);
                    out.push_str(&serde_json::to_string(k).unwrap());
                    out.push_str(": ");
                    write(item, indent + 1, out);
                    out.push_str(if i + 1 < map.len() { ",\n" } else { "\n" });
                }
                out.push_str(&"  ".repeat(indent));
                out.push('}');
            }
            _ => out.push_str(&compact),
        }
    }
    let mut out = String::new();
    write(v, 0, &mut out);
    out.push('\n');
    out
}

fn check(name: &str, generated: Value) -> Value {
    let path = vectors_dir().join(name);
    if std::env::var_os("NESTWATCH_WRITE_VECTORS").is_some() {
        std::fs::create_dir_all(vectors_dir()).unwrap();
        std::fs::write(&path, to_text(&generated)).unwrap();
    }
    let stored: Value = serde_json::from_str(
        &std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display())),
    )
    .unwrap();
    assert_eq!(stored, generated, "{name} differs from the implementation");
    stored
}

#[test]
fn media_2_1_codec2_vectors() {
    let stored = check("codec2.json", codec2_vectors());
    // Independently of regeneration: decoding each stored encoding gives the
    // stored samples, and every frame decodes on its own.
    for case in stored["cases"].as_array().unwrap() {
        for f in case["frames"].as_array().unwrap() {
            let enc = unhex(f["encoded"].as_str().unwrap());
            assert_eq!(enc.len(), 164);
            let dec: Vec<i64> = decode_frame(&enc)
                .unwrap()
                .iter()
                .map(|&s| i64::from(s))
                .collect();
            let want: Vec<i64> = f["decoded"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_i64().unwrap())
                .collect();
            assert_eq!(dec, want, "{}", case["name"]);
            assert_eq!(f["input"][0], f["decoded"][0], "predictor stored exactly");
        }
    }
    let c = &stored["concealment"]["outputs"];
    assert!(c[0].is_array() && c[2].is_array() && c[3].is_null());
}

#[test]
fn media_3_gate_vectors() {
    let stored = check("gate.json", gate_vectors());
    let cry = &stored["cases"][1]["steps"][40]["out"];
    assert_eq!(cry.as_array().unwrap().len(), 27);
    assert_eq!(cry[0]["flags"], 1);
    assert_eq!(cry[25]["flags"], 2);
    assert_eq!(cry[26]["activity"], true);
}
