//! Optimize: a one-click list of recommended changes, free storage, rein in
//! background and start-at-boot apps, pick a lighter home app, cap cached
//! apps, block ads by DNS, and measure the TV and time app launches before
//! and after. Reading is always safe; every change asks or is recorded in
//! the History so it can be undone.

use std::collections::{BTreeMap, BTreeSet};

use iced::widget::{button, checkbox, column, container, progress_bar, row, space, tooltip};
use iced::{Alignment, Color, Element, Length, Task};

use super::{Ctx, View};
use crate::android::actions;
use crate::android::changes::{self, Outcome};
use crate::android::health::{self, Fix};
use crate::android::optimize::{self, BackgroundApp, Storage};
use crate::android::sysinfo::fmt_bytes;
use crate::android::tuneup::{self, Step, TvState};
use crate::design::{self as ds, style};
use crate::i18n::{tr, trf};
use crate::jobs::{JobId, Unit};
use crate::journal::Change;
use crate::speedtests::{self, Checkup, Checkups, Runs};
use crate::ui::{self, text};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Section {
    #[default]
    Recommended,
    Storage,
    Background,
    Startup,
    HomeMemory,
    SpeedTest,
}

impl Section {
    const ALL: [Section; 6] =
        [Section::Recommended, Section::Storage, Section::Background, Section::Startup, Section::HomeMemory, Section::SpeedTest];

    fn label(self) -> &'static str {
        match self {
            Section::Recommended => "Recommended",
            Section::Storage => "Storage",
            Section::Background => "Background Apps",
            Section::Startup => "Startup Apps",
            Section::HomeMemory => "Home & Memory",
            Section::SpeedTest => "Speed Test",
        }
    }
}

/// Ad-blocking Private DNS providers: (name, hostname).
const AD_BLOCK_DNS: [(&str, &str); 2] = [("AdGuard DNS", "dns.adguard-dns.com"), ("Mullvad (ad-block)", "adblock.dns.mullvad.net")];

/// Cached background apps to keep: (label, value; `None` = Android's default).
const CACHED_LIMITS: [(&str, Option<&str>); 4] = [("Default", None), ("32", Some("32")), ("24", Some("24")), ("16", Some("16"))];

/// The name of the snapshot (and measurement) taken before the Recommended changes.
const BEFORE_RECOMMENDED: &str = "Before Recommended changes";

/// Settings screens the TV Settings card opens (`am start -a`).
const ACCOUNTS_SCREEN: &str = "android.settings.SYNC_SETTINGS";
const ASSISTANT_SCREEN: &str = "android.search.action.SEARCH_SETTINGS";
const PLAY_STORE_SCREEN: &str = "com.google.android.finsky.tv.SETTINGS";
const SCREEN_SAVER_SCREEN: &str = "android.settings.DREAM_SETTINGS";

/// Days of uptime after which a restart is suggested.
const RESTART_AFTER_DAYS: u64 = 7;

#[derive(Debug, Default)]
pub struct State {
    section: Section,
    tv: Option<Result<TvState, String>>,
    /// The Recommended list for `tv`, and the ticked steps.
    steps: Vec<Step>,
    picks: BTreeSet<Step>,
    storage: Option<Result<Storage, String>>,
    background: Option<Result<Vec<BackgroundApp>, String>>,
    boot: Option<Result<BTreeMap<String, Vec<String>>, String>>,
    home: Option<Result<(Vec<String>, Option<String>), String>>,
    cached_limit: Option<Option<String>>,
    private_dns: Option<(String, String)>,
    loading: BTreeSet<&'static str>,
    busy: bool,
    speed_picks: BTreeSet<String>,
    runs: Option<Runs>,
    checkups: Option<Checkups>,
    measuring: bool,
}

