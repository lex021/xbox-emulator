use core::ffi::c_void;
use core::sync::atomic::{AtomicU32, Ordering};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6};
use std::sync::OnceLock;

use super::{AF_INET, AF_INET6, AF_UNSPEC, ERROR_INSUFFICIENT_BUFFER, ERROR_INVALID_PARAMETER, ERROR_SUCCESS, NIC_IPV4, NIC_IPV6, NIC_IF_INDEX};

extern "system" {
    fn CreateFileMappingW(file: *mut c_void, sa: *mut c_void, protect: u32, high: u32, low: u32, name: *const u16) -> *mut c_void;
    fn MapViewOfFile(h: *mut c_void, access: u32, high: u32, low: u32, n: usize) -> *mut c_void;
    fn GetCurrentProcessId() -> u32;
    fn GetSystemTimeAsFileTime(t: *mut u64);
    fn GetModuleHandleA(name: *const u8) -> *mut c_void;
    fn GetProcAddress(h: *mut c_void, name: *const u8) -> *mut c_void;
}

pub const MIB_TCP_STATE_LISTEN: u32 = 2;
pub const MIB_TCP_STATE_SYN_SENT: u32 = 3;
pub const MIB_TCP_STATE_ESTAB: u32 = 5;
pub const MIB_TCP_STATE_RESERVED: u32 = 100;
pub const SIO_TCP_INFO: u32 = 0xD800_0027;
pub const SO_CONNECT_TIME: i32 = 0x700C;
pub const SO_ERROR: i32 = 0x1007;
pub const TCPSTATE_CLOSED: u32 = 0;
pub const TCPSTATE_SYN_SENT: u32 = 2;
pub const TCPSTATE_SYN_RCVD: u32 = 3;

pub const TCP_TABLE_OWNER_PID_LISTENER: u32 = 3;
pub const TCP_TABLE_OWNER_PID_CONNECTIONS: u32 = 4;
pub const TCP_TABLE_OWNER_PID_ALL: u32 = 5;
pub const TCP_TABLE_OWNER_MODULE_LISTENER: u32 = 6;
pub const TCP_TABLE_OWNER_MODULE_CONNECTIONS: u32 = 7;
pub const TCP_TABLE_OWNER_MODULE_ALL: u32 = 8;
pub const UDP_TABLE_OWNER_PID: u32 = 1;
pub const UDP_TABLE_OWNER_MODULE: u32 = 2;

pub const TCP_ROW_OWNER_MODULE_SIZE: usize = 0xa0;
pub const TCP6_ROW_OWNER_MODULE_SIZE: usize = 0xc0;
pub const UDP_ROW_OWNER_MODULE_SIZE: usize = 0xa0;
pub const UDP6_ROW_OWNER_MODULE_SIZE: usize = 0xb0;
pub const TCP_ROW_OWNER_PID_SIZE: usize = 24;
pub const TCP6_ROW_OWNER_PID_SIZE: usize = 56;
pub const UDP_ROW_OWNER_PID_SIZE: usize = 12;
pub const UDP6_ROW_OWNER_PID_SIZE: usize = 28;

