//! The sidebar: finding TVs (scan, mDNS, history) and connecting to one.

use std::collections::{BTreeMap, BTreeSet};
use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::time::Duration;

use iced::widget::{Space, button, checkbox, column, container, hover, progress_bar, row, scrollable, space};
use iced::{Alignment, Color, Element, Length, Padding, Task, color};

use crate::devices::{self, Reach};
use crate::history::{self, History};
use crate::log::Log;
use crate::mdns::{self, MdnsDevice};
use crate::android::telemetry;
use crate::design::{self as ds, alpha, style};
use crate::platform::Session;
use crate::i18n::trf;
use crate::platform::Platform;
use crate::{android, lg, net, samsung};

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
    /// Samsung/LG TVs: IP → the port to connect on.
    others: BTreeMap<Ipv4Addr, u16>,
    /// Scans still running (one per port).
    scans: u8,
    /// Android TVs that batch actions also run on.
    pub batch: Vec<crate::tabs::Target>,
    mdns: Vec<MdnsDevice>,
    pub subnet: String,
    scanning: bool,
    listening: bool,
    ip: String,
    port: String,
    pub connecting: bool,
    pub hint: Option<String>,
    /// A newer app release.
    pub update_available: Option<crate::update::Release>,
    pub update_state: UpdateState,
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
    ScanFinished([u8; 3], u16, Vec<Ipv4Addr>),
    Probed(Vec<(Ipv4Addr, u16, bool)>),
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
    UpdateNow,
    Restart,
    /// Re-check saved TVs and listen for announcements (no full scan).
    Refresh,
    Diagnostics,
    /// Tick or untick a TV (serial, name) for batch actions.
    ToggleBatch(String, String),
    /// Send a Wake-on-LAN packet to a saved TV.
    Wake(Ipv4Addr),
    TogglePair,
    PairAddress(String),
    PairCode(String),
    Pair,
    Paired(Result<String, String>),
}

/// Where an in-app update stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateState {
    Available,
    Downloading,
    /// Installed; a restart switches to it.
    Ready,
}

/// What the sidebar asks the app shell to do.
pub enum Request {
    /// Download and install the available update.
    Update,
    /// Start the new version and quit this one.
    Restart,
    Connect(String),
    Disconnect,
    /// Diagnostics: save the log to a file.
    SaveLog,
}

/// Live numbers for the telemetry card, kept by the app shell.
#[derive(Debug, Clone, Default)]
pub struct Telemetry {
    /// CPU busy share 0–1, once two samples exist.
    pub cpu: Option<f32>,
    /// "4-core A73 @ 1.8GHz"
    pub cpu_description: String,
    pub ram_used: u64,
    pub ram_total: u64,
    pub latency_ms: Option<f32>,
}