#[derive(Debug, Clone)]
pub enum Msg {
    Section(Section),
    Refresh,
    TvRead(Result<TvState, String>),
    Pick(Step, bool),
    ApplyRecommended,
    RecommendedConfirmed(bool),
    /// The measurement and snapshot taken before the Recommended changes.
    RecommendedReady(JobId, Result<(Option<Checkup>, Result<crate::journal::State, String>), String>),
    RecommendedDone(JobId, Result<Outcome, String>),
    OpenOnTv(&'static str),
    Opened(Result<Outcome, String>),
    Measure,
    Measured(JobId, Result<Result<Checkup, String>, String>),
    StorageRead(Result<Storage, String>),
    BackgroundRead(Result<Vec<BackgroundApp>, String>),
    BootRead(Result<BTreeMap<String, Vec<String>>, String>),
    HomeRead(Result<(Vec<String>, Option<String>), String>, Option<String>, (String, String)),
    /// A change, confirmed when it needs to be.
    Run(Action),
    Confirmed(Action, bool),
    Done(JobId, Result<Result<Outcome, String>, String>),
    SpeedPick(String, bool),
    SpeedRun,
    SpeedConfirmed(bool),
    SpeedDone(JobId, Result<Vec<(String, Option<u32>)>, String>),
}

/// Everything the tab can change on the TV.
#[derive(Debug, Clone)]
pub enum Action {
    ClearAllCaches,
    ClearCache(String),
    ClearData(String),
    RemoveUpdate(String),
    DeleteFile(String),
    Restrict(String),
    Unrestrict(String),
    BlockBoot(String, Vec<String>),
    AllowBoot(String),
    SetHome(String),
    CachedLimit(Option<&'static str>),
    PrivateDns(Option<&'static str>),
}

impl Action {
    /// Asked first for anything that can't be undone or is noticeable.
    fn confirmation(&self) -> Option<(String, String)> {
        let name = |p: &str| format!("{} ({p})", health::app_label(p));
        Some(match self {
            Action::ClearAllCaches => return Fix::ClearCaches.confirmation(),
            Action::ClearCache(p) => return crate::android::packages::Action::ClearCache.confirmation(&[p.clone()]),
            Action::ClearData(p) => (
                "Clear Data".into(),
                format!(
                    "Delete EVERYTHING {} saved: logins, settings, add-ons, playlists and downloads, as if it were freshly \
                     installed?\n\nThis can't be undone. To free space safely, use Clear Cache instead.",
                    name(p)
                ),
            ),
            Action::RemoveUpdate(p) => (
                "Remove Update".into(),
                format!(
                    "Go back to the factory version of {}?\n\nThis frees the space its update takes. The Play Store may update it \
                     again later (it usually does when you open it). This can't be undone from here.",
                    name(p)
                ),
            ),
            Action::DeleteFile(path) => ("Delete File".into(), format!("Delete {path} from the TV?\n\nThis can't be undone.")),
            Action::BlockBoot(p, components) => (
                "Don't Start at Boot".into(),
                format!(
                    "Stop {} from starting when the TV turns on?\n\nIt still works when you open it. {} start-up \
                     component(s) are switched off; undo it from History if something stops working.",
                    name(p),
                    components.len()
                ),
            ),
            Action::SetHome(c) => (
                "Change Home App".into(),
                format!("Make {c} the home screen?\n\nThe Home button opens it from now on. Undo it from History or pick another here."),
            ),
            Action::CachedLimit(Some(n)) => (
                "Limit Cached Apps".into(),
                format!(
                    "Keep at most {n} apps cached in the background?\n\nLess swapping on low-memory TVs; apps you switch back to \
                     may reload. The TV may reset this when it syncs with Google. Undo it from History."
                ),
            ),
            _ => return None,
        })
    }

    async fn run(self, conn: crate::android::Connection) -> Result<Outcome, String> {
        let one = |line: String, change: Change| Outcome { lines: vec![line], changes: vec![change] };
        Ok(match self {
            Action::ClearAllCaches => return health::apply(conn, Fix::ClearCaches).await,
            Action::ClearCache(p) => {
                let progress = crate::jobs::Progress::new(Unit::Items);
                return Ok(crate::android::packages::run(conn, crate::android::packages::Action::ClearCache, vec![p], progress).await);
            }
            Action::ClearData(p) => {
                let out = conn.shell(format!("pm clear {}", crate::android::shell_quote(&p)), std::time::Duration::from_secs(60)).await;
                let out = out.map_err(|e| e.to_string())?;
                one(format!("Clear Data {p}: {}", out.trim()), Change::Note { text: format!("Cleared the data of {p}") })
            }
            Action::RemoveUpdate(p) => {
                let line = optimize::remove_system_update(conn, p.clone()).await?;
                one(line, Change::Note { text: format!("Removed the Play Store update of {p}") })
            }
            Action::DeleteFile(path) => {
                let line = optimize::delete_file(conn, path.clone()).await?;
                one(line, Change::Note { text: format!("Deleted {path}") })
            }
            Action::Restrict(p) => {
                let bucket = optimize::set_bucket(&conn, &p, "restricted").await?;
                let op = optimize::set_appop(&conn, &p, "RUN_ANY_IN_BACKGROUND", "ignore").await?;
                Outcome { lines: vec![format!("Restricted {p} in the background.")], changes: vec![bucket, op] }
            }
            Action::Unrestrict(p) => {
                let bucket = optimize::set_bucket(&conn, &p, "active").await?;
                let op = optimize::set_appop(&conn, &p, "RUN_ANY_IN_BACKGROUND", "default").await?;
                Outcome { lines: vec![format!("{p} may run in the background again.")], changes: vec![bucket, op] }
            }
            Action::BlockBoot(p, components) => {
                let mut outcome = Outcome::default();
                for c in components {
                    match optimize::set_component(&conn, &c, false).await {
                        Ok(change) => outcome.changes.push(change),
                        Err(e) => outcome.lines.push(format!("{c}: {e}")),
                    }
                }
                outcome.lines.insert(0, format!("{p} won't start at boot ({} component(s) off).", outcome.changes.len()));
                outcome
            }
            Action::AllowBoot(component) => {
                let change = optimize::set_component(&conn, &component, true).await?;
                one(format!("{component} starts at boot again."), change)
            }
            Action::SetHome(c) => {
                let change = optimize::set_home(&conn, &c).await?;
                one(format!("Home app is now {c}."), change)
            }
            Action::CachedLimit(n) => {
                let change = optimize::set_config(&conn, "activity_manager", "max_cached_processes", n).await?;
                one(change.describe(), change)
            }
            Action::PrivateDns(host) => {
                let mut changes = Vec::new();
                match host {
                    Some(h) => {
                        changes.push(changes::put_setting(&conn, "global", "private_dns_specifier", Some(h)).await?);
                        changes.push(changes::put_setting(&conn, "global", "private_dns_mode", Some("hostname")).await?);
                    }
                    None => changes.push(changes::put_setting(&conn, "global", "private_dns_mode", Some("off")).await?),
                }
                let line = match host {
                    Some(h) => format!("Private DNS set to {h}."),
                    None => "Private DNS turned off.".into(),
                };
                Outcome { lines: vec![line], changes }
            }
        })
    }
}

impl State {
    pub fn reset(&mut self) {
        *self = State::default();
    }

    pub fn on_open(&mut self, ctx: &mut Ctx) -> Task<Msg> {
        self.runs.get_or_insert_with(speedtests::load);
        self.checkups.get_or_insert_with(speedtests::load_checkups);
        self.load(self.section, false, ctx)
    }

    /// Read what a section shows (read-only); `force` re-reads.
    fn load(&mut self, section: Section, force: bool, ctx: &mut Ctx) -> Task<Msg> {
        let Some(conn) = ctx.conn() else { return Task::none() };
        let key = section.label();
        if self.loading.contains(key) {
            return Task::none();
        }
        let have = match section {
            Section::Recommended => self.tv.is_some(),
            Section::Storage => self.storage.is_some(),
            Section::Background | Section::SpeedTest => self.background.is_some(),
            Section::Startup => self.boot.is_some(),
            Section::HomeMemory => self.home.is_some(),
        };
        if have && !force {
            return Task::none();
        }
        self.loading.insert(key);
        match section {
            Section::Recommended => Task::perform(tuneup::read(conn), Msg::TvRead),
            Section::Storage => Task::perform(optimize::storage(conn), Msg::StorageRead),
            Section::Background | Section::SpeedTest => Task::perform(optimize::background_apps(conn), Msg::BackgroundRead),
            Section::Startup => Task::perform(optimize::boot_receivers(conn), Msg::BootRead),
            Section::HomeMemory => Task::perform(
                async move {
                    let home = optimize::home_apps(conn.clone()).await;
                    let cached = optimize::read_config(&conn, "activity_manager", "max_cached_processes").await.ok().flatten();
                    let get = |k: &'static str| {
                        let conn = conn.clone();
                        async move {
                            let out = conn.shell(format!("settings get global {k}"), std::time::Duration::from_secs(10)).await;
                            out.map(|s| s.trim().to_owned()).unwrap_or_default()
                        }
                    };
                    let dns = (get("private_dns_mode").await, get("private_dns_specifier").await);
                    (home, cached, dns)
                },
                |(home, cached, dns)| Msg::HomeRead(home, cached, dns),
            ),
        }
    }

    pub fn update(&mut self, msg: Msg, ctx: &mut Ctx) -> Task<Msg> {
        match msg {
            Msg::Section(section) => {
                self.section = section;
                return self.load(section, false, ctx);
            }
            Msg::Refresh => return self.load(self.section, true, ctx),
            Msg::TvRead(r) => {
                self.loading.remove(Section::Recommended.label());
                if ctx.session.is_none() {
                    return Task::none();
                }
                if let Ok(tv) = &r {
                    self.steps = tuneup::steps(tv);
                    self.picks = self.steps.iter().filter(|s| s.default_on()).cloned().collect();
                }
                self.tv = Some(r);
            }
            Msg::Pick(step, on) => {
                if on {
                    self.picks.insert(step);
                } else {
                    self.picks.remove(&step);
                }
            }
            Msg::ApplyRecommended => {
                if self.picks.is_empty() || self.busy {
                    return Task::none();
                }
                let list: Vec<String> = self.picks.iter().map(|s| format!("• {}", s.title())).collect();
                let mut question = trf(
                    "Make these {} change(s) on the TV?\n\n{}\n\nThe TV is measured and a snapshot is saved first, so you can \
                     compare afterwards (Speed Test) and put everything back (History → Snapshots).",
                    &[&self.picks.len(), &list.join("\n")],
                );
                if self.picks.contains(&Step::Compile) {
                    question.push_str("\n\n");
                    question.push_str(tr("Compiling takes several minutes and the TV is slow meanwhile."));
                }
                return ui::confirm("Apply Recommended Changes", question, Msg::RecommendedConfirmed);
            }
            Msg::RecommendedConfirmed(false) => {}
            Msg::RecommendedConfirmed(true) => {
                if self.busy {
                    return Task::none();
                }
                self.busy = true;
                ctx.log("Recommended changes: measuring the TV and saving a snapshot first…");
                // Measure before the snapshot: reading every setting makes the CPU busy.
                return ctx.job(
                    "Measuring and saving a snapshot",
                    Unit::Items,
                    |conn, _| async move {
                        let checkup = tuneup::checkup(conn.clone()).await.ok();
                        (checkup, changes::capture(conn).await)
                    },
                    Msg::RecommendedReady,
                );
            }
            Msg::RecommendedReady(id, result) => {
                ctx.jobs.finish(id);
                let (checkup, state) = match result {
                    Ok((checkup, Ok(state))) => (checkup, state),
                    Ok((_, Err(e))) | Err(e) => {
                        self.busy = false;
                        ctx.log(format!("Recommended changes not made: couldn't save a snapshot first ({e})."));
                        return Task::none();
                    }
                };
                let Some(session) = ctx.session else {
                    self.busy = false;
                    return Task::none();
                };
                ctx.journal.add_snapshot(session.address(), &session.label(), BEFORE_RECOMMENDED.into(), false, state);
                if let Err(e) = ctx.journal.save() {
                    ctx.log(format!("Could not save the snapshot: {e}"));
                }
                ctx.log(format!("Snapshot \"{BEFORE_RECOMMENDED}\" saved."));
                if let Some(mut checkup) = checkup {
                    checkup.note = BEFORE_RECOMMENDED.into();
                    self.add_checkup(session.address(), checkup, ctx);
                }
                let steps: Vec<Step> = self.picks.iter().cloned().collect();
                return ctx.job("Applying recommended changes", Unit::Items, move |conn, p| tuneup::apply(conn, steps, p), Msg::RecommendedDone);
            }
            Msg::RecommendedDone(id, result) => {
                ctx.jobs.finish(id);
                self.busy = false;
                match result {
                    Ok(outcome) => {
                        ctx.apply(outcome);
                        ctx.log(
                            "Recommended changes done. Measure again in Optimize → Speed Test once the TV has settled \
                             (a few minutes, or after a restart) to see the difference.",
                        );
                    }
                    Err(e) => ctx.log(format!("Recommended changes failed: {e}")),
                }
                // Free space and the app lists changed too.
                self.storage = None;
                self.background = None;
                self.boot = None;
                return self.load(Section::Recommended, true, ctx);
            }
            Msg::OpenOnTv(screen) => {
                let Some(conn) = ctx.conn() else { return Task::none() };
                return Task::perform(actions::run(conn, actions::Action::OpenSettings(screen)), Msg::Opened);
            }
            Msg::Opened(result) => match result {
                Ok(outcome) => ctx.apply(outcome),
                Err(e) => ctx.log(format!("Couldn't open the screen: {e}")),
            },
            Msg::Measure => {
                if self.measuring || self.busy {
                    return Task::none();
                }
                self.measuring = true;
                return ctx.job("Measuring the TV", Unit::Items, |conn, _| tuneup::checkup(conn), Msg::Measured);
            }
            Msg::Measured(id, result) => {
                ctx.jobs.finish(id);
                self.measuring = false;
                match result.and_then(|r| r) {
                    Ok(c) => {
                        ctx.log(format!(
                            "Measured: CPU {:.0}% busy, {} memory free, {} swap used, {} storage free, {} apps running.",
                            c.cpu_busy,
                            fmt_bytes(c.mem_available),
                            fmt_bytes(c.swap_used),
                            fmt_bytes(c.storage_free),
                            c.apps_running
                        ));
                        if let Some(session) = ctx.session {
                            self.add_checkup(session.address(), c, ctx);
                        }
                    }
                    Err(e) => ctx.log(format!("Couldn't measure the TV: {e}")),
                }
            }
            Msg::StorageRead(r) => {
                self.loading.remove(Section::Storage.label());
                self.storage = ctx.session.map(|_| r);
            }
            Msg::BackgroundRead(r) => {
                self.loading.remove(Section::Background.label());
                self.loading.remove(Section::SpeedTest.label());
                self.background = ctx.session.map(|_| r);
            }
            Msg::BootRead(r) => {
                self.loading.remove(Section::Startup.label());
                self.boot = ctx.session.map(|_| r);
            }
            Msg::HomeRead(home, cached, dns) => {
                self.loading.remove(Section::HomeMemory.label());
                if ctx.session.is_some() {
                    self.home = Some(home);
                    self.cached_limit = Some(cached);
                    self.private_dns = Some(dns);
                }
            }
            Msg::Run(action) => {
                if self.busy {
                    return Task::none();
                }
                return match action.confirmation() {
                    Some((title, question)) => ui::confirm(title, question, move |yes| Msg::Confirmed(action, yes)),
                    None => self.update(Msg::Confirmed(action, true), ctx),
                };
            }
            Msg::Confirmed(_, false) => {}
            Msg::Confirmed(action, true) => {
                self.busy = true;
                let title = match &action {
                    Action::ClearAllCaches => "Clearing caches".to_owned(),
                    other => format!("{other:?}").split('(').next().unwrap_or("Optimizing").to_owned(),
                };
                return ctx.job(title, Unit::Items, move |conn, _| action.run(conn), Msg::Done);
            }
            Msg::Done(id, result) => {
                ctx.jobs.finish(id);
                self.busy = false;
                match result.and_then(|r| r) {
                    Ok(outcome) => ctx.apply(outcome),
                    Err(e) => ctx.log(format!("Failed: {e}")),
                }
                return self.load(self.section, true, ctx);
            }
            Msg::SpeedPick(p, on) => {
                if on {
                    self.speed_picks.insert(p);
                } else {
                    self.speed_picks.remove(&p);
                }
            }
            Msg::SpeedRun => {
                if self.speed_picks.is_empty() || self.busy {
                    return Task::none();
                }
                return ui::confirm(
                    "Speed Test",
                    format!(
                        "Time how long {} app(s) take to open?\n\nEach app is closed, opened on the TV and then left with Home, \
                         so whatever is playing is interrupted. It takes about 5 seconds per app.",
                        self.speed_picks.len()
                    ),
                    Msg::SpeedConfirmed,
                );
            }
            Msg::SpeedConfirmed(false) => {}
            Msg::SpeedConfirmed(true) => {
                self.busy = true;
                let picks: Vec<String> = self.speed_picks.iter().cloned().collect();
                ctx.log(format!("Speed test: timing {} app launch(es)…", picks.len()));
                return ctx.job("Speed test", Unit::Items, move |conn, p| optimize::speed_test(conn, picks, p), Msg::SpeedDone);
            }
            Msg::SpeedDone(id, result) => {
                ctx.jobs.finish(id);
                self.busy = false;
                match result {
                    Ok(results) => {
                        let Some(session) = ctx.session else { return Task::none() };
                        let runs = self.runs.get_or_insert_with(speedtests::load);
                        let run = speedtests::Run { time: crate::journal::now(), results };
                        speedtests::add(runs, session.address(), run);
                        if let Err(e) = speedtests::save(runs) {
                            ctx.log(format!("Could not save the speed test: {e}"));
                        }
                        ctx.log("Speed test finished.");
                    }
                    Err(e) => ctx.log(format!("Speed test failed: {e}")),
                }
            }
        }
        Task::none()
    }

    pub fn view<'a>(&'a self, v: &View<'a>) -> Element<'a, Msg> {
        let serial = v.session.map(|s| s.address()).unwrap_or("");
        let pills = row(Section::ALL.iter().map(|&s| {
            let active = s == self.section;
            let caption = if active { ds::medium(s.label(), ds::TEXT_BASE, ds::TEXT) } else { ds::label(s.label(), ds::TEXT_BASE, ds::TEXT_SECONDARY) };
            button(caption).height(24).padding([0, 10]).style(style::pill(active)).on_press(Msg::Section(s)).into()
        }))
        .spacing(4)
        .align_y(Alignment::Center);
        let loading = self.loading.contains(self.section.label());
        let bar = row![
            pills,
            space::horizontal(),
            ds::icon_button(Some((ds::Icon::RefreshCw, ds::TEXT_SECONDARY)), if loading { "Reading…" } else { "Refresh" }, None, style::standard, ds::CONTROL_HEIGHT, (!loading).then_some(Msg::Refresh)),
        ]
        .spacing(8)
        .align_y(Alignment::Center);
        let toolbar = container(column![container(bar).padding([0, 12]).center_y(Length::Fill), ds::hline(ds::BORDER)])
            .height(ds::TOOLBAR_HEIGHT)
            .style(style::fill(ds::SIDEBAR));
        let body = match self.section {
            Section::Recommended => self.recommended_view(),
            Section::Storage => self.storage_view(),
            Section::Background => self.background_view(),
            Section::Startup => self.startup_view(v.journal, serial),
            Section::HomeMemory => self.home_view(),
            Section::SpeedTest => self.speed_view(serial),
        };
        column![toolbar, container(body).height(Length::Fill)].into()
    }

    fn waiting<'a>(&self, what: Option<&'a Result<impl Sized, String>>) -> Option<Element<'a, Msg>> {
        match what {
            None if self.loading.contains(self.section.label()) => Some(ui::empty_state("Reading…", "")),
            None => Some(ui::empty_state("Not read yet", "Press Refresh.")),
            Some(Err(e)) => Some(ui::empty_state("Couldn't read this", e)),
            Some(Ok(_)) => None,
        }
    }

