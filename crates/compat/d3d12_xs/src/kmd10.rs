use core::ffi::c_void;
use core::ptr::{null, null_mut};
use core::sync::atomic::{AtomicBool, AtomicI32, AtomicIsize, AtomicUsize, Ordering};

pub const INVALID: isize = -1;
pub const E_INVALIDARG: i32 = 0x8007_0057u32 as i32;
pub const E_FAIL: i32 = 0x8000_4005u32 as i32;
pub const E_POINTER: i32 = 0x8000_4003u32 as i32;
pub const STATUS_UNSUCCESSFUL: i32 = 0xc000_0001u32 as i32;
pub const KMD10_PATH: &str = "\\\\.\\XgsKmd10";
pub const IOCTL_PROBE: u32 = 0x7900_0000;
pub const IOCTL_HANDSHAKE: u32 = 0x7900_c190;
pub const IOCTL_RELEASE: u32 = 0x7900_c194;
pub const IOCTL_UPLOAD_MICROCODE: u32 = 0x7900_c223;
pub const IOCTL_MEMORY_PROFILING: u32 = 0x7900_c1cc;
pub const LEGACY_BUILD: u32 = 22000;

extern "system" {
    fn GetLastError() -> u32;
    fn GetModuleHandleW(n: *const u16) -> *mut c_void;
    fn GetProcAddress(h: *mut c_void, n: *const u8) -> *mut c_void;
}

type DirectCall = unsafe extern "system" fn(u32, *const c_void, u32, *mut c_void, *mut u32) -> i32;

static HANDLE: AtomicIsize = AtomicIsize::new(INVALID);
static DIRECT: AtomicBool = AtomicBool::new(false);
static LOCK: AtomicI32 = AtomicI32::new(0);
static DIRECT_FN: AtomicUsize = AtomicUsize::new(0);

pub fn os_build() -> u32 {
    compat_common::os_host::k32legacy::console_os_build()
}

pub fn legacy() -> bool {
    os_build() < LEGACY_BUILD
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(core::iter::once(0)).collect()
}

pub fn hresult_from_win32(e: u32) -> i32 {
    if (e as i32) <= 0 {
        e as i32
    } else {
        ((e & 0xffff) | 0x8007_0000) as i32
    }
}

fn last_error_hr() -> i32 {
    hresult_from_win32(unsafe { GetLastError() })
}

fn direct_fn() -> Option<DirectCall> {
    let mut v = DIRECT_FN.load(Ordering::Acquire);
    if v == 0 {
        let m = unsafe { GetModuleHandleW(wide("ntdll.dll").as_ptr()) };
        let f = if m.is_null() { null_mut() } else { unsafe { GetProcAddress(m, b"NtDirectGraphicsCall\0".as_ptr()) } };
        v = if f.is_null() { 1 } else { f as usize };
        DIRECT_FN.store(v, Ordering::Release);
    }
    if v == 1 {
        None
    } else {
        Some(unsafe { core::mem::transmute::<usize, DirectCall>(v) })
    }
}

unsafe fn direct(code: u32, inb: *const c_void, inl: u32, outb: *mut c_void, outl: *mut u32) -> Option<i32> {
    direct_fn().map(|f| f(code, inb, inl, outb, outl))
}

fn lock() {
    loop {
        if LOCK.fetch_add(1, Ordering::AcqRel) + 1 == 1 {
            return;
        }
        LOCK.fetch_sub(1, Ordering::AcqRel);
        core::hint::spin_loop();
    }
}

fn unlock() {
    LOCK.fetch_sub(1, Ordering::AcqRel);
}

pub fn cached() -> isize {
    HANDLE.load(Ordering::Acquire)
}

pub fn direct_enabled() -> bool {
    DIRECT.load(Ordering::Acquire)
}

unsafe fn ioctl(h: isize, code: u32, inb: *const c_void, inl: u32, outb: *mut c_void, outl: u32, ret: &mut u32) -> bool {
    xgs_kmd_emu::device_io_control(h, code, inb, inl, outb, outl, ret)
}

fn create_kmd() -> isize {
    xgs_kmd_emu::open()
}

