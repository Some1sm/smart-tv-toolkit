//! Health Check (Performance tab): a read-only scan that turns the TV's state
//! into findings, each with the fixes that apply. Nothing changes on the TV
//! until the user runs a fix.

use std::collections::{BTreeSet, HashMap};
use std::time::Duration;

use super::sysinfo::{self, KB, fmt_bytes, parse_df, parse_meminfo, split_sections};
use super::{Connection, transport};

const SCRIPT: &str = "echo @@meminfo; cat /proc/meminfo; \
    echo @@df; df -k /data 2>/dev/null; \
    echo @@settings; for k in window_animation_scale transition_animation_scale \
    animator_duration_scale cached_apps_freezer; do echo $k=$(settings get global $k); done; \
    echo @@pss; dumpsys meminfo 2>/dev/null | sed -n '/Total [PR]SS by process/,/^$/p'; \
    echo @@cpu; dumpsys cpuinfo 2>/dev/null; \
    echo @@all_pkgs; pm list packages; \
    echo @@user_pkgs; pm list packages -3; \
    echo @@disabled; pm list packages -d; \
    echo @@disk; dumpsys diskstats 2>/dev/null; \
    echo @@usage; dumpsys usagestats 2>/dev/null \
    | sed -n '/In-memory monthly stats/,/In-memory yearly stats/p' | grep package=; \
    echo @@top; dumpsys activity activities 2>/dev/null | grep -m1 -E 'mResumedActivity|ResumedActivity'";

const ANIMATION_KEYS: [&str; 3] = ["window_animation_scale", "transition_animation_scale", "animator_duration_scale"];

/// Preinstalled services that run in the background on many TVs and can be
/// turned off without breaking the TV: (package, name, what it does, cost).
const KNOWN_SERVICES: [(&str, &str, &str, &str); 6] = [
    ("tv.samba.ssm", "Samba TV",
     "Recognises what is on screen to build viewing and ad data; runs constantly.",
     "Samba TV recommendations."),
    ("com.google.android.katniss", "Google Assistant", "Voice search and the Assistant.",
     "voice search and the remote's mic button."),
    ("com.google.android.apps.mediashell", "Chromecast built-in", "Lets phones cast to the TV.",
     "casting from phones and Chrome."),
    ("com.sony.dtv.airplayapp", "AirPlay", "Lets Apple devices cast to the TV.",
     "AirPlay from iPhone, iPad and Mac."),
    ("com.vewd.core.service", "Vewd", "Browser engine for broadcast red-button (HbbTV) extras.",
     "red-button extras on broadcast channels, and possibly some built-in web apps."),
    ("com.uei.quicksetsdk.sony", "UEI QuickSet",
     "Lets the TV remote control other devices (set-top boxes, soundbars).",
     "controlling other devices with the TV remote."),
];

fn known(pkg: &str) -> Option<&'static (&'static str, &'static str, &'static str, &'static str)> {
    KNOWN_SERVICES.iter().find(|s| s.0 == pkg)
}

