//! Real native APIs stay opt-in and use only temporary files/generated HKCU roots.
#![cfg(windows)]
use mypass_platform::{
    windows::{HostFiles, WinRegistry},
    windows_host_install::*,
    windows_registry::*,
    *,
};
use std::{path::Path, sync::Arc};
fn opt_in() {
    assert_eq!(
        std::env::var("MYPASS_WINDOWS_INTEGRATION_TESTS").as_deref(),
        Ok("1")
    );
}

#[test]
#[ignore = "real registry; isolated native runner only"]
fn windows_integration_registry_round_trip_missing_unicode_and_owned_cleanup() {
    opt_in();
    let r = WinRegistry::isolated();
    for view in [View::Registry32, View::Registry64] {
        let key = Key::new(
            Hive::CurrentUser,
            view,
            "Software\\Google\\Chrome\\NativeMessagingHosts\\com.mypass.chromehost",
        )
        .unwrap();
        assert_eq!(r.read(&key, "", ValueType::String).unwrap(), Read::Missing);
        r.create(&key).unwrap();
        let owner = Value::String("C:\\\u{41a}\u{43e}\u{440}\u{438}\u{441}\u{442}\u{443}\u{432}\u{430}\u{447} \u{65e5}\u{672c}\u{8a9e}\\host.json".into());
        r.set(&key, "", owner.clone()).unwrap();
        assert_eq!(
            r.read(&key, "", ValueType::String).unwrap(),
            Read::Value(owner.clone())
        );
        assert_eq!(
            r.read(&key, "", ValueType::Binary).unwrap(),
            Read::WrongType
        );
        r.set(&key, "foreign", Value::Binary(vec![0, 1, 255]))
            .unwrap();
        let child = Key::new(Hive::CurrentUser, view, &format!("{}\\child", key.subkey())).unwrap();
        r.create(&child).unwrap();
        assert!(!r
            .delete_owned_key(&key, "", &Value::String("elsewhere".into()))
            .unwrap());
        assert!(r.delete_owned_key(&key, "", &owner).unwrap());
        assert_eq!(
            r.read(&key, "foreign", ValueType::Binary).unwrap(),
            Read::Value(Value::Binary(vec![0, 1, 255]))
        );
        assert!(r.key_exists(&child).unwrap());
        r.delete_value(&key, "absent").unwrap();
    }
    let hklm = Key::new(
        Hive::LocalMachine,
        View::Registry64,
        "Software\\MyPassNoWrites",
    )
    .unwrap();
    assert!(WinRegistry::new().create(&hklm).is_err());
}

