#!/usr/bin/env bash
# The single local and CI test entry point (agents call it too).
#
#   scripts/test.sh [--ci] [--only rust|ui|js] [--no-js]
#
# Parts: rust (cargo test --workspace), ui (pnpm; the hook is filled by stage 2b), js (extension
# suites; the hook is filled by stage 3 through scripts/test-extension.sh). Later stages fill the
# hook bodies below and never change this command line.
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage: scripts/test.sh [--ci] [--only rust|ui|js] [--no-js]

  --ci            CI mode: cargo runs with --locked; requires the tool of every part that
                  exists; rejects --no-js.
  --only PART     Run one part only: rust, ui or js. Cannot be combined with --no-js.
  --no-js         Skip the JavaScript suites (local machine without node only). The run says
                  that they were NOT run; this is never a way to call a change that touches
                  extension/ tested.
  -h, --help      Show this help.

Without --only every part runs. Exit codes: 0 pass, 1 a part failed or a tool is missing,
2 invalid arguments.
USAGE
}

die_usage() {
  echo "error: $1" >&2
  echo "Run 'scripts/test.sh --help' for usage." >&2
  exit 2
}

ci=0
only=""
no_js=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --ci) ci=1 ;;
    --no-js) no_js=1 ;;
    --only)
      [[ $# -ge 2 ]] || die_usage "--only needs a value (rust, ui or js)"
      only="$2"
      shift
      ;;
    --only=*) only="${1#--only=}" ;;
    -h | --help)
      usage
      exit 0
      ;;
    *) die_usage "unknown argument: $1" ;;
  esac
  shift
done

# Flag validation happens before any preflight or cargo call.
case "$only" in
  "" | rust | ui | js) ;;
  *) die_usage "unknown --only value: '$only' (expected rust, ui or js)" ;;
esac
if [[ -n "$only" && $no_js -eq 1 ]]; then
  die_usage "--only cannot be combined with --no-js"
fi
if [[ $ci -eq 1 && $no_js -eq 1 ]]; then
  die_usage "--ci rejects --no-js (CI must run every suite)"
fi

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

# Which parts run.
want_rust=0
want_ui=0
want_js=0
if [[ -z "$only" ]]; then
  want_rust=1
  want_ui=1
  [[ $no_js -eq 1 ]] || want_js=1
else
  case "$only" in
    rust) want_rust=1 ;;
    ui) want_ui=1 ;;
    js) want_js=1 ;;
  esac
fi

has_ui() { [[ -f package.json ]]; }
has_js() { [[ -f scripts/test-extension.sh ]]; }

need_tool() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "error: '$1' is required for the $2 part but is not on PATH" >&2
    exit 1
  }
}

# Preflight: only the tools of the selected parts, and a part only needs its tool once it exists.
# A local run without node fails when JS suites exist, unless --no-js was given.
[[ $want_rust -eq 1 ]] && need_tool cargo rust
[[ $want_ui -eq 1 ]] && has_ui && need_tool pnpm ui
[[ $want_js -eq 1 ]] && has_js && need_tool node js

run_rust() {
  # The notice generator reads the locked graph; CI must never silently omit this check.
  if [[ $ci -eq 1 ]]; then
    need_tool python3 notices
  fi
  if command -v python3 >/dev/null 2>&1; then
    python3 crates/mypass-kdbx/tests/crosscompat/update_notices.py --check
  else
    echo "notices: skipped (python3 unavailable)"
  fi
  echo "== rust: cargo test --workspace"
  local args=(test --workspace)
  [[ $ci -eq 1 ]] && args+=(--locked)
  cargo "${args[@]}"
}

# Stage 2b fills this hook with install, type-check and unit tests.
run_ui() {
  if ! has_ui; then
    echo "ui: skipped (no package.json yet)"
    return 0
  fi
  echo "error: package.json exists but the ui hook of scripts/test.sh is not filled in yet" >&2
  return 1
}

# Stage 3 adds scripts/test-extension.sh; until then there is nothing to run.
run_js() {
  if ! has_js; then
    echo "js: skipped (no extension suites yet)"
    return 0
  fi
  echo "== js: scripts/test-extension.sh"
  bash scripts/test-extension.sh
}

[[ $want_rust -eq 1 ]] && run_rust
[[ $want_ui -eq 1 ]] && run_ui
[[ $want_js -eq 1 ]] && run_js

if [[ -z "$only" && $no_js -eq 1 ]]; then
  echo "js: NOT RUN (--no-js). The JavaScript suites were not run; do not report a change that touches extension/ as tested."
fi
echo "test.sh: done"
