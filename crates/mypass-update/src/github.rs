//! The GitHub release client (feature `github`, off by default; only `src-tauri` enables it).
//!
//! The only file in the workspace that talks HTTP. A plain unauthenticated `GET` of the latest
//! release: nothing about the user, the vault or the machine is sent beyond the app version in the
//! `User-Agent` header that GitHub requires. TLS is the operating system's (Security.framework on
//! macOS, SChannel on Windows) and trusts the OS root store; no CA list is bundled.

use crate::feed::{parse_release_payload, ReleaseFeed, ReleaseFeedError, ReleaseInfo, REPOSITORY};
use std::time::Duration;
use ureq::config::Config;
use ureq::tls::{RootCerts, TlsConfig, TlsProvider};
use ureq::{Agent, Error};

/// The whole request (connect, send, receive) is abandoned after this long.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// A release answer larger than this is refused; the real one is a few kilobytes of JSON.
pub const MAX_BODY_BYTES: u64 = 512 * 1024;

const API_HOST: &str = "https://api.github.com";
const ACCEPT: &str = "application/vnd.github+json";
const API_VERSION: &str = "2022-11-28";

/// Asks the GitHub REST API for the latest published release.
pub struct GitHubReleaseClient {
    agent: Agent,
    endpoint: String,
}

impl std::fmt::Debug for GitHubReleaseClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GitHubReleaseClient")
            .field("endpoint", &self.endpoint)
            .finish_non_exhaustive()
    }
}

impl GitHubReleaseClient {
    /// The client for `https://api.github.com/repos/slucheninov/mypass-app/releases/latest`.
    /// `app_version` goes into `User-Agent: MyPass/<version>`. HTTPS only, no redirects, no
    /// cookies, no cache, a [`REQUEST_TIMEOUT`] deadline.
    pub fn new(app_version: &str) -> Self {
        Self::build(
            format!("{API_HOST}/repos/{REPOSITORY}/releases/latest"),
            app_version,
            Settings::production(),
        )
    }

    fn build(endpoint: String, app_version: &str, settings: Settings) -> Self {
        let tls = TlsConfig::builder()
            .provider(TlsProvider::NativeTls)
            // The OS trust store. The `WebPki` default (a bundled list) is not compiled in.
            .root_certs(RootCerts::PlatformVerifier)
            .build();
        let mut builder = Agent::config_builder()
            .tls_config(tls)
            .https_only(settings.https_only)
            .timeout_global(Some(settings.timeout))
            // A 3xx answer is a status, not a hop: the endpoint is fixed and never followed
            // elsewhere. No `cookies` feature is compiled in, so none are stored or sent.
            .max_redirects(0)
            .max_redirects_will_error(false)
            // A status other than 200 is checked by hand, before any body is parsed.
            .http_status_as_error(false)
            .user_agent(user_agent(app_version))
            .accept(ACCEPT);
        if !settings.use_system_proxy {
            builder = builder.proxy(None);
        }
        let config: Config = builder.build();
        Self {
            agent: Agent::new_with_config(config),
            endpoint,
        }
    }

    /// The client against a local fake server (plain `http://127.0.0.1:<port>`): the URL is
    /// replaced and `https_only` and the system proxy are off, so a test never leaves the machine.
    #[cfg(test)]
    fn with_base_url(base_url: &str, app_version: &str) -> Self {
        Self::with_base_url_and_timeout(base_url, app_version, REQUEST_TIMEOUT)
    }

    #[cfg(test)]
    fn with_base_url_and_timeout(base_url: &str, app_version: &str, timeout: Duration) -> Self {
        Self::build(
            format!("{base_url}/repos/{REPOSITORY}/releases/latest"),
            app_version,
            Settings {
                https_only: false,
                use_system_proxy: false,
                timeout,
            },
        )
    }
}

struct Settings {
    https_only: bool,
    /// Production honours the user's system proxy settings (a corporate network); tests never do.
    use_system_proxy: bool,
    timeout: Duration,
}

impl Settings {
    fn production() -> Self {
        Self {
            https_only: true,
            use_system_proxy: true,
            timeout: REQUEST_TIMEOUT,
        }
    }
}

/// `MyPass/<version>`, keeping only characters that are safe in a header value.
fn user_agent(app_version: &str) -> String {
    let version: String = app_version
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+'))
        .take(32)
        .collect();
    format!("MyPass/{version}")
}

impl ReleaseFeed for GitHubReleaseClient {
    fn latest(&self) -> Result<ReleaseInfo, ReleaseFeedError> {
        let mut response = self
            .agent
            .get(&self.endpoint)
            .header("X-GitHub-Api-Version", API_VERSION)
            .call()
            .map_err(map_error)?;
        // The status is checked before the body is touched: an error body (rate limit, outage
        // page) is not a release and must not be parsed as one.
        let status = response.status().as_u16();
        if status != 200 {
            return Err(ReleaseFeedError::BadStatus(status));
        }
        let body = response
            .body_mut()
            .with_config()
            .limit(MAX_BODY_BYTES)
            .read_to_vec()
            .map_err(map_error)?;
        parse_release_payload(&body)
    }
}

