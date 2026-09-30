//! XDG autostart — a plain `~/.config/autostart/*.desktop`, no portal.
//!
//! `docs/plan.md` Phase 3: Flatpak was dropped, so there is nothing sandboxed about this. The file
//! is ours, written only when the user asks for it, and the app's own config
//! (`~/.config/protonvpn-gui/config.json`) records the intent. Proton's `app-config.json` is never
//! touched — it belongs to the official app (`docs/architecture.md` §0).

use std::fs;
use std::io;
use std::path::PathBuf;

/// Basename of the desktop entry, and of the installed icon.
pub const DESKTOP_FILE: &str = "protonvpn-gui.desktop";
pub const APP_ID: &str = "protonvpn-gui";

/// `$XDG_CONFIG_HOME/autostart/protonvpn-gui.desktop`, or `~/.config/autostart/…`.
pub fn autostart_path() -> PathBuf {
    config_home().join("autostart").join(DESKTOP_FILE)
}

fn config_home() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."))
}

/// The desktop entry. `exec` is the absolute path to this binary.
///
/// `StartupWMClass` matters because the window's app id is ours and must never be Proton's: the
/// CLI refuses to run while `proton.vpn.app.gtk` is on the session bus (`docs/cli-surface.md` §2).
pub fn desktop_entry(exec: &str) -> String {
    format!(
        "\
[Desktop Entry]
Type=Application
Version=1.0
Name=Proton VPN GUI
Comment=Console-first wrapper around the official protonvpn CLI
Exec={exec}
Icon={APP_ID}
Terminal=false
Categories=Network;Security;
Keywords=VPN;Proton;protonvpn;
StartupNotify=false
StartupWMClass={APP_ID}
X-GNOME-Autostart-enabled=true
"
    )
}

/// Writes or removes the autostart entry, keeping it in step with the config flag.
pub fn sync(enabled: bool, exec: &str) -> io::Result<Option<PathBuf>> {
    let path = autostart_path();
    if enabled {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, desktop_entry(exec))?;
        Ok(Some(path))
    } else {
        match fs::remove_file(&path) {
            Ok(()) => Ok(None),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }
}

pub fn is_enabled() -> bool {
    autostart_path().exists()
}

/// The running binary's path, for the `Exec=` line.
pub fn current_exec() -> String {
    std::env::current_exe()
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_else(|_| APP_ID.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_entry_is_an_autostart_desktop_file_with_our_own_app_id() {
        let entry = desktop_entry("/usr/bin/protonvpn-gui");
        assert!(entry.starts_with("[Desktop Entry]"));
        assert!(entry.contains("Type=Application"));
        assert!(entry.contains("Exec=/usr/bin/protonvpn-gui"));
        assert!(entry.contains("X-GNOME-Autostart-enabled=true"));
        assert!(entry.contains("StartupWMClass=protonvpn-gui"));
        // The one bus name we must never claim.
        assert!(!entry.contains("proton.vpn.app.gtk"));
        assert!(!entry.contains("proton-vpn-app"));
    }

    #[test]
    fn the_path_is_the_xdg_autostart_directory() {
        let path = autostart_path();
        assert!(
            path.ends_with("autostart/protonvpn-gui.desktop"),
            "{}",
            path.display()
        );
    }
}
