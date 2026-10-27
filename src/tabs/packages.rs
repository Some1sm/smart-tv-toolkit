//! Packages: every app on the TV in a grouped grid, a "⋯" inspector per app
//! and a command bar for acting on the selection.

use std::collections::BTreeSet;
use std::path::PathBuf;

use iced::widget::{Space, button, checkbox, column, container, mouse_area, opaque, responsive, row, scrollable, space, stack, tooltip};
use iced::{Alignment, Color, Element, Length, Padding, Task};

use super::{Ctx, View};
use crate::android::changes::Outcome;
use crate::android::labels::{self, Cache};
use crate::android::packages::{self, Action, Details, Group, ListKind, Origin, Package};
use crate::android::sysinfo::fmt_bytes;
use crate::design::{self as ds, alpha, style};
use crate::jobs::{JobId, Unit};
use crate::log::Level;
use crate::ui;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Column {
    Name,
    Package,
    Origin,
    Version,
    Size,
    Status,
}

#[derive(Debug)]
struct Inspector {
    pkg: String,
    details: Option<Result<Details, String>>,
    /// Showing the permission list instead of the actions.
    permissions: bool,
}

#[derive(Debug)]
pub struct State {
    packages: Vec<Package>,
    loading: bool,
    kind: ListKind,
    filter: String,
    selected: BTreeSet<String>,
    collapsed: BTreeSet<Group>,
    sort: Column,
    descending: bool,
    inspector: Option<Inspector>,
    busy: bool,
    /// Real app names by `package@version`, loaded from disk on first use.
    names: Option<Cache>,
    /// The grid's scroll position and height, to draw only rows in view.
    scroll_y: f32,
    view_height: f32,
    /// App icons saved on this computer, by `package@version`.
    icons: Option<std::collections::HashMap<String, Option<iced::widget::image::Handle>>>,
    fetching_names: bool,
}

impl Default for State {
    fn default() -> Self {
        State {
            packages: Vec::new(),
            loading: false,
            kind: ListKind::All,
            filter: String::new(),
            selected: BTreeSet::new(),
            collapsed: BTreeSet::new(),
            sort: Column::Name,
            descending: false,
            inspector: None,
            busy: false,
            names: None,
            icons: None,
            scroll_y: 0.0,
            view_height: 900.0,
            fetching_names: false,
        }
    }
}

#[derive(Debug, Clone)]
pub enum Msg {
    Load,
    Loaded(JobId, Result<Result<Vec<Package>, String>, String>),
    Kind(ListKind),
    Filter(String),
    Sort(Column),
    Toggle(String),
    ToggleGroup(Group),
    ToggleShown,
    Collapse(Group),
    SelectShown,
    ClearSelection,
    /// Act on the selection.
    Act(Action),
    /// Act on one package (row buttons and the inspector).
    ActOn(Action, String),
    ActConfirmed(Action, Vec<String>, bool),
    SaveApk,
    SaveApkFor(Vec<String>),
    SaveApkTo(Vec<String>, Option<PathBuf>),
    Backup,
    BackupTo(Vec<(String, String, String)>, Option<PathBuf>),
    Done(JobId, Result<Outcome, String>),
    BatchDone(JobId, super::Target, Result<Outcome, String>),
    LoadNames,
    NamesLoaded(JobId, Result<Cache, String>),
    Inspect(String),
    Inspected(String, Result<Details, String>),
    CloseInspector,
    Permissions,
    Launch(String),
    Launched(String),
    /// The grid scrolled: (offset, visible height).
    Scrolled(f32, f32),
}

/// The real name if it's known, else the one derived from the package id.
fn display_name<'a>(p: &'a Package, names: Option<&'a Cache>) -> &'a str {
    names
        .and_then(|n| n.get(&labels::key(&p.id, &p.version)))
        .filter(|n| !n.is_empty())
        .map_or(p.name.as_str(), String::as_str)
}

fn status(p: &Package) -> (&'static str, Color) {
    if !p.installed {
        ("Removed", ds::TEXT_TERTIARY)
    } else if p.disabled {
        ("Disabled", ds::AMBER)
    } else if p.running {
        ("Running", ds::GREEN)
    } else {
        ("Idle", ds::TEXT_SECONDARY)
    }
}

/// Ctrl+A, Del, Ctrl+S and Esc while the Packages tab is open. Keys a
/// focused field used (e.g. Ctrl+A in the search box) never get here.
pub fn key_for(event: iced::keyboard::Event) -> Option<Msg> {
    use iced::keyboard::{Event, Key, key::Named};
    let Event::KeyPressed { key, modifiers, repeat: false, .. } = event else { return None };
    match key.as_ref() {
        Key::Character("a" | "A") if modifiers.command() => Some(Msg::SelectShown),
        Key::Character("s" | "S") if modifiers.command() => Some(Msg::SaveApk),
        Key::Named(Named::Delete) => Some(Msg::Act(Action::Uninstall)),
        Key::Named(Named::Escape) => Some(Msg::CloseInspector),
        _ => None,
    }
}

impl State {
    pub fn reset(&mut self) {
        *self = State::default();
    }

    /// The tab bar's badge: how many packages are installed.
    pub fn count(&self) -> Option<usize> {
        (!self.packages.is_empty()).then(|| self.packages.iter().filter(|p| p.installed).count())
    }

    pub fn on_open(&mut self, ctx: &mut Ctx) -> Task<Msg> {
        if self.packages.is_empty() && !self.loading { self.load(ctx) } else { Task::none() }
    }

    fn load(&mut self, ctx: &mut Ctx) -> Task<Msg> {
        let Some(session) = ctx.android() else { return Task::none() };
        self.loading = true;
        ctx.log.cmd("pm list packages -f -i -u");
        let maker = session.summary.manufacturer.clone();
        ctx.job("Reading packages", Unit::Items, move |conn, _| packages::inventory(conn, maker), Msg::Loaded)
    }

