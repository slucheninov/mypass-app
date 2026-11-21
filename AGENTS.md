# AGENTS.md

Shared instruction file for every coding agent working in this repository (Claude Code, Codex,
Cursor, and anything else that reads `AGENTS.md`; harnesses may be named here, in this intro, and
nowhere else in this file). `CLAUDE.md` points here; there is no second source of truth.

User instructions in the chat ALWAYS win. Everything below is the default for when the user did not
specify otherwise.

---

# 1. The project

**MyPass** is a password manager built on the KeePass **KDBX4** vault format. It is a Rust + Tauri
rewrite of the earlier Swift app: macOS and Windows x64, with a Chromium browser extension
(autofill, TOTP, passkeys). The vault engine is written from scratch in Rust
on RustCrypto primitives; there is no hand-rolled crypto.

## Crates

| Crate | Status | Role |
|---|---|---|
| `mypass-kdbx` | implemented | KDBX4 engine: format pipeline, model, database API, vault file, `FileIo` seam |
| `mypass-core` | implemented | Everything that consumes the model: TOTP, generator and password rules, Public Suffix List, entry matcher, merge engine, WebAuthn passkey ceremonies and the KeePassXC passkey record (`passkey`) |
| `repo-guards` | implemented | Test-only (`publish = false`, never shipped): guard tests for the workspace, CI, docs and scripts |
| `mypass-platform` | macOS and Windows implemented; device unlock and coordinated iCloud access are macOS-only | OS glue behind traits (system events, secure clipboard, secret stores, device unlock, native host registrar, launch at login, keyboard layout) and `AppPaths`; fakes behind the `test-util` feature, `Unsupported` fallbacks where a platform body has not landed |
| `mypass-sync` | implemented | Secondary-folder sync, hash checkpoints, watcher and coordinated iCloud storage with offline cache and conflict merging |
| `mypass-update` | implemented | GitHub release checks behind the `github` feature; banner-only updates, no automatic installation |
| `mypass-session` | implemented | Vault session, settings, scheduling and the `AppCore` command facade; no Tauri dependency |
| `src/` (Svelte UI) | implemented (stage 2b) | The webview UI: typed API wrapper, stores (`src/lib/stores`), i18n, design kit, the screens (`src/screens`, `src/screens/editors`) and the windows (`src/windows`: main, settings, tray panel) |
| `src-tauri` (package `mypass-app`, lib `mypass_app_lib`) | implemented | The Tauri 2 app shell: the typed commands and events (`tauri-specta`), capabilities, native file dialogs, plugins, tray, global hotkeys, single instance, quit flow, native-side i18n; a workspace member listed explicitly |
| `mypass-bridge` | implemented | Browser bridge protocol, codec, endpoint, relay and loopback server (`server` feature, off for the host); the passkey service (`passkeys`) |
| `mypass-host` | implemented | Native messaging host binary; never links the session, core or KDBX crates |
| `extension/` | implemented | The Chromium extension (Manifest V3 JavaScript, vendored `vendor/jsQR.js`), with its 45 Node suites in `extension/tests/`, run by `scripts/test-extension.sh`; its text lives in `extension/_locales/{en,uk,ru,de}/messages.json` (`chrome.i18n`, read through `shared/i18n.js`, `en` is the default locale), pinned by `tests/locales.test.js` |

## Repo layout

