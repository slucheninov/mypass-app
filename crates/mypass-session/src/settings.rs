//! The one persisted settings file and its store.
//!
//! [`Settings`] holds EVERY persisted field of the app: later stages add no field elsewhere and
//! never keep a second settings file. It carries a struct-level `#[serde(default)]`, so a file
//! written by an older build (missing fields) loads with each missing field at its default, and
//! unknown old fields (for example the dropped `trayClickAction`) are ignored. Loading is also
//! field-tolerant: one field with a bad value falls back to its default alone (see
//! [`SettingsStore::load`]), and a file that cannot be read is never written over. The schema is
//! pinned by `settings_schema_snapshot` against `tests/snapshots/settings.default.json`.

use crate::scheduler::lock;
use crate::shortcut;
use serde::{Deserialize, Deserializer, Serialize};
use std::fs;
use std::io::ErrorKind;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// Auto-lock after this much inactivity.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub enum IdleTimeout {
    Immediately,
    OneMinute,
    FiveMinutes,
    FifteenMinutes,
    #[default]
    ThirtyMinutes,
    OneHour,
    FourHours,
    Never,
}

impl IdleTimeout {
    /// The timeout in seconds; `None` for `Never`. `Immediately` is 2 s, not 0: a literal 0 would
    /// lock before the unlock transition finishes (the old app's `AutoLockIdleTimeout` did the
    /// same).
    pub fn seconds(self) -> Option<u64> {
        match self {
            Self::Immediately => Some(2),
            Self::OneMinute => Some(60),
            Self::FiveMinutes => Some(300),
            Self::FifteenMinutes => Some(900),
            Self::ThirtyMinutes => Some(1800),
            Self::OneHour => Some(3600),
            Self::FourHours => Some(14_400),
            Self::Never => None,
        }
    }
}

/// Minimum time between two backup snapshots of the same vault.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub enum BackupInterval {
    #[default]
    EverySave,
    TenMinutes,
    Hourly,
    Daily,
}

impl BackupInterval {
    /// The interval in seconds (`EverySave` is 0).
    pub fn seconds(self) -> u64 {
        match self {
            Self::EverySave => 0,
            Self::TenMinutes => 600,
            Self::Hourly => 3600,
            Self::Daily => 86_400,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub enum Language {
    #[default]
    System,
    En,
    Uk,
    Ru,
}

/// How long a device unlock (Touch ID / account password) stays allowed after a master-password
/// unlock. Names mirror the old `UnlockWindowPolicy`; the seconds table and the policy are
/// stage 5.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub enum UnlockWithDeviceWindow {
    OneHour,
    FourHours,
    #[default]
    EightHours,
    TwentyFourHours,
    Never,
}

/// How often the master password is required again regardless of device unlock (stage 5).
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub enum RequireAccountPasswordWindow {
    OneDay,
    SevenDays,
    FourteenDays,
    #[default]
    ThirtyDays,
    NinetyDays,
    Never,
}

/// Generator defaults remembered between sessions (a partial object fills the rest from the
/// defaults).
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
#[serde(default, rename_all = "camelCase")]
pub struct GeneratorOptionsDto {
    pub length: u32,
    pub lowercase: bool,
    pub uppercase: bool,
    pub digits: bool,
    pub symbols: bool,
}

impl Default for GeneratorOptionsDto {
    fn default() -> Self {
        Self {
            length: 20,
            lowercase: true,
            uppercase: true,
            digits: true,
            symbols: true,
        }
    }
}

/// Accepted generator lengths.
pub const GENERATOR_LENGTH: std::ops::RangeInclusive<u32> = 8..=64;
/// Accepted backup counts (values outside are clamped).
pub const BACKUP_COUNT: std::ops::RangeInclusive<u32> = 1..=50;
/// Clipboard clear delay range in seconds (clamped, then rounded to a multiple of 5).
pub const CLIPBOARD_CLEAR_SECONDS: std::ops::RangeInclusive<u32> = 10..=120;
/// Maximum length of the recent-vaults list.
pub const MAX_RECENT_VAULTS: usize = 8;

/// Every persisted setting. Exactly one `Default` impl; there is no tray-click-action setting.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    pub language: Language,
    pub theme: Theme,
    pub auto_lock_idle_timeout: IdleTimeout,
    pub lock_on_device_sleep: bool,
    pub lock_on_screen_lock: bool,
    /// 10..=120 in steps of 5; no "never" (the old stepper).
    pub clipboard_clear_seconds: u32,
    /// 1..=50.
    pub backup_count: u32,
    pub backup_interval: BackupInterval,
    pub launch_at_login: bool,
    pub show_menu_bar_extra: bool,
    /// Canonical accelerator (see `shortcut`), or empty for none.
    pub shortcut_lock: String,
    pub shortcut_show: String,
    pub generator: GeneratorOptionsDto,
    pub passkey_proxy_enabled: bool,
    pub unlock_touch_id_enabled: bool,
    pub unlock_mac_password_enabled: bool,
    pub unlock_with_device_window: UnlockWithDeviceWindow,
    pub require_account_password_window: RequireAccountPasswordWindow,
    /// Stage 5 validates and uses it.
    pub secondary_folder: Option<String>,
    pub update_check_enabled: bool,
    pub vault_path: Option<String>,
    /// Most recent first, unique, at most [`MAX_RECENT_VAULTS`].
    pub recent_vaults: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            language: Language::System,
            theme: Theme::System,
            auto_lock_idle_timeout: IdleTimeout::ThirtyMinutes,
            lock_on_device_sleep: true,
            lock_on_screen_lock: true,
            clipboard_clear_seconds: 30,
            backup_count: 10,
            backup_interval: BackupInterval::EverySave,
            launch_at_login: false,
            show_menu_bar_extra: true,
            shortcut_lock: "CommandOrControl+Shift+L".to_owned(),
            shortcut_show: String::new(),
            generator: GeneratorOptionsDto::default(),
            passkey_proxy_enabled: true,
            unlock_touch_id_enabled: false,
            unlock_mac_password_enabled: false,
            unlock_with_device_window: UnlockWithDeviceWindow::EightHours,
            require_account_password_window: RequireAccountPasswordWindow::ThirtyDays,
            secondary_folder: None,
            update_check_enabled: true,
            vault_path: None,
            recent_vaults: Vec::new(),
        }
    }
}

