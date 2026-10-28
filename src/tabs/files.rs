//! Files: a file explorer for the TV's storage. Upload files and folders
//! (or drop them on the window), download, move and copy with Cut/Copy and
//! Paste, rename, make folders, delete, and play media on the TV.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use iced::widget::{button, checkbox, column, container, row, scrollable, space};
use iced::{Alignment, Element, Length, Task};

use super::{Ctx, View};
use crate::android::changes::Outcome;
use crate::android::files::{self, Entry, Sort, Transfer};
use crate::android::sysinfo::fmt_bytes;
use crate::design::{self as ds, style};
use crate::i18n::{tr, trf};
use crate::jobs::{JobId, Unit};
use crate::journal::Change;
use crate::ui;

const START_DIR: &str = "/sdcard/";
/// Two clicks on the same row within this time open it.
const DOUBLE_CLICK: Duration = Duration::from_millis(450);
/// Files dropped together arrive one by one; wait this long to upload them as one job.
const DROP_SETTLE: Duration = Duration::from_millis(300);

/// Shortcuts in the left column: (label, folder).
const PLACES: [(&str, &str); 6] = [
    ("Internal storage", "/sdcard/"),
    ("Download", "/sdcard/Download/"),
    ("Movies", "/sdcard/Movies/"),
    ("Music", "/sdcard/Music/"),
    ("Pictures", "/sdcard/Pictures/"),
    ("Temp folder", "/data/local/tmp/"),
];

#[derive(Debug, Clone, PartialEq, Eq)]
enum Naming {
    NewFolder(String),
    Rename { from: String, text: String },
}

#[derive(Debug)]
pub struct State {
    cwd: String,
    path_input: String,
    entries: Option<Result<Vec<Entry>, String>>,
    /// Names in `cwd`.
    selected: BTreeSet<String>,
    sort: Sort,
    show_hidden: bool,
    volumes: Vec<String>,
    /// Cut or copied items (full paths), waiting for Paste.
    clipboard: Option<(Transfer, Vec<String>)>,
    naming: Option<Naming>,
    last_click: Option<(String, Instant)>,
    dropped: Vec<PathBuf>,
    hovering: bool,
    busy: bool,
}

impl Default for State {
    fn default() -> Self {
        State {
            cwd: START_DIR.into(),
            path_input: START_DIR.into(),
            entries: None,
            selected: BTreeSet::new(),
            sort: Sort::Name,
            show_hidden: false,
            volumes: Vec::new(),
            clipboard: None,
            naming: None,
            last_click: None,
            dropped: Vec::new(),
            hovering: false,
            busy: false,
        }
    }
}

#[derive(Debug, Clone)]
pub enum Msg {
    Open(String),
    PathInput(String),
    Go,
    Up,
    Refresh,
    Listed(String, Result<Vec<Entry>, String>),
    Volumes(Vec<String>),
    Click(String),
    Tick(String, bool),
    TickAll(bool),
    SortBy(Sort),
    ShowHidden(bool),
    UploadFiles,
    UploadFolder,
    Picked(Vec<PathBuf>),
    UploadConfirmed(Vec<PathBuf>, bool),
    /// A file dragged from the computer onto the window.
    Dropped(PathBuf),
    DropsSettled,
    Hovering(bool),
    Download,
    DownloadTo(Option<PathBuf>),
    Cut,
    Copy,
    Paste,
    ClearClipboard,
    NewFolder,
    Rename,
    NameInput(String),
    NameSubmit,
    NameCancel,
    Delete,
    DeleteConfirmed(bool),
    Play,
    /// A quick operation finished: its log line and what it changed.
    Finished(Result<Outcome, String>),
    /// A job finished: its log lines.
    Done(JobId, Result<Vec<String>, String>),
}

