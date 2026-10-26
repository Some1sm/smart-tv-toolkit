//! Screenshot: capture the TV's screen and save it.

use std::path::PathBuf;

use iced::widget::{button, column, container, image, row, text};
use iced::{Alignment, Element, Fill, Task};

use super::{Ctx, View};
use crate::android::{remote, sysinfo};
use crate::theme;
use crate::ui;

#[derive(Debug, Default)]
pub struct State {
    /// Decoded handle for display, PNG bytes for saving.
    shot: Option<(image::Handle, Vec<u8>)>,
    status: String,
    capturing: bool,
}

#[derive(Debug, Clone)]
pub enum Msg {
    Take,
    Taken(Result<Vec<u8>, String>),
    Save,
    SaveTo(Option<PathBuf>),
}

impl State {
    pub fn reset(&mut self) {
        self.capturing = false;
    }

    pub fn update(&mut self, msg: Msg, ctx: &mut Ctx) -> Task<Msg> {
        match msg {
            Msg::Take => {
                let Some(conn) = ctx.conn() else { return Task::none() };
                self.capturing = true;
                self.status = "Capturing…".into();
                return Task::perform(remote::screenshot(conn), Msg::Taken);
            }
            Msg::Taken(result) => {
                self.capturing = false;
                match result {
                    Ok(png) => {
                        let time = chrono::Local::now().format("%H:%M:%S");
                        self.status = format!("Captured at {time} ({}).", sysinfo::fmt_bytes(png.len() as u64));
                        self.shot = Some((image::Handle::from_bytes(png.clone()), png));
                    }
                    Err(e) => {
                        self.status = "Capture failed; see the log.".into();
                        ctx.log(format!("Screenshot failed: {e}"));
                    }
                }
            }
            Msg::Save => {
                let save = ui::save_file("Save screenshot", ui::timestamped("screenshot", "png"), ("PNG image", &["png"]));
                return Task::perform(save, Msg::SaveTo);
            }
            Msg::SaveTo(None) => {}
            Msg::SaveTo(Some(path)) => {
                if let Some((_, png)) = &self.shot {
                    ctx.log(match std::fs::write(&path, png) {
                        Ok(()) => format!("Saved {}", path.display()),
                        Err(e) => format!("Could not save {}: {e}", path.display()),
                    });
                }
            }
        }
        Task::none()
    }

    pub fn view<'a>(&'a self, _v: &View<'a>) -> Element<'a, Msg> {
        let buttons = row![
            button(text(if self.capturing { "Capturing…" } else { "Take Screenshot" }).size(13))
                .padding([8, 16])
                .style(theme::primary_button)
                .on_press_maybe((!self.capturing).then_some(Msg::Take)),
            button(text("Save As…").size(13))
                .padding([8, 16])
                .style(theme::secondary_button)
                .on_press_maybe(self.shot.is_some().then_some(Msg::Save)),
            text(&self.status).size(12).color(theme::TEXT_MUTED),
        ]
        .spacing(8)
        .align_y(Alignment::Center);

        let view: Element<'a, Msg> = match &self.shot {
            Some((handle, _)) => image(handle.clone()).content_fit(iced::ContentFit::Contain).width(Fill).height(Fill).into(),
            None => ui::empty_state("No screenshot yet", "Press Take Screenshot to capture what's on the TV."),
        };
        column![buttons, container(view).padding(8).width(Fill).height(Fill).style(theme::panel)]
            .spacing(8)
            .padding(4)
            .into()
    }
}