| Path | Contents |
|---|---|
| `Cargo.toml` | Workspace manifest. `members = ["crates/*", "src-tauri"]`; the single `[workspace.dependencies]` version table; `[workspace.lints]` |
| `rust-toolchain.toml` | Pinned toolchain `1.99.1` (components `rustfmt`, `clippy`) |
| `crates/` | The Rust crates listed above |
| `src-tauri/` | The Tauri app shell (`mypass-app`): `tauri.conf.json`, capabilities, owner artwork in `icons/source.png` and generated app icons |
| `src/lib/bindings.ts` | Generated TypeScript bindings of the Tauri commands; never edited by hand |
| `pnpm-workspace.yaml` | pnpm settings: `allowBuilds: esbuild: false` (esbuild's postinstall is not needed, the platform binary comes as an optional dependency) |
| `package.json`, `pnpm-lock.yaml`, `.nvmrc` | Node and pnpm pins, the exact-pinned npm dependencies and the scripts `dev`, `build`, `check`, `test`, `tauri`; the lockfile is committed |
| `index.html`, `vite.config.ts`, `svelte.config.js`, `tsconfig.json`, `vitest.setup.ts` | The Svelte 5 (runes) + Vite + Vitest toolchain of the UI |
| `src/` | The UI: `main.ts` picks the root for the window label (`windows/`), `lib/` the API wrapper, stores and i18n, `screens/` the screens and editors, `test/` holds the fake backend (type-checked against the bindings), the render helper and the shared source scan `textScan.ts`, `scaffold.test.ts` holds the static source guards, `journey.test.ts`, `a11y.test.ts` and `i18n.audit.test.ts` the journey, accessibility and string-audit suites |
| `scripts/test.sh` | The single local and CI test entry point |
| `scripts/build-host.sh`, `scripts/dev-app.sh` | Build the `mypass-host` sidecar (signed when `MYPASS_SIGNING_IDENTITY` names a certificate SHA-1 on macOS; Windows is unsigned; never ad-hoc), and build and run the app locally with it |
| `scripts/package-macos.sh`, `scripts/verify-bundle.sh`, `scripts/check-host-deps.sh` | arm64 DMG, signed app/host verification and isolated shipped-host dependency check |
| `scripts/bump-version.sh`, `scripts/check-version.sh`, `scripts/package-extension.sh`, `scripts/verify-release-assets.sh` | Version consistency, deterministic extension ZIP and release artifact verification |
| `.github/workflows/release.yml` | Tag-driven macOS signed DMG and Windows unsigned ZIP: full tests, temporary Apple certificate import/cleanup, exact five draft assets verified, publish last |
| `scripts/build-windows.sh`, `scripts/package-windows.sh` | Production x64 MSVC app/sidecar build, deterministic unsigned ZIP and sibling checksum |
| `scripts/test-extension.sh` | Runs every `extension/tests/*.test.js` suite in its own node process (`--list` prints them); part of the `js` part |
| `scripts/make-locked-icons.mjs` | Generates the four locked toolbar PNGs from the shipped key icons. A Node test compares the committed pixels with the generator |
| `scripts/convert-xcstrings.mjs` | One-time converter of the old app's String Catalogs; its output (`src/lib/i18n/locales/{en,uk,ru}.json`, `src-tauri/resources/macos/{en,uk,ru}.lproj/InfoPlist.strings`) is then the hand-edited source of truth; `de.json` and `de.lproj` were written by hand and never went through it |
| `.github/` | Workflows (`ci.yml`, `codeql.yml`), Dependabot config, issue forms, pull request template |
| `RUN.md` | The Ukrainian manual checklist: checks that cannot be automated (tray, quit dialog, hotkey conflicts), the Stage 2b UI walk-through and the Stage 3 browser bridge checklist (M1 to M14) and the Stage 4 passkey checklist (P1 to P13), Stage 5 checks (S1 to S21), cutover (C1 to C10) Stage 6 Windows checks (W1 to W17) and the extension popup checks (E1 to E10) |
| `docs/implementation/ROADMAP.md` | The only tracked document under `docs/`: unfinished work |
| `LICENSE`, `THIRD_PARTY_NOTICES.md` | Apache-2.0 and the notices for vendored data and linked crates |

## Stage status

Stage 1 (the Rust core: KDBX4 engine, TOTP, generator, Public Suffix List, matcher, merge, atomic
vault file) is implemented.
Stage 2a (the app backend behind the glass: `mypass-platform` macOS bodies, `mypass-session`, the Tauri shell with commands, events, capabilities, tray, hotkeys, single instance and the quit flow) is implemented; its open follow-ups are in `docs/implementation/ROADMAP.md`.
Stage 2b (the Svelte UI: unlock, create and open, the three-pane shell, editors, history and bin, generator, settings window, CSV import, tray panel, themes, uk/ru/en/de) has landed; its manual checks live in `RUN.md`.
Stage 3 (the browser bridge: `mypass-bridge`, the `mypass-host` native messaging host, the Settings Browser pane with pairing and save proposals, and the extension moved into `extension/` with its suites in CI) is implemented apart from the owner's manual checks, which live in `RUN.md`.
Stage 4 (passkeys: the WebAuthn create and get ceremonies in `mypass_core::passkey`, the bridge `PasskeyService`, the app commands and toggle, and the passkey UI) is implemented apart from the owner's manual checks, which live in `RUN.md`.
Stage 5 (iCloud primary storage and relocation, secondary-folder sync, device unlock, per-ceremony passkey presence, launch at login, update checks, signed arm64 packaging and release tooling) is implemented apart from the owner's manual checks in `RUN.md`.
Stage 6 (Windows 10 22H2 and Windows 11, x64: DPAPI stores, registry browser integration, events,
secure clipboard, input layout, launch at login, shell/UI, unsigned ZIP and multi-platform releases)
is implemented apart from the owner checks W1 to W17 in `RUN.md`. Windows Hello is deferred.
Native desktop behavior remains unverified; Windows CI lint/test/app/package coverage does not
prove desktop acceptance. `windows-integration` stays repository-variable-gated and skipped unless
explicitly enabled.
Later stages and deferred release work are listed in `docs/implementation/ROADMAP.md`.

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
  to install) and the extension suites (`scripts/test-extension.sh`: every `extension/tests/*.test.js` in its own `node` process, `N suites, M failed`; `--list` prints the discovered files). `--no-js` is only for a local machine without
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
- `crosscompat` runs on Ubuntu with Python 3.12 and CI-only `pykeepass==4.2.0`, installed in one step
  from `crates/mypass-core/tests/crosscompat/requirements.txt` (the single pin list of the job's Python
  packages): Rust writes / Python reads, then Python writes / Rust reads. It is path-gated on pull requests through
  `changes.outputs.crosscompat` and always runs on main pushes and manual dispatch.
  The env-gated Rust test prints a skip locally unless `MYPASS_PYKEEPASS_VAULT` is set.
  It also runs the passkey oracle (`crates/mypass-core/tests/crosscompat/passkey_oracle_check.sh`, packages
  pinned in `crates/mypass-core/tests/crosscompat/requirements.txt`): the `write_passkey_vectors` example writes our
  create and get responses and a vault; `python3 -I passkey_oracle.py` verifies them with Yubico `fido2`
  (`register_complete`, `authenticate_complete`), `pykeepass` and `cryptography`, and writes a KeePassXC-style vault
  that the env-gated `tests/passkey_pykeepass.rs` reads back. The oracle only verifies; the known-answer vectors are
  pinned in `tests/passkey_vectors.rs`. Locally the script skips with a message when python or a package is absent;
  with `CI=true` (always set in the job) a skip is a failure.
