use core::ffi::c_void;
use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::collections::BTreeMap;
use std::sync::Mutex;

use crate::sys::*;

pub const GRAN: usize = 0x1_0000;
pub const DEFAULT_MIN: usize = 0x10_0000;
pub const HEAP_GENERATE_EXCEPTIONS: u32 = 0x4;
pub const HEAP_ZERO_MEMORY: u32 = 0x8;
pub const HEAP_REALLOC_IN_PLACE_ONLY: u32 = 0x10;
pub const PROCESS_HEAP_ENTRY_BUSY: u16 = 0x4;
pub const ERROR_NOT_ENOUGH_MEMORY: u32 = 8;
pub const ERROR_NO_MORE_ITEMS: u32 = 259;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Block {
    pub len: usize,
    pub size: usize,
    pub heap: usize,
}

static BLOCKS: Mutex<BTreeMap<usize, Block>> = Mutex::new(BTreeMap::new());
static LIVE_BYTES: AtomicU64 = AtomicU64::new(0);
static SERVED: AtomicU64 = AtomicU64::new(0);
static ON: AtomicUsize = AtomicUsize::new(2);
static MIN: AtomicUsize = AtomicUsize::new(0);

fn locked() -> std::sync::MutexGuard<'static, BTreeMap<usize, Block>> {
    BLOCKS.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn parse_size(s: &str) -> Option<usize> {
    let t = s.trim();
    let (digits, radix) = match t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        Some(h) => (h, 16),
        None => (t, 10),
    };
    let (digits, mul) = match digits.chars().last().map(|c| c.to_ascii_lowercase()) {
        Some('k') if radix == 10 => (&digits[..digits.len() - 1], 1usize << 10),
        Some('m') if radix == 10 => (&digits[..digits.len() - 1], 1usize << 20),
        _ => (digits, 1usize),
    };
    usize::from_str_radix(digits, radix).ok()?.checked_mul(mul)
}

pub fn enabled() -> bool {
    match ON.load(Ordering::Relaxed) {
        0 => false,
        1 => true,
        _ => {
            let on = std::env::var("TOMBOX_XMEM_WW_HEAP").map(|v| v.trim() == "1").unwrap_or(false) && crate::write_watch::wanted();
            ON.store(on as usize, Ordering::Relaxed);
            on
        }
    }
}

pub fn min_size() -> usize {
    let m = MIN.load(Ordering::Relaxed);
    if m != 0 {
        return m;
    }
    let v = std::env::var("TOMBOX_XMEM_WW_HEAP_MIN").ok().and_then(|s| parse_size(&s)).unwrap_or(DEFAULT_MIN).max(GRAN);
    MIN.store(v, Ordering::Relaxed);
    v
}

#[cfg(test)]
pub fn set_for_test(on: bool, min: usize) {
    ON.store(on as usize, Ordering::Relaxed);
    MIN.store(min.max(GRAN), Ordering::Relaxed);
}

pub fn title_heap(heap: usize) -> bool {
    heap != 0 && (heap == unsafe { GetProcessHeap() } as usize || crate::heap_acct::is_title(heap))
}

pub fn wants(heap: usize, size: usize) -> bool {
    size >= min_size() && enabled() && title_heap(heap)
}

pub fn live_bytes() -> u64 {
    LIVE_BYTES.load(Ordering::Relaxed)
}

pub fn served() -> u64 {
    SERVED.load(Ordering::Relaxed)
}

pub fn alloc(heap: usize, size: usize) -> Option<usize> {
    let len = size.max(1).checked_add(GRAN - 1)? & !(GRAN - 1);
    let p = crate::write_watch::reserve(len, GRAN, MEM_RESERVE | MEM_COMMIT, PAGE_READWRITE) as usize;
    if p == 0 {
        return None;
    }
    if crate::write_watch::owner(p).0 != crate::write_watch::OWNER_EMULATOR {
        unsafe { VirtualFree(p as *mut c_void, 0, MEM_RELEASE) };
        crate::write_watch::forget(p);
        return None;
    }
    locked().insert(p, Block { len, size, heap });
    LIVE_BYTES.fetch_add(size as u64, Ordering::Relaxed);
    let n = SERVED.fetch_add(1, Ordering::Relaxed);
    if n == 0 || (n + 1) % 256 == 0 {
        compat_common::record_raw(
            "xmem",
            "ww_heap",
            &format!("title heap blocks >= {:#x} served from write-watched memory: {} served, {:.1}MB live (opt-in via TOMBOX_XMEM_WW_HEAP=1)", min_size(), served(), live_bytes() as f64 / (1u64 << 20) as f64),
        );
    }
    Some(p)
}

