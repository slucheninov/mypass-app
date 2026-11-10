"""Check the synthetic Rust fixture using the independent pykeepass implementation."""
import sys
from pathlib import Path

from pykeepass import PyKeePass


def check(path):
    if not Path(path).is_file():
        raise ValueError(f"vault file does not exist: {path}")
    vault = PyKeePass(path, password="RoundTripPass123")
    assert vault.encryption_algorithm == "aes256", "expected AES-256"
    assert vault.kdf_algorithm == "argon2id", "expected Argon2id"
    assert vault.kdbx.header.value.dynamic_header.compression_flags.data.compression, "expected gzip"
    assert len(vault.entries) == 2, "expected exactly two entries"
    entry = vault.find_entries(title="Round Trip Entry", first=True)
    assert entry is not None, "missing Round Trip Entry"
    assert entry.username == "roundtrip-user", "username mismatch"
    assert entry.password == "roundtrip-pass", "password mismatch"
    assert len(entry.history) == 1, "expected one history version"
    assert entry.history[0].password == "roundtrip-pass", "history password mismatch"
    assert len(entry.attachments) == 1, "expected one attachment"
    attachment = entry.attachments[0]
    assert attachment.filename == "data.bin", "attachment filename mismatch"
    assert attachment.data == bytes(range(256)), "attachment bytes mismatch"
    unicode = vault.find_entries(title="\u0411\u0430\u043d\u043a", first=True)
    assert unicode is not None, "missing Unicode entry"
    assert unicode.username in (None, ""), "Unicode username mismatch"
    assert unicode.password == "\u043f\u0430\u0440\u043e\u043b\u044c-1", "Unicode password mismatch"


if __name__ == "__main__":
    try:
        if len(sys.argv) != 2:
            raise ValueError("usage: pykeepass_check.py <vault.kdbx>")
        check(sys.argv[1])
    except Exception as error:
        sys.exit(f"PYKEEPASS_CHECK_FAILED: {error}")
    print("PYKEEPASS_OK")
