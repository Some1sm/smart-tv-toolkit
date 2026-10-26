//! Browsing the TV's storage and copying files both ways.

use std::path::PathBuf;
use std::time::Duration;

use super::Connection;

pub use super::shell_quote as quote;
use crate::jobs::{Progress, Unit};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub is_dir: bool,
}

/// Parse `ls -1ap` (folders end in `/`): folders first, then by name.
pub fn parse_ls(out: &str) -> Result<Vec<Entry>, String> {
    let lines: Vec<&str> = out.lines().map(|l| l.trim_end_matches('\r')).filter(|l| !l.trim().is_empty()).collect();
    let errors: Vec<&str> = lines.iter().copied().filter(|l| l.trim_start().starts_with("ls:")).collect();
    if !lines.is_empty() && errors.len() == lines.len() {
        return Err(errors.join("\n"));
    }
    let mut entries: Vec<Entry> = lines
        .into_iter()
        .filter(|l| !matches!(*l, "./" | "../" | "." | "..") && !l.trim_start().starts_with("ls:"))
        .filter_map(|l| {
            let is_dir = l.ends_with('/');
            let name = l.trim_end_matches('/');
            (!name.is_empty()).then(|| Entry { name: name.to_owned(), is_dir })
        })
        .collect();
    entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    Ok(entries)
}

/// Normalise a folder path to end in exactly one `/`.
pub fn as_dir(path: &str) -> String {
    let p = path.trim();
    if p.is_empty() { "/".into() } else { format!("{}/", p.trim_end_matches('/')) }
}

/// Parent folder of `dir` (`/sdcard/Download/` → `/sdcard/`).
pub fn parent(dir: &str) -> String {
    let trimmed = dir.trim_end_matches('/');
    match trimmed.rsplit_once('/') {
        Some((p, _)) if !p.is_empty() => format!("{p}/"),
        _ => "/".into(),
    }
}

pub async fn list(conn: Connection, dir: String) -> Result<Vec<Entry>, String> {
    let out = conn.shell(format!("ls -1ap {}", quote(&dir)), Duration::from_secs(20)).await.map_err(|e| e.to_string())?;
    parse_ls(&out)
}

/// Copy a local file into a folder on the TV.
pub async fn send(conn: Connection, local: PathBuf, remote_dir: String, progress: Progress) -> Result<String, String> {
    let name = local.file_name().ok_or("not a file")?.to_string_lossy().into_owned();
    let remote = format!("{}{name}", as_dir(&remote_dir));
    let size = std::fs::metadata(&local).map_err(|e| e.to_string())?.len();
    progress.set_detail(name.clone());
    conn.push(local, remote.clone(), Duration::from_secs(60 + size / 1_000_000), Some(progress))
        .await
        .map_err(|e| e.to_string())?;
    Ok(format!("Sent {name} to {remote} ({}).", super::sysinfo::fmt_bytes(size)))
}

/// Copy a file or a whole folder from the TV into `dest` on the PC.
/// Returns one log line per file plus a summary.
pub async fn download(conn: Connection, remote: String, dest: PathBuf, progress: Progress) -> Vec<String> {
    let remote = remote.trim().to_owned();
    let is_dir = conn
        .shell(format!("[ -d {} ] && echo DIR", quote(&remote)), Duration::from_secs(10))
        .await
        .is_ok_and(|o| o.contains("DIR"));
    let base = remote.trim_end_matches('/');
    let name = base.rsplit('/').next().filter(|n| !n.is_empty()).unwrap_or("device");

    let files: Vec<(String, PathBuf)> = if is_dir {
        let listing = conn
            .shell(format!("cd {} && find . -type f", quote(base)), Duration::from_secs(60))
            .await
            .unwrap_or_default();
        listing
            .lines()
            .filter_map(|l| l.trim().strip_prefix("./"))
            .map(|rel| (format!("{base}/{rel}"), dest.join(name).join(rel.replace('/', std::path::MAIN_SEPARATOR_STR))))
            .collect()
    } else {
        vec![(remote.clone(), dest.join(name))]
    };
    if files.is_empty() {
        return vec![format!("Nothing to download in {remote}.")];
    }

    let mut log = Vec::new();
    let (mut ok, mut total) = (0usize, 0u64);
    let count = files.len() as u64;
    for (i, (src, local)) in files.iter().enumerate() {
        let name = src.rsplit('/').next().unwrap_or(src);
        let detail = if count > 1 { format!("{name} ({} of {count})", i + 1) } else { name.to_owned() };
        progress.reset(Unit::Bytes, 0, detail);
        if let Some(size) = conn.file_size(src.clone()).await {
            progress.set_total(size);
        }
        if let Some(dir) = local.parent()
            && let Err(e) = std::fs::create_dir_all(dir)
        {
            log.push(format!("Can't create {}: {e}", dir.display()));
            continue;
        }
        match conn.pull(src.clone(), local.clone(), Duration::from_secs(900), Some(progress.clone())).await {
            Ok(size) => {
                ok += 1;
                total += size;
                if files.len() <= 20 {
                    log.push(format!("Saved {}", local.display()));
                }
            }
            Err(e) => log.push(format!("Failed {src}: {e}")),
        }
    }
    log.push(format!(
        "Downloaded {ok} of {} file(s), {} to {}.",
        files.len(),
        super::sysinfo::fmt_bytes(total),
        dest.display()
    ));
    log
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ls_folders_first() {
        let out = "./\n../\n.config\nDownload/\nAlarms/\nnotes.txt\nls: ./secret: Permission denied\n";
        let names: Vec<(String, bool)> = parse_ls(out).unwrap().into_iter().map(|e| (e.name, e.is_dir)).collect();
        assert_eq!(names, [
            ("Alarms".into(), true),
            ("Download".into(), true),
            (".config".into(), false),
            ("notes.txt".into(), false),
        ]);
        assert!(parse_ls("ls: /x: No such file or directory").unwrap_err().contains("No such file"));
        assert_eq!(parse_ls("").unwrap(), []);
    }

    #[test]
    fn paths() {
        assert_eq!(quote("/sdcard/My Movies/it's.mp4"), "'/sdcard/My Movies/it'\\''s.mp4'");
        assert_eq!(as_dir("/sdcard"), "/sdcard/");
        assert_eq!(as_dir("/sdcard//"), "/sdcard/");
        assert_eq!(parent("/sdcard/Download/"), "/sdcard/");
        assert_eq!(parent("/sdcard/"), "/");
        assert_eq!(parent("/"), "/");
    }
}
