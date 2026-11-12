//! The MyPass desktop app shell: thin `#[tauri::command]` wrappers over `mypass-session`, typed
//! IPC through `tauri-specta`, and the plugin wiring.
//!
//! `src/lib/bindings.ts` is generated from [`specta_builder`], never edited by hand; the command
//! list is pinned by `tests/commands.snap`.

pub mod commands;
pub mod events;
pub mod files;
pub mod hotkeys;
pub mod i18n;
pub mod lifecycle;
pub mod shell;
pub mod shell_runtime;
pub mod state;
#[cfg(test)]
mod testing;
pub mod tray;
pub mod wiring;

use std::path::Path;

use specta_typescript::Typescript;
use tauri_specta::{collect_commands, collect_events, Builder, Commands};

/// Every command, in one place (the tests read their names from here).
pub fn commands() -> Commands<tauri::Wry> {
    use commands::{app, attachment, backup, draft, entry, generator, group, import};
    use commands::{platform, settings, sidebar, vault};
    collect_commands![
        app::app_info,
        app::app_open_entry,
        app::app_open_settings,
        app::app_record_activity,
        app::app_reveal_backups,
        attachment::attachment_export,
        attachment::attachment_pick,
        backup::backup_list,
        backup::backup_restore,
        draft::draft_clear,
        draft::draft_save,
        draft::draft_versions,
        entry::entry_copy,
        entry::entry_delete,
        entry::entry_edit_model,
        entry::entry_empty_bin,
        entry::entry_get,
        entry::entry_history,
        entry::entry_history_restore,
        entry::entry_list,
        entry::entry_move,
        entry::entry_new_model,
        entry::entry_purge,
        entry::entry_recent,
        entry::entry_restore,
        entry::entry_reveal,
        entry::entry_save,
        entry::entry_set_favorite,
        entry::entry_totp,
        entry::entry_touch,
        generator::generator_copy,
        generator::generator_generate,
        generator::generator_history_copy,
        generator::generator_history_delete,
        generator::generator_history_list,
        generator::generator_history_reveal,
        generator::generator_rules_summary,
        group::group_create,
        group::group_delete,
        group::group_move,
        group::group_update,
        import::import_csv_commit,
        import::import_csv_preview,
        platform::platform_keyboard_layout,
        settings::settings_get,
        settings::settings_login_item_status,
        settings::settings_update,
        settings::shortcut_conflicts_take,
        sidebar::sidebar_get,
        vault::vault_create,
        vault::vault_find_files,
        vault::vault_forget,
        vault::vault_lock,
        vault::vault_open,
        vault::vault_open_warnings,
        vault::vault_recent,
        vault::vault_status,
        vault::vault_unlock,
    ]
}

/// The single place where commands and events are registered. Both the running app and the
/// bindings exporter go through it, so the generated TypeScript can never drift from the app.
pub fn specta_builder() -> Builder<tauri::Wry> {
    use events::{
        ClipboardClearedEvent, OpenEntryEvent, SaveStateChanged, SessionChanged, SettingsChanged,
        ShortcutConflictEvent, VaultChanged,
    };
    Builder::<tauri::Wry>::new()
        .commands(commands())
        .events(collect_events![
            ClipboardClearedEvent,
            OpenEntryEvent,
            SaveStateChanged,
            SessionChanged,
            SettingsChanged,
            ShortcutConflictEvent,
            VaultChanged,
        ])
}

/// Write the TypeScript bindings to `path`.
pub fn export_bindings(path: &Path) -> Result<(), String> {
    specta_builder()
        .export(Typescript::default(), path)
        .map_err(|error| error.to_string())
}

