//! Saved devices (`devices.json`).
//!
//! Same format as Android TV Desktop Toolkit's `data/devices.json` — an object
//! keyed by IP — so that file can be imported as is. Fields this app doesn't
//! use (e.g. `verified`, `wireless`) are kept when saving.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use chrono::{Local, NaiveDateTime};
use serde::{Deserialize, Serialize};

const FILE: &str = "devices.json";
/// `last_seen` format, local time: `2026-10-25 23:21`.
const TIME_FORMAT: &str = "%Y-%m-%d %H:%M";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedDevice {
    pub label: String,
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default)]
    pub last_seen: String,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

fn default_port() -> u16 {
    crate::android::ADB_PORT
}

/// IP → device, sorted for a stable file.
pub type History = BTreeMap<String, SavedDevice>;

pub fn path() -> Option<PathBuf> {
    Some(crate::config::data_dir()?.join(FILE))
}

pub fn load_from(path: &Path) -> Result<History, String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    // Editors such as Windows Notepad may add a byte-order mark.
    serde_json::from_str(text.trim_start_matches('\u{feff}')).map_err(|e| format!("not a devices.json file: {e}"))
}

pub fn load() -> History {
    path().and_then(|p| load_from(&p).ok()).unwrap_or_default()
}

pub fn save(history: &History) -> Result<(), String> {
    let path = path().ok_or("no config directory")?;
    let text = serde_json::to_string_pretty(history).map_err(|e| e.to_string())?;
    std::fs::write(path, text).map_err(|e| e.to_string())
}

pub fn now() -> String {
    Local::now().format(TIME_FORMAT).to_string()
}

/// Record a successful connection.
pub fn remember(history: &mut History, ip: &str, port: u16, label: &str) {
    let entry = history.entry(ip.to_owned()).or_insert_with(|| SavedDevice {
        label: String::new(),
        port,
        last_seen: String::new(),
        extra: Default::default(),
    });
    entry.label = label.to_owned();
    entry.port = port;
    entry.last_seen = now();
}

/// Merge `other` into `history`; per IP, the more recently seen entry wins.
/// Returns how many entries were added or updated.
pub fn merge(history: &mut History, other: History) -> usize {
    let mut changed = 0;
    for (ip, device) in other {
        // The timestamp format sorts chronologically as text.
        let newer = history.get(&ip).is_none_or(|old| device.last_seen > old.last_seen);
        if newer {
            history.insert(ip, device);
            changed += 1;
        }
    }
    changed
}

/// "just now", "5m ago", "3h ago", "2d ago" — or "" if unknown.
pub fn age(last_seen: &str, now: NaiveDateTime) -> String {
    let Ok(seen) = NaiveDateTime::parse_from_str(last_seen, TIME_FORMAT) else {
        return String::new();
    };
    let minutes = (now - seen).num_minutes();
    match minutes {
        i64::MIN..2 => "just now".into(),
        2..60 => format!("{minutes}m ago"),
        60..1440 => format!("{}h ago", minutes / 60),
        _ => format!("{}d ago", minutes / 1440),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PYTHON_FILE: &str = r#"{
      "192.168.1.20": {"label": "Google Chromecast", "port": 5555, "last_seen": "2026-10-25 22:43"},
      "192.168.1.78": {"label": "Sony BRAVIA 4K VH2", "port": 5555, "last_seen": "2026-10-25 23:21",
                       "verified": true, "wireless": false},
      "10.0.0.9": {"label": "Old TV"}
    }"#;

    #[test]
    fn reads_the_python_apps_file_and_keeps_unknown_fields() {
        let history: History = serde_json::from_str(PYTHON_FILE).unwrap();
        assert_eq!(history.len(), 3);
        assert_eq!(history["10.0.0.9"].port, 5555);
        let sony = &history["192.168.1.78"];
        assert_eq!(sony.label, "Sony BRAVIA 4K VH2");
        let saved = serde_json::to_string(&history).unwrap();
        assert!(saved.contains(r#""verified":true"#));
    }

    #[test]
    fn merge_keeps_the_newer_entry() {
        let mut ours: History = serde_json::from_str(
            r#"{"192.168.1.78": {"label": "Sony XR-65A80J", "port": 5555, "last_seen": "2026-10-26 10:00"}}"#,
        )
        .unwrap();
        let theirs: History = serde_json::from_str(PYTHON_FILE).unwrap();
        assert_eq!(merge(&mut ours, theirs), 2); // .20 and .9 added; .78 is older there
        assert_eq!(ours["192.168.1.78"].label, "Sony XR-65A80J");
        assert_eq!(ours.len(), 3);
    }

    #[test]
    fn ages() {
        let now = NaiveDateTime::parse_from_str("2026-10-26 12:00", TIME_FORMAT).unwrap();
        assert_eq!(age("2026-10-26 11:59", now), "just now");
        assert_eq!(age("2026-10-26 11:15", now), "45m ago");
        assert_eq!(age("2026-10-26 09:00", now), "3h ago");
        assert_eq!(age("2026-10-24 12:00", now), "2d ago");
        assert_eq!(age("", now), "");
    }
}
