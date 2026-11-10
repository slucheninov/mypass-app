"""Regenerate and audit the Cargo notice tables from the locked dependency graph.

Run from anywhere: python3 crates/mypass-kdbx/tests/crosscompat/update_notices.py
Use --check to verify without writing. No network access is needed with cached crates.
"""
import json
from pathlib import Path
import re
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[4]
START = "<!-- BEGIN GENERATED CRATES -->"
END = "<!-- END GENERATED CRATES -->"
# These exact expressions have been reviewed: each offers a permissive/MPL choice.
# LGPL appears only as an alternative; select MIT for that dual-licensed crate.
APPROVED = {
    "(MIT OR Apache-2.0) AND Unicode-3.0", "0BSD OR MIT OR Apache-2.0",
    "Apache-2.0", "Apache-2.0 / MIT", "Apache-2.0 OR BSL-1.0",
    "Apache-2.0 OR MIT", "Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT",
    "BSD-2-Clause OR Apache-2.0 OR MIT", "BSD-3-Clause",
    "CC0-1.0 OR MIT-0 OR Apache-2.0", "MIT", "MIT OR Apache-2.0",
    "MIT OR Apache-2.0 OR LGPL-2.1-or-later", "MIT OR Zlib OR Apache-2.0",
    "MIT/Apache-2.0", "Unicode-3.0", "Unlicense OR MIT", "Zlib", "MPL-2.0",
}


def cargo(*args):
    return subprocess.check_output(["cargo", *args], cwd=ROOT, text=True)


def generate():
    metadata = json.loads(cargo("metadata", "--locked", "--format-version", "1"))
    normal = cargo("tree", "--locked", "--workspace", "--exclude", "repo-guards",
                   "--target", "all", "-e", "normal", "--prefix", "none", "--format", "{p}")
    linked = set()
    for line in normal.splitlines():
        match = re.match(r"([^ ]+) v([^ ]+)", line)
        if match:
            linked.add(match.groups())
    packages = sorted(metadata["packages"], key=lambda p: (p["name"], p["version"]))
    for package in packages:
        license = package["license"]
        if license not in APPROVED:
            raise ValueError(f"unreviewed licence: {package['name']} {license}")
        if "GPL" in license and license != "MIT OR Apache-2.0 OR LGPL-2.1-or-later":
            raise ValueError(f"GPL/AGPL dependency: {package['name']}")
    sections = [START, "<!-- Generate/verify: python3 crates/mypass-kdbx/tests/crosscompat/update_notices.py [--check] -->"]
    for release, title in [(True, "Shipped/linked in release"), (False, "Test/dev only (including build tooling and target-only lock entries)")]:
        sections += ["", f"### {title}", "", "| Crate | Version | Licence |", "|---|---|---|"]
        for p in packages:
            if p["id"] in metadata["workspace_members"]:
                continue
            if ((p["name"], p["version"]) in linked) == release:
                sections.append(f"| `{p['name']}` | {p['version']} | {p['license']} |")
    sections += ["", END]
    return "\n".join(sections), len(packages), len(linked)


if __name__ == "__main__":
    table, count, linked = generate()
    path = ROOT / "THIRD_PARTY_NOTICES.md"
    text = path.read_text()
    before, rest = text.split(START)
    _, after = rest.split(END)
    updated = before + table + after
    if "--check" in sys.argv:
        if updated != text:
            sys.exit("NOTICES_FAILED: regenerate Cargo tables")
    else:
        path.write_text(updated)
    print(f"NOTICES_OK: {count} packages audited; {linked} release graph packages; permissive/MPL choices only")
