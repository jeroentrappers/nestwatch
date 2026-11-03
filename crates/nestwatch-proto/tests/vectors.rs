// SPDX-License-Identifier: AGPL-3.0-or-later
//! Conformance vectors in `vectors/proto/*.json` (test-vectors.md §1).
//!
//! `generate_*` builds each file from fixed inputs; `vectors_are_up_to_date`
//! fails if a file differs (set `NESTWATCH_REGEN_VECTORS=1` to rewrite them). The
//! `check_*` tests interpret the JSON on its own and verify the crate
//! against it, the way an independent implementation would.

use std::net::IpAddr;
use std::path::PathBuf;

use minicbor::Decoder;
use minicbor::data::Type;
use nestwatch_proto::control::{
    CameraFacing, CloseReason, Hello, Listen, ListenMode, Role, StationStatus, TalkStatus,
    VideoStart, VideoState, VideoStateKind, caps,
};
use nestwatch_proto::endpoint::{InterfaceAddr, order_endpoints};
use nestwatch_proto::frame::{FrameDecoder, encode_frame};
use nestwatch_proto::preface::prologue;
use nestwatch_proto::record::{AudioFlags, AudioStream};
use nestwatch_proto::{
    AudioHeader, Control, DeviceId, Endpoint, Host, Invitation, ListenerPreface, Mode, Record,
    StationPreface, StationTxt, Status, VideoHeader,
};
use serde_json::{Map, Value, json};

// ---------------------------------------------------------------- helpers

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(s: &str) -> Vec<u8> {
    assert!(s.len() % 2 == 0, "odd hex length");
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

fn pattern(len: usize, seed: u8) -> Vec<u8> {
    (0..len)
        .map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed))
        .collect()
}

fn vectors_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../vectors/proto")
}

fn load(name: &str) -> Value {
    let path = vectors_dir().join(name);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path:?}: {e}"));
    serde_json::from_str(&text).unwrap()
}

fn render(v: &Value) -> String {
    let mut s = serde_json::to_string_pretty(v).unwrap();
    s.push('\n');
    s
}

/// Decodes CBOR into JSON (text-keyed maps, arrays, text, ints, floats,
/// bools, null) for documenting control messages.
fn cbor_to_json(d: &mut Decoder<'_>) -> Value {
    match d.datatype().unwrap() {
        Type::Map => {
            let n = d.map().unwrap().unwrap();
            let mut m = Map::new();
            for _ in 0..n {
                let k = d.str().unwrap().to_owned();
                m.insert(k, cbor_to_json(d));
            }
            Value::Object(m)
        }
        Type::Array => {
            let n = d.array().unwrap().unwrap();
            Value::Array((0..n).map(|_| cbor_to_json(d)).collect())
        }
        Type::String => json!(d.str().unwrap()),
        Type::U8 | Type::U16 | Type::U32 | Type::U64 => json!(d.u64().unwrap()),
        Type::I8 | Type::I16 | Type::I32 | Type::I64 => json!(d.i64().unwrap()),
        Type::Bool => json!(d.bool().unwrap()),
        Type::Null => {
            d.null().unwrap();
            Value::Null
        }
        Type::F16 => {
            let p = d.position();
            let b = &d.input()[p + 1..p + 3];
            let x = f16_to_f64(u16::from_be_bytes([b[0], b[1]]));
            d.set_position(p + 3);
            json!(x)
        }
        Type::F32 | Type::F64 => json!(d.f64().unwrap()),
        other => panic!("unexpected CBOR type {other:?}"),
    }
}

fn f16_to_f64(bits: u16) -> f64 {
    let sign = if bits & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exp = i32::from((bits >> 10) & 0x1f);
    let mant = f64::from(bits & 0x03ff);
    sign * match exp {
        0 => mant * 2f64.powi(-24),
        e => (1024.0 + mant) * 2f64.powi(e - 25),
    }
}

fn f64_to_f16(x: f64) -> Option<u16> {
    (0u16..0x7c00)
        .chain(0x8000..0xfc00)
        .find(|&h| f16_to_f64(h) == x && (h & 0x8000 != 0) == x.is_sign_negative())
}

fn cbor_head(major: u8, n: u64, out: &mut Vec<u8>) {
    let m = major << 5;
    match n {
        0..=23 => out.push(m | n as u8),
        24..=0xff => out.extend_from_slice(&[m | 24, n as u8]),
        0x100..=0xffff => {
            out.push(m | 25);
            out.extend_from_slice(&(n as u16).to_be_bytes());
        }
        0x1_0000..=0xffff_ffff => {
            out.push(m | 26);
            out.extend_from_slice(&(n as u32).to_be_bytes());
        }
        _ => {
            out.push(m | 27);
            out.extend_from_slice(&n.to_be_bytes());
        }
    }
}

