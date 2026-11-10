//! Guards for `scripts/test.sh`: it parses, and every flag-validation path rejects bad input
//! before it would run cargo. None of these tests runs cargo.
//!
//! test-only support code: helpers here unwrap and index freely, a guard must fail loudly.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use repo_guards::{bash_command, repo_root};
use std::io::ErrorKind;
use std::process::Output;

/// Run `bash <args>` from the repo root. `None` (after printing a note) when bash is not on PATH.
/// `bash_command` picks Git Bash over the WSL launcher in `System32` on Windows.
fn bash(args: &[&str]) -> Option<Output> {
    match bash_command().args(args).current_dir(repo_root()).output() {
        Ok(out) => Some(out),
        Err(e) if e.kind() == ErrorKind::NotFound => {
            println!("skipped: bash is not on PATH");
            None
        }
        Err(e) => panic!("cannot run bash: {e}"),
    }
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn test_sh_is_valid_bash() {
    let Some(out) = bash(&["-n", "scripts/test.sh"]) else {
        return;
    };
    assert!(out.status.success(), "bash -n failed: {}", stderr(&out));
}

#[test]
fn test_sh_rejects_only_with_no_js() {
    let Some(out) = bash(&["scripts/test.sh", "--only", "rust", "--no-js"]) else {
        return;
    };
    assert_eq!(out.status.code(), Some(2), "stderr: {}", stderr(&out));
    assert!(stderr(&out).contains("--only"));
}

#[test]
fn test_sh_rejects_ci_with_no_js() {
    let Some(out) = bash(&["scripts/test.sh", "--ci", "--no-js"]) else {
        return;
    };
    assert_eq!(out.status.code(), Some(2), "stderr: {}", stderr(&out));
    assert!(stderr(&out).contains("--ci"));
}

#[test]
fn test_sh_rejects_unknown_only_value() {
    let Some(out) = bash(&["scripts/test.sh", "--only", "bogus"]) else {
        return;
    };
    assert_eq!(out.status.code(), Some(2), "stderr: {}", stderr(&out));
    assert!(stderr(&out).contains("bogus"));
}

#[test]
fn test_sh_help_exits_zero() {
    let Some(out) = bash(&["scripts/test.sh", "--help"]) else {
        return;
    };
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let stdout = String::from_utf8_lossy(&out.stdout);
    for flag in ["--ci", "--only", "--no-js"] {
        assert!(stdout.contains(flag), "--help does not mention {flag}");
    }
}

#[test]
fn rust_test_entrypoint_checks_generated_notices() {
    let script = repo_guards::read("scripts/test.sh");
    assert!(
        script.contains("python3 crates/mypass-kdbx/tests/crosscompat/update_notices.py --check")
    );
    assert!(script.contains("need_tool python3"));
    assert!(script.contains("notices: skipped"));
}

#[test]
fn rust_test_entrypoint_reports_every_failing_test_binary() {
    // Without `--no-fail-fast` cargo stops at the first failing test binary and hides failures
    // in the later ones.
    let script = repo_guards::read("scripts/test.sh");
    assert!(script.contains("local args=(test --workspace --no-fail-fast)"));
    assert!(script.contains("args+=(--locked)"));
}

#[test]
fn test_sh_ui_part_says_plainly_when_no_ui_suite_exists() {
    // package.json carries only the tooling pins until stage 2b adds a `test` script. The ui part
    // must then pass without pnpm AND say that nothing ran: it never pretends to have tested.
    let script = repo_guards::read("scripts/test.sh");
    assert!(script.contains("ui: NOT RUN"));
    let has_test_script = repo_guards::read("package.json").contains("\"test\"");
    if has_test_script {
        return;
    }
    let Some(out) = bash(&["scripts/test.sh", "--only", "ui"]) else {
        return;
    };
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("ui: NOT RUN"), "stdout: {stdout}");
}
