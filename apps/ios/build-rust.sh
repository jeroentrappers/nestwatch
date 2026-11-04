#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-or-later
# Builds nestwatch-ffi for iOS and packages it for the Xcode project:
#
#   apps/ios/Frameworks/NestwatchFFI.xcframework   static library + C header
#                                                  (device arm64, simulator
#                                                  arm64 [+ x86_64])
#   apps/ios/Generated/NestwatchFFI.swift          UniFFI Swift bindings
#
# Both are build outputs (gitignored). Run on macOS with Xcode and rustup.
#
# Usage: apps/ios/build-rust.sh [release|debug]
# Environment:
#   NESTWATCH_IOS_SIM_X86=0   skip the x86_64 simulator slice (Intel Macs
#                             need it; Apple silicon does not)
#   NESTWATCH_IOS_DEVICE=0    skip the device slice (simulator-only builds)
set -eu
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
cd "$root"

profile=${1:-release}
case "$profile" in
  release | debug) ;;
  *) echo "usage: $0 [release|debug]" >&2; exit 2 ;;
esac
outdir=$profile

# Objects must match the app's deployment target (platforms.md §1: iOS 15).
export IPHONEOS_DEPLOYMENT_TARGET=15.0

device=aarch64-apple-ios
sims=aarch64-apple-ios-sim
[ "${NESTWATCH_IOS_SIM_X86:-1}" = 0 ] || sims="$sims x86_64-apple-ios"
targets=$sims
[ "${NESTWATCH_IOS_DEVICE:-1}" = 0 ] || targets="$device $targets"

if command -v rustup > /dev/null 2>&1; then
  # shellcheck disable=SC2086 # a list of target names
  rustup target add $targets > /dev/null
fi

for t in $targets; do
  echo "building nestwatch-ffi for $t ($profile)"
  # Only the static library: the crate also lists cdylib for Android.
  cargo rustc --locked -p nestwatch-ffi --lib --crate-type staticlib \
    --profile "$([ "$profile" = debug ] && echo dev || echo release)" --target "$t"
done

# Swift bindings, C header and module map (library mode, host build).
crates/nestwatch-ffi/generate-bindings.sh swift
gen=target/uniffi/swift

work=target/ios
rm -rf "$work"
mkdir -p "$work/headers" "$work/sim"
cp "$gen/NestwatchFFIFFI.h" "$work/headers/"
# Xcode picks up a module map named module.modulemap next to the header.
cp "$gen/NestwatchFFIFFI.modulemap" "$work/headers/module.modulemap"

lib=libnestwatch_ffi.a
simlibs=""
for t in $sims; do simlibs="$simlibs target/$t/$outdir/$lib"; done
# shellcheck disable=SC2086 # a list of paths
lipo -create $simlibs -output "$work/sim/$lib"

frameworks="$here/Frameworks"
mkdir -p "$frameworks" "$here/Generated"
rm -rf "$frameworks/NestwatchFFI.xcframework"
set -- -library "$work/sim/$lib" -headers "$work/headers"
if [ "${NESTWATCH_IOS_DEVICE:-1}" != 0 ]; then
  set -- -library "target/$device/$outdir/$lib" -headers "$work/headers" "$@"
fi
xcodebuild -create-xcframework "$@" -output "$frameworks/NestwatchFFI.xcframework"
cp "$gen/NestwatchFFI.swift" "$here/Generated/NestwatchFFI.swift"
echo "done: $frameworks/NestwatchFFI.xcframework, $here/Generated/NestwatchFFI.swift"
