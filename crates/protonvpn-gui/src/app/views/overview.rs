//! The overview page: what the tunnel is doing, what the probe says, and the connection manager.

use iced::widget::{Space, button, column, container, row, scrollable, text};
use iced::{Alignment, Element, Font, Length, Padding, Theme};

use protonvpn_core::config::SavedConnection;
use protonvpn_core::launcher::Intent;
use protonvpn_core::model::{ConnectionStatus, EgressReading, Observation, PortForwarding};

use crate::app::{App, Message, SystemPreset, selected_target};
use crate::theme;
use crate::widgets::{self, Tone};

use super::status_tone;

pub(crate) fn view(app: &App) -> Element<'_, Message> {
    let header = row![
        column![widgets::eyebrow("Обзор"), text("Соединение").size(22),]
            .spacing(2)
            .width(Length::Fill),
        button(text("Обновить статус").size(13))
            .padding(Padding::from([8, 14]))
            .style(theme::outlined(theme::BORDER, theme::TEXT))
            .on_press(Message::RefreshStatus),
    ]
    .align_y(Alignment::Center)
    .width(Length::Fill);

    let left = column![status_card(app), egress_card(app)].spacing(14);
    let right = column![connections_card(app), port_card(app)].spacing(14);

    let body = row![
        container(left).width(Length::FillPortion(3)),
        container(right).width(Length::FillPortion(2)),
    ]
    .spacing(14)
    .width(Length::Fill);

    scrollable(column![header, body].spacing(16))
        .height(Length::Fill)
        .into()
}

// --- status ---------------------------------------------------------------------------------

fn status_card(app: &App) -> Element<'_, Message> {
    let connection = &app.shared.state.connection;
    let status = &connection.value;

    let (title, extra) = match status {
        ConnectionStatus::Connected(info) => (format!("{} в {}", info.server, info.location), None),
        ConnectionStatus::Connecting => ("Подключаюсь…".to_string(), None),
        ConnectionStatus::Disconnected => ("Нет активного туннеля".to_string(), None),
        ConnectionStatus::Error(message) => ("CLI отказал".to_string(), Some(message.clone())),
        ConnectionStatus::Unknown => (
            "Состояние неизвестно".to_string(),
            Some("CLI ещё не отвечал — состояние не выдумывается.".to_string()),
        ),
    };

    let connected = status.is_connected();
    let action = button(
        text(if connected {
            "Отключиться"
        } else {
            "Подключиться"
        })
        .size(14),
    )
    .padding(Padding::from([9, 18]))
    .style(theme::filled(if connected {
        theme::DANGER
    } else {
        theme::ACCENT
    }))
    .on_press(Message::PrimaryAction);

    let head = row![
        widgets::status_chip(status_tone(status), status.label()),
        Space::new(Length::Fill, Length::Fixed(1.0)),
        action,
    ]
    .align_y(Alignment::Center);

    let facts = match status {
        ConnectionStatus::Connected(info) => row![
            widgets::tile("Сервер", info.server.clone()),
            widgets::tile("Город", location_city(&info.location)),
            widgets::tile(
                "Нагрузка",
                info.load_percent
                    .map(|load| format!("{load}%"))
                    .unwrap_or_else(|| "—".into()),
            ),
            widgets::tile(
                "Протокол",
                info.protocol.clone().unwrap_or_else(|| "—".into()),
            ),
        ]
        .spacing(10),
        _ => row![
            widgets::tile("Сервер", "—"),
            widgets::tile("Город", "—"),
            widgets::tile("Нагрузка", "—"),
            widgets::tile("Протокол", "—"),
        ]
        .spacing(10),
    };

    let mut body = column![
        head,
        text(title).size(19),
        widgets::muted(format!(
            "{} · соединение: {}",
            connection.age_text(),
            app.selected_name()
        )),
        text(format!("$ {}", selected_argv(app)))
            .size(12)
            .font(Font::MONOSPACE)
            .style(|_: &Theme| iced::widget::text::Style {
                color: Some(theme::TEXT_MUTED),
            }),
    ]
    .spacing(12);

    if let Some(extra) = extra {
        body = body.push(widgets::note(extra));
    }
    body = body.push(facts);

    widgets::card(body)
}

