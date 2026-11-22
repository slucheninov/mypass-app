//! Unlocking a vault with Touch ID or the account password (stage 5 implements macOS).
//!
//! Stage 5's `mode_for_toggles` maps the two Settings toggles onto the two modes: Touch ID only ->
//! [`DeviceUnlockMode::Biometrics`]; Mac password only, or both -> [`DeviceUnlockMode::AccountPassword`],
//! whose OS policy offers Touch ID first.

use crate::{PlatformError, PlatformResult};
use secrecy::SecretString;
use std::time::Duration;

/// Time before the platform cancels a device unlock evaluation.
pub const DEVICE_UNLOCK_TIMEOUT: Duration = Duration::from_secs(120);

/// How the device proves the user is present.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceUnlockMode {
    /// Biometrics only.
    Biometrics,
    /// The account password (the OS may offer biometrics first).
    AccountPassword,
}

/// Whether device unlock can be offered at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceUnlockAvailability {
    /// Not available; `app_info.deviceUnlockAvailable` is `availability != Unavailable`.
    Unavailable,
    /// Available, with or without biometrics.
    Available {
        /// Biometrics are set up.
        biometrics: bool,
    },
}

/// Device unlock: stores a vault's master password behind a device check.
pub trait DeviceUnlock: Send + Sync {
    /// Current availability; unsupported platforms report `Unavailable`.
    fn availability(&self) -> DeviceUnlockAvailability;
    /// Whether `vault_id` is enrolled for `mode`.
    fn is_enrolled(&self, mode: DeviceUnlockMode, vault_id: &str) -> PlatformResult<bool>;
    /// Enrols `vault_id` for `mode` with its master password.
    fn enroll(
        &self,
        mode: DeviceUnlockMode,
        vault_id: &str,
        secret: &SecretString,
    ) -> PlatformResult<()>;
    /// Runs the device check (showing `reason`) and returns the enrolled master password.
    fn unlock(
        &self,
        mode: DeviceUnlockMode,
        vault_id: &str,
        reason: &str,
    ) -> PlatformResult<SecretString>;
    /// Removes every mode's enrolment of `vault_id`.
    fn forget(&self, vault_id: &str) -> PlatformResult<()>;
    /// Asks the device owner to prove presence (showing `reason`), with the policy that offers
    /// biometrics and falls back to the account password. Needs no Keychain item and no
    /// enrolment; returns no secret. The passkey ceremonies call it before every phase two
    /// (stage 5). Platforms without a body report `DeviceUnlockUnavailable`.
    fn verify_presence(&self, reason: &str) -> PlatformResult<()> {
        let _ = reason;
        Err(PlatformError::DeviceUnlockUnavailable)
    }
}
