//! The app shell: title bar, sidebar, tab bar, activity bar and terminal,
//! plus the connection itself (connecting, health checks, reconnecting) and
//! the sidebar's live telemetry. Tabs live in `tabs/`.

use std::path::PathBuf;
use std::time::Duration;

use iced::widget::{Space, button, column, container, mouse_area, pick_list, progress_bar, responsive, row, scrollable, space, stack, text, tooltip};
use iced::window::Direction;
use iced::{Alignment, Element, Length, Padding, Size, Subscription, Task, Theme, mouse};

use crate::android::telemetry::{self, Sample};
use crate::android::transport::Ping;
use crate::journal::device_key;
use crate::platform::{self, Connect, Platform};
use crate::design::{self as ds, alpha, style};
use crate::jobs::{JobId, Jobs, Unit};
use crate::journal::Journal;
use crate::log::{Filter, Level, Line, Log};
use crate::platform::Session;
use crate::prefs::{self, Prefs};
use crate::sidebar::{self, Request, Telemetry};
use crate::tabs::{self, Ctx, Event, Tab, View};
use crate::{ui, update};

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
/// Time between telemetry samples for the sidebar card.
const TELEMETRY_EVERY: Duration = Duration::from_secs(3);
/// Time between process list samples while the Performance tab is open.
const PROCESSES_EVERY: Duration = Duration::from_secs(4);

/// Windows gets the design's own title bar and caption buttons; macOS and
/// Linux keep the native one (their window managers expect it).
const CUSTOM_CHROME: bool = cfg!(windows);
/// Width of the invisible resize borders around an undecorated window.
const RESIZE_BORDER: f32 = 5.0;
/// Zoom levels Ctrl + = / Ctrl + − step through.
const ZOOM_LEVELS: [f32; 8] = [0.8, 0.9, 1.0, 1.1, 1.2, 1.3, 1.4, 1.5];
/// Smallest window where the sidebar, tabs and terminal fit. Like every
/// window size iced handles, it is in zoomed units (iced multiplies by the
/// zoom), so the window grows with it.
const MIN_SIZE: Size = Size::new(1000.0, 620.0);

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
    files: tabs::files::State,
    remote: tabs::remote::State,
    screenshot: tabs::screenshot::State,
    history: tabs::history::State,
    apps: tabs::apps::State,
    optimize: tabs::optimize::State,
}

/// The terminal drawer under the tabs.
#[derive(Default)]
struct Terminal {
    filter: Filter,
    /// Lines frozen when the stream was paused.
    paused: Option<Vec<Line>>,
    collapsed: bool,
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
    terminal: Terminal,
    jobs: Jobs,
    prefs: Prefs,
    events: Vec<Event>,
    journal: Journal,
    window_size: Option<Size>,
    maximized: bool,
    /// Minimized (Windows reports a 0×0 size): live polling pauses.
    minimized: bool,
    telemetry: Telemetry,
    last_sample: Option<Sample>,
    sampling: bool,
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
    Files(tabs::files::Msg),
    Remote(tabs::remote::Msg),
    Screenshot(tabs::screenshot::Msg),
    History(tabs::history::Msg),
    Apps(tabs::apps::Msg),
    Optimize(tabs::optimize::Msg),
    TabSelected(Tab),
    Connected(Connect),
    Heartbeat,
    Pinged(Result<Ping, String>),
    Reconnect,
    Reconnected(Connect),
    Sample,
    Sampled(String, Result<Sample, String>),
    CpuDescribed(String, String),
    /// Redraw so job progress moves.
    Tick,
    LogCopy,
    LogClear,
    LogSave,
    LogSaveTo(Option<PathBuf>),
    LogFilter(Filter),
    LogPause,
    LogCollapse,
    Resized(Size),
    Maximized(bool),
    Window(WindowAction),
    Zoom(Zoom),
    /// Switch the interface between English and Spanish.
    Language,
    CloseRequested,
    UpdateChecked(Option<update::Release>),
    /// The connected TV's wired MAC address (for Wake-on-LAN).
    MacFound(String, Option<String>),
    UpdateInstalled(JobId, Result<update::Installed, String>),
}

#[derive(Debug, Clone, Copy)]
pub enum Zoom {
    In,
    Out,
    Reset,
}

/// Files dragged from the computer onto the window (for the Files tab).
fn file_drop(event: iced::Event, _: iced::event::Status, _: iced::window::Id) -> Option<tabs::files::Msg> {
    use iced::window::Event as W;
    match event {
        iced::Event::Window(W::FileDropped(path)) => Some(tabs::files::Msg::Dropped(path)),
        iced::Event::Window(W::FileHovered(_)) => Some(tabs::files::Msg::Hovering(true)),
        iced::Event::Window(W::FilesHoveredLeft) => Some(tabs::files::Msg::Hovering(false)),
        _ => None,
    }
}

/// Ctrl + = / Ctrl + − / Ctrl + 0, as in browsers.
fn zoom_key(event: iced::keyboard::Event) -> Option<Message> {
    use iced::keyboard::{Event, Key};
    let Event::KeyPressed { key, modifiers, .. } = event else { return None };
    if !modifiers.command() {
        return None;
    }
    match key.as_ref() {
        Key::Character("=" | "+") => Some(Message::Zoom(Zoom::In)),
        Key::Character("-" | "_") => Some(Message::Zoom(Zoom::Out)),
        Key::Character("0") => Some(Message::Zoom(Zoom::Reset)),
        _ => None,
    }
}

