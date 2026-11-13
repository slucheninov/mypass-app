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
