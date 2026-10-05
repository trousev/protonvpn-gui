//! The two desktop entries we own: the autostart one, and the one that gives the window its name
//! and its icon.
//!
//! `docs/architecture.md` §12. The second entry is not decoration. Wayland has no window icons —
//! for the same reason winit's `set_visible` is "not possible on Wayland", there is no
//! `_NET_WM_ICON` to set — so a compositor learns what a window *is* by matching its app id (X11:
//! `WM_CLASS`) against the basename of a `.desktop` file. With no file to match, GNOME builds a
//! window-backed application around the window and shows it as an unknown program under the
//! generic `application-x-executable` icon: the gear, and no name.
//!
//! Both files are written only where a desktop looks for them — `$XDG_CONFIG_HOME/autostart` and
//! `$XDG_DATA_HOME/{applications,icons/hicolor}` — and both are ours. Proton's `app-config.json`
//! is never touched; it belongs to the official app (§0).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use protonvpn_core::i18n::{I18n, Locale};

/// Basename of the desktop entry, and of the installed icon. It is also the window's app id: a
/// compositor matches one against the other, and that match is the whole point. It must never be
/// `proton.vpn.app.gtk`, which is Proton's own name and would stop the CLI from running
/// (`docs/cli-surface.md` §2).
pub const APP_ID: &str = "protonvpn-gui";
pub const DESKTOP_FILE: &str = "protonvpn-gui.desktop";

/// The icon, byte for byte the file `packaging/` ships and the AppImage installs. The path leaves
/// this crate on purpose: the icon is packaging metadata, and one copy of it is enough — the
/// build would rather fail to compile than quietly ship a second, different one.
const ICON: &[u8] = include_bytes!("../../../packaging/icons/protonvpn-gui.png");
const ICON_FILE: &str = "protonvpn-gui.png";
/// The size bucket of the hicolor theme the icon is installed under. The file is 256×256; a test
/// pins that, so this cannot drift away from the bytes above.
const ICON_SIZE: u32 = 256;

/// Where the two entries live.
///
/// `Default` reads the XDG environment, which is what the application wants; tests build one over
/// a temporary directory, the way `ConfigStore::at` does for the config.
#[derive(Debug, Clone)]
pub struct Desktop {
    config_home: PathBuf,
    data_home: PathBuf,
}

impl Default for Desktop {
    fn default() -> Self {
        Self::at(
            env_home("XDG_CONFIG_HOME", ".config"),
            env_home("XDG_DATA_HOME", ".local/share"),
        )
    }
}

impl Desktop {
    pub fn at(config_home: impl Into<PathBuf>, data_home: impl Into<PathBuf>) -> Self {
        Self {
            config_home: config_home.into(),
            data_home: data_home.into(),
        }
    }

    /// `$XDG_CONFIG_HOME/autostart/protonvpn-gui.desktop`, or `~/.config/autostart/…`.
    pub fn autostart_path(&self) -> PathBuf {
        self.config_home.join("autostart").join(DESKTOP_FILE)
    }

    /// `$XDG_DATA_HOME/applications/protonvpn-gui.desktop`, or `~/.local/share/applications/…`.
    pub fn entry_path(&self) -> PathBuf {
        self.data_home.join("applications").join(DESKTOP_FILE)
    }

    /// `$XDG_DATA_HOME/icons/hicolor/256x256/apps/protonvpn-gui.png`, or under `~/.local/share`.
    ///
    /// This is both the file [`Desktop::sync_entry`] writes and the path the entry's `Icon=` key
    /// spells out, so the icon is never *looked up* — see [`entry`] for why that matters. It also
    /// stays where the icon theme spec says an application icon belongs, because the packaged
    /// entry, which has no user's home to point at, can only name it.
    pub fn icon_path(&self) -> PathBuf {
        self.data_home
            .join("icons")
            .join("hicolor")
            .join(format!("{ICON_SIZE}x{ICON_SIZE}"))
            .join("apps")
            .join(ICON_FILE)
    }

    /// Writes or removes the application entry and its icon.
    ///
    /// `Ok(Some(path))` means the entry is installed, `Ok(None)` that it was removed. Both files
    /// are left alone when their content already matches, so an ordinary start does not touch
    /// mtimes.
    pub fn sync_entry(
        &self,
        enabled: bool,
        exec: &str,
        i18n: &I18n,
    ) -> io::Result<Option<PathBuf>> {
        let entry_path = self.entry_path();
        let icon_path = self.icon_path();

        if !enabled {
            remove_if_present(&entry_path)?;
            remove_if_present(&icon_path)?;
            return Ok(None);
        }

        write_if_changed(&entry_path, entry(exec, &icon_path, i18n).as_bytes())?;
        write_if_changed(&icon_path, ICON)?;
        Ok(Some(entry_path))
    }

