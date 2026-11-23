use core::ffi::c_void;
use core::sync::atomic::{AtomicUsize, Ordering};

pub const PAGE: usize = 0x1000;
const WRITE_WATCH_FLAG_RESET: u32 = 1;
const ABSENT: usize = 1;
pub const OWNER_NONE: u32 = 0;
pub const OWNER_EMULATOR: u32 = 1;
pub const OWNER_TITLE: u32 = 2;

extern "system" {
    fn GetWriteWatch(flags: u32, base: *mut c_void, size: usize, addresses: *mut *mut c_void, count: *mut usize, granularity: *mut u32) -> u32;
    fn GetModuleHandleA(name: *const u8) -> *mut c_void;
    fn GetProcAddress(module: *mut c_void, name: *const u8) -> *mut c_void;
    fn GetLastError() -> u32;
    fn VirtualQuery(addr: *const c_void, mbi: *mut Mbi, len: usize) -> usize;
}

#[repr(C)]
#[derive(Default)]
struct Mbi {
    base: usize,
    alloc_base: usize,
    alloc_protect: u32,
    partition: u16,
    pad0: u16,
    size: usize,
    state: u32,
    protect: u32,
    ty: u32,
    pad1: u32,
}

pub fn watched_ranges(lo: usize, len: usize) -> Option<Vec<(usize, usize, usize)>> {
    if !enabled() || len == 0 {
        return None;
    }
    let hi = lo + len;
    let mut out: Vec<(usize, usize, usize)> = Vec::new();
    let mut a = lo;
    while a < hi {
        let mut m = Mbi::default();
        if unsafe { VirtualQuery(a as *const c_void, &mut m, core::mem::size_of::<Mbi>()) } == 0 {
            return None;
        }
        let mut end = (m.base + m.size).min(hi);
        if end <= a {
            return None;
        }
        if m.state == 0x1000 {
            let (o, b, n) = owner(a);
            if o != OWNER_EMULATOR {
                return None;
            }
            end = end.min(b + n);
            if end <= a {
                return None;
            }
            let mut one: *mut c_void = core::ptr::null_mut();
            let mut count = 1usize;
            let mut gran = 0u32;
            if unsafe { GetWriteWatch(0, (a & !(PAGE - 1)) as *mut c_void, PAGE, &mut one, &mut count, &mut gran) } != 0 {
                return None;
            }
            match out.last_mut() {
                Some(l) if l.1 == a && l.2 == b => l.1 = end,
                _ => out.push((a, end, b)),
            }
        }
        a = end;
    }
    Some(out)
}

pub fn committed(lo: usize, len: usize) -> bool {
    let probe = |a: usize| {
        let mut m = Mbi::default();
        unsafe { VirtualQuery(a as *const c_void, &mut m, core::mem::size_of::<Mbi>()) != 0 && m.state == 0x1000 }
    };
    len > 0 && probe(lo) && probe(lo + len - 1)
}

pub fn region(addr: usize) -> String {
    let mut m = Mbi::default();
    let n = unsafe { VirtualQuery(addr as *const c_void, &mut m, core::mem::size_of::<Mbi>()) };
    if n == 0 {
        return format!("VirtualQuery({addr:#x}) failed (error {})", unsafe { GetLastError() });
    }
    let ty = match m.ty {
        0x2_0000 => "private",
        0x4_0000 => "mapped",
        0x100_0000 => "image",
        _ => "none",
    };
    let st = match m.state {
        0x1000 => "commit",
        0x2000 => "reserve",
        0x1_0000 => "free",
        _ => "?",
    };
    format!("type={ty} state={st} protect={:#x} alloc_base={:#x} alloc_protect={:#x} region={:#x}+{:#x} va-alloc_base={:#x}", m.protect, m.alloc_base, m.alloc_protect, m.base, m.size, addr.wrapping_sub(m.alloc_base))
}

type OwnerFn = unsafe extern "system" fn(usize, *mut usize, *mut usize) -> u32;

static OWNER_FN: AtomicUsize = AtomicUsize::new(0);
static OWNER_TRIES: AtomicUsize = AtomicUsize::new(0);
static OWNER_LAST_TRY: std::sync::Mutex<Option<std::time::Instant>> = std::sync::Mutex::new(None);
static TEST_OWNER: std::sync::Mutex<Vec<(usize, usize)>> = std::sync::Mutex::new(Vec::new());

pub fn enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("TOMBOX_WRITE_WATCH").map(|v| v.trim() != "0").unwrap_or(true))
}

