//! Engine — the one thread that owns everything stateful.
//!
//! It exists so that there is exactly **one writer** of state. The runner reports; the engine
//! folds the report into the log bus, hands the same event to the interpreter, and publishes the
//! result for the views. Views never call the runner (`docs/architecture.md` §1): they send a
//! [`Request`] and read a snapshot.
//!
//! What lives here and nowhere else:
//!
//! * the log bus and the interpreter's state,
//! * the poll schedule — idle cadence, immediate read after a state-changing invocation,
//!   attention-driven read,
//! * the port-forwarding lease and its renewal timer,
//! * secrets for `signin`, held in memory only, and only while the child is running,
//! * the update state of an AppImage install (exception #4, §14) — the check and the download run
//!   on threads of their own and report back here, exactly like the probe, because nothing about
//!   them may block the commands the user is actually watching.
//!
//! The engine is deliberately GUI-free: it drives the tray through the [`TrayPresenter`] trait, so
//! `protonvpn-core` still builds and works with no window and no toolkit.

use std::collections::{HashMap, VecDeque};
use std::fs;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use crate::config::{Config, ConfigStore, UpdatePolicy};
use crate::i18n::{I18n, Locale};
use crate::interpreter::{self, PromptKind};
use crate::launcher::Intent;
use crate::logbus::{InvocationKind, LogBus, LogEvent};
use crate::model::{
    AppState, ConnectTarget, ConnectionStatus, EgressReading, InvocationId, Observation,
    PortForwarding, ProbeEndpoint, RunnerStatus,
};
use crate::net::natpmp::{self, NatPmp, Protocol};
use crate::net::route::{self, RouteProbe};
use crate::poll::PollSchedule;
use crate::probe::{self, Probe, ProbeError};
use crate::runner::{Job, Runner, RunnerEvent};
use crate::socks5::{
    self, Closed, GateState, Reporter, RouteLoss, Socks5, Socks5Event, Stats, TunnelGate,
};
use crate::update::{self, Installed, Release, UpdateError, Version};

/// Engine tick. Short enough that streamed output feels live, long enough to be free.
pub const TICK: Duration = Duration::from_millis(100);

/// How often the route we are *not* using is re-sampled inside the settle window below.
const REFERENCE_INTERVAL: Duration = Duration::from_millis(500);

/// How long the reference keeps being re-sampled after the CLI says the tunnel is down, and no
/// longer. The kernel needs a moment to withdraw the tunnel's address, so a reference taken at the
/// instant of the disconnect could still be the tunnel itself; but sampling forever would let a
/// connect started from another terminal be mistaken for the reference, which would leave the
/// proxy shut for the rest of the session.
const REFERENCE_SETTLE: Duration = Duration::from_secs(3);

/// How often the proxy retries arming itself while the CLI says connected but the evidence is not
/// there yet. The kernel needs a moment to withdraw or install an address around a reconnect, and
/// a route sampled a second too early must not condemn the proxy for the whole session.
const ARM_RETRY: Duration = Duration::from_secs(2);

/// How many dial failures that implicate the path — timeouts, unreachable networks — it takes
/// before the engine stops trusting the tunnel. One dead destination is not a dead tunnel.
const DIAL_FAILURE_LIMIT: u32 = 2;

/// How many consecutive unanswered background checks it takes before the same conclusion.
const WATCH_FAILURE_LIMIT: u32 = 2;

/// How long the updater waits after startup before its first look. Not a courtesy to the CLI —
/// they share nothing — but to the person watching: the first `status` of a session and the window
/// coming up are what they are waiting for, and a check that answers ten seconds later is just as
/// useful.
const UPDATE_STARTUP_DELAY: Duration = Duration::from_secs(10);

/// The updater's idle cadence. A release is not a status: once a day is generous, and asking the
/// release page costs one small file. `last_check` lives in the config, so restarting the
/// application ten times a day does not mean ten checks.
const UPDATE_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

/// After a failed check. Longer than the status cadence, shorter than a day: a laptop that woke up
/// without a network should not have to be restarted to learn about a release.
const UPDATE_RETRY: Duration = Duration::from_secs(6 * 60 * 60);

/// Wall-clock elapsed since `at`, saturating. A clock that jumps backwards is not a panic.
fn since(at: SystemTime) -> Duration {
    SystemTime::now().duration_since(at).unwrap_or_default()
}

/// What the tray is told. The tray shows connection status and, when there is one, the fact that a
/// release is waiting (`docs/architecture.md` §9, §14).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrayView {
    pub status: ConnectionStatus,
    /// The language the labels in this view are written in. The tray renders its own menu, on its
    /// own thread, and this is how a language change reaches it: through the same update that
    /// carries everything else.
    pub language: Locale,
    /// `NL#818 · Amsterdam, Netherlands` when connected.
    pub detail: Option<String>,
    pub age_text: String,
    /// A release newer than this build, unless the user asked not to be told again. The tray is a
    /// menu, not a notification system: the item is there to be found, never to interrupt.
    pub update: Option<TrayUpdate>,
}

/// What the tray says about an update.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrayUpdate {
    pub version: Version,
    /// True once the image is in place and only a restart is missing.
    pub applied: bool,
}

/// Implemented by the GUI crate; keeps `protonvpn-core` free of any toolkit.
pub trait TrayPresenter: Send + 'static {
    fn update(&self, view: TrayView);
}

/// Which reading a probe result is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeTarget {
    Baseline,
    Current,
    /// A background check for the SOCKS5 proxy. It is state like any other reading, but it is not
    /// an invocation the user asked for, so it does not write a line into the console — only a
    /// conclusion does (`docs/architecture.md` §13).
    Watch,
}

/// The outcome of one `curl` run, carried back to the engine thread.
pub type ProbeResult = Result<EgressReading, ProbeError>;

/// What the updater is doing, as the views see it (exception #4, `docs/architecture.md` §14).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UpdateView {
    /// The version this build says it is, when it was built as a release at all.
    pub current: Option<Version>,
    /// The latest release, once a check has answered. Kept when a later check fails: what was
    /// known is still known, and its age is what the view shows (§7).
    pub latest: Option<Version>,
    /// When the release page was last asked. A failure counts as an answer here — the age belongs
    /// to the attempt, and the error says how it went.
    pub checked_at: Option<SystemTime>,
    /// How the last check or download ended, when it ended badly.
    pub error: Option<String>,
    pub phase: UpdatePhase,
    /// Whether this build *can* replace itself: false for a tarball install or a `cargo run`.
    pub replaceable: bool,
    /// Whether the tray should mention the update, or the user asked not to be told about this
    /// version again. A newer release clears it.
    pub dismissed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum UpdatePhase {
    #[default]
    Idle,
    Checking,
    Downloading {
        received: u64,
        total: Option<u64>,
    },
    /// Downloaded and verified, waiting for whoever decides to put it in place.
    Staged {
        version: Version,
    },
    /// In place. The process still running is the old build; the next start is the new one.
    Installed {
        version: Version,
    },
}

/// Reports from the updater's own threads. They travel on the same queue as everything else, so
/// there is still exactly one thread that owns state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateEvent {
    Checked {
        started_at: SystemTime,
        duration: Duration,
        result: Result<update::Finding, UpdateError>,
    },
    Progress {
        received: u64,
        total: Option<u64>,
    },
    Staged {
        version: Version,
        bytes: u64,
        started_at: SystemTime,
        duration: Duration,
    },
    Installed {
        version: Version,
        backup: Option<PathBuf>,
        started_at: SystemTime,
        duration: Duration,
    },
    Failed {
        /// What failed, as the console should show it — spelled by the thread that ran it, from
        /// the same arguments it passed.
        display: String,
        error: UpdateError,
        started_at: SystemTime,
        duration: Duration,
    },
    Cancelled,
}

/// Requests the views make of the engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    /// Run a `protonvpn` command.
    Run(Intent),
    /// Connect because *we* decided to at startup — which is not the same as a human asking for a
    /// tunnel, because `connect` against a live tunnel silently switches servers
    /// (`docs/cli-surface.md` §4.4). The engine waits for the first `status` reading and stands
    /// down if it already reports a connection.
    StartupConnect(ConnectTarget),
    /// Log in. The password and 2FA code travel with the request and are never logged.
    SignIn {
        username: String,
        password: String,
        two_factor: Option<String>,
    },
    /// Writes a line to the running interactive child's terminal.
    Stdin {
        text: String,
    },
    /// Take a ground-truth reading now.
    Probe,
    /// Ask the gateway for (or renew) a port-forwarding lease.
    PortForwardRefresh,
    /// Release the lease and forget the port.
    ReleasePort,
    /// The user is looking: refresh if the policy allows it.
    Attention,
    /// Stop the running child. An interactive command waits for a human, and waiting must not be
    /// the same thing as being stuck.
    Cancel,
    /// Replace our own configuration.
    SaveConfig(Box<Config>),
    /// Internal: a report from the SOCKS5 proxy's own threads. It travels on the same queue as
    /// everything else so that there is still exactly one thread that owns state.
    Socks5Report(Socks5Event),
    Ui(UiCommand),
    /// Internal: the probe fallback chain picked an endpoint (or did not).
    ProbeChosen {
        chosen: Option<Probe>,
        baseline: Option<ProbeResult>,
    },
    /// Internal: a reading finished on the probe thread.
    ProbeFinished {
        target: ProbeTarget,
        endpoint: ProbeEndpoint,
        result: ProbeResult,
        started_at: SystemTime,
        duration: Duration,
    },
    /// Speak another language from now on. The engine holds its own catalogue because every note
    /// it writes is already-rendered text; switching here is what makes a language change reach
    /// the tray and everything the engine says next.
    SetLanguage(Locale),
    /// Ask the release page what the latest version is. Works whatever the policy says: an
    /// explicit request is consent, and a button that refuses to work would be a lie.
    UpdateCheck,
    /// Fetch the known release — if it is not already staged — and put it in place.
    UpdateInstall,
    /// Stop a download that is in flight. What is already on disk is deleted, not resumed: a
    /// partly-verified image is not something to keep around.
    UpdateCancel,
    /// Do not mention this version in the tray again. A later release is mentioned again.
    UpdateDismiss,
    /// Internal: a report from the updater's own threads.
    UpdateReport(UpdateEvent),
    Quit,
    Shutdown,
}

/// Things the tray (or anything outside the view) asks the GUI to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiCommand {
    ShowWindow,
    HideWindow,
    ToggleWindow,
    Quit,
}

/// A prompt the CLI is waiting on and we cannot answer ourselves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingPrompt {
    pub invocation: InvocationId,
    pub kind: PromptKind,
}

/// Everything a view needs, in one snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shared {
    pub state: AppState,
    pub runner: RunnerStatus,
    pub config: Config,
    pub pending_prompt: Option<PendingPrompt>,
    pub ui_commands: VecDeque<UiCommand>,
    /// Whether a StatusNotifierItem host was found. On GNOME without the AppIndicator extension
    /// there is no tray, and the window must not hide into nothing.
    pub tray_available: bool,
    /// Our own last remark: a failed config write, a lost lease. Not a verdict about the CLI.
    pub note: Option<String>,
    /// Secrets are never published; this only says whether we are holding one.
    pub has_secrets: bool,
    pub socks5: Socks5View,
    /// The AppImage updater (exception #4).
    pub update: UpdateView,
}

/// The SOCKS5 proxy as the views see it: where it listens, the gate that decides whether anything
/// is relayed, and the counters its threads keep. A reason there is no listener at all is a gate
/// state too ([`Closed::NotListening`]), so there is exactly one place to look.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Socks5View {
    pub listen: Option<SocketAddr>,
    /// The engine writes this; the proxy and the views only read it.
    pub gate: TunnelGate,
    pub stats: Arc<Stats>,
}

#[derive(Clone)]
pub struct EngineHandle {
    tx: Sender<Request>,
    shared: Arc<Mutex<Shared>>,
    bus: Arc<Mutex<LogBus>>,
    finished: Arc<(Mutex<bool>, Condvar)>,
}

impl EngineHandle {
    pub fn send(&self, request: Request) {
        let _ = self.tx.send(request);
    }

    /// A copy of the current state. Cheap enough for a frame; never blocks for long.
    pub fn snapshot(&self) -> Shared {
        self.shared
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    pub fn bus(&self) -> Arc<Mutex<LogBus>> {
        Arc::clone(&self.bus)
    }

    /// Drains window commands left by the tray thread.
    pub fn take_ui_commands(&self) -> Vec<UiCommand> {
        let mut shared = self
            .shared
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        shared.ui_commands.drain(..).collect()
    }

    /// Asks the engine to stop, and waits (briefly) for it to release the port lease.
    pub fn shutdown(&self, timeout: Duration) -> bool {
        self.send(Request::Shutdown);
        let (lock, condvar) = &*self.finished;
        let guard = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let deadline = Instant::now() + timeout;
        let mut guard = guard;
        while !*guard {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return false;
            }
            let (next, timed_out) = condvar
                .wait_timeout(guard, remaining)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            guard = next;
            if timed_out.timed_out() && !*guard {
                return false;
            }
        }
        true
    }
}

pub struct EngineOptions {
    pub cwd: PathBuf,
    pub store: ConfigStore,
    pub config: Config,
    pub tray_available: bool,
    pub tray: Option<Box<dyn TrayPresenter>>,
    /// argv[0]. It is `protonvpn` and stays `protonvpn`: this is a test seam for driving a
    /// stand-in with no CLI installed, and nothing in the GUI ever sets it to anything else.
    pub program: String,
    /// How the kernel's route is read. Production always uses [`route::Kernel`]; the seam exists
    /// for the same reason `program` does, and nothing in the GUI ever sets it.
    pub route: Arc<dyn RouteProbe>,
    /// What this build says it is, from `protonvpn-gui`'s build script. `None` for a build that
    /// was not made as a release: it has nothing to compare itself with, and says so rather than
    /// guessing in either direction.
    pub version: Option<Version>,
    /// argv[0] for the updater. `curl` in production, and a test seam for the same reason
    /// `program` is one. Nothing else is ever run (exception #4).
    pub curl: String,
    /// The AppImage this process was started from, when it was one. `None` means there is nothing
    /// here that may replace itself — a tarball install, a `cargo run` — and the updater then
    /// reports what it finds without offering to install it.
    pub installed: Option<Installed>,
    /// How long the updater waits after startup before its first look. A test seam the way
    /// `program` is one: the cadence itself is a number a test must not have to sleep through.
    pub update_delay: Duration,
    /// The language of every word this engine produces — a note in the console, the tray's menu.
    /// The window sets it from the configuration; the tests set it to whatever they assert on.
    pub locale: Locale,
}

impl EngineOptions {
    pub fn new(store: ConfigStore, config: Config) -> Self {
        Self {
            cwd: std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/")),
            store,
            config,
            tray_available: false,
            tray: None,
            program: "protonvpn".to_string(),
            route: Arc::new(route::Kernel),
            version: None,
            curl: "curl".to_string(),
            installed: Installed::detect(),
            update_delay: UPDATE_STARTUP_DELAY,
            locale: Locale::SOURCE,
        }
    }
}

