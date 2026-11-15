//! Native messaging relay; no session, core, KDBX or update dependency.
//! Keep the console subsystem. Chromium hides the native host window when launching it
//! with redirected pipes (launch_context_win.cc, formerly native_process_launcher_win.cc). Browser arguments are ignored.
//! Rust uses ReadFile/WriteFile directly for these pipes, without CRT text translation;
//! no _setmode is needed. The relay explicitly flushes every complete response frame.
use mypass_bridge::endpoint::EndpointStore;
use mypass_bridge::relay::{load_endpoint, run_relay_verified, RelayOptions};
use mypass_platform::endpoint_store::{host_shared_store, running_store_kind, StoreSide};
use mypass_platform::AppPaths;
use std::io::{stdin, stdout};
fn main() {
    let loaded = AppPaths::resolve().ok().map(|paths| {
        let (kind, store) = host_shared_store(&paths, None);
        (kind, load_endpoint(&EndpointStore::new(store)))
    });
    let (kind, endpoint) = loaded.unwrap_or((running_store_kind(StoreSide::Host), None));
    let pid = endpoint.as_ref().map_or(0, |record| record.pid);
    let _ = run_relay_verified(
        stdin().lock(),
        stdout(),
        endpoint,
        &RelayOptions::default(),
        &|port| mypass_platform::peer_process::endpoint_peer_is_trusted(kind, port, pid),
    );
}
