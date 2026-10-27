//! The Packages tab: listing installed apps and acting on a selection.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::health::pretty_pkg_name;
use super::bloat::{Rating, Verdict};
use super::changes::{self, Outcome};
use super::sysinfo::split_sections;
use crate::journal::{Change, PkgState};
use crate::jobs::{Progress, Unit};
use super::Connection;

const PLAY_STORE: &str = "com.android.vending";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListKind {
    All,
    System,
    User,
    /// Removed for the current user but still on the system image, so they
    /// can be restored (e.g. preinstalled apps that were uninstalled).
    Uninstalled,
    PlayStore,
    Sideloaded,
    /// Preinstalled apps rated as bloatware.
    Bloatware,
    /// Preinstalled extras that are fine to remove if unused.
    Optional,
}

impl ListKind {
    pub const ALL: [ListKind; 8] = [
        ListKind::All,
        ListKind::System,
        ListKind::User,
        ListKind::PlayStore,
        ListKind::Sideloaded,
        ListKind::Bloatware,
        ListKind::Optional,
        ListKind::Uninstalled,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ListKind::All => "All",
            ListKind::System => "System",
            ListKind::User => "User",
            ListKind::Uninstalled => "Uninstalled",
            ListKind::PlayStore => "Play Store",
            ListKind::Sideloaded => "Sideloaded",
            ListKind::Bloatware => "Bloatware",
            ListKind::Optional => "Optional",
        }
    }

    pub fn matches(self, p: &Package) -> bool {
        let system = matches!(p.origin, Origin::System | Origin::Oem);
        match self {
            ListKind::All => p.installed,
            ListKind::System => p.installed && system,
            ListKind::User => p.installed && !system,
            ListKind::PlayStore => p.installed && p.installer == PLAY_STORE,
            ListKind::Sideloaded => p.installed && p.origin == Origin::Sideloaded,
            ListKind::Bloatware => p.installed && p.rating() == Some(Rating::Bloat),
            ListKind::Optional => p.installed && p.rating() == Some(Rating::Optional),
            ListKind::Uninstalled => !p.installed,
        }
    }
}

/// Where a package came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    GooglePlay,
    Sideloaded,
    System,
    /// Preinstalled by the TV maker (its name is in the package id).
    Oem,
}

impl Origin {
    pub fn label(self) -> &'static str {
        match self {
            Origin::GooglePlay => "Google Play",
            Origin::Sideloaded => "Sideloaded",
            Origin::System => "System",
            Origin::Oem => "System (OEM)",
        }
    }
}

/// The grid's groups, in display order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Group {
    User,
    Sideloaded,
    System,
    Uninstalled,
}

impl Group {
    pub const ALL: [Group; 4] = [Group::User, Group::Sideloaded, Group::System, Group::Uninstalled];

