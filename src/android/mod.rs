//! Android TV / Google TV support, over ADB.

pub mod adb;

use std::path::{Path, PathBuf};

/// Outcome of `adb connect`.
#[derive(Debug, Clone)]
pub enum ConnectResult {
    Connected { serial: String, summary: DeviceSummary },
    /// The TV is showing "Allow USB debugging?"; the user must accept it.
    NeedsApproval(String),
    Failed(String),
}

/// The few properties shown right after connecting. The full Info tab
/// comes in a later milestone.
#[derive(Debug, Clone, Default)]
pub struct DeviceSummary {
    pub manufacturer: String,
    pub model: String,
    pub model_number: String,
    pub android_version: String,
    pub sdk: String,
    pub abi: String,
}

impl DeviceSummary {
    /// "Sony XR-65A80J" — prefers the retail model number when the TV
    /// reports one, since `ro.product.model` is often a shared platform name.
    pub fn label(&self) -> String {
        let model = if self.model_number.is_empty() { &self.model } else { &self.model_number };
        if model.to_lowercase().starts_with(&self.manufacturer.to_lowercase()) {
            model.clone()
        } else {
            format!("{} {}", self.manufacturer, model).trim().to_owned()
        }
    }
}

/// Parse a full `getprop` dump into (key, value) pairs. Lines that are not
/// `[key]: [value]` (e.g. ADB daemon startup noise) are ignored.
pub fn parse_getprop(raw: &str) -> Vec<(String, String)> {
    raw.lines()
        .filter_map(|line| {
            let line = line.trim();
            let rest = line.strip_prefix('[')?;
            let (key, rest) = rest.split_once("]: [")?;
            let value = rest.strip_suffix(']')?;
            Some((key.to_owned(), value.trim().to_owned()))
        })
        .collect()
}

fn summary_from_props(props: &[(String, String)]) -> DeviceSummary {
    let get = |k: &str| {
        props.iter().find(|(key, _)| key == k).map(|(_, v)| v.clone()).unwrap_or_default()
    };
    DeviceSummary {
        manufacturer: get("ro.product.manufacturer"),
        model: get("ro.product.model"),
        model_number: get("ro.boot.product.hardware.sku"),
        android_version: get("ro.build.version.release"),
        sdk: get("ro.build.version.sdk"),
        abi: get("ro.product.cpu.abi"),
    }
}

/// `host` or `host:port` → `host:port` (5555 is ADB's default port).
pub fn serial_for(address: &str, port: &str) -> String {
    let address = address.trim();
    if address.contains(':') {
        address.to_owned()
    } else {
        let port = port.trim();
        format!("{address}:{}", if port.is_empty() { "5555" } else { port })
    }
}

pub async fn connect(adb: PathBuf, serial: String) -> ConnectResult {
    let out = match adb::run(&adb, None, &["connect", &serial], adb::SHORT).await {
        Ok(out) => out,
        Err(e) => return ConnectResult::Failed(e.to_string()),
    };
    let lower = out.to_lowercase();
    if lower.contains("failed to authenticate") || lower.contains("unauthorized") {
        return ConnectResult::NeedsApproval(out);
    }
    if !(lower.contains("connected to") || lower.contains("already connected")) {
        return ConnectResult::Failed(out);
    }
    match fetch_summary(&adb, &serial).await {
        Ok(summary) => ConnectResult::Connected { serial, summary },
        // Connected at the TCP level but the shell is not usable yet: this is
        // what an unanswered "Allow debugging?" prompt looks like.
        Err(e) if e.contains("unauthorized") => ConnectResult::NeedsApproval(e),
        Err(e) => ConnectResult::Failed(e),
    }
}

async fn fetch_summary(adb: &Path, serial: &str) -> Result<DeviceSummary, String> {
    let raw = adb::run(adb, Some(serial), &["shell", "getprop"], adb::SHORT)
        .await
        .map_err(|e| e.to_string())?;
    let props = parse_getprop(&raw);
    if props.is_empty() {
        return Err(raw);
    }
    Ok(summary_from_props(&props))
}

pub async fn disconnect(adb: PathBuf, serial: String) -> String {
    adb::run(&adb, None, &["disconnect", &serial], adb::SHORT)
        .await
        .unwrap_or_else(|e| e.to_string())
}

pub async fn start_server(adb: PathBuf) -> Result<String, String> {
    let version = adb::verify(&adb).await?;
    adb::run(&adb, None, &["start-server"], adb::SHORT)
        .await
        .map_err(|e| e.to_string())?;
    Ok(version)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn getprop_ignores_daemon_noise() {
        let raw = "* daemon not running; starting now at tcp:5037\n\
                   * daemon started successfully\n\
                   [ro.product.manufacturer]: [Sony]\n\
                   [ro.product.model]: [BRAVIA 4K VH2]\n\
                   [ro.empty]: []\n";
        let props = parse_getprop(raw);
        assert_eq!(props.len(), 3);
        assert_eq!(props[1], ("ro.product.model".into(), "BRAVIA 4K VH2".into()));
        assert_eq!(props[2].1, "");
    }

    #[test]
    fn label_prefers_model_number_and_avoids_doubled_brand() {
        let mut s = DeviceSummary {
            manufacturer: "Sony".into(),
            model: "BRAVIA 4K VH2".into(),
            model_number: "XR-65A80J".into(),
            ..Default::default()
        };
        assert_eq!(s.label(), "Sony XR-65A80J");
        s.model_number.clear();
        assert_eq!(s.label(), "Sony BRAVIA 4K VH2");
        s.manufacturer = "Google".into();
        s.model = "Google TV Streamer".into();
        assert_eq!(s.label(), "Google TV Streamer");
    }

    /// Talks to a real TV: `STT_TEST_TV=192.168.1.78:5555 cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn live_connect() {
        let serial = std::env::var("STT_TEST_TV").expect("set STT_TEST_TV=ip:port");
        let adb = adb::find().expect("adb not found");
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        rt.block_on(async {
            println!("{}", start_server(adb.clone()).await.expect("adb start"));
            match connect(adb, serial).await {
                ConnectResult::Connected { summary, .. } => {
                    println!("{summary:?}\nlabel: {}", summary.label());
                    assert!(!summary.manufacturer.is_empty());
                }
                other => panic!("not connected: {other:?}"),
            }
        });
    }

    #[test]
    fn serial_defaults_port() {
        assert_eq!(serial_for(" 192.168.1.78 ", ""), "192.168.1.78:5555");
        assert_eq!(serial_for("192.168.1.78", "5556"), "192.168.1.78:5556");
        assert_eq!(serial_for("192.168.1.78:40000", "5555"), "192.168.1.78:40000");
    }
}
