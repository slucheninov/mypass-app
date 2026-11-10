//! The active keyboard layout, shown on the unlock screen so a password is not typed in the wrong
//! layout (D2).

use crate::{PlatformResult, Subscription};
use std::sync::Arc;

/// The active input source.
pub trait KeyboardLayout: Send + Sync {
    /// A short upper-case label such as `"EN"` or `"UK"`; `None` when unknown.
    fn current(&self) -> PlatformResult<Option<String>>;
    /// Delivers the new label to `sink` on every input-source change until the subscription drops.
    fn subscribe(
        &self,
        sink: Arc<dyn Fn(Option<String>) + Send + Sync>,
    ) -> PlatformResult<Subscription>;
}