/// `NL#818 in Amsterdam, Netherlands` → `Amsterdam`. The CLI's own words, just the city half.
fn location_city(location: &str) -> String {
    match location.split_once(',') {
        Some((city, _)) => city.trim().to_string(),
        None if location.is_empty() => "—".to_string(),
        None => location.to_string(),
    }
}

// --- egress probe ---------------------------------------------------------------------------

fn egress_card(app: &App) -> Element<'_, Message> {
    type Reading = Observation<EgressReading>;
    let egress = &app.shared.state.egress;
    let current: Option<&Reading> = egress.current.as_ref();
    let baseline: Option<&Reading> = egress.baseline.as_ref();

    let verdict: Element<'_, Message> = match egress.egress_changed() {
        Some(true) => text("адрес изменился")
            .size(12)
            .style(|_: &Theme| iced::widget::text::Style {
                color: Some(theme::SUCCESS),
            })
            .into(),
        Some(false) => text("адрес не изменился")
            .size(12)
            .style(|_: &Theme| iced::widget::text::Style {
                color: Some(theme::WARNING),
            })
            .into(),
        None => widgets::faint("—"),
    };

    let head = row![
        widgets::eyebrow("Проба egress · ground truth"),
        Space::new(Length::Fill, Length::Fixed(1.0)),
        verdict,
        button(text("Измерить").size(12))
            .padding(Padding::from([4, 10]))
            .style(theme::outlined(theme::BORDER, theme::TEXT))
            .on_press(Message::Probe),
    ]
    .spacing(8)
    .align_y(Alignment::Center);

    let address = |reading: Option<&Reading>| match reading {
        Some(observation) => observation.value.ip.to_string(),
        None => "—".to_string(),
    };
    let country = current
        .and_then(|observation| observation.value.country.clone())
        .unwrap_or_else(|| "—".into());
    let asn = current
        .and_then(|observation| observation.value.asn_org.clone())
        .unwrap_or_else(|| "—".into());
    let endpoint = current
        .map(|observation| observation.value.endpoint.name().to_string())
        .unwrap_or_else(|| "—".into());

    let rows = column![
        widgets::fact_row("IPv4 (проба)", address(current)),
        widgets::separator(),
        widgets::fact_row("До подключения", address(baseline)),
        widgets::separator(),
        widgets::fact_row("Страна (справочно)", country),
        widgets::separator(),
        widgets::fact_row("ASN / организация", asn),
        widgets::separator(),
        widgets::fact_row("Источник", endpoint),
    ]
    .spacing(8);

    let explanation = match egress.egress_changed() {
        Some(true) => "Адрес изменился — трафик идёт через туннель.".to_string(),
        Some(false) => "Адрес не изменился. Если CLI говорит «подключено», туннель не несёт \
                        трафик."
            .to_string(),
        None if current.is_some() => {
            "Сравнить не с чем: адрес получен до того, как мы начали мерить.".to_string()
        }
        None => "Проба выполняется после подключения — туннель не активен.".to_string(),
    };

    widgets::card(
        column![
            head,
            widgets::separator(),
            rows,
            widgets::note(explanation),
            widgets::faint(
                "Проба к ifconfig.co/json — единственный внешний вызов помимо protonvpn. CLI \
                 может ошибаться в адресе выхода, поэтому источник истины — измерение, а не \
                 самоотчёт. Страна и ASN — только для чтения: базы GeoIP расходятся между собой."
            ),
        ]
        .spacing(12),
    )
}

// --- connections ----------------------------------------------------------------------------

fn connections_card(app: &App) -> Element<'_, Message> {
    let head = row![
        widgets::eyebrow("Соединения"),
        Space::new(Length::Fill, Length::Fixed(1.0)),
        button(text("Добавить соединение").size(13))
            .padding(Padding::from([7, 12]))
            .style(theme::outlined(theme::BORDER, theme::TEXT))
            .on_press(Message::ConnectionNew),
    ]
    .align_y(Alignment::Center);

    let mut list = column![
        widgets::faint("Системные · не редактируются"),
        preset_row(app, SystemPreset::Fastest),
        preset_row(app, SystemPreset::SecureCore),
        preset_row(app, SystemPreset::P2p),
        widgets::faint("Мои соединения"),
    ]
    .spacing(8);

    if app.config.connections.is_empty() {
        list = list.push(widgets::note(
            "Пока нет своих соединений. «Добавить соединение» соберёт профиль: страна, город, \
             P2P, Secure Core, Tor и проброс порта.",
        ));
    }
    for saved in &app.config.connections {
        list = list.push(saved_row(app, saved));
    }

    widgets::card(
        column![
            head,
            list,
            widgets::faint(
                "Подключение всегда выполняется для выбранного соединения: protonvpn connect."
            ),
        ]
        .spacing(12),
    )
}

