//! Change history (`changes.json`): every change the app makes to a TV,
//! with the value it replaced so it can be undone, plus snapshots of a TV's
//! package states and settings to roll back to.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

const FILE: &str = "changes.json";
/// Oldest entries are dropped beyond this.
const MAX_ENTRIES: usize = 5000;
/// Automatic snapshots kept per TV (manual ones are never pruned).
const MAX_AUTO_SNAPSHOTS: usize = 10;

/// A package's state for the TV's main user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PkgState {
    Enabled,
    Disabled,
    /// Removed for the user but still on the system image (restorable).
    Uninstalled,
}

impl PkgState {
    pub fn label(self) -> &'static str {
        match self {
            PkgState::Enabled => "enabled",
            PkgState::Disabled => "disabled",
            PkgState::Uninstalled => "uninstalled",
        }
    }
}

/// One change to a TV.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Change {
    /// `settings put/delete <namespace> <key>`; `None` means not set.
    Setting { namespace: String, key: String, before: Option<String>, after: Option<String> },
    Package { package: String, before: PkgState, after: PkgState },
    /// `wm density`; `None` means the TV's own density.
    Density { before: Option<u32>, after: Option<u32> },
    /// App standby bucket (`am set-standby-bucket`): `active`, `rare`, `restricted`…
    Bucket { package: String, before: String, after: String },
    /// An app-op mode (`cmd appops set`): `allow`, `ignore` or `default`.
    AppOp { package: String, op: String, before: String, after: String },
    /// A `device_config` flag; `None` means not set.
    DeviceConfig { namespace: String, key: String, before: Option<String>, after: Option<String> },
    /// One app component (e.g. a start-at-boot receiver) switched on or off.
    Component { component: String, before: bool, after: bool },
    /// The default home app (launcher activity).
    Home { before: String, after: String },
    /// Something that can't be undone (clearing data, stopping apps…).
    Note { text: String },
}

fn shown(v: &Option<String>) -> &str {
    v.as_deref().unwrap_or("not set")
}

impl Change {
    pub fn describe(&self) -> String {
        match self {
            Change::Setting { namespace, key, before, after } => {
                format!("Setting {namespace}/{key}: {} → {}", shown(before), shown(after))
            }
            Change::Package { package, before, after } => format!("{package}: {} → {}", before.label(), after.label()),
            Change::Density { before, after } => {
                let d = |v: &Option<u32>| v.map_or("default".to_owned(), |d| format!("{d} dpi"));
                format!("Display density: {} → {}", d(before), d(after))
            }
            Change::Bucket { package, before, after } => format!("{package}: standby group {before} → {after}"),
            Change::AppOp { package, op, before, after } => format!("{package}: {op} {before} → {after}"),
            Change::DeviceConfig { namespace, key, before, after } => {
                format!("device_config {namespace}/{key}: {} → {}", shown(before), shown(after))
            }
            Change::Component { component, before, after } => {
                let s = |on: &bool| if *on { "on" } else { "off" };
                format!("{component}: {} → {}", s(before), s(after))
            }
            Change::Home { before, after } => format!("Home app: {before} → {after}"),
            Change::Note { text } => text.clone(),
        }
    }

    /// The change that puts things back; `None` if it can't be undone.
    pub fn inverse(&self) -> Option<Change> {
        match self {
            Change::Setting { namespace, key, before, after } => Some(Change::Setting {
                namespace: namespace.clone(),
                key: key.clone(),
                before: after.clone(),
                after: before.clone(),
            }),
            Change::Package { package, before, after } => {
                Some(Change::Package { package: package.clone(), before: *after, after: *before })
            }
            Change::Density { before, after } => Some(Change::Density { before: *after, after: *before }),
            Change::Bucket { package, before, after } => {
                Some(Change::Bucket { package: package.clone(), before: after.clone(), after: before.clone() })
            }
            Change::AppOp { package, op, before, after } => Some(Change::AppOp {
                package: package.clone(),
                op: op.clone(),
                before: after.clone(),
                after: before.clone(),
            }),
            Change::DeviceConfig { namespace, key, before, after } => Some(Change::DeviceConfig {
                namespace: namespace.clone(),
                key: key.clone(),
                before: after.clone(),
                after: before.clone(),
            }),
            Change::Component { component, before, after } => {
                Some(Change::Component { component: component.clone(), before: *after, after: *before })
            }
            Change::Home { before, after } => Some(Change::Home { before: after.clone(), after: before.clone() }),
            Change::Note { .. } => None,
        }
    }

