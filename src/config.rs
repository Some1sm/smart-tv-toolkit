//! Where the app keeps its own files.
//!
//! Windows: `%APPDATA%\SmartTVToolkit`, macOS: `~/Library/Application Support/SmartTVToolkit`,
//! Linux: `~/.config/SmartTVToolkit`.

use std::path::PathBuf;

const APP_DIR: &str = "SmartTVToolkit";

pub fn data_dir() -> Option<PathBuf> {
    let dir = dirs::config_dir()?.join(APP_DIR);
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}
