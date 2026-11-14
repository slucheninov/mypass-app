# Third-party notices

MyPass itself is licensed under the Apache License 2.0 (see `LICENSE`). This file lists the
third-party code and data that the MyPass build artifacts contain, in source or object form,
together with the licence notices those components require.

**Maintenance.** Add or update an entry here in the same change that adds, removes or bumps a
dependency, or re-vendors a file (see `CONTRIBUTING.md`). The Cargo tables are generated deterministically and reviewed; vendored notices are maintained manually.

RustCrypto `sha2` and `hmac` enable their upstream `zeroize` features to erase
intermediate hash state. Versions and licences are unchanged.

## Vendored data

### jsQR 1.4.0 (`extension/vendor/jsQR.js`)

- Upstream: https://github.com/cozmo/jsQR, commit `34d8eec1ec5d85496f3948ff02fcfe6406f89d81`
  ("1.4.0"); also published on npm as `jsqr@1.4.0`.
- Vendored file: `extension/vendor/jsQR.js` (Chromium extension). Its content is byte-identical to
  upstream `dist/jsQR.js` at the commit above, with one banner comment line prepended as the first
  line ("jsQR v1.4.0 - (c) Cosmo Wolfe, Apache License 2.0").
- Licence: Apache License 2.0 (the text is in `LICENSE` at the root of this repository). Upstream has
  no `NOTICE` file; its `LICENSE` is the unmodified Apache-2.0 text with the appendix template left
  unfilled.

### Apple password-manager-resources (`quirks/password-rules.json`)

- Upstream: https://github.com/apple/password-manager-resources
- Vendored file: `quirks/password-rules.json`, commit `6073487115b722596d3e1826d1b71b524b73bac5`
  (retrieved 2026-09-12, 436 entries at that commit). The file is committed as-is, with no
  reformatting at `crates/mypass-core/resources/password-rules.json`. Its verbatim provenance
  and licence are in `crates/mypass-core/resources/PASSWORD_RULES_PROVENANCE.md`; refresh with
  `scripts/update-password-rules.sh`.
- Licence: MIT. Text copied verbatim from upstream's `LICENSE.md` at the commit above:

~~~~text
Copyright 2020 - 2026 Apple Inc.

Permission is hereby granted, free of charge, to any person obtaining a copy of this software and associated documentation files (the "Software"), to deal in the Software without restriction, including without limitation the rights to use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of the Software, and to permit persons to whom the Software is furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.
~~~~

### Mozilla Public Suffix List

- Upstream: https://publicsuffix.org/ (source repository https://github.com/publicsuffix/list).
- Vendored file: `crates/mypass-core/resources/public_suffix_list.dat`, copied verbatim from
  the MyPassKit resource. Header VERSION: `2026-08-19_19-18-48_UTC`, COMMIT:
  `e8c9a2b2b2856b6449999dd0ec0d118f364ed0cd`. Includes ICANN and PRIVATE rules.
- Licence: Mozilla Public License 2.0, https://www.mozilla.org/en-US/MPL/2.0/.
  The original licence notice remains at the top of the resource. Source form is distributed
  in this repository at the path above, with no modifications.

## Linked crates

These tables include all transitive packages from `cargo metadata --locked --format-version 1`.
The release set is `cargo tree --locked --workspace --exclude repo-guards --target all -e normal`:
normal dependencies of the two production crates across supported and lockfile targets. This is
conservative about platform-specific packages. Remaining packages serve tests, dev dependencies,
repository guards or build tooling; they are not runtime dependencies of the production crates.
Workspace crates themselves are MyPass code under Apache-2.0 and are not third-party components.
Where a licence offers alternatives, MyPass selects MIT or Apache-2.0; in particular the optional
LGPL alternative is not selected. Crate archives contain the full component licence texts.

<!-- BEGIN GENERATED CRATES -->
<!-- Generate/verify: python3 crates/mypass-kdbx/tests/crosscompat/update_notices.py [--check] -->

### Shipped/linked in release

