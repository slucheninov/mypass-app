# Roadmap: unfinished work

**Status file.** This lists only work that is still open. Remove an item once it is implemented and
its tests pass, or, for manual-only items, once the manual check has actually been walked. History
lives in `git log`, not here.

Last updated: 2026-11-13

---

## Stage 1 follow-ups

- [ ] Manual: confirm that saving a vault on an SMB or FUSE mounted volume tolerates a failing
  directory fsync. On Unix the atomic save treats a directory-sync `Unsupported` or `InvalidInput`
  error as success by design; any other directory-sync error is reported after the replacement has
  committed, and the caller must reload. Only real mounts can show which error they return.
- [ ] Migrate `rand` to 0.10 (generator API, `rand_core` unification). The public `generate` takes
  `R: CryptoRng + RngCore` and `generate_os` uses `rand::rngs::OsRng`; 0.10 moves
  `RngCore`, `OsRng` and `rand::Error`, and the lockfile still carries `rand_core` 0.6 next to 0.10,
  so check that the RustCrypto crates agree on one trait set first. Update the generator tests
  (`BoundaryRng`) and `THIRD_PARTY_NOTICES.md` in the same change.

## Stage 2a backend

- [ ] `VaultFile`: reject non-regular files (FIFO/device symlink targets) at open and cap the
  encoded vault read size, once stage 2 fixes a size policy.
- [ ] `mypass-platform`: OS glue behind traits (lock and sleep events, concealed clipboard, secret
  stores, launch at login, keyboard layout).
- [ ] `mypass-session`: the unlocked vault session with debounced atomic saves, concurrent-write
  merge, backups, auto-lock, clipboard clear, note drafts, entry and group operations, generator
  history, CSV import and `.kdbx` discovery.
- [ ] Typed Tauri command and event surface with generated TypeScript bindings, and the i18n catalog
  pipeline.

## Stage 2b UI follow-ups

- [ ] Manual: walk the Stage 2b checklist in `RUN.md` on a copy of the owner's vault (unlock, shell,
  editors, history and bin, generator, settings, CSV import, tray panel, quit flow, themes, languages).
- [ ] Restore from the bin to the original group: the UI restores to the root today because the backend
  keeps no origin; use the KDBX 4.1 `PreviousParentGroup` in `mypass-session`.
- [ ] Empty Deleted or deleting a parent group while one of its entries is open in the editor: define
  and test what the editor does (close, or keep and refuse to save).
- [ ] History values are not revealable: add a backend command for per-version reveal, then the UI.
- [ ] Generator Use is not recorded in the generator history (only Copy is).
- [ ] Reserved hotkeys are refused in the UI only; move the check into the backend hotkey parser.
- [ ] CSV import error causes are matched by their English message; add separate error codes
  (`csvMissingColumn`, `csvNotUtf8`, `csvEmpty`) instead.
- [ ] Per-window command ACL: `app_quit`, `app_show_main` and `app_set_editor_dirty` are callable from every window, including
  the tray panel and settings; restrict by window label where a window does not need them.
- [ ] Flaky test `keyboard_layout_off_main_without_a_run_loop_is_unknown_not_a_deadlock` in
  `mypass-platform` (timing; failed once in two full runs): make it deterministic.
- [ ] Prune unused legacy catalog keys (418 at the time of writing) once stage 3 has ported its screens.
- [ ] npm dependencies are bumped by hand (exact pins); add an npm ecosystem with grouping to
  Dependabot, with the same cooldown as cargo.

## Stage 3 extension bridge

- [ ] `mypass-bridge`: the browser protocol, loopback server, per-launch token and pairing.
- [ ] `mypass-host`: the native messaging host, with Keychain secret sharing between app and host and
  self-registration of native messaging manifests for all Chromium browsers.
- [ ] The extension moved into this repository as `extension/` with its Node suites running in CI.
- [ ] Browser settings pane with pairing and save proposals.

## Stage 4 passkeys

- [ ] WebAuthn create and get ceremonies (ES256) through the extension, stored in
  KeePassXC-compatible custom fields, with strict origin and relying-party validation and a
  consent-gated flow.
- [ ] Passkey UI: detail row, editor actions, Passkeys category and the app-side proxy toggle.

## Stage 5 macOS release

- [ ] iCloud Drive as a primary vault location with conflict merging and migration.
- [ ] "Also sync to folder" secondary destination (`mypass-sync`).
- [ ] Touch ID and device-password unlock, launch at login, and the daily update check.
- [ ] Signed macOS packaging with a stable team (DMG, extension zip, checksums, tag-driven release
  workflow).
- [ ] Manual checklist against a copy of the owner's vault and the cutover checklist from the old
  Swift app.

## Stage 6 Windows

- [ ] Windows bodies of every `mypass-platform` trait (DPAPI secret stores, lock and suspend events;
  the clipboard history exclusion already ships through `arboard`).
- [ ] Native messaging host registration through the registry, Windows shell behaviour and zip
  packaging.
- [ ] Windows jobs in CI and the release workflow, and the Windows-only integration suite behind
  `MYPASS_WINDOWS_INTEGRATION_TESTS`.
- [ ] Manual checklist on the owner's Windows machine.

## Carried over from the Swift app

- [ ] German localisation for the app, and extension `_locales` for any language (the extension copy
  is English only).
- [ ] Passkey AAGUID registry: the upstream registry PR (`passkey-authenticator-aaguids`, which needs a
  vector icon) submitted and merged.
- [ ] Google shows "MyPass" for newly created passkeys. Manual check; depends on Google ingesting
  the registry.
- [ ] Apply background sync merges locally. The merged result of a background sync is discarded, so
  entries another device added reach the remote but only appear locally at the next unlock. Apply and
  publish the merged result without replacing newer local edits made while the sync ran.

## Vault features

- [ ] Change master password (re-key an open vault).
