//! Portable Windows policy tests: never open a native clipboard/window.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
use mypass_platform::{windows_desktop::*, PlatformError, PlatformResult, SystemEvent::*};

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
    data: std::collections::BTreeMap<String, Vec<u8>>,
}
impl FakeClipboard {
    fn close(&mut self) {
        assert!(self.open);
        self.open = false;
        if self.data.contains_key("CF_UNICODETEXT") {
            // Windows synthesizes other text/locale formats on CloseClipboard.
            self.data.insert("CF_TEXT".into(), vec![0]);
            self.seq += 1;
        }
    }
    fn reopen(&mut self) {
        assert!(!self.open);
        self.open = true;
    }
}
impl ClipboardTransaction for FakeClipboard {
    fn empty(&mut self) -> PlatformResult<()> {
        assert!(self.open);
        self.calls.push("empty".into());
        self.data.clear();
        self.seq += 1;
        Ok(())
    }
    fn set(&mut self, name: &str, bytes: &[u8]) -> PlatformResult<()> {
        assert!(self.open);
        self.calls.push(format!("{name}:{bytes:?}"));
        self.writes += 1;
        // Win32 rejects discarded, zero-size HGLOBALs in SetClipboardData.
        if bytes.is_empty() {
            return Err(PlatformError::Failed(
                "zero-size clipboard allocation".into(),
            ));
        }
        if self.fail == Some(self.writes) {
            return Err(PlatformError::Failed("injected".into()));
        }
        self.seq += 1;
        self.data.insert(name.into(), bytes.to_vec());
        Ok(())
    }
    fn sequence(&self) -> PlatformResult<u64> {
        assert!(self.open);
        Ok(self.seq)
    }
    fn has_format(&self, format: &str) -> PlatformResult<bool> {
        assert!(self.open);
        Ok(self.data.contains_key(format))
    }
    fn read_unicode_text(&self) -> PlatformResult<Option<zeroize::Zeroizing<Vec<u8>>>> {
        assert!(self.open);
        Ok(self
            .data
            .get("CF_UNICODETEXT")
            .and_then(|text| bounded_unicode_text(text)))
    }
}
#[test]
fn clipboard_exclusions_use_nonzero_allocations_and_precede_utf16_text() {
    let mut tx = FakeClipboard {
        open: true,
        seq: 10,
        ..Default::default()
    };
    let stamp = write_clipboard(&mut tx, "A\u{1f512}").unwrap();
    assert_eq!(stamp.stamp.0, 16);
    assert_eq!(
        tx.calls,
        [
            "empty",
            "ExcludeClipboardContentFromMonitorProcessing:[0]",
            "CanIncludeInClipboardHistory:[0, 0, 0, 0]",
            "CanUploadToCloudClipboard:[0, 0, 0, 0]",
            "Clipboard Viewer Ignore:[0]",
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
        assert!(tx.data.is_empty());
    }
}
#[test]
fn clipboard_clear_survives_close_time_synthesis() {
    let mut tx = FakeClipboard {
        open: true,
        ..Default::default()
    };
    let stamp = write_clipboard(&mut tx, "secret").unwrap();
    tx.close();
    tx.reopen();
    assert!(clear_clipboard(&mut tx, &stamp).unwrap());
    assert!(tx.data.is_empty());
    assert!(!clear_clipboard(&mut tx, &stamp).unwrap());
}
#[test]
fn clipboard_clear_preserves_foreign_text_even_with_our_marker_and_sequence() {
    let mut tx = FakeClipboard {
        open: true,
        ..Default::default()
    };
    let stamp = write_clipboard(&mut tx, "secret").unwrap();
    let seq = tx.seq;
    tx.close();
    tx.reopen();
    tx.set("CF_UNICODETEXT", &[70, 0, 0, 0]).unwrap();
    tx.seq = seq; // Sequence wrap/equality must never authorize a clear by itself.
    assert!(!clear_clipboard(&mut tx, &stamp).unwrap());
    assert_eq!(tx.data["CF_UNICODETEXT"], [70, 0, 0, 0]);
}
#[test]
fn clipboard_clear_preserves_identical_foreign_text_without_our_marker() {
    let mut tx = FakeClipboard {
        open: true,
        ..Default::default()
    };
    let stamp = write_clipboard(&mut tx, "secret").unwrap();
    let seq = tx.seq;
    let text = tx.data["CF_UNICODETEXT"].clone();
    tx.close();
    tx.reopen();
    tx.empty().unwrap();
    tx.set("CF_UNICODETEXT", &text).unwrap();
    tx.seq = seq;
    assert!(!clear_clipboard(&mut tx, &stamp).unwrap());
    assert_eq!(tx.data["CF_UNICODETEXT"], text);
}
#[test]
fn clipboard_clear_rejects_unterminated_odd_or_oversized_readback() {
    let mut oversized = vec![0; MAX_CLIPBOARD_BYTES + 2];
    oversized[0] = 65; // Matching text plus excess padding must still be refused.
    for malformed in [vec![65, 0], vec![65, 0, 0], oversized] {
        let mut tx = FakeClipboard {
            open: true,
            ..Default::default()
        };
        let written = write_clipboard(&mut tx, "A").unwrap();
        tx.set("CF_UNICODETEXT", &malformed).unwrap();
        assert!(!clear_clipboard(&mut tx, &written).unwrap());
        assert_eq!(tx.data["CF_UNICODETEXT"], malformed);
    }
}
#[test]
fn clipboard_clear_ignores_hglobal_padding_after_the_utf16_terminator() {
    let mut tx = FakeClipboard {
        open: true,
        ..Default::default()
    };
    let written = write_clipboard(&mut tx, "A").unwrap();
    tx.set("CF_UNICODETEXT", &[65, 0, 0, 0, 99, 99, 99, 99])
        .unwrap();
    assert!(clear_clipboard(&mut tx, &written).unwrap());
    assert!(tx.data.is_empty());
}
#[test]
fn oversized_secret_is_rejected_before_destroying_the_previous_clipboard() {
    let mut tx = FakeClipboard {
        open: true,
        ..Default::default()
    };
    let written = write_clipboard(&mut tx, "previous").unwrap();
    assert!(write_clipboard(&mut tx, &"A".repeat(MAX_CLIPBOARD_BYTES / 2)).is_err());
    assert!(clear_clipboard(&mut tx, &written).unwrap());
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
        state: MutexGuard<'a, FakeClipboard>,
        barrier: Arc<Barrier>,
    }
    impl ClipboardTransaction for Locked<'_> {
        fn sequence(&self) -> PlatformResult<u64> {
            self.state.sequence()
        }
        fn has_format(&self, format: &str) -> PlatformResult<bool> {
            self.state.has_format(format)
        }
        fn read_unicode_text(&self) -> PlatformResult<Option<zeroize::Zeroizing<Vec<u8>>>> {
            let text = self.state.read_unicode_text()?;
            self.barrier.wait();
            self.barrier.wait();
            Ok(text)
        }
        fn empty(&mut self) -> PlatformResult<()> {
            self.state.empty()
        }
        fn set(&mut self, _: &str, _: &[u8]) -> PlatformResult<()> {
            unreachable!()
        }
    }
    let mut initial = FakeClipboard {
        open: true,
        ..Default::default()
    };
    let written = write_clipboard(&mut initial, "secret").unwrap();
    let state = Arc::new(Mutex::new(initial));
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
        let mut tx = other_state.lock().unwrap();
        tx.empty().unwrap();
        tx.set("CF_UNICODETEXT", &[70, 0, 0, 0]).unwrap();
    });
    {
        let mut tx = Locked {
            state: state.lock().unwrap(),
            barrier,
        };
        assert!(clear_clipboard(&mut tx, &written).unwrap());
    }
    foreign.join().unwrap();
    assert_eq!(state.lock().unwrap().data["CF_UNICODETEXT"], [70, 0, 0, 0]);
}

#[test]
fn keyboard_layout_uses_our_foreground_ui_or_the_captured_ui_never_the_command_worker() {
    assert_eq!(layout_thread(10, 10, 42, 7), 42);
    assert_eq!(layout_thread(10, 11, 42, 7), 7);
    assert_eq!(layout_thread(10, 0, 0, 7), 7);
    assert_eq!(layout_thread(10, 10, 0, 7), 7);
}
