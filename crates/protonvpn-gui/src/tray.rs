//! The tray — a `ksni` StatusNotifierItem on its own thread.
//!
//! `docs/architecture.md` §9. The tray is a **peer view**, not a sub-feature of the window:
//!
//! * it shows **connection status only** — `RunnerStatus` is deliberately absent, because the tray
//!   answers "am I connected", not "what are you doing",
//! * it works with no window ever shown, which is why the engine lives in a GUI-free crate,
//! * its menu offers connect/disconnect and show/quit,
//! * and when there is no StatusNotifierItem host (GNOME without the AppIndicator extension), it
//!   reports that instead of letting the app hide into nothing.
//!
//! The item never takes the bus name `proton.vpn.app.gtk`: ksni owns
//! `org.kde.StatusNotifierItem-<pid>-<id>`, and that is the only name we ever hold.

use std::sync::mpsc::Sender;

use ksni::blocking::{Handle, TrayMethods};
use ksni::menu::{MenuItem, StandardItem};
use protonvpn_core::engine::TrayView;
use protonvpn_core::model::ConnectionStatus;

/// Size of the generated icon. StatusNotifierItems are usually rendered at 22 px.
const ICON_SIZE: i32 = 22;

/// What the tray asks the window to do.
///
/// The tray never talks to the engine directly: it sends these to the GUI, which is the single
/// place that turns a click into a [`protonvpn_core::engine::Request`]. One writer, one path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayCommand {
    Show,
    Connect,
    Disconnect,
    Quit,
}

pub struct ProtonTray {
    tx: Sender<TrayCommand>,
    view: TrayView,
}

impl ksni::Tray for ProtonTray {
    fn id(&self) -> String {
        "protonvpn-gui".to_string()
    }

    fn title(&self) -> String {
        "Proton VPN".to_string()
    }