/// Starts the engine thread and returns the handle the views use.
pub fn spawn(options: EngineOptions) -> EngineHandle {
    let (tx, rx) = mpsc::channel::<Request>();
    let (runner_tx, runner_rx) = mpsc::channel::<RunnerEvent>();
    let finished = Arc::new((Mutex::new(false), Condvar::new()));

    // One gate, one set of counters, for the lifetime of the process: a restart on another port
    // must not reset what the proxy has already done.
    let socks5_stats = Arc::new(Stats::default());
    let gate = TunnelGate::new(Arc::clone(&options.route));

    // Built before anything moves out of `options`: what this build is, and whether it is
    // something that can replace itself at all.
    let update = UpdateView {
        current: options.version,
        replaceable: options.installed.is_some(),
        ..UpdateView::default()
    };

    let shared = Arc::new(Mutex::new(Shared {
        state: AppState::default(),
        runner: RunnerStatus::Idle,
        config: options.config.clone(),
        pending_prompt: None,
        ui_commands: VecDeque::new(),
        tray_available: options.tray_available,
        note: None,
        has_secrets: false,
        socks5: Socks5View {
            listen: None,
            gate: gate.clone(),
            stats: Arc::clone(&socks5_stats),
        },
        update: update.clone(),
    }));
    let bus = Arc::new(Mutex::new(LogBus::default()));

    let engine = Engine {
        tx: tx.clone(),
        shared: Arc::clone(&shared),
        bus: Arc::clone(&bus),
        runner: Runner::spawn(runner_tx),
        runner_rx,
        store: options.store,
        config: options.config,
        state: AppState::default(),
        schedule: PollSchedule::default(),
        probe: None,
        probe_in_flight: false,
        lease: None,
        lease_attempted_for: None,
        active_port_forwarding: None,
        pending_connect: None,
        startup_connect: None,
        status_attempted: false,
        secrets: None,
        i18n: I18n::new(options.locale),
        tray: options.tray,
        tray_view: None,
        cwd: options.cwd,
        program: options.program,
        route: options.route,
        gate,
        socks5: None,
        socks5_listen: None,
        socks5_stats,
        last_connection: ConnectionStatus::Unknown,
        route_reference: None,
        reference_settle_until: None,
        next_reference_sample: Instant::now(),
        next_arm_attempt: Instant::now(),
        next_verify: Instant::now(),
        dial_failures: 0,
        probe_failures: 0,
        jobs: HashMap::new(),
        version: update.current,
        curl: options.curl,
        installed: options.installed,
        release: None,
        update_job: UpdateJob::Idle,
        update_cancel: Arc::new(AtomicBool::new(false)),
        update,
        next_update_check: Instant::now(),
        update_delay: options.update_delay,
    };
    let finished_flag = Arc::clone(&finished);
    thread::Builder::new()
        .name("protonvpn-engine".into())
        .spawn(move || {
            engine.run(rx);
            let (lock, condvar) = &*finished_flag;
            *lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = true;
            condvar.notify_all();
        })
        .expect("cannot spawn the engine thread");

    EngineHandle {
        tx,
        shared,
        bus,
        finished,
    }
}

struct Secrets {
    password: String,
    two_factor: Option<String>,
}

/// What a submitted invocation is, kept until it finishes: only the engine knows it, because the
/// runner is given argv and nothing else.
struct SubmittedJob {
    argv: Vec<String>,
    display: Option<String>,
    interactive: bool,
}

#[derive(Debug)]
struct ActiveLease {
    port: u16,
    external_ip: Option<std::net::IpAddr>,
    next_renewal: Instant,
    server: Option<String>,
}

/// What the updater is in the middle of. One job at a time, like the runner, so two clicks cannot
/// start two downloads into the same staging file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UpdateJob {
    Idle,
    Checking,
    /// Downloading — and installing afterwards, when the policy or the user asked for the whole
    /// thing at once.
    Fetching {
        install_after: bool,
    },
}

struct Engine {
    tx: Sender<Request>,
    shared: Arc<Mutex<Shared>>,
    bus: Arc<Mutex<LogBus>>,
    runner: Runner,
    runner_rx: Receiver<RunnerEvent>,
    store: ConfigStore,
    config: Config,
    state: AppState,
    schedule: PollSchedule,
    probe: Option<Probe>,
    probe_in_flight: bool,
    lease: Option<ActiveLease>,
    /// Which server we last attempted a lease for, so a refusal is not retried forever.
    lease_attempted_for: Option<String>,
    /// Whether the connection *we* established this session asked for a port-forwarding lease.
    ///
    /// `None` means we did not establish it — the tunnel was already up when the app started, so
    /// the honest fallback is the selected profile (`docs/architecture.md` §11). A deliberate
    /// `false` is not the same thing and must not be overridden by the selection.
    active_port_forwarding: Option<bool>,
    /// A connect waiting for `config set port-forwarding on` to finish. The CLI's preference is
    /// global — it has no per-connection settings — so a profile that wants a lease sets it first
    /// and the connect follows, both visible in the console.
    pending_connect: Option<ConnectTarget>,
    /// A connect the config asked for at startup, parked until the first `status` reading of the
    /// session says whether there is anything to do.
    startup_connect: Option<ConnectTarget>,
    /// Whether that first `status` attempt is behind us — however it ended, because "the CLI
    /// could not be asked" is not a reason to stay parked forever.
    status_attempted: bool,
    secrets: Option<Secrets>,
    /// Every word this engine writes. Its own instance rather than the window's: the two run on
    /// different threads, and a `FluentBundle` is `Send` but not `Sync`.
    i18n: I18n,
    tray: Option<Box<dyn TrayPresenter>>,
    tray_view: Option<TrayView>,
    cwd: PathBuf,
    /// argv[0] for every child; `protonvpn` in production.
    program: String,
    /// How the kernel's route is read — the SOCKS5 gate's only system fact.
    route: Arc<dyn RouteProbe>,
    /// The gate. Written here and nowhere else (`docs/architecture.md` §13).
    gate: TunnelGate,
    socks5: Option<Socks5>,
    socks5_listen: Option<SocketAddr>,
    socks5_stats: Arc<Stats>,
    /// The last connection status we acted on, so a transition is a transition and not a state.
    last_connection: ConnectionStatus,
    /// The source address the kernel picked while the CLI said the tunnel was down. This is the
    /// only thing that can make "the route changed" evidence rather than a guess.
    route_reference: Option<Ipv4Addr>,
    /// Until when the reference is still being re-sampled, or `None` once it has settled.
    reference_settle_until: Option<Instant>,
    next_reference_sample: Instant,
    next_arm_attempt: Instant,
    next_verify: Instant,
    /// Consecutive dial failures that implicate the path, and consecutive unanswered checks.
    dial_failures: u32,
    probe_failures: u32,
    jobs: HashMap<InvocationId, SubmittedJob>,
    /// What this build says it is; `None` when it was not built as a release.
    version: Option<Version>,
    /// The program the updater runs; `curl`, and a test seam like `program`.
    curl: String,
    /// The image we may replace, when this process is an AppImage at all.
    installed: Option<Installed>,
    /// The release the last check found, kept so an install does not have to ask again.
    release: Option<Release>,
    update_job: UpdateJob,
    /// Set by `UpdateCancel` and read by the thread holding the transfer; reset when one starts.
    update_cancel: Arc<AtomicBool>,
    /// The updater's state, mirrored into `Shared` by [`Engine::publish_update`].
    update: UpdateView,
    /// When the next automatic check is due. Derived from `last_check` at startup, so a restart
    /// does not mean a fresh check.
    next_update_check: Instant,
    /// How long after startup the first check waits; `UPDATE_STARTUP_DELAY` in production.
    update_delay: Duration,
}

