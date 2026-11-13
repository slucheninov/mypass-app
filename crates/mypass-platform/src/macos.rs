//! macOS: sleep, wake and screen-lock events, the concealed clipboard, the keyboard layout and the
//! running-application lookup (the Keychain secret stores live in `macos_keychain.rs`).
//!
//! Why screen lock is observed separately from sleep: the old app only observed `willSleep`, so a
//! locked-but-awake Mac (hot corner, Cmd+Ctrl+Q, lid closed on an external display) never locked
//! the vault. The distributed `com.apple.screenIsLocked` notification is the fix (spec section 8).
//!
//! # Threads
//!
//! Observers are registered with a `nil` queue, so each sink runs synchronously on the thread that
//! posts the notification: `NSWorkspace` posts sleep and wake on the main thread, and distributed
//! notifications (screen lock, input-source change) are delivered through the main thread's run
//! loop. In the app every sink therefore runs on the main thread; it must be cheap and must not
//! block (hand the event to a channel). Registration and removal work from any thread
//! (`NSNotificationCenter` is thread-safe). Objective-C objects that are not `Send` are never
//! stored: the notification centers and pasteboards are singletons re-fetched inside each call.

// `unsafe` is denied crate-wide (`[lints.rust] unsafe_code = "deny"` in this crate's manifest) and
// allowed in this module only: the Foundation bindings mark observer registration and removal
// `unsafe`, and the Carbon Text Input Source API has no Rust binding at all. Every `unsafe` block
// is minimal, sits in a small safe function and carries a `// SAFETY:` note. The guard
// `unsafe_only_in_allow_listed_modules` keeps `unsafe` out of every other file.
#![allow(unsafe_code)]

use crate::fallback::{
    UnsupportedDeviceUnlock, UnsupportedHostRegistrar, UnsupportedLaunchAtLogin,
};
use crate::macos_keychain::MacKeychainSecretStore;
use crate::{
    AppPaths, ClipboardStamp, KeyboardLayout, Platform, PlatformError, PlatformResult,
    SecureClipboard, Subscription, SystemEvent, SystemEvents,
};
use block2::RcBlock;
use objc2::rc::{autoreleasepool, Retained};
use objc2::runtime::{AnyObject, NSObjectProtocol, ProtocolObject};
use objc2::MainThreadMarker;
use objc2_app_kit::{NSPasteboard, NSPasteboardContentsOptions, NSRunningApplication, NSWorkspace};
use objc2_foundation::{
    NSArray, NSData, NSDistributedNotificationCenter, NSNotification, NSNotificationCenter,
    NSOperationQueue, NSString,
};
use std::ffi::c_void;
use std::path::PathBuf;
use std::ptr::NonNull;
use std::sync::{mpsc, Arc};
use std::time::Duration;

/// Value of AppKit's `NSWorkspaceWillSleepNotification` (pinned by a test against the SDK).
pub(crate) const WILL_SLEEP: &str = "NSWorkspaceWillSleepNotification";
/// Value of AppKit's `NSWorkspaceDidWakeNotification` (pinned by a test against the SDK).
pub(crate) const DID_WAKE: &str = "NSWorkspaceDidWakeNotification";
/// Distributed notification posted when the screen locks.
pub(crate) const SCREEN_LOCKED: &str = "com.apple.screenIsLocked";
/// Distributed notification posted when the screen unlocks.
pub(crate) const SCREEN_UNLOCKED: &str = "com.apple.screenIsUnlocked";
/// The nspasteboard.org marker that tells clipboard managers not to record an item.
pub(crate) const CONCEALED_TYPE: &str = "org.nspasteboard.ConcealedType";
/// Value of AppKit's `NSPasteboardTypeString` (pinned by a test against the SDK).
pub(crate) const STRING_TYPE: &str = "public.utf8-plain-text";
/// Distributed notification posted when the selected keyboard input source changes.
pub(crate) const INPUT_SOURCE_CHANGED: &str =
    "com.apple.Carbon.TISNotifySelectedKeyboardInputSourceChanged";

/// How long a background thread waits for the main thread to read the keyboard layout.
const MAIN_THREAD_WAIT: Duration = Duration::from_millis(500);

