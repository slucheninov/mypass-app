use std::path::Path;

#[path = "build_support.rs"]
mod build_support;

fn main() {
    // `tauri::generate_context!` embeds `frontendDist` (`../dist`) and fails when it is missing.
    // `dist/` is git-ignored and built by the UI toolchain (stage 2b), so a Rust-only checkout
    // (`cargo test --workspace`, no Node) gets a placeholder page instead of a build error.
    // A real frontend build overwrites it.
    let dist = Path::new("..").join("dist");
    // Keyed on the page, not the directory: an empty `dist/` would fail the same way.
    if !dist.join("index.html").exists() {
        let written = std::fs::create_dir_all(&dist).and_then(|()| {
            std::fs::write(
                dist.join("index.html"),
                "<!doctype html><title>MyPass</title>",
            )
        });
        if let Err(error) = written {
            // `tauri_build::build()` below then fails with its own message about `frontendDist`.
            println!("cargo:warning=cannot create the placeholder ../dist/index.html: {error}");
        }
    }
    println!("cargo:rerun-if-changed=../dist");

    // The same pattern for the `externalBin` host: `tauri-build` fails when it is missing.
    let triple = std::env::var("TARGET").unwrap_or_default();
    if let Err(error) = build_support::ensure_sidecar_placeholder(Path::new("binaries"), &triple) {
        println!("cargo:warning=cannot create the placeholder sidecar in binaries/: {error}");
    }

    let attributes = if targets_windows_msvc() {
        embed_windows_manifest();
        tauri_build::Attributes::new()
            .windows_attributes(tauri_build::WindowsAttributes::new_without_app_manifest())
    } else {
        tauri_build::Attributes::new()
    };
    // Listing every command opts the app into Tauri's IPC ACL, including tauri-specta handlers.
    // Keep this list explicit: the guards compare it to the registered command snapshot.
    let attributes = attributes.app_manifest(tauri_build::AppManifest::new().commands(&[
        "app_info",
        "app_open_entry",
        "app_open_settings",
        "app_quit",
        "app_record_activity",
        "app_reveal_backups",
        "app_set_editor_dirty",
        "app_show_main",
        "attachment_export",
        "attachment_pick",
        "backup_list",
        "backup_restore",
        "browser_pairing_cancel",
        "browser_proposal_take",
        "browser_reinstall_host",
        "browser_unregister_host",
        "browser_reset_pairings",
        "browser_revoke",
        "browser_show_extension_folder",
        "browser_status",
        "device_unlock_configure",
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
        "entry_recent",
        "entry_restore",
        "entry_reveal",
        "entry_save",
        "entry_set_favorite",
        "entry_totp",
        "entry_touch",
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
        "settings_login_item_status",
        "settings_update",
        "shortcut_conflicts_take",
        "sidebar_get",
        "sync_connect_secondary",
        "sync_disconnect_secondary",
        "sync_displaced_delete",
        "sync_displaced_status",
        "sync_now",
        "sync_status",
        "update_check_now",
        "update_dismiss",
        "update_open_release",
        "update_status",
        "vault_create",
        "vault_find_files",
        "vault_forget",
        "vault_location_change",
        "vault_location_pick",
        "vault_location_probe",
        "vault_lock",
        "vault_open",
        "vault_open_warnings",
        "vault_recent",
        "vault_status",
        "vault_storage_status",
        "vault_unlock",
        "vault_unlock_device",
    ]));
    if let Err(error) = tauri_build::try_build(attributes) {
        println!("cargo:warning=tauri-build failed: {error:#}");
        std::process::exit(1);
    }
}

/// Build scripts run on the host, so the target is read from the `CARGO_CFG_*` variables.
fn targets_windows_msvc() -> bool {
    std::env::var("CARGO_CFG_TARGET_OS").is_ok_and(|os| os == "windows")
        && std::env::var("CARGO_CFG_TARGET_ENV").is_ok_and(|env| env == "msvc")
}

/// Embeds the Common-Controls v6 application manifest into EVERY target of this package.
///
/// By default `tauri-build` embeds its manifest only into the package's bin target. The lib unit
/// test executable (and any integration test) links `tauri` and so imports comctl32 entry points
/// that exist only in Common-Controls v6; without the manifest Windows loads v5 and the process
/// dies at startup with `0xc0000139` (`STATUS_ENTRYPOINT_NOT_FOUND`) before a single test runs.
/// `cargo:rustc-link-arg` applies to all targets of the package, so the bin, the lib tests and
/// future integration tests all carry the manifest. `/WX` turns a manifest problem into an error.
/// The file mirrors the default manifest of `tauri-build`; do not remove this without a
/// replacement, the Windows CI test job fails without it.
fn embed_windows_manifest() {
    let manifest = std::env::var("CARGO_MANIFEST_DIR")
        .map(|dir| Path::new(&dir).join("windows-app-manifest.xml"));
    let manifest = match manifest {
        Ok(path) => path,
        Err(error) => {
            println!("cargo:warning=CARGO_MANIFEST_DIR is not readable: {error}");
            std::process::exit(1);
        }
    };
    println!("cargo:rerun-if-changed={}", manifest.display());
    println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
    println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
    println!("cargo:rustc-link-arg=/WX");
}
