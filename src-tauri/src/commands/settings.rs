//! `settings_*`.

use super::blocking;
use crate::events::SettingsChanged;
use crate::shell::{settings_effects, Effect};
use crate::state::{AppState, EffectRunner, ShellState};
use mypass_session::dto::LoginItemStatus;
use mypass_session::{ApiError, AppCore, Settings, SettingsPatch};
use std::sync::{Mutex, PoisonError};
use tauri::{AppHandle, State};
use tauri_specta::Event;

#[tauri::command]
#[specta::specta]
pub fn settings_get(state: State<'_, AppState>) -> Result<Settings, ApiError> {
    state.0.settings_get()
}

/// The shortcut conflicts found while registering the global shortcuts at startup, once: the
/// frontend calls it after registering its `ShortcutConflictEvent` listener, so no timing
/// decides whether the notice is seen. Read-only, takes nothing from the webview.
#[tauri::command]
#[specta::specta]
pub fn shortcut_conflicts_take(
    webview: tauri::Webview,
    gate: State<'_, std::sync::Arc<crate::hotkeys::ConflictGate>>,
) -> Vec<crate::events::ShortcutConflictEvent> {
    if !serves_conflicts(webview.label()) {
        return Vec::new();
    }
    gate.take()
}

/// Only the main window is served the startup conflicts (the tray panel must not consume them).
pub fn serves_conflicts(label: &str) -> bool {
    label == super::app::MAIN_WINDOW
}

/// Applies `patch` and returns the new settings with the effects the change calls for.
pub fn update(core: &AppCore, patch: SettingsPatch) -> Result<(Settings, Vec<Effect>), ApiError> {
    let old = core.settings_get()?;
    let new = core.settings_update(patch)?;
    let effects = settings_effects(&old, &new);
    Ok((new, effects))
}

/// The whole update sequence under `gate`: read the old settings, apply the patch, compute and run
/// the effects, then `emit`. Two quick updates therefore cannot interleave: each one's effects
/// are computed against the previous committed settings and the events go out in call order.
pub fn apply(
    core: &AppCore,
    gate: &Mutex<()>,
    runner: &dyn EffectRunner,
    emit: impl FnOnce(&Settings),
    patch: SettingsPatch,
) -> Result<Settings, ApiError> {
    let _serial = gate.lock().unwrap_or_else(PoisonError::into_inner);
    let (new, effects) = update(core, patch)?;
    runner.run(effects);
    // The effects may have corrected the store (a refused login item is written back off), so
    // the webview gets what is stored now, not the value the patch asked for.
    let current = core.settings_get().unwrap_or(new);
    emit(&current);
    Ok(current)
}

