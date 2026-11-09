# MyPass

MyPass is a password manager built on the open KeePass **KDBX4** vault format, so your vault stays a
plain `.kdbx` file that KeePassXC and other KeePass-compatible tools can open too. It is a Rust and
Tauri rewrite of an earlier Swift app: macOS first, Windows right after, with a Chromium browser
extension for autofill, TOTP and passkeys arriving in later stages.

## Status

**Stage 1, the Rust core, is in progress.** The workspace holds the portable core: a KDBX4 engine
written from scratch on RustCrypto primitives (`mypass-kdbx`), and the logic on top of the vault model
(`mypass-core`: TOTP, password generator, Public Suffix List matching, entry matching, merge). There
is no app, UI or browser extension yet; they follow in later stages. What is still open is listed in
[`docs/implementation/ROADMAP.md`](docs/implementation/ROADMAP.md).

## Build and test

You need the Rust toolchain pinned in `rust-toolchain.toml` (rustup installs it on first use).

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
bash scripts/test.sh          # the full suite; add --ci for CI mode
cargo test --workspace        # Rust tests only
```

`scripts/test.sh [--ci] [--only rust|ui|js] [--no-js]` is the one test entry point for people, agents
and CI. On Windows, run `cargo` from PowerShell and the shell scripts from Git Bash.

## Contributing and security

See [`CONTRIBUTING.md`](CONTRIBUTING.md) for how to build, test and submit a change, and
[`SECURITY.md`](SECURITY.md) for how to report a vulnerability privately. Coding agents read
[`AGENTS.md`](AGENTS.md).

## Licence

MyPass is licensed under the Apache License 2.0, see [`LICENSE`](LICENSE). Third-party components are
listed in [`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md).
