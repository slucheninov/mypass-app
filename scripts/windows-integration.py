"""Run only explicitly ignored Windows native tests; fail closed on zero selection/execution."""
import os
import re
import subprocess
import sys

SMOKE = "windows_integration_known_folders_resolve_to_existing_absolute_dirs"


def selected_tests(output):
    names = re.findall(r"^(.+): test$", output, re.MULTILINE)
    if SMOKE not in names or any("windows_integration" not in name for name in names):
        raise ValueError("Windows native test inventory is empty or missing the required smoke test")
    return names


def verify_execution(names, output):
    passed = re.findall(r"^test (.+) \.\.\. ok$", output, re.MULTILINE)
    if sorted(passed) != sorted(names):
        raise ValueError("Windows native tests did not all execute successfully")


def main():
    if sys.platform != "win32" or os.environ.get("MYPASS_WINDOWS_INTEGRATION_TESTS") != "1":
        raise ValueError("Windows native runner requires Windows and explicit environment opt-in")
    env = dict(os.environ, RUST_TEST_THREADS="1")
    base = ["cargo", "test", "--locked", "--workspace", "--", "--ignored", "windows_integration"]
    listing = subprocess.run(base + ["--list"], env=env, text=True, capture_output=True, check=True)
    names = selected_tests(listing.stdout)
    result = subprocess.run(base + ["--test-threads=1"], env=env, text=True, capture_output=True)
    print(result.stdout, end="")
    print(result.stderr, end="", file=sys.stderr)
    result.check_returncode()
    verify_execution(names, result.stdout)
    print(f"Windows native integration: {len(names)} selected and passed")


if __name__ == "__main__":
    main()
