//! Interpreter — a pure reducer over the log bus.
//!
//! `docs/architecture.md` §5. `(AppState, LogEvent) -> AppState`, no I/O, no clock except the
//! timestamps carried by the events themselves, no access to the runner.
//!
//! Two rules are load-bearing:
//!
//! 1. **Never guess.** An unrecognised line produces no state. Unknown output stays visible as
//!    raw text in the console and the affected state keeps its previous value and its age.
//! 2. **Never invent a status.** Silence is `Unknown`, not `Disconnected`.
//!
//! The whole fixture corpus is the test suite for this module.

use std::time::SystemTime;

use crate::logbus::{Invocation, InvocationKind, LogEvent};
use crate::model::{
    Account, AppState, ConnectedInfo, ConnectionStatus, InvocationId, Observation, PortForwarding,
};
use crate::parse;

/// Fold one event into the state.
///
/// `record` is the invocation the event belongs to, as the log bus holds it — that is where the
/// argv and the accumulated output come from, so the reducer itself stays free of I/O.
pub fn interpret(state: AppState, event: &LogEvent, record: Option<&Invocation>) -> AppState {
    let mut state = state;

    match event {
        LogEvent::Started { argv, at, .. } => {
            on_started(&mut state, argv, *at);
        }
        LogEvent::Line { id, line } => {
            if let Some(record) = record {
                on_line(&mut state, &record.argv, *id, &line.text, line.at);
            }
        }
        LogEvent::Finished {
            id, exit_code, at, ..
        } => {
            if let Some(record) = record {
                on_finished(&mut state, record, *id, *exit_code, *at);
            }
        }
    }

    state
}

/// Optimistic transitions we cause ourselves. The CLI has not spoken yet, and the UI should not
/// pretend it has: this is exactly what `Connecting` means.
fn on_started(state: &mut AppState, argv: &[String], at: SystemTime) {
    // `disconnect` is idempotent and cheap, but the tunnel is only gone once the CLI says so:
    // nothing to claim when one starts.
    if let Some("connect") = subcommand(argv) {
        state.connection = Observation::at(ConnectionStatus::Connecting, at);
        state.port_forwarding = Observation::at(PortForwarding::Pending, at);
        state.last_error = None;
    }
}

/// Mid-command signals. Deliberately narrow: only lines the CLI says about *itself*.
fn on_line(state: &mut AppState, argv: &[String], id: InvocationId, text: &str, at: SystemTime) {
    if let Some(error) = parse::error(text, id) {
        // A coexistence failure is fatal to every subsequent command, and must be explained
        // rather than merely logged (`docs/cli-surface.md` §2).
        state.connection = Observation::at(ConnectionStatus::Error(error.message.clone()), at);
        state.last_error = Some(Observation::at(error, at));
        return;
    }

    if subcommand(argv) == Some("connect")
        && let Some(outcome) = parse::connect(text)
    {
        // The tunnel is up as far as the CLI is concerned; load and protocol arrive with the
        // status poll that this invocation triggers.
        state.connection = Observation::at(
            ConnectionStatus::Connected(ConnectedInfo {
                server: outcome.server,
                location: outcome.location,
                load_percent: None,
                protocol: None,
            }),
            at,
        );
    }
}