fn owner_fn() -> Option<OwnerFn> {
    let f = OWNER_FN.load(Ordering::Acquire);
    if f > ABSENT {
        return Some(unsafe { core::mem::transmute::<usize, OwnerFn>(f) });
    }
    let now = std::time::Instant::now();
    {
        let mut last = OWNER_LAST_TRY.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(t) = *last {
            if now.duration_since(t) < std::time::Duration::from_millis(250) {
                return None;
            }
        }
        *last = Some(now);
    }
    for m in [&b"xmem.dll\0"[..], b"xmem_x.dll\0", b"xmem_xs.dll\0"] {
        let h = unsafe { GetModuleHandleA(m.as_ptr()) };
        if h.is_null() {
            continue;
        }
        let p = unsafe { GetProcAddress(h, b"TomboxWriteWatchOwner\0".as_ptr()) } as usize;
        if p > ABSENT {
            OWNER_FN.store(p, Ordering::Release);
            return Some(unsafe { core::mem::transmute::<usize, OwnerFn>(p) });
        }
    }
    OWNER_TRIES.fetch_add(1, Ordering::Relaxed);
    None
}

pub fn register_test_region(base: usize, len: usize) {
    let mut r = TEST_OWNER.lock().unwrap_or_else(|e| e.into_inner());
    r.retain(|&(b, n)| b + n <= base || base + len <= b);
    r.push((base, len));
}

pub fn owner(addr: usize) -> (u32, usize, usize) {
    if let Some(&(b, n)) = TEST_OWNER.lock().unwrap_or_else(|e| e.into_inner()).iter().find(|&&(b, n)| addr >= b && addr < b + n) {
        return (OWNER_EMULATOR, b, n);
    }
    let Some(f) = owner_fn() else { return (OWNER_NONE, 0, 0) };
    let (mut b, mut n) = (0usize, 0usize);
    let o = unsafe { f(addr, &mut b, &mut n) };
    (o, b, n)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    Watched,
    Disabled,
    NoOwnerApi,
    TitleOwned,
    NotWatched,
    SpansRegions,
}

impl Class {
    pub fn why(self) -> &'static str {
        match self {
            Class::Watched => "watched",
            Class::Disabled => "TOMBOX_WRITE_WATCH=0",
            Class::NoOwnerApi => "xmem exports no TomboxWriteWatchOwner, so a watched range could be the title's own",
            Class::TitleOwned => "the title allocated this memory with MEM_WRITE_WATCH itself; resetting would corrupt its XMemGetWriteWatch",
            Class::NotWatched => "the backing memory has no write watch (heap block, physical or file-mapped view, or a reservation made before write watch)",
            Class::SpansRegions => "the resource spans more than one watched reservation",
        }
    }
}

pub fn classify(lo: usize, len: usize) -> Class {
    if !enabled() {
        return Class::Disabled;
    }
    if len == 0 {
        return Class::NotWatched;
    }
    let (o, b, n) = owner(lo);
    match o {
        OWNER_EMULATOR => {}
        OWNER_TITLE => return Class::TitleOwned,
        _ => {
            return if owner_fn().is_none() && TEST_OWNER.lock().unwrap_or_else(|e| e.into_inner()).is_empty() { Class::NoOwnerApi } else { Class::NotWatched };
        }
    }
    if lo + len > b + n {
        return Class::SpansRegions;
    }
    let mut one: *mut c_void = core::ptr::null_mut();
    let mut count = 1usize;
    let mut gran = 0u32;
    let first = lo & !(PAGE - 1);
    let r = unsafe { GetWriteWatch(0, first as *mut c_void, PAGE, &mut one, &mut count, &mut gran) };
    if r != 0 {
        return Class::NotWatched;
    }
    Class::Watched
}

pub fn take_dirty(lo: usize, hi: usize, buf: &mut Vec<usize>) -> Result<Vec<(usize, usize)>, u32> {
    let a = lo & !(PAGE - 1);
    let b = (hi + PAGE - 1) & !(PAGE - 1);
    if a >= b {
        return Ok(Vec::new());
    }
    let pages = (b - a) / PAGE;
    if buf.len() < pages {
        buf.resize(pages, 0);
    }
    let mut count = pages;
    let mut gran = 0u32;
    let r = unsafe { GetWriteWatch(WRITE_WATCH_FLAG_RESET, a as *mut c_void, b - a, buf.as_mut_ptr() as *mut *mut c_void, &mut count, &mut gran) };
    if r != 0 {
        return Err(unsafe { GetLastError() });
    }
    let g = if gran == 0 { PAGE } else { gran as usize };
    Ok(runs(&buf[..count.min(pages)], g))
}

pub fn runs(pages: &[usize], gran: usize) -> Vec<(usize, usize)> {
    let mut out: Vec<(usize, usize)> = Vec::new();
    let mut v: Vec<usize> = pages.to_vec();
    v.sort_unstable();
    for p in v {
        match out.last_mut() {
            Some(last) if last.1 >= p => last.1 = last.1.max(p + gran),
            _ => out.push((p, p + gran)),
        }
    }
    out
}

