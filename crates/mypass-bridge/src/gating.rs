//! Gating rules: which requests are answered while the vault is locked, which need a paired
//! browser, which count as user activity for the idle auto-lock, and which responses count as
//! "served".
//!
//! Every `match` below is exhaustive with no catch-all arm (clippy enforces it through
//! `wildcard_enum_match_arm`, and a test scans the source): a new message must make an explicit
//! decision here rather than silently inheriting one.

#![deny(clippy::wildcard_enum_match_arm)]

use crate::protocol::{Request, Response};

impl Request {
    /// The endpoint token every request carries.
    pub fn token(&self) -> &str {
        match self {
            Self::GetCredentials { token, .. }
            | Self::GetFavorites { token, .. }
            | Self::ListEntries { token, .. }
            | Self::SearchEntries { token, .. }
            | Self::CreateEntry { token, .. }
            | Self::FillAddress { token, .. }
            | Self::FillCredential { token, .. }
            | Self::GetTotp { token, .. }
            | Self::ProposeSaveLogin { token, .. }
            | Self::AddEntryIdentifier { token, .. }
            | Self::ProposeSaveTotp { token, .. }
            | Self::StartPairing { token, .. }
            | Self::CompletePairing { token, .. }
            | Self::RequestUnlock { token, .. }
            | Self::LockVault { token, .. }
            | Self::OpenApp { token, .. }
            | Self::OpenBrowserSettings { token, .. }
            | Self::ReportBrowserDiagnostics { token, .. }
            | Self::GetPasskeyProxyStatus { token, .. }
            | Self::SetPasskeyProxyEnabled { token, .. }
            | Self::PasskeyCreate { token, .. }
            | Self::PasskeyCreateConfirm { token, .. }
            | Self::PasskeyGet { token, .. }
            | Self::PasskeyGetApprove { token, .. }
            | Self::PasskeyCancel { token, .. }
            | Self::GeneratePassword { token, .. }
            | Self::RecordGeneratedPassword { token, .. }
            | Self::GetGeneratedPasswordHistory { token, .. }
            | Self::DeleteGeneratedPassword { token, .. } => token,
        }
    }

    /// The pairing secret, when the request carries one. The two handshake requests have none.
    pub fn pairing(&self) -> Option<&str> {
        match self {
            Self::StartPairing { .. } | Self::CompletePairing { .. } => None,
            Self::GetCredentials { pairing, .. }
            | Self::GetFavorites { pairing, .. }
            | Self::ListEntries { pairing, .. }
            | Self::SearchEntries { pairing, .. }
            | Self::CreateEntry { pairing, .. }
            | Self::FillAddress { pairing, .. }
            | Self::FillCredential { pairing, .. }
            | Self::GetTotp { pairing, .. }
            | Self::ProposeSaveLogin { pairing, .. }
            | Self::AddEntryIdentifier { pairing, .. }
            | Self::ProposeSaveTotp { pairing, .. }
            | Self::RequestUnlock { pairing, .. }
            | Self::LockVault { pairing, .. }
            | Self::OpenApp { pairing, .. }
            | Self::OpenBrowserSettings { pairing, .. }
            | Self::ReportBrowserDiagnostics { pairing, .. }
            | Self::GetPasskeyProxyStatus { pairing, .. }
            | Self::SetPasskeyProxyEnabled { pairing, .. }
            | Self::PasskeyCreate { pairing, .. }
            | Self::PasskeyCreateConfirm { pairing, .. }
            | Self::PasskeyGet { pairing, .. }
            | Self::PasskeyGetApprove { pairing, .. }
            | Self::PasskeyCancel { pairing, .. }
            | Self::GeneratePassword { pairing, .. }
            | Self::RecordGeneratedPassword { pairing, .. }
            | Self::GetGeneratedPasswordHistory { pairing, .. }
            | Self::DeleteGeneratedPassword { pairing, .. } => pairing.as_deref(),
        }
    }

