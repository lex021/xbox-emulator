//! In-process network policy for the title.
//!
//! Project rule: the title must never talk to Fortnite/Epic production services. The title
//! and the SDKs it links use WinHTTP and Winsock directly, so the policy is enforced inside
//! the title process, at the API entry points, before anything leaves the machine.
//!
//! Two modes:
//!
//!   * **deny (default)** — every name resolution, outbound connect/sendto and WinHTTP
//!     connection fails locally.
//!   * **redirect** — `TOMBOX_NET_REDIRECT=<host>` (the project owner's own server). Every
//!     host name the title asks for is replaced by `<host>`:
//!       - `getaddrinfo` / `GetAddrInfoW` / `GetAddrInfoExW/A` / `gethostbyname` /
//!         `WSAConnectByNameW/A` resolve `<host>` instead of the requested name;
//!       - `WinHttpConnect` connects to `<host>` (same port), so TLS validates against
//!         `<host>`'s certificate and the Host header names `<host>`;
//!       - `connect` / `WSAConnect` / `sendto` / `WSASendTo` are allowed only towards
//!         loopback or an address `<host>` resolves to; any other destination is refused.
//!     Null/empty names, `localhost` and this machine's own name are passed through.
//!
//! Mechanism: each entry point is overwritten with `mov rax, imm64; jmp rax` to a hook. In
//! redirect mode the displaced instructions are relocated into a trampoline (iced-x86
//! block encoder) so the hook can call the original.
//!
//! Limits (documented, not hidden): a socket connected through `ConnectEx` to a literal IP
//! address is not covered; DNS-over-anything not using these APIs is not covered; other
//! processes are unaffected. Only one compat DLL per process installs the policy.
//! `TOMBOX_ALLOW_NETWORK=1` disables the policy entirely (never use with this title).

use core::ffi::c_void;
use core::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

mod ext;
mod ioctl;

extern "system" {
    fn GetModuleHandleA(name: *const u8) -> *mut c_void;
    fn LoadLibraryA(name: *const u8) -> *mut c_void;
    fn GetProcAddress(h: *mut c_void, name: *const u8) -> *mut c_void;
    fn VirtualProtect(addr: *mut c_void, size: usize, new_protect: u32, old_protect: *mut u32) -> i32;
    fn VirtualAlloc(addr: *mut c_void, size: usize, ty: u32, protect: u32) -> *mut c_void;
    fn FlushInstructionCache(process: *mut c_void, addr: *const c_void, size: usize) -> i32;
    fn GetCurrentProcess() -> *mut c_void;
    fn GetCurrentProcessId() -> u32;
    fn SetLastError(code: u32);
    fn GetLastError() -> u32;
    fn GetComputerNameExA(kind: u32, buf: *mut u8, size: *mut u32) -> i32;
    fn GetTickCount64() -> u64;
}

const WSAENETUNREACH: u32 = 10051;
const WSAHOST_NOT_FOUND: i32 = 11001;
const ERROR_WINHTTP_CANNOT_CONNECT: u32 = 12029;

static DENIED: AtomicUsize = AtomicUsize::new(0);
static REDIRECTED: AtomicUsize = AtomicUsize::new(0);

fn note(what: &str) {
    let n = DENIED.fetch_add(1, Ordering::Relaxed);
    if n < 40 || n % 500 == 0 {
        crate::record_raw("net_guard", "denied", &format!("{what} (total blocked calls: {})", n + 1));
    }
}

fn note_redirect(api: &str, original: &str) {
    let n = REDIRECTED.fetch_add(1, Ordering::Relaxed);
    if n < 80 || n % 500 == 0 {
        crate::record_raw("net_guard", "redirect", &format!(
            "{api}: '{original}' -> '{}' (total redirected: {})", target().map(|t| t.name.as_str()).unwrap_or("?"), n + 1));
    }
}

// ---- deny stubs (argument-agnostic: the x64 caller cleans up) --------------------------------

unsafe extern "system" fn deny_sock() -> i32 {
    note("socket connect/send");
    SetLastError(WSAENETUNREACH); // WSAGetLastError reads the thread's last-error value
    -1
}
unsafe extern "system" fn deny_sock_bool() -> i32 {
    note("WSAConnectByName/List");
    SetLastError(WSAENETUNREACH);
    0
}
unsafe extern "system" fn deny_resolve() -> i32 {
    note("name resolution");
    SetLastError(WSAHOST_NOT_FOUND as u32);
    WSAHOST_NOT_FOUND
}
unsafe extern "system" fn deny_null() -> usize {
    note("WinHTTP connect / gethostbyname");
    SetLastError(ERROR_WINHTTP_CANNOT_CONNECT);
    0
}
unsafe extern "system" fn deny_false() -> i32 {
    note("WinHttpSendRequest");
    SetLastError(ERROR_WINHTTP_CANNOT_CONNECT);
    0
}

// ---- patching ---------------------------------------------------------------------------------

pub(crate) unsafe fn resolve_export(module: &[u8], func: &[u8]) -> *mut u8 {
    let mut h = GetModuleHandleA(module.as_ptr());
    if h.is_null() {
        h = LoadLibraryA(module.as_ptr());
    }
    if h.is_null() {
        return core::ptr::null_mut();
    }
    GetProcAddress(h, func.as_ptr()) as *mut u8
}

pub(crate) unsafe fn write_jump(p: *mut u8, stub: usize) -> bool {
    let mut code = [0u8; 12]; // mov rax, imm64 ; jmp rax
    code[0] = 0x48;
    code[1] = 0xB8;
    code[2..10].copy_from_slice(&(stub as u64).to_le_bytes());
    code[10] = 0xFF;
    code[11] = 0xE0;
    let mut old = 0u32;
    if VirtualProtect(p as *mut c_void, code.len(), 0x40, &mut old) == 0 {
        return false;
    }
    core::ptr::copy_nonoverlapping(code.as_ptr(), p, code.len());
    let mut tmp = 0u32;
    VirtualProtect(p as *mut c_void, code.len(), old, &mut tmp);
    FlushInstructionCache(GetCurrentProcess(), p as *const c_void, code.len());
    true
}

/// Executable page within +-1 GiB of `near` (so rip-relative operands can be re-encoded).
unsafe fn alloc_near(near: usize) -> *mut u8 {
    let base = near & !0xFFFF;
    for i in 1..0x3F00usize {
        for cand in [base.wrapping_sub(i << 16), base.wrapping_add(i << 16)] {
            let p = VirtualAlloc(cand as *mut c_void, 0x1000, 0x3000, 0x40);
            if !p.is_null() {
                return p as *mut u8;
            }
        }
    }
    core::ptr::null_mut()
}

/// Relocate the first >= 12 bytes of `p` into a trampoline that continues in the original.
unsafe fn make_trampoline(p: *mut u8) -> Option<usize> {
    use iced_x86::{BlockEncoder, BlockEncoderOptions, Decoder, DecoderOptions, FlowControl, InstructionBlock};
    let bytes = core::slice::from_raw_parts(p, 32);
    let mut dec = Decoder::with_ip(64, bytes, p as u64, DecoderOptions::NONE);
    let mut insns = Vec::new();
    let mut len = 0usize;
    while len < 12 {
        let i = dec.decode();
        if i.is_invalid() {
            return None;
        }
        // A return or indirect branch inside the displaced bytes means the function is too
        // short / oddly shaped to hook safely.
        if matches!(i.flow_control(), FlowControl::Return | FlowControl::IndirectBranch | FlowControl::Interrupt | FlowControl::Exception) {
            return None;
        }
        len += i.len();
        insns.push(i);
    }
    let tramp = alloc_near(p as usize);
    if tramp.is_null() {
        return None;
    }
    let block = InstructionBlock::new(&insns, tramp as u64);
    let enc = BlockEncoder::encode(64, block, BlockEncoderOptions::NONE).ok()?;
    let code = enc.code_buffer;
    if code.len() + 14 > 0x1000 {
        return None;
    }
    core::ptr::copy_nonoverlapping(code.as_ptr(), tramp, code.len());
    // jmp qword ptr [rip+0] ; dq original+len
    let tail = tramp.add(code.len());
    let j: [u8; 6] = [0xFF, 0x25, 0, 0, 0, 0];
    core::ptr::copy_nonoverlapping(j.as_ptr(), tail, 6);
    core::ptr::write_unaligned(tail.add(6) as *mut u64, p as u64 + len as u64);
    FlushInstructionCache(GetCurrentProcess(), tramp as *const c_void, 0x1000);
    Some(tramp as usize)
}

// ---- redirect mode ------------------------------------------------------------------------------

struct Target {
    name: String,
    ansi: Vec<u8>,  // NUL-terminated
    wide: Vec<u16>, // NUL-terminated
}
static TARGET: OnceLock<Option<Target>> = OnceLock::new();

fn target() -> Option<&'static Target> {
    TARGET.get().and_then(|t| t.as_ref())
}

/// Trampolines to the original functions, indexed by `H_*`.
const H_GETADDRINFO: usize = 0;
const H_GETADDRINFOW: usize = 1;
const H_GETADDRINFOEXW: usize = 2;
const H_GETADDRINFOEXA: usize = 3;
const H_GETHOSTBYNAME: usize = 4;
const H_CONNECT: usize = 5;
const H_WSACONNECT: usize = 6;
const H_CONNECTBYNAMEW: usize = 7;
const H_CONNECTBYNAMEA: usize = 8;
const H_SENDTO: usize = 9;
const H_WSASENDTO: usize = 10;
const H_WINHTTPCONNECT: usize = 11;
const H_WINHTTPOPENREQUEST: usize = 12;
const H_WINHTTPOPEN: usize = 13;
const H_WINHTTPSETOPTION: usize = 14;
const H_WINHTTPQUERYOPTION: usize = 15;
const H_WINHTTPSENDREQUEST: usize = 16;
const H_WINHTTPSETCALLBACK: usize = 17;
const H_WINHTTPRECEIVE: usize = 18;
const H_WINHTTPQUERYAVAIL: usize = 19;
const H_WINHTTPREADDATA: usize = 20;
const H_WINHTTPQUERYHEADERS: usize = 21;
const H_WINHTTPWSUPGRADE: usize = 22;
const H_BIND: usize = 23;
const H_LISTEN: usize = 24;
const H_ACCEPT: usize = 25;
const H_WSAACCEPT: usize = 26;
const H_CLOSESOCKET: usize = 27;
const H_WSAIOCTL: usize = 28;
const H_SETSOCKOPT: usize = 29;
static ORIG: [AtomicUsize; 30] = [const { AtomicUsize::new(0) }; 30];
static GAME_CB: Mutex<Vec<(usize, usize, u32)>> = Mutex::new(Vec::new());
static PARENT: Mutex<Vec<(usize, usize)>> = Mutex::new(Vec::new());

fn forget_handle(h: usize) {
    if let Ok(mut g) = GAME_CB.lock() {
        g.retain(|(k, _, _)| *k != h);
    }
    if let Ok(mut g) = PARENT.lock() {
        g.retain(|(c, _)| *c != h);
    }
}

fn set_parent(child: usize, parent: usize) {
    if child == 0 { return; }
    if let Ok(mut g) = GAME_CB.lock() {
        g.retain(|(k, _, _)| *k != child);
    }
    if let Ok(mut g) = PARENT.lock() {
        g.retain(|(c, _)| *c != child);
        g.push((child, parent));
        if g.len() > 8192 { g.remove(0); }
    }
    if let Some((cb, fl)) = find_cb(parent) {
        if let Ok(mut g) = GAME_CB.lock() {
            g.push((child, cb, fl));
            if g.len() > 4096 { g.remove(0); }
        }
    }
}

thread_local! {
    static CREATING_UNDER: core::cell::Cell<usize> = const { core::cell::Cell::new(0) };
}

fn adopt_creating(h: usize) -> bool {
    let parent = CREATING_UNDER.with(|c| c.get());
    if parent == 0 || parent == h {
        return false;
    }
    set_parent(h, parent);
    true
}

fn find_cb(h: usize) -> Option<(usize, u32)> {
    let cbs = GAME_CB.lock().ok()?;
    let par = PARENT.lock().ok()?;
    let mut cur = h;
    for _ in 0..4 {
        if let Some(&(_, c, f)) = cbs.iter().rev().find(|(k, _, _)| *k == cur) { return Some((c, f)); }
        match par.iter().rev().find(|(c, _)| *c == cur) { Some(&(_, p)) => cur = p, None => return None }
    }
    None
}
static TRACE_N: AtomicUsize = AtomicUsize::new(0);

