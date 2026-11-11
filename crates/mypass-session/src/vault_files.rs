//! Facts about vault files on disk: the status of the session's file, `.kdbx` discovery for the
//! welcome screen, and the open-time guards (iCloud location, the old Swift app running).

pub use crate::dto::{FoundVaultFileDto, OpenWarning, RecentVaultDto};
use serde::{Deserialize, Serialize};
use std::collections::{HashSet, VecDeque};
use std::ffi::OsStr;
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant, UNIX_EPOCH};

/// What the session's vault file looks like right now. Sizes and instants are `f64` (bytes and
/// Unix milliseconds) because the TypeScript bindings cannot carry 64-bit integers.
#[derive(Serialize, Deserialize, specta::Type, Clone, PartialEq, Debug)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum VaultFileStatus {
    /// The session has no vault path.
    NoVault,
    /// The file exists. `modified` is 0 when the file system reports no modification time.
    Connected { size: f64, modified: f64 },
    /// The path no longer exists (moved, deleted, unmounted).
    Missing,
    /// The path exists but its metadata cannot be read.
    Unreadable { message: String },
}

/// Stats `path` (one `metadata` call; never reads the content).
pub fn file_status(path: Option<&Path>) -> VaultFileStatus {
    let Some(path) = path else {
        return VaultFileStatus::NoVault;
    };
    match std::fs::metadata(path) {
        // A directory (or any non-file) is not a vault: report it rather than "connected".
        Ok(meta) if !meta.is_file() => VaultFileStatus::Unreadable {
            message: "the path is not a file".into(),
        },
        Ok(meta) => VaultFileStatus::Connected {
            size: meta.len() as f64,
            modified: millis(&meta),
        },
        Err(e) if e.kind() == ErrorKind::NotFound => VaultFileStatus::Missing,
        Err(e) => VaultFileStatus::Unreadable {
            message: e.to_string(),
        },
    }
}

// ---- discovery ----

/// Bounds of one scan.
#[derive(Clone, Debug)]
pub struct ScanLimits {
    /// Directories below a root that are still read (0 reads only the root itself).
    pub max_depth: usize,
    /// The scan stops once this many files are found.
    pub max_results: usize,
    /// The scan stops reading directories once this much time has passed.
    pub time_budget: Duration,
}

impl Default for ScanLimits {
    fn default() -> Self {
        Self {
            max_depth: 6,
            max_results: 100,
            time_budget: Duration::from_secs(10),
        }
    }
}

/// Directory extensions that are opaque packages: their insides are never a user's vault, and
/// walking a photo library or an app bundle is slow and pointless.
const OPAQUE_EXTENSIONS: [&str; 9] = [
    "app",
    "bundle",
    "framework",
    "plugin",
    "appex",
    "kext",
    "xpc",
    "photoslibrary",
    "photolibrary",
];

fn is_opaque_directory(name: &OsStr) -> bool {
    let path = Path::new(name);
    if name.to_string_lossy().eq_ignore_ascii_case("node_modules") {
        // Thousands of small directories, never a vault, and the classic scan-time sink.
        return true;
    }
    path.extension().is_some_and(|ext| {
        OPAQUE_EXTENSIONS
            .iter()
            .any(|opaque| ext.to_string_lossy().eq_ignore_ascii_case(opaque))
    })
}

fn is_hidden(name: &OsStr) -> bool {
    name.to_string_lossy().starts_with('.')
}

fn is_kdbx(name: &OsStr) -> bool {
    Path::new(name)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("kdbx"))
}

fn millis(meta: &std::fs::Metadata) -> f64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0.0, |d| d.as_millis() as f64)
}

/// The clock is also read every this many directory entries, so one huge flat directory cannot
/// outlast the time budget.
const ENTRIES_PER_CLOCK_CHECK: usize = 256;

