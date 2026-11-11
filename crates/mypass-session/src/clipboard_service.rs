//! Copying secrets: a concealed write, then a clear after `clipboardClearSeconds` that never wipes
//! something the user copied afterwards.
//!
//! The clear is always scheduled: the setting is 10..=120 s in steps of 5 and has no "never"
//! (the old app's stepper, decision D3). A second copy cancels the first timer. The clear goes
//! through `SecureClipboard::clear_if_unchanged`, so a later copy by the user or another app is
//! left alone (that check has a documented, accepted compare-then-clear window of a few
//! microseconds). The copied text is never stored, logged or `Debug`-printed here.
//!
//! The scheduled clear runs on the scheduler's thread. The service's `pending` mutex is held
//! while writing and scheduling (so a copy and its timer are recorded together) but never while
//! clearing or calling the sink.

use crate::scheduler::{lock, Scheduler, TaskHandle};
use crate::settings::{SettingsStore, CLIPBOARD_CLEAR_SECONDS};
use mypass_platform::{ClipboardStamp, PlatformResult, SecureClipboard};
use std::sync::{Arc, Mutex, PoisonError, RwLock, Weak};
use std::time::Duration;

/// Told when a scheduled (or quit-time) clear actually cleared our secret; `src-tauri` turns it
/// into `ClipboardClearedEvent`. Must be cheap and must not call back into the service.
pub trait ClearedSink: Send + Sync {
    fn cleared(&self);
}

struct Inner {
    clipboard: Arc<dyn SecureClipboard>,
    scheduler: Arc<dyn Scheduler>,
    settings: SettingsStore,
    pending: Mutex<Option<(ClipboardStamp, TaskHandle)>>,
    sink: RwLock<Option<Arc<dyn ClearedSink>>>,
}

/// The clipboard service (cheap to clone; every clone is the same service).
#[derive(Clone)]
pub struct ClipboardService(Arc<Inner>);

impl std::fmt::Debug for ClipboardService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClipboardService")
            .field("clear_pending", &lock(&self.0.pending).is_some())
            .finish_non_exhaustive()
    }
}

impl ClipboardService {
    /// A service with no cleared sink yet (see [`ClipboardService::set_cleared_sink`]).
    pub fn new(
        clipboard: Arc<dyn SecureClipboard>,
        scheduler: Arc<dyn Scheduler>,
        settings: SettingsStore,
    ) -> Self {
        Self(Arc::new(Inner {
            clipboard,
            scheduler,
            settings,
            pending: Mutex::new(None),
            sink: RwLock::new(None),
        }))
    }

    /// Sets (or replaces) the sink told about every clear of our secret. Separate from `new`
    /// because the shell creates its event emitter after the core services.
    pub fn set_cleared_sink(&self, sink: Arc<dyn ClearedSink>) {
        *self.0.sink.write().unwrap_or_else(PoisonError::into_inner) = Some(sink);
    }

    fn notify(&self) {
        let sink = self
            .0
            .sink
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        if let Some(sink) = sink {
            sink.cleared();
        }
    }

    fn clear_after(&self) -> Duration {
        let secs = self.0.settings.get().clipboard_clear_seconds.clamp(
            *CLIPBOARD_CLEAR_SECONDS.start(),
            *CLIPBOARD_CLEAR_SECONDS.end(),
        );
        Duration::from_secs(u64::from(secs))
    }

    /// Writes `text` (concealed) and schedules its clear after `clipboardClearSeconds`,
    /// cancelling the timer of a previous copy. A failed write is returned and schedules nothing
    /// (a previous copy keeps its timer).
    pub fn copy_secret(&self, text: &str) -> PlatformResult<()> {
        let after = self.clear_after();
        let mut pending = lock(&self.0.pending);
        let stamp = self.0.clipboard.write_secret(text)?;
        let weak: Weak<Inner> = Arc::downgrade(&self.0);
        let job_stamp = stamp.clone();
        let handle = self.0.scheduler.schedule(
            after,
            Box::new(move || {
                if let Some(inner) = weak.upgrade() {
                    ClipboardService(inner).clear_if_current(&job_stamp);
                }
            }),
        );
        if let Some((_, previous)) = pending.replace((stamp, handle)) {
            previous.cancel();
        }
        Ok(())
    }

    /// The timer: clears only when `stamp` is still our latest copy (a cancelled job may still
    /// run once) and the clipboard still holds it.
    fn clear_if_current(&self, stamp: &ClipboardStamp) {
        let ours = {
            let mut pending = lock(&self.0.pending);
            match &*pending {
                Some((s, _)) if s == stamp => pending.take(),
                _ => None,
            }
        };
        if ours.is_some() {
            self.clear(stamp);
        }
    }

    fn clear(&self, stamp: &ClipboardStamp) {
        if let Ok(true) = self.0.clipboard.clear_if_unchanged(stamp) {
            self.notify();
        }
    }

