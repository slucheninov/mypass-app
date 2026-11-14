//! The macOS body of [`CoordinatedFiles`]: `NSFileCoordinator`, `NSFileVersion`, the ubiquity
//! resource keys and an `NSFilePresenter`.
//!
//! Every call builds its own `NSURL` from the path, so a resource value is never served from the
//! cache of an older URL object (and the cache of the new one is cleared before reading anyway).
//! The coordinator's accessor block runs synchronously on the calling thread, before the
//! coordination call returns; a panic inside the accessor is caught at the block boundary (it
//! must not unwind through Objective-C frames) and resumed once the call has returned.
//!
//! Nothing here touches iCloud in an automated test unless `MYPASS_ICLOUD_INTEGRATION_TESTS=1`:
//! the default tests use a plain temporary directory, which is valid input for every call.

// `unsafe` is denied crate-wide and allowed in this module only: the presenter is an Objective-C
// subclass (`define_class!` and its protocol conformance are `unsafe` by nature), the resource
// keys are extern statics, and the accessor block receives a raw `NonNull<NSURL>`. Each block is
// minimal and carries a `// SAFETY:` note; the guard `unsafe_only_in_allow_listed_modules` lists
// this file.
#![allow(unsafe_code)]

use crate::coordinated::{
    map_download_status, Accessor, ChangeCallback, ConflictVersion, CoordinatedFiles, DownloadState,
};
use crate::Subscription;
use objc2::rc::{autoreleasepool, Retained};
use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{define_class, msg_send, AllocAnyThread, DefinedClass};
use objc2_foundation::{
    NSArray, NSError, NSFileCoordinator, NSFileCoordinatorReadingOptions,
    NSFileCoordinatorWritingOptions, NSFileManager, NSFilePresenter, NSFileVersion, NSNumber,
    NSOperationQueue, NSString, NSURLIsUbiquitousItemKey, NSURLUbiquitousItemDownloadingStatusKey,
    NSURL,
};
use std::cell::RefCell;
use std::io;
use std::panic::{catch_unwind, resume_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::ptr::NonNull;
use std::time::{Duration, UNIX_EPOCH};

/// [`CoordinatedFiles`] over Foundation's file coordination.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct MacCoordinatedFiles;

/// The file URL of `path` (a fresh object every call).
fn file_url(path: &Path) -> io::Result<Retained<NSURL>> {
    NSURL::from_file_path(path).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "the path cannot be turned into a file URL",
        )
    })
}

/// The path of a file URL handed back by Foundation.
fn url_path(url: &NSURL) -> io::Result<PathBuf> {
    url.to_file_path().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "the system returned a URL that is not a file path",
        )
    })
}

/// `NSCocoaErrorDomain` codes that map to a specific `io::ErrorKind`.
const NS_FILE_NO_SUCH_FILE: isize = 4;
const NS_FILE_READ_NO_SUCH_FILE: isize = 260;
const NS_FILE_READ_NO_PERMISSION: isize = 257;
const NS_FILE_WRITE_NO_PERMISSION: isize = 513;

/// Maps an `NSError` to an `io::Error`: POSIX errors keep their errno, the Cocoa "no such file"
/// and "no permission" codes keep their kind, everything else is `Other` with the system's text.
fn ns_error(error: &NSError) -> io::Error {
    let domain = error.domain().to_string();
    let code = error.code();
    if domain == "NSPOSIXErrorDomain" {
        if let Ok(errno) = i32::try_from(code) {
            return io::Error::from_raw_os_error(errno);
        }
    }
    let kind = match (domain.as_str(), code) {
        ("NSCocoaErrorDomain", NS_FILE_NO_SUCH_FILE | NS_FILE_READ_NO_SUCH_FILE) => {
            io::ErrorKind::NotFound
        }
        ("NSCocoaErrorDomain", NS_FILE_READ_NO_PERMISSION | NS_FILE_WRITE_NO_PERMISSION) => {
            io::ErrorKind::PermissionDenied
        }
        _ => io::ErrorKind::Other,
    };
    io::Error::new(
        kind,
        format!("{} ({domain} {code})", error.localizedDescription()),
    )
}