fn close_kmd(h: isize) -> bool {
    xgs_kmd_emu::close(h)
}

pub unsafe fn open(out: *mut isize, handshake: bool) -> i32 {
    if out.is_null() {
        return E_INVALIDARG;
    }
    let h = HANDLE.load(Ordering::Acquire);
    if h != INVALID {
        *out = h;
        return 0;
    }
    lock();
    let mut hr = 0;
    let h = HANDLE.load(Ordering::Acquire);
    if h != INVALID {
        *out = h;
    } else {
        let probe = if legacy() { 0 } else { IOCTL_PROBE };
        DIRECT.store(direct(probe, null(), 0, null_mut(), null_mut()) == Some(0), Ordering::Release);
        let f = create_kmd();
        if f == INVALID {
            hr = last_error_hr();
        } else {
            let mut keep = true;
            if handshake {
                let mut ret = 0u32;
                let st = if !ioctl(f, IOCTL_HANDSHAKE, null(), 0, null_mut(), 0, &mut ret) {
                    last_error_hr()
                } else if ret != 0 {
                    E_FAIL
                } else {
                    0
                };
                if st < 0 {
                    close_kmd(f);
                    hr = st;
                    keep = false;
                }
            }
            if keep {
                HANDLE.store(f, Ordering::Release);
                *out = f;
            }
        }
    }
    unlock();
    hr
}

unsafe fn open_global(handshake: bool) -> i32 {
    let mut h = INVALID;
    let hr = open(&mut h, handshake);
    if hr >= 0 {
        HANDLE.store(h, Ordering::Release);
    }
    hr
}

unsafe fn device_status(code: u32, inb: *const c_void, inl: u32, outb: *mut c_void, outl: u32) -> i32 {
    let mut ret = 0u32;
    let hr = if ioctl(HANDLE.load(Ordering::Acquire), code, inb, inl, outb, outl, &mut ret) { 0 } else { last_error_hr() };
    if hr != 0 {
        STATUS_UNSUCCESSFUL
    } else {
        0
    }
}

unsafe fn device_hresult(h: isize, code: u32, inb: *const c_void, inl: u32) -> i32 {
    let mut ret = 0u32;
    if ioctl(h, code, inb, inl, null_mut(), 0, &mut ret) {
        0
    } else {
        last_error_hr()
    }
}

struct Direct {
    inb: *const c_void,
    inl: u32,
    outb: *mut c_void,
    size: Option<u32>,
}

unsafe fn direct_call(code: u32, d: Direct) -> Option<i32> {
    if !DIRECT.load(Ordering::Acquire) {
        return None;
    }
    let mut size = d.size.unwrap_or(0);
    let sp = if d.size.is_some() { &mut size as *mut u32 } else { null_mut() };
    direct(code, d.inb, d.inl, d.outb, sp)
}

unsafe fn simple(code: u32, inb: *const c_void, inl: u32) -> i32 {
    if let Some(st) = direct_call(code, Direct { inb, inl, outb: null_mut(), size: None }) {
        return st;
    }
    device_status(code, inb, inl, null_mut(), 0)
}

unsafe fn transient(code: u32, inb: *const c_void, inl: u32, has_direct: bool) -> i32 {
    if has_direct {
        if let Some(st) = direct_call(code, Direct { inb, inl, outb: null_mut(), size: None }) {
            return st;
        }
    }
    let was_closed = HANDLE.load(Ordering::Acquire) == INVALID;
    if open_global(false) < 0 {
        return STATUS_UNSUCCESSFUL;
    }
    let st = device_status(code, inb, inl, null_mut(), 0);
    if was_closed && !legacy() {
        close_kmd(HANDLE.load(Ordering::Acquire));
        HANDLE.store(INVALID, Ordering::Release);
    }
    st
}

unsafe extern "system" fn open_device(out: *mut isize) -> i32 {
    open(out, true)
}

