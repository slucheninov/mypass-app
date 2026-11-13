//! The managed state of the running app and the small seams the commands go through.

use crate::files::{FileGrants, FilePicker};
use crate::shell::{Effect, WindowOps};
use mypass_session::session::SessionEvent;
use mypass_session::vault_files::ProcessLookup;
use mypass_session::{AppCore, SessionState};
use std::path::PathBuf;
use std::sync::Arc;

/// The one `AppCore` of the process; every command reaches the backend through it.
pub struct AppState(pub Arc<AppCore>);

/// Whether the main window's editor holds unsaved changes, as the webview last reported it
/// (`app_set_editor_dirty`). Advisory only: a webview that lies can cause or suppress the
/// "discard changes" prompt on quit, and nothing else. Rust also clears it when the session locks,
/// closes or switches vault (`on_event`), so a reloaded or crashed webview cannot leave it stuck.
#[derive(Debug, Default, Clone)]
pub struct EditorDirty(Arc<std::sync::atomic::AtomicBool>);

impl EditorDirty {
    pub fn set(&self, dirty: bool) {
        self.0.store(dirty, std::sync::atomic::Ordering::SeqCst);
    }

    pub fn get(&self) -> bool {
        self.0.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Session observer hook: anything but an unlocked vault has no editor.
    pub fn on_event(&self, event: &SessionEvent) {
        if clears_editor_dirty(event) {
            self.set(false);
        }
    }
}

/// Whether a session event ends the editor's life: the session is no longer unlocked.
pub fn clears_editor_dirty(event: &SessionEvent) -> bool {
    matches!(event, SessionEvent::StateChanged(status) if status.state != SessionState::Unlocked)
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::open_world;

    #[test]
    fn a_lock_clears_the_dirty_flag_and_other_events_keep_it() {
        let w = open_world();
        let flag = EditorDirty::default();
        flag.set(true);
        flag.on_event(&SessionEvent::StateChanged(w.core.session.status()));
        assert!(flag.get(), "an unlocked status keeps it");
        flag.on_event(&SessionEvent::VaultChanged);
        assert!(flag.get());
        w.core.vault_lock().unwrap();
        flag.on_event(&SessionEvent::StateChanged(w.core.session.status()));
        assert!(!flag.get(), "a lock clears it");
    }
}
