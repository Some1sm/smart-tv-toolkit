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

/// Try every (ip, port) at once; report which accepted a TCP connection.
pub async fn probe(targets: Vec<(Ipv4Addr, u16)>, timeout: Duration) -> Vec<(Ipv4Addr, u16, bool)> {
    let tasks: Vec<_> = targets
        .into_iter()
        .map(|(ip, port)| {
            tokio::spawn(async move {
                let open = tokio::time::timeout(timeout, tokio::net::TcpStream::connect((ip, port)))
                    .await
                    .is_ok_and(|r| r.is_ok());
                (ip, port, open)
            })
        })
        .collect();
    let mut results = Vec::with_capacity(tasks.len());
    for task in tasks {
        if let Ok(result) = task.await {
            results.push(result);
        }
    }
    results
}

/// Parse `aa:bb:cc:dd:ee:ff` (or with dashes).
pub fn parse_mac(mac: &str) -> Option<[u8; 6]> {
    let parts: Vec<u8> = mac.trim().split([':', '-']).map(|p| u8::from_str_radix(p, 16)).collect::<Result<_, _>>().ok()?;
    parts.try_into().ok()
}

/// A Wake-on-LAN "magic packet": six 0xFF bytes, then the MAC 16 times.
pub fn magic_packet(mac: [u8; 6]) -> Vec<u8> {
    let mut packet = vec![0xFF; 6];
    for _ in 0..16 {
        packet.extend_from_slice(&mac);
    }
    packet
}

/// Send a magic packet to the whole network (and the TV's /24 broadcast)
/// on the usual ports. TVs only wake for it when their network standby /
/// "remote start" option is on, and usually only over Ethernet.
pub fn wake_on_lan(mac: &str, ip: Option<Ipv4Addr>) -> Result<(), String> {
    let mac = parse_mac(mac).ok_or_else(|| format!("not a MAC address: {mac}"))?;
    let packet = magic_packet(mac);
    let socket = UdpSocket::bind("0.0.0.0:0").map_err(|e| e.to_string())?;
    socket.set_broadcast(true).map_err(|e| e.to_string())?;
    let mut targets = vec![Ipv4Addr::BROADCAST];
    if let Some(ip) = ip {
        let [a, b, c, _] = ip.octets();
        targets.push(Ipv4Addr::new(a, b, c, 255));
    }
    for target in targets {
        for port in [9u16, 7] {
            socket.send_to(&packet, (target, port)).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// Every host address of a /24.
pub fn hosts(prefix: [u8; 3]) -> impl Iterator<Item = Ipv4Addr> {
    (1..=254u8).map(move |host| Ipv4Addr::new(prefix[0], prefix[1], prefix[2], host))
}

/// Try `port` on every host of a /24 at once; return the ones that accept.
pub async fn scan(prefix: [u8; 3], port: u16, timeout: Duration) -> Vec<Ipv4Addr> {
    let targets = hosts(prefix).map(|ip| (ip, port)).collect();
    probe(targets, timeout).await.into_iter().filter_map(|(ip, _, open)| open.then_some(ip)).collect()
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
    fn wake_on_lan_packets() {
        assert_eq!(parse_mac("14:3f:a6:cd:2e:b3"), Some([0x14, 0x3f, 0xa6, 0xcd, 0x2e, 0xb3]));
        assert_eq!(parse_mac("14-3F-A6-CD-2E-B3"), Some([0x14, 0x3f, 0xa6, 0xcd, 0x2e, 0xb3]));
        assert_eq!(parse_mac("nope"), None);
        let p = magic_packet([1, 2, 3, 4, 5, 6]);
        assert_eq!(p.len(), 102);
        assert_eq!(&p[..6], &[0xFF; 6]);
        assert_eq!(&p[96..], &[1, 2, 3, 4, 5, 6]);
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
