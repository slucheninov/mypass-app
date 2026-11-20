//! Where the open vault is stored (stage 5, T8; rulings R17, R19, R37, R38).
//!
//! A vault path resolves to a [`ResolvedStorage`]: the `FileIo` its `VaultFile` reads and writes
//! through, its [`VaultLocation`] and, for iCloud Drive, the [`ICloudFileIo`] that also merges
//! conflict versions, serves the offline cache and reports a remote change. The session asks the
//! installed [`IoResolver`] (production: [`icloud_resolver`]) once per open; without one, or when
//! it returns `None`, the vault uses the session's own IO seam (`SessionDeps::file_io`, replaced by
//! `VaultSession::set_file_io`).
//!
//! One home per name (R19): `vault_location_of`, `ICloudDriveLocation`, `DownloadProbe` and
//! `ConflictVersions` live in `mypass_sync::icloud`; this module holds the status, the resolved
//! storage and the resolver types.

use mypass_kdbx::{FileIo, KdfLimits};
use mypass_platform::CoordinatedFiles;
use mypass_sync::icloud::{vault_location_of, ICloudFileIo, IoHealth, VaultLocation};
use serde::Serialize;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

/// How long the session waits after a remote-change notification before it checks the file
/// (the notifications of one sync come in bursts).
pub const REMOTE_CHANGE_DEBOUNCE: Duration = Duration::from_millis(500);

/// The merge step handed to a [`ConflictResolver`]: `(current, version) -> merged bytes`; an
/// `Err` leaves that version unresolved.
pub type ConflictMerge<'a> = &'a mut dyn FnMut(&[u8], &[u8]) -> io::Result<Vec<u8>>;

/// Merges the unresolved conflict versions of the vault at the path into the file, through the
/// given merge step; `Ok(true)` when the file was rewritten.
pub type ConflictResolver = Arc<dyn Fn(&Path, ConflictMerge<'_>) -> io::Result<bool> + Send + Sync>;

/// The storage of a vault path, or `None` for the session's own IO seam.
pub type IoResolver = Arc<dyn Fn(&Path) -> Option<ResolvedStorage> + Send + Sync>;

/// Told the new [`StorageStatus`] whenever it changed; runs after the session lock is released.
pub type StorageObserver = Arc<dyn Fn(&StorageStatus) + Send + Sync>;

/// The storage one vault was opened with.
#[derive(Clone)]
pub struct ResolvedStorage {
    pub io: Arc<dyn FileIo>,
    pub location: VaultLocation,
    /// Set for iCloud Drive: conflict versions, offline cache, presenter and health.
    pub icloud: Option<Arc<ICloudFileIo>>,
}

impl fmt::Debug for ResolvedStorage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ResolvedStorage")
            .field("location", &self.location)
            .field("icloud", &self.icloud.is_some())
            .finish_non_exhaustive()
    }
}

impl ResolvedStorage {
    /// A plain local file through `io`.
    pub fn plain(io: Arc<dyn FileIo>) -> Self {
        Self {
            io,
            location: VaultLocation::Local,
            icloud: None,
        }
    }

    /// A vault in iCloud Drive.
    pub fn icloud(io: Arc<ICloudFileIo>) -> Self {
        Self {
            io: io.clone(),
            location: VaultLocation::ICloudDrive,
            icloud: Some(io),
        }
    }

    pub(crate) fn kdf_limits(&self) -> KdfLimits {
        match self.location {
            VaultLocation::Local => KdfLimits::Local,
            VaultLocation::ICloudDrive => KdfLimits::Remote,
        }
    }

    /// The conflict resolver of this storage (iCloud Drive only).
    pub fn conflicts(&self) -> Option<ConflictResolver> {
        let icloud = self.icloud.clone()?;
        Some(Arc::new(move |_path: &Path, merge: ConflictMerge<'_>| {
            icloud.resolve_conflicts(merge).map(|o| o.changed())
        }))
    }

    /// The current health (all clear for a plain file).
    pub fn health(&self) -> IoHealth {
        self.icloud
            .as_ref()
            .map(|io| io.health())
            .unwrap_or_default()
    }

    /// The status the UI shows.
    pub fn status(&self) -> StorageStatus {
        let health = self.health();
        StorageStatus {
            location: self.location,
            fallback_active: health.fallback_active,
            cache_ahead: health.cache_ahead,
            downloading: health.downloading,
        }
    }

    /// Whether two storages are the same instance (an unlock reuses the open vault's).
    pub(crate) fn same_as(&self, other: &ResolvedStorage) -> bool {
        Arc::ptr_eq(&self.io, &other.io)
    }
}