/// Encodes JSON as CBOR in core deterministic encoding (RFC 8949 §4.2.1),
/// written independently of the crate's encoder.
fn json_to_cbor(v: &Value, out: &mut Vec<u8>) {
    match v {
        Value::Null => out.push(0xf6),
        Value::Bool(b) => out.push(if *b { 0xf5 } else { 0xf4 }),
        Value::Number(n) => {
            if let Some(u) = n.as_u64() {
                cbor_head(0, u, out);
            } else if let Some(i) = n.as_i64() {
                cbor_head(1, (-1 - i) as u64, out);
            } else {
                let x = n.as_f64().unwrap();
                if let Some(h) = f64_to_f16(x) {
                    out.push(0xf9);
                    out.extend_from_slice(&h.to_be_bytes());
                } else if f64::from(x as f32) == x {
                    out.push(0xfa);
                    out.extend_from_slice(&(x as f32).to_be_bytes());
                } else {
                    out.push(0xfb);
                    out.extend_from_slice(&x.to_be_bytes());
                }
            }
        }
        Value::String(s) => {
            cbor_head(3, s.len() as u64, out);
            out.extend_from_slice(s.as_bytes());
        }
        Value::Array(a) => {
            cbor_head(4, a.len() as u64, out);
            for x in a {
                json_to_cbor(x, out);
            }
        }
        Value::Object(m) => {
            let mut entries: Vec<(Vec<u8>, Vec<u8>)> = m
                .iter()
                .map(|(k, v)| {
                    let mut kb = Vec::new();
                    json_to_cbor(&json!(k), &mut kb);
                    let mut vb = Vec::new();
                    json_to_cbor(v, &mut vb);
                    (kb, vb)
                })
                .collect();
            entries.sort();
            cbor_head(5, entries.len() as u64, out);
            for (k, v) in entries {
                out.extend_from_slice(&k);
                out.extend_from_slice(&v);
            }
        }
    }
}

fn control_json(c: &Control) -> Value {
    let bytes = c.encode().unwrap();
    cbor_to_json(&mut Decoder::new(&bytes))
}

// ---------------------------------------------------------------- inputs

fn station_id() -> DeviceId {
    DeviceId::parse("mfrggzdfmztwq2lk").unwrap()
}

fn control_cases() -> Vec<(&'static str, Control)> {
    vec![
        (
            "hello-station",
            Control::Hello(Hello {
                name: "Nursery".into(),
                role: Role::Station,
                app: "nestwatch/0.1.0".into(),
                caps: [
                    caps::AUDIO,
                    caps::VOX,
                    caps::TALK,
                    caps::VIDEO,
                    caps::BATTERY,
                ]
                .map(String::from)
                .to_vec(),
                audio: vec![1, 2],
                video: vec![1],
            }),
        ),
        (
            "hello-listener",
            Control::Hello(Hello {
                name: "Kitchen tablet".into(),
                role: Role::Listener,
                app: "nestwatch-web/0.1.0".into(),
                caps: vec![caps::AUDIO.into(), caps::VIDEO.into()],
                audio: vec![2],
                video: vec![],
            }),
        ),
        ("audio-config", Control::AudioConfig { codec: 2 }),
        ("listen-defaults", Control::Listen(Listen::default())),
        (
            "listen-continuous",
            Control::Listen(Listen {
                mode: ListenMode::Continuous,
                threshold: -90.0,
                hold_ms: 60_000,
                preroll_ms: 0,
            }),
        ),
        (
            "listen-f32-threshold",
            Control::Listen(Listen {
                mode: ListenMode::Activated,
                threshold: -42.3,
                hold_ms: 2_500,
                preroll_ms: 2_000,
            }),
        ),
        (
            "level",
            Control::Level {
                dbfs: -63.5,
                open: false,
            },
        ),
        ("activity-open", Control::Activity { open: true }),
        (
            "status-full",
            Control::Status(StationStatus {
                name: "Nursery".into(),
                battery: Some(14),
                charging: Some(false),
                listeners: 3,
                camera: true,
                talker: Some("Dad".into()),
            }),
        ),
        (
            "status-nulls",
            Control::Status(StationStatus {
                name: "Nursery".into(),
                battery: None,
                charging: None,
                listeners: 1,
                camera: false,
                talker: None,
            }),
        ),
        (
            "video-start",
            Control::VideoStart(VideoStart {
                max_w: 640,
                max_h: 480,
                max_fps: 10,
                codecs: vec![1],
            }),
        ),
        ("video-stop", Control::VideoStop),
        (
            "video-options",
            Control::VideoOptions {
                camera: CameraFacing::Front,
                night: true,
            },
        ),
        (
            "video-state-streaming",
            Control::VideoState(VideoState {
                state: VideoStateKind::Streaming,
                codec: Some(1),
                w: Some(640),
                h: Some(480),
                fps: Some(10),
            }),
        ),
        (
            "video-state-disabled",
            Control::VideoState(VideoState {
                state: VideoStateKind::Disabled,
                codec: None,
                w: None,
                h: None,
                fps: None,
            }),
        ),
        ("talk-request", Control::TalkRequest),
        ("talk-release", Control::TalkRelease),
        (
            "talk-state-granted",
            Control::TalkState {
                state: TalkStatus::Granted,
                by: None,
            },
        ),
        (
            "talk-state-busy",
            Control::TalkState {
                state: TalkStatus::Busy,
                by: Some("Mum".into()),
            },
        ),
        (
            "paired",
            Control::Paired {
                station_id: station_id(),
            },
        ),
        ("unpair", Control::Unpair),
        (
            "close-revoked",
            Control::Close {
                reason: CloseReason::Revoked,
                detail: None,
            },
        ),
        (
            "close-protocol-error",
            Control::Close {
                reason: CloseReason::ProtocolError,
                detail: Some("unexpected record before hello".into()),
            },
        ),
    ]
}

