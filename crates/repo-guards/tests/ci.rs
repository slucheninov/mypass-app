//! Guards for the GitHub Actions workflows: aggregator wiring, concurrency, caching, pinning.
//!
//! test-only support code: helpers here unwrap and index freely, a guard must fail loudly.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use repo_guards::{read, workflow_files};
use serde_yaml_ng::{Mapping, Value};

/// Jobs whose `skipped` result the aggregator may accept: gated on a repository variable
/// (`windows-integration`) or on changed paths (`crosscompat`, `package-smoke`, added by later
/// tasks). Every other job, `changes` included, must be `success`.
const GATED_JOBS: [&str; 3] = ["windows-integration", "crosscompat", "package-smoke"];

fn workflow(rel: &str) -> Value {
    serde_yaml_ng::from_str(&read(rel)).unwrap_or_else(|e| panic!("{rel} does not parse: {e}"))
}

fn jobs(doc: &Value) -> &Mapping {
    doc.get("jobs")
        .and_then(Value::as_mapping)
        .expect("workflow has `jobs`")
}

fn job<'a>(doc: &'a Value, id: &str) -> &'a Value {
    jobs(doc)
        .get(id)
        .unwrap_or_else(|| panic!("workflow has no job `{id}`"))
}

fn steps(job: &Value) -> Vec<&Value> {
    job.get("steps")
        .and_then(Value::as_sequence)
        .map(|s| s.iter().collect())
        .unwrap_or_default()
}

fn run_text(step: &Value) -> String {
    step.get("run")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned()
}

/// The workflow `on:` block. YAML 1.1 parsers may read the key `on` as boolean true.
fn triggers(doc: &Value) -> &Mapping {
    let map = doc.as_mapping().expect("workflow is a mapping");
    for (key, value) in map {
        let is_on = key.as_str() == Some("on") || key.as_bool() == Some(true);
        if is_on {
            return value.as_mapping().expect("`on:` is a mapping");
        }
    }
    panic!("workflow has no `on:` block");
}

fn needs_of(job: &Value) -> Vec<String> {
    match job.get("needs") {
        Some(Value::Sequence(seq)) => seq
            .iter()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect(),
        Some(Value::String(s)) => vec![s.clone()],
        _ => Vec::new(),
    }
}

#[test]
fn ci_aggregator_needs_every_job() {
    let doc = workflow(".github/workflows/ci.yml");
    let all: Vec<String> = jobs(&doc)
        .keys()
        .map(|k| k.as_str().expect("job id is a string").to_owned())
        .collect();
    assert!(
        all.iter().any(|j| j == "ci"),
        "ci.yml needs the aggregator job `ci`"
    );
    let needs = needs_of(job(&doc, "ci"));
    let mut sorted_needs = needs.clone();
    sorted_needs.sort();
    assert_eq!(
        sorted_needs,
        [
            "changes",
            "crosscompat",
            "i18n",
            "lint",
            "test",
            "windows-integration"
        ],
        "`ci.needs` must be exactly the parallel jobs of the workflow"
    );
    for id in all.iter().filter(|j| *j != "ci") {
        assert!(
            needs.contains(id),
            "job `{id}` is not in `ci.needs`: add it so the required check covers it"
        );
    }
    for need in &needs {
        assert!(all.contains(need), "`ci.needs` names unknown job `{need}`");
    }
}

