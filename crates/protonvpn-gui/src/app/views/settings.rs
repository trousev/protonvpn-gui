//! The settings page — `protonvpn config list` in a form a person can use, plus the handful of
//! settings that are ours.
//!
//! Two rules shape it. First, **nothing is invented**: a row exists because the CLI printed that
//! key in `config list`, and a key we have never seen still appears, under its own name, in
//! «Общие». Second, every write is followed by a fresh `config list`, because our own command
//! succeeding is not evidence that the CLI agreed (`docs/architecture.md` §5).

use iced::widget::{
    Space, button, checkbox, column, container, pick_list, row, scrollable, text, text_input,
};
use iced::{Alignment, Element, Length, Padding, Theme};

use protonvpn_core::model::Setting;

use crate::app::{App, AppToggle, Message, SettingsTab, setting_label, setting_values};
use crate::theme;
use crate::widgets::{self, Tone};

/// Which of the CLI's settings belong on which tab. A key that appears here is rendered there and
/// nowhere else; a key the CLI grows tomorrow falls through to «Общие».
fn cli_keys(tab: SettingsTab) -> &'static [&'static str] {
    match tab {
        SettingsTab::General => &["kill-switch", "ipv6", "anonymous-crash-reports"],
        SettingsTab::Connection => &["netshield", "vpn-accelerator", "moderate-nat", "custom-dns"],
        SettingsTab::Port => &["port-forwarding"],
        SettingsTab::Polling => &[],
        SettingsTab::Account => &[],
    }
}

/// Every key we put somewhere. Anything outside this list is shown in «Общие» rather than hidden.
const KNOWN_KEYS: &[&str] = &[
    "netshield",
    "kill-switch",
    "port-forwarding",
    "custom-dns",
    "vpn-accelerator",
    "moderate-nat",
    "ipv6",
    "anonymous-crash-reports",
];

pub(crate) fn view(app: &App) -> Element<'_, Message> {
    let header = row![
        column![
            widgets::eyebrow("Настройки"),
            text("Параметры CLI").size(22),
        ]
        .spacing(2)
        .width(Length::Fill),
        button(text("Прочитать config list").size(13))
            .padding(Padding::from([8, 14]))
            .style(theme::outlined(theme::BORDER, theme::TEXT))
            .on_press(Message::RefreshSettings),
    ]
    .align_y(Alignment::Center)
    .width(Length::Fill);

    let tabs =
        row(SettingsTab::ALL
            .iter()
            .map(|tab| {
                let active = *tab == app.settings_tab;
                button(text(tab.label()).size(13).style(move |_: &Theme| {
                    iced::widget::text::Style {
                        color: Some(if active {
                            theme::ACCENT
                        } else {
                            theme::TEXT_MUTED
                        }),
                    }
                }))
                .padding(Padding::from([6, 2]))
                .style(theme::bare())
                .on_press(Message::SettingsTabSelected(*tab))
                .into()
            })
            .collect::<Vec<Element<'_, Message>>>())
        .spacing(22)
        .align_y(Alignment::Center);

    let mut content = column![].spacing(14);
    if !cli_keys(app.settings_tab).is_empty() {
        content = content.push(cli_card(app, app.settings_tab));
    }
    content = content.push(app_card(app, app.settings_tab));

    scrollable(column![header, tabs, widgets::separator(), content].spacing(16))
        .height(Length::Fill)
        .into()
}

// --- the CLI's own settings -----------------------------------------------------------------

fn cli_card(app: &App, tab: SettingsTab) -> Element<'_, Message> {
    let Some(observation) = &app.shared.state.settings else {
        return widgets::card(
            column![
                widgets::eyebrow("protonvpn config list"),
                widgets::muted(
                    "Настройки CLI ещё не прочитаны. Одна команда, около секунды — и здесь \
                     появится то, что CLI ответил."
                ),
                button(text("Прочитать config list").size(13))
                    .padding(Padding::from([8, 14]))
                    .style(theme::filled(theme::ACCENT))
                    .on_press(Message::RefreshSettings),
            ]
            .spacing(12),
        );
    };

    let wanted = cli_keys(tab);
    let mut rows: Vec<Element<'_, Message>> = Vec::new();

    for key in wanted {
        if let Some(setting) = observation.value.iter().find(|setting| &setting.key == key) {
            rows.push(setting_row(app, setting));
        }
    }

    // A key the CLI has and we do not describe goes on the first tab, verbatim. Dropping it would
    // be a lie of omission about what the CLI can do.
    if tab == SettingsTab::General {
        for setting in &observation.value {
            if !KNOWN_KEYS.contains(&setting.key.as_str()) {
                rows.push(setting_row(app, setting));
            }
        }
    }

    let head = row![
        widgets::eyebrow("protonvpn config list"),
        Space::new().width(Length::Fill).height(Length::Fixed(1.0)),
        widgets::faint(observation.age_text()),
    ]
    .align_y(Alignment::Center);

    widgets::card(column![head, column(rows).spacing(10)].spacing(12))
}