    /// An icon generated here rather than named from a theme: on a minimal desktop there may be no
    /// icon theme at all, and a status light that cannot be seen is not a status light.
    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        vec![status_icon(&self.view.status)]
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            icon_name: String::new(),
            icon_pixmap: vec![status_icon(&self.view.status)],
            title: format!("Proton VPN · {}", self.view.status.label()),
            description: match &self.view.detail {
                Some(detail) => format!("{detail}\n{}", self.view.age_text),
                None => self.view.age_text.clone(),
            },
        }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        let _ = self.tx.send(TrayCommand::Show);
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let connected = self.view.status.is_connected();
        vec![
            StandardItem {
                label: format!("Статус: {}", self.view.status.label()),
                enabled: false,
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: self.view.age_text.clone(),
                enabled: false,
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "Подключиться".into(),
                enabled: !connected,
                icon_name: "network-vpn".into(),
                activate: Box::new(|tray: &mut Self| {
                    let _ = tray.tx.send(TrayCommand::Connect);
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Отключиться".into(),
                enabled: connected,
                icon_name: "network-offline".into(),
                activate: Box::new(|tray: &mut Self| {
                    let _ = tray.tx.send(TrayCommand::Disconnect);
                }),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "Открыть окно".into(),
                icon_name: "window-new".into(),
                activate: Box::new(|tray: &mut Self| {
                    let _ = tray.tx.send(TrayCommand::Show);
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Выход".into(),
                icon_name: "application-exit".into(),
                activate: Box::new(|tray: &mut Self| {
                    let _ = tray.tx.send(TrayCommand::Quit);
                }),
                ..Default::default()
            }
            .into(),
        ]
    }
}

/// Keeps the engine's view in the tray. The engine calls this; the tray thread does the rest.
pub struct TrayPresenter {
    handle: Handle<ProtonTray>,
}

impl TrayPresenter {
    /// Shuts the tray service down, so the process can exit without a dangling name.
    pub fn shutdown(&self) {
        self.handle.shutdown().wait();
    }
}

impl protonvpn_core::engine::TrayPresenter for TrayPresenter {
    fn update(&self, view: TrayView) {
        self.handle.update(|tray: &mut ProtonTray| tray.view = view);
    }
}

/// Starts the tray. `None` means there is no StatusNotifierItem host — the caller must then keep
/// the window reachable rather than hiding it.
pub fn spawn(tx: Sender<TrayCommand>, view: TrayView) -> Option<TrayPresenter> {
    let tray = ProtonTray { tx, view };
    match tray.spawn() {
        Ok(handle) => Some(TrayPresenter { handle }),
        Err(error) => {
            eprintln!("protonvpn-gui: системный трей недоступен: {error}");
            None
        }
    }
}

/// A filled circle in the status colour, ARGB32 in network byte order.
fn status_icon(status: &ConnectionStatus) -> ksni::Icon {
    let (r, g, b) = status_rgb(status);
    let size = ICON_SIZE as usize;
    let mut data = Vec::with_capacity(size * size * 4);
    let center = (ICON_SIZE as f32 - 1.0) / 2.0;
    let radius = center;

    for y in 0..size {
        for x in 0..size {
            let dx = x as f32 - center;
            let dy = y as f32 - center;
            let distance = (dx * dx + dy * dy).sqrt();
            // A soft edge, so the dot does not look like a square on a HiDPI panel.
            let alpha = ((radius - distance).clamp(0.0, 1.0) * 255.0) as u8;
            data.extend_from_slice(&[alpha, r, g, b]);
        }
    }

    ksni::Icon {
        width: ICON_SIZE,
        height: ICON_SIZE,
        data,
    }
}

/// Status colours, and the reason each one is what it is: green only when the CLI says connected,
/// amber only while we are waiting for it to make up its mind, grey for "no idea" as opposed to
/// "off" — the two are different, and the tray must not conflate them.
fn status_rgb(status: &ConnectionStatus) -> (u8, u8, u8) {
    match status {
        ConnectionStatus::Connected(_) => (76, 175, 80),
        ConnectionStatus::Connecting => (255, 179, 0),
        ConnectionStatus::Disconnected => (158, 158, 158),
        ConnectionStatus::Error(_) => (229, 57, 53),
        ConnectionStatus::Unknown => (96, 125, 139),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // The trait methods (`id`, `menu`, `tool_tip`) are only callable with it in scope.
    use ksni::Tray as _;

    fn view(status: ConnectionStatus) -> TrayView {
        TrayView {
            status,
            detail: None,
            age_text: "updated just now".into(),
        }
    }

    #[test]
    fn the_icon_is_a_square_argb_pixmap_of_the_expected_size() {
        let icon = status_icon(&ConnectionStatus::Disconnected);
        assert_eq!(icon.width, ICON_SIZE);
        assert_eq!(icon.height, ICON_SIZE);
        assert_eq!(icon.data.len(), (ICON_SIZE * ICON_SIZE * 4) as usize);
        // The centre is opaque, the corner is not.
        let centre = ((ICON_SIZE * ICON_SIZE / 2 + ICON_SIZE / 2) * 4) as usize;
        assert_eq!(icon.data[centre], 255);
        assert_eq!(icon.data[0], 0);
    }

    #[test]
    fn connected_and_disconnected_are_visibly_different() {
        let connected = status_icon(&ConnectionStatus::Connected(Default::default()));
        let disconnected = status_icon(&ConnectionStatus::Disconnected);
        assert_ne!(connected.data, disconnected.data);
        // Green, not grey.
        assert_eq!(
            status_rgb(&ConnectionStatus::Connected(Default::default())),
            (76, 175, 80)
        );
        assert_eq!(status_rgb(&ConnectionStatus::Disconnected), (158, 158, 158));
    }

    #[test]
    fn unknown_is_not_painted_as_disconnected() {
        assert_ne!(
            status_rgb(&ConnectionStatus::Unknown),
            status_rgb(&ConnectionStatus::Disconnected)
        );
    }

    #[test]
    fn the_tooltip_says_what_is_known_and_how_old_it_is() {
        let tray = ProtonTray {
            tx: std::sync::mpsc::channel().0,
            view: TrayView {
                status: ConnectionStatus::Connected(Default::default()),
                detail: Some("NL#818 · Amsterdam, Netherlands".into()),
                age_text: "updated 3 mins ago".into(),
            },
        };
        let tooltip = tray.tool_tip();
        assert!(tooltip.title.contains("подключено"));
        assert!(tooltip.description.contains("NL#818"));
        assert!(tooltip.description.contains("updated 3 mins ago"));
        // The tray never mentions the runner: that is the window's business.
        assert!(!tooltip.description.contains("работаю"));
    }

    #[test]
    fn the_menu_offers_connect_disconnect_show_and_quit() {
        let tray = ProtonTray {
            tx: std::sync::mpsc::channel().0,
            view: view(ConnectionStatus::Disconnected),
        };
        let labels: Vec<String> = tray
            .menu()
            .iter()
            .filter_map(|item| match item {
                MenuItem::Standard(standard) => Some(standard.label.clone()),
                _ => None,
            })
            .collect();
        assert!(labels.iter().any(|l| l.contains("Подключиться")));
        assert!(labels.iter().any(|l| l.contains("Отключиться")));
        assert!(labels.iter().any(|l| l.contains("Открыть окно")));
        assert!(labels.iter().any(|l| l.contains("Выход")));
    }

    #[test]
    fn the_tray_id_is_ours_and_never_protons() {
        let tray = ProtonTray {
            tx: std::sync::mpsc::channel().0,
            view: view(ConnectionStatus::Unknown),
        };
        assert_eq!(tray.id(), "protonvpn-gui");
        assert_ne!(tray.id(), "proton.vpn.app.gtk");
    }
}
