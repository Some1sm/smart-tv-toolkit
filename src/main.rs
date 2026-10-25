// Hide the console window in release builds on Windows.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod android;
mod config;
mod platform;
mod theme;

use std::path::PathBuf;

use iced::widget::{button, column, container, row, scrollable, space, text, text_input};
use iced::{Element, Fill, Font, Length, Task, Theme};

use android::{ConnectResult, DeviceSummary, adb};
use platform::{Device, Platform};

const PLATFORM_TOOLS_URL: &str = "https://developer.android.com/tools/releases/platform-tools";

pub fn main() -> iced::Result {
    iced::application(App::new, App::update, App::view)
        .title(App::title)
        .theme(App::theme)
        .window_size((1100.0, 700.0))
        .centered()
        .run()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Info,
    Packages,
    Performance,
    Install,
    Settings,
    Tools,
    Media,
    Remote,
    Screenshot,
}

impl Tab {
    const ALL: [Tab; 9] = [
        Tab::Info,
        Tab::Packages,
        Tab::Performance,
        Tab::Install,
        Tab::Settings,
        Tab::Tools,
        Tab::Media,
        Tab::Remote,
        Tab::Screenshot,
    ];

    fn name(self) -> &'static str {
        match self {
            Tab::Info => "Info",
            Tab::Packages => "Packages",
            Tab::Performance => "Performance",
            Tab::Install => "Install",
            Tab::Settings => "Settings",
            Tab::Tools => "Tools",
            Tab::Media => "Media",
            Tab::Remote => "Remote",
            Tab::Screenshot => "Screenshot",
        }
    }
}

/// Where the app stands with ADB.
#[derive(Debug, Clone)]
enum AdbStatus {
    Starting,
    Ready { path: PathBuf, version: String },
    Missing,
    Broken { path: PathBuf, error: String },
}

impl AdbStatus {
    /// The working adb, if any.
    fn path(&self) -> Option<&PathBuf> {
        match self {
            AdbStatus::Ready { path, .. } => Some(path),
            _ => None,
        }
    }

    /// Where the file picker should open: next to the current or failed adb.
    fn last_dir(&self) -> Option<&std::path::Path> {
        match self {
            AdbStatus::Ready { path, .. } | AdbStatus::Broken { path, .. } => path.parent(),
            _ => None,
        }
    }
}

struct App {
    adb: AdbStatus,
    ip: String,
    port: String,
    connecting: bool,
    connect_hint: Option<String>,
    device: Option<Device>,
    summary: Option<DeviceSummary>,
    tab: Tab,
    log: Vec<String>,
}

#[derive(Debug, Clone)]
enum Message {
    AdbStarted(PathBuf, Result<String, String>),
    ChangeAdb,
    AdbPicked(Option<PathBuf>),
    OpenPlatformToolsPage,
    IpChanged(String),
    PortChanged(String),
    Connect,
    Connected(ConnectResult),
    Disconnect,
    Disconnected(String),
    TabSelected(Tab),
}

impl App {
    fn new() -> (Self, Task<Message>) {
        let mut app = App {
            adb: AdbStatus::Missing,
            ip: String::new(),
            port: "5555".into(),
            connecting: false,
            connect_hint: None,
            device: None,
            summary: None,
            tab: Tab::Info,
            log: Vec::new(),
        };
        let task = match adb::find() {
            Some(path) => app.start_adb(path),
            None => {
                app.log("ADB not found. Choose its location in the sidebar.");
                Task::none()
            }
        };
        (app, task)
    }

    fn title(&self) -> String {
        match &self.device {
            Some(d) => format!("Smart TV Toolkit — {}", d.label),
            None => "Smart TV Toolkit".into(),
        }
    }

    fn theme(&self) -> Theme {
        theme::app_theme()
    }

    fn log(&mut self, line: impl Into<String>) {
        self.log.push(line.into());
    }

