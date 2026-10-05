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
//! * it never asks a third party anything: one URL, ours (`releases/latest/download/SHA256SUMS`),
//!   and the assets that file names;
//! * it never touches the `protonvpn` CLI, its queue, or any state derived from it.

use std::fmt;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use sha2::{Digest, Sha256};

/// How often the staging file is measured while a download runs. It is the only progress signal
/// there is: `curl` writes the bytes, we watch the file they land in.
const POLL: Duration = Duration::from_millis(200);

/// The repository this updater is allowed to talk to, as `owner/name`.
///
/// A test below pins it against `CARGO_PKG_REPOSITORY`, so the release page and the crate cannot
/// drift apart. Everything else about the network surface of this module follows from it: two URLs,
/// both on `github.com`, both ours.
const REPO_SLUG: &str = "trousev/protonvpn-gui";

/// The checksums `scripts/release.sh` publishes beside the assets.
const SUMS_ASSET: &str = "SHA256SUMS";

/// How an AppImage is named by that script. The version in the middle is the release tag, which is
/// how this module learns the version at all — without a GitHub API, without JSON, and with the
/// checksum and the version arriving in the same document.
const ASSET_PREFIX: &str = "ProtonVPN-GUI-";
const ASSET_SUFFIX: &str = ".AppImage";

/// The architecture whose image this process could become. `uname -m` at build time, spelled the
/// way `packaging/appimage/build.sh` writes it into the file name.
const ARCH: &str = std::env::consts::ARCH;

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

/// The file listing the checksums of the latest release. `latest/download/<asset>` is a permanent
/// URL: it follows to whatever was published most recently, so no API, no rate limit and no schema
/// that can change under us.
pub fn sums_url() -> String {
    format!("https://github.com/{REPO_SLUG}/releases/latest/download/{SUMS_ASSET}")
}

/// One asset of one release. The URL is derived from the version and the name rather than from an
/// API response, because those two are exactly what the checksum file states.
pub fn asset_url(version: Version, asset: &str) -> String {
    format!(
        "https://github.com/{REPO_SLUG}/releases/download/{}/{asset}",
        version.tag()
    )
}

/// An image of ours, as named in `SHA256SUMS` and on the release page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub version: Version,
    pub asset: String,
    pub sha256: String,
}

impl Release {
    pub fn url(&self) -> String {
        asset_url(self.version, &self.asset)
    }
}

/// How an image of `version` is named on the release page — the convention this module and
/// `packaging/appimage/build.sh` share, pinned by a test on both sides.
pub fn asset_name(version: Version) -> String {
    format!("{ASSET_PREFIX}{version}-{ARCH}{ASSET_SUFFIX}")
}

/// Reads the release out of a `SHA256SUMS` body.
///
/// The format is `sha256sum`'s own — `<hex>  <name>`, with a `*` before the name in binary mode —
/// and only the line naming this architecture's AppImage is of interest: a tarball, or an image for
/// a machine this is not, is not something this process can become.
pub fn parse_sums(body: &str) -> Result<Release, UpdateError> {
    let want_suffix = format!("-{ARCH}{ASSET_SUFFIX}");

    for line in body.lines() {
        let mut fields = line.split_whitespace();
        let (Some(hash), Some(name)) = (fields.next(), fields.next()) else {
            continue;
        };
        // `sha256sum --binary` writes a `*` before the name; it is not part of it.
        let name = name.trim_start_matches('*');
        let Some(middle) = name
            .strip_prefix(ASSET_PREFIX)
            .and_then(|rest| rest.strip_suffix(&want_suffix))
        else {
            continue;
        };
        let (Some(version), true) = (Version::parse(middle), is_sha256(hash)) else {
            continue;
        };

        return Ok(Release {
            version,
            asset: name.to_string(),
            sha256: hash.to_ascii_lowercase(),
        });
    }

    Err(UpdateError::NoAsset(format!(
        "в {SUMS_ASSET} нет строки вида {ASSET_PREFIX}X.Y.N{want_suffix}"
    )))
}

fn is_sha256(text: &str) -> bool {
    text.len() == 64 && text.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// What the running build is, compared with what the latest release is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Standing {
    /// Up to date — or ahead of the release line, which a build from `main` between releases is.
    /// Either way there is nothing to install, and a downgrade is not an update.
    Current,
    /// A later release exists.
    Behind(Version),
    /// This build does not know its own version (it was not built as a release), so nothing can be
    /// compared. It says so rather than guessing in either direction.
    Unversioned,
}

