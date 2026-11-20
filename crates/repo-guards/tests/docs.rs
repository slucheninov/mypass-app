//! Guards for the project documents: roadmap, AGENTS.md, CONTRIBUTING, SECURITY and the GitHub
//! templates, plus the repository-wide Cyrillic and docs-layout rules.
//!
//! test-only support code: helpers here unwrap and index freely, a guard must fail loudly.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use repo_guards::{read, rel_str, repo_root, tracked_files, working_tree_files};

/// The sentence D11 requires verbatim in AGENTS.md.
const MANUAL_CHECKS_SENTENCE: &str = "Manual-only checks (signed-build, Windows, real-vault) are walked by the owner in daily use and never block a merge or a release; a task whose only open work is a manual check is complete once its automated part is green.";

/// Collapse all whitespace runs (including hard line wraps) to single spaces.
fn squash(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[test]
fn roadmap_has_only_open_boxes() {
    let roadmap = read("docs/implementation/ROADMAP.md");
    assert!(
        !roadmap.contains("[x]") && !roadmap.contains("[X]"),
        "ROADMAP.md lists only unfinished work"
    );
    assert!(
        roadmap.contains("[ ]"),
        "ROADMAP.md must contain at least one open box"
    );
}

#[test]
fn roadmap_lists_open_stage_followups() {
    // Completed implementations keep a heading while deferred/manual follow-ups remain.
    // Remove a stage here only when its last open follow-up has actually been resolved.
    let roadmap = read("docs/implementation/ROADMAP.md");
    let headings: Vec<&str> = roadmap.lines().filter(|l| l.starts_with('#')).collect();
    for stage in [
        "Stage 2a", "Stage 2b", "Stage 3", "Stage 4", "Stage 5", "Stage 6",
    ] {
        assert!(
            headings.iter().any(|h| h.contains(stage)),
            "ROADMAP.md has no heading for `{stage}`"
        );
    }
    assert!(
        roadmap
            .lines()
            .any(|l| l.trim_start().starts_with("- [ ]") && l.contains("Change master password")),
        "ROADMAP.md must list the open item `Change master password`"
    );
}

#[test]
fn agents_md_has_no_harness_specific_tool_names() {
    let agents = read("AGENTS.md");
    // Only the intro (first 12 lines) may name the harnesses that read this file.
    let body: String = agents.lines().skip(12).collect::<Vec<_>>().join("\n");
    for banned in [
        "Claude Code",
        "Codex",
        "Cursor",
        "Agent(",
        "Task(",
        ".cursor",
        ".claude",
    ] {
        assert!(
            !body.contains(banned),
            "AGENTS.md mentions `{banned}` outside its intro"
        );
    }
}

#[test]
fn agents_md_does_not_port_retired_tool_lists() {
    let agents = read("AGENTS.md");
    for banned in ["claude-mem", "gsd-"] {
        assert!(
            !agents.contains(banned),
            "AGENTS.md must not mention `{banned}`"
        );
    }
}

#[test]
fn agents_md_states_manual_checks_never_block_merge() {
    assert!(
        squash(&read("AGENTS.md")).contains(MANUAL_CHECKS_SENTENCE),
        "AGENTS.md must contain the D11 sentence verbatim"
    );
}

#[test]
fn agents_md_references_scripts_test_sh() {
    assert!(read("AGENTS.md").contains("scripts/test.sh"));
}

#[test]
fn claude_md_points_to_agents_md() {
    let claude = read("CLAUDE.md");
    assert!(claude.contains(
        "Read and follow all instructions in @AGENTS.md before working in this repository."
    ));
    assert!(claude.lines().any(|l| l.trim() == "@AGENTS.md"));
}

/// Link targets of inline `[text](target)` links and reference definitions `[id]: target`.
fn link_targets(markdown: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in markdown.lines() {
        let mut rest = line;
        while let Some(pos) = rest.find("](") {
            let after = &rest[pos + 2..];
            let end = after.find(')').unwrap_or(after.len());
            out.push(after[..end].trim().to_owned());
            rest = &after[end..];
        }
        let t = line.trim_start();
        if t.starts_with('[') {
            if let Some((_, target)) = t.split_once("]:") {
                out.push(target.trim().to_owned());
            }
        }
    }
    out
}

#[test]
fn tracked_docs_never_link_docs_superpowers() {
    let Some(files) = tracked_files() else {
        println!("skipped: git is not available");
        return;
    };
    for file in files {
        let rel = rel_str(&file);
        if !rel.ends_with(".md") || !repo_root().join(&file).is_file() {
            continue;
        }
        let text = read(&rel);
        for target in link_targets(&text) {
            assert!(
                !target.contains("docs/superpowers/")
                    && !target.contains("superpowers/specs")
                    && !target.contains("superpowers/plans"),
                "{rel} links to local-only `{target}`; docs/superpowers/ is gitignored"
            );
        }
    }
}

#[test]
fn docs_dir_tracks_only_the_roadmap() {
    let Some(files) = tracked_files() else {
        println!("skipped: git is not available");
        return;
    };
    let mut docs: Vec<String> = files
        .iter()
        .map(|f| rel_str(f))
        .filter(|f| f.starts_with("docs/") && !f.starts_with("docs/superpowers/"))
        .collect();
    docs.sort();
    assert_eq!(docs, ["docs/implementation/ROADMAP.md"]);
}

/// Cyrillic and its supplements: U+0400-052F, U+1C80-1C8F, U+2DE0-2DFF, U+A640-A69F.
/// Written as numeric ranges so this file itself contains no Cyrillic character.
fn has_cyrillic(text: &str) -> bool {
    text.chars().any(|c| {
        matches!(c as u32, 0x0400..=0x052F | 0x1C80..=0x1C8F | 0x2DE0..=0x2DFF | 0xA640..=0xA69F)
    })
}

fn cyrillic_allowed(rel: &str) -> bool {
    rel.ends_with(".xcstrings")
        || rel == "RUN.md"
        || rel == "crates/mypass-kdbx/tests/unicode.rs"
        // Synthetic encrypted fixtures are opaque binary data. Keep their README and
        // generator subject to the normal text rules.
        || (rel.starts_with("crates/mypass-kdbx/tests/fixtures/") && rel.ends_with(".kdbx"))
        // Vendored upstream data (Public Suffix List, Apple password rules) is added verbatim by
        // later tasks and is not ours to rewrite; the PSL contains internationalised names.
        || rel.starts_with("crates/mypass-core/resources/")
        // The UI translation catalogs and the macOS InfoPlist translations. Exactly these files:
        // the English catalog stays Cyrillic-free (pinned by the Node completeness tests).
        || matches!(
            rel,
            // The extension keeps localized keyword lists (and the tests that pin them) as copied.
            "extension/shared/form-detection.js"
                | "extension/shared/page-identifier.js"
                | "extension/tests/formDetection.test.js"
                | "extension/tests/inlineIcon.test.js"
                | "extension/tests/pageIdentifier.test.js"
                | "src/lib/i18n/locales/uk.json"
                | "src/lib/i18n/locales/ru.json"
                | "src-tauri/resources/macos/uk.lproj/InfoPlist.strings"
                | "src-tauri/resources/macos/ru.lproj/InfoPlist.strings"
        )
}

#[test]
fn cyrillic_allowlist_covers_only_the_translation_files() {
    assert!(cyrillic_allowed("src/lib/i18n/locales/uk.json"));
    assert!(cyrillic_allowed("src/lib/i18n/locales/ru.json"));
    assert!(cyrillic_allowed(
        "src-tauri/resources/macos/uk.lproj/InfoPlist.strings"
    ));
    assert!(cyrillic_allowed(
        "src-tauri/resources/macos/ru.lproj/InfoPlist.strings"
    ));
    for f in [
        "extension/shared/form-detection.js",
        "extension/shared/page-identifier.js",
        "extension/tests/formDetection.test.js",
        "extension/tests/inlineIcon.test.js",
        "extension/tests/pageIdentifier.test.js",
    ] {
        assert!(cyrillic_allowed(f), "{f}");
    }
    assert!(!cyrillic_allowed("extension/background.js"));
    assert!(!cyrillic_allowed("extension/content.js"));
    assert!(!cyrillic_allowed("src/lib/i18n/locales/en.json"));
    // German has no Cyrillic: its catalog and InfoPlist.strings stay under the plain-text rule.
    assert!(!cyrillic_allowed("src/lib/i18n/locales/de.json"));
    assert!(!cyrillic_allowed(
        "src-tauri/resources/macos/de.lproj/InfoPlist.strings"
    ));
    assert!(!cyrillic_allowed("src/lib/i18n/index.ts"));
    assert!(!cyrillic_allowed("scripts/convert-xcstrings.test.mjs"));
    assert!(!cyrillic_allowed(
        "src-tauri/resources/macos/en.lproj/InfoPlist.strings"
    ));
}

#[test]
fn cyrillic_detector_detects_cyrillic() {
    assert!(has_cyrillic("\u{43f}\u{440}\u{438}\u{432}\u{456}\u{442}"));
    assert!(!has_cyrillic("privit, plain ASCII and caf\u{e9}"));
}

fn cyrillic_violations(root: &std::path::Path) -> Option<Vec<String>> {
    let files = working_tree_files(root)?;
    Some(
        files
            .into_iter()
            .filter_map(|file| {
                let rel = rel_str(&file);
                if cyrillic_allowed(&rel) {
                    return None;
                }
                let bytes = std::fs::read(root.join(&file)).ok()?;
                let text = String::from_utf8(bytes).ok()?;
                has_cyrillic(&text).then_some(rel)
            })
            .collect(),
    )
}

#[test]
fn cyrillic_only_in_allowed_files() {
    let Some(violations) = cyrillic_violations(&repo_root()) else {
        println!("skipped: git is not available");
        return;
    };
    assert!(violations.is_empty(), "{violations:?} contain Cyrillic; use \\u{{..}} escapes (see AGENTS.md for the exact allow-list)");
}

#[test]
fn cyrillic_guard_checks_unstaged_windows_sources_and_preserves_exemptions() {
    use std::{
        fs,
        process::Command,
        time::{SystemTime, UNIX_EPOCH},
    };
    // Isolated inventory: never stage files or change ignore rules in the real repository.
    struct Scratch(std::path::PathBuf);
    impl Drop for Scratch {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }
    let root = Scratch(std::env::temp_dir().join(format!("mypass-cyrillic-{}-{}",
        std::process::id(), SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos())));
    fs::create_dir(&root.0).unwrap();
    let git = |args: &[&str]| {
        assert!(Command::new("git")
            .arg("-C")
            .arg(&root.0)
            .args(args)
            .output()
            .unwrap()
            .status
            .success());
    };
    git(&["init", "--quiet"]);
    fs::write(root.0.join("tracked.rs"), "// \u{41f}").unwrap();
    git(&["add", "tracked.rs"]);
    fs::create_dir_all(root.0.join("crates/example/tests")).unwrap();
    let untracked = "crates/example/tests/windows_paths.rs";
    fs::write(root.0.join(untracked), "#![cfg(windows)]\n// \u{41f}").unwrap();
    fs::write(root.0.join(".gitignore"), "ignored.rs\n").unwrap();
    fs::write(root.0.join("ignored.rs"), "// \u{41f}").unwrap();
    fs::write(root.0.join("RUN.md"), "\u{41f}").unwrap();
    fs::write(root.0.join("escaped.rs"), r"// \u{41f}").unwrap();
    assert_eq!(
        cyrillic_violations(&root.0).unwrap(),
        [untracked, "tracked.rs"]
    );
    // Staging must not hide or duplicate a violation; ignored tracked files still count.
    git(&["add", untracked]);
    git(&["add", "--force", "ignored.rs"]);
    assert_eq!(
        cyrillic_violations(&root.0).unwrap(),
        [untracked, "ignored.rs", "tracked.rs"]
    );
}

#[test]
fn contributing_states_licensing_rule() {
    let contributing = read("CONTRIBUTING.md");
    assert!(contributing.contains("THIRD_PARTY_NOTICES.md"));
    assert!(contributing.contains("Apache-2.0"));
}

#[test]
fn security_md_has_private_reporting_channel() {
    let security = read("SECURITY.md");
    assert!(
        security.contains("security/advisories/new"),
        "SECURITY.md must link the private advisory form"
    );
    assert!(security
        .to_lowercase()
        .contains("private vulnerability reporting"));
    assert!(security.contains("Report a vulnerability"));
}

#[test]
fn issue_templates_and_pr_template_exist_and_parse() {
    for form in ["bug_report.yml", "feature_request.yml"] {
        let rel = format!(".github/ISSUE_TEMPLATE/{form}");
        let doc: serde_yaml_ng::Value = serde_yaml_ng::from_str(&read(&rel))
            .unwrap_or_else(|e| panic!("{rel} does not parse: {e}"));
        assert!(doc.get("name").is_some(), "{rel} needs `name`");
        assert!(
            doc.get("body")
                .and_then(|b| b.as_sequence())
                .is_some_and(|b| !b.is_empty()),
            "{rel} needs `body`"
        );
    }
    let config: serde_yaml_ng::Value =
        serde_yaml_ng::from_str(&read(".github/ISSUE_TEMPLATE/config.yml"))
            .expect("config.yml parses");
    assert!(
        config.get("blank_issues_enabled").is_some(),
        "config.yml needs `blank_issues_enabled`"
    );
    assert!(!read(".github/pull_request_template.md").trim().is_empty());
}

#[test]
fn every_locked_package_has_a_third_party_notice() {
    let lock: toml::Value = toml::from_str(&read("Cargo.lock")).unwrap();
    let notices = read("THIRD_PARTY_NOTICES.md");
    for package in lock["package"].as_array().unwrap() {
        // Workspace packages are first-party, not third-party components.
        if package.get("source").is_none() {
            continue;
        }
        let name = package["name"].as_str().unwrap();
        let version = package["version"].as_str().unwrap();
        let row = format!("| `{name}` | {version} |");
        assert!(
            notices.contains(&row),
            "missing notice for {name} {version}"
        );
    }
}

#[test]
fn agents_md_lists_the_bridge_crates_extension_and_scripts() {
    let agents = read("AGENTS.md");
    for needle in [
        "mypass-bridge",
        "mypass-host",
        "extension/",
        "scripts/test-extension.sh",
        "scripts/build-host.sh",
        "scripts/dev-app.sh",
        "RUN.md",
        "Browser bridge.",
        "browser_proposal_take",
    ] {
        assert!(agents.contains(needle), "AGENTS.md must mention {needle}");
    }
    for stale in ["| in progress (stage 3) |", "wire up in progress"] {
        assert!(!agents.contains(stale), "stale stage 3 wording: {stale}");
    }
}

#[test]
fn run_md_has_the_stage_3_sections_and_only_open_boxes() {
    let run = read("RUN.md");
    assert!(has_cyrillic(&run), "RUN.md is the Ukrainian checklist");
    for needle in [
        "**M1 ",
        "**M14 ",
        "Unlock MyPass",
        "python3 -m http.server 8765",
        "extension/tests/fixtures",
        "127.0.0.1:8766",
        "scripts/dev-app.sh",
    ] {
        assert!(run.contains(needle), "RUN.md must contain {needle}");
    }
    assert!(
        !run.contains("SFTP"),
        "SFTP is removed from the product; RUN.md must not mention it"
    );
    for line in run.lines().filter(|l| l.trim_start().starts_with("- [")) {
        assert!(
            line.trim_start().starts_with("- [ ]"),
            "RUN.md lists unwalked checks only: {line}"
        );
    }
}

#[test]
fn run_md_has_the_stage_5_sections_and_only_open_boxes() {
    let run = read("RUN.md");
    for prefix in ["S", "C"] {
        let end = if prefix == "S" { 21 } else { 10 };
        for n in 1..=end {
            let marker = format!("**{prefix}{n} ");
            assert_eq!(
                run.matches(&marker).count(),
                1,
                "missing or duplicate {marker}"
            );
        }
    }
    for line in run.lines().filter(|l| l.trim_start().starts_with("- [")) {
        assert!(
            line.trim_start().starts_with("- [ ]"),
            "unwalked only: {line}"
        );
    }
}

#[test]
fn agents_documents_stage_5_delivery_and_remaining_boundaries() {
    let agents = squash(&read("AGENTS.md"));
    for needle in [
        "`mypass-sync`",
        "`mypass-update`",
        "scripts/package-macos.sh",
        "release.yml",
        "Stage 5",
        "Apple Development",
        "notarization",
        "SecAccessControl",
    ] {
        assert!(agents.contains(needle), "AGENTS.md must mention {needle}");
    }
}

#[test]
fn agents_md_documents_windows_layout() {
    let agents = squash(&read("AGENTS.md"));
    for needle in [
        r"%LOCALAPPDATA%\MyPass",
        "MYPASS_WINDOWS_INTEGRATION_TESTS",
        "MYPASS_WINDOWS_DATA_ROOT",
        "DPAPI",
        "Credential Manager",
        r"HKCU\Software\Google\Chrome\NativeMessagingHosts\com.mypass.chromehost",
        r"HKCU\Software\Microsoft\Edge\NativeMessagingHosts\com.mypass.chromehost",
        "scripts/package-windows.sh",
        "content",
        "digest",
        "same-user",
        "StartupApproved",
        "windows-x64.zip.sha256",
        "Stage 6",
    ] {
        assert!(
            agents.contains(needle),
            "missing Windows contract: {needle}"
        );
    }
    assert_eq!(agents.matches(MANUAL_CHECKS_SENTENCE).count(), 1);
    assert!(!agents.contains("Windows bodies in stage 6"));
}

#[test]
fn third_party_notices_lists_windows_crate_and_7zip_note() {
    let notices = read("THIRD_PARTY_NOTICES.md");
    assert!(notices.contains("| `windows` | 0.62.2 | MIT OR Apache-2.0 |"));
    assert!(notices.contains("7-Zip is a build tool, not shipped"));
}

#[test]
fn roadmap_windows_items() {
    let roadmap = read("docs/implementation/ROADMAP.md");
    let windows = roadmap
        .split("## Stage 6 Windows")
        .nth(1)
        .unwrap()
        .split("\n## ")
        .next()
        .unwrap();
    for needle in [
        "Windows Hello",
        "Authenticode",
        "installer",
        "ARM64",
        "snap-layout",
        "Brave/Vivaldi/Opera",
        "W1",
        "W17",
    ] {
        assert!(
            windows.contains(needle),
            "missing deferred Windows item: {needle}"
        );
    }
    for stale in [
        "Windows bodies of every",
        "UnsupportedSecretStore",
        "Windows jobs in CI and",
    ] {
        assert!(!windows.contains(stale), "implemented item: {stale}");
    }
}

#[test]
fn run_md_has_windows_section() {
    let run = read("RUN.md");
    assert_eq!(run.matches("## Stage 6 Windows").count(), 1);
    let windows = run.split("## Stage 6 Windows").nth(1).unwrap();
    for n in 1..=17 {
        let heading = format!("### W{n} ");
        assert_eq!(windows.matches(&heading).count(), 1, "{heading}");
        let item = windows
            .split(&heading)
            .nth(1)
            .unwrap()
            .split("\n### ")
            .next()
            .unwrap();
        // Expected-result label, escaped to preserve the repository's source language rule.
        assert!(item.contains("\u{41e}\u{447}\u{456}\u{43a}\u{443}\u{432}\u{430}\u{43d}\u{438}\u{439} \u{440}\u{435}\u{437}\u{443}\u{43b}\u{44c}\u{442}\u{430}\u{442}:"), "W{n} needs an expected result");
        assert!(item.contains("- [ ]"));
    }
    for needle in [
        "userVerification",
        "required",
        "preferred",
        "discouraged",
        "StartupApproved",
        "icacls",
        "SYSTEM",
        "ReparsePoint",
    ] {
        assert!(windows.contains(needle), "missing owner check: {needle}");
    }
}

#[test]
fn readme_install_section_links_windows_zip() {
    let readme = read("README.md");
    let install = readme
        .split("## Install")
        .nth(1)
        .unwrap()
        .split("\n## ")
        .next()
        .unwrap();
    assert!(install.contains("windows-x64.zip"));
    assert!(install.contains("https://github.com/slucheninov/mypass-app/releases"));
    assert!(install.contains("Unblock"));
    assert!(!install.contains("run the installer"));
}
