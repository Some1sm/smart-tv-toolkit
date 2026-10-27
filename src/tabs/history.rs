//! History: every change the app made to this TV, each with Undo, and
//! snapshots to roll the TV's apps and settings back to.

use iced::widget::{button, checkbox, column, container, row, space, tooltip};
use iced::{Alignment, Element, Length, Task};

use super::{Ctx, View};
use crate::android::changes::{self, Outcome};
use crate::design::{self as ds, style};
use crate::jobs::{JobId, Unit};
use crate::journal::{self, Change};
use crate::ui;

/// Rows shown at once; older ones stay in the file.
const SHOWN: usize = 300;

#[derive(Debug)]
struct Preview {
    snapshot: u64,
    /// The changes needed, each with whether it's ticked.
    plan: Option<Vec<(Change, bool)>>,
}

#[derive(Debug, Default)]
pub struct State {
    busy: bool,
    capturing: bool,
    name: String,
    preview: Option<Preview>,
}

#[derive(Debug, Clone)]
pub enum Msg {
    Undo(u64),
    Undone(u64, JobId, Result<Result<Change, String>, String>),
    Name(String),
    TakeSnapshot,
    Captured(JobId, String, bool, Result<Result<journal::State, String>, String>),
    Preview(u64),
    Planned(u64, JobId, Result<Result<journal::State, String>, String>),
    Tick(usize, bool),
    ClosePreview,
    Restore,
    Restored(JobId, Result<(Vec<Change>, Vec<String>), String>),
    DeleteSnapshot(u64),
    /// Give the batch TVs this snapshot's setup.
    CopyTo(u64),
    CopyConfirmed(u64, bool),
    Copied(JobId, super::Target, Result<(Vec<Change>, Vec<String>), String>),
}

fn icon_for(change: &Change) -> (ds::Icon, iced::Color) {
    match change {
        Change::Setting { .. } => (ds::Icon::SlidersHorizontal, ds::ACCENT),
        Change::Package { .. } => (ds::Icon::Package, ds::PURPLE),
        Change::Density { .. } => (ds::Icon::Monitor, ds::ACCENT),
        Change::Bucket { .. } | Change::AppOp { .. } => (ds::Icon::Moon, ds::PURPLE),
        Change::DeviceConfig { .. } => (ds::Icon::MemoryStick, ds::ACCENT),
        Change::Component { .. } => (ds::Icon::Power, ds::AMBER),
        Change::Home { .. } => (ds::Icon::House, ds::ACCENT),
        Change::Note { .. } => (ds::Icon::Info, ds::TEXT_TERTIARY),
    }
}

/// `2026-10-26 23:21:05` → `Oct 26 23:21`.
pub(super) fn short_time(time: &str) -> String {
    chrono::NaiveDateTime::parse_from_str(time, "%Y-%m-%d %H:%M:%S")
        .map(|t| t.format("%b %d %H:%M").to_string())
        .unwrap_or_else(|_| time.to_owned())
}

impl State {
    pub fn reset(&mut self) {
        *self = State::default();
    }

    /// Take the day's automatic snapshot, so there's always a recent state
    /// to go back to before anything gets changed.
    pub fn on_connect(&mut self, ctx: &mut Ctx) -> Task<Msg> {
        let Some(session) = ctx.session else { return Task::none() };
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        if ctx.journal.has_snapshot_on(session.address(), &today) {
            return Task::none();
        }
        self.capture("Automatic (on connect)".into(), true, ctx)
    }

    fn capture(&mut self, name: String, auto: bool, ctx: &mut Ctx) -> Task<Msg> {
        self.capturing = true;
        ctx.job("Taking a snapshot", Unit::Items, |conn, _| changes::capture(conn), move |id, r| Msg::Captured(id, name.clone(), auto, r))
    }

