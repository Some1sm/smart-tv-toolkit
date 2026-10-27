//! Performance: the Health Check and the optimisation tools.

use iced::widget::{button, column, container, progress_bar, row, space};
use iced::{Alignment, Element, Fill, Length, Task};

use super::{Ctx, View};
use crate::ui::text;
use crate::android::changes::Outcome;
use crate::android::health::{self, Finding, Fix, Severity};
use crate::android::processes::{self, Process};
use crate::android::sysinfo::fmt_bytes;
use crate::android::Connection;
use crate::jobs::{JobId, Unit};
use crate::design::{self as ds, style};
use crate::ui::{self, ButtonStyle};

#[derive(Debug, Default)]
enum Health {
    #[default]
    Idle,
    Scanning,
    Failed(String),
    Done { findings: Vec<Finding>, scanned_at: String },
}

/// Which processes the list shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Show {
    #[default]
    All,
    Apps,
    System,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortBy {
    #[default]
    Cpu,
    Memory,
    Name,
}

/// Processes listed at once.
const PROCESS_ROWS: usize = 30;

#[derive(Debug, Default)]
pub struct State {
    health: Health,
    /// A fix is running (its buttons are disabled meanwhile).
    fixing: bool,
    processes: Vec<Process>,
    /// Its own connection, so `top` never holds up the remote or heartbeat.
    process_conn: Option<Connection>,
    sampling: bool,
    paused: bool,
    show: Show,
    sort: SortBy,
}

#[derive(Debug, Clone)]
pub enum Msg {
    RunHealthCheck,
    Scanned(JobId, Result<Result<Vec<Finding>, String>, String>),
    FixPressed(Fix),
    FixConfirmed(Fix, bool),
    FixApplied(JobId, Result<Result<Outcome, String>, String>),
    /// Time to sample processes again (while the tab is open).
    ProcessTick,
    ProcessConn(Result<Connection, String>),
    Processes(Result<Vec<Process>, String>),
    Pause,
    Show(Show),
    Sort(SortBy),
}

impl State {
    pub fn reset(&mut self) {
        *self = State::default();
    }

    /// Whether the process list wants regular samples.
    pub fn live(&self) -> bool {
        !self.paused
    }

    pub fn on_open(&mut self, ctx: &mut Ctx) -> Task<Msg> {
        self.update(Msg::ProcessTick, ctx)
    }

    fn sample(&mut self) -> Task<Msg> {
        let Some(conn) = self.process_conn.clone() else { return Task::none() };
        self.sampling = true;
        Task::perform(processes::list(conn), Msg::Processes)
    }

    fn scan(&mut self, ctx: &mut Ctx) -> Task<Msg> {
        self.health = Health::Scanning;
        ctx.job("Health Check", Unit::Items, |conn, _| health::scan(conn), Msg::Scanned)
    }

