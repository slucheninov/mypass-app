#!/usr/bin/env bash
# Only the final tag job calls this; producer outcomes are passed from needs, never prior CI.
set -euo pipefail
[[ $# == 4 ]] || { echo 'usage: publish-release.sh TAG EMPTY_DIRECTORY MACOS_RESULT WINDOWS_RESULT' >&2; exit 1; }
tag="$1"
folder="$2"
[[ "$3" == success && "$4" == success ]] || { echo 'Both release producers must succeed' >&2; exit 1; }
[[ "$tag" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo 'Stable version tag required' >&2; exit 1; }
[[ ! -e "$folder" ]] || { echo 'Download directory must not exist' >&2; exit 1; }
# Check status separately: a failed API may still have printed a draft value.
# Explicit exits also hold when a caller disables errexit (including Bash 3.2).
draft="$(gh release view "$tag" --json isDraft --jq .isDraft)" || { echo 'Cannot read release draft status' >&2; exit 1; }
[[ "$draft" == true ]] || { echo 'Release must be a draft' >&2; exit 1; }
gh release download "$tag" --dir "$folder" || { echo 'Release download failed' >&2; exit 1; }
python3 "$(dirname "$0")/release.py" check-assets "$folder" "${tag#v}" --platform all || { echo 'Release asset verification failed' >&2; exit 1; }
gh release edit "$tag" --draft=false || { echo 'Release publication failed' >&2; exit 1; }
