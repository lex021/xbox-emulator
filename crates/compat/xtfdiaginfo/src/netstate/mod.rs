use crate::imports;
use crate::sys::*;
use crate::titles;
use crate::trace::{ADAPTERS_GUID, PORTS_GUID, PROCESSES_GUID, TEREDO_GUID, TRACE_MESSAGE_FLAGS, WPP, WPP_STATE, WPP_STATE_FLAG};
use compat_common::netview;
use compat_common::netview::sockets;
use core::ffi::c_void;
use core::ptr::{null, null_mut};
use std::sync::atomic::Ordering;

pub mod guids;
pub mod wfp;
pub mod wfp_format;
pub mod wfp_tables;

pub const REDACTED: u64 = 0x1111_1111_1111_1111;
pub const ERROR_FLAG: u32 = 0x1000_0000;
pub const TRACE_FLAG: u32 = 0x2000_0000;
pub const HR_NO_MORE_ITEMS: u32 = 0x8007_0103;
pub const HR_NO_DATA: u32 = 0x8007_00e8;
pub const HR_OUT_OF_MEMORY: u32 = 0x8007_000e;

const SP8: &str = "        ";
const SP12: &str = "            ";
const SP16: &str = "                ";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gate {
    Error,
    Trace,
    State,
}

#[derive(Clone, Copy)]
pub enum Arg<'a> {
    U32(u32),
    U64(u64),
    W(Option<&'a [u16]>),
    A(Option<&'a [u8]>),
}

pub const Z: Arg<'static> = Arg::U64(0);

pub fn r(p: usize) -> Arg<'static> {
    Arg::U64(if p != 0 { REDACTED } else { 0 })
}

pub fn p(v: usize) -> Arg<'static> {
    Arg::U64(v as u64)
}

pub fn u(v: u32) -> Arg<'static> {
    Arg::U32(v)
}

pub fn w(s: &[u16]) -> Arg<'_> {
    Arg::W(Some(s))
}

