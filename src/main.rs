// Hide the console window in release builds on Windows.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod android;
mod config;
mod devices;
mod history;
mod mdns;
mod net;
mod platform;
mod theme;

use std::collections::BTreeSet;
use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::time::Duration;

use iced::widget::{button, column, container, row, rule, scrollable, space, text, text_input};
use iced::{Alignment, Element, Fill, Font, Length, Task, Theme};

use android::{ConnectResult, Connection, DeviceSummary};
use devices::Reach;
use history::History;
use mdns::MdnsDevice;
use platform::{Device, Platform};

/// How long each host gets to answer during a network scan.
const SCAN_TIMEOUT: Duration = Duration::from_millis(600);
/// How long to listen for mDNS announcements.
const MDNS_LISTEN: Duration = Duration::from_secs(3);

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

struct App {
    history: History,
    /// IPs where ADB answered, and every IP whose ADB port has been checked.
    open: BTreeSet<Ipv4Addr>,
    checked: BTreeSet<Ipv4Addr>,
    mdns: Vec<MdnsDevice>,
    subnet: String,
    scanning: bool,
    listening: bool,
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
    ScanFinished([u8; 3], Vec<Ipv4Addr>),
    Probed(Vec<(Ipv4Addr, bool)>),
    MdnsFinished(Result<Vec<MdnsDevice>, String>),
    Pick(Ipv4Addr, u16),
    Forget(Ipv4Addr),
    Import,
    ImportPicked(Option<PathBuf>),
    IpChanged(String),
    PortChanged(String),
    Connect,
    Connected(ConnectResult),
    Disconnect,
    TabSelected(Tab),
}

