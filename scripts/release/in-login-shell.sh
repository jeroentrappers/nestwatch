#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# Runs a command in the runner user's login shell, the environment of a
# manual release: Homebrew, rbenv's Ruby with bundler, cargo (on mini4: zsh).
# The store workflow (.github/workflows/stores.yml) wraps its fastlane steps
# in it. The arguments are passed as they are; the job's environment is
# inherited.
#   scripts/release/in-login-shell.sh bundle exec fastlane release
set -euo pipefail
[[ $# -gt 0 ]] || { echo "usage: in-login-shell.sh <command> [args...]" >&2; exit 2; }
if command -v zsh >/dev/null 2>&1; then sh=zsh; else sh=bash; fi
exec "$sh" -l -i -c 'exec "$@"' nestwatch-release "$@"
