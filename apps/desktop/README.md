# lull-desktop

The Dioxus desktop app for Linux, macOS and Windows (platforms.md §2.4). It
runs the listener engine always and the station engine when the station
role is active, both on `lull-platform`'s `DesktopPlatform`, and renders the
shared `lull-ui` screens in a webview.

| Concern | Where |
|---------|-------|
| Engine storage (device key, trust, settings) | OS key store via `KeyStoreStorage` (`LULL_DATA_DIR=<dir>` uses a plain directory) |
| Station audio | microphone → `push_capture`, `pull_talkback` → speaker |
| Station camera | started on `CameraWanted`, stopped on `CameraNotWanted`; permission/absence reported as camera availability |
| Station power | system sleep inhibited while the station runs (PLATFORM-4) |
| Station battery | read every 30 s, sent in `status` |
| Browser listener | served from the first of `$LULL_WEB_DIR`, `<exe dir>/web`, `<exe dir>/../share/lull/web`, `target/lull-web` (built by `apps/web/build.sh`); a placeholder page otherwise |
| Listener audio | `pull_playback` → speaker, plus a two-tone alert sound |
| Talk-back | microphone on `MicrophoneWanted` → `push_talk_capture` |
| Alerts | system notifications (`Notifier`), in-app notices |
| Tray | icon with "Show Lull" / "Quit Lull"; tooltip lists the state; closing the window hides it and keeps monitoring |

## Build and run

Linux needs the WebKitGTK stack and OpenSSL headers (Debian:
`libwebkit2gtk-4.1-dev libgtk-3-dev libsoup-3.0-dev libxdo-dev libssl-dev
libayatana-appindicator3-dev`), and `pulseaudio-utils` at run time for audio.

```sh
apps/web/build.sh                       # browser bundle the station serves
cargo run -p lull-desktop --release
```

Environment for tests and several instances on one machine:
`LULL_DATA_DIR`, `LULL_STATION_NAME`, `LULL_TCP_PORT`, `LULL_HTTP_PORT`,
`LULL_HTTPS_PORT`, `LULL_LOOPBACK=1`, `LULL_NO_MDNS=1`, `LULL_WEB_DIR`.

Without the webview libraries, the host library still builds and tests:

```sh
cargo test -p lull-desktop --no-default-features
```
