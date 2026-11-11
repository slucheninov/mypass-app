//! The vault session, settings, scheduling and the command facade behind the Tauri shell.
//!
//! This crate has no Tauri dependency, so it is testable without a window server. Every timing
//! rule runs through [`scheduler::Scheduler`]; tests drive it with [`scheduler::ManualScheduler`].

pub mod backups;
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
