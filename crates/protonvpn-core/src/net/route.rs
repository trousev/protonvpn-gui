//! Reading the kernel's routing decision — the local half of exception #3
//! (`docs/architecture.md` §13).
//!
//! The question here is not "how does the VPN work", it is "where would the kernel send this
//! program's traffic, and from which address". A **connected UDP socket** answers it: `connect()`
//! makes the kernel run its route lookup, `getsockname()` reports the source address it picked,
//! and no packet is ever sent — the socket is dropped without a single write. No DNS is involved
//! either, because the destination is a literal address.
//!
//! That is the only system fact the SOCKS5 proxy needs, and it is available without
//! NetworkManager, without D-Bus and without touching a single Proton file: it is the same
//! question any ordinary client asks the kernel when it opens a socket. We do not learn what the
//! tunnel is; we learn only whether the kernel's answer has changed.

use std::fmt;
use std::net::{IpAddr, Ipv4Addr, UdpSocket};
use std::sync::Mutex;

/// The off-link destination the route lookup is aimed at. It is **never contacted**: the socket
/// exists only so the kernel has something to route, and it is closed without a write. A literal
/// address, so the lookup cannot depend on a resolver.
const OFF_LINK: Ipv4Addr = Ipv4Addr::new(1, 1, 1, 1);
/// Any port will do; the socket is never used.
const OFF_LINK_PORT: u16 = 53;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RouteError {
    /// The kernel has no route to an off-link destination at all.
    NoRoute(String),
    /// The kernel answered, but with an address that cannot be a source address.
    Unusable(String),
}

impl fmt::Display for RouteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoRoute(e) => write!(f, "нет маршрута: {e}"),
            Self::Unusable(e) => write!(f, "ядро не выбрало адрес источника: {e}"),
        }
    }
}

impl std::error::Error for RouteError {}

/// Reports the source address the kernel would use for off-link IPv4 traffic.
///
/// A trait so the proxy's fail-closed logic can be tested without a VPN, a route or a network —
/// the same seam [`crate::engine::EngineOptions::program`] is for the CLI.
pub trait RouteProbe: Send + Sync + 'static {
    fn source(&self) -> Result<Ipv4Addr, RouteError>;
}

/// The real probe: one `connect(2)`, one `getsockname(2)`, no packets.
#[derive(Debug, Default, Clone, Copy)]
pub struct Kernel;

impl RouteProbe for Kernel {
    fn source(&self) -> Result<Ipv4Addr, RouteError> {
        let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))
            .map_err(|error| RouteError::NoRoute(error.to_string()))?;
        socket
            .connect((OFF_LINK, OFF_LINK_PORT))
            .map_err(|error| RouteError::NoRoute(error.to_string()))?;
        let local = socket
            .local_addr()
            .map_err(|error| RouteError::NoRoute(error.to_string()))?
            .ip();
        match local {
            IpAddr::V4(ip) if usable_source(ip) => Ok(ip),
            other => Err(RouteError::Unusable(other.to_string())),
        }
    }
}

/// An address the kernel could only have picked because there is no real route: never a source.
/// Refusing it is the fail-closed reading — we would rather refuse every connection than bind to
/// something that cannot reach anything.
fn usable_source(ip: Ipv4Addr) -> bool {
    !ip.is_unspecified() && !ip.is_loopback() && !ip.is_broadcast() && !ip.is_multicast()
}

/// A probe whose answer the caller sets. Test seam, in the spirit of `EngineOptions::program`:
/// production code always uses [`Kernel`].
#[derive(Debug, Default)]
pub struct ScriptedRoute {
    /// `None` means "no route", which is a real answer on a machine that is offline.
    answer: Mutex<Option<Ipv4Addr>>,
}

impl ScriptedRoute {
    pub fn new(answer: Ipv4Addr) -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self {
            answer: Mutex::new(Some(answer)),
        })
    }

    /// The kernel starts choosing a different address — a tunnel going up, or coming down.
    pub fn set(&self, answer: Ipv4Addr) {
        *self.answer.lock().unwrap_or_else(|p| p.into_inner()) = Some(answer);
    }

    /// There is no route any more.
    pub fn lose(&self) {
        *self.answer.lock().unwrap_or_else(|p| p.into_inner()) = None;
    }
}

impl RouteProbe for ScriptedRoute {
    fn source(&self) -> Result<Ipv4Addr, RouteError> {
        match *self.answer.lock().unwrap_or_else(|p| p.into_inner()) {
            Some(ip) => Ok(ip),
            None => Err(RouteError::NoRoute("маршрута нет".to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_scripted_probe_says_what_it_was_told() {
        let probe = ScriptedRoute::new(Ipv4Addr::new(10, 2, 0, 2));
        assert_eq!(probe.source().unwrap(), Ipv4Addr::new(10, 2, 0, 2));
        probe.set(Ipv4Addr::new(192, 168, 1, 10));
        assert_eq!(probe.source().unwrap(), Ipv4Addr::new(192, 168, 1, 10));
        probe.lose();
        assert!(matches!(probe.source(), Err(RouteError::NoRoute(_))));
    }

    #[test]
    fn addresses_that_cannot_be_a_source_are_refused() {
        assert!(usable_source(Ipv4Addr::new(10, 2, 0, 2)));
        assert!(!usable_source(Ipv4Addr::UNSPECIFIED));
        assert!(!usable_source(Ipv4Addr::LOCALHOST));
        assert!(!usable_source(Ipv4Addr::BROADCAST));
        assert!(!usable_source(Ipv4Addr::new(224, 0, 0, 1)));
    }

    /// Not an assertion about this machine's network, which may be offline: it asserts that the
    /// real probe answers or fails, and never invents an unusable source.
    #[test]
    fn the_real_probe_answers_or_says_why_not() {
        match Kernel.source() {
            Ok(ip) => assert!(usable_source(ip), "{ip}"),
            Err(error) => assert!(!error.to_string().is_empty()),
        }
    }
}