    pub fn title(self) -> &'static str {
        match self {
            Group::User => "User Installed Applications",
            Group::Sideloaded => "Sideloaded Applications",
            Group::System => "System & Preinstalled",
            Group::Uninstalled => "Uninstalled (Restorable)",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Package {
    pub id: String,
    pub name: String,
    pub version: String,
    pub origin: Origin,
    /// Installer package, or `null`.
    pub installer: String,
    /// The base APK on the TV.
    pub path: String,
    /// Size of the base APK in bytes.
    pub size: Option<u64>,
    /// False for packages removed for this user but still on the image.
    pub installed: bool,
    pub disabled: bool,
    pub running: bool,
    /// Bloatware rating, worked out once when listed.
    pub verdict: Option<Verdict>,
}

impl Package {
    /// The bloatware rating of a preinstalled app (see [`super::bloat`]).
    pub fn verdict(&self) -> Option<Verdict> {
        self.verdict
    }

    pub fn rating(&self) -> Option<Rating> {
        self.verdict.map(|v| v.rating)
    }

    pub fn group(&self) -> Group {
        match (self.installed, self.origin) {
            (false, _) => Group::Uninstalled,
            (true, Origin::GooglePlay) => Group::User,
            (true, Origin::Sideloaded) => Group::Sideloaded,
            (true, Origin::System | Origin::Oem) => Group::System,
        }
    }
}

/// `pm list packages` output → package ids, in order, without duplicates.
pub fn parse_list(text: &str) -> Vec<String> {
    let mut seen = BTreeSet::new();
    text.lines()
        .filter_map(|l| l.trim().strip_prefix("package:"))
        .map(|rest| rest.split_whitespace().next().unwrap_or("").to_owned())
        .filter(|p| !p.is_empty() && seen.insert(p.clone()))
        .collect()
}

/// `pm list packages -i` → `{package: installer}` (`null` when none).
#[cfg(test)]
pub fn parse_installers(text: &str) -> HashMap<String, String> {
    parse_full_list(text).into_iter().map(|(id, _, installer)| (id, installer)).collect()
}

/// `pm list packages -f -i` → `(package, base APK path, installer)`. The path
/// may itself contain `=` (`~~b9d==`), so the package is after the last one.
pub fn parse_full_list(text: &str) -> Vec<(String, String, String)> {
    let mut seen = BTreeSet::new();
    text.lines()
        .filter_map(|l| {
            let rest = l.trim().strip_prefix("package:")?;
            let mut parts = rest.split_whitespace();
            let first = parts.next()?;
            let (path, id) = first.rsplit_once('=').unwrap_or(("", first));
            let installer = parts.find_map(|p| p.strip_prefix("installer=")).unwrap_or("null");
            seen.insert(id.to_owned()).then(|| (id.to_owned(), path.to_owned(), installer.to_owned()))
        })
        .collect()
}

/// One `dumpsys package packages` pass → `{package: versionName}`. The first
/// block wins: "Hidden system packages:" repeats some packages later with
/// the factory version.
pub fn parse_versions(text: &str) -> HashMap<String, String> {
    let mut found = HashMap::new();
    let mut current: Option<String> = None;
    for line in text.lines().map(str::trim) {
        if let Some(rest) = line.strip_prefix("Package [") {
            current = rest.split(']').next().map(str::to_owned);
        } else if let Some(version) = line.strip_prefix("versionName=")
            && let Some(pkg) = current.take()
        {
            found.entry(pkg).or_insert_with(|| version.to_owned());
        }
    }
    found
}

/// `stat -c '%s %n'` lines → `{path: bytes}`.
fn parse_sizes(text: &str) -> HashMap<String, u64> {
    text.lines()
        .filter_map(|l| {
            let (size, path) = l.trim().split_once(' ')?;
            Some((path.to_owned(), size.parse().ok()?))
        })
        .collect()
}

/// Running process names → packages with a process (`pkg` or `pkg:service`).
fn running_packages(ps: &str) -> BTreeSet<String> {
    ps.lines().map(|l| l.trim().split(':').next().unwrap_or("").to_owned()).filter(|n| n.contains('.')).collect()
}

async fn sh(conn: &Connection, cmd: &str, secs: u64) -> Result<String, String> {
    conn.shell(cmd, Duration::from_secs(secs)).await.map_err(|e| e.to_string())
}

const INVENTORY: &str = "\
    echo @@all; pm list packages -f -i -u; \
    echo @@installed; pm list packages; \
    echo @@system; pm list packages -s -u; \
    echo @@disabled; pm list packages -d; \
    echo @@ps; ps -A -o NAME; \
    echo @@versions; dumpsys package packages | grep -E '^ *(Package \\[|versionName=)'; \
    echo @@sizes; pm list packages -f -u | sed -e 's/^package://' -e 's/=[^=]*$//' | xargs stat -c '%s %n' 2>/dev/null";

/// Build the inventory from the script's sections. `maker` is the TV's
/// manufacturer, used to tell its own preinstalled apps apart.
pub fn parse_inventory(raw: &str, maker: &str) -> Vec<Package> {
    let sec = split_sections(raw);
    let get = |k: &str| sec.get(k).map(String::as_str).unwrap_or("");
    let installed: BTreeSet<String> = parse_list(get("installed")).into_iter().collect();
    let system: BTreeSet<String> = parse_list(get("system")).into_iter().collect();
    let disabled: BTreeSet<String> = parse_list(get("disabled")).into_iter().collect();
    let running = running_packages(get("ps"));
    let versions = parse_versions(get("versions"));
    let sizes = parse_sizes(get("sizes"));
    let maker = maker.trim().to_lowercase();
    let mut list: Vec<Package> = parse_full_list(get("all"))
        .into_iter()
        .map(|(id, path, installer)| {
            let origin = if system.contains(&id) {
                if !maker.is_empty() && id.split('.').any(|part| part == maker) { Origin::Oem } else { Origin::System }
            } else if installer == PLAY_STORE {
                Origin::GooglePlay
            } else {
                Origin::Sideloaded
            };
            let preinstalled = matches!(origin, Origin::System | Origin::Oem);
            Package {
                verdict: super::bloat::rate(&id, preinstalled),
                name: pretty_pkg_name(&id),
                version: versions.get(&id).cloned().unwrap_or_default(),
                origin,
                installer,
                size: sizes.get(&path).copied(),
                path,
                installed: installed.contains(&id),
                disabled: disabled.contains(&id),
                running: running.contains(&id),
                id,
            }
        })
        .collect();
    list.sort_by(|a, b| a.id.cmp(&b.id));
    list
}

/// Every package on the TV (installed or restorable), in one pass.
pub async fn inventory(conn: Connection, maker: String) -> Result<Vec<Package>, String> {
    let raw = sh(&conn, INVENTORY, 120).await?;
    let list = parse_inventory(&raw, &maker);
    if list.is_empty() { Err("the TV listed no packages".into()) } else { Ok(list) }
}

/// What the "⋯" inspector shows about one package.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Details {
    pub uid: Option<u32>,
    pub min_sdk: Option<u32>,
    pub target_sdk: Option<u32>,
    pub installer: String,
    pub code_path: String,
    /// Every APK of the install (base first, then splits).
    pub apks: Vec<String>,
    pub apk_bytes: u64,
    pub requested: Vec<String>,
    /// Runtime (dangerous) permissions and whether each is granted.
    pub runtime: Vec<(String, bool)>,
    pub pid: Option<u32>,
}

impl Details {
    /// `u0_a142 (10142)` for apps, `system (1000)` and the like otherwise.
    pub fn uid_label(&self) -> String {
        match self.uid {
            Some(uid) if uid >= 10_000 => format!("u0_a{} ({uid})", uid - 10_000),
            Some(1000) => "system (1000)".into(),
            Some(uid) => uid.to_string(),
            None => "—".into(),
        }
    }
}

/// Lines indented under `header` (a line reading just that), until the
/// indent drops back.
fn block<'a>(text: &'a str, header: &str) -> Vec<&'a str> {
    let mut lines = text.lines();
    let Some(start) = lines.by_ref().find(|l| l.trim() == header) else { return Vec::new() };
    let indent = start.len() - start.trim_start().len();
    lines
        .take_while(|l| !l.trim().is_empty() && l.len() - l.trim_start().len() > indent)
        .map(str::trim)
        .collect()
}

