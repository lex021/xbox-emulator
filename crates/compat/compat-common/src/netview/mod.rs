use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

pub mod iphlp;
pub mod sockets;
pub mod wfp;
pub(crate) mod wfp_codec;
pub mod wfp_names;

#[cfg(test)]
mod tests;

pub const AF_UNSPEC: u32 = 0;
pub const AF_INET: u32 = 2;
pub const AF_INET6: u32 = 23;

pub const ERROR_SUCCESS: u32 = 0;
pub const ERROR_NOT_READY: u32 = 0x15;
pub const ERROR_INVALID_PARAMETER: u32 = 87;
pub const ERROR_BUFFER_OVERFLOW: u32 = 111;
pub const ERROR_INSUFFICIENT_BUFFER: u32 = 122;
pub const ERROR_NOT_FOUND: u32 = 1168;

pub const GAA_FLAG_SKIP_UNICAST: u32 = 0x1;
pub const GAA_FLAG_SKIP_ANYCAST: u32 = 0x2;
pub const GAA_FLAG_SKIP_MULTICAST: u32 = 0x4;
pub const GAA_FLAG_SKIP_DNS_SERVER: u32 = 0x8;
pub const GAA_FLAG_INCLUDE_PREFIX: u32 = 0x10;
pub const GAA_FLAG_SKIP_FRIENDLY_NAME: u32 = 0x20;

pub const IF_TYPE_ETHERNET_CSMACD: u32 = 6;
pub const IF_TYPE_SOFTWARE_LOOPBACK: u32 = 24;
pub const IF_OPER_STATUS_UP: u32 = 1;

pub const ADAPTER_SIZE: usize = 0x1c0;
pub const UNICAST_SIZE: usize = 0x40;
pub const MULTICAST_SIZE: usize = 0x20;
pub const PREFIX_SIZE: usize = 0x28;
pub const ROUTE_ROW_SIZE: usize = 0x68;

pub const NIC_IF_INDEX: u32 = 2;
pub const LOOPBACK_IF_INDEX: u32 = 1;
pub const NIC_MAC: [u8; 6] = [0x02, 0x54, 0x42, 0x00, 0x00, 0x01];
pub const NIC_IPV4: Ipv4Addr = Ipv4Addr::new(169, 254, 66, 1);
pub const NIC_IPV6: Ipv6Addr = Ipv6Addr::new(0xfe80, 0, 0, 0, 0x0054, 0x42ff, 0xfe00, 0x0001);

pub const TEREDO_STATE_OFFLINE: u32 = 0;
pub const TEREDO_STATE_QUALIFIED: u32 = 5;
pub const NSI_TEREDO_OBJECT_STATE: u32 = 1;
pub const NSI_TEREDO_OBJECT_PORT: u32 = 3;
pub const NPI_MS_TEREDO_MODULEID: [u8; 24] = [0x18, 0, 0, 0, 1, 0, 0, 0, 0x1c, 0x4a, 0x00, 0xeb, 0x1a, 0x9b, 0xd4, 0x11, 0x91, 0x23, 0x00, 0x50, 0x04, 0x77, 0x59, 0xbc];