pub fn lookup(p: usize) -> Option<Block> {
    if p == 0 || p & (GRAN - 1) != 0 {
        return None;
    }
    locked().get(&p).copied()
}

pub fn is_block(p: usize) -> bool {
    lookup(p).is_some()
}

pub fn size_of(p: usize) -> Option<usize> {
    lookup(p).map(|b| b.size)
}

pub fn free(p: usize) -> Option<usize> {
    if p == 0 || p & (GRAN - 1) != 0 {
        return None;
    }
    let b = locked().remove(&p)?;
    crate::write_watch::forget(p);
    unsafe { VirtualFree(p as *mut c_void, 0, MEM_RELEASE) };
    LIVE_BYTES.fetch_sub(b.size as u64, Ordering::Relaxed);
    Some(b.size)
}

pub enum Resize {
    InPlace(usize),
    Moved(usize),
    Failed,
}

pub unsafe fn resize(p: usize, size: usize, zero: bool, in_place_only: bool, fallback: impl FnOnce(usize) -> usize) -> Resize {
    let Some(b) = lookup(p) else { return Resize::Failed };
    if size <= b.len && size > 0 {
        if zero && size > b.size {
            core::ptr::write_bytes((p + b.size) as *mut u8, 0, size - b.size);
        } else if size < b.size {
            core::ptr::write_bytes((p + size) as *mut u8, 0, b.size - size);
        }
        if let Some(e) = locked().get_mut(&p) {
            LIVE_BYTES.fetch_add(size as u64, Ordering::Relaxed);
            LIVE_BYTES.fetch_sub(e.size as u64, Ordering::Relaxed);
            e.size = size;
        }
        return Resize::InPlace(p);
    }
    if in_place_only {
        SetLastError(ERROR_NOT_ENOUGH_MEMORY);
        return Resize::Failed;
    }
    let n = if size >= min_size() { alloc(b.heap, size) } else { None }.unwrap_or_else(|| fallback(size));
    if n == 0 {
        return Resize::Failed;
    }
    core::ptr::copy_nonoverlapping(p as *const u8, n as *mut u8, b.size.min(size));
    if zero && size > b.size && !is_block(n) {
        core::ptr::write_bytes((n + b.size) as *mut u8, 0, size - b.size);
    }
    free(p);
    Resize::Moved(n)
}

pub unsafe fn migrate(heap: usize, old: usize, old_size: usize, size: usize) -> Option<usize> {
    let n = alloc(heap, size)?;
    core::ptr::copy_nonoverlapping(old as *const u8, n as *mut u8, old_size.min(size));
    Some(n)
}

#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct HeapEntry {
    pub data: usize,
    pub cb_data: u32,
    pub cb_overhead: u8,
    pub region_index: u8,
    pub flags: u16,
    pub rest: [u64; 3],
}

pub fn next_after(heap: usize, after: Option<usize>) -> Option<(usize, Block)> {
    let m = locked();
    let mut it: Box<dyn Iterator<Item = (&usize, &Block)>> = match after {
        Some(a) => Box::new(m.range(a + 1..)),
        None => Box::new(m.iter()),
    };
    it.find(|(_, b)| b.heap == heap).map(|(p, b)| (*p, *b))
}

const _: () = assert!(core::mem::size_of::<HeapEntry>() == 40);

pub unsafe fn walk(heap: usize, entry: *mut HeapEntry, real: impl FnOnce() -> i32) -> i32 {
    if entry.is_null() {
        return real();
    }
    let cur = (*entry).data;
    let ours = is_block(cur);
    if !ours {
        let r = real();
        if r != 0 || GetLastError() != ERROR_NO_MORE_ITEMS || !title_heap(heap) {
            return r;
        }
    }
    match next_after(heap, if ours { Some(cur) } else { None }) {
        Some((p, b)) => {
            *entry = HeapEntry { data: p, cb_data: b.size.min(u32::MAX as usize) as u32, cb_overhead: 0, region_index: 0, flags: PROCESS_HEAP_ENTRY_BUSY, rest: [0; 3] };
            1
        }
        None => {
            SetLastError(ERROR_NO_MORE_ITEMS);
            0
        }
    }
}

