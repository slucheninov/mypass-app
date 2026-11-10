# Vendored: Apple password-manager-resources (quirks/password-rules.json)

- Upstream: https://github.com/apple/password-manager-resources
- Vendored file: quirks/password-rules.json
- Commit: 6073487115b722596d3e1826d1b71b524b73bac5
- Retrieved on: 2026-09-12
- Entry count at that commit: 436

The vendored JSON is committed as-is, with no reformatting, so a future
`git diff` against upstream stays readable. Regenerate with
`scripts/update-password-rules.sh` -- this file and `password-rules.json`
are always rewritten together, in the same run, so they can never
independently drift out of sync (see that script for why: it is exactly
the same commit/date/count triple recorded here, plus a fresh copy of
upstream's own LICENSE.md, generated fresh on every run).

## Licence

Copied verbatim from upstream's `LICENSE.md` at the commit above:

Copyright 2020 - 2026 Apple Inc.

Permission is hereby granted, free of charge, to any person obtaining a copy of this software and associated documentation files (the "Software"), to deal in the Software without restriction, including without limitation the rights to use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of the Software, and to permit persons to whom the Software is furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.
