//! In-memory fakes of every platform trait, for `AppCore` tests and stages 3 to 6.
//!
//! Compiled for this crate's own tests and, for other crates, only behind the `test-util` feature
//! (the pattern `mypass-kdbx` uses for `FailingFileIo`); never enabled in a shipped build.

use crate::{
    Browser, BrowserRegistration, ClipboardStamp, DeviceUnlock, DeviceUnlockAvailability,
    DeviceUnlockMode, HostRegistrationReport, KeyboardLayout, LaunchAtLogin, LoginItemStatus,
    NativeHostRegistrar, Platform, PlatformError, PlatformResult, RegistrationState, SecretStore,
    SecureClipboard, Subscription, SystemEvent, SystemEvents,
};
use secrecy::{ExposeSecret, SecretString};
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

/// Locks a fake's state; a poisoned lock (a panicking test thread) still yields the data.
fn lock<T: ?Sized>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The sinks of one fake, shared with the subscriptions that remove them.
struct Sinks<T: ?Sized> {
    next_id: u64,
    sinks: Vec<(u64, Arc<T>)>,
}

type SharedSinks<T> = Arc<Mutex<Sinks<T>>>;

fn new_sinks<T: ?Sized>() -> SharedSinks<T> {
    Arc::new(Mutex::new(Sinks {
        next_id: 0,
        sinks: Vec::new(),
    }))
}

/// Registers `sink`; the subscription holds only a weak reference, so it never keeps the fake
/// alive, and cancelling it drops the fake's reference to the sink.
fn add_sink<T: ?Sized + Send + Sync + 'static>(
    shared: &SharedSinks<T>,
    sink: Arc<T>,
) -> Subscription {
    let id = {
        let mut s = lock(shared);
        s.next_id += 1;
        let id = s.next_id;
        s.sinks.push((id, sink));
        id
    };
    let weak = Arc::downgrade(shared);
    Subscription::new(move || {
        if let Some(shared) = weak.upgrade() {
            lock(&shared).sinks.retain(|(sid, _)| *sid != id);
        }
    })
}

/// The current sinks, copied out so they run without the lock held (a sink may subscribe).
fn snapshot<T: ?Sized>(shared: &SharedSinks<T>) -> Vec<Arc<T>> {
    lock(shared)
        .sinks
        .iter()
        .map(|(_, s)| Arc::clone(s))
        .collect()
}

type EventSink = dyn Fn(SystemEvent) + Send + Sync;
type LayoutSink = dyn Fn(Option<String>) + Send + Sync;

/// [`SystemEvents`] driven by [`FakeSystemEvents::emit`]; sinks run on the emitting thread.
pub struct FakeSystemEvents {
    sinks: SharedSinks<EventSink>,
}

impl FakeSystemEvents {
    pub fn new() -> Arc<Self> {
        Arc::new(Self { sinks: new_sinks() })
    }

    /// Delivers `ev` to every live subscriber, synchronously.
    pub fn emit(&self, ev: SystemEvent) {
        for sink in snapshot(&self.sinks) {
            sink(ev);
        }
    }

    pub fn subscriber_count(&self) -> usize {
        lock(&self.sinks).sinks.len()
    }
}

impl SystemEvents for FakeSystemEvents {
    fn subscribe(&self, sink: Arc<EventSink>) -> PlatformResult<Subscription> {
        Ok(add_sink(&self.sinks, sink))
    }
}

impl std::fmt::Debug for FakeSystemEvents {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FakeSystemEvents")
            .field("subscribers", &self.subscriber_count())
            .finish()
    }
}

#[derive(Default)]
struct ClipboardState {
    contents: Option<String>,
    /// Like the macOS `changeCount`: bumped by every write and every clear.
    change_count: u64,
}

/// [`SecureClipboard`] in memory, with a macOS-style change counter.
#[derive(Default)]
pub struct FakeClipboard {
    state: Mutex<ClipboardState>,
}

