# Lull Wire Protocol 1 (LWP/1)

## 1. Overview

A session connects one **listener** (always the initiator) to one
**station** (always the responder) over a reliable, ordered byte transport.
The connection proceeds through four phases:

1. **Preface**: a fixed plaintext exchange that selects the protocol
   version and the handshake mode.
2. **Handshake**: a Noise Protocol Framework handshake that authenticates
   both devices and derives transport keys.
3. **Setup**: both sides exchange `hello` control messages.
4. **Operation**: encrypted records carry control messages, audio, video
   and keep-alives in both directions.

Every byte after the preface is part of a Noise message.

## 2. Identities

### 2.1 Device keys

Every installation generates one X25519 static key pair on first launch, the
**device key**. It never leaves the device. A browser listener stores it in
the origin's persistent storage.

### 2.2 Device ID

`device_id = base32(SHA-256("lull/1 device id" || public_key)[0..10])`,
using the RFC 4648 base32 alphabet in lower case without padding
(16 characters). The ID is shown to users in grouped form (`abcd-efgh-ijkl-mnop`)
and is used in discovery records and code pairing.

### 2.3 Trust stores

- A station keeps a **trusted listener** list: listener public key, display
  name, date paired, date last seen.
- A listener keeps a **known station** list: station public key, display
  name, known endpoints (with last-success time), and per-station settings.

## 3. Transports

### 3.1 Framing

The byte stream is a sequence of **frames**:

```
frame = length (u16) || noise_message (length bytes, 1..65535)
```

### 3.2 TCP

A station listens on TCP port 47100 on all interfaces (IPv4 and IPv6). If the
port is unavailable it tries 47101..47109 and advertises the port it got.
Listeners SHOULD enable `TCP_NODELAY`.

### 3.3 WebSocket

A station serves the browser listener app over HTTP on port 47180 and over
HTTPS on port 47181 (falling back upward like §3.2), and accepts WebSocket
connections at path `/lull/1` on both. Over WebSocket, the preface (§4) is
sent as one binary message, and each subsequent binary message carries
exactly one `noise_message` **without** the length prefix. Text messages are
a protocol error.

The HTTPS certificate is self-signed (ECDSA P-256), generated once per
station and valid for 10 years. Its SHA-256 fingerprint is shown on the
station so users can verify it when accepting it in a browser.

### 3.4 Discovery

Stations advertise the DNS-SD service `_lull._tcp.local.` while active.

- Instance name: the station's display name (made unique by the mDNS
  responder as usual).
- Port: the TCP port of §3.2.
- TXT keys: `v=1`; `id=<device_id>`; `pair=1` while pairing mode is open,
  otherwise `pair=0`; `http=<port>`; `https=<port>`.

Listeners match discovered stations to known stations by `id`, then verify
the full public key during the handshake. mDNS data is never trusted for
authentication.

### 3.5 Endpoints and invitations

An **endpoint** is `host:port` where host is a DNS name, an IPv4 literal or a
bracketed IPv6 literal. A station lists its endpoints in this order: VPN DNS
names (for example a Tailscale MagicDNS name), VPN addresses
(`100.64.0.0/10`, `fd7a:115c:a1e0::/48`, or interfaces named `tailscale*`,
`wg*`, `utun*`, `tun*`), then LAN addresses (IPv4 before IPv6). Link-local
IPv6 addresses are never listed.

An **invitation** is a URI:

```
lull://invite?v=1&s=<station public key>&t=<invite token>&n=<name>&a=<endpoint>&a=<endpoint>...
```

- `s`: 32-byte station public key, base64url.
- `t`: 16-byte random invite token, base64url, valid only while pairing mode
  is open (product.md FR-PAIR-3).
- `n`: station display name, percent-encoded UTF-8, at most 64 bytes.
- `a`: zero to 8 endpoints, percent-encoded.

Unknown parameters MUST be ignored. A browser listener receives the
invitation in the URL fragment of the station's web address
(`http://host:47180/#<percent-encoded invitation>`), which never reaches a
server.

## 4. Preface

The listener sends 6 bytes:

```
"LULL" (4 bytes ASCII) || version (u8 = 1) || mode (u8)
```

| Mode | Value | Use |
|------|-------|-----|
| `trusted` | 1 | Reconnect of an already paired listener |
| `invite` | 2 | First pairing with an invitation |
| `code` | 3 | First pairing with a pairing code |

The station answers 6 bytes:

```
"LULL" || version (u8 = 1) || status (u8)
```

| Status | Value | Meaning |
|--------|-------|---------|
| `ok` | 0 | Proceed with the handshake for the requested mode |
| `unsupported-version` | 1 | Close |
| `pairing-closed` | 2 | Mode `invite`/`code` requested but pairing mode is not open; close |
| `busy` | 3 | Listener limit reached; close |
| `rate-limited` | 4 | Too many failed attempts; retry after 30 s; close |

After a non-`ok` status both sides close. A station MUST answer within
5 seconds of accepting a connection, and the whole handshake MUST complete
within 15 seconds or the station closes the connection.

