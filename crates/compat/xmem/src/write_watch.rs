use core::ffi::c_void;
use std::sync::Mutex;

use crate::sys::*;

pub const OWNER_NONE: u32 = 0;
pub const OWNER_EMULATOR: u32 = 1;
pub const OWNER_TITLE: u32 = 2;

static REGIONS: Mutex<Vec<(usize, usize, u32, bool)>> = Mutex::new(Vec::new());

pub fn wanted() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("TOMBOX_WRITE_WATCH").map(|v| v.trim() != "0").unwrap_or(true))
}

fn note(base: usize, len: usize, owner: u32) {
    let mut r = REGIONS.lock().unwrap_or_else(|e| e.into_inner());
    r.retain(|&(b, n, _, _)| b + n <= base || base + len <= b);
    let at = r.partition_point(|&(b, _, _, _)| b < base);
    r.insert(at, (base, len, owner, true));
}

pub fn reserve_at(base: usize, len: usize, align: usize, alloc_type: u32, protect: u32) -> *mut c_void {
    let title = alloc_type & MEM_WRITE_WATCH != 0;
    let add = !title && wanted() && alloc_type & MEM_RESERVE != 0;
    let go = |t: u32| -> *mut c_void {
        if base == 0 && align > 0x1_0000 {
            reserve_aligned(len, align, t, protect)
        } else {
            unsafe { VirtualAlloc(base as *mut c_void, len, t, protect) }
        }
    };
    let mut p = if add { go(alloc_type | MEM_WRITE_WATCH) } else { core::ptr::null_mut() };
    let watched = !p.is_null();
    if p.is_null() {
        p = go(alloc_type);
    }
    if !p.is_null() && alloc_type & MEM_RESERVE != 0 {
        if title {
            note(p as usize, len, OWNER_TITLE);
        } else if watched {
            note(p as usize, len, OWNER_EMULATOR);
        }
    }
    p
}

pub fn forget(base: usize) {
    let mut r = REGIONS.lock().unwrap_or_else(|e| e.into_inner());
    r.retain(|&(b, _, _, _)| b != base);
}

pub fn note_title(base: usize, len: usize) {
    note(base, len, OWNER_TITLE);
}

pub fn forget_range(addr: usize, size: usize) {
    let end = addr.saturating_add(size.max(1));
    let mut r = REGIONS.lock().unwrap_or_else(|e| e.into_inner());
    r.retain(|&(b, n, _, _)| b + n <= addr || end <= b);
}

pub fn invalidate_range(addr: usize, size: usize) {
    let end = addr.saturating_add(size.max(1));
    let mut r = REGIONS.lock().unwrap_or_else(|e| e.into_inner());
    for e in r.iter_mut() {
        if e.0 < end && addr < e.0 + e.1 {
            e.3 = false;
        }
    }
}

fn still_watched(base: usize, len: usize) -> bool {
    if alloc_base(base + len - 1) != Some(base) {
        return false;
    }
    let mut a = [core::ptr::null_mut::<c_void>(); 1];
    let mut c = 1usize;
    let mut g = 0u32;
    unsafe { GetWriteWatch(0, base as *mut c_void, 0x1000, a.as_mut_ptr(), &mut c, &mut g) == 0 }
}

pub fn reserve(len: usize, align: usize, alloc_type: u32, protect: u32) -> *mut c_void {
    reserve_at(0, len, align, alloc_type, protect)
}

fn alloc_base(addr: usize) -> Option<usize> {
    let mut m = MemoryBasicInformation::default();
    let n = unsafe { VirtualQueryEx(GetCurrentProcess(), addr as *const c_void, &mut m, core::mem::size_of::<MemoryBasicInformation>()) };
    if n == 0 || m.state == 0x1_0000 {
        None
    } else {
        Some(m.alloc_base)
    }
}

pub fn owner(addr: usize) -> (u32, usize, usize) {
    let mut r = REGIONS.lock().unwrap_or_else(|e| e.into_inner());
    let i = r.partition_point(|&(b, _, _, _)| b <= addr);
    if i == 0 {
        return (OWNER_NONE, 0, 0);
    }
    let (b, n, o, ok) = r[i - 1];
    if addr >= b + n {
        return (OWNER_NONE, 0, 0);
    }
    if alloc_base(addr) != Some(b) || (!ok && !still_watched(b, n)) {
        r.remove(i - 1);
        return (OWNER_NONE, 0, 0);
    }
    r[i - 1].3 = true;
    (o, b, n)
}