// ---------------------------------------------------------------- generators

fn generate_preface() -> Value {
    let listener: Vec<Value> = [
        ("trusted", Mode::Trusted),
        ("invite", Mode::Invite),
        ("code", Mode::Code),
    ]
    .iter()
    .map(|(n, m)| json!({"mode": n, "hex": hex(&ListenerPreface::new(*m).encode())}))
    .collect();
    let station: Vec<Value> = [
        ("ok", Status::Ok),
        ("unsupported-version", Status::UnsupportedVersion),
        ("pairing-closed", Status::PairingClosed),
        ("busy", Status::Busy),
        ("rate-limited", Status::RateLimited),
    ]
    .iter()
    .map(|(n, s)| json!({"status": n, "hex": hex(&StationPreface::new(*s).encode())}))
    .collect();
    let l = ListenerPreface::new(Mode::Invite).encode();
    let s = StationPreface::new(Status::Ok).encode();
    json!({
        "description": "NWP/1 prefaces and Noise prologue (protocol.md §4)",
        "listener": listener,
        "station": station,
        "prologue": [{
            "listener_hex": hex(&l),
            "station_hex": hex(&s),
            "hex": hex(&prologue(&l, &s)),
        }],
        "invalid": [
            {"name": "truncated", "hex": "4c554c4c01"},
            {"name": "bad-magic", "hex": "4c554c5801 01".replace(' ', "")},
            {"name": "version-2", "hex": "4c554c4c0201"},
            {"name": "unknown-mode", "hex": "4c554c4c0109"},
        ],
    })
}

fn generate_frames() -> Value {
    let cases: Vec<Value> = [vec![0x42], pattern(48, 1), pattern(300, 2)]
        .iter()
        .map(|m| json!({"message": hex(m), "hex": hex(&encode_frame(m).unwrap())}))
        .collect();
    json!({
        "description": "TCP framing: length (u16 BE) || noise_message (protocol.md §3.1)",
        "vectors": cases,
        "invalid": [{"name": "zero-length", "hex": "0000"}],
    })
}

fn audio_json(h: &AudioHeader, payload: &[u8]) -> Value {
    json!({
        "type": "audio",
        "stream": h.stream as u8,
        "codec": h.codec,
        "flags": h.flags.0,
        "seq": h.seq,
        "timestamp": h.timestamp,
        "payload": hex(payload),
    })
}

fn video_json(h: &VideoHeader, payload: &[u8]) -> Value {
    json!({
        "type": "video",
        "codec": h.codec,
        "frame_id": h.frame_id,
        "capture_ms": h.capture_ms,
        "width": h.width,
        "height": h.height,
        "frag_index": h.frag_index,
        "frag_count": h.frag_count,
        "payload": hex(payload),
    })
}

fn record_json(r: &Record) -> Value {
    match r {
        Record::Control(c) => json!({"type": "control", "message": control_json(c)}),
        Record::Audio { header, payload } => audio_json(header, payload),
        Record::Video { header, payload } => video_json(header, payload),
        Record::Ping(t) => json!({"type": "ping", "clock_ms": t}),
        Record::Pong(t) => json!({"type": "pong", "clock_ms": t}),
        Record::Unknown { record_type, body } => {
            json!({"type": "unknown", "record_type": record_type, "body": hex(body)})
        }
    }
}