/// What the sidebar shows about the connected TV.
pub struct Context<'a> {
    pub session: Option<&'a Session>,
    pub reconnecting: bool,
    pub telemetry: &'a Telemetry,
    pub log_lines: usize,
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
            others: BTreeMap::new(),
            scans: 0,
            batch: Vec::new(),
            mdns: Vec::new(),
            subnet,
            scanning: false,
            listening: false,
            ip: String::new(),
            port: android::ADB_PORT.to_string(),
            connecting: false,
            hint: None,
            update_available: None,
            update_state: UpdateState::Available,
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

    /// A Samsung or LG remote-control port answered on `ip`.
    fn note_remote_api(&mut self, ip: Ipv4Addr, port: u16) {
        self.checked.insert(ip);
        let connect_port = match port {
            samsung::REST_PORT | samsung::PORT => samsung::PORT,
            // Prefer LG's secure port when both answer.
            lg::PLAIN_PORT if self.others.get(&ip) == Some(&lg::PORT) => lg::PORT,
            other => other,
        };
        self.others.insert(ip, connect_port);
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

    /// Remember a TV's wired MAC address (for Wake-on-LAN).
    pub fn remember_mac(&mut self, serial: &str, mac: &str, log: &mut Log) {
        if history::set_mac(&mut self.history, &crate::journal::device_key(serial), mac) {
            self.save_history(log);
        }
    }

    /// A TV connected: remember it.
    pub fn connected(&mut self, serial: &str, label: &str, log: &mut Log) {
        // The connected TV gets every action anyway.
        let ip = crate::journal::device_key(serial);
        self.batch.retain(|t| crate::journal::device_key(&t.serial) != ip);
        self.connecting = false;
        self.hint = None;
        if let Some((ip, port)) = serial.rsplit_once(':') {
            history::remember(&mut self.history, ip, port.parse().unwrap_or(android::ADB_PORT), label);
            self.save_history(log);
            if let Ok(ip) = ip.parse() {
                match port.parse().map(Platform::for_port) {
                    Ok(Platform::AndroidTv) | Err(_) => {
                        self.open.insert(ip);
                    }
                    Ok(_) => {
                        self.others.insert(ip, port.parse().unwrap_or(samsung::PORT));
                    }
                }
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
                // ADB, Samsung's REST port and both LG ports, all at once.
                let ports = [android::ADB_PORT, samsung::REST_PORT, lg::PORT, lg::PLAIN_PORT];
                self.scans = ports.len() as u8;
                let scans = ports.map(|port| {
                    Task::perform(net::scan(prefix, port, SCAN_TIMEOUT), move |found| Msg::ScanFinished(prefix, port, found))
                });
                return (Task::batch(scans.into_iter().chain([self.listen()])), None);
            }
            Msg::ScanFinished(prefix, port, found) => {
                self.scans = self.scans.saturating_sub(1);
                self.scanning = self.scans > 0;
                match Platform::for_port(port) {
                    Platform::AndroidTv => {
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
                    platform => {
                        if !found.is_empty() {
                            log.push(format!("Scan: {} {} TV(s).", found.len(), platform.name()));
                        }
                        for ip in found {
                            self.note_remote_api(ip, port);
                        }
                    }
                }
            }
            Msg::Probed(results) => {
                let adb: Vec<_> = results.iter().filter(|(_, port, _)| Platform::for_port(*port) == Platform::AndroidTv).collect();
                let up = adb.iter().filter(|(_, _, open)| *open).count();
                let total = adb.len();
                for (ip, port, open) in results {
                    match Platform::for_port(port) {
                        Platform::AndroidTv => {
                            self.checked.insert(ip);
                            if open {
                                self.open.insert(ip);
                            } else {
                                self.open.remove(&ip);
                            }
                        }
                        _ if open => self.note_remote_api(ip, port),
                        _ => {
                            self.checked.insert(ip);
                            self.others.remove(&ip);
                        }
                    }
                }
                if total > 0 {
                    log.push(format!("Checked {total} device(s): {up} with ADB open."));
                }
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
            Msg::Refresh => {
                log.push("Refreshing devices…");
                return (Task::batch([self.probe_saved(), self.listen()]), None);
            }
            Msg::Diagnostics => return (Task::none(), Some(Request::SaveLog)),
            Msg::Wake(ip) => match history::mac(&self.history, &ip.to_string()) {
                Some(mac) => log.push(match net::wake_on_lan(&mac, Some(ip)) {
                    Ok(()) => format!("Sent Wake-on-LAN to {mac}. If the TV allows it, it turns on in a few seconds; then Connect."),
                    Err(e) => format!("Couldn't send Wake-on-LAN: {e}"),
                }),
                None => log.push("This TV's network address isn't known yet; connect to it once while it's on."),
            },
            Msg::ToggleBatch(serial, label) => {
                if let Some(i) = self.batch.iter().position(|t| t.serial == serial) {
                    self.batch.remove(i);
                } else {
                    log.push(format!("{label} will also get Packages and Install actions."));
                    self.batch.push(crate::tabs::Target { serial, label });
                }
            }
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
                if let Some(release) = &self.update_available
                    && let Err(e) = open::that(&release.page)
                {
                    log.push(format!("Could not open the browser: {e}"));
                }
            }
            Msg::UpdateNow => return (Task::none(), Some(Request::Update)),
            Msg::Restart => return (Task::none(), Some(Request::Restart)),
        }
        (Task::none(), None)
    }
}

impl State {
    fn header(&self, connected: usize, listed: usize) -> Element<'_, Msg> {
        let busy = self.scanning || self.listening;
        let title = row![
            ds::overline("Target Devices"),
            ds::tinted_badge(format!("{connected}/{listed}"), ds::ACCENT),
            space::horizontal(),
            ds::square_button(ds::Icon::RefreshCw, ds::TEXT_SECONDARY, 24.0, style::ghost, (!busy).then_some(Msg::Refresh)),
        ]
        .spacing(8)
        .align_y(Alignment::Center);
        let scan = row![
            ds::input("192.168.1.0/24", &self.subnet, true, color!(0x181818)).on_input(Msg::SubnetChanged).on_submit(Msg::Scan),
            ds::icon_button(
                Some((ds::Icon::Scan, ds::TEXT_SECONDARY)),
                if self.scanning { "Scanning…" } else { "Scan" },
                None,
                style::standard,
                ds::CONTROL_HEIGHT,
                (!self.scanning).then_some(Msg::Scan),
            ),
        ]
        .spacing(6)
        .align_y(Alignment::Center);
        container(column![title, scan].spacing(8)).padding(12).into()
    }

    /// The connected TV's card: accent pill, green dot, details, ✕ on hover.
    fn connected_card<'a>(&'a self, session: &'a Session, reconnecting: bool) -> Element<'a, Msg> {
        let label = session.label();
        let (tag, line) = match session.android() {
            Some(s) => {
                // The shared platform name ("BRAVIA 4K VH2") next to the retail model.
                let tag = if s.summary.model != label { s.summary.model.clone() } else { String::new() };
                (tag, format!("Android {} · API {} · {}", s.summary.android_version, s.summary.sdk, s.summary.abi))
            }
            None => (String::new(), session.platform().name().to_owned()),
        };
        let (line, line_color) =
            if reconnecting { ("Connection lost · reconnecting…".to_owned(), ds::AMBER) } else { (line, ds::TEXT_TERTIARY) };
        let details = column![
            row![ds::medium(label, ds::TEXT_BASE, ds::TEXT), ds::mono(tag, ds::TEXT_XS, ds::TEXT_TERTIARY)]
                .spacing(6)
                .align_y(Alignment::Center),
            ds::mono(session.address().to_owned(), ds::TEXT_SM, ds::TEXT_SECONDARY),
            ds::label(line, ds::TEXT_XS, line_color),
        ]
        .spacing(2);
        let pill = container(container(Space::new()).width(3).height(Length::Fill).style(style::accent_bar))
            .padding(Padding::default().top(8).bottom(8))
            .height(Length::Fill);
        let body = row![container(ds::dot(if reconnecting { ds::AMBER } else { ds::GREEN }, 8.0)).padding(Padding::default().top(4)), details]
            .spacing(8)
            .padding(Padding { top: 10.0, right: 10.0, bottom: 10.0, left: 7.0 });
        let base = container(row![pill, body]).width(Length::Fill).style(style::panel(ds::CARD, ds::BORDER, ds::RADIUS));
        let close = container(ds::square_button(ds::Icon::X, ds::TEXT_SECONDARY, 22.0, style::ghost, Some(Msg::Disconnect)))
            .align_right(Length::Fill)
            .padding(6);
        hover(base, close)
    }

    fn device_row<'a>(&'a self, e: devices::Entry) -> Element<'a, Msg> {
        let dot = match e.reach {
            Reach::Ready => ds::GREEN,
            Reach::DebuggingOff | Reach::NeedsPairing => alpha(ds::AMBER, 0.8),
            Reach::Offline | Reach::Unknown => ds::TEXT_TERTIARY,
        };
        let serial = format!("{}:{}", e.ip, e.port);
        let mut details = column![
            ds::medium(e.title.clone(), ds::TEXT_BASE, ds::TEXT),
            ds::mono(serial.clone(), ds::TEXT_SM, ds::TEXT_SECONDARY),
            ds::label(e.status.clone(), ds::TEXT_XS, ds::TEXT_TERTIARY),
        ]
        .spacing(2);
        // Ready Android TVs can join batch actions.
        if e.reach == Reach::Ready && Platform::for_port(e.port) == Platform::AndroidTv {
            let ticked = self.batch.iter().any(|t| t.serial == serial);
            let (s, title) = (serial.clone(), e.title.clone());
            details = details.push(
                checkbox(ticked)
                    .label("Include in batch actions")
                    .on_toggle(move |_| Msg::ToggleBatch(s.clone(), title.clone()))
                    .size(12)
                    .text_size(ds::TEXT_XS)
                    .style(style::checkbox),
            );
        }
        let content = row![container(ds::dot(dot, 8.0)).padding(Padding::default().top(4)), details].spacing(8);
        let base = button(container(content).padding(Padding::from([10, 12])).width(Length::Fill))
            .padding(0)
            .width(Length::Fill)
            .style(style::row(false))
            .on_press_maybe((!self.connecting).then_some(Msg::Pick(e.ip, e.port)));
        let mut actions = row![].spacing(4).align_y(Alignment::Center);
        // Off TVs we know the MAC of can be woken up.
        if e.reach != Reach::Ready && history::mac(&self.history, &e.ip.to_string()).is_some() {
            actions = actions.push(
                button(ds::label("Wake", ds::TEXT_SM, ds::TEXT)).padding(Padding::from([3, 8])).style(style::standard).on_press(Msg::Wake(e.ip)),
            );
        }
        if e.saved {
            actions = actions.push(ds::square_button(ds::Icon::X, ds::TEXT_TERTIARY, 22.0, style::ghost, Some(Msg::Forget(e.ip))));
        }
        actions = actions.push(
            button(ds::label("Connect", ds::TEXT_SM, ds::TEXT))
                .padding(Padding::from([3, 8]))
                .style(style::standard)
                .on_press_maybe((!self.connecting).then_some(Msg::Pick(e.ip, e.port))),
        );
        hover(base, container(actions).align_right(Length::Fill).padding(8))
    }

    fn telemetry_card<'a>(&'a self, session: &'a Session, t: &'a Telemetry, reconnecting: bool) -> Element<'a, Msg> {
        let port = session.address().rsplit(':').next().unwrap_or("5555");
        let badge = if reconnecting {
            ds::tinted_badge("Reconnecting", ds::AMBER)
        } else {
            ds::tinted_badge(trf("Port {}: Open", &[&port]), ds::GREEN)
        };
        let header = row![
            ds::icon_any(ds::Icon::Activity, 14.0, ds::ACCENT),
            ds::medium("Live Telemetry", ds::TEXT_SM, ds::TEXT_SECONDARY),
            space::horizontal(),
            badge,
        ]
        .spacing(6)
        .align_y(Alignment::Center);

        let gauge = |name: &'a str, value: String, color: Color, fraction: f32, note: String| -> Element<'a, Msg> {
            container(
                column![
                    row![ds::label(name, 10.5, ds::TEXT_TERTIARY), space::horizontal(), ds::mono(value, ds::TEXT_BASE, color).font(ds::MONO_MEDIUM)]
                        .align_y(Alignment::End),
                    progress_bar(0.0..=1.0, fraction).girth(6).style(style::gauge(color)),
                    ds::mono(note, ds::TEXT_2XS, ds::TEXT_TERTIARY),
                ]
                .spacing(4),
            )
            .padding(8)
            .width(Length::Fill)
            .style(style::panel(ds::WELL, alpha(ds::BORDER, 0.6), ds::RADIUS))
            .into()
        };
        let cpu = t.cpu.unwrap_or(0.0);
        let ram = if t.ram_total > 0 { t.ram_used as f32 / t.ram_total as f32 } else { 0.0 };
        let gauges = row![
            gauge("CPU", t.cpu.map(|c| format!("{:.0}%", c * 100.0)).unwrap_or_else(|| "—".into()), ds::ACCENT, cpu, t.cpu_description.clone()),
            gauge(
                "RAM",
                if t.ram_total > 0 { format!("{} / {} GB", telemetry::gb(t.ram_used), telemetry::gb(t.ram_total)) } else { "—".into() },
                ds::PURPLE,
                ram,
                trf("{}% committed", &[&format!("{:.0}", ram * 100.0)]),
            ),
        ]
        .spacing(8);
        let footer = row![
            ds::dot(if reconnecting { ds::AMBER } else { ds::GREEN }, 6.0),
            ds::mono(t.latency_ms.map(|l| trf("Latency: {} ms", &[&format!("{l:.0}")])).unwrap_or_else(|| crate::i18n::tr("Latency: —").into()), ds::TEXT_XS, ds::TEXT_SECONDARY),
            space::horizontal(),
            ds::mono("Debugging authorized", ds::TEXT_XS, ds::TEXT_TERTIARY),
        ]
        .spacing(4)
        .align_y(Alignment::Center);
        container(column![header, gauges, ds::hline(alpha(ds::BORDER_SUBTLE, 0.6)), footer].spacing(8))
            .padding(10)
            .style(style::panel(ds::CARD_DIM, ds::BORDER_SUBTLE, ds::RADIUS))
            .into()
    }

    fn connect_box<'a>(&'a self, connected: bool) -> Element<'a, Msg> {
        let title = row![
            ds::medium("Direct ADB Connect", ds::TEXT_SM, ds::TEXT_SECONDARY),
            space::horizontal(),
            ds::mono(Platform::for_port(self.port.trim().parse().unwrap_or(android::ADB_PORT)).name(), ds::TEXT_XS, ds::TEXT_TERTIARY),
        ];
        let fields = row![
            ds::input("Host IP", &self.ip, true, ds::INSET).on_input(Msg::IpChanged).on_submit(Msg::Connect),
            ds::input("Port", &self.port, true, ds::INSET).on_input(Msg::PortChanged).on_submit(Msg::Connect).width(64),
        ]
        .spacing(6);
        let action: Element<'a, Msg> = if connected {
            ds::icon_button(Some((ds::Icon::Power, ds::RED)), "Disconnect Session", None, style::danger, ds::CONTROL_HEIGHT, Some(Msg::Disconnect))
                .width(Length::Fill)
                .into()
        } else {
            ds::icon_button(
                Some((ds::Icon::Plug, ds::ACCENT_TEXT)),
                if self.connecting { "Connecting…" } else { "Connect" },
                None,
                style::primary,
                ds::CONTROL_HEIGHT,
                (!self.connecting).then_some(Msg::Connect),
            )
            .width(Length::Fill)
            .into()
        };
        let mut col = column![title, fields, action].spacing(8);
        if let Some(hint) = &self.hint {
            col = col.push(ds::label(hint.as_str(), ds::TEXT_SM, ds::AMBER));
        }
        let pair_link = button(
            row![
                ds::icon_any(ds::Icon::QrCode, 12.0, if self.pair_open { ds::ACCENT } else { ds::TEXT_TERTIARY }),
                ds::label("Wireless Pairing Code (Android 11+)", ds::TEXT_SM, if self.pair_open { ds::ACCENT } else { ds::TEXT_SECONDARY }),
            ]
            .spacing(4)
            .align_y(Alignment::Center),
        )
        .padding(0)
        .style(style::link(ds::ACCENT))
        .on_press(Msg::TogglePair);
        col = col.push(container(pair_link).center_x(Length::Fill));
        if self.pair_open {
            col = col.push(self.pair_form());
        }
        container(col).padding(12).width(Length::Fill).style(style::fill(ds::SIDEBAR_LOW)).into()
    }

    /// Android 11+ wireless debugging: pair once with the code on the TV.
    fn pair_form(&self) -> Element<'_, Msg> {
        let ready = android::pairing::valid_code(&self.pair_code) && !self.pairing;
        column![
            ds::label("On the TV: Developer options → Wireless debugging → Pair device with pairing code.", ds::TEXT_XS, ds::TEXT_TERTIARY),
            row![
                ds::input("IP:pairing port", &self.pair_address, true, ds::INSET).on_input(Msg::PairAddress),
                ds::input("Code", &self.pair_code, true, ds::INSET).on_input(Msg::PairCode).on_submit(Msg::Pair).width(72),
            ]
            .spacing(6),
            ds::icon_button(
                Some((ds::Icon::Link, ds::TEXT_SECONDARY)),
                if self.pairing { "Pairing…" } else { "Pair" },
                None,
                style::standard,
                ds::CONTROL_HEIGHT,
                ready.then_some(Msg::Pair),
            )
            .width(Length::Fill),
        ]
        .spacing(6)
        .into()
    }

    fn footer(&self, online: bool, log_lines: usize) -> Element<'_, Msg> {
        let status = if online { ("Online", ds::GREEN) } else { ("Offline", ds::TEXT_TERTIARY) };
        let link = |caption: &'static str, msg: Msg| {
            button(ds::label(caption, ds::TEXT_XS, ds::TEXT_SECONDARY)).padding(0).style(style::link(ds::TEXT)).on_press(msg)
        };
        container(
            column![
                row![
                    ds::mono("ADB: built-in (native)", ds::TEXT_XS, ds::TEXT_SECONDARY),
                    space::horizontal(),
                    row![ds::dot(status.1, 6.0), ds::mono(status.0, ds::TEXT_XS, status.1)].spacing(4).align_y(Alignment::Center),
                ]
                .align_y(Alignment::Center),
                row![
                    ds::label(trf("Log: {} lines", &[&log_lines]), ds::TEXT_XS, ds::TEXT_TERTIARY),
                    space::horizontal(),
                    link("Import…", Msg::Import),
                    link("Diagnostics", Msg::Diagnostics),
                ]
                .spacing(10)
                .align_y(Alignment::Center),
            ]
            .spacing(4),
        )
        .padding(Padding::from([8, 10]))
        .width(Length::Fill)
        .style(style::fill(ds::FOOTER))
        .into()
    }

    fn update_notice(&self) -> Option<Element<'_, Msg>> {
        let release = self.update_available.as_ref()?;
        let (message, actions): (String, Element<'_, Msg>) = match self.update_state {
            UpdateState::Available => (
                trf("Version {} is available.", &[&release.version]),
                row![
                    ds::icon_button(
                        Some((ds::Icon::Download, ds::ACCENT_TEXT)),
                        if release.asset.is_some() { "Update now" } else { "Download" },
                        None,
                        style::primary,
                        24.0,
                        Some(if release.asset.is_some() { Msg::UpdateNow } else { Msg::OpenRelease }),
                    ),
                    button(ds::label("What's new", ds::TEXT_SM, ds::TEXT_SECONDARY)).padding(0).style(style::link(ds::ACCENT)).on_press(Msg::OpenRelease),
                ]
                .spacing(10)
                .align_y(Alignment::Center)
                .into(),
            ),
            UpdateState::Downloading => (trf("Downloading version {}…", &[&release.version]), Space::new().into()),
            UpdateState::Ready => (
                trf("Version {} is installed.", &[&release.version]),
                ds::icon_button(Some((ds::Icon::RefreshCw, ds::ACCENT_TEXT)), "Restart now", None, style::primary, 24.0, Some(Msg::Restart))
                    .into(),
            ),
        };
        Some(
            container(
                container(column![ds::label(message, ds::TEXT_SM, ds::ACCENT), actions].spacing(6))
                    .padding(Padding::from([8, 10]))
                    .width(Length::Fill)
                    .style(style::panel(alpha(ds::ACCENT, 0.08), alpha(ds::ACCENT, 0.3), ds::RADIUS)),
            )
            .padding(Padding::from([4, 8]))
            .into(),
        )
    }

    pub fn view<'a>(&'a self, cx: Context<'a>) -> Element<'a, Msg> {
        let now = chrono::Local::now().naive_local();
        let connected_ip = cx.session.map(|s| s.address().split(':').next().unwrap_or("").to_owned());
        let entries: Vec<devices::Entry> = devices::merge(&self.history, &self.open, &self.checked, &self.others, &self.mdns, now)
            .into_iter()
            .filter(|e| connected_ip.as_deref() != Some(e.ip.to_string().as_str()))
            .collect();
        let listed = entries.len() + usize::from(cx.session.is_some());

        let mut list = column![].spacing(4);
        if let Some(session) = cx.session {
            list = list.push(self.connected_card(session, cx.reconnecting));
        }
        if entries.is_empty() && cx.session.is_none() {
            let hint = if self.scanning || self.listening { "Looking for TVs…" } else { "No devices yet. Press Scan." };
            list = list.push(container(ds::label(hint, ds::TEXT_SM, ds::TEXT_TERTIARY)).padding(8));
        }
        for e in entries {
            list = list.push(self.device_row(e));
        }
        let mut body = column![list].spacing(8).padding(Padding::from([8, 8]));
        // Live telemetry needs ADB.
        if let Some(session) = cx.session.filter(|s| s.android().is_some()) {
            body = body.push(self.telemetry_card(session, cx.telemetry, cx.reconnecting));
        }

        let mut col = column![
            self.header(usize::from(cx.session.is_some()), listed),
            ds::hline(ds::BORDER_SUBTLE),
            scrollable(body).height(Length::Fill).direction(ds::thin_scrollbar()).style(style::scrollbar),
        ];
        if let Some(notice) = self.update_notice() {
            col = col.push(notice);
        }
        col = col
            .push(ds::hline(ds::BORDER))
            .push(self.connect_box(cx.session.is_some()))
            .push(ds::hline(ds::BORDER))
            .push(self.footer(cx.session.is_some() && !cx.reconnecting, cx.log_lines));

        row![
            container(col).width(ds::SIDEBAR_WIDTH).height(Length::Fill).style(style::fill(ds::SIDEBAR)),
            ds::vline_fill(),
        ]
        .into()
    }
}
