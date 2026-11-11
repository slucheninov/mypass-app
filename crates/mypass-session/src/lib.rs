//! The vault session, settings, scheduling and the command facade behind the Tauri shell.
//!
//! This crate has no Tauri dependency, so it is testable without a window server. Every timing
//! rule runs through [`scheduler::Scheduler`]; tests drive it with [`scheduler::ManualScheduler`].

pub mod autolock;
pub mod backups;
pub mod clipboard_service;
pub mod dto;
pub mod editing;
pub mod fields;
pub mod queries;
pub mod saving;
pub mod scheduler;
pub mod session;
pub mod settings;
pub mod shortcut;
pub mod unlock_request;
pub mod vault_files;

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
            (vec![], false),
        ]
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
