//! Which URLs a webview of the app may navigate to.
//!
//! Tauri 2 (2.12.5) does not restrict top-level navigation by itself: its navigation handler only
//! calls the plugin hooks, and the default of every hook is "allow". Vault data (notes, websites)
//! is attacker-controlled, so a link or a dropped URL must never replace the UI with a remote page.
//! The rule is a plain function over the URL; the plugin below is the thin glue that applies it to
//! every webview, including the ones created from `tauri.conf.json`.

use tauri::plugin::{Builder, TauriPlugin};
use tauri::{Manager, Runtime, Url};

/// `true` when a webview may load `url`: the app's own origin only.
///
/// The app origin is `tauri://localhost` (macOS, Linux), `http(s)://tauri.localhost` (Windows,
/// and the https variant of the scheme option), plus `about:blank`. In a debug build the dev
/// server origin (`devUrl`, the same scheme, host and port) is allowed too, for hot reload.
pub fn navigation_allowed(url: &Url, dev_url: Option<&Url>) -> bool {
    match url.scheme() {
        "tauri" => url.host_str() == Some("localhost"),
        "http" | "https" => {
            url.host_str() == Some("tauri.localhost")
                || dev_url.is_some_and(|dev| {
                    url.scheme() == dev.scheme()
                        && url.host_str() == dev.host_str()
                        && url.port_or_known_default() == dev.port_or_known_default()
                })
        }
        "about" => url.as_str() == "about:blank",
        _ => false,
    }
}

/// The plugin that refuses every navigation `navigation_allowed` rejects.
pub fn guard<R: Runtime>() -> TauriPlugin<R> {
    Builder::<R>::new("mypass-navigation-guard")
        .on_navigation(|webview, url| {
            let dev_url = if cfg!(debug_assertions) {
                webview.app_handle().config().build.dev_url.clone()
            } else {
                None
            };
            navigation_allowed(url, dev_url.as_ref())
        })
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    #[test]
    fn the_app_origin_is_allowed() {
        for s in [
            "tauri://localhost/index.html",
            "tauri://localhost/index.html#/settings",
            "http://tauri.localhost/index.html",
            "https://tauri.localhost/",
            "about:blank",
        ] {
            assert!(navigation_allowed(&url(s), None), "{s}");
        }
    }

    #[test]
    fn remote_and_other_schemes_are_refused() {
        for s in [
            "https://example.com/",
            "http://example.com/",
            "https://tauri.localhost.evil.com/",
            "http://evil.com/tauri.localhost",
            "tauri://evil/",
            "http://localhost:1420/",
            "file:///etc/passwd",
            "javascript:alert(1)",
            "data:text/html,<p>x</p>",
            "about:srcdoc",
            "blob:https://tauri.localhost/x",
        ] {
            assert!(!navigation_allowed(&url(s), None), "{s}");
        }
    }

    #[test]
    fn the_dev_server_origin_is_allowed_only_when_given() {
        let dev = url("http://localhost:1420");
        assert!(navigation_allowed(
            &url("http://localhost:1420/index.html"),
            Some(&dev)
        ));
        assert!(!navigation_allowed(
            &url("http://localhost:1421/"),
            Some(&dev)
        ));
        assert!(!navigation_allowed(
            &url("https://localhost:1420/"),
            Some(&dev)
        ));
        assert!(!navigation_allowed(
            &url("http://localhost.evil.com:1420/"),
            Some(&dev)
        ));
        assert!(!navigation_allowed(&url("http://localhost:1420/"), None));
    }
}
