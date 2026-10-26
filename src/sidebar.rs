//! The sidebar: finding TVs (scan, mDNS, history) and connecting to one.

use std::collections::BTreeSet;
use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::time::Duration;

use iced::widget::{button, column, container, row, rule, scrollable, space, text, text_input};
use iced::{Alignment, Element, Fill, Task};

use crate::devices::{self, Reach};
use crate::history::{self, History};
use crate::log::Log;
use crate::mdns::{self, MdnsDevice};
use crate::{android, net, theme};

/// How long each host gets to answer during a network scan.
const SCAN_TIMEOUT: Duration = Duration::from_millis(600);
/// How long to listen for mDNS announcements.
const MDNS_LISTEN: Duration = Duration::from_secs(3);

#[derive(Debug)]
pub struct State {
    history: History,
    /// IPs where ADB answered, and every IP whose ADB port has been checked.
    open: BTreeSet<Ipv4Addr>,
    checked: BTreeSet<Ipv4Addr>,
    mdns: Vec<MdnsDevice>,
    pub subnet: String,
    scanning: bool,
    listening: bool,
    ip: String,
    port: String,
    pub connecting: bool,
    pub hint: Option<String>,
    /// A newer app release: (version, page URL).
    pub update_available: Option<(String, String)>,
    /// The Android 11+ pairing form.
    pair_open: bool,
    pair_address: String,
    pair_code: String,
    pairing: bool,
}

#[derive(Debug, Clone)]
pub enum Msg {
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
    Disconnect,
    OpenRelease,
    TogglePair,
    PairAddress(String),
    PairCode(String),
    Pair,
    Paired(Result<String, String>),
}

/// What the sidebar asks the app shell to do.
pub enum Request {
    Connect(String),
    Disconnect,
}

/// The connection line under the Connect button.
pub enum Status {
    None,
    Connected { label: String, detail: String },
    Reconnecting { label: String },
}

impl State {
    pub fn new(subnet: Option<String>) -> Self {
        let subnet = subnet
            .or_else(|| net::local_ipv4().map(net::subnet_prefix))
            .unwrap_or_else(|| "192.168.1".into());
        State {
            history: history::load(),
            open: BTreeSet::new(),
            checked: BTreeSet::new(),
            mdns: Vec::new(),
            subnet,
            scanning: false,
            listening: false,
            ip: String::new(),
            port: android::ADB_PORT.to_string(),
            connecting: false,
            hint: None,
            update_available: None,
            pair_open: false,
            pair_address: String::new(),
            pair_code: String::new(),
            pairing: false,
        }
    }

    pub fn saved_count(&self) -> usize {
        self.history.len()
    }

    /// On launch: see which saved devices are up, and listen for TVs.
    pub fn startup(&mut self) -> Task<Msg> {
        Task::batch([self.probe_saved(), self.listen()])
    }

    fn probe_saved(&self) -> Task<Msg> {
        let targets: Vec<(Ipv4Addr, u16)> =
            self.history.iter().filter_map(|(ip, d)| Some((ip.parse().ok()?, d.port))).collect();
        if targets.is_empty() {
            return Task::none();
        }
        Task::perform(net::probe(targets, Duration::from_secs(1)), Msg::Probed)
    }

    fn listen(&mut self) -> Task<Msg> {
        self.listening = true;
        Task::perform(mdns::discover(MDNS_LISTEN), Msg::MdnsFinished)
    }

    fn save_history(&self, log: &mut Log) {
        if let Err(e) = history::save(&self.history) {
            log.push(format!("Could not save device history: {e}"));
        }
    }

    /// Fill the IP/port fields, e.g. for reconnecting to the last TV.
    pub fn set_target(&mut self, serial: &str) {
        let (ip, port) = serial.rsplit_once(':').unwrap_or((serial, "5555"));
        self.ip = ip.to_owned();
        self.port = port.to_owned();
    }