    /// Writes or removes the autostart entry: [`Desktop::sync_entry`]'s contract for the other
    /// file, which is the same entry plus the one key XDG autostart adds.
    pub fn sync_autostart(
        &self,
        enabled: bool,
        exec: &str,
        i18n: &I18n,
    ) -> io::Result<Option<PathBuf>> {
        let path = self.autostart_path();
        if enabled {
            write_if_changed(
                &path,
                autostart_entry(exec, &self.icon_path(), i18n).as_bytes(),
            )?;
            Ok(Some(path))
        } else {
            remove_if_present(&path)?;
            Ok(None)
        }
    }

    /// Whether the shell can currently find the entry. Checked rather than remembered, because
    /// the user may have deleted the file.
    pub fn entry_installed(&self) -> bool {
        self.entry_path().is_file() && self.icon_path().is_file()
    }

    pub fn autostart_enabled(&self) -> bool {
        self.autostart_path().exists()
    }
}

/// The application entry: what a desktop reads to name our window and draw its icon.
///
/// `Icon=` is the **absolute path** of the file the app installs, never the bare name
/// `protonvpn-gui`. A name is an icon-theme lookup, and GTK answers a lookup of
/// `~/.local/share/icons/hicolor` out of that directory's `icon-theme.cache` when one is there —
/// a listing that nothing rebuilds when a file appears underneath it. Measured: with a cache
/// dated 2026-09-27 and our PNG written 2026-10-05, `Gtk.IconTheme.has_icon("protonvpn-gui")`
/// was false in a fresh GTK 3 and a fresh GTK 4 process, while `steam.png` and the `chrome-*`
/// icons in the same directory resolved — they predate the cache; removing or rebuilding it made
/// ours resolve too. GNOME Shell 50 drew the gear for exactly that reason. A path needs no theme,
/// no cache and no cooperation, and every desktop accepts one.
pub fn entry(exec: &str, icon: &Path, i18n: &I18n) -> String {
    format!(
        "\
[Desktop Entry]
Type=Application
Version=1.0
{identity}Exec={exec}
Icon={icon}
Terminal=false
Categories=Network;Security;
Keywords=VPN;Proton;protonvpn;
StartupNotify=false
StartupWMClass={APP_ID}
",
        identity = identity_keys(i18n),
        exec = exec_value(exec),
        icon = icon_value(icon),
    )
}

/// The three keys that name the application, in every language this build carries.
///
/// The freedesktop spec localizes a key by suffixing it with the language (`Name[ru]=…`), and a
/// desktop picks the one matching its own locale — which is why every language is written at once
/// rather than the selected one. This file belongs to the desktop, not to the running process: it
/// is read when a menu is built, by a program that has never heard of our catalogue, and rewriting
/// it on a language change would only move the problem to whoever is not looking.
///
/// The unsuffixed key is the fallback the spec names and the one a desktop with no matching
/// translation reads, so it is always written. `build.rs` is what guarantees the suffixed ones are
/// all there is to write.
fn identity_keys(i18n: &I18n) -> String {
    let mut out = String::new();
    for (key, id, source) in [
        ("Name", "desktop-name", i18n.desktop_name()),
        (
            "GenericName",
            "desktop-generic-name",
            i18n.desktop_generic_name(),
        ),
        ("Comment", "desktop-comment", i18n.desktop_comment()),
    ] {
        out.push_str(&format!("{key}={source}\n"));
        for locale in Locale::ALL.iter().copied().filter(|l| *l != Locale::SOURCE) {
            // Not the selected language's value: the suffix names the language, so the text has to
            // be that language's.
            let value = i18n.localized(locale, id).unwrap_or_else(|| source.clone());
            out.push_str(&format!("{key}[{}]={value}\n", locale.id()));
        }
    }
    out
}

/// The autostart entry. XDG autostart is the same file with one extra key — nothing in it depends
/// on what autostart adds, so this stays one template instead of two that drift apart.
pub fn autostart_entry(exec: &str, icon: &Path, i18n: &I18n) -> String {
    format!(
        "{}X-GNOME-Autostart-enabled=true\n",
        entry(exec, icon, i18n)
    )
}

/// The running program, as a `.desktop` file has to spell it.
///
/// An AppImage unpacks itself into a temporary mount that is gone by the next login, so the image
/// is the thing worth writing; its runtime exports that path in `$APPIMAGE`. Anything else is the
/// binary we were started from.
pub fn current_exec() -> String {
    std::env::var_os("APPIMAGE")
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::current_exe().ok())
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_else(|| APP_ID.to_string())
}

