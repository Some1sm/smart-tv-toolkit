//! The Optimize tab's work: reading what takes space, runs in the background
//! or starts at boot (read-only), and the changes that free resources. Every
//! change that can be undone returns a [`Change`] for the History.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::time::Duration;

use super::sysinfo::{parse_df, split_sections};
use super::{Connection, shell_quote};
use crate::journal::Change;

async fn sh(conn: &Connection, cmd: &str, secs: u64) -> Result<String, String> {
    conn.shell(cmd, Duration::from_secs(secs)).await.map_err(|e| e.to_string())
}

// ── Storage ──────────────────────────────────────────────────────────────────

/// A preinstalled app with a Play Store update on top, and how much the
/// update takes (the factory version stays on the system image).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemUpdate {
    pub package: String,
    pub bytes: u64,
    /// Seconds on screen this month (None when usage stats aren't readable).
    pub used_secs: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Storage {
    pub free: u64,
    pub total: u64,
    pub updates: Vec<SystemUpdate>,
    /// Large files in shared storage: (path, bytes).
    pub big_files: Vec<(String, u64)>,
    /// (package, app bytes, data bytes, cache bytes), biggest first.
    pub apps: Vec<(String, u64, u64, u64)>,
    /// When Android last measured those sizes (`2026-10-27 19:30`).
    pub apps_measured: Option<String>,
}

const STORAGE: &str = "\
    echo @@df; df -k /data 2>/dev/null; \
    echo @@updated; pm list packages -s -f 2>/dev/null | grep '^package:/data/app'; \
    echo @@sizes; pm list packages -s -f 2>/dev/null | grep '^package:/data/app' | sed -e 's/^package://' -e 's/=[^=]*$//' \
    | xargs stat -c '%s %n' 2>/dev/null; \
    echo @@files; find /sdcard -type f -size +20M -exec stat -c '%s %n' {} + 2>/dev/null; \
    echo @@disk; dumpsys diskstats 2>/dev/null; \
    echo @@diskstamp; ls -l /data/system/diskstats_cache.json 2>/dev/null; \
    echo @@usage; dumpsys usagestats 2>/dev/null | sed -n '/In-memory monthly stats/,/In-memory yearly stats/p' | grep package=";

pub fn parse_storage(raw: &str) -> Storage {
    let sec = split_sections(raw);
    let get = |k: &str| sec.get(k).map(String::as_str).unwrap_or("");
    let df = parse_df(get("df"));
    let sizes: HashMap<String, u64> = get("sizes")
        .lines()
        .filter_map(|l| {
            let (n, p) = l.trim().split_once(' ')?;
            Some((p.to_owned(), n.parse().ok()?))
        })
        .collect();
    // A dir holds base.apk plus splits and compiled code; base.apk stands in.
    let usage = super::health::parse_usage(get("usage"));
    let have_usage = !usage.is_empty();
    let mut updates: Vec<SystemUpdate> = super::packages::parse_full_list(get("updated"))
        .into_iter()
        .map(|(package, path, _)| SystemUpdate {
            bytes: sizes.get(&path).copied().unwrap_or(0),
            used_secs: have_usage.then(|| usage.get(&package).copied().unwrap_or(0)),
            package,
        })
        .collect();
    updates.sort_by(|a, b| b.bytes.cmp(&a.bytes));
    let mut big_files: Vec<(String, u64)> = get("files")
        .lines()
        .filter_map(|l| {
            let (n, p) = l.trim().split_once(' ')?;
            Some((p.to_owned(), n.parse().ok()?))
        })
        .collect();
    big_files.sort_by(|a, b| b.1.cmp(&a.1));
    let mut apps = super::health::parse_diskstats(get("disk"));
    apps.sort_by(|a, b| (b.2 + b.3).cmp(&(a.2 + a.3)));
    // `ls -l`: "-rw------- 1 system system 312 2026-10-27 19:30 /data/…"
    let apps_measured = get("diskstamp").split_whitespace().collect::<Vec<_>>().windows(2).find_map(|w| {
        (w[0].len() == 10 && w[0].chars().filter(|c| *c == '-').count() == 2 && w[1].contains(':')).then(|| format!("{} {}", w[0], w[1]))
    });
    Storage {
        apps_measured,
        free: df.first().map_or(0, |r| r.avail),
        total: df.first().map_or(0, |r| r.total),
        updates,
        big_files,
        apps,
    }
}

pub async fn storage(conn: Connection) -> Result<Storage, String> {
    Ok(parse_storage(&sh(&conn, STORAGE, 90).await?))
}