pub fn ws(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

pub fn hr_map(r: u32) -> u32 {
    if (r as i32) > 0 {
        0x8007_0000 | (r & 0xffff)
    } else {
        r
    }
}

fn encode(a: &Arg) -> Vec<u8> {
    match a {
        Arg::U32(v) => v.to_le_bytes().to_vec(),
        Arg::U64(v) => v.to_le_bytes().to_vec(),
        Arg::W(s) => {
            let mut o: Vec<u8> = Vec::new();
            for c in s.unwrap_or(&[0x4e, 0x55, 0x4c, 0x4c]).iter() {
                o.extend_from_slice(&c.to_le_bytes());
            }
            o.extend_from_slice(&[0, 0]);
            o
        }
        Arg::A(s) => {
            let mut o = s.unwrap_or(b"NULL").to_vec();
            o.push(0);
            o
        }
    }
}

#[link(name = "advapi32")]
extern "system" {
    fn TraceMessageVa(logger: u64, flags: u32, guid: *const GUID, number: u16, args: *mut c_void) -> u32;
}

#[link(name = "ws2_32")]
extern "system" {
    fn WSAStartup(version: u16, data: *mut u8) -> i32;
    fn GetNameInfoW(sa: *const u8, len: i32, host: *mut u16, host_len: u32, serv: *mut u16, serv_len: u32, flags: i32) -> i32;
}

#[link(name = "kernel32")]
extern "system" {
    fn K32EnumProcessModulesEx(h: HANDLE, modules: *mut usize, cb: u32, needed: *mut u32, filter: u32) -> BOOL;
    fn K32GetModuleBaseNameW(h: HANDLE, module: usize, name: *mut u16, size: u32) -> u32;
}

#[cfg(test)]
pub mod capture {
    use super::Gate;
    use crate::sys::GUID;
    use std::cell::RefCell;

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct Record {
        pub gate: Gate,
        pub guid: GUID,
        pub id: u16,
        pub parts: Vec<Vec<u8>>,
    }

    thread_local! {
        static CAP: RefCell<Option<Vec<Record>>> = const { RefCell::new(None) };
    }

    pub fn start() {
        CAP.with(|c| *c.borrow_mut() = Some(Vec::new()));
    }

    pub fn take() -> Vec<Record> {
        CAP.with(|c| c.borrow_mut().take().unwrap_or_default())
    }

    pub fn active() -> bool {
        CAP.with(|c| c.borrow().is_some())
    }

    pub(super) fn push(gate: Gate, guid: &GUID, id: u16, parts: &[Vec<u8>]) -> bool {
        CAP.with(|c| match c.borrow_mut().as_mut() {
            Some(v) => {
                v.push(Record { gate, guid: *guid, id, parts: parts.to_vec() });
                true
            }
            None => false,
        })
    }
}

pub fn enabled(g: Gate) -> bool {
    #[cfg(test)]
    if capture::active() {
        return true;
    }
    match g {
        Gate::Error => WPP.logger.load(Ordering::Acquire) != 0 && WPP.flags.load(Ordering::Acquire) & ERROR_FLAG != 0,
        Gate::Trace => WPP.logger.load(Ordering::Acquire) != 0 && WPP.flags.load(Ordering::Acquire) & TRACE_FLAG != 0,
        Gate::State => WPP_STATE.logger.load(Ordering::Acquire) != 0 && WPP_STATE.flags.load(Ordering::Acquire) & WPP_STATE_FLAG != 0,
    }
}

pub unsafe fn rec(g: Gate, guid: &GUID, id: u16, args: &[Arg]) {
    if !enabled(g) {
        return;
    }
    let parts: Vec<Vec<u8>> = args.iter().map(encode).collect();
    #[cfg(test)]
    if capture::push(g, guid, id, &parts) {
        return;
    }
    let logger = match g {
        Gate::State => WPP_STATE.logger.load(Ordering::Acquire),
        _ => WPP.logger.load(Ordering::Acquire),
    };
    if logger == 0 {
        return;
    }
    let mut slots: Vec<usize> = Vec::with_capacity(parts.len() * 2 + 1);
    for x in parts.iter() {
        slots.push(x.as_ptr() as usize);
        slots.push(x.len());
    }
    slots.push(0);
    TraceMessageVa(logger, TRACE_MESSAGE_FLAGS, guid, id, slots.as_mut_ptr() as *mut c_void);
}

pub unsafe fn st(guid: &GUID, id: u16, args: &[Arg]) {
    rec(Gate::State, guid, id, args)
}

pub unsafe fn er(guid: &GUID, id: u16, args: &[Arg]) {
    rec(Gate::Error, guid, id, args)
}

pub unsafe fn tr(guid: &GUID, id: u16, args: &[Arg]) {
    rec(Gate::Trace, guid, id, args)
}

unsafe fn r8(a: usize) -> u8 {
    (a as *const u8).read_unaligned()
}

unsafe fn r16(a: usize) -> u16 {
    (a as *const u16).read_unaligned()
}

unsafe fn r32(a: usize) -> u32 {
    (a as *const u32).read_unaligned()
}

unsafe fn r64(a: usize) -> u64 {
    (a as *const u64).read_unaligned()
}

unsafe fn rp(a: usize) -> usize {
    r64(a) as usize
}

unsafe fn wide_at(a: usize) -> Option<Vec<u16>> {
    if a == 0 {
        return None;
    }
    let mut v = Vec::new();
    let mut i = 0;
    loop {
        let c = r16(a + i * 2);
        if c == 0 {
            return Some(v);
        }
        v.push(c);
        i += 1;
    }
}

unsafe fn narrow_at(a: usize) -> Option<Vec<u8>> {
    if a == 0 {
        return None;
    }
    let mut v = Vec::new();
    let mut i = 0;
    loop {
        let c = r8(a + i);
        if c == 0 {
            return Some(v);
        }
        v.push(c);
        i += 1;
    }
}

fn aligned(bytes: u32) -> Result<Vec<u64>, ()> {
    let n = (bytes as usize).div_ceil(8);
    let mut v: Vec<u64> = Vec::new();
    v.try_reserve_exact(n).map_err(|_| ())?;
    v.resize(n, 0);
    Ok(v)
}

fn prefix_origin(v: u32) -> &'static [u8] {
    match v {
        0 => b"Other",
        1 => b"Manual",
        2 => b"WellKnown",
        3 => b"Dhcp",
        4 => b"RouterAdvertisement",
        16 => b"Unchanged",
        _ => b"???",
    }
}

fn suffix_origin(v: u32) -> &'static [u8] {
    match v {
        0 => b"Other",
        1 => b"Manual",
        2 => b"WellKnown",
        3 => b"Dhcp",
        4 => b"LinkLayerAddress",
        5 => b"Random",
        16 => b"Unchanged",
        _ => b"???",
    }
}

fn dad_state(v: u32) -> &'static [u8] {
    match v {
        0 => b"Invalid",
        1 => b"Tentative",
        2 => b"Duplicate",
        3 => b"Deprecated",
        4 => b"Preferred",
        _ => b"???",
    }
}

unsafe fn winsock_startup() {
    let mut data = [0u8; 0x198];
    let e = WSAStartup(0x202, data.as_mut_ptr());
    if e != 0 {
        er(&ADAPTERS_GUID, 0x0a, &[Z, u(e as u32)]);
    }
}

