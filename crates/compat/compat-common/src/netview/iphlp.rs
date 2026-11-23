use super::*;
use core::ffi::c_void;
use std::sync::Mutex;

pub const IF_ROW2_SIZE: usize = 0x548;
pub const IF_ROW_SIZE: usize = 0x360;
pub const IF_TABLE2_HEADER: usize = 8;
pub const ERROR_FILE_NOT_FOUND: u32 = 2;
pub const ERROR_INVALID_HANDLE: u32 = 6;
pub const ERROR_NOT_ENOUGH_MEMORY: u32 = 8;
pub const ERROR_NETWORK_UNREACHABLE: u32 = 1231;
pub const MIB_IF_ENTRY_NORMAL: u32 = 0;
pub const MIB_IF_ENTRY_NORMAL_WITHOUT_STATISTICS: u32 = 2;
pub const MIB_IF_OPER_STATUS_OPERATIONAL: u32 = 5;
pub const SIO_ADDRESS_LIST_SORT: u32 = 0xC800_0019;
pub const NPI_MS_IPV4_MODULEID: [u8; 24] = [0x18, 0, 0, 0, 1, 0, 0, 0, 0x00, 0x4a, 0x00, 0xeb, 0x1a, 0x9b, 0xd4, 0x11, 0x91, 0x23, 0x00, 0x50, 0x04, 0x77, 0x59, 0xbc];
pub const NPI_MS_IPV6_MODULEID: [u8; 24] = [0x18, 0, 0, 0, 1, 0, 0, 0, 0x01, 0x4a, 0x00, 0xeb, 0x1a, 0x9b, 0xd4, 0x11, 0x91, 0x23, 0x00, 0x50, 0x04, 0x77, 0x59, 0xbc];
pub const NSI_STORE_ACTIVE: u32 = 1;
pub const NSI_SET_DEFAULT: u32 = 0;
pub const NL_INTERFACE_OBJECT: u32 = 7;
pub const NL_INTERFACE_RW_SIZE: usize = 0xf0;

#[link(name = "kernel32")]
extern "system" {
    fn GetProcessHeap() -> *mut c_void;
    fn HeapAlloc(heap: *mut c_void, flags: u32, size: usize) -> *mut c_void;
    fn HeapFree(heap: *mut c_void, flags: u32, mem: *mut c_void) -> i32;
}

