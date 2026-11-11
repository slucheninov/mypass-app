//! Global hotkeys: the pure plan (which accelerator does what), the registration seam and the
//! small decisions of what a press does.
//!
//! A press is a Rust callback of the global-shortcut plugin, never a webview event. `Lock` hands
//! the lock to the scheduler (a lock waits for an in-flight save and a callback must not block);
//! `Show` shows and focuses the main window, and a locked vault simply stays on its lock screen.

use crate::events::{ShortcutConflictEvent, ShortcutKind};
use crate::lifecycle::lock_via_scheduler;
use crate::shell::{current_os, show_main, WindowOps};
use mypass_session::settings::Settings;
use mypass_session::shortcut;
use mypass_session::{AppCore, LockReason};

/// What a hotkey does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyAction {
    Lock,
    Show,
}

impl HotkeyAction {
    fn kind(self) -> ShortcutKind {
        match self {
            Self::Lock => ShortcutKind::Lock,
            Self::Show => ShortcutKind::Show,
        }
    }
}

/// The hotkeys the settings ask for, as `(action, canonical accelerator)`, Lock first. An empty
/// shortcut means "none" and is left out; a legacy spelling is canonicalised; a string that does
/// not parse is kept as stored, so registering it fails and is reported as a conflict.
pub fn hotkey_plan(settings: &Settings) -> Vec<(HotkeyAction, String)> {
    [
        (HotkeyAction::Lock, &settings.shortcut_lock),
        (HotkeyAction::Show, &settings.shortcut_show),
    ]
    .into_iter()
    .filter(|(_, raw)| !raw.trim().is_empty())
    .map(|(action, raw)| {
        let accelerator = shortcut::canonical(raw).unwrap_or_else(|_| raw.clone());
        (action, accelerator)
    })
    .collect()
}

/// The canonical accelerator written for the plugin on `os`: `CommandOrControl` is spelled out as
/// `Cmd` on macOS and `Ctrl` elsewhere, so the plugin never has to guess.
pub fn render_for_plugin(accelerator: &str, os: &str) -> String {
    let command = if os == "macos" { "Cmd" } else { "Ctrl" };
    accelerator
        .split('+')
        .map(|part| {
            if part == "CommandOrControl" {
                command
            } else {
                part
            }
        })
        .collect::<Vec<_>>()
        .join("+")
}

/// Where the hotkeys are registered; the plugin in production, a fake in tests.
pub trait HotkeyRegistry {
    /// Drops every binding this app registered earlier.
    fn unregister_all(&self);
    /// Registers `accelerator` (already rendered for the plugin) for `action`; `Err` when it is
    /// invalid or already taken.
    fn register(&self, action: HotkeyAction, accelerator: &str) -> Result<(), String>;
}

/// Replaces the registered hotkeys with the plan of `settings`. A shortcut that cannot be
/// registered is reported, never fatal: the rest still registers and the app starts.
pub fn register_with(
    registry: &dyn HotkeyRegistry,
    settings: &Settings,
    os: &str,
) -> Vec<ShortcutConflictEvent> {
    registry.unregister_all();
    let mut conflicts = Vec::new();
    for (action, accelerator) in hotkey_plan(settings) {
        let rendered = render_for_plugin(&accelerator, os);
        if registry.register(action, &rendered).is_err() {
            conflicts.push(ShortcutConflictEvent {
                kind: action.kind(),
                shortcut: accelerator,
            });
        }
    }
    conflicts
}

/// What a press does. `Lock` is a no-op while nothing is unlocked (the session ignores it).
pub fn run_hotkey(action: HotkeyAction, core: &AppCore, windows: &dyn WindowOps) {
    match action {
        HotkeyAction::Lock => lock_via_scheduler(core, LockReason::Hotkey),
        // A window that cannot be shown is not worth failing a key press for.
        HotkeyAction::Show => {
            let _ = show_main(windows);
        }
    }
}

/// Production [`HotkeyRegistry`] over the global-shortcut plugin.
pub struct PluginRegistry<R: tauri::Runtime = tauri::Wry>(pub tauri::AppHandle<R>);

impl<R: tauri::Runtime> HotkeyRegistry for PluginRegistry<R> {
    fn unregister_all(&self) {
        use tauri_plugin_global_shortcut::GlobalShortcutExt;
        // Nothing registered yet is not an error.
        let _ = self.0.global_shortcut().unregister_all();
    }

    fn register(&self, action: HotkeyAction, accelerator: &str) -> Result<(), String> {
        use tauri::Manager;
        use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
        let handle = self.0.clone();
        self.0
            .global_shortcut()
            .on_shortcut(accelerator, move |_app, _shortcut, event| {
                if event.state != ShortcutState::Pressed {
                    return;
                }
                let (Some(core), Some(shell)) = (
                    handle.try_state::<crate::state::AppState>(),
                    handle.try_state::<crate::state::ShellState>(),
                ) else {
                    return;
                };
                run_hotkey(action, &core.0, shell.windows.as_ref());
            })
            .map_err(|e| e.to_string())
    }
}

