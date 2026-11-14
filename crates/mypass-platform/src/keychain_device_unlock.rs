//! Device unlock over a secret-store item and a local-authentication seam: the portable core of
//! the macOS body (`macos_device_unlock.rs` supplies the `LAContext` side).
//!
//! # Decisions (stage 5, T11 with rulings R3 and AB-1)
//!
//! 1. **LAContext gating plus a file-keychain item, not `SecAccessControl(biometryCurrentSet)`.**
//!    A `SecAccessControl` item needs the data-protection keychain, which needs the
//!    `keychain-access-groups` entitlement and a provisioning profile for a non-sandboxed app:
//!    extra Apple-side setup and a restricted entitlement, while every other MyPass item already
//!    lives in the login keychain. Cost: code running as the user inside our own process can read
//!    the item without Touch ID (the old app had the same exposure); the item's access list still
//!    stops OTHER apps. Improvement over the old app: the biometric domain state
//!    (`LAContext.evaluatedPolicyDomainState`, stored as a SHA-256 hash in a second item) is
//!    compared on every biometrics-only check, which gives `biometryCurrentSet` semantics: adding
//!    or removing a fingerprint invalidates a [`DeviceUnlockMode::Biometrics`] enrolment and the
//!    master password is needed again. [`DeviceUnlockMode::AccountPassword`] (policy
//!    `deviceOwnerAuthentication`, which offers Touch ID first and the account password as
//!    fallback) is not invalidated by a biometric change: the account password unlocks the Mac
//!    anyway. The `SecAccessControl` upgrade is a ROADMAP item.
//! 2. **The existing app-only store, unchanged (R3).** No `kSecAttrAccessible` attribute and no
//!    retry around it: file-keychain items never sync and ignore or reject the attribute.
//! 3. **Names.** Service `com.mypass.app.biometric` (the old app's), accounts
//!    `deviceUnlockPassword.<vault_id>.<mode>` (one item per mode, matching the trait) and
//!    `deviceUnlockDomainState.<vault_id>`. The old app's un-suffixed `deviceUnlockPassword` item
//!    is never touched here: it belongs to the old app's access list (deleting it could prompt)
//!    and the cutover checklist removes it.
//! 4. **`forget` sweeps (AB-1).** `forget(v)` lists the service's accounts and deletes every item
//!    of `v`, whatever mode tag it carries (an item of a mode this build no longer writes is
//!    stale, too), then removes stale items of OTHER vaults: a domain-state item whose vault has
//!    no password item left, and password items whose mode tag this build does not know. A valid
//!    enrolment of another vault is kept (the contract requires it).
//! 5. **No prompt without an item.** `unlock` checks the enrolment silently first and answers
//!    `Invalidated` without showing a prompt when nothing usable is stored.
//!
//! No secret reaches an error, a log or a `Debug` output; the bytes read from the store are
//! zeroized on drop.

use crate::{
    DeviceUnlock, DeviceUnlockAvailability, DeviceUnlockMode, PlatformError, PlatformResult,
    SecretStore, VaultId,
};
use secrecy::{ExposeSecret, SecretString};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use std::time::Duration;
use zeroize::Zeroizing;

/// Keychain service of every device-unlock item (the old app's service).
pub(crate) const SERVICE: &str = "com.mypass.app.biometric";
/// Account prefix of the stored master password (the old app's account name).
pub(crate) const PASSWORD_ACCOUNT: &str = "deviceUnlockPassword";
/// Account prefix of the stored biometric domain-state hash.
pub(crate) const DOMAIN_STATE_ACCOUNT: &str = "deviceUnlockDomainState";
/// Ceiling of one unlock prompt.
pub(crate) const UNLOCK_TIMEOUT: Duration = Duration::from_secs(120);
/// Ceiling of one presence prompt (R9; the passkey relay deadline is longer).
pub(crate) const PRESENCE_TIMEOUT: Duration = Duration::from_secs(60);

/// The LocalAuthentication policy a check runs under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AuthPolicy {
    /// `LAPolicyDeviceOwnerAuthenticationWithBiometrics` (1): Touch ID only.
    Biometrics,
    /// `LAPolicyDeviceOwnerAuthentication` (2): Touch ID first, the account password as fallback.
    DeviceOwner,
}

impl AuthPolicy {
    /// The raw `LAPolicy` value.
    pub(crate) fn raw(self) -> isize {
        match self {
            AuthPolicy::Biometrics => 1,
            AuthPolicy::DeviceOwner => 2,
        }
    }
}

