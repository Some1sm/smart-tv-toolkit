//! The output log: timestamped, tagged by channel, capped so long sessions
//! don't grow forever.

use std::collections::VecDeque;

const CAP: usize = 2000;

/// Which channel a line belongs to; drives its tag, colour and filtering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    /// A command sent to the TV (`[adb] > adb connect …`).
    Adb,
    /// Normal output.
    Out,
    /// Something that worked (connected, installed, saved…).
    Ok,
    Warn,
    Error,
    /// Selection changes.
    Select,
}

impl Level {
    pub fn tag(self) -> &'static str {
        match self {
            Level::Adb => "[adb]",
            Level::Out | Level::Ok => "[stdout]",
            Level::Warn => "[warn]",
            Level::Error => "[stderr]",
            Level::Select => "[select]",
        }
    }

    /// Guess the channel of a message from its wording.
    fn infer(text: &str) -> Level {
        let lower = text.to_lowercase();
        let starts = |p: &[&str]| p.iter().any(|w| text.starts_with(w));
        if ["failed", "error", "couldn't", "could not", "can't", "not reachable", "refused", "mismatch"]
            .iter()
            .any(|w| lower.contains(w))
        {
            Level::Error
        } else if starts(&["Lost ", "Warning", "The TV hasn't"]) || lower.contains("reconnecting") {
            Level::Warn
        } else if starts(&[
            "Connected", "Reconnected", "Installed", "Saved", "Downloaded", "Freed", "Paired", "Sent", "Read ",
            "Using scrcpy", "Started scrcpy", "Imported", "Shizuku is running",
        ]) {
            Level::Ok
        } else {
            Level::Out
        }
    }
}

/// Which lines the terminal shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Filter {
    #[default]
    All,
    Stdout,
    Stderr,
    Adb,
}

impl Filter {
    pub const ALL: [Filter; 4] = [Filter::All, Filter::Stdout, Filter::Stderr, Filter::Adb];

    pub fn shows(self, level: Level) -> bool {
        match self {
            Filter::All => true,
            Filter::Stdout => matches!(level, Level::Out | Level::Ok | Level::Select),
            Filter::Stderr => matches!(level, Level::Warn | Level::Error),
            Filter::Adb => level == Level::Adb,
        }
    }
}

impl std::fmt::Display for Filter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(crate::i18n::tr(match self {
            Filter::All => "All Channels",
            Filter::Stdout => "stdout",
            Filter::Stderr => "stderr",
            Filter::Adb => "adb commands",
        }))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub time: String,
    pub level: Level,
    pub text: String,
}

#[derive(Debug, Default)]
pub struct Log {
    lines: VecDeque<Line>,
    /// Lines dropped because of the cap, shown so a copy isn't misleading.
    dropped: usize,
}

impl Log {
    /// Add a message; its channel is inferred from the wording.
    pub fn push(&mut self, text: impl Into<String>) {
        let text = text.into();
        let level = Level::infer(&text);
        self.push_at(level, text);
    }

    /// A command sent to the TV.
    pub fn cmd(&mut self, text: impl Into<String>) {
        self.push_at(Level::Adb, text.into());
    }

    pub fn push_at(&mut self, level: Level, text: String) {
        let time = chrono::Local::now().format("%H:%M:%S").to_string();
        #[cfg(debug_assertions)]
        eprintln!("[{time}] {} {}", level.tag(), text.trim_end());
        // Multi-line output (e.g. a console command) stays one entry.
        self.lines.push_back(Line { time, level, text: text.trim_end().to_owned() });
        while self.lines.len() > CAP {
            self.lines.pop_front();
            self.dropped += 1;
        }
    }

    pub fn lines(&self) -> impl Iterator<Item = &Line> {
        self.lines.iter()
    }

    pub fn len(&self) -> usize {
        self.lines.len()
    }

    pub fn clear(&mut self) {
        self.lines.clear();
        self.dropped = 0;
    }

    /// The whole log as text, for Copy and Export.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        if self.dropped > 0 {
            out.push_str(&format!("({} older lines not shown)\n", self.dropped));
        }
        for l in &self.lines {
            out.push_str(&format!("[{}] {} {}\n", l.time, l.level.tag(), l.text));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caps_and_exports() {
        let mut log = Log::default();
        for i in 0..CAP + 5 {
            log.push(format!("line {i}\n"));
        }
        assert_eq!(log.lines().count(), CAP);
        assert_eq!(log.lines().next().unwrap().text, "line 5");
        let text = log.to_text();
        assert!(text.starts_with("(5 older lines not shown)\n["));
        assert!(text.contains("[stdout] line 6"));
        log.clear();
        assert_eq!(log.to_text(), "");
    }

    #[test]
    fn infers_channels_from_wording() {
        assert_eq!(Level::infer("Connected: Sony XR-65A80J"), Level::Ok);
        assert_eq!(Level::infer("Connection failed: refused"), Level::Error);
        assert_eq!(Level::infer("Lost the connection to Sony. Reconnecting…"), Level::Warn);
        assert_eq!(Level::infer("Checked 2 device(s): 1 with ADB open."), Level::Out);
        assert!(Filter::Stderr.shows(Level::Warn) && !Filter::Stderr.shows(Level::Ok));
        assert!(Filter::Adb.shows(Level::Adb) && !Filter::Adb.shows(Level::Out));
    }
}