fn clamp_clipboard_seconds(v: u32) -> u32 {
    let v = v.clamp(
        *CLIPBOARD_CLEAR_SECONDS.start(),
        *CLIPBOARD_CLEAR_SECONDS.end(),
    );
    // Nearest multiple of 5 (12 -> 10, 13 -> 15); the clamped range keeps the result inside it.
    (v + 2) / 5 * 5
}

impl Settings {
    /// Repairs out-of-range values without failing: runs on load (a hand-edited or old file) and
    /// on every update. Invalid shortcuts are left as stored (the hotkey layer reports them).
    fn normalize(&mut self) {
        self.clipboard_clear_seconds = clamp_clipboard_seconds(self.clipboard_clear_seconds);
        self.backup_count = self
            .backup_count
            .clamp(*BACKUP_COUNT.start(), *BACKUP_COUNT.end());
        let g = &mut self.generator;
        g.length = g
            .length
            .clamp(*GENERATOR_LENGTH.start(), *GENERATOR_LENGTH.end());
        if !(g.lowercase || g.uppercase || g.digits || g.symbols) {
            *g = GeneratorOptionsDto {
                length: g.length,
                ..GeneratorOptionsDto::default()
            };
        }
        for s in [&mut self.shortcut_lock, &mut self.shortcut_show] {
            if let Ok(c) = shortcut::canonical(s) {
                *s = c;
            }
        }
        let mut seen = Vec::with_capacity(self.recent_vaults.len());
        self.recent_vaults.retain(|p| {
            let fresh = !seen.contains(p);
            seen.push(p.clone());
            fresh
        });
        self.recent_vaults.truncate(MAX_RECENT_VAULTS);
    }
}

/// Distinguishes a missing field (outer `None`: untouched) from `null` (`Some(None)`: clear).
fn double_option<'de, D, T>(d: D) -> Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(d).map(Some)
}

/// A partial update: every field of [`Settings`] except `recent_vaults` (owned by
/// [`SettingsStore::remember_vault`] / [`SettingsStore::forget_vault`]). `None` leaves a field
/// untouched; `vault_path` and `secondary_folder` use `Some(None)` (JSON `null`) to clear;
/// `generator` replaces the whole options object.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct SettingsPatch {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Language>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub theme: Option<Theme>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_lock_idle_timeout: Option<IdleTimeout>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lock_on_device_sleep: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lock_on_screen_lock: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clipboard_clear_seconds: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backup_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backup_interval: Option<BackupInterval>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub launch_at_login: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_menu_bar_extra: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shortcut_lock: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shortcut_show: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generator: Option<GeneratorOptionsDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub passkey_proxy_enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unlock_touch_id_enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unlock_mac_password_enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unlock_with_device_window: Option<UnlockWithDeviceWindow>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub require_account_password_window: Option<RequireAccountPasswordWindow>,
    #[serde(
        deserialize_with = "double_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub secondary_folder: Option<Option<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub update_check_enabled: Option<bool>,
    #[serde(
        deserialize_with = "double_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub vault_path: Option<Option<String>>,
}

