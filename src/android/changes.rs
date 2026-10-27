//! Changes that remember what they replaced: settings, package states and
//! display density. Everything that alters a TV goes through here so the
//! History tab can undo it, and snapshots are read here too.

use std::collections::BTreeSet;
use std::time::Duration;

use super::sysinfo::split_sections;
use super::{Connection, shell_quote};
use crate::journal::{Change, PkgState, State};

pub const NAMESPACES: [&str; 3] = ["system", "secure", "global"];

/// What an operation did: lines for the log and the changes to record.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Outcome {
    pub lines: Vec<String>,
    pub changes: Vec<Change>,
}

impl Outcome {
    pub fn lines(lines: Vec<String>) -> Outcome {
        Outcome { lines, changes: Vec::new() }
    }
}

async fn sh(conn: &Connection, cmd: &str, secs: u64) -> Result<String, String> {
    conn.shell(cmd, Duration::from_secs(secs)).await.map_err(|e| e.to_string())
}

/// `settings get` prints `null` for keys that aren't set.
fn setting_value(out: &str) -> Option<String> {
    let v = out.trim();
    (!v.is_empty() && v != "null").then(|| v.to_owned())
}

/// A key as the `settings` command accepts it (letters, digits, `_.-:`).
pub fn is_setting_key(key: &str) -> bool {
    !key.is_empty() && key.chars().all(|c| c.is_ascii_alphanumeric() || "_.-:".contains(c))
}

/// Set (or with `None`, delete) a setting and return what changed.
pub async fn put_setting(conn: &Connection, namespace: &str, key: &str, value: Option<&str>) -> Result<Change, String> {
    if !NAMESPACES.contains(&namespace) || !is_setting_key(key) {
        return Err(format!("not a valid setting: {namespace}/{key}"));
    }
    let before = setting_value(&sh(conn, &format!("settings get {namespace} {key}"), 15).await?);
    let cmd = match value {
        Some(v) => format!("settings put {namespace} {key} {}", shell_quote(v)),
        None => format!("settings delete {namespace} {key}"),
    };
    let out = sh(conn, &cmd, 15).await?;
    if out.contains("Exception") || out.contains("Error") {
        return Err(out.trim().to_owned());
    }
    let after = setting_value(&sh(conn, &format!("settings get {namespace} {key}"), 15).await?);
    Ok(Change::Setting { namespace: namespace.into(), key: key.into(), before, after })
}

/// Where a package stands right now; `None` if it isn't on the TV at all.
pub async fn package_state(conn: &Connection, pkg: &str) -> Result<Option<PkgState>, String> {
    let q = shell_quote(&format!("package:{pkg}"));
    let script = format!(
        "if pm list packages | grep -qx {q}; then \
           if pm list packages -d | grep -qx {q}; then echo disabled; else echo enabled; fi; \
         elif pm list packages -u | grep -qx {q}; then echo uninstalled; else echo missing; fi"
    );
    Ok(match sh(conn, &script, 30).await?.trim() {
        "enabled" => Some(PkgState::Enabled),
        "disabled" => Some(PkgState::Disabled),
        "uninstalled" => Some(PkgState::Uninstalled),
        _ => None,
    })
}

/// `pm` reports failures in its output rather than its exit code.
fn failed(out: &str) -> bool {
    ["Failure", "Error", "Exception", "not installed", "Unknown package"].iter().any(|w| out.contains(w))
}

