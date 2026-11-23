use core::ffi::c_void;
use core::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use super::{make_trampoline, resolve_export, write_jump};
use crate::netview::iphlp;

pub const WSAEINTR: u32 = 10004;
pub const WSAEFAULT: u32 = 10014;
pub const WSAEINVAL: u32 = 10022;
pub const WSAEWOULDBLOCK: u32 = 10035;
pub const WSAENOTSOCK: u32 = 10038;
pub const WSAENETUNREACH: u32 = 10051;
pub const WSA_IO_PENDING: u32 = 997;
pub const WSA_OPERATION_ABORTED: u32 = 995;
pub const STATUS_PENDING: usize = 0x103;
pub const STATUS_CANCELLED: usize = 0xC000_0120;
pub const FIONBIO: i32 = 0x8004_667Eu32 as i32;
pub const FILE_SKIP_COMPLETION_PORT_ON_SUCCESS: u8 = 1;
const SO_PROTOCOL_INFOW: i32 = 0x2005;
const PROTOCOL_INFO_SIZE: usize = 628;

#[link(name = "kernel32")]
extern "system" {
    fn SetLastError(code: u32);
    fn SetEvent(h: *mut c_void) -> i32;
    fn CreateEventW(sa: *mut c_void, manual: i32, initial: i32, name: *const u16) -> *mut c_void;
    fn WaitForSingleObject(h: *mut c_void, ms: u32) -> u32;
    fn QueueUserAPC(f: unsafe extern "system" fn(usize), thread: *mut c_void, data: usize) -> u32;
    fn GetCurrentThread() -> *mut c_void;
    fn GetCurrentProcess() -> *mut c_void;
    fn DuplicateHandle(sp: *mut c_void, s: *mut c_void, tp: *mut c_void, t: *mut *mut c_void, access: u32, inherit: i32, options: u32) -> u32;
    fn CloseHandle(h: *mut c_void) -> i32;
}

#[link(name = "ntdll")]
extern "system" {
    fn NtSetIoCompletion(port: *mut c_void, key: usize, apc: usize, status: i32, info: usize) -> i32;
}

const I_IOCTLSOCKET: usize = 0;
const I_EVENTSELECT: usize = 1;
const I_ASYNCSELECT: usize = 2;
const I_CREATEIOCP: usize = 3;
const I_SETFCNM: usize = 4;
static REAL: [AtomicUsize; 5] = [const { AtomicUsize::new(0) }; 5];

static NONBLOCKING: Mutex<Vec<usize>> = Mutex::new(Vec::new());
static PORTS: Mutex<Vec<Binding>> = Mutex::new(Vec::new());
static PENDING: Mutex<Vec<Pending>> = Mutex::new(Vec::new());

#[derive(Clone, Copy)]
struct Binding {
    handle: usize,
    port: usize,
    key: usize,
    skip_on_success: bool,
}

struct Pending {
    s: usize,
    ov: usize,
    cr: usize,
    thread: usize,
    event: usize,
}

unsafe impl Send for Pending {}

type WsaCompletion = unsafe extern "system" fn(u32, u32, *mut c_void, u32);

struct Apc {
    cr: usize,
    error: u32,
    bytes: u32,
    ov: usize,
    thread: usize,
}

unsafe extern "system" fn run_apc(data: usize) {
    let a = Box::from_raw(data as *mut Apc);
    let f: WsaCompletion = core::mem::transmute(a.cr);
    f(a.error, a.bytes, a.ov as *mut c_void, 0);
    CloseHandle(a.thread as *mut c_void);
}

unsafe fn current_thread() -> usize {
    let mut t: *mut c_void = core::ptr::null_mut();
    DuplicateHandle(GetCurrentProcess(), GetCurrentThread(), GetCurrentProcess(), &mut t, 0, 0, 2);
    t as usize
}

unsafe fn queue_apc(thread: usize, cr: usize, error: u32, bytes: u32, ov: usize) {
    let a = Box::into_raw(Box::new(Apc { cr, error, bytes, ov, thread }));
    if QueueUserAPC(run_apc, thread as *mut c_void, a as usize) == 0 {
        drop(Box::from_raw(a));
        CloseHandle(thread as *mut c_void);
    }
}