fn generate_records() -> Value {
    let adpcm = AudioHeader {
        stream: AudioStream::Station,
        codec: 2,
        flags: AudioFlags::new(false, true),
        seq: 0,
        timestamp: 0,
    };
    let preroll = AudioHeader {
        flags: AudioFlags::new(true, false),
        seq: 41,
        timestamp: 41 * 960,
        ..adpcm
    };
    let talk = AudioHeader {
        stream: AudioStream::TalkBack,
        codec: 1,
        flags: AudioFlags::default(),
        seq: u32::MAX,
        timestamp: 0xffff_fc40,
    };
    let video = VideoHeader {
        codec: 1,
        frame_id: 1234,
        capture_ms: 0x0102_0304,
        width: 640,
        height: 480,
        frag_index: 2,
        frag_count: 3,
    };
    let records = vec![
        (
            "control-hello",
            Record::Control(control_cases()[0].1.clone()),
        ),
        ("control-video-stop", Record::Control(Control::VideoStop)),
        (
            "audio-adpcm-burst-start",
            Record::Audio {
                header: adpcm,
                payload: pattern(164, 7),
            },
        ),
        (
            "audio-preroll",
            Record::Audio {
                header: preroll,
                payload: pattern(16, 9),
            },
        ),
        (
            "audio-talkback-wrapping",
            Record::Audio {
                header: talk,
                payload: pattern(60, 3),
            },
        ),
        (
            "video-last-fragment",
            Record::Video {
                header: video,
                payload: pattern(40, 5),
            },
        ),
        ("ping", Record::Ping(1_700_000_000_123)),
        ("pong", Record::Pong(1_700_000_000_123)),
        (
            "unknown-type-0x10",
            Record::Unknown {
                record_type: 0x10,
                body: vec![1, 2, 3],
            },
        ),
    ];
    let vectors: Vec<Value> = records
        .iter()
        .map(
            |(n, r)| json!({"name": n, "record": record_json(r), "hex": hex(&r.encode().unwrap())}),
        )
        .collect();
    let video_bytes = records[5].1.encode().unwrap();
    let mut zero_count = video_bytes.clone();
    zero_count[14..18].fill(0);
    let mut bad_index = video_bytes.clone();
    bad_index[14..16].copy_from_slice(&3u16.to_be_bytes());
    json!({
        "description": "NWP/1 records: type (u8) || body (protocol.md §6). Control messages use RFC 8949 core deterministic encoding.",
        "vectors": vectors,
        "invalid": [
            {"name": "empty", "hex": ""},
            {"name": "audio-truncated-header", "hex": "0200020000000000000000"},
            {"name": "audio-bad-stream", "hex": "020202000000000000000000aa"},
            {"name": "video-truncated-header", "hex": hex(&video_bytes[..17])},
            {"name": "video-zero-frag-count", "hex": hex(&zero_count)},
            {"name": "video-index-not-below-count", "hex": hex(&bad_index)},
            {"name": "ping-short", "hex": "0401020304050607"},
            {"name": "pong-long", "hex": "05010203040506070809"},
            {"name": "control-not-a-map", "hex": "0180"},
            {"name": "control-trailing-bytes", "hex": "01a16174 66756e7061697200".replace(' ', "")},
        ],
    })
}

fn generate_control() -> Value {
    let vectors: Vec<Value> = control_cases()
        .iter()
        .map(|(n, c)| json!({"name": n, "message": control_json(c), "hex": hex(&c.encode().unwrap())}))
        .collect();
    let mut decode_only = Vec::new();
    let mut push = |name: &str, input: Value, expect: Value| {
        let mut bytes = Vec::new();
        json_to_cbor(&input, &mut bytes);
        decode_only
            .push(json!({"name": name, "hex": hex(&bytes), "input": input, "expect": expect}));
    };
    push(
        "unknown-keys-ignored",
        json!({"t": "activity", "open": false, "future": [1, {"x": null}], "zz": "y"}),
        json!({"t": "activity", "open": false}),
    );
    push(
        "unknown-type",
        json!({"t": "night-light", "level": 3}),
        json!({"unknown": "night-light"}),
    );
    push(
        "threshold-as-f64",
        json!({"t": "listen", "mode": "activated", "threshold": -12.345678901234, "hold_ms": 4000, "preroll_ms": 500}),
        json!({"t": "listen", "mode": "activated", "threshold": f64::from(-12.345_678_901_234f64 as f32), "hold_ms": 4000, "preroll_ms": 500}),
    );
    push(
        "optional-key-null",
        json!({"t": "talk-state", "state": "idle", "by": null}),
        json!({"t": "talk-state", "state": "idle"}),
    );
    push(
        "close-unknown-reason",
        json!({"t": "close", "reason": "overheated"}),
        json!({"t": "close", "reason": "overheated"}),
    );
    let mut invalid = Vec::new();
    let mut bad = |name: &str, input: Value| {
        let mut bytes = Vec::new();
        json_to_cbor(&input, &mut bytes);
        invalid.push(json!({"name": name, "hex": hex(&bytes), "input": input}));
    };
    bad("missing-t", json!({"open": true}));
    bad("t-not-text", json!({"t": 1}));
    bad("missing-required-key", json!({"t": "activity"}));
    bad("wrong-type-bool", json!({"t": "activity", "open": 1}));
    bad(
        "wrong-type-uint",
        json!({"t": "audio-config", "codec": "2"}),
    );
    bad("negative-uint", json!({"t": "audio-config", "codec": -2}));
    bad("codec-over-255", json!({"t": "audio-config", "codec": 256}));
    bad(
        "threshold-int",
        json!({"t": "listen", "mode": "activated", "threshold": -50, "hold_ms": 4000, "preroll_ms": 500}),
    );
    bad(
        "threshold-above-0",
        json!({"t": "listen", "mode": "activated", "threshold": 0.5, "hold_ms": 4000, "preroll_ms": 500}),
    );
    bad(
        "threshold-below-90",
        json!({"t": "listen", "mode": "activated", "threshold": -90.5, "hold_ms": 4000, "preroll_ms": 500}),
    );
    bad(
        "hold-too-long",
        json!({"t": "listen", "mode": "activated", "threshold": -50.0, "hold_ms": 60001, "preroll_ms": 500}),
    );
    bad(
        "preroll-too-long",
        json!({"t": "listen", "mode": "activated", "threshold": -50.0, "hold_ms": 4000, "preroll_ms": 2001}),
    );
    bad(
        "bad-mode",
        json!({"t": "listen", "mode": "loud", "threshold": -50.0, "hold_ms": 4000, "preroll_ms": 500}),
    );
    bad(
        "name-65-bytes",
        json!({"t": "hello", "name": "x".repeat(65), "role": "station", "app": "a", "caps": [], "audio": [2], "video": []}),
    );
    bad(
        "bad-role",
        json!({"t": "hello", "name": "x", "role": "server", "app": "a", "caps": [], "audio": [2], "video": []}),
    );
    bad(
        "battery-over-100",
        json!({"t": "status", "name": "x", "battery": 101, "charging": null, "listeners": 1, "camera": false, "talker": null}),
    );
    bad(
        "status-missing-nullable",
        json!({"t": "status", "name": "x", "battery": null, "charging": null, "listeners": 1, "camera": false}),
    );
    bad(
        "bad-station-id",
        json!({"t": "paired", "station_id": "ABC"}),
    );
    bad("not-a-map", json!(["t", "hello"]));
    invalid.push(json!({"name": "duplicate-key", "hex": "a261746675 6e70616972617466756e70616972".replace(' ', "")}));
    invalid.push(json!({"name": "truncated", "hex": "a2617466"}));
    json!({
        "description": "NWP/1 control messages (protocol.md §7) in RFC 8949 core deterministic encoding: definite lengths, shortest integers and floats, keys sorted by encoded bytes. 'message' is the CBOR map in JSON form; JSON numbers with a fraction or exponent are CBOR floats.",
        "vectors": vectors,
        "decode_only": decode_only,
        "invalid": invalid,
    })
}