fn trace(msg: &str) {
    if TRACE_N.fetch_add(1, Ordering::Relaxed) < 4000 {
        crate::record_raw("net_guard", "winhttp", msg);
    }
}
static LOCAL_PORT: AtomicUsize = AtomicUsize::new(0);
static BUILTIN_PORT: AtomicUsize = AtomicUsize::new(0);

macro_rules! orig {
    ($idx:expr, fn($($at:ty),*) -> $rt:ty) => {{
        let f: unsafe extern "system" fn($($at),*) -> $rt = core::mem::transmute(ORIG[$idx].load(Ordering::Relaxed));
        f
    }};
}

unsafe fn cstr(p: *const u8) -> String {
    let mut n = 0;
    while n < 1024 && *p.add(n) != 0 {
        n += 1;
    }
    String::from_utf8_lossy(core::slice::from_raw_parts(p, n)).into_owned()
}
unsafe fn wstr(p: *const u16) -> String {
    let mut n = 0;
    while n < 1024 && *p.add(n) != 0 {
        n += 1;
    }
    String::from_utf16_lossy(core::slice::from_raw_parts(p, n))
}

fn local_names() -> &'static Vec<String> {
    static N: OnceLock<Vec<String>> = OnceLock::new();
    N.get_or_init(|| {
        let mut v = Vec::new();
        for kind in [0u32, 1, 3] { // NetBIOS, DnsHostname, DnsFullyQualified
            let mut buf = [0u8; 256];
            let mut n = buf.len() as u32;
            if unsafe { GetComputerNameExA(kind, buf.as_mut_ptr(), &mut n) } != 0 && n > 0 {
                v.push(String::from_utf8_lossy(&buf[..n as usize]).to_ascii_lowercase());
            }
        }
        v
    })
}

/// Should this requested name be left alone (not a remote host)?
fn passthrough(name: &str) -> bool {
    let n = name.trim().to_ascii_lowercase();
    n.is_empty() || n == "localhost" || n == "127.0.0.1" || n == "::1" || n == "0.0.0.0" || n == "::"
        || local_names().iter().any(|l| *l == n)
        || target().map_or(false, |t| t.name.eq_ignore_ascii_case(&n))
}

// Allowed destination addresses (IPv4 stored v4-mapped).
static ALLOWED: Mutex<Vec<[u8; 16]>> = Mutex::new(Vec::new());
static LAST_REFRESH: AtomicUsize = AtomicUsize::new(0);

fn v4(a: [u8; 4]) -> [u8; 16] {
    let mut o = [0u8; 16];
    o[10] = 0xff;
    o[11] = 0xff;
    o[12..].copy_from_slice(&a);
    o
}

unsafe fn sockaddr_ip(sa: *const u8, len: i32) -> Option<[u8; 16]> {
    if sa.is_null() || len < 8 {
        return None;
    }
    match core::ptr::read_unaligned(sa as *const u16) {
        2 if len >= 8 => Some(v4([*sa.add(4), *sa.add(5), *sa.add(6), *sa.add(7)])),
        23 if len >= 24 => {
            let mut o = [0u8; 16];
            core::ptr::copy_nonoverlapping(sa.add(8), o.as_mut_ptr(), 16);
            Some(o)
        }
        _ => None,
    }
}

static LAN_PEERS: Mutex<Vec<[u8; 16]>> = Mutex::new(Vec::new());
static LAN_DISCOVERY_PORT: AtomicUsize = AtomicUsize::new(0);

pub fn is_private_v4(a: [u8; 4]) -> bool {
    a[0] == 10 || a[0] == 127 || (a[0] == 172 && (16..=31).contains(&a[1])) || (a[0] == 192 && a[1] == 168) || (a[0] == 169 && a[1] == 254)
}

pub fn allow_lan_peer(a: [u8; 4]) -> bool {
    if !is_private_v4(a) {
        crate::record_raw("net_guard", "lan", &format!("refused LAN peer {}.{}.{}.{}: not a private, link-local or loopback address", a[0], a[1], a[2], a[3]));
        return false;
    }
    let ip = v4(a);
    let mut g = LAN_PEERS.lock().unwrap_or_else(|e| e.into_inner());
    if !g.contains(&ip) {
        g.push(ip);
        crate::record_raw("net_guard", "lan", &format!("LAN peer {}.{}.{}.{} allowed", a[0], a[1], a[2], a[3]));
    }
    true
}

pub fn allow_lan_broadcast(port: u16) {
    LAN_DISCOVERY_PORT.store(port as usize, Ordering::Relaxed);
}

pub fn lan_dest_ok(ip: &[u8; 16], port: u16) -> bool {
    if LAN_PEERS.lock().unwrap_or_else(|e| e.into_inner()).contains(ip) {
        return true;
    }
    let d = LAN_DISCOVERY_PORT.load(Ordering::Relaxed);
    if d == 0 || port as usize != d || ip[..10] != [0u8; 10] || ip[10] != 0xff || ip[11] != 0xff {
        return false;
    }
    let a = [ip[12], ip[13], ip[14], ip[15]];
    a == [255; 4] || (is_private_v4(a) && a[3] == 255)
}

unsafe fn sockaddr_port(sa: *const u8, len: i32) -> Option<u16> {
    if sa.is_null() || len < 4 {
        return None;
    }
    Some(u16::from_be_bytes([*sa.add(2), *sa.add(3)]))
}

fn is_loopback(ip: &[u8; 16]) -> bool {
    let mut l6 = [0u8; 16];
    l6[15] = 1;
    *ip == l6 || (ip[..10] == [0u8; 10] && ip[10] == 0xff && ip[11] == 0xff && ip[12] == 127)
}

/// Resolve the redirect host with the original getaddrinfo and remember its addresses.
unsafe fn refresh_allowed() {
    let Some(t) = target() else { return };
    let f = orig!(H_GETADDRINFO, fn(*const u8, *const u8, *const c_void, *mut *mut u8) -> i32);
    // Winsock may not be initialised yet when the policy is installed.
    static WSA: std::sync::Once = std::sync::Once::new();
    WSA.call_once(|| {
        let p = resolve_export(b"ws2_32.dll\0", b"WSAStartup\0");
        if !p.is_null() {
            let startup: unsafe extern "system" fn(u16, *mut u8) -> i32 = core::mem::transmute(p);
            let mut data = [0u8; 512];
            startup(0x0202, data.as_mut_ptr());
        }
    });
    let mut res: *mut u8 = core::ptr::null_mut();
    let rc = f(t.ansi.as_ptr(), core::ptr::null(), core::ptr::null(), &mut res);
    LAST_REFRESH.store(GetTickCount64() as usize, Ordering::Relaxed);
    if rc != 0 || res.is_null() {
        crate::record_raw("net_guard", "resolve", &format!("'{}' did not resolve (error {rc}) - outbound sockets stay refused", t.name));
        return;
    }
    let mut found = Vec::new();
    let mut p = res;
    while !p.is_null() {
        // addrinfo: ai_addrlen @16 (size_t), ai_addr @32, ai_next @40
        let alen = core::ptr::read_unaligned(p.add(16) as *const usize) as i32;
        let addr = core::ptr::read_unaligned(p.add(32) as *const *const u8);
        if let Some(ip) = sockaddr_ip(addr, alen) {
            if !found.contains(&ip) {
                found.push(ip);
            }
        }
        p = core::ptr::read_unaligned(p.add(40) as *const *mut u8);
    }
    let free = resolve_export(b"ws2_32.dll\0", b"freeaddrinfo\0");
    if !free.is_null() {
        let free: unsafe extern "system" fn(*mut u8) = core::mem::transmute(free);
        free(res);
    }
    let mut g = ALLOWED.lock().unwrap();
    let before = g.len();
    for ip in found {
        if !g.contains(&ip) {
            g.push(ip);
        }
    }
    if g.len() != before {
        crate::record_raw("net_guard", "resolve", &format!("'{}' -> {} address(es) allowed for outbound sockets", t.name, g.len()));
    }
}

/// May the title send to this socket address?
unsafe fn dest_allowed(sa: *const u8, len: i32) -> bool {
    if sa.is_null() {
        return true; // connected socket / no explicit destination
    }
    let Some(ip) = sockaddr_ip(sa, len) else {
        return false; // unknown address family
    };
    if is_loopback(&ip) || ALLOWED.lock().unwrap().contains(&ip) {
        return true;
    }
    if sockaddr_port(sa, len).map_or(false, |p| lan_dest_ok(&ip, p)) {
        return true;
    }
    // The redirect host's addresses may have changed: refresh at most every 10 s.
    let now = GetTickCount64() as usize;
    if now.wrapping_sub(LAST_REFRESH.load(Ordering::Relaxed)) > 10_000 {
        refresh_allowed();
        return ALLOWED.lock().unwrap().contains(&ip);
    }
    false
}

fn deny_dest(api: &str, sa: *const u8, len: i32) {
    let ip = unsafe { sockaddr_ip(sa, len) };
    let s = match ip {
        Some(ip) if ip[..10] == [0u8; 10] && ip[10] == 0xff => format!("{}.{}.{}.{}", ip[12], ip[13], ip[14], ip[15]),
        Some(ip) => ip.chunks(2).map(|c| format!("{:02x}{:02x}", c[0], c[1])).collect::<Vec<_>>().join(":"),
        None => "unknown-family".into(),
    };
    note(&format!("{api} to {s} (not the redirect host)"));
    unsafe { SetLastError(WSAENETUNREACH) };
}

unsafe extern "system" fn h_getaddrinfo(node: *const u8, service: *const u8, hints: *const c_void, res: *mut *mut u8) -> i32 {
    let f = orig!(H_GETADDRINFO, fn(*const u8, *const u8, *const c_void, *mut *mut u8) -> i32);
    if node.is_null() || passthrough(&cstr(node)) {
        return f(node, service, hints, res);
    }
    note_redirect("getaddrinfo", &cstr(node));
    f(target().unwrap().ansi.as_ptr(), service, hints, res)
}

unsafe extern "system" fn h_getaddrinfow(node: *const u16, service: *const u16, hints: *const c_void, res: *mut *mut u8) -> i32 {
    let f = orig!(H_GETADDRINFOW, fn(*const u16, *const u16, *const c_void, *mut *mut u8) -> i32);
    if node.is_null() || passthrough(&wstr(node)) {
        return f(node, service, hints, res);
    }
    note_redirect("GetAddrInfoW", &wstr(node));
    f(target().unwrap().wide.as_ptr(), service, hints, res)
}

type Ex<T> = unsafe extern "system" fn(*const T, *const T, u32, *const c_void, *const c_void, *mut *mut u8, *const c_void, *mut c_void, *const c_void, *mut *mut c_void) -> i32;

unsafe extern "system" fn h_getaddrinfoexw(
    name: *const u16, service: *const u16, ns: u32, nsp: *const c_void, hints: *const c_void, res: *mut *mut u8,
    timeout: *const c_void, ov: *mut c_void, routine: *const c_void, handle: *mut *mut c_void,
) -> i32 {
    let f: Ex<u16> = core::mem::transmute(ORIG[H_GETADDRINFOEXW].load(Ordering::Relaxed));
    if name.is_null() || passthrough(&wstr(name)) {
        return f(name, service, ns, nsp, hints, res, timeout, ov, routine, handle);
    }
    note_redirect("GetAddrInfoExW", &wstr(name));
    f(target().unwrap().wide.as_ptr(), service, ns, nsp, hints, res, timeout, ov, routine, handle)
}

unsafe extern "system" fn h_getaddrinfoexa(
    name: *const u8, service: *const u8, ns: u32, nsp: *const c_void, hints: *const c_void, res: *mut *mut u8,
    timeout: *const c_void, ov: *mut c_void, routine: *const c_void, handle: *mut *mut c_void,
) -> i32 {
    let f: Ex<u8> = core::mem::transmute(ORIG[H_GETADDRINFOEXA].load(Ordering::Relaxed));
    if name.is_null() || passthrough(&cstr(name)) {
        return f(name, service, ns, nsp, hints, res, timeout, ov, routine, handle);
    }
    note_redirect("GetAddrInfoExA", &cstr(name));
    f(target().unwrap().ansi.as_ptr(), service, ns, nsp, hints, res, timeout, ov, routine, handle)
}

unsafe extern "system" fn h_gethostbyname(name: *const u8) -> *mut c_void {
    let f = orig!(H_GETHOSTBYNAME, fn(*const u8) -> *mut c_void);
    if name.is_null() || passthrough(&cstr(name)) {
        return f(name);
    }
    note_redirect("gethostbyname", &cstr(name));
    f(target().unwrap().ansi.as_ptr())
}