fn setting_row<'a>(app: &'a App, setting: &'a Setting) -> Element<'a, Message> {
    let (name, description) = match setting_label(&setting.key) {
        Some((name, description)) => (name.to_string(), description.to_string()),
        None => (
            setting.key.clone(),
            "Ключ, которого нет в описании приложения: показан как есть.".to_string(),
        ),
    };

    let control: Element<'a, Message> = match setting_values(&setting.key) {
        // Exactly two values and the first is `off`: a switch is honest, and its "on" position
        // means the second value — `standard`, in the kill switch's case. Three or more values
        // are a choice, and a choice is a list.
        Some([off, on]) if *off == "off" => {
            let key = setting.key.clone();
            let on = on.to_string();
            iced::widget::toggler(setting.value == on)
                .text_size(13)
                .on_toggle(move |value| Message::SettingChoice {
                    key: key.clone(),
                    value: if value { on.clone() } else { "off".to_string() },
                })
                .into()
        }
        Some(values) => {
            let key = setting.key.clone();
            let options: Vec<Choice> = values
                .iter()
                .map(|value| Choice {
                    label: value_label(&setting.key, value).to_string(),
                    value: value.to_string(),
                })
                .collect();
            let selected = options
                .iter()
                .find(|choice| choice.value == setting.value)
                .cloned();
            pick_list(options, selected, move |choice| Message::SettingChoice {
                key: key.clone(),
                value: choice.value,
            })
            .text_size(13)
            .width(Length::Fixed(240.0))
            .into()
        }
        None => {
            let key = setting.key.clone();
            row![
                text_input("значение", app.setting_draft(&setting.key))
                    .on_input({
                        let key = key.clone();
                        move |value| Message::SettingDraft {
                            key: key.clone(),
                            value,
                        }
                    })
                    .on_submit(Message::SettingApply { key: key.clone() })
                    .padding(Padding::from([7, 10]))
                    .width(Length::Fixed(180.0)),
                button(text("Применить").size(12))
                    .padding(Padding::from([6, 12]))
                    .style(theme::outlined(theme::BORDER, theme::TEXT))
                    .on_press(Message::SettingApply { key: key.clone() }),
            ]
            .spacing(6)
            .align_y(Alignment::Center)
            .into()
        }
    };

    let mut details = column![
        row![
            text(name).size(14),
            widgets::badge(setting.key.clone(), Tone::Neutral),
        ]
        .spacing(8)
        .align_y(Alignment::Center),
        widgets::muted(description),
    ]
    .spacing(3);

    // `custom-dns on` is meaningless without servers, so the servers travel with the setting.
    if setting.key == "custom-dns" && setting.value == "on" {
        details = details.push(
            row![
                widgets::faint("Серверы"),
                text_input("1.1.1.1,8.8.8.8", &app.dns_servers)
                    .on_input(Message::DnsChanged)
                    .padding(Padding::from([6, 10]))
                    .width(Length::Fixed(240.0)),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        );
    }

    container(
        row![
            container(details).width(Length::Fill),
            container(control).width(Length::Shrink),
        ]
        .spacing(12)
        .align_y(Alignment::Center),
    )
    .padding(Padding::from([10, 12]))
    .width(Length::Fill)
    .style(theme::flat_card)
    .into()
}

/// A `<select>` option that shows Russian and carries the CLI's value.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Choice {
    value: String,
    label: String,
}

impl std::fmt::Display for Choice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.label)
    }
}

fn value_label<'a>(key: &str, value: &'a str) -> &'a str {
    match (key, value) {
        ("netshield", "off") => "выключен",
        ("netshield", "malware-only") => "только вредоносные",
        ("netshield", "malware-ads-trackers") => "вредоносные, реклама и трекеры",
        ("kill-switch", "off") => "выключен",
        ("kill-switch", "standard") => "стандартный",
        (_, "on") => "включено",
        (_, "off") => "выключено",
        (_, other) => other,
    }
}

// --- our own settings -----------------------------------------------------------------------