- `js` runs on Ubuntu with Node from `.nvmrc` (`actions/setup-node`): `bash scripts/test.sh --ci
  --only js`, i.e. `node --test scripts/` (the xcstrings converter and the catalog completeness
  tests) and `scripts/test-extension.sh` (the extension suites); built-in runner, no dependencies, no package install. It always runs and must be `success`.
- `ui` runs on Ubuntu with pnpm (`pnpm/action-setup`, `version` equal to `packageManager`) and Node from
  `.nvmrc` with the pnpm cache: `bash scripts/test.sh --ci --only ui`. It has no rust-cache (it never
  runs cargo), always runs and must be `success`.
- `package-smoke` is path-gated on pull requests and runs on main/manual builds: it packages and
  verifies an explicitly unsigned arm64 development build without signing secrets.
- `windows-app` needs `ui` and always builds the production x64 Windows UI, app and native
  messaging sidecar, checks PE architecture/subsystems and host provenance, and runs the app tests.
- `windows-package` is gated by the `package` path filter on pull requests and always runs on
  main/manual builds. It runs offline package tests, builds the unsigned x64 ZIP, verifies and
  extracts it, checks a framed `app_not_running` response from the shipped host, and uploads the
  ZIP and checksum for 7 days. No Windows signing credentials are used.
