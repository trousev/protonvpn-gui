//! A local SOCKS5 proxy that only exists while the tunnel does — sanctioned exception #3
//! (`docs/architecture.md` §13).
//!
//! The idea is a **paranoid option**, not a general-purpose proxy: an application that must never
//! touch the network without the VPN gets pointed at `127.0.0.1:1080`, and the proxy refuses to
//! relay anything it cannot prove is going through the tunnel. Off by default, loopback only,
//! IPv4 only (an IPv6 hop we cannot pin is refused rather than guessed at), and it speaks exactly
//! one SOCKS5 command — `CONNECT`.
//!
//! How "prove" works, without knowing anything about how Proton connects:
//!
//! 1. **The gate.** The engine opens it only when the CLI reports `Connected` *and* the kernel's
//!    source address for off-link traffic differs from an address observed while it was not
//!    connected ([`crate::net::route`]). "The route is different from the one we had before the
//!    VPN" is the same evidence the ground-truth probe uses for the egress address
//!    (`docs/architecture.md` §8), applied to the local route.
//! 2. **The pin.** The gate is pinned to the source address the kernel chose while the tunnel was
//!    up, and every dial must come from it. The standard library cannot bind a source address
//!    before connecting, so the pin is enforced twice: the route is re-read immediately before the
//!    dial, and the socket's own `local_addr` is checked immediately after it — before a single
//!    byte of the application's is relayed. What that leaves is a TCP handshake in the
//!    microseconds between the two checks: no payload, and it is detected and reported.
//! 3. **The watchdog.** While the gate is open, the route is re-read every 200 ms — no packets, no
//!    third party, one syscall. Divergence is reported to the engine, which closes the gate and
//!    drops every relayed connection.
//!
//! What this module is *not*: it does not run anything, it does not read NetworkManager, D-Bus or
//! a Proton file, and it does not decide what "connected" means — the CLI does that, as always.

use std::collections::HashMap;
use std::fmt;
use std::io::{self, Read, Write};
use std::net::{
    IpAddr, Ipv4Addr, Shutdown, SocketAddr, SocketAddrV4, TcpListener, TcpStream, ToSocketAddrs,
};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::i18n::I18n;
use crate::net::route::RouteProbe;

/// How long a client has to complete the SOCKS5 handshake before we give up on it.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
/// How long a dial to the destination may take. A tunnel that is up but dead shows up here.
const DIAL_TIMEOUT: Duration = Duration::from_secs(10);
/// How often the watchdog re-reads the kernel's route while the gate is open. Four syscalls, no
/// packets: the cost of being fast is effectively zero.
const WATCH_INTERVAL: Duration = Duration::from_millis(200);
/// How often the watchdog looks for commands. The engine's "drop everything" must not wait.
const WATCH_TICK: Duration = Duration::from_millis(50);
/// Relay and session threads copy bytes; they do not need a megabyte of stack each.
const STACK: usize = 256 * 1024;
/// How many sessions may exist at once. A desktop application opens tens of connections; anything
/// past this is a loop, and a loop deserves a refusal rather than a thread storm.
const MAX_SESSIONS: i64 = 256;
/// How long the stop path waits for its own listener to answer. It only has to reach the accept
/// queue, so it is measured in milliseconds and must never be unbounded.
const WAKE_TIMEOUT: Duration = Duration::from_millis(200);

const SOCKS5: u8 = 0x05;
const METHOD_NO_AUTH: u8 = 0x00;
const METHOD_NONE: u8 = 0xFF;
const CMD_CONNECT: u8 = 0x01;
const ATYP_IPV4: u8 = 0x01;
const ATYP_DOMAIN: u8 = 0x03;
const ATYP_IPV6: u8 = 0x04;

// --- the gate -------------------------------------------------------------------------------

/// Why the gate is closed. Every variant is a fact about evidence, never a verdict about the CLI
/// (`docs/architecture.md` §5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Closed {
    /// The proxy is not enabled in our own configuration.
    Disabled,
    /// The CLI does not report a connection, so there is nothing to pin.
    NotConnected,
    /// The CLI says connected, but this route has never been seen to differ from a route observed
    /// while it was not: we cannot claim it is the tunnel, so we claim nothing.
    Unverified { candidate: Ipv4Addr },
    /// The gate was open and the kernel now chooses something else.
    RouteChanged {
        expected: Ipv4Addr,
        observed: Option<Ipv4Addr>,
    },
    /// The route or the pinned address is gone; `detail` is the kernel's own complaint.
    RouteLost { detail: String },
    /// The ground-truth probe reports the pre-connection egress address again: the tunnel is not
    /// carrying traffic, whatever the CLI says.
    EgressIsBaseline { ip: IpAddr },
    /// Enabled, but there is no listener: the address is not loopback, or the port is taken.
    NotListening { detail: String },
    /// The background tunnel check stopped answering. Not a statement about the route — a failed
    /// check is not evidence, and saying "the route is lost" would be a verdict we did not earn
    /// (`docs/architecture.md` §5).
    ProbeUnanswered { detail: String },
}

