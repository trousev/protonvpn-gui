//! The settings page — `protonvpn config list` in a form a person can use, plus the handful of
//! settings that are ours.
//!
//! Two rules shape it. First, **nothing is invented**: a row exists because the CLI printed that
//! key in `config list`, and a key we have never seen still appears, under its own name, on the
//! General tab. Second, every write is followed by a fresh `config list`, because our own command
//! succeeding is not evidence that the CLI agreed (`docs/architecture.md` §5).
//!
//! Every sentence on it comes from the catalogue; what does not is data — the keys `config list`
//! printed, the values it accepts, an address, a command line. The words around the data are ours
//! and are translated; the data is shown exactly as it arrived.

use iced::widget::{
    Space, button, checkbox, column, container, pick_list, radio, row, scrollable, text, text_input,
};
use iced::{Alignment, Element, Length, Padding, Theme};

use protonvpn_core::config::UpdatePolicy;
use protonvpn_core::engine::{UpdatePhase, UpdateView};
use protonvpn_core::i18n::I18n;
use protonvpn_core::model::Setting;
use protonvpn_core::socks5::{Closed, GateState};
use protonvpn_core::update::human_bytes;

use crate::app::{App, AppToggle, LanguageChoice, Message, SettingsTab, setting_values};
use crate::theme;
use crate::widgets::{self, Tone};

/// Which of the CLI's settings belong on which tab. A key that appears here is rendered there and
/// nowhere else; a key the CLI grows tomorrow falls through to the General tab.
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

