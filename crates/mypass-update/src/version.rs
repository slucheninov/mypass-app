//! SemVer ordering for the stable and numbered prerelease tags published by MyPass.

use std::fmt;
use std::str::FromStr;

/// The derived order compares the numeric core before the prerelease channel and sequence.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct SemanticVersion {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
    stage: Stage,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
enum Stage {
    Alpha(u64),
    Beta(u64),
    Rc(u64),
    Stable,
}

/// Why a version string was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("not a MyPass release version")]
pub struct VersionParseError;

impl SemanticVersion {
    /// A version from its three numbers.
    pub const fn new(major: u64, minor: u64, patch: u64) -> Self {
        Self {
            major,
            minor,
            patch,
            stage: Stage::Stable,
        }
    }

    pub fn is_prerelease(self) -> bool {
        self.stage != Stage::Stable
    }

    /// Parses an optional leading `v` and an optional `-alpha.N`, `-beta.N` or `-rc.N` suffix.
    pub fn parse(text: &str) -> Result<Self, VersionParseError> {
        let body = text.strip_prefix('v').unwrap_or(text);
        let (base, stage) = match body.split_once('-') {
            None => (body, Stage::Stable),
            Some((base, suffix)) => {
                let (channel, sequence) = suffix.split_once('.').ok_or(VersionParseError)?;
                let sequence = number(sequence)?;
                let stage = match channel {
                    "alpha" => Stage::Alpha(sequence),
                    "beta" => Stage::Beta(sequence),
                    "rc" => Stage::Rc(sequence),
                    _ => return Err(VersionParseError),
                };
                (base, stage)
            }
        };
        let mut parts = base.split('.');
        let (Some(major), Some(minor), Some(patch), None) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return Err(VersionParseError);
        };
        Ok(Self {
            major: number(major)?,
            minor: number(minor)?,
            patch: number(patch)?,
            stage,
        })
    }
}

fn number(part: &str) -> Result<u64, VersionParseError> {
    if part.is_empty()
        || (part.len() > 1 && part.starts_with('0'))
        || !part.bytes().all(|b| b.is_ascii_digit())
    {
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
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)?;
        match self.stage {
            Stage::Alpha(n) => write!(f, "-alpha.{n}"),
            Stage::Beta(n) => write!(f, "-beta.{n}"),
            Stage::Rc(n) => write!(f, "-rc.{n}"),
            Stage::Stable => Ok(()),
        }
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
    }

    #[test]
    fn rejects_malformed_or_unsupported_release_versions() {
        for bad in [
            "0.2.0-beta",
            "0.2.0-alpha.01",
            "0.2.0-nightly.1",
            "007.0.0",
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

    #[test]
    fn prereleases_preserve_the_suffix_and_follow_semver_precedence() {
        let mut previous = SemanticVersion::new(0, 7, 0);
        for text in [
            "0.7.1-alpha.2",
            "0.7.1-alpha.10",
            "0.7.1-beta.1",
            "0.7.1-rc.1",
            "0.7.1",
        ] {
            let version = SemanticVersion::parse(&format!("v{text}")).unwrap();
            assert!(version > previous);
            assert_eq!(version.to_string(), text);
            previous = version;
        }
    }
}
