# Security and privacy

## 1. Goals

1. Only devices the user paired can hear, see or talk through a station.
2. Nobody on the network path (Wi-Fi, VPN, ISP) can read or alter media or
   control messages, or replay old ones.
3. Losing or reselling a device can be contained by revoking it.
4. Nothing about the household leaves the household's devices.

## 2. Adversaries

| Adversary | Capability | In scope |
|-----------|------------|----------|
| Passive network observer | Records all traffic | Yes |
| Active LAN attacker | Injects, drops, rewrites traffic; runs fake mDNS stations; connects to stations | Yes |
| Guest who saw the QR code | Holds an invitation | Yes, limited by pairing window |
| Malicious paired listener | Holds valid keys | Partly: it can listen until revoked; it cannot impersonate other devices |
| Compromised OS or device | Reads memory and storage | No |

## 3. Mechanisms

- **Authenticated key exchange**: Noise handshakes (protocol.md §5) give
  mutual authentication by static keys, forward secrecy through ephemeral
  keys, and identity hiding for the listener's static key in `IK` modes.
- **Per-device trust**: every listener has its own key; revoking one does not
  affect others, and there is no shared household secret to leak.
- **Pairing window**: invitations and codes work only in pairing mode
  (≤ 10 minutes, ≤ 8 pairings, ≤ 5 failed code attempts).
- **Membership-oracle resistance**: a station rejects unknown `trusted`
  listeners by closing silently, indistinguishably from a network failure.
- **Verification number**: both sides display a number bound to the
  handshake, letting users detect a man-in-the-middle during code pairing.
- **Transport**: all records are Noise transport messages (ChaChaPoly with
  implicit nonces), so reordering, replay and truncation inside a session are
  detected and end the session.

## 4. Code pairing analysis

The code mode (protocol.md §5.3) mixes an Argon2id-stretched 50-bit code
into `XXpsk3`.

- A **passive** observer learns nothing about the code: the PSK is mixed
  together with ephemeral Diffie-Hellman results it cannot compute.
- An attacker posing as the **listener** to a real station gets one guess per
  connection; the station closes pairing mode after 5 failures.
- An attacker posing as the **station** (fake mDNS record) can record the
  listener's third handshake message and guess offline. Each guess costs one
  Argon2id evaluation (19 MiB, 2 passes; roughly 20–50 ms on a desktop core).
  Exhausting half of the 2^50 space then takes on the order of 10^5–10^6
  core-years, while the code is only useful during the pairing window. The
  fake station also fails the `device_id` check unless it controls the
  discovery record, and the verification number differs on the two screens.

Invitation pairing (`IKpsk2`) has none of these issues: the listener already
knows the station's static key, and the 128-bit token is never guessable.

## 5. Key storage

| Platform | Device key and trust store |
|----------|----------------------------|
| Android | Encrypted with an Android Keystore AES-GCM key (API 23+) in app-private storage |
| iOS | Keychain item, `kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly` |
| macOS | Keychain |
| Windows | DPAPI (user scope) protected file in `%APPDATA%` |
| Linux | Secret Service (libsecret) when available, else a `0600` file in `$XDG_DATA_HOME` |
| Browser | IndexedDB of the station's origin; non-extractable where Web Crypto allows, else as stored bytes |

Keys are never exported, logged or included in backups (platform backup
flags MUST exclude them). Moving to a new device means pairing again.

## 6. Privacy rules

- **PRIV-1** No network traffic except to the user's own devices and, only
  when the user enables it, the rendezvous/relay server (rendezvous.md §7
  lists what that server can see). No update checks, analytics, crash
  reporting, fonts or CDN loads (the browser app is fully self-contained and
  served by the station or by the relay server's own origin).
- **PRIV-2** No recording of audio or video, no media in logs. Logs contain
  no device names unless the user exports diagnostics explicitly.
- **PRIV-3** The camera runs only on explicit listener request, with the
  station indicator required by product.md FR-CAM-2, and is never used for
  QR scanning while monitoring.
- **PRIV-4** Microphone and camera permissions are requested only at the
  moment a feature first needs them, with an explanation.

## 7. Browser specifics

The browser listener runs on the station's origin. Over plain HTTP the
session is still end-to-end protected by Noise; HTTPS additionally protects
the delivery of the app code itself and enables talk-back. Users who need
that guarantee use the HTTPS port and verify the certificate fingerprint.
