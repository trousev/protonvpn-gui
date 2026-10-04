//! The window's palette, and the styles the design is built out of.
//!
//! The look is a light, quiet shell: grey page, white cards, one accent blue for the thing the
//! user is supposed to press. Nothing here encodes state — a colour that means "connected" lives
//! next to the connection status in `views`, so there is exactly one place that decides what
//! green means.
//!
//! The palette is deliberately small. Every style function below takes the colours it needs and
//! derives hover and pressed states from them, so a widget cannot invent a shade.

use iced::widget::{button, container};
use std::sync::OnceLock;

use iced::{Background, Border, Color, Shadow, Theme};

pub const BG: Color = Color::from_rgb(0.949, 0.953, 0.961); // #F2F3F5
pub const SURFACE: Color = Color::from_rgb(1.0, 1.0, 1.0);
pub const SURFACE_MUTED: Color = Color::from_rgb(0.972, 0.976, 0.980); // #F8F9FA
pub const BORDER: Color = Color::from_rgb(0.890, 0.902, 0.921); // #E3E6EB

pub const TEXT: Color = Color::from_rgb(0.086, 0.094, 0.114); // #16181D
pub const TEXT_MUTED: Color = Color::from_rgb(0.420, 0.447, 0.502); // #6B7280
pub const TEXT_FAINT: Color = Color::from_rgb(0.604, 0.631, 0.675); // #9AA1AC

pub const ACCENT: Color = Color::from_rgb(0.184, 0.435, 0.922); // #2F6FEB
pub const ACCENT_WEAK: Color = Color::from_rgb(0.910, 0.937, 0.988); // #E8EFFC
pub const ACCENT_BORDER: Color = Color::from_rgb(0.725, 0.804, 0.965);

pub const SUCCESS: Color = Color::from_rgb(0.086, 0.639, 0.290); // #16A34A
pub const SUCCESS_WEAK: Color = Color::from_rgb(0.906, 0.965, 0.929);
pub const WARNING: Color = Color::from_rgb(0.851, 0.467, 0.024); // #D97706
pub const WARNING_WEAK: Color = Color::from_rgb(0.992, 0.953, 0.890);
pub const DANGER: Color = Color::from_rgb(0.863, 0.149, 0.149); // #DC2626
pub const DANGER_WEAK: Color = Color::from_rgb(0.992, 0.925, 0.925);
pub const NEUTRAL: Color = Color::from_rgb(0.541, 0.573, 0.616); // #8A929D
pub const NEUTRAL_WEAK: Color = Color::from_rgb(0.949, 0.957, 0.969);

/// The base theme, built once. Built-in widget styling (toggler, pick list, text input) derives
/// from this palette, so those widgets land in the same family as our own styles.
pub fn app() -> Theme {
    static THEME: OnceLock<Theme> = OnceLock::new();
    THEME
        .get_or_init(|| {
            Theme::custom(
                "Proton VPN".to_string(),
                iced::theme::Palette {
                    background: BG,
                    text: TEXT,
                    primary: ACCENT,
                    success: SUCCESS,
                    warning: WARNING,
                    danger: DANGER,
                },
            )
        })
        .clone()
}

pub fn card(_theme: &Theme) -> container::Style {
    container::Style {
        text_color: Some(TEXT),
        background: Some(Background::Color(SURFACE)),
        border: Border {
            color: BORDER,
            width: 1.0,
            radius: 10.0.into(),
        },
        // No drop shadow, deliberately. The software renderer (tiny-skia) is what this app ships
        // with, and a blurred translucent quad is the one thing it does not composite reliably:
        // measured, a card's shadow accumulates into a grey wash across the card over the first
        // ten seconds of ticking. A hairline border reads just as well and cannot go wrong.
        shadow: Shadow::default(),
        snap: false,
    }
}

/// A card that is not asking to be looked at: no border, slightly grey.
pub fn flat_card(_theme: &Theme) -> container::Style {
    container::Style {
        text_color: Some(TEXT),
        background: Some(Background::Color(SURFACE_MUTED)),
        border: Border {
            color: Color::TRANSPARENT,
            width: 0.0,
            radius: 8.0.into(),
        },
        shadow: Shadow::default(),
        snap: false,
    }
}

pub fn tile(_theme: &Theme) -> container::Style {
    container::Style {
        text_color: Some(TEXT),
        background: Some(Background::Color(SURFACE)),
        border: Border {
            color: BORDER,
            width: 1.0,
            radius: 8.0.into(),
        },
        shadow: Shadow::default(),
        snap: false,
    }
}

pub fn sidebar(_theme: &Theme) -> container::Style {
    container::Style {
        text_color: Some(TEXT),
        background: Some(Background::Color(SURFACE)),
        border: Border {
            color: BORDER,
            width: 1.0,
            radius: 0.0.into(),
        },
        shadow: Shadow::default(),
        snap: false,
    }
}