- `windows-integration` runs only when the repository variable `MYPASS_WINDOWS_INTEGRATION_TESTS`
  is `true`, otherwise it is skipped. The job sets the environment value to `1` and runs statically
  ignored native tests serially through the inventory/accounting runner; an empty selection fails.
  Ordinary Windows tests never opt into real DPAPI, registry or clipboard integration.
- `ci` is the aggregator and the required status check. It needs every other job (`changes`, `lint`,
  `test`, `js`, `ui`, `windows-integration`, `crosscompat`, `package-smoke`, `windows-app`,
  `windows-package`); a gated job may be `skipped`, every other result
  (`lint` and `test` included) must be `success`. A job added later must be wired into it
  (pinned by `ci_aggregator_needs_every_job`).
- `release.yml` runs only for version tags: `release-macos` creates the signed DMG, extension ZIP
  and `SHA256SUMS.txt`; `release-windows` follows it and validates/builds the unsigned Windows ZIP
  and its `.zip.sha256`. `publish` needs both successful producers, downloads exactly five assets
  and verifies both checksum manifests before publishing last. Native OS integration is not run
  by the release job. Apple temporary keychain cleanup remains unconditional; no Windows signing.
  For the first alpha, the owner runs `git tag v0.7.1-alpha.1 && git push origin v0.7.1-alpha.1`
  from the approved version bump. The workflow publishes it as a prerelease without setting Latest
  and requires `APPLE_SIGNING_IDENTITY`, `APPLE_CERTIFICATE` and `APPLE_CERTIFICATE_PASSWORD` for macOS.
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
  return `Err` and never panic. User-initiated open, create, unlock and backup validation use the
  standard local KDF limits, including for iCloud files and the migration source. Automatic
  reloads after an external change or save race, secondary sync, iCloud conflict versions and
  migration destination validation use an additional budget. Each budget dimension is the
  larger of the default ceiling and the already authenticated vault's cost for that dimension.
  The default ceilings are 1 GiB Argon2 memory, 4 GiB of memory times iterations, and 300 million
  AES-KDF rounds. Argon2 lane limits are those of the local format. During an explicit iCloud
  open or unlock, the selected main file establishes the budget for conflict versions. After a
  backup restore, readback uses the validated backup's cost as its baseline.
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
  the session is dirty). A changed file within the automatic KDF budget is authenticated and
  adopted wholesale by a clean session (nothing written) or two-way merged into a dirty session
  and then written. The remote save settings are kept, along with remote metadata unless edited
  locally. An over-budget replacement produces a dedicated error and leaves the live contents
  and any pending edits intact. On explicit unlock, reconciliation uses the local-open limits.
  `flush` is `Ok` only when every edit is on disk. `lock` saves first and keeps the encrypted bytes when that
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
- **Windows paths and secrets.** Known folders resolve data under `%LOCALAPPDATA%\MyPass`
  (backups, drafts, host, webview2 and `secrets/shared`, `secrets/app`); settings live under
  `%APPDATA%\MyPass`. `MYPASS_WINDOWS_DATA_ROOT` is a debug-only absolute root override for
  isolated tests; release builds ignore it. All locations still come from `AppPaths`.
  DPAPI was chosen over Credential Manager to support bounded file records and protected atomic
  replacement. Current-user DPAPI uses `CRYPTPROTECT_UI_FORBIDDEN`, never machine scope, with
  distinct public entropy `MyPass.shared.v1` and `MyPass.app.v1`; entropy is domain separation,
  not same-user isolation. Protected current-user/SYSTEM DACLs and ownership are checked by
  handle; reparse points and multiply-linked records are refused. Invalid existing ACLs fail
  closed without falling back to plaintext or silently repairing permissions. Recovery is in
  `RUN.md` W6; host Repair cannot repair secret-store ACLs. Legacy plaintext endpoints are
  discarded and freshly published, never trusted or imported. Windows has no device unlock,
  no Windows Hello and no iCloud backend; ordinary master-password unlock remains available.
