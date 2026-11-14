//! Device unlock (Touch ID / the Mac account password) for the open vault.
//!
//! - [`UnlockWindowPolicy`] decides whether device unlock may be offered, from when it and the
//!   real master password were each last used (a port of the old app's policy).
//! - [`UnlockStateStore`] keeps those two timestamps per vault in a Rust-only file.
//! - [`PromptCoordinator`] keeps at most one system prompt in flight.
//! - [`DeviceUnlockService`] ties them to the session, the settings toggles and the platform's
//!   [`DeviceUnlock`]: status, enrolment, reconcile and the explicit device unlock. It is pure
//!   over the platform trait, so every rule is tested on the fake.
//!
//! Enrolment is per `(mode, vault_id)` (the platform contract). The toggles are global settings,
//! so opening a vault that is not enrolled turns them off (reconcile); the other vault's item
//! stays until it is forgotten or re-enrolled.

use crate::app_core::{ApiError, AppPaths, ErrorCode};
use crate::scheduler::{lock, Scheduler};
use crate::session::{SessionError, SessionEvent, SessionState, VaultSession};
use crate::settings::{
    RequireAccountPasswordWindow, Settings, SettingsStore, UnlockWithDeviceWindow,
};
use crate::unlock_request::UnlockRequestState;
use mypass_platform::{DeviceUnlock, DeviceUnlockAvailability, DeviceUnlockMode, PlatformError};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};
use std::thread::ThreadId;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Whether device unlock may be offered now. Pure; the timestamps come from the Rust-only unlock
/// state store, never from `Settings`, so the webview cannot extend a window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnlockWindowPolicy;

impl UnlockWindowPolicy {
    /// `true` when both events happened and each lies within its window (inclusive: exactly the
    /// window length is still allowed). A missing timestamp means "never happened" and always
    /// gives `false`, even for a `Never` window: an unlimited window only excuses the time passed
    /// since a real event, not the absence of one. A timestamp in the future (the clock went
    /// backwards) counts as elapsed 0.
    pub fn device_auth_allowed(
        now: SystemTime,
        last_device_unlock: Option<SystemTime>,
        last_full_password: Option<SystemTime>,
        device: UnlockWithDeviceWindow,
        account: RequireAccountPasswordWindow,
    ) -> bool {
        let (Some(device_at), Some(password_at)) = (last_device_unlock, last_full_password) else {
            return false;
        };
        within(now, device_at, device.seconds()) && within(now, password_at, account.seconds())
    }
}

/// `now - at <= window`; a `None` window never expires; `at` after `now` is elapsed 0.
fn within(now: SystemTime, at: SystemTime, window: Option<u64>) -> bool {
    let Some(window) = window else {
        return true;
    };
    let elapsed = now.duration_since(at).unwrap_or(Duration::ZERO);
    elapsed <= Duration::from_secs(window)
}

/// The device-unlock mode the two Settings toggles select: Touch ID only is
/// [`DeviceUnlockMode::Biometrics`]; the Mac password, alone or with Touch ID, is
/// [`DeviceUnlockMode::AccountPassword`] (its OS policy still offers Touch ID first); both off is
/// `None`.
pub fn mode_for_toggles(touch_id: bool, mac_password: bool) -> Option<DeviceUnlockMode> {
    match (touch_id, mac_password) {
        (false, false) => None,
        (true, false) => Some(DeviceUnlockMode::Biometrics),
        (_, true) => Some(DeviceUnlockMode::AccountPassword),
    }
}

/// Why device unlock cannot be offered right now.
#[derive(Serialize, specta::Type, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase")]
pub enum OfferBlock {
    /// Both toggles are off.
    Disabled,
    /// The toggles are on but this vault has no enrolment for their mode.
    NotEnrolled,
    /// The last device unlock is older than the device window.
    DeviceWindowExpired,
    /// The master password was last typed longer ago than the account window (or never).
    AccountWindowExpired,
}

/// What the Security pane and the unlock screen show about device unlock.
#[derive(Serialize, specta::Type, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct DeviceUnlockStatus {
    /// The device can unlock at all (`availability != Unavailable`).
    pub available: bool,
    /// Biometrics are set up on the device.
    pub biometrics: bool,
    /// The current vault is enrolled for the mode the toggles select.
    pub enrolled: bool,
    pub touch_id_enabled: bool,
    pub mac_password_enabled: bool,
    /// The unlock screen may show the device-unlock button now.
    pub can_offer_now: bool,
    /// What blocks the button: the toggles, the enrolment or a window. `None` when none of them
    /// does, and `can_offer_now` is then `available`.
    pub reason: Option<OfferBlock>,
}

/// When device unlock and the master password were last used for one vault (Unix seconds).
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct UnlockStamps {
    pub last_device_unlock: i64,
    pub last_full_password: i64,
}

/// Why the unlock state could not be saved (never fatal: a lost stamp only shortens a window).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum UnlockStateError {
    #[error("the unlock state could not be saved: {0}")]
    Io(String),
}

/// The Rust-only store of [`UnlockStamps`] per vault id: `<config_dir>/unlock-state.json`, written
/// atomically. It is never part of `Settings`, so no webview can extend a window. A missing,
/// unreadable, oversized or corrupt file reads as empty: device unlock is then not offered until
/// the next master-password unlock, which is the safe direction.
#[derive(Debug)]
pub struct UnlockStateStore {
    path: PathBuf,
    /// Serialises the read-modify-write of the file.
    write: Mutex<()>,
}

impl UnlockStateStore {
    /// The file name under `config_dir`.
    pub const FILE_NAME: &'static str = "unlock-state.json";
    /// A larger file is not read (it holds a few dozen bytes per vault).
    pub const MAX_FILE_BYTES: u64 = 1024 * 1024;

    /// The store at `<config_dir>/unlock-state.json`.
    pub fn new(config_dir: &Path) -> Self {
        Self {
            path: config_dir.join(Self::FILE_NAME),
            write: Mutex::new(()),
        }
    }

    /// The stamps of `vault_id`, if any.
    pub fn get(&self, vault_id: &str) -> Option<UnlockStamps> {
        self.read_all().get(vault_id).copied()
    }

    /// A master-password unlock, open or create (or a verified enrolment): sets BOTH stamps.
    pub fn record_password_unlock(
        &self,
        vault_id: &str,
        at: SystemTime,
    ) -> Result<(), UnlockStateError> {
        let at = unix_seconds(at);
        self.update(|all| {
            all.insert(
                vault_id.to_owned(),
                UnlockStamps {
                    last_device_unlock: at,
                    last_full_password: at,
                },
            );
            true
        })
    }