    /// Quit path, best effort: clears our latest secret now if nobody copied anything since,
    /// and cancels its timer.
    pub fn clear_now_if_ours(&self) {
        let taken = lock(&self.0.pending).take();
        if let Some((stamp, handle)) = taken {
            handle.cancel();
            self.clear(&stamp);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scheduler::ManualScheduler;
    use crate::settings::SettingsPatch;
    use mypass_platform::testing::FakeClipboard;
    use mypass_platform::PlatformError;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::UNIX_EPOCH;

    #[derive(Default)]
    struct Counter(AtomicUsize);
    impl ClearedSink for Counter {
        fn cleared(&self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    impl Counter {
        fn get(&self) -> usize {
            self.0.load(Ordering::SeqCst)
        }
    }

    struct Fx {
        clipboard: Arc<FakeClipboard>,
        clock: Arc<ManualScheduler>,
        settings: SettingsStore,
        sink: Arc<Counter>,
        service: ClipboardService,
    }

    fn fx() -> Fx {
        let clipboard = FakeClipboard::new();
        let clock = ManualScheduler::new(UNIX_EPOCH + Duration::from_secs(1_700_000_000));
        let settings = SettingsStore::in_memory();
        let service = ClipboardService::new(clipboard.clone(), clock.clone(), settings.clone());
        let sink = Arc::new(Counter::default());
        service.set_cleared_sink(sink.clone());
        Fx {
            clipboard,
            clock,
            settings,
            sink,
            service,
        }
    }

    impl Fx {
        fn advance(&self, secs: u64) {
            self.clock.advance(Duration::from_secs(secs));
        }
        fn seconds(&self, s: u32) {
            self.settings
                .update(SettingsPatch {
                    clipboard_clear_seconds: Some(s),
                    ..SettingsPatch::default()
                })
                .unwrap();
        }
    }

    #[test]
    fn copy_clears_after_configured_seconds() {
        let f = fx();
        f.service.copy_secret("pw").unwrap();
        f.advance(29);
        assert_eq!(f.clipboard.contents().as_deref(), Some("pw"));
        assert_eq!(f.sink.get(), 0);
        f.advance(1);
        assert_eq!(f.clipboard.contents(), None);
        assert_eq!(f.sink.get(), 1);
        f.advance(300);
        assert_eq!(f.sink.get(), 1);
    }

    #[test]
    fn clear_skips_when_clipboard_changed_since() {
        let f = fx();
        f.service.copy_secret("pw").unwrap();
        f.advance(10);
        f.clipboard.user_copies("other");
        f.advance(21);
        assert_eq!(f.clipboard.contents().as_deref(), Some("other"));
        assert_eq!(f.sink.get(), 0);
    }

    #[test]
    fn second_copy_cancels_first_timer() {
        let f = fx();
        f.service.copy_secret("one").unwrap();
        f.advance(20);
        f.service.copy_secret("two").unwrap();
        f.advance(10);
        assert_eq!(
            f.clipboard.contents().as_deref(),
            Some("two"),
            "not at 30 s"
        );
        f.advance(20);
        assert_eq!(f.clipboard.contents(), None, "at 50 s");
        assert_eq!(f.sink.get(), 1);
    }

    #[test]
    fn shortest_setting_ten_seconds_still_clears() {
        let f = fx();
        f.seconds(10);
        f.service.copy_secret("pw").unwrap();
        f.advance(9);
        assert_eq!(f.clipboard.contents().as_deref(), Some("pw"));
        f.advance(1);
        assert_eq!(f.clipboard.contents(), None);
    }

    #[test]
    fn longest_setting_120_seconds_clears_at_120() {
        let f = fx();
        f.seconds(120);
        f.service.copy_secret("pw").unwrap();
        f.advance(119);
        assert_eq!(f.clipboard.contents().as_deref(), Some("pw"));
        f.advance(1);
        assert_eq!(f.clipboard.contents(), None);
    }

    #[test]
    fn clear_now_if_ours_respects_change() {
        let f = fx();
        f.service.copy_secret("pw").unwrap();
        f.clipboard.user_copies("other");
        f.service.clear_now_if_ours();
        assert_eq!(f.clipboard.contents().as_deref(), Some("other"));
        assert_eq!(f.sink.get(), 0);
        f.service.copy_secret("pw").unwrap();
        f.service.clear_now_if_ours();
        assert_eq!(f.clipboard.contents(), None);
        assert_eq!(f.sink.get(), 1);
        assert_eq!(f.clock.pending(), 0, "the timer is cancelled");
        f.service.clear_now_if_ours();
        assert_eq!(f.sink.get(), 1, "nothing of ours left");
    }

    struct BrokenClipboard;
    impl SecureClipboard for BrokenClipboard {
        fn write_secret(&self, _: &str) -> PlatformResult<ClipboardStamp> {
            Err(PlatformError::Failed("pasteboard unavailable".into()))
        }
        fn clear_if_unchanged(&self, _: &ClipboardStamp) -> PlatformResult<bool> {
            Ok(false)
        }
    }

    #[test]
    fn copy_failure_is_returned_and_no_timer_scheduled() {
        let clock = ManualScheduler::new(UNIX_EPOCH);
        let service = ClipboardService::new(
            Arc::new(BrokenClipboard),
            clock.clone(),
            SettingsStore::in_memory(),
        );
        assert_eq!(
            service.copy_secret("pw"),
            Err(PlatformError::Failed("pasteboard unavailable".into()))
        );
        assert_eq!(clock.pending(), 0);
    }

    #[test]
    fn works_without_a_sink() {
        let clipboard = FakeClipboard::new();
        let clock = ManualScheduler::new(UNIX_EPOCH);
        let service =
            ClipboardService::new(clipboard.clone(), clock.clone(), SettingsStore::in_memory());
        service.copy_secret("pw").unwrap();
        clock.advance(Duration::from_secs(30));
        assert_eq!(clipboard.contents(), None);
    }

    #[test]
    fn debug_never_shows_the_secret() {
        let f = fx();
        f.service.copy_secret("hunter2").unwrap();
        let text = format!("{:?}", f.service);
        assert!(!text.contains("hunter2"), "{text}");
        assert!(text.contains("clear_pending: true"), "{text}");
    }
}