fn invitation_json(inv: &Invitation) -> Value {
    json!({
        "station_key": hex(&inv.station_key),
        "token": hex(&inv.token),
        "name": inv.name,
        "endpoints": inv.endpoints.iter().map(ToString::to_string).collect::<Vec<_>>(),
    })
}

fn sample_invitation() -> Invitation {
    Invitation {
        station_key: pattern(32, 0x11).try_into().unwrap(),
        token: pattern(16, 0xA5).try_into().unwrap(),
        name: "Baby's room".into(),
        endpoints: [
            "nursery.tail1234.ts.net:47100",
            "100.101.102.103:47100",
            "[fd7a:115c:a1e0::7]:47100",
            "192.168.1.20:47100",
            "[2001:db8::20]:47100",
        ]
        .iter()
        .map(|e| Endpoint::parse(e).unwrap())
        .collect(),
    }
}

fn generate_invitations() -> Value {
    let full = sample_invitation();
    let minimal = Invitation {
        name: String::new(),
        endpoints: vec![],
        ..full.clone()
    };
    let unicode = Invitation {
        name: "Chambre de Zoé 🍼".into(),
        endpoints: vec![Endpoint::parse("10.0.0.5:47101").unwrap()],
        ..full.clone()
    };
    let origin = "http://192.168.1.20:47180";
    let vectors: Vec<Value> = [
        ("full", &full),
        ("minimal", &minimal),
        ("unicode-name", &unicode),
    ]
    .iter()
    .map(|(n, inv)| {
        json!({
            "name": n,
            "invitation": invitation_json(inv),
            "uri": inv.to_uri().unwrap(),
            "web_origin": origin,
            "web_url": inv.to_web_url(origin).unwrap(),
        })
    })
    .collect();
    let uri = full.to_uri().unwrap();
    let query = uri.strip_prefix("nestwatch://invite?").unwrap();
    let (mut reordered, endpoints): (Vec<&str>, Vec<&str>) =
        query.split('&').partition(|p| !p.starts_with("a="));
    reordered.reverse();
    reordered.splice(0..0, endpoints);
    let nine = format!(
        "{uri}&a=10.0.0.1%3A47100&a=10.0.0.2%3A47100&a=10.0.0.3%3A47100&a=10.0.0.4%3A47100"
    );
    let mut nine_expect = full.clone();
    nine_expect
        .endpoints
        .extend((1..=3).map(|i| Endpoint::parse(&format!("10.0.0.{i}:47100")).unwrap()));
    let parse_only = vec![
        json!({"name": "unknown-parameters", "uri": format!("{uri}&x=1&future=%E2%9C%93"), "invitation": invitation_json(&full)}),
        json!({"name": "reordered-and-uppercase-scheme", "uri": format!("NESTWATCH://INVITE?{}", reordered.join("&")), "invitation": invitation_json(&full)}),
        json!({"name": "no-name-parameter", "uri": minimal.to_uri().unwrap().replace("&n=", ""), "invitation": invitation_json(&minimal)}),
        json!({"name": "more-than-8-endpoints", "uri": nine, "invitation": invitation_json(&nine_expect)}),
    ];
    let invalid = vec![
        json!({"name": "wrong-scheme", "uri": uri.replace("nestwatch://", "http://")}),
        json!({"name": "version-2", "uri": uri.replace("v=1", "v=2")}),
        json!({"name": "missing-key", "uri": uri.replace(&format!("s={}&", query.split('&').nth(1).unwrap().trim_start_matches("s=")), "")}),
        json!({"name": "short-token", "uri": format!("nestwatch://invite?v=1&s={}&t=AAAA", query.split('&').nth(1).unwrap().trim_start_matches("s="))}),
        json!({"name": "padded-key", "uri": uri.replacen("&t=", "=&t=", 1)}),
        json!({"name": "duplicate-key", "uri": format!("{uri}&v=1")}),
        json!({"name": "name-too-long", "uri": minimal.to_uri().unwrap().replace("&n=", &format!("&n={}", "x".repeat(65)))}),
        json!({"name": "bad-endpoint", "uri": format!("{uri}&a=fd7a%3A%3A1%3A47100")}),
    ];
    json!({
        "description": "Invitation URIs (protocol.md §3.5). Keys and tokens are base64url without padding in the URI; hex here. Percent-encoding leaves only RFC 3986 unreserved characters unescaped.",
        "vectors": vectors,
        "parse_only": parse_only,
        "invalid": invalid,
    })
}

