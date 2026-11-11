//! The MyPass desktop app shell: thin `#[tauri::command]` wrappers over `mypass-session`, typed
//! IPC through `tauri-specta`, and the plugin wiring.
//!
//! Stage 2a scaffold: no commands or events are registered yet; they arrive with the `AppCore`
//! facade. `src/lib/bindings.ts` is generated from [`specta_builder`], never edited by hand.

pub mod i18n;
pub mod shell;

use std::path::Path;

use specta_typescript::Typescript;
use tauri_specta::{collect_commands, collect_events, Builder};

/// The single place where commands and events are registered. Both the running app and the
/// bindings exporter go through it, so the generated TypeScript can never drift from the app.
pub fn specta_builder() -> Builder<tauri::Wry> {
    Builder::<tauri::Wry>::new()
        .commands(collect_commands![])
        .events(collect_events![])
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
    // The single-instance plugin must be the first one registered.
    #[cfg(desktop)]
    let tauri_builder = tauri_builder
        .plugin(tauri_plugin_single_instance::init(|_app, _argv, _cwd| {}))
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init());
    tauri_builder
        .invoke_handler(builder.invoke_handler())
        .setup(move |app| {
            builder.mount_events(app);
            Ok(())
        })
        .run(tauri::generate_context!())
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
        let windows = conf["app"]["windows"].as_array().unwrap();
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0]["label"], "main");
    }
}
