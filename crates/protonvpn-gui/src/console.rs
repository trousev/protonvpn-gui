//! The console pane's model — the transcript, prepared for rendering.
//!
//! `docs/architecture.md` §4: the console is the product. It shows, in order, the command line
//! **verbatim** (flags included, never paraphrased), the output **verbatim**, and per invocation
//! the exit code and duration. It is read-only: a transcript, not a terminal emulator.
//!
//! This module is deliberately free of any UI type, so what the console will show can be tested
//! without a toolkit. Rendering lives in `app.rs`.
//!
//! Two bounds keep a chatty child from turning the window into a memory test, and both are
//! *visible*: how many invocations are rendered, and how many lines of a single invocation. When
//! something is dropped the pane says so, because a transcript that quietly loses the middle is
//! worse than one that admits its limits.

use protonvpn_core::logbus::LogBus;
use protonvpn_core::model::InvocationId;

/// How many invocations the expanded console renders at once.
pub const MAX_INVOCATIONS: usize = 40;
/// How many lines of each invocation. `countries list` is ~250, so this holds a whole table.
pub const MAX_LINES: usize = 400;

/// One invocation, ready to draw.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub id: InvocationId,
    pub command: String,
    pub output: String,
    pub footer: String,
    pub running: bool,
    /// Lines hidden by [`MAX_LINES`]. Shown when non-zero.
    pub lines_hidden: usize,
    /// Whole-invocation text for this block's copy button.
    pub transcript: String,
}

/// The console as the view sees it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConsoleModel {
    /// The bus version this model was built from, so a frame can skip a rebuild.
    version: u64,
    pub blocks: Vec<Block>,
    /// Invocations dropped by the bus's ring buffer, reported rather than hidden.
    pub dropped_invocations: u64,
    /// Whole-transcript text for the "copy everything" button.
    pub transcript: String,
}

impl ConsoleModel {
    /// Rebuilds only if the bus has changed since the last build.
    pub fn refresh(&mut self, bus: &LogBus) {
        if self.version == bus_version(bus) {
            return;
        }
        self.rebuild(bus);
    }

    pub fn rebuild(&mut self, bus: &LogBus) {
        self.version = bus_version(bus);
        self.dropped_invocations = bus.dropped_invocations();
        self.transcript = bus.transcript();

        let mut blocks: Vec<Block> = bus
            .iter()
            .rev()
            .take(MAX_INVOCATIONS)
            .map(|invocation| {
                let total = invocation.lines.len();
                let skip = total.saturating_sub(MAX_LINES);
                let lines: Vec<&str> = invocation
                    .lines
                    .iter()
                    .skip(skip)
                    .map(|line| line.text.as_str())
                    .collect();
                let mut output = lines.join("\n");
                if !output.is_empty() {
                    output.push('\n');
                }
                Block {
                    id: invocation.id,
                    command: invocation.command_line(),
                    output,
                    footer: invocation.footer().trim_end().to_string(),
                    running: invocation.is_running(),
                    lines_hidden: skip + invocation.lines_dropped,
                    transcript: block_transcript(invocation),
                }
            })
            .collect();
        blocks.reverse();
        self.blocks = blocks;
    }

    /// The bus version this model reflects, for change detection in a frame.
    pub fn version(&self) -> u64 {
        self.version
    }

    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }
}

fn bus_version(bus: &LogBus) -> u64 {
    bus.version()
}

