//! Vault sync (stage 5): the secondary-folder backends and sync engine, the folder watcher and the
//! iCloud Drive `FileIo` over the `mypass-platform` coordinated-file seam.
//!
//! Safe code only (`unsafe_code = "forbid"` through `[workspace.lints]`): every Objective-C call
//! stays in `mypass-platform`. Portable: nothing here depends on the platform or session crates.
//! Logs and errors carry codes and paths only, never vault bytes or passwords (R37).

pub mod backend;
pub mod checkpoint;
pub mod engine;
pub mod local_folder;
pub mod watcher;

pub use backend::{FileIoBackend, StorageBackend, StorageError};
pub use checkpoint::{
    FileCheckpointStore, InMemoryCheckpointStore, SyncCheckpoint, SyncCheckpointStore,
};
pub use engine::{
    MergedRemote, PendingCheckpoint, SyncEngine, SyncError, SyncResult, MAX_SYNC_ATTEMPTS,
};
pub use local_folder::LocalFolderBackend;
pub use watcher::{Debouncer, EchoFilter, FolderWatcher, WATCH_DEBOUNCE};