const SOCK_STREAM: i32 = 1;
const SOCK_DGRAM: i32 = 2;
const SOL_SOCKET: i32 = 0xffff;
const SO_TYPE: i32 = 0x1008;
const SO_ACCEPTCONN: i32 = 0x0002;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Proto {
    Tcp,
    Udp,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Endpoint {
    pub socket: u64,
    pub pid: u32,
    pub proto: Proto,
    pub local: SocketAddr,
    pub remote: Option<SocketAddr>,
    pub state: u32,
    pub created: u64,
}

const CAPACITY: usize = 256;

#[cfg(test)]
pub(crate) static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[repr(C)]
#[derive(Clone, Copy)]
struct Slot {
    socket: u64,
    created: u64,
    pid: u32,
    proto: u8,
    family: u8,
    used: u8,
    has_remote: u8,
    state: u32,
    local: [u8; 16],
    local_scope: u32,
    local_port: u16,
    remote_port: u16,
    remote: [u8; 16],
    remote_scope: u32,
}

#[repr(C)]
struct Shared {
    lock: AtomicU32,
    slots: [Slot; CAPACITY],
}

fn shared() -> Option<&'static Shared> {
    static P: OnceLock<usize> = OnceLock::new();
    let p = *P.get_or_init(|| unsafe {
        let name: Vec<u16> = format!("Local\\TomBox.NetView.Sockets.{}", GetCurrentProcessId()).encode_utf16().chain(Some(0)).collect();
        let size = core::mem::size_of::<Shared>() as u32;
        let h = CreateFileMappingW(-1isize as *mut c_void, core::ptr::null_mut(), 0x04, 0, size, name.as_ptr());
        if h.is_null() {
            return 0;
        }
        MapViewOfFile(h, 0x0006, 0, 0, size as usize) as usize
    });
    if p == 0 {
        None
    } else {
        Some(unsafe { &*(p as *const Shared) })
    }
}

fn with_slots<R>(f: impl FnOnce(&mut [Slot; CAPACITY]) -> R) -> Option<R> {
    let s = shared()?;
    while s.lock.compare_exchange(0, 1, Ordering::Acquire, Ordering::Relaxed).is_err() {
        std::thread::yield_now();
    }
    let slots = unsafe { &mut *(core::ptr::addr_of!(s.slots) as *mut [Slot; CAPACITY]) };
    let r = f(slots);
    s.lock.store(0, Ordering::Release);
    Some(r)
}

fn split(a: &SocketAddr) -> (u8, [u8; 16], u32, u16) {
    match a {
        SocketAddr::V4(v) => {
            let mut b = [0u8; 16];
            b[..4].copy_from_slice(&v.ip().octets());
            (AF_INET as u8, b, 0, v.port())
        }
        SocketAddr::V6(v) => (AF_INET6 as u8, v.ip().octets(), v.scope_id(), v.port()),
    }
}

fn join(family: u8, b: [u8; 16], scope: u32, port: u16) -> SocketAddr {
    if family as u32 == AF_INET {
        SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(b[0], b[1], b[2], b[3]), port))
    } else {
        SocketAddr::V6(SocketAddrV6::new(Ipv6Addr::from(b), port, 0, scope))
    }
}

fn to_slot(e: &Endpoint) -> Slot {
    let (family, local, local_scope, local_port) = split(&e.local);
    let (has_remote, remote, remote_scope, remote_port) = match &e.remote {
        Some(r) => {
            let (_, b, s, p) = split(r);
            (1, b, s, p)
        }
        None => (0, [0u8; 16], 0, 0),
    };
    Slot { socket: e.socket, created: e.created, pid: e.pid, proto: (e.proto == Proto::Udp) as u8, family, used: 1, has_remote, state: e.state, local, local_scope, local_port, remote_port, remote, remote_scope }
}

fn from_slot(s: &Slot) -> Endpoint {
    Endpoint {
        socket: s.socket,
        pid: s.pid,
        proto: if s.proto == 1 { Proto::Udp } else { Proto::Tcp },
        local: join(s.family, s.local, s.local_scope, s.local_port),
        remote: if s.has_remote == 1 { Some(join(s.family, s.remote, s.remote_scope, s.remote_port)) } else { None },
        state: s.state,
        created: s.created,
    }
}

pub fn now_filetime() -> u64 {
    let mut t = 0u64;
    unsafe { GetSystemTimeAsFileTime(&mut t) };
    t
}

pub fn console_local(a: SocketAddr) -> SocketAddr {
    match a {
        SocketAddr::V4(v) if !v.ip().is_loopback() && !v.ip().is_unspecified() => SocketAddr::V4(SocketAddrV4::new(NIC_IPV4, v.port())),
        SocketAddr::V6(v) if !v.ip().is_loopback() && !v.ip().is_unspecified() && v.ip().to_ipv4_mapped().map(|m| !m.is_loopback()).unwrap_or(true) => SocketAddr::V6(SocketAddrV6::new(NIC_IPV6, v.port(), 0, NIC_IF_INDEX)),
        other => other,
    }
}