| Crate | Version | Licence |
|---|---|---|
| `Inflector` | 0.11.4 | BSD-2-Clause |
| `adler2` | 2.0.1 | 0BSD OR MIT OR Apache-2.0 |
| `aead` | 0.6.1 | MIT OR Apache-2.0 |
| `aes` | 0.9.4 | MIT OR Apache-2.0 |
| `aho-corasick` | 1.1.5 | Unlicense OR MIT |
| `alloc-no-stdlib` | 3.0.0 | BSD-3-Clause |
| `alloc-stdlib` | 0.3.0 | BSD-3-Clause |
| `anyhow` | 1.0.104 | MIT OR Apache-2.0 |
| `arboard` | 3.7.0 | MIT OR Apache-2.0 |
| `argon2` | 0.6.0 | MIT OR Apache-2.0 |
| `async-broadcast` | 0.7.2 | MIT OR Apache-2.0 |
| `async-channel` | 2.5.0 | Apache-2.0 OR MIT |
| `async-executor` | 1.14.1 | Apache-2.0 OR MIT |
| `async-io` | 2.6.0 | Apache-2.0 OR MIT |
| `async-lock` | 3.5.0 | Apache-2.0 OR MIT |
| `async-process` | 2.5.0 | Apache-2.0 OR MIT |
| `async-recursion` | 1.2.0 | MIT OR Apache-2.0 |
| `async-signal` | 0.2.14 | Apache-2.0 OR MIT |
| `async-task` | 4.7.1 | Apache-2.0 OR MIT |
| `async-trait` | 0.1.92 | MIT OR Apache-2.0 |
| `atk` | 0.18.2 | MIT |
| `atk-sys` | 0.18.2 | MIT |
| `atomic-waker` | 1.1.2 | Apache-2.0 OR MIT |
| `base16ct` | 1.0.0 | Apache-2.0 OR MIT |
| `base64` | 0.21.7 | MIT OR Apache-2.0 |
| `base64` | 0.23.1 | MIT OR Apache-2.0 |
| `base64ct` | 1.8.3 | Apache-2.0 OR MIT |
| `bit-set` | 0.11.1 | Apache-2.0 OR MIT |
| `bit-set` | 0.8.0 | Apache-2.0 OR MIT |
| `bit-vec` | 0.10.1 | Apache-2.0 OR MIT |
| `bit-vec` | 0.8.0 | Apache-2.0 OR MIT |
| `bitflags` | 1.3.2 | MIT/Apache-2.0 |
| `bitflags` | 2.13.2 | MIT OR Apache-2.0 |
| `blake2` | 0.11.0 | MIT OR Apache-2.0 |
| `block-buffer` | 0.10.4 | MIT OR Apache-2.0 |
| `block-buffer` | 0.12.1 | MIT OR Apache-2.0 |
| `block-padding` | 0.4.2 | MIT OR Apache-2.0 |
| `block2` | 0.6.2 | MIT |
| `blocking` | 1.7.0 | Apache-2.0 OR MIT |
| `brotli` | 9.1.1 | BSD-3-Clause AND MIT |
| `brotli-decompressor` | 6.3.1 | BSD-3-Clause/MIT |
| `bumpalo` | 3.20.3 | MIT OR Apache-2.0 |
| `bytemuck` | 1.25.2 | Zlib OR Apache-2.0 OR MIT |
| `byteorder` | 1.5.0 | Unlicense OR MIT |
| `bytes` | 1.12.1 | MIT |
| `cairo-rs` | 0.18.5 | MIT |
| `cairo-sys-rs` | 0.18.2 | MIT |
| `camino` | 1.2.6 | MIT OR Apache-2.0 |
| `cargo-platform` | 0.1.9 | MIT OR Apache-2.0 |
| `cargo_metadata` | 0.19.2 | MIT |
| `cbc` | 0.2.1 | MIT OR Apache-2.0 |
| `cesu8` | 1.1.0 | Apache-2.0/MIT |
| `cfb` | 0.14.0 | MIT |
| `cfg-if` | 1.0.5 | MIT OR Apache-2.0 |
| `chacha20` | 0.10.2 | MIT OR Apache-2.0 |
| `chacha20poly1305` | 0.11.0 | Apache-2.0 OR MIT |
| `cipher` | 0.5.2 | MIT OR Apache-2.0 |
| `clipboard-win` | 5.4.3 | BSL-1.0 |
| `cmov` | 0.5.4 | Apache-2.0 OR MIT |
| `combine` | 4.6.9 | MIT |
| `concurrent-queue` | 2.5.0 | Apache-2.0 OR MIT |
| `const-oid` | 0.10.2 | Apache-2.0 OR MIT |
| `cookie` | 0.18.2 | MIT OR Apache-2.0 |
| `core-foundation` | 0.10.1 | MIT OR Apache-2.0 |
| `core-foundation-sys` | 0.8.7 | MIT OR Apache-2.0 |
| `core-graphics` | 0.25.0 | MIT OR Apache-2.0 |
| `core-graphics-types` | 0.2.0 | MIT OR Apache-2.0 |
| `cpubits` | 0.1.1 | MIT OR Apache-2.0 |
| `cpufeatures` | 0.2.17 | MIT OR Apache-2.0 |
| `cpufeatures` | 0.3.1 | MIT OR Apache-2.0 |
| `crc32fast` | 1.5.4 | MIT OR Apache-2.0 |
| `crossbeam-channel` | 0.5.17 | MIT OR Apache-2.0 |
| `crossbeam-utils` | 0.8.23 | MIT OR Apache-2.0 |
| `crypto-bigint` | 0.7.11 | Apache-2.0 OR MIT |
| `crypto-common` | 0.1.7 | MIT OR Apache-2.0 |
| `crypto-common` | 0.2.2 | MIT OR Apache-2.0 |
| `cssparser` | 0.37.0 | MPL-2.0 |
| `cssparser` | 0.38.0 | MPL-2.0 |
| `cssparser-macros` | 0.7.1 | MPL-2.0 |
| `ctor` | 1.0.13 | Apache-2.0 OR MIT |
| `ctutils` | 0.4.8 | Apache-2.0 OR MIT |
| `darling` | 0.24.1 | MIT |
| `darling_core` | 0.24.1 | MIT |
| `darling_macro` | 0.24.1 | MIT |
| `data-encoding` | 2.11.1 | MIT |
| `dbus` | 0.9.12 | Apache-2.0/MIT |
| `der` | 0.8.3 | Apache-2.0 OR MIT |
| `deranged` | 0.5.9 | MIT OR Apache-2.0 |
| `derive_more` | 2.1.1 | MIT |
| `derive_more-impl` | 2.1.1 | MIT |
| `digest` | 0.10.7 | MIT OR Apache-2.0 |
| `digest` | 0.11.3 | MIT OR Apache-2.0 |
| `dirs` | 7.0.0 | MIT OR Apache-2.0 |
| `dirs-sys` | 0.5.0 | MIT OR Apache-2.0 |
| `dispatch2` | 0.3.1 | Zlib OR Apache-2.0 OR MIT |
| `displaydoc` | 0.2.7 | MIT OR Apache-2.0 |
| `dlopen2` | 0.8.2 | MIT |
| `dlopen2_derive` | 0.4.3 | MIT |
| `dom_query` | 0.28.0 | MIT |
| `dom_query` | 0.29.0 | MIT |
| `dpi` | 0.1.2 | Apache-2.0 AND MIT |
| `dtoa` | 1.0.11 | MIT OR Apache-2.0 |
| `dtoa-short` | 0.3.5 | MPL-2.0 |
| `dunce` | 1.0.5 | CC0-1.0 OR MIT-0 OR Apache-2.0 |
| `dyn-clone` | 1.0.20 | MIT OR Apache-2.0 |
| `ecdsa` | 0.17.0 | Apache-2.0 OR MIT |
| `elliptic-curve` | 0.14.3 | Apache-2.0 OR MIT |
| `embed_plist` | 1.2.2 | MIT OR Apache-2.0 |
| `endi` | 1.1.1 | MIT |
| `enumflags2` | 0.7.12 | MIT OR Apache-2.0 |
| `enumflags2_derive` | 0.7.12 | MIT OR Apache-2.0 |
| `equivalent` | 1.0.3 | Apache-2.0 OR MIT |
| `erased-serde` | 0.4.11 | MIT OR Apache-2.0 |
| `errno` | 0.3.14 | MIT OR Apache-2.0 |
| `error-code` | 3.4.0 | BSL-1.0 |
| `event-listener` | 5.4.2 | Apache-2.0 OR MIT |
| `event-listener-strategy` | 0.5.4 | Apache-2.0 OR MIT |
| `fastrand` | 2.5.0 | Apache-2.0 OR MIT |
| `fdeflate` | 0.3.7 | MIT OR Apache-2.0 |
| `ff` | 0.14.0 | MIT/Apache-2.0 |
| `field-offset` | 0.3.6 | MIT OR Apache-2.0 |
| `flate2` | 1.1.11 | MIT OR Apache-2.0 |
| `fnv` | 1.0.7 | Apache-2.0 / MIT |
| `foldhash` | 0.2.0 | Zlib |
| `foreign-types` | 0.5.0 | MIT/Apache-2.0 |
| `foreign-types-macros` | 0.2.4 | MIT/Apache-2.0 |
| `foreign-types-shared` | 0.3.1 | MIT/Apache-2.0 |
| `form_urlencoded` | 1.2.2 | MIT OR Apache-2.0 |
| `futures-channel` | 0.3.34 | MIT OR Apache-2.0 |
| `futures-core` | 0.3.34 | MIT OR Apache-2.0 |
| `futures-executor` | 0.3.34 | MIT OR Apache-2.0 |
| `futures-io` | 0.3.34 | MIT OR Apache-2.0 |
| `futures-lite` | 2.6.1 | Apache-2.0 OR MIT |
| `futures-macro` | 0.3.34 | MIT OR Apache-2.0 |
| `futures-sink` | 0.3.34 | MIT OR Apache-2.0 |
| `futures-task` | 0.3.34 | MIT OR Apache-2.0 |
| `futures-util` | 0.3.34 | MIT OR Apache-2.0 |
| `gdk` | 0.18.2 | MIT |
| `gdk-pixbuf` | 0.18.5 | MIT |
| `gdk-pixbuf-sys` | 0.18.0 | MIT |
| `gdk-sys` | 0.18.2 | MIT |
| `gdkwayland-sys` | 0.18.2 | MIT |
| `gdkx11` | 0.18.2 | MIT |
| `gdkx11-sys` | 0.18.2 | MIT |
| `generic-array` | 0.14.7 | MIT |
| `gethostname` | 1.1.0 | Apache-2.0 |
| `getrandom` | 0.2.17 | MIT OR Apache-2.0 |
| `getrandom` | 0.3.4 | MIT OR Apache-2.0 |
| `getrandom` | 0.4.3 | MIT OR Apache-2.0 |
| `gio` | 0.18.4 | MIT |
| `gio-sys` | 0.18.1 | MIT |
| `glib` | 0.18.5 | MIT |
| `glib-macros` | 0.18.5 | MIT |
| `glib-sys` | 0.18.1 | MIT |
| `glob` | 0.3.4 | MIT OR Apache-2.0 |
| `global-hotkey` | 0.8.0 | Apache-2.0 OR MIT |
| `gobject-sys` | 0.18.0 | MIT |
| `group` | 0.14.0 | MIT/Apache-2.0 |
| `gtk` | 0.18.2 | MIT |
| `gtk-sys` | 0.18.2 | MIT |
| `gtk3-macros` | 0.18.2 | MIT |
| `hashbrown` | 0.12.3 | MIT OR Apache-2.0 |
| `hashbrown` | 0.17.1 | MIT OR Apache-2.0 |
| `heck` | 0.4.1 | MIT OR Apache-2.0 |
| `heck` | 0.5.0 | MIT OR Apache-2.0 |
| `hermit-abi` | 0.5.3 | MIT OR Apache-2.0 |
| `hex` | 0.4.3 | MIT OR Apache-2.0 |
| `hmac` | 0.13.0 | MIT OR Apache-2.0 |
| `html5ever` | 0.39.0 | MIT OR Apache-2.0 |
| `html5ever` | 0.40.1 | MIT OR Apache-2.0 |
| `http` | 1.5.0 | MIT OR Apache-2.0 |
| `http-body` | 1.1.0 | MIT |
| `http-body-util` | 0.1.5 | MIT |
| `httparse` | 1.10.1 | MIT OR Apache-2.0 |
| `hybrid-array` | 0.4.15 | MIT OR Apache-2.0 |
| `hyper` | 1.12.0 | MIT |
| `hyper-util` | 0.1.21 | MIT |
| `ico` | 0.6.0 | MIT |
| `icu_collections` | 2.4.0 | Unicode-3.0 |
| `icu_locale_core` | 2.4.0 | Unicode-3.0 |
| `icu_normalizer` | 2.4.0 | Unicode-3.0 |
| `icu_normalizer_data` | 2.4.0 | Unicode-3.0 |
| `icu_properties` | 2.4.0 | Unicode-3.0 |
| `icu_properties_data` | 2.4.0 | Unicode-3.0 |
| `icu_provider` | 2.4.0 | Unicode-3.0 |
| `ident_case` | 1.0.1 | MIT/Apache-2.0 |
| `idna` | 1.1.0 | MIT OR Apache-2.0 |
| `idna_adapter` | 1.2.2 | Apache-2.0 OR MIT |
| `indexmap` | 1.9.3 | Apache-2.0 OR MIT |
| `indexmap` | 2.14.2 | Apache-2.0 OR MIT |
| `infer` | 0.22.0 | MIT |
| `inout` | 0.2.6 | MIT OR Apache-2.0 |
| `ipnet` | 2.12.2 | MIT OR Apache-2.0 |
| `is-docker` | 0.2.0 | MIT |
| `is-wsl` | 0.4.0 | MIT |
| `itoa` | 1.0.19 | MIT OR Apache-2.0 |
| `javascriptcore-rs` | 1.1.2 | MIT |
| `javascriptcore-rs-sys` | 1.1.1 | MIT |
| `jni` | 0.21.1 | MIT/Apache-2.0 |
| `jni-sys` | 0.3.1 | MIT OR Apache-2.0 |
| `jni-sys` | 0.4.1 | MIT OR Apache-2.0 |
| `jni-sys-macros` | 0.4.1 | MIT OR Apache-2.0 |
| `js-sys` | 0.3.107 | MIT OR Apache-2.0 |
| `json-patch` | 4.2.0 | MIT/Apache-2.0 |
| `jsonptr` | 0.7.1 | MIT OR Apache-2.0 |
| `keyboard-types` | 0.7.0 | MIT OR Apache-2.0 |
| `keyboard-types` | 0.8.3 | MIT OR Apache-2.0 |
| `libappindicator` | 0.9.0 | Apache-2.0 OR MIT |
| `libappindicator-sys` | 0.9.0 | Apache-2.0 OR MIT |
| `libc` | 0.2.191 | MIT OR Apache-2.0 |
| `libdbus-sys` | 0.2.7 | Apache-2.0/MIT |
| `libloading` | 0.7.4 | ISC |
| `libredox` | 0.1.25 | MIT |
| `linux-raw-sys` | 0.12.1 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT |
| `litemap` | 0.8.3 | Unicode-3.0 |
| `lock_api` | 0.4.14 | MIT OR Apache-2.0 |
| `log` | 0.4.35 | MIT OR Apache-2.0 |
| `markup5ever` | 0.39.0 | MIT OR Apache-2.0 |
| `markup5ever` | 0.40.0 | MIT OR Apache-2.0 |
| `memchr` | 2.8.3 | Unlicense OR MIT |
| `memoffset` | 0.9.1 | MIT |
| `mime` | 0.3.18 | MIT OR Apache-2.0 |
| `miniz_oxide` | 0.8.9 | MIT OR Zlib OR Apache-2.0 |
| `miniz_oxide` | 0.9.1 | MIT OR Zlib OR Apache-2.0 |
| `mio` | 1.2.4 | MIT |
| `muda` | 0.20.0 | Apache-2.0 OR MIT |
| `ndk` | 0.9.0 | MIT OR Apache-2.0 |
| `ndk-context` | 0.1.1 | MIT OR Apache-2.0 |
| `ndk-sys` | 0.6.0+11769913 | MIT OR Apache-2.0 |
| `new_debug_unreachable` | 1.0.6 | MIT |
| `num-conv` | 0.2.2 | MIT OR Apache-2.0 |
| `num-traits` | 0.2.19 | MIT OR Apache-2.0 |
| `num_enum` | 0.7.6 | BSD-3-Clause OR MIT OR Apache-2.0 |
| `num_enum_derive` | 0.7.6 | BSD-3-Clause OR MIT OR Apache-2.0 |
| `objc2` | 0.6.5 | MIT |
| `objc2-app-kit` | 0.3.2 | Zlib OR Apache-2.0 OR MIT |
| `objc2-cloud-kit` | 0.3.2 | Zlib OR Apache-2.0 OR MIT |
| `objc2-core-data` | 0.3.2 | Zlib OR Apache-2.0 OR MIT |
| `objc2-core-foundation` | 0.3.2 | Zlib OR Apache-2.0 OR MIT |
| `objc2-core-graphics` | 0.3.2 | Zlib OR Apache-2.0 OR MIT |
| `objc2-core-image` | 0.3.2 | Zlib OR Apache-2.0 OR MIT |
| `objc2-core-location` | 0.3.2 | Zlib OR Apache-2.0 OR MIT |
| `objc2-core-text` | 0.3.2 | Zlib OR Apache-2.0 OR MIT |
| `objc2-encode` | 4.1.0 | MIT |
| `objc2-exception-helper` | 0.1.1 | Zlib OR Apache-2.0 OR MIT |
| `objc2-foundation` | 0.3.2 | MIT |
| `objc2-quartz-core` | 0.3.2 | Zlib OR Apache-2.0 OR MIT |
| `objc2-ui-kit` | 0.3.2 | Zlib OR Apache-2.0 OR MIT |
| `objc2-user-notifications` | 0.3.2 | Zlib OR Apache-2.0 OR MIT |
| `objc2-web-kit` | 0.3.2 | Zlib OR Apache-2.0 OR MIT |
| `once_cell` | 1.21.4 | MIT OR Apache-2.0 |
| `open` | 5.4.5 | MIT |
| `option-ext` | 0.2.0 | MPL-2.0 |
| `ordered-stream` | 0.2.0 | MIT OR Apache-2.0 |
| `p256` | 0.14.0 | Apache-2.0 OR MIT |
| `pango` | 0.18.3 | MIT |
| `pango-sys` | 0.18.0 | MIT |
| `parking` | 2.2.1 | Apache-2.0 OR MIT |
| `parking_lot` | 0.12.5 | MIT OR Apache-2.0 |
| `parking_lot_core` | 0.9.12 | MIT OR Apache-2.0 |
| `password-hash` | 0.6.1 | MIT OR Apache-2.0 |
| `paste` | 1.0.15 | MIT OR Apache-2.0 |
| `pem-rfc7468` | 1.0.0 | Apache-2.0 OR MIT |
| `percent-encoding` | 2.3.2 | MIT OR Apache-2.0 |
| `phc` | 0.6.1 | Apache-2.0 OR MIT |
| `phf` | 0.13.1 | MIT |
| `phf` | 0.14.0 | MIT |
| `phf_generator` | 0.13.1 | MIT |
| `phf_generator` | 0.14.0 | MIT |
| `phf_macros` | 0.13.1 | MIT |
| `phf_macros` | 0.14.0 | MIT |
| `phf_shared` | 0.13.1 | MIT |
| `phf_shared` | 0.14.0 | MIT |
| `pin-project-lite` | 0.2.17 | Apache-2.0 OR MIT |
| `piper` | 0.2.5 | MIT OR Apache-2.0 |
| `pkcs8` | 0.11.0 | Apache-2.0 OR MIT |
| `plist` | 1.10.3 | MIT |
| `png` | 0.18.1 | MIT OR Apache-2.0 |
| `polling` | 3.11.0 | Apache-2.0 OR MIT |
| `poly1305` | 0.9.1 | Apache-2.0 OR MIT |
| `potential_utf` | 0.1.6 | Unicode-3.0 |
| `powerfmt` | 0.2.1 | MIT OR Apache-2.0 |
| `ppv-lite86` | 0.2.21 | MIT OR Apache-2.0 |
| `precomputed-hash` | 0.1.1 | MIT |
| `primefield` | 0.14.0 | Apache-2.0 OR MIT |
| `primeorder` | 0.14.0 | Apache-2.0 OR MIT |
| `proc-macro-crate` | 1.3.1 | MIT OR Apache-2.0 |
| `proc-macro-crate` | 2.0.2 | MIT OR Apache-2.0 |
| `proc-macro-crate` | 3.5.0 | MIT OR Apache-2.0 |
| `proc-macro-error` | 1.0.4 | MIT OR Apache-2.0 |
| `proc-macro-error-attr` | 1.0.4 | MIT OR Apache-2.0 |
| `proc-macro2` | 1.0.107 | MIT OR Apache-2.0 |
| `quick-xml` | 0.42.0 | MIT |
| `quote` | 1.0.47 | MIT OR Apache-2.0 |
| `r-efi` | 5.3.0 | MIT OR Apache-2.0 OR LGPL-2.1-or-later |
| `r-efi` | 6.0.0 | MIT OR Apache-2.0 OR LGPL-2.1-or-later |
| `rand` | 0.8.8 | MIT OR Apache-2.0 |
| `rand_chacha` | 0.3.1 | MIT OR Apache-2.0 |
| `rand_core` | 0.10.1 | MIT OR Apache-2.0 |
| `rand_core` | 0.6.4 | MIT OR Apache-2.0 |
| `raw-window-handle` | 0.6.2 | MIT OR Apache-2.0 OR Zlib |
| `redox_syscall` | 0.5.18 | MIT |
| `redox_users` | 0.5.3 | MIT |
| `regex` | 1.13.1 | MIT OR Apache-2.0 |
| `regex-automata` | 0.4.18 | MIT OR Apache-2.0 |
| `regex-syntax` | 0.8.11 | MIT OR Apache-2.0 |
| `reqwest` | 0.13.5 | MIT OR Apache-2.0 |
| `rfc6979` | 0.6.0 | Apache-2.0 OR MIT |
| `rfd` | 0.17.2 | MIT |
| `rustc-hash` | 2.1.3 | Apache-2.0 OR MIT |
| `rustix` | 1.1.6 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT |
| `rustversion` | 1.0.23 | MIT OR Apache-2.0 |
| `salsa20` | 0.11.0 | MIT OR Apache-2.0 |
| `same-file` | 1.0.6 | Unlicense/MIT |
| `schemars` | 0.8.22 | MIT |
| `schemars_derive` | 0.8.22 | MIT |
| `scopeguard` | 1.2.0 | MIT OR Apache-2.0 |
| `sec1` | 0.8.1 | Apache-2.0 OR MIT |
| `secrecy` | 0.10.3 | Apache-2.0 OR MIT |
| `security-framework` | 3.7.0 | MIT OR Apache-2.0 |
| `security-framework-sys` | 2.17.0 | MIT OR Apache-2.0 |
| `selectors` | 0.38.0 | MPL-2.0 |
| `selectors` | 0.41.0 | MPL-2.0 |
| `semver` | 1.0.28 | MIT OR Apache-2.0 |
| `serde` | 1.0.229 | MIT OR Apache-2.0 |
| `serde-untagged` | 0.1.9 | MIT OR Apache-2.0 |
| `serde_core` | 1.0.229 | MIT OR Apache-2.0 |
| `serde_derive` | 1.0.229 | MIT OR Apache-2.0 |
| `serde_derive_internals` | 0.29.1 | MIT OR Apache-2.0 |
| `serde_json` | 1.0.154 | MIT OR Apache-2.0 |
| `serde_repr` | 0.1.21 | MIT OR Apache-2.0 |
| `serde_spanned` | 0.6.9 | MIT OR Apache-2.0 |
| `serde_spanned` | 1.1.3 | MIT OR Apache-2.0 |
| `serde_with` | 3.24.0 | MIT OR Apache-2.0 |
| `serde_with_macros` | 3.24.0 | MIT OR Apache-2.0 |
| `serialize-to-javascript` | 0.1.2 | MIT OR Apache-2.0 |
| `serialize-to-javascript-impl` | 0.1.2 | MIT OR Apache-2.0 |
| `servo_arc` | 0.4.3 | MIT OR Apache-2.0 |
| `servo_arc` | 0.5.0 | MIT OR Apache-2.0 |
| `sha1` | 0.11.0 | MIT OR Apache-2.0 |
| `sha2` | 0.10.9 | MIT OR Apache-2.0 |
| `sha2` | 0.11.1 | MIT OR Apache-2.0 |
| `signal-hook-registry` | 1.4.8 | MIT OR Apache-2.0 |
| `signature` | 3.0.0 | Apache-2.0 OR MIT |
| `simd-adler32` | 0.3.10 | MIT |
| `siphasher` | 1.0.4 | MIT OR Apache-2.0 |
| `slab` | 0.4.12 | MIT |
| `smallvec` | 1.16.5 | MIT OR Apache-2.0 |
| `socket2` | 0.6.5 | MIT OR Apache-2.0 |
| `softbuffer` | 0.4.8 | MIT OR Apache-2.0 |
| `soup3` | 0.5.0 | MIT |
| `soup3-sys` | 0.5.0 | MIT |
| `specta` | 2.0.0-rc.26 | MIT |
| `specta-macros` | 2.0.0-rc.26 | MIT |
| `specta-serde` | 0.0.13 | MIT |
| `specta-typescript` | 0.0.13 | MIT |
| `specta-util` | 0.0.13 | MIT |
| `spki` | 0.8.1 | Apache-2.0 OR MIT |
| `stable_deref_trait` | 1.2.1 | MIT OR Apache-2.0 |
| `string_cache` | 0.11.0 | MIT OR Apache-2.0 |
| `string_cache` | 0.9.0 | MIT OR Apache-2.0 |
| `strsim` | 0.11.1 | MIT |
| `subtle` | 2.6.1 | BSD-3-Clause |
| `swift-rs` | 1.0.9 | MIT OR Apache-2.0 |
| `syn` | 1.0.109 | MIT OR Apache-2.0 |
| `syn` | 2.0.119 | MIT OR Apache-2.0 |
| `syn` | 3.0.10 | MIT OR Apache-2.0 |
| `sync_wrapper` | 1.0.2 | Apache-2.0 |
| `synstructure` | 0.14.0 | MIT |
| `sys-locale` | 0.3.2 | MIT OR Apache-2.0 |
| `tao` | 0.37.2 | Apache-2.0 |
| `tao-macros` | 0.1.4 | MIT OR Apache-2.0 |
| `tauri` | 2.12.5 | Apache-2.0 OR MIT |
| `tauri-codegen` | 2.7.4 | Apache-2.0 OR MIT |
| `tauri-macros` | 2.7.4 | Apache-2.0 OR MIT |
| `tauri-plugin-dialog` | 2.8.4 | Apache-2.0 OR MIT |
| `tauri-plugin-fs` | 2.7.1 | Apache-2.0 OR MIT |
| `tauri-plugin-global-shortcut` | 2.4.1 | Apache-2.0 OR MIT |
| `tauri-plugin-opener` | 2.7.2 | Apache-2.0 OR MIT |
| `tauri-plugin-single-instance` | 2.5.5 | Apache-2.0 OR MIT |
| `tauri-runtime` | 2.12.2 | Apache-2.0 OR MIT |
| `tauri-runtime-wry` | 2.12.3 | Apache-2.0 OR MIT |
| `tauri-specta` | 2.0.0-rc.26 | MIT |
| `tauri-specta-macros` | 2.0.0-rc.26 | MIT |
| `tauri-utils` | 2.10.2 | Apache-2.0 OR MIT |
| `tempfile` | 3.27.0 | MIT OR Apache-2.0 |
| `tendril` | 0.5.1 | MIT OR Apache-2.0 |
| `thiserror` | 1.0.69 | MIT OR Apache-2.0 |
| `thiserror` | 2.0.21 | MIT OR Apache-2.0 |
| `thiserror-impl` | 1.0.69 | MIT OR Apache-2.0 |
| `thiserror-impl` | 2.0.21 | MIT OR Apache-2.0 |
| `time` | 0.3.55 | MIT OR Apache-2.0 |
| `time-core` | 0.1.9 | MIT OR Apache-2.0 |
| `time-macros` | 0.2.32 | MIT OR Apache-2.0 |
| `tinystr` | 0.8.5 | Unicode-3.0 |
| `tokio` | 1.54.1 | MIT |
| `tokio-util` | 0.7.20 | MIT |
| `toml` | 1.2.1+spec-1.1.0 | MIT OR Apache-2.0 |
| `toml_datetime` | 0.6.3 | MIT OR Apache-2.0 |
| `toml_datetime` | 1.1.3+spec-1.1.0 | MIT OR Apache-2.0 |
| `toml_edit` | 0.19.15 | MIT OR Apache-2.0 |
| `toml_edit` | 0.20.2 | MIT OR Apache-2.0 |
| `toml_edit` | 0.25.19+spec-1.1.0 | MIT OR Apache-2.0 |
| `toml_parser` | 1.1.5+spec-1.1.0 | MIT OR Apache-2.0 |
| `toml_writer` | 1.2.0+spec-1.1.0 | MIT OR Apache-2.0 |
| `tower` | 0.5.3 | MIT |
| `tower-http` | 0.6.11 | MIT |
| `tower-layer` | 0.3.3 | MIT |
| `tower-service` | 0.3.3 | MIT |
| `tracing` | 0.1.44 | MIT |
| `tracing-attributes` | 0.1.31 | MIT |
| `tracing-core` | 0.1.36 | MIT |
| `tray-icon` | 0.25.1 | MIT OR Apache-2.0 |
| `try-lock` | 0.2.5 | MIT |
| `typeid` | 1.0.3 | MIT OR Apache-2.0 |
| `typenum` | 1.20.1 | MIT OR Apache-2.0 |
| `uds_windows` | 1.2.1 | MIT |
| `unicode-ident` | 1.0.27 | (MIT OR Apache-2.0) AND Unicode-3.0 |
| `unicode-segmentation` | 1.13.3 | MIT OR Apache-2.0 |
| `universal-hash` | 0.6.1 | MIT OR Apache-2.0 |
| `url` | 2.5.8 | MIT OR Apache-2.0 |
| `urlpattern` | 0.6.0 | MIT |
| `utf8_iter` | 1.1.0 | Apache-2.0 OR MIT |
| `utf_types` | 1.0.0 | Apache-2.0 OR MIT |
| `uuid` | 1.29.2 | Apache-2.0 OR MIT |
| `walkdir` | 2.5.0 | Unlicense/MIT |
| `want` | 0.3.2 | MIT |
| `wasi` | 0.11.1+wasi-snapshot-preview1 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT |
| `wasip2` | 1.0.4+wasi-0.2.12 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT |
| `wasm-bindgen` | 0.2.130 | MIT OR Apache-2.0 |
| `wasm-bindgen-futures` | 0.4.80 | MIT OR Apache-2.0 |
| `wasm-bindgen-macro` | 0.2.130 | MIT OR Apache-2.0 |
| `wasm-bindgen-macro-support` | 0.2.130 | MIT OR Apache-2.0 |
| `wasm-bindgen-shared` | 0.2.130 | MIT OR Apache-2.0 |
| `wasm-streams` | 0.5.0 | MIT OR Apache-2.0 |
| `web-sys` | 0.3.107 | MIT OR Apache-2.0 |
| `web-time` | 1.1.0 | MIT OR Apache-2.0 |
| `web_atoms` | 0.2.6 | MIT OR Apache-2.0 |
| `web_atoms` | 0.3.0 | MIT OR Apache-2.0 |
| `webkit2gtk` | 2.0.2 | MIT |
| `webkit2gtk-sys` | 2.0.2 | MIT |
| `webview2-com` | 0.39.1 | MIT |
| `webview2-com-macros` | 0.8.1 | MIT |
| `webview2-com-sys` | 0.39.1 | MIT |
| `winapi` | 0.3.9 | MIT/Apache-2.0 |
| `winapi-i686-pc-windows-gnu` | 0.4.0 | MIT/Apache-2.0 |
| `winapi-util` | 0.1.11 | Unlicense OR MIT |
| `winapi-x86_64-pc-windows-gnu` | 0.4.0 | MIT/Apache-2.0 |
| `window-vibrancy` | 0.8.1 | Apache-2.0 OR MIT |
| `windows` | 0.62.2 | MIT OR Apache-2.0 |
| `windows-collections` | 0.3.2 | MIT OR Apache-2.0 |
| `windows-core` | 0.62.2 | MIT OR Apache-2.0 |
| `windows-future` | 0.3.2 | MIT OR Apache-2.0 |
| `windows-implement` | 0.60.2 | MIT OR Apache-2.0 |
| `windows-interface` | 0.59.3 | MIT OR Apache-2.0 |
| `windows-link` | 0.2.1 | MIT OR Apache-2.0 |
| `windows-numerics` | 0.3.1 | MIT OR Apache-2.0 |
| `windows-result` | 0.4.1 | MIT OR Apache-2.0 |
| `windows-strings` | 0.5.1 | MIT OR Apache-2.0 |
| `windows-sys` | 0.45.0 | MIT OR Apache-2.0 |
| `windows-sys` | 0.59.0 | MIT OR Apache-2.0 |
| `windows-sys` | 0.61.2 | MIT OR Apache-2.0 |
| `windows-targets` | 0.42.2 | MIT OR Apache-2.0 |
| `windows-targets` | 0.52.6 | MIT OR Apache-2.0 |
| `windows-threading` | 0.2.1 | MIT OR Apache-2.0 |
| `windows-version` | 0.1.7 | MIT OR Apache-2.0 |
| `windows_aarch64_gnullvm` | 0.42.2 | MIT OR Apache-2.0 |
| `windows_aarch64_gnullvm` | 0.52.6 | MIT OR Apache-2.0 |
| `windows_aarch64_msvc` | 0.42.2 | MIT OR Apache-2.0 |
| `windows_aarch64_msvc` | 0.52.6 | MIT OR Apache-2.0 |
| `windows_i686_gnu` | 0.42.2 | MIT OR Apache-2.0 |
| `windows_i686_gnu` | 0.52.6 | MIT OR Apache-2.0 |
| `windows_i686_gnullvm` | 0.52.6 | MIT OR Apache-2.0 |
| `windows_i686_msvc` | 0.42.2 | MIT OR Apache-2.0 |
| `windows_i686_msvc` | 0.52.6 | MIT OR Apache-2.0 |
| `windows_x86_64_gnu` | 0.42.2 | MIT OR Apache-2.0 |
| `windows_x86_64_gnu` | 0.52.6 | MIT OR Apache-2.0 |
| `windows_x86_64_gnullvm` | 0.42.2 | MIT OR Apache-2.0 |
| `windows_x86_64_gnullvm` | 0.52.6 | MIT OR Apache-2.0 |
| `windows_x86_64_msvc` | 0.42.2 | MIT OR Apache-2.0 |
| `windows_x86_64_msvc` | 0.52.6 | MIT OR Apache-2.0 |
| `winnow` | 0.5.40 | MIT |
| `winnow` | 1.0.5 | MIT |
| `wit-bindgen` | 0.57.1 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT |
| `wnaf` | 0.14.1 | Apache-2.0 OR MIT |
| `writeable` | 0.6.4 | Unicode-3.0 |
| `wry` | 0.57.2 | Apache-2.0 OR MIT |
| `x11` | 2.21.0 | MIT |
| `x11-dl` | 2.21.0 | MIT |
| `x11rb` | 0.13.2 | MIT OR Apache-2.0 |
| `x11rb-protocol` | 0.13.2 | MIT OR Apache-2.0 |
| `xkeysym` | 0.2.1 | MIT OR Apache-2.0 OR Zlib |
| `yoke` | 0.8.4 | Unicode-3.0 |
| `yoke-derive` | 0.8.4 | Unicode-3.0 |
| `zbus` | 5.19.0 | MIT |
| `zbus_macros` | 5.19.0 | MIT |
| `zbus_names` | 4.3.5 | MIT |
| `zcheapstr` | 1.2.0 | MIT |
| `zerocopy` | 0.8.63 | BSD-2-Clause OR Apache-2.0 OR MIT |
| `zerocopy-derive` | 0.8.63 | BSD-2-Clause OR Apache-2.0 OR MIT |
| `zerofrom` | 0.1.8 | Unicode-3.0 |
| `zerofrom-derive` | 0.1.8 | Unicode-3.0 |
| `zeroize` | 1.9.1 | Apache-2.0 OR MIT |
| `zerotrie` | 0.2.5 | Unicode-3.0 |
| `zerovec` | 0.11.9 | Unicode-3.0 |
| `zerovec-derive` | 0.11.7 | Unicode-3.0 |
| `zmij` | 1.0.23 | MIT |
| `zvariant` | 5.15.1 | MIT |
| `zvariant_derive` | 5.15.1 | MIT |
| `zvariant_utils` | 4.2.0 | MIT |

