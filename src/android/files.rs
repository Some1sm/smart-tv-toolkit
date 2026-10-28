//! The TV's files: listing folders, copying files both ways, and moving,
//! copying, renaming and deleting on the TV itself.

use std::path::{Path, PathBuf};
use std::time::Duration;

use super::Connection;

pub use super::shell_quote as quote;
use crate::jobs::{Progress, Unit};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub is_dir: bool,
    /// Bytes (files only).
    pub size: Option<u64>,
    /// `2026-10-27 19:30`
    pub modified: String,
}

/// Skip `n` whitespace-separated fields, returning the rest of the line
/// untouched (file names can hold runs of spaces).
fn after_fields(line: &str, n: usize) -> Option<&str> {
    let mut rest = line.trim_start();
    for _ in 0..n {
        let end = rest.find(char::is_whitespace)?;
        rest = rest[end..].trim_start();
    }
    (!rest.is_empty()).then_some(rest)
}

/// Parse toybox `ls -lAL`:
/// `drwxrwx--- 3 root everybody 4096 2025-06-17 17:12 Alarms`.
/// Folders first, then by name.
pub fn parse_ls(out: &str) -> Result<Vec<Entry>, String> {
    let lines: Vec<&str> = out.lines().map(|l| l.trim_end_matches('\r')).filter(|l| !l.trim().is_empty()).collect();
    let errors: Vec<&str> = lines.iter().copied().filter(|l| l.trim_start().starts_with("ls:")).collect();
    if !lines.is_empty() && errors.len() == lines.len() {
        return Err(errors.join("\n"));
    }
    let mut entries: Vec<Entry> = lines
        .into_iter()
        .filter_map(|l| {
            let fields: Vec<&str> = l.split_whitespace().take(7).collect();
            let [perms, _, _, _, size, date, time] = fields[..] else { return None };
            if perms.len() < 10 || !date.contains('-') || !time.contains(':') {
                return None;
            }
            let name = after_fields(l, 7)?;
            let is_dir = perms.starts_with('d');
            (!matches!(name, "." | "..")).then(|| Entry {
                name: name.to_owned(),
                is_dir,
                size: if is_dir { None } else { size.parse().ok() },
                modified: format!("{date} {time}"),
            })
        })
        .collect();
    sort(&mut entries, Sort::Name);
    Ok(entries)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Sort {
    #[default]
    Name,
    Size,
    Modified,
}

/// Folders first; then by name (A–Z), size or date (biggest/newest first).
pub fn sort(entries: &mut [Entry], by: Sort) {
    entries.sort_by(|a, b| {
        b.is_dir.cmp(&a.is_dir).then_with(|| match by {
            Sort::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
            Sort::Size => b.size.cmp(&a.size).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())),
            Sort::Modified => b.modified.cmp(&a.modified),
        })
    });
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

/// The last part of a path (`/sdcard/Movies/` → `Movies`).
pub fn base_name(path: &str) -> &str {
    path.trim_end_matches('/').rsplit('/').next().unwrap_or(path)
}

/// Folders that must never be deleted, moved or renamed from here.
const ROOTS: [&str; 13] = [
    "/",
    "/sdcard/",
    "/storage/",
    "/storage/emulated/",
    "/storage/emulated/0/",
    "/storage/self/",
    "/storage/self/primary/",
    "/data/",
    "/data/local/",
    "/data/local/tmp/",
    "/mnt/",
    "/system/",
    "/vendor/",
];

pub fn is_protected(path: &str) -> bool {
    ROOTS.contains(&as_dir(path).as_str())
}

/// A name for a new file or folder: no slashes, not `.` or `..`.
pub fn valid_name(name: &str) -> bool {
    let n = name.trim();
    !n.is_empty() && !n.contains('/') && n != "." && n != ".." && !n.contains('\0')
}

async fn sh(conn: &Connection, cmd: String, secs: u64) -> Result<String, String> {
    conn.shell(cmd, Duration::from_secs(secs)).await.map_err(|e| e.to_string())
}

pub async fn list(conn: Connection, dir: String) -> Result<Vec<Entry>, String> {
    // -L follows links (so /sdcard and USB links show as folders); broken ones are left out.
    // Piped, so ls doesn't escape spaces in names as it does for a terminal.
    parse_ls(&sh(&conn, format!("ls -lAL {} 2>&1 | cat", quote(&dir)), 20).await?)
}