unsafe extern "system" fn h_connect(s: usize, name: *const u8, len: i32) -> i32 {
    if !dest_allowed(name, len) {
        deny_dest("connect", name, len);
        return -1;
    }
    let r = orig!(H_CONNECT, fn(usize, *const u8, i32) -> i32)(s, name, len);
    note_socket(s, r, name, len);
    r
}

unsafe fn note_socket(s: usize, r: i32, remote: *const u8, len: i32) {
    let e = GetLastError();
    if r >= 0 {
        crate::netview::sockets::report_socket(s, remote, len);
    } else if e == WSAEWOULDBLOCK {
        crate::netview::sockets::report_event(s, crate::netview::sockets::Event::Connecting, remote, len);
    }
    SetLastError(e);
}

unsafe fn note_event(s: usize, event: crate::netview::sockets::Event, remote: *const u8, len: i32) {
    let e = GetLastError();
    crate::netview::sockets::report_event(s, event, remote, len);
    SetLastError(e);
}

unsafe extern "system" fn h_bind(s: usize, name: *const u8, len: i32) -> i32 {
    let r = orig!(H_BIND, fn(usize, *const u8, i32) -> i32)(s, name, len);
    if r == 0 {
        note_event(s, crate::netview::sockets::Event::Bound, core::ptr::null(), 0);
    }
    r
}

unsafe extern "system" fn h_listen(s: usize, backlog: i32) -> i32 {
    let r = orig!(H_LISTEN, fn(usize, i32) -> i32)(s, backlog);
    if r == 0 {
        note_event(s, crate::netview::sockets::Event::Listening, core::ptr::null(), 0);
    }
    r
}

unsafe fn accepted_len(len: *const i32) -> i32 {
    if len.is_null() { 0 } else { *len }
}

unsafe extern "system" fn h_accept(s: usize, addr: *mut u8, len: *mut i32) -> usize {
    let r = orig!(H_ACCEPT, fn(usize, *mut u8, *mut i32) -> usize)(s, addr, len);
    if r != usize::MAX {
        note_event(r, crate::netview::sockets::Event::Accepted, addr, accepted_len(len));
    }
    r
}

unsafe extern "system" fn h_wsaaccept(s: usize, addr: *mut u8, len: *mut i32, cond: *const c_void, data: usize) -> usize {
    let r = orig!(H_WSAACCEPT, fn(usize, *mut u8, *mut i32, *const c_void, usize) -> usize)(s, addr, len, cond, data);
    if r != usize::MAX {
        note_event(r, crate::netview::sockets::Event::Accepted, addr, accepted_len(len));
    }
    r
}

unsafe extern "system" fn h_closesocket(s: usize) -> i32 {
    ioctl::socket_closing(s);
    crate::netview::sockets::forget(s as u64);
    PENDING_ACCEPTS.lock().unwrap_or_else(|e| e.into_inner()).retain(|(_, a)| *a != s);
    orig!(H_CLOSESOCKET, fn(usize) -> i32)(s)
}

const WSAEWOULDBLOCK: u32 = 10035;
const ERROR_IO_PENDING: u32 = 997;
const SIO_GET_EXTENSION_FUNCTION_POINTER: u32 = 0xC800_0006;
const SO_UPDATE_ACCEPT_CONTEXT: i32 = 0x700B;
const SO_UPDATE_CONNECT_CONTEXT: i32 = 0x7010;
const WSAID_ACCEPTEX: [u8; 16] = [0xf1, 0x7d, 0x36, 0xb5, 0xac, 0xcb, 0xcf, 0x11, 0x95, 0xca, 0x00, 0x80, 0x5f, 0x48, 0xa1, 0x92];
const WSAID_GETACCEPTEXSOCKADDRS: [u8; 16] = [0xf2, 0x7d, 0x36, 0xb5, 0xac, 0xcb, 0xcf, 0x11, 0x95, 0xca, 0x00, 0x80, 0x5f, 0x48, 0xa1, 0x92];
const WSAID_CONNECTEX: [u8; 16] = [0xb9, 0x07, 0xa2, 0x25, 0xf3, 0xdd, 0x60, 0x46, 0x8e, 0xe9, 0x76, 0xe5, 0x8c, 0x74, 0x06, 0x3e];

static REAL_ACCEPTEX: AtomicUsize = AtomicUsize::new(0);
static REAL_GETACCEPTEXSOCKADDRS: AtomicUsize = AtomicUsize::new(0);
static REAL_CONNECTEX: AtomicUsize = AtomicUsize::new(0);
static UNWRAPPED_EXTENSIONS: AtomicUsize = AtomicUsize::new(0);
static PENDING_ACCEPTS: Mutex<Vec<(usize, usize)>> = Mutex::new(Vec::new());

type AcceptExFn = unsafe extern "system" fn(usize, usize, *mut u8, u32, u32, u32, *mut u32, *mut c_void) -> i32;
type GetAcceptExSockaddrsFn = unsafe extern "system" fn(*mut u8, u32, u32, u32, *mut *mut u8, *mut i32, *mut *mut u8, *mut i32);
type ConnectExFn = unsafe extern "system" fn(usize, *const u8, i32, *const c_void, u32, *mut u32, *mut c_void) -> i32;

unsafe extern "system" fn w_acceptex(listen: usize, accept: usize, buf: *mut u8, recv: u32, local_len: u32, remote_len: u32, got: *mut u32, ov: *mut c_void) -> i32 {
    let f: AcceptExFn = core::mem::transmute(REAL_ACCEPTEX.load(Ordering::SeqCst));
    {
        let mut g = PENDING_ACCEPTS.lock().unwrap_or_else(|e| e.into_inner());
        g.retain(|(b, a)| *b != buf as usize && *a != accept);
        g.push((buf as usize, accept));
    }
    let r = f(listen, accept, buf, recv, local_len, remote_len, got, ov);
    let e = GetLastError();
    if r == 0 && e != ERROR_IO_PENDING {
        PENDING_ACCEPTS.lock().unwrap_or_else(|e| e.into_inner()).retain(|(b, _)| *b != buf as usize);
    }
    SetLastError(e);
    r
}

unsafe extern "system" fn w_getacceptexsockaddrs(buf: *mut u8, recv: u32, local_len: u32, remote_len: u32, local: *mut *mut u8, ll: *mut i32, remote: *mut *mut u8, rl: *mut i32) {
    let f: GetAcceptExSockaddrsFn = core::mem::transmute(REAL_GETACCEPTEXSOCKADDRS.load(Ordering::SeqCst));
    f(buf, recv, local_len, remote_len, local, ll, remote, rl);
    let e = GetLastError();
    let accept = {
        let mut g = PENDING_ACCEPTS.lock().unwrap_or_else(|e| e.into_inner());
        let found = g.iter().find(|(b, _)| *b == buf as usize).map(|(_, a)| *a);
        g.retain(|(b, _)| *b != buf as usize);
        found
    };
    if let Some(a) = accept {
        if !local.is_null() && !remote.is_null() && !ll.is_null() && !rl.is_null() {
            crate::netview::sockets::report_accepted(a, *local, *ll, *remote, *rl);
        }
    }
    SetLastError(e);
}

unsafe extern "system" fn w_connectex(s: usize, name: *const u8, len: i32, send: *const c_void, send_len: u32, sent: *mut u32, ov: *mut c_void) -> i32 {
    if !dest_allowed(name, len) {
        deny_dest("ConnectEx", name, len);
        return 0;
    }
    let f: ConnectExFn = core::mem::transmute(REAL_CONNECTEX.load(Ordering::SeqCst));
    let r = f(s, name, len, send, send_len, sent, ov);
    let e = GetLastError();
    if r != 0 {
        crate::netview::sockets::report_socket(s, name, len);
    } else if e == ERROR_IO_PENDING {
        crate::netview::sockets::report_event(s, crate::netview::sockets::Event::Connecting, name, len);
    }
    SetLastError(e);
    r
}

fn wrap_extension(guid: &[u8; 16], real: usize) -> usize {
    let (slot, wrapper) = match *guid {
        WSAID_ACCEPTEX => (&REAL_ACCEPTEX, w_acceptex as *const () as usize),
        WSAID_GETACCEPTEXSOCKADDRS => (&REAL_GETACCEPTEXSOCKADDRS, w_getacceptexsockaddrs as *const () as usize),
        WSAID_CONNECTEX => (&REAL_CONNECTEX, w_connectex as *const () as usize),
        _ => return real,
    };
    if real == 0 || real == wrapper {
        return real;
    }
    match slot.compare_exchange(0, real, Ordering::SeqCst, Ordering::SeqCst) {
        Ok(_) => wrapper,
        Err(known) if known == real => wrapper,
        Err(_) => {
            if UNWRAPPED_EXTENSIONS.fetch_add(1, Ordering::SeqCst) == 0 {
                note("a second provider returned its own AcceptEx/ConnectEx/GetAcceptExSockaddrs; its sockets are not reported");
            }
            real
        }
    }
}

unsafe extern "system" fn h_wsaioctl(s: usize, code: u32, input: *const c_void, in_len: u32, out: *mut c_void, out_len: u32, ret: *mut u32, ov: *mut c_void, cr: *mut c_void) -> i32 {
    if let Some(r) = ioctl::handle_ioctl(s, code, input, in_len, out, out_len, ret, ov, cr) {
        return r;
    }
    let r = orig!(H_WSAIOCTL, fn(usize, u32, *const c_void, u32, *mut c_void, u32, *mut u32, *mut c_void, *mut c_void) -> i32)(s, code, input, in_len, out, out_len, ret, ov, cr);
    if r == 0 && code == SIO_GET_EXTENSION_FUNCTION_POINTER && !input.is_null() && in_len as usize >= 16 && !out.is_null() && out_len as usize >= core::mem::size_of::<usize>() {
        let guid = core::ptr::read_unaligned(input as *const [u8; 16]);
        let real = core::ptr::read_unaligned(out as *const usize);
        core::ptr::write_unaligned(out as *mut usize, wrap_extension(&guid, real));
    }
    if r == 0 {
        ext::guard_extension_out(code, input as *const u8, in_len, out as *mut u8, out_len);
    }
    r
}

pub(crate) fn wsaioctl_shared() -> bool {
    ORIG[H_WSAIOCTL].load(Ordering::SeqCst) != 0
}

unsafe extern "system" fn h_setsockopt(s: usize, level: i32, name: i32, val: *const u8, len: i32) -> i32 {
    let r = orig!(H_SETSOCKOPT, fn(usize, i32, i32, *const u8, i32) -> i32)(s, level, name, val, len);
    if r == 0 && level == 0xffff {
        match name {
            SO_UPDATE_ACCEPT_CONTEXT => {
                PENDING_ACCEPTS.lock().unwrap_or_else(|e| e.into_inner()).retain(|(_, a)| *a != s);
                note_event(s, crate::netview::sockets::Event::Accepted, core::ptr::null(), 0);
            }
            SO_UPDATE_CONNECT_CONTEXT => note_event(s, crate::netview::sockets::Event::Connected, core::ptr::null(), 0),
            _ => {}
        }
    }
    r
}

const SOCKET_TABLE_HOOKS: usize = 7;

fn install_socket_table_hooks() -> (usize, Vec<String>) {
    let ws = b"ws2_32.dll\0";
    let table: [(&[u8], usize, usize); SOCKET_TABLE_HOOKS] = [
        (b"bind\0", H_BIND, h_bind as *const () as usize),
        (b"listen\0", H_LISTEN, h_listen as *const () as usize),
        (b"accept\0", H_ACCEPT, h_accept as *const () as usize),
        (b"WSAAccept\0", H_WSAACCEPT, h_wsaaccept as *const () as usize),
        (b"closesocket\0", H_CLOSESOCKET, h_closesocket as *const () as usize),
        (b"WSAIoctl\0", H_WSAIOCTL, h_wsaioctl as *const () as usize),
        (b"setsockopt\0", H_SETSOCKOPT, h_setsockopt as *const () as usize),
    ];
    let (mut hooked, mut missing) = (0, Vec::new());
    for (f, idx, hook) in table {
        let name = String::from_utf8_lossy(&f[..f.len() - 1]).into_owned();
        if ORIG[idx].load(Ordering::SeqCst) != 0 {
            hooked += 1;
            continue;
        }
        let p = unsafe { resolve_export(ws, f) };
        if p.is_null() {
            missing.push(name);
            continue;
        }
        match unsafe { make_trampoline(p) } {
            Some(t) => {
                ORIG[idx].store(t, Ordering::SeqCst);
                if unsafe { write_jump(p, hook) } { hooked += 1 } else { missing.push(name) }
            }
            None => missing.push(name),
        }
    }
    (hooked, missing)
}