impl FakeClipboard {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// What a paste would produce now.
    pub fn contents(&self) -> Option<String> {
        lock(&self.state).contents.clone()
    }

    /// A copy by the user or another app: replaces the contents and bumps the stamp.
    pub fn user_copies(&self, text: &str) {
        let mut s = lock(&self.state);
        s.change_count += 1;
        s.contents = Some(text.to_owned());
    }
}

impl SecureClipboard for FakeClipboard {
    fn write_secret(&self, text: &str) -> PlatformResult<ClipboardStamp> {
        let mut s = lock(&self.state);
        s.change_count += 1;
        s.contents = Some(text.to_owned());
        Ok(ClipboardStamp(s.change_count))
    }

    fn clear_if_unchanged(&self, stamp: &ClipboardStamp) -> PlatformResult<bool> {
        let mut s = lock(&self.state);
        if s.change_count != stamp.0 {
            return Ok(false);
        }
        s.change_count += 1;
        s.contents = None;
        Ok(true)
    }
}

impl std::fmt::Debug for FakeClipboard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = lock(&self.state);
        f.debug_struct("FakeClipboard")
            .field("contents", &s.contents.as_ref().map(|_| "<redacted>"))
            .field("change_count", &s.change_count)
            .finish()
    }
}

/// [`SecretStore`] in a `HashMap`.
#[derive(Default)]
pub struct InMemorySecretStore {
    items: Mutex<HashMap<(String, String), Vec<u8>>>,
}

impl InMemorySecretStore {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }
}

impl SecretStore for InMemorySecretStore {
    fn set(&self, service: &str, account: &str, secret: &[u8]) -> PlatformResult<()> {
        lock(&self.items).insert((service.to_owned(), account.to_owned()), secret.to_vec());
        Ok(())
    }

    fn get(&self, service: &str, account: &str) -> PlatformResult<Option<Vec<u8>>> {
        Ok(lock(&self.items)
            .get(&(service.to_owned(), account.to_owned()))
            .cloned())
    }

    fn delete(&self, service: &str, account: &str) -> PlatformResult<()> {
        lock(&self.items).remove(&(service.to_owned(), account.to_owned()));
        Ok(())
    }
}

impl std::fmt::Debug for InMemorySecretStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InMemorySecretStore")
            .field("items", &lock(&self.items).len())
            .finish()
    }
}

#[derive(Default)]
struct DeviceUnlockState {
    enrolled: HashMap<(DeviceUnlockModeKey, String), SecretString>,
    fail_next: Option<PlatformError>,
    evaluations: usize,
}

/// `DeviceUnlockMode` as a map key (the public enum does not derive `Hash`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum DeviceUnlockModeKey {
    Biometrics,
    AccountPassword,
}

impl From<DeviceUnlockMode> for DeviceUnlockModeKey {
    fn from(mode: DeviceUnlockMode) -> Self {
        match mode {
            DeviceUnlockMode::Biometrics => Self::Biometrics,
            DeviceUnlockMode::AccountPassword => Self::AccountPassword,
        }
    }
}

/// [`DeviceUnlock`] that stores the secret per `(mode, vault_id)`. `unlock` of a non-enrolled pair
/// returns `Err(Invalidated)`; with `Unavailable` every call but `availability` fails with
/// `DeviceUnlockUnavailable`.
pub struct FakeDeviceUnlock {
    availability: DeviceUnlockAvailability,
    state: Mutex<DeviceUnlockState>,
}

impl FakeDeviceUnlock {
    pub fn new(availability: DeviceUnlockAvailability) -> Arc<Self> {
        Arc::new(Self {
            availability,
            state: Mutex::new(DeviceUnlockState::default()),
        })
    }

    /// The next `unlock` fails with `e` (and still counts as an evaluation).
    pub fn fail_next_unlock(&self, e: PlatformError) {
        lock(&self.state).fail_next = Some(e);
    }

    /// Drops every `Biometrics` enrolment, like a changed fingerprint set.
    pub fn invalidate_biometrics(&self) {
        lock(&self.state)
            .enrolled
            .retain(|(mode, _), _| *mode != DeviceUnlockModeKey::Biometrics);
    }

