//! The window — an elm-style update loop over the engine's snapshot.
//!
//! Views never call the runner (`docs/architecture.md` §1). Everything they do is a
//! [`Request`] sent to the engine, and everything they show comes from a snapshot plus the log
//! bus. That is what keeps the tray, the window and the interpreter from ever disagreeing about
//! what the CLI said.
//!
//! Two pieces of the contract are easy to lose in a UI and are therefore called out here:
//!
//! * **freshness is an age, never a verdict** (§7) — there is no "stale" badge anywhere below,
//! * **nothing is invented** (§5) — `Unknown` renders as "неизвестно", not as "отключено".

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use iced::widget::{
    button, checkbox, column, container, horizontal_rule, pick_list, row, scrollable, text,
    text_input, toggler,
};
use iced::{Alignment, Color, Element, Font, Length, Padding, Subscription, Task, Theme};
use iced::{clipboard, exit, time, window};

use protonvpn_core::config::{Config, ConfigStore};
use protonvpn_core::engine::{
    EngineHandle, EngineOptions, PendingPrompt, Request, Shared, TrayView, UiCommand,
};
use protonvpn_core::interpreter::PromptKind;
use protonvpn_core::launcher::Intent;
use protonvpn_core::model::{
    City, ConnectTarget, ConnectionStatus, Country, InvocationId, PortForwarding, RunnerStatus,
};

use crate::autostart;
use crate::console::ConsoleModel;
use crate::tray::{self, TrayCommand};

/// The window redraws on this beat: it is what makes the console feel live and what keeps the
/// "updated N mins ago" line climbing while the user watches. It is not a status poll — the
/// engine's polling policy (§7) is untouched by how often the view repaints.
const TICK: Duration = Duration::from_millis(200);
const CONSOLE_ID: &str = "console-transcript";

pub fn run() -> iced::Result {
    let store = ConfigStore::default();
    let (config, config_error) = match store.load() {
        Ok(config) => (config, None),
        Err(error) => (Config::default(), Some(error.to_string())),
    };

    // Keep the autostart entry in step with the config on every start: a user who deletes the
    // file should not be surprised by it coming back unasked, and one who enables it in the app
    // should not have to enable it twice.
    let autostart_note = match autostart::sync(config.autostart, &autostart::current_exec()) {
        Ok(_) => None,
        Err(error) => Some(format!("автозапуск: {error}")),
    };

    let start_minimized = config.start_minimized;
    let startup_connect = config
        .connect_at_startup
        .then(|| match &config.startup_country {
            Some(country) if !country.is_empty() => ConnectTarget::country(country.clone()),
            _ => ConnectTarget::fastest(),
        });

    // The tray comes first, because whether there *is* one decides whether a start-to-tray run is
    // possible at all: hiding into nothing is exactly what §9 forbids.
    let (tray_commands, tray_rx) = std::sync::mpsc::channel::<TrayCommand>();
    let initial_view = TrayView {
        status: ConnectionStatus::Unknown,
        detail: None,
        age_text: "updated just now".into(),
    };
    let tray = tray::spawn(tray_commands, initial_view);
    let tray_available = tray.is_some();
    let presenter = tray.map(Arc::new);

    // The window's app id is ours, and must never be `proton.vpn.app.gtk`: the CLI refuses to run
    // while that name is on the session bus (`docs/cli-surface.md` §2). It is also what lets the
    // desktop match the window to packaging/protonvpn-gui.desktop.
    let window_settings = window::Settings {
        size: iced::Size::new(900.0, 760.0),
        min_size: Some(iced::Size::new(560.0, 420.0)),
        exit_on_close_request: false,
        platform_specific: window::settings::PlatformSpecific {
            application_id: autostart::APP_ID.to_string(),
            ..Default::default()
        },
        ..Default::default()
    };
    let start_in_tray = start_minimized && tray_available;

    // A **daemon**, not an application, and this is the whole reason: `iced::application` exits
    // when its last window is destroyed, and winit cannot hide a window on Wayland at all
    // ("Not possible on Wayland" in `set_visible`). Closing the window therefore has to mean
    // *destroying* it, and the app has to keep running afterwards — which is what a daemon does.
    iced::daemon("Proton VPN", App::update, App::view)
        .subscription(App::subscription)
        .theme(|_state: &App, _window: window::Id| Theme::Dark)
        .run_with(move || {
            let mut options = EngineOptions::new(store, config.clone());
            options.tray_available = tray_available;
            options.tray = presenter.as_ref().map(|presenter| {
                Box::new(SharedPresenter(Arc::clone(presenter)))
                    as Box<dyn protonvpn_core::engine::TrayPresenter>
            });
            let engine = protonvpn_core::engine::spawn(options);

            let mut app = App::new(engine, config, tray_rx, presenter, window_settings);
            app.tray_available = tray_available;
            app.notice = config_error.or(autostart_note);

            // No window at all when the app is meant to start in the tray: "the app is in the tray
            // and connected to the configured country, no window" is an exit criterion.
            let open = if start_in_tray {
                Task::none()
            } else {
                let (id, open) = window::open(app.window_settings.clone());
                app.window = Some(id);
                open.map(Message::WindowOpened)
            };

            let task = Task::batch([
                open,
                match startup_connect {
                    Some(target) => Task::done(Message::StartupConnect(target)),
                    None => Task::none(),
                },
            ]);
            (app, task)
        })
}

/// Adapter so the tray handle can be shared with the engine and still be shut down by the app.
struct SharedPresenter(Arc<tray::TrayPresenter>);

impl protonvpn_core::engine::TrayPresenter for SharedPresenter {
    fn update(&self, view: TrayView) {
        self.0.update(view);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Connect,
    Countries,
    Settings,
    Port,
    Account,
}

impl Tab {
    fn label(self) -> &'static str {
        match self {
            Self::Connect => "Подключение",
            Self::Countries => "Страны и города",
            Self::Settings => "Настройки",
            Self::Port => "Проброс порта",
            Self::Account => "Аккаунт",
        }
    }

    const ALL: [Tab; 5] = [
        Tab::Connect,
        Tab::Countries,
        Tab::Settings,
        Tab::Port,
        Tab::Account,
    ];
}

/// Presets, exactly the ones the CLI exposes (`docs/cli-surface.md` §1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preset {
    Fastest,
    P2p,
    SecureCore,
    Tor,
    Random,
}

impl Preset {
    fn label(self) -> &'static str {
        match self {
            Self::Fastest => "Быстрейший",
            Self::P2p => "P2P",
            Self::SecureCore => "Secure Core",
            Self::Tor => "Tor",
            Self::Random => "Случайный",
        }
    }

    const ALL: [Preset; 5] = [
        Preset::Fastest,
        Preset::P2p,
        Preset::SecureCore,
        Preset::Tor,
        Preset::Random,
    ];
}

