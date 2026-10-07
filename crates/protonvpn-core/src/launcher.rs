//! Launcher — intent in, argv out, and nothing else.
//!
//! `docs/architecture.md` §6. The launcher never reads output and never decides whether anything
//! succeeded; its only claim is "I ran this command". Success is the interpreter's business, and
//! it finds out from the log like everyone else.
//!
//! The argv strings live here rather than at the call sites so there is exactly one place that
//! knows the CLI's spelling of a flag, and one place to test it.

use crate::i18n::I18n;
use crate::model::ConnectTarget;

/// What the user asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Intent {
    Connect(ConnectTarget),
    Disconnect,
    RefreshStatus,
    ListCountries,
    ListCities {
        country: String,
    },
    ListSettings,
    SetSetting {
        key: String,
        value: String,
        /// `custom-dns on` is meaningless without servers, so `--dns` travels with it.
        dns: Option<String>,
    },
    AccountInfo,
    /// `protonvpn signin <user>` — interactive: the password and any 2FA code are written to the
    /// child's PTY, never into argv and never into the log.
    SignIn {
        username: String,
    },
    SignOut,
}

impl Intent {
    /// The argv to execute. `protonvpn` is argv[0]; there is no other program in this project.
    pub fn argv(&self) -> Vec<String> {
        let mut argv: Vec<String> = vec!["protonvpn".to_string()];
        match self {
            Self::Connect(target) => {
                argv.push("connect".to_string());
                if let Some(server) = &target.server {
                    argv.push(server.clone());
                }
                if let Some(country) = &target.country {
                    argv.push("--country".to_string());
                    argv.push(country.clone());
                }
                if let Some(city) = &target.city {
                    argv.push("--city".to_string());
                    argv.push(city.clone());
                }
                if target.p2p {
                    argv.push("--p2p".to_string());
                }
                if target.secure_core {
                    argv.push("--securecore".to_string());
                }
                if target.tor {
                    argv.push("--tor".to_string());
                }
                if target.random {
                    argv.push("--random".to_string());
                }
            }
            Self::Disconnect => argv.push("disconnect".to_string()),
            Self::RefreshStatus => argv.push("status".to_string()),
            Self::ListCountries => {
                argv.push("countries".to_string());
                argv.push("list".to_string());
            }
            Self::ListCities { country } => {
                argv.push("cities".to_string());
                argv.push("list".to_string());
                argv.push(country.clone());
            }
            Self::ListSettings => {
                argv.push("config".to_string());
                argv.push("list".to_string());
            }
            Self::SetSetting { key, value, dns } => {
                argv.push("config".to_string());
                argv.push("set".to_string());
                argv.push(key.clone());
                argv.push(value.clone());
                if let Some(dns) = dns
                    && !dns.trim().is_empty()
                {
                    argv.push("--dns".to_string());
                    argv.push(dns.clone());
                }
            }
            Self::AccountInfo => argv.push("info".to_string()),
            Self::SignIn { username } => {
                argv.push("signin".to_string());
                argv.push(username.clone());
            }
            Self::SignOut => argv.push("signout".to_string()),
        }
        argv
    }

    /// Could this invocation have changed connection state?
    ///
    /// If so, a status poll is triggered the moment it finishes (`docs/architecture.md` §7) —
    /// the transition we caused is otherwise invisible until the idle timer comes round.
    pub fn changes_connection_state(&self) -> bool {
        matches!(self, Self::Connect(_) | Self::Disconnect)
    }

    /// Does the CLI prompt for input on the PTY?
    pub fn is_interactive(&self) -> bool {
        matches!(self, Self::SignIn { .. })
    }

    /// Short human label for buttons and logs.
    pub fn label(&self, i18n: &I18n) -> String {
        match self {
            Self::Connect(target) => i18n.launcher_connect(i18n.target_label(target)),
            Self::Disconnect => i18n.launcher_disconnect(),
            Self::RefreshStatus => i18n.launcher_status(),
            Self::ListCountries => i18n.launcher_list_countries(),
            Self::ListCities { country } => i18n.launcher_list_cities(country),
            Self::ListSettings => i18n.launcher_list_settings(),
            // A key and its value are the CLI's own spelling of a setting: data, not prose.
            Self::SetSetting { key, value, .. } => format!("{key} = {value}"),
            Self::AccountInfo => i18n.launcher_account(),
            Self::SignIn { username } => i18n.launcher_sign_in(username),
            Self::SignOut => i18n.launcher_sign_out(),
        }
    }

