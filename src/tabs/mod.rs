//! The tabs. Each module owns its state, messages, `update` and `view`; the
//! app shell routes messages to it and hands it a [`Ctx`] for the things
//! shared between tabs (the connected TV, the log, background jobs, prefs).

use std::future::Future;

use iced::Task;

use crate::android::changes::Outcome;
use crate::android::{self, Connection};
use crate::design::Icon;
use crate::jobs::{JobId, Jobs, Progress, Unit};
use crate::journal::Journal;
use crate::log::Log;
use crate::platform::Session;
use crate::prefs::Prefs;

pub mod apps;
pub mod history;
pub mod info;
pub mod install;
pub mod media;
pub mod optimize;
pub mod packages;
pub mod performance;
pub mod remote;
pub mod screenshot;
pub mod settings;
pub mod tools;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Info,
    Packages,
    Apps,
    Performance,
    Optimize,
    Install,
    Settings,
    Tools,
    Media,
    Remote,
    Screenshot,
    History,
}

impl Tab {
    pub const ALL: [Tab; 12] = [
        Tab::Info,
        Tab::Packages,
        Tab::Apps,
        Tab::Performance,
        Tab::Optimize,
        Tab::Install,
        Tab::Settings,
        Tab::Tools,
        Tab::Media,
        Tab::Remote,
        Tab::Screenshot,
        Tab::History,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Tab::Info => "Info",
            Tab::Packages => "Packages",
            Tab::Apps => "Apps",
            Tab::Optimize => "Optimize",
            Tab::Performance => "Performance",
            Tab::Install => "Install",
            Tab::Settings => "Settings",
            Tab::Tools => "Tools",
            Tab::Media => "Media",
            Tab::Remote => "Remote",
            Tab::Screenshot => "Screenshot",
            Tab::History => "History",
        }
    }

    /// The tab bar's caption (`name` stays the stable id saved in prefs).
    pub fn title(self) -> &'static str {
        match self {
            Tab::Info => "Overview",
            Tab::Install => "Install APK",
            other => other.name(),
        }
    }

    pub fn icon(self) -> Icon {
        match self {
            Tab::Info => Icon::Info,
            Tab::Packages => Icon::Package,
            Tab::Apps => Icon::LayoutGrid,
            Tab::Optimize => Icon::Rocket,
            Tab::Performance => Icon::Gauge,
            Tab::Install => Icon::FileUp,
            Tab::Settings => Icon::SlidersHorizontal,
            Tab::Tools => Icon::Wrench,
            Tab::Media => Icon::SquarePlay,
            Tab::Remote => Icon::Tv,
            Tab::Screenshot => Icon::Camera,
            Tab::History => Icon::FileClock,
        }
    }

    pub fn from_name(name: &str) -> Option<Tab> {
        Tab::ALL.into_iter().find(|t| t.name() == name)
    }
}

/// Something a tab asks the app shell to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// The TV is going away (e.g. rebooting): drop the session.
    Disconnect,
    /// `ctx.prefs` changed and should be saved.
    SavePrefs,
}

/// Another Android TV that batch actions are repeated on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub serial: String,
    pub label: String,
}

/// What a tab can reach while handling a message.
pub struct Ctx<'a> {
    pub session: Option<&'a Session>,
    pub log: &'a mut Log,
    pub jobs: &'a mut Jobs,
    pub prefs: &'a mut Prefs,
    pub events: &'a mut Vec<Event>,
    pub journal: &'a mut Journal,
    /// TVs ticked for batch actions (never the connected one).
    pub batch: &'a [Target],
}

