//! The tray — a `ksni` StatusNotifierItem on its own thread.
//!
//! `docs/architecture.md` §9. The tray is a **peer view**, not a sub-feature of the window:
//!
//! * it shows **connection status only** — `RunnerStatus` is deliberately absent, because the tray
//!   answers "am I connected", not "what are you doing",
//! * its icon is a shield in the status colour, struck through when there is no connection: at 22 px
//!   a colour on its own is a weak signal, and "off" is the state most worth noticing,
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
#[cfg(test)]
use protonvpn_core::engine::TrayUpdate;
use protonvpn_core::engine::TrayView;
use protonvpn_core::i18n::I18n;
use protonvpn_core::model::ConnectionStatus;
#[cfg(test)]
use protonvpn_core::update::Version;

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

/// The cut that marks "no connection": a band through the middle of the box, this wide measured
/// across it — about three of the twenty-two pixels, which is the narrowest that still reads as a
/// slash rather than as a seam.
const STRIKE_WIDTH: f32 = 0.15;

/// Slope of that band, in screen coordinates: one is the 45° of a "no" sign, running from the top
/// left to the bottom right.
const STRIKE_SLOPE: f32 = 1.0;

/// What the tray asks the window to do.
///
/// The tray never talks to the engine directly: it sends these to the GUI, which is the single
/// place that turns a click into a [`protonvpn_core::engine::Request`]. One writer, one path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayCommand {
    Show,
    /// Open the window on the update, rather than on whatever page it was left on: the menu item
    /// said "Update 0.1.43", and a click that lands somewhere else is a click that lied.
    ShowUpdate,
    Connect,
    Disconnect,
    Quit,
}

pub struct ProtonTray {
    tx: Sender<TrayCommand>,
    view: TrayView,
    /// The tray's own catalogue. It lives on its own thread, so it cannot borrow the window's; it
    /// is swapped whenever [`TrayView::language`] says the language has moved on.
    i18n: I18n,
}

impl ProtonTray {
    /// A tray item together with the catalogue it speaks through. The language is taken from the
    /// view, which is the only thing that knows it at this point.
    fn new(tx: Sender<TrayCommand>, view: TrayView) -> Self {
        let i18n = I18n::new(view.language);
        Self { tx, view, i18n }
    }
}

impl ksni::Tray for ProtonTray {
    fn id(&self) -> String {
        "protonvpn-gui".to_string()
    }