pub fn standing(current: Option<Version>, latest: Version) -> Standing {
    match current {
        None => Standing::Unversioned,
        Some(current) if current < latest => Standing::Behind(latest),
        Some(_) => Standing::Current,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateError {
    /// `curl` is not installed, or could not be run.
    CurlMissing(String),
    /// `curl` ran and failed: no route, a timeout, a 404, a write error.
    CurlFailed {
        code: Option<i32>,
        stderr: String,
    },
    /// The release page answered, but there is nothing in it this build could become.
    NoAsset(String),
    /// The bytes are not the bytes the release published a checksum for.
    ChecksumMismatch {
        expected: String,
        got: String,
    },
    /// Bytes that hash correctly and are still not a type-2 AppImage.
    NotAnAppImage(String),
    /// Nowhere to put the new image, so there is no point downloading it.
    NotWritable(String),
    Cancelled,
    Io(String),
}

impl fmt::Display for UpdateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CurlMissing(e) => write!(f, "curl недоступен: {e}"),
            Self::CurlFailed { code, stderr } => {
                write!(f, "curl завершился с кодом {}", code.unwrap_or(-1))?;
                if !stderr.trim().is_empty() {
                    write!(f, ": {}", stderr.trim())?;
                }
                Ok(())
            }
            Self::NoAsset(why) => write!(f, "нечего устанавливать: {why}"),
            Self::ChecksumMismatch { expected, got } => write!(
                f,
                "контрольная сумма не совпала: ожидалась {expected}, получена {got}"
            ),
            Self::NotAnAppImage(why) => write!(f, "скачанное не похоже на AppImage: {why}"),
            Self::NotWritable(why) => {
                write!(
                    f,
                    "не могу записать новый образ рядом с установленным: {why}"
                )
            }
            Self::Cancelled => write!(f, "загрузка отменена"),
            Self::Io(e) => write!(f, "ошибка ввода-вывода: {e}"),
        }
    }
}

impl std::error::Error for UpdateError {}

/// Asks the release page what the latest version is. One `curl`, one URL, no user data.
pub fn latest(curl: &str) -> Result<Release, UpdateError> {
    let body = curl_text(curl, &sums_url())?;
    parse_sums(&body)
}

/// Downloads the release's image next to the installed one and proves it is what it claims to be.
///
/// The result is a staged file, not an installed one: [`install`] is a separate step so that the
/// user can be asked in between, and so that a failure here has changed nothing at all.
///
/// `progress` is called with the bytes written so far and the total the server announced, which is
/// `None` when it did not announce one. Nothing about this call blocks the caller: the child is
/// polled, so a 70 MB download stays cancellable at any moment.
pub fn fetch(
    curl: &str,
    installed: &Installed,
    release: &Release,
    cancel: &AtomicBool,
    mut progress: impl FnMut(u64, Option<u64>),
) -> Result<PathBuf, UpdateError> {
    installed.writable()?;
    let staged = installed.staging_path();

    // Already downloaded and verified? Then there is nothing to do: the transfer this function was
    // about to make has already happened, and an application restarted between downloading and
    // installing must not pay for it twice. The checksum is what makes trusting the file safe — it
    // is exactly the proof this function exists to produce, and it comes from the release page, not
    // from the file.
    if staged.is_file() && verify(&staged, release).is_ok() {
        let bytes = fs::metadata(&staged).map(|meta| meta.len()).unwrap_or(0);
        progress(bytes, Some(bytes));
        return Ok(staged);
    }

    let url = release.url();
    let total = head_size(curl, &url);
    progress(0, total);
    download(curl, &url, &staged, cancel, |received| {
        progress(received, total)
    })?;

    if let Err(error) = verify(&staged, release) {
        let _ = fs::remove_file(&staged);
        return Err(error);
    }

    make_executable(&staged, installed.path())?;
    // Before the rename, not after: a rename can reach the disk before the data does, and a crash
    // in that window would leave a zero-length file where the application used to be.
    fsync(&staged)?;
    Ok(staged)
}

/// Whether these are the bytes the release published a checksum for, and an AppImage at all — the
/// two things [`fetch`] promises about what it returns.
///
/// What the checksum is worth is spelled out where it belongs: it proves the download agrees with a
/// document fetched over the same connection, which is worth having against a truncated file, a
/// proxy, or a mirror serving yesterday's image. It is not proof of authorship — `SECURITY.md`.
///
/// The shape check is here because a correct hash of the wrong document is a real thing: an error
/// page behind a captive portal agrees perfectly with a checksum file served from the same portal.
fn verify(path: &Path, release: &Release) -> Result<(), UpdateError> {
    let got = sha256_file(path)?;
    if !got.eq_ignore_ascii_case(&release.sha256) {
        return Err(UpdateError::ChecksumMismatch {
            expected: release.sha256.clone(),
            got,
        });
    }

    let head = read_head(path, 12)?;
    if !looks_like_appimage(&head) {
        return Err(UpdateError::NotAnAppImage(describe_head(&head)));
    }
    Ok(())
}

