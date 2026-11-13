//! The command facade: one method per Tauri command, with the identical snake_case name, so the
//! shell's `#[tauri::command]` wrappers are one-line delegations and every rule below is testable
//! without a window server.
//!
//! The facade is thin on purpose: it parses ids (`Uuid::parse_str`, a malformed id is
//! `InvalidInput`), calls the module that owns the behaviour and maps the error to [`ApiError`].
//!
//! # Rules that hold for every method
//!
//! - **Locked means locked.** A method that touches the vault checks the session state BEFORE any
//!   other work: before ids are parsed and before a file is read (attachment `Add` operations,
//!   CSV files). `Locked` (or `NoVault`) comes first, `Quitting` second. Mutating and opening
//!   calls are refused while quitting; reads, `vault_lock` and flushing keep working.
//! - **Nothing slow under the session lock.** The closures given to `with_db` / `mutate` / `touch`
//!   only copy data; file I/O, the clipboard and the draft store run outside them.
//! - **The clock is injected.** Times (TOTP codes, drafts) come from the scheduler's wall clock, so
//!   tests with a `ManualScheduler` are deterministic.
//! - **Secrets stay in Rust.** Copy actions hand the value to the clipboard service and return
//!   `()`; an [`ApiError`] message never holds a password, note, field value or CSV cell.

use crate::autolock::AutoLock;
use crate::backups::{BackupManager, BackupStatus};
use crate::clipboard_service::ClipboardService;
use crate::csv_import::{self, CsvImportError};
use crate::drafts::{self, DraftStore, DraftVersionDto};
use crate::dto::{
    AppInfo, CsvPreview, EntryDetailDto, EntryEditModel, EntryQuery, EntrySummary, EntryTypeDto,
    FieldRef, FoundVaultFileDto, GenerateRequest, GeneratedPassword, GeneratorHistoryItem,
    HistoryItem, ImportResult, LoginItemStatus, OpenEntryEvent, OpenWarning, RecentVaultDto,
    RulesSummary, Sidebar, TotpDto,
};
use crate::editing;
use crate::generator_service;
use crate::queries;
use crate::scheduler::{lock, Scheduler, ThreadScheduler};
use crate::session::{
    GeneratorRulesError, LockReason, SessionDeps, SessionError, SessionStatus, VaultSession,
};
use crate::settings::{Settings, SettingsError, SettingsPatch, SettingsStore};
use crate::unlock_request::UnlockRequestState;
use crate::vault_files::{self, ProcessLookup, ScanLimits};
use mypass_kdbx::{Database, PlainFileIo};
pub use mypass_platform::{AppPaths, Platform};
use mypass_platform::{DeviceUnlockAvailability, PlatformError, VaultId};
use secrecy::SecretString;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Instant, UNIX_EPOCH};
use uuid::Uuid;
use zeroize::Zeroizing;

/// The stable error code the UI localises. Serialised in camelCase.
#[derive(Serialize, specta::Type, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase")]
pub enum ErrorCode {
    Locked,
    NoVault,
    WrongPassword,
    AlreadyExists,
    NotFound,
    InvalidInput,
    LimitExceeded,
    Corrupt,
    Unsupported,
    ChangedOnDisk,
    Io,
    Quitting,
    GeneratorNoClass,
    GeneratorUnsatisfiable,
    Cancelled,
    Lockout,
    Invalidated,
    NotEnrolledOnDevice,
    DeviceUnlockUnavailable,
    Internal,
}

/// What every command returns on failure. `message` is English diagnostics for logs: it never
/// contains a secret (no password, note, field value or CSV cell; only field names, counts and
/// file-system wording), and it is empty for codes the UI fully explains by themselves.
#[derive(Serialize, specta::Type, Clone, PartialEq, Debug, thiserror::Error)]
#[error("{code:?}: {message}")]
pub struct ApiError {
    pub code: ErrorCode,
    pub message: String,
}

impl ApiError {
    fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl From<SessionError> for ApiError {
    fn from(e: SessionError) -> Self {
        use ErrorCode as C;
        use SessionError as S;
        let (code, message) = match &e {
            S::Locked => (C::Locked, e.to_string()),
            S::NoVault => (C::NoVault, e.to_string()),
            S::WrongPassword => (C::WrongPassword, e.to_string()),
            S::AlreadyExists => (C::AlreadyExists, e.to_string()),
            S::NotFound => (C::NotFound, e.to_string()),
            S::InvalidInput(m) => (C::InvalidInput, m.clone()),
            S::LimitExceeded(m) => (C::LimitExceeded, m.clone()),
            S::Corrupt(m) => (C::Corrupt, m.clone()),
            S::Unsupported(m) => (C::Unsupported, m.clone()),
            S::ChangedOnDisk => (C::ChangedOnDisk, e.to_string()),
            S::Io(m) => (C::Io, m.clone()),
            S::Quitting => (C::Quitting, e.to_string()),
            // Bare codes: the UI has its own wording for both.
            S::GeneratorRules(GeneratorRulesError::NoClass) => (C::GeneratorNoClass, String::new()),
            S::GeneratorRules(GeneratorRulesError::Unsatisfiable) => {
                (C::GeneratorUnsatisfiable, String::new())
            }
        };
        Self::new(code, message)
    }
}

impl From<PlatformError> for ApiError {
    fn from(e: PlatformError) -> Self {
        use ErrorCode as C;
        match &e {
            PlatformError::Unsupported(what) => Self::new(C::Unsupported, *what),
            PlatformError::Failed(m) => Self::new(C::Internal, m.clone()),
            PlatformError::Cancelled => Self::new(C::Cancelled, String::new()),
            PlatformError::Lockout => Self::new(C::Lockout, String::new()),
            PlatformError::Invalidated => Self::new(C::Invalidated, String::new()),
            PlatformError::NotEnrolledOnDevice => Self::new(C::NotEnrolledOnDevice, String::new()),
            PlatformError::DeviceUnlockUnavailable => {
                Self::new(C::DeviceUnlockUnavailable, String::new())
            }
        }
    }
}

impl From<SettingsError> for ApiError {
    fn from(e: SettingsError) -> Self {
        match e {
            SettingsError::InvalidValue(field) => Self::new(ErrorCode::InvalidInput, field),
            SettingsError::Io(m) => Self::new(ErrorCode::Io, m),
        }
    }
}

impl From<CsvImportError> for ApiError {
    fn from(e: CsvImportError) -> Self {
        // The messages name a column or nothing, never a cell.
        match e {
            CsvImportError::TooLarge => {
                Self::new(ErrorCode::LimitExceeded, "the file is too large")
            }
            other => Self::new(ErrorCode::InvalidInput, other.to_string()),
        }
    }
}

/// `ProcessLookup` default: no other MyPass processes (the shell installs the real lookup).
struct NoOtherProcesses;

impl ProcessLookup for NoOtherProcesses {
    fn other_app_executables(&self, _bundle_id: &str) -> Vec<PathBuf> {
        Vec::new()
    }
}

fn parse_id(id: &str) -> Result<Uuid, SessionError> {
    Uuid::parse_str(id).map_err(|_| SessionError::InvalidInput("malformed id".into()))
}

/// The facade over the whole backend (cheap to clone; every clone is the same app).
#[derive(Clone)]
pub struct AppCore {
    pub session: VaultSession,
    pub settings: SettingsStore,
    pub autolock: AutoLock,
    pub clipboard: ClipboardService,
    pub backups: BackupManager,
    pub unlock_requests: UnlockRequestState,
    platform: Platform,
    scheduler: Arc<dyn Scheduler>,
    paths: AppPaths,
    processes: Arc<dyn ProcessLookup>,
    /// Serialises the read-modify-write of the draft files (the store itself is unsynchronised).
    /// Serialises draft work. Holds the id of the vault whose draft key was created in memory
    /// but has not yet reached the disk (cleared only by a successful flush).
    drafts: Arc<Mutex<Option<String>>>,
}

impl AppCore {
    /// Production wiring: the real settings file under `config_dir` and worker threads for the
    /// timers. Two schedulers, not one: the session, auto-lock and unlock-request timers share
    /// one, and the clipboard clear has its own, so a vault write that hangs on a stuck drive
    /// cannot hold back the clearing of a copied password. (A single scheduler would otherwise
    /// serve every service: the services only need the `Scheduler` trait.)
    pub fn new(paths: AppPaths, platform: Platform) -> Result<Self, ApiError> {
        let worker = |name: &str| {
            ThreadScheduler::new(name).map_err(|e| ApiError::new(ErrorCode::Io, e.to_string()))
        };
        let scheduler = worker("mypass-scheduler")?;
        let clipboard_scheduler = worker("mypass-clipboard")?;
        let settings = SettingsStore::load(paths.config_dir.join("settings.json"));
        Ok(Self::assemble(
            paths,
            platform,
            scheduler,
            clipboard_scheduler,
            settings,
        ))
    }

    /// Wiring for tests: every timer on `scheduler` (a `ManualScheduler`), settings as given.
    pub fn with_parts(
        paths: AppPaths,
        platform: Platform,
        scheduler: Arc<dyn Scheduler>,
        settings: SettingsStore,
    ) -> Self {
        Self::assemble(paths, platform, scheduler.clone(), scheduler, settings)
    }

    fn assemble(
        paths: AppPaths,
        platform: Platform,
        scheduler: Arc<dyn Scheduler>,
        clipboard_scheduler: Arc<dyn Scheduler>,
        settings: SettingsStore,
    ) -> Self {
        let session = VaultSession::new(SessionDeps {
            settings: settings.clone(),
            scheduler: scheduler.clone(),
            backups_root: paths.backups_dir.clone(),
            file_io: Arc::new(PlainFileIo),
        });
        let autolock = AutoLock::new(session.clone(), settings.clone(), scheduler.clone());
        let clipboard = ClipboardService::new(
            platform.clipboard.clone(),
            clipboard_scheduler,
            settings.clone(),
        );
        Self {
            backups: session.backups(),
            unlock_requests: UnlockRequestState::new(scheduler.clone()),
            session,
            settings,
            autolock,
            clipboard,
            platform,
            scheduler,
            paths,
            processes: Arc::new(NoOtherProcesses),
            drafts: Arc::new(Mutex::new(None)),
        }
    }

