//! The log bus — one ordered, verbatim stream with two consumers.
//!
//! `docs/architecture.md` §3. The console pane and the interpreter read **the same events**, so
//! they can never disagree about what the CLI said. Text is stored exactly as received: no
//! rewrapping, no re-parsing in place, no sanitising. Formatting is a render-time concern.
//!
//! Storage is bounded twice over, because a wrapper must survive a child that prints forever:
//!
//! * a ring buffer of finished invocations (oldest dropped whole, and counted, never silently),
//! * a per-invocation line cap (oldest lines dropped, and counted).

use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use crate::model::InvocationId;

/// Default ring-buffer size: enough scrollback for a session, small enough to be free.
pub const DEFAULT_INVOCATION_CAPACITY: usize = 250;
/// Per-invocation line cap. `countries list` is ~250 lines; this is two orders of magnitude up.
pub const DEFAULT_LINE_CAPACITY: usize = 20_000;

/// Where an invocation came from. A `Note` is **our** narration (the `curl` probe, a NAT-PMP
/// renewal) and must never be dressed up as a `protonvpn` command —
/// `docs/architecture.md` §0.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvocationKind {
    /// A real `protonvpn` child process attached to a PTY.
    ProtonVpn,
    /// Something else we did, shown honestly as such: `curl`, NAT-PMP.
    Note,
}

impl InvocationKind {
    pub fn is_protonvpn(self) -> bool {
        matches!(self, Self::ProtonVpn)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogLine {
    pub at: SystemTime,
    /// The line as received, minus the trailing newline. Never rewritten.
    pub text: String,
}

/// The full invocation record required by `docs/architecture.md` §2.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invocation {
    pub id: InvocationId,
    pub kind: InvocationKind,
    /// argv verbatim, `argv[0]` included. Empty for a pure note.
    pub argv: Vec<String>,
    /// Display override for notes, e.g. `POST http://localhost:8080/api/v2/app/setPreferences`.
    pub display: Option<String>,
    pub cwd: PathBuf,
    pub started_at: SystemTime,
    pub finished_at: Option<SystemTime>,
    pub exit_code: Option<u32>,
    pub duration: Option<Duration>,
    pub lines: Vec<LogLine>,
    /// Lines dropped from the front of `lines` by the per-invocation cap. Non-zero only when a
    /// child out-printed the cap, and always shown.
    pub lines_dropped: usize,
}

impl Invocation {
    pub fn is_running(&self) -> bool {
        self.finished_at.is_none()
    }

    /// The command line as displayed: verbatim argv for real commands, the honest note label
    /// otherwise.
    pub fn command_line(&self) -> String {
        match &self.display {
            Some(display) => display.clone(),
            None => crate::pty::command_line(&self.argv),
        }
    }

    /// Output, verbatim, one line per element.
    pub fn output(&self) -> String {
        let mut out = String::new();
        for line in &self.lines {
            out.push_str(&line.text);
            out.push('\n');
        }
        out
    }

    /// Everything the console's copy button should copy: command, output, and the footer.
    pub fn transcript(&self) -> String {
        let mut out = format!("$ {}\n", self.command_line());
        if self.lines_dropped > 0 {
            out.push_str(&format!("… {} earlier lines dropped\n", self.lines_dropped));
        }
        out.push_str(&self.output());
        out.push_str(&self.footer());
        out
    }

    pub fn footer(&self) -> String {
        match (self.exit_code, self.duration) {
            (Some(code), Some(duration)) => {
                format!("exit {code} · {:.1}s\n", duration.as_secs_f64())
            }
            _ => "… running\n".to_string(),
        }
    }
}

/// One step of the stream. This is what both consumers read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogEvent {
    Started {
        id: InvocationId,
        kind: InvocationKind,
        argv: Vec<String>,
        display: Option<String>,
        cwd: PathBuf,
        at: SystemTime,
    },
    Line {
        id: InvocationId,
        line: LogLine,
    },
    Finished {
        id: InvocationId,
        exit_code: Option<u32>,
        duration: Duration,
        at: SystemTime,
    },
}

impl LogEvent {
    pub fn invocation(&self) -> InvocationId {
        match self {
            Self::Started { id, .. } | Self::Line { id, .. } | Self::Finished { id, .. } => *id,
        }
    }

    /// The raw text, when this event carries any.
    pub fn text(&self) -> Option<&str> {
        match self {
            Self::Line { line, .. } => Some(&line.text),
            _ => None,
        }
    }
}

/// The bus. One instance, owned by the engine thread; readers take a short lock.
#[derive(Debug)]
pub struct LogBus {
    invocations: VecDeque<Invocation>,
    invocation_capacity: usize,
    line_capacity: usize,
    next_id: u64,
    dropped_invocations: u64,
    /// Bumped on every mutation, so a view can tell in one comparison whether its rendering is
    /// still current instead of rebuilding the transcript every frame.
    version: u64,
}

