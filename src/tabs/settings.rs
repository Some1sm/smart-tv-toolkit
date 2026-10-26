//! Settings: TV settings with their current values.

use iced::widget::{button, column, pick_list, row, scrollable, text, text_input};
use iced::{Alignment, Element, Task};

use super::{Ctx, View};
use crate::android::actions::{self, Action, Current};
use crate::theme;
use crate::ui::{self, ButtonStyle};

#[derive(Debug, Default)]
pub struct State {
    now: Option<Current>,
    density: String,
    busy: bool,
}

#[derive(Debug, Clone)]
pub enum Msg {
    Load,
    Loaded(Result<Current, String>),
    Act(Action),
    Confirmed(Action, bool),
    Done(Result<String, String>),
    DensityInput(String),
    ApplyDensity,
}

impl State {
    pub fn reset(&mut self) {
        *self = State::default();
    }

    /// Show live values whenever the tab is opened.
    pub fn on_open(&mut self, ctx: &mut Ctx) -> Task<Msg> {
        self.load(ctx)
    }

    fn load(&mut self, ctx: &mut Ctx) -> Task<Msg> {
        let Some(conn) = ctx.conn() else { return Task::none() };
        Task::perform(actions::read_current(conn), Msg::Loaded)
    }

    pub fn update(&mut self, msg: Msg, ctx: &mut Ctx) -> Task<Msg> {
        match msg {
            Msg::Load => return self.load(ctx),
            Msg::Loaded(result) => match result {
                Ok(now) if ctx.session.is_some() => self.now = Some(now),
                Ok(_) => {}
                Err(e) => ctx.log(format!("Could not read settings: {e}")),
            },
            Msg::Act(action) => match action.confirmation() {
                Some((title, question)) => return ui::confirm(title, question, move |yes| Msg::Confirmed(action, yes)),
                None => return self.update(Msg::Confirmed(action, true), ctx),
            },
            Msg::Confirmed(_, false) => {}
            Msg::Confirmed(action, true) => {
                let Some(conn) = ctx.conn() else { return Task::none() };
                self.busy = true;
                return Task::perform(actions::run(conn, action), Msg::Done);
            }
            Msg::Done(result) => {
                self.busy = false;
                ctx.log(result.unwrap_or_else(|e| format!("Failed: {e}")));
                return self.load(ctx);
            }
            Msg::DensityInput(v) => self.density = v,
            Msg::ApplyDensity => match self.density.trim().parse::<u32>() {
                Ok(d) if (120..=640).contains(&d) => return self.update(Msg::Act(Action::Density(d)), ctx),
                _ => ctx.log("Enter a density between 120 and 640 (TVs usually use 213, 240 or 320)."),
            },
        }
        Task::none()
    }

    pub fn view<'a>(&'a self, _v: &View<'a>) -> Element<'a, Msg> {
        let busy = self.busy;
        let now = |f: fn(&Current) -> &String| self.now.as_ref().map(|c| f(c).as_str());
        let rotation_labels: Vec<&'static str> = actions::ROTATIONS.iter().map(|(l, _)| *l).collect();
        let timeout_labels: Vec<&'static str> = actions::TIMEOUTS.iter().map(|(l, _)| *l).collect();
        let pick_rotation = pick_list(rotation_labels, None::<&'static str>, |label| {
            let v = actions::ROTATIONS.iter().find(|(l, _)| *l == label).map(|(_, v)| *v).unwrap_or("0");
            Msg::Act(Action::Rotation(v))
        })
        .placeholder("Change…")
        .text_size(12)
        .width(200);
        let pick_timeout = pick_list(timeout_labels, None::<&'static str>, |label| {
            let v = actions::TIMEOUTS.iter().find(|(l, _)| *l == label).map(|(_, v)| *v).unwrap_or("600000");
            Msg::Act(Action::ScreenTimeout(v))
        })
        .placeholder("Change…")
        .text_size(12)
        .width(200);
        let density_controls = row![
            text_input("e.g. 320", &self.density).on_input(Msg::DensityInput).on_submit(Msg::ApplyDensity).size(12).width(90),
            button(text("Apply").size(12))
                .padding([6, 12])
                .style(theme::secondary_button)
                .on_press_maybe((!busy).then_some(Msg::ApplyDensity)),
            act("Reset", Action::ResetDensity, busy),
        ]
        .spacing(6)
        .align_y(Alignment::Center);
        let pair = |on: Action, off: Action, labels: (&'static str, &'static str)| -> Element<'a, Msg> {
            row![act(labels.0, on, busy), act(labels.1, off, busy)].spacing(6).into()
        };
        let onoff = ("Turn on", "Turn off");

        let display = ui::card(
            "Display",
            column![
                setting_row("Rotation", now(|c| &c.rotation), pick_rotation.into()),
                setting_row("Screen timeout", now(|c| &c.timeout), pick_timeout.into()),
                setting_row("Density", now(|c| &c.density), density_controls.into()),
            ]
            .spacing(10)
            .into(),
        );
        let system = ui::card(
            "System",
            column![
                setting_row("Developer options", now(|c| &c.developer_options),
                            pair(Action::DeveloperOptions(true), Action::DeveloperOptions(false), ("Show", "Hide"))),
                setting_row("Location", now(|c| &c.location), pair(Action::Location(true), Action::Location(false), onoff)),
                setting_row("Play Protect", now(|c| &c.play_protect), pair(Action::PlayProtect(true), Action::PlayProtect(false), onoff)),
                setting_row("Ambient display", now(|c| &c.ambient), pair(Action::AmbientDisplay(true), Action::AmbientDisplay(false), onoff)),
                setting_row("Time server", now(|c| &c.ntp), act("Use pool.ntp.org", Action::RepairNtp, busy)),
            ]
            .spacing(10)
            .into(),
        );
        let refresh = button(text("Refresh").size(12)).padding([6, 12]).style(theme::tab_button(false)).on_press(Msg::Load);
        scrollable(column![display, system, refresh].spacing(8).padding(4)).into()
    }
}

fn act(label: &str, action: Action, busy: bool) -> Element<'_, Msg> {
    let style: ButtonStyle = if action.confirmation().is_some() { theme::danger_button } else { theme::secondary_button };
    button(text(label).size(12)).padding([6, 12]).style(style).on_press_maybe((!busy).then_some(Msg::Act(action))).into()
}

fn setting_row<'a>(label: &'a str, now: Option<&'a str>, controls: Element<'a, Msg>) -> Element<'a, Msg> {
    row![
        text(label).size(13).color(theme::TEXT).width(150),
        text(format!("Now: {}", now.unwrap_or("…"))).size(12).color(theme::TEXT_MUTED).width(170),
        controls,
    ]
    .spacing(12)
    .align_y(Alignment::Center)
    .into()
}
