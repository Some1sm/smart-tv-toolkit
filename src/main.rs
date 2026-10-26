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
mod views;

use std::collections::BTreeSet;
use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::time::Duration;

use iced::widget::{button, column, container, row, rule, scrollable, space, text, text_input};
use iced::{Alignment, Element, Fill, Font, Length, Task, Theme};

use android::health::{self, Finding, Fix};
use android::packages::{self, Action, ListKind, Package};
use android::{actions, catalog, files, install, media, remote, shizuku};
use android::sysinfo::{self, Section};
use android::{ConnectResult, Connection};
use devices::Reach;
use history::History;
use mdns::MdnsDevice;
use platform::{Device, Platform};

/// How long each host gets to answer during a network scan.
const SCAN_TIMEOUT: Duration = Duration::from_millis(600);
/// How long to listen for mDNS announcements.
const MDNS_LISTEN: Duration = Duration::from_secs(3);

pub fn main() -> iced::Result {
    let window = iced::window::Settings {
        size: iced::Size::new(1100.0, 700.0),
        // Below this the sidebar, tab bar and log no longer fit.
        min_size: Some(iced::Size::new(900.0, 560.0)),
        position: iced::window::Position::Centered,
        icon: iced::window::icon::from_file_data(include_bytes!("../assets/icon-256.png"), None).ok(),
        ..Default::default()
    };
    iced::application(App::new, App::update, App::view)
        .title(App::title)
        .theme(App::theme)
        .subscription(App::subscription)
        .window(window)
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
    info: Option<Vec<Section>>,
    info_loading: bool,
    health: Health,
    fixing: bool,
    /// Android API level of the connected TV (gates per-app Clear Cache).
    sdk: u32,
    pkg_kind: Option<ListKind>,
    packages: Vec<Package>,
    pkg_filter: String,
    pkg_selected: BTreeSet<String>,
    pkg_loading: bool,
    pkg_busy: bool,
    /// CPU types of the connected TV, preferred first.
    abis: Vec<String>,
    downloading: BTreeSet<&'static str>,
    install_busy: bool,
    /// Last screenshot: decoded handle for display, PNG bytes for saving.
    screenshot: Option<(iced::widget::image::Handle, Vec<u8>)>,
    screenshot_status: String,
    capturing: bool,
    settings_now: Option<actions::Current>,
    density_input: String,
    tv_busy: bool,
    launch_input: String,
    text_input: String,
    console_input: String,
    recording: views::Recording,
    files_cwd: String,
    files_path_input: String,
    files_entries: Option<Result<Vec<files::Entry>, String>>,
    files_selected: Option<String>,
    files_busy: bool,
    tab: Tab,
    log: Vec<String>,
}

