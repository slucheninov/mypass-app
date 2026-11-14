//! The vault session, settings, scheduling and the command facade behind the Tauri shell.
//!
//! This crate has no Tauri dependency, so it is testable without a window server. Every timing
//! rule runs through [`scheduler::Scheduler`]; tests drive it with [`scheduler::ManualScheduler`].

pub mod app_core;
pub mod autolock;
pub mod backups;
pub mod clipboard_service;
pub mod csv_import;
pub mod drafts;
pub mod dto;
pub mod editing;
pub mod fields;
pub mod generator_service;
pub mod queries;
pub mod saving;
pub mod scheduler;
pub mod session;
pub mod settings;
pub mod shortcut;
pub mod unlock_request;
pub mod vault_files;

pub use app_core::{ApiError, AppCore, AppPaths, ErrorCode, Platform};
pub use backups::{BackupFailure, BackupItem, BackupStatus};
pub use drafts::DraftVersionDto;
pub use dto::*;
pub use session::{LockReason, SaveState, SaveStatus, SessionState, SessionStatus};
pub use settings::{
    BackupInterval, GeneratorOptionsDto, IdleTimeout, Language, RequireAccountPasswordWindow,
    Settings, SettingsPatch, Theme, UnlockWithDeviceWindow,
};
pub use vault_files::VaultFileStatus;

/// Test support shared by this crate's tests and by later crates' tests. Not part of the app.
#[doc(hidden)]
pub mod testing {
    use mypass_kdbx::{Argon2Variant, Database, KdfParams};

    /// The shared `has_passkey` contract table: each case is a list of `(field key, value)`
    /// pairs on an otherwise empty entry and whether the entry then carries a usable passkey. The
    /// bridge and the passkey stage reuse it so the definition lives in one place.
    pub fn has_passkey_cases() -> Vec<(Vec<(&'static str, &'static str)>, bool)> {
        use crate::fields::{PASSKEY_CREDENTIAL_ID, PASSKEY_PRIVATE_KEY_PEM};
        vec![
            (
                vec![
                    (PASSKEY_CREDENTIAL_ID, "abc"),
                    (PASSKEY_PRIVATE_KEY_PEM, "pem"),
                ],
                true,
            ),
            (vec![(PASSKEY_CREDENTIAL_ID, "abc")], false),
            (vec![(PASSKEY_PRIVATE_KEY_PEM, "pem")], false),
            (
                vec![(PASSKEY_CREDENTIAL_ID, ""), (PASSKEY_PRIVATE_KEY_PEM, "")],
                false,
            ),
            // A credential id that is not base64 at all.
            (
                vec![
                    (PASSKEY_CREDENTIAL_ID, "!!"),
                    (PASSKEY_PRIVATE_KEY_PEM, "pem"),
                ],
                false,
            ),
            // A credential id that decodes to zero bytes, next to a usable PEM.
            (
                vec![
                    (PASSKEY_CREDENTIAL_ID, ""),
                    (PASSKEY_PRIVATE_KEY_PEM, "pem"),
                ],
                false,
            ),
            (vec![], false),
        ]
    }

    /// The private key PEM value [`add_test_passkey`] stores. Not a key and deliberately not PEM
    /// armoured (nothing here parses it), so secret scanners have nothing to flag.
    #[cfg(any(test, feature = "test-util"))]
    pub const TEST_PASSKEY_PEM: &str = "test-passkey-private-key-TestPasskeyBodyXyz";
    /// The base64url credential id [`add_test_passkey`] stores (bytes 1 to 8).
    #[cfg(any(test, feature = "test-util"))]
    pub const TEST_PASSKEY_CREDENTIAL_ID: &str = "AQIDBAUGBwg";
    /// The base64url user handle [`add_test_passkey`] stores (`user-handle-1`).
    #[cfg(any(test, feature = "test-util"))]
    pub const TEST_PASSKEY_USER_HANDLE: &str = "dXNlci1oYW5kbGUtMQ";
    /// The `KPEX_PASSKEY_CREATED` value [`add_test_passkey`] stores when asked to.
    #[cfg(any(test, feature = "test-util"))]
    pub const TEST_PASSKEY_CREATED: &str = "2023-11-14T22:13:20Z";