/// USB drives and SD cards: folders in /storage besides the internal ones.
pub async fn volumes(conn: Connection) -> Vec<String> {
    let out = sh(&conn, "ls -1 /storage 2>/dev/null".into(), 10).await.unwrap_or_default();
    parse_volumes(&out)
}

pub fn parse_volumes(out: &str) -> Vec<String> {
    out.lines()
        .map(str::trim)
        .filter(|n| !n.is_empty() && !["emulated", "self", "sdcard", "sdcard0"].contains(n))
        .map(|n| format!("/storage/{n}/"))
        .collect()
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

/// Every file under `dir` with its path relative to `dir` (`a/b.txt`).
fn local_files(dir: &Path) -> Vec<(PathBuf, String)> {
    let mut out = Vec::new();
    let mut stack = vec![(dir.to_path_buf(), String::new())];
    while let Some((path, rel)) = stack.pop() {
        let Ok(read) = std::fs::read_dir(&path) else { continue };
        for entry in read.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let rel = if rel.is_empty() { name } else { format!("{rel}/{name}") };
            match entry.file_type() {
                Ok(t) if t.is_dir() => stack.push((entry.path(), rel)),
                Ok(t) if t.is_file() => out.push((entry.path(), rel)),
                _ => {}
            }
        }
    }
    out.sort_by(|a, b| a.1.cmp(&b.1));
    out
}

