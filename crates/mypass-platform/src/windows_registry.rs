//! Safe, typed Windows registry contract. Native operations land with registry consumers;
//! portable policy tests use MemoryRegistry. No stringly typed absolute write paths.
use crate::{PlatformError, PlatformResult};
use std::{
    collections::BTreeMap,
    sync::{Mutex, PoisonError},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Hive {
    CurrentUser,
    LocalMachine,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum View {
    Registry32,
    Registry64,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    String(String),
    Binary(Vec<u8>),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueType {
    String,
    Binary,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Read {
    Missing,
    WrongType,
    Value(Value),
}

/// A validated relative key; separators and segments have exactly one canonical form.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Key {
    pub hive: Hive,
    pub view: View,
    subkey: String,
}
impl Key {
    pub fn new(hive: Hive, view: View, subkey: &str) -> PlatformResult<Self> {
        if subkey
            .split('\\')
            .any(|s| s.is_empty() || s == "." || s == "..")
            || subkey.contains(['/', '\0', ':'])
        {
            return Err(PlatformError::Failed(
                "invalid relative registry key".into(),
            ));
        }
        Ok(Self {
            hive,
            view,
            subkey: subkey.to_ascii_lowercase(),
        })
    }
    pub fn subkey(&self) -> &str {
        &self.subkey
    }
}

pub trait Registry: Send + Sync {
    fn key_exists(&self, key: &Key) -> PlatformResult<bool>;
    fn read(&self, key: &Key, name: &str, kind: ValueType) -> PlatformResult<Read>;
    fn create(&self, key: &Key) -> PlatformResult<()>;
    fn set(&self, key: &Key, name: &str, value: Value) -> PlatformResult<()>;
    fn delete_value(&self, key: &Key, name: &str) -> PlatformResult<()>;
    /// Delete only the matching owned value, then its key if empty; preserve other values/children.
    fn delete_owned_key(&self, key: &Key, owner_name: &str, owner: &Value) -> PlatformResult<bool>;
}

/// Maps every operation into a fresh HKCU test subtree. Callers cannot supply an absolute prefix.
/// Apply this adapter around a native registry only in explicitly gated integration helpers.
pub struct IsolatedRegistry<R> {
    inner: R,
    root: String,
}
impl<R> IsolatedRegistry<R> {
    pub fn new(inner: R) -> Self {
        Self {
            inner,
            root: format!("Software\\MyPassTests\\{}", uuid::Uuid::new_v4()),
        }
    }
    pub fn root(&self) -> &str {
        &self.root
    }
    fn map(&self, key: &Key) -> PlatformResult<Key> {
        Key::new(
            Hive::CurrentUser,
            key.view,
            &format!(
                "{}\\{}",
                self.root,
                key.subkey()
                    .strip_prefix("software\\")
                    .unwrap_or(key.subkey())
            ),
        )
    }
}
impl<R: Registry> Registry for IsolatedRegistry<R> {
    fn key_exists(&self, k: &Key) -> PlatformResult<bool> {
        self.inner.key_exists(&self.map(k)?)
    }
    fn read(&self, k: &Key, n: &str, t: ValueType) -> PlatformResult<Read> {
        self.inner.read(&self.map(k)?, n, t)
    }
    fn create(&self, k: &Key) -> PlatformResult<()> {
        self.inner.create(&self.map(k)?)
    }
    fn set(&self, k: &Key, n: &str, v: Value) -> PlatformResult<()> {
        self.inner.set(&self.map(k)?, n, v)
    }
    fn delete_value(&self, k: &Key, n: &str) -> PlatformResult<()> {
        self.inner.delete_value(&self.map(k)?, n)
    }
    fn delete_owned_key(&self, k: &Key, n: &str, v: &Value) -> PlatformResult<bool> {
        self.inner.delete_owned_key(&self.map(k)?, n, v)
    }
}

type Keys = BTreeMap<Key, BTreeMap<String, Value>>;
#[derive(Default)]
pub struct MemoryRegistry {
    keys: Mutex<Keys>,
}
impl MemoryRegistry {
    fn write_key(key: &Key, name: &str) -> PlatformResult<()> {
        if key.hive != Hive::CurrentUser || name.contains('\0') {
            return Err(PlatformError::Failed("registry write denied".into()));
        }
        Ok(())
    }
}
impl Registry for MemoryRegistry {
    fn key_exists(&self, k: &Key) -> PlatformResult<bool> {
        Ok(self
            .keys
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains_key(k))
    }
    fn read(&self, k: &Key, n: &str, t: ValueType) -> PlatformResult<Read> {
        if n.contains('\0') {
            return Err(PlatformError::Failed("invalid registry value name".into()));
        }
        let keys = self.keys.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(
            match keys
                .get(k)
                .and_then(|values| values.get(&n.to_ascii_lowercase()))
            {
                None => Read::Missing,
                Some(v)
                    if matches!(
                        (v, t),
                        (Value::String(_), ValueType::String)
                            | (Value::Binary(_), ValueType::Binary)
                    ) =>
                {
                    Read::Value(v.clone())
                }
                Some(_) => Read::WrongType,
            },
        )
    }
    fn create(&self, k: &Key) -> PlatformResult<()> {
        Self::write_key(k, "")?;
        self.keys
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(k.clone())
            .or_default();
        Ok(())
    }
    fn set(&self, k: &Key, n: &str, v: Value) -> PlatformResult<()> {
        Self::write_key(k, n)?;
        self.keys
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get_mut(k)
            .ok_or_else(|| PlatformError::Failed("registry key missing".into()))?
            .insert(n.to_ascii_lowercase(), v);
        Ok(())
    }
    fn delete_value(&self, k: &Key, n: &str) -> PlatformResult<()> {
        Self::write_key(k, n)?;
        if let Some(values) = self
            .keys
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get_mut(k)
        {
            values.remove(&n.to_ascii_lowercase());
        }
        Ok(())
    }
    fn delete_owned_key(&self, k: &Key, n: &str, owner: &Value) -> PlatformResult<bool> {
        Self::write_key(k, n)?;
        let mut keys = self.keys.lock().unwrap_or_else(PoisonError::into_inner);
        let prefix = format!("{}\\", k.subkey());
        if keys.get(k).and_then(|v| v.get(&n.to_ascii_lowercase())) != Some(owner) {
            return Ok(false);
        }
        if let Some(values) = keys.get_mut(k) {
            values.remove(&n.to_ascii_lowercase());
        }
        if keys.get(k).is_some_and(BTreeMap::is_empty)
            && !keys.keys().any(|child| {
                child.hive == k.hive && child.view == k.view && child.subkey().starts_with(&prefix)
            })
        {
            keys.remove(k);
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn windows_registry_test_adapter_has_no_second_software_prefix() {
        let isolated = IsolatedRegistry::new(MemoryRegistry::default());
        let key = Key::new(
            Hive::LocalMachine,
            View::Registry64,
            "Software\\Google\\Chrome",
        )
        .unwrap();
        let mapped = isolated.map(&key).unwrap();
        assert_eq!(mapped.hive, Hive::CurrentUser);
        assert_eq!(
            mapped.subkey(),
            format!("{}\\google\\chrome", isolated.root().to_ascii_lowercase())
        );
    }
}
