//! Installing APKs: upload to the TV's temp folder, `pm install -r -g`
//! (replace if present, grant runtime permissions), then clean up.

use std::path::{Path, PathBuf};
use std::time::Duration;

use super::{Connection, transport};
use crate::jobs::{Progress, Unit};

/// Play Protect settings that can block or prompt during sideloading.
const VERIFIER_KEYS: [&str; 2] = ["package_verifier_enable", "package_verifier_user_consent"];

async fn sh(conn: &Connection, cmd: &str, secs: u64) -> Result<String, String> {
    conn.shell(cmd, Duration::from_secs(secs)).await.map_err(|e| e.to_string())
}

/// Turn off Play Protect's install checks and return the previous values,
/// so they can be restored exactly — some people keep verification off.
async fn relax_verifier(conn: &Connection) -> Vec<(&'static str, String)> {
    let mut previous = Vec::new();
    for key in VERIFIER_KEYS {
        let value = sh(conn, &format!("settings get global {key}"), 10).await.unwrap_or_default();
        previous.push((key, value.trim().to_owned()));
    }
    let _ = sh(
        conn,
        "settings put global package_verifier_enable 0; settings put global package_verifier_user_consent -1",
        10,
    )
    .await;
    previous
}

async fn restore_verifier(conn: &Connection, previous: Vec<(&'static str, String)>) {
    for (key, value) in previous {
        let cmd = if value.is_empty() || value == "null" {
            format!("settings delete global {key}")
        } else {
            format!("settings put global {key} {}", super::shell_quote(&value))
        };
        let _ = sh(conn, &cmd, 10).await;
    }
}

/// Install one APK while the verifier is already relaxed.
async fn install_one(conn: &Connection, apk: &Path, progress: &Progress) -> Result<String, String> {
    let remote = format!("/data/local/tmp/stt-{}.apk", std::process::id());
    let size = std::fs::metadata(apk).map_err(|e| e.to_string())?.len();
    // Allow ~1 MB/s plus slack for slow Wi-Fi.
    let upload_secs = 60 + size / 1_000_000;
    progress.reset(Unit::Bytes, size, apk.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
    conn.push(apk.to_path_buf(), remote.clone(), Duration::from_secs(upload_secs), Some(progress.clone()))
        .await
        .map_err(|e| format!("upload failed: {e}"))?;
    progress.set_detail("Installing on the TV…");
    let result = sh(conn, &format!("pm install -r -g '{remote}'"), 300).await;
    let _ = sh(conn, &format!("rm -f '{remote}'"), 10).await;
    let out = result?;
    if out.contains("Success") { Ok(out) } else { Err(if out.is_empty() { "no output from pm".into() } else { out }) }
}

/// Install several APKs one after another; one log line each.
pub async fn install_many(conn: Connection, apks: Vec<PathBuf>, progress: Progress) -> Vec<String> {
    let previous = relax_verifier(&conn).await;
    let mut log = Vec::with_capacity(apks.len());
    for (i, apk) in apks.iter().enumerate() {
        let name = apk.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let prefix = if apks.len() > 1 { format!("[{}/{}] ", i + 1, apks.len()) } else { String::new() };
        log.push(match install_one(&conn, apk, &progress).await {
            Ok(_) => format!("{prefix}Installed {name}."),
            Err(e) => format!("{prefix}{name}: {}", e.replace('\n', " ")),
        });
    }
    restore_verifier(&conn, previous).await;
    log
}

/// Install one APK, then run follow-up shell commands if it succeeded (e.g.
/// a permission `-g` can't grant).
pub async fn install_with(
    conn: Connection,
    apk: PathBuf,
    post_install: &'static [&'static str],
    progress: Progress,
) -> Result<String, String> {
    let previous = relax_verifier(&conn).await;
    let result = install_one(&conn, &apk, &progress).await;
    restore_verifier(&conn, previous).await;
    result?;
    let mut lines = vec!["Installed.".to_owned()];
    for cmd in post_install {
        let out = sh(&conn, cmd, 30).await.unwrap_or_else(|e| e);
        lines.push(if out.is_empty() { format!("Ran: {cmd}") } else { format!("{cmd}: {out}") });
    }
    Ok(lines.join(" "))
}

/// Is `package` installed?
pub async fn is_installed(conn: &Connection, package: &str) -> bool {
    conn.shell(format!("pm path {}", super::shell_quote(package)), transport::SHORT).await.is_ok_and(|o| o.contains("package:"))
}

/// `*.apk` files directly inside `dir`, sorted by name.
pub fn apks_in(dir: &Path) -> Vec<PathBuf> {
    let mut apks: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.is_file() && p.extension().is_some_and(|x| x.eq_ignore_ascii_case("apk")))
                .collect()
        })
        .unwrap_or_default();
    apks.sort();
    apks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_apks_case_insensitively_and_skips_folders() {
        let dir = std::env::temp_dir().join(format!("stt-apks-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("split.apk")).unwrap(); // a folder, not a file
        for f in ["b.apk", "A.APK", "notes.txt"] {
            std::fs::write(dir.join(f), b"").unwrap();
        }
        let names: Vec<String> =
            apks_in(&dir).iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(names, ["A.APK", "b.apk"]);
        std::fs::remove_dir_all(dir).ok();
    }
}