    /// A TV connected: remember it.
    pub fn connected(&mut self, serial: &str, label: &str, log: &mut Log) {
        self.connecting = false;
        self.hint = None;
        if let Some((ip, port)) = serial.rsplit_once(':') {
            history::remember(&mut self.history, ip, port.parse().unwrap_or(android::ADB_PORT), label);
            self.save_history(log);
            if let Ok(ip) = ip.parse() {
                self.open.insert(ip);
                self.checked.insert(ip);
            }
        }
    }

    pub fn update(&mut self, msg: Msg, log: &mut Log) -> (Task<Msg>, Option<Request>) {
        match msg {
            Msg::SubnetChanged(subnet) => self.subnet = subnet,
            Msg::Scan => {
                let Some(prefix) = net::parse_prefix(&self.subnet) else {
                    log.push("Enter a subnet like 192.168.1 to scan.");
                    return (Task::none(), None);
                };
                let [a, b, c] = prefix;
                log.push(format!("Scanning {a}.{b}.{c}.1–254 and listening for TVs…"));
                self.scanning = true;
                let scan = Task::perform(net::scan(prefix, android::ADB_PORT, SCAN_TIMEOUT), move |found| {
                    Msg::ScanFinished(prefix, found)
                });
                return (Task::batch([scan, self.listen()]), None);
            }
            Msg::ScanFinished(prefix, found) => {
                self.scanning = false;
                for ip in net::hosts(prefix) {
                    self.checked.insert(ip);
                    self.open.remove(&ip);
                }
                log.push(match found.len() {
                    0 => "Scan: no devices with ADB open.".to_owned(),
                    n => format!("Scan: {n} device(s) with ADB open."),
                });
                self.open.extend(found);
            }
            Msg::Probed(results) => {
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
                log.push(format!("Checked {total} device(s): {up} with ADB open."));
            }
            Msg::MdnsFinished(result) => {
                self.listening = false;
                match result {
                    Ok(found) => {
                        let tvs: Vec<String> = found
                            .iter()
                            .filter(|d| d.is_tv())
                            .filter_map(|d| d.name.clone().or_else(|| d.ip.map(|ip| ip.to_string())))
                            .collect();
                        if !tvs.is_empty() {
                            log.push(format!("Announced on the network: {}", tvs.join(", ")));
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
                            return (Task::perform(net::probe(unchecked, Duration::from_secs(1)), Msg::Probed), None);
                        }
                    }
                    Err(e) => log.push(format!("mDNS discovery failed: {e}")),
                }
            }
            Msg::Pick(ip, port) => {
                self.ip = ip.to_string();
                self.port = port.to_string();
                return self.update(Msg::Connect, log);
            }
            Msg::Forget(ip) => {
                if let Some(d) = self.history.remove(&ip.to_string()) {
                    log.push(format!("Forgot {}.", d.label));
                    self.save_history(log);
                }
            }
            Msg::Import => {
                let dialog =
                    rfd::AsyncFileDialog::new().set_title("Import devices.json").add_filter("Device history", &["json"]);
                return (Task::perform(async move { dialog.pick_file().await.map(|f| f.path().to_path_buf()) }, Msg::ImportPicked), None);
            }
            Msg::ImportPicked(None) => {}
            Msg::ImportPicked(Some(path)) => match history::load_from(&path) {
                Ok(imported) => {
                    let n = history::merge(&mut self.history, imported);
                    log.push(format!("Imported {n} device(s) from {}.", path.display()));
                    self.save_history(log);
                    return (self.probe_saved(), None);
                }
                Err(e) => log.push(format!("Import failed: {e}")),
            },
            Msg::IpChanged(ip) => self.ip = ip,
            Msg::PortChanged(port) => self.port = port,
            Msg::Connect => {
                if self.ip.trim().is_empty() {
                    log.push("Enter the TV's IP address first.");
                    return (Task::none(), None);
                }
                self.connecting = true;
                self.hint = None;
                return (Task::none(), Some(Request::Connect(android::serial_for(&self.ip, &self.port))));
            }
            Msg::Disconnect => return (Task::none(), Some(Request::Disconnect)),
            Msg::TogglePair => {
                self.pair_open = !self.pair_open;
                if self.pair_open && self.pair_address.is_empty() && !self.ip.trim().is_empty() {
                    self.pair_address = format!("{}:", self.ip.trim());
                }
            }
            Msg::PairAddress(v) => self.pair_address = v,
            Msg::PairCode(v) => self.pair_code = v.chars().filter(char::is_ascii_digit).take(6).collect(),
            Msg::Pair => {
                if !android::pairing::valid_code(&self.pair_code) {
                    log.push("The pairing code is the six digits shown on the TV.");
                    return (Task::none(), None);
                }
                self.pairing = true;
                log.push(format!("Pairing with {}…", self.pair_address.trim()));
                let (address, code) = (self.pair_address.clone(), self.pair_code.clone());
                return (Task::perform(android::pairing::pair(address, code), Msg::Paired), None);
            }
            Msg::Paired(result) => {
                self.pairing = false;
                match result {
                    Ok(_) => {
                        self.pair_code.clear();
                        self.pair_open = false;
                        log.push("Paired. The TV now trusts this computer.");
                        // Connect straight away if the TV announced its connect port.
                        let ip = self.pair_address.trim().rsplit_once(':').map(|(ip, _)| ip.to_owned()).unwrap_or_default();
                        let tls_port = self
                            .mdns
                            .iter()
                            .find(|d| d.ip.is_some_and(|a| a.to_string() == ip))
                            .and_then(|d| d.tls_port);
                        match tls_port {
                            Some(port) => {
                                self.ip = ip;
                                self.port = port.to_string();
                                return self.update(Msg::Connect, log);
                            }
                            None => {
                                self.ip = ip;
                                self.port.clear();
                                self.hint = Some(
                                    "Paired. Now enter the port shown under \"IP address & port\" on the TV's Wireless debugging screen (not the pairing port), then Connect.".into(),
                                );
                            }
                        }
                    }
                    Err(e) => log.push(format!("Pairing failed: {e}")),
                }
            }
            Msg::OpenRelease => {
                if let Some((_, url)) = &self.update_available
                    && let Err(e) = open::that(url)
                {
                    log.push(format!("Could not open the browser: {e}"));
                }
            }
        }
        (Task::none(), None)
    }

