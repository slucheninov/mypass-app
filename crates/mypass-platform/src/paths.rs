//! Every on-disk location of the app (K18): the single source of truth.
//!
//! The macOS and Windows layouts are plain functions over injected roots without `cfg` gates, so
//! both are unit-tested on every runner. Only [`AppPaths::resolve`] reads the environment.

use crate::{PlatformError, PlatformResult};
use std::path::{Path, PathBuf};

/// The bundle identifier, also the macOS data folder name.
pub const BUNDLE_ID: &str = "com.mypass.app";
/// File name of the native messaging host manifest.
pub const HOST_MANIFEST_FILE: &str = "com.mypass.chromehost.json";

/// The Chromium-family browsers the native messaging host is registered with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Browser {
    Chrome,
    ChromeForTesting,
    ChromeBeta,
    ChromeCanary,
    ChromeDev,
    Chromium,
    Edge,
    EdgeBeta,
    EdgeDev,
    EdgeCanary,
    Brave,
    BraveBeta,
    BraveNightly,
    Vivaldi,
    VivaldiSnapshot,
    Opera,
    OperaGx,
    Arc,
}

impl Browser {
    /// The 18 Chromium-family targets of the old `scripts/dev.sh`, in declaration order.
    pub const ALL: [Browser; 18] = [
        Browser::Chrome,
        Browser::ChromeForTesting,
        Browser::ChromeBeta,
        Browser::ChromeCanary,
        Browser::ChromeDev,
        Browser::Chromium,
        Browser::Edge,
        Browser::EdgeBeta,
        Browser::EdgeDev,
        Browser::EdgeCanary,
        Browser::Brave,
        Browser::BraveBeta,
        Browser::BraveNightly,
        Browser::Vivaldi,
        Browser::VivaldiSnapshot,
        Browser::Opera,
        Browser::OperaGx,
        Browser::Arc,
    ];

    /// Stable snake_case id: the key in registration reports and the Browser pane.
    pub fn id(self) -> &'static str {
        match self {
            Browser::Chrome => "chrome",
            Browser::ChromeForTesting => "chrome_for_testing",
            Browser::ChromeBeta => "chrome_beta",
            Browser::ChromeCanary => "chrome_canary",
            Browser::ChromeDev => "chrome_dev",
            Browser::Chromium => "chromium",
            Browser::Edge => "edge",
            Browser::EdgeBeta => "edge_beta",
            Browser::EdgeDev => "edge_dev",
            Browser::EdgeCanary => "edge_canary",
            Browser::Brave => "brave",
            Browser::BraveBeta => "brave_beta",
            Browser::BraveNightly => "brave_nightly",
            Browser::Vivaldi => "vivaldi",
            Browser::VivaldiSnapshot => "vivaldi_snapshot",
            Browser::Opera => "opera",
            Browser::OperaGx => "opera_gx",
            Browser::Arc => "arc",
        }
    }

    /// Human-readable name.
    pub fn display_name(self) -> &'static str {
        match self {
            Browser::Chrome => "Google Chrome",
            Browser::ChromeForTesting => "Google Chrome for Testing",
            Browser::ChromeBeta => "Google Chrome Beta",
            Browser::ChromeCanary => "Google Chrome Canary",
            Browser::ChromeDev => "Google Chrome Dev",
            Browser::Chromium => "Chromium",
            Browser::Edge => "Microsoft Edge",
            Browser::EdgeBeta => "Microsoft Edge Beta",
            Browser::EdgeDev => "Microsoft Edge Dev",
            Browser::EdgeCanary => "Microsoft Edge Canary",
            Browser::Brave => "Brave",
            Browser::BraveBeta => "Brave Beta",
            Browser::BraveNightly => "Brave Nightly",
            Browser::Vivaldi => "Vivaldi",
            Browser::VivaldiSnapshot => "Vivaldi Snapshot",
            Browser::Opera => "Opera",
            Browser::OperaGx => "Opera GX",
            Browser::Arc => "Arc",
        }
    }

    /// macOS base folder below `<home>/Library/Application Support/`.
    pub fn vendor_dir(self) -> &'static str {
        match self {
            Browser::Chrome => "Google/Chrome",
            Browser::ChromeForTesting => "Google/Chrome for Testing",
            Browser::ChromeBeta => "Google/Chrome Beta",
            Browser::ChromeCanary => "Google/Chrome Canary",
            Browser::ChromeDev => "Google/Chrome Dev",
            Browser::Chromium => "Chromium",
            Browser::Edge => "Microsoft Edge",
            Browser::EdgeBeta => "Microsoft Edge Beta",
            Browser::EdgeDev => "Microsoft Edge Dev",
            Browser::EdgeCanary => "Microsoft Edge Canary",
            Browser::Brave => "BraveSoftware/Brave-Browser",
            Browser::BraveBeta => "BraveSoftware/Brave-Browser-Beta",
            Browser::BraveNightly => "BraveSoftware/Brave-Browser-Nightly",
            Browser::Vivaldi => "Vivaldi",
            Browser::VivaldiSnapshot => "Vivaldi Snapshot",
            Browser::Opera => "com.operasoftware.Opera",
            Browser::OperaGx => "com.operasoftware.OperaGX",
            Browser::Arc => "Arc/User Data",
        }
    }

    /// `true` for Chrome and Chrome for Testing only: their manifest folder is created even when
    /// the browser has never run; every other browser is registered only when its base folder
    /// already exists.
    pub fn always_register(self) -> bool {
        matches!(self, Browser::Chrome | Browser::ChromeForTesting)
    }
}

