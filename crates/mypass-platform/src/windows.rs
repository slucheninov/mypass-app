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

/// Built by the shell setup on the main UI thread, before commands can query layouts.
pub(crate) fn platform(paths: &AppPaths) -> crate::Platform {
    crate::Platform {
        events: std::sync::Arc::new(WinSystemEvents),
        clipboard: std::sync::Arc::new(WinClipboard::default()),
        secrets: std::sync::Arc::new(crate::DpapiSecretStore::app(paths)),
        shared_secrets: crate::app_shared_store(paths, None).1,
        device_unlock: std::sync::Arc::new(crate::fallback::UnsupportedDeviceUnlock),
        host_registrar: std::sync::Arc::new(
            crate::windows_host_install::WindowsHostRegistrar::new(
                paths.clone(),
                std::sync::Arc::new(WinRegistry::new()),
            ),
        ),
        launch_at_login: std::sync::Arc::new(
            crate::windows_launch_at_login::WinLaunchAtLogin::new(
                std::sync::Arc::new(WinRegistry::new()),
                std::env::current_exe().unwrap_or_default(),
            ),
        ),
        keyboard_layout: std::sync::Arc::new(WinKeyboardLayout::capture()),
        // The window icons follow the theme through the app shell (Tauri); no Dock to set.
        app_icon: std::sync::Arc::new(crate::fallback::UnsupportedAppIcon),
    }
}

// DPAPI, protected file I/O and unsigned Windows listener ownership. All pointer operations
// stay in this file; portable policy and the fake protector contain no native calls.
use crate::dpapi::{invalid, ProtectedFiles, Protector, MAX_ENVELOPE, MAX_PLAINTEXT};
use std::{
    fs::File,
    io::{Read, Write},
    mem::{offset_of, size_of},
    path::{Component, Path},
};
use windows::{
    core::{w, PCWSTR},
    Win32::{
        Foundation::{
            CloseHandle, LocalFree, ERROR_ALREADY_EXISTS, ERROR_FILE_NOT_FOUND,
            ERROR_INSUFFICIENT_BUFFER, ERROR_INVALID_FUNCTION, ERROR_INVALID_PARAMETER,
            ERROR_NOT_SUPPORTED, ERROR_PATH_NOT_FOUND, HANDLE, HLOCAL, WAIT_TIMEOUT,
        },
        NetworkManagement::IpHelper::*,
        Networking::WinSock::AF_INET,
        Security::{Authorization::*, Cryptography::*, *},
        Storage::FileSystem::*,
        System::Threading::*,
    },
};
use zeroize::{Zeroize, Zeroizing};

/// Atomically move a file on the same volume, failing if the destination exists.
pub fn rename_noreplace(from: &Path, to: &Path) -> std::io::Result<()> {
    let wide_path = |path: &Path| {
        to_wide(path.as_os_str())
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))
    };
    let from = wide_path(from)?;
    let to = wide_path(to)?;
    // SAFETY: both buffers are NUL-terminated and live for the call. With no flags the
    // move cannot replace a destination or copy across volumes.
    unsafe {
        MoveFileExW(
            PCWSTR(from.as_ptr()),
            PCWSTR(to.as_ptr()),
            MOVE_FILE_FLAGS(0),
        )
    }
    .map_err(|e| std::io::Error::from_raw_os_error(e.code().0 & 0xffff))
}

/// Query by owned handle, following junctions/symlinks; canonical strings cannot identify
/// hard links or correctly model case-sensitive Windows directories.
pub(crate) fn file_identity(path: &Path) -> std::io::Result<crate::FileIdentity> {
    use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
    let file = std::fs::OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES.0)
        .share_mode(FILE_SHARE_READ.0 | FILE_SHARE_WRITE.0 | FILE_SHARE_DELETE.0)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS.0)
        .open(path)?;
    let mut info = FILE_ID_INFO::default();
    // SAFETY: file owns a live handle throughout the call, and info is a correctly sized
    // writable output. File closes the handle on every return path. No raw handle escapes.
    unsafe {
        GetFileInformationByHandleEx(
            HANDLE(file.as_raw_handle()),
            FileIdInfo,
            (&mut info as *mut FILE_ID_INFO).cast(),
            size_of::<FILE_ID_INFO>() as u32,
        )
    }
    .map_err(|e| {
        // The handle and buffer/class are fixed and valid. These errors mean the volume or
        // redirector cannot supply FileIdInfo; do not guess identity from path spelling.
        if [
            ERROR_INVALID_FUNCTION,
            ERROR_NOT_SUPPORTED,
            ERROR_INVALID_PARAMETER,
        ]
        .iter()
        .any(|code| e.code() == code.to_hresult())
        {
            std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "volume file identity unavailable",
            )
        } else {
            std::io::Error::last_os_error()
        }
    })?;
    Ok(crate::FileIdentity {
        volume: info.VolumeSerialNumber,
        file: info.FileId.Identifier,
    })
}