    /// A device unlock: sets the device stamp only. Without a password stamp nothing is
    /// recorded, so a device unlock can never stand in for a master password entry.
    pub fn record_device_unlock(
        &self,
        vault_id: &str,
        at: SystemTime,
    ) -> Result<(), UnlockStateError> {
        let at = unix_seconds(at);
        self.update(|all| match all.get_mut(vault_id) {
            Some(stamps) => {
                stamps.last_device_unlock = at;
                true
            }
            None => false,
        })
    }

    fn update(
        &self,
        change: impl FnOnce(&mut BTreeMap<String, UnlockStamps>) -> bool,
    ) -> Result<(), UnlockStateError> {
        let _guard = lock(&self.write);
        let mut all = self.read_all();
        if !change(&mut all) {
            return Ok(());
        }
        let io = |e: String| UnlockStateError::Io(e);
        let mut text = serde_json::to_vec_pretty(&all).map_err(|e| io(e.to_string()))?;
        text.push(b'\n');
        if let Some(parent) = self.path.parent().filter(|p| !p.as_os_str().is_empty()) {
            fs::create_dir_all(parent).map_err(|e| io(e.to_string()))?;
        }
        mypass_kdbx::write_atomic(&self.path, &text).map_err(|e| io(e.to_string()))
    }

    fn read_all(&self) -> BTreeMap<String, UnlockStamps> {
        let Ok(file) = fs::File::open(&self.path) else {
            return BTreeMap::new();
        };
        let mut bytes = Vec::new();
        let limit = Self::MAX_FILE_BYTES + 1;
        if file.take(limit).read_to_end(&mut bytes).is_err()
            || bytes.len() as u64 > Self::MAX_FILE_BYTES
        {
            return BTreeMap::new();
        }
        serde_json::from_slice(&bytes).unwrap_or_default()
    }
}

/// Seconds since the Unix epoch (negative before it), saturating.
fn unix_seconds(t: SystemTime) -> i64 {
    match t.duration_since(UNIX_EPOCH) {
        Ok(d) => i64::try_from(d.as_secs()).unwrap_or(i64::MAX),
        Err(e) => -i64::try_from(e.duration().as_secs()).unwrap_or(i64::MAX),
    }
}

/// The inverse of [`unix_seconds`], saturating at the representable range.
fn from_unix_seconds(secs: i64) -> SystemTime {
    let magnitude = Duration::from_secs(secs.unsigned_abs());
    let t = if secs >= 0 {
        UNIX_EPOCH.checked_add(magnitude)
    } else {
        UNIX_EPOCH.checked_sub(magnitude)
    };
    t.unwrap_or(UNIX_EPOCH)
}

/// An in-flight prompt older than this is treated as abandoned (the OS callback never came), so
/// it cannot wedge device unlock for the rest of the locked session. The 10 s freshness of an
/// extension unlock request is owned by [`UnlockRequestState`].
pub const PROMPT_STALE_AFTER: Duration = Duration::from_secs(60);

/// Keeps at most one device-unlock prompt in flight across every surface (unlock screen, tray
/// panel, extension request). A prompt is only ever explicit: nothing here starts one because a
/// view appeared, the vault locked or a window gained focus, and locking never notes an extension
/// request. Port of the old `DeviceUnlockPromptCoordinator` without its extension-request half,
/// which is the stage 3 [`UnlockRequestState`] (the bridge calls `note`; this coordinator clears
/// it when the vault unlocks by any means).
///
/// Staleness is measured on the scheduler's monotonic timeline, so a wall clock set back cannot
/// keep a prompt "in flight" for longer than [`PROMPT_STALE_AFTER`].
pub struct PromptCoordinator {
    scheduler: Arc<dyn Scheduler>,
    unlock_requests: UnlockRequestState,
    /// Monotonic start of the prompt in flight.
    started: Mutex<Option<Duration>>,
}

impl std::fmt::Debug for PromptCoordinator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PromptCoordinator")
            .field("started", &*lock(&self.started))
            .finish_non_exhaustive()
    }
}

impl PromptCoordinator {
    pub fn new(scheduler: Arc<dyn Scheduler>, unlock_requests: UnlockRequestState) -> Self {
        Self {
            scheduler,
            unlock_requests,
            started: Mutex::new(None),
        }
    }

    /// Claims the one prompt slot. `false` while another prompt is in flight (and younger than
    /// [`PROMPT_STALE_AFTER`]): the caller must not show a second system sheet.
    pub fn request_explicit_prompt(&self) -> bool {
        let now = self.scheduler.monotonic_now();
        let mut started = lock(&self.started);
        if let Some(at) = *started {
            if now.saturating_sub(at) < PROMPT_STALE_AFTER {
                return false;
            }
        }
        *started = Some(now);
        true
    }

    /// The prompt ended (success, failure or cancel).
    pub fn prompt_finished(&self) {
        *lock(&self.started) = None;
    }

    /// The vault locked: clears the in-flight state only. It never notes an extension request,
    /// so a lock alone can never cause a prompt.
    pub fn vault_did_lock(&self) {
        *lock(&self.started) = None;
    }

    /// The vault unlocked by any means: a pending extension request nobody consumed must not
    /// survive, or a lock within its freshness window would find it and prompt.
    pub fn vault_did_unlock(&self) {
        self.unlock_requests.clear();
    }
}

/// Releases the prompt slot when the device-unlock call returns, on every path.
struct PromptSlot<'a>(&'a PromptCoordinator);

impl Drop for PromptSlot<'_> {
    fn drop(&mut self) {
        self.0.prompt_finished();
    }
}

/// What the session observer last saw.
#[derive(Debug, Clone, Default, PartialEq)]
struct Observed {
    unlocked: bool,
    vault_id: Option<String>,
}

struct ServiceInner {
    session: VaultSession,
    settings: SettingsStore,
    platform: Arc<dyn DeviceUnlock>,
    scheduler: Arc<dyn Scheduler>,
    unlock_requests: UnlockRequestState,
    state: UnlockStateStore,
    prompts: PromptCoordinator,
    observed: Mutex<Observed>,
    /// The thread running a device unlock's `session.unlock`: the session observer runs on that
    /// thread, and must not count that unlock as a master-password entry.
    device_unlock_thread: Mutex<Option<ThreadId>>,
}

/// Device unlock for the open vault: status, enrolment, reconcile and the explicit prompt.
///
/// The session tells it about every unlock through an observer (so `session.rs` does not depend
/// on this module): an unlock by master password (unlock, open, create) records both timestamps,
/// an unlock by device records only the device one. Cheap to clone; clones share everything.
#[derive(Clone)]
pub struct DeviceUnlockService(Arc<ServiceInner>);

