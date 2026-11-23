use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use crate::sys::*;

const DEFAULT_MIN: usize = 0x10_0000;
const PLAIN: u32 = MEM_RESERVE | MEM_COMMIT | MEM_TOP_DOWN;
const PROTECT_MODIFIERS: u32 = 0x200 | 0x400;

static ON: AtomicUsize = AtomicUsize::new(2);
static MIN: AtomicUsize = AtomicUsize::new(0);
static SERVED: AtomicUsize = AtomicUsize::new(0);
static BYTES: AtomicU64 = AtomicU64::new(0);

#[cfg(test)]
thread_local! {
    static TEST: core::cell::Cell<(bool, usize)> = const { core::cell::Cell::new((false, DEFAULT_MIN)) };
}

#[cfg(test)]
pub fn enabled() -> bool {
    TEST.with(|t| t.get().0) && crate::write_watch::wanted()
}

#[cfg(test)]
pub fn min_size() -> usize {
    TEST.with(|t| t.get().1)
}

#[cfg(not(test))]
pub fn enabled() -> bool {
    match ON.load(Ordering::Relaxed) {
        0 => false,
        1 => true,
        _ => {
            let on = std::env::var("TOMBOX_XMEM_WW_VALLOC").map(|v| v.trim() == "1").unwrap_or(false) && crate::write_watch::wanted();
            ON.store(on as usize, Ordering::Relaxed);
            on
        }
    }
}

#[cfg(not(test))]
pub fn min_size() -> usize {
    let m = MIN.load(Ordering::Relaxed);
    if m != 0 {
        return m;
    }
    let v = std::env::var("TOMBOX_XMEM_WW_VALLOC_MIN").ok().and_then(|s| crate::ww_heap::parse_size(&s)).unwrap_or(DEFAULT_MIN).max(0x1_0000);
    MIN.store(v, Ordering::Relaxed);
    v
}

pub fn served() -> usize {
    SERVED.load(Ordering::Relaxed)
}

pub fn wants(addr: usize, size: usize, kind: u32, protect: u32) -> bool {
    addr == 0 && kind & MEM_RESERVE != 0 && kind & !PLAIN == 0 && protect & !PROTECT_MODIFIERS == PAGE_READWRITE && enabled() && size >= min_size()
}

pub fn alloc(size: usize, kind: u32, protect: u32) -> usize {
    let p = crate::write_watch::reserve(size, 0, kind, protect) as usize;
    if p == 0 || crate::write_watch::owner(p).0 != crate::write_watch::OWNER_EMULATOR {
        return p;
    }
    BYTES.fetch_add(size as u64, Ordering::Relaxed);
    let n = SERVED.fetch_add(1, Ordering::Relaxed);
    if n == 0 || (n + 1) % 256 == 0 {
        compat_common::record_raw(
            "xmem",
            "ww_valloc",
            &format!("title VirtualAlloc reservations >= {:#x} given write watch (owner=emulator): {} served, {:.1}MB reserved in total (opt-in via TOMBOX_XMEM_WW_VALLOC=1)", min_size(), n + 1, BYTES.load(Ordering::Relaxed) as f64 / (1u64 << 20) as f64),
        );
    }
    p
}

#[cfg(test)]
pub fn set_for_test(on: bool, min: usize) {
    TEST.with(|t| t.set((on, min)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::ffi::c_void;
    use crate::write_watch::{owner, OWNER_EMULATOR, OWNER_NONE, OWNER_TITLE};

    fn valloc(addr: usize, size: usize, kind: u32, protect: u32) -> usize {
        let f: unsafe extern "system" fn(usize, usize, u32, u32) -> usize = unsafe { core::mem::transmute(crate::hooks::valloc_wrapper()) };
        unsafe { f(addr, size, kind, protect) }
    }

    fn release(p: usize) {
        let f: unsafe extern "system" fn(usize, usize, u32) -> i32 = unsafe { core::mem::transmute(crate::hooks::vfree_wrapper()) };
        assert_ne!(unsafe { f(p, 0, MEM_RELEASE) }, 0);
    }

    fn dirty(p: usize, n: usize) -> Option<Vec<usize>> {
        let mut a = vec![core::ptr::null_mut::<c_void>(); n / 0x1000];
        let mut c = a.len();
        let mut g = 0u32;
        let r = unsafe { GetWriteWatch(0, p as *mut c_void, n, a.as_mut_ptr(), &mut c, &mut g) };
        (r == 0).then(|| a[..c].iter().map(|&x| x as usize - p).collect())
    }

    #[test]
    fn large_title_reservations_get_an_emulator_owned_write_watch() {
        if !crate::write_watch::wanted() {
            return;
        }
        set_for_test(true, 0x10_0000);
        let n = 0x2ab400;
        let p = valloc(0, n, MEM_RESERVE | MEM_COMMIT, PAGE_READWRITE);
        assert_ne!(p, 0);
        let (o, b, len) = owner(p + 0x1234);
        assert_eq!((o, b, len), (OWNER_EMULATOR, p, n));
        assert_eq!(dirty(p, 0x2ab000), Some(vec![]));
        unsafe { *((p + 0x20_0010) as *mut u8) = 9 };
        assert_eq!(dirty(p, 0x2ab000), Some(vec![0x20_0000]));
        release(p);
        assert_eq!(owner(p).0, OWNER_NONE);
        set_for_test(false, 0x10_0000);
    }

    #[test]
    fn small_odd_or_switched_off_reservations_are_left_to_the_os() {
        set_for_test(true, 0x10_0000);
        let small = valloc(0, 0x8_0000, MEM_RESERVE | MEM_COMMIT, PAGE_READWRITE);
        assert_ne!(small, 0);
        assert_eq!(owner(small).0, OWNER_NONE);
        assert_eq!(dirty(small, 0x8_0000), None);
        release(small);
        let exec = valloc(0, 0x20_0000, MEM_RESERVE | MEM_COMMIT, 0x40);
        assert_ne!(exec, 0);
        assert_eq!(owner(exec).0, OWNER_NONE);
        release(exec);
        let titled = valloc(0, 0x20_0000, MEM_RESERVE | MEM_COMMIT | MEM_WRITE_WATCH, PAGE_READWRITE);
        assert_ne!(titled, 0);
        assert_eq!(owner(titled), (OWNER_TITLE, titled, 0x20_0000));
        release(titled);
        set_for_test(false, 0x10_0000);
        let off = valloc(0, 0x20_0000, MEM_RESERVE | MEM_COMMIT, PAGE_READWRITE);
        assert_ne!(off, 0);
        assert_eq!(owner(off).0, OWNER_NONE);
        assert_eq!(dirty(off, 0x20_0000), None);
        release(off);
    }

    #[test]
    fn reserve_then_commit_keeps_the_watch() {
        if !crate::write_watch::wanted() {
            return;
        }
        set_for_test(true, 0x10_0000);
        let n = 0x40_0000;
        let p = valloc(0, n, MEM_RESERVE, PAGE_READWRITE);
        assert_ne!(p, 0);
        assert_eq!(owner(p).0, OWNER_EMULATOR);
        assert_eq!(valloc(p + 0x10_0000, 0x1_0000, MEM_COMMIT, PAGE_READWRITE), p + 0x10_0000);
        unsafe { *((p + 0x10_0008) as *mut u8) = 1 };
        assert_eq!(dirty(p, n), Some(vec![0x10_0000]));
        release(p);
        set_for_test(false, 0x10_0000);
    }
}