    /// Changes that set something to what it already was.
    pub fn is_noop(&self) -> bool {
        match self {
            Change::Setting { before, after, .. } => before == after,
            Change::Package { before, after, .. } => before == after,
            Change::Density { before, after } => before == after,
            Change::Bucket { before, after, .. } | Change::AppOp { before, after, .. } | Change::Home { before, after } => before == after,
            Change::DeviceConfig { before, after, .. } => before == after,
            Change::Component { before, after, .. } => before == after,
            Change::Note { .. } => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub id: u64,
    /// Local time, `2026-10-26 23:21:05`.
    pub time: String,
    /// The TV's IP address (ports change with wireless debugging).
    pub device: String,
    pub label: String,
    pub change: Change,
    /// When it was undone.
    #[serde(default)]
    pub reverted: Option<String>,
}

/// A TV's package states and settings at one moment.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    pub packages: BTreeMap<String, PkgState>,
    /// namespace → key → value
    pub settings: BTreeMap<String, BTreeMap<String, String>>,
    /// The density override, if any.
    pub density: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    pub id: u64,
    pub time: String,
    pub device: String,
    pub label: String,
    pub name: String,
    /// Taken by the app on connect (pruned), not by the user.
    pub auto: bool,
    pub state: State,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Journal {
    #[serde(default)]
    entries: Vec<Entry>,
    #[serde(default)]
    snapshots: Vec<Snapshot>,
    #[serde(default)]
    next_id: u64,
}

pub fn now() -> String {
    chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

/// `ip:port` → `ip`.
pub fn device_key(serial: &str) -> String {
    serial.rsplit_once(':').map_or(serial, |(ip, _)| ip).to_owned()
}

fn path() -> Option<PathBuf> {
    Some(crate::config::data_dir()?.join(FILE))
}

impl Journal {
    pub fn load() -> Journal {
        path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|text| serde_json::from_str(text.trim_start_matches('\u{feff}')).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> Result<(), String> {
        let path = path().ok_or("no data folder")?;
        let text = serde_json::to_string(self).map_err(|e| e.to_string())?;
        std::fs::write(path, text).map_err(|e| e.to_string())
    }

    fn id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    /// Add changes made on `serial`; no-op changes are skipped.
    pub fn record(&mut self, serial: &str, label: &str, changes: impl IntoIterator<Item = Change>) -> usize {
        let mut added = 0;
        for change in changes.into_iter().filter(|c| !c.is_noop()) {
            let id = self.id();
            self.entries.push(Entry { id, time: now(), device: device_key(serial), label: label.to_owned(), change, reverted: None });
            added += 1;
        }
        if self.entries.len() > MAX_ENTRIES {
            let extra = self.entries.len() - MAX_ENTRIES;
            self.entries.drain(..extra);
        }
        added
    }

    pub fn mark_reverted(&mut self, id: u64) {
        if let Some(e) = self.entries.iter_mut().find(|e| e.id == id) {
            e.reverted = Some(now());
        }
    }

    pub fn entry(&self, id: u64) -> Option<&Entry> {
        self.entries.iter().find(|e| e.id == id)
    }

    /// A TV's changes, newest first.
    pub fn entries_for<'a>(&'a self, serial: &str) -> impl Iterator<Item = &'a Entry> {
        let device = device_key(serial);
        self.entries.iter().rev().filter(move |e| e.device == device)
    }

    pub fn snapshots_for<'a>(&'a self, serial: &str) -> impl Iterator<Item = &'a Snapshot> {
        let device = device_key(serial);
        self.snapshots.iter().rev().filter(move |s| s.device == device)
    }

    pub fn snapshot(&self, id: u64) -> Option<&Snapshot> {
        self.snapshots.iter().find(|s| s.id == id)
    }

    pub fn add_snapshot(&mut self, serial: &str, label: &str, name: String, auto: bool, state: State) -> u64 {
        let id = self.id();
        let device = device_key(serial);
        self.snapshots.push(Snapshot { id, time: now(), device: device.clone(), label: label.to_owned(), name, auto, state });
        // Keep the newest automatic snapshots of this TV.
        let autos: Vec<u64> = self.snapshots.iter().filter(|s| s.auto && s.device == device).map(|s| s.id).collect();
        if autos.len() > MAX_AUTO_SNAPSHOTS {
            let drop = &autos[..autos.len() - MAX_AUTO_SNAPSHOTS];
            self.snapshots.retain(|s| !drop.contains(&s.id));
        }
        id
    }

