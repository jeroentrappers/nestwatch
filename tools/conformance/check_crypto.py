#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Independent check of the Lull crypto vectors in vectors/crypto/.

Re-derives every output from the inputs recorded in the vectors, using
implementations that share no code with the Rust core (test-vectors.md §2):
hashlib, `cryptography` (HKDF, X25519), `argon2-cffi` and `noiseprotocol`.

Usage: check_crypto.py [VECTOR_DIR]    (default: vectors/crypto at the repo root)
Exit status 0 when every vector agrees.
"""

import base64
import hashlib
import json
import sys
import warnings
from pathlib import Path

from argon2.low_level import Type, hash_secret_raw
from cryptography.hazmat.primitives import hashes
from cryptography.hazmat.primitives.asymmetric.x25519 import X25519PrivateKey
from cryptography.hazmat.primitives.kdf.hkdf import HKDF
from noise.connection import Keypair, NoiseConnection

CROCKFORD = "0123456789ABCDEFGHJKMNPQRSTVWXYZ"
MODE_BYTE = {"trusted": 1, "invite": 2, "code": 3}

# noiseprotocol warns when a fixed ephemeral key is set; that is the point here.
warnings.filterwarnings("ignore", message="One of ephemeral keypairs is already set")

failures = 0


def check(label, computed, vector):
    """Compares an independently computed value with the vector's value."""
    global failures
    if computed != vector:
        failures += 1
        print(f"FAIL {label}: vector has {vector!r}, independent result is {computed!r}")


def unhex(s):
    return bytes.fromhex(s)


def device_id(public_key):
    digest = hashlib.sha256(b"lull/1 device id" + public_key).digest()[:10]
    return base64.b32encode(digest).decode("ascii").lower()


def grouped(device):
    return "-".join(device[i : i + 4] for i in range(0, 16, 4))


def x25519_public(secret):
    return X25519PrivateKey.from_private_bytes(secret).public_key().public_bytes_raw()


def invite_psk(token, station_public_key):
    return HKDF(
        algorithm=hashes.SHA256(), length=32, salt=station_public_key, info=b"lull/1 invite psk"
    ).derive(token)


def normalise_code(text):
    out = []
    for c in text:
        if c in " -":
            continue
        c = c.upper() if c.isascii() else c
        c = {"O": "0", "I": "1", "L": "1"}.get(c, c)
        if c not in CROCKFORD:
            return None
        out.append(c)
    return "".join(out) if len(out) == 10 else None


def code_salt(station_device_id):
    return hashlib.sha256(b"lull/1 code salt" + station_device_id.encode("ascii")).digest()[:16]


def code_psk(code, station_device_id):
    return hash_secret_raw(
        secret=code.encode("ascii"),
        salt=code_salt(station_device_id),
        time_cost=2,
        memory_cost=19456,
        parallelism=1,
        hash_len=32,
        type=Type.ID,
        version=19,
    )


def verification(h):
    n = int.from_bytes(h[:4], "big") % 1_000_000
    return n, f"{n // 1000:03d} {n % 1000:03d}"


def check_device_ids(v):
    for case in v["cases"]:
        pk = unhex(case["public_key"])
        check(f"device_id {case['public_key'][:8]}", device_id(pk), case["device_id"])
        check(f"grouped {case['public_key'][:8]}", grouped(device_id(pk)), case["grouped"])


def check_invite_psk(v):
    for case in v["cases"]:
        psk = invite_psk(unhex(case["invite_token"]), unhex(case["station_public_key"]))
        check(f"invite psk {case['invite_token'][:8]}", psk.hex(), case["psk"])


def check_code_psk(v):
    for case in v["normalisation"]:
        check(f"normalise {case['input']!r}", normalise_code(case["input"]), case["normalised"])
    for case in v["cases"]:
        check(f"code salt {case['code']}", code_salt(case["station_device_id"]).hex(), case["salt"])
        psk = code_psk(case["code"], case["station_device_id"])
        check(f"code psk {case['code']}", psk.hex(), case["psk"])


