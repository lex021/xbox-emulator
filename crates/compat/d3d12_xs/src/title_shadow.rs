use compat_common::record;
use core::ffi::c_void;
use core::sync::atomic::{AtomicUsize, Ordering};
use std::collections::HashMap;
use std::sync::{mpsc, Mutex};

#[repr(C)]
struct Guid(u32, u16, u16, [u8; 8]);
const IID_HEAP: Guid = Guid(0x6b3b2502, 0x6e51, 0x45b3, [0x90, 0xee, 0x98, 0x84, 0x26, 0x5e, 0x8d, 0xf3]);
const IID_RESOURCE: Guid = Guid(0x696442be, 0xa72e, 0x4059, [0xbc, 0x79, 0x5b, 0x5c, 0x98, 0x04, 0x0f, 0xad]);
const IID_DEVICE: Guid = Guid(0x189819f1, 0x1db6, 0x4b57, [0xbe, 0x54, 0x18, 0x21, 0x33, 0x9b, 0x85, 0xf7]);
const IID_FENCE: Guid = Guid(0x0a753dcf, 0xc4d8, 0x4b91, [0xad, 0xf6, 0xbe, 0x5a, 0x60, 0xd9, 0x5a, 0x76]);

const HEAP_TYPE_CUSTOM: u32 = 4;
const CPU_PAGE_WRITE_BACK: u32 = 3;
const POOL_L0: u32 = 1;
const HEAP_ALLOW_ONLY_BUFFERS: u32 = 0xc0;
const MEM_COMMIT: u32 = 0x1000;
const PAGE_NOACCESS: u32 = 0x01;
const PAGE_GUARD: u32 = 0x100;
const READ_CHUNK: usize = 1 << 20;
const BLOCK: usize = 256;
const CURRENT_PROCESS: isize = -1;
const INFINITE: u32 = u32::MAX;

#[link(name = "ntdll")]
extern "system" {
    fn NtReadVirtualMemory(process: isize, addr: *const c_void, buf: *mut c_void, len: usize, done: *mut usize) -> i32;
    fn NtWriteVirtualMemory(process: isize, addr: *mut c_void, buf: *const c_void, len: usize, done: *mut usize) -> i32;
}

