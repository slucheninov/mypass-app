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
//! 6. **Silent checks never show Keychain UI.** Every enrolment check (`is_enrolled`, the check
//!    before an unlock prompt) reads item attributes only, through
//!    [`ItemStore::attributes_silently`], which never returns item data and fails instead of
//!    showing UI. The domain-state hash is therefore kept as a public attribute (the item's
//!    comment, hex) of the domain-state item; it is a hash of an opaque state, not a secret. The
//!    master password is read only on the explicit unlock path, after the user passed the
//!    LocalAuthentication prompt. An attribute read that needs interaction is an error, never
//!    "not enrolled".
//! 7. **Only a changed finger set invalidates.** A biometric preflight that fails for a reason that
//!    says nothing about the enrolled fingers (lockout, sensor not available, for example with the
//!    lid closed, passcode not set, an unknown code) is reported as its error and the enrolment is
//!    kept ([`classify_biometric_preflight`]); only a domain state that differs from the stored one,
//!    or no fingers enrolled at all (`biometryNotEnrolled`, the set was emptied), invalidates.
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
        // authenticationFailed: the prompt rejected the finger or the password.
        -1 => PlatformError::AuthenticationFailed,
        // biometryLockout.
        -8 => PlatformError::Lockout,
        // passcodeNotSet, biometryNotAvailable, biometryNotEnrolled.
        -7..=-5 => PlatformError::NotEnrolledOnDevice,
        other => PlatformError::Failed(format!("local authentication error {other}")),
    }
}

/// `LAErrorBiometryNotEnrolled`: no fingers are enrolled on this Mac.
const LA_BIOMETRY_NOT_ENROLLED: i64 = -7;

/// The raw result of a biometric preflight: the domain state, or the `LAError` code.
pub(crate) type Preflight = Result<Option<Vec<u8>>, i64>;

/// What a silent biometric preflight says about the enrolled fingers.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum CurrentBiometrics {
    /// The current domain state, compared with the stored hash.
    State(Vec<u8>),
    /// No fingers are enrolled any more: the enrolled set changed, a biometrics enrolment is
    /// invalid.
    NoneEnrolled,
    /// The state cannot be read now; this says nothing about a change, so the check reports the
    /// error and keeps the enrolment.
    Unknown(PlatformError),
}

/// Classifies the result of a biometric preflight ([`LocalAuth::biometric_domain_state`]): `Err`
/// carries the `LAError` code of `canEvaluatePolicy`, `Ok(None)` a successful preflight without a
/// state (which Apple's contract rules out; treated as unknown, never as a change).
pub(crate) fn classify_biometric_preflight(result: Preflight) -> CurrentBiometrics {
    match result {
        Ok(Some(state)) => CurrentBiometrics::State(state),
        Ok(None) => CurrentBiometrics::Unknown(PlatformError::Failed(
            "the biometric domain state is not available".to_owned(),
        )),
        Err(LA_BIOMETRY_NOT_ENROLLED) => CurrentBiometrics::NoneEnrolled,
        // biometryLockout -> Lockout, biometryNotAvailable and passcodeNotSet ->
        // NotEnrolledOnDevice, anything else -> Failed: an error, the enrolment stays.
        Err(code) => CurrentBiometrics::Unknown(map_la_error(code)),
    }
}

/// Lowercase hex of `bytes` (the public form of the stored domain-state hash).
fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The bytes of a lowercase or uppercase hex string; `None` for anything else.
fn from_hex(text: &str) -> Option<Vec<u8>> {
    // An odd length leaves a one-byte last chunk, which the match refuses.
    let digit = |c: u8| char::from(c).to_digit(16);
    text.as_bytes()
        .chunks(2)
        .map(|pair| match pair {
            [high, low] => Some(u8::try_from(digit(*high)? << 4 | digit(*low)?).ok()?),
            _ => None,
        })
        .collect()
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
    /// The current biometric domain state, read without a prompt after a biometric preflight;
    /// `Err` carries the preflight's `LAError` code (classified by
    /// [`classify_biometric_preflight`]), `Ok(None)` a successful preflight without a state.
    fn biometric_domain_state(&self) -> Preflight;
    /// Shows the prompt (`reason`) and waits at most `timeout`; a timeout cancels the prompt and
    /// reports `Cancelled`. `Ok` carries the domain state the evaluation saw, when any.
    fn evaluate(
        &self,
        policy: AuthPolicy,
        reason: &str,
        timeout: Duration,
    ) -> PlatformResult<Option<Vec<u8>>>;
}