fn generate_endpoints() -> Value {
    let ip = |s: &str| s.parse::<IpAddr>().unwrap();
    let addrs = [
        ("wlan0", "2001:db8::20"),
        ("wlan0", "fe80::1c2b:3aff:fe4d:5e6f"),
        ("wlan0", "192.168.1.20"),
        ("lo", "127.0.0.1"),
        ("tailscale0", "fd7a:115c:a1e0::7"),
        ("tailscale0", "100.101.102.103"),
        ("wg0", "10.8.0.2"),
        ("eth0", "100.64.12.1"),
    ];
    let ifaces: Vec<InterfaceAddr> = addrs
        .iter()
        .map(|(i, a)| InterfaceAddr::new(*i, ip(a)))
        .collect();
    let names = [Host::parse("nursery.tail1234.ts.net").unwrap()];
    let ordered: Vec<String> = order_endpoints(&names, &ifaces, 47100)
        .iter()
        .map(ToString::to_string)
        .collect();
    json!({
        "description": "Endpoint text form and station endpoint order (protocol.md §3.5)",
        "parse": [
            {"input": "nursery.tail1234.ts.net:47100", "canonical": "nursery.tail1234.ts.net:47100"},
            {"input": "Nursery.LOCAL.:47100", "canonical": "nursery.local:47100"},
            {"input": "192.168.1.20:47100", "canonical": "192.168.1.20:47100"},
            {"input": "[FD7A:115C:A1E0:0::7]:47109", "canonical": "[fd7a:115c:a1e0::7]:47109"},
        ],
        "invalid": ["host", "host:0", "host:65536", "fd7a::1:47100", "[fe80::1%25eth0]:47100", "[::1]", "1.2.3:47100", "-x.local:1"],
        "order": [{
            "vpn_dns_names": ["nursery.tail1234.ts.net"],
            "port": 47100,
            "addresses": addrs.iter().map(|(i, a)| json!({"interface": i, "addr": a})).collect::<Vec<_>>(),
            "endpoints": ordered,
        }],
    })
}

fn generate_discovery() -> Value {
    let open = StationTxt {
        id: station_id(),
        pair: true,
        http: Some(47180),
        https: Some(47181),
    };
    let closed = StationTxt {
        pair: false,
        http: Some(47182),
        https: None,
        ..open.clone()
    };
    let txt_json = |t: &StationTxt| json!({"id": t.id.as_str(), "pair": t.pair, "http": t.http, "https": t.https});
    json!({
        "description": "DNS-SD TXT strings for _nestwatch._tcp.local. (protocol.md §3.4)",
        "vectors": [
            {"name": "pairing-open", "txt": txt_json(&open), "strings": open.to_strings()},
            {"name": "pairing-closed-no-https", "txt": txt_json(&closed), "strings": closed.to_strings()},
        ],
        "invalid": [
            {"name": "version-2", "strings": ["v=2", "id=mfrggzdfmztwq2lk", "pair=0"]},
            {"name": "missing-id", "strings": ["v=1", "pair=0"]},
            {"name": "bad-pair", "strings": ["v=1", "id=mfrggzdfmztwq2lk", "pair=yes"]},
        ],
    })
}

fn all_files() -> Vec<(&'static str, Value)> {
    vec![
        ("preface.json", generate_preface()),
        ("frames.json", generate_frames()),
        ("records.json", generate_records()),
        ("control.json", generate_control()),
        ("invitations.json", generate_invitations()),
        ("endpoints.json", generate_endpoints()),
        ("discovery.json", generate_discovery()),
    ]
}

