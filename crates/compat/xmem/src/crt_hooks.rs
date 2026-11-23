use core::sync::atomic::{AtomicUsize, Ordering};

use crate::heap_acct::{block_size, slot};
use crate::sys::{GetProcessHeap, Handle, HeapAlloc, HeapFree};
use crate::ww_heap::{self, Resize, GRAN};

pub const NAMES: [&[u8]; 13] = [b"malloc", b"calloc", b"realloc", b"free", b"_aligned_malloc", b"_aligned_realloc", b"_aligned_free", b"_strdup", b"_wcsdup", b"_msize", b"_expand", b"_recalloc", b"_aligned_msize"];

static ORIG: [AtomicUsize; 13] = [const { AtomicUsize::new(0) }; 13];

const HEAP_ZERO_MEMORY: u32 = 0x8;

fn orig(i: usize) -> usize {
    ORIG[i].load(Ordering::Acquire)
}

fn heap() -> usize {
    unsafe { GetProcessHeap() as usize }
}

unsafe fn aligned_raw(p: usize) -> usize {
    core::ptr::read_unaligned(((p & !7) - 8) as *const usize)
}

unsafe fn plain_size(p: usize) -> Option<usize> {
    block_size(heap(), 0, p)
}

unsafe fn aligned_size(p: usize) -> Option<usize> {
    if p < 16 {
        return None;
    }
    block_size(heap(), 0, aligned_raw(p))
}

fn add(n: Option<usize>) {
    if let (Some(n), Some(a)) = (n, slot(heap())) {
        a.add(n);
    }
}

fn sub(n: Option<usize>) {
    if let (Some(n), Some(a)) = (n, slot(heap())) {
        a.sub(n);
    }
}

unsafe fn sized(p: usize) -> Option<usize> {
    ww_heap::size_of(p).or_else(|| plain_size(p))
}

fn ww_new(n: usize, align: usize) -> Option<usize> {
    if align > GRAN || !ww_heap::wants(heap(), n) {
        return None;
    }
    let p = ww_heap::alloc(heap(), n)?;
    add(Some(n));
    Some(p)
}

unsafe fn ww_resize(p: usize, n: usize, zero: bool, fallback: impl FnOnce(usize) -> usize) -> Option<usize> {
    let b = ww_heap::lookup(p)?;
    if n == 0 {
        ww_heap::free(p);
        sub(Some(b.size));
        return Some(0);
    }
    match ww_heap::resize(p, n, zero, false, fallback) {
        Resize::InPlace(q) | Resize::Moved(q) => {
            sub(Some(b.size));
            add(sized(q));
            Some(q)
        }
        Resize::Failed => Some(0),
    }
}

unsafe fn ww_migrate(p: usize, usable: usize, n: usize, release: impl FnOnce(usize)) -> Option<usize> {
    if p == 0 || !ww_heap::wants(heap(), n) {
        return None;
    }
    let q = ww_heap::migrate(heap(), p, usable, n)?;
    let old = plain_size(p);
    release(p);
    sub(old);
    add(Some(n));
    Some(q)
}

unsafe fn real_malloc(n: usize) -> usize {
    HeapAlloc(GetProcessHeap(), 0, n.max(1)) as usize
}

unsafe fn real_free(p: usize) {
    HeapFree(GetProcessHeap() as Handle, 0, p as *mut core::ffi::c_void);
}

unsafe extern "C" fn malloc_w(n: usize) -> usize {
    if let Some(p) = ww_new(n, 16) {
        return p;
    }
    let r = core::mem::transmute::<usize, unsafe extern "C" fn(usize) -> usize>(orig(0))(n);
    if r != 0 {
        add(plain_size(r));
    }
    r
}

unsafe extern "C" fn calloc_w(n: usize, s: usize) -> usize {
    if let Some(p) = n.checked_mul(s).and_then(|t| ww_new(t, 16)) {
        return p;
    }
    let r = core::mem::transmute::<usize, unsafe extern "C" fn(usize, usize) -> usize>(orig(1))(n, s);
    if r != 0 {
        add(plain_size(r));
    }
    r
}

