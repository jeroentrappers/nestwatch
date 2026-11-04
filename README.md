# Nestwatch

A private, local-first baby monitor for every device you own.

- **Station and listener on any device**: Linux, macOS, Windows, Android,
  iPhone/iPad, plus the browser: a listener served by any station or relay
  server, and a station through the relay.
- **Several listeners per station, several stations per listener**, mixed
  into one calm view.
- **Sound activation**: stations only send audio while the room is louder
  than the threshold each listener chooses, with pre-roll so the start of a
  cry is never cut off.
- **Talk-back** and an **on-demand camera** that is off whenever nobody looks.
- **No cloud, no account**: devices talk directly on your home network or
  over your own VPN (Tailscale, WireGuard). Away from home without a VPN, an
  optional relay server passes on the encrypted stream: the project's at
  `nestwatch.appmire.be`, or [your own](docs/self-hosting.md) with Docker
  Compose or a single binary. End-to-end encrypted with the Noise Protocol
  Framework either way.

Nestwatch is not a medical device and does not replace supervision.

## Status

First release, 0.1.0: apps on
[GitHub Releases](https://github.com/jeroentrappers/nestwatch/releases), the
web app at <https://nestwatch.appmire.be/app/>, the relay server as a binary
and as the container image `ghcr.io/jeroentrappers/nestwatch-server`.
Behaviour is defined in [specs/](specs/README.md).

| Step | State |
|------|-------|
| Specifications | first version |
| Design system | done (`design/`) |
| Rust core (`crates/`) | first version |
| Desktop apps (Dioxus) and browser listener (wasm) | first version (`apps/`) |
| UniFFI bindings for Kotlin and Swift (`crates/nestwatch-ffi`) | first version |
| Android app (Compose) | first version (`apps/android`) |
| Rendezvous and relay client (`crates/nestwatch-relay`) and server (`tools/nestwatch-server`) | first version; used by every app and the CLI; [self-hosting](docs/self-hosting.md) |
| iOS / iPadOS app (SwiftUI) on the Rust core | first version (`apps/ios`), build from source |

## License

GNU Affero General Public License v3.0 or later, see [LICENSE](LICENSE).

The AGPL's terms conflict with Apple App Store distribution rules. Before an
iOS release, the copyright holder intends to grant an explicit additional
permission (AGPL section 7) for app-store distribution, or to distribute iOS
builds through other channels.
