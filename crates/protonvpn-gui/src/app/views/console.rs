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
                    app.i18n.console_collapse()
                } else {
                    app.i18n.console_transcript()
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
                button(text(app.i18n.console_cancel()).size(12))
                    .padding(Padding::from([3, 9]))
                    .style(theme::outlined(theme::BORDER, theme::DANGER))
                    .on_press(Message::CancelRun)
                    .into()
            } else {
                widgets::eyebrow(app.i18n.console_read_only())
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
            widgets::faint(app.i18n.console_transcript_caption()),
            iced::widget::Space::new()
                .width(Length::Fill)
                .height(Length::Fixed(1.0)),
            button(text(app.i18n.console_bottom()).size(12))
                .padding(Padding::from([4, 10]))
                .style(theme::outlined(theme::BORDER, theme::TEXT_MUTED))
                .on_press(Message::ScrollToBottom),
            button(text(app.i18n.console_copy_all()).size(12))
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
        list = list.push(widgets::faint(
            app.i18n
                .console_dropped(app.console.dropped_invocations as i64),
        ));
    }
    if app.console.is_empty() {
        list = list.push(widgets::faint(app.i18n.console_empty()));
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

fn block_view<'a>(app: &'a App, block: &'a crate::console::Block) -> Element<'a, Message> {
    let header = row![
        text(format!("$ {}", block.command))
            .font(Font::MONOSPACE)
            .size(12)
            .width(Length::Fill),
        button(text(app.i18n.console_copy()).size(11))
            .padding(Padding::from([2, 8]))
            .style(theme::ghost(theme::TEXT_FAINT, false))
            .on_press(Message::CopyInvocation(block.id)),
    ]
    .spacing(8)
    .align_y(Alignment::Center);

    let mut body = column![header].spacing(2);
    if block.lines_hidden > 0 {
        body = body.push(widgets::faint(
            app.i18n.console_lines_hidden(block.lines_hidden as i64),
        ));
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

#[cfg(test)]
mod tests {
    use protonvpn_core::i18n::{I18n, Locale};

    fn english() -> I18n {
        I18n::new(Locale::SOURCE)
    }

    /// How many different sentence *shapes* a set of counts produces, the digits removed. The
    /// number is in every sentence, so only the words around it can distinguish CLDR's categories;
    /// counting the shapes is how "English has two forms, Russian has three" becomes a test.
    fn shapes(texts: [String; 6]) -> usize {
        let mut seen: Vec<String> = Vec::new();
        for text in texts {
            let shape: String = text.chars().filter(|c| !c.is_ascii_digit()).collect();
            if !seen.contains(&shape) {
                seen.push(shape);
            }
        }
        seen.len()
    }

    /// Tests run in the source language, so this is the wording the pane actually draws. Pinning it
    /// here means a change to the catalogue has to be a decision made twice.
    #[test]
    fn the_console_wording_is_the_catalogue() {
        let i18n = english();
        assert_eq!(i18n.console_collapse(), "Collapse");
        assert_eq!(i18n.console_transcript(), "Transcript");
        assert_eq!(i18n.console_cancel(), "Cancel");
        assert_eq!(i18n.console_read_only(), "console is read-only");
        assert_eq!(
            i18n.console_transcript_caption(),
            "transcript · the CLI's output is shown verbatim"
        );
        assert_eq!(i18n.console_bottom(), "Bottom");
        assert_eq!(i18n.console_copy_all(), "Copy all");
        assert_eq!(i18n.console_copy(), "Copy");
        assert_eq!(
            i18n.console_empty(),
            "Nothing has run yet. Every command will appear here verbatim."
        );
    }

    /// The two counts in this pane are plural selections, and English has two forms where Russian
    /// has three. A bare `{ $count }` would render as a string in Russian and silently stop
    /// declining the noun, so the numbers go through the selection itself.
    #[test]
    fn the_counts_select_a_plural() {
        let i18n = english();
        assert_eq!(
            i18n.console_dropped(1),
            "… 1 earlier invocation dropped from the buffer"
        );
        assert_eq!(
            i18n.console_dropped(3),
            "… 3 earlier invocations dropped from the buffer"
        );
        assert_eq!(
            i18n.console_lines_hidden(1),
            "… 1 line hidden (display limit)"
        );
        assert_eq!(
            i18n.console_lines_hidden(50),
            "… 50 lines hidden (display limit)"
        );
    }

    /// The one test that is about Russian: the catalogue is built explicitly, because the pane
    /// runs in the source language everywhere else. Russian selects on `[one]`, `[few]` and
    /// `[many]`, and a translation that collapsed them into English's two forms would still
    /// compile and still render — this is the only place that would notice. It asserts shapes
    /// rather than sentences on purpose: no second copy of the wording to keep in step.
    #[test]
    fn russian_counts_in_more_than_two_forms() {
        let russian = I18n::new(Locale::from_id("ru").unwrap());
        let source = english();

        for (one, few, many) in [(1, 3, 5), (21, 22, 25)] {
            let forms = [one, few, many].map(|count| russian.console_dropped(count));
            assert_ne!(forms[0], forms[1], "{one} and {few} read the same");
            assert_ne!(forms[1], forms[2], "{few} and {many} read the same");
            assert_ne!(forms[0], forms[2], "{one} and {many} read the same");
            // The number itself survives into the sentence, and no `$count` is left raw.
            for (count, text) in [one, few, many].into_iter().zip(&forms) {
                assert!(text.contains(&count.to_string()), "{text}");
                assert!(!text.contains("count"), "{text}");
            }
        }

        // Two forms are where English stops; Russian does not. Over a set of counts that covers
        // both languages' categories, English collapses to two sentences and Russian needs three —
        // which is the whole reason a count is a selection and not a number pasted into a phrase.
        let counts = [1, 3, 5, 21, 22, 25];
        assert_eq!(shapes(counts.map(|count| source.console_dropped(count))), 2);
        assert_eq!(
            shapes(counts.map(|count| russian.console_dropped(count))),
            3
        );
        assert_eq!(
            shapes(counts.map(|count| russian.console_lines_hidden(count))),
            3
        );
        assert_ne!(
            russian.console_dropped(3),
            source.console_dropped(3),
            "the Russian catalogue is a translation, not a copy"
        );
    }
}