    /// The wire `type` string of this request.
    pub fn name(&self) -> &'static str {
        match self {
            Self::GetCredentials { .. } => "getCredentials",
            Self::GetFavorites { .. } => "getFavorites",
            Self::ListEntries { .. } => "listEntries",
            Self::SearchEntries { .. } => "searchEntries",
            Self::CreateEntry { .. } => "createEntry",
            Self::FillAddress { .. } => "fillAddress",
            Self::FillCredential { .. } => "fillCredential",
            Self::GetTotp { .. } => "getTotp",
            Self::ProposeSaveLogin { .. } => "proposeSaveLogin",
            Self::AddEntryIdentifier { .. } => "addEntryIdentifier",
            Self::ProposeSaveTotp { .. } => "proposeSaveTotp",
            Self::StartPairing { .. } => "startPairing",
            Self::CompletePairing { .. } => "completePairing",
            Self::RequestUnlock { .. } => "requestUnlock",
            Self::LockVault { .. } => "lockVault",
            Self::OpenApp { .. } => "openApp",
            Self::OpenBrowserSettings { .. } => "openBrowserSettings",
            Self::ReportBrowserDiagnostics { .. } => "reportBrowserDiagnostics",
            Self::GetPasskeyProxyStatus { .. } => "getPasskeyProxyStatus",
            Self::SetPasskeyProxyEnabled { .. } => "setPasskeyProxyEnabled",
            Self::PasskeyCreate { .. } => "passkeyCreate",
            Self::PasskeyCreateConfirm { .. } => "passkeyCreateConfirm",
            Self::PasskeyGet { .. } => "passkeyGet",
            Self::PasskeyGetApprove { .. } => "passkeyGetApprove",
            Self::PasskeyCancel { .. } => "passkeyCancel",
            Self::GeneratePassword { .. } => "generatePassword",
            Self::RecordGeneratedPassword { .. } => "recordGeneratedPassword",
            Self::GetGeneratedPasswordHistory { .. } => "getGeneratedPasswordHistory",
            Self::DeleteGeneratedPassword { .. } => "deleteGeneratedPassword",
        }
    }

    /// Only these requests may be answered while the vault is locked: asking for an unlock, the
    /// passkey proxy status poll (the extension must learn the vault is locked), and the
    /// app-level actions that touch no vault data.
    pub fn is_answerable_while_locked(&self) -> bool {
        match self {
            Self::RequestUnlock { .. }
            | Self::GetPasskeyProxyStatus { .. }
            | Self::OpenBrowserSettings { .. }
            | Self::ReportBrowserDiagnostics { .. }
            | Self::OpenApp { .. } => true,
            Self::GetCredentials { .. }
            | Self::GetFavorites { .. }
            | Self::ListEntries { .. }
            | Self::SearchEntries { .. }
            | Self::CreateEntry { .. }
            | Self::FillAddress { .. }
            | Self::FillCredential { .. }
            | Self::GetTotp { .. }
            | Self::ProposeSaveLogin { .. }
            | Self::AddEntryIdentifier { .. }
            | Self::ProposeSaveTotp { .. }
            | Self::StartPairing { .. }
            | Self::CompletePairing { .. }
            | Self::LockVault { .. }
            | Self::SetPasskeyProxyEnabled { .. }
            | Self::PasskeyCreate { .. }
            | Self::PasskeyCreateConfirm { .. }
            | Self::PasskeyGet { .. }
            | Self::PasskeyGetApprove { .. }
            | Self::PasskeyCancel { .. }
            | Self::GeneratePassword { .. }
            | Self::RecordGeneratedPassword { .. }
            | Self::GetGeneratedPasswordHistory { .. }
            | Self::DeleteGeneratedPassword { .. } => false,
        }
    }

    /// Whether handling this request should reset the app's idle auto-lock timer.
    ///
    /// A user working entirely in the browser extension never touches the MyPass window, so the
    /// idle timer's only other reset source (window key and mouse events) never fires for them and
    /// the vault would silently re-lock under an active user. The fix is to reset on extension
    /// traffic too, but ONLY on requests that cannot be produced without a deliberate user
    /// gesture. Background polling and anything a page can trigger on its own must NOT reset the
    /// timer: the content script fires `getCredentials` automatically whenever a login form is
    /// found (all frames, all URLs), and the background script forwards every
    /// `webAuthenticationProxy` create/get event, which page script triggers on its own timing
    /// through `navigator.credentials.create/get()` (including conditional mediation at page
    /// load, with no user gesture at all). Resetting on those would let a hostile or merely noisy
    /// background tab defeat idle auto-lock entirely.
    ///
    /// Classification rule: resets only if the request cannot be produced without a deliberate
    /// user gesture (a trusted click in the content script, or a popup button).
    pub fn resets_auto_lock_timer(&self) -> bool {
        match self {
            // Gesture-gated: a popup button click (`fillCredential`, `getTotp`,
            // `proposeSaveLogin`, `proposeSaveTotp`, `passkeyCreateConfirm`,
            // `passkeyGetApprove`, `requestUnlock`, the list and search screens and the
            // generator): none of these can fire on a page's own timing.
            Self::FillAddress { .. }
            | Self::FillCredential { .. }
            | Self::GetTotp { .. }
            | Self::ProposeSaveLogin { .. }
            | Self::ProposeSaveTotp { .. }
            | Self::PasskeyCreateConfirm { .. }
            | Self::PasskeyGetApprove { .. }
            | Self::RequestUnlock { .. }
            | Self::GeneratePassword { .. }
            | Self::GetFavorites { .. }
            | Self::ListEntries { .. }
            | Self::SearchEntries { .. }
            | Self::CreateEntry { .. }
            | Self::AddEntryIdentifier { .. }
            | Self::RecordGeneratedPassword { .. }
            | Self::GetGeneratedPasswordHistory { .. }
            | Self::DeleteGeneratedPassword { .. } => true,
            // Page-triggerable, no user gesture required: the automatic credential lookup on any
            // detected login form, and the passkey create/get events forwarded for
            // page-initiated `navigator.credentials` calls.
            Self::GetCredentials { .. } | Self::PasskeyCreate { .. } | Self::PasskeyGet { .. } => {
                false
            }
            // Can all fire without the user acting: status polling, the app-side toggle mirrored
            // from Settings, the pairing handshake driven by the browser side and the
            // diagnostics report. `lockVault` is a popup gesture, but its job is to lock, so it
            // must not record activity and keep the idle timer alive.
            Self::GetPasskeyProxyStatus { .. }
            | Self::SetPasskeyProxyEnabled { .. }
            | Self::StartPairing { .. }
            | Self::CompletePairing { .. }
            | Self::PasskeyCancel { .. }
            | Self::OpenBrowserSettings { .. }
            | Self::ReportBrowserDiagnostics { .. }
            | Self::LockVault { .. }
            | Self::OpenApp { .. } => false,
        }
    }

    /// Whether the request needs a paired browser: everything except the two handshake requests.
    pub fn requires_pairing(&self) -> bool {
        match self {
            Self::StartPairing { .. } | Self::CompletePairing { .. } => false,
            Self::GetCredentials { .. }
            | Self::GetFavorites { .. }
            | Self::ListEntries { .. }
            | Self::SearchEntries { .. }
            | Self::CreateEntry { .. }
            | Self::FillAddress { .. }
            | Self::FillCredential { .. }
            | Self::GetTotp { .. }
            | Self::ProposeSaveLogin { .. }
            | Self::AddEntryIdentifier { .. }
            | Self::ProposeSaveTotp { .. }
            | Self::RequestUnlock { .. }
            | Self::LockVault { .. }
            | Self::OpenApp { .. }
            | Self::OpenBrowserSettings { .. }
            | Self::ReportBrowserDiagnostics { .. }
            | Self::GetPasskeyProxyStatus { .. }
            | Self::SetPasskeyProxyEnabled { .. }
            | Self::PasskeyCreate { .. }
            | Self::PasskeyCreateConfirm { .. }
            | Self::PasskeyGet { .. }
            | Self::PasskeyGetApprove { .. }
            | Self::PasskeyCancel { .. }
            | Self::GeneratePassword { .. }
            | Self::RecordGeneratedPassword { .. }
            | Self::GetGeneratedPasswordHistory { .. }
            | Self::DeleteGeneratedPassword { .. } => true,
        }
    }

    /// The seven passkey cases (the proxy status and toggle included).
    pub fn is_passkey(&self) -> bool {
        match self {
            Self::PasskeyCreate { .. }
            | Self::PasskeyCreateConfirm { .. }
            | Self::PasskeyGet { .. }
            | Self::PasskeyGetApprove { .. }
            | Self::PasskeyCancel { .. }
            | Self::GetPasskeyProxyStatus { .. }
            | Self::SetPasskeyProxyEnabled { .. } => true,
            Self::GetCredentials { .. }
            | Self::GetFavorites { .. }
            | Self::ListEntries { .. }
            | Self::SearchEntries { .. }
            | Self::CreateEntry { .. }
            | Self::FillAddress { .. }
            | Self::FillCredential { .. }
            | Self::GetTotp { .. }
            | Self::ProposeSaveLogin { .. }
            | Self::AddEntryIdentifier { .. }
            | Self::ProposeSaveTotp { .. }
            | Self::StartPairing { .. }
            | Self::CompletePairing { .. }
            | Self::RequestUnlock { .. }
            | Self::LockVault { .. }
            | Self::OpenApp { .. }
            | Self::OpenBrowserSettings { .. }
            | Self::ReportBrowserDiagnostics { .. }
            | Self::GeneratePassword { .. }
            | Self::RecordGeneratedPassword { .. }
            | Self::GetGeneratedPasswordHistory { .. }
            | Self::DeleteGeneratedPassword { .. } => false,
        }
    }
}

