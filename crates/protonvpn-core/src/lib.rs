//! `protonvpn-core` — everything that touches the VPN.
//!
//! Design contract: [`docs/architecture.md`](../../../docs/architecture.md). The short version:
//!
//! * **The only program we execute is `protonvpn`.** We do not know, and must not know, how it
//!   connects: no NetworkManager, no D-Bus, no keyring, no Proton-internal files. Four exceptions
//!   are sanctioned and bounded — a `curl` ground-truth probe ([`probe`]), NAT-PMP for the
//!   port-forwarding lease ([`net::natpmp`]), a loopback SOCKS5 listener ([`socks5`]) and the
//!   AppImage updater ([`update`]).
//! * **The console is the product.** Every invocation is recorded verbatim — argv, exit code and
//!   raw output — in [`logbus`], and both the console and the state reducer read that one stream.
//! * **The launcher maps intents to argv and never interprets results** ([`launcher`]).
//! * **The interpreter is a pure reducer** over the log ([`interpreter`]); it never guesses and
//!   never invents a status.
//!
//! The layers, and the only direction data flows:
//!
//! ```text
//!   views ──snapshot──► engine ──Request──► launcher ──argv──► runner ──PTY──► protonvpn
//!                        │                                        │
//!                        │  ◄──────────── lines + exit code ───────┘
//!                        ▼
//!                   log bus ──same stream──► interpreter ──► state
//! ```
//!
//! This crate has no UI dependency on purpose: the tray must work with no window.

pub mod config;
pub mod engine;
pub mod i18n;
pub mod interpreter;
pub mod launcher;
pub mod logbus;
pub mod model;
pub mod net;
pub mod parse;
pub mod poll;
pub mod probe;
pub mod pty;
pub mod runner;
pub mod socks5;
pub mod update;

pub use engine::{EngineHandle, EngineOptions, Request, TrayPresenter, TrayView};
pub use i18n::{I18n, Locale};
pub use launcher::Intent;
pub use model::{AppState, ConnectionStatus, RunnerStatus};
