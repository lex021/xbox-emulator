use core::ffi::c_void;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use super::{cstr, deny_dest, deny_null, deny_resolve, deny_sock_bool, dest_allowed, is_loopback, make_trampoline, note, passthrough, resolve_export, sockaddr_ip, v4, write_jump, wstr};

extern "system" {
    fn SetLastError(code: u32);
}

pub const SIO_GET_EXTENSION_FUNCTION_POINTER: u32 = 0xC800_0006;
pub const WSAID_CONNECTEX: [u8; 16] = [0xb9, 0x07, 0xa2, 0x25, 0xf3, 0xdd, 0x60, 0x46, 0x8e, 0xe9, 0x76, 0xe5, 0x8c, 0x74, 0x06, 0x3e];
pub const WSAID_WSASENDMSG: [u8; 16] = [0x12, 0xe7, 0x41, 0xa4, 0x4f, 0x75, 0xca, 0x43, 0x84, 0xa7, 0x0d, 0xee, 0x44, 0xcf, 0x60, 0x6d];
const WSAENETUNREACH: u32 = 10051;
const WSAHOST_NOT_FOUND: i32 = 11001;
const DNS_ERROR_RCODE_NAME_ERROR: i32 = 9003;
const NI_NUMERICHOST: i32 = 0x02;
const NI_NAMEREQD: i32 = 0x04;
const INVALID_SOCKET: usize = !0;

const X_WSAIOCTL: usize = 0;
const X_WSASENDMSG: usize = 1;
const X_CONNECTBYLIST: usize = 2;
const X_JOINLEAF: usize = 3;
const X_GETHOSTBYADDR: usize = 4;
const X_GETNAMEINFO: usize = 5;
const X_GETNAMEINFOW: usize = 6;
const X_DNSQUERYW: usize = 7;
const X_DNSQUERYA: usize = 8;
const X_DNSQUERYUTF8: usize = 9;
const X_DNSQUERYEX: usize = 10;
static XORIG: [AtomicUsize; 11] = [const { AtomicUsize::new(0) }; 11];
static REDIRECT: AtomicBool = AtomicBool::new(false);
static CX_REAL: [AtomicUsize; 4] = [const { AtomicUsize::new(0) }; 4];
static SM_REAL: [AtomicUsize; 4] = [const { AtomicUsize::new(0) }; 4];

macro_rules! xorig {
    ($idx:expr, fn($($at:ty),*) -> $rt:ty) => {{
        let f: unsafe extern "system" fn($($at),*) -> $rt = core::mem::transmute(XORIG[$idx].load(Ordering::Relaxed));
        f
    }};
}

unsafe fn allowed(sa: *const u8, len: i32) -> bool {
    REDIRECT.load(Ordering::Relaxed) && dest_allowed(sa, len)
}

fn ip_allowed_for_lookup(ip: Option<[u8; 16]>) -> bool {
    REDIRECT.load(Ordering::Relaxed) && ip.map_or(false, |i| is_loopback(&i))
}

fn register(table: &[AtomicUsize; 4], real: usize) -> Option<usize> {
    for (i, slot) in table.iter().enumerate() {
        let cur = slot.load(Ordering::Acquire);
        if cur == real {
            return Some(i);
        }
        if cur == 0 && slot.compare_exchange(0, real, Ordering::AcqRel, Ordering::Acquire).map_or_else(|v| v == real, |_| true) {
            return Some(i);
        }
    }
    None
}

type ConnectEx = unsafe extern "system" fn(usize, *const u8, i32, *mut c_void, u32, *mut u32, *mut c_void) -> i32;
type SendMsg = unsafe extern "system" fn(usize, *const u8, u32, *mut u32, *mut c_void, *const c_void) -> i32;

unsafe fn connectex_call(i: usize, s: usize, name: *const u8, len: i32, buf: *mut c_void, blen: u32, sent: *mut u32, ov: *mut c_void) -> i32 {
    let real = CX_REAL[i].load(Ordering::Acquire);
    if real == 0 || !allowed(name, len) {
        deny_dest("ConnectEx", name, len);
        return 0;
    }
    let f: ConnectEx = core::mem::transmute(real);
    f(s, name, len, buf, blen, sent, ov)
}

unsafe fn msg_dest(msg: *const u8) -> (*const u8, i32) {
    if msg.is_null() {
        return (core::ptr::null(), 0);
    }
    (core::ptr::read_unaligned(msg as *const *const u8), core::ptr::read_unaligned(msg.add(8) as *const i32))
}