/// Move a package to `target`; returns the change and `pm`'s own words.
pub async fn set_package(conn: &Connection, pkg: &str, target: PkgState) -> Result<(Change, String), String> {
    let before = package_state(conn, pkg).await?.ok_or_else(|| format!("{pkg} isn't on the TV"))?;
    let q = shell_quote(pkg);
    let mut cmds = Vec::new();
    if before == PkgState::Uninstalled && target != PkgState::Uninstalled {
        cmds.push(format!("cmd package install-existing {q}"));
    }
    match target {
        PkgState::Enabled if before == PkgState::Disabled => cmds.push(format!("pm enable {q}")),
        PkgState::Disabled if before != PkgState::Disabled => cmds.push(format!("pm disable-user --user 0 {q}")),
        PkgState::Uninstalled if before != PkgState::Uninstalled => cmds.push(format!("pm uninstall --user 0 {q}")),
        _ => {}
    }
    let mut said = Vec::new();
    for cmd in cmds {
        match sh(conn, &cmd, 60).await {
            Ok(out) if !failed(&out) => said.push(out.trim().replace('\n', " ")),
            // A retried command can report a failure for work it already
            // did: what counts is where the package ends up.
            Ok(out) => {
                if package_state(conn, pkg).await? != Some(target) {
                    return Err(out.trim().replace('\n', " "));
                }
            }
            Err(e) => {
                if package_state(conn, pkg).await? != Some(target) {
                    return Err(e);
                }
            }
        }
    }
    let after = package_state(conn, pkg).await?.unwrap_or(before);
    if after != target {
        return Err(format!("{pkg} is still {}", after.label()));
    }
    let words = said.into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" ");
    Ok((Change::Package { package: pkg.into(), before, after }, words))
}

/// `wm density` → the override, if one is set.
pub fn parse_density_override(out: &str) -> Option<u32> {
    out.lines().find_map(|l| l.trim().strip_prefix("Override density:")).and_then(|v| v.trim().parse().ok())
}

pub async fn set_density(conn: &Connection, density: Option<u32>) -> Result<Change, String> {
    let before = parse_density_override(&sh(conn, "wm density", 15).await?);
    let cmd = match density {
        Some(d) => format!("wm density {d}"),
        None => "wm density reset".into(),
    };
    sh(conn, &cmd, 15).await?;
    let after = parse_density_override(&sh(conn, "wm density", 15).await?);
    Ok(Change::Density { before, after })
}

/// Carry out `change` (its `after` side); returns what actually changed.
pub async fn apply(conn: &Connection, change: &Change) -> Result<Change, String> {
    match change {
        Change::Setting { namespace, key, after, .. } => put_setting(conn, namespace, key, after.as_deref()).await,
        Change::Package { package, after, .. } => set_package(conn, package, *after).await.map(|(c, _)| c),
        Change::Density { after, .. } => set_density(conn, *after).await,
        Change::Bucket { package, after, .. } => super::optimize::set_bucket(conn, package, after).await,
        Change::AppOp { package, op, after, .. } => super::optimize::set_appop(conn, package, op, after).await,
        Change::DeviceConfig { namespace, key, after, .. } => super::optimize::set_config(conn, namespace, key, after.as_deref()).await,
        Change::Component { component, after, .. } => super::optimize::set_component(conn, component, *after).await,
        Change::Home { after, .. } if after.is_empty() => Err("the previous home app isn't known".into()),
        Change::Home { after, .. } => super::optimize::set_home(conn, after).await,
        Change::Note { .. } => Err("this change can't be undone".into()),
    }
}

/// Undo `change`.
pub async fn revert(conn: Connection, change: Change) -> Result<Change, String> {
    let inverse = change.inverse().ok_or("this change can't be undone")?;
    apply(&conn, &inverse).await
}

const CAPTURE: &str = "\
    echo @@all; pm list packages -u; \
    echo @@installed; pm list packages; \
    echo @@disabled; pm list packages -d; \
    echo @@system; settings list system; \
    echo @@secure; settings list secure; \
    echo @@global; settings list global; \
    echo @@density; wm density";

