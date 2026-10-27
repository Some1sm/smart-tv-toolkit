//! Tools: power, navigation, settings shortcuts, launching apps, typing text
//! and a shell console.

use iced::widget::{button, checkbox, column, container, row, scrollable, space, text_input};
use iced::{Alignment, Element, Fill, Length, Task};

use super::{Ctx, Event, View};
use crate::ui::text;
use crate::android::actions::{self, Action};
use crate::android::changes::Outcome;
use crate::android::logs::{self, Level, Source};
use crate::android::{catalog, install, keyboard};
use crate::jobs::{JobId, Unit};
use crate::design::{self as ds, style};
use crate::ui::{self, ButtonStyle};

#[derive(Debug, Default)]
pub struct State {
    launch: String,
    text: String,
    console: String,
    busy: bool,
    /// Text waiting for ADBKeyboard to be installed.
    pending_text: Option<String>,
    log_package: String,
    log_level: Level,
    log_source: Option<Source>,
    log_output: Option<String>,
    log_live: bool,
    log_reading: bool,
}

#[derive(Debug, Clone)]
pub enum Msg {
    Act(Action),
    Confirmed(Action, bool),
    Done(Action, Result<Outcome, String>),
    ConsoleDone(JobId, Result<Result<Outcome, String>, String>),
    LaunchInput(String),
    LaunchSubmit,
    TextInput(String),
    TextSubmit,
    TextSent(Result<String, keyboard::Error>),
    InstallKeyboard(bool),
    KeyboardInstalled(JobId, Result<Result<String, String>, String>),
    ConsoleInput(String),
    ConsoleSubmit,
    LogPackage(String),
    LogLevel(Level),
    LogRead(Source),
    LogLoaded(Result<String, String>),
    LogLive(bool),
    /// Re-read the shown log (auto-refresh).
    LogTick,
    LogCopy,
}

impl State {
    pub fn reset(&mut self) {
        self.busy = false;
        self.pending_text = None;
        self.log_output = None;
        self.log_source = None;
        self.log_live = false;
        self.log_reading = false;
    }

    /// Whether the shown log should refresh by itself.
    pub fn log_live(&self) -> bool {
        self.log_live && self.log_source.is_some()
    }

    fn read_log(&mut self, ctx: &mut Ctx) -> Task<Msg> {
        let (Some(conn), Some(source)) = (ctx.conn(), self.log_source) else { return Task::none() };
        if self.log_reading {
            return Task::none();
        }
        self.log_reading = true;
        Task::perform(logs::read(conn, source, self.log_package.trim().to_owned(), self.log_level), Msg::LogLoaded)
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
                match result {
                    Ok(outcome) => ctx.apply(outcome),
                    Err(e) => ctx.log(format!("Failed: {e}")),
                }
                if action.is_reboot() {
                    ctx.events.push(Event::Disconnect);
                }
            }
            Msg::ConsoleDone(id, result) => {
                ctx.jobs.finish(id);
                self.busy = false;
                match result.and_then(|r| r) {
                    Ok(outcome) => ctx.apply(outcome),
                    Err(e) => ctx.log(format!("Failed: {e}")),
                }
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
            Msg::LogPackage(p) => self.log_package = p,
            Msg::LogLevel(level) => {
                self.log_level = level;
                return self.read_log(ctx);
            }
            Msg::LogRead(source) => {
                self.log_source = Some(source);
                return self.read_log(ctx);
            }
            Msg::LogLoaded(result) => {
                self.log_reading = false;
                if ctx.session.is_some() {
                    self.log_output = Some(result.unwrap_or_else(|e| format!("Couldn't read the log: {e}")));
                }
            }
            Msg::LogLive(on) => {
                self.log_live = on;
                return self.read_log(ctx);
            }
            Msg::LogTick => return self.read_log(ctx),
            Msg::LogCopy => {
                if let Some(text) = &self.log_output {
                    ctx.log("Log copied to the clipboard.");
                    return iced::clipboard::write(text.clone());
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
                text_input(placeholder, value).on_input(on_input).on_submit(submit.clone()).size(ds::TEXT_BASE).padding([5, 8]).style(style::input(ds::INSET)),
                button(text(label).size(12))
                    .padding([6, 14])
                    .style(style::standard)
                    .on_press_maybe((!busy && !value.trim().is_empty()).then_some(submit)),
            ]
            .spacing(6)
            .align_y(Alignment::Center)
            .into()
        };
        let inputs = ui::card(
            "Apps, Text & Console",
            column![
                text("Launch an app by package name").size(12).color(ds::TEXT_SECONDARY),
                input_row("com.example.app", &self.launch, Msg::LaunchInput, Msg::LaunchSubmit, "Launch"),
                text("Type text on the TV (into the focused field). Accents and other scripts work too.")
                    .size(12)
                    .color(ds::TEXT_SECONDARY),
                input_row("Text to send…", &self.text, Msg::TextInput, Msg::TextSubmit, "Send"),
                text("Shell command (an 'adb shell' prefix is fine); output goes to the log").size(12).color(ds::TEXT_SECONDARY),
                input_row("input keyevent 3", &self.console, Msg::ConsoleInput, Msg::ConsoleSubmit, "Run"),
            ]
            .spacing(6)
            .into(),
        );
        ui::page(column![row![power, navigation, shortcuts].spacing(8), inputs, self.logs()].spacing(12))
    }

