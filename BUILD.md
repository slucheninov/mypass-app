# Building and testing MyPass

This file covers building and testing the workspace. For what MyPass is, see
[`README.md`](README.md); for how to submit a change, see [`CONTRIBUTING.md`](CONTRIBUTING.md);
to report a vulnerability, see [`SECURITY.md`](SECURITY.md); coding agents read
[`AGENTS.md`](AGENTS.md).

## Prerequisites

You need [rustup](https://rustup.rs). The toolchain is pinned in `rust-toolchain.toml` (with
`rustfmt` and `clippy`) and rustup installs it on first use. `bash` is needed for
`scripts/test.sh`. Node.js (the version in `.nvmrc`) is needed for the `js` part of `scripts/test.sh`, which runs the
i18n converter and catalog tests with the built-in `node --test` runner (no packages to install);
`cargo test --workspace` itself needs no Node. pnpm is needed only once the UI has tests (stage 2b);
until then `scripts/test.sh` says `ui: NOT RUN`.

On Windows, run `cargo` commands from PowerShell and `scripts/*.sh` from Git Bash.

## Crates

| Crate | Status | Role |
|---|---|---|
| `mypass-kdbx` | implemented | KDBX4 engine: format pipeline, model, database API, vault file |
| `mypass-core` | implemented | TOTP, generator and password rules, Public Suffix List, entry matcher, merge engine |
| `repo-guards` | implemented | Test-only guard tests for the workspace, CI, docs and scripts (never shipped) |
| `mypass-platform` | scaffold (stage 2a) | OS glue behind traits: clipboard, secret store, system events |
| `mypass-session` | scaffold (stage 2a) | Vault session, settings, scheduling and the command facade; no Tauri dependency |
| `src-tauri` (`mypass-app`) | scaffold (stage 2a) | The Tauri 2 app shell and the typed IPC bindings (`src/lib/bindings.ts`, generated) |
| `mypass-bridge`, `mypass-host`, `extension/` | planned | Browser bridge, native messaging host and the Chromium extension |

## Build and check

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
```

## Test

```bash
bash scripts/test.sh [--ci] [--only rust|ui|js] [--no-js]   # the full suite
cargo test --workspace                                        # Rust tests only
cargo test -p mypass-app export_bindings_regenerate -- --ignored   # regenerate src/lib/bindings.ts
cargo test --workspace -- --ignored                           # includes the 50 MiB round trip
cargo test -p <crate> <filter>                                # one crate, one filter
cargo run -p mypass-kdbx --example write_crosscompat -- <out.kdbx>   # synthetic fixture
```

- `scripts/test.sh` is the one entry point for people, agents and CI. Without `--only` it runs every
  part (`rust`, `ui`, `js`); a part with nothing to run yet prints that it was skipped. The `rust`
  part runs `cargo test --workspace --no-fail-fast`, so one failing test binary does not hide
  failures in the later ones.
- `--ci` runs cargo with `--locked`, requires every tool of every part that exists and rejects
  `--no-js`.
- The `js` part runs `node --test scripts/`: `convert-xcstrings.test.mjs` (the converter) and
  `i18n-completeness.test.mjs` (the committed catalogs: every key in `uk` and `ru`, matching
  placeholders, plural `other` branches, InfoPlist strings). `npm run i18n:test` is the same command.
  The JSON catalogs in `src/lib/i18n/locales/` came from `node scripts/convert-xcstrings.mjs --in
  <Localizable.xcstrings> --out src/lib/i18n/locales --strict` once and are now edited by hand; add
  a new key to all three files together (`--merge` keeps hand edits if the converter is re-run).
- `--no-js` is only for a local machine without `node`, and it prints that the JavaScript suites
  were NOT run. It is never a way to report a change as tested when that change touches
  `extension/`. `--only` cannot be combined with `--no-js`.
- Integration test files are selected with `--test <file>`; several name filters go after `--`
  (`cargo test -p mypass-kdbx -- filter1 filter2`).
- Argon2 at 64 MiB x 3 passes takes minutes at opt-level 0, so dependencies are built at opt-level 3
  in the dev and test profiles (`[profile.dev.package."*"]`, `[profile.test.package."*"]`).
- CI runs `lint` (clippy on macOS and Windows, format check on macOS), `test` (macOS and Windows),
  `i18n` (the `js` part on Ubuntu), `crosscompat` and a disabled Windows integration skeleton as
  parallel jobs, joined by the `ci`
  aggregator.
- The `crosscompat` CI job (Rust writes and Python reads, then the reverse) uses `pykeepass`. The
  env-gated Rust test prints a skip locally unless `MYPASS_PYKEEPASS_VAULT` is set.

Run `bash scripts/test.sh` before you call a change done; CI runs after the fact and is not a
substitute.