pub fn report(e: Endpoint) -> bool {
    let slot = to_slot(&e);
    with_slots(|slots| {
        if let Some(s) = slots.iter_mut().find(|s| s.used == 1 && s.socket == e.socket && s.pid == e.pid) {
            *s = slot;
            return true;
        }
        match slots.iter_mut().find(|s| s.used == 0) {
            Some(s) => {
                *s = slot;
                true
            }
            None => false,
        }
    })
    .unwrap_or(false)
}

pub fn forget(socket: u64) {
    let pid = unsafe { GetCurrentProcessId() };
    with_slots(|slots| {
        for s in slots.iter_mut().filter(|s| s.used == 1 && s.socket == socket && s.pid == pid) {
            s.used = 0;
        }
    });
}

pub fn snapshot() -> Vec<Endpoint> {
    with_slots(|slots| slots.iter().filter(|s| s.used == 1).map(from_slot).collect()).unwrap_or_default()
}

type GetSockOpt = unsafe extern "system" fn(usize, i32, i32, *mut u8, *mut i32) -> i32;
type GetSockName = unsafe extern "system" fn(usize, *mut u8, *mut i32) -> i32;

fn ws_fn(name: &[u8]) -> usize {
    unsafe {
        let m = GetModuleHandleA(b"ws2_32.dll\0".as_ptr());
        if m.is_null() {
            return 0;
        }
        GetProcAddress(m, name.as_ptr()) as usize
    }
}

unsafe fn socket_type(s: usize) -> Option<i32> {
    let f = ws_fn(b"getsockopt\0");
    if f == 0 {
        return None;
    }
    let f: GetSockOpt = core::mem::transmute(f);
    let mut t = 0i32;
    let mut n = 4i32;
    if f(s, SOL_SOCKET, SO_TYPE, &mut t as *mut i32 as *mut u8, &mut n) != 0 {
        return None;
    }
    Some(t)
}

unsafe fn accepting(s: usize) -> bool {
    let f = ws_fn(b"getsockopt\0");
    if f == 0 {
        return false;
    }
    let f: GetSockOpt = core::mem::transmute(f);
    let mut v = 0i32;
    let mut n = 4i32;
    f(s, SOL_SOCKET, SO_ACCEPTCONN, &mut v as *mut i32 as *mut u8, &mut n) == 0 && v != 0
}

pub unsafe fn parse_sockaddr(p: *const u8, len: i32) -> Option<SocketAddr> {
    if p.is_null() || len < 16 {
        return None;
    }
    let b = core::slice::from_raw_parts(p, len as usize);
    let family = u16::from_le_bytes([b[0], b[1]]) as u32;
    let port = u16::from_be_bytes([b[2], b[3]]);
    if family == AF_INET {
        Some(SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(b[4], b[5], b[6], b[7]), port)))
    } else if family == AF_INET6 && len >= 28 {
        let mut a = [0u8; 16];
        a.copy_from_slice(&b[8..24]);
        Some(SocketAddr::V6(SocketAddrV6::new(Ipv6Addr::from(a), port, 0, u32::from_le_bytes([b[24], b[25], b[26], b[27]]))))
    } else {
        None
    }
}

unsafe fn local_name(s: usize) -> Option<SocketAddr> {
    let f = ws_fn(b"getsockname\0");
    if f == 0 {
        return None;
    }
    let f: GetSockName = core::mem::transmute(f);
    let mut b = [0u8; 128];
    let mut n = b.len() as i32;
    if f(s, b.as_mut_ptr(), &mut n) != 0 {
        return None;
    }
    parse_sockaddr(b.as_ptr(), n)
}

