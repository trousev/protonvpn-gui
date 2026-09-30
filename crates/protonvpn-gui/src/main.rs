//! `protonvpn-gui` — the window and the tray.
//!
//! This crate is views only. All VPN logic lives in `protonvpn-core`, which has no UI dependency,
//! so the tray works with no window (`docs/architecture.md` §9). The design contract is
//! [`docs/architecture.md`](../../../docs/architecture.md); the short version is that the console
//! is the product and the only program we run is `protonvpn`.

mod app;
mod autostart;
mod console;
mod tray;

fn main() -> iced::Result {
    app::run()
}