fn socket_table_note() -> String {
    let (mut hooked, mut missing) = install_socket_table_hooks();
    let (ih, im) = ioctl::install();
    hooked += ih;
    missing.extend(im);
    format!("; socket table: {hooked}/{} hooked{}", SOCKET_TABLE_HOOKS + ioctl::HOOKS, if missing.is_empty() { String::new() } else { format!(" (not patched: {})", missing.join(", ")) })
}

unsafe extern "system" fn h_wsaconnect(s: usize, name: *const u8, len: i32, a: *mut c_void, b: *mut c_void, c: *mut c_void, d: *mut c_void) -> i32 {
    if !dest_allowed(name, len) {
        deny_dest("WSAConnect", name, len);
        return -1;
    }
    let r = orig!(H_WSACONNECT, fn(usize, *const u8, i32, *mut c_void, *mut c_void, *mut c_void, *mut c_void) -> i32)(s, name, len, a, b, c, d);
    note_socket(s, r, name, len);
    r
}

type ByName<T> = unsafe extern "system" fn(usize, *const T, *const T, *mut u32, *mut c_void, *mut u32, *mut c_void, *const c_void, *mut c_void) -> i32;

unsafe extern "system" fn h_connectbynamew(
    s: usize, node: *const u16, service: *const u16, ll: *mut u32, la: *mut c_void, rl: *mut u32, ra: *mut c_void, timeout: *const c_void, reserved: *mut c_void,
) -> i32 {
    let f: ByName<u16> = core::mem::transmute(ORIG[H_CONNECTBYNAMEW].load(Ordering::Relaxed));
    if node.is_null() || passthrough(&wstr(node)) {
        return f(s, node, service, ll, la, rl, ra, timeout, reserved);
    }
    note_redirect("WSAConnectByNameW", &wstr(node));
    f(s, target().unwrap().wide.as_ptr(), service, ll, la, rl, ra, timeout, reserved)
}

unsafe extern "system" fn h_connectbynamea(
    s: usize, node: *const u8, service: *const u8, ll: *mut u32, la: *mut c_void, rl: *mut u32, ra: *mut c_void, timeout: *const c_void, reserved: *mut c_void,
) -> i32 {
    let f: ByName<u8> = core::mem::transmute(ORIG[H_CONNECTBYNAMEA].load(Ordering::Relaxed));
    if node.is_null() || passthrough(&cstr(node)) {
        return f(s, node, service, ll, la, rl, ra, timeout, reserved);
    }
    note_redirect("WSAConnectByNameA", &cstr(node));
    f(s, target().unwrap().ansi.as_ptr(), service, ll, la, rl, ra, timeout, reserved)
}

unsafe extern "system" fn h_sendto(s: usize, buf: *const u8, len: i32, flags: i32, to: *const u8, tolen: i32) -> i32 {
    if !dest_allowed(to, tolen) {
        deny_dest("sendto", to, tolen);
        return -1;
    }
    let r = orig!(H_SENDTO, fn(usize, *const u8, i32, i32, *const u8, i32) -> i32)(s, buf, len, flags, to, tolen);
    note_socket(s, r, to, tolen);
    r
}

unsafe extern "system" fn h_wsasendto(
    s: usize, bufs: *const c_void, count: u32, sent: *mut u32, flags: u32, to: *const u8, tolen: i32, ov: *mut c_void, routine: *const c_void,
) -> i32 {
    if !dest_allowed(to, tolen) {
        deny_dest("WSASendTo", to, tolen);
        return -1;
    }
    let r = orig!(H_WSASENDTO, fn(usize, *const c_void, u32, *mut u32, u32, *const u8, i32, *mut c_void, *const c_void) -> i32)(s, bufs, count, sent, flags, to, tolen, ov, routine);
    note_socket(s, r, to, tolen);
    r
}

static LOCAL_PROXY_SESSIONS: Mutex<Vec<usize>> = Mutex::new(Vec::new());

fn proxy_is_local(list: &str) -> bool {
    let entries: Vec<&str> = list.split([';', ' ']).map(str::trim).filter(|e| !e.is_empty()).collect();
    !entries.is_empty() && entries.iter().all(|e| {
        let e = e.rsplit('=').next().unwrap_or(e);
        let e = e.split("://").last().unwrap_or(e);
        let host = if let Some(r) = e.strip_prefix('[') { r.split(']').next().unwrap_or("") } else { e.rsplit_once(':').map_or(e, |(h, _)| h) };
        matches!(host.to_ascii_lowercase().as_str(), "127.0.0.1" | "localhost" | "::1")
    })
}

fn session_uses_local_proxy(session: *mut c_void) -> bool {
    LOCAL_PROXY_SESSIONS.lock().map_or(false, |g| g.contains(&(session as usize)))
}
unsafe extern "system" fn h_winhttpconnect(session: *mut c_void, server: *const u16, port: u16, reserved: u32) -> *mut c_void {
    let f = orig!(H_WINHTTPCONNECT, fn(*mut c_void, *const u16, u16, u32) -> *mut c_void);
    if !server.is_null() && session_uses_local_proxy(session) {
        trace(&format!("WinHttpConnect '{}' port {port} goes through a proxy on this machine; the proxy is the hop", wstr(server)));
        let c = f(session, server, port, reserved);
        set_parent(c as usize, session as usize);
        return c;
    }
    if server.is_null() || passthrough(&wstr(server)) {
        let bp = BUILTIN_PORT.load(Ordering::Relaxed) as u16;
        let loopback = !server.is_null() && matches!(wstr(server).trim().to_ascii_lowercase().as_str(), "127.0.0.1" | "localhost");
        let port = if bp != 0 && loopback && port == 80 {
            trace(&format!("WinHttpConnect loopback:80 -> built-in online services port {bp}"));
            bp
        } else {
            port
        };
        let c = f(session, server, port, reserved);
        set_parent(c as usize, session as usize);
        return c;
    }
    note_redirect(&format!("WinHttpConnect port {port}"), &wstr(server));
    let lp = LOCAL_PORT.load(Ordering::Relaxed) as u16;
    let c = f(session, target().unwrap().wide.as_ptr(), if lp != 0 { lp } else { port }, reserved);
    set_parent(c as usize, session as usize);
    if lp != 0 && !c.is_null() {
        set_orig_host(c as usize, wstr(server));
    }
    c
}

static ORIG_HOST: Mutex<Vec<(usize, String)>> = Mutex::new(Vec::new());

fn set_orig_host(connect: usize, host: String) {
    if let Ok(mut g) = ORIG_HOST.lock() {
        g.retain(|(c, _)| *c != connect);
        g.push((connect, host));
        if g.len() > 4096 { g.remove(0); }
    }
}

pub const INPROC_PORT_ENV: &str = "TOMBOX_NET_INPROC_PORT";

pub fn in_process_port() -> u16 {
    match LOCAL_PORT.load(Ordering::Relaxed) {
        0 => std::env::var(INPROC_PORT_ENV).ok().and_then(|v| v.trim().parse::<u16>().ok()).unwrap_or(0),
        p => p as u16,
    }
}

unsafe fn request_url(req: *mut c_void) -> Option<String> {
    static Q: OnceLock<usize> = OnceLock::new();
    let p = *Q.get_or_init(|| resolve_export(b"winhttp.dll\0", b"WinHttpQueryOption\0") as usize);
    if p == 0 {
        return None;
    }
    let q: unsafe extern "system" fn(*mut c_void, u32, *mut c_void, *mut u32) -> i32 = core::mem::transmute(p);
    let mut buf = vec![0u16; 4096];
    let mut n = (buf.len() * 2) as u32;
    let e = GetLastError();
    let ok = q(req, 34, buf.as_mut_ptr() as *mut c_void, &mut n) != 0;
    SetLastError(e);
    if !ok {
        return None;
    }
    Some(String::from_utf16_lossy(&buf[..(n as usize / 2).min(buf.len())]).trim_end_matches('\0').to_string())
}

pub fn url_is_in_process(url: &str, port: u16) -> bool {
    if port == 0 {
        return false;
    }
    let l = url.to_ascii_lowercase();
    let Some(rest) = l.strip_prefix("http://") else { return false };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let authority = authority.rsplit('@').next().unwrap_or(authority);
    authority == format!("127.0.0.1:{port}")
}

pub fn in_process_request(req: *mut c_void) -> Option<String> {
    if req.is_null() {
        return None;
    }
    if LOCAL_PORT.load(Ordering::Relaxed) != 0 {
        let local = PARENT.lock().ok().and_then(|p| p.iter().rev().find(|(c, _)| *c == req as usize).map(|(_, p)| *p)).and_then(|connect| {
            ORIG_HOST.lock().ok()?.iter().rev().find(|(c, _)| *c == connect).map(|(_, h)| h.clone())
        });
        if local.is_some() {
            return local;
        }
    }
    let url = unsafe { request_url(req) }?;
    if url_is_in_process(&url, in_process_port()) {
        Some(url)
    } else {
        None
    }
}