pub fn parse_details(raw: &str) -> Details {
    let sec = split_sections(raw);
    let get = |k: &str| sec.get(k).map(String::as_str).unwrap_or("");
    // `dumpsys package <pkg>` starts with resolver tables; the facts are
    // in the "Packages:" section.
    let dump = get("dump");
    let dump = dump.find("\nPackages:").map_or(dump, |i| &dump[i..]);
    let field = |key: &str| -> Option<String> {
        dump.split_whitespace().find_map(|w| w.strip_prefix(key)).map(|v| v.trim_end_matches(',').to_owned())
    };
    let number = |key: &str| field(key).and_then(|v| v.parse().ok());
    let runtime = block(dump, "runtime permissions:")
        .into_iter()
        .filter_map(|l| {
            let (name, rest) = l.split_once(':')?;
            Some((name.trim().to_owned(), rest.contains("granted=true")))
        })
        .collect();
    Details {
        uid: number("userId=").or_else(|| number("appId=")),
        min_sdk: number("minSdk="),
        target_sdk: number("targetSdk="),
        installer: field("installerPackageName=").unwrap_or_else(|| "null".into()),
        code_path: field("codePath=").unwrap_or_default(),
        apks: get("path").lines().filter_map(|l| l.trim().strip_prefix("package:")).map(str::to_owned).collect(),
        apk_bytes: get("size").lines().filter_map(|l| l.trim().parse::<u64>().ok()).sum(),
        requested: block(dump, "requested permissions:")
            .into_iter()
            .map(|l| l.split(':').next().unwrap_or(l).to_owned())
            .collect(),
        runtime,
        pid: get("pid").split_whitespace().next().and_then(|p| p.parse().ok()),
    }
}

