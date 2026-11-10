# AGENTS.md

Shared instruction file for every coding agent working in this repository (Claude Code, Codex,
Cursor, and anything else that reads `AGENTS.md`; harnesses may be named here, in this intro, and
nowhere else in this file). `CLAUDE.md` points here; there is no second source of truth.

User instructions in the chat ALWAYS win. Everything below is the default for when the user did not
specify otherwise.

---

# 1. The project

**MyPass** is a password manager built on the KeePass **KDBX4** vault format. It is a Rust + Tauri
rewrite of the earlier Swift app: macOS first, Windows right after, with a Chromium browser extension
(autofill, TOTP, passkeys) arriving in later stages. The vault engine is written from scratch in Rust
on RustCrypto primitives; there is no hand-rolled crypto.

## Crates

| Crate | Status | Role |
|---|---|---|
| `mypass-kdbx` | implemented | KDBX4 engine: format pipeline, model, database API, vault file, `FileIo` seam |
| `mypass-core` | implemented | Everything that consumes the model: TOTP, generator and password rules, Public Suffix List, entry matcher, merge engine |
| `repo-guards` | implemented | Test-only (`publish = false`, never shipped): guard tests for the workspace, CI, docs and scripts |
| `mypass-bridge` | planned | Browser bridge protocol and loopback server |
| `mypass-platform` | planned | OS glue behind traits (clipboard, secret store, lock events) |
| `mypass-host` | planned | Native messaging host binary |
| `src-tauri` | planned | The Tauri app shell (added to the workspace members explicitly when it exists) |
| `extension/` | planned | The Chromium extension (JavaScript, with its own Node test suites) |

## Repo layout

| Path | Contents |
|---|---|
| `Cargo.toml` | Workspace manifest. `members = ["crates/*"]` for now; the single `[workspace.dependencies]` version table; `[workspace.lints]` |
| `rust-toolchain.toml` | Pinned toolchain `1.99.1` (components `rustfmt`, `clippy`) |
| `crates/` | The Rust crates listed above |
| `scripts/test.sh` | The single local and CI test entry point |
| `.github/` | Workflows (`ci.yml`, `codeql.yml`), Dependabot config, issue forms, pull request template |
| `docs/implementation/ROADMAP.md` | The only tracked document under `docs/`: unfinished work |
| `LICENSE`, `THIRD_PARTY_NOTICES.md` | Apache-2.0 and the notices for vendored data and linked crates |

## Stage status

Stage 1 (the Rust core: KDBX4 engine, TOTP, generator, Public Suffix List, matcher, merge, atomic
vault file) is implemented.
Later stages (app backend, UI, extension bridge, passkeys, macOS release, Windows) are listed in `docs/implementation/ROADMAP.md`.

## Build, run, test

