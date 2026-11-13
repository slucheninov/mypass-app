//! The sidecar placeholder rules of `build_support.rs` (shared with `build.rs` through `#[path]`).
#![allow(clippy::unwrap_used, clippy::expect_used)] // test-only support code

#[path = "../build_support.rs"]
mod build_support;

use build_support::ensure_sidecar_placeholder;

#[test]
fn ensure_sidecar_placeholder_creates_missing_file_with_triple_and_exe_suffix() {
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("binaries");
    assert!(ensure_sidecar_placeholder(&bin, "aarch64-apple-darwin").unwrap());
    let mac = bin.join("mypass-native-host-aarch64-apple-darwin");
    assert!(mac.is_file());
    assert_eq!(std::fs::metadata(&mac).unwrap().len(), 0);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&mac).unwrap().permissions().mode() & 0o777,
            0o755
        );
    }
    assert!(ensure_sidecar_placeholder(&bin, "x86_64-pc-windows-msvc").unwrap());
    assert!(bin
        .join("mypass-native-host-x86_64-pc-windows-msvc.exe")
        .is_file());
}

#[test]
fn ensure_sidecar_placeholder_never_overwrites_an_existing_binary() {
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().to_path_buf();
    let file = bin.join("mypass-native-host-aarch64-apple-darwin");
    std::fs::write(&file, b"real host bytes").unwrap();
    assert!(!ensure_sidecar_placeholder(&bin, "aarch64-apple-darwin").unwrap());
    assert_eq!(std::fs::read(&file).unwrap(), b"real host bytes");
}

#[test]
fn binaries_dir_is_gitignored_and_declared_as_external_bin() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let ignore = std::fs::read_to_string(root.join("../.gitignore")).unwrap();
    assert!(ignore
        .lines()
        .any(|l| l.trim_end_matches('/') == "/src-tauri/binaries"));
    let conf: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(root.join("tauri.conf.json")).unwrap())
            .unwrap();
    assert_eq!(
        conf["bundle"]["externalBin"],
        serde_json::json!(["binaries/mypass-native-host"])
    );
}

/// One name for the bundled host, everywhere: the sidecar in `tauri.conf.json`, the staging script,
/// the placeholder and the registrar's lookup; never cargo's `mypass-host`.
#[test]
fn the_bundled_host_name_is_the_same_everywhere_and_not_cargos() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let script = std::fs::read_to_string(root.join("../scripts/build-host.sh")).unwrap();
    assert!(script.contains("src-tauri/binaries/mypass-native-host-<target triple>"));
    assert!(script.contains("mypass-native-host-$TRIPLE"));
    assert_eq!(
        mypass_platform::BUNDLED_HOST_FILE.trim_end_matches(".exe"),
        "mypass-native-host"
    );
    assert_ne!(mypass_platform::BUNDLED_HOST_FILE, "mypass-host");
}
