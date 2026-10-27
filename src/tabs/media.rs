//! Media: scrcpy, screen recording and the file browser.

use std::path::PathBuf;

use iced::widget::{button, column, container, row, scrollable, text_input};
use iced::{Alignment, Element, Fill, Task};

use super::{Ctx, Event, View};
use crate::ui::text;
use crate::android::{files, media};
use crate::jobs::{JobId, Unit};
use crate::prefs::Prefs;
use crate::design::{self as ds, style};
use crate::{scrcpy, ui};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Recording {
    #[default]
    Idle,
    Recording,
    Stopping,
}

#[derive(Debug)]
pub struct State {
    play_url: String,
    scrcpy: Option<PathBuf>,
    scrcpy_version: Option<String>,
    downloading_scrcpy: bool,
    recording: Recording,
    cwd: String,
    path_input: String,
    entries: Option<Result<Vec<files::Entry>, String>>,
    selected: Option<String>,
    busy: bool,
}

#[derive(Debug, Clone)]
pub enum Msg {
    LocateScrcpy,
    ScrcpyPicked(Option<PathBuf>),
    DownloadScrcpy,
    ScrcpyDownloaded(JobId, Result<PathBuf, String>),
    ScrcpyVersion(Option<String>),
    LaunchScrcpy,
    OpenUrl(&'static str),
    RecordStart,
    RecordStop,
    RecordStopped,
    RecordEnded(Result<(), String>),
    RecordSaveTo(Option<PathBuf>),
    RecordSaved(JobId, Result<Result<String, String>, String>),
    PathInput(String),
    Go,
    Up,
    Refresh,
    Listed(String, Result<Vec<files::Entry>, String>),
    Clicked(String, bool),
    Download,
    DownloadTo(Option<PathBuf>),
    Send,
    SendFile(Option<PathBuf>),
    Done(JobId, Result<Vec<String>, String>),
    PlayUrl(String),
    Play,
    PlayFile,
    PlayFilePicked(Option<PathBuf>),
    Played(Result<String, String>),
    PlayedFile(JobId, Result<Result<String, String>, String>),
}

const START_DIR: &str = "/sdcard/";

impl State {
    pub fn new(prefs: &Prefs) -> (Self, Task<Msg>) {
        let scrcpy = scrcpy::find(prefs.scrcpy.as_deref());
        let task = match scrcpy.clone() {
            Some(exe) => Task::perform(scrcpy::version(exe), Msg::ScrcpyVersion),
            None => Task::none(),
        };
        let state = State {
            play_url: String::new(),
            scrcpy,
            scrcpy_version: None,
            downloading_scrcpy: false,
            recording: Recording::Idle,
            cwd: START_DIR.into(),
            path_input: START_DIR.into(),
            entries: None,
            selected: None,
            busy: false,
        };
        (state, task)
    }

    pub fn reset(&mut self) {
        self.recording = Recording::Idle;
        self.entries = None;
        self.selected = None;
        self.busy = false;
    }

    /// List the TV's files the first time the tab is opened.
    pub fn on_open(&mut self, ctx: &mut Ctx) -> Task<Msg> {
        if self.entries.is_none() { self.list(self.cwd.clone(), ctx) } else { Task::none() }
    }

    fn list(&mut self, dir: String, ctx: &mut Ctx) -> Task<Msg> {
        let Some(conn) = ctx.conn() else { return Task::none() };
        let dir = files::as_dir(&dir);
        self.cwd = dir.clone();
        self.path_input = dir.clone();
        self.entries = None;
        self.selected = None;
        Task::perform(files::list(conn, dir.clone()), move |r| Msg::Listed(dir.clone(), r))
    }

    fn set_scrcpy(&mut self, exe: PathBuf, ctx: &mut Ctx) -> Task<Msg> {
        ctx.log(format!("Using scrcpy at {}", exe.display()));
        ctx.prefs.scrcpy = Some(exe.clone());
        ctx.events.push(Event::SavePrefs);
        self.scrcpy = Some(exe.clone());
        self.scrcpy_version = None;
        Task::perform(scrcpy::version(exe), Msg::ScrcpyVersion)
    }