impl Default for LogBus {
    fn default() -> Self {
        Self::new(DEFAULT_INVOCATION_CAPACITY, DEFAULT_LINE_CAPACITY)
    }
}

impl LogBus {
    pub fn new(invocation_capacity: usize, line_capacity: usize) -> Self {
        Self {
            invocations: VecDeque::new(),
            invocation_capacity: invocation_capacity.max(1),
            line_capacity: line_capacity.max(1),
            next_id: 1,
            dropped_invocations: 0,
            version: 0,
        }
    }

    /// Reserves the next invocation id.
    ///
    /// The id is allocated when the caller *decides* to run something and handed to
    /// [`LogBus::begin`] when the record is opened, because the two moments are not the same: a job
    /// queued behind a running one has an id before it has a record. Letting `begin` allocate
    /// instead gave the bus a second counter, and anything recorded in between — a `curl` reading,
    /// a NAT-PMP renewal — took the queued job's number. The swap is not cosmetic: every later line
    /// of that job is filed under the note, and the interpreter reads the note's output as the
    /// command's.
    pub fn next_id(&mut self) -> InvocationId {
        let id = InvocationId(self.next_id);
        self.next_id += 1;
        id
    }

    pub fn begin(
        &mut self,
        id: InvocationId,
        kind: InvocationKind,
        argv: Vec<String>,
        display: Option<String>,
        cwd: PathBuf,
        at: SystemTime,
    ) {
        self.version += 1;
        self.invocations.push_back(Invocation {
            id,
            kind,
            argv,
            display,
            cwd,
            started_at: at,
            finished_at: None,
            exit_code: None,
            duration: None,
            lines: Vec::new(),
            lines_dropped: 0,
        });
        while self.invocations.len() > self.invocation_capacity {
            self.invocations.pop_front();
            self.dropped_invocations += 1;
        }
    }

    /// Appends a line, applying the per-invocation cap.
    pub fn push_line(&mut self, id: InvocationId, text: impl Into<String>, at: SystemTime) {
        self.version += 1;
        let capacity = self.line_capacity;
        let Some(invocation) = self.find_mut(id) else {
            return;
        };
        invocation.lines.push(LogLine {
            at,
            text: text.into(),
        });
        while invocation.lines.len() > capacity {
            invocation.lines.remove(0);
            invocation.lines_dropped += 1;
        }
    }

    pub fn finish(
        &mut self,
        id: InvocationId,
        exit_code: Option<u32>,
        duration: Duration,
        at: SystemTime,
    ) {
        self.version += 1;
        if let Some(invocation) = self.find_mut(id) {
            invocation.finished_at = Some(at);
            invocation.exit_code = exit_code;
            invocation.duration = Some(duration);
        }
    }

    fn find_mut(&mut self, id: InvocationId) -> Option<&mut Invocation> {
        // The running invocation is always last, so searching backwards hits it immediately.
        self.invocations.iter_mut().rev().find(|i| i.id == id)
    }

    pub fn get(&self, id: InvocationId) -> Option<&Invocation> {
        self.invocations.iter().rev().find(|i| i.id == id)
    }

    pub fn running(&self) -> Option<&Invocation> {
        self.invocations.iter().rev().find(|i| i.is_running())
    }

    pub fn last(&self) -> Option<&Invocation> {
        self.invocations.back()
    }

    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &Invocation> {
        self.invocations.iter()
    }

    pub fn len(&self) -> usize {
        self.invocations.len()
    }

    pub fn is_empty(&self) -> bool {
        self.invocations.is_empty()
    }

    /// How many invocations the ring buffer has discarded. Shown in the console, so the user
    /// knows the transcript is partial rather than believing it is complete.
    pub fn dropped_invocations(&self) -> u64 {
        self.dropped_invocations
    }

    /// Monotonic change counter for view caching.
    pub fn version(&self) -> u64 {
        self.version
    }

