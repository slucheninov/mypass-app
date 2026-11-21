//! Handles the one native message that needs no connection to the app.

use std::borrow::Cow;
use std::io::{Read, Write};

use serde::Deserialize;
use zeroize::Zeroizing;

use crate::codec::read_frame;
use crate::endpoint::Endpoint;
use crate::relay::{
    connect_verified, error_frame, run_relay_with_first_frame, RelayOptions, RelayOutcome,
};

pub const LAUNCH_APP: &str = "launchApp";
pub const LAUNCH_UNAVAILABLE: &str = "launch_unavailable";
pub const LAUNCH_FAILED: &str = "launch_failed";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchResult {
    Started,
    Unavailable,
    Failed,
}

#[derive(Deserialize)]
struct MessageType<'a> {
    #[serde(borrow, rename = "type")]
    kind: Cow<'a, str>,
}

pub fn run_host<R: Read, W: Write + Send + 'static>(
    mut stdin: R,
    mut stdout: W,
    endpoint: Option<Endpoint>,
    opts: &RelayOptions,
    verify_peer: &dyn Fn(u16) -> bool,
    launch: &dyn Fn() -> LaunchResult,
) -> RelayOutcome {
    let first = read_frame(&mut stdin, opts.max_frame).map(|frame| frame.map(Zeroizing::new));
    let is_launch = first
        .as_ref()
        .ok()
        .and_then(Option::as_ref)
        .is_some_and(|frame| {
            // Read the type alone so an ordinary request does not make another copy of its fields.
            serde_json::from_slice::<MessageType<'_>>(frame)
                .is_ok_and(|message| message.kind == LAUNCH_APP)
        });
    if !is_launch {
        return run_relay_with_first_frame(stdin, stdout, endpoint, opts, verify_peer, Some(first));
    }

    let running = endpoint.is_some_and(|record| {
        connect_verified(record.port, opts.connect_timeout, verify_peer).is_some()
    });
    let response = if running {
        launch_reply(br#"{"type":"launchNotNeeded"}"#)
    } else {
        match launch() {
            LaunchResult::Started => launch_reply(br#"{"type":"launchStarted"}"#),
            LaunchResult::Unavailable => error_frame(LAUNCH_UNAVAILABLE),
            LaunchResult::Failed => error_frame(LAUNCH_FAILED),
        }
    };
    let _ = stdout.write_all(&response).and_then(|()| stdout.flush());
    RelayOutcome::HostHandled
}

fn launch_reply(body: &[u8]) -> Vec<u8> {
    crate::codec::encode_frame(body).unwrap_or_default()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)] // test-only
mod tests {
    use super::*;
    use crate::codec::{encode_frame, MAX_FRAME_BYTES};
    use crate::endpoint::{generate_token, hello_proof, Handshake};
    use crate::relay::{inject_token, run_relay};
    use serde_json::{json, Value};
    use std::net::{Ipv4Addr, TcpListener};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    #[derive(Clone, Default)]
    struct Output(Arc<Mutex<Vec<u8>>>);

    impl Write for Output {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().write(bytes)
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn frame(body: &str) -> Vec<u8> {
        encode_frame(body.as_bytes()).unwrap()
    }

    fn answer(out: &Output) -> Value {
        let bytes = out.0.lock().unwrap();
        let body = read_frame(&mut bytes.as_slice(), MAX_FRAME_BYTES)
            .unwrap()
            .unwrap();
        serde_json::from_slice(&body).unwrap()
    }

    fn endpoint(listener: &TcpListener) -> Endpoint {
        Endpoint {
            port: listener.local_addr().unwrap().port(),
            token: generate_token(),
            pid: std::process::id(),
            started_at: 1,
        }
    }

    fn launch_test(
        endpoint: Option<Endpoint>,
        verify: bool,
        result: LaunchResult,
    ) -> (Value, usize) {
        let out = Output::default();
        let calls = std::cell::Cell::new(0);
        let input = frame(r#"{"type":"launchApp"}"#);
        let outcome = run_host(
            input.as_slice(),
            out.clone(),
            endpoint,
            &RelayOptions::default(),
            &|_| verify,
            &|| {
                calls.set(calls.get() + 1);
                result
            },
        );
        assert_eq!(outcome, RelayOutcome::HostHandled);
        (answer(&out), calls.get())
    }

    #[test]
    fn launch_app_with_no_endpoint_launches_once() {
        assert_eq!(
            launch_test(None, true, LaunchResult::Started),
            (json!({"type":"launchStarted"}), 1)
        );
    }

    #[test]
    fn launch_app_with_live_endpoint_answers_not_needed_without_spawning() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        assert_eq!(
            launch_test(Some(endpoint(&listener)), true, LaunchResult::Failed),
            (json!({"type":"launchNotNeeded"}), 0)
        );
    }

    #[test]
    fn launch_app_with_untrusted_peer_launches() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        assert_eq!(
            launch_test(Some(endpoint(&listener)), false, LaunchResult::Started),
            (json!({"type":"launchStarted"}), 1)
        );
    }

    #[test]
    fn launch_results_map_to_error_messages() {
        for (status, message) in [
            (LaunchResult::Unavailable, "launch_unavailable"),
            (LaunchResult::Failed, "launch_failed"),
        ] {
            assert_eq!(
                launch_test(None, true, status),
                (json!({"type":"error","message":message}), 1)
            );
        }
    }

    fn responding_peer(
        token: &str,
        response: &'static str,
    ) -> (Endpoint, std::thread::JoinHandle<Option<Vec<u8>>>) {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let mut endpoint = endpoint(&listener);
        endpoint.token = token.to_owned();
        let token = endpoint.token.clone();
        let peer = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let hello = read_frame(&mut stream, MAX_FRAME_BYTES).unwrap().unwrap();
            let Handshake::Hello { nonce } = serde_json::from_slice(&hello).unwrap() else {
                panic!("expected hello");
            };
            stream
                .write_all(&frame(
                    &json!({"type":"helloAck","proof":hello_proof(&token, &nonce)}).to_string(),
                ))
                .unwrap();
            let request = read_frame(&mut stream, MAX_FRAME_BYTES).unwrap();
            if request.is_some() {
                stream.write_all(&frame(response)).unwrap();
            }
            request
        });
        (endpoint, peer)
    }

    #[test]
    fn non_launch_first_frame_is_relayed_unchanged() {
        let token = generate_token();
        let (endpoint, peer) = responding_peer(&token, r#"{"type":"locked"}"#);
        let body = r#"{"type":"getCredentials","url":"https://example.test","pairing":"proof"}"#;
        let out = Output::default();
        let input = frame(body);
        let outcome = run_host(
            input.as_slice(),
            out.clone(),
            Some(endpoint),
            &RelayOptions::default(),
            &|_| true,
            &|| panic!("ordinary request launched"),
        );
        assert_eq!(outcome, RelayOutcome::StdinClosed);
        assert_eq!(answer(&out), json!({"type":"locked"}));
        assert_eq!(
            peer.join().unwrap(),
            Some(inject_token(body.as_bytes(), &token).unwrap())
        );
    }

    #[test]
    fn stdin_closed_before_first_frame_answers_app_not_running() {
        let out = Output::default();
        assert_eq!(
            run_host(
                &[][..],
                out.clone(),
                None,
                &RelayOptions::default(),
                &|_| true,
                &|| panic!("no request")
            ),
            RelayOutcome::AppNotRunning
        );
        assert_eq!(
            answer(&out),
            json!({"type":"error","message":"app_not_running"})
        );
    }

    #[test]
    fn malformed_first_frame_goes_to_the_relay() {
        let check = |host_local| {
            let token = generate_token();
            let (endpoint, peer) = responding_peer(&token, "");
            let out = Output::default();
            let input = frame("invalid JSON");
            let opts = RelayOptions::default();
            let outcome = if host_local {
                run_host(
                    input.as_slice(),
                    out.clone(),
                    Some(endpoint),
                    &opts,
                    &|_| true,
                    &|| panic!("malformed request launched"),
                )
            } else {
                run_relay(input.as_slice(), out.clone(), Some(endpoint), &opts)
            };
            assert_eq!(outcome, RelayOutcome::StdinClosed);
            assert_eq!(peer.join().unwrap(), None);
            answer(&out)
        };
        assert_eq!(check(true), check(false));
    }
}
