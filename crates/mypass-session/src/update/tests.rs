use super::*;
use crate::scheduler::ManualScheduler;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
struct Feed(AtomicUsize);
impl ReleaseFeed for Feed {
    fn latest(&self) -> Result<ReleaseInfo, mypass_update::ReleaseFeedError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(ReleaseInfo {
            assets: [
                "MyPass-2.0.0-macos-arm64.dmg",
                "SHA256SUMS.txt",
                "MyPass-2.0.0-windows-x64.zip",
                "MyPass-2.0.0-windows-x64.zip.sha256",
            ]
            .into_iter()
            .map(|name| mypass_update::ReleaseAsset { name: name.into() })
            .collect(),
            version: SemanticVersion::new(2, 0, 0),
            tag: "v2.0.0".into(),
            url: "https://github.com/slucheninov/mypass-app/releases/tag/v2.0.0".into(),
        })
    }
}
#[test]
fn schedule_and_restart_have_one_timer_and_shutdown_stops_it() {
    let dir = tempfile::tempdir().unwrap();
    let clock = ManualScheduler::new(UNIX_EPOCH);
    let feed = Arc::new(Feed(AtomicUsize::new(0)));
    let service = UpdateService::new_with_platform(
        clock.clone(),
        SettingsStore::in_memory(),
        feed.clone(),
        "1.0.0",
        dir.path(),
        Some(ReleasePlatform::MacosArm64),
    );
    let (tx, rx) = std::sync::mpsc::channel();
    service.add_observer(Arc::new(move |s| {
        if s.status != UpdateState::Checking {
            tx.send(()).unwrap();
        }
    }));
    service.start_scheduling();
    service.start_scheduling();
    clock.advance(Duration::from_secs(9));
    assert_eq!(feed.0.load(Ordering::SeqCst), 0);
    clock.advance(Duration::from_secs(1));
    rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(feed.0.load(Ordering::SeqCst), 1);
    service.stop_scheduling();
    service.start_scheduling();
    clock.advance(Duration::from_secs(10));
    rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(feed.0.load(Ordering::SeqCst), 2);
    clock.advance(Duration::from_secs(86400));
    rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(feed.0.load(Ordering::SeqCst), 3);
    service.shutdown();
    clock.advance(Duration::from_secs(86400));
    assert_eq!(feed.0.load(Ordering::SeqCst), 3);
}
#[test]
fn dismiss_persists_and_disable_clears_banner() {
    let dir = tempfile::tempdir().unwrap();
    let settings = SettingsStore::in_memory();
    let service = UpdateService::new_with_platform(
        ManualScheduler::new(UNIX_EPOCH),
        settings.clone(),
        Arc::new(Feed(AtomicUsize::new(0))),
        "1.0.0",
        dir.path(),
        Some(ReleasePlatform::MacosArm64),
    );
    service.check(true);
    assert!(service.status().banner.is_some());
    service.dismiss().unwrap();
    service.check(true);
    assert!(service.status().banner.is_none());
    assert!(service.release_url().is_some());
    settings
        .update(crate::settings::SettingsPatch {
            update_check_enabled: Some(false),
            ..Default::default()
        })
        .unwrap();
    service.enabled_changed();
    assert_eq!(service.status().status, UpdateState::Idle);
}
#[test]
fn invalid_running_version_and_noop_do_not_use_network() {
    let dir = tempfile::tempdir().unwrap();
    let service = UpdateService::new_with_platform(
        ManualScheduler::new(UNIX_EPOCH),
        SettingsStore::in_memory(),
        Arc::new(mypass_update::NoopFeed),
        "bad",
        dir.path(),
        Some(ReleasePlatform::MacosArm64),
    );
    service.check(true);
    assert_eq!(
        service.status().status,
        UpdateState::Failed {
            kind: UpdateFailure::UnknownVersion
        }
    );
}

