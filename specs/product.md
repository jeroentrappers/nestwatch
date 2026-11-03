# Product specification

## 1. Purpose

Lull turns phones, tablets, computers and browsers into a baby monitor. A
**station** sits near the child and captures sound (and, on request, video).
One or more **listeners** follow one or more stations. Everything runs
directly between the user's own devices, on the home network or across a
private VPN the user already operates. There is no account, cloud service,
relay or telemetry.

Lull is not a medical device and does not replace supervision. Every app MUST
say so on first launch and in its about screen.

## 2. Roles and terms

| Term | Meaning |
|------|---------|
| Station | Device in the child's room. Captures audio, optionally video; plays talk-back. |
| Listener | Device a caregiver uses to follow one or more stations. |
| Household | The set of stations and listeners that trust each other. There is no central household object; trust is pairwise. |
| Pairing | One-time introduction that makes a listener trusted by a station. |
| Session | One authenticated connection between a listener and a station. |
| Sound activation | Sending audio to a listener only while the room is louder than that listener's threshold. |
| Talk-back | Listener speaks through the station's speaker. |

Any native app can act as station or listener. The browser app is a listener
only (browsers cannot accept connections or advertise services).

## 3. Functional requirements

### 3.1 Station

- **FR-ST-1** A station MUST capture mono audio continuously while active and
  show a live level meter.
- **FR-ST-2** A station MUST serve at least 8 simultaneously connected
  listeners.
- **FR-ST-3** A station MUST show, at all times while active: how many
  listeners are connected, whether any camera feed is being sent, and
  whether a listener is talking.
- **FR-ST-4** The station's display MUST be dimmable to near-black while
  active (night mode) without stopping monitoring.
- **FR-ST-5** A station MUST keep monitoring with the screen off or the app
  in the background where the platform allows it (see platforms.md), and
  MUST clearly warn when the platform will stop it.
- **FR-ST-6** A station MUST report its battery level and charging state to
  listeners where the platform exposes them.
- **FR-ST-7** A station MUST let the user rename it, list trusted listeners
  (name, last seen) and revoke any of them.
- **FR-ST-8** A station MUST allow turning camera access and talk-back off
  entirely. When off, listeners see the feature as unavailable.

### 3.2 Listener

- **FR-LS-1** A listener MUST be able to follow up to 8 stations at once,
  each with independent connection, volume, mute and alerts.
- **FR-LS-2** Audio of all followed, unmuted stations MUST be mixed into one
  output. A "solo" action MUST let the user hear one station only.
- **FR-LS-3** Each station tile MUST show: name, connection state, live level
  or last reported level, sound-activation state, battery, and the network
  path in use (local network or VPN).
- **FR-LS-4** A listener MUST keep receiving and alerting with the screen off
  or the app in the background where the platform allows it.
- **FR-LS-5** A listener MUST reconnect automatically after network changes,
  trying all known addresses of the station, and MUST show reconnect progress.

### 3.3 Pairing

- **FR-PAIR-1** A station MUST offer an **invitation** as a QR code and as a
  copyable link. Scanning or opening it pairs a listener without typing.
- **FR-PAIR-2** A station MUST offer a **pairing code** of 10 characters for
  devices without a camera. Entering the code pairs a listener found on the
  local network.
- **FR-PAIR-3** Invitations and codes MUST only work while the station is in
  pairing mode, which ends after 10 minutes, when the user closes it, or
  after 8 successful pairings, whichever comes first.
- **FR-PAIR-4** After pairing, the listener MUST reconnect without any code.
  A station's pairing mode MUST NOT be needed for reconnects.
- **FR-PAIR-5** Pairing MUST show both devices a short verification phrase
  derived from the session (see protocol.md §5.5) so users can spot a
  man-in-the-middle when pairing by code.
- **FR-PAIR-6** Revoking a listener on the station MUST end its sessions
  immediately and reject it afterwards. Forgetting a station on a listener
  MUST tell the station (if reachable) so it removes the listener.

### 3.4 Audio and sound activation

- **FR-AU-1** Listeners MUST hear station audio with at most 400 ms mouth-to-ear
  latency on a healthy local network (target 150 ms).
- **FR-AU-2** Each listener MUST be able to set, per station, either
  **continuous** audio or **sound activation** with a threshold and a
  hold time. The threshold UI MUST show the station's current room level so
  the user can place it above the background noise.
