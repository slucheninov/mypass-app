//! The release feed seam: what the checker asks ("what is the latest release?"), the answer, and
//! the pure parsing and validation of GitHub's `releases/latest` payload.
//!
//! The HTTP client itself is behind the `github` feature (`github.rs`); everything here is plain
//! data and string handling, so it is tested without a network and linked by every host.

use crate::version::SemanticVersion;
use serde::Deserialize;
use url::Url;

/// The repository whose releases the app checks.
pub const REPOSITORY: &str = "slucheninov/mypass-app";

/// The path prefix every accepted release page lives under.
const RELEASES_PATH_PREFIX: &str = "/slucheninov/mypass-app/releases/";

/// The newest published release, in the banner-only shape of this stage.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ReleaseInfo {
    pub version: SemanticVersion,
    /// The release tag as published (`v0.7.1`).
    pub tag: String,
    /// The release page, validated by [`validate_release_url`]: always
    /// `https://github.com/slucheninov/mypass-app/releases/...`, no credentials, query or fragment.
    pub url: String,
}

/// Why the feed produced no release. The variants carry no free text that could reach a webview;
/// the UI maps the kind to a localised message.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ReleaseFeedError {
    /// The network, DNS, TLS or the timeout failed, or no feed is configured (`NoopFeed`).
    #[error("the release feed is unreachable")]
    Offline,
    /// The server answered with a status other than 200.
    #[error("the release feed answered with status {0}")]
    BadStatus(u16),
    /// The answer is not a release this app accepts (bad JSON, bad or foreign release URL, a body
    /// that is too large).
    #[error("the release feed answer is malformed")]
    Malformed,
    /// The release tag is not a plain `MAJOR.MINOR.PATCH` version.
    #[error("the release tag {0:?} is not a version")]
    UnparsableVersion(String),
}

/// Asks "what is the latest release?". Blocking: the caller runs it off the UI thread.
pub trait ReleaseFeed: Send + Sync {
    fn latest(&self) -> Result<ReleaseInfo, ReleaseFeedError>;
}

/// A feed that never reaches anything: every call is `Err(Offline)`. What every test host, and a
/// debug build without `MYPASS_UPDATE_CHECK=1`, uses, so no test or developer run ever talks to
/// GitHub.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoopFeed;

impl ReleaseFeed for NoopFeed {
    fn latest(&self) -> Result<ReleaseInfo, ReleaseFeedError> {
        Err(ReleaseFeedError::Offline)
    }
}

/// Accepts only a release page of this repository and returns its canonical form.
///
/// The URL is later opened in the user's browser (from Rust, never from a webview), so a
/// compromised or confused answer must not point the user at another host, another repository, a
/// non-release page or a non-web scheme. Required: scheme `https`, host exactly `github.com`
/// (so `github.com.evil.test` and `evilgithub.com` fail), default port, no user info, and a path
/// under `/slucheninov/mypass-app/releases/`. The query and fragment are dropped.
pub fn validate_release_url(raw: &str) -> Option<String> {
    let mut url = Url::parse(raw).ok()?;
    if url.scheme() != "https"
        || url.host_str() != Some("github.com")
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || !url.path().starts_with(RELEASES_PATH_PREFIX)
        || url.path().len() == RELEASES_PATH_PREFIX.len()
    {
        return None;
    }
    url.set_query(None);
    url.set_fragment(None);
    Some(url.into())
}

#[derive(Deserialize)]
struct Payload {
    tag_name: String,
    html_url: String,
}