/// Updates other apps depend on: rolling these back breaks apps or loses
/// security fixes, so they're never offered.
pub fn keep_update(package: &str) -> bool {
    matches!(
        package,
        "com.google.android.gms"
            | "com.google.android.gsf"
            | "com.android.vending"
            | "com.google.android.webview"
            | "com.android.webview"
            | "com.google.android.tvlauncher"
            | "com.google.android.apps.tv.launcherx"
            | "com.google.android.tungsten.setupwraith"
            | "com.google.android.apps.tv.dreamx"
            | "com.google.android.packageinstaller"
            | "com.google.android.permissioncontroller"
            | "com.google.android.modulemetadata"
            | "com.google.android.ext.services"
    ) || package.starts_with("com.google.android.overlay")
}

/// Go back to the factory version of a preinstalled app. Can't be undone
/// from here: the Play Store updates it again when it's opened.
pub async fn remove_system_update(conn: Connection, package: String) -> Result<String, String> {
    if keep_update(&package) {
        return Err(format!("{package} is needed by other apps; keeping its update"));
    }
    let out = sh(&conn, &format!("pm uninstall-system-updates {}", shell_quote(&package)), 60).await?;
    if out.contains("Failure") || out.contains("Error") || out.contains("Exception") {
        return Err(out.trim().to_owned());
    }
    Ok(if out.trim().is_empty() { format!("Removed the update of {package}.") } else { format!("{package}: {}", out.trim()) })
}

/// Delete a file from shared storage (`/sdcard` only).
pub async fn delete_file(conn: Connection, path: String) -> Result<String, String> {
    if !(path.starts_with("/sdcard/") || path.starts_with("/storage/emulated/0/")) || path.contains("/../") {
        return Err("only files in shared storage can be deleted".into());
    }
    sh(&conn, &format!("rm -f {}", shell_quote(&path)), 30).await?;
    Ok(format!("Deleted {path}."))
}

// ── Background apps ──────────────────────────────────────────────────────────

