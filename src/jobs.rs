//! Long-running work shown in the activity bar, with live progress.
//!
//! Operations get a [`Progress`] handle and bump it from whatever thread they
//! run on; the UI reads it on a timer while any job is active.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    Bytes,
    Items,
}

#[derive(Debug)]
struct Inner {
    done: AtomicU64,
    total: AtomicU64,
    unit: Mutex<Unit>,
    detail: Mutex<String>,
}

/// Shared progress counter. Cheap to clone; a total of 0 means "unknown".
#[derive(Debug, Clone)]
pub struct Progress(Arc<Inner>);

impl Progress {
    pub fn new(unit: Unit) -> Self {
        Progress(Arc::new(Inner {
            done: AtomicU64::new(0),
            total: AtomicU64::new(0),
            unit: Mutex::new(unit),
            detail: Mutex::new(String::new()),
        }))
    }

    pub fn set_total(&self, total: u64) {
        self.0.total.store(total, Ordering::Relaxed);
    }

    pub fn add(&self, n: u64) {
        self.0.done.fetch_add(n, Ordering::Relaxed);
    }

    /// Restart counting for a new phase, e.g. the next file of several.
    pub fn reset(&self, unit: Unit, total: u64, detail: impl Into<String>) {
        *self.0.unit.lock().unwrap() = unit;
        self.0.done.store(0, Ordering::Relaxed);
        self.0.total.store(total, Ordering::Relaxed);
        self.set_detail(detail);
    }

    pub fn set_detail(&self, detail: impl Into<String>) {
        *self.0.detail.lock().unwrap() = detail.into();
    }

    /// (fraction 0–1 if known, text like "12.3 MB of 40.0 MB").
    pub fn snapshot(&self) -> (Option<f32>, String) {
        let done = self.0.done.load(Ordering::Relaxed);
        let total = self.0.total.load(Ordering::Relaxed);
        let unit = *self.0.unit.lock().unwrap();
        let detail = self.0.detail.lock().unwrap().clone();
        let fmt = |n: u64| match unit {
            Unit::Bytes => crate::android::sysinfo::fmt_bytes(n),
            Unit::Items => n.to_string(),
        };
        let amount = match (total, unit) {
            (0, _) if done == 0 => String::new(),
            (0, _) => fmt(done),
            (_, Unit::Items) => format!("{done} of {total}"),
            (_, Unit::Bytes) => format!("{} of {}", fmt(done), fmt(total)),
        };
        let text = match (detail.is_empty(), amount.is_empty()) {
            (true, _) => amount,
            (false, true) => detail,
            (false, false) => format!("{detail} · {amount}"),
        };
        let fraction = (total > 0).then(|| (done as f32 / total as f32).min(1.0));
        (fraction, text)
    }
}

/// Wraps a reader and counts bytes read into a [`Progress`].
pub struct CountingReader<R> {
    pub inner: R,
    pub progress: Progress,
}

impl<R: std::io::Read> std::io::Read for CountingReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.progress.add(n as u64);
        Ok(n)
    }
}

/// Wraps a writer and counts bytes written into a [`Progress`].
pub struct CountingWriter<W> {
    pub inner: W,
    pub progress: Progress,
}

impl<W: std::io::Write> std::io::Write for CountingWriter<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let n = self.inner.write(buf)?;
        self.progress.add(n as u64);
        Ok(n)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

pub type JobId = u64;

#[derive(Debug)]
pub struct Job {
    pub id: JobId,
    pub title: String,
    pub progress: Progress,
    pub started: Instant,
}

/// Everything currently running, oldest first.
#[derive(Debug, Default)]
pub struct Jobs {
    next: JobId,
    pub active: Vec<Job>,
}

impl Jobs {
    pub fn start(&mut self, title: impl Into<String>, unit: Unit) -> (JobId, Progress) {
        self.next += 1;
        let progress = Progress::new(unit);
        self.active.push(Job { id: self.next, title: title.into(), progress: progress.clone(), started: Instant::now() });
        (self.next, progress)
    }

    pub fn finish(&mut self, id: JobId) {
        self.active.retain(|j| j.id != id);
    }

    pub fn is_empty(&self) -> bool {
        self.active.is_empty()
    }

    pub fn clear(&mut self) {
        self.active.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    #[test]
    fn progress_text_and_fraction() {
        let p = Progress::new(Unit::Bytes);
        assert_eq!(p.snapshot(), (None, String::new()));
        p.set_total(4 * 1024 * 1024);
        p.add(1024 * 1024);
        assert_eq!(p.snapshot(), (Some(0.25), "1.0 MB of 4.0 MB".into()));
        p.reset(Unit::Items, 3, "base.apk");
        p.add(2);
        let (f, text) = p.snapshot();
        assert!((f.unwrap() - 2.0 / 3.0).abs() < 1e-6);
        assert_eq!(text, "base.apk · 2 of 3");
    }

    #[test]
    fn counting_io() {
        let p = Progress::new(Unit::Bytes);
        let mut r = CountingReader { inner: &b"hello world"[..], progress: p.clone() };
        let mut out = Vec::new();
        r.read_to_end(&mut out).unwrap();
        let mut w = CountingWriter { inner: Vec::new(), progress: p.clone() };
        w.write_all(b"abc").unwrap();
        assert_eq!(p.snapshot().1, "14 B");
    }

    #[test]
    fn jobs_start_and_finish() {
        let mut jobs = Jobs::default();
        let (a, _) = jobs.start("one", Unit::Items);
        let (b, _) = jobs.start("two", Unit::Bytes);
        assert_ne!(a, b);
        jobs.finish(a);
        assert_eq!(jobs.active.iter().map(|j| j.title.as_str()).collect::<Vec<_>>(), ["two"]);
    }
}
