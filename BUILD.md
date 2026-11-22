# Building and testing MyPass

This file covers building and testing the workspace. For what MyPass is, see
[`README.md`](README.md); for how to submit a change, see [`CONTRIBUTING.md`](CONTRIBUTING.md);
to report a vulnerability, see [`SECURITY.md`](SECURITY.md); coding agents read
[`AGENTS.md`](AGENTS.md).

## Prerequisites

You need [rustup](https://rustup.rs). The toolchain is pinned in `rust-toolchain.toml` (with
`rustfmt` and `clippy`) and rustup installs it on first use. `bash` is needed for
`scripts/test.sh`. Node.js (the version in `.nvmrc`) is needed for the `js` part of `scripts/test.sh`, which runs the
i18n converter and catalog tests and the Chromium extension suites (`scripts/test-extension.sh`, Node 26 or newer) with the built-in Node runners (no packages to install);
`cargo test --workspace` itself needs no Node. pnpm (the version in `packageManager` of `package.json`) is needed for the `ui` part and for working on
the frontend; without it `scripts/test.sh` says `ui: NOT RUN`, and `--ci` fails instead.

On Windows, run `cargo` commands from PowerShell and `scripts/*.sh` from Git Bash.

## Crates

| Crate | Status | Role |
|---|---|---|
| `mypass-kdbx` | implemented | KDBX4 engine: format pipeline, model, database API, vault file |
| `mypass-core` | implemented | TOTP, generator and password rules, Public Suffix List, entry matcher, merge engine |
| `repo-guards` | implemented | Test-only guard tests for the workspace, CI, docs and scripts (never shipped) |
| `mypass-platform` | implemented | OS glue behind traits (system events, secure clipboard, secret stores, device unlock, native host registrar, launch at login, keyboard layout) and `AppPaths`; fakes behind the `test-util` feature, `Unsupported` fallbacks where a platform body has not landed |
| `mypass-sync` | implemented | Secondary-folder sync and iCloud storage, offline cache and conflict merging |
| `mypass-update` | implemented | GitHub release checks and persisted banner dismissal; no automatic installation |
| `mypass-session` | implemented | Vault session, settings, scheduling and the command facade; no Tauri dependency |
| `src/` (Svelte UI) | implemented (stage 2b) | The webview UI: typed API wrapper, stores, i18n, design kit, screens and the three windows (main, settings, tray panel), with its Vitest suites |
| `src-tauri` (`mypass-app`) | implemented | The Tauri 2 app shell and the typed IPC bindings (`src/lib/bindings.ts`, generated) |
| `mypass-bridge`, `mypass-host` | implemented | Browser bridge (protocol, loopback server, pairing) and the native messaging host; `scripts/build-host.sh` builds the host and `scripts/dev-app.sh` runs the app with it |
| `extension/` | implemented | The Chromium extension (Manifest V3 JavaScript) with its Node suites, run by `scripts/test-extension.sh` |

