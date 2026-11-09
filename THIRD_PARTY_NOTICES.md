# Third-party notices

MyPass itself is licensed under the Apache License 2.0 (see `LICENSE`). This file lists the
third-party code and data that the MyPass build artifacts contain, in source or object form,
together with the licence notices those components require.

**Maintenance.** Add or update an entry here in the same change that adds, removes or bumps a
dependency, or re-vendors a file (see `CONTRIBUTING.md`). The Cargo tables are generated deterministically and reviewed; vendored notices are maintained manually.

## Vendored data

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
| `adler2` | 2.0.1 | 0BSD OR MIT OR Apache-2.0 |
| `aes` | 0.9.4 | MIT OR Apache-2.0 |
| `argon2` | 0.6.0 | MIT OR Apache-2.0 |
| `base64` | 0.23.1 | MIT OR Apache-2.0 |
| `base64ct` | 1.8.3 | Apache-2.0 OR MIT |
| `blake2` | 0.11.0 | MIT OR Apache-2.0 |
| `block-buffer` | 0.12.1 | MIT OR Apache-2.0 |
| `block-padding` | 0.4.2 | MIT OR Apache-2.0 |
| `cbc` | 0.2.1 | MIT OR Apache-2.0 |
| `cfg-if` | 1.0.5 | MIT OR Apache-2.0 |
| `chacha20` | 0.10.2 | MIT OR Apache-2.0 |
| `cipher` | 0.5.2 | MIT OR Apache-2.0 |
| `cmov` | 0.5.4 | Apache-2.0 OR MIT |
| `const-oid` | 0.10.2 | Apache-2.0 OR MIT |
| `cpubits` | 0.1.1 | MIT OR Apache-2.0 |
| `cpufeatures` | 0.3.1 | MIT OR Apache-2.0 |
| `crc32fast` | 1.5.4 | MIT OR Apache-2.0 |
| `crypto-common` | 0.2.2 | MIT OR Apache-2.0 |
| `ctutils` | 0.4.8 | Apache-2.0 OR MIT |
| `data-encoding` | 2.11.1 | MIT |
| `digest` | 0.11.3 | MIT OR Apache-2.0 |
| `displaydoc` | 0.2.7 | MIT OR Apache-2.0 |
| `flate2` | 1.1.11 | MIT OR Apache-2.0 |
| `form_urlencoded` | 1.2.2 | MIT OR Apache-2.0 |
| `getrandom` | 0.2.17 | MIT OR Apache-2.0 |
| `getrandom` | 0.4.3 | MIT OR Apache-2.0 |
| `hmac` | 0.13.0 | MIT OR Apache-2.0 |
| `hybrid-array` | 0.4.15 | MIT OR Apache-2.0 |
| `icu_collections` | 2.4.0 | Unicode-3.0 |
| `icu_locale_core` | 2.4.0 | Unicode-3.0 |
| `icu_normalizer` | 2.4.0 | Unicode-3.0 |
| `icu_normalizer_data` | 2.4.0 | Unicode-3.0 |
| `icu_properties` | 2.4.0 | Unicode-3.0 |
| `icu_properties_data` | 2.4.0 | Unicode-3.0 |
| `icu_provider` | 2.4.0 | Unicode-3.0 |
| `idna` | 1.1.0 | MIT OR Apache-2.0 |
| `idna_adapter` | 1.2.2 | Apache-2.0 OR MIT |
| `inout` | 0.2.6 | MIT OR Apache-2.0 |
| `itoa` | 1.0.19 | MIT OR Apache-2.0 |
| `libc` | 0.2.191 | MIT OR Apache-2.0 |
| `litemap` | 0.8.3 | Unicode-3.0 |
| `memchr` | 2.8.3 | Unlicense OR MIT |
| `miniz_oxide` | 0.9.1 | MIT OR Zlib OR Apache-2.0 |
| `password-hash` | 0.6.1 | MIT OR Apache-2.0 |
| `percent-encoding` | 2.3.2 | MIT OR Apache-2.0 |
| `phc` | 0.6.1 | Apache-2.0 OR MIT |
| `potential_utf` | 0.1.6 | Unicode-3.0 |
| `ppv-lite86` | 0.2.21 | MIT OR Apache-2.0 |
| `proc-macro2` | 1.0.107 | MIT OR Apache-2.0 |
| `quick-xml` | 0.42.0 | MIT |
| `quote` | 1.0.47 | MIT OR Apache-2.0 |
| `r-efi` | 6.0.0 | MIT OR Apache-2.0 OR LGPL-2.1-or-later |
| `rand` | 0.8.8 | MIT OR Apache-2.0 |
| `rand_chacha` | 0.3.1 | MIT OR Apache-2.0 |
| `rand_core` | 0.6.4 | MIT OR Apache-2.0 |
| `salsa20` | 0.11.0 | MIT OR Apache-2.0 |
| `secrecy` | 0.10.3 | Apache-2.0 OR MIT |
| `serde` | 1.0.229 | MIT OR Apache-2.0 |
| `serde_core` | 1.0.229 | MIT OR Apache-2.0 |
| `serde_derive` | 1.0.229 | MIT OR Apache-2.0 |
| `serde_json` | 1.0.154 | MIT OR Apache-2.0 |
| `sha1` | 0.11.0 | MIT OR Apache-2.0 |
| `sha2` | 0.11.1 | MIT OR Apache-2.0 |
| `simd-adler32` | 0.3.10 | MIT |
| `smallvec` | 1.16.5 | MIT OR Apache-2.0 |
| `stable_deref_trait` | 1.2.1 | MIT OR Apache-2.0 |
| `subtle` | 2.6.1 | BSD-3-Clause |
| `syn` | 2.0.119 | MIT OR Apache-2.0 |
| `syn` | 3.0.10 | MIT OR Apache-2.0 |
| `synstructure` | 0.14.0 | MIT |
| `thiserror` | 2.0.21 | MIT OR Apache-2.0 |
| `thiserror-impl` | 2.0.21 | MIT OR Apache-2.0 |
| `tinystr` | 0.8.5 | Unicode-3.0 |
| `typenum` | 1.20.1 | MIT OR Apache-2.0 |
| `unicode-ident` | 1.0.27 | (MIT OR Apache-2.0) AND Unicode-3.0 |
| `url` | 2.5.8 | MIT OR Apache-2.0 |
| `utf8_iter` | 1.1.0 | Apache-2.0 OR MIT |
| `utf_types` | 1.0.0 | Apache-2.0 OR MIT |
| `uuid` | 1.29.2 | Apache-2.0 OR MIT |
| `wasi` | 0.11.1+wasi-snapshot-preview1 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT |
| `writeable` | 0.6.4 | Unicode-3.0 |
| `yoke` | 0.8.4 | Unicode-3.0 |
| `yoke-derive` | 0.8.4 | Unicode-3.0 |
| `zerocopy` | 0.8.63 | BSD-2-Clause OR Apache-2.0 OR MIT |
| `zerocopy-derive` | 0.8.63 | BSD-2-Clause OR Apache-2.0 OR MIT |
| `zerofrom` | 0.1.8 | Unicode-3.0 |
| `zerofrom-derive` | 0.1.8 | Unicode-3.0 |
| `zeroize` | 1.9.1 | Apache-2.0 OR MIT |
| `zerotrie` | 0.2.5 | Unicode-3.0 |
| `zerovec` | 0.11.9 | Unicode-3.0 |
| `zerovec-derive` | 0.11.7 | Unicode-3.0 |
| `zmij` | 1.0.23 | MIT |