pub fn merge_extents(mut ext: Vec<(usize, usize)>) -> Vec<(usize, usize)> {
    ext.sort_unstable();
    let mut out: Vec<(usize, usize)> = Vec::new();
    for (a, b) in ext {
        let (a, b) = (a & !(PAGE - 1), (b + PAGE - 1) & !(PAGE - 1));
        match out.last_mut() {
            Some(last) if last.1 >= a => last.1 = last.1.max(b),
            _ => out.push((a, b)),
        }
    }
    out
}

pub fn row_extents(offs: &[u32], w_e: usize, h_e: usize, bpe: usize) -> Vec<(u32, u32)> {
    (0..h_e)
        .map(|y| {
            let row = &offs[y * w_e..(y + 1) * w_e];
            let lo = row.iter().copied().min().unwrap_or(0);
            let hi = row.iter().copied().max().unwrap_or(0) + bpe as u32;
            (lo, hi)
        })
        .collect()
}

pub fn dirty_bands(rows: &[(u32, u32)], base: usize, dirty: &[(usize, usize)], gap: usize) -> Vec<(usize, usize)> {
    let mut out: Vec<(usize, usize)> = Vec::new();
    for (y, &(lo, hi)) in rows.iter().enumerate() {
        let (a, b) = (base + lo as usize, base + hi as usize);
        let hit = dirty.iter().any(|&(da, db)| da < b && a < db);
        if !hit {
            continue;
        }
        match out.last_mut() {
            Some(last) if y <= last.1 + gap => last.1 = y + 1,
            _ => out.push((y, y + 1)),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    extern "system" {
        fn VirtualAlloc(addr: *mut c_void, len: usize, ty: u32, protect: u32) -> *mut c_void;
        fn VirtualFree(addr: *mut c_void, len: usize, ty: u32) -> i32;
    }

    const MEM_COMMIT_RESERVE: u32 = 0x3000;
    const MEM_WRITE_WATCH: u32 = 0x20_0000;

    #[test]
    fn dirty_pages_coalesce_into_runs() {
        assert_eq!(runs(&[0x3000, 0x1000, 0x2000, 0x6000], PAGE), vec![(0x1000, 0x4000), (0x6000, 0x7000)]);
        assert_eq!(merge_extents(vec![(0x2010, 0x2020), (0x1000, 0x2001), (0x9000, 0x9001)]), vec![(0x1000, 0x3000), (0x9000, 0xa000)]);
    }

    #[test]
    fn a_partial_page_write_marks_only_the_rows_on_that_page() {
        let (w, h, bpe) = (256usize, 64usize, 4usize);
        let offs: Vec<u32> = (0..h).flat_map(|y| (0..w).map(move |x| ((y * w + x) * bpe) as u32)).collect();
        let rows = row_extents(&offs, w, h, bpe);
        assert_eq!(rows[1], (1024, 2048));
        let base = 0x10_0000;
        let bands = dirty_bands(&rows, base, &[(base + 0x3000, base + 0x4000)], 0);
        assert_eq!(bands, vec![(12, 16)]);
        let bands = dirty_bands(&rows, base, &[(base, base + 0x1000), (base + 0x2000, base + 0x3000)], 0);
        assert_eq!(bands, vec![(0, 4), (8, 12)]);
        assert_eq!(dirty_bands(&rows, base, &[(base, base + 0x1000), (base + 0x2000, base + 0x3000)], 4), vec![(0, 12)]);
        assert!(dirty_bands(&rows, base, &[(base + 0x40000, base + 0x41000)], 0).is_empty());
    }

    #[test]
    fn get_write_watch_reports_writes_after_a_reset() {
        unsafe {
            let n = 8 * PAGE;
            let p = VirtualAlloc(core::ptr::null_mut(), n, MEM_COMMIT_RESERVE | MEM_WRITE_WATCH, 0x04) as usize;
            assert_ne!(p, 0);
            register_test_region(p, n);
            if enabled() {
                assert_eq!(classify(p + 16, 3 * PAGE), Class::Watched);
            }
            let mut buf = Vec::new();
            let _ = take_dirty(p, p + n, &mut buf).unwrap();
            *((p + 2 * PAGE + 100) as *mut u8) = 7;
            *((p + 3 * PAGE) as *mut u8) = 7;
            *((p + 6 * PAGE + 5) as *mut u8) = 7;
            assert_eq!(take_dirty(p, p + n, &mut buf).unwrap(), vec![(p + 2 * PAGE, p + 4 * PAGE), (p + 6 * PAGE, p + 7 * PAGE)]);
            assert!(take_dirty(p, p + n, &mut buf).unwrap().is_empty(), "RESET clears what was reported");
            let q = VirtualAlloc(core::ptr::null_mut(), n, MEM_COMMIT_RESERVE, 0x04) as usize;
            assert!(take_dirty(q, q + n, &mut buf).is_err(), "memory without MEM_WRITE_WATCH is refused, not reported clean");
            VirtualFree(p as *mut c_void, 0, 0x8000);
            VirtualFree(q as *mut c_void, 0, 0x8000);
        }
    }
}