The **prologue** for the Noise handshake is
`"lull/1" || listener_preface || station_preface` (18 bytes).

## 5. Handshake

All modes use Curve25519, ChaChaPoly and BLAKE2s:
`Noise_<pattern>_25519_ChaChaPoly_BLAKE2s`. The listener is the Noise
initiator.

### 5.1 Mode `trusted`: `IK`

The listener knows the station's static key. The station accepts only if the
listener's static key is in its trusted listener list; otherwise it aborts
the handshake by closing the connection (it MUST NOT send a distinguishable
error, to avoid becoming a key-membership oracle).

### 5.2 Mode `invite`: `IKpsk2`

`psk = HKDF-SHA256(ikm = invite_token, salt = station_public_key, info = "lull/1 invite psk", L = 32)`.

The station accepts if pairing mode is open and the handshake succeeds; it
then adds the listener's static key to its trusted list.

### 5.3 Mode `code`: `XXpsk3`

A pairing code is 10 characters from the Crockford base32 alphabet
(`0123456789ABCDEFGHJKMNPQRSTVWXYZ`), shown as `XXXXX-XXXXX`. Input is
normalised by removing spaces and hyphens, upper-casing, and mapping
`O→0`, `I→1`, `L→1`. A code carries 50 bits of entropy.

`psk = Argon2id(password = normalised_code, salt = SHA-256("lull/1 code salt" || station_device_id_ascii)[0..16], m = 19456 KiB, t = 2, p = 1, length = 32)`

The listener learns `station_device_id` from the station's discovery record
(`id`). After the handshake the listener MUST check that the station's
static key hashes to that `device_id` (§2.2) and abort otherwise.

A station MUST count failed `code` handshakes and close pairing mode after 5
failures. Rationale and the offline-guessing analysis are in security.md §4.

### 5.4 Handshake payloads

Handshake message payloads MUST be empty. Identity and capability data are
exchanged after the handshake (§7.2), when both directions are encrypted
and authenticated.

### 5.5 Verification phrase

After a pairing handshake (`invite` or `code`) both devices display a
verification number derived from the Noise handshake hash `h`:

`n = u32(h[0..4]) mod 1_000_000`, shown as two groups of three digits
(`042 917`). Users compare the numbers; the product UI says what to do if
they differ (forget and pair again).

### 5.6 Transport phase limits

Implementations MUST close the session before either direction's Noise
nonce reaches 2^32. At 50 audio records per second this is more than two
years; there is no rekeying in LWP/1.

## 6. Records

The plaintext of each transport message is one **record**:

```
record = type (u8) || body
```

| Type | Name | Direction | Body |
|------|------|-----------|------|
| `0x01` | CONTROL | both | One CBOR data item (§7) |
| `0x02` | AUDIO | both | §6.1 |
| `0x03` | VIDEO | station→listener | §6.2 |
| `0x04` | PING | both | u64 sender clock in ms |
| `0x05` | PONG | both | the 8 bytes of the PING being answered |

Unknown record types MUST be ignored, so later versions can add records.
A record whose body does not parse MUST end the session with a
`protocol-error` close (§7.12).

### 6.1 AUDIO

```
stream    u8   0 = station microphone, 1 = talk-back
codec     u8   see media.md §2
flags     u8   bit 0: pre-roll (sent late because sound activation opened)
               bit 1: first frame of a burst (after silence, gap or talk start)
seq       u32  per stream and direction, +1 per frame, starting at 0
timestamp u32  capture time in 1/48000 s units, wrapping
payload   ...  one codec frame of 20 ms
```

Stream 0 flows station→listener only, stream 1 listener→station only.
Frames that the sender drops or withholds (sound activation) leave gaps in
`seq`; receivers conceal gaps and MUST NOT treat them as errors. A frame
with a `seq` not greater than the last accepted one for that stream is
discarded.

### 6.2 VIDEO

```
codec        u8   1 = JPEG (baseline)
frame_id     u32  +1 per captured frame
capture_ms   u32  station clock, wrapping
width        u16
height       u16
frag_index   u16  0-based
frag_count   u16  >= 1
payload      ...  bytes [frag_index * fragment size ...] of the encoded frame
```

A sender MUST use one fragment size per frame. Receivers reassemble by
`frame_id`, display the newest complete frame and discard incomplete older
ones. A complete frame MUST NOT exceed 2 MiB.

## 7. Control messages

### 7.1 Encoding

A control message is a CBOR map (RFC 8949) with text keys. Key `"t"` holds
the message type. Unknown keys MUST be ignored. Unknown message types MUST be
ignored. Missing required keys are a `protocol-error`.

Values below use CBOR types: `tstr`, `uint`, `int`, `bool`, `float`,
`array`, `null`.

### 7.2 `hello` (both directions, first record after the handshake)

