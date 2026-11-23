use std::net::{Ipv4Addr, Ipv6Addr};

pub const NETWORK_LIST_SUBTREE: &str = "Microsoft\\Windows NT\\CurrentVersion\\NetworkList";
pub const ROOT_DNS_IPV6_VALUE: &str = "RootDnsIpv6Addr";
pub const PROBE_SIZE: usize = 0x80;
pub const HR_FILE_NOT_FOUND: i32 = 0x8007_0002u32 as i32;
pub const HR_NT_INVALID_PARAMETER: i32 = 0xD000_000Du32 as i32;

pub fn network_list_key() -> String {
    format!("{}\\{}", console_sandbox::hive::SOFTWARE_ROOT, NETWORK_LIST_SUBTREE)
}

fn ensure_seeded() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        console_sandbox::hive::seed_software(NETWORK_LIST_SUBTREE);
    });
}

fn split_port(s: &str) -> Option<(&str, u16)> {
    match s.rsplit_once(':') {
        Some((a, p)) if !p.is_empty() && p.bytes().all(|c| c.is_ascii_digit()) => Some((a, p.parse().ok()?)),
        _ => None,
    }
}

pub fn parse_ipv4_strict(s: &str) -> Option<(Ipv4Addr, u16)> {
    if let Ok(a) = s.parse::<Ipv4Addr>() {
        return Some((a, 0));
    }
    let (a, p) = split_port(s)?;
    Some((a.parse::<Ipv4Addr>().ok()?, p))
}

pub fn parse_ipv6(s: &str) -> Option<(Ipv6Addr, u32, u16)> {
    let (body, port) = match s.strip_prefix('[') {
        Some(rest) => {
            let (inner, tail) = rest.split_once(']')?;
            let port = match tail {
                "" => 0,
                t => t.strip_prefix(':')?.parse().ok()?,
            };
            (inner, port)
        }
        None => (s, 0),
    };
    let (addr, scope) = match body.split_once('%') {
        Some((a, sc)) => (a, sc.parse().ok()?),
        None => (body, 0),
    };
    Some((addr.parse().ok()?, scope, port))
}

pub fn probe_from_text(text: &str) -> Result<[u8; PROBE_SIZE], i32> {
    let mut out = [0u8; PROBE_SIZE];
    if let Some((a, port)) = parse_ipv4_strict(text) {
        out[0..2].copy_from_slice(&2u16.to_le_bytes());
        out[2..4].copy_from_slice(&port.to_be_bytes());
        out[4..8].copy_from_slice(&a.octets());
        return Ok(out);
    }
    match parse_ipv6(text) {
        Some((a, scope, port)) => {
            out[0..2].copy_from_slice(&0x17u16.to_le_bytes());
            out[2..4].copy_from_slice(&port.to_be_bytes());
            out[8..24].copy_from_slice(&a.octets());
            out[0x18..0x1c].copy_from_slice(&scope.to_le_bytes());
            Ok(out)
        }
        None => Err(HR_NT_INVALID_PARAMETER),
    }
}

pub fn read_probe(value: &str) -> Result<[u8; PROBE_SIZE], i32> {
    ensure_seeded();
    let text = console_sandbox::registry::get_string(&network_list_key(), value).ok_or(HR_FILE_NOT_FOUND)?;
    probe_from_text(&text)
}

pub fn probe_addresses() -> Result<[[u8; PROBE_SIZE]; 2], i32> {
    Ok([read_probe("")?, read_probe(ROOT_DNS_IPV6_VALUE)?])
}

pub fn as_sort_entry(p: &[u8; PROBE_SIZE]) -> [u8; 28] {
    let mut s = [0u8; 28];
    s.copy_from_slice(&p[..28]);
    if u16::from_le_bytes([p[0], p[1]]) == 2 {
        s = [0u8; 28];
        s[0..2].copy_from_slice(&0x17u16.to_le_bytes());
        s[2..4].copy_from_slice(&p[2..4]);
        s[8 + 10] = 0xff;
        s[8 + 11] = 0xff;
        s[8 + 12..24].copy_from_slice(&p[4..8]);
    }
    s
}