/// Our own switches, as opposed to the CLI's settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppToggle {
    ConnectAtStartup,
    StartMinimized,
    Autostart,
    Probe,
    PortForwarding,
}

/// The CLI's settings and the values it accepts.
///
/// Captured from `protonvpn config set <key> --help` on 1.0.3 rather than guessed. `None` means
/// the CLI's value set is open-ended, and the UI offers a text field instead of inventing options.
pub fn setting_values(key: &str) -> Option<&'static [&'static str]> {
    match key {
        "netshield" => Some(&["off", "malware-only", "malware-ads-trackers"]),
        "kill-switch" => Some(&["off", "standard"]),
        "port-forwarding"
        | "custom-dns"
        | "vpn-accelerator"
        | "moderate-nat"
        | "ipv6"
        | "anonymous-crash-reports" => Some(&["off", "on"]),
        _ => None,
    }
}

#[derive(Debug, Clone)]
pub enum Message {
    Tick,
    StartupConnect(ConnectTarget),
    CloseRequested(window::Id),
    WindowOpened(window::Id),
    WindowClosed(window::Id),
    ShowWindow,
    HideWindow,
    Quit,
    TabSelected(Tab),
    ToggleConsole,
    PrimaryAction,
    PresetSelected(Preset),
    ServerInputChanged(String),
    ConnectServer,
    Refresh,
    Probe,
    CountryFilterChanged(String),
    CountrySelected(String),
    ConnectCountry(String),
    CityFilterChanged(String),
    ConnectCity(String),
    LoadCountries,
    SettingChoice { key: String, value: String },
    DnsChanged(String),
    SettingDraft { key: String, value: String },
    SettingApply { key: String },
    Toggle(AppToggle, bool),
    LoginUsername(String),
    LoginPassword(String),
    LoginTwoFactor(String),
    StartupCountryChanged(String),
    LoginSubmit,
    Logout,
    ManualInput(String),
    ManualSend,
    PortRefresh,
    PortRelease,
    CopyPort,
    CopyInvocation(InvocationId),
    CopyAll,
    QbEnabled(bool),
    QbHost(String),
    QbPort(String),
    QbUsername(String),
    QbPassword(String),
    QbPushNow,
    ConsoleScrolled(scrollable::Viewport),
    ScrollToBottom,
    DismissNotice,
}

pub struct App {
    engine: EngineHandle,
    config: Config,
    shared: Shared,
    console: ConsoleModel,
    tray_commands: Receiver<TrayCommand>,
    tray: Option<Arc<tray::TrayPresenter>>,
    tray_available: bool,
    window: Option<window::Id>,
    window_settings: window::Settings,
    tab: Tab,
    console_expanded: bool,
    stick_to_bottom: bool,
    preset: Preset,
    server_input: String,
    country_filter: String,
    city_filter: String,
    selected_country: Option<String>,
    dns_servers: String,
    setting_drafts: HashMap<String, String>,
    login_username: String,
    login_password: String,
    login_two_factor: String,
    manual_input: String,
    qb_host: String,
    qb_port: String,
    qb_username: String,
    qb_password: String,
    notice: Option<String>,
    copied_port_at: Option<Instant>,
}

impl App {
    fn new(
        engine: EngineHandle,
        config: Config,
        tray_commands: Receiver<TrayCommand>,
        tray: Option<Arc<tray::TrayPresenter>>,
        window_settings: window::Settings,
    ) -> Self {
        let shared = engine.snapshot();
        let mut console = ConsoleModel::default();
        console.refresh(&engine.bus().lock().unwrap_or_else(|p| p.into_inner()));
        Self {
            engine,
            qb_host: config.qbittorrent.host.clone(),
            qb_port: config.qbittorrent.port.to_string(),
            qb_username: config.qbittorrent.username.clone(),
            config,
            shared,
            console,
            tray_commands,
            tray,
            tray_available: false,
            window: None,
            window_settings,
            tab: Tab::Connect,
            console_expanded: false,
            stick_to_bottom: true,
            preset: Preset::Fastest,
            server_input: String::new(),
            country_filter: String::new(),
            city_filter: String::new(),
            selected_country: None,
            dns_servers: String::new(),
            setting_drafts: HashMap::new(),
            login_username: String::new(),
            login_password: String::new(),
            login_two_factor: String::new(),
            manual_input: String::new(),
            qb_password: String::new(),
            notice: None,
            copied_port_at: None,
        }
    }