/// Why [`SettingsStore::update`] refused or could not persist a change.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SettingsError {
    /// The named field (camelCase wire name, or `generator.length` / `generator.classes`) holds a
    /// value that cannot be repaired; nothing was changed.
    #[error("invalid value for {0}")]
    InvalidValue(&'static str),
    /// The change is applied in memory but the file could not be written.
    #[error("settings could not be saved: {0}")]
    Io(String),
}

struct StoreInner {
    path: Option<PathBuf>,
    /// The file existed but could not be read at load: it is never written over (see `load`).
    degraded: bool,
    settings: Mutex<Settings>,
}

/// The settings handle shared by the whole app (cheap to clone). Writes are serialised by the
/// internal lock and go through the atomic `write_atomic` of the vault engine.
#[derive(Clone)]
pub struct SettingsStore(Arc<StoreInner>);

impl std::fmt::Debug for SettingsStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SettingsStore")
            .field("path", &self.0.path)
            .finish_non_exhaustive()
    }
}

impl SettingsStore {
    /// Loads `path`.
    ///
    /// - A missing file gives the defaults (nothing is written until the first change).
    /// - Loading is field-tolerant: a field whose value does not deserialize (an unknown enum
    ///   value, a wrong type) falls back to its default ALONE, nested objects (`generator`) field
    ///   by field, and unknown fields are ignored, so one bad value never costs `vaultPath` or the
    ///   recent vaults.
    /// - Only a file that is not a JSON object at all gives the defaults and is renamed
    ///   `<name>.bad` (once: afterwards the file is gone, so the next load finds no file).
    /// - Any other read failure (permissions, a directory in the way, a transient I/O error)
    ///   gives the defaults in memory and marks the store degraded: the file is left alone and
    ///   never written over for the life of this store; every persist returns
    ///   [`SettingsError::Io`] (changes still apply in memory), and the next start reads the file
    ///   again. Chosen over retrying the read before the first write because a retry that
    ///   succeeds would have to merge the file with the in-memory changes.
    pub fn load(path: PathBuf) -> Self {
        let (settings, degraded) = match fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice::<serde_json::Value>(&bytes) {
                Ok(serde_json::Value::Object(fields)) => {
                    let mut s = tolerant_settings(fields);
                    s.normalize();
                    (s, false)
                }
                _ => {
                    let mut bad = path.clone().into_os_string();
                    bad.push(".bad");
                    // Best effort: if even the rename fails, the next save overwrites the file.
                    let _ = fs::rename(&path, PathBuf::from(bad));
                    (Settings::default(), false)
                }
            },
            Err(e) if e.kind() == ErrorKind::NotFound => (Settings::default(), false),
            Err(_) => (Settings::default(), true),
        };
        Self(Arc::new(StoreInner {
            path: Some(path),
            degraded,
            settings: Mutex::new(settings),
        }))
    }

    /// `true` when the settings file existed but could not be read at load: the store runs on
    /// defaults and refuses to write over that file (see [`SettingsStore::load`]).
    pub fn is_degraded(&self) -> bool {
        self.0.degraded
    }

    /// A store that never touches the disk (tests and previews).
    pub fn in_memory() -> Self {
        Self(Arc::new(StoreInner {
            path: None,
            degraded: false,
            settings: Mutex::new(Settings::default()),
        }))
    }

    /// A copy of the current settings.
    pub fn get(&self) -> Settings {
        lock(&self.0.settings).clone()
    }

    /// Applies `patch`. Out-of-range counts and delays are clamped; an invalid generator or
    /// shortcut is refused with [`SettingsError::InvalidValue`] and changes nothing. A failed write
    /// keeps the new value in memory and returns [`SettingsError::Io`].
    pub fn update(&self, patch: SettingsPatch) -> Result<Settings, SettingsError> {
        let mut guard = lock(&self.0.settings);
        let mut next = guard.clone();
        apply_patch(&mut next, patch)?;
        next.normalize();
        *guard = next.clone();
        self.persist(&guard)?;
        Ok(next)
    }

    /// Moves `path` to the front of the recent list (unique, at most 8). Persisting is best
    /// effort: the in-memory list is always updated.
    pub fn remember_vault(&self, path: &str) {
        let mut guard = lock(&self.0.settings);
        guard.recent_vaults.retain(|p| p != path);
        guard.recent_vaults.insert(0, path.to_owned());
        guard.recent_vaults.truncate(MAX_RECENT_VAULTS);
        let _ = self.persist(&guard);
    }

    /// Removes `path` from the recent list and clears `vault_path` when it equals `path`.
    /// Persisting is best effort.
    pub fn forget_vault(&self, path: &str) {
        let mut guard = lock(&self.0.settings);
        guard.recent_vaults.retain(|p| p != path);
        if guard.vault_path.as_deref() == Some(path) {
            guard.vault_path = None;
        }
        let _ = self.persist(&guard);
    }

    fn persist(&self, settings: &Settings) -> Result<(), SettingsError> {
        let Some(path) = &self.0.path else {
            return Ok(());
        };
        let io = |e: String| SettingsError::Io(e);
        if self.0.degraded {
            return Err(io(
                "the settings file could not be read at startup and is left untouched".into(),
            ));
        }
        let mut text = serde_json::to_vec_pretty(settings).map_err(|e| io(e.to_string()))?;
        text.push(b'\n');
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            match fs::create_dir_all(parent) {
                Ok(()) => {}
                Err(e) if e.kind() == ErrorKind::AlreadyExists => {}
                Err(e) => return Err(io(e.to_string())),
            }
        }
        mypass_kdbx::write_atomic(path, &text).map_err(|e| io(e.to_string()))
    }
}

