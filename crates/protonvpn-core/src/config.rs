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

use crate::model::ConnectTarget;

/// The selected connection when the user has never chosen one. The three system presets are not
/// stored: they are the CLI's own shortcuts, spelled once in the GUI, and they cannot be edited.
pub const SYSTEM_FASTEST: &str = "system:fastest";
pub const SYSTEM_SECURE_CORE: &str = "system:secure-core";
pub const SYSTEM_P2P: &str = "system:p2p";

/// Every system preset id, in the order the connection list shows them.
pub const SYSTEM_PRESETS: [&str; 3] = [SYSTEM_FASTEST, SYSTEM_SECURE_CORE, SYSTEM_P2P];

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
    /// Connect to the selected connection as soon as the application starts (tray-only start
    /// included).
    pub connect_at_startup: bool,
    /// Start with the window hidden, so autostart goes straight to the tray.
    pub start_minimized: bool,
    /// Keep `~/.config/autostart/protonvpn-gui.desktop` in sync with `autostart`.
    pub autostart: bool,
    pub probe_enabled: bool,
    /// Where the app writes user-created connections. Everything about a connection — the
    /// country, the city, P2P / Secure Core / Tor and whether we maintain a port-forwarding
    /// lease — belongs to the profile (`docs/architecture.md` §11). There are deliberately no
    /// global "default preset" settings left: the CLI has connect flags, not defaults.
    pub connections: Vec<SavedConnection>,
    /// Which connection the "Подключиться" button and the tray menu use. One namespace:
    /// [`SYSTEM_FASTEST`] and friends, or the `id` of a [`SavedConnection`].
    pub selected_connection: Option<String>,
    pub qbittorrent: QBittorrent,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            connect_at_startup: false,
            start_minimized: false,
            autostart: false,
            probe_enabled: true,
            connections: Vec::new(),
            selected_connection: Some(SYSTEM_FASTEST.to_string()),
            qbittorrent: QBittorrent::default(),
        }
    }
}

/// One connection the user built: the answer to "what should `protonvpn connect` be?".
///
/// The checkbox set is exactly the CLI's connect surface (`docs/cli-surface.md` §1) plus port
/// forwarding, which is ours and is documented as such. Nothing here is a global default — the
/// whole point of the connection manager is that `--p2p` belongs to a profile, not to the app.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SavedConnection {
    /// Stable, opaque, unique. Survives a rename, which is what the selection points at.
    pub id: String,
    pub name: String,
    /// Country code (`US`), as `countries list` prints it. `None` means "any country".
    pub country: Option<String>,
    /// City name as `cities list <CC>` prints it. Only meaningful together with `country`.
    pub city: Option<String>,
    pub p2p: bool,
    pub secure_core: bool,
    pub tor: bool,
    /// Whether we keep a NAT-PMP lease while this connection is up. Not a CLI flag: see
    /// [`ConnectTarget::port_forwarding`].
    pub port_forwarding: bool,
}

impl SavedConnection {
    /// The intent this profile stands for.
    pub fn target(&self) -> ConnectTarget {
        ConnectTarget {
            country: self.country.clone(),
            city: self.city.clone(),
            server: None,
            p2p: self.p2p,
            secure_core: self.secure_core,
            tor: self.tor,
            random: false,
            port_forwarding: self.port_forwarding,
        }
    }

    /// The one-line summary under the name in the list.
    pub fn summary(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        match (&self.country, &self.city) {
            (Some(country), Some(city)) => parts.push(format!("{country} · {city}")),
            (Some(country), None) => parts.push(format!("{country} · любой город")),
            (None, _) => parts.push("любая страна".to_string()),
        }
        parts.join(" · ")
    }

    /// The badges the list shows next to the name.
    pub fn badges(&self) -> Vec<String> {
        let mut badges = Vec::new();
        if self.secure_core {
            badges.push("SECURE CORE".to_string());
        }
        if self.tor {
            badges.push("TOR".to_string());
        }
        if self.p2p {
            badges.push("P2P".to_string());
        }
        if self.port_forwarding {
            badges.push("ПОРТ".to_string());
        }
        badges
    }
}