fn selected(app: &App, id: &str) -> bool {
    app.config.selected_connection.as_deref() == Some(id)
}

fn preset_row(app: &App, preset: SystemPreset) -> Element<'_, Message> {
    let is_selected = selected(app, preset.id());
    let badges: Element<'_, Message> = row(preset
        .badges()
        .iter()
        .map(|badge| widgets::badge(*badge, Tone::Neutral))
        .collect::<Vec<_>>())
    .spacing(4)
    .into();

    let body = row![
        widgets::monogram(preset.monogram(), Tone::Accent),
        column![
            text(preset.name()).size(14),
            widgets::muted(preset.summary()),
        ]
        .spacing(1)
        .width(Length::Fill),
        badges,
    ]
    .spacing(10)
    .align_y(Alignment::Center);

    row_card(
        is_selected,
        button(body)
            .width(Length::Fill)
            .padding(Padding::from([0, 0]))
            .style(theme::bare())
            .on_press(Message::ConnectionSelected(preset.id().to_string())),
        None,
    )
}

fn saved_row<'a>(app: &'a App, saved: &'a SavedConnection) -> Element<'a, Message> {
    let is_selected = selected(app, &saved.id);
    let id = saved.id.clone();

    let monogram = match &saved.country {
        Some(country) => widgets::monogram(country.clone(), Tone::Neutral),
        None => widgets::monogram("··", Tone::Neutral),
    };

    let badges: Element<'_, Message> = row(saved
        .badges()
        .into_iter()
        .map(|badge| widgets::badge(badge, Tone::Neutral))
        .collect::<Vec<_>>())
    .spacing(4)
    .into();

    // The right column is narrow, and a profile row carries more than a preset row: badges, a
    // rename and a delete. Stacking them under the name keeps the name from being squeezed into
    // one character per line.
    let body = column![
        row![
            monogram,
            column![
                text(saved.name.clone()).size(14),
                widgets::muted(app.summary_for(saved)),
            ]
            .spacing(1)
            .width(Length::Fill),
        ]
        .spacing(10)
        .align_y(Alignment::Center),
        row![
            badges,
            Space::new(Length::Fill, Length::Fixed(1.0)),
            button(text("Изменить").size(12))
                .padding(Padding::from([4, 8]))
                .style(theme::ghost(theme::TEXT_MUTED, false))
                .on_press(Message::ConnectionEdit(id.clone())),
            button(text("Удалить").size(12))
                .padding(Padding::from([4, 6]))
                .style(theme::ghost(theme::TEXT_MUTED, false))
                .on_press(Message::ConnectionDelete(id.clone())),
        ]
        .spacing(2)
        .align_y(Alignment::Center),
    ]
    .spacing(8);

    row_card(
        is_selected,
        button(body)
            .width(Length::Fill)
            .padding(Padding::from([0, 0]))
            .style(theme::bare())
            .on_press(Message::ConnectionSelected(id)),
        None,
    )
}

/// One row of the connection list: a selectable body, and whatever actions it allows. The actions
/// are siblings of the body button, never nested inside it — a button in a button is not a thing
/// iced can hit-test honestly.
fn row_card<'a>(
    is_selected: bool,
    body: iced::widget::Button<'a, Message>,
    actions: Option<Element<'a, Message>>,
) -> Element<'a, Message> {
    let mut content = row![body].spacing(6).align_y(Alignment::Center);
    if let Some(actions) = actions {
        content = content.push(actions);
    }

    container(content)
        .padding(Padding::from([8, 10]))
        .width(Length::Fill)
        .style(theme::connection_card(is_selected))
        .into()
}

// --- port forwarding ------------------------------------------------------------------------