/// The outcome of the accessor inside a coordination call.
enum AccessorOutcome {
    NotRun,
    Returned(io::Result<()>),
    Panicked(Box<dyn std::any::Any + Send>),
}

/// Runs one coordination call (`coordinate` gets the block and the error slot) around `accessor`.
fn coordinate(
    accessor: Accessor<'_>,
    coordinate: impl FnOnce(
        &block2::DynBlock<dyn Fn(NonNull<NSURL>) + '_>,
        &mut Option<Retained<NSError>>,
    ),
) -> io::Result<()> {
    let accessor = RefCell::new(accessor);
    let outcome = RefCell::new(AccessorOutcome::NotRun);
    let block = block2::StackBlock::new(|url: NonNull<NSURL>| {
        // SAFETY: the coordinator passes a valid, live `NSURL` for the duration of the block.
        let url = unsafe { url.as_ref() };
        let result = catch_unwind(AssertUnwindSafe(|| {
            let path = url_path(url)?;
            let mut accessor = accessor.borrow_mut();
            (*accessor)(&path)
        }));
        *outcome.borrow_mut() = match result {
            Ok(result) => AccessorOutcome::Returned(result),
            Err(payload) => AccessorOutcome::Panicked(payload),
        };
    });
    let mut error = None;
    coordinate(&block, &mut error);
    match outcome.into_inner() {
        AccessorOutcome::Returned(result) => result,
        AccessorOutcome::Panicked(payload) => resume_unwind(payload),
        AccessorOutcome::NotRun => Err(error.map_or_else(
            || io::Error::other("the file coordinator did not run the accessor"),
            |e| ns_error(&e),
        )),
    }
}

fn new_coordinator() -> Retained<NSFileCoordinator> {
    NSFileCoordinator::initWithFilePresenter(NSFileCoordinator::alloc(), None)
}

impl CoordinatedFiles for MacCoordinatedFiles {
    fn coordinated_read(&self, path: &Path, accessor: Accessor<'_>) -> io::Result<()> {
        autoreleasepool(|_| {
            let url = file_url(path)?;
            let coordinator = new_coordinator();
            coordinate(accessor, |block, error| {
                coordinator.coordinateReadingItemAtURL_options_error_byAccessor(
                    &url,
                    NSFileCoordinatorReadingOptions::empty(),
                    Some(error),
                    block,
                );
            })
        })
    }

    fn coordinated_write(&self, path: &Path, accessor: Accessor<'_>) -> io::Result<()> {
        autoreleasepool(|_| {
            let url = file_url(path)?;
            let coordinator = new_coordinator();
            coordinate(accessor, |block, error| {
                coordinator.coordinateWritingItemAtURL_options_error_byAccessor(
                    &url,
                    NSFileCoordinatorWritingOptions::ForReplacing,
                    Some(error),
                    block,
                );
            })
        })
    }

    fn download_state(&self, path: &Path) -> io::Result<DownloadState> {
        autoreleasepool(|_| {
            let url = file_url(path)?;
            url.removeAllCachedResourceValues();
            // SAFETY: both keys are immutable `NSString` constants exported by Foundation, valid
            // for the life of the process.
            let (ubiquitous_key, status_key) = unsafe {
                (
                    NSURLIsUbiquitousItemKey,
                    NSURLUbiquitousItemDownloadingStatusKey,
                )
            };
            let keys = NSArray::from_slice(&[ubiquitous_key, status_key]);
            let values = url
                .resourceValuesForKeys_error(&keys)
                .map_err(|e| ns_error(&e))?;
            let ubiquitous = values
                .objectForKey(ubiquitous_key)
                .and_then(|v| v.downcast::<NSNumber>().ok())
                .is_some_and(|n| n.boolValue());
            let status = values
                .objectForKey(status_key)
                .and_then(|v| v.downcast::<NSString>().ok())
                .map(|s| s.to_string());
            // Foundation answers "not ubiquitous" for a missing file instead of failing. A
            // ubiquitous placeholder may have no file under its own name (the older `.icloud`
            // stub scheme), so only the non-ubiquitous answer is checked against the disk.
            if !ubiquitous {
                std::fs::metadata(path)?;
            }
            Ok(map_download_status(ubiquitous, status.as_deref()))
        })
    }