### Test/dev only (including build tooling and target-only lock entries)

| Crate | Version | Licence |
|---|---|---|
| `android_system_properties` | 0.1.6 | MIT OR Apache-2.0 |
| `arrayvec` | 0.7.8 | MIT OR Apache-2.0 |
| `autocfg` | 1.5.1 | Apache-2.0 OR MIT |
| `base64` | 0.22.1 | MIT OR Apache-2.0 |
| `blake2b_simd` | 1.0.5 | MIT |
| `block-modes` | 0.9.1 | MIT OR Apache-2.0 |
| `borsh` | 1.8.1 | MIT OR Apache-2.0 |
| `borsh-derive` | 1.8.1 | Apache-2.0 |
| `bs58` | 0.5.1 | MIT/Apache-2.0 |
| `cargo_toml` | 1.0.1 | Apache-2.0 OR MIT |
| `cc` | 1.11.1 | MIT OR Apache-2.0 |
| `cfg-expr` | 0.15.8 | MIT OR Apache-2.0 |
| `cfg_aliases` | 0.2.2 | MIT |
| `chrono` | 0.4.45 | MIT OR Apache-2.0 |
| `ciborium` | 0.2.2 | Apache-2.0 |
| `ciborium-io` | 0.2.2 | Apache-2.0 |
| `ciborium-ll` | 0.2.2 | Apache-2.0 |
| `constant_time_eq` | 0.4.2 | CC0-1.0 OR MIT-0 OR Apache-2.0 |
| `core_detect` | 1.0.0 | MIT/Apache-2.0 |
| `crunchy` | 0.2.4 | MIT |
| `embed-resource` | 3.0.12 | MIT |
| `find-msvc-tools` | 0.1.15 | MIT OR Apache-2.0 |
| `half` | 2.7.1 | MIT OR Apache-2.0 |
| `hex-literal` | 1.1.0 | MIT OR Apache-2.0 |
| `iana-time-zone` | 0.1.65 | MIT OR Apache-2.0 |
| `iana-time-zone-haiku` | 0.1.2 | MIT OR Apache-2.0 |
| `jiff` | 0.2.42 | Unlicense OR MIT |
| `jiff-core` | 0.1.4 | Unlicense OR MIT |
| `jiff-static` | 0.2.42 | Unlicense OR MIT |
| `jiff-tzdb` | 0.1.9 | Unlicense OR MIT |
| `jiff-tzdb-platform` | 0.1.3 | Unlicense OR MIT |
| `keepass` | 0.17.0 | MIT |
| `objc2-io-surface` | 0.3.2 | Zlib OR Apache-2.0 OR MIT |
| `phf_codegen` | 0.13.1 | MIT |
| `phf_codegen` | 0.14.0 | MIT |
| `pkg-config` | 0.3.34 | MIT OR Apache-2.0 |
| `portable-atomic` | 1.15.0 | Apache-2.0 OR MIT |
| `portable-atomic-util` | 0.2.8 | Apache-2.0 OR MIT |
| `proptest` | 1.12.0 | MIT OR Apache-2.0 |
| `quick-error` | 1.2.3 | MIT/Apache-2.0 |
| `rand` | 0.10.3 | MIT OR Apache-2.0 |
| `rand_xorshift` | 0.5.0 | MIT OR Apache-2.0 |
| `ref-cast` | 1.0.27 | MIT OR Apache-2.0 |
| `ref-cast-impl` | 1.0.27 | MIT OR Apache-2.0 |
| `rust-argon2` | 3.0.0 | MIT/Apache-2.0 |
| `rustc_version` | 0.4.1 | MIT OR Apache-2.0 |
| `rusty-fork` | 0.3.1 | MIT/Apache-2.0 |
| `ryu` | 1.0.23 | Apache-2.0 OR BSL-1.0 |
| `schemars` | 0.9.0 | MIT |
| `schemars` | 1.2.3 | MIT |
| `serde_yaml_ng` | 0.10.0 | MIT |
| `serdect` | 0.4.3 | Apache-2.0 OR MIT |
| `shlex` | 2.0.1 | MIT OR Apache-2.0 |
| `static_vcruntime` | 3.0.0 | MIT OR Apache-2.0 OR Zlib |
| `string_cache_codegen` | 0.11.2 | MIT OR Apache-2.0 |
| `string_cache_codegen` | 0.6.1 | MIT OR Apache-2.0 |
| `system-deps` | 6.2.2 | MIT OR Apache-2.0 |
| `target-lexicon` | 0.12.16 | Apache-2.0 WITH LLVM-exception |
| `tauri-build` | 2.7.4 | Apache-2.0 OR MIT |
| `tauri-plugin` | 2.7.2 | Apache-2.0 OR MIT |
| `tauri-winres` | 0.3.7 | MIT |
| `tinyvec` | 1.13.3 | Zlib OR Apache-2.0 OR MIT |
| `toml` | 0.8.2 | MIT OR Apache-2.0 |
| `twofish` | 0.8.0 | MIT OR Apache-2.0 |
| `unarray` | 0.1.4 | MIT OR Apache-2.0 |
| `unsafe-libyaml` | 0.2.11 | MIT |
| `version-compare` | 0.2.1 | MIT |
| `version_check` | 0.9.5 | MIT/Apache-2.0 |
| `vswhom` | 0.1.0 | MIT |
| `vswhom-sys` | 0.1.3 | MIT |
| `wait-timeout` | 0.2.1 | MIT/Apache-2.0 |
| `winreg` | 0.56.0 | MIT |
| `zeroize_derive` | 1.5.0 | Apache-2.0 OR MIT |
| `zlib-rs` | 0.6.8 | Zlib |

