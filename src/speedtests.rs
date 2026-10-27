//! App launch-time results (`speedtests.json`) and measurements of how hard
//! the TV works (`checkups.json`), per TV, so a run after optimizing can be
//! compared with one before.

use std::collections::BTreeMap;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

const FILE: &str = "speedtests.json";
const CHECKUPS: &str = "checkups.json";
/// Runs (and measurements) kept per TV.
const KEEP: usize = 10;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Run {
    pub time: String,
    /// (package, cold-start milliseconds; `None` if it didn't report one).
    pub results: Vec<(String, Option<u32>)>,
}

/// TV IP → runs, oldest first.
pub type Runs = BTreeMap<String, Vec<Run>>;

/// How hard a TV was working at one moment.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Checkup {
    pub time: String,
    /// Why it was taken ("Before Recommended changes"); empty when by hand.
    #[serde(default)]
    pub note: String,
    /// CPU busy over ten seconds, 0–100.
    pub cpu_busy: f32,
    pub mem_available: u64,
    pub mem_total: u64,
    pub swap_used: u64,
    pub storage_free: u64,
    /// App processes running.
    pub apps_running: u32,
}

/// TV IP → measurements, oldest first.
pub type Checkups = BTreeMap<String, Vec<Checkup>>;

fn read<T: DeserializeOwned + Default>(file: &str) -> T {
    crate::config::data_dir()
        .and_then(|dir| std::fs::read_to_string(dir.join(file)).ok())
        .and_then(|text| serde_json::from_str(text.trim_start_matches('\u{feff}')).ok())
        .unwrap_or_default()
}

fn write<T: Serialize>(file: &str, value: &T) -> Result<(), String> {
    let dir = crate::config::data_dir().ok_or("no data folder")?;
    std::fs::write(dir.join(file), serde_json::to_string_pretty(value).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
}

/// Add `item` to the TV's list, keeping the newest [`KEEP`].
fn push<T>(map: &mut BTreeMap<String, Vec<T>>, serial: &str, item: T) {
    let list = map.entry(crate::journal::device_key(serial)).or_default();
    list.push(item);
    if list.len() > KEEP {
        let extra = list.len() - KEEP;
        list.drain(..extra);
    }
}

pub fn load() -> Runs {
    read(FILE)
}

pub fn save(runs: &Runs) -> Result<(), String> {
    write(FILE, runs)
}

pub fn add(runs: &mut Runs, serial: &str, run: Run) {
    push(runs, serial, run);
}

pub fn load_checkups() -> Checkups {
    read(CHECKUPS)
}

pub fn save_checkups(checkups: &Checkups) -> Result<(), String> {
    write(CHECKUPS, checkups)
}

pub fn add_checkup(checkups: &mut Checkups, serial: &str, checkup: Checkup) {
    push(checkups, serial, checkup);
}

/// Per app: (latest ms, the ms from the run before it).
pub fn compare(runs: &[Run]) -> Vec<(String, Option<u32>, Option<u32>)> {
    let Some(latest) = runs.last() else { return Vec::new() };
    let previous = runs.len().checked_sub(2).and_then(|i| runs.get(i));
    latest
        .results
        .iter()
        .map(|(pkg, ms)| {
            let before = previous.and_then(|p| p.results.iter().find(|(q, _)| q == pkg)).and_then(|(_, m)| *m);
            (pkg.clone(), *ms, before)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_with_the_previous_run() {
        let mut runs = Runs::new();
        let r = |ms: u32| Run { time: "t".into(), results: vec![("com.a".into(), Some(ms)), ("com.b".into(), None)] };
        add(&mut runs, "10.0.0.2:5555", r(2000));
        add(&mut runs, "10.0.0.2:41000", r(1500));
        let c = compare(&runs["10.0.0.2"]);
        assert_eq!(c[0], ("com.a".into(), Some(1500), Some(2000)));
        assert_eq!(c[1], ("com.b".into(), None, None));
        for i in 0..KEEP + 2 {
            add(&mut runs, "10.0.0.2:5555", r(i as u32));
        }
        assert_eq!(runs["10.0.0.2"].len(), KEEP);
    }
}
