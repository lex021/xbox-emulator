use crate::sys::*;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

pub const WSAEHOSTUNREACH_HR: HRESULT = 0x8007_2751u32 as i32;
pub const WSAHOST_NOT_FOUND_HR: HRESULT = 0x8007_2af9u32 as i32;

pub fn parse_literal(text: &str) -> Option<IpAddr> {
    let t = text.trim();
    let t = t.strip_prefix('[').and_then(|s| s.strip_suffix(']')).unwrap_or(t);
    let t = t.split('%').next().unwrap_or(t);
    t.parse::<IpAddr>().ok()
}

pub const CONSOLE_HOST_V4: Ipv4Addr = Ipv4Addr::new(169, 254, 66, 1);
pub const CONSOLE_HOST_V6: Ipv6Addr = Ipv6Addr::new(0xfe80, 0, 0, 0, 0x0054, 0x42ff, 0xfe00, 0x0001);

pub fn ip_is_loopback(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => v.is_loopback(),
        IpAddr::V6(v) => v.is_loopback() || v.to_ipv4_mapped().map(|m| m.is_loopback()).unwrap_or(false),
    }
}

pub fn ip_is_console_host(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => *v == CONSOLE_HOST_V4,
        IpAddr::V6(v) => *v == CONSOLE_HOST_V6 || v.to_ipv4_mapped() == Some(CONSOLE_HOST_V4),
    }
}

pub fn ip_is_local(ip: &IpAddr) -> bool {
    ip_is_loopback(ip) || ip_is_console_host(ip)
}

pub unsafe fn localize_sockaddr(addr: *mut u8, len: usize) -> bool {
    let Some(ip) = sockaddr_ip(addr, len) else {
        return false;
    };
    if !ip_is_console_host(&ip) {
        return false;
    }
    match ip {
        IpAddr::V4(_) => core::ptr::copy_nonoverlapping([127u8, 0, 0, 1].as_ptr(), addr.add(4), 4),
        IpAddr::V6(v) if v.to_ipv4_mapped().is_some() => core::ptr::copy_nonoverlapping([127u8, 0, 0, 1].as_ptr(), addr.add(8 + 12), 4),
        IpAddr::V6(_) => {
            let mut b = [0u8; 16];
            b[15] = 1;
            core::ptr::copy_nonoverlapping(b.as_ptr(), addr.add(8), 16);
            if len >= 0x1c {
                core::ptr::write_unaligned(addr.add(24) as *mut u32, 0);
            }
        }
    }
    true
}

pub fn name_is_localhost(text: &str) -> bool {
    let t = text.trim().trim_end_matches('.');
    t.eq_ignore_ascii_case("localhost") || t.to_ascii_lowercase().ends_with(".localhost")
}

pub enum HostClass {
    Default,
    Local,
    RemoteLiteral,
    RemoteName,
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
        Some(ip) if ip_is_local(&ip) => HostClass::Local,
        Some(_) => HostClass::RemoteLiteral,
        None => HostClass::RemoteName,
    }
}

pub unsafe fn classify_wide(p: *const u16) -> HostClass {
    if p.is_null() {
        return HostClass::Default;
    }
    classify_host(&from_wide(p))
}

pub unsafe fn sockaddr_ip(addr: *const u8, len: usize) -> Option<IpAddr> {
    if addr.is_null() || len < 8 {
        return None;
    }
    let fam = core::ptr::read_unaligned(addr as *const u16);
    if fam == 2 && len >= 8 {
        let b = core::slice::from_raw_parts(addr.add(4), 4);
        Some(IpAddr::V4(Ipv4Addr::new(b[0], b[1], b[2], b[3])))
    } else if fam == 0x17 && len >= 0x18 {
        let mut b = [0u8; 16];
        core::ptr::copy_nonoverlapping(addr.add(8), b.as_mut_ptr(), 16);
        Some(IpAddr::V6(Ipv6Addr::from(b)))
    } else {
        None
    }
}

pub unsafe fn sockaddr_is_loopback(addr: *const u8, len: usize) -> bool {
    sockaddr_ip(addr, len).map(|ip| ip_is_loopback(&ip)).unwrap_or(false)
}

