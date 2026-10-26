//! Tools: power, navigation, settings shortcuts, launching apps, typing text
//! and a shell console.

use iced::widget::{button, column, row, scrollable, text, text_input};
use iced::{Alignment, Element, Fill, Task};

use super::{Ctx, Event, View};
use crate::android::actions::{self, Action};
use crate::android::{catalog, install, keyboard};
use crate::jobs::{JobId, Unit};
use crate::theme;
use crate::ui::{self, ButtonStyle};

#[derive(Debug, Default)]
pub struct State {
    launch: String,
    text: String,
    console: String,
    busy: bool,
    /// Text waiting for ADBKeyboard to be installed.
    pending_text: Option<String>,
}

#[derive(Debug, Clone)]
pub enum Msg {
    Act(Action),
    Confirmed(Action, bool),
    Done(Action, Result<String, String>),
    ConsoleDone(JobId, Result<Result<String, String>, String>),
    LaunchInput(String),
    LaunchSubmit,
    TextInput(String),
    TextSubmit,
    TextSent(Result<String, keyboard::Error>),
    InstallKeyboard(bool),
    KeyboardInstalled(JobId, Result<Result<String, String>, String>),
    ConsoleInput(String),
    ConsoleSubmit,
}

impl State {
    pub fn reset(&mut self) {
        self.busy = false;
        self.pending_text = None;
    }

    fn send_text(&mut self, text: String, ctx: &mut Ctx) -> Task<Msg> {
        let Some(conn) = ctx.conn() else { return Task::none() };
        if keyboard::needs_keyboard(&text) {
            self.busy = true;
            // Kept until sent, so it survives installing ADBKeyboard first.
            self.pending_text = Some(text.clone());
            Task::perform(keyboard::send(conn, text), Msg::TextSent)
        } else {
            self.update(Msg::Act(Action::SendText(text)), ctx)
        }
    }

    pub fn update(&mut self, msg: Msg, ctx: &mut Ctx) -> Task<Msg> {
        match msg {
            Msg::Act(action) => match action.confirmation() {
                Some((title, question)) => return ui::confirm(title, question, move |yes| Msg::Confirmed(action, yes)),
                None => return self.update(Msg::Confirmed(action, true), ctx),
            },
            Msg::Confirmed(_, false) => {}
            Msg::Confirmed(Action::Console(cmd), true) => {
                ctx.log(format!("$ {}", actions::console_command(&cmd)));
                self.busy = true;
                // Its own connection: a slow command mustn't block the remote.
                let title = format!("Console: {}", actions::console_command(&cmd));
                return ctx.job(title, Unit::Items, move |conn, _| actions::run(conn, Action::Console(cmd)), Msg::ConsoleDone);
            }
            Msg::Confirmed(action, true) => {
                let Some(conn) = ctx.conn() else { return Task::none() };
                self.busy = true;
                return Task::perform(actions::run(conn, action.clone()), move |r| Msg::Done(action.clone(), r));
            }
            Msg::Done(action, result) => {
                self.busy = false;
                ctx.log(result.unwrap_or_else(|e| format!("Failed: {e}")));
                if action.is_reboot() {
                    ctx.events.push(Event::Disconnect);
                }
            }
            Msg::ConsoleDone(id, result) => {
                ctx.jobs.finish(id);
                self.busy = false;
                ctx.log(result.and_then(|r| r).unwrap_or_else(|e| format!("Failed: {e}")));
            }
            Msg::LaunchInput(v) => self.launch = v,
            Msg::LaunchSubmit => {
                let pkg = self.launch.trim().to_owned();
                if !pkg.is_empty() {
                    return self.update(Msg::Act(Action::LaunchApp(pkg)), ctx);
                }
            }
            Msg::TextInput(v) => self.text = v,
            Msg::TextSubmit => {
                let text = std::mem::take(&mut self.text);
                if !text.is_empty() {
                    return self.send_text(text, ctx);
                }
            }
            Msg::TextSent(result) => {
                self.busy = false;
                match result {
                    Ok(line) => {
                        self.pending_text = None;
                        ctx.log(line);
                    }
                    Err(keyboard::Error::Failed(e)) => {
                        self.pending_text = None;
                        ctx.log(format!("Sending text failed: {e}"));
                    }
                    Err(keyboard::Error::NotInstalled) => {
                        return ui::confirm(
                            "Install ADBKeyboard",
                            "Typing accents or non-Latin text needs ADBKeyboard, a small free keyboard app, on the TV.\n\n\
                             Download it from GitHub and install it now? The TV's own keyboard stays the default; \
                             ADBKeyboard is switched in only while sending text.",
                            Msg::InstallKeyboard,
                        );
                    }
                }
            }
            Msg::InstallKeyboard(false) => self.pending_text = None,
            Msg::InstallKeyboard(true) => {
                ctx.log("Installing ADBKeyboard…");
                self.busy = true;
                return ctx.job("Installing ADBKeyboard", Unit::Bytes, |conn, progress| async move {
                    catalog::fetch(catalog::ADB_KEYBOARD, Vec::new(), progress.clone()).await?;
                    let apk = catalog::download_path(&catalog::ADB_KEYBOARD, &[]).ok_or("no data folder")?;
                    install::install_with(conn, apk, &[], progress).await
                }, Msg::KeyboardInstalled);
            }
            Msg::KeyboardInstalled(id, result) => {
                ctx.jobs.finish(id);
                self.busy = false;
                match result.and_then(|r| r) {
                    Ok(_) => {
                        ctx.log("ADBKeyboard installed.");
                        if let Some(text) = self.pending_text.take() {
                            return self.send_text(text, ctx);
                        }
                    }
                    Err(e) => ctx.log(format!("Couldn't install ADBKeyboard: {e}")),
                }
            }
            Msg::ConsoleInput(v) => self.console = v,
            Msg::ConsoleSubmit => {
                let cmd = std::mem::take(&mut self.console);
                if !cmd.trim().is_empty() {
                    return self.update(Msg::Act(Action::Console(cmd)), ctx);
                }
            }
        }
        Task::none()
    }