    /// The one scheduler of the session, auto-lock and unlock requests (shell and bridge reuse it).
    pub fn scheduler(&self) -> Arc<dyn Scheduler> {
        self.scheduler.clone()
    }

    pub fn paths(&self) -> &AppPaths {
        &self.paths
    }

    pub fn platform(&self) -> &Platform {
        &self.platform
    }

    /// Installs the lookup behind [`AppCore::vault_open_warnings`] (default: no other processes).
    pub fn set_process_lookup(&mut self, lookup: Arc<dyn ProcessLookup>) {
        self.processes = lookup;
    }

    pub fn begin_quit(&self) {
        self.session.begin_quit();
    }

    pub fn cancel_quit(&self) {
        self.session.cancel_quit();
    }

    pub fn is_quitting(&self) -> bool {
        self.session.is_quitting()
    }

    // ---- guards and small helpers ----

    /// `Locked` / `NoVault` unless the vault is open.
    fn unlocked(&self) -> Result<(), ApiError> {
        self.session.with_db(|_| ())?;
        Ok(())
    }

    /// The guard of every mutating call: `Locked` first, `Quitting` second.
    fn writable(&self) -> Result<(), ApiError> {
        self.unlocked()?;
        if self.session.is_quitting() {
            return Err(SessionError::Quitting.into());
        }
        Ok(())
    }

    /// Runs `f` on the open database; the state is checked before `f` (so before ids are parsed).
    fn read<R>(&self, f: impl FnOnce(&Database) -> Result<R, SessionError>) -> Result<R, ApiError> {
        Ok(self.session.with_db(f)??)
    }

    /// The injected clock as the signed seconds the draft store takes.
    fn now_i64(&self) -> i64 {
        i64::try_from(self.now_secs()).unwrap_or(i64::MAX)
    }

    fn now_secs(&self) -> u64 {
        self.scheduler
            .wall_now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs())
    }

    fn copy(&self, text: &str) -> Result<(), ApiError> {
        Ok(self.clipboard.copy_secret(text)?)
    }

    fn vault_id(&self) -> Result<String, ApiError> {
        Ok(self.session.vault_id().ok_or(SessionError::NoVault)?)
    }

    // ---- app and platform ----

    pub fn app_info(&self) -> AppInfo {
        AppInfo {
            version: env!("CARGO_PKG_VERSION").to_owned(),
            os: std::env::consts::OS.to_owned(),
            device_unlock_available: self.platform.device_unlock.availability()
                != DeviceUnlockAvailability::Unavailable,
        }
    }