fn clamp_zoom(zoom: f32) -> f32 {
    if zoom.is_finite() { zoom.clamp(ZOOM_LEVELS[0], ZOOM_LEVELS[ZOOM_LEVELS.len() - 1]) } else { 1.0 }
}

#[derive(Debug, Clone, Copy)]
pub enum WindowAction {
    Drag,
    Resize(Direction),
    Minimize,
    ToggleMaximize,
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
            journal: &mut $app.journal,
            batch: &$app.sidebar.batch,
        }
    };
}

pub fn window_settings(prefs: &Prefs) -> iced::window::Settings {
    // Sizes are saved unzoomed; iced zooms whatever it is given.
    let zoom = clamp_zoom(prefs.zoom);
    let (w, h) = match (prefs.window_width, prefs.window_height) {
        (Some(w), Some(h)) => (w, h),
        _ => (1280.0, 800.0),
    };
    iced::window::Settings {
        size: Size::new((w / zoom).max(MIN_SIZE.width), (h / zoom).max(MIN_SIZE.height)),
        min_size: Some(MIN_SIZE),
        position: iced::window::Position::Centered,
        icon: iced::window::icon::from_file_data(include_bytes!("../assets/icon-256.png"), None).ok(),
        decorations: !CUSTOM_CHROME,
        // Closing goes through CloseRequested, to save preferences first.
        exit_on_close_request: false,
        #[cfg(windows)]
        platform_specific: iced::window::settings::PlatformSpecific {
            undecorated_shadow: true,
            ..Default::default()
        },
        ..Default::default()
    }
}

