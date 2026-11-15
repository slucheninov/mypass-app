//! Debug-only isolation for Windows subprocess fixtures: MYPASS_WINDOWS_DATA_ROOT is a final
//! data/config root, with no extra MyPass component. Release builds never read the variable.
//! MYPASS_SECRET_STORE / plaintext dev-secret-store overrides are deliberately unsupported.
use crate::{PlatformError, PlatformResult};
use std::{ffi::OsStr, path::PathBuf};

/// Validate an injected environment value without reading the process environment.
pub fn windows_data_root(
    debug_build: bool,
    value: Option<&OsStr>,
) -> PlatformResult<Option<PathBuf>> {
    if !debug_build {
        return Ok(None);
    }
    let Some(value) = value else { return Ok(None) };
    let path = PathBuf::from(value);
    #[cfg(windows)]
    let has_nul = {
        use std::os::windows::ffi::OsStrExt;
        value.encode_wide().any(|unit| unit == 0)
    };
    #[cfg(not(windows))]
    let has_nul = value.as_encoded_bytes().contains(&0);
    if !path.is_absolute() || has_nul {
        return Err(PlatformError::Failed(
            "invalid Windows debug data root".into(),
        ));
    }
    Ok(Some(path))
}

#[cfg(windows)]
pub(crate) fn read_windows_data_root() -> PlatformResult<Option<PathBuf>> {
    if cfg!(debug_assertions) {
        windows_data_root(
            true,
            std::env::var_os("MYPASS_WINDOWS_DATA_ROOT").as_deref(),
        )
    } else {
        Ok(None)
    }
}