    pub fn remove_snapshot(&mut self, id: u64) {
        self.snapshots.retain(|s| s.id != id);
    }

    /// Whether `serial` has a snapshot taken on `date` (`YYYY-MM-DD`).
    pub fn has_snapshot_on(&self, serial: &str, date: &str) -> bool {
        self.snapshots_for(serial).any(|s| s.time.starts_with(date))
    }

    /// Settings keys the app changed on `serial` since `time`, as `ns/key`.
    pub fn touched_settings_since(&self, serial: &str, time: &str) -> Vec<String> {
        self.entries_for(serial)
            .filter(|e| e.time.as_str() >= time)
            .filter_map(|e| match &e.change {
                Change::Setting { namespace, key, .. } => Some(format!("{namespace}/{key}")),
                _ => None,
            })
            .collect()
    }
}

/// What it takes to bring a TV from `current` back to `target`: one change
/// per package or setting that differs (keys the TV has dropped or added
/// since are left alone, as are the ones that would cut the connection).
pub fn restore_plan(target: &State, current: &State) -> Vec<Change> {
    let mut plan = Vec::new();
    for (pkg, want) in &target.packages {
        if let Some(now) = current.packages.get(pkg)
            && now != want
        {
            plan.push(Change::Package { package: pkg.clone(), before: *now, after: *want });
        }
    }
    for (ns, keys) in &target.settings {
        let now = current.settings.get(ns);
        for (key, want) in keys {
            if unsafe_to_restore(key) {
                continue;
            }
            let have = now.and_then(|m| m.get(key));
            if have != Some(want) {
                plan.push(Change::Setting { namespace: ns.clone(), key: key.clone(), before: have.cloned(), after: Some(want.clone()) });
            }
        }
    }
    if target.density != current.density {
        plan.push(Change::Density { before: current.density, after: target.density });
    }
    plan
}

/// What it takes to give another TV (`current`) the same setup as
/// `source`: apps disabled or uninstalled there get the same state here
/// (when this TV has them), and the settings in `keys` (`ns/key`, the ones
/// this app changed on the source TV) get the source's values.
pub fn copy_plan(source: &State, keys: &[String], current: &State) -> Vec<Change> {
    let mut plan = Vec::new();
    for (pkg, want) in &source.packages {
        if *want == PkgState::Enabled {
            continue;
        }
        if let Some(now) = current.packages.get(pkg)
            && now != want
        {
            plan.push(Change::Package { package: pkg.clone(), before: *now, after: *want });
        }
    }
    for k in keys {
        let Some((ns, key)) = k.split_once('/') else { continue };
        if unsafe_to_restore(key) {
            continue;
        }
        let want = source.settings.get(ns).and_then(|m| m.get(key));
        let have = current.settings.get(ns).and_then(|m| m.get(key));
        if let Some(want) = want
            && have != Some(want)
        {
            plan.push(Change::Setting { namespace: ns.into(), key: key.into(), before: have.cloned(), after: Some(want.clone()) });
        }
    }
    plan
}