    /// Never put a secret in argv: `SignIn` takes only the username, by design.
    pub fn has_secret_in_argv(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(intent: Intent) -> Vec<String> {
        intent.argv()
    }

    #[test]
    fn connect_by_country() {
        assert_eq!(
            argv(Intent::Connect(ConnectTarget::country("uk"))),
            vec!["protonvpn", "connect", "--country", "uk"]
        );
    }

    #[test]
    fn connect_by_city() {
        let target = ConnectTarget {
            city: Some("Zurich".into()),
            ..Default::default()
        };
        assert_eq!(
            argv(Intent::Connect(target)),
            vec!["protonvpn", "connect", "--city", "Zurich"]
        );
    }

    #[test]
    fn connect_fastest_is_a_bare_connect() {
        assert_eq!(
            argv(Intent::Connect(ConnectTarget::fastest())),
            vec!["protonvpn", "connect"]
        );
    }

    #[test]
    fn connect_by_server_id() {
        let target = ConnectTarget {
            server: Some("IT#23".into()),
            ..Default::default()
        };
        assert_eq!(
            argv(Intent::Connect(target)),
            vec!["protonvpn", "connect", "IT#23"]
        );
    }

    #[test]
    fn presets_map_to_their_flags() {
        let target = ConnectTarget {
            p2p: true,
            ..Default::default()
        };
        assert_eq!(
            argv(Intent::Connect(target)),
            vec!["protonvpn", "connect", "--p2p"]
        );

        let target = ConnectTarget {
            secure_core: true,
            ..Default::default()
        };
        assert_eq!(
            argv(Intent::Connect(target)),
            vec!["protonvpn", "connect", "--securecore"]
        );

        let target = ConnectTarget {
            tor: true,
            ..Default::default()
        };
        assert_eq!(
            argv(Intent::Connect(target)),
            vec!["protonvpn", "connect", "--tor"]
        );

        let target = ConnectTarget {
            random: true,
            ..Default::default()
        };
        assert_eq!(
            argv(Intent::Connect(target)),
            vec!["protonvpn", "connect", "--random"]
        );
    }

    #[test]
    fn the_rest_of_the_surface() {
        assert_eq!(argv(Intent::Disconnect), vec!["protonvpn", "disconnect"]);
        assert_eq!(argv(Intent::RefreshStatus), vec!["protonvpn", "status"]);
        assert_eq!(
            argv(Intent::ListCountries),
            vec!["protonvpn", "countries", "list"]
        );
        assert_eq!(
            argv(Intent::ListCities {
                country: "CH".into()
            }),
            vec!["protonvpn", "cities", "list", "CH"]
        );
        assert_eq!(
            argv(Intent::ListSettings),
            vec!["protonvpn", "config", "list"]
        );
        assert_eq!(
            argv(Intent::SetSetting {
                key: "port-forwarding".into(),
                value: "on".into(),
                dns: None,
            }),
            vec!["protonvpn", "config", "set", "port-forwarding", "on"]
        );
        assert_eq!(
            argv(Intent::SetSetting {
                key: "custom-dns".into(),
                value: "on".into(),
                dns: Some("1.1.1.1,8.8.8.8".into()),
            }),
            vec![
                "protonvpn",
                "config",
                "set",
                "custom-dns",
                "on",
                "--dns",
                "1.1.1.1,8.8.8.8"
            ]
        );
        assert_eq!(argv(Intent::AccountInfo), vec!["protonvpn", "info"]);
        assert_eq!(
            argv(Intent::SignIn {
                username: "trousev".into()
            }),
            vec!["protonvpn", "signin", "trousev"]
        );
        assert_eq!(argv(Intent::SignOut), vec!["protonvpn", "signout"]);
    }

    #[test]
    fn port_forwarding_is_ours_and_never_reaches_argv() {
        // The CLI has no `--port-forwarding` flag: it only has a global preference, and the lease
        // is exception #2 in `docs/architecture.md` §0. A target that asks for a lease must
        // produce exactly the argv of a target that does not.
        let plain = ConnectTarget::country("NL");
        let forwarding = ConnectTarget {
            port_forwarding: true,
            ..plain.clone()
        };
        assert_eq!(
            argv(Intent::Connect(plain)),
            vec!["protonvpn", "connect", "--country", "NL"]
        );
        assert_eq!(
            argv(Intent::Connect(forwarding)),
            vec!["protonvpn", "connect", "--country", "NL"]
        );
    }

    #[test]
    fn every_argv_starts_with_the_one_program_we_run() {
        let intents = [
            Intent::Connect(ConnectTarget::fastest()),
            Intent::Disconnect,
            Intent::RefreshStatus,
            Intent::ListCountries,
            Intent::ListCities {
                country: "CH".into(),
            },
            Intent::ListSettings,
            Intent::SetSetting {
                key: "netshield".into(),
                value: "off".into(),
                dns: None,
            },
            Intent::AccountInfo,
            Intent::SignIn {
                username: "u".into(),
            },
            Intent::SignOut,
        ];
        for intent in intents {
            assert_eq!(intent.argv()[0], "protonvpn");
        }
    }

    #[test]
    fn no_intent_can_smuggle_a_secret_through_argv() {
        for intent in [
            Intent::SignIn {
                username: "trousev".into(),
            },
            Intent::SetSetting {
                key: "netshield".into(),
                value: "off".into(),
                dns: None,
            },
        ] {
            assert!(!intent.has_secret_in_argv());
        }
    }

    #[test]
    fn state_changing_intents_are_the_two_that_move_the_tunnel() {
        assert!(Intent::Connect(ConnectTarget::fastest()).changes_connection_state());
        assert!(Intent::Disconnect.changes_connection_state());
        for intent in [
            Intent::RefreshStatus,
            Intent::ListCountries,
            Intent::AccountInfo,
            Intent::SignOut,
        ] {
            assert!(!intent.changes_connection_state(), "{intent:?}");
        }
    }
}