    pub fn update(&mut self, msg: Msg, ctx: &mut Ctx) -> Task<Msg> {
        match msg {
            Msg::Undo(id) => {
                let Some(change) = ctx.journal.entry(id).map(|e| e.change.clone()) else { return Task::none() };
                if self.busy {
                    return Task::none();
                }
                self.busy = true;
                let title = format!("Undo: {}", change.describe());
                return ctx.job(title, Unit::Items, move |conn, _| changes::revert(conn, change), move |job, r| Msg::Undone(id, job, r));
            }
            Msg::Undone(id, job, result) => {
                ctx.jobs.finish(job);
                self.busy = false;
                match result.and_then(|r| r) {
                    Ok(change) => {
                        ctx.log(format!("Undone: {}", change.describe()));
                        ctx.journal.mark_reverted(id);
                        if let Err(e) = ctx.journal.save() {
                            ctx.log(format!("Could not save the change history: {e}"));
                        }
                    }
                    Err(e) => ctx.log(format!("Couldn't undo: {e}")),
                }
            }
            Msg::Name(name) => self.name = name,
            Msg::TakeSnapshot => {
                if self.capturing {
                    return Task::none();
                }
                let name = match self.name.trim() {
                    "" => "Manual snapshot".to_owned(),
                    n => n.to_owned(),
                };
                self.name.clear();
                return self.capture(name, false, ctx);
            }
            Msg::Captured(id, name, auto, result) => {
                ctx.jobs.finish(id);
                self.capturing = false;
                let Some(session) = ctx.session else { return Task::none() };
                match result.and_then(|r| r) {
                    Ok(state) => {
                        let settings: usize = state.settings.values().map(|m| m.len()).sum();
                        ctx.log(format!("Snapshot \"{name}\": {} packages, {settings} settings.", state.packages.len()));
                        ctx.journal.add_snapshot(session.address(), &session.label(), name, auto, state);
                        if let Err(e) = ctx.journal.save() {
                            ctx.log(format!("Could not save the snapshot: {e}"));
                        }
                    }
                    Err(e) => ctx.log(format!("Couldn't take a snapshot: {e}")),
                }
            }
            Msg::Preview(snapshot) => {
                self.preview = Some(Preview { snapshot, plan: None });
                return ctx.job("Comparing with the snapshot", Unit::Items, |conn, _| changes::capture(conn), move |id, r| {
                    Msg::Planned(snapshot, id, r)
                });
            }
            Msg::Planned(snapshot, id, result) => {
                ctx.jobs.finish(id);
                let (Some(preview), Some(session)) = (self.preview.as_mut(), ctx.session) else { return Task::none() };
                if preview.snapshot != snapshot {
                    return Task::none();
                }
                let Some(target) = ctx.journal.snapshot(snapshot) else { return Task::none() };
                match result.and_then(|r| r) {
                    Ok(current) => {
                        // Settings change on their own all the time; only the
                        // ones this app changed since are ticked by default.
                        let touched = ctx.journal.touched_settings_since(session.address(), &target.time);
                        let plan = journal::restore_plan(&target.state, &current)
                            .into_iter()
                            .map(|c| {
                                let tick = match &c {
                                    Change::Setting { namespace, key, .. } => touched.contains(&format!("{namespace}/{key}")),
                                    _ => true,
                                };
                                (c, tick)
                            })
                            .collect();
                        preview.plan = Some(plan);
                    }
                    Err(e) => {
                        ctx.log(format!("Couldn't read the TV's state: {e}"));
                        self.preview = None;
                    }
                }
            }
            Msg::Tick(i, on) => {
                if let Some((_, tick)) = self.preview.as_mut().and_then(|p| p.plan.as_mut()).and_then(|p| p.get_mut(i)) {
                    *tick = on;
                }
            }
            Msg::ClosePreview => self.preview = None,
            Msg::Restore => {
                let Some(plan) = self.preview.take().and_then(|p| p.plan) else { return Task::none() };
                let picked: Vec<Change> = plan.into_iter().filter(|(_, t)| *t).map(|(c, _)| c).collect();
                if picked.is_empty() || self.busy {
                    return Task::none();
                }
                self.busy = true;
                let title = format!("Restoring {} change(s)", picked.len());
                return ctx.job(title, Unit::Items, move |conn, p| changes::restore(conn, picked, p), Msg::Restored);
            }
            Msg::Restored(id, result) => {
                ctx.jobs.finish(id);
                self.busy = false;
                match result {
                    Ok((done, errors)) => {
                        let mut lines = vec![format!("Restored {} change(s).", done.len())];
                        lines.extend(errors.into_iter().map(|e| format!("Couldn't restore {e}")));
                        ctx.apply(Outcome { lines, changes: done });
                    }
                    Err(e) => ctx.log(format!("Restore failed: {e}")),
                }
            }
            Msg::CopyTo(id) => {
                let Some(s) = ctx.journal.snapshot(id) else { return Task::none() };
                if ctx.batch.is_empty() {
                    ctx.log("Tick the TVs to copy to in the sidebar (Include in batch actions) first.");
                    return Task::none();
                }
                let off = s.state.packages.values().filter(|p| **p != journal::PkgState::Enabled).count();
                let question = format!(
                    "Copy \"{}\" to: {}?\n\nApps disabled or uninstalled in it ({off}) get the same state on those TVs, and the \
                     settings this app changed here are copied. Each TV is read first; only what differs changes, and it all goes \
                     into that TV's History.",
                    s.name,
                    ctx.batch_names()
                );
                return ui::confirm("Copy Setup", question, move |yes| Msg::CopyConfirmed(id, yes));
            }
            Msg::CopyConfirmed(_, false) => {}
            Msg::CopyConfirmed(id, true) => {
                let (Some(s), Some(session)) = (ctx.journal.snapshot(id), ctx.session) else { return Task::none() };
                let source = s.state.clone();
                // Settings this app changed on this TV (and not undone).
                let mut keys: Vec<String> = ctx
                    .journal
                    .entries_for(session.address())
                    .filter(|e| e.reverted.is_none())
                    .filter_map(|e| match &e.change {
                        Change::Setting { namespace, key, .. } => Some(format!("{namespace}/{key}")),
                        _ => None,
                    })
                    .collect();
                keys.sort();
                keys.dedup();
                ctx.log(format!("Copying \"{}\" to {}…", s.name, ctx.batch_names()));
                return ctx.job_on_batch(
                    "Copying setup",
                    Unit::Items,
                    move |conn, p| {
                        let (source, keys) = (source.clone(), keys.clone());
                        async move {
                            match changes::capture(conn.clone()).await {
                                Ok(current) => changes::restore(conn, journal::copy_plan(&source, &keys, &current), p).await,
                                Err(e) => (Vec::new(), vec![format!("couldn't read the TV: {e}")]),
                            }
                        }
                    },
                    Msg::Copied,
                );
            }
            Msg::Copied(id, target, result) => {
                ctx.jobs.finish(id);
                match result {
                    Ok((done, errors)) => {
                        let mut lines = vec![format!("Copied {} change(s).", done.len())];
                        lines.extend(errors.into_iter().map(|e| format!("Couldn't copy {e}")));
                        ctx.apply_on(&target, Outcome { lines, changes: done });
                    }
                    Err(e) => ctx.log(format!("[{}] Copy failed: {e}", target.label)),
                }
            }
            Msg::DeleteSnapshot(id) => {
                ctx.journal.remove_snapshot(id);
                if self.preview.as_ref().is_some_and(|p| p.snapshot == id) {
                    self.preview = None;
                }
                if let Err(e) = ctx.journal.save() {
                    ctx.log(format!("Could not save the change history: {e}"));
                }
            }
        }
        Task::none()
    }