    fn action_button<'a>(&self, caption: &'a str, style_fn: ui::ButtonStyle, action: Action) -> Element<'a, Msg> {
        ui::small_button(caption, style_fn, (!self.busy).then_some(Msg::Run(action)))
    }

    fn add_checkup(&mut self, serial: &str, checkup: Checkup, ctx: &mut Ctx) {
        let checkups = self.checkups.get_or_insert_with(speedtests::load_checkups);
        speedtests::add_checkup(checkups, serial, checkup);
        if let Err(e) = speedtests::save_checkups(checkups) {
            ctx.log(format!("Could not save the measurement: {e}"));
        }
    }

    // ── Recommended ──

    fn recommended_view(&self) -> Element<'_, Msg> {
        if let Some(e) = self.waiting(self.tv.as_ref()) {
            return e;
        }
        let Some(Ok(tv)) = &self.tv else { return ui::empty_state("", "") };
        let mut list = column![ds::label(
            "Safe changes, made together. The TV is measured and a snapshot is saved first, so you can compare afterwards \
             (Speed Test) and put everything back (History → Snapshots). Disabled apps can also be turned back on from Packages.",
            ds::TEXT_SM,
            ds::TEXT_SECONDARY
        )]
        .spacing(10);
        for step in &self.steps {
            let s = step.clone();
            list = list.push(
                row![
                    checkbox(self.picks.contains(step)).on_toggle(move |on| Msg::Pick(s.clone(), on)).size(14).style(style::checkbox),
                    column![ds::label(step.title(), ds::TEXT_BASE, ds::TEXT), ds::label(step.detail(), ds::TEXT_SM, ds::TEXT_TERTIARY)]
                        .spacing(2)
                        .width(Length::Fill),
                ]
                .spacing(10),
            );
        }
        let mut done = Vec::new();
        if tv.bloat.is_empty() {
            done.push(tr("no bloatware left enabled"));
        }
        if tv.animations_fast {
            done.push(tr("animations sped up"));
        }
        if tv.freezer_on {
            done.push(tr("app freezer on"));
        }
        if !done.is_empty() {
            list = list.push(ds::label(trf("Already done: {}.", &[&done.join(", ")]), ds::TEXT_SM, ds::GREEN));
        }
        let picked = self.picks.len();
        list = list.push(
            row![
                ds::icon_button(
                    Some((ds::Icon::Rocket, ds::ACCENT_TEXT)),
                    if self.busy { "Working…" } else { "Apply Selected Changes" },
                    None,
                    style::primary,
                    32.0,
                    (picked > 0 && !self.busy).then_some(Msg::ApplyRecommended),
                ),
                ds::label(trf("{} of {} selected", &[&picked, &self.steps.len()]), ds::TEXT_SM, ds::TEXT_TERTIARY),
            ]
            .spacing(10)
            .align_y(Alignment::Center),
        );
        ui::page(column![ui::card("Recommended", list.into()), ui::card("TV Settings", self.tv_settings(tv))].spacing(12))
    }

    /// Settings that matter for speed but live where ADB can't (or
    /// shouldn't) change them: what the TV reports, and a way to the screen.
    fn tv_settings(&self, tv: &TvState) -> Element<'_, Msg> {
        let item = |title: &'static str, status: String, color: Color, advice: &'static str, screen: Option<&'static str>| -> Element<'static, Msg> {
            let mut line = row![
                column![
                    row![ds::medium(title, ds::TEXT_BASE, ds::TEXT), ds::label(status, ds::TEXT_SM, color)].spacing(8).align_y(Alignment::Center),
                    ds::label(advice, ds::TEXT_SM, ds::TEXT_TERTIARY),
                ]
                .spacing(2)
                .width(Length::Fill)
            ]
            .spacing(10)
            .align_y(Alignment::Center);
            if let Some(s) = screen {
                line = line.push(ui::small_button("Open on TV", style::standard, Some(Msg::OpenOnTv(s))));
            }
            line.into()
        };
        let unreadable = || tr("can't be read over ADB").to_owned();
        let (mic, mic_color) = match tv.mic_muted {
            Some(true) => (tr("microphone muted").to_owned(), ds::GREEN),
            Some(false) => (tr("microphone not muted").to_owned(), ds::TEXT_SECONDARY),
            None => (tr("unknown").to_owned(), ds::TEXT_TERTIARY),
        };
        let (saver, saver_color) = match &tv.screensaver {
            None => (tr("turned off").to_owned(), ds::GREEN),
            Some(p) if tv.screensaver_disabled => (trf("set to {}, a disabled app, so the TV uses its default one", &[&health::app_label(p)]), ds::TEXT_SECONDARY),
            Some(p) => (trf("on: {}", &[&health::app_label(p)]), ds::TEXT_SECONDARY),
        };
        let (uptime, uptime_color) = match tv.uptime_secs {
            Some(secs) => {
                let (d, h, m) = (secs / 86_400, secs % 86_400 / 3_600, secs % 3_600 / 60);
                let ago = if d > 0 { format!("{d} d {h} h") } else { format!("{h} h {m} min") };
                (trf("{} ago", &[&ago]), if d >= RESTART_AFTER_DAYS { ds::AMBER } else { ds::GREEN })
            }
            None => (tr("unknown").to_owned(), ds::TEXT_TERTIARY),
        };
        column![
            item(
                "Apps only mode",
                unreadable(),
                ds::TEXT_TERTIARY,
                "Settings → Accounts & Sign In → your account → Apps only mode. The home screen stops loading recommendations and \
                 trailers, the biggest relief for the launcher. Leave it off if you like those suggestions.",
                Some(ACCOUNTS_SCREEN),
            ),
            item(
                "Hands-free \"Hey Google\"",
                mic,
                mic_color,
                "A TV with a built-in microphone can listen for \"Hey Google\" all the time. If you only use the remote's mic button, \
                 turn hands-free off in the Google Assistant settings or with the mic switch on the TV.",
                Some(ASSISTANT_SCREEN),
            ),
            item(
                "App auto-updates",
                unreadable(),
                ds::TEXT_TERTIARY,
                "Play Store → Settings → Auto-update apps. Updating by hand now and then stops updates installing while you watch \
                 and filling the storage.",
                Some(PLAY_STORE_SCREEN),
            ),
            item(
                "Screen saver",
                saver,
                saver_color,
                "Photo screen savers download pictures while the TV is idle; a simple one (or none) uses less memory and network.",
                Some(SCREEN_SAVER_SCREEN),
            ),
            item(
                "Last restart",
                uptime,
                uptime_color,
                "A restart about once a week clears memory that builds up while the TV sits in standby. Use Tools → Reboot or the \
                 TV's power menu.",
                None,
            ),
        ]
        .spacing(12)
        .into()
    }

    // ── Storage ──

    fn storage_view(&self) -> Element<'_, Msg> {
        if let Some(e) = self.waiting(self.storage.as_ref()) {
            return e;
        }
        let Some(Ok(s)) = &self.storage else { return ui::empty_state("", "") };
        let used = if s.total > 0 { 1.0 - s.free as f32 / s.total as f32 } else { 0.0 };
        let summary = ui::card(
            "Free Space",
            column![
                row![
                    ds::label(format!("{} free of {}", fmt_bytes(s.free), fmt_bytes(s.total)), ds::TEXT_BASE, ds::TEXT),
                    space::horizontal(),
                    self.action_button("Clear All App Caches", style::primary, Action::ClearAllCaches),
                ]
                .align_y(Alignment::Center),
                progress_bar(0.0..=1.0, used).girth(6).style(style::usage(used)),
                ds::label(
                    "Below about 10% free, Android stops optimising apps in the background and the storage gets slower. Clearing \
                     caches is safe: logins and settings stay, apps rebuild what they need. Android 12 keeps part of the cache of \
                     apps you use a lot; clear those with Clear Cache in Biggest Apps.",
                    ds::TEXT_SM,
                    ds::TEXT_SECONDARY,
                ),
            ]
            .spacing(10)
            .into(),
        );
        let measured = match &s.apps_measured {
            Some(t) => crate::i18n::trf("Sizes from Android's own report of {} (it's refreshed about once a day, so it can lag behind a clean-up; Free Space above is live).", &[t]),
            None => crate::i18n::tr("Android's own size report (it updates now and then, and may list only some apps).").to_owned(),
        };
        let mut apps = column![
            ds::label(measured, ds::TEXT_SM, ds::TEXT_TERTIARY),
            ds::label("Clear Cache is safe; Clear Data resets the app (logins, settings, add-ons).", ds::TEXT_SM, ds::TEXT_TERTIARY),
        ]
        .spacing(6);
        for (pkg, app, data, cache) in s.apps.iter().take(15) {
            apps = apps.push(
                row![
                    column![ds::label(health::app_label(pkg), ds::TEXT_BASE, ds::TEXT), ds::mono(pkg.clone(), ds::TEXT_XS, ds::TEXT_TERTIARY)]
                        .spacing(2)
                        .width(Length::Fill),
                    ds::mono(format!("app {}", fmt_bytes(*app)), ds::TEXT_SM, ds::TEXT_SECONDARY).width(110),
                    // Android counts the cache inside "data"; show what clearing data would really lose.
                    ds::mono(format!("data {}", fmt_bytes(data.saturating_sub(*cache))), ds::TEXT_SM, ds::TEXT).width(120),
                    ds::mono(format!("cache {}", fmt_bytes(*cache)), ds::TEXT_SM, if *cache > 500 << 20 { ds::AMBER } else { ds::TEXT }).width(120),
                    self.action_button("Clear Cache", style::standard, Action::ClearCache(pkg.clone())),
                    self.action_button("Clear Data", style::danger, Action::ClearData(pkg.clone())),
                ]
                .spacing(10)
                .align_y(Alignment::Center),
            );
        }
        let mut updates = column![ds::label(
            "Preinstalled apps carry their Play Store update as well as the factory copy. Removing an update you don't use frees its space.",
            ds::TEXT_SM,
            ds::TEXT_TERTIARY
        )]
        .spacing(6);
        for u in &s.updates {
            let used = match u.used_secs {
                Some(0) => crate::i18n::tr("no screen time this month").to_owned(),
                Some(secs) => crate::i18n::trf("{} min on screen this month", &[&secs.div_ceil(60)]),
                None => String::new(),
            };
            let control = if optimize::keep_update(&u.package) {
                tooltip(
                    ds::label("Keep", ds::TEXT_SM, ds::TEXT_TERTIARY),
                    container(ds::label("Other apps rely on this update; removing it can break them.", ds::TEXT_SM, ds::TEXT))
                        .padding([4, 8])
                        .style(style::flyout),
                    tooltip::Position::Left,
                )
                .into()
            } else {
                self.action_button("Remove Update", style::standard, Action::RemoveUpdate(u.package.clone()))
            };
            updates = updates.push(
                row![
                    column![ds::label(health::app_label(&u.package), ds::TEXT_BASE, ds::TEXT), ds::mono(u.package.clone(), ds::TEXT_XS, ds::TEXT_TERTIARY)]
                        .spacing(2)
                        .width(Length::Fill),
                    ds::label(used, ds::TEXT_SM, if u.used_secs == Some(0) { ds::AMBER } else { ds::TEXT_TERTIARY }).width(170),
                    ds::mono(fmt_bytes(u.bytes), ds::TEXT_SM, ds::TEXT).width(80),
                    container(control).width(130).align_right(130),
                ]
                .spacing(10)
                .align_y(Alignment::Center),
            );
        }
        let mut files = column![].spacing(6);
        if s.big_files.is_empty() {
            files = files.push(ds::label("No files over 20 MB in the TV's shared storage.", ds::TEXT_BASE, ds::TEXT_TERTIARY));
        }
        for (path, bytes) in &s.big_files {
            files = files.push(
                row![
                    ds::mono(path.clone(), ds::TEXT_SM, ds::TEXT).width(Length::Fill),
                    ds::mono(fmt_bytes(*bytes), ds::TEXT_SM, ds::TEXT).width(80),
                    self.action_button("Delete", style::danger, Action::DeleteFile(path.clone())),
                ]
                .spacing(10)
                .align_y(Alignment::Center),
            );
        }
        ui::page(
            column![
                summary,
                ui::card("Biggest Apps", apps.into()),
                ui::card("System App Updates", updates.into()),
                ui::card("Large Files", files.into())
            ]
            .spacing(12),
        )
    }

    // ── Background ──

    fn background_view(&self) -> Element<'_, Msg> {
        if let Some(e) = self.waiting(self.background.as_ref()) {
            return e;
        }
        let Some(Ok(apps)) = &self.background else { return ui::empty_state("", "") };
        let mut rows = column![ds::label(
            "Restricted apps still open normally, but can't wake up or run in the background on their own. Good for apps you rarely \
             use; not for VPNs, file servers or anything that must keep running. Android may move an app back when you use it.",
            ds::TEXT_SM,
            ds::TEXT_SECONDARY
        )]
        .spacing(6);
        for a in apps {
            let restricted = a.bucket == "restricted" || a.background == "ignore";
            let used = match a.used_secs {
                Some(0) => crate::i18n::tr("no screen time this month").to_owned(),
                Some(secs) => crate::i18n::trf("{} min on screen this month", &[&secs.div_ceil(60)]),
                None => String::new(),
            };
            let state = format!("{} · bg {}", a.bucket, a.background);
            rows = rows.push(
                row![
                    column![ds::label(health::app_label(&a.package), ds::TEXT_BASE, ds::TEXT), ds::mono(a.package.clone(), ds::TEXT_XS, ds::TEXT_TERTIARY)]
                        .spacing(2)
                        .width(Length::Fill),
                    ds::label(used, ds::TEXT_SM, ds::TEXT_TERTIARY).width(150),
                    container(tooltip(
                        ds::mono(state, ds::TEXT_XS, if restricted { ds::PURPLE } else { ds::TEXT_SECONDARY }).wrapping(iced::widget::text::Wrapping::None),
                        container(ds::label("Standby group · whether it may run in the background", ds::TEXT_SM, ds::TEXT)).padding([4, 8]).style(style::flyout),
                        tooltip::Position::Left,
                    ))
                    .width(200),
                    if restricted {
                        self.action_button("Allow", style::standard, Action::Unrestrict(a.package.clone()))
                    } else {
                        self.action_button("Restrict", style::standard, Action::Restrict(a.package.clone()))
                    },
                ]
                .spacing(10)
                .align_y(Alignment::Center),
            );
        }
        ui::page(ui::card("Background Apps", rows.into()))
    }

    // ── Startup ──

    fn startup_view<'a>(&'a self, journal: &'a crate::journal::Journal, serial: &str) -> Element<'a, Msg> {
        if let Some(e) = self.waiting(self.boot.as_ref()) {
            return e;
        }
        let Some(Ok(boot)) = &self.boot else { return ui::empty_state("", "") };
        // Turned off by this app (and not undone): they no longer show up as receivers.
        let mut off: Vec<String> = Vec::new();
        for e in journal.entries_for(serial).filter(|e| e.reverted.is_none()) {
            if let Change::Component { component, after: false, .. } = &e.change
                && !off.contains(component)
                && !boot.values().flatten().any(|c| c == component)
            {
                off.push(component.clone());
            }
        }
        let mut rows = column![ds::label(
            "Apps that start themselves when the TV turns on. Turning that off makes start-up lighter; the app still works when you \
             open it. Core services are left alone.",
            ds::TEXT_SM,
            ds::TEXT_SECONDARY
        )]
        .spacing(6);
        let mut list: Vec<(&String, &Vec<String>)> = boot.iter().collect();
        list.sort_by_key(|(p, _)| (optimize::is_protected(p), health::app_label(p)));
        for (pkg, components) in list {
            let protected = optimize::is_protected(pkg);
            let control: Element<'_, Msg> = if protected {
                ds::label("Core service", ds::TEXT_SM, ds::TEXT_TERTIARY).into()
            } else {
                self.action_button("Don't Start at Boot", style::standard, Action::BlockBoot(pkg.clone(), components.clone()))
            };
            rows = rows.push(
                row![
                    column![ds::label(health::app_label(pkg), ds::TEXT_BASE, if protected { ds::TEXT_TERTIARY } else { ds::TEXT }), ds::mono(pkg.clone(), ds::TEXT_XS, ds::TEXT_TERTIARY)]
                        .spacing(2)
                        .width(Length::Fill),
                    ds::mono(format!("{} component(s)", components.len()), ds::TEXT_SM, ds::TEXT_TERTIARY).width(130),
                    control,
                ]
                .spacing(10)
                .align_y(Alignment::Center),
            );
        }
        let mut page = column![ui::card("Startup Apps", rows.into())].spacing(12);
        if !off.is_empty() {
            let mut list = column![].spacing(6);
            for c in off {
                list = list.push(
                    row![ds::mono(c.clone(), ds::TEXT_SM, ds::TEXT).width(Length::Fill), self.action_button("Start at Boot Again", style::standard, Action::AllowBoot(c))]
                        .spacing(10)
                        .align_y(Alignment::Center),
                );
            }
            page = page.push(ui::card("Turned Off by This App", list.into()));
        }
        ui::page(page)
    }

    // ── Home & memory ──

    fn home_view(&self) -> Element<'_, Msg> {
        if let Some(e) = self.waiting(self.home.as_ref()) {
            return e;
        }
        let Some(Ok((homes, current))) = &self.home else { return ui::empty_state("", "") };
        let mut list = column![ds::label(
            "The home screen runs all the time; a lighter one leaves more memory for apps. Install one first (for example Projectivy \
             Launcher or FLauncher from the Play Store), then pick it here.",
            ds::TEXT_SM,
            ds::TEXT_SECONDARY
        )]
        .spacing(6);
        for c in homes {
            let pkg = c.split('/').next().unwrap_or("");
            let using = current.as_deref() == Some(c.as_str());
            list = list.push(
                row![
                    column![ds::label(health::app_label(pkg), ds::TEXT_BASE, ds::TEXT), ds::mono(c.clone(), ds::TEXT_XS, ds::TEXT_TERTIARY)]
                        .spacing(2)
                        .width(Length::Fill),
                    if using {
                        ds::tinted_badge("In use", ds::GREEN)
                    } else {
                        self.action_button("Use as Home", style::standard, Action::SetHome(c.clone()))
                    },
                ]
                .spacing(10)
                .align_y(Alignment::Center),
            );
        }
        let cached = self.cached_limit.clone().flatten();
        let mut limits = row![].spacing(6).align_y(Alignment::Center);
        for (label, value) in CACHED_LIMITS {
            let active = cached.as_deref() == value;
            limits = limits.push(
                button(text(label).size(ds::TEXT_SM))
                    .padding([4, 12])
                    .style(style::pill(active))
                    .on_press_maybe((!active && !self.busy).then_some(Msg::Run(Action::CachedLimit(value)))),
            );
        }
        let memory = column![
            ds::label(
                "How many closed apps Android keeps in memory. A lower number means less swapping on TVs with little memory, but apps \
                 you return to may reload.",
                ds::TEXT_SM,
                ds::TEXT_SECONDARY
            ),
            row![ds::label("Cached apps:", ds::TEXT_BASE, ds::TEXT), limits].spacing(10).align_y(Alignment::Center),
        ]
        .spacing(8);
        let (mode, host) = self.private_dns.clone().unwrap_or_default();
        // Never set counts as off.
        let mode = if mode.is_empty() || mode == "null" { "off".to_owned() } else { mode };
        let dns_now = if mode == "hostname" { host.clone() } else { mode.clone() };
        let mut dns = row![].spacing(6).align_y(Alignment::Center);
        for (name, h) in AD_BLOCK_DNS {
            let active = mode == "hostname" && host == h;
            dns = dns.push(
                button(text(name).size(ds::TEXT_SM))
                    .padding([4, 12])
                    .style(style::pill(active))
                    .on_press_maybe((!active && !self.busy).then_some(Msg::Run(Action::PrivateDns(Some(h))))),
            );
        }
        dns = dns.push(
            button(text("Off").size(ds::TEXT_SM))
                .padding([4, 12])
                .style(style::pill(mode == "off"))
                .on_press_maybe((mode != "off" && !self.busy).then_some(Msg::Run(Action::PrivateDns(None)))),
        );
        let ads = column![
            ds::label(
                "Private DNS with an ad-blocking provider removes many ads and trackers in apps and menus, for the whole TV. Some \
                 services that rely on ads may misbehave; switch it off if so.",
                ds::TEXT_SM,
                ds::TEXT_SECONDARY
            ),
            row![ds::label("Now:", ds::TEXT_BASE, ds::TEXT), ds::mono(dns_now, ds::TEXT_SM, ds::TEXT_SECONDARY), space::horizontal(), dns]
                .spacing(10)
                .align_y(Alignment::Center),
        ]
        .spacing(8);
        ui::page(column![ui::card("Home App", list.into()), ui::card("Memory", memory.into()), ui::card("Ad Blocking", ads.into())].spacing(12))
    }

    // ── Speed test ──

    fn speed_view(&self, serial: &str) -> Element<'_, Msg> {
        let runs = self.runs.as_ref().and_then(|r| r.get(&crate::journal::device_key(serial)));
        let mut results = column![].spacing(6);
        match runs {
            Some(runs) if !runs.is_empty() => {
                let latest = runs.last().map(|r| r.time.clone()).unwrap_or_default();
                results = results.push(ds::label(format!("Latest run: {latest} ({} run(s) saved)", runs.len()), ds::TEXT_SM, ds::TEXT_TERTIARY));
                for (pkg, ms, before) in speedtests::compare(runs) {
                    let show = |m: Option<u32>| m.map_or("—".to_owned(), |m| format!("{:.1} s", m as f32 / 1000.0));
                    let delta = match (ms, before) {
                        (Some(a), Some(b)) if b > 0 => {
                            let pct = (a as f32 - b as f32) / b as f32 * 100.0;
                            ds::mono(format!("{pct:+.0}%"), ds::TEXT_SM, if pct <= 0.0 { ds::GREEN } else { ds::AMBER })
                        }
                        _ => ds::mono("", ds::TEXT_SM, ds::TEXT_TERTIARY),
                    };
                    results = results.push(
                        row![
                            ds::label(health::app_label(&pkg), ds::TEXT_BASE, ds::TEXT).width(Length::Fill),
                            ds::mono(format!("before {}", show(before)), ds::TEXT_SM, ds::TEXT_TERTIARY).width(120),
                            ds::mono(format!("now {}", show(ms)), ds::TEXT_SM, ds::TEXT).width(100),
                            delta.width(60),
                        ]
                        .spacing(10)
                        .align_y(Alignment::Center),
                    );
                }
            }
            _ => results = results.push(ds::label("No runs yet. Run one before optimizing and one after to compare.", ds::TEXT_BASE, ds::TEXT_TERTIARY)),
        }
        let picker: Element<'_, Msg> = match &self.background {
            Some(Ok(apps)) => {
                let mut list = column![].spacing(4);
                for a in apps {
                    let ticked = self.speed_picks.contains(&a.package);
                    let pkg = a.package.clone();
                    list = list.push(
                        checkbox(ticked)
                            .label(format!("{}  ({})", health::app_label(&a.package), a.package))
                            .on_toggle(move |on| Msg::SpeedPick(pkg.clone(), on))
                            .size(14)
                            .text_size(ds::TEXT_BASE)
                            .style(style::checkbox),
                    );
                }
                list.into()
            }
            _ => ds::label("Reading the TV's apps…", ds::TEXT_BASE, ds::TEXT_TERTIARY).into(),
        };
        let run = column![
            ds::label(
                "Times how long each app takes to open from closed. It opens the apps on the TV, so run it when nobody is watching.",
                ds::TEXT_SM,
                ds::AMBER
            ),
            picker,
            ds::icon_button(
                Some((ds::Icon::Timer, ds::ACCENT_TEXT)),
                "Run Speed Test",
                None,
                style::primary,
                32.0,
                (!self.speed_picks.is_empty() && !self.busy).then_some(Msg::SpeedRun),
            ),
        ]
        .spacing(10);
        ui::page(column![self.checkups_view(serial), ui::card("App Launch Times", results.into()), ui::card("Run", run.into())].spacing(12))
    }

    /// Measurements of the TV at rest, each compared with the one before.
    fn checkups_view(&self, serial: &str) -> Element<'_, Msg> {
        let list = self.checkups.as_ref().and_then(|c| c.get(&crate::journal::device_key(serial)));
        let mut body = column![ds::label(
            "How hard the TV works at rest: CPU over 10 seconds, free memory, swap, free storage and running apps. It only reads, \
             so it's safe any time. Measure with the TV in the same state each time (for example on the home screen, nothing playing).",
            ds::TEXT_SM,
            ds::TEXT_SECONDARY
        )]
        .spacing(6);
        match list {
            Some(list) if !list.is_empty() => {
                let head = |caption: &'static str, width: f32| ds::label(caption, ds::TEXT_XS, ds::TEXT_TERTIARY).width(width);
                body = body.push(
                    row![
                        ds::label("When", ds::TEXT_XS, ds::TEXT_TERTIARY).width(Length::Fill),
                        head("CPU busy", 80.0),
                        head("Free memory", 100.0),
                        head("Swap used", 100.0),
                        head("Free storage", 110.0),
                        head("Apps running", 90.0),
                    ]
                    .spacing(10),
                );
                const MB: f64 = 1024.0 * 1024.0;
                let mut previous: Option<&Checkup> = None;
                for c in list {
                    // Green when clearly better than the measurement before, amber when worse.
                    let shade = |now: f64, before: Option<f64>, margin: f64, lower_is_better: bool| match before {
                        Some(b) if (now - b).abs() >= margin => if (now < b) == lower_is_better { ds::GREEN } else { ds::AMBER },
                        _ => ds::TEXT,
                    };
                    let cell = |value: String, color: Color, width: f32| ds::mono(value, ds::TEXT_SM, color).width(width);
                    let mut when = column![ds::mono(super::history::short_time(&c.time), ds::TEXT_SM, ds::TEXT)].spacing(2);
                    if !c.note.is_empty() {
                        when = when.push(ds::label(c.note.as_str(), ds::TEXT_XS, ds::TEXT_TERTIARY));
                    }
                    body = body.push(
                        row![
                            when.width(Length::Fill),
                            cell(format!("{:.0}%", c.cpu_busy), shade(c.cpu_busy as f64, previous.map(|p| p.cpu_busy as f64), 3.0, true), 80.0),
                            cell(
                                fmt_bytes(c.mem_available),
                                shade(c.mem_available as f64, previous.map(|p| p.mem_available as f64), 50.0 * MB, false),
                                100.0
                            ),
                            cell(fmt_bytes(c.swap_used), shade(c.swap_used as f64, previous.map(|p| p.swap_used as f64), 50.0 * MB, true), 100.0),
                            cell(
                                fmt_bytes(c.storage_free),
                                shade(c.storage_free as f64, previous.map(|p| p.storage_free as f64), 50.0 * MB, false),
                                110.0
                            ),
                            cell(c.apps_running.to_string(), shade(c.apps_running as f64, previous.map(|p| p.apps_running as f64), 2.0, true), 90.0),
                        ]
                        .spacing(10)
                        .align_y(Alignment::Center),
                    );
                    previous = Some(c);
                }
            }
            _ => body = body.push(ds::label("No measurements yet. Measure once now and again after optimizing.", ds::TEXT_BASE, ds::TEXT_TERTIARY)),
        }
        body = body.push(ds::icon_button(
            Some((ds::Icon::Gauge, ds::ACCENT_TEXT)),
            if self.measuring { "Measuring…" } else { "Measure Now" },
            None,
            style::primary,
            32.0,
            (!self.measuring && !self.busy).then_some(Msg::Measure),
        ));
        ui::card("Before & After", body.spacing(10).into())
    }
}
