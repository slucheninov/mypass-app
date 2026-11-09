#!/usr/bin/env python3
"""Generates pykeepass-attachments.kdbx, the third-party attachments fixture.

Purpose: KdbxAttachmentRoundTripTests.testPykeepassFixtureWithAttachmentsRoundTrips
opens a KDBX4 file produced by an independent implementation (not by MyPass) that
carries attachments on a current entry, on a history version, and the same bytes on
two different entries (pool dedup).

Produced with pykeepass 4.2.0 (https://github.com/libkeepass/pykeepass), which is
GPL-3.0 licensed. It is test tooling only: it is not linked into or shipped with
MyPass, and only its generated output (a data file) is checked in, so it does not
belong in THIRD_PARTY_NOTICES.md.

Content (password `fixture-password`, KDBX4):
  Docs  attachments note.txt (3 bytes "abc"), blob.bin (70 KB seeded pseudo-random);
        one history version that carries old.txt
  Copy  attachment same.txt, bytes identical to note.txt

Run (output is not byte-reproducible: pykeepass randomises UUIDs, salts and times):
  python3 -m venv /tmp/venv && /tmp/venv/bin/pip install pykeepass==4.2.0
  /tmp/venv/bin/python make-pykeepass-attachments.py pykeepass-attachments.kdbx
"""
import random
import sys

from pykeepass import create_database

out = sys.argv[1] if len(sys.argv) > 1 else "pykeepass-attachments.kdbx"
kp = create_database(out, password="fixture-password")

note = b"abc"
blob = random.Random(20261107).randbytes(70 * 1024)

docs = kp.add_entry(kp.root_group, "Docs", "docs-user", "docs-pass")
# First version of the entry carries old.txt; the snapshot taken by the next save
# becomes the history item, and old.txt is then dropped from the current entry.
docs.add_attachment(kp.add_binary(b"old"), "old.txt")
kp.save()
docs.save_history()
for att in list(docs.attachments):
    docs.delete_attachment(att)
docs.add_attachment(kp.add_binary(note), "note.txt")
docs.add_attachment(kp.add_binary(blob), "blob.bin")

copy = kp.add_entry(kp.root_group, "Copy", "copy-user", "copy-pass")
copy.add_attachment(kp.add_binary(note), "same.txt")
kp.save()
print("wrote", out)