unsafe fn sendmsg_check(api: &str, msg: *const u8) -> bool {
    let (name, len) = msg_dest(msg);
    if name.is_null() {
        if REDIRECT.load(Ordering::Relaxed) {
            return true;
        }
        note(&format!("{api} on a connected socket"));
        SetLastError(WSAENETUNREACH);
        return false;
    }
    if !allowed(name, len) {
        deny_dest(api, name, len);
        return false;
    }
    true
}

unsafe fn sendmsg_call(i: usize, s: usize, msg: *const u8, flags: u32, sent: *mut u32, ov: *mut c_void, routine: *const c_void) -> i32 {
    let real = SM_REAL[i].load(Ordering::Acquire);
    if real == 0 || !sendmsg_check("WSASendMsg (extension pointer)", msg) {
        if real == 0 {
            SetLastError(WSAENETUNREACH);
        }
        return -1;
    }
    let f: SendMsg = core::mem::transmute(real);
    f(s, msg, flags, sent, ov, routine)
}

macro_rules! cx_slot {
    ($name:ident, $i:expr) => {
        unsafe extern "system" fn $name(s: usize, name: *const u8, len: i32, buf: *mut c_void, blen: u32, sent: *mut u32, ov: *mut c_void) -> i32 {
            connectex_call($i, s, name, len, buf, blen, sent, ov)
        }
    };
}
macro_rules! sm_slot {
    ($name:ident, $i:expr) => {
        unsafe extern "system" fn $name(s: usize, msg: *const u8, flags: u32, sent: *mut u32, ov: *mut c_void, routine: *const c_void) -> i32 {
            sendmsg_call($i, s, msg, flags, sent, ov, routine)
        }
    };
}
cx_slot!(cx0, 0);
cx_slot!(cx1, 1);
cx_slot!(cx2, 2);
cx_slot!(cx3, 3);
sm_slot!(sm0, 0);
sm_slot!(sm1, 1);
sm_slot!(sm2, 2);
sm_slot!(sm3, 3);

unsafe extern "system" fn cx_deny(_s: usize, name: *const u8, len: i32, _b: *mut c_void, _bl: u32, _sent: *mut u32, _ov: *mut c_void) -> i32 {
    deny_dest("ConnectEx (no free wrapper slot)", name, len);
    0
}
unsafe extern "system" fn sm_deny(_s: usize, _m: *const u8, _f: u32, _sent: *mut u32, _ov: *mut c_void, _r: *const c_void) -> i32 {
    note("WSASendMsg (no free wrapper slot)");
    SetLastError(WSAENETUNREACH);
    -1
}

fn cx_wrapper(i: usize) -> usize {
    [cx0 as *const () as usize, cx1 as *const () as usize, cx2 as *const () as usize, cx3 as *const () as usize][i]
}
fn sm_wrapper(i: usize) -> usize {
    [sm0 as *const () as usize, sm1 as *const () as usize, sm2 as *const () as usize, sm3 as *const () as usize][i]
}

#[cfg(test)]
pub(crate) fn guarded_inner(p: usize) -> Option<usize> {
    (0..4).find_map(|i| {
        if cx_wrapper(i) == p {
            Some(CX_REAL[i].load(Ordering::Acquire))
        } else if sm_wrapper(i) == p {
            Some(SM_REAL[i].load(Ordering::Acquire))
        } else {
            None
        }
    })
}

pub unsafe extern "system" fn h_wsaioctl(
    s: usize, code: u32, inb: *const u8, inlen: u32, outb: *mut u8, outlen: u32, ret: *mut u32, ov: *mut c_void, routine: *const c_void,
) -> i32 {
    let rc = xorig!(X_WSAIOCTL, fn(usize, u32, *const u8, u32, *mut u8, u32, *mut u32, *mut c_void, *const c_void) -> i32)(s, code, inb, inlen, outb, outlen, ret, ov, routine);
    if rc == 0 {
        guard_extension_out(code, inb, inlen, outb, outlen);
    }
    rc
}