/// The real macOS platform: events, clipboard, keyboard layout and the Keychain secret stores;
/// `Unsupported` placeholders for the host registrar (stage 3) and device unlock and launch at
/// login (stage 5).
pub(crate) fn platform(paths: &AppPaths) -> Platform {
    // The shared store trusts the running app and the installed host. Without the app's own path
    // the list could not name the creator, so it falls back to the app-only access (the host then
    // cannot read, but nothing is exposed).
    let shared = match std::env::current_exe() {
        Ok(app_exe) => MacKeychainSecretStore::shared_with(vec![app_exe, paths.host_exe.clone()]),
        Err(_) => MacKeychainSecretStore::app_only(),
    };
    Platform {
        events: Arc::new(MacSystemEvents),
        clipboard: Arc::new(MacClipboard::general()),
        secrets: Arc::new(MacKeychainSecretStore::app_only()),
        shared_secrets: Arc::new(shared),
        device_unlock: Arc::new(UnsupportedDeviceUnlock),
        host_registrar: Arc::new(UnsupportedHostRegistrar),
        launch_at_login: Arc::new(UnsupportedLaunchAtLogin),
        keyboard_layout: Arc::new(MacKeyboardLayout),
    }
}

// ---------------------------------------------------------------------------------------------
// Notification observers
// ---------------------------------------------------------------------------------------------

/// A notification center, re-fetched on every use instead of storing a non-`Send` object.
#[derive(Debug, Clone, Copy)]
enum Center {
    /// `NSWorkspace.sharedWorkspace.notificationCenter` (sleep and wake).
    Workspace,
    /// `NSDistributedNotificationCenter.defaultCenter` (screen lock, input source).
    Distributed,
}

impl Center {
    fn get(self) -> Retained<NSNotificationCenter> {
        match self {
            Center::Workspace => NSWorkspace::sharedWorkspace().notificationCenter(),
            Center::Distributed => {
                Retained::into_super(NSDistributedNotificationCenter::defaultCenter())
            }
        }
    }
}

/// One registered observer; dropping it removes the observer from its center.
///
/// The center keeps the observer token, the token keeps the block and the block keeps the
/// handler (and so the sink). Removal makes the center drop its reference and dropping this value
/// drops ours, so after cancellation nothing keeps the sink alive: there is no cycle.
struct ObserverToken {
    center: Center,
    token: Retained<ProtocolObject<dyn NSObjectProtocol>>,
}

// SAFETY: the token is the opaque object `addObserverForName:object:queue:usingBlock:` returns for
// the sole purpose of being handed back to `removeObserver:`. We never call any other method on
// it; `NSNotificationCenter` is documented thread-safe, so adding on one thread and removing on
// another is allowed, and retain / release are atomic, so dropping the `Retained` on any thread is
// sound. The block it keeps alive captures only `Send + Sync` state (see `add_observer`).
unsafe impl Send for ObserverToken {}

impl Drop for ObserverToken {
    fn drop(&mut self) {
        let observer: &AnyObject = self.token.as_ref();
        autoreleasepool(|_| {
            // SAFETY: `observer` is exactly the token this center returned from
            // `addObserverForName:object:queue:usingBlock:`, the object type `removeObserver:`
            // expects; removing an already removed observer is a documented no-op.
            unsafe { self.center.get().removeObserver(observer) }
        });
    }
}

/// Runs `sink` and swallows a panic instead of letting it unwind into Foundation.
///
/// Unwinding through an Objective-C block into `NSNotificationCenter` / `NSOperationQueue` frames
/// is undefined behaviour. A lost notification is better than undefined state inside system code,
/// and library code must not print, so the panic is dropped silently: the subscriber is
/// responsible for not panicking.
fn call_sink_guarded(sink: &dyn Fn()) {
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(sink));
}

/// Runs `handler` for every `name` notification on `center` until the token drops. The handler
/// runs synchronously on the posting thread (`nil` queue; see the module docs).
fn add_observer(
    center: Center,
    name: &str,
    handler: impl Fn() + Send + Sync + 'static,
) -> ObserverToken {
    let block = RcBlock::new(move |_: NonNull<NSNotification>| call_sink_guarded(&handler));
    let name = NSString::from_str(name);
    autoreleasepool(|_| {
        // SAFETY: `object` is `None`, so its type cannot be wrong; the `nil` queue has no
        // threading requirement (the block runs on the posting thread); the block is sendable
        // because it captures only `handler`, which is `Send + Sync`, and the center copies it
        // before this call returns.
        let token = unsafe {
            center
                .get()
                .addObserverForName_object_queue_usingBlock(Some(&name), None, None, &block)
        };
        ObserverToken { center, token }
    })
}