unsafe fn numeric_name(sa: usize, len: i32, err_id: u16) -> Vec<u16> {
    winsock_startup();
    let mut buf = [0u16; 0x80];
    let e = GetNameInfoW(sa as *const u8, len, buf.as_mut_ptr(), 0x80, null_mut(), 0, 0x0a);
    if e != 0 {
        er(&ADAPTERS_GUID, err_id, &[Z, u(e as u32)]);
        return Vec::new();
    }
    buf.iter().take_while(|c| **c != 0).copied().collect()
}

unsafe fn socket_address(prefix: &str, a: usize) {
    let ptr = rp(a);
    let len = r32(a + 8) as i32;
    let text = if len > 0 && ptr != 0 { numeric_name(ptr, len, 0x0d) } else { ws("nullptr") };
    let pre = ws(prefix);
    st(&ADAPTERS_GUID, 0x0e, &[Z, w(&pre), r(ptr), w(&text)]);
    st(&ADAPTERS_GUID, 0x0f, &[Z, w(&pre), u(len as u32)]);
}

unsafe fn sockaddr_inet(prefix: &str, a: usize) {
    let text = numeric_name(a, 0x1c, 0x0b);
    let pre = ws(prefix);
    st(&ADAPTERS_GUID, 0x0c, &[Z, w(&pre), r(a), w(&text)]);
}

unsafe fn address_list(head: usize, base: u16) {
    let s = |id: u16, args: &[Arg]| st(&ADAPTERS_GUID, id, args);
    s(base, &[Z, r(head)]);
    let mut x = head;
    while x != 0 {
        s(base + 1, &[Z, r(x)]);
        s(base + 2, &[Z, u(r32(x))]);
        s(base + 3, &[Z, u(r32(x + 4))]);
        s(base + 4, &[Z, r(rp(x + 8))]);
        s(base + 5, &[Z, r(x + 0x10)]);
        socket_address(SP12, x + 0x10);
        x = rp(x + 8);
    }
}

pub unsafe fn dump_adapter(a: usize) {
    let s = |id: u16, args: &[Arg]| st(&ADAPTERS_GUID, id, args);
    s(0x10, &[Z, u(r32(a))]);
    s(0x11, &[Z, u(r32(a + 4))]);
    s(0x12, &[Z, r(rp(a + 8))]);
    let name = narrow_at(rp(a + 0x10));
    s(0x13, &[Z, Arg::A(name.as_deref())]);
    s(0x14, &[Z, r(rp(a + 0x18))]);
    let mut x = rp(a + 0x18);
    while x != 0 {
        s(0x15, &[Z, r(x)]);
        s(0x16, &[Z, u(r32(x))]);
        s(0x17, &[Z, u(r32(x + 4))]);
        s(0x18, &[Z, r(rp(x + 8))]);
        s(0x19, &[Z, r(x + 0x10)]);
        socket_address(SP12, x + 0x10);
        let po = r32(x + 0x20);
        s(0x1a, &[Z, u(po), Arg::A(Some(prefix_origin(po)))]);
        let so = r32(x + 0x24);
        s(0x1b, &[Z, u(so), Arg::A(Some(suffix_origin(so)))]);
        let ds = r32(x + 0x28);
        s(0x1c, &[Z, u(ds), Arg::A(Some(dad_state(ds)))]);
        s(0x1d, &[Z, u(r32(x + 0x2c))]);
        s(0x1e, &[Z, u(r32(x + 0x30))]);
        s(0x1f, &[Z, u(r32(x + 0x34))]);
        s(0x20, &[Z, u(r8(x + 0x38) as u32)]);
        x = rp(x + 8);
    }
    address_list(rp(a + 0x20), 0x21);
    address_list(rp(a + 0x28), 0x27);
    address_list(rp(a + 0x30), 0x2d);
    let dns = wide_at(rp(a + 0x38));
    s(0x33, &[Z, Arg::W(dns.as_deref())]);
    let desc = wide_at(rp(a + 0x40));
    s(0x34, &[Z, Arg::W(desc.as_deref())]);
    let friendly = wide_at(rp(a + 0x48));
    s(0x35, &[Z, Arg::W(friendly.as_deref())]);
    let plen = r32(a + 0x58);
    match plen {
        6 | 8 => {
            let mut args = vec![Z];
            for i in 0..plen as usize {
                args.push(u(r8(a + 0x50 + i) as u32));
            }
            s(if plen == 6 { 0x36 } else { 0x37 }, &args);
        }
        _ => s(0x38, &[Z, r(a + 0x50)]),
    }
    s(0x39, &[Z, u(plen)]);
    let flags = r32(a + 0x5c);
    s(0x3a, &[Z, u(flags)]);
    for n in 0..10u16 {
        s(0x3b + n, &[Z, u((flags >> n) & 1)]);
    }
    s(0x45, &[Z, u(r32(a + 0x60))]);
    s(0x46, &[Z, u(r32(a + 0x64))]);
    s(0x47, &[Z, u(r32(a + 0x68))]);
    s(0x48, &[Z, u(r32(a + 0x6c))]);
    s(0x49, &[Z, r(a + 0x70)]);
    s(0x4a, &[Z, r(rp(a + 0xb0))]);
    let mut x = rp(a + 0xb0);
    while x != 0 {
        s(0x4b, &[Z, r(x)]);
        s(0x4c, &[Z, u(r32(x))]);
        s(0x4d, &[Z, u(r32(x + 4))]);
        s(0x4e, &[Z, r(rp(x + 8))]);
        s(0x4f, &[Z, r(x + 0x10)]);
        socket_address(SP12, x + 0x10);
        s(0x50, &[Z, u(r32(x + 0x20))]);
        x = rp(x + 8);
    }
    s(0x51, &[Z, Arg::U64(r64(a + 0xb8))]);
    s(0x52, &[Z, Arg::U64(r64(a + 0xc0))]);
    s(0x53, &[Z, u(r32(a + 0xd8))]);
    s(0x54, &[Z, u(r32(a + 0xdc))]);
    s(0x55, &[Z, Arg::U64(r64(a + 0xe0))]);
    s(0x56, &[Z, r(a + 0xe8)]);
    socket_address(SP8, a + 0xe8);
    s(0x57, &[Z, u(r32(a + 0xf8))]);
    let mut g = vec![Z, u(r32(a + 0xfc)), u(r16(a + 0x100) as u32), u(r16(a + 0x102) as u32)];
    for i in 0..8 {
        g.push(u(r8(a + 0x104 + i) as u32));
    }
    s(0x58, &g);
    s(0x59, &[Z, u(r32(a + 0x10c))]);
    s(0x5a, &[Z, u(r32(a + 0x110))]);
    s(0x5b, &[Z, r(a + 0x118)]);
    socket_address(SP8, a + 0x118);
    s(0x5c, &[Z, r(a + 0x128)]);
    s(0x5d, &[Z, u(r32(a + 0x1ac))]);
    s(0x5e, &[Z, u(r32(a + 0x1b0))]);
    s(0x5f, &[Z, r(rp(a + 0x1b8))]);
}

