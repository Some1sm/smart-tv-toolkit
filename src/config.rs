//! Where the app keeps its own files (saved ADB location, device history).
//!
//! Windows: `%APPDATA%\SmartTVToolkit`, macOS: `~/Library/Application Support/SmartTVToolkit`,
//! Linux: `~/.config/SmartTVToolkit`.

use std::path::PathBuf;

const APP_DIR: &str = "SmartTVToolkit";
const ADB_PATH_FILE: &str = "adb_path.txt";

pub fn data_dir() -> Option<PathBuf> {
    let dir = dirs::config_dir()?.join(APP_DIR);
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

pub fn saved_adb_path() -> Option<PathBuf> {
    let text = std::fs::read_to_string(data_dir()?.join(ADB_PATH_FILE)).ok()?;
    let text = text.trim();
    (!text.is_empty()).then(|| PathBuf::from(text))
}

pub fn save_adb_path(path: &std::path::Path) -> std::io::Result<()> {
    let dir = data_dir().ok_or_else(|| std::io::Error::other("no config directory"))?;
    std::fs::write(dir.join(ADB_PATH_FILE), path.to_string_lossy().as_bytes())
}
