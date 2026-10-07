//! A live check of the updater: the real code, the real release page, the real asset.
//!
//! Every other test in this repository talks to a stand-in `curl`, on purpose — a suite that
//! reaches the network is a suite that fails when the network does. This file is the exception a
//! human asks for, so that "the updater works" is a measurement rather than a belief:
//!
//! ```sh
//! cargo test -p protonvpn-core --test live_update -- --ignored --nocapture
//! ```
//!
//! Both tests are `#[ignore]`d, so `cargo test` never runs them and CI never sees them. They need
//! the network, and one of them downloads about 5 MB from `github.com`.
//!
//! What they cover, in the order they matter:
//!
//! * the release page is reachable and its answer is parsed — a release, or a finding with a
//!   reason, and never a guess;
//! * the real HTTPS request, the redirect to `objects.githubusercontent.com`, the `Content-Length`
//!   that has to survive it, the progress polling, the SHA-256 of a real 5 MB file, the type-2
//!   AppImage check on real bytes, and the swap: hard link, atomic rename, the old image kept.
//!
//! The release below is the newest published one at the time of writing. When a release exists
//! whose image carries its version in the name, the first test starts answering with a release
//! instead of a finding — both are accepted, and the second test does not depend on that at all.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

use protonvpn_core::i18n::{I18n, Locale};
use protonvpn_core::update::{self, Finding, Installed, Release, Version};

/// The release these live checks are written against.
const TAG: &str = "0.1.20";
const ASSET: &str = "ProtonVPN-GUI-x86_64.AppImage";
const SHA256: &str = "70c8c123460c7d127d05d25b2b676965f4548a823467114e8615094c0b3cb8a5";

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("protonvpn-live-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
#[ignore = "reaches github.com — run by hand, on purpose"]
fn the_release_page_answers_the_real_question() {
    let finding = update::latest("curl").expect("the real release page must answer");
    println!("live finding: {finding:#?}");

    match finding {
        // A release whose image cannot be installed from still has to say *why*, and the reason
        // comes from the file rather than from this test.
        Finding::Nothing(why) => {
            let why = why.describe(&I18n::new(Locale::SOURCE));
            assert!(why.contains("SHA256SUMS"), "{why}");
        }
        Finding::Release(release) => {
            assert!(release.sha256.len() == 64, "{release:?}");
            println!("installable release: {release:?}");
        }
    }
}

#[test]
#[ignore = "downloads ~5 MB from github.com — run by hand, on purpose"]
fn a_real_image_is_really_downloaded_verified_and_swapped() {
    let dir = scratch("swap");
    let installed_path = dir.join(format!("ProtonVPN-GUI-{TAG}-x86_64.AppImage"));

    // What "already installed" means to `fetch`: a path with a mode. The bytes only matter to the
    // backup check at the end, which is why they are recognisable rather than an AppImage.
    let previous = b"# not an AppImage: whatever was installed before\n".to_vec();
    fs::write(&installed_path, &previous).unwrap();
    let installed = Installed::at(&installed_path).expect("a file to replace");

    let release = Release {
        version: Version::parse(TAG).unwrap(),
        asset: ASSET.to_string(),
        sha256: SHA256.to_string(),
    };

    let mut readings = Vec::new();
    let staged = update::fetch(
        "curl",
        &installed,
        &release,
        &AtomicBool::new(false),
        |got, total| readings.push((got, total)),
    )
    .expect("the real asset must download and verify");

    let downloaded = fs::read(&staged).unwrap();
    println!(
        "downloaded {} bytes in {} progress readings; first {:?}, last {:?}",
        downloaded.len(),
        readings.len(),
        readings.first(),
        readings.last()
    );
    assert_eq!(readings.first().unwrap().0, 0);
    assert_eq!(
        readings.first().unwrap().1,
        Some(downloaded.len() as u64),
        "the size has to survive the redirect to objects.githubusercontent.com"
    );
    assert_eq!(readings.last().unwrap().0, downloaded.len() as u64);
    assert!(
        update::looks_like_appimage(&downloaded[..12]),
        "a real type-2 AppImage"
    );

    // Independently of this crate's own hashing, with the program a human would reach for.
    let checked = std::process::Command::new("sha256sum")
        .arg(&staged)
        .output()
        .expect("sha256sum");
    let text = String::from_utf8_lossy(&checked.stdout);
    println!("sha256sum says: {}", text.trim());
    assert!(text.starts_with(SHA256), "{text}");

    let backup = update::install(&installed, &staged)
        .unwrap()
        .expect("a way back");
    assert_eq!(fs::read(installed.path()).unwrap(), downloaded, "in place");
    assert_eq!(
        fs::read(&backup).unwrap(),
        previous,
        "the old bytes are still reachable"
    );
    assert!(
        !installed.staging_path().exists(),
        "the staging file became the image"
    );

    // The next start is what throws the copy away.
    update::discard_backup(&installed);
    assert!(!backup.exists());
}