    /// Packages passing the chip and the search box, sorted.
    fn shown(&self) -> Vec<&Package> {
        let names = self.names.as_ref();
        let q = self.filter.trim().to_lowercase();
        let mut list: Vec<&Package> = self
            .packages
            .iter()
            .filter(|p| self.kind.matches(p))
            .filter(|p| q.is_empty() || p.id.to_lowercase().contains(&q) || display_name(p, names).to_lowercase().contains(&q))
            .collect();
        list.sort_by(|a, b| {
            let order = match self.sort {
                Column::Name => display_name(a, names).to_lowercase().cmp(&display_name(b, names).to_lowercase()),
                Column::Package => a.id.cmp(&b.id),
                Column::Origin => a.origin.label().cmp(b.origin.label()),
                Column::Version => a.version.cmp(&b.version),
                Column::Size => a.size.cmp(&b.size),
                Column::Status => status(a).0.cmp(status(b).0),
            };
            if self.descending { order.reverse() } else { order }
        });
        list
    }

    /// Listed packages whose real name hasn't been looked up yet. System
    /// apps run to hundreds and take a while, so they wait for a click.
    fn missing_names(&self, system: bool) -> Vec<(String, String)> {
        let names = self.names.as_ref();
        let icons = self.icons.as_ref();
        self.packages
            .iter()
            .filter(|p| p.installed && (p.group() == Group::System) == system)
            .filter(|p| {
                names.is_none_or(|n| !n.contains_key(&labels::key(&p.id, &p.version)))
                    || icons.is_none_or(|i| !i.contains_key(&labels::icon_key(&p.id, &p.version)))
            })
            .map(|p| (p.id.clone(), p.version.clone()))
            .collect()
    }

    /// (Re)read the icons saved on this computer.
    fn load_icons(&mut self) {
        self.icons = Some(
            labels::icons_on_disk()
                .into_iter()
                .map(|(k, path)| (k, path.map(iced::widget::image::Handle::from_path)))
                .collect(),
        );
    }

    fn icon(&self, p: &Package) -> Option<&iced::widget::image::Handle> {
        self.icons.as_ref()?.get(&labels::icon_key(&p.id, &p.version))?.as_ref()
    }

    fn fetch_names(&mut self, wanted: Vec<(String, String)>, ctx: &mut Ctx) -> Task<Msg> {
        if wanted.is_empty() || self.fetching_names {
            return Task::none();
        }
        self.fetching_names = true;
        let title = format!("Reading app names ({})", wanted.len());
        ctx.job(title, Unit::Items, move |conn, p| labels::fetch(conn, wanted, p), Msg::NamesLoaded)
    }

    fn act(&mut self, action: Action, pkgs: Vec<String>, ctx: &mut Ctx) -> Task<Msg> {
        if pkgs.is_empty() || self.busy {
            return Task::none();
        }
        let also = if ctx.batch.is_empty() { String::new() } else { format!("\n\nAlso on: {}", ctx.batch_names()) };
        match action.confirmation(&pkgs) {
            Some((title, question)) => ui::confirm(title, format!("{question}{also}"), move |yes| Msg::ActConfirmed(action, pkgs, yes)),
            None => self.update(Msg::ActConfirmed(action, pkgs, true), ctx),
        }
    }

