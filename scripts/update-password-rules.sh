#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"

UPSTREAM_URL="https://github.com/apple/password-manager-resources.git"
UPSTREAM_DATA_FILE="quirks/password-rules.json"
UPSTREAM_LICENSE_FILE="LICENSE.md"

DATA_TARGET="$REPO_ROOT/crates/mypass-core/resources/password-rules.json"
PROVENANCE_TARGET="$REPO_ROOT/crates/mypass-core/resources/PASSWORD_RULES_PROVENANCE.md"

require_command() {
  local cmd="$1"
  local hint="${2:-}"
  if ! command -v "$cmd" >/dev/null 2>&1; then
    echo "Error: required command '$cmd' not found." >&2
    [[ -n "$hint" ]] && echo "$hint" >&2
    exit 1
  fi
}

require_command git
# python3, not jq: python3 ships with the Xcode command line tools, so using it
# for JSON handling here avoids adding a second JSON tool as a preflight
# dependency. (scripts/dev.sh install-host deliberately needs neither.)
require_command python3

WORK_DIR="$(mktemp -d)"
# DATA_TMP and PROVENANCE_TMP are populated once the corresponding temp file
# exists (below), so this cleanup is a harmless no-op until then and only
# ever removes a temp file that was never successfully moved into place --
# see the atomic-write comment further down for why.
DATA_TMP=""
PROVENANCE_TMP=""
cleanup() {
  rm -rf "$WORK_DIR"
  [[ -n "$DATA_TMP" && -f "$DATA_TMP" ]] && rm -f "$DATA_TMP"
  [[ -n "$PROVENANCE_TMP" && -f "$PROVENANCE_TMP" ]] && rm -f "$PROVENANCE_TMP"
  # A false `[[ ... ]]` above (the normal case once both temp files have
  # already been moved into place) would otherwise leak out as this
  # function's -- and then the whole script's -- exit status under `set -e`.
  # Cleanup succeeding at "nothing to clean up" is still success.
  return 0
}
trap cleanup EXIT

echo "Cloning $UPSTREAM_URL (shallow)..."
git clone --depth 1 "$UPSTREAM_URL" "$WORK_DIR/repo" >/dev/null

COMMIT_SHA="$(git -C "$WORK_DIR/repo" rev-parse HEAD)"
RETRIEVAL_DATE="$(date -u +%Y-%m-%d)"

if [[ ! -f "$WORK_DIR/repo/$UPSTREAM_DATA_FILE" ]]; then
  echo "Error: $UPSTREAM_DATA_FILE not found at $UPSTREAM_URL@$COMMIT_SHA -- upstream layout may have changed." >&2
  exit 1
fi

ENTRY_COUNT="$(python3 -c "
import json, sys
with open(sys.argv[1], encoding='utf-8') as f:
    data = json.load(f)
print(len(data))
" "$WORK_DIR/repo/$UPSTREAM_DATA_FILE")"

if [[ "$ENTRY_COUNT" -lt 1 ]]; then
  echo "Error: fetched password-rules.json has zero entries -- refusing to vendor an empty snapshot." >&2
  exit 1
fi

# Both targets are assembled as temp files in the same directory as their
# final destination (so the later `mv` is a same-filesystem rename, not a
# copy) and only moved into place once each is fully written. This file and
# PASSWORD_RULES_PROVENANCE.md assert, in prose, that they are always
# rewritten together and can never independently drift out of sync -- a
# `cp`/heredoc straight to the final path would let a kill or a full disk
# partway through leave a truncated file sitting at that path, which is
# exactly the drift the prose promises can't happen. Writing to a temp file
# first and renaming it in means the target path only ever holds a complete
# previous version or a complete new one, never a partial write.
DATA_TMP="$(mktemp "$DATA_TARGET.XXXXXX")"
# Committed as-is, no reformatting -- see PASSWORD_RULES_PROVENANCE.md for
# why: it keeps a future `git diff` against upstream readable.
cp "$WORK_DIR/repo/$UPSTREAM_DATA_FILE" "$DATA_TMP"

PROVENANCE_TMP="$(mktemp "$PROVENANCE_TARGET.XXXXXX")"
cat > "$PROVENANCE_TMP" <<EOF
# Vendored: Apple password-manager-resources (quirks/password-rules.json)

- Upstream: https://github.com/apple/password-manager-resources
- Vendored file: quirks/password-rules.json
- Commit: $COMMIT_SHA
- Retrieved on: $RETRIEVAL_DATE
- Entry count at that commit: $ENTRY_COUNT

The vendored JSON is committed as-is, with no reformatting, so a future
\`git diff\` against upstream stays readable. Regenerate with
\`scripts/update-password-rules.sh\` -- this file and \`password-rules.json\`
are always rewritten together, in the same run, so they can never
independently drift out of sync (see that script for why: it is exactly
the same commit/date/count triple recorded here, plus a fresh copy of
upstream's own LICENSE.md, generated fresh on every run).

## Licence

Copied verbatim from upstream's \`LICENSE.md\` at the commit above:

EOF

cat "$WORK_DIR/repo/$UPSTREAM_LICENSE_FILE" >> "$PROVENANCE_TMP"

# Move both into place back-to-back, with nothing else in between that could
# fail -- this is the earliest point at which either target's old content is
# disturbed.
mv "$DATA_TMP" "$DATA_TARGET"
mv "$PROVENANCE_TMP" "$PROVENANCE_TARGET"

echo "Updated $DATA_TARGET ($ENTRY_COUNT entries) and $PROVENANCE_TARGET at commit $COMMIT_SHA."