    fn title(&self) -> String {
        // The brand: the same words in every language, but still read from the catalogue, so that
        // "it is never translated" is a decision rather than an accident (i18n/en/chrome.ftl).
        self.i18n.app_name()
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
            title: format!(
                "{} · {}",
                self.i18n.app_name(),
                self.i18n.connection_label(&self.view.status)
            ),
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
        let mut items: Vec<MenuItem<Self>> = vec![
            StandardItem {
                // The status word comes from the catalogue too, and may legitimately be the same
                // in the status chip and here — one place decides what "connected" is called.
                label: self
                    .i18n
                    .tray_status(self.i18n.connection_label(&self.view.status)),
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
                label: self.i18n.tray_connect(),
                enabled: !connected,
                icon_name: "network-vpn".into(),
                activate: Box::new(|tray: &mut Self| {
                    let _ = tray.tx.send(TrayCommand::Connect);
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: self.i18n.tray_disconnect(),
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
                label: self.i18n.tray_show_window(),
                icon_name: "window-new".into(),
                activate: Box::new(|tray: &mut Self| {
                    let _ = tray.tx.send(TrayCommand::Show);
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: self.i18n.tray_quit(),
                icon_name: "application-exit".into(),
                activate: Box::new(|tray: &mut Self| {
                    let _ = tray.tx.send(TrayCommand::Quit);
                }),
                ..Default::default()
            }
            .into(),
        ];

        // An update is a menu item and not a notification: it is there to be found, never to
        // interrupt (`docs/architecture.md` §9, §14). It says which of the two true things it is —
        // ready and waiting for a restart, or merely available.
        if let Some(update) = &self.view.update {
            let version = update.version.to_string();
            let label = if update.applied {
                self.i18n.tray_update_applied(version)
            } else {
                self.i18n.tray_update_available(version)
            };
            items.insert(
                items.len() - 1,
                StandardItem {
                    label,
                    icon_name: "system-software-update".into(),
                    activate: Box::new(|tray: &mut Self| {
                        let _ = tray.tx.send(TrayCommand::ShowUpdate);
                    }),
                    ..Default::default()
                }
                .into(),
            );
        }

        items
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
        self.handle.update(|tray: &mut ProtonTray| {
            if tray.i18n.locale() != view.language {
                tray.i18n = I18n::new(view.language);
            }
            tray.view = view;
        });
    }
}

/// Starts the tray. `None` means there is no StatusNotifierItem host — the caller must then keep
/// the window reachable rather than hiding it.
pub fn spawn(tx: Sender<TrayCommand>, view: TrayView) -> Option<TrayPresenter> {
    // The language is remembered before the item is built: `spawn` consumes it, and the one place
    // that has to speak after that is the failure below.
    let language = view.language;
    let tray = ProtonTray::new(tx, view);
    match tray.spawn() {
        Ok(handle) => Some(TrayPresenter { handle }),
        Err(error) => {
            // A second catalogue, built only on this path: a `FluentBundle` is `Send` but not
            // `Sync`, and the tray's own went with the item.
            eprintln!(
                "{}",
                I18n::new(language).tray_unavailable(error.to_string())
            );
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

/// Is the point inside the cut that strikes the shield out? A band through the middle of the box,
/// as wide across the shield as [`STRIKE_WIDTH`] says.
fn on_the_strike(x: f32, y: f32) -> bool {
    // Distance from the line `y − ½ = slope · (x − ½)`, rescaled from `|…|` to a perpendicular.
    let signed = (y - 0.5 - STRIKE_SLOPE * (x - 0.5)).abs();
    signed < STRIKE_WIDTH / 2.0 * (1.0 + STRIKE_SLOPE * STRIKE_SLOPE).sqrt()
}

/// Whether this status is drawn as a struck-out shield. Only "off" is: an error is not the same
/// thing as the user having turned the tunnel off, and the two should not share a silhouette.
fn struck_out(status: &ConnectionStatus) -> bool {
    matches!(status, ConnectionStatus::Disconnected)
}

/// The tray icon: a shield in the status colour, struck through when there is no connection,
/// ARGB32 in network byte order.
///
/// A shield rather than the dot this used to be. The tray has exactly one question to answer — "am
/// I covered" — and the silhouette says which question it is at a glance, while the colour says
/// which answer it got. The one answer worth more than a colour is "no": at 22 px on a panel whose
/// background we do not control, red alone is easy to miss and easy to mistake for an amber
/// "connecting", so "off" is also cut across. It stays a generated bitmap rather than a themed
/// icon name: on a minimal desktop there may be no icon theme at all.
fn status_icon(status: &ConnectionStatus) -> ksni::Icon {
    let (r, g, b) = status_rgb(status);
    let size = ICON_SIZE as usize;
    let samples = (SUBSAMPLES * SUBSAMPLES) as u32;
    let step = 1.0 / (SUBSAMPLES * size) as f32;
    let struck = struck_out(status);
    let mut data = Vec::with_capacity(size * size * 4);

    for y in 0..size {
        for x in 0..size {
            let mut hits = 0;
            for sy in 0..SUBSAMPLES {
                for sx in 0..SUBSAMPLES {
                    let px = (x * SUBSAMPLES + sx) as f32 * step + step / 2.0;
                    let py = (y * SUBSAMPLES + sy) as f32 * step + step / 2.0;
                    let painted = inside(px, py) && !(struck && on_the_strike(px, py));
                    hits += u32::from(painted);
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
/// amber only while we are waiting for it to make up its mind, red for "off" — the state worth
/// noticing — and slate for "no idea yet", which is not the same as off and must not be painted as
/// if it were. An error is red too, and is told apart from "off" by the cut, not by the shade.
fn status_rgb(status: &ConnectionStatus) -> (u8, u8, u8) {
    match status {
        ConnectionStatus::Connected(_) => (76, 175, 80),
        ConnectionStatus::Connecting => (255, 179, 0),
        ConnectionStatus::Disconnected => (229, 57, 53),
        ConnectionStatus::Error(_) => (229, 57, 53),
        ConnectionStatus::Unknown => (96, 125, 139),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // The trait methods (`id`, `menu`, `tool_tip`) are only callable with it in scope.
    use ksni::Tray as _;
    use protonvpn_core::i18n::Locale;

    /// A tray item with a channel nobody reads: these tests assert on what the menu says, not on
    /// what a click sends.
    fn tray(view: TrayView) -> ProtonTray {
        ProtonTray::new(std::sync::mpsc::channel().0, view)
    }

    fn view(status: ConnectionStatus) -> TrayView {
        view_in(Locale::SOURCE, status)
    }

    /// A view in a chosen language, age line included: the engine composes that line before the
    /// tray ever sees it, so a Russian view with an English age would be a fixture, not a state.
    fn view_in(language: Locale, status: ConnectionStatus) -> TrayView {
        TrayView {
            status,
            language,
            detail: None,
            age_text: I18n::new(language).age_text(std::time::Duration::ZERO),
            update: None,
        }
    }

    /// The alpha channel, which is the silhouette on its own: the colour says which answer, the
    /// shape says which question.
    fn silhouette(icon: &ksni::Icon) -> Vec<u8> {
        icon.data.iter().step_by(4).copied().collect()
    }

    #[test]
    fn the_icon_is_a_square_argb_pixmap_of_the_expected_size() {
        let icon = status_icon(&ConnectionStatus::Connected(Default::default()));
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
        let icon = status_icon(&ConnectionStatus::Connected(Default::default()));
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
        // Green when the CLI says connected, red when it says there is nothing.
        assert_eq!(
            status_rgb(&ConnectionStatus::Connected(Default::default())),
            (76, 175, 80)
        );
        assert_eq!(status_rgb(&ConnectionStatus::Disconnected), (229, 57, 53));
    }

    #[test]
    fn no_connection_is_struck_through_and_nothing_else_is() {
        let size = ICON_SIZE as usize;
        let whole = silhouette(&status_icon(&ConnectionStatus::Connected(
            Default::default(),
        )));
        let cut = silhouette(&status_icon(&ConnectionStatus::Disconnected));

        // The cut only takes pixels away, and only from the diagonal band: a strike-through, not a
        // second shape. The band is `STRIKE_WIDTH / √2` either side of the diagonal in `x − y`,
        // plus the pixel and a half that the soft edge can spill into.
        let spill = STRIKE_WIDTH / std::f32::consts::SQRT_2 + 1.5 / size as f32;
        let mut removed = 0;
        for y in 0..size {
            for x in 0..size {
                let at = y * size + x;
                assert!(cut[at] <= whole[at], "the cut added a pixel at ({x}, {y})");
                if cut[at] < whole[at] {
                    removed += 1;
                    let (fx, fy) = (
                        (x as f32 + 0.5) / size as f32,
                        (y as f32 + 0.5) / size as f32,
                    );
                    assert!(
                        (fx - fy).abs() <= spill,
                        "took ({x}, {y}), which is off the band"
                    );
                }
            }
        }
        assert!(
            removed > size,
            "removed only {removed} pixels: not a strike-through"
        );

        // Every other status keeps the whole shield, the error included: "off" is the only state
        // that is ours to cut out, and an error is not the same thing as the tunnel being down.
        // The error text is the CLI's own and is data, so its language does not matter here.
        for status in [
            ConnectionStatus::Connecting,
            ConnectionStatus::Error("denied".into()),
            ConnectionStatus::Unknown,
        ] {
            assert_eq!(silhouette(&status_icon(&status)), whole, "{status:?}");
        }
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
        let tray = tray(TrayView {
            status: ConnectionStatus::Connected(Default::default()),
            language: Locale::SOURCE,
            detail: Some("NL#818 · Amsterdam, Netherlands".into()),
            age_text: "updated 3 mins ago".into(),
            update: None,
        });
        let tooltip = tray.tool_tip();
        assert_eq!(tooltip.title, "Proton VPN · Connected");
        assert!(tooltip.description.contains("NL#818"));
        assert!(tooltip.description.contains("updated 3 mins ago"));
        // The tray never mentions the runner: that is the window's business.
        assert!(!tooltip.description.contains("working"));
    }

    #[test]
    fn the_menu_offers_connect_disconnect_show_and_quit() {
        let tray = tray(view(ConnectionStatus::Disconnected));
        let labels: Vec<String> = tray
            .menu()
            .iter()
            .filter_map(|item| match item {
                MenuItem::Standard(standard) => Some(standard.label.clone()),
                _ => None,
            })
            .collect();
        assert!(labels.iter().any(|l| l.contains("Connect")));
        assert!(labels.iter().any(|l| l.contains("Disconnect")));
        assert!(labels.iter().any(|l| l.contains("Show window")));
        assert!(labels.iter().any(|l| l.contains("Quit")));
    }

    fn labels(tray: &ProtonTray) -> Vec<String> {
        tray.menu()
            .iter()
            .filter_map(|item| match item {
                MenuItem::Standard(standard) => Some(standard.label.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn an_update_is_a_menu_item_that_says_which_of_the_two_things_it_is() {
        let available = tray(TrayView {
            update: Some(TrayUpdate {
                version: Version::parse("0.1.43").unwrap(),
                applied: false,
            }),
            ..view(ConnectionStatus::Disconnected)
        });
        let available_labels = labels(&available);
        assert!(
            available_labels.iter().any(|l| l == "Update 0.1.43"),
            "{available_labels:?}"
        );
        // Above "Quit", and never instead of it.
        assert_eq!(available_labels.last().map(String::as_str), Some("Quit"));

        let applied = tray(TrayView {
            update: Some(TrayUpdate {
                version: Version::parse("0.1.43").unwrap(),
                applied: true,
            }),
            ..view(ConnectionStatus::Disconnected)
        });
        let applied_labels = labels(&applied);
        assert!(
            applied_labels
                .iter()
                .any(|l| l.contains("restart to apply")),
            "{applied_labels:?}"
        );

        // And nothing at all when there is nothing to say.
        let quiet = labels(&tray(view(ConnectionStatus::Disconnected)));
        assert!(!quiet.iter().any(|l| l.contains("Update")), "{quiet:?}");
    }

    /// The tray names the brand, and it is read from the catalogue like every other word — so a
    /// build whose catalogue were short would say so here rather than inventing a title.
    #[test]
    fn the_tray_title_is_the_catalogue_brand() {
        let tray = tray(view(ConnectionStatus::Unknown));
        assert_eq!(tray.title(), I18n::new(Locale::SOURCE).app_name());
        assert_eq!(tray.title(), "Proton VPN");
    }

    /// The menu in another language: translated, not copied. No Cyrillic literal is pinned here —
    /// wording belongs to the catalogue — so the test says what it can prove without it: every
    /// item moves, and the words that are the CLI's or Proton's own do not.
    #[test]
    fn the_menu_is_translated_and_not_copied() {
        let russian = Locale::from_id("ru").unwrap();
        let english = labels(&tray(view(ConnectionStatus::Disconnected)));
        let translated = labels(&tray(view_in(russian, ConnectionStatus::Disconnected)));

        assert_eq!(english.len(), translated.len());
        for (en, ru) in english.iter().zip(&translated) {
            assert_ne!(en, ru, "not translated: {en}");
        }
        // The status line is the status chip's own word, so it moves with it rather than being
        // spelled a second time in the tray.
        let i18n = I18n::new(russian);
        assert_eq!(
            translated.first().map(String::as_str),
            Some(
                i18n.tray_status(i18n.connection_label(&ConnectionStatus::Disconnected))
                    .as_str()
            )
        );
    }

    #[test]
    fn the_tray_id_is_ours_and_never_protons() {
        let tray = tray(view(ConnectionStatus::Unknown));
        assert_eq!(tray.id(), "protonvpn-gui");
        assert_ne!(tray.id(), "proton.vpn.app.gtk");
    }
}