fn win_error(e: windows::core::Error) -> PlatformError {
    PlatformError::Failed(format!("Windows error 0x{:08x}", e.code().0))
}
struct LocalMemory(*mut std::ffi::c_void);
impl Drop for LocalMemory {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: every instance owns one LocalAlloc-family output, uniquely, until drop.
            unsafe {
                let _ = LocalFree(Some(HLOCAL(self.0)));
            }
        }
    }
}
struct WinHandle(HANDLE);
impl Drop for WinHandle {
    fn drop(&mut self) {
        // SAFETY: each wrapper owns one successful non-pseudo native handle, closed exactly once.
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}
struct DpapiOutput(CRYPT_INTEGER_BLOB);
impl Drop for DpapiOutput {
    fn drop(&mut self) {
        if !self.0.pbData.is_null() {
            // SAFETY: the DPAPI allocation has cbData initialized bytes and remains owned here.
            // Wipe decrypted OS output BEFORE returning the allocation to LocalFree.
            unsafe {
                std::slice::from_raw_parts_mut(self.0.pbData, self.0.cbData as usize).zeroize();
                let _ = LocalFree(Some(HLOCAL(self.0.pbData.cast())));
            }
        }
    }
}
pub(crate) struct DpapiProtector;
fn dpapi_transform(
    input: &[u8],
    entropy: &[u8],
    decrypt: bool,
) -> PlatformResult<Zeroizing<Vec<u8>>> {
    if input.len() > MAX_ENVELOPE {
        return Err(invalid());
    }
    let mut input = Zeroizing::new(input.to_vec());
    let mut entropy = entropy.to_vec();
    let data = CRYPT_INTEGER_BLOB {
        cbData: input.len() as u32,
        pbData: input.as_mut_ptr(),
    };
    let entropy = CRYPT_INTEGER_BLOB {
        cbData: entropy.len() as u32,
        pbData: entropy.as_mut_ptr(),
    };
    let mut out = DpapiOutput(CRYPT_INTEGER_BLOB::default());
    // SAFETY: both input blobs borrow live buffers for this synchronous call. out is zeroed and
    // immediately owns any returned allocation even on error. No prompt, machine scope or output
    // description is requested; CryptUnprotectData therefore allocates no description to free.
    unsafe {
        if decrypt {
            CryptUnprotectData(
                &data,
                None,
                Some(&entropy),
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out.0,
            )
        } else {
            CryptProtectData(
                &data,
                w!("MyPass"),
                Some(&entropy),
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out.0,
            )
        }
    }
    .map_err(win_error)?;
    let limit = if decrypt { MAX_PLAINTEXT } else { MAX_ENVELOPE };
    if out.0.cbData as usize > limit {
        return Err(invalid());
    }
    if out.0.cbData == 0 {
        return Ok(Zeroizing::new(Vec::new()));
    }
    if out.0.pbData.is_null() {
        return Err(invalid());
    }
    // SAFETY: successful DPAPI returned cbData initialized bytes; copy before the RAII wipe/free.
    Ok(Zeroizing::new(
        unsafe { std::slice::from_raw_parts(out.0.pbData, out.0.cbData as usize) }.to_vec(),
    ))
}
impl Protector for DpapiProtector {
    fn protect(&self, plain: &[u8], entropy: &[u8]) -> PlatformResult<Vec<u8>> {
        Ok(dpapi_transform(plain, entropy, false)?.to_vec())
    }
    fn unprotect(&self, cipher: &[u8], entropy: &[u8]) -> PlatformResult<Zeroizing<Vec<u8>>> {
        dpapi_transform(cipher, entropy, true)
    }
}

// Aligned token buffer, with its SID pointer borrowed only while this heap allocation lives.
struct TokenUserBuffer(Vec<usize>);
impl TokenUserBuffer {
    fn for_process(process: HANDLE) -> PlatformResult<Self> {
        let mut token = HANDLE::default();
        // SAFETY: process is either a live owned handle or GetCurrentProcess's pseudo handle;
        // token receives a distinct TOKEN_QUERY handle, immediately wrapped for cleanup.
        unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) }.map_err(win_error)?;
        let token = WinHandle(token);
        let mut len = 0;
        // SAFETY: this sizing call writes only len, with no supplied data buffer.
        let result = unsafe { GetTokenInformation(token.0, TokenUser, None, 0, &mut len) };
        if result.is_ok() || len < size_of::<TOKEN_USER>() as u32 || len > 64 * 1024 {
            return Err(invalid());
        }
        let mut buf = vec![0usize; (len as usize).div_ceil(size_of::<usize>())];
        // SAFETY: buf is aligned for TOKEN_USER and has at least len writable bytes.
        unsafe {
            GetTokenInformation(
                token.0,
                TokenUser,
                Some(buf.as_mut_ptr().cast()),
                len,
                &mut len,
            )
        }
        .map_err(win_error)?;
        Ok(Self(buf))
    }
    fn current() -> PlatformResult<Self> {
        // SAFETY: GetCurrentProcess returns a borrowed pseudo handle, never closed by this code.
        Self::for_process(unsafe { GetCurrentProcess() })
    }
    fn sid(&self) -> PSID {
        // SAFETY: the successful TokenUser query initialized TOKEN_USER in the aligned allocation,
        // including its SID pointer; the allocation remains alive while this SID is borrowed.
        unsafe { (*self.0.as_ptr().cast::<TOKEN_USER>()).User.Sid }
    }
    fn string(&self) -> PlatformResult<String> {
        let mut text = PWSTR::null();
        // SAFETY: sid points into the live token buffer; text receives a LocalFree allocation.
        unsafe { ConvertSidToStringSidW(self.sid(), &mut text) }.map_err(win_error)?;
        let allocation = LocalMemory(text.0.cast());
        // SAFETY: successful conversion returns a terminated UTF-16 SID string in allocation.
        let value = unsafe { text.to_string() }.map_err(|_| invalid())?;
        drop(allocation);
        Ok(value)
    }
}
fn protected_descriptor(user: &TokenUserBuffer) -> PlatformResult<LocalMemory> {
    let sddl = format!(
        "O:{}D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;{})",
        user.string()?,
        user.string()?
    );
    let wide = to_wide(OsStr::new(&sddl))?;
    let mut sd = PSECURITY_DESCRIPTOR::default();
    // SAFETY: wide is terminated for the duration of the call; sd receives a LocalFree allocation.
    unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            PCWSTR(wide.as_ptr()),
            SDDL_REVISION_1,
            &mut sd,
            None,
        )
    }
    .map_err(win_error)?;
    Ok(LocalMemory(sd.0))
}
fn validate_security(handle: HANDLE, user: &TokenUserBuffer) -> PlatformResult<()> {
    let mut owner = PSID::default();
    let mut dacl = std::ptr::null_mut();
    let mut sd = PSECURITY_DESCRIPTOR::default();
    // SAFETY: handle names an opened object. All returned interior pointers borrow the returned
    // security descriptor allocation, retained until every SID/ACL inspection is finished.
    let status = unsafe {
        GetSecurityInfo(
            handle,
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            Some(&mut owner),
            None,
            Some(&mut dacl),
            None,
            Some(&mut sd),
        )
    };
    let _allocation = LocalMemory(sd.0);
    status.ok().map_err(win_error)?;
    let mut control = 0;
    let mut revision = 0;
    // SAFETY: sd is the successful security query's live allocation, owner/dacl point into it.
    unsafe {
        GetSecurityDescriptorControl(sd, &mut control, &mut revision).map_err(win_error)?;
        if owner.is_invalid()
            || EqualSid(owner, user.sid()).is_err()
            || dacl.is_null()
            || !IsValidAcl(dacl).as_bool()
            || control & SE_DACL_PROTECTED.0 == 0
        {
            return Err(invalid());
        }
    }
    let mut system = PSID::default();
    // SAFETY: constant SID string is terminated and system receives a LocalFree allocation.
    unsafe { ConvertStringSidToSidW(w!("S-1-5-18"), &mut system) }.map_err(win_error)?;
    let _system = LocalMemory(system.0);
    let mut user_grant = false;
    let mut system_grant = false;
    // SAFETY: IsValidAcl succeeded and the descriptor allocation remains alive.
    let count = unsafe { (*dacl).AceCount };
    for index in 0..u32::from(count) {
        let mut ace = std::ptr::null_mut();
        // SAFETY: GetAce bounds-checks index in the valid ACL and returns an interior ACE pointer.
        unsafe { GetAce(dacl, index, &mut ace) }.map_err(win_error)?;
        // SAFETY: a validated ACL contains a complete ACE_HEADER for every returned ACE.
        let header = unsafe { &*ace.cast::<ACE_HEADER>() };
        // Only conventional, effective allow ACEs for these two SIDs are accepted. Refuse
        // unfamiliar/object/callback ACEs rather than trying to approximate effective access.
        if header.AceType != 0
            || header.AceFlags & INHERIT_ONLY_ACE.0 as u8 != 0
            || usize::from(header.AceSize) < offset_of!(ACCESS_ALLOWED_ACE, SidStart) + 8
        {
            return Err(invalid());
        }
        // SAFETY: the header/type/size checks establish a conventional ACCESS_ALLOWED_ACE;
        // its variable SID is inside the valid ACL. Windows validates both SIDs in EqualSid.
        let allowed = unsafe { &*ace.cast::<ACCESS_ALLOWED_ACE>() };
        let sid = PSID(std::ptr::addr_of!(allowed.SidStart).cast_mut().cast());
        // SAFETY: the checked ACE includes the SID's eight-byte header. Bound its variable
        // subauthority array within this ACE before asking Windows to inspect/compare the SID.
        let subauthorities = unsafe { *sid.0.cast::<u8>().add(1) } as usize;
        if 8 + subauthorities * 4
            > usize::from(header.AceSize) - offset_of!(ACCESS_ALLOWED_ACE, SidStart)
        {
            return Err(invalid());
        }
        // SAFETY: the complete SID is now bounded by its live ACE allocation.
        if !unsafe { IsValidSid(sid) }.as_bool() {
            return Err(invalid());
        }

        // SAFETY: these SIDs are part of live validated native allocations.
        let (is_user, is_system) = unsafe {
            (
                EqualSid(sid, user.sid()).is_ok(),
                EqualSid(sid, system).is_ok(),
            )
        };
        if !is_user && !is_system {
            return Err(invalid());
        }
        if allowed.Mask & FILE_ALL_ACCESS.0 == FILE_ALL_ACCESS.0 {
            user_grant |= is_user;
            system_grant |= is_system;
        }
    }
    if !user_grant || !system_grant {
        return Err(invalid());
    }
    Ok(())
}
fn open_checked(
    path: &Path,
    directory: bool,
    sensitive: bool,
    user: &TokenUserBuffer,
) -> PlatformResult<Option<File>> {
    use std::os::windows::io::{AsRawHandle, FromRawHandle};
    let wide = to_wide(path.as_os_str())?;
    let access = if directory {
        FILE_READ_ATTRIBUTES.0 | READ_CONTROL.0
    } else {
        FILE_GENERIC_READ.0
    };
    // Ancestors cannot be renamed while an operation is in progress. File handles can share
    // deletion: a concurrent encrypted atomic replacement does not change the validated object
    // this handle reads, and the protected parent stays pinned.
    let share = if directory {
        FILE_SHARE_READ | FILE_SHARE_WRITE
    } else {
        FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE
    };
    // SAFETY: terminated path, no borrowed security attributes, returned handle immediately owned.
    let handle = match unsafe {
        CreateFileW(
            PCWSTR(wide.as_ptr()),
            access,
            share,
            None,
            OPEN_EXISTING,
            FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS,
            None,
        )
    } {
        Ok(h) => h,
        Err(e)
            if e.code() == ERROR_FILE_NOT_FOUND.to_hresult()
                || e.code() == ERROR_PATH_NOT_FOUND.to_hresult() =>
        {
            return Ok(None)
        }
        Err(e) => return Err(win_error(e)),
    };
    // SAFETY: successful CreateFileW yielded one owned OS file handle, now owned by File.
    let file = unsafe { File::from_raw_handle(handle.0) };
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: file owns a live handle and info is writable for the call.
    unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut info) }
        .map_err(win_error)?;
    if info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0
        || (info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY.0 != 0) != directory
        || (!directory && info.nNumberOfLinks != 1)
    {
        return Err(invalid());
    }
    if sensitive {
        validate_security(handle, user)?;
    }
    Ok(Some(file))
}
fn validate_secret_path(dir: &Path) -> PlatformResult<()> {
    use std::path::Prefix;
    let valid_name = |name: &OsStr| {
        let units: Vec<_> = name.encode_wide().collect();
        !units.is_empty()
            && !units.iter().any(|c| matches!(*c, 0 | 58))
            && !matches!(units.last(), Some(32 | 46))
    };
    let mut parts = dir.components();
    let valid_prefix = match parts.next() {
        Some(Component::Prefix(p)) => match p.kind() {
            Prefix::Disk(_) | Prefix::VerbatimDisk(_) => true,
            Prefix::UNC(server, share) | Prefix::VerbatimUNC(server, share) => {
                valid_name(server) && valid_name(share)
            }
            _ => false,
        },
        _ => false,
    };
    if !valid_prefix || !matches!(parts.next(), Some(Component::RootDir)) {
        return Err(invalid());
    }
    for part in parts {
        let Component::Normal(name) = part else {
            return Err(invalid());
        };
        if !valid_name(name) {
            return Err(invalid());
        }
    }
    Ok(())
}

/// Pins every ancestor with non-delete-shared handles; never follows reparse points. Existing
/// secret directory/file ownership and effective ACLs are checked, never repaired or loosened.
fn protected_directory(
    dir: &Path,
    create: bool,
    user: &TokenUserBuffer,
) -> PlatformResult<Option<Vec<File>>> {
    validate_secret_path(dir)?;
    let descriptor = protected_descriptor(user)?;
    let attrs = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor.0,
        bInheritHandle: false.into(),
    };
    let mut pinned = Vec::new();
    let mut path = PathBuf::new();
    for part in dir.components() {
        path.push(part.as_os_str());
        if matches!(part, Component::Prefix(_)) {
            continue;
        }
        let mut opened = open_checked(&path, true, path == dir, user)?;
        if opened.is_none() && create {
            let wide = to_wide(path.as_os_str())?;
            // SAFETY: the parent is pinned, wide and attrs/descriptor live for the synchronous
            // creation. A racing creator is never trusted: its object is opened/validated next.
            match unsafe { CreateDirectoryW(PCWSTR(wide.as_ptr()), Some(&attrs)) } {
                Ok(()) => {}
                Err(e) if e.code() == ERROR_ALREADY_EXISTS.to_hresult() => {}
                Err(e) => return Err(win_error(e)),
            }
            opened = open_checked(&path, true, true, user)?;
        }
        let Some(file) = opened else {
            return Ok(None);
        };
        pinned.push(file);
    }
    Ok(Some(pinned))
}