    pub fn update(&mut self, msg: Msg, ctx: &mut Ctx) -> Task<Msg> {
        match msg {
            Msg::RunHealthCheck => {
                ctx.log("Running Health Check (read-only)…");
                return self.scan(ctx);
            }
            Msg::Scanned(id, result) => {
                ctx.jobs.finish(id);
                if ctx.session.is_none() {
                    return Task::none();
                }
                self.health = match result.and_then(|r| r) {
                    Ok(findings) => {
                        let scanned_at = chrono::Local::now().format("%H:%M").to_string();
                        Health::Done { findings, scanned_at }
                    }
                    Err(e) => {
                        ctx.log(format!("Health Check failed: {e}"));
                        Health::Failed(e)
                    }
                };
            }
            Msg::FixPressed(fix) => match fix.confirmation() {
                Some((title, question)) => return ui::confirm(title, question, move |yes| Msg::FixConfirmed(fix, yes)),
                None => return self.update(Msg::FixConfirmed(fix, true), ctx),
            },
            Msg::FixConfirmed(_, false) => {}
            Msg::FixConfirmed(fix, true) => {
                ctx.log(format!("{}…", fix.label()));
                self.fixing = true;
                return ctx.job(fix.label(), Unit::Items, move |conn, _| health::apply(conn, fix), Msg::FixApplied);
            }
            Msg::ProcessTick => {
                if self.sampling || self.paused {
                    return Task::none();
                }
                if self.process_conn.is_some() {
                    return self.sample();
                }
                let Some(session) = ctx.android() else { return Task::none() };
                self.sampling = true;
                return Task::perform(async move { session.job_connection().await }, Msg::ProcessConn);
            }
            Msg::ProcessConn(result) => {
                self.sampling = false;
                match result {
                    Ok(conn) if ctx.session.is_some() => {
                        self.process_conn = Some(conn);
                        return self.sample();
                    }
                    Ok(_) => {}
                    Err(e) => ctx.log(format!("Couldn't open a connection for the process list: {e}")),
                }
            }
            Msg::Processes(result) => {
                self.sampling = false;
                match result {
                    Ok(list) if ctx.session.is_some() => self.processes = list,
                    Ok(_) => {}
                    Err(e) => {
                        // A dropped connection gets reopened on the next tick.
                        self.process_conn = None;
                        ctx.log(format!("Couldn't list processes: {e}"));
                    }
                }
            }
            Msg::Pause => {
                self.paused = !self.paused;
                if !self.paused {
                    return self.update(Msg::ProcessTick, ctx);
                }
            }
            Msg::Show(show) => self.show = show,
            Msg::Sort(sort) => self.sort = sort,
            Msg::FixApplied(id, result) => {
                ctx.jobs.finish(id);
                self.fixing = false;
                match result.and_then(|r| r) {
                    Ok(outcome) => ctx.apply(outcome),
                    Err(e) => ctx.log(format!("Fix failed: {e}")),
                }
                // Show the effect straight away, if a scan is on screen.
                if !matches!(self.health, Health::Idle) && ctx.session.is_some() {
                    return self.scan(ctx);
                }
            }
        }
        Task::none()
    }

    pub fn view<'a>(&'a self, _v: &View<'a>) -> Element<'a, Msg> {
        let busy = self.fixing;
        let scanning = matches!(self.health, Health::Scanning);
        let header = row![
            ui::section_title("Health Check"),
            space::horizontal(),
            button(text(if scanning { "Scanning…" } else { "Run Health Check" }).size(13))
                .padding([8, 16])
                .style(style::primary)
                .on_press_maybe((!scanning && !busy).then_some(Msg::RunHealthCheck)),
        ]
        .align_y(Alignment::Center);

        let (status, color, findings): (String, _, &[Finding]) = match &self.health {
            Health::Idle => (
                "Scans memory, storage, CPU and apps. Read-only: nothing on the TV changes unless you press a fix button."
                    .into(),
                ds::TEXT_TERTIARY,
                &[],
            ),
            Health::Scanning => ("Scanning the TV (about 10 seconds)…".into(), ds::TEXT_TERTIARY, &[]),
            Health::Failed(e) => (format!("Scan failed: {e}"), ds::RED, &[]),
            Health::Done { findings, scanned_at } => {
                let problems = findings.iter().filter(|f| matches!(f.severity, Severity::Bad | Severity::Warn)).count();
                let lead = if problems > 0 { format!("{problems} thing(s) worth fixing. ") } else { "Looks healthy. ".into() };
                (format!("{lead}Scanned {scanned_at}. Nothing changes unless you press a fix button."), ds::TEXT_TERTIARY, findings)
            }
        };

        let panel = container(
            column![header, text(status).size(12).color(color)]
                .push(column(findings.iter().map(|f| finding_card(f, busy))).spacing(8))
                .spacing(10),
        )
        .padding(16)
        .width(Fill)
        .style(style::section);

        let tool = |fix: Fix, style: ButtonStyle| -> Element<'static, Msg> {
            button(text(fix.label()).size(13).center().width(Fill))
                .padding([8, 12])
                .style(style)
                .on_press_maybe((!busy).then(|| Msg::FixPressed(fix)))
                .into()
        };
        let tools_card = |title: &'static str, buttons: Vec<Element<'static, Msg>>| {
            container(column![ui::section_title(title), column(buttons).spacing(6)].spacing(10))
                .padding(16)
                .width(Fill)
                .style(style::section)
        };
        let optimizations = tools_card(
            "Optimizations",
            vec![
                tool(Fix::CompileApps, style::primary),
                tool(Fix::EnableFreezer, style::standard),
                tool(Fix::AllOptimizations, style::standard),
                text("Compiling takes several minutes; the rest of the app keeps working meanwhile.")
                    .size(11)
                    .color(ds::AMBER)
                    .into(),
            ],
        );
        let memory = tools_card(
            "Animations & Memory",
            vec![
                tool(Fix::Animations("0.5"), style::standard),
                tool(Fix::Animations("0"), style::standard),
                tool(Fix::Animations("1"), style::standard),
                tool(Fix::ClearCaches, style::standard),
                tool(Fix::KillBackground, style::danger),
            ],
        );

