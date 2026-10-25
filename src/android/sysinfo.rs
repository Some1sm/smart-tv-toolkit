//! The Info tab: everything readable by the unprivileged ADB `shell` user,
//! gathered in two calls (a `getprop` dump and one batched script whose
//! sections are tagged with `@@name` marker lines).

use std::collections::HashMap;

use super::{Connection, parse_getprop, transport};

const SCRIPT: &str = "echo @@meminfo; cat /proc/meminfo; \
    echo @@df; df -k 2>/dev/null; \
    echo @@cpuinfo; cat /proc/cpuinfo; \
    echo @@maxfreq; cat /sys/devices/system/cpu/cpu*/cpufreq/cpuinfo_max_freq 2>/dev/null; \
    echo @@uptime; cat /proc/uptime; \
    echo @@kernel; uname -r; \
    echo @@wmsize; wm size; \
    echo @@density; wm density; \
    echo @@ip; ip -o -4 addr show 2>/dev/null; \
    echo @@battery; dumpsys battery 2>/dev/null; \
    echo @@ime; dumpsys input_method 2>/dev/null | grep -m1 mCurMethodId; \
    echo @@dns; settings get global private_dns_mode; \
    echo @@pkgv; settings get global package_verifier_user_consent; \
    echo @@gles; dumpsys SurfaceFlinger 2>/dev/null | grep -m1 GLES";

pub const KB: u64 = 1024;

/// Split marker-tagged script output into `{section: text}`.
pub fn split_sections(raw: &str) -> HashMap<String, String> {
    let mut sections = HashMap::new();
    let mut current: Option<(String, Vec<&str>)> = None;
    for line in raw.lines() {
        let line = line.trim_end_matches('\r');
        if let Some(name) = line.strip_prefix("@@") {
            if let Some((n, buf)) = current.take() {
                sections.insert(n, buf.join("\n"));
            }
            current = Some((name.trim().to_owned(), Vec::new()));
        } else if let Some((_, buf)) = current.as_mut() {
            buf.push(line);
        }
    }
    if let Some((n, buf)) = current {
        sections.insert(n, buf.join("\n"));
    }
    sections
}

pub fn fmt_bytes(n: u64) -> String {
    let mut v = n as f64;
    for unit in ["B", "KB", "MB", "GB"] {
        if v < 1024.0 {
            return if unit == "B" { format!("{v:.0} B") } else { format!("{v:.1} {unit}") };
        }
        v /= 1024.0;
    }
    format!("{v:.2} TB")
}