- **Windows bridge policy.** Before hello, the host checks the exact IPv4 loopback listener PID
  and its current-user SID; HMAC proof still precedes token transmission. This is a same-user
  peer policy, not publisher authentication. Same-user tampering and PID/port reuse remain risks.
  Registry host registration uses both 32-bit and 64-bit views of
  `HKCU\Software\Google\Chrome\NativeMessagingHosts\com.mypass.chromehost` and
  `HKCU\Software\Microsoft\Edge\NativeMessagingHosts\com.mypass.chromehost` unconditionally;
  `HKCU\Software\Chromium\NativeMessagingHosts\com.mypass.chromehost` requires an installed
  marker. Brave/Vivaldi/Opera fallback is unverified. HKLM is never written. The shared manifest
  points to `%LOCALAPPDATA%\MyPass\host\mypass-host.exe`, copied from `mypass-native-host.exe`.
  Startup publishes the endpoint/server before unconditional background hash repair/registration;
  mapped-image replacement is bounded and preserves the old bytes on failure. Removal touches
  only verified owned registry values/files, suppresses automatic registration for this run,
  and registers again next launch; explicit Repair restores immediately. Removal is settings-only.
- **Windows desktop.** Clipboard history/cloud exclusion formats precede CF_UNICODETEXT.
  `clear_if_unchanged` checks a per-write keyed content digest of UTF-16 text and our exclusion
  marker under the same open clipboard lock; sequence numbers never authorize clearing alone
  (Windows can synthesize formats on close). An identical re-copy retaining our marker may clear.
  Win+V/cloud exclusion needs owner observation. Lock/suspend and conservative display-off events
  feed the scheduler. Launch at login uses the owned MyPass value in
  `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, a quoted executable and `--autostarted`.
  StartupApproved stays OS-owned: any present/unknown record conservatively requires approval;
  saved intent stays on and moved paths heal without clearing Task Manager disable state.
  Autostart hides to tray only if the tray exists. The custom title bar has no Windows 11 maximize
  snap-layout flyout; keyboard and edge snapping remain available. WebView2 preflight uses the
  safe loader API and a localized native error before runtime creation.
- **Update asset selection.** Only supported OS/architecture pairs advertise updates: macOS arm64
  requires `MyPass-X.Y.Z-macos-arm64.dmg` plus `SHA256SUMS.txt`; Windows x64 requires
  `MyPass-X.Y.Z-windows-x64.zip` plus `MyPass-X.Y.Z-windows-x64.zip.sha256`. A missing payload or
  checksum suppresses availability. Checks inspect asset names, not downloaded checksum contents;
  the banner opens the validated release page and never downloads or installs binaries.
- **File dialogs.** The webview never supplies a path for an attachment, an attachment export or
  a CSV import. The dialogs open from Rust behind `mypass_app_lib::files::FilePicker`: picked
  attachments come back as single-use random tokens that `entry_save` swaps for the real path, the
  CSV chosen for a preview is remembered for one `import_csv_commit`, and `attachment_export`
  opens the save dialog itself. Only vault open and create take a path from the UI. Capabilities
  grant no `fs`, `shell`, `http`, `clipboard-manager` or `global-shortcut` permission to any window
  (pinned by `capabilities_grant_no_fs_shell_or_clipboard_read`).
  A build manifest and per-window command ACLs grant each window only its UI commands.
- **Shell.** Decisions are plain functions, the Tauri glue is thin: `shell.rs` (settings effects, close
  and quit rules, login-item reconcile, `WindowOps`), `tray.rs` (menu model and ids), `hotkeys.rs`
  (plan, `HotkeyRegistry` seam), `lifecycle.rs` (platform events into auto-lock, the quit flow,
  the external-change watcher, `ShellEffects`, tray refresh) and `bridge.rs` (the browser bridge: pairing window and unlock-focus rules, start and quit, over the `BridgeEvents` seam; `commands/browser.rs` holds the `browser_*` commands) and `i18n.rs` (the shared catalogs, `{0}`
  substitution, English fallback) and `navigation.rs` (a webview may navigate only to the app's own origin); only `shell_runtime.rs` touches the runtime. Webviews hold
  `core:event:default`, so no Rust listener acts on a webview-emittable event: every trigger is
  a Rust call (session observer, platform sink, menu, tray, window or run-event callback). Tray
  icon and menus are built in Rust only (no `core:tray`/`core:menu` permission). Work for the main
  thread is posted with `run_on_main_thread` and never awaited; callbacks that would lock hand the
  lock to the scheduler (`lock_via_scheduler`). The quit flow (first, when the main window reported a dirty
  editor through `app_set_editor_dirty`, the Quit / Cancel prompt `confirm_discard_edits`, where
  Cancel changes nothing and shows the main window; then `on_exit_requested`: `begin_quit`, then
  `flush_blocking(5 s)`, then `quit_decision`, Cancel re-opens the session) runs on a
  worker because it blocks on a dialog: `ExitRequested` is always prevented and a confirmed exit
  calls `exit` again through `ExitGate`. The effect runner is called off the main thread under the
  settings gate and never takes it.
- **Languages.** The app ships `en`, `uk`, `ru` and `de`: the UI catalogs `src/lib/i18n/locales/<lang>.json`
  (one flat key-to-ICU-message map per language, the same key set and placeholders in every file, every key
  referenced from a `t("literal")` call in `src/` or listed as native-side in `src/i18n.audit.test.ts`), the
  macOS `src-tauri/resources/macos/<lang>.lproj/InfoPlist.strings` bundled through `tauri.macos.conf.json`
  and `CFBundleLocalizations` (`src-tauri/Info.plist`, verified by `scripts/release.py`), and the
  `mypass_session::settings::Language` setting (`system` or one language, typed into `bindings.ts`). The
  fallback chain is the chosen language, then English, then the key itself, in the webview (`t`) and in the
  native shell (`src-tauri/src/i18n.rs`, which embeds the same JSON catalogs). `system` maps the first
  system locale by its primary subtag (`uk*`, `ru*`, `de*` such as de-DE, de-AT, de-CH; anything else is
  English) in both `resolveSystemLanguage` and `i18n::language_for_locale`. A new language means all of those
  places plus the guards. A catalog key nothing references is deleted from every catalog
  (`every_catalog_key_is_referenced`), so the key set only holds live strings. The extension keeps its own
  catalogs.
- **UI.** The Svelte UI talks only through the generated `src/lib/bindings.ts`; tests run on a fake backend
  (`src/test/fakeBackend.ts`) that is type-checked against those bindings. Secrets held by stores are
  cleared on close, save and lock, and the single `{@html}` site renders sanitised markdown. The main
  window reports whether its editor holds unsaved changes (`app_set_editor_dirty`), and the quit flow
  asks before discarding them.
  `browser_proposal_take` is the one command that hands a browser's proposed password to the webview, once, after
  the user opened the proposal; like typed input it may land in an encrypted editor draft. `browser_status` never
  returns a secret or a hash.
- **Browser bridge.** Three hops: the extension talks to `mypass-host` (native messaging, one host process per
  message), the host to the app's loopback server (`127.0.0.1`, `mypass_bridge::server`, thread per connection, no async
  runtime). The host never links the session, core or KDBX crates. Each launch draws a fresh token and port and
  publishes them as an endpoint record in the team-restricted Keychain store for signed macOS
  builds (0600 `AppPaths::secrets_shared` file store for unsigned development); the pairing hashes
  live in the app-only Keychain and the host can never read them. A connection
  starts with a handshake: the host sends a hello with a fresh nonce, the server answers with an HMAC proof over
  the token, and only after the host verified it does the host send a token-bearing request, so a squatter on a stale
  port never receives the token. Unauthenticated connections have an absolute deadline. The gate order is quitting,
  pairing, then locked. Gating `match`es stay exhaustive (no wildcard arm), so a new request type is a compile error
  until it is classified. Team-signed macOS builds store the endpoint in a Keychain item
  trusting the app and host team requirements and register the host inside the running bundle.
  Before sending a token-bearing request the signed host resolves the loopback listener PID via
  libproc and checks its app code requirement with Security guest-by-pid. Unsigned development
  uses the 0600 file store and the Application Support host copy (diagnostics: `developmentFile`),
  retaining the same-user endpoint/host-replacement exposure. Signed-build residuals: PID reuse
  between lookup and verification, and the check-then-create Keychain ACL race; the earlier shared
  Keychain access-group idea was dropped.
  The popup starts pairing automatically and can pair while the vault is locked. A lock transition dismisses
  the code. Paired popups can unlock through the existing app-core password and macOS device-unlock operations.
  From the fifth wrong password the bridge applies an in-memory, monotonic backoff starting at 30 seconds and
  doubling up to 900 seconds. A successful unlock clears it. Popup device unlock permits one prompt
  in flight and ignores requests within two seconds of the last prompt start. macOS enforces its
  own authentication lockout. The popup clears the password field on submit.
  The native host relays raw JSON values through zeroizing buffers, inserting its token without
  decoding the password. The app decodes popup passwords directly into a zeroizing string and
  passes them to the session in a `SecretString`. Serde can retain temporary scratch copies of
  escaped passwords, and browser, OS and standard I/O buffers can retain copies. Logs and reply
  details carry no master password.
  The host handles `launchApp` locally, before the relay handshake. On macOS it prefers the bundle containing
  the registered host. For an installed host copy, the app writes `app-location.json` in `AppPaths::host_dir`.
  On macOS, the host checks the bundle's Info.plist for the MyPass bundle identifier. A same-user process can
  replace the record with a path to another executable with an accepted MyPass name or a bundle claiming
  that identifier, which the host could then launch.
  - **Residual risks.** Pairing while locked requires the code shown by the app and can proceed
    without the master password. Anyone with access to the unlocked OS session can pair another
    browser profile. That profile can access vault data after the owner unlocks and can try
    passwords at the bridge's throttled rate.
- **Passkeys.** `mypass_core::passkey` holds the pure parts: request parsing and origin validation (`https` only,
  the rpId must be a registrable suffix of the origin host through the Public Suffix List, no IP literals, no
  localhost exception, so passkeys never work on `localhost`), the ES256 ceremonies (`perform_create`,
  `perform_get`; the key is a `PasskeySigningKey` that is neither `Clone` nor `Debug`), the hand-written emit-only CBOR
  encoder and the KeePassXC-compatible `PasskeyRecord` (the `KPEX_PASSKEY_*` custom fields; the private key is a
  PKCS#8 PEM in a protected field). One `has_passkey` predicate owned by core decides what counts as a passkey for the
  category, counts and badges. The bridge `PasskeyService` (`mypass_bridge::passkeys`) runs two phases: phase one
  (`passkeyCreate`, `passkeyGet`) only lists candidates and parks a pending item in the pending table (120 s TTL,
  16 per kind, no key material); phase two (`passkeyCreateConfirm`, `passkeyGetApprove`) re-resolves everything inside
  one session call. A lock bumps the service epoch, so a phase one that raced a lock never parks, and every phase two
  starts with `vault_matches(db)` (the root UUID recorded in phase one) before any check or write. A create is durable
  before the response (`flush`), and a failed flush rolls the entry back. The `passkey_proxy_enabled` setting (default
  on) detaches the extension's WebAuthn proxy when off. Locking does not detach it: while MyPass is locked the
  proxy stays as it was, and the bridge gate answers every passkey request `locked`, so the page gets
  `NotAllowedError` "MyPass is locked".
  - Stage 5 closed the missing per-ceremony UV/presence check and broad webview command access:
    per-window ACLs now restrict every command to its intended windows. Every macOS phase two
    verifies device-owner presence before taking the session lock. The
    localized prompt names the validated rpId; cancellation and timeout give `NotAllowedError`.
    UV is set only when that ceremony's presence check succeeded; a platform without presence
    refuses `userVerification: required` before key access. UP stays 1. Client extensions are
    `credProps` only, the counter stays 0, and attestation stays `none` (packed self-attestation
    for `direct`, `indirect` and `enterprise`).
  - Device unlock uses ordinary Keychain items plus LocalAuthentication and biometric-domain
    invalidation; a `SecAccessControl(biometryCurrentSet)` upgrade remains deferred.
  - **Residual risks (owner-accepted).**
    3. The pending table fails closed: an evicted or expired item answers `NotAllowedError`. Two copies of the same
       vault share a root UUID, so `vault_matches` cannot tell them apart; the lock observer clears pending items on a
       switch, which leaves only a narrow window.
    4. Any get with no matching passkey answers `NotAllowedError` immediately, with or without `allowCredentials`,
       while a get with a match waits for the user in the popup. The website itself sees that difference (an
       immediate rejection against a promise that stays pending), without a user gesture, so a relying party can
       silently probe whether this browser's MyPass holds a passkey for it and, with `allowCredentials`, test a
       given credential id. Candidate rows reuse the stage 3 summaries (title, user name, non-protected fields); credential ids
       and user handles never leave the entry.
    5. While the proxy is attached, hardware keys and other providers are unavailable to Chrome, also while MyPass
       is locked (the proxy stays attached and every request fails with "MyPass is locked"; only the toggle
       detaches it), and a conflicting proxy-owning extension surfaces as `attachment-error`. Passkeys work only on `https` sites.
- **Signing.** Stage 5 uses the stable team `W645B93PPQ` with an Apple Development certificate,
  explicit app/host identifier-plus-team designated requirements and hardened runtime. The release
  workflow signs but does not notarize (owner policy Q1); INSTALL.txt documents quarantine removal.
  Developer ID signing and notarization remain deferred. Never sign ad-hoc: it breaks the
  Keychain access that guards the unlock key.
- **Manual checks and the old app.** The manual checks of every stage live in `RUN.md` (Ukrainian). Never run the old
  Swift MyPass side by side with this app, and open only a copy of a real vault.

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
never translated). Literal Cyrillic is also allowed in the extension files `extension/shared/{form-detection,page-identifier}.js` and `extension/tests/{formDetection,inlineIcon,pageIdentifier}.test.js` (localized keyword lists and their tests; the extension is vendored as copied), and in the translation files
`src/lib/i18n/locales/{uk,ru}.json` and `src-tauri/resources/macos/{uk,ru}.lproj/InfoPlist.strings`
(exactly these four of the translation files; the English and German files stay Cyrillic-free, and the Node tests read their expectations
from the catalogs), in the extension's `chrome.i18n` catalogs `extension/_locales/{uk,ru}/messages.json` (the `en` and `de`
catalogs stay Cyrillic-free), in `crates/mypass-kdbx/tests/unicode.rs`,
vendored upstream data under `crates/mypass-core/resources/`, and opaque binary fixtures
`crates/mypass-kdbx/tests/fixtures/*.kdbx` (fixture README and generator files follow the text rules).
Pinned by `cyrillic_only_in_allowed_files`.

## Commits and pull requests

Conventional Commits with a lowercase imperative subject and no trailing period, for example
`feat(kdbx): read the outer header`. Scopes: `core`, `kdbx`, `ci`, `docs`, `repo`, `ui`, `i18n` (later stages add
`bridge`, `platform`, `host`, `app`, `extension`). No AI-attribution footers and no co-author
trailers on commits or pull request text. A dependency version bump is its own `chore(deps)` commit. Dependabot covers cargo, npm and GitHub
Actions: npm minor and patch bumps arrive as one grouped pull request and each major as its own, but the `@tauri-apps/*`
packages are ignored there and bumped by hand together with the Rust `tauri` family (the CLI rejects mismatched minors).

## Rust rules

- No `unsafe` in our crates (`unsafe_code = "forbid"` through `[workspace.lints]`), with one
  exception: `mypass-platform` carries its own `[lints]` tables, an exact copy of the workspace
  tables except `unsafe_code = "deny"`, and only the allow-listed native glue files of that crate
  lift it with a module-level `#![allow(unsafe_code)]` for the Objective-C, Carbon and Security
  calls: `src/macos.rs`, `src/macos_login_item.rs`, `src/macos_device_unlock.rs`,
  `src/macos_icloud.rs`, `src/macos_keychain_acl.rs`, `src/macos_peer_process.rs` and
  `src/windows.rs` (all Win32 unsafe operations). Every `unsafe` block there is minimal, wrapped in a small safe
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
`/dist/`, `/src-tauri/binaries/`, `/src-tauri/gen/`, `.DS_Store`, `.worktrees/`, `*.profraw`), vault files with real data, and secrets.

## Real data

Never test against the owner's real vault. Manual checks use a copy.

## Manual checks

Manual-only checks (signed-build, Windows, real-vault) are walked by the owner in daily use and never block a merge or a release; a task whose only open work is a manual check is complete once its automated part is green.