    /// Number of `unlock` calls, so tests prove that no prompt happened.
    pub fn evaluation_count(&self) -> usize {
        lock(&self.state).evaluations
    }

    fn check_available(&self) -> PlatformResult<()> {
        match self.availability {
            DeviceUnlockAvailability::Unavailable => Err(PlatformError::DeviceUnlockUnavailable),
            DeviceUnlockAvailability::Available { .. } => Ok(()),
        }
    }
}

impl DeviceUnlock for FakeDeviceUnlock {
    fn availability(&self) -> DeviceUnlockAvailability {
        self.availability
    }

    fn is_enrolled(&self, mode: DeviceUnlockMode, vault_id: &str) -> PlatformResult<bool> {
        self.check_available()?;
        Ok(lock(&self.state)
            .enrolled
            .contains_key(&(mode.into(), vault_id.to_owned())))
    }

    fn enroll(
        &self,
        mode: DeviceUnlockMode,
        vault_id: &str,
        secret: &SecretString,
    ) -> PlatformResult<()> {
        self.check_available()?;
        let copy = SecretString::from(secret.expose_secret().to_owned());
        lock(&self.state)
            .enrolled
            .insert((mode.into(), vault_id.to_owned()), copy);
        Ok(())
    }

    fn unlock(
        &self,
        mode: DeviceUnlockMode,
        vault_id: &str,
        _reason: &str,
    ) -> PlatformResult<SecretString> {
        self.check_available()?;
        let mut s = lock(&self.state);
        s.evaluations += 1;
        if let Some(e) = s.fail_next.take() {
            return Err(e);
        }
        s.enrolled
            .get(&(mode.into(), vault_id.to_owned()))
            .map(|secret| SecretString::from(secret.expose_secret().to_owned()))
            .ok_or(PlatformError::Invalidated)
    }

    fn forget(&self, vault_id: &str) -> PlatformResult<()> {
        self.check_available()?;
        lock(&self.state).enrolled.retain(|(_, v), _| v != vault_id);
        Ok(())
    }
}

impl std::fmt::Debug for FakeDeviceUnlock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = lock(&self.state);
        f.debug_struct("FakeDeviceUnlock")
            .field("availability", &self.availability)
            .field("enrolled", &s.enrolled.len())
            .field("evaluations", &s.evaluations)
            .finish()
    }
}

struct RegistrarState {
    states: HashMap<Browser, RegistrationState>,
    register_calls: usize,
    unregister_calls: usize,
    fail_next: Option<PlatformError>,
}

/// [`NativeHostRegistrar`] that records calls. `register` marks every `always_register` browser
/// `Ok` and reports the others as skipped (not installed); `status()` reports the configured states
/// (`Missing` for every browser until `register`).
pub struct FakeHostRegistrar {
    paths: crate::AppPaths,
    state: Mutex<RegistrarState>,
}

impl FakeHostRegistrar {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            paths: crate::AppPaths::macos(Path::new("/fake-home")),
            state: Mutex::new(RegistrarState {
                states: HashMap::new(),
                register_calls: 0,
                unregister_calls: 0,
                fail_next: None,
            }),
        })
    }

    pub fn set_state(&self, browser: Browser, state: RegistrationState) {
        lock(&self.state).states.insert(browser, state);
    }

    pub fn register_calls(&self) -> usize {
        lock(&self.state).register_calls
    }

    pub fn unregister_calls(&self) -> usize {
        lock(&self.state).unregister_calls
    }

    /// The next `register` or `unregister` fails with `e`.
    pub fn fail_next(&self, e: PlatformError) {
        lock(&self.state).fail_next = Some(e);
    }

    fn row(&self, browser: Browser, state: RegistrationState) -> BrowserRegistration {
        BrowserRegistration {
            id: browser.id().to_owned(),
            display: browser.display_name().to_owned(),
            state,
            manifest_path: (state != RegistrationState::Missing)
                .then(|| self.paths.manifest_path(browser)),
            detail: None,
        }
    }
}

