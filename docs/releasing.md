# Releasing Nestwatch

Releases are built by [.github/workflows/release.yml](../.github/workflows/release.yml)
when a tag `v<version>` is pushed. It builds every download, attaches them to
a **draft** GitHub Release with a `SHA256SUMS` file, and leaves publishing to
a maintainer.

## 1. Cut a release

1. Make sure `main` is green in CI ([ci.yml](../.github/workflows/ci.yml)).
2. Set the version everywhere it is written down:
   - `Cargo.toml` → `[workspace.package] version` (desktop, CLI, core).
   - `apps/android/app/build.gradle.kts` → `versionName`, and raise
     `versionCode` by one (Android refuses updates with a lower or equal code).
   - Run `cargo update --workspace` so `Cargo.lock` follows.
3. Commit (`Release 0.2.0`), then tag and push:

   ```sh
   git tag -a v0.2.0 -m "Nestwatch 0.2.0"
   git push origin main v0.2.0
   ```

   A tag with a suffix (`v0.2.0-rc.1`) becomes a pre-release. The workflow
   warns when the tag and `Cargo.toml` disagree.
4. Wait for the **Release** workflow. It ends with a draft release named
   `Nestwatch 0.2.0` containing the files below and generated notes.
5. Download a few files, check them against `SHA256SUMS`, try the installers,
   edit the notes, and **Publish**. Only published releases are served by
   `releases/latest/download/…`, which the website's download buttons use.

To rebuild an existing tag (for example after fixing the workflow), run the
workflow manually (**Actions → Release → Run workflow**) with the tag name;
files of an existing release are replaced.

## 2. What gets built

| Job | Runner | Output |
|-----|--------|--------|
| `web` | ubuntu-24.04 | `nestwatch-web.tar.gz`: the browser listener (`apps/web/build.sh`) |
| `site` | ubuntu-24.04 | `nestwatch-site.tar.gz`: the website (`apps/site/build.sh`) |
| `linux` | ubuntu-22.04 (x86_64), ubuntu-22.04-arm (aarch64) | `nestwatch-linux-amd64.deb`, `nestwatch-linux-arm64.deb`, `nestwatch-linux-x86_64.rpm`, `nestwatch-linux-aarch64.rpm`, `nestwatch-linux-x86_64.tar.gz`, `nestwatch-linux-aarch64.tar.gz` |
| `macos` | macos-15 | `nestwatch-macos-universal.dmg` (Apple silicon + Intel, macOS 12+) |
| `windows` | windows-2025 | `nestwatch-windows-x86_64.msi` |
| `android` | ubuntu-24.04 | `nestwatch-android.apk` (signed with the project key) or `nestwatch-android-testkey.apk` (see §4) |
| `server` | ubuntu-22.04 | `nestwatch-server-linux-x86_64`, only once `tools/nestwatch-server` exists in the tagged tree |
| `release` | ubuntu-24.04 | `SHA256SUMS` and the draft release |

Asset names carry no version on purpose: the website links to
`https://github.com/jeroentrappers/nestwatch/releases/latest/download/<name>`
(`ASSETS` in [apps/site/build.py](../apps/site/build.py)). Rename an asset
in both places.

Every desktop package contains `nestwatch-desktop`, `nestwatch-station`
(headless station) and the browser listener bundle at the path the station
serves it from (OPEN-QUESTIONS UI-10): `<exe dir>/../share/nestwatch/web` on
Linux and macOS, `<exe dir>\web` on Windows.

## 3. Packaging choices

The desktop app is a plain Cargo binary using Dioxus' desktop renderer, so
packaging needs no Dioxus CLI (`dx bundle`). Packages are made from the
release binaries with each platform's own tools, in scripts that also run
locally:

- **Linux** ([packaging/linux/package.sh](../packaging/linux/package.sh)):
  `dpkg-deb` with dependencies computed by `dpkg-shlibdeps` (plus the
  appindicator library, which is loaded at run time, and `pulseaudio-utils`
  as a recommendation); `rpmbuild` with a generated spec whose library
  dependencies `rpmbuild` detects; and a relocatable `.tar.gz`. Building on
  Ubuntu 22.04 keeps the requirement at glibc 2.35 and WebKitGTK 4.1, which
  Debian 12, Ubuntu 22.04 and Fedora 37 or newer provide. (platforms.md
  lists glibc 2.31; no distribution with that glibc ships WebKitGTK 4.1.)

  ```sh
  cargo build --release -p nestwatch-desktop --bins && apps/web/build.sh
  packaging/linux/package.sh 0.2.0 x86_64 target/release target/nestwatch-web dist
  ```

- **macOS** ([packaging/macos/make-dmg.sh](../packaging/macos/make-dmg.sh)):
  both architectures built with `MACOSX_DEPLOYMENT_TARGET=12.0`, merged with
  `lipo`, wrapped in `Nestwatch.app` (Info.plist with the microphone, camera
  and local-network usage strings and `_nestwatch._tcp`), icons from
  `packaging/icons/nestwatch-1024.png` via `sips`/`iconutil`, ad-hoc signed
  with `codesign -s -`, and packed with `hdiutil` (UDZO).

- **Windows** ([packaging/windows/make-wxs.py](../packaging/windows/make-wxs.py)):
  generates a WiX source listing every staged file, built with the WiX .NET
  tool (`dotnet tool install --global wix --version 5.0.2`, then
  `wix build -arch x64`). Per-machine install to `C:\Program Files\Nestwatch`
  with a Start menu shortcut; the fixed `UpgradeCode` makes newer versions
  replace older ones. MSI versions are numeric, so a pre-release `0.2.0-rc.1`
  installs as `0.2.0`.

- **Android**: `./gradlew assembleRelease` (R8 minified, all three ABIs),
  then `zipalign` and `apksigner` from the SDK build-tools.

## 4. Signing

### macOS (not signed yet)

The `.dmg` is only ad-hoc signed and not notarized, so Gatekeeper blocks the
first launch. Users:

1. Open the `.dmg` and drag **Nestwatch** to **Applications**.
2. Open Nestwatch once; macOS says it cannot verify the developer.
3. **System Settings → Privacy & Security**, scroll to Security, click
   **Open Anyway**, confirm with your password. (macOS 14 and older:
   Control-click the app → **Open** also works.)
4. Alternatively, in Terminal: `xattr -dr com.apple.quarantine /Applications/Nestwatch.app`.

Signing with a Developer ID and notarizing (`codesign --options runtime`,
`xcrun notarytool submit --wait`, `xcrun stapler staple`) can be added to the
`macos` job once the project has an Apple Developer account; the browser
bundle should then move to `Contents/Resources` with a matching lookup in the
app.

### Windows (not signed yet)

SmartScreen warns about the unsigned installer: **More info → Run anyway**.
Code signing (`signtool sign /fd SHA256 /tr <timestamp-url> /td SHA256`) can
be added after `wix build` when a certificate is available.

### Android

Configure these repository secrets (**Settings → Secrets and variables →
Actions**) to sign with the project's release key:

| Secret | Value |
|--------|-------|
| `ANDROID_KEYSTORE_BASE64` | `base64 -w0 release.jks` |
| `ANDROID_KEYSTORE_PASSWORD` | keystore password |
| `ANDROID_KEY_ALIAS` | key alias |
| `ANDROID_KEY_PASSWORD` | key password (optional; defaults to the keystore password) |

Create the key once and keep it safe; every later update must use the same
key:

```sh
keytool -genkeypair -keystore release.jks -alias nestwatch -keyalg RSA -keysize 4096 -validity 10000
```

Without the secrets the workflow signs the APK with a key generated for that
run only and names it `nestwatch-android-testkey.apk`. It installs and runs,
but cannot be updated by a later build: users must uninstall it first (which
deletes its pairings). The website's download button points at
`nestwatch-android.apk`, so configure the secrets before publishing a release
meant for users.

## 5. After publishing

- Deploy `nestwatch-site.tar.gz` to `https://nestwatch.appmire.be/` (and
  `nestwatch-web.tar.gz` to `/app/` once the relay is live), see
  [self-hosting.md](self-hosting.md) §2 for the layout.
- Check the download buttons on `/download/` resolve.