    /// The short keyboard-layout label; a platform without one reports `None`.
    pub fn platform_keyboard_layout(&self) -> Result<Option<String>, ApiError> {
        match self.platform.keyboard_layout.current() {
            Ok(label) => Ok(label),
            Err(PlatformError::Unsupported(_)) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// `Unsupported` is a value (the General pane hides the row), not an error.
    pub fn settings_login_item_status(&self) -> Result<LoginItemStatus, ApiError> {
        match self.platform.launch_at_login.status() {
            Ok(status) => Ok(status.into()),
            Err(PlatformError::Unsupported(_)) => Ok(LoginItemStatus::Unsupported),
            Err(e) => Err(e.into()),
        }
    }

    pub fn app_record_activity(&self) -> Result<(), ApiError> {
        self.autolock.record_activity();
        Ok(())
    }

    /// The tray's Edit button: `None` while locked (the shell then only shows `main`, whose unlock
    /// screen appears), the event to emit otherwise.
    pub fn open_entry_request(
        &self,
        id: &str,
        edit: bool,
    ) -> Result<Option<OpenEntryEvent>, ApiError> {
        if !self.session.is_unlocked() {
            return Ok(None);
        }
        if self.session.is_quitting() {
            return Err(SessionError::Quitting.into());
        }
        let entry = parse_id(id)?;
        self.read(|db| {
            db.find_entry(entry)
                .map(|_| ())
                .ok_or(SessionError::NotFound)
        })?;
        Ok(Some(OpenEntryEvent {
            id: entry.simple().to_string(),
            edit,
        }))
    }

    /// The current vault's backup folder, created when missing (`app_reveal_backups` opens it).
    pub fn backup_folder(&self) -> Result<PathBuf, ApiError> {
        let id = self.vault_id()?;
        Ok(self.backups.ensure_folder(&id)?)
    }

    // ---- vault ----

    pub fn vault_status(&self) -> Result<SessionStatus, ApiError> {
        Ok(self.session.status())
    }

    pub fn vault_create(
        &self,
        path: &str,
        password: SecretString,
        name: &str,
    ) -> Result<SessionStatus, ApiError> {
        self.session.create_vault(Path::new(path), password, name)?;
        Ok(self.session.status())
    }

    pub fn vault_open(
        &self,
        path: &str,
        password: SecretString,
    ) -> Result<SessionStatus, ApiError> {
        self.session.open_vault(Path::new(path), password)?;
        self.purge_expired_drafts();
        Ok(self.session.status())
    }

    pub fn vault_unlock(&self, password: SecretString) -> Result<SessionStatus, ApiError> {
        self.session.unlock(password)?;
        self.purge_expired_drafts();
        Ok(self.session.status())
    }

    pub fn vault_lock(&self) -> Result<(), ApiError> {
        self.session.lock(LockReason::Manual);
        Ok(())
    }

    /// `.kdbx` files below `roots` (the command supplies the roots), newest first.
    pub fn vault_find_files(
        &self,
        roots: Vec<PathBuf>,
    ) -> Result<Vec<FoundVaultFileDto>, ApiError> {
        Ok(vault_files::scan_for_vaults(
            &roots,
            &ScanLimits::default(),
            Instant::now,
        ))
    }

    pub fn vault_recent(&self) -> Result<Vec<RecentVaultDto>, ApiError> {
        Ok(vault_files::recent_vaults(
            &self.settings.get().recent_vaults,
        ))
    }

    pub fn vault_forget(&self, path: &str) -> Result<(), ApiError> {
        self.settings.forget_vault(path);
        Ok(())
    }

    /// What to confirm before creating or opening the vault at `path` (iCloud, the old app).
    pub fn vault_open_warnings(&self, path: &str) -> Result<Vec<OpenWarning>, ApiError> {
        let current = std::env::current_exe().unwrap_or_default();
        Ok(vault_files::open_warnings(
            Path::new(path),
            self.paths.home(),
            self.processes.as_ref(),
            &current,
        ))
    }

    // ---- sidebar and entries (reads) ----

    pub fn sidebar_get(&self) -> Result<Sidebar, ApiError> {
        self.read(|db| Ok(queries::sidebar(db)))
    }

    pub fn entry_list(&self, query: EntryQuery) -> Result<Vec<EntrySummary>, ApiError> {
        // In-app search covers everything visible; the bridge uses `SearchScope::Autofill`.
        self.read(|db| Ok(queries::list_entries(db, &query, queries::SearchScope::App)))
    }

    pub fn entry_recent(&self, limit: u32) -> Result<Vec<EntrySummary>, ApiError> {
        self.read(|db| Ok(queries::recent_entries(db, limit as usize)))
    }

    pub fn entry_get(&self, id: &str) -> Result<EntryDetailDto, ApiError> {
        self.read(|db| queries::entry_detail(db, parse_id(id)?).ok_or(SessionError::NotFound))
    }

    pub fn entry_totp(&self, id: &str) -> Result<TotpDto, ApiError> {
        let now = self.now_secs();
        self.read(|db| {
            let id = parse_id(id)?;
            if db.find_entry(id).is_none() {
                return Err(SessionError::NotFound);
            }
            queries::totp_now(db, id, now)
                .ok_or_else(|| SessionError::InvalidInput("the entry has no valid TOTP".into()))
        })
    }

    /// One plain value, after an explicit click.
    pub fn entry_reveal(&self, id: &str, field: FieldRef) -> Result<String, ApiError> {
        let now = self.now_secs();
        self.read(|db| editing::reveal_at(db, parse_id(id)?, &field, now))
    }

    /// Copies a field to the clipboard from Rust (cleared after the configured seconds); the
    /// value never leaves the backend.
    pub fn entry_copy(&self, id: &str, field: FieldRef) -> Result<(), ApiError> {
        let now = self.now_secs();
        let value =
            Zeroizing::new(self.read(|db| editing::reveal_at(db, parse_id(id)?, &field, now))?);
        self.copy(&value)
    }

    pub fn entry_new_model(
        &self,
        entry_type: EntryTypeDto,
        group_id: Option<&str>,
    ) -> Result<EntryEditModel, ApiError> {
        self.read(|db| {
            let group = match group_id {
                Some(g) => parse_id(g)?,
                None => db.root_id(),
            };
            if db.find_group(group).is_none() {
                return Err(SessionError::NotFound);
            }
            Ok(editing::new_model(entry_type, group))
        })
    }

    pub fn entry_edit_model(&self, id: &str) -> Result<EntryEditModel, ApiError> {
        self.read(|db| editing::edit_model(db, parse_id(id)?))
    }

    pub fn entry_history(&self, id: &str) -> Result<Vec<HistoryItem>, ApiError> {
        self.read(|db| editing::history(db, parse_id(id)?))
    }

    // ---- entries (writes) ----

    /// Creates or saves an entry; a Notes entry also drops its drafts. Returns the entry id.
    pub fn entry_save(&self, model: EntryEditModel) -> Result<String, ApiError> {
        // Before `save_entry` reads any attachment file.
        self.writable()?;
        let notes = model.entry_type == EntryTypeDto::Notes;
        let id = editing::save_entry(&self.session, model)?;
        if notes {
            self.clear_drafts(&[id]);
        }
        Ok(id.simple().to_string())
    }

    pub fn entry_delete(&self, id: &str) -> Result<(), ApiError> {
        self.writable()?;
        let id = parse_id(id)?;
        editing::delete_entry(&self.session, id)?;
        self.clear_drafts(&[id]);
        Ok(())
    }

    /// Moves a bin entry to `group_id` (the root when `None`).
    pub fn entry_restore(&self, id: &str, group_id: Option<&str>) -> Result<(), ApiError> {
        self.writable()?;
        let id = parse_id(id)?;
        let group = match group_id {
            Some(g) => parse_id(g)?,
            None => self.session.with_db(Database::root_id)?,
        };
        Ok(editing::restore_entry(&self.session, id, group)?)
    }

    pub fn entry_purge(&self, id: &str) -> Result<(), ApiError> {
        self.writable()?;
        let id = parse_id(id)?;
        editing::purge_entry(&self.session, id)?;
        self.clear_drafts(&[id]);
        Ok(())
    }

    /// Permanently removes everything in the recycle bin; returns how many entries went.
    pub fn entry_empty_bin(&self) -> Result<u32, ApiError> {
        self.writable()?;
        let (count, removed) = editing::empty_bin_removed(&self.session)?;
        self.clear_drafts(&removed);
        Ok(count)
    }

    pub fn entry_set_favorite(&self, id: &str, favorite: bool) -> Result<(), ApiError> {
        self.writable()?;
        Ok(editing::set_favorite(
            &self.session,
            parse_id(id)?,
            favorite,
        )?)
    }

    pub fn entry_move(&self, id: &str, group_id: &str) -> Result<(), ApiError> {
        self.writable()?;
        Ok(editing::move_entry(
            &self.session,
            parse_id(id)?,
            parse_id(group_id)?,
        )?)
    }

    /// Records that the entry was used (access time and count); never marks the vault dirty.
    pub fn entry_touch(&self, id: &str) -> Result<(), ApiError> {
        self.writable()?;
        let id = parse_id(id)?;
        if self.session.touch(|db, now| db.mark_accessed(id, now))? {
            Ok(())
        } else {
            Err(SessionError::NotFound.into())
        }
    }

    pub fn entry_history_restore(&self, id: &str, index: u32) -> Result<(), ApiError> {
        self.writable()?;
        Ok(editing::restore_history(
            &self.session,
            parse_id(id)?,
            index as usize,
        )?)
    }

    /// Writes one attachment to `dest_path`. The bytes are copied out under the session lock; the
    /// file is written after it is released.
    pub fn attachment_export(&self, id: &str, name: &str, dest_path: &str) -> Result<(), ApiError> {
        let bytes = self.read(|db| editing::attachment_bytes(db, parse_id(id)?, name))?;
        Ok(editing::write_attachment(Path::new(dest_path), &bytes)?)
    }

    // ---- groups ----

    /// Creates a group under `parent_id` (the root when `None`); returns its id.
    pub fn group_create(
        &self,
        parent_id: Option<&str>,
        name: &str,
        emoji: Option<&str>,
    ) -> Result<String, ApiError> {
        self.writable()?;
        let parent = match parent_id {
            Some(p) => parse_id(p)?,
            None => self.session.with_db(Database::root_id)?,
        };
        let id = editing::create_group(&self.session, parent, name, emoji)?;
        Ok(id.simple().to_string())
    }

    pub fn group_update(&self, id: &str, name: &str, emoji: Option<&str>) -> Result<(), ApiError> {
        self.writable()?;
        Ok(editing::update_group(
            &self.session,
            parse_id(id)?,
            name,
            emoji,
        )?)
    }

    pub fn group_move(&self, id: &str, parent_id: &str) -> Result<(), ApiError> {
        self.writable()?;
        Ok(editing::move_group(
            &self.session,
            parse_id(id)?,
            parse_id(parent_id)?,
        )?)
    }

    /// Deletes a group with its entries (to the bin) and drops the drafts of those entries.
    pub fn group_delete(&self, id: &str) -> Result<(), ApiError> {
        self.writable()?;
        let id = parse_id(id)?;
        let affected = editing::delete_group_removed(&self.session, id)?;
        self.clear_drafts(&affected);
        Ok(())
    }

    // ---- note drafts ----

    /// The draft store of the open vault and whether its key exists, without modifying anything.
    fn draft_context(&self) -> Result<(String, Option<[u8; 32]>), ApiError> {
        Ok(self
            .session
            .with_db(|db| (db.root_id().simple().to_string(), drafts::existing_key(db)))?)
    }

    fn draft_store(&self, vault_id: &str, key: [u8; 32]) -> Result<DraftStore, ApiError> {
        let id = VaultId::new(vault_id)?;
        Ok(DraftStore::new(self.paths.drafts_dir(&id), vault_id, key))
    }

    /// Drops drafts past their time to live; runs after a successful open or unlock. Best effort
    /// (it never fails the caller); with no draft key there are no drafts.
    fn purge_expired_drafts(&self) {
        let _serial = lock(&self.drafts);
        if let Ok((vault_id, Some(key))) = self.draft_context() {
            if let Ok(store) = self.draft_store(&vault_id, key) {
                store.purge_expired(self.now_i64());
            }
        }
    }

    /// Best effort and never fails the caller: the mutation it follows already succeeded. With no
    /// draft key there are no drafts.
    fn clear_drafts(&self, ids: &[Uuid]) {
        if ids.is_empty() {
            return;
        }
        let _serial = lock(&self.drafts);
        if let Ok((vault_id, Some(key))) = self.draft_context() {
            if let Ok(store) = self.draft_store(&vault_id, key) {
                for id in ids {
                    store.clear(*id);
                }
            }
        }
    }

    /// Saves a version of the notes being edited. The first save of a vault's life creates the
    /// key in the vault and WRITES THE VAULT before the draft: a draft encrypted under a key that
    /// never reached the disk could not be recovered after a crash. When that write fails the
    /// result is `false` and no draft file is written. `false` also means the store refused
    /// (identical text, oversized notes).
    pub fn draft_save(&self, entry_id: &str, notes: &str) -> Result<bool, ApiError> {
        self.writable()?;
        let entry = parse_id(entry_id)?;
        self.read(|db| {
            db.find_entry(entry)
                .map(|_| ())
                .ok_or(SessionError::NotFound)
        })?;
        let mut unpersisted = lock(&self.drafts);
        let (vault_id, existing) = self.draft_context()?;
        let key = match existing {
            Some(key) => key,
            None => {
                let key = drafts::load_or_create_key(&self.session)?;
                *unpersisted = Some(vault_id.clone());
                key
            }
        };
        // A key that exists only in memory (just created, or created by an earlier call whose
        // write failed) must reach the disk before any draft is written under it. A key that is
        // already on disk needs no flush, so drafts keep working while vault saves fail.
        if unpersisted.as_deref() == Some(vault_id.as_str()) {
            if self.session.flush().is_err() {
                return Ok(false);
            }
            *unpersisted = None;
        }
        let store = self.draft_store(&vault_id, key)?;
        let now = self.now_i64();
        Ok(store.save(entry, notes, now))
    }

    /// The saved versions, newest first; empty when the vault has no draft key yet. Never
    /// modifies the vault.
    pub fn draft_versions(&self, entry_id: &str) -> Result<Vec<DraftVersionDto>, ApiError> {
        self.unlocked()?;
        let entry = parse_id(entry_id)?;
        let _serial = lock(&self.drafts);
        let (vault_id, existing) = self.draft_context()?;
        let Some(key) = existing else {
            return Ok(Vec::new());
        };
        let now = self.now_i64();
        Ok(self.draft_store(&vault_id, key)?.versions(entry, now))
    }

    pub fn draft_clear(&self, entry_id: &str) -> Result<(), ApiError> {
        self.unlocked()?;
        let entry = parse_id(entry_id)?;
        self.clear_drafts(&[entry]);
        Ok(())
    }

    // ---- generator ----

    /// Generates a password. With `record_url` it is also written to the generator history, which
    /// needs the unlocked vault; without it the vault is never touched.
    pub fn generator_generate(&self, req: GenerateRequest) -> Result<GeneratedPassword, ApiError> {
        if req.record_url.is_some() {
            self.writable()?;
        }
        Ok(generator_service::generate_password(
            Some(&self.session),
            &req,
        )?)
    }

    pub fn generator_rules_summary(&self, host: &str) -> Result<Option<RulesSummary>, ApiError> {
        Ok(generator_service::rules_summary(host))
    }

    pub fn generator_history_list(&self) -> Result<Vec<GeneratorHistoryItem>, ApiError> {
        self.read(|db| Ok(generator_service::history_list(db)))
    }

    /// Copies `value` to the clipboard. With `record_url` the history entry is written FIRST and
    /// the copy happens only when that succeeded: never a copied password the user believes is in
    /// the history when it is not.
    pub fn generator_copy(&self, value: &str, record_url: Option<&str>) -> Result<(), ApiError> {
        if let Some(url) = record_url {
            self.writable()?;
            generator_service::record_history(&self.session, value, url)?;
        }
        self.copy(value)
    }

    pub fn generator_history_copy(&self, id: &str) -> Result<(), ApiError> {
        let value =
            Zeroizing::new(self.read(|db| generator_service::history_reveal(db, parse_id(id)?))?);
        self.copy(&value)
    }

    pub fn generator_history_reveal(&self, id: &str) -> Result<String, ApiError> {
        self.read(|db| generator_service::history_reveal(db, parse_id(id)?))
    }

    pub fn generator_history_delete(&self, id: &str) -> Result<(), ApiError> {
        self.writable()?;
        Ok(generator_service::history_delete(
            &self.session,
            parse_id(id)?,
        )?)
    }

    // ---- settings ----

    pub fn settings_get(&self) -> Result<Settings, ApiError> {
        Ok(self.settings.get())
    }

    /// Applies the patch and re-arms the idle timer with the new timeout.
    pub fn settings_update(&self, patch: SettingsPatch) -> Result<Settings, ApiError> {
        let updated = self.settings.update(patch)?;
        self.autolock.settings_changed();
        Ok(updated)
    }

    // ---- backups ----

    /// The backups of the current vault (also while it is locked: the folder is the vault's).
    pub fn backup_list(&self) -> Result<BackupStatus, ApiError> {
        Ok(self.backups.status(&self.vault_id()?))
    }

    /// Replaces the vault with backup `id`, but only when that backup opens with the CURRENT
    /// master password. The refusal comes before any write, so a refused restore leaves the vault
    /// file, the open database, the drafts and the backup folder untouched. Then: the current file
    /// is snapshotted, the backup is written atomically over the vault, the open database is
    /// reloaded and every draft is dropped (they belong to the replaced content).
    pub fn backup_restore(&self, id: &str) -> Result<(), ApiError> {
        self.writable()?;
        let vault_id = self.vault_id()?;
        let bytes = self.backups.read(&vault_id, id)?;
        let restored = self.session.restore_from_backup(&bytes);
        // A reload failure after the write locks the session: the file already holds the
        // restored content, so the drafts are dropped in that case too.
        if restored.is_ok() || !self.session.is_unlocked() {
            let _serial = lock(&self.drafts);
            // `clear_all` needs no key; the zero key only satisfies the constructor.
            self.draft_store(&vault_id, [0u8; 32])?.clear_all();
        }
        Ok(restored?)
    }

    // ---- CSV import ----

    /// Counts and the first titles of a CSV file; reads the file, never the vault.
    pub fn import_csv_preview(&self, path: &str) -> Result<CsvPreview, ApiError> {
        let bytes = Zeroizing::new(csv_import::read_csv_file(Path::new(path))?);
        let parse = csv_import::parse_csv(&bytes)?;
        Ok(CsvPreview::from_parse(&parse))
    }

    /// Imports the file's logins into `group_id` in one mutation (all or none).
    pub fn import_csv_commit(&self, path: &str, group_id: &str) -> Result<ImportResult, ApiError> {
        // Before the file is read.
        self.writable()?;
        let group = parse_id(group_id)?;
        let bytes = Zeroizing::new(csv_import::read_csv_file(Path::new(path))?);
        let parse = csv_import::parse_csv(&bytes)?;
        let skipped = u32::try_from(parse.skipped.len()).unwrap_or(u32::MAX);
        let imported = csv_import::import_logins(&self.session, group, parse.logins)?;
        Ok(ImportResult { imported, skipped })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dto::{AttachmentOp, EntryTypeDto, FieldRef, GenerateRequest};
    use crate::editing::new_model;
    use crate::scheduler::ManualScheduler;
    use crate::session::SaveState;
    use crate::settings::{GeneratorOptionsDto, SettingsStore};
    use crate::testing::fast_database;
    use mypass_kdbx::test_util::FailingFileIo;
    use mypass_platform::testing::{fake_platform, FakePlatform};
    use mypass_platform::PlatformError;
    use secrecy::SecretString;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::{Duration, UNIX_EPOCH};
    use tempfile::TempDir;
    use uuid::Uuid;

    const T0: u64 = 1_700_000_000;

    fn pw(s: &str) -> SecretString {
        SecretString::from(s.to_owned())
    }

    #[allow(dead_code)]
    struct Fx {
        dir: TempDir,
        core: AppCore,
        fake: FakePlatform,
        clock: Arc<ManualScheduler>,
        path: PathBuf,
    }

    /// An `AppCore` over a temp home with no vault open.
    fn fx() -> Fx {
        let dir = tempfile::tempdir().unwrap();
        let fake = fake_platform();
        let clock = ManualScheduler::new(UNIX_EPOCH + Duration::from_secs(T0));
        let core = AppCore::with_parts(
            AppPaths::macos(dir.path()),
            fake.platform.clone(),
            clock.clone(),
            SettingsStore::in_memory(),
        );
        let path = dir.path().join("v.kdbx");
        Fx {
            dir,
            core,
            fake,
            clock,
            path,
        }
    }

    /// An unlocked vault `v.kdbx` (password `pw`, cheap KDF) with no entries.
    fn fx_open() -> Fx {
        let f = fx();
        f.core
            .session
            .create_vault_with(&f.path, pw("pw"), fast_database("Test Vault"))
            .unwrap();
        f
    }

    impl Fx {
        fn root(&self) -> String {
            self.core.sidebar_get().unwrap().root_group_id
        }

        /// Saves a Login entry through the facade and returns its id.
        fn login(&self, title: &str, password: &str) -> String {
            let root = Uuid::parse_str(&self.root()).unwrap();
            let mut m = new_model(EntryTypeDto::Login, root);
            m.title = title.into();
            m.username = "user".into();
            m.password = password.into();
            self.core.entry_save(m).unwrap()
        }
    }

    fn code<T: std::fmt::Debug>(r: Result<T, ApiError>) -> ErrorCode {
        r.unwrap_err().code
    }

    #[test]
    fn api_error_mapping_table() {
        use crate::session::GeneratorRulesError as G;
        let cases: Vec<(SessionError, ErrorCode)> = vec![
            (SessionError::Locked, ErrorCode::Locked),
            (SessionError::NoVault, ErrorCode::NoVault),
            (SessionError::WrongPassword, ErrorCode::WrongPassword),
            (SessionError::AlreadyExists, ErrorCode::AlreadyExists),
            (SessionError::NotFound, ErrorCode::NotFound),
            (
                SessionError::InvalidInput("x".into()),
                ErrorCode::InvalidInput,
            ),
            (
                SessionError::LimitExceeded("x".into()),
                ErrorCode::LimitExceeded,
            ),
            (SessionError::Corrupt("x".into()), ErrorCode::Corrupt),
            (
                SessionError::Unsupported("x".into()),
                ErrorCode::Unsupported,
            ),
            (SessionError::ChangedOnDisk, ErrorCode::ChangedOnDisk),
            (SessionError::Io("x".into()), ErrorCode::Io),
            (SessionError::Quitting, ErrorCode::Quitting),
            (
                SessionError::GeneratorRules(G::NoClass),
                ErrorCode::GeneratorNoClass,
            ),
            (
                SessionError::GeneratorRules(G::Unsatisfiable),
                ErrorCode::GeneratorUnsatisfiable,
            ),
        ];
        for (error, expected) in cases {
            assert_eq!(ApiError::from(error.clone()).code, expected, "{error:?}");
        }
        // Generator failures are bare codes: the UI localises them.
        assert_eq!(
            ApiError::from(SessionError::GeneratorRules(G::NoClass)).message,
            ""
        );
        assert_eq!(
            ApiError::from(PlatformError::Unsupported("clipboard")).code,
            ErrorCode::Unsupported
        );
    }

    #[test]
    fn platform_error_to_api_error_covers_device_unlock_variants() {
        let cases = [
            (PlatformError::Cancelled, ErrorCode::Cancelled),
            (PlatformError::Lockout, ErrorCode::Lockout),
            (PlatformError::Invalidated, ErrorCode::Invalidated),
            (
                PlatformError::NotEnrolledOnDevice,
                ErrorCode::NotEnrolledOnDevice,
            ),
            (
                PlatformError::DeviceUnlockUnavailable,
                ErrorCode::DeviceUnlockUnavailable,
            ),
            (PlatformError::Failed("x".into()), ErrorCode::Internal),
        ];
        for (error, expected) in cases {
            assert_eq!(ApiError::from(error.clone()).code, expected, "{error:?}");
        }
    }

    #[test]
    fn api_error_serialises_with_camel_case_codes() {
        let e = ApiError {
            code: ErrorCode::WrongPassword,
            message: String::new(),
        };
        assert_eq!(
            serde_json::to_string(&e).unwrap(),
            r#"{"code":"wrongPassword","message":""}"#
        );
    }

    /// The arguments every call of the locked table shares.
    struct Args {
        root: String,
        entry: String,
        model: EntryEditModel,
        generate: GenerateRequest,
    }

    type Call = Box<dyn Fn(&AppCore, &Args) -> Result<(), ApiError>>;

    /// Every vault-touching call, with arguments that would otherwise succeed or fail with
    /// something other than `Locked`: the attachment and CSV paths do not exist, so a file read
    /// before the lock check would surface as `NotFound`.
    fn vault_calls() -> Vec<(&'static str, Call)> {
        let mut calls: Vec<(&'static str, Call)> = Vec::new();
        macro_rules! call {
            ($name:literal, $c:ident, $a:ident => $e:expr) => {
                calls.push((
                    $name,
                    Box::new(|$c: &AppCore, $a: &Args| {
                        let _ = $a;
                        $e.map(|_| ())
                    }),
                ))
            };
        }
        call!("sidebar_get", c, a => c.sidebar_get());
        call!("entry_list", c, a => c.entry_list(crate::dto::EntryQuery {
            scope: crate::dto::Scope::Category { category: crate::dto::Category::All },
            search: String::new(),
            sort: crate::dto::SortKey::Title,
        }));
        call!("entry_recent", c, a => c.entry_recent(5));
        call!("entry_get", c, a => c.entry_get(&a.entry));
        call!("entry_totp", c, a => c.entry_totp(&a.entry));
        call!("entry_reveal", c, a => c.entry_reveal(&a.entry, FieldRef::Password));
        call!("entry_copy", c, a => c.entry_copy(&a.entry, FieldRef::Password));
        call!("entry_new_model", c, a => c.entry_new_model(EntryTypeDto::Login, None));
        call!("entry_edit_model", c, a => c.entry_edit_model(&a.entry));
        call!("entry_save", c, a => c.entry_save(a.model.clone()));
        call!("entry_delete", c, a => c.entry_delete(&a.entry));
        call!("entry_restore", c, a => c.entry_restore(&a.entry, None));
        call!("entry_purge", c, a => c.entry_purge(&a.entry));
        call!("entry_empty_bin", c, a => c.entry_empty_bin());
        call!("entry_set_favorite", c, a => c.entry_set_favorite(&a.entry, true));
        call!("entry_move", c, a => c.entry_move(&a.entry, &a.root));
        call!("entry_touch", c, a => c.entry_touch(&a.entry));
        call!("entry_history", c, a => c.entry_history(&a.entry));
        call!("entry_history_restore", c, a => c.entry_history_restore(&a.entry, 0));
        call!("attachment_export", c, a => c.attachment_export(&a.entry, "a.txt", "/nonexistent/out.bin"));
        call!("group_create", c, a => c.group_create(None, "g", None));
        call!("group_update", c, a => c.group_update(&a.root, "g", None));
        call!("group_move", c, a => c.group_move(&a.entry, &a.root));
        call!("group_delete", c, a => c.group_delete(&a.entry));
        call!("draft_save", c, a => c.draft_save(&a.entry, "text"));
        call!("draft_versions", c, a => c.draft_versions(&a.entry));
        call!("draft_clear", c, a => c.draft_clear(&a.entry));
        call!("generator_history_list", c, a => c.generator_history_list());
        call!("generator_history_copy", c, a => c.generator_history_copy(&a.entry));
        call!("generator_history_reveal", c, a => c.generator_history_reveal(&a.entry));
        call!("generator_history_delete", c, a => c.generator_history_delete(&a.entry));
        call!("generator_generate+record", c, a => c.generator_generate(a.generate.clone()));
        call!("generator_copy+record", c, a => c.generator_copy("secret", Some("https://example.com")));
        call!("import_csv_commit", c, a => c.import_csv_commit("/nonexistent/in.csv", &a.root));
        call!("backup_restore", c, a => c.backup_restore("backup-1.kdbx"));
        calls
    }

    fn args(root: &str, entry: &str) -> Args {
        let mut model = new_model(EntryTypeDto::Login, Uuid::parse_str(root).unwrap());
        model.attachment_ops = vec![AttachmentOp::Add {
            path: "/nonexistent/file.bin".into(),
        }];
        Args {
            root: root.into(),
            entry: entry.into(),
            model,
            generate: GenerateRequest {
                options: GeneratorOptionsDto::default(),
                host: None,
                record_url: Some("https://example.com".into()),
            },
        }
    }

    #[test]
    fn every_vault_command_rejects_when_locked() {
        let f = fx_open();
        let entry = f.login("Mail", "hunter2");
        let root = f.root();
        f.core.vault_lock().unwrap();
        let a = args(&root, &entry);
        for (name, call) in vault_calls() {
            assert_eq!(code(call(&f.core, &a)), ErrorCode::Locked, "{name}");
        }
        // Without a record URL these never touch the vault, so they work while locked.
        let no_record = GenerateRequest {
            options: GeneratorOptionsDto::default(),
            host: None,
            record_url: None,
        };
        assert!(f.core.generator_generate(no_record).is_ok());
        assert!(f.core.generator_copy("pw", None).is_ok());
        assert!(f.core.generator_rules_summary("example.com").is_ok());
        assert_eq!(f.fake.clipboard.contents().as_deref(), Some("pw"));
    }

    #[test]
    fn every_mutating_command_rejects_when_quitting_and_reads_still_work() {
        let f = fx_open();
        let entry = f.login("Mail", "hunter2");
        let root = f.root();
        f.core.begin_quit();
        assert!(f.core.is_quitting());
        let model = new_model(EntryTypeDto::Login, Uuid::parse_str(&root).unwrap());
        assert_eq!(code(f.core.entry_save(model.clone())), ErrorCode::Quitting);
        assert_eq!(code(f.core.entry_delete(&entry)), ErrorCode::Quitting);
        assert_eq!(
            code(f.core.group_create(None, "g", None)),
            ErrorCode::Quitting
        );
        assert_eq!(
            code(f.core.vault_open("/nowhere.kdbx", pw("pw"))),
            ErrorCode::Quitting
        );
        assert_eq!(code(f.core.vault_unlock(pw("pw"))), ErrorCode::Quitting);
        assert_eq!(
            code(f.core.import_csv_commit("/nowhere.csv", &root)),
            ErrorCode::Quitting
        );
        assert_eq!(
            code(f.core.backup_restore("backup-1.kdbx")),
            ErrorCode::Quitting
        );
        assert!(f.core.entry_get(&entry).is_ok());
        assert!(f.core.sidebar_get().is_ok());
        assert!(f.core.vault_status().is_ok());
        f.core.vault_lock().unwrap();
        f.core.cancel_quit();
        assert!(!f.core.is_quitting());
        f.core.vault_unlock(pw("pw")).unwrap();
        assert!(f.core.entry_save(model).is_ok());
    }

    // ---- platform, info, requests ----

    /// A keyboard layout that reports nothing, like the fallback of a platform without one.
    struct NoLayout;

    impl mypass_platform::KeyboardLayout for NoLayout {
        fn current(&self) -> mypass_platform::PlatformResult<Option<String>> {
            Err(PlatformError::Unsupported("keyboard layout"))
        }
        fn subscribe(
            &self,
            _: Arc<dyn Fn(Option<String>) + Send + Sync>,
        ) -> mypass_platform::PlatformResult<mypass_platform::Subscription> {
            Err(PlatformError::Unsupported("keyboard layout"))
        }
    }

    /// Device unlock as the `Unsupported` fallback has it.
    struct NoDeviceUnlock;

    impl mypass_platform::DeviceUnlock for NoDeviceUnlock {
        fn availability(&self) -> DeviceUnlockAvailability {
            DeviceUnlockAvailability::Unavailable
        }
        fn is_enrolled(
            &self,
            _: mypass_platform::DeviceUnlockMode,
            _: &str,
        ) -> mypass_platform::PlatformResult<bool> {
            Err(PlatformError::Unsupported("device unlock"))
        }
        fn enroll(
            &self,
            _: mypass_platform::DeviceUnlockMode,
            _: &str,
            _: &SecretString,
        ) -> mypass_platform::PlatformResult<()> {
            Err(PlatformError::Unsupported("device unlock"))
        }
        fn unlock(
            &self,
            _: mypass_platform::DeviceUnlockMode,
            _: &str,
            _: &str,
        ) -> mypass_platform::PlatformResult<SecretString> {
            Err(PlatformError::Unsupported("device unlock"))
        }
        fn forget(&self, _: &str) -> mypass_platform::PlatformResult<()> {
            Err(PlatformError::Unsupported("device unlock"))
        }
    }

    /// A launch-at-login that reports `Unsupported` as an error, like a stub.
    struct NoLoginItem;

    impl mypass_platform::LaunchAtLogin for NoLoginItem {
        fn status(&self) -> mypass_platform::PlatformResult<mypass_platform::LoginItemStatus> {
            Err(PlatformError::Unsupported("launch at login"))
        }
        fn set_enabled(&self, _: bool) -> mypass_platform::PlatformResult<()> {
            Err(PlatformError::Unsupported("launch at login"))
        }
    }

    fn core_with(f: &Fx, platform: Platform) -> AppCore {
        AppCore::with_parts(
            f.core.paths().clone(),
            platform,
            f.clock.clone(),
            SettingsStore::in_memory(),
        )
    }

    #[test]
    fn platform_keyboard_layout_passes_label_and_hides_when_unsupported() {
        let f = fx();
        assert_eq!(
            f.core.platform_keyboard_layout().unwrap().as_deref(),
            Some("EN")
        );
        f.fake.keyboard_layout.set(Some("UK"));
        assert_eq!(
            f.core.platform_keyboard_layout().unwrap().as_deref(),
            Some("UK")
        );
        f.fake.keyboard_layout.set(None);
        assert_eq!(f.core.platform_keyboard_layout().unwrap(), None);
        let unsupported = core_with(
            &f,
            Platform {
                keyboard_layout: Arc::new(NoLayout),
                ..f.fake.platform.clone()
            },
        );
        assert_eq!(unsupported.platform_keyboard_layout().unwrap(), None);
    }

    #[test]
    fn settings_login_item_status_reports_fake_status() {
        let f = fx();
        assert_eq!(
            f.core.settings_login_item_status().unwrap(),
            LoginItemStatus::Disabled
        );
        for (platform, dto) in [
            (
                mypass_platform::LoginItemStatus::Enabled,
                LoginItemStatus::Enabled,
            ),
            (
                mypass_platform::LoginItemStatus::RequiresApproval,
                LoginItemStatus::RequiresApproval,
            ),
            (
                mypass_platform::LoginItemStatus::NotFound,
                LoginItemStatus::NotFound,
            ),
            (
                mypass_platform::LoginItemStatus::Unsupported,
                LoginItemStatus::Unsupported,
            ),
        ] {
            f.fake.launch_at_login.set_status(platform);
            assert_eq!(f.core.settings_login_item_status().unwrap(), dto);
        }
        // An implementation that fails with `Unsupported` is the same value, not an error.
        let stub = core_with(
            &f,
            Platform {
                launch_at_login: Arc::new(NoLoginItem),
                ..f.fake.platform.clone()
            },
        );
        assert_eq!(
            stub.settings_login_item_status().unwrap(),
            LoginItemStatus::Unsupported
        );
    }

    #[test]
    fn open_entry_request_unlocked_returns_event_locked_returns_none_and_unknown_id_is_not_found() {
        let f = fx_open();
        let id = f.login("Mail", "pw");
        assert_eq!(
            f.core.open_entry_request(&id, true).unwrap(),
            Some(OpenEntryEvent {
                id: id.clone(),
                edit: true
            })
        );
        assert_eq!(
            code(
                f.core
                    .open_entry_request(&Uuid::new_v4().simple().to_string(), false)
            ),
            ErrorCode::NotFound
        );
        assert_eq!(
            code(f.core.open_entry_request("not-an-id", false)),
            ErrorCode::InvalidInput
        );
        f.core.begin_quit();
        assert_eq!(
            code(f.core.open_entry_request(&id, false)),
            ErrorCode::Quitting
        );
        f.core.cancel_quit();
        f.core.vault_lock().unwrap();
        assert_eq!(f.core.open_entry_request(&id, true).unwrap(), None);
        // Locked wins over quitting.
        f.core.begin_quit();
        assert_eq!(f.core.open_entry_request(&id, true).unwrap(), None);
    }

    #[test]
    fn app_info_reports_device_unlock_available() {
        let f = fx();
        let info = f.core.app_info();
        assert!(info.device_unlock_available, "the fake has Touch ID");
        assert_eq!(info.os, std::env::consts::OS);
        assert!(!info.version.is_empty());
        let unsupported = core_with(
            &f,
            Platform {
                device_unlock: Arc::new(NoDeviceUnlock),
                ..f.fake.platform.clone()
            },
        );
        assert!(!unsupported.app_info().device_unlock_available);
        let account_only =
            mypass_platform::testing::FakeDeviceUnlock::new(DeviceUnlockAvailability::Available {
                biometrics: false,
            });
        let core = core_with(
            &f,
            Platform {
                device_unlock: account_only,
                ..f.fake.platform.clone()
            },
        );
        assert!(core.app_info().device_unlock_available);
    }

    #[test]
    fn app_core_exposes_the_one_scheduler_and_paths() {
        let f = fx();
        let injected: Arc<dyn Scheduler> = f.clock.clone();
        assert!(Arc::ptr_eq(&f.core.scheduler(), &injected));
        assert_eq!(f.core.paths(), &AppPaths::macos(f.dir.path()));
        assert!(Arc::ptr_eq(
            &f.core.platform().clipboard,
            &f.fake.platform.clipboard
        ));
    }

    #[test]
    fn production_constructor_builds_a_working_core() {
        let dir = tempfile::tempdir().unwrap();
        let fake = fake_platform();
        let core = AppCore::new(AppPaths::macos(dir.path()), fake.platform.clone()).unwrap();
        assert_eq!(
            core.vault_status().unwrap().state,
            crate::session::SessionState::NoVault
        );
        // The settings file lives under the config dir.
        core.settings_update(SettingsPatch {
            backup_count: Some(3),
            ..SettingsPatch::default()
        })
        .unwrap();
        assert!(core.paths().config_dir.join("settings.json").exists());
    }

    #[test]
    fn backup_folder_is_vault_scoped_and_created() {
        let f = fx();
        assert_eq!(code(f.core.backup_folder()), ErrorCode::NoVault);
        let f = fx_open();
        let folder = f.core.backup_folder().unwrap();
        let id = f.core.session.vault_id().unwrap();
        assert_eq!(folder, f.core.paths().backups_dir.join(&id));
        assert!(folder.is_dir());
    }

    #[test]
    fn vault_open_warnings_pass_through_guards() {
        struct Old(PathBuf);
        impl ProcessLookup for Old {
            fn other_app_executables(&self, bundle_id: &str) -> Vec<PathBuf> {
                assert_eq!(bundle_id, "com.mypass.app");
                vec![self.0.clone()]
            }
        }
        let mut f = fx();
        let icloud = f
            .core
            .paths()
            .home()
            .join("Library")
            .join("Mobile Documents")
            .join("v.kdbx");
        // Default lookup: no other processes.
        assert_eq!(
            f.core
                .vault_open_warnings(f.path.to_str().unwrap())
                .unwrap(),
            Vec::<OpenWarning>::new()
        );
        assert_eq!(
            f.core
                .vault_open_warnings(icloud.to_str().unwrap())
                .unwrap(),
            vec![OpenWarning::Icloud]
        );
        f.core
            .set_process_lookup(Arc::new(Old(PathBuf::from("/Applications/Old/MyPass"))));
        assert_eq!(
            f.core
                .vault_open_warnings(f.path.to_str().unwrap())
                .unwrap(),
            vec![OpenWarning::OldAppRunning {
                executable: "/Applications/Old/MyPass".into()
            }]
        );
    }

    #[test]
    fn vault_lifecycle_through_the_facade() {
        let f = fx();
        assert_eq!(
            f.core.vault_status().unwrap().state,
            crate::session::SessionState::NoVault
        );
        assert_eq!(code(f.core.sidebar_get()), ErrorCode::NoVault);
        // The default KDF is slow but real: one create, one open.
        let status = f
            .core
            .vault_create(f.path.to_str().unwrap(), pw("pw"), "Mine")
            .unwrap();
        assert_eq!(status.vault_name.as_deref(), Some("Mine"));
        assert_eq!(
            code(
                f.core
                    .vault_create(f.path.to_str().unwrap(), pw("pw"), "Mine")
            ),
            ErrorCode::AlreadyExists
        );
        f.core.vault_lock().unwrap();
        assert_eq!(
            code(f.core.vault_unlock(pw("wrong"))),
            ErrorCode::WrongPassword
        );
        assert_eq!(
            f.core.vault_unlock(pw("pw")).unwrap().state,
            crate::session::SessionState::Unlocked
        );
        assert_eq!(
            code(f.core.vault_open("/nonexistent/v.kdbx", pw("pw"))),
            ErrorCode::NotFound
        );
        let recent = f.core.vault_recent().unwrap();
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].file_name, "v.kdbx");
        assert!(recent[0].exists);
        f.core.vault_forget(&recent[0].path).unwrap();
        assert!(f.core.vault_recent().unwrap().is_empty());
        let roots = vec![f.dir.path().to_path_buf()];
        let found = f.core.vault_find_files(roots).unwrap();
        assert_eq!(found.len(), 1);
    }

    // ---- entries ----

    #[test]
    fn malformed_ids_are_invalid_input_when_unlocked() {
        let f = fx_open();
        let root = f.root();
        assert_eq!(code(f.core.entry_get("zzz")), ErrorCode::InvalidInput);
        assert_eq!(code(f.core.entry_delete("zzz")), ErrorCode::InvalidInput);
        assert_eq!(
            code(f.core.entry_move("zzz", &root)),
            ErrorCode::InvalidInput
        );
        assert_eq!(
            code(f.core.group_update("zzz", "x", None)),
            ErrorCode::InvalidInput
        );
        assert_eq!(code(f.core.draft_clear("zzz")), ErrorCode::InvalidInput);
        assert_eq!(
            code(f.core.import_csv_commit("/nowhere.csv", "zzz")),
            ErrorCode::InvalidInput
        );
        let unknown = Uuid::new_v4().simple().to_string();
        assert_eq!(code(f.core.entry_get(&unknown)), ErrorCode::NotFound);
    }

    #[test]
    fn entry_touch_marks_accessed_without_dirty_and_unknown_id_is_not_found() {
        let f = fx_open();
        let id = f.login("Mail", "pw");
        f.clock.advance(crate::saving::AUTOSAVE_DEBOUNCE);
        assert_eq!(f.core.session.status().save.state, SaveState::Idle);
        f.clock.advance(Duration::from_secs(100));
        let uuid = Uuid::parse_str(&id).unwrap();
        let before = f
            .core
            .session
            .with_db(|db| db.find_entry(uuid).map(|e| (e.accessed, e.usage_count)))
            .unwrap()
            .unwrap();
        f.core.entry_touch(&id).unwrap();
        let after = f
            .core
            .session
            .with_db(|db| db.find_entry(uuid).map(|e| (e.accessed, e.usage_count)))
            .unwrap()
            .unwrap();
        assert!(after.0 > before.0, "access time moved");
        assert_eq!(after.1, before.1 + 1);
        assert_eq!(f.core.session.status().save.state, SaveState::Idle);
        assert_eq!(
            code(f.core.entry_touch(&Uuid::new_v4().simple().to_string())),
            ErrorCode::NotFound
        );
    }

    #[test]
    fn entry_touch_does_not_schedule_save() {
        let f = fx_open();
        let id = f.login("Mail", "pw");
        f.clock.advance(crate::saving::AUTOSAVE_DEBOUNCE);
        let pending = f.clock.pending();
        let bytes = std::fs::read(&f.path).unwrap();
        f.core.entry_touch(&id).unwrap();
        assert_eq!(f.clock.pending(), pending, "no save job armed");
        f.clock.advance(Duration::from_secs(10));
        assert_eq!(
            std::fs::read(&f.path).unwrap(),
            bytes,
            "the file is untouched"
        );
    }

    // ---- clipboard ----

    #[test]
    fn entry_copy_puts_value_in_clipboard_and_returns_unit() {
        let f = fx_open();
        let id = f.login("Mail", "hunter2");
        let result: () = f.core.entry_copy(&id, FieldRef::Password).unwrap();
        assert_eq!(result, ());
        assert_eq!(f.fake.clipboard.contents().as_deref(), Some("hunter2"));
        f.core.entry_copy(&id, FieldRef::Username).unwrap();
        assert_eq!(f.fake.clipboard.contents().as_deref(), Some("user"));
    }

    #[test]
    fn entry_copy_clears_after_30s_via_manual_scheduler() {
        let f = fx_open();
        let id = f.login("Mail", "hunter2");
        f.core.entry_copy(&id, FieldRef::Password).unwrap();
        f.clock.advance(Duration::from_secs(29));
        assert_eq!(f.fake.clipboard.contents().as_deref(), Some("hunter2"));
        f.clock.advance(Duration::from_secs(1));
        assert_eq!(f.fake.clipboard.contents(), None);
    }

    #[test]
    fn copy_totp_puts_current_code_in_clipboard() {
        let f = fx_open();
        let root = Uuid::parse_str(&f.root()).unwrap();
        let mut m = new_model(EntryTypeDto::Login, root);
        m.title = "Codes".into();
        m.totp = "JBSWY3DPEHPK3PXP".into();
        let id = f.core.entry_save(m).unwrap();
        let expected = mypass_core::totp::TotpConfig::parse("JBSWY3DPEHPK3PXP")
            .unwrap()
            .code_at(T0)
            .unwrap();
        let shown = f.core.entry_totp(&id).unwrap();
        assert_eq!(shown.code, expected, "uses the injected clock");
        f.core.entry_copy(&id, FieldRef::Totp).unwrap();
        assert_eq!(f.fake.clipboard.contents(), Some(expected.clone()));
        assert_eq!(f.core.entry_reveal(&id, FieldRef::Totp).unwrap(), expected);
        // An entry without a TOTP is not found as one.
        let plain = f.login("Plain", "x");
        assert_eq!(code(f.core.entry_totp(&plain)), ErrorCode::InvalidInput);
    }

    #[test]
    fn generator_copy_copies_value_and_records_history_when_url_given() {
        let f = fx_open();
        f.core
            .generator_copy("Gen#Pass-1", Some("https://login.example.com/path"))
            .unwrap();
        assert_eq!(f.fake.clipboard.contents().as_deref(), Some("Gen#Pass-1"));
        let history = f.core.generator_history_list().unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].title, "login.example.com");
        assert_eq!(
            f.core.generator_history_reveal(&history[0].id).unwrap(),
            "Gen#Pass-1"
        );
        f.core.generator_history_copy(&history[0].id).unwrap();
        f.core.generator_copy("Other", None).unwrap();
        assert_eq!(f.core.generator_history_list().unwrap().len(), 1);
    }

