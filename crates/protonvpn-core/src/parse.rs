//! Parsers for the CLI's human-readable output.
//!
//! Written **against the frozen fixture corpus** in `tests/fixtures/pty/`, never against memory —
//! see `AGENTS.md`. The properties those fixtures pin down, all measured:
//!
//! * no ANSI escapes at all (the interpreter strips them anyway, defensively),
//! * `\r\n` line endings,
//! * tables are width-independent: two columns separated by runs of spaces, with a `-----` rule
//!   underneath the header,
//! * exactly one line of progress chatter to skip:
//!   `Server list is outdated, updating... This may take a moment.`
//!
//! Parsers here are **strict about shape and tolerant about noise**: an unrecognised line never
//! produces state. If the shape changes, the parser returns `None` and the caller keeps the raw
//! text instead of inventing a value.

use crate::model::{
    Account, City, CliError, CliErrorKind, ConnectedInfo, ConnectionStatus, Country, Setting,
};

/// Output with ANSI removed and CRLF normalised — the only form the parsers accept.
pub fn normalise(raw: &str) -> String {
    crate::pty::strip_ansi(raw)
}

/// Lines that are pure progress chatter, never state.
fn is_noise(line: &str) -> bool {
    let line = line.trim();
    line.is_empty() || line.starts_with("Server list is outdated")
}

fn meaningful_lines(text: &str) -> impl Iterator<Item = &str> {
    text.lines().map(str::trim_end).filter(|l| !is_noise(l))
}

