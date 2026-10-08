//! The window's pages, and the pieces they share.
//!
//! Views are functions of the app state and nothing else: they read a snapshot, they return an
//! [`Element`], and the only way they can change anything is by producing a [`Message`]. No view
//! touches the engine.

pub(crate) mod console;
pub(crate) mod editor;
pub(crate) mod login;
pub(crate) mod overview;
pub(crate) mod settings;

use iced::widget::{Space, button, column, container, row, rule, text};
use iced::{Alignment, Color, Element, Length, Padding, Theme};

use crate::app::{App, Message, Page};
use crate::theme;
use crate::widgets;
use protonvpn_core::model::ConnectionStatus;

/// The tone a connection status is drawn in. One place, so the sidebar, the status card and the
/// console bar can never disagree about what "connected" looks like.
pub(crate) fn status_tone(status: &ConnectionStatus) -> widgets::Tone {
    match status {
        ConnectionStatus::Connected(_) => widgets::Tone::Success,
        ConnectionStatus::Connecting => widgets::Tone::Warning,
        ConnectionStatus::Disconnected => widgets::Tone::Neutral,
        ConnectionStatus::Error(_) => widgets::Tone::Danger,
        ConnectionStatus::Unknown => widgets::Tone::Neutral,
    }
}

/// The left rail: where you are, what the CLI last said, and who you are.
pub(crate) fn sidebar(app: &App) -> Element<'_, Message> {
    // The brand block. The monogram is a mark rather than a word and is not translated; the name
    // and the line under it are the catalogue's, and the name is the same string the window title
    // and the tray item carry.
    let brand = row![
        widgets::monogram("PV", widgets::Tone::Accent),
        column![
            text(app.i18n.app_name()).size(15),
            widgets::faint(app.i18n.chrome_brand_subtitle()),
        ]
        .spacing(1),
    ]
    .spacing(10)
    .align_y(Alignment::Center);

    let nav = column(Page::ALL.iter().map(|page| {
        let active = *page == app.page;
        button(
            row![
                // A drawn rule, not a glyph: the fonts a Linux desktop actually has do not carry
                // the icon characters a design mock-up can use, and a substituted box is worse
                // than no icon at all.
                nav_marker(active),
                text(page.label(&app.i18n)).size(14),
            ]
            .spacing(10)
            .align_y(Alignment::Center),
        )
        .width(Length::Fill)
        .width(Length::Fill)
        .padding(Padding::from([9, 12]))
        .style(theme::ghost(theme::TEXT, active))
        .on_press(Message::PageSelected(*page))
        .into()
    }))
    .spacing(4);

    let status = &app.shared.state.connection.value;
    let status_line = row![
        widgets::dot(status_tone(status).color()),
        text(app.i18n.connection_label(status)).size(13),
    ]
    .spacing(8)
    .align_y(Alignment::Center);

    let account: Element<'_, Message> = match app.account_name() {
        Some(name) => column![
            row![
                avatar(name),
                column![
                    text(name.to_string()).size(13),
                    // The command that named the account, spelled as `protonvpn` spells it: a
                    // command line is data, like every other byte the console shows.
                    widgets::faint("protonvpn info"),
                ]
                .spacing(1)
                .width(Length::Fill),
                button(text(app.i18n.chrome_sign_out()).size(12))
                    .padding(Padding::from([4, 8]))
                    .style(theme::ghost(theme::TEXT_MUTED, false))
                    .on_press(Message::Logout),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        ]
        .spacing(6)
        .into(),
        None => column![
            widgets::faint(app.i18n.chrome_account_unnamed()),
            widgets::faint(app.account_age().unwrap_or_else(|| "—".into())),
            // Never leave the user with no way to sign in: the gate above is a claim about what
            // the CLI said, and a claim can be wrong.
            button(text(app.i18n.chrome_sign_in()).size(12))
                .padding(Padding::from([4, 8]))
                .style(theme::ghost(theme::ACCENT, false))
                .on_press(Message::SignInRequested),
        ]
        .spacing(6)
        .into(),
    };

    let rail = container(
        column![
            brand,
            nav,
            iced::widget::Space::new()
                .width(Length::Fill)
                .height(Length::Fill),
            rule::vertical(1.0).style(|_: &Theme| rule::Style {
                color: theme::BORDER,
                radius: 0.0.into(),
                fill_mode: rule::FillMode::Full,
                snap: false,
            }),
            status_line,
            account,
            quit_button(app, Length::Fill),
        ]
        .spacing(14)
        .padding(Padding::from([16, 14])),
    )
    .width(Length::Fixed(238.0))
    .height(Length::Fill)
    .style(theme::sidebar);

    row![
        rail,
        rule::vertical(1.0).style(|_: &Theme| rule::Style {
            color: theme::BORDER,
            radius: 0.0.into(),
            fill_mode: rule::FillMode::Full,
            snap: false,
        })
    ]
    .height(Length::Fill)
    .into()
}

