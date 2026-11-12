//! The managed state of the running app and the small seams the commands go through.

use crate::files::{FileGrants, FilePicker};
use crate::shell::{Effect, WindowOps};
use mypass_session::vault_files::ProcessLookup;
use mypass_session::AppCore;
use std::path::PathBuf;
use std::sync::Arc;

/// The one `AppCore` of the process; every command reaches the backend through it.
pub struct AppState(pub Arc<AppCore>);

/// Runs the effects `settings_update` computed (autostart, tray, hotkeys, theme, language).
/// The real runner is `lifecycle::ShellEffects`; tests use fakes.
///
/// `run` is called OFF the main thread (on the blocking pool, while `settings_update` holds the
/// shell's settings lock), so an implementation must hop to the main thread itself for tray,
/// menu and window work (`AppHandle::run_on_main_thread`), and must not call `settings_update`.
pub trait EffectRunner: Send + Sync {
    fn run(&self, effects: Vec<Effect>);
}

/// Everything besides the core that the commands need: the dialogs and what they granted, the
/// window operations and the effect runner. All trait objects, so tests use fakes.
pub struct ShellState {
    pub picker: Arc<dyn FilePicker>,
    pub grants: FileGrants,
    pub windows: Arc<dyn WindowOps>,
    pub effects: Arc<dyn EffectRunner>,
    /// Serialises `settings_update` end to end (read old, apply, compute effects, run effects,
    /// emit), so effects are computed against the previous committed settings and the events
    /// leave in call order.
    pub settings_gate: Arc<std::sync::Mutex<()>>,
}

/// [`ProcessLookup`] over `mypass_platform::running_app_executables` (macOS; empty elsewhere).
pub struct SystemProcessLookup;

impl ProcessLookup for SystemProcessLookup {
    fn other_app_executables(&self, bundle_id: &str) -> Vec<PathBuf> {
        mypass_platform::running_app_executables(bundle_id)
    }
}