/// Escapes one JSON Pointer segment (RFC 6901).
fn pointer_segment(key: &str) -> String {
    key.replace('~', "~0").replace('/', "~1")
}

/// Puts `value` at `pointer` of `root` (the defaults as JSON) and keeps it only when the whole
/// object still deserializes as [`Settings`]; otherwise the default stays.
fn try_set(root: &mut serde_json::Value, pointer: &str, value: serde_json::Value) {
    let Some(slot) = root.pointer_mut(pointer) else {
        return;
    };
    let previous = std::mem::replace(slot, value);
    if serde_json::from_value::<Settings>(root.clone()).is_err() {
        if let Some(slot) = root.pointer_mut(pointer) {
            *slot = previous;
        }
    }
}

/// Builds [`Settings`] from a parsed file field by field: every known field that deserializes is
/// taken, every other one keeps its default, nested objects are merged one level down, and
/// unknown fields are ignored.
fn tolerant_settings(fields: serde_json::Map<String, serde_json::Value>) -> Settings {
    let Ok(mut root) = serde_json::to_value(Settings::default()) else {
        return Settings::default();
    };
    for (key, value) in fields {
        let pointer = format!("/{}", pointer_segment(&key));
        match (root.pointer(&pointer), value) {
            (None, _) => {}
            (Some(serde_json::Value::Object(_)), serde_json::Value::Object(nested)) => {
                for (sub, v) in nested {
                    try_set(
                        &mut root,
                        &format!("{pointer}/{}", pointer_segment(&sub)),
                        v,
                    );
                }
            }
            (Some(_), value) => try_set(&mut root, &pointer, value),
        }
    }
    serde_json::from_value(root).unwrap_or_default()
}

fn checked_shortcut(value: String, field: &'static str) -> Result<String, SettingsError> {
    if value.trim().is_empty() {
        return Ok(String::new());
    }
    shortcut::canonical(&value).map_err(|_| SettingsError::InvalidValue(field))
}

