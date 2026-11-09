# Third-party notices

MyPass itself is licensed under the Apache License 2.0 (see `LICENSE`). This file lists the
third-party code and data that the MyPass build artifacts contain, in source or object form,
together with the licence notices those components require.

**Maintenance.** Add or update an entry here in the same change that adds, removes or bumps a
dependency, or re-vendors a file (see `CONTRIBUTING.md`). This file is a reviewed list, not
machine-generated.

## Vendored data

### Apple password-manager-resources (`quirks/password-rules.json`)

- Upstream: https://github.com/apple/password-manager-resources
- Vendored file: `quirks/password-rules.json`, commit `6073487115b722596d3e1826d1b71b524b73bac5`
  (retrieved 2026-09-12, 436 entries at that commit). The file is committed as-is, with no
  reformatting. It is added to this repository by a later stage 1 task, which records the exact
  location and provenance here in the same change.
- Licence: MIT. Text copied verbatim from upstream's `LICENSE.md` at the commit above:

~~~~text
Copyright 2020 - 2026 Apple Inc.

Permission is hereby granted, free of charge, to any person obtaining a copy of this software and associated documentation files (the "Software"), to deal in the Software without restriction, including without limitation the rights to use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of the Software, and to permit persons to whom the Software is furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.
~~~~

### Mozilla Public Suffix List

- Upstream: https://publicsuffix.org/ (source repository https://github.com/publicsuffix/list).
- Vendored file: `public_suffix_list.dat`, shipped as a bundled resource of `mypass-core`. The data
  file is unmodified, as served by publicsuffix.org. It is added to this repository by a later
  stage 1 task, which records the commit and list version here in the same change.
- Licence: Mozilla Public License 2.0, https://www.mozilla.org/en-US/MPL/2.0/. The source form of
  the covered file is the file itself, available at the links above. The full licence text is
  added together with the file.

## Linked crates

Completed in Task 18 of the stage 1 plan, once every dependency of the product crates is in
place, generated with `cargo metadata` and reviewed by hand.

| Crate | Version | Licence | Goes into |
|---|---|---|---|
| `mypass-kdbx` | workspace | Apache-2.0 | MyPass (own code) |
| `mypass-core` | workspace | Apache-2.0 | MyPass (own code) |
| `sha2` | 0.11.1 | MIT OR Apache-2.0 | `mypass-kdbx` header hashes |
| `hmac` | 0.13.0 | MIT OR Apache-2.0 | `mypass-kdbx` header authentication |
| `subtle` | 2.6.1 | BSD-3-Clause | `mypass-kdbx` constant-time digest comparison |
| `zeroize` | 1.9.1 | Apache-2.0 OR MIT | `mypass-kdbx` derived key cleanup |
| `argon2` | 0.6.0 | MIT OR Apache-2.0 | `mypass-kdbx` Argon2d/id key derivation |
| `aes` | 0.9.4 | MIT OR Apache-2.0 | `mypass-kdbx` AES-KDF and AES-256-CBC outer cipher |
| `secrecy` | 0.10.3 | Apache-2.0 OR MIT | `mypass-kdbx` master password protection |
| `getrandom` | 0.4.3 | MIT OR Apache-2.0 | `mypass-kdbx` OS-generated KDF salts and seeds |
| `cbc` | 0.2.1 | MIT OR Apache-2.0 | `mypass-kdbx` CBC outer cipher mode and PKCS7 padding |
| `chacha20` | 0.10.2 | MIT OR Apache-2.0 | `mypass-kdbx` ChaCha20 outer cipher |
| `flate2` | 1.1.11 | MIT OR Apache-2.0 | `mypass-kdbx` gzip compression and bounded decompression |

## Test-only (not part of any shipped artifact)

`repo-guards` is a test-only workspace crate (`publish = false`) and is never shipped. The
dev-only crates used by tests, the oracle and the repository guards (`tempfile`, `proptest`,
`keepass`, `toml`, `serde_yaml_ng`) are not linked into any artifact.
