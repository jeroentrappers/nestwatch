#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-or-later
# Installs the wasm-bindgen CLI whose version matches the wasm-bindgen crate
# in Cargo.lock (they must match exactly), from the project's release
# tarball, checked against its published SHA-256; falls back to cargo install.
set -eu
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
V=$(sed -n '/^name = "wasm-bindgen"$/{n;s/^version = "\(.*\)"$/\1/p;}' "$ROOT/Cargo.lock")
[ -n "$V" ] || { echo "wasm-bindgen not found in Cargo.lock" >&2; exit 1; }
if command -v wasm-bindgen >/dev/null && wasm-bindgen --version | grep -q " $V\$"; then
  echo "wasm-bindgen $V already installed"; exit 0
fi
case "$(uname -s)-$(uname -m)" in
  Linux-x86_64) T=x86_64-unknown-linux-musl ;;
  Linux-aarch64) T=aarch64-unknown-linux-musl ;;
  Darwin-arm64) T=aarch64-apple-darwin ;;
  Darwin-x86_64) T=x86_64-apple-darwin ;;
  *) T="" ;;
esac
DEST=${CARGO_HOME:-$HOME/.cargo}/bin
mkdir -p "$DEST"
if [ -n "$T" ]; then
  N="wasm-bindgen-$V-$T"
  URL="https://github.com/wasm-bindgen/wasm-bindgen/releases/download/$V/$N.tar.gz"
  TMP=$(mktemp -d)
  if curl -fsSL "$URL" -o "$TMP/$N.tar.gz" && curl -fsSL "$URL.sha256sum" -o "$TMP/sum"; then
    want=$(cut -d' ' -f1 "$TMP/sum")
    have=$( (sha256sum "$TMP/$N.tar.gz" 2>/dev/null || shasum -a 256 "$TMP/$N.tar.gz") | cut -d' ' -f1)
    [ "$want" = "$have" ] && tar -C "$TMP" -xzf "$TMP/$N.tar.gz" \
      && install -m 0755 "$TMP/$N/wasm-bindgen" "$DEST/wasm-bindgen" && rm -rf "$TMP" \
      && wasm-bindgen --version && exit 0
  fi
  rm -rf "$TMP"
  echo "prebuilt download failed; building from source" >&2
fi
cargo install --locked wasm-bindgen-cli --version "$V"
