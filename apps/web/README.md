# nestwatch-web: the browser listener

The listener app that every station serves on its HTTP(S) port
(protocol.md §3.3, platforms.md §2.5). Listener only; it runs the
`nestwatch-core` listener engine compiled to wasm32 (`web` feature) and the
shared Dioxus components of `nestwatch-ui`.

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
