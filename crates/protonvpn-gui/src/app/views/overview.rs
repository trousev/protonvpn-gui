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
        column![
            widgets::eyebrow(app.i18n.overview_page_eyebrow()),
            text(app.i18n.overview_page_title()).size(22),
        ]
        .spacing(2)
        .width(Length::Fill),
        button(text(app.i18n.overview_refresh_status()).size(13))
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
        ConnectionStatus::Connected(info) => (
            app.i18n
                .overview_status_connected(info.server.as_str(), info.location.as_str()),
            None,
        ),
        ConnectionStatus::Connecting => (app.i18n.overview_status_connecting(), None),
        ConnectionStatus::Disconnected => (app.i18n.overview_status_disconnected(), None),
        ConnectionStatus::Error(message) => {
            (app.i18n.overview_status_error(), Some(message.clone()))
        }
        ConnectionStatus::Unknown => (
            app.i18n.overview_status_unknown(),
            Some(app.i18n.overview_status_unknown_note()),
        ),
    };

    let connected = status.is_connected();
    let action = button(
        text(if connected {
            app.i18n.overview_disconnect()
        } else {
            app.i18n.overview_connect()
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
        widgets::status_chip(status_tone(status), app.i18n.connection_label(status)),
        Space::new().width(Length::Fill).height(Length::Fixed(1.0)),
        action,
    ]
    .align_y(Alignment::Center);

    let facts = match status {
        ConnectionStatus::Connected(info) => row![
            widgets::tile(app.i18n.overview_tile_server(), info.server.clone()),
            widgets::tile(app.i18n.overview_tile_city(), location_city(&info.location)),
            widgets::tile(
                app.i18n.overview_tile_load(),
                info.load_percent
                    .map(|load| format!("{load}%"))
                    .unwrap_or_else(|| "—".into()),
            ),
            widgets::tile(
                app.i18n.overview_tile_protocol(),
                info.protocol.clone().unwrap_or_else(|| "—".into()),
            ),
        ]
        .spacing(10),
        _ => row![
            widgets::tile(app.i18n.overview_tile_server(), "—"),
            widgets::tile(app.i18n.overview_tile_city(), "—"),
            widgets::tile(app.i18n.overview_tile_load(), "—"),
            widgets::tile(app.i18n.overview_tile_protocol(), "—"),
        ]
        .spacing(10),
    };

    let mut body = column![
        head,
        text(title).size(19),
        widgets::muted(
            app.i18n
                .overview_age_and_target(app.i18n.age_text(connection.age()), app.selected_name(),)
        ),
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
        Some(true) => text(app.i18n.overview_egress_changed())
            .size(12)
            .style(|_: &Theme| iced::widget::text::Style {
                color: Some(theme::SUCCESS),
            })
            .into(),
        Some(false) => text(app.i18n.overview_egress_unchanged())
            .size(12)
            .style(|_: &Theme| iced::widget::text::Style {
                color: Some(theme::WARNING),
            })
            .into(),
        None => widgets::faint("—"),
    };

    let head = row![
        widgets::eyebrow(app.i18n.overview_egress_eyebrow()),
        Space::new().width(Length::Fill).height(Length::Fixed(1.0)),
        verdict,
        button(text(app.i18n.overview_egress_measure()).size(12))
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
        widgets::fact_row(app.i18n.overview_egress_ipv4(), address(current)),
        widgets::separator(),
        widgets::fact_row(app.i18n.overview_egress_baseline(), address(baseline)),
        widgets::separator(),
        widgets::fact_row(app.i18n.overview_egress_country(), country),
        widgets::separator(),
        widgets::fact_row(app.i18n.overview_egress_asn(), asn),
        widgets::separator(),
        widgets::fact_row(app.i18n.overview_egress_source(), endpoint),
    ]
    .spacing(8);

    let explanation = match egress.egress_changed() {
        Some(true) => app.i18n.overview_egress_changed_note(),
        Some(false) => app.i18n.overview_egress_unchanged_note(),
        None if current.is_some() => app.i18n.overview_egress_no_baseline(),
        None => app.i18n.overview_egress_no_tunnel(),
    };

    widgets::card(
        column![
            head,
            widgets::separator(),
            rows,
            widgets::note(explanation),
            widgets::faint(app.i18n.overview_egress_provenance()),
        ]
        .spacing(12),
    )
}

// --- connections ----------------------------------------------------------------------------

fn connections_card(app: &App) -> Element<'_, Message> {
    let head = row![
        widgets::eyebrow(app.i18n.overview_connections_eyebrow()),
        Space::new().width(Length::Fill).height(Length::Fixed(1.0)),
        button(text(app.i18n.overview_connections_add()).size(13))
            .padding(Padding::from([7, 12]))
            .style(theme::outlined(theme::BORDER, theme::TEXT))
            .on_press(Message::ConnectionNew),
    ]
    .align_y(Alignment::Center);

    let mut list = column![
        widgets::faint(app.i18n.overview_connections_system()),
        preset_row(app, SystemPreset::Fastest),
        preset_row(app, SystemPreset::SecureCore),
        preset_row(app, SystemPreset::P2p),
        widgets::faint(app.i18n.overview_connections_mine()),
    ]
    .spacing(8);

    if app.config.connections.is_empty() {
        list = list.push(widgets::note(app.i18n.overview_connections_empty()));
    }
    for saved in &app.config.connections {
        list = list.push(saved_row(app, saved));
    }

    widgets::card(
        column![
            head,
            list,
            widgets::faint(app.i18n.overview_connections_footer()),
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
        .badges(&app.i18n)
        .into_iter()
        .map(|badge| widgets::badge(badge, Tone::Neutral))
        .collect::<Vec<_>>())
    .spacing(4)
    .into();

    let body = row![
        widgets::monogram(preset.monogram(), Tone::Accent),
        column![
            text(preset.name(&app.i18n)).size(14),
            widgets::muted(preset.summary(&app.i18n)),
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
        .badges(&app.i18n)
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
            Space::new().width(Length::Fill).height(Length::Fixed(1.0)),
            button(text(app.i18n.overview_connection_edit()).size(12))
                .padding(Padding::from([4, 8]))
                .style(theme::ghost(theme::TEXT_MUTED, false))
                .on_press(Message::ConnectionEdit(id.clone())),
            button(text(app.i18n.overview_connection_delete()).size(12))
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
        app.i18n.overview_port_off()
    } else {
        match &state.value {
            PortForwarding::Active { .. } => app.i18n.overview_port_active(),
            PortForwarding::Pending => app.i18n.overview_port_pending(),
            PortForwarding::Unsupported => app.i18n.overview_port_unsupported(),
            PortForwarding::Unavailable(_) => app.i18n.overview_port_unavailable(),
            PortForwarding::Idle => app.i18n.overview_port_idle(),
        }
    };

    let head = row![
        widgets::eyebrow(app.i18n.overview_port_eyebrow()),
        Space::new().width(Length::Fill).height(Length::Fixed(1.0)),
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
                button(text(app.i18n.overview_port_copy()).size(12))
                    .padding(Padding::from([6, 12]))
                    .style(theme::filled(theme::ACCENT))
                    .on_press(Message::CopyPort),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
            widgets::muted(app.i18n.overview_port_lease(lifetime.as_secs() as i64)),
            if app
                .copied_port_at
                .is_some_and(|at| at.elapsed() < std::time::Duration::from_secs(2))
            {
                widgets::muted(app.i18n.overview_port_copied())
            } else {
                widgets::faint(app.i18n.overview_port_volatile())
            },
        ]
        .spacing(6)
        .into(),
        (true, PortForwarding::Unavailable(reason)) => column![
            text("—").size(34).font(Font::MONOSPACE),
            widgets::note(reason.clone()),
            widgets::faint(app.i18n.overview_port_hidden()),
        ]
        .spacing(6)
        .into(),
        (true, PortForwarding::Pending) => column![
            text("—").size(34).font(Font::MONOSPACE),
            widgets::muted(app.i18n.overview_port_requesting()),
        ]
        .spacing(6)
        .into(),
        (true, PortForwarding::Unsupported) => column![
            text("—").size(34).font(Font::MONOSPACE),
            widgets::muted(app.i18n.overview_port_unsupported_note()),
        ]
        .spacing(6)
        .into(),
        _ => column![
            text("—").size(34).font(Font::MONOSPACE),
            widgets::muted(app.i18n.overview_port_none()),
        ]
        .spacing(6)
        .into(),
    };

    let toggle: Element<'_, Message> = match app.selected_saved() {
        Some(saved) => row![
            iced::widget::toggler(saved.port_forwarding)
                .label(app.i18n.overview_port_keep())
                .text_size(13)
                .on_toggle(Message::SelectedPortForwarding),
        ]
        .into(),
        None => widgets::faint(app.i18n.overview_port_system_preset()),
    };

    let controls = row![
        button(text(app.i18n.overview_port_refresh()).size(12))
            .padding(Padding::from([5, 10]))
            .style(theme::outlined(theme::BORDER, theme::TEXT))
            .on_press(Message::PortRefresh),
        button(text(app.i18n.overview_port_release()).size(12))
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
            widgets::faint(app.i18n.overview_port_provenance()),
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

#[cfg(test)]
mod tests {
    use protonvpn_core::i18n::{I18n, Locale};

    fn english() -> I18n {
        I18n::new(Locale::SOURCE)
    }

    /// Tests run in the source language, so these are the words the page actually draws. A change
    /// to the catalogue then has to be a decision taken twice, here as well as there.
    #[test]
    fn the_status_card_wording_is_the_catalogue() {
        let i18n = english();
        assert_eq!(i18n.overview_page_eyebrow(), "Overview");
        assert_eq!(i18n.overview_page_title(), "Connection");
        assert_eq!(i18n.overview_refresh_status(), "Refresh status");
        assert_eq!(i18n.overview_status_connecting(), "Connecting…");
        assert_eq!(i18n.overview_status_disconnected(), "No active tunnel");
        assert_eq!(i18n.overview_status_error(), "The CLI refused");
        assert_eq!(i18n.overview_status_unknown(), "The state is unknown");
        // The honesty of §5 is part of the wording, not of the layout: an unknown state says so.
        assert_eq!(
            i18n.overview_status_unknown_note(),
            "The CLI has not answered yet — the state is not invented."
        );
        assert_eq!(i18n.overview_disconnect(), "Disconnect");
        assert_eq!(i18n.overview_connect(), "Connect");
        assert_eq!(i18n.overview_tile_server(), "Server");
        assert_eq!(i18n.overview_tile_city(), "City");
        assert_eq!(i18n.overview_tile_load(), "Load");
        assert_eq!(i18n.overview_tile_protocol(), "Protocol");
    }

    /// The probe card is the one place the application contradicts the CLI out loud. Every one of
    /// these lines is a design decision (`docs/architecture.md` §13, `docs/cli-surface.md` §4.9),
    /// and softening one of them is a change of meaning — so they are pinned too.
    #[test]
    fn the_egress_card_wording_is_the_catalogue() {
        let i18n = english();
        assert_eq!(i18n.overview_egress_changed(), "address changed");
        assert_eq!(i18n.overview_egress_unchanged(), "address did not change");
        assert_eq!(
            i18n.overview_egress_eyebrow(),
            "Egress probe · ground truth"
        );
        assert_eq!(i18n.overview_egress_measure(), "Measure");
        assert_eq!(i18n.overview_egress_ipv4(), "IPv4 (probe)");
        assert_eq!(i18n.overview_egress_baseline(), "Before connecting");
        assert_eq!(i18n.overview_egress_country(), "Country (advisory)");
        assert_eq!(i18n.overview_egress_asn(), "ASN / organization");
        assert_eq!(i18n.overview_egress_source(), "Source");
        assert_eq!(
            i18n.overview_egress_changed_note(),
            "The address changed — traffic is going through the tunnel."
        );
        assert_eq!(
            i18n.overview_egress_unchanged_note(),
            "The address did not change. If the CLI says \"connected\", the tunnel is not \
             carrying traffic."
        );
        assert_eq!(
            i18n.overview_egress_no_baseline(),
            "There is nothing to compare against: this address was read before we started \
             measuring."
        );
        assert_eq!(
            i18n.overview_egress_no_tunnel(),
            "The probe runs after connecting — the tunnel is not active."
        );
        assert_eq!(
            i18n.overview_egress_provenance(),
            "The probe to ifconfig.co/json is the only external call besides protonvpn. The CLI \
             can be wrong about the egress address, so the ground truth is a measurement, not a \
             self-report. Country and ASN are advisory only: GeoIP databases disagree with each \
             other."
        );
    }

    #[test]
    fn the_connections_and_port_wording_is_the_catalogue() {
        let i18n = english();
        assert_eq!(i18n.overview_connections_eyebrow(), "Connections");
        assert_eq!(i18n.overview_connections_add(), "Add connection");
        assert_eq!(i18n.overview_connections_system(), "System · not editable");
        assert_eq!(i18n.overview_connections_mine(), "My connections");
        assert_eq!(
            i18n.overview_connections_empty(),
            "No connections of your own yet. \"Add connection\" builds a profile: country, city, \
             P2P, Secure Core, Tor and port forwarding."
        );
        assert_eq!(
            i18n.overview_connections_footer(),
            "Connecting always runs for the selected connection: protonvpn connect."
        );
        assert_eq!(i18n.overview_connection_edit(), "Edit");
        assert_eq!(i18n.overview_connection_delete(), "Delete");

        assert_eq!(i18n.overview_port_eyebrow(), "Port forwarding");
        assert_eq!(i18n.overview_port_off(), "off for this connection");
        assert_eq!(i18n.overview_port_active(), "lease active");
        assert_eq!(i18n.overview_port_pending(), "asking for a lease");
        assert_eq!(
            i18n.overview_port_unsupported(),
            "the server does not support it"
        );
        assert_eq!(i18n.overview_port_unavailable(), "unavailable");
        assert_eq!(i18n.overview_port_idle(), "no lease");
        assert_eq!(i18n.overview_port_copy(), "Copy");
        assert_eq!(i18n.overview_port_copied(), "Port copied to the clipboard");
        assert_eq!(
            i18n.overview_port_volatile(),
            "The gateway hands out the port, and it changes after a reconnect."
        );
        assert_eq!(
            i18n.overview_port_hidden(),
            "The port is deliberately not shown: showing a number nobody is renewing would be \
             misleading."
        );
        assert_eq!(
            i18n.overview_port_unsupported_note(),
            "This server does not support port forwarding. Connect to a P2P server."
        );
        assert_eq!(i18n.overview_port_none(), "There is no lease.");
        assert_eq!(
            i18n.overview_port_requesting(),
            "Asking the gateway for a lease over NAT-PMP…"
        );
        assert_eq!(i18n.overview_port_keep(), "Keep a lease for this profile");
        assert_eq!(
            i18n.overview_port_system_preset(),
            "System presets are not editable: a profile is created with the \"Add connection\" \
             button."
        );
        assert_eq!(i18n.overview_port_refresh(), "Request again");
        assert_eq!(i18n.overview_port_release(), "Release");
        assert_eq!(
            i18n.overview_port_provenance(),
            "The port comes from a NAT-PMP lease at 10.2.0.1:5351 (RFC 6886), and only when the \
             selected connection asks for it — port forwarding is part of a profile. An opcode 0 \
             is sent first: if the gateway does not answer, no port is shown."
        );
    }

    /// The messages that carry data. Our words around it are translated and the data is pasted in
    /// untouched — a server name, a location, a connection's name and a lease's seconds.
    #[test]
    fn data_is_passed_through_and_never_paraphrased() {
        let i18n = english();
        assert_eq!(
            i18n.overview_status_connected("NL#818", "Amsterdam, Netherlands"),
            "NL#818 in Amsterdam, Netherlands"
        );
        assert_eq!(
            i18n.overview_age_and_target("updated 3 mins ago", "Secure Core"),
            "updated 3 mins ago · connection: Secure Core"
        );
        assert_eq!(
            i18n.overview_port_lease(3600),
            "lease 3600 s, renewed automatically"
        );
    }
}