    #[test]
    fn generator_copy_does_not_copy_when_recording_fails() {
        let f = fx_open();
        f.core.vault_lock().unwrap();
        assert_eq!(
            code(f.core.generator_copy("secret", Some("https://example.com"))),
            ErrorCode::Locked
        );
        assert_eq!(f.fake.clipboard.contents(), None, "nothing was copied");
        f.core.vault_unlock(pw("pw")).unwrap();
        f.core.begin_quit();
        assert_eq!(
            code(f.core.generator_copy("secret", Some("https://example.com"))),
            ErrorCode::Quitting
        );
        assert_eq!(f.fake.clipboard.contents(), None);
    }

    // ---- drafts ----

    fn draft_files(f: &Fx) -> usize {
        let id = VaultId::new(&f.core.session.vault_id().unwrap()).unwrap();
        std::fs::read_dir(f.core.paths().drafts_dir(&id))
            .map(|d| {
                d.filter_map(Result::ok)
                    .filter(|e| e.path().extension().is_some_and(|x| x == "mpd"))
                    .count()
            })
            .unwrap_or(0)
    }

    fn has_draft_key(path: &Path) -> bool {
        let (_, db) = mypass_kdbx::VaultFile::open(path, &pw("pw")).unwrap();
        db.meta
            .custom_data
            .iter()
            .any(|i| i.key == drafts::DRAFT_KEY_CUSTOM_DATA)
    }

