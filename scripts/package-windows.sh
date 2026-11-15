#!/usr/bin/env bash
# Unsigned x64 ZIP; Offline --no-build fixtures use only Bash and Python.
set -euo pipefail
command -v python3 >/dev/null || { echo 'Error: required command python3 not found' >&2; exit 1; }
exec python3 "$(dirname "$0")/windows-package.py" "$@"