pub unsafe fn dump_route(row: usize) {
    let pre = ws(SP8);
    let s = |id: u16, v: Arg| st(&ADAPTERS_GUID, id, &[Z, w(&pre), v]);
    s(0x60, Arg::U64(r64(row)));
    s(0x61, u(r32(row + 8)));
    s(0x62, r(row + 0xc));
    s(0x63, r(row + 0xc));
    sockaddr_inet(SP16, row + 0xc);
    s(0x64, u(r8(row + 0x28) as u32));
    s(0x65, r(row + 0x2c));
    sockaddr_inet(SP12, row + 0x2c);
    s(0x66, u(r8(row + 0x48) as u32));
    s(0x67, u(r32(row + 0x4c)));
    s(0x68, u(r32(row + 0x50)));
    s(0x69, u(r32(row + 0x54)));
    s(0x6a, u(r32(row + 0x58)));
    s(0x6b, u(r8(row + 0x5c) as u32));
    s(0x6c, u(r8(row + 0x5d) as u32));
    s(0x6d, u(r8(row + 0x5e) as u32));
    s(0x6e, u(r8(row + 0x5f) as u32));
    s(0x6f, u(r32(row + 0x60)));
    s(0x70, u(r32(row + 0x64)));
}

unsafe fn adapters_buffer() -> Result<Vec<u64>, u32> {
    let mut size = 0u32;
    let mut alloc = 0u32;
    let mut buf: Vec<u64> = Vec::new();
    loop {
        let ptr = if alloc == 0 { null_mut() } else { buf.as_mut_ptr() as *mut u8 };
        let r = netview::get_adapters_addresses(netview::AF_UNSPEC, 6, ptr, &mut size);
        match r {
            0 => {
                if alloc == 0 {
                    er(&ADAPTERS_GUID, 0x8a, &[Z]);
                    return Err(0xe8);
                }
                return Ok(buf);
            }
            0x6f | 0x7a => {
                if size <= alloc {
                    er(&ADAPTERS_GUID, 0x8c, &[Z, u(size), u(alloc)]);
                    return Err(13);
                }
                match aligned(size) {
                    Ok(b) => {
                        buf = b;
                        alloc = size;
                    }
                    Err(()) => {
                        er(&ADAPTERS_GUID, 0x8d, &[Z, u(size)]);
                        return Err(8);
                    }
                }
            }
            e => {
                er(&ADAPTERS_GUID, 0x8b, &[Z, u(e)]);
                return Err(e);
            }
        }
    }
}