    fn notes_entry(f: &Fx, title: &str, notes: &str) -> String {
        let root = Uuid::parse_str(&f.root()).unwrap();
        let mut m = new_model(EntryTypeDto::Notes, root);
        m.title = title.into();
        m.notes = notes.into();
        f.core.entry_save(m).unwrap()
    }

    #[test]
    fn draft_read_paths_never_modify_the_vault() {
        let f = fx_open();
        let id = f.login("Mail", "pw");
        f.clock.advance(crate::saving::AUTOSAVE_DEBOUNCE);
        assert!(f.core.draft_versions(&id).unwrap().is_empty());
        f.core.draft_clear(&id).unwrap();
        assert_eq!(f.core.session.status().save.state, SaveState::Idle);
        assert!(!has_draft_key(&f.path));
        assert_eq!(draft_files(&f), 0);
    }

    #[test]
    fn draft_save_and_versions_round_trip() {
        let f = fx_open();
        let id = notes_entry(&f, "Diary", "saved");
        assert!(f.core.draft_save(&id, "first draft").unwrap());
        f.clock.advance(Duration::from_secs(5));
        assert!(f.core.draft_save(&id, "second draft").unwrap());
        let versions = f.core.draft_versions(&id).unwrap();
        assert_eq!(
            versions
                .iter()
                .map(|v| v.notes.as_str())
                .collect::<Vec<_>>(),
            ["second draft", "first draft"]
        );
        f.core.draft_clear(&id).unwrap();
        assert!(f.core.draft_versions(&id).unwrap().is_empty());
    }