    /// The whole transcript, for the console's "copy everything" button.
    pub fn transcript(&self) -> String {
        let mut out = String::new();
        if self.dropped_invocations > 0 {
            out.push_str(&format!(
                "… {} earlier invocations dropped from the buffer\n\n",
                self.dropped_invocations
            ));
        }
        for invocation in &self.invocations {
            out.push_str(&invocation.transcript());
            out.push('\n');
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bus() -> LogBus {
        LogBus::new(3, 4)
    }

    #[test]
    fn keeps_the_ring_buffer_bounded_and_counts_what_it_dropped() {
        let mut bus = bus();
        for n in 0..5 {
            let id = bus.next_id();
            bus.begin(
                id,
                InvocationKind::ProtonVpn,
                vec!["protonvpn".into(), format!("cmd{n}")],
                None,
                PathBuf::from("/tmp"),
                SystemTime::now(),
            );
            bus.push_line(id, format!("line {n}"), SystemTime::now());
            bus.finish(id, Some(0), Duration::from_secs(1), SystemTime::now());
        }
        assert_eq!(bus.len(), 3);
        assert_eq!(bus.dropped_invocations(), 2);
        assert!(bus.transcript().contains("2 earlier invocations dropped"));
        assert!(bus.last().unwrap().command_line().contains("cmd4"));
    }

    #[test]
    fn caps_lines_per_invocation_and_counts_them() {
        let mut bus = bus();
        let id = bus.next_id();
        bus.begin(
            id,
            InvocationKind::ProtonVpn,
            vec!["protonvpn".into(), "status".into()],
            None,
            PathBuf::from("/tmp"),
            SystemTime::now(),
        );
        for n in 0..6 {
            bus.push_line(id, format!("l{n}"), SystemTime::now());
        }
        let invocation = bus.get(id).unwrap();
        assert_eq!(invocation.lines.len(), 4);
        assert_eq!(invocation.lines_dropped, 2);
        assert_eq!(invocation.lines[0].text, "l2");
        assert!(invocation.transcript().contains("2 earlier lines dropped"));
    }

    #[test]
    fn invocation_records_exit_code_and_duration() {
        let mut bus = bus();
        let id = bus.next_id();
        bus.begin(
            id,
            InvocationKind::ProtonVpn,
            vec!["protonvpn".into(), "disconnect".into()],
            None,
            PathBuf::from("/tmp"),
            SystemTime::now(),
        );
        bus.push_line(id, "Disconnected.", SystemTime::now());
        bus.finish(id, Some(0), Duration::from_millis(1340), SystemTime::now());
        let invocation = bus.get(id).unwrap();
        assert_eq!(invocation.exit_code, Some(0));
        assert_eq!(invocation.footer(), "exit 0 · 1.3s\n");
        assert!(!invocation.is_running());
        assert_eq!(invocation.output(), "Disconnected.\n");
    }

    #[test]
    fn a_running_invocation_has_no_verdict_yet() {
        let mut bus = bus();
        let id = bus.next_id();
        bus.begin(
            id,
            InvocationKind::ProtonVpn,
            vec!["protonvpn".into(), "connect".into()],
            None,
            PathBuf::from("/tmp"),
            SystemTime::now(),
        );
        let invocation = bus.get(id).unwrap();
        assert!(invocation.is_running());
        assert_eq!(invocation.footer(), "… running\n");
        assert_eq!(invocation.exit_code, None);
        assert!(bus.running().is_some());
    }

    #[test]
    fn the_version_advances_on_every_change() {
        let mut bus = bus();
        let start = bus.version();
        let id = bus.next_id();
        bus.begin(
            id,
            InvocationKind::ProtonVpn,
            vec!["protonvpn".into(), "status".into()],
            None,
            PathBuf::from("/tmp"),
            SystemTime::now(),
        );
        assert!(bus.version() > start);
        let after_begin = bus.version();
        bus.push_line(id, "Status: Disconnected", SystemTime::now());
        assert!(bus.version() > after_begin);
        let after_line = bus.version();
        bus.finish(id, Some(0), Duration::from_secs(1), SystemTime::now());
        assert!(bus.version() > after_line);
    }

    /// Ids belong to the caller, so a record opened *later* than another still owns the number it
    /// reserved first. This is the shape that used to break: a job is queued under one id,
    /// something else is recorded before that job starts, and the two records swap — after which
    /// every line of the job is filed under the note, and the interpreter reads the note's output
    /// as the command's.
    #[test]
    fn a_queued_job_keeps_its_id_when_something_else_is_recorded_first() {
        let mut bus = bus();
        let queued = bus.next_id();
        let note = bus.next_id();

        bus.begin(
            note,
            InvocationKind::Note,
            Vec::new(),
            Some("NAT-PMP renew 39949".into()),
            PathBuf::from("/tmp"),
            SystemTime::now(),
        );
        bus.push_line(note, "аренда продлена", SystemTime::now());

        bus.begin(
            queued,
            InvocationKind::ProtonVpn,
            vec!["protonvpn".into(), "info".into()],
            None,
            PathBuf::from("/tmp"),
            SystemTime::now(),
        );
        bus.push_line(queued, "Account: 'trousev'", SystemTime::now());

        assert_eq!(bus.get(note).unwrap().output(), "аренда продлена\n");
        assert_eq!(bus.get(queued).unwrap().output(), "Account: 'trousev'\n");
    }

    #[test]
    fn notes_are_labelled_honestly_and_never_look_like_commands() {
        let mut bus = bus();
        let id = bus.next_id();
        bus.begin(
            id,
            InvocationKind::Note,
            Vec::new(),
            Some(
                "POST http://localhost:8080/api/v2/app/setPreferences {listen_port: 39949}".into(),
            ),
            PathBuf::from("/tmp"),
            SystemTime::now(),
        );
        let invocation = bus.get(id).unwrap();
        assert_eq!(
            invocation.command_line(),
            "POST http://localhost:8080/api/v2/app/setPreferences {listen_port: 39949}"
        );
        assert!(!invocation.kind.is_protonvpn());
        assert!(invocation.argv.is_empty());
    }
}
