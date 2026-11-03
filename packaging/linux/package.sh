#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-or-later
# Packages prebuilt Nestwatch desktop binaries for Linux as .deb, .rpm and
# .tar.gz, with plain dpkg-deb and rpmbuild (no packaging plugins).
#
#   packaging/linux/package.sh VERSION ARCH BIN_DIR WEB_DIR OUT_DIR [deb] [rpm] [tar]
#
# ARCH is x86_64 or aarch64. BIN_DIR holds nestwatch-desktop and
# nestwatch-station (cargo's target/<triple>/release), WEB_DIR the browser
# listener bundle from apps/web/build.sh. Without format arguments all three
# are built. Layout (the desktop app finds the web bundle at
# <exe dir>/../share/nestwatch/web, OPEN-QUESTIONS UI-10):
#
#   bin/nestwatch-desktop, bin/nestwatch-station
#   share/nestwatch/web/...
#   share/applications/org.nestwatch.Nestwatch.desktop
#   share/icons/hicolor/{scalable,512x512,256x256,128x128}/apps/org.nestwatch.Nestwatch.*
#
# The .deb and .rpm install under /usr; the .tar.gz unpacks to
# nestwatch-linux-ARCH/ with the same tree.
#
# Needs: dpkg-deb and dpkg-shlibdeps (dpkg-dev) for .deb, rpmbuild (rpm) for
# .rpm, tar and gzip.
set -eu
[ $# -ge 5 ] || { sed -n '3,22p' "$0" >&2; exit 2; }
VERSION=$1 ARCH=$2 BIN=$3 WEB=$4 OUT=$5
shift 5
FORMATS=${*:-deb rpm tar}
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
ID=org.nestwatch.Nestwatch
case "$ARCH" in
  x86_64) DEB_ARCH=amd64 ;;
  aarch64) DEB_ARCH=arm64 ;;
  *) echo "unknown ARCH $ARCH" >&2; exit 2 ;;
esac
for f in nestwatch-desktop nestwatch-station; do
  [ -x "$BIN/$f" ] || { echo "missing $BIN/$f" >&2; exit 1; }
done
[ -f "$WEB/index.html" ] || { echo "missing $WEB/index.html (run apps/web/build.sh)" >&2; exit 1; }
mkdir -p "$OUT"
OUT=$(cd "$OUT" && pwd)
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

# stage PREFIX: the installed tree below PREFIX.
stage() {
  p=$1
  install -d "$p/bin" "$p/share/nestwatch" "$p/share/applications" "$p/share/doc/nestwatch"
  install -m 0755 "$BIN/nestwatch-desktop" "$BIN/nestwatch-station" "$p/bin/"
  cp -R "$WEB" "$p/share/nestwatch/web"
  chmod -R u=rwX,go=rX "$p/share/nestwatch/web"
  install -m 0644 "$HERE/$ID.desktop" "$p/share/applications/$ID.desktop"
  install -Dm 0644 "$ROOT/design/logo.svg" "$p/share/icons/hicolor/scalable/apps/$ID.svg"
  for s in 128 256 512; do
    install -Dm 0644 "$ROOT/packaging/icons/nestwatch-$s.png" "$p/share/icons/hicolor/${s}x${s}/apps/$ID.png"
  done
  install -m 0644 "$ROOT/LICENSE" "$p/share/doc/nestwatch/copyright"
  install -m 0644 "$ROOT/README.md" "$p/share/doc/nestwatch/README.md"
}

SUMMARY="Private, local-first baby monitor"
DESCRIPTION="Nestwatch turns computers, phones and tablets into a baby monitor.
Stations capture sound (and video on request); listeners follow one or more
stations with sound activation, talk-back and alerts. Devices talk directly on
the home network or over your own VPN, end-to-end encrypted. No account, no
cloud. Not a medical device; it does not replace supervision."

