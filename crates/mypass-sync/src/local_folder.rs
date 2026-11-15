//! The secondary-folder backend: one vault file in a folder the user picked (a USB stick, a
//! network share, a folder another sync client mirrors).
//!
//! It is a [`FileIoBackend`] over `PlainFileIo` (R20) plus a folder check before every operation:
//! a missing folder (an unplugged volume, an unmounted share) is `Unavailable` and is never created,
//! because creating it would write the vault onto whatever disk now sits under that path.
use crate::backend::{FileIoBackend, StorageBackend, StorageError, WriteOutcome};
use mypass_kdbx::PlainFileIo;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::SystemTime,
};

/// Windows path length (`MAX_PATH`, 260) minus room for an 8.3 file name and the separator: a
/// folder at or past this length gets the `\\?\` verbatim form, which lifts the limit.
const WINDOWS_LONG_FOLDER_LEN: usize = 248;

/// One vault file `file_name` inside the folder `dir`.
#[derive(Debug)]
pub struct LocalFolderBackend {
    dir: PathBuf,
    file_name: String,
    inner: FileIoBackend,
}

impl LocalFolderBackend {
    /// A backend for `dir/file_name`. `dir` must be absolute; it is normalised (trailing
    /// separators, duplicate and mixed separators; on Windows the `\\?\` form for long paths).
    /// The folder is not checked here: an unplugged volume is reported by each operation.
    pub fn new(dir: impl AsRef<Path>, file_name: &str) -> Result<Self, StorageError> {
        validate_file_name(file_name)?;
        let dir = normalise_dir(dir.as_ref());
        if !dir.is_absolute() {
            return Err(StorageError::InvalidPath(format!(
                "folder is not absolute: {}",
                dir.display()
            )));
        }
        let inner = FileIoBackend::new(Arc::new(PlainFileIo), dir.join(file_name));
        Ok(Self {
            dir,
            file_name: file_name.to_owned(),
            inner,
        })
    }

    /// Lower the read limit (tests). See [`FileIoBackend::with_read_limit`].
    pub fn with_read_limit(mut self, max_len: u64) -> Self {
        self.inner = self.inner.with_read_limit(max_len);
        self
    }

    /// The normalised folder.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The vault file name inside the folder.
    pub fn file_name(&self) -> &str {
        &self.file_name
    }

    /// The full path of the vault file.
    pub fn path(&self) -> &Path {
        self.inner.path()
    }

    fn check_dir(&self) -> Result<(), StorageError> {
        match fs::metadata(&self.dir) {
            Ok(meta) if meta.is_dir() => Ok(()),
            Ok(_) => Err(StorageError::Unavailable(format!(
                "not a folder: {}",
                self.dir.display()
            ))),
            Err(e) => Err(StorageError::Unavailable(format!(
                "folder unavailable ({:?}): {}",
                e.kind(),
                self.dir.display()
            ))),
        }
    }
}

impl StorageBackend for LocalFolderBackend {
    fn vault_path(&self) -> Option<&Path> {
        Some(self.path())
    }
    fn exists(&self) -> Result<bool, StorageError> {
        self.check_dir()?;
        self.inner.exists()
    }

    fn modification_date(&self) -> Result<Option<SystemTime>, StorageError> {
        self.check_dir()?;
        self.inner.modification_date()
    }

    fn read(&self) -> Result<Option<Vec<u8>>, StorageError> {
        self.check_dir()?;
        self.inner.read()
    }

    fn write(&self, bytes: &[u8]) -> Result<(), StorageError> {
        self.check_dir()?;
        self.inner.write(bytes)
    }

    fn write_if(
        &self,
        expected: Option<&[u8; 32]>,
        bytes: &[u8],
    ) -> Result<WriteOutcome, StorageError> {
        self.check_dir()?;
        self.inner.write_if(expected, bytes)
    }
}

