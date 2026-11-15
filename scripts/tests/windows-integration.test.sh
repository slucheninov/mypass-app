#!/usr/bin/env bash
# Portable regression tests for native runner inventory and execution accounting.
set -euo pipefail
cd "$(dirname "$0")/../.."
python3 - <<'PY'
import sys
sys.dont_write_bytecode = True
import importlib.util
spec = importlib.util.spec_from_file_location("native_runner", "scripts/windows-integration.py")
m = importlib.util.module_from_spec(spec)
spec.loader.exec_module(m)
names = m.selected_tests(m.SMOKE + ": test\n")
m.verify_execution(names, "test " + m.SMOKE + " ... ok\n")
for output in ["", "0 tests, 0 benchmarks", "unrelated: test\n"]:
    try:
        m.selected_tests(output)
    except ValueError:
        pass
    else:
        raise AssertionError("accepted empty/unrelated inventory")
for output in ["", "test " + m.SMOKE + " ... ignored\n", "test unrelated ... ok\n"]:
    try:
        m.verify_execution(names, output)
    except ValueError:
        pass
    else:
        raise AssertionError("accepted missing native execution")
print("windows-integration runner: portable checks passed")
PY
