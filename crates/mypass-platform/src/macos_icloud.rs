//! The macOS body of [`CoordinatedFiles`]: `NSFileCoordinator`, `NSFileVersion`, the ubiquity
//! resource keys and an `NSFilePresenter`.
//!
//! Every call builds its own `NSURL` from the path, so a resource value is never served from the
//! cache of an older URL object (and the cache of the new one is cleared before reading anyway).
//! The coordinator's accessor block runs synchronously on the calling thread, before the
//! coordination call returns; a panic inside the accessor is caught at the block boundary (it
//! must not unwind through Objective-C frames) and resumed once the call has returned.
//!
//! Every coordination call runs under a deadline (`with_deadline`): a watcher thread cancels the
//! coordinator (`-[NSFileCoordinator cancel]`, documented for use from another thread) when the
//! accessor has not started in time, which makes the blocked call return with
//! `NSUserCancelledError`; that is reported as `ErrorKind::TimedOut`. The accessor block asks the
//! deadline gate before it runs the accessor, so an access whose deadline won never runs, even if
//! Foundation calls the block after the cancel. The block is a stack block borrowed for the
//! synchronous call only: once the call has returned, Foundation never calls it again.
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
    coordination_timed_out, map_download_status, with_deadline, Accessor, ChangeCallback,
    ConflictVersion, CoordinatedFiles, DeadlineGate, DownloadState,
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

/// Runs one coordination call (`coordinate` gets the block and the error slot) around `accessor`;
/// the accessor runs only if `gate` lets it in before the deadline.
fn coordinate(
    gate: &DeadlineGate,
    accessor: Accessor<'_>,
    coordinate: impl FnOnce(
        &block2::DynBlock<dyn Fn(NonNull<NSURL>) + '_>,
        &mut Option<Retained<NSError>>,
    ),
) -> io::Result<()> {
    let accessor = RefCell::new(accessor);
    let outcome = RefCell::new(AccessorOutcome::NotRun);
    let block = block2::StackBlock::new(|url: NonNull<NSURL>| {
        if !gate.enter() {
            // The deadline won (or the block is called a second time): never touch the file.
            return;
        }
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
        AccessorOutcome::NotRun if gate.expired() => Err(coordination_timed_out()),
        AccessorOutcome::NotRun => Err(error.map_or_else(
            || io::Error::other("the file coordinator did not run the accessor"),
            |e| ns_error(&e),
        )),
    }
}

fn new_coordinator() -> Retained<NSFileCoordinator> {
    NSFileCoordinator::initWithFilePresenter(NSFileCoordinator::alloc(), None)
}

/// The coordinator as the deadline watcher sees it: it only ever calls `cancel`.
struct CancelHandle<'a>(&'a NSFileCoordinator);

// SAFETY: the watcher thread only calls `-[NSFileCoordinator cancel]`, which Apple documents as
// the way to cancel a coordination call from another thread; the coordinator outlives the handle
// (`with_deadline` joins the watcher before it returns, and the coordinator lives past that).
unsafe impl Send for CancelHandle<'_> {}

impl CancelHandle<'_> {
    fn cancel(self) {
        self.0.cancel();
    }
}

/// One coordinated access of `path` under a deadline; `run` issues the coordination call.
fn coordinated(
    path: &Path,
    timeout: Duration,
    accessor: Accessor<'_>,
    run: impl FnOnce(
        &NSFileCoordinator,
        &NSURL,
        &block2::DynBlock<dyn Fn(NonNull<NSURL>) + '_>,
        &mut Option<Retained<NSError>>,
    ),
) -> io::Result<()> {
    autoreleasepool(|_| {
        let url = file_url(path)?;
        let coordinator = new_coordinator();
        let handle = CancelHandle(&coordinator);
        with_deadline(
            timeout,
            move || handle.cancel(),
            |gate| {
                coordinate(gate, accessor, |block, error| {
                    run(&coordinator, &url, block, error);
                })
            },
        )
    })
}

impl CoordinatedFiles for MacCoordinatedFiles {
    fn coordinated_read(
        &self,
        path: &Path,
        timeout: Duration,
        accessor: Accessor<'_>,
    ) -> io::Result<()> {
        coordinated(path, timeout, accessor, |coordinator, url, block, error| {
            coordinator.coordinateReadingItemAtURL_options_error_byAccessor(
                url,
                NSFileCoordinatorReadingOptions::empty(),
                Some(error),
                block,
            );
        })
    }

