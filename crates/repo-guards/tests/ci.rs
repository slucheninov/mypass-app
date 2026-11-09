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
        Some("ci-${{ github.ref }}")
    );
    assert_eq!(
        concurrency
            .get("cancel-in-progress")
            .and_then(Value::as_bool),
        Some(true)
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
                run_text(s)
                    .lines()
                    .any(|l| l.trim_start().starts_with("cargo ") || l.contains("scripts/test.sh"))
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