/// Build and run the application.
pub fn run() -> tauri::Result<()> {
    let builder = specta_builder();
    let tauri_builder = tauri::Builder::default();
    // The single-instance plugin must be the first one registered; there is no autostart plugin
    // (launch at login goes through the platform seam).
    #[cfg(desktop)]
    let tauri_builder = tauri_builder
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            use tauri::Manager;
            // The state may not exist yet when a second launch arrives very early.
            if let Some(shell) = app.try_state::<state::ShellState>() {
                lifecycle::handle_second_instance(shell.windows.as_ref(), &argv);
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build());
    let app = tauri_builder
        .invoke_handler(builder.invoke_handler())
        .on_window_event(shell_runtime::on_window_event)
        .setup(move |app| {
            builder.mount_events(app);
            wiring::install(app)?;
            shell_runtime::start(app);
            Ok(())
        })
        .build(tauri::generate_context!())?;
    app.run(|app, event| shell_runtime::on_run_event(app, &event));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Test-only: where the committed bindings live, independent of the working directory.
    fn committed_bindings() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../src/lib/bindings.ts")
    }

    fn normalise(text: &str) -> String {
        text.replace("\r\n", "\n")
    }

    #[test]
    fn bindings_are_up_to_date() {
        let dir = tempfile::tempdir().unwrap();
        let generated_path = dir.path().join("bindings.ts");
        export_bindings(&generated_path).unwrap();
        let generated = normalise(&fs::read_to_string(&generated_path).unwrap());
        let committed = normalise(&fs::read_to_string(committed_bindings()).unwrap());
        assert_eq!(
            generated, committed,
            "src/lib/bindings.ts is stale: run `cargo test -p mypass-app export_bindings_regenerate -- --ignored`"
        );
    }

    /// Not a check: rewrites the committed bindings. Run it explicitly after a command or type
    /// change (`-- --ignored`).
    #[test]
    #[ignore = "regenerates src/lib/bindings.ts"]
    fn export_bindings_regenerate() {
        export_bindings(&committed_bindings()).unwrap();
    }

    fn tauri_conf() -> serde_json::Value {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tauri.conf.json");
        serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
    }

    #[test]
    fn tauri_conf_csp_has_no_remote_origins() {
        let conf = tauri_conf();
        let csp = conf["app"]["security"]["csp"].as_str().unwrap();
        // The only URL allowed in the CSP is the Windows IPC origin.
        let without_ipc = csp.replace("http://ipc.localhost", "");
        assert!(
            !without_ipc.contains("http://") && !without_ipc.contains("https://"),
            "CSP names a remote origin: {csp}"
        );
        // `devCsp` is the production policy plus exactly the Vite HMR websocket (the dev server
        // origin itself is `'self'`); it never ships, but it must not drift into a wider policy.
        let dev_csp = conf["app"]["security"]["devCsp"].as_str().unwrap();
        assert_eq!(
            dev_csp,
            csp.replace(
                "connect-src ipc: http://ipc.localhost",
                "connect-src ipc: http://ipc.localhost ws://localhost:1420"
            ),
            "devCsp must be csp plus the Vite websocket only"
        );
        let windows = conf["app"]["windows"].as_array().unwrap();
        let labels: Vec<_> = windows
            .iter()
            .map(|w| w["label"].as_str().unwrap())
            .collect();
        // `settings` is created lazily by `app_open_settings`, so it is not declared here.
        assert_eq!(labels, ["main", "tray-panel"]);
    }

    #[test]
    fn tray_panel_window_is_a_hidden_frameless_always_on_top_panel() {
        let conf = tauri_conf();
        let panel = &conf["app"]["windows"][1];
        assert_eq!(panel["label"], "tray-panel");
        assert_eq!(panel["decorations"], false);
        assert_eq!(panel["visible"], false);
        assert_eq!(panel["alwaysOnTop"], true);
        assert_eq!(panel["skipTaskbar"], true);
        assert_eq!(panel["width"], 320);
        assert_eq!(panel["height"], 440);
    }

    fn command_names() -> Vec<String> {
        let mut types = specta::Types::default();
        let collected = (commands().1)(&mut types);
        let mut names: Vec<String> = collected
            .functions
            .iter()
            .map(|f| f.name().to_string())
            .collect();
        names.sort();
        names
    }

    fn committed_snapshot() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/commands.snap")
    }

    #[test]
    fn command_names_follow_domain_verb_convention() {
        const DOMAINS: [&str; 13] = [
            "vault",
            "entry",
            "group",
            "attachment",
            "draft",
            "generator",
            "settings",
            "backup",
            "import",
            "sidebar",
            "app",
            "platform",
            "shortcut",
        ];
        let names = command_names();
        assert!(!names.is_empty());
        for name in &names {
            let (domain, rest) = name.split_once('_').unwrap();
            assert!(DOMAINS.contains(&domain), "{name}: unknown domain");
            assert!(
                !rest.is_empty()
                    && rest.split('_').all(|part| {
                        !part.is_empty() && part.chars().all(|c| c.is_ascii_lowercase())
                    }),
                "{name}: not <domain>_<verb>[_<noun>] in snake_case"
            );
        }
        let mut unique = names.clone();
        unique.dedup();
        assert_eq!(unique, names, "a command is registered twice");
    }

    #[test]
    fn command_snapshot_matches() {
        let committed = normalise(&fs::read_to_string(committed_snapshot()).unwrap());
        let actual = command_names().join("\n") + "\n";
        assert_eq!(
            actual, committed,
            "src-tauri/tests/commands.snap is stale: run `cargo test -p mypass-app command_snapshot_regenerate -- --ignored`"
        );
    }

    /// Not a check: rewrites the committed command list. Run it explicitly in the same commit as
    /// any command change (`-- --ignored`).
    #[test]
    #[ignore = "regenerates src-tauri/tests/commands.snap"]
    fn command_snapshot_regenerate() {
        fs::write(committed_snapshot(), command_names().join("\n") + "\n").unwrap();
    }

    #[test]
    fn ipc_contract_commands_consumed_by_2b_are_registered() {
        let names = command_names();
        let snapshot = fs::read_to_string(committed_snapshot()).unwrap();
        for required in [
            "platform_keyboard_layout",
            "app_open_entry",
            "app_reveal_backups",
            "settings_login_item_status",
            "vault_open_warnings",
            "app_open_settings",
            "attachment_pick",
            "attachment_export",
            "import_csv_preview",
            "import_csv_commit",
            "entry_save",
            "vault_unlock",
        ] {
            assert!(names.iter().any(|n| n == required), "{required} missing");
            assert!(
                snapshot.lines().any(|l| l == required),
                "{required} not in snap"
            );
        }
        use tauri_specta::Event;
        assert_eq!(events::OpenEntryEvent::NAME, "open-entry-event");
    }

    #[test]
    fn events_have_stable_names() {
        use tauri_specta::Event;
        assert_eq!(events::SessionChanged::NAME, "session-changed");
        assert_eq!(events::VaultChanged::NAME, "vault-changed");
        assert_eq!(events::SaveStateChanged::NAME, "save-state-changed");
        assert_eq!(events::SettingsChanged::NAME, "settings-changed");
        assert_eq!(
            events::ClipboardClearedEvent::NAME,
            "clipboard-cleared-event"
        );
        assert_eq!(
            events::ShortcutConflictEvent::NAME,
            "shortcut-conflict-event"
        );
        assert_eq!(events::OpenEntryEvent::NAME, "open-entry-event");
    }

    /// Every permission identifier of every capability file, with the file's windows.
    fn capabilities() -> Vec<(String, Vec<String>, Vec<String>)> {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("capabilities");
        let mut out = Vec::new();
        for item in fs::read_dir(dir).unwrap() {
            let path = item.unwrap().path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let json: serde_json::Value =
                serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
            let strings = |v: &serde_json::Value| -> Vec<String> {
                v.as_array()
                    .unwrap()
                    .iter()
                    .map(|p| match p {
                        serde_json::Value::String(s) => s.clone(),
                        other => other["identifier"].as_str().unwrap().to_owned(),
                    })
                    .collect()
            };
            out.push((
                json["identifier"].as_str().unwrap().to_owned(),
                strings(&json["windows"]),
                strings(&json["permissions"]),
            ));
        }
        out
    }

    #[test]
    fn capabilities_grant_no_fs_shell_or_clipboard_read() {
        let all = capabilities();
        assert_eq!(all.len(), 3);
        for (id, windows, permissions) in &all {
            assert_eq!(
                windows.as_slice(),
                std::slice::from_ref(id),
                "a capability is scoped to its own window"
            );
            for permission in permissions {
                for forbidden in [
                    "fs:",
                    "shell:",
                    "http:",
                    "clipboard-manager:",
                    "global-shortcut:",
                ] {
                    assert!(
                        !permission.starts_with(forbidden),
                        "{id} grants {permission}"
                    );
                }
                assert!(!permission.starts_with("core:tray:"), "{id}: {permission}");
                assert!(!permission.starts_with("core:menu:"), "{id}: {permission}");
                assert_ne!(
                    permission, "core:default",
                    "{id} grants the blanket core set"
                );
                if id == "tray-panel" {
                    assert!(!permission.starts_with("core:window:allow-set-"));
                }
                if permission == "opener:allow-open-url" {
                    assert_eq!(id, "main", "only main may open URLs");
                }
            }
        }
    }

    /// The exact permission set of every window: widening one (an `opener:default`, an
    /// `opener:allow-open-path`, any fs or shell permission) fails here and needs a review.
    #[test]
    fn capability_permission_sets_are_exact() {
        let all = capabilities();
        let sorted = |id: &str| {
            let mut set = all.iter().find(|c| c.0 == id).unwrap().2.clone();
            set.sort();
            set
        };
        assert_eq!(
            sorted("main"),
            [
                "core:event:default",
                "core:window:allow-close",
                "core:window:allow-is-maximized",
                "core:window:allow-is-minimized",
                "core:window:allow-maximize",
                "core:window:allow-minimize",
                "core:window:allow-start-dragging",
                "core:window:allow-unmaximize",
                "dialog:allow-ask",
                "dialog:allow-open",
                "dialog:allow-save",
                "opener:allow-open-url",
            ]
        );
        assert_eq!(sorted("tray-panel"), ["core:event:default"]);
        assert_eq!(
            sorted("settings"),
            [
                "core:event:default",
                "dialog:allow-open",
                "dialog:allow-save"
            ]
        );
    }

    #[test]
    fn opener_scope_is_http_and_https_only() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("capabilities/main.json");
        let json: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
        let opener = json["permissions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["identifier"] == "opener:allow-open-url")
            .unwrap();
        let urls: Vec<&str> = opener["allow"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a["url"].as_str().unwrap())
            .collect();
        assert_eq!(urls, ["https://*", "http://*"]);
    }

    #[test]
    fn command_wrappers_do_not_log_arguments() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files = vec![
            dir.join("wiring.rs"),
            dir.join("state.rs"),
            dir.join("files.rs"),
        ];
        for item in fs::read_dir(dir.join("commands")).unwrap() {
            files.push(item.unwrap().path());
        }
        for file in files {
            let text = fs::read_to_string(&file).unwrap();
            // Only code before the test module is the shipped code.
            let code = text.split("#[cfg(test)]").next().unwrap();
            for needle in [
                "println!",
                "eprintln!",
                "print!",
                "dbg!",
                "log::",
                "tracing::",
                "info!(",
                "warn!(",
                "error!(",
                "debug!(",
                "trace!(",
            ] {
                // A needle counts only at the start of a path or macro name (`dialog::` is not
                // `log::`).
                let found = code.match_indices(needle).any(|(at, _)| {
                    !code[..at]
                        .chars()
                        .next_back()
                        .is_some_and(|c| c.is_alphanumeric() || c == '_')
                });
                assert!(!found, "{} uses {needle}", file.display());
            }
        }
    }

    /// Proves: the secret holds the value, the source `String` is emptied (zeroize clears it in
    /// place), and the wrapper's `Debug` redacts. Cannot prove, without `unsafe`: that the freed
    /// capacity bytes are zero, that no other copy exists (the IPC deserialiser holds its own
    /// before the command runs), or that each command calls `into_secret` on its first line.
    #[test]
    fn password_argument_is_zeroized_after_wrapping() {
        use secrecy::ExposeSecret;
        let mut password = String::from("hunter2-correct-horse");
        let secret = commands::into_secret(&mut password);
        assert_eq!(secret.expose_secret(), "hunter2-correct-horse");
        // The source `String` was zeroized in place (and so is empty).
        assert!(password.is_empty());
        // The wrapper redacts itself.
        let shown = format!("{secret:?}");
        assert!(!shown.contains("hunter2"), "{shown}");
    }

    #[test]
    fn error_serialisation_shape() {
        use mypass_session::{ApiError, ErrorCode};
        let json = |code| {
            serde_json::to_value(ApiError {
                code,
                message: "x".to_owned(),
            })
            .unwrap()
        };
        assert_eq!(
            json(ErrorCode::Locked),
            serde_json::json!({"code": "locked", "message": "x"})
        );
        for (code, wire) in [
            (ErrorCode::Quitting, "quitting"),
            (ErrorCode::GeneratorNoClass, "generatorNoClass"),
            (ErrorCode::NotEnrolledOnDevice, "notEnrolledOnDevice"),
            (
                ErrorCode::DeviceUnlockUnavailable,
                "deviceUnlockUnavailable",
            ),
            (ErrorCode::Cancelled, "cancelled"),
        ] {
            assert_eq!(json(code)["code"], wire);
        }
    }
}
