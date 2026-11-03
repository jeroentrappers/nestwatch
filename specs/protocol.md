# Nestwatch Wire Protocol 1 (NWP/1)

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

`device_id = base32(SHA-256("nestwatch/1 device id" || public_key)[0..10])`,
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
connections at path `/nestwatch/1` on both. Over WebSocket, the preface (§4) is
sent as one binary message, and each subsequent binary message carries
exactly one `noise_message` **without** the length prefix. Text messages are
a protocol error.

The HTTPS certificate is self-signed (ECDSA P-256), generated once per
station and valid for 10 years. Its SHA-256 fingerprint is shown on the
station so users can verify it when accepting it in a browser.

### 3.4 Discovery

Stations advertise the DNS-SD service `_nestwatch._tcp.local.` while active.

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
nestwatch://invite?v=1&s=<station public key>&t=<invite token>&n=<name>&a=<endpoint>&a=<endpoint>...
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
"NEST" (4 bytes ASCII) || version (u8 = 1) || mode (u8)
```

| Mode | Value | Use |
|------|-------|-----|
| `trusted` | 1 | Reconnect of an already paired listener |
| `invite` | 2 | First pairing with an invitation |
| `code` | 3 | First pairing with a pairing code |

The station answers 6 bytes:

```
"NEST" || version (u8 = 1) || status (u8)
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
`"nestwatch/1" || listener_preface || station_preface` (18 bytes).

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

`psk = HKDF-SHA256(ikm = invite_token, salt = station_public_key, info = "nestwatch/1 invite psk", L = 32)`.

The station accepts if pairing mode is open and the handshake succeeds. In
`IKpsk2` the station's handshake completes before it has proof that the
listener knew the PSK, so the station MUST add the listener's static key to
its trusted list only after the first transport message from the listener
(its `hello`) decrypts successfully. The same rule applies to mode `code`.

### 5.3 Mode `code`: `XXpsk3`

A pairing code is 10 characters from the Crockford base32 alphabet
(`0123456789ABCDEFGHJKMNPQRSTVWXYZ`), shown as `XXXXX-XXXXX`. Input is
normalised by removing spaces and hyphens, upper-casing, and mapping
`O→0`, `I→1`, `L→1`. A code carries 50 bits of entropy.

`psk = Argon2id(password = normalised_code, salt = SHA-256("nestwatch/1 code salt" || station_device_id_ascii)[0..16], m = 19456 KiB, t = 2, p = 1, length = 32)`

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
years; there is no rekeying in NWP/1.

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

## Appendix A. Clarifications

Resolved implementation questions. These statements are normative and take
precedence over looser wording above.

