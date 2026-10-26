//! Packages: list installed apps and act on a selection.

use std::collections::BTreeSet;
use std::path::PathBuf;

use iced::widget::{button, column, container, row, scrollable, space, text, text_input, tooltip};
use iced::{Alignment, Element, Fill, Font, Task};

use super::{Ctx, View};
use crate::android::labels::{self, Cache};
use crate::android::packages::{self, Action, ListKind, Package};
use crate::jobs::{JobId, Unit};
use crate::theme;
use crate::ui;

#[derive(Debug, Default)]
pub struct State {
    kind: Option<ListKind>,
    packages: Vec<Package>,
    filter: String,
    selected: BTreeSet<String>,
    loading: bool,
    busy: bool,
    /// Real app names by `package@version`, loaded from disk on first use.
    names: Option<Cache>,
    fetching_names: bool,
}

#[derive(Debug, Clone)]
pub enum Msg {
    List(ListKind),
    Listed(ListKind, Result<Vec<Package>, String>),
    Filter(String),
    Toggle(String),
    SelectShown,
    ClearSelection,
    Act(Action),
    ActConfirmed(Action, Vec<String>, bool),
    SaveApk,
    SaveApkTo(Vec<String>, Option<PathBuf>),
    Done(JobId, Result<Vec<String>, String>),
    LoadNames,
    NamesLoaded(JobId, Result<Cache, String>),
}

/// The real name if it's known, else the one derived from the package id.
fn display_name<'a>(p: &'a Package, names: Option<&'a Cache>) -> &'a str {
    names
        .and_then(|n| n.get(&labels::key(&p.id, &p.version)))
        .filter(|n| !n.is_empty())
        .map_or(p.name.as_str(), String::as_str)
}

/// Packages matching the filter (by id or name, case-insensitive).
fn visible<'a>(packages: &'a [Package], names: Option<&'a Cache>, filter: &str) -> impl Iterator<Item = &'a Package> {
    let q = filter.trim().to_lowercase();
    packages.iter().filter(move |p| {
        q.is_empty() || p.id.to_lowercase().contains(&q) || display_name(p, names).to_lowercase().contains(&q)
    })
}

impl State {
    pub fn reset(&mut self) {
        *self = State::default();
    }

    /// Listed packages whose real name hasn't been looked up yet.
    fn missing_names(&self) -> Vec<(String, String)> {
        let names = self.names.as_ref();
        self.packages
            .iter()
            .filter(|p| names.is_none_or(|n| !n.contains_key(&labels::key(&p.id, &p.version))))
            .map(|p| (p.id.clone(), p.version.clone()))
            .collect()
    }

    fn fetch_names(&mut self, ctx: &mut Ctx) -> Task<Msg> {
        let wanted = self.missing_names();
        if wanted.is_empty() || self.fetching_names {
            return Task::none();
        }
        self.fetching_names = true;
        let title = format!("Reading app names ({})", wanted.len());
        ctx.job(title, Unit::Items, move |conn, p| labels::fetch(conn, wanted, p), Msg::NamesLoaded)
    }

    fn list(&mut self, kind: ListKind, ctx: &mut Ctx) -> Task<Msg> {
        let Some(conn) = ctx.conn() else { return Task::none() };
        self.kind = Some(kind);
        self.loading = true;
        Task::perform(packages::list(conn, kind), move |r| Msg::Listed(kind, r))
    }