#[test]
fn vectors_are_up_to_date() {
    let regen = std::env::var_os("NESTWATCH_REGEN_VECTORS").is_some();
    if regen {
        std::fs::create_dir_all(vectors_dir()).unwrap();
    }
    for (name, value) in all_files() {
        let path = vectors_dir().join(name);
        let text = render(&value);
        if regen {
            std::fs::write(&path, text).unwrap();
        } else {
            let on_disk = std::fs::read_to_string(&path).unwrap_or_default();
            assert!(
                on_disk == text,
                "{name} is stale; rerun with NESTWATCH_REGEN_VECTORS=1"
            );
        }
    }
}

// ---------------------------------------------------------------- checks

fn s(v: &Value) -> &str {
    v.as_str().unwrap()
}

fn n(v: &Value) -> u64 {
    v.as_u64().unwrap()
}

fn arr(v: &Value) -> &Vec<Value> {
    v.as_array().unwrap()
}

#[test]
fn check_preface() {
    let f = load("preface.json");
    for v in arr(&f["listener"]) {
        let mode = match s(&v["mode"]) {
            "trusted" => Mode::Trusted,
            "invite" => Mode::Invite,
            "code" => Mode::Code,
            m => panic!("{m}"),
        };
        let bytes = unhex(s(&v["hex"]));
        assert_eq!(ListenerPreface::new(mode).encode().to_vec(), bytes);
        assert_eq!(ListenerPreface::decode(&bytes).unwrap().mode, mode);
    }
    for (i, v) in arr(&f["station"]).iter().enumerate() {
        let status = Status::from_u8(i as u8).unwrap();
        let bytes = unhex(s(&v["hex"]));
        assert_eq!(StationPreface::decode(&bytes).unwrap().status, status);
        assert_eq!(StationPreface::new(status).encode().to_vec(), bytes);
    }
    for v in arr(&f["prologue"]) {
        let l: [u8; 6] = unhex(s(&v["listener_hex"])).try_into().unwrap();
        let st: [u8; 6] = unhex(s(&v["station_hex"])).try_into().unwrap();
        assert_eq!(prologue(&l, &st).to_vec(), unhex(s(&v["hex"])));
    }
    for v in arr(&f["invalid"]) {
        assert!(
            ListenerPreface::decode(&unhex(s(&v["hex"]))).is_err(),
            "{v}"
        );
    }
}

#[test]
fn check_frames() {
    let f = load("frames.json");
    let mut stream = Vec::new();
    let mut msgs = Vec::new();
    for v in arr(&f["vectors"]) {
        let m = unhex(s(&v["message"]));
        let frame = unhex(s(&v["hex"]));
        assert_eq!(encode_frame(&m).unwrap(), frame);
        stream.extend_from_slice(&frame);
        msgs.push(m);
    }
    let mut dec = FrameDecoder::new();
    let mut got = Vec::new();
    for b in &stream {
        dec.push(std::slice::from_ref(b));
        got.extend(dec.by_ref().map(Result::unwrap));
    }
    assert_eq!(got, msgs);
    for v in arr(&f["invalid"]) {
        let mut dec = FrameDecoder::new();
        dec.push(&unhex(s(&v["hex"])));
        assert!(dec.next_message().is_err());
    }
}

fn control_from_json(message: &Value) -> Control {
    let mut bytes = Vec::new();
    json_to_cbor(message, &mut bytes);
    Control::decode(&bytes).unwrap()
}

fn record_from_json(r: &Value) -> Record {
    match s(&r["type"]) {
        "control" => Record::Control(control_from_json(&r["message"])),
        "audio" => Record::Audio {
            header: AudioHeader {
                stream: match n(&r["stream"]) {
                    0 => AudioStream::Station,
                    _ => AudioStream::TalkBack,
                },
                codec: n(&r["codec"]) as u8,
                flags: AudioFlags(n(&r["flags"]) as u8),
                seq: n(&r["seq"]) as u32,
                timestamp: n(&r["timestamp"]) as u32,
            },
            payload: unhex(s(&r["payload"])),
        },
        "video" => Record::Video {
            header: VideoHeader {
                codec: n(&r["codec"]) as u8,
                frame_id: n(&r["frame_id"]) as u32,
                capture_ms: n(&r["capture_ms"]) as u32,
                width: n(&r["width"]) as u16,
                height: n(&r["height"]) as u16,
                frag_index: n(&r["frag_index"]) as u16,
                frag_count: n(&r["frag_count"]) as u16,
            },
            payload: unhex(s(&r["payload"])),
        },
        "ping" => Record::Ping(n(&r["clock_ms"])),
        "pong" => Record::Pong(n(&r["clock_ms"])),
        "unknown" => Record::Unknown {
            record_type: n(&r["record_type"]) as u8,
            body: unhex(s(&r["body"])),
        },
        t => panic!("unknown vector record type {t}"),
    }
}

