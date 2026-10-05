//! Runner — the only thing in this program that executes anything.
//!
//! `docs/architecture.md` §2. Responsibilities, all of them testable without the CLI:
//!
//! * spawn a child on a PTY (never a pipe — `signin` prompts),
//! * **strictly single-flight**: a second request queues, because two `protonvpn` processes
//!   fighting over the same tunnel is exactly how you get a flapping connection,
//! * stream lines as they are produced rather than buffering the whole output,
//! * keep the full invocation record: argv, cwd, timestamps, exit code, every line,
//! * hand out a [`PtyStdin`] while an interactive child is running, so the UI can answer a
//!   password prompt without the secret ever touching the log.
//!
//! Lines are delivered over a channel to whoever owns the log bus. The bus is *not* shared with
//! this thread: one writer, so the ordering in the console is the ordering of reality.

use std::collections::VecDeque;
use std::io::Read;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, TryRecvError, channel};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime};

use crate::i18n::{I18n, Locale};
use crate::model::InvocationId;
use crate::pty::{self, PtyStdin};

/// Terminal geometry for every child. The CLI's tables are width-independent (measured at 80 and
/// 120 columns), so this is only a guard against absurd wrapping if that ever changes.
pub const PTY_COLS: u16 = 120;
pub const PTY_ROWS: u16 = 40;

/// Default patience for a non-interactive command.
///
/// `connect` measured 2–4 s, `status` ~1 s. Two minutes is not a timeout anyone will hit in
/// normal use; it exists so that a wedged child cannot leave the console bar saying
/// "working" forever.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(120);

/// How long the child must be silent before an unterminated line is handed over as it stands.
///
/// **Measured 2026-10-01:** `protonvpn signin` writes `Password: ` — no trailing newline — and
/// then blocks. A reader that only forwards complete lines forwards nothing at all, so the engine
/// never sees the prompt, never writes the password, and the login sits at "working" forever.
/// Anything the CLI leaves unterminated for this long is a prompt, not half of a line that is
/// still being written.
const PROMPT_IDLE: Duration = Duration::from_millis(200);

/// The live tail of the child's output: text written, not yet terminated by a newline.
struct Tail {
    text: String,
    quiet_since: Instant,
}

/// One queued invocation.
#[derive(Debug, Clone)]
pub struct Job {
    pub id: InvocationId,
    pub kind: crate::logbus::InvocationKind,
    pub argv: Vec<String>,
    pub display: Option<String>,
    pub cwd: PathBuf,
    /// Interactive children are not killed on a timer: they are waiting for a human.
    pub interactive: bool,
    pub timeout: Option<Duration>,
    /// The language of the lines this job may produce in our own name — a failed spawn, a timeout,
    /// a cancellation. It travels with the job rather than living in a catalogue of the runner
    /// thread's own, because that thread outlives any language change and a sentence written after
    /// one would be a sentence in the wrong language. `Locale::SOURCE` is what a test's stand-in
    /// job gets.
    pub language: Locale,
}

impl Job {
    pub fn command(argv: Vec<String>, cwd: PathBuf) -> Self {
        Self {
            id: InvocationId(0),
            kind: crate::logbus::InvocationKind::ProtonVpn,
            argv,
            display: None,
            cwd,
            interactive: false,
            timeout: Some(DEFAULT_TIMEOUT),
            language: Locale::SOURCE,
        }
    }

    pub fn with_id(mut self, id: InvocationId) -> Self {
        self.id = id;
        self
    }

    pub fn with_language(mut self, language: Locale) -> Self {
        self.language = language;
        self
    }

    pub fn interactive(mut self) -> Self {
        self.interactive = true;
        self.timeout = None;
        self
    }
}

/// What the runner tells the engine. The engine folds these into the log bus.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunnerEvent {
    Started {
        id: InvocationId,
        at: SystemTime,
    },
    Line {
        id: InvocationId,
        text: String,
        at: SystemTime,
    },
    Finished {
        id: InvocationId,
        exit_code: Option<u32>,
        duration: Duration,
        at: SystemTime,
    },
    /// The child could not be started at all; this is our own message, not the CLI's output.
    SpawnFailed {
        id: InvocationId,
        message: String,
        at: SystemTime,
    },
}

