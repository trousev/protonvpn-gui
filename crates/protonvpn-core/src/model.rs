//! The state model — what the interpreter produces and the views render.
//!
//! Two rules from `docs/architecture.md` shape everything here:
//!
//! * **Every piece of state carries a timestamp** (§7). State is polled, so it can be minutes old;
//!   the UI shows the *age* and lets the user judge. There is deliberately no "stale" flag and no
//!   `is_fresh()` helper for a view to turn into a warning.
//! * **Never invent a status** (§5). Absence of information is [`ConnectionStatus::Unknown`], not
//!   `Disconnected`.

use std::fmt;
use std::net::IpAddr;
use std::time::{Duration, SystemTime};

use crate::i18n::I18n;

/// Identifies one recorded invocation. Monotonic within a process run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct InvocationId(pub u64);

impl fmt::Display for InvocationId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{}", self.0)
    }
}

/// A value together with the moment we learned it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Observation<T> {
    pub value: T,
    pub at: SystemTime,
}

impl<T> Observation<T> {
    pub fn now(value: T) -> Self {
        Self::at(value, SystemTime::now())
    }

    pub fn at(value: T, at: SystemTime) -> Self {
        Self { value, at }
    }

    pub fn age(&self) -> Duration {
        self.age_at(SystemTime::now())
    }

    pub fn age_at(&self, now: SystemTime) -> Duration {
        now.duration_since(self.at).unwrap_or(Duration::ZERO)
    }

    /// The age, rendered per the wording table in `docs/architecture.md` §7.
    ///
    /// Deliberately never says "stale" and carries no icon: the number is the whole message.
    pub fn age_text(&self, i18n: &I18n) -> String {
        i18n.age_text(self.age())
    }
}

/// What the CLI last told us about the connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionStatus {
    /// Nothing has been observed yet. **Not** the same as disconnected.
    Unknown,
    Disconnected,
    /// Optimistic, set when we start a `connect` child. Reconciled by the next poll.
    Connecting,
    Connected(ConnectedInfo),
    /// The CLI refused or failed. The raw message is kept verbatim.
    Error(String),
}

impl ConnectionStatus {
    pub fn is_connected(&self) -> bool {
        matches!(self, Self::Connected(_))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ConnectedInfo {
    /// `NL#818`
    pub server: String,
    /// `Amsterdam, Netherlands`
    pub location: String,
    /// `59`
    pub load_percent: Option<u8>,
    /// `wireguard`
    pub protocol: Option<String>,
}

impl ConnectedInfo {
    pub fn describe(&self) -> String {
        if self.location.is_empty() {
            self.server.clone()
        } else {
            format!("{} · {}", self.server, self.location)
        }
    }
}

/// The account, as `protonvpn info` reports it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Account {
    /// `None` means the CLI did not report an account name (logged out, or unrecognised output).
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Country {
    pub name: String,
    pub code: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct City {
    pub name: String,
    /// `P2P`, `Tor`, `Secure Core`, … — the CLI prints them comma-separated.
    pub features: Vec<String>,
}

impl City {
    pub fn has_feature(&self, needle: &str) -> bool {
        self.features.iter().any(|f| f.eq_ignore_ascii_case(needle))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Setting {
    pub key: String,
    pub value: String,
}

/// Why the CLI failed, as far as we are willing to claim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliError {
    pub kind: CliErrorKind,
    /// The CLI's own text, verbatim.
    pub message: String,
    pub invocation: InvocationId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CliErrorKind {
    /// Exit code 2: bad country, bad server, usage.
    Validation,
    /// The official GTK app holds `proton.vpn.app.gtk`, so the CLI refuses to run at all.
    /// We are never allowed to own that bus name — see `docs/cli-surface.md` §2.
    Coexistence,
    NotFound,
    Other,
}

/// Port-forwarding, as we know it. The lease is the one thing the CLI cannot maintain
/// (`docs/architecture.md` §10.1), so this state comes from our own NAT-PMP client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PortForwarding {
    /// Preference off, or nothing requested yet.
    Idle,
    /// A connect to a P2P server is in progress; no lease attempted yet.
    Pending,
    /// The server itself says it does not support forwarding.
    Unsupported,
    /// We tried and failed; the gateway is not answering, or returned a non-zero result.
    Unavailable(String),
    /// A live lease. `renewed_at` is the age the UI shows.
    Active {
        port: u16,
        lifetime: Duration,
        external_ip: Option<IpAddr>,
    },
}

impl PortForwarding {
    pub fn port(&self) -> Option<u16> {
        match self {
            Self::Active { port, .. } => Some(*port),
            _ => None,
        }
    }
}

/// Where a ground-truth reading came from. Baseline and current readings must use the same one,
/// or a GeoIP disagreement could masquerade as a state change (`docs/architecture.md` §8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeEndpoint {
    IfConfigCo,
    IpInfoIo,
    IfConfigMe,
}

impl ProbeEndpoint {
    pub fn url(&self) -> &'static str {
        match self {
            Self::IfConfigCo => "https://ifconfig.co/json",
            Self::IpInfoIo => "https://ipinfo.io/json",
            Self::IfConfigMe => "https://ifconfig.me/all.json",
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::IfConfigCo => "ifconfig.co",
            Self::IpInfoIo => "ipinfo.io",
            Self::IfConfigMe => "ifconfig.me",
        }
    }
}

/// One egress reading. Country and ASN are for *display only* — measured to disagree between
/// databases for the same address, so they are never a correctness signal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EgressReading {
    pub endpoint: ProbeEndpoint,
    pub ip: IpAddr,
    pub country: Option<String>,
    pub asn_org: Option<String>,
}

/// The probe's two readings, kept side by side.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Egress {
    /// Taken before we asked for a connection. The tunnel question is "did this change?".
    pub baseline: Option<Observation<EgressReading>>,
    /// Latest reading, typically taken after a connect.
    pub current: Option<Observation<EgressReading>>,
}