    fn request_download(&self, path: &Path) -> io::Result<()> {
        autoreleasepool(|_| {
            let url = file_url(path)?;
            NSFileManager::defaultManager()
                .startDownloadingUbiquitousItemAtURL_error(&url)
                .map_err(|e| ns_error(&e))
        })
    }

    fn unresolved_conflicts(&self, path: &Path) -> io::Result<Vec<ConflictVersion>> {
        autoreleasepool(|_| {
            let url = file_url(path)?;
            let Some(versions) = NSFileVersion::unresolvedConflictVersionsOfItemAtURL(&url) else {
                return Ok(Vec::new());
            };
            versions
                .iter()
                .map(|version| {
                    Ok(ConflictVersion {
                        path: url_path(&version.URL())?,
                        modified: version.modificationDate().and_then(|date| {
                            let secs = date.timeIntervalSince1970();
                            Duration::try_from_secs_f64(secs)
                                .ok()
                                .and_then(|d| UNIX_EPOCH.checked_add(d))
                        }),
                        saving_computer: version
                            .localizedNameOfSavingComputer()
                            .map(|name| name.to_string()),
                    })
                })
                .collect()
        })
    }

    fn mark_conflicts_resolved(&self, path: &Path, versions: &[ConflictVersion]) -> io::Result<()> {
        if versions.is_empty() {
            return Ok(());
        }
        autoreleasepool(|_| {
            let url = file_url(path)?;
            let Some(current) = NSFileVersion::unresolvedConflictVersionsOfItemAtURL(&url) else {
                return Ok(());
            };
            for version in current.iter() {
                let version_path = url_path(&version.URL())?;
                if versions.iter().any(|v| v.path == version_path) {
                    version.setResolved(true);
                }
            }
            Ok(())
        })
    }

    fn icloud_available(&self) -> bool {
        autoreleasepool(|_| {
            NSFileManager::defaultManager()
                .ubiquityIdentityToken()
                .is_some()
        })
    }

    fn present(&self, path: &Path, on_change: ChangeCallback) -> io::Result<Subscription> {
        autoreleasepool(|_| {
            let url = file_url(path)?;
            let queue = NSOperationQueue::new();
            // One change at a time, in order: the callback only hands the event on.
            queue.setMaxConcurrentOperationCount(1);
            let presenter = Presenter::new(PresenterIvars {
                url,
                queue,
                on_change,
            });
            NSFileCoordinator::addFilePresenter(ProtocolObject::from_ref(&*presenter));
            Ok(Subscription::new(move || {
                autoreleasepool(|_| {
                    NSFileCoordinator::removeFilePresenter(ProtocolObject::from_ref(&*presenter));
                });
            }))
        })
    }
}

/// The presenter's state; immutable after construction.
struct PresenterIvars {
    url: Retained<NSURL>,
    queue: Retained<NSOperationQueue>,
    on_change: ChangeCallback,
}

impl PresenterIvars {
    /// Calls the callback; a panic stops here instead of unwinding into Foundation.
    fn notify(&self) {
        let _ = catch_unwind(AssertUnwindSafe(|| (self.on_change)()));
    }
}

define_class!(
    // SAFETY:
    // - The superclass NSObject has no subclassing requirements.
    // - `Presenter` does not implement `Drop`; the ivars are dropped by the generated code.
    // - The class name is unique to this crate.
    #[unsafe(super(NSObject))]
    #[name = "MyPassCoordinatedFilePresenter"]
    #[ivars = PresenterIvars]
    struct Presenter;

    unsafe impl NSObjectProtocol for Presenter {}

    // SAFETY: the method signatures match the `NSFilePresenter` protocol declarations.
    unsafe impl NSFilePresenter for Presenter {
        #[unsafe(method_id(presentedItemURL))]
        fn presented_item_url(&self) -> Option<Retained<NSURL>> {
            Some(self.ivars().url.clone())
        }

        #[unsafe(method_id(presentedItemOperationQueue))]
        fn presented_item_operation_queue(&self) -> Retained<NSOperationQueue> {
            self.ivars().queue.clone()
        }

        #[unsafe(method(presentedItemDidChange))]
        fn presented_item_did_change(&self) {
            self.ivars().notify();
        }

        #[unsafe(method(presentedItemDidGainVersion:))]
        fn presented_item_did_gain_version(&self, _version: &NSFileVersion) {
            self.ivars().notify();
        }
    }
);