    pub fn view<'a>(&'a self, v: &View<'a>) -> Element<'a, Msg> {
        let Some(session) = v.session else { return ui::empty_state("No TV connected", "") };
        let journal = v.journal;
        let header = column![
            ds::semibold("Change History", ds::TEXT_LG, ds::TEXT),
            ds::label(
                "Everything this app changed on this TV, newest first. Undo puts back the value that was there before.",
                ds::TEXT_BASE,
                ds::TEXT_SECONDARY,
            ),
        ]
        .spacing(2);
        let mut page = column![header, self.snapshots(session.address(), journal, v.batch.len())].spacing(12);
        if let Some(preview) = &self.preview {
            page = page.push(self.preview_card(preview, journal));
        }
        page = page.push(self.changes(session.address(), journal));
        ui::page(page)
    }

    fn snapshots<'a>(&'a self, serial: &str, journal: &'a journal::Journal, batch: usize) -> Element<'a, Msg> {
        let take = row![
            ds::input("Snapshot name (optional)", &self.name, false, ds::INSET).on_input(Msg::Name).on_submit(Msg::TakeSnapshot).width(260),
            ds::icon_button(
                Some((ds::Icon::Camera, ds::TEXT_SECONDARY)),
                if self.capturing { "Taking…" } else { "Take Snapshot" },
                None,
                style::standard,
                ds::CONTROL_HEIGHT,
                (!self.capturing).then_some(Msg::TakeSnapshot),
            ),
        ]
        .spacing(8)
        .align_y(Alignment::Center);
        let mut list = column![].spacing(2);
        let mut any = false;
        for s in journal.snapshots_for(serial) {
            any = true;
            let settings: usize = s.state.settings.values().map(|m| m.len()).sum();
            let previewing = self.preview.as_ref().is_some_and(|p| p.snapshot == s.id);
            list = list.push(
                container(
                    row![
                        ds::icon_any(if s.auto { ds::Icon::Clock } else { ds::Icon::Camera }, 14.0, ds::TEXT_SECONDARY),
                        column![
                            ds::medium(s.name.clone(), ds::TEXT_BASE, ds::TEXT),
                            ds::mono(
                                format!("{} · {} packages · {settings} settings", short_time(&s.time), s.state.packages.len()),
                                ds::TEXT_XS,
                                ds::TEXT_TERTIARY,
                            ),
                        ]
                        .spacing(2)
                        .width(Length::Fill),
                        ds::icon_button(
                            Some((ds::Icon::ArchiveRestore, ds::TEXT_SECONDARY)),
                            if previewing { "Comparing…" } else { "Restore…" },
                            None,
                            style::standard,
                            ds::CONTROL_HEIGHT,
                            (!previewing && !self.busy).then_some(Msg::Preview(s.id)),
                        ),
                        tooltip(
                            ds::icon_button(
                                Some((ds::Icon::Copy, ds::TEXT_SECONDARY)),
                                "Copy to batch TVs",
                                None,
                                style::standard,
                                ds::CONTROL_HEIGHT,
                                (batch > 0 && !self.busy).then_some(Msg::CopyTo(s.id)),
                            ),
                            container(ds::label(
                                "Gives the TVs ticked in the sidebar the same disabled apps and changed settings.",
                                ds::TEXT_SM,
                                ds::TEXT,
                            ))
                            .padding([4, 8])
                            .max_width(260)
                            .style(style::flyout),
                            tooltip::Position::Top,
                        ),
                        ds::square_button(ds::Icon::Trash, ds::TEXT_TERTIARY, ds::CONTROL_HEIGHT, style::ghost, Some(Msg::DeleteSnapshot(s.id))),
                    ]
                    .spacing(10)
                    .align_y(Alignment::Center),
                )
                .padding([8, 10])
                .style(style::well),
            );
        }
        if !any {
            list = list.push(ds::label("No snapshots yet. One is taken automatically each day the TV connects.", ds::TEXT_BASE, ds::TEXT_TERTIARY));
        }
        ui::card("Snapshots", column![take, list].spacing(10).into())
    }

