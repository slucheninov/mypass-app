//! Startup wiring: builds the `AppCore`, manages the app state and forwards backend events to
//! the webview.
//!
//! Tray, global hotkeys, exit handling and platform event subscriptions are not wired here (the
//! next task owns them); they reach the core through [`AppState`] and the effect runner of
//! [`ShellState`].

use crate::events::{ClipboardClearedEvent, SaveStateChanged, SessionChanged, VaultChanged};
use crate::files::{FileGrants, TauriFilePicker};
use crate::shell::TauriWindowOps;
use crate::state::{AppState, NoEffects, ShellState, SystemProcessLookup};
use mypass_session::clipboard_service::ClearedSink;
use mypass_session::session::SessionEvent;
use mypass_session::{AppCore, AppPaths, SaveStatus, SessionStatus};
use std::sync::Arc;
use tauri::{AppHandle, Manager};
use tauri_specta::Event;

/// What a session event becomes on the wire.
#[derive(Debug, Clone, PartialEq)]
pub enum Outgoing {
    Session(SessionStatus),
    Vault,
    Save(SaveStatus),
}

/// The one mapping from session events to wire events.
pub fn outgoing(event: &SessionEvent) -> Outgoing {
    match event {
        SessionEvent::StateChanged(status) => Outgoing::Session(status.clone()),
        SessionEvent::VaultChanged => Outgoing::Vault,
        SessionEvent::SaveChanged(status) => Outgoing::Save(status.clone()),
    }
}

fn emit(handle: &AppHandle, out: Outgoing) {
    // A window that is gone is not an error of the backend.
    let _ = match out {
        Outgoing::Session(status) => SessionChanged(status).emit(handle),
        Outgoing::Vault => VaultChanged.emit(handle),
        Outgoing::Save(status) => SaveStateChanged(status).emit(handle),
    };
}

struct EmitCleared(AppHandle);

impl ClearedSink for EmitCleared {
    fn cleared(&self) {
        let _ = ClipboardClearedEvent.emit(&self.0);
    }
}

/// Builds the core and manages the state. The error (paths, worker threads) ends the setup hook,
/// so the app never runs half-wired.
pub fn install(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let handle = app.handle().clone();
    let paths = AppPaths::resolve()?;
    let platform = mypass_platform::current(&paths);
    let mut core = AppCore::new(paths, platform)?;
    core.set_process_lookup(Arc::new(SystemProcessLookup));

    let observer_handle = handle.clone();
    core.session
        .add_observer(Arc::new(move |event: &SessionEvent| {
            emit(&observer_handle, outgoing(event));
        }));
    core.clipboard
        .set_cleared_sink(Arc::new(EmitCleared(handle.clone())));

    app.manage(AppState(Arc::new(core)));
    app.manage(ShellState {
        picker: Arc::new(TauriFilePicker(handle.clone())),
        grants: FileGrants::default(),
        windows: Arc::new(TauriWindowOps(handle)),
        effects: Arc::new(NoEffects),
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_events_map_one_to_one() {
        let save = SaveStatus::idle();
        assert_eq!(
            outgoing(&SessionEvent::SaveChanged(save.clone())),
            Outgoing::Save(save)
        );
        assert_eq!(outgoing(&SessionEvent::VaultChanged), Outgoing::Vault);
        let status = crate::testing::world().core.session.status();
        assert_eq!(
            outgoing(&SessionEvent::StateChanged(status.clone())),
            Outgoing::Session(status)
        );
    }
}
