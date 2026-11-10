//! The one error type of every platform trait.

/// Errors of the platform layer.
///
/// The device-unlock outcomes (`Cancelled` .. `DeviceUnlockUnavailable`) are distinct variants so
/// stage 5 can map LocalAuthentication / Keychain codes onto them and the fakes can raise every one,
/// which keeps the `AppCore` mapping and the UI testable without a Mac. No variant ever carries a
/// secret: `Failed` holds an OS error description only.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PlatformError {
    /// The operation has no implementation on this platform (yet).
    #[error("unsupported on this platform: {0}")]
    Unsupported(&'static str),
    /// The OS call failed; the text is the OS error description.
    #[error("platform error: {0}")]
    Failed(String),
    /// The user dismissed the device-unlock prompt.
    #[error("cancelled by the user")]
    Cancelled,
    /// Device unlock is locked out after too many failed attempts.
    #[error("too many attempts")]
    Lockout,
    /// The enrolment no longer matches the device (for example a changed fingerprint set).
    #[error("device unlock enrolment is no longer valid")]
    Invalidated,
    /// Neither biometrics nor a device password is set up.
    #[error("no biometrics or device password set up on this device")]
    NotEnrolledOnDevice,
    /// Device unlock cannot be used here at all.
    #[error("device unlock is not available here")]
    DeviceUnlockUnavailable,
}

/// Result alias of the platform layer.
pub type PlatformResult<T> = Result<T, PlatformError>;