extern "system" {
    fn VirtualQuery(addr: *const c_void, info: *mut Mbi, len: usize) -> usize;
    fn CreateEventW(attr: *mut c_void, manual: i32, initial: i32, name: *const u16) -> *mut c_void;
    fn WaitForSingleObject(h: *mut c_void, ms: u32) -> u32;
    fn CloseHandle(h: *mut c_void) -> i32;
    fn GetWriteWatch(flags: u32, base: *mut c_void, size: usize, addrs: *mut *mut c_void, count: *mut usize, gran: *mut u32) -> u32;
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct Mbi {
    base: usize,
    alloc_base: usize,
    alloc_protect: u32,
    partition: u16,
    pad0: u16,
    size: usize,
    state: u32,
    protect: u32,
    kind: u32,
    pad1: u32,
}

unsafe fn vt(obj: *mut c_void) -> *const usize {
    unsafe { *(obj as *const *const usize) }
}

macro_rules! vcall {
    ($obj:expr, $slot:expr, fn($($at:ty),*) -> $rt:ty $(, $arg:expr)*) => {{
        let f: unsafe extern "system" fn(*mut c_void $(, $at)*) -> $rt = core::mem::transmute(*vt($obj).add($slot));
        f($obj $(, $arg)*)
    }};
}

pub fn enabled() -> bool {
    static V: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *V.get_or_init(|| std::env::var("TOMBOX_TITLE_SHADOW").map(|v| v != "0").unwrap_or(true))
}

pub fn merge_down(title: &[u8], snap: &mut [u8], shadow: &mut [u8]) -> usize {
    let mut n = 0;
    let mut i = 0;
    while i < title.len() {
        let e = (i + BLOCK).min(title.len());
        if title[i..e] != snap[i..e] {
            for j in i..e {
                if title[j] != snap[j] {
                    shadow[j] = title[j];
                    snap[j] = title[j];
                    n += 1;
                }
            }
        }
        i = e;
    }
    n
}

pub fn merge_up(shadow: &[u8], snap: &mut [u8]) -> Vec<(usize, usize)> {
    let mut runs = Vec::new();
    let mut i = 0;
    while i < shadow.len() {
        let e = (i + BLOCK).min(shadow.len());
        if shadow[i..e] == snap[i..e] {
            i = e;
            continue;
        }
        let mut j = i;
        while j < e {
            if shadow[j] == snap[j] {
                j += 1;
                continue;
            }
            let start = j;
            while j < shadow.len() && shadow[j] != snap[j] {
                snap[j] = shadow[j];
                j += 1;
            }
            match runs.last_mut() {
                Some((s, l)) if *s + *l == start => *l += j - start,
                _ => runs.push((start, j - start)),
            }
        }
        i = j.max(e);
    }
    runs
}

pub fn coalesce(mut r: Vec<(u64, u64)>) -> Vec<(u64, u64)> {
    r.retain(|x| x.1 > x.0);
    r.sort_unstable();
    let mut out: Vec<(u64, u64)> = Vec::new();
    for (a, b) in r {
        match out.last_mut() {
            Some(l) if a <= l.1 => l.1 = l.1.max(b),
            _ => out.push((a, b)),
        }
    }
    out
}

fn readable(m: &Mbi) -> bool {
    m.state == MEM_COMMIT && m.protect & (PAGE_NOACCESS | PAGE_GUARD) == 0 && m.protect != 0
}

fn writable(m: &Mbi) -> bool {
    m.state == MEM_COMMIT && m.protect & 0xcc != 0 && m.protect & PAGE_GUARD == 0
}

fn regions(lo: u64, hi: u64, ok: fn(&Mbi) -> bool) -> Vec<(u64, u64)> {
    let mut out = Vec::new();
    let mut a = lo;
    while a < hi {
        let mut m = Mbi::default();
        if unsafe { VirtualQuery(a as *const c_void, &mut m, core::mem::size_of::<Mbi>()) } == 0 || m.size == 0 {
            break;
        }
        let end = (m.base as u64 + m.size as u64).min(hi);
        if ok(&m) {
            match out.last_mut() {
                Some((_, e)) if *e == a => *e = end,
                _ => out.push((a, end)),
            }
        }
        a = end;
    }
    out
}

const WATCH_PAGE: u64 = 0x1000;
const WRITE_WATCH_FLAG_RESET: u32 = 1;

pub fn dirty_pages(lo: u64, hi: u64) -> Option<Vec<u64>> {
    let a = lo & !(WATCH_PAGE - 1);
    let b = (hi + WATCH_PAGE - 1) & !(WATCH_PAGE - 1);
    let cap = ((b - a) / WATCH_PAGE) as usize;
    let mut out = vec![0usize; cap.max(1)];
    let mut count = cap;
    let mut gran = 0u32;
    let r = unsafe { GetWriteWatch(WRITE_WATCH_FLAG_RESET, a as *mut c_void, (b - a) as usize, out.as_mut_ptr() as *mut *mut c_void, &mut count, &mut gran) };
    if r != 0 || gran as u64 != WATCH_PAGE {
        return None;
    }
    out.truncate(count);
    Some(out.into_iter().map(|p| p as u64).collect())
}

fn read_title(addr: u64, buf: &mut [u8]) -> bool {
    let mut done = 0usize;
    let st = unsafe { NtReadVirtualMemory(CURRENT_PROCESS, addr as *const c_void, buf.as_mut_ptr() as *mut c_void, buf.len(), &mut done) };
    st >= 0 && done == buf.len()
}

fn write_title(addr: u64, buf: &[u8]) -> bool {
    let mut done = 0usize;
    let st = unsafe { NtWriteVirtualMemory(CURRENT_PROCESS, addr as *mut c_void, buf.as_ptr() as *const c_void, buf.len(), &mut done) };
    st >= 0 && done == buf.len()
}

struct Shadow {
    lo: u64,
    hi: u64,
    view: usize,
    cpu: usize,
    snap: Vec<u8>,
    live: HashMap<usize, (u64, u64)>,
}

unsafe impl Send for Shadow {}

impl Shadow {
    fn ranges(&self) -> Vec<(u64, u64)> {
        coalesce(self.live.values().copied().collect())
    }

    unsafe fn shadow_mut(&mut self, off: usize, len: usize) -> &mut [u8] {
        unsafe { core::slice::from_raw_parts_mut((self.cpu + off) as *mut u8, len) }
    }

    unsafe fn down(&mut self, tmp: &mut Vec<u8>) -> usize {
        let mut n = 0;
        for (a, b) in self.ranges() {
            if let Some(pages) = dirty_pages(self.lo + a, self.lo + b) {
                for p in pages {
                    let (x, y) = (p.max(self.lo + a), (p + WATCH_PAGE).min(self.lo + b));
                    let len = (y - x) as usize;
                    tmp.resize(len, 0);
                    if read_title(x, &mut tmp[..len]) {
                        let off = (x - self.lo) as usize;
                        let sh = unsafe { core::slice::from_raw_parts_mut((self.cpu + off) as *mut u8, len) };
                        n += merge_down(&tmp[..len], &mut self.snap[off..off + len], sh);
                    }
                }
                continue;
            }
            let len = (b - a) as usize;
            if len <= READ_CHUNK {
                tmp.resize(len, 0);
                if read_title(self.lo + a, &mut tmp[..len]) {
                    let off = a as usize;
                    let sh = unsafe { core::slice::from_raw_parts_mut((self.cpu + off) as *mut u8, len) };
                    n += merge_down(&tmp[..len], &mut self.snap[off..off + len], sh);
                    continue;
                }
            }
            for (ra, rb) in regions(self.lo + a, self.lo + b, readable) {
                let mut p = ra;
                while p < rb {
                    let len = ((rb - p) as usize).min(READ_CHUNK);
                    tmp.resize(len, 0);
                    if read_title(p, &mut tmp[..len]) {
                        let off = (p - self.lo) as usize;
                        let sh = unsafe { core::slice::from_raw_parts_mut((self.cpu + off) as *mut u8, len) };
                        n += merge_down(&tmp[..len], &mut self.snap[off..off + len], sh);
                    }
                    p += len as u64;
                }
            }
        }
        n
    }

    unsafe fn up(&mut self) -> (usize, usize) {
        let (mut wrote, mut lost) = (0usize, 0usize);
        for (a, b) in self.ranges() {
            let (a, b) = (a as usize, b as usize);
            let sh = unsafe { core::slice::from_raw_parts((self.cpu + a) as *const u8, b - a) };
            let runs = merge_up(sh, &mut self.snap[a..b]);
            if runs.is_empty() {
                continue;
            }
            let ok = regions(self.lo + a as u64, self.lo + b as u64, writable);
            for (s, l) in runs {
                let (s, e) = (s + a, s + a + l);
                for &(ra, rb) in &ok {
                    let (ws, we) = ((ra - self.lo) as usize, (rb - self.lo) as usize);
                    let (x, y) = (s.max(ws), e.min(we));
                    if x < y {
                        if write_title(self.lo + x as u64, &self.snap[x..y]) {
                            wrote += y - x;
                        } else {
                            lost += y - x;
                        }
                    }
                }
            }
        }
        (wrote, lost)
    }
}

static SHADOWS: Mutex<Option<HashMap<usize, Shadow>>> = Mutex::new(None);
static RES: Mutex<Option<HashMap<usize, u64>>> = Mutex::new(None);

fn shadows<R>(f: impl FnOnce(&mut HashMap<usize, Shadow>) -> R) -> R {
    f(SHADOWS.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new))
}

