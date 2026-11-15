//! The sole Win32 unsafe boundary. Portable policies live in safe windows_* modules.
#![allow(unsafe_code)]

use crate::{AppPaths, PlatformError, PlatformResult};
use std::{
    ffi::{OsStr, OsString},
    os::windows::ffi::{OsStrExt, OsStringExt},
    path::PathBuf,
};
use windows::{
    core::{GUID, PWSTR},
    Win32::{
        System::Com::CoTaskMemFree,
        UI::Shell::{
            FOLDERID_LocalAppData, FOLDERID_Profile, FOLDERID_RoamingAppData, SHGetKnownFolderPath,
            KNOWN_FOLDER_FLAG,
        },
    },
};

pub fn to_wide(value: &OsStr) -> PlatformResult<Vec<u16>> {
    let mut units: Vec<_> = value.encode_wide().collect();
    if units.contains(&0) {
        return Err(PlatformError::Failed(
            "interior NUL in Windows string".into(),
        ));
    }
    units.push(0);
    Ok(units)
}

/// Decode an explicitly sized WTF-16 string (no implicit terminator stripping).
pub fn from_wide(units: &[u16]) -> OsString {
    OsString::from_wide(units)
}

struct KnownFolder(PWSTR);
impl Drop for KnownFolder {
    fn drop(&mut self) {
        // SAFETY: SHGetKnownFolderPath allocated this pointer with the COM task allocator;
        // this owner is unique and frees it once, after the copied path no longer borrows it.
        unsafe { CoTaskMemFree(Some(self.0.as_ptr().cast())) };
    }
}

fn known_folder(id: &GUID) -> PlatformResult<PathBuf> {
    // SAFETY: id references a static known-folder GUID; flags=0 keeps verification enabled,
    // and no token means the current user. The successful allocation is immediately owned.
    let allocation = KnownFolder(
        unsafe { SHGetKnownFolderPath(id, KNOWN_FOLDER_FLAG(0), None) }
            .map_err(|e| PlatformError::Failed(e.to_string()))?,
    );
    if allocation.0.is_null() {
        return Err(PlatformError::Failed("null known-folder path".into()));
    }
    // SAFETY: the successful shell API returns a NUL-terminated, valid allocation that remains
    // owned by allocation for the entire borrow. Copy WTF-16 before freeing it.
    let path = PathBuf::from(from_wide(unsafe { allocation.0.as_wide() }));
    if !path.is_absolute() {
        return Err(PlatformError::Failed(
            "known-folder path is not absolute".into(),
        ));
    }
    Ok(path)
}

pub fn known_folders() -> PlatformResult<(PathBuf, PathBuf, PathBuf)> {
    Ok((
        known_folder(&FOLDERID_LocalAppData)?,
        known_folder(&FOLDERID_RoamingAppData)?,
        known_folder(&FOLDERID_Profile)?,
    ))
}

pub fn resolve_windows() -> PlatformResult<AppPaths> {
    if let Some(root) = crate::dev_overrides::read_windows_data_root()? {
        return Ok(AppPaths::windows_data_root(&root));
    }
    let (local, roaming, profile) = known_folders()?;
    Ok(AppPaths::windows(&roaming, &local, &profile))
}

/// Native services are replaced batch by batch; never construct unimplemented native bodies.
pub(crate) fn platform(paths: &AppPaths) -> crate::Platform {
    crate::fallback::platform(paths)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn to_wide_is_nul_terminated_and_rejects_interior_nul() {
        assert_eq!(to_wide(OsStr::new("abc")).unwrap(), vec![97, 98, 99, 0]);
        assert!(to_wide(OsStr::new("a\0b")).is_err());
    }
    #[test]
    fn from_wide_round_trips_unpaired_surrogate_on_windows() {
        let value = from_wide(&[0xd800, 65]);
        assert_eq!(to_wide(&value).unwrap(), vec![0xd800, 65, 0]);
    }
}