/// [`SystemEvents`] from `NSWorkspace` sleep / wake and the distributed screen-lock notifications.
#[derive(Debug, Default)]
pub(crate) struct MacSystemEvents;

impl SystemEvents for MacSystemEvents {
    fn subscribe(
        &self,
        sink: Arc<dyn Fn(SystemEvent) + Send + Sync>,
    ) -> PlatformResult<Subscription> {
        let observers: Vec<ObserverToken> = [
            (Center::Workspace, WILL_SLEEP, SystemEvent::Sleep),
            (Center::Workspace, DID_WAKE, SystemEvent::Wake),
            (
                Center::Distributed,
                SCREEN_LOCKED,
                SystemEvent::ScreenLocked,
            ),
            (
                Center::Distributed,
                SCREEN_UNLOCKED,
                SystemEvent::ScreenUnlocked,
            ),
        ]
        .into_iter()
        .map(|(center, name, event)| {
            let sink = Arc::clone(&sink);
            add_observer(center, name, move || sink(event))
        })
        .collect();
        Ok(Subscription::new(move || drop(observers)))
    }
}

// ---------------------------------------------------------------------------------------------
// Clipboard
// ---------------------------------------------------------------------------------------------

/// [`SecureClipboard`] over a named `NSPasteboard`: every write carries the nspasteboard.org
/// concealed marker, and the stamp is the pasteboard's `changeCount`.
///
/// Only the pasteboard *name* is stored (a `String` is `Send + Sync`, `NSPasteboard` is not); the
/// pasteboard is looked up by name in each call. Production uses the general pasteboard; tests use
/// a uniquely named one so the developer's clipboard is never touched.
#[derive(Debug, Clone)]
pub(crate) struct MacClipboard {
    name: String,
}

impl MacClipboard {
    /// The general (system) pasteboard.
    pub(crate) fn general() -> Self {
        Self::over(&NSPasteboard::generalPasteboard())
    }

    /// The pasteboard `pasteboard` (looked up again by its name in every call).
    pub(crate) fn over(pasteboard: &NSPasteboard) -> Self {
        Self {
            name: pasteboard.name().to_string(),
        }
    }

    fn pasteboard(&self) -> Retained<NSPasteboard> {
        NSPasteboard::pasteboardWithName(&NSString::from_str(&self.name))
    }
}

fn stamp_of(change_count: isize) -> PlatformResult<ClipboardStamp> {
    u64::try_from(change_count)
        .map(ClipboardStamp)
        .map_err(|_| PlatformError::Failed("negative pasteboard change count".to_owned()))
}

impl SecureClipboard for MacClipboard {
    /// Starts a new pasteboard generation with `CurrentHostOnly`, which stops Handoff / Universal
    /// Clipboard from copying the password to the user's other devices. The concealed marker and
    /// the string land in that one generation; the stamp is the change count after all writes.
    fn write_secret(&self, text: &str) -> PlatformResult<ClipboardStamp> {
        autoreleasepool(|_| {
            let pasteboard = self.pasteboard();
            pasteboard
                .prepareForNewContentsWithOptions(NSPasteboardContentsOptions::CurrentHostOnly);
            // The concealed marker goes first, so a clipboard manager that polls between the two
            // writes never sees the text without it.
            let concealed = pasteboard
                .setData_forType(Some(&NSData::new()), &NSString::from_str(CONCEALED_TYPE));
            let written = pasteboard
                .setString_forType(&NSString::from_str(text), &NSString::from_str(STRING_TYPE));
            if !(concealed && written) {
                return Err(PlatformError::Failed(
                    "the pasteboard rejected the write".to_owned(),
                ));
            }
            stamp_of(pasteboard.changeCount())
        })
    }

    /// The OS has no atomic compare-and-clear: a copy by another app between the change-count
    /// check and `clearContents` can be wiped. The window is a few microseconds and is accepted.
    fn clear_if_unchanged(&self, stamp: &ClipboardStamp) -> PlatformResult<bool> {
        autoreleasepool(|_| {
            let pasteboard = self.pasteboard();
            if stamp_of(pasteboard.changeCount())? != *stamp {
                return Ok(false);
            }
            pasteboard.clearContents();
            Ok(true)
        })
    }
}

// ---------------------------------------------------------------------------------------------
// Keyboard layout (Carbon Text Input Sources)
// ---------------------------------------------------------------------------------------------

