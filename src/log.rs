//! The output log: timestamped, capped so long sessions don't grow forever.

use std::collections::VecDeque;

const CAP: usize = 2000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub time: String,
    pub text: String,
}

#[derive(Debug, Default)]
pub struct Log {
    lines: VecDeque<Line>,
    /// Lines dropped because of the cap, shown so a copy isn't misleading.
    dropped: usize,
}

impl Log {
    pub fn push(&mut self, text: impl Into<String>) {
        let time = chrono::Local::now().format("%H:%M:%S").to_string();
        // Multi-line output (e.g. a console command) stays one entry.
        self.lines.push_back(Line { time, text: text.into().trim_end().to_owned() });
        while self.lines.len() > CAP {
            self.lines.pop_front();
            self.dropped += 1;
        }
    }

    pub fn lines(&self) -> impl Iterator<Item = &Line> {
        self.lines.iter()
    }

    pub fn clear(&mut self) {
        self.lines.clear();
        self.dropped = 0;
    }

    /// The whole log as text, for Copy and Save.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        if self.dropped > 0 {
            out.push_str(&format!("({} older lines not shown)\n", self.dropped));
        }
        for l in &self.lines {
            out.push_str(&format!("[{}] {}\n", l.time, l.text));
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
        log.clear();
        assert_eq!(log.to_text(), "");
    }
}
