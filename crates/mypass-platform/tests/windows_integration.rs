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

#[cfg(windows)]
mod dpapi {
    use mypass_platform::{AppPaths, DpapiSecretStore, SecretClass, SecretStore};
    fn opt_in() {
        assert_eq!(
            std::env::var("MYPASS_WINDOWS_INTEGRATION_TESTS").as_deref(),
            Ok("1")
        );
    }
    #[test]
    #[ignore = "real DPAPI; use isolated native runner"]
    fn windows_integration_dpapi_round_trip_entropy_and_plaintext_absence() {
        opt_in();
        let tmp = tempfile::tempdir().unwrap();
        let paths = AppPaths::windows_data_root(
            &tmp.path()
                .join("\u{441}\u{435}\u{43a}\u{440}\u{435}\u{442}\u{438} \u{65e5}\u{672c}\u{8a9e}"),
        );
        let platform = mypass_platform::current(&paths);
        let shared = platform.shared_secrets;
        let app = platform.secrets;
        for side in [
            mypass_platform::StoreSide::App,
            mypass_platform::StoreSide::Host,
        ] {
            assert_eq!(
                mypass_platform::endpoint_store::running_store_kind(side),
                mypass_platform::SharedStoreKind::Dpapi
            );
        }
        assert_eq!(
            mypass_platform::app_shared_store(&paths, None).0,
            mypass_platform::SharedStoreKind::Dpapi
        );
        let (kind, host) = mypass_platform::host_shared_store(&paths, None);
        assert_eq!(kind, mypass_platform::SharedStoreKind::Dpapi);
        assert_ne!(paths.secrets_shared, paths.secrets_app);
        shared.set("empty", "item", b"").unwrap();
        assert_eq!(shared.get("empty", "item").unwrap(), Some(Vec::new()));
        shared.delete("empty", "item").unwrap();
        assert_eq!(shared.get("s", "a").unwrap(), None);
        shared.delete("s", "a").unwrap();
        shared.set("s", "a", b"hunter2-token").unwrap();
        assert_eq!(shared.get("s", "a").unwrap().unwrap(), b"hunter2-token");
        assert_eq!(app.get("s", "a").unwrap(), None);
        app.set("s", "a", b"app-only item").unwrap();
        assert_eq!(host.get("s", "a").unwrap().unwrap(), b"hunter2-token");
        assert_eq!(app.get("s", "a").unwrap().unwrap(), b"app-only item");
        let wrong = DpapiSecretStore::at(paths.secrets_shared.clone(), SecretClass::AppOnly);
        let error = wrong.get("s", "a").unwrap_err();
        assert!(!format!("{error:?} {error}").contains("hunter2-token"));
        let item = std::fs::read_dir(&paths.secrets_shared)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let bytes = std::fs::read(&item).unwrap();
        assert!(bytes.starts_with(b"MPS1"));
        assert!(!bytes.windows(13).any(|w| w == b"hunter2-token"));
        // A write of the exact legacy filename replaces it with encrypted bytes, no later delete.
        std::fs::write(&item, b"legacy plaintext endpoint").unwrap();
        assert!(shared.get("s", "a").is_err());
        shared.set("s", "a", b"replacement").unwrap();
        assert_eq!(shared.get("s", "a").unwrap().unwrap(), b"replacement");
        assert_eq!(std::fs::read_dir(&paths.secrets_shared).unwrap().count(), 1);
        shared.delete("s", "a").unwrap();
        shared.delete("s", "a").unwrap();
        assert_eq!(app.get("s", "a").unwrap().unwrap(), b"app-only item");
    }
    #[test]
    #[ignore = "real DPAPI; use isolated native runner"]
    fn windows_integration_dpapi_concurrent_atomic_publication() {
        opt_in();
        let tmp = tempfile::tempdir().unwrap();
        let paths = AppPaths::windows_data_root(tmp.path());
        let store = std::sync::Arc::new(DpapiSecretStore::shared(&paths));
        store.set("s", "a", b"initial").unwrap();
        let threads: Vec<_> = (0..8)
            .map(|n| {
                let store = store.clone();
                std::thread::spawn(move || {
                    for i in 0..50 {
                        store
                            .set("s", "a", format!("secret-{n}-{i}").as_bytes())
                            .unwrap();
                        assert!(store.get("s", "a").unwrap().is_some());
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        assert_eq!(std::fs::read_dir(&paths.secrets_shared).unwrap().count(), 1);
    }
    #[test]
    #[ignore = "real DPAPI and ACLs; use isolated native runner"]
    fn windows_integration_dpapi_rejects_weakened_dacl_ads_and_reparse_ancestors() {
        opt_in();
        let tmp = tempfile::tempdir().unwrap();
        let paths = AppPaths::windows_data_root(tmp.path());
        let store = DpapiSecretStore::shared(&paths);
        store.set("s", "a", b"secret").unwrap();
        // SID spelling is locale independent. The production inspection permits only the current
        // token user and SYSTEM, and requires both full grants with no inherited parent access.
        let status = std::process::Command::new("icacls")
            .arg(&paths.secrets_shared)
            .args(["/grant", "*S-1-1-0:(RX)"])
            .output()
            .unwrap();
        assert!(status.status.success());
        assert!(store.get("s", "a").is_err());
        assert!(store.set("s", "a", b"other").is_err());
        let ads = DpapiSecretStore::at(tmp.path().join("bad:stream"), SecretClass::Shared);
        assert!(ads.set("s", "a", b"secret").is_err());
        let target = tmp.path().join("target");
        std::fs::create_dir(&target).unwrap();
        let junction = tmp.path().join("junction");
        let status = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&junction)
            .arg(&target)
            .output()
            .unwrap();
        assert!(status.status.success());
        let redirected = DpapiSecretStore::at(junction.join("shared"), SecretClass::Shared);
        assert!(redirected.set("s", "a", b"secret").is_err());
        assert!(std::fs::read_dir(&target).unwrap().next().is_none());
        std::fs::remove_dir(&junction).unwrap();
    }
    #[test]
    #[ignore = "real IP Helper and process token APIs; use isolated native runner"]
    fn windows_integration_dpapi_listener_requires_exact_live_pid() {
        opt_in();
        let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        use mypass_platform::peer_process::endpoint_peer_is_trusted;
        use mypass_platform::SharedStoreKind::Dpapi;
        assert!(endpoint_peer_is_trusted(Dpapi, port, std::process::id()));
        assert!(!endpoint_peer_is_trusted(Dpapi, port, 0));
        assert!(!endpoint_peer_is_trusted(Dpapi, port, u32::MAX));
        drop(listener);
        assert!(!endpoint_peer_is_trusted(Dpapi, port, std::process::id()));
    }
}

#[cfg(windows)]
#[test]
#[ignore = "real DPAPI and Windows replacement; use isolated native runner"]
fn windows_integration_dpapi_failed_publication_preserves_old_record_and_cleans_temp() {
    use mypass_platform::{AppPaths, DpapiSecretStore, SecretStore};
    use std::os::windows::fs::OpenOptionsExt;
    assert_eq!(
        std::env::var("MYPASS_WINDOWS_INTEGRATION_TESTS").as_deref(),
        Ok("1")
    );
    let tmp = tempfile::tempdir().unwrap();
    let paths = AppPaths::windows_data_root(tmp.path());
    let store = DpapiSecretStore::shared(&paths);
    store.set("s", "a", b"old record").unwrap();
    let file = std::fs::read_dir(&paths.secrets_shared)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    // This live handle deliberately denies delete sharing so MoveFileExW must fail.
    let locked = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(3)
        .open(&file)
        .unwrap();
    assert!(store.set("s", "a", b"new record").is_err());
    drop(locked);
    assert_eq!(std::fs::read_dir(&paths.secrets_shared).unwrap().count(), 1);
    assert_eq!(store.get("s", "a").unwrap().unwrap(), b"old record");
    // Corrupted protected blobs fail closed, without echoing data or treating corruption as None.
    let mut bytes = std::fs::read(&file).unwrap();
    *bytes.last_mut().unwrap() ^= 1;
    std::fs::write(&file, bytes).unwrap();
    assert!(store.get("s", "a").is_err());
}
