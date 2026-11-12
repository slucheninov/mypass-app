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
| `mypass-platform` | in progress (stage 2a) | OS glue behind traits (system events, secure clipboard, secret stores, device unlock, native host registrar, launch at login, keyboard layout) and `AppPaths`; fakes behind the `test-util` feature, `Unsupported` fallbacks where a platform body has not landed |
| `mypass-session` | in progress (stage 2a) | Vault session, settings, scheduling and the `AppCore` command facade; no Tauri dependency |
| `src/` (Svelte UI) | in progress (stage 2b) | The webview UI: typed API wrapper, stores, i18n, design kit; screens follow |
| `src-tauri` (package `mypass-app`, lib `mypass_app_lib`) | in progress (stage 2a) | The Tauri 2 app shell: the typed commands and events (`tauri-specta`), capabilities, native file dialogs, plugins, tray, global hotkeys, single instance, quit flow, native-side i18n; a workspace member listed explicitly |
| `mypass-bridge` | planned | Browser bridge protocol and loopback server |
| `mypass-host` | planned | Native messaging host binary |
| `extension/` | planned | The Chromium extension (JavaScript, with its own Node test suites) |

## Repo layout

| Path | Contents |
|---|---|
| `Cargo.toml` | Workspace manifest. `members = ["crates/*", "src-tauri"]`; the single `[workspace.dependencies]` version table; `[workspace.lints]` |
| `rust-toolchain.toml` | Pinned toolchain `1.99.1` (components `rustfmt`, `clippy`) |
| `crates/` | The Rust crates listed above |
| `src-tauri/` | The Tauri app shell (`mypass-app`): `tauri.conf.json`, capabilities, placeholder icons (replaced in stage 5) |
| `src/lib/bindings.ts` | Generated TypeScript bindings of the Tauri commands; never edited by hand |
| `pnpm-workspace.yaml` | pnpm settings: `allowBuilds: esbuild: false` (esbuild's postinstall is not needed, the platform binary comes as an optional dependency) |
| `package.json`, `pnpm-lock.yaml`, `.nvmrc` | Node and pnpm pins, the exact-pinned npm dependencies and the scripts `dev`, `build`, `check`, `test`, `tauri`; the lockfile is committed |
| `index.html`, `vite.config.ts`, `svelte.config.js`, `tsconfig.json`, `vitest.setup.ts` | The Svelte 5 (runes) + Vite + Vitest toolchain of the UI |
| `src/` | The UI: `main.ts` picks the root for the window label (`windows/`), `test/` holds the fake backend and the render helper, `scaffold.test.ts` holds the static source guards |
| `scripts/test.sh` | The single local and CI test entry point |
| `scripts/convert-xcstrings.mjs` | One-time converter of the old app's String Catalogs; its output (`src/lib/i18n/locales/{en,uk,ru}.json`, `src-tauri/resources/macos/*.lproj/InfoPlist.strings`) is then the hand-edited source of truth |
| `.github/` | Workflows (`ci.yml`, `codeql.yml`), Dependabot config, issue forms, pull request template |
| `RUN.md` | The Ukrainian manual checklist: checks that cannot be automated (tray, quit dialog, hotkey conflicts) |
| `docs/implementation/ROADMAP.md` | The only tracked document under `docs/`: unfinished work |
| `LICENSE`, `THIRD_PARTY_NOTICES.md` | Apache-2.0 and the notices for vendored data and linked crates |

## Stage status

Stage 1 (the Rust core: KDBX4 engine, TOTP, generator, Public Suffix List, matcher, merge, atomic
vault file) is implemented.
Stage 2a (the app backend behind the glass: `mypass-platform`, `mypass-session`, the Tauri shell) is in progress; the command layer, events, capabilities and the shell integration (tray, hotkeys, single instance, quit flow) have landed.
Later stages (rest of the app backend, UI, extension bridge, passkeys, macOS release, Windows) are listed in `docs/implementation/ROADMAP.md`.

## Build, run, test

Full build and test instructions live in [`BUILD.md`](BUILD.md); the gates an agent runs before
calling a change done:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
bash scripts/test.sh [--ci] [--only rust|ui|js] [--no-js]   # the full suite
```

UI commands (Node from `.nvmrc`, pnpm from `packageManager`):

```bash
pnpm install --frozen-lockfile   # never edit pnpm-lock.yaml by hand
pnpm tauri dev                   # the app with hot reload (starts `pnpm dev` through tauri.conf.json)
pnpm check                       # svelte-check, warnings are errors
pnpm test                        # Vitest (jsdom), including the static source guards
pnpm build                       # production bundle into dist/
```

- `scripts/test.sh` is the one entry point for humans, agents and CI (`--ci` is the CI mode, see
  `BUILD.md` for every flag).
- The `ui` part runs `pnpm install --frozen-lockfile`, `pnpm check`, `pnpm test` and `pnpm build`;
  `--ci` makes a missing node or pnpm a failure, a local run without them prints `ui: NOT RUN`.
- The `js` part runs `node --test scripts/` (the i18n converter and catalog tests; plain Node, nothing
  to install) and, from stage 3, the extension suites. `--no-js` is only for a local machine without
  `node`, and it prints that the JavaScript suites were NOT run. It is never a way to report a change as tested when that change touches `extension/`.
- Windows: run `cargo` commands from PowerShell, and `scripts/*.sh` from Git Bash.

## Continuous integration

`.github/workflows/ci.yml` runs on pull requests, on pushes to `main` and on manual dispatch, with
`concurrency` that cancels a superseded pull request run (pull requests share one group per ref; runs
on `main` and manual runs each get their own group by run id, so none is cancelled or dropped). The
jobs are independent and run in parallel, so the wall-clock time is the slowest job:

- `lint` is a matrix over `macos-latest` and `windows-latest` (fail-fast off): `cargo clippy
  --workspace --all-targets -- -D warnings` on both (macOS lints the `cfg(unix)` code, Windows the
  `cfg(windows)` code of `crates/mypass-kdbx/src/file.rs`, and both compile the Tauri shell) and
  `cargo fmt --all -- --check` on macOS only. There is no Linux leg: Linux is not a target and the
  Tauri shell needs WebKitGTK system packages there.
- `test` is a matrix over `macos-latest` and `windows-latest` (fail-fast off): only
  `bash scripts/test.sh --ci --only rust`.
- Every cargo job restores `Swatinem/rust-cache` with `cache-on-failure`, and the workflow sets
  `CARGO_PROFILE_DEV_DEBUG=line-tables-only` for smaller, faster builds.
- `changes` is a cheap path filter that gates the crosscompat job.
- `crosscompat` runs on Ubuntu with Python 3.12 and CI-only `pykeepass==4.2.0`: Rust writes /
  Python reads, then Python writes / Rust reads. It is path-gated on pull requests through
  `changes.outputs.crosscompat` and always runs on main pushes and manual dispatch.
  The env-gated Rust test prints a skip locally unless `MYPASS_PYKEEPASS_VAULT` is set.
- `i18n` runs on Ubuntu with Node from `.nvmrc` (`actions/setup-node`): `bash scripts/test.sh --ci
  --only js`, i.e. `node --test scripts/` (the xcstrings converter and the catalog completeness
  tests; built-in runner, no dependencies, no package install). It always runs and must be `success`.
- `ui` runs on Ubuntu with pnpm (`pnpm/action-setup`, `version` equal to `packageManager`) and Node from
  `.nvmrc` with the pnpm cache: `bash scripts/test.sh --ci --only ui`. It has no rust-cache (it never
  runs cargo), always runs and must be `success`.
- `windows-integration` is a disabled skeleton for stage 6: it runs only when the repository variable
  `MYPASS_WINDOWS_INTEGRATION_TESTS` is `true`, otherwise it is skipped.
- `ci` is the aggregator and the required status check. It needs every other job (`changes`, `lint`,
  `test`, `i18n`, `ui`, `windows-integration`, `crosscompat`); a gated job may be `skipped`, every other result
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
- **Generated files.** `src/lib/bindings.ts` is generated: regenerate it with
  `cargo test -p mypass-app export_bindings_regenerate -- --ignored` and never edit it (the test
  `bindings_are_up_to_date` fails when it drifts). `src-tauri/tests/commands.snap` lists every
  command, sorted, one per line; a stage that adds or removes a command regenerates it with
  `cargo test -p mypass-app command_snapshot_regenerate -- --ignored` in the same commit
  (`command_snapshot_matches` fails otherwise). A command added or removed also needs a handler in
  `src/test/fakeBackend.ts` and an entry in `src/lib/api.ts`, otherwise `pnpm check` and
  `fake_backend_covers_every_binding_command` fail the `ui` job.
- **Node-free Rust build.** `src-tauri/build.rs` creates a placeholder `dist/index.html` when `dist/`
  is absent, so `cargo test --workspace` never needs Node. `dist/` is git-ignored.
- **Windows manifest.** `src-tauri/build.rs` embeds `src-tauri/windows-app-manifest.xml`
  (Common-Controls v6) into every target of the app package through linker arguments, because
  `tauri-build` alone covers only the bin and the lib test binary then dies with `0xc0000139`.
- **Scheduler seam.** Every timing rule of the session (autosave, auto-lock, clipboard clear,
  retries) runs through `mypass_session::scheduler::Scheduler`: `ThreadScheduler` (one worker
  thread) in production, `ManualScheduler` (manual monotonic and wall clocks) in tests. Production
  code never sleeps for a timing rule.
- **Settings.** `mypass_session::settings::Settings` is the one persisted settings struct (every
  field, struct-level `serde(default)`, one `Default` impl), stored atomically in
  `<config_dir>/settings.json`; a field with a bad value falls back to its default alone (nested
  objects field by field, unknown fields ignored), only a file that is not a JSON object loads as
  defaults and is renamed `settings.json.bad`, and a file that cannot be read is never written over
  (the store runs degraded on defaults).
  The schema is pinned by `crates/mypass-session/tests/snapshots/settings.default.json`.
- **Vault session.** `mypass_session::session::VaultSession` owns the one open vault (`NoVault`,
  `Locked`, `Unlocked`). Edits go through `mutate`, which edits a clone and swaps it in only on
  `Ok` (rollback by construction). Saving (`mypass_session::saving`) is a debounced autosave
  (1 s) that writes outside the session lock, one write at a time (an in-flight flag plus a
  condvar orders autosave, `flush` and `lock`, and its one finish path re-arms the autosave while
  the session is dirty); a file changed on disk is reloaded and adopted wholesale by a clean
  session (nothing written) or two-way merged into a dirty one, which is then written, never
  overwritten blindly (the remote's save settings are always kept, the remote's metadata unless
  it was edited locally); `flush` is `Ok` only when every edit is on disk. `lock` saves first and keeps the encrypted bytes when that
  save fails: a timer writes them over an unchanged file, otherwise the next unlock merges them,
  and switching vaults first writes them as an `unsaved-*` copy into the backup folder (the
  path when the password was changed elsewhere); observers run after the session lock is released;
  `begin_quit` closes every opening and mutating call with `Quitting` while `lock` and `flush`
  keep working. The master password lives only in the session as a `SecretString`. Tests use `mypass_session::testing::fast_database`
  (Argon2id 8 MiB, 1 pass) so saves stay fast.
- **Backups.** `mypass_session::backups::BackupManager` copies the on-disk vault bytes to
  `<backups_dir>/<vault_id>/backup-<unix_secs>[-<n>].kdbx` before it is overwritten (first write of
  a session always, then per `backupInterval`; once per disk state, so retries of a failed write
  take none, and never when the bytes equal the newest backup), keeps the newest `backupCount`,
  and never blocks a save: a failure is recorded in `last-failure.json` and cleared by the next
  success.
- **Auto-lock and clipboard clear.** `mypass_session::autolock::AutoLock` locks on the idle
  timer, on sleep and screen lock (per settings) and, after `Wake`, when the idle time ran out
  (idle time is the larger of the wall-clock and the monotonic measure, so neither a sleep nor a
  clock set back defeats it); platform event sinks only hand events to the scheduler (`AutoLock::event_sink`),
  because a sink must never block and `lock` waits for an in-flight save.
  `mypass_session::clipboard_service::ClipboardService` always schedules the clear and clears
  only through `clear_if_unchanged`, so a later copy by the user is never wiped.
- **Platform seam.** Everything OS-specific goes through `mypass_platform::Platform` (trait objects,
  built by `mypass_platform::current`) and every on-disk location through `AppPaths`. Tests use
  `mypass_platform::testing::fake_platform()` under the `test-util` feature; no non-ignored test touches the
  real clipboard, Keychain or system notifications.
- **File dialogs.** The webview never supplies a path for an attachment, an attachment export or
  a CSV import. The dialogs open from Rust behind `mypass_app_lib::files::FilePicker`: picked
  attachments come back as single-use random tokens that `entry_save` swaps for the real path, the
  CSV chosen for a preview is remembered for one `import_csv_commit`, and `attachment_export`
  opens the save dialog itself. Only vault open and create take a path from the UI. Capabilities
  grant no `fs`, `shell`, `http`, `clipboard-manager` or `global-shortcut` permission to any window
  (pinned by `capabilities_grant_no_fs_shell_or_clipboard_read`).
- **Shell.** Decisions are plain functions, the Tauri glue is thin: `shell.rs` (settings effects, close
  and quit rules, login-item reconcile, `WindowOps`), `tray.rs` (menu model and ids), `hotkeys.rs`
  (plan, `HotkeyRegistry` seam), `lifecycle.rs` (platform events into auto-lock, the quit flow,
  the external-change watcher, `ShellEffects`, tray refresh) and `i18n.rs` (the shared catalogs, `{0}`
  substitution, English fallback); only `shell_runtime.rs` touches the runtime. Webviews hold
  `core:event:default`, so no Rust listener acts on a webview-emittable event: every trigger is
  a Rust call (session observer, platform sink, menu, tray, window or run-event callback). Tray
  icon and menus are built in Rust only (no `core:tray`/`core:menu` permission). Work for the main
  thread is posted with `run_on_main_thread` and never awaited; callbacks that would lock hand the
  lock to the scheduler (`lock_via_scheduler`). The quit flow (`on_exit_requested`: `begin_quit`
  first, then `flush_blocking(5 s)`, then `quit_decision`, Cancel re-opens the session) runs on a
  worker because it blocks on a dialog: `ExitRequested` is always prevented and a confirmed exit
  calls `exit` again through `ExitGate`. The effect runner is called off the main thread under the
  settings gate and never takes it.
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
never translated). Literal Cyrillic is also allowed in the translation files
`src/lib/i18n/locales/{uk,ru}.json` and `src-tauri/resources/macos/{uk,ru}.lproj/InfoPlist.strings`
(exactly these four; the English files stay Cyrillic-free, and the Node tests read their expectations
from the catalogs), in `crates/mypass-kdbx/tests/unicode.rs`,
vendored upstream data under `crates/mypass-core/resources/`, and opaque binary fixtures
`crates/mypass-kdbx/tests/fixtures/*.kdbx` (fixture README and generator files follow the text rules).
Pinned by `cyrillic_only_in_allowed_files`.

## Commits and pull requests

Conventional Commits with a lowercase imperative subject and no trailing period, for example
`feat(kdbx): read the outer header`. Scopes: `core`, `kdbx`, `ci`, `docs`, `repo`, `ui`, `i18n` (later stages add
`bridge`, `platform`, `host`, `app`, `extension`). No AI-attribution footers and no co-author
trailers on commits or pull request text. A dependency version bump is its own `chore(deps)` commit. Dependabot covers cargo and GitHub
Actions only: the npm pins in `package.json` are bumped by hand, and the `@tauri-apps/*` packages
move together with the Rust `tauri` family (the CLI rejects mismatched minors).

## Rust rules

- No `unsafe` in our crates (`unsafe_code = "forbid"` through `[workspace.lints]`), with one
  exception: `mypass-platform` carries its own `[lints]` tables, an exact copy of the workspace
  tables except `unsafe_code = "deny"`, and only `crates/mypass-platform/src/macos.rs` lifts it
  with a module-level `#![allow(unsafe_code)]` for the Objective-C and Carbon calls (stage 6 may
  add `windows.rs` the same way). Every `unsafe` block there is minimal, wrapped in a small safe
  function and carries a `// SAFETY:` comment stating the invariant. Pinned by
  `workspace_lints_forbid_unsafe` and `unsafe_only_in_allow_listed_modules`.
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
