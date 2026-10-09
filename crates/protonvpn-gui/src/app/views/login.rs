//! The login page.
//!
//! It is a page, not a tab: until `protonvpn info` names an account there is nothing else the app
//! can honestly do, and hiding the form behind a tab made signing in look optional.
//!
//! There is no demo mode, because there is no demo: the wrapper runs `protonvpn signin` for real,
//! and a button that only pretends would be the one lie the whole design exists to avoid.

use iced::widget::{button, column, container, row, text, text_input};
use iced::{Alignment, Color, Element, Length, Padding, Theme};

use crate::app::{App, Message};
use crate::theme;
use crate::widgets::{self, Tone};

pub(crate) fn view(app: &App) -> Element<'_, Message> {
    let two_factor =
        app.shared.pending_prompt.as_ref().is_some_and(|pending| {
            pending.kind == protonvpn_core::interpreter::PromptKind::TwoFactor
        });

    let content = row![
        container(login_card(app, two_factor))
            .width(Length::Fixed(470.0))
            .padding(Padding::from([26, 28]))
            .style(theme::card),
        container(steps_card(app)).width(Length::FillPortion(1)),
    ]
    .spacing(48)
    .align_y(Alignment::Start);

    let page = container(content)
        .padding(Padding::from([48, 56]))
        .width(Length::Fill)
        .height(Length::Fill);

    column![page, super::console::view(app)]
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

fn login_card(app: &App, two_factor: bool) -> Element<'_, Message> {
    // Only when the user opened this page themselves: if the CLI says nobody is signed in, there
    // is nothing to go back to.
    let back: Element<'_, Message> = if app.login_was_forced() {
        button(text(app.i18n.login_back()).size(12))
            .padding(Padding::from([4, 10]))
            .style(theme::ghost(theme::TEXT_MUTED, false))
            .on_press(Message::SignInCancelled)
            .into()
    } else {
        iced::widget::Space::new()
            .width(Length::Fixed(1.0))
            .height(Length::Fixed(1.0))
            .into()
    };

    let logo = container(
        text("PV")
            .size(18)
            .style(|_: &Theme| iced::widget::text::Style {
                color: Some(Color::WHITE),
            }),
    )
    .width(Length::Fixed(40.0))
    .height(Length::Fixed(40.0))
    .center_x(Length::Fixed(40.0))
    .center_y(Length::Fixed(40.0))
    .style(|_: &Theme| container::Style {
        background: Some(iced::Background::Color(theme::TEXT)),
        border: iced::Border {
            radius: 10.0.into(),
            ..Default::default()
        },
        ..container::Style::default()
    });

    let step = |number: &'static str, label: String, active: bool| -> Element<'_, Message> {
        row![
            text(number)
                .size(12)
                .style(move |_: &Theme| iced::widget::text::Style {
                    color: Some(if active {
                        theme::ACCENT
                    } else {
                        theme::TEXT_FAINT
                    }),
                }),
            widgets::faint("·"),
            text(label)
                .size(12)
                .style(move |_: &Theme| iced::widget::text::Style {
                    color: Some(if active {
                        theme::ACCENT
                    } else {
                        theme::TEXT_FAINT
                    }),
                }),
        ]
        .spacing(6)
        .into()
    };

    let steps = row![
        step("1", app.i18n.login_progress_account(), !two_factor),
        step("2", app.i18n.login_progress_two_factor(), two_factor),
    ]
    .spacing(18);

    let form: Element<'_, Message> = if two_factor {
        column![
            text(app.i18n.login_two_factor_label()).size(13),
            row![
                text_input(
                    &app.i18n.login_two_factor_placeholder(),
                    &app.login_two_factor
                )
                .secure(true)
                .on_input(Message::LoginTwoFactor)
                .on_submit(Message::LoginTwoFactorSubmit)
                .padding(Padding::from([10, 12]))
                .width(Length::Fill),
                button(text(app.i18n.login_two_factor_submit()).size(13))
                    .padding(Padding::from([10, 16]))
                    .style(theme::filled(theme::ACCENT))
                    .on_press(Message::LoginTwoFactorSubmit),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
            widgets::muted(app.i18n.login_two_factor_note()),
        ]
        .spacing(8)
        .into()
    } else {
        column![
            text(app.i18n.login_username_label()).size(13),
            text_input(&app.i18n.login_username_placeholder(), &app.login_username)
                .on_input(Message::LoginUsername)
                .padding(Padding::from([10, 12])),
            text(app.i18n.login_password_label()).size(13),
            row![
                text_input("", &app.login_password)
                    .secure(!app.login_show_password)
                    .on_input(Message::LoginPassword)
                    .on_submit(Message::LoginSubmit)
                    .padding(Padding::from([10, 12]))
                    .width(Length::Fill),
                button(
                    text(if app.login_show_password {
                        app.i18n.login_password_hide()
                    } else {
                        app.i18n.login_password_show()
                    })
                    .size(12),
                )
                .padding(Padding::from([8, 10]))
                .style(theme::ghost(theme::TEXT_MUTED, false))
                .on_press(Message::LoginShowPassword(!app.login_show_password)),
            ]
            .spacing(6)
            .align_y(Alignment::Center),
            button(text(app.i18n.login_submit()).size(14))
                .padding(Padding::from([11, 18]))
                .width(Length::Fill)
                .style(theme::filled(theme::ACCENT))
                .on_press(Message::LoginSubmit),
        ]
        .spacing(8)
        .into()
    };

    column![
        row![
            logo,
            iced::widget::Space::new()
                .width(Length::Fill)
                .height(Length::Fixed(1.0)),
            back,
            // The window's way out, on this page too: a signed-out run has no sidebar to hold it,
            // and with no tray the close button is a refusal rather than a way out (§9).
            super::quit_button(app, Length::Shrink),
        ]
        .spacing(8)
        .align_y(Alignment::Center),
        text(app.i18n.login_title()).size(24),
        widgets::muted(app.i18n.login_explainer()),
        widgets::separator(),
        steps,
        form,
        widgets::faint(app.i18n.login_pty_note()),
    ]
    .spacing(16)
    .into()
}

