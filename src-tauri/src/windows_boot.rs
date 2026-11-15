//! Safe, injectable startup policy; native dialogs belong to the platform crate.
/// Injectable loader check; the caller owns a clean exit before constructing Tauri.
pub fn ensure_webview2(
    detect: impl FnOnce() -> Option<String>,
    show: impl FnOnce(&str, &str),
) -> bool {
    if detect().is_some_and(|version| !version.trim().is_empty() && version.trim() != "0.0.0.0") {
        return true;
    }
    let message = crate::i18n::t_resolved(crate::i18n::Language::En,
        "MyPass needs the Microsoft Edge WebView2 Runtime, which is missing on this PC. Install it from https://go.microsoft.com/fwlink/p/?LinkId=2124703 and start MyPass again.", &[]);
    show("MyPass", &message);
    false
}

/// Safe loader API, rather than a second registry-based runtime detector.
#[cfg(windows)]
pub fn preflight() -> bool {
    ensure_webview2(
        || tauri::webview_version().ok(),
        mypass_platform::show_startup_error,
    )
}

/// Only an actually available tray makes a hidden autostart recoverable.
pub fn show_main_on_start(args: &[String], tray_available: bool) -> bool {
    !(tray_available && args.iter().skip(1).any(|arg| arg == "--autostarted"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn webview2_missing_reports_message_and_exits_cleanly() {
        let shown = std::cell::RefCell::new(Vec::new());
        assert!(!ensure_webview2(
            || None,
            |title, text| shown.borrow_mut().push((title.to_owned(), text.to_owned()))
        ));
        assert_eq!(shown.borrow()[0].0, "MyPass");
        assert!(shown.borrow()[0]
            .1
            .contains("https://go.microsoft.com/fwlink/p/?LinkId=2124703"));
        for version in ["", "0.0.0.0"] {
            assert!(!ensure_webview2(|| Some(version.into()), |_, _| {}));
        }
        assert!(ensure_webview2(
            || Some("140.1.2.3".into()),
            |_, _| panic!("unexpected dialog")
        ));
    }
    #[test]
    fn autostarted_flag_hides_main_window_only_when_tray_is_available() {
        for autostarted in [false, true] {
            for tray in [false, true] {
                let args = if autostarted {
                    vec!["MyPass.exe".to_owned(), "--autostarted".to_owned()]
                } else {
                    vec!["MyPass.exe".to_owned()]
                };
                assert_eq!(show_main_on_start(&args, tray), !(autostarted && tray));
            }
        }
    }
}
