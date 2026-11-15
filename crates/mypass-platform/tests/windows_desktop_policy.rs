//! Portable Windows policy tests: never open a native clipboard/window.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
use mypass_platform::{
    windows_desktop::*, ClipboardStamp, PlatformError, PlatformResult, SystemEvent::*,
};

#[test]
fn message_mapping_table_and_resume_suspend_does_not_double_fire() {
    for (message, expected) in [
        (Message::Session(7), Some(ScreenLocked)),
        (Message::Session(8), Some(ScreenUnlocked)),
        (Message::Session(4), Some(ScreenLocked)),
        (Message::Session(2), Some(ScreenLocked)),
        (Message::Session(6), Some(ScreenLocked)),
        (Message::Session(3), None),
        (Message::Session(1), None),
        (Message::Session(5), None),
        (Message::Session(99), None),
        (Message::Power(4), Some(Sleep)),
        (Message::Power(18), Some(Wake)),
        (Message::Power(7), None),
        (Message::Power(99), None),
        (Message::Display(0), Some(Sleep)),
        (Message::Display(1), Some(Wake)),
        (Message::Display(2), Some(Wake)),
        (Message::Display(3), None),
    ] {
        assert_eq!(map_message(message), expected);
    }
}

#[derive(Default)]
struct FakeClipboard {
    calls: Vec<String>,
    fail: Option<usize>,
    writes: usize,
    seq: u64,
    open: bool,
}
impl ClipboardTransaction for FakeClipboard {
    fn empty(&mut self) -> PlatformResult<()> {
        assert!(self.open);
        self.calls.push("empty".into());
        self.seq += 1;
        Ok(())
    }
    fn set(&mut self, name: &str, bytes: &[u8]) -> PlatformResult<()> {
        assert!(self.open);
        self.calls.push(format!("{name}:{bytes:?}"));
        self.writes += 1;
        if self.fail == Some(self.writes) {
            return Err(PlatformError::Failed("injected".into()));
        }
        self.seq += 1;
        Ok(())
    }
    fn sequence(&self) -> PlatformResult<u64> {
        assert!(self.open);
        Ok(self.seq)
    }
}
#[test]
fn clipboard_exclusions_precede_text_and_stamp_is_captured_before_close() {
    let mut tx = FakeClipboard {
        open: true,
        seq: 10,
        ..Default::default()
    };
    let stamp = write_clipboard(&mut tx, "A\u{1f512}").unwrap();
    assert_eq!(stamp, ClipboardStamp(16));
    assert_eq!(
        tx.calls,
        [
            "empty",
            "ExcludeClipboardContentFromMonitorProcessing:[0]",
            "CanIncludeInClipboardHistory:[0, 0, 0, 0]",
            "CanUploadToCloudClipboard:[0, 0, 0, 0]",
            "Clipboard Viewer Ignore:[]",
            "CF_UNICODETEXT:[65, 0, 61, 216, 18, 221, 0, 0]"
        ]
    );
}
#[test]
fn clipboard_failure_rolls_back_before_unlock_and_never_returns_a_stamp() {
    for failed_write in 1..=5 {
        let mut tx = FakeClipboard {
            open: true,
            fail: Some(failed_write),
            ..Default::default()
        };
        assert!(write_clipboard(&mut tx, "secret").is_err());
        assert_eq!(tx.calls.last().unwrap(), "empty");
        assert_eq!(tx.calls.len(), failed_write + 2);
    }
}
#[test]
fn clipboard_clear_checks_sequence_under_the_same_lock_and_preserves_foreign_copy() {
    let mut tx = FakeClipboard {
        open: true,
        seq: 42,
        ..Default::default()
    };
    assert!(!clear_clipboard(&mut tx, &ClipboardStamp(41)).unwrap());
    assert!(tx.calls.is_empty());
    assert!(clear_clipboard(&mut tx, &ClipboardStamp(42)).unwrap());
    assert_eq!(tx.calls, ["empty"]);
    assert!(!clear_clipboard(&mut tx, &ClipboardStamp(42)).unwrap());
}
#[test]
fn invalid_secret_does_not_erase_the_existing_clipboard() {
    let mut tx = FakeClipboard {
        open: true,
        ..Default::default()
    };
    assert!(write_clipboard(&mut tx, "a\0b").is_err());
    assert!(tx.calls.is_empty());
}
#[test]
fn layout_labels_are_short_ascii_and_poll_only_reports_changes() {
    for (lang, want) in [(0x0409, "EN"), (0x0422, "UK"), (0x0419, "RU")] {
        assert_eq!(layout_label(lang, None).as_deref(), Some(want));
    }
    assert_eq!(layout_label(0xffff, Some("pl")), Some("PL".into()));
    for hostile in [
        "C:\\secret",
        "password",
        "\u{0443}\u{043a}",
        "en-US",
        "1x",
        "",
    ] {
        assert_eq!(layout_label(0xffff, Some(hostile)), None);
    }
    let mut poll = LayoutPoll::new(Some("EN".into()));
    assert_eq!(poll.tick(Some("EN".into())), None);
    assert_eq!(poll.tick(Some("UK".into())), Some(Some("UK".into())));
    assert_eq!(poll.tick(None), Some(None));
    poll.cancel();
    assert_eq!(poll.tick(Some("RU".into())), None);
}

#[test]
fn clipboard_compare_and_clear_holds_the_lock_across_a_foreign_copy_barrier() {
    use std::sync::{Arc, Barrier, Mutex, MutexGuard};
    struct Locked<'a> {
        state: MutexGuard<'a, u64>,
        barrier: Arc<Barrier>,
    }
    impl ClipboardTransaction for Locked<'_> {
        fn sequence(&self) -> PlatformResult<u64> {
            self.barrier.wait();
            self.barrier.wait();
            Ok(*self.state)
        }
        fn empty(&mut self) -> PlatformResult<()> {
            *self.state += 1;
            Ok(())
        }
        fn set(&mut self, _: &str, _: &[u8]) -> PlatformResult<()> {
            unreachable!()
        }
    }
    let state = Arc::new(Mutex::new(42));
    let barrier = Arc::new(Barrier::new(2));
    let other_state = state.clone();
    let other_barrier = barrier.clone();
    let foreign = std::thread::spawn(move || {
        other_barrier.wait();
        assert!(
            other_state.try_lock().is_err(),
            "foreign copy cannot interleave the comparison and clear"
        );
        other_barrier.wait();
        *other_state.lock().unwrap() = 99;
    });
    {
        let mut tx = Locked {
            state: state.lock().unwrap(),
            barrier,
        };
        assert!(clear_clipboard(&mut tx, &ClipboardStamp(42)).unwrap());
    }
    foreign.join().unwrap();
    assert_eq!(*state.lock().unwrap(), 99);
}

#[test]
fn keyboard_layout_uses_our_foreground_ui_or_the_captured_ui_never_the_command_worker() {
    assert_eq!(layout_thread(10, 10, 42, 7), 42);
    assert_eq!(layout_thread(10, 11, 42, 7), 7);
    assert_eq!(layout_thread(10, 0, 0, 7), 7);
    assert_eq!(layout_thread(10, 10, 0, 7), 7);
}