## Build and check

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
pnpm install --frozen-lockfile   # once, and after a dependency change
pnpm check                       # svelte-check (warnings are errors)
pnpm build                       # production bundle into dist/
pnpm tauri dev                   # run the app with hot reload
```

## Test

```bash
bash scripts/test.sh [--ci] [--only rust|ui|js] [--no-js]   # the full suite
cargo test --workspace                                        # Rust tests only
pnpm test                                                     # UI tests only (Vitest, jsdom)
cargo test -p mypass-app export_bindings_regenerate -- --ignored   # regenerate src/lib/bindings.ts
cargo test -p mypass-kdbx --test roundtrip fifty_megabyte_attachment_round_trips -- --ignored  # opt-in 50 MiB round trip
cargo test -p <crate> <filter>                                # one crate, one filter
cargo run -p mypass-kdbx --example write_crosscompat -- <out.kdbx>   # synthetic fixture
```

- `scripts/test.sh` is the one entry point for people, agents and CI. Without `--only` it runs every
  part (`rust`, `ui`, `js`); a part with nothing to run yet prints that it was skipped. The `rust`
  part runs `cargo test --workspace --no-fail-fast`, so one failing test binary does not hide
  failures in the later ones.
- `--ci` runs cargo with `--locked`, requires every tool of every part that exists and rejects
  `--no-js`.
- The `ui` part runs `pnpm install --frozen-lockfile`, `pnpm check`, `pnpm test` and `pnpm build`.
- The `js` part runs `node --test scripts/`: `convert-xcstrings.test.mjs` (the converter) and
  `i18n-completeness.test.mjs` (the committed catalogs: every key in `uk` and `ru`, matching
  placeholders, plural `other` branches, InfoPlist strings). `npm run i18n:test` is the same command.
  The same part then runs `scripts/test-extension.sh`: every `extension/tests/*.test.js` in its own node
  process, ending with `N suites, M failed`; `--list` prints the suite files.
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
  `js` (the `js` part on Ubuntu), `ui` (the `ui` part on Ubuntu: frozen pnpm install, check, tests, build), `crosscompat`, path-gated `package-smoke`, `windows-app`, path-gated `windows-package` and a variable-gated Windows integration suite as
  parallel jobs, joined by the `ci`
  aggregator.
- The `crosscompat` CI job (Rust writes and Python reads, then the reverse) uses `pykeepass`. The
  env-gated Rust test prints a skip locally unless `MYPASS_PYKEEPASS_VAULT` is set.

Run `bash scripts/test.sh` before you call a change done; CI runs after the fact and is not a
substitute.

## macOS packaging and releases

The release tools and their offline script tests require Python 3.11 or newer
(`tomllib` from the standard library); CI selects Python 3.12 explicitly.

The app version comes from `src-tauri/Cargo.toml`. Tauri's configuration omits
`version`. The release scripts accept `X.Y.Z` and `X.Y.Z-alpha.N`, `-beta.N` or
`-rc.N`. `scripts/check-version.sh [VERSION]` checks the app, npm package, extension
manifest (and extension package if versioned), macOS plist and Cargo.lock entry.
The app and release filenames show full SemVer. Both macOS bundle version fields
and Chromium's manifest `version` use the numeric core. Chromium displays the
full prerelease through `version_name` when the extension is loaded for manual
testing. Other workspace crates keep their internal versions.

```bash
bash scripts/bump-version.sh 0.7.1-alpha.1  # validates and refreshes Cargo.lock offline
bash scripts/check-version.sh
bash scripts/package-macos.sh --allow-unsigned --dry-run
bash scripts/package-macos.sh --allow-unsigned --out /path/to/empty-output
# Choose the valid certificate SHA-1, never its ambiguous display name:
security find-identity -v -p codesigning
MYPASS_SIGNING_IDENTITY=<40-hex-SHA-1> bash scripts/package-macos.sh --out /path/to/empty-output
# Release additionally requires a clean tree at exactly v<version>:
MYPASS_SIGNING_IDENTITY=<40-hex-SHA-1> bash scripts/package-macos.sh --release --out /path/to/empty-output
```

Packaging builds `aarch64-apple-darwin` only, with minimum macOS 14.0. It stages a
production-only extension, builds the host sidecar, builds Tauri's `app` target,
re-signs inside out with explicit team-based designated requirements, verifies
both binaries, then uses `hdiutil` to create a DMG containing the app, Applications
link, INSTALL.txt and both license files. No sandbox or entitlements are needed;
hardened runtime is required for signed app and host. The team is W645B93PPQ and
the identifiers are `com.mypass.app` and `com.mypass.chromehost`.

No ad-hoc signing is allowed. With no certificate, `--allow-unsigned` explicitly
opts into a development build (`--no-sign`, linker seals only). `--release` always
fails without a 40-hex identity. `--dry-run` validates metadata and release/tag
preconditions but writes nothing and never builds, signs, imports a certificate
or creates a DMG. To compile locally without any packaging/signing use
`pnpm tauri build --no-bundle --no-sign` (build the host first if needed).

`verify-bundle.sh <app> --host-source <staged-sidecar> --extension-zip <zip>` checks
strict/deep codesign verification, exact designated requirements in Apple’s
`csreq` canonical rendering, team, runtime,
entitlements, plist metadata, localizations, notices, host byte provenance and the
extension archive against the bundled extension. Only explicit `--allow-unsigned`
permits linker seals; those builds cannot prove signed verification. The staged
host is `src-tauri/binaries/mypass-native-host-aarch64-apple-darwin`, produced from
`cargo build -p mypass-host --release --target aarch64-apple-darwin`. The dependency
check rejects session/core/KDBX/update code in the host's normal dependency tree.

The output is exactly `MyPass-X.Y.Z-macos-arm64.dmg`,
`MyPass-chrome-extension-X.Y.Z.zip` and `SHA256SUMS.txt`. The checksum file lists the
two payloads with relative names (it cannot hash itself). Extension ZIP entries
are sorted, have fixed timestamps and modes, and exclude tests and package.json.
The installed app copies the bundled extension into a versioned writable folder
before revealing it; Chrome must never load unpacked files from the signed bundle.

The separate `.github/workflows/release.yml` runs only on version tags. Configure
these GitHub Actions repository secrets before tagging:

- `APPLE_CERTIFICATE`: base64-encoded PKCS#12 export of the **Apple Development**
  signing certificate and private key for team W645B93PPQ.
- `APPLE_CERTIFICATE_PASSWORD`: the export password for that PKCS#12 file.
- `APPLE_SIGNING_IDENTITY`: the valid certificate's exact 40-hex SHA-1 hash.

All three secrets are required. The workflow imports them into a temporary
keychain, deletes the keychain and certificate file even after failure, runs
`bash scripts/test.sh --ci` before packaging, uploads a draft, downloads and checks
its exact artifact set and hashes, then rechecks in the final publish job after Windows production succeeds. A stale
draft may be replaced; an already published release is refused. No release was
published merely by building locally. The `package-smoke` CI job packages an
explicitly unsigned development build without secrets on relevant pull requests.

Per the owner's release policy, notarization is disabled even if unrelated Apple
credentials are present in the environment. Users follow INSTALL.txt's
`xattr -dr com.apple.quarantine` instructions. Developer ID signing/notarization,
Intel/universal builds and `.kdbx` association are future work. The app icon (the old app's
green key) is drawn by `node scripts/make-app-icons.mjs`
in a light and a dark variant; the bundle icons are then generated with
`pnpm tauri icon src-tauri/icons/source.png -o <tmp>`, copying only the files
`tauri.conf.json` lists plus `icon.png` into `src-tauri/icons/`. The running app shows
the dark variant in the Dock (macOS) and on its windows (Windows) while the effective
theme is dark.

The Rust test entry point also runs `scripts/tests/*.test.sh` (offline version,
packaging and verification tests). Regenerate dependency notices with
`python3 crates/mypass-kdbx/tests/crosscompat/update_notices.py`; `--check` verifies
that the committed table matches Cargo metadata without editing it.

Before a release, run `bash scripts/check-version.sh`, commit the version change,
merge it through the normal review process, then create and push `vX.Y.Z` when
publishing is authorized. The workflow has no manual-dispatch release path.
For the first alpha, run these commands from the approved commit containing
the `0.7.1-alpha.1` bump:

```bash
bash scripts/check-version.sh --tag v0.7.1-alpha.1
git tag v0.7.1-alpha.1 && git push origin v0.7.1-alpha.1
```

The workflow publishes alpha, beta and release candidate tags as GitHub
prereleases without setting Latest. The macOS job requires the three Apple
secrets listed above. Windows ships an unsigned ZIP and requires no signing
secrets. Stable builds check GitHub's latest stable release. Prerelease builds
rank the first 20 GitHub release entries by SemVer, ignoring drafts, and can
offer a newer prerelease or stable.

Signed installation, certificate renewal, quarantine removal, release failure
paths and cutover remain owner checks in `RUN.md`; local script tests do not
establish Gatekeeper acceptance or real Keychain behavior.

## Windows builds, packaging and isolated integration

The supported target is `x86_64-pc-windows-msvc`, Windows 10 22H2 / Windows 11.
Install Visual Studio Build Tools with Desktop development with C++ and the Windows SDK,
plus Evergreen WebView2 Runtime. Use pinned rustup, Node and pnpm; run shell scripts in Git Bash.
Python 3.11+ handles ZIPs and hashes; 7-Zip is not required. Native ARM64, Authenticode,
an installer and Windows Hello are deferred.

```bash
bash scripts/build-windows.sh
bash scripts/package-windows.sh --out /path/to/empty-output
# Only from a clean tree at the exact authorized v<version> tag:
bash scripts/package-windows.sh --release --out /path/to/empty-output
```

The production build runs UI generation and explicitly targets MSVC. It checks x64 PE outputs,
GUI app / console host subsystems, actual frontend assets and staged-host byte provenance.
The root of `MyPass-X.Y.Z-windows-x64.zip` contains `MyPass.exe` (built as `mypass-app.exe`),
`mypass-native-host.exe`, `extension/`, `INSTALL.txt`, `LICENSE` and `THIRD_PARTY_NOTICES.md`.
The sibling `MyPass-X.Y.Z-windows-x64.zip.sha256` hashes that ZIP. INSTALL is ASCII/LF in
`scripts/package/INSTALL-windows.txt` and converted to CRLF only in the archive. Packaging is
unsigned and never invokes Apple signing. `--no-build --stage-from <directory>` is for verified
fixtures; `--release` requires a source build. Offline package tests require no 7z or Windows.

Known-folder data lives under `%LOCALAPPDATA%\MyPass`, settings under `%APPDATA%\MyPass`.
Debug builds alone accept `MYPASS_WINDOWS_DATA_ROOT` as the exact absolute config/data root;
release ignores it. Use disposable roots for tests, never a real vault or profile. The protected
DPAPI stores are `secrets\shared` and `secrets\app`; refused-ACL recovery is in RUN.md W6.

The repository variable `MYPASS_WINDOWS_INTEGRATION_TESTS=true` enables the separate CI job.
It sets `MYPASS_WINDOWS_INTEGRATION_TESTS=1`, `RUST_TEST_THREADS=1` and invokes
`bash scripts/test.sh --ci --only rust`. The native runner discovers statically ignored
`windows_integration` cases, checks nonempty selection and executed results, and runs serially.
Do not blanket-run all ignored tests: clipboard cases change clipboard content and registry/DPAPI
cases require isolation. This job remains skipped unless explicitly opted in. Native desktop,
minimum-version, real browser, missing-WebView2, Win+V and Modern Standby results remain W1-W17
owner checks; portable tests and cross-Clippy do not establish them.

CI `windows-app` always builds production app/sidecar after `ui`; `windows-package` runs on
package-relevant PRs and all main/manual runs, verifies/extracts the unsigned ZIP, executes a framed
`app_not_running` request against its shipped host and uploads ZIP/checksum for seven days.
Both feed required `ci`; the package job may be skipped when path-gated. Windows lint/test/app/package
have CI coverage; the variable-gated native integration suite is a separate validation boundary.

Tagged release flow is `release-macos` -> `release-windows` -> `publish`. Each producer runs its
validation; Windows has no signing credentials and does not opt into native OS integration.
The final job requires both successes, downloads and verifies exactly five files: the DMG,
extension ZIP, `SHA256SUMS.txt` (those two macOS payloads), Windows ZIP and its `.zip.sha256`.
It refuses missing/extra assets, bad hashes and published releases, then publishes last.
`scripts/publish-release.sh` also checks each gate's exit status. Local/offline checks do not prove
a live tagged release. Update banners require the matching OS/architecture payload and checksum
asset names and only open the release page; no automatic download or installation occurs.
