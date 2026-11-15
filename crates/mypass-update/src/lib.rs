//! The update check (stage 5): semantic versions, the release feed, the update state store and the
//! pure update policy, plus the GitHub release client behind the `github` feature.
//!
//! Depends on no MyPass crate: the session owns the scheduling side (`UpdateService`: scheduler,
//! settings, observers, the in-flight state), so there is no dependency cycle. This crate only
//! decides ([`UpdatePolicy`]), remembers ([`UpdateStateStore`]) and fetches (a [`ReleaseFeed`]).
//! Nothing here downloads or installs anything.
//!
//! The HTTP client is the `github` feature (default off), enabled by `src-tauri` alone, so no other
//! crate and never `mypass-host` links it.

mod feed;
mod policy;
mod state;
mod version;

#[cfg(feature = "github")]
mod github;

pub use feed::{
    parse_release_payload, validate_release_url, NoopFeed, ReleaseFeed, ReleaseFeedError,
    ReleaseInfo, REPOSITORY,
};
#[cfg(feature = "github")]
pub use github::{GitHubReleaseClient, MAX_BODY_BYTES, REQUEST_TIMEOUT};
pub use policy::{
    BannerChange, BeginDecision, CheckOutcome, EnabledChange, FinishInput, UpdateErrorKind,
    UpdatePolicy, UpdateStatus, INITIAL_DELAY, INTERVAL,
};
pub use state::{UpdateState, UpdateStateError, UpdateStateStore};
pub use version::{SemanticVersion, VersionParseError};