impl App {
    pub fn new() -> (Self, Task<Message>) {
        let prefs = prefs::load();
        update::cleanup();
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
                files: Default::default(),
                remote: Default::default(),
                screenshot: Default::default(),
                history: Default::default(),
                apps: Default::default(),
                optimize: Default::default(),
            },
            tab: Tab::Info,
            pending_tab: prefs.last_tab.as_deref().and_then(Tab::from_name),
            log: Log::default(),
            terminal: Terminal::default(),
            jobs: Jobs::default(),
            prefs,
            events: Vec::new(),
            journal: Journal::load(),
            window_size: None,
            maximized: false,
            minimized: false,
            telemetry: Telemetry::default(),
            last_sample: None,
            sampling: false,
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
        ds::theme()
    }

    /// The user's zoom; iced multiplies it with the display's own scale.
    pub fn scale_factor(&self) -> f32 {
        clamp_zoom(self.prefs.zoom)
    }

    pub fn subscription(&self) -> Subscription<Message> {
        let mut subs = vec![
            iced::window::resize_events().map(|(_, size)| Message::Resized(size)),
            iced::window::close_requests().map(|_| Message::CloseRequested),
            iced::keyboard::listen().filter_map(zoom_key),
        ];
        if self.session.is_some() {
            subs.push(match self.link {
                Link::Healthy => iced::time::every(HEARTBEAT).map(|_| Message::Heartbeat),
                Link::Reconnecting => iced::time::every(RECONNECT_EVERY).map(|_| Message::Reconnect),
            });
            // Live views poll the TV only while someone can see them.
            if self.link == Link::Healthy && !self.minimized {
                subs.push(iced::time::every(TELEMETRY_EVERY).map(|_| Message::Sample));
            }
            // While the Remote tab is open, the keyboard drives the TV. Only
            // keys no widget used arrive here, so typing in a field is safe.
            if self.tab == Tab::Remote {
                subs.push(iced::keyboard::listen().filter_map(tabs::remote::key_for).map(Message::Remote));
            }
            if self.tab == Tab::Packages {
                subs.push(iced::keyboard::listen().filter_map(tabs::packages::key_for).map(Message::Packages));
            }
            // Files dragged onto the window are uploaded into the open folder.
            if self.tab == Tab::Files && self.tabs.files.accepts_drops() {
                subs.push(iced::event::listen_with(file_drop).map(Message::Files));
            }
            if self.tab == Tab::Tools && self.tabs.tools.log_live() && !self.minimized {
                subs.push(iced::time::every(Duration::from_secs(3)).map(|_| Message::Tools(tabs::tools::Msg::LogTick)));
            }
            if self.tab == Tab::Performance && self.tabs.performance.live() && !self.minimized {
                subs.push(iced::time::every(PROCESSES_EVERY).map(|_| Message::Performance(tabs::performance::Msg::ProcessTick)));
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
        match Platform::for_serial(&serial) {
            Platform::AndroidTv => self.log.cmd(format!("adb connect {serial}")),
            other => self.log.push(format!(
                "Connecting to {serial} ({}). If the TV asks, choose Allow on its screen.",
                other.name()
            )),
        }
        self.sidebar.connecting = true;
        self.sidebar.hint = None;
        self.connecting_to = Some(serial.clone());
        Task::perform(platform::connect(serial), Message::Connected)
    }

    fn disconnect(&mut self) {
        if let Some(session) = self.session.take() {
            session.close();
            self.log.push(format!("Disconnected from {}.", session.label()));
        }
        self.link = Link::Healthy;
        self.reconnecting = false;
        // A connection still being set up is abandoned too.
        self.connecting_to = None;
        self.sidebar.connecting = false;
        self.jobs.clear();
        self.telemetry = Telemetry::default();
        self.last_sample = None;
        self.sampling = false;
        let t = &mut self.tabs;
        t.info.reset();
        t.packages.reset();
        t.performance.reset();
        t.install.reset();
        t.settings.reset();
        t.tools.reset();
        t.media.reset();
        t.files.reset();
        t.screenshot.reset();
        t.history.reset();
        t.apps.reset();
        t.optimize.reset();
        self.tab = Tab::Info;
    }

    fn select(&mut self, tab: Tab) -> Task<Message> {
        self.tab = tab;
        self.prefs.last_tab = Some(tab.name().to_owned());
        match tab {
            Tab::Packages => self.tabs.packages.on_open(&mut ctx!(self)).map(Message::Packages),
            Tab::Performance => self.tabs.performance.on_open(&mut ctx!(self)).map(Message::Performance),
            Tab::Settings => self.tabs.settings.on_open(&mut ctx!(self)).map(Message::Settings),
            Tab::Files => self.tabs.files.on_open(&mut ctx!(self)).map(Message::Files),
            Tab::Apps => self.tabs.apps.on_open(&mut ctx!(self)).map(Message::Apps),
            Tab::Optimize => self.tabs.optimize.on_open(&mut ctx!(self)).map(Message::Optimize),
            Tab::Install => self.tabs.install.on_open(&mut ctx!(self)).map(Message::Install),
            _ => Task::none(),
        }
    }

    fn window(action: WindowAction) -> Task<Message> {
        iced::window::latest().and_then(move |id| match action {
            WindowAction::Drag => iced::window::drag(id),
            WindowAction::Resize(direction) => iced::window::drag_resize(id, direction),
            WindowAction::Minimize => iced::window::minimize(id, true),
            WindowAction::ToggleMaximize => iced::window::toggle_maximize(id),
        })
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
                    Some(Request::SaveLog) => Task::done(Message::LogSave),
                    Some(Request::Update) => match self.sidebar.update_available.as_ref().and_then(|r| r.asset.clone()) {
                        Some(asset) => {
                            self.sidebar.update_state = sidebar::UpdateState::Downloading;
                            let version = self.sidebar.update_available.as_ref().map(|r| r.version.clone()).unwrap_or_default();
                            self.log.push(format!("Downloading Smart TV Toolkit {version}…"));
                            let (id, progress) = self.jobs.start(format!("Updating to {version}"), Unit::Bytes);
                            Task::perform(update::install(asset, progress), move |r| Message::UpdateInstalled(id, r))
                        }
                        None => Task::none(),
                    },
                    Some(Request::Restart) => {
                        self.prefs.subnet = Some(self.sidebar.subnet.clone());
                        self.save_prefs();
                        match update::relaunch() {
                            Ok(()) => iced::exit(),
                            Err(e) => {
                                self.log.push(format!("Couldn't restart: {e}. Close and open the app yourself."));
                                Task::none()
                            }
                        }
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
            Message::Files(m) => self.tabs.files.update(m, &mut ctx!(self)).map(Message::Files),
            Message::Remote(m) => self.tabs.remote.update(m, &mut ctx!(self)).map(Message::Remote),
            Message::Screenshot(m) => self.tabs.screenshot.update(m, &mut ctx!(self)).map(Message::Screenshot),
            Message::History(m) => self.tabs.history.update(m, &mut ctx!(self)).map(Message::History),
            Message::Apps(m) => self.tabs.apps.update(m, &mut ctx!(self)).map(Message::Apps),
            Message::Optimize(m) => self.tabs.optimize.update(m, &mut ctx!(self)).map(Message::Optimize),
            Message::TabSelected(tab) => self.select(tab),
            Message::Connected(result) => {
                // A slower answer for a TV that's no longer the one asked for.
                // (Compared by IP: LG may answer on its other port.)
                if let Connect::Connected(s) = &result
                    && self.connecting_to.as_deref().map(device_key) != Some(device_key(s.address()))
                {
                    return Task::none();
                }
                self.sidebar.connecting = false;
                self.connecting_to = None;
                match result {
                    Connect::Connected(session) => {
                        let label = session.label();
                        let serial = session.address().to_owned();
                        self.log.push(match &session {
                            Session::Android(s) => format!(
                                "Connected: {label} (Android {} · API {} · {})",
                                s.summary.android_version, s.summary.sdk, s.summary.abi
                            ),
                            other => format!("Connected: {label} ({})", other.platform().name()),
                        });
                        self.sidebar.connected(&serial, &label, &mut self.log);
                        self.prefs.last_device = Some(serial.clone());
                        self.save_prefs();
                        let describe = match &session {
                            Session::Android(s) => {
                                let (conn, mac_conn, mac_serial) = (s.conn.clone(), s.conn.clone(), serial.clone());
                                Task::batch([
                                    Task::perform(telemetry::cpu_description(conn), move |d| Message::CpuDescribed(serial.clone(), d)),
                                    Task::perform(crate::android::logs::ethernet_mac(mac_conn), move |m| {
                                        Message::MacFound(mac_serial.clone(), m)
                                    }),
                                ])
                            }
                            Session::Samsung(s) => Task::done(Message::MacFound(serial.clone(), s.info.wifi_mac.clone())),
                            Session::Lg(_) => Task::none(),
                        };
                        let android = session.android().is_some();
                        self.session = Some(session);
                        self.link = Link::Healthy;
                        self.misses = 0;
                        self.busy_checks = 0;
                        let mut tasks = vec![describe];
                        if android {
                            self.tabs.install.on_connect(&mut ctx!(self));
                            tasks.push(self.tabs.info.on_connect(&mut ctx!(self)).map(Message::Info));
                            tasks.push(self.tabs.history.on_connect(&mut ctx!(self)).map(Message::History));
                            tasks.push(Task::done(Message::Sample));
                        }
                        tasks.push(match self.pending_tab.take() {
                            Some(tab) if self.session.as_ref().is_some_and(|s| s.supports(tab)) => self.select(tab),
                            _ => Task::none(),
                        });
                        Task::batch(tasks)
                    }
                    Connect::NeedsApproval(hint) => {
                        self.log.push("The TV is waiting for approval on its screen.");
                        self.sidebar.hint = Some(hint);
                        Task::none()
                    }
                    Connect::Failed(out) => {
                        self.log.push(format!("Connection failed: {out}"));
                        self.sidebar.hint = Some(
                            "Check the IP. Android TVs need network debugging on in Developer options; \
                             Samsung (port 8002) and LG (port 3001) TVs must be switched on."
                                .into(),
                        );
                        Task::none()
                    }
                }
            }
            Message::Heartbeat => match &self.session {
                Some(_) if self.link != Link::Healthy => Task::none(),
                Some(Session::Android(s)) => {
                    let conn = s.conn.clone();
                    Task::perform(async move { conn.ping(PING_TIMEOUT).await.map_err(|e| e.to_string()) }, Message::Pinged)
                }
                Some(Session::Samsung(s)) => {
                    Task::perform(crate::samsung::ping(s.clone()), |r| Message::Pinged(r.map(|()| Ping::Alive)))
                }
                Some(Session::Lg(s)) => Task::perform(crate::lg::ping(s.clone()), |r| Message::Pinged(r.map(|()| Ping::Alive))),
                None => Task::none(),
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
                    Task::perform(platform::connect(s.address().to_owned()), Message::Reconnected)
                }
                _ => Task::none(),
            },
            Message::Reconnected(result) => {
                self.reconnecting = false;
                // Only if we're still trying to get back to this same TV.
                let current = self.session.as_ref().map(Session::address);
                if let Connect::Connected(s) = result
                    && self.link == Link::Reconnecting
                    && current.map(device_key) == Some(device_key(s.address()))
                {
                    self.log.push(format!("Reconnected to {}.", s.label()));
                    // Tabs keep their state; they reach the TV through the new connection.
                    self.session = Some(s);
                    self.link = Link::Healthy;
                    self.last_sample = None;
                }
                Task::none()
            }
            Message::Sample => match self.session.as_ref().and_then(Session::android) {
                Some(s) if self.link == Link::Healthy && !self.sampling => {
                    self.sampling = true;
                    let (serial, conn) = (s.serial.clone(), s.conn.clone());
                    Task::perform(telemetry::sample(conn), move |r| Message::Sampled(serial.clone(), r))
                }
                _ => Task::none(),
            },
            Message::Sampled(serial, result) => {
                self.sampling = false;
                // Only numbers from the TV that's still connected.
                if self.session.as_ref().map(Session::address) == Some(serial.as_str())
                    && let Ok(sample) = result
                {
                    let t = &mut self.telemetry;
                    if let Some(previous) = &self.last_sample {
                        t.cpu = telemetry::cpu_load(previous, &sample).or(t.cpu);
                    }
                    t.ram_total = sample.mem_total;
                    t.ram_used = sample.mem_total.saturating_sub(sample.mem_available);
                    t.latency_ms = Some(sample.latency.as_secs_f32() * 1000.0);
                    self.last_sample = Some(sample);
                }
                Task::none()
            }
            Message::CpuDescribed(serial, description) => {
                if self.session.as_ref().map(Session::address) == Some(serial.as_str()) {
                    self.telemetry.cpu_description = description;
                }
                Task::none()
            }
            Message::Tick => Task::none(),
            Message::LogCopy => {
                let text = self.log.to_text();
                self.log.push("Log copied to the clipboard.");
                iced::clipboard::write(text)
            }
            Message::LogClear => {
                self.log.clear();
                if self.terminal.paused.is_some() {
                    self.terminal.paused = Some(Vec::new());
                }
                Task::none()
            }
            Message::LogSave => Task::perform(
                ui::save_file("Export log", ui::timestamped("smart-tv-toolkit-log", "txt"), ("Text", &["txt"])),
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
            Message::LogFilter(filter) => {
                self.terminal.filter = filter;
                Task::none()
            }
            Message::LogPause => {
                self.terminal.paused = match self.terminal.paused {
                    Some(_) => None,
                    None => Some(self.log.lines().cloned().collect()),
                };
                Task::none()
            }
            Message::LogCollapse => {
                self.terminal.collapsed = !self.terminal.collapsed;
                Task::none()
            }
            Message::Resized(size) => {
                self.minimized = size.width < 1.0 || size.height < 1.0;
                if self.minimized {
                    return Task::none();
                }
                self.window_size = Some(size);
                if CUSTOM_CHROME {
                    iced::window::latest().and_then(iced::window::is_maximized).map(Message::Maximized)
                } else {
                    Task::none()
                }
            }
            Message::Maximized(maximized) => {
                self.maximized = maximized;
                Task::none()
            }
            Message::Window(action) => Self::window(action),
            Message::Language => {
                let next = match crate::i18n::current() {
                    crate::i18n::Lang::En => crate::i18n::Lang::Es,
                    crate::i18n::Lang::Es => crate::i18n::Lang::En,
                };
                crate::i18n::set(next);
                self.prefs.language = Some(next.code().to_owned());
                self.save_prefs();
                Task::none()
            }
            Message::Zoom(step) => {
                let current = clamp_zoom(self.prefs.zoom);
                let zoom = match step {
                    Zoom::In => ZOOM_LEVELS.iter().copied().find(|z| *z > current + 0.01).unwrap_or(current),
                    Zoom::Out => ZOOM_LEVELS.iter().rev().copied().find(|z| *z < current - 0.01).unwrap_or(current),
                    Zoom::Reset => 1.0,
                };
                // No resize event follows a zoom change, so convert the last size here.
                if let Some(size) = &mut self.window_size {
                    *size = Size::new(size.width * current / zoom, size.height * current / zoom);
                }
                self.prefs.zoom = zoom;
                self.save_prefs();
                // Re-apply the minimum so it scales with the new zoom.
                iced::window::latest().and_then(|id| iced::window::set_min_size(id, Some(MIN_SIZE)))
            }
            Message::CloseRequested => {
                // Don't remember a maximized size as the normal one.
                // Saved unzoomed, so a later zoom change doesn't resize the window.
                if let Some(size) = self.window_size.filter(|_| !self.maximized) {
                    let zoom = clamp_zoom(self.prefs.zoom);
                    self.prefs.window_width = Some(size.width * zoom);
                    self.prefs.window_height = Some(size.height * zoom);
                }
                self.prefs.subnet = Some(self.sidebar.subnet.clone());
                self.save_prefs();
                iced::exit()
            }
            Message::UpdateChecked(found) => {
                if let Some(release) = &found {
                    self.log.push(format!("Smart TV Toolkit {} is available.", release.version));
                }
                self.sidebar.update_available = found;
                self.sidebar.update_state = sidebar::UpdateState::Available;
                Task::none()
            }
            Message::MacFound(serial, mac) => {
                if let Some(mac) = mac {
                    self.sidebar.remember_mac(&serial, &mac, &mut self.log);
                }
                Task::none()
            }
            Message::UpdateInstalled(id, result) => {
                self.jobs.finish(id);
                match result {
                    Ok(update::Installed::Restart) => {
                        self.log.push("Update installed. Restart to use it.");
                        self.sidebar.update_state = sidebar::UpdateState::Ready;
                    }
                    Ok(update::Installed::Opened(path)) => {
                        self.log.push(format!("Opened {}: drag the app into Applications to finish.", path.display()));
                        self.sidebar.update_state = sidebar::UpdateState::Available;
                    }
                    Err(e) => {
                        self.log.push(format!("Update failed: {e}"));
                        self.sidebar.update_state = sidebar::UpdateState::Available;
                    }
                }
                Task::none()
            }
        };
        self.process_events();
        task
    }

    pub fn view(&self) -> Element<'_, Message> {
        let cx = sidebar::Context {
            session: self.session.as_ref(),
            reconnecting: self.link == Link::Reconnecting,
            telemetry: &self.telemetry,
            log_lines: self.log.len(),
        };
        let mut main = column![self.tab_bar(), container(self.tab_content()).width(Length::Fill).height(Length::Fill)];
        if !self.jobs.is_empty() {
            main = main.push(self.activity_bar());
        }
        main = main.push(self.terminal());
        let body = row![
            self.sidebar.view(cx).map(Message::Sidebar),
            container(main).width(Length::Fill).height(Length::Fill).style(style::fill(ds::SOLID)),
        ];
        let window = container(column![self.title_bar(), body]).width(Length::Fill).height(Length::Fill).style(style::fill(ds::SIDEBAR));
        if CUSTOM_CHROME && !self.maximized { stack![window, resize_borders()].into() } else { window.into() }
    }

    /// The 36px title bar: app, device breadcrumb, ADB badge, caption buttons.
    fn title_bar(&self) -> Element<'_, Message> {
        let device = match &self.session {
            Some(s) => s.label(),
            None => "No device".into(),
        };
        let identity = row![
            ds::icon_any(ds::Icon::TvMinimal, 16.0, ds::ACCENT),
            ds::medium("Smart TV Toolkit", ds::TEXT_BASE, ds::TEXT),
            ds::label("/", ds::TEXT_BASE, ds::TEXT_TERTIARY),
            ds::label(device, ds::TEXT_BASE, ds::TEXT_SECONDARY),
            container(ds::mono(concat!("ADB native · v", env!("CARGO_PKG_VERSION")), ds::TEXT_XS, ds::TEXT_SECONDARY))
                .padding([1, 6])
                .style(style::panel(ds::CARD, ds::BORDER, ds::RADIUS)),
        ]
        .spacing(10)
        .align_y(Alignment::Center);
        let grip = container(ds::label("• • •", ds::TEXT_XS, alpha(ds::TEXT_TERTIARY, 0.4))).center_x(Length::Fill);
        let zoom = clamp_zoom(self.prefs.zoom);
        let zoom_control = row![
            hint(ds::square_button(ds::Icon::ZoomOut, ds::TEXT_SECONDARY, 24.0, style::ghost, (zoom > ZOOM_LEVELS[0]).then_some(Message::Zoom(Zoom::Out))), "Zoom out (Ctrl + −)"),
            hint(
                button(ds::mono(format!("{:.0}%", zoom * 100.0), ds::TEXT_XS, ds::TEXT_SECONDARY))
                    .padding([2, 4])
                    .style(style::ghost)
                    .on_press(Message::Zoom(Zoom::Reset)),
                "Reset zoom (Ctrl + 0)",
            ),
            hint(ds::square_button(ds::Icon::ZoomIn, ds::TEXT_SECONDARY, 24.0, style::ghost, (zoom < ZOOM_LEVELS[ZOOM_LEVELS.len() - 1]).then_some(Message::Zoom(Zoom::In))), "Zoom in (Ctrl + =)"),
        ]
        .spacing(2)
        .align_y(Alignment::Center);
        let language = hint(
            button(ds::mono(crate::i18n::current().code().to_uppercase(), ds::TEXT_XS, ds::TEXT_SECONDARY))
                .padding([2, 6])
                .style(style::chip)
                .on_press(Message::Language),
            "Interface language",
        );
        let mut bar = row![identity, grip, language, Space::new().width(6), zoom_control, Space::new().width(8)].align_y(Alignment::Center).height(Length::Fill).padding(Padding::default().left(12));
        if CUSTOM_CHROME {
            let caption = |glyph, size, close, msg| {
                button(container(ds::icon_any(glyph, size, ds::TEXT_SECONDARY)).center(Length::Fill))
                    .width(44)
                    .height(Length::Fill)
                    .padding(0)
                    .style(style::caption(close))
                    .on_press(msg)
            };
            bar = bar.push(row![
                caption(ds::Icon::Minus, 12.0, false, Message::Window(WindowAction::Minimize)),
                caption(
                    if self.maximized { ds::Icon::Copy } else { ds::Icon::Square },
                    12.0,
                    false,
                    Message::Window(WindowAction::ToggleMaximize)
                ),
                caption(ds::Icon::X, 14.0, true, Message::CloseRequested),
            ]);
        } else {
            bar = bar.push(Space::new().width(12));
        }
        let bar = container(column![bar.height(Length::Fill), ds::hline(ds::BORDER)])
            .height(ds::TITLE_BAR_HEIGHT)
            .width(Length::Fill)
            .style(style::fill(ds::BG));
        if CUSTOM_CHROME {
            mouse_area(bar)
                .on_press(Message::Window(WindowAction::Drag))
                .on_double_click(Message::Window(WindowAction::ToggleMaximize))
                .into()
        } else {
            bar.into()
        }
    }

    fn tab_bar(&self) -> Element<'_, Message> {
        container(column![responsive(move |size| self.tab_strip(size.width)), ds::hline(ds::BORDER)])
            .height(ds::TAB_BAR_HEIGHT)
            .width(Length::Fill)
            .style(style::fill(ds::HEADER))
            .into()
    }

    /// The tabs, sized to `width`: every caption when they fit, else only
    /// the open tab's caption, else icons alone (captions become tooltips).
    fn tab_strip(&self, width: f32) -> Element<'_, Message> {
        let tabs: Vec<Tab> = Tab::ALL.into_iter().filter(|t| self.session.as_ref().is_none_or(|s| s.supports(*t))).collect();
        let badge = self.tabs.packages.count().map(|n| n.to_string().len() as f32 * 7.0 + 22.0);
        // Rough caption widths at 12px Inter; generous so nothing clips.
        let full_width = |tab: Tab| {
            48.0 + tab.title().chars().count() as f32 * 7.0 + if tab == Tab::Packages { badge.unwrap_or(0.0) } else { 0.0 }
        };
        let gaps = 16.0 + 4.0 * tabs.len() as f32;
        let all_captions = gaps + tabs.iter().map(|t| full_width(*t)).sum::<f32>();
        let open_caption = gaps + 32.0 * (tabs.len() - 1) as f32 + full_width(self.tab);
        let caption_shown = |tab: Tab| width >= all_captions || (tab == self.tab && width >= open_caption);
        let items = tabs.iter().map(|&tab| {
            // Only Overview works without a device, as in the original app.
            let enabled = tab == Tab::Info || self.session.is_some();
            let active = tab == self.tab;
            let (icon_color, text_color) = match (active, enabled) {
                (true, _) => (ds::ACCENT, ds::TEXT),
                (false, true) => (ds::TEXT_TERTIARY, ds::TEXT_SECONDARY),
                (false, false) => (alpha(ds::TEXT_TERTIARY, 0.6), ds::TEXT_TERTIARY),
            };
            let glyph = ds::icon_any(tab.icon(), 14.0, icon_color);
            let on_press = enabled.then_some(Message::TabSelected(tab));
            if !caption_shown(tab) {
                let b = button(container(glyph).center(Length::Fill)).width(32).height(32).padding(0).style(style::tab(active)).on_press_maybe(on_press);
                return hint(b, tab.title());
            }
            let caption = if active { ds::medium(tab.title(), ds::TEXT_BASE, text_color) } else { ds::label(tab.title(), ds::TEXT_BASE, text_color) };
            let mut content = row![glyph, caption].spacing(6).align_y(Alignment::Center);
            if tab == Tab::Packages
                && let Some(n) = self.tabs.packages.count()
            {
                content = content.push(Space::new().width(2)).push(ds::count(n, if active { ds::WELL } else { ds::BADGE }));
            }
            button(container(content).center_y(Length::Fill)).height(32).padding([0, 12]).style(style::tab(active)).on_press_maybe(on_press).into()
        });
        container(row(items).spacing(4).align_y(Alignment::Center)).padding([0, 8]).center_y(Length::Fill).width(Length::Fill).clip(true).into()
    }

    fn tab_content(&self) -> Element<'_, Message> {
        let v = View { session: self.session.as_ref(), journal: &self.journal, batch: &self.sidebar.batch };
        let t = &self.tabs;
        match self.tab {
            Tab::Info => t.info.view(&v).map(Message::Info),
            Tab::Packages => t.packages.view(&v).map(Message::Packages),
            Tab::Performance => t.performance.view(&v).map(Message::Performance),
            Tab::Install => t.install.view(&v).map(Message::Install),
            Tab::Settings => t.settings.view(&v).map(Message::Settings),
            Tab::Tools => t.tools.view(&v).map(Message::Tools),
            Tab::Media => t.media.view(&v).map(Message::Media),
            Tab::Files => t.files.view(&v).map(Message::Files),
            Tab::Remote => t.remote.view(&v).map(Message::Remote),
            Tab::Screenshot => t.screenshot.view(&v).map(Message::Screenshot),
            Tab::History => t.history.view(&v).map(Message::History),
            Tab::Apps => t.apps.view(&v).map(Message::Apps),
            Tab::Optimize => t.optimize.view(&v).map(Message::Optimize),
        }
    }

    /// What's running on the TV right now, with progress where it's known.
    fn activity_bar(&self) -> Element<'_, Message> {
        let rows = self.jobs.active.iter().map(|job| {
            let (fraction, detail) = job.progress.snapshot();
            let secs = job.started.elapsed().as_secs();
            let bar: Element<'_, Message> = match fraction {
                Some(f) => progress_bar(0.0..=1.0, f).length(220).girth(6).style(style::gauge(ds::ACCENT)).into(),
                None => ds::mono(format!("working… {secs}s"), ds::TEXT_SM, ds::TEXT_TERTIARY).width(220).into(),
            };
            row![
                ds::icon_any(ds::Icon::Loader, 12.0, ds::ACCENT),
                ds::label(&job.title, ds::TEXT_BASE, ds::TEXT).width(Length::FillPortion(2)),
                bar,
                ds::label(detail, ds::TEXT_SM, ds::TEXT_SECONDARY).width(Length::FillPortion(3)),
            ]
            .spacing(10)
            .align_y(Alignment::Center)
            .into()
        });
        container(column![ds::hline(ds::BORDER), container(column(rows).spacing(4)).padding([8, 12])])
            .width(Length::Fill)
            .style(style::fill(ds::GROUP))
            .into()
    }

    /// The terminal drawer: channel-tagged log with pause, filter and export.
    fn terminal(&self) -> Element<'_, Message> {
        let t = &self.terminal;
        let target = match &self.session {
            Some(s) => format!("adb@{}", s.address()),
            None => "adb@—".to_owned(),
        };
        let stream = move || button(
            row![
                ds::icon_any(if t.paused.is_some() { ds::Icon::Pause } else { ds::Icon::Play }, 10.0, if t.paused.is_some() { ds::AMBER } else { ds::GREEN }),
                ds::label(if t.paused.is_some() { "Stream: Paused" } else { "Stream: Live" }, ds::TEXT_XS, ds::TEXT),
            ]
            .spacing(4)
            .align_y(Alignment::Center),
        )
        .padding([2, 6])
        .style(style::chip)
        .on_press(Message::LogPause);
        let filter = move || {
            pick_list(Filter::ALL, Some(t.filter), Message::LogFilter)
                .text_size(ds::TEXT_XS)
                .font(ds::MONO)
                .padding([2, 6])
                .style(style::pick_list)
                .menu_style(style::menu)
        };
        let header = responsive(move |size| {
            // Narrow: no address, and the actions become icons.
            let roomy = size.width >= 860.0;
            let action = |glyph, caption: &'static str, msg: Message| -> Element<'_, Message> {
                let mut content = row![ds::icon_any(glyph, 12.0, ds::TEXT_TERTIARY)].spacing(4).align_y(Alignment::Center);
                if roomy {
                    content = content.push(text(crate::i18n::tr(caption)).size(ds::TEXT_SM));
                }
                let b = button(content).padding(0).style(style::link(ds::TEXT)).on_press(msg);
                if roomy { b.into() } else { hint(b, caption) }
            };
            let mut left = row![ds::icon_any(ds::Icon::Terminal, 14.0, ds::TEXT_SECONDARY), ds::medium("Terminal Output", ds::TEXT_SM, ds::TEXT)]
                .spacing(10)
                .align_y(Alignment::Center);
            if roomy {
                left = left.push(ds::label("|", ds::TEXT_SM, ds::TEXT_TERTIARY)).push(ds::mono(target.clone(), ds::TEXT_XS, ds::TEXT_SECONDARY));
            }
            let row = row![
            left,
            Space::new().width(4),
            stream(),
            row![ds::label("Filter:", ds::TEXT_XS, ds::TEXT_TERTIARY), filter()].spacing(4).align_y(Alignment::Center),
            space::horizontal(),
            row![
                action(ds::Icon::Copy, "Copy", Message::LogCopy),
                action(ds::Icon::FileDown, "Export", Message::LogSave),
                action(ds::Icon::Trash, "Clear", Message::LogClear),
                ds::vline(12.0),
                button(ds::icon_any(if t.collapsed { ds::Icon::ChevronUp } else { ds::Icon::ChevronDown }, 14.0, ds::TEXT_SECONDARY))
                    .padding(2)
                    .style(style::ghost)
                    .on_press(Message::LogCollapse),
            ]
            .spacing(12)
            .align_y(Alignment::Center),
        ]
        .spacing(10)
        .align_y(Alignment::Center);
            container(row).center_y(Length::Fill).into()
        });
        let header = container(column![
            container(header).padding([0, 12]).center_y(Length::Fill),
            ds::hline(ds::BORDER),
        ])
        .height(32)
        .style(style::fill(ds::WELL));

        let mut drawer = column![ds::hline(ds::BORDER), header];
        if !t.collapsed {
            let lines: Box<dyn Iterator<Item = &Line>> = match &t.paused {
                Some(frozen) => Box::new(frozen.iter()),
                None => Box::new(self.log.lines()),
            };
            let rows = column(lines.filter(|l| t.filter.shows(l.level)).map(terminal_line)).spacing(2);
            drawer = drawer.push(
                container(
                    scrollable(container(rows).padding(12).width(Length::Fill))
                        .anchor_bottom()
                        .direction(ds::thin_scrollbar())
                        .style(style::scrollbar)
                        .height(Length::Fill),
                )
                .height(ds::TERMINAL_HEIGHT - 32.0)
                .style(style::fill(ds::TERMINAL)),
            );
        }
        container(drawer).width(Length::Fill).into()
    }
}

