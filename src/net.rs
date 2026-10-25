//! Finding TVs on the local network.

use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::time::Duration;

/// This computer's LAN address. "Connecting" a UDP socket sends nothing; it
/// only makes the OS pick the interface it would route through.
pub fn local_ipv4() -> Option<Ipv4Addr> {
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:80").ok()?;
    match socket.local_addr().ok()? {
        SocketAddr::V4(addr) if !addr.ip().is_loopback() => Some(*addr.ip()),
        _ => None,
    }
}

/// `192.168.1.20` → `192.168.1`.
pub fn subnet_prefix(ip: Ipv4Addr) -> String {
    let [a, b, c, _] = ip.octets();
    format!("{a}.{b}.{c}")
}

/// Parse `192.168.1` (a trailing dot or `.0`/`.x` host part is tolerated).
pub fn parse_prefix(prefix: &str) -> Option<[u8; 3]> {
    let parts: Vec<&str> = prefix.trim().trim_end_matches('.').split('.').collect();
    let octets = match parts.as_slice() {
        [a, b, c] | [a, b, c, _] => [*a, *b, *c],
        _ => return None,
    };
    let mut out = [0u8; 3];
    for (slot, part) in out.iter_mut().zip(octets) {
        *slot = part.parse().ok()?;
    }
    Some(out)
}

/// Try `port` on every host of a /24 at once; return the ones that accept.
pub async fn scan(prefix: [u8; 3], port: u16, timeout: Duration) -> Vec<Ipv4Addr> {
    let probes = (1..=254u8).map(|host| {
        let ip = Ipv4Addr::new(prefix[0], prefix[1], prefix[2], host);
        tokio::spawn(async move {
            let open = tokio::time::timeout(timeout, tokio::net::TcpStream::connect((ip, port)))
                .await
                .is_ok_and(|r| r.is_ok());
            open.then_some(ip)
        })
    });
    let mut found = Vec::new();
    for probe in probes.collect::<Vec<_>>() {
        if let Ok(Some(ip)) = probe.await {
            found.push(ip);
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Scans the real network: `STT_TEST_TV=192.168.1.78:5555 cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn live_scan_finds_tv() {
        let tv = std::env::var("STT_TEST_TV").expect("set STT_TEST_TV=ip:port");
        let ip: Ipv4Addr = tv.split(':').next().unwrap().parse().unwrap();
        let prefix = parse_prefix(&subnet_prefix(ip)).unwrap();
        let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
        let start = std::time::Instant::now();
        let found = rt.block_on(scan(prefix, 5555, Duration::from_millis(600)));
        println!("found {found:?} in {:?}", start.elapsed());
        assert!(found.contains(&ip));
    }

    #[test]
    fn prefixes() {
        assert_eq!(subnet_prefix(Ipv4Addr::new(192, 168, 1, 20)), "192.168.1");
        assert_eq!(parse_prefix("192.168.1"), Some([192, 168, 1]));
        assert_eq!(parse_prefix(" 10.0.0. "), Some([10, 0, 0]));
        assert_eq!(parse_prefix("192.168.1.x"), Some([192, 168, 1]));
        assert_eq!(parse_prefix("192.168"), None);
        assert_eq!(parse_prefix("192.168.300"), None);
    }
}