unsafe fn add_forwarded_host(req: *mut c_void, connect: usize) {
    let host = match ORIG_HOST.lock() {
        Ok(g) => g.iter().rev().find(|(c, _)| *c == connect).map(|(_, h)| h.clone()),
        Err(_) => None,
    };
    let Some(host) = host.filter(|h| h.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-' || b == b':')) else { return };
    static ADD: OnceLock<usize> = OnceLock::new();
    let p = *ADD.get_or_init(|| resolve_export(b"winhttp.dll\0", b"WinHttpAddRequestHeaders\0") as usize);
    if p == 0 { return; }
    let add: unsafe extern "system" fn(*mut c_void, *const u16, u32, u32) -> i32 = core::mem::transmute(p);
    let line: Vec<u16> = format!("X-Forwarded-Host: {host}").encode_utf16().chain(Some(0)).collect();
    add(req, line.as_ptr(), u32::MAX, 0xA000_0000);
}

unsafe extern "system" fn h_winhttpopenrequest(
    connect: *mut c_void, verb: *const u16, object: *const u16, version: *const u16, referrer: *const u16, accept: *const *const u16, flags: u32,
) -> *mut c_void {
    let f = orig!(H_WINHTTPOPENREQUEST, fn(*mut c_void, *const u16, *const u16, *const u16, *const u16, *const *const u16, u32) -> *mut c_void);
    let local = LOCAL_PORT.load(Ordering::Relaxed) != 0;
    let nflags = if local { flags & !0x0080_0000 } else { flags };
    let h = f(connect, verb, object, version, referrer, accept, nflags);
    set_parent(h as usize, connect as usize);
    if local && !h.is_null() {
        add_forwarded_host(h, connect as usize);
    }
    if local {
        let v = if verb.is_null() { "GET".to_string() } else { wstr(verb) };
        let o = if object.is_null() { "/".to_string() } else { wstr(object) };
        let o: String = o.chars().take(160).collect();
        trace(&format!("OpenRequest {v} {o} flags={flags:#x} -> {:#x} err={}", h as usize, if h.is_null() { GetLastError() } else { 0 }));
    }
    h
}

unsafe extern "system" fn h_winhttpsetoption(h: *mut c_void, opt: u32, buf: *mut c_void, len: u32) -> i32 {
    let f = orig!(H_WINHTTPSETOPTION, fn(*mut c_void, u32, *mut c_void, u32) -> i32);
    if opt == 47 && buf.is_null() && len == 0 && in_process_request(h).is_some() {
        return 1;
    }
    let r = f(h, opt, buf, len);
    if r == 0 && LOCAL_PORT.load(Ordering::Relaxed) != 0 {
        let e = GetLastError();
        trace(&format!("SetOption {:#x} opt={opt} len={len} failed err={e}", h as usize));
        SetLastError(e);
    }
    r
}

unsafe extern "system" fn h_winhttpqueryoption(h: *mut c_void, opt: u32, buf: *mut c_void, len: *mut u32) -> i32 {
    let f = orig!(H_WINHTTPQUERYOPTION, fn(*mut c_void, u32, *mut c_void, *mut u32) -> i32);
    let r = f(h, opt, buf, len);
    if r == 0 && LOCAL_PORT.load(Ordering::Relaxed) != 0 {
        let e = GetLastError();
        if e != 122 {
            trace(&format!("QueryOption {:#x} opt={opt} failed err={e}", h as usize));
        }
        SetLastError(e);
    }
    r
}

unsafe extern "system" fn h_winhttpsendrequest(h: *mut c_void, hdr: *const u16, hlen: u32, opt: *mut c_void, olen: u32, total: u32, ctx: usize) -> i32 {
    let f = orig!(H_WINHTTPSENDREQUEST, fn(*mut c_void, *const u16, u32, *mut c_void, u32, u32, usize) -> i32);
    let r = f(h, hdr, hlen, opt, olen, total, ctx);
    if LOCAL_PORT.load(Ordering::Relaxed) != 0 {
        let e = if r == 0 { GetLastError() } else { 0 };
        trace(&format!("SendRequest {:#x} body={olen}/{total} -> {r} err={e}", h as usize));
        if r == 0 { SetLastError(e); }
    }
    r
}

unsafe extern "system" fn h_winhttpreceive(h: *mut c_void, reserved: *mut c_void) -> i32 {
    let f = orig!(H_WINHTTPRECEIVE, fn(*mut c_void, *mut c_void) -> i32);
    let r = f(h, reserved);
    if LOCAL_PORT.load(Ordering::Relaxed) != 0 {
        let e = if r == 0 { GetLastError() } else { 0 };
        trace(&format!("ReceiveResponse {:#x} -> {r} err={e}", h as usize));
        if r == 0 { SetLastError(e); }
    }
    r
}

unsafe extern "system" fn h_winhttpqueryavail(h: *mut c_void, n: *mut u32) -> i32 {
    let f = orig!(H_WINHTTPQUERYAVAIL, fn(*mut c_void, *mut u32) -> i32);
    let r = f(h, n);
    if LOCAL_PORT.load(Ordering::Relaxed) != 0 {
        let e = if r == 0 { GetLastError() } else { 0 };
        let v = if n.is_null() || r == 0 { -1 } else { *n as i64 };
        trace(&format!("QueryDataAvailable {:#x} -> {r} n={v} err={e}", h as usize));
        if r == 0 { SetLastError(e); }
    }
    r
}

unsafe extern "system" fn h_winhttpreaddata(h: *mut c_void, buf: *mut c_void, len: u32, read: *mut u32) -> i32 {
    let f = orig!(H_WINHTTPREADDATA, fn(*mut c_void, *mut c_void, u32, *mut u32) -> i32);
    let r = f(h, buf, len, read);
    if LOCAL_PORT.load(Ordering::Relaxed) != 0 {
        let e = if r == 0 { GetLastError() } else { 0 };
        let v = if read.is_null() || r == 0 { -1 } else { *read as i64 };
        trace(&format!("ReadData {:#x} len={len} -> {r} read={v} err={e}", h as usize));
        if r == 0 { SetLastError(e); }
    }
    r
}

unsafe extern "system" fn h_winhttpqueryheaders(h: *mut c_void, level: u32, name: *const u16, buf: *mut c_void, len: *mut u32, idx: *mut u32) -> i32 {
    let f = orig!(H_WINHTTPQUERYHEADERS, fn(*mut c_void, u32, *const u16, *mut c_void, *mut u32, *mut u32) -> i32);
    let r = f(h, level, name, buf, len, idx);
    if LOCAL_PORT.load(Ordering::Relaxed) != 0 {
        let e = if r == 0 { GetLastError() } else { 0 };
        let val = if r != 0 && !buf.is_null() && level & 0x2000_0000 != 0 { format!(" num={}", *(buf as *const u32)) } else { String::new() };
        trace(&format!("QueryHeaders {:#x} level={level:#x} -> {r}{val} err={e}", h as usize));
        if r == 0 { SetLastError(e); }
    }
    r
}

unsafe extern "system" fn h_winhttpwsupgrade(req: *mut c_void, ctx: usize) -> *mut c_void {
    let f = orig!(H_WINHTTPWSUPGRADE, fn(*mut c_void, usize) -> *mut c_void);
    let prior = CREATING_UNDER.with(|c| c.replace(req as usize));
    let ws = f(req, ctx);
    CREATING_UNDER.with(|c| c.set(prior));
    if !ws.is_null() {
        set_parent(ws as usize, req as usize);
    }
    if LOCAL_PORT.load(Ordering::Relaxed) != 0 {
        let e = if ws.is_null() { GetLastError() } else { 0 };
        trace(&format!("WebSocketCompleteUpgrade {:#x} -> {:#x} err={e}", req as usize, ws as usize));
        if ws.is_null() { SetLastError(e); }
    }
    ws
}

type StatusCb = unsafe extern "system" fn(*mut c_void, usize, u32, *mut c_void, u32);

unsafe extern "system" fn cb_wrap(h: *mut c_void, ctx: usize, status: u32, info: *mut c_void, len: u32) {
    match status {
        0x0020_0000 if !info.is_null() => {
            let res = core::ptr::read_unaligned(info as *const usize);
            let err = core::ptr::read_unaligned((info as *const u8).add(8) as *const u32);
            trace(&format!("callback {:#x} REQUEST_ERROR api={res} err={err}", h as usize));
        }
        0x0001_0000 => {
            let fl = if info.is_null() { 0 } else { core::ptr::read_unaligned(info as *const u32) };
            trace(&format!("callback {:#x} SECURE_FAILURE flags={fl:#x}", h as usize));
        }
        0x0002_0000 => trace(&format!("callback {:#x} HEADERS_AVAILABLE", h as usize)),
        0x0040_0000 => trace(&format!("callback {:#x} SENDREQUEST_COMPLETE", h as usize)),
        _ => {}
    }
    let owner = find_cb(h as usize).or_else(|| if adopt_creating(h as usize) { find_cb(h as usize) } else { None });
    let Some((cb, fl)) = owner else {
        trace(&format!("callback {:#x} status={status:#x} has no known owner - dropped", h as usize));
        return;
    };
    if status & fl != 0 {
        let f: StatusCb = core::mem::transmute(cb);
        f(h, ctx, status, info, len);
    }
    if status == 0x0000_0800 {
        forget_handle(h as usize);
    }
}

unsafe extern "system" fn h_winhttpsetcallback(h: *mut c_void, cb: usize, flags: u32, reserved: usize) -> usize {
    let f = orig!(H_WINHTTPSETCALLBACK, fn(*mut c_void, usize, u32, usize) -> usize);
    if LOCAL_PORT.load(Ordering::Relaxed) == 0 || cb == 0 || cb == usize::MAX {
        return f(h, cb, flags, reserved);
    }
    if let Ok(mut g) = GAME_CB.lock() {
        g.retain(|(k, _, _)| *k != h as usize);
        g.push((h as usize, cb, flags));
        if g.len() > 4096 { g.remove(0); }
    }
    let prev = f(h, cb_wrap as usize, flags | 0x0001_0000 | 0x0020_0000 | 0x0002_0000 | 0x0040_0000 | 0x0000_0800, reserved);
    if prev == cb_wrap as usize { 0 } else { prev }
}

unsafe extern "system" fn h_winhttpopen(agent: *const u16, access: u32, proxy: *const u16, bypass: *const u16, flags: u32) -> *mut c_void {
    let f = orig!(H_WINHTTPOPEN, fn(*const u16, u32, *const u16, *const u16, u32) -> *mut c_void);
    let nflags = if LOCAL_PORT.load(Ordering::Relaxed) != 0 && flags & 0x3000_0000 == 0x3000_0000 { (flags & !0x3000_0000) | 0x1000_0000 } else { flags };
    let h = f(agent, access, proxy, bypass, nflags);
    if !h.is_null() && access == 3 && !proxy.is_null() && proxy_is_local(&wstr(proxy)) {
        if let Ok(mut g) = LOCAL_PROXY_SESSIONS.lock() {
            g.retain(|s| *s != h as usize);
            g.push(h as usize);
            if g.len() > 256 { g.remove(0); }
        }
    }
    if LOCAL_PORT.load(Ordering::Relaxed) != 0 {
        trace(&format!("Open flags={flags:#x} -> {nflags:#x} -> {:#x}", h as usize));
    }
    h
}

// ---- installation -------------------------------------------------------------------------------

fn install_deny() {
    let ws = b"ws2_32.dll\0";
    let wh = b"winhttp.dll\0";
    let table: [(&[u8], &[u8], usize); 14] = [
        (ws, b"getaddrinfo\0", deny_resolve as usize),
        (ws, b"GetAddrInfoW\0", deny_resolve as usize),
        (ws, b"GetAddrInfoExW\0", deny_resolve as usize),
        (ws, b"GetAddrInfoExA\0", deny_resolve as usize),
        (ws, b"gethostbyname\0", deny_null as usize),
        (ws, b"connect\0", deny_sock as usize),
        (ws, b"WSAConnect\0", deny_sock as usize),
        (ws, b"WSAConnectByNameW\0", deny_sock_bool as usize),
        (ws, b"WSAConnectByNameA\0", deny_sock_bool as usize),
        (ws, b"sendto\0", deny_sock as usize),
        (ws, b"WSASendTo\0", deny_sock as usize),
        (wh, b"WinHttpConnect\0", deny_null as usize),
        (wh, b"WinHttpOpenRequest\0", deny_null as usize),
        (wh, b"WinHttpSendRequest\0", deny_false as usize),
    ];
    let mut ok = 0;
    let mut missing = Vec::new();
    for (m, f, stub) in table {
        let p = unsafe { resolve_export(m, f) };
        if !p.is_null() && unsafe { write_jump(p, stub) } {
            ok += 1;
        } else {
            missing.push(String::from_utf8_lossy(&f[..f.len() - 1]).into_owned());
        }
    }
    let sockets = socket_table_note();
    crate::record_raw("net_guard", "installed", &format!(
        "{ok}/14 network entry points blocked in-process{}{sockets}",
        if missing.is_empty() { String::new() } else { format!(" (not patched: {})", missing.join(", ")) }));
    ext::install(false);
}

fn install_redirect(host: &str) {
    let ws = b"ws2_32.dll\0";
    let wh = b"winhttp.dll\0";
    // (module, export, index, hook, deny stub used if the original cannot be trampolined)
    let table: [(&[u8], &[u8], usize, usize, usize); 23] = [
        (ws, b"getaddrinfo\0", H_GETADDRINFO, h_getaddrinfo as usize, deny_resolve as usize),
        (ws, b"GetAddrInfoW\0", H_GETADDRINFOW, h_getaddrinfow as usize, deny_resolve as usize),
        (ws, b"GetAddrInfoExW\0", H_GETADDRINFOEXW, h_getaddrinfoexw as usize, deny_resolve as usize),
        (ws, b"GetAddrInfoExA\0", H_GETADDRINFOEXA, h_getaddrinfoexa as usize, deny_resolve as usize),
        (ws, b"gethostbyname\0", H_GETHOSTBYNAME, h_gethostbyname as usize, deny_null as usize),
        (ws, b"connect\0", H_CONNECT, h_connect as usize, deny_sock as usize),
        (ws, b"WSAConnect\0", H_WSACONNECT, h_wsaconnect as usize, deny_sock as usize),
        (ws, b"WSAConnectByNameW\0", H_CONNECTBYNAMEW, h_connectbynamew as usize, deny_sock_bool as usize),
        (ws, b"WSAConnectByNameA\0", H_CONNECTBYNAMEA, h_connectbynamea as usize, deny_sock_bool as usize),
        (ws, b"sendto\0", H_SENDTO, h_sendto as usize, deny_sock as usize),
        (ws, b"WSASendTo\0", H_WSASENDTO, h_wsasendto as usize, deny_sock as usize),
        (wh, b"WinHttpConnect\0", H_WINHTTPCONNECT, h_winhttpconnect as usize, deny_null as usize),
        (wh, b"WinHttpOpenRequest\0", H_WINHTTPOPENREQUEST, h_winhttpopenrequest as usize, deny_null as usize),
        (wh, b"WinHttpOpen\0", H_WINHTTPOPEN, h_winhttpopen as usize, deny_null as usize),
        (wh, b"WinHttpSetOption\0", H_WINHTTPSETOPTION, h_winhttpsetoption as usize, deny_false as usize),
        (wh, b"WinHttpQueryOption\0", H_WINHTTPQUERYOPTION, h_winhttpqueryoption as usize, deny_false as usize),
        (wh, b"WinHttpSendRequest\0", H_WINHTTPSENDREQUEST, h_winhttpsendrequest as usize, deny_false as usize),
        (wh, b"WinHttpSetStatusCallback\0", H_WINHTTPSETCALLBACK, h_winhttpsetcallback as usize, deny_null as usize),
        (wh, b"WinHttpReceiveResponse\0", H_WINHTTPRECEIVE, h_winhttpreceive as usize, deny_false as usize),
        (wh, b"WinHttpQueryDataAvailable\0", H_WINHTTPQUERYAVAIL, h_winhttpqueryavail as usize, deny_false as usize),
        (wh, b"WinHttpReadData\0", H_WINHTTPREADDATA, h_winhttpreaddata as usize, deny_false as usize),
        (wh, b"WinHttpQueryHeaders\0", H_WINHTTPQUERYHEADERS, h_winhttpqueryheaders as usize, deny_false as usize),
        (wh, b"WinHttpWebSocketCompleteUpgrade\0", H_WINHTTPWSUPGRADE, h_winhttpwsupgrade as usize, deny_null as usize),
    ];
    let (mut hooked, mut denied, mut missing) = (0, Vec::new(), Vec::new());
    for (m, f, idx, hook, deny) in table {
        let name = String::from_utf8_lossy(&f[..f.len() - 1]).into_owned();
        let p = unsafe { resolve_export(m, f) };
        if p.is_null() {
            missing.push(name);
            continue;
        }
        match unsafe { make_trampoline(p) } {
            Some(t) => {
                ORIG[idx].store(t, Ordering::SeqCst);
                if unsafe { write_jump(p, hook) } { hooked += 1 } else { missing.push(name) }
            }
            None => {
                // Cannot call the original safely: fail closed for this entry point.
                if unsafe { write_jump(p, deny) } { denied.push(name) } else { missing.push(name) }
            }
        }
    }
    let sockets = socket_table_note();
    crate::record_raw("net_guard", "installed", &format!(
        "REDIRECT mode: every host -> '{host}'{}; {hooked}/23 entry points hooked{}{}{sockets}",
        match LOCAL_PORT.load(Ordering::Relaxed) { 0 => String::new(), p => format!(" (local backend: WinHTTP -> port {p}, plain HTTP)") },
        if denied.is_empty() { String::new() } else { format!("; fail-closed (no trampoline): {}", denied.join(", ")) },
        if missing.is_empty() { String::new() } else { format!("; NOT PATCHED: {}", missing.join(", ")) }));
    ext::install(true);
    if ORIG[H_GETADDRINFO].load(Ordering::Relaxed) != 0 {
        unsafe { refresh_allowed() };
    }
}

/// Install the policy once per process (the first compat DLL to log wins).
pub fn ensure() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        // Several compat DLLs link this crate; only one may patch the process.
        let marker = format!("{}", unsafe { GetCurrentProcessId() });
        if std::env::var("TOMBOX_NET_GUARD_OWNER").ok().as_deref() == Some(marker.as_str()) {
            let _ = TARGET.set(None);
            return;
        }
        std::env::set_var("TOMBOX_NET_GUARD_OWNER", &marker);

        // Process-wide one-time setup that is not network policy: the local Xbox user.
        crate::gdk_user::install();
        if let Some(locale) = crate::cfg("locale").filter(|v| !v.is_empty()) {
            crate::locale::install(&locale);
        }

        if std::env::var_os("TOMBOX_ALLOW_NETWORK").map(|v| v == "1").unwrap_or(false) {
            let _ = TARGET.set(None);
            crate::record_raw("net_guard", "disabled", "TOMBOX_ALLOW_NETWORK=1 — title network access NOT restricted");
            return;
        }
        // Redirect host: environment first, else a `net_redirect=<host>` line in `tombox.cfg`
        // next to the title executable (so a plain double-click launch gets the same policy).
        let from_cfg = || crate::cfg("net_redirect");
        let builtin = crate::online::requested();
        let host = if builtin {
            match crate::online::start_for_config() {
                Ok(p) => {
                    BUILTIN_PORT.store(p as usize, Ordering::Relaxed);
                    crate::record_raw("net_guard", "online", &format!("built-in online services listening on 127.0.0.1:{p}"));
                    Some("127.0.0.1".to_string())
                }
                Err(e) => {
                    crate::record_raw("net_guard", "online", &format!("built-in online services failed to start ({e}) - falling back to deny-all"));
                    None
                }
            }
        } else {
            std::env::var("TOMBOX_NET_REDIRECT").ok().or_else(from_cfg).map(|h| h.trim().to_string()).filter(|h| !h.is_empty())
        };
        match host {
            Some(h) if h.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-') => {
                let mut ansi = h.clone().into_bytes();
                ansi.push(0);
                let wide: Vec<u16> = h.encode_utf16().chain(Some(0)).collect();
                let _ = TARGET.set(Some(Target { name: h.clone(), ansi, wide }));
                let lp = match BUILTIN_PORT.load(Ordering::Relaxed) {
                    0 => std::env::var("TOMBOX_NET_LOCAL_PORT").ok().or_else(|| crate::cfg("net_local_port"))
                        .and_then(|v| v.trim().parse::<u16>().ok()).unwrap_or(0),
                    p => p as u16,
                };
                if lp != 0 && h == "127.0.0.1" {
                    LOCAL_PORT.store(lp as usize, Ordering::Relaxed);
                    std::env::set_var(INPROC_PORT_ENV, lp.to_string());
                }
                install_redirect(&h);
            }
            Some(h) => {
                let _ = TARGET.set(None);
                crate::record_raw("net_guard", "config", &format!("TOMBOX_NET_REDIRECT='{h}' is not a plain host name - falling back to deny-all"));
                install_deny();
            }
            None => {
                let _ = TARGET.set(None);
                install_deny();
            }
        }
    });
}

