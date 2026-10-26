//! Finding TVs by what they announce over mDNS (Bonjour / zeroconf).
//!
//! Unlike the port scan this works before connecting, and gives the TV's
//! own name ("Living Room TV") without making it show any prompt.

use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::time::{Duration, Instant};

use mdns_sd::{ServiceDaemon, ServiceEvent};

/// Android TV Remote service: only Android TV / Google TV devices announce it,
/// so it tells TVs apart from cast-only devices such as speakers.
const ANDROID_TV_REMOTE: &str = "_androidtvremote2._tcp.local.";
/// Chromecast built-in; its TXT record carries the friendly name and model.
const GOOGLE_CAST: &str = "_googlecast._tcp.local.";
/// ADB over the network (classic network debugging).
const ADB: &str = "_adb._tcp.local.";
/// Android 11+ wireless debugging (needs pairing).
const ADB_TLS: &str = "_adb-tls-connect._tcp.local.";

const SERVICES: [&str; 4] = [ANDROID_TV_REMOTE, GOOGLE_CAST, ADB, ADB_TLS];

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MdnsDevice {
    pub ip: Option<Ipv4Addr>,
    /// The name set on the TV, e.g. "Living Room TV".
    pub name: Option<String>,
    pub model: Option<String>,
    pub android_tv: bool,
    pub adb_port: Option<u16>,
    pub wireless_debugging: bool,
    /// Android 11+ wireless debugging's connect port (used after pairing).
    pub tls_port: Option<u16>,
}

impl MdnsDevice {
    /// Worth listing: an Android TV, or anything with ADB enabled.
    pub fn is_tv(&self) -> bool {
        self.android_tv || self.adb_port.is_some() || self.wireless_debugging
    }
}

/// "Living Room TV._androidtvremote2._tcp.local." → "Living Room TV".
fn instance_name(fullname: &str, ty_domain: &str) -> String {
    fullname.strip_suffix(ty_domain).unwrap_or(fullname).trim_end_matches('.').replace("\\ ", " ")
}

fn record(found: &mut HashMap<Ipv4Addr, MdnsDevice>, ip: Ipv4Addr, kind: &str, instance: String,
          port: u16, friendly_name: Option<&str>, model: Option<&str>) {
    let d = found.entry(ip).or_insert_with(|| MdnsDevice { ip: Some(ip), ..Default::default() });
    match kind {
        ANDROID_TV_REMOTE => {
            d.android_tv = true;
            d.name.get_or_insert(instance);
        }
        GOOGLE_CAST => {
            // The cast name is what the user set on the TV, so it wins.
            if let Some(name) = friendly_name.filter(|n| !n.is_empty()) {
                d.name = Some(name.to_owned());
            }
            if let Some(model) = model.filter(|m| !m.is_empty()) {
                d.model = Some(model.to_owned());
            }
        }
        ADB => d.adb_port = Some(port),
        ADB_TLS => {
            d.wireless_debugging = true;
            d.tls_port = Some(port);
        }
        _ => {}
    }
}

/// Listen for `duration` and return what was heard, one entry per IP.
pub async fn discover(duration: Duration) -> Result<Vec<MdnsDevice>, String> {
    tokio::task::spawn_blocking(move || discover_blocking(duration))
        .await
        .map_err(|e| e.to_string())?
}

fn discover_blocking(duration: Duration) -> Result<Vec<MdnsDevice>, String> {
    let daemon = ServiceDaemon::new().map_err(|e| e.to_string())?;
    let receivers = SERVICES
        .iter()
        .map(|s| daemon.browse(s).map(|r| (*s, r)))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    let mut found = HashMap::new();
    let deadline = Instant::now() + duration;
    while Instant::now() < deadline {
        let mut idle = true;
        for (kind, receiver) in &receivers {
            while let Ok(event) = receiver.try_recv() {
                idle = false;
                if let ServiceEvent::ServiceResolved(svc) = event {
                    let instance = instance_name(svc.get_fullname(), kind);
                    for ip in svc.get_addresses_v4() {
                        record(&mut found, ip, kind, instance.clone(), svc.get_port(),
                               svc.get_property_val_str("fn"), svc.get_property_val_str("md"));
                    }
                }
            }
        }
        if idle {
            std::thread::sleep(Duration::from_millis(30));
        }
    }
    let _ = daemon.shutdown();

    let mut devices: Vec<MdnsDevice> = found.into_values().collect();
    devices.sort_by_key(|d| d.ip);
    Ok(devices)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instance_names() {
        assert_eq!(instance_name("Living Room TV._androidtvremote2._tcp.local.", ANDROID_TV_REMOTE),
                   "Living Room TV");
        assert_eq!(instance_name("Televisión._androidtvremote2._tcp.local.", ANDROID_TV_REMOTE), "Televisión");
    }

    #[test]
    fn cast_name_wins_and_speakers_are_not_tvs() {
        let ip = Ipv4Addr::new(192, 168, 1, 78);
        let mut found = HashMap::new();
        record(&mut found, ip, ANDROID_TV_REMOTE, "BRAVIA".into(), 6466, None, None);
        record(&mut found, ip, GOOGLE_CAST, "x".into(), 8009, Some("Televisión"), Some("BRAVIA 4K VH2"));
        record(&mut found, ip, ADB, "adb-123".into(), 5555, None, None);
        let tv = &found[&ip];
        assert_eq!(tv.name.as_deref(), Some("Televisión"));
        assert_eq!(tv.model.as_deref(), Some("BRAVIA 4K VH2"));
        assert!(tv.android_tv && tv.is_tv());
        assert_eq!(tv.adb_port, Some(5555));

        let speaker_ip = Ipv4Addr::new(192, 168, 1, 50);
        record(&mut found, speaker_ip, GOOGLE_CAST, "x".into(), 8009, Some("Kitchen"), Some("Google Nest Mini"));
        assert!(!found[&speaker_ip].is_tv());
    }

    /// Listens on the real network: `cargo test live_mdns -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn live_mdns() {
        let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
        let devices = rt.block_on(discover(Duration::from_secs(4))).unwrap();
        for d in &devices {
            println!("{d:?}  tv={}", d.is_tv());
        }
    }
}
