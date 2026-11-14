# Roadmap: unfinished work

**Status file.** This lists only work that is still open. Remove an item once it is implemented and
its tests pass, or, for manual-only items, once the manual check has actually been walked. History
lives in `git log`, not here.

Last updated: 2026-11-14

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

## Stage 2a backend

- [ ] `VaultFile`: reject non-regular files (FIFO/device symlink targets) at open and cap the
  encoded vault read size, once stage 2 fixes a size policy.
- [ ] `mypass-platform` macOS bodies that still return `Unsupported`: device unlock (Touch ID and
  device password) and launch at login. System events, the
  concealed clipboard, the keyboard layout and the Keychain secret stores already ship.

## Stage 2a follow-ups

- [ ] Per-window command access: every window (main, settings, tray panel) can call every app
  command (`app_quit` and `app_show_main` included); add per-window command permissions through a
  `tauri_build` app manifest so the tray panel and settings window get only what they use. This
  includes `entry_passkey_details` (it returns a passkey's credential id, so only the main window
  should call it) and the `browser_*` commands: `browser_proposal_take` returns a proposal's password, so only
  the main window should be able to call it (and `browser_pairing_cancel`); `browser_status`,
  `browser_revoke`, `browser_reset_pairings`, `browser_reinstall_host` and
  `browser_show_extension_folder` are used by the Settings Browser pane, so they belong to the
  settings window (and `browser_status`, which carries the live pairing code, to the main window too)
  and not to the tray panel.
- [ ] Startup failure is reported only on stderr (invisible in a Windows release build and when
  launched from Finder); show a native dialog.
- [ ] Quit-dialog default button: with tauri-plugin-dialog 2.8.4 a closed dialog is reported as the
  cancel-slot button, so "Quit anyway" has to be the first (Return) button for Esc to never quit; a
  custom dialog would let Cancel be the default.
- [ ] Manual: confirm that Cmd+Q and logout/restart raise `ExitRequested` on macOS, and session end
  on Windows (checklist in `RUN.md`).
- [ ] `secondaryFolder` in settings is a string from the webview; whatever later reads it (Stage 5
  sync) must treat it as untrusted.
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
  save flows, lock behaviour, crash and quit, other browsers, iCloud guard) on a copy of the owner's
  vault and record the results in the PR.
- [ ] `PairingManager`: add one atomic "clear all" (a single save plus the cache). `reset_pairings` on a
  running bridge revokes one browser at a time, so an error midway leaves a partial reset.
- [ ] Before any `connectNative` use: host handshake lazily on the first frame, reconnect with a fresh
  handshake, never resend a frame already written.
- [ ] `mypass-host` dev-dependencies enable `mypass-bridge` with the `server` feature, so `cargo test`
  and `cargo build --workspace` produce a `mypass-host` with the vault crates linked; the shipped
  binary (`cargo build -p mypass-host` in `scripts/build-host.sh`) does not. Check the shipped
  binary's dependency tree in CI, and let the e2e suite use that binary.
- [ ] A failed `LoopbackServer::start` (for example a `secrets/shared` folder with group or other
  bits) shows `listening: false` with no reason and no action in the Browser pane; surface an
  "endpoint store unusable" state.

## Stage 4 passkeys

- [ ] Manual: walk the Stage 4 passkey checklist in `RUN.md` (P1 to P13, on a public https demo site,
  with a copy of the owner's vault and KeePassXC interop) and record the results in the PR.

## Stage 5 macOS release

- [ ] iCloud Drive as a primary vault location with conflict merging and migration.
- [ ] "Also sync to folder" secondary destination (`mypass-sync`).
- [ ] Touch ID and device-password unlock, launch at login, and the daily update check.
- [ ] Move the bridge endpoint from the 0600 file store into a Keychain item that trusts only the
  host binary inside the signed app bundle, verified against a code requirement pinned to the team
  (`anchor apple generic and certificate leaf[subject.OU] = "W645B93PPQ"` plus the host
  identifier); never trust a path under Application Support. Delete-then-add on write (a crash in
  between only loses the per-launch endpoint), then delete the file.
  Until then a same-user process that captures the pairing secret can also write passkeys: it can
  rewrite the passkey fields of any https entry it can name and churn entry history with
  unbounded confirm calls (each Replace snapshots the entry, bounded only by HistoryMaxItems).
- [ ] The native messaging manifests point at a host copy under Application Support, which a
  same-user process can replace (the same accepted risk as the 0600 endpoint file). Point the
  manifests at the host inside the signed app bundle (`Contents/MacOS/mypass-native-host`, the Tauri sidecar, named so it cannot replace cargo's `mypass-host`, which Chrome
  accepts as any absolute path) or verify the copy's signature before registering.
- [ ] Signed macOS packaging with a stable team (DMG, extension zip, checksums, tag-driven release
  workflow).
- [ ] Manual checklist against a copy of the owner's vault and the cutover checklist from the old
  Swift app.

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
- [ ] Apply background sync merges locally. The merged result of a background sync is discarded, so
  entries another device added reach the remote but only appear locally at the next unlock. Apply and
  publish the merged result without replacing newer local edits made while the sync ran.

## Vault features

- [ ] Change master password (re-key an open vault).
