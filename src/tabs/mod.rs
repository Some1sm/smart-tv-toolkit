//! The tabs. Each module owns its state, messages, `update` and `view`; the
//! app shell routes messages to it and hands it a [`Ctx`] for the things
//! shared between tabs (the connected TV, the log, background jobs, prefs).

use std::future::Future;

use iced::Task;

use crate::android::{self, Connection};
use crate::jobs::{JobId, Jobs, Progress, Unit};
use crate::log::Log;
use crate::platform::Session;
use crate::prefs::Prefs;

pub mod info;
pub mod install;
pub mod media;
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
    Performance,
    Install,
    Settings,
    Tools,
    Media,
    Remote,
    Screenshot,
}

impl Tab {
    pub const ALL: [Tab; 9] = [
        Tab::Info,
        Tab::Packages,
        Tab::Performance,
        Tab::Install,
        Tab::Settings,
        Tab::Tools,
        Tab::Media,
        Tab::Remote,
        Tab::Screenshot,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Tab::Info => "Info",
            Tab::Packages => "Packages",
            Tab::Performance => "Performance",
            Tab::Install => "Install",
            Tab::Settings => "Settings",
            Tab::Tools => "Tools",
            Tab::Media => "Media",
            Tab::Remote => "Remote",
            Tab::Screenshot => "Screenshot",
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

/// What a tab can reach while handling a message.
pub struct Ctx<'a> {
    pub session: Option<&'a Session>,
    pub log: &'a mut Log,
    pub jobs: &'a mut Jobs,
    pub prefs: &'a mut Prefs,
    pub events: &'a mut Vec<Event>,
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
}

impl View<'_> {
    pub fn android(&self) -> Option<&android::Session> {
        self.session.and_then(Session::android)
    }
}
