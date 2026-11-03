// SPDX-License-Identifier: AGPL-3.0-or-later
//! Nestwatch conformance vectors in `vectors/crypto/` (test-vectors.md §1).
//!
//! The test regenerates every file and compares it byte for byte. Set
//! `NESTWATCH_UPDATE_VECTORS=1` to rewrite the files after an intended change.

use std::path::PathBuf;

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::*;

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Fixed test key material: `SHA-256("nestwatch/1 test vector " || label)`.
fn fixed(label: &str) -> [u8; 32] {
    Sha256::new()
        .chain_update(b"nestwatch/1 test vector ")
        .chain_update(label.as_bytes())
        .finalize()
        .into()
}

#[derive(Serialize)]
struct DeviceIdCase {
    public_key: String,
    device_id: String,
    grouped: String,
}

#[derive(Serialize)]
struct DeviceIds {
    spec: &'static str,
    cases: Vec<DeviceIdCase>,
}

fn device_ids() -> DeviceIds {
    let mut keys = vec![[0u8; 32], core::array::from_fn(|i| i as u8), [0xff; 32]];
    keys.extend(["listener static", "station static"].map(|l| {
        *DeviceKey::from_secret_bytes(&fixed(l))
            .public_key()
            .as_bytes()
    }));
    DeviceIds {
        spec: "protocol.md 2.2",
        cases: keys
            .into_iter()
            .map(|k| {
                let id = PublicKey::from_bytes(k).device_id();
                DeviceIdCase {
                    public_key: hex(&k),
                    device_id: id.to_string(),
                    grouped: id.grouped(),
                }
            })
            .collect(),
    }
}

#[derive(Serialize)]
struct InvitePskCase {
    invite_token: String,
    station_public_key: String,
    psk: String,
}

#[derive(Serialize)]
struct InvitePsk {
    spec: &'static str,
    hkdf: &'static str,
    info: &'static str,
    cases: Vec<InvitePskCase>,
}

fn invite_psk() -> InvitePsk {
    let station = *DeviceKey::from_secret_bytes(&fixed("station static")).public_key();
    let tokens = [
        [0u8; 16],
        [0xff; 16],
        fixed("invite token")[..16].try_into().unwrap(),
    ];
    InvitePsk {
        spec: "protocol.md 5.2",
        hkdf: "HKDF-SHA256(ikm = invite_token, salt = station_public_key, info, L = 32)",
        info: "nestwatch/1 invite psk",
        cases: tokens
            .iter()
            .map(|t| InvitePskCase {
                invite_token: hex(t),
                station_public_key: hex(station.as_bytes()),
                psk: hex(InviteToken::from_bytes(t).psk(&station).as_bytes()),
            })
            .collect(),
    }
}

#[derive(Serialize)]
struct NormalisationCase {
    input: &'static str,
    normalised: Option<String>,
}

#[derive(Serialize)]
struct CodePskCase {
    code: String,
    station_device_id: String,
    salt: String,
    psk: String,
}

#[derive(Serialize)]
struct CodePsk {
    spec: &'static str,
    argon2: &'static str,
    normalisation: Vec<NormalisationCase>,
    cases: Vec<CodePskCase>,
}

const NORMALISATION_INPUTS: [&str; 12] = [
    "7K3QD-9XW2M",
    "7k3qd9xw2m",
    " 7k3qd - 9xw2m ",
    "OoIiLl0011",
    "ABCDE-FGHJK",
    "mnpqr-stvwx",
    "yz012-34567",
    "7K3QD-9XW2",
    "7K3QD-9XW2MM",
    "7K3QD-9XW2U",
    "7K3QD_9XW2M",
    "7K3QD\t9XW2M",
];