    fn start_adb(&mut self, path: PathBuf) -> Task<Message> {
        self.adb = AdbStatus::Starting;
        Task::perform(android::start_server(path.clone()), move |r| {
            Message::AdbStarted(path.clone(), r)
        })
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::AdbStarted(path, Ok(version)) => {
                self.log(format!("ADB ready: {version}"));
                self.log(format!("Using {}", path.display()));
                if let Err(e) = config::save_adb_path(&path) {
                    self.log(format!("Could not save the ADB location: {e}"));
                }
                self.adb = AdbStatus::Ready { path, version };
            }
            Message::AdbStarted(path, Err(error)) => {
                self.log(format!("ADB at {} failed: {error}", path.display()));
                self.adb = AdbStatus::Broken { path, error };
            }
            Message::ChangeAdb => {
                let mut dialog = rfd::AsyncFileDialog::new().set_title("Locate adb");
                if cfg!(windows) {
                    dialog = dialog.add_filter("ADB executable", &["exe"]);
                }
                if let Some(dir) = self.adb.last_dir() {
                    dialog = dialog.set_directory(dir);
                }
                return Task::perform(
                    async move { dialog.pick_file().await.map(|f| f.path().to_path_buf()) },
                    Message::AdbPicked,
                );
            }
            Message::AdbPicked(None) => {}
            Message::AdbPicked(Some(picked)) => match adb::resolve(&picked) {
                Some(path) => return self.start_adb(path),
                None => self.log(format!("No {} at {}", adb::ADB_NAME, picked.display())),
            },
            Message::OpenPlatformToolsPage => {
                if let Err(e) = open::that(PLATFORM_TOOLS_URL) {
                    self.log(format!("Could not open the browser: {e}"));
                }
            }
            Message::IpChanged(ip) => self.ip = ip,
            Message::PortChanged(port) => self.port = port,
            Message::Connect => {
                let Some(path) = self.adb.path().cloned() else {
                    self.log("Set up ADB first (sidebar).");
                    return Task::none();
                };
                if self.ip.trim().is_empty() {
                    self.log("Enter the TV's IP address first.");
                    return Task::none();
                }
                let serial = android::serial_for(&self.ip, &self.port);
                self.log(format!("Connecting to {serial}…"));
                self.connecting = true;
                self.connect_hint = None;
                return Task::perform(android::connect(path, serial), Message::Connected);
            }
            Message::Connected(result) => {
                self.connecting = false;
                match result {
                    ConnectResult::Connected { serial, summary } => {
                        let label = summary.label();
                        self.log(format!(
                            "Connected: {label}  |  Android {} (API {})  |  {}",
                            summary.android_version, summary.sdk, summary.abi
                        ));
                        self.device = Some(Device { platform: Platform::AndroidTv, address: serial, label });
                        self.summary = Some(summary);
                    }
                    ConnectResult::NeedsApproval(out) => {
                        self.log(out);
                        self.connect_hint = Some(
                            "Accept \"Allow debugging?\" on the TV, then press Connect again.".into(),
                        );
                    }
                    ConnectResult::Failed(out) => {
                        self.log(format!("Connection failed: {out}"));
                        self.connect_hint = Some(
                            "Check the IP, and that network debugging is on in Developer options.".into(),
                        );
                    }
                }
            }
            Message::Disconnect => {
                if let (Some(device), Some(path)) = (self.device.take(), self.adb.path().cloned()) {
                    self.summary = None;
                    self.tab = Tab::Info;
                    return Task::perform(android::disconnect(path, device.address), Message::Disconnected);
                }
            }
            Message::Disconnected(out) => self.log(out),
            Message::TabSelected(tab) => self.tab = tab,
        }
        Task::none()
    }

    fn view(&self) -> Element<'_, Message> {
        let main = column![self.tab_bar(), self.tab_content(), self.log_panel()]
            .spacing(8)
            .padding(8);
        row![self.sidebar(), main].into()
    }

    fn sidebar(&self) -> Element<'_, Message> {
        let small = |s: &'static str| text(s).size(11).color(theme::TEXT_DISABLED);

        let connect_button = if self.device.is_some() {
            button(text("Disconnect").center().width(Fill))
                .style(theme::danger_button)
                .on_press(Message::Disconnect)
        } else {
            let label = if self.connecting { "Connecting…" } else { "Connect" };
            button(text(label).center().width(Fill))
                .style(theme::primary_button)
                .on_press_maybe((!self.connecting).then_some(Message::Connect))
        };

        let status: Element<'_, Message> = match (&self.device, &self.connect_hint) {
            (Some(d), _) => column![
                text(&d.label).size(13).color(theme::SUCCESS),
                text(format!("{} · {}", d.platform.name(), d.address)).size(11).color(theme::TEXT_MUTED),
            ]
            .spacing(2)
            .into(),
            (None, Some(hint)) => text(hint).size(11).color(theme::WARNING).into(),
            (None, None) => space().into(),
        };

        let content = column![
            text("Devices").size(16).color(theme::TEXT),
            small("Connect by IP"),
            row![
                text_input("192.168.1.x", &self.ip)
                    .on_input(Message::IpChanged)
                    .on_submit(Message::Connect)
                    .size(13),
                text_input("5555", &self.port)
                    .on_input(Message::PortChanged)
                    .on_submit(Message::Connect)
                    .size(13)
                    .width(64),
            ]
            .spacing(6),
            connect_button.padding([8, 12]),
            status,
            space::vertical(),
            self.adb_status(),
        ]
        .spacing(10)
        .padding(14);

        container(content).width(260).height(Fill).style(theme::sidebar).into()
    }

    fn adb_status(&self) -> Element<'_, Message> {
        let change = |label| {
            button(text(label).size(11))
                .padding(0)
                .style(theme::link_button)
                .on_press(Message::ChangeAdb)
        };
        match &self.adb {
            AdbStatus::Ready { path, version } => {
                let short = match (path.parent().and_then(|p| p.file_name()), path.file_name()) {
                    (Some(dir), Some(file)) => {
                        format!("{}{}{}", dir.to_string_lossy(), std::path::MAIN_SEPARATOR, file.to_string_lossy())
                    }
                    _ => path.display().to_string(),
                };
                column![
                    row![
                        text(format!("ADB: {short}")).size(10).color(theme::TEXT_DISABLED).width(Fill),
                        change("Change…"),
                    ],
                    text(version).size(10).color(theme::TEXT_DISABLED),
                ]
                .spacing(2)
                .into()
            }
            AdbStatus::Starting => text("Starting ADB…").size(10).color(theme::TEXT_DISABLED).into(),
            AdbStatus::Missing | AdbStatus::Broken { .. } => {
                let reason = match &self.adb {
                    AdbStatus::Broken { error, .. } => format!("ADB isn't working: {error}"),
                    _ => "ADB not found.".into(),
                };
                container(
                    column![
                        text(reason).size(12).color(theme::DANGER),
                        text("Pick adb, or the platform-tools folder that contains it.")
                            .size(11)
                            .color(theme::TEXT_MUTED),
                        row![
                            button(text("Choose…").size(12))
                                .style(theme::secondary_button)
                                .on_press(Message::ChangeAdb),
                            button(text("Download").size(12))
                                .style(theme::link_button)
                                .on_press(Message::OpenPlatformToolsPage),
                        ]
                        .spacing(8),
                    ]
                    .spacing(6),
                )
                .padding(10)
                .style(theme::panel)
                .into()
            }
        }
    }

    fn tab_bar(&self) -> Element<'_, Message> {
        let tabs = Tab::ALL.iter().map(|&tab| {
            // Only Info works without a device, as in the original app.
            let enabled = tab == Tab::Info || self.device.is_some();
            button(text(tab.name()).size(13))
                .padding([6, 12])
                .style(theme::tab_button(tab == self.tab))
                .on_press_maybe(enabled.then_some(Message::TabSelected(tab)))
                .into()
        });
        container(row(tabs).spacing(2)).padding(4).width(Fill).style(theme::tab_bar).into()
    }

    fn tab_content(&self) -> Element<'_, Message> {
        let content: Element<'_, Message> = match self.tab {
            Tab::Info => self.info_tab(),
            other => empty_state(
                &format!("{} is being ported", other.name()),
                "This tab arrives in a later milestone of the Rust rewrite.",
            ),
        };
        container(content).width(Fill).height(Fill).into()
    }

    fn info_tab(&self) -> Element<'_, Message> {
        let Some(s) = &self.summary else {
            return empty_state("No TV connected", "Enter your TV's IP address in the sidebar and press Connect.");
        };
        let or_dash = |v: &str| if v.is_empty() { "—".to_owned() } else { v.to_owned() };
        let android = match (s.android_version.is_empty(), s.sdk.is_empty()) {
            (false, false) => format!("{} (API {})", s.android_version, s.sdk),
            _ => or_dash(&s.android_version),
        };
        let rows = [
            ("Manufacturer", or_dash(&s.manufacturer)),
            ("Model", or_dash(&s.model)),
            ("Model Number", or_dash(&s.model_number)),
            ("Android Version", android),
            ("CPU ABI", or_dash(&s.abi)),
        ];
        let fields = column(rows.into_iter().map(|(label, value)| {
            row![
                text(format!("{label}:")).size(13).color(theme::TEXT_MUTED).width(150).align_x(iced::alignment::Horizontal::Right),
                text(value).size(13).color(theme::TEXT),
            ]
            .spacing(12)
            .into()
        }))
        .spacing(10);
        let card = container(
            column![text("Device").size(12).color(theme::TEXT_MUTED), fields].spacing(10),
        )
        .padding(16)
        .width(Fill)
        .style(theme::panel);
        scrollable(column![card].padding(4)).into()
    }

    fn log_panel(&self) -> Element<'_, Message> {
        let lines = column(self.log.iter().map(|l| {
            text(l).size(12).font(Font::MONOSPACE).color(theme::TEXT).into()
        }))
        .spacing(2);
        container(
            column![
                text("Output Log").size(12).color(theme::TEXT_MUTED),
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

fn empty_state<'a>(title: &str, subtitle: &str) -> Element<'a, Message> {
    container(
        column![
            text(title.to_owned()).size(16).color(theme::TEXT),
            text(subtitle.to_owned()).size(12).color(theme::TEXT_MUTED),
        ]
        .spacing(6)
        .align_x(iced::Alignment::Center),
    )
    .center(Fill)
    .into()
}