pub(crate) struct DpapiFiles {
    dir: PathBuf,
}
impl DpapiFiles {
    pub(crate) fn new(dir: PathBuf) -> Self {
        Self { dir }
    }
}
struct TempFile(Option<PathBuf>);
impl Drop for TempFile {
    fn drop(&mut self) {
        if let Some(path) = self.0.take() {
            let _ = std::fs::remove_file(path);
        }
    }
}
impl ProtectedFiles for DpapiFiles {
    fn read(&self, name: &str) -> PlatformResult<Option<Vec<u8>>> {
        let user = TokenUserBuffer::current()?;
        let Some(_pins) = protected_directory(&self.dir, false, &user)? else {
            return Ok(None);
        };
        let Some(file) = open_checked(&self.dir.join(name), false, true, &user)? else {
            return Ok(None);
        };
        let mut bytes = Zeroizing::new(Vec::new());
        file.take(MAX_ENVELOPE as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| invalid())?;
        if bytes.len() > MAX_ENVELOPE {
            return Err(invalid());
        }
        Ok(Some(std::mem::take(&mut *bytes)))
    }
    fn publish(&self, name: &str, bytes: &[u8]) -> PlatformResult<()> {
        use std::os::windows::io::{AsRawHandle, FromRawHandle};
        if bytes.len() > MAX_ENVELOPE {
            return Err(invalid());
        }
        let user = TokenUserBuffer::current()?;
        let _pins = protected_directory(&self.dir, true, &user)?.ok_or_else(invalid)?;
        // Refuse hostile existing files. Dropping this handle before replacement allows concurrent
        // encrypted writers; directory handles continue to prevent ancestor swaps.
        drop(open_checked(&self.dir.join(name), false, true, &user)?);
        let sd = protected_descriptor(&user)?;
        let attrs = SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: sd.0,
            bInheritHandle: false.into(),
        };
        let temp_path = self
            .dir
            .join(format!(".dpapi-{}.tmp", uuid::Uuid::new_v4()));
        let wide = to_wide(temp_path.as_os_str())?;
        // SAFETY: create-new cannot overwrite another file. The explicit protected descriptor
        // grants only current token user + SYSTEM. The handle is immediately transferred to File.
        let handle = unsafe {
            CreateFileW(
                PCWSTR(wide.as_ptr()),
                FILE_GENERIC_WRITE.0 | READ_CONTROL.0,
                FILE_SHARE_READ,
                Some(&attrs),
                CREATE_NEW,
                FILE_ATTRIBUTE_NORMAL | FILE_FLAG_OPEN_REPARSE_POINT,
                None,
            )
        }
        .map_err(win_error)?;
        // Arm cleanup only after create-new succeeds; a collision must not delete existing data.
        // Declare it before File so error unwinding closes the handle before removing its path.
        let mut temp = TempFile(Some(temp_path));
        // SAFETY: unique ownership of the successful CreateFileW handle transfers to File.
        let mut file = unsafe { File::from_raw_handle(handle.0) };
        validate_security(HANDLE(file.as_raw_handle()), &user)?;
        file.write_all(bytes)
            .and_then(|()| file.sync_all())
            .map_err(|_| invalid())?;
        drop(file);
        let target = to_wide(self.dir.join(name).as_os_str())?;
        // SAFETY: both terminated paths name children of the pinned protected directory. Only
        // encrypted bytes were written. Same-volume replacement publishes one complete envelope.
        unsafe {
            MoveFileExW(
                PCWSTR(wide.as_ptr()),
                PCWSTR(target.as_ptr()),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        }
        .map_err(win_error)?;
        // Disarm immediately after publication; never delete a reused former temp filename.
        temp.0 = None;
        Ok(())
    }
    fn delete(&self, name: &str) -> PlatformResult<()> {
        let user = TokenUserBuffer::current()?;
        let Some(_pins) = protected_directory(&self.dir, false, &user)? else {
            return Ok(());
        };
        let path = self.dir.join(name);
        let Some(_file) = open_checked(&path, false, true, &user)? else {
            return Ok(());
        };
        match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err(invalid()),
        }
    }
}

pub(crate) struct WindowsListenerProbe;
impl crate::peer_process::WindowsListenerProbe for WindowsListenerProbe {
    fn listener_pid(&self, port: u16) -> Option<u32> {
        tcp_listener_pid(port).ok().flatten()
    }
    fn current_user_owns_live_process(&self, pid: u32) -> bool {
        let check = || -> PlatformResult<bool> {
            let current = TokenUserBuffer::current()?;
            // SAFETY: pid is a positive endpoint PID; request query-only access, no inheritance.
            let process = WinHandle(
                unsafe {
                    OpenProcess(
                        PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
                        false,
                        pid,
                    )
                }
                .map_err(win_error)?,
            );
            let owner = TokenUserBuffer::for_process(process.0)?;
            // SAFETY: owned process has SYNCHRONIZE access and both SID allocations are live.
            // A zero-time wait distinguishes process exit even if its exit code is STILL_ACTIVE.
            Ok(unsafe { WaitForSingleObject(process.0, 0) } == WAIT_TIMEOUT
                && unsafe { EqualSid(current.sid(), owner.sid()) }.is_ok())
        };
        check().unwrap_or(false)
    }
}
fn tcp_listener_pid(port: u16) -> PlatformResult<Option<u32>> {
    let mut bytes = 0;
    // SAFETY: sizing call writes bytes only; IPv4 owner-PID listeners are explicitly requested.
    let status = unsafe {
        GetExtendedTcpTable(
            None,
            &mut bytes,
            false,
            AF_INET.0 as u32,
            TCP_TABLE_OWNER_PID_LISTENER,
            0,
        )
    };
    if status != ERROR_INSUFFICIENT_BUFFER.0 {
        return Err(invalid());
    }
    // Retry boundedly if another listener appears between size query and retrieval.
    for _ in 0..3 {
        if bytes as usize > 16 * 1024 * 1024
            || (bytes as usize) < offset_of!(MIB_TCPTABLE_OWNER_PID, table)
        {
            return Err(invalid());
        }
        let mut data = vec![0usize; (bytes as usize).div_ceil(size_of::<usize>())];
        let capacity = data.len() * size_of::<usize>();
        // SAFETY: aligned allocation holds at least bytes writable bytes; all parsing below is
        // bounded by both the reported initialized byte length and the allocated capacity.
        let status = unsafe {
            GetExtendedTcpTable(
                Some(data.as_mut_ptr().cast()),
                &mut bytes,
                false,
                AF_INET.0 as u32,
                TCP_TABLE_OWNER_PID_LISTENER,
                0,
            )
        };
        if status == ERROR_INSUFFICIENT_BUFFER.0 {
            continue;
        }
        if status != 0 || bytes as usize > capacity {
            return Err(invalid());
        }
        let table = data.as_ptr().cast::<MIB_TCPTABLE_OWNER_PID>();
        // SAFETY: the successful result initialized the table header in the aligned allocation.
        let count = unsafe { (*table).dwNumEntries } as usize;
        let offset = offset_of!(MIB_TCPTABLE_OWNER_PID, table);
        if count > (bytes as usize).saturating_sub(offset) / size_of::<MIB_TCPROW_OWNER_PID>() {
            return Err(invalid());
        }
        // SAFETY: header count and allocation bounds were checked; rows are initialized/aligned.
        let rows = unsafe {
            std::slice::from_raw_parts(
                data.as_ptr()
                    .cast::<u8>()
                    .add(offset)
                    .cast::<MIB_TCPROW_OWNER_PID>(),
                count,
            )
        };
        let mut pids = rows
            .iter()
            .filter(|row| {
                row.dwLocalAddr.to_ne_bytes() == [127, 0, 0, 1]
                    && row.dwState == MIB_TCP_STATE_LISTEN.0 as u32
                    && u32::from(u16::from_be(port)) == row.dwLocalPort
            })
            .map(|row| row.dwOwningPid);
        let first = pids.next();
        return if pids.next().is_none() {
            Ok(first)
        } else {
            Err(invalid())
        };
    }
    Err(invalid())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn secret_paths_allow_redirected_roots_and_reject_streams_and_devices() {
        for path in [
            r"C:\Users\user\MyPass",
            r"\\server\share\MyPass",
            r"\\?\C:\Users\user\MyPass",
        ] {
            assert!(validate_secret_path(Path::new(path)).is_ok(), "{path}");
        }
        for path in [
            r"C:\Users\user\bad:stream",
            r"relative\MyPass",
            r"\\.\pipe\name",
            r"C:\trailing.",
            r"C:\a\..\b",
        ] {
            assert!(validate_secret_path(Path::new(path)).is_err(), "{path}");
        }
    }
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

// Typed registry adapter. Only HKCU writes are permitted; HKLM is read-only.
use crate::windows_registry::{Hive, Key, Read as RegistryRead, Registry, Value, ValueType, View};
use windows::Win32::Foundation::ERROR_MORE_DATA;
use windows::Win32::System::Registry::*;

struct RegistryHandle(HKEY);
impl Drop for RegistryHandle {
    fn drop(&mut self) {
        // SAFETY: this is an owned opened/created key, never a predefined hive handle.
        let _ = unsafe { RegCloseKey(self.0) };
    }
}
#[derive(Default)]
pub struct WinRegistry {
    test_root: Option<String>,
}
impl WinRegistry {
    /// Production adapter: paths remain relative to the selected hive.
    pub fn new() -> Self {
        Self::default()
    }
    /// Every operation, including HKLM reads, is confined to a generated HKCU test subtree.
    #[cfg(any(test, feature = "test-util"))]
    pub fn isolated() -> Self {
        Self {
            test_root: Some(format!("Software\\MyPassTests\\{}", uuid::Uuid::new_v4())),
        }
    }
    fn mapped(&self, key: &Key) -> PlatformResult<Key> {
        if let Some(root) = &self.test_root {
            Key::new(
                Hive::CurrentUser,
                key.view,
                &format!(
                    "{root}\\{}",
                    key.subkey()
                        .strip_prefix("software\\")
                        .unwrap_or(key.subkey())
                ),
            )
        } else {
            Ok(key.clone())
        }
    }
    fn open(
        &self,
        key: &Key,
        access: REG_SAM_FLAGS,
        create: bool,
    ) -> PlatformResult<Option<RegistryHandle>> {
        let key = self.mapped(key)?;
        let path = to_wide(OsStr::new(key.subkey()))?;
        let root = match key.hive {
            Hive::CurrentUser => HKEY_CURRENT_USER,
            Hive::LocalMachine => HKEY_LOCAL_MACHINE,
        };
        let view = match key.view {
            View::Registry32 => KEY_WOW64_32KEY,
            View::Registry64 => KEY_WOW64_64KEY,
        };
        let mut result = HKEY::default();
        // SAFETY: validated relative key, terminated path and live output; only returned owned
        // keys enter RegistryHandle. No caller can pass raw handles or override access flags.
        let code = unsafe {
            if create {
                RegCreateKeyExW(
                    root,
                    PCWSTR(path.as_ptr()),
                    None,
                    PCWSTR::null(),
                    REG_OPTION_NON_VOLATILE,
                    access | view,
                    None,
                    &mut result,
                    None,
                )
            } else {
                RegOpenKeyExW(
                    root,
                    PCWSTR(path.as_ptr()),
                    None,
                    access | view,
                    &mut result,
                )
            }
        };
        if code == ERROR_FILE_NOT_FOUND || code == ERROR_PATH_NOT_FOUND {
            return Ok(None);
        }
        code.ok().map_err(win_error)?;
        Ok(Some(RegistryHandle(result)))
    }
    fn write_allowed(&self, key: &Key) -> PlatformResult<()> {
        if self.test_root.is_none() && key.hive != Hive::CurrentUser {
            return Err(PlatformError::Failed("HKLM writes denied".into()));
        }
        Ok(())
    }
}
impl Drop for WinRegistry {
    fn drop(&mut self) {
        if let Some(root) = &self.test_root {
            if let Ok(wide) = to_wide(OsStr::new(root)) {
                // SAFETY: root is generated internally, never caller supplied; recursive cleanup
                // is confined to this adapter's unique test subtree, not any production host key.
                let _ = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, PCWSTR(wide.as_ptr())) };
            }
        }
    }
}
impl Registry for WinRegistry {
    fn key_exists(&self, key: &Key) -> PlatformResult<bool> {
        Ok(self.open(key, KEY_READ, false)?.is_some())
    }
    fn read(&self, key: &Key, name: &str, kind: ValueType) -> PlatformResult<RegistryRead> {
        let wide = to_wide(OsStr::new(name))?;
        let Some(handle) = self.open(key, KEY_QUERY_VALUE, false)? else {
            return Ok(RegistryRead::Missing);
        };
        // Bounded, single-buffer read: concurrent growth yields an error, never unbounded retries.
        let mut bytes = vec![0u8; 65536];
        let mut length = bytes.len() as u32;
        let mut ty = REG_VALUE_TYPE::default();
        // SAFETY: owned key, terminated name, and output pointers into live sized allocations.
        let code = unsafe {
            RegQueryValueExW(
                handle.0,
                PCWSTR(wide.as_ptr()),
                None,
                Some(&mut ty),
                Some(bytes.as_mut_ptr()),
                Some(&mut length),
            )
        };
        if code == ERROR_FILE_NOT_FOUND {
            return Ok(RegistryRead::Missing);
        }
        if code == ERROR_MORE_DATA {
            return Err(PlatformError::Failed("registry value exceeds limit".into()));
        }
        code.ok().map_err(win_error)?;
        bytes.truncate(length as usize);
        match (ty, kind) {
            (REG_BINARY, ValueType::Binary) => Ok(RegistryRead::Value(Value::Binary(bytes))),
            (REG_SZ, ValueType::String) => {
                if !bytes.len().is_multiple_of(2) {
                    return Err(invalid());
                }
                let mut units: Vec<u16> = bytes
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|p| u16::from_le_bytes(*p))
                    .collect();
                if units.last() == Some(&0) {
                    units.pop();
                }
                if units.contains(&0) {
                    return Err(invalid());
                }
                let text = String::from_utf16(&units).map_err(|_| invalid())?;
                Ok(RegistryRead::Value(Value::String(text)))
            }
            _ => Ok(RegistryRead::WrongType),
        }
    }
    fn create(&self, key: &Key) -> PlatformResult<()> {
        self.write_allowed(key)?;
        self.open(key, KEY_SET_VALUE, true)?.ok_or_else(invalid)?;
        Ok(())
    }
    fn set(&self, key: &Key, name: &str, value: Value) -> PlatformResult<()> {
        self.write_allowed(key)?;
        let name = to_wide(OsStr::new(name))?;
        let (kind, bytes) = match value {
            Value::String(s) => (
                REG_SZ,
                to_wide(OsStr::new(&s))?
                    .into_iter()
                    .flat_map(u16::to_le_bytes)
                    .collect::<Vec<_>>(),
            ),
            Value::Binary(b) => (REG_BINARY, b),
        };
        if bytes.len() > 65536 {
            return Err(invalid());
        }
        let handle = self.open(key, KEY_SET_VALUE, false)?.ok_or_else(invalid)?;
        // SAFETY: owned key, terminated name, borrowed byte buffer lives through the call.
        unsafe { RegSetValueExW(handle.0, PCWSTR(name.as_ptr()), None, kind, Some(&bytes)) }
            .ok()
            .map_err(win_error)
    }
    fn delete_value(&self, key: &Key, name: &str) -> PlatformResult<()> {
        self.write_allowed(key)?;
        let name = to_wide(OsStr::new(name))?;
        let Some(handle) = self.open(key, KEY_SET_VALUE, false)? else {
            return Ok(());
        };
        // SAFETY: owned key and terminated value name; no subtree deletion.
        let code = unsafe { RegDeleteValueW(handle.0, PCWSTR(name.as_ptr())) };
        if code == ERROR_FILE_NOT_FOUND {
            return Ok(());
        }
        code.ok().map_err(win_error)
    }
    fn delete_owned_key(&self, key: &Key, name: &str, owner: &Value) -> PlatformResult<bool> {
        self.write_allowed(key)?;
        let kind = match owner {
            Value::String(_) => ValueType::String,
            Value::Binary(_) => ValueType::Binary,
        };
        if self.read(key, name, kind)? != RegistryRead::Value(owner.clone()) {
            return Ok(false);
        }
        self.delete_value(key, name)?;
        let Some(handle) = self.open(key, KEY_READ, false)? else {
            return Ok(true);
        };
        let (mut children, mut values) = (0, 0);
        // SAFETY: owned key; only requested count outputs are non-null and live.
        unsafe {
            RegQueryInfoKeyW(
                handle.0,
                None,
                None,
                None,
                Some(&mut children),
                None,
                None,
                Some(&mut values),
                None,
                None,
                None,
                None,
            )
        }
        .ok()
        .map_err(win_error)?;
        if children == 0 && values == 0 {
            let mapped = self.mapped(key)?;
            let path = to_wide(OsStr::new(mapped.subkey()))?;
            let view = match mapped.view {
                View::Registry32 => KEY_WOW64_32KEY,
                View::Registry64 => KEY_WOW64_64KEY,
            };
            // SAFETY: HKCU-only validated relative key and explicit view. RegDeleteKeyEx is
            // nonrecursive; newly appearing children prevent deletion rather than being removed.
            let code =
                unsafe { RegDeleteKeyExW(HKEY_CURRENT_USER, PCWSTR(path.as_ptr()), view.0, None) };
            if code != ERROR_FILE_NOT_FOUND {
                code.ok().map_err(win_error)?;
            }
        }
        Ok(true)
    }
}

