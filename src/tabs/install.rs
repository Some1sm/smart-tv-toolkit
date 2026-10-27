//! Install: APK files, quick-install apps and Shizuku.

use std::collections::BTreeSet;
use std::path::PathBuf;

use iced::widget::{button, column, container, row, space};
use iced::{Alignment, Element, Fill, Task};

use super::{Ctx, View};
use crate::ui::text;
use crate::android::catalog::{self, AppSpec, SHIZUKU};
use crate::android::{install, shizuku};
use crate::jobs::{JobId, Unit};
use crate::design::{self as ds, style};
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
    /// Catalog apps installed on the TV: package → version.
    installed: std::collections::HashMap<String, String>,
    /// Newest release per catalog app name.
    latest: std::collections::HashMap<&'static str, String>,
    checked_updates: bool,
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
    BatchDone(JobId, super::Target, Result<Vec<String>, String>),
    OpenUrl(&'static str),
    Installed(std::collections::HashMap<String, String>),
    Latest(&'static str, Result<String, String>),
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

    /// Check for newer versions of installed catalog apps, once per
    /// connection: versions on the TV (read-only) and latest releases.
    pub fn on_open(&mut self, ctx: &mut Ctx) -> Task<Msg> {
        let Some(conn) = ctx.conn() else { return Task::none() };
        if self.checked_updates {
            return Task::none();
        }
        self.checked_updates = true;
        let mut tasks = vec![Task::perform(catalog::installed_versions(conn), Msg::Installed)];
        for spec in catalog::APPS.iter().chain([&SHIZUKU]).filter(|s| catalog::package_id(s).is_some()) {
            let name = spec.name;
            tasks.push(Task::perform(catalog::latest_version(*spec), move |r| Msg::Latest(name, r)));
        }
        Task::batch(tasks)
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
                let others = apks.clone();
                let batch = ctx.job_on_batch(&title, Unit::Bytes, move |conn, p| install::install_many(conn, others.clone(), p), Msg::BatchDone);
                let here = ctx.job(title, Unit::Bytes, move |conn, p| install::install_many(conn, apks, p), Msg::Done);
                return Task::batch([here, batch]);
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
                let other_apk = apk.clone();
                let batch = ctx.job_on_batch(
                    &format!("Installing {name}"),
                    Unit::Bytes,
                    move |conn, p| install::install_with(conn, other_apk.clone(), post, p),
                    move |id, target, r| Msg::BatchDone(id, target, r.map(|inner| vec![format!("{name}: {}", inner.unwrap_or_else(|e| e))])),
                );
                let here = ctx.job(
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
                return Task::batch([here, batch]);
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
                // Show the new version straight away.
                if let Some(conn) = ctx.conn() {
                    return Task::perform(catalog::installed_versions(conn), Msg::Installed);
                }
            }
            Msg::BatchDone(id, target, result) => {
                ctx.jobs.finish(id);
                match result {
                    Ok(lines) => lines.into_iter().for_each(|l| ctx.log(format!("[{}] {l}", target.label))),
                    Err(e) => ctx.log(format!("[{}] Install failed: {e}", target.label)),
                }
            }
            Msg::Installed(found) => {
                if ctx.session.is_some() {
                    self.installed = found;
                }
            }
            Msg::Latest(name, result) => {
                if let Ok(version) = result {
                    self.latest.insert(name, version);
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
        // Installed on the TV, and is there something newer?
        let installed = catalog::package_id(spec).and_then(|p| self.installed.get(p));
        let version_note: Option<(String, iced::Color)> = installed.map(|v| match self.latest.get(spec.name) {
            Some(latest) if crate::update::is_newer(latest, v) => {
                (crate::i18n::trf("Installed {} · {} available", &[v, latest]), ds::AMBER)
            }
            Some(_) => (crate::i18n::trf("Installed {} · up to date", &[v]), ds::GREEN),
            None => (crate::i18n::trf("Installed {}", &[v]), ds::TEXT_TERTIARY),
        });
        let download_label = if downloading { "Downloading…" } else if downloaded { "Update" } else { "Download" };
        let buttons = row![
            ui::small_button(download_label, style::standard, (!downloading && build.is_ok()).then_some(Msg::Download(spec.name))),
            ui::small_button("Install", style::primary, (downloaded && !downloading && !self.busy).then_some(Msg::QuickInstall(spec.name))),
            text(status).size(11).color(ds::TEXT_TERTIARY),
        ]
        .spacing(6)
        .align_y(Alignment::Center);
        container(
            column![
                button(text(spec.name).size(13).color(ds::ACCENT))
                    .padding(0)
                    .style(|_, _| button::Style::default())
                    .on_press(Msg::OpenUrl(spec.homepage)),
                text(spec.description).size(11).color(ds::TEXT_SECONDARY),
                ds::label(version_note.as_ref().map(|(t, _)| t.clone()).unwrap_or_default(), ds::TEXT_SM, version_note.as_ref().map_or(ds::TEXT_TERTIARY, |(_, c)| *c)),
                buttons,
            ]
            .spacing(6),
        )
        .padding(12)
        .width(Fill)
        .style(style::well)
        .into()
    }

    pub fn view<'a>(&'a self, v: &View<'a>) -> Element<'a, Msg> {
        let batch = v.batch_note().map(|note| format!("{note} (ticked in the sidebar).")).unwrap_or_default();
        let apk = ui::card(
            "Install APK",
            column![
                text("Pick APK files, or a folder of them. Play Protect's install checks are paused during the install and put back as they were afterwards.")
                    .size(12)
                    .color(ds::TEXT_SECONDARY),
                ds::label(batch, ds::TEXT_SM, ds::ACCENT),
                row![
                    ui::small_button("Install APK files…", style::primary, (!self.busy).then_some(Msg::PickApks)),
                    ui::small_button("Install folder…", style::standard, (!self.busy).then_some(Msg::PickFolder)),
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
                text(SHIZUKU.description).size(12).color(ds::TEXT_SECONDARY),
                row![
                    ui::small_button(label, style::standard, (!shizuku_downloading).then_some(Msg::Download(SHIZUKU.name))),
                    ui::small_button("Install & Start", style::primary, (shizuku_ready && !self.busy).then_some(Msg::ShizukuInstall)),
                    ui::small_button("Start Shizuku", style::standard, (!self.busy).then_some(Msg::ShizukuStart)),
                ]
                .spacing(6),
            ]
            .spacing(8)
            .into(),
        );

        ui::page(column![apk, quick, shizuku].spacing(12))
    }
}