/// Renames verified bytes over the installed image, keeping the old one as `<name>.old`.
///
/// The rename is within one directory and therefore atomic: no moment exists in which the installed
/// path is missing a file. The backup is made as a hard link before it, which is what keeps that
/// true for the swap itself — the old inode stays reachable under its second name.
///
/// A running AppImage is unaffected: its runtime holds the file open, so this process keeps running
/// on the old bytes and the new image takes effect at the next start.
pub fn install(installed: &Installed, staged: &Path) -> Result<Option<PathBuf>, UpdateError> {
    let backup = installed.backup_path();
    let _ = fs::remove_file(&backup);

    // Best effort. On a filesystem without hard links there is simply no way back — which is worth
    // losing the undo for, and not worth refusing the update over.
    let backup = fs::hard_link(installed.path(), &backup)
        .ok()
        .map(|()| backup);

    fs::rename(staged, installed.path()).map_err(|error| {
        UpdateError::Io(format!(
            "{} → {}: {error}",
            staged.display(),
            installed.path().display()
        ))
    })?;

    // The rename is atomic; its durability is not. Until the directory itself is flushed, a crash
    // can leave the machine with no image under either name.
    if let Some(directory) = installed.path().parent()
        && let Ok(handle) = fs::File::open(directory)
    {
        let _ = handle.sync_all();
    }

    Ok(backup)
}

/// Deletes the `<name>.old` a previous update left beside the image.
///
/// Called once at startup: if the new image started, it works, and the copy is only disk space. It
/// refuses to act when the installed image is missing — then the backup is the last copy of
/// anything, and deleting it would be the one unrecoverable thing this module could do.
pub fn discard_backup(installed: &Installed) {
    let backup = installed.backup_path();
    if installed.path().is_file() && backup.is_file() {
        let _ = fs::remove_file(backup);
    }
}

/// An AppImage that is running, and therefore knows which file to replace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installed {
    path: PathBuf,
}

impl Installed {
    /// The image this process was started from, from `$APPIMAGE`.
    ///
    /// The runtime exports the path of the image itself, which is the only correct answer to "where
    /// am I": everything else is the temporary mount it unpacked itself into, gone by the next
    /// login. `None` means this is not an AppImage — a tarball install, a `cargo run` — and then
    /// there is nothing this module may replace.
    pub fn detect() -> Option<Self> {
        std::env::var_os("APPIMAGE").and_then(Self::at)
    }

