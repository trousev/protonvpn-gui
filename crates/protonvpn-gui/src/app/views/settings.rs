//! The settings page — `protonvpn config list` in a form a person can use, plus the handful of
//! settings that are ours.
//!
//! Two rules shape it. First, **nothing is invented**: a row exists because the CLI printed that
//! key in `config list`, and a key we have never seen still appears, under its own name, in
//! «Общие». Second, every write is followed by a fresh `config list`, because our own command
//! succeeding is not evidence that the CLI agreed (`docs/architecture.md` §5).

use iced::widget::{
    Space, button, checkbox, column, container, pick_list, radio, row, scrollable, text, text_input,
};
use iced::{Alignment, Element, Length, Padding, Theme};

use protonvpn_core::config::UpdatePolicy;
use protonvpn_core::engine::{UpdatePhase, UpdateView};
use protonvpn_core::model::{Setting, render_age};
use protonvpn_core::socks5::{Closed, GateState};
use protonvpn_core::update::human_bytes;

use crate::app::{App, AppToggle, Message, SettingsTab, setting_label, setting_values};
use crate::theme;
use crate::widgets::{self, Tone};

/// Which of the CLI's settings belong on which tab. A key that appears here is rendered there and
/// nowhere else; a key the CLI grows tomorrow falls through to «Общие».
fn cli_keys(tab: SettingsTab) -> &'static [&'static str] {
    match tab {
        SettingsTab::General => &["kill-switch", "ipv6", "anonymous-crash-reports"],
        SettingsTab::Connection => &[
            "netshield",
            "vpn-accelerator",
            "moderate-nat",
            "custom-dns",
            "port-forwarding",
        ],
        SettingsTab::Proxy => &[],
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
    // First on «Общие», because it is the one card that may need an answer today — and because the
    // tray's update item lands here.
    if app.settings_tab == SettingsTab::General {
        content = content.push(updates_card(app));
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
                     нет и приложение живёт в трее. Если CLI уже сообщает о подключении, туннель \
                     не трогаем: connect по живому подключению молча меняет сервер.",
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
        SettingsTab::Proxy => socks5_card(app),
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

/// The local SOCKS5 proxy — exception #3, and the one screen where a paranoid setting is allowed to
/// explain itself at length (`docs/architecture.md` §13).
///
/// The point of the whole screen is the state line: an application pointed at this address must be
/// able to tell whether it is actually protected, and "the proxy is running" is not the same claim
/// as "the proxy will refuse when the tunnel is gone".
fn socks5_card(app: &App) -> Element<'_, Message> {
    let view = &app.shared.socks5;
    let stats = view.stats.snapshot();

    let (tone, headline, advice): (Tone, String, Option<String>) = match view.gate.state() {
        GateState::Open { source } => (
            Tone::Success,
            format!("открыт · маршрут подтверждён: {source}"),
            None,
        ),
        GateState::Closed(reason) => {
            let tone = match reason {
                Closed::Disabled | Closed::NotConnected => Tone::Neutral,
                _ => Tone::Warning,
            };
            let advice = match &reason {
                Closed::Disabled => None,
                Closed::NotConnected => {
                    Some("Прокси откроется сам, как только CLI сообщит о подключении.".to_string())
                }
                Closed::Unverified { .. } => Some(
                    "Приложение не видело, каким маршрут был до подключения, и не может \
                     утверждать, что нынешний — туннель. Переподключитесь (Отключить, затем \
                     Подключиться): тогда маршрут подтвердится."
                        .to_string(),
                ),
                Closed::RouteChanged { .. } | Closed::RouteLost { .. } => Some(
                    "Переподключитесь, чтобы подтвердить туннель заново: пока это не сделано, \
                     прокси не выпустит ни байта."
                        .to_string(),
                ),
                Closed::EgressIsBaseline { .. } => Some(
                    "Проверка внешнего адреса увидела тот же адрес, что и до подключения. \
                     Переподключитесь."
                        .to_string(),
                ),
                Closed::ProbeUnanswered { .. } => Some(
                    "Внешняя проверка туннеля не отвечает: без неё остаётся только локальная \
                     проверка маршрута, а она видит не всё. Проверьте связь и переподключитесь, \
                     чтобы туннель подтвердился заново."
                        .to_string(),
                ),
                Closed::NotListening { .. } => {
                    Some("Проверьте адрес и порт: слушать можно только localhost.".to_string())
                }
            };
            (tone, format!("закрыт · {}", reason.describe()), advice)
        }
    };

    let mut content = column![
        widgets::eyebrow("Приложение · SOCKS5"),
        widgets::muted(
            "Локальный SOCKS5-прокси для программ, которые должны ходить в сеть только через \
             VPN: приложение настраивается на этот адрес, а прокси отказывает всему, пока не \
             подтверждено, что трафик идёт через туннель. Выключено по умолчанию."
        ),
        checkbox(app.config.socks5.enabled)
            .label("Включить локальный SOCKS5-прокси")
            .text_size(13)
            .on_toggle(Message::Socks5Enabled),
        row![
            widgets::faint("Адрес"),
            text_input("127.0.0.1", &app.socks5_address)
                .on_input(Message::Socks5Address)
                .padding(Padding::from([7, 10]))
                .width(Length::Fixed(170.0)),
            widgets::faint("Порт"),
            text_input("1080", &app.socks5_port)
                .on_input(Message::Socks5Port)
                .padding(Padding::from([7, 10]))
                .width(Length::Fixed(90.0)),
            widgets::faint("Проверка туннеля, с"),
            text_input("30", &app.socks5_verify)
                .on_input(Message::Socks5Verify)
                .padding(Padding::from([7, 10]))
                .width(Length::Fixed(70.0)),
            button(text("Применить").size(13))
                .padding(Padding::from([8, 14]))
                .style(theme::filled(theme::ACCENT))
                .on_press(Message::Socks5Apply),
        ]
        .spacing(8)
        .align_y(Alignment::Center),
        widgets::separator(),
        row![
            widgets::status_chip(tone, headline),
            Space::new().width(Length::Fill).height(Length::Fixed(1.0)),
            button(text("Копировать адрес").size(12))
                .padding(Padding::from([6, 12]))
                .style(theme::outlined(theme::BORDER, theme::TEXT))
                .on_press(Message::CopySocks5),
        ]
        .spacing(10)
        .align_y(Alignment::Center),
        if app
            .copied_socks5_at
            .is_some_and(|at| at.elapsed() < std::time::Duration::from_secs(2))
        {
            widgets::muted(format!("{} скопирован", app.socks5_address_text()))
        } else {
            widgets::faint(format!(
                "Настройте приложение на {} (SOCKS5, без аутентификации).",
                app.socks5_address_text()
            ))
        },
        widgets::muted(format!(
            "принято {} · отказано {} · активно {} · передано {} / {}{}",
            stats.accepted,
            stats.refused,
            stats.active,
            human_bytes(stats.up),
            human_bytes(stats.down),
            if stats.overloaded > 0 {
                format!(" · нет места {}", stats.overloaded)
            } else {
                String::new()
            },
        )),
    ]
    .spacing(12);

    if let Some(advice) = advice {
        content = content.push(widgets::note(advice));
    }
    if !app.config.probe_enabled {
        content = content.push(widgets::faint(
            "Внешняя проверка туннеля выключена вместе с проверкой внешнего адреса на вкладке \
             «Опрос»: остаётся только локальная проверка маршрута, каждые 200 мс.",
        ));
    }

    content = content.push(widgets::faint(
        "Только localhost и только IPv4; из команд SOCKS5 — только CONNECT. Аутентификации нет: \
         порт слушает петлевой интерфейс, ровно как у `ssh -D`.",
    ));
    content = content.push(widgets::faint(
        "Маршрут ядра перечитывается каждые 200 мс — без пакетов и без третьих сторон. Раз в \
         указанное число секунд туннель подтверждается внешним адресом той же проверкой curl, что \
         и на «Обзоре»; 0 выключает её, локальная проверка остаётся.",
    ));

    widgets::card(content)
}

// --- updates (exception #4) ------------------------------------------------------------------

/// Every policy, in the order the card shows them: from "leave me alone" to "do everything".
const UPDATE_POLICIES: [UpdatePolicy; 4] = [
    UpdatePolicy::Off,
    UpdatePolicy::Notify,
    UpdatePolicy::Download,
    UpdatePolicy::Install,
];

fn policy_label(policy: UpdatePolicy) -> &'static str {
    match policy {
        UpdatePolicy::Off => "не проверять",
        UpdatePolicy::Notify => "только сообщать",
        UpdatePolicy::Download => "скачивать",
        UpdatePolicy::Install => "скачивать и ставить",
    }
}

/// The AppImage updater — exception #4 (`docs/architecture.md` §14).
///
/// The card answers three questions in order: what this build is, what the release page last said,
/// and what the application may do about it without being asked. The state line is the point of it:
/// it says which true thing is the case, including the two that are easy to lie about — "a verified
/// image is waiting for a restart" and "this build is not an AppImage and cannot replace itself".
fn updates_card(app: &App) -> Element<'_, Message> {
    let update = &app.shared.update;
    let (tone, headline) = update_headline(update);
    let busy = matches!(
        update.phase,
        UpdatePhase::Checking | UpdatePhase::Downloading { .. }
    );
    // Cancelling is offered while bytes are moving, and only then: a check is one small file and
    // over in a moment, so a button that visibly did nothing would be worse than no button.
    let downloading = matches!(update.phase, UpdatePhase::Downloading { .. });
    let staged = matches!(update.phase, UpdatePhase::Staged { .. });
    let installed = matches!(update.phase, UpdatePhase::Installed { .. });
    let behind = match (update.current, update.latest) {
        (Some(current), Some(latest)) => current < latest,
        _ => false,
    };

    let mut checked = format!("Версия: {}", crate::version::label());
    match update.checked_at {
        Some(at) => checked.push_str(&format!(
            " · страница релизов: {}",
            render_age(
                std::time::SystemTime::now()
                    .duration_since(at)
                    .unwrap_or_default()
            )
        )),
        None => checked.push_str(" · страницу релизов ещё не спрашивали"),
    }

    let mut buttons = row![
        button(text("Проверить сейчас").size(13))
            .padding(Padding::from([8, 14]))
            .style(theme::outlined(theme::BORDER, theme::TEXT))
            .on_press_maybe((!busy).then_some(Message::UpdateCheckNow)),
    ]
    .spacing(8)
    .align_y(Alignment::Center);

    if downloading {
        buttons = buttons.push(
            button(text("Отменить").size(13))
                .padding(Padding::from([8, 14]))
                .style(theme::outlined(theme::BORDER, theme::DANGER))
                .on_press(Message::UpdateCancel),
        );
    } else if behind && update.replaceable && !installed {
        buttons = buttons.push(
            button(
                text(if staged {
                    "Установить"
                } else {
                    "Скачать и установить"
                })
                .size(13),
            )
            .padding(Padding::from([8, 14]))
            .style(theme::filled(theme::ACCENT))
            .on_press(Message::UpdateInstall),
        );
    }
    if behind && !update.dismissed && !installed {
        buttons = buttons.push(
            button(text("Не напоминать").size(12))
                .padding(Padding::from([6, 12]))
                .style(theme::bare())
                .on_press(Message::UpdateDismiss),
        );
    }

    let mut options = row![].spacing(18).align_y(Alignment::Center);
    for policy in UPDATE_POLICIES {
        options = options.push(
            radio(
                policy_label(policy),
                policy,
                Some(app.config.update.policy),
                Message::UpdatePolicySelected,
            )
            .text_size(13)
            .size(15),
        );
    }

    let mut content = column![
        widgets::eyebrow("Приложение · Обновления"),
        widgets::muted(
            "AppImage не обновляет никто, кроме него самого: приложение спрашивает свою страницу \
             релизов, сверяет скачанное с SHA256SUMS из того же релиза и подменяет файл на месте. \
             Скачанное никогда не запускается само — новая версия заработает при следующем \
             запуске, а предыдущая остаётся рядом как <имя>.old на один запуск."
        ),
        options,
        widgets::faint(checked),
        widgets::separator(),
        row![
            widgets::status_chip(tone, headline),
            Space::new().width(Length::Fill).height(Length::Fixed(1.0)),
        ]
        .align_y(Alignment::Center),
        buttons,
        widgets::faint(
            "Проверка — раз в сутки и через десять секунд после запуска; политика решает, что \
             приложение делает само, а кнопки работают всегда. Новая версия начинает работать \
             после перезапуска: подмена не трогает уже запущенный процесс."
        ),
    ]
    .spacing(12);

    if let Some(error) = &update.error {
        content = content.push(widgets::note(format!(
            "Последняя попытка не удалась: {error}"
        )));
    }
    if !update.replaceable {
        content = content.push(widgets::note(
            "Эта сборка не AppImage (или запущена не из образа): подменить себя она не может. \
             Обновление придётся скачать со страницы релизов вручную.",
        ));
    }
    content = content.push(widgets::faint(
        "SHA256SUMS закрывает обрыв, порчу и зеркало, отдающее вчерашний образ, но не подмену на \
         стороне GitHub: подлинность проверяется отдельно, `gh attestation verify` — SECURITY.md.",
    ));

    widgets::card(content)
}

