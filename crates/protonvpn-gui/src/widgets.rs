//! Small pieces the two pages share: labels, badges, tiles, the row that holds a value.
//!
//! Everything here is presentation only. Where a widget would need to know what a colour *means*
//! — connected, failed, unavailable — it takes a [`Tone`], and the caller decides. That keeps
//! `views` the one place where state turns into a verdict.

use iced::widget::{Space, column, container, row, text};
use iced::{Alignment, Element, Length, Padding, Theme};

use crate::theme;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Neutral,
    Accent,
    Success,
    Warning,
    Danger,
}

impl Tone {
    /// The tone's colour. The point of routing colour through a tone is that a view says what it
    /// means — success, danger — and this module decides what that looks like.
    pub fn color(self) -> iced::Color {
        self.strong()
    }

    fn strong(self) -> iced::Color {
        match self {
            Self::Neutral => theme::NEUTRAL,
            Self::Accent => theme::ACCENT,
            Self::Success => theme::SUCCESS,
            Self::Warning => theme::WARNING,
            Self::Danger => theme::DANGER,
        }
    }

    fn weak(self) -> iced::Color {
        match self {
            Self::Neutral => theme::NEUTRAL_WEAK,
            Self::Accent => theme::ACCENT_WEAK,
            Self::Success => theme::SUCCESS_WEAK,
            Self::Warning => theme::WARNING_WEAK,
            Self::Danger => theme::DANGER_WEAK,
        }
    }
}

/// The small uppercase caption above a block: `ОБЗОР`, `СОЕДИНЕНИЯ`.
pub fn eyebrow<'a, M: 'a>(value: impl Into<String>) -> Element<'a, M> {
    text(value.into().to_uppercase())
        .size(11)
        .style(|_: &Theme| text::Style {
            color: Some(theme::TEXT_FAINT),
        })
        .into()
}

pub fn muted<'a, M: 'a>(value: impl Into<String>) -> Element<'a, M> {
    text(value.into())
        .size(12)
        .style(|_: &Theme| text::Style {
            color: Some(theme::TEXT_MUTED),
        })
        .into()
}

pub fn faint<'a, M: 'a>(value: impl Into<String>) -> Element<'a, M> {
    text(value.into())
        .size(11)
        .style(|_: &Theme| text::Style {
            color: Some(theme::TEXT_FAINT),
        })
        .into()
}

pub fn badge<'a, M: 'a>(value: impl Into<String>, tone: Tone) -> Element<'a, M> {
    container(
        text(value.into().to_uppercase())
            .size(10)
            .style(move |_: &Theme| text::Style {
                color: Some(tone.strong()),
            }),
    )
    .padding(Padding::from([2, 6]))
    .style(theme::pill(tone.weak()))
    .into()
}

/// A coloured dot, for a status chip.
pub fn dot<'a, M: 'a>(color: iced::Color) -> Element<'a, M> {
    container(
        Space::new()
            .width(Length::Fixed(8.0))
            .height(Length::Fixed(8.0)),
    )
    .style(move |_: &Theme| container::Style {
        background: Some(iced::Background::Color(color)),
        border: iced::Border {
            radius: 4.0.into(),
            ..Default::default()
        },
        ..container::Style::default()
    })
    .into()
}

/// A status chip: `● Отключено`. The dot and the words share the tone; the words themselves are
/// the caller's, because only the caller knows what is actually known.
pub fn status_chip<'a, M: 'a>(tone: Tone, label: impl Into<String>) -> Element<'a, M> {
    let color = tone.color();
    container(
        row![
            dot(color),
            text(label.into())
                .size(13)
                .style(move |_: &Theme| text::Style { color: Some(color) }),
        ]
        .spacing(6)
        .align_y(Alignment::Center),
    )
    .padding(Padding::from([4, 10]))
    .style(theme::pill(tone.weak()))
    .into()
}

/// A white card with a border. The basic building block of both pages.
pub fn card<'a, M: 'a>(content: impl Into<Element<'a, M>>) -> Element<'a, M> {
    container(content)
        .padding(16)
        .width(Length::Fill)
        .style(theme::card)
        .into()
}

/// One of the four facts under the connection status: `СЕРВЕР / NL#818`.
pub fn tile<'a, M: 'a>(label: &'a str, value: impl Into<String>) -> Element<'a, M> {
    container(
        column![
            text(label.to_uppercase())
                .size(10)
                .style(|_: &Theme| text::Style {
                    color: Some(theme::TEXT_FAINT)
                }),
            text(value.into()).size(14),
        ]
        .spacing(6),
    )
    .padding(Padding::from([10, 12]))
    .width(Length::FillPortion(1))
    .style(theme::tile)
    .into()
}

/// `label ......... value`, as the egress probe card reads.
pub fn fact_row<'a, M: 'a>(label: &'a str, value: impl Into<String>) -> Element<'a, M> {
    let value = value.into();
    row![
        text(label.to_string())
            .size(13)
            .width(Length::Fill)
            .style(|_: &Theme| text::Style {
                color: Some(theme::TEXT_MUTED)
            }),
        text(value)
            .size(13)
            .font(iced::Font::MONOSPACE)
            .style(|_: &Theme| text::Style {
                color: Some(theme::TEXT)
            }),
    ]
    .align_y(Alignment::Center)
    .into()
}

/// The dashed separator GitHub-style lists use between facts.
pub fn separator<'a, M: 'a>() -> Element<'a, M> {
    container(Space::new().width(Length::Fill).height(Length::Fixed(1.0)))
        .style(|_: &Theme| container::Style {
            background: Some(iced::Background::Color(theme::BORDER)),
            ..container::Style::default()
        })
        .into()
}

/// The square monogram on a connection row: `NL`, `P2P`, `∞`.
pub fn monogram<'a, M: 'a>(value: impl Into<String>, tone: Tone) -> Element<'a, M> {
    container(
        text(value.into())
            .size(12)
            .style(move |_: &Theme| text::Style {
                color: Some(tone.strong()),
            }),
    )
    .width(Length::Fixed(34.0))
    .height(Length::Fixed(34.0))
    .center_x(Length::Fixed(34.0))
    .center_y(Length::Fixed(34.0))
    .style(move |_: &Theme| container::Style {
        background: Some(iced::Background::Color(tone.weak())),
        border: iced::Border {
            radius: 8.0.into(),
            ..Default::default()
        },
        ..container::Style::default()
    })
    .into()
}

/// An informational line with a left accent bar — used for the "the probe runs after connecting"
/// note, which is an explanation and not a warning.
pub fn note<'a, M: 'a>(value: impl Into<String>) -> Element<'a, M> {
    container(text(value.into()).size(12).style(|_: &Theme| text::Style {
        color: Some(theme::TEXT_MUTED),
    }))
    .padding(Padding::from([8, 12]))
    .width(Length::Fill)
    .style(theme::flat_card)
    .into()
}
