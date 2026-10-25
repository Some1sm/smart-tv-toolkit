// Hide the console window in release builds on Windows.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod android;
mod net;
mod platform;
mod theme;

use std::net::Ipv4Addr;
use std::time::Duration;

use iced::widget::{button, column, container, row, rule, scrollable, space, text, text_input};
use iced::{Element, Fill, Font, Length, Task, Theme};

use android::{ConnectResult, Connection, DeviceSummary};
use platform::{Device, Platform};

/// How long each host gets to answer during a network scan.
const SCAN_TIMEOUT: Duration = Duration::from_millis(600);

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

#[derive(Debug, Clone, PartialEq, Eq)]
enum ScanState {
    Idle,
    Scanning,
    Done(Vec<Ipv4Addr>),
}

struct App {
    subnet: String,
    scan: ScanState,
    ip: String,
    port: String,
    connecting: bool,
    connect_hint: Option<String>,
    connection: Option<Connection>,
    device: Option<Device>,
    summary: Option<DeviceSummary>,
    tab: Tab,
    log: Vec<String>,
}

#[derive(Debug, Clone)]
enum Message {
    SubnetChanged(String),
    Scan,
    ScanFinished(Vec<Ipv4Addr>),
    PickFound(Ipv4Addr),
    IpChanged(String),
    PortChanged(String),
    Connect,
    Connected(ConnectResult),
    Disconnect,
    TabSelected(Tab),
}

impl App {
    fn new() -> Self {
        let subnet = net::local_ipv4().map(net::subnet_prefix).unwrap_or_else(|| "192.168.1".into());
        App {
            subnet,
            scan: ScanState::Idle,
            ip: String::new(),
            port: android::ADB_PORT.to_string(),
            connecting: false,
            connect_hint: None,
            connection: None,
            device: None,
            summary: None,
            tab: Tab::Info,
            log: vec!["Ready. Scan your network or enter the TV's IP address.".into()],
        }
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

    fn start_connect(&mut self) -> Task<Message> {
        if self.ip.trim().is_empty() {
            self.log("Enter the TV's IP address first.");
            return Task::none();
        }
        // Switching TVs: drop the old connection first.
        self.disconnect();
        let serial = android::serial_for(&self.ip, &self.port);
        self.log(format!("Connecting to {serial}…"));
        self.connecting = true;
        self.connect_hint = None;
        Task::perform(android::connect(serial), Message::Connected)
    }

    fn disconnect(&mut self) {
        if let Some(device) = self.device.take() {
            self.log(format!("Disconnected from {}.", device.label));
        }
        self.connection = None;
        self.summary = None;
        self.tab = Tab::Info;
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::SubnetChanged(subnet) => self.subnet = subnet,
            Message::Scan => {
                let Some(prefix) = net::parse_prefix(&self.subnet) else {
                    self.log("Enter a subnet like 192.168.1 to scan.");
                    return Task::none();
                };
                let [a, b, c] = prefix;
                self.log(format!("Scanning {a}.{b}.{c}.1–254 for TVs (port {})…", android::ADB_PORT));
                self.scan = ScanState::Scanning;
                return Task::perform(
                    net::scan(prefix, android::ADB_PORT, SCAN_TIMEOUT),
                    Message::ScanFinished,
                );
            }
            Message::ScanFinished(found) => {
                self.log(match found.len() {
                    0 => "No TVs found. Check that network debugging is on, or enter the IP.".to_owned(),
                    n => format!("Found {n} device(s) with ADB open."),
                });
                self.scan = ScanState::Done(found);
            }
            Message::PickFound(ip) => {
                self.ip = ip.to_string();
                self.port = android::ADB_PORT.to_string();
                return self.start_connect();
            }
            Message::IpChanged(ip) => self.ip = ip,
            Message::PortChanged(port) => self.port = port,
            Message::Connect => return self.start_connect(),
            Message::Connected(result) => {
                self.connecting = false;
                match result {
                    ConnectResult::Connected { serial, connection, summary } => {
                        let label = summary.label();
                        self.log(format!(
                            "Connected: {label}  |  Android {} (API {})  |  {}",
                            summary.android_version, summary.sdk, summary.abi
                        ));
                        self.connection = Some(connection);
                        self.device = Some(Device { platform: Platform::AndroidTv, address: serial, label });
                        self.summary = Some(summary);
                    }
                    ConnectResult::NeedsApproval => {
                        self.log("The TV hasn't allowed debugging from this computer yet.");
                        self.connect_hint = Some(
                            "On the TV, choose Allow (tick \"Always allow\"), then press Connect again.".into(),
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
            Message::Disconnect => self.disconnect(),
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

        let scanning = self.scan == ScanState::Scanning;
        let scan_row = row![
            text_input("192.168.1", &self.subnet)
                .on_input(Message::SubnetChanged)
                .on_submit(Message::Scan)
                .size(13),
            button(text(if scanning { "Scanning…" } else { "Scan" }).size(13))
                .padding([6, 12])
                .style(theme::secondary_button)
                .on_press_maybe((!scanning).then_some(Message::Scan)),
        ]
        .spacing(6);

        let found: Element<'_, Message> = match &self.scan {
            ScanState::Done(ips) if ips.is_empty() => small("No TVs found on this subnet.").into(),
            ScanState::Done(ips) => column(ips.iter().map(|&ip| {
                let connected = self.device.as_ref().is_some_and(|d| d.address.starts_with(&format!("{ip}:")));
                let label = if connected { format!("●  {ip}") } else { ip.to_string() };
                button(text(label).size(13).width(Fill))
                    .padding([6, 10])
                    .style(theme::tab_button(connected))
                    .on_press_maybe((!self.connecting).then_some(Message::PickFound(ip)))
                    .into()
            }))
            .spacing(2)
            .into(),
            _ => space().into(),
        };

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
            small("Find TVs on your network"),
            scan_row,
            scrollable(found).height(Length::Shrink),
            rule::horizontal(1),
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
            small("Built-in ADB: no adb.exe needed."),
        ]
        .spacing(10)
        .padding(14);

        container(content).width(260).height(Fill).style(theme::sidebar).into()
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
            return empty_state(
                "No TV connected",
                "Scan your network in the sidebar, or enter the TV's IP address and press Connect.",
            );
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
                text(format!("{label}:"))
                    .size(13)
                    .color(theme::TEXT_MUTED)
                    .width(150)
                    .align_x(iced::alignment::Horizontal::Right),
                text(value).size(13).color(theme::TEXT),
            ]
            .spacing(12)
            .into()
        }))
        .spacing(10);
        let card = container(column![text("Device").size(12).color(theme::TEXT_MUTED), fields].spacing(10))
            .padding(16)
            .width(Fill)
            .style(theme::panel);
        scrollable(column![card].padding(4)).into()
    }

    fn log_panel(&self) -> Element<'_, Message> {
        let lines = column(
            self.log
                .iter()
                .map(|l| text(l).size(12).font(Font::MONOSPACE).color(theme::TEXT).into()),
        )
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