unsafe fn peer_name(s: usize) -> Option<SocketAddr> {
    let f = ws_fn(b"getpeername\0");
    if f == 0 {
        return None;
    }
    let f: GetSockName = core::mem::transmute(f);
    let mut b = [0u8; 128];
    let mut n = b.len() as i32;
    if f(s, b.as_mut_ptr(), &mut n) != 0 {
        return None;
    }
    parse_sockaddr(b.as_ptr(), n)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    Bound,
    Listening,
    Connected,
    Accepted,
    Sent,
    Connecting,
}

pub unsafe fn report_event(s: usize, event: Event, remote: *const u8, remote_len: i32) -> bool {
    let proto = match socket_type(s) {
        Some(SOCK_STREAM) => Proto::Tcp,
        Some(SOCK_DGRAM) => Proto::Udp,
        _ => return false,
    };
    let Some(local) = local_name(s) else {
        return false;
    };
    let pid = GetCurrentProcessId();
    let prior = snapshot().into_iter().find(|e| e.socket == s as u64 && e.pid == pid && e.proto == proto);
    let (remote, state) = match proto {
        Proto::Udp => (None, 0),
        Proto::Tcp => match event {
            Event::Listening => (None, MIB_TCP_STATE_LISTEN),
            Event::Bound => match prior {
                Some(p) => (p.remote, p.state),
                None => (None, if accepting(s) { MIB_TCP_STATE_LISTEN } else { MIB_TCP_STATE_RESERVED }),
            },
            Event::Accepted => match peer_name(s).or_else(|| parse_sockaddr(remote, remote_len)) {
                Some(r) => (Some(r), MIB_TCP_STATE_ESTAB),
                None => return false,
            },
            Event::Connecting => match parse_sockaddr(remote, remote_len) {
                Some(r) => (Some(r), MIB_TCP_STATE_SYN_SENT),
                None => return false,
            },
            Event::Connected | Event::Sent => match parse_sockaddr(remote, remote_len).or_else(|| peer_name(s)) {
                Some(r) => (Some(r), MIB_TCP_STATE_ESTAB),
                None if accepting(s) => (None, MIB_TCP_STATE_LISTEN),
                None => match prior {
                    Some(p) => (p.remote, p.state),
                    None => (None, MIB_TCP_STATE_RESERVED),
                },
            },
        },
    };
    let created = prior.map(|e| e.created).unwrap_or_else(now_filetime);
    report(Endpoint { socket: s as u64, pid, proto, local: console_local(local), remote, state, created })
}

pub unsafe fn report_accepted(s: usize, local: *const u8, local_len: i32, remote: *const u8, remote_len: i32) -> bool {
    if socket_type(s) != Some(SOCK_STREAM) {
        return false;
    }
    let (Some(l), Some(r)) = (parse_sockaddr(local, local_len), parse_sockaddr(remote, remote_len)) else {
        return false;
    };
    let pid = GetCurrentProcessId();
    let created = snapshot().into_iter().find(|e| e.socket == s as u64 && e.pid == pid).map(|e| e.created).unwrap_or_else(now_filetime);
    report(Endpoint { socket: s as u64, pid, proto: Proto::Tcp, local: console_local(l), remote: Some(r), state: MIB_TCP_STATE_ESTAB, created })
}

type WsaIoctl = unsafe extern "system" fn(usize, u32, *const c_void, u32, *mut c_void, u32, *mut u32, *mut c_void, *mut c_void) -> i32;

pub unsafe fn tcp_state(s: usize) -> Option<u32> {
    let f = ws_fn(b"WSAIoctl\0");
    if f == 0 {
        return None;
    }
    let f: WsaIoctl = core::mem::transmute(f);
    let version = 0u32;
    let mut info = [0u8; 256];
    let mut got = 0u32;
    if f(s, SIO_TCP_INFO, &version as *const u32 as *const c_void, 4, info.as_mut_ptr() as *mut c_void, info.len() as u32, &mut got, core::ptr::null_mut(), core::ptr::null_mut()) != 0 || got < 4 {
        return None;
    }
    Some(u32::from_le_bytes([info[0], info[1], info[2], info[3]]))
}