/// A short tooltip below `content`.
fn hint<'a>(content: impl Into<Element<'a, Message>>, text: &'a str) -> Element<'a, Message> {
    tooltip(content, container(ds::label(text, ds::TEXT_SM, ds::TEXT)).padding([4, 8]).style(style::flyout), tooltip::Position::Bottom)
        .gap(4)
        .into()
}

/// One terminal line: time, channel tag, prompt and message.
fn terminal_line(l: &Line) -> Element<'_, Message> {
    let (tag_color, text_color) = match l.level {
        Level::Adb => (ds::CYAN, ds::TEXT_SECONDARY),
        Level::Out => (ds::GREEN, ds::TEXT_SECONDARY),
        Level::Ok => (ds::GREEN, ds::GREEN),
        Level::Warn => (ds::AMBER, ds::AMBER),
        Level::Error => (ds::RED, ds::RED),
        Level::Select => (ds::PURPLE, ds::PURPLE_SOFT),
    };
    let prompt = if l.level == Level::Out { ds::TEXT_TERTIARY } else { ds::ACCENT };
    row![
        ds::mono(format!("[{}]", l.time), ds::TEXT_SM, ds::TEXT_TERTIARY),
        ds::mono(l.level.tag().to_uppercase(), ds::TEXT_XS, tag_color).font(ds::MONO_SEMIBOLD).line_height(1.6),
        ds::mono(">", ds::TEXT_SM, prompt).font(ds::MONO_SEMIBOLD),
        ds::mono(&l.text, ds::TEXT_SM, text_color).width(Length::Fill),
    ]
    .spacing(8)
    .into()
}