impl NativeHostRegistrar for FakeHostRegistrar {
    fn register(&self, _bundled_host: &Path) -> PlatformResult<HostRegistrationReport> {
        let mut s = lock(&self.state);
        s.register_calls += 1;
        if let Some(e) = s.fail_next.take() {
            return Err(e);
        }
        let mut report = HostRegistrationReport {
            host_copied: true,
            ..HostRegistrationReport::default()
        };
        for browser in Browser::ALL {
            if browser.always_register() {
                s.states.insert(browser, RegistrationState::Ok);
                report
                    .registrations
                    .push(self.row(browser, RegistrationState::Ok));
            } else {
                report.skipped.push(browser.id().to_owned());
            }
        }
        Ok(report)
    }

    fn status(&self) -> Vec<BrowserRegistration> {
        let s = lock(&self.state);
        Browser::ALL
            .into_iter()
            .map(|b| {
                let state = s
                    .states
                    .get(&b)
                    .copied()
                    .unwrap_or(RegistrationState::Missing);
                self.row(b, state)
            })
            .collect()
    }

    fn unregister(&self) -> PlatformResult<HostRegistrationReport> {
        let mut s = lock(&self.state);
        s.unregister_calls += 1;
        if let Some(e) = s.fail_next.take() {
            return Err(e);
        }
        s.states.clear();
        Ok(HostRegistrationReport {
            host_copied: true,
            registrations: Browser::ALL
                .into_iter()
                .map(|b| self.row(b, RegistrationState::Missing))
                .collect(),
            skipped: Vec::new(),
        })
    }
}

impl std::fmt::Debug for FakeHostRegistrar {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FakeHostRegistrar").finish_non_exhaustive()
    }
}

struct LaunchState {
    status: LoginItemStatus,
    set_calls: usize,
    fail_next: Option<PlatformError>,
}

/// [`LaunchAtLogin`] whose status follows `set_enabled` unless overridden by `set_status`.
pub struct FakeLaunchAtLogin {
    state: Mutex<LaunchState>,
}

impl FakeLaunchAtLogin {
    pub fn new(initial: bool) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(LaunchState {
                status: if initial {
                    LoginItemStatus::Enabled
                } else {
                    LoginItemStatus::Disabled
                },
                set_calls: 0,
                fail_next: None,
            }),
        })
    }

    pub fn set_status(&self, s: LoginItemStatus) {
        lock(&self.state).status = s;
    }

    pub fn set_calls(&self) -> usize {
        lock(&self.state).set_calls
    }

    /// The next `set_enabled` fails with `e` and changes nothing.
    pub fn fail_next(&self, e: PlatformError) {
        lock(&self.state).fail_next = Some(e);
    }
}

impl LaunchAtLogin for FakeLaunchAtLogin {
    fn status(&self) -> PlatformResult<LoginItemStatus> {
        Ok(lock(&self.state).status)
    }

    fn set_enabled(&self, enabled: bool) -> PlatformResult<()> {
        let mut s = lock(&self.state);
        s.set_calls += 1;
        if let Some(e) = s.fail_next.take() {
            return Err(e);
        }
        s.status = if enabled {
            LoginItemStatus::Enabled
        } else {
            LoginItemStatus::Disabled
        };
        Ok(())
    }
}

impl std::fmt::Debug for FakeLaunchAtLogin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = lock(&self.state);
        f.debug_struct("FakeLaunchAtLogin")
            .field("status", &s.status)
            .field("set_calls", &s.set_calls)
            .finish()
    }
}

/// [`KeyboardLayout`] whose label is changed by [`FakeKeyboardLayout::set`].
pub struct FakeKeyboardLayout {
    label: Mutex<Option<String>>,
    sinks: SharedSinks<LayoutSink>,
}

impl FakeKeyboardLayout {
    pub fn new(initial: Option<&str>) -> Arc<Self> {
        Arc::new(Self {
            label: Mutex::new(initial.map(str::to_owned)),
            sinks: new_sinks(),
        })
    }

