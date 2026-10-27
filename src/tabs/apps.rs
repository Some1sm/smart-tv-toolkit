//! Apps (Samsung and LG): the TV's installed apps, to open or close. Android
//! TVs use the Packages tab instead.

use iced::widget::{button, column, container, row, space};
use iced::{Alignment, Element, Length, Padding, Task};

use super::{Ctx, View};
use crate::design::{self as ds, style};
use crate::platform::Session;
use crate::{lg, samsung, ui};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct App {
    pub id: String,
    pub name: String,
    /// Samsung's launch type (2 web app, 4 native); unused on LG.
    pub kind: u64,
}

#[derive(Debug, Default)]
pub struct State {
    apps: Vec<App>,
    loading: bool,
    filter: String,
}

#[derive(Debug, Clone)]
pub enum Msg {
    Load,
    Loaded(Result<Vec<App>, String>),
    Filter(String),
    Launch(App),
    Close(App),
    Done(String, Result<(), String>),
}

impl State {
    pub fn reset(&mut self) {
        *self = State::default();
    }

    pub fn on_open(&mut self, ctx: &mut Ctx) -> Task<Msg> {
        if self.apps.is_empty() && !self.loading { self.load(ctx) } else { Task::none() }
    }

    fn load(&mut self, ctx: &mut Ctx) -> Task<Msg> {
        let task: Option<Task<Msg>> = match ctx.session {
            Some(Session::Samsung(s)) => Some(Task::perform(samsung::apps(s.clone()), |r| {
                Msg::Loaded(r.map(|list| list.into_iter().map(|a| App { id: a.id, name: a.name, kind: a.app_type }).collect()))
            })),
            Some(Session::Lg(s)) => Some(Task::perform(lg::apps(s.clone()), |r| {
                Msg::Loaded(r.map(|list| list.into_iter().map(|a| App { id: a.id, name: a.name, kind: 0 }).collect()))
            })),
            _ => None,
        };
        match task {
            Some(t) => {
                self.loading = true;
                t
            }
            None => Task::none(),
        }
    }

    pub fn update(&mut self, msg: Msg, ctx: &mut Ctx) -> Task<Msg> {
        match msg {
            Msg::Load => return self.load(ctx),
            Msg::Loaded(result) => {
                self.loading = false;
                match result {
                    Ok(apps) if ctx.session.is_some() => {
                        ctx.log(format!("Listed {} app(s).", apps.len()));
                        self.apps = apps;
                    }
                    Ok(_) => {}
                    Err(e) => ctx.log(format!("Could not list apps: {e}")),
                }
            }
            Msg::Filter(f) => self.filter = f,
            Msg::Launch(app) => {
                let what = format!("Opened {}.", app.name);
                return match ctx.session {
                    Some(Session::Samsung(s)) => {
                        let sent = samsung::launch(s, &samsung::App { id: app.id, name: app.name, app_type: app.kind });
                        Task::done(Msg::Done(what, sent))
                    }
                    Some(Session::Lg(s)) => Task::perform(lg::launch(s.clone(), app.id), move |r| Msg::Done(what.clone(), r)),
                    _ => Task::none(),
                };
            }
            Msg::Close(app) => {
                let what = format!("Closed {}.", app.name);
                return match ctx.session {
                    Some(Session::Samsung(s)) => Task::perform(samsung::close(s.clone(), app.id), move |r| Msg::Done(what.clone(), r)),
                    Some(Session::Lg(s)) => Task::perform(lg::close(s.clone(), app.id), move |r| Msg::Done(what.clone(), r)),
                    _ => Task::none(),
                };
            }
            Msg::Done(what, result) => ctx.log(match result {
                Ok(()) => what,
                Err(e) => format!("Failed: {e}"),
            }),
        }
        Task::none()
    }

    pub fn view<'a>(&'a self, _v: &View<'a>) -> Element<'a, Msg> {
        let q = self.filter.trim().to_lowercase();
        let shown: Vec<&App> =
            self.apps.iter().filter(|a| q.is_empty() || a.name.to_lowercase().contains(&q) || a.id.to_lowercase().contains(&q)).collect();
        let search = container(
            row![
                ds::icon_any(ds::Icon::Search, 14.0, ds::TEXT_TERTIARY),
                ds::input("Filter apps…", &self.filter, false, ds::INSET).on_input(Msg::Filter).style(style::bare_input).padding(Padding::from([5, 0])),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        )
        .padding(Padding::from([0, 10]))
        .width(240)
        .height(ds::CONTROL_HEIGHT)
        .style(style::panel(ds::INSET, ds::BORDER, ds::RADIUS));
        let header = row![
            column![
                ds::semibold("Apps", ds::TEXT_LG, ds::TEXT),
                ds::label(crate::i18n::trf("{} installed on the TV", &[&self.apps.len()]), ds::TEXT_BASE, ds::TEXT_SECONDARY),
            ]
            .spacing(2),
            space::horizontal(),
            ds::square_button(ds::Icon::RefreshCw, ds::TEXT_SECONDARY, ds::CONTROL_HEIGHT, style::ghost, (!self.loading).then_some(Msg::Load)),
            search,
        ]
        .spacing(8)
        .align_y(Alignment::Center);
        if self.apps.is_empty() {
            let title = if self.loading { "Reading apps…" } else { "No apps listed" };
            return column![container(header).padding(16), ui::empty_state(title, "")].into();
        }
        let mut grid = column![].spacing(8);
        for chunk in shown.chunks(3) {
            let mut line = row![].spacing(8);
            for app in chunk {
                line = line.push(
                    container(
                        row![
                            ds::avatar(&app.name, 28.0, ds::ACCENT),
                            column![
                                ds::medium(app.name.clone(), ds::TEXT_BASE, ds::TEXT),
                                ds::mono(app.id.clone(), ds::TEXT_XS, ds::TEXT_TERTIARY),
                            ]
                            .spacing(2)
                            .width(Length::Fill),
                            ui::small_button("Open", style::primary, Some(Msg::Launch((*app).clone()))),
                            button(container(crate::ui::text("Close").size(ds::TEXT_SM)).center_y(Length::Fill))
                                .height(ds::CONTROL_HEIGHT)
                                .padding([0, 10])
                                .style(style::outline)
                                .on_press(Msg::Close((*app).clone())),
                        ]
                        .spacing(10)
                        .align_y(Alignment::Center),
                    )
                    .padding([10, 12])
                    .width(Length::Fill)
                    .style(style::well),
                );
            }
            for _ in chunk.len()..3 {
                line = line.push(space::horizontal());
            }
            grid = grid.push(line);
        }
        ui::page(column![header, grid].spacing(12))
    }
}