unsafe extern "C" fn realloc_w(p: usize, n: usize) -> usize {
    if let Some(q) = ww_resize(p, n, false, |m| real_malloc(m)) {
        return q;
    }
    if let Some(q) = plain_size(p).and_then(|o| ww_migrate(p, o, n, |x| real_free(x))) {
        return q;
    }
    let old = if p != 0 { plain_size(p) } else { None };
    let r = core::mem::transmute::<usize, unsafe extern "C" fn(usize, usize) -> usize>(orig(2))(p, n);
    if r != 0 {
        sub(old);
        add(plain_size(r));
    } else if n == 0 {
        sub(old);
    }
    r
}

unsafe extern "C" fn free_w(p: usize) {
    if let Some(s) = ww_heap::free(p) {
        sub(Some(s));
        return;
    }
    let old = if p != 0 { plain_size(p) } else { None };
    core::mem::transmute::<usize, unsafe extern "C" fn(usize)>(orig(3))(p);
    sub(old);
}

unsafe extern "C" fn aligned_malloc_w(n: usize, align: usize) -> usize {
    if align.is_power_of_two() {
        if let Some(p) = ww_new(n, align) {
            return p;
        }
    }
    let r = core::mem::transmute::<usize, unsafe extern "C" fn(usize, usize) -> usize>(orig(4))(n, align);
    if r != 0 {
        add(aligned_size(r));
    }
    r
}

unsafe extern "C" fn aligned_realloc_w(p: usize, n: usize, align: usize) -> usize {
    if ww_heap::is_block(p) {
        let fallback = |m: usize| core::mem::transmute::<usize, unsafe extern "C" fn(usize, usize) -> usize>(orig(4))(m, align);
        if align > GRAN {
            let Some(b) = ww_heap::lookup(p) else { return 0 };
            let q = if n == 0 { 0 } else { fallback(n) };
            if n != 0 && q == 0 {
                return 0;
            }
            if q != 0 {
                core::ptr::copy_nonoverlapping(p as *const u8, q as *mut u8, b.size.min(n));
                add(aligned_size(q));
            }
            ww_heap::free(p);
            sub(Some(b.size));
            return q;
        }
        return ww_resize(p, n, false, fallback).unwrap_or(0);
    }
    if p != 0 && align.is_power_of_two() && align <= GRAN {
        let raw = aligned_raw(p);
        let usable = plain_size(raw).map(|s| s.saturating_sub(p - raw));
        if let Some(u) = usable {
            let free_aligned = |x: usize| core::mem::transmute::<usize, unsafe extern "C" fn(usize)>(orig(6))(x);
            let old = aligned_size(p);
            if ww_heap::wants(heap(), n) {
                if let Some(q) = ww_heap::migrate(heap(), p, u, n) {
                    free_aligned(p);
                    sub(old);
                    add(Some(n));
                    return q;
                }
            }
        }
    }
    let old = if p != 0 { aligned_size(p) } else { None };
    let r = core::mem::transmute::<usize, unsafe extern "C" fn(usize, usize, usize) -> usize>(orig(5))(p, n, align);
    if r != 0 {
        sub(old);
        add(aligned_size(r));
    } else if n == 0 {
        sub(old);
    }
    r
}

unsafe extern "C" fn aligned_free_w(p: usize) {
    if let Some(s) = ww_heap::free(p) {
        sub(Some(s));
        return;
    }
    let old = if p != 0 { aligned_size(p) } else { None };
    core::mem::transmute::<usize, unsafe extern "C" fn(usize)>(orig(6))(p);
    sub(old);
}

unsafe extern "C" fn strdup_w(s: usize) -> usize {
    let r = core::mem::transmute::<usize, unsafe extern "C" fn(usize) -> usize>(orig(7))(s);
    if r != 0 {
        add(plain_size(r));
    }
    r
}

unsafe extern "C" fn wcsdup_w(s: usize) -> usize {
    let r = core::mem::transmute::<usize, unsafe extern "C" fn(usize) -> usize>(orig(8))(s);
    if r != 0 {
        add(plain_size(r));
    }
    r
}

unsafe extern "C" fn msize_w(p: usize) -> usize {
    if let Some(s) = ww_heap::size_of(p) {
        return s;
    }
    core::mem::transmute::<usize, unsafe extern "C" fn(usize) -> usize>(orig(9))(p)
}

