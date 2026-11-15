#!/usr/bin/env bash
# Apple Development (team W645B93PPQ), no notarization. Never sign ad-hoc.
set -euo pipefail
repo_root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$repo_root"
out="$repo_root/release"
release=0
allow_unsigned=0
dry_run=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --out) out="$2"; shift ;;
    --release) release=1 ;;
    --allow-unsigned) allow_unsigned=1 ;;
    --dry-run) dry_run=1 ;;
    *) echo "Usage: $0 [--out DIR] [--release] [--allow-unsigned] [--dry-run]" >&2; exit 2 ;;
  esac
  shift
done
identity="${MYPASS_SIGNING_IDENTITY-${APPLE_SIGNING_IDENTITY-}}"
if [[ ${MYPASS_SIGNING_IDENTITY+x} || ${APPLE_SIGNING_IDENTITY+x} ]]; then
  if ! [[ "$identity" =~ ^[0-9A-Fa-f]{40}$ ]]; then
    echo 'Error: signing identity must be a 40-hex certificate SHA-1; no ad-hoc signing.' >&2
    echo 'Find valid identities: security find-identity -v -p codesigning' >&2
    exit 2
  fi
  if [[ -n "${APPLE_SIGNING_IDENTITY:-}" && "$APPLE_SIGNING_IDENTITY" != "$identity" ]]; then
    echo 'Error: conflicting signing identity variables' >&2; exit 2
  fi
fi
if [[ $release -eq 1 && -z "$identity" ]]; then
  echo 'Error: --release requires a signing identity' >&2; exit 2
fi
if [[ -z "$identity" && $allow_unsigned -ne 1 ]]; then
  echo 'Error: UNSIGNED development build requires --allow-unsigned' >&2; exit 2
fi
version="$(bash scripts/check-version.sh)"
if [[ $release -eq 1 ]]; then
  [[ "$(git describe --exact-match --tags 2>/dev/null)" == "v$version" ]] || { echo "Error: release requires exact tag v$version" >&2; exit 1; }
  [[ -z "$(git status --porcelain --untracked-files=normal)" ]] || { echo 'Error: release requires a clean tree' >&2; exit 1; }
fi
if [[ -z "$identity" ]]; then
  echo 'UNSIGNED development build (linker seals only; no codesign signing calls)'
else
  echo "Signing identity SHA-1: $identity"
fi
if [[ $dry_run -eq 1 ]]; then
  echo 'DRY RUN: no builds, signing, keychain access, DMG creation or output writes'
  echo 'build-host --release --target aarch64-apple-darwin -> Tauri app -> explicit host/app requirements -> verify -> DMG -> checksums'
  bash scripts/release-assets.sh "$version"
  exit 0
fi
for tool in cargo pnpm hdiutil codesign csreq shasum python3; do
  command -v "$tool" >/dev/null || { echo "Error: missing $tool" >&2; exit 1; }
done
[[ "$(uname -s)" == Darwin ]] || { echo 'Error: macOS is required' >&2; exit 1; }
# Q1: never pick up unrelated notarization credentials from a developer environment.
unset APPLE_ID APPLE_PASSWORD APPLE_TEAM_ID APPLE_API_KEY APPLE_API_ISSUER APPLE_API_KEY_PATH
# CI already imported the certificate; do not let Tauri create a second keychain.
unset APPLE_CERTIFICATE APPLE_CERTIFICATE_PASSWORD
bash scripts/check-host-deps.sh
mkdir -p "$out"
out="$(cd "$out" && pwd)"
[[ -z "$(ls -A "$out")" ]] || { echo 'Error: output directory must be empty' >&2; exit 1; }
stage="$(mktemp -d)"
trap 'rm -rf "$stage"' EXIT
bash scripts/package-extension.sh --out "$out" --stage-dir "$stage/extension"
sign_args=(--no-sign)
if [[ -n "$identity" ]]; then
  export MYPASS_SIGNING_IDENTITY="$identity" APPLE_SIGNING_IDENTITY="$identity"
  sign_args=()
fi
bash scripts/build-host.sh --release --target aarch64-apple-darwin
# Resource staging is temporary and never changes a tracked bundle configuration.
python3 - "$stage" <<'PY'
import json,sys
from pathlib import Path
stage=Path(sys.argv[1])
(stage/'bundle.json').write_text(json.dumps({'bundle': {'resources': {str(stage/'extension'): 'extension'}}}))
PY
pnpm tauri build --target aarch64-apple-darwin --bundles app ${sign_args[@]+"${sign_args[@]}"} --config "$stage/bundle.json"
target_dir="$(cargo metadata --format-version 1 --no-deps | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')"
app="$target_dir/aarch64-apple-darwin/release/bundle/macos/MyPass.app"
host="$repo_root/src-tauri/binaries/mypass-native-host-aarch64-apple-darwin"
# Tauri signs first. Replace its host with our explicitly signed staged cargo output, then
# re-seal the containing app with the team-based requirement (inside out, no --deep signing).
if [[ -n "$identity" ]]; then
  codesign --force --options runtime --identifier com.mypass.chromehost \
    '-r=designated => identifier "com.mypass.chromehost" and anchor apple generic and certificate leaf[subject.OU] = "W645B93PPQ"' \
    --sign "$identity" "$host"
fi
cp "$host" "$app/Contents/MacOS/mypass-native-host"
if [[ -n "$identity" ]]; then
  codesign --force --options runtime --identifier com.mypass.app \
    '-r=designated => identifier "com.mypass.app" and anchor apple generic and certificate leaf[subject.OU] = "W645B93PPQ"' \
    --sign "$identity" "$app"
fi
verify_args=()
[[ -n "$identity" ]] || verify_args=(--allow-unsigned)
bash scripts/verify-bundle.sh "$app" --host-source "$host" \
  --extension-zip "$out/MyPass-chrome-extension-$version.zip" ${verify_args[@]+"${verify_args[@]}"}
mkdir "$stage/image"
ditto "$app" "$stage/image/MyPass.app"
ln -s /Applications "$stage/image/Applications"
cp scripts/package/INSTALL.txt LICENSE THIRD_PARTY_NOTICES.md "$stage/image/"
dmg="MyPass-$version-macos-arm64.dmg"
hdiutil create -volname "MyPass $version" -srcfolder "$stage/image" -fs HFS+ -format UDZO "$out/$dmg"
if [[ -n "$identity" ]]; then
  codesign --sign "$identity" "$out/$dmg"
  codesign --verify --strict "$out/$dmg"
fi
(cd "$out" && shasum -a 256 "$dmg" "MyPass-chrome-extension-$version.zip" > SHA256SUMS.txt)
python3 scripts/release.py check-assets "$out" "$version"
echo 'Not notarized: follow the quarantine instructions in INSTALL.txt.'
