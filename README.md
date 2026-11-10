<h1 align="center">MyPass</h1>

<p align="center">A password manager built on the open KeePass KDBX4 format.</p>

<p align="center">
  <a href="https://github.com/slucheninov/mypass-app/releases/latest"><img src="https://img.shields.io/github/v/release/slucheninov/mypass-app?label=Download&color=blue&style=for-the-badge" alt="Download"></a>
  <a href="https://github.com/slucheninov/mypass-app/releases/latest"><img src="https://img.shields.io/github/downloads/slucheninov/mypass-app/total?style=for-the-badge" alt="Total downloads"></a>
</p>

<p align="center">
  <img src="https://img.shields.io/badge/macOS-000000?style=for-the-badge&logo=apple&logoColor=white" alt="macOS">
  <img src="https://img.shields.io/badge/Windows-0078D6?style=for-the-badge&logo=windows&logoColor=white" alt="Windows">
  <img src="https://img.shields.io/badge/Chromium_extension-4285F4?style=for-the-badge&logo=googlechrome&logoColor=white" alt="Chromium extension">
</p>

<p align="center">
  <a href="SECURITY.md"><img src="https://img.shields.io/badge/Security-policy-green?style=for-the-badge" alt="Security policy"></a>
  <a href="https://github.com/slucheninov/mypass-app/actions/workflows/ci.yml"><img src="https://img.shields.io/github/actions/workflow/status/slucheninov/mypass-app/ci.yml?branch=main&label=CI&style=for-the-badge" alt="CI"></a>
  <a href="LICENSE"><img src="https://img.shields.io/github/license/slucheninov/mypass-app?style=for-the-badge" alt="License"></a>
</p>

MyPass keeps your passwords, notes and one-time codes in a single encrypted file in the KeePass
**KDBX4** format, so your vault also opens in KeePassXC and other KeePass-compatible apps. There is
no lock-in. It is a native desktop app: macOS first, Windows right after. A Chromium browser
extension (autofill, one-time codes, passkeys) follows.

> [!NOTE]
> MyPass is under development. The vault engine and the core logic (the Rust core) are done and
> tested. The desktop app, the installers and the browser extension are not released yet, so there
> is nothing to download yet and the Download badge stays empty until the first release. See
> [`docs/implementation/ROADMAP.md`](docs/implementation/ROADMAP.md) for what is next.

## Features

- 📦 **Open vault format.** KDBX 4.0 and 4.1 vaults that work with KeePassXC and other KeePass tools. Compatibility is checked against `pykeepass` in both directions in CI.
- 🔐 **Strong encryption.** New vaults use Argon2id key derivation (64 MiB, 3 passes), AES-256 or ChaCha20 encryption and gzip compression. An existing vault keeps its own cipher and key-derivation settings when it is saved.
- 🛡️ **Safe against damaged or hostile files.** Size and cost limits apply, and a bad file is rejected with an error, never a crash.
- 💾 **Safe saving.** Writes are atomic, so a failed save never corrupts your vault. MyPass notices when another program changed the file in the meantime and does not silently overwrite it. On macOS the vault file is readable and writable by its owner only.
- 🗂️ **Entries and groups.** Previous versions of an entry are kept as history, and deleted items can go to a recycle bin.
- 🔢 **One-time codes.** Time-based one-time passwords (TOTP).
- 🎲 **Password generator.** It honours per-site password rules (based on Apple's password rules data).
- 🌐 **Smart login matching.** Logins are matched to sites with the Public Suffix List, not guesswork, with several match modes. Credentials are offered only on `https` pages.
- 🔄 **Merge two copies of a vault.** If the same vault changed on two computers, the copies merge without losing data: entries are matched by identity, the newest change wins, history is kept and deletions are respected.
- 🧼 **Careful with secrets.** Your vault is protected by a master password, and secrets are wiped from memory where possible and never printed in debug output.

### Coming next

- 🖥️ Desktop app for macOS, then Windows
- 🧩 Browser extension with autofill, one-time codes and passkeys
- 🔑 Unlock key stored in the operating system keychain or secret store

## Download

Releases will be published at <https://github.com/slucheninov/mypass-app/releases/latest>, macOS
first and Windows after. There are no files yet. This section will list the files when the first
release is out.

## Install

For now there is nothing to install. Installation steps will be added here with the first release.
The expected flow is to open the disk image and drag the app to Applications on macOS, and to run
the installer on Windows.

## Your data and privacy

Your vault is a plain KDBX4 file that you own. You can move it, back it up and open it with
KeePassXC at any time. The core crates make no network calls.

## Security

To report a vulnerability, see [`SECURITY.md`](SECURITY.md). Private vulnerability reporting is
enabled on GitHub. Never post vault files or passwords in issues.

## Build from source and contribute

To build and test MyPass yourself, see [`BUILD.md`](BUILD.md). To propose a change, see
[`CONTRIBUTING.md`](CONTRIBUTING.md).

## Licence

MyPass is licensed under the Apache License 2.0, see [`LICENSE`](LICENSE). Third-party components
are listed in [`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md).