unsafe extern "system" fn release_device(h: isize) -> i32 {
    lock();
    if HANDLE.load(Ordering::Acquire) != INVALID {
        HANDLE.store(INVALID, Ordering::Release);
        if h != INVALID {
            let mut ret = 0u32;
            if !ioctl(h, IOCTL_RELEASE, null(), 0, null_mut(), 0, &mut ret) {
                GetLastError();
            }
        }
        close_kmd(h);
    }
    unlock();
    0
}

unsafe fn query(h: isize, code: u32, inb: *const c_void, inl: u32, outb: *mut c_void, outl: u32) -> Result<(), i32> {
    if h == INVALID {
        return Err(E_INVALIDARG);
    }
    let mut ret = 0u32;
    if ioctl(h, code, inb, inl, outb, outl, &mut ret) {
        if ret != outl {
            return Err(E_FAIL);
        }
        return Ok(());
    }
    let hr = last_error_hr();
    if hr < 0 {
        Err(hr)
    } else {
        Ok(())
    }
}

unsafe extern "system" fn set_pair(h: isize, a: u32, b: u32) -> i32 {
    let inb = [a, b];
    match query(h, 0x7900_c19c, inb.as_ptr().cast(), 8, null_mut(), 0) {
        Ok(()) => 0,
        Err(e) => e,
    }
}

#[repr(C)]
struct CounterSet {
    count: u32,
    pad: u32,
    ids: [u64; 32],
    a: [u32; 32],
    b: [u32; 32],
}

unsafe extern "system" fn set_counters(h: isize, count: u32, ids: *const u64, a: *const u32, b: *const u32) -> i32 {
    let mut s = CounterSet { count: 0, pad: 0, ids: [0; 32], a: [0; 32], b: [0; 32] };
    if count > 0x20 {
        return E_INVALIDARG;
    }
    s.count = count;
    for i in 0..count as usize {
        s.ids[i] = *ids.add(i);
        s.a[i] = *a.add(i);
        s.b[i] = *b.add(i);
    }
    match query(h, 0x7900_c1ec, (&s as *const CounterSet).cast(), 0x208, null_mut(), 0) {
        Ok(()) => 0,
        Err(e) => e,
    }
}

unsafe extern "system" fn read_u32(h: isize, out: *mut u32) -> i32 {
    if h == INVALID {
        return E_INVALIDARG;
    }
    let mut v = 0u32;
    let mut ret = 0u32;
    if !ioctl(h, 0x7900_c1a8, null(), 0, (&mut v as *mut u32).cast(), 4, &mut ret) {
        return last_error_hr();
    }
    if !out.is_null() {
        *out = v;
    }
    0
}

unsafe extern "system" fn read_pair(h: isize, a: *mut u64, b: *mut u64) -> i32 {
    let mut o = [0u64; 2];
    match query(h, 0x7900_c1e0, null(), 0, o.as_mut_ptr().cast(), 0x10) {
        Ok(()) => {
            *a = o[0];
            *b = o[1];
            0
        }
        Err(e) => e,
    }
}

unsafe extern "system" fn read_triple(h: isize, a: *mut u64, b: *mut u32, c: *mut u32) -> i32 {
    let mut o = [0u64; 2];
    match query(h, 0x7900_c1fc, null(), 0, o.as_mut_ptr().cast(), 0x10) {
        Ok(()) => {
            *a = o[0];
            *b = o[1] as u32;
            *c = (o[1] >> 32) as u32;
            0
        }
        Err(e) => e,
    }
}

#[repr(C)]
struct AllocIn {
    a: u64,
    align: u64,
    flags: u32,
    pad: u32,
    extra: u64,
}

#[allow(clippy::too_many_arguments)]
unsafe extern "system" fn allocate(h: isize, a: u64, align: u64, flags: u32, extra: u64, o1: *mut u64, o2: *mut u64, o3: *mut u32) -> i32 {
    if o1.is_null() || ((flags as i32) >= 0 && o2.is_null()) {
        return E_INVALIDARG;
    }
    if align != 0 && align & (align - 1) != 0 {
        return E_INVALIDARG;
    }
    if flags & 0x4000_0000 != 0 && extra.wrapping_sub(1) <= 0x0fff_fffe {
        return E_INVALIDARG;
    }
    if flags & 0x2000_0004 == 0x2000_0004 {
        return E_INVALIDARG;
    }
    let inb = AllocIn { a, align, flags, pad: 0, extra };
    let mut o = [0u64; 3];
    match query(h, 0x7900_c198, (&inb as *const AllocIn).cast(), 0x20, o.as_mut_ptr().cast(), 0x18) {
        Ok(()) => {
            *o1 = o[0];
            if !o2.is_null() {
                *o2 = o[1];
            }
            if !o3.is_null() {
                *o3 = o[2] as u32;
            }
            0
        }
        Err(e) => e,
    }
}