/// The production resolver: a path inside a ubiquity container (`~/Library/Mobile Documents`)
/// gets an [`ICloudFileIo`] over `coordinated`, caching under `cache_dir`
/// (`<data dir>/icloud-cache`); every other path the session's own IO seam.
pub fn icloud_resolver(
    coordinated: Arc<dyn CoordinatedFiles>,
    home: PathBuf,
    cache_dir: PathBuf,
) -> IoResolver {
    Arc::new(move |path: &Path| match vault_location_of(path, &home) {
        VaultLocation::ICloudDrive => Some(ResolvedStorage::icloud(Arc::new(ICloudFileIo::new(
            coordinated.clone(),
            path,
            &cache_dir,
        )))),
        VaultLocation::Local => None,
    })
}

/// Windows has no coordinated iCloud backend: even a macOS-looking path stays local.
/// The explicit policy input keeps the Windows selection covered on every test host.
pub(crate) fn platform_resolver(
    windows: bool,
    coordinated: Arc<dyn CoordinatedFiles>,
    home: PathBuf,
    cache_dir: PathBuf,
) -> Option<IoResolver> {
    if windows {
        None
    } else {
        Some(icloud_resolver(coordinated, home, cache_dir))
    }
}

/// Where the open vault is stored and how that storage is doing (the `StorageStatusChanged`
/// event payload).
#[derive(Serialize, specta::Type, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct StorageStatus {
    /// `local` also when no vault is open.
    pub location: VaultLocation,
    /// The vault is served from the offline copy (iCloud Drive is unavailable, or the copy holds
    /// edits not in iCloud yet).
    pub fallback_active: bool,
    /// The offline copy holds edits that are not in iCloud yet; they are merged into iCloud when
    /// it is available again.
    pub cache_ahead: bool,
    /// The last read waited too long for iCloud to download the vault.
    pub downloading: bool,
}

impl Default for StorageStatus {
    fn default() -> Self {
        Self {
            location: VaultLocation::Local,
            fallback_active: false,
            cache_ahead: false,
            downloading: false,
        }
    }
}

#[cfg(test)]
#[path = "storage_tests.rs"]
mod session_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use mypass_kdbx::PlainFileIo;
    use mypass_platform::testing::FakeCoordinatedFiles;
    use mypass_sync::icloud::ICloudDriveLocation;

    #[test]
    fn windows_production_resolver_keeps_cloud_lookalikes_local() {
        let home = tempfile::tempdir().unwrap();
        let inside = ICloudDriveLocation::root(home.path()).join("v.kdbx");
        for windows in [true, false] {
            let resolver = platform_resolver(
                windows,
                Arc::new(FakeCoordinatedFiles::new()),
                home.path().into(),
                home.path().join("cache"),
            );
            let resolved = resolver.and_then(|r| r(&inside));
            assert_eq!(resolved.is_none(), windows);
        }
        assert!(!home.path().join("cache").exists());
    }

    #[test]
    fn io_resolver_is_applied_per_vault_path() {
        let home = tempfile::tempdir().unwrap();
        let resolver = icloud_resolver(
            Arc::new(FakeCoordinatedFiles::new()),
            home.path().to_path_buf(),
            home.path().join("cache"),
        );
        let inside = ICloudDriveLocation::root(home.path()).join("v.kdbx");
        let storage = resolver(&inside).expect("iCloud storage");
        assert_eq!(storage.location, VaultLocation::ICloudDrive);
        assert!(storage.icloud.is_some());
        assert!(storage.conflicts().is_some());
        assert!(resolver(&home.path().join("Documents").join("v.kdbx")).is_none());
        let plain = ResolvedStorage::plain(Arc::new(PlainFileIo));
        assert!(plain.conflicts().is_none());
        assert_eq!(plain.status(), StorageStatus::default());
    }

    #[test]
    fn storage_status_serialises_camel_case() {
        let v = serde_json::to_value(StorageStatus {
            location: VaultLocation::ICloudDrive,
            fallback_active: true,
            cache_ahead: false,
            downloading: false,
        })
        .unwrap();
        assert_eq!(
            v,
            serde_json::json!({
                "location": "iCloudDrive",
                "fallbackActive": true,
                "cacheAhead": false,
                "downloading": false,
            })
        );
    }
}