/// `am get-standby-bucket` numbers → names `am set-standby-bucket` takes.
pub fn bucket_name(code: &str) -> String {
    match code.trim() {
        "5" => "exempted",
        "10" => "active",
        "20" => "working_set",
        "30" => "frequent",
        "40" => "rare",
        "45" => "restricted",
        "50" => "never",
        other => other,
    }
    .to_owned()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackgroundApp {
    pub package: String,
    pub bucket: String,
    /// `RUN_ANY_IN_BACKGROUND` mode: `allow`, `ignore` or `default`.
    pub background: String,
    pub used_secs: Option<u64>,
}

/// `cmd appops get <pkg> <op>` → its mode (`default` when never set).
pub fn parse_appop(out: &str, op: &str) -> String {
    out.lines()
        .find_map(|l| l.trim().strip_prefix(&format!("{op}: ")))
        .and_then(|rest| rest.split([';', ' ']).next())
        .map_or_else(|| "default".to_owned(), str::to_owned)
}

/// Apps people see (TV launcher entries) and third-party apps.
const APPS: &str = "{ cmd package query-activities --brief -a android.intent.action.MAIN \
    -c android.intent.category.LEANBACK_LAUNCHER 2>/dev/null | grep / | cut -d/ -f1; \
    pm list packages -3 | sed 's/^package://'; } | sed 's/^ *//' | sort -u";

pub async fn background_apps(conn: Connection) -> Result<Vec<BackgroundApp>, String> {
    let script = format!(
        "for p in $({APPS}); do echo \"@@$p\"; am get-standby-bucket $p 2>&1; \
         cmd appops get $p RUN_ANY_IN_BACKGROUND 2>&1; done; \
         echo @@~usage; dumpsys usagestats 2>/dev/null | sed -n '/In-memory monthly stats/,/In-memory yearly stats/p' | grep package="
    );
    let raw = sh(&conn, &script, 120).await?;
    Ok(parse_background(&raw))
}

pub fn parse_background(raw: &str) -> Vec<BackgroundApp> {
    let sec = split_sections(raw);
    let usage = super::health::parse_usage(sec.get("~usage").map(String::as_str).unwrap_or(""));
    let have_usage = !usage.is_empty();
    let mut apps: Vec<BackgroundApp> = sec
        .iter()
        .filter(|(k, _)| !k.starts_with('~') && k.contains('.'))
        .map(|(package, out)| BackgroundApp {
            bucket: out.lines().next().map(bucket_name).unwrap_or_default(),
            background: parse_appop(out, "RUN_ANY_IN_BACKGROUND"),
            used_secs: have_usage.then(|| usage.get(package).copied().unwrap_or(0)),
            package: package.clone(),
        })
        .collect();
    // Least used first: the best candidates to restrict.
    apps.sort_by(|a, b| a.used_secs.cmp(&b.used_secs).then(a.package.cmp(&b.package)));
    apps
}

pub async fn set_bucket(conn: &Connection, package: &str, bucket: &str) -> Result<Change, String> {
    let q = shell_quote(package);
    let before = bucket_name(&sh(conn, &format!("am get-standby-bucket {q}"), 15).await?);
    let out = sh(conn, &format!("am set-standby-bucket {q} {}", shell_quote(bucket)), 15).await?;
    if out.contains("Exception") || out.contains("Error") {
        return Err(out.trim().to_owned());
    }
    let after = bucket_name(&sh(conn, &format!("am get-standby-bucket {q}"), 15).await?);
    Ok(Change::Bucket { package: package.into(), before, after })
}

pub async fn set_appop(conn: &Connection, package: &str, op: &str, mode: &str) -> Result<Change, String> {
    let (q, o) = (shell_quote(package), shell_quote(op));
    let before = parse_appop(&sh(conn, &format!("cmd appops get {q} {o}"), 15).await?, op);
    let out = sh(conn, &format!("cmd appops set {q} {o} {}", shell_quote(mode)), 15).await?;
    if out.contains("Exception") || out.contains("Error") {
        return Err(out.trim().to_owned());
    }
    let after = parse_appop(&sh(conn, &format!("cmd appops get {q} {o}"), 15).await?, op);
    Ok(Change::AppOp { package: package.into(), op: op.into(), before, after })
}

// ── Memory ───────────────────────────────────────────────────────────────────

fn config_value(out: &str) -> Option<String> {
    let v = out.trim();
    (!v.is_empty() && v != "null").then(|| v.to_owned())
}

pub async fn read_config(conn: &Connection, namespace: &str, key: &str) -> Result<Option<String>, String> {
    Ok(config_value(&sh(conn, &format!("device_config get {} {}", shell_quote(namespace), shell_quote(key)), 15).await?))
}

/// Set (or with `None`, delete) a `device_config` flag. The TV may reset
/// flags when it syncs with Google's servers.
pub async fn set_config(conn: &Connection, namespace: &str, key: &str, value: Option<&str>) -> Result<Change, String> {
    let before = read_config(conn, namespace, key).await?;
    let (ns, k) = (shell_quote(namespace), shell_quote(key));
    let cmd = match value {
        Some(v) => format!("device_config put {ns} {k} {}", shell_quote(v)),
        None => format!("device_config delete {ns} {k}"),
    };
    let out = sh(conn, &cmd, 15).await?;
    if out.contains("Exception") || out.contains("Error") {
        return Err(out.trim().to_owned());
    }
    let after = read_config(conn, namespace, key).await?;
    Ok(Change::DeviceConfig { namespace: namespace.into(), key: key.into(), before, after })
}

// ── Startup ──────────────────────────────────────────────────────────────────

/// Packages whose start-at-boot can't be turned off from here: the
/// framework, Google Play services and the TV maker's core services.
pub fn is_protected(package: &str) -> bool {
    package == "android"
        || ["com.android.", "com.google.android.gms", "com.google.android.gsf", "com.mediatek.", "com.droidlogic.", "com.amlogic."]
            .iter()
            .any(|p| package.starts_with(p))
        || package.contains("systemservice")
        || package.contains("vendorservice")
}

/// `query-receivers --brief` output → enabled receiver components.
pub fn parse_receivers(out: &str) -> Vec<String> {
    let mut seen = BTreeSet::new();
    out.lines()
        .map(str::trim)
        .filter(|l| l.contains('/') && !l.contains(' ') && !l.starts_with("priority"))
        .filter(|l| seen.insert((*l).to_owned()))
        .map(str::to_owned)
        .collect()
}

/// Components that run when the TV starts, by package.
pub async fn boot_receivers(conn: Connection) -> Result<BTreeMap<String, Vec<String>>, String> {
    let out = sh(&conn, "cmd package query-receivers --brief -a android.intent.action.BOOT_COMPLETED", 30).await?;
    let mut by_package: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for component in parse_receivers(&out) {
        let package = component.split('/').next().unwrap_or("").to_owned();
        by_package.entry(package).or_default().push(component);
    }
    Ok(by_package)
}

/// Switch one component on or off (`pm enable/disable`).
pub async fn set_component(conn: &Connection, component: &str, on: bool) -> Result<Change, String> {
    let package = component.split('/').next().unwrap_or("");
    if is_protected(package) {
        return Err(format!("{package} is a core service; leaving it alone"));
    }
    let out = sh(conn, &format!("pm {} {}", if on { "enable" } else { "disable" }, shell_quote(component)), 15).await?;
    if !out.contains("new state") {
        return Err(out.trim().to_owned());
    }
    let after = out.contains("new state: enabled") || out.contains("new state: default");
    Ok(Change::Component { component: component.into(), before: !on, after })
}

// ── Home app ─────────────────────────────────────────────────────────────────

/// `query-activities --brief` for HOME → launcher components, minus the
/// setup and fallback screens that aren't real home apps.
pub fn parse_home_activities(out: &str) -> Vec<String> {
    out.lines()
        .map(str::trim)
        .filter(|l| l.contains('/') && !l.contains(' ') && !l.starts_with("priority"))
        .filter(|l| !l.contains("FallbackHome") && !l.contains("setupwraith") && !l.contains("RecoveryActivity"))
        .map(str::to_owned)
        .collect()
}

/// (home apps installed, the one in use).
pub async fn home_apps(conn: Connection) -> Result<(Vec<String>, Option<String>), String> {
    let list = sh(&conn, "cmd package query-activities --brief -a android.intent.action.MAIN -c android.intent.category.HOME", 15).await?;
    let current = sh(&conn, "cmd package resolve-activity --brief -a android.intent.action.MAIN -c android.intent.category.HOME", 15).await?;
    Ok((parse_home_activities(&list), parse_home_activities(&current).into_iter().next()))
}

pub async fn set_home(conn: &Connection, component: &str) -> Result<Change, String> {
    let (_, before) = home_apps(conn.clone()).await?;
    let out = sh(conn, &format!("cmd package set-home-activity {}", shell_quote(component)), 15).await?;
    if !out.contains("Success") {
        return Err(out.trim().to_owned());
    }
    Ok(Change::Home { before: before.unwrap_or_default(), after: component.into() })
}

// ── Speed test ───────────────────────────────────────────────────────────────

/// `am start -W` → the launch time in ms.
pub fn parse_launch_ms(out: &str) -> Option<u32> {
    out.lines().find_map(|l| l.trim().strip_prefix("TotalTime:")).and_then(|v| v.trim().parse().ok())
}

/// The TV launcher entry of `package`.
pub async fn launch_component(conn: &Connection, package: &str) -> Option<String> {
    let q = shell_quote(package);
    for category in ["android.intent.category.LEANBACK_LAUNCHER", "android.intent.category.LAUNCHER"] {
        let out = sh(conn, &format!("cmd package resolve-activity --brief -a android.intent.action.MAIN -c {category} {q}"), 15)
            .await
            .ok()?;
        if let Some(c) = out.lines().map(str::trim).find(|l| l.contains('/') && !l.contains(' ')) {
            return Some(c.to_owned());
        }
    }
    None
}

/// Cold-start each app (stopped first) and time it, then go back to the
/// home screen. This DOES open apps on the TV.
pub async fn speed_test(conn: Connection, packages: Vec<String>, progress: crate::jobs::Progress) -> Vec<(String, Option<u32>)> {
    progress.reset(crate::jobs::Unit::Items, packages.len() as u64, "");
    let mut results = Vec::new();
    for package in packages {
        progress.set_detail(package.clone());
        let ms = match launch_component(&conn, &package).await {
            Some(component) => {
                let _ = sh(&conn, &format!("am force-stop {}", shell_quote(&package)), 15).await;
                tokio::time::sleep(Duration::from_millis(500)).await;
                let out = sh(&conn, &format!("am start -W -n {}", shell_quote(&component)), 60).await.unwrap_or_default();
                tokio::time::sleep(Duration::from_secs(2)).await;
                let _ = sh(&conn, "input keyevent 3", 10).await;
                tokio::time::sleep(Duration::from_secs(1)).await;
                parse_launch_ms(&out)
            }
            None => None,
        };
        results.push((package, ms));
        progress.add(1);
    }
    results
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_storage() {
        let raw = "@@df\nFilesystem 1K-blocks Used Available Use% Mounted on\n/dev/block/dm-5 4300000 3800000 500000 89% /data\n\
                   @@updated\npackage:/data/app/~~a==/com.google.android.katniss-b==/base.apk=com.google.android.katniss\n\
                   @@sizes\n120000000 /data/app/~~a==/com.google.android.katniss-b==/base.apk\n\
                   @@files\n52428800 /sdcard/Download/old.apk\n209715200 /sdcard/Movies/clip.mp4\n\
                   @@disk\nPackage Names: [\"com.a\",\"com.b\"]\nApp Sizes: [10,20]\nApp Data Sizes: [300,5]\nCache Sizes: [1,900]\n\
                   @@diskstamp\n-rw------- 1 system system 312 2026-10-27 19:30 /data/system/diskstats_cache.json\n\
                   @@usage\n";
        let s = parse_storage(raw);
        assert_eq!(s.free, 500000 * 1024);
        assert_eq!(s.updates[0].package, "com.google.android.katniss");
        assert_eq!(s.updates[0].bytes, 120_000_000);
        assert_eq!(s.updates[0].used_secs, None);
        assert_eq!(s.big_files[0].0, "/sdcard/Movies/clip.mp4");
        assert_eq!(s.apps[0].0, "com.b");
        assert_eq!(s.apps_measured.as_deref(), Some("2026-10-27 19:30"));
    }

    #[test]
    fn reads_background_state() {
        assert_eq!(bucket_name("45"), "restricted");
        assert_eq!(parse_appop("No operations.\nDefault mode: allow\n", "RUN_ANY_IN_BACKGROUND"), "default");
        assert_eq!(parse_appop("RUN_ANY_IN_BACKGROUND: ignore; time=+1m2s ago\n", "RUN_ANY_IN_BACKGROUND"), "ignore");
        let raw = "@@com.b.app\n30\nNo operations.\n@@com.a.app\n45\nRUN_ANY_IN_BACKGROUND: ignore\n@@~usage\n";
        let apps = parse_background(raw);
        assert_eq!(apps.len(), 2);
        assert_eq!((apps[0].package.as_str(), apps[0].bucket.as_str(), apps[0].background.as_str()), ("com.a.app", "restricted", "ignore"));
    }

    #[test]
    fn reads_receivers_and_home_apps() {
        let out = "122 receivers found:\n  Receiver #0:\n    priority=1000 preferredOrder=0\n    android/com.android.server.BootReceiver\n\
                   \x20 Receiver #1:\n    priority=0 preferredOrder=0\n    com.google.android.play.games/com.x.Boot\n";
        assert_eq!(parse_receivers(out), ["android/com.android.server.BootReceiver", "com.google.android.play.games/com.x.Boot"]);
        assert!(is_protected("android") && is_protected("com.mediatek.agent") && is_protected("com.sony.dtv.sonysystemservice"));
        assert!(!is_protected("com.google.android.play.games"));
        let home = "4 activities found:\n  Activity #0:\n    priority=2\n    com.google.android.apps.tv.launcherx/.home.HomeActivity\n\
                    \x20   com.google.android.tungsten.setupwraith/.RecoveryActivity\n    com.android.tv.settings/.system.FallbackHome\n";
        assert_eq!(parse_home_activities(home), ["com.google.android.apps.tv.launcherx/.home.HomeActivity"]);
        assert_eq!(parse_launch_ms("Status: ok\nLaunchState: COLD\nTotalTime: 1834\nWaitTime: 1850\n"), Some(1834));
    }

    /// Read-only: storage, background state, boot receivers and home apps.
    /// `STT_TEST_TV=ip:port cargo test live_optimize_reads -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn live_optimize_reads() {
        let serial = std::env::var("STT_TEST_TV").expect("set STT_TEST_TV=ip:port");
        let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
        rt.block_on(async {
            let conn = super::super::transport::connect(serial).await.expect("connect");
            let t = std::time::Instant::now();
            let s = storage(conn.clone()).await.expect("storage");
            println!("storage ({:?}): {} free of {}, {} updates, {} big files, {} apps", t.elapsed(), s.free, s.total, s.updates.len(), s.big_files.len(), s.apps.len());
            for u in s.updates.iter().take(5) {
                println!("  update {} {} bytes used {:?}s", u.package, u.bytes, u.used_secs);
            }
            let t = std::time::Instant::now();
            let bg = background_apps(conn.clone()).await.expect("background");
            println!("background ({:?}): {} apps", t.elapsed(), bg.len());
            for a in bg.iter().take(6) {
                println!("  {} bucket={} bg={} used={:?}", a.package, a.bucket, a.background, a.used_secs);
            }
            let boot = boot_receivers(conn.clone()).await.expect("boot");
            let open: usize = boot.iter().filter(|(p, _)| !is_protected(p)).map(|(_, v)| v.len()).sum();
            println!("boot: {} packages, {open} receivers in unprotected apps", boot.len());
            println!("home: {:?}", home_apps(conn.clone()).await.expect("home"));
            println!("cached procs: {:?}", read_config(&conn, "activity_manager", "max_cached_processes").await);
            println!("launch component smarttube: {:?}", launch_component(&conn, "org.smarttube.beta").await);
        });
    }
}