unsafe fn sock_opt_u32(s: usize, name: i32) -> Option<u32> {
    let f = ws_fn(b"getsockopt\0");
    if f == 0 {
        return None;
    }
    let f: GetSockOpt = core::mem::transmute(f);
    let mut v = 0u32;
    let mut n = 4i32;
    if f(s, SOL_SOCKET, name, &mut v as *mut u32 as *mut u8, &mut n) != 0 {
        return None;
    }
    Some(v)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pending {
    Still,
    Established,
    Failed,
}

pub unsafe fn connect_progress(s: usize) -> Pending {
    match tcp_state(s) {
        Some(TCPSTATE_CLOSED) => Pending::Failed,
        Some(TCPSTATE_SYN_SENT | TCPSTATE_SYN_RCVD) => Pending::Still,
        Some(_) => Pending::Established,
        None => match (sock_opt_u32(s, SO_CONNECT_TIME), sock_opt_u32(s, SO_ERROR)) {
            (Some(t), _) if t != u32::MAX => Pending::Established,
            (_, Some(e)) if e != 0 => Pending::Failed,
            _ => Pending::Still,
        },
    }
}

fn settle(e: Endpoint) -> Option<Endpoint> {
    if e.proto != Proto::Tcp || e.state != MIB_TCP_STATE_SYN_SENT {
        return Some(e);
    }
    match unsafe { connect_progress(e.socket as usize) } {
        Pending::Still => Some(e),
        Pending::Failed => {
            forget(e.socket);
            None
        }
        Pending::Established => {
            let up = Endpoint { state: MIB_TCP_STATE_ESTAB, ..e };
            report(up);
            Some(up)
        }
    }
}

pub unsafe fn report_socket(s: usize, remote: *const u8, remote_len: i32) -> bool {
    report_event(s, Event::Connected, remote, remote_len)
}

pub fn report_std<S: std::os::windows::io::AsRawSocket>(s: &S) -> bool {
    report_owned(s.as_raw_socket())
}

pub fn report_owned(s: u64) -> bool {
    unsafe {
        let event = match socket_type(s as usize) {
            Some(SOCK_STREAM) if accepting(s as usize) => Event::Listening,
            Some(_) => Event::Connected,
            None => return false,
        };
        report_event(s as usize, event, core::ptr::null(), 0)
    }
}

pub fn endpoints() -> Vec<Endpoint> {
    let pid = unsafe { GetCurrentProcessId() };
    let mut out = Vec::new();
    for e in snapshot() {
        if e.pid == pid {
            let want = if e.proto == Proto::Tcp { SOCK_STREAM } else { SOCK_DGRAM };
            if unsafe { socket_type(e.socket as usize) } != Some(want) {
                forget(e.socket);
                continue;
            }
            match settle(e) {
                Some(e) => out.push(e),
                None => continue,
            }
            continue;
        }
        out.push(e);
    }
    out
}

fn family_of(a: &SocketAddr) -> u32 {
    if a.is_ipv4() {
        AF_INET
    } else {
        AF_INET6
    }
}

fn key(e: &Endpoint) -> (Vec<u8>, u16, Vec<u8>, u16) {
    let ip = |a: &SocketAddr| match a.ip() {
        IpAddr::V4(v) => v.octets().to_vec(),
        IpAddr::V6(v) => v.octets().to_vec(),
    };
    let (r, rp) = e.remote.map(|r| (ip(&r), r.port())).unwrap_or_default();
    (ip(&e.local), e.local.port(), r, rp)
}

fn port_dword(p: u16) -> [u8; 4] {
    let b = p.to_be_bytes();
    [b[0], b[1], 0, 0]
}

fn v4(a: &SocketAddr) -> [u8; 4] {
    match a.ip() {
        IpAddr::V4(v) => v.octets(),
        IpAddr::V6(_) => [0; 4],
    }
}

fn v6(a: &SocketAddr) -> ([u8; 16], u32) {
    match a {
        SocketAddr::V6(v) => (v.ip().octets(), v.scope_id()),
        SocketAddr::V4(_) => ([0; 16], 0),
    }
}

fn unspecified(family: u32) -> SocketAddr {
    if family == AF_INET {
        SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0))
    } else {
        SocketAddr::V6(SocketAddrV6::new(Ipv6Addr::UNSPECIFIED, 0, 0, 0))
    }
}

