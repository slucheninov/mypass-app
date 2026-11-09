# Contributing to MyPass

Thanks for helping. This file is the short version of how to build, test and submit a change.
[`README.md`](README.md) explains what the project is; [`AGENTS.md`](AGENTS.md) is the instruction
file for coding agents and carries the same project conventions.

## Setup

You need [rustup](https://rustup.rs). The toolchain is pinned in `rust-toolchain.toml` (with
`rustfmt` and `clippy`) and rustup installs it on first use. `bash` is needed for `scripts/test.sh`:
on Windows use Git Bash for the scripts and PowerShell for `cargo`. Node.js and pnpm are needed only
once the UI and the browser extension exist (later stages); `scripts/test.sh` says so when a part
needs a tool you do not have.

## Commands

| Command | Purpose |
|---|---|
| `cargo fmt --all -- --check` | Check formatting (`cargo fmt --all` fixes it). |
| `cargo clippy --workspace --all-targets -- -D warnings` | Lint everything; warnings fail. |
| `bash scripts/test.sh [--ci] [--only rust\|ui\|js] [--no-js]` | The full test suite: the single entry point for people, agents and CI. |
| `cargo test --workspace` | Rust tests only. |
| `cargo test -p <crate> <filter>` | One crate, one name filter. |

## Tests must really run

Run `bash scripts/test.sh` before you say a change is done, and show real results in your pull
request. Tests are not allowed to be skipped, weakened or deleted to make something pass.

- `--no-js` exists only for a local machine without `node`, and it prints that the JavaScript suites
  were **not** run. A change that touches `extension/` needs them run.
- `--ci` is the mode used by the CI workflow: cargo runs with `--locked`, it requires every tool of
  every part that exists and it rejects `--no-js`.
- `.github/workflows/ci.yml` runs on pull requests, on pushes to `main` and on manual dispatch: a
  test matrix on macOS and Windows, a path-filtered bidirectional pykeepass `crosscompat` job, a disabled Windows integration skeleton and a final `ci` job that
  reports the combined result. It runs after the fact, so it is not a substitute for running the
  suite yourself. `.github/workflows/codeql.yml` is a separate workflow that runs after the merge,
  weekly and on manual dispatch.
- New behaviour comes with tests next to the code it covers. Format with `cargo fmt`; clippy runs
  with `-D warnings`.
- Manual-only checks (signed build, Windows, real vault) are walked by the maintainer in daily use and
  never block a merge.

## Language

English is the source language: code, comments, commit messages, docs, error messages. Cyrillic
appears only in `*.xcstrings` files, the root `RUN.md` and as `\u{..}` escapes in tests (literal
Cyrillic only in `crates/mypass-kdbx/tests/unicode.rs`).

## Commits and pull requests

Commits follow Conventional Commits with a scope, a lowercase imperative subject and no trailing
period, for example:

```text
fix(kdbx): reject a header with a zero-length KDF salt
```

Scopes in use: `core`, `kdbx`, `ci`, `docs`, `repo` (later stages add `bridge`, `platform`, `host`,
`app` and `extension`). A dependency version bump is its own `chore(deps)` commit. Do not add
AI-attribution footers or co-author trailers to commits or pull request descriptions.

Keep a pull request to one concern. Comments that record why a constraint exists are a feature of
this code base; preserve them when editing. `docs/implementation/ROADMAP.md` lists only work that is
still open: remove an item once it is implemented and its tests pass. Design notes and plans are
local working files (`docs/superpowers/` is gitignored); do not commit them, and put anything
contributors need to know in the README, this file or the code comments.

Do not commit the generated or local files listed in `.gitignore` (`/target/`, `/dist/`,
`node_modules/`, `docs/superpowers/`, `.worktrees/` and similar), vault files with real data, or
secrets.

## Dependencies and the notices checklist

MyPass is licensed under Apache-2.0. Adding, removing or upgrading a dependency, or re-vendoring a
file, changes what the build artifacts contain. In the same pull request:

- [ ] Update [`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md): component name, version and commit,
      licence, where it lives, and the upstream licence text copied verbatim, plus the upstream
      `NOTICE` content when the licence (for example Apache-2.0 section 4(d)) requires it.
- [ ] Declare third-party crates only in `[workspace.dependencies]` of the root `Cargo.toml`, with an
      exact version, and use `dep.workspace = true` in member crates. Commit `Cargo.lock`.
- [ ] Check transitive dependencies against what the artifacts really link; the notices file is a
      reviewed list, not generated blindly from the lock file.
- [ ] Confirm the licence is compatible with an Apache-2.0 project.

## Security

Do not put passwords, vault files, master keys, tokens or key material in an issue, a pull request or
a log you attach. Do not describe a security vulnerability in detail in a public issue or pull
request. Report vulnerabilities as described in [`SECURITY.md`](SECURITY.md).

## Licence

Under section 5 of the Apache License 2.0, a contribution you submit for inclusion is licensed under
that license unless you state otherwise (see [`LICENSE`](LICENSE)).

## Main branch protection (owner action)

After pushing and checking CI, the owner can apply this ruleset. These commands are documented
only; implementation agents do not apply them. `ci` is the only required status check. It accepts
skipped path/variable-gated jobs, so irrelevant changes do not leave required checks pending.
Zero approvals are required, pull requests are required, and only squash merging is allowed.
Force pushes and branch deletion are blocked. Run in the repository with authenticated `gh`:

```bash
gh api --method POST repos/{owner}/{repo}/rulesets --input - <<'JSON'
{
  "name": "main",
  "target": "branch",
  "enforcement": "active",
  "bypass_actors": [],
  "conditions": {
    "ref_name": {"include": ["refs/heads/main"], "exclude": []}
  },
  "rules": [
    {"type": "deletion"},
    {"type": "non_fast_forward"},
    {
      "type": "pull_request",
      "parameters": {
        "required_approving_review_count": 0,
        "dismiss_stale_reviews_on_push": false,
        "require_code_owner_review": false,
        "require_last_push_approval": false,
        "required_review_thread_resolution": false,
        "allowed_merge_methods": ["squash"]
      }
    },
    {
      "type": "required_status_checks",
      "parameters": {
        "required_status_checks": [{"context": "ci"}],
        "strict_required_status_checks_policy": false
      }
    }
  ]
}
JSON

gh api --method PATCH repos/{owner}/{repo} \
  -F allow_squash_merge=true -F allow_merge_commit=false -F allow_rebase_merge=false \
  -F delete_branch_on_merge=true
```

The repository settings command enables automatic head-branch deletion after merging. The ruleset
POST creates a rule; if `main` already exists, inspect its ID and use the ruleset update endpoint
instead of creating a duplicate. Applying these settings is pending owner action.
