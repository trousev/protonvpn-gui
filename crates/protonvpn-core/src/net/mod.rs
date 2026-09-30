//! `net` — the network bits that are not `protonvpn`.
//!
//! Exactly one module lives here so far: NAT-PMP, sanctioned exception #2
//! (`docs/architecture.md` §10.1). Nothing else may join it without a human decision.

pub mod natpmp;