/// Which OS layout an [`AppPaths`] was built for (decides the manifest location).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Layout {
    MacOs,
    Windows,
}

/// Every on-disk location of the app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppPaths {
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
    pub backups_dir: PathBuf,
    pub host_dir: PathBuf,
    pub host_exe: PathBuf,
    pub secrets_shared: PathBuf,
    pub secrets_app: PathBuf,
    home: PathBuf,
    layout: Layout,
}

impl AppPaths {
    /// The macOS layout below `home` (the one constructor tests use over a temporary folder).
    /// `config_dir` and `data_dir` are both `<home>/Library/Application Support/com.mypass.app`.
    pub fn macos(home: &Path) -> Self {
        let data_dir = home
            .join("Library")
            .join("Application Support")
            .join(BUNDLE_ID);
        Self::with_data(
            data_dir.clone(),
            data_dir,
            home,
            "mypass-host",
            Layout::MacOs,
        )
    }

    /// The Windows layout: `config_dir` is `<APPDATA>/MyPass`, `data_dir` is
    /// `<LOCALAPPDATA>/MyPass`; the sub-folder names are the macOS ones.
    pub fn windows(app_data: &Path, local_app_data: &Path, home: &Path) -> Self {
        Self::with_data(
            app_data.join("MyPass"),
            local_app_data.join("MyPass"),
            home,
            "mypass-host.exe",
            Layout::Windows,
        )
    }

    fn with_data(
        config_dir: PathBuf,
        data_dir: PathBuf,
        home: &Path,
        host_file: &str,
        layout: Layout,
    ) -> Self {
        let host_dir = data_dir.join("host");
        let secrets = data_dir.join("secrets");
        Self {
            config_dir,
            backups_dir: data_dir.join("backups"),
            host_exe: host_dir.join(host_file),
            host_dir,
            secrets_shared: secrets.join("shared"),
            secrets_app: secrets.join("app"),
            data_dir,
            home: home.to_path_buf(),
            layout,
        }
    }