impl std::fmt::Debug for DeviceUnlockService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeviceUnlockService")
            .field("state", &self.0.state)
            .finish_non_exhaustive()
    }
}

/// Marks the current thread as the device-unlock thread until dropped.
struct DeviceThreadMark<'a>(&'a Mutex<Option<ThreadId>>);

impl<'a> DeviceThreadMark<'a> {
    fn set(slot: &'a Mutex<Option<ThreadId>>) -> Self {
        *lock(slot) = Some(std::thread::current().id());
        Self(slot)
    }
}

impl Drop for DeviceThreadMark<'_> {
    fn drop(&mut self) {
        *lock(self.0) = None;
    }
}

fn api(code: ErrorCode, message: impl Into<String>) -> ApiError {
    ApiError::new(code, message)
}

impl DeviceUnlockService {
    /// Builds the service and registers its session observer (which holds only a weak reference,
    /// so the session never keeps the service alive).
    pub fn new(
        session: VaultSession,
        settings: SettingsStore,
        platform: Arc<dyn DeviceUnlock>,
        scheduler: Arc<dyn Scheduler>,
        unlock_requests: UnlockRequestState,
        paths: &AppPaths,
    ) -> Self {
        let inner = Arc::new(ServiceInner {
            prompts: PromptCoordinator::new(scheduler.clone(), unlock_requests.clone()),
            state: UnlockStateStore::new(&paths.config_dir),
            observed: Mutex::new(Observed::default()),
            device_unlock_thread: Mutex::new(None),
            session: session.clone(),
            settings,
            platform,
            scheduler,
            unlock_requests,
        });
        let weak: Weak<ServiceInner> = Arc::downgrade(&inner);
        session.add_observer(Arc::new(move |event: &SessionEvent| {
            if let (SessionEvent::StateChanged(status), Some(inner)) = (event, weak.upgrade()) {
                DeviceUnlockService(inner).on_state(status.state);
            }
        }));
        Self(inner)
    }

    /// The prompt coordinator (the unlock screen asks it before pressing the button for an
    /// extension request).
    pub fn prompts(&self) -> &PromptCoordinator {
        &self.0.prompts
    }

    /// The Rust-only timestamp store.
    pub fn state_store(&self) -> &UnlockStateStore {
        &self.0.state
    }

    /// `true` at most once per fresh extension unlock request (stage 3 notes it); the unlock
    /// screen presses the device button only then, never on a plain appearance.
    pub fn consume_extension_request(&self) -> bool {
        self.0.unlock_requests.consume()
    }

    /// The status for the current vault; runs [`DeviceUnlockService::reconcile`] first.
    pub fn status(&self) -> DeviceUnlockStatus {
        self.reconcile();
        let s = self.0.settings.get();
        let availability = self.0.platform.availability();
        let available = availability != DeviceUnlockAvailability::Unavailable;
        let biometrics = availability == DeviceUnlockAvailability::Available { biometrics: true };
        let mode = mode_for_toggles(s.unlock_touch_id_enabled, s.unlock_mac_password_enabled);
        let vault_id = self.0.session.vault_id();
        let enrolled = match (mode, vault_id.as_deref()) {
            (Some(mode), Some(id)) => self.0.platform.is_enrolled(mode, id).unwrap_or(false),
            _ => false,
        };
        let reason = if mode.is_none() {
            Some(OfferBlock::Disabled)
        } else if !enrolled {
            Some(OfferBlock::NotEnrolled)
        } else {
            let stamps = vault_id.as_deref().and_then(|id| self.0.state.get(id));
            self.window_block(stamps, &s)
        };
        DeviceUnlockStatus {
            available,
            biometrics,
            enrolled,
            touch_id_enabled: s.unlock_touch_id_enabled,
            mac_password_enabled: s.unlock_mac_password_enabled,
            can_offer_now: available && reason.is_none(),
            reason,
        }
    }

    /// `None` when [`UnlockWindowPolicy`] allows device unlock now; otherwise which window
    /// blocks it (the account window first: then only the master password helps).
    fn window_block(&self, stamps: Option<UnlockStamps>, s: &Settings) -> Option<OfferBlock> {
        let now = self.0.scheduler.wall_now();
        let device_at = stamps.map(|st| from_unix_seconds(st.last_device_unlock));
        let password_at = stamps.map(|st| from_unix_seconds(st.last_full_password));
        let (device, account) = (
            s.unlock_with_device_window,
            s.require_account_password_window,
        );
        if UnlockWindowPolicy::device_auth_allowed(now, device_at, password_at, device, account) {
            return None;
        }
        let account_ok = password_at.is_some_and(|at| within(now, at, account.seconds()));
        Some(if account_ok {
            OfferBlock::DeviceWindowExpired
        } else {
            OfferBlock::AccountWindowExpired
        })
    }

    /// When the toggles are on but the vault has no enrolment for their mode (the item is gone,
    /// the biometrics changed, the Keychain was reset, a new Mac), turns both toggles off. Never
    /// prompts and never fails; an error from the platform leaves the toggles as they are.
    pub fn reconcile(&self) {
        let s = self.0.settings.get();
        let Some(mode) = mode_for_toggles(s.unlock_touch_id_enabled, s.unlock_mac_password_enabled)
        else {
            return;
        };
        let Some(id) = self.0.session.vault_id() else {
            return;
        };
        if self.0.platform.is_enrolled(mode, &id) == Ok(false) {
            let _ = self.0.settings.set_device_unlock_toggles(false, false);
        }
    }

    /// A master-password unlock, open or create of the current vault: sets both timestamps.
    /// Recorded only while device unlock is switched on (turning it on records them anyway).
    /// Best effort: a failed write only means device unlock is offered later.
    pub fn record_password_unlock(&self) {
        let s = self.0.settings.get();
        if mode_for_toggles(s.unlock_touch_id_enabled, s.unlock_mac_password_enabled).is_none() {
            return;
        }
        if let Some(id) = self.0.session.vault_id() {
            let _ = self
                .0
                .state
                .record_password_unlock(&id, self.0.scheduler.wall_now());
        }
    }