// SAFETY: the ivars are immutable after construction and each is `Send + Sync` (`NSURL` and
// `NSOperationQueue` are thread-safe Foundation classes, the callback is `Send + Sync`); NSObject's
// retain and release are thread-safe. Foundation itself calls the presenter from its own queues.
unsafe impl Send for Presenter {}
// SAFETY: as above; no method mutates state.
unsafe impl Sync for Presenter {}

impl Presenter {
    fn new(ivars: PresenterIvars) -> Retained<Self> {
        let this = Self::alloc().set_ivars(ivars);
        // SAFETY: `init` is NSObject's designated initializer, called once on a fresh allocation.
        unsafe { msg_send![super(this), init] }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coordinated::{STATUS_CURRENT, STATUS_DOWNLOADED, STATUS_NOT_DOWNLOADED};
    use objc2_foundation::{
        NSURLUbiquitousItemDownloadingStatusCurrent,
        NSURLUbiquitousItemDownloadingStatusDownloaded,
        NSURLUbiquitousItemDownloadingStatusNotDownloaded,
    };
    use std::sync::mpsc;

    #[test]
    fn sdk_status_strings_match_the_mapping_constants() {
        // SAFETY: immutable Foundation string constants.
        let (current, downloaded, not_downloaded) = unsafe {
            (
                NSURLUbiquitousItemDownloadingStatusCurrent.to_string(),
                NSURLUbiquitousItemDownloadingStatusDownloaded.to_string(),
                NSURLUbiquitousItemDownloadingStatusNotDownloaded.to_string(),
            )
        };
        assert_eq!(current, STATUS_CURRENT);
        assert_eq!(downloaded, STATUS_DOWNLOADED);
        assert_eq!(not_downloaded, STATUS_NOT_DOWNLOADED);
    }

    #[test]
    fn coordinated_write_then_read_round_trips_on_a_plain_file() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("vault.kdbx");
        let files = MacCoordinatedFiles;
        files
            .coordinated_write(&path, &mut |p| std::fs::write(p, b"kdbx bytes"))
            .unwrap();
        let mut read = Vec::new();
        files
            .coordinated_read(&path, &mut |p| {
                read = std::fs::read(p)?;
                Ok(())
            })
            .unwrap();
        assert_eq!(read, b"kdbx bytes");
    }