/// The public attributes of one item, read without its data.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ItemAttributes {
    /// The item's note (`kSecAttrComment`), when it has one.
    pub(crate) note: Option<String>,
}

/// A [`SecretStore`] that can also answer from item attributes alone, never reading item data and
/// never showing UI.
pub(crate) trait ItemStore: SecretStore {
    /// Every account stored under `service`; empty when there is none.
    fn accounts(&self, service: &str) -> PlatformResult<Vec<String>>;

    /// The attributes of one item (`None`: no such item), from an attribute-only query that may
    /// not show any UI: when the store would need interaction (a locked keychain, an access
    /// prompt), this is an error, never `None`.
    fn attributes_silently(
        &self,
        service: &str,
        account: &str,
    ) -> PlatformResult<Option<ItemAttributes>>;

    /// Stores `secret` like [`SecretStore::set`], with `note` as a public attribute that
    /// [`Self::attributes_silently`] returns.
    fn set_with_note(
        &self,
        service: &str,
        account: &str,
        secret: &[u8],
        note: &str,
    ) -> PlatformResult<()>;
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

    /// Read the stored master password after device authentication.
    fn read_secret(&self, account: &str) -> PlatformResult<Option<Zeroizing<Vec<u8>>>> {
        Ok(self.store.get(&self.service, account)?.map(Zeroizing::new))
    }

    /// The stored domain-state hash, from the domain-state item's public note (silent).
    fn stored_domain_hash(&self, v: &VaultId) -> PlatformResult<Option<Vec<u8>>> {
        Ok(self
            .store
            .attributes_silently(&self.service, &domain_state_account(v))?
            .and_then(|attributes| attributes.note)
            .and_then(|note| from_hex(&note)))
    }

    /// The silent enrolment check, from item attributes only: an item exists and, for the
    /// biometrics-only mode, the stored domain-state hash matches the current state (`evaluated`
    /// when an evaluation just reported one, otherwise a silent preflight). A preflight that
    /// cannot tell (lockout and the like) is an error and changes nothing.
    fn enrolled_with(
        &self,
        mode: DeviceUnlockMode,
        v: &VaultId,
        evaluated: Option<Vec<u8>>,
    ) -> PlatformResult<bool> {
        if self
            .store
            .attributes_silently(&self.service, &account_for(mode, v))?
            .is_none()
        {
            return Ok(false);
        }
        if mode == DeviceUnlockMode::AccountPassword {
            return Ok(true);
        }
        let stored = self.stored_domain_hash(v)?;
        let current = match evaluated {
            Some(state) => state,
            None => match classify_biometric_preflight(self.auth.biometric_domain_state()) {
                CurrentBiometrics::State(state) => state,
                CurrentBiometrics::NoneEnrolled => return Ok(false),
                CurrentBiometrics::Unknown(error) => return Err(error),
            },
        };
        Ok(domain_state_matches(
            mode,
            stored.as_deref(),
            Some(&current),
        ))
    }

