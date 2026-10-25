//! Quick-install apps: where each one is published and how to pick the right
//! file from a release. Downloads land in the app's data folder.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// GitHub releases; `tag` pins one release, otherwise the latest.
    GitHub { repo: &'static str, tag: Option<&'static str> },
    /// Aurora Store's GitLab releases.
    AuroraGitLab,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppSpec {
    pub name: &'static str,
    /// File name stem for downloads.
    pub slug: &'static str,
    pub description: &'static str,
    pub homepage: &'static str,
    pub source: Source,
    /// Asset name must contain this (lowercase), e.g. "smarttube_stable".
    pub contains: &'static str,
    /// Assets containing any of these are skipped (e.g. "fdroid" builds).
    pub exclude: &'static [&'static str],
    /// Per-CPU builds: (Android ABI, token in the asset name).
    pub arch: &'static [(&'static str, &'static str)],
    /// Asset token to use when no build matches the TV's CPU (e.g. "universal").
    pub fallback: Option<&'static str>,
    /// Shell commands to run after a successful install.
    pub post_install: &'static [&'static str],
}

const fn spec(name: &'static str, slug: &'static str, description: &'static str,
              homepage: &'static str, source: Source) -> AppSpec {
    AppSpec { name, slug, description, homepage, source, contains: "", exclude: &[], arch: &[],
              fallback: None, post_install: &[] }
}

pub const APPS: [AppSpec; 7] = [
    spec("Aurora Store", "aurora_store", "An unofficial FOSS client to Google Play.",
         "https://gitlab.com/AuroraOSS/AuroraStore", Source::AuroraGitLab),
    AppSpec {
        arch: &[("arm64-v8a", "arm64-v8a"), ("armeabi-v7a", "armeabi-v7a"), ("x86_64", "x86_64"), ("x86", "x86")],
        fallback: Some("universal"),
        ..spec("Flicky", "flicky", "Yet Another F-Droid client (wide screen / TV friendly).",
               "https://github.com/mlm-games/flicky", Source::GitHub { repo: "mlm-games/flicky", tag: None })
    },
    AppSpec {
        arch: &[("arm64-v8a", "arm64v8"), ("armeabi-v7a", "arm32v7"), ("x86_64", "x64")],
        ..spec("LocalSend", "localsend",
               "Open-source AirDrop alternative: share files over the local network.",
               "https://github.com/localsend/localsend", Source::GitHub { repo: "localsend/localsend", tag: None })
    },
    AppSpec {
        // Pinned: later builds live on a fork; this is the last release here
        // with the ADB variant.
        contains: "adb",
        post_install: &["pm grant io.github.virresh.matvt android.permission.WRITE_SECURE_SETTINGS"],
        ..spec("MaTVT", "matvt",
               "Virtual mouse for Android TV, for apps without D-pad navigation.",
               "https://github.com/virresh/matvt", Source::GitHub { repo: "virresh/matvt", tag: Some("v1.0.7-pre") })
    },
    AppSpec {
        arch: &[("arm64-v8a", "arm64-v8a"), ("armeabi-v7a", "armeabi-v7a"), ("x86_64", "x86_64")],
        exclude: &["fdroid"],
        ..spec("ObtainX", "obtainx", "Get Android app updates straight from the source.",
               "https://github.com/bikram-agarwal/ObtainX", Source::GitHub { repo: "bikram-agarwal/ObtainX", tag: None })
    },
    AppSpec {
        contains: "smarttube_stable",
        arch: &[("arm64-v8a", "arm64-v8a"), ("armeabi-v7a", "armeabi-v7a"), ("x86", "x86")],
        fallback: Some("universal"),
        ..spec("SmartTube Stable", "smarttube_stable",
               "Ad-free YouTube client for Android TV.", "https://github.com/yuliskov/SmartTube",
               Source::GitHub { repo: "yuliskov/SmartTube", tag: None })
    },
    AppSpec {
        arch: &[("arm64-v8a", "cobalt-arm64"), ("armeabi-v7a", "cobalt-arm.")],
        ..spec("TizenTube", "tizentube",
               "YouTube with ad removal, SponsorBlock and speed control.",
               "https://github.com/reisxd/TizenTubeCobalt", Source::GitHub { repo: "reisxd/TizenTubeCobalt", tag: None })
    },
];

pub const SHIZUKU: AppSpec = spec("Shizuku", "shizuku",
    "Lets apps use system APIs through ADB, without root.", "https://github.com/RikkaApps/Shizuku",
    Source::GitHub { repo: "RikkaApps/Shizuku", tag: None });

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asset {
    pub name: String,
    pub url: String,
}

