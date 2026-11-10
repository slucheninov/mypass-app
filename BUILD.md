# Building and testing MyPass

This file covers building and testing the workspace. For what MyPass is, see
[`README.md`](README.md); for how to submit a change, see [`CONTRIBUTING.md`](CONTRIBUTING.md);
to report a vulnerability, see [`SECURITY.md`](SECURITY.md); coding agents read
[`AGENTS.md`](AGENTS.md).

## Prerequisites

You need [rustup](https://rustup.rs). The toolchain is pinned in `rust-toolchain.toml` (with
`rustfmt` and `clippy`) and rustup installs it on first use. `bash` is needed for
`scripts/test.sh`. Node.js and pnpm are needed only once the UI and the browser extension exist
(later stages); `scripts/test.sh` says so when a part needs a tool you do not have.

On Windows, run `cargo` commands from PowerShell and `scripts/*.sh` from Git Bash.

## Crates

| Crate | Status | Role |
|---|---|---|
| `mypass-kdbx` | implemented | KDBX4 engine: format pipeline, model, database API, vault file |
| `mypass-core` | implemented | TOTP, generator and password rules, Public Suffix List, entry matcher, merge engine |
| `repo-guards` | implemented | Test-only guard tests for the workspace, CI, docs and scripts (never shipped) |
| `mypass-bridge`, `mypass-platform`, `mypass-host`, `src-tauri`, `extension/` | planned | Browser bridge, OS glue, native messaging host, the Tauri app shell and the Chromium extension |

## Build and check

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
```

## Test

```bash
bash scripts/test.sh [--ci] [--only rust|ui|js] [--no-js]   # the full suite
cargo test --workspace                                        # Rust tests only
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
- `--no-js` is only for a local machine without `node`, and it prints that the JavaScript suites
  were NOT run. It is never a way to report a change as tested when that change touches
  `extension/`. `--only` cannot be combined with `--no-js`.
- Integration test files are selected with `--test <file>`; several name filters go after `--`
  (`cargo test -p mypass-kdbx -- filter1 filter2`).
- Argon2 at 64 MiB x 3 passes takes minutes at opt-level 0, so dependencies are built at opt-level 3
  in the dev and test profiles (`[profile.dev.package."*"]`, `[profile.test.package."*"]`).
- CI runs `lint` (clippy on Linux and Windows, format check on Linux), `test` (macOS and Windows),
  `crosscompat` and a disabled Windows integration skeleton as parallel jobs, joined by the `ci`
  aggregator.
- The `crosscompat` CI job (Rust writes and Python reads, then the reverse) uses `pykeepass`. The
  env-gated Rust test prints a skip locally unless `MYPASS_PYKEEPASS_VAULT` is set.

Run `bash scripts/test.sh` before you call a change done; CI runs after the fact and is not a
substitute.
