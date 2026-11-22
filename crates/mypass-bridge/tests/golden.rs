//! Golden JSON tests of the wire protocol (ports of the Swift `ExtensionBridgeMessagesTests`).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)] // test-only support code

use std::collections::BTreeSet;

use mypass_bridge::protocol::*;
use serde_json::{json, Value};

fn s(v: &str) -> String {
    v.to_owned()
}

fn so(v: &str) -> Option<String> {
    Some(v.to_owned())
}

/// Structural equality that compares every number as `f64`.
fn json_eq(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64(),
        (Value::Array(x), Value::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(p, q)| json_eq(p, q))
        }
        (Value::Object(x), Value::Object(y)) => {
            x.len() == y.len()
                && x.iter()
                    .all(|(k, v)| y.get(k).is_some_and(|w| json_eq(v, w)))
        }
        _ => a == b,
    }
}

fn report() -> BrowserDiagnosticsReport {
    BrowserDiagnosticsReport {
        proxy_state: ProxyState::AttachmentError,
        permission_state: PermissionState::NotGranted,
        password_saving_enabled: Some(false),
        extension_scan_state: ExtensionScanState::NotChecked,
        enabled_managers: vec![s("Chrome")],
    }
}

fn request_cases() -> Vec<(Request, &'static str)> {
    let t = || s("tok-1");
    vec![
        (
            Request::GetCredentials {
                token: t(),
                url: s("https://example.com"),
                pairing: None,
            },
            r#"{"type":"getCredentials","token":"tok-1","url":"https://example.com"}"#,
        ),
        (
            Request::GetFavorites {
                token: t(),
                pairing: so("pair"),
            },
            r#"{"type":"getFavorites","token":"tok-1","pairing":"pair"}"#,
        ),
        (
            Request::ListEntries {
                token: t(),
                category: s("addresses"),
                pairing: None,
            },
            r#"{"type":"listEntries","token":"tok-1","category":"addresses"}"#,
        ),
        (
            Request::SearchEntries {
                token: t(),
                query: s("bank"),
                pairing: None,
            },
            r#"{"type":"searchEntries","token":"tok-1","query":"bank"}"#,
        ),
        (
            Request::CreateEntry {
                token: t(),
                entry_type: s("Login"),
                title: s("T"),
                username: s("u"),
                password: s("p"),
                url: s("https://a.example"),
                card_number: s("4111"),
                cardholder_name: s("N"),
                expiry: s("12/30"),
                cvv: s("123"),
                notes: s("n"),
                favorite: true,
                r#match: s("m"),
                otp: s("otpauth://x"),
                pairing: None,
            },
            r#"{"type":"createEntry","token":"tok-1","entryType":"Login","title":"T","username":"u","password":"p","url":"https://a.example","cardNumber":"4111","cardholderName":"N","expiry":"12/30","cvv":"123","notes":"n","favorite":true,"match":"m","otp":"otpauth://x"}"#,
        ),
        (
            Request::FillAddress {
                token: t(),
                id: s("a1"),
                url: s("https://x.example"),
                pairing: None,
            },
            r#"{"type":"fillAddress","token":"tok-1","id":"a1","url":"https://x.example"}"#,
        ),
        (
            Request::FillPerson {
                token: t(),
                id: s("p1"),
                url: s("https://x.example"),
                address_id: None,
                pairing: None,
            },
            r#"{"type":"fillPerson","token":"tok-1","id":"p1","url":"https://x.example"}"#,
        ),
        (
            Request::FillCredential {
                token: t(),
                id: s("c1"),
                url: s("https://x.example"),
                pairing: None,
            },
            r#"{"type":"fillCredential","token":"tok-1","id":"c1","url":"https://x.example"}"#,
        ),
        (
            Request::GetTotp {
                token: t(),
                id: s("c1"),
                url: s("https://x.example"),
                pairing: None,
            },
            r#"{"type":"getTotp","token":"tok-1","id":"c1","url":"https://x.example"}"#,
        ),
        (
            Request::ProposeSaveLogin {
                token: t(),
                url: s("https://x.example"),
                username: s("u"),
                password: s("p"),
                extra_fields: Vec::new(),
                username_is_email: false,
                pairing: None,
            },
            r#"{"type":"proposeSaveLogin","token":"tok-1","url":"https://x.example","username":"u","password":"p"}"#,
        ),
        (
            Request::AddEntryIdentifier {
                token: t(),
                id: s("c1"),
                url: s("https://x.example"),
                value: s("+1555"),
                field_key: s("Phone"),
                pairing: None,
            },
            r#"{"type":"addEntryIdentifier","token":"tok-1","id":"c1","url":"https://x.example","value":"+1555","fieldKey":"Phone"}"#,
        ),
        (
            Request::ProposeSaveTotp {
                token: t(),
                url: s("https://x.example"),
                otpauth_or_secret: s("JBSWY3DP"),
                entry_id: so("c1"),
                pairing: None,
            },
            r#"{"type":"proposeSaveTotp","token":"tok-1","url":"https://x.example","otpauthOrSecret":"JBSWY3DP","entryId":"c1"}"#,
        ),
        (
            Request::StartPairing {
                token: s("tok-p"),
                browser: so("Google Chrome"),
            },
            r#"{"type":"startPairing","token":"tok-p","browser":"Google Chrome"}"#,
        ),
        (
            Request::CompletePairing {
                token: t(),
                code: s("123456"),
            },
            r#"{"type":"completePairing","token":"tok-1","code":"123456"}"#,
        ),
        (
            Request::RequestUnlock {
                token: t(),
                pairing: None,
            },
            r#"{"type":"requestUnlock","token":"tok-1"}"#,
        ),
        (
            Request::LockVault {
                token: t(),
                pairing: None,
            },
            r#"{"type":"lockVault","token":"tok-1"}"#,
        ),
        (
            Request::OpenApp {
                token: t(),
                new_entry: None,
                pairing: None,
            },
            r#"{"type":"openApp","token":"tok-1"}"#,
        ),
        (
            Request::OpenBrowserSettings {
                token: t(),
                pairing: None,
            },
            r#"{"type":"openBrowserSettings","token":"tok-1"}"#,
        ),
        (
            Request::ReportBrowserDiagnostics {
                token: t(),
                report: report(),
                pairing: None,
            },
            r#"{"type":"reportBrowserDiagnostics","token":"tok-1","report":{"proxyState":"attachment-error","permissionState":"not-granted","passwordSavingEnabled":false,"extensionScanState":"not-checked","enabledManagers":["Chrome"]}}"#,
        ),
        (
            Request::GetPasskeyProxyStatus {
                token: t(),
                pairing: None,
            },
            r#"{"type":"getPasskeyProxyStatus","token":"tok-1"}"#,
        ),
        (
            Request::SetPasskeyProxyEnabled {
                token: t(),
                enabled: true,
                pairing: None,
            },
            r#"{"type":"setPasskeyProxyEnabled","token":"tok-1","enabled":true}"#,
        ),
        (
            Request::PasskeyCreate {
                token: t(),
                request_id: -7,
                request_details_json: s("{}"),
                origin_url: s("https://x.example"),
                pairing: None,
            },
            r#"{"type":"passkeyCreate","token":"tok-1","requestId":-7,"requestDetailsJson":"{}","originUrl":"https://x.example"}"#,
        ),
        (
            Request::PasskeyCreateConfirm {
                token: t(),
                request_id: 3,
                entry_id: so("e1"),
                replace: true,
                pairing: None,
            },
            r#"{"type":"passkeyCreateConfirm","token":"tok-1","requestId":3,"entryId":"e1","replace":true}"#,
        ),
        (
            Request::PasskeyGet {
                token: t(),
                request_id: 4,
                request_details_json: s("{}"),
                origin_url: s("https://x.example"),
                pairing: None,
            },
            r#"{"type":"passkeyGet","token":"tok-1","requestId":4,"requestDetailsJson":"{}","originUrl":"https://x.example"}"#,
        ),
        (
            Request::PasskeyGetApprove {
                token: t(),
                request_id: 4,
                entry_id: s("e1"),
                pairing: None,
            },
            r#"{"type":"passkeyGetApprove","token":"tok-1","requestId":4,"entryId":"e1"}"#,
        ),
        (
            Request::PasskeyCancel {
                token: t(),
                request_id: 4,
                pairing: None,
            },
            r#"{"type":"passkeyCancel","token":"tok-1","requestId":4}"#,
        ),
        (
            Request::GeneratePassword {
                token: t(),
                domain: s("example.com"),
                pairing: None,
            },
            r#"{"type":"generatePassword","token":"tok-1","domain":"example.com"}"#,
        ),
        (
            Request::RecordGeneratedPassword {
                token: t(),
                password: s("pw"),
                url: s("https://x.example"),
                created_at: Some(1_700_000_000_000.0),
                pairing: None,
            },
            r#"{"type":"recordGeneratedPassword","token":"tok-1","password":"pw","url":"https://x.example","createdAt":1700000000000}"#,
        ),
        (
            Request::GetGeneratedPasswordHistory {
                token: t(),
                pairing: None,
            },
            r#"{"type":"getGeneratedPasswordHistory","token":"tok-1"}"#,
        ),
        (
            Request::DeleteGeneratedPassword {
                token: t(),
                id: s("h1"),
                pairing: None,
            },
            r#"{"type":"deleteGeneratedPassword","token":"tok-1","id":"h1"}"#,
        ),
        (
            Request::UnlockWithPassword {
                token: t(),
                password: WirePassword::new(s("pw")),
                pairing: None,
            },
            r#"{"type":"unlockWithPassword","token":"tok-1","password":"pw"}"#,
        ),
        (
            Request::UnlockWithDevice {
                token: t(),
                pairing: None,
            },
            r#"{"type":"unlockWithDevice","token":"tok-1"}"#,
        ),
        (
            Request::GetUnlockOptions {
                token: t(),
                pairing: None,
            },
            r#"{"type":"getUnlockOptions","token":"tok-1"}"#,
        ),
    ]
}