/// `/proc/meminfo` → `{key: bytes}` (the file reports kB).
pub fn parse_meminfo(text: &str) -> HashMap<String, u64> {
    text.lines()
        .filter_map(|line| {
            let (key, rest) = line.split_once(':')?;
            let kb: u64 = rest.split_whitespace().next()?.parse().ok()?;
            Some((key.trim().to_owned(), kb * KB))
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DfRow {
    pub mount: String,
    pub total: u64,
    pub used: u64,
    pub avail: u64,
}

/// `df -k` → rows in bytes. The header and malformed lines are skipped.
pub fn parse_df(text: &str) -> Vec<DfRow> {
    text.lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() < 6 {
                return None;
            }
            let num = |i: usize| parts[i].parse::<u64>().ok().map(|v| v * KB);
            Some(DfRow { mount: parts[5..].join(" "), total: num(1)?, used: num(2)?, avail: num(3)? })
        })
        .collect()
}

/// Internal storage (the /data partition) and external volumes (USB drives,
/// SD cards, which appear as /storage/<UUID> and/or /mnt/media_rw/<UUID>,
/// deduplicated by UUID).
pub fn storage_volumes(rows: &[DfRow]) -> (Option<DfRow>, Vec<DfRow>) {
    // Some TVs only list /data through a bind mount such as /data/user/0.
    let internal = rows
        .iter()
        .find(|r| r.mount == "/data")
        .or_else(|| rows.iter().find(|r| r.mount.starts_with("/data/")))
        .cloned();
    let mut external: Vec<DfRow> = Vec::new();
    for row in rows {
        for prefix in ["/storage/", "/mnt/media_rw/"] {
            if let Some(name) = row.mount.strip_prefix(prefix)
                && !name.is_empty()
                && !name.contains('/')
                && name != "emulated"
                && name != "self"
                && row.total > 0
                && !external.iter().any(|e| e.mount == name)
            {
                external.push(DfRow { mount: name.to_owned(), ..row.clone() });
            }
        }
    }
    (internal, external)
}

pub fn fmt_usage(total: u64, used: u64, avail: u64) -> String {
    let pct = if total > 0 { (used as f64 / total as f64 * 100.0).round() } else { 0.0 };
    format!("{} free of {}  ({pct:.0}% used)", fmt_bytes(avail), fmt_bytes(total))
}

/// Core count and per-cluster max clock: `4 cores @ 1.9 GHz`, or
/// `6 cores (2× 2.0 GHz + 4× 1.5 GHz)` on big.LITTLE chips.
pub fn parse_cpu(cpuinfo: &str, maxfreq: &str) -> String {
    let mut cores = cpuinfo
        .lines()
        .filter(|l| l.split(':').next().is_some_and(|k| k.trim() == "processor"))
        .count();
    let freqs: Vec<u64> = maxfreq.split_whitespace().filter_map(|f| f.parse().ok()).collect();
    if cores == 0 {
        cores = freqs.len();
    }
    if cores == 0 {
        return "—".into();
    }
    let mut summary = format!("{cores} core{}", if cores == 1 { "" } else { "s" });
    let mut clusters: Vec<(u64, usize)> = Vec::new();
    for f in freqs {
        match clusters.iter_mut().find(|(freq, _)| *freq == f) {
            Some((_, n)) => *n += 1,
            None => clusters.push((f, 1)),
        }
    }
    clusters.sort_by(|a, b| b.0.cmp(&a.0));
    let ghz = |khz: u64| khz as f64 / 1_000_000.0;
    match clusters.as_slice() {
        [] => {}
        [(f, _)] => summary.push_str(&format!(" @ {:.1} GHz", ghz(*f))),
        many => {
            let parts: Vec<String> = many.iter().map(|(f, n)| format!("{n}× {:.1} GHz", ghz(*f))).collect();
            summary.push_str(&format!(" ({})", parts.join(" + ")));
        }
    }
    summary
}

/// The `Hardware` line older ARM kernels expose in /proc/cpuinfo.
pub fn cpuinfo_hardware(cpuinfo: &str) -> String {
    cpuinfo
        .lines()
        .filter_map(|l| l.split_once(':'))
        .find(|(k, v)| k.trim() == "Hardware" && !v.trim().is_empty())
        .map(|(_, v)| v.trim().to_owned())
        .unwrap_or_default()
}

pub fn fmt_uptime(text: &str) -> String {
    let Some(secs) = text.split_whitespace().next().and_then(|s| s.parse::<f64>().ok()) else {
        return "—".into();
    };
    let secs = secs as u64;
    let (d, h, m) = (secs / 86_400, secs % 86_400 / 3_600, secs % 3_600 / 60);
    if d > 0 { format!("{d}d {h}h {m}m") } else { format!("{h}h {m}m") }
}

/// `ip -o -4 addr show` → `wlan0 192.168.1.20, eth0 10.0.0.5` (no loopback).
pub fn parse_ip_addrs(text: &str) -> String {
    text.lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let _index = parts.next()?;
            let iface = parts.next()?;
            if parts.next()? != "inet" || iface == "lo" {
                return None;
            }
            let addr = parts.next()?.split('/').next()?;
            Some(format!("{iface} {addr}"))
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// `wm density` → `320 dpi`; an override wins over the physical value.
pub fn parse_density(text: &str) -> String {
    let (mut physical, mut over) = (String::new(), String::new());
    for line in text.lines() {
        let val = line.rsplit(':').next().unwrap_or("").trim().to_owned();
        if line.contains("Override") {
            over = val;
        } else if line.contains("Physical") {
            physical = val;
        }
    }
    match (physical.is_empty(), over.is_empty()) {
        (_, false) if over != physical => format!("{over} dpi (physical {physical})"),
        (false, _) => format!("{physical} dpi"),
        _ => "—".into(),
    }
}

/// `wm size` → `3840x2160`, plus the size the UI is drawn at when it differs
/// (4K TVs often render their interface at 1080p).
fn parse_resolution(text: &str) -> String {
    let find = |key: &str| {
        text.lines().find(|l| l.contains(key)).map(|l| l.rsplit(':').next().unwrap_or("").trim().to_owned())
    };
    match (find("Physical size"), find("Override size")) {
        (Some(p), Some(o)) if p != o => format!("{p} (UI at {o})"),
        (Some(p), _) => p,
        (None, Some(o)) => o,
        (None, None) => "—".into(),
    }
}

fn parse_battery(text: &str) -> String {
    let mut level = None;
    for line in text.lines().map(str::trim) {
        if line == "present: false" {
            return "None (mains powered)".into();
        }
        if let Some(v) = line.strip_prefix("level:") {
            level = Some(format!("{}%", v.trim()));
        }
    }
    level.unwrap_or_else(|| "—".into())
}

/// One row of the Info tab.
#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    pub label: &'static str,
    pub value: String,
    /// Fraction used (0–1) for rows that get a usage bar.
    pub usage: Option<f32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Section {
    pub title: &'static str,
    pub fields: Vec<Field>,
}

fn field(label: &'static str, value: impl Into<String>) -> Field {
    let value = value.into();
    let value = if value.trim().is_empty() { "—".to_owned() } else { value };
    Field { label, value, usage: None }
}

/// Build the Info tab from a `getprop` dump and the script's sections.
pub fn build(props: &[(String, String)], sec: &HashMap<String, String>) -> Vec<Section> {
    let prop = |k: &str| props.iter().find(|(key, _)| key == k).map(|(_, v)| v.as_str()).unwrap_or("");
    let s = |k: &str| sec.get(k).map(String::as_str).unwrap_or("");

    // Device
    let mut model_number = prop("ro.boot.product.hardware.sku").to_owned();
    if !model_number.is_empty()
        && let Ok(year) = prop("ro.boot.svp.modelyear").parse::<u32>()
    {
        model_number.push_str(&format!("  ({})", if year < 100 { 2000 + year } else { year }));
    }

    // Memory & storage
    let mem = parse_meminfo(s("meminfo"));
    let ram_total = mem.get("MemTotal").copied().unwrap_or(0);
    let ram_avail = mem.get("MemAvailable").copied().unwrap_or_else(|| {
        mem.get("MemFree").copied().unwrap_or(0) + mem.get("Cached").copied().unwrap_or(0)
    });
    let ram = if ram_total > 0 {
        Field {
            label: "RAM",
            value: format!("{} available of {}", fmt_bytes(ram_avail), fmt_bytes(ram_total)),
            usage: Some(1.0 - ram_avail as f32 / ram_total as f32),
        }
    } else {
        field("RAM", "")
    };
    let swap_total = mem.get("SwapTotal").copied().unwrap_or(0);
    let swap = if swap_total > 0 {
        let used = swap_total.saturating_sub(mem.get("SwapFree").copied().unwrap_or(0));
        format!("{} used of {}", fmt_bytes(used), fmt_bytes(swap_total))
    } else {
        "None".into()
    };
    let (internal, external) = storage_volumes(&parse_df(s("df")));
    let storage = match internal.filter(|r| r.total > 0) {
        Some(r) => Field {
            label: "Internal Storage",
            value: fmt_usage(r.total, r.used, r.avail),
            usage: Some(r.used as f32 / r.total as f32),
        },
        None => field("Internal Storage", ""),
    };
    let ext = if external.is_empty() {
        "None mounted".to_owned()
    } else {
        external.iter().map(|r| format!("{}: {}", r.mount, fmt_usage(r.total, r.used, r.avail))).collect::<Vec<_>>().join("\n")
    };

    // Hardware
    let cpuinfo = s("cpuinfo");
    let soc = if !prop("ro.soc.model").is_empty() {
        format!("{} {}", prop("ro.soc.manufacturer"), prop("ro.soc.model")).trim().to_owned()
    } else {
        [cpuinfo_hardware(cpuinfo), prop("ro.board.platform").into(), prop("ro.hardware").into()]
            .into_iter()
            .find(|v| !v.is_empty())
            .unwrap_or_default()
    };
    let gles = s("gles").trim();
    let gpu = if gles.starts_with("GLES") { gles.split_once(':').map(|(_, v)| v.trim()).unwrap_or("") } else { "" };
    let abilist = prop("ro.product.cpu.abilist");
    let abi = if abilist.is_empty() { prop("ro.product.cpu.abi").to_owned() } else { abilist.replace(',', ", ") };

    // Software
    let (release, sdk) = (prop("ro.build.version.release"), prop("ro.build.version.sdk"));
    let android = if !release.is_empty() && !sdk.is_empty() { format!("{release} (API {sdk})") } else { release.into() };

    // Network & settings
    let ime = s("ime").rsplit('=').next().unwrap_or("").trim().to_owned();

    vec![
        Section {
            title: "Device",
            fields: vec![
                field("Manufacturer", prop("ro.product.manufacturer")),
                field("Model", prop("ro.product.model")),
                field("Model Number", model_number),
                field("Codename", prop("ro.product.device")),
                field("Serial Number", prop("ro.serialno")),
                field("Uptime", fmt_uptime(s("uptime"))),
            ],
        },
        Section {
            title: "Memory & Storage",
            fields: vec![ram, field("Swap / zRAM", swap), storage, field("External Storage", ext)],
        },
        Section {
            title: "Hardware",
            fields: vec![
                field("Chipset", soc),
                field("CPU", parse_cpu(cpuinfo, s("maxfreq"))),
                field("CPU ABI", abi),
                field("GPU", gpu),
                field("Resolution", parse_resolution(s("wmsize"))),
                field("Display Density", parse_density(s("density"))),
                field("Battery", parse_battery(s("battery"))),
            ],
        },
        Section {
            title: "Software",
            fields: vec![
                field("Android Version", android),
                field("Security Patch", prop("ro.build.version.security_patch")),
                field("Build / Firmware", prop("ro.build.display.id")),
                field("Kernel", s("kernel").trim()),
            ],
        },
        Section {
            title: "Network & Settings",
            fields: vec![
                field("IP Address", parse_ip_addrs(s("ip"))),
                field("DNS Mode", s("dns").trim()),
                field("Package Verifier", s("pkgv").trim()),
                field("Input Method", ime),
            ],
        },
    ]
}

/// Read everything for the Info tab from the TV.
pub async fn collect(conn: Connection) -> Result<Vec<Section>, String> {
    let props = conn.shell("getprop", transport::SHORT).await.map_err(|e| e.to_string())?;
    let raw = conn
        .shell(SCRIPT, std::time::Duration::from_secs(30))
        .await
        .map_err(|e| e.to_string())?;
    Ok(build(&parse_getprop(&props), &split_sections(&raw)))
}

/// Plain-text copy of the Info tab, for the clipboard.
pub fn to_text(sections: &[Section]) -> String {
    sections
        .iter()
        .map(|s| {
            let fields: Vec<String> =
                s.fields.iter().map(|f| format!("{}: {}", f.label, f.value.replace('\n', "; "))).collect();
            format!("[{}]\n{}", s.title, fields.join("\n"))
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    // Trimmed from real Android TV devices (toybox df, /proc files).
    const SYSINFO: &str = "\
@@meminfo
MemTotal:        1999324 kB
MemFree:          123456 kB
MemAvailable:     999662 kB
Cached:           400000 kB
SwapTotal:        999660 kB
SwapFree:         499830 kB
@@df
Filesystem                 1K-blocks    Used Available Use% Mounted on
/dev/root                    1548636 1530000     2000  99% /
tmpfs                         999662    1200    998462   1% /dev
/dev/block/dm-5              4194304 1048576   3145728  25% /data
/dev/fuse                    4194304 1048576   3145728  25% /storage/emulated
/dev/block/vold/public:8,1  30000000 10000000 20000000  34% /mnt/media_rw/1A2B-3C4D
/dev/fuse                   30000000 10000000 20000000  34% /storage/1A2B-3C4D
@@cpuinfo
processor\t: 0
processor\t: 1
processor\t: 2
processor\t: 3
Hardware\t: Amlogic
@@maxfreq
1908000
1908000
1908000
1908000
@@uptime
273845.12 1000000.00
@@kernel
5.4.210-android12
@@wmsize
Physical size: 1920x1080
@@density
Physical density: 320
@@ip
1: lo    inet 127.0.0.1/8 scope host lo\\       valid_lft forever
9: wlan0    inet 192.168.1.20/24 brd 192.168.1.255 scope global wlan0
@@battery
Current Battery Service state:
  present: false
  level: 100
@@ime
  mCurMethodId=com.google.android.tvkeyboard/.Ime
@@dns
opportunistic
@@pkgv
1
@@gles
GLES: ARM, Mali-G31, OpenGL ES 3.2 v1.r26p0
";

    fn sections() -> HashMap<String, String> {
        split_sections(SYSINFO)
    }

    #[test]
    fn sections_and_meminfo() {
        let sec = sections();
        assert_eq!(sec.len(), 14);
        let mem = parse_meminfo(&sec["meminfo"]);
        assert_eq!(mem["MemTotal"], 1_999_324 * KB);
        assert_eq!(mem["MemAvailable"], 999_662 * KB);
    }

    #[test]
    fn storage_internal_and_external_deduped() {
        let (internal, external) = storage_volumes(&parse_df(&sections()["df"]));
        let internal = internal.unwrap();
        assert_eq!((internal.total, internal.used, internal.avail), (4_194_304 * KB, 1_048_576 * KB, 3_145_728 * KB));
        assert_eq!(external.iter().map(|e| e.mount.as_str()).collect::<Vec<_>>(), ["1A2B-3C4D"]);
        assert_eq!(fmt_usage(internal.total, internal.used, internal.avail), "3.0 GB free of 4.0 GB  (25% used)");
    }

    #[test]
    fn bind_mounted_data_is_still_internal() {
        let rows = parse_df("x 4277040 3504464 667780 84% /data/user/0");
        assert_eq!(storage_volumes(&rows).0.unwrap().avail, 667_780 * KB);
    }

    #[test]
    fn cpu_summary() {
        let sec = sections();
        assert_eq!(parse_cpu(&sec["cpuinfo"], &sec["maxfreq"]), "4 cores @ 1.9 GHz");
        assert_eq!(cpuinfo_hardware(&sec["cpuinfo"]), "Amlogic");
        assert_eq!(parse_cpu("", "2000000\n2000000\n1500000\n1500000\n1500000\n1500000"), "6 cores (2× 2.0 GHz + 4× 1.5 GHz)");
        assert_eq!(parse_cpu("", ""), "—");
    }

    #[test]
    fn misc_fields() {
        let sec = sections();
        assert_eq!(fmt_uptime(&sec["uptime"]), "3d 4h 4m");
        assert_eq!(parse_density(&sec["density"]), "320 dpi");
        assert_eq!(parse_density("Physical density: 320\nOverride density: 240"), "240 dpi (physical 320)");
        assert_eq!(parse_ip_addrs(&sec["ip"]), "wlan0 192.168.1.20");
        assert_eq!(parse_battery(&sec["battery"]), "None (mains powered)");
        assert_eq!(parse_resolution("Physical size: 3840x2160\nOverride size: 1920x1080"), "3840x2160 (UI at 1920x1080)");
        assert_eq!(fmt_bytes(512), "512 B");
        assert_eq!(fmt_bytes(1536 * 1024 * 1024), "1.5 GB");
    }

    #[test]
    fn builds_all_sections() {
        let props = parse_getprop(
            "[ro.product.manufacturer]: [Sony]\n[ro.product.model]: [BRAVIA 4K VH2]\n\
             [ro.boot.product.hardware.sku]: [XR-65A80J]\n[ro.boot.svp.modelyear]: [21]\n\
             [ro.build.version.release]: [12]\n[ro.build.version.sdk]: [31]\n\
             [ro.soc.manufacturer]: [Mediatek]\n[ro.soc.model]: [MT5835]\n\
             [ro.product.cpu.abilist]: [armeabi-v7a,armeabi]\n",
        );
        let built = build(&props, &sections());
        let get = |label: &str| {
            built.iter().flat_map(|s| &s.fields).find(|f| f.label == label).unwrap().clone()
        };
        assert_eq!(get("Model Number").value, "XR-65A80J  (2021)");
        assert_eq!(get("Android Version").value, "12 (API 31)");
        assert_eq!(get("Chipset").value, "Mediatek MT5835");
        assert_eq!(get("CPU ABI").value, "armeabi-v7a, armeabi");
        assert_eq!(get("GPU").value, "ARM, Mali-G31, OpenGL ES 3.2 v1.r26p0");
        assert_eq!(get("Input Method").value, "com.google.android.tvkeyboard/.Ime");
        assert_eq!(get("Resolution").value, "1920x1080");
        assert_eq!(get("Codename").value, "—");
        let ram = get("RAM");
        assert!((ram.usage.unwrap() - 0.5).abs() < 0.01);
        assert!(to_text(&built).starts_with("[Device]\nManufacturer: Sony\n"));
    }
}