pub fn count() -> usize {
    shadows(|m| m.len())
}

fn buf_desc(width: u64) -> [u8; 0x38] {
    let mut d = [0u8; 0x38];
    d[0..4].copy_from_slice(&1u32.to_le_bytes());
    d[0x10..0x18].copy_from_slice(&width.to_le_bytes());
    d[0x18..0x1c].copy_from_slice(&1u32.to_le_bytes());
    d[0x1c..0x1e].copy_from_slice(&1u16.to_le_bytes());
    d[0x1e..0x20].copy_from_slice(&1u16.to_le_bytes());
    d[0x24..0x28].copy_from_slice(&1u32.to_le_bytes());
    d[0x2c..0x30].copy_from_slice(&1u32.to_le_bytes());
    d
}

pub unsafe fn create_heap(real: *mut c_void, lo: u64, hi: u64) -> Option<usize> {
    if crate::dred::removed() {
        return None;
    }
    let size = hi.checked_sub(lo).filter(|s| *s > 0)?;
    let mut hd = [0u8; 0x30];
    hd[0..8].copy_from_slice(&size.to_le_bytes());
    hd[8..12].copy_from_slice(&HEAP_TYPE_CUSTOM.to_le_bytes());
    hd[0xc..0x10].copy_from_slice(&CPU_PAGE_WRITE_BACK.to_le_bytes());
    hd[0x10..0x14].copy_from_slice(&POOL_L0.to_le_bytes());
    hd[0x14..0x18].copy_from_slice(&1u32.to_le_bytes());
    hd[0x18..0x1c].copy_from_slice(&1u32.to_le_bytes());
    hd[0x28..0x2c].copy_from_slice(&HEAP_ALLOW_ONLY_BUFFERS.to_le_bytes());
    let mut h: *mut c_void = core::ptr::null_mut();
    let hr = unsafe { vcall!(real, 28, fn(*const u8, *const Guid, *mut *mut c_void) -> i32, hd.as_ptr(), &IID_HEAP, &mut h) };
    if hr < 0 || h.is_null() {
        record("d3d12_xs", "title_shadow", &format!("CreateHeap(CUSTOM write-back L0, {size:#x}) for title span {lo:#x} hr={hr:#010x}"));
        return None;
    }
    let vd = buf_desc(size);
    let mut v: *mut c_void = core::ptr::null_mut();
    let hr = unsafe { vcall!(real, 29, fn(*mut c_void, u64, *const u8, u32, *const c_void, *const Guid, *mut *mut c_void) -> i32, h, 0, vd.as_ptr(), 0, core::ptr::null(), &IID_RESOURCE, &mut v) };
    let mut cpu: *mut u8 = core::ptr::null_mut();
    let mapped = hr >= 0 && !v.is_null() && unsafe { vcall!(v, 8, fn(u32, *const c_void, *mut *mut u8) -> i32, 0, core::ptr::null(), &mut cpu) } >= 0 && !cpu.is_null();
    if !mapped {
        record("d3d12_xs", "title_shadow", &format!("shadow view over {lo:#x}+{size:#x} hr={hr:#010x}; not shadowed"));
        if !v.is_null() {
            unsafe { vcall!(v, 2, fn() -> u32) };
        }
        unsafe { vcall!(h, 2, fn() -> u32) };
        return None;
    }
    let mut snap = vec![0u8; size as usize];
    for (ra, rb) in regions(lo, hi, readable) {
        let off = (ra - lo) as usize;
        let len = (rb - ra) as usize;
        if !read_title(ra, &mut snap[off..off + len]) {
            snap[off..off + len].fill(0);
        }
    }
    unsafe { core::ptr::copy_nonoverlapping(snap.as_ptr(), cpu, snap.len()) };
    shadows(|m| m.insert(h as usize, Shadow { lo, hi, view: v as usize, cpu: cpu as usize, snap, live: HashMap::new() }));
    static L: AtomicUsize = AtomicUsize::new(0);
    if L.fetch_add(1, Ordering::Relaxed) < 24 {
        record("d3d12_xs", "title_shadow", &format!("title span {lo:#x}..{hi:#x} shadowed by write-back heap {h:p}; CPU and GPU bytes merged at execute and completion, title memory stays unpinned"));
    }
    Some(h as usize)
}

