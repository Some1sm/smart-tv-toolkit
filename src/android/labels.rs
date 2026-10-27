//! Real app names ("Netflix" rather than "Ninja") and icons, read from each
//! APK's manifest and resource table.
//!
//! ADB has no command for an app's label, so the TV unzips just
//! AndroidManifest.xml and resources.arsc (a few MB, not the whole APK) and
//! sends them back as base64 text, and they're parsed here. Results are cached per
//! app version in `labels.json`, so each version is only ever fetched once.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use apk_info_axml::{ARSC, AXML};

use super::{Connection, shell_quote};
use crate::jobs::{Progress, Unit};

const CACHE_FILE: &str = "labels.json";
const ICONS_DIR: &str = "icons";

/// `package@version` → label.
pub type Cache = HashMap<String, String>;

pub fn key(package: &str, version: &str) -> String {
    format!("{package}@{version}")
}

fn cache_path() -> Option<PathBuf> {
    Some(crate::config::data_dir()?.join(CACHE_FILE))
}

pub fn load_cache() -> Cache {
    cache_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

pub fn save_cache(cache: &Cache) {
    if let (Some(path), Ok(text)) = (cache_path(), serde_json::to_string(cache)) {
        let _ = std::fs::write(path, text);
    }
}

/// Resolve `<application android:label>` from the two raw files.
pub fn parse_label(manifest: &[u8], arsc: &[u8]) -> Option<String> {
    let arsc = ARSC::new(&mut &arsc[..]).ok();
    let axml = AXML::new(&mut &manifest[..], arsc.as_ref()).ok()?;
    let label = axml.get_attribute_value("application", "label", arsc.as_ref())?;
    // An unresolved reference comes back as "@…" or a resource id.
    let label = label.trim().to_owned();
    (!label.is_empty() && !label.starts_with('@') && !label.starts_with("0x")).then_some(label)
}

/// Where icons are kept on this computer: `<package>@<version>.png|webp`,
/// or `.none` when the app has no usable bitmap icon.
pub fn icons_dir() -> Option<PathBuf> {
    let dir = crate::config::data_dir()?.join(ICONS_DIR);
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

/// Every icon on disk, by `package@version` (no entry: not fetched yet;
/// `None`: the app has none).
pub fn icons_on_disk() -> HashMap<String, Option<PathBuf>> {
    let Some(dir) = icons_dir() else { return HashMap::new() };
    let Ok(entries) = std::fs::read_dir(dir) else { return HashMap::new() };
    entries
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let path = e.path();
            let (stem, ext) = (path.file_stem()?.to_str()?.to_owned(), path.extension()?.to_str()?.to_owned());
            Some((stem, (ext != "none").then_some(path)))
        })
        .collect()
}

/// File names on disk can't contain everything a version string can.
fn icon_stem(key: &str) -> String {
    key.chars().map(|c| if c.is_ascii_alphanumeric() || "._@-+".contains(c) { c } else { '_' }).collect()
}

pub fn icon_key(package: &str, version: &str) -> String {
    icon_stem(&key(package, version))
}

/// Density folders, best first for a small icon in a list.
const DENSITIES: [&str; 6] = ["xhdpi", "xxhdpi", "hdpi", "xxxhdpi", "mdpi", "nodpi"];

/// Pick the bitmap for `<application android:icon>` (or the TV banner):
/// a path the resource table resolves to, else the same-named file in the
/// best density folder of the APK's file `listing` (`unzip -l`).
pub fn icon_entry(manifest: &[u8], arsc: &[u8], listing: &str) -> Option<String> {
    let arsc = ARSC::new(&mut &arsc[..]).ok();
    let axml = AXML::new(&mut &manifest[..], arsc.as_ref()).ok()?;
    let files: Vec<&str> = listing.split_whitespace().filter(|w| w.starts_with("res/")).collect();
    let bitmap = |p: &str| p.ends_with(".png") || p.ends_with(".webp");
    for attr in ["icon", "banner"] {
        if let Some(path) = axml.get_attribute_value("application", attr, arsc.as_ref()).filter(|p| bitmap(p))
            && files.contains(&path.as_str())
        {
            return Some(path);
        }
        // "@mipmap/ic_launcher" → res/mipmap-<density>/ic_launcher.png|webp
        let Some(raw) = axml.get_attribute_value("application", attr, None) else { continue };
        let Some((kind, name)) = raw.trim_start_matches('@').split_once('/') else { continue };
        let candidates: Vec<&str> = files
            .iter()
            .copied()
            .filter(|f| bitmap(f))
            .filter(|f| {
                let mut parts = f.trim_start_matches("res/").splitn(2, '/');
                let (folder, file) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
                (folder == kind || folder.starts_with(&format!("{kind}-")))
                    && file.rsplit_once('.').is_some_and(|(stem, _)| stem == name)
            })
            .collect();
        let rank = |f: &&&str| DENSITIES.iter().position(|d| f.contains(&format!("-{d}"))).unwrap_or(DENSITIES.len());
        if let Some(best) = candidates.iter().min_by_key(rank) {
            return Some((*best).to_owned());
        }
    }
    None
}