/// Which per-CPU build to fetch for a TV, as (asset token, file suffix,
/// note for the log). `None` token means the app has a single build.
pub fn build_for(spec: &AppSpec, abis: &[String]) -> Result<(Option<&'static str>, String, Option<String>), String> {
    if spec.arch.is_empty() {
        return Ok((None, String::new(), None));
    }
    if abis.is_empty() {
        return Err(format!("Connect to the TV first: {} has a separate build per CPU type.", spec.name));
    }
    if let Some((abi, token)) = abis.iter().find_map(|a| spec.arch.iter().find(|(abi, _)| abi == a)) {
        return Ok((Some(token), format!("-{abi}"), Some(format!("{abi} build"))));
    }
    match spec.fallback {
        Some(fb) => Ok((Some(fb), format!("-{fb}"), Some(format!("{fb} build (no native {} build)", abis[0])))),
        None => Err(format!("{} has no build for this TV's CPU ({}).", spec.name, abis.join(", "))),
    }
}

/// Pick the APK asset from a GitHub release.
pub fn pick_asset(spec: &AppSpec, assets: &[Asset], token: Option<&str>) -> Option<Asset> {
    assets
        .iter()
        .find(|a| {
            let n = a.name.to_lowercase();
            n.ends_with(".apk")
                && token.is_none_or(|t| n.contains(t))
                && (spec.contains.is_empty() || n.contains(spec.contains))
                && !spec.exclude.iter().any(|x| n.contains(x))
        })
        .cloned()
}

/// Where a downloaded APK is kept. Per-CPU apps keep one file per CPU type,
/// so switching TVs doesn't install the wrong build.
pub fn download_path(spec: &AppSpec, abis: &[String]) -> Option<PathBuf> {
    let suffix = build_for(spec, abis).map(|(_, s, _)| s).unwrap_or_default();
    Some(crate::config::data_dir()?.join("downloads").join(format!("{}{suffix}.apk", spec.slug)))
}

const API_AGENT: &str = concat!("SmartTVToolkit/", env!("CARGO_PKG_VERSION"));
/// Some hosts (e.g. auroraoss.com) reject non-browser agents for downloads.
const DOWNLOAD_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64)";

fn get_json(url: &str) -> Result<Value, String> {
    let mut resp = ureq::get(url)
        .header("User-Agent", API_AGENT)
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| e.to_string())?;
    let text = resp.body_mut().read_to_string().map_err(|e| e.to_string())?;
    serde_json::from_str(&text).map_err(|e| e.to_string())
}

/// Find the download URL for `spec` (blocking). Returns (url, release tag).
pub fn resolve(spec: &AppSpec, token: Option<&str>) -> Result<(String, String), String> {
    match spec.source {
        Source::AuroraGitLab => {
            let data = get_json("https://gitlab.com/api/v4/projects/AuroraOSS%2FAuroraStore/releases")?;
            let release = data.get(0).ok_or("no releases found")?;
            let tag = release["tag_name"].as_str().unwrap_or("").to_owned();
            let link = release["assets"]["links"]
                .as_array()
                .and_then(|links| links.iter().filter_map(|l| l["url"].as_str()).find(|u| u.ends_with(".apk")))
                .map(str::to_owned);
            // Recent releases don't attach .apk links; use the stable URL.
            let url = link.unwrap_or_else(|| format!("https://auroraoss.com/downloads/AuroraStore/Release/AuroraStore-{tag}.apk"));
            Ok((url, tag))
        }
        Source::GitHub { repo, tag } => {
            let api = match tag {
                Some(t) => format!("https://api.github.com/repos/{repo}/releases/tags/{t}"),
                None => format!("https://api.github.com/repos/{repo}/releases/latest"),
            };
            let data = get_json(&api)?;
            let assets: Vec<Asset> = data["assets"]
                .as_array()
                .map(|list| {
                    list.iter()
                        .filter_map(|a| Some(Asset {
                            name: a["name"].as_str()?.to_owned(),
                            url: a["browser_download_url"].as_str()?.to_owned(),
                        }))
                        .collect()
                })
                .unwrap_or_default();
            let asset = pick_asset(spec, &assets, token).ok_or_else(|| format!("no matching APK in the {} release", spec.name))?;
            Ok((asset.url, data["tag_name"].as_str().unwrap_or("").to_owned()))
        }
    }
}