#[link(name = "Carbon", kind = "framework")]
extern "C" {
    /// Copy rule: returns a +1 `TISInputSourceRef` (or NULL) that the caller must `CFRelease`.
    fn TISCopyCurrentKeyboardInputSource() -> *mut c_void;
    /// Get rule: returns a borrowed `CFTypeRef` (or NULL) owned by `source`; never released.
    fn TISGetInputSourceProperty(source: *mut c_void, key: *const c_void) -> *mut c_void;
    /// `CFStringRef` key whose value is a `CFArrayRef` of `CFStringRef` language codes.
    static kTISPropertyInputSourceLanguages: *const c_void;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFRelease(cf: *const c_void);
}

/// The first language of the current keyboard input source (`"en"`, `"uk"`, `"zh-Hans"`).
///
/// Main thread only (the marker proves it): since macOS 14 the Text Input Source functions assert
/// that they run on the main queue and crash the process otherwise.
fn input_source_language(_main: MainThreadMarker) -> Option<String> {
    // SAFETY: no arguments; called on the main thread (the marker). The result is a +1 reference
    // under the Copy rule, or NULL, which is checked before use.
    let source = unsafe { TISCopyCurrentKeyboardInputSource() };
    if source.is_null() {
        return None;
    }
    // SAFETY: `source` is a live input source we own until the `CFRelease` below; the key is the
    // HIToolbox constant, an immutable `CFStringRef` valid for the life of the process. The value
    // follows the Get rule (borrowed from `source`, may be NULL) and, for this key, is a
    // `CFArrayRef` of `CFStringRef`, toll-free bridged to `NSArray<NSString>`; `Retained::retain`
    // takes our own +1 reference (NULL gives `None`), so the array outlives `source`.
    let languages: Option<Retained<NSArray<NSString>>> = unsafe {
        let value = TISGetInputSourceProperty(source, kTISPropertyInputSourceLanguages);
        Retained::retain(value.cast::<NSArray<NSString>>())
    };
    // SAFETY: `source` came from a Copy-rule function, so we own exactly one reference; it is
    // released exactly once, here, and not used afterwards.
    unsafe { CFRelease(source) };
    languages?
        .firstObject()
        .map(|language| language.to_string())
}

/// `"zh-Hans"` -> `"ZH"`: the primary subtag upper-cased, when it is 2 or 3 ASCII letters.
pub(crate) fn label_from_language(language: &str) -> Option<String> {
    let primary = language.split(['-', '_']).next().unwrap_or_default();
    let valid =
        (2..=3).contains(&primary.len()) && primary.chars().all(|c| c.is_ascii_alphabetic());
    valid.then(|| primary.to_ascii_uppercase())
}

fn label_on_main(main: MainThreadMarker) -> Option<String> {
    input_source_language(main).and_then(|language| label_from_language(&language))
}

/// The current label from any thread. Off the main thread the read is queued on the main
/// operation queue and awaited for at most [`MAIN_THREAD_WAIT`]; a busy or absent main run loop
/// (a unit-test thread, for one) gives `None` ("unknown") instead of a deadlock.
fn current_label() -> Option<String> {
    if let Some(main) = MainThreadMarker::new() {
        return label_on_main(main);
    }
    let (tx, rx) = mpsc::channel();
    let block = RcBlock::new(move || {
        // A late answer after the waiter gave up is simply dropped.
        call_sink_guarded(&|| {
            let _ = tx.send(MainThreadMarker::new().and_then(label_on_main));
        });
    });
    autoreleasepool(|_| {
        // SAFETY: the block captures only an `mpsc::Sender`, which is `Send`, so it is sendable to
        // the main queue as the method requires; the queue copies the block before returning.
        unsafe { NSOperationQueue::mainQueue().addOperationWithBlock(&block) }
    });
    rx.recv_timeout(MAIN_THREAD_WAIT).ok().flatten()
}

/// [`KeyboardLayout`] from the Carbon Text Input Source API (D2).
#[derive(Debug, Default)]
pub(crate) struct MacKeyboardLayout;

impl KeyboardLayout for MacKeyboardLayout {
    fn current(&self) -> PlatformResult<Option<String>> {
        Ok(current_label())
    }

    fn subscribe(
        &self,
        sink: Arc<dyn Fn(Option<String>) + Send + Sync>,
    ) -> PlatformResult<Subscription> {
        let observer = add_observer(Center::Distributed, INPUT_SOURCE_CHANGED, move || {
            sink(current_label())
        });
        Ok(Subscription::new(move || drop(observer)))
    }
}

