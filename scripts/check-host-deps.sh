#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
target=aarch64-apple-darwin
if [[ $# -gt 0 ]]; then
  [[ $# -eq 2 && "$1" == --target && -n "$2" ]] || { echo 'Usage: check-host-deps.sh [--target TRIPLE]' >&2; exit 2; }
  target="$2"
fi
tree="$(cargo tree --locked -p mypass-host -e normal --target "$target")"
if [[ "$tree" =~ mypass-(session|core|kdbx|update) ]]; then
  echo 'Error: shipped host links forbidden vault/update crates' >&2
  exit 1
fi