fn apply_patch(s: &mut Settings, p: SettingsPatch) -> Result<(), SettingsError> {
    if let Some(g) = p.generator {
        if !GENERATOR_LENGTH.contains(&g.length) {
            return Err(SettingsError::InvalidValue("generator.length"));
        }
        if !(g.lowercase || g.uppercase || g.digits || g.symbols) {
            return Err(SettingsError::InvalidValue("generator.classes"));
        }
        s.generator = g;
    }
    if let Some(v) = p.shortcut_lock {
        s.shortcut_lock = checked_shortcut(v, "shortcutLock")?;
    }
    if let Some(v) = p.shortcut_show {
        s.shortcut_show = checked_shortcut(v, "shortcutShow")?;
    }
    macro_rules! set {
        ($($field:ident),* $(,)?) => {
            $(if let Some(v) = p.$field { s.$field = v; })*
        };
    }
    set!(
        language,
        theme,
        auto_lock_idle_timeout,
        lock_on_device_sleep,
        lock_on_screen_lock,
        clipboard_clear_seconds,
        backup_count,
        backup_interval,
        launch_at_login,
        show_menu_bar_extra,
        passkey_proxy_enabled,
        unlock_touch_id_enabled,
        unlock_mac_password_enabled,
        unlock_with_device_window,
        require_account_password_window,
        secondary_folder,
        update_check_enabled,
        vault_path,
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;

    fn store_in(dir: &Path) -> (SettingsStore, PathBuf) {
        let path = dir.join("settings.json");
        (SettingsStore::load(path.clone()), path)
    }

    fn snapshot_path() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("snapshots")
            .join("settings.default.json")
    }

    #[test]
    fn settings_defaults_match_spec() {
        let s = Settings::default();
        assert_eq!(s.language, Language::System);
        assert_eq!(s.theme, Theme::System);
        assert_eq!(s.auto_lock_idle_timeout, IdleTimeout::ThirtyMinutes);
        assert!(s.lock_on_device_sleep);
        assert!(s.lock_on_screen_lock);
        assert_eq!(s.clipboard_clear_seconds, 30);
        assert_eq!(s.backup_count, 10);
        assert_eq!(s.backup_interval, BackupInterval::EverySave);
        assert!(!s.launch_at_login);
        assert!(s.show_menu_bar_extra);
        assert_eq!(s.shortcut_lock, "CommandOrControl+Shift+L");
        assert_eq!(s.shortcut_show, "");
        assert_eq!(
            s.generator,
            GeneratorOptionsDto {
                length: 20,
                lowercase: true,
                uppercase: true,
                digits: true,
                symbols: true
            }
        );
        assert!(s.passkey_proxy_enabled);
        assert!(!s.unlock_touch_id_enabled);
        assert!(!s.unlock_mac_password_enabled);
        assert_eq!(
            s.unlock_with_device_window,
            UnlockWithDeviceWindow::EightHours
        );
        assert_eq!(
            s.require_account_password_window,
            RequireAccountPasswordWindow::ThirtyDays
        );
        assert_eq!(s.secondary_folder, None);
        assert!(s.update_check_enabled);
        assert_eq!(s.vault_path, None);
        assert!(s.recent_vaults.is_empty());
    }

    #[test]
    fn settings_schema_snapshot() {
        let actual = serde_json::to_value(Settings::default()).unwrap();
        let text = fs::read_to_string(snapshot_path()).expect("committed snapshot");
        let expected: serde_json::Value = serde_json::from_str(&text).unwrap();
        let (a, e) = (actual.as_object().unwrap(), expected.as_object().unwrap());
        for (key, value) in e {
            assert_eq!(a.get(key), Some(value), "snapshot key {key}");
        }
        for key in a.keys() {
            assert!(e.contains_key(key), "key {key} missing from the snapshot");
        }
    }

    /// Regenerates the committed snapshot after an intended schema change.
    #[test]
    #[ignore = "regenerates tests/snapshots/settings.default.json"]
    fn settings_snapshot_regenerate() {
        let mut text = serde_json::to_string_pretty(&Settings::default()).unwrap();
        text.push('\n');
        fs::write(snapshot_path(), text).unwrap();
    }

    #[test]
    fn enums_serialise_camel_case() {
        assert_eq!(
            serde_json::to_value(IdleTimeout::FifteenMinutes).unwrap(),
            "fifteenMinutes"
        );
        assert_eq!(
            serde_json::to_value(BackupInterval::EverySave).unwrap(),
            "everySave"
        );
        assert_eq!(serde_json::to_value(Language::Uk).unwrap(), "uk");
        assert_eq!(
            serde_json::to_value(UnlockWithDeviceWindow::TwentyFourHours).unwrap(),
            "twentyFourHours"
        );
    }

    #[test]
    fn settings_loads_old_file_with_missing_fields() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        fs::write(&path, r#"{"language":"uk","trayClickAction":"openApp"}"#).unwrap();
        let store = SettingsStore::load(path.clone());
        let s = store.get();
        assert_eq!(s.language, Language::Uk);
        assert_eq!(
            Settings {
                language: Language::System,
                ..s
            },
            Settings::default()
        );
        assert!(path.exists());
        assert!(!dir.path().join("settings.json.bad").exists());
    }

    #[test]
    fn idle_timeout_seconds_table() {
        use IdleTimeout::*;
        let table = [
            (Immediately, Some(2)),
            (OneMinute, Some(60)),
            (FiveMinutes, Some(300)),
            (FifteenMinutes, Some(900)),
            (ThirtyMinutes, Some(1800)),
            (OneHour, Some(3600)),
            (FourHours, Some(14400)),
            (Never, None),
        ];
        for (t, secs) in table {
            assert_eq!(t.seconds(), secs, "{t:?}");
        }
    }

    #[test]
    fn backup_interval_seconds_table() {
        assert_eq!(BackupInterval::EverySave.seconds(), 0);
        assert_eq!(BackupInterval::TenMinutes.seconds(), 600);
        assert_eq!(BackupInterval::Hourly.seconds(), 3600);
        assert_eq!(BackupInterval::Daily.seconds(), 86400);
    }

    #[test]
    fn backup_count_clamped_to_1_50() {
        let store = SettingsStore::in_memory();
        for (given, want) in [(0, 1), (99, 50), (7, 7)] {
            let s = store
                .update(SettingsPatch {
                    backup_count: Some(given),
                    ..SettingsPatch::default()
                })
                .unwrap();
            assert_eq!(s.backup_count, want);
        }
    }

    #[test]
    fn clipboard_seconds_clamped_to_10_120_step_5() {
        let store = SettingsStore::in_memory();
        for (given, want) in [(0, 10), (7, 10), (12, 10), (13, 15), (45, 45), (300, 120)] {
            let s = store
                .update(SettingsPatch {
                    clipboard_clear_seconds: Some(given),
                    ..SettingsPatch::default()
                })
                .unwrap();
            assert_eq!(s.clipboard_clear_seconds, want, "given {given}");
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        fs::write(&path, r#"{"clipboardClearSeconds":300,"backupCount":0}"#).unwrap();
        let s = SettingsStore::load(path).get();
        assert_eq!(s.clipboard_clear_seconds, 120);
        assert_eq!(s.backup_count, 1);
    }

    #[test]
    fn generator_options_validated_on_update() {
        let store = SettingsStore::in_memory();
        let with = |length, classes: bool| SettingsPatch {
            generator: Some(GeneratorOptionsDto {
                length,
                lowercase: classes,
                uppercase: classes,
                digits: classes,
                symbols: classes,
            }),
            ..SettingsPatch::default()
        };
        for bad in [7, 65] {
            assert_eq!(
                store.update(with(bad, true)),
                Err(SettingsError::InvalidValue("generator.length"))
            );
        }
        assert_eq!(
            store.update(with(20, false)),
            Err(SettingsError::InvalidValue("generator.classes"))
        );
        assert_eq!(store.get().generator, GeneratorOptionsDto::default());
        assert_eq!(store.update(with(8, true)).unwrap().generator.length, 8);
        assert_eq!(store.update(with(64, true)).unwrap().generator.length, 64);
        let one_class = GeneratorOptionsDto {
            length: 12,
            lowercase: false,
            uppercase: false,
            digits: true,
            symbols: false,
        };
        let s = store
            .update(SettingsPatch {
                generator: Some(one_class.clone()),
                ..SettingsPatch::default()
            })
            .unwrap();
        assert_eq!(s.generator, one_class);
    }

    #[test]
    fn patch_updates_only_given_fields() {
        let store = SettingsStore::in_memory();
        let before = store.get();
        let after = store
            .update(SettingsPatch {
                theme: Some(Theme::Dark),
                lock_on_screen_lock: Some(false),
                ..SettingsPatch::default()
            })
            .unwrap();
        assert_eq!(after.theme, Theme::Dark);
        assert!(!after.lock_on_screen_lock);
        assert_eq!(
            Settings {
                theme: before.theme,
                lock_on_screen_lock: before.lock_on_screen_lock,
                ..after.clone()
            },
            before
        );
        assert_eq!(store.get(), after);
    }

    #[test]
    fn patch_deserialises_null_as_clear_and_missing_as_untouched() {
        let p: SettingsPatch = serde_json::from_str(r#"{"vaultPath":null}"#).unwrap();
        assert_eq!(p.vault_path, Some(None));
        assert_eq!(p.secondary_folder, None);
        let p: SettingsPatch = serde_json::from_str(r#"{"secondaryFolder":"/x"}"#).unwrap();
        assert_eq!(p.secondary_folder, Some(Some("/x".to_owned())));
        let store = SettingsStore::in_memory();
        store
            .update(SettingsPatch {
                vault_path: Some(Some("/v.kdbx".into())),
                ..SettingsPatch::default()
            })
            .unwrap();
        let s = store
            .update(SettingsPatch {
                vault_path: Some(None),
                ..SettingsPatch::default()
            })
            .unwrap();
        assert_eq!(s.vault_path, None);
    }

    #[test]
    fn settings_round_trip_through_file() {
        let dir = tempfile::tempdir().unwrap();
        let (store, path) = store_in(&dir.path().join("nested"));
        let patch = SettingsPatch {
            language: Some(Language::Ru),
            theme: Some(Theme::Light),
            auto_lock_idle_timeout: Some(IdleTimeout::FourHours),
            lock_on_device_sleep: Some(false),
            lock_on_screen_lock: Some(false),
            clipboard_clear_seconds: Some(55),
            backup_count: Some(3),
            backup_interval: Some(BackupInterval::Daily),
            launch_at_login: Some(true),
            show_menu_bar_extra: Some(false),
            shortcut_lock: Some("ctrl+alt+K".into()),
            shortcut_show: Some("cmd+shift+M".into()),
            generator: Some(GeneratorOptionsDto {
                length: 33,
                lowercase: true,
                uppercase: false,
                digits: true,
                symbols: false,
            }),
            passkey_proxy_enabled: Some(false),
            unlock_touch_id_enabled: Some(true),
            unlock_mac_password_enabled: Some(true),
            unlock_with_device_window: Some(UnlockWithDeviceWindow::Never),
            require_account_password_window: Some(RequireAccountPasswordWindow::SevenDays),
            secondary_folder: Some(Some("/secondary".into())),
            update_check_enabled: Some(false),
            vault_path: Some(Some("/vault.kdbx".into())),
        };
        let saved = store.update(patch).unwrap();
        store.remember_vault("/a.kdbx");
        let saved = Settings {
            recent_vaults: vec!["/a.kdbx".into()],
            ..saved
        };
        assert_eq!(saved.shortcut_lock, "CommandOrControl+Alt+K");
        assert_eq!(saved.shortcut_show, "CommandOrControl+Shift+M");
        let reloaded = SettingsStore::load(path).get();
        assert_eq!(reloaded, saved);
        assert_eq!(reloaded, store.get());
    }

    #[test]
    fn corrupt_file_loads_defaults_and_renames_bad() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        fs::write(&path, b"{not json").unwrap();
        let store = SettingsStore::load(path.clone());
        assert_eq!(store.get(), Settings::default());
        let bad = dir.path().join("settings.json.bad");
        assert_eq!(fs::read(&bad).unwrap(), b"{not json");
        assert!(!path.exists());
        // Loading again finds no file: defaults, and the .bad copy is left alone.
        assert_eq!(SettingsStore::load(path).get(), Settings::default());
        assert_eq!(fs::read(&bad).unwrap(), b"{not json");
    }

    #[test]
    fn missing_file_loads_defaults_without_writing() {
        let dir = tempfile::tempdir().unwrap();
        let (store, path) = store_in(dir.path());
        assert_eq!(store.get(), Settings::default());
        assert!(!path.exists());
    }

    #[test]
    fn failed_write_keeps_value_in_memory_and_reports_io() {
        let dir = tempfile::tempdir().unwrap();
        let blocker = dir.path().join("file");
        fs::write(&blocker, b"x").unwrap();
        // The parent of the settings path is a regular file: every write fails.
        let store = SettingsStore::load(blocker.join("settings.json"));
        let r = store.update(SettingsPatch {
            theme: Some(Theme::Dark),
            ..SettingsPatch::default()
        });
        assert!(matches!(r, Err(SettingsError::Io(_))), "{r:?}");
        assert_eq!(store.get().theme, Theme::Dark);
    }

    #[test]
    fn recent_vaults_unique_most_recent_first_max_8() {
        let store = SettingsStore::in_memory();
        for i in 0..10 {
            store.remember_vault(&format!("/v{i}.kdbx"));
        }
        store.remember_vault("/v5.kdbx");
        let r = store.get().recent_vaults;
        assert_eq!(r.len(), 8);
        assert_eq!(r[0], "/v5.kdbx");
        assert_eq!(r[1], "/v9.kdbx");
        assert_eq!(r.iter().filter(|p| *p == "/v5.kdbx").count(), 1);
        assert!(!r.contains(&"/v0.kdbx".to_owned()));
    }

    #[test]
    fn forget_vault_removes_from_recents_and_clears_vault_path_when_equal() {
        let store = SettingsStore::in_memory();
        store.remember_vault("/a.kdbx");
        store.remember_vault("/b.kdbx");
        store
            .update(SettingsPatch {
                vault_path: Some(Some("/b.kdbx".into())),
                ..SettingsPatch::default()
            })
            .unwrap();
        store.forget_vault("/a.kdbx");
        let s = store.get();
        assert_eq!(s.recent_vaults, vec!["/b.kdbx".to_owned()]);
        assert_eq!(s.vault_path.as_deref(), Some("/b.kdbx"));
        store.forget_vault("/b.kdbx");
        let s = store.get();
        assert!(s.recent_vaults.is_empty());
        assert_eq!(s.vault_path, None);
    }

    #[test]
    fn invalid_shortcut_rejected_on_update() {
        let store = SettingsStore::in_memory();
        for bad in ["L", "Cmd+Shift", "Cmd+K+L"] {
            assert_eq!(
                store.update(SettingsPatch {
                    shortcut_show: Some(bad.into()),
                    ..SettingsPatch::default()
                }),
                Err(SettingsError::InvalidValue("shortcutShow"))
            );
        }
        assert_eq!(
            store.update(SettingsPatch {
                shortcut_lock: Some("Hyper+L".into()),
                ..SettingsPatch::default()
            }),
            Err(SettingsError::InvalidValue("shortcutLock"))
        );
        // Empty means "no shortcut" and is accepted.
        let s = store
            .update(SettingsPatch {
                shortcut_lock: Some(String::new()),
                ..SettingsPatch::default()
            })
            .unwrap();
        assert_eq!(s.shortcut_lock, "");
    }

    fn load_text(text: &str) -> (tempfile::TempDir, SettingsStore) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        fs::write(&path, text).unwrap();
        let store = SettingsStore::load(path);
        (dir, store)
    }

    #[test]
    fn one_bad_field_falls_back_alone_and_keeps_the_vault_path() {
        let (dir, store) = load_text(
            r#"{"language":"klingon","theme":"dark","backupCount":"many",
                "vaultPath":"/v.kdbx","recentVaults":["/v.kdbx"]}"#,
        );
        let s = store.get();
        assert_eq!(s.language, Language::System);
        assert_eq!(s.backup_count, 10);
        assert_eq!(s.theme, Theme::Dark);
        assert_eq!(s.vault_path.as_deref(), Some("/v.kdbx"));
        assert_eq!(s.recent_vaults, vec!["/v.kdbx".to_owned()]);
        assert!(!dir.path().join("settings.json.bad").exists());
        assert!(!store.is_degraded());
    }

    #[test]
    fn partial_generator_keeps_other_fields_at_defaults() {
        let (_dir, store) = load_text(r#"{"generator":{"length":30}}"#);
        let g = store.get().generator;
        assert_eq!(
            g,
            GeneratorOptionsDto {
                length: 30,
                ..GeneratorOptionsDto::default()
            }
        );
        let (_dir, store) = load_text(r#"{"generator":{"length":"x","digits":false}}"#);
        let g = store.get().generator;
        assert_eq!(g.length, 20);
        assert!(!g.digits);
        assert!(g.symbols);
    }

    #[test]
    fn newer_file_with_extra_fields_loads() {
        let (dir, store) = load_text(
            r#"{"futureField":1,"theme":"light","generator":{"length":12,"futureFlag":true},
                "vaultPath":"/v.kdbx"}"#,
        );
        let s = store.get();
        assert_eq!(s.theme, Theme::Light);
        assert_eq!(s.generator.length, 12);
        assert_eq!(s.vault_path.as_deref(), Some("/v.kdbx"));
        assert!(!dir.path().join("settings.json.bad").exists());
    }

    #[test]
    fn json_that_is_not_an_object_goes_to_bad() {
        let (dir, store) = load_text("[1, 2]");
        assert_eq!(store.get(), Settings::default());
        assert!(dir.path().join("settings.json.bad").exists());
        assert!(!dir.path().join("settings.json").exists());
    }

    #[test]
    fn unreadable_path_is_never_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        // A directory where the file belongs: reading fails with something other than NotFound.
        fs::create_dir(&path).unwrap();
        let store = SettingsStore::load(path.clone());
        assert!(store.is_degraded());
        assert_eq!(store.get(), Settings::default());
        let r = store.update(SettingsPatch {
            theme: Some(Theme::Dark),
            ..SettingsPatch::default()
        });
        assert!(matches!(r, Err(SettingsError::Io(_))), "{r:?}");
        assert_eq!(store.get().theme, Theme::Dark);
        assert!(path.is_dir());
    }

    #[cfg(unix)]
    #[test]
    fn unreadable_file_is_not_replaced_by_defaults() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let original = br#"{"vaultPath":"/real.kdbx"}"#;
        fs::write(&path, original).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();
        if fs::read(&path).is_ok() {
            // Running as a user that ignores permissions: the failure cannot be produced here.
            return;
        }
        let store = SettingsStore::load(path.clone());
        assert!(store.is_degraded());
        let r = store.update(SettingsPatch {
            theme: Some(Theme::Dark),
            ..SettingsPatch::default()
        });
        assert!(matches!(r, Err(SettingsError::Io(_))), "{r:?}");
        store.remember_vault("/other.kdbx");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(fs::read(&path).unwrap(), original);
    }
}