/// What kind of file a name is, for its icon and whether it can play.
fn file_icon(e: &Entry) -> ds::Icon {
    if e.is_dir {
        return ds::Icon::Folder;
    }
    match e.name.rsplit('.').next().unwrap_or("").to_lowercase().as_str() {
        "mp4" | "m4v" | "mkv" | "webm" | "avi" | "mov" | "ts" => ds::Icon::FilePlay,
        "mp3" | "flac" | "m4a" | "aac" | "ogg" | "wav" | "opus" => ds::Icon::FileMusic,
        "jpg" | "jpeg" | "png" | "gif" | "webp" => ds::Icon::FileImage,
        "zip" | "rar" | "7z" | "tar" | "gz" | "apk" | "xapk" | "apks" => ds::Icon::FileArchive,
        "txt" | "log" | "json" | "xml" | "srt" | "md" => ds::Icon::FileText,
        _ => ds::Icon::File,
    }
}

fn playable(e: &Entry) -> bool {
    !e.is_dir && matches!(file_icon(e), ds::Icon::FilePlay | ds::Icon::FileMusic | ds::Icon::FileImage)
}

/// History notes for file changes (they can't be undone, but are listed).
fn notes(lines: &[String], prefixes: &[&str]) -> Vec<Change> {
    lines.iter().filter(|l| prefixes.iter().any(|p| l.starts_with(p))).map(|l| Change::Note { text: l.trim_end_matches('.').to_owned() }).collect()
}

impl State {
    pub fn reset(&mut self) {
        *self = State::default();
    }

    /// List the TV's files the first time the tab is opened.
    pub fn on_open(&mut self, ctx: &mut Ctx) -> Task<Msg> {
        if self.entries.is_some() {
            return Task::none();
        }
        let Some(conn) = ctx.conn() else { return Task::none() };
        Task::batch([self.list(self.cwd.clone(), ctx), Task::perform(files::volumes(conn), Msg::Volumes)])
    }

    /// Accept files dropped on the window only while the tab is ready for them.
    pub fn accepts_drops(&self) -> bool {
        !self.busy
    }

    fn list(&mut self, dir: String, ctx: &mut Ctx) -> Task<Msg> {
        let Some(conn) = ctx.conn() else { return Task::none() };
        let dir = files::as_dir(&dir);
        if dir != self.cwd {
            self.selected.clear();
        }
        self.cwd = dir.clone();
        self.path_input = dir.clone();
        self.entries = None;
        self.naming = None;
        Task::perform(files::list(conn, dir.clone()), move |r| Msg::Listed(dir.clone(), r))
    }

    fn entries(&self) -> Vec<&Entry> {
        match &self.entries {
            Some(Ok(list)) => list.iter().filter(|e| self.show_hidden || !e.name.starts_with('.')).collect(),
            _ => Vec::new(),
        }
    }

    fn selected_paths(&self) -> Vec<String> {
        self.selected.iter().map(|n| format!("{}{n}", self.cwd)).collect()
    }

    fn selected_entries(&self) -> Vec<&Entry> {
        self.entries().into_iter().filter(|e| self.selected.contains(&e.name)).collect()
    }

    fn upload(&mut self, locals: Vec<PathBuf>, ctx: &mut Ctx) -> Task<Msg> {
        if locals.is_empty() || self.busy {
            return Task::none();
        }
        self.busy = true;
        let dir = self.cwd.clone();
        let title = match locals.as_slice() {
            [one] => format!("Uploading {}", one.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()),
            many => format!("Uploading {} items", many.len()),
        };
        ctx.job(title, Unit::Bytes, move |conn, p| files::upload(conn, locals, dir, p), Msg::Done)
    }