impl Response {
    /// Whether the app actually served the request: false for the refusals (`Locked`,
    /// `PairingRequired`, `Error`) and for a passkey completion that carries an error.
    pub fn was_served(&self) -> bool {
        match self {
            Self::Locked | Self::PairingRequired | Self::Error { .. } => false,
            Self::PasskeyCreateComplete { error, .. } | Self::PasskeyGetComplete { error, .. } => {
                error.is_none()
            }
            Self::PairingStarted
            | Self::Paired { .. }
            | Self::CredentialList { .. }
            | Self::Address { .. }
            | Self::Credential { .. }
            | Self::Totp { .. }
            | Self::SaveProposalAccepted
            | Self::UnlockPresented
            | Self::UnlockNotNeeded
            | Self::PasskeyProxyStatus { .. }
            | Self::PasskeyProxyEnabled { .. }
            | Self::PasskeyCreatePending { .. }
            | Self::PasskeyGetPending { .. }
            | Self::PasskeyCancelled { .. }
            | Self::GeneratedPassword { .. }
            | Self::EntryCreated
            | Self::Acknowledged
            | Self::PasswordHistory { .. } => true,
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)] // test-only
mod tests {
    use super::*;
    use crate::protocol::{PasskeyErrorDetails, REQUEST_TYPES};
    use serde_json::{json, Value};