/// Re-registers the hotkeys from `settings` and returns the conflicts.
pub fn register_all<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    settings: &Settings,
) -> Vec<ShortcutConflictEvent> {
    register_with(&PluginRegistry(app.clone()), settings, current_os())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::FakeWindowOps;
    use crate::testing::open_world;
    use mypass_session::SessionState;
    use std::cell::RefCell;
    use std::collections::HashSet;

    fn settings(lock: &str, show: &str) -> Settings {
        Settings {
            shortcut_lock: lock.into(),
            shortcut_show: show.into(),
            ..Settings::default()
        }
    }

    #[test]
    fn hotkey_registration_plan_skips_empty_show_shortcut() {
        assert_eq!(
            hotkey_plan(&Settings::default()),
            [(HotkeyAction::Lock, "CommandOrControl+Shift+L".to_owned())]
        );
        assert_eq!(
            hotkey_plan(&settings("", "")),
            Vec::<(HotkeyAction, String)>::new()
        );
        assert_eq!(
            hotkey_plan(&settings("", "CommandOrControl+Shift+M")),
            [(HotkeyAction::Show, "CommandOrControl+Shift+M".to_owned())]
        );
        let both = hotkey_plan(&settings("Ctrl+Alt+K", "Ctrl+Alt+M"));
        assert_eq!(both.len(), 2);
        assert_eq!(both[0].0, HotkeyAction::Lock);
        assert_eq!(both[1].0, HotkeyAction::Show);
    }

    #[test]
    fn hotkey_plan_canonicalises_legacy_strings() {
        // The old app stored `shift+command+L`.
        assert_eq!(
            hotkey_plan(&settings("shift+command+L", "option+cmd+m")),
            [
                (HotkeyAction::Lock, "CommandOrControl+Shift+L".to_owned()),
                (HotkeyAction::Show, "CommandOrControl+Alt+M".to_owned()),
            ]
        );
        // An unparsable string is kept as stored so the registration reports it.
        assert_eq!(
            hotkey_plan(&settings("Hyper+L", "")),
            [(HotkeyAction::Lock, "Hyper+L".to_owned())]
        );
    }

    #[test]
    fn hotkey_plan_windows_defaults() {
        let plan = hotkey_plan(&Settings::default());
        assert_eq!(
            plan,
            [(HotkeyAction::Lock, "CommandOrControl+Shift+L".to_owned())]
        );
        let accelerator = &plan[0].1;
        assert_eq!(render_for_plugin(accelerator, "windows"), "Ctrl+Shift+L");
        assert_eq!(render_for_plugin(accelerator, "macos"), "Cmd+Shift+L");
        assert_eq!(render_for_plugin("Alt+Shift+K", "macos"), "Alt+Shift+K");
        assert_eq!(
            render_for_plugin("CommandOrControl+Alt+Super+X", "windows"),
            "Ctrl+Alt+Super+X"
        );
    }

    #[derive(Default)]
    struct FakeRegistry {
        taken: HashSet<String>,
        log: RefCell<Vec<String>>,
    }

    impl HotkeyRegistry for FakeRegistry {
        fn unregister_all(&self) {
            self.log.borrow_mut().push("unregister_all".into());
        }
        fn register(&self, action: HotkeyAction, accelerator: &str) -> Result<(), String> {
            self.log
                .borrow_mut()
                .push(format!("register {action:?} {accelerator}"));
            if self.taken.contains(accelerator) {
                Err("already registered".into())
            } else {
                Ok(())
            }
        }
    }

    #[test]
    fn register_unregisters_first_and_registers_the_rendered_plan() {
        let registry = FakeRegistry::default();
        let conflicts = register_with(&registry, &settings("", "x"), "macos");
        // `x` does not parse, so it is kept as stored and rendered unchanged; the fake accepts it.
        assert!(conflicts.is_empty());
        let conflicts = register_with(&registry, &Settings::default(), "windows");
        assert!(conflicts.is_empty());
        assert_eq!(
            *registry.log.borrow(),
            [
                "unregister_all",
                "register Show x",
                "unregister_all",
                "register Lock Ctrl+Shift+L"
            ]
        );
    }

    #[test]
    fn a_taken_accelerator_is_reported_as_a_conflict_not_a_failure() {
        let registry = FakeRegistry {
            taken: ["Cmd+Shift+L".to_owned()].into(),
            ..FakeRegistry::default()
        };
        let conflicts = register_with(
            &registry,
            &settings("CommandOrControl+Shift+L", "CommandOrControl+Alt+M"),
            "macos",
        );
        assert_eq!(
            conflicts,
            [ShortcutConflictEvent {
                kind: ShortcutKind::Lock,
                shortcut: "CommandOrControl+Shift+L".into()
            }]
        );
        // The Show shortcut still registered.
        assert!(registry
            .log
            .borrow()
            .contains(&"register Show Cmd+Alt+M".to_owned()));
    }

    #[test]
    fn lock_hotkey_locks_through_the_scheduler_and_is_a_no_op_when_locked() {
        let w = open_world();
        let windows = FakeWindowOps::default();
        run_hotkey(HotkeyAction::Lock, &w.core, &windows);
        // Handed to the scheduler: nothing happened inline.
        assert!(w.core.session.is_unlocked());
        w.run_scheduled();
        let status = w.core.session.status();
        assert_eq!(status.state, SessionState::Locked);
        assert_eq!(status.lock_reason, Some(LockReason::Hotkey));
        // Locked already: pressing again changes nothing and does not panic.
        run_hotkey(HotkeyAction::Lock, &w.core, &windows);
        w.run_scheduled();
        assert_eq!(w.core.session.status().state, SessionState::Locked);
        assert!(windows.calls().is_empty());
    }

    #[test]
    fn show_hotkey_shows_then_focuses_main_and_leaves_a_locked_vault_locked() {
        let w = open_world();
        w.core.vault_lock().unwrap();
        let windows = FakeWindowOps::default();
        run_hotkey(HotkeyAction::Show, &w.core, &windows);
        let calls: Vec<_> = windows.calls().into_iter().map(|c| (c.0, c.1)).collect();
        assert_eq!(
            calls,
            [
                ("show".to_string(), "main".to_string()),
                ("focus".to_string(), "main".to_string())
            ]
        );
        assert_eq!(w.core.session.status().state, SessionState::Locked);
    }
}