impl Egress {
    /// Did traffic actually start going somewhere else? `None` when we cannot tell — which is not
    /// the same as "yes" or "no".
    pub fn egress_changed(&self) -> Option<bool> {
        let (baseline, current) = (self.baseline.as_ref()?, self.current.as_ref()?);
        if baseline.value.endpoint != current.value.endpoint {
            return None;
        }
        Some(baseline.value.ip != current.value.ip)
    }
}

/// Runner status — "working / idle", independent of connection status.
///
/// This lives **only** in the main window's collapsed console bar; the tray answers
/// "am I connected" and nothing else (`docs/architecture.md` §9).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum RunnerStatus {
    #[default]
    Idle,
    Running {
        id: InvocationId,
        argv: Vec<String>,
        started_at: SystemTime,
    },
    Queued {
        depth: usize,
    },
}

/// Everything the interpreter knows. One value, replaced wholesale by the reducer so it stays
/// pure and testable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppState {
    pub connection: Observation<ConnectionStatus>,
    pub account: Option<Observation<Account>>,
    pub countries: Option<Observation<Vec<Country>>>,
    pub cities: Option<Observation<Vec<City>>>,
    /// Which country the city list belongs to, so a stale list is not shown for the wrong country.
    pub cities_country: Option<String>,
    pub settings: Option<Observation<Vec<Setting>>>,
    pub last_error: Option<Observation<CliError>>,
    pub port_forwarding: Observation<PortForwarding>,
    pub egress: Egress,
    /// Last invocation that completed, for the collapsed console. Purely informational.
    pub last_run: Option<Observation<String>>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            connection: Observation::now(ConnectionStatus::Unknown),
            account: None,
            countries: None,
            cities: None,
            cities_country: None,
            settings: None,
            last_error: None,
            port_forwarding: Observation::now(PortForwarding::Idle),
            egress: Egress::default(),
            last_run: None,
        }
    }
}

impl AppState {
    pub fn status(&self) -> &ConnectionStatus {
        &self.connection.value
    }
}

/// A connect target, as the user picked it. The launcher turns this into argv.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ConnectTarget {
    pub country: Option<String>,
    pub city: Option<String>,
    pub server: Option<String>,
    pub p2p: bool,
    pub secure_core: bool,
    pub tor: bool,
    pub random: bool,
    /// Whether we commit to holding a NAT-PMP lease for the connection this target establishes.
    ///
    /// **This is not a CLI flag** and must never appear in argv: the CLI cannot forward a port
    /// (`docs/architecture.md` §10.1). `protonvpn config set port-forwarding on` only tells the
    /// server to offer a mapping at connect time; the lease itself is exception #2, and it
    /// belongs to the profile being connected, not to the app (`docs/architecture.md` §11).
    pub port_forwarding: bool,
}

impl ConnectTarget {
    pub fn fastest() -> Self {
        Self::default()
    }

    pub fn country(code: impl Into<String>) -> Self {
        Self {
            country: Some(code.into()),
            ..Self::default()
        }
    }
}

/// A live port-forwarding lease, as held by the NAT-PMP client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lease {
    pub port: u16,
    pub lifetime: Duration,
    pub external_ip: Option<IpAddr>,
    pub acquired_at: SystemTime,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absence_of_information_is_unknown_not_disconnected() {
        assert_eq!(AppState::default().status(), &ConnectionStatus::Unknown);
        assert!(!AppState::default().status().is_connected());
    }

    #[test]
    fn egress_change_needs_the_same_endpoint() {
        let ip_a: IpAddr = "1.2.3.4".parse().unwrap();
        let ip_b: IpAddr = "5.6.7.8".parse().unwrap();
        let reading = |endpoint, ip| EgressReading {
            endpoint,
            ip,
            country: None,
            asn_org: None,
        };

        let mut egress = Egress {
            baseline: Some(Observation::now(reading(ProbeEndpoint::IfConfigCo, ip_a))),
            current: Some(Observation::now(reading(ProbeEndpoint::IfConfigCo, ip_b))),
        };
        assert_eq!(egress.egress_changed(), Some(true));

        egress.current = Some(Observation::now(reading(ProbeEndpoint::IfConfigCo, ip_a)));
        assert_eq!(egress.egress_changed(), Some(false));

        // Different endpoint: refuse to answer rather than compare across databases.
        egress.current = Some(Observation::now(reading(ProbeEndpoint::IpInfoIo, ip_b)));
        assert_eq!(egress.egress_changed(), None);

        egress.baseline = None;
        assert_eq!(egress.egress_changed(), None);
    }
}