fn select(proto: Proto, family: u32, order: bool, keep: impl Fn(&Endpoint) -> bool) -> Vec<Endpoint> {
    let mut v: Vec<Endpoint> = endpoints().into_iter().filter(|e| e.proto == proto && family_of(&e.local) == family && keep(e)).collect();
    if order {
        v.sort_by_key(key);
    }
    v
}

fn tcp_row(e: &Endpoint, family: u32, module: bool) -> Vec<u8> {
    let r = e.remote.unwrap_or_else(|| unspecified(family));
    if family == AF_INET {
        let mut b = vec![0u8; if module { TCP_ROW_OWNER_MODULE_SIZE } else { TCP_ROW_OWNER_PID_SIZE }];
        b[0..4].copy_from_slice(&e.state.to_le_bytes());
        b[4..8].copy_from_slice(&v4(&e.local));
        b[8..0xc].copy_from_slice(&port_dword(e.local.port()));
        b[0xc..0x10].copy_from_slice(&v4(&r));
        b[0x10..0x14].copy_from_slice(&port_dword(r.port()));
        b[0x14..0x18].copy_from_slice(&e.pid.to_le_bytes());
        if module {
            b[0x18..0x20].copy_from_slice(&e.created.to_le_bytes());
        }
        b
    } else {
        let mut b = vec![0u8; if module { TCP6_ROW_OWNER_MODULE_SIZE } else { TCP6_ROW_OWNER_PID_SIZE }];
        let (la, ls) = v6(&e.local);
        let (ra, rs) = v6(&r);
        b[0..0x10].copy_from_slice(&la);
        b[0x10..0x14].copy_from_slice(&ls.to_le_bytes());
        b[0x14..0x18].copy_from_slice(&port_dword(e.local.port()));
        b[0x18..0x28].copy_from_slice(&ra);
        b[0x28..0x2c].copy_from_slice(&rs.to_le_bytes());
        b[0x2c..0x30].copy_from_slice(&port_dword(r.port()));
        b[0x30..0x34].copy_from_slice(&e.state.to_le_bytes());
        b[0x34..0x38].copy_from_slice(&e.pid.to_le_bytes());
        if module {
            b[0x38..0x40].copy_from_slice(&e.created.to_le_bytes());
        }
        b
    }
}

fn udp_row(e: &Endpoint, family: u32, module: bool) -> Vec<u8> {
    if family == AF_INET {
        let mut b = vec![0u8; if module { UDP_ROW_OWNER_MODULE_SIZE } else { UDP_ROW_OWNER_PID_SIZE }];
        b[0..4].copy_from_slice(&v4(&e.local));
        b[4..8].copy_from_slice(&port_dword(e.local.port()));
        b[8..0xc].copy_from_slice(&e.pid.to_le_bytes());
        if module {
            b[0x10..0x18].copy_from_slice(&e.created.to_le_bytes());
        }
        b
    } else {
        let mut b = vec![0u8; if module { UDP6_ROW_OWNER_MODULE_SIZE } else { UDP6_ROW_OWNER_PID_SIZE }];
        let (la, ls) = v6(&e.local);
        b[0..0x10].copy_from_slice(&la);
        b[0x10..0x14].copy_from_slice(&ls.to_le_bytes());
        b[0x14..0x18].copy_from_slice(&port_dword(e.local.port()));
        b[0x18..0x1c].copy_from_slice(&e.pid.to_le_bytes());
        if module {
            b[0x20..0x28].copy_from_slice(&e.created.to_le_bytes());
        }
        b
    }
}

fn table(rows: Vec<Vec<u8>>, header: usize, empty_row: usize) -> Vec<u8> {
    let mut t = vec![0u8; header];
    t[0..4].copy_from_slice(&(rows.len() as u32).to_le_bytes());
    for r in &rows {
        t.extend_from_slice(r);
    }
    if rows.is_empty() {
        t.resize(header + empty_row, 0);
    }
    t
}

