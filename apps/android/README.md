# Lull for Android

Kotlin + Jetpack Compose app (minSdk 23, target and compile SDK 36) on the
Rust core through `crates/lull-ffi` (UniFFI, JNA). Station and listener,
the screens of `design/README.md` §5 with the Lull tokens mapped to
Material 3 (§6).

| Concern | Where |
|---------|-------|
| Engines | `core/Lull.kt` owns the `Station` and `Listener` objects and republishes snapshots, events and video frames as flows |
| Storage (device key, trust, settings) | `core/KeystoreStorage.kt`: AES-256-GCM under an Android Keystore key, files in `noBackupFilesDir`, backups off (ANDROID-2) |
| Station | `service/StationService.kt`: foreground service (microphone, camera if granted; plain on API 23–29), wake and Wi-Fi locks, `AudioRecord` → `push_capture_i16`, talk-back `AudioTrack`, CameraX on `CameraWanted` only, battery, NSD advertising |
| Listener | `service/ListenerService.kt`: foreground service (mediaPlayback on API 29+), `pull_playback` → `AudioTrack`, talk-back microphone, NSD browsing under a multicast lock, alert notifications |
| mDNS | Android NSD, not the core's responder (ANDROID-1) |
| QR | shown from the core's `qr_code`; scanned with CameraX + ZXing core (no Google Play services) |
| Permissions | microphone and camera when the station starts or talk-back / scanning is first used, notifications on API 33+, each with an explanation first |

## Build

Needs the Android SDK (platform 36, build-tools), NDK 30.0.16248370,
JDK 17+, Rust with the Android targets and `cargo-ndk`:

```sh
rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android
cargo install --locked cargo-ndk
sdkmanager "ndk;30.0.16248370"
cd apps/android
echo "sdk.dir=$HOME/Android/Sdk" > local.properties
./gradlew assembleDebug testDebugUnitTest
```

The `cargoNdkBuild` task builds `liblull_ffi.so` (release profile) for
`lull.abis` (`gradle.properties`; `-Plull.abis=x86_64` for a quick emulator
build), and `uniffiBindings` generates the Kotlin bindings from it into
`app/build/generated/uniffi`.

## Try it against the command-line tools

The emulator reaches the host at `10.0.2.2`:

```sh
cargo run --release -p lull-cli -- --data /tmp/st station --pair --tone \
  --no-mdns --no-web --port 47210 --endpoint 10.0.2.2:47210
adb shell "am start -a android.intent.action.VIEW -d '<invite>' org.lull.android"
```

For a station in the emulator, forward its port and pair by code:

```sh
adb forward tcp:47290 tcp:47100
cargo run --release -p lull-cli -- --data /tmp/ls listen --code <code> \
  --station-id <id> --route 127.0.0.1:47290 --no-mdns
```
