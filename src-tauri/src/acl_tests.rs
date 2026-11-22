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
    "browser_pairing_status",
    "browser_proposal_take",
    "device_unlock_consume_extension_request",
    "device_unlock_offer",
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
    "settings_get_public",
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
    "app_show_main",
    "backup_list",
    "backup_restore",
    "browser_reinstall_host",
    "browser_reset_pairings",
    "browser_revoke",
    "browser_show_extension_folder",
    "browser_status",
    "browser_unregister_host",
    "device_unlock_configure",
    "device_unlock_offer",
    "device_unlock_status",
    "settings_get",
    "settings_get_public",
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
    "app_open_settings",
    "app_quit",
    "app_record_activity",
    "app_show_main",
    "device_unlock_offer",
    "entry_copy",
    "entry_recent",
    "entry_touch",
    "platform_keyboard_layout",
    "settings_get",
    "settings_get_public",
    "tray_panel_fit",
    "tray_panel_hide",
    "update_open_release",
    "update_status",
    "vault_lock",
    "vault_status",
    "vault_unlock",
    "vault_unlock_device",
];

/// Settings-window commands that refuse unless the vault is unlocked: the first statement of each
/// is `commands::ensure_unlocked` (after `into_secret` when a password arrives). They read
/// security-relevant settings or state, change settings or run a settings-pane action.
const SETTINGS_GATED: &[&str] = &[
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
    "vault_location_change",
    "vault_location_pick",
    "vault_location_probe",
];

/// Settings-window commands that stay open while the vault is not unlocked, each with the reason.
/// None of them returns or changes a setting that matters to security.
const SETTINGS_UNGATED: &[(&str, &str)] = &[
    (
        "app_info",
        "version and platform information, including the home directory used in locked vault path labels",
    ),
    ("app_record_activity", "a no-op while locked"),
    (
        "app_show_main",
        "the Unlock button of the locked placeholder",
    ),
    (
        "device_unlock_offer",
        "read-only unlock screen availability, label and prompt outcomes",
    ),
    (
        "settings_get_public",
        "theme, language, shortcut hints and the update flag, which locked windows render",
    ),
    (
        "update_open_release",
        "the update banner of a locked main window or tray panel",
    ),
    (
        "update_status",
        "the update banner of a locked main window or tray panel",
    ),
    (
        "vault_status",
        "the session state itself, which the placeholder follows",
    ),
    (
        "vault_storage_status",
        "the storage banner of a locked main window",
    ),
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
            // The tray panel only queries recents.
            "allow-app-new-entry",
            "allow-app-open-entry",
            "allow-entry-get",
            "allow-entry-reveal",
            "allow-entry-totp",
            "allow-entry-list",
            "allow-tray-search",
            "core:window:allow-hide",
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

#[test]
fn every_settings_window_command_is_gated_or_has_a_reason() {
    let gated: BTreeSet<_> = SETTINGS_GATED.iter().copied().collect();
    let open: BTreeSet<_> = SETTINGS_UNGATED.iter().map(|(name, _)| *name).collect();
    assert!(gated.is_disjoint(&open), "a command is both gated and open");
    let classified: BTreeSet<_> = gated.union(&open).copied().collect();
    let granted: BTreeSet<_> = SETTINGS.iter().copied().collect();
    assert_eq!(
        classified, granted,
        "classify every settings-window command as gated or open (with a reason)"
    );
    assert!(SETTINGS_UNGATED.iter().all(|(_, why)| !why.is_empty()));
}

/// The body of `fn name(` in the shipped part of the command modules.
fn command_body(name: &str) -> String {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/commands");
    let needle = format!("fn {name}(");
    let mut found = Vec::new();
    for item in std::fs::read_dir(dir).unwrap() {
        let text = std::fs::read_to_string(item.unwrap().path()).unwrap();
        let code = text.split("#[cfg(test)]").next().unwrap().to_owned();
        if let Some(at) = code.find(&needle) {
            let rest = &code[at..];
            let open = rest.find('{').unwrap();
            found.push(rest[open + 1..].to_owned());
        }
    }
    assert_eq!(found.len(), 1, "{name}: exactly one definition");
    found.pop().unwrap()
}

#[test]
fn gated_settings_commands_check_the_session_first() {
    for name in SETTINGS_GATED {
        let body = command_body(name);
        let mut statements = body.split(';').map(str::trim);
        let first = statements.next().unwrap();
        // A password is wrapped (and its `String` zeroized) before anything can return.
        let gate = if first.contains("into_secret") {
            statements.next().unwrap()
        } else {
            first
        };
        // A command that writes later keeps the incarnation for its commit point.
        let gate = gate.strip_prefix("let incarnation = ").unwrap_or(gate);
        assert!(
            gate.starts_with("super::ensure_unlocked(&") || gate.starts_with("ensure_unlocked(&"),
            "{name} must call ensure_unlocked first, found `{gate}`"
        );
        assert!(
            gate.ends_with(")?"),
            "{name}: the refusal must be returned, `{gate}`"
        );
    }
}

#[test]
fn ungated_settings_commands_do_not_return_the_full_settings() {
    for (name, _) in SETTINGS_UNGATED {
        let body = command_body(name);
        let signature_and_body = body.split("\n}\n").next().unwrap();
        assert!(
            !signature_and_body.contains("settings_get()"),
            "{name} reads the full settings without the gate"
        );
    }
}

/// Main-window commands the unlock screen uses in any session state that touch settings: listing
/// the recent vaults and removing one. Removal while not unlocked changes only that list (never
/// the startup vault; see `AppCore::vault_forget`).
const UNLOCK_SCREEN_SETTINGS_EXCEPTIONS: &[(&str, &str)] = &[
    (
        "vault_recent",
        "the unlock and open screens list the recent vaults before any vault is unlocked",
    ),
    (
        "vault_forget",
        "the open screen removes a recent vault; while not unlocked only the list changes",
    ),
];

#[test]
fn the_unlock_screen_settings_exceptions_are_main_window_only_and_open() {
    for (name, why) in UNLOCK_SCREEN_SETTINGS_EXCEPTIONS {
        assert!(!why.is_empty());
        for (label, commands) in window_commands() {
            assert_eq!(commands.contains(name), label == "main", "{label}: {name}");
        }
        let body = command_body(name);
        let body = body.split("\n}\n").next().unwrap();
        assert!(
            !body.contains("ensure_unlocked"),
            "{name} is documented as open on the unlock screen"
        );
    }
}