/// Every key we put somewhere. Anything outside this list is shown on the General tab rather than
/// hidden.
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
            widgets::eyebrow(app.i18n.settings_eyebrow()),
            text(app.i18n.settings_heading()).size(22),
        ]
        .spacing(2)
        .width(Length::Fill),
        button(text(app.i18n.settings_refresh()).size(13))
            .padding(Padding::from([8, 14]))
            .style(theme::outlined(theme::BORDER, theme::TEXT))
            .on_press(Message::RefreshSettings),
    ]
    .align_y(Alignment::Center)
    .width(Length::Fill);

    let tabs = row(SettingsTab::ALL
        .iter()
        .map(|tab| {
            let active = *tab == app.settings_tab;
            button(text(tab.label(&app.i18n)).size(13).style(move |_: &Theme| {
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
    // First on the General tab, because it is the one card that may need an answer today — and
    // because the tray's update item lands here.
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
                widgets::muted(app.i18n.settings_unread()),
                button(text(app.i18n.settings_refresh()).size(13))
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
        widgets::faint(app.i18n.age_text(observation.age())),
    ]
    .align_y(Alignment::Center);

    widgets::card(column![head, column(rows).spacing(10)].spacing(12))
}

fn setting_row<'a>(app: &'a App, setting: &'a Setting) -> Element<'a, Message> {
    let (name, description) = match setting_label(&setting.key, &app.i18n) {
        Some((name, description)) => (name, description),
        // A key we have never seen keeps its own name and says why there is no description: the
        // CLI can grow a setting, and hiding it would be worse than showing it undescribed.
        None => (setting.key.clone(), app.i18n.settings_unknown_key()),
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
                    label: value_label(&setting.key, value, &app.i18n),
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
                text_input(
                    &app.i18n.settings_value_placeholder(),
                    app.setting_draft(&setting.key)
                )
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
                button(text(app.i18n.settings_apply()).size(12))
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
                widgets::faint(app.i18n.settings_dns_servers()),
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

/// A `<select>` option: the words from the catalogue, carrying the CLI's own value.
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

/// What one value of a CLI setting is called, in words a person reads. The value itself — `off`,
/// `malware-only`, `standard` — is data, and for a key we do not describe it is shown as it is
/// rather than dressed up in words we would have had to invent.
fn value_label(key: &str, value: &str, i18n: &I18n) -> String {
    match (key, value) {
        ("netshield", "off") => i18n.settings_value_netshield_off(),
        ("netshield", "malware-only") => i18n.settings_value_netshield_malware_only(),
        ("netshield", "malware-ads-trackers") => {
            i18n.settings_value_netshield_malware_ads_trackers()
        }
        ("kill-switch", "off") => i18n.settings_value_kill_switch_off(),
        ("kill-switch", "standard") => i18n.settings_value_kill_switch_standard(),
        (_, "on") => i18n.settings_value_on(),
        (_, "off") => i18n.settings_value_off(),
        (_, other) => other.to_string(),
    }
}

// --- our own settings -----------------------------------------------------------------------

/// The language picker: one radio per language this build carries, plus "System".
///
/// It is the first thing on the General tab on purpose. A user who has landed in a language they
/// cannot read has to be able to find their way out without reading anything — which is also why
/// every option is written in the language it names.
fn language_options(app: &App) -> Element<'_, Message> {
    let mut options = row![].spacing(18).align_y(Alignment::Center);
    for choice in LanguageChoice::all() {
        options = options.push(
            radio(
                choice.label(&app.i18n),
                choice,
                Some(app.language_choice()),
                Message::LanguageSelected,
            )
            .text_size(13)
            .size(15),
        );
    }
    options.into()
}

fn app_card(app: &App, tab: SettingsTab) -> Element<'_, Message> {
    match tab {
        SettingsTab::General => widgets::card(
            column![
                widgets::eyebrow(app.i18n.settings_language_title()),
                widgets::muted(app.i18n.settings_language_hint()),
                language_options(app),
                widgets::separator(),
                widgets::eyebrow(app.i18n.settings_app_section()),
                widget_toggle(
                    app.config.autostart,
                    app.i18n.settings_autostart_label(),
                    app.i18n.settings_autostart_hint(),
                    AppToggle::Autostart,
                ),
                widget_toggle(
                    app.config.desktop_entry,
                    app.i18n.settings_desktop_entry_label(),
                    app.i18n.settings_desktop_entry_hint(),
                    AppToggle::DesktopEntry,
                ),
                widget_toggle(
                    app.config.start_minimized,
                    app.i18n.settings_start_minimized_label(),
                    app.i18n.settings_start_minimized_hint(),
                    AppToggle::StartMinimized,
                ),
                widgets::faint(app.i18n.settings_files_line(
                    if app.desktop.autostart_enabled() {
                        app.i18n.settings_yes()
                    } else {
                        app.i18n.settings_no()
                    },
                    if app.desktop.entry_installed() {
                        app.i18n.settings_yes()
                    } else {
                        app.i18n.settings_no()
                    },
                )),
            ]
            .spacing(12),
        ),
        SettingsTab::Connection => widgets::card(
            column![
                widgets::eyebrow(app.i18n.settings_app_section()),
                widget_toggle(
                    app.config.connect_at_startup,
                    app.i18n.settings_connect_at_startup_label(),
                    app.i18n.settings_connect_at_startup_hint(),
                    AppToggle::ConnectAtStartup,
                ),
                widgets::muted(app.i18n.settings_selected_connection(
                    app.selected_name(),
                    super::overview::selected_argv(app),
                )),
            ]
            .spacing(12),
        ),
        SettingsTab::Proxy => socks5_card(app),
        SettingsTab::Polling => {
            let age = app.i18n.age_text(app.shared.state.connection.age());
            widgets::card(
                column![
                    widgets::eyebrow(app.i18n.settings_polling_eyebrow()),
                    widgets::muted(app.i18n.settings_polling_intro()),
                    bullet(app.i18n.settings_polling_idle()),
                    bullet(app.i18n.settings_polling_after_command()),
                    bullet(app.i18n.settings_polling_when_you_look()),
                    widgets::note(app.i18n.settings_polling_consequence(age)),
                    widget_toggle(
                        app.config.probe_enabled,
                        app.i18n.settings_probe_label(),
                        app.i18n.settings_probe_hint(),
                        AppToggle::Probe,
                    ),
                ]
                .spacing(12),
            )
        }
        SettingsTab::Account => {
            let mut content =
                column![widgets::eyebrow(app.i18n.settings_account_eyebrow())].spacing(12);

            match app.account_name() {
                Some(name) => {
                    content = content.push(
                        row![
                            text(app.i18n.settings_account_signed_in(name))
                                .size(15)
                                .width(Length::Fill),
                            button(text(app.i18n.settings_account_logout()).size(13))
                                .padding(Padding::from([7, 14]))
                                .style(theme::outlined(theme::BORDER, theme::DANGER))
                                .on_press(Message::Logout),
                        ]
                        .align_y(Alignment::Center),
                    );
                    // `protonvpn info` is a command line and `age` is already a sentence: the two
                    // joined by the same separator the console uses is the whole provenance line.
                    if let Some(age) = app.account_age() {
                        content = content.push(widgets::faint(format!("protonvpn info · {age}")));
                    }
                }
                None => {
                    content = content.push(
                        row![
                            widgets::muted(app.i18n.settings_account_unconfirmed()),
                            Space::new().width(Length::Fill).height(Length::Fixed(1.0)),
                            button(text(app.i18n.settings_account_signin()).size(13))
                                .padding(Padding::from([7, 14]))
                                .style(theme::filled(theme::ACCENT))
                                .on_press(Message::SignInRequested),
                        ]
                        .align_y(Alignment::Center),
                    );
                }
            }

            content = content.push(widgets::separator());
            content = content.push(widgets::muted(app.i18n.settings_account_secret_hint()));

            content = content.push(match &app.shared.pending_prompt {
                Some(pending) => column![
                    widgets::muted(
                        app.i18n
                            .settings_prompt_waiting(prompt_label(&pending.kind, &app.i18n))
                    ),
                    row![
                        text_input(&app.i18n.settings_value_placeholder(), &app.manual_input)
                            .secure(true)
                            .on_input(Message::ManualInput)
                            .on_submit(Message::ManualSend)
                            .padding(Padding::from([7, 10]))
                            .width(Length::Fixed(260.0)),
                        button(text(app.i18n.settings_send()).size(13))
                            .padding(Padding::from([7, 14]))
                            .style(theme::filled(theme::ACCENT))
                            .on_press(Message::ManualSend),
                    ]
                    .spacing(8)
                    .align_y(Alignment::Center),
                ]
                .spacing(8)
                .into(),
                None => widgets::faint(app.i18n.settings_account_manual_hint()),
            });

            widgets::card(content)
        }
    }
}

fn bullet<'a>(value: String) -> Element<'a, Message> {
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
            app.i18n.settings_socks5_open(source.to_string()),
            None,
        ),
        GateState::Closed(reason) => {
            let tone = match reason {
                Closed::Disabled | Closed::NotConnected => Tone::Neutral,
                _ => Tone::Warning,
            };
            // Every branch says what is true and what to do about it. None of them is an error:
            // a proxy that cannot show it is protecting you must not claim it is (§13).
            let advice = match &reason {
                Closed::Disabled => None,
                Closed::NotConnected => Some(app.i18n.settings_socks5_advice_not_connected()),
                Closed::RouteChanged { .. } | Closed::RouteLost { .. } => {
                    Some(app.i18n.settings_socks5_advice_route())
                }
                Closed::ProbeUnanswered { .. } => {
                    Some(app.i18n.settings_socks5_advice_probe_silent())
                }
                Closed::NotListening { .. } => {
                    Some(app.i18n.settings_socks5_advice_not_listening())
                }
            };
            (
                tone,
                app.i18n.settings_socks5_closed(reason.describe(&app.i18n)),
                advice,
            )
        }
    };

    // The counters are numbers and byte counts, not sentences: the byte counts arrive already
    // written in this language by `human_bytes`, and only the words around them are translated.
    let mut counters = app.i18n.settings_socks5_counters(
        stats.accepted.to_string(),
        stats.refused.to_string(),
        stats.active.to_string(),
        human_bytes(stats.up, &app.i18n),
        human_bytes(stats.down, &app.i18n),
    );
    if stats.overloaded > 0 {
        counters.push_str(&app.i18n.settings_socks5_overloaded(stats.overloaded as i64));
    }

    let mut content = column![
        widgets::eyebrow(app.i18n.settings_socks5_eyebrow()),
        widgets::muted(app.i18n.settings_socks5_intro()),
        checkbox(app.config.socks5.enabled)
            .label(app.i18n.settings_socks5_enable())
            .text_size(13)
            .on_toggle(Message::Socks5Enabled),
        row![
            widgets::faint(app.i18n.settings_socks5_address_label()),
            text_input("127.0.0.1", &app.socks5_address)
                .on_input(Message::Socks5Address)
                .padding(Padding::from([7, 10]))
                .width(Length::Fixed(170.0)),
            widgets::faint(app.i18n.settings_socks5_port_label()),
            text_input("1080", &app.socks5_port)
                .on_input(Message::Socks5Port)
                .padding(Padding::from([7, 10]))
                .width(Length::Fixed(90.0)),
            widgets::faint(app.i18n.settings_socks5_verify_label()),
            text_input("30", &app.socks5_verify)
                .on_input(Message::Socks5Verify)
                .padding(Padding::from([7, 10]))
                .width(Length::Fixed(70.0)),
            button(text(app.i18n.settings_apply()).size(13))
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
            button(text(app.i18n.settings_socks5_copy()).size(12))
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
            widgets::muted(app.i18n.settings_socks5_copied(app.socks5_address_text()))
        } else {
            widgets::faint(app.i18n.settings_socks5_point_at(app.socks5_address_text()))
        },
        widgets::muted(counters),
    ]
    .spacing(12);

    if let Some(advice) = advice {
        content = content.push(widgets::note(advice));
    }
    if !app.config.probe_enabled {
        content = content.push(widgets::faint(app.i18n.settings_socks5_probe_off()));
    }

    content = content.push(widgets::faint(app.i18n.settings_socks5_loopback()));
    content = content.push(widgets::faint(app.i18n.settings_socks5_watchdog()));

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

fn policy_label(policy: UpdatePolicy, i18n: &I18n) -> String {
    match policy {
        UpdatePolicy::Off => i18n.settings_update_policy_off(),
        UpdatePolicy::Notify => i18n.settings_update_policy_notify(),
        UpdatePolicy::Download => i18n.settings_update_policy_download(),
        UpdatePolicy::Install => i18n.settings_update_policy_install(),
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
    let (tone, headline) = update_headline(update, app);
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

    let mut checked = app
        .i18n
        .settings_updates_version(crate::version::label(&app.i18n));
    checked.push_str(" · ");
    checked.push_str(&match update.checked_at {
        Some(at) => app.i18n.settings_updates_page_age(
            app.i18n.age_text(
                std::time::SystemTime::now()
                    .duration_since(at)
                    .unwrap_or_default(),
            ),
        ),
        None => app.i18n.settings_updates_page_never(),
    });

    let mut buttons = row![
        button(text(app.i18n.settings_updates_check_now()).size(13))
            .padding(Padding::from([8, 14]))
            .style(theme::outlined(theme::BORDER, theme::TEXT))
            .on_press_maybe((!busy).then_some(Message::UpdateCheckNow)),
    ]
    .spacing(8)
    .align_y(Alignment::Center);

    if downloading {
        buttons = buttons.push(
            button(text(app.i18n.settings_updates_cancel()).size(13))
                .padding(Padding::from([8, 14]))
                .style(theme::outlined(theme::BORDER, theme::DANGER))
                .on_press(Message::UpdateCancel),
        );
    } else if behind && update.replaceable && !installed {
        let label = if staged {
            app.i18n.settings_updates_install()
        } else {
            app.i18n.settings_updates_download_install()
        };
        buttons = buttons.push(
            button(text(label).size(13))
                .padding(Padding::from([8, 14]))
                .style(theme::filled(theme::ACCENT))
                .on_press(Message::UpdateInstall),
        );
    }
    if behind && !update.dismissed && !installed {
        buttons = buttons.push(
            button(text(app.i18n.settings_updates_dismiss()).size(12))
                .padding(Padding::from([6, 12]))
                .style(theme::bare())
                .on_press(Message::UpdateDismiss),
        );
    }

    let mut options = row![].spacing(18).align_y(Alignment::Center);
    for policy in UPDATE_POLICIES {
        options = options.push(
            radio(
                policy_label(policy, &app.i18n),
                policy,
                Some(app.config.update.policy),
                Message::UpdatePolicySelected,
            )
            .text_size(13)
            .size(15),
        );
    }

    let mut content = column![
        widgets::eyebrow(app.i18n.settings_updates_eyebrow()),
        widgets::muted(app.i18n.settings_updates_intro()),
        options,
        widgets::faint(checked),
        widgets::separator(),
        row![
            widgets::status_chip(tone, headline),
            Space::new().width(Length::Fill).height(Length::Fixed(1.0)),
        ]
        .align_y(Alignment::Center),
        buttons,
        widgets::faint(app.i18n.settings_updates_footnote()),
    ]
    .spacing(12);

    if let Some(error) = &update.error {
        content = content.push(widgets::note(app.i18n.settings_updates_error(error)));
    }
    if !update.replaceable {
        content = content.push(widgets::note(app.i18n.settings_updates_not_replaceable()));
    }
    content = content.push(widgets::faint(app.i18n.settings_updates_sums()));

    widgets::card(content)
}

/// The state line. Every branch is something that is actually true right now, and the two that a
/// status line usually gets wrong are spelled out: "downloaded" is not "installed", and "installed"
/// is not "running".
fn update_headline(update: &UpdateView, app: &App) -> (Tone, String) {
    let i18n = &app.i18n;
    match &update.phase {
        UpdatePhase::Checking => (Tone::Neutral, i18n.settings_update_checking()),
        UpdatePhase::Downloading { received, total } => (
            Tone::Neutral,
            match total {
                Some(total) => i18n.settings_update_downloading(
                    human_bytes(*received, i18n),
                    human_bytes(*total, i18n),
                ),
                None => i18n.settings_update_downloading_unknown(human_bytes(*received, i18n)),
            },
        ),
        UpdatePhase::Staged { version } => (
            Tone::Warning,
            i18n.settings_update_staged(version.to_string()),
        ),
        UpdatePhase::Installed { version } => (
            Tone::Success,
            i18n.settings_update_installed(version.to_string()),
        ),
        UpdatePhase::Idle => match (update.current, update.latest) {
            // "Never asked" and "asked, and there was nothing" are different answers, and the
            // second one is what a build older than the naming convention will actually see.
            (_, None) if update.checked_at.is_none() => {
                (Tone::Neutral, i18n.settings_update_never_checked())
            }
            (_, None) => (Tone::Neutral, i18n.settings_update_no_image()),
            (Some(current), Some(latest)) if current < latest => (
                Tone::Warning,
                i18n.settings_update_available(latest.to_string()),
            ),
            (Some(current), Some(latest)) if current > latest => (
                Tone::Success,
                i18n.settings_update_ahead(current.to_string(), latest.to_string()),
            ),
            (Some(current), _) => (
                Tone::Success,
                i18n.settings_update_current(current.to_string()),
            ),
            (None, Some(latest)) => (
                Tone::Neutral,
                i18n.settings_update_versionless(latest.to_string()),
            ),
        },
    }
}

fn widget_toggle<'a>(
    value: bool,
    label: String,
    description: String,
    which: AppToggle,
) -> Element<'a, Message> {
    container(
        row![
            container(column![text(label).size(14), widgets::muted(description)].spacing(3),)
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

/// What the CLI is waiting for, in words a person reads. The kind is the interpreter's reading of
/// the CLI's own prompt; the prompt itself stays in the console, verbatim.
fn prompt_label(kind: &protonvpn_core::interpreter::PromptKind, i18n: &I18n) -> String {
    match kind {
        protonvpn_core::interpreter::PromptKind::Password => i18n.settings_prompt_password(),
        protonvpn_core::interpreter::PromptKind::TwoFactor => i18n.settings_prompt_two_factor(),
        protonvpn_core::interpreter::PromptKind::Unrecognised => {
            i18n.settings_prompt_unrecognised()
        }
    }
}

/// What a CLI setting is called, in words a person reads: its name and the sentence under it.
/// `None` for a key we have never seen — the settings tab still shows it, under its own name,
/// rather than hiding it.
///
/// The key itself is data and is never translated, and so are Proton's own names for its features:
/// NetShield, Kill switch, VPN Accelerator, Moderate NAT and IPv6 are the same words in every
/// language, which is what their catalogue entries say.
pub fn setting_label(key: &str, i18n: &I18n) -> Option<(String, String)> {
    Some(match key {
        "netshield" => (
            i18n.settings_name_netshield(),
            i18n.settings_hint_netshield(),
        ),
        "kill-switch" => (
            i18n.settings_name_kill_switch(),
            i18n.settings_hint_kill_switch(),
        ),
        "port-forwarding" => (
            i18n.settings_name_port_forwarding(),
            i18n.settings_hint_port_forwarding(),
        ),
        "custom-dns" => (
            i18n.settings_name_custom_dns(),
            i18n.settings_hint_custom_dns(),
        ),
        "vpn-accelerator" => (
            i18n.settings_name_vpn_accelerator(),
            i18n.settings_hint_vpn_accelerator(),
        ),
        "moderate-nat" => (
            i18n.settings_name_moderate_nat(),
            i18n.settings_hint_moderate_nat(),
        ),
        "ipv6" => (i18n.settings_name_ipv6(), i18n.settings_hint_ipv6()),
        "anonymous-crash-reports" => (
            i18n.settings_name_anonymous_crash_reports(),
            i18n.settings_hint_anonymous_crash_reports(),
        ),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use protonvpn_core::i18n::Locale;

    /// The catalogue the application is written in. Tests run in the source language, so a test
    /// that asserts wording asserts English; a test *about* another language builds that catalogue
    /// itself, as `russian_declines_what_english_does_not` does.
    fn english() -> I18n {
        I18n::new(Locale::SOURCE)
    }

    #[test]
    fn every_setting_we_label_is_one_the_cli_actually_has() {
        // The list is `protonvpn config list` on 1.0.3 (`docs/cli-surface.md` §1). If the CLI
        // grows a key we do not know, it still shows up — under its own name.
        let i18n = english();
        for key in [
            "netshield",
            "kill-switch",
            "port-forwarding",
            "custom-dns",
            "vpn-accelerator",
            "moderate-nat",
            "ipv6",
            "anonymous-crash-reports",
        ] {
            let (name, hint) = setting_label(key, &i18n).unwrap_or_else(|| panic!("{key}"));
            assert!(!name.is_empty(), "{key}");
            assert!(!hint.is_empty(), "{key}");
            assert!(setting_values(key).is_some(), "{key}");
        }
        assert!(setting_label("split-tunneling", &i18n).is_none());
        assert!(setting_label("auto-connect", &i18n).is_none());
    }

    /// The names Proton gives its own features are the same words in every language, and the
    /// catalogue entry is where that decision is written down rather than a literal in the view.
    #[test]
    fn a_name_we_do_not_translate_comes_from_the_catalogue_anyway() {
        let i18n = english();
        let (name, hint) = setting_label("netshield", &i18n).unwrap();
        assert_eq!(name, "NetShield");
        assert_eq!(hint, "Block malicious domains at the gateway's DNS level.");

        let (name, hint) = setting_label("kill-switch", &i18n).unwrap();
        assert_eq!(name, "Kill switch");
        assert_eq!(hint, "Block traffic if the tunnel drops.");

        let (name, _) = setting_label("port-forwarding", &i18n).unwrap();
        assert_eq!(name, "Port forwarding");
    }

    /// `protonvpn config list` prints values, not words: the picker shows our words for the ones we
    /// describe, and the CLI's own value for the ones we do not.
    #[test]
    fn a_value_is_ours_when_we_describe_it_and_the_clis_when_we_do_not() {
        let i18n = english();
        assert_eq!(value_label("netshield", "off", &i18n), "Off");
        assert_eq!(
            value_label("netshield", "malware-only", &i18n),
            "Malware only"
        );
        assert_eq!(
            value_label("netshield", "malware-ads-trackers", &i18n),
            "Malware, ads and trackers"
        );
        assert_eq!(value_label("kill-switch", "standard", &i18n), "Standard");
        assert_eq!(value_label("ipv6", "on", &i18n), "On");
        // A key or a value we do not describe is data, and data is shown as it arrived.
        assert_eq!(
            value_label("something-new", "some-value", &i18n),
            "some-value"
        );
    }

    /// One test about Russian, because that is where a copy of the English would still compile and
    /// still be wrong: the value names agree with the setting they describe, the generic ones do not
    /// agree the same way, and a count of sessions needs three forms where English needs two.
    #[test]
    fn russian_declines_what_english_does_not() {
        let russian = I18n::new(Locale::from_id("ru").unwrap());
        assert_eq!(value_label("netshield", "off", &russian), "выключен");
        assert_eq!(value_label("kill-switch", "off", &russian), "выключен");
        assert_eq!(value_label("ipv6", "off", &russian), "выключено");
        let (name, hint) = setting_label("netshield", &russian).unwrap();
        // Proton's own name is not translated, and neither is the fact that it blocks at the DNS
        // level of the gateway.
        assert_eq!(name, "NetShield");
        assert!(hint.contains("DNS"), "{hint}");

        // CLDR's categories, not English's: 1 and 21 are `one`, 3 is `few`, 5 is `many`.
        assert_eq!(
            russian.settings_socks5_overloaded(1),
            "· нет места ещё для 1 сессии"
        );
        assert_eq!(
            russian.settings_socks5_overloaded(3),
            "· нет места ещё для 3 сессий"
        );
        assert_eq!(
            russian.settings_socks5_overloaded(5),
            "· нет места ещё для 5 сессий"
        );
        assert_eq!(
            russian.settings_socks5_overloaded(21),
            "· нет места ещё для 21 сессии"
        );
    }

    /// The same count in English, which has two forms and no more.
    #[test]
    fn an_english_count_has_two_forms() {
        let i18n = english();
        assert_eq!(
            i18n.settings_socks5_overloaded(1),
            "· no room left for 1 more session"
        );
        assert_eq!(
            i18n.settings_socks5_overloaded(3),
            "· no room left for 3 more sessions"
        );
    }

    /// The three prompts the interpreter recognises are three different answers, and an
    /// unrecognised one says where the question can actually be read.
    #[test]
    fn every_prompt_kind_has_its_own_words() {
        use protonvpn_core::interpreter::PromptKind;

        let i18n = english();
        assert_eq!(prompt_label(&PromptKind::Password, &i18n), "password");
        assert_eq!(prompt_label(&PromptKind::TwoFactor, &i18n), "2FA code");
        assert!(prompt_label(&PromptKind::Unrecognised, &i18n).contains("console"));
    }
}
