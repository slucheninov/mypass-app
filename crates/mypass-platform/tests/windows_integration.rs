//! Real profile APIs are opt-in even when an integration environment flag leaks into normal tests.
#[cfg(windows)]
#[test]
#[ignore = "real Windows profile API; use the isolated native runner"]
fn windows_integration_known_folders_resolve_to_existing_absolute_dirs() {
    assert_eq!(
        std::env::var("MYPASS_WINDOWS_INTEGRATION_TESTS").as_deref(),
        Ok("1")
    );
    let (local, roaming, profile) = mypass_platform::windows::known_folders().unwrap();
    for path in [local, roaming, profile] {
        assert!(path.is_absolute());
        assert!(path.is_dir());
    }
}