fn hex_nibble(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

pub fn parse_guid(text: &str) -> Option<[u8; 16]> {
    let t = text.trim().trim_start_matches('{').trim_end_matches('}');
    let parts: Vec<&str> = t.split('-').collect();
    if parts.len() != 5 || parts.iter().map(|p| p.len()).collect::<Vec<_>>() != [8, 4, 4, 4, 12] {
        return None;
    }
    let hex: Vec<u8> = parts.concat().bytes().collect();
    let mut raw = [0u8; 16];
    for (i, r) in raw.iter_mut().enumerate() {
        *r = (hex_nibble(hex[2 * i])? << 4) | hex_nibble(hex[2 * i + 1])?;
    }
    let mut g = raw;
    g[0..4].copy_from_slice(&[raw[3], raw[2], raw[1], raw[0]]);
    g[4..6].copy_from_slice(&[raw[5], raw[4]]);
    g[6..8].copy_from_slice(&[raw[7], raw[6]]);
    Some(g)
}

pub fn interface_guid(a: &Adapter) -> [u8; 16] {
    parse_guid(a.name).unwrap_or([0; 16])
}

pub fn find_adapter(luid: u64, if_index: u32) -> Option<Adapter> {
    adapters().into_iter().find(|a| if luid != 0 { a.luid == luid } else { a.if_index == if_index })
}

fn put_wide(dst: &mut [u8], s: &str) {
    for (i, c) in s.encode_utf16().take(dst.len() / 2 - 1).enumerate() {
        dst[2 * i..2 * i + 2].copy_from_slice(&c.to_le_bytes());
    }
}

fn put32(b: &mut [u8], o: usize, v: u32) {
    b[o..o + 4].copy_from_slice(&v.to_le_bytes());
}

fn put64(b: &mut [u8], o: usize, v: u64) {
    b[o..o + 8].copy_from_slice(&v.to_le_bytes());
}

pub struct IfMedia {
    pub media_type: u32,
    pub physical_medium: u32,
    pub access_type: u32,
    pub status_flags: u8,
}

pub fn if_media(a: &Adapter) -> IfMedia {
    if a.if_type == IF_TYPE_SOFTWARE_LOOPBACK {
        IfMedia { media_type: 0, physical_medium: 0, access_type: 1, status_flags: 0 }
    } else {
        IfMedia { media_type: 0, physical_medium: 14, access_type: 2, status_flags: 0x1 | 0x4 }
    }
}

pub fn if_row2(a: &Adapter) -> Vec<u8> {
    let mut r = vec![0u8; IF_ROW2_SIZE];
    let m = if_media(a);
    put64(&mut r, 0, a.luid);
    put32(&mut r, 8, a.if_index);
    r[0xc..0x1c].copy_from_slice(&interface_guid(a));
    put_wide(&mut r[0x1c..0x21e], a.friendly_name);
    put_wide(&mut r[0x21e..0x420], a.description);
    put32(&mut r, 0x420, a.mac.len() as u32);
    r[0x424..0x424 + a.mac.len()].copy_from_slice(a.mac);
    r[0x444..0x444 + a.mac.len()].copy_from_slice(a.mac);
    put32(&mut r, 0x464, a.mtu);
    put32(&mut r, 0x468, a.if_type);
    put32(&mut r, 0x46c, 0);
    put32(&mut r, 0x470, m.media_type);
    put32(&mut r, 0x474, m.physical_medium);
    put32(&mut r, 0x478, m.access_type);
    put32(&mut r, 0x47c, 0);
    r[0x480] = m.status_flags;
    put32(&mut r, 0x484, a.oper_status);
    put32(&mut r, 0x488, 1);
    put32(&mut r, 0x48c, 1);
    r[0x490..0x4a0].copy_from_slice(&a.network_guid);
    put32(&mut r, 0x4a0, a.connection_type);
    put64(&mut r, 0x4a8, a.speed);
    put64(&mut r, 0x4b0, a.speed);
    r
}

pub fn if_table2() -> Vec<u8> {
    let rows = adapters();
    let mut t = vec![0u8; IF_TABLE2_HEADER + rows.len() * IF_ROW2_SIZE];
    put32(&mut t, 0, rows.len() as u32);
    for (i, a) in rows.iter().enumerate() {
        let o = IF_TABLE2_HEADER + i * IF_ROW2_SIZE;
        t[o..o + IF_ROW2_SIZE].copy_from_slice(&if_row2(a));
    }
    t
}

pub fn if_row(a: &Adapter) -> Vec<u8> {
    let mut r = vec![0u8; IF_ROW_SIZE];
    let g = interface_guid(a);
    let name = format!("\\DEVICE\\TCPIP_{{{:08X}-{:04X}-{:04X}-{:02X}{:02X}-{}}}", u32::from_le_bytes([g[0], g[1], g[2], g[3]]), u16::from_le_bytes([g[4], g[5]]), u16::from_le_bytes([g[6], g[7]]), g[8], g[9], g[10..16].iter().map(|b| format!("{:02X}", b)).collect::<String>());
    put_wide(&mut r[0..0x200], &name);
    put32(&mut r, 0x200, a.if_index);
    put32(&mut r, 0x204, a.if_type);
    put32(&mut r, 0x208, a.mtu);
    put32(&mut r, 0x20c, a.speed.min(u32::MAX as u64) as u32);
    let n = a.mac.len().min(8);
    put32(&mut r, 0x210, n as u32);
    r[0x214..0x214 + n].copy_from_slice(&a.mac[..n]);
    put32(&mut r, 0x21c, 1);
    put32(&mut r, 0x220, if a.oper_status == IF_OPER_STATUS_UP { MIB_IF_OPER_STATUS_OPERATIONAL } else { 0 });
    let d = a.description.as_bytes();
    let dn = d.len().min(255);
    put32(&mut r, 0x258, dn as u32 + 1);
    r[0x25c..0x25c + dn].copy_from_slice(&d[..dn]);
    r
}

pub unsafe fn get_if_entry2_ex(level: u32, row: *mut u8) -> u32 {
    if row.is_null() || !matches!(level, MIB_IF_ENTRY_NORMAL | MIB_IF_ENTRY_NORMAL_WITHOUT_STATISTICS) {
        return ERROR_INVALID_PARAMETER;
    }
    let luid = core::ptr::read_unaligned(row as *const u64);
    let index = core::ptr::read_unaligned(row.add(8) as *const u32);
    if luid == 0 && index == 0 {
        return ERROR_INVALID_PARAMETER;
    }
    match find_adapter(luid, index) {
        Some(a) => {
            let r = if_row2(&a);
            core::ptr::copy_nonoverlapping(r.as_ptr(), row, IF_ROW2_SIZE);
            ERROR_SUCCESS
        }
        None => ERROR_FILE_NOT_FOUND,
    }
}

pub unsafe fn get_if_entry2(row: *mut u8) -> u32 {
    get_if_entry2_ex(MIB_IF_ENTRY_NORMAL, row)
}

pub unsafe fn get_if_entry(row: *mut u8) -> u32 {
    if row.is_null() {
        return ERROR_INVALID_PARAMETER;
    }
    let index = core::ptr::read_unaligned(row.add(0x200) as *const u32);
    match adapters().into_iter().find(|a| a.if_index == index) {
        Some(a) => {
            let r = if_row(&a);
            core::ptr::copy_nonoverlapping(r.as_ptr(), row, IF_ROW_SIZE);
            ERROR_SUCCESS
        }
        None => ERROR_INVALID_PARAMETER,
    }
}

pub unsafe fn get_if_table2(table: *mut *mut u8) -> u32 {
    if table.is_null() {
        return ERROR_INVALID_PARAMETER;
    }
    let t = if_table2();
    let p = HeapAlloc(GetProcessHeap(), 0, t.len()) as *mut u8;
    if p.is_null() {
        *table = core::ptr::null_mut();
        return ERROR_NOT_ENOUGH_MEMORY;
    }
    core::ptr::copy_nonoverlapping(t.as_ptr(), p, t.len());
    *table = p;
    ERROR_SUCCESS
}

pub unsafe fn free_mib_table(table: *mut c_void) {
    if !table.is_null() {
        HeapFree(GetProcessHeap(), 0, table);
    }
}

pub unsafe fn convert_interface_guid_to_luid(guid: *const [u8; 16], luid: *mut u64) -> u32 {
    if guid.is_null() || luid.is_null() {
        return ERROR_INVALID_PARAMETER;
    }
    let g = core::ptr::read_unaligned(guid);
    match adapters().into_iter().find(|a| interface_guid(a) == g) {
        Some(a) => {
            core::ptr::write_unaligned(luid, a.luid);
            ERROR_SUCCESS
        }
        None => ERROR_NOT_FOUND,
    }
}

pub unsafe fn convert_interface_luid_to_index(luid: *const u64, index: *mut u32) -> u32 {
    if luid.is_null() || index.is_null() {
        return ERROR_INVALID_PARAMETER;
    }
    let l = core::ptr::read_unaligned(luid);
    match adapters().into_iter().find(|a| a.luid == l) {
        Some(a) => {
            core::ptr::write_unaligned(index, a.if_index);
            ERROR_SUCCESS
        }
        None => ERROR_NOT_FOUND,
    }
}

fn interface_metric(r: &Route) -> u32 {
    adapters().into_iter().find(|a| a.luid == r.luid).map(|a| if r.dest.is_ipv4() { a.ipv4_metric } else { a.ipv6_metric }).unwrap_or(0)
}

pub fn best_route(ip: &IpAddr) -> Option<Route> {
    let ip = match ip {
        IpAddr::V6(v) => v.to_ipv4_mapped().map(IpAddr::V4).unwrap_or(*ip),
        _ => *ip,
    };
    routes().into_iter().filter(|r| prefix_matches(&r.dest, r.prefix_len, &ip)).min_by_key(|r| (core::cmp::Reverse(r.prefix_len), r.metric + interface_metric(r)))
}

pub unsafe fn sockaddr_to_ip(addr: *const u8) -> Option<IpAddr> {
    if addr.is_null() {
        return None;
    }
    match core::ptr::read_unaligned(addr as *const u16) as u32 {
        AF_INET => {
            let b = core::ptr::read_unaligned(addr.add(4) as *const [u8; 4]);
            Some(IpAddr::V4(Ipv4Addr::from(b)))
        }
        AF_INET6 => Some(IpAddr::V6(Ipv6Addr::from(core::ptr::read_unaligned(addr.add(8) as *const [u8; 16])))),
        _ => None,
    }
}

pub unsafe fn get_best_interface_ex(dest: *const u8, index: *mut u32) -> u32 {
    if index.is_null() {
        return ERROR_INVALID_PARAMETER;
    }
    let Some(ip) = sockaddr_to_ip(dest) else {
        return ERROR_INVALID_PARAMETER;
    };
    match best_route(&ip) {
        Some(r) => {
            *index = r.if_index;
            ERROR_SUCCESS
        }
        None => ERROR_NETWORK_UNREACHABLE,
    }
}

pub type PhysicalInterfaceCallback = unsafe extern "system" fn(*mut c_void, *const [u8; 16], u32);

pub unsafe fn physical_interface_for_destination(dest: *const u8, out: *mut [u8; 16], handle: *mut usize) -> u32 {
    if out.is_null() {
        return ERROR_INVALID_PARAMETER;
    }
    if !handle.is_null() {
        *handle = 0;
    }
    let Some(ip) = sockaddr_to_ip(dest) else {
        return ERROR_INVALID_PARAMETER;
    };
    let Some(r) = best_route(&ip) else {
        return ERROR_NETWORK_UNREACHABLE;
    };
    match adapters().into_iter().find(|a| a.luid == r.luid) {
        Some(a) => {
            core::ptr::write_unaligned(out, interface_guid(&a));
            ERROR_SUCCESS
        }
        None => ERROR_NETWORK_UNREACHABLE,
    }
}

pub unsafe fn close_physical_interface_for_destination(handle: *mut usize) -> u32 {
    if handle.is_null() || *handle == 0 {
        return ERROR_SUCCESS;
    }
    ERROR_INVALID_HANDLE
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Policy {
    pub precedence: u32,
    pub label: u32,
}

const POLICY: [([u8; 16], u8, u32, u32); 9] = [
    ([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1], 128, 50, 0),
    ([0; 16], 0, 40, 1),
    ([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff, 0, 0, 0, 0], 96, 35, 4),
    ([0x20, 0x02, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0], 16, 30, 2),
    ([0x20, 0x01, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0], 32, 5, 5),
    ([0xfc, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0], 7, 3, 13),
    ([0; 16], 96, 1, 3),
    ([0xfe, 0xc0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0], 10, 1, 11),
    ([0x3f, 0xfe, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0], 16, 1, 12),
];

fn as_v6(ip: &IpAddr) -> Ipv6Addr {
    match ip {
        IpAddr::V4(v) => v.to_ipv6_mapped(),
        IpAddr::V6(v) => *v,
    }
}

pub fn policy(ip: &IpAddr) -> Policy {
    let v = IpAddr::V6(as_v6(ip));
    POLICY.iter().filter(|(p, len, _, _)| prefix_matches(&IpAddr::V6(Ipv6Addr::from(*p)), *len, &v)).max_by_key(|(_, len, _, _)| *len).map(|(_, _, precedence, label)| Policy { precedence: *precedence, label: *label }).unwrap_or(Policy { precedence: 40, label: 1 })
}

pub fn scope(ip: &IpAddr) -> u32 {
    let v = as_v6(ip);
    if let Some(m) = v.to_ipv4_mapped() {
        let o = m.octets();
        return if o[0] == 127 || (o[0] == 169 && o[1] == 254) { 2 } else { 14 };
    }
    let s = v.segments();
    if s[0] >> 8 == 0xff {
        return (s[0] & 0xf) as u32;
    }
    if v.is_loopback() || (s[0] & 0xffc0) == 0xfe80 {
        return 2;
    }
    if (s[0] & 0xffc0) == 0xfec0 {
        return 5;
    }
    14
}

pub fn source_for(dest: &IpAddr) -> Option<IpAddr> {
    let r = best_route(dest)?;
    let a = adapters().into_iter().find(|a| a.luid == r.luid)?;
    let want_v4 = match dest {
        IpAddr::V4(_) => true,
        IpAddr::V6(v) => v.to_ipv4_mapped().is_some(),
    };
    let ds = scope(dest);
    let mut c: Vec<IpAddr> = a.unicast.iter().map(|u| u.addr).filter(|x| x.is_ipv4() == want_v4).collect();
    c.sort_by_key(|x| (scope(x) != ds, scope(x) < ds));
    c.first().map(|x| if want_v4 && dest.is_ipv6() { IpAddr::V6(as_v6(x)) } else { *x })
}

fn common_prefix(a: &Ipv6Addr, b: &Ipv6Addr) -> u32 {
    let (x, y) = (u128::from_be_bytes(a.octets()), u128::from_be_bytes(b.octets()));
    (x ^ y).leading_zeros().min(64)
}

pub fn destination_order(a: &IpAddr, b: &IpAddr) -> core::cmp::Ordering {
    use core::cmp::Ordering::*;
    let (sa, sb) = (source_for(a), source_for(b));
    match (sa.is_some(), sb.is_some()) {
        (true, false) => return Less,
        (false, true) => return Greater,
        (false, false) => return Equal,
        _ => {}
    }
    let (sa, sb) = (sa.unwrap(), sb.unwrap());
    let m = |d: &IpAddr, s: &IpAddr| scope(d) == scope(s);
    if m(a, &sa) != m(b, &sb) {
        return if m(a, &sa) { Less } else { Greater };
    }
    let l = |d: &IpAddr, s: &IpAddr| policy(d).label == policy(s).label;
    if l(a, &sa) != l(b, &sb) {
        return if l(a, &sa) { Less } else { Greater };
    }
    let (pa, pb) = (policy(a).precedence, policy(b).precedence);
    if pa != pb {
        return pb.cmp(&pa);
    }
    let (ca, cb) = (scope(a), scope(b));
    if ca != cb {
        return ca.cmp(&cb);
    }
    let native = |d: &IpAddr| as_v6(d).to_ipv4_mapped().is_none();
    if native(a) && native(b) && native(&sa) && native(&sb) {
        let (la, lb) = (common_prefix(&as_v6(a), &as_v6(&sa)), common_prefix(&as_v6(b), &as_v6(&sb)));
        if la != lb {
            return lb.cmp(&la);
        }
    }
    Equal
}

pub unsafe fn address_list_sort(list: *mut u8, len: u32) -> Result<u32, u32> {
    if list.is_null() || (len as usize) < 4 {
        return Err(ERROR_INVALID_PARAMETER);
    }
    let n = core::ptr::read_unaligned(list as *const i32);
    if n < 0 {
        return Err(ERROR_INVALID_PARAMETER);
    }
    let need = 8 + 16 * n as usize;
    if (len as usize) < need {
        return Err(ERROR_INVALID_PARAMETER);
    }
    let mut entries: Vec<[u8; 16]> = (0..n as usize).map(|i| core::ptr::read_unaligned(list.add(8 + 16 * i) as *const [u8; 16])).collect();
    let mut keyed: Vec<([u8; 16], Option<IpAddr>)> = Vec::with_capacity(entries.len());
    for e in &entries {
        let p = usize::from_le_bytes(e[0..8].try_into().unwrap()) as *const u8;
        let l = i32::from_le_bytes(e[8..12].try_into().unwrap());
        if p.is_null() || l < 0x1c || core::ptr::read_unaligned(p as *const u16) as u32 != AF_INET6 {
            return Err(ERROR_INVALID_PARAMETER);
        }
        keyed.push((*e, sockaddr_to_ip(p)));
    }
    keyed.sort_by(|x, y| destination_order(x.1.as_ref().unwrap(), y.1.as_ref().unwrap()));
    for (i, (e, _)) in keyed.iter().enumerate() {
        entries[i] = *e;
        core::ptr::write_unaligned(list.add(8 + 16 * i) as *mut [u8; 16], *e);
    }
    Ok(need as u32)
}

struct NsiRow {
    module: [u8; 24],
    object: u32,
    key: Vec<u8>,
    rw: Vec<u8>,
}

static NSI_ROWS: Mutex<Vec<NsiRow>> = Mutex::new(Vec::new());

fn ip_module(m: &[u8]) -> bool {
    m == NPI_MS_IPV4_MODULEID || m == NPI_MS_IPV6_MODULEID
}

#[allow(clippy::too_many_arguments)]
pub unsafe fn nsi_set_all_parameters(store: u32, action: u32, module: *const u8, object: u32, key: *const u8, key_len: u32, rw: *const u8, rw_len: u32) -> u32 {
    if module.is_null() || key.is_null() || rw.is_null() {
        return ERROR_INVALID_PARAMETER;
    }
    let m: [u8; 24] = core::ptr::read_unaligned(module as *const [u8; 24]);
    if !ip_module(&m) || object != NL_INTERFACE_OBJECT {
        return ERROR_NOT_FOUND;
    }
    if store != NSI_STORE_ACTIVE || action != NSI_SET_DEFAULT || key_len != 8 || rw_len as usize != NL_INTERFACE_RW_SIZE {
        return ERROR_INVALID_PARAMETER;
    }
    let luid = core::ptr::read_unaligned(key as *const u64);
    if !adapters().iter().any(|a| a.luid == luid) {
        return ERROR_NOT_FOUND;
    }
    let new = core::slice::from_raw_parts(rw, NL_INTERFACE_RW_SIZE);
    let k = luid.to_le_bytes().to_vec();
    let mut rows = NSI_ROWS.lock().unwrap_or_else(|e| e.into_inner());
    let idx = match rows.iter().position(|r| r.module == m && r.object == object && r.key == k) {
        Some(i) => i,
        None => {
            rows.push(NsiRow { module: m, object, key: k, rw: vec![0xff; NL_INTERFACE_RW_SIZE] });
            rows.len() - 1
        }
    };
    let cur = &mut rows[idx].rw;
    for g in (0..NL_INTERFACE_RW_SIZE).step_by(4) {
        if new[g..g + 4] != [0xff; 4] {
            cur[g..g + 4].copy_from_slice(&new[g..g + 4]);
        }
    }
    ERROR_SUCCESS
}

pub fn nsi_interface_rw(module: &[u8; 24], luid: u64) -> Option<Vec<u8>> {
    let k = luid.to_le_bytes().to_vec();
    NSI_ROWS.lock().unwrap_or_else(|e| e.into_inner()).iter().find(|r| &r.module == module && r.object == NL_INTERFACE_OBJECT && r.key == k).map(|r| r.rw.clone())
}

pub fn nsi_reset() {
    NSI_ROWS.lock().unwrap_or_else(|e| e.into_inner()).clear();
}

pub const SIO_ADDRESS_LIST_QUERY: u32 = 0x4800_0016;
pub const SIO_ADDRESS_LIST_CHANGE: u32 = 0x2800_0017;
pub const SIO_ROUTING_INTERFACE_QUERY: u32 = 0xC800_0014;
pub const SIO_ROUTING_INTERFACE_CHANGE: u32 = 0x8800_0015;

pub fn local_addresses(family: u32) -> Vec<(IpAddr, u32)> {
    adapters().into_iter().filter(|a| a.if_type != IF_TYPE_SOFTWARE_LOOPBACK).flat_map(|a| a.unicast.into_iter().map(move |u| (u.addr, scope_of(&u.addr, a.if_index)))).filter(|(ip, _)| family_matches(family, ip)).collect()
}

pub fn address_list_size(family: u32) -> usize {
    let a = local_addresses(family);
    8 + 16 * a.len() + a.iter().map(|(ip, _)| if ip.is_ipv4() { 16 } else { 28 }).sum::<usize>()
}

pub unsafe fn write_address_list(family: u32, out: *mut u8, out_len: usize) -> Result<usize, usize> {
    let need = address_list_size(family);
    if out.is_null() || out_len < need {
        return Err(need);
    }
    let a = local_addresses(family);
    core::ptr::write_bytes(out, 0, need);
    core::ptr::write_unaligned(out as *mut i32, a.len() as i32);
    let mut tail = 8 + 16 * a.len();
    for (i, (ip, sc)) in a.iter().enumerate() {
        let b = sockaddr_bytes(ip, 0, *sc);
        core::ptr::copy_nonoverlapping(b.as_ptr(), out.add(tail), b.len());
        core::ptr::write_unaligned(out.add(8 + 16 * i) as *mut usize, out.add(tail) as usize);
        core::ptr::write_unaligned(out.add(16 + 16 * i) as *mut i32, b.len() as i32);
        tail += b.len();
    }
    Ok(need)
}

pub fn routing_interface(dest: &IpAddr) -> Option<Vec<u8>> {
    let src = source_for(dest)?;
    let r = best_route(dest)?;
    Some(sockaddr_bytes(&src, 0, scope_of(&src, r.if_index)))
}

pub const ERROR_NOT_SUPPORTED: u32 = 50;

#[allow(clippy::too_many_arguments)]
pub unsafe fn nsi_get_interface_rw(module: &[u8], object: u32, key: *const u8, key_len: u32, rw: *mut u8, rw_len: u32, rod: *mut u8, rod_len: u32, ros: *mut u8, ros_len: u32) -> u32 {
    if object != NL_INTERFACE_OBJECT {
        return ERROR_NOT_FOUND;
    }
    if key.is_null() || key_len != 8 || (rw.is_null() && rw_len != 0) || (!rw.is_null() && rw_len as usize != NL_INTERFACE_RW_SIZE) {
        return ERROR_INVALID_PARAMETER;
    }
    if !rod.is_null() || rod_len != 0 || !ros.is_null() || ros_len != 0 {
        return ERROR_NOT_SUPPORTED;
    }
    let m: [u8; 24] = module.try_into().unwrap_or([0; 24]);
    let luid = core::ptr::read_unaligned(key as *const u64);
    if !adapters().iter().any(|a| a.luid == luid) {
        return ERROR_NOT_FOUND;
    }
    match nsi_interface_rw(&m, luid) {
        Some(row) => {
            if !rw.is_null() {
                core::ptr::copy_nonoverlapping(row.as_ptr(), rw, NL_INTERFACE_RW_SIZE);
            }
            ERROR_SUCCESS
        }
        None => ERROR_NOT_FOUND,
    }
}
