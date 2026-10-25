//! Locating and running the `adb` executable.
//!
//! Every call goes through [`run`], which applies a timeout, hides the console
//! window on Windows and decodes output as UTF-8 (lossily: device output is
//! not always valid UTF-8, and Windows' default code page cannot decode it).

use std::path::{Path, PathBuf};
use std::time::Duration;

use tokio::process::Command;

use crate::config;

#[cfg(windows)]
pub const ADB_NAME: &str = "adb.exe";
#[cfg(not(windows))]
pub const ADB_NAME: &str = "adb";

/// Default timeout for short commands such as `getprop` or `connect`.
pub const SHORT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    Timeout,
    Spawn(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Timeout => write!(f, "ADB timed out"),
            Error::Spawn(e) => write!(f, "could not run ADB: {e}"),
        }
    }
}

/// Run `adb [-s serial] args...` and return stdout and stderr joined and
/// trimmed. ADB reports most failures on stdout with exit code 0, so callers
/// inspect the text rather than the exit status.
pub async fn run(
    adb: &Path,
    serial: Option<&str>,
    args: &[&str],
    timeout: Duration,
) -> Result<String, Error> {
    let mut cmd = Command::new(adb);
    if let Some(serial) = serial {
        cmd.args(["-s", serial]);
    }
    cmd.args(args).kill_on_drop(true);
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let output = tokio::time::timeout(timeout, cmd.output())
        .await
        .map_err(|_| Error::Timeout)?
        .map_err(|e| Error::Spawn(e.to_string()))?;

    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    Ok(text.trim().to_owned())
}

/// Accept either the adb executable or the folder containing it (e.g. an
/// unzipped `platform-tools/`). Returns the executable path if it exists.
pub fn resolve(path: &Path) -> Option<PathBuf> {
    let trimmed = path.to_string_lossy();
    let trimmed = trimmed.trim().trim_matches('"');
    if trimmed.is_empty() {
        return None;
    }
    let mut p = PathBuf::from(trimmed);
    if p.is_dir() {
        p.push(ADB_NAME);
    }
    p.is_file().then_some(p)
}

/// Find adb: the saved choice first, then next to the app, the usual SDK
/// install folders, and finally `PATH`.
pub fn find() -> Option<PathBuf> {
    if let Some(saved) = config::saved_adb_path().and_then(|p| resolve(&p)) {
        return Some(saved);
    }
    candidates().into_iter().find(|p| p.is_file()).or_else(find_on_path)
}

fn candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) {
        out.push(dir.join("platform-tools").join(ADB_NAME));
        out.push(dir.join("bin").join(ADB_NAME));
    }
    #[cfg(windows)]
    {
        let env = |k: &str| std::env::var_os(k).map(PathBuf::from);
        if let Some(local) = env("LOCALAPPDATA") {
            out.push(local.join("Android/Sdk/platform-tools/adb.exe"));
        }
        for var in ["PROGRAMFILES", "PROGRAMFILES(X86)"] {
            if let Some(pf) = env(var) {
                out.push(pf.join("Android/android-sdk/platform-tools/adb.exe"));
            }
        }
        out.push(PathBuf::from("C:/platform-tools/adb.exe"));
    }
    #[cfg(not(windows))]
    {
        if let Some(home) = dirs::home_dir() {
            out.push(home.join("Library/Android/sdk/platform-tools/adb"));
            out.push(home.join("Android/Sdk/platform-tools/adb"));
        }
        out.push(PathBuf::from("/usr/local/bin/adb"));
        out.push(PathBuf::from("/opt/homebrew/bin/adb"));
    }
    out
}

fn find_on_path() -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(ADB_NAME))
        .find(|p| p.is_file())
}

/// Check that `path` really is a working adb; returns its version line.
pub async fn verify(path: &Path) -> Result<String, String> {
    let out = run(path, None, &["version"], SHORT)
        .await
        .map_err(|e| e.to_string())?;
    out.lines()
        .next()
        .filter(|l| l.contains("Android Debug Bridge"))
        .map(str::to_owned)
        .ok_or_else(|| "That file didn't run as ADB.".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("stt-test-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn resolve_accepts_file_or_folder() {
        let dir = temp_dir("resolve");
        let exe = dir.join(ADB_NAME);
        std::fs::write(&exe, b"").unwrap();

        assert_eq!(resolve(&exe), Some(exe.clone()));
        assert_eq!(resolve(&dir), Some(exe.clone()));
        let quoted = PathBuf::from(format!("  \"{}\" ", exe.display()));
        assert_eq!(resolve(&quoted), Some(exe));
        assert_eq!(resolve(&dir.join("missing")), None);
        assert_eq!(resolve(Path::new("  ")), None);

        std::fs::remove_dir_all(dir).ok();
    }
}