/// The window's own way out.
///
/// The tray has a "Quit" of its own, but the tray is not always there: GNOME without the
/// AppIndicator extension has no StatusNotifierItem host at all, and a panel that has not appeared
/// yet is not one either. Without this button the only exit in that case was a signal from outside
/// — the close button refuses on purpose, because hiding the window would hide the whole
/// application into nothing (`docs/architecture.md` §9). One word, from the same catalogue entry
/// the tray menu uses, so the two can never disagree about what quitting is called.
pub(crate) fn quit_button<'a>(app: &'a App, width: Length) -> Element<'a, Message> {
    button(text(app.i18n.tray_quit()).size(12))
        .padding(Padding::from([6, 10]))
        .width(width)
        .style(theme::ghost(theme::TEXT_MUTED, false))
        .on_press(Message::Quit)
        .into()
}

/// The little accent rule that marks the current page.
fn nav_marker<'a>(active: bool) -> Element<'a, Message> {
    container(
        Space::new()
            .width(Length::Fixed(3.0))
            .height(Length::Fixed(16.0)),
    )
    .style(move |_: &Theme| container::Style {
        background: Some(iced::Background::Color(if active {
            theme::ACCENT
        } else {
            Color::TRANSPARENT
        })),
        border: iced::Border {
            radius: 2.0.into(),
            ..Default::default()
        },
        ..container::Style::default()
    })
    .into()
}

/// A round monogram from an account name: `user@proton.me` becomes `UP`.
fn avatar<'a>(name: &'a str) -> Element<'a, Message> {
    let local = name.split('@').next().unwrap_or(name);
    let initials: String = local
        .split(['.', '_', '-', ' '])
        .filter_map(|part| part.chars().next())
        .take(2)
        .collect::<String>()
        .to_uppercase();
    let initials = if initials.is_empty() {
        "?".to_string()
    } else {
        initials
    };

    container(
        text(initials)
            .size(11)
            .style(|_: &Theme| iced::widget::text::Style {
                color: Some(Color::WHITE),
            }),
    )
    .width(Length::Fixed(28.0))
    .height(Length::Fixed(28.0))
    .center_x(Length::Fixed(28.0))
    .center_y(Length::Fixed(28.0))
    .style(|_: &Theme| container::Style {
        background: Some(iced::Background::Color(theme::TEXT)),
        border: iced::Border {
            radius: 14.0.into(),
            ..Default::default()
        },
        ..container::Style::default()
    })
    .into()
}

/// The dismissible remark bar. It is ours, not the CLI's, so it never pretends to be output.
pub(crate) fn notice_bar<'a>(
    app: &'a App,
    notice: &'a str,
    content: Element<'a, Message>,
) -> Element<'a, Message> {
    let bar = container(
        row![
            text(notice).size(13).width(Length::Fill),
            button(text(app.i18n.chrome_notice_dismiss()).size(12))
                .padding(Padding::from([4, 10]))
                .style(theme::outlined(theme::BORDER, theme::TEXT_MUTED))
                .on_press(Message::DismissNotice),
        ]
        .spacing(8)
        .align_y(Alignment::Center),
    )
    .padding(Padding::from([8, 14]))
    .width(Length::Fill)
    .style(|_: &Theme| container::Style {
        background: Some(iced::Background::Color(theme::WARNING_WEAK)),
        border: iced::Border {
            color: theme::WARNING,
            width: 1.0,
            radius: 0.0.into(),
        },
        ..container::Style::default()
    });

    column![bar, content].into()
}
