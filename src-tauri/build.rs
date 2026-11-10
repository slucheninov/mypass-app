use std::path::Path;

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
    tauri_build::build();
}