/// The policy of an unlock in `mode`.
pub(crate) fn policy_for(mode: DeviceUnlockMode) -> AuthPolicy {
    match mode {
        DeviceUnlockMode::Biometrics => AuthPolicy::Biometrics,
        DeviceUnlockMode::AccountPassword => AuthPolicy::DeviceOwner,
    }
}

/// The account-name tag of `mode`.
fn mode_tag(mode: DeviceUnlockMode) -> &'static str {
    match mode {
        DeviceUnlockMode::Biometrics => "biometrics",
        DeviceUnlockMode::AccountPassword => "accountPassword",
    }
}

/// `deviceUnlockPassword.<vault_id>.<mode>`.
pub(crate) fn account_for(mode: DeviceUnlockMode, vault_id: &VaultId) -> String {
    format!(
        "{PASSWORD_ACCOUNT}.{}.{}",
        vault_id.as_str(),
        mode_tag(mode)
    )
}

/// `deviceUnlockDomainState.<vault_id>`.
pub(crate) fn domain_state_account(vault_id: &VaultId) -> String {
    format!("{DOMAIN_STATE_ACCOUNT}.{}", vault_id.as_str())
}

/// What one of our accounts is, parsed back from its name.
#[derive(Debug, PartialEq, Eq)]
enum AccountKind<'a> {
    /// A password item of `vault`, with a mode tag this build writes (`known`) or not.
    Password { vault: &'a str, known: bool },
    /// The domain-state item of `vault`.
    DomainState { vault: &'a str },
}

/// Parses one of our account names; `None` for anything else (the old app's un-suffixed item).
fn parse_account(account: &str) -> Option<AccountKind<'_>> {
    if let Some(rest) = account.strip_prefix(DOMAIN_STATE_ACCOUNT) {
        let vault = rest.strip_prefix('.')?;
        return (!vault.is_empty()).then_some(AccountKind::DomainState { vault });
    }
    let rest = account.strip_prefix(PASSWORD_ACCOUNT)?.strip_prefix('.')?;
    // Vault ids never contain a dot (`VaultId`), so the last dot separates the mode tag.
    let (vault, tag) = rest.rsplit_once('.').unwrap_or((rest, ""));
    if vault.is_empty() {
        return None;
    }
    let known = [
        DeviceUnlockMode::Biometrics,
        DeviceUnlockMode::AccountPassword,
    ]
    .iter()
    .any(|m| mode_tag(*m) == tag);
    Some(AccountKind::Password { vault, known })
}

/// Maps an `LAError` code onto the platform error.
pub(crate) fn map_la_error(code: i64) -> PlatformError {
    match code {
        // userCancel, userFallback (the fallback button is hidden for the biometrics policy, so
        // this only means "not now"), systemCancel, appCancel (also our own timeout).
        -2 | -3 | -4 | -9 => PlatformError::Cancelled,
        // biometryLockout.
        -8 => PlatformError::Lockout,
        // passcodeNotSet, biometryNotAvailable, biometryNotEnrolled.
        -7..=-5 => PlatformError::NotEnrolledOnDevice,
        other => PlatformError::Failed(format!("local authentication error {other}")),
    }
}

/// SHA-256 of a biometric domain state (only the hash is stored).
pub(crate) fn domain_state_hash(state: &[u8]) -> [u8; 32] {
    Sha256::digest(state).into()
}

/// Whether the stored domain-state hash still matches the device. Only the biometrics-only mode is
/// bound to the enrolled fingers; a missing stored hash or a missing current state (no Touch ID
/// any more) invalidates that mode.
pub(crate) fn domain_state_matches(
    mode: DeviceUnlockMode,
    stored_hash: Option<&[u8]>,
    current_state: Option<&[u8]>,
) -> bool {
    match mode {
        DeviceUnlockMode::AccountPassword => true,
        DeviceUnlockMode::Biometrics => match (stored_hash, current_state) {
            (Some(stored), Some(current)) => stored == domain_state_hash(current).as_slice(),
            _ => false,
        },
    }
}