    pub fn update(&mut self, msg: Msg, ctx: &mut Ctx) -> Task<Msg> {
        match msg {
            Msg::Load => return self.load(ctx),
            Msg::Loaded(id, result) => {
                ctx.jobs.finish(id);
                self.loading = false;
                match result.and_then(|r| r) {
                    Ok(list) if ctx.session.is_some() => {
                        let installed = list.iter().filter(|p| p.installed).count();
                        ctx.log(format!("Listed {installed} installed package(s), {} restorable.", list.len() - installed));
                        let ids: BTreeSet<&String> = list.iter().map(|p| &p.id).collect();
                        self.selected.retain(|p| ids.contains(p));
                        if let Some(i) = &self.inspector
                            && !ids.contains(&i.pkg)
                        {
                            self.inspector = None;
                        }
                        self.packages = list;
                        self.names.get_or_insert_with(labels::load_cache);
                        self.load_icons();
                        // The user's own apps are few: name them right away.
                        let wanted = self.missing_names(false);
                        return self.fetch_names(wanted, ctx);
                    }
                    Ok(_) => {}
                    Err(e) => ctx.log(format!("Could not list packages: {e}")),
                }
            }
            Msg::Kind(kind) => self.kind = kind,
            Msg::Filter(f) => self.filter = f,
            Msg::Sort(column) => {
                self.descending = self.sort == column && !self.descending;
                self.sort = column;
            }
            Msg::Toggle(pkg) => {
                if !self.selected.remove(&pkg) {
                    self.selected.insert(pkg);
                }
            }
            Msg::ToggleGroup(group) => {
                let ids: Vec<String> = self.shown().into_iter().filter(|p| p.group() == group).map(|p| p.id.clone()).collect();
                self.toggle_all(ids);
            }
            Msg::ToggleShown => {
                let ids: Vec<String> = self.shown().into_iter().map(|p| p.id.clone()).collect();
                self.toggle_all(ids);
            }
            Msg::Collapse(group) => {
                if !self.collapsed.remove(&group) {
                    self.collapsed.insert(group);
                }
            }
            Msg::SelectShown => {
                let shown: Vec<String> = self.shown().into_iter().map(|p| p.id.clone()).collect();
                self.selected.extend(shown);
            }
            Msg::ClearSelection => self.selected.clear(),
            Msg::Act(action) => {
                let pkgs: Vec<String> = self.selected.iter().cloned().collect();
                return self.act(action, pkgs, ctx);
            }
            Msg::ActOn(action, pkg) => return self.act(action, vec![pkg], ctx),
            Msg::ActConfirmed(_, _, false) => {}
            Msg::ActConfirmed(action, pkgs, true) => {
                self.busy = true;
                ctx.log.push_at(Level::Select, format!("Selected: [{}] · {}", pkgs.join(", "), action.label()));
                let title = format!("{} {} package(s)", action.label(), pkgs.len());
                let others = pkgs.clone();
                let batch = ctx.job_on_batch(&title, Unit::Items, move |conn, p| packages::run(conn, action, others.clone(), p), Msg::BatchDone);
                let here = ctx.job(title, Unit::Items, move |conn, p| packages::run(conn, action, pkgs, p), Msg::Done);
                return Task::batch([here, batch]);
            }
            Msg::SaveApk => {
                let pkgs: Vec<String> = self.selected.iter().cloned().collect();
                return self.update(Msg::SaveApkFor(pkgs), ctx);
            }
            Msg::SaveApkFor(pkgs) => {
                if pkgs.is_empty() || self.busy {
                    return Task::none();
                }
                let title = match pkgs.as_slice() {
                    [one] => format!("Save APK for {one}: choose a folder"),
                    many => format!("Save APKs for {} packages: choose a folder", many.len()),
                };
                let pick = ui::pick_folder(title);
                return Task::perform(async move { (pkgs, pick.await) }, |(pkgs, dir)| Msg::SaveApkTo(pkgs, dir));
            }
            Msg::SaveApkTo(_, None) => {}
            Msg::SaveApkTo(pkgs, Some(dir)) => {
                ctx.log.cmd(format!("adb pull ({} package(s)) → {}", pkgs.len(), dir.display()));
                self.busy = true;
                let title = format!("Saving {} APK(s)", pkgs.len());
                return ctx.job(title, Unit::Bytes, move |conn, p| saving(conn, pkgs, dir, p), Msg::Done);
            }
            Msg::Backup => {
                if self.selected.is_empty() || self.busy {
                    return Task::none();
                }
                let names = self.names.as_ref();
                let rows: Vec<(String, String, String)> = self
                    .packages
                    .iter()
                    .filter(|p| self.selected.contains(&p.id))
                    .map(|p| (p.id.clone(), p.version.clone(), display_name(p, names).to_owned()))
                    .collect();
                let pick = ui::pick_folder(format!("Back up {} app(s): choose a folder", rows.len()));
                return Task::perform(async move { (rows, pick.await) }, |(rows, dir)| Msg::BackupTo(rows, dir));
            }
            Msg::BackupTo(_, None) => {}
            Msg::BackupTo(rows, Some(dir)) => {
                let dir = dir.join(format!("smart-tv-backup-{}", chrono::Local::now().format("%Y%m%d-%H%M%S")));
                if let Err(e) = std::fs::create_dir_all(&dir) {
                    ctx.log(format!("Could not create {}: {e}", dir.display()));
                    return Task::none();
                }
                let list: String = rows.iter().map(|(id, version, name)| format!("{id}\t{version}\t{name}\n")).collect();
                if let Err(e) = std::fs::write(dir.join("packages.txt"), list) {
                    ctx.log(format!("Could not write the package list: {e}"));
                }
                ctx.log.cmd(format!("adb pull ({} package(s)) → {}", rows.len(), dir.display()));
                self.busy = true;
                let pkgs: Vec<String> = rows.into_iter().map(|(id, _, _)| id).collect();
                let title = format!("Backing up {} app(s)", pkgs.len());
                return ctx.job(title, Unit::Bytes, move |conn, p| saving(conn, pkgs, dir, p), Msg::Done);
            }
            Msg::LoadNames => {
                let wanted = self.missing_names(true);
                return self.fetch_names(wanted, ctx);
            }
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
                        self.load_icons();
                    }
                    Err(e) => ctx.log(format!("Couldn't read app names: {e}")),
                }
            }
            Msg::Done(id, result) => {
                ctx.jobs.finish(id);
                self.busy = false;
                match result {
                    Ok(outcome) => ctx.apply(outcome),
                    Err(e) => ctx.log(format!("Failed: {e}")),
                }
                // Reload so uninstalled/disabled/stopped state shows.
                if let Some(i) = &mut self.inspector {
                    i.details = None;
                }
                let reopen = self.inspector.as_ref().map(|i| i.pkg.clone());
                let reload = self.load(ctx);
                return match reopen.and_then(|pkg| ctx.conn().map(|c| (pkg, c))) {
                    Some((pkg, conn)) => Task::batch([reload, inspect(pkg, conn)]),
                    None => reload,
                };
            }
            Msg::BatchDone(id, target, result) => {
                ctx.jobs.finish(id);
                match result {
                    Ok(outcome) => ctx.apply_on(&target, outcome),
                    Err(e) => ctx.log(format!("[{}] Failed: {e}", target.label)),
                }
            }
            Msg::Inspect(pkg) => {
                if self.inspector.as_ref().is_some_and(|i| i.pkg == pkg) {
                    self.inspector = None;
                    return Task::none();
                }
                let Some(conn) = ctx.conn() else { return Task::none() };
                self.inspector = Some(Inspector { pkg: pkg.clone(), details: None, permissions: false });
                return inspect(pkg, conn);
            }
            Msg::Inspected(pkg, result) => {
                if let Some(i) = self.inspector.as_mut().filter(|i| i.pkg == pkg) {
                    i.details = Some(result);
                }
            }
            Msg::CloseInspector => self.inspector = None,
            Msg::Permissions => {
                if let Some(i) = &mut self.inspector {
                    i.permissions = !i.permissions;
                }
            }
            Msg::Launch(pkg) => {
                let Some(conn) = ctx.conn() else { return Task::none() };
                ctx.log.cmd(format!("monkey -p {pkg} -c LEANBACK_LAUNCHER 1"));
                return Task::perform(packages::launch(conn, pkg), Msg::Launched);
            }
            Msg::Launched(line) => ctx.log(line),
            Msg::Scrolled(y, height) => {
                self.scroll_y = y;
                self.view_height = height;
            }
        }
        Task::none()
    }

    /// Select every id in `ids`, or clear them all if they already are.
    fn toggle_all(&mut self, ids: Vec<String>) {
        if ids.iter().all(|id| self.selected.contains(id)) {
            ids.iter().for_each(|id| {
                self.selected.remove(id);
            });
        } else {
            self.selected.extend(ids);
        }
    }

    pub fn view<'a>(&'a self, v: &View<'a>) -> Element<'a, Msg> {
        let sdk = v.android().map(|s| s.sdk()).unwrap_or(0);
        let shown = self.shown();
        let body: Element<'a, Msg> = if self.packages.is_empty() {
            let (title, subtitle) =
                if self.loading { ("Reading packages…", "Listing apps, versions and sizes from the TV.") } else { ("No packages loaded", "Press Refresh to read the TV's apps.") };
            container(ui::empty_state(title, subtitle)).style(style::fill(ds::BG)).into()
        } else {
            let rows = shown.clone();
            let grid: Element<'a, Msg> = responsive(move |size| self.grid(&rows, sdk, Columns::fitting(size.width))).into();
            match &self.inspector {
                Some(inspector) => stack![
                    grid,
                    // Light dismiss: a click outside the flyout closes it.
                    mouse_area(Space::new().width(Length::Fill).height(Length::Fill)).on_press(Msg::CloseInspector),
                    container(opaque(self.flyout(inspector, sdk)))
                        .align_right(Length::Fill)
                        .padding(Padding::default().top(48).right(48)),
                ]
                .into(),
                None => grid,
            }
        };
        let batch = v.batch_note();
        let bar = responsive(move |size| self.command_bar(&shown, sdk, Fit::for_width(size.width), batch.as_deref()));
        column![self.toolbar(), container(body).height(Length::Fill), container(bar).height(ds::COMMAND_BAR_HEIGHT)].into()
    }

    /// Filter pills with counts, refresh and search.
    fn toolbar(&self) -> Element<'_, Msg> {
        container(column![responsive(move |size| self.toolbar_strip(size.width)), ds::hline(ds::BORDER)])
            .height(ds::TOOLBAR_HEIGHT)
            .style(style::fill(ds::SIDEBAR))
            .into()
    }

    /// The toolbar's contents; the search box narrows before the pills do.
    fn toolbar_strip(&self, width: f32) -> Element<'_, Msg> {
        let pills = row(ListKind::ALL.iter().map(|&kind| {
            let active = kind == self.kind;
            let n = self.packages.iter().filter(|p| kind.matches(p)).count();
            let caption = if active { ds::medium(kind.label(), ds::TEXT_BASE, ds::TEXT) } else { ds::label(kind.label(), ds::TEXT_BASE, ds::TEXT_SECONDARY) };
            button(row![caption, ds::mono(n.to_string(), ds::TEXT_XS, ds::TEXT_TERTIARY)].spacing(4).align_y(Alignment::Center))
                .height(24)
                .padding([0, 10])
                .style(style::pill(active))
                .on_press(Msg::Kind(kind))
                .into()
        }))
        .spacing(4)
        .align_y(Alignment::Center);
        let (search_width, placeholder) = if width >= 1000.0 {
            (256.0, "Filter by name or package…")
        } else if width >= 820.0 {
            (180.0, "Filter by name…")
        } else {
            (130.0, "Filter…")
        };
        let search = container(
            row![
                ds::icon_any(ds::Icon::Search, 14.0, ds::TEXT_TERTIARY),
                ds::input(placeholder, &self.filter, false, ds::INSET)
                    .on_input(Msg::Filter)
                    .style(style::bare_input)
                    .padding(Padding::from([5, 0])),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        )
        .padding(Padding::from([0, 10]))
        .width(search_width)
        .height(ds::CONTROL_HEIGHT)
        .style(style::panel(ds::INSET, ds::BORDER, ds::RADIUS));
        let refresh = ds::square_button(ds::Icon::RefreshCw, ds::TEXT_SECONDARY, ds::CONTROL_HEIGHT, style::ghost, (!self.loading).then_some(Msg::Load));
        container(row![pills, space::horizontal(), refresh, search].spacing(8).align_y(Alignment::Center))
            .padding([0, 12])
            .center_y(Length::Fill)
            .into()
    }

    fn grid<'a>(&'a self, shown: &[&'a Package], sdk: u32, cols: Columns) -> Element<'a, Msg> {
        let names = self.names.as_ref();
        let all_selected = !shown.is_empty() && shown.iter().all(|p| self.selected.contains(&p.id));
        let header_cell = |title: &'a str, column: Column, width: Length| -> Element<'a, Msg> {
            let active = self.sort == column;
            let mut content = row![ds::medium(title, ds::TEXT_SM, ds::TEXT_SECONDARY)].spacing(4).align_y(Alignment::Center);
            if active {
                content = content.push(ds::icon_any(if self.descending { ds::Icon::ArrowUp } else { ds::Icon::ArrowDown }, 12.0, ds::ACCENT));
            }
            button(row![container(content).padding([0, 12]).width(Length::Fill).center_y(Length::Fill), container(ds::vline(16.0)).center_y(Length::Fill)].height(Length::Fill))
                .width(width)
                .height(Length::Fill)
                .padding(0)
                .style(style::header_cell)
                .on_press(Msg::Sort(column))
                .into()
        };
        let mut titles = row![
            container(checkbox(all_selected).on_toggle(|_| Msg::ToggleShown).size(14).style(style::checkbox))
                .width(COL_CHECK)
                .padding(Padding::default().left(12))
                .center_y(Length::Fill),
            header_cell("Name", Column::Name, COL_NAME),
            header_cell("Package Identifier", Column::Package, COL_PACKAGE),
        ];
        if cols.origin {
            titles = titles.push(header_cell("Origin", Column::Origin, COL_ORIGIN.into()));
        }
        if cols.version {
            titles = titles.push(header_cell("Version", Column::Version, COL_VERSION.into()));
        }
        let actions_width = cols.actions_width();
        let titles = titles.push(header_cell("Size", Column::Size, COL_SIZE.into())).push(header_cell("Status", Column::Status, COL_STATUS.into())).push(
            container(ds::medium(if cols.row_buttons { "Actions" } else { "" }, ds::TEXT_SM, ds::TEXT_SECONDARY))
                .width(actions_width)
                .align_right(actions_width)
                .padding(Padding::default().right(16))
                .center_y(Length::Fill),
        );
        let header = container(titles)
        .height(32)
        .style(style::fill(ds::GRID_HEADER));

        // Only rows near the visible part are built; spacers stand in for
        // the rest so the scrollbar stays true (rows have fixed heights).
        let (top, bottom) = (self.scroll_y - VIRTUAL_MARGIN, self.scroll_y + self.view_height + VIRTUAL_MARGIN);
        let mut rows = column![];
        let mut any = false;
        let (mut y, mut skipped) = (0.0_f32, 0.0_f32);
        let mut place = |rows: iced::widget::Column<'a, Msg>, height: f32, build: &dyn Fn() -> Element<'a, Msg>| {
            let visible = y + height >= top && y <= bottom;
            y += height;
            if visible {
                let rows = if skipped > 0.0 { rows.push(Space::new().height(skipped)) } else { rows };
                skipped = 0.0;
                rows.push(build())
            } else {
                skipped += height;
                rows
            }
        };
        for group in Group::ALL {
            let members: Vec<&Package> = shown.iter().copied().filter(|p| p.group() == group).collect();
            if members.is_empty() {
                continue;
            }
            any = true;
            rows = place(rows, GROUP_ROW_HEIGHT, &|| self.group_row(group, &members));
            if self.collapsed.contains(&group) {
                continue;
            }
            for p in members {
                rows = place(rows, PACKAGE_ROW_HEIGHT, &|| {
                    column![ds::hline(ds::DIVIDER), self.package_row(p, display_name(p, names), sdk, cols)].into()
                });
            }
        }
        if skipped > 0.0 {
            rows = rows.push(Space::new().height(skipped));
        }
        let body: Element<'a, Msg> = if any {
            scrollable(rows)
                .direction(ds::thin_scrollbar())
                .style(style::scrollbar)
                .height(Length::Fill)
                .on_scroll(|v| Msg::Scrolled(v.absolute_offset().y, v.bounds().height))
                .into()
        } else {
            ui::empty_state("Nothing matches", "Try another filter or clear the search.")
        };
        container(column![header, ds::hline(ds::BORDER), body]).width(Length::Fill).height(Length::Fill).style(style::fill(ds::BG)).into()
    }

    fn group_row<'a>(&'a self, group: Group, members: &[&Package]) -> Element<'a, Msg> {
        let picked = members.iter().filter(|p| self.selected.contains(&p.id)).count();
        let collapsed = self.collapsed.contains(&group);
        let mut right = row![].spacing(14).align_y(Alignment::Center);
        if group == Group::System && !self.fetching_names {
            let unnamed = self.missing_names(true).len();
            if unnamed > 0 {
                right = right.push(
                    button(ds::label(crate::i18n::trf("Load real names ({}, ~{} min)", &[&unnamed, &(unnamed * 13 / 10).div_ceil(60)]), ds::TEXT_SM, ds::TEXT_SECONDARY))
                        .padding(0)
                        .style(style::link(ds::TEXT))
                        .on_press(Msg::LoadNames),
                );
            }
        }
        right = right.push(
            button(ds::label(if collapsed { "Expand Group" } else { "Collapse Group" }, ds::TEXT_SM, ds::TEXT_SECONDARY))
                .padding(0)
                .style(style::link(ds::TEXT))
                .on_press(Msg::Collapse(group)),
        );
        let content = row![
            container(checkbox(picked == members.len()).on_toggle(move |_| Msg::ToggleGroup(group)).size(14).style(style::checkbox))
                .width(COL_CHECK)
                .padding(Padding::default().left(12)),
            ds::icon_any(if collapsed { ds::Icon::ChevronRight } else { ds::Icon::ChevronDown }, 14.0, ds::TEXT_SECONDARY),
            ds::semibold(group.title(), 11.5, ds::TEXT),
            ds::count(members.len(), ds::BADGE),
            ds::label(crate::i18n::trf("{} selected", &[&picked]), ds::TEXT_SM, ds::TEXT_TERTIARY),
            space::horizontal(),
            right,
        ]
        .spacing(8)
        .padding(Padding::default().right(16))
        .align_y(Alignment::Center);
        column![
            ds::hline(ds::BORDER),
            button(container(content).height(34).center_y(34)).padding(0).width(Length::Fill).style(style::grid_row(false, true)).on_press(Msg::Collapse(group)),
        ]
        .into()
    }

    fn package_row<'a>(&'a self, p: &'a Package, name: &'a str, sdk: u32, cols: Columns) -> Element<'a, Msg> {
        let selected = self.selected.contains(&p.id);
        let inspecting = self.inspector.as_ref().is_some_and(|i| i.pkg == p.id);
        let (state, state_color) = status(p);
        let cell = |content: Element<'a, Msg>, width: Length| container(content).padding([0, 12]).width(width).center_y(Length::Fill).clip(true);
        let mono = |text: String, color: Color| -> Element<'a, Msg> { ds::mono(text, ds::TEXT_SM, color).wrapping(iced::widget::text::Wrapping::None).into() };

        // Row buttons: Disable/Enable (Restore when removed), Clear Cache, ⋯.
        let (toggle, toggle_action) = if !p.installed {
            ("Restore", Action::Restore)
        } else if p.disabled {
            ("Enable", Action::Enable)
        } else {
            ("Disable", Action::Disable)
        };
        let busy = self.busy;
        let small = |caption: &'a str, msg: Option<Msg>| {
            button(container(crate::ui::text(caption).size(ds::TEXT_SM)).center_y(Length::Fill))
                .height(24)
                .padding([0, 8])
                .style(style::outline)
                .on_press_maybe(msg.filter(|_| !busy))
        };
        let _ = sdk;
        let cache: Element<'a, Msg> = small("Clear Cache", p.installed.then(|| Msg::ActOn(Action::ClearCache, p.id.clone()))).into();
        let more = button(container(ds::icon_any(ds::Icon::Ellipsis, 14.0, if inspecting { ds::ACCENT } else { ds::TEXT_SECONDARY })).center(Length::Fill))
            .width(24)
            .height(24)
            .padding(0)
            .style(if inspecting { style::outline_active } else { style::outline })
            .on_press(Msg::Inspect(p.id.clone()));
        // Narrow grids keep only "⋯"; the inspector has the same actions.
        let actions = if cols.row_buttons {
            row![small(toggle, Some(Msg::ActOn(toggle_action, p.id.clone()))), cache, more]
        } else {
            row![more]
        }
        .spacing(4)
        .align_y(Alignment::Center);
        let actions_width = cols.actions_width();

        let name_color = if p.installed { ds::TEXT } else { ds::TEXT_SECONDARY };
        let mut line = row![
            container(Space::new()).width(2).height(Length::Fill).style(if selected { style::accent_fill } else { style::clear }),
            container(checkbox(selected).on_toggle(|_| Msg::Toggle(p.id.clone())).size(14).style(style::checkbox))
                .width(COL_CHECK - 2.0)
                .padding(Padding::default().left(10))
                .center_y(Length::Fill),
            cell(
                row![
                    match self.icon(p) {
                        Some(handle) => iced::widget::image(handle.clone()).width(18).height(18).into(),
                        None => ds::avatar(name, 16.0, if p.running { ds::GREEN } else { ds::TEXT_SECONDARY }),
                    },
                    ds::medium(name, ds::TEXT_BASE, name_color).wrapping(iced::widget::text::Wrapping::None),
                    rating_tag(p),
                ]
                .spacing(8)
                .padding(Padding::default().left(12))
                .align_y(Alignment::Center)
                .into(),
                COL_NAME,
            ),
            cell(mono(p.id.clone(), ds::TEXT_SECONDARY), COL_PACKAGE),
        ];
        if cols.origin {
            line = line.push(cell(ds::label(p.origin.label(), ds::TEXT_BASE, ds::TEXT_SECONDARY).into(), COL_ORIGIN.into()));
        }
        if cols.version {
            let version = if p.version.is_empty() { "—".into() } else { format!("v{}", p.version) };
            line = line.push(cell(mono(version, ds::TEXT_SECONDARY), COL_VERSION.into()));
        }
        let line = line
            .push(cell(mono(p.size.map(fmt_bytes).unwrap_or_else(|| "—".into()), ds::TEXT), COL_SIZE.into()))
            .push(cell(
                if p.running { ds::medium(state, ds::TEXT_BASE, state_color) } else { ds::label(state, ds::TEXT_BASE, state_color) }.into(),
                COL_STATUS.into(),
            ))
            .push(container(actions).width(actions_width).align_right(actions_width).padding(Padding::default().right(16)).center_y(Length::Fill))
            .height(36);
        button(line).padding(0).width(Length::Fill).style(style::grid_row(selected, false)).on_press(Msg::Toggle(p.id.clone())).into()
    }

    /// The "⋯" inspector: identity, metadata grid and per-app actions.
    fn flyout<'a>(&'a self, inspector: &'a Inspector, sdk: u32) -> Element<'a, Msg> {
        let Some(p) = self.packages.iter().find(|p| p.id == inspector.pkg) else { return Space::new().into() };
        let name = display_name(p, self.names.as_ref());
        let details = inspector.details.as_ref().and_then(|d| d.as_ref().ok());
        let tint = match p.origin {
            Origin::GooglePlay => ds::ACCENT,
            Origin::Sideloaded => ds::AMBER,
            Origin::System | Origin::Oem => ds::TEXT_SECONDARY,
        };
        let letter = name.chars().find(|c| c.is_alphanumeric()).map(|c| c.to_uppercase().to_string()).unwrap_or_default();
        let badge: Element<'a, Msg> = match details.and_then(|d| d.pid) {
            Some(pid) => ds::tinted_badge(format!("PID {pid}"), ds::GREEN),
            None => ds::tinted_badge(status(p).0, status(p).1),
        };
        let sdk_line = match details {
            Some(d) => format!(
                "Target SDK {} · Min SDK {}",
                d.target_sdk.map_or("?".into(), |v| v.to_string()),
                d.min_sdk.map_or("?".into(), |v| v.to_string())
            ),
            None => match &inspector.details {
                Some(Err(e)) => format!("Couldn't read details: {e}"),
                _ => "Reading details…".into(),
            },
        };
        let identity = row![
            container(ds::mono(letter, ds::TEXT_MD, tint).font(ds::MONO_SEMIBOLD)).width(36).height(36).center(36).style(style::tint(tint)),
            column![
                row![ds::semibold(name, ds::TEXT_BASE, ds::TEXT).width(Length::Fill), badge].spacing(6).align_y(Alignment::Center),
                ds::mono(p.id.clone(), ds::TEXT_SM, ds::TEXT_SECONDARY),
                ds::label(sdk_line, ds::TEXT_XS, ds::TEXT_TERTIARY),
            ]
            .spacing(2)
            .width(Length::Fill),
        ]
        .spacing(10);
        let identity: Element<'a, Msg> = match p.verdict() {
            Some(v) => column![
                identity,
                row![rating_tag(p), ds::label(v.reason, ds::TEXT_SM, ds::TEXT_SECONDARY)].spacing(8).align_y(Alignment::Center),
            ]
            .spacing(8)
            .into(),
            None => identity.into(),
        };

        let fact = |title: &'a str, value: String, mono: bool| -> Element<'a, Msg> {
            column![
                ds::label(title, ds::TEXT_2XS, ds::TEXT_TERTIARY),
                if mono { ds::mono(value, 10.5, ds::TEXT) } else { ds::label(value, 10.5, ds::TEXT_SECONDARY) },
            ]
            .spacing(1)
            .width(Length::Fill)
            .into()
        };
        let facts: Element<'a, Msg> = match details {
            Some(d) => {
                let splits = d.apks.len().saturating_sub(1);
                let size = match splits {
                    0 => fmt_bytes(d.apk_bytes),
                    n => format!("{} ({n} split{})", fmt_bytes(d.apk_bytes), if n == 1 { "" } else { "s" }),
                };
                let base = d.apks.first().cloned().unwrap_or_else(|| p.path.clone());
                let granted = d.runtime.iter().filter(|(_, g)| *g).count();
                column![
                    row![fact("User ID (UID)", d.uid_label(), true), fact("APK Size / Split", size, true)].spacing(8),
                    column![ds::label("Base APK Path", ds::TEXT_2XS, ds::TEXT_TERTIARY), ds::mono(base, ds::TEXT_XS, ds::TEXT_SECONDARY)].spacing(1),
                    row![
                        fact("Permissions", format!("{} declared ({} runtime, {granted} granted)", d.requested.len(), d.runtime.len()), true),
                        fact("Installer Source", d.installer.clone(), false),
                    ]
                    .spacing(8),
                ]
                .spacing(6)
                .into()
            }
            None => ds::label(if inspector.details.is_none() { "Reading…" } else { "—" }, ds::TEXT_SM, ds::TEXT_TERTIARY).into(),
        };

        let item = |glyph: ds::Icon, caption: &'a str, hint: Option<&'a str>, danger: bool, msg: Option<Msg>| -> Element<'a, Msg> {
            let mut content = row![
                ds::icon_any(glyph, 14.0, if danger { ds::RED } else { ds::TEXT_SECONDARY }),
                ds::label(caption, ds::TEXT_BASE, if danger { ds::RED_SOFT } else { ds::TEXT }),
            ]
            .spacing(8)
            .align_y(Alignment::Center);
            if let Some(hint) = hint {
                content = content.push(space::horizontal()).push(ds::mono(hint, ds::TEXT_XS, ds::TEXT_TERTIARY));
            }
            button(container(content).center_y(Length::Fill))
                .width(Length::Fill)
                .height(28)
                .padding([0, 8])
                .style(style::menu_item(danger))
                .on_press_maybe(msg.filter(|_| !self.busy))
                .into()
        };
        let id = p.id.clone();
        let lower: Element<'a, Msg> = if inspector.permissions {
            let mut list = column![].spacing(2);
            match details {
                Some(d) if !d.runtime.is_empty() || !d.requested.is_empty() => {
                    for (perm, granted) in &d.runtime {
                        list = list.push(
                            row![
                                ds::icon_any(if *granted { ds::Icon::Check } else { ds::Icon::X }, 12.0, if *granted { ds::GREEN } else { ds::TEXT_TERTIARY }),
                                ds::mono(perm.trim_start_matches("android.permission.").to_owned(), ds::TEXT_XS, ds::TEXT),
                            ]
                            .spacing(6)
                            .align_y(Alignment::Center),
                        );
                    }
                    let other = d.requested.iter().filter(|r| !d.runtime.iter().any(|(n, _)| n == *r));
                    for perm in other {
                        list = list.push(ds::mono(perm.trim_start_matches("android.permission.").to_owned(), ds::TEXT_XS, ds::TEXT_SECONDARY));
                    }
                }
                _ => list = list.push(ds::label("No permissions requested.", ds::TEXT_SM, ds::TEXT_TERTIARY)),
            }
            column![
                row![
                    ds::medium("Runtime permissions (✓ granted), then the rest", ds::TEXT_XS, ds::TEXT_SECONDARY),
                    space::horizontal(),
                    button(ds::label("Back", ds::TEXT_SM, ds::ACCENT)).padding(0).style(style::link(ds::ACCENT_HOVER)).on_press(Msg::Permissions),
                ]
                .align_y(Alignment::Center),
                scrollable(list).height(Length::Fixed(180.0)).direction(ds::thin_scrollbar()).style(style::scrollbar),
            ]
            .spacing(6)
            .into()
        } else {
            let installed = p.installed;
            let (toggle_icon, toggle_caption, toggle_action) = if !installed {
                (ds::Icon::RotateCcw, "Restore Package (install-existing)", Action::Restore)
            } else if p.disabled {
                (ds::Icon::CirclePlay, "Enable Package (pm enable)", Action::Enable)
            } else {
                (ds::Icon::Ban, "Disable Package (pm disable-user)", Action::Disable)
            };
            column![
                item(ds::Icon::Play, "Launch Activity (am start)", None, false, (installed && !p.disabled).then(|| Msg::Launch(id.clone()))),
                item(ds::Icon::Download, "Extract & Pull APK", Some("adb pull"), false, installed.then(|| Msg::SaveApkFor(vec![id.clone()]))),
                item(ds::Icon::Archive, "Clear Cache", Some("logins kept"), false, installed.then(|| Msg::ActOn(Action::ClearCache, id.clone()))),
                item(ds::Icon::Eraser, "Clear Data (resets the app)", None, true, installed.then(|| Msg::ActOn(Action::ClearData, id.clone()))),
                item(ds::Icon::ShieldCheck, "Review Android Permissions…", None, false, details.map(|_| Msg::Permissions)),
                container(ds::hline(ds::BORDER)).padding([4, 0]),
                item(ds::Icon::Square, "Force Stop Process", Some("am force-stop"), true, installed.then(|| Msg::ActOn(Action::ForceStop, id.clone()))),
                item(toggle_icon, toggle_caption, None, !matches!(toggle_action, Action::Enable | Action::Restore), Some(Msg::ActOn(toggle_action, id.clone()))),
            ]
            .spacing(2)
            .into()
        };
        let _ = sdk;
        container(
            column![
                identity,
                container(ds::hline(ds::BORDER_SUBTLE)).padding(Padding::default().top(10)),
                container(facts).padding([8, 0]),
                ds::hline(ds::BORDER_SUBTLE),
                container(lower).padding(Padding::default().top(6)),
            ],
        )
        .width(320)
        .padding(10)
        .style(style::flyout)
        .into()
    }

    /// Selection status, accelerators and the batch actions.
    fn command_bar<'a>(&'a self, shown: &[&'a Package], sdk: u32, fit: Fit, batch: Option<&str>) -> Element<'a, Msg> {
        let n = self.selected.len();
        let total = self.packages.iter().filter(|p| p.installed).count();
        let can = n > 0 && !self.busy && !self.loading;
        let link = |caption: &'a str, hint: Option<&'a str>, msg: Option<Msg>| {
            let mut content = row![ds::label(caption, ds::TEXT_BASE, ds::TEXT_SECONDARY)].spacing(4).align_y(Alignment::Center);
            if let Some(h) = hint {
                content = content.push(ds::kbd(h, ds::SUBTLE, ds::BORDER, ds::TEXT_TERTIARY));
            }
            button(content).padding(0).style(style::link(ds::TEXT)).on_press_maybe(msg)
        };
        let status = row![
            row![
                ds::dot(if n > 0 { ds::ACCENT } else { ds::TEXT_TERTIARY }, 8.0),
                ds::medium(
                    if self.busy {
                        crate::i18n::tr("Working…").to_owned()
                    } else if n == 1 {
                        crate::i18n::tr("1 package selected").to_owned()
                    } else {
                        crate::i18n::trf("{} packages selected", &[&n])
                    },
                    ds::TEXT_BASE,
                    ds::TEXT,
                ),
                ds::mono(if fit == Fit::Full { format!("(of {total})") } else { String::new() }, ds::TEXT_SM, ds::TEXT_TERTIARY),
            ]
            .spacing(6)
            .align_y(Alignment::Center),
            ds::vline(16.0),
            link("Select all", (fit != Fit::Icons).then_some("Ctrl+A"), (!shown.is_empty()).then_some(Msg::SelectShown)),
            link("Clear", None, (n > 0).then_some(Msg::ClearSelection)),
        ]
        .spacing(12)
        .align_y(Alignment::Center);
        // Batch TVs get the same actions; say so where they're pressed.
        let status: Element<'a, Msg> = match batch {
            Some(note) => row![
                status,
                tooltip(ds::tinted_badge("+ batch", ds::ACCENT), tip_owned(note.to_owned()), tooltip::Position::Top),
            ]
            .spacing(10)
            .align_y(Alignment::Center)
            .into(),
            None => status.into(),
        };

        let all_disabled = n > 0 && self.packages.iter().filter(|p| self.selected.contains(&p.id)).all(|p| p.disabled);
        let (toggle_caption, toggle_icon, toggle_action) =
            if all_disabled { ("Enable", ds::Icon::CirclePlay, Action::Enable) } else { ("Disable", ds::Icon::CirclePause, Action::Disable) };
        // One command: icon + caption (+ key hint), or just the icon with the
        // caption as a tooltip when space is short.
        let command = |glyph: ds::Icon, color: Color, caption: &'a str, key: Option<Element<'a, Msg>>, style_fn: ui::ButtonStyle, msg: Option<Msg>, help: &'a str| -> Element<'a, Msg> {
            if fit == Fit::Icons {
                let b = button(container(ds::icon_any(glyph, 14.0, color)).center(Length::Fill)).width(32).height(32).padding(0).style(style_fn).on_press_maybe(msg);
                tooltip(b, tip(help), tooltip::Position::Top).into()
            } else {
                let b = ds::icon_button(Some((glyph, color)), caption, if fit == Fit::Full { key } else { None }, style_fn, 32.0, msg);
                if help == caption { b.into() } else { tooltip(b, tip(help), tooltip::Position::Top).into() }
            }
        };
        let standard = |glyph: ds::Icon, caption: &'a str, msg: Option<Msg>| command(glyph, ds::TEXT_SECONDARY, caption, None, style::standard, msg.filter(|_| can), caption);
        let _ = sdk;
        let cache = standard(ds::Icon::Archive, "Clear Cache", Some(Msg::Act(Action::ClearCache)));
        let actions = row![
            command(
                ds::Icon::Trash,
                ds::RED,
                "Uninstall",
                Some(ds::kbd("Del", alpha(color_red_950(), 0.6), alpha(ds::RED, 0.3), ds::RED_SOFT)),
                style::danger,
                can.then_some(Msg::Act(Action::Uninstall)),
                "Uninstall",
            ),
            standard(toggle_icon, toggle_caption, Some(Msg::Act(toggle_action))),
            standard(ds::Icon::RotateCcw, "Reinstall", Some(Msg::Act(Action::Restore))),
            cache,
            standard(ds::Icon::HardDrive, "Clear Data", Some(Msg::Act(Action::ClearData))),
            standard(ds::Icon::FolderSync, "Batch Backup", Some(Msg::Backup)),
            command(
                ds::Icon::Download,
                ds::ACCENT_TEXT,
                "Extract APK",
                Some(ds::kbd("Ctrl+S", ds::ACCENT_KBD, alpha(ds::ACCENT, 0.4), ds::ACCENT)),
                style::primary,
                can.then_some(Msg::SaveApk),
                "Extract APK",
            ),
        ]
        .spacing(4)
        .align_y(Alignment::Center);
        container(column![
            ds::hline(ds::BORDER),
            container(row![status, space::horizontal(), actions].spacing(8).align_y(Alignment::Center)).padding([0, 12]).center_y(Length::Fill),
        ])
        .height(ds::COMMAND_BAR_HEIGHT)
        .style(style::fill(ds::SIDEBAR))
        .into()
    }
}