unsafe fn route_table() -> Result<Vec<u8>, u32> {
    match netview::ip_forward_table2(netview::AF_UNSPEC) {
        Err(e) => {
            er(&ADAPTERS_GUID, 0x8e, &[Z, u(e)]);
            Err(e)
        }
        Ok(t) => {
            if u32::from_le_bytes(t[0..4].try_into().unwrap()) == 0 {
                er(&ADAPTERS_GUID, 0x8f, &[Z]);
                return Err(0xe8);
            }
            Ok(t)
        }
    }
}

pub unsafe fn log_adapters() -> u32 {
    st(&ADAPTERS_GUID, 0x92, &[Z]);
    let hr = match adapters_buffer() {
        Err(e) => {
            er(&ADAPTERS_GUID, 0x95, &[Z]);
            hr_map(e)
        }
        Ok(buf) => {
            let mut a = buf.as_ptr() as usize;
            while a != 0 {
                st(&ADAPTERS_GUID, 0x96, &[Z, r(a)]);
                dump_adapter(a);
                a = rp(a + 8);
            }
            match route_table() {
                Err(e) => {
                    er(&ADAPTERS_GUID, 0x97, &[Z]);
                    hr_map(e)
                }
                Ok(t) => {
                    let n = u32::from_le_bytes(t[0..4].try_into().unwrap());
                    let base = t.as_ptr() as usize;
                    st(&ADAPTERS_GUID, 0x98, &[Z, u(n), r(base)]);
                    let mut i = 0u32;
                    while i < n {
                        let row = base + 8 + i as usize * netview::ROUTE_ROW_SIZE;
                        i += 1;
                        if i & 0x3f == 0 {
                            Sleep(0x10);
                        }
                        st(&ADAPTERS_GUID, 0x71, &[Z, r(row)]);
                        dump_route(row);
                    }
                    0
                }
            }
        }
    };
    st(&ADAPTERS_GUID, 0x99, &[Z, u(hr)]);
    hr
}

pub unsafe fn log_teredo() -> u32 {
    st(&TEREDO_GUID, 0x12, &[Z]);
    let id = netview::NPI_MS_TEREDO_MODULEID;
    let mut rw = [0u8; 12];
    let r = netview::nsi_get_all_parameters(1, id.as_ptr(), netview::NSI_TEREDO_OBJECT_STATE, null(), 0, rw.as_mut_ptr(), 12, null_mut(), 0, null_mut(), 0);
    if r != 0 {
        er(&TEREDO_GUID, 0x14, &[Z, u(r), u(r)]);
        let hr = hr_map(r);
        st(&TEREDO_GUID, 0x1b, &[Z, u(hr)]);
        return hr;
    }
    let mut port = [0u8; 2];
    let r = netview::nsi_get_all_parameters(1, id.as_ptr(), netview::NSI_TEREDO_OBJECT_PORT, null(), 0, port.as_mut_ptr(), 2, null_mut(), 0, null_mut(), 0);
    if r == netview::ERROR_NOT_READY {
        st(&TEREDO_GUID, 0x16, &[Z]);
    } else if r != 0 {
        er(&TEREDO_GUID, 0x15, &[Z, u(r), u(r)]);
        let hr = hr_map(r);
        st(&TEREDO_GUID, 0x1b, &[Z, u(hr)]);
        return hr;
    }
    let d = |o: usize| u32::from_le_bytes(rw[o..o + 4].try_into().unwrap());
    st(&TEREDO_GUID, 0x18, &[Z, u(d(0)), u(d(4)), u(rw[8] as u32), u(rw[9] as u32), u(u16::from_le_bytes(port) as u32)]);
    st(&TEREDO_GUID, 0x1b, &[Z, u(0)]);
    0
}

pub unsafe fn process_name(pid: u32) -> Vec<u16> {
    if pid == 0 {
        return ws("<invalid process ID>");
    }
    if pid == 4 {
        return ws("<system process ID>");
    }
    let h = titles::open_process(0x1000, pid);
    if h.is_null() {
        let e = GetLastError();
        er(&PROCESSES_GUID, 0x1a, &[Z, u(pid), u(pid), u(e)]);
        return ws(&format!("<couldn't open process, err {}>", e));
    }
    let mut buf = vec![0u16; 0x400];
    let mut size = 0x400u32;
    let name = if QueryFullProcessImageNameW(h, 0, buf.as_mut_ptr(), &mut size) == 0 {
        let e = GetLastError();
        er(&PROCESSES_GUID, 0x1c, &[Z, u(pid), u(pid), u(e)]);
        ws(&format!("<couldn't get process filepath, err {}>", e))
    } else {
        buf.truncate(size as usize);
        buf
    };
    CloseHandle(h);
    name
}

