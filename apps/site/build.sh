#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-or-later
# Builds the static Nestwatch website (https://nestwatch.appmire.be/).
#
#   apps/site/build.sh [OUT_DIR]
#
# OUT_DIR defaults to target/nestwatch-site. Needs only Python 3.12 or newer
# (standard library). The output is plain files: HTML per page and language,
# design/nestwatch.css and the logo, site.css, site.js, images, robots.txt and
# sitemap.xml. No CDN, no web fonts, no trackers, no cookies (PRIV-1).
set -eu
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
OUT=${1:-"$ROOT/target/nestwatch-site"}
exec python3 "$ROOT/apps/site/build.py" "$OUT"
