//! The update check (stage 5): semantic versions, the release feed, the update state store and the
//! pure update policy, plus the GitHub release client behind the `github` feature.
//!
//! Depends on no MyPass crate: the session owns the scheduling side (`UpdateService`), so there is
//! no dependency cycle. The modules land in later stage 5 tasks; this is the scaffold.