/// The label and (bitmap bytes, extension) of the icon of one app.
async fn read_app(conn: &Connection, package: &str) -> (Option<String>, Option<(Vec<u8>, &'static str)>) {
    let sh = |cmd: String| async move { conn.shell(cmd, Duration::from_secs(30)).await.ok() };
    let Some(path) = sh(format!("pm path {} | head -n1", shell_quote(package))).await else { return (None, None) };
    let Some(apk) = path.trim().strip_prefix("package:").map(str::to_owned) else { return (None, None) };
    let apk = shell_quote(&apk);
    // Files come back as base64 text, so nothing is written on the TV.
    let read = |entry: String| {
        let cmd = format!("unzip -p {apk} {entry} 2>/dev/null | base64");
        async move {
            use base64::Engine;
            let b64 = conn.shell(cmd, Duration::from_secs(120)).await.ok()?;
            let bytes = base64::engine::general_purpose::STANDARD.decode(b64.split_whitespace().collect::<String>()).ok()?;
            (!bytes.is_empty()).then_some(bytes)
        }
    };
    let Some(manifest) = read("AndroidManifest.xml".into()).await else { return (None, None) };
    let arsc = read("resources.arsc".into()).await.unwrap_or_default();
    let label = parse_label(&manifest, &arsc);
    let listing = sh(format!("unzip -l {apk} 'res/*' 2>/dev/null")).await.unwrap_or_default();
    let icon = match icon_entry(&manifest, &arsc, &listing) {
        Some(entry) => {
            let ext = if entry.ends_with(".webp") { "webp" } else { "png" };
            read(shell_quote(&entry)).await.map(|bytes| (bytes, ext))
        }
        None => None,
    };
    (label, icon)
}

/// Save an icon (or a "none" marker) for `package@version`.
fn save_icon(key: &str, icon: Option<(Vec<u8>, &'static str)>) {
    let Some(dir) = icons_dir() else { return };
    let stem = icon_stem(key);
    let _ = match icon {
        Some((bytes, ext)) => std::fs::write(dir.join(format!("{stem}.{ext}")), bytes),
        None => std::fs::write(dir.join(format!("{stem}.none")), b""),
    };
}

/// Look up labels and icons for `(package, version)` pairs. Returns the
/// labels (keyed like the cache); apps without a readable label are
/// recorded as "" so they aren't fetched again. Icons go straight to disk.
pub async fn fetch(conn: Connection, wanted: Vec<(String, String)>, progress: Progress) -> Cache {
    let mut found = Cache::new();
    progress.reset(Unit::Items, wanted.len() as u64, "");
    for (package, version) in wanted {
        progress.set_detail(package.clone());
        let (label, icon) = read_app(&conn, &package).await;
        let key = key(&package, &version);
        save_icon(&key, icon);
        found.insert(key, label.unwrap_or_default());
        progress.add(1);
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Read-only (nothing written on the TV): finds an app's icon through
    /// base64 output. `STT_TEST_TV=ip:port STT_TEST_PKG=org.smarttube.beta
    /// cargo test live_icon -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn live_icon() {
        use base64::Engine;
        let serial = std::env::var("STT_TEST_TV").expect("set STT_TEST_TV=ip:port");
        let pkg = std::env::var("STT_TEST_PKG").unwrap_or_else(|_| "org.smarttube.beta".into());
        let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
        rt.block_on(async {
            let conn = super::super::transport::connect(serial).await.expect("connect");
            let sh = |cmd: String| {
                let conn = conn.clone();
                async move { conn.shell(cmd, Duration::from_secs(60)).await.unwrap_or_default() }
            };
            let b64 = |s: String| base64::engine::general_purpose::STANDARD.decode(s.split_whitespace().collect::<String>()).unwrap_or_default();
            let apk = sh(format!("pm path {pkg} | head -n1")).await.trim().trim_start_matches("package:").to_owned();
            let manifest = b64(sh(format!("unzip -p {apk} AndroidManifest.xml | base64")).await);
            let arsc = b64(sh(format!("unzip -p {apk} resources.arsc | base64")).await);
            let listing = sh(format!("unzip -l {apk} 'res/*'")).await;
            println!("label: {:?}", parse_label(&manifest, &arsc));
            let entry = icon_entry(&manifest, &arsc, &listing);
            println!("icon entry: {entry:?}");
            if let Some(e) = entry {
                let bytes = b64(sh(format!("unzip -p {apk} '{e}' | base64")).await);
                println!("icon bytes: {} (png: {})", bytes.len(), bytes.starts_with(&[0x89, b'P', b'N', b'G']));
            }
            println!("installed catalog apps: {:?}", super::super::catalog::installed_versions(conn.clone()).await);
        });
    }

    #[test]
    fn garbage_gives_no_label() {
        assert_eq!(parse_label(b"not a manifest", b""), None);
        assert_eq!(key("com.a", "1.0"), "com.a@1.0");
        assert_eq!(icon_key("com.a", "26.19.0+rc3/2026"), "com.a@26.19.0+rc3_2026");
        assert_eq!(icon_entry(b"nope", b"", "res/mipmap-xhdpi/ic_launcher.png"), None);
    }

    /// Reads real labels from a TV (reads only; temp files are removed):
    /// `STT_TEST_TV=ip:port cargo test live_labels -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn live_labels() {
        let serial = std::env::var("STT_TEST_TV").expect("set STT_TEST_TV=ip:port");
        let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
        rt.block_on(async {
            let conn = super::super::transport::connect(serial).await.expect("connect");
            let pkgs = super::super::packages::inventory(conn.clone(), String::new()).await.unwrap();
            let wanted = pkgs.iter().filter(|p| p.installed && !p.path.starts_with("/system")).map(|p| (p.id.clone(), p.version.clone())).collect();
            let t = std::time::Instant::now();
            let found = fetch(conn, wanted, Progress::new(Unit::Items)).await;
            println!("{} labels in {:?}", found.len(), t.elapsed());
            let mut rows: Vec<_> = found.into_iter().collect();
            rows.sort();
            for (k, label) in rows {
                println!("  {k:60} {label:?}");
            }
        });
    }
}
