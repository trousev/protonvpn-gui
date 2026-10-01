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
        container(steps_card()).width(Length::FillPortion(1)),
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
        button(text("Назад").size(12))
            .padding(Padding::from([4, 10]))
            .style(theme::ghost(theme::TEXT_MUTED, false))
            .on_press(Message::SignInCancelled)
            .into()
    } else {
        iced::widget::Space::new(Length::Fixed(1.0), Length::Fixed(1.0)).into()
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

    let step = |number: &'static str, label: &'static str, active: bool| -> Element<'_, Message> {
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
        step("1", "Аккаунт", !two_factor),
        step("2", "Двухфакторный код", two_factor),
    ]
    .spacing(18);

    let form: Element<'_, Message> = if two_factor {
        column![
            text("Код двухфакторной аутентификации").size(13),
            row![
                text_input("6 цифр", &app.login_two_factor)
                    .secure(true)
                    .on_input(Message::LoginTwoFactor)
                    .on_submit(Message::LoginTwoFactorSubmit)
                    .padding(Padding::from([10, 12]))
                    .width(Length::Fill),
                button(text("Подтвердить").size(13))
                    .padding(Padding::from([10, 16]))
                    .style(theme::filled(theme::ACCENT))
                    .on_press(Message::LoginTwoFactorSubmit),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
            widgets::muted(
                "CLI запросил код в PTY-сессии — он уходит прямо в процесс и не появляется ни в \
                 аргументах, ни в консоли.",
            ),
        ]
        .spacing(8)
        .into()
    } else {
        column![
            text("Имя пользователя Proton").size(13),
            text_input("user@proton.me", &app.login_username)
                .on_input(Message::LoginUsername)
                .padding(Padding::from([10, 12])),
            text("Пароль").size(13),
            row![
                text_input("", &app.login_password)
                    .secure(!app.login_show_password)
                    .on_input(Message::LoginPassword)
                    .on_submit(Message::LoginSubmit)
                    .padding(Padding::from([10, 12]))
                    .width(Length::Fill),
                button(
                    text(if app.login_show_password {
                        "Скрыть"
                    } else {
                        "Показать"
                    })
                    .size(12),
                )
                .padding(Padding::from([8, 10]))
                .style(theme::ghost(theme::TEXT_MUTED, false))
                .on_press(Message::LoginShowPassword(!app.login_show_password)),
            ]
            .spacing(6)
            .align_y(Alignment::Center),
            button(text("Продолжить").size(14))
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
            iced::widget::Space::new(Length::Fill, Length::Fixed(1.0)),
            back,
        ]
        .align_y(Alignment::Center),
        text("Вход в ProtonVPN").size(24),
        widgets::muted(
            "Обёртка выполняет protonvpn signin и передаёт пароль и код напрямую в PTY. Секреты \
             не попадают в консоль.",
        ),
        widgets::separator(),
        steps,
        form,
        widgets::faint(
            "Ввод идёт в PTY · транскрипт ниже показывает только имена команд и вывод CLI."
        ),
    ]
    .spacing(16)
    .into()
}

fn steps_card() -> Element<'static, Message> {
    let items: [(&str, &str); 4] = [
        ("1", "protonvpn signin запускается в PTY-сессии."),
        (
            "2",
            "Пароль пишется в поток процесса, а не в аргументы командной строки.",
        ),
        (
            "3",
            "Если на аккаунте включён 2FA, CLI запрашивает код — поле появляется здесь же.",
        ),
        (
            "4",
            "В консоли остаются только имена команд и вывод CLI, без секретов.",
        ),
    ];

    column![
        widgets::eyebrow("Что происходит во время входа"),
        column(items.iter().map(|(number, value)| {
            let number = number.to_string();
            row![
                widgets::monogram(number, Tone::Neutral),
                text(value.to_string()).size(13).width(Length::Fill),
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