/// Network-level failures are `Offline`; anything that arrived but cannot be used is `Malformed`.
/// The error text is dropped (it can carry a host or an OS message).
fn map_error(error: Error) -> ReleaseFeedError {
    match error {
        Error::Timeout(_)
        | Error::Io(_)
        | Error::HostNotFound
        | Error::ConnectionFailed
        | Error::Tls(_) => ReleaseFeedError::Offline,
        _ => ReleaseFeedError::Malformed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::thread;

    const GOOD_URL: &str = "https://github.com/slucheninov/mypass-app/releases/tag/v0.7.1";

    fn json(tag: &str, url: &str) -> String {
        serde_json::json!({ "tag_name": tag, "html_url": url }).to_string()
    }

    fn http(status_line: &str, content_type: &str, body: &str) -> Vec<u8> {
        format!(
            "HTTP/1.1 {status_line}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .into_bytes()
    }

    /// Serves exactly one request on 127.0.0.1 with `response`; returns the base URL and a
    /// receiver of the raw request head the server saw.
    fn serve(response: Vec<u8>) -> (String, mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let mut head = Vec::new();
            let mut buf = [0u8; 1024];
            while !head.windows(4).any(|w| w == b"\r\n\r\n") {
                match stream.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => head.extend_from_slice(&buf[..n]),
                }
            }
            let _ = tx.send(String::from_utf8_lossy(&head).into_owned());
            let _ = stream.write_all(&response);
            let _ = stream.flush();
        });
        (base, rx)
    }

    fn client(base: &str) -> GitHubReleaseClient {
        GitHubReleaseClient::with_base_url(base, "0.7.0")
    }

    #[test]
    fn maps_200_to_release() {
        let (base, request) = serve(http(
            "200 OK",
            "application/json",
            &json("v0.7.1", GOOD_URL),
        ));
        let info = client(&base).latest().unwrap();
        assert_eq!(info.version.to_string(), "0.7.1");
        assert_eq!(info.tag, "v0.7.1");
        assert_eq!(info.url, GOOD_URL);
        let head = request.recv().unwrap();
        assert!(
            head.starts_with("GET /repos/slucheninov/mypass-app/releases/latest HTTP/1.1\r\n"),
            "{head}"
        );
    }

    #[test]
    fn encoded_separator_release_urls_from_the_server_are_malformed() {
        for url in [
            "https://github.com/slucheninov/mypass-app/releases/%2f..%2f..%2fissues/1",
            "https://github.com/slucheninov/mypass-app/releases/%5C..%5C..%5Cissues/1",
            "https://github.com/slucheninov/mypass-app/releases/tag/v1%2F2",
            "https://github.com/slucheninov/mypass-app/releases/tag/v1%252F2",
        ] {
            let (base, _) = serve(http("200 OK", "application/json", &json("v0.7.1", url)));
            assert_eq!(
                client(&base).latest(),
                Err(ReleaseFeedError::Malformed),
                "{url}"
            );
        }
    }

    #[test]
    fn status_checked_before_body_parse() {
        // A 503 whose body would parse as a release is still a status error.
        let (base, _) = serve(http(
            "503 Service Unavailable",
            "text/html",
            "<html>down</html>",
        ));
        assert_eq!(
            client(&base).latest(),
            Err(ReleaseFeedError::BadStatus(503))
        );
        let (base, _) = serve(http(
            "500 Internal Server Error",
            "application/json",
            &json("v9.9.9", GOOD_URL),
        ));
        assert_eq!(
            client(&base).latest(),
            Err(ReleaseFeedError::BadStatus(500))
        );
    }

    #[test]
    fn rate_limit_statuses_are_reported_as_statuses() {
        for (line, code) in [("403 Forbidden", 403), ("429 Too Many Requests", 429)] {
            let (base, _) = serve(http(line, "application/json", "{\"message\":\"limit\"}"));
            assert_eq!(
                client(&base).latest(),
                Err(ReleaseFeedError::BadStatus(code))
            );
        }
    }

    #[test]
    fn a_redirect_is_a_status_and_is_never_followed() {
        let elsewhere = TcpListener::bind("127.0.0.1:0").unwrap();
        elsewhere.set_nonblocking(true).unwrap();
        let target = format!("http://{}/", elsewhere.local_addr().unwrap());
        let response = format!(
            "HTTP/1.1 301 Moved Permanently\r\nLocation: {target}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
        let (base, _) = serve(response.into_bytes());
        assert_eq!(
            client(&base).latest(),
            Err(ReleaseFeedError::BadStatus(301))
        );
        assert!(
            elsewhere.accept().is_err(),
            "the redirect target must not be contacted"
        );
    }

    #[test]
    fn release_url_must_be_github_releases_path() {
        for bad in [
            "http://github.com/slucheninov/mypass-app/releases/tag/v0.7.1",
            "https://example.com/slucheninov/mypass-app/releases/tag/v0.7.1",
            "https://github.com.evil.test/slucheninov/mypass-app/releases/tag/v0.7.1",
            "https://github.com/other/mypass-app/releases/tag/v0.7.1",
            "https://github.com/slucheninov/mypass-app/issues/1",
        ] {
            let (base, _) = serve(http("200 OK", "application/json", &json("v0.7.1", bad)));
            assert_eq!(
                client(&base).latest(),
                Err(ReleaseFeedError::Malformed),
                "{bad}"
            );
        }
    }

    #[test]
    fn a_non_version_tag_and_broken_json_are_refused() {
        let (base, _) = serve(http(
            "200 OK",
            "application/json",
            &json("nightly", GOOD_URL),
        ));
        assert_eq!(
            client(&base).latest(),
            Err(ReleaseFeedError::UnparsableVersion("nightly".into()))
        );
        let (base, _) = serve(http("200 OK", "application/json", "not json"));
        assert_eq!(client(&base).latest(), Err(ReleaseFeedError::Malformed));
    }

    #[test]
    fn an_oversized_answer_is_refused() {
        let pad = "x".repeat(usize::try_from(MAX_BODY_BYTES).unwrap() + 1024);
        let body =
            format!("{{\"tag_name\":\"v0.7.1\",\"html_url\":\"{GOOD_URL}\",\"body\":\"{pad}\"}}");
        let (base, _) = serve(http("200 OK", "application/json", &body));
        assert_eq!(client(&base).latest(), Err(ReleaseFeedError::Malformed));
    }

    #[test]
    fn sends_user_agent_and_api_version_and_nothing_else_identifying() {
        let (base, request) = serve(http(
            "200 OK",
            "application/json",
            &json("v0.7.1", GOOD_URL),
        ));
        GitHubReleaseClient::with_base_url(&base, "0.7.0")
            .latest()
            .unwrap();
        let head = request.recv().unwrap();
        let headers: Vec<(String, String)> = head
            .split("\r\n")
            .skip(1)
            .filter_map(|line| line.split_once(':'))
            .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_owned()))
            .collect();
        let get = |name: &str| {
            headers
                .iter()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.as_str())
        };
        assert_eq!(get("user-agent"), Some("MyPass/0.7.0"));
        assert_eq!(get("accept"), Some("application/vnd.github+json"));
        assert_eq!(get("x-github-api-version"), Some("2022-11-28"));
        // Nothing beyond transport headers: no cookie, credential, referrer or origin.
        for (name, _) in &headers {
            assert!(
                matches!(
                    name.as_str(),
                    "host"
                        | "user-agent"
                        | "accept"
                        | "x-github-api-version"
                        | "accept-encoding"
                        | "connection"
                ),
                "unexpected request header {name}: {head}"
            );
        }
        assert!(get("cookie").is_none() && get("authorization").is_none());
    }

    #[test]
    fn user_agent_keeps_only_header_safe_characters() {
        assert_eq!(user_agent("0.7.0"), "MyPass/0.7.0");
        assert_eq!(user_agent("1.0.0-rc.1+b5"), "MyPass/1.0.0-rc.1+b5");
        assert_eq!(user_agent("0.7.0\r\nX-Evil: 1"), "MyPass/0.7.0X-Evil1");
    }

    #[test]
    fn ten_second_timeout_is_set() {
        assert_eq!(REQUEST_TIMEOUT, Duration::from_secs(10));
        let production = GitHubReleaseClient::new("0.7.0");
        let config = production.agent.config();
        assert_eq!(config.timeouts().global, Some(Duration::from_secs(10)));
        assert!(config.https_only(), "production talks https only");
        assert_eq!(config.max_redirects(), 0);
        assert_eq!(
            production.endpoint,
            "https://api.github.com/repos/slucheninov/mypass-app/releases/latest"
        );
        // The OS trust store, through the OS TLS stack; never the bundled list.
        assert_eq!(config.tls_config().provider(), TlsProvider::NativeTls);
        assert!(matches!(
            config.tls_config().root_certs(),
            RootCerts::PlatformVerifier
        ));
    }

    #[test]
    fn a_silent_server_times_out_as_offline() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let (release, hold) = mpsc::channel::<()>();
        thread::spawn(move || {
            // Accept and say nothing until the test ends.
            let _conn = listener.accept();
            let _ = hold.recv();
        });
        let client = GitHubReleaseClient::with_base_url_and_timeout(
            &base,
            "0.7.0",
            Duration::from_millis(300),
        );
        assert_eq!(client.latest(), Err(ReleaseFeedError::Offline));
        drop(release);
    }

    #[test]
    fn a_refused_connection_is_offline() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        drop(listener);
        assert_eq!(client(&base).latest(), Err(ReleaseFeedError::Offline));
    }

    #[test]
    fn a_plain_http_endpoint_is_refused_by_the_production_config() {
        // `https_only` is what keeps the release request off plain http: build a client with the
        // production settings against a local http server and check it never connects.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let client = GitHubReleaseClient::build(
            format!("{base}/repos/{REPOSITORY}/releases/latest"),
            "0.7.0",
            Settings {
                use_system_proxy: false,
                ..Settings::production()
            },
        );
        assert!(client.latest().is_err());
        assert!(
            listener.accept().is_err(),
            "no connection may be made over http"
        );
    }
}
