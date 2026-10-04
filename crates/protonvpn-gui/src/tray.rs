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

/// Samples per axis per pixel. The shield is nothing but slanted edges, and at 22 px an unsampled
/// edge is a staircase. Sixteen samples per pixel cost nothing at the rate this is rebuilt — once
/// per status change, not per frame.
const SUBSAMPLES: usize = 4;

/// The shield, in fractions of the icon box with `y` downwards.
///
/// * `TOP` / `BOTTOM` — the flat top edge and the tip.
/// * `HALF` — half the width of the body.
/// * `SHOULDER` — how far down the sides stay straight before the taper begins.
/// * `CORNER` — the radius that keeps the top corners from ending in two spikes.
/// * `TAPER_POW` / `TAPER_ROOT` — the taper follows `HALF · (1 − u^POW)^ROOT`. A root below one is
///   what turns the bottom into a point instead of a "U".
const TOP: f32 = 0.07;
const BOTTOM: f32 = 0.96;
const HALF: f32 = 0.39;
const SHOULDER: f32 = 0.30;
const CORNER: f32 = 0.14;
const TAPER_POW: f32 = 1.2;
const TAPER_ROOT: f32 = 0.50;

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

/// Half the shield's width at height `y`, in the fractions the constants above are written in.
/// `None` is outside the shield's vertical extent.
fn half_width(y: f32) -> Option<f32> {
    if !(TOP..=BOTTOM).contains(&y) {
        return None;
    }

    let shoulder = TOP + SHOULDER * (BOTTOM - TOP);
    if y < TOP + CORNER {
        // A quarter circle at each top corner, so the side meets the top edge squarely.
        let drop = TOP + CORNER - y;
        Some(HALF - CORNER + (CORNER * CORNER - drop * drop).max(0.0).sqrt())
    } else if y <= shoulder {
        // Straight sides up here: this is what makes the silhouette a shield and not a teardrop.
        Some(HALF)
    } else {
        let u = (y - shoulder) / (BOTTOM - shoulder);
        Some(HALF * (1.0 - u.powf(TAPER_POW)).powf(TAPER_ROOT))
    }
}

/// Is the point inside the shield? `x` and `y` are fractions of the icon box, `y` downwards.
fn inside(x: f32, y: f32) -> bool {
    half_width(y).is_some_and(|half| (x - 0.5).abs() <= half)
}

/// The tray icon: a shield in the status colour, ARGB32 in network byte order.
///
/// A shield rather than the dot this used to be. The tray has exactly one question to answer — "am
/// I covered" — and the silhouette says which question it is at a glance, while the colour says
/// which answer it got. It stays a generated bitmap rather than a themed icon name: on a minimal
/// desktop there may be no icon theme at all.
fn status_icon(status: &ConnectionStatus) -> ksni::Icon {
    let (r, g, b) = status_rgb(status);
    let size = ICON_SIZE as usize;
    let samples = (SUBSAMPLES * SUBSAMPLES) as u32;
    let step = 1.0 / (SUBSAMPLES * size) as f32;
    let mut data = Vec::with_capacity(size * size * 4);

    for y in 0..size {
        for x in 0..size {
            let mut hits = 0;
            for sy in 0..SUBSAMPLES {
                for sx in 0..SUBSAMPLES {
                    let px = (x * SUBSAMPLES + sx) as f32 * step + step / 2.0;
                    let py = (y * SUBSAMPLES + sy) as f32 * step + step / 2.0;
                    hits += u32::from(inside(px, py));
                }
            }
            // Alpha is how much of the pixel the shield covers. The colour is flat, because the
            // panel decides what is behind the icon and only alpha can blend with that.
            data.extend_from_slice(&[(hits * 255 / samples) as u8, r, g, b]);
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

    /// The columns a row paints at more than half opacity — the silhouette, minus the soft edge.
    fn painted_span(icon: &ksni::Icon, row: usize) -> Option<(usize, usize)> {
        let size = ICON_SIZE as usize;
        let line = &icon.data[row * size * 4..(row + 1) * size * 4];
        let lit = |column: usize| line[column * 4] > 127;
        let left = (0..size).find(|&column| lit(column))?;
        let right = (0..size).rev().find(|&column| lit(column))?;
        Some((left, right))
    }

    #[test]
    fn the_icon_is_a_shield_and_not_a_disc() {
        let icon = status_icon(&ConnectionStatus::Disconnected);
        let size = ICON_SIZE as usize;
        let widths: Vec<usize> = (0..size)
            .filter_map(|row| painted_span(&icon, row).map(|(left, right)| right - left + 1))
            .collect();
        assert!(widths.len() >= size - 4, "painted rows: {}", widths.len());

        // A disc enters and leaves the box at a point. A shield enters along a flat top edge, which
        // is the whole difference between the two silhouettes at this size.
        let widest = *widths.iter().max().unwrap();
        assert!(
            widths[0] * 10 >= widest * 7,
            "top row {} vs widest {widest}",
            widths[0]
        );

        // And leaves at a point, below the shoulder, narrowing the rest of the way.
        let last = *widths.last().unwrap();
        assert!(last * 3 <= widest, "bottom row {last} vs widest {widest}");
        let shoulder = widths.iter().position(|width| *width == widest).unwrap();
        assert!(
            widths[shoulder..].windows(2).all(|pair| pair[1] <= pair[0]),
            "the taper is not monotone: {widths:?}"
        );

        // Symmetrical about the centre column: a shield is not a leaf.
        for row in 0..size {
            if let Some((left, right)) = painted_span(&icon, row) {
                assert_eq!(left + right, size - 1, "row {row} leans");
            }
        }
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
        // Same silhouette either way, because only the answer differs: a status that changed the
        // shape too would be two things to read instead of one.
        let alpha = |icon: &ksni::Icon| icon.data.iter().step_by(4).copied().collect::<Vec<u8>>();
        assert_eq!(alpha(&connected), alpha(&disconnected));
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