    /// The paths of the running user.
    ///
    /// macOS: the home folder from `std::env::home_dir` (on Unix it falls back to the user
    /// database when `HOME` is unset). Windows: `APPDATA` and `LOCALAPPDATA` from the environment
    /// (stage 6 may switch to the known-folder API) and the profile from `std::env::home_dir`.
    /// An unset, empty or relative value is an error, never a relative path. Other targets:
    /// `Unsupported`. The `MYPASS_WINDOWS_DATA_ROOT` dev override is not handled here.
    pub fn resolve() -> PlatformResult<Self> {
        #[cfg(target_os = "macos")]
        {
            let home = required_root("home folder", std::env::home_dir())?;
            Ok(Self::macos(&home))
        }
        #[cfg(windows)]
        {
            let app_data =
                required_root("APPDATA", std::env::var_os("APPDATA").map(PathBuf::from))?;
            let local = required_root(
                "LOCALAPPDATA",
                std::env::var_os("LOCALAPPDATA").map(PathBuf::from),
            )?;
            let home = required_root("user profile", std::env::home_dir())?;
            Ok(Self::windows(&app_data, &local, &home))
        }
        #[cfg(not(any(target_os = "macos", windows)))]
        {
            Err(PlatformError::Unsupported("app paths"))
        }
    }

    /// The user's home folder.
    pub fn home(&self) -> &Path {
        &self.home
    }

    /// Where `browser` looks for our native messaging host manifest. macOS: below the browser's
    /// vendor folder. Windows: one shared manifest in `host_dir` for every browser (the
    /// per-browser registry key that points at it is stage 6).
    pub fn manifest_path(&self, browser: Browser) -> PathBuf {
        match self.layout {
            Layout::MacOs => self
                .home
                .join("Library")
                .join("Application Support")
                .join(browser.vendor_dir())
                .join("NativeMessagingHosts")
                .join(HOST_MANIFEST_FILE),
            Layout::Windows => self.host_dir.join(HOST_MANIFEST_FILE),
        }
    }

    /// Backups of one vault: `backups_dir/<vault_id>`.
    pub fn vault_backups_dir(&self, vault_id: &VaultId) -> PathBuf {
        self.backups_dir.join(vault_id.as_str())
    }

    /// Drafts of one vault: `data_dir/drafts/<vault_id>`.
    pub fn drafts_dir(&self, vault_id: &VaultId) -> PathBuf {
        self.data_dir.join("drafts").join(vault_id.as_str())
    }
}

/// A vault id that is safe to use as one path segment: 1 to 64 characters of `[A-Za-z0-9_-]`.
///
/// Joining an unvalidated id onto a base path lets an absolute value replace the base and `..`
/// escape it, so the vault-scoped directory functions take this type instead of a bare `&str`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct VaultId(String);

impl VaultId {
    /// Validates `id`; anything that is not a single plain segment is rejected.
    pub fn new(id: &str) -> PlatformResult<Self> {
        let valid = (1..=64).contains(&id.len())
            && id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
        if valid {
            Ok(Self(id.to_owned()))
        } else {
            Err(PlatformError::Failed(
                "invalid vault id: expected 1 to 64 characters of [A-Za-z0-9_-]".to_owned(),
            ))
        }
    }