    pub fn update(&mut self, msg: Msg, ctx: &mut Ctx) -> Task<Msg> {
        match msg {
            Msg::Open(dir) => return self.list(dir, ctx),
            Msg::PathInput(v) => self.path_input = v,
            Msg::Go => return self.list(self.path_input.clone(), ctx),
            Msg::Up => return self.list(files::parent(&self.cwd), ctx),
            Msg::Refresh => return self.list(self.cwd.clone(), ctx),
            Msg::Listed(dir, result) => {
                if dir == self.cwd && ctx.session.is_some() {
                    // Keep the ticks that still exist after a refresh.
                    if let Ok(list) = &result {
                        self.selected.retain(|n| list.iter().any(|e| &e.name == n));
                    }
                    let mut result = result;
                    if let Ok(list) = &mut result {
                        files::sort(list, self.sort);
                    }
                    self.entries = Some(result);
                }
            }
            Msg::Volumes(v) => self.volumes = v,
            Msg::Click(name) => {
                let now = Instant::now();
                let again = self.last_click.as_ref().is_some_and(|(n, t)| *n == name && now.duration_since(*t) < DOUBLE_CLICK);
                self.last_click = Some((name.clone(), now));
                if again {
                    self.last_click = None;
                    if self.entries().iter().any(|e| e.name == name && e.is_dir) {
                        return self.list(format!("{}{name}", self.cwd), ctx);
                    }
                }
                self.selected = BTreeSet::from([name]);
            }
            Msg::Tick(name, on) => {
                if on {
                    self.selected.insert(name);
                } else {
                    self.selected.remove(&name);
                }
            }
            Msg::TickAll(on) => {
                self.selected = if on { self.entries().iter().map(|e| e.name.clone()).collect() } else { BTreeSet::new() };
            }
            Msg::SortBy(by) => {
                self.sort = by;
                if let Some(Ok(list)) = &mut self.entries {
                    files::sort(list, by);
                }
            }
            Msg::ShowHidden(on) => {
                self.show_hidden = on;
                if !on {
                    self.selected.retain(|n| !n.starts_with('.'));
                }
            }
            Msg::UploadFiles => {
                let dialog = rfd::AsyncFileDialog::new().set_title(tr("Upload files to the TV"));
                return Task::perform(
                    async move { dialog.pick_files().await.map(|f| f.iter().map(|h| h.path().to_path_buf()).collect()).unwrap_or_default() },
                    Msg::Picked,
                );
            }
            Msg::UploadFolder => {
                let dialog = rfd::AsyncFileDialog::new().set_title(tr("Upload a folder to the TV"));
                return Task::perform(async move { dialog.pick_folder().await.map(|f| vec![f.path().to_path_buf()]).unwrap_or_default() }, Msg::Picked);
            }
            Msg::Picked(locals) => {
                if locals.is_empty() {
                    return Task::none();
                }
                // Ask before replacing anything with the same name.
                let names: Vec<String> = locals.iter().filter_map(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).collect();
                let clashes: Vec<&str> = self.entries().iter().filter(|e| names.contains(&e.name)).map(|e| e.name.as_str()).collect();
                if clashes.is_empty() {
                    return self.upload(locals, ctx);
                }
                let question = trf(
                    "{} item(s) in {} already have these names and will be replaced:\n\n{}\n\nUpload anyway?",
                    &[&clashes.len(), &self.cwd, &clashes.join("\n")],
                );
                return ui::confirm("Replace Files", question, move |yes| Msg::UploadConfirmed(locals, yes));
            }
            Msg::UploadConfirmed(locals, true) => return self.upload(locals, ctx),
            Msg::UploadConfirmed(_, false) => {}
            Msg::Dropped(path) => {
                self.hovering = false;
                let first = self.dropped.is_empty();
                self.dropped.push(path);
                if first {
                    return Task::perform(tokio::time::sleep(DROP_SETTLE), |_| Msg::DropsSettled);
                }
            }
            Msg::DropsSettled => {
                let locals = std::mem::take(&mut self.dropped);
                return self.update(Msg::Picked(locals), ctx);
            }
            Msg::Hovering(on) => self.hovering = on && !self.busy,
            Msg::Download => {
                if !self.selected.is_empty() {
                    return Task::perform(ui::pick_folder(tr("Download to folder")), Msg::DownloadTo);
                }
            }
            Msg::DownloadTo(None) => {}
            Msg::DownloadTo(Some(dest)) => {
                let targets = self.selected_paths();
                self.busy = true;
                let title = match targets.as_slice() {
                    [one] => format!("Downloading {}", files::base_name(one)),
                    many => format!("Downloading {} items", many.len()),
                };
                return ctx.job(title, Unit::Bytes, move |conn, p| files::download_many(conn, targets, dest, p), Msg::Done);
            }
            Msg::Cut | Msg::Copy => {
                let mode = if matches!(msg, Msg::Cut) { Transfer::Move } else { Transfer::Copy };
                let paths = self.selected_paths();
                if !paths.is_empty() {
                    self.clipboard = Some((mode, paths));
                }
            }
            Msg::ClearClipboard => self.clipboard = None,
            Msg::Paste => {
                let Some((mode, sources)) = self.clipboard.clone() else { return Task::none() };
                if self.busy {
                    return Task::none();
                }
                // Moved items are gone from where they were; copies can be pasted again.
                if mode == Transfer::Move {
                    self.clipboard = None;
                }
                self.busy = true;
                let dest = self.cwd.clone();
                let title = if mode == Transfer::Move { "Moving files" } else { "Copying files" };
                return ctx.job(title, Unit::Items, move |conn, p| files::transfer(conn, mode, sources, dest, p), Msg::Done);
            }
            Msg::NewFolder => self.naming = Some(Naming::NewFolder(String::new())),
            Msg::Rename => {
                if let [one] = self.selected.iter().collect::<Vec<_>>()[..] {
                    self.naming = Some(Naming::Rename { from: format!("{}{one}", self.cwd), text: one.clone() });
                }
            }
            Msg::NameInput(v) => match &mut self.naming {
                Some(Naming::NewFolder(text)) | Some(Naming::Rename { text, .. }) => *text = v,
                None => {}
            },
            Msg::NameCancel => self.naming = None,
            Msg::NameSubmit => {
                let (Some(naming), Some(conn)) = (self.naming.clone(), ctx.conn()) else { return Task::none() };
                let name = match &naming {
                    Naming::NewFolder(t) | Naming::Rename { text: t, .. } => t.trim().to_owned(),
                };
                if !files::valid_name(&name) {
                    ctx.log("Names can't be empty or contain /.");
                    return Task::none();
                }
                self.naming = None;
                let to_outcome = |r: Result<String, String>, note: bool| {
                    r.map(|line| Outcome { changes: if note { vec![Change::Note { text: line.trim_end_matches('.').to_owned() }] } else { vec![] }, lines: vec![line] })
                };
                return match naming {
                    Naming::NewFolder(_) => {
                        self.selected = BTreeSet::from([name.clone()]);
                        Task::perform(files::make_dir(conn, self.cwd.clone(), name), move |r| Msg::Finished(to_outcome(r, false)))
                    }
                    Naming::Rename { from, .. } => {
                        self.selected = BTreeSet::from([name.clone()]);
                        Task::perform(files::rename(conn, from, name), move |r| Msg::Finished(to_outcome(r, true)))
                    }
                };
            }
            Msg::Delete => {
                let names: Vec<String> = self.selected.iter().cloned().collect();
                if names.is_empty() || self.busy {
                    return Task::none();
                }
                let mut shown: Vec<String> = names.iter().take(12).map(|n| format!("• {n}")).collect();
                if names.len() > 12 {
                    shown.push(trf("…and {} more", &[&(names.len() - 12)]));
                }
                let question = trf(
                    "Delete these {} item(s) from {}? Folders are deleted with everything in them.\n\n{}\n\nThis can't be undone.",
                    &[&names.len(), &self.cwd, &shown.join("\n")],
                );
                return ui::confirm("Delete Files", question, Msg::DeleteConfirmed);
            }
            Msg::DeleteConfirmed(false) => {}
            Msg::DeleteConfirmed(true) => {
                let paths = self.selected_paths();
                self.busy = true;
                self.selected.clear();
                return ctx.job("Deleting files", Unit::Items, move |conn, p| files::delete(conn, paths, p), Msg::Done);
            }
            Msg::Play => {
                let Some(conn) = ctx.conn() else { return Task::none() };
                let Some(e) = self.selected_entries().into_iter().find(|e| playable(e)) else { return Task::none() };
                let uri = format!("file://{}{}", self.cwd, e.name);
                return Task::perform(crate::android::logs::play(conn, uri), |r| Msg::Finished(r.map(|l| Outcome::lines(vec![l]))));
            }
            Msg::Finished(result) => {
                match result {
                    Ok(outcome) => ctx.apply(outcome),
                    Err(e) => ctx.log(format!("Failed: {e}")),
                }
                return self.list(self.cwd.clone(), ctx);
            }
            Msg::Done(id, result) => {
                ctx.jobs.finish(id);
                self.busy = false;
                match result {
                    Ok(lines) => {
                        let changes = notes(&lines, &["Deleted ", "Moved "]);
                        ctx.apply(Outcome { lines, changes });
                    }
                    Err(e) => ctx.log(format!("Failed: {e}")),
                }
                return self.list(self.cwd.clone(), ctx);
            }
        }
        Task::none()
    }

