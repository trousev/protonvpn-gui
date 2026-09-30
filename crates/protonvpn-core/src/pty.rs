//! PTY driver — the one way this project runs `protonvpn`.
//!
//! Every command is spawned attached to a pseudo-terminal rather than a pipe, for two reasons:
//!
//! 1. `protonvpn signin` prompts for a password and possibly a 2FA code, which needs a TTY.
//!    Using a PTY everywhere means one code path instead of two.
//! 2. The console shows the CLI's *real* output, ANSI sequences included, rather than a
//!    sanitised approximation. Interpreting that output is the interpreter's job, and it strips
//!    the escape sequences before parsing.
//!
//! See `docs/architecture.md` §10.3.

use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use portable_pty::{CommandBuilder, PtySize, native_pty_system};

/// The command line as it would be typed, for display in the console.
///
/// `argv` is rendered verbatim; only arguments that genuinely need quoting get it, and quoting is
/// purely cosmetic — the launcher never builds a shell string.
pub fn command_line(argv: &[String]) -> String {
    argv.iter()
        .map(|a| shell_quote(a))
        .collect::<Vec<_>>()
        .join(" ")
}

/// A handle for writing to a running child's terminal, usable from another thread.
///
/// This is how `signin` is answered: the child blocks on a password prompt, and whichever thread
/// the UI is on writes the secret straight to the PTY. Secrets never touch the log bus.
#[derive(Clone)]
pub struct PtyStdin {
    writer: Arc<Mutex<Option<Box<dyn Write + Send>>>>,
}

impl PtyStdin {
    pub fn write_line(&self, line: &str) -> std::io::Result<()> {
        let mut guard = self
            .writer
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match guard.as_mut() {
            Some(writer) => {
                writer.write_all(line.as_bytes())?;
                writer.write_all(b"\n")?;
                writer.flush()
            }
            None => Err(std::io::Error::other("the child's terminal is closed")),
        }
    }

    /// Close our end of the terminal, so a child waiting on input sees EOF instead of hanging.
    pub fn close(&self) {
        let mut guard = self
            .writer
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *guard = None;
    }

    pub fn is_open(&self) -> bool {
        self.writer
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_some()
    }
}

/// A completed invocation, recorded verbatim.
#[derive(Debug, Clone)]
pub struct Invocation {
    /// The command as executed, including argv[0].
    pub argv: Vec<String>,
    /// Terminal geometry the command ran with.
    pub cols: u16,
    pub rows: u16,
    /// Process exit code, as reported by the terminal layer.
    ///
    /// Note the CLI's contract, measured: `0` means success and `2` means a validation error.
    /// Anything else is not mapped — consumers should show the raw output rather than guess.
    pub exit_code: u32,
    /// Raw output, stdout and stderr merged by the terminal, **escape sequences included**.
    pub output: String,
    pub duration: Duration,
}

impl Invocation {
    /// Output with ANSI escape sequences removed — what the interpreter should parse.
    pub fn output_plain(&self) -> String {
        strip_ansi(&self.output)
    }

    /// The command line as it would be typed, for display in the console.
    pub fn command_line(&self) -> String {
        command_line(&self.argv)
    }
}

/// Errors are kept as strings so `anyhow` (which `portable-pty` uses internally) does not leak
/// into this crate's public API.
#[derive(Debug)]
pub enum PtyError {
    OpenPty(String),
    Spawn(String),
    Io(String),
}

impl std::fmt::Display for PtyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OpenPty(e) => write!(f, "failed to open pty: {e}"),
            Self::Spawn(e) => write!(f, "failed to spawn child: {e}"),
            Self::Io(e) => write!(f, "pty i/o error: {e}"),
        }
    }
}

impl std::error::Error for PtyError {}

/// Runs a command attached to a fresh PTY and captures everything it writes.
///
/// Blocks until the child exits. Use [`SpawnedInvocation`] instead when the output needs to be
/// streamed — which is what the console requires.
pub fn run(argv: &[String], cols: u16, rows: u16) -> Result<Invocation, PtyError> {
    let mut spawned = spawn(argv, cols, rows)?;
    let output = spawned.read_to_end()?;
    spawned.finish(output)
}