unsafe extern "system" fn query_fresh(zero: usize, a: u32, o1: *mut u64, o2: *mut u64) -> i32 {
    if o1.is_null() || zero != 0 {
        return E_INVALIDARG;
    }
    let f = create_kmd();
    if f == INVALID {
        return last_error_hr();
    }
    let mut o = [0u64; 2];
    let r = query(f, 0x7900_c1b0, (&a as *const u32).cast(), 4, o.as_mut_ptr().cast(), 0x10);
    close_kmd(f);
    match r {
        Ok(()) => {
            *o1 = o[0];
            if !o2.is_null() {
                *o2 = o[1];
            }
            0
        }
        Err(e) => e,
    }
}

unsafe extern "system" fn debug_break() -> i32 {
    core::arch::asm!("int3");
    STATUS_UNSUCCESSFUL
}

unsafe extern "system" fn no_op() -> i32 {
    0
}

unsafe extern "system" fn c20b(p: *const c_void) -> i32 {
    simple(0x7900_c20b, p, 0x20)
}

unsafe extern "system" fn c207(a: u32, b: u32) -> i32 {
    let inb = [a, b];
    simple(0x7900_c207, inb.as_ptr().cast(), 8)
}

unsafe extern "system" fn c20f(p: *const c_void) -> i32 {
    simple(0x7900_c20f, p, 0x20)
}

unsafe extern "system" fn c1bb(p: *const c_void, out: *mut c_void) -> i32 {
    if let Some(st) = direct_call(0x7900_c1bb, Direct { inb: p, inl: 0x130, outb: out, size: Some(0x10) }) {
        return st;
    }
    device_status(0x7900_c1bb, p, 0x130, out, 0x10)
}

unsafe extern "system" fn c1cb(p: *const c_void) -> i32 {
    simple(0x7900_c1cb, p, 0x20)
}

unsafe extern "system" fn c23f(p: *const c_void, out: *mut c_void) -> i32 {
    if let Some(st) = direct_call(0x7900_c23f, Direct { inb: p, inl: 4, outb: out, size: Some(0x90) }) {
        return st;
    }
    device_status(0x7900_c23f, p, 4, out, 0x90)
}

unsafe extern "system" fn c21b(p: *const c_void) -> i32 {
    transient(0x7900_c21b, p, 0x20, true)
}

#[repr(C)]
struct Quad {
    a: u64,
    b: u32,
    c: u32,
    d: u64,
}

unsafe extern "system" fn c1af(a: u64, b: u32, c: u32, d: u64) -> i32 {
    let q = Quad { a, b, c, d };
    simple(0x7900_c1af, (&q as *const Quad).cast(), 0x18)
}

#[repr(C)]
struct Pair {
    a: u32,
    pad: u32,
    b: u64,
}

unsafe extern "system" fn c24b(a: u32, b: u64) -> i32 {
    let p = Pair { a, pad: 0, b };
    simple(0x7900_c24b, (&p as *const Pair).cast(), 0x10)
}

unsafe extern "system" fn c24f(h: isize, p: *const c_void) -> i32 {
    let mut h = h;
    if h == 0 {
        let hr = open(&mut h, true);
        if hr < 0 {
            return hr;
        }
    }
    device_hresult(h, 0x7900_c24f, p, 0x20)
}

unsafe extern "system" fn c1a7(p: *const c_void) -> i32 {
    simple(0x7900_c1a7, p, 0x4028)
}

unsafe extern "system" fn c237(p: *const c_void) -> i32 {
    simple(0x7900_c237, p, 0x20)
}