for fmt in $FORMATS; do
  case "$fmt" in
  deb)
    # Debian versions must not contain '-' without a revision: 0.2.0-rc.1 -> 0.2.0~rc.1
    v=$(printf %s "$VERSION" | sed 's/-/~/g')
    d="$WORK/deb"
    stage "$d/usr"
    mkdir -p "$d/DEBIAN" "$WORK/shlibs/debian"
    # dpkg-shlibdeps needs a debian/control to run from.
    printf 'Source: nestwatch\n\nPackage: nestwatch\nArchitecture: any\n' > "$WORK/shlibs/debian/control"
    deps=$(cd "$WORK/shlibs" && dpkg-shlibdeps -O "$d/usr/bin/nestwatch-desktop" "$d/usr/bin/nestwatch-station" 2>/dev/null \
      | sed -n 's/^shlibs:Depends=//p')
    [ -n "$deps" ] || deps="libwebkit2gtk-4.1-0, libgtk-3-0, libxdo3, libssl3"
    size=$(du -sk "$d/usr" | cut -f1)
    {
      echo "Package: nestwatch"
      echo "Version: $v"
      echo "Architecture: $DEB_ARCH"
      echo "Maintainer: Jeroen Trappers <jeroen.trappers@gmail.com>"
      echo "Installed-Size: $size"
      echo "Depends: $deps, libayatana-appindicator3-1 | libappindicator3-1"
      echo "Recommends: pulseaudio-utils"
      echo "Section: video"
      echo "Priority: optional"
      echo "Homepage: https://nestwatch.appmire.be/"
      echo "Description: $SUMMARY"
      printf '%s\n' "$DESCRIPTION" | sed 's/^/ /'
    } > "$d/DEBIAN/control"
    dpkg-deb --root-owner-group -Zxz --build "$d" "$OUT/nestwatch-linux-$DEB_ARCH.deb"
    ;;
  rpm)
    v=$(printf %s "$VERSION" | sed 's/-/~/g')
    r="$WORK/rpm"
    mkdir -p "$r/stage" "$r/top"
    stage "$r/stage/usr"
    cat > "$r/nestwatch.spec" <<SPEC
%global debug_package %{nil}
%global __os_install_post %{nil}
Name: nestwatch
Version: $v
Release: 1
Summary: $SUMMARY
License: AGPL-3.0-or-later
URL: https://nestwatch.appmire.be/
Requires: pulseaudio-utils
Recommends: libayatana-appindicator-gtk3

%description
$DESCRIPTION

%install
mkdir -p %{buildroot}
cp -a $r/stage/. %{buildroot}/

%files
%license /usr/share/doc/nestwatch/copyright
/usr/bin/nestwatch-desktop
/usr/bin/nestwatch-station
/usr/share/nestwatch
/usr/share/applications/$ID.desktop
/usr/share/icons/hicolor/*/apps/$ID.*
/usr/share/doc/nestwatch/README.md
SPEC
    rpmbuild --quiet --define "_topdir $r/top" --target "$ARCH" -bb "$r/nestwatch.spec"
    cp "$r"/top/RPMS/"$ARCH"/nestwatch-*.rpm "$OUT/nestwatch-linux-$ARCH.rpm"
    ;;
  tar)
    name="nestwatch-linux-$ARCH"
    stage "$WORK/tar/$name"
    cp "$ROOT/LICENSE" "$WORK/tar/$name/LICENSE"
    cat > "$WORK/tar/$name/README.txt" <<TXT
Nestwatch $VERSION for Linux ($ARCH)

Run bin/nestwatch-desktop (desktop app) or bin/nestwatch-station (station
without a window). Keep the share/ directory next to bin/: the station serves
the browser listener from share/nestwatch/web.

Needs WebKitGTK 4.1 and GTK 3, and PipeWire or PulseAudio with pactl, parec
and pacat (Debian/Ubuntu/Fedora: pulseaudio-utils) for sound.

Nestwatch is not a medical device and does not replace supervision.
https://nestwatch.appmire.be/
TXT
    tar -C "$WORK/tar" --owner=0 --group=0 -czf "$OUT/$name.tar.gz" "$name"
    ;;
  *) echo "unknown format $fmt" >&2; exit 2 ;;
  esac
done
ls -l "$OUT"
