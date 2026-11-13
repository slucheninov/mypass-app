//! `browser_*`: the Browser pane's backend (paired browsers, pairing, host registration, save
//! proposals).
//!
//! `browser_status` never returns a secret or a hash. `browser_proposal_take` is the one place a
//! proposal's password crosses to the webview, after the user opened the proposal. A pairing store
//! failure is never described to the webview: [`pairing_api_error`] maps every `PairingError` to
//! the same `Internal` error.

use crate::bridge::{BridgeHandle, BridgeState};
use crate::state::AppState;
use mypass_bridge::pairing::{PairingChallenge, PairingError};
use mypass_bridge::protocol::BrowserDiagnosticsReport;
use mypass_bridge::ui::Proposal;
use mypass_platform::{BrowserRegistration, NativeHostRegistrar};
use mypass_session::dto::{EntryEditModel, EntryTypeDto};
use mypass_session::{ApiError, AppCore, ErrorCode};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tauri::{AppHandle, Manager, State};

/// Unix seconds as the `f64` Unix milliseconds every DTO timestamp uses (specta rejects 64-bit
/// integers).
#[allow(clippy::cast_precision_loss)] // a Unix time in milliseconds is far below 2^53
pub fn unix_ms(seconds: i64) -> f64 {
    seconds as f64 * 1000.0
}

/// A pairing code waiting for the browser. `expires_at` (Unix milliseconds) lets the UI count down and
/// then re-query `browser_status`.
#[derive(Serialize, Deserialize, specta::Type, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PairingChallengeDto {
    pub code: String,
    pub browser: String,
    pub expires_at: f64,
}

impl From<&PairingChallenge> for PairingChallengeDto {
    fn from(c: &PairingChallenge) -> Self {
        Self {
            code: c.code.clone(),
            browser: c.browser.clone(),
            expires_at: unix_ms(c.expires_at),
        }
    }
}

/// The latest report a paired browser sent (the enums as their wire strings).
#[derive(Serialize, specta::Type, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct BrowserDiagnosticsDto {
    pub proxy_state: String,
    pub permission_state: String,
    pub password_saving_enabled: Option<bool>,
    pub extension_scan_state: String,
    pub enabled_managers: Vec<String>,
    pub received_at: f64,
}

/// A paired browser. No secret, no hash.
#[derive(Serialize, specta::Type, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PairedBrowserDto {
    pub id: String,
    pub label: String,
    pub created_at: f64,
    pub diagnostics: Option<BrowserDiagnosticsDto>,
}

/// One browser's native messaging registration (`state`: `ok`, `missing`, `stale` or `error`).
#[derive(Serialize, specta::Type, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct BrowserRegistrationDto {
    pub id: String,
    pub display: String,
    pub state: String,
    pub manifest_path: Option<String>,
    pub detail: Option<String>,
}

/// Everything the Browser pane shows.
#[derive(Serialize, specta::Type, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct BrowserStatus {
    /// The loopback server is running.
    pub listening: bool,
    /// The stored list of paired browsers could not be read, so the bridge did not start; the pane
    /// offers `browser_reset_pairings`.
    pub pairing_store_failed: bool,
    pub paired: Vec<PairedBrowserDto>,
    pub pairing: Option<PairingChallengeDto>,
    pub registrations: Vec<BrowserRegistrationDto>,
    /// The host binary shipped with the app, when there is one.
    pub host_path: Option<String>,
}

/// Every `PairingError` is the same fixed `Internal` error: no store detail reaches the webview.
pub fn pairing_api_error(_: &PairingError) -> ApiError {
    super::internal("internal error")
}

fn wire<T: Serialize>(value: &T) -> String {
    match serde_json::to_value(value) {
        Ok(serde_json::Value::String(s)) => s,
        _ => String::new(),
    }
}

fn registration_dto(r: &BrowserRegistration) -> BrowserRegistrationDto {
    BrowserRegistrationDto {
        id: r.id.clone(),
        display: r.display.clone(),
        state: match r.state {
            mypass_platform::RegistrationState::Ok => "ok",
            mypass_platform::RegistrationState::Missing => "missing",
            mypass_platform::RegistrationState::Stale => "stale",
            mypass_platform::RegistrationState::Error => "error",
        }
        .to_owned(),
        manifest_path: r.manifest_path.as_ref().map(|p| p.display().to_string()),
        detail: r.detail.clone(),
    }
}

