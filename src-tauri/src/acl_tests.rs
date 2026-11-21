//! Test-only: pin command grants independently of generated permissions and resolve the baked ACL.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use std::collections::BTreeSet;
use std::path::Path;

const MAIN: &[&str] = &[
    "app_info",
    "app_open_settings",
    "app_record_activity",
    "app_set_editor_dirty",
    "app_take_new_entry",
    "attachment_export",
    "attachment_pick",
    "browser_pairing_cancel",
    "browser_proposal_take",
    "browser_status",
    "device_unlock_consume_extension_request",
    "device_unlock_status",
    "draft_clear",
    "draft_save",
    "draft_versions",
    "entry_copy",
    "entry_delete",
    "entry_edit_model",
    "entry_empty_bin",
    "entry_get",
    "entry_history",
    "entry_history_restore",
    "entry_history_reveal",
    "entry_list",
    "entry_move",
    "entry_new_model",
    "entry_passkey_details",
    "entry_purge",
    "entry_restore",
    "entry_reveal",
    "entry_save",
    "entry_set_favorite",
    "entry_totp",
    "generator_copy",
    "generator_generate",
    "generator_history_copy",
    "generator_history_delete",
    "generator_history_list",
    "generator_history_reveal",
    "generator_rules_summary",
    "generator_use",
    "group_create",
    "group_delete",
    "group_move",
    "group_update",
    "import_csv_commit",
    "import_csv_preview",
    "platform_keyboard_layout",
    "settings_get",
    "settings_update",
    "shortcut_conflicts_take",
    "sidebar_get",
    "sync_status",
    "update_dismiss",
    "update_open_release",
    "update_status",
    "vault_create",
    "vault_find_files",
    "vault_forget",
    "vault_lock",
    "vault_open",
    "vault_open_warnings",
    "vault_recent",
    "vault_status",
    "vault_storage_status",
    "vault_unlock",
    "vault_unlock_device",
];

const SETTINGS: &[&str] = &[
    "app_info",
    "app_record_activity",
    "app_reveal_backups",
    "backup_list",
    "backup_restore",
    "browser_reinstall_host",
    "browser_reset_pairings",
    "browser_revoke",
    "browser_show_extension_folder",
    "browser_status",
    "browser_unregister_host",
    "device_unlock_configure",
    "device_unlock_status",
    "settings_get",
    "settings_login_item_status",
    "settings_update",
    "sync_connect_secondary",
    "sync_disconnect_secondary",
    "sync_displaced_delete",
    "sync_displaced_status",
    "sync_now",
    "sync_status",
    "update_check_now",
    "update_open_release",
    "update_status",
    "vault_location_change",
    "vault_location_pick",
    "vault_location_probe",
    "vault_status",
    "vault_storage_status",
];

const TRAY_PANEL: &[&str] = &[
    "app_info",
    "app_new_entry",
    "app_open_entry",
    "app_open_settings",
    "app_quit",
    "app_record_activity",
    "app_show_main",
    "device_unlock_status",
    "entry_copy",
    "entry_get",
    "entry_recent",
    "entry_reveal",
    "entry_totp",
    "entry_touch",
    "platform_keyboard_layout",
    "settings_get",
    "tray_search",
    "update_open_release",
    "update_status",
    "vault_lock",
    "vault_status",
    "vault_unlock",
    "vault_unlock_device",
];

fn window_commands() -> [(&'static str, &'static [&'static str]); 3] {
    [
        ("main", MAIN),
        ("settings", SETTINGS),
        ("tray-panel", TRAY_PANEL),
    ]
}

fn capability(label: &str) -> serde_json::Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("capabilities")
        .join(format!("{label}.json"));
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn app_manifest_covers_every_registered_command() {
    let build = include_str!("../build.rs");
    let commands = build
        .split(".app_manifest(")
        .nth(1)
        .unwrap()
        .split(".commands(&[")
        .nth(1)
        .unwrap()
        .split("])")
        .next()
        .unwrap();
    let declared: BTreeSet<_> = commands.split('"').skip(1).step_by(2).collect();
    let registered: BTreeSet<_> = include_str!("../tests/commands.snap").lines().collect();
    assert_eq!(
        declared, registered,
        "every handler must opt into the app ACL"
    );
    let granted: BTreeSet<_> = window_commands()
        .into_iter()
        .flat_map(|(_, commands)| commands.iter().copied())
        .collect();
    assert_eq!(
        granted, registered,
        "no command may be unreachable or unknown"
    );
}

#[test]
fn per_window_app_command_grants_are_exact() {
    for (label, expected) in window_commands() {
        let json = capability(label);
        assert_eq!(json["windows"], serde_json::json!([label]));
        assert!(
            json.get("webviews").is_none(),
            "no alternate webview grants"
        );
        assert!(json.get("remote").is_none(), "local app content only");
        assert_ne!(json.get("local"), Some(&serde_json::json!(false)));
        let grants: Vec<_> = json["permissions"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|p| p.as_str().and_then(|s| s.strip_prefix("allow-")))
            .map(|s| s.replace('-', "_"))
            .collect();
        assert_eq!(grants, expected, "{label} command grants changed");
    }
}

#[test]
fn tray_panel_cannot_configure_device_move_vault_read_passkeys_or_take_proposals() {
    let json = capability("tray-panel");
    for permission in json["permissions"].as_array().unwrap() {
        let id = permission
            .as_str()
            .or_else(|| permission["identifier"].as_str())
            .unwrap();
        assert!(!id.starts_with("allow-sync-"));
        for forbidden in [
            "allow-device-unlock-configure",
            "allow-vault-location-change",
            "allow-entry-passkey-details",
            "allow-browser-proposal-take",
            // The general list takes any query, the Deleted category included; the tray panel
            // searches through `tray_search` only.
            "allow-entry-list",
        ] {
            assert_ne!(id, forbidden);
        }
    }
}

#[test]
fn compiled_acl_resolves_specta_commands_only_for_their_window() {
    // Generate the production context, without constructing a runtime or opening a window.
    // Tauri checks this authority before dispatching to tauri-specta's invoke handler.
    let mut context: tauri::Context<tauri::Wry> = super::app_context();
    let authority = context.runtime_authority_mut();
    for command in super::tests::command_names() {
        for (label, expected) in window_commands() {
            let allowed = authority
                .resolve_access(&command, label, label, &tauri::ipc::Origin::Local)
                .is_some();
            assert_eq!(
                allowed,
                expected.contains(&command.as_str()),
                "{label}: {command}"
            );
        }
        assert!(authority
            .resolve_access(&command, "unknown", "unknown", &tauri::ipc::Origin::Local)
            .is_none());
    }
}

#[test]
fn browser_unregister_host_is_settings_only() {
    for label in ["main", "tray-panel", "settings"] {
        let json = capability(label);
        let granted = json["permissions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p.as_str() == Some("allow-browser-unregister-host"));
        assert_eq!(granted, label == "settings");
    }
}
