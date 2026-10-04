//! `net` — the network bits that are not `protonvpn`.
//!
//! Two modules live here, and each needed a human decision: NAT-PMP, sanctioned exception #2
//! (`docs/architecture.md` §10.1), and the kernel route read that the SOCKS5 proxy's fail-closed
//! gate is built on, exception #3 (§13). Nothing else may join them.

pub mod natpmp;
pub mod route;