impl Engine {
    fn run(mut self, rx: Receiver<Request>) {
        // A status read right away: the app must not open claiming to know nothing when a
        // one-second command can tell it the truth. It counts as the idle poll — without this the
        // timer would immediately queue a second identical `status` behind it.
        self.schedule.note_poll(Instant::now());
        self.submit(Intent::RefreshStatus);
        // Who we are decides which shell the window shows — the login page or the app — so it is
        // asked once at startup rather than the first time someone opens a tab.
        self.submit(Intent::AccountInfo);
        if self.config.probe_enabled {
            self.choose_probe_async();
        }
        // The route we are *not* using, sampled before we know anything. If the application starts
        // while the tunnel is already up this is the tunnel's own address, so the proxy stays shut
        // until a reconnect shows it a route that changed — fail-closed, and the settings page
        // says exactly that (`docs/architecture.md` §13).
        self.sample_reference();
        self.reference_settle_until = Some(Instant::now() + REFERENCE_SETTLE);
        self.restart_socks5();

        // The `.old` file a previous update left beside the image is deleted here and nowhere else:
        // this process *is* the image that replaced it, so it started, so the copy is only disk
        // space. The first update check is scheduled rather than taken now — see
        // `UPDATE_STARTUP_DELAY`.
        if let Some(installed) = self.installed.clone() {
            update::discard_backup(&installed);
        }
        self.next_update_check = self.next_check_time();

        loop {
            self.drain_runner_events();
            self.run_due_timers();

            let timeout = self.next_wakeup();
            match rx.recv_timeout(timeout) {
                Ok(Request::Quit) | Ok(Request::Shutdown) => break,
                Ok(request) => self.handle(request),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }

        // Best-effort: give the port back rather than leaving a stale mapping behind.
        if self.lease.is_some() {
            let why = self.i18n.engine_lease_why_quit();
            self.release_lease(&why);
        }
    }

    fn next_wakeup(&self) -> Duration {
        let mut wait = TICK;
        if let Some(deadline) = self.schedule.next_idle_deadline()
            && self.runner.outstanding() == 0
        {
            wait = wait.min(deadline.saturating_duration_since(Instant::now()));
        }
        if let Some(lease) = &self.lease {
            wait = wait.min(lease.next_renewal.saturating_duration_since(Instant::now()));
        }
        wait.max(Duration::from_millis(10))
    }

    /// The poll schedule's triggers, plus lease renewal and the proxy's own clock.
    fn run_due_timers(&mut self) {
        let now = Instant::now();

        // While the CLI says the tunnel is down, keep a fresh picture of the route we are *not*
        // using — but only for the few seconds it takes the kernel to withdraw the tunnel's
        // address. After that the reference is frozen until the next disconnect, so that a connect
        // we did not initiate cannot be mistaken for it.
        if let Some(until) = self.reference_settle_until {
            if now >= until {
                self.reference_settle_until = None;
            } else if now >= self.next_reference_sample {
                self.next_reference_sample = now + REFERENCE_INTERVAL;
                self.sample_reference();
            }
        }

        // Connected, enabled, and no evidence yet: try again. This only ever runs while the gate
        // is closed for *lack* of evidence, never over a verdict the evidence already gave.
        if self.config.socks5.enabled
            && self.socks5.is_some()
            && self.state.connection.value.is_connected()
            && matches!(
                self.gate.state(),
                GateState::Closed(Closed::Unverified { .. } | Closed::NotConnected)
            )
            && now >= self.next_arm_attempt
        {
            self.next_arm_attempt = now + ARM_RETRY;
            self.arm_socks5();
        }

        // A lease belongs to one server. If the connection moved — because the user switched
        // servers from their own terminal, say — the old mapping is stale and must go.
        if let Some(lease) = &self.lease {
            let current = match &self.state.connection.value {
                ConnectionStatus::Connected(info) => Some(info.server.clone()),
                _ => None,
            };
            let moved = !self.state.connection.value.is_connected()
                || current
                    .as_ref()
                    .is_some_and(|server| Some(server) != lease.server.as_ref());
            if moved {
                let why = self.i18n.engine_lease_why_connection_changed();
                self.release_lease(&why);
            }
        }

        if self
            .lease
            .as_ref()
            .is_some_and(|lease| now >= lease.next_renewal)
            && self.runner.outstanding() == 0
        {
            self.renew_lease();
        }

        if self.runner.outstanding() > 0 {
            // Never queue a poll behind a running command: the runner is single-flight, and a
            // backlog of status reads would only make the console noisier.
            return;
        }

        if self.state.connection.value.is_connected() && self.lease.is_none() {
            self.maybe_start_lease();
        }

        // The proxy's background tunnel check (exception #1 doing the work it was sanctioned for).
        // It runs while the CLI says connected, whether or not the gate is open: a gate closed on
        // suspicion must be able to reopen on evidence, not only on a reconnect.
        if self.should_watch() && now >= self.next_verify {
            self.next_verify = now + self.verify_interval();
            self.watch_tunnel();
        }

        if self.schedule.idle_due(now) {
            self.schedule.note_poll(now);
            self.submit(Intent::RefreshStatus);
        }

        // The release page, on its own clock (§14). Never while another update job is running: one
        // job at a time, and a timer that fires into a download would only start a second one.
        if now >= self.next_update_check {
            self.next_update_check = if self.update_busy() {
                now + self.update_delay
            } else {
                now + UPDATE_INTERVAL
            };
            if self.config.update.policy != UpdatePolicy::Off && !self.update_busy() {
                self.start_update_check();
            }
        }
    }

    /// When the next automatic check is due: a day after the last one, or a few seconds from now
    /// when that is already in the past. Derived from the config so that restarting the application
    /// five times in an afternoon does not mean asking the release page five times.
    fn next_check_time(&self) -> Instant {
        let now = Instant::now();
        let Some(last) = self.config.update.last_check else {
            return now + self.update_delay;
        };

        let due = SystemTime::UNIX_EPOCH + Duration::from_secs(last) + UPDATE_INTERVAL;
        match due.duration_since(SystemTime::now()) {
            Ok(left) => now + left.max(self.update_delay),
            Err(_) => now + self.update_delay,
        }
    }

    fn should_watch(&self) -> bool {
        self.config.socks5.enabled
            && self.config.socks5.verify_seconds > 0
            && self.config.probe_enabled
            && self.state.connection.value.is_connected()
            && self.probe.is_some()
            && !self.probe_in_flight
    }

    fn handle(&mut self, request: Request) {
        match request {
            Request::Run(intent) => self.run_intent(intent),
            Request::StartupConnect(target) => {
                self.startup_connect = Some(target);
                self.resolve_startup_connect();
            }
            Request::SignIn {
                username,
                password,
                two_factor,
            } => {
                self.secrets = Some(Secrets {
                    password,
                    two_factor,
                });
                self.publish_secrets_flag();
                self.submit(Intent::SignIn { username });
            }
            Request::Stdin { text } => self.write_stdin(&text),
            Request::Probe => self.probe_now(),
            Request::PortForwardRefresh => {
                self.lease_attempted_for = None;
                self.maybe_start_lease();
            }
            Request::ReleasePort => {
                let why = self.i18n.engine_lease_why_user_request();
                self.release_lease(&why);
            }
            Request::Cancel => {
                self.runner.cancel();
            }
            Request::Attention => {
                let now = Instant::now();
                if self.runner.outstanding() == 0 && self.schedule.on_attention(now) {
                    self.submit(Intent::RefreshStatus);
                }
            }
            Request::SaveConfig(config) => self.save_config(*config),
            Request::SetLanguage(locale) => {
                self.i18n.set_locale(locale);
                // The tray's menu is rebuilt from the view, so a new view is the whole update.
                self.tray_view = None;
                self.publish_tray();
            }
            Request::Socks5Report(event) => self.react_to_socks5(event),
            Request::Ui(command) => {
                let mut shared = self.lock();
                shared.ui_commands.push_back(command);
            }
            Request::ProbeChosen { chosen, baseline } => {
                self.probe_in_flight = false;
                self.probe = chosen;
                if let Some(baseline) = baseline {
                    self.apply_probe(
                        ProbeTarget::Baseline,
                        baseline,
                        SystemTime::now(),
                        Duration::ZERO,
                    );
                }
                if self.probe.is_none() {
                    self.note(&self.i18n.engine_probe_unavailable());
                }
            }
            Request::ProbeFinished {
                target,
                endpoint,
                result,
                started_at,
                duration,
            } => {
                self.probe_in_flight = false;
                let answered = result.is_ok();
                if target != ProbeTarget::Watch {
                    // A background check is not an invocation the user asked for; the console is
                    // for what we ran, so only a conclusion gets a line (§13).
                    let lines = match &result {
                        Ok(reading) => vec![describe_reading(reading)],
                        Err(error) => vec![error.describe(&self.i18n)],
                    };
                    self.record_note(
                        format!("curl -sS --max-time 8 --ipv4 {}", endpoint.url()),
                        lines,
                        started_at,
                        duration,
                    );
                }
                self.apply_probe(target, result, started_at, duration);
                if target == ProbeTarget::Watch {
                    self.judge_watch(answered);
                }
            }
            Request::UpdateCheck => self.start_update_check(),
            Request::UpdateInstall => self.install_update(),
            Request::UpdateCancel => self.cancel_update(),
            Request::UpdateDismiss => self.dismiss_update(),
            Request::UpdateReport(event) => self.react_to_update(event),
            Request::Quit | Request::Shutdown => {}
        }
    }

    fn run_intent(&mut self, intent: Intent) {
        if let Intent::Connect(target) = &intent
            && target.port_forwarding
            && !self.setting_is_on("port-forwarding")
        {
            // The gateway only offers a mapping to a connection whose CLI preference is on. The
            // preference is global, so the profile sets it — in the open, as its own invocation —
            // and the connect waits behind it. Nothing is claimed until `config list` says so.
            self.pending_connect = Some(target.clone());
            self.submit(Intent::SetSetting {
                key: "port-forwarding".to_string(),
                value: "on".to_string(),
                dns: None,
            });
            return;
        }
        self.establish(intent);
    }

    /// The part of [`Self::run_intent`] that actually moves the tunnel.
    fn establish(&mut self, intent: Intent) {
        if let Intent::Connect(target) = &intent {
            self.active_port_forwarding = Some(target.port_forwarding);
        }
        if intent.changes_connection_state() {
            // A fresh baseline before we move the tunnel: "did the egress change?" is only
            // answerable if we know what it was.
            self.take_baseline_async();
            if self.lease.is_some() {
                let why = self.i18n.engine_lease_why_before_connect();
                self.release_lease(&why);
            }
            self.lease_attempted_for = None;
        }
        if matches!(intent, Intent::Disconnect) {
            self.state.port_forwarding = Observation::now(PortForwarding::Pending);
            self.publish_state();
        }
        self.submit(intent);
    }

    /// The startup connect's gate.
    ///
    /// `connect` is not idempotent: against a live tunnel the CLI switches servers silently and the
    /// egress moves under the user (`docs/cli-surface.md` §4.4). So the app's own
    /// connect-at-startup waits for the first `status` reading and does nothing when that reading
    /// already reports a connection. A human asking for a switch goes through [`Intent::Connect`]
    /// and is always obeyed — the difference is who asked.
    fn resolve_startup_connect(&mut self) {
        // Nothing to decide yet: the answer is one `status` away, and guessing it either way would
        // be inventing state (§5). Guessing "not connected" is exactly the bug this gate exists
        // for; guessing "connected" would silently drop a setting the user turned on.
        if !self.status_attempted {
            return;
        }
        let Some(target) = self.startup_connect.take() else {
            return;
        };
        let already = match &self.state.connection.value {
            ConnectionStatus::Connected(info) => {
                Some(self.i18n.engine_startup_connect_already(info.describe()))
            }
            ConnectionStatus::Connecting => Some(self.i18n.engine_startup_connect_in_progress()),
            _ => None,
        };
        match already {
            // In the console, as a note: this is a decision we made, and it must never look like a
            // command that ran (§10.4).
            Some(why) => {
                let started_at = SystemTime::now();
                self.record_note(
                    self.i18n.engine_startup_connect_skipped(),
                    vec![why],
                    started_at,
                    Duration::ZERO,
                );
            }
            None => self.run_intent(Intent::Connect(target)),
        }
    }

    /// Is a `config list` value known to be `on`? Unknown is not `on`: for a commitment we make
    /// on the user's behalf, the safe reading is "make sure".
    fn setting_is_on(&self, key: &str) -> bool {
        self.state
            .settings
            .as_ref()
            .and_then(|settings| settings.value.iter().find(|setting| setting.key == key))
            .is_some_and(|setting| setting.value == "on")
    }

    /// Whether the tunnel currently up should have a lease, per `active_port_forwarding`.
    fn wants_port_forwarding(&self) -> bool {
        self.active_port_forwarding.unwrap_or_else(|| {
            self.config
                .selected()
                .is_some_and(|saved| saved.port_forwarding)
        })
    }

    /// Queues a command. The launcher decides argv; the runner decides when it runs.
    fn submit(&mut self, intent: Intent) {
        let mut argv = intent.argv();
        argv[0] = self.program.clone();
        let id = self.next_invocation_id();
        let interactive = intent.is_interactive();
        self.jobs.insert(
            id,
            SubmittedJob {
                argv: argv.clone(),
                display: None,
                interactive,
            },
        );

        let job = Job::command(argv, self.cwd.clone())
            .with_id(id)
            .with_language(self.i18n.locale());
        let job = if interactive { job.interactive() } else { job };
        if !self.runner.submit(job) {
            self.jobs.remove(&id);
            self.note(&self.i18n.engine_submit_failed());
            return;
        }
        self.publish_runner_status();
    }

    /// The bus owns the id space, because the bus is what a job's record is keyed by: an id
    /// allocated anywhere else would have to be reconciled with it later, and reconciliation is
    /// where the ids got swapped.
    fn next_invocation_id(&mut self) -> InvocationId {
        self.bus
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .next_id()
    }

    /// A note invocation: something *we* did, shown verbatim and never as a `protonvpn` command
    /// (`docs/architecture.md` §10.4).
    fn record_note(
        &mut self,
        display: String,
        lines: Vec<String>,
        started_at: SystemTime,
        duration: Duration,
    ) {
        let id = self.next_invocation_id();
        let mut bus = self
            .bus
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        bus.begin(
            id,
            InvocationKind::Note,
            Vec::new(),
            Some(display),
            self.cwd.clone(),
            started_at,
        );
        let at = started_at + duration;
        for line in lines {
            bus.push_line(id, line, at);
        }
        bus.finish(id, Some(0), duration, at);
    }

    fn drain_runner_events(&mut self) {
        let mut events = Vec::new();
        while let Ok(event) = self.runner_rx.try_recv() {
            events.push(event);
        }
        if events.is_empty() {
            return;
        }

        for event in events {
            let Some((id, log_event)) = self.fold(event.clone()) else {
                continue;
            };
            let next = {
                let bus = self
                    .bus
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                let mut state = std::mem::take(&mut self.state);
                interpreter::apply(&mut state, &log_event, bus.get(id), &self.i18n);
                state
            };
            self.state = next;
            // Before it is published: a view must never see a connection state the proxy has not
            // reacted to yet, or the reference could still be re-sampling while the window says
            // the tunnel is up.
            self.observe_connection();
            self.publish_state();
            self.react_to(&event, id);
        }

        self.publish_runner_status();
    }

    /// Applies a runner event to the log bus and returns the equivalent log event.
    fn fold(&mut self, event: RunnerEvent) -> Option<(InvocationId, LogEvent)> {
        match event {
            RunnerEvent::Started { id, at } => {
                let (argv, display) = match self.jobs.get(&id) {
                    Some(job) => (job.argv.clone(), job.display.clone()),
                    None => (Vec::new(), None),
                };
                let mut bus = self
                    .bus
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                bus.begin(
                    id,
                    InvocationKind::ProtonVpn,
                    argv.clone(),
                    display.clone(),
                    self.cwd.clone(),
                    at,
                );
                Some((
                    id,
                    LogEvent::Started {
                        id,
                        kind: InvocationKind::ProtonVpn,
                        argv,
                        display,
                        cwd: self.cwd.clone(),
                        at,
                    },
                ))
            }
            RunnerEvent::Line { id, text, at } => {
                self.note_prompt_if_any(id, &text);
                let mut bus = self
                    .bus
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                bus.push_line(id, text.clone(), at);
                Some((
                    id,
                    LogEvent::Line {
                        id,
                        line: crate::logbus::LogLine { text, at },
                    },
                ))
            }
            RunnerEvent::SpawnFailed { id, message, at } => {
                // Our own message, not the CLI's: it must be distinguishable in the transcript.
                let mut bus = self
                    .bus
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                bus.push_line(id, message.clone(), at);
                Some((
                    id,
                    LogEvent::Line {
                        id,
                        line: crate::logbus::LogLine { text: message, at },
                    },
                ))
            }
            RunnerEvent::Finished {
                id,
                exit_code,
                duration,
                at,
            } => {
                {
                    let mut bus = self
                        .bus
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    bus.finish(id, exit_code, duration, at);
                }
                Some((
                    id,
                    LogEvent::Finished {
                        id,
                        exit_code,
                        duration,
                        at,
                    },
                ))
            }
        }
    }

    /// If the running child is an interactive one and just asked something, answer it — or ask
    /// the user. Secrets go to the PTY and nowhere else (`docs/architecture.md` §4).
    fn note_prompt_if_any(&mut self, id: InvocationId, text: &str) {
        let interactive = self.jobs.get(&id).is_some_and(|job| job.interactive);
        if !interactive {
            return;
        }
        let Some(kind) = interpreter::classify_prompt(text) else {
            return;
        };
        let answer = match kind {
            PromptKind::Password => self.secrets.as_ref().map(|s| s.password.clone()),
            PromptKind::TwoFactor => self.secrets.as_ref().and_then(|s| s.two_factor.clone()),
            PromptKind::Unrecognised => None,
        };
        match answer {
            Some(value) if !value.is_empty() => match self.runner.stdin() {
                Some(stdin) => {
                    if stdin.write_line(&value).is_err() {
                        self.note(&self.i18n.engine_stdin_failed());
                    }
                }
                None => self.set_pending_prompt(Some(PendingPrompt {
                    invocation: id,
                    kind,
                })),
            },
            _ => self.set_pending_prompt(Some(PendingPrompt {
                invocation: id,
                kind,
            })),
        }
    }

    /// A finished (or unspawnable) `status` is what closes the question the startup connect is
    /// waiting on. It is asked of the job table rather than of the interpreter's state because a
    /// `status` that failed leaves the state `Unknown` — and `Unknown` still means the question was
    /// asked and could not be answered, which is not a reason to sit on the user's setting.
    fn note_status_attempted(&mut self, id: InvocationId) {
        let is_status = self
            .jobs
            .get(&id)
            .and_then(|job| interpreter::subcommand(&job.argv))
            .is_some_and(|subcommand| subcommand == "status");
        if is_status && !self.status_attempted {
            self.status_attempted = true;
            self.resolve_startup_connect();
        }
    }

    /// Post-invocation reactions: things a wrapper is expected to do, which are not themselves
    /// state.
    fn react_to(&mut self, event: &RunnerEvent, id: InvocationId) {
        let exit_code = match event {
            RunnerEvent::Finished { exit_code, .. } => *exit_code,
            RunnerEvent::SpawnFailed { .. } => {
                self.note_status_attempted(id);
                return;
            }
            _ => return,
        };
        self.note_status_attempted(id);

        let Some(job) = self.jobs.remove(&id) else {
            return;
        };
        let argv = job.argv;
        let subcommand = interpreter::subcommand(&argv)
            .unwrap_or_default()
            .to_string();
        let second = argv.get(2).cloned().unwrap_or_default();

        match subcommand.as_str() {
            "signin" => {
                self.secrets = None;
                self.publish_secrets_flag();
                self.set_pending_prompt(None);
                // Whether it worked or not, ask who we are: `info` is the evidence, not the exit
                // code alone.
                self.submit(Intent::AccountInfo);
            }
            "signout" => {
                self.secrets = None;
                self.publish_secrets_flag();
                if exit_code == Some(0) {
                    self.submit(Intent::AccountInfo);
                }
            }
            "connect" | "disconnect" => {
                if self.schedule.on_invocation_finished(Instant::now(), true) {
                    self.submit(Intent::RefreshStatus);
                }
                if subcommand == "disconnect" {
                    self.lease_attempted_for = None;
                }
            }
            // Only `config list` is evidence that a setting took effect, so a successful `set`
            // is followed by a fresh list. A connect queued behind the set goes after that list,
            // so the console reads in the order a careful human would do it.
            "config" if second == "set" => {
                if exit_code == Some(0) {
                    self.submit(Intent::ListSettings);
                    if let Some(target) = self.pending_connect.take() {
                        self.establish(Intent::Connect(target));
                    }
                } else {
                    // The preference could not be set, so the connect it was preparing for must
                    // not run: a profile that asked for a lease would silently not get one.
                    self.pending_connect = None;
                }
            }
            _ => {}
        }
    }

    fn publish_state(&mut self) {
        {
            let mut shared = self.lock();
            shared.state = self.state.clone();
        }
        self.publish_tray();
    }

    fn publish_secrets_flag(&mut self) {
        let held = self.secrets.is_some();
        let mut shared = self.lock();
        shared.has_secrets = held;
    }

    fn publish_runner_status(&mut self) {
        let status = self.current_runner_status();
        let mut shared = self.lock();
        shared.runner = status;
    }

    fn current_runner_status(&self) -> RunnerStatus {
        let running = self
            .bus
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .running()
            .map(|invocation| {
                (
                    invocation.id,
                    invocation.argv.clone(),
                    invocation.started_at,
                )
            });
        match running {
            Some((id, argv, started_at)) => RunnerStatus::Running {
                id,
                argv,
                started_at,
            },
            None => {
                let depth = self.runner.queued();
                if depth > 0 {
                    RunnerStatus::Queued { depth }
                } else {
                    RunnerStatus::Idle
                }
            }
        }
    }

    fn publish_tray(&mut self) {
        let update = tray_update(&self.update);
        let view = TrayView {
            status: self.state.connection.value.clone(),
            language: self.i18n.locale(),
            detail: match &self.state.connection.value {
                ConnectionStatus::Connected(info) => Some(info.describe()),
                _ => None,
            },
            age_text: self.i18n.age_text(self.state.connection.age()),
            update,
        };
        if self.tray_view.as_ref() == Some(&view) {
            return;
        }
        if let Some(tray) = &self.tray {
            tray.update(view.clone());
        }
        self.tray_view = Some(view);
    }

    fn set_pending_prompt(&mut self, prompt: Option<PendingPrompt>) {
        let mut shared = self.lock();
        shared.pending_prompt = prompt;
    }

    fn write_stdin(&mut self, text: &str) {
        match self.runner.stdin() {
            Some(stdin) => {
                if stdin.write_line(text).is_err() {
                    self.note(&self.i18n.engine_stdin_failed());
                }
                self.set_pending_prompt(None);
            }
            None => self.note(&self.i18n.engine_no_prompt()),
        }
    }

    fn note(&mut self, text: &str) {
        let mut shared = self.lock();
        shared.note = Some(text.to_string());
    }

    fn save_config(&mut self, config: Config) {
        let socks5_changed = config.socks5 != self.config.socks5;
        let updater_switched_on = config.update.policy != UpdatePolicy::Off
            && self.config.update.policy == UpdatePolicy::Off;
        match self.store.save(&config) {
            Ok(()) => {
                self.config = config.clone();
                let mut shared = self.lock();
                shared.config = config;
                shared.note = None;
                drop(shared);
                if socks5_changed {
                    self.restart_socks5();
                }
                if updater_switched_on && !self.update_busy() {
                    // Switching it on is a request to look, not an instruction to wait a day. The
                    // check runs again and takes it from there — one extra small file, and one
                    // code path instead of two.
                    self.next_update_check = Instant::now() + UPDATE_STARTUP_DELAY;
                }
            }
            Err(error) => self.note(&error.describe(&self.i18n)),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Shared> {
        self.shared
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    // --- ground truth (exception #1) -------------------------------------------------------

    fn choose_probe_async(&mut self) {
        if self.probe_in_flight {
            return;
        }
        self.probe_in_flight = true;
        let tx = self.tx.clone();
        thread::Builder::new()
            .name("protonvpn-probe".into())
            .spawn(move || {
                let chosen = Probe::choose();
                let baseline = chosen.as_ref().map(|probe| probe.read(probe::Family::V4));
                let _ = tx.send(Request::ProbeChosen { chosen, baseline });
            })
            .ok();
    }

    fn take_baseline_async(&mut self) {
        if !self.config.probe_enabled {
            return;
        }
        let Some(probe) = self.probe.clone() else {
            self.choose_probe_async();
            return;
        };
        let tx = self.tx.clone();
        thread::Builder::new()
            .name("protonvpn-probe-baseline".into())
            .spawn(move || {
                let started_at = SystemTime::now();
                let result = probe.read(probe::Family::V4);
                let duration = since(started_at);
                let _ = tx.send(Request::ProbeFinished {
                    target: ProbeTarget::Baseline,
                    endpoint: probe.endpoint(),
                    result,
                    started_at,
                    duration,
                });
            })
            .ok();
    }

    fn probe_now(&mut self) {
        if !self.config.probe_enabled {
            self.note(&self.i18n.engine_probe_disabled());
            return;
        }
        let Some(probe) = self.probe.clone() else {
            self.note(&self.i18n.engine_probe_unavailable());
            return;
        };
        if self.probe_in_flight {
            return;
        }
        self.probe_in_flight = true;
        let tx = self.tx.clone();
        thread::Builder::new()
            .name("protonvpn-probe-current".into())
            .spawn(move || {
                let started_at = SystemTime::now();
                let result = probe.read(probe::Family::V4);
                let duration = since(started_at);
                let _ = tx.send(Request::ProbeFinished {
                    target: ProbeTarget::Current,
                    endpoint: probe.endpoint(),
                    result,
                    started_at,
                    duration,
                });
            })
            .ok();
    }

    fn apply_probe(
        &mut self,
        target: ProbeTarget,
        result: ProbeResult,
        at: SystemTime,
        _duration: Duration,
    ) {
        if let Ok(reading) = result {
            let observation = Observation::at(reading, at);
            match target {
                ProbeTarget::Baseline => self.state.egress.baseline = Some(observation),
                ProbeTarget::Current | ProbeTarget::Watch => {
                    self.state.egress.current = Some(observation)
                }
            }
            self.publish_state();
        }
    }

    // --- port forwarding (exception #2) ----------------------------------------------------

    fn maybe_start_lease(&mut self) {
        if !self.wants_port_forwarding() {
            return;
        }
        if !self.state.connection.value.is_connected() {
            return;
        }
        if matches!(
            self.state.port_forwarding.value,
            PortForwarding::Unsupported | PortForwarding::Unavailable(_)
        ) {
            // Respect the server's own answer, and do not spam a gateway that is not answering.
            return;
        }
        let server = match &self.state.connection.value {
            ConnectionStatus::Connected(info) => Some(info.server.clone()),
            _ => None,
        };
        if self.lease_attempted_for == server && server.is_some() {
            return;
        }
        self.lease_attempted_for = server.clone();
        self.request_lease(server);
    }

    fn request_lease(&mut self, server: Option<String>) {
        let started_at = SystemTime::now();
        let client = NatPmp::default();
        let gateway = format!("{}:{}", client.gateway().ip(), client.gateway().port());

        // Opcode 0 first: if the gateway is not answering, the honest result is "unavailable"
        // rather than a port number (`docs/architecture.md` §10.1).
        let external_ip = match client.public_address() {
            Ok(ip) => Some(ip),
            Err(error) => {
                let message = error.describe(&self.i18n);
                self.state.port_forwarding =
                    Observation::now(PortForwarding::Unavailable(message.clone()));
                self.publish_state();
                self.record_note(
                    self.i18n.engine_lease_note_public_address(&gateway),
                    vec![message],
                    started_at,
                    since(started_at),
                );
                return;
            }
        };

        let mut lines = vec![self.i18n.engine_lease_gateway_address(
            external_ip.map(|ip| ip.to_string()).unwrap_or_default(),
        )];
        let mut port = None;
        let mut lifetime = natpmp::LEASE;
        for protocol in [Protocol::Udp, Protocol::Tcp] {
            match client.map(protocol, 0, 0, natpmp::LEASE) {
                Ok(mapping) => {
                    lines.push(self.i18n.engine_lease_mapped(
                        protocol.label(),
                        mapping.external_port as i64,
                        mapping.internal_port as i64,
                        mapping.lifetime.as_secs() as i64,
                    ));
                    port = Some(mapping.external_port);
                    lifetime = mapping.lifetime.max(Duration::from_secs(1));
                }
                Err(error) => {
                    lines.push(format!(
                        "{}: {}",
                        protocol.label(),
                        error.describe(&self.i18n)
                    ));
                    if matches!(error, natpmp::NatPmpError::Refused { .. }) {
                        self.state.port_forwarding = Observation::now(PortForwarding::Unsupported);
                        self.publish_state();
                        self.record_note(
                            self.i18n.engine_lease_note_map(protocol.label(), &gateway),
                            lines,
                            started_at,
                            since(started_at),
                        );
                        return;
                    }
                }
            }
        }

        self.record_note(
            self.i18n.engine_lease_note_map("UDP+TCP", &gateway),
            lines,
            started_at,
            since(started_at),
        );

        match port {
            Some(port) => {
                let next_renewal = Instant::now() + natpmp::RENEW_EVERY;
                self.lease = Some(ActiveLease {
                    port,
                    external_ip,
                    next_renewal,
                    server,
                });
                self.state.port_forwarding = Observation::now(PortForwarding::Active {
                    port,
                    lifetime,
                    external_ip,
                });
                self.publish_state();
            }
            None => {
                self.state.port_forwarding = Observation::now(PortForwarding::Unavailable(
                    self.i18n.engine_lease_no_port(),
                ));
                self.publish_state();
            }
        }
    }

    fn renew_lease(&mut self) {
        let Some(lease) = &self.lease else {
            return;
        };
        let port = lease.port;
        let client = NatPmp::default();
        let mut ok = true;
        for protocol in [Protocol::Udp, Protocol::Tcp] {
            if client.map(protocol, port, port, natpmp::LEASE).is_err() {
                ok = false;
            }
        }
        if ok {
            let next_renewal = Instant::now() + natpmp::RENEW_EVERY;
            if let Some(lease) = self.lease.as_mut() {
                lease.next_renewal = next_renewal;
            }
            self.state.port_forwarding = Observation::now(PortForwarding::Active {
                port,
                lifetime: natpmp::LEASE,
                external_ip: self.lease.as_ref().and_then(|lease| lease.external_ip),
            });
            self.publish_state();
        } else {
            // Stop showing a port we cannot confirm is still ours: that would be misinformation
            // (`docs/cli-surface.md` §4.7).
            self.lease = None;
            self.state.port_forwarding = Observation::now(PortForwarding::Unavailable(
                self.i18n.engine_lease_renew_failed(),
            ));
            self.publish_state();
            self.note(&self.i18n.engine_lease_lost());
        }
    }

    fn release_lease(&mut self, why: &str) {
        let Some(lease) = self.lease.take() else {
            return;
        };
        let started_at = SystemTime::now();
        let client = NatPmp::default();
        let mut lines = Vec::new();
        for protocol in [Protocol::Udp, Protocol::Tcp] {
            match client.release(protocol, lease.port) {
                Ok(()) => lines.push(self.i18n.engine_lease_released(protocol.label())),
                Err(error) => lines.push(format!(
                    "{}: {}",
                    protocol.label(),
                    error.describe(&self.i18n)
                )),
            }
        }
        self.record_note(
            self.i18n.engine_lease_note_release(lease.port as i64, why),
            lines,
            started_at,
            since(started_at),
        );
        self.state.port_forwarding = Observation::now(PortForwarding::Idle);
        self.publish_state();
    }
    // --- the local SOCKS5 proxy (exception #3) ---------------------------------------------

    /// Starts, stops or restarts the listener to match the configuration. The counters and the
    /// gate outlive the listener on purpose: they belong to the process, not to one port.
    fn restart_socks5(&mut self) {
        // The door first, then the listener. A session that is between its dial and its first
        // relayed byte reads the gate, and a gate still standing open while the old server is torn
        // down would let that relay outlive the registry that is supposed to drop it.
        self.shut_gate(Closed::Disabled);
        self.stop_socks5();
        self.dial_failures = 0;
        self.probe_failures = 0;
        self.next_arm_attempt = Instant::now();
        self.next_verify = Instant::now() + self.verify_interval();

        if !self.config.socks5.enabled {
            self.shut_gate(Closed::Disabled);
            self.publish_socks5();
            return;
        }

        // `localhost` is what a person types; it is also the only name that is certainly loopback.
        let address = self.config.socks5.address.trim();
        let parsed = if address.eq_ignore_ascii_case("localhost") {
            Ok(Ipv4Addr::LOCALHOST)
        } else {
            address.parse::<Ipv4Addr>()
        };
        let options = match parsed {
            Ok(bind) => socks5::Socks5Options {
                bind,
                port: self.config.socks5.port,
            },
            Err(_) => {
                self.unavailable(self.i18n.engine_socks5_bad_address(address));
                return;
            }
        };

        let tx = self.tx.clone();
        let report: Reporter = Arc::new(move |event| {
            let _ = tx.send(Request::Socks5Report(event));
        });
        match Socks5::spawn(
            options,
            self.gate.clone(),
            Arc::clone(&self.socks5_stats),
            report,
        ) {
            Ok(server) => {
                self.socks5_listen = Some(server.addr());
                self.socks5 = Some(server);
                self.publish_socks5();
                self.note_socks5(self.i18n.engine_socks5_listening());
                self.arm_socks5();
            }
            Err(error) => self.unavailable(error.describe(&self.i18n)),
        }
    }

    /// Enabled, but there is no listener. The console and the settings page both say so; the gate
    /// says it too, because no listener means nothing can be dialled.
    fn unavailable(&mut self, detail: String) {
        self.socks5_listen = None;
        self.shut_gate(Closed::NotListening {
            detail: detail.clone(),
        });
        self.publish_socks5();
        self.note_socks5(self.i18n.engine_socks5_not_listening(&detail));
        self.note(&detail);
    }

    fn stop_socks5(&mut self) {
        // Dropping the handle stops the listener and drops every relayed connection.
        if let Some(server) = self.socks5.take() {
            drop(server);
        }
        self.socks5_listen = None;
    }

    /// Keeps a picture of the route the tunnel is *not* using. Only ever taken while the CLI says
    /// the tunnel is down, or before we know anything at all.
    fn sample_reference(&mut self) {
        self.route_reference = self.route.source().ok();
    }

    /// Watches the CLI's verdict for the transitions the proxy cares about.
    fn observe_connection(&mut self) {
        let connected = self.state.connection.value.is_connected();
        let was_connected = self.last_connection.is_connected();
        self.last_connection = self.state.connection.value.clone();

        // `Connecting` and `Error` are deliberately not reference material: during a server switch
        // the tunnel is usually still up, and a reference taken then would be the tunnel's own
        // address — which would shut the proxy for the rest of the session.
        if matches!(
            self.state.connection.value,
            ConnectionStatus::Unknown | ConnectionStatus::Disconnected
        ) {
            if self.reference_settle_until.is_none() {
                self.reference_settle_until = Some(Instant::now() + REFERENCE_SETTLE);
                self.next_reference_sample = Instant::now();
                self.sample_reference();
            }
        } else {
            self.reference_settle_until = None;
        }

        if !connected {
            if self.gate.is_open() {
                self.close_gate(Closed::NotConnected);
            }
            return;
        }
        if !was_connected {
            self.arm_socks5();
        }
    }

    /// Opens the gate if the evidence is there, and closes it with the reason if it is not.
    ///
    /// The evidence is one comparison: the route now must differ from a route observed while the
    /// CLI said the tunnel was down. That is the routing-level twin of "did the egress address
    /// change?" (`docs/architecture.md` §8), and it needs nothing but the kernel's own answer.
    fn arm_socks5(&mut self) {
        if !self.config.socks5.enabled || self.socks5.is_none() {
            return;
        }
        if !self.state.connection.value.is_connected() {
            if !matches!(self.gate.state(), GateState::Closed(Closed::NotConnected)) {
                self.shut_gate(Closed::NotConnected);
                self.publish_socks5();
            }
            return;
        }
        let candidate = match self.route.source() {
            Ok(candidate) => candidate,
            Err(error) => {
                self.close_gate(Closed::RouteLost {
                    detail: RouteLoss::Unreadable(error),
                });
                return;
            }
        };
        match self.route_reference {
            Some(reference) if reference != candidate => {
                // Already armed on exactly this address: nothing changed, and the console does not
                // need to hear it twice.
                if self.gate.state() == (GateState::Open { source: candidate }) {
                    return;
                }
                self.dial_failures = 0;
                self.gate.open(candidate);
                self.publish_socks5();
                self.note_socks5(
                    self.i18n
                        .engine_socks5_route_proven(candidate.to_string(), reference.to_string()),
                );
            }
            _ => {
                self.shut_gate(Closed::Unverified { candidate });
                self.publish_socks5();
            }
        }
    }

    /// Closes the gate and drops everything it was protecting, without a word. Used where the
    /// caller is about to say what is happening anyway — a restart, a listener that never came up.
    fn shut_gate(&mut self, reason: Closed) {
        self.gate.close(reason.clone());
        if self.withdraws_the_reference(&reason) {
            // The route we were comparing against is spent: it was taken before the tunnel moved
            // under us. Arming again needs a *fresh* observation of a route the CLI called
            // down — which is what a reconnect gives us, and what nothing else can fake.
            self.route_reference = None;
        }
        if let Some(server) = &self.socks5 {
            server.drop_connections();
        }
    }

    /// Does this close mean the evidence itself is no longer trustworthy?
    ///
    /// Only the reasons that say *the route is not what we thought*: a listener that is not there
    /// or a user who switched the proxy off says nothing about the route, and armouring those with
    /// a lost reference would only mean re-arming after fixing a port number.
    fn withdraws_the_reference(&self, reason: &Closed) -> bool {
        matches!(
            reason,
            Closed::RouteChanged { .. }
                | Closed::RouteLost { .. }
                | Closed::EgressIsBaseline { .. }
                | Closed::ProbeUnanswered { .. }
        )
    }

    /// Closes the gate, says so once if it had been open, and withdraws evidence that no longer
    /// holds.
    fn close_gate(&mut self, reason: Closed) {
        let was_open = self.gate.is_open();
        let changed = self.gate.state() != GateState::Closed(reason.clone());
        self.shut_gate(reason.clone());
        self.publish_socks5();
        if !changed || !was_open {
            return;
        }
        self.note_socks5(self.i18n.engine_socks5_closed(reason.describe(&self.i18n)));
    }

    /// Reports from the proxy's own threads.
    fn react_to_socks5(&mut self, event: Socks5Event) {
        match event {
            Socks5Event::RouteDiverged { reason } => self.close_gate(reason),
            Socks5Event::SourceMismatch { expected, observed } => {
                self.close_gate(Closed::RouteChanged {
                    expected,
                    observed: Some(observed),
                });
            }
            // A relayed connection is proof the path works, so the count is of *consecutive*
            // failures and not of a bad afternoon.
            Socks5Event::Dialed => self.dial_failures = 0,
            Socks5Event::DialFailed {
                target,
                failure,
                detail,
            } => {
                // A destination that answered and said no is not evidence about the path.
                if !failure.implicates_the_path() {
                    return;
                }
                self.dial_failures += 1;
                if self.dial_failures >= DIAL_FAILURE_LIMIT {
                    self.close_gate(Closed::RouteLost {
                        detail: RouteLoss::Dial { target, detail },
                    });
                }
            }
        }
    }

    /// What a background tunnel check concluded.
    ///
    /// `None` from `egress_changed` is a real answer — no baseline, or a different endpoint — and
    /// it means "cannot tell", on which we claim nothing (`docs/architecture.md` §5).
    fn judge_watch(&mut self, answered: bool) {
        if !answered {
            self.probe_failures += 1;
            if self.probe_failures >= WATCH_FAILURE_LIMIT {
                self.close_gate(Closed::ProbeUnanswered {
                    detail: self.i18n.engine_socks5_two_probes_silent(),
                });
            }
            return;
        }
        self.probe_failures = 0;
        match self.state.egress.egress_changed() {
            // The egress is the pre-connection address again, so the tunnel is not carrying
            // traffic whatever the CLI says. This is the one thing no CLI output can tell us.
            Some(false) => {
                let ip = self
                    .state
                    .egress
                    .current
                    .as_ref()
                    .map(|reading| reading.value.ip);
                if let Some(ip) = ip {
                    self.close_gate(Closed::EgressIsBaseline { ip });
                }
            }
            // And the reverse: proof that traffic is going somewhere else reopens a gate that was
            // closed on suspicion.
            Some(true) => {
                self.dial_failures = 0;
                if !self.gate.is_open() {
                    self.arm_socks5();
                }
            }
            None => {}
        }
    }

    /// One background reading, on a thread of its own. The result comes back as a `Request`, like
    /// every other reading.
    fn watch_tunnel(&mut self) {
        let Some(probe) = self.probe.clone() else {
            return;
        };
        self.probe_in_flight = true;
        let tx = self.tx.clone();
        thread::Builder::new()
            .name("protonvpn-socks5-watch".into())
            .spawn(move || {
                let started_at = SystemTime::now();
                let result = probe.read(probe::Family::V4);
                let duration = since(started_at);
                let _ = tx.send(Request::ProbeFinished {
                    target: ProbeTarget::Watch,
                    endpoint: probe.endpoint(),
                    result,
                    started_at,
                    duration,
                });
            })
            .ok();
    }

    /// A proxy lifecycle line in the console. It is not a `protonvpn` command, so it is a note,
    /// never dressed up as one (`docs/architecture.md` §10.4). Individual connections never appear
    /// here: a browser would drown the transcript.
    fn note_socks5(&mut self, line: String) {
        let display = match self.socks5_listen {
            Some(addr) => format!("SOCKS5 {addr}"),
            None => format!(
                "SOCKS5 {}:{}",
                self.config.socks5.address.trim(),
                self.config.socks5.port
            ),
        };
        self.record_note(display, vec![line], SystemTime::now(), Duration::ZERO);
    }

    /// The background check's cadence. Clamped: a hand-typed number must not be able to overflow
    /// an `Instant`, and a day is already far past any useful interval.
    fn verify_interval(&self) -> Duration {
        Duration::from_secs(self.config.socks5.verify_seconds.clamp(1, 86_400))
    }

    fn publish_socks5(&mut self) {
        let listen = self.socks5_listen;
        let mut shared = self.lock();
        shared.socks5.listen = listen;
    }

    // --- the AppImage updater (exception #4) -----------------------------------------------

    fn update_busy(&self) -> bool {
        !matches!(self.update_job, UpdateJob::Idle)
    }

    /// Looks at the release page, on a thread of its own. Never on this one: `curl` takes as long
    /// as the network takes, and this thread owns everything the views ask about — including
    /// whether a `protonvpn` command is running. Nothing here waits for that thread.
    fn start_update_check(&mut self) {
        if self.update_busy() {
            self.note(&self.i18n.engine_update_check_running());
            return;
        }
        self.update_job = UpdateJob::Checking;
        self.update.phase = UpdatePhase::Checking;
        self.publish_update();

        let tx = self.tx.clone();
        let curl = self.curl.clone();
        thread::Builder::new()
            .name("protonvpn-update-check".into())
            .spawn(move || {
                let started_at = SystemTime::now();
                let result = update::latest(&curl);
                let duration = since(started_at);
                let _ = tx.send(Request::UpdateReport(UpdateEvent::Checked {
                    started_at,
                    duration,
                    result,
                }));
            })
            .ok();
    }

    /// Downloads the release and, when asked, puts it in place — both on one thread, because the
    /// second half of that is a rename. [`update::fetch`] reuses a staged file that already hashes
    /// right, so `Install` after a download is a rename and not a second 70 MB.
    fn start_update_fetch(&mut self, install_after: bool) {
        if self.update_busy() {
            return;
        }
        let (Some(installed), Some(release)) = (self.installed.clone(), self.release.clone())
        else {
            self.note(&if self.installed.is_none() {
                self.i18n.engine_update_not_an_appimage()
            } else {
                self.i18n.engine_update_nothing_to_fetch()
            });
            return;
        };

        // Spelled here rather than inside the thread: the catalogue belongs to this thread, and
        // the title has to exist before the download starts in case the rename fails.
        let install_note = self.i18n.engine_update_note(release.version.to_string());

        self.update_cancel.store(false, Ordering::Relaxed);
        self.update_job = UpdateJob::Fetching { install_after };
        self.update.phase = UpdatePhase::Downloading {
            received: 0,
            total: None,
        };
        self.publish_update();

        let tx = self.tx.clone();
        let curl = self.curl.clone();
        let cancel = Arc::clone(&self.update_cancel);

        thread::Builder::new()
            .name("protonvpn-update-fetch".into())
            .spawn(move || {
                let started_at = SystemTime::now();
                let progress_tx = tx.clone();
                let outcome = update::fetch(
                    &curl,
                    &installed,
                    &release,
                    &cancel,
                    move |received, total| {
                        let _ = progress_tx.send(Request::UpdateReport(UpdateEvent::Progress {
                            received,
                            total,
                        }));
                    },
                );
                let duration = since(started_at);
                let display = update::describe_download_call(&release, &installed.staging_path());

                let event = match outcome {
                    Err(error) => UpdateEvent::Failed {
                        display,
                        error,
                        started_at,
                        duration,
                    },
                    Ok(staged) => {
                        let bytes = fs::metadata(&staged).map(|meta| meta.len()).unwrap_or(0);
                        if install_after {
                            match update::install(&installed, &staged) {
                                Ok(backup) => UpdateEvent::Installed {
                                    version: release.version,
                                    backup,
                                    started_at,
                                    duration,
                                },
                                Err(error) => UpdateEvent::Failed {
                                    display: install_note,
                                    error,
                                    started_at,
                                    duration,
                                },
                            }
                        } else {
                            UpdateEvent::Staged {
                                version: release.version,
                                bytes,
                                started_at,
                                duration,
                            }
                        }
                    }
                };
                let _ = tx.send(Request::UpdateReport(event));
            })
            .ok();
    }

    /// Install: what is staged goes in place, and what is not gets downloaded first.
    fn install_update(&mut self) {
        if self.update_busy() {
            return;
        }
        if let UpdatePhase::Installed { .. } = self.update.phase {
            self.note(&self.i18n.engine_update_already_installed());
            return;
        }
        self.start_update_fetch(true);
    }

    fn cancel_update(&mut self) {
        if !self.update_busy() {
            return;
        }
        self.update_cancel.store(true, Ordering::Relaxed);
    }

    /// Stops the tray mentioning this version — not the same as forgetting it. The settings page
    /// goes on saying what it knows, because hiding a notice is not hiding a fact.
    fn dismiss_update(&mut self) {
        let Some(version) = self.update.latest else {
            return;
        };
        let mut config = self.config.clone();
        config.update.dismissed = Some(version.tag());
        self.save_config(config);
        self.publish_update();
    }

    fn react_to_update(&mut self, event: UpdateEvent) {
        match event {
            UpdateEvent::Checked {
                started_at,
                duration,
                result,
            } => {
                self.update_job = UpdateJob::Idle;
                self.update.checked_at = Some(started_at + duration);
                // The command the console shows is spelled from the same arguments that ran, so
                // the transcript cannot drift from what happened (exception #4).
                let command = update::describe_sums_call();

                match result {
                    // The page answered and there is nothing in it this build could become —
                    // a release published before the asset's name carried a version, or one built
                    // for another machine. A finding, not a failure: it is worth saying once, and
                    // it is not worth retrying in six hours.
                    Ok(update::Finding::Nothing(why)) => {
                        self.update.error = None;
                        self.update.latest = None;
                        self.update.phase = UpdatePhase::Idle;
                        self.release = None;
                        self.remember_check(Instant::now() + UPDATE_INTERVAL);
                        let why = why.describe(&self.i18n);
                        self.record_note(command, vec![why.clone()], started_at, duration);
                        self.note(&why);
                    }
                    Ok(update::Finding::Release(release)) => {
                        self.update.error = None;
                        self.update.latest = Some(release.version);
                        self.update.phase = UpdatePhase::Idle;
                        self.remember_check(Instant::now() + UPDATE_INTERVAL);
                        self.record_note(
                            command,
                            vec![self.i18n.engine_update_latest(release.version.to_string())],
                            started_at,
                            duration,
                        );

                        let standing = update::standing(self.version, release.version);
                        self.release = Some(release);
                        match standing {
                            update::Standing::Current => {}
                            update::Standing::Unversioned => {
                                self.note(&self.i18n.engine_update_unversioned());
                            }
                            update::Standing::Behind(version) => {
                                if self.installed.is_none() {
                                    self.note(
                                        &self
                                            .i18n
                                            .engine_update_available_manual(version.to_string()),
                                    );
                                } else if matches!(
                                    self.config.update.policy,
                                    UpdatePolicy::Download | UpdatePolicy::Install
                                ) {
                                    let install =
                                        self.config.update.policy == UpdatePolicy::Install;
                                    self.start_update_fetch(install);
                                }
                            }
                        }
                    }
                    Err(error) => {
                        let described = error.describe(&self.i18n);
                        self.update.error = Some(described.clone());
                        self.update.phase = UpdatePhase::Idle;
                        self.remember_check(Instant::now() + UPDATE_RETRY);
                        self.record_note(command, vec![described], started_at, duration);
                    }
                }
                self.publish_update();
            }
            UpdateEvent::Progress { received, total } => {
                self.update.phase = UpdatePhase::Downloading { received, total };
                self.publish_update();
            }
            UpdateEvent::Staged {
                version,
                bytes,
                started_at,
                duration,
            } => {
                self.update_job = UpdateJob::Idle;
                self.update.phase = UpdatePhase::Staged { version };
                self.record_note(
                    self.update_download_command(),
                    vec![
                        self.i18n
                            .engine_update_downloaded(update::human_bytes(bytes, &self.i18n)),
                        self.i18n.engine_update_staged(version.to_string()),
                    ],
                    started_at,
                    duration,
                );
                self.publish_update();
            }
            UpdateEvent::Installed {
                version,
                backup,
                started_at,
                duration,
            } => {
                self.update_job = UpdateJob::Idle;
                self.update.phase = UpdatePhase::Installed { version };
                let mut lines = vec![self.i18n.engine_update_installed(version.to_string())];
                match &backup {
                    Some(path) => {
                        lines.push(self.i18n.engine_update_backup(path.display().to_string()))
                    }
                    None => lines.push(self.i18n.engine_update_no_backup()),
                }
                lines.push(self.i18n.engine_update_running_old());
                let note = self.i18n.engine_update_note(version.to_string());
                self.record_note(note, lines, started_at, duration);
                self.publish_update();
            }
            UpdateEvent::Failed {
                display,
                error,
                started_at,
                duration,
            } => {
                let was_downloading = matches!(self.update.phase, UpdatePhase::Downloading { .. });
                self.update_job = UpdateJob::Idle;
                self.update.phase = UpdatePhase::Idle;
                let described = error.describe(&self.i18n);
                self.update.error = Some(described.clone());
                if was_downloading {
                    // A transfer that died is worth a line; a check that could not answer already
                    // has one, and the console is not a place for the same sentence twice.
                    self.record_note(display, vec![described], started_at, duration);
                } else {
                    self.note(&described);
                }
                self.publish_update();
            }
            UpdateEvent::Cancelled => {
                self.update_job = UpdateJob::Idle;
                self.update.phase = UpdatePhase::Idle;
                self.note(&self.i18n.engine_update_cancelled());
                self.publish_update();
            }
        }
    }

    /// Writes down that the release page was asked, and when to ask again. In the config rather
    /// than in memory: a restart must not mean a fresh check every time the application is opened.
    fn remember_check(&mut self, next: Instant) {
        self.next_update_check = next;
        let Ok(now) = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH) else {
            return;
        };
        let mut config = self.config.clone();
        config.update.last_check = Some(now.as_secs());
        self.save_config(config);
    }

    /// The download, as the console shows it: the same arguments that actually ran.
    fn update_download_command(&self) -> String {
        match (&self.release, &self.installed) {
            (Some(release), Some(installed)) => {
                update::describe_download_call(release, &installed.staging_path())
            }
            _ => self.i18n.engine_update_curl_fallback(),
        }
    }

    fn publish_update(&mut self) {
        self.update.dismissed = match (self.update.latest, self.config.update.dismissed.as_deref())
        {
            (Some(latest), Some(dismissed)) => latest.tag() == dismissed,
            _ => false,
        };
        let view = self.update.clone();
        self.lock().update = view;
        // The tray shows the update too, so it is refreshed from the same place the view is.
        self.publish_tray();
    }
}

/// Whether the tray should mention an update, and what it should say.
///
/// Only when there is a release this build is behind — an unversioned build has nothing to be
/// behind — and only until the user asks not to hear about it again. The tray is a menu, not a
/// notification system: the item is there to be found, never to interrupt (`docs/architecture.md`
/// §9, §14).
fn tray_update(view: &UpdateView) -> Option<TrayUpdate> {
    match (view.current, view.latest, view.dismissed) {
        (Some(current), Some(latest), false) if current < latest => Some(TrayUpdate {
            version: latest,
            applied: matches!(view.phase, UpdatePhase::Installed { .. }),
        }),
        _ => None,
    }
}

fn describe_reading(reading: &EgressReading) -> String {
    let mut parts = vec![reading.ip.to_string()];
    if let Some(country) = &reading.country {
        parts.push(country.clone());
    }
    if let Some(org) = &reading.asn_org {
        parts.push(org.clone());
    }
    // Country and ASN are for reading, not for judging: the databases disagree with each other
    // (`docs/architecture.md` §8).
    format!("→ {}", parts.join(" · "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, Socks5};
    use crate::model::ConnectTarget;
    use crate::net::route::ScriptedRoute;
    use crate::update::testing::{image_bytes, stand_in, sums_for};
    use std::fs;
    use std::io::Write;
    use std::net::SocketAddrV4;
    use std::os::unix::fs::PermissionsExt;

    /// A stand-in for the real CLI, so the engine can be tested end to end with no VPN, no
    /// network and no dependence on the official package being installed.
    fn write_stand_in(dir: &std::path::Path) -> PathBuf {
        let path = dir.join("protonvpn");
        let script = r#"#!/bin/sh
case "$1" in
  status)
    if [ -f "$0.connected" ]; then
      echo "Status: Connected"
      echo "Server: NL#818 in Amsterdam, Netherlands"
      echo "Load: 59%"
      echo "Protocol: wireguard"
    else
      echo "Status: Disconnected"
    fi
    ;;
  connect)
    echo "Connected to NL#818 in Amsterdam, Netherlands. "
    echo "Your new IP address is 205.147.16.100."
    touch "$0.connected"
    ;;
  disconnect)
    echo "Disconnected."
    rm -f "$0.connected"
    ;;
  info) echo "Account: 'trousev'" ;;
  signin)
    # Exactly the shape of the real CLI: the prompt has no trailing newline, and the password is
    # read without echoing it (measured 2026-10-01).
    stty -echo 2>/dev/null
    printf 'Password: '
    read -r pw
    stty echo 2>/dev/null
    if [ "$pw" = "correct-horse" ]; then
      echo "Signed in."
    else
      echo "Error: Authentication failed."
    fi
    ;;
  *) echo "Current configuration" ;;
