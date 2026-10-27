//! Pairing tokens TVs hand out after the user presses Allow (`pairings.json`):
//! Samsung's token and LG's client key, by IP, so the prompt shows only once.

use std::collections::BTreeMap;
use std::path::PathBuf;

const FILE: &str = "pairings.json";

fn path() -> Option<PathBuf> {
    Some(crate::config::data_dir()?.join(FILE))
}

fn load() -> BTreeMap<String, String> {
    path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|text| serde_json::from_str(text.trim_start_matches('\u{feff}')).ok())
        .unwrap_or_default()
}

pub fn get(ip: &str) -> Option<String> {
    load().remove(ip)
}

pub fn set(ip: &str, token: &str) -> Result<(), String> {
    let mut all = load();
    if all.get(ip).map(String::as_str) == Some(token) {
        return Ok(());
    }
    all.insert(ip.to_owned(), token.to_owned());
    let path = path().ok_or("no data folder")?;
    let text = serde_json::to_string_pretty(&all).map_err(|e| e.to_string())?;
    std::fs::write(path, text).map_err(|e| e.to_string())
}
