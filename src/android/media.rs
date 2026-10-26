//! Screen recording.

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
pub async fn save_recording(conn: Connection, dest: Option<PathBuf>, progress: crate::jobs::Progress) -> Result<String, String> {
    if let Some(size) = conn.file_size(RECORDING.into()).await {
        progress.set_total(size);
    }
    let result = match dest {
        Some(dest) => conn
            .pull(RECORDING.into(), dest.clone(), Duration::from_secs(600), Some(progress))
            .await
            .map(|size| format!("Saved {} ({}).", dest.display(), super::sysinfo::fmt_bytes(size)))
            .map_err(|e| e.to_string()),
        None => Ok("Recording discarded.".into()),
    };
    let _ = conn.shell(format!("rm -f {RECORDING}"), Duration::from_secs(10)).await;
    result
}