/// `$XDG_*` if it is absolute, `$HOME/<fallback>` otherwise, and the current directory as a last
/// resort — the same rules the config follows, so both ends of the app agree on where home is.
fn env_home(variable: &str, fallback: &str) -> PathBuf {
    std::env::var_os(variable)
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(fallback)))
        .unwrap_or_else(|| PathBuf::from("."))
}

/// A path as the `Exec` key spells it.
///
/// This is not shell quoting. Inside the double quotes the spec defines, the reserved characters
/// are `"`, backtick, `$` and `\`, and a literal `%` is a field code and has to be doubled.
/// Getting this wrong is quiet: a path with a space in it becomes two arguments, and the entry
/// starts something else, or nothing.
fn exec_value(path: &str) -> String {
    let escaped = path.replace('%', "%%");

    if !escaped.contains([' ', '\t', '"', '\'', '\\', '$', '`']) {
        return escaped;
    }

    let mut quoted = String::with_capacity(escaped.len() + 2);
    quoted.push('"');
    for character in escaped.chars() {
        if matches!(character, '"' | '`' | '$' | '\\') {
            quoted.push('\\');
        }
        quoted.push(character);
    }
    quoted.push('"');
    quoted
}

/// A path as the `Icon` key spells it.
///
/// This is not [`exec_value`]: `Icon` has no quoting rules, and a space in the path is just a
/// space. What it does share is a key file's escapes — `\\`, `\n`, `\t` and `\r` are read back as
/// something else — so those four are written the long way. A path without them passes through
/// untouched, which is every path anyone actually has.
fn icon_value(path: &Path) -> String {
    let mut value = String::with_capacity(path.as_os_str().len());

    for character in path.to_string_lossy().chars() {
        match character {
            '\\' => value.push_str("\\\\"),
            '\n' => value.push_str("\\n"),
            '\t' => value.push_str("\\t"),
            '\r' => value.push_str("\\r"),
            _ => value.push(character),
        }
    }
    value
}

