#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# Submits an uploaded iOS build for App Review with the committed metadata
# (apps/ios/fastlane lane `submit`).
#   scripts/release/app-store-submit.sh 0.2.0 131
# App Store Connect takes a few minutes to process a fresh upload; until it
# lists the build, deliver fails with "could not find the build". That
# failure is retried every 60 s (ATTEMPTS times); any other is not.
# AUTOMATIC_RELEASE=true releases the version as soon as it is approved.
set -euo pipefail
version="${1:?usage: app-store-submit.sh <version> <build number>}"
build="${2:?build number}"
attempts="${ATTEMPTS:-20}"
log="${RUNNER_TEMP:-${TMPDIR:-/tmp}}/nestwatch-submit-$build.log"
cd "$(dirname "$0")/../../apps/ios"
for ((i = 1; i <= attempts; i++)); do
  echo "fastlane submit $version ($build), attempt $i of $attempts"
  if bundle exec fastlane submit "build_number:$build" "app_version:$version" \
    "automatic_release:${AUTOMATIC_RELEASE:-false}" 2>&1 | tee "$log"; then
    exit 0
  fi
  if ! grep -Eiq "could ?n[o']t find (the |a |any )?build|no build (was )?found|build .*(not found|does ?n[o']t exist)|cannot find build|still processing|processing" "$log"; then
    echo "::error::fastlane submit failed (not a missing build; see the log above)"
    exit 1
  fi
  (( i < attempts )) && sleep 60
done
echo "::error::App Store Connect still has no build $build after $attempts attempts"
exit 1