/// The LocalAuthentication operations device unlock needs (the seam over `LAContext`).
pub(crate) trait LocalAuth: Send + Sync {
    /// Whether `policy` can be evaluated now; `Err` carries the `LAError` code. Never prompts.
    fn can_evaluate(&self, policy: AuthPolicy) -> Result<(), i64>;
    /// The current biometric domain state, read without a prompt; `None` without Touch ID.
    fn biometric_domain_state(&self) -> Option<Vec<u8>>;
    /// Shows the prompt (`reason`) and waits at most `timeout`; a timeout cancels the prompt and
    /// reports `Cancelled`. `Ok` carries the domain state the evaluation saw, when any.
    fn evaluate(
        &self,
        policy: AuthPolicy,
        reason: &str,
        timeout: Duration,
    ) -> PlatformResult<Option<Vec<u8>>>;
}

/// A [`SecretStore`] that can also list the accounts of a service (attributes only, never data).
pub(crate) trait ItemStore: SecretStore {
    /// Every account stored under `service`; empty when there is none.
    fn accounts(&self, service: &str) -> PlatformResult<Vec<String>>;
}

/// [`DeviceUnlock`] over an [`ItemStore`] and a [`LocalAuth`].
pub(crate) struct KeychainDeviceUnlock<A> {
    auth: A,
    store: Arc<dyn ItemStore>,
    service: String,
}

impl<A> std::fmt::Debug for KeychainDeviceUnlock<A> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeychainDeviceUnlock")
            .field("service", &self.service)
            .finish_non_exhaustive()
    }
}

fn vault(vault_id: &str) -> PlatformResult<VaultId> {
    VaultId::new(vault_id).map_err(|_| PlatformError::Failed("invalid vault id".to_owned()))
}

fn check_reason(reason: &str) -> PlatformResult<()> {
    // LocalAuthentication raises an Objective-C exception for an empty reason.
    if reason.trim().is_empty() {
        return Err(PlatformError::Failed(
            "the prompt reason is empty".to_owned(),
        ));
    }
    Ok(())
}

impl<A: LocalAuth> KeychainDeviceUnlock<A> {
    /// Device unlock over `store`, with every item under `service` (tests use a unique one).
    pub(crate) fn new(auth: A, store: Arc<dyn ItemStore>, service: &str) -> Self {
        Self {
            auth,
            store,
            service: service.to_owned(),
        }
    }

    fn read(&self, account: &str) -> PlatformResult<Option<Zeroizing<Vec<u8>>>> {
        Ok(self.store.get(&self.service, account)?.map(Zeroizing::new))
    }

    fn stored_domain_hash(&self, v: &VaultId) -> PlatformResult<Option<Zeroizing<Vec<u8>>>> {
        self.read(&domain_state_account(v))
    }

    /// The silent enrolment check: an item exists and, for the biometrics-only mode, the stored
    /// domain-state hash matches `current_state`.
    fn enrolled_with(
        &self,
        mode: DeviceUnlockMode,
        v: &VaultId,
        current_state: Option<&[u8]>,
    ) -> PlatformResult<bool> {
        if self.read(&account_for(mode, v))?.is_none() {
            return Ok(false);
        }
        if mode == DeviceUnlockMode::AccountPassword {
            return Ok(true);
        }
        let stored = self.stored_domain_hash(v)?;
        Ok(domain_state_matches(
            mode,
            stored.as_deref().map(Vec::as_slice),
            current_state,
        ))
    }

    fn current_state_for(&self, mode: DeviceUnlockMode) -> Option<Vec<u8>> {
        match mode {
            DeviceUnlockMode::Biometrics => self.auth.biometric_domain_state(),
            DeviceUnlockMode::AccountPassword => None,
        }
    }
}

impl<A: LocalAuth> DeviceUnlock for KeychainDeviceUnlock<A> {
    fn availability(&self) -> DeviceUnlockAvailability {
        if self.auth.can_evaluate(AuthPolicy::Biometrics).is_ok() {
            DeviceUnlockAvailability::Available { biometrics: true }
        } else if self.auth.can_evaluate(AuthPolicy::DeviceOwner).is_ok() {
            DeviceUnlockAvailability::Available { biometrics: false }
        } else {
            DeviceUnlockAvailability::Unavailable
        }
    }

    fn is_enrolled(&self, mode: DeviceUnlockMode, vault_id: &str) -> PlatformResult<bool> {
        let v = vault(vault_id)?;
        let current = self.current_state_for(mode);
        self.enrolled_with(mode, &v, current.as_deref())
    }