/// Invisible borders that resize the undecorated window when dragged.
fn resize_borders<'a>() -> Element<'a, Message> {
    let edge = |direction, interaction, width: Length, height: Length| -> Element<'a, Message> {
        mouse_area(Space::new().width(width).height(height))
            .interaction(interaction)
            .on_press(Message::Window(WindowAction::Resize(direction)))
            .into()
    };
    let b = Length::Fixed(RESIZE_BORDER);
    let fill = Length::Fill;
    column![
        row![
            edge(Direction::NorthWest, mouse::Interaction::ResizingDiagonallyDown, b, b),
            edge(Direction::North, mouse::Interaction::ResizingVertically, fill, b),
            edge(Direction::NorthEast, mouse::Interaction::ResizingDiagonallyUp, b, b),
        ],
        row![
            edge(Direction::West, mouse::Interaction::ResizingHorizontally, b, fill),
            Space::new().width(fill).height(fill),
            edge(Direction::East, mouse::Interaction::ResizingHorizontally, b, fill),
        ]
        .height(fill),
        row![
            edge(Direction::SouthWest, mouse::Interaction::ResizingDiagonallyUp, b, b),
            edge(Direction::South, mouse::Interaction::ResizingVertically, fill, b),
            edge(Direction::SouthEast, mouse::Interaction::ResizingDiagonallyDown, b, b),
        ],
    ]
    .into()
}