    pub fn update(&mut self, msg: Msg, ctx: &mut Ctx) -> Task<Msg> {
        match msg {
            Msg::LocateScrcpy => {
                let mut dialog = rfd::AsyncFileDialog::new().set_title("Locate scrcpy");
                if cfg!(windows) {
                    dialog = dialog.add_filter("scrcpy", &["exe"]);
                }
                if let Some(dir) = self.scrcpy.as_ref().and_then(|p| p.parent()) {
                    dialog = dialog.set_directory(dir);
                }
                return Task::perform(async move { dialog.pick_file().await.map(|f| f.path().to_path_buf()) }, Msg::ScrcpyPicked);
            }
            Msg::ScrcpyPicked(None) => {}
            Msg::ScrcpyPicked(Some(path)) => match scrcpy::resolve(&path) {
                Some(exe) => return self.set_scrcpy(exe, ctx),
                None => ctx.log(format!("No scrcpy at {}", path.display())),
            },
            Msg::DownloadScrcpy => {
                self.downloading_scrcpy = true;
                ctx.log("Downloading scrcpy…");
                let (id, progress) = ctx.jobs.start("Downloading scrcpy", Unit::Bytes);
                return Task::perform(scrcpy::download(progress), move |r| Msg::ScrcpyDownloaded(id, r));
            }
            Msg::ScrcpyDownloaded(id, result) => {
                ctx.jobs.finish(id);
                self.downloading_scrcpy = false;
                match result {
                    Ok(exe) => return self.set_scrcpy(exe, ctx),
                    Err(e) => ctx.log(format!("scrcpy download failed: {e}")),
                }
            }
            Msg::ScrcpyVersion(v) => self.scrcpy_version = v,
            Msg::LaunchScrcpy => {
                if let (Some(exe), Some(session)) = (&self.scrcpy, ctx.session) {
                    let line = scrcpy::launch(exe, session.address()).unwrap_or_else(|e| e);
                    ctx.log(line);
                }
            }
            Msg::OpenUrl(url) => {
                if let Err(e) = open::that(url) {
                    ctx.log(format!("Could not open the browser: {e}"));
                }
            }
            Msg::RecordStart => {
                let Some(serial) = ctx.android().map(|s| s.serial) else { return Task::none() };
                self.recording = Recording::Recording;
                ctx.log("Recording the screen…");
                return Task::perform(media::record(serial), Msg::RecordEnded);
            }
            Msg::RecordStop => {
                let Some(conn) = ctx.conn() else { return Task::none() };
                self.recording = Recording::Stopping;
                return Task::perform(media::stop(conn), |_| Msg::RecordStopped);
            }
            Msg::RecordStopped => {}
            Msg::RecordEnded(result) => {
                self.recording = Recording::Stopping;
                if let Err(e) = result {
                    ctx.log(format!("Recording failed: {}", e.replace('\n', " ")));
                    return self.update(Msg::RecordSaveTo(None), ctx);
                }
                let save = ui::save_file("Save recording", ui::timestamped("recording", "mp4"), ("MP4 video", &["mp4"]));
                return Task::perform(save, Msg::RecordSaveTo);
            }
            Msg::RecordSaveTo(dest) => {
                if ctx.session.is_none() {
                    self.recording = Recording::Idle;
                    return Task::none();
                }
                return ctx.job("Saving recording", Unit::Bytes, move |conn, p| media::save_recording(conn, dest, p), Msg::RecordSaved);
            }
            Msg::RecordSaved(id, result) => {
                ctx.jobs.finish(id);
                self.recording = Recording::Idle;
                ctx.log(result.and_then(|r| r).unwrap_or_else(|e| format!("Could not save the recording: {e}")));
            }
            Msg::PathInput(v) => self.path_input = v,
            Msg::Go => return self.list(self.path_input.clone(), ctx),
            Msg::Up => return self.list(files::parent(&self.cwd), ctx),
            Msg::Refresh => return self.list(self.cwd.clone(), ctx),
            Msg::Listed(dir, result) => {
                if dir == self.cwd && ctx.session.is_some() {
                    self.entries = Some(result);
                }
            }
            Msg::Clicked(name, true) => return self.list(format!("{}{name}", self.cwd), ctx),
            Msg::Clicked(name, false) => {
                let full = format!("{}{name}", self.cwd);
                // Clicking the selected file again deselects it (back to the folder).
                self.selected = if self.selected.as_deref() == Some(full.as_str()) { None } else { Some(full) };
            }
            Msg::Download => return Task::perform(ui::pick_folder("Download to folder"), Msg::DownloadTo),
            Msg::DownloadTo(None) => {}
            Msg::DownloadTo(Some(dest)) => {
                let target = self.selected.clone().unwrap_or_else(|| self.cwd.clone());
                ctx.log(format!("Downloading {target}…"));
                self.busy = true;
                let title = format!("Downloading {}", target.trim_end_matches('/').rsplit('/').next().unwrap_or(&target));
                return ctx.job(title, Unit::Bytes, move |conn, p| files::download(conn, target, dest, p), Msg::Done);
            }
            Msg::Send => {
                let dialog = rfd::AsyncFileDialog::new().set_title("Send a file to the TV");
                return Task::perform(async move { dialog.pick_file().await.map(|f| f.path().to_path_buf()) }, Msg::SendFile);
            }
            Msg::SendFile(None) => {}
            Msg::SendFile(Some(local)) => {
                self.busy = true;
                let dir = self.cwd.clone();
                let name = local.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                return ctx.job(format!("Sending {name}"), Unit::Bytes, move |conn, p| files::send(conn, local, dir, p), |id, r| {
                    Msg::Done(id, r.map(|inner| vec![inner.unwrap_or_else(|e| format!("Send failed: {e}"))]))
                });
            }
            Msg::Done(id, result) => {
                ctx.jobs.finish(id);
                self.busy = false;
                match result {
                    Ok(lines) => lines.into_iter().for_each(|l| ctx.log(l)),
                    Err(e) => ctx.log(format!("Failed: {e}")),
                }
                return self.list(self.cwd.clone(), ctx);
            }
            Msg::PlayUrl(u) => self.play_url = u,
            Msg::Play => {
                let Some(conn) = ctx.conn() else { return Task::none() };
                let url = self.play_url.trim().to_owned();
                if url.is_empty() {
                    return Task::none();
                }
                return Task::perform(crate::android::logs::play(conn, url), Msg::Played);
            }
            Msg::PlayFile => {
                let dialog = rfd::AsyncFileDialog::new()
                    .set_title("Play a file on the TV")
                    .add_filter("Media", &["mp4", "mkv", "webm", "mov", "avi", "m4v", "mp3", "flac", "m4a", "jpg", "jpeg", "png"]);
                return Task::perform(async move { dialog.pick_file().await.map(|f| f.path().to_path_buf()) }, Msg::PlayFilePicked);
            }
            Msg::PlayFilePicked(None) => {}
            Msg::PlayFilePicked(Some(path)) => {
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                self.busy = true;
                return ctx.job(format!("Sending {name}"), Unit::Bytes, move |conn, p| crate::android::logs::send_and_play(conn, path, p), Msg::PlayedFile);
            }
            Msg::Played(result) => ctx.log(result.unwrap_or_else(|e| format!("Couldn't play it: {e}"))),
            Msg::PlayedFile(id, result) => {
                ctx.jobs.finish(id);
                self.busy = false;
                ctx.log(result.and_then(|r| r).unwrap_or_else(|e| format!("Couldn't play it: {e}")));
            }
        }
        Task::none()
    }

