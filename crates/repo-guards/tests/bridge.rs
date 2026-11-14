//! Guards for the bridge and host crates.
//!
//! test-only support code: helpers here unwrap and index freely, a guard must fail loudly.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use repo_guards::{read, rel_str, repo_root, tracked_files};
use toml::{Table, Value};

fn manifest(path: &str) -> Table {
    read(path).parse::<Table>().unwrap()
}

fn dep<'a>(manifest: &'a Table, name: &str) -> &'a Table {
    manifest["dependencies"][name]
        .as_table()
        .unwrap_or_else(|| panic!("{name} is a table dependency"))
}

/// The host must never link the session, the core or the KDBX engine: it asks the app instead.
#[test]
fn host_links_no_vault_code() {
    let host = manifest("crates/mypass-host/Cargo.toml");
    let bridge_dep = dep(&host, "mypass-bridge");
    assert_eq!(
        bridge_dep.get("default-features"),
        Some(&Value::Boolean(false)),
        "mypass-host must depend on mypass-bridge with default-features = false"
    );
    assert!(
        !bridge_dep.contains_key("features"),
        "mypass-host must not enable bridge features"
    );
    for forbidden in ["mypass-session", "mypass-core", "mypass-kdbx"] {
        assert!(
            !host["dependencies"]
                .as_table()
                .unwrap()
                .contains_key(forbidden),
            "mypass-host depends on {forbidden}"
        );
    }

    let bridge = manifest("crates/mypass-bridge/Cargo.toml");
    let features = bridge["features"].as_table().unwrap();
    let default: Vec<&str> = features["default"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(default, ["server"]);
    // Everything that touches the vault is optional and enabled only by `server`.
    let server: Vec<&str> = features["server"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    for forbidden in ["mypass-session", "mypass-core", "mypass-kdbx"] {
        assert_eq!(
            dep(&bridge, forbidden).get("optional"),
            Some(&Value::Boolean(true)),
            "{forbidden} must be optional in mypass-bridge"
        );
        assert!(
            server.contains(&format!("dep:{forbidden}").as_str()),
            "the server feature must enable {forbidden}"
        );
    }
    // The platform crate is needed by the ungated endpoint and relay code.
    assert!(!dep(&bridge, "mypass-platform").contains_key("optional"));
}

/// Stage 4 retired the stage 3 passkey placeholder: the app, the tests and the host e2e all run
/// the real `PasskeyService`, so no tracked file may name the old type again.
#[test]
fn not_supported_passkeys_no_longer_exists() {
    let Some(files) = tracked_files() else {
        println!("skipped: not a git checkout");
        return;
    };
    // Spelled in two halves so this guard does not match itself.
    let needle = format!("{}{}", "NotSupported", "Passkeys");
    let root = repo_root();
    let mut offenders = Vec::new();
    let mut scanned = 0;
    for path in files {
        // Deleted from the work tree but still in the index, or not text: nothing to scan.
        let Ok(text) = std::fs::read_to_string(root.join(&path)) else {
            continue;
        };
        scanned += 1;
        if text.contains(&needle) {
            offenders.push(rel_str(&path));
        }
    }
    assert!(scanned > 100, "the scan saw only {scanned} files");
    assert!(
        offenders.is_empty(),
        "{needle} is still named in {offenders:?}"
    );
}