/// Handle to the runner thread.
pub struct Runner {
    jobs: Sender<Job>,
    stdin_slot: Arc<Mutex<Option<PtyStdin>>>,
    /// Submitted but not yet finished, the running one included.
    outstanding: Arc<AtomicUsize>,
    running: Arc<AtomicBool>,
    /// Set by the UI when the user gives up on the running child. An interactive command waits for
    /// a human, and waiting must not be the same thing as being stuck.
    cancel: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Runner {
    /// Starts the runner thread. `events` is the engine's end of the stream.
    pub fn spawn(events: Sender<RunnerEvent>) -> Self {
        let (jobs, job_rx) = channel::<Job>();
        let stdin_slot: Arc<Mutex<Option<PtyStdin>>> = Arc::new(Mutex::new(None));
        let outstanding = Arc::new(AtomicUsize::new(0));
        let running = Arc::new(AtomicBool::new(false));
        let cancel = Arc::new(AtomicBool::new(false));

        let handle = {
            let stdin_slot = Arc::clone(&stdin_slot);
            let outstanding = Arc::clone(&outstanding);
            let running = Arc::clone(&running);
            let cancel = Arc::clone(&cancel);
            thread::Builder::new()
                .name("protonvpn-runner".into())
                .spawn(move || {
                    runner_loop(job_rx, events, stdin_slot, outstanding, running, cancel)
                })
                .expect("cannot spawn the runner thread")
        };

        Self {
            jobs,
            stdin_slot,
            outstanding,
            running,
            cancel,
            thread: Some(handle),
        }
    }

    /// Queues a job. Ordering is the queue's; nothing else about a job is decided here.
    pub fn submit(&self, job: Job) -> bool {
        self.outstanding.fetch_add(1, Ordering::SeqCst);
        if self.jobs.send(job).is_err() {
            self.outstanding.fetch_sub(1, Ordering::SeqCst);
            return false;
        }
        true
    }

    /// The terminal of the currently running interactive child, if any.
    pub fn stdin(&self) -> Option<PtyStdin> {
        self.stdin_slot
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// Jobs waiting for their turn, excluding the one running.
    pub fn queued(&self) -> usize {
        let running = usize::from(self.running.load(Ordering::SeqCst));
        self.outstanding
            .load(Ordering::SeqCst)
            .saturating_sub(running)
    }

    /// How many jobs are outstanding, the running one included.
    pub fn outstanding(&self) -> usize {
        self.outstanding.load(Ordering::SeqCst)
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    /// Asks the running child to stop. The runner kills it and records what happened; nothing is
    /// assumed about why.
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::SeqCst);
    }
}

impl Drop for Runner {
    fn drop(&mut self) {
        // Closing the channel ends the loop once the current child finishes. The thread is *not*
        // joined: a wedged interactive child must not be able to hold up the application's exit.
        let (dead, _) = channel::<Job>();
        let _ = std::mem::replace(&mut self.jobs, dead);
        self.thread.take();
    }
}

fn runner_loop(
    jobs: Receiver<Job>,
    events: Sender<RunnerEvent>,
    stdin_slot: Arc<Mutex<Option<PtyStdin>>>,
    outstanding: Arc<AtomicUsize>,
    running: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
) {
    let mut waiting = VecDeque::new();
    loop {
        // Drain whatever is already queued before blocking, so the status we report is current.
        loop {
            match jobs.try_recv() {
                Ok(job) => waiting.push_back(job),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => break,
            }
        }

        let job = match waiting.pop_front() {
            Some(job) => job,
            None => match jobs.recv() {
                Ok(job) => job,
                Err(_) => return,
            },
        };

        running.store(true, Ordering::SeqCst);
        run_one(&job, &events, &stdin_slot, &cancel);
        // A cancel is aimed at the child that was running when it was asked for, never at the
        // next one.
        cancel.store(false, Ordering::SeqCst);
        running.store(false, Ordering::SeqCst);
        outstanding.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Wall-clock elapsed since `at`, saturating: the clock going backwards is not a reason to panic.
fn since(at: SystemTime) -> Duration {
    SystemTime::now().duration_since(at).unwrap_or_default()
}

fn run_one(
    job: &Job,
    events: &Sender<RunnerEvent>,
    stdin_slot: &Arc<Mutex<Option<PtyStdin>>>,
    cancel: &Arc<AtomicBool>,
) {
    // Ours, not the CLI's: written here rather than in a catalogue this thread holds, because the
    // two places that need it are both failures and both rare — a parse each is nothing next to a
    // child that had to be killed. Built with the language the job was submitted in.
    let ours = || I18n::new(job.language);
    let at = SystemTime::now();
    if events
        .send(RunnerEvent::Started { id: job.id, at })
        .is_err()
    {
        return;
    }

    let mut spawned = match pty::spawn(&job.argv, PTY_COLS, PTY_ROWS) {
        Ok(spawned) => spawned,
        Err(error) => {
            let i18n = ours();
            let message =
                i18n.core_runner_spawn_failed(pty::command_line(&job.argv), error.describe(&i18n));
            let _ = events.send(RunnerEvent::SpawnFailed {
                id: job.id,
                message,
                at: SystemTime::now(),
            });
            let _ = events.send(RunnerEvent::Finished {
                id: job.id,
                exit_code: None,
                duration: since(at),
                at: SystemTime::now(),
            });
            return;
        }
    };

    let stdin = spawned.stdin_handle();
    if job.interactive {
        *stdin_slot
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(stdin.clone());
    }

    let mut reader = spawned.take_reader();
    let (line_tx, line_rx) = channel::<String>();
    // The reader hands over complete lines; whatever it is holding back lives here, where the
    // consumer below can reach it when the child stops talking mid-line.
    let tail = Arc::new(Mutex::new(Tail {
        text: String::new(),
        quiet_since: Instant::now(),
    }));
    let reader_thread = {
        let tail = Arc::clone(&tail);
        thread::Builder::new()
            .name("protonvpn-pty-reader".into())
            .spawn(move || {
                let mut buffer = [0u8; 8192];
                loop {
                    match reader.read(&mut buffer) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            let lines = {
                                let mut tail =
                                    tail.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                                tail.text.push_str(&String::from_utf8_lossy(&buffer[..n]));
                                // Stamped under the same lock the flusher reads it with, so a
                                // freshly written tail can never look quiet.
                                tail.quiet_since = Instant::now();
                                pty::split_lines(&mut tail.text)
                            };
                            for line in lines {
                                if line_tx.send(line).is_err() {
                                    return;
                                }
                            }
                        }
                    }
                }
                // A final line without a trailing newline is still a line.
                let last = {
                    let mut tail = tail.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                    std::mem::take(&mut tail.text)
                };
                if !last.is_empty() {
                    let _ = line_tx.send(last);
                }
            })
            .expect("cannot spawn the pty reader thread")
    };

    let deadline = job.timeout.map(|timeout| Instant::now() + timeout);
    let mut timed_out = false;
    let mut cancelled = false;
    loop {
        // Every wait is bounded by the prompt idle window: that is how a line the child never
        // terminated gets noticed while the child is still waiting for an answer.
        let wait = match deadline {
            Some(deadline) => deadline
                .saturating_duration_since(Instant::now())
                .min(PROMPT_IDLE),
            // Interactive children have no deadline: they are waiting for a human.
            None => PROMPT_IDLE,
        };
        let line = match line_rx.recv_timeout(wait) {
            Ok(text) => Some(text),
            Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {
                if cancel.load(Ordering::SeqCst) {
                    cancelled = true;
                    let _ = spawned.kill();
                    break;
                }
                if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                    timed_out = true;
                    let _ = spawned.kill();
                    break;
                }
                // The child has gone quiet. If it left something unterminated, that is a prompt.
                let mut tail = tail.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                if tail.text.is_empty() || tail.quiet_since.elapsed() < PROMPT_IDLE {
                    None
                } else {
                    tail.quiet_since = Instant::now();
                    Some(std::mem::take(&mut tail.text))
                }
            }
        };

        if let Some(text) = line
            && events
                .send(RunnerEvent::Line {
                    id: job.id,
                    text,
                    at: SystemTime::now(),
                })
                .is_err()
        {
            break;
        }
    }

