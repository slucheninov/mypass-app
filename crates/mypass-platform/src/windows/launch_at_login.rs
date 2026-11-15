//! HKCU Run policy. StartupApproved is OS-owned and is never modified here.
use crate::windows_registry::{Hive, Key, Read, Registry, Value, ValueType, View};
use crate::{LaunchAtLogin, LoginItemStatus, PlatformError, PlatformResult};
use std::{path::PathBuf, sync::Arc};
const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const APPROVED: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";

pub struct WinLaunchAtLogin {
    registry: Arc<dyn Registry>,
    exe: PathBuf,
}
impl WinLaunchAtLogin {
    pub fn new(registry: Arc<dyn Registry>, exe: PathBuf) -> Self {
        Self { registry, exe }
    }
    fn key(path: &str) -> PlatformResult<Key> {
        Key::new(Hive::CurrentUser, View::Registry64, path)
    }
    fn command(&self) -> PlatformResult<String> {
        let exe = self
            .exe
            .to_str()
            .ok_or_else(|| PlatformError::Failed("login executable is not Unicode".into()))?;
        let absolute = exe.as_bytes().get(1..3) == Some(b":\\") || exe.starts_with(r"\\");
        if !absolute || exe.contains(['\0', '"']) {
            return Err(PlatformError::Failed(
                "login executable must be an absolute Windows path".into(),
            ));
        }
        Ok(format!("\"{exe}\" --autostarted"))
    }
    fn run_value(&self) -> PlatformResult<Read> {
        self.registry
            .read(&Self::key(RUN)?, "MyPass", ValueType::String)
    }
}
impl LaunchAtLogin for WinLaunchAtLogin {
    fn preserve_intent_on_approval(&self) -> bool {
        true
    }
    fn status(&self) -> PlatformResult<LoginItemStatus> {
        match self.run_value()? {
            Read::Missing => return Ok(LoginItemStatus::Disabled),
            Read::WrongType | Read::Value(Value::Binary(_)) => {
                return Err(PlatformError::Failed("invalid login Run value".into()))
            }
            Read::Value(Value::String(_)) => {}
        }
        // No StartupApproved layout is documented by this implementation. Even an apparently
        // enabled record requires OS approval rather than guessing or undoing Task Manager.
        Ok(
            match self
                .registry
                .read(&Self::key(APPROVED)?, "MyPass", ValueType::Binary)?
            {
                Read::Missing => LoginItemStatus::Enabled,
                _ => LoginItemStatus::RequiresApproval,
            },
        )
    }
    fn set_enabled(&self, enabled: bool) -> PlatformResult<()> {
        let key = Self::key(RUN)?;
        if enabled {
            let command = self.command()?;
            self.registry.create(&key)?;
            self.registry.set(&key, "MyPass", Value::String(command))
        } else {
            self.registry.delete_value(&key, "MyPass")
        }
    }
    fn prepare_startup(&self, desired: bool) -> PlatformResult<()> {
        if !desired {
            return Ok(());
        }
        if let Read::Value(Value::String(old)) = self.run_value()? {
            let command = self.command()?;
            // Only repair our recognizable quoted launch command with the same executable name.
            let owned = old
                .strip_prefix('"')
                .and_then(|s| s.strip_suffix("\" --autostarted"))
                .and_then(|s| s.rsplit('\\').next())
                .zip(self.exe.to_str().and_then(|s| s.rsplit('\\').next()))
                .is_some_and(|(old, current)| old.eq_ignore_ascii_case(current));
            if owned && old != command {
                self.registry
                    .set(&Self::key(RUN)?, "MyPass", Value::String(command))?;
            }
        }
        Ok(())
    }
}