/// Read one package's details for the inspector.
pub async fn details(conn: Connection, pkg: String) -> Result<Details, String> {
    let q = super::shell_quote(&pkg);
    let script = format!(
        "echo @@dump; dumpsys package {q}; echo @@path; pm path {q}; \
         echo @@size; pm path {q} | sed 's/^package://' | xargs stat -c %s 2>/dev/null; echo @@pid; pidof {q}"
    );
    Ok(parse_details(&sh(&conn, &script, 30).await?))
}

/// Open an app on the TV: its TV (Leanback) entry first, then a phone one.
pub async fn launch(conn: Connection, pkg: String) -> String {
    let q = super::shell_quote(&pkg);
    for category in ["android.intent.category.LEANBACK_LAUNCHER", "android.intent.category.LAUNCHER"] {
        let out = sh(&conn, &format!("monkey -p {q} -c {category} 1 2>&1"), 20).await.unwrap_or_else(|e| e);
        if !out.contains("No activities found") && !out.contains("aborted") {
            return format!("Launched {pkg}.");
        }
    }
    format!("{pkg} has no app entry to open.")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Uninstall,
    Disable,
    Enable,
    /// Reinstall a package removed for this user (undoes Uninstall).
    Restore,
    /// Per-app cache clearing; needs Android 14 (API 34) or newer.
    ClearCache,
    ClearData,
    ForceStop,
}

/// First Android API level with `pm clear --cache-only`.
pub const CACHE_ONLY_API: u32 = 34;

impl Action {
    pub fn label(self) -> &'static str {
        match self {
            Action::Uninstall => "Uninstall",
            Action::Disable => "Disable",
            Action::Enable => "Enable",
            Action::Restore => "Restore",
            Action::ClearCache => "Clear Cache",
            Action::ClearData => "Clear Data",
            Action::ForceStop => "Force Stop",
        }
    }


    /// The command for actions that don't change a package's state (the
    /// others go through [`changes::set_package`]).
    fn command(self, pkg: &str) -> String {
        let pkg = super::shell_quote(pkg);
        match self {
            Action::ClearCache => format!("pm clear --cache-only {pkg}"),
            Action::ClearData => format!("pm clear {pkg}"),
            Action::ForceStop => format!("am force-stop {pkg}"),
            Action::Uninstall | Action::Disable | Action::Enable | Action::Restore => String::new(),
        }
    }

    /// Asked before running on `pkgs`; None when no confirmation is needed.
    pub fn confirmation(self, pkgs: &[String]) -> Option<(String, String)> {
        let what = match self {
            Action::Uninstall => "This removes the app and its data. Preinstalled apps can be brought back with Restore.",
            Action::ClearData => {
                "This deletes EVERYTHING the app saved: logins, settings, add-ons, playlists and downloads, as if it \
                 were freshly installed. It can't be undone.\n\nTo free space safely, use Clear Cache instead."
            }
            Action::ClearCache => {
                "Only cached files are deleted; apps rebuild them as needed. Logins, settings and add-ons are kept.\n\n\
                 On Android 13 and older, ADB can only trim every app's cache at once, and Android keeps part of the cache \
                 of apps you use a lot. For a single app, its App info screen then opens on the TV so you can press \
                 \"Clear cache\" there to clear all of it."
            }
            _ => return None,
        };
        let list = pkgs.iter().take(12).cloned().collect::<Vec<_>>().join("\n");
        let more = if pkgs.len() > 12 { "\n…" } else { "" };
        let subject = if pkgs.len() == 1 { pkgs[0].clone() } else { format!("{} packages", pkgs.len()) };
        Some((
            format!("{} {subject}", self.label()),
            format!("{} {subject}?\n\n{what}\n\n{list}{more}", self.label()),
        ))
    }
}