/// Looks for `.kdbx` files below `roots`, newest modification first, unique by canonical path.
///
/// Bounded by depth, result count and time (`now` is the injected clock; it is read once at the
/// start and once before each directory is read). The roots are walked as given; nothing below
/// them is ever reached through a symlink. Unreadable directories are skipped silently. When the
/// result cap is hit the scan stops, so the newest files of an unvisited directory can be missed.
pub fn scan_for_vaults(
    roots: &[PathBuf],
    limits: &ScanLimits,
    now: impl Fn() -> Instant,
) -> Vec<FoundVaultFileDto> {
    let start = now();
    let mut found: Vec<FoundVaultFileDto> = Vec::new();
    let mut seen: HashSet<PathBuf> = HashSet::new();
    // Breadth first: shallow vaults (the likely ones) are found before the cap or the clock hit.
    let mut queue: VecDeque<(PathBuf, usize)> = roots.iter().map(|r| (r.clone(), 0)).collect();

    while let Some((dir, depth)) = queue.pop_front() {
        if found.len() >= limits.max_results
            || now().saturating_duration_since(start) >= limits.time_budget
        {
            break;
        }
        let Ok(read) = std::fs::read_dir(&dir) else {
            continue;
        };
        for (n, item) in read.enumerate() {
            if found.len() >= limits.max_results {
                break;
            }
            if n > 0
                && n % ENTRIES_PER_CLOCK_CHECK == 0
                && now().saturating_duration_since(start) >= limits.time_budget
            {
                break;
            }
            let Ok(item) = item else { continue };
            let name = item.file_name();
            // Hidden entries are skipped, which also covers the `.name.kdbx.icloud` placeholders
            // iCloud leaves for files that are not downloaded: opening one would start a download.
            if is_hidden(&name) {
                continue;
            }
            // `DirEntry::file_type` does not follow symlinks: a link (to a file or a directory,
            // possibly a cycle or a network mount) is neither a vault nor a directory to enter.
            let Ok(file_type) = item.file_type() else {
                continue;
            };
            if file_type.is_dir() {
                if depth < limits.max_depth && !is_opaque_directory(&name) {
                    queue.push_back((item.path(), depth + 1));
                }
            } else if file_type.is_file() && is_kdbx(&name) {
                let path = item.path();
                let canonical = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
                // The same file can be reached through overlapping roots (or a root that is itself
                // a link): list it once, by its real path.
                if !seen.insert(canonical) {
                    continue;
                }
                let Ok(meta) = item.metadata() else { continue };
                found.push(FoundVaultFileDto {
                    path: path.to_string_lossy().into_owned(),
                    modified: millis(&meta),
                    size: meta.len() as f64,
                });
            }
        }
    }
    // Newest first: the vault the user touched last is the one they most likely want; the path
    // breaks ties so the order is deterministic.
    found.sort_by(|a, b| {
        b.modified
            .total_cmp(&a.modified)
            .then_with(|| a.path.cmp(&b.path))
    });
    found
}

// ---- open-time guards ----

/// The bundle id of the MyPass app, shared with the old Swift app (D7).
pub const APP_BUNDLE_ID: &str = "com.mypass.app";

/// Finds running processes by bundle id. The real implementation (over
/// `mypass_platform::running_app_executables`) lives in the app shell.
pub trait ProcessLookup: Send + Sync {
    /// Executable paths of every running process with `bundle_id`.
    fn other_app_executables(&self, bundle_id: &str) -> Vec<PathBuf>;
}

/// `path` without `.` and resolvable `..` components, no file system access.
fn lexical(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !matches!(out.components().next_back(), Some(Component::Normal(_))) || !out.pop()
                {
                    out.push(component);
                }
            }
            other => out.push(other),
        }
    }
    out
}

/// `path` with symlinks resolved when it exists, or when its parent does (a file about to be
/// created); otherwise the lexical form.
fn resolved(path: &Path) -> PathBuf {
    if let Ok(real) = std::fs::canonicalize(path) {
        return real;
    }
    if let (Some(parent), Some(name)) = (path.parent(), path.file_name()) {
        if let Ok(real) = std::fs::canonicalize(parent) {
            return real.join(name);
        }
    }
    lexical(path)
}