impl Ctx<'_> {
    pub fn android(&self) -> Option<android::Session> {
        self.session.and_then(Session::android).cloned()
    }

    /// The quick, interactive connection.
    pub fn conn(&self) -> Option<Connection> {
        self.session.and_then(Session::android).map(|s| s.conn.clone())
    }

    pub fn log(&mut self, line: impl Into<String>) {
        self.log.push(line);
    }

    /// Log what an operation did and record its changes in the History.
    pub fn apply(&mut self, outcome: Outcome) {
        outcome.lines.into_iter().for_each(|l| self.log.push(l));
        if outcome.changes.is_empty() {
            return;
        }
        let Some(session) = self.session else { return };
        self.journal.record(session.address(), &session.label(), outcome.changes);
        if let Err(e) = self.journal.save() {
            self.log.push(format!("Could not save the change history: {e}"));
        }
    }

    /// The same as [`Ctx::apply`] for a batch TV: lines tagged with its name,
    /// changes recorded in its own history.
    pub fn apply_on(&mut self, target: &Target, outcome: Outcome) {
        outcome.lines.into_iter().for_each(|l| self.log.push(format!("[{}] {l}", target.label)));
        if self.journal.record(&target.serial, &target.label, outcome.changes) > 0
            && let Err(e) = self.journal.save()
        {
            self.log.push(format!("Could not save the change history: {e}"));
        }
    }

    /// Batch TVs' names, for confirmations and banners.
    pub fn batch_names(&self) -> String {
        self.batch.iter().map(|t| t.label.as_str()).collect::<Vec<_>>().join(", ")
    }

    /// Run `op` on every batch TV, each on its own connection and job.
    pub fn job_on_batch<T, Fut, M>(
        &mut self,
        title: &str,
        unit: Unit,
        op: impl Fn(Connection, Progress) -> Fut + Clone + Send + 'static,
        done: impl Fn(JobId, Target, Result<T, String>) -> M + Clone + Send + 'static,
    ) -> Task<M>
    where
        T: Send + 'static,
        Fut: Future<Output = T> + Send + 'static,
        M: Send + 'static,
    {
        let tasks: Vec<Task<M>> = self
            .batch
            .to_vec()
            .into_iter()
            .map(|target| {
                let (id, progress) = self.jobs.start(format!("{title} on {}", target.label), unit);
                let (op, done, serial) = (op.clone(), done.clone(), target.serial.clone());
                Task::perform(
                    async move {
                        let conn = android::transport::connect(serial).await.map_err(|e| e.to_string())?;
                        Ok(op(conn, progress).await)
                    },
                    move |r| done(id, target.clone(), r),
                )
            })
            .collect();
        Task::batch(tasks)
    }

    /// Run a long operation on its own connection, shown in the activity bar
    /// with `progress` until it finishes. `done` gets the job id back so the
    /// tab can call `ctx.jobs.finish(id)`, plus the result (or the reason no
    /// job connection could be opened).
    pub fn job<T, Fut, M>(
        &mut self,
        title: impl Into<String>,
        unit: Unit,
        op: impl FnOnce(Connection, Progress) -> Fut + Send + 'static,
        done: impl FnOnce(JobId, Result<T, String>) -> M + Send + 'static,
    ) -> Task<M>
    where
        T: Send + 'static,
        Fut: Future<Output = T> + Send + 'static,
        M: Send + 'static,
    {
        // No TV: answer at once through `done`, so the tab's usual error path
        // runs and clears any busy flag it set before calling this.
        let Some(session) = self.android() else { return Task::done(done(0, Err("not connected to a TV".into()))) };
        let (id, progress) = self.jobs.start(title, unit);
        Task::perform(
            async move {
                let conn = session.job_connection().await?;
                Ok(op(conn, progress).await)
            },
            move |result| done(id, result),
        )
    }
}

/// Read-only context for drawing.
pub struct View<'a> {
    pub session: Option<&'a Session>,
    pub journal: &'a Journal,
    pub batch: &'a [Target],
}

impl View<'_> {
    /// "Also on: Bedroom TV, Office" when batch TVs are ticked.
    pub fn batch_note(&self) -> Option<String> {
        (!self.batch.is_empty()).then(|| format!("Also on: {}", self.batch.iter().map(|t| t.label.as_str()).collect::<Vec<_>>().join(", ")))
    }
}

impl View<'_> {
    pub fn android(&self) -> Option<&android::Session> {
        self.session.and_then(Session::android)
    }
}
