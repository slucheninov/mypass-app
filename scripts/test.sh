#!/usr/bin/env bash
# The single local and CI test entry point (agents call it too).
#
#   scripts/test.sh [--ci] [--only rust|ui|js] [--no-js]
#
# Parts: rust (cargo test --workspace --no-fail-fast), ui (pnpm install --frozen-lockfile, check, test, build), js
# (the Node built-in suites of scripts/*.test.mjs, i.e. the i18n converter and catalog tests, plus the
# extension suites once stage 3 adds scripts/test-extension.sh). Later stages fill the hook bodies
# below and never change this command line.
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

# A UI suite exists once package.json declares a `test` script (stage 2b). Without one there is
# nothing to run and pnpm is not required.
has_ui() { [[ -f package.json ]] && grep -q '"test"[[:space:]]*:' package.json; }
has_node_suites() { compgen -G "scripts/*.test.mjs" >/dev/null; }
has_js() { has_node_suites || [[ -f scripts/test-extension.sh ]]; }

need_tool() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "error: '$1' is required for the $2 part but is not on PATH" >&2
    exit 1
  }
}

# Preflight: only the tools of the selected parts, and a part only needs its tool once it exists.
# A local run without node fails when JS suites exist, unless --no-js was given.
[[ $want_rust -eq 1 ]] && need_tool cargo rust
# The ui part needs node and pnpm. In --ci a missing tool is a failure; locally run_ui says
# `ui: NOT RUN` instead, so a machine without the JS toolchain can still run the rust part.
if [[ $want_ui -eq 1 && $ci -eq 1 ]] && has_ui; then
  need_tool node ui
  need_tool pnpm ui
fi
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
  echo "== rust: cargo test --workspace --no-fail-fast"
  # --no-fail-fast: one failing test binary must not hide failures in the binaries after it.
  local args=(test --workspace --no-fail-fast)
  [[ $ci -eq 1 ]] && args+=(--locked)
  cargo "${args[@]}"
}

# The frontend: a frozen install (the lockfile is the contract), svelte-check with warnings as
# errors, the Vitest suites and a production build (the build is what `tauri build` bundles, so a
# broken bundle must fail here and not in packaging).
run_ui() {
  if [[ ! -f package.json ]]; then
    echo "ui: skipped (no package.json yet)"
    return 0
  fi
  if ! has_ui; then
    echo "ui: NOT RUN (package.json has no test script yet; no UI suite exists)"
    return 0
  fi
  if [[ $ci -eq 0 ]] && ! { command -v node >/dev/null 2>&1 && command -v pnpm >/dev/null 2>&1; }; then
    echo "ui: NOT RUN (node and pnpm are required for the UI suite and are not on PATH; install them or use --only rust)"
    return 0
  fi
  echo "== ui: pnpm install --frozen-lockfile"
  pnpm install --frozen-lockfile
  echo "== ui: pnpm check"
  pnpm check
  echo "== ui: pnpm test"
  pnpm test
  echo "== ui: pnpm build"
  pnpm build
}

# The Node suites need nothing but node itself (built-in test runner, no npm install). Stage 3 adds
# scripts/test-extension.sh for the extension suites.
run_js() {
  if ! has_js; then
    echo "js: skipped (no JavaScript suites yet)"
    return 0
  fi
  if has_node_suites; then
    echo "== js: node --test scripts/"
    node --test scripts/
  fi
  if [[ -f scripts/test-extension.sh ]]; then
    echo "== js: scripts/test-extension.sh"
    bash scripts/test-extension.sh
  fi
}

[[ $want_rust -eq 1 ]] && run_rust
[[ $want_ui -eq 1 ]] && run_ui
[[ $want_js -eq 1 ]] && run_js

if [[ -z "$only" && $no_js -eq 1 ]]; then
  echo "js: NOT RUN (--no-js). The JavaScript suites were not run; do not report a change that touches extension/ as tested."
fi
echo "test.sh: done"