    pub fn view<'a>(&'a self, _v: &View<'a>) -> Element<'a, Msg> {
        let at_root = self.cwd == "/";
        let bar = row![
            ds::square_button(ds::Icon::ArrowUp, ds::TEXT_SECONDARY, ds::CONTROL_HEIGHT, style::standard, (!at_root).then_some(Msg::Up)),
            ds::square_button(ds::Icon::RefreshCw, ds::TEXT_SECONDARY, ds::CONTROL_HEIGHT, style::standard, Some(Msg::Refresh)),
            ds::input("Folder on the TV", &self.path_input, true, ds::INSET).on_input(Msg::PathInput).on_submit(Msg::Go).width(Length::Fill),
            checkbox(self.show_hidden).label(tr("Show hidden")).on_toggle(Msg::ShowHidden).size(14).text_size(ds::TEXT_BASE).style(style::checkbox),
        ]
        .spacing(6)
        .align_y(Alignment::Center);
        let toolbar = container(column![container(bar).padding([0, 12]).center_y(Length::Fill), ds::hline(ds::BORDER)])
            .height(ds::TOOLBAR_HEIGHT)
            .style(style::fill(ds::SIDEBAR));

        let ready = !self.busy;
        let picked = self.selected_entries();
        let one = picked.len() == 1;
        let any = !picked.is_empty();
        let btn = |icon: ds::Icon, caption: &'static str, style_fn: ui::ButtonStyle, on: bool, msg: Msg| -> Element<'a, Msg> {
            ds::icon_button(Some((icon, ds::TEXT_SECONDARY)), caption, None, style_fn, ds::CONTROL_HEIGHT, on.then_some(msg)).into()
        };
        let paste_caption = match &self.clipboard {
            Some((_, items)) => trf("Paste ({})", &[&items.len()]),
            None => tr("Paste").to_owned(),
        };
        let actions = row![
            btn(ds::Icon::Upload, "Upload Files…", style::standard, ready, Msg::UploadFiles),
            btn(ds::Icon::FolderUp, "Upload Folder…", style::standard, ready, Msg::UploadFolder),
            btn(ds::Icon::FolderPlus, "New Folder", style::standard, ready, Msg::NewFolder),
            ds::vline(20.0),
            btn(ds::Icon::Download, "Download…", style::standard, ready && any, Msg::Download),
            btn(ds::Icon::Scissors, "Cut", style::standard, any, Msg::Cut),
            btn(ds::Icon::Copy, "Copy", style::standard, any, Msg::Copy),
            button(
                container(
                    row![
                        ds::icon_any(ds::Icon::ClipboardPaste, 12.0, ds::TEXT_SECONDARY),
                        ds::label(paste_caption, ds::TEXT_BASE, if self.clipboard.is_some() { ds::TEXT } else { ds::TEXT_TERTIARY })
                    ]
                        .spacing(6)
                        .align_y(Alignment::Center)
                )
                .center_y(Length::Fill)
            )
            .height(ds::CONTROL_HEIGHT)
            .padding([0, 9])
            .style(style::standard)
            .on_press_maybe((ready && self.clipboard.is_some()).then_some(Msg::Paste)),
            btn(ds::Icon::Pencil, "Rename", style::standard, ready && one, Msg::Rename),
            btn(ds::Icon::Trash, "Delete", style::danger, ready && any, Msg::Delete),
            btn(ds::Icon::Play, "Play on TV", style::standard, one && picked.iter().all(|e| playable(e)), Msg::Play),
        ]
        .spacing(6)
        .align_y(Alignment::Center);

        let mut top = column![actions].spacing(8).padding([10, 12]);
        if let Some(naming) = &self.naming {
            let (prompt, value, go) = match naming {
                Naming::NewFolder(t) => ("New folder name:", t, "Create"),
                Naming::Rename { text, .. } => ("New name:", text, "Rename"),
            };
            top = top.push(
                row![
                    ds::label(prompt, ds::TEXT_BASE, ds::TEXT),
                    ds::input("", value, false, ds::INSET).on_input(Msg::NameInput).on_submit(Msg::NameSubmit).width(320),
                    ui::small_button(go, style::primary, files::valid_name(value).then_some(Msg::NameSubmit)),
                    ui::small_button("Cancel", style::standard, Some(Msg::NameCancel)),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
            );
        }
        if let Some((mode, items)) = &self.clipboard {
            let what = if *mode == Transfer::Move {
                trf("{} item(s) cut. Open the folder to move them to and press Paste.", &[&items.len()])
            } else {
                trf("{} item(s) copied. Open the folder to copy them to and press Paste.", &[&items.len()])
            };
            top = top.push(
                row![ds::label(what, ds::TEXT_SM, ds::ACCENT), ui::small_button("Clear", style::ghost, Some(Msg::ClearClipboard))]
                    .spacing(8)
                    .align_y(Alignment::Center),
            );
        }

        column![toolbar, top, ds::hline(ds::BORDER_SUBTLE), row![self.places(), ds::vline_fill(), self.listing()].height(Length::Fill)].into()
    }

    fn places(&self) -> Element<'_, Msg> {
        let place = |icon: ds::Icon, label: String, dir: String| -> Element<'_, Msg> {
            let active = self.cwd == dir;
            button(
                row![ds::icon_any(icon, 13.0, if active { ds::ACCENT } else { ds::TEXT_TERTIARY }), ds::label(label, ds::TEXT_BASE, ds::TEXT)]
                    .spacing(8)
                    .align_y(Alignment::Center),
            )
            .padding([6, 10])
            .width(Length::Fill)
            .style(style::row(active))
            .on_press(Msg::Open(dir))
            .into()
        };
        let mut list = column![ds::overline("Places")].spacing(2);
        for (label, dir) in PLACES {
            // Folder shortcuts keep the TV's own folder names; only the descriptive ones translate.
            let (icon, label) = match dir {
                START_DIR => (ds::Icon::HardDrive, tr(label)),
                "/data/local/tmp/" => (ds::Icon::Folder, tr(label)),
                _ => (ds::Icon::Folder, label),
            };
            list = list.push(place(icon, label.to_owned(), dir.to_owned()));
        }
        for v in &self.volumes {
            list = list.push(place(ds::Icon::Usb, trf("USB drive ({})", &[&files::base_name(v)]), v.clone()));
        }
        container(scrollable(list.padding([12, 10])).direction(ds::thin_scrollbar()).style(style::scrollbar))
            .width(200)
            .height(Length::Fill)
            .style(style::fill(ds::SIDEBAR_LOW))
            .into()
    }

    fn listing(&self) -> Element<'_, Msg> {
        let body: Element<'_, Msg> = match &self.entries {
            None => ui::empty_state("Reading…", ""),
            Some(Err(e)) => ui::empty_state("Couldn't open this folder", e),
            Some(Ok(_)) => {
                let entries = self.entries();
                if entries.is_empty() {
                    ui::empty_state("This folder is empty", "Upload files, or drop them here from your computer.")
                } else {
                    let rows = column(entries.iter().map(|e| {
                        let ticked = self.selected.contains(&e.name);
                        let name = e.name.clone();
                        let color = if e.is_dir { ds::TEXT } else { ds::TEXT_SECONDARY };
                        button(
                            row![
                                checkbox(ticked).on_toggle(move |on| Msg::Tick(name.clone(), on)).size(13).style(style::checkbox),
                                ds::icon_any(file_icon(e), 14.0, if e.is_dir { ds::ACCENT } else { ds::TEXT_TERTIARY }),
                                ds::label(e.name.as_str(), ds::TEXT_BASE, color).width(Length::Fill),
                                ds::mono(e.size.map(fmt_bytes).unwrap_or_default(), ds::TEXT_SM, ds::TEXT_SECONDARY).width(90),
                                ds::mono(e.modified.as_str(), ds::TEXT_SM, ds::TEXT_TERTIARY).width(130),
                            ]
                            .spacing(10)
                            .align_y(Alignment::Center),
                        )
                        .padding([5, 12])
                        .width(Length::Fill)
                        .style(style::grid_row(ticked, false))
                        .on_press(Msg::Click(e.name.clone()))
                        .into()
                    }));
                    scrollable(rows).direction(ds::thin_scrollbar()).style(style::scrollbar).height(Length::Fill).into()
                }
            }
        };
        let entries = self.entries();
        let all = !entries.is_empty() && entries.iter().all(|e| self.selected.contains(&e.name));
        let head = |caption: &'static str, by: Sort, width: Length| {
            let arrow = if self.sort == by { " ↓" } else { "" };
            button(ds::label(format!("{}{arrow}", tr(caption)), ds::TEXT_XS, ds::TEXT_TERTIARY))
                .padding([2, 0])
                .width(width)
                .style(style::header_cell)
                .on_press(Msg::SortBy(by))
        };
        let header = container(
            row![
                checkbox(all).on_toggle(Msg::TickAll).size(13).style(style::checkbox),
                space::horizontal().width(14),
                head("Name", Sort::Name, Length::Fill),
                head("Size", Sort::Size, Length::Fixed(90.0)),
                head("Modified", Sort::Modified, Length::Fixed(130.0)),
            ]
            .spacing(10)
            .align_y(Alignment::Center),
        )
        .padding([6, 12])
        .style(style::fill(ds::GRID_HEADER));
        let status = if self.hovering {
            ds::label(trf("Drop to upload into {}", &[&self.cwd]), ds::TEXT_SM, ds::ACCENT)
        } else {
            let selected = self.selected_entries();
            let bytes: u64 = selected.iter().filter_map(|e| e.size).sum();
            let mut line = trf("{} item(s)", &[&entries.len()]);
            if !selected.is_empty() {
                line.push_str(" · ");
                line.push_str(&trf("{} selected", &[&selected.len()]));
                if bytes > 0 {
                    line.push_str(&format!(" ({})", fmt_bytes(bytes)));
                }
            }
            line.push_str(" · ");
            line.push_str(tr("Double-click a folder to open it; drag files here from your computer to upload them."));
            ds::label(line, ds::TEXT_SM, ds::TEXT_TERTIARY)
        };
        let footer = container(status).padding([6, 12]).width(Length::Fill).style(style::fill(if self.hovering { ds::ACCENT_WELL } else { ds::FOOTER }));
        column![header, container(body).height(Length::Fill).width(Length::Fill), footer].width(Length::Fill).into()
    }
}
