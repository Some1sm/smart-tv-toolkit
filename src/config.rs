//! Where the app keeps its own files.
//!
//! Windows: `%APPDATA%\SmartTVToolkit`, macOS: `~/Library/Application Support/SmartTVToolkit`,
//! Linux: `~/.config/SmartTVToolkit`.

use std::path::PathBuf;
use std::sync::OnceLock;

const APP_DIR: &str = "SmartTVToolkit";

/// The data folder, created on first use. Resolved once: views call this
/// while drawing, so it must not touch the disk every frame.
pub fn data_dir() -> Option<PathBuf> {
    static DIR: OnceLock<Option<PathBuf>> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = dirs::config_dir()?.join(APP_DIR);
        std::fs::create_dir_all(&dir).ok()?;
        Some(dir)
    })
    .clone()
}
