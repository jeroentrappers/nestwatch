#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-or-later
# Builds the Lull browser listener into a self-contained static bundle
# (security.md PRIV-1: no CDN, no external fonts).
#
#   apps/web/build.sh [OUT_DIR] [--demo] [--debug]
#
# OUT_DIR defaults to target/lull-web (where lull-desktop looks for it).
# Needs the wasm32-unknown-unknown target and wasm-bindgen-cli matching the
# wasm-bindgen version in Cargo.lock:
#   cargo install --locked wasm-bindgen-cli --version <version from Cargo.lock>
set -eu
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
OUT="$ROOT/target/lull-web"
PROFILE=release
FEATURES=""
for a in "$@"; do
  case "$a" in
    --demo) FEATURES="--features demo" ;;
    --debug) PROFILE=debug ;;
    -*) echo "unknown option $a" >&2; exit 2 ;;
    *) OUT=$a ;;
  esac
done
cd "$ROOT"
if [ "$PROFILE" = release ]; then
  cargo build -p lull-web --target wasm32-unknown-unknown --release $FEATURES
else
  cargo build -p lull-web --target wasm32-unknown-unknown $FEATURES
fi
mkdir -p "$OUT"
wasm-bindgen --target web --no-typescript --out-dir "$OUT" \
  "$ROOT/target/wasm32-unknown-unknown/$PROFILE/lull-web.wasm"
cp "$ROOT/design/lull.css" "$ROOT/design/logo.svg" "$ROOT/apps/ui/app.css" \
   "$ROOT/apps/web/static/index.html" "$ROOT/apps/web/static/boot.js" \
   "$ROOT/apps/web/static/audio-worklet.js" "$OUT/"
echo "lull-web bundle in $OUT"