fn diagnostics_dto(report: &BrowserDiagnosticsReport, received_at: i64) -> BrowserDiagnosticsDto {
    BrowserDiagnosticsDto {
        proxy_state: wire(&report.proxy_state),
        permission_state: wire(&report.permission_state),
        password_saving_enabled: report.password_saving_enabled,
        extension_scan_state: wire(&report.extension_scan_state),
        enabled_managers: report.enabled_managers.clone(),
        received_at: unix_ms(received_at),
    }
}

/// The pane's state. Reads the registration rows from disk, so callers run it on the blocking pool.
pub fn status(
    handle: Option<&BridgeHandle>,
    store_failed: bool,
    registrar: &dyn NativeHostRegistrar,
    host_path: Option<&Path>,
) -> BrowserStatus {
    let (paired, pairing) = handle.map_or((Vec::new(), None), |h| {
        let paired = h
            .pairing
            .list()
            .iter()
            .map(|b| PairedBrowserDto {
                id: mypass_bridge::views::wire_id(b.id),
                label: b.label.clone(),
                created_at: unix_ms(b.created_at),
                diagnostics: h
                    .diagnostics
                    .get(b.id)
                    .map(|s| diagnostics_dto(&s.report, s.received_at)),
            })
            .collect();
        (paired, h.pairing.active().as_ref().map(Into::into))
    });
    BrowserStatus {
        listening: handle.is_some(),
        pairing_store_failed: store_failed,
        paired,
        pairing,
        registrations: registrar.status().iter().map(registration_dto).collect(),
        host_path: host_path.map(|p| p.display().to_string()),
    }
}

fn running(state: &BridgeState) -> Result<Arc<BridgeHandle>, ApiError> {
    state
        .handle()
        .ok_or_else(|| super::internal("the browser bridge is not running"))
}

/// Forgets one paired browser and its diagnostics; `BridgeChangedEvent` follows from the pairing
/// manager's change hook.
pub fn revoke(state: &BridgeState, id: &str) -> Result<(), ApiError> {
    let id = mypass_bridge::views::parse_wire_id(id).ok_or_else(|| ApiError {
        code: ErrorCode::InvalidInput,
        message: String::new(),
    })?;
    let handle = running(state)?;
    handle
        .pairing
        .revoke(id)
        .map_err(|e| pairing_api_error(&e))?;
    handle.diagnostics.remove(id);
    Ok(())
}

/// Drops the visible pairing code (the change hook lowers the window and tells the webview).
pub fn pairing_cancel(state: &BridgeState) -> Result<(), ApiError> {
    state.handle().inspect(|h| h.pairing.cancel());
    Ok(())
}

/// Registers the host with every installed browser again.
pub fn reinstall_host(
    state: &BridgeState,
    registrar: &dyn NativeHostRegistrar,
    bundled_host: Option<&Path>,
) -> Result<Vec<BrowserRegistrationDto>, ApiError> {
    let Some(bundled) = bundled_host else {
        return Err(ApiError {
            code: ErrorCode::NotFound,
            message: "the host binary is not bundled with this build".to_owned(),
        });
    };
    let report = registrar
        .register(bundled)
        .map_err(|_| super::internal("host registration failed"))?;
    let rows = report.registrations.iter().map(registration_dto).collect();
    state.record_registration(report);
    Ok(rows)
}

fn host_of(url: &str) -> String {
    mypass_core::matcher::host_of(url).unwrap_or_else(|| url.to_owned())
}

/// The editor model of a proposal. The proposal is spent by this call.
pub fn proposal_take(
    core: &AppCore,
    state: &BridgeState,
    id: &str,
) -> Result<EntryEditModel, ApiError> {
    // Not taken while locked: a lock clears the proposals anyway, and a failed read must not eat it.
    super::ensure_unlocked(core)?;
    let handle = running(state)?;
    let proposal = handle.proposals.take(id).ok_or(ApiError {
        code: ErrorCode::NotFound,
        message: String::new(),
    })?;
    let new_login = |url: &str| -> Result<EntryEditModel, ApiError> {
        let mut model = core.entry_new_model(EntryTypeDto::Login, None)?;
        model.title = host_of(url);
        model.urls = vec![url.to_owned()];
        Ok(model)
    };
    match proposal {
        Proposal::Login {
            url,
            username,
            password,
        } => {
            let mut model = new_login(&url)?;
            model.username = username;
            model.password = (*password).clone();
            Ok(model)
        }
        Proposal::Totp { url, otp, entry_id } => {
            let existing = match entry_id {
                Some(entry) => match core.entry_edit_model(&entry.simple().to_string()) {
                    Ok(model) => Some(model),
                    // The entry was deleted meanwhile: the secret still gets a new login.
                    Err(e) if e.code == ErrorCode::NotFound => None,
                    Err(e) => return Err(e),
                },
                None => None,
            };
            let mut model = match existing {
                Some(model) => model,
                None => new_login(&url)?,
            };
            model.totp = (*otp).clone();
            Ok(model)
        }
    }
}