impl Config {
    pub fn connection(&self, id: &str) -> Option<&SavedConnection> {
        self.connections.iter().find(|saved| saved.id == id)
    }

    pub fn connection_mut(&mut self, id: &str) -> Option<&mut SavedConnection> {
        self.connections.iter_mut().find(|saved| saved.id == id)
    }

    pub fn selected(&self) -> Option<&SavedConnection> {
        self.selected_connection
            .as_deref()
            .and_then(|id| self.connection(id))
    }

    /// Inserts a new profile, assigning it an id that cannot collide with the system presets.
    /// Returns the id.
    pub fn add_connection(&mut self, mut saved: SavedConnection) -> String {
        let id = self.fresh_connection_id(&saved.name);
        saved.id = id.clone();
        self.connections.push(saved);
        id
    }

    fn fresh_connection_id(&self, name: &str) -> String {
        let mut slug: String = name
            .trim()
            .to_lowercase()
            .chars()
            .map(|c| if c.is_alphanumeric() { c } else { '-' })
            .collect();
        while slug.contains("--") {
            slug = slug.replace("--", "-");
        }
        let slug = slug.trim_matches('-').to_string();
        let slug = if slug.is_empty() {
            "connection".to_string()
        } else if slug.starts_with("system") {
            // The system preset ids live in the same namespace; never shadow one.
            format!("user-{slug}")
        } else {
            slug
        };
        if !self.connections.iter().any(|saved| saved.id == slug) {
            return slug;
        }
        for n in 2.. {
            let candidate = format!("{slug}-{n}");
            if !self.connections.iter().any(|saved| saved.id == candidate) {
                return candidate;
            }
        }
        unreachable!("the loop above always terminates")
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
            start_minimized: true,
            autostart: true,
            probe_enabled: false,
            connections: vec![SavedConnection {
                id: "work".into(),
                name: "Работа".into(),
                country: Some("NL".into()),
                city: None,
                p2p: true,
                secure_core: false,
                tor: false,
                port_forwarding: true,
            }],
            selected_connection: Some("work".into()),
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
    fn a_profile_becomes_exactly_the_cli_flags_it_stands_for() {
        let saved = SavedConnection {
            id: "work".into(),
            name: "Работа".into(),
            country: Some("NL".into()),
            city: None,
            p2p: true,
            secure_core: false,
            tor: false,
            port_forwarding: true,
        };
        let target = saved.target();
        assert_eq!(target.country.as_deref(), Some("NL"));
        assert!(target.p2p);
        // Ours, not the CLI's: it never reaches argv, it decides whether we hold a lease.
        assert!(target.port_forwarding);
        assert_eq!(saved.badges(), vec!["P2P", "ПОРТ"]);
    }

    #[test]
    fn connection_ids_cannot_shadow_the_system_presets_or_each_other() {
        let mut config = Config::default();
        let mut saved = SavedConnection {
            name: "System Tor".into(),
            ..Default::default()
        };
        let first = config.add_connection(saved.clone());
        saved.name = "System  Tor".into();
        let second = config.add_connection(saved);
        assert!(first.starts_with("user-"), "{first}");
        assert_ne!(first, second);
        assert_eq!(config.connections.len(), 2);
        // Renaming later does not move the id the selection points at.
        assert!(config.connection(&first).is_some());
    }

    #[test]
    fn a_profile_without_a_country_is_any_country() {
        let saved = SavedConnection {
            name: "Где угодно".into(),
            ..Default::default()
        };
        assert_eq!(saved.summary(), "любая страна");
        assert!(saved.badges().is_empty());
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
        assert!(config.connections.is_empty());
        assert_eq!(config.selected_connection.as_deref(), Some(SYSTEM_FASTEST));
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