pub fn parse_state(raw: &str) -> State {
    let sec = split_sections(raw);
    let get = |k: &str| sec.get(k).map(String::as_str).unwrap_or("");
    let installed: BTreeSet<String> = super::packages::parse_list(get("installed")).into_iter().collect();
    let disabled: BTreeSet<String> = super::packages::parse_list(get("disabled")).into_iter().collect();
    let mut state = State::default();
    for pkg in super::packages::parse_list(get("all")) {
        let s = if disabled.contains(&pkg) {
            PkgState::Disabled
        } else if installed.contains(&pkg) {
            PkgState::Enabled
        } else {
            PkgState::Uninstalled
        };
        state.packages.insert(pkg, s);
    }
    for ns in NAMESPACES {
        let map = state.settings.entry(ns.to_owned()).or_default();
        for line in get(ns).lines() {
            if let Some((k, v)) = line.split_once('=')
                && is_setting_key(k)
                && v != "null"
            {
                map.insert(k.to_owned(), v.to_owned());
            }
        }
    }
    state.density = parse_density_override(get("density"));
    state
}

/// Read a TV's package states and settings.
pub async fn capture(conn: Connection) -> Result<State, String> {
    let state = parse_state(&sh(&conn, CAPTURE, 60).await?);
    if state.packages.is_empty() { Err("the TV listed no packages".into()) } else { Ok(state) }
}

/// Apply a restore plan; returns the changes made and a line per failure.
pub async fn restore(conn: Connection, plan: Vec<Change>, progress: crate::jobs::Progress) -> (Vec<Change>, Vec<String>) {
    progress.reset(crate::jobs::Unit::Items, plan.len() as u64, "");
    let (mut done, mut errors) = (Vec::new(), Vec::new());
    for change in plan {
        progress.set_detail(change.describe());
        match apply(&conn, &change).await {
            Ok(c) => done.push(c),
            Err(e) => errors.push(format!("{}: {e}", change.describe())),
        }
        progress.add(1);
    }
    (done, errors)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_state_from_capture_output() {
        let raw = "@@all\npackage:com.a\npackage:com.b\npackage:com.c\n@@installed\npackage:com.a\npackage:com.b\n\
                   @@disabled\npackage:com.b\n@@system\nscreen_off_timeout=600000\nbad key=1\nnothing=null\n\
                   @@secure\n@@global\nwindow_animation_scale=0.5\n@@density\nPhysical density: 320\nOverride density: 240\n";
        let s = parse_state(raw);
        assert_eq!(s.packages["com.a"], PkgState::Enabled);
        assert_eq!(s.packages["com.b"], PkgState::Disabled);
        assert_eq!(s.packages["com.c"], PkgState::Uninstalled);
        assert_eq!(s.settings["system"].len(), 1);
        assert_eq!(s.settings["global"]["window_animation_scale"], "0.5");
        assert_eq!(s.density, Some(240));
        assert_eq!(parse_density_override("Physical density: 320"), None);
    }

    #[test]
    fn values_and_keys() {
        assert_eq!(setting_value("null\n"), None);
        assert_eq!(setting_value(" 0.5\n"), Some("0.5".into()));
        assert!(is_setting_key("window_animation_scale"));
        assert!(!is_setting_key("a; reboot"));
        assert!(failed("Failure [DELETE_FAILED_INTERNAL_ERROR]"));
        assert!(!failed("Success"));
    }

    /// Reads a full snapshot from a real TV (changes nothing):
    /// `STT_TEST_TV=ip:port cargo test live_capture -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn live_capture() {
        let serial = std::env::var("STT_TEST_TV").expect("set STT_TEST_TV=ip:port");
        let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
        rt.block_on(async {
            let conn = super::super::transport::connect(serial).await.expect("connect");
            let t = std::time::Instant::now();
            let s = capture(conn.clone()).await.expect("capture");
            let n: usize = s.settings.values().map(|m| m.len()).sum();
            println!("{} packages, {n} settings, density {:?} in {:?}", s.packages.len(), s.density, t.elapsed());
            println!("state of com.netflix.ninja: {:?}", package_state(&conn, "com.netflix.ninja").await);
            println!("state of org.smarttube.beta: {:?}", package_state(&conn, "org.smarttube.beta").await);
        });
    }
}
