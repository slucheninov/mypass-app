use mypass_platform::{dev_overrides::windows_data_root, AppPaths, Browser};
use std::ffi::OsStr;
use std::path::Path;

#[test]
fn windows_data_root_override_honoured_in_debug_only() {
    let root = tempfile::tempdir().unwrap();
    assert_eq!(
        windows_data_root(true, Some(root.path().as_os_str())).unwrap(),
        Some(root.path().to_path_buf())
    );
    assert_eq!(
        windows_data_root(false, Some(root.path().as_os_str())).unwrap(),
        None
    );
    for invalid in ["", "relative", "a\0b"] {
        assert!(windows_data_root(true, Some(OsStr::new(invalid))).is_err());
    }
}

#[test]
fn windows_app_paths_match_spec_locations() {
    let home = Path::new("C:\\Users\\\u{412}\u{44f}\u{447}\u{435}\u{441}\u{43b}\u{430}\u{432}");
    let local = home.join("AppData/Local");
    let roaming = home.join("AppData/Roaming");
    let p = AppPaths::windows(&roaming, &local, home);
    assert_eq!(p.data_dir, local.join("MyPass"));
    assert_eq!(p.config_dir, roaming.join("MyPass"));
    assert_eq!(p.host_dir, p.data_dir.join("host"));
    assert_eq!(p.host_exe, p.host_dir.join("mypass-host.exe"));
    assert_eq!(
        p.manifest_path(Browser::Chrome),
        p.host_dir.join("com.mypass.chromehost.json")
    );
    assert_eq!(p.secrets_shared, p.data_dir.join("secrets/shared"));
    assert_eq!(p.secrets_app, p.data_dir.join("secrets/app"));
    assert_eq!(p.backups_dir, p.data_dir.join("backups"));
    assert_eq!(p.home(), home);
    let isolated = AppPaths::windows_data_root(&local);
    assert_eq!(isolated.config_dir, local);
    assert_eq!(isolated.data_dir, local);
}

#[test]
fn release_build_ignores_windows_data_root_env() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    for entry in std::fs::read_dir(root).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|s| s.to_str()) != Some("rs") {
            continue;
        }
        let source = std::fs::read_to_string(&path).unwrap();
        if source.contains("var_os(\"MYPASS_WINDOWS_DATA_ROOT\")") {
            assert_eq!(path.file_name().unwrap(), "dev_overrides.rs");
            assert!(source.contains("if cfg!(debug_assertions)"));
        }
    }
}