/// Which grid columns fit the available width.
#[derive(Debug, Clone, Copy)]
struct Columns {
    origin: bool,
    version: bool,
    /// Disable / Clear Cache next to "⋯" (else "⋯" alone).
    row_buttons: bool,
}

impl Columns {
    fn fitting(width: f32) -> Columns {
        Columns { origin: width >= 1000.0, version: width >= 820.0, row_buttons: width >= 1000.0 }
    }

    fn actions_width(self) -> f32 {
        if self.row_buttons { COL_ACTIONS } else { 56.0 }
    }
}

/// How much of the command bar fits: everything, no key hints, or icons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fit {
    Full,
    NoHints,
    Icons,
}

impl Fit {
    fn for_width(width: f32) -> Fit {
        // Spanish captions run about a fifth longer.
        let scale = if crate::i18n::current() == crate::i18n::Lang::Es { 1.2 } else { 1.0 };
        if width >= 1180.0 * scale {
            Fit::Full
        } else if width >= 1000.0 * scale {
            Fit::NoHints
        } else {
            Fit::Icons
        }
    }
}

/// Heights the virtual grid counts on (1px line + the row itself).
const GROUP_ROW_HEIGHT: f32 = 35.0;
const PACKAGE_ROW_HEIGHT: f32 = 37.0;
/// Rows built beyond the visible area, so fast scrolling doesn't flash.
const VIRTUAL_MARGIN: f32 = 600.0;