fn on_finished(
    state: &mut AppState,
    record: &Invocation,
    id: InvocationId,
    exit_code: Option<u32>,
    at: SystemTime,
) {
    if record.kind == InvocationKind::Note {
        return;
    }

    let text = parse::normalise(&record.output());
    let argv = record.argv.clone();
    state.last_run = Some(Observation::at(record.command_line(), at));

    if let Some(error) = parse::error(&text, id) {
        state.connection = Observation::at(ConnectionStatus::Error(error.message.clone()), at);
        state.last_error = Some(Observation::at(error, at));
        return;
    }

    match subcommand(&argv) {
        Some("status") => {
            if let Some(status) = parse::status(&text) {
                state.connection = Observation::at(status, at);
            }
            // Unparseable status output changes nothing: the previous value keeps its own age,
            // and the raw text is in the console for the user to read.
        }
        Some("countries") => {
            if let Some(countries) = parse::countries(&text) {
                state.countries = Some(Observation::at(countries, at));
            }
        }
        Some("cities") => {
            if let Some((country, cities)) = parse::cities(&text)
                && let Some(requested) = argv.get(3)
            {
                state.cities = Some(Observation::at(cities, at));
                state.cities_country = Some(if country.is_empty() {
                    requested.clone()
                } else {
                    country
                });
            }
        }
        Some("config") => match argv.get(2).map(String::as_str) {
            Some("list") => {
                if let Some(settings) = parse::settings(&text) {
                    state.settings = Some(Observation::at(settings, at));
                }
            }
            Some("set") => {
                // The value we asked for is not evidence that it took effect; `config list` is.
                // The engine re-lists after a successful set, and that list lands here.
            }
            _ => {}
        },
        Some("info") => {
            if let Some(account) = parse::account(&text) {
                state.account = Some(Observation::at(account, at));
            } else if parse::not_signed_in(&text) {
                state.account = Some(Observation::at(Account { name: None }, at));
            }
        }
        Some("connect") => {
            if exit_code.is_some_and(|code| code != 0) {
                // The error line, if the CLI printed one, was already handled above; a silent
                // failure must not leave the UI claiming a connection.
                if !matches!(state.connection.value, ConnectionStatus::Error(_)) {
                    state.connection = Observation::at(
                        ConnectionStatus::Error(format!(
                            "`{}` завершилась с кодом {}",
                            record.command_line(),
                            exit_code.unwrap_or_default()
                        )),
                        at,
                    );
                }
                state.port_forwarding = Observation::at(PortForwarding::Idle, at);
                return;
            }
            if let Some(outcome) = parse::connect(&text) {
                state.connection = Observation::at(
                    ConnectionStatus::Connected(ConnectedInfo {
                        server: outcome.server,
                        location: outcome.location,
                        load_percent: None,
                        protocol: None,
                    }),
                    at,
                );
                state.port_forwarding = Observation::at(
                    match outcome.port_forwarding {
                        parse::PortForwardingClaim::Active => PortForwarding::Pending,
                        parse::PortForwardingClaim::Unsupported => PortForwarding::Unsupported,
                        parse::PortForwardingClaim::Silent => PortForwarding::Idle,
                    },
                    at,
                );
            }
        }
        Some("disconnect") => {
            if exit_code == Some(0) {
                state.connection = Observation::at(ConnectionStatus::Disconnected, at);
                state.port_forwarding = Observation::at(PortForwarding::Idle, at);
            }
        }
        Some("signout") if exit_code == Some(0) => {
            state.account = Some(Observation::at(Account { name: None }, at));
        }
        _ => {}
    }
}

/// The CLI subcommand of an argv, `protonvpn` skipped. `None` for anything that is not ours.
/// The executable is matched by file name, so an absolute path — or the stand-in the engine
/// tests drive — is recognised, while a note invocation (which has no argv at all) is not.
pub fn subcommand(argv: &[String]) -> Option<&str> {
    let program = argv.first()?;
    let name = std::path::Path::new(program)
        .file_name()
        .and_then(|name| name.to_str())?;
    if name != "protonvpn" {
        return None;
    }
    argv.get(1).map(String::as_str)
}

/// Applies one event to a state in place.
///
/// The pure [`interpret`] remains the definition; this exists so the engine can fold a line into
/// the state without cloning it (the country list is a few hundred entries).
pub fn apply(state: &mut AppState, event: &LogEvent, record: Option<&Invocation>) {
    let owned = std::mem::take(state);
    *state = interpret(owned, event, record);
}

/// What the CLI is asking for on the PTY.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptKind {
    Password,
    TwoFactor,
    /// An unrecognised prompt. The UI still offers a masked field, because a human can read the
    /// console and answer it even when we cannot classify it.
    Unrecognised,
}