    fn subscription(&self) -> Subscription<Message> {
        Subscription::batch([
            time::every(TICK).map(|_| Message::Tick),
            window::close_requests().map(Message::CloseRequested),
            // A destroyed window must be forgotten, or the tray's "Открыть окно" would try to
            // focus something that no longer exists.
            window::close_events().map(Message::WindowClosed),
        ])
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Tick => self.tick(),
            Message::StartupConnect(target) => {
                self.engine.send(Request::Run(Intent::Connect(target)));
                Task::none()
            }
            Message::CloseRequested(id) => {
                if self.tray_available {
                    // Closing returns to the tray, which is the point of the app existing without
                    // a window. With no tray, hiding would hide it into nothing, so we say why
                    // instead (§9).
                    self.window = None;
                    return window::close(id);
                }
                self.notice = Some(
                    "Трея нет, скрывать окно некуда. Закрыть приложение — кнопка «Выход».".into(),
                );
                Task::none()
            }
            Message::WindowOpened(id) => {
                // The window the tray asked for exists now; remember it so the next click just
                // focuses it instead of opening another one.
                self.window = Some(id);
                Task::none()
            }
            Message::WindowClosed(id) => {
                if self.window == Some(id) {
                    self.window = None;
                }
                Task::none()
            }
            Message::ShowWindow => {
                self.engine.send(Request::Attention);
                match self.window {
                    Some(id) => Task::batch([
                        window::change_mode(id, window::Mode::Windowed),
                        window::gain_focus(id),
                    ]),
                    None => {
                        // The window was destroyed on close; the tray brings a fresh one back.
                        // The daemon opened nothing by itself, so there is exactly one at a time.
                        let (id, open) = window::open(self.window_settings.clone());
                        self.window = Some(id);
                        Task::batch([open.map(Message::WindowOpened)])
                    }
                }
            }
            Message::HideWindow => match (self.window, self.tray_available) {
                (Some(id), true) => {
                    self.window = None;
                    window::close(id)
                }
                _ => {
                    self.notice = Some("Трея нет — окно остаётся открытым.".into());
                    Task::none()
                }
            },
            Message::Quit => {
                if let Some(tray) = &self.tray {
                    tray.shutdown();
                }
                // Give the engine a moment to hand the forwarded port back before the process
                // goes away; a stale mapping outliving the app is exactly the misinformation
                // §10.1 warns about.
                self.engine.shutdown(Duration::from_millis(1500));
                exit()
            }
            Message::TabSelected(tab) => {
                self.tab = tab;
                // A tab that needs state asks for it the first time it is opened: the CLI costs
                // about a second per invocation, so nothing is fetched until it is wanted.
                match tab {
                    Tab::Countries if self.shared.state.countries.is_none() => {
                        self.engine.send(Request::Run(Intent::ListCountries));
                    }
                    Tab::Settings if self.shared.state.settings.is_none() => {
                        self.engine.send(Request::Run(Intent::ListSettings));
                    }
                    Tab::Account if self.shared.state.account.is_none() => {
                        self.engine.send(Request::Run(Intent::AccountInfo));
                    }
                    _ => {}
                }
                Task::none()
            }
            Message::ToggleConsole => {
                self.console_expanded = !self.console_expanded;
                if self.console_expanded {
                    self.stick_to_bottom = true;
                    return self.scroll_to_bottom();
                }
                Task::none()
            }
            Message::PrimaryAction => {
                if self.shared.state.connection.value.is_connected() {
                    self.engine.send(Request::Run(Intent::Disconnect));
                } else {
                    let target = self.connect_target();
                    self.engine.send(Request::Run(Intent::Connect(target)));
                }
                Task::none()
            }
            Message::PresetSelected(preset) => {
                self.preset = preset;
                self.server_input.clear();
                Task::none()
            }
            Message::ServerInputChanged(value) => {
                self.server_input = value;
                Task::none()
            }
            Message::ConnectServer => {
                let server = self.server_input.trim().to_string();
                if server.is_empty() {
                    return Task::none();
                }
                self.engine
                    .send(Request::Run(Intent::Connect(ConnectTarget {
                        server: Some(server),
                        ..Default::default()
                    })));
                Task::none()
            }
            Message::ConnectCountry(code) => {
                self.selected_country = Some(code.clone());
                self.engine
                    .send(Request::Run(Intent::Connect(ConnectTarget::country(code))));
                Task::none()
            }
            Message::Refresh => {
                self.engine.send(Request::Run(Intent::RefreshStatus));
                Task::none()
            }
            Message::Probe => {
                self.engine.send(Request::Probe);
                Task::none()
            }
            Message::CountryFilterChanged(value) => {
                self.country_filter = value;
                Task::none()
            }
            Message::CountrySelected(code) => {
                self.selected_country = Some(code.clone());
                self.city_filter.clear();
                self.engine
                    .send(Request::Run(Intent::ListCities { country: code }));
                Task::none()
            }
            Message::CityFilterChanged(value) => {
                self.city_filter = value;
                Task::none()
            }
            Message::ConnectCity(city) => {
                self.engine
                    .send(Request::Run(Intent::Connect(ConnectTarget {
                        city: Some(city),
                        ..Default::default()
                    })));
                Task::none()
            }
            Message::LoadCountries => {
                self.engine.send(Request::Run(Intent::ListCountries));
                Task::none()
            }
            Message::SettingChoice { key, value } => {
                self.apply_setting(key, value);
                Task::none()
            }
            Message::SettingDraft { key, value } => {
                self.setting_drafts.insert(key, value);
                Task::none()
            }
            Message::SettingApply { key } => {
                let value = self.setting_drafts.remove(&key).unwrap_or_default();
                if value.trim().is_empty() {
                    self.notice = Some("Значение не может быть пустым.".into());
                    return Task::none();
                }
                self.apply_setting(key, value.trim().to_string());
                Task::none()
            }
            Message::DnsChanged(value) => {
                self.dns_servers = value;
                Task::none()
            }
            Message::Toggle(which, value) => {
                self.toggle(which, value);
                Task::none()
            }
            Message::StartupCountryChanged(value) => {
                let mut config = self.config.clone();
                config.startup_country = Some(value).filter(|code| !code.trim().is_empty());
                self.save_config(config);
                Task::none()
            }
            Message::LoginUsername(value) => {
                self.login_username = value;
                Task::none()
            }
            Message::LoginPassword(value) => {
                self.login_password = value;
                Task::none()
            }
            Message::LoginTwoFactor(value) => {
                self.login_two_factor = value;
                Task::none()
            }
            Message::LoginSubmit => {
                let username = self.login_username.trim().to_string();
                if username.is_empty() || self.login_password.is_empty() {
                    self.notice = Some("Нужны имя пользователя и пароль.".into());
                    return Task::none();
                }
                self.engine.send(Request::SignIn {
                    username,
                    password: self.login_password.clone(),
                    two_factor: Some(self.login_two_factor.clone())
                        .filter(|code| !code.trim().is_empty()),
                });
                // The password does not linger in the window state either.
                self.login_password.clear();
                self.login_two_factor.clear();
                Task::none()
            }
            Message::Logout => {
                self.engine.send(Request::Run(Intent::SignOut));
                Task::none()
            }
            Message::ManualInput(value) => {
                self.manual_input = value;
                Task::none()
            }
            Message::ManualSend => {
                let text = std::mem::take(&mut self.manual_input);
                if !text.is_empty() {
                    self.engine.send(Request::Stdin { text });
                }
                Task::none()
            }
            Message::PortRefresh => {
                self.engine.send(Request::PortForwardRefresh);
                Task::none()
            }
            Message::PortRelease => {
                self.engine.send(Request::ReleasePort);
                Task::none()
            }
            Message::CopyPort => match self.shared.state.port_forwarding.value.port() {
                Some(port) => {
                    self.copied_port_at = Some(Instant::now());
                    clipboard::write(port.to_string())
                }
                None => Task::none(),
            },
            Message::CopyInvocation(id) => {
                let text = self
                    .console
                    .blocks
                    .iter()
                    .find(|block| block.id == id)
                    .map(|block| block.transcript.clone());
                match text {
                    Some(text) => clipboard::write(text),
                    None => Task::none(),
                }
            }
            Message::CopyAll => clipboard::write(self.console.transcript.clone()),
            Message::QbEnabled(enabled) => {
                let mut config = self.config.clone();
                config.qbittorrent.enabled = enabled;
                self.save_config(config);
                Task::none()
            }
            Message::QbHost(value) => {
                self.qb_host = value;
                Task::none()
            }
            Message::QbPort(value) => {
                self.qb_port = value;
                Task::none()
            }
            Message::QbUsername(value) => {
                self.qb_username = value;
                Task::none()
            }
            Message::QbPassword(value) => {
                self.qb_password = value;
                Task::none()
            }
            Message::QbPushNow => {
                let port = self.qb_port.trim().parse::<u16>().unwrap_or(8080);
                let mut config = self.config.clone();
                config.qbittorrent.host = self.qb_host.trim().to_string();
                config.qbittorrent.port = port;
                config.qbittorrent.username = self.qb_username.clone();
                self.save_config(config);
                self.engine
                    .send(Request::SetQBittorrentPassword(self.qb_password.clone()));
                // A push happens on the next port change; this nudges the lease so the change is
                // real rather than cosmetic.
                self.engine.send(Request::PortForwardRefresh);
                Task::none()
            }
            Message::ConsoleScrolled(viewport) => {
                // "Stick to bottom" while the user is at the bottom; stop as soon as they scroll
                // up, because fighting a reader for the scroll position is the classic way to make
                // a live log unusable.
                let at_bottom = viewport.relative_offset().y >= 0.995;
                self.stick_to_bottom = at_bottom;
                Task::none()
            }
            Message::ScrollToBottom => {
                self.stick_to_bottom = true;
                self.scroll_to_bottom()
            }
            Message::DismissNotice => {
                self.notice = None;
                Task::none()
            }
        }
    }

    /// One beat: take whatever the tray and the engine have produced, and repaint.
    fn tick(&mut self) -> Task<Message> {
        let mut tasks: Vec<Task<Message>> = Vec::new();

        while let Ok(command) = self.tray_commands.try_recv() {
            match command {
                TrayCommand::Show => tasks.push(Task::done(Message::ShowWindow)),
                TrayCommand::Quit => tasks.push(Task::done(Message::Quit)),
                TrayCommand::Connect => self
                    .engine
                    .send(Request::Run(Intent::Connect(self.connect_target()))),
                TrayCommand::Disconnect => self.engine.send(Request::Run(Intent::Disconnect)),
            }
        }

        let console_before = self.console.version();
        let shared = self.engine.snapshot();
        let prompt_changed = shared.pending_prompt != self.shared.pending_prompt;
        self.shared = shared;

        for command in self.engine.take_ui_commands() {
            match command {
                UiCommand::ShowWindow => tasks.push(Task::done(Message::ShowWindow)),
                UiCommand::HideWindow => tasks.push(Task::done(Message::HideWindow)),
                UiCommand::ToggleWindow => {
                    tasks.push(Task::done(Message::ToggleConsole));
                }
                UiCommand::Quit => tasks.push(Task::done(Message::Quit)),
            }
        }

        {
            let bus = self.engine.bus();
            let bus = bus.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            self.console.refresh(&bus);
        }

        if prompt_changed {
            self.manual_input.clear();
        }

        if self.console_expanded && self.stick_to_bottom && self.console.version() != console_before
        {
            tasks.push(self.scroll_to_bottom());
        }

        if tasks.is_empty() {
            Task::none()
        } else {
            Task::batch(tasks)
        }
    }

    fn scroll_to_bottom(&self) -> Task<Message> {
        scrollable::snap_to(
            scrollable::Id::new(CONSOLE_ID),
            scrollable::RelativeOffset::END,
        )
    }

    fn connect_target(&self) -> ConnectTarget {
        if !self.server_input.trim().is_empty() {
            return ConnectTarget {
                server: Some(self.server_input.trim().to_string()),
                ..Default::default()
            };
        }
        match self.preset {
            Preset::Fastest => match &self.selected_country {
                Some(country) => ConnectTarget::country(country.clone()),
                None => ConnectTarget::fastest(),
            },
            Preset::P2p => ConnectTarget {
                p2p: true,
                ..Default::default()
            },
            Preset::SecureCore => ConnectTarget {
                secure_core: true,
                ..Default::default()
            },
            Preset::Tor => ConnectTarget {
                tor: true,
                ..Default::default()
            },
            Preset::Random => ConnectTarget {
                random: true,
                ..Default::default()
            },
        }
    }

    fn apply_setting(&mut self, key: String, value: String) {
        let dns = (key == "custom-dns" && value == "on").then(|| self.dns_servers.clone());
        if let Some(dns) = &dns
            && dns.trim().is_empty()
        {
            self.notice =
                Some("Для custom-dns on укажите серверы: без них CLI отклонит команду.".into());
            return;
        }
        self.engine
            .send(Request::Run(Intent::SetSetting { key, value, dns }));
    }

    fn toggle(&mut self, which: AppToggle, value: bool) {
        match which {
            AppToggle::Probe => {
                let mut config = self.config.clone();
                config.probe_enabled = value;
                self.save_config(config);
            }
            AppToggle::PortForwarding => {
                let mut config = self.config.clone();
                config.port_forwarding_enabled = value;
                self.save_config(config);
            }
            AppToggle::ConnectAtStartup => {
                let mut config = self.config.clone();
                config.connect_at_startup = value;
                if value && config.startup_country.is_none() {
                    config.startup_country = self.selected_country.clone();
                }
                self.save_config(config);
            }
            AppToggle::StartMinimized => {
                let mut config = self.config.clone();
                config.start_minimized = value;
                self.save_config(config);
            }
            AppToggle::Autostart => {
                let mut config = self.config.clone();
                config.autostart = value;
                match autostart::sync(value, &autostart::current_exec()) {
                    Ok(_) => self.save_config(config),
                    Err(error) => self.notice = Some(format!("автозапуск: {error}")),
                }
            }
        }
    }

    fn save_config(&mut self, config: Config) {
        self.config = config.clone();
        self.engine.send(Request::SaveConfig(Box::new(config)));
    }

    // --- views ------------------------------------------------------------------------------

    fn view(&self, _window: window::Id) -> Element<'_, Message> {
        let header = self.header();
        let tabs = row(Tab::ALL.iter().map(|tab| {
            let selected = *tab == self.tab;
            button(text(tab.label()).size(14))
                .padding(Padding::from([6, 12]))
                .style(if selected {
                    button::primary
                } else {
                    button::secondary
                })
                .on_press(Message::TabSelected(*tab))
                .into()
        }))
        .spacing(6)
        .width(Length::Fill);

        let body: Element<'_, Message> = match self.tab {
            Tab::Connect => self.connection_view(),
            Tab::Countries => self.countries_view(),
            Tab::Settings => self.settings_view(),
            Tab::Port => self.port_view(),
            Tab::Account => self.account_view(),
        };

        let shell = column![
            header,
            tabs,
            container(body)
                .height(Length::Fill)
                .padding(Padding::from([4, 0])),
            self.console_view(),
        ]
        .spacing(8)
        .padding(12)
        .width(Length::Fill);

        if let Some(notice) = &self.notice {
            column![
                container(row![
                    text(notice).width(Length::Fill),
                    button("ок")
                        .on_press(Message::DismissNotice)
                        .style(button::secondary),
                ])
                .padding(8)
                .style(container::bordered_box),
                shell,
            ]
            .into()
        } else {
            shell.into()
        }
    }

    fn header(&self) -> Element<'_, Message> {
        let state = &self.shared.state;
        let status = &state.connection.value;
        let color = status_color(status);

        // The status block and the actions are stacked rather than side by side: a row that has
        // to share its width between a long status line and five buttons is the kind of layout
        // that looks fine at one window size and clips at another.
        let status_line = row![
            text(status.label())
                .size(24)
                .style(move |_theme: &Theme| text::Style { color: Some(color) }),
            text(state.connection.age_text()).size(13),
        ]
        .spacing(12)
        .align_y(Alignment::Center);

        let mut facts: Vec<String> = Vec::new();
        match status {
            ConnectionStatus::Connected(info) => {
                facts.push(info.describe());
                if let Some(load) = info.load_percent {
                    facts.push(format!("загрузка {load}%"));
                }
                if let Some(protocol) = &info.protocol {
                    facts.push(protocol.clone());
                }
            }
            ConnectionStatus::Error(message) => facts.push(message.clone()),
            ConnectionStatus::Unknown => {
                facts.push("CLI ещё не отвечал — состояние не выдумывается".into())
            }
            _ => {}
        }
        if let Some(runner) = match &self.shared.runner {
            RunnerStatus::Running { .. } | RunnerStatus::Queued { .. } => {
                Some(self.shared.runner.render())
            }
            RunnerStatus::Idle => None,
        } {
            facts.push(runner);
        }

        let primary_label = if status.is_connected() {
            "Отключиться"
        } else {
            "Подключиться"
        };
        let primary_style = if status.is_connected() {
            button::danger
        } else {
            button::success
        };

        let actions = row![
            button(text(primary_label).size(16))
                .padding(Padding::from([10, 22]))
                .style(primary_style)
                .on_press(Message::PrimaryAction),
            button("Обновить")
                .on_press(Message::Refresh)
                .style(button::secondary),
            button("Проверить адрес")
                .on_press(Message::Probe)
                .style(button::secondary),
            button("В трей")
                .on_press(Message::HideWindow)
                .style(button::secondary),
            button("Выход")
                .on_press(Message::Quit)
                .style(button::secondary),
        ]
        .spacing(6)
        .align_y(Alignment::Center);

        container(column![status_line, text(facts.join(" · ")).size(14), actions,].spacing(6))
            .width(Length::Fill)
            .padding(12)
            .style(container::rounded_box)
            .into()
    }

    fn connection_view(&self) -> Element<'_, Message> {
        let presets = row(Preset::ALL.iter().map(|preset| {
            let selected = *preset == self.preset;
            button(text(preset.label()).size(13))
                .padding(Padding::from([5, 10]))
                .style(if selected {
                    button::primary
                } else {
                    button::secondary
                })
                .on_press(Message::PresetSelected(*preset))
                .into()
        }))
        .spacing(6)
        .width(Length::Fill);

        let countries: Vec<String> = self
            .shared
            .state
            .countries
            .as_ref()
            .map(|observation| observation.value.iter().map(|c| c.code.clone()).collect())
            .unwrap_or_default();
        let country_picker = if countries.is_empty() {
            row![
                text("Список стран ещё не загружен.").size(13),
                button("Загрузить страны")
                    .on_press(Message::LoadCountries)
                    .style(button::secondary),
            ]
            .spacing(8)
        } else {
            row![
                text("Страна:").size(13),
                pick_list(
                    countries,
                    self.selected_country.clone(),
                    Message::ConnectCountry
                )
                .text_size(13)
                .width(Length::Fixed(120.0)),
            ]
            .spacing(8)
            .align_y(Alignment::Center)
        };

        let egress = self.egress_view();

        let mut content = column![
            section("Подключение", presets.into()),
            container(
                column![
                    country_picker,
                    row![
                        text_input("или конкретный сервер, например IT#23", &self.server_input)
                            .on_input(Message::ServerInputChanged)
                            .on_submit(Message::ConnectServer)
                            .width(Length::Fixed(320.0)),
                        button("Подключить сервер")
                            .on_press(Message::ConnectServer)
                            .style(button::secondary),
                    ]
                    .spacing(8)
                    .align_y(Alignment::Center),
                    text(
                        "Пресеты и страна взаимоисключающи: пресет — это флаг CLI, страна — \
                         `--country`."
                    )
                    .size(12),
                ]
                .spacing(10),
            )
            .padding(12)
            .style(container::bordered_box),
            section("Что видно снаружи", egress),
        ]
        .spacing(14);

        if let Some(block) = self.console.last()
            && block.running
        {
            content = content.push(text(format!("Выполняется: {}", block.command)).size(13));
        }
        scrollable(content).height(Length::Fill).into()
    }

    fn egress_view(&self) -> Element<'_, Message> {
        let egress = &self.shared.state.egress;
        let mut lines: Vec<Element<'_, Message>> = Vec::new();

        match (&egress.baseline, &egress.current) {
            (Some(baseline), Some(current)) => {
                lines.push(
                    text(format!("до подключения: {}", baseline.value.ip))
                        .size(13)
                        .into(),
                );
                lines.push(
                    text(format!(
                        "сейчас: {} ({})",
                        current.value.ip,
                        current.value.asn_org.clone().unwrap_or_else(|| "—".into())
                    ))
                    .size(13)
                    .into(),
                );
                match egress.egress_changed() {
                    Some(true) => lines.push(
                        text("Адрес изменился — трафик идёт через туннель.")
                            .size(13)
                            .into(),
                    ),
                    Some(false) => lines.push(
                        text(
                            "Адрес не изменился. Если CLI говорит «подключено», туннель не несёт \
                             трафик.",
                        )
                        .size(13)
                        .into(),
                    ),
                    None => lines.push(
                        text("Сравнить не с чем: адреса получены от разных сервисов.")
                            .size(13)
                            .into(),
                    ),
                }
            }
            (Some(baseline), None) => {
                lines.push(
                    text(format!("до подключения: {}", baseline.value.ip))
                        .size(13)
                        .into(),
                );
                lines.push(text("Текущий адрес ещё не проверялся.").size(13).into());
            }
            _ => {
                lines.push(
                    text(
                        "Внешний адрес ещё не измерялся. Проверка отвечает на вопрос, который \
                         CLI не может: действительно ли трафик уходит через туннель.",
                    )
                    .size(13)
                    .into(),
                );
            }
        }

        lines.push(
            text(
                "Страна и ASN — только для чтения: базы GeoIP расходятся между собой, поэтому \
                 это не доказательство.",
            )
            .size(12)
            .into(),
        );

        column(lines).spacing(4).into()
    }

    fn countries_view(&self) -> Element<'_, Message> {
        let state = &self.shared.state;
        let countries: Vec<&Country> = match &state.countries {
            Some(observation) => {
                let needle = self.country_filter.trim().to_lowercase();
                observation
                    .value
                    .iter()
                    .filter(|country| {
                        needle.is_empty()
                            || country.name.to_lowercase().contains(&needle)
                            || country.code.to_lowercase().contains(&needle)
                    })
                    .collect()
            }
            None => Vec::new(),
        };

        let list = column(countries.into_iter().map(|country| {
            let selected = self.selected_country.as_deref() == Some(country.code.as_str());
            let label = format!("{} ({})", country.name, country.code);
            row![
                button(text(label).size(13))
                    .width(Length::Fill)
                    .padding(Padding::from([4, 8]))
                    .style(if selected {
                        button::primary
                    } else {
                        button::secondary
                    })
                    .on_press(Message::CountrySelected(country.code.clone())),
                button("Подключить")
                    .padding(Padding::from([4, 8]))
                    .style(button::success)
                    .on_press(Message::ConnectCountry(country.code.clone())),
            ]
            .spacing(6)
            .into()
        }))
        .spacing(3);

        let cities: Element<'_, Message> = match (&state.cities, &state.cities_country) {
            (Some(observation), Some(country)) => {
                let needle = self.city_filter.trim().to_lowercase();
                let items: Vec<&City> = observation
                    .value
                    .iter()
                    .filter(|city| needle.is_empty() || city.name.to_lowercase().contains(&needle))
                    .collect();
                let mut panel = column![
                    text(format!("Города: {country}")).size(15),
                    text(observation.age_text()).size(12),
                ]
                .spacing(4);
                if items.is_empty() {
                    panel = panel.push(text("В этой стране нет городов в списке.").size(13));
                }
                for city in items {
                    let features = if city.features.is_empty() {
                        "—".to_string()
                    } else {
                        city.features.join(", ")
                    };
                    panel = panel.push(
                        row![
                            text(city.name.clone()).size(13).width(Length::Fill),
                            text(features).size(12).width(Length::Fixed(180.0)),
                            button("Подключить")
                                .padding(Padding::from([3, 8]))
                                .style(button::success)
                                .on_press(Message::ConnectCity(city.name.clone())),
                        ]
                        .spacing(6)
                        .align_y(Alignment::Center),
                    );
                }
                scrollable(panel.spacing(4)).height(Length::Fill).into()
            }
            _ => container(
                text("Выберите страну слева, чтобы увидеть её города и их особенности.").size(13),
            )
            .padding(12)
            .into(),
        };

        column![
            row![
                text_input("Поиск страны", &self.country_filter)
                    .on_input(Message::CountryFilterChanged)
                    .width(Length::Fixed(220.0)),
                text_input("Поиск города", &self.city_filter)
                    .on_input(Message::CityFilterChanged)
                    .width(Length::Fixed(180.0)),
                button("Обновить список")
                    .on_press(Message::LoadCountries)
                    .style(button::secondary),
                match &state.countries {
                    Some(observation) => text(observation.age_text()).size(12),
                    None => text("список не загружен").size(12),
                },
            ]
            .spacing(8)
            .align_y(Alignment::Center),
            row![
                scrollable(list)
                    .height(Length::Fill)
                    .width(Length::FillPortion(1)),
                container(cities)
                    .width(Length::FillPortion(1))
                    .height(Length::Fill)
                    .padding(8)
                    .style(container::bordered_box),
            ]
            .spacing(10)
            .height(Length::Fill),
        ]
        .spacing(8)
        .height(Length::Fill)
        .into()
    }

    fn settings_view(&self) -> Element<'_, Message> {
        let state = &self.shared.state;

        let cli_settings: Element<'_, Message> = match &state.settings {
            Some(observation) => {
                let rows = column(observation.value.iter().map(|setting| {
                    let label = text(setting.key.clone())
                        .size(13)
                        .width(Length::FillPortion(2));
                    let control: Element<'_, Message> = match setting_values(&setting.key) {
                        Some(values) => pick_list(
                            values.iter().map(|v| v.to_string()).collect::<Vec<_>>(),
                            Some(setting.value.clone()),
                            move |value| Message::SettingChoice {
                                key: setting.key.clone(),
                                value,
                            },
                        )
                        .text_size(13)
                        .width(Length::Fixed(220.0))
                        .into(),
                        None => row![
                            text_input("значение", self.setting_draft(&setting.key))
                                .on_input({
                                    let key = setting.key.clone();
                                    move |value| Message::SettingDraft {
                                        key: key.clone(),
                                        value,
                                    }
                                })
                                .on_submit(Message::SettingApply {
                                    key: setting.key.clone()
                                })
                                .width(Length::Fixed(180.0)),
                            button("Применить").style(button::secondary).on_press(
                                Message::SettingApply {
                                    key: setting.key.clone()
                                }
                            ),
                        ]
                        .spacing(6)
                        .into(),
                    };
                    row![
                        label,
                        text(setting.value.clone())
                            .size(13)
                            .width(Length::Fixed(140.0)),
                        control,
                    ]
                    .spacing(8)
                    .align_y(Alignment::Center)
                    .into()
                }))
                .spacing(6);

                column![
                    row![
                        text("Настройки CLI").size(16),
                        text(observation.age_text()).size(12),
                        button("Обновить")
                            .on_press(Message::Refresh)
                            .style(button::secondary),
                    ]
                    .spacing(10)
                    .align_y(Alignment::Center),
                    text(
                        "Значения берутся из `config list`; после `config set` список \
                         перечитывается, потому что только он — доказательство."
                    )
                    .size(12),
                    rows,
                    row![
                        text("Серверы для custom-dns:").size(13),
                        text_input("1.1.1.1,8.8.8.8", &self.dns_servers)
                            .on_input(Message::DnsChanged)
                            .width(Length::Fixed(240.0)),
                    ]
                    .spacing(8)
                    .align_y(Alignment::Center),
                ]
                .spacing(8)
                .into()
            }
            None => column![
                text("Настройки CLI ещё не прочитаны.").size(13),
                button("Прочитать `config list`")
                    .on_press(Message::Refresh)
                    .style(button::secondary),
            ]
            .spacing(8)
            .into(),
        };

        let startup_country = self.config.startup_country.clone().unwrap_or_default();
        let ours = column![
            row![
                toggler(self.config.connect_at_startup)
                    .label("Подключаться при запуске")
                    .on_toggle(|value| Message::Toggle(AppToggle::ConnectAtStartup, value))
                    .text_size(13),
                row![
                    text("страна:").size(13),
                    text_input("код, напр. CH", &startup_country)
                        .on_input(Message::StartupCountryChanged)
                        .width(Length::Fixed(100.0)),
                ]
                .spacing(6)
                .align_y(Alignment::Center),
            ]
            .spacing(16)
            .align_y(Alignment::Center),
            toggler(self.config.start_minimized)
                .label("Запускать свёрнутым в трей")
                .on_toggle(|value| Message::Toggle(AppToggle::StartMinimized, value))
                .text_size(13),
            toggler(self.config.autostart)
                .label("Автозапуск (~/.config/autostart)")
                .on_toggle(|value| Message::Toggle(AppToggle::Autostart, value))
                .text_size(13),
            text(format!(
                "Файл автозапуска: {}",
                if autostart::is_enabled() {
                    "есть"
                } else {
                    "нет"
                }
            ))
            .size(12),
            toggler(self.config.probe_enabled)
                .label("Проверять внешний адрес через curl (исключение №1)")
                .on_toggle(|value| Message::Toggle(AppToggle::Probe, value))
                .text_size(13),
            toggler(self.config.port_forwarding_enabled)
                .label("Держать аренду порта через NAT-PMP (исключение №2)")
                .on_toggle(|value| Message::Toggle(AppToggle::PortForwarding, value))
                .text_size(13),
            text(
                "Настройки приложения хранятся в ~/.config/protonvpn-gui/config.json. \
                 Файлы официального приложения мы не читаем и не пишем."
            )
            .size(12),
        ]
        .spacing(8);

        scrollable(
            column![
                section("Настройки приложения", ours.into()),
                section("Настройки CLI", cli_settings),
            ]
            .spacing(16),
        )
        .height(Length::Fill)
        .into()
    }

    fn setting_draft<'a>(&'a self, key: &str) -> &'a str {
        self.setting_drafts
            .get(key)
            .map(String::as_str)
            .unwrap_or("")
    }

    fn port_view(&self) -> Element<'_, Message> {
        let state = &self.shared.state;
        let age = state.port_forwarding.age_text();

        let port_block: Element<'_, Message> = match &state.port_forwarding.value {
            PortForwarding::Active { port, lifetime, .. } => column![
                row![
                    text(port.to_string())
                        .size(40)
                        .font(Font::MONOSPACE)
                        .width(Length::Fill),
                    button("Скопировать")
                        .padding(Padding::from([8, 16]))
                        .style(button::primary)
                        .on_press(Message::CopyPort),
                ]
                .align_y(Alignment::Center),
                text(format!(
                    "аренда {} с, продлевается автоматически",
                    lifetime.as_secs()
                ))
                .size(13),
                if self
                    .copied_port_at
                    .is_some_and(|at| at.elapsed() < Duration::from_secs(2))
                {
                    text("Порт скопирован в буфер обмена").size(13)
                } else {
                    text("").size(13)
                },
            ]
            .spacing(4)
            .into(),
            PortForwarding::Pending => text("Запрашиваю аренду у шлюза через NAT-PMP…")
                .size(15)
                .into(),
            PortForwarding::Unsupported => text(
                "Этот сервер не поддерживает проброс порта. Подключитесь к P2P-серверу: \
                 в списке городов такие помечены.",
            )
            .size(15)
            .into(),
            PortForwarding::Unavailable(reason) => column![
                text("Проброс порта недоступен").size(15),
                text(reason.clone()).size(13),
                text(
                    "Порт не показывается намеренно: показывать номер, который никто не \
                         продлевает, значит вводить в заблуждение."
                )
                .size(12),
            ]
            .spacing(4)
            .into(),
            PortForwarding::Idle => text(
                "Аренды нет. Порт запрашивается после подключения к серверу с поддержкой \
                 проброса.",
            )
            .size(15)
            .into(),
        };

        let qb = &self.config.qbittorrent;
        let qbittorrent_panel = column![
            checkbox(
                "Передавать порт в qBittorrent (исключение №3, по умолчанию выключено)",
                qb.enabled,
            )
            .on_toggle(Message::QbEnabled)
            .text_size(13),
            row![
                text("Хост").size(13).width(Length::Fixed(90.0)),
                text_input("localhost", &self.qb_host)
                    .on_input(Message::QbHost)
                    .width(Length::Fixed(180.0)),
                text("Порт").size(13),
                text_input("8080", &self.qb_port)
                    .on_input(Message::QbPort)
                    .width(Length::Fixed(80.0)),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
            row![
                text("Логин").size(13).width(Length::Fixed(90.0)),
                text_input("обычно пусто", &self.qb_username)
                    .on_input(Message::QbUsername)
                    .width(Length::Fixed(180.0)),
                text("Пароль").size(13),
                text_input("не сохраняется", &self.qb_password)
                    .secure(true)
                    .on_input(Message::QbPassword)
                    .width(Length::Fixed(180.0)),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
            row![
                button("Сохранить и передать порт")
                    .style(button::primary)
                    .on_press(Message::QbPushNow),
                text(
                    "Пароль живёт только в памяти процесса и никогда не пишется в конфиг. \
                     Соединение только с localhost."
                )
                .size(12),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        ]
        .spacing(8);

        scrollable(
            column![
                section(
                    "Проброс порта",
                    column![
                        row![
                            text(age).size(12),
                            button("Запросить заново")
                                .style(button::secondary)
                                .on_press(Message::PortRefresh),
                            button("Освободить")
                                .style(button::secondary)
                                .on_press(Message::PortRelease),
                        ]
                        .spacing(8),
                        port_block,
                        text(
                            "Порт выдаётся шлюзом 10.2.0.1 по NAT-PMP и меняется после \
                             переподключения. Аренда — 60 секунд, продление — каждые 40."
                        )
                        .size(12),
                    ]
                    .spacing(8)
                    .into(),
                ),
                section("qBittorrent", qbittorrent_panel.into()),
            ]
            .spacing(16),
        )
        .height(Length::Fill)
        .into()
    }

    fn account_view(&self) -> Element<'_, Message> {
        let state = &self.shared.state;
        let account = state
            .account
            .as_ref()
            .and_then(|observation| observation.value.name.clone());

        let status = match &account {
            Some(name) => column![
                text(format!("Вы вошли как {name}")).size(16),
                text(state.account.as_ref().unwrap().age_text()).size(12),
                button("Выйти")
                    .style(button::danger)
                    .on_press(Message::Logout),
            ]
            .spacing(6),
            None => column![
                text("Аккаунт не подтверждён: `protonvpn info` не назвал имя.").size(14),
                text("Если вы не входили — заполните форму ниже.").size(12),
            ]
            .spacing(6),
        };

        let login = column![
            text("Вход в Proton").size(16),
            text(
                "Пароль и код 2FA уходят прямо в PTY-терминал процесса CLI. В журнал они не \
                 попадают: консоль показывает только то, что напечатал CLI."
            )
            .size(12),
            text_input("имя пользователя", &self.login_username)
                .on_input(Message::LoginUsername)
                .width(Length::Fixed(280.0)),
            text_input("пароль", &self.login_password)
                .secure(true)
                .on_input(Message::LoginPassword)
                .on_submit(Message::LoginSubmit)
                .width(Length::Fixed(280.0)),
            text_input("код 2FA (если включён)", &self.login_two_factor)
                .secure(true)
                .on_input(Message::LoginTwoFactor)
                .on_submit(Message::LoginSubmit)
                .width(Length::Fixed(280.0)),
            button("Войти")
                .style(button::primary)
                .on_press(Message::LoginSubmit),
        ]
        .spacing(8);

        let prompt: Element<'_, Message> = match &self.shared.pending_prompt {
            Some(pending) => column![
                text(format!("CLI ждёт ввода: {}", prompt_label(pending))).size(14),
                row![
                    text_input("значение", &self.manual_input)
                        .secure(true)
                        .on_input(Message::ManualInput)
                        .on_submit(Message::ManualSend)
                        .width(Length::Fixed(280.0)),
                    button("Отправить")
                        .style(button::primary)
                        .on_press(Message::ManualSend),
                ]
                .spacing(8),
            ]
            .spacing(6)
            .into(),
            None => text(
                "Если CLI задаст вопрос, который мы не смогли распознать, поле появится здесь, \
                 а сам вопрос будет виден в консоли.",
            )
            .size(12)
            .into(),
        };

        scrollable(
            column![
                section("Аккаунт", status.into()),
                section("Вход", login.into()),
                section("Ответ CLI", prompt),
            ]
            .spacing(16),
        )
        .height(Length::Fill)
        .into()
    }

    fn console_view(&self) -> Element<'_, Message> {
        let runner = self.shared.runner.render();
        let connection = self.shared.state.connection.value.label();
        let age = self.shared.state.connection.age_text();

        let bar = row![
            button(if self.console_expanded { "▼" } else { "▲" })
                .style(button::secondary)
                .on_press(Message::ToggleConsole),
            text(format!("консоль · {runner} · {connection} · {age}"))
                .font(Font::MONOSPACE)
                .size(13)
                .width(Length::Fill),
            button("Копировать всё")
                .style(button::secondary)
                .on_press(Message::CopyAll),
            button("Вниз")
                .style(button::secondary)
                .on_press(Message::ScrollToBottom),
        ]
        .spacing(8)
        .align_y(Alignment::Center);

        if !self.console_expanded {
            return container(bar).padding(6).style(container::dark).into();
        }

        let mut list = column![].spacing(4);
        if self.console.dropped_invocations > 0 {
            list = list.push(
                text(format!(
                    "… {} более ранних вызовов вытеснено из буфера",
                    self.console.dropped_invocations
                ))
                .size(12),
            );
        }
        if self.console.is_empty() {
            list = list.push(
                text("Пока ничего не запускалось. Каждая команда появится здесь дословно.")
                    .size(13),
            );
        }
        for block in &self.console.blocks {
            list = list.push(self.console_block(block));
        }

        column![
            bar,
            container(
                scrollable(list)
                    .id(scrollable::Id::new(CONSOLE_ID))
                    .on_scroll(Message::ConsoleScrolled)
                    .height(Length::Fill),
            )
            .height(Length::Fixed(260.0))
            .padding(8)
            .style(container::dark),
        ]
        .spacing(4)
        .into()
    }

    fn console_block<'a>(&'a self, block: &'a crate::console::Block) -> Element<'a, Message> {
        let header = row![
            text(format!("$ {}", block.command))
                .font(Font::MONOSPACE)
                .size(13)
                .width(Length::Fill),
            button("Копировать")
                .style(button::secondary)
                .on_press(Message::CopyInvocation(block.id)),
        ]
        .spacing(8)
        .align_y(Alignment::Center);

        let mut body = column![header].spacing(2);
        if block.lines_hidden > 0 {
            body = body.push(
                text(format!(
                    "… {} строк скрыто (лимит показа)",
                    block.lines_hidden
                ))
                .size(12),
            );
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
                .size(12)
                .style(if block.running {
                    |_theme: &Theme| text::Style {
                        color: Some(Color::from_rgb(1.0, 0.7, 0.2)),
                    }
                } else {
                    |_theme: &Theme| text::Style {
                        color: Some(Color::from_rgb(0.6, 0.6, 0.6)),
                    }
                }),
        );

        container(body)
            .padding(6)
            .style(container::bordered_box)
            .into()
    }
}