pub fn add_resource(heap: usize, res: usize, va: u64, width: u64) {
    let ok = shadows(|m| match m.get_mut(&heap) {
        Some(s) if va >= s.lo && va + width <= s.hi => {
            s.live.insert(res, (va - s.lo, va - s.lo + width));
            true
        }
        _ => false,
    });
    if ok {
        RES.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).insert(res, va);
    }
}

pub fn resource_gone(res: usize) {
    if RES.lock().unwrap_or_else(|e| e.into_inner()).as_mut().and_then(|m| m.remove(&res)).is_none() {
        return;
    }
    shadows(|m| {
        for s in m.values_mut() {
            s.live.remove(&res);
        }
    });
}

pub fn map_va(res: *mut c_void) -> Option<u64> {
    let va = *RES.lock().unwrap_or_else(|e| e.into_inner()).as_ref()?.get(&(res as usize))?;
    Some(va + crate::placed_offset::bytes(res as usize))
}

const TILE: u64 = 0x1_0000;
const IID_ALLOCATOR: Guid = Guid(0x6102dee4, 0xaf59, 0x4b09, [0xb9, 0x99, 0xb4, 0x4d, 0x73, 0xf0, 0x9b, 0x24]);
const IID_GFX_LIST: Guid = Guid(0x5b160d0f, 0xac1b, 0x4185, [0x8b, 0xa8, 0xb3, 0xae, 0x42, 0xa5, 0xa4, 0x55]);
const IID_QUEUE: Guid = Guid(0x0ec870a6, 0x5d7e, 0x4c22, [0x8c, 0xfc, 0x5b, 0xaa, 0xe0, 0x76, 0x16, 0xed]);

struct Tiles {
    queue: usize,
    alloc: usize,
    list: usize,
    fence: usize,
    value: u64,
    open: bool,
}

unsafe impl Send for Tiles {}

static TILES: Mutex<Option<HashMap<usize, Tiles>>> = Mutex::new(None);