/// Display name derived from the package id (`com.netflix.ninja` → "Netflix
/// Ninja"); real app labels live in APK resources that ADB can't read.
pub fn pretty_pkg_name(pkg: &str) -> String {
    let parts: Vec<&str> = pkg.split('.').filter(|p| !p.is_empty()).collect();
    let mut words: Vec<&str> = parts.iter().skip(1).copied().filter(|w| *w != "android").collect();
    if words.is_empty() {
        words = parts.last().copied().into_iter().collect();
    }
    words
        .iter()
        .map(|w| {
            let mut c = w.chars();
            c.next().map(|f| f.to_uppercase().collect::<String>() + &c.as_str().to_lowercase()).unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn app_label(pkg: &str) -> String {
    known(pkg).map(|s| s.1.to_owned()).unwrap_or_else(|| pretty_pkg_name(pkg))
}

// ── parsers ──────────────────────────────────────────────────────────────────

pub fn parse_pkg_list(text: &str) -> BTreeSet<String> {
    text.lines().filter_map(|l| l.trim().strip_prefix("package:")).map(str::to_owned).collect()
}

pub fn parse_settings(text: &str) -> HashMap<String, String> {
    text.lines()
        .filter_map(|l| l.trim().split_once('='))
        .map(|(k, v)| (k.to_owned(), v.trim().to_owned()))
        .collect()
}

/// `dumpsys meminfo` process totals → `{package: bytes}`. Prefers the PSS
/// table (Android 13+ prints RSS first); `com.app:service` processes are
/// summed under their package.
pub fn parse_pss(text: &str) -> HashMap<String, u64> {
    let mut tables: HashMap<&str, HashMap<String, u64>> = HashMap::new();
    let mut current: Option<&str> = None;
    for line in text.lines() {
        let t = line.trim();
        if let Some(kind) = t.strip_prefix("Total ").and_then(|r| r.strip_suffix(" by process:")) {
            current = Some(if kind == "PSS" { "PSS" } else { "RSS" });
            tables.entry(current.unwrap()).or_default();
            continue;
        }
        let (Some(table), Some((size, rest))) = (current, t.split_once("K: ")) else { continue };
        let Ok(kb) = size.replace(',', "").parse::<u64>() else { continue };
        let Some(process) = rest.split_once(" (pid").map(|(p, _)| p) else { continue };
        let pkg = process.split(':').next().unwrap_or(process).to_owned();
        *tables.entry(table).or_default().entry(pkg).or_default() += kb * KB;
    }
    tables.remove("PSS").filter(|t| !t.is_empty()).or_else(|| tables.remove("RSS")).unwrap_or_default()
}

/// `dumpsys cpuinfo` → (total busy %, [(pct, package or process)]).
pub fn parse_cpuinfo(text: &str) -> (Option<f32>, Vec<(f32, String)>) {
    let mut total = None;
    let mut procs = Vec::new();
    for line in text.lines() {
        let Some((pct, rest)) = line.trim().split_once("% ") else { continue };
        let Ok(pct) = pct.parse::<f32>() else { continue };
        if rest.starts_with("TOTAL:") {
            total = Some(pct);
        } else if let Some((pid_name, _)) = rest.split_once(": ")
            && let Some((pid, name)) = pid_name.split_once('/')
            && pid.chars().all(|c| c.is_ascii_digit())
        {
            procs.push((pct, name.split(':').next().unwrap_or(name).to_owned()));
        }
    }
    (total, procs)
}

/// UsageStats elapsed time (`35:17:55`, `25:54`) → seconds.
pub fn usage_secs(s: &str) -> u64 {
    s.split(':').try_fold(0u64, |acc, p| p.parse::<u64>().ok().map(|v| acc * 60 + v)).unwrap_or(0)
}

/// Monthly `dumpsys usagestats` lines → `{package: seconds on screen}`.
pub fn parse_usage(text: &str) -> HashMap<String, u64> {
    let mut usage: HashMap<String, u64> = HashMap::new();
    for line in text.lines() {
        let Some(pkg) = line.split("package=").nth(1).and_then(|r| r.split_whitespace().next()) else { continue };
        let Some(time) = line.split("totalTimeUsed=\"").nth(1).and_then(|r| r.split('"').next()) else { continue };
        let secs = usage_secs(time);
        let e = usage.entry(pkg.to_owned()).or_default();
        *e = (*e).max(secs);
    }
    usage
}

/// `dumpsys diskstats` → [(package, apk, data, cache)] in bytes. Filled by a
/// periodic system job, so it can be up to a day old.
pub fn parse_diskstats(text: &str) -> Vec<(String, u64, u64, u64)> {
    let arr = |name: &str| -> Vec<String> {
        text.lines()
            .find_map(|l| l.strip_prefix(&format!("{name}: [")))
            .and_then(|r| r.strip_suffix(']'))
            .map(|r| r.split(',').map(|v| v.trim().trim_matches('"').to_owned()).filter(|v| !v.is_empty()).collect())
            .unwrap_or_default()
    };
    let (names, apps, data, cache) = (arr("Package Names"), arr("App Sizes"), arr("App Data Sizes"), arr("Cache Sizes"));
    names
        .into_iter()
        .enumerate()
        .filter_map(|(i, n)| {
            Some((n, apps.get(i)?.parse().ok()?, data.get(i)?.parse().ok()?, cache.get(i)?.parse().ok()?))
        })
        .collect()
}

/// The app on screen, from `mResumedActivity: ActivityRecord{… u0 pkg/.Act …}`.
pub fn resumed_package(text: &str) -> String {
    text.split_whitespace()
        .skip_while(|w| !(w.starts_with('u') && w[1..].chars().all(|c| c.is_ascii_digit()) && w.len() > 1))
        .nth(1)
        .and_then(|w| w.split('/').next())
        .unwrap_or("")
        .to_owned()
}

// ── findings ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Bad,
    Warn,
    Ok,
    Info,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fix {
    ClearCaches,
    Stop(String),
    Disable(String),
    Uninstall(String),
    SpeedUpAnimations,
    EnableFreezer,
}

impl Fix {
    pub fn label(&self) -> &'static str {
        match self {
            Fix::ClearCaches => "Clear All App Caches",
            Fix::Stop(_) => "Stop",
            Fix::Disable(_) => "Disable",
            Fix::Uninstall(_) => "Uninstall",
            Fix::SpeedUpAnimations => "Speed Up Animations (0.5×)",
            Fix::EnableFreezer => "Enable App Freezer",
        }
    }

    /// The confirmation shown before running; None for harmless settings.
    pub fn confirmation(&self) -> Option<(String, String)> {
        let name = |pkg: &str| format!("{} ({pkg})", app_label(pkg));
        match self {
            Fix::ClearCaches => Some((
                "Clear All App Caches".into(),
                "Clear the cache of every app on the TV?\n\nLogins and settings are kept; apps rebuild \
                 their caches, so the first launch of each may be a little slower. Streaming apps \
                 lose any video they had cached."
                    .into(),
            )),
            Fix::Stop(p) => Some((
                "Stop App".into(),
                format!("Force-stop {}?\n\nIt closes now and starts again next time you open it. A VPN will disconnect.", name(p)),
            )),
            Fix::Disable(p) => Some((
                "Disable Service".into(),
                format!("Disable {}?\n\nYou can turn it back on any time from Packages → Enable.", name(p)),
            )),
            Fix::Uninstall(p) => Some(("Uninstall App".into(), format!("Uninstall {}?\n\nThis removes the app and its data.", name(p)))),
            Fix::SpeedUpAnimations | Fix::EnableFreezer => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Finding {
    pub severity: Severity,
    pub title: &'static str,
    pub detail: String,
    pub fixes: Vec<Fix>,
    /// Per-app lines, each with its own fixes.
    pub items: Vec<(String, Vec<Fix>)>,
}

const MB: u64 = 1024 * 1024;

pub fn findings(sec: &HashMap<String, String>) -> Vec<Finding> {
    let s = |k: &str| sec.get(k).map(String::as_str).unwrap_or("");
    let all_pkgs = parse_pkg_list(s("all_pkgs"));
    let user_pkgs = parse_pkg_list(s("user_pkgs"));
    let disabled = parse_pkg_list(s("disabled"));
    let foreground = resumed_package(s("top"));
    let manageable = |p: &str| (user_pkgs.contains(p) || known(p).is_some()) && p != foreground;
    let mut out = Vec::new();

    // Storage: `df -k /data` returns the one filesystem holding /data, but
    // its mount can be a bind like /data/user/0, so take the row as is.
    let mut caches: Vec<(u64, String)> =
        parse_diskstats(s("disk")).into_iter().filter(|r| r.3 >= 50 * MB).map(|r| (r.3, r.0)).collect();
    caches.sort_by(|a, b| b.cmp(a));
    caches.truncate(5);
    if let Some(row) = parse_df(s("df")).into_iter().next().filter(|r| r.total > 0) {
        let free_pct = row.avail as f64 / row.total as f64 * 100.0;
        let severity = if free_pct < 15.0 { Severity::Bad } else if free_pct < 25.0 { Severity::Warn } else { Severity::Ok };
        let mut detail = format!("{} free of {} ({free_pct:.0}%).", fmt_bytes(row.avail), fmt_bytes(row.total));
        if severity != Severity::Ok {
            detail.push_str(" With this little space Android skips its background app optimisation, and the flash storage gets slower.");
        }
        if !caches.is_empty() {
            let list: Vec<String> = caches.iter().map(|(c, p)| format!("{} {}", app_label(p), fmt_bytes(*c))).collect();
            detail.push_str(&format!("\nLargest app caches: {}", list.join(", ")));
        }
        let fixes = if severity != Severity::Ok || !caches.is_empty() { vec![Fix::ClearCaches] } else { vec![] };
        out.push(Finding { severity, title: "Storage", detail, fixes, items: vec![] });
    }

    // Memory
    let mem = parse_meminfo(s("meminfo"));
    if let Some(&total) = mem.get("MemTotal").filter(|t| **t > 0) {
        let avail = mem.get("MemAvailable").copied().unwrap_or_else(|| {
            mem.get("MemFree").copied().unwrap_or(0) + mem.get("Cached").copied().unwrap_or(0)
        });
        let avail_pct = avail as f64 / total as f64 * 100.0;
        let (swap_total, swap_free) = (mem.get("SwapTotal").copied().unwrap_or(0), mem.get("SwapFree").copied().unwrap_or(0));
        let swap_full = swap_total > 0 && (swap_free as f64) < swap_total as f64 * 0.05;
        let severity = if avail_pct < 15.0 || (swap_full && avail_pct < 25.0) {
            Severity::Bad
        } else if avail_pct < 25.0 || swap_full {
            Severity::Warn
        } else {
            Severity::Ok
        };
        let mut detail = format!("{} available of {} ({avail_pct:.0}%).", fmt_bytes(avail), fmt_bytes(total));
        if swap_total > 0 {
            detail.push_str(&format!(" Swap {} of {} used.", fmt_bytes(swap_total - swap_free), fmt_bytes(swap_total)));
        }
        if severity != Severity::Ok {
            detail.push_str(" Android has to close and reload apps when you switch, which feels like lag.");
        }
        let mut stoppable: Vec<(u64, String)> = parse_pss(s("pss"))
            .into_iter()
            .filter(|(p, size)| manageable(p) && !disabled.contains(p) && *size >= 20 * MB)
            .map(|(p, size)| (size, p))
            .collect();
        stoppable.sort_by(|a, b| b.cmp(a));
        let items: Vec<(String, Vec<Fix>)> = stoppable
            .into_iter()
            .take(6)
            .map(|(size, p)| (format!("{}  ({p}) — {}", app_label(&p), fmt_bytes(size)), vec![Fix::Stop(p)]))
            .collect();
        if !items.is_empty() {
            detail.push_str("\nBiggest background apps you can stop (they restart when opened):");
        }
        out.push(Finding { severity, title: "Memory", detail, fixes: vec![], items });
    }

    // CPU
    let (busy, procs) = parse_cpuinfo(s("cpu"));
    if let Some(busy) = busy {
        let severity = if busy >= 70.0 { Severity::Warn } else { Severity::Ok };
        let mut detail = format!(
            "{busy:.0}% busy on average over the last minute or so. (The Linux load average is \
             misleading on TV chips: it counts idle driver threads.)"
        );
        let items: Vec<(String, Vec<Fix>)> = procs
            .into_iter()
            .filter(|(pct, p)| *pct >= 3.0 && manageable(p))
            .take(5)
            .map(|(pct, p)| (format!("{}  ({p}) — {pct:.0}% CPU", app_label(&p)), vec![Fix::Stop(p)]))
            .collect();
        if !items.is_empty() {
            detail.push_str("\nBackground apps using noticeable CPU:");
        }
        out.push(Finding { severity, title: "CPU", detail, fixes: vec![], items });
    }

    // Unused apps
    let usage = parse_usage(s("usage"));
    if !usage.is_empty() && !user_pkgs.is_empty() {
        let unused: Vec<&String> = user_pkgs
            .iter()
            .filter(|p| usage.get(*p).copied().unwrap_or(0) < 60 && !disabled.contains(*p) && **p != foreground)
            .collect();
        let detail = if unused.is_empty() {
            "Every installed app has been used this month.".to_owned()
        } else {
            format!(
                "{} installed app(s) had less than a minute on screen this month. Apps that only work \
                 in the background (VPNs, file servers) can show up here even if you rely on them.",
                unused.len()
            )
        };
        out.push(Finding {
            severity: if unused.is_empty() { Severity::Ok } else { Severity::Warn },
            title: "Unused Apps",
            detail,
            fixes: vec![],
            items: unused.into_iter().map(|p| (format!("{}  ({p})", app_label(p)), vec![Fix::Uninstall(p.clone())])).collect(),
        });
    }

    // Known background services
    let services: Vec<(String, Vec<Fix>)> = KNOWN_SERVICES
        .iter()
        .filter(|(p, ..)| all_pkgs.contains(*p) && !disabled.contains(*p))
        .map(|(p, name, what, cost)| (format!("{name}: {what}\nTurning it off loses {cost}"), vec![Fix::Disable((*p).to_owned())]))
        .collect();
    if !services.is_empty() {
        out.push(Finding {
            severity: Severity::Info,
            title: "Optional Background Services",
            detail: "Preinstalled services that stay loaded. Only turn off what you don't use; re-enable \
                     any of them from Packages → Enable."
                .into(),
            fixes: vec![],
            items: services,
        });
    }

    // Quick tweaks already applied?
    let settings = parse_settings(s("settings"));
    if !settings.is_empty() {
        // "null" means never set, i.e. the default 1×.
        let scale = |k: &str| settings.get(k).and_then(|v| v.parse::<f32>().ok()).unwrap_or(1.0);
        let anim_ok = ANIMATION_KEYS.iter().all(|k| scale(k) <= 0.5);
        let freezer_ok = settings.get("cached_apps_freezer").is_some_and(|v| v == "enabled");
        let mut fixes = vec![];
        if !anim_ok {
            fixes.push(Fix::SpeedUpAnimations);
        }
        if !freezer_ok {
            fixes.push(Fix::EnableFreezer);
        }
        out.push(Finding {
            severity: if fixes.is_empty() { Severity::Ok } else { Severity::Info },
            title: "Quick Tweaks",
            detail: format!(
                "Animations: {}  App freezer: {}",
                if anim_ok { "already sped up." } else { "normal speed." },
                if freezer_ok { "on." } else { "off." }
            ),
            fixes,
            items: vec![],
        });
    }
    out
}

/// Run the read-only scan.
pub async fn scan(conn: Connection) -> Result<Vec<Finding>, String> {
    let raw = conn.shell(SCRIPT, Duration::from_secs(90)).await.map_err(|e| e.to_string())?;
    Ok(findings(&split_sections(&raw)))
}

async fn data_free(conn: &Connection) -> Option<u64> {
    let out = conn.shell("df -k /data", transport::SHORT).await.ok()?;
    sysinfo::parse_df(&out).first().map(|r| r.avail)
}

/// Apply one fix; returns a line for the log.
pub async fn apply(conn: Connection, fix: Fix) -> Result<String, String> {
    let run = |cmd: String, secs: u64| {
        let conn = conn.clone();
        async move { conn.shell(cmd, Duration::from_secs(secs)).await.map_err(|e| e.to_string()) }
    };
    let out = match &fix {
        Fix::ClearCaches => {
            let before = data_free(&conn).await;
            // trim-caches deletes cache only until the requested amount of
            // space is free, so "0" frees nothing. Ask for far more than any
            // TV has (1 TiB) to clear every app's cache.
            let out = run(format!("pm trim-caches {}", 1u64 << 40), 180).await?;
            let after = data_free(&conn).await;
            return Ok(match (before, after) {
                (Some(b), Some(a)) => format!("Freed {}; {} now free.", fmt_bytes(a.saturating_sub(b)), fmt_bytes(a)),
                _ if !out.is_empty() => out,
                _ => "Caches cleared.".into(),
            });
        }
        Fix::Stop(p) => run(format!("am force-stop {p}"), 30).await?,
        Fix::Disable(p) => run(format!("pm disable-user --user 0 {p}"), 30).await?,
        Fix::Uninstall(p) => run(format!("pm uninstall --user 0 {p}"), 60).await?,
        Fix::SpeedUpAnimations => {
            let cmds: Vec<String> = ANIMATION_KEYS.iter().map(|k| format!("settings put global {k} 0.5")).collect();
            run(cmds.join("; "), 30).await?
        }
        Fix::EnableFreezer => run("settings put global cached_apps_freezer enabled".into(), 30).await?,
    };
    Ok(if out.is_empty() { format!("{}: done.", fix.label()) } else { format!("{}: {out}", fix.label()) })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Mirrors real Android TV shells (dumpsys formats from Android 12).
    const SCAN: &str = r#"@@meminfo
MemTotal:        3000000 kB
MemAvailable:     420000 kB
SwapTotal:        102400 kB
SwapFree:              0 kB
@@df
Filesystem            1K-blocks    Used Available Use% Mounted on
/dev/block/mmcblk0p50   4000000 3400000    600000  85% /data/user/0
@@settings
window_animation_scale=0.5
transition_animation_scale=0.5
animator_duration_scale=null
cached_apps_freezer=enabled
@@pss
Total PSS by process:
    141,011K: system (pid 3079)
    102,108K: com.example.player (pid 6028 / activities)
     94,549K: com.example.vpn (pid 10272)
     60,000K: tv.samba.ssm (pid 10154)
     30,000K: tv.samba.ssm:worker (pid 10155)
      9,000K: com.example.tiny (pid 1)

@@cpu
Load: 53.08 / 53.06 / 53.15
CPU usage from 62871ms to 14024ms ago:
  25% 3557/aud_cpupost: 11% user + 14% kernel
  6.9% 10154/tv.samba.ssm: 6.1% user + 0.7% kernel / faults: 4432 minor
  4% 7172/com.example.player: 2.5% user + 1.2% kernel
  1% 9/com.example.vpn: 1% user + 0% kernel
28% TOTAL: 12% user + 15% kernel + 0% iowait
@@all_pkgs
package:android
package:tv.samba.ssm
package:com.google.android.katniss
package:com.example.player
package:com.example.vpn
package:com.example.unused
package:com.example.tiny
@@user_pkgs
package:com.example.player
package:com.example.vpn
package:com.example.unused
package:com.example.tiny
@@disabled
package:com.google.android.katniss
@@disk
Data-Free: 600000K / 4000000K total = 15% free
Package Names: ["com.example.player","com.example.vpn"]
App Sizes: [100000000,20000000]
App Data Sizes: [2300000000,1000000]
Cache Sizes: [2200000000,500000]
@@usage
        package=com.example.player totalTimeUsed="35:17:55" lastTimeUsed="2026-10-25 22:56:31"
        package=com.example.vpn totalTimeUsed="07:14" lastTimeUsed="2026-10-25 22:56:50"
        package=com.example.unused totalTimeUsed="00:00" lastTimeUsed="1970-01-01 01:00:00"
@@top
    mResumedActivity: ActivityRecord{d085aae u0 com.example.player/.MainActivity t1452}
"#;

    fn by_title() -> HashMap<&'static str, Finding> {
        findings(&split_sections(SCAN)).into_iter().map(|f| (f.title, f)).collect()
    }

    fn item_pkgs(f: &Finding) -> Vec<String> {
        f.items
            .iter()
            .map(|(_, fixes)| match &fixes[0] {
                Fix::Stop(p) | Fix::Disable(p) | Fix::Uninstall(p) => p.clone(),
                other => panic!("unexpected {other:?}"),
            })
            .collect()
    }

    /// Read-only Info + Health scan of a real TV (applies no fixes):
    /// `STT_TEST_TV=ip:port cargo test live_info_and_health -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn live_info_and_health() {
        let serial = std::env::var("STT_TEST_TV").expect("set STT_TEST_TV=ip:port");
        let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
        rt.block_on(async {
            let conn = transport::connect(serial).await.expect("connect");
            let t = std::time::Instant::now();
            let info = sysinfo::collect(conn.clone()).await.expect("info");
            println!("--- Info ({:?})\n{}", t.elapsed(), sysinfo::to_text(&info));
            let t = std::time::Instant::now();
            for f in scan(conn).await.expect("scan") {
                println!("--- [{:?}] {}: {}", f.severity, f.title, f.detail.replace('\n', " / "));
                for fix in &f.fixes {
                    println!("    fix: {}", fix.label());
                }
                for (text, fixes) in &f.items {
                    println!("    - {} [{}]", text.replace('\n', " / "), fixes[0].label());
                }
            }
            println!("--- scan took {:?}", t.elapsed());
        });
    }

    #[test]
    fn parsers() {
        let sec = split_sections(SCAN);
        assert_eq!(parse_pss(&sec["pss"])["tv.samba.ssm"], 90_000 * KB);
        let (total, procs) = parse_cpuinfo(&sec["cpu"]);
        assert_eq!(total, Some(28.0));
        assert!(procs.contains(&(6.9, "tv.samba.ssm".into())));
        assert_eq!(usage_secs("35:17:55"), 35 * 3600 + 17 * 60 + 55);
        assert_eq!(usage_secs("07:14"), 434);
        assert_eq!(parse_diskstats(&sec["disk"])[0], ("com.example.player".into(), 100_000_000, 2_300_000_000, 2_200_000_000));
        assert_eq!(resumed_package(&sec["top"]), "com.example.player");
        assert_eq!(pretty_pkg_name("com.netflix.ninja"), "Netflix Ninja");
        assert_eq!(pretty_pkg_name("com.google.android.youtube.tvkids"), "Google Youtube Tvkids");
    }

    #[test]
    fn rss_fallback_and_pss_preferred() {
        let both = "Total RSS by process:\n    50,000K: com.a (pid 1)\n\nTotal PSS by process:\n    20,000K: com.a (pid 1)\n";
        assert_eq!(parse_pss(both)["com.a"], 20_000 * KB);
        assert_eq!(parse_pss("Total RSS by process:\n    50,000K: com.a (pid 1)\n")["com.a"], 50_000 * KB);
    }

    #[test]
    fn storage_uses_bind_mounted_data_row() {
        let f = &by_title()["Storage"];
        assert_eq!(f.severity, Severity::Warn); // exactly 15% free; Bad is < 15%
        assert!(f.detail.contains("2.0 GB"));
        assert_eq!(f.fixes, [Fix::ClearCaches]);
    }

    #[test]
    fn memory_lists_stoppable_apps_but_not_foreground() {
        let f = &by_title()["Memory"];
        assert_eq!(f.severity, Severity::Bad);
        assert_eq!(item_pkgs(f), ["com.example.vpn", "tv.samba.ssm"]);
    }

    #[test]
    fn cpu_names_background_hogs() {
        let f = &by_title()["CPU"];
        assert_eq!(f.severity, Severity::Ok);
        assert_eq!(item_pkgs(f), ["tv.samba.ssm"]);
    }

    #[test]
    fn unused_apps_skip_foreground_and_used() {
        assert_eq!(item_pkgs(&by_title()["Unused Apps"]), ["com.example.tiny", "com.example.unused"]);
    }

    #[test]
    fn known_services_skip_disabled_ones() {
        assert_eq!(item_pkgs(&by_title()["Optional Background Services"]), ["tv.samba.ssm"]);
    }

    #[test]
    fn quick_tweaks_treat_null_as_default_speed() {
        let f = &by_title()["Quick Tweaks"];
        assert_eq!(f.fixes, [Fix::SpeedUpAnimations]);
        assert!(f.detail.contains("App freezer: on."));
    }

    #[test]
    fn no_usage_data_skips_unused_finding() {
        let mut sec = split_sections(SCAN);
        sec.insert("usage".into(), String::new());
        assert!(findings(&sec).iter().all(|f| f.title != "Unused Apps"));
    }
}
