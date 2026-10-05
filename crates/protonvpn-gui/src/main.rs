//! `protonvpn-gui` — the window and the tray.
//!
//! This crate is views only. All VPN logic lives in `protonvpn-core`, which has no UI dependency,
//! so the tray works with no window (`docs/architecture.md` §9). The design contract is
//! [`docs/architecture.md`](../../../docs/architecture.md); the short version is that the console
//! is the product and the only program we run is `protonvpn`.

mod app;
mod console;
mod desktop;
mod theme;
mod tray;
mod version;
mod widgets;

fn main() -> iced::Result {
    // `--version` is the only argument this program understands, and it exists because a file has
    // to be able to say what it is without starting a window: it is what someone checks before
    // letting an AppImage replace itself, and what a bug report needs. Everything else is ignored,
    // as it was before.
    if std::env::args()
        .skip(1)
        .any(|argument| argument == "--version" || argument == "-V")
    {
        println!("{}", version::cli_line());
        return Ok(());
    }

    app::run()
}