| Key | Type | Meaning |
|-----|------|---------|
| `t` | `"hello"` | |
| `name` | tstr | Display name, at most 64 bytes |
| `role` | `"station"` / `"listener"` | |
| `app` | tstr | Implementation and version, for diagnostics |
| `caps` | array of tstr | Supported features: `"audio"`, `"vox"`, `"talk"`, `"video"`, `"battery"` |
| `audio` | array of uint | Audio codecs supported, in preference order |
| `video` | array of uint | Video codecs supported (station: available now) |

Neither side sends any other record before receiving the peer's `hello`.
After both `hello`s, the station sends `audio-config` (§7.3) and `status`
(§7.6). A station that was paired in this session also sends `paired`
(§7.10) first.

The listener's capability list tells the station what it may send; the
station's list tells the listener which controls to show. A station whose
user disabled camera or talk-back leaves `"video"`/`"talk"` out.

### 7.3 `audio-config` (station → listener)

`{ t: "audio-config", codec: uint }`: the codec of stream 0 for this session,
the first entry of the listener's `audio` list that the station supports.
Talk-back (stream 1) uses the same codec.

### 7.4 `listen` (listener → station)

| Key | Type | Meaning |
|-----|------|---------|
| `t` | `"listen"` | |
| `mode` | `"continuous"` / `"activated"` | |
| `threshold` | float | dBFS, `-90.0..0.0`, used when `activated` |
| `hold_ms` | uint | `0..60000` |
| `preroll_ms` | uint | `0..2000` |

Sent after `hello` and whenever the user changes it. Until the first
`listen`, a station sends continuous audio.

### 7.5 `level` and `activity` (station → listener)

- `{ t: "level", dbfs: float, open: bool }`: peak short-term level since the
  previous `level` and whether audio currently flows to this listener. Sent
  every 250 ms to every listener. It doubles as the keep-alive.
- `{ t: "activity", open: bool }`: sent the moment sound activation opens or
  closes for this listener. Listeners alert on `open: true` (product.md FR-AL-1).

### 7.6 `status` (station → listener)

| Key | Type | Meaning |
|-----|------|---------|
| `t` | `"status"` | |
| `name` | tstr | Current station name |
| `battery` | uint or null | Percent |
| `charging` | bool or null | |
| `listeners` | uint | Connected listeners |
| `camera` | bool | Camera currently sending to anyone |
| `talker` | tstr or null | Name of the listener talking |

Sent after setup, on every change, and at least every 30 seconds.

### 7.7 Video control

- `{ t: "video-start", max_w: uint, max_h: uint, max_fps: uint, codecs: [uint] }`
  (listener → station).
- `{ t: "video-stop" }` (listener → station).
- `{ t: "video-options", camera: "front"/"back", night: bool }` (listener → station).
- `{ t: "video-state", state: tstr, codec: uint?, w: uint?, h: uint?, fps: uint? }`
  (station → listener), with `state` one of `"streaming"`, `"stopped"`,
  `"unavailable"`, `"denied"` (OS permission), `"disabled"` (user setting).

### 7.8 Talk-back control

- `{ t: "talk-request" }` and `{ t: "talk-release" }` (listener → station).
- `{ t: "talk-state", state: "granted"/"busy"/"idle"/"disabled", by: tstr? }`
  (station → listener). `granted` goes to the requester; other listeners
  learn the talker from `status`. A listener sends AUDIO stream 1 only while
  granted.

### 7.9 `ping` timing

A side that has sent nothing for 5 seconds sends PING. A side that has
received nothing for 15 seconds MUST close the session (NFR-REL-1). PONG
answers each PING immediately; listeners MAY display the round-trip time.

### 7.10 Pairing results

- `{ t: "paired", station_id: tstr }` (station → listener): the listener is
  now trusted; the listener stores the station.
- `{ t: "unpair" }` (listener → station): the listener forgets this station;
  the station removes the listener from its trusted list and closes.

### 7.11 Revocation

When a station revokes a listener it sends `close` with reason `revoked` to
every session of that listener and removes the key. A listener receiving
`revoked` marks the station as "needs pairing".

### 7.12 `close`

`{ t: "close", reason: tstr, detail: tstr? }`, sent before closing the
transport. Reasons: `shutdown`, `revoked`, `protocol-error`, `replaced` (the
same listener connected again), `idle`.

## 8. Scheduling and backpressure

Each session's sender keeps three queues, served in strict priority:
CONTROL/PING/PONG, then AUDIO, then VIDEO.

- The AUDIO queue holds at most 500 ms; when full, the oldest frame is dropped.
- The VIDEO queue holds at most one complete frame; a newer frame replaces a
  queued one. A station SHOULD lower video frame rate and JPEG quality when
  frames are replaced repeatedly (media.md §5).
- If the CONTROL queue cannot drain for 10 seconds, the sender closes the
  session.

## 9. Multiple sessions

- A station accepts at most 8 concurrent listener sessions; the 9th gets
  `busy`.
- If a trusted listener connects while it already has a session, the station
  closes the older one with `replaced`.
- A listener holds at most one session per station and follows up to 8
  stations.

## 10. Versioning

The preface version is incremented only for incompatible changes. Within a
version, new record types, control messages, keys and capability names may
be added; receivers ignore what they do not know.