/// The exact text a per-invocation copy button should put on the clipboard.
fn block_transcript(invocation: &protonvpn_core::logbus::Invocation) -> String {
    let mut out = format!("$ {}\n", invocation.command_line());
    if invocation.lines_dropped > 0 {
        out.push_str(&format!(
            "… {} earlier lines dropped\n",
            invocation.lines_dropped
        ));
    }
    out.push_str(&invocation.output());
    out.push_str(&invocation.footer());
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use protonvpn_core::logbus::{InvocationKind, LogBus};
    use std::path::PathBuf;
    use std::time::{Duration, SystemTime};

    fn bus_with_invocations(count: usize) -> LogBus {
        let mut bus = LogBus::default();
        for index in 0..count {
            let id = bus.begin(
                InvocationKind::ProtonVpn,
                vec!["protonvpn".into(), format!("cmd{index}")],
                None,
                PathBuf::from("/tmp"),
                SystemTime::now(),
            );
            bus.push_line(id, format!("output {index}"), SystemTime::now());
            bus.finish(id, Some(0), Duration::from_millis(1500), SystemTime::now());
        }
        bus
    }

    #[test]
    fn renders_command_output_and_footer_verbatim() {
        let mut bus = LogBus::default();
        let id = bus.begin(
            InvocationKind::ProtonVpn,
            vec![
                "protonvpn".into(),
                "connect".into(),
                "--country".into(),
                "uk".into(),
            ],
            None,
            PathBuf::from("/tmp"),
            SystemTime::now(),
        );
        bus.push_line(
            id,
            "Connected to UK#123 in London, United Kingdom.",
            SystemTime::now(),
        );
        bus.finish(id, Some(0), Duration::from_millis(3400), SystemTime::now());

        let mut model = ConsoleModel::default();
        model.refresh(&bus);
        let block = &model.blocks[0];
        assert_eq!(block.command, "protonvpn connect --country uk");
        assert_eq!(
            block.output,
            "Connected to UK#123 in London, United Kingdom.\n"
        );
        assert_eq!(block.footer, "exit 0 · 3.4s");
        assert!(!block.running);
        assert_eq!(
            block.transcript,
            "$ protonvpn connect --country uk\nConnected to UK#123 in London, United Kingdom.\nexit 0 · 3.4s\n"
        );
    }

    #[test]
    fn a_running_invocation_has_no_verdict_yet() {
        let mut bus = LogBus::default();
        let id = bus.begin(
            InvocationKind::ProtonVpn,
            vec!["protonvpn".into(), "status".into()],
            None,
            PathBuf::from("/tmp"),
            SystemTime::now(),
        );
        bus.push_line(id, "Status: Connected", SystemTime::now());

        let mut model = ConsoleModel::default();
        model.refresh(&bus);
        assert!(model.blocks[0].running);
        assert_eq!(model.blocks[0].footer, "… running");
        assert_eq!(model.blocks[0].output, "Status: Connected\n");
    }

    #[test]
    fn keeps_chronological_order_and_takes_the_tail() {
        let bus = bus_with_invocations(60);
        let mut model = ConsoleModel::default();
        model.refresh(&bus);
        assert_eq!(model.blocks.len(), MAX_INVOCATIONS);
        // Oldest first, newest last: the console reads top to bottom like a terminal.
        assert_eq!(model.blocks.first().unwrap().command, "protonvpn cmd20");
        assert_eq!(model.blocks.last().unwrap().command, "protonvpn cmd59");
    }

    #[test]
    fn reports_what_the_bus_discarded_instead_of_pretending() {
        let mut bus = LogBus::new(2, 100);
        for index in 0..5 {
            let id = bus.begin(
                InvocationKind::ProtonVpn,
                vec!["protonvpn".into(), format!("cmd{index}")],
                None,
                PathBuf::from("/tmp"),
                SystemTime::now(),
            );
            bus.finish(id, Some(0), Duration::from_secs(1), SystemTime::now());
        }
        let mut model = ConsoleModel::default();
        model.refresh(&bus);
        assert_eq!(model.dropped_invocations, 3);
        assert!(model.transcript.contains("3 earlier invocations dropped"));
    }

    #[test]
    fn hides_the_middle_of_an_oversized_invocation_and_says_so() {
        let mut bus = LogBus::new(10, 1000);
        let id = bus.begin(
            InvocationKind::ProtonVpn,
            vec!["protonvpn".into(), "countries".into(), "list".into()],
            None,
            PathBuf::from("/tmp"),
            SystemTime::now(),
        );
        for line in 0..(MAX_LINES + 50) {
            bus.push_line(id, format!("line {line}"), SystemTime::now());
        }

        let mut model = ConsoleModel::default();
        model.refresh(&bus);
        let block = &model.blocks[0];
        assert_eq!(block.lines_hidden, 50);
        assert!(block.output.starts_with("line 50\n"));
        assert!(
            block
                .output
                .trim_end()
                .ends_with(&format!("line {}", MAX_LINES + 49))
        );
        assert_eq!(block.output.lines().count(), MAX_LINES);
    }

    #[test]
    fn rebuilds_only_when_the_bus_has_changed() {
        let mut bus = bus_with_invocations(2);
        let mut model = ConsoleModel::default();
        model.refresh(&bus);
        let version = model.version;

        // Untouched bus: the model is reused as is.
        model.refresh(&bus);
        assert_eq!(model.version, version);

        let id = bus.begin(
            InvocationKind::ProtonVpn,
            vec!["protonvpn".into(), "status".into()],
            None,
            PathBuf::from("/tmp"),
            SystemTime::now(),
        );
        bus.push_line(id, "Status: Disconnected", SystemTime::now());
        model.refresh(&bus);
        assert_ne!(model.version, version);
        assert_eq!(
            model.blocks.last().unwrap().output,
            "Status: Disconnected\n"
        );
    }

    #[test]
    fn notes_are_shown_as_notes_not_as_commands() {
        let mut bus = LogBus::default();
        let id = bus.begin(
            InvocationKind::Note,
            Vec::new(),
            Some(
                "POST http://localhost:8080/api/v2/app/setPreferences {listen_port: 39949}".into(),
            ),
            PathBuf::from("/tmp"),
            SystemTime::now(),
        );
        bus.push_line(id, "→ 200 OK", SystemTime::now());
        bus.finish(id, Some(0), Duration::from_millis(12), SystemTime::now());

        let mut model = ConsoleModel::default();
        model.refresh(&bus);
        assert_eq!(
            model.blocks[0].command,
            "POST http://localhost:8080/api/v2/app/setPreferences {listen_port: 39949}"
        );
        assert!(!model.blocks[0].transcript.starts_with("$ protonvpn"));
    }
}