/// Classifies an interactive prompt. Only ever consulted while an interactive child is running,
/// which is what keeps it from firing on unrelated output.
pub fn classify_prompt(line: &str) -> Option<PromptKind> {
    let lower = line.to_ascii_lowercase();
    if lower.contains("two-factor")
        || lower.contains("two factor")
        || lower.contains("2fa")
        || lower.contains("authentication code")
        || lower.contains("verification code")
    {
        return Some(PromptKind::TwoFactor);
    }
    if lower.contains("password") {
        return Some(PromptKind::Password);
    }
    // A bare prompt ending in `: ` is the CLI's usual shape for `click.prompt`.
    if line.trim_end().ends_with(':') && !line.trim().is_empty() {
        return Some(PromptKind::Unrecognised);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logbus::{InvocationKind, LogBus};
    use crate::model::{City, CliError, Country, Setting};
    use std::path::PathBuf;

    fn fixture(name: &str) -> String {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/pty")
            .join(format!("{name}.txt"));
        std::fs::read_to_string(&path).unwrap()
    }

    /// Runs a whole invocation through the bus and the reducer, exactly as the engine does.
    fn run(state: AppState, argv: &[&str], output: &str, exit_code: Option<u32>) -> AppState {
        let mut bus = LogBus::default();
        let argv: Vec<String> = argv.iter().map(|s| s.to_string()).collect();
        let at = SystemTime::now();
        let id = bus.begin(
            InvocationKind::ProtonVpn,
            argv.clone(),
            None,
            PathBuf::from("/tmp"),
            at,
        );
        let started = LogEvent::Started {
            id,
            kind: InvocationKind::ProtonVpn,
            argv: argv.clone(),
            display: None,
            cwd: PathBuf::from("/tmp"),
            at,
        };
        let mut state = interpret(state, &started, bus.get(id));
        for line in output.lines() {
            let text = line.trim_end_matches('\r').to_string();
            let event = LogEvent::Line {
                id,
                line: crate::logbus::LogLine {
                    at: SystemTime::now(),
                    text: text.clone(),
                },
            };
            bus.push_line(id, text, SystemTime::now());
            state = interpret(state, &event, bus.get(id));
        }
        let finished = LogEvent::Finished {
            id,
            exit_code,
            duration: std::time::Duration::from_secs(1),
            at: SystemTime::now(),
        };
        bus.finish(
            id,
            exit_code,
            std::time::Duration::from_secs(1),
            SystemTime::now(),
        );
        interpret(state, &finished, bus.get(id))
    }

    fn connected() -> State {
        State::new(AppState::default())
    }

    /// Tiny helper so the tests read as a sequence of invocations.
    struct State {
        state: AppState,
    }

    impl State {
        fn new(state: AppState) -> Self {
            Self { state }
        }

        fn run(mut self, argv: &[&str], output: &str, exit: Option<u32>) -> Self {
            self.state = run(self.state, argv, output, exit);
            self
        }
    }

    #[test]
    fn status_fixture_produces_connected_state() {
        let state = connected().run(
            &["protonvpn", "status"],
            &fixture("status_connected"),
            Some(0),
        );
        match &state.state.connection.value {
            ConnectionStatus::Connected(info) => {
                assert_eq!(info.server, "NL#818");
                assert_eq!(info.location, "Amsterdam, Netherlands");
                assert_eq!(info.load_percent, Some(59));
                assert_eq!(info.protocol.as_deref(), Some("wireguard"));
            }
            other => panic!("expected Connected, got {other:?}"),
        }
    }

    #[test]
    fn disconnected_fixture_produces_disconnected_state() {
        let state = connected().run(
            &["protonvpn", "status"],
            &fixture("status_disconnected"),
            Some(0),
        );
        assert_eq!(state.state.connection.value, ConnectionStatus::Disconnected);
    }

    #[test]
    fn unknown_output_leaves_the_previous_state_and_its_age_intact() {
        let before = connected().run(
            &["protonvpn", "status"],
            &fixture("status_connected"),
            Some(0),
        );
        let at_before = before.state.connection.at;
        let after = before.run(
            &["protonvpn", "status"],
            "Whatever: something new\n",
            Some(0),
        );
        assert!(after.state.connection.value.is_connected());
        assert_eq!(after.state.connection.at, at_before);
    }

    #[test]
    fn countries_fixture_produces_the_country_list() {
        let state = connected().run(
            &["protonvpn", "countries", "list"],
            &fixture("countries_list"),
            Some(0),
        );
        let countries: &Vec<Country> = &state.state.countries.as_ref().unwrap().value;
        assert!(countries.contains(&Country {
            name: "Switzerland".into(),
            code: "CH".into()
        }));
    }

    #[test]
    fn cities_fixture_knows_which_country_it_describes() {
        let state = connected().run(
            &["protonvpn", "cities", "list", "CH"],
            &fixture("cities_list_ch"),
            Some(0),
        );
        assert_eq!(state.state.cities_country.as_deref(), Some("Switzerland"));
        assert_eq!(
            state.state.cities.as_ref().unwrap().value,
            vec![City {
                name: "Zurich".into(),
                features: vec!["P2P".into(), "Tor".into()]
            }]
        );
    }

    #[test]
    fn settings_fixture_produces_the_settings_table() {
        let state = connected().run(
            &["protonvpn", "config", "list"],
            &fixture("config_list"),
            Some(0),
        );
        let settings: &Vec<Setting> = &state.state.settings.as_ref().unwrap().value;
        assert!(settings.contains(&Setting {
            key: "port-forwarding".into(),
            value: "on".into()
        }));
    }

    #[test]
    fn info_fixture_produces_the_account() {
        let state = connected().run(&["protonvpn", "info"], &fixture("info"), Some(0));
        assert_eq!(
            state.state.account.as_ref().unwrap().value.name.as_deref(),
            Some("trousev")
        );
    }

    #[test]
    fn connect_fixture_flips_to_connected_before_the_status_poll() {
        let state = connected().run(
            &["protonvpn", "connect", "--country", "NL"],
            &fixture("connect_nl"),
            Some(0),
        );
        match &state.state.connection.value {
            ConnectionStatus::Connected(info) => assert_eq!(info.server, "NL#818"),
            other => panic!("expected Connected, got {other:?}"),
        }
        // Load and protocol are unknown until `status` answers; they must not be invented.
        if let ConnectionStatus::Connected(info) = &state.state.connection.value {
            assert_eq!(info.load_percent, None);
            assert_eq!(info.protocol, None);
        }
        // The CLI said forwarding is active, so a lease is worth asking for.
        assert_eq!(state.state.port_forwarding.value, PortForwarding::Pending);
    }

    #[test]
    fn a_server_without_forwarding_is_recorded_as_such() {
        let text = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/pipe/connect_ch.txt"),
        )
        .unwrap();
        let state = connected().run(&["protonvpn", "connect", "--country", "CH"], &text, Some(0));
        assert_eq!(
            state.state.port_forwarding.value,
            PortForwarding::Unsupported
        );
    }

    #[test]
    fn a_validation_error_is_kept_verbatim_and_classified() {
        let state = connected().run(
            &["protonvpn", "connect", "--country", "ZZ"],
            &fixture("connect_invalid_country"),
            Some(2),
        );
        let error: &CliError = &state.state.last_error.as_ref().unwrap().value;
        assert!(error.message.contains("Invalid country code 'ZZ'"));
        assert_eq!(error.kind, crate::model::CliErrorKind::Validation);
        assert!(matches!(
            state.state.connection.value,
            ConnectionStatus::Error(_)
        ));
    }

    #[test]
    fn the_coexistence_error_is_recognised_as_its_own_kind() {
        let text = "Error: Proton VPN desktop app is currently running\nThe CLI and GUI cannot run \
                    simultaneously. Please close the GUI application and try again.";
        let state = connected().run(&["protonvpn", "status"], text, Some(1));
        assert_eq!(
            state.state.last_error.as_ref().unwrap().value.kind,
            crate::model::CliErrorKind::Coexistence
        );
    }

    #[test]
    fn disconnect_fixture_yields_disconnected() {
        let state = connected()
            .run(
                &["protonvpn", "status"],
                &fixture("status_connected"),
                Some(0),
            )
            .run(
                &["protonvpn", "disconnect"],
                &fixture("disconnect"),
                Some(0),
            );
        assert_eq!(state.state.connection.value, ConnectionStatus::Disconnected);
        assert_eq!(state.state.port_forwarding.value, PortForwarding::Idle);
    }

    #[test]
    fn a_failed_connect_does_not_leave_a_connection_claim_behind() {
        let state = connected()
            .run(
                &["protonvpn", "status"],
                &fixture("status_connected"),
                Some(0),
            )
            .run(
                &["protonvpn", "connect", "--country", "NL"],
                "some silence\n",
                Some(1),
            );
        assert!(matches!(
            state.state.connection.value,
            ConnectionStatus::Error(_)
        ));
    }

    #[test]
    fn starting_a_connect_is_optimistically_connecting() {
        let mut bus = LogBus::default();
        let at = SystemTime::now();
        let argv = vec!["protonvpn".to_string(), "connect".to_string()];
        let id = bus.begin(
            InvocationKind::ProtonVpn,
            argv.clone(),
            None,
            PathBuf::from("/tmp"),
            at,
        );
        let event = LogEvent::Started {
            id,
            kind: InvocationKind::ProtonVpn,
            argv,
            display: None,
            cwd: PathBuf::from("/tmp"),
            at,
        };
        let state = interpret(AppState::default(), &event, bus.get(id));
        assert_eq!(state.connection.value, ConnectionStatus::Connecting);
    }

    #[test]
    fn notes_never_touch_state() {
        let mut bus = LogBus::default();
        let at = SystemTime::now();
        let id = bus.begin(
            InvocationKind::Note,
            Vec::new(),
            Some("curl https://ifconfig.co/json".into()),
            PathBuf::from("/tmp"),
            at,
        );
        let event = LogEvent::Started {
            id,
            kind: InvocationKind::Note,
            argv: Vec::new(),
            display: Some("curl".into()),
            cwd: PathBuf::from("/tmp"),
            at,
        };
        let before = AppState::default();
        let after = interpret(before.clone(), &event, bus.get(id));
        assert_eq!(before, after);
    }

    #[test]
    fn signout_clears_the_account_on_success_only() {
        let base = connected().run(&["protonvpn", "info"], &fixture("info"), Some(0));
        let failed = run(
            base.state.clone(),
            &["protonvpn", "signout"],
            "Error: nope\n",
            Some(1),
        );
        assert!(failed.account.as_ref().unwrap().value.name.is_some());

        let ok = run(
            base.state,
            &["protonvpn", "signout"],
            "Signed out.\n",
            Some(0),
        );
        assert!(ok.account.unwrap().value.name.is_none());
    }

    #[test]
    fn classifies_the_prompts_we_expect_from_signin() {
        assert_eq!(classify_prompt("Password: "), Some(PromptKind::Password));
        assert_eq!(
            classify_prompt("Enter your Proton account password:"),
            Some(PromptKind::Password)
        );
        assert_eq!(
            classify_prompt("Two-factor authentication code: "),
            Some(PromptKind::TwoFactor)
        );
        assert_eq!(classify_prompt("2FA code:"), Some(PromptKind::TwoFactor));
        assert_eq!(classify_prompt(""), None);
        assert_eq!(
            classify_prompt("Username: "),
            Some(PromptKind::Unrecognised)
        );
    }

    #[test]
    fn subcommand_ignores_anything_that_is_not_protonvpn() {
        assert_eq!(
            subcommand(&["protonvpn".into(), "status".into()]),
            Some("status")
        );
        assert_eq!(subcommand(&["curl".into(), "https://x".into()]), None);
        assert_eq!(subcommand(&[]), None);
    }
}