/// Is `path` inside `<home>/Library/Mobile Documents` (iCloud Drive)? The comparison is by path
/// component, so `Mobile Documents backup` does not match, and it is made both as written and
/// with symlinks resolved, so a link into iCloud is caught. Never panics.
pub fn is_icloud_path(path: &Path, home: &Path) -> bool {
    let base = home.join("Library").join("Mobile Documents");
    lexical(path).starts_with(lexical(&base)) || resolved(path).starts_with(resolved(&base))
}

/// Two executable paths name the same program: canonical forms when both exist, else as given.
fn same_executable(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

/// What to confirm before creating or opening the vault at `path`: [`OpenWarning::Icloud`] for an
/// iCloud location, and one [`OpenWarning::OldAppRunning`] for every distinct `com.mypass.app`
/// executable that is not `current_exe` (the old Swift app). The UI requires an explicit Continue.
pub fn open_warnings(
    path: &Path,
    home: &Path,
    procs: &dyn ProcessLookup,
    current_exe: &Path,
) -> Vec<OpenWarning> {
    let mut warnings = Vec::new();
    // STAGE5-REMOVE: stage 5 Task 8 deletes this block (with `is_icloud_path`, the `Icloud`
    // variant and `icloud_path_warns_until_stage_5`) once vault files may live in iCloud Drive.
    if is_icloud_path(path, home) {
        warnings.push(OpenWarning::Icloud);
    }
    let mut seen: Vec<PathBuf> = Vec::new();
    for exe in procs.other_app_executables(APP_BUNDLE_ID) {
        if same_executable(&exe, current_exe) || seen.contains(&exe) {
            continue;
        }
        warnings.push(OpenWarning::OldAppRunning {
            executable: exe.to_string_lossy().into_owned(),
        });
        seen.push(exe);
    }
    warnings
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_status_reports_each_case() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("v.kdbx");
        assert_eq!(file_status(None), VaultFileStatus::NoVault);
        assert_eq!(file_status(Some(&path)), VaultFileStatus::Missing);
        std::fs::write(&path, b"12345").unwrap();
        match file_status(Some(&path)) {
            VaultFileStatus::Connected { size, modified } => {
                assert_eq!(size, 5.0);
                assert!(modified > 0.0);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn serialises_with_a_kind_tag() {
        let v = serde_json::to_value(VaultFileStatus::Connected {
            size: 1.0,
            modified: 2.0,
        })
        .unwrap();
        assert_eq!(
            v,
            serde_json::json!({"kind": "connected", "size": 1.0, "modified": 2.0})
        );
        assert_eq!(
            serde_json::to_value(VaultFileStatus::NoVault).unwrap(),
            serde_json::json!({"kind": "noVault"})
        );
    }

    // ---- status (the brief's four cases) ----

    #[test]
    fn no_path_is_no_vault() {
        assert_eq!(file_status(None), VaultFileStatus::NoVault);
    }

    #[test]
    fn existing_file_is_connected_with_size() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("v.kdbx");
        std::fs::write(&path, b"abc").unwrap();
        match file_status(Some(&path)) {
            VaultFileStatus::Connected { size, modified } => {
                assert_eq!(size, 3.0);
                assert!(modified > 0.0);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn missing_file_is_missing() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            file_status(Some(&dir.path().join("gone.kdbx"))),
            VaultFileStatus::Missing
        );
    }

    #[test]
    fn directory_path_is_unreadable() {
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(
            file_status(Some(dir.path())),
            VaultFileStatus::Unreadable { .. }
        ));
    }

    // ---- scanner ----

    fn touch(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, b"x").unwrap();
    }

    fn scan(root: &Path) -> Vec<FoundVaultFileDto> {
        scan_for_vaults(&[root.to_path_buf()], &ScanLimits::default(), Instant::now)
    }

    fn names(found: &[FoundVaultFileDto]) -> Vec<String> {
        let mut v: Vec<String> = found
            .iter()
            .map(|f| {
                Path::new(&f.path)
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        v.sort();
        v
    }

    #[test]
    fn finds_kdbx_up_to_depth_six_not_seven() {
        let dir = tempfile::tempdir().unwrap();
        let mut six = dir.path().to_path_buf();
        for part in ["a", "b", "c", "d", "e", "f"] {
            six.push(part);
        }
        touch(&six.join("six.kdbx"));
        touch(&six.join("g").join("seven.kdbx"));
        touch(&dir.path().join("root.kdbx"));
        assert_eq!(names(&scan(dir.path())), ["root.kdbx", "six.kdbx"]);
    }

    #[test]
    fn skips_hidden_and_icloud_placeholders() {
        let dir = tempfile::tempdir().unwrap();
        touch(&dir.path().join("ok.kdbx"));
        touch(&dir.path().join(".hidden.kdbx"));
        touch(&dir.path().join(".cloud.kdbx.icloud"));
        touch(&dir.path().join(".dir").join("inside.kdbx"));
        assert_eq!(names(&scan(dir.path())), ["ok.kdbx"]);
    }

    #[test]
    fn does_not_descend_into_bundle_directories_or_node_modules() {
        let dir = tempfile::tempdir().unwrap();
        for opaque in [
            "X.app",
            "X.bundle",
            "X.framework",
            "X.plugin",
            "X.appex",
            "X.kext",
            "X.xpc",
            "X.photoslibrary",
            "X.photolibrary",
            "X.APP",
            "node_modules",
        ] {
            touch(&dir.path().join(opaque).join("no.kdbx"));
        }
        touch(&dir.path().join("plain.dir").join("yes.kdbx"));
        assert_eq!(names(&scan(dir.path())), ["yes.kdbx"]);
    }

    #[cfg(unix)]
    #[test]
    fn does_not_follow_symlinks() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        touch(&outside.path().join("outside.kdbx"));
        touch(&outside.path().join("real.kdbx"));
        let root = dir.path().join("root");
        std::fs::create_dir_all(&root).unwrap();
        symlink(outside.path(), root.join("linked-dir")).unwrap();
        symlink(outside.path().join("real.kdbx"), root.join("linked.kdbx")).unwrap();
        symlink(&root, root.join("loop")).unwrap();
        touch(&root.join("own.kdbx"));
        assert_eq!(names(&scan(&root)), ["own.kdbx"]);
    }

    #[test]
    fn result_count_is_capped() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..10 {
            touch(&dir.path().join(format!("v{i}.kdbx")));
        }
        let limits = ScanLimits {
            max_results: 4,
            ..ScanLimits::default()
        };
        let found = scan_for_vaults(&[dir.path().to_path_buf()], &limits, Instant::now);
        assert_eq!(found.len(), 4);
    }

    #[test]
    fn time_budget_stops_scan() {
        let dir = tempfile::tempdir().unwrap();
        touch(&dir.path().join("top.kdbx"));
        touch(&dir.path().join("sub").join("deep.kdbx"));
        let base = Instant::now();
        // The first two reads (the scan start and the check before the first directory) see no
        // time passing; every read after that is 11 s on, however many more the scan makes.
        let calls = std::cell::Cell::new(0_u32);
        let clock = || {
            let n = calls.get();
            calls.set(n + 1);
            if n < 2 {
                base
            } else {
                base + Duration::from_secs(11)
            }
        };
        let found = scan_for_vaults(&[dir.path().to_path_buf()], &ScanLimits::default(), clock);
        assert_eq!(names(&found), ["top.kdbx"]);
    }

    #[test]
    fn time_budget_interrupts_one_huge_directory() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..(ENTRIES_PER_CLOCK_CHECK * 3) {
            touch(&dir.path().join(format!("v{i}.kdbx")));
        }
        let base = Instant::now();
        let calls = std::cell::Cell::new(0_u32);
        let clock = || {
            let n = calls.get();
            calls.set(n + 1);
            if n < 2 {
                base
            } else {
                base + Duration::from_secs(11)
            }
        };
        let found = scan_for_vaults(&[dir.path().to_path_buf()], &ScanLimits::default(), clock);
        assert!(!found.is_empty());
        assert!(found.len() < ENTRIES_PER_CLOCK_CHECK * 3);
    }

    #[test]
    fn non_kdbx_extensions_ignored_case_insensitively() {
        let dir = tempfile::tempdir().unwrap();
        touch(&dir.path().join("VAULT.KDBX"));
        touch(&dir.path().join("a.kdb"));
        touch(&dir.path().join("b.kdbx.bak"));
        touch(&dir.path().join("kdbx"));
        assert_eq!(names(&scan(dir.path())), ["VAULT.KDBX"]);
    }

    #[test]
    fn newest_modified_first() {
        let dir = tempfile::tempdir().unwrap();
        let old = dir.path().join("old.kdbx");
        let new = dir.path().join("new.kdbx");
        touch(&old);
        touch(&new);
        let base = std::time::SystemTime::now();
        let set = |p: &Path, t: std::time::SystemTime| {
            std::fs::File::options()
                .write(true)
                .open(p)
                .unwrap()
                .set_modified(t)
                .unwrap();
        };
        set(&old, base - Duration::from_secs(3600));
        set(&new, base);
        let found = scan(dir.path());
        assert_eq!(found.len(), 2);
        assert!(found[0].path.ends_with("new.kdbx"));
        assert!(found[0].modified > found[1].modified);
    }

    #[test]
    fn overlapping_roots_do_not_duplicate_results() {
        let dir = tempfile::tempdir().unwrap();
        touch(&dir.path().join("sub").join("v.kdbx"));
        let found = scan_for_vaults(
            &[dir.path().to_path_buf(), dir.path().join("sub")],
            &ScanLimits::default(),
            Instant::now,
        );
        assert_eq!(found.len(), 1);
    }

    #[test]
    fn missing_or_file_root_is_skipped_silently() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("f.kdbx");
        touch(&file);
        let found = scan_for_vaults(
            &[dir.path().join("nope"), file],
            &ScanLimits::default(),
            Instant::now,
        );
        assert!(found.is_empty());
    }

    // ---- guards ----

    struct Procs(Vec<PathBuf>);

    impl ProcessLookup for Procs {
        fn other_app_executables(&self, bundle_id: &str) -> Vec<PathBuf> {
            assert_eq!(bundle_id, APP_BUNDLE_ID);
            self.0.clone()
        }
    }

    fn icloud_file(home: &Path) -> PathBuf {
        home.join("Library")
            .join("Mobile Documents")
            .join("com~apple~CloudDocs")
            .join("v.kdbx")
    }

    fn warnings(path: &Path, home: &Path, procs: Vec<PathBuf>, exe: &Path) -> Vec<OpenWarning> {
        open_warnings(path, home, &Procs(procs), exe)
    }

    // The block marked STAGE5-REMOVE in `open_warnings` (and this test with it) goes in stage 5.
    #[test]
    fn icloud_path_warns_until_stage_5() {
        let home = tempfile::tempdir().unwrap();
        let exe = home.path().join("me");
        assert_eq!(
            warnings(&icloud_file(home.path()), home.path(), vec![], &exe),
            vec![OpenWarning::Icloud]
        );
        // The same location once it exists on disk.
        let real = icloud_file(home.path());
        touch(&real);
        assert_eq!(
            warnings(&real, home.path(), vec![], &exe),
            vec![OpenWarning::Icloud]
        );
    }

    #[test]
    fn non_icloud_path_has_no_warning() {
        let home = tempfile::tempdir().unwrap();
        let path = home.path().join("Documents").join("v.kdbx");
        assert!(warnings(&path, home.path(), vec![], &home.path().join("me")).is_empty());
        assert!(!is_icloud_path(Path::new(""), home.path()));
        assert!(!is_icloud_path(Path::new("."), Path::new("")));
    }

    #[test]
    fn icloud_lookalike_directory_name_does_not_warn() {
        let home = tempfile::tempdir().unwrap();
        let path = home
            .path()
            .join("Documents")
            .join("Mobile Documents backup")
            .join("v.kdbx");
        assert!(!is_icloud_path(&path, home.path()));
        let sibling = home
            .path()
            .join("Library")
            .join("Mobile Documents backup")
            .join("v.kdbx");
        touch(&sibling);
        assert!(!is_icloud_path(&sibling, home.path()));
    }

    #[test]
    fn dot_dot_cannot_hide_an_icloud_path() {
        let home = tempfile::tempdir().unwrap();
        let sneaky = home
            .path()
            .join("Documents")
            .join("..")
            .join("Library")
            .join("Mobile Documents")
            .join("v.kdbx");
        assert!(is_icloud_path(&sneaky, home.path()));
        let escaping = icloud_file(home.path())
            .join("..")
            .join("..")
            .join("..")
            .join("v.kdbx");
        assert!(!is_icloud_path(&escaping, home.path()));
    }

    #[cfg(unix)]
    #[test]
    fn symlink_into_icloud_warns_when_it_exists() {
        let home = tempfile::tempdir().unwrap();
        let real = icloud_file(home.path());
        touch(&real);
        let link = home.path().join("Documents").join("link.kdbx");
        std::fs::create_dir_all(link.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&real, &link).unwrap();
        assert!(is_icloud_path(&link, home.path()));
        // A file about to be created inside a linked directory resolves through its parent.
        let dir_link = home.path().join("linked-dir");
        std::os::unix::fs::symlink(real.parent().unwrap(), &dir_link).unwrap();
        assert!(is_icloud_path(&dir_link.join("new.kdbx"), home.path()));
    }

    #[test]
    fn old_app_running_with_other_executable_warns() {
        let home = tempfile::tempdir().unwrap();
        let old = PathBuf::from("/Applications/MyPass.app/Contents/MacOS/MyPass");
        let path = home.path().join("v.kdbx");
        assert_eq!(
            warnings(
                &path,
                home.path(),
                vec![old.clone()],
                &home.path().join("me")
            ),
            vec![OpenWarning::OldAppRunning {
                executable: old.to_string_lossy().into_owned()
            }]
        );
    }

    #[test]
    fn same_executable_does_not_warn() {
        let home = tempfile::tempdir().unwrap();
        let exe = home.path().join("me");
        touch(&exe);
        let path = home.path().join("v.kdbx");
        assert!(warnings(&path, home.path(), vec![exe.clone()], &exe).is_empty());
        // A different spelling of the same file counts as the same executable.
        let spelled = home.path().join("sub").join("..").join("me");
        std::fs::create_dir_all(home.path().join("sub")).unwrap();
        assert!(warnings(&path, home.path(), vec![spelled], &exe).is_empty());
    }

    #[test]
    fn no_other_process_no_warning() {
        let home = tempfile::tempdir().unwrap();
        let path = home.path().join("v.kdbx");
        assert!(warnings(&path, home.path(), vec![], &home.path().join("me")).is_empty());
    }

    #[test]
    fn both_warnings_are_reported_together() {
        let home = tempfile::tempdir().unwrap();
        let old = PathBuf::from("/Applications/MyPass.app/Contents/MacOS/MyPass");
        // Helper processes of the old app share one executable: reported once.
        let got = warnings(
            &icloud_file(home.path()),
            home.path(),
            vec![old.clone(), old.clone()],
            &home.path().join("me"),
        );
        assert_eq!(
            got,
            vec![
                OpenWarning::Icloud,
                OpenWarning::OldAppRunning {
                    executable: old.to_string_lossy().into_owned()
                }
            ]
        );
    }

    #[test]
    fn dtos_serialise_camel_case() {
        let v = serde_json::to_value(OpenWarning::OldAppRunning {
            executable: "x".into(),
        })
        .unwrap();
        assert_eq!(
            v,
            serde_json::json!({"kind": "oldAppRunning", "executable": "x"})
        );
        assert_eq!(
            serde_json::to_value(OpenWarning::Icloud).unwrap(),
            serde_json::json!({"kind": "icloud"})
        );
    }
}