use crate::windows_host_install::{InstallDirectory, InstallFiles, MoveError, MAX_HOST_BYTES};
use windows::Win32::Foundation::{ERROR_ACCESS_DENIED, ERROR_SHARING_VIOLATION};
/// Safe callers receive a session retaining all ancestor handles, never raw Win32 handles.
pub struct HostFiles;
struct HostDirectory {
    path: PathBuf,
    _pins: Vec<File>,
    user: TokenUserBuffer,
}
fn pin_host_ancestors(path: &Path, user: &TokenUserBuffer) -> PlatformResult<Vec<File>> {
    validate_secret_path(path)?;
    let mut pinned = Vec::new();
    let mut current = PathBuf::new();
    for part in path.components() {
        current.push(part.as_os_str());
        if matches!(part, Component::Prefix(_)) {
            continue;
        }
        pinned.push(open_checked(&current, true, false, user)?.ok_or_else(invalid)?);
    }
    Ok(pinned)
}
fn read_host_file(file: File) -> PlatformResult<Vec<u8>> {
    let mut bytes = Vec::new();
    file.take(MAX_HOST_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| invalid())?;
    if bytes.len() > MAX_HOST_BYTES {
        return Err(invalid());
    }
    Ok(bytes)
}
// Browser manifests keep their ordinary <=240-unit path. Internal create-new and backup
// names can be longer, so native file operations use explicit extended paths, without relying
// on the launching browser/test executable carrying a longPathAware process manifest.
fn extended_host_path(path: &Path) -> PlatformResult<PathBuf> {
    use std::path::Prefix;
    validate_secret_path(path)?;
    let Some(Component::Prefix(prefix)) = path.components().next() else {
        return Err(invalid());
    };
    match prefix.kind() {
        Prefix::VerbatimDisk(_) | Prefix::VerbatimUNC(_, _) => Ok(path.into()),
        Prefix::Disk(_) => {
            let mut value = OsString::from(r"\\?\");
            value.push(path);
            Ok(value.into())
        }
        Prefix::UNC(_, _) => {
            let units: Vec<_> = path.as_os_str().encode_wide().skip(2).collect();
            let mut value = OsString::from(r"\\?\UNC\");
            value.push(from_wide(&units));
            Ok(value.into())
        }
        _ => Err(invalid()),
    }
}
impl InstallFiles for HostFiles {
    fn source(&self, path: &Path) -> PlatformResult<Vec<u8>> {
        let user = TokenUserBuffer::current()?;
        let path = extended_host_path(path)?;
        let path = path.as_path();
        validate_secret_path(path)?;
        let _pins = pin_host_ancestors(path.parent().ok_or_else(invalid)?, &user)?;
        read_host_file(open_checked(path, false, false, &user)?.ok_or_else(invalid)?)
    }
    fn directory(
        &self,
        path: &Path,
        create: bool,
    ) -> PlatformResult<Option<Box<dyn InstallDirectory>>> {
        let user = TokenUserBuffer::current()?;
        let path = extended_host_path(path)?;
        let path = path.as_path();
        let Some(pins) = protected_directory(path, create, &user)? else {
            return Ok(None);
        };
        Ok(Some(Box::new(HostDirectory {
            path: path.into(),
            _pins: pins,
            user,
        })))
    }
}
impl HostDirectory {
    fn child(&self, name: &str) -> PlatformResult<PathBuf> {
        if name.is_empty()
            || name.contains(['/', '\\', ':', '\0'])
            || name == "."
            || name == ".."
            || name.ends_with(['.', ' '])
        {
            return Err(invalid());
        }
        Ok(self.path.join(name))
    }
}
impl InstallDirectory for HostDirectory {
    fn read(&self, name: &str) -> PlatformResult<Option<Vec<u8>>> {
        open_checked(&self.child(name)?, false, true, &self.user)?
            .map(read_host_file)
            .transpose()
    }
    fn create_new(&self, name: &str, bytes: &[u8]) -> PlatformResult<()> {
        use std::os::windows::io::FromRawHandle;
        if bytes.len() > MAX_HOST_BYTES {
            return Err(invalid());
        }
        let path = self.child(name)?;
        let wide = to_wide(path.as_os_str())?;
        let descriptor = protected_descriptor(&self.user)?;
        let attrs = SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor.0,
            bInheritHandle: false.into(),
        };
        // SAFETY: create-new in the pinned, protected host directory; explicit protected DACL,
        // terminated child path and live attributes. Successful handle gets a unique File owner.
        let handle = unsafe {
            CreateFileW(
                PCWSTR(wide.as_ptr()),
                FILE_GENERIC_WRITE.0,
                FILE_SHARE_READ,
                Some(&attrs),
                CREATE_NEW,
                FILE_ATTRIBUTE_NORMAL | FILE_FLAG_OPEN_REPARSE_POINT,
                None,
            )
        }
        .map_err(win_error)?;
        // SAFETY: successful CreateFileW transferred its unique live handle to File.
        let mut file = unsafe { File::from_raw_handle(handle.0) };
        let result = file
            .write_all(bytes)
            .and_then(|()| file.sync_all())
            .map_err(|_| invalid());
        drop(file);
        if result.is_err() {
            let _ = self.remove(name);
        }
        result
    }
    fn rename(&self, from: &str, to: &str, replace: bool) -> Result<(), MoveError> {
        let operation = (|| {
            let from = self.child(from)?;
            let to = self.child(to)?;
            drop(open_checked(&from, false, true, &self.user)?.ok_or_else(invalid)?);
            drop(open_checked(&to, false, true, &self.user)?);
            Ok::<_, PlatformError>((to_wide(from.as_os_str())?, to_wide(to.as_os_str())?))
        })()
        .map_err(MoveError::Failed)?;
        let flags = if replace {
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH
        } else {
            MOVEFILE_WRITE_THROUGH
        };
        // SAFETY: both validated regular children of pinned protected ancestors, terminated
        // paths live through the call. No-replace is used for move-aside and rollback.
        match unsafe {
            MoveFileExW(
                PCWSTR(operation.0.as_ptr()),
                PCWSTR(operation.1.as_ptr()),
                flags,
            )
        } {
            Ok(()) => Ok(()),
            Err(e)
                if e.code() == ERROR_ACCESS_DENIED.to_hresult()
                    || e.code() == ERROR_SHARING_VIOLATION.to_hresult() =>
            {
                Err(MoveError::Busy)
            }
            Err(e) => Err(MoveError::Failed(win_error(e))),
        }
    }
    fn remove(&self, name: &str) -> PlatformResult<()> {
        let path = self.child(name)?;
        let Some(_validated) = open_checked(&path, false, true, &self.user)? else {
            return Ok(());
        };
        // Caller proves exact manifest/hash receipt/temp/obsolete-name ownership; pinned ancestry
        // and the validated delete-shared regular file handle rule out reparse traversal.
        std::fs::remove_file(&path).map_err(|e| PlatformError::Failed(e.to_string()))
    }
    fn names(&self) -> PlatformResult<Vec<String>> {
        let entries =
            std::fs::read_dir(&self.path).map_err(|e| PlatformError::Failed(e.to_string()))?;
        entries
            .map(|e| {
                e.map_err(|e| PlatformError::Failed(e.to_string()))?
                    .file_name()
                    .into_string()
                    .map_err(|_| invalid())
            })
            .collect()
    }
}

// Desktop services: each HWND and registration is owned and destroyed on its creating thread.
use crate::windows_desktop::{self as desktop, ClipboardTransaction, LayoutPoll, Message};
use crate::{
    ClipboardStamp, KeyboardLayout, SecureClipboard, Subscription, SystemEvent, SystemEvents,
};
use std::{
    cell::RefCell,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    thread::{self, JoinHandle},
};
use windows::Win32::{
    Foundation::{GlobalFree, HGLOBAL, HINSTANCE, HWND, LPARAM, LRESULT, WPARAM},
    Globalization::{GetLocaleInfoEx, LCIDToLocaleName, LOCALE_SISO639LANGNAME},
    System::{
        DataExchange::*, Diagnostics::Debug::ReadProcessMemory, LibraryLoader::GetModuleHandleW,
        Memory::*, Power::*, RemoteDesktop::*, SystemServices::GUID_CONSOLE_DISPLAY_STATE,
    },
    UI::{Input::KeyboardAndMouse::GetKeyboardLayout, WindowsAndMessaging::*},
};

pub struct WinSystemEvents;
impl SystemEvents for WinSystemEvents {
    fn subscribe(
        &self,
        sink: Arc<dyn Fn(SystemEvent) + Send + Sync>,
    ) -> PlatformResult<Subscription> {
        let owner = DesktopThread::start(WindowMode::Events(sink))?;
        Ok(Subscription::new(move || drop(owner)))
    }
}

/// Construct on the main MyPass UI thread. Command workers never become the fallback thread.
pub struct WinKeyboardLayout {
    ui_thread: u32,
}
impl WinKeyboardLayout {
    pub fn capture() -> Self {
        // SAFETY: returns this thread's identity without borrowing OS memory.
        Self {
            ui_thread: unsafe { GetCurrentThreadId() },
        }
    }
}
impl KeyboardLayout for WinKeyboardLayout {
    fn current(&self) -> PlatformResult<Option<String>> {
        query_layout(self.ui_thread)
    }
    fn subscribe(
        &self,
        sink: Arc<dyn Fn(Option<String>) + Send + Sync>,
    ) -> PlatformResult<Subscription> {
        let owner = DesktopThread::start(layout_mode(self.ui_thread, sink)?)?;
        Ok(Subscription::new(move || drop(owner)))
    }
}
fn query_layout(ui_thread: u32) -> PlatformResult<Option<String>> {
    let mut pid = 0;
    // SAFETY: the optional foreground HWND can disappear; these queries then return zero.
    // pid is a valid writable DWORD. Only our own process's foreground thread is selected.
    let foreground = unsafe { GetForegroundWindow() };
    // SAFETY: valid local output; handles are only queried, never dereferenced by Rust.
    let foreground_thread = unsafe { GetWindowThreadProcessId(foreground, Some(&mut pid)) };
    // SAFETY: process identity is a scalar, no handle ownership.
    let current_pid = unsafe { GetCurrentProcessId() };
    let target = desktop::layout_thread(current_pid, pid, foreground_thread, ui_thread);
    // SAFETY: target is a captured MyPass thread id, never an arbitrary command-worker id.
    let hkl = unsafe { GetKeyboardLayout(target) };
    if hkl.0.is_null() {
        return Ok(None);
    }
    let lang = (hkl.0 as usize & 0xffff) as u16;
    let mut name = [0u16; 85];
    // SAFETY: buffers are sized slices and LANGID is promoted to its default-sort LCID.
    let count = unsafe { LCIDToLocaleName(u32::from(lang), Some(&mut name), 0) };
    let mut iso = [0u16; 16];
    let count = if count > 0 {
        // SAFETY: a successful LCIDToLocaleName terminated name within the live buffer.
        unsafe {
            GetLocaleInfoEx(
                PCWSTR(name.as_ptr()),
                LOCALE_SISO639LANGNAME,
                Some(&mut iso),
            )
        }
    } else {
        0
    };
    let label = usize::try_from(count)
        .ok()
        .and_then(|n| n.checked_sub(1))
        .and_then(|n| iso.get(..n))
        .and_then(|s| String::from_utf16(s).ok());
    Ok(desktop::layout_label(lang, label.as_deref()))
}
fn layout_mode(
    ui_thread: u32,
    sink: Arc<dyn Fn(Option<String>) + Send + Sync>,
) -> PlatformResult<WindowMode> {
    Ok(WindowMode::Layout {
        ui_thread,
        poll: LayoutPoll::new(query_layout(ui_thread)?),
        sink,
    })
}

enum WindowMode {
    Events(Arc<dyn Fn(SystemEvent) + Send + Sync>),
    Layout {
        ui_thread: u32,
        poll: LayoutPoll,
        sink: Arc<dyn Fn(Option<String>) + Send + Sync>,
    },
    Clipboard,
}
struct WindowState {
    active: Arc<AtomicBool>,
    mode: RefCell<WindowMode>,
}
struct HiddenWindow {
    hwnd: HWND,
    class: Vec<u16>,
    instance: HINSTANCE,
}
impl HiddenWindow {
    fn new(state: &WindowState) -> PlatformResult<Self> {
        let class = to_wide(OsStr::new(&format!(
            "MyPass.Desktop.{}",
            uuid::Uuid::new_v4()
        )))?;
        // SAFETY: query this loaded executable, without transferring module ownership.
        let instance = HINSTANCE(unsafe { GetModuleHandleW(None) }.map_err(win_error)?.0);
        let wc = WNDCLASSW {
            lpfnWndProc: Some(desktop_proc),
            hInstance: instance,
            lpszClassName: PCWSTR(class.as_ptr()),
            ..Default::default()
        };
        // SAFETY: wc and its class name remain live through registration; callback is static.
        if unsafe { RegisterClassW(&wc) } == 0 {
            return Err(win_error(windows::core::Error::from_thread()));
        }
        let mut result = Self {
            hwnd: HWND::default(),
            class,
            instance,
        };
        // SAFETY: state is pinned in a Box by the caller until AFTER this HWND is destroyed.
        // WS_POPUP without WS_VISIBLE/parent is a hidden TOP-LEVEL window, not HWND_MESSAGE:
        // broadcast power notifications need a top-level recipient. WS_EX_TOOLWINDOW hides taskbar UI.
        result.hwnd = unsafe {
            CreateWindowExW(
                WS_EX_TOOLWINDOW,
                PCWSTR(result.class.as_ptr()),
                w!("MyPass desktop service"),
                WS_POPUP,
                0,
                0,
                0,
                0,
                None,
                None,
                Some(instance),
                Some(std::ptr::from_ref(state).cast()),
            )
        }
        .map_err(win_error)?;
        Ok(result)
    }
}
impl Drop for HiddenWindow {
    fn drop(&mut self) {
        // SAFETY: this owner never leaves the creating thread; destroy precedes state/class drop.
        unsafe {
            if !self.hwnd.0.is_null() && IsWindow(Some(self.hwnd)).as_bool() {
                let _ = DestroyWindow(self.hwnd);
            }
            let _ = UnregisterClassW(PCWSTR(self.class.as_ptr()), Some(self.instance));
        }
    }
}

struct Notifications {
    hwnd: HWND,
    session: bool,
    suspend: Option<HPOWERNOTIFY>,
    display: Option<HPOWERNOTIFY>,
    timer: bool,
}
impl Notifications {
    fn register(hwnd: HWND, mode: &WindowMode) -> PlatformResult<Self> {
        let mut owner = Self {
            hwnd,
            session: false,
            suspend: None,
            display: None,
            timer: false,
        };
        match mode {
            WindowMode::Events(_) => {
                // SAFETY: live HWND on this thread. Each successful registration is immediately
                // recorded in RAII ownership, so any later failure rolls all earlier ones back.
                unsafe { WTSRegisterSessionNotification(hwnd, NOTIFY_FOR_THIS_SESSION) }
                    .map_err(win_error)?;
                owner.session = true;
                // SAFETY: valid HWND recipient with WINDOW_HANDLE flag, retained until unregister.
                owner.suspend = Some(
                    unsafe {
                        RegisterSuspendResumeNotification(
                            HANDLE(hwnd.0),
                            DEVICE_NOTIFY_WINDOW_HANDLE,
                        )
                    }
                    .map_err(win_error)?,
                );
                // SAFETY: static GUID and same owned HWND as above.
                owner.display = Some(
                    unsafe {
                        RegisterPowerSettingNotification(
                            HANDLE(hwnd.0),
                            &GUID_CONSOLE_DISPLAY_STATE,
                            DEVICE_NOTIFY_WINDOW_HANDLE,
                        )
                    }
                    .map_err(win_error)?,
                );
            }
            WindowMode::Layout { .. } => {
                // SAFETY: window-owned timer dispatched only by this thread's message loop;
                // no callback pointer and no timer exists outside a live subscription.
                if unsafe { SetTimer(Some(hwnd), 1, 500, None) } == 0 {
                    return Err(win_error(windows::core::Error::from_thread()));
                }
                owner.timer = true;
            }
            WindowMode::Clipboard => {}
        }
        Ok(owner)
    }
}
impl Drop for Notifications {
    fn drop(&mut self) {
        // SAFETY: all handles were returned by the matching registration; this runs before HWND
        // destruction, on its owner thread. Reverse-order cleanup also handles partial startup.
        unsafe {
            if self.timer {
                let _ = KillTimer(Some(self.hwnd), 1);
            }
            if let Some(h) = self.display.take() {
                let _ = UnregisterPowerSettingNotification(h);
            }
            if let Some(h) = self.suspend.take() {
                let _ = UnregisterSuspendResumeNotification(h);
            }
            if self.session {
                let _ = WTSUnRegisterSessionNotification(self.hwnd);
            }
        }
    }
}

/// Scalar handle storage permits sharing the kernel event without making a thread-affine HWND
/// Send. The last Arc closes it only after both cancellation and the message loop finish.
struct StopEvent(usize);
impl StopEvent {
    fn new() -> PlatformResult<Arc<Self>> {
        // SAFETY: unnamed manual-reset event, initially unsignaled, uniquely owned on success.
        let handle = unsafe { CreateEventW(None, true, false, None) }.map_err(win_error)?;
        Ok(Arc::new(Self(handle.0 as usize)))
    }
    fn handle(&self) -> HANDLE {
        HANDLE(self.0 as *mut _)
    }
    fn signal(&self) {
        // SAFETY: this Arc retains a live kernel event; it is closed only on last-owner drop.
        unsafe {
            let _ = SetEvent(self.handle());
        }
    }
    fn stopped(&self) -> bool {
        // SAFETY: same event lifetime; a zero-time wait never blocks.
        unsafe { WaitForSingleObject(self.handle(), 0).0 == 0 }
    }
}
impl Drop for StopEvent {
    fn drop(&mut self) {
        // SAFETY: last unique owner of the CreateEventW allocation, after all users finished.
        unsafe {
            let _ = CloseHandle(self.handle());
        }
    }
}
struct DesktopThread {
    #[cfg(test)]
    hwnd: usize,
    active: Arc<AtomicBool>,
    stop: Arc<StopEvent>,
    join: Option<JoinHandle<()>>,
}
impl DesktopThread {
    fn start(mode: WindowMode) -> PlatformResult<Self> {
        let active = Arc::new(AtomicBool::new(true));
        let state_active = active.clone();
        let stop = StopEvent::new()?;
        let worker_stop = stop.clone();
        let (ready, receive) = mpsc::sync_channel(1);
        let join = thread::Builder::new()
            .name("mypass-desktop".into())
            .spawn(move || {
                let state = Box::new(WindowState {
                    active: state_active,
                    mode: RefCell::new(mode),
                });
                let run = || -> PlatformResult<()> {
                    let window = HiddenWindow::new(&state)?;
                    let registrations = Notifications::register(window.hwnd, &state.mode.borrow())?;
                    if ready.send(Ok(window.hwnd.0 as usize)).is_err() {
                        return Ok(());
                    }
                    let mut message = MSG::default();
                    'messages: loop {
                        // SAFETY: the event and queue belong to this live worker. The event avoids
                        // shutdown depending on a successful PostMessage to a potentially full queue.
                        let wait = unsafe {
                            MsgWaitForMultipleObjectsEx(
                                Some(&[worker_stop.handle()]),
                                INFINITE,
                                QS_ALLINPUT,
                                MWMO_INPUTAVAILABLE,
                            )
                        };
                        if wait.0 != 1 {
                            break;
                        } // stop signaled or native wait failed
                        while !worker_stop.stopped() {
                            // SAFETY: writable MSG and this thread's queue. Dispatch only retrieved messages.
                            if !unsafe { PeekMessageW(&mut message, None, 0, 0, PM_REMOVE) }
                                .as_bool()
                            {
                                break;
                            }
                            if message.message == WM_QUIT {
                                break 'messages;
                            }
                            // SAFETY: retrieved message, window and callback state remain live.
                            unsafe {
                                let _ = TranslateMessage(&message);
                                DispatchMessageW(&message);
                            }
                        }
                        if worker_stop.stopped() {
                            break;
                        }
                    }
                    state.active.store(false, Ordering::Release);
                    if let WindowMode::Layout { poll, .. } = &mut *state.mode.borrow_mut() {
                        poll.cancel();
                    }
                    drop(registrations);
                    drop(window);
                    Ok(())
                };
                if let Err(error) = run() {
                    let _ = ready.send(Err(error));
                }
                // State/sink drops only after registrations and the HWND are gone.
            })
            .map_err(|e| PlatformError::Failed(e.to_string()))?;
        match receive.recv() {
            Ok(Ok(_hwnd)) => Ok(Self {
                #[cfg(test)]
                hwnd: _hwnd,
                active,
                stop,
                join: Some(join),
            }),
            Ok(Err(error)) => {
                let _ = join.join();
                Err(error)
            }
            Err(_) => {
                let _ = join.join();
                Err(PlatformError::Failed("desktop startup ended".into()))
            }
        }
    }
}
impl Drop for DesktopThread {
    fn drop(&mut self) {
        self.active.store(false, Ordering::Release);
        self.stop.signal();
        if let Some(join) = self.join.take() {
            // A sink may cancel its own subscription. Never self-join: the signaled event lets
            // the worker release its state as soon as that callback returns.
            if join.thread().id() != thread::current().id() {
                let _ = join.join();
            }
        }
    }
}