#[no_mangle]
pub unsafe extern "system" fn TomboxWriteWatchOwner(addr: usize, base: *mut usize, len: *mut usize) -> u32 {
    let (o, b, n) = owner(addr);
    if !base.is_null() {
        *base = b;
    }
    if !len.is_null() {
        *len = n;
    }
    o
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(p: usize) {
        let vfree: unsafe extern "system" fn(usize, usize, u32) -> i32 = unsafe { core::mem::transmute(crate::hooks::vfree_wrapper()) };
        assert_ne!(unsafe { vfree(p, 0, MEM_RELEASE) }, 0);
    }

    fn bytes(p: usize) -> String {
        let mut h = [0u8; 0x84];
        let ok = crate::iat::copy_out(p, &mut h);
        let mut m = MemoryBasicInformation::default();
        unsafe { VirtualQueryEx(GetCurrentProcess(), p as *const c_void, &mut m, core::mem::size_of::<MemoryBasicInformation>()) };
        format!("read={ok} b0={:02x}{:02x} lfanew={:08x} pe={:02x?} alloc_base={:#x} state={:#x} kind={:#x}", h[0], h[1], u32::from_le_bytes(h[0x3c..0x40].try_into().unwrap()), &h[0x80..0x84], m.alloc_base, m.state, m.kind)
    }

    fn dirty(p: usize, n: usize) -> Option<Vec<usize>> {
        let mut a = vec![core::ptr::null_mut::<c_void>(); n / 0x1000];
        let mut c = a.len();
        let mut g = 0u32;
        let r = unsafe { GetWriteWatch(1, p as *mut c_void, n, a.as_mut_ptr(), &mut c, &mut g) };
        (r == 0).then(|| a[..c].iter().map(|&x| x as usize - p).collect())
    }

    #[test]
    fn emulator_reservations_are_watched_and_owned() {
        let n = 0x2_0000;
        let p = reserve(n, 0x1_0000, MEM_RESERVE | MEM_COMMIT, PAGE_READWRITE) as usize;
        assert_ne!(p, 0);
        let want = if wanted() { OWNER_EMULATOR } else { OWNER_NONE };
        assert_eq!(owner(p + 0x1234), if wanted() { (want, p, n) } else { (OWNER_NONE, 0, 0) });
        if wanted() {
            assert_eq!(dirty(p, n), Some(vec![]), "{}", bytes(p));
            unsafe { *((p + 0x5010) as *mut u8) = 1 };
            assert_eq!(dirty(p, n), Some(vec![0x5000]), "{}", bytes(p));
        }
        release(p);
        let o = owner(p + 0x1234).0;
        assert!(o == OWNER_NONE || alloc_base(p).is_some(), "owner {o} for a released and unreused range");
    }

    #[test]
    fn a_hooked_alloc_into_a_recorded_range_forces_revalidation() {
        let n = 0x2_0000;
        let p = reserve(n, 0x1_0000, MEM_RESERVE | MEM_COMMIT, PAGE_READWRITE) as usize;
        assert_ne!(p, 0);
        if !wanted() {
            release(p);
            return;
        }
        assert_eq!(owner(p).0, OWNER_EMULATOR);
        unsafe { VirtualFree(p as *mut c_void, 0, MEM_RELEASE) };
        let valloc: unsafe extern "system" fn(usize, usize, u32, u32) -> usize = unsafe { core::mem::transmute(crate::hooks::valloc_wrapper()) };
        let q = unsafe { valloc(p, n, MEM_RESERVE | MEM_COMMIT, PAGE_READWRITE) };
        if q == p {
            assert_eq!(owner(p + 0x10).0, OWNER_NONE);
            release(q);
        }
    }

    #[test]
    #[ignore]
    fn probe_which_reads_mark_a_fresh_page_dirty() {
        let n = 0x2_0000;
        for (name, how) in [("plain read", 0u8), ("ReadProcessMemory", 1u8), ("VirtualQuery", 2u8)] {
            let p = reserve(n, 0x1_0000, MEM_RESERVE | MEM_COMMIT, PAGE_READWRITE) as usize;
            let before = dirty(p, n);
            match how {
                0 => unsafe { core::hint::black_box(core::ptr::read_volatile(p as *const u8)); },
                1 => { let mut h = [0u8; 0x40]; core::hint::black_box(crate::iat::copy_out(p, &mut h)); }
                _ => { core::hint::black_box(alloc_base(p)); }
            }
            println!("probe {name}: before={before:?} after={:?}", dirty(p, n));
            release(p);
        }
    }

    #[test]
    #[ignore]
    fn owner_lookup_cost() {
        let n = 0x2_0000;
        let p = reserve(n, 0x1_0000, MEM_RESERVE | MEM_COMMIT, PAGE_READWRITE) as usize;
        assert_ne!(p, 0);
        let k = 200_000u32;
        let t = std::time::Instant::now();
        for _ in 0..k {
            core::hint::black_box(alloc_base(core::hint::black_box(p + 0x10)));
        }
        let before = t.elapsed().as_nanos() as f64 / k as f64;
        let t = std::time::Instant::now();
        for _ in 0..k {
            core::hint::black_box(owner(core::hint::black_box(p + 0x10)));
        }
        let cached = t.elapsed().as_nanos() as f64 / k as f64;
        let t = std::time::Instant::now();
        for _ in 0..k {
            invalidate_range(p, 1);
            core::hint::black_box(owner(core::hint::black_box(p + 0x10)));
        }
        let full = t.elapsed().as_nanos() as f64 / k as f64;
        println!("owner_lookup_cost ns: before(one VirtualQuery)={before:.0} after_cached={cached:.0} after_revalidate={full:.0}");
        release(p);
    }

    #[test]
    fn a_reused_base_without_write_watch_is_not_reported_as_watched() {
        let n = 0x2_0000;
        let p = reserve(n, 0x1_0000, MEM_RESERVE | MEM_COMMIT, PAGE_READWRITE) as usize;
        assert_ne!(p, 0);
        if !wanted() {
            release(p);
            return;
        }
        assert_eq!(owner(p).0, OWNER_EMULATOR);
        let vfree: unsafe extern "system" fn(usize, usize, u32) -> i32 = unsafe { core::mem::transmute(crate::hooks::vfree_wrapper()) };
        assert_ne!(unsafe { vfree(p, 0, MEM_RELEASE) }, 0);
        let q = unsafe { VirtualAlloc(p as *mut c_void, n, MEM_RESERVE | MEM_COMMIT, PAGE_READWRITE) } as usize;
        if q == p {
            assert_eq!(owner(p + 0x10).0, OWNER_NONE);
            release(q);
        }
    }

    #[test]
    fn a_reused_base_with_a_smaller_region_is_not_reported_as_watched() {
        let n = 0x4_0000;
        let p = reserve(n, 0x1_0000, MEM_RESERVE | MEM_COMMIT, PAGE_READWRITE) as usize;
        assert_ne!(p, 0);
        if !wanted() {
            release(p);
            return;
        }
        assert_eq!(owner(p).0, OWNER_EMULATOR);
        unsafe { VirtualFree(p as *mut c_void, 0, MEM_RELEASE) };
        let valloc: unsafe extern "system" fn(usize, usize, u32, u32) -> usize = unsafe { core::mem::transmute(crate::hooks::valloc_wrapper()) };
        let q = unsafe { valloc(p, 0x1_0000, MEM_RESERVE | MEM_COMMIT | MEM_WRITE_WATCH, PAGE_READWRITE) };
        if q == p {
            assert_eq!(owner(p), (OWNER_TITLE, p, 0x1_0000));
            release(q);
        }
    }

    #[test]
    fn hooked_release_drops_the_region_at_once() {
        let n = 0x2_0000;
        let p = reserve(n, 0x1_0000, MEM_RESERVE | MEM_COMMIT, PAGE_READWRITE) as usize;
        assert_ne!(p, 0);
        let vfree: unsafe extern "system" fn(usize, usize, u32) -> i32 = unsafe { core::mem::transmute(crate::hooks::vfree_wrapper()) };
        assert_ne!(unsafe { vfree(p, 0, MEM_RELEASE) }, 0);
        assert!(!REGIONS.lock().unwrap_or_else(|e| e.into_inner()).iter().any(|&(b, _, _, _)| b == p));
    }

    #[test]
    fn title_write_watch_reservations_are_title_owned() {
        let n = 0x1_0000;
        let p = reserve(n, 0, MEM_RESERVE | MEM_COMMIT | MEM_WRITE_WATCH, PAGE_READWRITE) as usize;
        assert_ne!(p, 0);
        assert_eq!(owner(p).0, OWNER_TITLE);
        release(p);
    }
}
