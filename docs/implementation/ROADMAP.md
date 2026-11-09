# Roadmap: unfinished work

**Status file.** This lists only work that is still open. Remove an item once it is implemented and
its tests pass, or, for manual-only items, once the manual check has actually been walked. History
lives in `git log`, not here.

Last updated: 2026-11-08

---

## Stage 2a backend

- [ ] `VaultFile`: reject non-regular files (FIFO/device symlink targets) at open and cap the
  encoded vault read size, once stage 2 fixes a size policy.
- [ ] Tauri 2 shell that compiles on macOS and Windows, with tray, hotkeys, single instance and the
  quit gate.
- [ ] `mypass-platform`: OS glue behind traits (lock and sleep events, concealed clipboard, secret
  stores, launch at login, keyboard layout).
- [ ] `mypass-session`: the unlocked vault session with debounced atomic saves, concurrent-write
  merge, backups, auto-lock, clipboard clear, note drafts, entry and group operations, generator
  history, CSV import and `.kdbx` discovery.
- [ ] Typed Tauri command and event surface with generated TypeScript bindings, and the i18n catalog
  pipeline.

## Stage 2b UI

- [ ] Every screen of the app in Svelte 5: unlock, create, open and find vaults, the three-pane shell,
  entry editors, generator, settings, CSV import, tray panel, themes and localisation.

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

- [ ] Windows bodies of every `mypass-platform` trait (DPAPI secret stores, lock and suspend events,
  clipboard history exclusion).
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
- [ ] Per-vault backup folders. All vaults share one `backups/` folder today, so after a location
  change the Backups list can show, and restore, backups of a different vault. Use a folder per vault
  identity and migrate the existing flat backups.
- [ ] Note drafts after a restore. Drafts are keyed by vault path, so drafts written after the
  restored backup can be offered once the restored vault is unlocked. Decide whether to discard or
  flag them.
- [ ] Apply background sync merges locally. The merged result of a background sync is discarded, so
  entries another device added reach the remote but only appear locally at the next unlock. Apply and
  publish the merged result without replacing newer local edits made while the sync ran.

## Vault features

- [ ] Change master password (re-key an open vault).
