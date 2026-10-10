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
//! tunnel is; we learn which address the kernel picks now, and whether it still picks it.

use std::net::{IpAddr, Ipv4Addr, UdpSocket};
use std::sync::Mutex;

use crate::i18n::I18n;

/// The off-link destination the route lookup is aimed at. It is **never contacted**: the socket
/// exists only so the kernel has something to route, and it is closed without a write. A literal
/// address, so the lookup cannot depend on a resolver.
const OFF_LINK: Ipv4Addr = Ipv4Addr::new(1, 1, 1, 1);
/// Any port will do; the socket is never used.
const OFF_LINK_PORT: u16 = 53;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RouteError {
    /// The kernel has no route to an off-link destination at all. The string is the kernel's own
    /// complaint, and is empty when there is none to quote — a scripted answer has no `errno`.
    NoRoute(String),
    /// The kernel answered, but with an address that cannot be a source address.
    Unusable(String),
}

impl RouteError {
    /// One line for the console and the proxy's card.
    ///
    /// A catalogue rather than `Display`, for the reason [`crate::socks5::Closed::describe`]
    /// gives: this is a sentence shown to a person. The kernel's own complaint inside it is data
    /// and is never translated.
    pub fn describe(&self, i18n: &I18n) -> String {
        match self {
            Self::NoRoute(detail) if detail.is_empty() => i18n.core_route_unreachable(),
            Self::NoRoute(detail) => i18n.core_route_unreachable_detail(detail),
            Self::Unusable(address) => i18n.core_route_unusable(address),
        }
    }
}

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
            // No kernel was asked, so there is no complaint to quote: the reason is the absence
            // itself, and `describe` has a sentence for exactly that.
            None => Err(RouteError::NoRoute(String::new())),
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
        let i18n = I18n::new(crate::i18n::Locale::SOURCE);
        match Kernel.source() {
            Ok(ip) => assert!(usable_source(ip), "{ip}"),
            Err(error) => assert!(!error.describe(&i18n).is_empty()),
        }
    }

    #[test]
    fn a_lost_route_says_so_without_inventing_a_reason() {
        let i18n = I18n::new(crate::i18n::Locale::SOURCE);
        let probe = ScriptedRoute::default();
        assert_eq!(
            probe.source().unwrap_err().describe(&i18n),
            "no route to the outside"
        );
    }
}