/// Copy files and folders from the PC into `remote_dir` on the TV. Existing
/// files with the same name are replaced. One log line per item and a summary.
pub async fn upload(conn: Connection, locals: Vec<PathBuf>, remote_dir: String, progress: Progress) -> Vec<String> {
    let dir = as_dir(&remote_dir);
    // (local file, path on the TV)
    let mut files: Vec<(PathBuf, String)> = Vec::new();
    for local in &locals {
        let name = local.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if local.is_dir() {
            files.extend(local_files(local).into_iter().map(|(p, rel)| (p, format!("{dir}{name}/{rel}"))));
        } else {
            files.push((local.clone(), format!("{dir}{name}")));
        }
    }
    if files.is_empty() {
        return vec!["Nothing to upload (empty folders are skipped).".into()];
    }
    let mut log = Vec::new();
    let (mut ok, mut total) = (0usize, 0u64);
    let count = files.len();
    for (i, (local, remote)) in files.iter().enumerate() {
        let name = base_name(remote);
        let size = std::fs::metadata(local).map(|m| m.len()).unwrap_or(0);
        let detail = if count > 1 { format!("{name} ({} of {count})", i + 1) } else { name.to_owned() };
        progress.reset(Unit::Bytes, size, detail);
        let folder = parent(remote);
        if folder != dir && let Err(e) = sh(&conn, format!("mkdir -p {}", quote(&folder)), 15).await {
            log.push(format!("Can't create {folder}: {e}"));
            continue;
        }
        match conn.push(local.clone(), remote.clone(), Duration::from_secs(60 + size / 1_000_000), Some(progress.clone())).await {
            Ok(()) => {
                ok += 1;
                total += size;
                if count <= 20 {
                    log.push(format!("Sent {name} to {remote}."));
                }
            }
            Err(e) => log.push(format!("Failed {}: {e}", local.display())),
        }
    }
    log.push(format!("Uploaded {ok} of {count} file(s), {} to {dir}.", super::sysinfo::fmt_bytes(total)));
    log
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

/// Download several files and folders into `dest`.
pub async fn download_many(conn: Connection, remotes: Vec<String>, dest: PathBuf, progress: Progress) -> Vec<String> {
    let mut log = Vec::new();
    for remote in remotes {
        log.extend(download(conn.clone(), remote, dest.clone(), progress.clone()).await);
    }
    log
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transfer {
    Move,
    Copy,
}

/// Why `source` can't go into `dest_dir`, if it can't.
pub fn transfer_problem(source: &str, dest_dir: &str) -> Option<&'static str> {
    let (src, dest) = (as_dir(source), as_dir(dest_dir));
    if is_protected(source) {
        Some("it's a system folder")
    } else if dest.starts_with(&src) {
        Some("a folder can't go inside itself")
    } else if parent(&src) == dest {
        Some("it's already in this folder")
    } else {
        None
    }
}

/// Move or copy items into `dest_dir` on the TV, never replacing anything
/// already there. One log line per item.
pub async fn transfer(conn: Connection, mode: Transfer, sources: Vec<String>, dest_dir: String, progress: Progress) -> Vec<String> {
    let dest = as_dir(&dest_dir);
    progress.reset(Unit::Items, sources.len() as u64, "");
    let mut log = Vec::new();
    for source in sources {
        let name = base_name(&source).to_owned();
        progress.set_detail(name.clone());
        let target = format!("{dest}{name}");
        let verb = if mode == Transfer::Move { "Move" } else { "Copy" };
        if let Some(why) = transfer_problem(&source, &dest) {
            log.push(format!("{verb} {name}: skipped, {why}."));
            progress.add(1);
            continue;
        }
        let (src, tgt) = (quote(source.trim_end_matches('/')), quote(&target));
        let cmd = match mode {
            Transfer::Move => format!("if [ -e {tgt} ]; then echo EXISTS; else mv -n {src} {tgt} 2>&1; fi"),
            Transfer::Copy => format!("if [ -e {tgt} ]; then echo EXISTS; else cp -r -n {src} {tgt} 2>&1; fi"),
        };
        let line = match sh(&conn, cmd, 1800).await {
            Ok(out) if out.contains("EXISTS") => format!("{verb} {name}: skipped, {target} already exists."),
            Ok(out) if out.trim().is_empty() => format!("{}{name} to {dest}.", if mode == Transfer::Move { "Moved " } else { "Copied " }),
            Ok(out) => format!("{verb} {name}: {}", out.trim()),
            Err(e) => format!("{verb} {name} failed: {e}"),
        };
        log.push(line);
        progress.add(1);
    }
    log
}

pub async fn make_dir(conn: Connection, dir: String, name: String) -> Result<String, String> {
    if !valid_name(&name) {
        return Err("names can't contain /".into());
    }
    let path = format!("{}{}", as_dir(&dir), name.trim());
    let out = sh(&conn, format!("mkdir {} 2>&1", quote(&path)), 15).await?;
    if out.trim().is_empty() { Ok(format!("Created {path}.")) } else { Err(out.trim().to_owned()) }
}

pub async fn rename(conn: Connection, path: String, new_name: String) -> Result<String, String> {
    if !valid_name(&new_name) {
        return Err("names can't contain /".into());
    }
    if is_protected(&path) {
        return Err("that's a system folder".into());
    }
    let from = path.trim_end_matches('/').to_owned();
    let to = format!("{}{}", parent(&from), new_name.trim());
    let (f, t) = (quote(&from), quote(&to));
    let out = sh(&conn, format!("if [ -e {t} ]; then echo EXISTS; else mv -n {f} {t} 2>&1; fi"), 60).await?;
    match out.trim() {
        "" => Ok(format!("Renamed {from} to {to}.")),
        o if o.contains("EXISTS") => Err(format!("{to} already exists")),
        o => Err(o.to_owned()),
    }
}

/// Delete files and folders (with everything in them). One log line each.
pub async fn delete(conn: Connection, paths: Vec<String>, progress: Progress) -> Vec<String> {
    progress.reset(Unit::Items, paths.len() as u64, "");
    let mut log = Vec::new();
    for path in paths {
        progress.set_detail(base_name(&path).to_owned());
        let line = if is_protected(&path) {
            format!("Delete {path}: skipped, it's a system folder.")
        } else {
            let p = quote(path.trim_end_matches('/'));
            match sh(&conn, format!("rm -rf -- {p} 2>&1; [ -e {p} ] && echo STILL"), 600).await {
                Ok(out) if out.contains("STILL") => format!("Couldn't delete {path}: {}", out.replace("STILL", "").trim()),
                Ok(_) => format!("Deleted {path}."),
                Err(e) => format!("Delete {path} failed: {e}"),
            }
        };
        log.push(line);
        progress.add(1);
    }
    log
}

#[cfg(test)]
mod tests {
    use super::*;

    // From a Sony A80J (toybox 0.8.4).
    const LS: &str = "total 120
-rw-rw---- 1 root everybody  360 2025-06-16 21:31 .config
drwxrwx--- 3 root everybody 4096 2025-06-17 17:12 Alarms
-rw-rw---- 1 root everybody 37679179 2023-10-15 20:08 Magis-TV-la-versión-última.apk
drwxrwx--- 2 root everybody     4096 2021-11-26 17:47 Offline  pages
-rw-rw---- 1 root everybody 10 2026-01-01 10:00 a.txt
ls: ./secret: Permission denied
";

    #[test]
    fn parses_ls_folders_first() {
        let entries = parse_ls(LS).unwrap();
        let names: Vec<(&str, bool)> = entries.iter().map(|e| (e.name.as_str(), e.is_dir)).collect();
        assert_eq!(names, [("Alarms", true), ("Offline  pages", true), (".config", false), ("a.txt", false), ("Magis-TV-la-versión-última.apk", false)]);
        assert_eq!(entries[4].size, Some(37_679_179));
        assert_eq!(entries[0].size, None);
        assert_eq!(entries[0].modified, "2025-06-17 17:12");
        assert!(parse_ls("ls: /x: No such file or directory").unwrap_err().contains("No such file"));
        assert_eq!(parse_ls("").unwrap(), []);
        assert_eq!(parse_ls("total 0\n").unwrap(), []);
    }

    #[test]
    fn sorts() {
        let mut entries = parse_ls(LS).unwrap();
        sort(&mut entries, Sort::Size);
        assert_eq!(entries[2].name, "Magis-TV-la-versión-última.apk");
        sort(&mut entries, Sort::Modified);
        assert_eq!((entries[0].name.as_str(), entries[2].name.as_str()), ("Alarms", "a.txt"));
    }

    #[test]
    fn paths() {
        assert_eq!(quote("/sdcard/My Movies/it's.mp4"), "'/sdcard/My Movies/it'\\''s.mp4'");
        assert_eq!(as_dir("/sdcard"), "/sdcard/");
        assert_eq!(as_dir("/sdcard//"), "/sdcard/");
        assert_eq!(parent("/sdcard/Download/"), "/sdcard/");
        assert_eq!(parent("/sdcard/a.txt"), "/sdcard/");
        assert_eq!(parent("/sdcard/"), "/");
        assert_eq!(parent("/"), "/");
        assert_eq!(base_name("/sdcard/Movies/"), "Movies");
        assert_eq!(parse_volumes("emulated\nsdcard\nself\n1A2B-3C4D\n"), ["/storage/1A2B-3C4D/"]);
    }

    #[test]
    fn guards() {
        assert!(is_protected("/sdcard") && is_protected("/storage/emulated/0/") && !is_protected("/sdcard/Download"));
        assert!(valid_name("New folder") && !valid_name("a/b") && !valid_name("..") && !valid_name("  "));
        assert_eq!(transfer_problem("/sdcard/Movies/", "/sdcard/Movies/Old/"), Some("a folder can't go inside itself"));
        assert_eq!(transfer_problem("/sdcard/Movies/a.mp4", "/sdcard/Movies/"), Some("it's already in this folder"));
        assert_eq!(transfer_problem("/sdcard", "/sdcard/Download/"), Some("it's a system folder"));
        assert_eq!(transfer_problem("/sdcard/Movies/a.mp4", "/sdcard/Download/"), None);
        // "/sdcard/Movie" isn't a parent of "/sdcard/Movies/".
        assert_eq!(transfer_problem("/sdcard/Movie", "/sdcard/Movies/"), None);
    }

    /// Read-only: list a few folders and the USB drives.
    /// `STT_TEST_TV=ip:port cargo test live_files -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn live_files() {
        let serial = std::env::var("STT_TEST_TV").expect("set STT_TEST_TV=ip:port");
        let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
        rt.block_on(async {
            let conn = super::super::transport::connect(serial).await.expect("connect");
            for dir in ["/sdcard/", "/sdcard/Download/", "/data/local/tmp/", "/"] {
                match list(conn.clone(), dir.into()).await {
                    Ok(entries) => {
                        println!("{dir}: {} entries", entries.len());
                        for e in entries.iter().take(6) {
                            println!("  {:<40} dir={} size={:?} {}", e.name, e.is_dir, e.size, e.modified);
                        }
                    }
                    Err(e) => println!("{dir}: error {e}"),
                }
            }
            println!("volumes: {:?}", volumes(conn.clone()).await);
            println!("missing: {:?}", list(conn, "/sdcard/nope/".into()).await);
        });
    }

    #[test]
    fn finds_local_files() {
        let root = std::env::temp_dir().join(format!("stt-files-{}", std::process::id()));
        std::fs::create_dir_all(root.join("b/c")).unwrap();
        std::fs::write(root.join("a.txt"), "1").unwrap();
        std::fs::write(root.join("b/c/d.txt"), "2").unwrap();
        let rels: Vec<String> = local_files(&root).into_iter().map(|(_, r)| r).collect();
        std::fs::remove_dir_all(&root).unwrap();
        assert_eq!(rels, ["a.txt", "b/c/d.txt"]);
    }
}