/// A running child attached to a PTY, for streaming consumers.
pub struct SpawnedInvocation {
    child: Box<dyn portable_pty::Child + Send + Sync>,
    reader: Box<dyn Read + Send>,
    stdin: PtyStdin,
    argv: Vec<String>,
    cols: u16,
    rows: u16,
    started: Instant,
}

impl SpawnedInvocation {
    pub fn argv(&self) -> &[String] {
        &self.argv
    }

    /// A cloneable handle for writing to the child's terminal from another thread.
    pub fn stdin_handle(&self) -> PtyStdin {
        self.stdin.clone()
    }

    /// Writes to the child's stdin — used for password / 2FA prompts on `signin`.
    pub fn write_line(&mut self, line: &str) -> std::io::Result<()> {
        self.stdin.write_line(line)
    }

    /// Reads whatever is available; returns 0 at EOF. Callers stream this into the log bus.
    pub fn read_chunk(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.reader.read(buf)
    }

    pub fn read_to_end(&mut self) -> Result<String, PtyError> {
        let mut raw = Vec::new();
        self.reader
            .read_to_end(&mut raw)
            .map_err(|e| PtyError::Io(e.to_string()))?;
        Ok(String::from_utf8_lossy(&raw).into_owned())
    }

    /// Takes the reader out, so a streaming consumer can own it on its own thread.
    pub fn take_reader(&mut self) -> Box<dyn Read + Send> {
        std::mem::replace(&mut self.reader, Box::new(std::io::empty()))
    }

    /// Waits for the child and completes the record.
    pub fn finish(mut self, output: String) -> Result<Invocation, PtyError> {
        let status = self.wait()?;
        Ok(Invocation {
            argv: self.argv,
            cols: self.cols,
            rows: self.rows,
            exit_code: status,
            output,
            duration: self.started.elapsed(),
        })
    }

    /// Kills the child. Used only when an invocation exceeds its deadline, so that a wedged
    /// command cannot leave the application saying "работаю" forever.
    pub fn kill(&mut self) -> Result<(), PtyError> {
        self.child.kill().map_err(|e| PtyError::Io(e.to_string()))
    }

    /// Waits for the child, returning its exit code.
    pub fn wait(&mut self) -> Result<u32, PtyError> {
        let status = self.child.wait().map_err(|e| PtyError::Io(e.to_string()))?;
        Ok(status.exit_code())
    }

    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }
}

/// Spawns `argv` on a fresh PTY. The caller owns reading and reaping.
pub fn spawn(argv: &[String], cols: u16, rows: u16) -> Result<SpawnedInvocation, PtyError> {
    let argv0 = argv
        .first()
        .ok_or_else(|| PtyError::Spawn("empty argv".into()))?;

    let pty = native_pty_system()
        .openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| PtyError::OpenPty(e.to_string()))?;

    let mut cmd = CommandBuilder::new(argv0);
    for arg in &argv[1..] {
        cmd.arg(arg);
    }
    // Make the CLI's terminal detection behave as it would in a real terminal.
    cmd.env("TERM", "xterm-256color");

    let child = pty
        .slave
        .spawn_command(cmd)
        .map_err(|e| PtyError::Spawn(e.to_string()))?;
    drop(pty.slave);

    let reader = pty
        .master
        .try_clone_reader()
        .map_err(|e| PtyError::Io(e.to_string()))?;
    let writer = pty.master.take_writer().ok();

    Ok(SpawnedInvocation {
        child,
        reader,
        stdin: PtyStdin {
            writer: Arc::new(Mutex::new(writer)),
        },
        argv: argv.to_vec(),
        cols,
        rows,
        started: Instant::now(),
    })
}