/// Turns the body of a `200` answer into a [`ReleaseInfo`]. `releases/latest` already excludes
/// drafts and pre-releases, so no filtering happens here.
pub fn parse_release_payload(body: &[u8]) -> Result<ReleaseInfo, ReleaseFeedError> {
    let payload: Payload = serde_json::from_slice(body).map_err(|_| ReleaseFeedError::Malformed)?;
    let version = SemanticVersion::parse(&payload.tag_name)
        .map_err(|_| ReleaseFeedError::UnparsableVersion(payload.tag_name.clone()))?;
    let url = validate_release_url(&payload.html_url).ok_or(ReleaseFeedError::Malformed)?;
    Ok(ReleaseInfo {
        version,
        tag: payload.tag_name,
        url,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(tag: &str, url: &str) -> Vec<u8> {
        serde_json::json!({ "tag_name": tag, "html_url": url, "name": "ignored" })
            .to_string()
            .into_bytes()
    }

    const GOOD: &str = "https://github.com/slucheninov/mypass-app/releases/tag/v0.7.1";

    #[test]
    fn noop_feed_is_always_offline() {
        assert_eq!(NoopFeed.latest(), Err(ReleaseFeedError::Offline));
    }

    #[test]
    fn maps_a_release_payload() {
        let info = parse_release_payload(&body("v0.7.1", GOOD)).unwrap();
        assert_eq!(info.version, SemanticVersion::new(0, 7, 1));
        assert_eq!(info.tag, "v0.7.1");
        assert_eq!(info.url, GOOD);
    }

    #[test]
    fn release_url_must_be_github_releases_path() {
        for bad in [
            "http://github.com/slucheninov/mypass-app/releases/tag/v0.7.1",
            "https://example.com/slucheninov/mypass-app/releases/tag/v0.7.1",
            "https://github.com.evil.test/slucheninov/mypass-app/releases/tag/v0.7.1",
            "https://evilgithub.com/slucheninov/mypass-app/releases/tag/v0.7.1",
            "https://www.github.com/slucheninov/mypass-app/releases/tag/v0.7.1",
            "https://github.com/someone-else/mypass-app/releases/tag/v0.7.1",
            "https://github.com/slucheninov/mypass/releases/tag/v0.7.1",
            "https://github.com/slucheninov/mypass-app/issues/1",
            "https://github.com/slucheninov/mypass-app/releases",
            "https://github.com/slucheninov/mypass-app/releases/",
            "https://github.com/slucheninov/mypass-app/releasesx/tag/v0.7.1",
            "https://github.com/slucheninov/mypass-app/releases/../../evil/x",
            "https://user:pw@github.com/slucheninov/mypass-app/releases/tag/v0.7.1",
            "https://github.com@evil.test/slucheninov/mypass-app/releases/tag/v0.7.1",
            "https://github.com:8443/slucheninov/mypass-app/releases/tag/v0.7.1",
            "javascript:alert(1)",
            "file:///slucheninov/mypass-app/releases/tag/v0.7.1",
            "github.com/slucheninov/mypass-app/releases/tag/v0.7.1",
            "",
        ] {
            assert_eq!(
                parse_release_payload(&body("v0.7.1", bad)),
                Err(ReleaseFeedError::Malformed),
                "{bad:?} must be refused"
            );
        }
    }

    #[test]
    fn release_url_is_canonicalised_without_query_or_fragment() {
        let url = format!("{GOOD}?utm=1#frag");
        assert_eq!(validate_release_url(&url).as_deref(), Some(GOOD));
        assert_eq!(
            validate_release_url("https://GitHub.com:443/slucheninov/mypass-app/releases/latest")
                .as_deref(),
            Some("https://github.com/slucheninov/mypass-app/releases/latest")
        );
    }

    #[test]
    fn an_unparsable_tag_is_reported_with_the_tag() {
        assert_eq!(
            parse_release_payload(&body("0.7.1-beta", GOOD)),
            Err(ReleaseFeedError::UnparsableVersion("0.7.1-beta".into()))
        );
    }

    #[test]
    fn broken_json_and_missing_fields_are_malformed() {
        for bad in [
            &b"<html>503</html>"[..],
            b"",
            b"[]",
            b"{}",
            br#"{"tag_name":"v1.0.0"}"#,
            br#"{"html_url":"x"}"#,
            br#"{"tag_name":7,"html_url":"x"}"#,
        ] {
            assert_eq!(
                parse_release_payload(bad),
                Err(ReleaseFeedError::Malformed),
                "{:?}",
                String::from_utf8_lossy(bad)
            );
        }
    }
}
