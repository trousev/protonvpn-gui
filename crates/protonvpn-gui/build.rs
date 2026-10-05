//! Bakes the release version into the binary.
//!
//! The version is `X.Y.N`, computed by `scripts/version.sh`: `X.Y` is the line's base tag and `N`
//! the commit count. Cargo has no idea about any of that — `CARGO_PKG_VERSION` is a hand-kept
//! `0.1.0` in the manifest and is deliberately not what the updater compares.
//!
//! Unset means "not a release", which is what a plain `cargo build` gets. That is a real state the
//! application reports honestly rather than a version it invents: a development build must not
//! decide it is older than the latest release and start replacing itself.

fn main() {
    // Without this the version is only read when something else happens to invalidate the crate,
    // and a rebuilt AppImage could carry the previous release's number — an update offered forever
    // and applied never.
    println!("cargo:rerun-if-env-changed=PROTONVPN_GUI_VERSION");

    println!(
        "cargo:rustc-env=PROTONVPN_GUI_VERSION={}",
        std::env::var("PROTONVPN_GUI_VERSION").unwrap_or_default()
    );
}
