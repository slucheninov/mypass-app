//! Helpers for the repository guard tests under `tests/`. This crate is test-only (`publish = false`)
//! and is never shipped; it exists so that conventions (pinned dependencies, CI shape, docs rules,
//! script flags) fail a build instead of rotting in prose.
//!
//! test-only support code: the lints that forbid panicking shortcuts in library code are relaxed
//! here because a guard helper that cannot read a file must fail the test loudly.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

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