    fn enroll(
        &self,
        mode: DeviceUnlockMode,
        vault_id: &str,
        secret: &SecretString,
    ) -> PlatformResult<()> {
        let v = vault(vault_id)?;
        self.auth
            .can_evaluate(policy_for(mode))
            .map_err(map_la_error)?;
        let state_hash = match mode {
            DeviceUnlockMode::Biometrics => Some(domain_state_hash(
                &self
                    .auth
                    .biometric_domain_state()
                    .ok_or(PlatformError::NotEnrolledOnDevice)?,
            )),
            DeviceUnlockMode::AccountPassword => None,
        };
        let account = account_for(mode, &v);
        self.store
            .set(&self.service, &account, secret.expose_secret().as_bytes())?;
        if let Some(hash) = state_hash {
            if let Err(e) = self
                .store
                .set(&self.service, &domain_state_account(&v), &hash)
            {
                // Never leave a biometrics item that no domain state guards.
                let _ = self.store.delete(&self.service, &account);
                return Err(e);
            }
        }
        Ok(())
    }

    fn unlock(
        &self,
        mode: DeviceUnlockMode,
        vault_id: &str,
        reason: &str,
    ) -> PlatformResult<SecretString> {
        check_reason(reason)?;
        let v = vault(vault_id)?;
        if !self.is_enrolled(mode, vault_id)? {
            return Err(PlatformError::Invalidated);
        }
        let evaluated = self
            .auth
            .evaluate(policy_for(mode), reason, UNLOCK_TIMEOUT)?;
        if mode == DeviceUnlockMode::Biometrics {
            // The state the evaluation itself saw is authoritative (a finger added during the
            // prompt counts); fall back to a silent read when the context reported none.
            let current = evaluated.or_else(|| self.auth.biometric_domain_state());
            if !self.enrolled_with(mode, &v, current.as_deref())? {
                return Err(PlatformError::Invalidated);
            }
        }
        let bytes = self
            .read(&account_for(mode, &v))?
            .ok_or(PlatformError::Invalidated)?;
        let text = std::str::from_utf8(&bytes)
            .map_err(|_| PlatformError::Failed("the stored secret is not valid text".to_owned()))?;
        Ok(SecretString::from(text.to_owned()))
    }

