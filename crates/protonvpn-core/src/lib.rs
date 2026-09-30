//! `protonvpn-core` — everything that touches the VPN.
//!
//! Design contract: `docs/architecture.md`. The short version:
//!
//! * The only program we execute is `protonvpn`. We do not know, and must not know, how it
//!   connects. No NetworkManager, no D-Bus, no Proton-internal files.
//! * The console is the product. Every invocation is recorded verbatim — argv, exit code and
//!   raw output — and both the UI and the state interpreter read that one stream.
//! * The launcher maps intents to argv and never interprets results.
//!
//! This crate has no UI dependency on purpose: the tray must work with no window.

pub mod pty;