pub unsafe fn sockaddr_is_local(addr: *const u8, len: usize) -> bool {
    sockaddr_ip(addr, len).map(|ip| ip_is_local(&ip)).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_classes() {
        assert!(matches!(classify_host(""), HostClass::Default));
        assert!(matches!(classify_host("localhost"), HostClass::Local));
        assert!(matches!(classify_host("LocalHost."), HostClass::Local));
        assert!(matches!(classify_host("127.0.0.1"), HostClass::Local));
        assert!(matches!(classify_host("127.8.9.10"), HostClass::Local));
        assert!(matches!(classify_host("::1"), HostClass::Local));
        assert!(matches!(classify_host("[::1]"), HostClass::Local));
        assert!(matches!(classify_host("::ffff:127.0.0.1"), HostClass::Local));
        assert!(matches!(classify_host("192.168.1.20"), HostClass::RemoteLiteral));
        assert!(matches!(classify_host("10.0.0.1"), HostClass::RemoteLiteral));
        assert!(matches!(classify_host("fe80::1%3"), HostClass::RemoteLiteral));
        assert!(matches!(classify_host("169.254.66.1"), HostClass::Local));
        assert!(matches!(classify_host("169.254.66.2"), HostClass::RemoteLiteral));
        assert!(matches!(classify_host("fe80::54:42ff:fe00:1"), HostClass::Local));
        assert!(matches!(classify_host("[fe80::54:42ff:fe00:1%2]"), HostClass::Local));
        assert!(matches!(classify_host("::ffff:169.254.66.1"), HostClass::Local));
        assert!(matches!(classify_host("myxbox"), HostClass::RemoteName));
        assert!(matches!(classify_host("example.com"), HostClass::RemoteName));
    }

    #[test]
    fn sockaddr_loopback() {
        let mut v4 = [0u8; 16];
        v4[0] = 2;
        v4[4..8].copy_from_slice(&[127, 0, 0, 1]);
        unsafe { assert!(sockaddr_is_loopback(v4.as_ptr(), 16)) };
        v4[4..8].copy_from_slice(&[192, 168, 0, 2]);
        unsafe { assert!(!sockaddr_is_loopback(v4.as_ptr(), 16)) };
        let mut v6 = [0u8; 0x1c];
        v6[0] = 0x17;
        v6[8 + 10] = 0xff;
        v6[8 + 11] = 0xff;
        v6[8 + 12..8 + 16].copy_from_slice(&[127, 0, 0, 1]);
        unsafe { assert!(sockaddr_is_loopback(v6.as_ptr(), 0x1c)) };
        v6[8 + 12..8 + 16].copy_from_slice(&[8, 8, 8, 8]);
        unsafe { assert!(!sockaddr_is_loopback(v6.as_ptr(), 0x1c)) };
        let mut l6 = [0u8; 0x1c];
        l6[0] = 0x17;
        l6[8 + 15] = 1;
        unsafe { assert!(sockaddr_is_loopback(l6.as_ptr(), 0x1c)) };
    }

    #[test]
    fn console_host_addresses_are_local_and_map_to_loopback() {
        let mut v4 = [0u8; 16];
        v4[0] = 2;
        v4[2..4].copy_from_slice(&4550u16.to_be_bytes());
        v4[4..8].copy_from_slice(&[169, 254, 66, 1]);
        unsafe {
            assert!(sockaddr_is_local(v4.as_ptr(), 16));
            assert!(!sockaddr_is_loopback(v4.as_ptr(), 16));
            assert!(localize_sockaddr(v4.as_mut_ptr(), 16));
            assert_eq!(&v4[4..8], &[127, 0, 0, 1]);
            assert_eq!(&v4[2..4], &4550u16.to_be_bytes());
            assert!(!localize_sockaddr(v4.as_mut_ptr(), 16));
        }
        let mut m6 = [0u8; 0x1c];
        m6[0] = 0x17;
        m6[8 + 10] = 0xff;
        m6[8 + 11] = 0xff;
        m6[8 + 12..8 + 16].copy_from_slice(&[169, 254, 66, 1]);
        unsafe {
            assert!(localize_sockaddr(m6.as_mut_ptr(), 0x1c));
            assert!(sockaddr_is_loopback(m6.as_ptr(), 0x1c));
        }
        let mut n6 = [0u8; 0x1c];
        n6[0] = 0x17;
        n6[8..24].copy_from_slice(&CONSOLE_HOST_V6.octets());
        n6[24] = 2;
        unsafe {
            assert!(localize_sockaddr(n6.as_mut_ptr(), 0x1c));
            assert!(sockaddr_is_loopback(n6.as_ptr(), 0x1c));
            assert_eq!(&n6[24..28], &[0, 0, 0, 0]);
        }
    }

    #[test]
    fn console_host_addresses_match_the_netview_adapter() {
        use compat_common::netview;
        assert_eq!(CONSOLE_HOST_V4, netview::NIC_IPV4);
        assert_eq!(CONSOLE_HOST_V6, netview::NIC_IPV6);
        let nic = netview::adapters().into_iter().find(|a| a.if_index == netview::NIC_IF_INDEX).unwrap();
        let addrs: Vec<IpAddr> = nic.unicast.iter().map(|u| u.addr).collect();
        assert!(addrs.contains(&IpAddr::V4(CONSOLE_HOST_V4)));
        assert!(addrs.contains(&IpAddr::V6(CONSOLE_HOST_V6)));
        assert!(addrs.iter().all(ip_is_local));
    }
}
