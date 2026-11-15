#!/usr/bin/env bash
# Builds a local debug app bundle with the host sidecar.
# Usage: scripts/dev-app.sh [--release]
#
# With MYPASS_SIGNING_IDENTITY (the 40-hex SHA-1 of a valid certificate) the host and the app are
# signed with it, passed through unchanged. Without it the build stays unsigned: an unsigned
# rebuild can make macOS prompt for the pairing Keychain item. Never sign ad-hoc.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
HOST_FLAGS=()
TAURI_FLAGS=(--debug)
PROFILE_DIR=debug

for arg in "$@"; do
  case "$arg" in
    --release)
      HOST_FLAGS+=(--release)
      TAURI_FLAGS=()
      PROFILE_DIR=release
      ;;
    *)
      echo "Usage: $0 [--release]" >&2
      exit 2
      ;;
  esac
done

# Never sign ad-hoc ("-" makes codesign and Tauri sign ad-hoc; an empty value is a mistake).
if [[ -n "${MYPASS_SIGNING_IDENTITY+set}" ]]; then
  if [[ -z "$MYPASS_SIGNING_IDENTITY" || "$MYPASS_SIGNING_IDENTITY" == "-" ]]; then
    echo "Error: MYPASS_SIGNING_IDENTITY is empty or '-' (ad-hoc signing is refused)." >&2
    echo "  Unset it, or set the 40-hex SHA-1 of a valid certificate:" >&2
    echo "  security find-identity -v -p codesigning (use the entry that is not revoked)" >&2
    exit 2
  fi
fi

for cmd in cargo pnpm; do
  if ! command -v "$cmd" >/dev/null 2>&1; then
    echo "Error: required command '$cmd' not found." >&2
    exit 1
  fi
done

cd "$REPO_ROOT"
bash scripts/build-host.sh ${HOST_FLAGS[@]+"${HOST_FLAGS[@]}"}

if [[ -n "${MYPASS_SIGNING_IDENTITY:-}" ]]; then
  APPLE_SIGNING_IDENTITY="$MYPASS_SIGNING_IDENTITY" pnpm tauri build --bundles app ${TAURI_FLAGS[@]+"${TAURI_FLAGS[@]}"}
else
  echo "warning: building unsigned (set MYPASS_SIGNING_IDENTITY to sign)." >&2
  env -u APPLE_SIGNING_IDENTITY pnpm tauri build --bundles app ${TAURI_FLAGS[@]+"${TAURI_FLAGS[@]}"}
fi

if [[ -n "${MYPASS_SIGNING_IDENTITY:-}" ]]; then
  TARGET_DIR="$(cargo metadata --format-version 1 --no-deps | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')"
  BUNDLE="$TARGET_DIR/$PROFILE_DIR/bundle/macos/MyPass.app"
  codesign --force --options runtime --identifier com.mypass.chromehost \
    -r='designated => identifier "com.mypass.chromehost" and anchor apple generic and certificate leaf[subject.OU] = "W645B93PPQ"' \
    --sign "$MYPASS_SIGNING_IDENTITY" "$BUNDLE/Contents/MacOS/mypass-native-host"
  codesign --force --options runtime --identifier com.mypass.app \
    -r='designated => identifier "com.mypass.app" and anchor apple generic and certificate leaf[subject.OU] = "W645B93PPQ"' \
    --sign "$MYPASS_SIGNING_IDENTITY" "$BUNDLE"
  codesign --verify --deep --strict "$BUNDLE"
fi