/// Splits a chunk of PTY output into complete lines, returning the lines and the leftover tail.
///
/// The terminal layer translates the CLI's `\n` into `\r\n`, so the carriage return is stripped
/// here. A bare `\r` (progress overwrite) is kept verbatim inside the text: the console is a
/// transcript, and losing bytes is worse than showing an ugly line.
pub fn split_lines(buffer: &mut String) -> Vec<String> {
    let mut lines = Vec::new();
    while let Some(index) = buffer.find('\n') {
        let mut line: String = buffer.drain(..=index).collect();
        line.pop(); // the '\n'
        if line.ends_with('\r') {
            line.pop();
        }
        lines.push(line);
    }
    lines
}

/// Removes ANSI escape sequences (CSI, OSC and friends) and normalises CRLF to LF.
///
/// The console renders the raw stream; the interpreter parses this.
pub fn strip_ansi(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '\u{1b}' => match chars.next() {
                // CSI: ESC [ ... final-byte in @..~
                Some('[') => {
                    for c in chars.by_ref() {
                        if ('\u{40}'..='\u{7e}').contains(&c) {
                            break;
                        }
                    }
                }
                // OSC: ESC ] ... terminated by BEL or ST
                Some(']') => {
                    while let Some(c) = chars.next() {
                        if c == '\u{7}' {
                            break;
                        }
                        if c == '\u{1b}' && chars.peek() == Some(&'\\') {
                            chars.next();
                            break;
                        }
                    }
                }
                // Two-character escapes (ESC ( B, ESC =, ...)
                Some(_) | None => {}
            },
            '\r' => {
                // Keep the newline, drop bare carriage returns.
                if chars.peek() == Some(&'\n') {
                    // The '\n' will be pushed on the next iteration.
                }
            }
            _ => out.push(c),
        }
    }

    out
}

/// Quotes an argument for display, only when it needs it.
pub fn shell_quote(arg: &str) -> String {
    let safe = !arg.is_empty()
        && arg
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./=:@#+".contains(c));
    if safe {
        arg.to_string()
    } else {
        format!("'{}'", arg.replace('\'', r"'\''"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_csi_colour() {
        assert_eq!(strip_ansi("\u{1b}[31mred\u{1b}[0m"), "red");
    }

    #[test]
    fn strips_osc_title() {
        assert_eq!(strip_ansi("\u{1b}]0;title\u{7}body"), "body");
    }

    #[test]
    fn normalises_crlf() {
        assert_eq!(strip_ansi("a\r\nb"), "a\nb");
    }

    #[test]
    fn leaves_plain_text_alone() {
        let s = "Status: Connected\nServer: CH#274 in Zurich, Switzerland\n";
        assert_eq!(strip_ansi(s), s);
    }

    #[test]
    fn quotes_only_when_needed() {
        assert_eq!(shell_quote("connect"), "connect");
        assert_eq!(shell_quote("--country"), "--country");
        assert_eq!(shell_quote("IT#23"), "IT#23");
        assert_eq!(shell_quote("New York"), "'New York'");
    }

    #[test]
    fn renders_a_command_line_for_display() {
        let argv: Vec<String> = ["protonvpn", "connect", "--city", "New York"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(command_line(&argv), "protonvpn connect --city 'New York'");
    }

    #[test]
    fn splits_streamed_lines_and_keeps_the_tail() {
        let mut buffer = String::from("Status: Connected\r\nServer: NL#818");
        let lines = split_lines(&mut buffer);
        assert_eq!(lines, vec!["Status: Connected".to_string()]);
        assert_eq!(buffer, "Server: NL#818");

        buffer.push_str(" in Amsterdam, Netherlands\r\n");
        let lines = split_lines(&mut buffer);
        assert_eq!(
            lines,
            vec!["Server: NL#818 in Amsterdam, Netherlands".to_string()]
        );
        assert!(buffer.is_empty());
    }

    #[test]
    fn a_bare_carriage_return_is_preserved_verbatim() {
        let mut buffer = String::from("progress 10%\rprogress 20%\n");
        let lines = split_lines(&mut buffer);
        assert_eq!(lines, vec!["progress 10%\rprogress 20%".to_string()]);
    }
}