// Copy power broadcast data through ReadProcessMemory: even a malformed synthetic pointer must
// fail closed rather than fault the callback. Read only the fixed header, then exactly one DWORD.
#[repr(C)]
#[derive(Default)]
struct PowerHeader {
    guid: GUID,
    length: u32,
}
fn display_state(pointer: isize) -> Option<u32> {
    if pointer == 0 {
        return None;
    }
    let mut header = PowerHeader::default();
    let mut read = 0;
    // SAFETY: current-process read into a sized initialized local; OS validates the source address.
    unsafe {
        ReadProcessMemory(
            GetCurrentProcess(),
            pointer as *const _,
            std::ptr::from_mut(&mut header).cast(),
            size_of::<PowerHeader>(),
            Some(&mut read),
        )
    }
    .ok()?;
    if read != size_of::<PowerHeader>()
        || header.guid != GUID_CONSOLE_DISPLAY_STATE
        || header.length != 4
    {
        return None;
    }
    let mut data = 0u32;
    let address = (pointer as usize).checked_add(size_of::<PowerHeader>())?;
    // SAFETY: same checked-copy contract, reading only the validated DWORD payload length.
    unsafe {
        ReadProcessMemory(
            GetCurrentProcess(),
            address as *const _,
            std::ptr::from_mut(&mut data).cast(),
            size_of::<u32>(),
            Some(&mut read),
        )
    }
    .ok()?;
    (read == 4).then_some(data)
}
extern "system" fn desktop_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    // No Rust unwind, including a sink panic or RefCell reentrancy panic, crosses the ABI.
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        desktop_dispatch(hwnd, msg, wp, lp)
    }))
    .unwrap_or(LRESULT(0))
}
fn desktop_dispatch(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if msg == WM_NCCREATE {
        // SAFETY: WM_NCCREATE's CREATESTRUCTW and lpCreateParams are supplied by our own
        // CreateWindowExW. The pinned state lives beyond WM_NCDESTROY.
        unsafe {
            let create = &*(lp.0 as *const CREATESTRUCTW);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
        }
    }
    // SAFETY: the slot was set only by WM_NCCREATE and cleared on NCDESTROY; state is boxed
    // on this window's owner thread and never accessed through this slot on another thread.
    let state = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *const WindowState;
    if msg == WM_NCDESTROY {
        // SAFETY: stop exposing the state before the HWND can outlive it or be reused.
        unsafe {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
        }
    } else if let Some(state) = unsafe { state.as_ref() } {
        // SAFETY: lifetime invariant above.
        if state.active.load(Ordering::Acquire) {
            let mapped = match msg {
                WM_WTSSESSION_CHANGE => desktop::map_message(Message::Session(wp.0 as u32)),
                WM_POWERBROADCAST if wp.0 == PBT_POWERSETTINGCHANGE as usize => {
                    display_state(lp.0).and_then(|v| desktop::map_message(Message::Display(v)))
                }
                WM_POWERBROADCAST => desktop::map_message(Message::Power(wp.0 as u32)),
                _ => None,
            };
            let mut mode = state.mode.borrow_mut();
            match &mut *mode {
                WindowMode::Events(sink) => {
                    if let Some(event) = mapped {
                        sink(event);
                    }
                }
                WindowMode::Layout {
                    ui_thread,
                    poll,
                    sink,
                } if msg == WM_TIMER && wp.0 == 1 => {
                    if let Ok(next) = query_layout(*ui_thread) {
                        if let Some(next) = poll.tick(next) {
                            sink(next);
                        }
                    }
                }
                _ => {}
            }
        }
    }
    // SAFETY: pass all unhandled and lifecycle messages to the default procedure on this thread.
    unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
}

