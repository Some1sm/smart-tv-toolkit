//! The app shell: sidebar, tab bar, activity bar and log, plus the connection
//! itself (connecting, health checks, reconnecting). Tabs live in `tabs/`.

use std::path::PathBuf;
use std::time::Duration;

use iced::widget::{button, column, container, progress_bar, row, scrollable, space, text};
use iced::{Alignment, Element, Fill, Font, Length, Size, Subscription, Task, Theme};

use crate::android::transport::Ping;
use crate::android::{self, ConnectResult};
use crate::jobs::Jobs;
use crate::log::Log;
use crate::platform::Session;
use crate::prefs::{self, Prefs};
use crate::sidebar::{self, Request, Status};
use crate::tabs::{self, Ctx, Event, Tab, View};
use crate::{theme, ui, update};

/// How often the connection is checked while connected.
const HEARTBEAT: Duration = Duration::from_secs(10);
/// How long a health check may take before it counts as a miss.
const PING_TIMEOUT: Duration = Duration::from_secs(5);
/// Misses in a row before the connection is treated as lost.
const MAX_MISSES: u8 = 2;
/// Checks in a row that find a command still running before that's treated
/// as a hung connection (quick commands never take this long).
const MAX_BUSY: u8 = 4;
/// Time between reconnect attempts.
const RECONNECT_EVERY: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Link {
    Healthy,
    Reconnecting,
}

struct Tabs {
    info: tabs::info::State,
    packages: tabs::packages::State,
    performance: tabs::performance::State,
    install: tabs::install::State,
    settings: tabs::settings::State,
    tools: tabs::tools::State,
    media: tabs::media::State,
    remote: tabs::remote::State,
    screenshot: tabs::screenshot::State,
}

pub struct App {
    session: Option<Session>,
    link: Link,
    misses: u8,
    busy_checks: u8,
    reconnecting: bool,
    /// The TV the latest Connect was for; answers for any other are stale.
    connecting_to: Option<String>,
    sidebar: sidebar::State,
    tabs: Tabs,
    tab: Tab,
    /// Tab to reopen once the remembered TV reconnects on launch.
    pending_tab: Option<Tab>,
    log: Log,
    jobs: Jobs,
    prefs: Prefs,
    events: Vec<Event>,
    window_size: Option<Size>,
}

#[derive(Debug, Clone)]
pub enum Message {
    Sidebar(sidebar::Msg),
    Info(tabs::info::Msg),
    Packages(tabs::packages::Msg),
    Performance(tabs::performance::Msg),
    Install(tabs::install::Msg),
    Settings(tabs::settings::Msg),
    Tools(tabs::tools::Msg),
    Media(tabs::media::Msg),
    Remote(tabs::remote::Msg),
    Screenshot(tabs::screenshot::Msg),
    TabSelected(Tab),
    Connected(ConnectResult),
    Heartbeat,
    Pinged(Result<Ping, String>),
    Reconnect,
    Reconnected(ConnectResult),
    /// Redraw so job progress moves.
    Tick,
    LogCopy,
    LogClear,
    LogSave,
    LogSaveTo(Option<PathBuf>),
    Resized(Size),
    CloseRequested,
    UpdateChecked(Option<(String, String)>),
}

/// Borrow the shared pieces a tab needs, next to a mutable borrow of the tab.
macro_rules! ctx {
    ($app:ident) => {
        Ctx {
            session: $app.session.as_ref(),
            log: &mut $app.log,
            jobs: &mut $app.jobs,
            prefs: &mut $app.prefs,
            events: &mut $app.events,
        }
    };
}

pub fn window_settings(prefs: &Prefs) -> iced::window::Settings {
    let size = match (prefs.window_width, prefs.window_height) {
        (Some(w), Some(h)) if w >= 900.0 && h >= 560.0 => Size::new(w, h),
        _ => Size::new(1100.0, 700.0),
    };
    iced::window::Settings {
        size,
        // Below this the sidebar, tab bar and log no longer fit.
        min_size: Some(Size::new(900.0, 560.0)),
        position: iced::window::Position::Centered,
        icon: iced::window::icon::from_file_data(include_bytes!("../assets/icon-256.png"), None).ok(),
        // Closing goes through CloseRequested, to save preferences first.
        exit_on_close_request: false,
        ..Default::default()
    }
}

