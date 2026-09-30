//! Our own configuration — the only settings file this project reads.
//!
//! Proton's `app-config.json` and `settings.json` are **forbidden** (`docs/architecture.md` §0):
//! they belong to the official app, and reading them would be exactly the "we get to know how it
//! works" mistake the project exists to avoid. So the GUI keeps its own file, and everything the
//! user toggles in the official app is read through `protonvpn config list` like any other state.
//!
//! One deliberate omission: **the qBittorrent password is never written to disk.** It is a
//! credential for a third-party service; `docs/architecture.md` §10.4 leaves the question open,
//! and until it is answered the safe reading is "keep it in memory for the session". Everything
//! else (host, port, username, enable flag) is ordinary configuration.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Location of our config, honouring `XDG_CONFIG_HOME`.
pub fn default_config_path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("protonvpn-gui").join("config.json")
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Connect to `country` as soon as the application starts (tray-only start included).
    pub connect_at_startup: bool,
    /// Country code for `connect_at_startup`. `None` means "fastest".
    pub startup_country: Option<String>,
    /// Start with the window hidden, so autostart goes straight to the tray.
    pub start_minimized: bool,
    /// Keep `~/.config/autostart/protonvpn-gui.desktop` in sync with `autostart`.
    pub autostart: bool,
    /// Last country chosen in the picker, so the window opens where the user left it.
    pub last_country: Option<String>,
    pub probe_enabled: bool,
    /// Whether we maintain a NAT-PMP lease at all. On by default: it is the one feature the
    /// official CLI cannot do, and it is not attempted on servers that do not support it.
    pub port_forwarding_enabled: bool,
    pub qbittorrent: QBittorrent,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            connect_at_startup: false,
            startup_country: None,
            start_minimized: false,
            autostart: false,
            last_country: None,
            probe_enabled: true,
            port_forwarding_enabled: true,
            qbittorrent: QBittorrent::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct QBittorrent {
    /// Off by default: enabling it changes another application's configuration
    /// (`docs/architecture.md` §10.4).
    pub enabled: bool,
    pub host: String,
    pub port: u16,
    pub username: String,
}

impl Default for QBittorrent {
    fn default() -> Self {
        Self {
            enabled: false,
            host: "localhost".to_string(),
            port: 8080,
            username: String::new(),
        }
    }
}

/// Loads and saves [`Config`], remembering whether the file was there at all.
#[derive(Debug, Clone)]
pub struct ConfigStore {
    path: PathBuf,
}

impl Default for ConfigStore {
    fn default() -> Self {
        Self::at(default_config_path())
    }
}

impl ConfigStore {
    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Reads the file. A missing file is not an error: it is a first run, and defaults are
    /// correct. A *corrupt* file is reported, because silently discarding a user's settings is
    /// worse than telling them.
    pub fn load(&self) -> Result<Config, ConfigError> {
        match fs::read_to_string(&self.path) {
            Ok(text) => serde_json::from_str(&text).map_err(|e| ConfigError::Parse(e.to_string())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
            Err(error) => Err(ConfigError::Io(error.to_string())),
        }
    }

    pub fn save(&self, config: &Config) -> Result<(), ConfigError> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|e| ConfigError::Io(e.to_string()))?;
        }
        let text =
            serde_json::to_string_pretty(config).map_err(|e| ConfigError::Parse(e.to_string()))?;
        fs::write(&self.path, text).map_err(|e| ConfigError::Io(e.to_string()))
    }
}

#[derive(Debug)]
pub enum ConfigError {
    Io(String),
    Parse(String),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "cannot read or write the config: {e}"),
            Self::Parse(e) => write!(f, "cannot parse the config: {e}"),
        }
    }
}

impl std::error::Error for ConfigError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store(name: &str) -> ConfigStore {
        let dir =
            std::env::temp_dir().join(format!("protonvpn-gui-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        ConfigStore::at(dir.join("config.json"))
    }

    #[test]
    fn a_missing_file_is_a_first_run_not_an_error() {
        let store = temp_store("missing");
        assert_eq!(store.load().unwrap(), Config::default());
    }

    #[test]
    fn round_trips_through_disk() {
        let store = temp_store("roundtrip");
        let config = Config {
            connect_at_startup: true,
            startup_country: Some("CH".into()),
            start_minimized: true,
            autostart: true,
            last_country: Some("NL".into()),
            probe_enabled: false,
            port_forwarding_enabled: true,
            qbittorrent: QBittorrent {
                enabled: true,
                host: "localhost".into(),
                port: 8080,
                username: "admin".into(),
            },
        };
        store.save(&config).unwrap();
        assert_eq!(store.load().unwrap(), config);
    }

    #[test]
    fn a_corrupt_file_is_reported_rather_than_silently_replaced() {
        let store = temp_store("corrupt");
        fs::create_dir_all(store.path().parent().unwrap()).unwrap();
        fs::write(store.path(), "{not json").unwrap();
        assert!(matches!(store.load(), Err(ConfigError::Parse(_))));
    }

    #[test]
    fn unknown_keys_are_rejected_rather_than_ignored() {
        // A typo in a hand-edited config must be visible, not silently dropped.
        let store = temp_store("unknown");
        fs::create_dir_all(store.path().parent().unwrap()).unwrap();
        fs::write(store.path(), r#"{"start_minimised": true}"#).unwrap();
        assert!(matches!(store.load(), Err(ConfigError::Parse(_))));
    }

    #[test]
    fn partial_files_fall_back_to_defaults_for_missing_keys() {
        let store = temp_store("partial");
        fs::create_dir_all(store.path().parent().unwrap()).unwrap();
        fs::write(store.path(), r#"{"start_minimized": true}"#).unwrap();
        let config = store.load().unwrap();
        assert!(config.start_minimized);
        assert!(!config.qbittorrent.enabled);
        assert_eq!(config.qbittorrent.port, 8080);
    }

    #[test]
    fn the_qbittorrent_password_is_not_part_of_the_file_format() {
        let store = temp_store("nosecret");
        let config = Config {
            qbittorrent: QBittorrent {
                enabled: true,
                username: "admin".into(),
                ..Default::default()
            },
            ..Default::default()
        };
        store.save(&config).unwrap();
        let text = fs::read_to_string(store.path()).unwrap();
        assert!(!text.contains("password"), "{text}");
    }

    #[test]
    fn the_config_path_is_under_the_xdg_config_home() {
        let path = default_config_path();
        assert!(
            path.ends_with("protonvpn-gui/config.json"),
            "{}",
            path.display()
        );
    }
}