// ---------------------------------------------------------------------------------------------
// Running applications
// ---------------------------------------------------------------------------------------------

/// Executable paths of every running process with `bundle_id`.
pub(crate) fn running_app_executables(bundle_id: &str) -> Vec<PathBuf> {
    autoreleasepool(|_| {
        NSRunningApplication::runningApplicationsWithBundleIdentifier(&NSString::from_str(
            bundle_id,
        ))
        .iter()
        .filter_map(|app| app.executableURL())
        .filter_map(|url| url.path())
        .map(|path| PathBuf::from(path.to_string()))
        .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use objc2_app_kit::NSPasteboard;
    use objc2_foundation::{NSDate, NSDistributedNotificationCenter, NSRunLoop, NSString};
    use std::sync::Mutex;

    /// A private, uniquely named pasteboard released (`releaseGlobally`) on drop, so the tests never
    /// touch the developer's real clipboard.
    struct ScratchPasteboard(Retained<NSPasteboard>);

    impl ScratchPasteboard {
        fn new() -> Self {
            Self(NSPasteboard::pasteboardWithUniqueName())
        }
    }

    impl Drop for ScratchPasteboard {
        fn drop(&mut self) {
            // SAFETY: `releaseGlobally` is an `NSPasteboard` instance method that takes no
            // arguments and returns `void`; `self.0` is a live `NSPasteboard`. The bindings do not
            // expose it, hence the raw message send. After this the pasteboard is not used again.
            unsafe {
                let _: () = objc2::msg_send![&*self.0, releaseGlobally];
            }
        }
    }

    fn has_type(pb: &NSPasteboard, ty: &str) -> bool {
        pb.types()
            .map(|types| types.iter().any(|t| t.to_string() == ty))
            .unwrap_or(false)
    }

    fn string_of(pb: &NSPasteboard) -> Option<String> {
        pb.stringForType(&NSString::from_str(STRING_TYPE))
            .map(|s| s.to_string())
    }

    #[test]
    fn notification_name_constants_are_exact() {
        assert_eq!(SCREEN_LOCKED, "com.apple.screenIsLocked");
        assert_eq!(SCREEN_UNLOCKED, "com.apple.screenIsUnlocked");
        assert_eq!(CONCEALED_TYPE, "org.nspasteboard.ConcealedType");
        assert_eq!(
            INPUT_SOURCE_CHANGED,
            "com.apple.Carbon.TISNotifySelectedKeyboardInputSourceChanged"
        );
    }

    #[test]
    fn sdk_constants_match_the_string_literals_we_use() {
        // SAFETY: the three statics are immutable `NSString` constants exported by AppKit, valid
        // for the life of the process; reading them has no other precondition.
        let (sleep, wake, string) = unsafe {
            (
                objc2_app_kit::NSWorkspaceWillSleepNotification.to_string(),
                objc2_app_kit::NSWorkspaceDidWakeNotification.to_string(),
                objc2_app_kit::NSPasteboardTypeString.to_string(),
            )
        };
        assert_eq!(sleep, WILL_SLEEP);
        assert_eq!(wake, DID_WAKE);
        assert_eq!(string, STRING_TYPE);
    }

    #[test]
    fn clipboard_write_marks_concealed_and_clear_respects_change_count() {
        let scratch = ScratchPasteboard::new();
        let pb = &scratch.0;
        let clipboard = MacClipboard::over(pb);

        let stamp = clipboard.write_secret("mypass-test-secret").unwrap();
        assert!(has_type(pb, CONCEALED_TYPE));
        assert_eq!(string_of(pb).as_deref(), Some("mypass-test-secret"));
        assert_eq!(stamp_of(pb.changeCount()), Ok(stamp.clone()));
        assert_eq!(clipboard.clear_if_unchanged(&stamp), Ok(true));
        assert_eq!(string_of(pb), None);

        let stamp = clipboard.write_secret("mypass-test-secret").unwrap();
        // A foreign copy, as another app would do it.
        pb.clearContents();
        pb.setString_forType(
            &NSString::from_str("foreign"),
            &NSString::from_str(STRING_TYPE),
        );
        assert_eq!(clipboard.clear_if_unchanged(&stamp), Ok(false));
        assert_eq!(string_of(pb).as_deref(), Some("foreign"));
    }

    #[test]
    fn guarded_sink_call_swallows_panics_and_runs_later_calls() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        call_sink_guarded(&|| panic!("subscriber bug"));
        let count = AtomicUsize::new(0);
        call_sink_guarded(&|| {
            count.fetch_add(1, Ordering::SeqCst);
        });
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn mac_clipboard_debug_names_the_pasteboard_only() {
        let scratch = ScratchPasteboard::new();
        let clipboard = MacClipboard::over(&scratch.0);
        clipboard.write_secret("mypass-debug-secret").unwrap();
        assert!(!format!("{clipboard:?}").contains("mypass-debug-secret"));
    }

    #[test]
    fn language_labels_are_short_upper_case_primary_subtags() {
        assert_eq!(label_from_language("en"), Some("EN".to_owned()));
        assert_eq!(label_from_language("uk"), Some("UK".to_owned()));
        assert_eq!(label_from_language("zh-Hans"), Some("ZH".to_owned()));
        assert_eq!(label_from_language("haw"), Some("HAW".to_owned()));
        assert_eq!(label_from_language(""), None);
        assert_eq!(label_from_language("x"), None);
        assert_eq!(label_from_language("e1"), None);
        assert_eq!(label_from_language("toolong"), None);
    }

    #[test]
    fn mac_system_events_subscribe_and_cancel_without_a_run_loop() {
        // Registering and removing the four observers must work from any thread; no event is
        // posted here.
        let sink: Arc<dyn Fn(SystemEvent) + Send + Sync> = Arc::new(|_| {});
        let weak = Arc::downgrade(&sink);
        let sub = MacSystemEvents.subscribe(sink).unwrap();
        assert!(weak.upgrade().is_some());
        sub.cancel();
        // Cancelling released every observer block, so nothing keeps the sink alive.
        assert!(weak.upgrade().is_none());
    }

    #[test]
    #[ignore = "posts the system-wide com.apple.screenIsLocked distributed notification, which other running apps observe; run only on a dedicated machine"]
    fn distributed_screen_lock_notification_reaches_sink() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink_seen = Arc::clone(&seen);
        let _sub = MacSystemEvents
            .subscribe(Arc::new(move |ev| sink_seen.lock().unwrap().push(ev)))
            .unwrap();
        let center = NSDistributedNotificationCenter::defaultCenter();
        // SAFETY: no `userInfo` dictionary is passed, so its generic type cannot be wrong.
        unsafe {
            center.postNotificationName_object_userInfo_deliverImmediately(
                &NSString::from_str(SCREEN_LOCKED),
                None,
                None,
                true,
            );
        }
        NSRunLoop::currentRunLoop().runUntilDate(&NSDate::dateWithTimeIntervalSinceNow(1.0));
        assert_eq!(*seen.lock().unwrap(), vec![SystemEvent::ScreenLocked]);
    }

    #[test]
    fn keyboard_layout_off_main_without_a_run_loop_is_unknown_not_a_deadlock() {
        // libtest never runs a test on the main thread and nothing pumps the main queue here, so
        // the hop times out and reports "unknown".
        let started = std::time::Instant::now();
        assert_eq!(MacKeyboardLayout.current(), Ok(None));
        assert!(started.elapsed() < MAIN_THREAD_WAIT * 4);
    }

    #[test]
    #[ignore = "needs a main run loop and a keyboard"]
    fn keyboard_layout_current_is_short_uppercase_label() {
        // libtest runs tests on worker threads only, so the main-thread proof is forged here.
        // SAFETY: `MainThreadMarker` only stands for "the TIS calls run on the main queue"; off
        // the main thread the worst case is the TIS main-queue assertion aborting this test
        // process, never memory corruption. Ignored, so no automated run takes that risk.
        let main = unsafe { MainThreadMarker::new_unchecked() };
        let label = label_on_main(main).unwrap();
        assert!((2..=3).contains(&label.len()), "{label}");
        assert!(label.chars().all(|c| c.is_ascii_uppercase()), "{label}");
    }

    #[test]
    #[ignore = "only meaningful from a bundled build"]
    fn running_app_executables_finds_own_process_by_bundle_id() {
        let own = std::env::current_exe().unwrap();
        assert!(running_app_executables(crate::BUNDLE_ID).contains(&own));
    }

    #[test]
    #[ignore = "needs a logged-in GUI session with Finder running"]
    fn running_app_executables_finds_finder() {
        let finder = PathBuf::from("/System/Library/CoreServices/Finder.app/Contents/MacOS/Finder");
        assert!(running_app_executables("com.apple.finder").contains(&finder));
        assert!(running_app_executables("com.mypass.no-such-app").is_empty());
    }
}