impl Tiles {
    unsafe fn new(dev: *mut c_void) -> Option<Tiles> {
        let (mut q, mut a, mut l, mut f) = (core::ptr::null_mut(), core::ptr::null_mut(), core::ptr::null_mut(), core::ptr::null_mut());
        let qd = [0u32; 4];
        unsafe {
            if vcall!(dev, 8, fn(*const u32, *const Guid, *mut *mut c_void) -> i32, qd.as_ptr(), &IID_QUEUE, &mut q) < 0
                || vcall!(dev, 9, fn(u32, *const Guid, *mut *mut c_void) -> i32, 0, &IID_ALLOCATOR, &mut a) < 0
                || vcall!(dev, 12, fn(u32, u32, *mut c_void, *mut c_void, *const Guid, *mut *mut c_void) -> i32, 0, 0, a, core::ptr::null_mut(), &IID_GFX_LIST, &mut l) < 0
                || vcall!(dev, 36, fn(u64, u32, *const Guid, *mut *mut c_void) -> i32, 0, 0, &IID_FENCE, &mut f) < 0
            {
                return None;
            }
            vcall!(l, 9, fn() -> i32);
        }
        Some(Tiles { queue: q as usize, alloc: a as usize, list: l as usize, fence: f as usize, value: 0, open: false })
    }

    unsafe fn map(&mut self, res: *mut c_void, tiles: u32, heap: *mut c_void, heap_tile: u32) {
        let coord = [0u32; 4];
        let size = [tiles, 0, 0, 0];
        let flags = [0u32];
        let start = [heap_tile];
        let counts = [tiles];
        unsafe {
            vcall!(self.queue as *mut c_void, 8, fn(*mut c_void, u32, *const u32, *const u32, *mut c_void, u32, *const u32, *const u32, *const u32, u32) -> (),
                res, 1, coord.as_ptr(), size.as_ptr(), heap, 1, flags.as_ptr(), start.as_ptr(), counts.as_ptr(), 0)
        };
    }

    unsafe fn copy(&mut self, dst: *mut c_void, src: *mut c_void, len: u64) {
        let l = self.list as *mut c_void;
        unsafe {
            if !self.open {
                vcall!(self.alloc as *mut c_void, 8, fn() -> i32);
                vcall!(l, 10, fn(*mut c_void, *mut c_void) -> i32, self.alloc as *mut c_void, core::ptr::null_mut());
                self.open = true;
            }
            vcall!(l, 15, fn(*mut c_void, u64, *mut c_void, u64, u64) -> (), dst, 0, src, 0, len);
        }
    }

    unsafe fn flush(&mut self) -> bool {
        let q = self.queue as *mut c_void;
        let f = self.fence as *mut c_void;
        unsafe {
            if self.open {
                self.open = false;
                let l = self.list as *mut c_void;
                if vcall!(l, 9, fn() -> i32) >= 0 {
                    let ls = [l];
                    vcall!(q, 10, fn(u32, *const *mut c_void) -> (), 1, ls.as_ptr());
                }
            }
            self.value += 1;
            if vcall!(q, 14, fn(*mut c_void, u64) -> i32, f, self.value) < 0 {
                return false;
            }
            wait_fence(f, self.value)
        }
    }
}

unsafe fn wait_fence(f: *mut c_void, v: u64) -> bool {
    unsafe {
        if vcall!(f, 8, fn() -> u64) >= v {
            return true;
        }
        let ev = CreateEventW(core::ptr::null_mut(), 0, 0, core::ptr::null());
        if ev.is_null() || vcall!(f, 9, fn(u64, *mut c_void) -> i32, v, ev) < 0 {
            while vcall!(f, 8, fn() -> u64) < v {
                std::thread::yield_now();
            }
        } else {
            WaitForSingleObject(ev, INFINITE);
        }
        if !ev.is_null() {
            CloseHandle(ev);
        }
        vcall!(f, 8, fn() -> u64) >= v
    }
}

unsafe fn with_tiles<R>(dev: *mut c_void, f: impl FnOnce(&mut Tiles) -> R) -> Option<R> {
    let mut g = TILES.lock().unwrap_or_else(|e| e.into_inner());
    let m = g.get_or_insert_with(HashMap::new);
    if !m.contains_key(&(dev as usize)) {
        let t = unsafe { Tiles::new(dev) }?;
        m.insert(dev as usize, t);
    }
    m.get_mut(&(dev as usize)).map(f)
}

pub fn tiles_for(len: u64) -> u32 {
    len.div_ceil(TILE) as u32
}

pub unsafe fn place_reserved(real: *mut c_void, heap: usize, offset: u64, desc: &[u8; 0x38], init_state: u32, ppv: *mut *mut c_void) -> i32 {
    let hr = unsafe { vcall!(real, 30, fn(*const u8, u32, *const c_void, *const Guid, *mut *mut c_void) -> i32, desc.as_ptr(), init_state, core::ptr::null(), &IID_RESOURCE, ppv) };
    if hr < 0 || unsafe { *ppv }.is_null() {
        return hr;
    }
    let res = unsafe { *ppv };
    let width = u64::from_le_bytes(desc[0x10..0x18].try_into().unwrap());
    let ok = unsafe {
        with_tiles(real, |t| {
            t.map(res, tiles_for(width), heap as *mut c_void, (offset / TILE) as u32);
            t.flush()
        })
    };
    if ok != Some(true) {
        unsafe { vcall!(res, 2, fn() -> u32) };
        unsafe { *ppv = core::ptr::null_mut() };
        return 0x8000_4005u32 as i32;
    }
    hr
}