unsafe fn enumerate_pids() -> Result<Vec<u32>, u32> {
    let pids = titles::title_pids();
    let mut size = 0u32;
    let returned = loop {
        size += 0x100;
        st(&PROCESSES_GUID, 0x0b, &[Z, u(size)]);
        let returned = ((pids.len() * 4) as u32).min(size);
        if returned < size {
            break returned;
        }
    };
    let count = (returned / 4) as usize;
    if count == 0 {
        er(&PROCESSES_GUID, 0x0d, &[Z]);
        return Err(HR_NO_DATA);
    }
    st(&PROCESSES_GUID, 0x0e, &[Z, u(count as u32)]);
    Ok(pids[..count].to_vec())
}

unsafe fn enumerate_modules(h: HANDLE) -> Result<Vec<usize>, u32> {
    let mut size = 0u32;
    loop {
        size += 0x200;
        st(&PROCESSES_GUID, 0x10, &[Z, u(size)]);
        let mut buf = vec![0usize; size as usize / 8];
        let mut needed = 0u32;
        if K32EnumProcessModulesEx(h, buf.as_mut_ptr(), size, &mut needed, 3) == 0 {
            let e = GetLastError();
            er(&PROCESSES_GUID, 0x11, &[Z, u(e)]);
            return Err(hr_map(e));
        }
        if needed < size {
            let count = needed as usize / 8;
            if count == 0 {
                er(&PROCESSES_GUID, 0x12, &[Z]);
                return Err(HR_NO_DATA);
            }
            st(&PROCESSES_GUID, 0x13, &[Z, u(count as u32)]);
            buf.truncate(count);
            return Ok(buf);
        }
    }
}

unsafe fn log_modules(pid: u32) {
    let h = titles::open_process(0x410, pid);
    if h.is_null() {
        let e = GetLastError();
        if e != 5 {
            er(&PROCESSES_GUID, 0x14, &[Z, u(pid), u(pid), u(e)]);
            return;
        }
        st(&PROCESSES_GUID, 0x15, &[Z, u(pid), u(pid)]);
        st(&PROCESSES_GUID, 0x18, &[Z, u(0)]);
        return;
    }
    match enumerate_modules(h) {
        Err(_) => er(&PROCESSES_GUID, 0x17, &[Z]),
        Ok(mods) => {
            st(&PROCESSES_GUID, 0x18, &[Z, u(mods.len() as u32)]);
            for (i, m) in mods.iter().enumerate() {
                if (i + 1) & 0x3f == 0 {
                    Sleep(0x10);
                }
                let mut buf = vec![0u16; 0x104];
                let n = K32GetModuleBaseNameW(h, *m, buf.as_mut_ptr(), 0x104);
                let name = if n == 0 {
                    let e = GetLastError();
                    ws(&format!("<couldn't get module index {} (0x{:016X}) name, err {}>", i, m, e))
                } else {
                    buf.truncate(n as usize);
                    buf
                };
                st(&PROCESSES_GUID, 0x19, &[Z, p(*m), w(&name)]);
            }
        }
    }
    CloseHandle(h);
}

pub unsafe fn log_processes() -> u32 {
    st(&PROCESSES_GUID, 0x1d, &[Z, u(1)]);
    let hr = match enumerate_pids() {
        Err(hr) => {
            er(&PROCESSES_GUID, 0x1e, &[Z]);
            hr
        }
        Ok(pids) => {
            st(&PROCESSES_GUID, 0x1f, &[Z, u(pids.len() as u32)]);
            let mut i = 0u32;
            for pid in pids {
                i += 1;
                if i & 3 == 0 {
                    Sleep(0x10);
                }
                let name = process_name(pid);
                st(&PROCESSES_GUID, 0x20, &[Z, u(pid), u(pid), w(&name)]);
                if pid & !4 != 0 {
                    log_modules(pid);
                }
            }
            0
        }
    };
    st(&PROCESSES_GUID, 0x21, &[Z, u(hr)]);
    hr
}

fn tcp_state_name(s: u32) -> Vec<u16> {
    const NAMES: [&str; 12] = [
        "MIB_TCP_STATE_CLOSED",
        "MIB_TCP_STATE_LISTEN",
        "MIB_TCP_STATE_SYN_SENT",
        "MIB_TCP_STATE_SYN_RCVD",
        "MIB_TCP_STATE_ESTAB",
        "MIB_TCP_STATE_FIN_WAIT1",
        "MIB_TCP_STATE_FIN_WAIT2",
        "MIB_TCP_STATE_CLOSE_WAIT",
        "MIB_TCP_STATE_CLOSING",
        "MIB_TCP_STATE_LAST_ACK",
        "MIB_TCP_STATE_TIME_WAIT",
        "MIB_TCP_STATE_DELETE_TCB",
    ];
    match s {
        1..=12 => ws(NAMES[s as usize - 1]),
        _ => ws(&format!("??? {}/0x{:08x}", s as i32, s)),
    }
}

