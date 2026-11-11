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
/// Task 17 installs the real runner; until then [`NoEffects`] drops them.
pub trait EffectRunner: Send + Sync {
    fn run(&self, effects: Vec<Effect>);
}

/// An [`EffectRunner`] that does nothing.
pub struct NoEffects;

impl EffectRunner for NoEffects {
    fn run(&self, _effects: Vec<Effect>) {}
}

/// Everything besides the core that the commands need: the dialogs and what they granted, the
/// window operations and the effect runner. All trait objects, so tests use fakes.
pub struct ShellState {
    pub picker: Arc<dyn FilePicker>,
    pub grants: FileGrants,
    pub windows: Arc<dyn WindowOps>,
    pub effects: Arc<dyn EffectRunner>,
}

/// [`ProcessLookup`] over `mypass_platform::running_app_executables` (macOS; empty elsewhere).
pub struct SystemProcessLookup;

impl ProcessLookup for SystemProcessLookup {
    fn other_app_executables(&self, bundle_id: &str) -> Vec<PathBuf> {
        mypass_platform::running_app_executables(bundle_id)
    }
}