#[test]
fn ci_aggregator_accepts_skipped_only_for_gated_jobs() {
    let doc = workflow(".github/workflows/ci.yml");
    let ci = job(&doc, "ci");
    assert_eq!(
        ci.get("if").and_then(Value::as_str).map(str::trim),
        Some("always()"),
        "`ci` must run `if: always()` so a failed need cannot leave the check pending"
    );

    // The aggregator script declares the tolerated set on one line: `gated=" a b c "`.
    let script: String = steps(ci)
        .iter()
        .map(|s| run_text(s))
        .collect::<Vec<_>>()
        .join("\n");
    let line = script
        .lines()
        .find(|l| l.trim_start().starts_with("gated="))
        .expect("aggregator script declares `gated=\"...\"`");
    let value = line.split_once('=').unwrap().1.trim().trim_matches('"');
    let gated: Vec<&str> = value.split_whitespace().collect();
    assert!(
        gated.contains(&"windows-integration"),
        "windows-integration must be tolerated when skipped"
    );
    assert!(
        gated.contains(&"crosscompat"),
        "crosscompat must tolerate skipped"
    );
    for name in &gated {
        assert!(
            GATED_JOBS.contains(name),
            "`{name}` must not be accepted as skipped; only {GATED_JOBS:?} are gated"
        );
    }
    assert!(
        !gated.contains(&"changes"),
        "`changes` must be `success`, never `skipped`"
    );
    assert!(
        script.contains("skipped") && script.contains("success"),
        "aggregator must compare each result against `success` and the gated `skipped`"
    );
    // Every need that exists in this workflow and is not gated must therefore be required to succeed.
    // (A `failure` or `cancelled` result is never accepted: only the two literals above pass.)
    assert!(
        !script.contains("cancelled"),
        "aggregator must not whitelist `cancelled`"
    );
}

#[test]
fn ci_has_concurrency_cancel_in_progress() {
    let doc = workflow(".github/workflows/ci.yml");
    let concurrency = doc
        .get("concurrency")
        .expect("ci.yml has top-level `concurrency`");
    assert_eq!(
        concurrency.get("group").and_then(Value::as_str),
        Some("ci-${{ github.event_name == 'pull_request' && github.ref || github.run_id }}"),
        "pull requests share one group per ref (so a newer push cancels the older run); every other \
         run gets its own group, because GitHub keeps only ONE pending run per group and a burst of \
         pushes to main would otherwise drop the CI result of the intermediate commits"
    );
    assert_eq!(
        concurrency
            .get("cancel-in-progress")
            .and_then(Value::as_str),
        Some("${{ github.event_name == 'pull_request' }}"),
        "only pull request runs may be cancelled; runs on main and manual runs never are"
    );
    let env = doc.get("env").expect("ci.yml has top-level `env`");
    assert_eq!(
        env.get("CARGO_PROFILE_DEV_DEBUG").and_then(Value::as_str),
        Some("line-tables-only")
    );
    assert_eq!(
        env.get("CARGO_TERM_COLOR").and_then(Value::as_str),
        Some("always")
    );
    let perms = doc
        .get("permissions")
        .expect("ci.yml has top-level `permissions`");
    assert_eq!(perms.get("contents").and_then(Value::as_str), Some("read"));
}

#[test]
fn ci_triggers_cover_pull_request_main_and_dispatch() {
    let doc = workflow(".github/workflows/ci.yml");
    let on = triggers(&doc);
    assert!(on.contains_key("pull_request"));
    assert!(on.contains_key("workflow_dispatch"));
    let branches = on
        .get("push")
        .and_then(|p| p.get("branches"))
        .and_then(Value::as_sequence)
        .expect("push.branches");
    assert!(branches.iter().any(|b| b.as_str() == Some("main")));
}

#[test]
fn ci_cargo_jobs_use_rust_cache() {
    let mut cargo_jobs = 0;
    for file in workflow_files() {
        let doc = workflow(&file);
        for (id, job) in jobs(&doc) {
            let steps = steps(job);
            let first_cargo = steps.iter().position(|s| {
                run_text(s).lines().any(|l| {
                    l.trim_start().starts_with("cargo ")
                            // `--only js` runs Node suites only and never invokes cargo.
                            || (l.contains("scripts/test.sh") && !l.contains("--only js"))
                })
            });
            let Some(first_cargo) = first_cargo else {
                continue;
            };
            cargo_jobs += 1;
            let cache = steps.iter().position(|s| {
                s.get("uses")
                    .and_then(Value::as_str)
                    .is_some_and(|u| u.starts_with("Swatinem/rust-cache@"))
            });
            let id = id.as_str().unwrap_or("?");
            let cache = cache.unwrap_or_else(|| {
                panic!("{file}: cargo job `{id}` has no Swatinem/rust-cache step")
            });
            assert!(
                cache < first_cargo,
                "{file}: job `{id}` restores the cache after its first cargo step"
            );
        }
    }
    assert!(
        cargo_jobs > 0,
        "no cargo job found in any workflow: the guard would pass vacuously"
    );

    // The parallel cargo jobs of ci.yml are named explicitly, and each one saves its cache even
    // when a cold run fails, so the retry starts warm.
    let doc = workflow(".github/workflows/ci.yml");
    for id in ["lint", "test", "windows-integration", "crosscompat"] {
        let cache = steps(job(&doc, id))
            .into_iter()
            .find(|s| {
                s.get("uses")
                    .and_then(Value::as_str)
                    .is_some_and(|u| u.starts_with("Swatinem/rust-cache@"))
            })
            .unwrap_or_else(|| panic!("ci.yml job `{id}` has no Swatinem/rust-cache step"));
        assert_eq!(
            cache
                .get("with")
                .and_then(|w| w.get("cache-on-failure"))
                .and_then(Value::as_bool),
            Some(true),
            "ci.yml job `{id}`: rust-cache needs `cache-on-failure: true`"
        );
    }
}

