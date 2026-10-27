//! Info: everything the TV reports about itself.

use iced::widget::{column, progress_bar, row, space};
use iced::{Alignment, Element, Fill, Task};

use super::{Ctx, View};
use crate::android::sysinfo::{self, Section};
use crate::design::{self as ds, style};
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

    pub fn view<'a>(&'a self, v: &View<'a>) -> Element<'a, Msg> {
        // Samsung and LG TVs report a few facts over their remote API.
        if let Some(session) = v.session
            && let Some(facts) = session.facts()
        {
            let rows = facts.iter().map(|(label, value)| {
                row![ds::label(label.as_str(), ds::TEXT_BASE, ds::TEXT_SECONDARY).width(170), ds::label(value.as_str(), ds::TEXT_BASE, ds::TEXT)]
                    .spacing(12)
                    .into()
            });
            let header = column![
                ds::semibold("Device Overview", ds::TEXT_LG, ds::TEXT),
                ds::label(
                    crate::i18n::trf("{} · connected through the TV's own remote-control API", &[&crate::i18n::tr(session.platform().name())]),
                    ds::TEXT_BASE,
                    ds::TEXT_SECONDARY,
                ),
            ]
            .spacing(2);
            return ui::page(column![header, ui::card("Device", column(rows).spacing(10).into())].spacing(12));
        }
        let Some(sections) = &self.sections else {
            return if self.loading {
                ui::empty_state("Reading device info…", "")
            } else {
                ui::empty_state("No TV connected", "Pick a TV in the sidebar, or enter its IP address and press Connect.")
            };
        };

        let cards = sections.iter().map(|section| {
            let rows = section.fields.iter().map(|f| {
                let mut value = column![ds::label(&f.value, ds::TEXT_BASE, ds::TEXT)].spacing(6);
                if let Some(used) = f.usage {
                    value = value.push(progress_bar(0.0..=1.0, used).length(320).girth(6).style(style::usage(used)));
                }
                row![ds::label(f.label, ds::TEXT_BASE, ds::TEXT_SECONDARY).width(170), value.width(Fill)].spacing(12).into()
            });
            ui::card(section.title, column(rows).spacing(10).into())
        });

        let header = row![
            column![
                ds::semibold("Device Overview", ds::TEXT_LG, ds::TEXT),
                ds::label("Hardware, software, storage and network, read from the TV.", ds::TEXT_BASE, ds::TEXT_SECONDARY),
            ]
            .spacing(2),
            space::horizontal(),
            ds::icon_button(
                Some((ds::Icon::RefreshCw, ds::TEXT_SECONDARY)),
                if self.loading { "Refreshing…" } else { "Refresh" },
                None,
                style::standard,
                32.0,
                (!self.loading).then_some(Msg::Refresh),
            ),
            ds::icon_button(Some((ds::Icon::Copy, ds::TEXT_SECONDARY)), "Copy All", None, style::standard, 32.0, Some(Msg::Copy)),
        ]
        .spacing(8)
        .align_y(Alignment::Center);

        ui::page(column![header].extend(cards).spacing(12))
    }
}
