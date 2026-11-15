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
    let mut platform = crate::fallback::platform(paths);
    platform.secrets = std::sync::Arc::new(crate::DpapiSecretStore::app(paths));
    platform.shared_secrets = crate::app_shared_store(paths, None).1;
    platform
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
            ERROR_INSUFFICIENT_BUFFER, ERROR_PATH_NOT_FOUND, HANDLE, HLOCAL, WAIT_TIMEOUT,
        },
        NetworkManagement::IpHelper::*,
        Networking::WinSock::AF_INET,
        Security::{Authorization::*, Cryptography::*, *},
        Storage::FileSystem::*,
        System::Threading::*,
    },
};
use zeroize::{Zeroize, Zeroizing};

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
