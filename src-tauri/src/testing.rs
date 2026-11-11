//! Test-only support code: an `AppCore` over a temporary home and the fakes, never a real
//! clipboard, Keychain, notification or dialog.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use mypass_platform::testing::fake_platform;
use mypass_session::dto::EntryTypeDto;
use mypass_session::editing::new_model;
use mypass_session::scheduler::ManualScheduler;
use mypass_session::settings::SettingsStore;
use mypass_session::testing::fast_database;
use mypass_session::{AppCore, AppPaths};
use secrecy::SecretString;
use std::path::PathBuf;
use std::time::{Duration, UNIX_EPOCH};
use tempfile::TempDir;
use uuid::Uuid;

pub struct World {
    pub dir: TempDir,
    pub core: AppCore,
    pub path: PathBuf,
}

pub fn pw(text: &str) -> SecretString {
    SecretString::from(text.to_owned())
}

/// A core with no vault open.
pub fn world() -> World {
    let dir = tempfile::tempdir().unwrap();
    let fake = fake_platform();
    let clock = ManualScheduler::new(UNIX_EPOCH + Duration::from_secs(1_700_000_000));
    let core = AppCore::with_parts(
        AppPaths::macos(dir.path()),
        fake.platform.clone(),
        clock,
        SettingsStore::in_memory(),
    );
    let path = dir.path().join("v.kdbx");
    World { dir, core, path }
}

/// A core with an unlocked, empty vault (password `pw`, cheap KDF).
pub fn open_world() -> World {
    let w = world();
    w.core
        .session
        .create_vault_with(&w.path, pw("pw"), fast_database("Test"))
        .unwrap();
    w
}

impl World {
    pub fn root(&self) -> String {
        self.core.sidebar_get().unwrap().root_group_id
    }

    /// Saves a login and returns its id.
    pub fn login(&self, title: &str) -> String {
        let mut model = new_model(EntryTypeDto::Login, Uuid::parse_str(&self.root()).unwrap());
        model.title = title.into();
        model.username = "user".into();
        model.password = "secret".into();
        self.core.entry_save(model).unwrap()
    }

    /// Writes a file below the temp dir and returns its path.
    pub fn file(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.dir.path().join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }
}