fn code_psk() -> CodePsk {
    let station = DeviceKey::from_secret_bytes(&fixed("station static")).device_id();
    let other = PublicKey::from_bytes([0u8; 32]).device_id();
    let cases = [
        ("7K3QD9XW2M", station),
        ("0000000000", station),
        ("ZZZZZZZZZZ", other),
    ];
    CodePsk {
        spec: "protocol.md 5.3",
        argon2: "Argon2id v0x13, password = normalised code (ASCII), salt = SHA-256(\"nestwatch/1 code salt\" || station_device_id)[0..16], m = 19456 KiB, t = 2, p = 1, length = 32, no secret, no associated data",
        normalisation: NORMALISATION_INPUTS
            .iter()
            .map(|&input| NormalisationCase {
                input,
                normalised: PairingCode::parse(input)
                    .ok()
                    .map(|c| c.as_str().to_owned()),
            })
            .collect(),
        cases: cases
            .iter()
            .map(|(code, id)| {
                let c = PairingCode::parse(code).unwrap();
                CodePskCase {
                    code: c.as_str().to_owned(),
                    station_device_id: id.to_string(),
                    salt: hex(&PairingCode::salt(id)),
                    psk: hex(c.psk(id).as_bytes()),
                }
            })
            .collect(),
    }
}

#[derive(Serialize)]
struct Party {
    static_secret: String,
    static_public: String,
    ephemeral_secret: String,
    preface: String,
}

#[derive(Serialize)]
struct Message {
    sender: &'static str,
    payload: String,
    message: String,
}

#[derive(Serialize)]
struct Handshake {
    spec: &'static str,
    protocol_name: &'static str,
    mode: &'static str,
    listener: Party,
    station: Party,
    prologue: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    invite_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pairing_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    station_device_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    psk: Option<String>,
    handshake_messages: Vec<Message>,
    handshake_hash: String,
    verification_number: Option<u32>,
    verification_display: Option<String>,
    transport_messages: Vec<Message>,
}

const TRANSPORT_PLAINTEXTS: [(&str, &[u8]); 4] = [
    ("listener", b"\x04\x00\x00\x00\x00\x00\x00\x00\x2a"),
    ("station", b"\x05\x00\x00\x00\x00\x00\x00\x00\x2a"),
    ("listener", b"\x01\xa1\x61\x74\x65\x68\x65\x6c\x6c\x6f"),
    ("station", b""),
];