#[derive(Clone, Debug, PartialEq)]
pub struct Unicast {
    pub addr: IpAddr,
    pub prefix_len: u8,
    pub prefix_origin: u32,
    pub suffix_origin: u32,
    pub dad_state: u32,
    pub flags: u32,
    pub valid: u32,
    pub preferred: u32,
    pub lease: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Adapter {
    pub if_index: u32,
    pub ipv6_if_index: u32,
    pub luid: u64,
    pub name: &'static str,
    pub network_guid: [u8; 16],
    pub friendly_name: &'static str,
    pub description: &'static str,
    pub if_type: u32,
    pub oper_status: u32,
    pub mtu: u32,
    pub mac: &'static [u8],
    pub flags: u32,
    pub speed: u64,
    pub ipv4_metric: u32,
    pub ipv6_metric: u32,
    pub connection_type: u32,
    pub unicast: Vec<Unicast>,
    pub multicast: Vec<IpAddr>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Route {
    pub luid: u64,
    pub if_index: u32,
    pub dest: IpAddr,
    pub prefix_len: u8,
    pub next_hop: IpAddr,
    pub metric: u32,
    pub loopback: bool,
}

pub const ROUTE_PROTOCOL_LOCAL: u32 = 2;
pub const ROUTE_ORIGIN_WELL_KNOWN: u32 = 2;
pub const INFINITE_LIFETIME: u32 = 0xffff_ffff;

const fn luid(if_type: u32, index: u32) -> u64 {
    ((if_type as u64) << 48) | ((index as u64) << 24)
}

pub const NIC_LUID: u64 = luid(IF_TYPE_ETHERNET_CSMACD, 1);
pub const LOOPBACK_LUID: u64 = luid(IF_TYPE_SOFTWARE_LOOPBACK, 0);

fn unicast(addr: IpAddr, prefix_len: u8, prefix_origin: u32, suffix_origin: u32) -> Unicast {
    Unicast { addr, prefix_len, prefix_origin, suffix_origin, dad_state: 4, flags: 0, valid: INFINITE_LIFETIME, preferred: INFINITE_LIFETIME, lease: INFINITE_LIFETIME }
}

pub fn adapters() -> Vec<Adapter> {
    vec![
        Adapter {
            if_index: NIC_IF_INDEX,
            ipv6_if_index: NIC_IF_INDEX,
            luid: NIC_LUID,
            name: "{5A6F8E2C-1B3D-4E59-9C70-546F6D426F78}",
            network_guid: [0x2c, 0x8e, 0x6f, 0x5a, 0x3d, 0x1b, 0x59, 0x4e, 0x9c, 0x70, 0x54, 0x6f, 0x6d, 0x42, 0x6f, 0x78],
            friendly_name: "Ethernet",
            description: "TomBox Emulated Console Network Adapter",
            if_type: IF_TYPE_ETHERNET_CSMACD,
            oper_status: IF_OPER_STATUS_UP,
            mtu: 1500,
            mac: &NIC_MAC,
            flags: 0x1 | 0x2 | 0x4 | 0x80 | 0x100,
            speed: 1_000_000_000,
            ipv4_metric: 25,
            ipv6_metric: 25,
            connection_type: 1,
            unicast: vec![unicast(IpAddr::V6(NIC_IPV6), 64, 2, 4), unicast(IpAddr::V4(NIC_IPV4), 16, 2, 4)],
            multicast: vec![IpAddr::V6(Ipv6Addr::new(0xff01, 0, 0, 0, 0, 0, 0, 1)), IpAddr::V6(Ipv6Addr::new(0xff02, 0, 0, 0, 0, 0, 0, 1)), IpAddr::V4(Ipv4Addr::new(224, 0, 0, 1))],
        },
        Adapter {
            if_index: LOOPBACK_IF_INDEX,
            ipv6_if_index: LOOPBACK_IF_INDEX,
            luid: LOOPBACK_LUID,
            name: "{6C6F6F70-6261-636B-2054-6F6D426F7801}",
            network_guid: [0; 16],
            friendly_name: "Loopback Pseudo-Interface 1",
            description: "Software Loopback Interface 1",
            if_type: IF_TYPE_SOFTWARE_LOOPBACK,
            oper_status: IF_OPER_STATUS_UP,
            mtu: 0xffff_ffff,
            mac: &[],
            flags: 0x80 | 0x100,
            speed: 1_073_741_824,
            ipv4_metric: 75,
            ipv6_metric: 75,
            connection_type: 1,
            unicast: vec![unicast(IpAddr::V6(Ipv6Addr::LOCALHOST), 128, 2, 2), unicast(IpAddr::V4(Ipv4Addr::LOCALHOST), 8, 2, 2)],
            multicast: vec![IpAddr::V6(Ipv6Addr::new(0xff02, 0, 0, 0, 0, 0, 0, 0xc)), IpAddr::V4(Ipv4Addr::new(239, 255, 255, 250))],
        },
    ]
}

fn route(luid: u64, if_index: u32, dest: IpAddr, prefix_len: u8, metric: u32, loopback: bool) -> Route {
    let next_hop = match dest {
        IpAddr::V4(_) => IpAddr::V4(Ipv4Addr::UNSPECIFIED),
        IpAddr::V6(_) => IpAddr::V6(Ipv6Addr::UNSPECIFIED),
    };
    Route { luid, if_index, dest, prefix_len, next_hop, metric, loopback }
}

pub fn routes() -> Vec<Route> {
    let v4 = |a, b, c, d| IpAddr::V4(Ipv4Addr::new(a, b, c, d));
    let (n, ni, l, li) = (NIC_LUID, NIC_IF_INDEX, LOOPBACK_LUID, LOOPBACK_IF_INDEX);
    vec![
        route(l, li, v4(127, 0, 0, 0), 8, 256, true),
        route(l, li, v4(127, 0, 0, 1), 32, 256, true),
        route(l, li, v4(127, 255, 255, 255), 32, 256, true),
        route(n, ni, v4(169, 254, 0, 0), 16, 256, false),
        route(n, ni, IpAddr::V4(NIC_IPV4), 32, 256, true),
        route(n, ni, v4(169, 254, 255, 255), 32, 256, false),
        route(l, li, v4(224, 0, 0, 0), 4, 256, true),
        route(n, ni, v4(224, 0, 0, 0), 4, 256, false),
        route(l, li, v4(255, 255, 255, 255), 32, 256, true),
        route(n, ni, v4(255, 255, 255, 255), 32, 256, false),
        route(l, li, IpAddr::V6(Ipv6Addr::LOCALHOST), 128, 256, true),
        route(n, ni, IpAddr::V6(Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 0)), 64, 256, false),
        route(n, ni, IpAddr::V6(NIC_IPV6), 128, 256, true),
        route(l, li, IpAddr::V6(Ipv6Addr::new(0xff00, 0, 0, 0, 0, 0, 0, 0)), 8, 256, true),
        route(n, ni, IpAddr::V6(Ipv6Addr::new(0xff00, 0, 0, 0, 0, 0, 0, 0)), 8, 256, false),
    ]
}

pub fn has_route_to(ip: &IpAddr) -> bool {
    routes().iter().any(|r| prefix_matches(&r.dest, r.prefix_len, ip))
}

fn prefix_matches(net: &IpAddr, len: u8, ip: &IpAddr) -> bool {
    let (a, b, bits): (Vec<u8>, Vec<u8>, usize) = match (net, ip) {
        (IpAddr::V4(n), IpAddr::V4(i)) => (n.octets().to_vec(), i.octets().to_vec(), 32),
        (IpAddr::V6(n), IpAddr::V6(i)) => (n.octets().to_vec(), i.octets().to_vec(), 128),
        _ => return false,
    };
    let len = (len as usize).min(bits);
    (0..len).all(|k| (a[k / 8] >> (7 - k % 8)) & 1 == (b[k / 8] >> (7 - k % 8)) & 1)
}

fn family_matches(family: u32, ip: &IpAddr) -> bool {
    match family {
        AF_INET => ip.is_ipv4(),
        AF_INET6 => ip.is_ipv6(),
        _ => true,
    }
}

pub fn sockaddr_bytes(ip: &IpAddr, port: u16, scope: u32) -> Vec<u8> {
    match ip {
        IpAddr::V4(a) => {
            let mut b = vec![0u8; 16];
            b[0..2].copy_from_slice(&(AF_INET as u16).to_le_bytes());
            b[2..4].copy_from_slice(&port.to_be_bytes());
            b[4..8].copy_from_slice(&a.octets());
            b
        }
        IpAddr::V6(a) => {
            let mut b = vec![0u8; 28];
            b[0..2].copy_from_slice(&(AF_INET6 as u16).to_le_bytes());
            b[2..4].copy_from_slice(&port.to_be_bytes());
            b[8..24].copy_from_slice(&a.octets());
            b[24..28].copy_from_slice(&scope.to_le_bytes());
            b
        }
    }
}

pub fn scope_of(ip: &IpAddr, if_index: u32) -> u32 {
    match ip {
        IpAddr::V6(a) if (a.segments()[0] & 0xffc0) == 0xfe80 || (a.segments()[0] & 0xff0f) == 0xff02 => if_index,
        _ => 0,
    }
}

fn wide(s: &str) -> Vec<u8> {
    let mut v: Vec<u8> = s.encode_utf16().flat_map(|c| c.to_le_bytes()).collect();
    v.extend_from_slice(&[0, 0]);
    v
}

fn align8(n: usize) -> usize {
    (n + 7) & !7
}

struct Writer {
    base: usize,
    buf: Vec<u8>,
}

impl Writer {
    fn alloc(&mut self, n: usize) -> usize {
        let o = align8(self.buf.len());
        self.buf.resize(o + n, 0);
        o
    }
    fn put(&mut self, o: usize, b: &[u8]) {
        self.buf[o..o + b.len()].copy_from_slice(b);
    }
    fn u32(&mut self, o: usize, v: u32) {
        self.put(o, &v.to_le_bytes());
    }
    fn u64(&mut self, o: usize, v: u64) {
        self.put(o, &v.to_le_bytes());
    }
    fn ptr(&mut self, o: usize, target: usize) {
        let v = (self.base + target) as u64;
        self.u64(o, v);
    }
    fn blob(&mut self, b: &[u8]) -> usize {
        let o = self.alloc(b.len());
        self.put(o, b);
        o
    }
    fn socket_address(&mut self, at: usize, ip: &IpAddr, scope: u32) {
        let sa = sockaddr_bytes(ip, 0, scope);
        let o = self.blob(&sa);
        self.ptr(at, o);
        self.u32(at + 8, sa.len() as u32);
    }
}

fn build_adapters(base: usize, family: u32, flags: u32) -> Vec<u8> {
    let list = adapters();
    let mut w = Writer { base, buf: Vec::new() };
    let heads: Vec<usize> = list.iter().map(|_| w.alloc(ADAPTER_SIZE)).collect();
    for (i, a) in list.iter().enumerate() {
        let h = heads[i];
        w.u32(h, ADAPTER_SIZE as u32);
        w.u32(h + 4, a.if_index);
        if i + 1 < heads.len() {
            w.ptr(h + 8, heads[i + 1]);
        }
        let mut name = a.name.as_bytes().to_vec();
        name.push(0);
        let o = w.blob(&name);
        w.ptr(h + 0x10, o);
        if flags & GAA_FLAG_SKIP_UNICAST == 0 {
            let us: Vec<&Unicast> = a.unicast.iter().filter(|u| family_matches(family, &u.addr)).collect();
            let entries: Vec<usize> = us.iter().map(|_| w.alloc(UNICAST_SIZE)).collect();
            for (k, u) in us.iter().enumerate() {
                let e = entries[k];
                w.u32(e, UNICAST_SIZE as u32);
                w.u32(e + 4, u.flags);
                if k + 1 < entries.len() {
                    w.ptr(e + 8, entries[k + 1]);
                }
                w.socket_address(e + 0x10, &u.addr, scope_of(&u.addr, a.if_index));
                w.u32(e + 0x20, u.prefix_origin);
                w.u32(e + 0x24, u.suffix_origin);
                w.u32(e + 0x28, u.dad_state);
                w.u32(e + 0x2c, u.valid);
                w.u32(e + 0x30, u.preferred);
                w.u32(e + 0x34, u.lease);
                w.buf[e + 0x38] = u.prefix_len;
            }
            if let Some(f) = entries.first() {
                w.ptr(h + 0x18, *f);
            }
        }
        if flags & GAA_FLAG_SKIP_MULTICAST == 0 {
            let ms: Vec<&IpAddr> = a.multicast.iter().filter(|m| family_matches(family, m)).collect();
            let entries: Vec<usize> = ms.iter().map(|_| w.alloc(MULTICAST_SIZE)).collect();
            for (k, m) in ms.iter().enumerate() {
                let e = entries[k];
                w.u32(e, MULTICAST_SIZE as u32);
                if k + 1 < entries.len() {
                    w.ptr(e + 8, entries[k + 1]);
                }
                w.socket_address(e + 0x10, m, scope_of(m, a.if_index));
            }
            if let Some(f) = entries.first() {
                w.ptr(h + 0x28, *f);
            }
        }
        let o = w.blob(&wide(""));
        w.ptr(h + 0x38, o);
        let o = w.blob(&wide(a.description));
        w.ptr(h + 0x40, o);
        let o = w.blob(&wide(if flags & GAA_FLAG_SKIP_FRIENDLY_NAME == 0 { a.friendly_name } else { "" }));
        w.ptr(h + 0x48, o);
        w.put(h + 0x50, a.mac);
        w.u32(h + 0x58, a.mac.len() as u32);
        w.u32(h + 0x5c, a.flags);
        w.u32(h + 0x60, a.mtu);
        w.u32(h + 0x64, a.if_type);
        w.u32(h + 0x68, a.oper_status);
        w.u32(h + 0x6c, a.ipv6_if_index);
        for z in 0..16usize {
            let v = match z {
                1..=3 => a.if_index,
                4..=13 => 1,
                _ => 0,
            };
            w.u32(h + 0x70 + z * 4, v);
        }
        if flags & GAA_FLAG_INCLUDE_PREFIX != 0 {
            let ps: Vec<(IpAddr, u8)> = a.unicast.iter().filter(|u| family_matches(family, &u.addr)).map(|u| (network_of(&u.addr, u.prefix_len), u.prefix_len)).collect();
            let entries: Vec<usize> = ps.iter().map(|_| w.alloc(PREFIX_SIZE)).collect();
            for (k, (p, len)) in ps.iter().enumerate() {
                let e = entries[k];
                w.u32(e, PREFIX_SIZE as u32);
                if k + 1 < entries.len() {
                    w.ptr(e + 8, entries[k + 1]);
                }
                w.socket_address(e + 0x10, p, 0);
                w.u32(e + 0x20, *len as u32);
            }
            if let Some(f) = entries.first() {
                w.ptr(h + 0xb0, *f);
            }
        }
        w.u64(h + 0xb8, a.speed);
        w.u64(h + 0xc0, a.speed);
        w.u32(h + 0xd8, a.ipv4_metric);
        w.u32(h + 0xdc, a.ipv6_metric);
        w.u64(h + 0xe0, a.luid);
        w.u32(h + 0xf8, 1);
        w.put(h + 0xfc, &a.network_guid);
        w.u32(h + 0x10c, a.connection_type);
    }
    w.buf
}

fn network_of(ip: &IpAddr, len: u8) -> IpAddr {
    let mask = |b: &mut [u8], len: usize| {
        for (k, x) in b.iter_mut().enumerate() {
            let keep = len.saturating_sub(k * 8).min(8);
            *x &= if keep == 0 { 0 } else { 0xffu8 << (8 - keep) };
        }
    };
    match ip {
        IpAddr::V4(a) => {
            let mut o = a.octets();
            mask(&mut o, len as usize);
            IpAddr::V4(Ipv4Addr::from(o))
        }
        IpAddr::V6(a) => {
            let mut o = a.octets();
            mask(&mut o, len as usize);
            IpAddr::V6(Ipv6Addr::from(o))
        }
    }
}

pub fn adapters_addresses_size(family: u32, flags: u32) -> usize {
    build_adapters(0, family, flags).len()
}

pub unsafe fn get_adapters_addresses(family: u32, flags: u32, buf: *mut u8, size: *mut u32) -> u32 {
    if size.is_null() || !matches!(family, AF_UNSPEC | AF_INET | AF_INET6) {
        return ERROR_INVALID_PARAMETER;
    }
    let need = adapters_addresses_size(family, flags);
    if buf.is_null() || (*size as usize) < need {
        *size = need as u32;
        return ERROR_BUFFER_OVERFLOW;
    }
    let bytes = build_adapters(buf as usize, family, flags);
    core::ptr::copy_nonoverlapping(bytes.as_ptr(), buf, bytes.len());
    ERROR_SUCCESS
}

fn sockaddr_inet(ip: &IpAddr, scope: u32) -> [u8; 28] {
    let mut out = [0u8; 28];
    let b = sockaddr_bytes(ip, 0, scope);
    out[..b.len()].copy_from_slice(&b);
    out
}

pub fn ip_forward_table2(family: u32) -> Result<Vec<u8>, u32> {
    if !matches!(family, AF_UNSPEC | AF_INET | AF_INET6) {
        return Err(ERROR_INVALID_PARAMETER);
    }
    let rows: Vec<Route> = routes().into_iter().filter(|r| family_matches(family, &r.dest)).collect();
    let mut t = vec![0u8; 8 + rows.len() * ROUTE_ROW_SIZE];
    t[0..4].copy_from_slice(&(rows.len() as u32).to_le_bytes());
    for (i, r) in rows.iter().enumerate() {
        let o = 8 + i * ROUTE_ROW_SIZE;
        let row = &mut t[o..o + ROUTE_ROW_SIZE];
        row[0..8].copy_from_slice(&r.luid.to_le_bytes());
        row[8..0xc].copy_from_slice(&r.if_index.to_le_bytes());
        row[0xc..0x28].copy_from_slice(&sockaddr_inet(&r.dest, scope_of(&r.dest, r.if_index)));
        row[0x28] = r.prefix_len;
        row[0x2c..0x48].copy_from_slice(&sockaddr_inet(&r.next_hop, 0));
        row[0x4c..0x50].copy_from_slice(&INFINITE_LIFETIME.to_le_bytes());
        row[0x50..0x54].copy_from_slice(&INFINITE_LIFETIME.to_le_bytes());
        row[0x54..0x58].copy_from_slice(&r.metric.to_le_bytes());
        row[0x58..0x5c].copy_from_slice(&ROUTE_PROTOCOL_LOCAL.to_le_bytes());
        row[0x5c] = r.loopback as u8;
        row[0x5d] = 1;
        row[0x5e] = 0;
        row[0x5f] = 1;
        row[0x64..0x68].copy_from_slice(&ROUTE_ORIGIN_WELL_KNOWN.to_le_bytes());
    }
    Ok(t)
}

pub fn teredo_state() -> u32 {
    TEREDO_STATE_OFFLINE
}

pub fn teredo_rw_parameters() -> [u8; 12] {
    let mut b = [0u8; 12];
    b[0..4].copy_from_slice(&teredo_state().to_le_bytes());
    b
}

pub fn teredo_port() -> Option<u16> {
    None
}

#[allow(clippy::too_many_arguments)]
pub unsafe fn nsi_get_all_parameters(_store: u32, module: *const u8, object: u32, key: *const u8, key_len: u32, rw: *mut u8, rw_len: u32, rod: *mut u8, rod_len: u32, ros: *mut u8, ros_len: u32) -> u32 {
    if module.is_null() {
        return ERROR_INVALID_PARAMETER;
    }
    let id = core::slice::from_raw_parts(module, NPI_MS_TEREDO_MODULEID.len());
    if id == iphlp::NPI_MS_IPV4_MODULEID || id == iphlp::NPI_MS_IPV6_MODULEID {
        return iphlp::nsi_get_interface_rw(id, object, key, key_len, rw, rw_len, rod, rod_len, ros, ros_len);
    }
    if id != NPI_MS_TEREDO_MODULEID {
        return ERROR_NOT_FOUND;
    }
    if !key.is_null() || key_len != 0 || !rod.is_null() || rod_len != 0 || !ros.is_null() || ros_len != 0 {
        return ERROR_INVALID_PARAMETER;
    }
    match object {
        NSI_TEREDO_OBJECT_STATE => {
            if rw.is_null() || rw_len != 12 {
                return ERROR_INVALID_PARAMETER;
            }
            let p = teredo_rw_parameters();
            core::ptr::copy_nonoverlapping(p.as_ptr(), rw, p.len());
            ERROR_SUCCESS
        }
        NSI_TEREDO_OBJECT_PORT => {
            if rw.is_null() || rw_len != 2 {
                return ERROR_INVALID_PARAMETER;
            }
            match teredo_port() {
                Some(p) => {
                    core::ptr::copy_nonoverlapping(p.to_le_bytes().as_ptr(), rw, 2);
                    ERROR_SUCCESS
                }
                None => ERROR_NOT_READY,
            }
        }
        _ => ERROR_NOT_FOUND,
    }
}
