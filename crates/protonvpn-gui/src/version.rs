//! What this build says it is.
//!
//! `build.rs` writes `PROTONVPN_GUI_VERSION` into the binary; `packaging/appimage/build.sh` fills
//! that variable from `scripts/version.sh`, which is the same number `scripts/release.sh` publishes
//! the release under. The running app compares it against `releases/latest` to decide whether an
//! update exists, so the two must be one string — see `docs/architecture.md` §14.
//!
//! Empty means the build was not made as a release, which is what a plain `cargo build` produces.
//! That is a state the application reports rather than papers over: a development build has nothing
//! to compare itself with, and inventing a version for it would have it replace itself with the
//! latest release.

use protonvpn_core::update::Version;

/// The raw string the build script baked in, empty when there was none.
pub const BAKED: &str = env!("PROTONVPN_GUI_VERSION");

/// The version to compare against a release, if this build has one. The engine is given this and
/// decides nothing itself: a build with no version reports the latest release instead of treating
/// itself as older than it.
pub fn current() -> Option<Version> {
    Version::parse(BAKED)
}

/// For `--version`: one line a script can read, not a sentence.
pub fn cli_line() -> String {
    if BAKED.is_empty() {
        format!("protonvpn-gui unversioned ({})", env!("CARGO_PKG_VERSION"))
    } else {
        format!("protonvpn-gui {BAKED}")
    }
}

/// For the settings page, where there is room to say why the number is missing.
pub fn label() -> &'static str {
    if BAKED.is_empty() {
        "сборка не из релиза"
    } else {
        BAKED
    }
}
