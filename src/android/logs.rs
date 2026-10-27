//! Reading the TV's logs (`logcat`, read-only) and playing media on it.

use std::path::PathBuf;
use std::time::Duration;

use super::{Connection, shell_quote};

/// Which log to read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// One app's log (by its running process).
    App,
    /// Recent crashes of any app.
    Crashes,
    /// Everything.
    System,
}

/// Lowest priority shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Level {
    #[default]
    All,
    Warnings,
    Errors,
}

impl Level {
    pub const ALL: [Level; 3] = [Level::All, Level::Warnings, Level::Errors];

    fn filter(self) -> &'static str {
        match self {
            Level::All => "*:V",
            Level::Warnings => "*:W",
            Level::Errors => "*:E",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Level::All => "All",
            Level::Warnings => "Warnings",
            Level::Errors => "Errors",
        }
    }
}

/// Lines read at once.
const LINES: u32 = 400;

pub fn command(source: Source, package: &str, level: Level) -> Result<String, String> {
    let filter = level.filter();
    Ok(match source {
        Source::Crashes => format!("logcat -d -b crash -t {LINES} {filter}"),
        Source::System => format!("logcat -d -t {LINES} {filter}"),
        Source::App => {
            if !super::actions::is_package_name(package) {
                return Err("enter the app's package name, e.g. com.example.app".into());
            }
            let q = shell_quote(package);
            format!(
                "pid=$(pidof -s {q}); if [ -n \"$pid\" ]; then logcat -d -t {LINES} --pid=$pid {filter}; \
                 else echo \"{package} isn't running; open it on the TV, then refresh.\"; fi"
            )
        }
    })
}

/// Read a log (changes nothing on the TV).
pub async fn read(conn: Connection, source: Source, package: String, level: Level) -> Result<String, String> {
    let cmd = command(source, &package, level)?;
    let out = conn.shell(cmd, Duration::from_secs(20)).await.map_err(|e| e.to_string())?;
    Ok(if out.trim().is_empty() { "(nothing logged)".into() } else { out })
}

/// The MIME type players expect for a file or link, from its extension.
pub fn mime_for(name: &str) -> &'static str {
    let ext = name.rsplit('.').next().unwrap_or("").to_lowercase();
    match ext.split('?').next().unwrap_or("") {
        "mp4" | "m4v" | "mkv" | "webm" | "avi" | "mov" | "ts" | "m3u8" => "video/*",
        "mp3" | "flac" | "m4a" | "aac" | "ogg" | "wav" | "opus" => "audio/*",
        "jpg" | "jpeg" | "png" | "gif" | "webp" => "image/*",
        _ => "video/*",
    }
}

/// Open `uri` (a web link or a file on the TV) in the TV's player.
pub async fn play(conn: Connection, uri: String) -> Result<String, String> {
    let ok_scheme = ["http://", "https://", "rtsp://", "file:///"].iter().any(|s| uri.starts_with(s));
    if !ok_scheme {
        return Err("links start with http://, https:// or rtsp://".into());
    }
    let cmd = format!("am start -a android.intent.action.VIEW -d {} -t {}", shell_quote(&uri), shell_quote(mime_for(&uri)));
    let out = conn.shell(cmd, Duration::from_secs(20)).await.map_err(|e| e.to_string())?;
    if out.contains("Error") || out.contains("Exception") {
        return Err(out.trim().to_owned());
    }
    Ok(format!("Playing {uri} on the TV."))
}

/// Copy a file to the TV's Movies folder, then play it.
pub async fn send_and_play(conn: Connection, local: PathBuf, progress: crate::jobs::Progress) -> Result<String, String> {
    let name = local.file_name().map(|n| n.to_string_lossy().into_owned()).ok_or("not a file")?;
    super::files::send(conn.clone(), local, "/sdcard/Movies/".into(), progress).await?;
    play(conn, format!("file:///sdcard/Movies/{name}")).await
}

/// `dumpsys ethernet` → the wired MAC address (Wake-on-LAN works over cable).
pub fn parse_ethernet_mac(out: &str) -> Option<String> {
    let rest = out.split("hwAddress: ").nth(1)?;
    let mac: String = rest.chars().take(17).collect();
    (mac.len() == 17 && mac.chars().filter(|c| *c == ':').count() == 5 && mac != "00:00:00:00:00:00").then_some(mac)
}

pub async fn ethernet_mac(conn: Connection) -> Option<String> {
    let out = conn.shell("dumpsys ethernet 2>/dev/null | grep -m1 hwAddress", Duration::from_secs(10)).await.ok()?;
    parse_ethernet_mac(&out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_log_commands() {
        assert_eq!(command(Source::Crashes, "", Level::All).unwrap(), "logcat -d -b crash -t 400 *:V");
        assert!(command(Source::App, "org.smarttube.beta", Level::Errors).unwrap().contains("--pid=$pid *:E"));
        assert!(command(Source::App, "x; reboot", Level::All).is_err());
        assert_eq!(mime_for("https://x/movie.MKV"), "video/*");
        assert_eq!(mime_for("song.flac"), "audio/*");
        let eth = "    eth0:NetworkInterfaceState{ iface: eth0, up: true, hwAddress: 14:3f:a6:cd:2e:b3, networkCapabilities";
        assert_eq!(parse_ethernet_mac(eth).as_deref(), Some("14:3f:a6:cd:2e:b3"));
        assert_eq!(parse_ethernet_mac("nothing"), None);
    }
}
