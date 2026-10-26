//! Install: APK files, quick-install apps and Shizuku.

use std::collections::BTreeSet;
use std::path::PathBuf;

use iced::widget::{button, column, container, row, scrollable, space, text};
use iced::{Alignment, Element, Fill, Task};

use super::{Ctx, View};
use crate::android::catalog::{self, AppSpec, SHIZUKU};
use crate::android::{install, shizuku};
use crate::jobs::{JobId, Unit};
use crate::theme;
use crate::ui;

#[derive(Debug, Default)]
pub struct State {
    /// CPU types of the connected TV, preferred first.
    abis: Vec<String>,
    downloading: BTreeSet<&'static str>,
    /// Apps whose APK (for the current CPU type) is already downloaded.
    /// Checked when it can change, not on every redraw.
    downloaded: BTreeSet<&'static str>,
    busy: bool,
}

#[derive(Debug, Clone)]
pub enum Msg {
    PickApks,
    PickFolder,
    Install(Vec<PathBuf>),
    Download(&'static str),
    Downloaded(JobId, &'static str, Result<String, String>),
    QuickInstall(&'static str),
    ShizukuInstall,
    ShizukuStart,
    Done(JobId, Result<Vec<String>, String>),
    OpenUrl(&'static str),
}

fn spec(name: &str) -> Option<AppSpec> {
    catalog::APPS.iter().chain([&SHIZUKU]).find(|a| a.name == name).copied()
}

impl State {
    pub fn reset(&mut self) {
        let downloading = std::mem::take(&mut self.downloading);
        *self = State { downloading, ..State::default() };
        self.refresh_downloaded();
    }

    pub fn on_connect(&mut self, ctx: &mut Ctx) {
        self.abis = ctx.android().map(|s| s.summary.abis).unwrap_or_default();
        self.refresh_downloaded();
    }

    fn refresh_downloaded(&mut self) {
        self.downloaded = catalog::APPS
            .iter()
            .filter(|s| catalog::download_path(s, &self.abis).is_some_and(|p| p.is_file()))
            .map(|s| s.name)
            .collect();
        if catalog::download_path(&SHIZUKU, &[]).is_some_and(|p| p.is_file()) {
            self.downloaded.insert(SHIZUKU.name);
        }
    }

    pub fn update(&mut self, msg: Msg, ctx: &mut Ctx) -> Task<Msg> {
        match msg {
            Msg::PickApks => {
                let dialog = rfd::AsyncFileDialog::new().set_title("Install APK files").add_filter("Android app", &["apk"]);
                return Task::perform(
                    async move {
                        dialog.pick_files().await.map(|f| f.iter().map(|f| f.path().to_path_buf()).collect()).unwrap_or_default()
                    },
                    Msg::Install,
                );
            }
            Msg::PickFolder => {
                let pick = ui::pick_folder("Folder with APK files");
                return Task::perform(async move { pick.await.map(|d| install::apks_in(&d)).unwrap_or_default() }, Msg::Install);
            }
            Msg::Install(apks) => {
                if apks.is_empty() {
                    return Task::none();
                }
                ctx.log(format!("Installing {} APK(s)…", apks.len()));
                self.busy = true;
                let title = format!("Installing {} APK(s)", apks.len());
                return ctx.job(title, Unit::Bytes, move |conn, p| install::install_many(conn, apks, p), Msg::Done);
            }
            Msg::Download(name) => {
                let Some(spec) = spec(name) else { return Task::none() };
                ctx.log(format!("Downloading {name}…"));
                self.downloading.insert(name);
                let (id, progress) = ctx.jobs.start(format!("Downloading {name}"), Unit::Bytes);
                let abis = self.abis.clone();
                return Task::perform(catalog::fetch(spec, abis, progress), move |r| Msg::Downloaded(id, name, r));
            }
            Msg::Downloaded(id, name, result) => {
                ctx.jobs.finish(id);
                self.downloading.remove(name);
                match result {
                    Ok(line) => ctx.log(line),
                    Err(e) => ctx.log(format!("Download failed for {name}: {e}")),
                }
                self.refresh_downloaded();
            }
            Msg::QuickInstall(name) => {
                let Some(spec) = spec(name) else { return Task::none() };
                let Some(apk) = catalog::download_path(&spec, &self.abis) else { return Task::none() };
                ctx.log(format!("Installing {name}…"));
                self.busy = true;
                let post = spec.post_install;
                return ctx.job(
                    format!("Installing {name}"),
                    Unit::Bytes,
                    move |conn, p| install::install_with(conn, apk, post, p),
                    move |id, r| {
                        Msg::Done(id, r.map(|inner| vec![match inner {
                            Ok(out) => format!("{name}: {out}"),
                            Err(e) => format!("{name}: {e}"),
                        }]))
                    },
                );
            }
            Msg::ShizukuInstall => {
                ctx.log("Installing and starting Shizuku…");
                self.busy = true;
                return ctx.job("Shizuku", Unit::Bytes, |conn, p| shizuku::install_and_start(conn, p), |id, r| {
                    Msg::Done(id, r.map(|inner| vec![inner.unwrap_or_else(|e| format!("Shizuku: {e}"))]))
                });
            }
            Msg::ShizukuStart => {
                ctx.log("Starting Shizuku…");
                self.busy = true;
                return ctx.job("Starting Shizuku", Unit::Items, |conn, _| shizuku::start(conn), |id, r| {
                    Msg::Done(id, r.map(|inner| vec![inner.unwrap_or_else(|e| format!("Shizuku: {e}"))]))
                });
            }
            Msg::Done(id, result) => {
                ctx.jobs.finish(id);
                self.busy = false;
                match result {
                    Ok(lines) => lines.into_iter().for_each(|l| ctx.log(l)),
                    Err(e) => ctx.log(format!("Install failed: {e}")),
                }
            }
            Msg::OpenUrl(url) => {
                if let Err(e) = open::that(url) {
                    ctx.log(format!("Could not open the browser: {e}"));
                }
            }
        }
        Task::none()
    }

    fn app_cell<'a>(&'a self, spec: &'static AppSpec) -> Element<'a, Msg> {
        let downloading = self.downloading.contains(spec.name);
        let build = catalog::build_for(spec, &self.abis);
        let downloaded = self.downloaded.contains(spec.name);
        let status = match (&build, downloaded) {
            (Err(e), _) if !self.abis.is_empty() => e.clone(),
            (Err(_), _) => "Connect to pick the build for your TV's CPU.".into(),
            (Ok((_, _, note)), true) => format!("Downloaded{}", note.as_ref().map(|n| format!(" ({n})")).unwrap_or_default()),
            (Ok(_), false) => String::new(),
        };
        let download_label = if downloading { "Downloading…" } else if downloaded { "Update" } else { "Download" };
        let buttons = row![
            ui::small_button(download_label, theme::secondary_button, (!downloading && build.is_ok()).then_some(Msg::Download(spec.name))),
            ui::small_button("Install", theme::primary_button, (downloaded && !downloading && !self.busy).then_some(Msg::QuickInstall(spec.name))),
            text(status).size(11).color(theme::TEXT_DISABLED),
        ]
        .spacing(6)
        .align_y(Alignment::Center);
        container(
            column![
                button(text(spec.name).size(13).color(theme::PRIMARY))
                    .padding(0)
                    .style(|_, _| button::Style::default())
                    .on_press(Msg::OpenUrl(spec.homepage)),
                text(spec.description).size(11).color(theme::TEXT_MUTED),
                buttons,
            ]
            .spacing(6),
        )
        .padding(12)
        .width(Fill)
        .style(theme::card)
        .into()
    }

    pub fn view<'a>(&'a self, _v: &View<'a>) -> Element<'a, Msg> {
        let apk = ui::card(
            "Install APK",
            column![
                text("Pick APK files, or a folder of them. Play Protect's install checks are paused during the install and put back as they were afterwards.")
                    .size(12)
                    .color(theme::TEXT_MUTED),
                row![
                    ui::small_button("Install APK files…", theme::primary_button, (!self.busy).then_some(Msg::PickApks)),
                    ui::small_button("Install folder…", theme::secondary_button, (!self.busy).then_some(Msg::PickFolder)),
                ]
                .spacing(6),
            ]
            .spacing(8)
            .into(),
        );

        let mut grid = column![].spacing(8);
        let mut cells = catalog::APPS.iter().map(|spec| self.app_cell(spec));
        while let Some(left) = cells.next() {
            let right = cells.next().unwrap_or_else(|| space().width(Fill).into());
            grid = grid.push(row![left, right].spacing(8));
        }
        let quick = ui::card("Quick Install Popular Apps", grid.into());

        let shizuku_downloading = self.downloading.contains(SHIZUKU.name);
        let shizuku_ready = self.downloaded.contains(SHIZUKU.name);
        let label = if shizuku_downloading {
            "Downloading…"
        } else if shizuku_ready {
            "Update Shizuku"
        } else {
            "Download Shizuku"
        };
        let shizuku = ui::card(
            "Shizuku",
            column![
                text(SHIZUKU.description).size(12).color(theme::TEXT_MUTED),
                row![
                    ui::small_button(label, theme::secondary_button, (!shizuku_downloading).then_some(Msg::Download(SHIZUKU.name))),
                    ui::small_button("Install & Start", theme::primary_button, (shizuku_ready && !self.busy).then_some(Msg::ShizukuInstall)),
                    ui::small_button("Start Shizuku", theme::secondary_button, (!self.busy).then_some(Msg::ShizukuStart)),
                ]
                .spacing(6),
            ]
            .spacing(8)
            .into(),
        );

        scrollable(column![apk, quick, shizuku].spacing(8).padding(4)).into()
    }
}