unsafe fn ipv4_text(a: usize) -> Vec<u16> {
    ws(&format!("{}.{}.{}.{}", r8(a), r8(a + 1), r8(a + 2), r8(a + 3)))
}

unsafe fn ipv6_text(a: usize) -> Vec<u16> {
    let g: Vec<String> = (0..8).map(|i| format!("{:02X}{:02X}", r8(a + i * 2), r8(a + i * 2 + 1))).collect();
    ws(&g.join(":"))
}

unsafe fn net_port(a: usize) -> u32 {
    let v = r32(a);
    ((v & 0xff) << 8) | ((v >> 8) & 0xff)
}

struct PortRows<'a> {
    pre: &'a [u16],
}

impl PortRows<'_> {
    unsafe fn s(&self, id: u16, rest: &[Arg]) {
        let mut args = vec![Z, w(self.pre)];
        args.extend_from_slice(rest);
        st(&PORTS_GUID, id, &args);
    }

    unsafe fn process(&self, id: u16, pid: u32) {
        let name = process_name(pid);
        self.s(id, &[u(pid), u(pid), w(&name)]);
    }

    unsafe fn state(&self, id: u16, v: u32) {
        let name = tcp_state_name(v);
        self.s(id, &[u(v), w(&name)]);
    }

    unsafe fn tcp4(&self, a: usize) {
        self.state(0x0a, r32(a));
        let ip = ipv4_text(a + 4);
        self.s(0x0b, &[w(&ip)]);
        self.s(0x0c, &[u(net_port(a + 8))]);
        let ip = ipv4_text(a + 0xc);
        self.s(0x0d, &[w(&ip)]);
        self.s(0x0e, &[u(net_port(a + 0x10))]);
        self.process(0x0f, r32(a + 0x14));
        self.s(0x10, &[Arg::U64(r64(a + 0x18))]);
        self.s(0x11, &[Arg::U64(r64(a + 0x20)), Arg::U64(r64(a + 0x28))]);
    }

    unsafe fn tcp6(&self, a: usize) {
        let ip = ipv6_text(a);
        self.s(0x13, &[w(&ip)]);
        self.s(0x14, &[u(r32(a + 0x10))]);
        self.s(0x15, &[u(net_port(a + 0x14))]);
        let ip = ipv6_text(a + 0x18);
        self.s(0x16, &[w(&ip)]);
        self.s(0x17, &[u(r32(a + 0x28))]);
        self.s(0x18, &[u(net_port(a + 0x2c))]);
        self.state(0x19, r32(a + 0x30));
        self.process(0x1a, r32(a + 0x34));
        self.s(0x1b, &[Arg::U64(r64(a + 0x38))]);
        self.s(0x1c, &[Arg::U64(r64(a + 0x40)), Arg::U64(r64(a + 0x48))]);
    }

    unsafe fn udp4(&self, a: usize) {
        let ip = ipv4_text(a);
        self.s(0x1e, &[w(&ip)]);
        self.s(0x1f, &[u(net_port(a + 4))]);
        self.process(0x20, r32(a + 8));
        self.s(0x21, &[Arg::U64(r64(a + 0x10))]);
        let f = r32(a + 0x18);
        self.s(0x22, &[u(f)]);
        self.s(0x23, &[u(if f & 1 != 0 { 0xffff_ffff } else { 0 })]);
        self.s(0x24, &[Arg::U64(r64(a + 0x20)), Arg::U64(r64(a + 0x28))]);
    }

    unsafe fn udp6(&self, a: usize) {
        let ip = ipv6_text(a);
        self.s(0x26, &[w(&ip)]);
        self.s(0x27, &[u(r32(a + 0x10))]);
        self.s(0x28, &[u(net_port(a + 0x14))]);
        self.process(0x29, r32(a + 0x18));
        self.s(0x2a, &[Arg::U64(r64(a + 0x20))]);
        let f = r32(a + 0x28);
        self.s(0x2b, &[u(f)]);
        self.s(0x2c, &[u(if f & 1 != 0 { 0xffff_ffff } else { 0 })]);
        self.s(0x2d, &[Arg::U64(r64(a + 0x30)), Arg::U64(r64(a + 0x38))]);
    }
}

struct TableSpec {
    tcp: bool,
    family: u32,
    initial: u32,
    alloc_err: u16,
    call_err: u16,
    size_err: u16,
}

