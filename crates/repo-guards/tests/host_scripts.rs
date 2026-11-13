//! Guards for the host build and dev scripts: they read the scripts as text and never sign or build.
//!
//! test-only support code: helpers here unwrap freely, a guard must fail loudly.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use repo_guards::{bash_command, read, repo_root};

fn bash_n(script: &str) {
    let out = match bash_command()
        .args(["-n", script])
        .current_dir(repo_root())
        .output()
    {
        Ok(out) => out,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            println!("skipped: bash is not on PATH");
            return;
        }
        Err(e) => panic!("cannot run bash: {e}"),
    };
    assert!(
        out.status.success(),
        "bash -n {script}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn build_host_sh_is_strict_and_bash_clean() {
    assert!(read("scripts/build-host.sh").contains("set -euo pipefail"));
    bash_n("scripts/build-host.sh");
}

#[test]
fn dev_app_sh_is_strict_and_bash_clean() {
    assert!(read("scripts/dev-app.sh").contains("set -euo pipefail"));
    bash_n("scripts/dev-app.sh");
}

#[test]
fn build_host_sh_signs_only_when_identity_set_and_never_ad_hoc() {
    let text = read("scripts/build-host.sh");
    assert!(text.contains("if [[ -n \"${MYPASS_SIGNING_IDENTITY:-}\" ]]"));
    assert!(text.contains("codesign --force --sign \"$MYPASS_SIGNING_IDENTITY\""));
    let code: Vec<&str> = text
        .lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .collect();
    assert_eq!(code.iter().filter(|l| l.contains("codesign ")).count(), 1);
    for line in code {
        assert!(!line.contains("--sign -"), "ad-hoc signing: {line}");
        assert!(!line.contains("--sign '-'"), "ad-hoc signing: {line}");
    }
}

#[test]
fn dev_app_sh_runs_build_host_first_and_passes_identity_unchanged() {
    let text = read("scripts/dev-app.sh");
    let build = text.find("scripts/build-host.sh").unwrap();
    let tauri = text.find("pnpm tauri build").unwrap();
    assert!(build < tauri);
    assert!(text.contains("APPLE_SIGNING_IDENTITY=\"$MYPASS_SIGNING_IDENTITY\""));
}

/// Runs the script with the identity set; the refusal comes before any build, so this is cheap.
fn run_with_identity(script: &str, identity: &str) -> Option<std::process::Output> {
    match bash_command()
        .arg(script)
        .env("MYPASS_SIGNING_IDENTITY", identity)
        .current_dir(repo_root())
        .output()
    {
        Ok(out) => Some(out),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            println!("skipped: bash is not on PATH");
            None
        }
        Err(e) => panic!("cannot run bash: {e}"),
    }
}

#[test]
fn scripts_refuse_ad_hoc_and_empty_identity_before_building() {
    for script in ["scripts/build-host.sh", "scripts/dev-app.sh"] {
        for identity in ["-", ""] {
            let Some(out) = run_with_identity(script, identity) else {
                return;
            };
            let err = String::from_utf8_lossy(&out.stderr);
            assert_eq!(out.status.code(), Some(2), "{script} {identity:?}: {err}");
            assert!(err.contains("ad-hoc"), "{script}: {err}");
            // Refused before cargo ran.
            assert!(!err.contains("Compiling"), "{script}: {err}");
        }
    }
}

#[test]
fn dev_app_sh_accepts_release_and_rejects_unknown_flags() {
    let text = read("scripts/dev-app.sh");
    assert!(text.contains("--release"));
    let out = bash_command()
        .args(["scripts/dev-app.sh", "--bogus"])
        .current_dir(repo_root())
        .output();
    if let Ok(out) = out {
        assert_eq!(out.status.code(), Some(2));
    }
}

#[test]
fn no_script_or_workflow_reintroduces_the_secret_store_override() {
    for rel in ["scripts/build-host.sh", "scripts/dev-app.sh"] {
        let text = read(rel);
        assert!(!text.contains("MYPASS_SECRET_STORE"), "{rel}");
        assert!(!text.contains("dev-secret-store"), "{rel}");
    }
}