    pub fn view<'a>(&'a self, _v: &View<'a>) -> Element<'a, Msg> {
        let busy = self.busy;
        let col = |items: Vec<Element<'static, Msg>>| column(items).spacing(6).width(Fill);
        let power = ui::card(
            "Power",
            col(vec![
                act("Wake", Action::Wake, busy),
                act("Sleep / stand-by", Action::Sleep, busy),
                act("Reboot", Action::Reboot, busy),
                act("Reboot to recovery", Action::RebootRecovery, busy),
            ])
            .into(),
        );
        let navigation = ui::card(
            "Navigation",
            col(vec![
                act("Open notifications", Action::Notifications, busy),
                act("Google search (weather)", Action::SearchWeather, busy),
                act("System updates", Action::SystemUpdates, busy),
            ])
            .into(),
        );
        let shortcuts = ui::card(
            "Open on the TV",
            col(actions::SETTINGS_SHORTCUTS.iter().map(|(label, a)| act(label, Action::OpenSettings(a), busy)).collect()).into(),
        );

        let input_row = |placeholder: &'a str, value: &'a str, on_input: fn(String) -> Msg, submit: Msg, label: &'a str| -> Element<'a, Msg> {
            row![
                text_input(placeholder, value).on_input(on_input).on_submit(submit.clone()).size(13),
                button(text(label).size(12))
                    .padding([6, 14])
                    .style(theme::secondary_button)
                    .on_press_maybe((!busy && !value.trim().is_empty()).then_some(submit)),
            ]
            .spacing(6)
            .align_y(Alignment::Center)
            .into()
        };
        let inputs = ui::card(
            "Apps, Text & Console",
            column![
                text("Launch an app by package name").size(12).color(theme::TEXT_MUTED),
                input_row("com.example.app", &self.launch, Msg::LaunchInput, Msg::LaunchSubmit, "Launch"),
                text("Type text on the TV (into the focused field). Accents and other scripts work too.")
                    .size(12)
                    .color(theme::TEXT_MUTED),
                input_row("Text to send…", &self.text, Msg::TextInput, Msg::TextSubmit, "Send"),
                text("Shell command (an 'adb shell' prefix is fine); output goes to the log").size(12).color(theme::TEXT_MUTED),
                input_row("input keyevent 3", &self.console, Msg::ConsoleInput, Msg::ConsoleSubmit, "Run"),
            ]
            .spacing(6)
            .into(),
        );
        scrollable(column![row![power, navigation, shortcuts].spacing(8), inputs].spacing(8).padding(4)).into()
    }
}

fn act(label: &str, action: Action, busy: bool) -> Element<'_, Msg> {
    let style: ButtonStyle = if action.confirmation().is_some() { theme::danger_button } else { theme::secondary_button };
    button(text(label).size(12)).padding([6, 12]).style(style).on_press_maybe((!busy).then_some(Msg::Act(action))).into()
}