#[test]
fn check_records() {
    let f = load("records.json");
    for v in arr(&f["vectors"]) {
        let want = unhex(s(&v["hex"]));
        let rec = record_from_json(&v["record"]);
        assert_eq!(rec.encode().unwrap(), want, "{}", v["name"]);
        assert_eq!(Record::decode(&want).unwrap(), rec, "{}", v["name"]);
    }
    for v in arr(&f["invalid"]) {
        assert!(
            Record::decode(&unhex(s(&v["hex"]))).is_err(),
            "{}",
            v["name"]
        );
    }
}

#[test]
fn check_control() {
    let f = load("control.json");
    let mut types = std::collections::BTreeSet::new();
    for v in arr(&f["vectors"]) {
        let want = unhex(s(&v["hex"]));
        let mut independent = Vec::new();
        json_to_cbor(&v["message"], &mut independent);
        assert_eq!(independent, want, "{}", v["name"]);
        let c = Control::decode(&want).unwrap();
        assert!(!matches!(c, Control::Unknown { .. }), "{}", v["name"]);
        assert_eq!(c.encode().unwrap(), want, "{}", v["name"]);
        types.insert(c.message_type().to_owned());
    }
    // Every message of protocol.md §7 has a vector.
    assert_eq!(types.len(), 16);
    for v in arr(&f["decode_only"]) {
        let c = Control::decode(&unhex(s(&v["hex"]))).unwrap();
        match v["expect"].get("unknown") {
            Some(t) => assert_eq!(c, Control::Unknown { t: s(t).into() }),
            None => assert_eq!(c, control_from_json(&v["expect"]), "{}", v["name"]),
        }
    }
    for v in arr(&f["invalid"]) {
        assert!(
            Control::decode(&unhex(s(&v["hex"]))).is_err(),
            "{}",
            v["name"]
        );
    }
}

fn invitation_from_json(v: &Value) -> Invitation {
    Invitation {
        station_key: unhex(s(&v["station_key"])).try_into().unwrap(),
        token: unhex(s(&v["token"])).try_into().unwrap(),
        name: s(&v["name"]).into(),
        endpoints: arr(&v["endpoints"])
            .iter()
            .map(|e| Endpoint::parse(s(e)).unwrap())
            .collect(),
    }
}

#[test]
fn check_invitations() {
    let f = load("invitations.json");
    for v in arr(&f["vectors"]) {
        let inv = invitation_from_json(&v["invitation"]);
        assert_eq!(inv.to_uri().unwrap(), s(&v["uri"]));
        assert_eq!(Invitation::parse(s(&v["uri"])).unwrap(), inv);
        assert_eq!(
            inv.to_web_url(s(&v["web_origin"])).unwrap(),
            s(&v["web_url"])
        );
        assert_eq!(Invitation::parse_web_url(s(&v["web_url"])).unwrap(), inv);
    }
    for v in arr(&f["parse_only"]) {
        let inv = invitation_from_json(&v["invitation"]);
        assert_eq!(
            Invitation::parse(s(&v["uri"])).unwrap(),
            inv,
            "{}",
            v["name"]
        );
    }
    for v in arr(&f["invalid"]) {
        assert!(Invitation::parse(s(&v["uri"])).is_err(), "{}", v["name"]);
    }
}

#[test]
fn check_endpoints() {
    let f = load("endpoints.json");
    for v in arr(&f["parse"]) {
        let e = Endpoint::parse(s(&v["input"])).unwrap();
        assert_eq!(e.to_string(), s(&v["canonical"]));
    }
    for v in arr(&f["invalid"]) {
        assert!(Endpoint::parse(s(v)).is_err(), "{v}");
    }
    for v in arr(&f["order"]) {
        let names: Vec<Host> = arr(&v["vpn_dns_names"])
            .iter()
            .map(|h| Host::parse(s(h)).unwrap())
            .collect();
        let addrs: Vec<InterfaceAddr> = arr(&v["addresses"])
            .iter()
            .map(|a| InterfaceAddr::new(s(&a["interface"]), s(&a["addr"]).parse().unwrap()))
            .collect();
        let got: Vec<String> = order_endpoints(&names, &addrs, n(&v["port"]) as u16)
            .iter()
            .map(ToString::to_string)
            .collect();
        let want: Vec<&str> = arr(&v["endpoints"]).iter().map(s).collect();
        assert_eq!(got, want);
    }
}

#[test]
fn check_discovery() {
    let f = load("discovery.json");
    for v in arr(&f["vectors"]) {
        let t = &v["txt"];
        let txt = StationTxt {
            id: DeviceId::parse(s(&t["id"])).unwrap(),
            pair: t["pair"].as_bool().unwrap(),
            http: t["http"].as_u64().map(|p| p as u16),
            https: t["https"].as_u64().map(|p| p as u16),
        };
        let strings: Vec<&str> = arr(&v["strings"]).iter().map(s).collect();
        assert_eq!(txt.to_strings(), strings);
        assert_eq!(
            StationTxt::from_strings(strings.iter().copied()).unwrap(),
            txt
        );
    }
    for v in arr(&f["invalid"]) {
        let strings: Vec<&str> = arr(&v["strings"]).iter().map(s).collect();
        assert!(StationTxt::from_strings(strings).is_err(), "{}", v["name"]);
    }
}