<!-- END GENERATED CRATES -->

### IDNA and Unicode data

`url`, `idna` and `idna_adapter` use the ICU4X crates and their compiled Unicode data.
The `icu_*` family and other packages marked Unicode-3.0 above are covered by the Unicode
License v3 (https://www.unicode.org/license.txt); their crate archives include the licence.
This permissive data licence applies alongside MIT/Apache where the expression uses `AND`.

Unicode data licence text (from `icu_normalizer_data` 2.4.0 `LICENSE`):

~~~~text
UNICODE LICENSE V3

COPYRIGHT AND PERMISSION NOTICE

Copyright © 2020-2024 Unicode, Inc.

NOTICE TO USER: Carefully read the following legal agreement. BY
DOWNLOADING, INSTALLING, COPYING OR OTHERWISE USING DATA FILES, AND/OR
SOFTWARE, YOU UNEQUIVOCALLY ACCEPT, AND AGREE TO BE BOUND BY, ALL OF THE
TERMS AND CONDITIONS OF THIS AGREEMENT. IF YOU DO NOT AGREE, DO NOT
DOWNLOAD, INSTALL, COPY, DISTRIBUTE OR USE THE DATA FILES OR SOFTWARE.

Permission is hereby granted, free of charge, to any person obtaining a
copy of data files and any associated documentation (the "Data Files") or
software and any associated documentation (the "Software") to deal in the
Data Files or Software without restriction, including without limitation
the rights to use, copy, modify, merge, publish, distribute, and/or sell
copies of the Data Files or Software, and to permit persons to whom the
Data Files or Software are furnished to do so, provided that either (a)
this copyright and permission notice appear with all copies of the Data
Files or Software, or (b) this copyright and permission notice appear in
associated Documentation.

THE DATA FILES AND SOFTWARE ARE PROVIDED "AS IS", WITHOUT WARRANTY OF ANY
KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF
MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT OF
THIRD PARTY RIGHTS.

IN NO EVENT SHALL THE COPYRIGHT HOLDER OR HOLDERS INCLUDED IN THIS NOTICE
BE LIABLE FOR ANY CLAIM, OR ANY SPECIAL INDIRECT OR CONSEQUENTIAL DAMAGES,
OR ANY DAMAGES WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS,
WHETHER IN AN ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION,
ARISING OUT OF OR IN CONNECTION WITH THE USE OR PERFORMANCE OF THE DATA
FILES OR SOFTWARE.

Except as contained in this notice, the name of a copyright holder shall
not be used in advertising or otherwise to promote the sale, use or other
dealings in these Data Files or Software without prior written
authorization of the copyright holder.

SPDX-License-Identifier: Unicode-3.0

—

Portions of ICU4X may have been adapted from ICU4C and/or ICU4J.
ICU 1.8.1 to ICU 57.1 © 1995-2016 International Business Machines Corporation and others.

~~~~

## Synthetic test fixtures

`crates/mypass-kdbx/tests/fixtures/` copies the synthetic MyPassKit test assets
(KeePassXC KDBX4 outputs, pykeepass attachment output and its generator) from the earlier
MyPass project. They are MyPass test data/code under Apache-2.0, not third-party program code.
KeePassXC (GPL-2.0/GPL-3.0) and pykeepass 4.2.0 (GPL-3.0) were producing tools only;
neither is linked or shipped. Fixture passwords and contents are documented in that directory.

## Test-only (not part of any shipped artifact)

`repo-guards` is a test-only workspace crate (`publish = false`) and is never shipped. The
dev-only crates used by tests, the oracle and the repository guards (`proptest`, `keepass`, `toml`,
`serde_yaml_ng`, `ciborium`) are linked into test artifacts only, never shipped.

### Test-only KDBX oracle

`keepass` 0.17.0 (MIT, copyright Stefan Seemayer and contributors),
https://github.com/sseemayer/keepass-rs, is a dev-dependency only. Its
`save_kdbx4` feature checks interoperability in both directions; it is never
linked into a shipped artifact. The MIT licence permits use, copying,
modification and distribution subject to retaining its copyright and permission
notice; the full licence is included in the crate's `LICENSE` file.

## Merge property-test tooling (development only)

`proptest` 1.12.0 (MIT OR Apache-2.0) generates and shrinks deterministic merge cases.
It is a dev-dependency and is never linked into the shipped product.

## Atomic-write test tooling (development only)

`tempfile` 3.27.0 (MIT OR Apache-2.0) supplies isolated temporary directories for
workspace tests, including vault-file tests. It is a dev-dependency and is never linked
into the shipped product.

## Independent Python CI oracle

`pykeepass==4.2.0` (GPL-3.0) is CI-only test tooling, installed into the crosscompat job's
Python environment, never linked into or shipped with MyPass. Optional local verification uses
an external scratch venv. It is not a Cargo dependency. Upstream: https://github.com/libkeepass/pykeepass.

The passkey oracle (`crates/mypass-core/tests/crosscompat/passkey_oracle.py`, pins in
`crates/mypass-core/tests/crosscompat/requirements.txt`) adds two more CI-only Python packages next
to `pykeepass==4.2.0`, installed into the same crosscompat job environment and never linked into or
shipped with MyPass:

- `fido2==2.2.1` (Yubico python-fido2; BSD-2-Clause, Apache-2.0 or MPL-2.0, MyPass selects
  BSD-2-Clause). Upstream: https://github.com/Yubico/python-fido2.
- `cryptography==50.0.2` (Apache-2.0 OR BSD-3-Clause, MyPass selects BSD-3-Clause). Upstream:
  https://github.com/pyca/cryptography.

## npm packages (user interface)

This section is maintained by hand (the generated Cargo tables above are produced by
`update_notices.py` and never contain npm packages). Versions are the exact pins of `package.json`
and `pnpm-lock.yaml`; refresh the lists with `pnpm licenses list --prod` when a pin changes. Where a
licence offers alternatives, MyPass selects MIT or Apache-2.0; in particular the MPL-2.0 alternative
of DOMPurify is not selected.

### Shipped in the frontend bundle once imported (`dependencies`)

| Package | Version | Licence |
|---|---|---|
| `@tauri-apps/api` | 2.12.4 | Apache-2.0 OR MIT |
| `@tauri-apps/plugin-dialog` | 2.8.4 | MIT OR Apache-2.0 |
| `@tauri-apps/plugin-opener` | 2.7.2 | MIT OR Apache-2.0 |
| `dompurify` | 3.4.17 | MPL-2.0 OR Apache-2.0 |
| `intl-messageformat` | 12.1.4 | BSD-3-Clause |
| `@formatjs/fast-memoize` (via `intl-messageformat`) | 3.1.7 | MIT |
| `@formatjs/icu-messageformat-parser` (via `intl-messageformat`) | 3.5.22 | MIT |
| `@formatjs/icu-skeleton-parser` (via `intl-messageformat`) | 2.1.12 | MIT |
| `marked` | 18.1.1 | MIT |

The Svelte runtime (`svelte` 5.57.3, MIT, a development dependency because the compiler runs at
build time) is also copied into the bundle by the compiled components, together with the two
runtime dependencies it imports from `svelte/internal`:

| Package | Version | Licence |
|---|---|---|
| `clsx` (via `svelte`) | 2.1.1 | MIT |
| `esm-env` (via `svelte`) | 1.2.2 | MIT |

The other entries of Svelte's `dependencies` (`acorn`, `esrap`, `magic-string`, `devalue`, and so on)
belong to the compiler or to server-side rendering and are not in the bundle.
`@types/trusted-types` 2.0.7 (MIT) is a type-only dependency of DOMPurify and is not bundled.

`intl-messageformat` is under the New BSD licence, which requires its notice to be reproduced with
binary distributions:

~~~~text
Copyright (c) 2023, Oath Inc.

Licensed under the terms of the New BSD license. See below for terms.

Redistribution and use of this software in source and binary forms,
with or without modification, are permitted provided that the following
conditions are met:

- Redistributions of source code must retain the above
  copyright notice, this list of conditions and the
  following disclaimer.

- Redistributions in binary form must reproduce the above
  copyright notice, this list of conditions and the
  following disclaimer in the documentation and/or other
  materials provided with the distribution.

- Neither the name of Oath Inc. nor the names of its
  contributors may be used to endorse or promote products
  derived from this software without specific prior
  written permission of Oath Inc.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS
IS" AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED
TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A
PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT
OWNER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT
LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE,
DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY
THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
(INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
~~~~

### Development tooling (`devDependencies`, never shipped)

Used to build and test the frontend only; none of it is part of a MyPass artifact.

| Package | Version | Licence |
|---|---|---|
| `@sveltejs/vite-plugin-svelte` | 6.2.4 | MIT |
| `@tauri-apps/cli` | 2.12.4 | Apache-2.0 OR MIT |
| `@testing-library/svelte` | 5.4.2 | MIT |
| `@types/node` | 26.6.5 | MIT |
| `jsdom` | 30.1.3 | MIT |
| `svelte` | 5.57.3 | MIT |
| `svelte-check` | 4.7.7 | MIT |
| `typescript` | 5.9.3 | Apache-2.0 |
| `vite` | 6.4.5 | MIT |
| `vitest` | 3.2.7 | MIT |

Their own transitive dependencies are listed in `pnpm-lock.yaml`.

### Test-only npm packages (not shipped)

These are `devDependencies` used by the component tests; nothing from them is in the bundle.

| Package | Version | Licence |
|---|---|---|
| `axe-core` | 4.11.0 | MPL-2.0 |
| `@testing-library/user-event` | 14.6.1 | MIT |