pub unsafe fn place_tiled(real: *mut c_void, heap: usize, offset: u64, desc: &[u8; 0x38], init_state: u32, pclear: *const c_void, ppv: *mut *mut c_void) -> i32 {
    let mut d = *desc;
    d[8..16].copy_from_slice(&0u64.to_le_bytes());
    d[0x2c..0x30].copy_from_slice(&2u32.to_le_bytes());
    let hr = unsafe { vcall!(real, 30, fn(*const u8, u32, *const c_void, *const Guid, *mut *mut c_void) -> i32, d.as_ptr(), init_state, pclear, &IID_RESOURCE, ppv) };
    if hr < 0 || unsafe { *ppv }.is_null() {
        unsafe { *ppv = core::ptr::null_mut() };
        return if hr < 0 { hr } else { 0x8000_4005u32 as i32 };
    }
    let res = unsafe { *ppv };
    let mut tiles = 0u32;
    let mut packed = [0u32; 3];
    let mut shape = [0u32; 3];
    let mut subs = 0u32;
    unsafe { vcall!(real, 42, fn(*mut c_void, *mut u32, *mut u32, *mut u32, *mut u32, u32, *mut c_void) -> (), res, &mut tiles, packed.as_mut_ptr(), shape.as_mut_ptr(), &mut subs, 0, core::ptr::null_mut()) };
    let mut hd = [0u8; 0x30];
    unsafe { vcall!(heap as *mut c_void, 8, fn(*mut u8) -> *mut u8, hd.as_mut_ptr()) };
    let heap_size = u64::from_le_bytes(hd[0..8].try_into().unwrap());
    let fits = tiles != 0 && offset % TILE == 0 && offset + tiles as u64 * TILE <= heap_size;
    let ok = fits && unsafe {
        with_tiles(real, |t| {
            t.map(res, tiles, heap as *mut c_void, (offset / TILE) as u32);
            t.flush()
        })
    } == Some(true);
    if !ok {
        record("d3d12_xs", "title_shadow", &format!("tiled MSAA at heap {heap:#x}+{offset:#x}: {tiles} tile(s) in a {heap_size:#x} heap could not be mapped"));
        unsafe { vcall!(res, 2, fn() -> u32) };
        unsafe { *ppv = core::ptr::null_mut() };
        return 0x8007_000eu32 as i32;
    }
    static L: AtomicUsize = AtomicUsize::new(0);
    if L.fetch_add(1, Ordering::Relaxed) < 16 {
        record("d3d12_xs", "title_shadow", &format!("MSAA texture at heap {heap:#x}+{offset:#x} (not a 4 MB step) is a reserved resource whose {tiles} tile(s) map that heap range, so it aliases the title's other placements there"));
    }
    hr
}

pub fn is_shadow_heap(heap: usize) -> bool {
    shadows(|m| m.contains_key(&heap))
}

pub fn backing(res: usize) -> Option<(usize, u64)> {
    shadows(|m| m.iter().find_map(|(&h, s)| s.live.get(&res).map(|&(a, _)| (h, a))))
}

pub unsafe fn place_helper(real: *mut c_void, heap: usize, offset: u64, desc: &[u8; 0x38], ppv: *mut *mut c_void) -> i32 {
    let hr = unsafe { place_reserved(real, heap, offset, desc, 0, ppv) };
    if hr >= 0 {
        let res = unsafe { *ppv } as usize;
        let width = u64::from_le_bytes(desc[0x10..0x18].try_into().unwrap());
        let tracked = shadows(|m| match m.get_mut(&heap) {
            Some(s) if offset + width <= s.hi - s.lo => {
                s.live.insert(res, (offset, offset + width));
                true
            }
            _ => false,
        });
        if !tracked {
            record("d3d12_xs", "title_shadow", &format!("view helper {res:#x} at heap {heap:#x}+{offset:#x} is outside its shadow span; it will not follow a migration"));
        }
    }
    hr
}

pub fn helper_gone(res: usize) {
    shadows(|m| {
        for s in m.values_mut() {
            s.live.remove(&res);
        }
    });
}

