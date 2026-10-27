//! Checking GitHub for a newer release of the app (once a day), and
//! installing it from inside the app.
//!
//! Windows and Linux: the release archive is downloaded, checked against the
//! SHA-256 GitHub publishes for it, and the program file is swapped in place
//! (the running one is renamed to `*.old` first and removed on the next
//! launch). macOS: the `.dmg` is downloaded and opened, since an app bundle
//! can't replace itself safely while running.

use std::io::Read;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::jobs::{CountingReader, Progress, Unit};

const LATEST: &str = "https://api.github.com/repos/Some1sm/smart-tv-toolkit/releases/latest";
const AGENT: &str = concat!("SmartTVToolkit/", env!("CARGO_PKG_VERSION"));

/// The release file for this computer.
#[cfg(windows)]
const ASSET_SUFFIX: &str = "-windows-x64.zip";
#[cfg(target_os = "macos")]
const ASSET_SUFFIX: &str = "-macos-universal.dmg";
#[cfg(all(unix, not(target_os = "macos")))]
const ASSET_SUFFIX: &str = "-linux-x86_64.tar.gz";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asset {
    pub name: String,
    pub url: String,
    /// `sha256:<hex>` as GitHub reports it, when it does.
    pub digest: Option<String>,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub version: String,
    pub page: String,
    /// The download for this computer, if the release has one.
    pub asset: Option<Asset>,
}

/// `v1.2.10` / `1.2.10` → [1, 2, 10]; pre-release suffixes are ignored.
fn parse(version: &str) -> Vec<u64> {
    version
        .trim()
        .trim_start_matches('v')
        .split(['-', '+'])
        .next()
        .unwrap_or("")
        .split('.')
        .map(|p| p.parse().unwrap_or(0))
        .collect()
}

pub fn is_newer(latest: &str, current: &str) -> bool {
    let (mut a, mut b) = (parse(latest), parse(current));
    let len = a.len().max(b.len());
    a.resize(len, 0);
    b.resize(len, 0);
    a > b
}

/// Read GitHub's "latest release" answer.
pub fn parse_release(release: &serde_json::Value, current: &str) -> Option<Release> {
    let tag = release["tag_name"].as_str()?;
    if !is_newer(tag, current) {
        return None;
    }
    let asset = release["assets"].as_array().into_iter().flatten().find_map(|a| {
        let name = a["name"].as_str()?;
        name.ends_with(ASSET_SUFFIX).then(|| Asset {
            name: name.to_owned(),
            url: a["browser_download_url"].as_str().unwrap_or("").to_owned(),
            digest: a["digest"].as_str().map(str::to_owned),
            size: a["size"].as_u64().unwrap_or(0),
        })
    });
    Some(Release {
        version: tag.trim_start_matches('v').to_owned(),
        page: release["html_url"].as_str().unwrap_or("").to_owned(),
        asset: asset.filter(|a| !a.url.is_empty()),
    })
}

/// The newest published release, if it's newer than this build. Network
/// errors and "no releases yet" both mean no notice.
pub async fn check() -> Option<Release> {
    tokio::task::spawn_blocking(|| {
        let mut resp = ureq::get(LATEST)
            .header("User-Agent", AGENT)
            .header("Accept", "application/vnd.github+json")
            .call()
            .ok()?;
        let release: serde_json::Value = serde_json::from_str(&resp.body_mut().read_to_string().ok()?).ok()?;
        parse_release(&release, env!("CARGO_PKG_VERSION"))
    })
    .await
    .ok()
    .flatten()
}

/// Remove what the last update left behind (`<exe>.old`).
pub fn cleanup() {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::fs::remove_file(old_path(&exe));
    }
}

fn old_path(exe: &Path) -> PathBuf {
    let mut name = exe.file_name().unwrap_or_default().to_os_string();
    name.push(".old");
    exe.with_file_name(name)
}

/// Whether `bytes` match GitHub's `sha256:<hex>` digest.
pub fn digest_matches(bytes: &[u8], digest: &str) -> bool {
    let Some(expected) = digest.strip_prefix("sha256:") else { return false };
    let actual: String = Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect();
    actual.eq_ignore_ascii_case(expected)
}

fn download(asset: &Asset, progress: &Progress) -> Result<Vec<u8>, String> {
    progress.reset(Unit::Bytes, asset.size, asset.name.clone());
    let resp = ureq::get(&asset.url).header("User-Agent", AGENT).call().map_err(|e| e.to_string())?;
    let mut reader = CountingReader { inner: resp.into_body().into_reader(), progress: progress.clone() };
    let mut bytes = Vec::with_capacity(asset.size as usize);
    reader.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    match &asset.digest {
        Some(d) if !digest_matches(&bytes, d) => Err("the download doesn't match the release's checksum; nothing was changed".into()),
        _ => Ok(bytes),
    }
}

