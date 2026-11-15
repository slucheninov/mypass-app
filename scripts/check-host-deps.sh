#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
tree="$(cargo tree --locked -p mypass-host -e normal --target aarch64-apple-darwin)"
if [[ "$tree" =~ mypass-(session|core|kdbx|update) ]]; then
  echo 'Error: shipped host links forbidden vault/update crates' >&2
  exit 1
fi