/// Splits a table row into columns on runs of two or more spaces.
///
/// The CLI's tables use fixed-width padding, so a single space belongs to a cell ("Costa Rica")
/// and a run of spaces separates cells. This is what makes the parser width-independent.
fn split_columns(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut spaces = 0usize;
    for ch in line.trim_end().chars() {
        if ch == ' ' {
            spaces += 1;
            continue;
        }
        if !current.is_empty() {
            if spaces >= 2 {
                out.push(std::mem::take(&mut current));
            } else if spaces == 1 {
                current.push(' ');
            }
        }
        spaces = 0;
        current.push(ch);
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

fn is_rule(line: &str) -> bool {
    let trimmed = line.trim();
    !trimmed.is_empty() && trimmed.chars().all(|c| c == '-' || c == ' ')
}

/// A parsed two-column table: header names plus rows, gated on the `-----` rule being present.
struct Table {
    headers: Vec<String>,
    rows: Vec<Vec<String>>,
}

/// Finds the first two-column table in `text` and parses it strictly.
///
/// Strictness is the point: the header, the dash rule and the column count must all agree, or
/// this returns `None` and the caller shows the raw output rather than a half-parsed table.
fn parse_table(text: &str) -> Option<Table> {
    let lines: Vec<&str> = text.lines().map(str::trim_end).collect();
    for (i, line) in lines.iter().enumerate() {
        let headers = split_columns(line);
        if headers.len() != 2 {
            continue;
        }
        // Two consecutive all-dash cells on the next meaningful line is the CLI's table rule.
        let Some(rule) = lines.get(i + 1) else {
            continue;
        };
        if !is_rule(rule) {
            continue;
        }
        let expected = split_columns(rule).len();
        if expected != 2 {
            continue;
        }

        let mut rows = Vec::new();
        for row in &lines[i + 2..] {
            if is_noise(row) {
                // A blank line ends the table; the warm-up notice is skipped rather than fatal.
                if row.trim().is_empty() {
                    break;
                }
                continue;
            }
            let columns = split_columns(row);
            if columns.len() != 2 {
                break;
            }
            rows.push(columns);
        }
        if rows.is_empty() {
            continue;
        }
        return Some(Table { headers, rows });
    }
    None
}

fn header_is(table: &Table, first: &str, second: &str) -> bool {
    table.headers[0].eq_ignore_ascii_case(first) && table.headers[1].eq_ignore_ascii_case(second)
}

/// `protonvpn status` — the state source.
///
/// Connected output is four `Key: Value` lines; disconnected is one. Anything else is `None`,
/// which the interpreter turns into "keep what we had, with its age".
pub fn status(text: &str) -> Option<ConnectionStatus> {
    let mut status: Option<&str> = None;
    let mut info = ConnectedInfo::default();

    for line in meaningful_lines(text) {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        match key.trim().to_ascii_lowercase().as_str() {
            "status" => status = Some(value.trim()),
            "server" => {
                // `NL#818 in Amsterdam, Netherlands`
                match value.trim().split_once(" in ") {
                    Some((server, location)) => {
                        info.server = server.trim().to_string();
                        info.location = location.trim().to_string();
                    }
                    None => info.server = value.trim().to_string(),
                }
            }
            "load" => {
                info.load_percent = value.trim().trim_end_matches('%').trim().parse::<u8>().ok();
            }
            "protocol" => info.protocol = Some(value.trim().to_string()),
            _ => {}
        }
    }

    match status? {
        s if s.eq_ignore_ascii_case("connected") => Some(ConnectionStatus::Connected(info)),
        s if s.eq_ignore_ascii_case("disconnected") => Some(ConnectionStatus::Disconnected),
        other => Some(ConnectionStatus::Error(other.to_string())),
    }
}

/// `protonvpn countries list`.
pub fn countries(text: &str) -> Option<Vec<Country>> {
    let table = parse_table(text)?;
    if !header_is(&table, "Country", "Code") {
        return None;
    }
    let countries: Vec<Country> = table
        .rows
        .into_iter()
        .filter_map(|mut row| {
            let code = row.pop()?;
            let name = row.pop()?;
            if code.is_empty() || name.is_empty() {
                return None;
            }
            Some(Country { name, code })
        })
        .collect();
    (!countries.is_empty()).then_some(countries)
}

/// `protonvpn cities list <CC>` — returns the cities and the country as the CLI titled it.
pub fn cities(text: &str) -> Option<(String, Vec<City>)> {
    // `Cities in Switzerland:` — the only place the full country name appears.
    let country = text
        .lines()
        .find_map(|l| {
            l.trim()
                .strip_prefix("Cities in ")
                .map(|s| s.trim_end_matches(':').to_string())
        })
        .unwrap_or_default();

    let table = parse_table(text)?;
    if !header_is(&table, "City", "Features") {
        return None;
    }
    let cities: Vec<City> = table
        .rows
        .into_iter()
        .filter_map(|mut row| {
            let features = row.pop()?;
            let name = row.pop()?;
            if name.is_empty() {
                return None;
            }
            let features = features
                .split(',')
                .map(|f| f.trim().to_string())
                .filter(|f| !f.is_empty())
                .collect();
            Some(City { name, features })
        })
        .collect();
    (!cities.is_empty()).then_some((country, cities))
}

/// `protonvpn config list`.
pub fn settings(text: &str) -> Option<Vec<Setting>> {
    let table = parse_table(text)?;
    if !header_is(&table, "Setting", "Value") {
        return None;
    }
    let settings: Vec<Setting> = table
        .rows
        .into_iter()
        .filter_map(|mut row| {
            let value = row.pop()?;
            let key = row.pop()?;
            if key.is_empty() {
                return None;
            }
            Some(Setting { key, value })
        })
        .collect();
    (!settings.is_empty()).then_some(settings)
}

/// `protonvpn info`.
pub fn account(text: &str) -> Option<Account> {
    for line in meaningful_lines(text) {
        if let Some(rest) = line.strip_prefix("Account:") {
            let name = rest.trim().trim_matches('\'').trim();
            return Some(Account {
                name: (!name.is_empty()).then(|| name.to_string()),
            });
        }
    }
    None
}

/// What a `connect` invocation claimed. Informational only: the egress IP the CLI prints is
/// **not** the egress IP (measured: `149.88.27.213` printed, `149.22.89.89` actual), so nothing
/// here is ground truth.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectOutcome {
    pub server: String,
    pub location: String,
    pub claimed_ip: Option<String>,
    pub port_forwarding: PortForwardingClaim,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortForwardingClaim {
    /// The CLI said nothing about port forwarding.
    Silent,
    /// `Port forwarding is active on this server.`
    Active,
    /// `Note: Port forwarding is enabled but this server does not support it.`
    Unsupported,
}

pub fn connect(text: &str) -> Option<ConnectOutcome> {
    let mut server = None;
    let mut location = String::new();
    let mut claimed_ip = None;
    let mut port_forwarding = PortForwardingClaim::Silent;

    for line in text.lines().map(str::trim) {
        if let Some(rest) = line.strip_prefix("Connected to ") {
            // `NL#818 in Amsterdam, Netherlands. `
            let rest = rest.trim().trim_end_matches('.');
            match rest.split_once(" in ") {
                Some((s, l)) => {
                    server = Some(s.trim().to_string());
                    location = l.trim().to_string();
                }
                None => server = Some(rest.to_string()),
            }
        } else if let Some(rest) = line.strip_prefix("Your new IP address is ") {
            claimed_ip = Some(rest.trim().trim_end_matches('.').to_string());
        } else if line.starts_with("Port forwarding is active") {
            port_forwarding = PortForwardingClaim::Active;
        } else if line.contains("does not support it") {
            port_forwarding = PortForwardingClaim::Unsupported;
        }
    }

    let server = server?;
    Some(ConnectOutcome {
        server,
        location,
        claimed_ip,
        port_forwarding,
    })
}

/// `Error: …` lines, plus the two special cases worth naming.
///
/// The coexistence message is the one from `docs/cli-surface.md` §2: the CLI refuses to run at
/// all while the official GTK app owns `proton.vpn.app.gtk`. It must be surfaced as itself, not
/// as a generic failure.
pub fn error(text: &str, invocation: crate::model::InvocationId) -> Option<CliError> {
    for line in meaningful_lines(text) {
        let message = if let Some(rest) = line.strip_prefix("Error:") {
            rest.trim()
        } else if line.contains("Proton VPN desktop app is currently running") {
            line
        } else {
            continue;
        };

        let kind = if message.contains("desktop app is currently running")
            || message.contains("cannot run simultaneously")
        {
            CliErrorKind::Coexistence
        } else if message.starts_with("Invalid") || message.contains("Please use a valid") {
            CliErrorKind::Validation
        } else {
            CliErrorKind::Other
        };
        return Some(CliError {
            kind,
            message: message.to_string(),
            invocation,
        });
    }
    None
}

/// True when the CLI itself says it is not signed in.
pub fn not_signed_in(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("not logged in")
        || lower.contains("please sign in")
        || lower.contains("not signed in")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::InvocationId;

    fn fixture(name: &str) -> String {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/pty")
            .join(format!("{name}.txt"));
        let raw = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
        normalise(&raw)
    }

    #[test]
    fn parses_connected_status() {
        let status = status(&fixture("status_connected")).unwrap();
        assert_eq!(
            status,
            ConnectionStatus::Connected(ConnectedInfo {
                server: "NL#818".into(),
                location: "Amsterdam, Netherlands".into(),
                load_percent: Some(59),
                protocol: Some("wireguard".into()),
            })
        );
    }

    #[test]
    fn parses_disconnected_status() {
        assert_eq!(
            status(&fixture("status_disconnected")),
            Some(ConnectionStatus::Disconnected)
        );
        assert_eq!(
            status(&fixture("status_disconnected_after")),
            Some(ConnectionStatus::Disconnected)
        );
    }

    #[test]
    fn status_tolerates_the_cache_warmup_notice() {
        let text = format!(
            "Server list is outdated, updating... This may take a moment.\n{}",
            fixture("status_connected")
        );
        assert!(matches!(
            status(&text),
            Some(ConnectionStatus::Connected(_))
        ));
    }

    #[test]
    fn status_refuses_to_guess_on_unknown_output() {
        assert_eq!(status("Login: please sign in"), None);
        assert_eq!(status(""), None);
    }

    #[test]
    fn parses_the_country_table_at_both_captured_widths() {
        let wide = countries(&fixture("countries_list")).unwrap();
        let narrow = countries(&fixture("countries_list_cols80")).unwrap();
        assert_eq!(wide, narrow);
        // 149 countries as captured; the number moves with Proton's network, the shape does not.
        assert!(
            wide.len() > 100,
            "expected the full country list, got {}",
            wide.len()
        );
        assert!(wide.contains(&Country {
            name: "Switzerland".into(),
            code: "CH".into()
        }));
        assert!(wide.contains(&Country {
            name: "Democratic Republic of the Congo".into(),
            code: "CD".into()
        }));
        // Two columns only: names with spaces must not be split.
        assert!(
            wide.iter()
                .all(|c| c.code.len() == 2 && c.code.chars().all(|ch| ch.is_ascii_uppercase()))
        );
    }

    #[test]
    fn country_table_skips_the_warmup_notice() {
        let text = format!(
            "Server list is outdated, updating... This may take a moment.\n{}",
            fixture("countries_list")
        );
        assert_eq!(
            countries(&text).unwrap().len(),
            countries(&fixture("countries_list")).unwrap().len()
        );
    }

    #[test]
    fn parses_the_city_table() {
        let (country, cities) = cities(&fixture("cities_list_ch")).unwrap();
        assert_eq!(country, "Switzerland");
        assert_eq!(
            cities,
            vec![City {
                name: "Zurich".into(),
                features: vec!["P2P".into(), "Tor".into()],
            }]
        );
    }

    #[test]
    fn parses_the_settings_table() {
        let settings = settings(&fixture("config_list")).unwrap();
        assert_eq!(settings.len(), 8);
        assert!(settings.contains(&Setting {
            key: "port-forwarding".into(),
            value: "on".into()
        }));
        // The trailing "Use 'protonvpn config set …'" help lines are not rows.
        assert!(settings.iter().all(|s| !s.key.contains("Use")));
    }

    #[test]
    fn parses_the_account() {
        assert_eq!(
            account(&fixture("info")),
            Some(Account {
                name: Some("trousev".into())
            })
        );
    }

    #[test]
    fn parses_connect_output() {
        let outcome = connect(&fixture("connect_nl")).unwrap();
        assert_eq!(outcome.server, "NL#818");
        assert_eq!(outcome.location, "Amsterdam, Netherlands");
        assert_eq!(outcome.claimed_ip.as_deref(), Some("205.147.16.100"));
        assert_eq!(outcome.port_forwarding, PortForwardingClaim::Active);
    }

    #[test]
    fn detects_a_server_that_does_not_support_forwarding() {
        // Captured through a pipe in Phase 0; kept in the pipe corpus.
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/pipe/connect_ch.txt");
        let raw = std::fs::read_to_string(path).unwrap();
        let outcome = connect(&normalise(&raw)).unwrap();
        assert_eq!(outcome.server, "CH#274");
        assert_eq!(outcome.port_forwarding, PortForwardingClaim::Unsupported);
    }

    #[test]
    fn connect_output_that_is_not_a_connect_parses_to_nothing() {
        assert_eq!(connect(&fixture("status_connected")), None);
    }

    #[test]
    fn parses_validation_errors() {
        let err = error(&fixture("connect_invalid_country"), InvocationId(1)).unwrap();
        assert_eq!(err.kind, CliErrorKind::Validation);
        assert!(err.message.contains("Invalid country code 'ZZ'"));

        let err = error(&fixture("connect_invalid_server"), InvocationId(2)).unwrap();
        assert!(err.message.contains("Invalid server ID 'ZZ#99'"));
    }

    #[test]
    fn recognises_the_coexistence_error() {
        let text = "Error: Proton VPN desktop app is currently running\nThe CLI and GUI cannot \
                    run simultaneously. Please close the GUI application and try again.";
        let err = error(text, InvocationId(3)).unwrap();
        assert_eq!(err.kind, CliErrorKind::Coexistence);
    }

    #[test]
    fn plain_output_produces_no_error() {
        assert_eq!(error(&fixture("status_connected"), InvocationId(1)), None);
        assert_eq!(error(&fixture("disconnect"), InvocationId(1)), None);
    }

    #[test]
    fn split_columns_keeps_inner_single_spaces() {
        assert_eq!(
            split_columns("Costa Rica                        CR"),
            vec!["Costa Rica".to_string(), "CR".to_string()]
        );
        assert_eq!(
            split_columns("City    Features"),
            vec!["City".to_string(), "Features".to_string()]
        );
    }
}