enum Health {
    Idle,
    Scanning,
    Failed(String),
    Done { findings: Vec<Finding>, scanned_at: String },
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
    RefreshInfo,
    InfoLoaded(Result<Vec<Section>, String>),
    CopyInfo,
    RunHealthCheck,
    HealthScanned(Result<Vec<Finding>, String>),
    FixPressed(Fix),
    FixConfirmed(Fix, bool),
    FixApplied(Result<String, String>),
    ListPackages(ListKind),
    PackagesLoaded(ListKind, Result<Vec<Package>, String>),
    PackageFilter(String),
    TogglePackage(String),
    SelectShown,
    ClearSelection,
    PackageAction(Action),
    PackageActionConfirmed(Action, Vec<String>, bool),
    PackageActionDone(Vec<String>),
    SaveApk,
    SaveApkTo(Vec<String>, Option<PathBuf>),
    PickApks,
    PickApkFolder,
    InstallApks(Vec<PathBuf>),
    InstallDone(Vec<String>),
    QuickDownload(&'static str),
    QuickDownloaded(&'static str, Result<String, String>),
    QuickInstall(&'static str),
    ShizukuInstall,
    ShizukuStart,
    InstallResult(Result<String, String>),
    OpenUrl(&'static str),
    RemoteKey(remote::Key),
    RemoteDone(Result<(), String>),
    TakeScreenshot,
    ScreenshotTaken(Result<Vec<u8>, String>),
    SaveScreenshot,
    ScreenshotSaveTo(Option<PathBuf>),
    TvAction(actions::Action),
    TvActionConfirmed(actions::Action, bool),
    TvActionDone(actions::Action, Result<String, String>),
    LoadSettings,
    SettingsLoaded(Result<actions::Current, String>),
    DensityInput(String),
    ApplyDensity,
    LaunchInput(String),
    LaunchSubmit,
    SendTextInput(String),
    SendTextSubmit,
    ConsoleInput(String),
    ConsoleSubmit,
    LaunchScrcpy,
    RecordStart,
    RecordStop,
    RecordEnded(Result<(), String>),
    RecordSaveTo(Option<PathBuf>),
    RecordSaved(Result<String, String>),
    FilesPathInput(String),
    FilesGo,
    FilesUp,
    FilesRefresh,
    FilesListed(String, Result<Vec<files::Entry>, String>),
    FileClicked(String, bool),
    FilesDownload,
    FilesDownloadTo(Option<PathBuf>),
    FilesSend,
    FilesSendFile(Option<PathBuf>),
    FilesDone(Vec<String>),
    Ignore,
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
            info: None,
            info_loading: false,
            health: Health::Idle,
            fixing: false,
            sdk: 0,
            pkg_kind: None,
            packages: Vec::new(),
            pkg_filter: String::new(),
            pkg_selected: BTreeSet::new(),
            pkg_loading: false,
            pkg_busy: false,
            abis: Vec::new(),
            downloading: BTreeSet::new(),
            install_busy: false,
            screenshot: None,
            screenshot_status: String::new(),
            capturing: false,
            settings_now: None,
            density_input: String::new(),
            tv_busy: false,
            launch_input: String::new(),
            text_input: String::new(),
            console_input: String::new(),
            recording: views::Recording::Idle,
            files_cwd: "/sdcard/".into(),
            files_path_input: "/sdcard/".into(),
            files_entries: None,
            files_selected: None,
            files_busy: false,
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

    /// While the Remote tab is open, the computer's keyboard drives the TV.
    /// Only keys no widget used arrive here, so typing in a field is safe.
    fn subscription(&self) -> iced::Subscription<Message> {
        if self.tab == Tab::Remote && self.connection.is_some() {
            iced::keyboard::listen().filter_map(views::remote_key)
        } else {
            iced::Subscription::none()
        }
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
        self.info = None;
        self.health = Health::Idle;
        self.pkg_kind = None;
        self.packages.clear();
        self.pkg_selected.clear();
        self.settings_now = None;
        self.files_entries = None;
        self.files_selected = None;
        self.recording = views::Recording::Idle;
        self.tab = Tab::Info;
    }

    fn load_settings(&mut self) -> Task<Message> {
        let Some(conn) = self.connection.clone() else { return Task::none() };
        Task::perform(actions::read_current(conn), Message::SettingsLoaded)
    }

    fn list_files(&mut self, dir: String) -> Task<Message> {
        let Some(conn) = self.connection.clone() else { return Task::none() };
        let dir = files::as_dir(&dir);
        self.files_cwd = dir.clone();
        self.files_path_input = dir.clone();
        self.files_entries = None;
        self.files_selected = None;
        Task::perform(files::list(conn, dir.clone()), move |r| Message::FilesListed(dir.clone(), r))
    }

    fn list_packages(&mut self, kind: ListKind) -> Task<Message> {
        let Some(conn) = self.connection.clone() else { return Task::none() };
        self.pkg_kind = Some(kind);
        self.pkg_loading = true;
        Task::perform(packages::list(conn, kind), move |r| Message::PackagesLoaded(kind, r))
    }

    fn load_info(&mut self) -> Task<Message> {
        let Some(conn) = self.connection.clone() else { return Task::none() };
        self.info_loading = true;
        Task::perform(sysinfo::collect(conn), Message::InfoLoaded)
    }

    fn run_health_check(&mut self) -> Task<Message> {
        let Some(conn) = self.connection.clone() else { return Task::none() };
        self.health = Health::Scanning;
        Task::perform(health::scan(conn), Message::HealthScanned)
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
                        self.sdk = summary.sdk.parse().unwrap_or(0);
                        self.abis = summary.abis.clone();
                        self.connection = Some(connection);
                        self.device = Some(Device { platform: Platform::AndroidTv, address: serial, label });
                        return self.load_info();
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
            Message::TabSelected(tab) => {
                self.tab = tab;
                // Show live values whenever these tabs are opened.
                match tab {
                    Tab::Settings => return self.load_settings(),
                    Tab::Media if self.files_entries.is_none() => return self.list_files(self.files_cwd.clone()),
                    _ => {}
                }
            }
            Message::RefreshInfo => return self.load_info(),
            Message::InfoLoaded(result) => {
                self.info_loading = false;
                match result {
                    // Ignore a late answer from a TV we've since left.
                    Ok(sections) if self.connection.is_some() => self.info = Some(sections),
                    Ok(_) => {}
                    Err(e) => self.log(format!("Could not read device info: {e}")),
                }
            }
            Message::CopyInfo => {
                if let Some(text) = self.info.as_deref().map(sysinfo::to_text) {
                    self.log("Device info copied to the clipboard.");
                    return iced::clipboard::write(text);
                }
            }
            Message::RunHealthCheck => {
                self.log("Running Health Check (read-only)…");
                return self.run_health_check();
            }
            Message::HealthScanned(result) => {
                if self.connection.is_none() {
                    return Task::none();
                }
                self.health = match result {
                    Ok(findings) => {
                        let scanned_at = chrono::Local::now().format("%H:%M").to_string();
                        Health::Done { findings, scanned_at }
                    }
                    Err(e) => {
                        self.log(format!("Health Check failed: {e}"));
                        Health::Failed(e)
                    }
                };
            }
            Message::FixPressed(fix) => match fix.confirmation() {
                Some((title, question)) => {
                    let dialog = rfd::AsyncMessageDialog::new()
                        .set_level(rfd::MessageLevel::Warning)
                        .set_title(title)
                        .set_description(question)
                        .set_buttons(rfd::MessageButtons::YesNo);
                    return Task::perform(dialog.show(), move |answer| {
                        Message::FixConfirmed(fix.clone(), answer == rfd::MessageDialogResult::Yes)
                    });
                }
                None => return self.update(Message::FixConfirmed(fix, true)),
            },
            Message::FixConfirmed(_, false) => {}
            Message::FixConfirmed(fix, true) => {
                let Some(conn) = self.connection.clone() else { return Task::none() };
                self.log(format!("{}…", fix.label()));
                self.fixing = true;
                return Task::perform(health::apply(conn, fix), Message::FixApplied);
            }
            Message::FixApplied(result) => {
                self.fixing = false;
                match result {
                    Ok(line) => self.log(line),
                    Err(e) => self.log(format!("Fix failed: {e}")),
                }
                // Show the effect straight away, if a scan is on screen.
                if !matches!(self.health, Health::Idle) {
                    return self.run_health_check();
                }
            }
            Message::ListPackages(kind) => {
                self.log(format!("Listing {} packages…", kind.label().to_lowercase()));
                return self.list_packages(kind);
            }
            Message::PackagesLoaded(kind, result) => {
                if self.pkg_kind != Some(kind) {
                    return Task::none(); // a newer list was requested meanwhile
                }
                self.pkg_loading = false;
                match result {
                    Ok(list) if self.connection.is_some() => {
                        self.log(format!("{} {} package(s).", list.len(), kind.label()));
                        let ids: BTreeSet<&String> = list.iter().map(|p| &p.id).collect();
                        self.pkg_selected.retain(|p| ids.contains(p));
                        self.packages = list;
                    }
                    Ok(_) => {}
                    Err(e) => self.log(format!("Could not list packages: {e}")),
                }
            }
            Message::PackageFilter(filter) => self.pkg_filter = filter,
            Message::TogglePackage(pkg) => {
                if !self.pkg_selected.remove(&pkg) {
                    self.pkg_selected.insert(pkg);
                }
            }
            Message::SelectShown => {
                let shown: Vec<String> = views::visible(&self.packages, &self.pkg_filter).map(|p| p.id.clone()).collect();
                self.pkg_selected.extend(shown);
            }
            Message::ClearSelection => self.pkg_selected.clear(),
            Message::PackageAction(action) => {
                let pkgs: Vec<String> = self.pkg_selected.iter().cloned().collect();
                match action.confirmation(&pkgs) {
                    Some((title, question)) => {
                        let dialog = rfd::AsyncMessageDialog::new()
                            .set_level(rfd::MessageLevel::Warning)
                            .set_title(title)
                            .set_description(question)
                            .set_buttons(rfd::MessageButtons::YesNo);
                        return Task::perform(dialog.show(), move |answer| {
                            Message::PackageActionConfirmed(action, pkgs.clone(), answer == rfd::MessageDialogResult::Yes)
                        });
                    }
                    None => return self.update(Message::PackageActionConfirmed(action, pkgs, true)),
                }
            }
            Message::PackageActionConfirmed(_, _, false) => {}
            Message::PackageActionConfirmed(action, pkgs, true) => {
                let Some(conn) = self.connection.clone() else { return Task::none() };
                self.pkg_busy = true;
                return Task::perform(packages::run(conn, action, pkgs), Message::PackageActionDone);
            }
            Message::PackageActionDone(lines) => {
                self.pkg_busy = false;
                for line in lines {
                    self.log(line);
                }
                // Reload so uninstalled/disabled state shows.
                if let Some(kind) = self.pkg_kind {
                    return self.list_packages(kind);
                }
            }
            Message::SaveApk => {
                let pkgs: Vec<String> = self.pkg_selected.iter().cloned().collect();
                let title = match pkgs.as_slice() {
                    [one] => format!("Save APK for {one}: choose a folder"),
                    many => format!("Save APKs for {} packages: choose a folder", many.len()),
                };
                return Task::perform(
                    async move {
                        let dir = rfd::AsyncFileDialog::new().set_title(title).pick_folder().await;
                        (pkgs, dir.map(|d| d.path().to_path_buf()))
                    },
                    |(pkgs, dir)| Message::SaveApkTo(pkgs, dir),
                );
            }
            Message::SaveApkTo(_, None) => {}
            Message::SaveApkTo(pkgs, Some(dir)) => {
                let Some(conn) = self.connection.clone() else { return Task::none() };
                self.log(format!("Saving {} APK(s) to {}…", pkgs.len(), dir.display()));
                self.pkg_busy = true;
                return Task::perform(packages::save_apks(conn, pkgs, dir), |lines| {
                    Message::PackageActionDone(lines)
                });
            }
            Message::PickApks => {
                return Task::perform(
                    async {
                        rfd::AsyncFileDialog::new()
                            .set_title("Install APK files")
                            .add_filter("Android app", &["apk"])
                            .pick_files()
                            .await
                            .map(|files| files.iter().map(|f| f.path().to_path_buf()).collect())
                            .unwrap_or_default()
                    },
                    Message::InstallApks,
                );
            }
            Message::PickApkFolder => {
                return Task::perform(
                    async {
                        let dir = rfd::AsyncFileDialog::new().set_title("Folder with APK files").pick_folder().await;
                        dir.map(|d| install::apks_in(d.path())).unwrap_or_default()
                    },
                    Message::InstallApks,
                );
            }
            Message::InstallApks(apks) => {
                if apks.is_empty() {
                    return Task::none();
                }
                let Some(conn) = self.connection.clone() else { return Task::none() };
                self.log(format!("Installing {} APK(s)…", apks.len()));
                self.install_busy = true;
                return Task::perform(install::install_many(conn, apks), Message::InstallDone);
            }
            Message::InstallDone(lines) => {
                self.install_busy = false;
                for line in lines {
                    self.log(line);
                }
            }
            Message::QuickDownload(name) => {
                let Some(spec) = catalog::APPS.iter().chain([&catalog::SHIZUKU]).find(|a| a.name == name).copied() else {
                    return Task::none();
                };
                self.log(format!("Downloading {name}…"));
                self.downloading.insert(name);
                return Task::perform(catalog::fetch(spec, self.abis.clone()), move |r| Message::QuickDownloaded(name, r));
            }
            Message::QuickDownloaded(name, result) => {
                self.downloading.remove(name);
                match result {
                    Ok(line) => self.log(line),
                    Err(e) => self.log(format!("Download failed for {name}: {e}")),
                }
            }
            Message::QuickInstall(name) => {
                let Some(spec) = catalog::APPS.iter().find(|a| a.name == name) else { return Task::none() };
                let (Some(conn), Some(apk)) = (self.connection.clone(), catalog::download_path(spec, &self.abis)) else {
                    return Task::none();
                };
                self.log(format!("Installing {name}…"));
                self.install_busy = true;
                return Task::perform(install::install_with(conn, apk, spec.post_install), move |r| {
                    Message::InstallResult(r.map(|out| format!("{name}: {out}")).map_err(|e| format!("{name}: {e}")))
                });
            }
            Message::ShizukuInstall | Message::ShizukuStart => {
                let Some(conn) = self.connection.clone() else { return Task::none() };
                self.install_busy = true;
                return if matches!(message, Message::ShizukuInstall) {
                    self.log("Installing and starting Shizuku…");
                    Task::perform(shizuku::install_and_start(conn), Message::InstallResult)
                } else {
                    self.log("Starting Shizuku…");
                    Task::perform(shizuku::start(conn), Message::InstallResult)
                };
            }
            Message::InstallResult(result) => {
                self.install_busy = false;
                match result {
                    Ok(line) => self.log(line),
                    Err(e) => self.log(format!("Install failed: {e}")),
                }
            }
            Message::OpenUrl(url) => {
                if let Err(e) = open::that(url) {
                    self.log(format!("Could not open the browser: {e}"));
                }
            }
            Message::RemoteKey(key) => {
                let Some(conn) = self.connection.clone() else { return Task::none() };
                return Task::perform(remote::press(conn, key), Message::RemoteDone);
            }
            Message::RemoteDone(Ok(())) => {}
            Message::RemoteDone(Err(e)) => self.log(format!("Remote: {e}")),
            Message::TakeScreenshot => {
                let Some(conn) = self.connection.clone() else { return Task::none() };
                self.capturing = true;
                self.screenshot_status = "Capturing…".into();
                return Task::perform(remote::screenshot(conn), Message::ScreenshotTaken);
            }
            Message::ScreenshotTaken(result) => {
                self.capturing = false;
                match result {
                    Ok(png) => {
                        let time = chrono::Local::now().format("%H:%M:%S");
                        self.screenshot_status = format!("Captured at {time} ({}).", sysinfo::fmt_bytes(png.len() as u64));
                        self.screenshot = Some((iced::widget::image::Handle::from_bytes(png.clone()), png));
                    }
                    Err(e) => {
                        self.screenshot_status = "Capture failed; see the log.".into();
                        self.log(format!("Screenshot failed: {e}"));
                    }
                }
            }
            Message::SaveScreenshot => {
                let name = format!("screenshot-{}.png", chrono::Local::now().format("%Y%m%d-%H%M%S"));
                return Task::perform(
                    async move {
                        rfd::AsyncFileDialog::new()
                            .set_title("Save screenshot")
                            .set_file_name(name)
                            .add_filter("PNG image", &["png"])
                            .save_file()
                            .await
                            .map(|f| f.path().to_path_buf())
                    },
                    Message::ScreenshotSaveTo,
                );
            }
            Message::ScreenshotSaveTo(None) => {}
            Message::ScreenshotSaveTo(Some(path)) => {
                if let Some((_, png)) = &self.screenshot {
                    let result = std::fs::write(&path, png);
                    self.log(match result {
                        Ok(()) => format!("Saved {}", path.display()),
                        Err(e) => format!("Could not save {}: {e}", path.display()),
                    });
                }
            }
            Message::Ignore => {}
            // ── Settings & Tools ──
            Message::TvAction(action) => match action.confirmation() {
                Some((title, question)) => {
                    let dialog = rfd::AsyncMessageDialog::new()
                        .set_level(rfd::MessageLevel::Warning)
                        .set_title(title)
                        .set_description(question)
                        .set_buttons(rfd::MessageButtons::YesNo);
                    return Task::perform(dialog.show(), move |answer| {
                        Message::TvActionConfirmed(action.clone(), answer == rfd::MessageDialogResult::Yes)
                    });
                }
                None => return self.update(Message::TvActionConfirmed(action, true)),
            },
            Message::TvActionConfirmed(_, false) => {}
            Message::TvActionConfirmed(action, true) => {
                let Some(conn) = self.connection.clone() else { return Task::none() };
                if let actions::Action::Console(cmd) = &action {
                    self.log(format!("$ {}", actions::console_command(cmd)));
                }
                self.tv_busy = true;
                return Task::perform(actions::run(conn, action.clone()), move |r| Message::TvActionDone(action.clone(), r));
            }
            Message::TvActionDone(action, result) => {
                self.tv_busy = false;
                match result {
                    Ok(out) => self.log(out),
                    Err(e) => self.log(format!("Failed: {e}")),
                }
                if action.is_reboot() {
                    self.disconnect();
                } else if self.tab == Tab::Settings {
                    return self.load_settings();
                }
            }
            Message::LoadSettings => return self.load_settings(),
            Message::SettingsLoaded(result) => match result {
                Ok(now) if self.connection.is_some() => self.settings_now = Some(now),
                Ok(_) => {}
                Err(e) => self.log(format!("Could not read settings: {e}")),
            },
            Message::DensityInput(v) => self.density_input = v,
            Message::ApplyDensity => match self.density_input.trim().parse::<u32>() {
                Ok(d) if (120..=640).contains(&d) => return self.update(Message::TvAction(actions::Action::Density(d))),
                _ => self.log("Enter a density between 120 and 640 (TVs usually use 213, 240 or 320)."),
            },
            Message::LaunchInput(v) => self.launch_input = v,
            Message::LaunchSubmit => {
                let pkg = self.launch_input.trim().to_owned();
                if !pkg.is_empty() {
                    return self.update(Message::TvAction(actions::Action::LaunchApp(pkg)));
                }
            }
            Message::SendTextInput(v) => self.text_input = v,
            Message::SendTextSubmit => {
                let text = std::mem::take(&mut self.text_input);
                if !text.is_empty() {
                    return self.update(Message::TvAction(actions::Action::SendText(text)));
                }
            }
            Message::ConsoleInput(v) => self.console_input = v,
            Message::ConsoleSubmit => {
                let cmd = std::mem::take(&mut self.console_input);
                if !cmd.trim().is_empty() {
                    return self.update(Message::TvAction(actions::Action::Console(cmd)));
                }
            }
            // ── Media ──
            Message::LaunchScrcpy => {
                if let Some(device) = &self.device {
                    let line = media::launch_scrcpy(&device.address).unwrap_or_else(|e| e);
                    self.log(line);
                }
            }
            Message::RecordStart => {
                let Some(address) = self.device.as_ref().map(|d| d.address.clone()) else { return Task::none() };
                self.recording = views::Recording::Recording;
                self.log("Recording the screen…");
                return Task::perform(media::record(address), Message::RecordEnded);
            }
            Message::RecordStop => {
                let Some(conn) = self.connection.clone() else { return Task::none() };
                self.recording = views::Recording::Stopping;
                return Task::perform(media::stop(conn), |_| Message::Ignore);
            }
            Message::RecordEnded(result) => {
                self.recording = views::Recording::Stopping;
                if let Err(e) = result {
                    self.log(format!("Recording failed: {}", e.replace('\n', " ")));
                    return self.update(Message::RecordSaveTo(None));
                }
                let name = format!("recording-{}.mp4", chrono::Local::now().format("%Y%m%d-%H%M%S"));
                return Task::perform(
                    async move {
                        rfd::AsyncFileDialog::new()
                            .set_title("Save recording")
                            .set_file_name(name)
                            .add_filter("MP4 video", &["mp4"])
                            .save_file()
                            .await
                            .map(|f| f.path().to_path_buf())
                    },
                    Message::RecordSaveTo,
                );
            }
            Message::RecordSaveTo(dest) => {
                let Some(conn) = self.connection.clone() else {
                    self.recording = views::Recording::Idle;
                    return Task::none();
                };
                return Task::perform(media::save_recording(conn, dest), Message::RecordSaved);
            }
            Message::RecordSaved(result) => {
                self.recording = views::Recording::Idle;
                self.log(result.unwrap_or_else(|e| format!("Could not save the recording: {e}")));
            }
            Message::FilesPathInput(v) => self.files_path_input = v,
            Message::FilesGo => return self.list_files(self.files_path_input.clone()),
            Message::FilesUp => return self.list_files(files::parent(&self.files_cwd)),
            Message::FilesRefresh => return self.list_files(self.files_cwd.clone()),
            Message::FilesListed(dir, result) => {
                if dir == self.files_cwd && self.connection.is_some() {
                    self.files_entries = Some(result);
                }
            }
            Message::FileClicked(name, true) => return self.list_files(format!("{}{name}", self.files_cwd)),
            Message::FileClicked(name, false) => {
                let full = format!("{}{name}", self.files_cwd);
                // Clicking the selected file again deselects it (back to the folder).
                self.files_selected = if self.files_selected.as_deref() == Some(full.as_str()) { None } else { Some(full) };
            }
            Message::FilesDownload => {
                return Task::perform(
                    async {
                        rfd::AsyncFileDialog::new().set_title("Download to folder").pick_folder().await.map(|d| d.path().to_path_buf())
                    },
                    Message::FilesDownloadTo,
                );
            }
            Message::FilesDownloadTo(None) => {}
            Message::FilesDownloadTo(Some(dest)) => {
                let Some(conn) = self.connection.clone() else { return Task::none() };
                let target = self.files_selected.clone().unwrap_or_else(|| self.files_cwd.clone());
                self.log(format!("Downloading {target}…"));
                self.files_busy = true;
                return Task::perform(files::download(conn, target, dest), Message::FilesDone);
            }
            Message::FilesSend => {
                return Task::perform(
                    async {
                        rfd::AsyncFileDialog::new().set_title("Send a file to the TV").pick_file().await.map(|f| f.path().to_path_buf())
                    },
                    Message::FilesSendFile,
                );
            }
            Message::FilesSendFile(None) => {}
            Message::FilesSendFile(Some(local)) => {
                let Some(conn) = self.connection.clone() else { return Task::none() };
                self.files_busy = true;
                let dir = self.files_cwd.clone();
                return Task::perform(files::send(conn, local, dir), |r| {
                    Message::FilesDone(vec![r.unwrap_or_else(|e| format!("Send failed: {e}"))])
                });
            }
            Message::FilesDone(lines) => {
                self.files_busy = false;
                for line in lines {
                    self.log(line);
                }
                return self.list_files(self.files_cwd.clone());
            }
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
            Tab::Info => views::info(self.info.as_deref(), self.info_loading),
            Tab::Performance => {
                let state = match &self.health {
                    Health::Idle => views::HealthState::Idle,
                    Health::Scanning => views::HealthState::Scanning,
                    Health::Failed(e) => views::HealthState::Failed(e),
                    Health::Done { findings, scanned_at } => views::HealthState::Done { findings, scanned_at },
                };
                views::performance(state, self.fixing)
            }
            Tab::Remote => views::remote(),
            Tab::Settings => views::settings(self.settings_now.as_ref(), &self.density_input, self.tv_busy),
            Tab::Tools => views::tools(views::ToolsView {
                launch: &self.launch_input,
                text: &self.text_input,
                console: &self.console_input,
                busy: self.tv_busy,
            }),
            Tab::Media => views::media(views::MediaView {
                recording: self.recording,
                scrcpy_found: media::find_scrcpy().is_some(),
                cwd: &self.files_cwd,
                path_input: &self.files_path_input,
                entries: self.files_entries.as_ref(),
                selected: self.files_selected.as_deref(),
                busy: self.files_busy,
            }),
            Tab::Screenshot => {
                views::screenshot(self.screenshot.as_ref().map(|(h, _)| h), &self.screenshot_status, self.capturing)
            }
            Tab::Install => views::install(views::InstallView {
                abis: &self.abis,
                downloading: &self.downloading,
                busy: self.install_busy,
            }),
            Tab::Packages => views::packages(views::PackagesView {
                kind: self.pkg_kind,
                packages: &self.packages,
                filter: &self.pkg_filter,
                selected: &self.pkg_selected,
                loading: self.pkg_loading,
                busy: self.pkg_busy,
                sdk: self.sdk,
            }),
        };
        container(content).width(Fill).height(Fill).into()
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

