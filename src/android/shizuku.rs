//! Shizuku: lets other apps use system APIs through ADB, without root.

use std::time::Duration;

use super::catalog::{self, SHIZUKU};
use super::{Connection, install};

pub const PACKAGE: &str = "moe.shizuku.privileged.api";

/// Shizuku v13+ ships a native starter (libshizuku.so) inside the APK; running
/// it from the ADB shell starts the server directly. The APK folder name
/// changes on every (re)install, so it is looked up via `pm path`. Prints
/// NO_STARTER on older builds, which use the legacy start.sh instead.
const START: &str = "p=$(pm path moe.shizuku.privileged.api | head -n1 | sed 's/^package://; s#/base.apk$##'); \
    d=$(ls \"$p/lib\" 2>/dev/null | head -n1); \
    if [ -n \"$d\" ] && [ -f \"$p/lib/$d/libshizuku.so\" ]; then \"$p/lib/$d/libshizuku.so\"; \
    else echo NO_STARTER; fi";
const LEGACY_START: &str = "sh /sdcard/Android/data/moe.shizuku.privileged.api/start.sh";

async fn sh(conn: &Connection, cmd: &str, secs: u64) -> String {
    conn.shell(cmd, Duration::from_secs(secs)).await.unwrap_or_else(|e| e.to_string())
}

/// Start the Shizuku server and confirm it's running.
pub async fn start(conn: Connection) -> Result<String, String> {
    let mut out = sh(&conn, START, 30).await;
    if out.contains("NO_STARTER") {
        // Older Shizuku: the app writes start.sh on first launch.
        sh(&conn, &format!("monkey -p {PACKAGE} -c android.intent.category.LAUNCHER 1"), 15).await;
        tokio::time::sleep(Duration::from_secs(2)).await;
        out = sh(&conn, LEGACY_START, 30).await;
    }
    // The starter forks the server and exits, so its output can look fine
    // while nothing started. Trust the process table; the fork takes a moment.
    for attempt in 0..6 {
        if attempt > 0 {
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        if sh(&conn, "ps -A -o NAME", 10).await.contains("shizuku_server") {
            return Ok("Shizuku is running. It stops on reboot; use Start Shizuku again after a restart.".into());
        }
    }
    Err(format!("Shizuku didn't start: {}", if out.trim().is_empty() { "no output from the starter" } else { out.trim() }))
}

/// Install the downloaded Shizuku if it isn't installed yet, then start it.
pub async fn install_and_start(conn: Connection, progress: crate::jobs::Progress) -> Result<String, String> {
    let mut lines = Vec::new();
    if install::is_installed(&conn, PACKAGE).await {
        lines.push("Shizuku is already installed.".to_owned());
    } else {
        let apk = catalog::download_path(&SHIZUKU, &[]).filter(|p| p.is_file()).ok_or("Download Shizuku first.")?;
        install::install_with(conn.clone(), apk, &[], progress).await.map_err(|e| format!("Shizuku install failed: {e}"))?;
        lines.push("Shizuku installed.".to_owned());
    }
    lines.push(start(conn).await?);
    Ok(lines.join(" "))
}
