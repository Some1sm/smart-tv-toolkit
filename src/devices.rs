//! The sidebar's device list: saved devices, scan results and mDNS
//! announcements merged into one entry per IP.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::net::Ipv4Addr;

use chrono::NaiveDateTime;

use crate::history::{self, History};
use crate::mdns::MdnsDevice;
use crate::i18n::tr;
use crate::platform::Platform;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reach {
    /// ADB answered on its port.
    Ready,
    /// Seen on the network, but ADB is closed (network debugging is off).
    DebuggingOff,
    /// Seen, but only offers Android 11+ wireless debugging (needs pairing).
    NeedsPairing,
    /// Checked and not answering.
    Offline,
    /// Not checked yet.
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub ip: Ipv4Addr,
    pub port: u16,
    pub title: String,
    /// e.g. "“Televisión” · Port 5555 open · seen 3h ago" (no address).
    pub status: String,
    pub reach: Reach,
    pub saved: bool,
}

/// Merge everything known into a list, reachable devices first.
///
/// `open` holds IPs where ADB answered; `checked` holds every IP whose port
/// has been probed (by a scan or the startup check), open or not. `others`
/// holds Samsung and LG TVs found by their remote-control port.
pub fn merge(
    history: &History,
    open: &BTreeSet<Ipv4Addr>,
    checked: &BTreeSet<Ipv4Addr>,
    others: &BTreeMap<Ipv4Addr, u16>,
    mdns: &[MdnsDevice],
    now: NaiveDateTime,
) -> Vec<Entry> {
    let mdns_by_ip: HashMap<Ipv4Addr, &MdnsDevice> =
        mdns.iter().filter(|d| d.is_tv()).filter_map(|d| Some((d.ip?, d))).collect();

    let mut ips: BTreeSet<Ipv4Addr> = history.keys().filter_map(|ip| ip.parse().ok()).collect();
    ips.extend(open);
    ips.extend(others.keys());
    ips.extend(mdns_by_ip.keys());

    let mut entries: Vec<Entry> = ips
        .into_iter()
        .map(|ip| {
            let saved = history.get(&ip.to_string());
            let announced = mdns_by_ip.get(&ip).copied();
            let mdns_name = announced.and_then(|d| d.name.clone());

            // A Samsung/LG remote API answering (and no ADB) makes it that kind of TV.
            let remote_api = others.get(&ip).copied().filter(|_| !open.contains(&ip));
            let platform = remote_api.map_or(Platform::AndroidTv, Platform::for_port);

            let title = saved
                .map(|s| s.label.clone())
                .filter(|l| !l.is_empty())
                .or_else(|| mdns_name.clone())
                .or_else(|| announced.and_then(|d| d.model.clone()))
                .unwrap_or_else(|| match platform {
                    Platform::Tizen => "Samsung TV".into(),
                    Platform::WebOs => "LG TV".into(),
                    Platform::AndroidTv => ip.to_string(),
                });

            let reach = if open.contains(&ip) || remote_api.is_some() {
                Reach::Ready
            } else if announced.is_some_and(|d| d.wireless_debugging) {
                Reach::NeedsPairing
            } else if announced.is_some() && checked.contains(&ip) {
                Reach::DebuggingOff
            } else if checked.contains(&ip) {
                Reach::Offline
            } else {
                Reach::Unknown
            };

            let port = remote_api
                .or_else(|| saved.map(|s| s.port))
                .or_else(|| announced.and_then(|d| d.adb_port.or(d.tls_port)))
                .unwrap_or(crate::android::ADB_PORT);

            let mut status = Vec::new();
            // Show the TV's own name when the title is the model from getprop.
            if let Some(name) = mdns_name.filter(|n| *n != title) {
                status.push(format!("“{name}”"));
            }
            status.push(match reach {
                Reach::Ready if remote_api.is_some() => format!("{} · port {port}", platform.name()),
                Reach::Ready => format!("Port {port} open"),
                Reach::DebuggingOff => tr("Debugging off").into(),
                Reach::NeedsPairing => tr("Needs pairing (Android 11+)").into(),
                Reach::Offline => tr("Offline").into(),
                Reach::Unknown => tr("Not checked yet").into(),
            });
            if matches!(reach, Reach::Ready | Reach::Offline | Reach::Unknown)
                && let Some(age) = saved.map(|s| history::age(&s.last_seen, now)).filter(|a| !a.is_empty())
            {
                status.push(crate::i18n::trf("seen {}", &[&age]));
            }

            Entry { ip, port, title, status: status.join(" · "), reach, saved: saved.is_some() }
        })
        .collect();

    let rank = |r: &Reach| match r {
        Reach::Ready => 0,
        Reach::NeedsPairing | Reach::DebuggingOff => 1,
        Reach::Unknown => 2,
        Reach::Offline => 3,
    };
    entries.sort_by(|a, b| rank(&a.reach).cmp(&rank(&b.reach)).then(a.title.cmp(&b.title)));
    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The real sidebar list: `STT_TEST_HISTORY=path/to/devices.json cargo test live_list -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn live_list() {
        let history = history::load_from(std::path::Path::new(
            &std::env::var("STT_TEST_HISTORY").expect("set STT_TEST_HISTORY"),
        ))
        .unwrap();
        let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
        let targets = history.iter().filter_map(|(ip, d)| Some((ip.parse().ok()?, d.port))).collect();
        let (probed, mdns) = rt.block_on(async {
            tokio::join!(
                crate::net::probe(targets, std::time::Duration::from_secs(1)),
                crate::mdns::discover(std::time::Duration::from_secs(3))
            )
        });
        let open = probed.iter().filter(|(_, _, o)| *o).map(|(ip, _, _)| *ip).collect();
        let checked = probed.iter().map(|(ip, _, _)| *ip).collect();
        for e in merge(&history, &open, &checked, &BTreeMap::new(), &mdns.unwrap(), chrono::Local::now().naive_local()) {
            println!("{:?} {} — {}", e.reach, e.title, e.status);
        }
    }

    fn ip(last: u8) -> Ipv4Addr {
        Ipv4Addr::new(192, 168, 1, last)
    }

    #[test]
    fn merges_history_scan_and_mdns() {
        let history: History = serde_json::from_str(
            r#"{"192.168.1.78": {"label": "Sony XR-65A80J", "port": 5555, "last_seen": "2026-10-26 09:00"},
                "192.168.1.20": {"label": "Google Chromecast", "port": 5555, "last_seen": "2026-10-25 22:43"}}"#,
        )
        .unwrap();
        let open = BTreeSet::from([ip(78), ip(99)]);
        let checked = BTreeSet::from([ip(78), ip(99), ip(20), ip(40)]);
        let mdns = vec![
            MdnsDevice { ip: Some(ip(78)), name: Some("Televisión".into()), android_tv: true, ..Default::default() },
            MdnsDevice { ip: Some(ip(40)), name: Some("Bedroom TV".into()), android_tv: true, ..Default::default() },
            MdnsDevice { ip: Some(ip(50)), name: Some("Kitchen speaker".into()), ..Default::default() },
        ];
        let now = NaiveDateTime::parse_from_str("2026-10-26 12:00", "%Y-%m-%d %H:%M").unwrap();
        let others = BTreeMap::from([(ip(60), 8002u16)]);
        let list = merge(&history, &open, &checked, &others, &mdns, now);

        let titles: Vec<&str> = list.iter().map(|e| e.title.as_str()).collect();
        // Ready first (by title), then debugging-off, then offline; the speaker is dropped.
        assert_eq!(titles, ["192.168.1.99", "Samsung TV", "Sony XR-65A80J", "Bedroom TV", "Google Chromecast"]);
        assert_eq!((list[1].port, list[1].status.as_str()), (8002, "Samsung Tizen · port 8002"));
        let list: Vec<Entry> = list.into_iter().filter(|e| e.port != 8002).collect();
        let sony = &list[1];
        assert_eq!(sony.reach, Reach::Ready);
        assert_eq!(sony.status, "“Televisión” · Port 5555 open · seen 3h ago");
        assert!(sony.saved);
        assert_eq!(list[2].reach, Reach::DebuggingOff);
        assert_eq!(list[3].reach, Reach::Offline);
    }
}
