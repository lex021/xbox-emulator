use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

pub const WSAEHOSTUNREACH: u32 = 10065;
pub const WSAHOST_NOT_FOUND: u32 = 11001;
pub const IP_DEST_HOST_UNREACHABLE: u32 = 11003;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostClass {
    Default,
    Local,
    RemoteLiteral,
    RemoteName,
}

pub fn parse_literal(text: &str) -> Option<IpAddr> {
    let t = text.trim();
    let t = t.strip_prefix('[').and_then(|s| s.strip_suffix(']')).unwrap_or(t);
    let t = t.split('%').next().unwrap_or(t);
    t.parse::<IpAddr>().ok()
}

pub fn ip_is_loopback(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => v.is_loopback(),
        IpAddr::V6(v) => v.is_loopback() || v.to_ipv4_mapped().map(|m| m.is_loopback()).unwrap_or(false),
    }
}

pub fn name_is_localhost(text: &str) -> bool {
    let t = text.trim().trim_end_matches('.');
    t.eq_ignore_ascii_case("localhost") || t.to_ascii_lowercase().ends_with(".localhost")
}

pub fn classify_host(text: &str) -> HostClass {
    let t = text.trim();
    if t.is_empty() {
        return HostClass::Default;
    }
    if name_is_localhost(t) {
        return HostClass::Local;
    }
    match parse_literal(t) {
        Some(ip) if ip_is_loopback(&ip) => HostClass::Local,
        Some(_) => HostClass::RemoteLiteral,
        None => HostClass::RemoteName,
    }
}

pub fn sockaddr_ip(sa: &[u8]) -> Option<IpAddr> {
    if sa.len() < 8 {
        return None;
    }
    let fam = u16::from_le_bytes([sa[0], sa[1]]);
    if fam == 2 {
        Some(IpAddr::V4(Ipv4Addr::new(sa[4], sa[5], sa[6], sa[7])))
    } else if fam == 0x17 && sa.len() >= 0x18 {
        let mut b = [0u8; 16];
        b.copy_from_slice(&sa[8..24]);
        Some(IpAddr::V6(Ipv6Addr::from(b)))
    } else {
        None
    }
}

pub fn sockaddr_is_loopback(sa: &[u8]) -> bool {
    sockaddr_ip(sa).map(|ip| ip_is_loopback(&ip)).unwrap_or(false)
}
