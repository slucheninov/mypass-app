//! Guards for the Cargo workspace: member list, exact dependency pins, lints and toolchain pins.
//!
//! test-only support code: helpers here unwrap and index freely, a guard must fail loudly.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use repo_guards::{read, rel_str, repo_root, tracked_files, workflow_files};
use std::path::PathBuf;
use toml::{Table, Value};

fn root_manifest() -> Table {
    read("Cargo.toml")
        .parse::<Table>()
        .expect("Cargo.toml parses")
}

fn workspace_table(root: &Table) -> &Table {
    root.get("workspace")
        .and_then(Value::as_table)
        .expect("[workspace] exists")
}

/// Manifests of every workspace member that exists on disk: `crates/*` plus `src-tauri` once present.
fn member_manifests() -> Vec<(String, Table)> {
    let mut paths: Vec<PathBuf> = Vec::new();
    let crates_dir = repo_root().join("crates");
    for entry in std::fs::read_dir(&crates_dir).expect("crates/ exists") {
        let manifest = entry.unwrap().path().join("Cargo.toml");
        if manifest.is_file() {
            paths.push(manifest);
        }
    }
    let tauri = repo_root().join("src-tauri").join("Cargo.toml");
    if tauri.is_file() {
        paths.push(tauri);
    }
    paths.sort();
    paths
        .into_iter()
        .map(|p| {
            let text = std::fs::read_to_string(&p).unwrap();
            let table = text
                .parse::<Table>()
                .unwrap_or_else(|e| panic!("{} does not parse: {e}", p.display()));
            (p.display().to_string(), table)
        })
        .collect()
}

/// A full `x.y.z` version, optionally with a pre-release suffix (`2.0.0-rc.26`).
fn is_full_version(v: &str) -> bool {
    let core = v.split('-').next().unwrap_or(v);
    let parts: Vec<&str> = core.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
}

fn must_be_eq_pinned(name: &str) -> bool {
    name == "tauri"
        || name == "tauri-build"
        || name.starts_with("tauri-plugin-")
        || name == "specta"
        || name == "tauri-specta"
}

fn dependency_version(name: &str, value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Table(t) => t
            .get("version")
            .and_then(Value::as_str)
            .unwrap_or_else(|| panic!("workspace dependency `{name}` has no `version`"))
            .to_owned(),
        other => panic!("workspace dependency `{name}` has unexpected shape {other:?}"),
    }
}

#[test]
fn workspace_members_are_the_crates_glob_until_src_tauri_exists() {
    let root = root_manifest();
    let members: Vec<&str> = workspace_table(&root)
        .get("members")
        .and_then(Value::as_array)
        .expect("workspace.members is an array")
        .iter()
        .map(|v| v.as_str().expect("member is a string"))
        .collect();
    let expected: Vec<&str> = if repo_root().join("src-tauri").join("Cargo.toml").exists() {
        vec!["crates/*", "src-tauri"]
    } else {
        vec!["crates/*"]
    };
    assert_eq!(members, expected);
}

#[test]
fn workspace_dependencies_are_exactly_pinned() {
    let root = root_manifest();
    let deps = workspace_table(&root)
        .get("dependencies")
        .and_then(Value::as_table)
        .expect("[workspace.dependencies] exists");

    for (name, value) in deps {
        if let Value::Table(t) = value {
            assert!(
                !t.contains_key("path") && !t.contains_key("git"),
                "`{name}`: workspace dependencies come from the registry, not path or git"
            );
        }
        let version = dependency_version(name, value);
        let (eq, bare) = match version.strip_prefix('=') {
            Some(rest) => (true, rest),
            None => (false, version.as_str()),
        };
        assert!(
            is_full_version(bare),
            "`{name}` must carry a full x.y.z version, found `{version}`"
        );
        if must_be_eq_pinned(name) {
            assert!(eq, "`{name}` must be pinned with `=`, found `{version}`");
        }
    }

    // The table is the single source of versions for stages 1-3: the crates the plans name must
    // all be declared (a missing one would make a member invent its own version literal).
    for required in [
        "aes",
        "cbc",
        "chacha20",
        "salsa20",
        "hmac",
        "sha1",
        "sha2",
        "subtle",
        "argon2",
        "flate2",
        "zeroize",
        "secrecy",
        "quick-xml",
        "uuid",
        "base64",
        "thiserror",
        "idna",
        "url",
        "serde",
        "serde_json",
        "data-encoding",
        "rand",
        "getrandom",
        "tempfile",
        "proptest",
        "keepass",
        "toml",
        "serde_yaml_ng",
        "tauri",
        "tauri-build",
        "tauri-plugin-single-instance",
        "tauri-plugin-global-shortcut",
        "tauri-plugin-dialog",
        "tauri-plugin-opener",
        "specta",
        "tauri-specta",
    ] {
        assert!(
            deps.contains_key(required),
            "`{required}` missing from [workspace.dependencies]"
        );
    }
}

