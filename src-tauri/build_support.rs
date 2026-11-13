//! Build-script helpers that are also compiled into tests (`#[path]`), so the rules are checked
//! without running a real build.

use std::io;
use std::path::Path;

/// Tauri's build script fails when a file named in `bundle.externalBin` is missing, which would
/// break `cargo test` on a clean checkout (the real host binary is built by a separate script).
/// Creates `<binaries_dir>/mypass-host-<target_triple>[.exe]` as an EMPTY executable file, only
/// when it is absent; an existing file is never touched. Returns `true` when it was created.
///
/// The placeholder is never installed: the registrar refuses an empty or non-executable host.
pub fn ensure_sidecar_placeholder(binaries_dir: &Path, target_triple: &str) -> io::Result<bool> {
    let suffix = if target_triple.contains("windows") {
        ".exe"
    } else {
        ""
    };
    let file = binaries_dir.join(format!("mypass-host-{target_triple}{suffix}"));
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

/// `tauri-build` copies the sidecar to `<profile dir>/mypass-host`, which is ALSO where cargo puts
/// the `mypass-host` binary of the `mypass-host` package. With the EMPTY placeholder that copy
/// would replace a real, already built host and break its tests (`CARGO_BIN_EXE_mypass-host`).
/// Reads the real binary before the copy ...
pub fn real_host_before_copy(dest: &Path) -> Option<Vec<u8>> {
    std::fs::read(dest).ok().filter(|bytes| !bytes.is_empty())
}

/// ... and puts it back when the copy left an empty file in its place. When the host is not built
/// yet nothing is saved and cargo's own link step writes the real one later. A sidecar that is
/// itself a real binary (the release script's) is left alone.
pub fn restore_real_host(dest: &Path, saved: Option<&[u8]>) -> io::Result<bool> {
    let Some(saved) = saved else {
        return Ok(false);
    };
    let replaced_by_placeholder = std::fs::metadata(dest).is_ok_and(|m| m.len() == 0);
    if !replaced_by_placeholder {
        return Ok(false);
    }
    std::fs::write(dest, saved)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dest, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(true)
}
