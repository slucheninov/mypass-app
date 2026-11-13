#!/usr/bin/env bash
# Builds a local debug app bundle with the host sidecar.
# Usage: scripts/dev-app.sh
#
# With MYPASS_SIGNING_IDENTITY (the 40-hex SHA-1 of a valid certificate) the host and the app are
# signed with it, passed through unchanged. Without it the build stays unsigned: an unsigned
# rebuild can make macOS prompt for the pairing Keychain item. Never sign ad-hoc.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"

for cmd in cargo pnpm; do
  if ! command -v "$cmd" >/dev/null 2>&1; then
    echo "Error: required command '$cmd' not found." >&2
    exit 1
  fi
done

cd "$REPO_ROOT"
bash scripts/build-host.sh

if [[ -n "${MYPASS_SIGNING_IDENTITY:-}" ]]; then
  APPLE_SIGNING_IDENTITY="$MYPASS_SIGNING_IDENTITY" pnpm tauri build --debug
else
  echo "warning: building unsigned (set MYPASS_SIGNING_IDENTITY to sign)." >&2
  pnpm tauri build --debug
fi