/// The dimming layer behind a modal. Reads as "the page is still there, it is just not yours
/// right now".
pub fn backdrop(_theme: &Theme) -> container::Style {
    container::Style {
        text_color: None,
        background: Some(Background::Color(Color {
            a: 0.35,
            ..Color::BLACK
        })),
        ..container::Style::default()
    }
}

/// A subtle pill: the badges in the connection list, the status chip.
pub fn pill(background: Color) -> impl Fn(&Theme) -> container::Style {
    move |_theme: &Theme| container::Style {
        text_color: Some(TEXT),
        background: Some(Background::Color(background)),
        border: Border {
            color: Color::TRANSPARENT,
            width: 0.0,
            radius: 6.0.into(),
        },
        shadow: Shadow::default(),
        snap: false,
    }
}

/// A filled button: the primary action, and nothing else.
pub fn filled(
    background: Color,
) -> impl Fn(&Theme, button::Status) -> button::Style + Clone + 'static {
    move |_theme, status| {
        let bg = match status {
            button::Status::Hovered => mix(background, Color::BLACK, 0.10),
            button::Status::Pressed => mix(background, Color::BLACK, 0.20),
            button::Status::Disabled => mix(background, BG, 0.55),
            _ => background,
        };
        button::Style {
            background: Some(Background::Color(bg)),
            text_color: Color::WHITE,
            border: Border {
                color: Color::TRANSPARENT,
                width: 0.0,
                radius: 8.0.into(),
            },
            shadow: Shadow::default(),
            snap: false,
        }
    }
}

/// An outlined button: everything else.
pub fn outlined(
    border: Color,
    text_color: Color,
) -> impl Fn(&Theme, button::Status) -> button::Style + Clone + 'static {
    move |_theme, status| {
        let (bg, border_color) = match status {
            button::Status::Hovered => (SURFACE_MUTED, mix(border, Color::BLACK, 0.15)),
            button::Status::Pressed => (mix(SURFACE_MUTED, Color::BLACK, 0.06), border),
            button::Status::Disabled => (SURFACE, mix(border, SURFACE, 0.5)),
            _ => (SURFACE, border),
        };
        button::Style {
            background: Some(Background::Color(bg)),
            text_color,
            border: Border {
                color: border_color,
                width: 1.0,
                radius: 8.0.into(),
            },
            shadow: Shadow::default(),
            snap: false,
        }
    }
}

/// A button with no chrome: the sidebar entries, the small icon actions.
pub fn ghost(
    text_color: Color,
    active: bool,
) -> impl Fn(&Theme, button::Status) -> button::Style + Clone + 'static {
    move |_theme, status| {
        let bg = if active {
            ACCENT_WEAK
        } else {
            match status {
                button::Status::Hovered => SURFACE_MUTED,
                button::Status::Pressed => mix(SURFACE_MUTED, Color::BLACK, 0.06),
                _ => Color::TRANSPARENT,
            }
        };
        button::Style {
            background: Some(Background::Color(bg)),
            text_color: if active { ACCENT } else { text_color },
            border: Border {
                color: Color::TRANSPARENT,
                width: 0.0,
                radius: 8.0.into(),
            },
            shadow: Shadow::default(),
            snap: false,
        }
    }
}

/// A connection row: selected rows get the same wash as an active sidebar entry.
pub fn connection_card(selected: bool) -> impl Fn(&Theme) -> container::Style + Clone {
    move |_theme: &Theme| container::Style {
        text_color: Some(TEXT),
        background: Some(Background::Color(if selected {
            ACCENT_WEAK
        } else {
            SURFACE
        })),
        border: Border {
            color: if selected { ACCENT_BORDER } else { BORDER },
            width: 1.0,
            radius: 10.0.into(),
        },
        shadow: Shadow::default(),
        snap: false,
    }
}

/// A button with no chrome and no hover wash: the clickable body of a card that draws its own
/// background and border.
pub fn bare() -> impl Fn(&Theme, button::Status) -> button::Style + Clone {
    move |_theme, _status| button::Style {
        background: None,
        text_color: TEXT,
        border: Border {
            color: Color::TRANSPARENT,
            width: 0.0,
            radius: 0.0.into(),
        },
        shadow: Shadow::default(),
        snap: false,
    }
}

/// `mix(a, b, t)` — linear interpolation, for hover and pressed states. Kept here so that no
/// widget needs its own shades of the accent.
fn mix(a: Color, b: Color, t: f32) -> Color {
    Color {
        r: a.r + (b.r - a.r) * t,
        g: a.g + (b.g - a.g) * t,
        b: a.b + (b.b - a.b) * t,
        a: a.a,
    }
}
