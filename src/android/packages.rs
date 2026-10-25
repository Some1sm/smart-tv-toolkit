//! The Packages tab: listing installed apps and acting on a selection.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::health::pretty_pkg_name;
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
}

impl ListKind {
    pub const ALL: [ListKind; 6] =
        [ListKind::All, ListKind::System, ListKind::User, ListKind::PlayStore, ListKind::Sideloaded, ListKind::Uninstalled];

    pub fn label(self) -> &'static str {
        match self {
            ListKind::All => "All",
            ListKind::System => "System",
            ListKind::User => "User",
            ListKind::Uninstalled => "Uninstalled",
            ListKind::PlayStore => "Play Store",
            ListKind::Sideloaded => "Sideloaded",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Package {
    pub id: String,
    pub name: String,
    pub version: String,
    pub disabled: bool,
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
pub fn parse_installers(text: &str) -> HashMap<String, String> {
    text.lines()
        .filter_map(|l| {
            let rest = l.trim().strip_prefix("package:")?;
            let mut parts = rest.split_whitespace();
            let pkg = parts.next()?.to_owned();
            let installer = parts.find_map(|p| p.strip_prefix("installer=")).unwrap_or("null").to_owned();
            Some((pkg, installer))
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

async fn sh(conn: &Connection, cmd: &str, secs: u64) -> Result<String, String> {
    conn.shell(cmd, Duration::from_secs(secs)).await.map_err(|e| e.to_string())
}

/// List packages of one kind, with versions and disabled state.
pub async fn list(conn: Connection, kind: ListKind) -> Result<Vec<Package>, String> {
    let ids: Vec<String> = match kind {
        ListKind::All => parse_list(&sh(&conn, "pm list packages", 30).await?),
        ListKind::System => parse_list(&sh(&conn, "pm list packages -s", 30).await?),
        ListKind::User => parse_list(&sh(&conn, "pm list packages -3", 30).await?),
        ListKind::Uninstalled => {
            // -u adds uninstalled packages to the normal list, so subtract it.
            let installed: BTreeSet<String> = parse_list(&sh(&conn, "pm list packages", 30).await?).into_iter().collect();
            parse_list(&sh(&conn, "pm list packages -u", 30).await?)
                .into_iter()
                .filter(|p| !installed.contains(p))
                .collect()
        }
        ListKind::PlayStore | ListKind::Sideloaded => {
            let flags = if kind == ListKind::PlayStore { "-i" } else { "-i -3" };
            let mut ids: Vec<String> = parse_installers(&sh(&conn, &format!("pm list packages {flags}"), 30).await?)
                .into_iter()
                .filter(|(_, installer)| (installer == PLAY_STORE) == (kind == ListKind::PlayStore))
                .map(|(p, _)| p)
                .collect();
            ids.sort();
            ids
        }
    };
    let versions = parse_versions(&sh(&conn, "dumpsys package packages", 90).await.unwrap_or_default());
    let disabled: BTreeSet<String> =
        parse_list(&sh(&conn, "pm list packages -d", 30).await.unwrap_or_default()).into_iter().collect();
    Ok(ids
        .into_iter()
        .map(|id| Package {
            name: pretty_pkg_name(&id),
            version: versions.get(&id).cloned().unwrap_or_default(),
            disabled: disabled.contains(&id),
            id,
        })
        .collect())
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
}

/// First Android API level with `pm clear --cache-only`.
pub const CACHE_ONLY_API: u32 = 34;

impl Action {
    pub const ALL: [Action; 6] =
        [Action::Uninstall, Action::Disable, Action::Enable, Action::Restore, Action::ClearCache, Action::ClearData];

    pub fn label(self) -> &'static str {
        match self {
            Action::Uninstall => "Uninstall",
            Action::Disable => "Disable",
            Action::Enable => "Enable",
            Action::Restore => "Restore",
            Action::ClearCache => "Clear Cache",
            Action::ClearData => "Clear Data",
        }
    }

    pub fn destructive(self) -> bool {
        matches!(self, Action::Uninstall | Action::ClearData)
    }

    fn command(self, pkg: &str) -> String {
        match self {
            Action::Uninstall => format!("pm uninstall --user 0 {pkg}"),
            Action::Disable => format!("pm disable-user --user 0 {pkg}"),
            Action::Enable => format!("pm enable {pkg}"),
            Action::Restore => format!("cmd package install-existing {pkg}"),
            Action::ClearCache => format!("pm clear --cache-only {pkg}"),
            Action::ClearData => format!("pm clear {pkg}"),
        }
    }

    /// Asked before running on `pkgs`; None when no confirmation is needed.
    pub fn confirmation(self, pkgs: &[String]) -> Option<(String, String)> {
        if !self.destructive() {
            return None;
        }
        let what = match self {
            Action::Uninstall => "This removes the app and its data. Preinstalled apps can be brought back with Restore.",
            _ => "This deletes all of the app's data, including logins and settings.",
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

/// Run `action` on each package in turn; returns one log line per package.
pub async fn run(conn: Connection, action: Action, pkgs: Vec<String>) -> Vec<String> {
    let mut log = Vec::with_capacity(pkgs.len());
    for (i, pkg) in pkgs.iter().enumerate() {
        let prefix = if pkgs.len() > 1 { format!("[{}/{}] ", i + 1, pkgs.len()) } else { String::new() };
        let out = sh(&conn, &action.command(pkg), 60).await.unwrap_or_else(|e| e);
        let out = if out.is_empty() { "done".to_owned() } else { out.replace('\n', " ") };
        log.push(format!("{prefix}{} {pkg}: {out}", action.label()));
    }
    log
}

/// Save each package's APK into `dest`: `<pkg>.apk`, or a `<pkg>/` folder
/// holding every split for app-bundle installs.
pub async fn save_apks(conn: Connection, pkgs: Vec<String>, dest: PathBuf) -> Vec<String> {
    let mut log = Vec::new();
    for pkg in pkgs {
        let paths: Vec<String> = match sh(&conn, &format!("pm path {pkg}"), 15).await {
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
            match conn.pull(remote.clone(), local.clone(), Duration::from_secs(600)).await {
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
            for line in save_apks(conn, vec![pkg], dir).await {
                println!("{line}");
            }
            println!("took {:?}", t.elapsed());
        });
    }

    /// Read-only listing on a real TV: `STT_TEST_TV=ip:port cargo test live_packages -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn live_packages() {
        let serial = std::env::var("STT_TEST_TV").expect("set STT_TEST_TV=ip:port");
        let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
        rt.block_on(async {
            let conn = crate::android::transport::connect(serial).await.expect("connect");
            for kind in ListKind::ALL {
                let t = std::time::Instant::now();
                let pkgs = list(conn.clone(), kind).await.expect("list");
                let with_version = pkgs.iter().filter(|p| !p.version.is_empty()).count();
                let disabled = pkgs.iter().filter(|p| p.disabled).count();
                println!("{:12} {:4} pkgs, {with_version} with version, {disabled} disabled ({:?})",
                         kind.label(), pkgs.len(), t.elapsed());
                if kind == ListKind::User {
                    for p in &pkgs {
                        println!("    {} | {} | {}", p.id, p.name, p.version);
                    }
                }
            }
        });
    }
}