fn app_card(app: &App, tab: SettingsTab) -> Element<'_, Message> {
    match tab {
        SettingsTab::General => widgets::card(
            column![
                widgets::eyebrow("Приложение"),
                widget_toggle(
                    app.config.autostart,
                    "Автозапуск",
                    "Держать ~/.config/autostart/protonvpn-gui.desktop в соответствии с этой \
                     настройкой.",
                    AppToggle::Autostart,
                ),
                widget_toggle(
                    app.config.desktop_entry,
                    "Ярлык в меню приложений",
                    "Установить .desktop и иконку в ~/.local/share. Это не украшение: на Wayland \
                     иконки у окна нет, и рабочий стол узнаёт имя и иконку только из .desktop — \
                     без него GNOME показывает окно как «Неизвестное приложение».",
                    AppToggle::DesktopEntry,
                ),
                widget_toggle(
                    app.config.start_minimized,
                    "Запускать свёрнутым в трей",
                    "Старт без окна: приложение сразу живёт в трее.",
                    AppToggle::StartMinimized,
                ),
                widgets::faint(format!(
                    "Автозапуск: {} · ярлык: {} · настройки приложения хранятся в \
                     ~/.config/protonvpn-gui/config.json. Файлы официального приложения мы не \
                     читаем и не пишем.",
                    if app.desktop.autostart_enabled() {
                        "есть"
                    } else {
                        "нет"
                    },
                    if app.desktop.entry_installed() {
                        "есть"
                    } else {
                        "нет"
                    }
                )),
            ]
            .spacing(12),
        ),
        SettingsTab::Connection => widgets::card(
            column![
                widgets::eyebrow("Приложение"),
                widget_toggle(
                    app.config.connect_at_startup,
                    "Подключаться при запуске",
                    "Поднимает выбранное соединение сразу после старта — в том числе когда окна \
                     нет и приложение живёт в трее.",
                    AppToggle::ConnectAtStartup,
                ),
                widgets::muted(format!(
                    "Сейчас выбрано: {} · $ {}",
                    app.selected_name(),
                    super::overview::selected_argv(app)
                )),
            ]
            .spacing(12),
        ),
        SettingsTab::Port => widgets::card(
            column![
                widgets::eyebrow("Приложение · qBittorrent"),
                widgets::muted(
                    "Необязательная передача проброшенного порта в локальный qBittorrent. \
                     Выключено по умолчанию: включение меняет настройки другой программы."
                ),
                checkbox(app.config.qbittorrent.enabled)
                    .label("Передавать порт в qBittorrent (localhost, Web API)")
                    .text_size(13)
                    .on_toggle(Message::QbEnabled),
                row![
                    widgets::faint("Хост"),
                    text_input("localhost", &app.qb_host)
                        .on_input(Message::QbHost)
                        .padding(Padding::from([7, 10]))
                        .width(Length::Fixed(200.0)),
                    widgets::faint("Порт"),
                    text_input("8080", &app.qb_port)
                        .on_input(Message::QbPort)
                        .padding(Padding::from([7, 10]))
                        .width(Length::Fixed(90.0)),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
                row![
                    widgets::faint("Логин"),
                    text_input("обычно пусто", &app.qb_username)
                        .on_input(Message::QbUsername)
                        .padding(Padding::from([7, 10]))
                        .width(Length::Fixed(200.0)),
                    widgets::faint("Пароль"),
                    text_input("не сохраняется", &app.qb_password)
                        .secure(true)
                        .on_input(Message::QbPassword)
                        .padding(Padding::from([7, 10]))
                        .width(Length::Fixed(200.0)),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
                row![
                    button(text("Сохранить и передать порт").size(13))
                        .padding(Padding::from([8, 14]))
                        .style(theme::filled(theme::ACCENT))
                        .on_press(Message::QbPushNow),
                    widgets::faint(
                        "Пароль живёт только в памяти процесса. Соединение только с localhost."
                    ),
                ]
                .spacing(10)
                .align_y(Alignment::Center),
                widgets::faint(
                    "Каждая передача видна в консоли как отдельная запись: это HTTP-запрос, а не \
                     команда protonvpn, и выдавать его за неё нельзя."
                ),
            ]
            .spacing(12),
        ),
        SettingsTab::Polling => {
            let age = app.shared.state.connection.age_text();
            widgets::card(
                column![
                    widgets::eyebrow("Как мы узнаём состояние"),
                    widgets::muted(
                        "protonvpn status стоит около секунды: каждый вызов запускает Python. \
                         Поэтому опрос устроен так:"
                    ),
                    bullet("спокойный режим — не чаще одного раза в 5 минут;"),
                    bullet("сразу после команды, которая могла изменить туннель;"),
                    bullet("когда вы смотрите — при открытии окна или клике по трею."),
                    widgets::note(format!(
                        "Следствие, которое мы не скрываем: состояние может быть старым. Рядом со \
                         статусом всегда стоит возраст: {age}."
                    )),
                    widget_toggle(
                        app.config.probe_enabled,
                        "Проверять внешний адрес через curl",
                        "Исключение №1: единственный внешний вызов помимо protonvpn. Отвечает \
                         на вопрос, которого CLI не может — идёт ли трафик через туннель.",
                        AppToggle::Probe,
                    ),
                ]
                .spacing(12),
            )
        }
        SettingsTab::Account => {
            let mut content = column![widgets::eyebrow("Аккаунт")].spacing(12);

            match app.account_name() {
                Some(name) => {
                    content = content.push(
                        row![
                            text(format!("Вы вошли как {name}"))
                                .size(15)
                                .width(Length::Fill),
                            button(text("Выйти").size(13))
                                .padding(Padding::from([7, 14]))
                                .style(theme::outlined(theme::BORDER, theme::DANGER))
                                .on_press(Message::Logout),
                        ]
                        .align_y(Alignment::Center),
                    );
                    if let Some(age) = app.account_age() {
                        content = content.push(widgets::faint(format!("protonvpn info · {age}")));
                    }
                }
                None => {
                    content = content.push(
                        row![
                            widgets::muted(
                                "Аккаунт не подтверждён: `protonvpn info` не назвал имя."
                            ),
                            Space::new().width(Length::Fill).height(Length::Fixed(1.0)),
                            button(text("Войти").size(13))
                                .padding(Padding::from([7, 14]))
                                .style(theme::filled(theme::ACCENT))
                                .on_press(Message::SignInRequested),
                        ]
                        .align_y(Alignment::Center),
                    );
                }
            }

            content = content.push(widgets::separator());
            content = content.push(widgets::muted(
                "Пароль и код 2FA уходят прямо в PTY-терминал процесса CLI и никогда не попадают \
                 в консоль: она показывает только то, что напечатал CLI.",
            ));

            content = content.push(match &app.shared.pending_prompt {
                Some(pending) => column![
                    widgets::muted(format!("CLI ждёт ввода: {}", prompt_label(pending))),
                    row![
                        text_input("значение", &app.manual_input)
                            .secure(true)
                            .on_input(Message::ManualInput)
                            .on_submit(Message::ManualSend)
                            .padding(Padding::from([7, 10]))
                            .width(Length::Fixed(260.0)),
                        button(text("Отправить").size(13))
                            .padding(Padding::from([7, 14]))
                            .style(theme::filled(theme::ACCENT))
                            .on_press(Message::ManualSend),
                    ]
                    .spacing(8)
                    .align_y(Alignment::Center),
                ]
                .spacing(8)
                .into(),
                None => widgets::faint(
                    "Если CLI задаст вопрос, который мы не смогли распознать, поле появится \
                     здесь, а сам вопрос будет виден в консоли.",
                ),
            });

            widgets::card(content)
        }
    }
}

fn bullet<'a>(value: &'a str) -> Element<'a, Message> {
    row![widgets::faint("•"), widgets::muted(value)]
        .spacing(8)
        .into()
}

fn widget_toggle<'a>(
    value: bool,
    label: &'a str,
    description: &'a str,
    which: AppToggle,
) -> Element<'a, Message> {
    container(
        row![
            container(
                column![
                    text(label.to_string()).size(14),
                    widgets::muted(description.to_string()),
                ]
                .spacing(3),
            )
            .width(Length::Fill),
            iced::widget::toggler(value)
                .text_size(13)
                .on_toggle(move |value| Message::Toggle(which, value)),
        ]
        .spacing(12)
        .align_y(Alignment::Center),
    )
    .padding(Padding::from([10, 12]))
    .width(Length::Fill)
    .style(theme::flat_card)
    .into()
}

fn prompt_label(pending: &protonvpn_core::engine::PendingPrompt) -> &'static str {
    match pending.kind {
        protonvpn_core::interpreter::PromptKind::Password => "пароль",
        protonvpn_core::interpreter::PromptKind::TwoFactor => "код 2FA",
        protonvpn_core::interpreter::PromptKind::Unrecognised => {
            "неопознанный запрос — смотрите консоль"
        }
    }
}