unsafe extern "C" fn expand_w(p: usize, n: usize) -> usize {
    if let Some(b) = ww_heap::lookup(p) {
        if n == 0 || n > b.len {
            return 0;
        }
        return match ww_heap::resize(p, n, false, true, |_| 0) {
            Resize::InPlace(q) => {
                sub(Some(b.size));
                add(Some(n));
                q
            }
            _ => 0,
        };
    }
    let old = if p != 0 { plain_size(p) } else { None };
    let r = core::mem::transmute::<usize, unsafe extern "C" fn(usize, usize) -> usize>(orig(10))(p, n);
    if r != 0 {
        sub(old);
        add(plain_size(r));
    }
    r
}

unsafe extern "C" fn recalloc_w(p: usize, count: usize, size: usize) -> usize {
    let Some(n) = count.checked_mul(size) else {
        return core::mem::transmute::<usize, unsafe extern "C" fn(usize, usize, usize) -> usize>(orig(11))(p, count, size);
    };
    if let Some(q) = ww_resize(p, n, true, |m| HeapAlloc(GetProcessHeap(), HEAP_ZERO_MEMORY, m.max(1)) as usize) {
        return q;
    }
    if let Some(q) = plain_size(p).and_then(|o| ww_migrate(p, o, n, |x| real_free(x))) {
        return q;
    }
    let old = if p != 0 { plain_size(p) } else { None };
    let r = core::mem::transmute::<usize, unsafe extern "C" fn(usize, usize, usize) -> usize>(orig(11))(p, count, size);
    if r != 0 {
        sub(old);
        add(plain_size(r));
    } else if n == 0 {
        sub(old);
    }
    r
}

unsafe extern "C" fn aligned_msize_w(p: usize, align: usize, offset: usize) -> usize {
    if let Some(s) = ww_heap::size_of(p) {
        return s;
    }
    core::mem::transmute::<usize, unsafe extern "C" fn(usize, usize, usize) -> usize>(orig(12))(p, align, offset)
}

pub fn wrappers() -> [usize; 13] {
    [
        malloc_w as *const () as usize,
        calloc_w as *const () as usize,
        realloc_w as *const () as usize,
        free_w as *const () as usize,
        aligned_malloc_w as *const () as usize,
        aligned_realloc_w as *const () as usize,
        aligned_free_w as *const () as usize,
        strdup_w as *const () as usize,
        wcsdup_w as *const () as usize,
        msize_w as *const () as usize,
        expand_w as *const () as usize,
        recalloc_w as *const () as usize,
        aligned_msize_w as *const () as usize,
    ]
}

fn crt_dll(dll: &[u8]) -> bool {
    let d = dll.to_ascii_lowercase();
    d.starts_with(b"api-ms-win-crt-") || d.starts_with(b"ucrtbase")
}

