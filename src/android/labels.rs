//! Real app names ("Netflix" rather than "Ninja"), read from each APK's
//! manifest and resource table.
//!
//! ADB has no command for an app's label, so the TV unzips just
//! AndroidManifest.xml and resources.arsc (a few MB, not the whole APK)
//! into a temp folder, and those are parsed here. Results are cached per
//! app version in `labels.json`, so each version is only ever fetched once.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use apk_info_axml::{ARSC, AXML};

use super::{Connection, shell_quote};
use crate::jobs::{Progress, Unit};

const CACHE_FILE: &str = "labels.json";
const TMP: &str = "/data/local/tmp/stt-label";

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

async fn label_for(conn: &Connection, package: &str) -> Option<String> {
    let sh = |cmd: String| async move { conn.shell(cmd, Duration::from_secs(30)).await.ok() };
    let path = sh(format!("pm path {} | head -n1", shell_quote(package))).await?;
    let apk = path.trim().strip_prefix("package:")?.to_owned();
    let apk = shell_quote(&apk);
    sh(format!(
        "unzip -p {apk} AndroidManifest.xml > {TMP}-m; unzip -p {apk} resources.arsc > {TMP}-r 2>/dev/null; true"
    ))
    .await?;
    let manifest = conn.pull_bytes(format!("{TMP}-m"), Duration::from_secs(60)).await.ok();
    let arsc = conn.pull_bytes(format!("{TMP}-r"), Duration::from_secs(120)).await.ok();
    let _ = sh(format!("rm -f {TMP}-m {TMP}-r")).await;
    parse_label(&manifest?, &arsc.unwrap_or_default())
}

/// Look up labels for `(package, version)` pairs not in the cache yet.
/// Returns the new entries (keyed like the cache); apps without a
/// readable label are recorded as "" so they aren't fetched again.
pub async fn fetch(conn: Connection, wanted: Vec<(String, String)>, progress: Progress) -> Cache {
    let mut found = Cache::new();
    progress.reset(Unit::Items, wanted.len() as u64, "");
    for (package, version) in wanted {
        progress.set_detail(package.clone());
        let label = label_for(&conn, &package).await.unwrap_or_default();
        found.insert(key(&package, &version), label);
        progress.add(1);
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn garbage_gives_no_label() {
        assert_eq!(parse_label(b"not a manifest", b""), None);
        assert_eq!(key("com.a", "1.0"), "com.a@1.0");
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
            let pkgs = super::super::packages::list(conn.clone(), super::super::packages::ListKind::User).await.unwrap();
            let wanted = pkgs.iter().map(|p| (p.id.clone(), p.version.clone())).collect();
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