struct HeldFeed {
    entered: std::sync::mpsc::Sender<()>,
    release: Mutex<std::sync::mpsc::Receiver<()>>,
    calls: AtomicUsize,
}
impl ReleaseFeed for HeldFeed {
    fn latest(&self) -> Result<ReleaseInfo, mypass_update::ReleaseFeedError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.entered.send(()).unwrap();
        self.release.lock().unwrap().recv().unwrap();
        Err(mypass_update::ReleaseFeedError::Offline)
    }
}
#[test]
fn manual_upgrade_does_not_start_another_feed_and_shutdown_discards_results() {
    let dir = tempfile::tempdir().unwrap();
    let (entered, accepted) = std::sync::mpsc::channel();
    let (release, wait) = std::sync::mpsc::channel();
    let feed = Arc::new(HeldFeed {
        entered,
        release: Mutex::new(wait),
        calls: AtomicUsize::new(0),
    });
    let service = UpdateService::new_with_platform(
        ManualScheduler::new(UNIX_EPOCH),
        SettingsStore::in_memory(),
        feed.clone(),
        "1.0.0",
        dir.path(),
        Some(ReleasePlatform::MacosArm64),
    );
    let worker_service = service.clone();
    let worker = std::thread::spawn(move || worker_service.check(false));
    accepted.recv().unwrap();
    assert_eq!(service.check(true).status, UpdateState::Checking);
    assert_eq!(feed.calls.load(Ordering::SeqCst), 1);
    release.send(()).unwrap();
    worker.join().unwrap();
    assert_eq!(
        service.status().status,
        UpdateState::Failed {
            kind: UpdateFailure::Offline
        }
    );
    assert_eq!(UpdateStateStore::new(dir.path()).load().last_run, Some(0));
    let worker_service = service.clone();
    let worker = std::thread::spawn(move || worker_service.check(true));
    accepted.recv().unwrap();
    service.shutdown();
    release.send(()).unwrap();
    worker.join().unwrap();
    assert_eq!(service.status().status, UpdateState::Checking);
    assert_eq!(feed.calls.load(Ordering::SeqCst), 2);
    service.check(true);
    assert_eq!(feed.calls.load(Ordering::SeqCst), 2);
}

#[test]
fn a_dispatched_old_generation_cannot_run_after_stop_and_restart() {
    let dir = tempfile::tempdir().unwrap();
    let feed = Arc::new(Feed(AtomicUsize::new(0)));
    let service = UpdateService::new_with_platform(
        ManualScheduler::new(UNIX_EPOCH),
        SettingsStore::in_memory(),
        feed.clone(),
        "1.0.0",
        dir.path(),
        Some(ReleasePlatform::MacosArm64),
    );
    service.start_scheduling();
    let generation = lock(&service.0.state).generation;
    service.stop_scheduling();
    service.start_scheduling();
    service.check_generation(false, Some(generation));
    assert_eq!(feed.0.load(Ordering::SeqCst), 0);
}

struct MutableFeed(Mutex<Result<ReleaseInfo, mypass_update::ReleaseFeedError>>);
impl ReleaseFeed for MutableFeed {
    fn latest(&self) -> Result<ReleaseInfo, mypass_update::ReleaseFeedError> {
        self.0.lock().unwrap().clone()
    }
}

#[test]
fn platform_banner_requires_complete_assets_and_keeps_release_page_action() {
    for platform in [
        Some(ReleasePlatform::MacosArm64),
        Some(ReleasePlatform::WindowsX64),
        None,
    ] {
        let dir = tempfile::tempdir().unwrap();
        let release = Feed(AtomicUsize::new(0)).latest().unwrap();
        let feed = Arc::new(MutableFeed(Mutex::new(Ok(release.clone()))));
        let service = UpdateService::new_with_platform(
            ManualScheduler::new(UNIX_EPOCH),
            SettingsStore::in_memory(),
            feed.clone(),
            "1.0.0",
            dir.path(),
            platform,
        );
        let status = service.check(true);
        if platform.is_none() {
            assert_eq!(status.status, UpdateState::UpToDate { checked_at: 0.0 });
            assert!(status.banner.is_none());
            assert!(service.release_url().is_none());
            continue;
        }
        assert!(matches!(status.status, UpdateState::Available { .. }));
        assert_eq!(service.release_url(), Some(release.url.clone()));
        *feed.0.lock().unwrap() = Err(mypass_update::ReleaseFeedError::Offline);
        service.check(true);
        assert_eq!(service.status().banner, status.banner);
        assert_eq!(service.release_url(), Some(release.url.clone()));

        // Each incomplete successful answer must remove even a previously displayed banner.
        for names in [
            vec![],
            vec!["MyPass-2.0.0-macos-arm64.dmg"],
            vec!["MyPass-2.0.0-windows-x64.zip"],
            vec![
                "MyPass-2.0.1-windows-x64.zip",
                "MyPass-2.0.1-windows-x64.zip.sha256",
            ],
            match platform.unwrap() {
                ReleasePlatform::MacosArm64 => vec![
                    "MyPass-2.0.0-windows-x64.zip",
                    "MyPass-2.0.0-windows-x64.zip.sha256",
                ],
                ReleasePlatform::WindowsX64 => {
                    vec!["MyPass-2.0.0-macos-arm64.dmg", "SHA256SUMS.txt"]
                }
            },
        ] {
            *feed.0.lock().unwrap() = Ok(release.clone());
            assert!(service.check(true).banner.is_some());
            let mut incomplete = release.clone();
            incomplete.assets = names
                .into_iter()
                .map(|name| mypass_update::ReleaseAsset { name: name.into() })
                .collect();
            *feed.0.lock().unwrap() = Ok(incomplete);
            let out = service.check(true);
            assert_eq!(out.status, UpdateState::UpToDate { checked_at: 0.0 });
            assert!(out.banner.is_none());
            assert!(service.release_url().is_none());
        }
    }
}