pub fn crt_rule(dll: &[u8], name: Option<&[u8]>, value: usize) -> Option<usize> {
    if !crt_dll(dll) || wrappers().contains(&value) {
        return None;
    }
    let i = NAMES.iter().position(|n| Some(*n) == name)?;
    match ORIG[i].compare_exchange(0, value, Ordering::AcqRel, Ordering::Acquire) {
        Ok(_) => Some(wrappers()[i]),
        Err(v) if v == value => Some(wrappers()[i]),
        Err(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    extern "C" {
        fn malloc(n: usize) -> usize;
        fn calloc(n: usize, s: usize) -> usize;
        fn realloc(p: usize, n: usize) -> usize;
        fn free(p: usize);
        fn _aligned_malloc(n: usize, a: usize) -> usize;
        fn _aligned_realloc(p: usize, n: usize, a: usize) -> usize;
        fn _aligned_free(p: usize);
        fn _strdup(s: usize) -> usize;
        fn _msize(p: usize) -> usize;
        fn _expand(p: usize, n: usize) -> usize;
        fn _recalloc(p: usize, c: usize, s: usize) -> usize;
        fn _aligned_msize(p: usize, a: usize, o: usize) -> usize;
    }

    fn w(i: usize, real: usize) -> usize {
        let dll: &[u8] = if i == 7 { b"api-ms-win-crt-string-l1-1-0.dll" } else { b"api-ms-win-crt-heap-l1-1-0.dll" };
        crt_rule(dll, Some(NAMES[i]), real).expect("crt slot claims")
    }

    #[test]
    fn ucrt_heap_is_the_process_heap() {
        let p = unsafe { malloc(100) };
        assert_eq!(unsafe { plain_size(p) }, Some(100));
        let q = unsafe { _aligned_malloc(100, 64) };
        assert_eq!(q % 64, 0);
        assert!(unsafe { aligned_size(q) }.unwrap() >= 100);
        unsafe {
            free(p);
            _aligned_free(q);
        }
    }

    #[test]
    fn title_crt_calls_move_the_title_heap_and_return_to_zero() {
        let h = heap();
        crate::heap_acct::register_title(h, 0, 0).unwrap();
        let base = crate::heap_acct::local_delta(h);
        let m: unsafe extern "C" fn(usize) -> usize = unsafe { core::mem::transmute(w(0, malloc as *const () as usize)) };
        let c: unsafe extern "C" fn(usize, usize) -> usize = unsafe { core::mem::transmute(w(1, calloc as *const () as usize)) };
        let r: unsafe extern "C" fn(usize, usize) -> usize = unsafe { core::mem::transmute(w(2, realloc as *const () as usize)) };
        let f: unsafe extern "C" fn(usize) = unsafe { core::mem::transmute(w(3, free as *const () as usize)) };
        let am: unsafe extern "C" fn(usize, usize) -> usize = unsafe { core::mem::transmute(w(4, _aligned_malloc as *const () as usize)) };
        let ar: unsafe extern "C" fn(usize, usize, usize) -> usize = unsafe { core::mem::transmute(w(5, _aligned_realloc as *const () as usize)) };
        let af: unsafe extern "C" fn(usize) = unsafe { core::mem::transmute(w(6, _aligned_free as *const () as usize)) };
        let sd: unsafe extern "C" fn(usize) -> usize = unsafe { core::mem::transmute(w(7, _strdup as *const () as usize)) };
        let d = || crate::heap_acct::local_delta(h) - base;
        unsafe {
            let a = m(0x1000);
            assert_eq!(d(), 0x1000);
            let b = c(4, 0x100);
            assert_eq!(d(), 0x1400);
            let a2 = r(a, 0x3000);
            assert_eq!(d(), 0x3400);
            let x = r(0, 0x20);
            assert_eq!(d(), 0x3420);
            assert_eq!(r(x, 0), 0);
            assert_eq!(d(), 0x3400);
            f(a2);
            f(b);
            f(0);
            assert_eq!(d(), 0);
            let al = am(0x500, 256);
            assert_eq!(al % 256, 0);
            let raw = d();
            assert!(raw >= 0x500 && raw < 0x500 + 256 + 16, "{raw:#x}");
            let al2 = ar(al, 0x2000, 256);
            assert_eq!(al2 % 256, 0);
            assert!(d() >= 0x2000);
            af(al2);
            assert_eq!(d(), 0);
            let al3 = am(0x40, 64);
            assert_eq!(ar(al3, 0, 64), 0);
            assert_eq!(d(), 0);
            let s = sd(b"title\0".as_ptr() as usize);
            assert_eq!(d(), 6);
            f(s);
            assert_eq!(d(), 0);
        }
        assert!(crt_rule(b"KERNEL32.dll", Some(b"malloc"), 0x1234).is_none());
        assert!(crt_rule(b"api-ms-win-crt-heap-l1-1-0.dll", Some(b"malloc"), 0x1234).is_none());
        assert!(crt_rule(b"api-ms-win-crt-heap-l1-1-0.dll", Some(b"malloc"), wrappers()[0]).is_none());
    }

    #[test]
    fn large_title_crt_blocks_come_from_watched_memory_and_round_trip() {
        let _g = crate::ww_heap::test_lock();
        crate::ww_heap::set_for_test(true, 0x10_0000);
        let h = heap();
        crate::heap_acct::register_title(h, 0, 0).unwrap();
        let base = crate::heap_acct::local_delta(h);
        let d = || crate::heap_acct::local_delta(h) - base;
        let m: unsafe extern "C" fn(usize) -> usize = unsafe { core::mem::transmute(w(0, malloc as *const () as usize)) };
        let c: unsafe extern "C" fn(usize, usize) -> usize = unsafe { core::mem::transmute(w(1, calloc as *const () as usize)) };
        let r: unsafe extern "C" fn(usize, usize) -> usize = unsafe { core::mem::transmute(w(2, realloc as *const () as usize)) };
        let f: unsafe extern "C" fn(usize) = unsafe { core::mem::transmute(w(3, free as *const () as usize)) };
        let am: unsafe extern "C" fn(usize, usize) -> usize = unsafe { core::mem::transmute(w(4, _aligned_malloc as *const () as usize)) };
        let ar: unsafe extern "C" fn(usize, usize, usize) -> usize = unsafe { core::mem::transmute(w(5, _aligned_realloc as *const () as usize)) };
        let af: unsafe extern "C" fn(usize) = unsafe { core::mem::transmute(w(6, _aligned_free as *const () as usize)) };
        let ms: unsafe extern "C" fn(usize) -> usize = unsafe { core::mem::transmute(w(9, _msize as *const () as usize)) };
        let ex: unsafe extern "C" fn(usize, usize) -> usize = unsafe { core::mem::transmute(w(10, _expand as *const () as usize)) };
        let rc: unsafe extern "C" fn(usize, usize, usize) -> usize = unsafe { core::mem::transmute(w(11, _recalloc as *const () as usize)) };
        let ams: unsafe extern "C" fn(usize, usize, usize) -> usize = unsafe { core::mem::transmute(w(12, _aligned_msize as *const () as usize)) };
        unsafe {
            let a = m(0x20_0000);
            assert!(crate::ww_heap::is_block(a));
            assert_eq!(crate::write_watch::owner(a).0, crate::write_watch::OWNER_EMULATOR);
            assert_eq!(ms(a), 0x20_0000);
            assert_eq!(d(), 0x20_0000);
            *((a + 0x1F_FFFF) as *mut u8) = 9;
            let a2 = r(a, 0x40_0000);
            assert!(crate::ww_heap::is_block(a2));
            assert_eq!(*((a2 + 0x1F_FFFF) as *const u8), 9);
            assert_eq!(d(), 0x40_0000);
            assert_eq!(ex(a2, 0x40_8000), 0);
            assert_eq!(ex(a2, 0x3F_0000), a2);
            assert_eq!(ms(a2), 0x3F_0000);
            assert_eq!(d(), 0x3F_0000);
            let z = rc(a2, 2, 0x20_0000);
            assert!(crate::ww_heap::is_block(z));
            assert_eq!(*((z + 0x3F_0000) as *const u8), 0);
            assert_eq!(d(), 0x40_0000);
            f(z);
            assert_eq!(d(), 0);
            let small = m(0x100);
            assert!(!crate::ww_heap::is_block(small));
            let grown = r(small, 0x18_0000);
            assert!(crate::ww_heap::is_block(grown));
            assert_eq!(d(), 0x18_0000);
            let back = r(grown, 0);
            assert_eq!(back, 0);
            assert_eq!(d(), 0);
            let cz = c(0x10, 0x1_0000);
            assert!(crate::ww_heap::is_block(cz));
            assert!(core::slice::from_raw_parts(cz as *const u8, 0x10_0000).iter().all(|&x| x == 0));
            f(cz);
            let al = am(0x10_0000, 4096);
            assert!(crate::ww_heap::is_block(al) && al % 4096 == 0);
            assert_eq!(ams(al, 4096, 0), 0x10_0000);
            let al2 = ar(al, 0x30_0000, 4096);
            assert!(crate::ww_heap::is_block(al2) && al2 % 4096 == 0);
            af(al2);
            let big_align = am(0x10_0000, 0x2_0000);
            assert!(!crate::ww_heap::is_block(big_align) && big_align % 0x2_0000 == 0);
            af(big_align);
            let plain_al = am(0x100, 64);
            let migrated = ar(plain_al, 0x20_0000, 64);
            assert!(crate::ww_heap::is_block(migrated));
            af(migrated);
            assert_eq!(d(), 0);
            crate::ww_heap::set_for_test(false, 0x10_0000);
            let off = m(0x20_0000);
            assert!(!crate::ww_heap::is_block(off));
            f(off);
            crate::ww_heap::set_for_test(true, 0x10_0000);
            assert_eq!(d(), 0);
        }
    }
}
