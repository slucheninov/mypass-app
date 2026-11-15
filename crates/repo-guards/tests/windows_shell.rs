//! Windows startup architecture guards. Test-only support code fails loudly on missing sources.
#![allow(clippy::unwrap_used)]
use repo_guards::{read, rel_str, repo_root, tracked_files};
#[test]
fn no_autostart_plugin_dependency_anywhere() {
    for path in tracked_files().unwrap() {
        let name = rel_str(&path);
        if name.ends_with("Cargo.toml") || name == "package.json" {
            assert!(!read(&name).contains("plugin-autostart"));
        }
    }
    for entry in std::fs::read_dir(repo_root().join("src-tauri/capabilities")).unwrap() {
        let source = std::fs::read_to_string(entry.unwrap().path()).unwrap();
        assert!(!source.contains("autostart:"));
    }
}
#[test]
fn no_relative_paths_in_shell_sources() {
    for entry in std::fs::read_dir(repo_root().join("src-tauri/src")).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "rs") {
            assert!(!std::fs::read_to_string(path)
                .unwrap()
                .contains("Path::new(\"./"));
        }
    }
}
#[test]
fn windows_boot_uses_safe_loader_and_platform_dialog_only() {
    let main = read("src-tauri/src/main.rs");
    assert!(
        main.find("windows_boot::preflight()").unwrap()
            < main.find("mypass_app_lib::run()").unwrap()
    );
    let source = read("src-tauri/src/windows_boot.rs");
    let boot = source.split("#[cfg(test)]").next().unwrap();
    assert!(boot.contains("tauri::webview_version().ok()"));
    assert!(boot.contains("mypass_platform::show_startup_error"));
    assert!(!boot.contains("Registry"));
    assert!(!boot.contains("unsafe"));
}