    pub fn update(&mut self, msg: Msg, ctx: &mut Ctx) -> Task<Msg> {
        match msg {
            Msg::List(kind) => {
                ctx.log(format!("Listing {} packages…", kind.label().to_lowercase()));
                return self.list(kind, ctx);
            }
            Msg::Listed(kind, result) => {
                if self.kind != Some(kind) {
                    return Task::none(); // a newer list was requested meanwhile
                }
                self.loading = false;
                match result {
                    Ok(list) if ctx.session.is_some() => {
                        ctx.log(format!("{} {} package(s).", list.len(), kind.label()));
                        let ids: BTreeSet<&String> = list.iter().map(|p| &p.id).collect();
                        self.selected.retain(|p| ids.contains(p));
                        self.packages = list;
                        self.names.get_or_insert_with(labels::load_cache);
                        // The user's own apps are few: name them right away.
                        // System lists run to hundreds; that waits for a click.
                        if matches!(kind, ListKind::User | ListKind::PlayStore | ListKind::Sideloaded) {
                            return self.fetch_names(ctx);
                        }
                    }
                    Ok(_) => {}
                    Err(e) => ctx.log(format!("Could not list packages: {e}")),
                }
            }
            Msg::Filter(f) => self.filter = f,
            Msg::Toggle(pkg) => {
                if !self.selected.remove(&pkg) {
                    self.selected.insert(pkg);
                }
            }
            Msg::SelectShown => {
                let shown: Vec<String> =
                    visible(&self.packages, self.names.as_ref(), &self.filter).map(|p| p.id.clone()).collect();
                self.selected.extend(shown);
            }
            Msg::ClearSelection => self.selected.clear(),
            Msg::Act(action) => {
                let pkgs: Vec<String> = self.selected.iter().cloned().collect();
                return match action.confirmation(&pkgs) {
                    Some((title, question)) => ui::confirm(title, question, move |yes| Msg::ActConfirmed(action, pkgs, yes)),
                    None => self.update(Msg::ActConfirmed(action, pkgs, true), ctx),
                };
            }
            Msg::ActConfirmed(_, _, false) => {}
            Msg::ActConfirmed(action, pkgs, true) => {
                self.busy = true;
                let title = format!("{} {} package(s)", action.label(), pkgs.len());
                return ctx.job(title, Unit::Items, move |conn, p| packages::run(conn, action, pkgs, p), Msg::Done);
            }
            Msg::SaveApk => {
                let pkgs: Vec<String> = self.selected.iter().cloned().collect();
                let title = match pkgs.as_slice() {
                    [one] => format!("Save APK for {one}: choose a folder"),
                    many => format!("Save APKs for {} packages: choose a folder", many.len()),
                };
                let pick = ui::pick_folder(title);
                return Task::perform(async move { (pkgs, pick.await) }, |(pkgs, dir)| Msg::SaveApkTo(pkgs, dir));
            }
            Msg::SaveApkTo(_, None) => {}
            Msg::SaveApkTo(pkgs, Some(dir)) => {
                ctx.log(format!("Saving {} APK(s) to {}…", pkgs.len(), dir.display()));
                self.busy = true;
                let title = format!("Saving {} APK(s)", pkgs.len());
                return ctx.job(title, Unit::Bytes, move |conn, p| packages::save_apks(conn, pkgs, dir, p), Msg::Done);
            }
            Msg::LoadNames => return self.fetch_names(ctx),
            Msg::NamesLoaded(id, result) => {
                ctx.jobs.finish(id);
                self.fetching_names = false;
                match result {
                    Ok(found) => {
                        let named = found.values().filter(|n| !n.is_empty()).count();
                        ctx.log(format!("Read {named} app name(s)."));
                        let names = self.names.get_or_insert_with(labels::load_cache);
                        names.extend(found);
                        labels::save_cache(names);
                    }
                    Err(e) => ctx.log(format!("Couldn't read app names: {e}")),
                }
            }
            Msg::Done(id, result) => {
                ctx.jobs.finish(id);
                self.busy = false;
                match result {
                    Ok(lines) => lines.into_iter().for_each(|l| ctx.log(l)),
                    Err(e) => ctx.log(format!("Failed: {e}")),
                }
                // Reload so uninstalled/disabled state shows.
                if let Some(kind) = self.kind {
                    return self.list(kind, ctx);
                }
            }
        }
        Task::none()
    }