fn steps_card(app: &App) -> Element<'static, Message> {
    // The digits are the list's own numbering, not a sentence: they are the same in every
    // language, and only the sentences beside them are translated.
    let items: [(&str, String); 4] = [
        ("1", app.i18n.login_steps_1()),
        ("2", app.i18n.login_steps_2()),
        ("3", app.i18n.login_steps_3()),
        ("4", app.i18n.login_steps_4()),
    ];

    column![
        widgets::eyebrow(app.i18n.login_steps_title()),
        column(items.into_iter().map(|(number, value)| {
            row![
                widgets::monogram(number, Tone::Neutral),
                text(value).size(13).width(Length::Fill),
            ]
            .spacing(12)
            .align_y(Alignment::Center)
            .into()
        }))
        .spacing(12),
    ]
    .spacing(16)
    .into()
}

#[cfg(test)]
mod tests {
    use protonvpn_core::i18n::{I18n, Locale};

    #[test]
    fn the_sign_in_page_reads_in_the_source_language() {
        let i18n = I18n::new(Locale::SOURCE);
        assert_eq!(i18n.login_title(), "Sign in to Proton VPN");
        assert_eq!(i18n.login_submit(), "Continue");
        assert_eq!(i18n.login_two_factor_placeholder(), "6 digits");
        assert_eq!(i18n.login_password_show(), "Show");
        assert_eq!(i18n.login_password_hide(), "Hide");
    }

    #[test]
    fn the_security_promise_says_where_the_secrets_go() {
        let english = I18n::new(Locale::SOURCE);
        let promise = english.login_explainer();
        // The command name is data: it is the program that actually runs.
        assert!(promise.contains("protonvpn signin"), "{promise}");
        assert!(promise.contains("never reach the console"), "{promise}");

        // A translation is a translation: the same facts, not the same sentence. The command name
        // survives it either way, because it is not a word of ours.
        let russian = I18n::new(Locale::from_id("ru").unwrap());
        assert_ne!(russian.login_explainer(), english.login_explainer());
        assert!(russian.login_explainer().contains("protonvpn signin"));
        assert!(russian.login_pty_note().contains("PTY"));
    }
}
