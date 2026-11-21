//! Locate the desktop app from the installed native messaging host.

use std::ffi::OsString;
use std::fs::{self, File, Metadata};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::{Deserialize, Serialize};

use crate::{AppPaths, PlatformError, PlatformResult};

pub const APP_LOCATION_MAX_BYTES: u64 = 4096;
#[cfg(windows)]
use crate::paths::APP_LOCATION_FILE;
const AUTOSTARTED: &str = "--autostarted";
#[cfg(target_os = "macos")]
const MAC_BUNDLE_IDENTIFIER: &str = "com.mypass.app";
#[cfg(target_os = "macos")]
const INFO_PLIST_MAX_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppTarget {
    MacBundle(PathBuf),
    Executable(PathBuf),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchCommand {
    pub program: PathBuf,
    pub args: Vec<OsString>,
}

#[derive(Serialize, Deserialize)]
struct Location {
    version: u32,
    path: PathBuf,
}

pub fn bundle_containing(exe: &Path) -> Option<PathBuf> {
    let macos = exe.parent()?;
    let contents = macos.parent()?;
    let bundle = contents.parent()?;
    (bundle.is_absolute()
        && macos.file_name()? == "MacOS"
        && contents.file_name()? == "Contents"
        && bundle.extension()? == "app")
        .then(|| bundle.to_path_buf())
}

pub fn current_app_target() -> Option<AppTarget> {
    let exe = std::env::current_exe().ok()?;
    #[cfg(target_os = "macos")]
    if let Some(bundle) = bundle_containing(&exe).and_then(validate_target) {
        return Some(bundle);
    }
    validate_target(exe)
}

fn plain_metadata(path: &Path) -> Option<Metadata> {
    let metadata = fs::symlink_metadata(path).ok()?;
    if metadata.file_type().is_symlink() {
        return None;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        use windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0 {
            return None;
        }
    }
    Some(metadata)
}

#[cfg(target_os = "macos")]
fn mac_bundle_identifier(bundle: &Path) -> Option<String> {
    #[derive(Deserialize)]
    struct Info {
        #[serde(rename = "CFBundleIdentifier")]
        identifier: String,
    }

    let path = bundle.join("Contents/Info.plist");
    let metadata = plain_metadata(&path)?;
    if !metadata.is_file() || metadata.len() > INFO_PLIST_MAX_BYTES {
        return None;
    }
    let mut bytes = Vec::new();
    File::open(path)
        .ok()?
        .take(INFO_PLIST_MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > INFO_PLIST_MAX_BYTES {
        return None;
    }
    plist::from_bytes::<Info>(&bytes)
        .ok()
        .map(|info| info.identifier)
}

fn validate_target(path: PathBuf) -> Option<AppTarget> {
    if !path.is_absolute() {
        return None;
    }
    let metadata = plain_metadata(&path)?;
    #[cfg(target_os = "macos")]
    if path.extension().is_some_and(|ext| ext == "app")
        && metadata.is_dir()
        && mac_bundle_identifier(&path).as_deref() == Some(MAC_BUNDLE_IDENTIFIER)
    {
        return Some(AppTarget::MacBundle(path));
    }
    let name = path.file_name()?.to_str()?;
    #[cfg(windows)]
    let valid_name = ["MyPass.exe", "mypass-app.exe"]
        .iter()
        .any(|candidate| name.eq_ignore_ascii_case(candidate));
    #[cfg(not(windows))]
    let valid_name = matches!(name, "MyPass" | "mypass-app");
    (metadata.is_file() && valid_name).then_some(AppTarget::Executable(path))
}

pub fn write_app_location(paths: &AppPaths, target: &AppTarget) -> PlatformResult<()> {
    let path = match target {
        AppTarget::MacBundle(path) | AppTarget::Executable(path) => path.clone(),
    };
    let bytes = serde_json::to_vec(&Location { version: 1, path }).map_err(failed)?;
    if bytes.len() as u64 > APP_LOCATION_MAX_BYTES {
        return Err(failed("app location record is too large"));
    }
    #[cfg(windows)]
    {
        use crate::windows_host_install::{publish, InstallFiles};
        let directory = crate::windows::HostFiles
            .directory(&paths.host_dir, true)?
            .ok_or_else(|| failed("host directory is unavailable"))?;
        publish(directory.as_ref(), APP_LOCATION_FILE, &bytes)
    }
    #[cfg(unix)]
    {
        crate::macos_host_install::write_app_location_record(paths, &bytes)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = paths;
        Err(PlatformError::Unsupported("app location"))
    }
}

pub fn read_app_location(paths: &AppPaths) -> Option<AppTarget> {
    if !plain_metadata(&paths.host_dir)?.is_dir() {
        return None;
    }
    let path = paths.app_location_file();
    let metadata = plain_metadata(&path)?;
    if !metadata.is_file() || metadata.len() > APP_LOCATION_MAX_BYTES {
        return None;
    }
    let mut bytes = Vec::new();
    File::open(path)
        .ok()?
        .take(APP_LOCATION_MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > APP_LOCATION_MAX_BYTES {
        return None;
    }
    let record: Location = serde_json::from_slice(&bytes).ok()?;
    if record.version != 1 {
        return None;
    }
    validate_target(record.path)
}

pub fn resolve_app(host_exe: &Path, paths: &AppPaths) -> Option<AppTarget> {
    #[cfg(target_os = "macos")]
    if let Some(bundle) = bundle_containing(host_exe).and_then(validate_target) {
        return Some(bundle);
    }
    #[cfg(not(target_os = "macos"))]
    let _ = host_exe;
    read_app_location(paths)
}

pub fn launch_command(target: &AppTarget) -> LaunchCommand {
    match target {
        AppTarget::MacBundle(bundle) => LaunchCommand {
            program: "/usr/bin/open".into(),
            args: [
                "-g".into(),
                "-j".into(),
                bundle.as_os_str().into(),
                "--args".into(),
                AUTOSTARTED.into(),
            ]
            .into(),
        },
        AppTarget::Executable(exe) => LaunchCommand {
            program: exe.clone(),
            args: vec![AUTOSTARTED.into()],
        },
    }
}

pub fn launch_hidden(target: &AppTarget) -> PlatformResult<()> {
    let launch = launch_command(target);
    let mut command = Command::new(launch.program);
    command
        .args(launch.args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    #[cfg(windows)]
    {
        use std::io::ErrorKind;
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x8;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x200;
        const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
        let detached = DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP;
        command.creation_flags(detached | CREATE_BREAKAWAY_FROM_JOB);
        match command.spawn() {
            Err(error) if error.kind() == ErrorKind::PermissionDenied => command
                .creation_flags(detached)
                .spawn()
                .map(drop)
                .map_err(failed),
            result => result.map(drop).map_err(failed),
        }
    }
    #[cfg(not(windows))]
    {
        if matches!(target, AppTarget::MacBundle(_)) {
            if command.status().map_err(failed)?.success() {
                return Ok(());
            }
            return Err(failed("opening the MyPass bundle failed"));
        }
        command.spawn().map(drop).map_err(failed)
    }
}

fn failed(error: impl std::fmt::Display) -> PlatformError {
    PlatformError::Failed(error.to_string())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)] // test-only
mod tests {
    use super::*;

    fn paths(dir: &tempfile::TempDir) -> AppPaths {
        #[cfg(windows)]
        return AppPaths::windows_data_root(dir.path());
        #[cfg(not(windows))]
        AppPaths::macos(dir.path())
    }

    fn executable(root: &Path) -> PathBuf {
        let name = if cfg!(windows) {
            "MyPass.exe"
        } else {
            "mypass-app"
        };
        let path = root.join(name);
        std::fs::write(&path, []).unwrap();
        path
    }

    fn raw_record(paths: &AppPaths, record: &[u8]) {
        std::fs::create_dir_all(&paths.host_dir).unwrap();
        std::fs::write(paths.app_location_file(), record).unwrap();
    }

    #[test]
    fn bundle_containing_walks_macos_contents_app() {
        let root = std::env::temp_dir();
        let bundle = root.join("MyPass.app");
        assert_eq!(
            bundle_containing(&bundle.join("Contents/MacOS/mypass-native-host")),
            Some(bundle)
        );
        assert_eq!(bundle_containing(&root.join("bin/host")), None);
    }

    #[test]
    fn launch_command_for_a_bundle_is_open_background_hidden() {
        let bundle = PathBuf::from("/x/MyPass.app");
        let command = launch_command(&AppTarget::MacBundle(bundle.clone()));
        assert_eq!(command.program, Path::new("/usr/bin/open"));
        assert_eq!(
            command.args,
            [
                OsString::from("-g"),
                OsString::from("-j"),
                bundle.into_os_string(),
                OsString::from("--args"),
                OsString::from("--autostarted"),
            ]
        );
    }

    #[test]
    fn launch_command_for_an_executable_passes_autostarted() {
        let path = PathBuf::from("MyPass.exe");
        let command = launch_command(&AppTarget::Executable(path.clone()));
        assert_eq!(command.program, path);
        assert_eq!(command.args, [OsString::from("--autostarted")]);
    }

    #[test]
    fn app_location_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths(&dir);
        let first = AppTarget::Executable(executable(dir.path()));
        write_app_location(&paths, &first).unwrap();
        assert_eq!(read_app_location(&paths), Some(first));
        let other = dir.path().join("other");
        std::fs::create_dir(&other).unwrap();
        let second = AppTarget::Executable(executable(&other));
        write_app_location(&paths, &second).unwrap();
        assert_eq!(read_app_location(&paths), Some(second));
    }

    #[cfg(unix)]
    #[test]
    fn app_location_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let paths = paths(&dir);
        write_app_location(&paths, &AppTarget::Executable(executable(dir.path()))).unwrap();
        let mode = paths
            .app_location_file()
            .metadata()
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn read_app_location_refuses_symlinks_relative_paths_and_foreign_names() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths(&dir);
        let valid = executable(dir.path());
        let foreign = dir.path().join("foreign.exe");
        std::fs::write(&foreign, []).unwrap();
        assert_eq!(read_app_location(&paths), None);
        for (version, path) in [
            (1, Path::new("mypass-app")),
            (1, foreign.as_path()),
            (2, valid.as_path()),
        ] {
            raw_record(
                &paths,
                &serde_json::to_vec(&serde_json::json!({"version": version, "path": path}))
                    .unwrap(),
            );
            assert_eq!(read_app_location(&paths), None);
        }
        raw_record(&paths, &vec![b' '; APP_LOCATION_MAX_BYTES as usize + 1]);
        assert_eq!(read_app_location(&paths), None);
        #[cfg(unix)]
        {
            let nested = dir.path().join("nested");
            std::fs::create_dir(&nested).unwrap();
            let link = nested.join(valid.file_name().unwrap());
            std::os::unix::fs::symlink(&valid, &link).unwrap();
            raw_record(
                &paths,
                &serde_json::to_vec(&serde_json::json!({"version": 1, "path": link})).unwrap(),
            );
            assert_eq!(read_app_location(&paths), None);
            let record = paths.app_location_file();
            std::fs::remove_file(&record).unwrap();
            let real_record = dir.path().join("record.json");
            std::fs::write(
                &real_record,
                serde_json::to_vec(&serde_json::json!({"version": 1, "path": valid})).unwrap(),
            )
            .unwrap();
            std::os::unix::fs::symlink(real_record, record).unwrap();
            assert_eq!(read_app_location(&paths), None);
        }
    }

    #[cfg(target_os = "macos")]
    fn bundle_plist(identifier: &str) -> String {
        format!(
            "<plist version=\"1.0\"><dict><key>CFBundleIdentifier</key><string>{identifier}</string></dict></plist>"
        )
    }

    #[cfg(target_os = "macos")]
    fn app_identifier() -> &'static str {
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../../../src-tauri/tauri.conf.json")).unwrap();
        assert_eq!(config["identifier"].as_str(), Some(MAC_BUNDLE_IDENTIFIER));
        MAC_BUNDLE_IDENTIFIER
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn read_app_location_requires_the_mypass_bundle_identifier() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths(&dir);
        let bundle = dir.path().join("MyPass.app");
        std::fs::create_dir_all(bundle.join("Contents")).unwrap();
        raw_record(
            &paths,
            &serde_json::to_vec(&serde_json::json!({"version": 1, "path": &bundle})).unwrap(),
        );
        assert_eq!(read_app_location(&paths), None);
        let info = bundle.join("Contents/Info.plist");
        std::fs::write(&info, []).unwrap();
        assert_eq!(read_app_location(&paths), None);
        std::fs::write(&info, bundle_plist("com.example.other")).unwrap();
        assert_eq!(read_app_location(&paths), None);
        std::fs::write(&info, bundle_plist(app_identifier())).unwrap();
        let expected = Some(AppTarget::MacBundle(bundle));
        assert_eq!(read_app_location(&paths), expected);

        let metadata = plist::Dictionary::from_iter([(
            "CFBundleIdentifier".to_owned(),
            plist::Value::String(app_identifier().to_owned()),
        )]);
        plist::to_file_binary(&info, &metadata).unwrap();
        assert_eq!(read_app_location(&paths), expected);
    }

    #[cfg(windows)]
    #[test]
    fn read_app_location_matches_exe_names_case_insensitively() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths(&dir);
        let exe = dir.path().join("mypass.EXE");
        std::fs::write(&exe, []).unwrap();
        raw_record(
            &paths,
            &serde_json::to_vec(&serde_json::json!({"version": 1, "path": &exe})).unwrap(),
        );
        assert_eq!(read_app_location(&paths), Some(AppTarget::Executable(exe)));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn resolve_app_prefers_the_containing_bundle_over_the_record() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths(&dir);
        write_app_location(&paths, &AppTarget::Executable(executable(dir.path()))).unwrap();
        let bundle = dir.path().join("MyPass.app");
        let host = bundle.join("Contents/MacOS/mypass-native-host");
        std::fs::create_dir_all(host.parent().unwrap()).unwrap();
        std::fs::write(
            bundle.join("Contents/Info.plist"),
            bundle_plist(app_identifier()),
        )
        .unwrap();
        assert_eq!(
            resolve_app(&host, &paths),
            Some(AppTarget::MacBundle(bundle))
        );
    }
}
