use crate::xgs_kmd::{self as kmd, E_FAIL, E_INVALIDARG, INVALID, STATUS_UNSUCCESSFUL};
use core::ffi::c_void;
use core::ptr::{null, null_mut};

pub const E_POINTER: i32 = 0x8000_4003u32 as i32;
pub const TABLE_SIZE: usize = 0x110;

unsafe extern "system" fn open_device(out: *mut isize) -> i32 {
    kmd::open(out, true)
}

unsafe extern "system" fn close_device(h: isize) -> i32 {
    kmd::release(h)
}

unsafe extern "system" fn close_other(h: isize) -> i32 {
    if h != kmd::cached() {
        kmd::close_handle(h);
    }
    0
}

unsafe fn query(h: isize, code: u32, inb: *const c_void, inl: u32, outb: *mut c_void, outl: u32) -> Result<(), i32> {
    if h == INVALID {
        return Err(E_INVALIDARG);
    }
    let mut ret = 0u32;
    if kmd::ioctl(h, code, inb, inl, outb, outl, &mut ret) {
        if ret != outl {
            return Err(E_FAIL);
        }
        return Ok(());
    }
    let hr = kmd::last_error();
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
    if !kmd::ioctl(h, 0x7900_c1a8, null(), 0, (&mut v as *mut u32).cast(), 4, &mut ret) {
        return kmd::last_error();
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
    let f = kmd::create();
    if f == INVALID {
        return kmd::last_error();
    }
    let mut o = [0u64; 2];
    let r = query(f, 0x7900_c1b0, (&a as *const u32).cast(), 4, o.as_mut_ptr().cast(), 0x10);
    kmd::close_handle(f);
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

unsafe extern "system" fn unsupported() -> i32 {
    STATUS_UNSUCCESSFUL
}

unsafe extern "system" fn no_op() -> i32 {
    0
}

unsafe extern "system" fn c20b(p: *const c_void) -> i32 {
    kmd::call_cached(0x7900_c20b, p, 0x20, null_mut(), 0)
}

unsafe extern "system" fn c207(a: u32, b: u32) -> i32 {
    let inb = [a, b];
    kmd::call_cached(0x7900_c207, inb.as_ptr().cast(), 8, null_mut(), 0)
}

unsafe extern "system" fn c20f(p: *const c_void) -> i32 {
    kmd::call_cached(0x7900_c20f, p, 0x20, null_mut(), 0)
}

unsafe extern "system" fn c1bb(p: *const c_void, out: *mut c_void) -> i32 {
    kmd::call_cached(0x7900_c1bb, p, 0xd8, out, 0x10)
}

unsafe extern "system" fn c1cb(p: *const c_void) -> i32 {
    kmd::call_cached(0x7900_c1cb, p, 0x20, null_mut(), 0)
}

unsafe extern "system" fn c23f(p: *const c_void, out: *mut c_void) -> i32 {
    kmd::call_opened(0x7900_c23f, p, 4, out, 0x90)
}

unsafe extern "system" fn c21b(p: *const c_void) -> i32 {
    kmd::call_opened(0x7900_c21b, p, 0x18, null_mut(), 0)
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
    kmd::call_opened(0x7900_c1af, (&q as *const Quad).cast(), 0x18, null_mut(), 0)
}

#[repr(C)]
struct Pair {
    a: u32,
    pad: u32,
    b: u64,
}

unsafe extern "system" fn c24b(a: u32, b: u64) -> i32 {
    let p = Pair { a, pad: 0, b };
    kmd::call_cached(0x7900_c24b, (&p as *const Pair).cast(), 0x10, null_mut(), 0)
}

unsafe extern "system" fn c1a7(p: *const c_void) -> i32 {
    kmd::call_cached(0x7900_c1a7, p, 0x4028, null_mut(), 0)
}

unsafe extern "system" fn c237(p: *const c_void) -> i32 {
    kmd::call_cached(0x7900_c237, p, 0x20, null_mut(), 0)
}

unsafe extern "system" fn c233(p: *const c_void) -> i32 {
    kmd::call_cached(0x7900_c233, p, 0x40, null_mut(), 0)
}

unsafe extern "system" fn c26f(p: *const c_void) -> i32 {
    kmd::call_cached(0x7900_c26f, p, 0x40, null_mut(), 0)
}

unsafe extern "system" fn c23b(a: u64, b: u32, c: u64) -> i32 {
    let q = Quad { a, b, c: 0, d: c };
    kmd::call_cached(0x7900_c23b, (&q as *const Quad).cast(), 0x18, null_mut(), 0)
}

unsafe extern "system" fn c273(p: *const c_void) -> i32 {
    kmd::call_device(0x7900_c273, p, 0x18)
}

unsafe extern "system" fn c217(out: *mut c_void) -> i32 {
    kmd::call_cached(0x7900_c217, null(), 0, out, 0x40)
}

unsafe extern "system" fn c27f(p: *const c_void) -> i32 {
    kmd::call_opened(0x7900_c27f, p, 4, null_mut(), 0)
}

fn entries() -> [(usize, usize); 29] {
    [
        (0x08, open_device as *const () as usize),
        (0x10, close_device as *const () as usize),
        (0x18, set_pair as *const () as usize),
        (0x20, set_counters as *const () as usize),
        (0x30, read_u32 as *const () as usize),
        (0x38, read_pair as *const () as usize),
        (0x40, c20b as *const () as usize),
        (0x48, c207 as *const () as usize),
        (0x58, c20f as *const () as usize),
        (0x60, c1bb as *const () as usize),
        (0x68, unsupported as *const () as usize),
        (0x70, c1cb as *const () as usize),
        (0x80, c23f as *const () as usize),
        (0x88, c21b as *const () as usize),
        (0x90, no_op as *const () as usize),
        (0x98, c1af as *const () as usize),
        (0xa8, c24b as *const () as usize),
        (0xb0, allocate as *const () as usize),
        (0xb8, query_fresh as *const () as usize),
        (0xc0, read_triple as *const () as usize),
        (0xc8, c1a7 as *const () as usize),
        (0xd0, close_other as *const () as usize),
        (0xd8, c237 as *const () as usize),
        (0xe0, c233 as *const () as usize),
        (0xe8, c26f as *const () as usize),
        (0xf0, c23b as *const () as usize),
        (0xf8, c273 as *const () as usize),
        (0x100, c217 as *const () as usize),
        (0x108, c27f as *const () as usize),
    ]
}

#[no_mangle]
pub unsafe extern "system" fn PixGetPixieEntryPoints(table: *mut u8) -> i32 {
    if table.is_null() {
        return E_POINTER;
    }
    let size = (table as *const u32).read_unaligned();
    let n = (size as usize).min(TABLE_SIZE);
    let mut local = [0u8; TABLE_SIZE];
    local[..4].copy_from_slice(&size.to_le_bytes());
    for (off, f) in entries() {
        local[off..off + 8].copy_from_slice(&(f as u64).to_le_bytes());
    }
    core::ptr::copy_nonoverlapping(local.as_ptr(), table, n);
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table() -> [u64; TABLE_SIZE / 8] {
        let mut t = [0u64; TABLE_SIZE / 8];
        t[0] = TABLE_SIZE as u64;
        assert_eq!(unsafe { PixGetPixieEntryPoints(t.as_mut_ptr().cast()) }, 0);
        t
    }

    #[test]
    fn null_table_is_e_pointer() {
        assert_eq!(unsafe { PixGetPixieEntryPoints(null_mut()) }, E_POINTER);
    }

    #[test]
    fn full_table_has_console_layout() {
        let t = table();
        assert_eq!(t[0] as u32, TABLE_SIZE as u32);
        let filled: Vec<usize> = (1..TABLE_SIZE / 8).filter(|&i| t[i] != 0).map(|i| i * 8).collect();
        let expect: Vec<usize> = entries().iter().map(|e| e.0).collect();
        let mut sorted = expect.clone();
        sorted.sort();
        assert_eq!(filled, sorted);
        for off in [0x28usize, 0x50, 0x78, 0xa0] {
            assert_eq!(t[off / 8], 0);
        }
    }

    #[test]
    fn copy_is_limited_to_caller_size() {
        let mut t = [0xaaaa_aaaa_aaaa_aaaau64; 6];
        t[0] = 0x18;
        assert_eq!(unsafe { PixGetPixieEntryPoints(t.as_mut_ptr().cast()) }, 0);
        assert_eq!(t[0] as u32, 0x18);
        assert_ne!(t[1], 0);
        assert_ne!(t[2], 0);
        assert_eq!(t[3], 0xaaaa_aaaa_aaaa_aaaa);
        let mut big = [0u64; 0x30];
        big[0] = 0x1000;
        assert_eq!(unsafe { PixGetPixieEntryPoints(big.as_mut_ptr().cast()) }, 0);
        assert_eq!(big[0] as u32, 0x1000);
        assert_eq!(big[TABLE_SIZE / 8], 0);
    }

    #[test]
    fn fixed_entries_return_console_values() {
        let t = table();
        let un: unsafe extern "system" fn() -> i32 = unsafe { core::mem::transmute(t[0x68 / 8] as usize) };
        let ok: unsafe extern "system" fn() -> i32 = unsafe { core::mem::transmute(t[0x90 / 8] as usize) };
        assert_eq!(unsafe { un() }, STATUS_UNSUCCESSFUL);
        assert_eq!(unsafe { ok() }, 0);
    }

    #[test]
    fn handle_entries_validate_like_console() {
        let mut a = 0u64;
        let mut b = 0u64;
        let mut c = 0u32;
        let mut d = 0u32;
        unsafe {
            assert_eq!(set_pair(INVALID, 1, 2), E_INVALIDARG);
            assert_eq!(set_counters(INVALID, 0x21, null(), null(), null()), E_INVALIDARG);
            assert_eq!(set_counters(INVALID, 0, null(), null(), null()), E_INVALIDARG);
            assert_eq!(read_u32(INVALID, &mut c), E_INVALIDARG);
            assert_eq!(read_pair(INVALID, &mut a, &mut b), E_INVALIDARG);
            assert_eq!(read_triple(INVALID, &mut a, &mut c, &mut d), E_INVALIDARG);
            assert_eq!(allocate(INVALID, 0, 0, 0, 0, null_mut(), &mut b, null_mut()), E_INVALIDARG);
            assert_eq!(allocate(INVALID, 0, 0, 0, 0, &mut a, null_mut(), null_mut()), E_INVALIDARG);
            assert_eq!(allocate(INVALID, 0, 3, 0x8000_0000, 0, &mut a, null_mut(), null_mut()), E_INVALIDARG);
            assert_eq!(allocate(INVALID, 0, 0, 0x4000_0000, 5, &mut a, &mut b, null_mut()), E_INVALIDARG);
            assert_eq!(allocate(INVALID, 0, 0, 0x2000_0004, 0, &mut a, &mut b, null_mut()), E_INVALIDARG);
            assert_eq!(allocate(INVALID, 0, 0x100, 0x4000_0000, 0, &mut a, &mut b, null_mut()), E_INVALIDARG);
            assert_eq!(query_fresh(1, 0, &mut a, null_mut()), E_INVALIDARG);
            assert_eq!(query_fresh(0, 0, null_mut(), null_mut()), E_INVALIDARG);
        }
    }

    #[test]
    fn device_entries_use_the_emulated_kmd() {
        let _g = kmd::test_serial();
        let t = table();
        let open: unsafe extern "system" fn(*mut isize) -> i32 = unsafe { core::mem::transmute(t[0x08 / 8] as usize) };
        let triple: unsafe extern "system" fn(isize, *mut u64, *mut u32, *mut u32) -> i32 = unsafe { core::mem::transmute(t[0xc0 / 8] as usize) };
        assert_eq!(unsafe { open(null_mut()) }, E_INVALIDARG);
        let mut h = INVALID;
        assert_eq!(unsafe { open(&mut h) }, 0);
        assert_ne!(h, INVALID);
        assert_eq!(kmd::cached(), h);
        let (mut f, mut m, mut cu) = (0u64, 0u32, 0u32);
        assert_eq!(unsafe { triple(h, &mut f, &mut m, &mut cu) }, 0);
        let r = xgs_kmd_emu::gpu_reply(xgs_kmd_emu::device_type());
        assert_eq!(f.to_le_bytes(), r[0..8]);
        assert_eq!(m.to_le_bytes(), r[8..12]);
        assert_eq!(cu.to_le_bytes(), r[12..16]);
        let not_supported = 0x8007_0032u32 as i32;
        let buf = [0u8; 0x4028];
        let mut out = [0u8; 0x90];
        let (mut a, mut b, mut c) = (0u64, 0u64, 0u32);
        unsafe {
            assert_eq!(read_u32(h, &mut c), not_supported);
            let t0 = core::arch::x86_64::_rdtsc();
            assert_eq!(read_pair(h, &mut a, &mut b), 0);
            assert!(a >= t0 && a <= core::arch::x86_64::_rdtsc());
            assert!(b != 0);
            assert_eq!(query_fresh(0, 0, &mut a, null_mut()), not_supported);
            assert_eq!(c20b(buf.as_ptr().cast()), STATUS_UNSUCCESSFUL);
            assert_eq!(c207(1, 2), STATUS_UNSUCCESSFUL);
            assert_eq!(c1bb(buf.as_ptr().cast(), out.as_mut_ptr().cast()), STATUS_UNSUCCESSFUL);
            assert_eq!(c23f(buf.as_ptr().cast(), out.as_mut_ptr().cast()), STATUS_UNSUCCESSFUL);
            assert_eq!(c1af(1, 2, 3, 4), STATUS_UNSUCCESSFUL);
            assert_eq!(c24b(1, 2), STATUS_UNSUCCESSFUL);
            assert_eq!(c1a7(buf.as_ptr().cast()), STATUS_UNSUCCESSFUL);
            assert_eq!(c23b(1, 2, 3), STATUS_UNSUCCESSFUL);
            assert_eq!(c273(buf.as_ptr().cast()), STATUS_UNSUCCESSFUL);
            assert_eq!(c217(out.as_mut_ptr().cast()), STATUS_UNSUCCESSFUL);
            let mut req = [0xffu8; 0x10];
            xgs_kmd_emu::ported::set_gpu_error(0, "");
            assert_eq!(c27f(req.as_mut_ptr().cast()), 0);
            assert_eq!(&req[12..16], &[0; 4]);
            assert_eq!(close_other(h), 0);
            assert!(xgs_kmd_emu::is_device(h));
            assert_eq!(close_device(h), 0);
            assert_eq!(close_device(INVALID), 0);
        }
        assert_eq!(kmd::cached(), INVALID);
        assert!(!xgs_kmd_emu::is_device(h));
        assert_eq!(unsafe { triple(h, &mut f, &mut m, &mut cu) }, 0x8007_0006u32 as i32);
    }
}