impl Closed {
    /// One line for the console and the settings page.
    ///
    /// It takes a catalogue rather than implementing `Display` because it is a *sentence*, shown to
    /// a person, and the two callers — the engine's console note and the settings card — are the
    /// only places it is read. An address inside it is data and is never translated.
    pub fn describe(&self, i18n: &I18n) -> String {
        match self {
            Self::Disabled => i18n.proxy_gate_disabled(),
            Self::NotConnected => i18n.proxy_gate_not_connected(),
            Self::Unverified { candidate } => i18n.proxy_gate_unverified(candidate.to_string()),
            Self::RouteChanged { expected, observed } => match observed {
                Some(observed) => {
                    i18n.proxy_gate_route_changed(expected.to_string(), observed.to_string())
                }
                None => i18n.proxy_gate_route_gone(expected.to_string()),
            },
            Self::RouteLost { detail } => i18n.proxy_gate_route_lost(detail),
            Self::EgressIsBaseline { ip } => i18n.proxy_gate_egress_baseline(ip.to_string()),
            Self::NotListening { detail } => i18n.proxy_gate_not_listening(detail),
            Self::ProbeUnanswered { detail } => i18n.proxy_gate_probe_unanswered(detail),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateState {
    Closed(Closed),
    /// Relaying is allowed, from `source`, and only while the kernel keeps choosing it.
    Open {
        source: Ipv4Addr,
    },
}

/// The one piece of state the engine writes and the proxy only reads.
///
/// The proxy deliberately **cannot open or close it**: it reports what it observes
/// ([`Socks5Event`]) and refuses dials on its own, but the decision to claim a tunnel stays in the
/// one thread that owns state (`docs/architecture.md` §1).
#[derive(Clone)]
pub struct TunnelGate {
    route: Arc<dyn RouteProbe>,
    state: Arc<Mutex<GateState>>,
}

impl TunnelGate {
    pub fn new(route: Arc<dyn RouteProbe>) -> Self {
        Self {
            route,
            state: Arc::new(Mutex::new(GateState::Closed(Closed::Disabled))),
        }
    }

    /// Opening and closing are the engine's business alone (`docs/architecture.md` §13.3): the
    /// proxy reads the gate and reports, so `pub(crate)` is the whole enforcement of that rule.
    pub(crate) fn open(&self, source: Ipv4Addr) {
        *self.lock() = GateState::Open { source };
    }

    pub(crate) fn close(&self, reason: Closed) {
        *self.lock() = GateState::Closed(reason);
    }

    pub fn state(&self) -> GateState {
        self.lock().clone()
    }

    pub fn is_open(&self) -> bool {
        matches!(*self.lock(), GateState::Open { .. })
    }

    /// The address a dial must be bound to, or the reason it must not happen at all.
    ///
    /// This re-reads the kernel's route on every call, so the answer cannot be older than the
    /// dial it authorises.
    pub fn permit(&self) -> Result<Ipv4Addr, Closed> {
        match self.state() {
            GateState::Closed(reason) => Err(reason),
            GateState::Open { source } => match self.route.source() {
                Ok(observed) if observed == source => Ok(source),
                Ok(observed) => Err(Closed::RouteChanged {
                    expected: source,
                    observed: Some(observed),
                }),
                Err(error) => Err(Closed::RouteLost {
                    detail: error.to_string(),
                }),
            },
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, GateState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl fmt::Debug for TunnelGate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "TunnelGate({:?})", self.state())
    }
}

/// Compared by value, so a view that keeps a snapshot can tell that the gate moved.
impl PartialEq for TunnelGate {
    fn eq(&self, other: &Self) -> bool {
        self.state() == other.state()
    }
}

impl Eq for TunnelGate {}

// --- counters -------------------------------------------------------------------------------

/// What the proxy has done since the process started. Atomics, because the sessions are threads
/// and the views read a snapshot; the engine is not in this path on purpose.
#[derive(Debug, Default)]
pub struct Stats {
    pub accepted: AtomicU64,
    /// Refused because the gate was closed — the number a paranoid user actually wants.
    pub refused: AtomicU64,
    /// A method, a command or an address type we do not speak.
    pub unsupported: AtomicU64,
    /// Upstream connections established.
    pub dialed: AtomicU64,
    /// Dials that failed, whatever the reason.
    pub failed: AtomicU64,
    /// Connections being relayed right now.
    pub active: AtomicI64,
    /// Sessions in flight, handshake included.
    pub sessions: AtomicI64,
    /// Refused for want of room — a cap, or a thread we could not spawn. Deliberately not folded
    /// into `refused`: that number is the gate's, and a paranoid user reads it as one.
    pub overloaded: AtomicU64,
    pub up: AtomicU64,
    pub down: AtomicU64,
}

/// The counters as plain numbers, for rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StatsSnapshot {
    pub accepted: u64,
    pub refused: u64,
    pub unsupported: u64,
    pub dialed: u64,
    pub failed: u64,
    pub active: i64,
    pub sessions: i64,
    pub overloaded: u64,
    pub up: u64,
    pub down: u64,
}

impl Stats {
    pub fn snapshot(&self) -> StatsSnapshot {
        StatsSnapshot {
            accepted: self.accepted.load(Ordering::Relaxed),
            refused: self.refused.load(Ordering::Relaxed),
            unsupported: self.unsupported.load(Ordering::Relaxed),
            dialed: self.dialed.load(Ordering::Relaxed),
            failed: self.failed.load(Ordering::Relaxed),
            active: self.active.load(Ordering::Relaxed),
            sessions: self.sessions.load(Ordering::Relaxed),
            overloaded: self.overloaded.load(Ordering::Relaxed),
            up: self.up.load(Ordering::Relaxed),
            down: self.down.load(Ordering::Relaxed),
        }
    }
}

/// Compared by value: `Shared` is diffed by the views to decide whether to repaint.
impl PartialEq for Stats {
    fn eq(&self, other: &Self) -> bool {
        self.snapshot() == other.snapshot()
    }
}

impl Eq for Stats {}

// --- the server -----------------------------------------------------------------------------

/// Where the listener goes. Loopback only: the proxy is for applications on this machine, and an
/// open relay is not a feature.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Socks5Options {
    pub bind: Ipv4Addr,
    pub port: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Socks5Error {
    /// The address is not loopback. Refused outright, whatever the config says.
    NotLoopback(String),
    Bind(String),
}

impl fmt::Display for Socks5Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotLoopback(address) => write!(
                f,
                "адрес `{address}` не является локальным: прокси слушает только localhost"
            ),
            Self::Bind(error) => write!(f, "не удалось занять порт: {error}"),
        }
    }
}

impl std::error::Error for Socks5Error {}