pub fn tcp_table(family: u32, class: u32, order: bool) -> Result<Vec<u8>, u32> {
    if !matches!(family, AF_INET | AF_INET6) {
        return Err(ERROR_INVALID_PARAMETER);
    }
    let (module, listen, conn) = match class {
        TCP_TABLE_OWNER_PID_LISTENER => (false, true, false),
        TCP_TABLE_OWNER_PID_CONNECTIONS => (false, false, true),
        TCP_TABLE_OWNER_PID_ALL => (false, true, true),
        TCP_TABLE_OWNER_MODULE_LISTENER => (true, true, false),
        TCP_TABLE_OWNER_MODULE_CONNECTIONS => (true, false, true),
        TCP_TABLE_OWNER_MODULE_ALL => (true, true, true),
        _ => return Err(ERROR_INVALID_PARAMETER),
    };
    let rows: Vec<Vec<u8>> = select(Proto::Tcp, family, order, |e| match e.state {
        MIB_TCP_STATE_RESERVED => false,
        MIB_TCP_STATE_LISTEN => listen,
        _ => conn,
    }).iter().map(|e| tcp_row(e, family, module)).collect();
    let row = match (family, module) {
        (AF_INET, true) => TCP_ROW_OWNER_MODULE_SIZE,
        (AF_INET, false) => TCP_ROW_OWNER_PID_SIZE,
        (_, true) => TCP6_ROW_OWNER_MODULE_SIZE,
        _ => TCP6_ROW_OWNER_PID_SIZE,
    };
    Ok(table(rows, if module { 8 } else { 4 }, row))
}

pub fn udp_table(family: u32, class: u32, order: bool) -> Result<Vec<u8>, u32> {
    if !matches!(family, AF_INET | AF_INET6) {
        return Err(ERROR_INVALID_PARAMETER);
    }
    let module = match class {
        UDP_TABLE_OWNER_PID => false,
        UDP_TABLE_OWNER_MODULE => true,
        _ => return Err(ERROR_INVALID_PARAMETER),
    };
    let rows: Vec<Vec<u8>> = select(Proto::Udp, family, order, |_| true).iter().map(|e| udp_row(e, family, module)).collect();
    let row = match (family, module) {
        (AF_INET, true) => UDP_ROW_OWNER_MODULE_SIZE,
        (AF_INET, false) => UDP_ROW_OWNER_PID_SIZE,
        (_, true) => UDP6_ROW_OWNER_MODULE_SIZE,
        _ => UDP6_ROW_OWNER_PID_SIZE,
    };
    Ok(table(rows, if module { 8 } else { 4 }, row))
}

unsafe fn copy_out(t: Result<Vec<u8>, u32>, buf: *mut u8, size: *mut u32) -> u32 {
    let t = match t {
        Ok(t) => t,
        Err(e) => return e,
    };
    if buf.is_null() || (*size as usize) < t.len() {
        *size = t.len() as u32;
        return ERROR_INSUFFICIENT_BUFFER;
    }
    core::ptr::copy_nonoverlapping(t.as_ptr(), buf, t.len());
    *size = t.len() as u32;
    ERROR_SUCCESS
}

pub unsafe fn get_extended_tcp_table(buf: *mut u8, size: *mut u32, order: bool, family: u32, class: u32, reserved: u32) -> u32 {
    if size.is_null() || reserved != 0 || family == AF_UNSPEC {
        return ERROR_INVALID_PARAMETER;
    }
    copy_out(tcp_table(family, class, order), buf, size)
}

pub unsafe fn get_extended_udp_table(buf: *mut u8, size: *mut u32, order: bool, family: u32, class: u32, reserved: u32) -> u32 {
    if size.is_null() || reserved != 0 || family == AF_UNSPEC {
        return ERROR_INVALID_PARAMETER;
    }
    copy_out(udp_table(family, class, order), buf, size)
}