#[test]
fn ci_lint_job_runs_fmt_and_clippy_on_macos_and_windows() {
    let doc = workflow(".github/workflows/ci.yml");
    let lint = job(&doc, "lint");
    let os: Vec<&str> = lint
        .get("strategy")
        .and_then(|s| s.get("matrix"))
        .and_then(|m| m.get("os"))
        .and_then(Value::as_sequence)
        .expect("lint strategy.matrix.os")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert_eq!(os, ["macos-latest", "windows-latest"]);
    assert_eq!(
        lint.get("strategy")
            .and_then(|s| s.get("fail-fast"))
            .and_then(Value::as_bool),
        Some(false)
    );
    assert_eq!(
        lint.get("name").and_then(Value::as_str),
        Some("lint (${{ matrix.os }})")
    );
    let fmt = steps(lint)
        .into_iter()
        .find(|s| run_text(s).trim() == "cargo fmt --all -- --check")
        .expect("lint must run `cargo fmt --all -- --check`");
    assert_eq!(
        fmt.get("if").and_then(Value::as_str),
        Some("matrix.os == 'macos-latest'"),
        "the format check is OS independent and runs on macOS only"
    );
    let clippy = steps(lint)
        .into_iter()
        .find(|s| run_text(s).trim() == "cargo clippy --workspace --all-targets -- -D warnings")
        .expect("lint must run `cargo clippy --workspace --all-targets -- -D warnings`");
    assert!(
        clippy.get("if").is_none(),
        "clippy must run on every lint OS so cfg(unix) and cfg(windows) code are both linted"
    );
    let components = steps(lint)
        .into_iter()
        .find_map(|s| s.get("with")?.get("components")?.as_str())
        .expect("lint installs the toolchain components");
    assert!(components.contains("rustfmt") && components.contains("clippy"));
}

#[test]
fn codeql_never_triggers_on_pull_request() {
    let doc = workflow(".github/workflows/codeql.yml");
    let on = triggers(&doc);
    assert!(
        !on.contains_key("pull_request"),
        "codeql.yml must not run on pull_request"
    );
    assert!(
        !on.contains_key("pull_request_target"),
        "codeql.yml must not run on pull_request_target"
    );
    assert!(on.contains_key("push"));
    assert!(on.contains_key("schedule"));
    assert!(on.contains_key("workflow_dispatch"));
}

#[test]
fn windows_integration_job_is_gated_on_the_variable() {
    let doc = workflow(".github/workflows/ci.yml");
    let job = job(&doc, "windows-integration");
    let condition = job.get("if").and_then(Value::as_str).expect("job `if`");
    assert!(
        condition.contains("vars.MYPASS_WINDOWS_INTEGRATION_TESTS"),
        "the job must be gated on the repository variable, found `{condition}`"
    );
    assert_eq!(
        job.get("runs-on").and_then(Value::as_str),
        Some("windows-latest")
    );
    let runs: Vec<String> = steps(job).iter().map(|s| run_text(s)).collect();
    assert!(
        runs.iter()
            .any(|r| r.trim() == "cargo test --workspace -- --ignored windows_integration"),
        "missing the `cargo test --workspace -- --ignored windows_integration` step"
    );
}

/// `uses:` lines of a workflow, read as text so the trailing tag comment is visible.
fn uses_lines(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|l| {
            let t = l.trim_start().trim_start_matches("- ").trim_start();
            t.strip_prefix("uses:").map(|rest| rest.trim().to_owned())
        })
        .collect()
}

