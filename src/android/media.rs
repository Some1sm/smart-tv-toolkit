//! Screen recording and launching scrcpy.

use std::path::PathBuf;
use std::time::Duration;

use super::{Connection, transport};

const RECORDING: &str = "/data/local/tmp/stt-recording.mp4";
/// Android's screenrecord stops by itself after 3 minutes.
pub const MAX_RECORDING: Duration = Duration::from_secs(180);

/// Record the screen until stopped (or the 3-minute limit). Runs on its own
/// connection: commands on one connection run one at a time, so this would
/// otherwise block Stop and everything else.
pub async fn record(serial: String) -> Result<(), String> {
    let conn = transport::connect(serial).await.map_err(|e| e.to_string())?;
    let out = conn
        .shell(format!("screenrecord --bit-rate 4000000 {RECORDING}"), MAX_RECORDING + Duration::from_secs(20))
        .await
        .map_err(|e| e.to_string())?;
    // screenrecord prints nothing on success; anything else is an error.
    if out.trim().is_empty() { Ok(()) } else { Err(out) }
}

/// Ask screenrecord to finish the file (SIGINT, not kill, or the MP4 is broken).
pub async fn stop(conn: Connection) {
    let _ = conn.shell("pkill -INT screenrecord", Duration::from_secs(10)).await;
}

/// Copy the finished recording to `dest` and delete it from the TV.
pub async fn save_recording(conn: Connection, dest: Option<PathBuf>) -> Result<String, String> {
    let result = match dest {
        Some(dest) => conn
            .pull(RECORDING.into(), dest.clone(), Duration::from_secs(600))
            .await
            .map(|size| format!("Saved {} ({}).", dest.display(), super::sysinfo::fmt_bytes(size)))
            .map_err(|e| e.to_string()),
        None => Ok("Recording discarded.".into()),
    };
    let _ = conn.shell(format!("rm -f {RECORDING}"), Duration::from_secs(10)).await;
    result
}

#[cfg(windows)]
const SCRCPY: &str = "scrcpy.exe";
#[cfg(not(windows))]
const SCRCPY: &str = "scrcpy";

/// Find scrcpy: next to this app (or in its bin/ folder), then on PATH.
pub fn find_scrcpy() -> Option<PathBuf> {
    let beside = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.to_path_buf()));
    let mut candidates: Vec<PathBuf> = beside.into_iter().flat_map(|d| [d.join(SCRCPY), d.join("bin").join(SCRCPY)]).collect();
    if let Some(path) = std::env::var_os("PATH") {
        candidates.extend(std::env::split_paths(&path).map(|d| d.join(SCRCPY)));
    }
    candidates.into_iter().find(|p| p.is_file())
}

/// Start scrcpy for `serial`. scrcpy brings its own adb, which reads the same
/// ~/.android/adbkey, so the TV doesn't ask to allow debugging again.
pub fn launch_scrcpy(serial: &str) -> Result<String, String> {
    let exe = find_scrcpy().ok_or("scrcpy not found. Put scrcpy next to this app, or install it on your PATH.")?;
    let mut cmd = std::process::Command::new(&exe);
    cmd.arg(format!("--tcpip={serial}"));
    if let Some(dir) = exe.parent() {
        cmd.current_dir(dir); // scrcpy looks for its adb and server files beside itself
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd.spawn().map_err(|e| format!("couldn't start scrcpy: {e}"))?;
    Ok(format!("Started scrcpy ({}).", exe.display()))
}
