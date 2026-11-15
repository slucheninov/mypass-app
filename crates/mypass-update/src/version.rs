//! `MAJOR.MINOR.PATCH` versions, ordered numerically (port of the old app's `SemanticVersion`).

use std::fmt;
use std::str::FromStr;

/// A release version: exactly three dot-separated numbers.
///
/// The derived `Ord` compares field by field in declaration order, which is the numeric order
/// (`0.10.0 > 0.9.9`); a lexical comparison of the strings would get that wrong.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct SemanticVersion {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
}

/// Why a version string was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("not a MAJOR.MINOR.PATCH version")]
pub struct VersionParseError;

impl SemanticVersion {
    /// A version from its three numbers.
    pub const fn new(major: u64, minor: u64, patch: u64) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }

    /// Parses an optional single leading `v`, then exactly three dot-separated parts made of
    /// ASCII digits only. Signs, pre-release or build suffixes, empty parts, a second `v`,
    /// whitespace and values that overflow `u64` are all refused: a tag that is not a plain
    /// release version is never treated as one.
    pub fn parse(text: &str) -> Result<Self, VersionParseError> {
        let body = text.strip_prefix('v').unwrap_or(text);
        let mut parts = body.split('.');
        let (Some(major), Some(minor), Some(patch), None) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return Err(VersionParseError);
        };
        Ok(Self {
            major: number(major)?,
            minor: number(minor)?,
            patch: number(patch)?,
        })
    }
}

fn number(part: &str) -> Result<u64, VersionParseError> {
    if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
        return Err(VersionParseError);
    }
    part.parse().map_err(|_| VersionParseError)
}

impl FromStr for SemanticVersion {
    type Err = VersionParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

impl fmt::Display for SemanticVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_and_v_prefixed() {
        assert_eq!(
            SemanticVersion::parse("1.2.3"),
            Ok(SemanticVersion::new(1, 2, 3))
        );
        assert_eq!(
            SemanticVersion::parse("v0.7.0"),
            Ok(SemanticVersion::new(0, 7, 0))
        );
        assert_eq!("v10.20.30".parse(), Ok(SemanticVersion::new(10, 20, 30)));
        assert_eq!(
            SemanticVersion::parse("007.0.0"),
            Ok(SemanticVersion::new(7, 0, 0))
        );
    }

    #[test]
    fn rejects_prerelease_build_sign_empty_two_part_and_double_v() {
        for bad in [
            "0.2.0-beta",
            "1.0.0+b",
            "+1.0.0",
            "-1.0.0",
            "1.0",
            "1.0.0.0",
            "vv1.0.0",
            "1..0",
            "",
            "v",
            "1.0.",
            ".1.0",
            " 1.0.0",
            "1.0.0 ",
            "1.0.0\n",
            "V1.0.0",
            "1.0.x",
            "99999999999999999999.0.0",
            "1.\u{0661}.0",
        ] {
            assert_eq!(
                SemanticVersion::parse(bad),
                Err(VersionParseError),
                "{bad:?} must be refused"
            );
        }
    }

    #[test]
    fn ordering_is_numeric_not_lexical() {
        let v = |s: &str| SemanticVersion::parse(s).unwrap();
        assert!(v("0.10.0") > v("0.9.9"));
        assert!(v("1.0.0") > v("0.99.99"));
        assert!(v("1.2.10") > v("1.2.9"));
        assert!(v("0.7.0") == v("v0.7.0"));
        assert!(v("0.7.0") < v("0.7.1"));
    }

    #[test]
    fn display_is_the_plain_three_part_form() {
        assert_eq!(SemanticVersion::new(0, 7, 0).to_string(), "0.7.0");
        let v = SemanticVersion::parse("v3.14.15").unwrap();
        assert_eq!(v.to_string(), "3.14.15");
    }
}