unsafe extern "system" fn c233(p: *const c_void) -> i32 {
    simple(0x7900_c233, p, 0x40)
}

unsafe extern "system" fn c273(p: *const c_void) -> i32 {
    simple(0x7900_c273, p, 0x40)
}

unsafe extern "system" fn c23b(a: u64, b: u32, c: u64) -> i32 {
    let q = Quad { a, b, c: 0, d: c };
    simple(0x7900_c23b, (&q as *const Quad).cast(), 0x18)
}

pub fn c217_size() -> u32 {
    if legacy() {
        0x48
    } else {
        0x50
    }
}

unsafe extern "system" fn c217(p: *mut c_void) -> i32 {
    let n = c217_size();
    if let Some(st) = direct_call(0x7900_c217, Direct { inb: null(), inl: 0, outb: p, size: Some(n) }) {
        return st;
    }
    device_status(0x7900_c217, p, n, null_mut(), 0)
}

unsafe extern "system" fn c283(p: *const c_void) -> i32 {
    transient(0x7900_c283, p, 4, true)
}

unsafe extern "system" fn c287(p: *const c_void) -> i32 {
    if legacy() {
        return device_status(0x7900_c287, p, 0x4c, null_mut(), 0);
    }
    transient(0x7900_c287, p, 0x4c, false)
}

unsafe extern "system" fn c1a3(p: *mut c_void) -> i32 {
    if let Some(st) = direct_call(0x7900_c1a3, Direct { inb: null(), inl: 0, outb: p, size: Some(4) }) {
        return st;
    }
    device_status(0x7900_c1a3, p, 4, null_mut(), 0)
}

unsafe extern "system" fn c28f(p: *const c_void) -> i32 {
    transient(0x7900_c28f, p, 0x10, false)
}

pub fn table_size() -> usize {
    if legacy() {
        0xf0
    } else {
        0x100
    }
}

pub fn entries() -> Vec<(usize, usize)> {
    let mut v: Vec<(usize, usize)> = vec![
        (0x08, open_device as *const () as usize),
        (0x10, release_device as *const () as usize),
        (0x18, set_pair as *const () as usize),
        (0x20, set_counters as *const () as usize),
        (0x28, read_triple as *const () as usize),
        (0x30, read_u32 as *const () as usize),
        (0x38, read_pair as *const () as usize),
        (0x40, c20b as *const () as usize),
        (0x48, c207 as *const () as usize),
        (0x50, c20f as *const () as usize),
        (0x58, c1bb as *const () as usize),
        (0x60, debug_break as *const () as usize),
        (0x68, c1cb as *const () as usize),
        (0x70, c23f as *const () as usize),
        (0x78, c21b as *const () as usize),
        (0x80, no_op as *const () as usize),
        (0x88, c1af as *const () as usize),
        (0x90, c24b as *const () as usize),
        (0x98, allocate as *const () as usize),
        (0xa0, query_fresh as *const () as usize),
        (0xa8, c24f as *const () as usize),
        (0xb0, c1a7 as *const () as usize),
        (0xb8, c237 as *const () as usize),
        (0xc0, c233 as *const () as usize),
        (0xc8, c273 as *const () as usize),
        (0xd0, c23b as *const () as usize),
        (0xd8, c217 as *const () as usize),
        (0xe0, c283 as *const () as usize),
        (0xe8, c287 as *const () as usize),
    ];
    if !legacy() {
        v.push((0xf0, c1a3 as *const () as usize));
        v.push((0xf8, c28f as *const () as usize));
    }
    v
}

#[no_mangle]
pub unsafe extern "system" fn PixGetPixieEntryPoints(table: *mut u8) -> i32 {
    if table.is_null() {
        return E_POINTER;
    }
    let size = (table as *const u32).read_unaligned();
    let cap = table_size();
    let n = (size as usize).min(cap);
    let mut local = [0u8; 0x100];
    local[..4].copy_from_slice(&size.to_le_bytes());
    for (off, f) in entries() {
        local[off..off + 8].copy_from_slice(&(f as u64).to_le_bytes());
    }
    core::ptr::copy_nonoverlapping(local.as_ptr(), table, n);
    0
}