#[test]
fn every_workflow_uses_is_pinned_to_a_full_sha() {
    let mut count = 0;
    for file in workflow_files() {
        for uses in uses_lines(&read(&file)) {
            count += 1;
            let (action, comment) = uses.split_once(" # ").unwrap_or_else(|| {
                panic!("{file}: `uses: {uses}` needs a trailing `# <tag>` comment")
            });
            assert!(
                !comment.trim().is_empty(),
                "{file}: empty tag comment on `{uses}`"
            );
            let (name, sha) = action
                .split_once('@')
                .unwrap_or_else(|| panic!("{file}: `uses: {uses}` has no @ref"));
            assert!(name.contains('/'), "{file}: `{name}` is not owner/repo");
            assert!(
                sha.len() == 40 && sha.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f')),
                "{file}: `{action}` is not pinned to a full 40-hex commit SHA"
            );
        }
    }
    assert!(
        count > 0,
        "no `uses:` found: the guard would pass vacuously"
    );
}

#[test]
fn ci_test_job_runs_scripts_test_sh_in_ci_mode() {
    let doc = workflow(".github/workflows/ci.yml");
    let test = job(&doc, "test");
    let runs: Vec<String> = steps(test).iter().map(|s| run_text(s)).collect();
    let step = runs
        .iter()
        .find(|r| r.contains("scripts/test.sh"))
        .expect("job `test` must run scripts/test.sh");
    assert!(
        step.contains("--ci"),
        "scripts/test.sh must run with --ci, found `{step}`"
    );
    assert!(!step.contains("--no-js"), "--ci rejects --no-js");
    let os = test
        .get("strategy")
        .and_then(|s| s.get("matrix"))
        .and_then(|m| m.get("os"))
        .and_then(Value::as_sequence)
        .expect("strategy.matrix.os");
    let os: Vec<&str> = os.iter().filter_map(Value::as_str).collect();
    assert_eq!(os, ["macos-latest", "windows-latest"]);
    let all_runs = runs.join("\n");
    assert!(
        !all_runs.contains("cargo fmt") && !all_runs.contains("cargo clippy"),
        "format and clippy belong to the parallel `lint` job, not to `test`"
    );
    assert_eq!(
        test.get("strategy")
            .and_then(|s| s.get("fail-fast"))
            .and_then(Value::as_bool),
        Some(false)
    );
}

#[test]
fn codeql_workflow_covers_rust_and_javascript_typescript() {
    let doc = workflow(".github/workflows/codeql.yml");
    let analyze = jobs(&doc).values().next().expect("codeql.yml has a job");
    let langs: Vec<&str> = analyze
        .get("strategy")
        .and_then(|s| s.get("matrix"))
        .and_then(|m| m.get("language"))
        .and_then(Value::as_sequence)
        .expect("strategy.matrix.language")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert!(
        langs.contains(&"rust"),
        "CodeQL must cover rust, found {langs:?}"
    );
    assert!(
        langs.contains(&"javascript-typescript"),
        "CodeQL must cover javascript-typescript, found {langs:?}"
    );
    let perms = analyze.get("permissions").expect("job permissions");
    assert_eq!(
        perms.get("security-events").and_then(Value::as_str),
        Some("write")
    );
}

#[test]
fn crosscompat_is_path_gated_and_runs_both_directions() {
    let doc = workflow(".github/workflows/ci.yml");
    let cross = job(&doc, "crosscompat");
    assert_eq!(needs_of(cross), ["changes"]);
    assert_eq!(
        cross.get("runs-on").and_then(Value::as_str),
        Some("ubuntu-latest")
    );
    assert_eq!(cross.get("if").and_then(Value::as_str), Some("${{ github.event_name != 'pull_request' || needs.changes.outputs.crosscompat == 'true' }}"));
    let filter = job(&doc, "changes");
    assert!(filter.get("outputs").unwrap().get("crosscompat").is_some());
    let filters = steps(filter)
        .iter()
        .find_map(|s| s.get("with")?.get("filters")?.as_str())
        .unwrap()
        .to_owned();
    for path in [
        "crates/mypass-kdbx/**",
        "Cargo.toml",
        "Cargo.lock",
        ".github/workflows/ci.yml",
    ] {
        assert!(filters.contains(path), "crosscompat must filter {path}");
    }
    let runs = steps(cross)
        .iter()
        .map(|s| run_text(s))
        .collect::<Vec<_>>()
        .join("\n");
    for command in [
        "pykeepass==4.2.0",
        "--example write_crosscompat",
        "crates/mypass-kdbx/tests/crosscompat/pykeepass_check.py",
        "crates/mypass-kdbx/tests/crosscompat/pykeepass_make.py",
        "MYPASS_PYKEEPASS_VAULT=",
        "--test pykeepass_generated",
    ] {
        assert!(runs.contains(command), "crosscompat missing {command}");
    }
}