/// Forgets every paired browser, including when the stored list is unreadable (the bridge then
/// did not start; it starts afterwards).
pub fn reset_pairings(state: &BridgeState) -> Result<(), ApiError> {
    state.reset_pairings().map_err(|e| pairing_api_error(&e))
}

/// The folder with the unpacked extension, from the places a build can hold it.
pub fn extension_folder_of(app: &AppHandle) -> Option<PathBuf> {
    let resources = app.path().resource_dir().ok();
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf));
    let dev_root = Path::new(env!("CARGO_MANIFEST_DIR")).parent();
    crate::bridge::extension_folder(resources.as_deref(), exe_dir.as_deref(), dev_root)
}

#[tauri::command]
#[specta::specta]
pub async fn browser_status(
    app_state: State<'_, AppState>,
    state: State<'_, BridgeState>,
) -> Result<BrowserStatus, ApiError> {
    let registrar = app_state.0.platform().host_registrar.clone();
    let (handle, failed, host) = (
        state.handle(),
        state.pairing_store_failed(),
        state.bundled_host(),
    );
    super::blocking(move || {
        Ok(status(
            handle.as_deref(),
            failed,
            registrar.as_ref(),
            host.as_deref(),
        ))
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub fn browser_revoke(state: State<'_, BridgeState>, id: String) -> Result<(), ApiError> {
    revoke(&state, &id)
}

#[tauri::command]
#[specta::specta]
pub fn browser_pairing_cancel(state: State<'_, BridgeState>) -> Result<(), ApiError> {
    pairing_cancel(&state)
}

#[tauri::command]
#[specta::specta]
pub fn browser_reset_pairings(state: State<'_, BridgeState>) -> Result<(), ApiError> {
    reset_pairings(&state)
}

#[tauri::command]
#[specta::specta]
pub async fn browser_reinstall_host(
    app_state: State<'_, AppState>,
    state: State<'_, BridgeState>,
) -> Result<Vec<BrowserRegistrationDto>, ApiError> {
    let registrar = app_state.0.platform().host_registrar.clone();
    let bridge = state.inner().clone();
    super::blocking(move || {
        let host = bridge.bundled_host();
        reinstall_host(&bridge, registrar.as_ref(), host.as_deref())
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub fn browser_proposal_take(
    app_state: State<'_, AppState>,
    state: State<'_, BridgeState>,
    id: String,
) -> Result<EntryEditModel, ApiError> {
    proposal_take(&app_state.0, &state, &id)
}

/// Reveals the extension folder with the same helper as `app_reveal_backups`.
#[tauri::command]
#[specta::specta]
pub fn browser_show_extension_folder(app: AppHandle) -> Result<(), ApiError> {
    let folder = extension_folder_of(&app).ok_or(ApiError {
        code: ErrorCode::NotFound,
        message: String::new(),
    })?;
    tauri_plugin_opener::reveal_item_in_dir(folder).map_err(|e| super::internal(e.to_string()))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use crate::bridge::test_support::{fixture, pair, unstarted};
    use mypass_bridge::protocol::{ExtensionScanState, PermissionState, ProxyState};
    use mypass_platform::testing::FakeHostRegistrar;
    use zeroize::Zeroizing;

    fn report() -> BrowserDiagnosticsReport {
        BrowserDiagnosticsReport {
            proxy_state: ProxyState::Active,
            permission_state: PermissionState::Granted,
            password_saving_enabled: Some(false),
            extension_scan_state: ExtensionScanState::Complete,
            enabled_managers: vec!["Other manager".into()],
        }
    }

    fn status_of(f: &crate::bridge::test_support::Fixture) -> BrowserStatus {
        status(
            f.state.handle().as_deref(),
            f.state.pairing_store_failed(),
            f.w.core.platform().host_registrar.as_ref(),
            None,
        )
    }

    #[test]
    fn browser_status_contains_no_secret_or_hash() {
        let f = fixture(true);
        let secret = pair(&f);
        let hash = mypass_bridge::pairing::sha256_hex(&secret);
        let id = f.handle().pairing.list()[0].id;
        f.handle().diagnostics.record(report(), id, 1_700_000_000);
        let status = status_of(&f);
        assert!(status.listening);
        assert_eq!(status.paired.len(), 1);
        assert_eq!(status.paired[0].label, "Chrome");
        assert_eq!(
            status.paired[0].diagnostics.as_ref().unwrap().proxy_state,
            "active"
        );
        let json = serde_json::to_string(&status).unwrap();
        for forbidden in [secret.as_str(), hash.as_str(), "secretHash", "secret_hash"] {
            assert!(!json.contains(forbidden), "status leaks {forbidden}");
        }
    }

    #[test]
    fn browser_status_shows_the_pairing_code_with_its_expiry() {
        let f = fixture(true);
        f.handle().pairing.start(Some("Chrome"));
        let pairing = status_of(&f).pairing.unwrap();
        assert_eq!(pairing.code.len(), 6);
        assert!((pairing.expires_at - 1_700_000_120_000.0).abs() < 0.5);
    }

    #[test]
    fn browser_status_of_a_bridge_that_did_not_start_says_so() {
        let f = unstarted(true, None);
        let status = status_of(&f);
        assert!(!status.listening);
        assert!(status.paired.is_empty());
    }

    #[test]
    fn browser_revoke_removes_pairing_and_diagnostics() {
        let f = fixture(true);
        let secret = pair(&f);
        let other = pair(&f);
        let id = f.handle().pairing.list()[0].id;
        let other_id = f.handle().pairing.list()[1].id;
        f.handle().diagnostics.record(report(), id, 1);
        f.handle().diagnostics.record(report(), other_id, 1);
        let changed = f.events.count("bridge_changed");
        revoke(&f.state, &mypass_bridge::views::wire_id(id)).unwrap();
        assert!(!f.handle().pairing.is_authorized(Some(&secret)));
        assert!(f.handle().pairing.is_authorized(Some(&other)));
        assert!(f.handle().diagnostics.get(id).is_none());
        assert!(
            f.handle().diagnostics.get(other_id).is_some(),
            "only that browser's"
        );
        assert!(
            f.events.count("bridge_changed") > changed,
            "BridgeChangedEvent went out"
        );
    }

    #[test]
    fn browser_revoke_rejects_a_malformed_id() {
        let f = fixture(true);
        assert_eq!(
            revoke(&f.state, "nope").unwrap_err().code,
            ErrorCode::InvalidInput
        );
    }

    #[test]
    fn browser_pairing_cancel_drops_the_code_and_lowers_the_window() {
        let f = fixture(true);
        f.handle()
            .service
            .handle(mypass_bridge::protocol::Request::StartPairing {
                token: "t".into(),
                browser: None,
            });
        pairing_cancel(&f.state).unwrap();
        assert!(f.handle().pairing.active().is_none());
        assert_eq!(f.on_top(), [true, false]);
    }

    #[test]
    fn every_pairing_error_maps_to_the_same_fixed_internal_error() {
        for e in [
            PairingError::Store("keychain says secret-detail".into()),
            PairingError::Corrupt,
            PairingError::TooMany,
        ] {
            let mapped = pairing_api_error(&e);
            assert_eq!(mapped.code, ErrorCode::Internal);
            assert_eq!(mapped.message, "internal error");
        }
    }

    #[test]
    fn reset_pairings_forgets_every_browser_and_their_diagnostics() {
        let f = fixture(true);
        let secret = pair(&f);
        let id = f.handle().pairing.list()[0].id;
        f.handle().diagnostics.record(report(), id, 1);
        reset_pairings(&f.state).unwrap();
        assert!(f.handle().pairing.list().is_empty());
        assert!(!f.handle().pairing.is_authorized(Some(&secret)));
        assert!(f.handle().diagnostics.get(id).is_none());
    }

    fn put_login(f: &crate::bridge::test_support::Fixture) -> String {
        f.handle().proposals.put(Proposal::Login {
            url: "https://www.example.com/login".into(),
            username: "me".into(),
            password: Zeroizing::new("hunter2".into()),
        })
    }

    #[test]
    fn proposal_take_returns_edit_model_once_and_then_not_found() {
        let f = fixture(true);
        let id = put_login(&f);
        let model = proposal_take(&f.w.core, &f.state, &id).unwrap();
        assert_eq!(model.password, "hunter2");
        let err = proposal_take(&f.w.core, &f.state, &id).unwrap_err();
        assert_eq!(err.code, ErrorCode::NotFound);
    }

    #[test]
    fn proposal_take_login_prefills_title_username_url() {
        let f = fixture(true);
        let id = put_login(&f);
        let model = proposal_take(&f.w.core, &f.state, &id).unwrap();
        assert_eq!(model.id, None);
        assert_eq!(model.entry_type, EntryTypeDto::Login);
        assert_eq!(model.title, "www.example.com");
        assert_eq!(model.username, "me");
        assert_eq!(model.urls, ["https://www.example.com/login"]);
        let root = f.w.core.sidebar_get().unwrap().root_group_id;
        assert_eq!(model.group_id, root);
    }

    #[test]
    fn proposal_take_totp_with_entry_sets_otp_on_that_entry_model() {
        let f = fixture(true);
        let mut model = f.w.core.entry_new_model(EntryTypeDto::Login, None).unwrap();
        model.title = "Mail".into();
        model.password = "pw".into();
        let entry = f.w.core.entry_save(model).unwrap();
        let entry_uuid = uuid::Uuid::parse_str(&entry).unwrap();
        let id = f.handle().proposals.put(Proposal::Totp {
            url: "https://mail.example".into(),
            otp: Zeroizing::new("JBSWY3DPEHPK3PXP".into()),
            entry_id: Some(entry_uuid),
        });
        let taken = proposal_take(&f.w.core, &f.state, &id).unwrap();
        assert_eq!(taken.id.as_deref(), Some(entry.as_str()));
        assert_eq!(taken.title, "Mail");
        assert_eq!(taken.totp, "JBSWY3DPEHPK3PXP");
    }

    #[test]
    fn proposal_take_totp_without_entry_is_a_new_login_with_the_otp() {
        let f = fixture(true);
        let id = f.handle().proposals.put(Proposal::Totp {
            url: "https://mail.example/x".into(),
            otp: Zeroizing::new("JBSWY3DPEHPK3PXP".into()),
            entry_id: None,
        });
        let taken = proposal_take(&f.w.core, &f.state, &id).unwrap();
        assert_eq!(taken.id, None);
        assert_eq!(taken.title, "mail.example");
        assert_eq!(taken.totp, "JBSWY3DPEHPK3PXP");
    }

    #[test]
    fn proposal_take_while_locked_is_locked_and_keeps_nothing_alive() {
        let f = fixture(true);
        let id = put_login(&f);
        f.w.core.vault_lock().unwrap();
        assert_eq!(
            proposal_take(&f.w.core, &f.state, &id).unwrap_err().code,
            ErrorCode::Locked
        );
    }

    #[test]
    fn reinstall_host_reports_every_row_and_needs_a_bundled_host() {
        let f = fixture(true);
        let registrar: &FakeHostRegistrar = &f.w.fake.host_registrar;
        let rows = reinstall_host(&f.state, registrar, Some(Path::new("/h/mypass-host"))).unwrap();
        assert!(rows.iter().any(|r| r.state == "ok"));
        assert_eq!(registrar.register_calls(), 1);
        assert_eq!(
            reinstall_host(&f.state, registrar, None).unwrap_err().code,
            ErrorCode::NotFound
        );
        assert_eq!(registrar.register_calls(), 1);
    }

    #[test]
    fn reinstall_host_failure_is_a_fixed_internal_error() {
        let f = fixture(true);
        f.w.fake
            .host_registrar
            .fail_next(mypass_platform::PlatformError::Failed(
                "/secret/path".into(),
            ));
        let err = reinstall_host(
            &f.state,
            f.w.fake.host_registrar.as_ref(),
            Some(Path::new("/h/mypass-host")),
        )
        .unwrap_err();
        assert_eq!(err.code, ErrorCode::Internal);
        assert!(!err.message.contains("/secret"));
    }
}
