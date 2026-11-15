//! Native messaging relay; no session, core, KDBX or update dependency.
use mypass_bridge::endpoint::EndpointStore;
use mypass_bridge::relay::{load_endpoint, run_relay_verified, RelayOptions};
use mypass_platform::endpoint_store::{host_shared_store, SharedStoreKind};
use mypass_platform::AppPaths;
use std::io::{stdin, stdout};
fn main() {
    let loaded = AppPaths::resolve().ok().map(|paths| {
        let (kind, store) = host_shared_store(&paths, None);
        (kind, load_endpoint(&EndpointStore::new(store)))
    });
    let (kind, endpoint) = loaded.unwrap_or((SharedStoreKind::File, None));
    let _ = run_relay_verified(
        stdin().lock(),
        stdout(),
        endpoint,
        &RelayOptions::default(),
        &|port| {
            kind == SharedStoreKind::File
                || mypass_platform::peer_process::signed_app_listens_on(port)
        },
    );
}
