//! Helpers for the repository guard tests under `tests/`. This crate is test-only (`publish = false`)
//! and is never shipped; it exists so that conventions (pinned dependencies, CI shape, docs rules,
//! script flags) fail a build instead of rotting in prose.
//!
//! test-only support code: the lints that forbid panicking shortcuts in library code are relaxed
//! here because a guard helper that cannot read a file must fail the test loudly.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The repository root: two levels above this crate's manifest directory.
pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

/// Read a repository file (path relative to the root, `/`-separated) as UTF-8 text.
///
/// Panics with the path in the message when the file is missing or unreadable: a guard that cannot
/// see the file it guards must fail, not pass.
pub fn read(rel: &str) -> String {
    let path = repo_root().join(rel);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {rel} ({}): {e}", path.display()))
}

/// Files tracked by git (`git ls-files`), relative to the root with `/` separators on every OS.
///
/// Returns `None` when git is missing or the tree is not a git checkout (for example a source
/// tarball); a guard that needs the list then prints "skipped" and passes.
pub fn tracked_files() -> Option<Vec<PathBuf>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_root())
        .args(["ls-files", "-z"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    Some(
        text.split('\0')
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .collect(),
    )
}

/// `/`-separated string form of a relative path, for comparisons that must not depend on the OS.
pub fn rel_str(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// Relative paths (`.github/workflows/<name>.yml`) of every workflow file, sorted.
pub fn workflow_files() -> Vec<String> {
    let dir = repo_root().join(".github").join("workflows");
    let mut files: Vec<String> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("cannot list {}: {e}", dir.display()))
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".yml") || name.ends_with(".yaml"))
        .map(|name| format!(".github/workflows/{name}"))
        .collect();
    files.sort();
    files
}

/// A `bash` command for running repository scripts (`scripts/*.sh`) and workflow `run:` text.
///
/// On Windows a bare `Command::new("bash")` goes through the `CreateProcess` search order, which
/// looks in the Windows system directory BEFORE `PATH`. There it finds `C:\Windows\System32\bash.exe`,
/// the WSL launcher, which exits non-zero with empty output when no distro is installed (as on
/// `windows-latest`). The repository expects Git Bash ("`scripts/*.sh` from Git Bash"), so on
/// Windows this scans `PATH` itself and takes the first `bash.exe` outside the Windows directory.
/// It falls back to plain `bash` when there is none, so a missing bash still reports `NotFound`.
/// On every other OS it is plain `bash`.
pub fn bash_command() -> Command {
    #[cfg(windows)]
    {
        let windir = std::env::var_os("SystemRoot")
            .or_else(|| std::env::var_os("WINDIR"))
            .unwrap_or_else(|| "C:\\Windows".into());
        let path = std::env::var_os("PATH").unwrap_or_default();
        if let Some(found) = find_bash_outside(&path, Path::new(&windir), "bash.exe", Path::is_file)
        {
            return Command::new(found);
        }
    }
    Command::new("bash")
}

/// First `<dir>/<exe>` over the `path_var` entries whose directory is not under `system_dir`
/// (compared case-insensitively, as Windows paths are) and for which `exists` holds.
/// Platform-independent so it can be unit tested anywhere; only [`bash_command`] calls it, on
/// Windows.
#[cfg_attr(not(windows), allow(dead_code))]
fn find_bash_outside(
    path_var: &OsStr,
    system_dir: &Path,
    exe: &str,
    exists: impl Fn(&Path) -> bool,
) -> Option<PathBuf> {
    let system = system_dir.to_string_lossy().to_lowercase();
    let system = system.trim_end_matches(['\\', '/']);
    std::env::split_paths(path_var)
        .filter(|dir| {
            let dir = dir.to_string_lossy().to_lowercase();
            let dir = dir.trim_end_matches(['\\', '/']);
            dir != system
                && !dir.starts_with(&format!("{system}\\"))
                && !dir.starts_with(&format!("{system}/"))
        })
        .map(|dir| dir.join(exe))
        .find(|candidate| exists(candidate))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bash_search_skips_the_windows_directory() {
        // Drive-less paths: `join_paths` rejects the `:` of `C:` on Unix, where `:` separates entries.
        let dirs = ["/WINDOWS/System32", "/Program Files/Git/bin", "/other"];
        let path = std::env::join_paths(dirs).unwrap();
        let found = find_bash_outside(&path, Path::new("/windows/"), "bash.exe", |_| true);
        assert_eq!(
            found,
            Some(PathBuf::from("/Program Files/Git/bin").join("bash.exe"))
        );
        let only_system = std::env::join_paths(["/Windows/System32", "/Windows"]).unwrap();
        let none = find_bash_outside(&only_system, Path::new("/Windows"), "bash.exe", |_| true);
        assert_eq!(none, None);
    }
}
