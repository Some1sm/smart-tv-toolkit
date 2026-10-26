//! Checking GitHub for a newer release of the app (once a day).

const LATEST: &str = "https://api.github.com/repos/Some1sm/smart-tv-toolkit/releases/latest";

/// `v1.2.10` / `1.2.10` → [1, 2, 10]; pre-release suffixes are ignored.
fn parse(version: &str) -> Vec<u64> {
    version
        .trim()
        .trim_start_matches('v')
        .split(['-', '+'])
        .next()
        .unwrap_or("")
        .split('.')
        .map(|p| p.parse().unwrap_or(0))
        .collect()
}

pub fn is_newer(latest: &str, current: &str) -> bool {
    let (mut a, mut b) = (parse(latest), parse(current));
    let len = a.len().max(b.len());
    a.resize(len, 0);
    b.resize(len, 0);
    a > b
}

/// The newest published release, as (version, page URL), if it's newer than
/// this build. Network errors and "no releases yet" both mean no notice.
pub async fn check() -> Option<(String, String)> {
    tokio::task::spawn_blocking(|| {
        let mut resp = ureq::get(LATEST)
            .header("User-Agent", concat!("SmartTVToolkit/", env!("CARGO_PKG_VERSION")))
            .header("Accept", "application/vnd.github+json")
            .call()
            .ok()?;
        let release: serde_json::Value = serde_json::from_str(&resp.body_mut().read_to_string().ok()?).ok()?;
        let tag = release["tag_name"].as_str()?;
        let url = release["html_url"].as_str()?;
        is_newer(tag, env!("CARGO_PKG_VERSION")).then(|| (tag.trim_start_matches('v').to_owned(), url.to_owned()))
    })
    .await
    .ok()
    .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_versions_numerically() {
        assert!(is_newer("v0.2.0", "0.1.0"));
        assert!(is_newer("0.1.10", "0.1.9"));
        assert!(is_newer("v1.0", "0.9.9"));
        assert!(!is_newer("v0.1.0", "0.1.0"));
        assert!(!is_newer("0.1.0", "0.1.1"));
        assert!(!is_newer("v0.1.0-beta", "0.1.0"));
    }
}