/// The state line. Every branch is something that is actually true right now, and the two that a
/// status line usually gets wrong are spelled out: "downloaded" is not "installed", and "installed"
/// is not "running".
fn update_headline(update: &UpdateView) -> (Tone, String) {
    match &update.phase {
        UpdatePhase::Checking => (Tone::Neutral, "спрашиваем страницу релизов…".to_string()),
        UpdatePhase::Downloading { received, total } => (
            Tone::Neutral,
            match total {
                Some(total) => format!(
                    "качаем {} из {}",
                    human_bytes(*received),
                    human_bytes(*total)
                ),
                None => format!("качаем {}", human_bytes(*received)),
            },
        ),
        UpdatePhase::Staged { version } => (
            Tone::Warning,
            format!("{version} скачана и проверена — ждёт перезапуска"),
        ),
        UpdatePhase::Installed { version } => (
            Tone::Success,
            format!("{version} на месте — заработает после перезапуска"),
        ),
        UpdatePhase::Idle => match (update.current, update.latest) {
            (_, None) => (Tone::Neutral, "ещё не проверяли".to_string()),
            (Some(current), Some(latest)) if current < latest => {
                (Tone::Warning, format!("доступна {latest}"))
            }
            (Some(current), Some(latest)) if current > latest => (
                Tone::Success,
                format!("{current} — новее последнего релиза ({latest})"),
            ),
            (Some(current), _) => (Tone::Success, format!("{current} — последняя версия")),
            (None, Some(latest)) => (
                Tone::Neutral,
                format!("последний релиз: {latest}; эта сборка без версии"),
            ),
        },
    }
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