def make_party(v, mode, initiator, psk):
    me = v["listener"] if initiator else v["station"]
    noise = NoiseConnection.from_name(v["protocol_name"].encode("ascii"))
    if initiator:
        noise.set_as_initiator()
    else:
        noise.set_as_responder()
    noise.set_prologue(unhex(v["prologue"]))
    noise.set_keypair_from_private_bytes(Keypair.STATIC, unhex(me["static_secret"]))
    noise.set_keypair_from_private_bytes(Keypair.EPHEMERAL, unhex(me["ephemeral_secret"]))
    if initiator and mode != "code":
        noise.set_keypair_from_public_bytes(
            Keypair.REMOTE_STATIC, unhex(v["station"]["static_public"])
        )
    if psk is not None:
        noise.set_psks(psk=psk)
    noise.start_handshake()
    return noise


def check_handshake(v):
    mode = v["mode"]
    label = f"handshake {mode}"
    check(
        f"{label} protocol",
        {
            "trusted": "Noise_IK_25519_ChaChaPoly_BLAKE2s",
            "invite": "Noise_IKpsk2_25519_ChaChaPoly_BLAKE2s",
            "code": "Noise_XXpsk3_25519_ChaChaPoly_BLAKE2s",
        }[mode],
        v["protocol_name"],
    )
    for side in ("listener", "station"):
        party = v[side]
        check(
            f"{label} {side} public",
            x25519_public(unhex(party["static_secret"])).hex(),
            party["static_public"],
        )
    check(f"{label} listener preface", f"4c554c4c01{MODE_BYTE[mode]:02x}", v["listener"]["preface"])
    check(f"{label} station preface", "4c554c4c0100", v["station"]["preface"])
    prologue = b"lull/1" + unhex(v["listener"]["preface"]) + unhex(v["station"]["preface"])
    check(f"{label} prologue", prologue.hex(), v["prologue"])

    station_public = unhex(v["station"]["static_public"])
    psk = None
    if mode == "invite":
        psk = invite_psk(unhex(v["invite_token"]), station_public)
    elif mode == "code":
        code = normalise_code(v["pairing_code"])
        check(f"{label} station id", device_id(station_public), v["station_device_id"])
        psk = code_psk(code, v["station_device_id"])
    check(f"{label} psk", psk.hex() if psk else None, v.get("psk"))

    listener = make_party(v, mode, True, psk)
    station = make_party(v, mode, False, psk)
    parties = {"listener": (listener, station), "station": (station, listener)}
    for i, m in enumerate(v["handshake_messages"]):
        tx, rx = parties[m["sender"]]
        message = bytes(tx.write_message(unhex(m["payload"])))
        check(f"{label} message {i}", message.hex(), m["message"])
        check(f"{label} payload {i}", bytes(rx.read_message(message)).hex(), "")
    for side, (noise, _) in parties.items():
        check(f"{label} {side} complete", noise.handshake_finished, True)
    h = listener.get_handshake_hash()
    check(f"{label} hash", h.hex(), v["handshake_hash"])
    check(f"{label} hash agrees", station.get_handshake_hash().hex(), v["handshake_hash"])
    if mode == "trusted":
        check(f"{label} verification", None, v["verification_number"])
    else:
        n, text = verification(h)
        check(f"{label} verification", n, v["verification_number"])
        check(f"{label} verification display", text, v["verification_display"])
    for i, m in enumerate(v["transport_messages"]):
        tx, rx = parties[m["sender"]]
        message = tx.encrypt(unhex(m["payload"]))
        check(f"{label} transport {i}", message.hex(), m["message"])
        check(f"{label} transport {i} decrypt", rx.decrypt(message).hex(), m["payload"])


def main():
    root = Path(__file__).resolve().parents[2]
    directory = Path(sys.argv[1]) if len(sys.argv) > 1 else root / "vectors" / "crypto"
    checks = {
        "device_ids.json": check_device_ids,
        "invite_psk.json": check_invite_psk,
        "code_psk.json": check_code_psk,
        "handshake_trusted.json": check_handshake,
        "handshake_invite.json": check_handshake,
        "handshake_code.json": check_handshake,
    }
    for name, fn in checks.items():
        before = failures
        fn(json.loads((directory / name).read_text()))
        print(f"{'ok  ' if failures == before else 'FAIL'} {name}")
    if failures:
        print(f"{failures} mismatches")
        return 1
    print("all crypto vectors agree")
    return 0


if __name__ == "__main__":
    sys.exit(main())