#[test]
#[ignore = "controlled helper child; isolated native runner only"]
fn windows_integration_host_lifetime_helper() {
    opt_in();
    if std::env::var_os("MYPASS_HOST_LIFETIME_HELPER").is_none() {
        return;
    }
    use std::io::{Read, Write};
    println!("host-helper-ready");
    std::io::stdout().flush().unwrap();
    let _ = std::io::stdin().read(&mut [0u8; 1]);
}
struct Child(std::process::Child);
impl Drop for Child {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
#[ignore = "real running image/filesystem/registry; isolated native runner only"]
fn windows_integration_running_host_moves_aside_then_cleans_after_exit() {
    opt_in();
    use std::{
        io::{BufRead, BufReader},
        process::{Command, Stdio},
        time::Duration,
    };
    let tmp = tempfile::tempdir().unwrap();
    let paths = AppPaths::windows_data_root(
        &tmp.path()
            .join("\u{41a}\u{43e}\u{440}\u{438}\u{441}\u{442}\u{443}\u{432}\u{430}\u{447}"),
    );
    let r = WindowsHostRegistrar::new(paths.clone(), Arc::new(WinRegistry::isolated()));
    let helper = std::env::current_exe().unwrap();
    r.register(&helper).unwrap();
    let modified = std::fs::metadata(&paths.host_exe)
        .unwrap()
        .modified()
        .unwrap();
    assert!(!r.register(&helper).unwrap().host_copied);
    assert_eq!(
        std::fs::metadata(&paths.host_exe)
            .unwrap()
            .modified()
            .unwrap(),
        modified
    );
    let mut child = Child(
        Command::new(&paths.host_exe)
            .args([
                "--ignored",
                "--exact",
                "windows_integration_host_lifetime_helper",
                "--nocapture",
            ])
            .env("MYPASS_HOST_LIFETIME_HELPER", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let stdout = child.0.stdout.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let reader = std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            if line.unwrap().contains("host-helper-ready") {
                let _ = tx.send(());
                break;
            }
        }
    });
    rx.recv_timeout(Duration::from_secs(10)).unwrap();
    reader.join().unwrap();
    let next = tmp.path().join("next.exe");
    std::fs::write(&next, b"different executable fixture").unwrap();
    let dir = HostFiles
        .directory(&paths.host_dir, false)
        .unwrap()
        .unwrap();
    assert_eq!(
        replace_host(dir.as_ref(), b"different executable fixture").unwrap(),
        ReplaceOutcome::SwappedPastRunningImage
    );
    assert_eq!(
        std::fs::read(&paths.host_exe).unwrap(),
        b"different executable fixture"
    );
    assert!(child.0.try_wait().unwrap().is_none());
    assert_eq!(
        dir.names()
            .unwrap()
            .iter()
            .filter(|n| n.starts_with("mypass-host.old-"))
            .count(),
        1
    );
    child.0.kill().unwrap();
    child.0.wait().unwrap();
    r.register(&next).unwrap();
    assert_eq!(
        dir.names()
            .unwrap()
            .iter()
            .filter(|n| n.starts_with("mypass-host.old-"))
            .count(),
        0
    );
    r.unregister().unwrap();
    assert!(!paths.host_exe.exists());
}

#[test]
#[ignore = "real filesystem junction refusal; isolated native runner only"]
fn windows_integration_registrar_refuses_junction_and_nonregular_source() {
    opt_in();
    let tmp = tempfile::tempdir().unwrap();
    let paths = AppPaths::windows_data_root(&tmp.path().join("data"));
    std::fs::create_dir_all(&paths.data_dir).unwrap();
    let target = tmp.path().join("target");
    std::fs::create_dir(&target).unwrap();
    let status = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(&paths.host_dir)
        .arg(&target)
        .output()
        .unwrap();
    assert!(status.status.success());
    let r = WindowsHostRegistrar::new(paths.clone(), Arc::new(WinRegistry::isolated()));
    assert!(r.register(&std::env::current_exe().unwrap()).is_err());
    assert!(r.register(tmp.path()).is_err());
    assert!(r.register(Path::new("missing.exe")).is_err());
    assert!(std::fs::read_dir(&target).unwrap().next().is_none());
    std::fs::remove_dir(&paths.host_dir).unwrap();
}

#[test]
#[ignore = "real long-path host filesystem/registry; isolated native runner only"]
fn windows_integration_host_internal_temp_and_backup_names_allow_long_paths() {
    opt_in();
    let tmp = tempfile::tempdir().unwrap();
    // The browser-visible executable stays within 240 UTF-16 units. Internal old/temp names
    // exceed MAX_PATH and must use extended native paths, regardless of process manifest flags.
    let paths = AppPaths::windows_data_root(&tmp.path().join("a".repeat(140)));
    assert!(
        paths
            .host_exe
            .as_os_str()
            .to_str()
            .unwrap()
            .encode_utf16()
            .count()
            <= 240
    );
    let r = WindowsHostRegistrar::new(paths.clone(), Arc::new(WinRegistry::isolated()));
    r.register(&std::env::current_exe().unwrap()).unwrap();
    r.unregister().unwrap();
    assert!(!paths.host_exe.exists());
}