/// What the proxy tells the engine. Lifecycle only: a refusal per connection would drown the
/// console, and the console is the product.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Socks5Event {
    /// The kernel no longer chooses the address the gate is pinned to.
    RouteDiverged { reason: Closed },
    /// A dial succeeded but came from somewhere other than the pinned address: the route moved in
    /// the microseconds between the check and the connect. Nothing of the application's was
    /// relayed, and the socket is dropped.
    SourceMismatch {
        expected: Ipv4Addr,
        observed: Ipv4Addr,
    },
    /// A dial was established: the path works. The engine counts *consecutive* failures, so it
    /// has to hear about the successes too.
    Dialed,
    /// A dial failed. The engine counts the ones that say the path itself is gone.
    DialFailed {
        target: String,
        failure: DialFailure,
        detail: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialFailure {
    /// The destination answered and said no. The path is fine.
    Refused,
    Timeout,
    Unreachable,
    Other,
}

impl DialFailure {
    /// Does this failure say something about the *path*, rather than about the destination?
    pub fn implicates_the_path(self) -> bool {
        matches!(self, Self::Timeout | Self::Unreachable)
    }
}

enum Command {
    DropConnections,
    Stop,
}

/// Where the proxy's threads report. A callback rather than a channel, so the engine can route
/// reports into its own request queue — and a test can watch them directly.
pub type Reporter = Arc<dyn Fn(Socks5Event) + Send + Sync>;

/// A running proxy. Dropping it stops the listener and drops every relayed connection.
pub struct Socks5 {
    addr: SocketAddr,
    commands: Sender<Command>,
    stop: Arc<AtomicBool>,
    registry: Arc<Registry>,
    threads: Vec<thread::JoinHandle<()>>,
}

impl Socks5 {
    /// Binds the listener and starts serving. `stats` outlives the handle so counters survive a
    /// restart of the server on another port.
    pub fn spawn(
        options: Socks5Options,
        gate: TunnelGate,
        stats: Arc<Stats>,
        events: Reporter,
    ) -> Result<Self, Socks5Error> {
        if !options.bind.is_loopback() {
            return Err(Socks5Error::NotLoopback(options.bind.to_string()));
        }
        let listener = TcpListener::bind(SocketAddrV4::new(options.bind, options.port))
            .map_err(|error| Socks5Error::Bind(error.to_string()))?;
        let addr = listener
            .local_addr()
            .map_err(|error| Socks5Error::Bind(error.to_string()))?;

        let (commands, command_rx) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let registry = Arc::new(Registry::default());
        let mut threads = Vec::new();

        threads.push(
            thread::Builder::new()
                .name("protonvpn-socks5".into())
                .stack_size(STACK)
                .spawn({
                    let (gate, stats, registry, events, stop) = (
                        gate.clone(),
                        Arc::clone(&stats),
                        Arc::clone(&registry),
                        events.clone(),
                        Arc::clone(&stop),
                    );
                    move || accept_loop(listener, stop, gate, stats, registry, events)
                })
                .map_err(|error| Socks5Error::Bind(error.to_string()))?,
        );

        threads.push(
            thread::Builder::new()
                .name("protonvpn-socks5-watch".into())
                .stack_size(STACK)
                .spawn({
                    let (gate, registry, stop) =
                        (gate.clone(), Arc::clone(&registry), Arc::clone(&stop));
                    move || watch_loop(addr, stop, gate, registry, command_rx, events)
                })
                .map_err(|error| Socks5Error::Bind(error.to_string()))?,
        );

        Ok(Self {
            addr,
            commands,
            stop,
            registry,
            threads,
        })
    }

    /// `127.0.0.1:1080` (or whatever port the kernel handed out when the config asked for 0).
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Tears down every relayed connection. The gate is what stops new ones.
    pub fn drop_connections(&self) {
        let _ = self.commands.send(Command::DropConnections);
    }
}

impl fmt::Debug for Socks5 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Socks5({})", self.addr)
    }
}

impl Drop for Socks5 {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = self.commands.send(Command::Stop);
        self.registry.drop_all();
        // The accept thread is woken by a connection to our own listener; the watchdog notices the
        // flag within one tick. Joining keeps a restart on the same port from racing the old
        // listener for it.
        let _ = TcpStream::connect_timeout(&self.addr, WAKE_TIMEOUT);
        for handle in self.threads.drain(..) {
            let _ = handle.join();
        }
    }
}

/// Relay sockets, so a closed gate can tear down connections that are already up. Bound sockets
/// belong to the tunnel's address; when it goes away the kernel breaks them, but "breaks
/// eventually" is not the same promise as "dropped now".
#[derive(Default)]
struct Registry {
    sockets: Mutex<HashMap<u64, (TcpStream, TcpStream)>>,
    next: AtomicU64,
}

impl Registry {
    fn register(&self, client: &TcpStream, upstream: &TcpStream) -> u64 {
        let id = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        if let (Ok(client), Ok(upstream)) = (client.try_clone(), upstream.try_clone()) {
            self.lock().insert(id, (client, upstream));
        }
        id
    }

    fn forget(&self, id: u64) {
        self.lock().remove(&id);
    }

    /// Removes one relay and closes it. Used when a disarm lands between a dial and its first
    /// relayed byte: either the close saw the registration or we see the closed gate, and between
    /// the two there is no moment where a connection is live under a shut door.
    fn drop_one(&self, id: u64) {
        if let Some((client, upstream)) = self.lock().remove(&id) {
            let _ = client.shutdown(Shutdown::Both);
            let _ = upstream.shutdown(Shutdown::Both);
        }
    }