fn drain_queues() {
    let pending: Vec<(usize, u64)> = QUEUES.lock().unwrap_or_else(|e| e.into_inner()).as_ref().map_or(Vec::new(), |m| m.values().map(|q| (q.copied, q.next)).collect());
    for (c, n) in pending {
        unsafe { wait_fence(c as *mut c_void, n) };
    }
}

pub unsafe fn migrate(real: *mut c_void, shadow_heap: usize, local_heap: usize) -> bool {
    if crate::dred::removed() {
        return false;
    }
    drain_queues();
    sync_up();
    sync_down();
    let Some(s) = shadows(|m| m.remove(&shadow_heap)) else { return false };
    let size = s.hi - s.lo;
    let vd = buf_desc(size);
    let mut lv: *mut c_void = core::ptr::null_mut();
    let hr = unsafe { vcall!(real, 29, fn(*mut c_void, u64, *const u8, u32, *const c_void, *const Guid, *mut *mut c_void) -> i32, local_heap as *mut c_void, 0, vd.as_ptr(), 0, core::ptr::null(), &IID_RESOURCE, &mut lv) };
    if hr < 0 || lv.is_null() {
        shadows(|m| m.insert(shadow_heap, s));
        return false;
    }
    let done = unsafe {
        with_tiles(real, |t| {
            t.copy(lv, s.view as *mut c_void, size);
            for (&res, &(a, b)) in &s.live {
                t.map(res as *mut c_void, tiles_for(b - a), local_heap as *mut c_void, (a / TILE) as u32);
            }
            t.flush()
        })
    };
    let Some(ok) = done else {
        unsafe { vcall!(lv, 2, fn() -> u32) };
        shadows(|m| m.insert(shadow_heap, s));
        return false;
    };
    unsafe {
        vcall!(lv, 2, fn() -> u32);
        let v = s.view as *mut c_void;
        vcall!(v, 9, fn(u32, *const c_void) -> (), 0, core::ptr::null());
        vcall!(v, 2, fn() -> u32);
    }
    if let Some(m) = RES.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        for r in s.live.keys() {
            m.remove(r);
        }
    }
    record("d3d12_xs", "title_shadow", &format!("title span {:#x}..{:#x} migrated to GPU-local heap {local_heap:#x} for a texture placement: {} buffer(s) remapped in place, bytes copied (ok={ok}); CPU access now through Map/Unmap", s.lo, s.hi, s.live.len()));
    true
}

pub fn merged_view(title: &[u8], snap: &[u8], shadow: &[u8]) -> Vec<u8> {
    (0..title.len()).map(|i| if title[i] != snap[i] { title[i] } else { shadow[i] }).collect()
}

pub fn read(addr: u64, len: usize) -> Option<Vec<u8>> {
    shadows(|m| {
        let s = m.values().find(|s| addr >= s.lo && addr + len as u64 <= s.hi && s.ranges().iter().any(|(a, b)| addr - s.lo >= *a && addr - s.lo + len as u64 <= *b))?;
        let off = (addr - s.lo) as usize;
        let sh = unsafe { core::slice::from_raw_parts((s.cpu + off) as *const u8, len) };
        let mut t = vec![0u8; len];
        if !read_title(addr, &mut t) {
            return Some(sh.to_vec());
        }
        Some(merged_view(&t, &s.snap[off..off + len], sh))
    })
}

pub fn forget_heap(heap: usize) {
    let Some(s) = shadows(|m| m.remove(&heap)) else { return };
    let v = s.view as *mut c_void;
    unsafe {
        vcall!(v, 9, fn(u32, *const c_void) -> (), 0, core::ptr::null());
        vcall!(v, 2, fn() -> u32);
    }
    if let Some(m) = RES.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        for r in s.live.keys() {
            m.remove(r);
        }
    }
}

pub fn sync_down() -> usize {
    let mut tmp = Vec::new();
    shadows(|m| m.values_mut().map(|s| unsafe { s.down(&mut tmp) }).sum())
}

pub fn sync_up() -> (usize, usize) {
    shadows(|m| m.values_mut().fold((0, 0), |acc, s| {
        let (w, l) = unsafe { s.up() };
        (acc.0 + w, acc.1 + l)
    }))
}

pub fn before_execute() {
    if count() == 0 {
        return;
    }
    let n = sync_down();
    static L: AtomicUsize = AtomicUsize::new(0);
    if n != 0 && L.fetch_add(1, Ordering::Relaxed) < 8 {
        record("d3d12_xs", "title_shadow", &format!("execute: {n} CPU-written title bytes merged into shadows"));
    }
}

struct QueueSync {
    done: usize,
    copied: usize,
    next: u64,
    tx: mpsc::Sender<u64>,
}