fn handshake(mode: Mode) -> Handshake {
    let (name, mode_byte) = match mode {
        Mode::Trusted => ("trusted", 1u8),
        Mode::Invite => ("invite", 2),
        Mode::Code => ("code", 3),
    };
    let listener_preface = [b'N', b'E', b'S', b'T', 1, mode_byte];
    let station_preface = *b"NEST\x01\x00";
    let prologue = Prologue::new(listener_preface, station_preface);
    let ls = DeviceKey::from_secret_bytes(&fixed("listener static"));
    let ss = DeviceKey::from_secret_bytes(&fixed("station static"));
    let le = fixed(&format!("{name} listener ephemeral"));
    let se = fixed(&format!("{name} station ephemeral"));
    let station_id = ss.device_id();

    let token = InviteToken::from_bytes(&fixed("invite token")[..16].try_into().unwrap());
    let code = PairingCode::parse("7K3QD-9XW2M").unwrap();
    let psk = match mode {
        Mode::Trusted => None,
        Mode::Invite => Some(token.psk(ss.public_key())),
        Mode::Code => Some(code.psk(&station_id)),
    };
    let (station_key, expected_id) = match mode {
        Mode::Code => (None, Some(station_id)),
        _ => (Some(ss.public_key()), None),
    };
    let mut initiator = Initiator::build(
        mode,
        &ls,
        station_key,
        psk.as_ref(),
        expected_id,
        &prologue,
        Some(&le),
    )
    .unwrap();
    let mut responder = Responder::build(mode, &ss, psk.as_ref(), &prologue, Some(&se)).unwrap();

    let mut messages = Vec::new();
    let mut record = |sender, m: &[u8]| {
        messages.push(Message {
            sender,
            payload: String::new(),
            message: hex(m),
        })
    };
    let m1 = initiator.start().unwrap();
    record("listener", &m1);
    let mut step = responder.read_message(&m1).unwrap();
    if let Step::Authorize(key) = &step {
        assert_eq!(key, ls.public_key());
        step = responder.accept().unwrap();
    }
    let (mut lt, mut st) = match step {
        Step::Complete(c) => {
            let m2 = c.message.unwrap();
            record("station", &m2);
            let Step::Complete(i) = initiator.read_message(&m2).unwrap() else {
                panic!("listener completes");
            };
            (i.transport, c.transport)
        }
        Step::Send(m2) => {
            record("station", &m2);
            let Step::Complete(i) = initiator.read_message(&m2).unwrap() else {
                panic!("listener completes");
            };
            let m3 = i.message.unwrap();
            record("listener", &m3);
            let Step::Complete(c) = responder.read_message(&m3).unwrap() else {
                panic!("station completes");
            };
            (i.transport, c.transport)
        }
        Step::Authorize(_) => unreachable!(),
    };
    assert_eq!(lt.handshake_hash(), st.handshake_hash());

    let transport_messages = TRANSPORT_PLAINTEXTS
        .iter()
        .map(|&(sender, plaintext)| {
            let (tx, rx) = if sender == "listener" {
                (&mut lt, &mut st)
            } else {
                (&mut st, &mut lt)
            };
            let c = tx.encrypt(plaintext).unwrap();
            assert_eq!(rx.decrypt(&c).unwrap(), plaintext);
            Message {
                sender,
                payload: hex(plaintext),
                message: hex(&c),
            }
        })
        .collect();

    let party = |key: &DeviceKey, e: &[u8; 32], preface: &[u8; 6]| Party {
        static_secret: hex(key.secret_bytes().as_slice()),
        static_public: hex(key.public_key().as_bytes()),
        ephemeral_secret: hex(e),
        preface: hex(preface),
    };
    let vn = lt.verification_number();
    Handshake {
        spec: "protocol.md 4, 5",
        protocol_name: mode.protocol_name(),
        mode: name,
        listener: party(&ls, &le, &listener_preface),
        station: party(&ss, &se, &station_preface),
        prologue: hex(prologue.as_bytes()),
        invite_token: (mode == Mode::Invite).then(|| hex(token.as_bytes())),
        pairing_code: (mode == Mode::Code).then(|| code.display().to_string()),
        station_device_id: (mode == Mode::Code).then(|| station_id.to_string()),
        psk: psk.map(|p| hex(p.as_bytes())),
        handshake_messages: messages,
        handshake_hash: hex(lt.handshake_hash()),
        verification_number: vn.map(VerificationNumber::value),
        verification_display: vn.map(|n| n.to_string()),
        transport_messages,
    }
}

fn render(value: &impl Serialize) -> String {
    let mut s = serde_json::to_string_pretty(value).unwrap();
    s.push('\n');
    s
}

fn all() -> Vec<(&'static str, String)> {
    vec![
        ("device_ids.json", render(&device_ids())),
        ("invite_psk.json", render(&invite_psk())),
        ("code_psk.json", render(&code_psk())),
        ("handshake_trusted.json", render(&handshake(Mode::Trusted))),
        ("handshake_invite.json", render(&handshake(Mode::Invite))),
        ("handshake_code.json", render(&handshake(Mode::Code))),
    ]
}

#[test]
fn vectors_match_files() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../vectors/crypto");
    let update = std::env::var_os("NESTWATCH_UPDATE_VECTORS").is_some_and(|v| v == "1");
    if update {
        std::fs::create_dir_all(&dir).unwrap();
    }
    for (name, content) in all() {
        let path = dir.join(name);
        if update {
            std::fs::write(&path, &content).unwrap();
        } else {
            let on_disk = std::fs::read_to_string(&path).unwrap_or_else(|e| {
                panic!(
                    "{}: {e}; run with NESTWATCH_UPDATE_VECTORS=1",
                    path.display()
                )
            });
            assert!(on_disk == content, "{name} differs from the implementation");
        }
    }
}

#[test]
fn vectors_are_deterministic() {
    assert_eq!(all(), all());
}