    #[test]
    fn first_draft_save_flushes_the_key_before_writing_the_draft() {
        let f = fx_open();
        let io = Arc::new(FailingFileIo::new(Arc::new(PlainFileIo)));
        f.core.session.set_file_io(io.clone());
        let id = notes_entry(&f, "Diary", "saved");
        f.clock.advance(crate::saving::AUTOSAVE_DEBOUNCE);

        // The vault cannot be written: no draft, and no draft file.
        io.fail_write_atomic(std::io::ErrorKind::Other);
        assert!(!f.core.draft_save(&id, "unsafe draft").unwrap());
        assert_eq!(draft_files(&f), 0);
        // The key now exists in memory only; a second try must not trust it.
        assert!(!f.core.draft_save(&id, "unsafe draft").unwrap());
        assert_eq!(draft_files(&f), 0);

        io.heal();
        assert!(f.core.draft_save(&id, "safe draft").unwrap());
        assert_eq!(draft_files(&f), 1);
        // Open the file from disk independently: the key is there.
        assert!(has_draft_key(&f.path));
    }

    #[test]
    fn unlock_purges_expired_drafts_but_keeps_fresh_ones() {
        let f = fx_open();
        let id = notes_entry(&f, "Diary", "saved");
        assert!(f.core.draft_save(&id, "draft").unwrap());
        // A quick lock and unlock: the draft is fresh and stays.
        f.core.vault_lock().unwrap();
        f.core.vault_unlock(pw("pw")).unwrap();
        assert_eq!(draft_files(&f), 1);
        // Past the TTL while locked: the next unlock drops it.
        f.core.vault_lock().unwrap();
        f.clock
            .advance(drafts::DRAFT_TTL + Duration::from_secs(3600));
        f.core.vault_unlock(pw("pw")).unwrap();
        assert_eq!(draft_files(&f), 0);
    }

