## Summary

<!-- What changes and why. Link the issue, spec or plan if there is one. -->

## Checklist

- [ ] I ran `bash scripts/test.sh` and it passed. I did not skip, weaken or delete any test to get there (`--no-js` is not acceptable when `extension/` changed).
- [ ] New or changed behaviour has tests next to the code it covers.
- [ ] Dependencies: I added, removed or upgraded none, or I updated `THIRD_PARTY_NOTICES.md` (version, commit, licence text, upstream NOTICE) and committed `Cargo.lock`.
- [ ] I updated `THIRD_PARTY_NOTICES.md` if dependencies or vendored files changed.
- [ ] Everything I wrote is in English (the only exception is the Ukrainian root `RUN.md`).
- [ ] Commits follow Conventional Commits with a scope, for example `fix(kdbx): ...`, with no AI-attribution footer.
- [ ] I did not commit generated or local files (`target/`, `dist/`, `node_modules/`), vault files with real data, or secrets.

## Test evidence

<!-- Paste the real result of the commands you ran, not a summary of what you expect. -->
