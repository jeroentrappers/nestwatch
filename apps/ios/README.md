# Nestwatch for iOS and iPadOS

The native iPhone and iPad app: SwiftUI on the Rust core through UniFFI
(`crates/nestwatch-ffi`). One universal app for both roles: a **station**
(the device in the child's room) and a **listener** that follows up to 8
stations. iOS 15 or later (`specs/platforms.md` §1, §2.3).

## Layout

| Path | What |
|------|------|
| `build-rust.sh` | Builds `nestwatch-ffi` for iOS and writes the XCFramework and the Swift bindings |
| `project.yml` | XcodeGen spec the committed `Nestwatch.xcodeproj` is generated from |
| `Nestwatch/App` | App entry, scene phase, `nestwatch://invite` links |
| `Nestwatch/Core` | Engine owner (`Nestwatch`), Keychain storage, preferences, level history, texts |
| `Nestwatch/Platform` | Audio (`AVAudioEngine`), station camera (`AVCaptureSession`), Bonjour, network, battery, alerts, the station and listener hosts |
| `Nestwatch/UI` | Theme tokens (`design/nestwatch.css`), components, screens |
| `Nestwatch/Resources` | `Info.plist`, entitlements, assets, `en.lproj/Localizable.strings` |
| `NestwatchTests` | Unit tests of the app logic (level mapping, history, texts, PCM and pixel conversion, Keychain storage) |
| `NestwatchUITests` | End-to-end flows against a real station or listener; skipped unless configured |
| `scripts/render-icon.swift` | Renders the app icon from the logo geometry |
| `Frameworks/`, `Generated/` | Build outputs of `build-rust.sh` (not committed) |

## Build

Needs macOS with Xcode 15 or later (tested with Xcode 26.1) and rustup.

```sh
apps/ios/build-rust.sh          # adds the Rust targets, builds the core (release)
open apps/ios/Nestwatch.xcodeproj
```

`build-rust.sh` builds `aarch64-apple-ios` (devices), `aarch64-apple-ios-sim`
and `x86_64-apple-ios` (simulators) as static libraries with
`IPHONEOS_DEPLOYMENT_TARGET=15.0`, packages them as
`Frameworks/NestwatchFFI.xcframework` and generates
`Generated/NestwatchFFI.swift` with `crates/nestwatch-ffi/generate-bindings.sh`.
`NESTWATCH_IOS_SIM_X86=0` skips the Intel simulator slice,
`NESTWATCH_IOS_DEVICE=0` the device slice; `build-rust.sh debug` builds
faster, unoptimised libraries. Run it again after changing Rust code.

From the command line:

```sh
cd apps/ios
xcodebuild -project Nestwatch.xcodeproj -scheme Nestwatch \
  -destination 'platform=iOS Simulator,name=iPhone 17' test      # unit tests
xcodebuild -project Nestwatch.xcodeproj -scheme Nestwatch -sdk iphoneos \
  -destination 'generic/platform=iOS' CODE_SIGNING_ALLOWED=NO build  # device build, unsigned
```

The project is generated with [XcodeGen](https://github.com/yonaskolb/XcodeGen)
2.46.0 but committed, so building needs only Xcode. After changing
`project.yml` (for example to add a file outside the existing folders),
regenerate it with `xcodegen generate` in this directory.

## Running on your own iPhone or iPad

No paid developer account is needed for your own devices:

1. In Xcode, Settings → Accounts, add your Apple ID (a free "Personal Team").
2. Select the **Nestwatch** target → Signing & Capabilities, tick
   "Automatically manage signing", pick your team and change the bundle
   identifier to something unique, e.g. `com.example.nestwatch`.
3. Connect the device, enable Developer Mode on it (Settings → Privacy &
   Security), choose it as the run destination and run.
4. On the device, trust the developer profile (Settings → General → VPN &
   Device Management). Free provisioning profiles expire after 7 days;
   run from Xcode again to renew.

If signing with a personal team rejects the time-sensitive notifications
capability, remove `com.apple.developer.usernotifications.time-sensitive`
from `Nestwatch/Resources/Nestwatch.entitlements`; alerts are then ordinary
notifications that a Focus may hold back.

## Platform behaviour

- **Background:** `UIBackgroundModes` `audio`. The station's microphone
  capture and the listener's playback keep the app running with the screen
  locked. The station uses `playAndRecord` without `mixWithOthers` (IOS-3).
- **Camera:** iOS stops the camera when the app leaves the foreground; the
  station then reports the camera as unavailable and resumes when it comes
  back. Use "Dim screen" instead of locking the station if listeners may want
  to look (IOS-4).
- **Keys:** Keychain items, `AfterFirstUnlockThisDeviceOnly`, never in
  backups (IOS-2).
- **Local network:** Bonjour advertising and browsing of `_nestwatch._tcp`
  through `NetService` (IOS-1). iOS asks for local-network access the first
  time.
- **Alerts:** local notifications, time-sensitive for sound and
  unreachable (IOS-5).
- **Relay:** offered once when the station first runs, off by default,
  server `https://nestwatch.appmire.be` (`specs/rendezvous.md` FR-RV-1).

## End-to-end tests

`NestwatchUITests` drives the app against a real counterpart and is skipped
otherwise:

```sh
# The app as listener: start a station, e.g. relay-only with a test tone and camera
nestwatch station --pair --tone --test-camera --relay https://nestwatch.appmire.be --no-endpoints
TEST_RUNNER_NESTWATCH_E2E_INVITE='nestwatch://invite?…' xcodebuild … test \
  -only-testing:NestwatchUITests/PairingFlowTests

# The app as station: the test prints "E2E-STATION code=… id=…"; pair with
#   nestwatch listen --code … --station-id … --route 127.0.0.1:47100
TEST_RUNNER_NESTWATCH_E2E_STATION=1 xcodebuild … test \
  -only-testing:NestwatchUITests/StationFlowTests
```

Debug builds also open a screen named in `NESTWATCH_DEBUG_SCREEN`
(`about`, `stationSettings`, `addStation`, `pairing`, `dim`,
`stationPrefs`, `camera`) for screenshots, e.g.
`SIMCTL_CHILD_NESTWATCH_DEBUG_SCREEN=pairing xcrun simctl launch …`.

## Not tested yet

Only simulators were used so far. Untested: real devices (signing, the
microphone and camera of an iPhone or iPad, the background lifetime with
the screen locked for hours, Bluetooth routes), the QR scanner (simulators
have no camera), local-network permission prompts, Focus and time-sensitive
delivery, and audio interruptions by calls.