/// Clear cached files only, never data. Android 14+ can do one app at a
/// time; older versions only all apps at once (`pm trim-caches`).
async fn clear_cache(conn: &Connection, pkgs: &[String]) -> Outcome {
    let sdk: u32 = sh(conn, "getprop ro.build.version.sdk", 10).await.ok().and_then(|s| s.trim().parse().ok()).unwrap_or(0);
    let mut outcome = Outcome::default();
    if sdk >= CACHE_ONLY_API {
        for pkg in pkgs {
            let line = match sh(conn, &format!("pm clear --cache-only {}", super::shell_quote(pkg)), 60).await {
                Ok(out) if !out.contains("Failed") && !out.contains("Exception") => {
                    outcome.changes.push(Change::Note { text: format!("Cleared the cache of {pkg}") });
                    "done".to_owned()
                }
                Ok(out) => format!("failed: {}", out.trim()),
                Err(e) => format!("failed: {e}"),
            };
            outcome.lines.push(format!("Clear Cache {pkg}: {line}"));
        }
        return outcome;
    }
    // Android 13 and older: `pm trim-caches` trims every app's cache, but
    // only what's over each app's cache allowance (apps used a lot keep more).
    let free = || async { super::sysinfo::parse_df(&sh(conn, "df -k /data", 10).await.unwrap_or_default()).first().map(|r| r.avail) };
    let before = free().await;
    match sh(conn, &format!("pm trim-caches {}", 1u64 << 40), 180).await {
        Ok(_) => {
            let freed = match (before, free().await) {
                (Some(b), Some(a)) => format!("freed {}, {} now free", super::sysinfo::fmt_bytes(a.saturating_sub(b)), super::sysinfo::fmt_bytes(a)),
                _ => "done".into(),
            };
            outcome.lines.push(format!("Trimmed app caches: {freed}. Logins, settings and add-ons are kept."));
            outcome.changes.push(Change::Note { text: "Trimmed all app caches".into() });
        }
        Err(e) => outcome.lines.push(format!("Clear Cache failed: {e}")),
    }
    // For one app, its App info screen clears all of its cache: open it.
    if let [pkg] = pkgs {
        let open = format!("am start -a android.settings.APPLICATION_DETAILS_SETTINGS -d {}", super::shell_quote(&format!("package:{pkg}")));
        match sh(conn, &open, 20).await {
            Ok(out) if !out.contains("Error") => outcome.lines.push(format!(
                "Android 12 keeps part of the cache of apps you use a lot. {pkg}'s App info screen is now open on the TV: \
                 choose \"Clear cache\" there (with the remote or the Remote tab) to clear all of it."
            )),
            _ => outcome.lines.push(format!("To clear all of {pkg}'s cache: on the TV, Settings → Apps → {pkg} → Clear cache.")),
        }
    }
    outcome
}

/// Run `action` on each package in turn: a log line per package, and the
/// changes made (package states to undo, notes for the rest).
pub async fn run(conn: Connection, action: Action, pkgs: Vec<String>, progress: Progress) -> Outcome {
    if action == Action::ClearCache {
        return clear_cache(&conn, &pkgs).await;
    }
    let mut outcome = Outcome::default();
    progress.reset(Unit::Items, pkgs.len() as u64, "");
    for (i, pkg) in pkgs.iter().enumerate() {
        progress.set_detail(pkg.clone());
        let prefix = if pkgs.len() > 1 { format!("[{}/{}] ", i + 1, pkgs.len()) } else { String::new() };
        let target = match action {
            Action::Uninstall => Some(PkgState::Uninstalled),
            Action::Disable => Some(PkgState::Disabled),
            Action::Enable | Action::Restore => Some(PkgState::Enabled),
            Action::ClearCache | Action::ClearData | Action::ForceStop => None,
        };
        let line = match target {
            Some(target) => match changes::set_package(&conn, pkg, target).await {
                Ok((change, words)) => {
                    outcome.changes.push(change);
                    if words.is_empty() { "done".to_owned() } else { words }
                }
                Err(e) => format!("failed: {e}"),
            },
            None => match sh(&conn, &action.command(pkg), 60).await {
                Ok(out) if !out.contains("Failed") && !out.contains("Exception") => {
                    outcome.changes.push(Change::Note { text: format!("{} {pkg}", action.label()) });
                    if out.trim().is_empty() { "done".to_owned() } else { out.trim().replace('\n', " ") }
                }
                Ok(out) => format!("failed: {}", out.trim().replace('\n', " ")),
                Err(e) => format!("failed: {e}"),
            },
        };
        outcome.lines.push(format!("{prefix}{} {pkg}: {line}", action.label()));
        progress.add(1);
    }
    outcome
}