impl App {
    fn new() -> (Self, Task<Message>) {
        let subnet = net::local_ipv4().map(net::subnet_prefix).unwrap_or_else(|| "192.168.1".into());
        let history = history::load();
        let mut app = App {
            history,
            open: BTreeSet::new(),
            checked: BTreeSet::new(),
            mdns: Vec::new(),
            subnet,
            scanning: false,
            listening: false,
            ip: String::new(),
            port: android::ADB_PORT.to_string(),
            connecting: false,
            connect_hint: None,
            connection: None,
            device: None,
            summary: None,
            tab: Tab::Info,
            log: Vec::new(),
        };
        app.log(match app.history.len() {
            0 => "Ready. Scan your network or enter the TV's IP address.".to_owned(),
            n => format!("Ready. Checking {n} saved device(s)…"),
        });
        // On launch: see which saved devices are up, and listen for TVs.
        let task = Task::batch([app.probe_saved(), app.listen()]);
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

    fn probe_saved(&self) -> Task<Message> {
        let targets: Vec<(Ipv4Addr, u16)> = self
            .history
            .iter()
            .filter_map(|(ip, d)| Some((ip.parse().ok()?, d.port)))
            .collect();
        if targets.is_empty() {
            return Task::none();
        }
        Task::perform(net::probe(targets, Duration::from_secs(1)), Message::Probed)
    }

    fn listen(&mut self) -> Task<Message> {
        self.listening = true;
        Task::perform(mdns::discover(MDNS_LISTEN), Message::MdnsFinished)
    }

    fn save_history(&mut self) {
        if let Err(e) = history::save(&self.history) {
            self.log(format!("Could not save device history: {e}"));
        }
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
                self.log(format!("Scanning {a}.{b}.{c}.1–254 and listening for TVs…"));
                self.scanning = true;
                let scan = Task::perform(net::scan(prefix, android::ADB_PORT, SCAN_TIMEOUT), move |found| {
                    Message::ScanFinished(prefix, found)
                });
                return Task::batch([scan, self.listen()]);
            }
            Message::ScanFinished(prefix, found) => {
                self.scanning = false;
                for ip in net::hosts(prefix) {
                    self.checked.insert(ip);
                    self.open.remove(&ip);
                }
                self.log(match found.len() {
                    0 => "Scan: no devices with ADB open.".to_owned(),
                    n => format!("Scan: {n} device(s) with ADB open."),
                });
                self.open.extend(found);
            }
            Message::Probed(results) => {
                let up = results.iter().filter(|(_, open)| *open).count();
                let total = results.len();
                for (ip, open) in results {
                    self.checked.insert(ip);
                    if open {
                        self.open.insert(ip);
                    } else {
                        self.open.remove(&ip);
                    }
                }
                self.log(format!("Checked {total} device(s): {up} with ADB open."));
            }
            Message::MdnsFinished(result) => {
                self.listening = false;
                match result {
                    Ok(found) => {
                        let tvs: Vec<String> = found
                            .iter()
                            .filter(|d| d.is_tv())
                            .filter_map(|d| d.name.clone().or_else(|| d.ip.map(|ip| ip.to_string())))
                            .collect();
                        if !tvs.is_empty() {
                            self.log(format!("Announced on the network: {}", tvs.join(", ")));
                        }
                        // Check ADB on announced TVs not checked yet, so the
                        // list can say "debugging off" instead of nothing.
                        let unchecked: Vec<(Ipv4Addr, u16)> = found
                            .iter()
                            .filter(|d| d.is_tv())
                            .filter_map(|d| Some((d.ip?, d.adb_port.unwrap_or(android::ADB_PORT))))
                            .filter(|(ip, _)| !self.checked.contains(ip))
                            .collect();
                        self.mdns = found;
                        if !unchecked.is_empty() {
                            return Task::perform(net::probe(unchecked, Duration::from_secs(1)), Message::Probed);
                        }
                    }
                    Err(e) => self.log(format!("mDNS discovery failed: {e}")),
                }
            }
            Message::Pick(ip, port) => {
                self.ip = ip.to_string();
                self.port = port.to_string();
                return self.start_connect();
            }
            Message::Forget(ip) => {
                if let Some(d) = self.history.remove(&ip.to_string()) {
                    self.log(format!("Forgot {}.", d.label));
                    self.save_history();
                }
            }
            Message::Import => {
                return Task::perform(
                    async {
                        rfd::AsyncFileDialog::new()
                            .set_title("Import devices.json")
                            .add_filter("Device history", &["json"])
                            .pick_file()
                            .await
                            .map(|f| f.path().to_path_buf())
                    },
                    Message::ImportPicked,
                );
            }
            Message::ImportPicked(None) => {}
            Message::ImportPicked(Some(path)) => match history::load_from(&path) {
                Ok(imported) => {
                    let n = history::merge(&mut self.history, imported);
                    self.log(format!("Imported {n} device(s) from {}.", path.display()));
                    self.save_history();
                    return self.probe_saved();
                }
                Err(e) => self.log(format!("Import failed: {e}")),
            },
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
                        if let Some((ip, port)) = serial.rsplit_once(':') {
                            let port = port.parse().unwrap_or(android::ADB_PORT);
                            history::remember(&mut self.history, ip, port, &label);
                            self.save_history();
                            if let Ok(ip) = ip.parse() {
                                self.open.insert(ip);
                                self.checked.insert(ip);
                            }
                        }
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

    fn device_list(&self) -> Element<'_, Message> {
        let now = chrono::Local::now().naive_local();
        let entries = devices::merge(&self.history, &self.open, &self.checked, &self.mdns, now);
        if entries.is_empty() {
            let hint = if self.scanning || self.listening { "Looking for TVs…" } else { "No devices yet. Press Scan." };
            return text(hint).size(11).color(theme::TEXT_DISABLED).into();
        }
        let connected_ip = self.device.as_ref().and_then(|d| d.address.split(':').next());
        column(entries.into_iter().map(|e| {
            let connected = connected_ip == Some(e.ip.to_string().as_str());
            let dot = match (connected, &e.reach) {
                (true, _) | (_, Reach::Ready) => theme::SUCCESS,
                (_, Reach::DebuggingOff | Reach::NeedsPairing) => theme::WARNING,
                (_, Reach::Offline) => theme::TEXT_DISABLED,
                (_, Reach::Unknown) => theme::TEXT_MUTED,
            };
            let label = row![
                text("●").size(12).color(dot),
                column![
                    text(e.title).size(13),
                    text(e.detail).size(10).color(theme::TEXT_MUTED),
                ]
                .spacing(1)
                .width(Fill),
            ]
            .spacing(8)
            .align_y(Alignment::Center);
            let pick = button(label)
                .padding([5, 8])
                .width(Fill)
                .style(theme::tab_button(connected))
                .on_press_maybe((!self.connecting).then_some(Message::Pick(e.ip, e.port)));
            let forget: Element<'_, Message> = if e.saved {
                button(text("×").size(13))
                    .padding([2, 6])
                    .style(theme::tab_button(false))
                    .on_press(Message::Forget(e.ip))
                    .into()
            } else {
                space().into()
            };
            row![pick, forget].spacing(2).align_y(Alignment::Center).into()
        }))
        .spacing(2)
        .into()
    }

    fn sidebar(&self) -> Element<'_, Message> {
        let small = |s: &'static str| text(s).size(11).color(theme::TEXT_DISABLED);

        let busy = self.scanning || self.listening;
        let scan_row = row![
            text_input("192.168.1", &self.subnet)
                .on_input(Message::SubnetChanged)
                .on_submit(Message::Scan)
                .size(13),
            button(text(if busy { "Scanning…" } else { "Scan" }).size(13))
                .padding([6, 12])
                .style(theme::secondary_button)
                .on_press_maybe((!self.scanning).then_some(Message::Scan)),
        ]
        .spacing(6);

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
            scan_row,
            scrollable(self.device_list()).height(Fill),
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
            row![
                small("Built-in ADB: no adb.exe needed.").width(Fill),
                button(text("Import…").size(11))
                    .padding([2, 6])
                    .style(theme::tab_button(false))
                    .on_press(Message::Import),
            ]
            .align_y(Alignment::Center),
        ]
        .spacing(10)
        .padding(14);

        container(content).width(280).height(Fill).style(theme::sidebar).into()
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
                "Pick a TV in the sidebar, or enter its IP address and press Connect.",
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
        .align_x(Alignment::Center),
    )
    .center(Fill)
    .into()
}