#[cfg(test)]
mod callback_owner_tests {
    use super::*;

    static SEEN: Mutex<Vec<(usize, u32, usize)>> = Mutex::new(Vec::new());

    unsafe extern "system" fn http_cb(h: *mut c_void, ctx: usize, status: u32, _: *mut c_void, _: u32) {
        SEEN.lock().unwrap().push((h as usize, status, ctx ^ 0x1));
    }

    unsafe extern "system" fn ws_cb(h: *mut c_void, ctx: usize, status: u32, _: *mut c_void, _: u32) {
        SEEN.lock().unwrap().push((h as usize, status, ctx ^ 0x2));
    }

    fn register(h: usize, cb: usize, flags: u32) {
        GAME_CB.lock().unwrap().push((h, cb, flags));
    }

    #[test]
    fn reused_handle_value_does_not_inherit_a_closed_handles_callback() {
        let (old_req, ws_parent, reused) = (0x7a10_0000usize, 0x7a20_0000usize, 0x7a30_0000usize);
        set_parent(old_req, 0x7a00_0000);
        register(old_req, http_cb as usize, 0xffff_ffff);
        register(reused, http_cb as usize, 0xffff_ffff);
        register(ws_parent, ws_cb as usize, 0x0008_0000);
        set_parent(reused, ws_parent);
        assert_eq!(find_cb(reused), Some((ws_cb as usize, 0x0008_0000)));
        unsafe { cb_wrap(old_req as *mut c_void, 0x10, 0x0000_0800, core::ptr::null_mut(), 0) };
        assert!(find_cb(old_req).is_none());
        let seen = SEEN.lock().unwrap().clone();
        assert!(seen.contains(&(old_req, 0x800, 0x11)), "{seen:x?}");
    }

    #[test]
    fn websocket_keeps_its_callback_after_the_upgrade_request_closes_and_its_value_is_reused() {
        let (session, req, ws, other_parent) = (0x7b00_0000usize, 0x7b10_0000usize, 0x7b20_0000usize, 0x7b30_0000usize);
        set_parent(req, session);
        register(req, ws_cb as usize, 0xffff_ffff);
        set_parent(ws, req);
        unsafe { cb_wrap(req as *mut c_void, 0x30, 0x0000_0800, core::ptr::null_mut(), 0) };
        register(other_parent, http_cb as usize, 0x0000_0800);
        set_parent(req, other_parent);
        register(req, http_cb as usize, 0xffff_ffff);
        assert_eq!(find_cb(ws), Some((ws_cb as usize, 0xffff_ffff)));
        unsafe { cb_wrap(ws as *mut c_void, 0x40, 0x0008_0000, core::ptr::null_mut(), 0) };
        assert!(SEEN.lock().unwrap().contains(&(ws, 0x0008_0000, 0x42)));
        unsafe { cb_wrap(ws as *mut c_void, 0x40, 0x0000_0800, core::ptr::null_mut(), 0) };
        assert!(find_cb(ws).is_none());
    }
}

#[cfg(test)]
mod socket_table_tests {
    use super::*;
    use crate::netview::sockets::{self, MIB_TCP_STATE_ESTAB, MIB_TCP_STATE_LISTEN, MIB_TCP_STATE_RESERVED, TCP_TABLE_OWNER_PID_ALL, UDP_TABLE_OWNER_PID};
    use crate::netview::AF_INET;

    #[link(name = "ws2_32")]
    extern "system" {
        fn socket(af: i32, ty: i32, proto: i32) -> usize;
    }