fn remove_if_present(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

/// Writes `bytes` to `path` unless they are already there, creating the directories on the way.
fn write_if_changed(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Ok(current) = fs::read(path)
        && current == bytes
    {
        return Ok(());
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The source language, which is what these tests assert on. `entry` writes a key per
    /// language, so the rest of the catalogue is exercised by
    /// `the_packaged_entry_says_the_same_thing_as_the_generated_one`.
    fn i18n() -> I18n {
        I18n::new(Locale::SOURCE)
    }

    /// A `Desktop` over a directory of its own, the way the config tests make a `ConfigStore`.
    fn temp_desktop(name: &str) -> Desktop {
        let dir =
            std::env::temp_dir().join(format!("protonvpn-gui-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        Desktop::at(dir.join("config"), dir.join("data"))
    }

    #[test]
    fn the_entry_is_an_application_entry_with_our_own_app_id() {
        let icon = Path::new("/home/u/.local/share/icons/hicolor/256x256/apps/protonvpn-gui.png");
        let entry = entry("/usr/bin/protonvpn-gui", icon, &i18n());
        assert!(entry.starts_with("[Desktop Entry]"));
        assert!(entry.contains("Type=Application"));
        assert!(entry.contains("Exec=/usr/bin/protonvpn-gui"));
        // A path, never the name `protonvpn-gui`: a name is a theme lookup, and a theme lookup is
        // answered from a cache that nothing rebuilds. `entry`'s doc has the measurement.
        assert!(
            entry
                .contains("Icon=/home/u/.local/share/icons/hicolor/256x256/apps/protonvpn-gui.png")
        );
        assert!(!entry.contains("Icon=protonvpn-gui\n"));
        assert!(entry.contains("StartupWMClass=protonvpn-gui"));
        assert!(entry.contains("Categories=Network;Security;"));
        // The one bus name we must never claim.
        assert!(!entry.contains("proton.vpn.app.gtk"));
        assert!(!entry.contains("proton-vpn-app"));
    }

    #[test]
    fn the_autostart_entry_is_the_same_entry_plus_the_autostart_key() {
        let icon = Path::new("/home/u/.local/share/icons/hicolor/256x256/apps/protonvpn-gui.png");
        let autostart = autostart_entry("/usr/bin/protonvpn-gui", icon, &i18n());
        assert!(autostart.contains("X-GNOME-Autostart-enabled=true"));
        // One template: dropping the extra key has to leave exactly the application entry.
        assert_eq!(
            autostart.replace("X-GNOME-Autostart-enabled=true\n", ""),
            self::entry("/usr/bin/protonvpn-gui", icon, &i18n())
        );
    }

    #[test]
    fn every_language_is_written_into_the_entry() {
        // The file is read by a desktop that has never heard of our catalogue, so it is written
        // with one key per language and the desktop picks the one its own locale matches. The
        // unsuffixed key is the fallback, and English is what it holds.
        let icon = Path::new("/home/u/.local/share/icons/hicolor/256x256/apps/protonvpn-gui.png");
        let entry = entry("/usr/bin/protonvpn-gui", icon, &i18n());

        assert!(entry.contains("Name=Proton VPN GUI\n"));
        assert!(entry.contains("GenericName=VPN client\n"));
        assert!(
            entry.contains("Comment=Console-first wrapper around the official protonvpn CLI\n")
        );

        for locale in Locale::ALL.iter().copied().filter(|l| *l != Locale::SOURCE) {
            let theirs = I18n::new(locale);
            for (key, value) in [
                ("Name", theirs.desktop_name()),
                ("GenericName", theirs.desktop_generic_name()),
                ("Comment", theirs.desktop_comment()),
            ] {
                let line = format!("{key}[{}]={value}\n", locale.id());
                assert!(entry.contains(&line), "{line} is missing from:\n{entry}");
            }
        }
    }

    #[test]
    fn the_packaged_entry_says_the_same_thing_as_the_generated_one() {
        // The AppImage ships `packaging/protonvpn-gui.desktop`; the app installs what `entry`
        // builds. They may differ in `Exec` (the package is started as `protonvpn-gui`, the
        // installed one knows its own path) and in `Icon` — a package is built before anyone's
        // home exists, so it can only name the icon and expect the installer to place the file in
        // the system theme, where a package manager also maintains the cache; the installed entry
        // points at its own copy instead. In nothing else may they differ: a name that drifted
        // apart would brand the window one way and the menu another.
        let packaged = include_str!("../../../packaging/protonvpn-gui.desktop");
        let generated = entry(
            "protonvpn-gui",
            Path::new("/home/u/.local/share/icons/hicolor/256x256/apps/protonvpn-gui.png"),
            &i18n(),
        );

        fn fields(text: &str) -> Vec<(String, String)> {
            let mut fields: Vec<(String, String)> = text
                .lines()
                .filter_map(|line| line.split_once('='))
                .map(|(key, value)| (key.to_string(), value.to_string()))
                .collect();
            fields.sort();
            fields
        }

        let mut packaged_fields = fields(packaged);
        let mut generated_fields = fields(&generated);
        for fields in [&mut packaged_fields, &mut generated_fields] {
            fields.retain(|(key, _)| key != "Exec" && key != "Icon");
        }
        assert_eq!(packaged_fields, generated_fields);

        // The exclusion above may only hide the difference just described: one names the icon,
        // the other spells it as a path, and it is the packaged one that names it.
        assert!(packaged.contains("Icon=protonvpn-gui\n"));
        assert!(generated.contains("Icon=/"));
    }

    #[test]
    fn a_path_is_quoted_exactly_when_the_exec_key_needs_it() {
        assert_eq!(
            exec_value("/usr/bin/protonvpn-gui"),
            "/usr/bin/protonvpn-gui"
        );
        assert_eq!(
            exec_value("/home/a/My Apps/Proton.AppImage"),
            "\"/home/a/My Apps/Proton.AppImage\""
        );
        // Reserved inside the quotes, and `%` is a field code wherever it appears.
        assert_eq!(exec_value("/a$b/c`d"), "\"/a\\$b/c\\`d\"");
        assert_eq!(exec_value("/100%/app"), "/100%%/app");
    }

    #[test]
    fn the_installed_entry_names_the_icon_the_app_installed() {
        // The property the whole arrangement exists for: what the entry says and what is on disk
        // are the same file, spelled as a path. A theme that never rebuilds its cache cannot
        // come between them.
        let desktop = temp_desktop("entry-icon");
        desktop
            .sync_entry(true, "/opt/Proton VPN.AppImage", &i18n())
            .unwrap();

        let icon = desktop.icon_path();
        assert!(icon.is_absolute(), "{}", icon.display());
        assert!(icon.is_file(), "{}", icon.display());

        let written = fs::read_to_string(desktop.entry_path()).unwrap();
        assert!(
            written.contains(&format!("Icon={}\n", icon.display())),
            "{written}"
        );

        let _ = fs::remove_dir_all(desktop.entry_path().parent().unwrap().parent().unwrap());
    }

    #[test]
    fn a_path_is_escaped_for_the_icon_key_but_never_quoted() {
        // This key has no quoting rules: a space is a space, and `$` and `%` mean nothing.
        assert_eq!(
            icon_value(Path::new("/home/a/My Icons/x.png")),
            "/home/a/My Icons/x.png"
        );
        assert_eq!(
            icon_value(Path::new("/home/a$b/100%/x.png")),
            "/home/a$b/100%/x.png"
        );
        // What it does have is a key file's escapes, and those are read back as something else.
        assert_eq!(
            icon_value(Path::new(r"/home/a\b/x.png")),
            r"/home/a\\b/x.png"
        );
        assert_eq!(
            icon_value(Path::new("/home/a\nb/x.png")),
            r"/home/a\nb/x.png"
        );
        assert_eq!(
            icon_value(Path::new("/home/a\tb/x.png")),
            r"/home/a\tb/x.png"
        );
    }

    #[test]
    fn installing_and_removing_the_entry_leaves_nothing_behind() {
        let desktop = temp_desktop("entry");

        let installed = desktop
            .sync_entry(true, "/opt/Proton VPN.AppImage", &i18n())
            .unwrap();
        assert_eq!(installed.as_deref(), Some(desktop.entry_path().as_path()));
        let written = fs::read_to_string(desktop.entry_path()).unwrap();
        assert!(written.contains("Exec=\"/opt/Proton VPN.AppImage\""));
        assert!(desktop.icon_path().is_file());
        assert!(desktop.entry_installed());

        // A second sync with the same content is a no-op, not a rewrite.
        let before = fs::metadata(desktop.entry_path())
            .unwrap()
            .modified()
            .unwrap();
        desktop
            .sync_entry(true, "/opt/Proton VPN.AppImage", &i18n())
            .unwrap();
        let after = fs::metadata(desktop.entry_path())
            .unwrap()
            .modified()
            .unwrap();
        assert_eq!(before, after);

        assert_eq!(
            desktop
                .sync_entry(false, "/opt/Proton VPN.AppImage", &i18n())
                .unwrap(),
            None
        );
        assert!(!desktop.entry_path().exists());
        assert!(!desktop.icon_path().exists());
        assert!(!desktop.entry_installed());
        // Removing what is not there is not an error: the user may have deleted it already.
        assert_eq!(desktop.sync_entry(false, "x", &i18n()).unwrap(), None);

        let _ = fs::remove_dir_all(desktop.entry_path().parent().unwrap().parent().unwrap());
    }

    #[test]
    fn the_autostart_entry_is_written_and_removed_the_same_way() {
        let desktop = temp_desktop("autostart");

        desktop
            .sync_autostart(true, "/usr/bin/protonvpn-gui", &i18n())
            .unwrap();
        assert!(desktop.autostart_enabled());
        assert!(
            fs::read_to_string(desktop.autostart_path())
                .unwrap()
                .contains("X-GNOME-Autostart-enabled=true")
        );

        desktop
            .sync_autostart(false, "/usr/bin/protonvpn-gui", &i18n())
            .unwrap();
        assert!(!desktop.autostart_enabled());
        assert_eq!(desktop.sync_autostart(false, "x", &i18n()).unwrap(), None);

        let _ = fs::remove_dir_all(desktop.autostart_path().parent().unwrap().parent().unwrap());
    }

    #[test]
    fn the_paths_are_the_ones_a_desktop_searches() {
        let desktop = Desktop::at("/home/u/.config", "/home/u/.local/share");
        assert!(
            desktop
                .autostart_path()
                .ends_with("autostart/protonvpn-gui.desktop")
        );
        assert!(
            desktop
                .entry_path()
                .ends_with("applications/protonvpn-gui.desktop")
        );
        assert!(
            desktop
                .icon_path()
                .ends_with("icons/hicolor/256x256/apps/protonvpn-gui.png")
        );
    }

    #[test]
    fn the_installed_icon_is_the_packaging_icon_and_is_a_256_png() {
        // The theme bucket above is a claim about these bytes; check it instead of trusting it.
        assert_eq!(&ICON[..8], b"\x89PNG\r\n\x1a\n");
        let width = u32::from_be_bytes(ICON[16..20].try_into().unwrap());
        let height = u32::from_be_bytes(ICON[20..24].try_into().unwrap());
        assert_eq!((width, height), (ICON_SIZE, ICON_SIZE));
    }

    #[test]
    fn the_running_program_is_spelled_as_an_absolute_path() {
        // `current_exec` reads the environment, so this pins only the fallback shape: without
        // `$APPIMAGE` it is this test binary's path.
        let exec = current_exec();
        assert!(exec.starts_with('/') || exec == APP_ID, "{exec}");
    }
}