/// Save each package's APK into `dest`: `<pkg>.apk`, or a `<pkg>/` folder
/// holding every split for app-bundle installs.
pub async fn save_apks(conn: Connection, pkgs: Vec<String>, dest: PathBuf, progress: Progress) -> Vec<String> {
    let mut log = Vec::new();
    for pkg in pkgs {
        let paths: Vec<String> = match sh(&conn, &format!("pm path {}", super::shell_quote(&pkg)), 15).await {
            Ok(out) => out.lines().filter_map(|l| l.trim().strip_prefix("package:")).map(str::to_owned).collect(),
            Err(e) => {
                log.push(format!("{pkg}: {e}"));
                continue;
            }
        };
        if paths.is_empty() {
            log.push(format!("{pkg}: no APK path found"));
            continue;
        }
        let targets: Vec<(String, PathBuf)> = if paths.len() == 1 {
            vec![(paths[0].clone(), dest.join(format!("{pkg}.apk")))]
        } else {
            let dir = dest.join(&pkg);
            if let Err(e) = std::fs::create_dir_all(&dir) {
                log.push(format!("{pkg}: can't create {}: {e}", dir.display()));
                continue;
            }
            paths.iter().map(|p| (p.clone(), dir.join(Path::new(p).file_name().unwrap_or_default()))).collect()
        };
        for (remote, local) in targets {
            let file = local.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            progress.reset(Unit::Bytes, conn.file_size(remote.clone()).await.unwrap_or(0), file);
            match conn.pull(remote.clone(), local.clone(), Duration::from_secs(600), Some(progress.clone())).await {
                Ok(size) => log.push(format!("Saved {} ({})", local.display(), super::sysinfo::fmt_bytes(size))),
                Err(e) => log.push(format!("{pkg}: failed to pull {remote}: {e}")),
            }
        }
    }
    log
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_dedupe_and_installers() {
        assert_eq!(parse_list("package:com.a\npackage:com.b\npackage:com.a\njunk\n"), ["com.a", "com.b"]);
        let inst = parse_installers(
            "package:com.netflix.ninja  installer=com.android.vending\npackage:org.smarttube.beta  installer=null\npackage:com.x\n",
        );
        assert_eq!(inst["com.netflix.ninja"], PLAY_STORE);
        assert_eq!(inst["org.smarttube.beta"], "null");
        assert_eq!(inst["com.x"], "null");
    }

    #[test]
    fn inventory_from_script_output() {
        let raw = "@@all\n\
            package:/data/app/~~b9d==/com.netflix.ninja-x1==/base.apk=com.netflix.ninja  installer=com.android.vending\n\
            package:/data/app/~~q==/org.smarttube.beta-y==/base.apk=org.smarttube.beta  installer=null\n\
            package:/system/priv-app/Sync/Sync.apk=com.sony.dtv.braviasync  installer=null\n\
            package:/system/app/YT/YT.apk=com.google.android.youtube.tv  installer=com.android.vending\n\
            package:/system/app/Gone/Gone.apk=com.example.gone  installer=null\n\
            @@installed\npackage:com.netflix.ninja\npackage:org.smarttube.beta\npackage:com.sony.dtv.braviasync\n\
            package:com.google.android.youtube.tv\n\
            @@system\npackage:com.sony.dtv.braviasync\npackage:com.google.android.youtube.tv\npackage:com.example.gone\n\
            @@disabled\npackage:org.smarttube.beta\n\
            @@ps\nNAME\ncom.netflix.ninja\ncom.google.android.youtube.tv:service\n\
            @@versions\n  Package [com.netflix.ninja] (1):\n    versionName=10.1.2\n\
            @@sizes\n88293211 /data/app/~~b9d==/com.netflix.ninja-x1==/base.apk\n";
        let pkgs = parse_inventory(raw, "Sony");
        let by = |id: &str| pkgs.iter().find(|p| p.id == id).unwrap();
        let netflix = by("com.netflix.ninja");
        assert_eq!((netflix.origin, netflix.group()), (Origin::GooglePlay, Group::User));
        assert_eq!((netflix.version.as_str(), netflix.size, netflix.running), ("10.1.2", Some(88_293_211), true));
        assert_eq!(by("org.smarttube.beta").origin, Origin::Sideloaded);
        assert!(by("org.smarttube.beta").disabled);
        assert_eq!(by("com.sony.dtv.braviasync").origin, Origin::Oem);
        assert!(by("com.google.android.youtube.tv").running);
        assert_eq!(by("com.example.gone").group(), Group::Uninstalled);
        let count = |k: ListKind| pkgs.iter().filter(|p| k.matches(p)).count();
        assert_eq!((count(ListKind::All), count(ListKind::System), count(ListKind::User)), (4, 2, 2));
        assert_eq!((count(ListKind::PlayStore), count(ListKind::Sideloaded), count(ListKind::Uninstalled)), (2, 1, 1));
    }

    #[test]
    fn package_details() {
        let raw = "@@dump\nActivity Resolver Table:\n  userId=1\nPackages:\n  Package [com.netflix.ninja] (abc):\n\
            \x20   userId=10142\n\
            \x20   codePath=/data/app/~~b9d==/com.netflix.ninja-x1==\n\
            \x20   versionCode=1 minSdk=21 targetSdk=31\n\
            \x20   installerPackageName=com.android.vending\n\
            \x20   requested permissions:\n\
            \x20     android.permission.INTERNET\n\
            \x20     android.permission.RECORD_AUDIO\n\
            \x20   install permissions:\n\
            \x20     android.permission.INTERNET: granted=true\n\
            \x20   User 0: ceDataInode=1\n\
            \x20     runtime permissions:\n\
            \x20       android.permission.RECORD_AUDIO: granted=false, flags=[ ]\n\
            @@path\npackage:/data/app/x/base.apk\npackage:/data/app/x/split_config.armeabi_v7a.apk\n\
            @@size\n1000\n234\n@@pid\n8912\n";
        let d = parse_details(raw);
        assert_eq!(d.uid_label(), "u0_a142 (10142)");
        assert_eq!((d.min_sdk, d.target_sdk), (Some(21), Some(31)));
        assert_eq!(d.installer, "com.android.vending");
        assert_eq!(d.code_path, "/data/app/~~b9d==/com.netflix.ninja-x1==");
        assert_eq!((d.apks.len(), d.apk_bytes, d.pid), (2, 1234, Some(8912)));
        assert_eq!(d.requested, ["android.permission.INTERNET", "android.permission.RECORD_AUDIO"]);
        assert_eq!(d.runtime, [("android.permission.RECORD_AUDIO".to_owned(), false)]);
    }

    #[test]
    fn versions_first_block_wins() {
        let dump = "Packages:\n  Package [com.a] (1234):\n    versionCode=5\n    versionName=2.0\n\
                    \n  Package [com.b] (99):\n    versionName=1.1.0\nHidden system packages:\n\
                    \n  Package [com.a] (77):\n    versionName=1.0-factory\n";
        let v = parse_versions(dump);
        assert_eq!(v["com.a"], "2.0");
        assert_eq!(v["com.b"], "1.1.0");
    }

    #[test]
    fn confirmations_only_for_destructive_actions() {
        let one = vec!["com.a".to_owned()];
        assert!(Action::Disable.confirmation(&one).is_none());
        let (_, cache) = Action::ClearCache.confirmation(&one).unwrap();
        assert!(cache.contains("kept"));
        let (title, body) = Action::Uninstall.confirmation(&one).unwrap();
        assert_eq!(title, "Uninstall com.a");
        assert!(body.contains("Restore"));
        let many: Vec<String> = (0..14).map(|i| format!("com.p{i}")).collect();
        let (title, body) = Action::ClearData.confirmation(&many).unwrap();
        assert_eq!(title, "Clear Data 14 packages");
        assert!(body.ends_with('…'));
    }

    /// Copies one APK off a real TV (reads only):
    /// `STT_TEST_TV=ip:port STT_TEST_PKG=org.x STT_TEST_DIR=out cargo test live_save_apk -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn live_save_apk() {
        let serial = std::env::var("STT_TEST_TV").expect("set STT_TEST_TV=ip:port");
        let pkg = std::env::var("STT_TEST_PKG").expect("set STT_TEST_PKG");
        let dir = PathBuf::from(std::env::var("STT_TEST_DIR").expect("set STT_TEST_DIR"));
        let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
        rt.block_on(async {
            let conn = crate::android::transport::connect(serial).await.expect("connect");
            let t = std::time::Instant::now();
            for line in save_apks(conn, vec![pkg], dir, Progress::new(Unit::Bytes)).await {
                println!("{line}");
            }
            println!("took {:?}", t.elapsed());
        });
    }

    /// Read-only inventory and details on a real TV:
    /// `STT_TEST_TV=ip:port cargo test live_packages -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn live_packages() {
        let serial = std::env::var("STT_TEST_TV").expect("set STT_TEST_TV=ip:port");
        let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
        rt.block_on(async {
            let conn = crate::android::transport::connect(serial).await.expect("connect");
            let t = std::time::Instant::now();
            let pkgs = inventory(conn.clone(), "Sony".into()).await.expect("inventory");
            println!("{} packages in {:?}", pkgs.len(), t.elapsed());
            for kind in ListKind::ALL {
                let n = pkgs.iter().filter(|p| kind.matches(p)).count();
                println!("  {:12} {n}", kind.label());
            }
            let sized = pkgs.iter().filter(|p| p.size.is_some()).count();
            let versioned = pkgs.iter().filter(|p| !p.version.is_empty()).count();
            let running = pkgs.iter().filter(|p| p.running).count();
            let oem = pkgs.iter().filter(|p| p.origin == Origin::Oem).count();
            for rating in [Rating::Bloat, Rating::Optional, Rating::Keep] {
                let ids: Vec<&str> = pkgs.iter().filter(|p| p.rating() == Some(rating)).map(|p| p.id.as_str()).collect();
                println!("  {:?}: {}", rating, ids.len());
                if rating != Rating::Keep {
                    println!("    {}", ids.join(" "));
                }
            }
            let unrated = pkgs.iter().filter(|p| p.verdict.is_none() && p.origin != Origin::GooglePlay && p.origin != Origin::Sideloaded).count();
            println!("  unrated preinstalled: {unrated}");
            println!("  {sized} sized, {versioned} versioned, {running} running, {oem} OEM");
            for p in pkgs.iter().filter(|p| p.group() != Group::System).take(40) {
                println!(
                    "    {:?} {} | {} | {} | {:?} | run={} dis={}",
                    p.group(), p.id, p.origin.label(), p.version, p.size, p.running, p.disabled
                );
            }
            let first = pkgs.iter().find(|p| p.origin == Origin::GooglePlay).or(pkgs.first()).unwrap();
            let t = std::time::Instant::now();
            let d = details(conn, first.id.clone()).await.expect("details");
            println!(
                "details for {} ({:?}): uid {} sdk {:?}/{:?} installer {} apks {:?} {} bytes, {} requested, {} runtime, pid {:?}",
                first.id, t.elapsed(), d.uid_label(), d.min_sdk, d.target_sdk, d.installer, d.apks, d.apk_bytes,
                d.requested.len(), d.runtime.len(), d.pid
            );
        });
    }
}
