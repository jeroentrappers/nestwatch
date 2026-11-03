# Architecture

## 1. Principles

- One Rust core implements everything that is not UI or OS glue: protocol,
  crypto, sessions, media processing, sound activation, mixing, trust
  stores, reconnect policy and state.
- UIs are thin: they render state published by the core and send intents.
- Platform glue (audio devices, camera, mDNS, key storage, background
  execution) sits behind traits; the core never calls an OS API directly.
- The core is runtime-light: protocol and media logic are synchronous and
  pure (easy to test and to run in wasm); I/O lives in small async adapters.

## 2. Workspace

```
crates/
  lull-proto      wire format: preface, frames, records, CBOR control messages, invitations, endpoints
  lull-crypto     device keys, device IDs, Noise handshakes (all modes), code PSK, verification number
  lull-media      capture chain (resample, AGC, level), codecs (ADPCM, Opus), VOX gate, jitter buffer, mixer, JPEG helpers
  lull-session    sans-I/O session state machines for station and listener: feed bytes in, get bytes and events out
  lull-core       station and listener engines: trust stores, pairing mode, multi-session scheduling, reconnect, alerts, settings
  lull-net        async transports: TCP, WebSocket client and server, HTTP(S) server for the web app, mDNS
  lull-platform   traits for audio I/O, camera, key storage, power, notifications, plus desktop implementations
  lull-ffi        UniFFI bindings exposing lull-core to Kotlin and Swift
apps/
  desktop         Dioxus desktop app (Linux, macOS, Windows)
  web             Dioxus web listener (wasm32), served by stations
  android         Kotlin + Jetpack Compose app on lull-ffi
  ios             SwiftUI app on lull-ffi
tools/
  lull-cli        headless station / listener / probe for testing and servers
  conformance     independent checks of test vectors (see test-vectors.md)
```

## 3. Layering

```
UI (Dioxus / Compose / SwiftUI)
   │  state snapshots + intents
lull-core (engines)  ── lull-platform traits ── OS adapters
   │
lull-session (sans-I/O) ── lull-proto, lull-crypto, lull-media
   │
lull-net (tokio on native, browser APIs on wasm)
```

`lull-session` consumes and produces bytes and typed events; it owns no
sockets, timers or threads (time is passed in). This makes every protocol
path testable deterministically, including loss, reordering and timeouts.

## 4. Key dependencies

| Need | Crate | Why |
|------|-------|-----|
| Noise | `snow` | Mature Noise implementation with pure-Rust backends that build for wasm |
| Argon2id | `argon2` (RustCrypto) | Pure Rust |
| HKDF, SHA-256 | `hkdf`, `sha2` | Pure Rust |
| CBOR | `minicbor` | Small, no-std capable, derive support |
| Opus | `opus` / `audiopus` (libopus) | Native only; browser uses WebCodecs |
| JPEG | `jpeg-encoder`, `zune-jpeg` | Pure Rust encode/decode |
| Async | `tokio` (native), `wasm-bindgen-futures` (web) | |
| mDNS | `mdns-sd` | Pure Rust, all desktop OSes |
| Audio I/O (desktop) | `cpal` | ALSA/PulseAudio/PipeWire, CoreAudio, WASAPI |
| Camera (desktop) | `nokhwa` | V4L2, AVFoundation, Media Foundation |
| UI | `dioxus` | Desktop and web from one codebase |
| Bindings | `uniffi` | Kotlin and Swift from one interface definition |

## 5. State model

The core publishes immutable snapshots (`StationState`, `ListenerState`)
through a subscription; UIs re-render on change. Intents are plain method
calls (`start_station`, `open_pairing`, `follow(station_id)`,
`set_listen_mode(...)`, `request_video(...)`, `talk(true)`, ...). High-rate
data (levels, video frames) is delivered through dedicated channels at
bounded rates (levels ≤ 10 Hz, video as decoded RGBA or JPEG bytes).
