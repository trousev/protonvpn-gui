//! The console pane — `docs/architecture.md` §4, the core UI element.
//!
//! Collapsed, it is one line: what the runner is doing, what the connection state is, and how old
//! that knowledge is. Expanded, it is the transcript, verbatim, with the exit code and duration of
//! every invocation. It is read-only, and it is deliberately the last thing in the window: the
//! bottom of the screen is where a person looks when something is happening.

use iced::widget::{Id, button, column, container, row, rule, scrollable, text};
use iced::{Alignment, Element, Font, Length, Padding, Theme};

use crate::app::{App, CONSOLE_ID, Message};
use crate::theme;
use crate::widgets;

pub(crate) fn view(app: &App) -> Element<'_, Message> {
    let connection = &app.shared.state.connection;
    // The way out of a command that is waiting for something that will never come.
    let busy = !matches!(app.shared.runner, protonvpn_core::model::RunnerStatus::Idle);
    let bar = container(
        row![
            button(
                text(if app.console_expanded {
                    "Свернуть"
                } else {
                    "Транскрипт"
                })
                .size(12)
            )
            .padding(Padding::from([3, 9]))
            .style(theme::ghost(theme::TEXT_MUTED, false))
            .on_press(Message::ToggleConsole),
            widgets::dot(crate::app::views::status_tone(&connection.value).color()),
            text(app.i18n.runner_label(&app.shared.runner))
                .size(12)
                .font(Font::MONOSPACE),
            widgets::faint("·"),
            text(app.i18n.connection_label(&connection.value)).size(12),
            widgets::faint("·"),
            text(app.i18n.age_text(connection.age())).size(12),
            iced::widget::Space::new()
                .width(Length::Fill)
                .height(Length::Fixed(1.0)),
            if busy {
                button(text("Прервать").size(12))
                    .padding(Padding::from([3, 9]))
                    .style(theme::outlined(theme::BORDER, theme::DANGER))
                    .on_press(Message::CancelRun)
                    .into()
            } else {
                widgets::eyebrow("консоль только для чтения")
            },
        ]
        .spacing(8)
        .align_y(Alignment::Center),
    )
    .padding(Padding::from([6, 12]))
    .width(Length::Fill);

    let rule = rule::horizontal(1.0).style(|_: &Theme| rule::Style {
        color: theme::BORDER,
        radius: 0.0.into(),
        fill_mode: rule::FillMode::Full,
        snap: false,
    });

    if !app.console_expanded {
        return column![rule, bar].into();
    }

    let controls = container(
        row![
            widgets::faint("транскрипт · вывод CLI показан дословно"),
            iced::widget::Space::new()
                .width(Length::Fill)
                .height(Length::Fixed(1.0)),
            button(text("Вниз").size(12))
                .padding(Padding::from([4, 10]))
                .style(theme::outlined(theme::BORDER, theme::TEXT_MUTED))
                .on_press(Message::ScrollToBottom),
            button(text("Копировать всё").size(12))
                .padding(Padding::from([4, 10]))
                .style(theme::outlined(theme::BORDER, theme::TEXT))
                .on_press(Message::CopyAll),
        ]
        .spacing(8)
        .align_y(Alignment::Center),
    )
    .padding(Padding::from([6, 12]))
    .width(Length::Fill);

    let mut list = column![].spacing(6);
    if app.console.dropped_invocations > 0 {
        list = list.push(widgets::faint(format!(
            "… {} более ранних вызовов вытеснено из буфера",
            app.console.dropped_invocations
        )));
    }
    if app.console.is_empty() {
        list = list.push(widgets::faint(
            "Пока ничего не запускалось. Каждая команда появится здесь дословно.",
        ));
    }
    for block in &app.console.blocks {
        list = list.push(block_view(app, block));
    }

    column![
        rule,
        bar,
        controls,
        container(
            scrollable(list)
                .id(Id::new(CONSOLE_ID))
                .on_scroll(Message::ConsoleScrolled)
                .height(Length::Fill),
        )
        .height(Length::Fixed(250.0))
        .padding(Padding::from([8, 12]))
        .width(Length::Fill)
        .style(theme::flat_card),
    ]
    .into()
}

fn block_view<'a>(_app: &'a App, block: &'a crate::console::Block) -> Element<'a, Message> {
    let header = row![
        text(format!("$ {}", block.command))
            .font(Font::MONOSPACE)
            .size(12)
            .width(Length::Fill),
        button(text("Копировать").size(11))
            .padding(Padding::from([2, 8]))
            .style(theme::ghost(theme::TEXT_FAINT, false))
            .on_press(Message::CopyInvocation(block.id)),
    ]
    .spacing(8)
    .align_y(Alignment::Center);

    let mut body = column![header].spacing(2);
    if block.lines_hidden > 0 {
        body = body.push(widgets::faint(format!(
            "… {} строк скрыто (лимит показа)",
            block.lines_hidden
        )));
    }
    if !block.output.is_empty() {
        body = body.push(
            text(block.output.as_str())
                .font(Font::MONOSPACE)
                .size(12)
                .width(Length::Fill),
        );
    }
    body = body.push(
        text(block.footer.as_str())
            .font(Font::MONOSPACE)
            .size(11)
            .style(move |_: &Theme| iced::widget::text::Style {
                color: Some(if block.running {
                    theme::WARNING
                } else {
                    theme::TEXT_FAINT
                }),
            }),
    );

    container(body)
        .padding(Padding::from([8, 10]))
        .width(Length::Fill)
        .style(|_: &Theme| container::Style {
            background: Some(iced::Background::Color(theme::SURFACE)),
            border: iced::Border {
                color: theme::BORDER,
                width: 1.0,
                radius: 8.0.into(),
            },
            ..container::Style::default()
        })
        .into()
}
