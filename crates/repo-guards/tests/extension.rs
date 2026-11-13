//! Guards for the vendored Chromium extension under `extension/`: its suites are discovered, the
//! manifest key pins the extension id, the host name is the one the registrar writes, and the copied
//! tests do not depend on the old repository layout.
//!
//! test-only support code: helpers here unwrap and index freely, a guard must fail loudly.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use base64::Engine as _;
use repo_guards::{bash_command, read, repo_root};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

const EXTENSION_ID: &str = "hjijbabodfldconmhdcdphlbgaajkejn";

fn suites() -> Vec<String> {
    let mut found: Vec<String> = std::fs::read_dir(repo_root().join("extension/tests"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".test.js"))
        .map(|n| format!("extension/tests/{n}"))
        .collect();
    found.sort();
    found
}

#[test]
fn extension_suites_are_discovered_and_manifest_pins_the_extension_id() {
    // 41 is the pin of the import from the old repository; a suite added later raises it here.
    assert_eq!(suites().len(), 41);
    let manifest: serde_json::Value =
        serde_json::from_str(&read("extension/manifest.json")).unwrap();
    let key = manifest["key"].as_str().expect("manifest.json needs a key");
    let der = base64::engine::general_purpose::STANDARD
        .decode(key)
        .unwrap();
    let digest = Sha256::digest(&der);
    let id: String = digest[..16]
        .iter()
        .flat_map(|b| [b >> 4, b & 0x0f])
        .map(|n| char::from(b'a' + n))
        .collect();
    assert_eq!(id, EXTENSION_ID);
}

#[test]
fn background_names_the_native_host() {
    assert!(read("extension/background.js").contains("com.mypass.chromehost"));
}

#[test]
fn copied_suites_reference_no_old_repo_paths() {
    // `MyPass/` alone would hit prose such as `/Unlock MyPass/` regexes, so only path shapes count.
    const FORBIDDEN: [&str; 6] = [
        "ChromeExtension/",
        "../../../",
        "Packages/",
        "MyPass/UI/",
        ".swift",
        "/Users/",
    ];
    let mut files: Vec<PathBuf> = Vec::new();
    let mut dirs = vec![repo_root().join("extension/tests")];
    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                dirs.push(path);
            } else {
                files.push(path);
            }
        }
    }
    assert!(!files.is_empty());
    for file in files {
        let Ok(text) = std::fs::read_to_string(&file) else {
            continue;
        };
        for needle in FORBIDDEN {
            assert!(
                !text.contains(needle),
                "{} contains the old-repo path `{needle}`",
                file.display()
            );
        }
    }
}

#[test]
fn extension_list_count_matches_glob() {
    if std::process::Command::new("node")
        .arg("--version")
        .output()
        .is_err()
    {
        println!("skipped: node is not on PATH");
        return;
    }
    let out = match bash_command()
        .args(["scripts/test-extension.sh", "--list"])
        .current_dir(repo_root())
        .output()
    {
        Ok(out) => out,
        Err(e) => {
            println!("skipped: cannot run bash: {e}");
            return;
        }
    };
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let listed: Vec<String> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|l| l.trim_end_matches('\r').to_owned())
        .collect();
    assert_eq!(listed, suites());
}

#[test]
fn test_sh_js_part_invokes_test_extension_sh() {
    let script = read("scripts/test.sh");
    assert!(script.contains("scripts/test-extension.sh"));
    assert!(script.contains("--no-js"));
}
