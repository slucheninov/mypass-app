# Roadmap: unfinished work

**Status file.** This lists only work that is still open. Remove an item once it is implemented and
its tests pass, or, for manual-only items, once the manual check has actually been walked. History
lives in `git log`, not here.

Last updated: 2026-11-20

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
- [ ] Before any `connectNative` use: host handshake lazily on the first frame, reconnect with a fresh
  handshake, never resend a frame already written.

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
- [ ] Explicit, race-free cleanup UI for retained `.displaced` copies (needs exact vault and copy
  identity persisted at write time, in-flight files named separately, and confirmation bound to the
  displayed ids and session). The surfacing itself is done: the Vault pane lists the copies read-only.
  These competing vault copies are deliberately never auto-deleted.
- [ ] Replacing a vault on the move-aside path (Windows and volumes without an atomic exchange)
  renames the current file to a working `.aside` name with a rename that replaces. If another
  program has put its own file under that exact name first, that file is overwritten. This
  predates stage 7. Fix it with a rename that never replaces (or a private reserved name) and
  fail closed where the volume cannot do that; test the collision with distinct bytes.
- [ ] Working-name leftovers (`.aside`, or the hidden temp name after an atomic exchange) are not
  listed in the sync status. The error now names the file when a failed replace strands the
  vault there, but a leftover from a crash, or from a working name that could not be removed
  after the copy was published, shows up only in the folder itself. List them read-only, apart
  from the real competing copies.
- [ ] The Vault pane's list of competing copies does not refresh while it stays open if a copy is
  kept by a save of the primary vault only. It refreshes when the pane opens, when the vault
  changes and when a sync finishes. Refresh it on a primary-only save too.
- [ ] macOS tray panel position: the click point is read when the click event is handled, not when
  it happened. If the pointer moves off the icon in between, the panel can open at the pointer's
  new spot (still on a real screen). Capture the point at event time if the tray API gives it,
  and check on a mixed-DPI, multi-monitor setup by hand.
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

- [ ] Manual: walk W1 to W17 in `RUN.md` on the owner's Windows machine using a vault copy,
  including the Windows 10 22H2 / Windows 11 support floor. Desktop behavior is unverified;
  passing Windows CI lint/test/app/package jobs do not establish desktop acceptance.
- [ ] Execute the opted-in native Windows integration suite in an isolated disposable session;
  `MYPASS_WINDOWS_INTEGRATION_TESTS` remains variable-gated and skipped by default.
- [ ] Windows Hello device unlock and per-ceremony user verification (currently unavailable;
  UV-required passkeys fail closed).
- [ ] Windows Authenticode code signing and an installer (current distribution is an unsigned ZIP).
- [ ] Native Windows ARM64 build and validation; x64 emulation is not an acceptance claim.
- [ ] A snap-layout friendly native title bar (custom maximize controls have no snap flyout).
- [ ] Add a documented Brave/Vivaldi/Opera key if the vendors publish one; fallback is unverified.

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
