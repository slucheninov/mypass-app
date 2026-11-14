#!/usr/bin/env bash
# Runs the Chromium extension suites: every extension/tests/*.test.js in its own node process.
#
#   scripts/test-extension.sh [--list]
#
# --list prints the discovered suite files (one per line, nothing else). Without it the script
# prints `N suites, M failed` and exits non-zero when any suite failed.
set -euo pipefail
# The glob order is the suite order; pin it to byte order so it never depends on the locale.
export LC_ALL=C

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

shopt -s nullglob
suites=(extension/tests/*.test.js)

if [[ "${1:-}" == "--list" ]]; then
  [[ $# -eq 1 ]] || { echo "error: --list takes no further arguments" >&2; exit 2; }
  for f in "${suites[@]}"; do echo "$f"; done
  exit 0
fi
[[ $# -eq 0 ]] || { echo "usage: scripts/test-extension.sh [--list]" >&2; exit 2; }

command -v node >/dev/null 2>&1 || { echo "error: 'node' is required to run the extension suites" >&2; exit 1; }
node_major="$(node -p 'process.versions.node.split(".")[0]')"
if (( node_major < 26 )); then
  echo "error: node >= 26 is required (found $(node -v))" >&2
  exit 1
fi
if (( ${#suites[@]} == 0 )); then
  echo "error: no extension suites found under extension/tests" >&2
  exit 1
fi

failed=0
for f in "${suites[@]}"; do
  if ! node "$f"; then
    echo "FAILED: $f" >&2
    failed=$((failed + 1))
  fi
done
echo "${#suites[@]} suites, $failed failed"
[[ $failed -eq 0 ]]