- **CRYPTO-1** (protocol.md §5.3). The listener checks the station key against `device_id` "after the handshake". May it check earlier? **Resolution:** The listener checks as soon as the station's static key is known (after reading message 2) and aborts before writing message 3, so a station with the wrong key never receives the PSK-bound message it could guess against offline. On the wire this is identical for honest stations.
- **CRYPTO-2** (protocol.md §5.3). Which text form of `station_device_id_ascii` goes into the code salt? **Resolution:** The canonical form: 16 lower-case base32 characters without hyphens (as in the `id` TXT key).
- **CRYPTO-3** (protocol.md §5.3). Argon2id version and optional inputs are not stated. **Resolution:** Version 0x13 (RFC 9106), password = the 10 ASCII characters of the normalised code, no secret key, no associated data.
- **CRYPTO-5** (protocol.md §5.6). "Close before the nonce reaches 2^32": is nonce 2^32 − 1 still usable? **Resolution:** Nonces 0 .. 2^32 − 1 are usable (2^32 messages per direction); encrypting or decrypting with nonce 2^32 fails with a nonce-exhausted error.
- **CRYPTO-6** (protocol.md §2.1, security.md §5). The stored format of the device secret is not specified. **Resolution:** 32 raw X25519 secret bytes as generated (not pre-clamped); the public key is X25519(secret, 9), clamping applied as RFC 7748 specifies.
- **CRYPTO-7** (protocol.md §5.3). Code normalisation: which whitespace is removed, and how is non-ASCII input upper-cased? **Resolution:** Only ASCII spaces and hyphens are removed; upper-casing is ASCII-only; any other character (tabs, `U`, non-ASCII) rejects the input.
- **CRYPTO-8** (protocol.md §2.2). Should device ID parsing accept the grouped and upper-case forms? **Resolution:** Parsing ignores hyphens and accepts either case; the canonical (displayed in TXT) form is lower case without hyphens.
- **PROTO-1** (protocol.md §4). No status is defined for a version-1 listener preface with an unknown mode, and the version byte of a station's `unsupported-version` reply is not stated. **Resolution:** `nestwatch-proto` reports `UnknownMode`; stations should answer `unsupported-version` and close. A station always sends its own version (1) in its preface.
- **PROTO-2** (protocol.md §3.1, §6). Maximum record size is not stated. **Resolution:** Derived from Noise: a transport message is at most 65535 bytes including the 16-byte tag, so a record is at most 65519 bytes and a VIDEO fragment payload at most 65501 bytes.
- **PROTO-3** (protocol.md §7.1, test-vectors.md §1). The CBOR encoding form is not specified, yet vectors compare bytes. **Resolution:** Encoders use RFC 8949 §4.2.1 core deterministic encoding (definite lengths, shortest ints, shortest exact float of f16/f32, keys sorted by encoded bytes). Decoders accept any float width, indefinite-length maps/arrays and keys in any order; duplicate keys and trailing bytes are errors; non-text keys are skipped like unknown keys.
- **PROTO-4** (protocol.md §7.2–§7.12). Which keys are required is implicit. `listen.threshold` is "used when activated"; `status` keys are "uint or null". **Resolution:** Every key without `?` is required, including `threshold` in continuous mode. Nullable keys (`battery`, `charging`, `talker`) must be present and may be `null`. Optional keys (`?`) treat `null` as absent and are omitted when encoding.
- **PROTO-5** (protocol.md §7, §10). Handling of unknown text values for enumerated keys (`role`, `mode`, `camera`, `state`, `reason`). **Resolution:** Unknown values are a `protocol-error`, except `close.reason`, which is kept as an opaque string so a close is never rejected.
- **PROTO-6** (protocol.md §7). Integer ranges beyond the stated ones. **Resolution:** Codec ids must fit u8 (they are u8 in records); `battery` is 0..=100; other uints must fit u32; a CBOR integer is not accepted where a float is specified (and vice versa); `level.dbfs` must be finite.
- **PROTO-7** (protocol.md §7.6, §7.8). No length limit for `status.talker` and `talk-state.by`. **Resolution:** They are listener display names, so the 64-byte name limit applies. `app` and `close.detail` are unlimited.
- **PROTO-8** (protocol.md §6, §6.1). Undefined AUDIO flag bits, stream values other than 0/1, empty payloads, and PING bodies of other lengths. **Resolution:** Undefined flag bits are sent as 0 and ignored on receipt; an unknown stream is a malformed body; an empty payload is accepted by the record layer (codecs decide); PING/PONG bodies must be exactly 8 bytes.
- **PROTO-9** (protocol.md §6.2). Behaviour on inconsistent fragments (header fields differing within a `frame_id`, non-uniform fragment size, last fragment larger than the others, duplicate index, empty fragment) and how many incomplete frames to keep. **Resolution:** Inconsistent fragments are an error (the frame is discarded; the session layer may close with `protocol-error`). Up to 3 incomplete frames are kept, oldest evicted. `frame_id` is compared with RFC 1982 wrapping arithmetic; fragments of frames not newer than the last delivered one are dropped.
- **PROTO-10** (protocol.md §3.5). Invitation parsing details: required parameters, more than 8 `a`, duplicates, case, which characters to percent-encode, and how the browser fragment is encoded. **Resolution:** `v`, `s`, `t` required; `n` optional (empty name); `a` beyond the eighth ignored; duplicate `v`/`s`/`t`/`n` are errors; scheme and host `invite` match case-insensitively; `n` and `a` escape everything except RFC 3986 unreserved characters (`+` is literal); the web form percent-encodes the whole invitation URI with the same set after `#`. Keys/tokens with padding or wrong length are rejected.
- **PROTO-11** (protocol.md §3.5). Endpoint list rules beyond link-local IPv6, order inside the VPN group, and host syntax. **Resolution:** Loopback, unspecified, multicast and broadcast addresses are also dropped; IPv4 before IPv6 inside the VPN group as well; IPv4 link-local is kept; interface prefixes match case-insensitively; duplicates removed. DNS names are lower-cased, a trailing dot removed, `_` allowed, all-numeric last labels rejected; port 0 and IPv6 zone IDs are invalid.
- **PROTO-12** (protocol.md §3.4). TXT parsing: whether `http`/`https` are mandatory, key case, repeated keys. **Resolution:** `v`, `id`, `pair` are required; `http`/`https` optional; keys case-insensitive and the first occurrence wins (RFC 6763 §6.4); `v` other than `1` is rejected.
- **NET-1** (protocol.md §3.2). How to listen on "all interfaces (IPv4 and IPv6)" and what makes a port "unavailable". **Resolution:** One dual-stack `[::]` socket (`IPV6_V6ONLY` off) when possible, else separate `0.0.0.0` and `[::]` sockets on the same port (IPv6 skipped if the host has none). Any bind failure of a required socket moves to the next port. `SO_REUSEADDR` is set except on Windows. IPv4-mapped peer addresses are reported as IPv4.
- **NET-2** (protocol.md §3.3). The HTTP fallback range 47180..47189 overlaps the HTTPS default 47181. **Resolution:** HTTP is bound first; HTTPS then takes the first free port from 47181 upward, so the two never collide. Both bound ports go into TXT `http`/`https`.
- **NET-3** (protocol.md §3.3). Maximum WebSocket frame/message size is not stated. **Resolution:** Frames and messages above 64 KiB are rejected by the WebSocket layer (the connection fails); the session layer checks the exact 65535-byte Noise limit. Senders refuse messages above 65535 bytes locally. Ping/pong/close frames are handled by the transport and never reach the session.
- **NET-4** (protocol.md §3.3, security.md §7). Should the station restrict which web origins may open `/nestwatch/1`? **Resolution:** Yes: if an `Origin` header is present its `host[:port]` must equal the request's `Host` (case-insensitive, scheme ignored), otherwise 403. Requests without `Origin` (native clients) are accepted. Configurable for deployments behind a proxy.
- **NET-5** (protocol.md §3.3, security.md PRIV-1). Which HTTP headers and routing rules the app server uses. **Resolution:** CSP `default-src 'none'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self'; img-src 'self' data: blob:; font-src 'self'; media-src 'self' blob:; connect-src 'self' ws://<Host> wss://<Host>; worker-src 'self' blob:; manifest-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'` (explicit ws/wss for browsers whose `'self'` misses WebSocket schemes), plus `nosniff`, `X-Frame-Options: DENY`, `Referrer-Policy: no-referrer`, COOP/CORP same-origin, a Permissions-Policy allowing only the microphone for self, and `Cache-Control: no-cache`. No HSTS (HTTP and self-signed HTTPS coexist). Only GET/HEAD; extensionless unknown paths serve `index.html` (app routes), unknown files with an extension are 404.
- **NET-6** (protocol.md §3.3). Certificate details and fingerprint display form. **Resolution:** Subject CN `Nestwatch station`; SANs exactly the names/addresses the caller passes; `notBefore` back-dated one day for clock skew, `notAfter` 3653 days after generation; key stored as PKCS#8 PEM. Fingerprint = SHA-256 of the DER certificate as upper-case hex pairs joined by `:` (as browsers show it).
- **NET-7** (protocol.md §3.4). SRV target host name and instance name limits. **Resolution:** SRV host is `nestwatch-<device_id>.local.` (does not reveal the computer's name). The display name is trimmed and truncated to 63 bytes at a character boundary (empty becomes `Nestwatch`); conflicts are resolved by the responder as `Name (2)`. TXT changes are re-announced by re-registering the service.
- **NET-8** (protocol.md §3.4, §3.5). Which discovered addresses a listener receives. **Resolution:** IPv6 link-local addresses are dropped (endpoints cannot carry a zone, PROTO-11); resolutions without any remaining address are not reported; addresses from successive resolutions of one instance are merged (at most 16), IPv4 first; an instance whose TXT becomes malformed is reported as lost.
- **NET-9** (protocol.md §3.5). How the VPN DNS name is found. **Resolution:** `tailscale status --json` (on `PATH`, or the macOS app bundle binary) with a 2 s timeout; `Self.DNSName` is used when `BackendState` is `Running` or absent. Any failure means no name.
- **NET-10** (protocol.md §3.2, §3.5). Dial behaviour for names with several addresses and the timeout scope. **Resolution:** Addresses are tried happy-eyeballs style (families interleaved, next attempt after 250 ms or on failure, first success wins). The timeout covers resolution and connection of one endpoint. Trying several endpoints is sequential in the caller's order.
- **NET-11** (protocol.md §8). Backpressure for the browser WebSocket, which has no awaitable send. **Resolution:** `send` returns after queuing once the browser's `bufferedAmount` is at most 256 KiB, polling every 5 ms.
- **SESSION-1** (protocol.md §4, §9). Which status wins when several non-`ok` statuses apply, and whether `busy`/`rate-limited` apply to `trusted` mode. **Resolution:** Checked in this order: `unsupported-version` (also for an unknown mode, PROTO-1), `busy`, `rate-limited`, `pairing-closed`. `busy` applies to every mode; whether to answer `rate-limited` is the station policy's decision per mode. A preface without the `NEST` magic (or, over WebSocket, a first message that is not exactly 6 bytes) gets no answer at all.
- **SESSION-2** (protocol.md §4). "A station MUST answer within 5 s" and "the whole handshake MUST complete within 15 s": what is measured, and does the listener have a deadline? **Resolution:** The station closes silently if no complete listener preface arrived 5 s after accept. Both roles close silently if setup (Noise handshake plus receipt of the peer's `hello`) is not done 15 s after the session started; including the `hello` matters because in `IKpsk2` the station's handshake completes before the listener proved the PSK.
- **SESSION-3** (protocol.md §7.2). "Neither side sends any other record before receiving the peer's `hello`" would deadlock if read literally; what may arrive before and during setup? **Resolution:** Each side sends its own `hello` as soon as its handshake completes (station: right after message 2 in `IK`/`IKpsk2`, after message 3 in `XXpsk3`), then nothing else until the peer's `hello`. The first known record must be the peer's `hello` with the right `role`. The listener then requires `paired` (pairing modes only, rejected in `trusted`), `audio-config` and `status` in that order; PING/PONG are allowed after `hello`. Unknown record types and unknown control types are ignored at any time; `close` is honoured at any time; anything else is a `protocol-error`. A pairing listener is trusted only when its first record decrypts *and* is a valid `hello`.
- **SESSION-4** (protocol.md §6, §7). Handling of valid records and messages sent in the wrong direction (for example `status` to a station, AUDIO stream 0 or VIDEO to a station, stream 1 to a listener), a second `hello`, and inconsistent VIDEO fragments (PROTO-9). **Resolution:** Wrong-direction messages and streams are ignored like unknown ones. A second `hello` is a `protocol-error`. An inconsistent or oversized VIDEO frame is discarded without ending the session. `audio-config` and `status` after setup are passed to the application.
- **SESSION-5** (protocol.md §8, media.md §5). "The VIDEO queue holds at most one complete frame; a newer frame replaces a queued one": what if the queued frame is partly sent? **Resolution:** A frame whose first fragment was handed to the socket is finished; at most one complete frame waits behind it, and a newer frame replaces that waiting frame. Every replacement is reported for adaptation. Replacing partly sent frames would mean that on a link slower than one frame per frame interval no frame ever completes.
- **SESSION-6** (protocol.md §5.6, §7.12). No `close` reason exists for nonce exhaustion. **Resolution:** The last usable sending nonce is reserved for `close` with reason `nonce-limit` (receivers keep unknown reasons, PROTO-5); a side whose receiving nonce reaches the limit also closes with `nonce-limit`.
- **SESSION-7** (protocol.md §7.9). What counts as "sent" and "received" for keep-alive, and when keep-alive starts. **Resolution:** "Sent" means bytes handed to the I/O layer; "received" means any bytes from the transport. PING starts once the peer's `hello` was received and is only queued when nothing else is waiting to be sent. On the 15 s receive timeout the session sends `close` with reason `idle` (best effort) and ends.
- **SESSION-8** (protocol.md §8). "If the CONTROL queue cannot drain for 10 seconds": drained completely, or making progress? **Resolution:** The session closes (without `close`, which could not be sent either) when the CONTROL queue is non-empty and no CONTROL record was taken by the socket for 10 s. The final flush of a `close` is also bounded by 10 s.
- **SESSION-9** (protocol.md §8, media.md §3). Size of "500 ms" of AUDIO and which frame goes when full. **Resolution:** 25 frames of 20 ms. When full, the oldest pre-roll frame is dropped if any is queued (media.md §3 step 5), otherwise the oldest frame.
- **SESSION-10** (protocol.md §7.8). When does a listener stop being "granted"? Who polices stream 1? **Resolution:** A listener stops sending stream 1 when it sends `talk-release` or receives any `talk-state` other than `granted`. The station session delivers stream 1 frames; dropping frames from a listener that is not the talker is the station engine's job.
