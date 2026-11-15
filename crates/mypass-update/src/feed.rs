//! The release feed seam: what the checker asks ("what is the latest release?"), the answer, and
//! the pure parsing and validation of GitHub's `releases/latest` payload.
//!
//! The HTTP client itself is behind the `github` feature (`github.rs`); everything here is plain
//! data and string handling, so it is tested without a network and linked by every host.

use crate::asset::ReleaseAsset;
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
    /// Published asset names only; no download URLs.
    pub assets: Vec<ReleaseAsset>,
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
/// (so `github.com.evil.test` and `evilgithub.com` fail), no port (not even `:443`), no user
/// info, and a path under `/slucheninov/mypass-app/releases/`. The query and fragment are
/// dropped.
///
/// Two layers, because `Url::parse` quietly rewrites some inputs before the parsed checks could
/// see them (a backslash becomes `/`, tabs and newlines vanish, dot segments are resolved, and
/// `%2F` / `%5C` stay encoded so a prefix check passes while a later decoder reads a separator):
///
/// 1. The raw text ([`raw_shape_is_plain`]): no control character, whitespace or backslash
///    anywhere; no `%` before the query (so no encoded separator, encoded dot or double encoding
///    in either hex case); no `:` or `@` in the authority (no port, no user info); no `.` or `..`
///    path segment.
/// 2. The parsed URL: the checks above, and every segment after the prefix is non-empty, made only
///    of ASCII letters, digits, `.`, `_` and `-`, and not a dot segment.
pub fn validate_release_url(raw: &str) -> Option<String> {
    if !raw_shape_is_plain(raw) {
        return None;
    }
    let mut url = Url::parse(raw).ok()?;
    if url.scheme() != "https"
        || url.host_str() != Some("github.com")
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return None;
    }
    let suffix = url.path().strip_prefix(RELEASES_PATH_PREFIX)?;
    if suffix.is_empty() || !suffix.split('/').all(is_plain_segment) {
        return None;
    }
    url.set_query(None);
    url.set_fragment(None);
    Some(url.into())
}

/// The raw-text layer of [`validate_release_url`]: what must be refused before `Url::parse`
/// normalises it away.
fn raw_shape_is_plain(raw: &str) -> bool {
    if raw
        .chars()
        .any(|c| c.is_control() || c.is_whitespace() || c == '\\')
    {
        return false;
    }
    let before_query = raw.split(['?', '#']).next().unwrap_or_default();
    if before_query.contains('%') {
        return false;
    }
    let Some(rest) = before_query.strip_prefix("https://") else {
        return false;
    };
    let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
    if authority.contains([':', '@']) {
        return false;
    }
    path.split('/')
        .all(|segment| segment != "." && segment != "..")
}

/// One path segment after the releases prefix: non-empty, `[A-Za-z0-9._-]` only, not `.` or `..`.
fn is_plain_segment(segment: &str) -> bool {
    !segment.is_empty()
        && segment != "."
        && segment != ".."
        && segment
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

#[derive(Deserialize)]
struct Payload {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    assets: Vec<ReleaseAsset>,
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
        assets: payload.assets,
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
            validate_release_url("https://GitHub.com/slucheninov/mypass-app/releases/latest")
                .as_deref(),
            Some("https://github.com/slucheninov/mypass-app/releases/latest")
        );
    }

    #[test]
    fn release_url_refuses_encoded_separators_and_path_tricks() {
        const P: &str = "https://github.com/slucheninov/mypass-app/releases/";
        let mut bad: Vec<String> = [
            // Encoded separators in both hex cases, alone and as traversal payloads.
            "%2f..%2f..%2fissues/1",
            "%2F..%2F..%2Fissues/1",
            "%5c..%5c..%5cissues/1",
            "%5C..%5C..%5Cissues/1",
            "tag/v1%2F2",
            "tag/v1%2f2",
            "tag/v1%5C2",
            "tag/v1%5c2",
            // Double encoding and encoded dots.
            "%252f..%252f..%252fissues/1",
            "tag/v1%252F2",
            "%2e%2e/%2e%2e/issues/1",
            "tag/%2E%2E/latest",
            "tag/v0.7.1%00",
            "tag/v%30.7.1",
            // Backslashes, which the parser would turn into separators.
            "tag\\v0.7.1",
            "..\\..\\issues/1",
            // Dot segments that resolve inside or outside the prefix.
            "tag/../latest",
            "./tag/v0.7.1",
            "tag/./v0.7.1",
            "tag/v0.7.1/..",
            // Empty segments, whitespace and control characters the parser strips.
            "tag//v0.7.1",
            "tag/v0.7.1/",
            "tag/v0.7.1\tx",
            "tag/\n../../issues/1",
            "tag/v 0.7.1",
            // Non-ASCII that would be percent-encoded by the parser.
            "tag/v\u{2215}0.7.1",
        ]
        .iter()
        .map(|suffix| format!("{P}{suffix}"))
        .collect();
        bad.extend(
            [
                // User info and ports, also the default one and an empty one.
                "https://a@github.com/slucheninov/mypass-app/releases/tag/v0.7.1",
                "https://:@github.com/slucheninov/mypass-app/releases/tag/v0.7.1",
                "https://github.com:443/slucheninov/mypass-app/releases/tag/v0.7.1",
                "https://github.com:/slucheninov/mypass-app/releases/tag/v0.7.1",
                // Other hosts and spellings of the host.
                "https://github.com./slucheninov/mypass-app/releases/tag/v0.7.1",
                "https://api.github.com/slucheninov/mypass-app/releases/tag/v0.7.1",
                "https://140.82.112.3/slucheninov/mypass-app/releases/tag/v0.7.1",
                "https://xn--github-xyz.com/slucheninov/mypass-app/releases/tag/v0.7.1",
                "https://github%2Ecom/slucheninov/mypass-app/releases/tag/v0.7.1",
                // An encoded separator before the prefix.
                "https://github.com/slucheninov%2Fmypass-app/releases/tag/v0.7.1",
                "https:\\\\github.com/slucheninov/mypass-app/releases/tag/v0.7.1",
                "HTTPS://github.com/slucheninov/mypass-app/releases/tag/v0.7.1",
            ]
            .map(String::from),
        );
        for url in &bad {
            assert_eq!(validate_release_url(url), None, "{url:?} must be refused");
            assert_eq!(
                parse_release_payload(&body("v0.7.1", url)),
                Err(ReleaseFeedError::Malformed),
                "{url:?} must be refused in a payload"
            );
        }
        // Plain release pages, with a percent sign only in the dropped query, stay accepted.
        for good in [
            GOOD.to_string(),
            format!("{P}latest"),
            format!("{P}tag/v0.7.1_rc-1"),
            format!("{GOOD}?q=%2F#%5C"),
        ] {
            assert!(
                validate_release_url(&good).is_some(),
                "{good:?} must be accepted"
            );
        }
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