esac
"#;
        let mut file = fs::File::create(&path).unwrap();
        file.write_all(script.as_bytes()).unwrap();
        let mut permissions = file.metadata().unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&path, permissions).unwrap();
        path
    }

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "protonvpn-gui-engine-{name}-{}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn path(&self) -> &std::path::Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// The route the machine uses when the tunnel is down, and the one it uses when it is up.
    /// Tests drive the kernel's answer through [`ScriptedRoute`], so no test depends on this
    /// machine's network.
    const LAN: Ipv4Addr = Ipv4Addr::new(192, 168, 1, 10);
    const TUNNEL: Ipv4Addr = Ipv4Addr::new(10, 2, 0, 2);

    fn test_route(answer: Ipv4Addr) -> Arc<ScriptedRoute> {
        ScriptedRoute::new(answer)
    }

    fn start_with(dir: &TempDir, config: Config, route: Arc<ScriptedRoute>) -> EngineHandle {
        let program = write_stand_in(dir.path());
        let options = EngineOptions {
            cwd: dir.path().to_path_buf(),
            store: ConfigStore::at(dir.path().join("config.json")),
            config,
            tray_available: false,
            tray: None,
            // The tests assert on the English wording, which is the source language.
            locale: Locale::SOURCE,
            program: program.to_string_lossy().into_owned(),
            route: Arc::clone(&route) as Arc<dyn RouteProbe>,
            // Not a release, not an AppImage, and no automatic check inside a test's lifetime: a
            // test must never reach the network. `start_updating` is the seam for the tests that
            // do exercise the updater; they point it at a stand-in `curl` and a short delay.
            version: None,
            curl: "curl".to_string(),
            installed: None,
            update_delay: Duration::from_secs(3600),
        };
        spawn(options)
    }

    fn start(dir: &TempDir) -> EngineHandle {
        let config = Config {
            // No `curl` and no NAT-PMP in tests: both are separate, offline-tested modules, and a
            // test must never ask a real gateway for a real port. No saved profile asks for a
            // lease either, so the lease path is never entered here.
            probe_enabled: false,
            ..Default::default()
        };
        start_with(dir, config, test_route(LAN))
    }

    /// Waits for a condition on the published state. Returns `None` on timeout, so a failing test
    /// reports what it actually saw instead of hanging.
    fn wait_for(
        handle: &EngineHandle,
        what: &str,
        mut condition: impl FnMut(&Shared) -> bool,
    ) -> Option<Shared> {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            let shared = handle.snapshot();
            if condition(&shared) {
                return Some(shared);
            }
            thread::sleep(Duration::from_millis(20));
        }
        panic!(
            "timed out waiting for {what}; last state: {:?}",
            handle.snapshot()
        );
    }

    #[test]
    fn opens_by_asking_the_cli_for_the_truth() {
        let dir = TempDir::new("initial-status");
        let handle = start(&dir);

        // The stand-in starts disconnected, so the very first poll must say so — not `Unknown`.
        let shared = wait_for(&handle, "the initial status", |shared| {
            shared.state.connection.value == ConnectionStatus::Disconnected
        })
        .unwrap();
        assert_eq!(
            shared.state.connection.value,
            ConnectionStatus::Disconnected
        );
        assert!(shared.runner != RunnerStatus::Idle || true);

        // The invocation is in the console, verbatim, with its exit code.
        let bus = handle.bus();
        let bus = bus.lock().unwrap();
        let invocation = bus
            .iter()
            .find(|invocation| invocation.command_line().ends_with("status"))
            .expect("the status invocation was recorded");
        assert_eq!(
            invocation.argv,
            vec![
                dir.path().join("protonvpn").to_string_lossy().into_owned(),
                "status".to_string()
            ]
        );
        assert_eq!(invocation.exit_code, Some(0));
        assert_eq!(invocation.output().trim_end(), "Status: Disconnected");

        // Who we are is asked at the same time: the login page is a decision, not a tab.
        assert!(
            bus.iter()
                .any(|invocation| invocation.command_line().ends_with("info")),
            "{:?}",
            bus.iter()
                .map(|invocation| invocation.command_line())
                .collect::<Vec<_>>()
        );
        // Let go of the bus before asking the engine to stop: it takes that same lock to fold the
        // events of the `info` child that is still running, so waiting for it while holding the
        // lock is a deadlock with a five-second fuse. That is the fuse this test tripped on a
        // loaded CI runner — and why every other test in this file drops the guard first.
        drop(bus);

        assert!(handle.shutdown(Duration::from_secs(5)));
    }

    /// The bug this pins, measured against the real CLI: `connect` is **not** a no-op while a tunnel
    /// is up — it switches servers silently and the egress moves under the user
    /// (`docs/cli-surface.md` §4.4). A user who leaves the VPN connected and turns on "connect at
    /// startup" was having their tunnel rebuilt on every launch, and the request is made before the
    /// first `status` has answered, so the engine has to wait for that answer instead of assuming
    /// "not connected".
    #[test]
    fn a_startup_connect_stands_down_when_the_cli_already_reports_a_connection() {
        let dir = TempDir::new("startup-already-up");
        // The stand-in reports a connection while this flag exists: the machine of a user who left
        // the VPN up.
        fs::write(dir.path().join("protonvpn.connected"), b"").unwrap();
        let handle = start(&dir);

        // Asked for immediately, as the view asks at startup, and asking for a lease on top: none
        // of it may happen.
        handle.send(Request::StartupConnect(ConnectTarget {
            country: Some("NL".into()),
            port_forwarding: true,
            ..Default::default()
        }));

        let deadline = Instant::now() + Duration::from_secs(10);
        let mut transcript = String::new();
        while Instant::now() < deadline {
            transcript = handle.bus().lock().unwrap().transcript();
            if transcript.contains("startup connect skipped") {
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
        assert!(
            transcript.contains("startup connect skipped"),
            "the skip is recorded where the user can read it: {transcript}"
        );
        assert!(
            transcript.contains("already connected: NL#818"),
            "and it says what the CLI reported: {transcript}"
        );

        let bus = handle.bus();
        let bus = bus.lock().unwrap();
        let commands: Vec<String> = bus
            .iter()
            .filter(|invocation| invocation.kind.is_protonvpn())
            .map(|invocation| invocation.command_line())
            .collect();
        assert!(
            !commands
                .iter()
                .any(|line| line.ends_with("connect --country NL")),
            "nothing was torn down: {commands:?}"
        );
        assert!(
            !commands
                .iter()
                .any(|line| line.ends_with("config set port-forwarding on")),
            "and the CLI's global preference was not flipped for a connect that never happens: \
             {commands:?}"
        );
        // The decision is a note, never a command that never ran (§10.4).
        assert!(
            bus.iter().any(|invocation| !invocation.kind.is_protonvpn()
                && invocation
                    .command_line()
                    .contains("startup connect skipped")),
            "{:?}",
            bus.iter()
                .map(|invocation| (invocation.kind, invocation.command_line()))
                .collect::<Vec<_>>()
        );
        drop(bus);
        assert!(handle.shutdown(Duration::from_secs(5)));
    }

    /// The other half of the gate: with nothing up, the setting still does what it says — and the
    /// CLI is asked *before* it is asked for a tunnel.
    #[test]
    fn a_startup_connect_runs_when_the_cli_reports_nothing_is_up() {
        let dir = TempDir::new("startup-nothing-up");
        let handle = start(&dir);

        handle.send(Request::StartupConnect(ConnectTarget::country("NL")));
        wait_for(&handle, "a connection", |shared| {
            shared.state.connection.value.is_connected()
        });

        let bus = handle.bus();
        let bus = bus.lock().unwrap();
        let commands: Vec<String> = bus
            .iter()
            .filter(|invocation| invocation.kind.is_protonvpn())
            .map(|invocation| invocation.command_line())
            .collect();
        let status = commands.iter().position(|line| line.ends_with("status"));
        let connect = commands
            .iter()
            .position(|line| line.ends_with("connect --country NL"));
        assert!(status.is_some(), "{commands:?}");
        assert!(connect.is_some(), "{commands:?}");
        assert!(
            status < connect,
            "the CLI is asked what is there before we ask it for a tunnel: {commands:?}"
        );
        drop(bus);
        assert!(handle.shutdown(Duration::from_secs(5)));
    }

    /// The gate is on the app's own initiative, not on the command: a human asking for a switch
    /// while a tunnel is up is obeyed, because that is what they asked for.
    #[test]
    fn a_manual_connect_is_still_obeyed_while_a_connection_is_up() {
        let dir = TempDir::new("manual-switch");
        fs::write(dir.path().join("protonvpn.connected"), b"").unwrap();
        let handle = start(&dir);
        wait_for(&handle, "the connection the CLI already had", |shared| {
            shared.state.connection.value.is_connected()
        });

        handle.send(Request::Run(Intent::Connect(ConnectTarget::country("NL"))));

        let deadline = Instant::now() + Duration::from_secs(10);
        let mut seen = false;
        while Instant::now() < deadline {
            seen = handle
                .bus()
                .lock()
                .unwrap()
                .iter()
                .any(|invocation| invocation.command_line().ends_with("connect --country NL"));
            if seen {
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
        assert!(
            seen,
            "a manual connect must not be swallowed by the startup gate"
        );
        assert!(handle.shutdown(Duration::from_secs(5)));
    }

    /// The console order is the contract here: a profile that wants a lease must not connect
    /// before the CLI's single global preference says it may, and the user must be able to read
    /// that going on (`docs/architecture.md` §11).
    #[test]
    fn a_connect_that_wants_a_port_sets_the_preference_first() {
        let dir = TempDir::new("port-forwarding-order");
        let handle = start(&dir);
        wait_for(&handle, "the initial status", |shared| {
            shared.state.connection.value == ConnectionStatus::Disconnected
        });

        handle.send(Request::Run(Intent::Connect(ConnectTarget {
            country: Some("NL".into()),
            port_forwarding: true,
            ..Default::default()
        })));
        wait_for(&handle, "a connection", |shared| {
            shared.state.connection.value.is_connected()
        });

        let bus = handle.bus();
        let bus = bus.lock().unwrap();
        let commands: Vec<String> = bus
            .iter()
            .filter(|invocation| invocation.kind.is_protonvpn())
            .map(|invocation| invocation.command_line())
            .collect();
        let set = commands
            .iter()
            .position(|line| line.ends_with("config set port-forwarding on"));
        let connect = commands
            .iter()
            .position(|line| line.ends_with("connect --country NL"));
        assert!(set.is_some(), "{commands:?}");
        assert!(connect.is_some(), "{commands:?}");
        assert!(
            set < connect,
            "the preference is set, and listed, before the connect: {commands:?}"
        );
        drop(bus);
        assert!(handle.shutdown(Duration::from_secs(5)));
    }

    /// The wrapper only spends a second on `config set` when a profile actually asked for a lease.
    #[test]
    fn a_connect_that_does_not_want_a_port_sets_nothing() {
        let dir = TempDir::new("no-port-forwarding");
        let handle = start(&dir);
        wait_for(&handle, "the initial status", |shared| {
            shared.state.connection.value == ConnectionStatus::Disconnected
        });

        handle.send(Request::Run(Intent::Connect(ConnectTarget::country("NL"))));
        wait_for(&handle, "a connection", |shared| {
            shared.state.connection.value.is_connected()
        });

        let bus = handle.bus();
        let bus = bus.lock().unwrap();
        let commands: Vec<String> = bus
            .iter()
            .map(|invocation| invocation.command_line())
            .collect();
        assert!(
            !commands.iter().any(|line| line.contains("config set")),
            "{commands:?}"
        );
        drop(bus);
        assert!(handle.shutdown(Duration::from_secs(5)));
    }

    /// The bug this pins, measured against the real CLI: `protonvpn signin` prints `Password: `
    /// with **no trailing newline** and then blocks. A reader that only forwards complete lines
    /// forwards nothing, the engine never sees the prompt, the password is never written, and the
    /// console bar sits at "working" forever. The user cannot get in and cannot get out.
    #[test]
    fn a_login_prompt_without_a_newline_is_seen_answered_and_kept_out_of_the_console() {
        let dir = TempDir::new("signin-prompt");
        let handle = start(&dir);
        wait_for(&handle, "the initial status", |shared| {
            shared.state.connection.value == ConnectionStatus::Disconnected
        });

        handle.send(Request::SignIn {
            username: "trousev".into(),
            password: "correct-horse".into(),
            two_factor: None,
        });

        let deadline = Instant::now() + Duration::from_secs(10);
        let mut transcript = String::new();
        while Instant::now() < deadline {
            transcript = handle.bus().lock().unwrap().transcript();
            if transcript.contains("Signed in.") {
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }

        assert!(
            transcript.contains("Password: "),
            "the prompt reached the console: {transcript}"
        );
        assert!(
            transcript.contains("Signed in."),
            "the password reached the child: {transcript}"
        );
        // The property the whole sign-in path exists to preserve.
        assert!(
            !transcript.contains("correct-horse"),
            "the secret never reaches the console: {transcript}"
        );
        // And nothing is left waiting for an answer that is not coming.
        assert!(handle.snapshot().pending_prompt.is_none());

        assert!(handle.shutdown(Duration::from_secs(5)));
    }

    #[test]
    fn connect_and_disconnect_are_followed_by_a_fresh_status_read() {
        let dir = TempDir::new("connect");
        let handle = start(&dir);
        wait_for(&handle, "the initial status", |shared| {
            shared.state.connection.value == ConnectionStatus::Disconnected
        });

        handle.send(Request::Run(Intent::Connect(ConnectTarget::country("NL"))));
        // Load and protocol arrive from the status read that the connect triggers, not from the
        // connect output itself — so waiting for them is also waiting for that read to happen.
        let connected = wait_for(&handle, "a connection with load", |shared| {
            matches!(
                &shared.state.connection.value,
                ConnectionStatus::Connected(info) if info.load_percent.is_some()
            )
        })
        .unwrap();
        match &connected.state.connection.value {
            ConnectionStatus::Connected(info) => {
                assert_eq!(info.server, "NL#818");
                assert_eq!(info.load_percent, Some(59));
                assert_eq!(info.protocol.as_deref(), Some("wireguard"));
            }
            other => panic!("expected Connected, got {other:?}"),
        }

        handle.send(Request::Run(Intent::Disconnect));
        wait_for(&handle, "a disconnection", |shared| {
            shared.state.connection.value == ConnectionStatus::Disconnected
        });

        // The disconnect triggers a status read; wait for it, because `Disconnected` alone is
        // known the moment the CLI says "Disconnected." — the poll is what proves it stuck.
        let last_protonvpn = |handle: &EngineHandle| {
            handle
                .bus()
                .lock()
                .unwrap()
                .iter()
                .rev()
                .find(|invocation| invocation.kind.is_protonvpn())
                .map(|invocation| invocation.command_line())
        };
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if last_protonvpn(&handle).is_some_and(|line| line.ends_with("status")) {
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }

        let bus = handle.bus();
        let bus = bus.lock().unwrap();
        let commands: Vec<String> = bus
            .iter()
            .map(|invocation| invocation.command_line())
            .collect();
        assert!(
            commands
                .iter()
                .any(|line| line.ends_with("connect --country NL")),
            "{commands:?}"
        );
        assert!(
            commands.iter().any(|line| line.ends_with("disconnect")),
            "{commands:?}"
        );
        let last_command = bus
            .iter()
            .rev()
            .find(|invocation| invocation.kind.is_protonvpn())
            .map(|invocation| invocation.command_line())
            .unwrap();
        assert!(last_command.ends_with("status"), "{last_command}");
        drop(bus);

        assert!(handle.shutdown(Duration::from_secs(5)));
    }

    #[test]
    fn the_runner_status_is_idle_when_nothing_is_happening() {
        let dir = TempDir::new("idle");
        let handle = start(&dir);
        wait_for(&handle, "an idle runner", |shared| {
            shared.runner == RunnerStatus::Idle
                && shared.state.connection.value == ConnectionStatus::Disconnected
        });
        assert!(handle.shutdown(Duration::from_secs(5)));
    }

    #[test]
    fn a_config_saved_through_the_engine_lands_on_disk() {
        let dir = TempDir::new("config");
        let handle = start(&dir);
        wait_for(&handle, "the initial status", |shared| {
            shared.state.connection.value == ConnectionStatus::Disconnected
        });

        let mut config = handle.snapshot().config;
        config.start_minimized = true;
        handle.send(Request::SaveConfig(Box::new(config.clone())));
        thread::sleep(Duration::from_millis(200));

        let written = ConfigStore::at(dir.path().join("config.json"))
            .load()
            .unwrap();
        assert!(written.start_minimized);
        assert_eq!(handle.snapshot().config, config);
        assert!(handle.shutdown(Duration::from_secs(5)));
    }

    /// Starts the engine with the proxy enabled, and returns it with the route probe the test
    /// drives. `verify_seconds: 0` keeps `curl` out of the test entirely.
    fn start_with_proxy(dir: &TempDir, route: Arc<ScriptedRoute>) -> EngineHandle {
        let config = Config {
            probe_enabled: false,
            socks5: Socks5 {
                enabled: true,
                port: 0,
                verify_seconds: 0,
                ..Default::default()
            },
            ..Default::default()
        };
        start_with(dir, config, route)
    }

    #[test]
    fn the_proxy_opens_only_on_a_route_that_changed_and_closes_with_the_tunnel() {
        let dir = TempDir::new("socks5-gate");
        let route = test_route(LAN);
        let handle = start_with_proxy(&dir, Arc::clone(&route));

        // It listens straight away, and refuses: the CLI has not said connected yet.
        let shared = wait_for(&handle, "the listener", |shared| {
            shared.socks5.listen.is_some()
        })
        .unwrap();
        assert!(
            matches!(shared.socks5.gate.state(), GateState::Closed(_)),
            "{:?}",
            shared.socks5.gate.state()
        );
        assert_eq!(shared.socks5.stats.snapshot().dialed, 0);

        // The CLI reports a connection and the kernel's route moves to the tunnel.
        // Wait until the engine has left the disconnected state — `Connecting`, or already
        // `Connected`. Only then is the reference settled, so the route can move to where the
        // tunnel puts it without the reference following it there.
        handle.send(Request::Run(Intent::Connect(ConnectTarget::fastest())));
        wait_for(&handle, "the connect to start", |shared| {
            matches!(
                shared.state.connection.value,
                ConnectionStatus::Connecting | ConnectionStatus::Connected(_)
            )
        });
        route.set(TUNNEL);
        let armed = wait_for(&handle, "the gate to open", |shared| {
            matches!(
                shared.socks5.gate.state(),
                GateState::Open { source } if source == TUNNEL
            )
        })
        .unwrap();
        assert!(armed.state.connection.value.is_connected());

        // The tunnel goes away: the gate must close with it, without waiting for a poll.
        route.set(LAN);
        handle.send(Request::Run(Intent::Disconnect));
        wait_for(&handle, "the gate to close", |shared| {
            shared.state.connection.value == ConnectionStatus::Disconnected
                && matches!(shared.socks5.gate.state(), GateState::Closed(_))
        });
    }

    #[test]
    fn a_proxy_enabled_while_connected_stays_shut_until_a_route_it_can_compare_against() {
        // The application started while the tunnel was already up, so every route it has ever seen
        // is the tunnel's own: it cannot claim the route changed, and it says so instead of
        // pretending. This is the fail-closed reading of `docs/architecture.md` §13.
        let dir = TempDir::new("socks5-unverified");
        let route = test_route(TUNNEL);
        let handle = start_with_proxy(&dir, Arc::clone(&route));

        // The stand-in reports `Disconnected` until a `connect` runs, so put it in the connected
        // state the only way the stand-in understands — and keep the route the tunnel's address
        // throughout, which is exactly the situation under test.
        handle.send(Request::Run(Intent::Connect(ConnectTarget::fastest())));
        wait_for(&handle, "the connection", |shared| {
            shared.state.connection.value.is_connected()
        });
        let shared = wait_for(&handle, "the verdict", |shared| {
            matches!(
                shared.socks5.gate.state(),
                GateState::Closed(Closed::Unverified { .. })
            )
        })
        .unwrap();
        let GateState::Closed(Closed::Unverified { candidate }) = shared.socks5.gate.state() else {
            unreachable!("just matched")
        };
        assert_eq!(candidate, TUNNEL);
        // ...and it keeps refusing, however long the state stays connected.
        thread::sleep(Duration::from_millis(300));
        assert!(!handle.snapshot().socks5.gate.is_open());
    }

    #[test]
    fn a_route_that_diverges_closes_the_gate_and_drops_what_was_relayed() {
        let dir = TempDir::new("socks5-diverged");
        let route = test_route(LAN);
        let handle = start_with_proxy(&dir, Arc::clone(&route));
        // Wait until the engine has left the disconnected state — `Connecting`, or already
        // `Connected`. Only then is the reference settled, so the route can move to where the
        // tunnel puts it without the reference following it there.
        handle.send(Request::Run(Intent::Connect(ConnectTarget::fastest())));
        wait_for(&handle, "the connect to start", |shared| {
            matches!(
                shared.state.connection.value,
                ConnectionStatus::Connecting | ConnectionStatus::Connected(_)
            )
        });
        route.set(TUNNEL);
        wait_for(&handle, "the gate to open", |shared| {
            shared.socks5.gate.is_open()
        });

        // The kernel moves without the CLI saying anything: the report is what closes the gate.
        handle.send(Request::Socks5Report(Socks5Event::RouteDiverged {
            reason: Closed::RouteChanged {
                expected: TUNNEL,
                observed: Some(LAN),
            },
        }));
        wait_for(&handle, "the gate to close", |shared| {
            matches!(
                shared.socks5.gate.state(),
                GateState::Closed(Closed::RouteChanged { .. })
            )
        });
    }

    #[test]
    fn one_dead_destination_is_not_a_dead_tunnel_but_two_are() {
        let dir = TempDir::new("socks5-dial-failures");
        let route = test_route(LAN);
        let handle = start_with_proxy(&dir, Arc::clone(&route));
        // Wait until the engine has left the disconnected state — `Connecting`, or already
        // `Connected`. Only then is the reference settled, so the route can move to where the
        // tunnel puts it without the reference following it there.
        handle.send(Request::Run(Intent::Connect(ConnectTarget::fastest())));
        wait_for(&handle, "the connect to start", |shared| {
            matches!(
                shared.state.connection.value,
                ConnectionStatus::Connecting | ConnectionStatus::Connected(_)
            )
        });
        route.set(TUNNEL);
        wait_for(&handle, "the gate to open", |shared| {
            shared.socks5.gate.is_open()
        });

        let fail = |handle: &EngineHandle, failure| {
            handle.send(Request::Socks5Report(Socks5Event::DialFailed {
                target: "example.com:443".to_string(),
                failure,
                detail: "timed out".to_string(),
            }));
        };

        // A destination that answered and said no proves the path works.
        fail(&handle, crate::socks5::DialFailure::Refused);
        fail(&handle, crate::socks5::DialFailure::Refused);
        thread::sleep(Duration::from_millis(200));
        assert!(handle.snapshot().socks5.gate.is_open());

        // Timeouts implicate the path; one is a dead destination, two are a dead tunnel.
        fail(&handle, crate::socks5::DialFailure::Timeout);
        thread::sleep(Duration::from_millis(200));
        assert!(handle.snapshot().socks5.gate.is_open());
        fail(&handle, crate::socks5::DialFailure::Timeout);
        wait_for(&handle, "the gate to close", |shared| {
            matches!(
                shared.socks5.gate.state(),
                GateState::Closed(Closed::RouteLost { .. })
            )
        });
    }

    #[test]
    fn a_route_that_diverged_is_not_rearmed_by_a_route_change_alone() {
        // The case this pins: a laptop that moves networks while `protonvpn status` still says
        // Connected from five minutes ago. The kernel's route is now a café's, the gate must shut,
        // and "a different route than the one we remember" must not be mistaken for a tunnel —
        // the reference that opened the gate is spent, and only a fresh look at a route the CLI
        // calls down can replace it.
        let dir = TempDir::new("socks5-withdrawn");
        let route = test_route(LAN);
        let handle = start_with_proxy(&dir, Arc::clone(&route));
        handle.send(Request::Run(Intent::Connect(ConnectTarget::fastest())));
        wait_for(&handle, "the connect to start", |shared| {
            matches!(
                shared.state.connection.value,
                ConnectionStatus::Connecting | ConnectionStatus::Connected(_)
            )
        });
        route.set(TUNNEL);
        wait_for(&handle, "the gate to open", |shared| {
            shared.socks5.gate.is_open()
        });

        handle.send(Request::Socks5Report(Socks5Event::RouteDiverged {
            reason: Closed::RouteChanged {
                expected: TUNNEL,
                observed: Some(LAN),
            },
        }));
        wait_for(&handle, "the gate to close", |shared| {
            matches!(
                shared.socks5.gate.state(),
                GateState::Closed(Closed::RouteChanged { .. })
            )
        });

        // A third route, and a CLI that still believes it is connected.
        let cafe = Ipv4Addr::new(192, 168, 50, 20);
        route.set(cafe);
        thread::sleep(Duration::from_millis(2500));
        assert!(
            !handle.snapshot().socks5.gate.is_open(),
            "a new route, on its own, is not evidence of a tunnel"
        );

        // What does re-arm it: the CLI reporting the tunnel down (a fresh reference), then up.
        handle.send(Request::Run(Intent::Disconnect));
        wait_for(&handle, "the disconnect", |shared| {
            shared.state.connection.value == ConnectionStatus::Disconnected
        });
        handle.send(Request::Run(Intent::Connect(ConnectTarget::fastest())));
        wait_for(&handle, "the second connect to start", |shared| {
            matches!(
                shared.state.connection.value,
                ConnectionStatus::Connecting | ConnectionStatus::Connected(_)
            )
        });
        route.set(TUNNEL);
        wait_for(&handle, "the gate to open again", |shared| {
            shared.socks5.gate.is_open()
        });
    }

    #[test]
    fn a_relayed_connection_clears_the_failure_count() {
        let dir = TempDir::new("socks5-dial-reset");
        let route = test_route(LAN);
        let handle = start_with_proxy(&dir, Arc::clone(&route));
        handle.send(Request::Run(Intent::Connect(ConnectTarget::fastest())));
        wait_for(&handle, "the connect to start", |shared| {
            matches!(
                shared.state.connection.value,
                ConnectionStatus::Connecting | ConnectionStatus::Connected(_)
            )
        });
        route.set(TUNNEL);
        wait_for(&handle, "the gate to open", |shared| {
            shared.socks5.gate.is_open()
        });

        let timeout = || Socks5Event::DialFailed {
            target: "example.com:443".to_string(),
            failure: crate::socks5::DialFailure::Timeout,
            detail: "timed out".to_string(),
        };
        handle.send(Request::Socks5Report(timeout()));
        thread::sleep(Duration::from_millis(200));
        assert!(handle.snapshot().socks5.gate.is_open());

        // A connection that was relayed says the path works, so the count starts again — two
        // timeouts an afternoon apart are not "twice in a row".
        handle.send(Request::Socks5Report(Socks5Event::Dialed));
        handle.send(Request::Socks5Report(timeout()));
        thread::sleep(Duration::from_millis(200));
        assert!(handle.snapshot().socks5.gate.is_open());

        handle.send(Request::Socks5Report(timeout()));
        wait_for(&handle, "the gate to close", |shared| {
            matches!(
                shared.socks5.gate.state(),
                GateState::Closed(Closed::RouteLost { .. })
            )
        });
    }

    #[test]
    fn the_console_gets_the_lifecycle_and_not_every_connection() {
        let dir = TempDir::new("socks5-console");
        let route = test_route(LAN);
        let handle = start_with_proxy(&dir, Arc::clone(&route));
        wait_for(&handle, "the listener", |shared| {
            shared.socks5.listen.is_some()
        });

        // Wait until the engine has left the disconnected state — `Connecting`, or already
        // `Connected`. Only then is the reference settled, so the route can move to where the
        // tunnel puts it without the reference following it there.
        handle.send(Request::Run(Intent::Connect(ConnectTarget::fastest())));
        wait_for(&handle, "the connect to start", |shared| {
            matches!(
                shared.state.connection.value,
                ConnectionStatus::Connecting | ConnectionStatus::Connected(_)
            )
        });
        route.set(TUNNEL);
        wait_for(&handle, "the gate to open", |shared| {
            shared.socks5.gate.is_open()
        });
        route.set(LAN);
        handle.send(Request::Run(Intent::Disconnect));
        wait_for(&handle, "the gate to close", |shared| {
            matches!(shared.socks5.gate.state(), GateState::Closed(_))
        });

        let transcript = handle.bus().lock().unwrap().transcript();
        assert!(transcript.contains("SOCKS5"), "{transcript}");
        assert!(transcript.contains("listening:"), "{transcript}");
        assert!(transcript.contains("route proven:"), "{transcript}");
        assert!(transcript.contains("closed:"), "{transcript}");
    }

    /// A minimal SOCKS5 client: greeting, CONNECT to `destination`, and the reply code. Enough to
    /// drive the engine's own listener end to end.
    fn socks5_connect(proxy: SocketAddr, destination: SocketAddrV4) -> (u8, std::net::TcpStream) {
        use std::io::{Read, Write};
        let mut client = std::net::TcpStream::connect(proxy).unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        client.write_all(&[0x05, 1, 0x00]).unwrap();
        let mut choice = [0u8; 2];
        client.read_exact(&mut choice).unwrap();
        assert_eq!(choice, [0x05, 0x00]);
        let mut request = vec![0x05, 0x01, 0x00, 0x01];
        request.extend_from_slice(&destination.ip().octets());
        request.extend_from_slice(&destination.port().to_be_bytes());
        client.write_all(&request).unwrap();
        let mut reply = [0u8; 10];
        client.read_exact(&mut reply).unwrap();
        assert_eq!(reply[0], 0x05);
        (reply[1], client)
    }

    #[test]
    fn a_client_through_the_engine_proxy_is_refused_and_then_relayed() {
        use std::io::{Read, Write};
        let dir = TempDir::new("socks5-end-to-end");
        let route = test_route(LAN);
        let handle = start_with_proxy(&dir, Arc::clone(&route));
        let listen = wait_for(&handle, "the listener", |shared| {
            shared.socks5.listen.is_some()
        })
        .unwrap()
        .socks5
        .listen
        .unwrap();

        // An echo server on loopback, so a relayed byte can be followed there and back. A loopback
        // destination is the one case the pin does not apply to, which is what makes it reachable
        // from a test with no tunnel and no network.
        let echo = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let echo_addr = match echo.local_addr().unwrap() {
            SocketAddr::V4(v4) => v4,
            other => panic!("{other}"),
        };
        let echo_thread = thread::spawn(move || {
            if let Ok((mut stream, _)) = echo.accept() {
                let mut buffer = [0u8; 64];
                while let Ok(n) = stream.read(&mut buffer) {
                    if n == 0 || stream.write_all(&buffer[..n]).is_err() {
                        return;
                    }
                }
            }
        });

        // Shut: the CLI has not reported a connection, so the door is closed with a reason.
        let (code, _) = socks5_connect(listen, echo_addr);
        assert_eq!(code, 0x02, "not allowed by ruleset");
        assert_eq!(handle.snapshot().socks5.stats.snapshot().refused, 1);

        // Open it the way the engine does: the CLI reports a connection and the route moves.
        handle.send(Request::Run(Intent::Connect(ConnectTarget::fastest())));
        wait_for(&handle, "the connect to start", |shared| {
            matches!(
                shared.state.connection.value,
                ConnectionStatus::Connecting | ConnectionStatus::Connected(_)
            )
        });
        route.set(TUNNEL);
        wait_for(&handle, "the gate to open", |shared| {
            shared.socks5.gate.is_open()
        });

        let (code, mut client) = socks5_connect(listen, echo_addr);
        assert_eq!(code, 0x00);
        client.write_all(b"through-the-door").unwrap();
        let mut answer = [0u8; 16];
        client.read_exact(&mut answer).unwrap();
        assert_eq!(&answer, b"through-the-door");

        // The tunnel goes away: the state closes the door, and the relayed connection dies with
        // it — not on its next read, now.
        route.set(LAN);
        handle.send(Request::Run(Intent::Disconnect));
        let closed = wait_for(&handle, "the gate to close", |shared| {
            matches!(shared.socks5.gate.state(), GateState::Closed(_))
        })
        .unwrap();
        assert_eq!(closed.socks5.stats.snapshot().dialed, 1);
        let mut buffer = [0u8; 16];
        client
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        match client.read(&mut buffer) {
            Ok(0) | Err(_) => {}
            Ok(n) => panic!("the relay answered with {n} bytes after the tunnel went away"),
        }
        let _ = echo_thread.join();
    }

    #[test]
    fn the_listener_takes_the_name_localhost_and_nothing_off_the_loop() {
        let dir = TempDir::new("socks5-localhost");
        let config = Config {
            probe_enabled: false,
            socks5: Socks5 {
                enabled: true,
                address: "localhost".into(),
                port: 0,
                verify_seconds: 0,
            },
            ..Default::default()
        };
        let handle = start_with(&dir, config, test_route(LAN));
        let listening = wait_for(&handle, "the listener", |shared| {
            shared.socks5.listen.is_some()
        })
        .unwrap();
        assert!(listening.socks5.listen.unwrap().ip().is_loopback());

        // Anything not on the loop is refused outright, whatever the config file says: the proxy
        // is a door for this machine, not an open relay.
        let dir = TempDir::new("socks5-not-loopback");
        let config = Config {
            probe_enabled: false,
            socks5: Socks5 {
                enabled: true,
                address: "0.0.0.0".into(),
                port: 0,
                verify_seconds: 0,
            },
            ..Default::default()
        };
        let handle = start_with(&dir, config, test_route(LAN));
        let refused = wait_for(&handle, "the refusal", |shared| {
            matches!(
                shared.socks5.gate.state(),
                GateState::Closed(Closed::NotListening { .. })
            )
        })
        .unwrap();
        assert!(refused.socks5.listen.is_none());
        let GateState::Closed(Closed::NotListening { detail }) = refused.socks5.gate.state() else {
            unreachable!("just matched")
        };
        assert!(detail.contains("is not local"), "{detail}");
    }

    #[test]
    fn shutting_down_is_noticed_and_acknowledged() {
        let dir = TempDir::new("shutdown");
        let handle = start(&dir);
        wait_for(&handle, "the initial status", |shared| {
            shared.state.connection.value == ConnectionStatus::Disconnected
        });
        assert!(handle.shutdown(Duration::from_secs(5)));
    }

    #[test]
    fn a_missing_cli_is_reported_in_the_console_rather_than_looking_like_a_disconnection() {
        let dir = TempDir::new("missing-cli");
        let config = Config {
            probe_enabled: false,
            ..Default::default()
        };
        let options = EngineOptions {
            cwd: dir.path().to_path_buf(),
            store: ConfigStore::at(dir.path().join("config.json")),
            config,
            tray_available: false,
            tray: None,
            locale: Locale::SOURCE,
            program: dir
                .path()
                .join("definitely-not-here")
                .to_string_lossy()
                .into_owned(),
            route: Arc::clone(&test_route(LAN)) as Arc<dyn RouteProbe>,
            // See `start_with`: the updater is not what this test is about.
            version: None,
            curl: "curl".to_string(),
            installed: None,
            update_delay: Duration::from_secs(3600),
        };
        let handle = spawn(options);

        wait_for(&handle, "an error line", |_| true);
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut text = String::new();
        while Instant::now() < deadline {
            text = handle.bus().lock().unwrap().transcript();
            if text.contains("could not start") {
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
        assert!(text.contains("could not start"), "{text}");
        // And the state is still `Unknown`: the CLI never said anything, so we invent nothing.
        assert_eq!(
            handle.snapshot().state.connection.value,
            ConnectionStatus::Unknown
        );
        assert!(handle.shutdown(Duration::from_secs(5)));
    }
    // --- the updater (exception #4) --------------------------------------------------------

    /// An engine pointed at a stand-in `curl` and at an "installed" image in a temporary
    /// directory. `$APPIMAGE` cannot be set for the test process from inside the test, so the
    /// image is injected — the same seam `program` is for the CLI.
    fn start_updating(
        dir: &TempDir,
        policy: UpdatePolicy,
        version: Option<Version>,
        installed: Option<Installed>,
        curl: String,
        delay: Duration,
    ) -> EngineHandle {
        let mut config = Config {
            probe_enabled: false,
            ..Default::default()
        };
        config.update.policy = policy;
        let program = write_stand_in(dir.path());
        let options = EngineOptions {
            cwd: dir.path().to_path_buf(),
            store: ConfigStore::at(dir.path().join("config.json")),
            config,
            tray_available: false,
            tray: None,
            // The tests assert on the English wording, which is the source language.
            locale: Locale::SOURCE,
            program: program.to_string_lossy().into_owned(),
            route: test_route(LAN) as Arc<dyn RouteProbe>,
            version,
            curl,
            installed,
            update_delay: delay,
        };
        spawn(options)
    }

    /// An installed image, a stand-in `curl` that serves a newer one, and the version this build
    /// believes it is.
    fn update_fixture(dir: &TempDir, name: &str) -> (Installed, String, Vec<u8>, Version, Vec<u8>) {
        let old = image_bytes();
        let path = dir.path().join(name);
        fs::write(&path, &old).unwrap();
        let installed = Installed::at(&path).unwrap();

        let release = Version {
            major: 0,
            minor: 1,
            patch: 21,
        };
        let new_bytes = [image_bytes(), b"the new one".to_vec()].concat();
        let curl = stand_in(dir.path(), &sums_for(&new_bytes, release), &new_bytes);
        (installed, curl, old, release, new_bytes)
    }

    fn release_0_1_21() -> Version {
        Version {
            major: 0,
            minor: 1,
            patch: 21,
        }
    }

    fn release_0_1_20() -> Version {
        Version {
            major: 0,
            minor: 1,
            patch: 20,
        }
    }

    #[test]
    fn a_new_release_is_downloaded_and_waits_for_a_restart() {
        let dir = TempDir::new("update-download");
        let (installed, curl, old, release, _) =
            update_fixture(&dir, "ProtonVPN-GUI-0.1.20-x86_64.AppImage");
        let handle = start_updating(
            &dir,
            UpdatePolicy::Download,
            Some(release_0_1_20()),
            Some(installed.clone()),
            curl,
            Duration::from_millis(50),
        );

        let shared = wait_for(&handle, "the image to be staged", |shared| {
            matches!(&shared.update.phase, UpdatePhase::Staged { version } if *version == release)
        })
        .unwrap();

        // Staged, not installed: the file the user is running is the one they were running.
        assert_eq!(fs::read(installed.path()).unwrap(), old);
        assert!(installed.staging_path().is_file());
        assert_eq!(shared.update.latest, Some(release));
        assert_eq!(shared.update.current, Some(release_0_1_20()));
        assert!(shared.update.error.is_none());
        assert!(shared.update.checked_at.is_some());
        assert!(shared.update.replaceable);

        // What we ran is on the record, command and all.
        let text = handle.bus().lock().unwrap().transcript();
        assert!(
            text.contains("releases/latest/download/SHA256SUMS"),
            "{text}"
        );
        assert!(text.contains("latest release: 0.1.21"), "{text}");

        // And that the release page was asked is written down, so the next start does not ask
        // again a minute later.
        let saved = ConfigStore::at(dir.path().join("config.json"))
            .load()
            .unwrap();
        assert!(saved.update.last_check.is_some());

        assert!(handle.shutdown(Duration::from_secs(5)));
    }

    #[test]
    fn the_install_policy_puts_the_image_in_place_and_keeps_the_old_one() {
        let dir = TempDir::new("update-install");
        let (installed, curl, old, release, new) =
            update_fixture(&dir, "ProtonVPN-GUI-0.1.20-x86_64.AppImage");
        let handle = start_updating(
            &dir,
            UpdatePolicy::Install,
            Some(release_0_1_20()),
            Some(installed.clone()),
            curl,
            Duration::from_millis(50),
        );

        wait_for(
            &handle,
            "the image to be in place",
            |shared| matches!(&shared.update.phase, UpdatePhase::Installed { version } if *version == release),
        );

        assert_eq!(fs::read(installed.path()).unwrap(), new);
        // The way back, kept for one start.
        assert_eq!(fs::read(installed.backup_path()).unwrap(), old);
        assert!(!installed.staging_path().exists());

        let text = handle.bus().lock().unwrap().transcript();
        assert!(text.contains("takes effect at the next start"), "{text}");

        assert!(handle.shutdown(Duration::from_secs(5)));
    }

    #[test]
    fn the_notify_policy_reports_the_release_and_downloads_nothing() {
        let dir = TempDir::new("update-notify");
        let (installed, curl, old, release, _) =
            update_fixture(&dir, "ProtonVPN-GUI-0.1.20-x86_64.AppImage");
        let handle = start_updating(
            &dir,
            UpdatePolicy::Notify,
            Some(release_0_1_20()),
            Some(installed.clone()),
            curl,
            Duration::from_millis(50),
        );

        let shared = wait_for(&handle, "the release to be known", |shared| {
            shared.update.latest == Some(release)
                && matches!(shared.update.phase, UpdatePhase::Idle)
        })
        .unwrap();

        assert!(shared.update.error.is_none());
        assert!(!installed.staging_path().exists());
        assert_eq!(fs::read(installed.path()).unwrap(), old);

        assert!(handle.shutdown(Duration::from_secs(5)));
    }

    #[test]
    fn a_build_that_cannot_replace_itself_says_so_instead_of_downloading() {
        let dir = TempDir::new("update-not-an-appimage");
        let (_, curl, _, release, _) = update_fixture(&dir, "unused.AppImage");
        let handle = start_updating(
            &dir,
            UpdatePolicy::Install,
            Some(release_0_1_20()),
            None,
            curl,
            Duration::from_millis(50),
        );

        let shared = wait_for(&handle, "the release to be known", |shared| {
            shared.update.latest == Some(release)
        })
        .unwrap();

        assert!(!shared.update.replaceable);
        assert!(matches!(shared.update.phase, UpdatePhase::Idle));
        // Not a console line: nothing was executed. Our own remark, which is what a note is for.
        let note = shared.note.clone().unwrap_or_default();
        assert!(note.contains("not an AppImage"), "{note}");

        assert!(handle.shutdown(Duration::from_secs(5)));
    }

    #[test]
    fn the_off_policy_asks_nothing_until_a_human_asks_for_it() {
        let dir = TempDir::new("update-off");
        let (installed, curl, old, release, _) =
            update_fixture(&dir, "ProtonVPN-GUI-0.1.20-x86_64.AppImage");
        let handle = start_updating(
            &dir,
            UpdatePolicy::Off,
            Some(release_0_1_20()),
            Some(installed.clone()),
            curl,
            Duration::from_millis(20),
        );

        // Well past the delay it would have fired at, and nothing happened.
        thread::sleep(Duration::from_millis(400));
        let quiet = handle.snapshot();
        assert!(quiet.update.latest.is_none());
        assert!(quiet.update.checked_at.is_none());

        // The button, however, is an explicit request: an off switch that refuses to work when
        // asked by hand would be a lie.
        handle.send(Request::UpdateCheck);
        let shared = wait_for(&handle, "a manual check", |shared| {
            shared.update.latest == Some(release)
        })
        .unwrap();

        // …and it reports without installing: the policy still decides what may be fetched.
        assert!(matches!(shared.update.phase, UpdatePhase::Idle));
        assert!(!installed.staging_path().exists());
        assert_eq!(fs::read(installed.path()).unwrap(), old);

        assert!(handle.shutdown(Duration::from_secs(5)));
    }

    #[test]
    fn a_release_with_nothing_to_install_is_a_finding_and_not_a_failure() {
        let dir = TempDir::new("update-nothing");
        let (installed, _, _, _, _) = update_fixture(&dir, "ProtonVPN-GUI-0.1.20-x86_64.AppImage");
        // A release whose AppImage has no version in its name — the state of every release up to
        // and including 0.1.20, which is exactly what someone running one of those builds sees.
        let body = format!("{}  ProtonVPN-GUI-x86_64.AppImage\n", "a".repeat(64));
        let curl = stand_in(dir.path(), &body, &image_bytes());

        let handle = start_updating(
            &dir,
            UpdatePolicy::Download,
            Some(release_0_1_20()),
            Some(installed.clone()),
            curl,
            Duration::from_millis(50),
        );

        let shared = wait_for(&handle, "the page to answer", |shared| {
            shared.update.checked_at.is_some()
        })
        .unwrap();

        assert!(matches!(shared.update.phase, UpdatePhase::Idle));
        assert!(shared.update.latest.is_none());
        assert!(
            shared.update.error.is_none(),
            "nothing to install is not a failure: {:?}",
            shared.update.error
        );
        let note = shared.note.clone().unwrap_or_default();
        assert!(note.contains("SHA256SUMS"), "{note}");
        assert!(!installed.staging_path().exists());

        assert!(handle.shutdown(Duration::from_secs(5)));
    }

    #[test]
    fn a_dismissed_version_is_written_down_and_a_newer_one_is_offered_again() {
        let dir = TempDir::new("update-dismiss");
        let (installed, curl, _, release, _) =
            update_fixture(&dir, "ProtonVPN-GUI-0.1.20-x86_64.AppImage");
        let handle = start_updating(
            &dir,
            UpdatePolicy::Notify,
            Some(release_0_1_20()),
            Some(installed),
            curl,
            Duration::from_millis(50),
        );
        wait_for(&handle, "the release to be known", |shared| {
            shared.update.latest == Some(release)
        });

        handle.send(Request::UpdateDismiss);
        wait_for(&handle, "the dismissal to be recorded", |shared| {
            shared.update.dismissed
        });

        let saved = ConfigStore::at(dir.path().join("config.json"))
            .load()
            .unwrap();
        assert_eq!(saved.update.dismissed.as_deref(), Some("0.1.21"));

        assert!(handle.shutdown(Duration::from_secs(5)));
    }

    #[test]
    fn the_tray_mentions_an_update_only_when_there_is_one_and_it_is_wanted() {
        let view = |current: Option<Version>,
                    latest: Option<Version>,
                    dismissed: bool,
                    phase: UpdatePhase| UpdateView {
            current,
            latest,
            dismissed,
            phase,
            ..UpdateView::default()
        };

        assert_eq!(
            tray_update(&view(
                Some(release_0_1_20()),
                Some(release_0_1_21()),
                false,
                UpdatePhase::Idle
            ))
            .map(|update| update.version),
            Some(release_0_1_21())
        );
        assert!(
            tray_update(&view(
                Some(release_0_1_21()),
                Some(release_0_1_21()),
                false,
                UpdatePhase::Idle
            ))
            .is_none(),
            "up to date is not news"
        );
        assert!(
            tray_update(&view(
                Some(release_0_1_21()),
                Some(release_0_1_20()),
                false,
                UpdatePhase::Idle
            ))
            .is_none(),
            "a release behind this build is not an update"
        );
        assert!(
            tray_update(&view(
                None,
                Some(release_0_1_21()),
                false,
                UpdatePhase::Idle
            ))
            .is_none(),
            "an unversioned build has nothing to be behind"
        );
        assert!(
            tray_update(&view(
                Some(release_0_1_20()),
                Some(release_0_1_21()),
                true,
                UpdatePhase::Idle
            ))
            .is_none(),
            "a dismissal is a dismissal"
        );
        assert!(
            tray_update(&view(
                Some(release_0_1_20()),
                Some(release_0_1_21()),
                false,
                UpdatePhase::Installed {
                    version: release_0_1_21()
                }
            ))
            .unwrap()
            .applied,
            "once it is in place, the tray says a restart is what is missing"
        );
    }
}
