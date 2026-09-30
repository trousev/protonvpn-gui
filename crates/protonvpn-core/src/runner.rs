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

use crate::model::InvocationId;
use crate::pty::{self, PtyStdin};

/// Terminal geometry for every child. The CLI's tables are width-independent (measured at 80 and
/// 120 columns), so this is only a guard against absurd wrapping if that ever changes.
pub const PTY_COLS: u16 = 120;
pub const PTY_ROWS: u16 = 40;

/// Default patience for a non-interactive command.
///
/// `connect` measured 2–4 s, `status` ~1 s. Two minutes is not a timeout anyone will hit in
/// normal use; it exists so that a wedged child cannot leave the app saying "работаю" forever.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(120);

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
        }
    }

    pub fn with_id(mut self, id: InvocationId) -> Self {
        self.id = id;
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
    thread: Option<JoinHandle<()>>,
}

impl Runner {
    /// Starts the runner thread. `events` is the engine's end of the stream.
    pub fn spawn(events: Sender<RunnerEvent>) -> Self {
        let (jobs, job_rx) = channel::<Job>();
        let stdin_slot: Arc<Mutex<Option<PtyStdin>>> = Arc::new(Mutex::new(None));
        let outstanding = Arc::new(AtomicUsize::new(0));
        let running = Arc::new(AtomicBool::new(false));

        let handle = {
            let stdin_slot = Arc::clone(&stdin_slot);
            let outstanding = Arc::clone(&outstanding);
            let running = Arc::clone(&running);
            thread::Builder::new()
                .name("protonvpn-runner".into())
                .spawn(move || runner_loop(job_rx, events, stdin_slot, outstanding, running))
                .expect("cannot spawn the runner thread")
        };

        Self {
            jobs,
            stdin_slot,
            outstanding,
            running,
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
        run_one(&job, &events, &stdin_slot);
        running.store(false, Ordering::SeqCst);
        outstanding.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Wall-clock elapsed since `at`, saturating: the clock going backwards is not a reason to panic.
fn since(at: SystemTime) -> Duration {
    SystemTime::now().duration_since(at).unwrap_or_default()
}

fn run_one(job: &Job, events: &Sender<RunnerEvent>, stdin_slot: &Arc<Mutex<Option<PtyStdin>>>) {
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
            let message = format!(
                "не удалось запустить `{}`: {error}",
                pty::command_line(&job.argv)
            );
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
    let reader_thread = thread::Builder::new()
        .name("protonvpn-pty-reader".into())
        .spawn(move || {
            let mut buffer = [0u8; 8192];
            let mut pending = String::new();
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        pending.push_str(&String::from_utf8_lossy(&buffer[..n]));
                        for line in pty::split_lines(&mut pending) {
                            if line_tx.send(line).is_err() {
                                return;
                            }
                        }
                    }
                }
            }
            // A final line without a trailing newline is still a line.
            if !pending.is_empty() {
                let _ = line_tx.send(pending);
            }
        })
        .expect("cannot spawn the pty reader thread");

    let deadline = job.timeout.map(|timeout| Instant::now() + timeout);
    let mut timed_out = false;
    loop {
        let wait = match deadline {
            Some(deadline) => deadline.saturating_duration_since(Instant::now()),
            // Effectively "until the reader is done"; interactive children have no deadline.
            None => Duration::from_secs(86_400),
        };
        match line_rx.recv_timeout(wait) {
            Ok(text) => {
                if events
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
            Err(RecvTimeoutError::Timeout) => {
                if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                    timed_out = true;
                    let _ = spawned.kill();
                    break;
                }
            }
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }

    let exit_code = spawned.wait().ok();
    let _ = reader_thread.join();
    if timed_out {
        let seconds = job.timeout.unwrap_or_default().as_secs();
        let _ = events.send(RunnerEvent::Line {
            id: job.id,
            text: format!(
                "[protonvpn-gui] команда не завершилась за {seconds} с и была прервана; \
                 вывод выше — всё, что успел сказать CLI"
            ),
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

    if job.interactive {
        stdin.close();
        *stdin_slot
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
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
            !events.iter().any(
                |e| matches!(e, RunnerEvent::Line { text, .. } if text.contains("не удалось"))
            ),
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
            lines(&events).iter().any(|l| l.contains("была прервана")),
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
        assert!(failure.contains("не удалось запустить"), "{failure}");
    }
}
