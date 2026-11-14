#!/usr/bin/env bash
# Runs the independent passkey oracles (python-fido2, pykeepass, cryptography) against our output.
#
#   Rust writes vectors.json + vault.kdbx  ->  passkey_oracle.py verifies them and writes a
#   KeePassXC-style vault with pykeepass   ->  Rust reads that vault back and signs with its keys.
#
# In CI (CI=true) a missing python3 or package is a failure. Locally it is a clear skip (exit 0).
# PYTHON overrides the interpreter (for example a venv with crosscompat/requirements.txt).
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
python="${PYTHON:-python3}"
skip() {
  echo "passkey oracle: $1" >&2
  if [ "${CI:-}" = "true" ]; then
    echo "passkey oracle: refusing to skip in CI" >&2
    exit 1
  fi
  echo "passkey oracle: SKIPPED (not a pass)" >&2
  exit 0
}

command -v "$python" >/dev/null 2>&1 || skip "$python not found"
"$python" -I -c 'import fido2, cryptography, pykeepass' >/dev/null 2>&1 \
  || skip "python packages missing; pip install -r crates/mypass-core/tests/crosscompat/requirements.txt"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
cd "$root"
cargo run --locked -q -p mypass-core --example write_passkey_vectors -- "$work"
"$python" -I crates/mypass-core/tests/crosscompat/passkey_oracle.py "$work"
MYPASS_PYKEEPASS_PASSKEY_DIR="$work/pykeepass" \
  cargo test --locked -p mypass-core --test passkey_pykeepass -- --nocapture
