//! Widgets and helpers shared by the tabs, built on [`crate::design`].

use std::future::Future;

use iced::widget::{button, column, container, scrollable};
use iced::{Alignment, Element, Fill, Task, Theme};

use crate::design::{self as ds, style};
use crate::i18n::{self, Text};

/// iced's `text`, translating fixed English (see [`crate::i18n`]).
pub fn text<'a>(content: impl Text<'a>) -> iced::widget::Text<'a> {
    iced::widget::text(content.localized())
}

pub type ButtonStyle = fn(&Theme, button::Status) -> button::Style;

pub fn empty_state<'a, M: 'a>(title: &str, subtitle: &str) -> Element<'a, M> {
    container(
        column![
            ds::semibold(i18n::tr(title).to_owned(), ds::TEXT_LG, ds::TEXT),
            ds::label(i18n::tr(subtitle).to_owned(), ds::TEXT_BASE, ds::TEXT_SECONDARY),
        ]
        .spacing(6)
        .align_x(Alignment::Center),
    )
    .center(Fill)
    .into()
}

pub fn section_title<'a, M: 'a>(title: &'a str) -> Element<'a, M> {
    ds::overline(title).into()
}

/// A titled section card.
pub fn card<'a, M: 'a>(title: &'a str, body: Element<'a, M>) -> Element<'a, M> {
    container(column![section_title(title), body].spacing(12))
        .padding(16)
        .width(Fill)
        .style(style::section)
        .into()
}

/// A 28px action button.
pub fn small_button<'a, M: Clone + 'a>(label: impl Text<'a>, style: ButtonStyle, msg: Option<M>) -> Element<'a, M> {
    button(container(text(label).size(ds::TEXT_BASE)).center_y(Fill))
        .height(ds::CONTROL_HEIGHT)
        .padding([0, 12])
        .style(style)
        .on_press_maybe(msg)
        .into()
}

/// A tab's scrolling page: 16px gutters and the thin scrollbar.
pub fn page<'a, M: 'a>(content: impl Into<Element<'a, M>>) -> Element<'a, M> {
    scrollable(container(content).padding(16).width(Fill))
        .direction(ds::thin_scrollbar())
        .style(style::scrollbar)
        .height(Fill)
        .into()
}

/// Ask a yes/no question in a native dialog.
pub fn confirm<M: Send + 'static>(
    title: impl Into<String>,
    question: impl Into<String>,
    on_answer: impl FnOnce(bool) -> M + Send + 'static,
) -> Task<M> {
    let dialog = rfd::AsyncMessageDialog::new()
        .set_level(rfd::MessageLevel::Warning)
        .set_title(i18n::tr_owned(title.into()))
        .set_description(i18n::tr_owned(question.into()))
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
