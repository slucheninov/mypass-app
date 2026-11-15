// Test-only support code: fixture construction must fail loudly.
#![allow(clippy::unwrap_used)]
use mypass_platform::{
    windows_launch_at_login::WinLaunchAtLogin,
    windows_registry::{Hive, Key, MemoryRegistry, Read, Registry, Value, ValueType, View},
    LaunchAtLogin, LoginItemStatus,
};
use std::{path::PathBuf, sync::Arc};
fn key(path: &str) -> Key {
    Key::new(Hive::CurrentUser, View::Registry64, path).unwrap()
}
fn run() -> Key {
    key(r"Software\Microsoft\Windows\CurrentVersion\Run")
}
fn approved() -> Key {
    key(r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run")
}
fn launcher(registry: Arc<MemoryRegistry>) -> WinLaunchAtLogin {
    WinLaunchAtLogin::new(
        registry,
        PathBuf::from("C:\\Program Files\\\u{41c}\u{456}\\MyPass.exe"),
    )
}
#[test]
fn win_launch_at_login_writes_quoted_exe_and_autostarted_flag() {
    let r = Arc::new(MemoryRegistry::default());
    let l = launcher(r.clone());
    l.set_enabled(true).unwrap();
    assert_eq!(
        r.read(&run(), "MyPass", ValueType::String).unwrap(),
        Read::Value(Value::String(
            "\"C:\\Program Files\\\u{41c}\u{456}\\MyPass.exe\" --autostarted".into()
        ))
    );
    assert_eq!(l.status().unwrap(), LoginItemStatus::Enabled);
    l.set_enabled(false).unwrap();
    l.set_enabled(false).unwrap();
    assert_eq!(l.status().unwrap(), LoginItemStatus::Disabled);
}
#[test]
fn autostart_entry_follows_exe_path() {
    let r = Arc::new(MemoryRegistry::default());
    let l = launcher(r.clone());
    l.prepare_startup(false).unwrap();
    assert_eq!(l.status().unwrap(), LoginItemStatus::Disabled);
    r.create(&run()).unwrap();
    r.set(
        &run(),
        "MyPass",
        Value::String(r#""C:\old\MyPass.exe" --autostarted"#.into()),
    )
    .unwrap();
    l.prepare_startup(false).unwrap();
    assert_eq!(
        r.read(&run(), "MyPass", ValueType::String).unwrap(),
        Read::Value(Value::String(r#""C:\old\MyPass.exe" --autostarted"#.into()))
    );
    l.prepare_startup(true).unwrap();
    assert_eq!(l.status().unwrap(), LoginItemStatus::Enabled);
    assert_ne!(
        r.read(&run(), "MyPass", ValueType::String).unwrap(),
        Read::Value(Value::String(r#""C:\old\MyPass.exe" --autostarted"#.into()))
    );
}
#[test]
fn win_launch_at_login_preserves_every_present_startup_approved_record() {
    for value in [
        Value::Binary(vec![2, 0, 0, 0]),
        Value::Binary(vec![3, 0, 0, 0]),
        Value::Binary(vec![]),
        Value::String("unknown".into()),
    ] {
        let r = Arc::new(MemoryRegistry::default());
        let l = launcher(r.clone());
        l.set_enabled(true).unwrap();
        r.create(&approved()).unwrap();
        r.set(&approved(), "MyPass", value.clone()).unwrap();
        assert_eq!(l.status().unwrap(), LoginItemStatus::RequiresApproval);
        l.prepare_startup(true).unwrap();
        l.set_enabled(true).unwrap();
        assert_eq!(
            r.read(
                &approved(),
                "MyPass",
                if matches!(value, Value::String(_)) {
                    ValueType::String
                } else {
                    ValueType::Binary
                }
            )
            .unwrap(),
            Read::Value(value)
        );
        assert_eq!(l.status().unwrap(), LoginItemStatus::RequiresApproval);
    }
}
#[test]
fn win_launch_at_login_rejects_relative_exe_and_preserves_unowned_run_value() {
    let r = Arc::new(MemoryRegistry::default());
    assert!(WinLaunchAtLogin::new(r.clone(), "relative.exe".into())
        .set_enabled(true)
        .is_err());
    r.create(&run()).unwrap();
    r.set(&run(), "MyPass", Value::String("other command".into()))
        .unwrap();
    launcher(r.clone()).prepare_startup(true).unwrap();
    assert_eq!(
        r.read(&run(), "MyPass", ValueType::String).unwrap(),
        Read::Value(Value::String("other command".into()))
    );
}

struct FailingRegistry;
impl Registry for FailingRegistry {
    fn key_exists(&self, _: &Key) -> mypass_platform::PlatformResult<bool> {
        Err(mypass_platform::PlatformError::Failed("injected".into()))
    }
    fn read(&self, _: &Key, _: &str, _: ValueType) -> mypass_platform::PlatformResult<Read> {
        Err(mypass_platform::PlatformError::Failed("injected".into()))
    }
    fn create(&self, _: &Key) -> mypass_platform::PlatformResult<()> {
        Err(mypass_platform::PlatformError::Failed("injected".into()))
    }
    fn set(&self, _: &Key, _: &str, _: Value) -> mypass_platform::PlatformResult<()> {
        Err(mypass_platform::PlatformError::Failed("injected".into()))
    }
    fn delete_value(&self, _: &Key, _: &str) -> mypass_platform::PlatformResult<()> {
        Err(mypass_platform::PlatformError::Failed("injected".into()))
    }
    fn delete_owned_key(
        &self,
        _: &Key,
        _: &str,
        _: &Value,
    ) -> mypass_platform::PlatformResult<bool> {
        Err(mypass_platform::PlatformError::Failed("injected".into()))
    }
}
#[test]
fn win_launch_at_login_error_maps_to_platform_failed() {
    let launcher = WinLaunchAtLogin::new(Arc::new(FailingRegistry), r"C:\MyPass.exe".into());
    assert!(matches!(
        launcher.status(),
        Err(mypass_platform::PlatformError::Failed(_))
    ));
    assert!(matches!(
        launcher.set_enabled(true),
        Err(mypass_platform::PlatformError::Failed(_))
    ));
    assert!(matches!(
        launcher.set_enabled(false),
        Err(mypass_platform::PlatformError::Failed(_))
    ));
    assert!(matches!(
        launcher.prepare_startup(true),
        Err(mypass_platform::PlatformError::Failed(_))
    ));
}
