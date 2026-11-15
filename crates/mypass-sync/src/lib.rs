//! Vault sync (stage 5): the secondary-folder backends and sync engine, the folder watcher and the
//! iCloud Drive `FileIo` over the `mypass-platform` coordinated-file seam.
//!
//! Safe code only (`unsafe_code = "forbid"` through `[workspace.lints]`): every Objective-C call
//! stays in `mypass-platform`. Portable: the only platform dependency is the `mypass-platform` seam (`CoordinatedFiles`), whose
//! fake runs every iCloud rule on any OS; nothing here depends on the session crate.
//! Logs and errors carry codes and paths only, never vault bytes or passwords (R37).

pub mod backend;
pub mod checkpoint;
pub mod engine;
pub mod icloud;
pub mod local_folder;
pub mod watcher;

pub use backend::{FileIoBackend, StorageBackend, StorageError, WriteOutcome};
pub use checkpoint::{
    FileCheckpointStore, InMemoryCheckpointStore, SyncCheckpoint, SyncCheckpointStore,
};
pub use engine::{
    MergedRemote, PendingCheckpoint, SyncEngine, SyncError, SyncMode, SyncResult, MAX_SYNC_ATTEMPTS,
};
pub use icloud::{
    ensure_downloaded, merge_vault_bytes, resolve_conflicts, vault_location_of, Clock,
    ConflictOutcome, ConflictVersions, CoordinatedProbe, DownloadProbe, ICloudDriveLocation,
    ICloudError, PlatformVersions, SystemClock, VaultLocation, DOWNLOAD_TIMEOUT,
};
pub use local_folder::LocalFolderBackend;
pub use watcher::{Debouncer, EchoFilter, FolderWatcher, WATCH_DEBOUNCE};