    fn request_json(name: &str, pairing: Option<&str>) -> Value {
        // One superset of every field a variant may require; serde ignores the extras.
        let mut v = json!({
            "type": name,
            "token": "tok-1",
            "url": "https://example.com/",
            "category": "logins",
            "query": "q",
            "entryType": "login",
            "id": "id-1",
            "value": "v",
            "fieldKey": "k",
            "otpauthOrSecret": "JBSWY3DPEHPK3PXP",
            "browser": "chrome",
            "code": "123456",
            "enabled": true,
            "requestId": 7,
            "requestDetailsJson": "{}",
            "originUrl": "https://example.com",
            "entryId": "e-1",
            "domain": "example.com",
            "username": "u",
            "password": "p",
            "report": {
                "proxyState": "active",
                "permissionState": "granted",
                "extensionScanState": "complete",
                "enabledManagers": []
            },
        });
        if let (Some(p), Some(o)) = (pairing, v.as_object_mut()) {
            o.insert("pairing".into(), json!(p));
        }
        v
    }

    fn all_requests(pairing: Option<&str>) -> Vec<Request> {
        REQUEST_TYPES
            .iter()
            .map(|n| {
                serde_json::from_value(request_json(n, pairing))
                    .unwrap_or_else(|e| panic!("{n}: {e}"))
            })
            .collect()
    }

    fn names_where(f: impl Fn(&Request) -> bool) -> Vec<&'static str> {
        let mut v: Vec<&'static str> = all_requests(None)
            .iter()
            .filter(|r| f(r))
            .map(Request::name)
            .collect();
        v.sort_unstable();
        v
    }