pub unsafe fn guard_extension_out(code: u32, inb: *const u8, inlen: u32, outb: *mut u8, outlen: u32) {
    if code != SIO_GET_EXTENSION_FUNCTION_POINTER || inb.is_null() || inlen < 16 || outb.is_null() || outlen < 8 {
        return;
    }
    let mut guid = [0u8; 16];
    core::ptr::copy_nonoverlapping(inb, guid.as_mut_ptr(), 16);
    let real = core::ptr::read_unaligned(outb as *const usize);
    if real == 0 {
        return;
    }
    let wrapped = if guid == WSAID_CONNECTEX {
        Some(register(&CX_REAL, real).map_or(cx_deny as *const () as usize, cx_wrapper))
    } else if guid == WSAID_WSASENDMSG {
        Some(register(&SM_REAL, real).map_or(sm_deny as *const () as usize, sm_wrapper))
    } else {
        None
    };
    if let Some(w) = wrapped {
        core::ptr::write_unaligned(outb as *mut usize, w);
    }
}

pub unsafe extern "system" fn h_wsasendmsg(s: usize, msg: *const u8, flags: u32, sent: *mut u32, ov: *mut c_void, routine: *const c_void) -> i32 {
    if !sendmsg_check("WSASendMsg", msg) {
        return -1;
    }
    xorig!(X_WSASENDMSG, fn(usize, *const u8, u32, *mut u32, *mut c_void, *const c_void) -> i32)(s, msg, flags, sent, ov, routine)
}

pub unsafe extern "system" fn h_connectbylist(
    s: usize, list: *const u8, ll: *mut u32, la: *mut c_void, rl: *mut u32, ra: *mut c_void, timeout: *const c_void, reserved: *mut c_void,
) -> i32 {
    if !list.is_null() {
        let n = core::ptr::read_unaligned(list as *const i32).max(0) as usize;
        for i in 0..n.min(4096) {
            let e = list.add(8 + i * 16);
            let sa = core::ptr::read_unaligned(e as *const *const u8);
            let len = core::ptr::read_unaligned(e.add(8) as *const i32);
            if sa.is_null() || !allowed(sa, len) {
                deny_dest("WSAConnectByList", sa, len);
                return 0;
            }
        }
    }
    xorig!(X_CONNECTBYLIST, fn(usize, *const u8, *mut u32, *mut c_void, *mut u32, *mut c_void, *const c_void, *mut c_void) -> i32)(s, list, ll, la, rl, ra, timeout, reserved)
}

pub unsafe extern "system" fn h_wsajoinleaf(
    s: usize, name: *const u8, len: i32, a: *mut c_void, b: *mut c_void, c: *mut c_void, d: *mut c_void, flags: u32,
) -> usize {
    if !allowed(name, len) {
        deny_dest("WSAJoinLeaf", name, len);
        return INVALID_SOCKET;
    }
    xorig!(X_JOINLEAF, fn(usize, *const u8, i32, *mut c_void, *mut c_void, *mut c_void, *mut c_void, u32) -> usize)(s, name, len, a, b, c, d, flags)
}

unsafe fn raw_ip(addr: *const u8, len: i32, family: i32) -> Option<[u8; 16]> {
    if addr.is_null() {
        return None;
    }
    match (family, len) {
        (2, l) if l >= 4 => Some(v4([*addr, *addr.add(1), *addr.add(2), *addr.add(3)])),
        (23, l) if l >= 16 => {
            let mut o = [0u8; 16];
            core::ptr::copy_nonoverlapping(addr, o.as_mut_ptr(), 16);
            Some(o)
        }
        _ => None,
    }
}

pub unsafe extern "system" fn h_gethostbyaddr(addr: *const u8, len: i32, family: i32) -> *mut c_void {
    if !ip_allowed_for_lookup(raw_ip(addr, len, family)) {
        note("gethostbyaddr reverse lookup of a non-loopback address");
        SetLastError(WSAHOST_NOT_FOUND as u32);
        return core::ptr::null_mut();
    }
    xorig!(X_GETHOSTBYADDR, fn(*const u8, i32, i32) -> *mut c_void)(addr, len, family)
}

unsafe fn nameinfo_flags(api: &str, sa: *const u8, salen: i32, host_wanted: bool, flags: i32) -> Result<i32, i32> {
    if !host_wanted || flags & NI_NUMERICHOST != 0 || ip_allowed_for_lookup(sockaddr_ip(sa, salen)) {
        return Ok(flags);
    }
    if flags & NI_NAMEREQD != 0 {
        note(&format!("{api} reverse lookup with NI_NAMEREQD"));
        SetLastError(WSAHOST_NOT_FOUND as u32);
        return Err(WSAHOST_NOT_FOUND);
    }
    Ok(flags | NI_NUMERICHOST)
}