### Test/dev only (including build tooling and target-only lock entries)

| Crate | Version | Licence |
|---|---|---|
| `android_system_properties` | 0.1.6 | MIT OR Apache-2.0 |
| `arrayvec` | 0.7.8 | MIT OR Apache-2.0 |
| `autocfg` | 1.5.1 | Apache-2.0 OR MIT |
| `base64` | 0.22.1 | MIT OR Apache-2.0 |
| `bit-set` | 0.11.1 | Apache-2.0 OR MIT |
| `bit-vec` | 0.10.1 | Apache-2.0 OR MIT |
| `bitflags` | 2.13.2 | MIT OR Apache-2.0 |
| `blake2b_simd` | 1.0.5 | MIT |
| `block-modes` | 0.9.1 | MIT OR Apache-2.0 |
| `borsh` | 1.8.1 | MIT OR Apache-2.0 |
| `borsh-derive` | 1.8.1 | Apache-2.0 |
| `bumpalo` | 3.20.3 | MIT OR Apache-2.0 |
| `byteorder` | 1.5.0 | Unlicense OR MIT |
| `bytes` | 1.12.1 | MIT |
| `cc` | 1.11.1 | MIT OR Apache-2.0 |
| `cfg_aliases` | 0.2.2 | MIT |
| `chrono` | 0.4.45 | MIT OR Apache-2.0 |
| `constant_time_eq` | 0.4.2 | CC0-1.0 OR MIT-0 OR Apache-2.0 |
| `core-foundation-sys` | 0.8.7 | MIT OR Apache-2.0 |
| `core_detect` | 1.0.0 | MIT/Apache-2.0 |
| `crossbeam-utils` | 0.8.23 | MIT OR Apache-2.0 |
| `equivalent` | 1.0.3 | Apache-2.0 OR MIT |
| `errno` | 0.3.14 | MIT OR Apache-2.0 |
| `fastrand` | 2.5.0 | Apache-2.0 OR MIT |
| `find-msvc-tools` | 0.1.15 | MIT OR Apache-2.0 |
| `fnv` | 1.0.7 | Apache-2.0 / MIT |
| `futures-core` | 0.3.34 | MIT OR Apache-2.0 |
| `futures-task` | 0.3.34 | MIT OR Apache-2.0 |
| `futures-util` | 0.3.34 | MIT OR Apache-2.0 |
| `hashbrown` | 0.17.1 | MIT OR Apache-2.0 |
| `hex` | 0.4.3 | MIT OR Apache-2.0 |
| `hex-literal` | 1.1.0 | MIT OR Apache-2.0 |
| `iana-time-zone` | 0.1.65 | MIT OR Apache-2.0 |
| `iana-time-zone-haiku` | 0.1.2 | MIT OR Apache-2.0 |
| `indexmap` | 2.14.2 | Apache-2.0 OR MIT |
| `js-sys` | 0.3.107 | MIT OR Apache-2.0 |
| `keepass` | 0.17.0 | MIT |
| `linux-raw-sys` | 0.12.1 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT |
| `log` | 0.4.35 | MIT OR Apache-2.0 |
| `num-traits` | 0.2.19 | MIT OR Apache-2.0 |
| `once_cell` | 1.21.4 | MIT OR Apache-2.0 |
| `pin-project-lite` | 0.2.17 | Apache-2.0 OR MIT |
| `proc-macro-crate` | 3.5.0 | MIT OR Apache-2.0 |
| `proptest` | 1.12.0 | MIT OR Apache-2.0 |
| `quick-error` | 1.2.3 | MIT/Apache-2.0 |
| `rand` | 0.10.3 | MIT OR Apache-2.0 |
| `rand_core` | 0.10.1 | MIT OR Apache-2.0 |
| `rand_xorshift` | 0.5.0 | MIT OR Apache-2.0 |
| `regex-syntax` | 0.8.11 | MIT OR Apache-2.0 |
| `rust-argon2` | 3.0.0 | MIT/Apache-2.0 |
| `rustix` | 1.1.6 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT |
| `rustversion` | 1.0.23 | MIT OR Apache-2.0 |
| `rusty-fork` | 0.3.1 | MIT/Apache-2.0 |
| `ryu` | 1.0.23 | Apache-2.0 OR BSL-1.0 |
| `serde_spanned` | 1.1.3 | MIT OR Apache-2.0 |
| `serde_yaml_ng` | 0.10.0 | MIT |
| `shlex` | 2.0.1 | MIT OR Apache-2.0 |
| `slab` | 0.4.12 | MIT |
| `tempfile` | 3.27.0 | MIT OR Apache-2.0 |
| `toml` | 1.2.1+spec-1.1.0 | MIT OR Apache-2.0 |
| `toml_datetime` | 1.1.3+spec-1.1.0 | MIT OR Apache-2.0 |
| `toml_edit` | 0.25.19+spec-1.1.0 | MIT OR Apache-2.0 |
| `toml_parser` | 1.1.5+spec-1.1.0 | MIT OR Apache-2.0 |
| `toml_writer` | 1.2.0+spec-1.1.0 | MIT OR Apache-2.0 |
| `twofish` | 0.8.0 | MIT OR Apache-2.0 |
| `unarray` | 0.1.4 | MIT OR Apache-2.0 |
| `unsafe-libyaml` | 0.2.11 | MIT |
| `wait-timeout` | 0.2.1 | MIT/Apache-2.0 |
| `wasm-bindgen` | 0.2.130 | MIT OR Apache-2.0 |
| `wasm-bindgen-macro` | 0.2.130 | MIT OR Apache-2.0 |
| `wasm-bindgen-macro-support` | 0.2.130 | MIT OR Apache-2.0 |
| `wasm-bindgen-shared` | 0.2.130 | MIT OR Apache-2.0 |
| `windows-core` | 0.62.2 | MIT OR Apache-2.0 |
| `windows-implement` | 0.60.2 | MIT OR Apache-2.0 |
| `windows-interface` | 0.59.3 | MIT OR Apache-2.0 |
| `windows-link` | 0.2.1 | MIT OR Apache-2.0 |
| `windows-result` | 0.4.1 | MIT OR Apache-2.0 |
| `windows-strings` | 0.5.1 | MIT OR Apache-2.0 |
| `windows-sys` | 0.61.2 | MIT OR Apache-2.0 |
| `winnow` | 1.0.5 | MIT |
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
`serde_yaml_ng`) are linked into test artifacts only, never shipped.

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