fn validate_file_name(name: &str) -> Result<(), StorageError> {
    let bad = name.is_empty()
        || name == "."
        || name == ".."
        || name.contains(['/', '\\', '\0'])
        // `C:x` names a drive-relative path on Windows.
        || name.contains(':');
    if bad {
        return Err(StorageError::InvalidPath(format!(
            "not a plain file name: {name:?}"
        )));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PathStyle {
    Unix,
    Windows,
}

impl PathStyle {
    fn current() -> Self {
        if cfg!(windows) {
            Self::Windows
        } else {
            Self::Unix
        }
    }
}

fn normalise_dir(dir: &Path) -> PathBuf {
    match dir.to_str() {
        Some(s) => PathBuf::from(normalise_dir_str(s, PathStyle::current())),
        // A non-UTF-8 Unix path is used as given.
        None => dir.to_path_buf(),
    }
}

/// Normalise a folder path written in `style`: one separator between components, no trailing
/// separator (except a root), `/` read as `\` on Windows, and on Windows the `\\?\` verbatim form
/// once the folder is long. Pure, so both styles are tested on every target.
fn normalise_dir_str(raw: &str, style: PathStyle) -> String {
    match style {
        PathStyle::Unix => normalise_unix(raw),
        PathStyle::Windows => normalise_windows(raw),
    }
}

fn normalise_unix(raw: &str) -> String {
    let absolute = raw.starts_with('/');
    let parts: Vec<&str> = raw.split('/').filter(|p| !p.is_empty()).collect();
    let joined = parts.join("/");
    match (absolute, joined.is_empty()) {
        (true, _) => format!("/{joined}"),
        (false, true) => raw.to_owned(),
        (false, false) => joined,
    }
}

fn normalise_windows(raw: &str) -> String {
    let s = raw.replace('/', "\\");
    // Verbatim (`\\?\`) and device (`\\.\`) paths are passed through apart from the tail.
    for verbatim in ["\\\\?\\", "\\\\.\\"] {
        if let Some(rest) = s.strip_prefix(verbatim) {
            return format!("{verbatim}{}", join_windows(rest, false));
        }
    }
    let (unc, rest) = match s.strip_prefix("\\\\") {
        Some(rest) => (true, rest),
        None => (false, s.as_str()),
    };
    let tail = join_windows(rest, !unc);
    let plain = if unc {
        format!("\\\\{tail}")
    } else {
        tail.clone()
    };
    if plain.chars().count() < WINDOWS_LONG_FOLDER_LEN {
        return plain;
    }
    if unc {
        // `\\server\share\dir` -> `\\?\UNC\server\share\dir`.
        format!("\\\\?\\UNC\\{}", resolve_dots(&tail))
    } else if is_drive_absolute(&tail) {
        format!("\\\\?\\{}", resolve_dots(&tail))
    } else {
        plain
    }
}

/// Join the non-empty `\`-separated parts; keep a drive root (`C:\`) whole.
fn join_windows(rest: &str, may_be_drive: bool) -> String {
    let parts: Vec<&str> = rest.split('\\').filter(|p| !p.is_empty()).collect();
    let joined = parts.join("\\");
    let drive_root = may_be_drive && parts.len() == 1 && is_drive(&joined) && rest.contains('\\');
    if drive_root {
        format!("{joined}\\")
    } else {
        joined
    }
}

fn is_drive(s: &str) -> bool {
    let mut chars = s.chars();
    matches!((chars.next(), chars.next(), chars.next()), (Some(c), Some(':'), None) if c.is_ascii_alphabetic())
}

fn is_drive_absolute(s: &str) -> bool {
    s.split('\\').next().is_some_and(is_drive) && s.contains('\\')
}

/// A verbatim path takes `.` and `..` literally, so resolve them first (Windows resolves them
/// lexically anyway). `..` never climbs above the first component (the drive or the share host).
fn resolve_dots(path: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    for part in path.split('\\') {
        match part {
            "." => {}
            ".." => {
                if out.len() > 1 {
                    out.pop();
                }
            }
            other => out.push(other),
        }
    }
    out.join("\\")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn entries(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn local_folder_round_trip_and_atomic_no_temp_left() {
        let d = tempfile::tempdir().unwrap();
        let backend = LocalFolderBackend::new(d.path(), "vault.kdbx").unwrap();
        assert_eq!(backend.exists(), Ok(false));
        assert_eq!(backend.read(), Ok(None));
        backend.write(b"first").unwrap();
        backend.write(b"second").unwrap();
        assert_eq!(backend.read(), Ok(Some(b"second".to_vec())));
        assert_eq!(backend.exists(), Ok(true));
        assert!(backend.modification_date().unwrap().is_some());
        // The atomic write leaves no `.vault.kdbx.*.tmp` behind.
        assert_eq!(entries(d.path()), ["vault.kdbx"]);
    }

    #[test]
    fn local_folder_missing_dir_is_unavailable_not_created() {
        let d = tempfile::tempdir().unwrap();
        let missing = d.path().join("unplugged");
        let backend = LocalFolderBackend::new(&missing, "vault.kdbx").unwrap();
        assert!(matches!(backend.read(), Err(StorageError::Unavailable(_))));
        assert!(matches!(
            backend.exists(),
            Err(StorageError::Unavailable(_))
        ));
        assert!(matches!(
            backend.modification_date(),
            Err(StorageError::Unavailable(_))
        ));
        assert!(matches!(
            backend.write(b"x"),
            Err(StorageError::Unavailable(_))
        ));
        assert!(!missing.exists());
    }

    #[test]
    fn local_folder_that_is_a_file_is_unavailable() {
        let d = tempfile::tempdir().unwrap();
        let file = d.path().join("not-a-folder");
        fs::write(&file, b"x").unwrap();
        let backend = LocalFolderBackend::new(&file, "vault.kdbx").unwrap();
        assert!(matches!(
            backend.write(b"y"),
            Err(StorageError::Unavailable(_))
        ));
        assert_eq!(fs::read(&file).unwrap(), b"x");
    }

    #[test]
    fn local_folder_rejects_relative_folder_and_path_like_names() {
        assert!(matches!(
            LocalFolderBackend::new("relative/dir", "vault.kdbx"),
            Err(StorageError::InvalidPath(_))
        ));
        let d = tempfile::tempdir().unwrap();
        for name in ["", ".", "..", "a/b.kdbx", "a\\b.kdbx", "C:x.kdbx", "a\0b"] {
            assert!(
                matches!(
                    LocalFolderBackend::new(d.path(), name),
                    Err(StorageError::InvalidPath(_))
                ),
                "{name:?}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn local_folder_read_only_folder_reports_read_only_and_keeps_the_file() {
        use std::os::unix::fs::PermissionsExt;
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("ro");
        fs::create_dir(&dir).unwrap();
        fs::write(dir.join("vault.kdbx"), b"keep").unwrap();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o500)).unwrap();
        let backend = LocalFolderBackend::new(&dir, "vault.kdbx").unwrap();
        let result = backend.write(b"new");
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(result, Err(StorageError::ReadOnly));
        assert_eq!(fs::read(dir.join("vault.kdbx")).unwrap(), b"keep");
    }

    #[test]
    fn local_folder_accepts_drive_letter_path() {
        // The path helper, on every target.
        let w = |s| normalise_dir_str(s, PathStyle::Windows);
        assert_eq!(w("C:\\Users\\me\\Sync"), "C:\\Users\\me\\Sync");
        assert_eq!(w("d:\\Vaults\\"), "d:\\Vaults");
        assert_eq!(w("C:\\"), "C:\\");
        assert_eq!(w("C:/"), "C:\\");
        assert_eq!(
            w("\\\\server\\share\\vaults\\"),
            "\\\\server\\share\\vaults"
        );
        // A real drive-letter folder on Windows.
        #[cfg(windows)]
        {
            let d = tempfile::tempdir().unwrap();
            let raw = d.path().to_str().unwrap().to_owned();
            assert!(is_drive(&raw[..2]), "{raw}");
            let backend = LocalFolderBackend::new(&raw, "vault.kdbx").unwrap();
            backend.write(b"drive").unwrap();
            assert_eq!(backend.read(), Ok(Some(b"drive".to_vec())));
        }
    }

    #[test]
    fn local_folder_accepts_non_ascii_directory_and_file_name() {
        let d = tempfile::tempdir().unwrap();
        // Ukrainian "keys" and Chinese "password": the folder, and the file name built from it.
        let folder = "\u{41a}\u{43b}\u{44e}\u{447}\u{456} \u{5bc6}\u{7801}";
        let name = format!("{folder} vault.kdbx");
        let dir = d.path().join(folder);
        fs::create_dir(&dir).unwrap();
        let backend = LocalFolderBackend::new(&dir, &name).unwrap();
        backend.write(b"unicode").unwrap();
        assert_eq!(backend.read(), Ok(Some(b"unicode".to_vec())));
        assert_eq!(entries(&dir), [name]);
    }

    #[test]
    fn local_folder_accepts_path_longer_than_260_chars() {
        // The helper turns a long Windows folder into the verbatim form, on every target.
        let long = format!("C:\\{}", vec!["x".repeat(60); 5].join("\\"));
        let verbatim = normalise_dir_str(&long, PathStyle::Windows);
        assert_eq!(verbatim, format!("\\\\?\\{long}"));
        let long_unc = format!("\\\\host\\share\\{}", "y".repeat(260));
        assert_eq!(
            normalise_dir_str(&long_unc, PathStyle::Windows),
            format!("\\\\?\\UNC\\host\\share\\{}", "y".repeat(260))
        );
        assert_eq!(
            normalise_dir_str(
                &format!("C:\\a\\..\\{}", "z".repeat(260)),
                PathStyle::Windows
            ),
            format!("\\\\?\\C:\\{}", "z".repeat(260))
        );
        // A real folder whose full path is over 300 characters (on Windows through `\\?\`).
        let d = tempfile::tempdir().unwrap();
        let mut dir = d.path().to_path_buf();
        while dir.as_os_str().len() < 300 {
            dir.push("d".repeat(50));
        }
        fs::create_dir_all(&dir).unwrap();
        let backend = LocalFolderBackend::new(&dir, "vault.kdbx").unwrap();
        #[cfg(windows)]
        assert!(backend.dir().to_str().unwrap().starts_with("\\\\?\\"));
        assert!(backend.path().as_os_str().len() > 300);
        backend.write(b"long").unwrap();
        assert_eq!(backend.read(), Ok(Some(b"long".to_vec())));
    }

    #[test]
    fn local_folder_trailing_separator_and_mixed_slashes_normalise() {
        let u = |s| normalise_dir_str(s, PathStyle::Unix);
        assert_eq!(u("/Users/me/Sync/"), "/Users/me/Sync");
        assert_eq!(u("/Users//me///Sync//"), "/Users/me/Sync");
        assert_eq!(u("/"), "/");
        assert_eq!(u("//"), "/");
        let w = |s| normalise_dir_str(s, PathStyle::Windows);
        assert_eq!(w("C:/Users\\me//Sync/"), "C:\\Users\\me\\Sync");
        assert_eq!(w("C:\\Users\\\\me\\Sync\\\\"), "C:\\Users\\me\\Sync");
        assert_eq!(w("\\\\?\\C:\\Vaults\\"), "\\\\?\\C:\\Vaults");
        // On the running platform, a trailing separator names the same file.
        let d = tempfile::tempdir().unwrap();
        let sep = std::path::MAIN_SEPARATOR;
        let raw = format!("{}{sep}{sep}", d.path().display());
        let backend = LocalFolderBackend::new(&raw, "vault.kdbx").unwrap();
        assert_eq!(backend.path(), d.path().join("vault.kdbx"));
        backend.write(b"slashes").unwrap();
        assert_eq!(
            fs::read(d.path().join("vault.kdbx")).unwrap(),
            b"slashes".to_vec()
        );
    }
}