    /// Stores a passkey on `e` the way KeePassXC lays it out (credential id, PEM and user handle
    /// protected, the rest plain), relying party `example.com`, account `alice@example.com`.
    /// With `created` it is the eight fields MyPass writes; without, a KeePassXC passkey (seven).
    #[cfg(any(test, feature = "test-util"))]
    pub fn add_test_passkey(e: &mut mypass_kdbx::Entry, created: bool) {
        use crate::fields::{
            PASSKEY_CREATED, PASSKEY_CREDENTIAL_ID, PASSKEY_FLAG_BE, PASSKEY_FLAG_BS,
            PASSKEY_PRIVATE_KEY_PEM, PASSKEY_RELYING_PARTY, PASSKEY_USERNAME, PASSKEY_USER_HANDLE,
        };
        e.set_field(PASSKEY_RELYING_PARTY, "example.com", false);
        e.set_field(PASSKEY_CREDENTIAL_ID, TEST_PASSKEY_CREDENTIAL_ID, true);
        e.set_field(PASSKEY_PRIVATE_KEY_PEM, TEST_PASSKEY_PEM, true);
        e.set_field(PASSKEY_USER_HANDLE, TEST_PASSKEY_USER_HANDLE, true);
        e.set_field(PASSKEY_USERNAME, "alice@example.com", false);
        e.set_field(PASSKEY_FLAG_BE, "1", false);
        e.set_field(PASSKEY_FLAG_BS, "1", false);
        if created {
            e.set_field(PASSKEY_CREATED, TEST_PASSKEY_CREATED, false);
        }
    }

    /// The Swift `PasskeyCategoryTests` vault: returns the database and the ids of the entries the
    /// Passkeys category must list ("Passkey only" in the root, "Password and passkey" in a nested
    /// group). Not listed: a credential id without a PEM, an undecodable credential id, an empty
    /// PEM, an empty credential id, a passkey in the generator history group, one in the recycle
    /// bin and one in a group inside the bin.
    #[cfg(any(test, feature = "test-util"))]
    pub fn passkey_category_database() -> (Database, Vec<uuid::Uuid>) {
        use crate::fields::{PASSKEY_CREDENTIAL_ID, PASSKEY_PRIVATE_KEY_PEM};
        use mypass_core::entry_ext::GENERATOR_HISTORY_GROUP_UUID;
        use mypass_kdbx::{Entry, Group, Timestamp};
        let now = Timestamp(1_700_000_000);
        let entry = |title: &str, password: &str| {
            let mut e = Entry::new(now);
            e.set_field("Title", title, false);
            if !password.is_empty() {
                e.set_field("Password", password, true);
            }
            add_test_passkey(&mut e, false);
            e
        };
        let mut db = fast_database("Passkeys");
        let root = db.root_id();
        let only = entry("Passkey only", "");
        let mixed = entry("Password and passkey", "fixture-password");
        let mut missing_key = entry("Missing key", "");
        missing_key.remove_field(PASSKEY_PRIVATE_KEY_PEM);
        let mut malformed_id = entry("Malformed ID", "");
        malformed_id.set_field(PASSKEY_CREDENTIAL_ID, "not base64!", true);
        let mut empty_key = entry("Empty key", "");
        empty_key.set_field(PASSKEY_PRIVATE_KEY_PEM, "", true);
        let mut empty_id = entry("Empty ID", "");
        empty_id.set_field(PASSKEY_CREDENTIAL_ID, "", true);
        let expected = vec![only.uuid, mixed.uuid];
        for e in [only, missing_key, malformed_id, empty_key, empty_id] {
            let _ = db.add_entry(root, e, now);
        }
        let nested = Group::new("Nested", now);
        let nested_id = nested.uuid;
        let _ = db.add_group(root, nested, now);
        let _ = db.add_entry(nested_id, mixed, now);
        let mut history = Group::new("History", now);
        history.uuid = GENERATOR_HISTORY_GROUP_UUID;
        let _ = db.add_group(root, history, now);
        let _ = db.add_entry(
            GENERATOR_HISTORY_GROUP_UUID,
            entry("Generated history", ""),
            now,
        );
        let deleted = entry("Deleted", "");
        let deleted_id = deleted.uuid;
        let _ = db.add_entry(root, deleted, now);
        let _ = db.delete_entry(deleted_id, now);
        let deleted_group = Group::new("Deleted group", now);
        let deleted_group_id = deleted_group.uuid;
        let _ = db.add_group(root, deleted_group, now);
        let _ = db.add_entry(deleted_group_id, entry("Deleted nested", ""), now);
        let _ = db.delete_group(deleted_group_id, now);
        (db, expected)
    }

    /// A new database with a cheap KDF (Argon2id, 8 MiB, 1 iteration, parallelism 1), so tests do
    /// not spend a second per save.
    pub fn fast_database(name: &str) -> Database {
        let mut db = Database::new(name);
        if let KdfParams::Argon2 {
            variant,
            memory_bytes,
            iterations,
            parallelism,
            ..
        } = &mut db.settings.kdf
        {
            *variant = Argon2Variant::Id;
            *memory_bytes = 8 * 1024 * 1024;
            *iterations = 1;
            *parallelism = 1;
        }
        db
    }
}
