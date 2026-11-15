//! Current-user DPAPI files, rather than size-limited Credential Manager blobs.
//! App and shared directories/entropy provide domain separation, not same-user isolation:
//! any code running as this user can call DPAPI. Public entropy is not a secret.
//! Only encrypted MPS1 envelopes reach disk; the host constructs only the shared namespace.

use crate::{PlatformError, PlatformResult, SecretStore};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use zeroize::Zeroizing;

pub(crate) const MAX_PLAINTEXT: usize = 1024 * 1024;
pub(crate) const MAX_ENVELOPE: usize = MAX_PLAINTEXT + 64 * 1024;
const MAGIC: &[u8] = b"MPS1";

#[derive(Clone, Copy)]
pub enum SecretClass {
    Shared,
    AppOnly,
}
impl SecretClass {
    fn entropy(self) -> &'static [u8] {
        match self {
            Self::Shared => b"MyPass.shared.v1",
            Self::AppOnly => b"MyPass.app.v1",
        }
    }
}
pub(crate) fn invalid() -> PlatformError {
    PlatformError::Failed("invalid protected secret".into())
}
pub(crate) trait Protector: Send + Sync {
    fn protect(&self, plain: &[u8], entropy: &[u8]) -> PlatformResult<Vec<u8>>;
    fn unprotect(&self, cipher: &[u8], entropy: &[u8]) -> PlatformResult<Zeroizing<Vec<u8>>>;
}
/// Native implementation pins directory ancestors and validates the opened handles before I/O.
pub(crate) trait ProtectedFiles: Send + Sync {
    fn read(&self, name: &str) -> PlatformResult<Option<Vec<u8>>>;
    fn publish(&self, name: &str, bytes: &[u8]) -> PlatformResult<()>;
    fn delete(&self, name: &str) -> PlatformResult<()>;
}

