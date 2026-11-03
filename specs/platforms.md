# Platforms

## 1. Matrix

| Platform | Roles | Minimum | UI | Core binding |
|----------|-------|---------|----|--------------|
| Linux | station, listener | x86_64 and aarch64, glibc 2.31; PipeWire, PulseAudio or ALSA | Dioxus desktop | Rust directly |
| macOS | station, listener | macOS 12, Apple silicon and Intel | Dioxus desktop | Rust directly |
| Windows | station, listener | Windows 10 21H2, x86_64 and arm64 | Dioxus desktop | Rust directly |
| Browser | listener | Chrome/Edge 113, Firefox 128, Safari 17 | Dioxus web (wasm) | Rust compiled to wasm32 |
| Android | station, listener | Android 6.0 (API 23) | Jetpack Compose | UniFFI (Kotlin) |
| iOS / iPadOS | station, listener | iOS 15 | SwiftUI | UniFFI (Swift) |

## 2. Obligations per platform

### 2.1 All native platforms

- Generate and store the device key as security.md §5 requires.
- Advertise and browse mDNS (`_lull._tcp`).
- Keep a station running while the screen is off where possible, and say
  clearly when it is not.

### 2.2 Android

- Station and listener run in a foreground service
  (`FOREGROUND_SERVICE_MICROPHONE`, `..._CAMERA` while video streams,
  `..._MEDIA_PLAYBACK` for listeners on API 34+; `foregroundServiceType` on
  API 29+; plain foreground service on API 23–28).
- Request `POST_NOTIFICATIONS` on API 33+, `RECORD_AUDIO` and `CAMERA` at
  first use. Hold a partial wake lock and a Wi-Fi lock while active.
- Offer to exempt the app from battery optimisation, explaining why.
- Use NSD for mDNS; acquire a `MulticastLock` while browsing.

### 2.3 iOS

- `UIBackgroundModes`: `audio` (station capture and listener playback keep
  the app alive). The station uses an `AVAudioSession` category of
  `playAndRecord` with `mixWithOthers` off.
- `NSMicrophoneUsageDescription`, `NSCameraUsageDescription`,
  `NSLocalNetworkUsageDescription`, `NSBonjourServices = [_lull._tcp]`.
- Camera capture stops when the app leaves the foreground (iOS rule); the
  station reports `video-state unavailable` and resumes on return.
- Alerts use local notifications with time-sensitive interruption level.

### 2.4 Desktop

- Tray / menu-bar icon showing state; closing the window keeps a running
  station or listener alive in the tray.
- Prevent system sleep while a station is active (inhibit on Linux via
  logind, `IOPMAssertion` on macOS, `SetThreadExecutionState` on Windows).
- The station serves the browser listener (protocol.md §3.3).

### 2.5 Browser

- Listener only. Talk-back only in a secure context.
- Audio output starts after the first user gesture (browser rule); the UI
  asks for a tap before playback.
- Uses the Web Audio API for playback and capture; Opus via WebCodecs where
  available, otherwise codec 2.
- Offers notifications via the Notification API when permitted; alerts are
  otherwise visual plus a sound.

## 3. Clarifications

Resolved implementation questions; normative.

- **PLATFORM-1** (security.md §5). Windows asks for a DPAPI-protected file; DPAPI needs `unsafe` FFI that the workspace forbids, and the Credential Manager limits blobs to 2560 bytes, which a trust store can outgrow. **Resolution:** Secrets are files sealed with XChaCha20-Poly1305 under a random 32-byte wrapping key held in the Credential Manager (`sealed` store, AAD = secret name). Same user-scope protection as DPAPI.
- **PLATFORM-2** (security.md §5). Linux "Secret Service when available, else a 0600 file": what happens to secrets written to the file while Secret Service was unavailable, once it becomes available? **Resolution:** Reads try the OS store, then the file store; writes go to the OS store and remove any file copy; deletes remove both. Secrets written to the OS store are not visible on a later run without it.
- **PLATFORM-3** (security.md §5). Naming of stored secrets is unspecified. **Resolution:** Names are `[a-z0-9._-]{1,64}`, not starting with `.`. OS stores use service `org.lull.Lull`, account = name; files are `<data dir>/keys/<name>.secret` (`$XDG_DATA_HOME/lull/keys` on Linux), directory 0700, files 0600, atomic replace.
- **PLATFORM-4** (platforms.md §2.4). Which sleep is inhibited, and should the screen stay on? **Resolution:** Idle and explicit sleep are inhibited (logind `sleep:idle` block locks on Linux, via keepawake, falling back to a `systemd-inhibit` child); the display may turn off.
- **PLATFORM-5** (protocol.md §7.7). Desktop cameras have no front/back. Which camera does `video-options camera` select? **Resolution:** Facing is guessed from the device name: built-in ("Integrated", "Built-in", "FaceTime", "front") is `front`; "back"/"rear" is `back`; others are unknown and match neither. `back` also matches external cameras. Without a match the first camera is used.
- **PLATFORM-6** (media.md §5, protocol.md §7.7). V4L2 lists metadata nodes and infrared sensors as video devices. **Resolution:** Only nodes that offer a colour capture format are cameras (grey-only IR sensors are not listed).
- **PLATFORM-7** (security.md PRIV-2). Without a notification service a log-only fallback is needed, but alert titles and bodies name stations. **Resolution:** The log fallback records only that an alert of a given urgency could not be shown, never its text.
- **PLATFORM-8** (protocol.md §7.6, product.md FR-AL-3). What does `charging` mean for a full battery or one held at a charge limit on mains power? **Resolution:** `charging` = on external power: a mains/USB supply is online, or the battery reports charging (or full when no supply node exists). Multiple batteries combine by energy; peripheral batteries (sysfs `scope=Device`) are ignored.
- **PLATFORM-9** (media.md §1, platforms.md §1). Linux lists PipeWire, PulseAudio and ALSA; which path is used? **Resolution:** When `pactl info` succeeds, audio goes through the sound server with `parec`/`pacat` (16-bit, server-side resampling, client name `Lull`); otherwise cpal (ALSA) when built with the `cpal` feature. Playback through a pipe is paced by the wall clock with 40 ms lead; the pipe is shrunk to 4 KiB so device clock drift adds at most ~128 ms of latency; underruns are those `pacat --verbose` reports.