#[test]
fn member_crates_take_versions_only_from_the_workspace_table() {
    let root = root_manifest();
    let table = workspace_table(&root)
        .get("dependencies")
        .and_then(Value::as_table)
        .expect("[workspace.dependencies] exists");

    fn check_section(manifest: &str, section: &str, deps: &Table, table: &Table) {
        for (name, value) in deps {
            let ctx = format!("{manifest}: [{section}] `{name}`");
            match value {
                Value::String(v) => panic!("{ctx} declares a version literal `{v}`"),
                Value::Table(t) => {
                    assert!(
                        !t.contains_key("version"),
                        "{ctx} declares a version literal"
                    );
                    let from_workspace = t.get("workspace").and_then(Value::as_bool) == Some(true);
                    let is_path = t.contains_key("path");
                    assert!(
                        from_workspace || is_path,
                        "{ctx} must use `workspace = true` (or `path` for a workspace member)"
                    );
                    if from_workspace {
                        assert!(
                            table.contains_key(name.as_str()),
                            "{ctx} is not in [workspace.dependencies]; add it there with an exact version"
                        );
                    }
                }
                other => panic!("{ctx} has unexpected shape {other:?}"),
            }
        }
    }

    let kinds = ["dependencies", "dev-dependencies", "build-dependencies"];
    for (manifest, doc) in member_manifests() {
        for kind in kinds {
            if let Some(deps) = doc.get(kind).and_then(Value::as_table) {
                check_section(&manifest, kind, deps, table);
            }
        }
        if let Some(targets) = doc.get("target").and_then(Value::as_table) {
            for (cfg, target) in targets {
                for kind in kinds {
                    if let Some(deps) = target.get(kind).and_then(Value::as_table) {
                        check_section(&manifest, &format!("target.{cfg}.{kind}"), deps, table);
                    }
                }
            }
        }
    }
}

