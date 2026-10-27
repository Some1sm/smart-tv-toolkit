//! Running processes, Task Manager style: CPU and memory per process from
//! `top`, sampled over one second.

use std::time::Duration;

use super::Connection;

#[derive(Debug, Clone, PartialEq)]
pub struct Process {
    pub pid: u32,
    pub user: String,
    /// Share of the whole CPU (all cores), 0–100.
    pub cpu: f32,
    /// Resident memory in bytes.
    pub memory: u64,
    pub name: String,
}

impl Process {
    /// Android app processes run as `u0_a123`; their name is the package,
    /// sometimes with `:service` after it.
    pub fn package(&self) -> Option<&str> {
        self.user.starts_with("u0_a").then(|| self.name.split(':').next().unwrap_or(&self.name))
    }

    pub fn is_kernel_thread(&self) -> bool {
        self.name.starts_with('[')
    }
}

/// `top` sizes: `330M`, `1.9M`, `512K`, `1.2G` or plain KiB.
fn parse_size(s: &str) -> Option<u64> {
    let (number, unit) = match s.chars().last()? {
        c if c.is_ascii_digit() => (s, 1024.0),
        'K' | 'k' => (&s[..s.len() - 1], 1024.0),
        'M' | 'm' => (&s[..s.len() - 1], 1024.0 * 1024.0),
        'G' | 'g' => (&s[..s.len() - 1], 1024.0 * 1024.0 * 1024.0),
        _ => return None,
    };
    Some((number.parse::<f64>().ok()? * unit) as u64)
}

/// Parse `top -b -n 1 -o PID,USER,%CPU,RES,NAME`. `top` counts 100% per
/// core, so CPU is divided by the core count from its `400%cpu` line.
pub fn parse_top(out: &str) -> Vec<Process> {
    let cores = out
        .lines()
        .find_map(|l| l.split_whitespace().next()?.strip_suffix("%cpu")?.parse::<f32>().ok())
        .map_or(1.0, |total| (total / 100.0).max(1.0));
    let mut lines = out.lines().skip_while(|l| !l.trim_start().starts_with("PID"));
    lines.next(); // the header
    lines
        .filter_map(|l| {
            let mut f = l.split_whitespace();
            let pid = f.next()?.parse().ok()?;
            let user = f.next()?.to_owned();
            let cpu = f.next()?.parse::<f32>().ok()? / cores;
            let memory = parse_size(f.next()?)?;
            let name = f.collect::<Vec<_>>().join(" ");
            (!name.is_empty()).then_some(Process { pid, user, cpu, memory, name })
        })
        // Leave out the `top` we just ran.
        .filter(|p| !(p.user == "shell" && (p.name == "top" || p.name == "sh")))
        .collect()
}

pub async fn list(conn: Connection) -> Result<Vec<Process>, String> {
    let out = conn
        .shell("top -b -n 1 -d 1 -o PID,USER,%CPU,RES,NAME -s 3", Duration::from_secs(15))
        .await
        .map_err(|e| e.to_string())?;
    let list = parse_top(&out);
    if list.is_empty() { Err("top printed no processes".into()) } else { Ok(list) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_toybox_top() {
        let out = "Tasks: 399 total,   1 running\n\
                   400%cpu 117%user   7%nice  87%sys 190%idle\n\
                   \x20 PID USER         [%CPU  RES NAME\n\
                   \x203079 system       83.3 330M system_server\n\
                   \x205069 u0_a144       3.3 148M com.google.android.apps.tv.launcherx\n\
                   \x205129 u0_a144       0.0 1.5G com.google.android.apps.tv.launcherx:coreservices\n\
                   20449 shell        20.0 3.7M top\n\
                   \x201002 root          3.3    0 [sugov:0]\n";
        let p = parse_top(out);
        assert_eq!(p.len(), 4);
        assert_eq!(p[0].pid, 3079);
        assert!((p[0].cpu - 20.825).abs() < 0.01, "{}", p[0].cpu);
        assert_eq!(p[0].memory, 330 * 1024 * 1024);
        assert_eq!(p[0].package(), None);
        assert_eq!(p[2].package(), Some("com.google.android.apps.tv.launcherx"));
        assert_eq!(p[2].memory, (1.5 * 1024.0 * 1024.0 * 1024.0) as u64);
        assert!(p[3].is_kernel_thread());
        assert_eq!(parse_size("512K"), Some(512 * 1024));
        assert_eq!(parse_size("2048"), Some(2048 * 1024));
    }
}