fn lock<T>(m: &'static Mutex<T>) -> std::sync::MutexGuard<'static, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn is_nonblocking(s: usize) -> bool {
    lock(&NONBLOCKING).contains(&s)
}

fn binding(s: usize) -> Option<Binding> {
    lock(&PORTS).iter().find(|b| b.handle == s).copied()
}

unsafe fn finish(s: usize, ov: usize, cr: usize, thread: usize, status: usize, error: u32, bytes: u32, synchronous: bool) {
    if ov != 0 {
        let o = ov as *mut usize;
        *o = status;
        *o.add(1) = bytes as usize;
    }
    if cr != 0 {
        let t = if thread != 0 { thread } else { current_thread() };
        queue_apc(t, cr, error, bytes, ov);
        return;
    }
    if thread != 0 {
        CloseHandle(thread as *mut c_void);
    }
    if ov == 0 {
        return;
    }
    let ev = *(ov as *const usize).add(3);
    if ev & !1 != 0 {
        SetEvent((ev & !1) as *mut c_void);
    }
    if ev & 1 == 0 {
        if let Some(b) = binding(s) {
            if !(synchronous && status == 0 && b.skip_on_success) {
                NtSetIoCompletion(b.port as *mut c_void, b.key, ov, status as u32 as i32, bytes as usize);
            }
        }
    }
}

unsafe fn fail(code: u32) -> i32 {
    SetLastError(code);
    -1
}

unsafe fn done(s: usize, ret: *mut u32, bytes: u32, ov: *mut c_void, cr: *mut c_void) -> i32 {
    if !ret.is_null() {
        *ret = bytes;
    }
    finish(s, ov as usize, cr as usize, 0, 0, 0, bytes, true);
    SetLastError(0);
    0
}

pub unsafe fn address_list_sort(s: usize, input: *const c_void, in_len: u32, out: *mut c_void, out_len: u32, ret: *mut u32, ov: *mut c_void, cr: *mut c_void) -> i32 {
    if input.is_null() || out.is_null() || (in_len as usize) < 4 {
        return fail(WSAEINVAL);
    }
    let n = core::ptr::read_unaligned(input as *const i32);
    let need = if n < 0 { usize::MAX } else { 8 + 16 * n as usize };
    if (in_len as usize) < need {
        return fail(WSAEINVAL);
    }
    if (out_len as usize) < need {
        return fail(WSAEFAULT);
    }
    if input as usize != out as usize {
        core::ptr::copy(input as *const u8, out as *mut u8, need);
    }
    match iphlp::address_list_sort(out as *mut u8, out_len) {
        Ok(b) => done(s, ret, b, ov, cr),
        Err(_) => fail(WSAEINVAL),
    }
}

unsafe fn is_socket(h: usize) -> bool {
    let p = resolve_export(b"ws2_32.dll\0", b"getsockopt\0");
    if p.is_null() {
        return false;
    }
    let f: unsafe extern "system" fn(usize, i32, i32, *mut u8, *mut i32) -> i32 = core::mem::transmute(p);
    let e = super::GetLastError();
    let mut ty = 0i32;
    let mut len = 4i32;
    let ok = f(h, 0xffff, 0x1008, &mut ty as *mut i32 as *mut u8, &mut len) == 0;
    SetLastError(e);
    ok
}

unsafe fn socket_family(s: usize) -> Option<u32> {
    let p = resolve_export(b"ws2_32.dll\0", b"getsockopt\0");
    if p.is_null() {
        return None;
    }
    let f: unsafe extern "system" fn(usize, i32, i32, *mut u8, *mut i32) -> i32 = core::mem::transmute(p);
    let mut info = [0u8; PROTOCOL_INFO_SIZE];
    let mut len = PROTOCOL_INFO_SIZE as i32;
    if f(s, 0xffff, SO_PROTOCOL_INFOW, info.as_mut_ptr(), &mut len) != 0 {
        return None;
    }
    Some(u32::from_le_bytes(info[0x4c..0x50].try_into().unwrap()))
}

pub unsafe fn address_list_query(s: usize, out: *mut c_void, out_len: u32, ret: *mut u32, ov: *mut c_void, cr: *mut c_void) -> i32 {
    let Some(family) = socket_family(s) else {
        return fail(WSAENOTSOCK);
    };
    match iphlp::write_address_list(family, out as *mut u8, out_len as usize) {
        Ok(n) => done(s, ret, n as u32, ov, cr),
        Err(need) => {
            if !ret.is_null() {
                *ret = need as u32;
            }
            fail(WSAEFAULT)
        }
    }
}

pub unsafe fn routing_interface_query(s: usize, input: *const c_void, in_len: u32, out: *mut c_void, out_len: u32, ret: *mut u32, ov: *mut c_void, cr: *mut c_void) -> i32 {
    if input.is_null() || in_len < 16 {
        return fail(WSAEFAULT);
    }
    let fam = core::ptr::read_unaligned(input as *const u16) as u32;
    if (fam == crate::netview::AF_INET6 && in_len < 0x1c) || !matches!(fam, crate::netview::AF_INET | crate::netview::AF_INET6) {
        return fail(WSAEINVAL);
    }
    let Some(ip) = iphlp::sockaddr_to_ip(input as *const u8) else {
        return fail(WSAEINVAL);
    };
    let Some(b) = iphlp::routing_interface(&ip) else {
        return fail(WSAENETUNREACH);
    };
    if out.is_null() || (out_len as usize) < b.len() {
        if !ret.is_null() {
            *ret = b.len() as u32;
        }
        return fail(WSAEFAULT);
    }
    core::ptr::copy_nonoverlapping(b.as_ptr(), out as *mut u8, b.len());
    done(s, ret, b.len() as u32, ov, cr)
}

pub unsafe fn change_notification(s: usize, ov: *mut c_void, cr: *mut c_void) -> i32 {
    if ov.is_null() && cr.is_null() {
        if is_nonblocking(s) {
            return fail(WSAEWOULDBLOCK);
        }
        let ev = CreateEventW(core::ptr::null_mut(), 1, 0, core::ptr::null());
        if ev.is_null() {
            return fail(WSAEINVAL);
        }
        lock(&PENDING).push(Pending { s, ov: 0, cr: 0, thread: 0, event: ev as usize });
        WaitForSingleObject(ev, 0xffff_ffff);
        CloseHandle(ev);
        return fail(WSAEINTR);
    }
    if ov.is_null() {
        return fail(WSAEINVAL);
    }
    *(ov as *mut usize) = STATUS_PENDING;
    *(ov as *mut usize).add(1) = 0;
    let thread = if cr.is_null() { 0 } else { current_thread() };
    lock(&PENDING).push(Pending { s, ov: ov as usize, cr: cr as usize, thread, event: 0 });
    fail(WSA_IO_PENDING)
}

pub fn pending_count(s: usize) -> usize {
    lock(&PENDING).iter().filter(|p| p.s == s).count()
}

pub unsafe fn socket_closing(s: usize) {
    let mine: Vec<Pending> = {
        let mut all = lock(&PENDING);
        let (m, rest): (Vec<Pending>, Vec<Pending>) = all.drain(..).partition(|p| p.s == s);
        *all = rest;
        m
    };
    for p in mine {
        if p.event != 0 {
            SetEvent(p.event as *mut c_void);
        } else {
            finish(s, p.ov, p.cr, p.thread, STATUS_CANCELLED, WSA_OPERATION_ABORTED, 0, false);
        }
    }
    lock(&NONBLOCKING).retain(|x| *x != s);
    lock(&PORTS).retain(|b| b.handle != s);
}

pub unsafe fn handle_ioctl(s: usize, code: u32, input: *const c_void, in_len: u32, out: *mut c_void, out_len: u32, ret: *mut u32, ov: *mut c_void, cr: *mut c_void) -> Option<i32> {
    match code {
        iphlp::SIO_ADDRESS_LIST_SORT => Some(address_list_sort(s, input, in_len, out, out_len, ret, ov, cr)),
        iphlp::SIO_ADDRESS_LIST_QUERY => Some(address_list_query(s, out, out_len, ret, ov, cr)),
        iphlp::SIO_ROUTING_INTERFACE_QUERY => Some(routing_interface_query(s, input, in_len, out, out_len, ret, ov, cr)),
        iphlp::SIO_ADDRESS_LIST_CHANGE | iphlp::SIO_ROUTING_INTERFACE_CHANGE => Some(change_notification(s, ov, cr)),
        _ => None,
    }
}

unsafe fn real(i: usize) -> usize {
    REAL[i].load(Ordering::Relaxed)
}

unsafe extern "system" fn h_ioctlsocket(s: usize, cmd: i32, arg: *mut u32) -> i32 {
    let f: unsafe extern "system" fn(usize, i32, *mut u32) -> i32 = core::mem::transmute(real(I_IOCTLSOCKET));
    let r = f(s, cmd, arg);
    if r == 0 && cmd == FIONBIO && !arg.is_null() {
        let mut nb = lock(&NONBLOCKING);
        nb.retain(|x| *x != s);
        if *arg != 0 {
            nb.push(s);
        }
    }
    r
}

fn mark_nonblocking(s: usize) {
    let mut nb = lock(&NONBLOCKING);
    if !nb.contains(&s) {
        nb.push(s);
    }
}

unsafe extern "system" fn h_wsaeventselect(s: usize, ev: *mut c_void, mask: i32) -> i32 {
    let f: unsafe extern "system" fn(usize, *mut c_void, i32) -> i32 = core::mem::transmute(real(I_EVENTSELECT));
    let r = f(s, ev, mask);
    if r == 0 && mask != 0 {
        mark_nonblocking(s);
    }
    r
}

unsafe extern "system" fn h_wsaasyncselect(s: usize, hwnd: *mut c_void, msg: u32, mask: i32) -> i32 {
    let f: unsafe extern "system" fn(usize, *mut c_void, u32, i32) -> i32 = core::mem::transmute(real(I_ASYNCSELECT));
    let r = f(s, hwnd, msg, mask);
    if r == 0 && mask != 0 {
        mark_nonblocking(s);
    }
    r
}

unsafe extern "system" fn h_create_iocp(file: usize, existing: usize, key: usize, threads: u32) -> usize {
    let f: unsafe extern "system" fn(usize, usize, usize, u32) -> usize = core::mem::transmute(real(I_CREATEIOCP));
    let r = f(file, existing, key, threads);
    if r != 0 && file != usize::MAX && file != 0 && is_socket(file) {
        let mut p = lock(&PORTS);
        p.retain(|b| b.handle != file);
        p.push(Binding { handle: file, port: r, key, skip_on_success: false });
    }
    r
}

unsafe extern "system" fn h_set_fcnm(handle: usize, flags: u8) -> i32 {
    let f: unsafe extern "system" fn(usize, u8) -> i32 = core::mem::transmute(real(I_SETFCNM));
    let r = f(handle, flags);
    if r != 0 {
        if let Some(b) = lock(&PORTS).iter_mut().find(|b| b.handle == handle) {
            b.skip_on_success = flags & FILE_SKIP_COMPLETION_PORT_ON_SUCCESS != 0;
        }
    }
    r
}

pub const HOOKS: usize = 5;

pub fn install() -> (usize, Vec<String>) {
    let table: [(&[u8], &[u8], usize, usize); HOOKS] = [
        (b"ws2_32.dll\0", b"ioctlsocket\0", I_IOCTLSOCKET, h_ioctlsocket as *const () as usize),
        (b"ws2_32.dll\0", b"WSAEventSelect\0", I_EVENTSELECT, h_wsaeventselect as *const () as usize),
        (b"ws2_32.dll\0", b"WSAAsyncSelect\0", I_ASYNCSELECT, h_wsaasyncselect as *const () as usize),
        (b"kernelbase.dll\0", b"CreateIoCompletionPort\0", I_CREATEIOCP, h_create_iocp as *const () as usize),
        (b"kernel32.dll\0", b"SetFileCompletionNotificationModes\0", I_SETFCNM, h_set_fcnm as *const () as usize),
    ];
    let (mut hooked, mut missing) = (0, Vec::new());
    for (dll, f, idx, hook) in table {
        let name = String::from_utf8_lossy(&f[..f.len() - 1]).into_owned();
        if REAL[idx].load(Ordering::SeqCst) != 0 {
            hooked += 1;
            continue;
        }
        let p = unsafe { resolve_export(dll, f) };
        if p.is_null() {
            missing.push(name);
            continue;
        }
        match unsafe { make_trampoline(p) } {
            Some(t) => {
                REAL[idx].store(t, Ordering::SeqCst);
                if unsafe { write_jump(p, hook) } { hooked += 1 } else { missing.push(name) }
            }
            None => missing.push(name),
        }
    }
    (hooked, missing)
}

#[cfg(test)]
pub fn arm_for_tests() {
    for (dll, f, idx) in [(&b"ws2_32.dll\0"[..], &b"ioctlsocket\0"[..], I_IOCTLSOCKET), (b"ws2_32.dll\0", b"WSAEventSelect\0", I_EVENTSELECT), (b"ws2_32.dll\0", b"WSAAsyncSelect\0", I_ASYNCSELECT), (b"kernelbase.dll\0", b"CreateIoCompletionPort\0", I_CREATEIOCP), (b"kernel32.dll\0", b"SetFileCompletionNotificationModes\0", I_SETFCNM)] {
        let p = unsafe { resolve_export(dll, f) };
        assert!(!p.is_null());
        REAL[idx].store(p as usize, Ordering::SeqCst);
    }
}

#[cfg(test)]
pub unsafe fn test_ioctlsocket(s: usize, cmd: i32, arg: *mut u32) -> i32 {
    h_ioctlsocket(s, cmd, arg)
}

#[cfg(test)]
pub unsafe fn test_create_iocp(file: usize, existing: usize, key: usize, threads: u32) -> usize {
    h_create_iocp(file, existing, key, threads)
}

#[cfg(test)]
pub unsafe fn test_set_fcnm(handle: usize, flags: u8) -> i32 {
    h_set_fcnm(handle, flags)
}

#[cfg(test)]
pub fn is_bound(h: usize) -> bool {
    binding(h).is_some()
}
