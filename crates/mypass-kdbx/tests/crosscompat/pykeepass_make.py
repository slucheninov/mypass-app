"""Create synthetic test data; never open an existing or owner's vault."""
import sys
from pathlib import Path

from pykeepass import create_database


def make(path):
    if Path(path).exists():
        raise ValueError(f"refusing to overwrite existing file: {path}")
    vault = create_database(path, password="fixture-password")
    vault.add_entry(vault.root_group, "Made By Pykeepass", "pk-user", "pk-pass")
    vault.add_entry(vault.root_group, "\u0411\u0430\u043d\u043a", "unicode-user",
                    "\u043f\u0430\u0440\u043e\u043b\u044c-1")
    vault.save()
    reopened = type(vault)(path, password="fixture-password")
    assert len(reopened.entries) == 2, "generated fixture entry count mismatch"


if __name__ == "__main__":
    try:
        if len(sys.argv) != 2:
            raise ValueError("usage: pykeepass_make.py <out.kdbx>")
        make(sys.argv[1])
    except Exception as error:
        sys.exit(f"PYKEEPASS_MAKE_FAILED: {error}")
    print("PYKEEPASS_OK")