    pub fn view<'a>(&'a self, v: &View<'a>) -> Element<'a, Msg> {
        let sdk = v.android().map(|s| s.sdk()).unwrap_or(0);
        let chips = row(ListKind::ALL.iter().map(|&kind| {
            button(text(kind.label()).size(12))
                .padding([5, 12])
                .style(theme::tab_button(self.kind == Some(kind)))
                .on_press_maybe((!self.loading).then_some(Msg::List(kind)))
                .into()
        }))
        .spacing(4);
        let top = row![
            chips,
            space::horizontal(),
            text_input("Filter packages…", &self.filter).on_input(Msg::Filter).size(13).width(200),
        ]
        .align_y(Alignment::Center)
        .spacing(8);

        let names = self.names.as_ref();
        let shown: Vec<&Package> = visible(&self.packages, names, &self.filter).collect();
        let list: Element<'a, Msg> = if self.loading {
            ui::empty_state("Loading packages…", "")
        } else if self.kind.is_none() {
            ui::empty_state("No package list loaded", "Choose a list above, e.g. All or User.")
        } else if self.packages.is_empty() {
            ui::empty_state("No packages in this list", "")
        } else {
            let header = row![
                text("").width(24),
                text("Package").size(11).color(theme::TEXT_MUTED).width(Fill),
                text("App Name").size(11).color(theme::TEXT_MUTED).width(220),
                text("Version").size(11).color(theme::TEXT_MUTED).width(140),
            ]
            .padding([0, 10]);
            let rows = shown.iter().map(|p| {
                let selected = self.selected.contains(&p.id);
                let name = display_name(p, names);
                let name = if p.disabled { format!("{name} (disabled)") } else { name.to_owned() };
                let line = row![
                    text(if selected { "☑" } else { "☐" }).size(13).width(24),
                    text(p.id.clone()).size(12).font(Font::MONOSPACE).width(Fill),
                    text(name).size(12).color(if p.disabled { theme::TEXT_DISABLED } else { theme::TEXT }).width(220),
                    text(p.version.clone()).size(12).font(Font::MONOSPACE).color(theme::TEXT_MUTED).width(140),
                ]
                .align_y(Alignment::Center);
                button(line)
                    .padding([3, 10])
                    .width(Fill)
                    .style(theme::tab_button(selected))
                    .on_press(Msg::Toggle(p.id.clone()))
                    .into()
            });
            column![header, scrollable(column(rows).spacing(1)).height(Fill)].spacing(4).into()
        };
        let list = container(list).padding(8).width(Fill).height(Fill).style(theme::log_box);

        let n = self.selected.len();
        let can_act = n > 0 && !self.busy && !self.loading;
        let action = |a: Action| -> Element<'a, Msg> {
            let cache_unsupported = a == Action::ClearCache && sdk < packages::CACHE_ONLY_API;
            let style = if a.destructive() { theme::danger_button } else { theme::secondary_button };
            let b = button(text(a.label()).size(12))
                .padding([6, 12])
                .style(style)
                .on_press_maybe((can_act && !cache_unsupported).then_some(Msg::Act(a)));
            if cache_unsupported {
                tooltip(
                    b,
                    container(text("Needs Android 14 or newer. Use Clear All App Caches in Performance instead.").size(11))
                        .padding(8)
                        .style(theme::card),
                    tooltip::Position::Top,
                )
                .into()
            } else {
                b.into()
            }
        };
        let actions = row(Action::ALL.iter().map(|&a| action(a)))
            .push(
                button(text("Save APK").size(12))
                    .padding([6, 12])
                    .style(theme::primary_button)
                    .on_press_maybe(can_act.then_some(Msg::SaveApk)),
            )
            .spacing(4);
        let status = if self.busy { "Working…".to_owned() } else { format!("{n} selected · {} shown", shown.len()) };
        let unnamed = if self.loading { 0 } else { self.missing_names().len() };
        let load_names: Element<'a, Msg> = if unnamed > 0 && !self.fetching_names {
            button(text(format!("Load real names ({unnamed}, about {} min)", (unnamed * 13 / 10).div_ceil(60))).size(12))
                .padding([4, 10])
                .style(theme::tab_button(false))
                .on_press(Msg::LoadNames)
                .into()
        } else {
            space().into()
        };
        let selection = row![
            text(status).size(12).color(theme::TEXT_MUTED),
            space::horizontal(),
            load_names,
            button(text("Select shown").size(12))
                .padding([4, 10])
                .style(theme::tab_button(false))
                .on_press_maybe((!shown.is_empty()).then_some(Msg::SelectShown)),
            button(text("Clear").size(12))
                .padding([4, 10])
                .style(theme::tab_button(false))
                .on_press_maybe((n > 0).then_some(Msg::ClearSelection)),
        ]
        .align_y(Alignment::Center)
        .spacing(4);

        column![top, list, selection, actions].spacing(8).padding(4).into()
    }
}