const COL_CHECK: f32 = 36.0;
const COL_NAME: Length = Length::FillPortion(5);
const COL_PACKAGE: Length = Length::FillPortion(6);
const COL_ORIGIN: f32 = 110.0;
const COL_VERSION: f32 = 110.0;
const COL_SIZE: f32 = 84.0;
const COL_STATUS: f32 = 84.0;
const COL_ACTIONS: f32 = 210.0;

const fn color_red_950() -> Color {
    Color { r: 69.0 / 255.0, g: 10.0 / 255.0, b: 10.0 / 255.0, a: 1.0 }
}

/// A small tag with the app's bloatware rating; its reason as a tooltip.
fn rating_tag<'a>(p: &Package) -> Element<'a, Msg> {
    let Some(v) = p.verdict() else { return Space::new().into() };
    let color = match v.rating {
        crate::android::bloat::Rating::Bloat => ds::RED,
        crate::android::bloat::Rating::Optional => ds::AMBER,
        crate::android::bloat::Rating::Keep => ds::GREEN,
    };
    tooltip(ds::tinted_badge(v.rating.label(), color), tip(v.reason), tooltip::Position::Right).into()
}

/// Saving APKs changes nothing on the TV: just log lines.
async fn saving(conn: crate::android::Connection, pkgs: Vec<String>, dir: PathBuf, p: crate::jobs::Progress) -> Outcome {
    Outcome::lines(packages::save_apks(conn, pkgs, dir, p).await)
}

fn inspect(pkg: String, conn: crate::android::Connection) -> Task<Msg> {
    Task::perform(packages::details(conn, pkg.clone()), move |r| Msg::Inspected(pkg.clone(), r))
}

fn tip_owned<'a>(text: String) -> Element<'a, Msg> {
    container(ds::label(text, ds::TEXT_SM, ds::TEXT)).padding(8).max_width(260).style(style::flyout).into()
}

fn tip<'a>(text: &'a str) -> Element<'a, Msg> {
    container(ds::label(text, ds::TEXT_SM, ds::TEXT)).padding(8).max_width(260).style(style::flyout).into()
}