#[test]
fn codeql_uses_gating_build_mode_and_identical_action_revision() {
    let doc = workflow(".github/workflows/codeql.yml");
    let analyze = job(&doc, "analyze");
    let mut revisions = vec![];
    for step in steps(analyze) {
        let Some(action) = step.get("uses").and_then(Value::as_str) else {
            continue;
        };
        if action.starts_with("github/codeql-action/") {
            revisions.push(action.split_once('@').unwrap().1);
            assert_eq!(
                step.get("if").and_then(Value::as_str),
                Some("steps.sources.outputs.has_code == 'true'")
            );
            if action.contains("/init@") {
                assert_eq!(
                    step.get("with")
                        .unwrap()
                        .get("build-mode")
                        .and_then(Value::as_str),
                    Some("none")
                );
            }
        }
    }
    assert_eq!(revisions.len(), 2);
    assert_eq!(revisions[0], revisions[1]);
}
#[test]
fn extracted_aggregator_executes_result_matrix() {
    // `bash_command`, not a bare `Command::new("bash")`: on Windows the latter finds the WSL
    // launcher in System32 instead of Git Bash.
    if !repo_guards::bash_command()
        .args(["-c", "command -v jq"])
        .output()
        .is_ok_and(|o| o.status.success())
    {
        println!("skipped aggregator execution: bash/jq unavailable");
        return;
    }
    let doc = workflow(".github/workflows/ci.yml");
    let script = steps(job(&doc, "ci"))
        .iter()
        .map(|s| run_text(s))
        .collect::<Vec<_>>()
        .join("\n");
    for name in needs_of(job(&doc, "ci")) {
        for result in ["success", "skipped", "failure", "cancelled"] {
            let input = serde_json::json!({name.clone(): {"result": result}}).to_string();
            let output = repo_guards::bash_command()
                .args(["-c", &script])
                .env("NEEDS_JSON", input)
                .output()
                .unwrap();
            let expected =
                result == "success" || (result == "skipped" && GATED_JOBS.contains(&name.as_str()));
            assert_eq!(output.status.success(), expected, "{name}: {result}");
        }
    }
}

#[test]
fn ci_i18n_job_runs_the_js_part_with_node_from_nvmrc() {
    let doc = workflow(".github/workflows/ci.yml");
    let i18n = job(&doc, "i18n");
    assert_eq!(
        i18n.get("runs-on").and_then(Value::as_str),
        Some("ubuntu-latest")
    );
    let setup = steps(i18n)
        .into_iter()
        .find(|s| {
            s.get("uses")
                .and_then(Value::as_str)
                .is_some_and(|u| u.starts_with("actions/setup-node@"))
        })
        .expect("job `i18n` must set up Node through actions/setup-node");
    assert_eq!(
        setup
            .get("with")
            .and_then(|w| w.get("node-version-file"))
            .and_then(Value::as_str),
        Some(".nvmrc"),
        "Node must come from .nvmrc"
    );
    let runs: Vec<String> = steps(i18n).iter().map(|s| run_text(s)).collect();
    let step = runs
        .iter()
        .find(|r| r.contains("scripts/test.sh"))
        .expect("job `i18n` must run scripts/test.sh");
    assert!(
        step.contains("--ci") && step.contains("--only js"),
        "expected `scripts/test.sh --ci --only js`, found `{step}`"
    );
    // The i18n job must be a hard requirement of the aggregator, never a tolerated skip.
    assert!(!GATED_JOBS.contains(&"i18n"));
}