- **FR-AU-3** With sound activation, the station MUST NOT send audio to that
  listener while the room stays below the threshold, except for the hold
  time after the last loud moment and a pre-roll of at least 300 ms before
  the trigger, so the onset of a cry is heard.
- **FR-AU-4** The defaults MUST be sound activation at a sensitive threshold
  (`-50 dBFS`), a 4-second hold time and 500 ms pre-roll.
- **FR-AU-5** A station MUST offer microphone sensitivity (gain) settings and
  SHOULD offer automatic gain control, on by default.
- **FR-AU-6** A listener MUST offer per-station volume and mute. Muting MUST
  NOT stop alerts.

### 3.5 Alerts

- **FR-AL-1** A listener MUST alert (sound, vibration where available, and a
  system notification) when a station that was quiet starts sending sound.
  Repeated alerts for the same station MUST be rate-limited (default: not
  more than once per 30 seconds).
- **FR-AL-2** A listener MUST alert when a station has been unreachable for
  longer than 20 seconds, and when it becomes reachable again.
- **FR-AL-3** A listener MUST alert when a station reports battery below 15 %
  while not charging.
- **FR-AL-4** Alerts MUST work while the listener is muted and, where the
  platform allows, while it is in the background.

### 3.6 Talk-back

- **FR-TB-1** A listener MUST offer push-to-talk per station. Only one
  listener may talk to a station at a time; others see who is talking.
- **FR-TB-2** While a listener talks, its own playback of that station MUST
  be muted to avoid feedback.
- **FR-TB-3** A station MUST play talk-back at a user-set volume and MUST
  indicate it visibly.

### 3.7 Camera

- **FR-CAM-1** A listener MUST be able to open a live camera view per station
  on demand. The station's camera MUST be off whenever no listener has an
  open view.
- **FR-CAM-2** The station MUST show a persistent, unmistakable indicator
  while its camera is sending.
- **FR-CAM-3** Video MUST adapt to the link: lower frame rate and quality
  before audio is affected. Audio always has priority over video.
- **FR-CAM-4** A listener MAY switch the station between front and back
  cameras and toggle a night-friendly (grayscale, boosted) image.

### 3.8 Networks

- **FR-NET-1** Stations MUST be discoverable on the local network via mDNS.
- **FR-NET-2** Lull MUST work across a user-operated VPN (for example
  Tailscale or WireGuard) without mDNS: invitations carry all station
  addresses including VPN addresses and VPN DNS names, and listeners MUST
  accept manually entered host names and addresses.
- **FR-NET-3** Listeners MUST remember every address that worked for a
  station and try them in order of last success.
- **FR-NET-4** No component may contact any server other than the user's own
  devices. Updates are delivered by the platform's normal channels.

### 3.9 Browser listener

- **FR-WEB-1** A native station MUST serve the browser listener app and a
  WebSocket endpoint on the local network (see protocol.md §3.3).
- **FR-WEB-2** The browser listener MUST provide listener features except
  background operation; talk-back requires a secure context (HTTPS or
  `localhost`), so stations SHOULD serve HTTPS with a self-signed
  certificate whose fingerprint the station displays.
- **FR-WEB-3** The browser listener MUST store its own device key so that it
  stays paired across reloads.

## 4. Non-functional requirements

- **NFR-LAT-1** Audio latency: see FR-AU-1.
- **NFR-BW-1** Continuous audio MUST fit in 40 kbit/s per listener with the
  preferred codec; the baseline codec MUST fit in 80 kbit/s.
- **NFR-BAT-1** A phone station on mains power MUST run for 12 hours without
  user interaction. A listener with sound activation and screen off SHOULD
  use less than 5 % battery per hour on a modern phone.
- **NFR-REL-1** A listener MUST detect a dead session within 15 seconds.
- **NFR-PRIV-1** No audio, video, level data or identifiers may leave the
  household's devices. No analytics, no crash upload.
- **NFR-A11Y-1** Every screen MUST work with the platform screen reader and
  with large text; status MUST never rely on colour alone.
- **NFR-I18N-1** User-facing text MUST be externalised for translation;
  English is the reference language.

## 5. Out of scope (version 1)

Cloud relays or accounts, recording or history of audio/video, breathing or
movement analysis, smart-home integrations, lullaby playback, multi-household
sharing beyond pairwise pairing.

## 6. Clarifications

Resolved implementation questions; normative.

