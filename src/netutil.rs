// SPDX-License-Identifier: GPL-3.0-or-later
//! Detect this host's reachable private (LAN) IPv4 address, for prefilling
//! the master address in generated network-client configs. Loopback is what
//! apctui itself polls; it's meaningless to any other machine.
//!
//! Range priority (per project spec): CGNAT 100.64.0.0/10 first (overlay
//! networks like Tailscale), then RFC1918 10/8, 172.16/12, 192.168/16.
//! Assumes static addressing on a local network.

use std::net::Ipv4Addr;

/// Private/locally-routable ranges we accept, in preference order.
const RANGES: &[(Ipv4Addr, u8)] = &[
    (Ipv4Addr::new(100, 64, 0, 0), 10), // CGNAT / Tailscale
    (Ipv4Addr::new(10, 0, 0, 0), 8),
    (Ipv4Addr::new(172, 16, 0, 0), 12),
    (Ipv4Addr::new(192, 168, 0, 0), 16),
];

fn in_range(ip: Ipv4Addr, net: Ipv4Addr, prefix: u8) -> bool {
    let mask = if prefix == 0 { 0 } else { u32::MAX << (32 - prefix) };
    (u32::from(ip) & mask) == (u32::from(net) & mask)
}

/// Index into RANGES, or None if not a usable private address.
fn range_rank(ip: Ipv4Addr) -> Option<usize> {
    if ip.is_loopback() || ip.is_link_local() || ip.is_unspecified() {
        return None;
    }
    RANGES.iter().position(|(net, p)| in_range(ip, *net, *p))
}

pub fn is_private(ip: Ipv4Addr) -> bool {
    range_rank(ip).is_some()
}

/// Pure ranking: best candidate by range priority, stable within a range.
pub fn pick_lan(candidates: &[Ipv4Addr]) -> Option<Ipv4Addr> {
    candidates
        .iter()
        .filter_map(|ip| range_rank(*ip).map(|r| (r, *ip)))
        .min_by_key(|(r, _)| *r)
        .map(|(_, ip)| ip)
}

/// All IPv4 addresses `hostname -I` reports (the standard short path on
/// Debian/Ubuntu, our installer's target platform).
fn candidates_from_hostname() -> Vec<Ipv4Addr> {
    std::process::Command::new("hostname")
        .arg("-I")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .split_whitespace()
                .filter_map(|t| t.parse::<Ipv4Addr>().ok())
                .collect()
        })
        .unwrap_or_default()
}

/// Default-route interface address via the classic UDP-connect trick: no
/// packet is sent, the kernel just picks the source address it would use.
fn candidate_from_route() -> Option<Ipv4Addr> {
    let sock = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    sock.connect("198.51.100.1:9").ok()?;
    match sock.local_addr().ok()? {
        std::net::SocketAddr::V4(a) => Some(*a.ip()),
        _ => None,
    }
}

/// This host's best private address, or None when nothing qualifies (in
/// which case the caller should keep its default and warn).
pub fn lan_ip() -> Option<Ipv4Addr> {
    let mut c = candidates_from_hostname();
    if let Some(ip) = candidate_from_route() {
        c.push(ip);
    }
    pick_lan(&c)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> Ipv4Addr {
        s.parse().unwrap()
    }

    #[test]
    fn private_ranges_accepted_public_and_special_rejected() {
        for good in ["10.1.2.3", "172.16.0.1", "172.31.255.254", "192.168.1.50", "100.64.0.1", "100.127.255.254"] {
            assert!(is_private(ip(good)), "{good} should be private");
        }
        for bad in [
            "8.8.8.8", "192.0.2.2", "172.32.0.1", "100.128.0.1", "99.64.0.1",
            "127.0.0.1", "169.254.10.10", "0.0.0.0",
        ] {
            assert!(!is_private(ip(bad)), "{bad} should be rejected");
        }
    }

    #[test]
    fn ranking_follows_spec_order() {
        // CGNAT beats everything, then 10/8, 172.16/12, 192.168/16
        let c = [ip("192.168.1.5"), ip("10.0.0.5"), ip("100.100.1.1"), ip("172.20.0.5")];
        assert_eq!(pick_lan(&c), Some(ip("100.100.1.1")));
        let c = [ip("192.168.1.5"), ip("172.20.0.5"), ip("10.0.0.5")];
        assert_eq!(pick_lan(&c), Some(ip("10.0.0.5")));
        let c = [ip("192.168.1.5"), ip("172.20.0.5")];
        assert_eq!(pick_lan(&c), Some(ip("172.20.0.5")));
        let c = [ip("192.168.1.5")];
        assert_eq!(pick_lan(&c), Some(ip("192.168.1.5")));
    }

    #[test]
    fn no_private_candidates_yields_none() {
        assert_eq!(pick_lan(&[ip("8.8.8.8"), ip("192.0.2.2"), ip("127.0.0.1")]), None);
        assert_eq!(pick_lan(&[]), None);
    }

    #[test]
    fn lan_ip_never_returns_non_private() {
        // Environment-dependent by nature; the invariant is the contract.
        if let Some(got) = lan_ip() {
            assert!(is_private(got), "lan_ip leaked non-private {got}");
        }
    }
}