/// Settings a restore must never touch: they'd drop the ADB connection or
/// confuse the TV's setup state.
fn unsafe_to_restore(key: &str) -> bool {
    key.starts_with("adb_")
        || matches!(key, "development_settings_enabled" | "device_provisioned" | "user_setup_complete" | "boot_count")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setting(ns: &str, key: &str, before: Option<&str>, after: Option<&str>) -> Change {
        Change::Setting {
            namespace: ns.into(),
            key: key.into(),
            before: before.map(Into::into),
            after: after.map(Into::into),
        }
    }

    #[test]
    fn inverses_undo() {
        let c = setting("global", "window_animation_scale", Some("1"), Some("0.5"));
        assert_eq!(c.inverse(), Some(setting("global", "window_animation_scale", Some("0.5"), Some("1"))));
        let p = Change::Package { package: "com.a".into(), before: PkgState::Enabled, after: PkgState::Uninstalled };
        assert_eq!(
            p.inverse(),
            Some(Change::Package { package: "com.a".into(), before: PkgState::Uninstalled, after: PkgState::Enabled })
        );
        assert_eq!(Change::Note { text: "Cleared data".into() }.inverse(), None);
        let home = Change::Home { before: "a/.Home".into(), after: "b/.Main".into() };
        assert_eq!(home.inverse(), Some(Change::Home { before: "b/.Main".into(), after: "a/.Home".into() }));
        let off = Change::Component { component: "x/.Boot".into(), before: true, after: false };
        assert_eq!(off.describe(), "x/.Boot: on → off");
        assert_eq!(setting("system", "k", None, Some("1")).describe(), "Setting system/k: not set → 1");
    }

    #[test]
    fn records_per_device_and_skips_noops() {
        let mut j = Journal::default();
        let added = j.record(
            "192.168.1.78:5555",
            "Sony",
            [setting("global", "a", Some("1"), Some("1")), setting("global", "b", Some("1"), Some("2"))],
        );
        assert_eq!(added, 1);
        j.record("192.168.1.10:5555", "Box", [Change::Note { text: "x".into() }]);
        // A new wireless-debugging port is still the same TV.
        assert_eq!(j.entries_for("192.168.1.78:41234").count(), 1);
        let id = j.entries_for("192.168.1.78:5555").next().unwrap().id;
        j.mark_reverted(id);
        assert!(j.entry(id).unwrap().reverted.is_some());
    }

    #[test]
    fn plans_a_restore() {
        let mut target = State::default();
        target.packages.insert("com.a".into(), PkgState::Enabled);
        target.packages.insert("com.b".into(), PkgState::Disabled);
        target.packages.insert("com.gone".into(), PkgState::Enabled);
        target.settings.entry("global".into()).or_default().insert("window_animation_scale".into(), "1".into());
        target.settings.entry("global".into()).or_default().insert("adb_enabled".into(), "1".into());
        let mut current = State::default();
        current.packages.insert("com.a".into(), PkgState::Uninstalled);
        current.packages.insert("com.b".into(), PkgState::Disabled);
        current.packages.insert("com.new".into(), PkgState::Enabled);
        current.settings.entry("global".into()).or_default().insert("window_animation_scale".into(), "0.5".into());
        current.settings.entry("global".into()).or_default().insert("adb_enabled".into(), "0".into());
        current.density = Some(240);
        let plan = restore_plan(&target, &current);
        assert_eq!(
            plan,
            vec![
                Change::Package { package: "com.a".into(), before: PkgState::Uninstalled, after: PkgState::Enabled },
                setting("global", "window_animation_scale", Some("0.5"), Some("1")),
                Change::Density { before: Some(240), after: None },
            ]
        );
    }

    #[test]
    fn copies_a_setup() {
        let mut source = State::default();
        source.packages.insert("com.bloat".into(), PkgState::Disabled);
        source.packages.insert("com.app".into(), PkgState::Enabled);
        source.packages.insert("com.only.there".into(), PkgState::Uninstalled);
        source.settings.entry("global".into()).or_default().insert("window_animation_scale".into(), "0.5".into());
        let mut other = State::default();
        other.packages.insert("com.bloat".into(), PkgState::Enabled);
        other.packages.insert("com.app".into(), PkgState::Disabled);
        other.settings.entry("global".into()).or_default().insert("window_animation_scale".into(), "1".into());
        let plan = copy_plan(&source, &["global/window_animation_scale".into(), "global/adb_enabled".into()], &other);
        assert_eq!(
            plan,
            vec![
                Change::Package { package: "com.bloat".into(), before: PkgState::Enabled, after: PkgState::Disabled },
                setting("global", "window_animation_scale", Some("1"), Some("0.5")),
            ]
        );
    }

    #[test]
    fn prunes_old_automatic_snapshots() {
        let mut j = Journal::default();
        let keep = j.add_snapshot("10.0.0.2:5555", "TV", "Mine".into(), false, State::default());
        for i in 0..MAX_AUTO_SNAPSHOTS + 3 {
            j.add_snapshot("10.0.0.2:5555", "TV", format!("Auto {i}"), true, State::default());
        }
        assert_eq!(j.snapshots_for("10.0.0.2:5555").filter(|s| s.auto).count(), MAX_AUTO_SNAPSHOTS);
        assert!(j.snapshot(keep).is_some());
    }
}