- **CORE-1** (protocol.md §2.1, §2.3, security.md §5). Stored format of the device key, trust stores and settings. **Resolution:** Each value is one version byte (`1`) followed by one CBOR item (minicbor index-keyed maps). Storage keys: `device-key` (`{0: secret bytes}`, shared by the station and listener roles of one installation), `station-trust`, `station-settings`, `station-cert` (PEM certificate and key), `listener-stations`, `listener-settings`. Unknown versions are an error, never silently reset.
- **CORE-2** (protocol.md §4, §9). What counts towards the 8-listener limit, and limits for connections still in the handshake. **Resolution:** `busy` is answered when the number of set-up sessions is at the limit when the preface arrives. At most 16 connections may be in preface or handshake at once; more are closed without an answer. A reconnecting listener whose stale session still counts may see `busy` until the stale session times out (≤ 15 s).
- **CORE-3** (protocol.md §4 (`rate-limited`), security.md §4). When to answer `rate-limited`. **Resolution:** Per remote address: 3 failed pairing handshakes (`invite`/`code` sessions that fail in the handshake, transport or before `hello`) within the last 30 s make further `invite`/`code` prefaces from that address `rate-limited`. `trusted` is never rate limited, so the answer reveals nothing about keys. Independently, 5 failed code attempts close the window (§5.3).
- **CORE-4** (product.md FR-PAIR-3, protocol.md §5.3). Pairing window details. **Resolution:** Opening while a window is open returns the open window. The code PSK is derived once on a blocking thread when the window opens. Expiry is checked every 250 ms and at every pairing preface. A pairing whose handshake began while the window was open completes even if the window closed meanwhile. The web URL uses the first advertised endpoint's host and the HTTP port.
- **CORE-5** (product.md FR-AL-1, MEDIA-7). Continuous-mode listeners get no `activity`, yet must alert when sound starts. **Resolution:** In continuous mode the listener raises the sound alert when `level.dbfs` rises to or above its own threshold setting; in activated mode on `activity {open: true}`. One 30 s rate limit per station covers both.
- **CORE-6** (protocol.md §7.10). Close reason after `unpair`. **Resolution:** The station removes the listener and closes with reason `revoked`, detail `unpaired`.
- **CORE-7** (product.md FR-AL-2, FR-AL-3). Timing and re-arming of reachability and battery alerts. **Resolution:** "Unreachable" counts from the end of the last live session (or from the start of following) and fires once per outage; "reachable again" only follows an "unreachable" alert. The battery alert fires once per drop below 15 % and re-arms when charging or at ≥ 15 %; unknown charging state counts as not charging.
- **CORE-8** (media.md §5, product.md FR-CAM-1, FR-CAM-4). Camera aggregation over listeners. **Resolution:** The camera request carries the largest `max_w`, `max_h` and `max_fps` of all viewers; facing and night come from the most recent `video-options` of any listener (one shared capture); default facing is `back`. `video-start` values are clamped (fps 1..30, sizes 16..4096). The camera is released at once when the last viewer stops or leaves. `video-state streaming` is sent with the first frame and whenever size or fps change. Refusals: `disabled` (FR-ST-8), `unavailable` (also when the listener offers no JPEG), `denied`. `status.camera` is true while anyone views.
- **CORE-9** (protocol.md §7.8, product.md FR-TB-1). Talk-back corner cases. **Resolution:** A `talk-request` from the current talker is answered `granted` again; `talk-release` from a non-talker is answered `idle`. Talk-back is released when the talker disconnects. Turning talk-back off while someone talks answers that talker `talk-state disabled`.
- **CORE-10** (product.md FR-LS-5, FR-NET-3). Reconnect order and back-off. **Resolution:** Candidates per attempt: the configured WebSocket URL (browser app), fresh mDNS addresses for the station's `id`, then stored routes by last success (never-successful ones last). Each route gets 3 s. After a failed round the delay doubles from 0.5 s to at most 10 s; after a live session ended the first retry follows after 0.5 s. `close revoked` stops reconnecting and marks the station "needs pairing" (persisted). A pairing session stays up and becomes the followed session; all invitation endpoints are remembered.
- **CORE-11** (product.md FR-LS-3). How to tell the LAN from the VPN path. **Resolution:** By the connected peer address: `100.64.0.0/10` and `fd7a:115c:a1e0::/48` (protocol.md §3.5) are VPN, anything else LAN. DNS names count by the address they resolved to.
- **CORE-12** (media.md §4, architecture.md §5). Pull-based playback timing and level publishing. **Resolution:** `pull_playback` / `pull_talkback` pull each 20 ms frame from the jitter buffers at the time of the call, so callers call at the output device's pace. Level-only snapshot updates are throttled to 10 Hz. A `level` covering 250 ms without captured audio reports -90 dBFS.
- **CORE-13** (architecture.md §1, security.md §5). Sync or async storage port. **Resolution:** The storage port is synchronous (small values, rare writes). Adapters for asynchronous stores (IndexedDB) keep an in-memory copy and write through.
- **CORE-14** (protocol.md §3.3). HTTPS certificate names and lifetime of stored certificates. **Resolution:** Generated once with SANs `localhost`, `127.0.0.1` and every advertised endpoint host at that time, then reused from storage; it is not regenerated when addresses change (browsers are asked to accept it by fingerprint anyway).
- **UI-1** (design/README.md §4 (`--level`, `--threshold`)). How does a dBFS level map to the 0..1 meter and ring position, and which thresholds can the user pick? **Resolution:** Linear in dB from −90 dBFS (0) to −10 dBFS (1), clamped. Thresholds −85..−10 dBFS in 1 dB steps (keyboard: arrows ±1, Page ±5, Home/End).
- **UI-2** (design/README.md §4 "Level ring"). "Tick marks the strictest listener threshold": highest or lowest threshold, and what about continuous listeners? **Resolution:** The lowest threshold of the connected sound-activated listeners (above it at least one of them hears the room); no tick without one. The ring says "sound" while sound activation is open for any sound-activated listener; continuous listeners do not count. `ConnectedListener` now reports `threshold_dbfs`.
- **UI-3** (product.md FR-PAIR-2, protocol.md §5.3). Code pairing needs the station's `device_id` from mDNS, which a browser cannot browse. **Resolution:** The station's pairing sheet shows its device ID under the code; the browser form asks for it and dials the page's own `ws(s)://host/lull/1`. Native listeners take the ID and addresses from the selected nearby station.
- **UI-4** (security.md §5 "Browser"). IndexedDB with non-extractable Web Crypto keys, or another store? **Resolution:** `localStorage` of the station's origin, values base64 under `lull:<key>`. The Noise implementation needs the raw X25519 secret, so a non-extractable key cannot be used; both stores have the same origin scope, and `localStorage` matches the synchronous storage port (CORE-13) without a cache.
- **UI-5** (product.md FR-AU-2, design/README.md §4 "Calibration"). Where does "the last minute of room level" come from? **Resolution:** The listener records the station's `level` reports itself (one point per 200 ms, peak kept) while the station is live; kept in memory for 60 s, not persisted.
- **UI-6** (product.md §1). When is the medical disclaimer acknowledged, and must it block the app? **Resolution:** It is shown on the welcome screen with the role choice (browser: an "I understand" button) and in About; choosing a role records the acknowledgement in the UI preferences.
- **UI-7** (platforms.md §2.5). Which gesture unlocks browser audio? **Resolution:** The first pointer or key press anywhere creates and resumes a 16 kHz `AudioContext`; a "Tap to start listening" prompt stays visible until playback runs.
- **UI-8** (platforms.md §2.5, product.md FR-AL-1). Browser alerts outside a secure context (plain `http://` LAN address) where the Notification API is unavailable. **Resolution:** Always an in-page notice and a short two-tone sound (even when muted, FR-AL-4); system notifications only in secure contexts after the user enables them.
- **UI-9** (platforms.md §2.5, media.md §4). Talk-back capture rate in the browser. **Resolution:** The microphone runs in its own `AudioContext` at the device rate (some browsers refuse a `MediaStream` source in a 16 kHz context); blocks go to `push_talk_capture` with that rate.
- **UI-10** (protocol.md §3.3, platforms.md §2.4). Where does a desktop station find the browser bundle it serves? **Resolution:** First of `$LULL_WEB_DIR`, `<exe dir>/web`, `<exe dir>/../share/lull/web`, the workspace's `target/lull-web`; otherwise a placeholder page that says the app is missing.
- **UI-11** (product.md FR-CAM-1). How are camera frames shown? **Resolution:** The newest complete JPEG as an `img` with a `data:` URI (allowed by `img-src data:`); same on desktop and web.