fn prompt_label(pending: &PendingPrompt) -> &'static str {
    match pending.kind {
        PromptKind::Password => "пароль",
        PromptKind::TwoFactor => "код 2FA",
        PromptKind::Unrecognised => "неопознанный запрос — смотрите консоль",
    }
}

fn section<'a>(title: &'a str, body: Element<'a, Message>) -> Element<'a, Message> {
    container(column![text(title).size(17), horizontal_rule(1.0), body,].spacing(6))
        .padding(10)
        .style(container::rounded_box)
        .into()
}

fn status_color(status: &ConnectionStatus) -> Color {
    let (r, g, b) = match status {
        ConnectionStatus::Connected(_) => (76, 175, 80),
        ConnectionStatus::Connecting => (255, 179, 0),
        ConnectionStatus::Disconnected => (158, 158, 158),
        ConnectionStatus::Error(_) => (229, 57, 53),
        ConnectionStatus::Unknown => (96, 125, 139),
    };
    Color::from_rgb(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cli_setting_choices_are_the_ones_the_cli_documents() {
        assert_eq!(
            setting_values("netshield"),
            Some(&["off", "malware-only", "malware-ads-trackers"][..])
        );
        assert_eq!(
            setting_values("kill-switch"),
            Some(&["off", "standard"][..])
        );
        assert_eq!(setting_values("ipv6"), Some(&["off", "on"][..]));
        // An unknown key offers no invented options.
        assert_eq!(setting_values("something-new"), None);
    }

    #[test]
    fn the_window_title_says_proton_vpn_and_never_protons_app_id() {
        // A reminder in code: this string is the window title, not a D-Bus name.
        assert_ne!("proton.vpn.app.gtk", "Proton VPN");
    }
}