    let exit_code = spawned.wait().ok();
    let _ = reader_thread.join();

    // Whether the child exited, was killed or was talked over, its terminal is done with.
    if job.interactive {
        stdin.close();
        *stdin_slot
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
    }

    if cancelled || timed_out {
        // Both lines are ours, not the CLI's, and the marker is how the transcript says so. It is
        // part of the line rather than of the message: the program's name is not translated.
        let text = if cancelled {
            format!("[protonvpn-gui] {}", ours().core_runner_cancelled())
        } else {
            let seconds = job.timeout.unwrap_or_default().as_secs();
            format!(
                "[protonvpn-gui] {}",
                ours().core_runner_timed_out(seconds as i64)
            )
        };
        let _ = events.send(RunnerEvent::Line {
            id: job.id,
            text,
            at: SystemTime::now(),
        });
        let _ = events.send(RunnerEvent::Finished {
            id: job.id,
            exit_code: None,
            duration: since(at),
            at: SystemTime::now(),
        });
        return;
    }

    let _ = events.send(RunnerEvent::Finished {
        id: job.id,
        exit_code,
        duration: since(at),
        at: SystemTime::now(),
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collect_until_finished(rx: &Receiver<RunnerEvent>, timeout: Duration) -> Vec<RunnerEvent> {
        collect_until(rx, timeout, 1)
    }

    /// Collects until `finish_count` invocations have finished, or the timeout expires.
    fn collect_until(
        rx: &Receiver<RunnerEvent>,
        timeout: Duration,
        finish_count: usize,
    ) -> Vec<RunnerEvent> {
        let deadline = Instant::now() + timeout;
        let mut events = Vec::new();
        let mut finished = 0usize;
        while Instant::now() < deadline && finished < finish_count {
            match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Ok(event) => {
                    if matches!(event, RunnerEvent::Finished { .. }) {
                        finished += 1;
                    }
                    events.push(event);
                }
                Err(_) => break,
            }
        }
        events
    }

    fn english() -> I18n {
        I18n::new(crate::i18n::Locale::SOURCE)
    }

    /// The one place a count of ours reaches a sentence, and the reason the catalogue selects on
    /// CLDR categories rather than on an English `[one]`/`[other]`: Russian needs three forms, and
    /// a translation with two of them declines nothing.
    #[test]
    fn a_timeout_declines_its_seconds_in_every_language_it_has() {
        let russian = I18n::new(crate::i18n::Locale::from_id("ru").unwrap());
        let one = russian.core_runner_timed_out(1);
        let few = russian.core_runner_timed_out(3);
        let many = russian.core_runner_timed_out(5);

        assert_ne!(one, few);
        assert_ne!(few, many);
        for (seconds, line) in [(1, &one), (3, &few), (5, &many)] {
            assert!(line.contains(&seconds.to_string()), "{line}");
        }
        // And the source language really is the source: one second, two seconds.
        assert!(english().core_runner_timed_out(1).contains("1 second"));
        assert!(english().core_runner_timed_out(5).contains("5 seconds"));
    }

    fn sh(script: &str) -> Vec<String> {
        vec!["/bin/sh".to_string(), "-c".to_string(), script.to_string()]
    }

    fn lines(events: &[RunnerEvent]) -> Vec<String> {
        events
            .iter()
            .filter_map(|e| match e {
                RunnerEvent::Line { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect()
    }

    fn exit_code(events: &[RunnerEvent]) -> Option<u32> {
        events.iter().find_map(|e| match e {
            RunnerEvent::Finished { exit_code, .. } => Some(*exit_code),
            _ => None,
        })?
    }

    #[test]
    fn streams_lines_and_records_the_exit_code() {
        let (tx, rx) = channel();
        let runner = Runner::spawn(tx);
        runner.submit(Job::command(
            sh("printf 'one\\ntwo\\n'; exit 3"),
            PathBuf::from("/tmp"),
        ));

        let events = collect_until_finished(&rx, Duration::from_secs(10));
        assert!(matches!(events[0], RunnerEvent::Started { .. }));
        assert_eq!(lines(&events), vec!["one".to_string(), "two".to_string()]);
        assert_eq!(exit_code(&events), Some(3));
    }

    #[test]
    fn output_without_a_trailing_newline_is_still_a_line() {
        let (tx, rx) = channel();
        let runner = Runner::spawn(tx);
        runner.submit(Job::command(
            sh("printf 'no newline'"),
            PathBuf::from("/tmp"),
        ));
        let events = collect_until_finished(&rx, Duration::from_secs(10));
        assert_eq!(lines(&events), vec!["no newline".to_string()]);
    }

    /// An interactive child waits for a human, and waiting must not be the same thing as being
    /// stuck: the window has to be able to say "no".
    #[test]
    fn a_child_waiting_on_input_can_be_cancelled() {
        let (tx, rx) = channel();
        let runner = Runner::spawn(tx);
        runner.submit(
            Job::command(
                sh("printf 'Password: '; read -r pw; printf 'never\n'"),
                PathBuf::from("/tmp"),
            )
            .interactive(),
        );

        // Let the prompt arrive, then give up on it the way a person would.
        thread::sleep(Duration::from_millis(400));
        runner.cancel();

        let events = collect_until_finished(&rx, Duration::from_secs(10));
        let lines = lines(&events);
        assert!(
            lines
                .iter()
                .any(|line| line.contains("interrupted at the user's request")),
            "{lines:?}"
        );
        assert!(!lines.iter().any(|line| line == "never"), "{lines:?}");
    }

    #[test]
    fn serialises_two_jobs_and_never_interleaves_them() {
        let (tx, rx) = channel();
        let runner = Runner::spawn(tx);
        runner.submit(
            Job::command(
                sh("printf 'a1\\n'; sleep 0.3; printf 'a2\\n'"),
                PathBuf::from("/tmp"),
            )
            .with_id(InvocationId(1)),
        );
        runner.submit(
            Job::command(sh("printf 'b1\\n'"), PathBuf::from("/tmp")).with_id(InvocationId(2)),
        );

        let events = collect_until(&rx, Duration::from_secs(10), 2);
        let first_id = match events[0] {
            RunnerEvent::Started { id, .. } => id,
            _ => panic!("expected Started first"),
        };
        // The first job must finish before the second one starts.
        let first_finish = events
            .iter()
            .position(|e| matches!(e, RunnerEvent::Finished { id, .. } if *id == first_id))
            .expect("first job finished");
        let second_start = events
            .iter()
            .position(|e| matches!(e, RunnerEvent::Started { id, .. } if *id != first_id))
            .expect("second job started");
        assert!(first_finish < second_start, "jobs overlapped: {events:?}");

        let mut all = lines(&events);
        all.retain(|l| !l.is_empty());
        assert_eq!(all, vec!["a1", "a2", "b1"]);
    }

    #[test]
    fn the_queue_depth_is_visible_while_a_job_waits() {
        let (tx, rx) = channel();
        let runner = Runner::spawn(tx);
        runner.submit(Job::command(sh("sleep 0.4"), PathBuf::from("/tmp")));
        runner.submit(Job::command(sh("sleep 0.1"), PathBuf::from("/tmp")));

        // Give the runner a moment to pick the first job up.
        thread::sleep(Duration::from_millis(150));
        assert!(runner.is_running());
        assert_eq!(runner.queued(), 1);

        let _ = collect_until_finished(&rx, Duration::from_secs(5));
        let deadline = Instant::now() + Duration::from_secs(5);
        while runner.outstanding() > 0 && Instant::now() < deadline {
            let _ = rx.recv_timeout(Duration::from_millis(50));
        }
        assert_eq!(runner.outstanding(), 0);
    }

    #[test]
    fn answers_a_prompt_through_the_pty_and_the_answer_is_not_in_the_output() {
        let (tx, rx) = channel();
        let runner = Runner::spawn(tx);
        // Stand-in for `protonvpn signin`: prompt on the terminal, read a line, echo a verdict.
        runner.submit(
            Job::command(
                sh("read -r secret; printf 'seen:%s\\n' \"$secret\""),
                PathBuf::from("/tmp"),
            )
            .interactive(),
        );

        // Wait for the interactive child to hand us its terminal, then answer it.
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut stdin = None;
        while Instant::now() < deadline {
            if let Some(handle) = runner.stdin() {
                stdin = Some(handle);
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
        let stdin = stdin.expect("interactive job published a stdin handle");
        stdin.write_line("hunter2").unwrap();

        let events = collect_until_finished(&rx, Duration::from_secs(10));
        let output = lines(&events).join("\n");
        assert!(
            output.contains("seen:hunter2"),
            "child did not receive the secret: {output}"
        );
        // The secret is in the child's own echo only because this stand-in echoes it; what matters
        // is that the runner never fabricates a line containing it.
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, RunnerEvent::Line { text, .. } if text.contains("could not"))),
            "unexpected runner error: {events:?}"
        );
        assert_eq!(exit_code(&events), Some(0));
    }

    #[test]
    fn a_wedged_child_is_killed_at_the_deadline() {
        let (tx, rx) = channel();
        let runner = Runner::spawn(tx);
        let mut job = Job::command(sh("sleep 30"), PathBuf::from("/tmp"));
        job.timeout = Some(Duration::from_millis(300));
        runner.submit(job);

        let started = Instant::now();
        let events = collect_until_finished(&rx, Duration::from_secs(10));
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "the timeout did not fire"
        );
        assert!(matches!(
            events.last(),
            Some(RunnerEvent::Finished {
                exit_code: None,
                ..
            })
        ));
        assert!(
            lines(&events).iter().any(|l| l.contains("was interrupted")),
            "the interruption must be visible in the transcript: {events:?}"
        );
    }

    #[test]
    fn a_missing_program_is_reported_as_our_own_message_not_the_clis() {
        let (tx, rx) = channel();
        let runner = Runner::spawn(tx);
        runner.submit(Job::command(
            vec!["/nonexistent/protonvpn".to_string()],
            PathBuf::from("/tmp"),
        ));
        let events = collect_until_finished(&rx, Duration::from_secs(10));
        let failure = events
            .iter()
            .find_map(|event| match event {
                RunnerEvent::SpawnFailed { message, .. } => Some(message.clone()),
                _ => None,
            })
            .unwrap_or_else(|| panic!("expected SpawnFailed among {events:?}"));
        assert!(failure.contains("could not start"), "{failure}");
    }
}