    fn sorted(list: &[&'static str]) -> Vec<&'static str> {
        let mut v = list.to_vec();
        v.sort_unstable();
        v
    }

    #[test]
    fn answerable_while_locked_is_exactly_five() {
        let expected = sorted(&[
            "requestUnlock",
            "getPasskeyProxyStatus",
            "openBrowserSettings",
            "reportBrowserDiagnostics",
            "openApp",
        ]);
        assert_eq!(names_where(Request::is_answerable_while_locked), expected);
        assert_eq!(expected.len(), 5);
    }

    #[test]
    fn resets_timer_set_is_exactly_the_spec_list() {
        let yes = [
            "fillAddress",
            "fillCredential",
            "getTotp",
            "proposeSaveLogin",
            "proposeSaveTotp",
            "passkeyCreateConfirm",
            "passkeyGetApprove",
            "requestUnlock",
            "generatePassword",
            "getFavorites",
            "listEntries",
            "searchEntries",
            "createEntry",
            "addEntryIdentifier",
            "recordGeneratedPassword",
            "getGeneratedPasswordHistory",
            "deleteGeneratedPassword",
        ];
        let no = [
            "getCredentials",
            "passkeyCreate",
            "passkeyGet",
            "getPasskeyProxyStatus",
            "setPasskeyProxyEnabled",
            "startPairing",
            "completePairing",
            "passkeyCancel",
            "openBrowserSettings",
            "reportBrowserDiagnostics",
            "lockVault",
            "openApp",
        ];
        assert_eq!(yes.len() + no.len(), 29);
        assert_eq!(names_where(Request::resets_auto_lock_timer), sorted(&yes));
        let others = names_where(|r| !r.resets_auto_lock_timer());
        assert_eq!(others, sorted(&no));
    }

    #[test]
    fn pairing_required_for_all_but_the_two_handshake_requests() {
        let no = names_where(|r| !r.requires_pairing());
        assert_eq!(no, vec!["completePairing", "startPairing"]);
        assert_eq!(names_where(Request::requires_pairing).len(), 27);
    }

    #[test]
    fn passkey_set_has_seven_members() {
        let expected = sorted(&[
            "passkeyCreate",
            "passkeyCreateConfirm",
            "passkeyGet",
            "passkeyGetApprove",
            "passkeyCancel",
            "getPasskeyProxyStatus",
            "setPasskeyProxyEnabled",
        ]);
        assert_eq!(names_where(Request::is_passkey), expected);
        assert_eq!(expected.len(), 7);
    }

    #[test]
    fn token_and_pairing_accessors_cover_every_variant() {
        for r in all_requests(Some("secret-1")) {
            assert_eq!(r.token(), "tok-1", "{}", r.name());
            let handshake = !r.requires_pairing();
            let expected = if handshake { None } else { Some("secret-1") };
            assert_eq!(r.pairing(), expected, "{}", r.name());
        }
        for r in all_requests(None) {
            assert_eq!(r.pairing(), None, "{}", r.name());
        }
    }

    #[test]
    fn name_matches_serde_tag_for_all_29() {
        let all = all_requests(None);
        assert_eq!(all.len(), 29);
        for (r, expected) in all.iter().zip(REQUEST_TYPES) {
            assert_eq!(r.name(), expected);
            let v = serde_json::to_value(r).unwrap();
            assert_eq!(v.get("type").and_then(Value::as_str), Some(expected));
        }
    }

    fn err() -> PasskeyErrorDetails {
        PasskeyErrorDetails {
            name: "NotAllowedError".into(),
            message: "no".into(),
        }
    }

    #[test]
    fn was_served_table() {
        assert!(!Response::Locked.was_served());
        assert!(!Response::PairingRequired.was_served());
        assert!(!Response::Error {
            message: "x".into()
        }
        .was_served());
        let create = |error| Response::PasskeyCreateComplete {
            request_id: 1,
            response_json: None,
            error,
        };
        let get = |error| Response::PasskeyGetComplete {
            request_id: 1,
            response_json: None,
            error,
        };
        assert!(!create(Some(err())).was_served());
        assert!(create(None).was_served());
        assert!(!get(Some(err())).was_served());
        assert!(get(None).was_served());
        for r in [
            Response::PairingStarted,
            Response::Paired { secret: "s".into() },
            Response::CredentialList {
                credentials: vec![],
            },
            Response::Credential {
                username: "u".into(),
                password: "p".into(),
                totp: None,
                urls: vec![],
            },
            Response::Totp { code: "1".into() },
            Response::SaveProposalAccepted,
            Response::UnlockPresented,
            Response::UnlockNotNeeded,
            Response::PasskeyProxyStatus {
                unlocked: true,
                paired: true,
                attach_allowed: true,
                enabled: true,
            },
            Response::PasskeyProxyEnabled { enabled: true },
            Response::PasskeyCreatePending {
                request_id: 1,
                rp_id: "r".into(),
                rp_name: "n".into(),
                matches: vec![],
            },
            Response::PasskeyGetPending {
                request_id: 1,
                rp_id: "r".into(),
                matches: vec![],
            },
            Response::PasskeyCancelled { request_id: 1 },
            Response::GeneratedPassword {
                password: "p".into(),
            },
            Response::EntryCreated,
            Response::Acknowledged,
            Response::PasswordHistory { entries: vec![] },
        ] {
            assert!(r.was_served(), "{r:?}");
        }
    }

    #[test]
    fn no_wildcard_arm_in_gating_matches() {
        let src = include_str!("gating.rs");
        let production = src.split("#[cfg(test)]").next().unwrap_or(src);
        for (i, line) in production.lines().enumerate() {
            let t = line.trim_start();
            assert!(!t.starts_with("_ =>"), "catch-all arm at line {}", i + 1);
            assert!(
                !t.starts_with("| _ =>") && !t.starts_with("_ |"),
                "catch-all arm at line {}",
                i + 1
            );
        }
    }
}
