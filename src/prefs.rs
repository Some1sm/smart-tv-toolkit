//! Preferences remembered between runs (`prefs.json` in the data folder).
//! Unknown or missing fields fall back to defaults, so old files keep working.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

const FILE: &str = "prefs.json";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    /// `ip:port` of the TV to reconnect to on launch.
    pub last_device: Option<String>,
    pub auto_connect: bool,
    pub subnet: Option<String>,
    pub last_tab: Option<String>,
    pub window_width: Option<f32>,
    pub window_height: Option<f32>,
    /// scrcpy executable chosen by the user or downloaded by the app.
    pub scrcpy: Option<PathBuf>,
    /// Date (YYYY-MM-DD) of the last check for a newer app version.
    pub last_update_check: Option<String>,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs {
            last_device: None,
            auto_connect: true,
            subnet: None,
            last_tab: None,
            window_width: None,
            window_height: None,
            scrcpy: None,
            last_update_check: None,
        }
    }
}

fn path() -> Option<PathBuf> {
    Some(crate::config::data_dir()?.join(FILE))
}

pub fn load() -> Prefs {
    path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        // Editors such as Windows Notepad may add a byte-order mark.
        .and_then(|text| serde_json::from_str(text.trim_start_matches('\u{feff}')).ok())
        .unwrap_or_default()
}

pub fn save(prefs: &Prefs) -> Result<(), String> {
    let path = path().ok_or("no data folder")?;
    let text = serde_json::to_string_pretty(prefs).map_err(|e| e.to_string())?;
    std::fs::write(path, text).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tolerates_a_byte_order_mark() {
        let text = "\u{feff}{\"last_device\": \"10.0.0.2:5555\"}";
        let p: Prefs = serde_json::from_str(text.trim_start_matches('\u{feff}')).unwrap();
        assert_eq!(p.last_device.as_deref(), Some("10.0.0.2:5555"));
    }

    #[test]
    fn missing_fields_use_defaults() {
        let p: Prefs = serde_json::from_str(r#"{"last_device": "192.168.1.78:5555", "future_field": 1}"#).unwrap();
        assert_eq!(p.last_device.as_deref(), Some("192.168.1.78:5555"));
        assert!(p.auto_connect);
        assert_eq!(p.scrcpy, None);
    }
}