    /// Changes the label and notifies every subscriber, synchronously.
    pub fn set(&self, label: Option<&str>) {
        let label = label.map(str::to_owned);
        *lock(&self.label) = label.clone();
        for sink in snapshot(&self.sinks) {
            sink(label.clone());
        }
    }
}

impl KeyboardLayout for FakeKeyboardLayout {
    fn current(&self) -> PlatformResult<Option<String>> {
        Ok(lock(&self.label).clone())
    }

    fn subscribe(&self, sink: Arc<LayoutSink>) -> PlatformResult<Subscription> {
        Ok(add_sink(&self.sinks, sink))
    }
}

impl std::fmt::Debug for FakeKeyboardLayout {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FakeKeyboardLayout")
            .field("label", &*lock(&self.label))
            .finish()
    }
}

/// A [`Platform`] made of fakes, plus typed handles to drive them.
#[derive(Debug)]
pub struct FakePlatform {
    pub platform: Platform,
    pub events: Arc<FakeSystemEvents>,
    pub clipboard: Arc<FakeClipboard>,
    pub secrets: Arc<InMemorySecretStore>,
    pub shared_secrets: Arc<InMemorySecretStore>,
    pub device_unlock: Arc<FakeDeviceUnlock>,
    pub host_registrar: Arc<FakeHostRegistrar>,
    pub launch_at_login: Arc<FakeLaunchAtLogin>,
    pub keyboard_layout: Arc<FakeKeyboardLayout>,
}