        ui::page(column![self.processes(busy), panel, row![optimizations, memory].spacing(8)].spacing(12))
    }

    /// Task Manager-style process list, refreshed every few seconds.
    fn processes(&self, busy: bool) -> Element<'_, Msg> {
        let shown: Vec<&Process> = {
            let mut list: Vec<&Process> = self
                .processes
                .iter()
                .filter(|p| !p.is_kernel_thread())
                .filter(|p| match self.show {
                    Show::All => true,
                    Show::Apps => p.package().is_some(),
                    Show::System => p.package().is_none(),
                })
                .collect();
            match self.sort {
                SortBy::Cpu => list.sort_by(|a, b| b.cpu.total_cmp(&a.cpu)),
                SortBy::Memory => list.sort_by(|a, b| b.memory.cmp(&a.memory)),
                SortBy::Name => list.sort_by(|a, b| a.name.cmp(&b.name)),
            }
            list
        };
        let total_cpu: f32 = self.processes.iter().map(|p| p.cpu).sum();
        let pill = |label: &'static str, active: bool, msg: Msg| -> Element<'_, Msg> {
            let caption = if active { ds::medium(label, ds::TEXT_BASE, ds::TEXT) } else { ds::label(label, ds::TEXT_BASE, ds::TEXT_SECONDARY) };
            button(caption).height(24).padding([0, 10]).style(style::pill(active)).on_press(msg).into()
        };
        let live = button(
            row![
                ds::icon_any(if self.paused { ds::Icon::Play } else { ds::Icon::Pause }, 10.0, if self.paused { ds::GREEN } else { ds::AMBER }),
                ds::label(if self.paused { "Resume" } else { "Pause" }, ds::TEXT_XS, ds::TEXT),
            ]
            .spacing(4)
            .align_y(Alignment::Center),
        )
        .padding([2, 6])
        .style(style::chip)
        .on_press(Msg::Pause);
        let header = row![
            ui::section_title("Processes"),
            ds::mono(
                if self.processes.is_empty() {
                    crate::i18n::tr("reading…").to_owned()
                } else {
                    crate::i18n::trf("{} running · {}% CPU", &[&self.processes.len(), &format!("{total_cpu:.0}")])
                },
                ds::TEXT_SM,
                ds::TEXT_TERTIARY,
            ),
            space::horizontal(),
            pill("All", self.show == Show::All, Msg::Show(Show::All)),
            pill("Apps", self.show == Show::Apps, Msg::Show(Show::Apps)),
            pill("System", self.show == Show::System, Msg::Show(Show::System)),
            live,
        ]
        .spacing(8)
        .align_y(Alignment::Center);
        let sortable = |title: &'static str, sort: SortBy, width: Length| -> Element<'_, Msg> {
            let active = self.sort == sort;
            let mut content = row![ds::medium(title, ds::TEXT_SM, ds::TEXT_SECONDARY)].spacing(4).align_y(Alignment::Center);
            if active {
                content = content.push(ds::icon_any(ds::Icon::ArrowDown, 12.0, ds::ACCENT));
            }
            button(content).padding([4, 0]).width(width).style(style::link(ds::TEXT)).on_press(Msg::Sort(sort)).into()
        };
        let columns = row![
            sortable("Name", SortBy::Name, Length::Fill),
            ds::medium("PID", ds::TEXT_SM, ds::TEXT_SECONDARY).width(64),
            ds::medium("User", ds::TEXT_SM, ds::TEXT_SECONDARY).width(90),
            sortable("CPU", SortBy::Cpu, Length::Fixed(140.0)),
            sortable("Memory", SortBy::Memory, Length::Fixed(90.0)),
            space::horizontal().width(128),
        ]
        .spacing(8)
        .align_y(Alignment::Center);
        let mut rows = column![columns, ds::hline(ds::BORDER)].spacing(0);
        for p in shown.iter().take(PROCESS_ROWS) {
            let (title, subtitle) = match p.package() {
                Some(pkg) => (health::app_label(pkg), p.name.clone()),
                None => (p.name.clone(), String::new()),
            };
            let stop: Element<'_, Msg> = match p.package() {
                Some(pkg) => button(container(text("Force Stop").size(ds::TEXT_SM).wrapping(iced::widget::text::Wrapping::None)).center_y(Length::Fill))
                    .height(24)
                    .padding([0, 8])
                    .style(style::outline)
                    .on_press_maybe((!busy).then(|| Msg::FixPressed(Fix::Stop(pkg.to_owned()))))
                    .into(),
                None => space::horizontal().width(0).into(),
            };
            let cpu_color = if p.cpu >= 25.0 { ds::RED } else if p.cpu >= 10.0 { ds::AMBER } else { ds::ACCENT };
            let mut name = column![ds::label(title, ds::TEXT_BASE, ds::TEXT)];
            if !subtitle.is_empty() {
                name = name.push(ds::mono(subtitle, ds::TEXT_XS, ds::TEXT_TERTIARY));
            }
            rows = rows.push(
                row![
                    container(name).width(Length::Fill).clip(true),
                    ds::mono(p.pid.to_string(), ds::TEXT_SM, ds::TEXT_SECONDARY).width(64),
                    ds::mono(p.user.clone(), ds::TEXT_SM, ds::TEXT_SECONDARY).width(90),
                    row![
                        progress_bar(0.0..=100.0, p.cpu.min(100.0)).length(70).girth(4).style(style::gauge(cpu_color)),
                        ds::mono(format!("{:.1}%", p.cpu), ds::TEXT_SM, ds::TEXT),
                    ]
                    .spacing(8)
                    .align_y(Alignment::Center)
                    .width(140),
                    ds::mono(fmt_bytes(p.memory), ds::TEXT_SM, ds::TEXT).width(90),
                    container(stop).width(128).align_right(128),
                ]
                .spacing(8)
                .padding([5, 0])
                .align_y(Alignment::Center),
            );
            rows = rows.push(ds::hline(ds::DIVIDER));
        }
        if shown.len() > PROCESS_ROWS {
            rows = rows.push(ds::label(format!("Top {PROCESS_ROWS} of {} shown.", shown.len()), ds::TEXT_SM, ds::TEXT_TERTIARY));
        }
        container(column![header, rows].spacing(10)).padding(16).width(Fill).style(style::section).into()
    }
}