impl App {
    pub fn new() -> (Self, Task<Message>) {
        let prefs = prefs::load();
        let (media, media_task) = tabs::media::State::new(&prefs);
        let mut app = App {
            session: None,
            link: Link::Healthy,
            misses: 0,
            busy_checks: 0,
            reconnecting: false,
            connecting_to: None,
            sidebar: sidebar::State::new(prefs.subnet.clone()),
            tabs: Tabs {
                info: Default::default(),
                packages: Default::default(),
                performance: Default::default(),
                install: Default::default(),
                settings: Default::default(),
                tools: Default::default(),
                media,
                remote: Default::default(),
                screenshot: Default::default(),
            },
            tab: Tab::Info,
            pending_tab: prefs.last_tab.as_deref().and_then(Tab::from_name),
            log: Log::default(),
            jobs: Jobs::default(),
            prefs,
            events: Vec::new(),
            window_size: None,
        };
        app.tabs.install.reset();
        app.log.push(match app.sidebar.saved_count() {
            0 => "Ready. Scan your network or enter the TV's IP address.".to_owned(),
            n => format!("Ready. Checking {n} saved device(s)…"),
        });

        let mut tasks = vec![app.sidebar.startup().map(Message::Sidebar), media_task.map(Message::Media)];
        // Reconnect to the last TV.
        if app.prefs.auto_connect
            && let Some(serial) = app.prefs.last_device.clone()
        {
            app.sidebar.set_target(&serial);
            tasks.push(app.connect(serial));
        }
        // Look for a newer version once a day.
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        if app.prefs.last_update_check.as_deref() != Some(today.as_str()) {
            app.prefs.last_update_check = Some(today);
            app.save_prefs();
            tasks.push(Task::perform(update::check(), Message::UpdateChecked));
        }
        (app, Task::batch(tasks))
    }

    pub fn title(&self) -> String {
        match (&self.session, self.link) {
            (Some(s), Link::Healthy) => format!("Smart TV Toolkit — {}", s.label()),
            (Some(s), Link::Reconnecting) => format!("Smart TV Toolkit — {} (reconnecting…)", s.label()),
            (None, _) => "Smart TV Toolkit".into(),
        }
    }

    pub fn theme(&self) -> Theme {
        theme::app_theme()
    }

    pub fn subscription(&self) -> Subscription<Message> {
        let mut subs = vec![
            iced::window::resize_events().map(|(_, size)| Message::Resized(size)),
            iced::window::close_requests().map(|_| Message::CloseRequested),
        ];
        if self.session.is_some() {
            subs.push(match self.link {
                Link::Healthy => iced::time::every(HEARTBEAT).map(|_| Message::Heartbeat),
                Link::Reconnecting => iced::time::every(RECONNECT_EVERY).map(|_| Message::Reconnect),
            });
            // While the Remote tab is open, the keyboard drives the TV. Only
            // keys no widget used arrive here, so typing in a field is safe.
            if self.tab == Tab::Remote {
                subs.push(iced::keyboard::listen().filter_map(tabs::remote::key_for).map(Message::Remote));
            }
        }
        if !self.jobs.is_empty() {
            subs.push(iced::time::every(Duration::from_millis(250)).map(|_| Message::Tick));
        }
        Subscription::batch(subs)
    }

    fn save_prefs(&mut self) {
        if let Err(e) = prefs::save(&self.prefs) {
            self.log.push(format!("Could not save preferences: {e}"));
        }
    }

    /// Handle what tabs asked for while updating.
    fn process_events(&mut self) {
        for event in std::mem::take(&mut self.events) {
            match event {
                Event::Disconnect => self.disconnect(),
                Event::SavePrefs => self.save_prefs(),
            }
        }
    }

    fn connect(&mut self, serial: String) -> Task<Message> {
        // Switching TVs: drop the old connection first.
        self.disconnect();
        self.log.push(format!("Connecting to {serial}…"));
        self.sidebar.connecting = true;
        self.sidebar.hint = None;
        self.connecting_to = Some(serial.clone());
        Task::perform(android::connect(serial), Message::Connected)
    }

