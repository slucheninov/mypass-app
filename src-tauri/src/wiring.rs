//! Startup wiring: builds the `AppCore`, manages the app state and forwards backend events to
//! the webview.
//!
//! The shell pieces that need the running app (tray, hotkeys, exit handling, platform events)
//! start from `shell_runtime::start` once this has managed the state; the effect runner of
//! [`ShellState`] is the real [`ShellEffects`] built here.

use crate::bridge::{build_state, TauriBridgeEvents};
use crate::events::{ClipboardClearedEvent, SaveStateChanged, SessionChanged, VaultChanged};
use crate::files::{FileGrants, TauriFilePicker};
use crate::hotkeys::ConflictGate;
use crate::lifecycle::{ExitGate, QuitPort};
use crate::lifecycle::{ShellEffects, TRAY_REFRESH_DELAY};
use crate::shell::{TauriWindowOps, WindowOps};
use crate::shell_runtime::{hide_panel, tray_refresher, TauriQuitPort, TauriTarget};
use crate::state::{AppState, ShellState, SystemProcessLookup};
use mypass_session::clipboard_service::ClearedSink;
use mypass_session::session::SessionEvent;
use mypass_session::{AppCore, AppPaths, SaveStatus, SessionState, SessionStatus};
use std::sync::{Arc, Mutex, PoisonError};
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

/// What a session event does to the tray.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayReaction {
    /// The lock state changed away from unlocked: hide the panel and rebuild the menu now.
    HideNow,
    /// The menu may be stale (state change to unlocked, or vault edits): rebuild it, coalesced.
    Refresh,
    None,
}

