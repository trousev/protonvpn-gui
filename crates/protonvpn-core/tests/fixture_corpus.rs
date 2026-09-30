//! Fixture-corpus guarantees.
//!
//! The parsers (added in the next phase) will be written against the **PTY** corpus, because a
//! PTY is what the application actually uses — `signin` requires one, so there is a single code
//! path rather than two. These tests pin down the properties that corpus was chosen for, so a
//! future `protonvpn` release that changes any of them fails loudly here instead of subtly
//! breaking a parser.
//!
//! See `docs/cli-surface.md` §4.8.

use std::fs;
use std::path::PathBuf;

use protonvpn_core::pty::strip_ansi;

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn read(dir: &str, name: &str) -> String {
    let path = fixtures_dir().join(dir).join(format!("{name}.txt"));
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

/// Commands whose output does not depend on which server is connected or when they ran.
///
/// Each was captured twice — once through a pipe, once through a PTY — and must agree.
const DETERMINISTIC_PAIRS: &[(&str, &str)] = &[
    ("status_disconnected", "status_disconnected"),
    ("connect_invalid_country", "connect_invalid_country"),
    ("connect_invalid_server", "connect_invalid_server"),
    ("disconnect", "disconnect"),
    // The no-op case was named differently in the original pipe corpus.
    ("disconnect_noop", "disconnect_when_disconnected"),
];

/// A catch-all list, so a newly added fixture is not silently skipped by the ANSI check.
const ALL_PTY_FIXTURES: &[&str] = &[
    "cities_list_ch",
    "config_list",
    "connect_invalid_country",
    "connect_invalid_server",
    "connect_nl",
    "countries_list",
    "countries_list_cols80",
    "disconnect",
    "disconnect_noop",
    "info",
    "status_connected",
    "status_disconnected",
    "status_disconnected_after",
];

#[test]
fn pty_output_matches_pipe_output_for_deterministic_commands() {
    for (pty_name, pipe_name) in DETERMINISTIC_PAIRS {
        let via_pty = strip_ansi(&read("pty", pty_name));
        let via_pipe = read("pipe", pipe_name);
        assert_eq!(
            via_pty.trim_end(),
            via_pipe.trim_end(),
            "PTY and pipe output diverged for `{pty_name}` — a TTY has started changing what \
             the CLI prints, and the parsers may need to account for it"
        );
    }
}

#[test]
fn pty_fixtures_contain_no_ansi_escapes() {
    for name in ALL_PTY_FIXTURES {
        let raw = read("pty", name);
        assert!(
            !raw.contains('\u{1b}'),
            "`{name}` now contains ANSI escapes; the interpreter's strip_ansi path is no longer \
             merely defensive and needs to be exercised for real"
        );
    }
}

#[test]
fn pty_fixtures_use_terminal_line_endings() {
    // Every capture is CRLF-terminated: the terminal layer translates the CLI's `\n`. Consumers
    // that read raw bytes must normalise. `strip_ansi` does.
    let raw = read("pty", "status_connected");
    assert!(
        raw.contains('\r'),
        "expected CRLF line endings in a PTY capture"
    );
    assert!(!strip_ansi(&raw).contains('\r'));
}

#[test]
fn countries_table_is_independent_of_terminal_width() {
    let wide = strip_ansi(&read("pty", "countries_list"));
    let narrow = strip_ansi(&read("pty", "countries_list_cols80"));

    // The only permissible difference is the server-list cache warm-up notice, which depends on
    // whether the cache happened to be stale, not on the terminal.
    let strip_warmup = |s: &str| {
        s.lines()
            .filter(|l| !l.starts_with("Server list is outdated"))
            .collect::<Vec<_>>()
            .join("\n")
    };

    assert_eq!(
        strip_warmup(&wide),
        strip_warmup(&narrow),
        "the countries table now depends on terminal width; parsers must not assume a fixed \
         layout, or the capture width must be pinned"
    );
}

#[test]
fn connected_status_has_the_expected_shape() {
    // Not a parser test — a corpus sanity check. If the CLI changes these keys, the parser has to
    // change with it, and this failure is the early warning.
    let text = strip_ansi(&read("pty", "status_connected"));
    for key in ["Status:", "Server:", "Load:", "Protocol:"] {
        assert!(
            text.lines().any(|l| l.starts_with(key)),
            "`{key}` is missing from the connected status fixture:\n{text}"
        );
    }
    assert!(text.starts_with("Status: Connected"));
}

#[test]
fn disconnected_status_is_a_single_line() {
    let text = strip_ansi(&read("pty", "status_disconnected"));
    assert_eq!(text.trim_end(), "Status: Disconnected");
}
