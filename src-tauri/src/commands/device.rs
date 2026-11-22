//! Device authentication may call Keychain/LocalAuthentication: every platform call is on a worker.
use super::{blocking, into_secret};
use crate::state::AppState;
use mypass_session::device_unlock::{DeviceUnlockOffer, DeviceUnlockStatus};
use mypass_session::ApiError;
use tauri::State;
/// The full status (toggles, enrolment, block reason) for the Security pane; unlocked only.
#[tauri::command]
#[specta::specta]
pub async fn device_unlock_status(
    state: State<'_, AppState>,
) -> Result<DeviceUnlockStatus, ApiError> {
    let incarnation = super::ensure_unlocked(&state.0)?;
    let core = state.0.clone();
    blocking(move || core.device_unlock.status_for(incarnation)).await
}
/// What an unlock screen needs to show the device-unlock button, in any session state. Reads
/// only: no reconcile, no setting written.
#[tauri::command]
#[specta::specta]
pub async fn device_unlock_offer(
    state: State<'_, AppState>,
) -> Result<DeviceUnlockOffer, ApiError> {
    let core = state.0.clone();
    blocking(move || Ok(core.device_unlock.offer())).await
}
#[tauri::command]
#[specta::specta]
pub async fn device_unlock_configure(
    app: tauri::AppHandle,
    _payload: ConfigurePayloadGuard,
    state: State<'_, AppState>,
    mut password: Option<String>,
    touch_id: bool,
    mac_password: bool,
) -> Result<DeviceUnlockStatus, ApiError> {
    let password = password.as_mut().map(into_secret);
    let incarnation = super::ensure_unlocked(&state.0)?;
    let core = state.0.clone();
    blocking(move || {
        let result =
            core.device_unlock
                .configure_for(incarnation, password, touch_id, mac_password);
        use tauri_specta::Event;
        let _ = crate::events::SettingsChanged::of(&core.settings.get()).emit(&app);
        result
    })
    .await
}
#[tauri::command]
#[specta::specta]
pub fn device_unlock_consume_extension_request(state: State<'_, AppState>) -> bool {
    state.0.device_unlock.consume_extension_request()
}
/// Hidden command argument: validate the flat IPC envelope without copying its password.
pub struct ConfigurePayloadGuard;
impl specta::function::FunctionArg for ConfigurePayloadGuard {
    fn to_datatype(_: &mut specta::Types) -> Option<specta::datatype::DataType> {
        None
    }
}
fn configure_keys_valid(body: &tauri::ipc::InvokeBody) -> bool {
    matches!(body, tauri::ipc::InvokeBody::Json(serde_json::Value::Object(map))
        if map.keys().all(|k| matches!(k.as_str(), "password" | "touchId" | "macPassword")))
}
impl<'de, R: tauri::Runtime> tauri::ipc::CommandArg<'de, R> for ConfigurePayloadGuard {
    fn from_command(
        command: tauri::ipc::CommandItem<'de, R>,
    ) -> Result<Self, tauri::ipc::InvokeError> {
        if configure_keys_valid(command.message.payload()) {
            Ok(Self)
        } else {
            Err("unknown device configuration field".into())
        }
    }
}
#[cfg(test)]
mod payload_tests {
    use super::*;
    #[test]
    fn device_commands_reject_unknown_field_in_configure_payload() {
        assert!(configure_keys_valid(&tauri::ipc::InvokeBody::Json(
            serde_json::json!({"password":null,"touchId":true,"macPassword":false})
        )));
        assert!(!configure_keys_valid(&tauri::ipc::InvokeBody::Json(
            serde_json::json!({"touchId":true,"unexpected":true})
        )));
    }
}

#[cfg(test)]
mod tests {
    use crate::testing::world;
    #[test]
    fn consume_command_returns_true_once_within_ten_seconds_then_false() {
        let w = world();
        w.core.unlock_requests.note();
        assert!(w.core.device_unlock.consume_extension_request());
        assert!(!w.core.device_unlock.consume_extension_request());
        w.core.unlock_requests.note();
        w.clock.advance(std::time::Duration::from_secs(11));
        assert!(!w.core.device_unlock.consume_extension_request());
    }
}
