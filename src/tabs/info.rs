//! Info: everything the TV reports about itself.

use iced::widget::{button, column, container, progress_bar, row, scrollable, text};
use iced::{Element, Fill, Task};

use super::{Ctx, View};
use crate::android::sysinfo::{self, Section};
use crate::theme;
use crate::ui;

#[derive(Debug, Default)]
pub struct State {
    sections: Option<Vec<Section>>,
    loading: bool,
}

#[derive(Debug, Clone)]
pub enum Msg {
    Refresh,
    Loaded(Result<Vec<Section>, String>),
    Copy,
}

impl State {
    pub fn reset(&mut self) {
        *self = State::default();
    }

    /// Load as soon as a TV connects.
    pub fn on_connect(&mut self, ctx: &mut Ctx) -> Task<Msg> {
        self.load(ctx)
    }

    fn load(&mut self, ctx: &mut Ctx) -> Task<Msg> {
        let Some(conn) = ctx.conn() else { return Task::none() };
        self.loading = true;
        Task::perform(sysinfo::collect(conn), Msg::Loaded)
    }

    pub fn update(&mut self, msg: Msg, ctx: &mut Ctx) -> Task<Msg> {
        match msg {
            Msg::Refresh => return self.load(ctx),
            Msg::Loaded(result) => {
                self.loading = false;
                match result {
                    // Ignore a late answer from a TV we've since left.
                    Ok(sections) if ctx.session.is_some() => self.sections = Some(sections),
                    Ok(_) => {}
                    Err(e) => ctx.log(format!("Could not read device info: {e}")),
                }
            }
            Msg::Copy => {
                if let Some(text) = self.sections.as_deref().map(sysinfo::to_text) {
                    ctx.log("Device info copied to the clipboard.");
                    return iced::clipboard::write(text);
                }
            }
        }
        Task::none()
    }

    pub fn view<'a>(&'a self, _v: &View<'a>) -> Element<'a, Msg> {
        let Some(sections) = &self.sections else {
            return if self.loading {
                ui::empty_state("Reading device info…", "")
            } else {
                ui::empty_state("No TV connected", "Pick a TV in the sidebar, or enter its IP address and press Connect.")
            };
        };

        let cards = sections.iter().map(|section| {
            let rows = section.fields.iter().map(|f| {
                let mut value = column![text(&f.value).size(13).color(theme::TEXT)].spacing(4);
                if let Some(used) = f.usage {
                    value = value.push(progress_bar(0.0..=1.0, used).length(320).girth(8).style(theme::usage_bar(used)));
                }
                row![
                    text(format!("{}:", f.label))
                        .size(13)
                        .color(theme::TEXT_MUTED)
                        .width(150)
                        .align_x(iced::alignment::Horizontal::Right),
                    value.width(Fill),
                ]
                .spacing(12)
                .into()
            });
            container(column![ui::section_title(section.title), column(rows).spacing(10)].spacing(10))
                .padding(16)
                .width(Fill)
                .style(theme::panel)
                .into()
        });

        let refresh = if self.loading { "Refreshing…" } else { "Refresh Info" };
        let buttons = row![
            button(text(refresh).size(13))
                .padding([8, 16])
                .style(theme::secondary_button)
                .on_press_maybe((!self.loading).then_some(Msg::Refresh)),
            button(text("Copy All").size(13)).padding([8, 16]).style(theme::secondary_button).on_press(Msg::Copy),
        ]
        .spacing(8);

        scrollable(column(cards).push(container(buttons).center_x(Fill)).spacing(10).padding(4)).into()
    }
}