fn port_card(app: &App) -> Element<'_, Message> {
    let wants = app.selected_port_forwarding();
    let state = &app.shared.state.port_forwarding;

    let status_text = if !wants {
        "выключен для этого соединения"
    } else {
        match &state.value {
            PortForwarding::Active { .. } => "аренда активна",
            PortForwarding::Pending => "запрашиваю аренду",
            PortForwarding::Unsupported => "сервер не поддерживает",
            PortForwarding::Unavailable(_) => "недоступен",
            PortForwarding::Idle => "аренды нет",
        }
    };

    let head = row![
        widgets::eyebrow("Порт-форвардинг"),
        Space::new(Length::Fill, Length::Fixed(1.0)),
        widgets::faint(status_text),
    ]
    .align_y(Alignment::Center);

    let port_block: Element<'_, Message> = match (wants, &state.value) {
        (true, PortForwarding::Active { port, lifetime, .. }) => column![
            row![
                text(port.to_string())
                    .size(34)
                    .font(Font::MONOSPACE)
                    .width(Length::Fill),
                button(text("Копировать").size(12))
                    .padding(Padding::from([6, 12]))
                    .style(theme::filled(theme::ACCENT))
                    .on_press(Message::CopyPort),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
            widgets::muted(format!(
                "аренда {} с, продлевается автоматически",
                lifetime.as_secs()
            )),
            if app
                .copied_port_at
                .is_some_and(|at| at.elapsed() < std::time::Duration::from_secs(2))
            {
                widgets::muted("Порт скопирован в буфер обмена")
            } else {
                widgets::faint("Порт выдаётся шлюзом и меняется после переподключения.")
            },
        ]
        .spacing(6)
        .into(),
        (true, PortForwarding::Unavailable(reason)) => column![
            text("—").size(34).font(Font::MONOSPACE),
            widgets::note(reason.clone()),
            widgets::faint(
                "Порт не показывается намеренно: показывать номер, который никто не продлевает, \
                 значит вводить в заблуждение."
            ),
        ]
        .spacing(6)
        .into(),
        (true, PortForwarding::Pending) => column![
            text("—").size(34).font(Font::MONOSPACE),
            widgets::muted("Запрашиваю аренду у шлюза через NAT-PMP…"),
        ]
        .spacing(6)
        .into(),
        (true, PortForwarding::Unsupported) => column![
            text("—").size(34).font(Font::MONOSPACE),
            widgets::muted(
                "Этот сервер не поддерживает проброс порта. Подключитесь к P2P-серверу."
            ),
        ]
        .spacing(6)
        .into(),
        _ => column![
            text("—").size(34).font(Font::MONOSPACE),
            widgets::muted("Аренды нет."),
        ]
        .spacing(6)
        .into(),
    };

    let toggle: Element<'_, Message> = match app.selected_saved() {
        Some(saved) => row![
            iced::widget::toggler(saved.port_forwarding)
                .label("Держать аренду для этого профиля")
                .text_size(13)
                .on_toggle(Message::SelectedPortForwarding),
        ]
        .into(),
        None => widgets::faint(
            "Системные пресеты не редактируются: профиль создаётся кнопкой «Добавить \
             соединение».",
        ),
    };

    let controls = row![
        button(text("Запросить заново").size(12))
            .padding(Padding::from([5, 10]))
            .style(theme::outlined(theme::BORDER, theme::TEXT))
            .on_press(Message::PortRefresh),
        button(text("Освободить").size(12))
            .padding(Padding::from([5, 10]))
            .style(theme::outlined(theme::BORDER, theme::TEXT_MUTED))
            .on_press(Message::PortRelease),
    ]
    .spacing(6);

    widgets::card(
        column![
            head,
            port_block,
            toggle,
            controls,
            widgets::faint(
                "Порт выдаётся лизом NAT-PMP у 10.2.0.1:5351 (RFC 6886) только если это включено \
                 в выбранном соединении — порт-форвардинг является частью профиля. Перед запросом \
                 отправляется opcode 0: если шлюз не отвечает, порт не показывается."
            ),
        ]
        .spacing(10),
    )
}

/// The argv the selected connection stands for, straight from the launcher — so the line the user
/// reads under the status can never drift from the command that will actually run.
pub(crate) fn selected_argv(app: &App) -> String {
    Intent::Connect(selected_target(&app.config))
        .argv()
        .join(" ")
}