    fn unlock_using(
        &self,
        mode: DeviceUnlockMode,
        vault_id: &str,
        reason: &str,
        policy: AuthPolicy,
    ) -> PlatformResult<SecretString> {
        check_reason(reason)?;
        let v = vault(vault_id)?;
        if !self.is_enrolled(mode, vault_id)? {
            return Err(PlatformError::Invalidated);
        }
        let evaluated = self
            .auth
            .evaluate(policy, reason, crate::DEVICE_UNLOCK_TIMEOUT)?;
        if mode == DeviceUnlockMode::Biometrics {
            // The state the evaluation itself saw is authoritative (a finger added during the
            // prompt counts); `enrolled_with` falls back to a silent preflight when the context
            // reported none.
            if !self.enrolled_with(mode, &v, evaluated)? {
                return Err(PlatformError::Invalidated);
            }
        }
        // The only read of item data: the user just passed the prompt for this unlock.
        let bytes = self
            .read_secret(&account_for(mode, &v))?
            .ok_or(PlatformError::Invalidated)?;
        let text = std::str::from_utf8(&bytes)
            .map_err(|_| PlatformError::Failed("the stored secret is not valid text".to_owned()))?;
        Ok(SecretString::from(text.to_owned()))
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
        self.enrolled_with(mode, &v, None)
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
            DeviceUnlockMode::Biometrics => {
                match classify_biometric_preflight(self.auth.biometric_domain_state()) {
                    CurrentBiometrics::State(state) => Some(domain_state_hash(&state)),
                    CurrentBiometrics::NoneEnrolled => {
                        return Err(PlatformError::NotEnrolledOnDevice)
                    }
                    CurrentBiometrics::Unknown(error) => return Err(error),
                }
            }
            DeviceUnlockMode::AccountPassword => None,
        };
        let account = account_for(mode, &v);
        self.store
            .set(&self.service, &account, secret.expose_secret().as_bytes())?;
        if let Some(hash) = state_hash {
            if let Err(e) = self.store.set_with_note(
                &self.service,
                &domain_state_account(&v),
                &hash,
                &to_hex(&hash),
            ) {
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
        self.unlock_using(mode, vault_id, reason, policy_for(mode))
    }

    fn unlock_with_biometrics(
        &self,
        mode: DeviceUnlockMode,
        vault_id: &str,
        reason: &str,
    ) -> PlatformResult<SecretString> {
        self.unlock_using(mode, vault_id, reason, AuthPolicy::Biometrics)
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

    /// One stored item: its data and its public note.
    type Item = (Vec<u8>, Option<String>);

    /// An [`ItemStore`] in memory, keyed by `(service, account)`. Counts the data reads (the
    /// only operation that could show Keychain UI) and can act as a keychain that needs
    /// interaction.
    #[derive(Default)]
    struct MemoryItems {
        items: Mutex<BTreeMap<(String, String), Item>>,
        data_reads: std::sync::atomic::AtomicUsize,
        needs_interaction: std::sync::atomic::AtomicBool,
    }

    impl MemoryItems {
        fn data_reads(&self) -> usize {
            self.data_reads.load(std::sync::atomic::Ordering::SeqCst)
        }
        fn set_needs_interaction(&self, needs: bool) {
            self.needs_interaction
                .store(needs, std::sync::atomic::Ordering::SeqCst);
        }
        fn snapshot(&self) -> BTreeMap<(String, String), Item> {
            lock(&self.items).clone()
        }
    }

    impl SecretStore for MemoryItems {
        fn set(&self, service: &str, account: &str, secret: &[u8]) -> PlatformResult<()> {
            lock(&self.items).insert((service.into(), account.into()), (secret.to_vec(), None));
            Ok(())
        }
        fn get(&self, service: &str, account: &str) -> PlatformResult<Option<Vec<u8>>> {
            self.data_reads
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(lock(&self.items)
                .get(&(service.into(), account.into()))
                .map(|(data, _)| data.clone()))
        }
        fn delete(&self, service: &str, account: &str) -> PlatformResult<()> {
            lock(&self.items).remove(&(service.into(), account.into()));
            Ok(())
        }
    }

    impl ItemStore for MemoryItems {
        fn accounts(&self, service: &str) -> PlatformResult<Vec<String>> {
            Ok(lock(&self.items)
                .keys()
                .filter(|(s, _)| s == service)
                .map(|(_, a)| a.clone())
                .collect())
        }
        fn attributes_silently(
            &self,
            service: &str,
            account: &str,
        ) -> PlatformResult<Option<ItemAttributes>> {
            if self
                .needs_interaction
                .load(std::sync::atomic::Ordering::SeqCst)
            {
                return Err(PlatformError::Failed("keychain status -25308".to_owned()));
            }
            Ok(lock(&self.items)
                .get(&(service.into(), account.into()))
                .map(|(_, note)| ItemAttributes { note: note.clone() }))
        }
        fn set_with_note(
            &self,
            service: &str,
            account: &str,
            secret: &[u8],
            note: &str,
        ) -> PlatformResult<()> {
            lock(&self.items).insert(
                (service.into(), account.into()),
                (secret.to_vec(), Some(note.to_owned())),
            );
            Ok(())
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
        /// The `LAError` code a biometric preflight fails with (lockout and the like) while the
        /// fingers are still enrolled.
        preflight_error: Option<i64>,
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
        fn biometric_domain_state(&self) -> Preflight {
            let s = lock(&self.0);
            if let Some(code) = s.preflight_error {
                return Err(code);
            }
            if !s.biometrics {
                return Err(LA_BIOMETRY_NOT_ENROLLED);
            }
            Ok(Some(s.domain_state.clone()))
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
        assert_eq!(map_la_error(-1), PlatformError::AuthenticationFailed);
        for code in [-5, -6, -7] {
            assert_eq!(
                map_la_error(code),
                PlatformError::NotEnrolledOnDevice,
                "{code}"
            );
        }
        for code in [-10, -1004, 0, 7] {
            assert_eq!(
                map_la_error(code),
                PlatformError::Failed(format!("local authentication error {code}"))
            );
        }
    }

    #[test]
    fn biometric_preflight_mapping_table() {
        use CurrentBiometrics::{NoneEnrolled, State, Unknown};
        let cases: [(Preflight, CurrentBiometrics); 8] = [
            (Ok(Some(b"s".to_vec())), State(b"s".to_vec())),
            // Only an emptied finger set says the enrolled fingers changed.
            (Err(-7), NoneEnrolled),
            // biometryLockout: temporary, the enrolment stays.
            (Err(-8), Unknown(PlatformError::Lockout)),
            // biometryNotAvailable (lid closed, no sensor right now) and passcodeNotSet.
            (Err(-6), Unknown(PlatformError::NotEnrolledOnDevice)),
            (Err(-5), Unknown(PlatformError::NotEnrolledOnDevice)),
            (
                Err(-1004),
                Unknown(PlatformError::Failed(
                    "local authentication error -1004".to_owned(),
                )),
            ),
            (Err(-1), Unknown(PlatformError::AuthenticationFailed)),
            (
                Ok(None),
                Unknown(PlatformError::Failed(
                    "the biometric domain state is not available".to_owned(),
                )),
            ),
        ];
        for (input, expected) in cases {
            let label = format!("{input:?}");
            assert_eq!(classify_biometric_preflight(input), expected, "{label}");
        }
    }

    #[test]
    fn hex_round_trips_and_refuses_garbage() {
        let hash = domain_state_hash(b"fingers-1");
        assert_eq!(from_hex(&to_hex(&hash)), Some(hash.to_vec()));
        assert_eq!(to_hex(&[0x00, 0xab, 0x0f]), "00ab0f");
        assert_eq!(from_hex("00AB0f"), Some(vec![0x00, 0xab, 0x0f]));
        for bad in ["0", "zz", "0g", "+1", "\u{e9}a"] {
            assert_eq!(from_hex(bad), None, "{bad}");
        }
    }

    #[test]
    fn preflight_errors_keep_a_valid_biometrics_enrolment() {
        for (code, expected) in [
            (-8, PlatformError::Lockout),
            (-6, PlatformError::NotEnrolledOnDevice),
            (-5, PlatformError::NotEnrolledOnDevice),
            (-1, PlatformError::AuthenticationFailed),
        ] {
            let f = fixture();
            f.du.enroll(DeviceUnlockMode::Biometrics, "vault-a", &secret("pw"))
                .unwrap();
            let before = f.items.snapshot();
            lock(&f.auth.0).preflight_error = Some(code);
            assert_eq!(
                f.du.is_enrolled(DeviceUnlockMode::Biometrics, "vault-a"),
                Err(expected.clone()),
                "{code}"
            );
            assert_eq!(
                f.du.unlock(DeviceUnlockMode::Biometrics, "vault-a", "r")
                    .map(|_| ()),
                Err(expected),
                "{code}"
            );
            assert_eq!(lock(&f.auth.0).evaluations, 0, "{code}: no prompt");
            assert_eq!(f.items.snapshot(), before, "{code}: items untouched");
            // The temporary condition passes: the same enrolment works again.
            lock(&f.auth.0).preflight_error = None;
            assert_eq!(
                f.du.is_enrolled(DeviceUnlockMode::Biometrics, "vault-a"),
                Ok(true),
                "{code}"
            );
            assert_eq!(
                f.du.unlock(DeviceUnlockMode::Biometrics, "vault-a", "r")
                    .unwrap()
                    .expose_secret(),
                "pw"
            );
        }
    }

    #[test]
    fn an_emptied_finger_set_invalidates_biometrics_only() {
        let f = fixture();
        f.du.enroll(DeviceUnlockMode::Biometrics, "vault-a", &secret("pw"))
            .unwrap();
        f.du.enroll(DeviceUnlockMode::AccountPassword, "vault-a", &secret("pw"))
            .unwrap();
        lock(&f.auth.0).biometrics = false;
        assert_eq!(
            f.du.is_enrolled(DeviceUnlockMode::Biometrics, "vault-a"),
            Ok(false)
        );
        assert_eq!(
            f.du.is_enrolled(DeviceUnlockMode::AccountPassword, "vault-a"),
            Ok(true)
        );
    }

    #[test]
    fn a_preflight_error_stops_a_biometrics_enrolment() {
        let f = fixture();
        lock(&f.auth.0).preflight_error = Some(-8);
        // `can_evaluate` still passes in the fake, so the state read decides.
        assert_eq!(
            f.du.enroll(DeviceUnlockMode::Biometrics, "vault-a", &secret("pw")),
            Err(PlatformError::Lockout)
        );
        assert!(f.items.accounts_of(SVC).is_empty());
    }

    #[test]
    fn silent_checks_never_read_item_data() {
        let f = fixture();
        f.du.enroll(DeviceUnlockMode::Biometrics, "vault-a", &secret("pw"))
            .unwrap();
        f.du.enroll(DeviceUnlockMode::AccountPassword, "vault-a", &secret("pw"))
            .unwrap();
        for mode in [
            DeviceUnlockMode::Biometrics,
            DeviceUnlockMode::AccountPassword,
        ] {
            assert_eq!(f.du.is_enrolled(mode, "vault-a"), Ok(true));
            assert_eq!(f.du.is_enrolled(mode, "vault-b"), Ok(false));
        }
        let _ = f.du.availability();
        assert_eq!(f.items.data_reads(), 0);
        // A refused unlock (no item) reads nothing either.
        assert_eq!(
            f.du.unlock(DeviceUnlockMode::Biometrics, "vault-b", "r")
                .map(|_| ()),
            Err(PlatformError::Invalidated)
        );
        assert_eq!(f.items.data_reads(), 0);
        // A cancelled prompt reads nothing.
        lock(&f.auth.0).fail_next = Some(PlatformError::Cancelled);
        assert_eq!(
            f.du.unlock(DeviceUnlockMode::Biometrics, "vault-a", "r")
                .map(|_| ()),
            Err(PlatformError::Cancelled)
        );
        assert_eq!(f.items.data_reads(), 0);
        // Only the explicit unlock, after the prompt, reads the secret: once.
        f.du.unlock(DeviceUnlockMode::Biometrics, "vault-a", "r")
            .unwrap();
        assert_eq!(f.items.data_reads(), 1);
    }

    #[test]
    fn a_keychain_that_needs_interaction_is_an_error_not_absent() {
        let f = fixture();
        f.du.enroll(DeviceUnlockMode::Biometrics, "vault-a", &secret("pw"))
            .unwrap();
        let before = f.items.snapshot();
        f.items.set_needs_interaction(true);
        for mode in [
            DeviceUnlockMode::Biometrics,
            DeviceUnlockMode::AccountPassword,
        ] {
            assert!(
                matches!(
                    f.du.is_enrolled(mode, "vault-a"),
                    Err(PlatformError::Failed(_))
                ),
                "{mode:?}"
            );
        }
        assert!(matches!(
            f.du.unlock(DeviceUnlockMode::Biometrics, "vault-a", "r"),
            Err(PlatformError::Failed(_))
        ));
        assert_eq!(lock(&f.auth.0).evaluations, 0);
        assert_eq!(f.items.data_reads(), 0);
        assert_eq!(f.items.snapshot(), before);
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
                .attributes_silently(SVC, &domain_state_account(&id("vault-a")))
                .unwrap()
                .and_then(|a| a.note),
            Some(to_hex(&domain_state_hash(b"fingers-1")))
        );
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
        assert_eq!(s.last_timeout, Some(crate::DEVICE_UNLOCK_TIMEOUT));
    }

    #[test]
    fn automatic_unlock_uses_biometrics_for_an_account_password_enrolment() {
        let f = fixture();
        let mode = DeviceUnlockMode::AccountPassword;
        f.du.enroll(mode, "vault-a", &secret("pw")).unwrap();
        let got =
            f.du.unlock_with_biometrics(mode, "vault-a", "Unlock MyPass")
                .unwrap();
        assert_eq!(got.expose_secret(), "pw");
        assert_eq!(lock(&f.auth.0).last_policy, Some(AuthPolicy::Biometrics));
        f.du.unlock(mode, "vault-a", "Unlock MyPass").unwrap();
        assert_eq!(lock(&f.auth.0).last_policy, Some(AuthPolicy::DeviceOwner));
    }

    #[test]
    fn finger_change_during_the_prompt_invalidates() {
        // The evaluation reports a different state than the stored one: Invalidated, no secret.
        struct ChangingAuth(FakeAuth);
        impl LocalAuth for ChangingAuth {
            fn can_evaluate(&self, p: AuthPolicy) -> Result<(), i64> {
                self.0.can_evaluate(p)
            }
            fn biometric_domain_state(&self) -> Preflight {
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
