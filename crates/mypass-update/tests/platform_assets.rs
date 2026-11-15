use mypass_update::{
    checksum_asset_for, current_release_platform, parse_release_payload, select_asset,
    BannerChange, FinishInput, ReleaseAsset, ReleasePlatform, SemanticVersion, UpdatePolicy,
    UpdateStatus,
};

fn assets(names: &[&str]) -> Vec<ReleaseAsset> {
    names
        .iter()
        .map(|name| ReleaseAsset {
            name: (*name).into(),
        })
        .collect()
}

#[test]
fn exact_platform_and_version_selection_and_checksums() {
    let version = SemanticVersion::new(1, 0, 0);
    let list = assets(&[
        "MyPass-1.0.0-macos-arm64.dmg",
        "SHA256SUMS.txt",
        "MyPass-1.0.0-windows-x64.zip",
        "MyPass-1.0.0-windows-x64.zip.sha256",
        "MyPass-chrome-extension-1.0.0.zip",
        "source.tar.gz",
    ]);
    for (platform, payload, checksum) in [
        (ReleasePlatform::MacosArm64, 0, 1),
        (ReleasePlatform::WindowsX64, 2, 3),
    ] {
        assert_eq!(
            select_asset(&list, platform, &version),
            Some(&list[payload])
        );
        assert_eq!(
            checksum_asset_for(&list[payload], &list),
            Some(&list[checksum])
        );
    }
    assert_eq!(checksum_asset_for(&list[4], &list), None);
    for platform in [ReleasePlatform::MacosArm64, ReleasePlatform::WindowsX64] {
        let stem = match platform {
            ReleasePlatform::MacosArm64 => "macos-arm64.dmg",
            ReleasePlatform::WindowsX64 => "windows-x64.zip",
        };
        for name in [
            format!("Evil-MyPass-1.0.0-{stem}"),
            format!("MyPass-1.0.0-{stem}.sig"),
            format!("MyPass-1.0.1-{stem}"),
            format!("mypass-1.0.0-{stem}"),
            "MyPass-1.0.0-windows-arm64.zip".into(),
        ] {
            assert_eq!(select_asset(&assets(&[&name]), platform, &version), None);
        }
    }
}

#[test]
fn feed_parses_names_only_and_missing_assets_fail_closed() {
    let info = parse_release_payload(serde_json::to_string(&serde_json::json!({
        "tag_name": "v1.0.0", "html_url": "https://github.com/slucheninov/mypass-app/releases/tag/v1.0.0",
        "assets": [{"name": "MyPass-1.0.0-windows-x64.zip", "browser_download_url": "https://evil.test/file"}]
    })).unwrap().as_bytes()).unwrap();
    assert_eq!(info.assets, assets(&["MyPass-1.0.0-windows-x64.zip"]));
    let empty = parse_release_payload(br#"{"tag_name":"v1.0.0","html_url":"https://github.com/slucheninov/mypass-app/releases/tag/v1.0.0"}"#).unwrap();
    assert!(empty.assets.is_empty());
}

#[test]
fn availability_requires_platform_payload_and_its_checksum_and_clears_stale_banner() {
    let mut release = parse_release_payload(br#"{"tag_name":"v1.0.0","html_url":"https://github.com/slucheninov/mypass-app/releases/tag/v1.0.0"}"#).unwrap();
    for platform in [
        Some(ReleasePlatform::MacosArm64),
        Some(ReleasePlatform::WindowsX64),
        None,
    ] {
        for names in [
            vec![],
            vec!["SHA256SUMS.txt"],
            vec!["MyPass-1.0.0-windows-x64.zip"],
            vec!["MyPass-1.0.0-macos-arm64.dmg"],
            vec![
                "MyPass-1.0.0-windows-x64.zip",
                "MyPass-1.0.1-windows-x64.zip.sha256",
            ],
            vec!["MyPass-1.0.0-macos-arm64.dmg", "SHA256SUMS.txt.sig"],
            vec!["MyPass-1.0.0-macos-arm64.dmg", "SHA256SUMS.txt"],
            vec![
                "MyPass-1.0.0-windows-x64.zip",
                "MyPass-1.0.0-windows-x64.zip.sha256",
            ],
        ] {
            release.assets = assets(&names);
            let expected = match platform {
                Some(ReleasePlatform::MacosArm64) => {
                    names.contains(&"MyPass-1.0.0-macos-arm64.dmg")
                        && names.contains(&"SHA256SUMS.txt")
                }
                Some(ReleasePlatform::WindowsX64) => {
                    names.contains(&"MyPass-1.0.0-windows-x64.zip")
                        && names.contains(&"MyPass-1.0.0-windows-x64.zip.sha256")
                }
                None => false,
            };
            let previous = UpdateStatus::Available(release.clone());
            let result = Ok(release.clone());
            let out = UpdatePolicy::finish(FinishInput {
                result: &result,
                current: SemanticVersion::new(0, 9, 0),
                platform,
                previous: &previous,
                manual: true,
                dismissed_version: None,
                now: 42,
            });
            if expected {
                assert_eq!(out.status, previous);
                assert_eq!(out.banner, BannerChange::Show(release.clone()));
            } else {
                assert_eq!(out.status, UpdateStatus::UpToDate { checked_at: 42 });
                assert_eq!(out.banner, BannerChange::Clear);
            }
        }
    }
}

#[test]
fn current_platform_uses_os_and_architecture() {
    let expected = if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        Some(ReleasePlatform::MacosArm64)
    } else if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        Some(ReleasePlatform::WindowsX64)
    } else {
        None
    };
    assert_eq!(current_release_platform(), expected);
}

#[test]
fn update_checks_never_download_binaries() {
    let github = include_str!("../src/github.rs")
        .split("#[cfg(test)]\nmod tests")
        .next()
        .unwrap();
    assert_eq!(github.matches(".get(").count(), 1);
    assert!(github.contains(".get(&self.endpoint)"));
    assert!(github.contains("/releases/latest"));
    assert!(!github.contains("browser_download_url"));
    for source in [
        include_str!("../src/asset.rs"),
        include_str!("../src/feed.rs"),
        include_str!("../src/policy.rs"),
    ] {
        let production = source.split("#[cfg(test)]\nmod tests").next().unwrap();
        assert!(!production.contains(".get("));
        assert!(!production.contains("browser_download_url"));
    }
}