    fn disconnect(&mut self) {
        if let Some(session) = self.session.take() {
            self.log.push(format!("Disconnected from {}.", session.label()));
        }
        self.link = Link::Healthy;
        self.reconnecting = false;
        // A connection still being set up is abandoned too.
        self.connecting_to = None;
        self.sidebar.connecting = false;
        self.jobs.clear();
        let t = &mut self.tabs;
        t.info.reset();
        t.packages.reset();
        t.performance.reset();
        t.install.reset();
        t.settings.reset();
        t.tools.reset();
        t.media.reset();
        t.screenshot.reset();
        self.tab = Tab::Info;
    }

    fn select(&mut self, tab: Tab) -> Task<Message> {
        self.tab = tab;
        self.prefs.last_tab = Some(tab.name().to_owned());
        match tab {
            Tab::Settings => self.tabs.settings.on_open(&mut ctx!(self)).map(Message::Settings),
            Tab::Media => self.tabs.media.on_open(&mut ctx!(self)).map(Message::Media),
            _ => Task::none(),
        }
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        let task = match message {
            Message::Sidebar(msg) => {
                let (task, request) = self.sidebar.update(msg, &mut self.log);
                let extra = match request {
                    Some(Request::Connect(serial)) => self.connect(serial),
                    Some(Request::Disconnect) => {
                        self.disconnect();
                        // Don't reconnect on the next launch after an explicit disconnect.
                        self.prefs.last_device = None;
                        self.save_prefs();
                        Task::none()
                    }
                    None => Task::none(),
                };
                Task::batch([task.map(Message::Sidebar), extra])
            }
            Message::Info(m) => self.tabs.info.update(m, &mut ctx!(self)).map(Message::Info),
            Message::Packages(m) => self.tabs.packages.update(m, &mut ctx!(self)).map(Message::Packages),
            Message::Performance(m) => self.tabs.performance.update(m, &mut ctx!(self)).map(Message::Performance),
            Message::Install(m) => self.tabs.install.update(m, &mut ctx!(self)).map(Message::Install),
            Message::Settings(m) => self.tabs.settings.update(m, &mut ctx!(self)).map(Message::Settings),
            Message::Tools(m) => self.tabs.tools.update(m, &mut ctx!(self)).map(Message::Tools),
            Message::Media(m) => self.tabs.media.update(m, &mut ctx!(self)).map(Message::Media),
            Message::Remote(m) => self.tabs.remote.update(m, &mut ctx!(self)).map(Message::Remote),
            Message::Screenshot(m) => self.tabs.screenshot.update(m, &mut ctx!(self)).map(Message::Screenshot),
            Message::TabSelected(tab) => self.select(tab),
            Message::Connected(result) => {
                // A slower answer for a TV that's no longer the one asked for.
                if let ConnectResult::Connected(s) = &result
                    && self.connecting_to.as_deref() != Some(s.serial.as_str())
                {
                    return Task::none();
                }
                self.sidebar.connecting = false;
                self.connecting_to = None;
                match result {
                    ConnectResult::Connected(s) => {
                        let label = s.summary.label();
                        self.log.push(format!(
                            "Connected: {label}  |  Android {} (API {})  |  {}",
                            s.summary.android_version, s.summary.sdk, s.summary.abi
                        ));
                        self.sidebar.connected(&s.serial, &label, &mut self.log);
                        self.prefs.last_device = Some(s.serial.clone());
                        self.save_prefs();
                        self.session = Some(Session::Android(s));
                        self.link = Link::Healthy;
                        self.misses = 0;
                        self.busy_checks = 0;
                        self.tabs.install.on_connect(&mut ctx!(self));
                        let info = self.tabs.info.on_connect(&mut ctx!(self)).map(Message::Info);
                        let reopen = match self.pending_tab.take() {
                            Some(tab) if self.session.as_ref().is_some_and(|s| s.supports(tab)) => self.select(tab),
                            _ => Task::none(),
                        };
                        Task::batch([info, reopen])
                    }
                    ConnectResult::NeedsApproval => {
                        self.log.push("The TV hasn't allowed debugging from this computer yet.");
                        self.sidebar.hint =
                            Some("On the TV, choose Allow (tick \"Always allow\"), then press Connect again.".into());
                        Task::none()
                    }
                    ConnectResult::Failed(out) => {
                        self.log.push(format!("Connection failed: {out}"));
                        self.sidebar.hint =
                            Some("Check the IP, and that network debugging is on in Developer options.".into());
                        Task::none()
                    }
                }
            }
            Message::Heartbeat => match self.session.as_ref().and_then(Session::android) {
                Some(s) if self.link == Link::Healthy => {
                    let conn = s.conn.clone();
                    Task::perform(async move { conn.ping(PING_TIMEOUT).await.map_err(|e| e.to_string()) }, Message::Pinged)
                }
                _ => Task::none(),
            },
            Message::Pinged(result) => {
                match result {
                    Ok(Ping::Alive) => {
                        self.misses = 0;
                        self.busy_checks = 0;
                    }
                    Ok(Ping::Busy) => self.busy_checks += 1,
                    Err(_) => self.misses += 1,
                }
                if self.session.is_some() && (self.misses >= MAX_MISSES || self.busy_checks >= MAX_BUSY) {
                    self.link = Link::Reconnecting;
                    self.misses = 0;
                    self.busy_checks = 0;
                    let label = self.session.as_ref().map(Session::label).unwrap_or_default();
                    self.log.push(format!("Lost the connection to {label}. Reconnecting…"));
                }
                Task::none()
            }
            Message::Reconnect => match &self.session {
                Some(s) if self.link == Link::Reconnecting && !self.reconnecting => {
                    self.reconnecting = true;
                    Task::perform(android::connect(s.address().to_owned()), Message::Reconnected)
                }
                _ => Task::none(),
            },
            Message::Reconnected(result) => {
                self.reconnecting = false;
                // Only if we're still trying to get back to this same TV.
                let current = self.session.as_ref().map(Session::address);
                if let ConnectResult::Connected(s) = result
                    && self.link == Link::Reconnecting
                    && current == Some(s.serial.as_str())
                {
                    self.log.push(format!("Reconnected to {}.", s.summary.label()));
                    // Tabs keep their state; they reach the TV through the new connection.
                    self.session = Some(Session::Android(s));
                    self.link = Link::Healthy;
                }
                Task::none()
            }
            Message::Tick => Task::none(),
            Message::LogCopy => {
                self.log.push("Log copied to the clipboard.");
                iced::clipboard::write(self.log.to_text())
            }
            Message::LogClear => {
                self.log.clear();
                Task::none()
            }
            Message::LogSave => Task::perform(
                ui::save_file("Save log", ui::timestamped("smart-tv-toolkit-log", "txt"), ("Text", &["txt"])),
                Message::LogSaveTo,
            ),
            Message::LogSaveTo(None) => Task::none(),
            Message::LogSaveTo(Some(path)) => {
                let result = std::fs::write(&path, self.log.to_text());
                self.log.push(match result {
                    Ok(()) => format!("Saved the log to {}", path.display()),
                    Err(e) => format!("Could not save the log: {e}"),
                });
                Task::none()
            }
            Message::Resized(size) => {
                self.window_size = Some(size);
                Task::none()
            }
            Message::CloseRequested => {
                if let Some(size) = self.window_size {
                    self.prefs.window_width = Some(size.width);
                    self.prefs.window_height = Some(size.height);
                }
                self.prefs.subnet = Some(self.sidebar.subnet.clone());
                self.save_prefs();
                iced::exit()
            }
            Message::UpdateChecked(found) => {
                if let Some((version, _)) = &found {
                    self.log.push(format!("Smart TV Toolkit {version} is available."));
                }
                self.sidebar.update_available = found;
                Task::none()
            }
        };
        self.process_events();
        task
    }