#[test]
fn workspace_lints_forbid_unsafe() {
    let root = root_manifest();
    let lints = workspace_table(&root)
        .get("lints")
        .and_then(Value::as_table)
        .expect("[workspace.lints] exists");
    let rust = lints
        .get("rust")
        .and_then(Value::as_table)
        .expect("[workspace.lints.rust]");
    assert_eq!(
        rust.get("unsafe_code").and_then(Value::as_str),
        Some("forbid")
    );
    let clippy = lints
        .get("clippy")
        .and_then(Value::as_table)
        .expect("[workspace.lints.clippy]");
    for lint in ["unwrap_used", "expect_used", "indexing_slicing"] {
        assert_eq!(
            clippy.get(lint).and_then(Value::as_str),
            Some("warn"),
            "clippy::{lint}"
        );
    }
    for (manifest, doc) in member_manifests() {
        let name = doc
            .get("package")
            .and_then(|p| p.get("name"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let member_lints = doc.get("lints").and_then(Value::as_table);
        if name == OWN_LINTS_MEMBER {
            // The one exception: the macOS glue needs Objective-C / Carbon calls, so the crate
            // copies the workspace tables verbatim except `unsafe_code = "deny"` (overridable per
            // module, unlike `forbid`). `unsafe_only_in_allow_listed_modules` pins where.
            let member_lints = member_lints
                .unwrap_or_else(|| panic!("{manifest} must have its own [lints] tables"));
            assert!(
                !member_lints.contains_key("workspace"),
                "{manifest} must not inherit the workspace lints (it overrides `unsafe_code`)"
            );
            let mut expected = lints.clone();
            expected
                .get_mut("rust")
                .and_then(Value::as_table_mut)
                .unwrap()
                .insert("unsafe_code".to_owned(), Value::String("deny".to_owned()));
            assert_eq!(
                member_lints, &expected,
                "{manifest}: [lints] must equal [workspace.lints] key by key except `unsafe_code = \"deny\"`"
            );
            continue;
        }
        let inherits = member_lints
            .and_then(|l| l.get("workspace"))
            .and_then(Value::as_bool);
        assert_eq!(
            inherits,
            Some(true),
            "{manifest} must have `[lints] workspace = true`"
        );
    }
}

/// The only member allowed its own `[lints]` tables (see `workspace_lints_forbid_unsafe`).
const OWN_LINTS_MEMBER: &str = "mypass-platform";

/// Files of our crates where `unsafe` code may appear. Stage 6 adds
/// `crates/mypass-platform/src/windows.rs` here for the Win32 calls; nothing else, ever.
const UNSAFE_ALLOWED_FILES: &[&str] = &["crates/mypass-platform/src/macos.rs"];

/// Blanks out comments (line, nested block), string literals (plain and raw) and char literals,
/// keeping every other byte, so token scans see code only. Lifetimes (`'a`) are left alone.
fn strip_comments_and_strings(src: &str) -> String {
    let c: Vec<char> = src.chars().collect();
    let mut out = String::with_capacity(src.len());
    let mut i = 0;
    let blank = |out: &mut String, ch: char| out.push(if ch == '\n' { '\n' } else { ' ' });
    while i < c.len() {
        let at = |k: usize| c.get(i + k).copied();
        match (c[i], at(1)) {
            ('/', Some('/')) => {
                while i < c.len() && c[i] != '\n' {
                    out.push(' ');
                    i += 1;
                }
            }
            ('/', Some('*')) => {
                let mut depth = 0usize;
                while i < c.len() {
                    if c[i] == '/' && c.get(i + 1) == Some(&'*') {
                        depth += 1;
                        out.push_str("  ");
                        i += 2;
                    } else if c[i] == '*' && c.get(i + 1) == Some(&'/') {
                        depth -= 1;
                        out.push_str("  ");
                        i += 2;
                        if depth == 0 {
                            break;
                        }
                    } else {
                        blank(&mut out, c[i]);
                        i += 1;
                    }
                }
            }
            ('r', _) if i == 0 || !is_ident(c[i - 1]) => {
                // Raw string: r"..", r#".."#, with any number of hashes.
                let hashes = c[i + 1..].iter().take_while(|&&h| h == '#').count();
                if c.get(i + 1 + hashes) == Some(&'"') {
                    let mut j = i + 2 + hashes;
                    while j < c.len() {
                        if c[j] == '"' && (1..=hashes).all(|k| c.get(j + k) == Some(&'#')) {
                            j += 1 + hashes;
                            break;
                        }
                        j += 1;
                    }
                    for &ch in &c[i..j.min(c.len())] {
                        blank(&mut out, ch);
                    }
                    i = j;
                } else {
                    out.push('r');
                    i += 1;
                }
            }
            ('"', _) => {
                out.push(' ');
                i += 1;
                while i < c.len() {
                    let ch = c[i];
                    blank(&mut out, ch);
                    i += 1;
                    if ch == '\\' && i < c.len() {
                        blank(&mut out, c[i]);
                        i += 1;
                    } else if ch == '"' {
                        break;
                    }
                }
            }
            ('\'', _) => {
                // A char literal is `'x'` or `'\..'`; anything else is a lifetime.
                let end = if at(1) == Some('\\') {
                    (i + 2..c.len().min(i + 12)).find(|&k| c[k] == '\'')
                } else if at(2) == Some('\'') {
                    Some(i + 2)
                } else {
                    None
                };
                if let Some(end) = end {
                    for &ch in &c[i..=end] {
                        blank(&mut out, ch);
                    }
                    i = end + 1;
                } else {
                    out.push('\'');
                    i += 1;
                }
            }
            (ch, _) => {
                out.push(ch);
                i += 1;
            }
        }
    }
    out
}

fn is_ident(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_'
}

/// Byte offsets where `word` occurs as a whole identifier in comment-free code.
fn word_positions<'a>(code: &'a str, word: &'a str) -> impl Iterator<Item = usize> + 'a {
    code.match_indices(word)
        .map(|(at, _)| at)
        .filter(move |&at| {
            let before = code.get(..at).and_then(|s| s.chars().next_back());
            let after = code.get(at + word.len()..).and_then(|s| s.chars().next());
            !before.is_some_and(is_ident) && !after.is_some_and(is_ident)
        })
}

/// True when comment- and string-free `code` uses the `unsafe` keyword as a block, fn, impl,
/// extern, trait or `#[unsafe(..)]` attribute.
fn uses_unsafe_code(src: &str) -> bool {
    let code = strip_comments_and_strings(src);
    let found = word_positions(&code, "unsafe").any(|at| {
        let rest = code.get(at + "unsafe".len()..).unwrap_or_default();
        let rest = rest.trim_start();
        let before = code.get(..at).unwrap_or_default().trim_end();
        rest.starts_with('{')
            || (rest.starts_with('(') && before.ends_with('['))
            || ["fn", "impl", "extern", "trait"].iter().any(|kw| {
                rest.strip_prefix(kw)
                    .is_some_and(|r| !r.chars().next().is_some_and(is_ident))
            })
    });
    found
}

/// True when `src` names `unsafe_code` inside any `allow(...)` or `expect(...)` attribute (also
/// nested in `cfg_attr`), inner or outer, alone or in a combined list, ignoring comments.
fn allows_unsafe_code(src: &str) -> bool {
    let code = strip_comments_and_strings(src);
    let lint = concat!("unsafe", "_code");
    let found = ["allow", "expect"].iter().any(|opener| {
        let hit = word_positions(&code, opener).any(|at| {
            let rest = code
                .get(at + opener.len()..)
                .unwrap_or_default()
                .trim_start();
            let Some(body) = rest.strip_prefix('(') else {
                return false;
            };
            let mut depth = 1usize;
            let end = body
                .char_indices()
                .find(|&(_, ch)| {
                    match ch {
                        '(' => depth += 1,
                        ')' => depth -= 1,
                        _ => {}
                    }
                    depth == 0
                })
                .map_or(body.len(), |(k, _)| k);
            body.get(..end)
                .unwrap_or_default()
                .split(',')
                .any(|item| item.trim() == lint)
        });
        hit
    });
    found
}

#[test]
fn unsafe_only_in_allow_listed_modules() {
    let Some(files) = tracked_files() else {
        println!("skipped: not a git checkout");
        return;
    };
    let mut offenders = Vec::new();
    for path in files {
        let rel = rel_str(&path);
        let ours = rel.starts_with("crates/") || rel.starts_with("src-tauri/");
        if !ours || !rel.ends_with(".rs") || UNSAFE_ALLOWED_FILES.contains(&rel.as_str()) {
            continue;
        }
        let text = read(&rel);
        if uses_unsafe_code(&text) {
            offenders.push(format!("{rel}: an unsafe block, fn, impl, extern or trait"));
        }
        if allows_unsafe_code(&text) {
            offenders.push(format!("{rel}: an allow/expect of the unsafe-code lint"));
        }
    }
    assert!(
        offenders.is_empty(),
        "unsafe code outside {UNSAFE_ALLOWED_FILES:?}:\n{}",
        offenders.join("\n")
    );
}

#[test]
fn unsafe_detector_detects_unsafe() {
    let kw = "unsafe";
    for sample in [
        format!("fn f() {{ {kw} {{ g() }} }}"),
        format!("fn f() {{ {kw}{{ g() }} }}"),
        format!("{kw}  impl Send for X {{}}"),
        format!("pub {kw}\nfn g() {{}}"),
        format!("{kw} extern \"C\" {{}}"),
        format!("{kw} trait T {{}}"),
        format!("#[{kw}(no_mangle)] fn g() {{}}"),
        format!("/* c */ {kw}/* c */{{ }}"),
        format!("let s = 'a'; {kw} {{ }}"),
    ] {
        assert!(uses_unsafe_code(&sample), "{sample}");
    }
    for sample in [
        format!("// {kw} {{ g() }}"),
        format!("/// {kw} fn is documented here"),
        format!("/* {kw} {{ /* nested */ }} */ fn f() {{}}"),
        format!("let s = \"{kw} {{ g() }}\";"),
        format!("let s = r#\"{kw} fn \"quoted\" \"#;"),
        "let unsafe_code = 1;".to_owned(),
        "fn not_unsafe() {}".to_owned(),
        format!("fn {kw}_fn() {{}}"),
        format!("fn f<'a>(x: &'a str) -> &'a str {{ x }} // {kw} {{"),
    ] {
        assert!(!uses_unsafe_code(&sample), "{sample}");
    }
}

#[test]
fn allow_matcher_sees_combined_and_inner_lists() {
    let lint = concat!("unsafe", "_code");
    for attr in [
        format!("#[allow({lint})]"),
        format!("#![allow({lint})]"),
        format!("#[allow(dead_code, {lint})]"),
        format!("#[allow(dead_code,{lint}, unused)]"),
        format!("#![expect(dead_code, {lint})]"),
        format!("#[allow(\n    dead_code,\n    {lint},\n)]"),
        format!("#[allow({lint} /* c */)]"),
        format!("#[allow(/* c */ {lint})]"),
        format!("#[allow ({lint})]"),
        format!("#[cfg_attr(test, allow({lint}))]"),
    ] {
        assert!(allows_unsafe_code(&attr), "{attr}");
    }
    for text in [
        format!("let {lint} = 1;"),
        "#[allow(dead_code)]".to_owned(),
        format!("#[deny({lint})]"),
        format!("#[allow(dead_code)] // mentions {lint} later"),
        format!("#[allow(not_{lint})]"),
        format!("// #[allow({lint})]"),
        format!("let s = \"{lint} = \\\"deny\\\"\";"),
        format!("let s = \"#[allow({lint})]\";"),
        format!("/// #[allow({lint})]\nfn f() {{}}"),
    ] {
        assert!(!allows_unsafe_code(&text), "{text}");
    }
}

#[test]
fn dependency_profiles_use_opt_level_3() {
    // Argon2 at 64 MiB x 3 passes is minutes at opt-level 0.
    let root = root_manifest();
    for profile in ["dev", "test"] {
        let level = root
            .get("profile")
            .and_then(Value::as_table)
            .and_then(|p| p.get(profile))
            .and_then(Value::as_table)
            .and_then(|p| p.get("package"))
            .and_then(Value::as_table)
            .and_then(|p| p.get("*"))
            .and_then(Value::as_table)
            .and_then(|p| p.get("opt-level"))
            .and_then(Value::as_integer);
        assert_eq!(
            level,
            Some(3),
            "[profile.{profile}.package.\"*\"] opt-level"
        );
    }
}

/// `major.minor` of a version string such as `1.99.1` or `1.99`.
fn major_minor(version: &str) -> String {
    version.split('.').take(2).collect::<Vec<_>>().join(".")
}

/// First run of ASCII digits in `s` (`">=26.13"` gives `26`).
fn first_number(s: &str) -> String {
    s.chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(char::is_ascii_digit)
        .collect()
}

/// Every `with:` map of every step of every job in a workflow document.
fn step_withs(doc: &serde_yaml_ng::Value) -> Vec<(String, serde_yaml_ng::Mapping)> {
    let mut out = Vec::new();
    let Some(jobs) = doc.get("jobs").and_then(|j| j.as_mapping()) else {
        return out;
    };
    for (_, job) in jobs {
        let Some(steps) = job.get("steps").and_then(|s| s.as_sequence()) else {
            continue;
        };
        for step in steps {
            let uses = step
                .get("uses")
                .and_then(|u| u.as_str())
                .unwrap_or("")
                .to_owned();
            if let Some(with) = step.get("with").and_then(|w| w.as_mapping()) {
                out.push((uses, with.clone()));
            }
        }
    }
    out
}

#[test]
fn toolchain_pins_agree() {
    let root = root_manifest();
    let rust_version = workspace_table(&root)
        .get("package")
        .and_then(Value::as_table)
        .and_then(|p| p.get("rust-version"))
        .and_then(Value::as_str)
        .expect("workspace.package.rust-version")
        .to_owned();
    let toolchain = read("rust-toolchain.toml")
        .parse::<Table>()
        .expect("rust-toolchain.toml parses");
    let channel = toolchain
        .get("toolchain")
        .and_then(Value::as_table)
        .and_then(|t| t.get("channel"))
        .and_then(Value::as_str)
        .expect("toolchain.channel")
        .to_owned();
    assert_eq!(
        major_minor(&channel),
        major_minor(&rust_version),
        "rust-toolchain.toml channel {channel} vs workspace rust-version {rust_version}"
    );

    // Every `toolchain:` input in CI is the pinned channel, exactly.
    let mut seen_workflow_toolchain = false;
    let mut pnpm_versions: Vec<String> = Vec::new();
    for file in workflow_files() {
        let doc: serde_yaml_ng::Value = serde_yaml_ng::from_str(&read(&file))
            .unwrap_or_else(|e| panic!("{file} does not parse: {e}"));
        for (uses, with) in step_withs(&doc) {
            if let Some(value) = with.get("toolchain") {
                let value = value.as_str().unwrap_or_default();
                assert_eq!(
                    value, channel,
                    "{file}: `toolchain:` must equal rust-toolchain.toml"
                );
                seen_workflow_toolchain = true;
            }
            if uses.starts_with("pnpm/action-setup@") {
                if let Some(v) = with.get("version") {
                    pnpm_versions.push(v.as_str().unwrap_or_default().to_owned());
                }
            }
        }
    }
    assert!(
        seen_workflow_toolchain,
        "no workflow sets a `toolchain:` (ci.yml must pin it)"
    );

    // Node and pnpm pins: checked once the files exist (stage 2a/2b), skipped before.
    let nvmrc = repo_root().join(".nvmrc");
    let package_json = repo_root().join("package.json");
    if nvmrc.is_file() && package_json.is_file() {
        let nvm = std::fs::read_to_string(&nvmrc).unwrap();
        let pkg: serde_json::Value =
            serde_json::from_str(&read("package.json")).expect("package.json parses");
        if let Some(node) = pkg.pointer("/engines/node").and_then(|v| v.as_str()) {
            assert_eq!(
                first_number(&nvm),
                first_number(node),
                ".nvmrc major vs package.json engines.node"
            );
        }
        if let Some(pm) = pkg.get("packageManager").and_then(|v| v.as_str()) {
            if let Some(version) = pm.strip_prefix("pnpm@") {
                let version = version.split('+').next().unwrap_or(version);
                // Until stage 2b no workflow touches pnpm (the `pnpm/action-setup` pin and the
                // Node steps arrive with the UI toolchain in 2b); the moment a workflow mentions
                // pnpm, it needs the pinned action with the matching version.
                let workflows_use_pnpm = workflow_files().iter().any(|f| read(f).contains("pnpm"));
                assert!(
                    !workflows_use_pnpm || !pnpm_versions.is_empty(),
                    "packageManager pnpm needs a CI action pin"
                );
                for declared in &pnpm_versions {
                    assert_eq!(
                        declared, version,
                        "pnpm/action-setup version vs packageManager"
                    );
                }
            }
        }
    }
}