/// What `AppCore` tests and stages 3 to 6 build on: device unlock available with biometrics, launch
/// at login off, keyboard layout `EN`.
pub fn fake_platform() -> FakePlatform {
    let events = FakeSystemEvents::new();
    let clipboard = FakeClipboard::new();
    let secrets = InMemorySecretStore::new();
    let shared_secrets = InMemorySecretStore::new();
    let device_unlock =
        FakeDeviceUnlock::new(DeviceUnlockAvailability::Available { biometrics: true });
    let host_registrar = FakeHostRegistrar::new();
    let launch_at_login = FakeLaunchAtLogin::new(false);
    let keyboard_layout = FakeKeyboardLayout::new(Some("EN"));
    let platform = Platform {
        events: events.clone(),
        clipboard: clipboard.clone(),
        secrets: secrets.clone(),
        shared_secrets: shared_secrets.clone(),
        device_unlock: device_unlock.clone(),
        host_registrar: host_registrar.clone(),
        launch_at_login: launch_at_login.clone(),
        keyboard_layout: keyboard_layout.clone(),
    };
    FakePlatform {
        platform,
        events,
        clipboard,
        secrets,
        shared_secrets,
        device_unlock,
        host_registrar,
        launch_at_login,
        keyboard_layout,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type Seen<T> = Arc<Mutex<Vec<T>>>;
    type Sink<T> = Arc<dyn Fn(T) + Send + Sync>;

    fn recorder<T: Send + 'static>() -> (Seen<T>, Sink<T>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink_seen = Arc::clone(&seen);
        (
            seen,
            Arc::new(move |v: T| sink_seen.lock().unwrap().push(v)),
        )
    }

    #[test]
    fn fake_events_deliver_to_subscribers_until_cancelled() {
        let events = FakeSystemEvents::new();
        let (seen, sink) = recorder::<SystemEvent>();
        let sub = events.subscribe(sink).unwrap();
        assert_eq!(events.subscriber_count(), 1);
        events.emit(SystemEvent::Sleep);
        assert_eq!(*seen.lock().unwrap(), vec![SystemEvent::Sleep]);
        sub.cancel();
        events.emit(SystemEvent::ScreenLocked);
        assert_eq!(*seen.lock().unwrap(), vec![SystemEvent::Sleep]);
        assert_eq!(events.subscriber_count(), 0);
    }

    #[test]
    fn fake_events_dropping_the_subscription_unsubscribes() {
        let events = FakeSystemEvents::new();
        let (seen, sink) = recorder::<SystemEvent>();
        drop(events.subscribe(sink).unwrap());
        events.emit(SystemEvent::Wake);
        assert!(seen.lock().unwrap().is_empty());
        assert_eq!(events.subscriber_count(), 0);
    }

    #[test]
    fn fake_clipboard_clear_if_unchanged_true_when_stamp_matches() {
        let clipboard = FakeClipboard::new();
        let stamp = clipboard.write_secret("pw").unwrap();
        assert_eq!(clipboard.contents().as_deref(), Some("pw"));
        assert_eq!(clipboard.clear_if_unchanged(&stamp), Ok(true));
        assert_eq!(clipboard.contents(), None);
    }

    #[test]
    fn fake_clipboard_clear_skips_after_foreign_copy() {
        let clipboard = FakeClipboard::new();
        let stamp = clipboard.write_secret("pw").unwrap();
        clipboard.user_copies("other");
        assert_eq!(clipboard.clear_if_unchanged(&stamp), Ok(false));
        assert_eq!(clipboard.contents().as_deref(), Some("other"));
    }

    #[test]
    fn fake_clipboard_debug_redacts_contents() {
        let clipboard = FakeClipboard::new();
        clipboard.write_secret("hunter2").unwrap();
        assert!(!format!("{clipboard:?}").contains("hunter2"));
    }

    #[test]
    fn in_memory_secret_store_round_trip_and_delete() {
        let store = InMemorySecretStore::new();
        assert_eq!(store.get("svc", "acct"), Ok(None));
        store.set("svc", "acct", b"s3cret").unwrap();
        assert_eq!(store.get("svc", "acct"), Ok(Some(b"s3cret".to_vec())));
        assert_eq!(store.get("svc", "other"), Ok(None));
        store.set("svc", "acct", b"new").unwrap();
        assert_eq!(store.get("svc", "acct"), Ok(Some(b"new".to_vec())));
        store.delete("svc", "acct").unwrap();
        assert_eq!(store.get("svc", "acct"), Ok(None));
        store.delete("svc", "acct").unwrap();
        assert!(!format!("{store:?}").contains("new"));
    }

    fn secret(s: &str) -> SecretString {
        SecretString::from(s.to_owned())
    }

    #[test]
    fn fake_device_unlock_invalidate_biometrics_keeps_account_password_and_counts_evaluations() {
        let du = FakeDeviceUnlock::new(DeviceUnlockAvailability::Available { biometrics: true });
        du.enroll(DeviceUnlockMode::Biometrics, "v", &secret("bio"))
            .unwrap();
        du.enroll(DeviceUnlockMode::AccountPassword, "v", &secret("acct"))
            .unwrap();
        assert_eq!(du.evaluation_count(), 0);
        du.invalidate_biometrics();
        assert_eq!(du.is_enrolled(DeviceUnlockMode::Biometrics, "v"), Ok(false));
        assert_eq!(
            du.unlock(DeviceUnlockMode::Biometrics, "v", "r")
                .map(|s| s.expose_secret().to_owned()),
            Err(PlatformError::Invalidated)
        );
        assert_eq!(du.evaluation_count(), 1);
        let pw = du
            .unlock(DeviceUnlockMode::AccountPassword, "v", "r")
            .unwrap();
        assert_eq!(pw.expose_secret(), "acct");
        assert_eq!(du.evaluation_count(), 2);
    }

    #[test]
    fn fake_device_unlock_fail_next_unlock_raises_once() {
        let du = FakeDeviceUnlock::new(DeviceUnlockAvailability::Available { biometrics: false });
        du.enroll(DeviceUnlockMode::AccountPassword, "v", &secret("pw"))
            .unwrap();
        du.fail_next_unlock(PlatformError::Cancelled);
        assert_eq!(
            du.unlock(DeviceUnlockMode::AccountPassword, "v", "r")
                .map(|_| ()),
            Err(PlatformError::Cancelled)
        );
        assert!(du
            .unlock(DeviceUnlockMode::AccountPassword, "v", "r")
            .is_ok());
        assert!(!format!("{du:?}").contains("pw\""));
    }

    #[test]
    fn platform_error_device_unlock_variants_are_distinct_and_display_text_has_no_secret() {
        let variants = [
            PlatformError::Cancelled,
            PlatformError::Lockout,
            PlatformError::Invalidated,
            PlatformError::NotEnrolledOnDevice,
            PlatformError::DeviceUnlockUnavailable,
            PlatformError::Unsupported("x"),
            PlatformError::Failed("os said no".to_owned()),
        ];
        for (i, a) in variants.iter().enumerate() {
            for (j, b) in variants.iter().enumerate() {
                assert_eq!(i == j, a == b);
            }
        }
        let texts: Vec<String> = variants.iter().map(ToString::to_string).collect();
        let distinct: std::collections::HashSet<&String> = texts.iter().collect();
        assert_eq!(distinct.len(), texts.len());
        let du = FakeDeviceUnlock::new(DeviceUnlockAvailability::Available { biometrics: true });
        du.enroll(DeviceUnlockMode::Biometrics, "v", &secret("TopSecret!"))
            .unwrap();
        du.fail_next_unlock(PlatformError::Lockout);
        let err = du
            .unlock(DeviceUnlockMode::Biometrics, "v", "r")
            .map(|_| ())
            .unwrap_err();
        assert!(!err.to_string().contains("TopSecret!"));
        assert!(!format!("{err:?}").contains("TopSecret!"));
    }

    #[test]
    fn fake_device_unlock_round_trip_per_mode_and_forget_clears_all() {
        let du = FakeDeviceUnlock::new(DeviceUnlockAvailability::Available { biometrics: true });
        du.enroll(DeviceUnlockMode::Biometrics, "v1", &secret("bio"))
            .unwrap();
        du.enroll(DeviceUnlockMode::AccountPassword, "v1", &secret("acct"))
            .unwrap();
        for mode in [
            DeviceUnlockMode::Biometrics,
            DeviceUnlockMode::AccountPassword,
        ] {
            assert_eq!(du.is_enrolled(mode, "v1"), Ok(true));
            assert_eq!(du.is_enrolled(mode, "v2"), Ok(false));
        }
        assert_eq!(
            du.unlock(DeviceUnlockMode::Biometrics, "v1", "r")
                .unwrap()
                .expose_secret(),
            "bio"
        );
        assert_eq!(
            du.unlock(DeviceUnlockMode::AccountPassword, "v1", "r")
                .unwrap()
                .expose_secret(),
            "acct"
        );
        du.forget("v1").unwrap();
        for mode in [
            DeviceUnlockMode::Biometrics,
            DeviceUnlockMode::AccountPassword,
        ] {
            assert_eq!(du.is_enrolled(mode, "v1"), Ok(false));
        }
        assert_eq!(
            du.availability(),
            DeviceUnlockAvailability::Available { biometrics: true }
        );
    }

    #[test]
    fn fake_launch_at_login_toggles_reports_status_and_can_fail() {
        let l = FakeLaunchAtLogin::new(false);
        assert_eq!(l.status(), Ok(LoginItemStatus::Disabled));
        assert_eq!(l.is_enabled(), Ok(false));
        l.set_enabled(true).unwrap();
        assert_eq!(l.status(), Ok(LoginItemStatus::Enabled));
        assert_eq!(l.is_enabled(), Ok(true));
        l.set_enabled(false).unwrap();
        assert_eq!(l.status(), Ok(LoginItemStatus::Disabled));
        assert_eq!(l.set_calls(), 2);
        l.set_status(LoginItemStatus::RequiresApproval);
        assert_eq!(l.is_enabled(), Ok(false));
        assert_eq!(l.status(), Ok(LoginItemStatus::RequiresApproval));
        l.fail_next(PlatformError::Failed("denied".to_owned()));
        assert_eq!(
            l.set_enabled(true),
            Err(PlatformError::Failed("denied".to_owned()))
        );
        assert_eq!(l.status(), Ok(LoginItemStatus::RequiresApproval));
        assert_eq!(l.set_calls(), 3);
        assert_eq!(FakeLaunchAtLogin::new(true).is_enabled(), Ok(true));
    }

    #[test]
    fn fake_keyboard_layout_notifies_subscribers_with_new_label() {
        let k = FakeKeyboardLayout::new(Some("EN"));
        assert_eq!(k.current(), Ok(Some("EN".to_owned())));
        let (seen, sink) = recorder::<Option<String>>();
        let sub = k.subscribe(sink).unwrap();
        k.set(Some("UK"));
        k.set(None);
        assert_eq!(k.current(), Ok(None));
        assert_eq!(*seen.lock().unwrap(), vec![Some("UK".to_owned()), None]);
        drop(sub);
        k.set(Some("EN"));
        assert_eq!(seen.lock().unwrap().len(), 2);
    }

    #[test]
    fn fake_host_registrar_register_status_unregister_and_counters() {
        let r = FakeHostRegistrar::new();
        let status = r.status();
        assert_eq!(status.len(), Browser::ALL.len());
        assert!(status.iter().all(|b| b.state == RegistrationState::Missing));
        let report = r.register(Path::new("/bundle/mypass-host")).unwrap();
        assert!(report.host_copied);
        let ok: Vec<&str> = report.registrations.iter().map(|b| b.id.as_str()).collect();
        assert_eq!(ok, vec!["chrome", "chrome_for_testing"]);
        assert!(report
            .registrations
            .iter()
            .all(|b| b.state == RegistrationState::Ok));
        assert_eq!(report.skipped.len(), Browser::ALL.len() - 2);
        assert!(report.skipped.contains(&"edge".to_owned()));
        let after = r.status();
        for b in &after {
            let expected = if b.id == "chrome" || b.id == "chrome_for_testing" {
                RegistrationState::Ok
            } else {
                RegistrationState::Missing
            };
            assert_eq!(b.state, expected, "{}", b.id);
        }
        r.set_state(Browser::Edge, RegistrationState::Stale);
        assert!(r
            .status()
            .iter()
            .any(|b| b.id == "edge" && b.state == RegistrationState::Stale));
        r.unregister().unwrap();
        assert!(r
            .status()
            .iter()
            .all(|b| b.state == RegistrationState::Missing));
        r.fail_next(PlatformError::Failed("disk full".to_owned()));
        assert_eq!(
            r.register(Path::new("/bundle/mypass-host")),
            Err(PlatformError::Failed("disk full".to_owned()))
        );
        assert_eq!(r.register_calls(), 2);
        assert_eq!(r.unregister_calls(), 1);
    }

    fn same<A: ?Sized, B: ?Sized>(a: &Arc<A>, b: &Arc<B>) -> bool {
        std::ptr::addr_eq(Arc::as_ptr(a), Arc::as_ptr(b))
    }

    #[test]
    fn fake_platform_wires_every_field() {
        let f = fake_platform();
        let p = &f.platform;
        assert!(same(&p.events, &f.events));
        assert!(same(&p.clipboard, &f.clipboard));
        assert!(same(&p.secrets, &f.secrets));
        assert!(same(&p.shared_secrets, &f.shared_secrets));
        assert!(!same(&p.secrets, &p.shared_secrets));
        assert!(same(&p.device_unlock, &f.device_unlock));
        assert!(same(&p.host_registrar, &f.host_registrar));
        assert!(same(&p.launch_at_login, &f.launch_at_login));
        assert!(same(&p.keyboard_layout, &f.keyboard_layout));
        let (seen, sink) = recorder::<SystemEvent>();
        let _sub = p.events.subscribe(sink).unwrap();
        f.events.emit(SystemEvent::ScreenLocked);
        assert_eq!(*seen.lock().unwrap(), vec![SystemEvent::ScreenLocked]);
    }
}
