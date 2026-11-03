# nestwatch-desktop

The Dioxus desktop app for Linux, macOS and Windows (platforms.md §2.4). It
runs the listener engine always and the station engine when the station
role is active, both on `nestwatch-platform`'s `DesktopPlatform`, and renders the
shared `nestwatch-ui` screens in a webview.

| Concern | Where |
|---------|-------|
| Engine storage (device key, trust, settings) | OS key store via `KeyStoreStorage` (`NESTWATCH_DATA_DIR=<dir>` uses a plain directory) |
| Station audio | microphone → `push_capture`, `pull_talkback` → speaker |
| Station camera | started on `CameraWanted`, stopped on `CameraNotWanted`; permission/absence reported as camera availability |
| Station power | system sleep inhibited while the station runs (PLATFORM-4) |
| Station battery | read every 30 s, sent in `status` |
| Browser listener | served from the first of `$NESTWATCH_WEB_DIR`, `<exe dir>/web`, `<exe dir>/../share/nestwatch/web`, `target/nestwatch-web` (built by `apps/web/build.sh`); a placeholder page otherwise |
| Listener audio | `pull_playback` → speaker, plus a two-tone alert sound |
| Talk-back | microphone on `MicrophoneWanted` → `push_talk_capture` |
| Alerts | system notifications (`Notifier`), in-app notices |
| Tray | icon with "Show Nestwatch" / "Quit Nestwatch"; tooltip lists the state; closing the window hides it and keeps monitoring |

## Build and run

Linux needs the WebKitGTK stack and OpenSSL headers (Debian:
`libwebkit2gtk-4.1-dev libgtk-3-dev libsoup-3.0-dev libxdo-dev libssl-dev
libayatana-appindicator3-dev`), and `pulseaudio-utils` at run time for audio.

```sh
apps/web/build.sh                       # browser bundle the station serves
cargo run -p nestwatch-desktop --release
```

Environment for tests and several instances on one machine:
`NESTWATCH_DATA_DIR`, `NESTWATCH_STATION_NAME`, `NESTWATCH_TCP_PORT`, `NESTWATCH_HTTP_PORT`,
`NESTWATCH_HTTPS_PORT`, `NESTWATCH_LOOPBACK=1`, `NESTWATCH_NO_MDNS=1`, `NESTWATCH_WEB_DIR`.

Without the webview libraries, the host library still builds and tests:

```sh
cargo test -p nestwatch-desktop --no-default-features
```
