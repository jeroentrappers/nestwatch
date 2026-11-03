#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-or-later
# Builds Nestwatch.app and an unsigned, compressed .dmg from prebuilt
# binaries with the tools that ship with macOS (lipo, sips, iconutil,
# codesign, hdiutil). Run on macOS.
#
#   packaging/macos/make-dmg.sh VERSION BIN_DIR WEB_DIR OUT.dmg
#
# BIN_DIR holds nestwatch-desktop and nestwatch-station (for example a
# universal build made with lipo). The browser listener bundle goes to
# Contents/share/nestwatch/web, which the app finds as
# <exe dir>/../share/nestwatch/web (OPEN-QUESTIONS UI-10).
#
# The app is only ad-hoc signed (no Apple Developer ID, not notarized), so
# Gatekeeper asks the user to confirm on first launch (docs/releasing.md).
set -eu
[ $# -eq 4 ] || { sed -n '3,15p' "$0" >&2; exit 2; }
VERSION=$1 BIN=$2 WEB=$3 OUT=$4
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
APP="$WORK/dmg/Nestwatch.app"
# CFBundleVersion must be numeric: 0.2.0-rc.1 -> 0.2.0
NUMERIC=$(printf %s "$VERSION" | sed 's/[^0-9.].*$//')

mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources" "$APP/Contents/share/nestwatch"
install -m 0755 "$BIN/nestwatch-desktop" "$BIN/nestwatch-station" "$APP/Contents/MacOS/"
cp -R "$WEB" "$APP/Contents/share/nestwatch/web"

ICONSET="$WORK/Nestwatch.iconset"
mkdir -p "$ICONSET"
SRC="$ROOT/packaging/icons/nestwatch-1024.png"
for s in 16 32 128 256 512; do
  sips -z $s $s "$SRC" --out "$ICONSET/icon_${s}x${s}.png" >/dev/null
  d=$((s * 2))
  sips -z $d $d "$SRC" --out "$ICONSET/icon_${s}x${s}@2x.png" >/dev/null
done
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/Nestwatch.icns"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Nestwatch</string>
  <key>CFBundleDisplayName</key><string>Nestwatch</string>
  <key>CFBundleIdentifier</key><string>org.nestwatch.Nestwatch</string>
  <key>CFBundleExecutable</key><string>nestwatch-desktop</string>
  <key>CFBundleIconFile</key><string>Nestwatch</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$NUMERIC</string>
  <key>CFBundleVersion</key><string>$NUMERIC</string>
  <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
  <key>LSMinimumSystemVersion</key><string>12.0</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.utilities</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSHumanReadableCopyright</key><string>GNU AGPL v3.0 or later</string>
  <key>NSMicrophoneUsageDescription</key><string>The station listens to the room, and talk-back uses the microphone while you hold to talk.</string>
  <key>NSCameraUsageDescription</key><string>The station sends camera images only while a paired listener has the camera view open.</string>
  <key>NSLocalNetworkUsageDescription</key><string>Nestwatch connects stations and listeners directly on your local network.</string>
  <key>NSBonjourServices</key><array><string>_nestwatch._tcp</string></array>
</dict>
</plist>
PLIST
plutil -lint "$APP/Contents/Info.plist"

# Ad-hoc signature: Apple silicon refuses to run unsigned arm64 code.
codesign --force --sign - "$APP/Contents/MacOS/nestwatch-station"
codesign --force --sign - "$APP" || {
  echo "bundle signature failed; signing the main executable only" >&2
  codesign --force --sign - "$APP/Contents/MacOS/nestwatch-desktop"
}
ln -s /Applications "$WORK/dmg/Applications"
rm -f "$OUT"
hdiutil create -volname "Nestwatch $VERSION" -srcfolder "$WORK/dmg" -fs HFS+ -format UDZO -ov "$OUT"
echo "built $OUT"