    /// Android 11+ wireless debugging: pair once with the code on the TV.
    fn pair_form(&self) -> Element<'_, Msg> {
        let toggle = button(
            text(if self.pair_open { "▾ Pair with a code (Android 11+)" } else { "▸ Pair with a code (Android 11+)" })
                .size(11)
                .color(theme::PRIMARY),
        )
        .padding([2, 4])
        .style(theme::tab_button(false))
        .on_press(Msg::TogglePair);
        if !self.pair_open {
            return toggle.into();
        }
        let ready = android::pairing::valid_code(&self.pair_code) && !self.pairing;
        column![
            toggle,
            text("On the TV: Developer options → Wireless debugging → Pair device with pairing code.")
                .size(10)
                .color(theme::TEXT_MUTED),
            row![
                text_input("IP:pairing port", &self.pair_address).on_input(Msg::PairAddress).size(12),
                text_input("Code", &self.pair_code).on_input(Msg::PairCode).on_submit(Msg::Pair).size(12).width(70),
            ]
            .spacing(6),
            button(text(if self.pairing { "Pairing…" } else { "Pair" }).size(12).center().width(Fill))
                .padding([6, 12])
                .style(theme::secondary_button)
                .on_press_maybe(ready.then_some(Msg::Pair)),
        ]
        .spacing(6)
        .into()
    }

    fn update_notice(&self) -> Element<'_, Msg> {
        match &self.update_available {
            Some((version, _)) => button(text(format!("Version {version} is available. Get it…")).size(11).color(theme::PRIMARY))
                .padding([4, 8])
                .width(Fill)
                .style(theme::tab_button(false))
                .on_press(Msg::OpenRelease)
                .into(),
            None => space().into(),
        }
    }

    fn device_list(&self, connected_ip: Option<&str>) -> Element<'_, Msg> {
        let now = chrono::Local::now().naive_local();
        let entries = devices::merge(&self.history, &self.open, &self.checked, &self.mdns, now);
        if entries.is_empty() {
            let hint = if self.scanning || self.listening { "Looking for TVs…" } else { "No devices yet. Press Scan." };
            return text(hint).size(11).color(theme::TEXT_DISABLED).into();
        }
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
                column![text(e.title).size(13), text(e.detail).size(10).color(theme::TEXT_MUTED)].spacing(1).width(Fill),
            ]
            .spacing(8)
            .align_y(Alignment::Center);
            let pick = button(label)
                .padding([5, 8])
                .width(Fill)
                .style(theme::tab_button(connected))
                .on_press_maybe((!self.connecting).then_some(Msg::Pick(e.ip, e.port)));
            let forget: Element<'_, Msg> = if e.saved {
                button(text("×").size(13)).padding([2, 6]).style(theme::tab_button(false)).on_press(Msg::Forget(e.ip)).into()
            } else {
                space().into()
            };
            row![pick, forget].spacing(2).align_y(Alignment::Center).into()
        }))
        .spacing(2)
        .into()
    }

    pub fn view(&self, status: Status) -> Element<'_, Msg> {
        let small = |s: &'static str| text(s).size(11).color(theme::TEXT_DISABLED);
        let busy = self.scanning || self.listening;
        let scan_row = row![
            text_input("192.168.1", &self.subnet).on_input(Msg::SubnetChanged).on_submit(Msg::Scan).size(13),
            button(text(if busy { "Scanning…" } else { "Scan" }).size(13))
                .padding([6, 12])
                .style(theme::secondary_button)
                .on_press_maybe((!self.scanning).then_some(Msg::Scan)),
        ]
        .spacing(6);

        let connected = !matches!(status, Status::None);
        let connect_button = if connected {
            button(text("Disconnect").center().width(Fill)).style(theme::danger_button).on_press(Msg::Disconnect)
        } else {
            let label = if self.connecting { "Connecting…" } else { "Connect" };
            button(text(label).center().width(Fill))
                .style(theme::primary_button)
                .on_press_maybe((!self.connecting).then_some(Msg::Connect))
        };

        let connected_ip: Option<String> = match &status {
            Status::Connected { detail, .. } => {
                detail.rsplit(" · ").next().and_then(|a| a.split(':').next()).map(str::to_owned)
            }
            _ => None,
        };
        let status_view: Element<'_, Msg> = match status {
            Status::Connected { label, detail } => column![
                text(label).size(13).color(theme::SUCCESS),
                text(detail).size(11).color(theme::TEXT_MUTED),
            ]
            .spacing(2)
            .into(),
            Status::Reconnecting { label } => column![
                text(label).size(13).color(theme::WARNING),
                text("Connection lost. Reconnecting…").size(11).color(theme::WARNING),
            ]
            .spacing(2)
            .into(),
            Status::None => match &self.hint {
                Some(hint) => text(hint).size(11).color(theme::WARNING).into(),
                None => space().into(),
            },
        };

        let content = column![
            text("Devices").size(16).color(theme::TEXT),
            scan_row,
            scrollable(self.device_list(connected_ip.as_deref())).height(Fill),
            rule::horizontal(1),
            small("Connect by IP"),
            row![
                text_input("192.168.1.x", &self.ip).on_input(Msg::IpChanged).on_submit(Msg::Connect).size(13),
                text_input("5555", &self.port).on_input(Msg::PortChanged).on_submit(Msg::Connect).size(13).width(64),
            ]
            .spacing(6),
            connect_button.padding([8, 12]),
            self.pair_form(),
            status_view,
            self.update_notice(),
            row![
                small("Built-in ADB: no adb.exe needed.").width(Fill),
                button(text("Import…").size(11)).padding([2, 6]).style(theme::tab_button(false)).on_press(Msg::Import),
            ]
            .align_y(Alignment::Center),
        ]
        .spacing(10)
        .padding(14);

        container(content).width(280).height(Fill).style(theme::sidebar).into()
    }
}