Full build and test instructions live in [`BUILD.md`](BUILD.md); the gates an agent runs before
calling a change done:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
bash scripts/test.sh [--ci] [--only rust|ui|js] [--no-js]   # the full suite
```

- `scripts/test.sh` is the one entry point for humans, agents and CI (`--ci` is the CI mode, see
  `BUILD.md` for every flag).
- `--no-js` is only for a local machine without `node`, and it prints that the JavaScript suites were
  NOT run. It is never a way to report a change as tested when that change touches `extension/`.
- Windows: run `cargo` commands from PowerShell, and `scripts/*.sh` from Git Bash.

## Continuous integration

`.github/workflows/ci.yml` runs on pull requests, on pushes to `main` and on manual dispatch, with
`concurrency` that cancels a superseded pull request run (pull requests share one group per ref; runs
on `main` and manual runs each get their own group by run id, so none is cancelled or dropped). The
jobs are independent and run in parallel, so the wall-clock time is the slowest job:

- `lint` is a matrix over `ubuntu-latest` and `windows-latest` (fail-fast off): `cargo clippy
  --workspace --all-targets -- -D warnings` on both (Linux lints the `cfg(unix)` code, Windows the
  `cfg(windows)` code of `crates/mypass-kdbx/src/file.rs`) and `cargo fmt --all -- --check` on Linux
  only.
- `test` is a matrix over `macos-latest` and `windows-latest` (fail-fast off): only
  `bash scripts/test.sh --ci --only rust`.
- Every cargo job restores `Swatinem/rust-cache` with `cache-on-failure`, and the workflow sets
  `CARGO_PROFILE_DEV_DEBUG=line-tables-only` for smaller, faster builds.
- `changes` is a cheap path filter that gates the crosscompat job.
- `crosscompat` runs on Ubuntu with Python 3.12 and CI-only `pykeepass==4.2.0`: Rust writes /
  Python reads, then Python writes / Rust reads. It is path-gated on pull requests through
  `changes.outputs.crosscompat` and always runs on main pushes and manual dispatch.
  The env-gated Rust test prints a skip locally unless `MYPASS_PYKEEPASS_VAULT` is set.
- `windows-integration` is a disabled skeleton for stage 6: it runs only when the repository variable
  `MYPASS_WINDOWS_INTEGRATION_TESTS` is `true`, otherwise it is skipped.
- `ci` is the aggregator and the required status check. It needs every other job (`changes`, `lint`,
  `test`, `windows-integration`, `crosscompat`); a gated job may be `skipped`, every other result
  (`lint` and `test` included) must be `success`. A job added later must be wired into it
  (pinned by `ci_aggregator_needs_every_job`).
- `.github/workflows/codeql.yml` is a separate workflow (push to `main`, weekly, manual; never on pull
  requests) and is not part of `ci`.
- Every `uses:` is pinned to a full commit SHA with the tag in a trailing comment.

CI is a second check that runs after the fact. Run `bash scripts/test.sh` yourself before saying a
change is done.

## Architecture essentials

- **KDBX pipeline.** `Database::open` and `Database::save` compose a chain of small pure modules:
  reader, variant dictionary, headers, KDF, block stream, ciphers, inner stream, XML. All primitives
  come from RustCrypto. KDBX 4.0/4.1 writes; major-4 minor versions are accepted and preserved
  on read/save. KDBX3 returns `UnsupportedVersion`; KDB1 and garbage return `NotKdbx`.
  Password-only composite key in stage 1, no Unicode normalisation.
- **Preserve-KDF rule.** Re-saving an opened vault keeps its cipher, compression, KDF type and KDF
  parameters and its minor version; every save draws a fresh master seed, KDF salt, IV and inner
  stream key from the OS RNG. Fresh vaults use Argon2id (64 MiB, 3 iterations, parallelism 2),
  AES-256-CBC, gzip and a ChaCha20 inner stream.
- **Hostile input.** Limits are named constants in `format/limits.rs` (Argon2 memory, iterations,
  parallelism, salt length, AES-KDF rounds, block length, decompressed size, XML nesting). Parsers
  return `Err` and never panic.
- **Merge.** Two-way merge keyed by UUID.
- **Domain matching.** Always through the Public Suffix List; never suffix heuristics. Credentials
  are offered only on `https` pages: a target whose parsed scheme is not `https` never matches (no
  localhost or IP exception), an `http://` entry is treated as `https://`, and a scheme-less target is
  read as `https`, so callers must pass the full page URL, never a bare host taken from an `http` page.
- **Bundled resources.** The Public Suffix List and the Apple password rules are vendored data files
  under `crates/mypass-core/resources/`, listed in `THIRD_PARTY_NOTICES.md`.
- **`FileIo` seam.** All vault-file I/O goes through the `FileIo` trait (`PlainFileIo` in production,
  `FailingFileIo` under the `test-util` feature in tests). There is no `cfg(test)` failure injection
  in production code. `VaultFile` (in `mypass-kdbx`) wraps atomic saves and conflict detection; `SaveOutcome` reports their result.
- **Signing.** The stable team `W645B93PPQ` arrives in stage 5. Never sign ad-hoc: it breaks the
  Keychain access that guards the unlock key.

---

# 2. Agent workflow conventions

This repository carries no harness-specific agent configuration. Everything an agent needs is in this
file, plus whatever skills and delegation mechanism the running harness provides.

## The orchestrator delegates execution

The main session runs on a deep-reasoning model because architectural reasoning, plan synthesis,
review judgement and multi-source integration benefit from depth. Direct writes and edits by that
model cost several times what a mid-tier model costs for the same mechanical output, so the contract is:

> **The orchestrator THINKS, DECIDES, DELEGATES, and VERIFIES.**
> **The orchestrator does NOT write code, run long shell pipelines, or read large file sets directly
> past the thresholds below.**

The thresholds are HARD numerical triggers, not preferences. When one fires, stop the in-progress
call and dispatch a subagent through the harness's delegation mechanism. No exceptions for "I already
have the context" or "it is just one file".

## Two tiers only

| Tier | Who | Model | May dispatch subagents? |
|---|---|---|---|
| Orchestrator | Main session only | Deep-reasoning model | Yes |
| Executor, volume | Writes and edits; read-only search | Mid-tier model (a small model for trivial mechanical work) | No: execute directly |
| Executor, judgement | Design work; verification and review | Mid-tier model | No: execute directly |

**Critical rule:** the thresholds apply only to the orchestrator. A dispatched subagent is an
executor: it reads, writes, edits and runs shell commands directly within its prompt scope and never
re-delegates the work it was sent to do.

## HARD RULES: check before every tool call

**Rule 1, writing files**

| File size | Action |
|---|---|
| Over 30 lines | STOP. Dispatch an executor. |
| 10 to 30 lines, mechanical | Prefer a dispatch. Write directly only when the user is reviewing inline. |
| Under 10 lines, hand-written | Direct write is fine. |

Two or more small files in a row batch into ONE dispatch.

**Rule 2, editing files**

| Edit volume | Action |
|---|---|
| One edit, under 10 lines of diff | Direct edit is fine. |
| Two or more edits, one file or several | STOP. Dispatch an executor with the full list. |

**Rule 3, reading files**

| Read volume | Action |
|---|---|
| One or two files | Direct read is fine. |
| Three or more files for analysis | STOP. Dispatch a read-only search executor with the question. |

Exception: the file the user is editing, or the literal subject of the question, is read directly.

**Rule 4, shell context-gathering**

| Command volume | Action |
|---|---|
| One or two quick commands | Direct shell is fine (`git status`, `ls`, a short `head`). |
| Three or more commands for context | STOP. Dispatch an executor with the batch. |

**Rule 5, self-audit at the start of every response.** Count what the plan implies before issuing any
tool call. Writes plus edits at 2 or more, reads at 3 or more, context commands at 3 or more, or any
single file over 30 lines: any one of those means the FIRST tool call in the response is the
dispatch. No "I will start and switch later".

## Dispatch template

Every dispatch carries: the model tier (mid-tier for execution, deep-reasoning only when the task
needs design judgement), a 3 to 5 word title, and a prompt with the exact paths, the content or specs
verbatim, and the acceptance criteria. Independent dispatches go out together so they run
concurrently. Pass context in the prompt: the subagent has none of yours.

## What the orchestrator does directly

Reads one or two files for high-level reasoning. Runs one or two orientation commands. Makes a single
edit of at most ten lines. Writes plan documents, holds design conversations and gives review
verdicts in chat. Synthesises subagent results. Talks to the user.

## What the orchestrator never does directly

Writes a file over 30 lines. Applies two or more edits in one response. Reads three or more files for
analysis. Runs three or more context-gathering commands. Does multi-file refactors. Runs a test
suite that takes over ten seconds: `scripts/test.sh` always goes to a subagent.

## Not valid reasons to skip a dispatch

| Tempting thought | Reality |
|---|---|
| "It is faster if I do it directly" | A subagent returns in 10 to 30 seconds. Count the actual seconds. |
| "I already have the context" | Pass it in the prompt. That is what prompts are for. |
| "Just this one file" | Measure it. Over 30 lines? Dispatch. |
| "Simple mechanical edit" | Simple means a cheaper model handles it trivially. Dispatch. |
| "The user is waiting" | The user waits longer when the budget burns on mechanical work. |
| "The subagent might miss context" | Pass exact paths and snippets in the prompt. |
| "I will start and dispatch the rest later" | No. The first tool call is the dispatch. |
| "A hook will catch me if I am wrong" | A hook is a safety net, not a steering wheel. |

## Process skills

If the harness provides skills, process skills come first and set the approach; implementation
skills carry it out. In this order:

1. Brainstorming before any creative or feature work.
2. Systematic debugging before proposing a fix for any bug or test failure.
3. Writing plans and executing plans for multi-step work. Plans and specs live under
   `docs/superpowers/`, which is local-only and gitignored.
4. Verification before completion: before claiming anything is done, fixed or passing, run the
   checks and read the output.
5. Code review (and a security review) on a finished branch.

The orchestrator invokes the skill at the right moment. Executors inherit skill instructions through
the dispatch prompt and execute directly instead of re-dispatching.

## Cost heuristic

A healthy week shows executor tokens well above orchestrator tokens: executors over 70% of the total,
the orchestrator under 30%. If the orchestrator share trends above 30%, these rules are being
skipped. Pause, re-read this file, dispatch the next chunk.

## Anti-patterns

- Sequential dispatches that could have been parallel. Batch them.
- Re-running tools already answered earlier in the session.
- Skipping a review pass. A mid-tier review costs cents; a bug caught late costs far more.
- An executor re-dispatching work it was sent to do. Executors do not delegate.
- Claiming work is done without running `bash scripts/test.sh` yourself. CI runs after the fact and
  is not a substitute.

---

# 3. Conventions

## Language

English for code, comments, commit messages, docs, plans and specs. Cyrillic appears only inside tests
as `\u{..}` escapes, in `*.xcstrings` and in the root `RUN.md` (the Ukrainian manual checklist,
never translated). Literal Cyrillic is also allowed in `crates/mypass-kdbx/tests/unicode.rs`,
vendored upstream data under `crates/mypass-core/resources/`, and opaque binary fixtures
`crates/mypass-kdbx/tests/fixtures/*.kdbx` (fixture README and generator files follow the text rules).
Pinned by `cyrillic_only_in_allowed_files`.

## Commits and pull requests

Conventional Commits with a lowercase imperative subject and no trailing period, for example
`feat(kdbx): read the outer header`. Scopes: `core`, `kdbx`, `ci`, `docs`, `repo` (later stages add
`bridge`, `platform`, `host`, `app`, `extension`). No AI-attribution footers and no co-author
trailers on commits or pull request text. A dependency version bump is its own `chore(deps)` commit.

## Rust rules

- No `unsafe` in our crates (`unsafe_code = "forbid"` through `[workspace.lints]`).
- No `unwrap`, `expect` or indexing in non-test code (`clippy::unwrap_used`, `expect_used` and
  `indexing_slicing` are `warn`, CI runs `-D warnings`). Parsers return `Err` and never panic.
  Test-only support code (guard helpers, integration-test helpers) carries a file-level `allow`
  with a "test-only support code" comment; library code never does.
- Errors are `thiserror` types. Library code never calls `process::exit` and never prints.
- Secrets: the master password is a `secrecy::SecretString`, derived keys are `Zeroizing`, and no
  secret is ever logged or `Debug`-printed (manual `Debug` impls redact).
- Every third-party crate is declared once in `[workspace.dependencies]` with an exact version;
  members write `dep.workspace = true`. Commit `Cargo.lock` with any dependency change. The tauri
  family, `specta` and `tauri-specta` are `=`-pinned.
- Tests are deterministic (seeded RNG, injected clock), use `tempfile` and never touch the network or
  a hard-coded `/tmp`. Text resources are LF-pinned by `.gitattributes`; parsers tolerate CRLF anyway.
- Long comments that record why a constraint exists are a feature; keep the rationale when porting
  code.

## Documentation habit

`docs/superpowers/` holds the spec and plan of each stage and is local-only (gitignored): never
commit it, never link to it from a tracked file. `docs/` tracks only
`docs/implementation/ROADMAP.md`, which lists open boxes only: delete an item once it is implemented
and verified.

## Licensing

MyPass is Apache-2.0. Whenever a dependency or a vendored file is added, removed or bumped, update
`THIRD_PARTY_NOTICES.md` in the same change.

## Never commit

Everything in `.gitignore` (`/docs/superpowers/`, `.superpowers/`, `/target/`, `node_modules/`,
`/dist/`, `.DS_Store`, `.worktrees/`, `*.profraw`), vault files with real data, and secrets.

## Real data

Never test against the owner's real vault. Manual checks use a copy.

## Manual checks

Manual-only checks (signed-build, Windows, real-vault) are walked by the owner in daily use and never block a merge or a release; a task whose only open work is a manual check is complete once its automated part is green.