    fn coordinated_write(
        &self,
        path: &Path,
        timeout: Duration,
        accessor: Accessor<'_>,
    ) -> io::Result<()> {
        coordinated(path, timeout, accessor, |coordinator, url, block, error| {
            coordinator.coordinateWritingItemAtURL_options_error_byAccessor(
                url,
                NSFileCoordinatorWritingOptions::ForReplacing,
                Some(error),
                block,
            );
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
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{mpsc, Arc};

    // Foundation presenter registration is process-wide. Isolate native cases from each other,
    // while each case still races its own holder and contender on separate threads.
    static NATIVE_CASE: std::sync::Mutex<()> = std::sync::Mutex::new(());
    const T: Duration = Duration::from_secs(60);

    /// Holds a coordinated write of `path` on another thread (another coordinator, as the sync
    /// daemon or a presenter would) until the returned sender is used or dropped.
    fn hold_write(path: &Path) -> (mpsc::Sender<()>, std::thread::JoinHandle<()>) {
        let (held_tx, held_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let path = path.to_path_buf();
        let holder = std::thread::spawn(move || {
            MacCoordinatedFiles
                .coordinated_write(&path, T, &mut |_| {
                    let _ = held_tx.send(());
                    let _ = release_rx.recv_timeout(T);
                    Ok(())
                })
                .unwrap();
        });
        held_rx.recv_timeout(T).expect("the holder got the item");
        (release_tx, holder)
    }

    #[test]
    fn a_read_waiting_for_a_held_item_times_out_and_never_runs() {
        let _case = NATIVE_CASE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("vault.kdbx");
        std::fs::write(&path, b"x").unwrap();
        let (release, holder) = hold_write(&path);
        let ran = Arc::new(AtomicBool::new(false));
        let r = Arc::clone(&ran);
        let err = MacCoordinatedFiles
            .coordinated_read(&path, Duration::from_millis(200), &mut |_| {
                r.store(true, Ordering::SeqCst);
                Ok(())
            })
            .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::TimedOut, "{err}");
        release.send(()).unwrap();
        holder.join().unwrap();
        assert!(!ran.load(Ordering::SeqCst));
        // The item is free again: the next read goes through.
        MacCoordinatedFiles
            .coordinated_read(&path, T, &mut |_| Ok(()))
            .unwrap();
    }

    #[test]
    fn a_cancelled_write_never_runs_later() {
        let _case = NATIVE_CASE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("vault.kdbx");
        std::fs::write(&path, b"original").unwrap();
        let (release, holder) = hold_write(&path);
        let ran = Arc::new(AtomicBool::new(false));
        let r = Arc::clone(&ran);
        let err = MacCoordinatedFiles
            .coordinated_write(&path, Duration::from_millis(200), &mut |p| {
                r.store(true, Ordering::SeqCst);
                std::fs::write(p, b"late write")
            })
            .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::TimedOut, "{err}");
        release.send(()).unwrap();
        holder.join().unwrap();
        // Another coordinated access after the release proves the queue drained; the cancelled
        // write still never ran.
        MacCoordinatedFiles
            .coordinated_read(&path, T, &mut |_| Ok(()))
            .unwrap();
        assert!(!ran.load(Ordering::SeqCst));
        assert_eq!(std::fs::read(&path).unwrap(), b"original");
    }

    #[test]
    fn sdk_status_strings_match_the_mapping_constants() {
        let _case = NATIVE_CASE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
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
        let _case = NATIVE_CASE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("vault.kdbx");
        let files = MacCoordinatedFiles;
        files
            .coordinated_write(&path, T, &mut |p| std::fs::write(p, b"kdbx bytes"))
            .unwrap();
        let mut read = Vec::new();
        files
            .coordinated_read(&path, T, &mut |p| {
                read = std::fs::read(p)?;
                Ok(())
            })
            .unwrap();
        assert_eq!(read, b"kdbx bytes");
    }

    #[test]
    fn coordinated_read_of_a_missing_file_is_not_found() {
        let _case = NATIVE_CASE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let tmp = tempfile::tempdir().unwrap();
        let err = MacCoordinatedFiles
            .coordinated_read(&tmp.path().join("missing.kdbx"), T, &mut |p| {
                std::fs::read(p).map(|_| ())
            })
            .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn accessor_errors_come_back_unchanged() {
        let _case = NATIVE_CASE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("vault.kdbx");
        std::fs::write(&path, b"x").unwrap();
        let err = MacCoordinatedFiles
            .coordinated_read(&path, T, &mut |_| {
                Err(io::Error::new(io::ErrorKind::TimedOut, "slow"))
            })
            .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::TimedOut);
    }

    #[test]
    fn accessor_panic_is_resumed_after_the_call() {
        let _case = NATIVE_CASE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("vault.kdbx");
        std::fs::write(&path, b"x").unwrap();
        let result = catch_unwind(|| {
            let _ =
                MacCoordinatedFiles.coordinated_read(&path, T, &mut |_| panic!("accessor boom"));
        });
        assert!(result.is_err());
    }

    #[test]
    fn plain_file_is_not_ubiquitous_and_has_no_conflicts() {
        let _case = NATIVE_CASE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
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
        let _case = NATIVE_CASE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
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
        let _case = NATIVE_CASE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
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
        let _case = NATIVE_CASE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
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
            .coordinated_write(&path, T, &mut |p| std::fs::write(p, b"two"))
            .unwrap();
        rx.recv_timeout(T).expect("the presenter heard the write");
    }

    #[test]
    fn non_posix_errors_keep_the_system_text_and_code() {
        let _case = NATIVE_CASE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
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
            .coordinated_write(&path, T, &mut |p| std::fs::write(p, b"icloud"))
            .unwrap();
        let state = files.download_state(&path).unwrap();
        println!("download state right after the write: {state:?}");
        assert_ne!(state, DownloadState::NotUbiquitous);
        assert!(state.is_local());
        files.request_download(&path).unwrap();
        assert!(files.unresolved_conflicts(&path).unwrap().is_empty());
        files
            .coordinated_write(&path, T, &mut |p| std::fs::remove_file(p))
            .unwrap();
    }
}