/// Each transaction uses an owned hidden HWND on a dedicated thread. No HWND, open clipboard
/// guard or native allocation crosses threads. Retain only a per-write key and keyed digest.
#[derive(Default)]
pub struct WinClipboard {
    written: std::sync::Mutex<Option<Arc<desktop::ClipboardWrite>>>,
}
fn clipboard_call<T: Send + 'static>(
    call: impl FnOnce(&mut NativeClipboard) -> PlatformResult<T> + Send + 'static,
) -> PlatformResult<T> {
    thread::Builder::new()
        .name("mypass-clipboard".into())
        .spawn(move || {
            let state = Box::new(WindowState {
                active: Arc::new(AtomicBool::new(true)),
                mode: RefCell::new(WindowMode::Clipboard),
            });
            let window = HiddenWindow::new(&state)?;
            let mut clipboard = NativeClipboard::open(window.hwnd)?;
            call(&mut clipboard)
            // close -> destroy HWND -> state drop, including on errors/unwind.
        })
        .map_err(|e| PlatformError::Failed(e.to_string()))?
        .join()
        .map_err(|_| PlatformError::Failed("clipboard worker ended".into()))?
}
impl SecureClipboard for WinClipboard {
    fn write_secret(&self, text: &str) -> PlatformResult<ClipboardStamp> {
        let mut written = self
            .written
            .lock()
            .map_err(|_| PlatformError::Failed("clipboard state poisoned".into()))?;
        let text = Zeroizing::new(text.to_owned());
        let next = clipboard_call(move |tx| desktop::write_clipboard(tx, &text))?;
        let stamp = next.stamp.clone();
        *written = Some(Arc::new(next));
        Ok(stamp)
    }
    fn clear_if_unchanged(&self, stamp: &ClipboardStamp) -> PlatformResult<bool> {
        let mut written = self
            .written
            .lock()
            .map_err(|_| PlatformError::Failed("clipboard state poisoned".into()))?;
        let Some(identity) = written
            .as_ref()
            .filter(|identity| identity.stamp == *stamp)
            .cloned()
        else {
            return Ok(false);
        };
        let result = clipboard_call(move |tx| desktop::clear_clipboard(tx, &identity));
        if result.is_ok() {
            *written = None;
        }
        result
    }
}
struct NativeClipboard;
impl NativeClipboard {
    fn open(hwnd: HWND) -> PlatformResult<Self> {
        for attempt in 0..10 {
            // SAFETY: non-null live HWND on this thread. Success is uniquely owned by this guard.
            match unsafe { OpenClipboard(Some(hwnd)) } {
                Ok(()) => return Ok(Self),
                Err(e) if attempt == 9 => return Err(win_error(e)),
                Err(_) => thread::sleep(std::time::Duration::from_millis(10)),
            }
        }
        Err(PlatformError::Failed("clipboard contention".into()))
    }
}
impl Drop for NativeClipboard {
    fn drop(&mut self) {
        // SAFETY: exactly one close per successful open, on its opening thread.
        unsafe {
            let _ = CloseClipboard();
        }
    }
}
impl ClipboardTransaction for NativeClipboard {
    fn empty(&mut self) -> PlatformResult<()> {
        // SAFETY: this guard holds the OS clipboard open; its HWND remains live.
        unsafe { EmptyClipboard() }.map_err(win_error)
    }
    fn sequence(&self) -> PlatformResult<u64> {
        // SAFETY: non-mutating scalar query, still under our open OS clipboard transaction.
        let sequence = unsafe { GetClipboardSequenceNumber() };
        if sequence == 0 {
            return Err(PlatformError::Failed(
                "clipboard sequence unavailable".into(),
            ));
        }
        Ok(u64::from(sequence))
    }
    fn has_format(&self, format: &str) -> PlatformResult<bool> {
        let id = clipboard_format(format)?;
        // SAFETY: non-mutating availability query under this guard's open clipboard lock.
        Ok(unsafe { IsClipboardFormatAvailable(id) }.is_ok())
    }
    fn read_unicode_text(&self) -> PlatformResult<Option<Zeroizing<Vec<u8>>>> {
        if !self.has_format("CF_UNICODETEXT")? {
            return Ok(None);
        }
        // SAFETY: this guard holds the clipboard lock throughout handle lookup and readback.
        let data = unsafe { GetClipboardData(13) }.map_err(win_error)?;
        let handle = HGLOBAL(data.0);
        // SAFETY: CF_UNICODETEXT is an OS-owned HGLOBAL, pinned by the open clipboard.
        let length = unsafe { GlobalSize(handle) };
        if !(2..=desktop::MAX_CLIPBOARD_BYTES).contains(&length) {
            return Ok(None);
        }
        // SAFETY: the OS-owned allocation stays live, locked only for the bounded copy below.
        let pointer = unsafe { GlobalLock(handle) };
        if pointer.is_null() {
            return Err(win_error(windows::core::Error::from_thread()));
        }
        // SAFETY: GlobalLock succeeded and GlobalSize bounds this borrowed readable byte range.
        let bytes = unsafe { std::slice::from_raw_parts(pointer.cast::<u8>(), length) };
        let text = desktop::bounded_unicode_text(bytes);
        // SAFETY: release our successful lock, never free or modify the Windows-owned allocation.
        let _ = unsafe { GlobalUnlock(handle) };
        Ok(text)
    }
    fn set(&mut self, format: &str, bytes: &[u8]) -> PlatformResult<()> {
        let id = clipboard_format(format)?;
        let mut allocation = ClipboardMemory::new(bytes)?;
        // SAFETY: clipboard open, owner is non-null; unlocked GMEM_MOVEABLE transfers only on Ok.
        unsafe { SetClipboardData(id, Some(HANDLE(allocation.0 .0))) }.map_err(win_error)?;
        allocation.0 = HGLOBAL::default();
        Ok(())
    }
}
fn clipboard_format(format: &str) -> PlatformResult<u32> {
    if format == "CF_UNICODETEXT" {
        return Ok(13);
    }
    let name = to_wide(OsStr::new(format))?;
    // SAFETY: terminated name is borrowed for this call only.
    let id = unsafe { RegisterClipboardFormatW(PCWSTR(name.as_ptr())) };
    if id == 0 {
        return Err(win_error(windows::core::Error::from_thread()));
    }
    Ok(id)
}
struct ClipboardMemory(HGLOBAL);
impl ClipboardMemory {
    fn new(bytes: &[u8]) -> PlatformResult<Self> {
        if bytes.is_empty() {
            return Err(PlatformError::Failed(
                "zero-size clipboard allocation".into(),
            ));
        }
        // SAFETY: request a nonzero movable, zero-initialized block. In particular, the legacy
        // presence marker needs one byte: SetClipboardData rejects zero-size discarded handles.
        let allocation = Self(
            unsafe { GlobalAlloc(GMEM_MOVEABLE | GMEM_ZEROINIT, bytes.len()) }
                .map_err(win_error)?,
        );
        if !bytes.is_empty() {
            // SAFETY: uniquely owned movable block, alive and at least bytes.len() bytes long.
            let pointer = unsafe { GlobalLock(allocation.0) };
            if pointer.is_null() {
                return Err(win_error(windows::core::Error::from_thread()));
            }
            // SAFETY: valid locked allocation, exact requested byte range, disjoint source.
            unsafe {
                std::ptr::copy_nonoverlapping(bytes.as_ptr(), pointer.cast(), bytes.len());
                let _ = GlobalUnlock(allocation.0);
            }
        }
        Ok(allocation)
    }
}
impl Drop for ClipboardMemory {
    fn drop(&mut self) {
        if self.0 .0.is_null() {
            return;
        }
        // SAFETY: only FAILED/untransferred allocations reach here. Successful SetClipboardData
        // disarms ownership; Windows-owned blocks are never zeroized, freed or written again.
        unsafe {
            let length = GlobalSize(self.0);
            let pointer = GlobalLock(self.0);
            if !pointer.is_null() {
                std::slice::from_raw_parts_mut(pointer.cast::<u8>(), length).zeroize();
                let _ = GlobalUnlock(self.0);
            }
            let _ = GlobalFree(Some(self.0));
        }
    }
}

