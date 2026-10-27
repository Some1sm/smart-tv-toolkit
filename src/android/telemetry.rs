//! Live telemetry for the sidebar card: CPU load, RAM in use and command
//! round-trip time, sampled every few seconds while connected.

use std::time::{Duration, Instant};

use super::Connection;
use super::sysinfo::{KB, parse_cpu, parse_meminfo, split_sections};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sample {
    /// Cumulative jiffies from /proc/stat's first line.
    pub cpu_total: u64,
    pub cpu_idle: u64,
    pub mem_total: u64,
    pub mem_available: u64,
    /// How long the sampling command took, there and back.
    pub latency: Duration,
}

/// `cpu  user nice system idle iowait irq softirq steal …` → (total, idle).
fn parse_stat(line: &str) -> Option<(u64, u64)> {
    let mut parts = line.split_whitespace();
    if parts.next()? != "cpu" {
        return None;
    }
    let values: Vec<u64> = parts.filter_map(|v| v.parse().ok()).collect();
    if values.len() < 4 {
        return None;
    }
    // Guest time is already counted in user/nice, so stop at steal.
    let total = values.iter().take(8).sum();
    let idle = values[3] + values.get(4).copied().unwrap_or(0);
    Some((total, idle))
}

pub async fn sample(conn: Connection) -> Result<Sample, String> {
    let started = Instant::now();
    let out = conn
        .shell("head -n1 /proc/stat; grep -E '^(MemTotal|MemAvailable):' /proc/meminfo", Duration::from_secs(5))
        .await
        .map_err(|e| e.to_string())?;
    let latency = started.elapsed();
    let (cpu_total, cpu_idle) = out.lines().find_map(parse_stat).ok_or("no CPU counters")?;
    let mem = parse_meminfo(&out);
    Ok(Sample {
        cpu_total,
        cpu_idle,
        mem_total: mem.get("MemTotal").copied().unwrap_or(0),
        mem_available: mem.get("MemAvailable").copied().unwrap_or(0),
        latency,
    })
}

/// CPU busy share between two samples, 0–1.
pub fn cpu_load(previous: &Sample, current: &Sample) -> Option<f32> {
    let total = current.cpu_total.checked_sub(previous.cpu_total)?;
    let idle = current.cpu_idle.checked_sub(previous.cpu_idle)?;
    (total > 0).then(|| (1.0 - idle as f32 / total as f32).clamp(0.0, 1.0))
}

/// ARM core names by `CPU part` id, for the common TV chips.
fn core_name(part: &str) -> Option<&'static str> {
    Some(match part.trim().to_lowercase().as_str() {
        "0xd03" => "A53",
        "0xd04" => "A35",
        "0xd05" => "A55",
        "0xd07" => "A57",
        "0xd08" => "A72",
        "0xd09" => "A73",
        "0xd0a" => "A75",
        "0xd0b" => "A76",
        "0xd0d" => "A77",
        "0xd41" => "A78",
        "0xd46" => "A510",
        "0xd47" => "A710",
        _ => return None,
    })
}

/// "4-core A73 @ 1.8GHz", from /proc/cpuinfo and the max clocks.
pub fn describe_cpu(cpuinfo: &str, maxfreq: &str) -> String {
    let summary = parse_cpu(cpuinfo, maxfreq); // "4 cores @ 1.8 GHz" or "8 cores (…)"
    let cores = summary.split_whitespace().next().unwrap_or("?");
    let core = cpuinfo
        .lines()
        .filter_map(|l| l.split_once(':'))
        .filter(|(k, _)| k.trim() == "CPU part")
        .find_map(|(_, v)| core_name(v));
    let max_ghz = maxfreq
        .split_whitespace()
        .filter_map(|f| f.parse::<u64>().ok())
        .max()
        .map(|khz| format!(" @ {:.1}GHz", khz as f64 / 1_000_000.0))
        .unwrap_or_default();
    match core {
        Some(core) => format!("{cores}-core {core}{max_ghz}"),
        None => format!("{cores}-core{max_ghz}"),
    }
}

/// Read the CPU description once per connection.
pub async fn cpu_description(conn: Connection) -> String {
    let script = "echo @@cpuinfo; cat /proc/cpuinfo; echo @@maxfreq; \
                  cat /sys/devices/system/cpu/cpu*/cpufreq/cpuinfo_max_freq 2>/dev/null";
    let out = conn.shell(script, Duration::from_secs(10)).await.unwrap_or_default();
    let sec = split_sections(&out);
    describe_cpu(sec.get("cpuinfo").map(String::as_str).unwrap_or(""), sec.get("maxfreq").map(String::as_str).unwrap_or(""))
}

/// Bytes → "2.4 GB" with one decimal, for the RAM gauge.
pub fn gb(bytes: u64) -> String {
    format!("{:.1}", bytes as f64 / (KB * KB * KB) as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_load_between_samples() {
        let (t1, i1) = parse_stat("cpu  100 0 50 800 50 0 0 0 0 0").unwrap();
        let (t2, i2) = parse_stat("cpu  130 0 70 840 60 0 0 0 0 0").unwrap();
        let s = |t, i| Sample { cpu_total: t, cpu_idle: i, mem_total: 0, mem_available: 0, latency: Duration::ZERO };
        let load = cpu_load(&s(t1, i1), &s(t2, i2)).unwrap();
        assert!((load - 0.5).abs() < 1e-6, "{load}"); // 100 jiffies, 50 idle
        assert_eq!(parse_stat("intr 1 2 3"), None);
    }

    /// Read-only: telemetry next to the Info read, as on connect.
    /// `STT_TEST_TV=ip:port cargo test live_telemetry -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn live_telemetry() {
        let serial = std::env::var("STT_TEST_TV").expect("set STT_TEST_TV=ip:port");
        let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
        rt.block_on(async {
            let conn = super::super::transport::connect(serial).await.expect("connect");
            let (cpu, first, info) = tokio::join!(
                cpu_description(conn.clone()),
                sample(conn.clone()),
                super::super::sysinfo::collect(conn.clone())
            );
            println!("cpu: {cpu}
first: {first:?}
info ok: {}", info.is_ok());
            if let Err(e) = &info {
                println!("info error: {e}");
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
            let second = sample(conn).await;
            println!("second: {second:?}");
            if let (Ok(a), Ok(b)) = (&first, &second) {
                println!("load: {:?}", cpu_load(a, b));
            }
        });
    }

    #[test]
    fn describes_the_cpu() {
        let info = "processor\t: 0\nCPU part\t: 0xd09\nprocessor\t: 1\nprocessor\t: 2\nprocessor\t: 3\n";
        assert_eq!(describe_cpu(info, "1800000\n1800000\n1800000\n1800000"), "4-core A73 @ 1.8GHz");
        assert_eq!(describe_cpu("processor\t: 0\n", ""), "1-core");
    }
}
