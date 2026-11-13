//! Build-script helpers that are also compiled into tests (`#[path]`), so the rules are checked
//! without running a real build.

use std::io;
use std::path::Path;

/// Tauri's build script fails when a file named in `bundle.externalBin` is missing, which would
/// break `cargo test` on a clean checkout (the real host binary is built by a separate script).
/// The sidecar is NOT called `mypass-host`: tauri-build copies it to `<profile dir>/<name>`, which
/// for that name is cargo's own output of the `mypass-host` package.
/// Creates `<binaries_dir>/mypass-native-host-<target_triple>[.exe]` as an EMPTY executable file, only
/// when it is absent; an existing file is never touched. Returns `true` when it was created.
///
/// The placeholder is never installed: the registrar refuses an empty or non-executable host.
pub fn ensure_sidecar_placeholder(binaries_dir: &Path, target_triple: &str) -> io::Result<bool> {
    let suffix = if target_triple.contains("windows") {
        ".exe"
    } else {
        ""
    };
    let file = binaries_dir.join(format!("mypass-native-host-{target_triple}{suffix}"));
    if file.exists() {
        return Ok(false);
    }
    std::fs::create_dir_all(binaries_dir)?;
    // `create_new` so a file that appears between the check and here is never truncated.
    let created = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&file);
    match created {
        Ok(_) => {}
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => return Ok(false),
        Err(e) => return Err(e),
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(true)
}