/// Download `url` to `dest` (blocking), via a temp file so a failed download
/// never leaves a broken APK behind.
fn download(url: &str, dest: &Path) -> Result<u64, String> {
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let tmp = dest.with_extension("part");
    let mut resp = ureq::get(url).header("User-Agent", DOWNLOAD_AGENT).call().map_err(|e| e.to_string())?;
    let mut reader = resp.body_mut().with_config().limit(1024 * 1024 * 1024).reader();
    let mut file = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
    let size = std::io::copy(&mut reader, &mut file).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        e.to_string()
    })?;
    drop(file);
    std::fs::rename(&tmp, dest).map_err(|e| e.to_string())?;
    Ok(size)
}

/// Resolve and download the right build of `spec` for a TV with `abis`.
pub async fn fetch(spec: AppSpec, abis: Vec<String>) -> Result<String, String> {
    let (token, _, note) = build_for(&spec, &abis)?;
    let dest = download_path(&spec, &abis).ok_or("no data folder")?;
    let task = tokio::task::spawn_blocking(move || -> Result<String, String> {
        let (url, tag) = resolve(&spec, token)?;
        let size = download(&url, &dest)?;
        let note = note.map(|n| format!(", {n}")).unwrap_or_default();
        Ok(format!("Downloaded {} {tag} ({}{note}).", spec.name, super::sysinfo::fmt_bytes(size)))
    });
    tokio::time::timeout(Duration::from_secs(600), task)
        .await
        .map_err(|_| "download timed out".to_owned())?
        .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assets(names: &[&str]) -> Vec<Asset> {
        names.iter().map(|n| Asset { name: (*n).into(), url: format!("https://x/{n}") }).collect()
    }

    fn abis(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).into()).collect()
    }

    fn app(name: &str) -> &'static AppSpec {
        APPS.iter().find(|a| a.name == name).unwrap()
    }

    #[test]
    fn picks_the_tvs_cpu_build_with_name_filter() {
        let smarttube = app("SmartTube Stable");
        let list = assets(&[
            "SmartTube_beta_30.1_arm64-v8a.apk",
            "SmartTube_stable_30.0_arm64-v8a.apk",
            "SmartTube_stable_30.0_armeabi-v7a.apk",
            "SmartTube_stable_30.0_universal.apk",
        ]);
        // A 32-bit TV like the Sony gets the v7a build, not arm64.
        let (token, suffix, _) = build_for(smarttube, &abis(&["armeabi-v7a", "armeabi"])).unwrap();
        assert_eq!(suffix, "-armeabi-v7a");
        assert_eq!(pick_asset(smarttube, &list, token).unwrap().name, "SmartTube_stable_30.0_armeabi-v7a.apk");
        // No x86_64 build: fall back to universal.
        let (token, suffix, note) = build_for(smarttube, &abis(&["x86_64"])).unwrap();
        assert_eq!((token, suffix.as_str()), (Some("universal"), "-universal"));
        assert!(note.unwrap().contains("no native x86_64"));
        assert_eq!(pick_asset(smarttube, &list, token).unwrap().name, "SmartTube_stable_30.0_universal.apk");
    }

    #[test]
    fn excludes_and_contains() {
        let obtainx = app("ObtainX");
        let list = assets(&["ObtainX-fdroid-armeabi-v7a.apk", "ObtainX-armeabi-v7a.apk"]);
        assert_eq!(pick_asset(obtainx, &list, Some("armeabi-v7a")).unwrap().name, "ObtainX-armeabi-v7a.apk");
        let matvt = app("MaTVT");
        let list = assets(&["matvt-1.0.7.apk", "matvt-adb-1.0.7.apk", "source.zip"]);
        assert_eq!(pick_asset(matvt, &list, None).unwrap().name, "matvt-adb-1.0.7.apk");
    }

    #[test]
    fn per_cpu_apps_need_a_connected_tv() {
        assert!(build_for(app("LocalSend"), &[]).unwrap_err().contains("Connect to the TV first"));
        assert!(build_for(app("TizenTube"), &abis(&["x86"])).unwrap_err().contains("no build"));
        assert_eq!(build_for(&SHIZUKU, &[]).unwrap(), (None, String::new(), None));
    }

    /// Resolves every app against the live release APIs; downloads nothing:
    /// `cargo test live_resolve -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn live_resolve() {
        let tv = abis(&["armeabi-v7a", "armeabi"]);
        for spec in APPS.iter().chain([&SHIZUKU]) {
            match build_for(spec, &tv).and_then(|(token, _, _)| resolve(spec, token)) {
                Ok((url, tag)) => println!("{:16} {tag:14} {url}", spec.name),
                Err(e) => println!("{:16} ERROR {e}", spec.name),
            }
        }
    }
}