unsafe fn port_table(t: &TableSpec) -> Result<Vec<u64>, u32> {
    let mut size = t.initial;
    let mut alloc = t.initial;
    let mut buf: Vec<u64> = if t.initial == 0 {
        Vec::new()
    } else {
        match aligned(t.initial) {
            Ok(b) => b,
            Err(()) => {
                er(&PORTS_GUID, t.alloc_err, &[Z, u(t.initial)]);
                return Err(HR_OUT_OF_MEMORY);
            }
        }
    };
    loop {
        let ptr = if alloc == 0 { null_mut() } else { buf.as_mut_ptr() as *mut u8 };
        let r = if t.tcp {
            sockets::get_extended_tcp_table(ptr, &mut size, true, t.family, sockets::TCP_TABLE_OWNER_MODULE_ALL, 0)
        } else {
            sockets::get_extended_udp_table(ptr, &mut size, true, t.family, sockets::UDP_TABLE_OWNER_MODULE, 0)
        };
        match r {
            0 => return Ok(buf),
            0x6f | 0x7a => {
                if size <= alloc {
                    er(&PORTS_GUID, t.size_err, &[Z, u(size), u(alloc)]);
                    return Err(hr_map(0xd));
                }
                match aligned(size) {
                    Ok(b) => {
                        buf = b;
                        alloc = size;
                    }
                    Err(()) => {
                        er(&PORTS_GUID, t.alloc_err, &[Z, u(size)]);
                        return Err(HR_OUT_OF_MEMORY);
                    }
                }
            }
            e => {
                er(&PORTS_GUID, t.call_err, &[Z, u(e)]);
                return Err(hr_map(e));
            }
        }
    }
}

pub unsafe fn log_ports() -> u32 {
    st(&PORTS_GUID, 0x3b, &[Z]);
    let pre = ws(SP8);
    let rows = PortRows { pre: &pre };
    let tables: [(TableSpec, u16, u16, u16, usize, unsafe fn(&PortRows, usize)); 4] = [
        (TableSpec { tcp: true, family: netview::AF_INET, initial: 0xa8, alloc_err: 0x2f, call_err: 0x30, size_err: 0x31 }, 0x3c, 0x3d, 0x12, sockets::TCP_ROW_OWNER_MODULE_SIZE, |r, a| r.tcp4(a)),
        (TableSpec { tcp: true, family: netview::AF_INET6, initial: 0xc8, alloc_err: 0x32, call_err: 0x33, size_err: 0x34 }, 0x3e, 0x3f, 0x1d, sockets::TCP6_ROW_OWNER_MODULE_SIZE, |r, a| r.tcp6(a)),
        (TableSpec { tcp: false, family: netview::AF_INET, initial: 0, alloc_err: 0x37, call_err: 0x35, size_err: 0x36 }, 0x40, 0x41, 0x25, sockets::UDP_ROW_OWNER_MODULE_SIZE, |r, a| r.udp4(a)),
        (TableSpec { tcp: false, family: netview::AF_INET6, initial: 0, alloc_err: 0x3a, call_err: 0x38, size_err: 0x39 }, 0x42, 0x43, 0x2e, sockets::UDP6_ROW_OWNER_MODULE_SIZE, |r, a| r.udp6(a)),
    ];
    let mut hr = 0u32;
    for (spec, fail_id, summary_id, row_id, stride, dump) in tables.iter() {
        match port_table(spec) {
            Err(e) => {
                er(&PORTS_GUID, *fail_id, &[Z]);
                hr = e;
                break;
            }
            Ok(t) => {
                let base = t.as_ptr() as usize;
                let n = r32(base);
                st(&PORTS_GUID, *summary_id, &[Z, u(n), p(base)]);
                for i in 0..n as usize {
                    let row = base + 8 + i * stride;
                    st(&PORTS_GUID, *row_id, &[Z, p(row)]);
                    dump(&rows, row);
                }
            }
        }
    }
    st(&PORTS_GUID, 0x44, &[Z, u(hr)]);
    hr
}

pub const SECTION_ERRORS: [&[u8]; 5] = [
    b"Couldn't do IP config adapters and routes display (err 0x%08x)!  Continuing.\0",
    b"Couldn't do Teredo display (err 0x%08x)!  Continuing.\0",
    b"Couldn't do processes display (err 0x%08x)!  Continuing.\0",
    b"Couldn't do TCP/UDP port display (err 0x%08x)!  Continuing.\0",
    b"Couldn't perform WFP enumeration (err 0x%08x)!  Continuing.\0",
];

pub unsafe fn log_state_sections() -> [u32; 5] {
    let sections: [unsafe fn() -> u32; 5] = [log_adapters, log_teredo, log_processes, log_ports, wfp::log_wfp];
    let mut out = [0u32; 5];
    for (i, f) in sections.iter().enumerate() {
        let hr = f();
        out[i] = hr;
        if (hr as i32) < 0 {
            imports::log_error(SECTION_ERRORS[i], &[hr as u64]);
        }
    }
    out
}

#[cfg(test)]
mod tests;
