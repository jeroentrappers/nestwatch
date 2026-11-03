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
  nestwatch-proto      wire format: preface, frames, records, CBOR control messages, invitations, endpoints
  nestwatch-crypto     device keys, device IDs, Noise handshakes (all modes), code PSK, verification number
  nestwatch-media      capture chain (resample, AGC, level), codecs (ADPCM, Opus), VOX gate, jitter buffer, mixer, JPEG helpers
  nestwatch-session    sans-I/O session state machines for station and listener: feed bytes in, get bytes and events out
  nestwatch-core       station and listener engines: trust stores, pairing mode, multi-session scheduling, reconnect, alerts, settings
  nestwatch-net        async transports: TCP, WebSocket client and server, HTTP(S) server for the web app, mDNS
  nestwatch-platform   traits for audio I/O, camera, key storage, power, notifications, plus desktop implementations
  nestwatch-ffi        UniFFI bindings exposing nestwatch-core to Kotlin and Swift
  nestwatch-relay      rendezvous client and server logic (NWR/1), STUN binding, QUIC direct transport
apps/
  desktop         Dioxus desktop app (Linux, macOS, Windows)
  web             Dioxus web listener (wasm32), served by stations
  android         Kotlin + Jetpack Compose app on nestwatch-ffi
  ios             SwiftUI app on nestwatch-ffi
tools/
  nestwatch-cli        headless station / listener / probe for testing and servers
  nestwatch-server     rendezvous/relay server binary; also serves the landing page, web app and downloads
  conformance     independent checks of test vectors (see test-vectors.md)
```

## 3. Layering

```
UI (Dioxus / Compose / SwiftUI)
   │  state snapshots + intents
nestwatch-core (engines)  ── nestwatch-platform traits ── OS adapters
   │
nestwatch-session (sans-I/O) ── nestwatch-proto, nestwatch-crypto, nestwatch-media
   │
nestwatch-net (tokio on native, browser APIs on wasm)
```

`nestwatch-session` consumes and produces bytes and typed events; it owns no
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