#[cfg(test)]
pub fn test_lock() -> std::sync::MutexGuard<'static, ()> {
    static L: Mutex<()> = Mutex::new(());
    L.lock().unwrap_or_else(|e| e.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[link(name = "kernel32")]
    extern "system" {
        fn GetWriteWatch(flags: u32, base: *mut c_void, size: usize, addrs: *mut *mut c_void, count: *mut usize, gran: *mut u32) -> u32;
    }

    fn heap() -> usize {
        unsafe { GetProcessHeap() as usize }
    }

    fn dirty(p: usize, len: usize) -> usize {
        let mut buf = vec![core::ptr::null_mut::<c_void>(); len / 0x1000];
        let mut count = buf.len();
        let mut gran = 0u32;
        let r = unsafe { GetWriteWatch(1, p as *mut c_void, len, buf.as_mut_ptr(), &mut count, &mut gran) };
        assert_eq!(r, 0, "GetWriteWatch failed on a ww heap block");
        count
    }

    #[test]
    fn size_parsing_accepts_hex_decimal_and_units() {
        assert_eq!(parse_size("0x100000"), Some(0x10_0000));
        assert_eq!(parse_size("1048576"), Some(0x10_0000));
        assert_eq!(parse_size("2m"), Some(0x20_0000));
        assert_eq!(parse_size("512K"), Some(0x8_0000));
        assert_eq!(parse_size("junk"), None);
    }

    #[test]
    fn large_process_heap_blocks_are_watched_aligned_and_zeroed() {
        let _g = test_lock();
        set_for_test(true, 0x10_0000);
        assert!(wants(heap(), 0x10_0000));
        assert!(!wants(heap(), 0xF_FFFF));
        let p = alloc(heap(), 0x12_3456).unwrap();
        assert_eq!(p & (GRAN - 1), 0);
        assert_eq!(size_of(p), Some(0x12_3456));
        let (o, b, n) = crate::write_watch::owner(p);
        assert_eq!(o, crate::write_watch::OWNER_EMULATOR);
        assert!(b <= p && p + 0x12_3456 <= b + n);
        assert!(unsafe { core::slice::from_raw_parts(p as *const u8, 0x12_3456) }.iter().all(|&x| x == 0));
        dirty(p, 0x13_0000);
        unsafe { *((p + 0x4_0010) as *mut u8) = 7 };
        assert_eq!(dirty(p, 0x13_0000), 1);
        assert_eq!(free(p), Some(0x12_3456));
        assert!(!is_block(p));
        assert_eq!(crate::write_watch::owner(p).0, crate::write_watch::OWNER_NONE);
    }

    #[test]
    fn unaligned_or_unknown_pointers_are_never_ours() {
        let _g = test_lock();
        set_for_test(true, 0x10_0000);
        assert!(!is_block(0));
        assert!(!is_block(0x1234_5670));
        let p = alloc(heap(), 0x10_0000).unwrap();
        assert!(!is_block(p + 0x10));
        assert_eq!(free(p + 0x10), None);
        assert_eq!(free(p), Some(0x10_0000));
        assert_eq!(free(p), None);
    }

    #[test]
    fn foreign_heaps_and_the_off_switch_are_left_alone() {
        let _g = test_lock();
        set_for_test(true, 0x10_0000);
        let other = unsafe { HeapCreate(0, 0, 0) } as usize;
        assert!(!wants(other, 0x100_0000));
        set_for_test(false, 0x10_0000);
        assert!(!wants(heap(), 0x100_0000));
        set_for_test(true, 0x10_0000);
    }

    #[test]
    fn resize_grows_in_place_within_the_reservation_and_moves_beyond_it() {
        let _g = test_lock();
        set_for_test(true, 0x10_0000);
        let p = alloc(heap(), 0x10_8000).unwrap();
        unsafe { core::ptr::write_bytes(p as *mut u8, 0xAB, 0x10_8000) };
        match unsafe { resize(p, 0x11_0000, true, false, |_| 0) } {
            Resize::InPlace(q) => assert_eq!(q, p),
            _ => panic!("expected in place"),
        }
        assert_eq!(unsafe { *((p + 0x10_8000) as *const u8) }, 0);
        assert!(matches!(unsafe { resize(p, 0x20_0000, false, true, |_| 0) }, Resize::Failed));
        assert!(is_block(p));
        let q = match unsafe { resize(p, 0x20_0000, false, false, |_| 0) } {
            Resize::Moved(q) => q,
            _ => panic!("expected move"),
        };
        assert!(is_block(q) && !is_block(p));
        assert_eq!(unsafe { *(q as *const u8) }, 0xAB);
        assert_eq!(unsafe { *((q + 0x10_7FFF) as *const u8) }, 0xAB);
        assert_eq!(free(q), Some(0x20_0000));
    }

    #[test]
    fn shrinking_below_the_threshold_keeps_the_block_and_clears_the_tail() {
        let _g = test_lock();
        set_for_test(true, 0x10_0000);
        let p = alloc(heap(), 0x10_0000).unwrap();
        unsafe { core::ptr::write_bytes(p as *mut u8, 0xCD, 0x10_0000) };
        assert!(matches!(unsafe { resize(p, 0x100, false, false, |_| 0) }, Resize::InPlace(q) if q == p));
        assert_eq!(size_of(p), Some(0x100));
        assert_eq!(unsafe { *((p + 0x100) as *const u8) }, 0);
        match unsafe { resize(p, 0x10_0000, true, false, |_| 0) } {
            Resize::InPlace(q) => assert_eq!(q, p),
            _ => panic!("expected in place"),
        }
        assert!(unsafe { core::slice::from_raw_parts((p + 0x100) as *const u8, 0x10_0000 - 0x100) }.iter().all(|&x| x == 0));
        free(p);
    }

    #[test]
    fn moving_to_a_small_size_uses_the_real_heap_fallback() {
        let _g = test_lock();
        set_for_test(true, 0x10_0000);
        let p = alloc(heap(), 0x10_0000).unwrap();
        unsafe { *(p as *mut u32) = 0xFEED_BEEF };
        let h = heap();
        let r = unsafe { resize(p, 0, false, false, |n| HeapAlloc(h as Handle, 0, n.max(1)) as usize) };
        let q = match r {
            Resize::Moved(q) => q,
            Resize::InPlace(_) => panic!("size 0 must not stay in place"),
            Resize::Failed => panic!("fallback failed"),
        };
        assert!(!is_block(q) && !is_block(p));
        unsafe { HeapFree(h as Handle, 0, q as *mut c_void) };
    }

    #[test]
    fn migrate_copies_a_real_heap_block_into_a_watched_one() {
        let _g = test_lock();
        set_for_test(true, 0x10_0000);
        let h = heap();
        let old = unsafe { HeapAlloc(h as Handle, 0, 0x2000) } as usize;
        unsafe { core::ptr::write_bytes(old as *mut u8, 0x5A, 0x2000) };
        let n = unsafe { migrate(h, old, 0x2000, 0x18_0000) }.unwrap();
        unsafe { HeapFree(h as Handle, 0, old as *mut c_void) };
        assert!(is_block(n));
        assert!(unsafe { core::slice::from_raw_parts(n as *const u8, 0x2000) }.iter().all(|&x| x == 0x5A));
        assert_eq!(unsafe { *((n + 0x2000) as *const u8) }, 0);
        free(n);
    }

    #[test]
    fn heap_walk_appends_watched_blocks_after_the_real_entries() {
        let _g = test_lock();
        set_for_test(true, 0x10_0000);
        let h = heap();
        let a = alloc(h, 0x10_0000).unwrap();
        let b = alloc(h, 0x30_0000).unwrap();
        let mut e = HeapEntry::default();
        let ep: *mut HeapEntry = &mut e;
        let mut seen = Vec::new();
        let mut real_left = 2usize;
        loop {
            let r = unsafe {
                walk(h, ep, || {
                    if real_left > 0 {
                        real_left -= 1;
                        (*ep).data = 0x1000 * (3 - real_left);
                        1
                    } else {
                        SetLastError(ERROR_NO_MORE_ITEMS);
                        0
                    }
                })
            };
            if r == 0 {
                assert_eq!(unsafe { GetLastError() }, ERROR_NO_MORE_ITEMS);
                break;
            }
            let cur = unsafe { *ep };
            seen.push((cur.data, cur.cb_data, cur.flags));
        }
        assert!(seen.contains(&(a, 0x10_0000, PROCESS_HEAP_ENTRY_BUSY)));
        assert!(seen.contains(&(b, 0x30_0000, PROCESS_HEAP_ENTRY_BUSY)));
        assert_eq!(seen.iter().filter(|s| s.0 < 0x10000).count(), 2);
        free(a);
        free(b);
    }
}
