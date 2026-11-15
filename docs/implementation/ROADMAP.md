# Roadmap: unfinished work

**Status file.** This lists only work that is still open. Remove an item once it is implemented and
its tests pass, or, for manual-only items, once the manual check has actually been walked. History
lives in `git log`, not here.

Last updated: 2026-11-15

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
- [ ] Zeroize protected field values in entry clones and history, not only when `Entry::set_field`
  replaces or removes one. Today every such `String` is freed without a wipe, so a passkey private
  key PEM (or password) can linger in freed memory: the old value `set_field` replaces, history
  copies, and the full `Entry` clones the passkey confirm keeps for its undo
  (`mypass-bridge/src/passkeys/create.rs`, `Undo.after` with the new key and, on Replace,
  `Undo.before` with the old one), alive from the write until the confirm answers.

## Stage 2a follow-ups

- [ ] Quit-dialog default button: with tauri-plugin-dialog 2.8.4 a closed dialog is reported as the
  cancel-slot button, so "Quit anyway" has to be the first (Return) button for Esc to never quit; a
  custom dialog would let Cancel be the default.
- [ ] Manual: confirm that Cmd+Q and logout/restart raise `ExitRequested` on macOS, and session end
  on Windows (checklist in `RUN.md`).
- [ ] Tray panel position uses the panel window's scale factor; use the scale factor of the monitor
  that holds the tray icon (mixed-DPI setups).

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
- [ ] Prune unused legacy catalog keys (418 at the time of writing) once stage 3 has ported its screens.
- [ ] npm dependencies are bumped by hand (exact pins); add an npm ecosystem with grouping to
  Dependabot, with the same cooldown as cargo.

## Stage 3 extension bridge

- [ ] Manual: walk the Stage 3 checklist in `RUN.md` (registration, pairing, endpoint file, autofill,
  save flows, lock behaviour, crash and quit, other browsers, old-app guard) on a copy of the owner's
  vault and record the results in the PR.
- [ ] `PairingManager`: add one atomic "clear all" (a single save plus the cache). `reset_pairings` on a
  running bridge revokes one browser at a time, so an error midway leaves a partial reset.
- [ ] Before any `connectNative` use: host handshake lazily on the first frame, reconnect with a fresh
  handshake, never resend a frame already written.
- [ ] A failed `LoopbackServer::start` (for example a `secrets/shared` folder with group or other
  bits) shows `listening: false` with no reason and no action in the Browser pane; surface an
  "endpoint store unusable" state.

## Stage 4 passkeys

- [ ] Manual: walk the Stage 4 passkey checklist in `RUN.md` (P1 to P13, on a public https demo site,
  with a copy of the owner's vault and KeePassXC interop) and record the results in the PR.

## Stage 5 macOS release

- [ ] Owner hardware check of iCloud Drive storage on two Macs (RUN.md S1 to S6): NSFileVersion
  conflict merging, download waits, file presenter, TCC prompts, sign-out and offline cache.
  A master password changed remotely while the offline cache is ahead, or a remotely deleted
  vault with cache-only edits, keeps the vault on the cache until the user resolves it.
- [ ] Manual: walk all Stage 5 S-checks and C1 to C10 in `RUN.md`, including real signed install,
  update/certificate-renewal Keychain survival, browser host registration, per-ceremony presence,
  per-window ACLs, startup dialog and the old Swift app cutover. These never block merge or release.
- [ ] Developer ID Application signing and notarization (Q1). Current releases use Apple Development
  team W645B93PPQ without notarization and require INSTALL.txt's quarantine-removal steps.
- [ ] Safe secondary-folder sync on FAT/exFAT: volumes without atomic exchange or hard links
  currently fail closed with unavailable/unsupportedFolder; never overwrite a competing vault.
- [ ] Surface retained `.displaced` copies in sync status and provide an explicit cleanup UI.
  These competing vault copies are deliberately never auto-deleted.
- [ ] Tighter KDF limits for remote/secondary opens: hostile-input limits exist, but expensive
  KDF work on an untrusted remote file can still consume substantial CPU/memory.
- [ ] Upgrade device-unlock storage to `SecAccessControl(biometryCurrentSet)` with a data-protection
  Keychain and provisioning profile; current ordinary Keychain + LocalAuthentication handling
  checks biometric domain state for Touch ID-only enrolments.
- [ ] Intel / universal macOS builds (the current DMG is arm64, macOS 14+).
- [ ] `.kdbx` file association and a validated OS path-open flow.
- [ ] Optional Sparkle-style in-app update installation. Current updates only show a banner and
  open the validated GitHub release page; no installer runs.
- [ ] Investigate the historical native coordinator test timeout root cause (H-flaky). The timeout
  was contained and subsequent parallel runs passed; the cause remains unconfirmed. Watch CI
  recurrence rather than treating serialization as proof of a fix.

## Stage 6 Windows

- [ ] Windows bodies of every `mypass-platform` trait (DPAPI secret stores, lock and suspend events;
  the clipboard history exclusion already ships through `arboard`).
- [ ] The Browser pane reports a broken pairing list on Windows only because `UnsupportedSecretStore`
  fails `PairingManager::new`; map `PlatformError::Unsupported` to a separate "not available on this
  platform" state until the DPAPI store lands.
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

## Vault features

- [ ] Change master password (re-key an open vault), including safe secondary rotation across
  sync runs and restart. A reused sync engine can merge its previously verified old-password
  secondary copy, but production creates a fresh engine for each sync run. An unreadable
  secondary therefore fails closed even without a restart, including when its ciphertext
  matches the checkpoint.