    #[test]
    fn draft_save_for_an_unknown_entry_is_not_found_and_writes_nothing() {
        let f = fx_open();
        let stray = Uuid::new_v4().simple().to_string();
        assert_eq!(code(f.core.draft_save(&stray, "text")), ErrorCode::NotFound);
        assert_eq!(draft_files(&f), 0);
        assert!(!has_draft_key(&f.path), "no key was created for a stray id");
    }

    #[test]
    fn draft_save_keeps_working_while_vault_saves_fail_once_the_key_is_on_disk() {
        let f = fx_open();
        let io = Arc::new(FailingFileIo::new(Arc::new(PlainFileIo)));
        f.core.session.set_file_io(io.clone());
        let id = notes_entry(&f, "Diary", "saved");
        assert!(f.core.draft_save(&id, "first").unwrap());
        f.clock.advance(Duration::from_secs(5));
        assert!(has_draft_key(&f.path), "the key is on disk");
        // Saves now fail, and an edit is pending.
        io.fail_write_atomic(std::io::ErrorKind::Other);
        f.login("Pending", "x");
        assert_ne!(f.core.session.status().save.state, SaveState::Idle);
        assert!(f.core.draft_save(&id, "second").unwrap());
        assert_eq!(f.core.draft_versions(&id).unwrap().len(), 2);
        assert_eq!(draft_files(&f), 1, "one file per entry");
    }

    #[test]
    fn key_created_while_saves_fail_is_flushed_once_the_io_recovers() {
        let f = fx_open();
        let io = Arc::new(FailingFileIo::new(Arc::new(PlainFileIo)));
        f.core.session.set_file_io(io.clone());
        let id = notes_entry(&f, "Diary", "saved");
        f.clock.advance(crate::saving::AUTOSAVE_DEBOUNCE);
        io.fail_write_atomic(std::io::ErrorKind::Other);
        assert!(!f.core.draft_save(&id, "one").unwrap());
        assert_eq!(draft_files(&f), 0);
        io.heal();
        assert!(f.core.draft_save(&id, "two").unwrap());
        assert_eq!(draft_files(&f), 1);
        assert!(has_draft_key(&f.path));
        // The key is persisted now: a later failure does not stop drafts.
        io.fail_write_atomic(std::io::ErrorKind::Other);
        f.login("Pending", "x");
        assert!(f.core.draft_save(&id, "three").unwrap());
    }

    #[test]
    fn notes_save_clears_drafts_for_that_entry() {
        let f = fx_open();
        let notes = notes_entry(&f, "Diary", "v1");
        let other = notes_entry(&f, "Other", "o1");
        assert!(f.core.draft_save(&notes, "draft").unwrap());
        assert!(f.core.draft_save(&other, "other draft").unwrap());
        let mut m = f.core.entry_edit_model(&notes).unwrap();
        m.notes = "v2".into();
        f.core.entry_save(m).unwrap();
        assert!(f.core.draft_versions(&notes).unwrap().is_empty());
        assert_eq!(f.core.draft_versions(&other).unwrap().len(), 1);
        // A login save leaves other drafts alone.
        let login = f.login("Mail", "pw");
        let mut m = f.core.entry_edit_model(&login).unwrap();
        m.password = "changed".into();
        f.core.entry_save(m).unwrap();
        assert_eq!(draft_files(&f), 1);
    }

    #[test]
    fn delete_entry_clears_its_drafts() {
        let f = fx_open();
        let id = notes_entry(&f, "Diary", "v1");
        let keep = notes_entry(&f, "Keep", "k");
        assert!(f.core.draft_save(&id, "draft").unwrap());
        assert!(f.core.draft_save(&keep, "kept draft").unwrap());
        f.core.entry_delete(&id).unwrap();
        assert!(f.core.draft_versions(&id).unwrap().is_empty());
        assert_eq!(f.core.draft_versions(&keep).unwrap().len(), 1);
    }