pub unsafe extern "system" fn h_getnameinfo(sa: *const u8, salen: i32, host: *mut u8, hostlen: u32, serv: *mut u8, servlen: u32, flags: i32) -> i32 {
    match nameinfo_flags("getnameinfo", sa, salen, !host.is_null() && hostlen > 0, flags) {
        Ok(f) => xorig!(X_GETNAMEINFO, fn(*const u8, i32, *mut u8, u32, *mut u8, u32, i32) -> i32)(sa, salen, host, hostlen, serv, servlen, f),
        Err(e) => e,
    }
}

pub unsafe extern "system" fn h_getnameinfow(sa: *const u8, salen: i32, host: *mut u16, hostlen: u32, serv: *mut u16, servlen: u32, flags: i32) -> i32 {
    match nameinfo_flags("GetNameInfoW", sa, salen, !host.is_null() && hostlen > 0, flags) {
        Ok(f) => xorig!(X_GETNAMEINFOW, fn(*const u8, i32, *mut u16, u32, *mut u16, u32, i32) -> i32)(sa, salen, host, hostlen, serv, servlen, f),
        Err(e) => e,
    }
}

fn dns_name_ok(name: &str) -> bool {
    REDIRECT.load(Ordering::Relaxed) && passthrough(name)
}

fn dns_deny(api: &str, name: &str) -> i32 {
    note(&format!("{api} '{name}'"));
    unsafe { SetLastError(DNS_ERROR_RCODE_NAME_ERROR as u32) };
    DNS_ERROR_RCODE_NAME_ERROR
}

type DnsQ<T> = unsafe extern "system" fn(*const T, u16, u32, *mut c_void, *mut *mut c_void, *mut c_void) -> i32;

pub unsafe extern "system" fn h_dnsqueryw(name: *const u16, ty: u16, opts: u32, extra: *mut c_void, res: *mut *mut c_void, reserved: *mut c_void) -> i32 {
    let n = if name.is_null() { String::new() } else { wstr(name) };
    if !dns_name_ok(&n) {
        return dns_deny("DnsQuery_W", &n);
    }
    let f: DnsQ<u16> = core::mem::transmute(XORIG[X_DNSQUERYW].load(Ordering::Relaxed));
    f(name, ty, opts, extra, res, reserved)
}

pub unsafe extern "system" fn h_dnsquerya(name: *const u8, ty: u16, opts: u32, extra: *mut c_void, res: *mut *mut c_void, reserved: *mut c_void) -> i32 {
    let n = if name.is_null() { String::new() } else { cstr(name) };
    if !dns_name_ok(&n) {
        return dns_deny("DnsQuery_A", &n);
    }
    let f: DnsQ<u8> = core::mem::transmute(XORIG[X_DNSQUERYA].load(Ordering::Relaxed));
    f(name, ty, opts, extra, res, reserved)
}

pub unsafe extern "system" fn h_dnsqueryutf8(name: *const u8, ty: u16, opts: u32, extra: *mut c_void, res: *mut *mut c_void, reserved: *mut c_void) -> i32 {
    let n = if name.is_null() { String::new() } else { cstr(name) };
    if !dns_name_ok(&n) {
        return dns_deny("DnsQuery_UTF8", &n);
    }
    let f: DnsQ<u8> = core::mem::transmute(XORIG[X_DNSQUERYUTF8].load(Ordering::Relaxed));
    f(name, ty, opts, extra, res, reserved)
}

pub unsafe extern "system" fn h_dnsqueryex(req: *const u8, res: *mut c_void, cancel: *mut c_void) -> i32 {
    let n = if req.is_null() {
        String::new()
    } else {
        let p = core::ptr::read_unaligned(req.add(8) as *const *const u16);
        if p.is_null() { String::new() } else { wstr(p) }
    };
    if !dns_name_ok(&n) {
        return dns_deny("DnsQueryEx", &n);
    }
    xorig!(X_DNSQUERYEX, fn(*const u8, *mut c_void, *mut c_void) -> i32)(req, res, cancel)
}

unsafe extern "system" fn deny_socket_handle() -> usize {
    note("WSAJoinLeaf");
    SetLastError(WSAENETUNREACH);
    INVALID_SOCKET
}
unsafe extern "system" fn deny_dns() -> i32 {
    note("DNS API query");
    SetLastError(DNS_ERROR_RCODE_NAME_ERROR as u32);
    DNS_ERROR_RCODE_NAME_ERROR
}
unsafe extern "system" fn deny_send_sock() -> i32 {
    note("WSASendMsg");
    SetLastError(WSAENETUNREACH);
    -1
}

