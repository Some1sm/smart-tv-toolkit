//! Widgets and helpers shared by the tabs.

use std::future::Future;

use iced::widget::{button, column, container, text};
use iced::{Alignment, Element, Fill, Task, Theme};

use crate::theme;

pub type ButtonStyle = fn(&Theme, button::Status) -> button::Style;

pub fn empty_state<'a, M: 'a>(title: &str, subtitle: &str) -> Element<'a, M> {
    container(
        column![
            text(title.to_owned()).size(16).color(theme::TEXT),
            text(subtitle.to_owned()).size(12).color(theme::TEXT_MUTED),
        ]
        .spacing(6)
        .align_x(Alignment::Center),
    )
    .center(Fill)
    .into()
}

pub fn section_title<'a, M: 'a>(title: &'a str) -> Element<'a, M> {
    text(title).size(12).color(theme::TEXT_MUTED).into()
}

/// A titled panel.
pub fn card<'a, M: 'a>(title: &'a str, body: Element<'a, M>) -> Element<'a, M> {
    container(column![section_title(title), body].spacing(10))
        .padding(16)
        .width(Fill)
        .style(theme::panel)
        .into()
}

pub fn small_button<'a, M: Clone + 'a>(label: impl text::IntoFragment<'a>, style: ButtonStyle, msg: Option<M>) -> Element<'a, M> {
    button(text(label).size(12)).padding([5, 12]).style(style).on_press_maybe(msg).into()
}

/// Ask a yes/no question in a native dialog.
pub fn confirm<M: Send + 'static>(
    title: impl Into<String>,
    question: impl Into<String>,
    on_answer: impl FnOnce(bool) -> M + Send + 'static,
) -> Task<M> {
    let dialog = rfd::AsyncMessageDialog::new()
        .set_level(rfd::MessageLevel::Warning)
        .set_title(title.into())
        .set_description(question.into())
        .set_buttons(rfd::MessageButtons::YesNo);
    Task::perform(dialog.show(), move |answer| on_answer(answer == rfd::MessageDialogResult::Yes))
}

/// Pick a folder; `None` if cancelled.
pub fn pick_folder(title: impl Into<String>) -> impl Future<Output = Option<std::path::PathBuf>> {
    let dialog = rfd::AsyncFileDialog::new().set_title(title.into());
    async move { dialog.pick_folder().await.map(|d| d.path().to_path_buf()) }
}

/// Pick where to save a file; `None` if cancelled.
pub fn save_file(title: impl Into<String>, name: String, filter: (&'static str, &'static [&'static str]))
-> impl Future<Output = Option<std::path::PathBuf>> {
    let dialog = rfd::AsyncFileDialog::new().set_title(title.into()).set_file_name(name).add_filter(filter.0, filter.1);
    async move { dialog.save_file().await.map(|f| f.path().to_path_buf()) }
}

/// `prefix-YYYYmmdd-HHMMSS.ext`
pub fn timestamped(prefix: &str, ext: &str) -> String {
    format!("{prefix}-{}.{ext}", chrono::Local::now().format("%Y%m%d-%H%M%S"))
}