    fn preview_card<'a>(&'a self, preview: &'a Preview, journal: &'a journal::Journal) -> Element<'a, Msg> {
        let name = journal.snapshot(preview.snapshot).map(|s| format!("{} ({})", s.name, short_time(&s.time))).unwrap_or_default();
        let body: Element<'a, Msg> = match &preview.plan {
            None => ds::label("Reading the TV's current state…", ds::TEXT_BASE, ds::TEXT_SECONDARY).into(),
            Some(plan) if plan.is_empty() => ds::label("The TV already matches this snapshot.", ds::TEXT_BASE, ds::GREEN).into(),
            Some(plan) => {
                let picked = plan.iter().filter(|(_, t)| *t).count();
                let mut rows = column![].spacing(4);
                for (i, (change, tick)) in plan.iter().enumerate() {
                    let (glyph, color) = icon_for(change);
                    rows = rows.push(
                        row![
                            checkbox(*tick).on_toggle(move |on| Msg::Tick(i, on)).size(14).style(style::checkbox),
                            ds::icon_any(glyph, 13.0, color),
                            ds::label(change.describe(), ds::TEXT_BASE, ds::TEXT),
                        ]
                        .spacing(8)
                        .align_y(Alignment::Center),
                    );
                }
                column![
                    ds::label(
                        "Ticked items are put back. Settings the TV changed by itself are left unticked; tick any you want back too.",
                        ds::TEXT_SM,
                        ds::TEXT_SECONDARY,
                    ),
                    rows,
                    row![
                        ds::icon_button(
                            Some((ds::Icon::ArchiveRestore, ds::ACCENT_TEXT)),
                            "Restore ticked",
                            None,
                            style::primary,
                            32.0,
                            (picked > 0 && !self.busy).then_some(Msg::Restore),
                        ),
                        ds::label(format!("{picked} of {} selected", plan.len()), ds::TEXT_SM, ds::TEXT_TERTIARY),
                    ]
                    .spacing(10)
                    .align_y(Alignment::Center),
                ]
                .spacing(10)
                .into()
            }
        };
        container(
            column![
                row![
                    ds::overline("Restore preview"),
                    ds::label(name, ds::TEXT_BASE, ds::TEXT_SECONDARY),
                    space::horizontal(),
                    ds::square_button(ds::Icon::X, ds::TEXT_SECONDARY, 24.0, style::ghost, Some(Msg::ClosePreview)),
                ]
                .spacing(10)
                .align_y(Alignment::Center),
                body,
            ]
            .spacing(12),
        )
        .padding(16)
        .width(Length::Fill)
        .style(style::preview)
        .into()
    }

    fn changes<'a>(&'a self, serial: &str, journal: &'a journal::Journal) -> Element<'a, Msg> {
        let total = journal.entries_for(serial).count();
        let mut list = column![].spacing(0);
        for (i, e) in journal.entries_for(serial).take(SHOWN).enumerate() {
            let (glyph, color) = icon_for(&e.change);
            let status: Element<'a, Msg> = match (&e.reverted, e.change.inverse()) {
                (Some(when), _) => ds::label(format!("Undone {}", short_time(when)), ds::TEXT_SM, ds::TEXT_TERTIARY).into(),
                (None, Some(_)) => button(container(crate::ui::text("Undo").size(ds::TEXT_SM)).center_y(Length::Fill))
                    .height(24)
                    .padding([0, 10])
                    .style(style::outline)
                    .on_press_maybe((!self.busy).then_some(Msg::Undo(e.id)))
                    .into(),
                (None, None) => ds::label("Can't be undone", ds::TEXT_SM, ds::TEXT_TERTIARY).into(),
            };
            if i > 0 {
                list = list.push(ds::hline(ds::DIVIDER));
            }
            let text_color = if e.reverted.is_some() { ds::TEXT_TERTIARY } else { ds::TEXT };
            list = list.push(
                row![
                    ds::mono(short_time(&e.time), ds::TEXT_SM, ds::TEXT_TERTIARY).width(96),
                    ds::icon_any(glyph, 14.0, color),
                    ds::label(e.change.describe(), ds::TEXT_BASE, text_color).width(Length::Fill),
                    status,
                ]
                .spacing(10)
                .padding([8, 4])
                .align_y(Alignment::Center),
            );
        }
        if total == 0 {
            list = list.push(ds::label("Nothing changed yet. Uninstalls, disables, settings and fixes will show up here.", ds::TEXT_BASE, ds::TEXT_TERTIARY));
        } else if total > SHOWN {
            list = list.push(ds::label(format!("Showing the latest {SHOWN} of {total}."), ds::TEXT_SM, ds::TEXT_TERTIARY));
        }
        ui::card("Changes", list.into())
    }
}