/// The program file inside a release archive.
fn program_from_archive(name: &str, bytes: &[u8]) -> Result<Vec<u8>, String> {
    if name.ends_with(".zip") {
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| e.to_string())?;
        for i in 0..zip.len() {
            let mut file = zip.by_index(i).map_err(|e| e.to_string())?;
            if file.name().ends_with(".exe") {
                let mut out = Vec::new();
                file.read_to_end(&mut out).map_err(|e| e.to_string())?;
                return Ok(out);
            }
        }
        Err("the archive has no program in it".into())
    } else {
        let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(bytes));
        for entry in archive.entries().map_err(|e| e.to_string())? {
            let mut entry = entry.map_err(|e| e.to_string())?;
            let path = entry.path().map_err(|e| e.to_string())?.into_owned();
            if path.file_name().is_some_and(|n| n == "smart-tv-toolkit") {
                let mut out = Vec::new();
                entry.read_to_end(&mut out).map_err(|e| e.to_string())?;
                return Ok(out);
            }
        }
        Err("the archive has no program in it".into())
    }
}

/// Put `program` where `exe` is: the running file moves aside first, and
/// comes back if writing the new one fails.
fn swap_in(exe: &Path, program: &[u8]) -> Result<(), String> {
    let old = old_path(exe);
    let _ = std::fs::remove_file(&old);
    std::fs::rename(exe, &old).map_err(|e| format!("can't move the current program aside: {e}"))?;
    if let Err(e) = std::fs::write(exe, program) {
        let _ = std::fs::rename(&old, exe);
        return Err(format!("can't write the new program: {e}"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(exe, std::fs::Permissions::from_mode(0o755));
    }
    Ok(())
}

/// What installing did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Installed {
    /// The new version is in place; restart to use it.
    Restart,
    /// The installer was opened for the user to finish (macOS).
    Opened(PathBuf),
}

/// Download and install `asset` (blocking work runs on a worker thread).
pub async fn install(asset: Asset, progress: Progress) -> Result<Installed, String> {
    tokio::task::spawn_blocking(move || {
        let bytes = download(&asset, &progress)?;
        if asset.name.ends_with(".dmg") {
            let dir = dirs::download_dir().unwrap_or_else(std::env::temp_dir);
            let path = dir.join(&asset.name);
            std::fs::write(&path, &bytes).map_err(|e| e.to_string())?;
            open::that(&path).map_err(|e| e.to_string())?;
            return Ok(Installed::Opened(path));
        }
        progress.set_detail("Installing");
        let program = program_from_archive(&asset.name, &bytes)?;
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        swap_in(&exe, &program)?;
        Ok(Installed::Restart)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Start the (new) program and let this one exit.
pub fn relaunch() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    std::process::Command::new(exe).spawn().map(|_| ()).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_versions_numerically() {
        assert!(is_newer("v0.2.0", "0.1.0"));
        assert!(is_newer("0.1.10", "0.1.9"));
        assert!(is_newer("v1.0", "0.9.9"));
        assert!(!is_newer("v0.1.0", "0.1.0"));
        assert!(!is_newer("0.1.0", "0.1.1"));
        assert!(!is_newer("v0.1.0-beta", "0.1.0"));
    }

    #[test]
    fn picks_this_computers_download() {
        let suffix = ASSET_SUFFIX;
        let release = serde_json::json!({
            "tag_name": "v0.2.0",
            "html_url": "https://github.com/Some1sm/smart-tv-toolkit/releases/tag/v0.2.0",
            "assets": [
                {"name": "SmartTVToolkit-0.2.0-other.bin", "browser_download_url": "https://x/other", "size": 1},
                {"name": format!("SmartTVToolkit-0.2.0{suffix}"), "browser_download_url": "https://x/mine", "size": 42,
                 "digest": "sha256:abc"}
            ]
        });
        let r = parse_release(&release, "0.1.0").unwrap();
        assert_eq!(r.version, "0.2.0");
        let asset = r.asset.unwrap();
        assert_eq!((asset.url.as_str(), asset.size, asset.digest.as_deref()), ("https://x/mine", 42, Some("sha256:abc")));
        assert!(parse_release(&release, "0.2.0").is_none());
    }

    #[test]
    fn checks_digests_and_swaps_files() {
        let digest = "sha256:2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"; // "hello"
        assert!(digest_matches(b"hello", digest));
        assert!(!digest_matches(b"hullo", digest));
        assert!(!digest_matches(b"hello", "md5:x"));

        let dir = std::env::temp_dir().join(format!("stt-update-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("app.exe");
        std::fs::write(&exe, b"old").unwrap();
        swap_in(&exe, b"new").unwrap();
        assert_eq!(std::fs::read(&exe).unwrap(), b"new");
        assert_eq!(std::fs::read(old_path(&exe)).unwrap(), b"old");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn finds_the_program_in_a_zip() {
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut buf);
            let opts = zip::write::SimpleFileOptions::default();
            zip.start_file("LICENSE", opts).unwrap();
            std::io::Write::write_all(&mut zip, b"license").unwrap();
            zip.start_file("Smart TV Toolkit.exe", opts).unwrap();
            std::io::Write::write_all(&mut zip, b"MZprogram").unwrap();
            zip.finish().unwrap();
        }
        assert_eq!(program_from_archive("x-windows-x64.zip", buf.get_ref()).unwrap(), b"MZprogram");
    }
}