pub fn install(redirect: bool) {
    REDIRECT.store(redirect, Ordering::SeqCst);
    let ws = b"ws2_32.dll\0";
    let dn = b"dnsapi.dll\0";
    let table: [(&[u8], &[u8], usize, usize, usize); 11] = [
        (ws, b"WSAIoctl\0", X_WSAIOCTL, h_wsaioctl as *const () as usize, 0),
        (ws, b"WSASendMsg\0", X_WSASENDMSG, h_wsasendmsg as *const () as usize, deny_send_sock as *const () as usize),
        (ws, b"WSAConnectByList\0", X_CONNECTBYLIST, h_connectbylist as *const () as usize, deny_sock_bool as *const () as usize),
        (ws, b"WSAJoinLeaf\0", X_JOINLEAF, h_wsajoinleaf as *const () as usize, deny_socket_handle as *const () as usize),
        (ws, b"gethostbyaddr\0", X_GETHOSTBYADDR, h_gethostbyaddr as *const () as usize, deny_null as *const () as usize),
        (ws, b"getnameinfo\0", X_GETNAMEINFO, h_getnameinfo as *const () as usize, deny_resolve as *const () as usize),
        (ws, b"GetNameInfoW\0", X_GETNAMEINFOW, h_getnameinfow as *const () as usize, deny_resolve as *const () as usize),
        (dn, b"DnsQuery_W\0", X_DNSQUERYW, h_dnsqueryw as *const () as usize, deny_dns as *const () as usize),
        (dn, b"DnsQuery_A\0", X_DNSQUERYA, h_dnsquerya as *const () as usize, deny_dns as *const () as usize),
        (dn, b"DnsQuery_UTF8\0", X_DNSQUERYUTF8, h_dnsqueryutf8 as *const () as usize, deny_dns as *const () as usize),
        (dn, b"DnsQueryEx\0", X_DNSQUERYEX, h_dnsqueryex as *const () as usize, deny_dns as *const () as usize),
    ];
    let (mut hooked, mut denied, mut missing) = (0, Vec::new(), Vec::new());
    for (m, f, idx, hook, deny) in table {
        let name = String::from_utf8_lossy(&f[..f.len() - 1]).into_owned();
        if idx == X_WSAIOCTL && super::wsaioctl_shared() {
            hooked += 1;
            continue;
        }
        let p = unsafe { resolve_export(m, f) };
        if p.is_null() {
            missing.push(name);
            continue;
        }
        match unsafe { make_trampoline(p) } {
            Some(t) => {
                XORIG[idx].store(t, Ordering::SeqCst);
                if unsafe { write_jump(p, hook) } { hooked += 1 } else { missing.push(name) }
            }
            None if deny != 0 => {
                if unsafe { write_jump(p, deny) } { denied.push(name) } else { missing.push(name) }
            }
            None => missing.push(format!("{name} (ConnectEx/WSASendMsg extension pointers unguarded)")),
        }
    }
    crate::record_raw("net_guard", "installed", &format!(
        "extended entry points ({}): {hooked}/11 hooked (ConnectEx and WSASendMsg extension pointers wrapped via WSAIoctl){}{}",
        if redirect { "redirect" } else { "deny-all" },
        if denied.is_empty() { String::new() } else { format!("; fail-closed (no trampoline): {}", denied.join(", ")) },
        if missing.is_empty() { String::new() } else { format!("; NOT PATCHED: {}", missing.join(", ")) }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    extern "system" {
        fn GetLastError() -> u32;
    }

    static SERIAL: Mutex<()> = Mutex::new(());
    static CALLS: AtomicUsize = AtomicUsize::new(0);
    static LAST_FLAGS: AtomicUsize = AtomicUsize::new(0);

    fn sa4(a: [u8; 4], port: u16) -> [u8; 16] {
        let mut s = [0u8; 16];
        s[0] = 2;
        s[2..4].copy_from_slice(&port.to_be_bytes());
        s[4..8].copy_from_slice(&a);
        s
    }

    fn guard(redirect: bool) -> std::sync::MutexGuard<'static, ()> {
        let g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        REDIRECT.store(redirect, Ordering::SeqCst);
        CALLS.store(0, Ordering::SeqCst);
        g
    }

    unsafe extern "system" fn fake_connectex(_s: usize, _n: *const u8, _l: i32, _b: *mut c_void, _bl: u32, _sent: *mut u32, _ov: *mut c_void) -> i32 {
        CALLS.fetch_add(1, Ordering::SeqCst);
        1
    }
    unsafe extern "system" fn fake_sendmsg(_s: usize, _m: *const u8, _f: u32, _sent: *mut u32, _ov: *mut c_void, _r: *const c_void) -> i32 {
        CALLS.fetch_add(1, Ordering::SeqCst);
        0
    }
    unsafe extern "system" fn fake_ioctl(_s: usize, code: u32, inb: *const u8, _il: u32, outb: *mut u8, _ol: u32, ret: *mut u32, _ov: *mut c_void, _r: *const c_void) -> i32 {
        if code != SIO_GET_EXTENSION_FUNCTION_POINTER {
            core::ptr::write_unaligned(outb as *mut usize, 0x1234);
            return 0;
        }
        let mut g = [0u8; 16];
        core::ptr::copy_nonoverlapping(inb, g.as_mut_ptr(), 16);
        let f = if g == WSAID_CONNECTEX { fake_connectex as *const () as usize } else if g == WSAID_WSASENDMSG { fake_sendmsg as *const () as usize } else { 0x5678 };
        core::ptr::write_unaligned(outb as *mut usize, f);
        *ret = 8;
        0
    }
    unsafe extern "system" fn fake_bylist(_s: usize, _l: *const u8, _a: *mut u32, _b: *mut c_void, _c: *mut u32, _d: *mut c_void, _t: *const c_void, _r: *mut c_void) -> i32 {
        CALLS.fetch_add(1, Ordering::SeqCst);
        1
    }
    unsafe extern "system" fn fake_joinleaf(_s: usize, _n: *const u8, _l: i32, _a: *mut c_void, _b: *mut c_void, _c: *mut c_void, _d: *mut c_void, _f: u32) -> usize {
        CALLS.fetch_add(1, Ordering::SeqCst);
        77
    }
    unsafe extern "system" fn fake_hostbyaddr(_a: *const u8, _l: i32, _t: i32) -> *mut c_void {
        CALLS.fetch_add(1, Ordering::SeqCst);
        0x99 as *mut c_void
    }
    unsafe extern "system" fn fake_nameinfo(_sa: *const u8, _l: i32, _h: *mut u8, _hl: u32, _s: *mut u8, _sl: u32, flags: i32) -> i32 {
        CALLS.fetch_add(1, Ordering::SeqCst);
        LAST_FLAGS.store(flags as usize, Ordering::SeqCst);
        0
    }
    unsafe extern "system" fn fake_dnsw(_n: *const u16, _t: u16, _o: u32, _e: *mut c_void, _r: *mut *mut c_void, _x: *mut c_void) -> i32 {
        CALLS.fetch_add(1, Ordering::SeqCst);
        0
    }
    unsafe extern "system" fn fake_dnsex(_q: *const u8, _r: *mut c_void, _c: *mut c_void) -> i32 {
        CALLS.fetch_add(1, Ordering::SeqCst);
        0
    }

    unsafe fn ext_ptr(guid: &[u8; 16]) -> usize {
        XORIG[X_WSAIOCTL].store(fake_ioctl as *const () as usize, Ordering::SeqCst);
        let mut out = 0usize;
        let mut ret = 0u32;
        let rc = h_wsaioctl(1, SIO_GET_EXTENSION_FUNCTION_POINTER, guid.as_ptr(), 16, &mut out as *mut usize as *mut u8, 8, &mut ret, core::ptr::null_mut(), core::ptr::null());
        assert_eq!(rc, 0);
        out
    }

    #[test]
    fn connectex_from_wsaioctl_blocks_literal_external_ip_and_allows_loopback() {
        let _g = guard(true);
        unsafe {
            let p = ext_ptr(&WSAID_CONNECTEX);
            assert_ne!(p, fake_connectex as *const () as usize);
            assert!([cx0 as *const () as usize, cx1 as *const () as usize, cx2 as *const () as usize, cx3 as *const () as usize].contains(&p));
            let f: ConnectEx = core::mem::transmute(p);
            let ext = sa4([203, 0, 113, 7], 443);
            assert_eq!(f(1, ext.as_ptr(), 16, core::ptr::null_mut(), 0, core::ptr::null_mut(), core::ptr::null_mut()), 0);
            assert_eq!(GetLastError(), WSAENETUNREACH);
            assert_eq!(CALLS.load(Ordering::SeqCst), 0);
            let lo = sa4([127, 0, 0, 1], 7777);
            assert_eq!(f(1, lo.as_ptr(), 16, core::ptr::null_mut(), 0, core::ptr::null_mut(), core::ptr::null_mut()), 1);
            assert_eq!(CALLS.load(Ordering::SeqCst), 1);
            assert_eq!(ext_ptr(&WSAID_CONNECTEX), p);
        }
    }

    #[test]
    fn deny_mode_connectex_blocks_loopback_too() {
        let _g = guard(false);
        unsafe {
            let f: ConnectEx = core::mem::transmute(ext_ptr(&WSAID_CONNECTEX));
            let lo = sa4([127, 0, 0, 1], 80);
            assert_eq!(f(1, lo.as_ptr(), 16, core::ptr::null_mut(), 0, core::ptr::null_mut(), core::ptr::null_mut()), 0);
            assert_eq!(CALLS.load(Ordering::SeqCst), 0);
        }
    }

    #[test]
    fn other_wsaioctl_requests_pass_through_unchanged() {
        let _g = guard(true);
        unsafe {
            XORIG[X_WSAIOCTL].store(fake_ioctl as *const () as usize, Ordering::SeqCst);
            let mut out = 0usize;
            let mut ret = 0u32;
            assert_eq!(h_wsaioctl(1, 0x4004_667F, core::ptr::null(), 0, &mut out as *mut usize as *mut u8, 8, &mut ret, core::ptr::null_mut(), core::ptr::null()), 0);
            assert_eq!(out, 0x1234);
            let other = [7u8; 16];
            assert_eq!(ext_ptr(&other), 0x5678);
        }
    }

    #[test]
    fn wsasendmsg_extension_and_export_check_the_message_destination() {
        let _g = guard(true);
        unsafe {
            let f: SendMsg = core::mem::transmute(ext_ptr(&WSAID_WSASENDMSG));
            let ext = sa4([8, 8, 8, 8], 53);
            let mut msg = [0u8; 56];
            core::ptr::write_unaligned(msg.as_mut_ptr() as *mut usize, ext.as_ptr() as usize);
            core::ptr::write_unaligned(msg.as_mut_ptr().add(8) as *mut i32, 16);
            assert_eq!(f(1, msg.as_ptr(), 0, core::ptr::null_mut(), core::ptr::null_mut(), core::ptr::null()), -1);
            XORIG[X_WSASENDMSG].store(fake_sendmsg as *const () as usize, Ordering::SeqCst);
            assert_eq!(h_wsasendmsg(1, msg.as_ptr(), 0, core::ptr::null_mut(), core::ptr::null_mut(), core::ptr::null()), -1);
            assert_eq!(CALLS.load(Ordering::SeqCst), 0);
            core::ptr::write_unaligned(msg.as_mut_ptr() as *mut usize, 0);
            assert_eq!(h_wsasendmsg(1, msg.as_ptr(), 0, core::ptr::null_mut(), core::ptr::null_mut(), core::ptr::null()), 0);
            assert_eq!(f(1, msg.as_ptr(), 0, core::ptr::null_mut(), core::ptr::null_mut(), core::ptr::null()), 0);
            assert_eq!(CALLS.load(Ordering::SeqCst), 2);
        }
    }

    #[test]
    fn connect_by_list_fails_closed_if_any_address_is_external() {
        let _g = guard(true);
        unsafe {
            XORIG[X_CONNECTBYLIST].store(fake_bylist as *const () as usize, Ordering::SeqCst);
            let lo = sa4([127, 0, 0, 1], 80);
            let ext = sa4([1, 1, 1, 1], 80);
            let mut list = [0u8; 8 + 32];
            core::ptr::write_unaligned(list.as_mut_ptr() as *mut i32, 2);
            core::ptr::write_unaligned(list.as_mut_ptr().add(8) as *mut usize, lo.as_ptr() as usize);
            core::ptr::write_unaligned(list.as_mut_ptr().add(16) as *mut i32, 16);
            core::ptr::write_unaligned(list.as_mut_ptr().add(24) as *mut usize, ext.as_ptr() as usize);
            core::ptr::write_unaligned(list.as_mut_ptr().add(32) as *mut i32, 16);
            let n = core::ptr::null_mut();
            assert_eq!(h_connectbylist(1, list.as_ptr(), n, n as *mut c_void, n, n as *mut c_void, core::ptr::null(), n as *mut c_void), 0);
            assert_eq!(GetLastError(), WSAENETUNREACH);
            core::ptr::write_unaligned(list.as_mut_ptr() as *mut i32, 1);
            assert_eq!(h_connectbylist(1, list.as_ptr(), n, n as *mut c_void, n, n as *mut c_void, core::ptr::null(), n as *mut c_void), 1);
            assert_eq!(CALLS.load(Ordering::SeqCst), 1);
        }
    }

    #[test]
    fn join_leaf_external_returns_invalid_socket() {
        let _g = guard(true);
        unsafe {
            XORIG[X_JOINLEAF].store(fake_joinleaf as *const () as usize, Ordering::SeqCst);
            let n = core::ptr::null_mut();
            let ext = sa4([239, 1, 2, 3], 9);
            assert_eq!(h_wsajoinleaf(1, ext.as_ptr(), 16, n, n, n, n, 0), INVALID_SOCKET);
            let lo = sa4([127, 0, 0, 1], 9);
            assert_eq!(h_wsajoinleaf(1, lo.as_ptr(), 16, n, n, n, n, 0), 77);
        }
    }

    #[test]
    fn reverse_lookups_of_external_addresses_never_reach_dns() {
        let _g = guard(true);
        unsafe {
            XORIG[X_GETHOSTBYADDR].store(fake_hostbyaddr as *const () as usize, Ordering::SeqCst);
            assert!(h_gethostbyaddr([93u8, 184, 216, 34].as_ptr(), 4, 2).is_null());
            assert_eq!(GetLastError(), WSAHOST_NOT_FOUND as u32);
            assert_eq!(h_gethostbyaddr([127u8, 0, 0, 1].as_ptr(), 4, 2) as usize, 0x99);
            XORIG[X_GETNAMEINFO].store(fake_nameinfo as *const () as usize, Ordering::SeqCst);
            let ext = sa4([93, 184, 216, 34], 443);
            let mut host = [0u8; 64];
            assert_eq!(h_getnameinfo(ext.as_ptr(), 16, host.as_mut_ptr(), 64, core::ptr::null_mut(), 0, 0), 0);
            assert_eq!(LAST_FLAGS.load(Ordering::SeqCst) as i32, NI_NUMERICHOST);
            assert_eq!(h_getnameinfo(ext.as_ptr(), 16, host.as_mut_ptr(), 64, core::ptr::null_mut(), 0, NI_NAMEREQD), WSAHOST_NOT_FOUND);
            assert_eq!(CALLS.load(Ordering::SeqCst), 2);
            let lo = sa4([127, 0, 0, 1], 80);
            assert_eq!(h_getnameinfo(lo.as_ptr(), 16, host.as_mut_ptr(), 64, core::ptr::null_mut(), 0, NI_NAMEREQD), 0);
            assert_eq!(LAST_FLAGS.load(Ordering::SeqCst) as i32, NI_NAMEREQD);
        }
    }

    #[test]
    fn dns_api_queries_for_external_names_are_refused() {
        let _g = guard(true);
        unsafe {
            XORIG[X_DNSQUERYW].store(fake_dnsw as *const () as usize, Ordering::SeqCst);
            XORIG[X_DNSQUERYEX].store(fake_dnsex as *const () as usize, Ordering::SeqCst);
            let ext: Vec<u16> = "ubisoft.com".encode_utf16().chain(Some(0)).collect();
            let n = core::ptr::null_mut();
            assert_eq!(h_dnsqueryw(ext.as_ptr(), 1, 0, n, core::ptr::null_mut(), n), DNS_ERROR_RCODE_NAME_ERROR);
            let lo: Vec<u16> = "localhost".encode_utf16().chain(Some(0)).collect();
            assert_eq!(h_dnsqueryw(lo.as_ptr(), 1, 0, n, core::ptr::null_mut(), n), 0);
            let mut req = [0u8; 64];
            core::ptr::write_unaligned(req.as_mut_ptr().add(8) as *mut usize, ext.as_ptr() as usize);
            assert_eq!(h_dnsqueryex(req.as_ptr(), n, n), DNS_ERROR_RCODE_NAME_ERROR);
            core::ptr::write_unaligned(req.as_mut_ptr().add(8) as *mut usize, lo.as_ptr() as usize);
            assert_eq!(h_dnsqueryex(req.as_ptr(), n, n), 0);
            assert_eq!(CALLS.load(Ordering::SeqCst), 2);
            REDIRECT.store(false, Ordering::SeqCst);
            assert_eq!(h_dnsqueryw(lo.as_ptr(), 1, 0, n, core::ptr::null_mut(), n), DNS_ERROR_RCODE_NAME_ERROR);
        }
    }
}
