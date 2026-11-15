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
no lock-in. It is a desktop app for macOS and Windows x64 with a Chromium browser
extension for autofill, one-time codes and passkeys.

> [!NOTE]
> MyPass is under development. The desktop app, extension and packaging are implemented;
> release downloads depend on published version tags. Windows desktop checks remain owner work.
> See [`docs/implementation/ROADMAP.md`](docs/implementation/ROADMAP.md) for open work.

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

## Download

See the [release assets](https://github.com/slucheninov/mypass-app/releases/latest) for available
versions: macOS arm64 DMG, Windows x64 ZIP and Chromium extension ZIP. Building locally does not
publish a release.

## Install

On macOS 14+, download `MyPass-X.Y.Z-macos-arm64.dmg` from the
[latest release](https://github.com/slucheninov/mypass-app/releases/latest), verify `SHA256SUMS.txt`
and follow its `INSTALL.txt` to remove quarantine and drag MyPass to Applications.
The Apple Development build is not notarized.

On Windows 10 22H2 or Windows 11 x64, download `MyPass-X.Y.Z-windows-x64.zip` and its
`.zip.sha256` from the [release assets](https://github.com/slucheninov/mypass-app/releases/latest).
Compare `Get-FileHash -Algorithm SHA256` with the checksum, then Properties > Unblock the ZIP
before extracting all files together. Run `MyPass.exe`; follow `INSTALL.txt` for SmartScreen's
More info > Run anyway path and the Microsoft Evergreen WebView2 Runtime if missing. The ZIP
is unsigned and portable; no administrator rights are needed. Keep `mypass-native-host.exe` and
`extension/` beside the app. Windows uses master-password unlock; Windows Hello is deferred.

Load `extension/` unpacked in Chrome or Edge and pair in Settings > Browser. Show extension folder
can reveal a versioned writable copy. First launch registers the native messaging host for the
current user. To remove integration use Settings > Browser > Remove browser integration; launching
again registers it again. Quit before deleting the extracted app folder. Preserve vaults and
backups; integration removal never deletes them. Recovery checks live in [`RUN.md`](RUN.md).

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
