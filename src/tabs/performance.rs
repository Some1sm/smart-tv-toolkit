//! Performance: the Health Check and the optimisation tools.

use iced::widget::{button, column, container, row, scrollable, space, text};
use iced::{Alignment, Element, Fill, Task};

use super::{Ctx, View};
use crate::android::health::{self, Finding, Fix, Severity};
use crate::jobs::{JobId, Unit};
use crate::theme;
use crate::ui::{self, ButtonStyle};

#[derive(Debug, Default)]
enum Health {
    #[default]
    Idle,
    Scanning,
    Failed(String),
    Done { findings: Vec<Finding>, scanned_at: String },
}

#[derive(Debug, Default)]
pub struct State {
    health: Health,
    /// A fix is running (its buttons are disabled meanwhile).
    fixing: bool,
}

#[derive(Debug, Clone)]
pub enum Msg {
    RunHealthCheck,
    Scanned(JobId, Result<Result<Vec<Finding>, String>, String>),
    FixPressed(Fix),
    FixConfirmed(Fix, bool),
    FixApplied(JobId, Result<Result<String, String>, String>),
}

impl State {
    pub fn reset(&mut self) {
        *self = State::default();
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
            Msg::FixApplied(id, result) => {
                ctx.jobs.finish(id);
                self.fixing = false;
                match result.and_then(|r| r) {
                    Ok(line) => ctx.log(line),
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
                .style(theme::primary_button)
                .on_press_maybe((!scanning && !busy).then_some(Msg::RunHealthCheck)),
        ]
        .align_y(Alignment::Center);

        let (status, color, findings): (String, _, &[Finding]) = match &self.health {
            Health::Idle => (
                "Scans memory, storage, CPU and apps. Read-only: nothing on the TV changes unless you press a fix button."
                    .into(),
                theme::TEXT_DISABLED,
                &[],
            ),
            Health::Scanning => ("Scanning the TV (about 10 seconds)…".into(), theme::TEXT_DISABLED, &[]),
            Health::Failed(e) => (format!("Scan failed: {e}"), theme::DANGER, &[]),
            Health::Done { findings, scanned_at } => {
                let problems = findings.iter().filter(|f| matches!(f.severity, Severity::Bad | Severity::Warn)).count();
                let lead = if problems > 0 { format!("{problems} thing(s) worth fixing. ") } else { "Looks healthy. ".into() };
                (format!("{lead}Scanned {scanned_at}. Nothing changes unless you press a fix button."), theme::TEXT_DISABLED, findings)
            }
        };

        let panel = container(
            column![header, text(status).size(12).color(color)]
                .push(column(findings.iter().map(|f| finding_card(f, busy))).spacing(8))
                .spacing(10),
        )
        .padding(16)
        .width(Fill)
        .style(theme::panel);

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
                .style(theme::panel)
        };
        let optimizations = tools_card(
            "Optimizations",
            vec![
                tool(Fix::CompileApps, theme::primary_button),
                tool(Fix::EnableFreezer, theme::secondary_button),
                tool(Fix::AllOptimizations, theme::secondary_button),
                text("Compiling takes several minutes; the rest of the app keeps working meanwhile.")
                    .size(11)
                    .color(theme::WARNING)
                    .into(),
            ],
        );
        let memory = tools_card(
            "Animations & Memory",
            vec![
                tool(Fix::Animations("0.5"), theme::secondary_button),
                tool(Fix::Animations("0"), theme::secondary_button),
                tool(Fix::Animations("1"), theme::secondary_button),
                tool(Fix::ClearCaches, theme::secondary_button),
                tool(Fix::KillBackground, theme::danger_button),
            ],
        );

        scrollable(column![panel, row![optimizations, memory].spacing(8)].spacing(8).padding(4)).into()
    }
}

fn fix_button(fix: &Fix, busy: bool, compact: bool) -> Element<'static, Msg> {
    let style = if matches!(fix, Fix::Uninstall(_)) { theme::danger_button } else { theme::secondary_button };
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
        Severity::Bad => theme::DANGER,
        Severity::Warn => theme::WARNING,
        Severity::Ok => theme::SUCCESS,
        Severity::Info => theme::PRIMARY,
    };
    let header = row![text("●").size(14).color(color), text(f.title).size(14).color(theme::TEXT), space::horizontal()]
        .extend(f.fixes.iter().map(|fix| fix_button(fix, busy, false)))
        .spacing(8)
        .align_y(Alignment::Center);

    let items = f.items.iter().map(|(label, fixes)| {
        row![text(label).size(12).color(theme::TEXT).width(Fill)]
            .extend(fixes.iter().map(|fix| fix_button(fix, busy, true)))
            .spacing(8)
            .align_y(Alignment::Center)
            .into()
    });

    container(
        column![header, text(&f.detail).size(12).color(theme::TEXT_MUTED)]
            .push(column(items).spacing(4).padding([0, 12]))
            .spacing(6),
    )
    .padding(12)
    .width(Fill)
    .style(theme::card)
    .into()
}
