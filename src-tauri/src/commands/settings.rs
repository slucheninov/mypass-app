//! `settings_*`.

use super::blocking;
use crate::events::SettingsChanged;
use crate::shell::{settings_effects, Effect};
use crate::state::{AppState, ShellState};
use mypass_session::dto::LoginItemStatus;
use mypass_session::{ApiError, AppCore, Settings, SettingsPatch};
use tauri::{AppHandle, State};
use tauri_specta::Event;

#[tauri::command]
#[specta::specta]
pub fn settings_get(state: State<'_, AppState>) -> Result<Settings, ApiError> {
    state.0.settings_get()
}

/// Applies `patch` and returns the new settings with the effects the change calls for.
pub fn update(core: &AppCore, patch: SettingsPatch) -> Result<(Settings, Vec<Effect>), ApiError> {
    let old = core.settings_get()?;
    let new = core.settings_update(patch)?;
    let effects = settings_effects(&old, &new);
    Ok((new, effects))
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
    let (new, effects) = blocking(move || update(&core, patch)).await?;
    // Task 17's runner executes them; the default drops them.
    shell.effects.run(effects);
    // A window that is gone is not an error of the update.
    let _ = SettingsChanged(new.clone()).emit(&app);
    Ok(new)
}

#[tauri::command]
#[specta::specta]
pub fn settings_login_item_status(state: State<'_, AppState>) -> Result<LoginItemStatus, ApiError> {
    state.0.settings_login_item_status()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::world;
    use mypass_session::Theme;

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
}
