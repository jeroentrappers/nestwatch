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