#[cfg(test)]
fn window_exists(hwnd: usize) -> bool {
    // SAFETY: scalar HWND validity query only.
    unsafe { IsWindow(Some(HWND(hwnd as *mut _))).as_bool() }
}
#[cfg(test)]
fn send_native(hwnd: usize, msg: u32, wp: usize, lp: isize) -> PlatformResult<()> {
    let mut result = 0;
    // SAFETY: bounded synchronous send; caller owns and retains any payload for the entire call.
    if unsafe {
        SendMessageTimeoutW(
            HWND(hwnd as *mut _),
            msg,
            WPARAM(wp),
            LPARAM(lp),
            SMTO_ABORTIFHUNG,
            2000,
            Some(&mut result),
        )
    }
    .0 == 0
    {
        return Err(win_error(windows::core::Error::from_thread()));
    }
    Ok(())
}
#[cfg(test)]
fn synthetic_message(hwnd: usize, message: Message) -> PlatformResult<()> {
    match message {
        Message::Session(v) => send_native(hwnd, WM_WTSSESSION_CHANGE, v as usize, 0),
        Message::Power(v) => send_native(hwnd, WM_POWERBROADCAST, v as usize, 0),
        Message::Display(v) => {
            #[repr(C)]
            struct Packet {
                header: PowerHeader,
                data: u32,
            }
            let packet = Packet {
                header: PowerHeader {
                    guid: GUID_CONSOLE_DISPLAY_STATE,
                    length: 4,
                },
                data: v,
            };
            send_native(
                hwnd,
                WM_POWERBROADCAST,
                PBT_POWERSETTINGCHANGE as usize,
                std::ptr::from_ref(&packet) as isize,
            )
        }
    }
}
#[cfg(test)]
fn tick_layout(hwnd: usize) -> PlatformResult<()> {
    send_native(hwnd, WM_TIMER, 1, 0)
}

