# Nestwatch

A private, local-first baby monitor for every device you own.

- **Station and listener on any device**: Linux, macOS, Windows, Android,
  iPhone/iPad, plus a browser listener served by any station.
- **Several listeners per station, several stations per listener**, mixed
  into one calm view.
- **Sound activation**: stations only send audio while the room is louder
  than the threshold each listener chooses, with pre-roll so the start of a
  cry is never cut off.
- **Talk-back** and an **on-demand camera** that is off whenever nobody looks.
- **No cloud, no account**: devices talk directly on your home network or
  over your own VPN (Tailscale, WireGuard). End-to-end encrypted with the
  Noise Protocol Framework.

Nestwatch is not a medical device and does not replace supervision.

## Status

Specification phase. Start with [specs/](specs/README.md).

| Step | State |
|------|-------|
| Specifications | first version |
| Design system | done (`design/`) |
| Rust core | planned |
| Desktop apps (Dioxus) and browser listener (wasm) | first version (`apps/`) |
| UniFFI bindings for Kotlin and Swift (`crates/nestwatch-ffi`) | first version |
| Android app (Compose) | first version (`apps/android`) |
| Rendezvous and relay client (`crates/nestwatch-relay`) and server (`tools/nestwatch-server`) | first version; used by the engines, the CLI, the desktop, browser and Android apps |
| iOS app (SwiftUI) on the Rust core | planned |

## License

GNU Affero General Public License v3.0 or later, see [LICENSE](LICENSE).

The AGPL's terms conflict with Apple App Store distribution rules. Before an
iOS release, the copyright holder intends to grant an explicit additional
permission (AGPL section 7) for app-store distribution, or to distribute iOS
builds through other channels.