    pub fn view(&self) -> Element<'_, Message> {
        let status = match (&self.session, self.link) {
            (Some(s), Link::Healthy) => {
                Status::Connected { label: s.label(), detail: format!("{} · {}", s.platform().name(), s.address()) }
            }
            (Some(s), Link::Reconnecting) => Status::Reconnecting { label: s.label() },
            (None, _) => Status::None,
        };
        let mut main = column![self.tab_bar(), self.tab_content()].spacing(8).padding(8);
        if !self.jobs.is_empty() {
            main = main.push(self.activity_bar());
        }
        main = main.push(self.log_panel());
        row![self.sidebar.view(status).map(Message::Sidebar), main].into()
    }

    fn tab_bar(&self) -> Element<'_, Message> {
        let tabs = Tab::ALL.iter().filter(|t| self.session.as_ref().is_none_or(|s| s.supports(**t))).map(|&tab| {
            // Only Info works without a device, as in the original app.
            let enabled = tab == Tab::Info || self.session.is_some();
            button(text(tab.name()).size(13))
                .padding([6, 12])
                .style(theme::tab_button(tab == self.tab))
                .on_press_maybe(enabled.then_some(Message::TabSelected(tab)))
                .into()
        });
        container(row(tabs).spacing(2)).padding(4).width(Fill).style(theme::tab_bar).into()
    }

    fn tab_content(&self) -> Element<'_, Message> {
        let v = View { session: self.session.as_ref() };
        let t = &self.tabs;
        let content = match self.tab {
            Tab::Info => t.info.view(&v).map(Message::Info),
            Tab::Packages => t.packages.view(&v).map(Message::Packages),
            Tab::Performance => t.performance.view(&v).map(Message::Performance),
            Tab::Install => t.install.view(&v).map(Message::Install),
            Tab::Settings => t.settings.view(&v).map(Message::Settings),
            Tab::Tools => t.tools.view(&v).map(Message::Tools),
            Tab::Media => t.media.view(&v).map(Message::Media),
            Tab::Remote => t.remote.view(&v).map(Message::Remote),
            Tab::Screenshot => t.screenshot.view(&v).map(Message::Screenshot),
        };
        container(content).width(Fill).height(Fill).into()
    }

    /// What's running on the TV right now, with progress where it's known.
    fn activity_bar(&self) -> Element<'_, Message> {
        let rows = self.jobs.active.iter().map(|job| {
            let (fraction, detail) = job.progress.snapshot();
            let secs = job.started.elapsed().as_secs();
            let bar: Element<'_, Message> = match fraction {
                Some(f) => progress_bar(0.0..=1.0, f).length(220).girth(8).style(theme::usage_bar(0.0)).into(),
                None => text(format!("working… {secs}s")).size(11).color(theme::TEXT_MUTED).width(220).into(),
            };
            row![
                text(&job.title).size(12).color(theme::TEXT).width(Length::FillPortion(2)),
                bar,
                text(detail).size(11).color(theme::TEXT_MUTED).width(Length::FillPortion(3)),
            ]
            .spacing(10)
            .align_y(Alignment::Center)
            .into()
        });
        container(column(rows).spacing(4)).padding([8, 12]).width(Fill).style(theme::card).into()
    }

    fn log_panel(&self) -> Element<'_, Message> {
        let lines = column(self.log.lines().map(|l| {
            row![
                text(&l.time).size(11).font(Font::MONOSPACE).color(theme::TEXT_DISABLED),
                text(&l.text).size(12).font(Font::MONOSPACE).color(theme::TEXT),
            ]
            .spacing(8)
            .into()
        }))
        .spacing(2);
        let small = |label, msg| button(text(label).size(11)).padding([2, 8]).style(theme::tab_button(false)).on_press(msg);
        container(
            column![
                row![
                    text("Output Log").size(12).color(theme::TEXT_MUTED),
                    space::horizontal(),
                    small("Copy", Message::LogCopy),
                    small("Save…", Message::LogSave),
                    small("Clear", Message::LogClear),
                ]
                .align_y(Alignment::Center),
                scrollable(lines).anchor_bottom().width(Fill).height(Fill),
            ]
            .spacing(6),
        )
        .padding(10)
        .height(Length::Fixed(170.0))
        .width(Fill)
        .style(theme::log_box)
        .into()
    }
}
