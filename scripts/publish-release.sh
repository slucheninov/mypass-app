#!/usr/bin/env bash
# Only the final tag job calls this; producer outcomes are passed from needs, never prior CI.
set -euo pipefail
[[ $# == 4 ]] || { echo 'usage: publish-release.sh TAG EMPTY_DIRECTORY MACOS_RESULT WINDOWS_RESULT' >&2; exit 1; }
tag="$1"
folder="$2"
[[ "$3" == success && "$4" == success ]] || { echo 'Both release producers must succeed' >&2; exit 1; }
[[ "$tag" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo 'Stable version tag required' >&2; exit 1; }
[[ ! -e "$folder" ]] || { echo 'Download directory must not exist' >&2; exit 1; }
[[ "$(gh release view "$tag" --json isDraft --jq .isDraft)" == true ]]
gh release download "$tag" --dir "$folder"
python3 "$(dirname "$0")/release.py" check-assets "$folder" "${tag#v}" --platform all
gh release edit "$tag" --draft=false
