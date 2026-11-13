#!/usr/bin/env bash
# Builds the native messaging host and stages it as the Tauri sidecar:
#   src-tauri/binaries/mypass-native-host-<target triple>   (Tauri `externalBin` naming; not
#   `mypass-host`, which would collide with cargo's own binary in the target directory)
# Usage: scripts/build-host.sh [--release]
#
# Signing: when MYPASS_SIGNING_IDENTITY is set the staged binary is signed with it, passed to
# codesign unchanged. Use the 40-hex SHA-1 of the valid certificate, never the name (a revoked
# duplicate with the same name makes a name ambiguous). Never sign ad-hoc.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PROFILE_DIR="debug"
CARGO_FLAGS=()

for arg in "$@"; do
  case "$arg" in
    --release)
      PROFILE_DIR="release"
      CARGO_FLAGS+=(--release)
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

for cmd in cargo rustc python3; do
  if ! command -v "$cmd" >/dev/null 2>&1; then
    echo "Error: required command '$cmd' not found." >&2
    exit 1
  fi
done

cd "$REPO_ROOT"
TRIPLE="$(rustc -vV | sed -n 's/^host: //p')"
if [[ -z "$TRIPLE" ]]; then
  echo "Error: could not read the host target triple from 'rustc -vV'." >&2
  exit 1
fi

cargo build -p mypass-host ${CARGO_FLAGS[@]+"${CARGO_FLAGS[@]}"}

TARGET_DIR="$(cargo metadata --format-version 1 --no-deps | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')"
EXE_SUFFIX=""
case "$TRIPLE" in *windows*) EXE_SUFFIX=".exe" ;; esac
SRC="$TARGET_DIR/$PROFILE_DIR/mypass-host$EXE_SUFFIX"
DEST_DIR="$REPO_ROOT/src-tauri/binaries"
DEST="$DEST_DIR/mypass-native-host-$TRIPLE$EXE_SUFFIX"

if [[ ! -s "$SRC" ]]; then
  echo "Error: $SRC was not built." >&2
  exit 1
fi

mkdir -p "$DEST_DIR"
cp -f "$SRC" "$DEST.tmp"
chmod 0755 "$DEST.tmp"
mv -f "$DEST.tmp" "$DEST"

if [[ -n "${MYPASS_SIGNING_IDENTITY:-}" ]]; then
  if ! [[ "$MYPASS_SIGNING_IDENTITY" =~ ^[0-9A-Fa-f]{40}$ ]]; then
    echo "warning: MYPASS_SIGNING_IDENTITY does not look like a 40-hex SHA-1 hash." >&2
    echo "  Find it with: security find-identity -v -p codesigning (use the entry that is not revoked)" >&2
  fi
  codesign --force --sign "$MYPASS_SIGNING_IDENTITY" "$DEST"
  echo "host signed: $DEST"
else
  echo "host left unsigned (set MYPASS_SIGNING_IDENTITY for a stable signature)"
  echo "  Find it with: security find-identity -v -p codesigning (use the entry that is not revoked)" >&2
fi
echo "host staged: $DEST"