    #[test]
    fn coordinated_read_of_a_missing_file_is_not_found() {
        let tmp = tempfile::tempdir().unwrap();
        let err = MacCoordinatedFiles
            .coordinated_read(&tmp.path().join("missing.kdbx"), &mut |p| {
                std::fs::read(p).map(|_| ())
            })
            .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn accessor_errors_come_back_unchanged() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("vault.kdbx");
        std::fs::write(&path, b"x").unwrap();
        let err = MacCoordinatedFiles
            .coordinated_read(&path, &mut |_| {
                Err(io::Error::new(io::ErrorKind::TimedOut, "slow"))
            })
            .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::TimedOut);
    }

    #[test]
    fn accessor_panic_is_resumed_after_the_call() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("vault.kdbx");
        std::fs::write(&path, b"x").unwrap();
        let result = catch_unwind(|| {
            let _ = MacCoordinatedFiles.coordinated_read(&path, &mut |_| panic!("accessor boom"));
        });
        assert!(result.is_err());
    }

    #[test]
    fn plain_file_is_not_ubiquitous_and_has_no_conflicts() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("vault.kdbx");
        std::fs::write(&path, b"x").unwrap();
        let files = MacCoordinatedFiles;
        assert_eq!(
            files.download_state(&path).unwrap(),
            DownloadState::NotUbiquitous
        );
        assert!(files.unresolved_conflicts(&path).unwrap().is_empty());
        files.mark_conflicts_resolved(&path, &[]).unwrap();
        let stale = ConflictVersion {
            path: tmp.path().join("gone"),
            modified: None,
            saving_computer: None,
        };
        files.mark_conflicts_resolved(&path, &[stale]).unwrap();
    }

    #[test]
    fn download_state_of_a_missing_file_is_not_found() {
        let tmp = tempfile::tempdir().unwrap();
        let err = MacCoordinatedFiles
            .download_state(&tmp.path().join("missing.kdbx"))
            .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
    }

    fn is_presented(path: &Path) -> bool {
        autoreleasepool(|_| {
            NSFileCoordinator::filePresenters().iter().any(|p| {
                p.presentedItemURL()
                    .and_then(|u| u.to_file_path())
                    .is_some_and(|p| p == path)
            })
        })
    }

    #[test]
    fn presenter_is_registered_until_the_subscription_drops() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("vault.kdbx");
        std::fs::write(&path, b"x").unwrap();
        let subscription = MacCoordinatedFiles
            .present(&path, std::sync::Arc::new(|| {}))
            .unwrap();
        assert!(is_presented(&path));
        drop(subscription);
        assert!(!is_presented(&path));
    }

    #[test]
    fn presenter_hears_a_coordinated_write_from_another_coordinator() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("vault.kdbx");
        std::fs::write(&path, b"one").unwrap();
        let (tx, rx) = mpsc::channel();
        let tx = std::sync::Mutex::new(tx);
        let _subscription = MacCoordinatedFiles
            .present(
                &path,
                std::sync::Arc::new(move || {
                    let _ = tx.lock().map(|tx| tx.send(()));
                }),
            )
            .unwrap();
        MacCoordinatedFiles
            .coordinated_write(&path, &mut |p| std::fs::write(p, b"two"))
            .unwrap();
        rx.recv_timeout(Duration::from_secs(10))
            .expect("the presenter heard the write");
    }

    #[test]
    fn non_posix_errors_keep_the_system_text_and_code() {
        let error = NSError::new(2, &NSString::from_str("SomeDomain"));
        let mapped = ns_error(&error);
        assert_eq!(mapped.kind(), io::ErrorKind::Other);
        assert!(mapped.to_string().contains("SomeDomain 2"));
    }

    /// Real iCloud Drive: needs a signed-in account and `MYPASS_ICLOUD_TEST_DIR` pointing at a
    /// folder inside iCloud Drive. Run with `MYPASS_ICLOUD_INTEGRATION_TESTS=1 cargo test -p
    /// mypass-platform -- --ignored icloud_`.
    #[test]
    #[ignore = "touches iCloud Drive; set MYPASS_ICLOUD_INTEGRATION_TESTS=1 and MYPASS_ICLOUD_TEST_DIR"]
    fn icloud_file_reports_a_ubiquitous_state() {
        if std::env::var("MYPASS_ICLOUD_INTEGRATION_TESTS").as_deref() != Ok("1") {
            println!("skipped: MYPASS_ICLOUD_INTEGRATION_TESTS is not 1");
            return;
        }
        let Ok(dir) = std::env::var("MYPASS_ICLOUD_TEST_DIR") else {
            println!("skipped: MYPASS_ICLOUD_TEST_DIR is not set");
            return;
        };
        let files = MacCoordinatedFiles;
        assert!(files.icloud_available(), "no iCloud account signed in");
        let path =
            PathBuf::from(dir).join(format!("mypass-icloud-test-{}.bin", std::process::id()));
        files
            .coordinated_write(&path, &mut |p| std::fs::write(p, b"icloud"))
            .unwrap();
        let state = files.download_state(&path).unwrap();
        println!("download state right after the write: {state:?}");
        assert_ne!(state, DownloadState::NotUbiquitous);
        assert!(state.is_local());
        files.request_download(&path).unwrap();
        assert!(files.unresolved_conflicts(&path).unwrap().is_empty());
        files
            .coordinated_write(&path, &mut |p| std::fs::remove_file(p))
            .unwrap();
    }
}