    /// The validated id.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A root folder read from the environment: unset, empty or relative is an error.
#[cfg(any(target_os = "macos", windows, test))]
fn required_root(name: &'static str, value: Option<PathBuf>) -> PlatformResult<PathBuf> {
    match value {
        Some(path) if path.is_absolute() => Ok(path),
        Some(path) if path.as_os_str().is_empty() => {
            Err(PlatformError::Failed(format!("{name} is empty")))
        }
        Some(_) => Err(PlatformError::Failed(format!(
            "{name} is not an absolute path"
        ))),
        None => Err(PlatformError::Failed(format!("{name} is not set"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn mac() -> AppPaths {
        AppPaths::macos(Path::new("/Users/x"))
    }

    fn win_roots() -> (PathBuf, PathBuf, PathBuf) {
        (
            PathBuf::from(r"C:\Users\x\AppData\Roaming"),
            PathBuf::from(r"C:\Users\x\AppData\Local"),
            PathBuf::from(r"C:\Users\x"),
        )
    }

    #[test]
    fn app_paths_macos_layout() {
        let p = mac();
        let data = PathBuf::from("/Users/x/Library/Application Support/com.mypass.app");
        assert_eq!(p.config_dir, data);
        assert_eq!(p.data_dir, data);
        assert_eq!(p.backups_dir, data.join("backups"));
        assert_eq!(p.host_dir, data.join("host"));
        assert_eq!(p.host_exe, data.join("host").join("mypass-host"));
        assert_eq!(p.secrets_app, data.join("secrets").join("app"));
        assert_eq!(p.secrets_shared, data.join("secrets").join("shared"));
        assert_eq!(p.home(), Path::new("/Users/x"));
    }

    #[test]
    fn app_paths_windows_layout() {
        let (roaming, local, home) = win_roots();
        let p = AppPaths::windows(&roaming, &local, &home);
        let data = local.join("MyPass");
        assert_eq!(p.config_dir, roaming.join("MyPass"));
        assert_eq!(p.data_dir, data);
        assert_eq!(p.backups_dir, data.join("backups"));
        assert_eq!(p.host_dir, data.join("host"));
        assert_eq!(p.host_exe, data.join("host").join("mypass-host.exe"));
        assert_eq!(p.secrets_app, data.join("secrets").join("app"));
        assert_eq!(p.secrets_shared, data.join("secrets").join("shared"));
        assert_eq!(p.home(), home.as_path());
    }

    #[test]
    fn app_paths_windows_handles_spaces_and_non_ascii_roots() {
        let roaming = PathBuf::from("C:\\Users\\J\u{f6}rg M\u{fc}ller\\AppData\\Roaming");
        let local = PathBuf::from("C:\\Users\\J\u{f6}rg M\u{fc}ller\\AppData\\Local");
        let home = PathBuf::from("C:\\Users\\J\u{f6}rg M\u{fc}ller");
        let p = AppPaths::windows(&roaming, &local, &home);
        assert_eq!(p.config_dir, roaming.join("MyPass"));
        assert_eq!(p.data_dir, local.join("MyPass"));
        assert!(p.host_exe.starts_with(&local));
        assert_eq!(p.home(), home.as_path());
    }

    #[test]
    fn manifest_path_per_browser_macos() {
        let p = mac();
        let chrome = p.manifest_path(Browser::Chrome);
        let brave = p.manifest_path(Browser::Brave);
        for m in [&chrome, &brave] {
            assert!(m.ends_with("NativeMessagingHosts/com.mypass.chromehost.json"));
        }
        assert_eq!(
            chrome,
            PathBuf::from("/Users/x/Library/Application Support/Google/Chrome/NativeMessagingHosts/com.mypass.chromehost.json")
        );
        assert_eq!(
            brave,
            PathBuf::from("/Users/x/Library/Application Support/BraveSoftware/Brave-Browser/NativeMessagingHosts/com.mypass.chromehost.json")
        );
        assert_ne!(chrome, brave);
    }

    #[test]
    fn manifest_path_windows_is_one_shared_file_under_host_dir() {
        let (roaming, local, home) = win_roots();
        let p = AppPaths::windows(&roaming, &local, &home);
        for browser in Browser::ALL {
            assert_eq!(
                p.manifest_path(browser),
                p.host_dir.join("com.mypass.chromehost.json")
            );
        }
    }

    #[test]
    fn browser_all_has_18_distinct_ids_and_exactly_two_always_register() {
        let ids: HashSet<&str> = Browser::ALL.iter().map(|b| b.id()).collect();
        assert_eq!(ids.len(), 18);
        let always: Vec<Browser> = Browser::ALL
            .into_iter()
            .filter(|b| b.always_register())
            .collect();
        assert_eq!(always, vec![Browser::Chrome, Browser::ChromeForTesting]);
        for b in Browser::ALL {
            assert!(b.id().chars().all(|c| c.is_ascii_lowercase() || c == '_'));
            assert!(!b.display_name().is_empty());
        }
        assert_eq!(Browser::Chrome.id(), "chrome");
        assert_eq!(Browser::ChromeForTesting.id(), "chrome_for_testing");
        assert_eq!(Browser::Edge.id(), "edge");
        assert_eq!(Browser::OperaGx.id(), "opera_gx");
        assert_eq!(Browser::Chrome.display_name(), "Google Chrome");
        assert_eq!(Browser::Edge.display_name(), "Microsoft Edge");
    }

    #[test]
    fn browser_vendor_dirs_match_the_old_dev_sh_table() {
        let expected = [
            "Google/Chrome",
            "Google/Chrome for Testing",
            "Google/Chrome Beta",
            "Google/Chrome Canary",
            "Google/Chrome Dev",
            "Chromium",
            "Microsoft Edge",
            "Microsoft Edge Beta",
            "Microsoft Edge Dev",
            "Microsoft Edge Canary",
            "BraveSoftware/Brave-Browser",
            "BraveSoftware/Brave-Browser-Beta",
            "BraveSoftware/Brave-Browser-Nightly",
            "Vivaldi",
            "Vivaldi Snapshot",
            "com.operasoftware.Opera",
            "com.operasoftware.OperaGX",
            "Arc/User Data",
        ];
        let actual: Vec<&str> = Browser::ALL.iter().map(|b| b.vendor_dir()).collect();
        assert_eq!(actual, expected);
    }

    #[test]
    fn manifest_path_opera_and_arc_macos() {
        let p = mac();
        assert_eq!(
            p.manifest_path(Browser::Opera),
            PathBuf::from("/Users/x/Library/Application Support/com.operasoftware.Opera/NativeMessagingHosts/com.mypass.chromehost.json")
        );
        assert_eq!(
            p.manifest_path(Browser::Arc),
            PathBuf::from("/Users/x/Library/Application Support/Arc/User Data/NativeMessagingHosts/com.mypass.chromehost.json")
        );
    }

    #[test]
    fn vault_scoped_dirs_end_with_vault_id() {
        let p = mac();
        let v1 = VaultId::new("v1").unwrap();
        let abc = VaultId::new("abc").unwrap();
        assert_eq!(p.vault_backups_dir(&v1), p.backups_dir.join("v1"));
        assert_eq!(p.drafts_dir(&v1), p.data_dir.join("drafts").join("v1"));
        assert!(p.vault_backups_dir(&abc).ends_with("abc"));
        assert!(p.drafts_dir(&abc).ends_with("abc"));
    }

    #[test]
    fn vault_id_rejects_anything_but_one_plain_segment() {
        let too_long = "a".repeat(65);
        for bad in [
            "/etc/passwd",
            "..",
            ".",
            "a/b",
            "a\\b",
            "",
            too_long.as_str(),
            "a b",
            "caf\u{e9}",
        ] {
            assert!(VaultId::new(bad).is_err(), "{bad:?}");
        }
        assert!(VaultId::new(&"a".repeat(64)).is_ok());
        assert!(VaultId::new("0123456789abcdef0123456789abcdef").is_ok());
        assert!(VaultId::new("550e8400-e29b-41d4-a716-446655440000").is_ok());
        assert!(VaultId::new("a_b-C").is_ok());
    }

    #[test]
    fn all_paths_are_absolute_and_distinct() {
        // A root that is absolute on the running OS (`/Users/x` is not absolute on Windows).
        let p = AppPaths::macos(&std::env::temp_dir().join("home"));
        let fields = [
            &p.backups_dir,
            &p.host_dir,
            &p.host_exe,
            &p.secrets_shared,
            &p.secrets_app,
            &p.data_dir,
        ];
        for f in fields {
            assert!(f.is_absolute(), "{}", f.display());
        }
        let distinct: HashSet<&PathBuf> = fields.into_iter().collect();
        assert_eq!(distinct.len(), 6);
        // On Windows config and data are distinct roots.
        let (roaming, local, home) = win_roots();
        let w = AppPaths::windows(&roaming, &local, &home);
        assert_ne!(w.config_dir, w.data_dir);
    }

    #[test]
    fn required_root_rejects_unset_empty_and_relative_values() {
        assert!(required_root("HOME", None).is_err());
        assert!(required_root("HOME", Some(PathBuf::new())).is_err());
        assert!(required_root("HOME", Some(PathBuf::from("relative/dir"))).is_err());
        let absolute = std::env::temp_dir();
        assert_eq!(required_root("HOME", Some(absolute.clone())), Ok(absolute));
    }
}