#[tauri::command]
#[specta::specta]
pub async fn settings_update(
    app: AppHandle,
    state: State<'_, AppState>,
    shell: State<'_, ShellState>,
    patch: SettingsPatch,
) -> Result<Settings, ApiError> {
    // The settings file is written atomically, so off the main thread.
    let core = state.0.clone();
    let gate = shell.settings_gate.clone();
    let runner = shell.effects.clone();
    blocking(move || {
        apply(
            &core,
            &gate,
            runner.as_ref(),
            // A window that is gone is not an error of the update.
            |new| {
                let _ = SettingsChanged(new.clone()).emit(&app);
            },
            patch,
        )
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub fn settings_login_item_status(state: State<'_, AppState>) -> Result<LoginItemStatus, ApiError> {
    state.0.settings_login_item_status()
}

#[cfg(test)]
mod tests {
    #[test]
    fn only_the_main_window_is_served_the_startup_conflicts() {
        assert!(super::serves_conflicts(crate::commands::app::MAIN_WINDOW));
        assert!(!super::serves_conflicts(crate::tray::PANEL_WINDOW));
        assert!(!super::serves_conflicts(""));
    }

    use super::*;
    use crate::testing::world;
    use mypass_session::{Language, Theme};
    use std::sync::mpsc;
    use std::sync::Arc;
    use std::time::Duration;

    /// Records every run; the first one blocks until released.
    struct Gated {
        log: Mutex<Vec<Vec<Effect>>>,
        entered: mpsc::SyncSender<()>,
        release: Mutex<mpsc::Receiver<()>>,
        first: std::sync::atomic::AtomicBool,
    }

    impl EffectRunner for Gated {
        fn run(&self, effects: Vec<Effect>) {
            if !self.first.swap(true, std::sync::atomic::Ordering::SeqCst) {
                self.entered.send(()).unwrap();
                self.release.lock().unwrap().recv().unwrap();
            }
            self.log.lock().unwrap().push(effects);
        }
    }

    #[test]
    fn concurrent_updates_are_serialised_and_computed_in_call_order() {
        let w = world();
        let core = Arc::new(w.core.clone());
        let gate = Arc::new(Mutex::new(()));
        let (entered_tx, entered_rx) = mpsc::sync_channel(1);
        let (release_tx, release_rx) = mpsc::channel();
        let runner = Arc::new(Gated {
            log: Mutex::new(Vec::new()),
            entered: entered_tx,
            release: Mutex::new(release_rx),
            first: false.into(),
        });
        let emitted: Arc<Mutex<Vec<Settings>>> = Arc::default();

        let spawn = |patch: SettingsPatch| {
            let (core, gate, runner, emitted) =
                (core.clone(), gate.clone(), runner.clone(), emitted.clone());
            std::thread::spawn(move || {
                apply(
                    &core,
                    &gate,
                    runner.as_ref(),
                    |s| emitted.lock().unwrap().push(s.clone()),
                    patch,
                )
                .unwrap()
            })
        };
        let first = spawn(SettingsPatch {
            theme: Some(Theme::Dark),
            ..SettingsPatch::default()
        });
        // The first update is now inside its effects, holding the gate.
        entered_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        let second = spawn(SettingsPatch {
            theme: Some(Theme::Light),
            language: Some(Language::Uk),
            ..SettingsPatch::default()
        });
        std::thread::sleep(Duration::from_millis(150));
        // The second one has not even applied its patch yet.
        assert_eq!(core.settings_get().unwrap().theme, Theme::Dark);
        assert!(emitted.lock().unwrap().is_empty());
        release_tx.send(()).unwrap();
        first.join().unwrap();
        second.join().unwrap();

        let emitted = emitted.lock().unwrap();
        assert_eq!(emitted.len(), 2);
        assert_eq!(emitted[0].theme, Theme::Dark);
        assert_eq!(emitted[1].theme, Theme::Light);
        // Each effect set is the diff against the previous committed settings.
        let log = runner.log.lock().unwrap();
        assert_eq!(log[0], vec![Effect::ApplyTheme(Theme::Dark)]);
        assert!(log[1].contains(&Effect::ApplyTheme(Theme::Light)));
        assert!(log[1].contains(&Effect::ApplyLanguage(Language::Uk)));
    }

    #[test]
    fn update_returns_the_new_settings_and_their_effects() {
        let w = world();
        let patch = SettingsPatch {
            theme: Some(Theme::Dark),
            ..SettingsPatch::default()
        };
        let (new, effects) = update(&w.core, patch).unwrap();
        assert_eq!(new.theme, Theme::Dark);
        assert_eq!(effects, vec![Effect::ApplyTheme(Theme::Dark)]);
        assert_eq!(w.core.settings_get().unwrap().theme, Theme::Dark);
    }

    #[test]
    fn an_update_that_changes_nothing_has_no_effects() {
        let w = world();
        let (_, effects) = update(&w.core, SettingsPatch::default()).unwrap();
        assert!(effects.is_empty());
    }

    /// Writes the theme back, as the login-item effect does when the OS refuses.
    struct Corrects(AppCore);

    impl EffectRunner for Corrects {
        fn run(&self, _effects: Vec<Effect>) {
            self.0
                .settings_update(SettingsPatch {
                    theme: Some(Theme::Dark),
                    ..SettingsPatch::default()
                })
                .unwrap();
        }
    }

    #[test]
    fn the_emitted_and_returned_settings_include_what_the_effects_corrected() {
        let w = world();
        let emitted = Mutex::new(None);
        let returned = apply(
            &w.core,
            &Mutex::new(()),
            &Corrects(w.core.clone()),
            |s| *emitted.lock().unwrap() = Some(s.clone()),
            SettingsPatch {
                language: Some(Language::Uk),
                ..SettingsPatch::default()
            },
        )
        .unwrap();
        assert_eq!(returned.language, Language::Uk);
        assert_eq!(returned.theme, Theme::Dark);
        assert_eq!(emitted.lock().unwrap().as_ref(), Some(&returned));
    }
}