    fn forget(&self, vault_id: &str) -> PlatformResult<()> {
        let v = vault(vault_id)?;
        // The known names first, so a failed listing still forgets this vault.
        for mode in [
            DeviceUnlockMode::Biometrics,
            DeviceUnlockMode::AccountPassword,
        ] {
            self.store.delete(&self.service, &account_for(mode, &v))?;
        }
        self.store
            .delete(&self.service, &domain_state_account(&v))?;

        let accounts = self.store.accounts(&self.service)?;
        let parsed: Vec<(&str, AccountKind<'_>)> = accounts
            .iter()
            .filter_map(|a| parse_account(a).map(|k| (a.as_str(), k)))
            .collect();
        let has_password = |vault: &str| {
            parsed
                .iter()
                .any(|(_, k)| matches!(k, AccountKind::Password { vault: x, known: true } if *x == vault))
        };
        for (account, kind) in &parsed {
            let stale = match kind {
                AccountKind::Password { vault, known } => *vault == v.as_str() || !known,
                AccountKind::DomainState { vault } => *vault == v.as_str() || !has_password(vault),
            };
            if stale {
                self.store.delete(&self.service, account)?;
            }
        }
        Ok(())
    }

    fn verify_presence(&self, reason: &str) -> PlatformResult<()> {
        check_reason(reason)?;
        self.auth
            .evaluate(AuthPolicy::DeviceOwner, reason, PRESENCE_TIMEOUT)
            .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::device_unlock_contract::{self as contract, CaseOutcome, ContractSubject};
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
        m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// An [`ItemStore`] in memory, keyed by `(service, account)`.
    #[derive(Default)]
    struct MemoryItems(Mutex<BTreeMap<(String, String), Vec<u8>>>);

    impl SecretStore for MemoryItems {
        fn set(&self, service: &str, account: &str, secret: &[u8]) -> PlatformResult<()> {
            lock(&self.0).insert((service.into(), account.into()), secret.to_vec());
            Ok(())
        }
        fn get(&self, service: &str, account: &str) -> PlatformResult<Option<Vec<u8>>> {
            Ok(lock(&self.0)
                .get(&(service.into(), account.into()))
                .cloned())
        }
        fn delete(&self, service: &str, account: &str) -> PlatformResult<()> {
            lock(&self.0).remove(&(service.into(), account.into()));
            Ok(())
        }
    }

    impl ItemStore for MemoryItems {
        fn accounts(&self, service: &str) -> PlatformResult<Vec<String>> {
            Ok(lock(&self.0)
                .keys()
                .filter(|(s, _)| s == service)
                .map(|(_, a)| a.clone())
                .collect())
        }
    }

    impl MemoryItems {
        fn accounts_of(&self, service: &str) -> Vec<String> {
            self.accounts(service).unwrap()
        }
    }

    #[derive(Default)]
    struct AuthState {
        biometrics: bool,
        device_owner: bool,
        domain_state: Vec<u8>,
        fail_next: Option<PlatformError>,
        evaluations: usize,
        presence: usize,
        last_policy: Option<AuthPolicy>,
        last_timeout: Option<Duration>,
    }

    /// A scripted [`LocalAuth`].
    #[derive(Clone, Default)]
    struct FakeAuth(Arc<Mutex<AuthState>>);

    impl FakeAuth {
        fn touch_id() -> Self {
            let a = Self::default();
            {
                let mut s = lock(&a.0);
                s.biometrics = true;
                s.device_owner = true;
                s.domain_state = b"fingers-1".to_vec();
            }
            a
        }
    }

    impl LocalAuth for FakeAuth {
        fn can_evaluate(&self, policy: AuthPolicy) -> Result<(), i64> {
            let s = lock(&self.0);
            match policy {
                AuthPolicy::Biometrics if s.biometrics => Ok(()),
                AuthPolicy::Biometrics => Err(-7),
                AuthPolicy::DeviceOwner if s.device_owner => Ok(()),
                AuthPolicy::DeviceOwner => Err(-5),
            }
        }
        fn biometric_domain_state(&self) -> Option<Vec<u8>> {
            let s = lock(&self.0);
            s.biometrics.then(|| s.domain_state.clone())
        }
        fn evaluate(
            &self,
            policy: AuthPolicy,
            _reason: &str,
            timeout: Duration,
        ) -> PlatformResult<Option<Vec<u8>>> {
            let mut s = lock(&self.0);
            s.last_policy = Some(policy);
            s.last_timeout = Some(timeout);
            if timeout == PRESENCE_TIMEOUT {
                s.presence += 1;
            } else {
                s.evaluations += 1;
            }
            if let Some(e) = s.fail_next.take() {
                return Err(e);
            }
            Ok(s.biometrics.then(|| s.domain_state.clone()))
        }
    }

    struct Fixture {
        auth: FakeAuth,
        items: Arc<MemoryItems>,
        du: KeychainDeviceUnlock<FakeAuth>,
    }

    const SVC: &str = "com.mypass.tests.unit";

    fn fixture() -> Fixture {
        let auth = FakeAuth::touch_id();
        let items = Arc::new(MemoryItems::default());
        let du = KeychainDeviceUnlock::new(auth.clone(), items.clone(), SVC);
        Fixture { auth, items, du }
    }

    fn secret(s: &str) -> SecretString {
        SecretString::from(s.to_owned())
    }

    fn id(s: &str) -> VaultId {
        VaultId::new(s).unwrap()
    }

    #[test]
    fn la_error_code_mapping_table() {
        for code in [-2, -3, -4, -9] {
            assert_eq!(map_la_error(code), PlatformError::Cancelled, "{code}");
        }
        assert_eq!(map_la_error(-8), PlatformError::Lockout);
        for code in [-5, -6, -7] {
            assert_eq!(
                map_la_error(code),
                PlatformError::NotEnrolledOnDevice,
                "{code}"
            );
        }
        for code in [-1, -10, -1004, 0, 7] {
            assert_eq!(
                map_la_error(code),
                PlatformError::Failed(format!("local authentication error {code}"))
            );
        }
    }

    #[test]
    fn policy_for_mode_table() {
        assert_eq!(
            policy_for(DeviceUnlockMode::Biometrics),
            AuthPolicy::Biometrics
        );
        assert_eq!(
            policy_for(DeviceUnlockMode::AccountPassword),
            AuthPolicy::DeviceOwner
        );
        assert_eq!(AuthPolicy::Biometrics.raw(), 1);
        assert_eq!(AuthPolicy::DeviceOwner.raw(), 2);
    }

    #[test]
    fn account_names_are_per_vault_and_start_with_legacy_prefix() {
        let a = id("vault-a");
        let b = id("vault_b");
        assert_eq!(SERVICE, "com.mypass.app.biometric");
        assert_eq!(
            account_for(DeviceUnlockMode::Biometrics, &a),
            "deviceUnlockPassword.vault-a.biometrics"
        );
        assert_eq!(
            account_for(DeviceUnlockMode::AccountPassword, &a),
            "deviceUnlockPassword.vault-a.accountPassword"
        );
        assert_eq!(domain_state_account(&b), "deviceUnlockDomainState.vault_b");
        for mode in [
            DeviceUnlockMode::Biometrics,
            DeviceUnlockMode::AccountPassword,
        ] {
            assert!(account_for(mode, &a).starts_with("deviceUnlockPassword."));
            assert_ne!(account_for(mode, &a), account_for(mode, &b));
        }
    }

    #[test]
    fn account_names_parse_back_and_ignore_foreign_items() {
        assert_eq!(
            parse_account("deviceUnlockPassword.v1.biometrics"),
            Some(AccountKind::Password {
                vault: "v1",
                known: true
            })
        );
        assert_eq!(
            parse_account("deviceUnlockPassword.v1.somethingNew"),
            Some(AccountKind::Password {
                vault: "v1",
                known: false
            })
        );
        assert_eq!(
            parse_account("deviceUnlockPassword.v1"),
            Some(AccountKind::Password {
                vault: "v1",
                known: false
            })
        );
        assert_eq!(
            parse_account("deviceUnlockDomainState.v1"),
            Some(AccountKind::DomainState { vault: "v1" })
        );
        // The old app's un-suffixed item and anything else are not ours to sweep.
        for foreign in [
            "deviceUnlockPassword",
            "deviceUnlockDomainState",
            "deviceUnlockDomainState.",
            "deviceUnlockPassword.",
            "deviceUnlockPasswordX.v1.biometrics",
            "other",
        ] {
            assert_eq!(parse_account(foreign), None, "{foreign}");
        }
    }

    #[test]
    fn domain_state_check_only_applies_to_biometric_modes() {
        let hash = domain_state_hash(b"fingers-1");
        let acct = DeviceUnlockMode::AccountPassword;
        let bio = DeviceUnlockMode::Biometrics;
        assert!(domain_state_matches(acct, None, None));
        assert!(domain_state_matches(acct, Some(&hash), Some(b"fingers-2")));
        assert!(domain_state_matches(bio, Some(&hash), Some(b"fingers-1")));
        assert!(!domain_state_matches(bio, Some(&hash), Some(b"fingers-2")));
        assert!(!domain_state_matches(bio, Some(&hash), None));
        // The raw state is never what is stored: only its hash matches.
        assert!(!domain_state_matches(
            bio,
            Some(b"fingers-1"),
            Some(b"fingers-1")
        ));
    }

    #[test]
    fn missing_domain_state_item_invalidates_only_biometrics_mode() {
        let f = fixture();
        let v = id("vault-a");
        f.du.enroll(DeviceUnlockMode::Biometrics, "vault-a", &secret("bio"))
            .unwrap();
        f.du.enroll(
            DeviceUnlockMode::AccountPassword,
            "vault-a",
            &secret("acct"),
        )
        .unwrap();
        f.items.delete(SVC, &domain_state_account(&v)).unwrap();
        assert_eq!(
            f.du.is_enrolled(DeviceUnlockMode::Biometrics, "vault-a"),
            Ok(false)
        );
        assert_eq!(
            f.du.is_enrolled(DeviceUnlockMode::AccountPassword, "vault-a"),
            Ok(true)
        );
        assert_eq!(
            f.du.unlock(DeviceUnlockMode::Biometrics, "vault-a", "r")
                .map(|_| ()),
            Err(PlatformError::Invalidated)
        );
    }

    #[test]
    fn enrolment_stores_only_the_hash_of_the_domain_state() {
        let f = fixture();
        f.du.enroll(DeviceUnlockMode::Biometrics, "vault-a", &secret("pw"))
            .unwrap();
        let stored = f
            .items
            .get(SVC, &domain_state_account(&id("vault-a")))
            .unwrap()
            .unwrap();
        assert_eq!(stored, domain_state_hash(b"fingers-1").to_vec());
        assert_eq!(
            f.items
                .get(
                    SVC,
                    &account_for(DeviceUnlockMode::Biometrics, &id("vault-a"))
                )
                .unwrap(),
            Some(b"pw".to_vec())
        );
    }

    #[test]
    fn biometrics_enrolment_needs_touch_id_and_account_password_does_not() {
        let f = fixture();
        {
            let mut s = lock(&f.auth.0);
            s.biometrics = false;
        }
        assert_eq!(
            f.du.enroll(DeviceUnlockMode::Biometrics, "vault-a", &secret("pw")),
            Err(PlatformError::NotEnrolledOnDevice)
        );
        assert!(f.items.accounts_of(SVC).is_empty());
        assert_eq!(
            f.du.enroll(DeviceUnlockMode::AccountPassword, "vault-a", &secret("pw")),
            Ok(())
        );
        lock(&f.auth.0).device_owner = false;
        assert_eq!(
            f.du.enroll(DeviceUnlockMode::AccountPassword, "vault-b", &secret("pw")),
            Err(PlatformError::NotEnrolledOnDevice)
        );
    }

    #[test]
    fn availability_follows_the_policies() {
        let f = fixture();
        assert_eq!(
            f.du.availability(),
            DeviceUnlockAvailability::Available { biometrics: true }
        );
        lock(&f.auth.0).biometrics = false;
        assert_eq!(
            f.du.availability(),
            DeviceUnlockAvailability::Available { biometrics: false }
        );
        lock(&f.auth.0).device_owner = false;
        assert_eq!(f.du.availability(), DeviceUnlockAvailability::Unavailable);
    }

    #[test]
    fn unlock_without_an_item_never_prompts() {
        let f = fixture();
        assert_eq!(
            f.du.unlock(DeviceUnlockMode::AccountPassword, "vault-a", "r")
                .map(|_| ()),
            Err(PlatformError::Invalidated)
        );
        assert_eq!(lock(&f.auth.0).evaluations, 0);
    }

    #[test]
    fn unlock_uses_the_mode_policy_and_the_unlock_ceiling() {
        let f = fixture();
        f.du.enroll(DeviceUnlockMode::Biometrics, "vault-a", &secret("pw"))
            .unwrap();
        let got =
            f.du.unlock(DeviceUnlockMode::Biometrics, "vault-a", "Unlock MyPass")
                .unwrap();
        assert_eq!(got.expose_secret(), "pw");
        let s = lock(&f.auth.0);
        assert_eq!(s.last_policy, Some(AuthPolicy::Biometrics));
        assert_eq!(s.last_timeout, Some(UNLOCK_TIMEOUT));
    }

    #[test]
    fn finger_change_during_the_prompt_invalidates() {
        // The evaluation reports a different state than the stored one: Invalidated, no secret.
        struct ChangingAuth(FakeAuth);
        impl LocalAuth for ChangingAuth {
            fn can_evaluate(&self, p: AuthPolicy) -> Result<(), i64> {
                self.0.can_evaluate(p)
            }
            fn biometric_domain_state(&self) -> Option<Vec<u8>> {
                self.0.biometric_domain_state()
            }
            fn evaluate(
                &self,
                _: AuthPolicy,
                _: &str,
                _: Duration,
            ) -> PlatformResult<Option<Vec<u8>>> {
                Ok(Some(b"fingers-2".to_vec()))
            }
        }
        let items = Arc::new(MemoryItems::default());
        let du = KeychainDeviceUnlock::new(ChangingAuth(FakeAuth::touch_id()), items, SVC);
        du.enroll(DeviceUnlockMode::Biometrics, "vault-a", &secret("pw"))
            .unwrap();
        assert_eq!(
            du.unlock(DeviceUnlockMode::Biometrics, "vault-a", "r")
                .map(|_| ()),
            Err(PlatformError::Invalidated)
        );
    }

    #[test]
    fn verify_presence_uses_device_owner_policy_and_sixty_second_ceiling() {
        let f = fixture();
        assert_eq!(f.du.verify_presence("Sign in to example.com"), Ok(()));
        let s = lock(&f.auth.0);
        assert_eq!(s.last_policy, Some(AuthPolicy::DeviceOwner));
        assert_eq!(s.last_timeout, Some(Duration::from_secs(60)));
        assert_eq!(s.presence, 1);
        drop(s);
        assert!(f.items.accounts_of(SVC).is_empty());
        lock(&f.auth.0).fail_next = Some(PlatformError::Cancelled);
        assert_eq!(f.du.verify_presence("r"), Err(PlatformError::Cancelled));
    }

    #[test]
    fn empty_reason_is_refused_before_any_prompt() {
        let f = fixture();
        f.du.enroll(DeviceUnlockMode::AccountPassword, "vault-a", &secret("pw"))
            .unwrap();
        for reason in ["", "  "] {
            assert!(matches!(
                f.du.verify_presence(reason),
                Err(PlatformError::Failed(_))
            ));
            assert!(matches!(
                f.du.unlock(DeviceUnlockMode::AccountPassword, "vault-a", reason),
                Err(PlatformError::Failed(_))
            ));
        }
        let s = lock(&f.auth.0);
        assert_eq!((s.evaluations, s.presence), (0, 0));
    }

    #[test]
    fn invalid_vault_ids_are_refused() {
        let f = fixture();
        for bad in ["", "a.b", "../x", "a/b"] {
            assert!(matches!(
                f.du.enroll(DeviceUnlockMode::AccountPassword, bad, &secret("pw")),
                Err(PlatformError::Failed(_))
            ));
            assert!(matches!(f.du.forget(bad), Err(PlatformError::Failed(_))));
        }
        assert!(f.items.accounts_of(SVC).is_empty());
    }

    #[test]
    fn forget_removes_secret_and_domain_state_and_sweeps_stale_items() {
        let f = fixture();
        f.du.enroll(DeviceUnlockMode::Biometrics, "vault-a", &secret("a"))
            .unwrap();
        f.du.enroll(DeviceUnlockMode::AccountPassword, "vault-a", &secret("a"))
            .unwrap();
        f.du.enroll(DeviceUnlockMode::Biometrics, "vault-b", &secret("b"))
            .unwrap();
        // Stale: a mode tag of another build for vault-a and vault-c, an orphan domain state of
        // vault-d. Foreign: the old app's un-suffixed item and another service's item.
        for account in [
            "deviceUnlockPassword.vault-a.legacyMode",
            "deviceUnlockPassword.vault-c.legacyMode",
            "deviceUnlockDomainState.vault-d",
            "deviceUnlockPassword",
        ] {
            f.items.set(SVC, account, b"x").unwrap();
        }
        f.items
            .set(
                "other.service",
                "deviceUnlockPassword.vault-a.biometrics",
                b"x",
            )
            .unwrap();

        f.du.forget("vault-a").unwrap();

        assert_eq!(
            f.items.accounts_of(SVC),
            [
                "deviceUnlockDomainState.vault-b",
                "deviceUnlockPassword",
                "deviceUnlockPassword.vault-b.biometrics",
            ]
        );
        assert_eq!(f.items.accounts_of("other.service").len(), 1);
        assert_eq!(
            f.du.is_enrolled(DeviceUnlockMode::Biometrics, "vault-b"),
            Ok(true)
        );
        // Forgetting again is not an error.
        f.du.forget("vault-a").unwrap();
    }

    /// A contract subject over the real logic with the fake LocalAuth: every hook is available.
    fn subject() -> ContractSubject {
        let auth = FakeAuth::touch_id();
        let du: Arc<dyn DeviceUnlock> = Arc::new(KeychainDeviceUnlock::new(
            auth.clone(),
            Arc::new(MemoryItems::default()),
            SVC,
        ));
        let (a, b, c, d, e) = (auth.clone(), auth.clone(), auth.clone(), auth.clone(), auth);
        ContractSubject {
            unlock: du,
            invalidate_biometrics: Some(Box::new(move || {
                lock(&a.0).domain_state = b"fingers-changed".to_vec();
            })),
            fail_next_unlock: Some(Box::new(move |err| lock(&b.0).fail_next = Some(err))),
            evaluation_count: Some(Box::new(move || lock(&c.0).evaluations)),
            fail_next_presence: Some(Box::new(move |err| lock(&d.0).fail_next = Some(err))),
            presence_count: Some(Box::new(move || lock(&e.0).presence)),
        }
    }

    #[test]
    fn keychain_logic_passes_the_whole_contract_with_hooks() {
        for (name, outcome) in contract::run(&subject) {
            assert_eq!(outcome, CaseOutcome::Ran, "{name}");
        }
    }

    #[test]
    fn debug_never_shows_items() {
        let f = fixture();
        f.du.enroll(
            DeviceUnlockMode::AccountPassword,
            "vault-a",
            &secret("hunter2"),
        )
        .unwrap();
        let text = format!("{:?}", f.du);
        assert!(!text.contains("hunter2"));
        assert!(text.contains(SVC));
    }
}