    /// Switches device unlock on, off or to another mode.
    ///
    /// - Both toggles off: the toggles go off and every enrolment of the vault is forgotten
    ///   (allowed while locked).
    /// - Turning on from off REQUIRES `password`, checked against the unlocked session
    ///   (`Locked` while locked, `WrongPassword` when it differs; nothing is stored and the
    ///   toggles stay as they were). Then the mode is enrolled, the toggles set and both
    ///   timestamps recorded (a verified password is a real password entry).
    /// - While on, any change needs the vault unlocked (`Locked` otherwise) but no password: a
    ///   mode change forgets the old enrolment and enrols the new mode from the session's
    ///   password (if that enrolment fails the toggles go off); newly enabling Touch ID in the
    ///   same mode re-enrols it, so the biometric state is captured again.
    pub fn configure(
        &self,
        password: Option<SecretString>,
        touch_id: bool,
        mac_password: bool,
    ) -> Result<DeviceUnlockStatus, ApiError> {
        self.reconcile();
        let current = self.0.settings.get();
        let current_mode = mode_for_toggles(
            current.unlock_touch_id_enabled,
            current.unlock_mac_password_enabled,
        );
        let Some(mode) = mode_for_toggles(touch_id, mac_password) else {
            let toggles = self.0.settings.set_device_unlock_toggles(false, false);
            if let Some(id) = self.0.session.vault_id() {
                self.0.platform.forget(&id)?;
            }
            toggles?;
            return Ok(self.status());
        };
        if self.0.platform.availability() == DeviceUnlockAvailability::Unavailable {
            return Err(PlatformError::DeviceUnlockUnavailable.into());
        }
        let id = self.0.session.vault_id().ok_or(SessionError::NoVault)?;
        // `Locked` / `NoVault` before anything is touched.
        self.0.session.with_db(|_| ())?;
        match current_mode {
            None => {
                let password = password.ok_or_else(|| {
                    api(ErrorCode::InvalidInput, "the master password is required")
                })?;
                self.0.session.verify_password(&password)?;
                self.0.platform.enroll(mode, &id, &password)?;
                self.0
                    .settings
                    .set_device_unlock_toggles(touch_id, mac_password)?;
                self.record_password_unlock();
            }
            Some(current_mode) => {
                let touch_id_newly_on = touch_id && !current.unlock_touch_id_enabled;
                if current_mode != mode || touch_id_newly_on {
                    let secret = self.0.session.with_db_and_password(|_, p| {
                        SecretString::from(p.expose_secret().to_owned())
                    })?;
                    if current_mode != mode {
                        self.0.platform.forget(&id)?;
                        if let Err(e) = self.0.platform.enroll(mode, &id, &secret) {
                            let _ = self.0.settings.set_device_unlock_toggles(false, false);
                            return Err(e.into());
                        }
                    } else {
                        self.0.platform.enroll(mode, &id, &secret)?;
                    }
                }
                self.0
                    .settings
                    .set_device_unlock_toggles(touch_id, mac_password)?;
            }
        }
        Ok(self.status())
    }

    /// Unlocks the locked vault with the device (an explicit prompt only: a button press or a
    /// consumed extension request). Runs the platform prompt on the calling thread, which blocks.
    ///
    /// Refused without a prompt when device unlock is unavailable (`DeviceUnlockUnavailable`),
    /// not offered now (`InvalidInput`, naming the [`OfferBlock`]) or another prompt is in flight
    /// (`Internal`, which the UI ignores). A cancel is `Cancelled` (the UI shows nothing) and
    /// keeps the enrolment; `Lockout` and `NotEnrolledOnDevice` pass through. An invalidated
    /// enrolment, or a stored password the vault no longer opens with (changed in another app),
    /// forgets the enrolment, turns the toggles off and reports `Invalidated`. Success records the
    /// device timestamp only. An unlocked vault is left alone, without a prompt.
    pub fn unlock_with_device(&self, reason: &str) -> Result<(), ApiError> {
        let id = self.0.session.vault_id().ok_or(SessionError::NoVault)?;
        if self.0.session.is_unlocked() {
            return Ok(());
        }
        let status = self.status();
        if !status.available {
            return Err(PlatformError::DeviceUnlockUnavailable.into());
        }
        let s = self.0.settings.get();
        let mode = match (
            status.can_offer_now,
            mode_for_toggles(s.unlock_touch_id_enabled, s.unlock_mac_password_enabled),
        ) {
            (true, Some(mode)) => mode,
            _ => {
                return Err(api(
                    ErrorCode::InvalidInput,
                    format!("device unlock is not offered now ({:?})", status.reason),
                ))
            }
        };
        if !self.0.prompts.request_explicit_prompt() {
            return Err(api(
                ErrorCode::Internal,
                "a device unlock prompt is already in flight",
            ));
        }
        let _slot = PromptSlot(&self.0.prompts);
        let secret = match self.0.platform.unlock(mode, &id, reason) {
            Ok(secret) => secret,
            Err(PlatformError::Invalidated) => {
                self.drop_enrolment(&id);
                return Err(PlatformError::Invalidated.into());
            }
            Err(e) => return Err(e.into()),
        };
        let unlocked = {
            let _mark = DeviceThreadMark::set(&self.0.device_unlock_thread);
            self.0.session.unlock(secret)
        };
        match unlocked {
            Ok(()) => {
                let _ = self
                    .0
                    .state
                    .record_device_unlock(&id, self.0.scheduler.wall_now());
                Ok(())
            }
            Err(SessionError::WrongPassword)
                if self.0.session.vault_id().as_deref() == Some(id.as_str()) =>
            {
                self.drop_enrolment(&id);
                Err(PlatformError::Invalidated.into())
            }
            Err(e) => Err(e.into()),
        }
    }

    /// Forgets the vault's enrolment and turns the toggles off (best effort).
    fn drop_enrolment(&self, vault_id: &str) {
        let _ = self.0.platform.forget(vault_id);
        let _ = self.0.settings.set_device_unlock_toggles(false, false);
    }

