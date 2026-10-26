//! scrcpy: find it, let the user pick it, or download it, then launch it.

use std::io::Read;
use std::path::{Path, PathBuf};


use sha2::{Digest, Sha256};

use crate::jobs::{CountingReader, Progress};

#[cfg(windows)]
const EXE: &str = "scrcpy.exe";
#[cfg(not(windows))]
const EXE: &str = "scrcpy";

/// Accept the executable or the folder that contains it.
pub fn resolve(path: &Path) -> Option<PathBuf> {
    let p = if path.is_dir() { path.join(EXE) } else { path.to_path_buf() };
    p.is_file().then_some(p)
}

/// The saved choice if it still exists, else next to this app, else PATH.
pub fn find(saved: Option<&Path>) -> Option<PathBuf> {
    if let Some(p) = saved.and_then(resolve) {
        return Some(p);
    }
    let beside = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf));
    let mut candidates: Vec<PathBuf> = beside.into_iter().flat_map(|d| [d.join(EXE), d.join("bin").join(EXE)]).collect();
    if let Some(path) = std::env::var_os("PATH") {
        candidates.extend(std::env::split_paths(&path).map(|d| d.join(EXE)));
    }
    candidates.into_iter().find(|p| p.is_file())
}

fn command(exe: &Path) -> std::process::Command {
    let mut cmd = std::process::Command::new(exe);
    if let Some(dir) = exe.parent() {
        cmd.current_dir(dir); // scrcpy looks for its adb and server beside itself
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// `scrcpy 5.0.1` from `scrcpy --version`, if it runs.
pub async fn version(exe: PathBuf) -> Option<String> {
    tokio::task::spawn_blocking(move || {
        let out = command(&exe).arg("--version").output().ok()?;
        let text = String::from_utf8_lossy(&out.stdout);
        let first = text.lines().next()?.trim();
        first.starts_with("scrcpy").then(|| first.split(" <").next().unwrap_or(first).to_owned())
    })
    .await
    .ok()
    .flatten()
}

/// Start scrcpy for `serial`. scrcpy brings its own adb, which reads the same
/// ~/.android/adbkey, so the TV doesn't ask to allow debugging again.
pub fn launch(exe: &Path, serial: &str) -> Result<String, String> {
    command(exe).arg(format!("--tcpip={serial}")).spawn().map_err(|e| format!("couldn't start scrcpy: {e}"))?;
    Ok(format!("Started scrcpy ({}).", exe.display()))
}

/// Release asset for this computer, e.g. `scrcpy-win64-v5.0.1.zip`.
fn asset_prefix() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => Some("scrcpy-win64-"),
        ("windows", "aarch64") => Some("scrcpy-winarm64-"),
        ("windows", "x86") => Some("scrcpy-win32-"),
        ("macos", "aarch64") => Some("scrcpy-macos-aarch64-"),
        ("macos", "x86_64") => Some("scrcpy-macos-x86_64-"),
        ("linux", "x86_64") => Some("scrcpy-linux-x86_64-"),
        _ => None,
    }
}

fn get(url: &str, agent: &str) -> Result<ureq::http::Response<ureq::Body>, String> {
    ureq::get(url).header("User-Agent", agent).call().map_err(|e| e.to_string())
}

/// Download the latest scrcpy for this computer into the data folder, check
/// it against the release's SHA256SUMS, unpack it, and return the executable.
pub async fn download(progress: Progress) -> Result<PathBuf, String> {
    tokio::task::spawn_blocking(move || download_blocking(&progress)).await.map_err(|e| e.to_string())?
}

fn download_blocking(progress: &Progress) -> Result<PathBuf, String> {
    const AGENT: &str = concat!("SmartTVToolkit/", env!("CARGO_PKG_VERSION"));
    let prefix = asset_prefix().ok_or("no scrcpy build for this computer; install it yourself and use Locate…")?;
    let mut resp = get("https://api.github.com/repos/Genymobile/scrcpy/releases/latest", AGENT)?;
    let release: serde_json::Value =
        serde_json::from_str(&resp.body_mut().read_to_string().map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let tag = release["tag_name"].as_str().unwrap_or("").to_owned();
    let assets = release["assets"].as_array().ok_or("release has no files")?;
    let url_of = |pred: &dyn Fn(&str) -> bool| {
        assets.iter().find_map(|a| {
            let name = a["name"].as_str()?;
            pred(name).then(|| (name.to_owned(), a["browser_download_url"].as_str().unwrap_or("").to_owned()))
        })
    };
    let (name, url) = url_of(&|n| n.starts_with(prefix) && (n.ends_with(".zip") || n.ends_with(".tar.gz")))
        .ok_or_else(|| format!("no {prefix}* file in scrcpy {tag}"))?;
    let (_, sums_url) = url_of(&|n| n == "SHA256SUMS.txt").ok_or("release has no SHA256SUMS.txt")?;

    // Expected checksum first, so a bad download is caught before unpacking.
    let sums = get(&sums_url, AGENT)?.body_mut().read_to_string().map_err(|e| e.to_string())?;
    let expected = sums
        .lines()
        .find_map(|l| {
            let (hash, file) = l.split_once(char::is_whitespace)?;
            (file.trim().trim_start_matches('*') == name).then(|| hash.to_lowercase())
        })
        .ok_or_else(|| format!("{name} isn't listed in SHA256SUMS.txt"))?;

    progress.set_detail(format!("scrcpy {tag}"));
    let mut resp = get(&url, "Mozilla/5.0")?;
    progress.set_total(resp.body().content_length().unwrap_or(0));
    let mut reader = CountingReader {
        inner: resp.body_mut().with_config().limit(200 * 1024 * 1024).reader(),
        progress: progress.clone(),
    };
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    let actual = format!("{:x}", Sha256::digest(&bytes));
    if actual != expected {
        return Err(format!("checksum mismatch for {name}; not installing it"));
    }

    let base = crate::config::data_dir().ok_or("no data folder")?.join("scrcpy");
    let dest = base.join(&tag);
    let _ = std::fs::remove_dir_all(&dest);
    std::fs::create_dir_all(&dest).map_err(|e| e.to_string())?;
    if name.ends_with(".zip") {
        zip::ZipArchive::new(std::io::Cursor::new(bytes))
            .and_then(|mut z| z.extract(&dest))
            .map_err(|e| e.to_string())?;
    } else {
        tar::Archive::new(flate2::read::GzDecoder::new(&bytes[..])).unpack(&dest).map_err(|e| e.to_string())?;
    }
    find_in(&dest).ok_or_else(|| format!("{EXE} not found in {name}"))
}

/// The scrcpy executable somewhere under `dir` (archives contain a folder).
fn find_in(dir: &Path) -> Option<PathBuf> {
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = find_in(&path) {
                return Some(found);
            }
        } else if path.file_name().is_some_and(|n| n == EXE) {
            return Some(path);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_file_or_folder() {
        let dir = std::env::temp_dir().join(format!("stt-scrcpy-{}", std::process::id()));
        let nested = dir.join("scrcpy-win64-v5.0.1");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join(EXE), b"").unwrap();
        assert_eq!(resolve(&nested), Some(nested.join(EXE)));
        assert_eq!(resolve(&nested.join(EXE)), Some(nested.join(EXE)));
        assert_eq!(resolve(&dir), None);
        assert_eq!(find_in(&dir), Some(nested.join(EXE)));
        assert_eq!(find(Some(&nested)), Some(nested.join(EXE)));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn has_an_asset_for_supported_builds() {
        if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
            assert_eq!(asset_prefix(), Some("scrcpy-win64-"));
        }
    }
}