    pub fn view<'a>(&'a self, _v: &View<'a>) -> Element<'a, Msg> {
        let where_ = match (&self.scrcpy, &self.scrcpy_version) {
            (Some(exe), Some(v)) => format!("{v} · {}", exe.display()),
            (Some(exe), None) => exe.display().to_string(),
            (None, _) => "Not found. Download it here, or Locate… an existing copy.".into(),
        };
        let scrcpy = ui::card(
            "scrcpy: mirror and control the TV",
            column![
                text("Shows the TV's screen in a window on your computer, with mouse and keyboard control.")
                    .size(12)
                    .color(ds::TEXT_SECONDARY),
                text(where_).size(11).color(if self.scrcpy.is_some() { ds::TEXT_TERTIARY } else { ds::AMBER }),
                row![
                    ui::small_button("Launch scrcpy", style::primary, self.scrcpy.is_some().then_some(Msg::LaunchScrcpy)),
                    ui::small_button("Locate…", style::standard, Some(Msg::LocateScrcpy)),
                    ui::small_button(
                        if self.downloading_scrcpy { "Downloading…" } else if self.scrcpy.is_some() { "Update" } else { "Download" },
                        style::standard,
                        (!self.downloading_scrcpy).then_some(Msg::DownloadScrcpy),
                    ),
                    ui::small_button("Website", style::standard, Some(Msg::OpenUrl("https://github.com/Genymobile/scrcpy"))),
                ]
                .spacing(6),
            ]
            .spacing(8)
            .into(),
        );

        let (status, color) = match self.recording {
            Recording::Idle => ("Not recording. Recordings stop by themselves after 3 minutes.", ds::TEXT_TERTIARY),
            Recording::Recording => ("● Recording…", ds::RED),
            Recording::Stopping => ("Finishing the file…", ds::AMBER),
        };
        let recording = ui::card(
            "Screen Recording",
            column![
                text(status).size(12).color(color),
                row![
                    ui::small_button("Start Recording", style::primary, (self.recording == Recording::Idle).then_some(Msg::RecordStart)),
                    ui::small_button("Stop & Save", style::danger, (self.recording == Recording::Recording).then_some(Msg::RecordStop)),
                ]
                .spacing(6),
            ]
            .spacing(8)
            .into(),
        );

        let nav = row![
            text_input("/sdcard/", &self.path_input).on_input(Msg::PathInput).on_submit(Msg::Go).size(ds::TEXT_BASE).padding([5, 8]).style(style::input(ds::INSET)),
            ui::small_button("Go", style::standard, Some(Msg::Go)),
            ui::small_button("Up", style::standard, (self.cwd != "/").then_some(Msg::Up)),
            ui::small_button("↻", style::standard, Some(Msg::Refresh)),
        ]
        .spacing(6)
        .align_y(Alignment::Center);
        let listing: Element<'a, Msg> = match &self.entries {
            None => text("Loading…").size(12).color(ds::TEXT_SECONDARY).into(),
            Some(Err(e)) => text(e.as_str()).size(12).color(ds::RED).into(),
            Some(Ok(list)) if list.is_empty() => text("(empty folder)").size(12).color(ds::TEXT_TERTIARY).into(),
            Some(Ok(list)) => column(list.iter().map(|e| {
                let full = format!("{}{}", self.cwd, e.name);
                let selected = self.selected.as_deref() == Some(full.as_str());
                let label = if e.is_dir { format!("📁 {}/", e.name) } else { e.name.clone() };
                button(text(label).size(12).color(if e.is_dir { ds::ACCENT } else { ds::TEXT }))
                    .padding([3, 10])
                    .width(Fill)
                    .style(style::pill(selected))
                    .on_press(Msg::Clicked(e.name.clone(), e.is_dir))
                    .into()
            }))
            .spacing(1)
            .into(),
        };
        let target = self.selected.as_deref().unwrap_or(&self.cwd);
        let files = ui::card(
            "Files on the TV",
            column![
                nav,
                container(scrollable(listing).height(260).direction(ds::thin_scrollbar()).style(style::scrollbar)).padding(6).width(Fill).style(style::inset),
                text(format!("Selected: {target}  (click a file to select it; with nothing selected, the whole folder)"))
                    .size(11)
                    .color(ds::TEXT_SECONDARY),
                row![
                    ui::small_button("Download to PC…", style::primary, (!self.busy).then_some(Msg::Download)),
                    ui::small_button("Send file here…", style::standard, (!self.busy).then_some(Msg::Send)),
                ]
                .spacing(6),
            ]
            .spacing(8)
            .into(),
        );

        let play = ui::card(
            "Play on TV",
            column![
                ds::label(
                    "Opens a video, music or picture link in the TV's player, or sends a file from this computer to the TV's \
                     Movies folder and plays it.",
                    ds::TEXT_SM,
                    ds::TEXT_SECONDARY,
                ),
                row![
                    ds::input("https://…/video.mp4", &self.play_url, true, ds::INSET).on_input(Msg::PlayUrl).on_submit(Msg::Play),
                    ui::small_button("Play Link", style::primary, (!self.play_url.trim().is_empty()).then_some(Msg::Play)),
                    ui::small_button("Send & Play File…", style::standard, (!self.busy).then_some(Msg::PlayFile)),
                ]
                .spacing(6)
                .align_y(Alignment::Center),
            ]
            .spacing(8)
            .into(),
        );
        ui::page(column![scrcpy, play, recording, files].spacing(12))
    }
}