    /// The session observer: tells the coordinator about lock transitions and records a
    /// master-password unlock (any transition into `Unlocked`, or to another vault, that is not
    /// a device unlock running on this thread).
    fn on_state(&self, state: SessionState) {
        let unlocked = state == SessionState::Unlocked;
        let vault_id = if unlocked {
            self.0.session.vault_id()
        } else {
            None
        };
        let next = Observed { unlocked, vault_id };
        let previous = std::mem::replace(&mut *lock(&self.0.observed), next.clone());
        if next.unlocked && (!previous.unlocked || previous.vault_id != next.vault_id) {
            self.0.prompts.vault_did_unlock();
            let by_device =
                *lock(&self.0.device_unlock_thread) == Some(std::thread::current().id());
            if !by_device {
                self.record_password_unlock();
            }
        } else if !next.unlocked && previous.unlocked {
            self.0.prompts.vault_did_lock();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::UNIX_EPOCH;

    use RequireAccountPasswordWindow as A;
    use UnlockWithDeviceWindow as D;

    const HOUR: u64 = 3600;
    const DAY: u64 = 86_400;

    fn t(secs: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(1_700_000_000 + secs)
    }

    fn allowed(now: u64, device_at: u64, password_at: u64, d: D, a: A) -> bool {
        UnlockWindowPolicy::device_auth_allowed(
            t(now),
            Some(t(device_at)),
            Some(t(password_at)),
            d,
            a,
        )
    }

    #[test]
    fn device_window_boundary_is_inclusive() {
        assert!(allowed(8 * HOUR, 0, 0, D::EightHours, A::ThirtyDays));
        assert!(!allowed(8 * HOUR + 1, 0, 0, D::EightHours, A::ThirtyDays));
        // Exactly 30 days on the account window, with a fresh device unlock.
        assert!(allowed(30 * DAY, 30 * DAY, 0, D::EightHours, A::ThirtyDays));
        assert!(!allowed(
            30 * DAY + 1,
            30 * DAY,
            0,
            D::EightHours,
            A::ThirtyDays
        ));
    }

    #[test]
    fn account_window_expiry_blocks_even_inside_device_window() {
        // Device unlock a minute ago, master password 31 days ago.
        assert!(!allowed(
            31 * DAY,
            31 * DAY - 60,
            0,
            D::TwentyFourHours,
            A::ThirtyDays
        ));
        assert!(!allowed(2 * DAY, 2 * DAY, 0, D::Never, A::OneDay));
    }

    #[test]
    fn missing_timestamp_never_allows_even_when_never() {
        let now = t(10);
        for (device_at, password_at) in [(None, Some(t(0))), (Some(t(0)), None), (None, None)] {
            assert!(
                !UnlockWindowPolicy::device_auth_allowed(
                    now,
                    device_at,
                    password_at,
                    D::Never,
                    A::Never
                ),
                "{device_at:?} {password_at:?}"
            );
        }
    }

    #[test]
    fn never_windows_allow_any_elapsed_with_real_events() {
        assert!(allowed(10_000 * DAY, 0, 0, D::Never, A::Never));
        assert!(allowed(
            89 * DAY,
            89 * DAY - HOUR,
            0,
            D::OneHour,
            A::NinetyDays
        ));
    }

    #[test]
    fn clock_going_backwards_does_not_panic_and_allows() {
        // Both events are dated after `now`: elapsed counts as 0 for every window.
        for d in [D::OneHour, D::Never] {
            for a in [A::OneDay, A::Never] {
                assert!(allowed(0, DAY, 2 * DAY, d, a), "{d:?} {a:?}");
            }
        }
        assert!(UnlockWindowPolicy::device_auth_allowed(
            UNIX_EPOCH,
            Some(t(0)),
            Some(t(0)),
            D::OneHour,
            A::OneDay
        ));
    }
}

#[cfg(test)]
mod service_tests {
    use super::*;
    use crate::scheduler::ManualScheduler;
    use crate::session::{LockReason, SessionDeps};
    use crate::settings::SettingsPatch;
    use crate::testing::fast_database;
    use mypass_kdbx::{Database, PlainFileIo};
    use mypass_platform::testing::FakeDeviceUnlock;
    use tempfile::TempDir;

    const T0: u64 = 1_700_000_000;

    fn pw(s: &str) -> SecretString {
        SecretString::from(s.to_owned())
    }

    fn t(secs: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(T0 + secs)
    }

    fn code<T: std::fmt::Debug>(r: Result<T, ApiError>) -> ErrorCode {
        r.unwrap_err().code
    }

    struct Fx {
        dir: TempDir,
        session: VaultSession,
        settings: SettingsStore,
        scheduler: Arc<ManualScheduler>,
        fake: Arc<FakeDeviceUnlock>,
        requests: UnlockRequestState,
        service: DeviceUnlockService,
    }

    fn fx_with(availability: DeviceUnlockAvailability) -> Fx {
        let dir = tempfile::tempdir().unwrap();
        let settings = SettingsStore::in_memory();
        let scheduler = ManualScheduler::new(t(0));
        let session = VaultSession::new(SessionDeps {
            settings: settings.clone(),
            scheduler: scheduler.clone(),
            backups_root: dir.path().join("backups"),
            file_io: Arc::new(PlainFileIo),
        });
        let fake = FakeDeviceUnlock::new(availability);
        let requests = UnlockRequestState::new(scheduler.clone());
        let service = DeviceUnlockService::new(
            session.clone(),
            settings.clone(),
            fake.clone(),
            scheduler.clone(),
            requests.clone(),
            &AppPaths::macos(dir.path()),
        );
        Fx {
            dir,
            session,
            settings,
            scheduler,
            fake,
            requests,
            service,
        }
    }

    /// An unlocked vault `v.kdbx` (password `pw`), device unlock available with biometrics.
    fn fx() -> Fx {
        let f = fx_with(DeviceUnlockAvailability::Available { biometrics: true });
        f.session
            .create_vault_with(&f.vault_path(), pw("pw"), fast_database("Vault"))
            .unwrap();
        f
    }

    impl Fx {
        fn vault_path(&self) -> PathBuf {
            self.dir.path().join("v.kdbx")
        }
        fn id(&self) -> String {
            self.session.vault_id().unwrap()
        }
        fn enable(&self, touch_id: bool, mac_password: bool) -> DeviceUnlockStatus {
            self.service
                .configure(Some(pw("pw")), touch_id, mac_password)
                .unwrap()
        }
        fn lock(&self) {
            self.session.lock(LockReason::Manual);
            assert!(!self.session.is_unlocked());
        }
        fn advance(&self, secs: u64) {
            self.scheduler.advance(Duration::from_secs(secs));
        }
        fn toggles(&self) -> (bool, bool) {
            let s = self.settings.get();
            (s.unlock_touch_id_enabled, s.unlock_mac_password_enabled)
        }
        fn stamps(&self) -> Option<UnlockStamps> {
            self.service.state_store().get(&self.id())
        }
        fn enrolled(&self, mode: DeviceUnlockMode) -> bool {
            self.fake.is_enrolled(mode, &self.id()).unwrap()
        }
        fn state_file(&self) -> PathBuf {
            AppPaths::macos(self.dir.path())
                .config_dir
                .join(UnlockStateStore::FILE_NAME)
        }
    }

    fn stamps(device: u64, password: u64) -> Option<UnlockStamps> {
        let secs = |s: u64| i64::try_from(T0 + s).unwrap();
        Some(UnlockStamps {
            last_device_unlock: secs(device),
            last_full_password: secs(password),
        })
    }

    // ---- toggles and policy wiring ----

    #[test]
    fn mode_from_toggles_table_matches_2a() {
        assert_eq!(mode_for_toggles(false, false), None);
        assert_eq!(
            mode_for_toggles(true, false),
            Some(DeviceUnlockMode::Biometrics)
        );
        assert_eq!(
            mode_for_toggles(false, true),
            Some(DeviceUnlockMode::AccountPassword)
        );
        assert_eq!(
            mode_for_toggles(true, true),
            Some(DeviceUnlockMode::AccountPassword)
        );
    }

    #[test]
    fn status_uses_the_policy_for_can_offer_now() {
        let f = fx();
        let off = f.service.status();
        assert!(off.available && off.biometrics && !off.enrolled && !off.can_offer_now);
        assert_eq!(off.reason, Some(OfferBlock::Disabled));
        f.enable(true, false);
        let check = |expect: bool, reason: Option<OfferBlock>| {
            let st = f.service.status();
            let s = f.settings.get();
            let stamps = f.stamps().unwrap();
            let policy = UnlockWindowPolicy::device_auth_allowed(
                f.scheduler.wall_now(),
                Some(from_unix_seconds(stamps.last_device_unlock)),
                Some(from_unix_seconds(stamps.last_full_password)),
                s.unlock_with_device_window,
                s.require_account_password_window,
            );
            assert_eq!(st.can_offer_now, policy);
            assert_eq!((st.can_offer_now, st.reason), (expect, reason));
        };
        check(true, None);
        f.advance(8 * 3600);
        check(true, None);
        f.advance(1);
        check(false, Some(OfferBlock::DeviceWindowExpired));
        f.settings
            .update(SettingsPatch {
                unlock_with_device_window: Some(UnlockWithDeviceWindow::Never),
                require_account_password_window: Some(RequireAccountPasswordWindow::OneDay),
                ..SettingsPatch::default()
            })
            .unwrap();
        check(true, None);
        f.advance(86_400);
        check(false, Some(OfferBlock::AccountWindowExpired));
    }

    #[test]
    fn unavailable_device_is_reported_and_cannot_be_enabled() {
        let f = fx_with(DeviceUnlockAvailability::Unavailable);
        f.session
            .create_vault_with(&f.vault_path(), pw("pw"), fast_database("Vault"))
            .unwrap();
        let st = f.service.status();
        assert!(!st.available && !st.biometrics && !st.can_offer_now);
        assert_eq!(
            code(f.service.configure(Some(pw("pw")), true, false)),
            ErrorCode::DeviceUnlockUnavailable
        );
        assert_eq!(f.toggles(), (false, false));
    }

    // ---- the prompt coordinator ----

    fn coordinator() -> (PromptCoordinator, Arc<ManualScheduler>, UnlockRequestState) {
        let s = ManualScheduler::new(t(0));
        let requests = UnlockRequestState::new(s.clone());
        (
            PromptCoordinator::new(s.clone(), requests.clone()),
            s,
            requests,
        )
    }

    #[test]
    fn second_explicit_prompt_while_in_flight_is_refused() {
        let (c, _, _) = coordinator();
        assert!(c.request_explicit_prompt());
        assert!(!c.request_explicit_prompt());
    }

    #[test]
    fn prompt_finished_allows_next() {
        let (c, _, _) = coordinator();
        assert!(c.request_explicit_prompt());
        c.prompt_finished();
        assert!(c.request_explicit_prompt());
    }

    #[test]
    fn stale_in_flight_prompt_self_heals_at_60s() {
        let (c, s, _) = coordinator();
        assert!(c.request_explicit_prompt());
        s.advance(Duration::from_millis(59_900));
        assert!(!c.request_explicit_prompt());
        s.advance(Duration::from_millis(100));
        assert!(c.request_explicit_prompt());
        // The healed request is a new prompt in flight.
        assert!(!c.request_explicit_prompt());
    }

    #[test]
    fn wall_clock_set_back_does_not_wedge_the_prompt_slot() {
        let (c, s, _) = coordinator();
        assert!(c.request_explicit_prompt());
        s.set_wall_clock(UNIX_EPOCH);
        s.advance(PROMPT_STALE_AFTER);
        assert!(c.request_explicit_prompt());
    }

    #[test]
    fn lock_clears_in_flight_flag() {
        let (c, _, _) = coordinator();
        assert!(c.request_explicit_prompt());
        c.vault_did_lock();
        assert!(c.request_explicit_prompt());
    }

    #[test]
    fn lock_never_creates_a_request() {
        let (c, _, requests) = coordinator();
        c.vault_did_lock();
        assert!(!requests.consume());
        let f = fx();
        f.enable(true, false);
        f.lock();
        assert!(!f.service.consume_extension_request());
        assert_eq!(f.fake.evaluation_count(), 0);
    }

    #[test]
    fn view_appearance_without_request_never_prompts() {
        let f = fx();
        f.enable(true, false);
        f.lock();
        // The unlock screen appears: nothing was noted, so nothing presses the device button.
        assert!(!f.service.consume_extension_request());
        assert_eq!(f.fake.evaluation_count(), 0);
        // A noted extension request is consumed once.
        f.requests.note();
        assert!(f.service.consume_extension_request());
        assert!(!f.service.consume_extension_request());
        assert_eq!(f.fake.evaluation_count(), 0);
    }

    #[test]
    fn unlock_by_other_means_clears_pending_request() {
        let f = fx();
        f.lock();
        f.requests.note();
        f.session.unlock(pw("pw")).unwrap();
        f.lock();
        assert!(!f.service.consume_extension_request());
    }

    // ---- enrolment ----

    #[test]
    fn enroll_with_wrong_password_stores_nothing() {
        let f = fx();
        assert_eq!(
            code(f.service.configure(Some(pw("wrong")), true, false)),
            ErrorCode::WrongPassword
        );
        assert!(!f.enrolled(DeviceUnlockMode::Biometrics));
        assert!(!f.enrolled(DeviceUnlockMode::AccountPassword));
        assert_eq!(f.fake.enroll_count(), 0);
        assert_eq!(f.toggles(), (false, false));
        assert_eq!(f.stamps(), None);
    }

    #[test]
    fn enroll_without_password_is_refused() {
        let f = fx();
        assert_eq!(
            code(f.service.configure(None, false, true)),
            ErrorCode::InvalidInput
        );
        assert_eq!(f.fake.enroll_count(), 0);
        assert_eq!(f.toggles(), (false, false));
    }

    #[test]
    fn enroll_success_records_both_timestamps_and_device_button_available() {
        let f = fx();
        let st = f.enable(true, false);
        assert!(st.enrolled && st.can_offer_now && st.touch_id_enabled && !st.mac_password_enabled);
        assert_eq!(st.reason, None);
        assert!(f.enrolled(DeviceUnlockMode::Biometrics));
        assert_eq!(f.stamps(), stamps(0, 0));
        f.lock();
        assert!(f.service.status().can_offer_now);
    }

    #[test]
    fn enroll_while_locked_refused() {
        let f = fx();
        f.lock();
        assert_eq!(
            code(f.service.configure(Some(pw("pw")), true, false)),
            ErrorCode::Locked
        );
        assert_eq!(f.fake.enroll_count(), 0);
        assert_eq!(f.toggles(), (false, false));
        assert_eq!(f.stamps(), None);
    }

    #[test]
    fn disabling_both_toggles_forgets_item() {
        let f = fx();
        f.enable(false, true);
        let st = f.service.configure(None, false, false).unwrap();
        assert_eq!(st.reason, Some(OfferBlock::Disabled));
        assert!(!st.enrolled && !st.can_offer_now);
        assert_eq!(f.toggles(), (false, false));
        assert!(!f.enrolled(DeviceUnlockMode::AccountPassword));
        // Disabling also works while locked.
        f.enable(true, false);
        f.lock();
        f.service.configure(None, false, false).unwrap();
        assert_eq!(f.toggles(), (false, false));
        assert!(!f.enrolled(DeviceUnlockMode::Biometrics));
    }

    /// R16: enrolment is per mode, so a mode change re-enrols the new mode from the session's
    /// password (no prompt for it) and forgets the old one.
    #[test]
    fn switching_mode_while_unlocked_needs_no_password_and_reenrols() {
        let f = fx();
        f.enable(true, false);
        let st = f.service.configure(None, false, true).unwrap();
        assert!(st.enrolled && st.can_offer_now);
        assert_eq!(f.toggles(), (false, true));
        assert!(f.enrolled(DeviceUnlockMode::AccountPassword));
        assert!(!f.enrolled(DeviceUnlockMode::Biometrics));
        let secret = f
            .fake
            .unlock(DeviceUnlockMode::AccountPassword, &f.id(), "r")
            .unwrap();
        assert_eq!(secret.expose_secret(), "pw");
    }

    #[test]
    fn switching_mode_while_locked_is_refused() {
        let f = fx();
        f.enable(true, false);
        f.lock();
        assert_eq!(
            code(f.service.configure(None, false, true)),
            ErrorCode::Locked
        );
        assert_eq!(f.toggles(), (true, false));
        assert!(f.enrolled(DeviceUnlockMode::Biometrics));
        assert!(!f.enrolled(DeviceUnlockMode::AccountPassword));
    }

    #[test]
    fn enabling_touch_id_recaptures_domain_state() {
        let f = fx();
        f.enable(false, true);
        assert_eq!(f.fake.enroll_count(), 1);
        // Same mode (account password), Touch ID newly on: re-enrolled, no password needed.
        let st = f.service.configure(None, true, true).unwrap();
        assert!(st.enrolled && st.touch_id_enabled && st.mac_password_enabled);
        assert_eq!(f.fake.enroll_count(), 2);
        // Turning Touch ID off again keeps the item and enrols nothing.
        f.service.configure(None, false, true).unwrap();
        assert_eq!(f.fake.enroll_count(), 2);
        assert!(f.enrolled(DeviceUnlockMode::AccountPassword));
    }

    // ---- reconcile ----

    #[test]
    fn reconcile_turns_toggles_off_when_item_missing_without_prompting() {
        let f = fx();
        f.enable(true, true);
        f.lock();
        f.fake.forget(&f.id()).unwrap();
        let st = f.service.status();
        assert_eq!(f.toggles(), (false, false));
        assert_eq!(st.reason, Some(OfferBlock::Disabled));
        assert_eq!(f.fake.evaluation_count(), 0);
        assert_eq!(f.fake.presence_count(), 0);
    }

    #[test]
    fn reconcile_is_noop_when_enrolled() {
        let f = fx();
        f.enable(true, false);
        f.service.reconcile();
        assert_eq!(f.toggles(), (true, false));
        assert!(f.enrolled(DeviceUnlockMode::Biometrics));
        assert_eq!(f.fake.evaluation_count(), 0);
    }

    // ---- unlock with the device ----

    #[test]
    fn unlock_with_device_success_unlocks_and_records_device_timestamp_only() {
        let f = fx();
        f.enable(true, false);
        f.advance(3600);
        f.lock();
        f.service.unlock_with_device("Unlock MyPass").unwrap();
        assert!(f.session.is_unlocked());
        assert_eq!(f.fake.evaluation_count(), 1);
        assert_eq!(f.stamps(), stamps(3600, 0));
        // The prompt slot is free again.
        assert!(f.service.prompts().request_explicit_prompt());
    }

    #[test]
    fn unlock_with_device_while_unlocked_does_not_prompt() {
        let f = fx();
        f.enable(true, false);
        f.service.unlock_with_device("r").unwrap();
        assert_eq!(f.fake.evaluation_count(), 0);
    }

    #[test]
    fn unlock_outside_device_window_is_refused_and_does_not_prompt() {
        let f = fx();
        f.enable(true, false);
        f.lock();
        f.advance(8 * 3600 + 1);
        assert_eq!(
            code(f.service.unlock_with_device("r")),
            ErrorCode::InvalidInput
        );
        assert_eq!(f.fake.evaluation_count(), 0);
        assert!(!f.session.is_unlocked());
        // Still enrolled: only the master password brings the button back.
        assert!(f.enrolled(DeviceUnlockMode::Biometrics));
    }

    #[test]
    fn unlock_cancel_leaves_locked_enrolled_and_no_error_toast_code_is_cancelled() {
        let f = fx();
        f.enable(true, false);
        f.lock();
        f.fake.fail_next_unlock(PlatformError::Cancelled);
        let err = f.service.unlock_with_device("r").unwrap_err();
        assert_eq!(err.code, ErrorCode::Cancelled);
        assert!(err.message.is_empty());
        assert!(!f.session.is_unlocked());
        assert!(f.enrolled(DeviceUnlockMode::Biometrics));
        assert_eq!(f.toggles(), (true, false));
        // The slot was released: the next press prompts and unlocks.
        f.service.unlock_with_device("r").unwrap();
        assert!(f.session.is_unlocked());
        assert_eq!(f.fake.evaluation_count(), 2);
    }

    #[test]
    fn lockout_and_not_enrolled_on_device_pass_through_and_keep_enrolment() {
        let f = fx();
        f.enable(false, true);
        f.lock();
        for (e, expected) in [
            (PlatformError::Lockout, ErrorCode::Lockout),
            (
                PlatformError::NotEnrolledOnDevice,
                ErrorCode::NotEnrolledOnDevice,
            ),
        ] {
            f.fake.fail_next_unlock(e);
            assert_eq!(code(f.service.unlock_with_device("r")), expected);
            assert!(f.enrolled(DeviceUnlockMode::AccountPassword));
            assert_eq!(f.toggles(), (false, true));
        }
    }

    #[test]
    fn prompt_in_flight_refuses_device_unlock_without_a_second_prompt() {
        let f = fx();
        f.enable(true, false);
        f.lock();
        assert!(f.service.prompts().request_explicit_prompt());
        assert_eq!(code(f.service.unlock_with_device("r")), ErrorCode::Internal);
        assert_eq!(f.fake.evaluation_count(), 0);
    }

    #[test]
    fn stale_enrollment_is_dropped_on_wrong_password() {
        let f = fx();
        f.enable(true, false);
        f.lock();
        // Another app re-encrypts the vault with a new master password.
        let path = f.vault_path();
        let db = Database::open(&fs::read(&path).unwrap(), &pw("pw")).unwrap();
        mypass_kdbx::write_atomic(&path, &db.save(&pw("changed")).unwrap()).unwrap();
        assert_eq!(
            code(f.service.unlock_with_device("r")),
            ErrorCode::Invalidated
        );
        assert!(!f.session.is_unlocked());
        assert!(!f.enrolled(DeviceUnlockMode::Biometrics));
        assert_eq!(f.toggles(), (false, false));
        f.session.unlock(pw("changed")).unwrap();
    }

    #[test]
    fn biometric_change_invalidates_biometrics_only_enrollment() {
        // Found silently by reconcile: no prompt, toggles off.
        let f = fx();
        f.enable(true, false);
        f.lock();
        f.fake.invalidate_biometrics();
        assert_eq!(
            code(f.service.unlock_with_device("r")),
            ErrorCode::InvalidInput
        );
        assert_eq!(f.fake.evaluation_count(), 0);
        assert_eq!(f.toggles(), (false, false));
        assert!(!f.session.is_unlocked());
        // Found only at evaluation (a real Keychain reports it then): forgotten, `Invalidated`.
        let f = fx();
        f.enable(true, false);
        f.lock();
        f.fake.fail_next_unlock(PlatformError::Invalidated);
        assert_eq!(
            code(f.service.unlock_with_device("r")),
            ErrorCode::Invalidated
        );
        assert!(!f.enrolled(DeviceUnlockMode::Biometrics));
        assert_eq!(f.toggles(), (false, false));
        assert!(!f.session.is_unlocked());
    }

    #[test]
    fn biometric_change_does_not_invalidate_account_password_mode() {
        let f = fx();
        f.enable(true, true);
        f.lock();
        f.fake.invalidate_biometrics();
        f.service.unlock_with_device("r").unwrap();
        assert!(f.session.is_unlocked());
        assert_eq!(f.toggles(), (true, true));
    }

    // ---- timestamps ----

    #[test]
    fn password_unlock_resets_both_windows() {
        let f = fx();
        f.enable(true, false);
        f.lock();
        f.advance(9 * 3600);
        assert_eq!(
            f.service.status().reason,
            Some(OfferBlock::DeviceWindowExpired)
        );
        f.session.unlock(pw("pw")).unwrap();
        assert_eq!(f.stamps(), stamps(9 * 3600, 9 * 3600));
        f.lock();
        assert!(f.service.status().can_offer_now);
    }

    #[test]
    fn password_unlock_is_not_recorded_while_device_unlock_is_off() {
        let f = fx();
        f.lock();
        f.session.unlock(pw("pw")).unwrap();
        assert_eq!(f.stamps(), None);
        assert!(!f.state_file().exists());
    }

    #[test]
    fn state_store_corrupt_file_reads_empty_and_is_never_fatal() {
        let f = fx();
        fs::create_dir_all(f.state_file().parent().unwrap()).unwrap();
        fs::write(f.state_file(), b"{ not json").unwrap();
        assert_eq!(f.stamps(), None);
        f.enable(true, false);
        assert_eq!(f.stamps(), stamps(0, 0));
        // Oversized: read as empty.
        let big = vec![b' '; usize::try_from(UnlockStateStore::MAX_FILE_BYTES).unwrap() + 1];
        fs::write(f.state_file(), big).unwrap();
        assert_eq!(f.stamps(), None);
        assert_eq!(
            f.service.status().reason,
            Some(OfferBlock::AccountWindowExpired)
        );
        // Unwritable (a directory in the way): every call still works, the button stays hidden
        // until a stamp can be written.
        fs::remove_file(f.state_file()).unwrap();
        fs::create_dir(f.state_file()).unwrap();
        assert!(f
            .service
            .state_store()
            .record_password_unlock(&f.id(), t(0))
            .is_err());
        f.lock();
        f.session.unlock(pw("pw")).unwrap();
        assert!(!f.service.status().can_offer_now);
    }

    #[test]
    fn state_is_per_vault_id() {
        let dir = tempfile::tempdir().unwrap();
        let store = UnlockStateStore::new(dir.path());
        store.record_password_unlock("vault-a", t(10)).unwrap();
        assert_eq!(store.get("vault-b"), None);
        // A device unlock never creates a password stamp.
        store.record_device_unlock("vault-b", t(20)).unwrap();
        assert_eq!(store.get("vault-b"), None);
        store.record_device_unlock("vault-a", t(30)).unwrap();
        assert_eq!(store.get("vault-a"), stamps(30, 10));
        let text = fs::read_to_string(dir.path().join(UnlockStateStore::FILE_NAME)).unwrap();
        let json: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "vault-a": { "lastDeviceUnlock": T0 + 30, "lastFullPassword": T0 + 10 }
            })
        );
    }

    #[test]
    fn unix_seconds_round_trip_and_saturate() {
        for secs in [0_i64, 1, -1, 1_700_000_000, -86_400] {
            assert_eq!(unix_seconds(from_unix_seconds(secs)), secs);
        }
        let _ = from_unix_seconds(i64::MIN);
        let _ = from_unix_seconds(i64::MAX);
    }
}