#[cfg(test)]
mod desktop_windows_integration {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)] // test-only support code
    use super::*;
    use crate::{
        KeyboardLayout, SecureClipboard,
        SystemEvent::{ScreenLocked, ScreenUnlocked, Sleep, Wake},
    };
    use std::sync::{mpsc, Arc};
    fn opt_in() {
        assert_eq!(
            std::env::var("MYPASS_WINDOWS_INTEGRATION_TESTS").as_deref(),
            Ok("1")
        );
    }
    #[test]
    #[ignore = "native Windows windows/events; isolated integration runner only"]
    fn windows_integration_events_independent_shutdown_and_panic_containment() {
        opt_in();
        let (tx, rx) = mpsc::channel();
        let thread = DesktopThread::start(WindowMode::Events(Arc::new(move |e| {
            tx.send(e).unwrap();
        })))
        .unwrap();
        let other = DesktopThread::start(WindowMode::Events(Arc::new(|_| {}))).unwrap();
        assert_ne!(thread.hwnd, other.hwnd);
        for (msg, want) in [
            (crate::windows_desktop::Message::Session(7), ScreenLocked),
            (crate::windows_desktop::Message::Session(8), ScreenUnlocked),
            (crate::windows_desktop::Message::Power(4), Sleep),
            (crate::windows_desktop::Message::Power(18), Wake),
            (crate::windows_desktop::Message::Display(0), Sleep),
            (crate::windows_desktop::Message::Display(2), Wake),
        ] {
            synthetic_message(thread.hwnd, msg).unwrap();
            assert_eq!(
                rx.recv_timeout(std::time::Duration::from_secs(3)).unwrap(),
                want
            );
        }
        let old = thread.hwnd;
        drop(thread);
        assert!(!window_exists(old));
        assert!(synthetic_message(old, crate::windows_desktop::Message::Session(7)).is_err());
        let (tx, rx) = mpsc::channel();
        let panicking = DesktopThread::start(WindowMode::Events(Arc::new(move |e| {
            if e == ScreenLocked {
                panic!("injected sink panic")
            }
            tx.send(e).unwrap();
        })))
        .unwrap();
        synthetic_message(panicking.hwnd, crate::windows_desktop::Message::Session(7)).unwrap();
        synthetic_message(panicking.hwnd, crate::windows_desktop::Message::Session(8)).unwrap();
        assert_eq!(
            rx.recv_timeout(std::time::Duration::from_secs(3)).unwrap(),
            ScreenUnlocked
        );
    }
    #[test]
    #[ignore = "native Windows clipboard; serial isolated integration runner only"]
    fn windows_integration_clipboard_markers_unicode_foreign_copy_and_contention() {
        opt_in();
        native_clipboard_checks();
    }
    #[test]
    #[ignore = "native Windows callbacks; isolated integration runner only"]
    fn windows_integration_event_sink_can_cancel_itself_and_power_payload_is_validated() {
        opt_in();
        struct Dropped(mpsc::Sender<()>);
        impl Drop for Dropped {
            fn drop(&mut self) {
                let _ = self.0.send(());
            }
        }
        let (dropped, finished) = mpsc::channel();
        let dropped = Dropped(dropped);
        let subscription = Arc::new(std::sync::Mutex::new(None::<Subscription>));
        let slot = subscription.clone();
        let (tx, rx) = mpsc::channel();
        let thread = DesktopThread::start(WindowMode::Events(Arc::new(move |_| {
            let _keep_until_state_drops = &dropped;
            slot.lock().unwrap().take();
            tx.send(()).unwrap();
        })))
        .unwrap();
        let hwnd = thread.hwnd;
        *subscription.lock().unwrap() = Some(Subscription::new(move || drop(thread)));
        // Invalid pointers and a mismatched GUID/size cannot produce an event or memory fault.
        send_native(hwnd, WM_POWERBROADCAST, PBT_POWERSETTINGCHANGE as usize, 1).unwrap();
        assert!(rx.try_recv().is_err());
        assert_eq!(display_state(0), None);
        let header = PowerHeader {
            guid: GUID::zeroed(),
            length: 4,
        };
        assert_eq!(display_state(std::ptr::from_ref(&header) as isize), None);
        let header = PowerHeader {
            guid: GUID_CONSOLE_DISPLAY_STATE,
            length: 3,
        };
        assert_eq!(display_state(std::ptr::from_ref(&header) as isize), None);
        synthetic_message(hwnd, Message::Session(7)).unwrap();
        rx.recv_timeout(std::time::Duration::from_secs(3)).unwrap();
        assert!(subscription.lock().unwrap().is_none());
        finished
            .recv_timeout(std::time::Duration::from_secs(3))
            .unwrap();
        assert!(!window_exists(hwnd));
    }
    fn native_clipboard_checks() {
        // This is deliberately destructive ONLY in the serial, isolated opt-in CI session. No
        // text-only backup pretends to preserve arbitrary user formats. Cleanup uses the stamp.
        let clipboard = WinClipboard::default();
        let stamp = clipboard.write_secret("A\u{0443}\u{1f512}").unwrap();
        clipboard_call(move |tx| {
            for (format, payload) in desktop::EXCLUSIONS {
                let name = to_wide(OsStr::new(format)).unwrap();
                // SAFETY: registered name and currently open clipboard; handle stays OS-owned.
                let id = unsafe { RegisterClipboardFormatW(PCWSTR(name.as_ptr())) };
                assert!(unsafe { IsClipboardFormatAvailable(id) }.is_ok());
                let data = unsafe { GetClipboardData(id) }.unwrap();
                if !payload.is_empty() {
                    let handle = HGLOBAL(data.0);
                    // SAFETY: OS-owned HGLOBAL remains pinned under this clipboard lock.
                    assert!(unsafe { GlobalSize(handle) } >= payload.len());
                    let pointer = unsafe { GlobalLock(handle) };
                    assert!(!pointer.is_null());
                    assert_eq!(
                        unsafe { std::slice::from_raw_parts(pointer.cast::<u8>(), payload.len()) },
                        payload
                    );
                    let _ = unsafe { GlobalUnlock(handle) };
                }
            }
            assert_eq!(
                *tx.read_unicode_text()?.unwrap(),
                [65, 0, 67, 4, 61, 216, 18, 221, 0, 0]
            );
            // Deterministic OS-lock barrier: another thread's entire bounded open retry fails
            // while this transaction remains open. No probabilistic alternating loop.
            let other = std::thread::spawn(|| WinClipboard::default().write_secret("blocked"));
            assert!(other.join().unwrap().is_err());
            Ok(())
        })
        .unwrap();
        // write_secret returned only after CloseClipboard synthesis and owner HWND destruction.
        assert!(clipboard.clear_if_unchanged(&stamp).unwrap());
        assert!(!clipboard.clear_if_unchanged(&stamp).unwrap());
        for foreign_text in ["F", "A\u{0443}\u{1f512}"] {
            let stamp = clipboard.write_secret("A\u{0443}\u{1f512}").unwrap();
            let bytes: Vec<_> = foreign_text
                .encode_utf16()
                .chain([0])
                .flat_map(u16::to_le_bytes)
                .collect();
            clipboard_call(move |tx| {
                tx.empty()?;
                tx.set("CF_UNICODETEXT", &bytes)
            })
            .unwrap();
            assert!(!clipboard.clear_if_unchanged(&stamp).unwrap());
            clipboard_call(move |tx| {
                let bytes = tx.read_unicode_text()?.unwrap();
                let expected: Vec<_> = foreign_text
                    .encode_utf16()
                    .chain([0])
                    .flat_map(u16::to_le_bytes)
                    .collect();
                assert_eq!(*bytes, expected);
                assert!(!tx.has_format(desktop::OUR_MARKER)?);
                Ok(())
            })
            .unwrap();
        }
        let stale = clipboard.write_secret("cleanup").unwrap();
        let again = clipboard.write_secret("cleanup").unwrap();
        assert!(!clipboard.clear_if_unchanged(&stale).unwrap());
        assert!(clipboard.clear_if_unchanged(&again).unwrap());
        assert!(!clipboard.clear_if_unchanged(&again).unwrap());
    }
    #[test]
    #[ignore = "native Windows layout; integration runner only"]
    fn windows_integration_layout_owned_timer_shutdown() {
        opt_in();
        let layout = WinKeyboardLayout::capture();
        let current = layout.current().unwrap().unwrap();
        assert!((2..=3).contains(&current.len()));
        assert!(current.bytes().all(|b| b.is_ascii_uppercase()));
        let thread =
            DesktopThread::start(layout_mode(layout.ui_thread, Arc::new(|_| {})).unwrap()).unwrap();
        let hwnd = thread.hwnd;
        tick_layout(hwnd).unwrap();
        drop(thread);
        assert!(!window_exists(hwnd));
    }
}

/// Last-resort dialog before WebView2 exists; text buffers live for the modal call.
pub fn show_startup_error(title: &str, message: &str) {
    use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};
    let (Ok(title), Ok(message)) = (to_wide(OsStr::new(title)), to_wide(OsStr::new(message)))
    else {
        return;
    };
    // SAFETY: both buffers are NUL-terminated and live through the synchronous dialog; no owner.
    unsafe {
        MessageBoxW(
            None,
            windows::core::PCWSTR(message.as_ptr()),
            windows::core::PCWSTR(title.as_ptr()),
            MB_OK | MB_ICONERROR,
        );
    }
}