unsafe impl Send for QueueSync {}

static QUEUES: Mutex<Option<HashMap<usize, QueueSync>>> = Mutex::new(None);

type FenceOp = unsafe extern "system" fn(*mut c_void, *mut c_void, u64) -> i32;

unsafe fn worker(done: usize, copied: usize, rx: mpsc::Receiver<u64>) {
    let ev = unsafe { CreateEventW(core::ptr::null_mut(), 0, 0, core::ptr::null()) };
    while let Ok(n) = rx.recv() {
        let f = done as *mut c_void;
        if unsafe { vcall!(f, 8, fn() -> u64) } < n {
            if ev.is_null() || unsafe { vcall!(f, 9, fn(u64, *mut c_void) -> i32, n, ev) } < 0 {
                while unsafe { vcall!(f, 8, fn() -> u64) } < n {
                    std::thread::yield_now();
                }
            } else {
                unsafe { WaitForSingleObject(ev, INFINITE) };
            }
        }
        let (w, l) = sync_up();
        static L: AtomicUsize = AtomicUsize::new(0);
        if (w != 0 || l != 0) && L.fetch_add(1, Ordering::Relaxed) < 8 {
            record("d3d12_xs", "title_shadow", &format!("completion {n}: {w} GPU-written bytes copied to title memory, {l} dropped (title pages no longer committed)"));
        }
        unsafe { vcall!(copied as *mut c_void, 10, fn(u64) -> i32, n) };
    }
}

unsafe fn queue_sync(queue: *mut c_void) -> Option<(usize, usize, u64, mpsc::Sender<u64>)> {
    let mut g = QUEUES.lock().unwrap_or_else(|e| e.into_inner());
    let m = g.get_or_insert_with(HashMap::new);
    if let Some(q) = m.get_mut(&(queue as usize)) {
        q.next += 1;
        return Some((q.done, q.copied, q.next, q.tx.clone()));
    }
    let mut dev: *mut c_void = core::ptr::null_mut();
    if unsafe { vcall!(queue, 7, fn(*const Guid, *mut *mut c_void) -> i32, &IID_DEVICE, &mut dev) } < 0 || dev.is_null() {
        return None;
    }
    let mut done: *mut c_void = core::ptr::null_mut();
    let mut copied: *mut c_void = core::ptr::null_mut();
    let ok = unsafe { vcall!(dev, 36, fn(u64, u32, *const Guid, *mut *mut c_void) -> i32, 0, 0, &IID_FENCE, &mut done) } >= 0
        && unsafe { vcall!(dev, 36, fn(u64, u32, *const Guid, *mut *mut c_void) -> i32, 0, 0, &IID_FENCE, &mut copied) } >= 0;
    unsafe { vcall!(dev, 2, fn() -> u32) };
    if !ok {
        record("d3d12_xs", "title_shadow", "CreateFence for the shadow copy-back failed; GPU writes to shadowed title memory stay GPU-side");
        return None;
    }
    let (tx, rx) = mpsc::channel();
    let (d, c) = (done as usize, copied as usize);
    std::thread::Builder::new().name("tombox-title-shadow".into()).spawn(move || unsafe { worker(d, c, rx) }).ok()?;
    m.insert(queue as usize, QueueSync { done: d, copied: c, next: 1, tx: tx.clone() });
    Some((d, c, 1, tx))
}

pub unsafe fn after_execute(queue: *mut c_void, signal: usize, wait: usize) -> Option<u64> {
    if queue.is_null() || count() == 0 {
        return None;
    }
    let signal: FenceOp = unsafe { core::mem::transmute(if signal != 0 { signal } else { *vt(queue).add(14) }) };
    let wait: FenceOp = unsafe { core::mem::transmute(if wait != 0 { wait } else { *vt(queue).add(15) }) };
    let (done, copied, n, tx) = unsafe { queue_sync(queue) }?;
    if unsafe { signal(queue, done as *mut c_void, n) } < 0 {
        return None;
    }
    if tx.send(n).is_err() {
        return None;
    }
    unsafe { wait(queue, copied as *mut c_void, n) };
    Some(n)
}

#[cfg(test)]
#[path = "title_shadow_tests.rs"]
mod tests;

#[cfg(test)]
mod removed_gate {
    #[test]
    fn no_shadow_heap_or_migration_after_device_removal() {
        crate::dred::mark_removed();
        let h = unsafe { super::create_heap(core::ptr::null_mut(), 0x1000, 0x2000) };
        let m = unsafe { super::migrate(core::ptr::null_mut(), 1, 2) };
        crate::dred::clear_removed_for_test();
        assert!(h.is_none());
        assert!(!m);
    }
}
