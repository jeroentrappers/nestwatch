# nestwatch-web: the browser app

The listener app that every station serves on its HTTP(S) port
(protocol.md §3.3, platforms.md §2.5), and a station reachable through a
relay server only (product.md §3.10, rendezvous.md FR-RV-7). It runs the
`nestwatch-core` listener and station engines compiled to wasm32 (`web`
feature) and the shared Dioxus components of `nestwatch-ui`.

## Build

```sh
rustup target add wasm32-unknown-unknown
cargo install --locked wasm-bindgen-cli --version 0.2.130   # = wasm-bindgen in Cargo.lock
apps/web/build.sh                  # release bundle in target/nestwatch-web/
apps/web/build.sh /some/dir        # elsewhere
apps/web/build.sh --demo DIR       # demo data instead of the engine (design previews)
```

The bundle is self-contained (PRIV-1: no CDN, no fonts): `index.html`,
`boot.js`, `nestwatch-web.js`, `nestwatch-web_bg.wasm`, `snippets/`,
`audio-worklet.js`, `nestwatch.css` (verbatim from `design/`), `app.css`,
`logo.svg`. Everything loads from `'self'`, so it works under the station's
CSP (NET-5): no inline scripts or styles; CSS variables are set through
`style.setProperty`.

## Serve

```sh
nestwatch station --pair --web-dir target/nestwatch-web          # CLI station
nestwatch-desktop                                            # looks in target/nestwatch-web, see apps/desktop
```

Open the printed `web:` address. The invitation in the URL fragment pairs
right away and is then removed from the address bar and history.

The relay server (`nestwatch-server --app-dir target/nestwatch-web`) serves
the same bundle at `/app/` (rendezvous.md FR-RV-5). Served from `/app/`, the
app is in **hosted mode**: it reaches stations only through that origin's
`/nwr/1` (stations whose invitation names this server in `r=`), never
through a station WebSocket, and pairs from `<origin>/app/#<invitation>` (the
`relay web:` address a station prints). Over HTTPS (or on `localhost` /
`127.0.0.1`) the page is a secure context, so talk-back works.

## Platform notes

- **Storage**: device key, known stations and settings live in the origin's
  `localStorage` (`nestwatch:<key>`, base64). See OPEN-QUESTIONS UI-4.
- **Audio**: a 16 kHz `AudioContext` created on the first tap (a "Tap to
  start listening" prompt is shown until then). An `AudioWorklet` requests
  20 ms frames whenever less than 100 ms are buffered; the main thread pulls
  them with `pull_playback`. Without `AudioWorklet` a `ScriptProcessorNode`
  pulls directly. `window.__nestwatchAudio` exposes counters (`pulled`,
  `nonSilent`, `played`, `underruns`, `micBlocks`).
- **Talk-back**: only in a secure context (HTTPS or `localhost`):
  `getUserMedia` at the device rate into `push_talk_capture`.
- **Alerts**: an in-page notice and a short sound always; the Notification
  API too when the page is a secure context and permission was granted.

## Browser station

"Use this device as a station" (welcome screen, or the listener home) runs
the station engine in the page, relay-only: it registers its mailbox at the
relay server over `/nwr/1`, accepts each listener's relay pipe and serves
NWP/1 over it. There are no direct endpoints, no mDNS and no pairing code
(OPEN-QUESTIONS WEBST-1); invitations carry `r`/`m` only.

- **Server**: under `/app/` the page's own origin (fixed); elsewhere the
  stored setting, default `https://nestwatch.appmire.be` (a page served by a
  station cannot reach it: its CSP allows only the station, WEBST-8).
- **Needs** a secure context (HTTPS, `localhost`, `127.0.0.1`) for the
  microphone; the server's `/app/` Permissions-Policy allows microphone and
  camera for the page itself.
- **Microphone**: `getUserMedia` without echo cancellation, noise suppression
  or gain control, at the device rate into `push_capture_f32`.
- **Talk-back**: a 16 kHz `AudioContext` pulls `pull_talkback` through the
  playback worklet. Both contexts start on a user gesture; after a reload
  the station screen asks for a tap.
- **Camera**: only while a listener watches: `getUserMedia` video drawn into
  a canvas at the requested size and rate; the engine JPEG-encodes per
  listener (WEBST-4). While the page is hidden the camera is reported
  unavailable.
- **Battery** (Battery Status API), **Screen Wake Lock** (again after every
  return to the visible page), a `beforeunload` confirmation, and a request
  for persistent storage.
- Keys, trust list, settings and the mailbox live in `localStorage` like the
  listener's (security.md §7.1).
- `window.__nestwatchStation` exposes counters (`micBlocks`, `micPeak`,
  `talkPulled`, `talkVoiced`, `cameraFrames`, `wakeLock`, `battery`, ...).

The CLI listener has test switches for this: `nestwatch listen --invite ...
--continuous --video --talk-tone` opens the camera view and talks for 10 s.
