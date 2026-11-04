#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-or-later
# Generates the Kotlin and Swift bindings of nestwatch-ffi (UniFFI library mode)
# into target/uniffi/{kotlin,swift}. The Android build generates its own
# Kotlin copy (apps/android, task uniffiBindings); apps/ios/build-rust.sh
# packages the Swift sources, header and module map from here.
#
# Usage: crates/nestwatch-ffi/generate-bindings.sh [kotlin|swift|all]
set -eu
cd "$(dirname "$0")/../.."
which=${1:-all}
cargo build -p nestwatch-ffi --lib
lib=target/debug/libnestwatch_ffi.so
[ -f "$lib" ] || lib=target/debug/libnestwatch_ffi.dylib
gen() {
  cargo run --quiet -p nestwatch-ffi --features bindgen --bin uniffi-bindgen -- \
    generate --library "$lib" --language "$1" --no-format --out-dir "target/uniffi/$1"
}
case "$which" in
  kotlin | swift) gen "$which" ;;
  all) gen kotlin; gen swift ;;
  *) echo "usage: $0 [kotlin|swift|all]" >&2; exit 2 ;;
esac
echo "bindings in target/uniffi/"