fn fix_button(fix: &Fix, busy: bool, compact: bool) -> Element<'static, Msg> {
    let style = if matches!(fix, Fix::Uninstall(_)) { style::danger } else { style::standard };
    let mut b = button(text(fix.label()).size(12))
        .padding([5, 12])
        .style(style)
        .on_press_maybe((!busy).then(|| Msg::FixPressed(fix.clone())));
    if compact {
        b = b.width(90);
    }
    b.into()
}

fn finding_card(f: &Finding, busy: bool) -> Element<'_, Msg> {
    let color = match f.severity {
        Severity::Bad => ds::RED,
        Severity::Warn => ds::AMBER,
        Severity::Ok => ds::GREEN,
        Severity::Info => ds::ACCENT,
    };
    let header = row![text("●").size(14).color(color), text(f.title).size(14).color(ds::TEXT), space::horizontal()]
        .extend(f.fixes.iter().map(|fix| fix_button(fix, busy, false)))
        .spacing(8)
        .align_y(Alignment::Center);

    let items = f.items.iter().map(|(label, fixes)| {
        row![text(label).size(12).color(ds::TEXT).width(Fill)]
            .extend(fixes.iter().map(|fix| fix_button(fix, busy, true)))
            .spacing(8)
            .align_y(Alignment::Center)
            .into()
    });

    container(
        column![header, text(&f.detail).size(12).color(ds::TEXT_SECONDARY)]
            .push(column(items).spacing(4).padding([0, 12]))
            .spacing(6),
    )
    .padding(12)
    .width(Fill)
    .style(style::well)
    .into()
}