    /// App logs and crashes from `logcat` (read-only).
    fn logs(&self) -> Element<'_, Msg> {
        let source_button = |caption: &'static str, source: Source| {
            button(text(caption).size(ds::TEXT_SM))
                .padding([4, 12])
                .style(style::pill(self.log_source == Some(source)))
                .on_press(Msg::LogRead(source))
        };
        let levels = row(Level::ALL.iter().map(|&l| {
            button(text(l.label()).size(ds::TEXT_SM)).padding([4, 10]).style(style::pill(self.log_level == l)).on_press(Msg::LogLevel(l)).into()
        }))
        .spacing(4);
        let controls = row![
            ds::input("Package, e.g. org.smarttube.beta", &self.log_package, true, ds::INSET)
                .on_input(Msg::LogPackage)
                .on_submit(Msg::LogRead(Source::App))
                .width(280),
            source_button("App Log", Source::App),
            source_button("Crashes", Source::Crashes),
            source_button("Whole TV", Source::System),
            space::horizontal(),
            levels,
        ]
        .spacing(6)
        .align_y(Alignment::Center);
        let tools = row![
            checkbox(self.log_live).label("Refresh every 3 s").on_toggle(Msg::LogLive).size(14).text_size(ds::TEXT_SM).style(style::checkbox),
            space::horizontal(),
            ui::small_button("Copy", style::standard, self.log_output.is_some().then_some(Msg::LogCopy)),
        ]
        .align_y(Alignment::Center);
        let output: Element<'_, Msg> = match &self.log_output {
            Some(text) => container(
                scrollable(container(ds::mono(text.as_str(), ds::TEXT_SM, ds::TERMINAL_TEXT)).padding(10).width(Fill))
                    .anchor_bottom()
                    .direction(ds::thin_scrollbar())
                    .style(style::scrollbar)
                    .height(Length::Fixed(320.0)),
            )
            .style(style::fill(ds::TERMINAL))
            .into(),
            None => ds::label("Pick a log above. Reading logs changes nothing on the TV.", ds::TEXT_SM, ds::TEXT_TERTIARY).into(),
        };
        ui::card("Logs", column![controls, tools, output].spacing(8).into())
    }
}

fn act(label: &str, action: Action, busy: bool) -> Element<'_, Msg> {
    let style: ButtonStyle = if action.confirmation().is_some() { style::danger } else { style::standard };
    button(text(label).size(12)).padding([6, 12]).style(style).on_press_maybe((!busy).then_some(Msg::Act(action))).into()
}