/// The one mapping from session events to tray work.
pub fn tray_reaction(event: &SessionEvent) -> TrayReaction {
    match event {
        SessionEvent::StateChanged(status) if status.state != SessionState::Unlocked => {
            TrayReaction::HideNow
        }
        SessionEvent::StateChanged(_) | SessionEvent::VaultChanged => TrayReaction::Refresh,
        SessionEvent::SaveChanged(_) => TrayReaction::None,
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

/// Forgets the file grants whenever the vault locks, closes or is switched, so a token minted
/// for one session can never be redeemed in another (auto-lock and every other path through the
/// session lock end here too).
pub struct GrantsReset {
    grants: FileGrants,
    last_path: Mutex<Option<String>>,
}

impl GrantsReset {
    pub fn new(grants: FileGrants) -> Self {
        Self {
            grants,
            last_path: Mutex::new(None),
        }
    }

    pub fn on_event(&self, event: &SessionEvent) {
        let SessionEvent::StateChanged(status) = event else {
            return;
        };
        let mut last = self
            .last_path
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if status.state != SessionState::Unlocked || *last != status.vault_path {
            self.grants.clear();
        }
        last.clone_from(&status.vault_path);
    }
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

    let core = Arc::new(core);
    let grants = FileGrants::default();
    let reset = GrantsReset::new(grants.clone());
    let refresher = tray_refresher(&handle, core.clone());
    let editor_dirty = crate::state::EditorDirty::default();
    let observer_dirty = editor_dirty.clone();
    let observer_handle = handle.clone();
    let observer_refresher = refresher.clone();
    core.session
        .add_observer(Arc::new(move |event: &SessionEvent| {
            reset.on_event(event);
            observer_dirty.on_event(event);
            emit(&observer_handle, outgoing(event));
            match tray_reaction(event) {
                TrayReaction::HideNow => {
                    hide_panel(&observer_handle);
                    observer_refresher.request(std::time::Duration::ZERO);
                }
                TrayReaction::Refresh => observer_refresher.request(TRAY_REFRESH_DELAY),
                TrayReaction::None => {}
            }
        }));
    core.clipboard
        .set_cleared_sink(Arc::new(EmitCleared(handle.clone())));

    let conflicts = Arc::new(ConflictGate::default());
    let target = Arc::new(TauriTarget::new(
        handle.clone(),
        core.clone(),
        refresher,
        conflicts.clone(),
    ));
    app.manage(conflicts);
    app.manage(ExitGate::default());
    app.manage(editor_dirty);
    app.manage::<Arc<dyn QuitPort>>(Arc::new(TauriQuitPort(handle.clone())));
    app.manage(AppState(core.clone()));
    let windows: Arc<dyn WindowOps> = Arc::new(TauriWindowOps(handle.clone()));
    // The bridge starts even while the vault is locked. A start failure (an unreadable pairing
    // list, a port that cannot be bound) leaves the app running; the Browser pane shows
    // `listening: false` and, for the pairing list, offers the reset.
    let bridge = build_state(
        &core,
        windows.clone(),
        Arc::new(TauriBridgeEvents(handle.clone())),
    );
    let _ = bridge.start();
    app.manage(bridge);
    app.manage(ShellState {
        picker: Arc::new(TauriFilePicker(handle)),
        grants,
        windows,
        effects: Arc::new(ShellEffects::new(core, target)),
        settings_gate: Arc::new(Mutex::new(())),
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn world_with_reset() -> (crate::testing::World, FileGrants) {
        let w = crate::testing::world();
        let grants = FileGrants::default();
        let reset = GrantsReset::new(grants.clone());
        w.core
            .session
            .add_observer(Arc::new(move |e: &SessionEvent| reset.on_event(e)));
        (w, grants)
    }

    fn open(w: &crate::testing::World) {
        w.core
            .session
            .create_vault_with(
                &w.path,
                crate::testing::pw("pw"),
                mypass_session::testing::fast_database("T"),
            )
            .unwrap();
    }

    #[test]
    fn lock_forgets_tokens_and_the_remembered_csv() {
        let (w, grants) = world_with_reset();
        open(&w);
        let file = w.file("a.txt", b"a");
        let token = grants.grant_tokens(vec![file.clone()]).remove(0).token;
        grants.set_csv(Some(file));
        w.core.vault_lock().unwrap();
        let mut ops = vec![mypass_session::dto::AttachmentOp::Add { path: token }];
        assert!(
            grants.redeem(&mut ops).is_err(),
            "token rejected after lock"
        );
        w.core.vault_unlock(crate::testing::pw("pw")).unwrap();
        // The CSV path did not survive the lock: a commit has nothing to import.
        let err = crate::commands::import::commit(&w.core, &grants, &w.root()).unwrap_err();
        assert_eq!(err.code, mypass_session::ErrorCode::InvalidInput);
    }

    #[test]
    fn a_status_event_of_the_same_unlocked_vault_keeps_the_grants() {
        let (w, grants) = world_with_reset();
        open(&w);
        let token = grants
            .grant_tokens(vec![w.file("a.txt", b"a")])
            .remove(0)
            .token;
        let reset = GrantsReset::new(grants.clone());
        let status = w.core.session.status();
        reset.on_event(&SessionEvent::StateChanged(status.clone()));
        // Re-grant after the first event (the first sight of a path counts as a switch).
        let token2 = grants
            .grant_tokens(vec![w.file("b.txt", b"b")])
            .remove(0)
            .token;
        reset.on_event(&SessionEvent::StateChanged(status));
        let mut ops = vec![mypass_session::dto::AttachmentOp::Add { path: token2 }];
        assert!(grants.redeem(&mut ops).is_ok());
        let mut old = vec![mypass_session::dto::AttachmentOp::Add { path: token }];
        assert!(grants.redeem(&mut old).is_err());
    }

    #[test]
    fn tray_reacts_to_lock_and_vault_changes_only() {
        let w = crate::testing::open_world();
        let unlocked = w.core.session.status();
        w.core.vault_lock().unwrap();
        let locked = w.core.session.status();
        assert_eq!(
            tray_reaction(&SessionEvent::StateChanged(locked)),
            TrayReaction::HideNow
        );
        assert_eq!(
            tray_reaction(&SessionEvent::StateChanged(unlocked)),
            TrayReaction::Refresh
        );
        assert_eq!(
            tray_reaction(&SessionEvent::VaultChanged),
            TrayReaction::Refresh
        );
        assert_eq!(
            tray_reaction(&SessionEvent::SaveChanged(SaveStatus::idle())),
            TrayReaction::None
        );
    }

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
