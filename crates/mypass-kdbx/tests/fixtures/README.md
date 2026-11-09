# Synthetic KDBX4 fixtures

Copied unchanged from the earlier MyPassKit test suite. These contain synthetic test data,
never an owner vault. KeePassXC and pykeepass are independent producing tools.
The data and generator are maintained as MyPass test assets under Apache-2.0.
pykeepass 4.2.0 (GPL-3.0) is generation tooling only and is not shipped or linked.

| File | Produced by | Password | Content |
|---|---|---|---|
| `aeskdf-aes256-none.kdbx` | KeePassXC | `TestMasterPass123` | AES-KDF, AES-256, no compression; root entry Title `Test Entry`, UserName `testuser`, Password `hunter2` |
| `argon2id-aes256-gzip.kdbx` | KeePassXC | `TestMasterPass123` | same entry |
| `argon2d-aes256-gzip.kdbx` | KeePassXC | `TestMasterPass123` | same entry |
| `chacha20-argon2id-gzip.kdbx` | KeePassXC | `TestMasterPass123` | same entry |
| `pykeepass-attachments.kdbx` | pykeepass 4.2.0 via `make-pykeepass-attachments.py` | `fixture-password` | entry `Docs`: attachments `note.txt` (`abc`), `blob.bin` (70 KiB, sha256 `6541e8907643d16a8a53fc9613ee0c8d7a8c5540085a14da9225ff0b09ef9deb`), one history version with `old.txt` (`old`); entry `Copy` with `same.txt` (`abc`); all binaries protected |

`make-pykeepass-attachments.py` is the original synthetic attachment generator.