pub struct DpapiSecretStore {
    entropy: &'static [u8],
    protector: Arc<dyn Protector>,
    files: Arc<dyn ProtectedFiles>,
}
impl std::fmt::Debug for DpapiSecretStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DpapiSecretStore").finish_non_exhaustive()
    }
}
impl DpapiSecretStore {
    #[cfg(windows)]
    pub fn shared(paths: &crate::AppPaths) -> Self {
        Self::at(paths.secrets_shared.clone(), SecretClass::Shared)
    }
    #[cfg(windows)]
    pub fn app(paths: &crate::AppPaths) -> Self {
        Self::at(paths.secrets_app.clone(), SecretClass::AppOnly)
    }
    #[cfg(windows)]
    pub fn at(dir: std::path::PathBuf, class: SecretClass) -> Self {
        Self::with_backends(
            class,
            Arc::new(crate::windows::DpapiProtector),
            Arc::new(crate::windows::DpapiFiles::new(dir)),
        )
    }
    fn with_backends(
        class: SecretClass,
        protector: Arc<dyn Protector>,
        files: Arc<dyn ProtectedFiles>,
    ) -> Self {
        Self {
            entropy: class.entropy(),
            protector,
            files,
        }
    }
}
pub(crate) fn item_name(service: &str, account: &str) -> PlatformResult<String> {
    if service.contains('\0') || account.contains('\0') {
        return Err(invalid());
    }
    let mut hash = Sha256::new();
    hash.update(service.as_bytes());
    hash.update([0]);
    hash.update(account.as_bytes());
    let mut name: String = hash
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    name.push_str(".secret");
    Ok(name)
}
impl SecretStore for DpapiSecretStore {
    fn set(&self, service: &str, account: &str, secret: &[u8]) -> PlatformResult<()> {
        let name = item_name(service, account)?;
        if secret.len() > MAX_PLAINTEXT {
            return Err(invalid());
        }
        let plain = Zeroizing::new(secret.to_vec());
        let cipher = self.protector.protect(&plain, self.entropy)?;
        if cipher.len() > MAX_ENVELOPE - MAGIC.len() {
            return Err(invalid());
        }
        let bytes = [MAGIC, &cipher].concat();
        self.files.publish(&name, &bytes)
    }
    fn get(&self, service: &str, account: &str) -> PlatformResult<Option<Vec<u8>>> {
        let Some(bytes) = self.files.read(&item_name(service, account)?)? else {
            return Ok(None);
        };
        let bytes = Zeroizing::new(bytes);
        if bytes.len() > MAX_ENVELOPE {
            return Err(invalid());
        }
        let cipher = bytes
            .strip_prefix(MAGIC)
            .filter(|b| !b.is_empty())
            .ok_or_else(invalid)?;
        let plain = self.protector.unprotect(cipher, self.entropy)?;
        if plain.len() > MAX_PLAINTEXT {
            return Err(invalid());
        }
        // SecretStore's public contract returns an owned Vec; consumers wrap it in Zeroizing.
        Ok(Some(plain.to_vec()))
    }
    fn delete(&self, service: &str, account: &str) -> PlatformResult<()> {
        self.files.delete(&item_name(service, account)?)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct MemoryFiles(Mutex<std::collections::HashMap<String, Vec<u8>>>);
    impl ProtectedFiles for MemoryFiles {
        fn read(&self, name: &str) -> PlatformResult<Option<Vec<u8>>> {
            Ok(self.0.lock().unwrap().get(name).cloned())
        }
        fn publish(&self, name: &str, bytes: &[u8]) -> PlatformResult<()> {
            self.0.lock().unwrap().insert(name.into(), bytes.into());
            Ok(())
        }
        fn delete(&self, name: &str) -> PlatformResult<()> {
            self.0.lock().unwrap().remove(name);
            Ok(())
        }
    }
    // Deliberately test-only deterministic envelope, not product cryptography.
    struct FakeProtector;
    impl Protector for FakeProtector {
        fn protect(&self, plain: &[u8], entropy: &[u8]) -> PlatformResult<Vec<u8>> {
            let mut envelope = [entropy, b"|", plain].concat();
            envelope.extend_from_slice(&Sha256::digest(&envelope));
            Ok(envelope)
        }
        fn unprotect(&self, cipher: &[u8], entropy: &[u8]) -> PlatformResult<Zeroizing<Vec<u8>>> {
            let n = cipher.len().checked_sub(32).ok_or_else(invalid)?;
            let (body, digest) = cipher.split_at(n);
            if Sha256::digest(body).as_slice() != digest {
                return Err(invalid());
            }
            body.strip_prefix([entropy, b"|"].concat().as_slice())
                .map(|b| Zeroizing::new(b.to_vec()))
                .ok_or_else(invalid)
        }
    }
    fn fixture(class: SecretClass) -> (DpapiSecretStore, Arc<MemoryFiles>) {
        let files = Arc::new(MemoryFiles::default());
        (
            DpapiSecretStore::with_backends(class, Arc::new(FakeProtector), files.clone()),
            files,
        )
    }
    #[test]
    fn file_name_is_hex_sha256_of_service_nul_account() {
        assert_eq!(
            item_name("svc", "acct").unwrap(),
            "9e90ed835d2872a97ff1a7d2f729658e132c9cb3fb2232e3261dba3ff72cdf1e.secret"
        );
        assert_ne!(item_name("ab", "c").unwrap(), item_name("a", "bc").unwrap());
        assert!(item_name("a\0b", "c").is_err());
        assert!(item_name("a", "b\0c").is_err());
    }
    #[test]
    fn missing_item_is_none_and_delete_of_missing_is_ok() {
        let (s, _) = fixture(SecretClass::Shared);
        assert_eq!(s.get("svc", "acct").unwrap(), None);
        s.delete("svc", "acct").unwrap();
        s.set("svc", "acct", b"sentinel").unwrap();
        assert_eq!(s.get("svc", "acct").unwrap().unwrap(), b"sentinel");
        s.delete("svc", "acct").unwrap();
        assert_eq!(s.get("svc", "acct").unwrap(), None);
    }
    #[test]
    fn corrupt_magic_decryption_and_oversize_are_errors_without_secret_text() {
        let (s, files) = fixture(SecretClass::Shared);
        for bytes in [
            b"SECRETWORD".to_vec(),
            b"MPS1SECRETWORD".to_vec(),
            vec![0; MAX_ENVELOPE + 1],
        ] {
            files
                .0
                .lock()
                .unwrap()
                .insert(item_name("s", "a").unwrap(), bytes);
            let err = s.get("s", "a").unwrap_err();
            assert!(!format!("{err:?} {err}").contains("SECRETWORD"));
        }
        assert!(s.set("s", "a", &vec![0; MAX_PLAINTEXT + 1]).is_err());
    }
    #[test]
    fn encryption_failure_does_not_publish_or_replace_and_tampering_is_refused() {
        struct Fails;
        impl Protector for Fails {
            fn protect(&self, _: &[u8], _: &[u8]) -> PlatformResult<Vec<u8>> {
                Err(invalid())
            }
            fn unprotect(&self, _: &[u8], _: &[u8]) -> PlatformResult<Zeroizing<Vec<u8>>> {
                Err(invalid())
            }
        }
        let (s, files) = fixture(SecretClass::Shared);
        s.set("s", "a", b"original").unwrap();
        let failing =
            DpapiSecretStore::with_backends(SecretClass::Shared, Arc::new(Fails), files.clone());
        assert!(failing.set("s", "a", b"SECRETWORD").is_err());
        assert_eq!(s.get("s", "a").unwrap().unwrap(), b"original");
        let name = item_name("s", "a").unwrap();
        if let Some(last) = files.0.lock().unwrap().get_mut(&name).unwrap().last_mut() {
            *last ^= 1;
        }
        assert!(s.get("s", "a").is_err());
    }
    #[test]
    fn classes_use_distinct_entropy_and_debug_never_exposes_buffers() {
        let (shared, files) = fixture(SecretClass::Shared);
        let app = DpapiSecretStore::with_backends(
            SecretClass::AppOnly,
            Arc::new(FakeProtector),
            files.clone(),
        );
        shared.set("s", "a", b"SECRETWORD").unwrap();
        assert!(app.get("s", "a").is_err());
        assert!(!format!("{shared:?}").contains("SECRETWORD"));
        assert!(files
            .read(&item_name("s", "a").unwrap())
            .unwrap()
            .unwrap()
            .starts_with(b"MPS1"));
    }
}