    /// An installed image at `path`, resolved through symlinks.
    ///
    /// Resolved because a symlink on `PATH` pointing at the real image is a plausible way to
    /// arrange things, and replacing the symlink would leave the image untouched: an "update" that
    /// reports success and changes nothing.
    pub fn at(path: impl Into<PathBuf>) -> Option<Self> {
        let path = fs::canonicalize(path.into()).ok()?;
        path.is_file().then_some(Self { path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Where the new image is written first. Same directory on purpose: the rename has to stay
    /// within one filesystem to be atomic.
    pub fn staging_path(&self) -> PathBuf {
        with_suffix(&self.path, ".update")
    }

    /// Where the old image is kept for one start, so a bad update is a rename away from being
    /// undone instead of a download away.
    pub fn backup_path(&self) -> PathBuf {
        with_suffix(&self.path, ".old")
    }

    /// Whether the new image could be written next to this one, checked *before* anything is
    /// downloaded — "cannot write there" is worth knowing before 70 MB, and worth saying plainly
    /// when the image lives in `/opt` or belongs to root.
    pub fn writable(&self) -> Result<(), UpdateError> {
        let staged = self.staging_path();
        match fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(&staged)
        {
            Ok(_) => Ok(()),
            Err(error) => {
                let _ = fs::remove_file(&staged);
                Err(UpdateError::NotWritable(format!(
                    "{}: {error}",
                    staged.display()
                )))
            }
        }
    }
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(suffix);
    PathBuf::from(name)
}

/// A byte count as a person reads it. One decimal is enough: the difference between 70.4 and
/// 70.5 MiB tells nobody anything.
pub fn human_bytes(bytes: u64) -> String {
    const UNITS: [(&str, u64); 3] = [("МиБ", 1024 * 1024), ("КиБ", 1024), ("Б", 1)];
    for (unit, size) in UNITS {
        if bytes >= size {
            return if size == 1 {
                format!("{bytes} {unit}")
            } else {
                format!("{:.1} {unit}", bytes as f64 / size as f64)
            };
        }
    }
    "0 Б".to_string()
}

// --- the invocations this module is allowed to make -------------------------------------------

/// The flags every call shares. `--proto '=https'` and `--tlsv1.2` are not decoration: they are the
/// difference between "a URL" and "a URL that cannot quietly become something else", and `-f` turns
/// a 404 into a failure instead of an error page saved as an AppImage.
fn base_args() -> Vec<String> {
    ["-f", "-s", "-S", "-L", "--proto", "=https", "--tlsv1.2"]
        .map(str::to_string)
        .to_vec()
}

fn sums_args(url: &str) -> Vec<String> {
    let mut args = base_args();
    args.extend(["--connect-timeout", "15", "--max-time", "30"].map(str::to_string));
    args.push(url.to_string());
    args
}

fn head_args(url: &str) -> Vec<String> {
    let mut args = base_args();
    args.push("-I".to_string());
    args.extend(["--connect-timeout", "15", "--max-time", "30"].map(str::to_string));
    args.push(url.to_string());
    args
}

fn download_args(url: &str, dest: &Path) -> Vec<String> {
    let mut args = base_args();
    args.push("--connect-timeout".to_string());
    args.push("15".to_string());
    // A stalled transfer is a failure after 30 seconds below 1 KB/s, rather than a download that
    // hangs until the user gives up on the application.
    args.extend(["--speed-limit", "1024", "--speed-time", "30"].map(str::to_string));
    args.extend(["--retry", "3", "--retry-delay", "2", "--retry-connrefused"].map(str::to_string));
    args.push("-o".to_string());
    args.push(dest.to_string_lossy().into_owned());
    args.push(url.to_string());
    args
}

fn curl_text(curl: &str, url: &str) -> Result<String, UpdateError> {
    let output = Command::new(curl)
        .args(sums_args(url))
        .output()
        .map_err(|error| UpdateError::CurlMissing(error.to_string()))?;

    if !output.status.success() {
        return Err(UpdateError::CurlFailed {
            code: output.status.code(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// The check, as the console shows it. Spelled from the same argument list that actually runs, so
/// the transcript cannot drift from what was executed (§3).
pub fn describe_sums_call() -> String {
    describe(&sums_args(&sums_url()))
}

/// The download, as the console shows it.
pub fn describe_download_call(release: &Release, dest: &Path) -> String {
    describe(&download_args(&release.url(), dest))
}

fn describe(args: &[String]) -> String {
    let mut line = String::from("curl");
    for arg in args {
        line.push(' ');
        // Not shell quoting — this line is for reading, and a path with a space in it is the only
        // thing that could make it ambiguous.
        if arg.contains(' ') {
            line.push('\'');
            line.push_str(arg);
            line.push('\'');
        } else {
            line.push_str(arg);
        }
    }
    line
}

/// The asset's size, asked for before the download so progress can be a fraction. Best effort in
/// both directions: a server that answers no headers costs a percentage, nothing more.
fn head_size(curl: &str, url: &str) -> Option<u64> {
    let output = Command::new(curl).args(head_args(url)).output().ok()?;
    if !output.status.success() {
        return None;
    }

    // The *last* one: `-L` follows redirects, and every response on the way may carry headers of
    // its own. The final response is the asset.
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.trim()
                .eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse().ok())?
        })
        .next_back()
}

/// Runs the download as a child we poll rather than wait for, for two reasons: the user must be
/// able to cancel a long transfer, and the caller must stay free while it happens — the whole point
/// of this module living off the engine's thread.
fn download(
    curl: &str,
    url: &str,
    dest: &Path,
    cancel: &AtomicBool,
    mut progress: impl FnMut(u64),
) -> Result<(), UpdateError> {
    let mut child = Command::new(curl)
        .args(download_args(url, dest))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| UpdateError::CurlMissing(error.to_string()))?;

    let mut seen = 0;
    loop {
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            let _ = fs::remove_file(dest);
            return Err(UpdateError::Cancelled);
        }

        match child.try_wait() {
            Ok(Some(status)) => {
                if status.success() {
                    // A last reading, because the transfer can finish between two polls and the
                    // view should not be left showing a bar that never filled.
                    if let Ok(meta) = fs::metadata(dest)
                        && meta.len() != seen
                    {
                        progress(meta.len());
                    }
                    return Ok(());
                }
                let mut stderr = String::new();
                if let Some(mut pipe) = child.stderr.take() {
                    let _ = pipe.read_to_string(&mut stderr);
                }
                let _ = fs::remove_file(dest);
                return Err(UpdateError::CurlFailed {
                    code: status.code(),
                    stderr,
                });
            }
            Ok(None) => {
                if let Ok(meta) = fs::metadata(dest)
                    && meta.len() != seen
                {
                    seen = meta.len();
                    progress(seen);
                }
                std::thread::sleep(POLL);
            }
            Err(error) => {
                let _ = fs::remove_file(dest);
                return Err(UpdateError::Io(error.to_string()));
            }
        }
    }
}

// --- hashing and shape ------------------------------------------------------------------------

pub fn sha256_file(path: &Path) -> Result<String, UpdateError> {
    let mut file = fs::File::open(path).map_err(|error| UpdateError::Io(error.to_string()))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 64 * 1024];

    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| UpdateError::Io(error.to_string()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex(&hasher.finalize()))
}

fn hex(bytes: &[u8]) -> String {
    use fmt::Write;
    bytes.iter().fold(String::new(), |mut out, byte| {
        let _ = write!(out, "{byte:02x}");
        out
    })
}

fn read_head(path: &Path, count: usize) -> Result<Vec<u8>, UpdateError> {
    let mut file = fs::File::open(path).map_err(|error| UpdateError::Io(error.to_string()))?;
    let mut head = vec![0u8; count];
    let read = file
        .read(&mut head)
        .map_err(|error| UpdateError::Io(error.to_string()))?;
    head.truncate(read);
    Ok(head)
}

/// An AppImage is an ELF that carries its own type marker: `AI\x02` at offset 8 says "type 2",
/// which is what `packaging/appimage/build.sh` assembles (the runtime is pinned there). Type 1 is
/// not accepted, because this build has never been one and guessing otherwise would be exactly the
/// kind of assumption this project refuses to make.
pub fn looks_like_appimage(head: &[u8]) -> bool {
    head.len() >= 11 && head.starts_with(b"\x7fELF") && &head[8..11] == b"AI\x02"
}

fn describe_head(head: &[u8]) -> String {
    let text = String::from_utf8_lossy(head);
    let printable: String = text
        .chars()
        .take(16)
        .map(|c| if c.is_ascii_graphic() { c } else { '.' })
        .collect();
    format!("первые байты: {printable:?}")
}

fn make_executable(staged: &Path, like: &Path) -> Result<(), UpdateError> {
    use std::os::unix::fs::PermissionsExt;

    // The old file's mode, not a fixed 0755: whoever arranged the permissions of the image they
    // have gets the same arrangement on the one that replaces it.
    let mode = fs::metadata(like)
        .map(|meta| meta.permissions().mode())
        .unwrap_or(0o755);

    fs::set_permissions(staged, fs::Permissions::from_mode(mode))
        .map_err(|error| UpdateError::Io(format!("{}: {error}", staged.display())))
}

fn fsync(path: &Path) -> Result<(), UpdateError> {
    fs::File::open(path)
        .and_then(|handle| handle.sync_all())
        .map_err(|error| UpdateError::Io(format!("{}: {error}", path.display())))
}

/// Helpers shared with the engine's tests, which drive the same stand-in `curl` through
/// `EngineOptions`.
#[cfg(test)]
pub(crate) mod testing {
    use super::*;

    pub(crate) fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "protonvpn-gui-update-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A type-2 AppImage header and some bytes behind it.
    pub(crate) fn image_bytes() -> Vec<u8> {
        let mut bytes = b"\x7fELF\x02\x01\x01\x00AI\x02".to_vec();
        bytes.extend(std::iter::repeat_n(0u8, 64));
        bytes
    }

    pub(crate) fn install_image(dir: &Path, name: &str, bytes: &[u8]) -> Installed {
        let path = dir.join(name);
        fs::write(&path, bytes).unwrap();
        Installed::at(&path).unwrap()
    }

    /// A stand-in for `curl`, the way the engine's tests drive a stand-in CLI through
    /// `EngineOptions::program`. It answers the three calls this module makes: the checksum file,
    /// a HEAD, and a download.
    pub(crate) fn stand_in(dir: &Path, sums: &str, image: &[u8]) -> String {
        fs::write(dir.join("sums"), sums).unwrap();
        fs::write(dir.join("image"), image).unwrap();
        let script = dir.join("curl");
        fs::write(
            &script,
            r#"#!/bin/sh
here="$(dirname "$0")"
out=""
head=0
while [ $# -gt 0 ]; do
    case "$1" in
        -I) head=1 ;;
        -o) shift; out="$1" ;;
    esac
    shift
done
if [ "$head" = 1 ]; then
    printf 'HTTP/2 200\r\ncontent-length: %s\r\n\r\n' "$(wc -c < "$here/image" | tr -d ' ')"
    exit 0
fi
if [ -n "$out" ]; then
    cp "$here/image" "$out"
    exit 0
fi
cat "$here/sums"
"#,
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        script.to_string_lossy().into_owned()
    }

    pub(crate) fn sums_for(bytes: &[u8], version: Version) -> String {
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        format!(
            "{}  protonvpn-gui-{version}-x86_64-linux.tar.gz\n{}  {}\n",
            "0".repeat(64),
            hex(&hasher.finalize()),
            asset_name(version)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::testing::*;
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

    #[test]
    fn standing_never_offers_a_downgrade() {
        let latest = Version::parse("0.1.20").unwrap();
        assert_eq!(standing(None, latest), Standing::Unversioned);
        assert_eq!(
            standing(Version::parse("0.1.19"), latest),
            Standing::Behind(latest)
        );
        assert_eq!(standing(Some(latest), latest), Standing::Current);
        // A build from `main` between releases is ahead, not behind by a negative amount.
        assert_eq!(
            standing(Version::parse("0.1.21"), latest),
            Standing::Current
        );
    }

    #[test]
    fn the_release_page_is_the_repository_this_crate_comes_from() {
        assert!(
            env!("CARGO_PKG_REPOSITORY").ends_with(REPO_SLUG),
            "the updater asks {REPO_SLUG}, but this crate says it comes from {}",
            env!("CARGO_PKG_REPOSITORY")
        );
    }

    #[test]
    fn the_urls_are_https_and_ours() {
        for url in [
            sums_url(),
            asset_url(Version::parse("0.1.20").unwrap(), "x"),
        ] {
            assert!(url.starts_with("https://github.com/trousev/protonvpn-gui/releases/"));
            assert!(!url.contains(' '));
        }
    }

    #[test]
    fn the_asset_name_is_the_one_the_packaging_script_writes() {
        // `packaging/appimage/build.sh` names the image `ProtonVPN-GUI-$VERSION-$ARCH.AppImage`, and
        // this module learns the version by reading that name back out of SHA256SUMS. The two
        // spellings are the contract; this pins the shape they share.
        let name = asset_name(Version::parse("0.1.20").unwrap());
        assert!(name.starts_with("ProtonVPN-GUI-0.1.20-"));
        assert!(name.ends_with(".AppImage"));
    }

    #[test]
    fn reads_our_image_out_of_a_checksum_file() {
        let version = Version::parse("0.1.20").unwrap();
        let body = format!(
            "{}  protonvpn-gui-0.1.20-x86_64-linux.tar.gz\n{}  {}\n",
            "a".repeat(64),
            "B".repeat(64),
            asset_name(version)
        );

        let release = parse_sums(&body).unwrap();
        assert_eq!(release.version, version);
        assert_eq!(release.sha256, "b".repeat(64), "hex case must not matter");
        assert_eq!(release.asset, asset_name(version));
        assert!(
            release
                .url()
                .ends_with(&format!("/0.1.20/{}", asset_name(version)))
        );
    }

    #[test]
    fn ignores_images_for_other_machines_and_anything_that_is_not_one() {
        let body = format!(
            "{}  ProtonVPN-GUI-0.1.20-riscv64.AppImage\n{}  ProtonVPN-GUI-latest-x86_64.AppImage\n{}  ProtonVPN-GUI-0.1.20-x86_64.zip\n",
            "a".repeat(64),
            "b".repeat(64),
            "c".repeat(64)
        );
        let error = parse_sums(&body).unwrap_err();
        assert!(matches!(error, UpdateError::NoAsset(_)), "{error:?}");

        // A hash that is not a hash is not a release to install from.
        let bad = format!(
            "not-a-hash  {}\n",
            asset_name(Version::parse("0.1.20").unwrap())
        );
        assert!(parse_sums(&bad).is_err());
    }

    #[test]
    fn a_release_from_before_the_naming_change_is_reported_and_not_guessed_at() {
        // The real `SHA256SUMS` of release 0.1.20 — the last one published before the asset's name
        // carried the version. The updater learns the version *from that name*, so this release
        // cannot be installed from, and saying so is the only honest answer. It also pins the
        // format against a file a real release produced rather than one a test wrote.
        let body = "22a73dd65dae76738b301b2913a274b15331cc32f678aab3eeb0f344556673a6  protonvpn-gui-0.1.20-x86_64-linux.tar.gz\n70c8c123460c7d127d05d25b2b676965f4548a823467114e8615094c0b3cb8a5  ProtonVPN-GUI-x86_64.AppImage\n";
        let error = parse_sums(body).unwrap_err();
        assert!(matches!(error, UpdateError::NoAsset(_)), "{error:?}");
    }

    #[test]
    fn binary_mode_and_extra_whitespace_are_still_sha256sum_output() {
        let version = Version::parse("0.1.7").unwrap();
        let body = format!("{} *{}\n", "d".repeat(64), asset_name(version));
        let release = parse_sums(&body).unwrap();
        assert_eq!(
            release.asset,
            asset_name(version),
            "the `*` is not part of the name"
        );
    }

    #[test]
    fn only_a_type_two_appimage_is_accepted() {
        assert!(looks_like_appimage(&image_bytes()));
        assert!(
            !looks_like_appimage(b"\x7fELF\x02\x01\x01\x00AI\x01"),
            "type 1"
        );
        assert!(!looks_like_appimage(b"<!DOCTYPE html>"));
        assert!(!looks_like_appimage(b"\x7fELF"));
        assert!(!looks_like_appimage(b""));
    }

    #[test]
    fn hashes_a_file_the_way_sha256sum_does() {
        let dir = temp_dir("hash");
        let path = dir.join("abc");
        fs::write(&path, b"abc").unwrap();
        assert_eq!(
            sha256_file(&path).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn the_staging_file_and_the_backup_live_next_to_the_image() {
        let dir = temp_dir("paths");
        let installed = install_image(&dir, "protonvpn-gui.AppImage", &image_bytes());
        assert_eq!(
            installed.staging_path(),
            dir.join("protonvpn-gui.AppImage.update")
        );
        assert_eq!(
            installed.backup_path(),
            dir.join("protonvpn-gui.AppImage.old")
        );
    }

    #[test]
    fn a_symlink_to_the_image_is_resolved_before_anything_is_replaced() {
        let dir = temp_dir("symlink");
        let real = install_image(&dir, "real.AppImage", &image_bytes());
        let link = dir.join("on-path.AppImage");
        std::os::unix::fs::symlink(real.path(), &link).unwrap();

        let installed = Installed::at(&link).unwrap();
        assert_eq!(installed.path(), real.path());
    }

    #[test]
    fn the_check_downloads_and_verifies_before_anything_is_installed() {
        let dir = temp_dir("fetch");
        let bytes = image_bytes();
        let version = Version::parse("0.1.20").unwrap();
        let curl = stand_in(&dir, &sums_for(&bytes, version), &bytes);
        let installed = install_image(&dir, "ProtonVPN-GUI-0.1.19-x86_64.AppImage", &image_bytes());

        let release = latest(&curl).unwrap();
        assert_eq!(release.version, version);

        let mut progress = Vec::new();
        let staged = fetch(
            &curl,
            &installed,
            &release,
            &AtomicBool::new(false),
            |got, total| progress.push((got, total)),
        )
        .unwrap();

        assert_eq!(fs::read(&staged).unwrap(), bytes);
        assert_eq!(progress.first(), Some(&(0, Some(bytes.len() as u64))));
        assert_eq!(progress.last().unwrap().0, bytes.len() as u64);
        // Staged, not installed: the image the user is running is untouched until they say so.
        assert_eq!(fs::read(installed.path()).unwrap(), image_bytes());
    }

    #[test]
    fn a_download_that_does_not_match_the_published_checksum_is_thrown_away() {
        let dir = temp_dir("mismatch");
        let bytes = image_bytes();
        let version = Version::parse("0.1.20").unwrap();
        // The checksum file describes different bytes than the server hands over.
        let other = [image_bytes(), b"extra".to_vec()].concat();
        let curl = stand_in(&dir, &sums_for(&other, version), &bytes);
        let installed = install_image(&dir, "app.AppImage", &image_bytes());

        let release = latest(&curl).unwrap();
        let error = fetch(
            &curl,
            &installed,
            &release,
            &AtomicBool::new(false),
            |_, _| {},
        )
        .unwrap_err();
        assert!(
            matches!(error, UpdateError::ChecksumMismatch { .. }),
            "{error:?}"
        );
        assert!(
            !installed.staging_path().exists(),
            "the partial file must not survive"
        );
        assert_eq!(fs::read(installed.path()).unwrap(), image_bytes());
    }

    #[test]
    fn a_correct_hash_of_the_wrong_document_is_still_refused() {
        let dir = temp_dir("not-an-image");
        let page = b"<!DOCTYPE html><html>captive portal</html>".to_vec();
        let version = Version::parse("0.1.20").unwrap();
        let curl = stand_in(&dir, &sums_for(&page, version), &page);
        let installed = install_image(&dir, "app.AppImage", &image_bytes());

        let release = latest(&curl).unwrap();
        let error = fetch(
            &curl,
            &installed,
            &release,
            &AtomicBool::new(false),
            |_, _| {},
        )
        .unwrap_err();
        assert!(matches!(error, UpdateError::NotAnAppImage(_)), "{error:?}");
        assert!(!installed.staging_path().exists());
    }

    #[test]
    fn a_download_can_be_cancelled_and_leaves_nothing_behind() {
        let dir = temp_dir("cancel");
        let bytes = image_bytes();
        let version = Version::parse("0.1.20").unwrap();
        let curl = stand_in(&dir, &sums_for(&bytes, version), &bytes);
        let installed = install_image(&dir, "app.AppImage", &image_bytes());

        let release = latest(&curl).unwrap();
        // Cancelled before it starts, which is the deterministic version of cancelling halfway.
        let cancel = AtomicBool::new(true);
        let error = fetch(&curl, &installed, &release, &cancel, |_, _| {}).unwrap_err();
        assert_eq!(error, UpdateError::Cancelled);
        assert!(!installed.staging_path().exists());
    }

    #[test]
    fn installing_replaces_the_file_atomically_and_keeps_a_way_back() {
        let dir = temp_dir("install");
        let old = image_bytes();
        let mut new = image_bytes();
        new.extend_from_slice(b"the new one");
        let installed = install_image(&dir, "app.AppImage", &old);
        let staged = installed.staging_path();
        fs::write(&staged, &new).unwrap();

        let backup = install(&installed, &staged).unwrap().expect("a way back");

        assert_eq!(fs::read(installed.path()).unwrap(), new);
        assert_eq!(fs::read(&backup).unwrap(), old);
        assert!(
            !staged.exists(),
            "the staging file became the installed one"
        );

        // The next start is proof the new image works, so the copy goes.
        discard_backup(&installed);
        assert!(!backup.exists());
    }

    #[test]
    fn the_backup_is_never_deleted_when_the_image_it_replaced_is_gone() {
        let dir = temp_dir("keep-backup");
        let installed = install_image(&dir, "app.AppImage", &image_bytes());
        let backup = installed.backup_path();
        fs::write(&backup, image_bytes()).unwrap();
        fs::remove_file(installed.path()).unwrap();

        discard_backup(&installed);
        assert!(backup.exists(), "the last copy of anything must survive");
    }

    #[test]
    fn an_image_that_cannot_be_written_next_to_is_reported_before_any_download() {
        use std::os::unix::fs::PermissionsExt;

        let dir = temp_dir("readonly");
        let installed = install_image(&dir, "app.AppImage", &image_bytes());
        let mut permissions = fs::metadata(&dir).unwrap().permissions();
        permissions.set_mode(0o500);
        fs::set_permissions(&dir, permissions).unwrap();

        let error = installed.writable().unwrap_err();
        assert!(matches!(error, UpdateError::NotWritable(_)), "{error:?}");

        // Put it back, or the temporary directory survives the test unremovable by its owner.
        let mut permissions = fs::metadata(&dir).unwrap().permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&dir, permissions).unwrap();
    }

    #[test]
    fn a_file_that_is_not_an_appimage_is_still_a_file() {
        let dir = temp_dir("not-appimage");
        let path = dir.join("protonvpn-gui");
        fs::write(&path, b"#!/bin/sh\n").unwrap();
        // `Installed::at` answers "is this a file", not "is this an AppImage": a tarball install is
        // a perfectly good file, and it is `detect` that only ever sees an AppImage.
        assert!(Installed::at(&path).is_some());
        assert!(Installed::at(dir.join("missing")).is_none());
    }

    #[test]
    fn the_invocations_are_curl_and_nothing_else() {
        let url = sums_url();
        for args in [sums_args(&url), head_args(&url)] {
            assert_eq!(args.last().unwrap(), &url);
            assert!(args.contains(&"--proto".to_string()));
            assert!(args.contains(&"=https".to_string()));
            assert!(args.contains(&"--tlsv1.2".to_string()));
            assert!(!args.iter().any(|arg| arg.contains("sh")), "{args:?}");
        }

        let dest = PathBuf::from("/tmp/x.update");
        let args = download_args(&url, &dest);
        assert!(args.windows(2).any(|pair| pair == ["-o", "/tmp/x.update"]));
        assert!(
            args.contains(&"--speed-time".to_string()),
            "a stall must end"
        );
    }
}