    #[test]
    fn purge_empty_bin_and_group_delete_clear_drafts() {
        let f = fx_open();
        let a = notes_entry(&f, "A", "a");
        let b = notes_entry(&f, "B", "b");
        let group = f.core.group_create(None, "Folder", None).unwrap();
        let mut m = f.core.entry_edit_model(&notes_entry(&f, "C", "c")).unwrap();
        m.group_id = group.clone();
        let c = f.core.entry_save(m).unwrap();
        for id in [&a, &b, &c] {
            assert!(f.core.draft_save(id, "draft").unwrap());
        }
        assert_eq!(draft_files(&f), 3);
        f.core.group_delete(&group).unwrap();
        assert!(f.core.draft_versions(&c).unwrap().is_empty());
        f.core.entry_delete(&a).unwrap();
        f.core.entry_purge(&a).unwrap();
        assert_eq!(draft_files(&f), 1, "only B's draft is left");
        assert!(f.core.draft_save(&c, "again").unwrap());
        f.core.entry_delete(&b).unwrap();
        assert_eq!(f.core.entry_empty_bin().unwrap(), 2);
        assert_eq!(draft_files(&f), 0);
    }

    // ---- backups ----

    /// Two saved states, so one backup (the first state) exists, and a draft on top.
    fn fx_with_backup_and_draft() -> (Fx, String) {
        let f = fx_open();
        let id = notes_entry(&f, "First", "one");
        f.clock.advance(Duration::from_secs(2));
        f.login("Second", "two");
        f.clock.advance(Duration::from_secs(2));
        assert!(f.core.draft_save(&id, "a draft").unwrap());
        assert!(!f.core.backup_list().unwrap().items.is_empty());
        (f, id)
    }

    #[test]
    fn backup_restore_clears_drafts() {
        let (f, id) = fx_with_backup_and_draft();
        assert_eq!(draft_files(&f), 1);
        let backup = f.core.backup_list().unwrap().items[0].id.clone();
        f.core.backup_restore(&backup).unwrap();
        assert_eq!(draft_files(&f), 0);
        assert!(f.core.draft_versions(&id).unwrap().is_empty());
    }

    #[test]
    fn backup_restore_with_failing_reload_still_clears_drafts_and_locks() {
        let (f, id) = fx_with_backup_and_draft();
        let io = Arc::new(FailingFileIo::new(Arc::new(PlainFileIo)));
        f.core.session.set_file_io(io.clone());
        let backup = f.core.backup_list().unwrap().items[0].id.clone();
        io.fail_read(std::io::ErrorKind::Other);
        let result = f.core.backup_restore(&backup);
        io.heal();
        assert!(result.is_err());
        // The write happened and the reload failed: drafts gone, session locked.
        assert_eq!(draft_files(&f), 0);
        assert!(!f.core.session.is_unlocked());
        f.core.vault_unlock(pw("pw")).unwrap();
        assert!(f.core.draft_versions(&id).unwrap().is_empty());
    }

    #[test]
    fn restore_of_backup_with_other_password_is_rejected_and_current_untouched() {
        let (f, id) = fx_with_backup_and_draft();
        let folder = f.core.backup_folder().unwrap();
        let foreign = folder.join("backup-1600000000.kdbx");
        std::fs::write(
            &foreign,
            fast_database("Foreign")
                .save(&pw("another password"))
                .unwrap(),
        )
        .unwrap();
        let file = std::fs::read(&f.path).unwrap();
        let live = f.core.session.with_db(Clone::clone).unwrap();
        let backups = f.core.backup_list().unwrap();
        let versions = f.core.draft_versions(&id).unwrap();
        assert_eq!(draft_files(&f), 1);

        let error = f.core.backup_restore("backup-1600000000.kdbx").unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidInput);
        assert!(error.message.contains("master password"), "{error}");

        assert_eq!(std::fs::read(&f.path).unwrap(), file, "the vault file");
        assert_eq!(
            f.core.session.with_db(Clone::clone).unwrap(),
            live,
            "the open database"
        );
        assert_eq!(f.core.backup_list().unwrap(), backups, "the backup folder");
        assert_eq!(f.core.draft_versions(&id).unwrap(), versions);
        assert_eq!(draft_files(&f), 1);
        assert_eq!(f.core.session.status().save.state, SaveState::Idle);
        // An unknown id and a path-like id are refused too.
        assert_eq!(
            code(f.core.backup_restore("backup-1.kdbx")),
            ErrorCode::NotFound
        );
        assert_eq!(
            code(f.core.backup_restore("../v.kdbx")),
            ErrorCode::InvalidInput
        );
    }

    #[test]
    fn backup_restore_snapshots_current_file_first() {
        let (f, _id) = fx_with_backup_and_draft();
        let current = std::fs::read(&f.path).unwrap();
        let oldest = f
            .core
            .backup_list()
            .unwrap()
            .items
            .last()
            .unwrap()
            .id
            .clone();
        f.core.backup_restore(&oldest).unwrap();
        let vault_id = f.core.session.vault_id().unwrap();
        let kept: Vec<Vec<u8>> = f
            .core
            .backup_list()
            .unwrap()
            .items
            .iter()
            .map(|b| f.core.backups.read(&vault_id, &b.id).unwrap())
            .collect();
        assert!(
            kept.contains(&current),
            "the replaced file is among the backups"
        );
        assert_ne!(std::fs::read(&f.path).unwrap(), current);
        assert_eq!(f.core.session.status().save.state, SaveState::Idle);
    }

    // ---- settings, activity ----

    #[test]
    fn settings_update_rearms_autolock() {
        let f = fx_open();
        let updated = f
            .core
            .settings_update(SettingsPatch {
                auto_lock_idle_timeout: Some(crate::settings::IdleTimeout::OneMinute),
                ..SettingsPatch::default()
            })
            .unwrap();
        assert_eq!(
            updated.auto_lock_idle_timeout,
            crate::settings::IdleTimeout::OneMinute
        );
        assert_eq!(f.core.settings_get().unwrap(), updated);
        f.clock.advance(Duration::from_secs(61));
        let status = f.core.vault_status().unwrap();
        assert_eq!(status.state, crate::session::SessionState::Locked);
        assert_eq!(status.lock_reason, Some(LockReason::Idle));
    }

    #[test]
    fn app_record_activity_resets_idle_and_is_ignored_while_locked() {
        let f = fx_open();
        f.core
            .settings_update(SettingsPatch {
                auto_lock_idle_timeout: Some(crate::settings::IdleTimeout::OneMinute),
                ..SettingsPatch::default()
            })
            .unwrap();
        f.clock.advance(Duration::from_secs(40));
        f.core.app_record_activity().unwrap();
        f.clock.advance(Duration::from_secs(40));
        assert!(
            f.core.session.is_unlocked(),
            "activity restarted the minute"
        );
        f.clock.advance(Duration::from_secs(25));
        assert!(!f.core.session.is_unlocked());
        let pending = f.clock.pending();
        f.core.app_record_activity().unwrap();
        assert_eq!(f.clock.pending(), pending, "no timer armed while locked");
    }

    // ---- CSV, import ----

    #[test]
    fn csv_errors_map_without_content() {
        assert_eq!(
            ApiError::from(CsvImportError::TooLarge).code,
            ErrorCode::LimitExceeded
        );
        for e in [
            CsvImportError::NotUtf8,
            CsvImportError::Empty,
            CsvImportError::MissingRequiredColumn("title".into()),
        ] {
            assert_eq!(ApiError::from(e).code, ErrorCode::InvalidInput);
        }
        assert_eq!(
            ApiError::from(CsvImportError::MissingRequiredColumn("title".into())).message,
            "missing required column title"
        );
        // The import dialog tells these causes apart by this English text (the code is the same
        // `InvalidInput` for all of them), so a rewording must change the UI in the same commit.
        assert_eq!(
            ApiError::from(CsvImportError::NotUtf8).message,
            "file is not valid UTF-8"
        );
        assert_eq!(
            ApiError::from(CsvImportError::Empty).message,
            "file is empty"
        );
    }

    #[test]
    fn import_preview_needs_no_unlock_and_commit_needs_one() {
        let f = fx_open();
        let csv = f.dir.path().join("in.csv");
        std::fs::write(
            &csv,
            "title,url,username,password\nA,https://a.example,u,secret-cell\n",
        )
        .unwrap();
        let root = f.root();
        f.core.vault_lock().unwrap();
        let preview = f.core.import_csv_preview(csv.to_str().unwrap()).unwrap();
        assert_eq!(preview.total, 1);
        assert_eq!(
            code(f.core.import_csv_commit(csv.to_str().unwrap(), &root)),
            ErrorCode::Locked
        );
        f.core.vault_unlock(pw("pw")).unwrap();
        let result = f
            .core
            .import_csv_commit(csv.to_str().unwrap(), &root)
            .unwrap();
        assert_eq!((result.imported, result.skipped), (1, 0));
        // Bad files: the error names no cell.
        std::fs::write(&csv, "a,b\n1,secret-cell\n").unwrap();
        let error = f
            .core
            .import_csv_preview(csv.to_str().unwrap())
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidInput);
        assert!(!error.message.contains("secret-cell"));
        assert_eq!(
            code(f.core.import_csv_preview("/nonexistent/in.csv")),
            ErrorCode::NotFound
        );
    }

    #[test]
    fn export_attachment_while_locked_writes_nothing() {
        let f = fx_open();
        let id = f.login("Mail", "pw");
        f.core.vault_lock().unwrap();
        let out = f.dir.path().join("out.bin");
        assert_eq!(
            code(
                f.core
                    .attachment_export(&id, "a.txt", out.to_str().unwrap())
            ),
            ErrorCode::Locked
        );
        assert!(!out.exists());
    }
}