    fn drop_all(&self) {
        for (_, (client, upstream)) in self.lock().drain() {
            let _ = client.shutdown(Shutdown::Both);
            let _ = upstream.shutdown(Shutdown::Both);
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<u64, (TcpStream, TcpStream)>> {
        self.sockets
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn accept_loop(
    listener: TcpListener,
    stop: Arc<AtomicBool>,
    gate: TunnelGate,
    stats: Arc<Stats>,
    registry: Arc<Registry>,
    events: Reporter,
) {
    loop {
        match listener.accept() {
            Ok((client, _)) => {
                if stop.load(Ordering::SeqCst) {
                    let _ = client.shutdown(Shutdown::Both);
                    return;
                }
                if stats.sessions.load(Ordering::Relaxed) >= MAX_SESSIONS {
                    stats.overloaded.fetch_add(1, Ordering::Relaxed);
                    let _ = client.shutdown(Shutdown::Both);
                    continue;
                }
                stats.accepted.fetch_add(1, Ordering::Relaxed);
                let session_stats = Arc::clone(&stats);
                let (gate, registry, events) =
                    (gate.clone(), Arc::clone(&registry), events.clone());
                let spawned = thread::Builder::new()
                    .name("protonvpn-socks5-session".into())
                    .stack_size(STACK)
                    .spawn(move || {
                        let _session = Gauge::enter(&session_stats.sessions);
                        session(client, &gate, &session_stats, &registry, &events);
                    });
                if spawned.is_err() {
                    // Not a gate refusal: there was no room to serve the connection at all. The
                    // client socket moved into the closure, so the failed spawn closes it.
                    stats.overloaded.fetch_add(1, Ordering::Relaxed);
                }
            }
            Err(_) if stop.load(Ordering::SeqCst) => return,
            Err(_) => thread::sleep(WATCH_TICK),
        }
    }
}

/// The fast half of the fail-closed promise: no packets, no third party, one route read every
/// [`WATCH_INTERVAL`]. It reports; the engine decides.
fn watch_loop(
    addr: SocketAddr,
    stop: Arc<AtomicBool>,
    gate: TunnelGate,
    registry: Arc<Registry>,
    commands: Receiver<Command>,
    events: Reporter,
) {
    let mut reported: Option<Closed> = None;
    let mut last_check = std::time::Instant::now();
    loop {
        if stop.load(Ordering::SeqCst) {
            registry.drop_all();
            return;
        }
        match commands.try_recv() {
            Ok(Command::DropConnections) => registry.drop_all(),
            Ok(Command::Stop) | Err(TryRecvError::Disconnected) => {
                stop.store(true, Ordering::SeqCst);
                registry.drop_all();
                // Wake the accept thread, which is parked in `accept()`.
                let _ = TcpStream::connect(addr);
                return;
            }
            Err(TryRecvError::Empty) => {}
        }

        if last_check.elapsed() >= WATCH_INTERVAL {
            last_check = std::time::Instant::now();
            if gate.is_open() {
                match gate.permit() {
                    Ok(_) => reported = None,
                    Err(reason) => {
                        // One report per divergence, not one per tick: the engine's answer is the
                        // close, and until it arrives we simply keep refusing dials.
                        if reported.as_ref() != Some(&reason) {
                            reported = Some(reason.clone());
                            (events)(Socks5Event::RouteDiverged { reason });
                        }
                    }
                }
            } else {
                reported = None;
            }
        }

        thread::sleep(WATCH_TICK);
    }
}

// --- one connection -------------------------------------------------------------------------

fn session(
    mut client: TcpStream,
    gate: &TunnelGate,
    stats: &Arc<Stats>,
    registry: &Arc<Registry>,
    events: &Reporter,
) {
    let _ = client.set_nodelay(true);
    let _ = client.set_read_timeout(Some(HANDSHAKE_TIMEOUT));

    if greet(&mut client).is_err() {
        stats.unsupported.fetch_add(1, Ordering::Relaxed);
        return;
    }

    let request = match read_request(&mut client) {
        Ok(request) => request,
        Err(RequestError::Io) => return,
        Err(RequestError::AddressType) => {
            stats.unsupported.fetch_add(1, Ordering::Relaxed);
            let _ = write_reply(
                &mut client,
                Reply::AddressTypeNotSupported,
                Ipv4Addr::UNSPECIFIED,
                0,
            );
            return;
        }
        Err(RequestError::Malformed) => {
            stats.unsupported.fetch_add(1, Ordering::Relaxed);
            let _ = write_reply(&mut client, Reply::GeneralFailure, Ipv4Addr::UNSPECIFIED, 0);
            return;
        }
    };

    if request.command != CMD_CONNECT {
        stats.unsupported.fetch_add(1, Ordering::Relaxed);
        let _ = write_reply(
            &mut client,
            Reply::CommandNotSupported,
            Ipv4Addr::UNSPECIFIED,
            0,
        );
        return;
    }

    // The gate comes first, before a single packet: not even a DNS lookup happens for an
    // application that is not allowed to be online.
    let source = match gate.permit() {
        Ok(source) => source,
        Err(_reason) => {
            // The watchdog is the reporter: it holds one report per divergence, and a client
            // retrying in a loop must not be able to fill the engine's queue with identical news.
            stats.refused.fetch_add(1, Ordering::Relaxed);
            let _ = write_reply(&mut client, Reply::NotAllowed, Ipv4Addr::UNSPECIFIED, 0);
            return;
        }
    };

    let target = request.host.describe();
    let destination = match resolve(&request.host, request.port) {
        Some(destination) => destination,
        None => {
            stats.failed.fetch_add(1, Ordering::Relaxed);
            let _ = write_reply(
                &mut client,
                Reply::HostUnreachable,
                Ipv4Addr::UNSPECIFIED,
                0,
            );
            return;
        }
    };

    let upstream = match TcpStream::connect_timeout(&destination.into(), DIAL_TIMEOUT) {
        Ok(upstream) => upstream,
        Err(error) => {
            stats.failed.fetch_add(1, Ordering::Relaxed);
            let (reply, failure) = classify(&error);
            let _ = write_reply(&mut client, reply, Ipv4Addr::UNSPECIFIED, 0);
            (events)(Socks5Event::DialFailed {
                target,
                failure,
                detail: error.to_string(),
            });
            return;
        }
    };

    // The pin, second half. The standard library cannot bind a source address before connecting,
    // so the address the kernel actually used is checked here — before a single byte of the
    // application's is relayed. A mismatch means the route moved in between, and nothing gets
    // through but the handshake that already happened.
    let local = match upstream.local_addr() {
        Ok(SocketAddr::V4(v4)) => Some(v4),
        _ => None,
    };
    let bound = match verify_source(local, source, destination.ip().is_loopback(), &target) {
        Ok(bound) => bound,
        Err(event) => {
            stats.failed.fetch_add(1, Ordering::Relaxed);
            let _ = write_reply(
                &mut client,
                Reply::NetworkUnreachable,
                Ipv4Addr::UNSPECIFIED,
                0,
            );
            let _ = upstream.shutdown(Shutdown::Both);
            (events)(event);
            return;
        }
    };
    // A gate that shut while this dial was in flight must not be told "success": the client would
    // start speaking into a socket we are about to close.
    if !gate.is_open() {
        stats.failed.fetch_add(1, Ordering::Relaxed);
        let _ = write_reply(&mut client, Reply::NotAllowed, Ipv4Addr::UNSPECIFIED, 0);
        let _ = upstream.shutdown(Shutdown::Both);
        return;
    }

    let _ = upstream.set_nodelay(true);
    stats.dialed.fetch_add(1, Ordering::Relaxed);
    (events)(Socks5Event::Dialed);
    if write_reply(&mut client, Reply::Success, *bound.ip(), bound.port()).is_err() {
        return;
    }

    let _ = client.set_read_timeout(None);
    let _ = upstream.set_read_timeout(None);
    let active = Gauge::enter(&stats.active);
    relay(client, upstream, stats, registry, gate);
    drop(active);
}

/// Keeps a counter honest even when what it counts returns early.
struct Gauge<'a>(&'a AtomicI64);

impl<'a> Gauge<'a> {
    fn enter(counter: &'a AtomicI64) -> Self {
        counter.fetch_add(1, Ordering::Relaxed);
        Self(counter)
    }
}

impl Drop for Gauge<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}

fn relay(
    client: TcpStream,
    upstream: TcpStream,
    stats: &Arc<Stats>,
    registry: &Arc<Registry>,
    gate: &TunnelGate,
) {
    let id = registry.register(&client, &upstream);
    if !gate.is_open() {
        // The gate was shut while this connection was being dialled — by the watchdog, by a dial
        // failure, or by the engine. The registration above and this check cannot both be missed.
        registry.drop_one(id);
        return;
    }
    let (Ok(client_down), Ok(upstream_down)) = (client.try_clone(), upstream.try_clone()) else {
        registry.forget(id);
        return;
    };
    // One thread per direction: `up` is what the application sends, `down` is what comes back.
    let down_stats = Arc::clone(stats);
    let down = thread::Builder::new()
        .name("protonvpn-socks5-down".into())
        .stack_size(STACK)
        .spawn(move || copy(upstream_down, client_down, down_stats, Direction::Down))
        .ok();
    copy(client, upstream, Arc::clone(stats), Direction::Up);
    if let Some(down) = down {
        let _ = down.join();
    }
    registry.forget(id);
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Direction {
    Up,
    Down,
}

/// One direction of a relayed connection. On EOF the *peer's* write half is closed, so a protocol
/// that half-closes works; on an error both sockets go, because a broken relay is not a relay.
fn copy(mut from: TcpStream, mut to: TcpStream, stats: Arc<Stats>, direction: Direction) {
    let counter = match direction {
        Direction::Up => &stats.up,
        Direction::Down => &stats.down,
    };
    let mut buffer = [0u8; 16 * 1024];
    loop {
        match from.read(&mut buffer) {
            Ok(0) => {
                let _ = to.shutdown(Shutdown::Write);
                return;
            }
            Ok(n) => {
                if to.write_all(&buffer[..n]).is_err() {
                    let _ = from.shutdown(Shutdown::Both);
                    let _ = to.shutdown(Shutdown::Both);
                    return;
                }
                counter.fetch_add(n as u64, Ordering::Relaxed);
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => {
                let _ = from.shutdown(Shutdown::Both);
                let _ = to.shutdown(Shutdown::Both);
                return;
            }
        }
    }
}

// --- the protocol ---------------------------------------------------------------------------

#[derive(Debug)]
struct Request {
    command: u8,
    host: Host,
    port: u16,
}

#[derive(Debug)]
enum Host {
    V4(Ipv4Addr),
    Name(String),
}

impl Host {
    fn describe(&self) -> String {
        match self {
            Self::V4(ip) => ip.to_string(),
            Self::Name(name) => name.clone(),
        }
    }
}

#[derive(Debug)]
enum RequestError {
    /// The client went away mid-handshake; there is nobody to answer.
    Io,
    /// An address type we do not speak (IPv6 literals included: see the module docs).
    AddressType,
    Malformed,
}

impl From<io::Error> for RequestError {
    fn from(_: io::Error) -> Self {
        Self::Io
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reply {
    Success,
    GeneralFailure,
    NotAllowed,
    NetworkUnreachable,
    HostUnreachable,
    ConnectionRefused,
    CommandNotSupported,
    AddressTypeNotSupported,
}

impl Reply {
    fn code(self) -> u8 {
        match self {
            Self::Success => 0x00,
            Self::GeneralFailure => 0x01,
            Self::NotAllowed => 0x02,
            Self::NetworkUnreachable => 0x03,
            Self::HostUnreachable => 0x04,
            Self::ConnectionRefused => 0x05,
            Self::CommandNotSupported => 0x07,
            Self::AddressTypeNotSupported => 0x08,
        }
    }
}

/// The greeting. No authentication is offered: the listener is loopback-only, which is the same
/// promise `ssh -D` makes.
fn greet(client: &mut TcpStream) -> io::Result<()> {
    let mut head = [0u8; 2];
    client.read_exact(&mut head)?;
    if head[0] != SOCKS5 {
        return Err(io::Error::other(format!("версия SOCKS {}", head[0])));
    }
    let mut methods = vec![0u8; head[1] as usize];
    client.read_exact(&mut methods)?;
    if methods.contains(&METHOD_NO_AUTH) {
        client.write_all(&[SOCKS5, METHOD_NO_AUTH])
    } else {
        client.write_all(&[SOCKS5, METHOD_NONE])?;
        Err(io::Error::other(
            "клиент не предложил метод без аутентификации",
        ))
    }
}

fn read_request(client: &mut TcpStream) -> Result<Request, RequestError> {
    let mut head = [0u8; 4];
    client.read_exact(&mut head)?;
    if head[0] != SOCKS5 {
        return Err(RequestError::Malformed);
    }
    let host = match head[3] {
        ATYP_IPV4 => {
            let mut octets = [0u8; 4];
            client.read_exact(&mut octets)?;
            Host::V4(Ipv4Addr::from(octets))
        }
        // We speak IPv4 and nothing else, and we say so rather than guessing: an IPv6 hop the
        // gate cannot pin is a hop the proxy does not take (see the module docs).
        ATYP_IPV6 => return Err(RequestError::AddressType),
        ATYP_DOMAIN => {
            let mut length = [0u8; 1];
            client.read_exact(&mut length)?;
            let mut name = vec![0u8; length[0] as usize];
            client.read_exact(&mut name)?;
            match String::from_utf8(name) {
                Ok(name) => Host::Name(name),
                Err(_) => return Err(RequestError::Malformed),
            }
        }
        _ => return Err(RequestError::AddressType),
    };
    let mut port = [0u8; 2];
    client.read_exact(&mut port)?;
    Ok(Request {
        command: head[1],
        host,
        port: u16::from_be_bytes(port),
    })
}

fn write_reply(
    client: &mut TcpStream,
    reply: Reply,
    address: Ipv4Addr,
    port: u16,
) -> io::Result<()> {
    let mut message = [0u8; 10];
    message[0] = SOCKS5;
    message[1] = reply.code();
    message[3] = ATYP_IPV4;
    message[4..8].copy_from_slice(&address.octets());
    message[8..10].copy_from_slice(&port.to_be_bytes());
    client.write_all(&message)
}

/// Names are resolved here, at the proxy, and only after the gate has said yes: that keeps DNS
/// inside the tunnel, and it means a refused application does not even leak a lookup.
fn resolve(host: &Host, port: u16) -> Option<SocketAddrV4> {
    match host {
        Host::V4(ip) => Some(SocketAddrV4::new(*ip, port)),
        Host::Name(name) => (name.as_str(), port)
            .to_socket_addrs()
            .ok()?
            .find_map(|address| match address {
                SocketAddr::V4(v4) => Some(v4),
                SocketAddr::V6(_) => None,
            }),
    }
}

/// The pin's second half. A dial must have come from the address the gate promised — except to a
/// loopback destination, which can only ever come from the loopback address and cannot leave the
/// machine. Returns the address to report back to the client, or the event that says why the dial
/// is being thrown away.
fn verify_source(
    local: Option<SocketAddrV4>,
    pin: Ipv4Addr,
    loopback_destination: bool,
    target: &str,
) -> Result<SocketAddrV4, Socks5Event> {
    let expected = if loopback_destination {
        Ipv4Addr::LOCALHOST
    } else {
        pin
    };
    match local {
        Some(local) if *local.ip() == expected => Ok(local),
        Some(local) => Err(Socks5Event::SourceMismatch {
            expected,
            observed: *local.ip(),
        }),
        None => Err(Socks5Event::DialFailed {
            target: target.to_string(),
            failure: DialFailure::Other,
            detail: "не удалось определить адрес источника".to_string(),
        }),
    }
}

fn classify(error: &io::Error) -> (Reply, DialFailure) {
    match error.kind() {
        io::ErrorKind::ConnectionRefused => (Reply::ConnectionRefused, DialFailure::Refused),
        io::ErrorKind::TimedOut => (Reply::HostUnreachable, DialFailure::Timeout),
        io::ErrorKind::NetworkUnreachable => (Reply::NetworkUnreachable, DialFailure::Unreachable),
        io::ErrorKind::HostUnreachable => (Reply::HostUnreachable, DialFailure::Unreachable),
        io::ErrorKind::AddrNotAvailable => (Reply::NetworkUnreachable, DialFailure::Unreachable),
        io::ErrorKind::PermissionDenied => (Reply::NotAllowed, DialFailure::Unreachable),
        _ => (Reply::GeneralFailure, DialFailure::Other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::route::ScriptedRoute;

    const TUNNEL: Ipv4Addr = Ipv4Addr::new(10, 2, 0, 2);
    const LAN: Ipv4Addr = Ipv4Addr::new(192, 168, 1, 10);
    /// An address no interface ever has (TEST-NET-3), so binding to it must fail.
    const NOWHERE: Ipv4Addr = Ipv4Addr::new(203, 0, 113, 7);

    fn gate_for(route: &Arc<ScriptedRoute>) -> TunnelGate {
        TunnelGate::new(Arc::clone(route) as Arc<dyn RouteProbe>)
    }

    /// An echo server on loopback, so a relayed byte can be followed all the way there and back.
    fn echo_server() -> (SocketAddrV4, thread::JoinHandle<()>) {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let addr = match listener.local_addr().unwrap() {
            SocketAddr::V4(v4) => v4,
            other => panic!("{other}"),
        };
        let handle = thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buffer = [0u8; 1024];
                while let Ok(n) = stream.read(&mut buffer) {
                    if n == 0 || stream.write_all(&buffer[..n]).is_err() {
                        return;
                    }
                }
            }
        });
        (addr, handle)
    }

    /// A running proxy, the gate the engine would hold, its counters and its reports. The gate is
    /// deliberately **not** opened here: a test that wants relaying must say so, because "closed
    /// until proven" is the property under test.
    struct Fixture {
        server: Socks5,
        gate: TunnelGate,
        stats: Arc<Stats>,
        events: Receiver<Socks5Event>,
    }

    fn proxy(route: &Arc<ScriptedRoute>) -> Fixture {
        let (tx, rx) = mpsc::channel();
        let events: Reporter = Arc::new(move |event| {
            let _ = tx.send(event);
        });
        let stats = Arc::new(Stats::default());
        let gate = gate_for(route);
        let server = Socks5::spawn(
            Socks5Options {
                bind: Ipv4Addr::LOCALHOST,
                port: 0,
            },
            gate.clone(),
            Arc::clone(&stats),
            events,
        )
        .unwrap();
        Fixture {
            server,
            gate,
            stats,
            events: rx,
        }
    }

    /// Sessions run on their own threads; give one a moment to record what it did before
    /// asserting on its counters.
    fn wait_for(
        stats: &Stats,
        what: &str,
        mut done: impl FnMut(&StatsSnapshot) -> bool,
    ) -> StatsSnapshot {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            let counters = stats.snapshot();
            if done(&counters) {
                return counters;
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("timed out waiting for {what}: {:?}", stats.snapshot());
    }

    /// The SOCKS5 greeting, then a CONNECT to `destination`, returning the reply code.
    fn connect_through(proxy: SocketAddr, destination: SocketAddrV4) -> (u8, TcpStream) {
        let mut client = TcpStream::connect(proxy).unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        client.write_all(&[SOCKS5, 1, METHOD_NO_AUTH]).unwrap();
        let mut choice = [0u8; 2];
        client.read_exact(&mut choice).unwrap();
        assert_eq!(choice, [SOCKS5, METHOD_NO_AUTH]);

        let mut request = vec![SOCKS5, CMD_CONNECT, 0, ATYP_IPV4];
        request.extend_from_slice(&destination.ip().octets());
        request.extend_from_slice(&destination.port().to_be_bytes());
        client.write_all(&request).unwrap();

        let mut reply = [0u8; 10];
        client.read_exact(&mut reply).unwrap();
        assert_eq!(reply[0], SOCKS5);
        (reply[1], client)
    }

    #[test]
    fn a_closed_gate_refuses_without_touching_the_network() {
        let route = ScriptedRoute::new(LAN);
        // A fresh gate is closed, and that is the only state it can be in until the engine has
        // evidence: here nothing has enabled the proxy at all.
        let Fixture { server, stats, .. } = proxy(&route);
        let mut client = TcpStream::connect(server.addr()).unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        client.write_all(&[SOCKS5, 1, METHOD_NO_AUTH]).unwrap();
        let mut choice = [0u8; 2];
        client.read_exact(&mut choice).unwrap();
        assert_eq!(choice, [SOCKS5, METHOD_NO_AUTH]);

        let mut request = vec![SOCKS5, CMD_CONNECT, 0, ATYP_IPV4, 127, 0, 0, 1];
        request.extend_from_slice(&80u16.to_be_bytes());
        client.write_all(&request).unwrap();
        let mut reply = [0u8; 10];
        client.read_exact(&mut reply).unwrap();
        assert_eq!(reply[1], Reply::NotAllowed.code());
        assert_eq!(stats.snapshot().refused, 1);
        assert_eq!(stats.snapshot().dialed, 0);
    }

    #[test]
    fn an_open_gate_relays_a_connection_to_a_local_server() {
        let route = ScriptedRoute::new(TUNNEL);
        let Fixture {
            server,
            gate,
            stats,
            ..
        } = proxy(&route);
        gate.open(TUNNEL);
        let (echo, _handle) = echo_server();

        let (code, mut client) = connect_through(server.addr(), echo);
        assert_eq!(code, Reply::Success.code());
        client.write_all(b"tunnel-bytes").unwrap();
        let mut answer = [0u8; 12];
        client.read_exact(&mut answer).unwrap();
        assert_eq!(&answer, b"tunnel-bytes");

        let counters = wait_for(&stats, "both directions to be counted", |counters| {
            counters.up >= 12 && counters.down >= 12
        });
        assert_eq!(counters.accepted, 1);
        assert_eq!(counters.dialed, 1);
        assert_eq!(counters.refused, 0);
    }

    #[test]
    fn a_hostname_is_resolved_at_the_proxy() {
        let route = ScriptedRoute::new(TUNNEL);
        let Fixture { server, gate, .. } = proxy(&route);
        gate.open(TUNNEL);
        let (echo, _handle) = echo_server();

        let mut client = TcpStream::connect(server.addr()).unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        client.write_all(&[SOCKS5, 1, METHOD_NO_AUTH]).unwrap();
        let mut choice = [0u8; 2];
        client.read_exact(&mut choice).unwrap();

        let name = b"localhost";
        let mut request = vec![SOCKS5, CMD_CONNECT, 0, ATYP_DOMAIN, name.len() as u8];
        request.extend_from_slice(name);
        request.extend_from_slice(&echo.port().to_be_bytes());
        client.write_all(&request).unwrap();
        let mut reply = [0u8; 10];
        client.read_exact(&mut reply).unwrap();
        assert_eq!(reply[1], Reply::Success.code());
    }

    #[test]
    fn an_ipv6_literal_is_refused_rather_than_guessed_at() {
        let route = ScriptedRoute::new(TUNNEL);
        let Fixture {
            server,
            gate,
            stats,
            ..
        } = proxy(&route);
        gate.open(TUNNEL);

        let mut client = TcpStream::connect(server.addr()).unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        client.write_all(&[SOCKS5, 1, METHOD_NO_AUTH]).unwrap();
        let mut choice = [0u8; 2];
        client.read_exact(&mut choice).unwrap();
        let mut request = vec![SOCKS5, CMD_CONNECT, 0, ATYP_IPV6];
        request.extend_from_slice(&[0u8; 16]);
        request.extend_from_slice(&443u16.to_be_bytes());
        client.write_all(&request).unwrap();
        let mut reply = [0u8; 10];
        client.read_exact(&mut reply).unwrap();
        assert_eq!(reply[1], Reply::AddressTypeNotSupported.code());
        assert_eq!(stats.snapshot().unsupported, 1);
    }

    #[test]
    fn a_command_we_do_not_speak_is_refused() {
        let route = ScriptedRoute::new(TUNNEL);
        let Fixture { server, gate, .. } = proxy(&route);
        gate.open(TUNNEL);

        let mut client = TcpStream::connect(server.addr()).unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        client.write_all(&[SOCKS5, 1, METHOD_NO_AUTH]).unwrap();
        let mut choice = [0u8; 2];
        client.read_exact(&mut choice).unwrap();
        // UDP ASSOCIATE, which a "paranoid" proxy has no business doing: it cannot pin datagrams.
        let mut request = vec![SOCKS5, 0x03, 0, ATYP_IPV4, 0, 0, 0, 0];
        request.extend_from_slice(&0u16.to_be_bytes());
        client.write_all(&request).unwrap();
        let mut reply = [0u8; 10];
        client.read_exact(&mut reply).unwrap();
        assert_eq!(reply[1], Reply::CommandNotSupported.code());
    }

    #[test]
    fn a_route_that_moves_closes_the_door_and_reports_it() {
        let route = ScriptedRoute::new(TUNNEL);
        let Fixture {
            server,
            gate,
            events,
            ..
        } = proxy(&route);
        gate.open(TUNNEL);

        route.set(LAN);
        let event = events
            .recv_timeout(Duration::from_secs(5))
            .expect("the watchdog must notice within its interval");
        assert_eq!(
            event,
            Socks5Event::RouteDiverged {
                reason: Closed::RouteChanged {
                    expected: TUNNEL,
                    observed: Some(LAN)
                }
            }
        );

        // And a dial that arrives before the engine has closed the gate is still refused.
        let (echo, _handle) = echo_server();
        let (code, _client) = connect_through(server.addr(), echo);
        assert_eq!(code, Reply::NotAllowed.code());
    }

    #[test]
    fn a_dial_that_came_from_another_address_is_thrown_away() {
        // The second half of the pin, tested where it can be observed: a dial that connected, but
        // from an address the gate never promised, is closed again before anything is relayed.
        let bound = SocketAddrV4::new(NOWHERE, 51234);
        assert_eq!(
            verify_source(Some(bound), TUNNEL, false, "example.com"),
            Err(Socks5Event::SourceMismatch {
                expected: TUNNEL,
                observed: NOWHERE,
            })
        );

        // A loopback destination can only come from the loopback address, whatever the pin is.
        let local = SocketAddrV4::new(Ipv4Addr::LOCALHOST, 51235);
        assert_eq!(
            verify_source(Some(local), TUNNEL, true, "localhost"),
            Ok(local)
        );
        assert!(matches!(
            verify_source(Some(bound), TUNNEL, true, "localhost"),
            Err(Socks5Event::SourceMismatch { .. })
        ));

        // No local address at all is not a licence to relay: fail closed, and say so.
        assert!(matches!(
            verify_source(None, TUNNEL, false, "example.com"),
            Err(Socks5Event::DialFailed {
                failure: DialFailure::Other,
                ..
            })
        ));

        // And the ordinary case: the dial came from the pin.
        let tunnel = SocketAddrV4::new(TUNNEL, 40000);
        assert_eq!(
            verify_source(Some(tunnel), TUNNEL, false, "example.com"),
            Ok(tunnel)
        );
    }

    #[test]
    fn dropping_connections_tears_down_a_live_relay() {
        let route = ScriptedRoute::new(TUNNEL);
        let Fixture { server, gate, .. } = proxy(&route);
        gate.open(TUNNEL);
        let (echo, _handle) = echo_server();
        let (code, mut client) = connect_through(server.addr(), echo);
        assert_eq!(code, Reply::Success.code());

        client.write_all(b"x").unwrap();
        let mut answer = [0u8; 1];
        client.read_exact(&mut answer).unwrap();

        server.drop_connections();
        // The relay is gone: the next read sees EOF (or a reset), never a byte.
        let mut buffer = [0u8; 16];
        match client.read(&mut buffer) {
            Ok(0) => {}
            Ok(n) => panic!("the relay answered with {n} bytes after being dropped"),
            Err(_) => {}
        }
    }

    #[test]
    fn a_non_loopback_listener_is_refused_outright() {
        let route = ScriptedRoute::new(TUNNEL);
        let events: Reporter = Arc::new(|_| {});
        let error = Socks5::spawn(
            Socks5Options {
                bind: Ipv4Addr::UNSPECIFIED,
                port: 1080,
            },
            gate_for(&route),
            Arc::new(Stats::default()),
            events,
        )
        .unwrap_err();
        assert!(matches!(error, Socks5Error::NotLoopback(_)));
    }

    #[test]
    fn a_client_that_offers_only_a_password_is_turned_away() {
        let route = ScriptedRoute::new(TUNNEL);
        let Fixture { server, stats, .. } = proxy(&route);
        let mut client = TcpStream::connect(server.addr()).unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        client.write_all(&[SOCKS5, 1, 0x02]).unwrap();
        let mut choice = [0u8; 2];
        client.read_exact(&mut choice).unwrap();
        assert_eq!(choice, [SOCKS5, METHOD_NONE]);
        wait_for(&stats, "the refusal to be counted", |counters| {
            counters.unsupported == 1
        });
    }
}
