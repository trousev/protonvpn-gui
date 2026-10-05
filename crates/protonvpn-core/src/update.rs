//! Auto-update for AppImage installs — sanctioned exception #4 (`docs/architecture.md` §0, §14).
//!
//! An AppImage has no package manager behind it. Without something here, "update" means "notice the
//! release, download the file, replace it by hand" — and the file that has to be replaced is the
//! one currently running. This module is the smallest thing that closes that loop without a daemon,
//! without a package manager and without executing anything new: it asks our own release page what
//! the latest version is, verifies the bytes against the checksums published with them, and renames
//! the new image over the old one.
//!
//! What it deliberately does **not** do:
//!
//! * it never runs the downloaded file, and never restarts the application — the new image takes
//!   effect at the next start, which is a decision the user keeps;
//! * it never asks a third party anything: one URL, ours (`releases/latest/download/SHA256SUMS`);
//! * it never touches the `protonvpn` CLI, its queue, or any state derived from it.

use std::fmt;

/// A release version: `X.Y.N`.
///
/// `X.Y` is the line's base tag and `N` the number of commits at the time of the release
/// (`scripts/version.sh`, which `scripts/release.sh` also publishes with). Ordered numerically:
/// `0.1.9 < 0.1.10`, where a string comparison would say the opposite — and a wrong answer here is
/// an update offered in the wrong direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
}

impl Version {
    /// Parses `X.Y.N`, tolerating whitespace and a leading `v`.
    ///
    /// Anything else is `None` rather than a guess: a pre-release tag, a date tag or a branch name
    /// is not something this program may interpret, and treating `0.2` as `0.2.0` would make it
    /// comparable with releases that were never made.
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        let text = text.strip_prefix('v').unwrap_or(text);
        let mut parts = text.split('.');

        let major = parts.next()?.parse().ok()?;
        let minor = parts.next()?.parse().ok()?;
        let patch = parts.next()?.parse().ok()?;
        if parts.next().is_some() {
            return None;
        }
        Some(Self {
            major,
            minor,
            patch,
        })
    }

    /// The tag a release of this version is published under.
    pub fn tag(self) -> String {
        self.to_string()
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

impl std::str::FromStr for Version {
    type Err = ParseVersionError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Self::parse(text).ok_or_else(|| ParseVersionError(text.to_string()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseVersionError(String);

impl fmt::Display for ParseVersionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "не версия вида X.Y.N: {}", self.0)
    }
}

impl std::error::Error for ParseVersionError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_version_shape_a_release_publishes() {
        assert_eq!(
            Version::parse("0.1.42"),
            Some(Version {
                major: 0,
                minor: 1,
                patch: 42
            })
        );
        assert_eq!(
            Version::parse(" 1.2.3 ").map(|v| v.tag()),
            Some("1.2.3".into())
        );
        assert_eq!(
            Version::parse("v1.2.3").map(|v| v.tag()),
            Some("1.2.3".into())
        );
    }

    #[test]
    fn refuses_anything_that_is_not_exactly_three_numbers() {
        for text in ["", "0.1", "0.1.2.3", "0.1.42-rc1", "0.1.x", "latest"] {
            assert_eq!(Version::parse(text), None, "{text:?} must not parse");
        }
    }

    #[test]
    fn orders_numerically_and_not_like_text() {
        let older = Version::parse("0.1.9").unwrap();
        let newer = Version::parse("0.1.10").unwrap();
        assert!(older < newer);
        assert!(
            "0.1.9" > "0.1.10",
            "the string comparison this type exists to avoid"
        );

        let next_line = Version::parse("0.2.0").unwrap();
        assert!(newer < next_line);
    }
}
