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
    let brand = row![
        widgets::monogram("PV", widgets::Tone::Accent),
        column![
            text("ProtonVPN").size(15),
            widgets::faint("обёртка над CLI"),
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
                text(page.label()).size(14),
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
        text(status.label()).size(13),
    ]
    .spacing(8)
    .align_y(Alignment::Center);

    let account: Element<'_, Message> = match app.account_name() {
        Some(name) => column![
            row![
                avatar(name),
                column![
                    text(name.to_string()).size(13),
                    widgets::faint("protonvpn info"),
                ]
                .spacing(1)
                .width(Length::Fill),
                button(text("Выйти").size(12))
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
            widgets::faint("Аккаунт ещё не назван"),
            widgets::faint(app.account_age().unwrap_or_else(|| "—".into())),
            // Never leave the user with no way to sign in: the gate above is a claim about what
            // the CLI said, and a claim can be wrong.
            button(text("Войти").size(12))
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
    notice: &'a str,
    content: Element<'a, Message>,
) -> Element<'a, Message> {
    let bar = container(
        row![
            text(notice).size(13).width(Length::Fill),
            button(text("ок").size(12))
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