#[repr(C)]
pub struct MicrocodeUpload {
    pub kind: u32,
    pub pad: u32,
    pub c: usize,
    pub b: usize,
    pub e: usize,
    pub d: usize,
    pub g: usize,
    pub f: usize,
}

#[no_mangle]
pub unsafe extern "system" fn D3D12UploadCustomMicrocodeX(kind: u32, b: usize, c: usize, d: usize, e: usize, f: usize, g: usize) -> i32 {
    if kind == 0 || b == 0 || c == 0 || d == 0 || e == 0 || f == 0 || g == 0 {
        return E_INVALIDARG;
    }
    let m = MicrocodeUpload { kind, pad: 0, c, b, e, d, g, f };
    let p = (&m as *const MicrocodeUpload).cast();
    let st = match direct_call(IOCTL_UPLOAD_MICROCODE, Direct { inb: p, inl: 0x38, outb: null_mut(), size: None }) {
        Some(st) => st,
        None => device_status(IOCTL_UPLOAD_MICROCODE, p, 0x38, null_mut(), 0),
    };
    if st != 0 {
        E_FAIL
    } else {
        0
    }
}

#[no_mangle]
pub unsafe extern "system" fn D3D12XboxEnableMemoryProfiling(a: u64, b: u64, c: u64) -> i32 {
    let hr = open_global(true);
    if hr < 0 {
        return hr;
    }
    let h = HANDLE.load(Ordering::Acquire);
    if h == INVALID {
        return E_INVALIDARG;
    }
    let inb = [a, b, c];
    device_hresult(h, IOCTL_MEMORY_PROFILING, inb.as_ptr().cast(), 0x18)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOT_SUPPORTED: i32 = 0x8007_0032u32 as i32;

    #[link(name = "kernel32")]
    extern "system" {
        fn AddVectoredExceptionHandler(first: u32, h: unsafe extern "system" fn(*mut *mut u8) -> i32) -> *mut c_void;
        fn RemoveVectoredExceptionHandler(h: *mut c_void) -> u32;
    }

    static BREAKS: AtomicUsize = AtomicUsize::new(0);

    unsafe extern "system" fn skip_break(info: *mut *mut u8) -> i32 {
        let rec = *info as *const u32;
        if *rec != 0x8000_0003 {
            return 0;
        }
        let ctx = *info.add(1);
        let rip = ctx.add(0xf8) as *mut u64;
        *rip += 1;
        BREAKS.fetch_add(1, Ordering::SeqCst);
        -1
    }

    fn table() -> Vec<u64> {
        let mut t = vec![0u64; 0x100 / 8];
        t[0] = 0x100;
        assert_eq!(unsafe { PixGetPixieEntryPoints(t.as_mut_ptr().cast()) }, 0);
        t
    }

    #[test]
    fn pixie_table_has_console_layout() {
        assert_eq!(unsafe { PixGetPixieEntryPoints(null_mut()) }, E_POINTER);
        let t = table();
        let cap = table_size();
        assert_eq!(cap, if legacy() { 0xf0 } else { 0x100 });
        assert_eq!(t[0], 0x100);
        let filled: Vec<usize> = (1..cap / 8).filter(|&i| t[i] != 0).map(|i| i * 8).collect();
        assert_eq!(filled, (1..cap / 8).map(|i| i * 8).collect::<Vec<_>>());
        let mut small = [0xaaaa_aaaa_aaaa_aaaau64; 4];
        small[0] = 0x10;
        assert_eq!(unsafe { PixGetPixieEntryPoints(small.as_mut_ptr().cast()) }, 0);
        assert_ne!(small[1], 0);
        assert_eq!(small[2], 0xaaaa_aaaa_aaaa_aaaa);
        let ok: unsafe extern "system" fn() -> i32 = unsafe { core::mem::transmute(t[0x80 / 8] as usize) };
        assert_eq!(unsafe { ok() }, 0);
        let brk: unsafe extern "system" fn() -> i32 = unsafe { core::mem::transmute(t[0x60 / 8] as usize) };
        let v = unsafe { AddVectoredExceptionHandler(1, skip_break) };
        let before = BREAKS.load(Ordering::SeqCst);
        assert_eq!(unsafe { brk() }, STATUS_UNSUCCESSFUL);
        assert_eq!(BREAKS.load(Ordering::SeqCst), before + 1, "console slot 0x60 breaks into the debugger first");
        unsafe { RemoveVectoredExceptionHandler(v) };
    }

    static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn serial() -> std::sync::MutexGuard<'static, ()> {
        SERIAL.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[test]
    fn pixie_entries_validate_like_console() {
        let _g = serial();
        let (mut a, mut b, mut c, mut d) = (0u64, 0u64, 0u32, 0u32);
        let t = table();
        let open: unsafe extern "system" fn(*mut isize) -> i32 = unsafe { core::mem::transmute(t[1] as usize) };
        assert_eq!(unsafe { open(null_mut()) }, E_INVALIDARG);
        assert!(!direct_enabled());
        unsafe {
            assert_eq!(set_pair(INVALID, 1, 2), E_INVALIDARG);
            assert_eq!(set_counters(INVALID, 0x21, null(), null(), null()), E_INVALIDARG);
            assert_eq!(set_counters(INVALID, 0, null(), null(), null()), E_INVALIDARG);
            assert_eq!(read_u32(INVALID, &mut c), E_INVALIDARG);
            assert_eq!(read_pair(INVALID, &mut a, &mut b), E_INVALIDARG);
            assert_eq!(read_triple(INVALID, &mut a, &mut c, &mut d), E_INVALIDARG);
            assert_eq!(allocate(INVALID, 0, 3, 0x8000_0000, 0, &mut a, null_mut(), null_mut()), E_INVALIDARG);
            assert_eq!(allocate(INVALID, 0, 0, 0x2000_0004, 0, &mut a, &mut b, null_mut()), E_INVALIDARG);
            assert_eq!(allocate(INVALID, 0, 0, 0, 0, &mut a, &mut b, null_mut()), E_INVALIDARG);
            assert_eq!(query_fresh(1, 0, &mut a, null_mut()), E_INVALIDARG);
        }
        assert_eq!(c217_size(), if legacy() { 0x48 } else { 0x50 });
    }

    #[test]
    fn read_triple_returns_emulated_kmd_gpu_configuration() {
        let _g = serial();
        let t = table();
        let open: unsafe extern "system" fn(*mut isize) -> i32 = unsafe { core::mem::transmute(t[0x08 / 8] as usize) };
        let release: unsafe extern "system" fn(isize) -> i32 = unsafe { core::mem::transmute(t[0x10 / 8] as usize) };
        let triple: unsafe extern "system" fn(isize, *mut u64, *mut u32, *mut u32) -> i32 = unsafe { core::mem::transmute(t[0x28 / 8] as usize) };
        let mut h = INVALID;
        assert_eq!(unsafe { open(&mut h) }, 0);
        assert_ne!(h, INVALID);
        assert_eq!(cached(), h);
        let (mut f, mut m, mut cu) = (0u64, 0u32, 0u32);
        assert_eq!(unsafe { triple(h, &mut f, &mut m, &mut cu) }, 0);
        let dt = xgs_kmd_emu::device_type();
        let r = crate::console_slots_dev2::kmd_reply(dt);
        assert_eq!(f.to_le_bytes(), r[0..8]);
        assert_eq!(m.to_le_bytes(), r[8..12]);
        assert_eq!(cu.to_le_bytes(), r[12..16]);
        assert_eq!(f, xgs_kmd_emu::gpu_frequency_hz(dt));
        let info = crate::console_slots_dev2::console_info(crate::console_slots_dev2::family(dt), dt);
        assert_eq!(&info[0..8], &r[0..8]);
        assert_eq!(unsafe { release(h) }, 0);
        assert_eq!(cached(), INVALID);
        assert!(!xgs_kmd_emu::is_device(h));
        assert_eq!(unsafe { triple(h, &mut f, &mut m, &mut cu) }, 0x8007_0006u32 as i32);
    }

    #[test]
    fn unmodeled_ioctls_fail_on_the_emulated_device() {
        let _g = serial();
        let t = table();
        let open: unsafe extern "system" fn(*mut isize) -> i32 = unsafe { core::mem::transmute(t[1] as usize) };
        let mut h = INVALID;
        assert_eq!(unsafe { open(&mut h) }, 0);
        let buf = [0u8; 0x4028];
        let mut out = [0u8; 0x90];
        let (mut a, mut b, mut c) = (0u64, 0u64, 0u32);
        unsafe {
            assert_eq!(read_u32(h, &mut c), NOT_SUPPORTED);
            let t0 = core::arch::x86_64::_rdtsc();
            assert_eq!(read_pair(h, &mut a, &mut b), 0);
            assert!(a >= t0 && b != 0);
            assert_eq!(set_pair(h, 1, 2), 0);
            assert_eq!(xgs_kmd_emu::ported::process_options().host_posts.get(&1), Some(&2));
            assert_eq!(allocate(h, 0, 0, 0, 0, &mut a, &mut b, null_mut()), NOT_SUPPORTED);
            assert_eq!(query_fresh(0, 0, &mut a, null_mut()), NOT_SUPPORTED);
            for f in [c20b as usize, c20f as usize, c1cb as usize, c1a7 as usize, c237 as usize, c233 as usize, c273 as usize] {
                let f: unsafe extern "system" fn(*const c_void) -> i32 = core::mem::transmute(f);
                assert_eq!(f(buf.as_ptr().cast()), STATUS_UNSUCCESSFUL);
            }
            for f in [c217 as usize, c1a3 as usize] {
                let f: unsafe extern "system" fn(*mut c_void) -> i32 = core::mem::transmute(f);
                assert_eq!(f(out.as_mut_ptr().cast()), STATUS_UNSUCCESSFUL);
            }
            assert_eq!(c207(1, 2), STATUS_UNSUCCESSFUL);
            assert_eq!(c1bb(buf.as_ptr().cast(), out.as_mut_ptr().cast()), STATUS_UNSUCCESSFUL);
            assert_eq!(c23f(buf.as_ptr().cast(), out.as_mut_ptr().cast()), STATUS_UNSUCCESSFUL);
            assert_eq!(c1af(1, 2, 3, 4), STATUS_UNSUCCESSFUL);
            assert_eq!(c24b(1, 2), STATUS_UNSUCCESSFUL);
            assert_eq!(c23b(1, 2, 3), STATUS_UNSUCCESSFUL);
            assert_eq!(c24f(0, buf.as_ptr().cast()), NOT_SUPPORTED);
            assert_eq!(D3D12XboxEnableMemoryProfiling(1, 2, 3), 0);
            assert_eq!(xgs_kmd_emu::ported::memory_profiling(), Some([1, 2, 3]));
            let code = [0u32; 16];
            let p = code.as_ptr() as usize;
            assert_eq!(D3D12UploadCustomMicrocodeX(1, p, 64, p, 1, 1, p), E_FAIL);
            assert_eq!(release_device(h), 0);
        }
        assert_eq!(cached(), INVALID);
        assert!(!xgs_kmd_emu::is_device(h));
    }

    #[test]
    fn microcode_upload_matches_console() {
        assert_eq!(core::mem::size_of::<MicrocodeUpload>(), 0x38);
        let m = MicrocodeUpload { kind: 1, pad: 0, c: 3, b: 2, e: 5, d: 4, g: 7, f: 6 };
        let w = unsafe { core::slice::from_raw_parts((&m as *const MicrocodeUpload).cast::<u64>(), 7) };
        assert_eq!(w, &[1, 3, 2, 5, 4, 7, 6]);
        let all = [1usize, 2, 3, 4, 5, 6, 7];
        for i in 0..7 {
            let mut a = all;
            a[i] = 0;
            assert_eq!(unsafe { D3D12UploadCustomMicrocodeX(a[0] as u32, a[1], a[2], a[3], a[4], a[5], a[6]) }, E_INVALIDARG);
        }
    }
}