#[test]
fn every_request_case_has_golden_json() {
    let cases = request_cases();
    assert_eq!(
        cases.len(),
        variant_count("Request"),
        "a Request variant has no golden case"
    );
    for (request, golden) in cases {
        let golden_value: Value = serde_json::from_str(golden).unwrap();
        let encoded = serde_json::to_value(&request).unwrap();
        assert!(
            json_eq(&encoded, &golden_value),
            "encode {golden}\n got {encoded}"
        );
        let decoded: Request = serde_json::from_str(golden).unwrap();
        assert_eq!(decoded, request, "decode {golden}");
    }
}

#[test]
fn fill_person_carries_the_picked_address_id() {
    let golden = r#"{"type":"fillPerson","token":"tok-1","id":"p1","url":"https://x.example","addressId":"a1"}"#;
    let request = Request::FillPerson {
        token: s("tok-1"),
        id: s("p1"),
        url: s("https://x.example"),
        address_id: so("a1"),
        pairing: None,
    };
    assert_eq!(serde_json::from_str::<Request>(golden).unwrap(), request);
    let encoded = serde_json::to_value(&request).unwrap();
    assert!(json_eq(&encoded, &serde_json::from_str(golden).unwrap()));
}

#[test]
fn request_types_are_distinct_and_match_the_tags() {
    let set: BTreeSet<&str> = REQUEST_TYPES.iter().copied().collect();
    assert_eq!(set.len(), 33);
    assert_eq!(set.len(), variant_count("Request"));
    let tags: BTreeSet<String> = request_cases()
        .iter()
        .map(|(r, _)| {
            serde_json::to_value(r).unwrap()["type"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect();
    let expected: BTreeSet<String> = REQUEST_TYPES.iter().map(|t| (*t).to_owned()).collect();
    assert_eq!(tags, expected);
}

#[test]
fn unknown_request_type_fails_to_decode() {
    assert!(serde_json::from_str::<Request>(r#"{"type":"nope","token":"t"}"#).is_err());
}

#[test]
fn open_app_new_entry_round_trips_and_rejects_unknown_kinds() {
    for (kind, wire) in [
        (NewEntryKind::Address, "Address"),
        (NewEntryKind::Files, "Files"),
    ] {
        let golden =
            format!(r#"{{"type":"openApp","token":"t","newEntry":"{wire}","pairing":"p"}}"#);
        let request = Request::OpenApp {
            token: "t".into(),
            new_entry: Some(kind),
            pairing: Some("p".into()),
        };
        assert_eq!(serde_json::from_str::<Request>(&golden).unwrap(), request);
        let golden_value: Value = serde_json::from_str(&golden).unwrap();
        assert!(json_eq(
            &serde_json::to_value(&request).unwrap(),
            &golden_value
        ));
    }
    // Absent and null both mean "only show the window".
    for frame in [
        r#"{"type":"openApp","token":"t"}"#,
        r#"{"type":"openApp","token":"t","newEntry":null}"#,
    ] {
        assert_eq!(
            serde_json::from_str::<Request>(frame).unwrap(),
            Request::OpenApp {
                token: "t".into(),
                new_entry: None,
                pairing: None,
            }
        );
    }
    // Only the two kinds the bridge cannot create, spelled exactly; anything else is malformed.
    for bad in [
        "Login", "Card", "Notes", "address", "files", "ADDRESS", "", "Group",
    ] {
        let frame = format!(r#"{{"type":"openApp","token":"t","newEntry":"{bad}"}}"#);
        assert!(serde_json::from_str::<Request>(&frame).is_err(), "{bad}");
    }
    for bad in ["1", "true", "[]", r#"{"kind":"Address"}"#] {
        let frame = format!(r#"{{"type":"openApp","token":"t","newEntry":{bad}}}"#);
        assert!(serde_json::from_str::<Request>(&frame).is_err(), "{bad}");
    }
}

#[test]
fn propose_save_login_extra_fields_round_trip_and_reject_unknown_kinds() {
    let golden = r#"{"type":"proposeSaveLogin","token":"t","url":"https://x.example","username":"u","password":"p","extraFields":[{"kind":"givenName","label":"First name","value":"Ann"},{"kind":"addressLine1","label":"Street","value":"1 Main St"},{"kind":"other","label":"Nickname","value":"annie"}]}"#;
    let request = Request::ProposeSaveLogin {
        token: s("t"),
        url: s("https://x.example"),
        username: s("u"),
        password: s("p"),
        extra_fields: vec![
            ProposalExtraField {
                kind: ExtraFieldKind::GivenName,
                label: s("First name"),
                value: s("Ann"),
            },
            ProposalExtraField {
                kind: ExtraFieldKind::AddressLine1,
                label: s("Street"),
                value: s("1 Main St"),
            },
            ProposalExtraField {
                kind: ExtraFieldKind::Other,
                label: s("Nickname"),
                value: s("annie"),
            },
        ],
        username_is_email: false,
        pairing: None,
    };
    assert_eq!(serde_json::from_str::<Request>(golden).unwrap(), request);
    let golden_value: Value = serde_json::from_str(golden).unwrap();
    assert!(json_eq(
        &serde_json::to_value(&request).unwrap(),
        &golden_value
    ));
    // Every kind the extension sends decodes; anything else refuses the whole request.
    for kind in [
        "name",
        "givenName",
        "familyName",
        "email",
        "phone",
        "birthday",
        "username",
        "organization",
        "addressLine1",
        "addressLine2",
        "city",
        "region",
        "postalCode",
        "country",
        "other",
    ] {
        let frame = format!(
            r#"{{"type":"proposeSaveLogin","token":"t","url":"u","username":"u","password":"p","extraFields":[{{"kind":"{kind}","label":"L","value":"v"}}]}}"#
        );
        assert!(serde_json::from_str::<Request>(&frame).is_ok(), "{kind}");
    }
    for bad in [
        r#"[{"kind":"password","label":"L","value":"v"}]"#,
        r#"[{"kind":"cardNumber","label":"L","value":"v"}]"#,
        r#"[{"kind":"GivenName","label":"L","value":"v"}]"#,
        r#"[{"kind":"givenName","value":"v"}]"#,
        r#"[{"kind":"givenName","label":"L"}]"#,
        r#"[{"kind":"givenName","label":"L","value":1}]"#,
        r#"{"kind":"givenName","label":"L","value":"v"}"#,
    ] {
        let frame = format!(
            r#"{{"type":"proposeSaveLogin","token":"t","url":"u","username":"u","password":"p","extraFields":{bad}}}"#
        );
        assert!(serde_json::from_str::<Request>(&frame).is_err(), "{bad}");
    }
    // Debug never shows a label or a value.
    let debug = format!("{:?}", request);
    assert!(!debug.contains("Ann") && !debug.contains("annie"));
    let field = ProposalExtraField {
        kind: ExtraFieldKind::Phone,
        label: s("Phone"),
        value: s("+380501234567"),
    };
    let debug = format!("{field:?}");
    assert!(debug.contains("Phone") && !debug.contains("+380501234567"));
}

#[test]
fn missing_token_fails_to_decode() {
    assert!(serde_json::from_str::<Request>(r#"{"type":"lockVault"}"#).is_err());
}

#[test]
fn optional_fields_are_omitted_not_null() {
    let mut values: Vec<Value> = request_cases()
        .iter()
        .map(|(r, _)| serde_json::to_value(r).unwrap())
        .collect();
    for r in response_samples() {
        values.push(serde_json::to_value(&r).unwrap());
    }
    values.push(
        serde_json::to_value(ExtensionCredentialSummary {
            id: s("1"),
            title: s("t"),
            username: s("u"),
            has_totp: false,
            has_passkey: false,
            url: None,
            entry_type: None,
            identifiers: vec![],
            address_ids: vec![],
        })
        .unwrap(),
    );
    fn no_null(v: &Value) {
        match v {
            Value::Null => panic!("null on the wire"),
            Value::Array(a) => a.iter().for_each(no_null),
            Value::Object(o) => o.values().for_each(no_null),
            _ => {}
        }
    }
    values.iter().for_each(no_null);

    let omitted: Vec<(Value, &str)> = vec![
        (
            serde_json::to_value(Request::StartPairing {
                token: s("t"),
                browser: None,
            })
            .unwrap(),
            "browser",
        ),
        (
            serde_json::to_value(Request::LockVault {
                token: s("t"),
                pairing: None,
            })
            .unwrap(),
            "pairing",
        ),
        (
            serde_json::to_value(Request::ProposeSaveTotp {
                token: s("t"),
                url: s("u"),
                otpauth_or_secret: s("x"),
                entry_id: None,
                pairing: None,
            })
            .unwrap(),
            "entryId",
        ),
        (
            serde_json::to_value(Request::RecordGeneratedPassword {
                token: s("t"),
                password: s("p"),
                url: s("u"),
                created_at: None,
                pairing: None,
            })
            .unwrap(),
            "createdAt",
        ),
        (
            serde_json::to_value(Response::Credential {
                username: s("u"),
                password: s("p"),
                totp: None,
                urls: vec![],
            })
            .unwrap(),
            "totp",
        ),
        (
            serde_json::to_value(Response::Credential {
                username: s("u"),
                password: s("p"),
                totp: None,
                urls: vec![],
            })
            .unwrap(),
            "urls",
        ),
        (
            serde_json::to_value(Response::PasskeyGetComplete {
                request_id: 1,
                response_json: None,
                error: None,
            })
            .unwrap(),
            "responseJson",
        ),
        (
            serde_json::to_value(Response::PasskeyGetComplete {
                request_id: 1,
                response_json: None,
                error: None,
            })
            .unwrap(),
            "error",
        ),
    ];
    for (value, key) in omitted {
        assert!(value.get(key).is_none(), "{key} must be omitted: {value}");
    }
    let summary = serde_json::to_value(ExtensionCredentialSummary {
        id: s("1"),
        title: s("t"),
        username: s("u"),
        has_totp: false,
        has_passkey: false,
        url: None,
        entry_type: None,
        identifiers: vec![],
        address_ids: vec![],
    })
    .unwrap();
    for key in ["url", "entryType", "identifiers", "addressIds"] {
        assert!(summary.get(key).is_none(), "{key} must be omitted");
    }
}

#[test]
fn create_entry_decode_defaults_match_swift() {
    let r: Request =
        serde_json::from_str(r#"{"type":"createEntry","token":"t","entryType":"Login"}"#).unwrap();
    assert_eq!(
        r,
        Request::CreateEntry {
            token: s("t"),
            entry_type: s("Login"),
            title: s(""),
            username: s(""),
            password: s(""),
            url: s(""),
            card_number: s(""),
            cardholder_name: s(""),
            expiry: s(""),
            cvv: s(""),
            notes: s(""),
            favorite: false,
            r#match: s(""),
            otp: s(""),
            pairing: None,
        }
    );
    let r: Request =
        serde_json::from_str(r#"{"type":"passkeyCreateConfirm","token":"t","requestId":1}"#)
            .unwrap();
    assert_eq!(
        r,
        Request::PasskeyCreateConfirm {
            token: s("t"),
            request_id: 1,
            entry_id: None,
            replace: false,
            pairing: None
        }
    );
    let r: Request =
        serde_json::from_str(r#"{"type":"recordGeneratedPassword","token":"t"}"#).unwrap();
    assert_eq!(
        r,
        Request::RecordGeneratedPassword {
            token: s("t"),
            password: s(""),
            url: s(""),
            created_at: None,
            pairing: None
        }
    );
    let r: Request =
        serde_json::from_str(r#"{"type":"deleteGeneratedPassword","token":"t"}"#).unwrap();
    assert_eq!(
        r,
        Request::DeleteGeneratedPassword {
            token: s("t"),
            id: s(""),
            pairing: None
        }
    );
}

fn summary(identifiers: Vec<String>) -> ExtensionCredentialSummary {
    ExtensionCredentialSummary {
        id: s("e1"),
        title: s("Example"),
        username: s("u"),
        has_totp: true,
        has_passkey: false,
        url: so("https://example.com"),
        entry_type: so("Login"),
        identifiers,
        address_ids: vec![],
    }
}

fn person() -> ExtensionPerson {
    ExtensionPerson {
        id: s("p1"),
        label: s("Me"),
        full_name: s("Ada Lovelace"),
        given_name: s("Ada"),
        middle_name: s("King"),
        family_name: s("Lovelace"),
        email: s("a@example.com"),
        birth_date: s("1815-12-10"),
    }
}

fn address() -> ExtensionAddress {
    ExtensionAddress {
        id: s("a1"),
        label: s("Home"),
        full_name: s("Ada Lovelace"),
        given_name: s("Ada"),
        family_name: s("Lovelace"),
        organization: s("Org"),
        address_line1: s("1 Main St"),
        address_line2: s("Apt 2"),
        city: s("London"),
        region: s("LDN"),
        postal_code: s("N1"),
        country_code: s("GB"),
        country_name: s("United Kingdom"),
        phone: s("+44"),
        email: s("a@example.com"),
    }
}

fn response_samples() -> Vec<Response> {
    response_cases().into_iter().map(|(r, _)| r).collect()
}

/// Counts the variants of a protocol enum by scanning the source, so a variant added without a
/// golden case fails the coverage tests below.
fn variant_count(name: &str) -> usize {
    let src = include_str!("../src/protocol.rs");
    let start = src.find(&format!("pub enum {name} {{")).unwrap();
    src[start..]
        .lines()
        .skip(1)
        .take_while(|l| *l != "}")
        .filter(|l| {
            l.starts_with("    ") && l.chars().nth(4).is_some_and(|c| c.is_ascii_uppercase())
        })
        .count()
}

fn response_cases() -> Vec<(Response, Value)> {
    vec![
        (Response::Locked, json!({"type":"locked"})),
        (Response::PairingRequired, json!({"type":"pairingRequired"})),
        (Response::PairingStarted, json!({"type":"pairingStarted"})),
        (
            Response::Paired { secret: s("sec") },
            json!({"type":"paired","secret":"sec"}),
        ),
        (
            Response::CredentialList {
                credentials: vec![summary(vec![]), summary(vec![s("+1")])],
            },
            json!({"type":"credentialList","credentials":[
                {"id":"e1","title":"Example","username":"u","hasTotp":true,"hasPasskey":false,"url":"https://example.com","entryType":"Login"},
                {"id":"e1","title":"Example","username":"u","hasTotp":true,"hasPasskey":false,"url":"https://example.com","entryType":"Login","identifiers":["+1"]}]}),
        ),
        (
            Response::Address {
                address: Box::new(address()),
            },
            json!({"type":"address","address":{"id":"a1","label":"Home","fullName":"Ada Lovelace","givenName":"Ada","familyName":"Lovelace","organization":"Org","addressLine1":"1 Main St","addressLine2":"Apt 2","city":"London","region":"LDN","postalCode":"N1","countryCode":"GB","countryName":"United Kingdom","phone":"+44","email":"a@example.com"}}),
        ),
        (
            Response::Person {
                person: Box::new(person()),
                address: None,
            },
            json!({"type":"person","person":{"id":"p1","label":"Me","fullName":"Ada Lovelace","givenName":"Ada","middleName":"King","familyName":"Lovelace","email":"a@example.com","birthDate":"1815-12-10"}}),
        ),
        (
            Response::Person {
                person: Box::new(person()),
                address: Some(Box::new(address())),
            },
            json!({"type":"person","person":{"id":"p1","label":"Me","fullName":"Ada Lovelace","givenName":"Ada","middleName":"King","familyName":"Lovelace","email":"a@example.com","birthDate":"1815-12-10"},"address":{"id":"a1","label":"Home","fullName":"Ada Lovelace","givenName":"Ada","familyName":"Lovelace","organization":"Org","addressLine1":"1 Main St","addressLine2":"Apt 2","city":"London","region":"LDN","postalCode":"N1","countryCode":"GB","countryName":"United Kingdom","phone":"+44","email":"a@example.com"}}),
        ),
        (
            Response::Credential {
                username: s("u"),
                password: s("p"),
                totp: None,
                urls: vec![],
            },
            json!({"type":"credential","username":"u","password":"p"}),
        ),
        (
            Response::Credential {
                username: s("u"),
                password: s("p"),
                totp: so("123456"),
                urls: vec![s("https://a.example")],
            },
            json!({"type":"credential","username":"u","password":"p","totp":"123456","urls":["https://a.example"]}),
        ),
        (
            Response::Totp { code: s("123456") },
            json!({"type":"totp","code":"123456"}),
        ),
        (
            Response::SaveProposalAccepted,
            json!({"type":"saveProposalAccepted"}),
        ),
        (Response::UnlockPresented, json!({"type":"unlockPresented"})),
        (Response::UnlockNotNeeded, json!({"type":"unlockNotNeeded"})),
        (
            Response::PasskeyProxyStatus {
                unlocked: true,
                paired: true,
                attach_allowed: false,
                enabled: false,
            },
            json!({"type":"passkeyProxyStatus","unlocked":true,"paired":true,"attachAllowed":false,"enabled":false}),
        ),
        (
            Response::PasskeyProxyEnabled { enabled: true },
            json!({"type":"passkeyProxyEnabled","enabled":true}),
        ),
        (
            Response::PasskeyCreatePending {
                request_id: 5,
                rp_id: s("example.com"),
                rp_name: s("Example"),
                matches: vec![summary(vec![])],
            },
            json!({"type":"passkeyCreatePending","requestId":5,"rpId":"example.com","rpName":"Example","matches":[
                {"id":"e1","title":"Example","username":"u","hasTotp":true,"hasPasskey":false,"url":"https://example.com","entryType":"Login"}]}),
        ),
        (
            Response::PasskeyCreateComplete {
                request_id: 5,
                response_json: so("{}"),
                error: None,
            },
            json!({"type":"passkeyCreateComplete","requestId":5,"responseJson":"{}"}),
        ),
        (
            Response::PasskeyCreateComplete {
                request_id: 5,
                response_json: None,
                error: Some(PasskeyErrorDetails {
                    name: s("NotAllowedError"),
                    message: s("denied"),
                }),
            },
            json!({"type":"passkeyCreateComplete","requestId":5,"error":{"name":"NotAllowedError","message":"denied"}}),
        ),
        (
            Response::PasskeyGetPending {
                request_id: 6,
                rp_id: s("example.com"),
                matches: vec![],
            },
            json!({"type":"passkeyGetPending","requestId":6,"rpId":"example.com","matches":[]}),
        ),
        (
            Response::PasskeyGetComplete {
                request_id: 6,
                response_json: so("{}"),
                error: None,
            },
            json!({"type":"passkeyGetComplete","requestId":6,"responseJson":"{}"}),
        ),
        (
            Response::PasskeyCancelled { request_id: 6 },
            json!({"type":"passkeyCancelled","requestId":6}),
        ),
        (
            Response::GeneratedPassword { password: s("gen") },
            json!({"type":"generatedPassword","password":"gen"}),
        ),
        (Response::EntryCreated, json!({"type":"entryCreated"})),
        (Response::Acknowledged, json!({"type":"ok"})),
        (
            Response::PasswordHistory {
                entries: vec![ExtensionPasswordHistoryItem {
                    id: s("h1"),
                    password: s("pw"),
                    url: s("https://x.example"),
                    created_at: 1_700_000_000_000.0,
                }],
            },
            json!({"type":"generatedPasswordHistory","entries":[{"id":"h1","password":"pw","url":"https://x.example","createdAt":1700000000000.0}]}),
        ),
        (
            Response::Error {
                message: s("invalid_code"),
            },
            json!({"type":"error","message":"invalid_code"}),
        ),
        (Response::Unlocked, json!({"type":"unlocked"})),
        (Response::WrongPassword, json!({"type":"wrongPassword"})),
        (
            Response::UnlockThrottled {
                retry_after_secs: 30,
            },
            json!({"type":"unlockThrottled","retryAfterSecs":30}),
        ),
        (
            Response::DeviceUnlockUnavailable,
            json!({"type":"deviceUnlockUnavailable"}),
        ),
        (Response::UnlockCancelled, json!({"type":"unlockCancelled"})),
        (Response::UnlockInApp, json!({"type":"unlockInApp"})),
        (
            Response::UnlockOptions {
                device_unlock: true,
                touch_id: true,
                retry_after_secs: None,
            },
            json!({"type":"unlockOptions","deviceUnlock":true,"touchId":true}),
        ),
    ]
}

#[test]
fn every_response_case_has_golden_json() {
    for (response, golden) in response_cases() {
        let encoded = serde_json::to_value(&response).unwrap();
        assert!(
            json_eq(&encoded, &golden),
            "encode {golden}\n got {encoded}"
        );
        let decoded: Response = serde_json::from_value(golden.clone()).unwrap();
        assert_eq!(decoded, response, "decode {golden}");
    }
    let tags: BTreeSet<String> = response_cases()
        .iter()
        .map(|(_, g)| g["type"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(
        tags.len(),
        variant_count("Response"),
        "a Response variant has no golden case"
    );
}

#[test]
fn credential_list_never_contains_a_password_key() {
    let v = serde_json::to_value(Response::CredentialList {
        credentials: vec![summary(vec![])],
    })
    .unwrap();
    assert!(!v.to_string().contains("password"));
}

#[test]
fn passkey_proxy_status_decodes_without_enabled() {
    let r: Response = serde_json::from_str(
        r#"{"type":"passkeyProxyStatus","unlocked":true,"paired":false,"attachAllowed":true}"#,
    )
    .unwrap();
    assert_eq!(
        r,
        Response::PasskeyProxyStatus {
            unlocked: true,
            paired: false,
            attach_allowed: true,
            enabled: false
        }
    );
}

#[test]
fn passkey_responses_carry_no_private_key_field() {
    for r in response_samples() {
        let text = serde_json::to_string(&r).unwrap().to_lowercase();
        if text.contains("passkey") {
            assert!(
                !text.contains("privatekey") && !text.contains("private_key"),
                "{text}"
            );
        }
    }
}

#[test]
fn no_response_serialises_a_token_key() {
    fn has_token(v: &Value) -> bool {
        match v {
            Value::Array(a) => a.iter().any(has_token),
            Value::Object(o) => o.contains_key("token") || o.values().any(has_token),
            _ => false,
        }
    }
    for r in response_samples() {
        assert!(!has_token(&serde_json::to_value(&r).unwrap()));
    }
}

#[test]
fn diagnostics_report_enums_use_kebab_strings_and_reject_unknown() {
    let base = |proxy: &str, perm: &str, scan: &str| {
        format!(
            r#"{{"proxyState":"{proxy}","permissionState":"{perm}","extensionScanState":"{scan}","enabledManagers":[]}}"#
        )
    };
    for proxy in [
        "active",
        "disabled",
        "locked",
        "unpaired",
        "unavailable",
        "attachment-error",
    ] {
        let r: BrowserDiagnosticsReport =
            serde_json::from_str(&base(proxy, "granted", "complete")).unwrap();
        assert_eq!(serde_json::to_value(r.proxy_state).unwrap(), json!(proxy));
    }
    for perm in ["granted", "unavailable", "not-granted"] {
        let r: BrowserDiagnosticsReport =
            serde_json::from_str(&base("active", perm, "complete")).unwrap();
        assert_eq!(
            serde_json::to_value(r.permission_state).unwrap(),
            json!(perm)
        );
    }
    for scan in ["complete", "unavailable", "not-checked"] {
        let r: BrowserDiagnosticsReport =
            serde_json::from_str(&base("active", "granted", scan)).unwrap();
        assert_eq!(
            serde_json::to_value(r.extension_scan_state).unwrap(),
            json!(scan)
        );
    }
    assert!(serde_json::from_str::<BrowserDiagnosticsReport>(&base(
        "bogus", "granted", "complete"
    ))
    .is_err());
    assert!(
        serde_json::from_str::<BrowserDiagnosticsReport>(&base("active", "bogus", "complete"))
            .is_err()
    );
    assert!(
        serde_json::from_str::<BrowserDiagnosticsReport>(&base("active", "granted", "bogus"))
            .is_err()
    );
    // Camel-case spellings of the kebab values are rejected too.
    assert!(serde_json::from_str::<BrowserDiagnosticsReport>(&base(
        "attachmentError",
        "granted",
        "complete"
    ))
    .is_err());
}

#[test]
fn explicit_null_decodes_as_the_default() {
    let r: Request = serde_json::from_str(
        r#"{"type":"createEntry","token":"t","entryType":"Login","title":null,"favorite":null,"match":null,"otp":null}"#,
    )
    .unwrap();
    let Request::CreateEntry {
        title,
        favorite,
        r#match,
        otp,
        ..
    } = r
    else {
        panic!("wrong variant");
    };
    assert_eq!(
        (title.as_str(), favorite, r#match.as_str(), otp.as_str()),
        ("", false, "", "")
    );
    let r: Request = serde_json::from_str(
        r#"{"type":"recordGeneratedPassword","token":"t","password":null,"url":null}"#,
    )
    .unwrap();
    assert!(
        matches!(r, Request::RecordGeneratedPassword { ref password, .. } if password.is_empty())
    );
    let r: Request = serde_json::from_str(
        r#"{"type":"passkeyCreateConfirm","token":"t","requestId":1,"replace":null}"#,
    )
    .unwrap();
    assert!(matches!(
        r,
        Request::PasskeyCreateConfirm { replace: false, .. }
    ));
    let r: Request =
        serde_json::from_str(r#"{"type":"deleteGeneratedPassword","token":"t","id":null}"#)
            .unwrap();
    assert!(matches!(r, Request::DeleteGeneratedPassword { ref id, .. } if id.is_empty()));
}

#[test]
fn debug_output_prints_the_tag_only() {
    let requests = vec![
        Request::FillCredential {
            token: s("tok-secret"),
            id: s("entry-id-1"),
            url: s("https://x.example"),
            pairing: so("tok-pair"),
        },
        Request::ProposeSaveLogin {
            token: s("tok-2"),
            url: s("https://x.example"),
            username: s("alice-user"),
            password: s("hunter2"),
            extra_fields: Vec::new(),
            username_is_email: false,
            pairing: None,
        },
        Request::CompletePairing {
            token: s("tok-3"),
            code: s("987654"),
        },
        Request::ProposeSaveTotp {
            token: s("tok-4"),
            url: s("https://x.example"),
            otpauth_or_secret: s("JBSWY3DPEHPK3PXP"),
            entry_id: None,
            pairing: None,
        },
    ];
    let responses = vec![
        Response::Paired { secret: s("abc") },
        Response::Address {
            address: Box::new(address()),
        },
        Response::Person {
            person: Box::new(person()),
            address: Some(Box::new(address())),
        },
        Response::CredentialList {
            credentials: vec![summary(vec![s("+1555")])],
        },
        Response::Credential {
            username: s("alice-user"),
            password: s("hunter2"),
            totp: so("112233"),
            urls: vec![],
        },
        Response::PasswordHistory {
            entries: vec![ExtensionPasswordHistoryItem {
                id: s("h"),
                password: s("hunter3"),
                url: s("https://x.example"),
                created_at: 1.0,
            }],
        },
    ];
    let mut text = String::new();
    for r in &requests {
        text.push_str(&format!("{r:?} "));
    }
    for r in &responses {
        text.push_str(&format!("{r:?} "));
    }
    text.push_str(&format!("{:?}", address()));
    text.push_str(&format!("{:?}", person()));
    assert!(text.contains("Request::FillCredential { .. }"));
    assert!(text.contains("Response::Paired { .. }"));
    for leak in [
        "tok-",
        "hunter",
        "abc",
        "alice",
        "987654",
        "JBSWY",
        "112233",
        "Lovelace",
        "King",
        "1815",
        "a@example.com",
        "1 Main",
        "+44",
        "+1555",
        "entry-id-1",
        "x.example",
        "Example",
    ] {
        assert!(!text.contains(leak), "{leak} leaked: {text}");
    }
}

#[test]
fn propose_save_login_username_is_email_round_trips_and_defaults_to_false() {
    let golden = r#"{"type":"proposeSaveLogin","token":"t","url":"https://x.example","username":"ann@example.test","password":"p","usernameIsEmail":true}"#;
    let request = Request::ProposeSaveLogin {
        token: s("t"),
        url: s("https://x.example"),
        username: s("ann@example.test"),
        password: s("p"),
        extra_fields: Vec::new(),
        username_is_email: true,
        pairing: None,
    };
    assert_eq!(serde_json::from_str::<Request>(golden).unwrap(), request);
    let golden_value: Value = serde_json::from_str(golden).unwrap();
    assert!(json_eq(
        &serde_json::to_value(&request).unwrap(),
        &golden_value
    ));
    // Absent means false, and false is not written.
    let plain =
        r#"{"type":"proposeSaveLogin","token":"t","url":"u","username":"u","password":"p"}"#;
    let Request::ProposeSaveLogin {
        username_is_email, ..
    } = serde_json::from_str::<Request>(plain).unwrap()
    else {
        panic!("not a login proposal");
    };
    assert!(!username_is_email);
}
