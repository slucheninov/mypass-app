//! The native messaging host: relays Chromium's stdin and stdout to the app's loopback server.
//!
//! It builds only the paths and the shared file store, never the full platform object set (no AppKit,
//! clipboard or Keychain objects in a process Chromium spawns per port). Stdout carries frames only
//! and the exit code is always 0, because Chromium treats anything else as a crash.

use std::io::{stdin, stdout};
use std::sync::Arc;

use mypass_bridge::endpoint::EndpointStore;
use mypass_bridge::relay::{load_endpoint, run_relay, RelayOptions};
use mypass_platform::{shared_store, AppPaths};

fn main() {
    let endpoint = AppPaths::resolve().ok().and_then(|paths| {
        let store = EndpointStore::new(Arc::new(shared_store(&paths)));
        load_endpoint(&store)
    });
    let _ = run_relay(stdin().lock(), stdout(), endpoint, &RelayOptions::default());
}