    fn arm() -> std::sync::MutexGuard<'static, ()> {
        let g = sockets::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _ = std::net::UdpSocket::bind("127.0.0.1:0");
        for (idx, name) in [(H_BIND, &b"bind\0"[..]), (H_LISTEN, b"listen\0"), (H_ACCEPT, b"accept\0"), (H_WSAACCEPT, b"WSAAccept\0"), (H_CLOSESOCKET, b"closesocket\0"), (H_CONNECT, b"connect\0"), (H_WSAIOCTL, b"WSAIoctl\0"), (H_SETSOCKOPT, b"setsockopt\0")] {
            let p = unsafe { resolve_export(b"ws2_32.dll\0", name) };
            assert!(!p.is_null());
            ORIG[idx].store(p as usize, Ordering::SeqCst);
        }
        g
    }

    fn loopback_any() -> [u8; 16] {
        let mut a = [0u8; 16];
        a[0] = 2;
        a[4..8].copy_from_slice(&[127, 0, 0, 1]);
        a
    }

    fn mine(s: usize) -> Option<sockets::Endpoint> {
        let pid = std::process::id();
        sockets::snapshot().into_iter().find(|e| e.socket == s as u64 && e.pid == pid)
    }

    fn rows(t: &[u8], size: usize) -> Vec<&[u8]> {
        let n = u32::from_le_bytes([t[0], t[1], t[2], t[3]]) as usize;
        (0..n).map(|i| &t[4 + i * size..4 + (i + 1) * size]).collect()
    }

    fn tcp_ports() -> Vec<(u16, u32)> {
        let t = sockets::tcp_table(AF_INET, TCP_TABLE_OWNER_PID_ALL, true).unwrap();
        rows(&t, sockets::TCP_ROW_OWNER_PID_SIZE).iter().filter(|r| u32::from_le_bytes([r[0x14], r[0x15], r[0x16], r[0x17]]) == std::process::id()).map(|r| (u16::from_be_bytes([r[8], r[9]]), u32::from_le_bytes([r[0], r[1], r[2], r[3]]))).collect()
    }

    #[test]
    fn bound_listening_accepted_and_closed_tcp_sockets_track_the_table() {
        let _g = arm();
        unsafe {
            let s = socket(2, 1, 6);
            assert_ne!(s, usize::MAX);
            let a = loopback_any();
            assert_eq!(h_bind(s, a.as_ptr(), 16), 0);
            let bound = mine(s).expect("bound socket recorded");
            assert_eq!(bound.state, MIB_TCP_STATE_RESERVED);
            let port = bound.local.port();
            assert_ne!(port, 0);
            assert!(!tcp_ports().iter().any(|(p, _)| *p == port));
            assert_eq!(h_listen(s, 4), 0);
            assert_eq!(mine(s).unwrap().state, MIB_TCP_STATE_LISTEN);
            assert!(tcp_ports().contains(&(port, MIB_TCP_STATE_LISTEN)));
            let client = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
            let mut peer = [0u8; 16];
            let mut plen = 16i32;
            let c = h_accept(s, peer.as_mut_ptr(), &mut plen);
            assert_ne!(c, usize::MAX);
            let acc = mine(c).expect("accepted socket recorded");
            assert_eq!(acc.state, MIB_TCP_STATE_ESTAB);
            assert_eq!(acc.local.port(), port);
            assert_eq!(acc.remote.unwrap().port(), client.local_addr().unwrap().port());
            assert!(tcp_ports().contains(&(port, MIB_TCP_STATE_ESTAB)));
            assert_eq!(h_closesocket(c), 0);
            assert!(mine(c).is_none());
            assert_eq!(h_closesocket(s), 0);
            assert!(mine(s).is_none());
            assert!(!tcp_ports().iter().any(|(p, _)| *p == port));
        }
    }

    #[test]
    fn bound_udp_sockets_show_in_the_udp_table_until_closed() {
        let _g = arm();
        unsafe {
            let s = socket(2, 2, 17);
            assert_ne!(s, usize::MAX);
            let a = loopback_any();
            assert_eq!(h_bind(s, a.as_ptr(), 16), 0);
            let port = mine(s).expect("bound udp recorded").local.port();
            let t = sockets::udp_table(AF_INET, UDP_TABLE_OWNER_PID, true).unwrap();
            assert!(rows(&t, sockets::UDP_ROW_OWNER_PID_SIZE).iter().any(|r| u16::from_be_bytes([r[4], r[5]]) == port));
            assert_eq!(h_closesocket(s), 0);
            assert!(mine(s).is_none());
        }
    }

    #[test]
    fn emulator_owned_listeners_are_reported() {
        let _g = arm();
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        assert!(sockets::report_std(&l));
        let port = l.local_addr().unwrap().port();
        assert!(tcp_ports().contains(&(port, MIB_TCP_STATE_LISTEN)));
        let u = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        assert!(sockets::report_std(&u));
        let up = u.local_addr().unwrap().port();
        let t = sockets::udp_table(AF_INET, UDP_TABLE_OWNER_PID, true).unwrap();
        assert!(rows(&t, sockets::UDP_ROW_OWNER_PID_SIZE).iter().any(|r| u16::from_be_bytes([r[4], r[5]]) == up));
        let raw = std::os::windows::io::AsRawSocket::as_raw_socket(&l);
        drop(l);
        assert!(!sockets::endpoints().iter().any(|e| e.socket == raw && e.pid == std::process::id()));
    }

    #[link(name = "ws2_32")]
    extern "system" {
        fn ioctlsocket(s: usize, cmd: i32, arg: *mut u32) -> i32;
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn CreateEventW(sa: *mut c_void, manual: i32, initial: i32, name: *const u16) -> *mut c_void;
        fn WaitForSingleObject(h: *mut c_void, ms: u32) -> u32;
    }

    const FIONBIO: i32 = 0x8004_667Eu32 as i32;

    fn sin(ip: [u8; 4], port: u16) -> [u8; 16] {
        let mut a = [0u8; 16];
        a[0] = 2;
        a[2..4].copy_from_slice(&port.to_be_bytes());
        a[4..8].copy_from_slice(&ip);
        a
    }

    fn settled(s: usize) -> Option<sockets::Endpoint> {
        let pid = std::process::id();
        sockets::endpoints().into_iter().find(|e| e.socket == s as u64 && e.pid == pid)
    }

    fn wait_for(ms: u64, mut f: impl FnMut() -> bool) -> bool {
        let start = std::time::Instant::now();
        while start.elapsed().as_millis() < ms as u128 {
            if f() {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        f()
    }

    unsafe fn extension(s: usize, guid: &[u8; 16]) -> usize {
        let mut p = 0usize;
        let mut got = 0u32;
        let r = h_wsaioctl(s, SIO_GET_EXTENSION_FUNCTION_POINTER, guid.as_ptr() as *const c_void, 16, &mut p as *mut usize as *mut c_void, 8, &mut got, core::ptr::null_mut(), core::ptr::null_mut());
        assert_eq!(r, 0);
        p
    }

    struct Overlapped {
        raw: Box<[usize; 4]>,
    }

    impl Overlapped {
        unsafe fn new() -> Overlapped {
            let ev = CreateEventW(core::ptr::null_mut(), 1, 0, core::ptr::null());
            assert!(!ev.is_null());
            Overlapped { raw: Box::new([0, 0, 0, ev as usize]) }
        }
        fn ptr(&mut self) -> *mut c_void {
            self.raw.as_mut_ptr() as *mut c_void
        }
        unsafe fn wait(&self) {
            assert_eq!(WaitForSingleObject(self.raw[3] as *mut c_void, 5000), 0);
        }
    }

    #[test]
    fn nonblocking_connects_show_syn_sent_then_established_or_vanish_on_failure() {
        let _g = arm();
        unsafe {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let port = l.local_addr().unwrap().port();
            let s = socket(2, 1, 6);
            let mut on = 1u32;
            assert_eq!(ioctlsocket(s, FIONBIO, &mut on), 0);
            let a = sin([127, 0, 0, 1], port);
            assert_eq!(h_connect(s, a.as_ptr(), 16), -1);
            assert_eq!(GetLastError(), WSAEWOULDBLOCK);
            let row = mine(s).expect("in-progress connect recorded");
            assert_eq!(row.state, sockets::MIB_TCP_STATE_SYN_SENT);
            assert_eq!(row.remote.unwrap().port(), port);
            assert!(wait_for(5000, || settled(s).map(|e| e.state) == Some(MIB_TCP_STATE_ESTAB)));
            let local = settled(s).unwrap().local.port();
            assert!(tcp_ports().contains(&(local, MIB_TCP_STATE_ESTAB)));
            assert_eq!(h_closesocket(s), 0);
            let dead = {
                let t = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
                t.local_addr().unwrap().port()
            };
            let f = socket(2, 1, 6);
            assert_eq!(ioctlsocket(f, FIONBIO, &mut on), 0);
            let b = sin([127, 0, 0, 1], dead);
            assert_eq!(h_connect(f, b.as_ptr(), 16), -1);
            assert_eq!(GetLastError(), WSAEWOULDBLOCK);
            assert_eq!(mine(f).unwrap().state, sockets::MIB_TCP_STATE_SYN_SENT);
            assert!(wait_for(10000, || settled(f).is_none()));
            assert!(mine(f).is_none());
            assert_eq!(h_closesocket(f), 0);
        }
    }

    #[test]
    fn connectex_pointers_are_wrapped_and_report_the_connection() {
        let _g = arm();
        unsafe {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let port = l.local_addr().unwrap().port();
            let s = socket(2, 1, 6);
            assert_eq!(h_bind(s, loopback_any().as_ptr(), 16), 0);
            let outer = extension(s, &WSAID_CONNECTEX);
            assert_ne!(outer, w_connectex as *const () as usize);
            let p = ext::guarded_inner(outer).expect("ConnectEx pointer guarded by ext");
            assert_eq!(p, w_connectex as *const () as usize);
            let transmit = [0xf0, 0x7d, 0x36, 0xb5, 0xac, 0xcb, 0xcf, 0x11, 0x95, 0xca, 0x00, 0x80, 0x5f, 0x48, 0xa1, 0x92];
            let mut real = 0usize;
            let mut got = 0u32;
            assert_eq!(orig!(H_WSAIOCTL, fn(usize, u32, *const c_void, u32, *mut c_void, u32, *mut u32, *mut c_void, *mut c_void) -> i32)(s, SIO_GET_EXTENSION_FUNCTION_POINTER, transmit.as_ptr() as *const c_void, 16, &mut real as *mut usize as *mut c_void, 8, &mut got, core::ptr::null_mut(), core::ptr::null_mut()), 0);
            assert_eq!(extension(s, &transmit), real);
            let connect_ex: ConnectExFn = core::mem::transmute(p);
            let mut ov = Overlapped::new();
            let a = sin([127, 0, 0, 1], port);
            let r = connect_ex(s, a.as_ptr(), 16, core::ptr::null(), 0, core::ptr::null_mut(), ov.ptr());
            if r == 0 {
                assert_eq!(GetLastError(), ERROR_IO_PENDING);
            }
            assert!(mine(s).is_some());
            ov.wait();
            assert!(wait_for(5000, || settled(s).map(|e| e.state) == Some(MIB_TCP_STATE_ESTAB)));
            assert_eq!(h_setsockopt(s, 0xffff, SO_UPDATE_CONNECT_CONTEXT, core::ptr::null(), 0), 0);
            let row = mine(s).unwrap();
            assert_eq!(row.state, MIB_TCP_STATE_ESTAB);
            assert_eq!(row.remote.unwrap().port(), port);
            assert_eq!(h_closesocket(s), 0);
            let d = socket(2, 1, 6);
            assert_eq!(h_bind(d, loopback_any().as_ptr(), 16), 0);
            LAST_REFRESH.store(GetTickCount64() as usize, Ordering::Relaxed);
            let far = sin([203, 0, 113, 1], 9);
            let mut ov2 = Overlapped::new();
            assert_eq!(connect_ex(d, far.as_ptr(), 16, core::ptr::null(), 0, core::ptr::null_mut(), ov2.ptr()), 0);
            assert_eq!(GetLastError(), WSAENETUNREACH);
            assert_ne!(mine(d).map(|e| e.state), Some(sockets::MIB_TCP_STATE_SYN_SENT));
            assert_eq!(h_closesocket(d), 0);
        }
    }

    #[test]
    fn acceptex_connections_appear_once_their_addresses_are_read() {
        let _g = arm();
        unsafe {
            let ls = socket(2, 1, 6);
            assert_eq!(h_bind(ls, loopback_any().as_ptr(), 16), 0);
            assert_eq!(h_listen(ls, 4), 0);
            let port = mine(ls).unwrap().local.port();
            let accept_ex: AcceptExFn = core::mem::transmute(extension(ls, &WSAID_ACCEPTEX));
            let p = extension(ls, &WSAID_GETACCEPTEXSOCKADDRS);
            assert_eq!(p, w_getacceptexsockaddrs as *const () as usize);
            let addrs: GetAcceptExSockaddrsFn = core::mem::transmute(p);
            let acc = socket(2, 1, 6);
            let mut buf = [0u8; 64];
            let mut got = 0u32;
            let mut ov = Overlapped::new();
            let r = accept_ex(ls, acc, buf.as_mut_ptr(), 0, 32, 32, &mut got, ov.ptr());
            if r == 0 {
                assert_eq!(GetLastError(), ERROR_IO_PENDING);
            }
            assert!(mine(acc).is_none());
            let client = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
            ov.wait();
            assert!(mine(acc).is_none());
            let (mut lp, mut rp) = (core::ptr::null_mut(), core::ptr::null_mut());
            let (mut ll, mut rl) = (0i32, 0i32);
            addrs(buf.as_mut_ptr(), 0, 32, 32, &mut lp, &mut ll, &mut rp, &mut rl);
            let row = mine(acc).expect("AcceptEx connection reported");
            assert_eq!(row.state, MIB_TCP_STATE_ESTAB);
            assert_eq!(row.local.port(), port);
            assert_eq!(row.remote.unwrap().port(), client.local_addr().unwrap().port());
            assert_eq!(h_closesocket(acc), 0);
            assert!(mine(acc).is_none());
            let acc2 = socket(2, 1, 6);
            let mut buf2 = [0u8; 64];
            let mut ov2 = Overlapped::new();
            let r = accept_ex(ls, acc2, buf2.as_mut_ptr(), 0, 32, 32, &mut got, ov2.ptr());
            if r == 0 {
                assert_eq!(GetLastError(), ERROR_IO_PENDING);
            }
            let client2 = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
            ov2.wait();
            assert_eq!(h_setsockopt(acc2, 0xffff, SO_UPDATE_ACCEPT_CONTEXT, &ls as *const usize as *const u8, 8), 0);
            let row = mine(acc2).expect("context-updated AcceptEx socket reported");
            assert_eq!(row.state, MIB_TCP_STATE_ESTAB);
            assert_eq!(row.remote.unwrap().port(), client2.local_addr().unwrap().port());
            assert!(PENDING_ACCEPTS.lock().unwrap().iter().all(|(_, a)| *a != acc2));
            assert_eq!(h_closesocket(acc2), 0);
            assert_eq!(h_closesocket(ls), 0);
        }
    }

    #[test]
    fn address_list_sort_is_answered_from_the_emulated_view() {
        let _g = arm();
        let mk = |ip: std::net::Ipv6Addr| {
            let mut b = [0u8; 28];
            b[0] = 0x17;
            b[8..24].copy_from_slice(&ip.octets());
            b
        };
        let root4 = mk(std::net::Ipv4Addr::new(198, 41, 0, 4).to_ipv6_mapped());
        let lo6 = mk(std::net::Ipv6Addr::LOCALHOST);
        let mut list = [0u8; 8 + 32];
        list[0] = 2;
        list[8..16].copy_from_slice(&(root4.as_ptr() as usize).to_le_bytes());
        list[16] = 28;
        list[24..32].copy_from_slice(&(lo6.as_ptr() as usize).to_le_bytes());
        list[32] = 28;
        let mut out = [0u8; 40];
        let mut got = 0u32;
        unsafe {
            let s = socket(23, 2, 0);
            assert_ne!(s, usize::MAX);
            let r = h_wsaioctl(s, crate::netview::iphlp::SIO_ADDRESS_LIST_SORT, list.as_ptr() as *const c_void, 40, out.as_mut_ptr() as *mut c_void, 40, &mut got, core::ptr::null_mut(), core::ptr::null_mut());
            assert_eq!(r, 0);
            assert_eq!(got, 40);
            assert_eq!(usize::from_le_bytes(out[8..16].try_into().unwrap()), lo6.as_ptr() as usize);
            assert_eq!(usize::from_le_bytes(out[24..32].try_into().unwrap()), root4.as_ptr() as usize);
            let r = h_wsaioctl(s, crate::netview::iphlp::SIO_ADDRESS_LIST_SORT, list.as_ptr() as *const c_void, 40, out.as_mut_ptr() as *mut c_void, 20, &mut got, core::ptr::null_mut(), core::ptr::null_mut());
            assert_eq!(r, -1);
            assert_eq!(GetLastError(), ioctl::WSAEFAULT);
            let ev = CreateEventW(core::ptr::null_mut(), 1, 0, core::ptr::null());
            let mut ov = [0usize; 4];
            ov[3] = ev as usize;
            let r = h_wsaioctl(s, crate::netview::iphlp::SIO_ADDRESS_LIST_SORT, list.as_ptr() as *const c_void, 40, list.as_mut_ptr() as *mut c_void, 40, &mut got, ov.as_mut_ptr() as *mut c_void, core::ptr::null_mut());
            assert_eq!(r, 0);
            assert_eq!(ov[1], 40);
            assert_eq!(WaitForSingleObject(ev, 0), 0);
            assert_eq!(usize::from_le_bytes(list[8..16].try_into().unwrap()), lo6.as_ptr() as usize);
            assert_eq!(h_closesocket(s), 0);
        }
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GetQueuedCompletionStatus(port: usize, bytes: *mut u32, key: *mut usize, ov: *mut *mut c_void, ms: u32) -> i32;
        fn SleepEx(ms: u32, alertable: i32) -> u32;
        fn CloseHandle(h: *mut c_void) -> i32;
    }

    fn arm_all() -> std::sync::MutexGuard<'static, ()> {
        let g = arm();
        ioctl::arm_for_tests();
        g
    }

    unsafe fn ioctl_call(s: usize, code: u32, input: &[u8], out: &mut [u8], ov: *mut c_void) -> (i32, u32, u32) {
        let mut got = 0u32;
        let r = h_wsaioctl(s, code, if input.is_empty() { core::ptr::null() } else { input.as_ptr() as *const c_void }, input.len() as u32, if out.is_empty() { core::ptr::null_mut() } else { out.as_mut_ptr() as *mut c_void }, out.len() as u32, &mut got, ov, core::ptr::null_mut());
        (r, got, if r == 0 { 0 } else { GetLastError() })
    }

    #[test]
    fn address_and_routing_queries_come_from_the_view() {
        use crate::netview::iphlp::{SIO_ADDRESS_LIST_QUERY, SIO_ROUTING_INTERFACE_QUERY};
        let _g = arm_all();
        unsafe {
            let s4 = socket(2, 2, 0);
            let (r, need, e) = ioctl_call(s4, SIO_ADDRESS_LIST_QUERY, &[], &mut [0u8; 4], core::ptr::null_mut());
            assert_eq!((r, e), (-1, ioctl::WSAEFAULT));
            assert_eq!(need, 8 + 16 + 16);
            let mut out = vec![0u8; need as usize];
            let (r, got, _) = ioctl_call(s4, SIO_ADDRESS_LIST_QUERY, &[], &mut out, core::ptr::null_mut());
            assert_eq!((r, got), (0, need));
            assert_eq!(i32::from_le_bytes(out[0..4].try_into().unwrap()), 1);
            let p = usize::from_le_bytes(out[8..16].try_into().unwrap());
            assert_eq!(p, out.as_ptr() as usize + 24);
            assert_eq!(&out[24 + 4..24 + 8], &[169, 254, 66, 1]);
            let s6 = socket(23, 2, 0);
            let mut out6 = vec![0u8; 256];
            let (r, got, _) = ioctl_call(s6, SIO_ADDRESS_LIST_QUERY, &[], &mut out6, core::ptr::null_mut());
            assert_eq!((r, got), (0, 8 + 16 + 28));
            assert_eq!(&out6[24 + 8..24 + 24], &crate::netview::NIC_IPV6.octets());
            assert_eq!(u32::from_le_bytes(out6[24 + 24..24 + 28].try_into().unwrap()), crate::netview::NIC_IF_INDEX);
            let mut o = [0u8; 16];
            let (r, got, _) = ioctl_call(s4, SIO_ROUTING_INTERFACE_QUERY, &sin([169, 254, 9, 9], 80), &mut o, core::ptr::null_mut());
            assert_eq!((r, got), (0, 16));
            assert_eq!(&o[4..8], &[169, 254, 66, 1]);
            let (r, got, _) = ioctl_call(s4, SIO_ROUTING_INTERFACE_QUERY, &sin([127, 0, 0, 1], 80), &mut o, core::ptr::null_mut());
            assert_eq!((r, got), (0, 16));
            assert_eq!(&o[4..8], &[127, 0, 0, 1]);
            let (r, _, e) = ioctl_call(s4, SIO_ROUTING_INTERFACE_QUERY, &sin([8, 8, 8, 8], 53), &mut o, core::ptr::null_mut());
            assert_eq!((r, e), (-1, ioctl::WSAENETUNREACH));
            let (r, need, e) = ioctl_call(s4, SIO_ROUTING_INTERFACE_QUERY, &sin([127, 0, 0, 1], 80), &mut [0u8; 8], core::ptr::null_mut());
            assert_eq!((r, need, e), (-1, 16, ioctl::WSAEFAULT));
            assert_eq!(h_closesocket(s4), 0);
            assert_eq!(h_closesocket(s6), 0);
        }
    }

    #[test]
    fn change_notifications_pend_until_close_and_complete_cancelled() {
        use crate::netview::iphlp::{SIO_ADDRESS_LIST_CHANGE, SIO_ROUTING_INTERFACE_CHANGE};
        let _g = arm_all();
        unsafe {
            let s = socket(2, 2, 0);
            let ev = CreateEventW(core::ptr::null_mut(), 1, 0, core::ptr::null());
            let mut ov = [0usize; 4];
            ov[3] = ev as usize;
            let (r, _, e) = ioctl_call(s, SIO_ADDRESS_LIST_CHANGE, &[], &mut [], ov.as_mut_ptr() as *mut c_void);
            assert_eq!((r, e), (-1, ioctl::WSA_IO_PENDING));
            assert_eq!(ov[0], ioctl::STATUS_PENDING);
            assert_eq!(WaitForSingleObject(ev, 50), 0x102);
            assert_eq!(ioctl::pending_count(s), 1);
            let mut nb = 1u32;
            assert_eq!(ioctl::test_ioctlsocket(s, ioctl::FIONBIO, &mut nb), 0);
            let (r, _, e) = ioctl_call(s, SIO_ROUTING_INTERFACE_CHANGE, &[], &mut [], core::ptr::null_mut());
            assert_eq!((r, e), (-1, ioctl::WSAEWOULDBLOCK));
            assert_eq!(h_closesocket(s), 0);
            assert_eq!(WaitForSingleObject(ev, 0), 0);
            assert_eq!(ov[0], ioctl::STATUS_CANCELLED);
            assert_eq!(ioctl::pending_count(s), 0);
            let b = socket(2, 2, 0);
            let t = std::thread::spawn(move || {
                let r = h_wsaioctl(b, SIO_ADDRESS_LIST_CHANGE, core::ptr::null(), 0, core::ptr::null_mut(), 0, &mut 0u32, core::ptr::null_mut(), core::ptr::null_mut());
                (r, GetLastError())
            });
            let mut spins = 0;
            while ioctl::pending_count(b) == 0 && spins < 500 {
                std::thread::sleep(std::time::Duration::from_millis(2));
                spins += 1;
            }
            assert_eq!(ioctl::pending_count(b), 1);
            assert_eq!(h_closesocket(b), 0);
            assert_eq!(t.join().unwrap(), (-1, ioctl::WSAEINTR));
        }
    }

    static APC_SEEN: std::sync::Mutex<Vec<(u32, u32)>> = std::sync::Mutex::new(Vec::new());

    unsafe extern "system" fn record_apc(err: u32, bytes: u32, _ov: *mut c_void, _flags: u32) {
        APC_SEEN.lock().unwrap().push((err, bytes));
    }

    #[test]
    fn overlapped_queries_post_to_bound_completion_ports_and_routines() {
        use crate::netview::iphlp::{SIO_ADDRESS_LIST_CHANGE, SIO_ADDRESS_LIST_SORT};
        let _g = arm_all();
        unsafe {
            let s = socket(23, 2, 0);
            let port = ioctl::test_create_iocp(s, 0, 0x77, 0);
            assert_ne!(port, 0);
            let lo6 = {
                let mut b = [0u8; 28];
                b[0] = 0x17;
                b[8 + 15] = 1;
                b
            };
            let mut list = [0u8; 24];
            list[0] = 1;
            list[8..16].copy_from_slice(&(lo6.as_ptr() as usize).to_le_bytes());
            list[16] = 28;
            let mut out = [0u8; 24];
            let mut ov = [0usize; 4];
            let mut got = 0u32;
            assert_eq!(h_wsaioctl(s, SIO_ADDRESS_LIST_SORT, list.as_ptr() as *const c_void, 24, out.as_mut_ptr() as *mut c_void, 24, &mut got, ov.as_mut_ptr() as *mut c_void, core::ptr::null_mut()), 0);
            let (mut bytes, mut key, mut po) = (0u32, 0usize, core::ptr::null_mut());
            assert_ne!(GetQueuedCompletionStatus(port, &mut bytes, &mut key, &mut po, 0), 0);
            assert_eq!((bytes, key, po as usize), (24, 0x77, ov.as_ptr() as usize));
            assert_eq!(ioctl::test_set_fcnm(s, ioctl::FILE_SKIP_COMPLETION_PORT_ON_SUCCESS), 1);
            assert_eq!(h_wsaioctl(s, SIO_ADDRESS_LIST_SORT, list.as_ptr() as *const c_void, 24, out.as_mut_ptr() as *mut c_void, 24, &mut got, ov.as_mut_ptr() as *mut c_void, core::ptr::null_mut()), 0);
            assert_eq!(GetQueuedCompletionStatus(port, &mut bytes, &mut key, &mut po, 0), 0);
            let mut ov2 = [0usize; 4];
            assert_eq!(h_wsaioctl(s, SIO_ADDRESS_LIST_CHANGE, core::ptr::null(), 0, core::ptr::null_mut(), 0, &mut got, ov2.as_mut_ptr() as *mut c_void, core::ptr::null_mut()), -1);
            assert_eq!(GetQueuedCompletionStatus(port, &mut bytes, &mut key, &mut po, 0), 0);
            assert_eq!(h_closesocket(s), 0);
            assert_eq!(GetQueuedCompletionStatus(port, &mut bytes, &mut key, &mut po, 0), 0);
            assert_eq!(GetLastError(), ioctl::WSA_OPERATION_ABORTED);
            assert_eq!(po as usize, ov2.as_ptr() as usize);
            let s2 = socket(23, 2, 0);
            let mut ov3 = [0usize; 4];
            APC_SEEN.lock().unwrap().clear();
            assert_eq!(h_wsaioctl(s2, SIO_ADDRESS_LIST_SORT, list.as_ptr() as *const c_void, 24, out.as_mut_ptr() as *mut c_void, 24, &mut got, ov3.as_mut_ptr() as *mut c_void, record_apc as *mut c_void), 0);
            assert_eq!(h_wsaioctl(s2, SIO_ADDRESS_LIST_CHANGE, core::ptr::null(), 0, core::ptr::null_mut(), 0, &mut got, ov3.as_mut_ptr() as *mut c_void, record_apc as *mut c_void), -1);
            assert_eq!(h_closesocket(s2), 0);
            SleepEx(0, 1);
            assert_eq!(*APC_SEEN.lock().unwrap(), vec![(0, 24), (ioctl::WSA_OPERATION_ABORTED, 0)]);
            CloseHandle(port as *mut c_void);
        }
    }

    #[test]
    fn completion_port_bindings_are_recorded_for_sockets_only() {
        use std::os::windows::io::AsRawHandle;
        let _g = arm_all();
        let path = std::env::temp_dir().join(format!("tombox_iocp_{}.bin", std::process::id()));
        let f = { use std::os::windows::fs::OpenOptionsExt; std::fs::OpenOptions::new().write(true).create(true).truncate(true).custom_flags(0x4000_0000).open(&path).unwrap() };
        let h = f.as_raw_handle() as usize;
        unsafe {
            let port = ioctl::test_create_iocp(h, 0, 0x55, 0);
            assert_ne!(port, 0);
            assert!(!ioctl::is_bound(h));
            let s = socket(2, 2, 0);
            assert!(!ioctl::is_bound(s));
            assert_eq!(ioctl::test_create_iocp(s, port, 0x66, 0), port);
            assert!(ioctl::is_bound(s));
            assert_eq!(h_closesocket(s), 0);
            assert!(!ioctl::is_bound(s));
            CloseHandle(port as *mut c_void);
        }
        drop(f);
        let _ = std::fs::remove_file(&path);
    }
}
