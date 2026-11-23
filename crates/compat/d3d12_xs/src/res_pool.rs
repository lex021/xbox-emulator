use compat_common::record;
use core::ffi::c_void;
use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::collections::{BTreeMap, HashMap};
use std::sync::{Mutex, OnceLock};

const HEAP_BYTES: u64 = 64 << 20;
const BLOCK_ALIGN: u64 = 64 << 10;
const SMALL_ALIGN: u64 = 4 << 10;
const MAX_POOLED: u64 = 8 << 20;
const MAX_SLACK_BYTES: u64 = 512 << 20;
const HEAP_FLAG_NOT_ZEROED: u32 = 0x1000;
const HEAP_FLAG_ALLOW_ONLY_BUFFERS: u32 = 0xC0;
const HEAP_FLAG_ALLOW_ONLY_NON_RT_DS_TEXTURES: u32 = 0x44;
const RES_FLAG_RT: u32 = 0x1;
const RES_FLAG_DS: u32 = 0x2;
const RES_FLAG_SIMULTANEOUS: u32 = 0x20;
const DIM_BUFFER: u32 = 1;
const DIM_TEX1D: u32 = 2;
const DIM_TEX3D: u32 = 4;
const DEVICE_ALLOC_INFO: usize = 25;
const DEVICE_CREATE_HEAP: usize = 28;
const DEVICE_CREATE_PLACED: usize = 29;
const VT_SLOTS: usize = 8;
const SET_BITS: u32 = 17;
const SET_LEN: usize = 1 << SET_BITS;
const TOMB: usize = 1;

#[repr(C)]
struct Guid(u32, u16, u16, [u8; 8]);
const IID_HEAP: Guid = Guid(0x6b3b2502, 0x6e51, 0x45b3, [0x90, 0xee, 0x98, 0x84, 0x26, 0x5e, 0x8d, 0xf3]);
const IID_RESOURCE: Guid = Guid(0x696442be, 0xa72e, 0x4059, [0xbc, 0x79, 0x5b, 0x5c, 0x98, 0x04, 0x0f, 0xad]);

fn setting(env: Option<String>, cfg: Option<String>) -> (bool, &'static str) {
    let on = |v: &str| matches!(v.trim(), "1" | "on");
    let off = |v: &str| matches!(v.trim(), "0" | "off");
    if let Some(v) = env.as_deref().filter(|v| on(v) || off(v)) {
        return (on(v), "TOMBOX_RES_POOL");
    }
    if let Some(v) = cfg.as_deref().filter(|v| on(v) || off(v)) {
        return (on(v), "tombox.cfg res_pool");
    }
    (false, "default off")
}

pub fn enabled() -> bool {
    static E: OnceLock<bool> = OnceLock::new();
    *E.get_or_init(|| {
        let (on, src) = setting(std::env::var("TOMBOX_RES_POOL").ok(), compat_common::cfg("res_pool"));
        if on {
            record("d3d12_xs", "res_pool", &format!("committed buffers up to {} KiB and non-RT/DS default-heap textures are placed in pooled {} MiB heaps ({src})", MAX_POOLED >> 10, HEAP_BYTES >> 20));
        }
        on
    })
}

#[derive(Debug, Default)]
pub struct Arena {
    pub size: u64,
    clean: BTreeMap<u64, u64>,
    dirty: BTreeMap<u64, u64>,
    pub live: u32,
}

fn take(map: &mut BTreeMap<u64, u64>, size: u64, align: u64) -> Option<u64> {
    let (off, len, start) = map.iter().find_map(|(&off, &len)| {
        let start = (off + align - 1) & !(align - 1);
        (start + size <= off + len).then_some((off, len, start))
    })?;
    map.remove(&off);
    if start > off {
        map.insert(off, start - off);
    }
    let end = start + size;
    if end < off + len {
        map.insert(end, off + len - end);
    }
    Some(start)
}

fn put(map: &mut BTreeMap<u64, u64>, mut off: u64, mut size: u64) {
    if let Some((&p, &plen)) = map.range(..off).next_back() {
        if p + plen == off {
            map.remove(&p);
            off = p;
            size += plen;
        }
    }
    if let Some(&nlen) = map.get(&(off + size)) {
        map.remove(&(off + size));
        size += nlen;
    }
    map.insert(off, size);
}

impl Arena {
    pub fn new(size: u64) -> Self {
        let mut clean = BTreeMap::new();
        clean.insert(0, size);
        Arena { size, clean, dirty: BTreeMap::new(), live: 0 }
    }

    pub fn alloc(&mut self, size: u64, align: u64, dirty_ok: bool) -> Option<(u64, bool)> {
        let r = if dirty_ok {
            take(&mut self.dirty, size, align).map(|o| (o, true)).or_else(|| take(&mut self.clean, size, align).map(|o| (o, false)))
        } else {
            take(&mut self.clean, size, align).map(|o| (o, false))
        };
        if r.is_some() {
            self.live += 1;
        }
        r
    }

    pub fn free(&mut self, off: u64, size: u64) {
        put(&mut self.dirty, off, size);
        self.live = self.live.saturating_sub(1);
    }

    pub fn dirty_bytes(&self) -> u64 {
        self.dirty.values().sum()
    }

    pub fn clean_bytes(&self) -> u64 {
        self.clean.values().sum()
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Class {
    Buffer(u64),
    Texture,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Skip {
    NoDesc,
    RtDs,
    Msaa,
    TexHeap,
    Big,
    HeapType,
    HeapFlags,
}

pub fn classify(heap: &[u32; 5], heap_flags: u32, desc: &[u8]) -> Result<Class, Skip> {
    if desc.len() < 0x38 {
        return Err(Skip::NoDesc);
    }
    let rd32 = |o: usize| u32::from_le_bytes(desc[o..o + 4].try_into().unwrap());
    if !(1..=3).contains(&heap[0]) {
        return Err(Skip::HeapType);
    }
    if heap_flags & !HEAP_FLAG_NOT_ZEROED != 0 {
        return Err(Skip::HeapFlags);
    }
    let dim = rd32(0);
    if dim == DIM_BUFFER {
        let width = u64::from_le_bytes(desc[0x10..0x18].try_into().unwrap());
        if width == 0 || width > MAX_POOLED {
            return Err(Skip::Big);
        }
        return Ok(Class::Buffer((width + BLOCK_ALIGN - 1) & !(BLOCK_ALIGN - 1)));
    }
    if !(DIM_TEX1D..=DIM_TEX3D).contains(&dim) {
        return Err(Skip::NoDesc);
    }
    if heap[0] != 1 {
        return Err(Skip::TexHeap);
    }
    if rd32(0x30) & (RES_FLAG_RT | RES_FLAG_DS | RES_FLAG_SIMULTANEOUS) != 0 {
        return Err(Skip::RtDs);
    }
    if rd32(0x24) > 1 {
        return Err(Skip::Msaa);
    }
    Ok(Class::Texture)
}

pub fn class_heap_flags(c: Class) -> u32 {
    match c {
        Class::Buffer(_) => HEAP_FLAG_ALLOW_ONLY_BUFFERS,
        Class::Texture => HEAP_FLAG_ALLOW_ONLY_NON_RT_DS_TEXTURES,
    }
}

pub fn heap_desc(heap: &[u32; 5], flags: u32) -> [u8; 48] {
    let mut d = [0u8; 48];
    d[0..8].copy_from_slice(&HEAP_BYTES.to_le_bytes());
    d[8..12].copy_from_slice(&heap[0].to_le_bytes());
    d[12..16].copy_from_slice(&if heap[0] == 4 { heap[1] } else { 0 }.to_le_bytes());
    d[16..20].copy_from_slice(&if heap[0] == 4 { heap[2] } else { 0 }.to_le_bytes());
    d[20..24].copy_from_slice(&heap[3].to_le_bytes());
    d[24..28].copy_from_slice(&heap[4].to_le_bytes());
    d[32..40].copy_from_slice(&BLOCK_ALIGN.to_le_bytes());
    d[40..44].copy_from_slice(&flags.to_le_bytes());
    d
}

fn tex_reuse() -> bool {
    static E: OnceLock<bool> = OnceLock::new();
    *E.get_or_init(|| std::env::var("TOMBOX_RES_POOL_TEX_REUSE").map(|v| v != "0").unwrap_or(true))
}

pub fn within_slack(heaps: u64, live_bytes: u64) -> bool {
    (heaps * HEAP_BYTES).saturating_sub(live_bytes) <= MAX_SLACK_BYTES
}

pub fn sized_ok(size: u64, align: u64) -> bool {
    size != u64::MAX && size != 0 && size <= MAX_POOLED && align.is_power_of_two() && align <= BLOCK_ALIGN
}

pub struct PtrSet {
    slots: Box<[AtomicUsize]>,
    used: AtomicUsize,
    full: AtomicBool,
}

impl PtrSet {
    pub fn new(len: usize) -> Self {
        PtrSet { slots: (0..len).map(|_| AtomicUsize::new(0)).collect(), used: AtomicUsize::new(0), full: AtomicBool::new(false) }
    }

    fn start(&self, p: usize) -> usize {
        ((p >> 4).wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 20) & (self.slots.len() - 1)
    }

    pub fn insert(&self, p: usize) -> bool {
        if self.used.load(Ordering::Relaxed) * 4 >= self.slots.len() * 3 {
            self.full.store(true, Ordering::Release);
            return false;
        }
        let n = self.slots.len();
        let mut i = self.start(p);
        for _ in 0..n {
            let v = self.slots[i].load(Ordering::Acquire);
            if v == 0 || v == TOMB {
                self.slots[i].store(p, Ordering::Release);
                if v == 0 {
                    self.used.fetch_add(1, Ordering::Relaxed);
                }
                return true;
            }
            i = (i + 1) & (n - 1);
        }
        self.full.store(true, Ordering::Release);
        false
    }

    pub fn contains(&self, p: usize) -> bool {
        if self.full.load(Ordering::Acquire) {
            return true;
        }
        let n = self.slots.len();
        let mut i = self.start(p);
        for _ in 0..n {
            let v = self.slots[i].load(Ordering::Acquire);
            if v == p {
                return true;
            }
            if v == 0 {
                return false;
            }
            i = (i + 1) & (n - 1);
        }
        false
    }

    pub fn remove(&self, p: usize) {
        let n = self.slots.len();
        let mut i = self.start(p);
        for _ in 0..n {
            let v = self.slots[i].load(Ordering::Acquire);
            if v == p {
                self.slots[i].store(TOMB, Ordering::Release);
                return;
            }
            if v == 0 {
                return;
            }
            i = (i + 1) & (n - 1);
        }
    }
}

struct PoolHeap {
    key: ([u32; 5], u32),
    heap: usize,
    arena: Arena,
}

struct Live {
    key: ([u32; 5], u32),
    heap: usize,
    off: u64,
    size: u64,
}

#[derive(Default)]
struct Pool {
    heaps: Vec<PoolHeap>,
    live: HashMap<usize, Live>,
    live_bytes: HashMap<([u32; 5], u32), u64>,
}

static POOL: Mutex<Option<Pool>> = Mutex::new(None);
static SET: OnceLock<PtrSet> = OnceLock::new();
static VT: [AtomicUsize; VT_SLOTS] = [const { AtomicUsize::new(0) }; VT_SLOTS];
static ORIG_RELEASE: [AtomicUsize; VT_SLOTS] = [const { AtomicUsize::new(0) }; VT_SLOTS];
static STAT: [AtomicU64; 16] = [const { AtomicU64::new(0) }; 16];
const S_BUF: usize = 0;
const S_REUSED: usize = 1;
const S_TEX: usize = 2;
const S_TEX_SMALL: usize = 3;
const S_RTDS: usize = 4;
const S_MSAA: usize = 5;
const S_TEXHEAP: usize = 6;
const S_BIG: usize = 7;
const S_HEAPTYPE: usize = 8;
const S_FLAGS: usize = 9;
const S_UNSIZED: usize = 10;
const S_FAIL: usize = 11;
const S_BUDGET: usize = 12;
const S_FREED: usize = 13;
const S_HEAPS: usize = 14;
const S_OTHER: usize = 15;
const COUNTED: [usize; 12] = [S_BUF, S_TEX, S_RTDS, S_MSAA, S_TEXHEAP, S_BIG, S_HEAPTYPE, S_FLAGS, S_UNSIZED, S_FAIL, S_BUDGET, S_OTHER];

thread_local! {
    static INSIDE: core::cell::Cell<bool> = const { core::cell::Cell::new(false) };
}

fn set() -> &'static PtrSet {
    SET.get_or_init(|| PtrSet::new(SET_LEN))
}

unsafe fn vslot(o: usize, i: usize) -> usize {
    unsafe { *(*(o as *const *const usize)).add(i) }
}

fn bump(i: usize) {
    STAT[i].fetch_add(1, Ordering::Relaxed);
    if !COUNTED.contains(&i) {
        return;
    }
    let total: u64 = COUNTED.iter().map(|&k| STAT[k].load(Ordering::Relaxed)).sum();
    if total % 2000 == 0 {
        let s: Vec<u64> = STAT.iter().map(|a| a.load(Ordering::Relaxed)).collect();
        record("d3d12_xs", "res_pool", &format!("pooled buffers {} (reused {}) textures {} (4K-aligned {}) freed {} heaps {} | committed: rtds {} msaa {} texNonDefaultHeap {} big {} heapType {} heapFlags {} unsized {} budget {} placeFail {} other {}", s[S_BUF], s[S_REUSED], s[S_TEX], s[S_TEX_SMALL], s[S_FREED], s[S_HEAPS], s[S_RTDS], s[S_MSAA], s[S_TEXHEAP], s[S_BIG], s[S_HEAPTYPE], s[S_FLAGS], s[S_UNSIZED], s[S_BUDGET], s[S_FAIL], s[S_OTHER]));
    }
}

fn skip_stat(s: &Skip) -> usize {
    match s {
        Skip::NoDesc => S_OTHER,
        Skip::RtDs => S_RTDS,
        Skip::Msaa => S_MSAA,
        Skip::TexHeap => S_TEXHEAP,
        Skip::Big => S_BIG,
        Skip::HeapType => S_HEAPTYPE,
        Skip::HeapFlags => S_FLAGS,
    }
}

unsafe fn hook_release(res: usize) {
    let vtbl = unsafe { *(res as *const usize) };
    if VT.iter().any(|v| v.load(Ordering::Acquire) == vtbl) {
        return;
    }
    extern "system" {
        fn VirtualProtect(p: *mut c_void, n: usize, prot: u32, old: *mut u32) -> i32;
    }
    let Some(i) = (0..VT_SLOTS).find(|&i| VT[i].load(Ordering::Acquire) == 0) else { return };
    let p = vtbl as *mut usize;
    unsafe {
        ORIG_RELEASE[i].store(*p.add(2), Ordering::Release);
        VT[i].store(vtbl, Ordering::Release);
        let mut old = 0u32;
        if VirtualProtect(p.add(2) as *mut c_void, 8, 0x04, &mut old) != 0 {
            *p.add(2) = release_hook as usize;
            let mut tmp = 0u32;
            VirtualProtect(p.add(2) as *mut c_void, 8, old, &mut tmp);
            record("d3d12_xs", "res_pool", &format!("resource vtable {vtbl:#x} Release hooked (#{i})"));
        } else {
            VT[i].store(0, Ordering::Release);
        }
    }
}

fn orig_release(res: *mut c_void) -> usize {
    let vtbl = unsafe { *(res as *const usize) };
    (0..VT_SLOTS).find(|&i| VT[i].load(Ordering::Acquire) == vtbl).map(|i| ORIG_RELEASE[i].load(Ordering::Acquire)).unwrap_or(0)
}

unsafe extern "system" fn release_hook(res: *mut c_void) -> u32 {
    let f: unsafe extern "system" fn(*mut c_void) -> u32 = unsafe { core::mem::transmute(orig_release(res)) };
    if INSIDE.with(|c| c.get()) || !set().contains(res as usize) {
        return unsafe { f(res) };
    }
    let mut g = POOL.lock().unwrap_or_else(|e| e.into_inner());
    INSIDE.with(|c| c.set(true));
    let n = unsafe { f(res) };
    INSIDE.with(|c| c.set(false));
    if n == 0 {
        if let Some(p) = g.as_mut() {
            if let Some(l) = p.live.remove(&(res as usize)) {
                set().remove(res as usize);
                if let Some(b) = p.live_bytes.get_mut(&l.key) {
                    *b = b.saturating_sub(l.size);
                }
                bump(S_FREED);
                if let Some(h) = p.heaps.iter_mut().find(|h| h.heap == l.heap) {
                    h.arena.free(l.off, l.size);
                }
                retire_empty(p);
            }
        }
    }
    n
}

fn retire_empty(p: &mut Pool) {
    let mut i = 0;
    while i < p.heaps.len() {
        let h = &p.heaps[i];
        let spare = p.heaps.iter().filter(|o| o.key == h.key && o.heap != h.heap).count();
        if h.arena.live == 0 && h.arena.dirty_bytes() > 0 && (h.arena.clean_bytes() < HEAP_BYTES / 4 || spare > 0) {
            let heap = h.heap;
            INSIDE.with(|c| c.set(true));
            unsafe {
                let rel: unsafe extern "system" fn(usize) -> u32 = core::mem::transmute(vslot(heap, 2));
                rel(heap);
            }
            INSIDE.with(|c| c.set(false));
            p.heaps.swap_remove(i);
            continue;
        }
        i += 1;
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct AllocInfo {
    pub size: u64,
    pub align: u64,
}

pub unsafe fn alloc_info(real: *mut c_void, desc: &[u8; 0x40]) -> AllocInfo {
    let f: unsafe extern "system" fn(*mut c_void, *mut AllocInfo, u32, u32, *const u8) -> *mut AllocInfo = unsafe { core::mem::transmute(vslot(real as usize, DEVICE_ALLOC_INFO)) };
    let mut out = AllocInfo::default();
    unsafe { f(real, &mut out, 0, 1, desc.as_ptr()) };
    out
}

pub unsafe fn size_texture(real: *mut c_void, d: &mut [u8; 0x40]) -> Option<(u64, u64, bool)> {
    d[8..16].copy_from_slice(&SMALL_ALIGN.to_le_bytes());
    let s = unsafe { alloc_info(real, d) };
    if s.align == SMALL_ALIGN && sized_ok(s.size, s.align) {
        return Some((s.size, SMALL_ALIGN, true));
    }
    d[8..16].copy_from_slice(&0u64.to_le_bytes());
    let s = unsafe { alloc_info(real, d) };
    sized_ok(s.size, s.align).then_some(((s.size + s.align - 1) & !(s.align - 1), s.align.max(SMALL_ALIGN), false))
}

pub unsafe fn try_create(real: *mut c_void, heap: *const u32, heap_flags: u32, desc: *const u32, init_state: u32, clear: *const c_void, ppv: *mut *mut c_void) -> Option<i32> {
    if heap.is_null() || desc.is_null() || ppv.is_null() || !enabled() || crate::dred::removed() {
        return None;
    }
    let hkey: [u32; 5] = unsafe { core::ptr::read_unaligned(heap as *const [u32; 5]) };
    let mut d: [u8; 0x40] = unsafe { core::ptr::read_unaligned(desc as *const [u8; 0x40]) };
    let class = match classify(&hkey, heap_flags, &d) {
        Ok(c) => c,
        Err(s) => {
            bump(skip_stat(&s));
            return None;
        }
    };
    let (size, align, small) = match class {
        Class::Buffer(s) => {
            d[8..16].copy_from_slice(&0u64.to_le_bytes());
            (s, BLOCK_ALIGN, false)
        }
        Class::Texture => match unsafe { size_texture(real, &mut d) } {
            Some(v) => v,
            None => {
                static ONCE: AtomicUsize = AtomicUsize::new(0);
                if ONCE.fetch_add(1, Ordering::Relaxed) < 4 {
                    let r32 = |o: usize| u32::from_le_bytes(d[o..o + 4].try_into().unwrap());
                    record("d3d12_xs", "res_pool", &format!("texture dim={} {}x{} arr={} mips={} fmt={} flags={:#x} could not be sized (or is over {} KiB); committed instead", r32(0), u64::from_le_bytes(d[0x10..0x18].try_into().unwrap()), r32(0x18), u16::from_le_bytes([d[0x1c], d[0x1d]]), u16::from_le_bytes([d[0x1e], d[0x1f]]), r32(0x20), r32(0x30), MAX_POOLED >> 10));
                }
                bump(S_UNSIZED);
                return None;
            }
        },
    };
    let key = (hkey, class_heap_flags(class));
    let cpu_visible = hkey[0] == 2 || hkey[0] == 3;
    let dirty_ok = heap_flags & HEAP_FLAG_NOT_ZEROED != 0 || (cpu_visible && matches!(class, Class::Buffer(_))) || (class == Class::Texture && tex_reuse());
    let mut g = POOL.lock().unwrap_or_else(|e| e.into_inner());
    let p = g.get_or_insert_with(Pool::default);
    let mut slot = p.heaps.iter_mut().filter(|h| h.key == key).find_map(|h| h.arena.alloc(size, align, dirty_ok).map(|(o, dirty)| (h.heap, o, dirty)));
    if slot.is_none() {
        let class_heaps = p.heaps.iter().filter(|h| h.key == key).count() as u64;
        if !within_slack(class_heaps + 1, p.live_bytes.get(&key).copied().unwrap_or(0) + size) {
            bump(S_BUDGET);
            return None;
        }
        let hd = heap_desc(&hkey, key.1);
        let mut hp: *mut c_void = core::ptr::null_mut();
        let ch: unsafe extern "system" fn(*mut c_void, *const u8, *const Guid, *mut *mut c_void) -> i32 = unsafe { core::mem::transmute(vslot(real as usize, DEVICE_CREATE_HEAP)) };
        INSIDE.with(|c| c.set(true));
        let ev = crate::frame_ev::begin(crate::frame_ev::HEAP);
        let hr = unsafe { ch(real, hd.as_ptr(), &IID_HEAP, &mut hp) };
        drop(ev);
        INSIDE.with(|c| c.set(false));
        if hr < 0 || hp.is_null() {
            bump(S_FAIL);
            record("d3d12_xs", "res_pool", &format!("CreateHeap {key:?} failed hr={hr:#010x}; committed instead"));
            return None;
        }
        bump(S_HEAPS);
        let mut arena = Arena::new(HEAP_BYTES);
        let o = arena.alloc(size, align, false).map(|(o, _)| o);
        p.heaps.push(PoolHeap { key, heap: hp as usize, arena });
        slot = o.map(|o| (hp as usize, o, false));
    }
    let (hp, off, dirty) = slot?;
    let cp: unsafe extern "system" fn(*mut c_void, usize, u64, *const u8, u32, *const c_void, *const Guid, *mut *mut c_void) -> i32 = unsafe { core::mem::transmute(vslot(real as usize, DEVICE_CREATE_PLACED)) };
    INSIDE.with(|c| c.set(true));
    let ev = crate::frame_ev::begin(crate::frame_ev::PLACED);
    let hr = unsafe { cp(real, hp, off, d.as_ptr(), init_state, clear, &IID_RESOURCE, ppv) };
    drop(ev);
    INSIDE.with(|c| c.set(false));
    let res = unsafe { *ppv };
    if hr < 0 || res.is_null() {
        if let Some(h) = p.heaps.iter_mut().find(|h| h.heap == hp) {
            h.arena.free(off, size);
        }
        bump(S_FAIL);
        static N: AtomicUsize = AtomicUsize::new(0);
        if N.fetch_add(1, Ordering::Relaxed) < 8 {
            record("d3d12_xs", "res_pool", &format!("CreatePlacedResource {class:?} heap {key:?} offset {off:#x} size {size:#x} align {align:#x} state {init_state:#x} failed hr={hr:#010x}; committed instead"));
        }
        return None;
    }
    if dirty && cpu_visible && heap_flags & HEAP_FLAG_NOT_ZEROED == 0 {
        let width = u64::from_le_bytes(d[0x10..0x18].try_into().unwrap()) as usize;
        let mut m: *mut u8 = core::ptr::null_mut();
        INSIDE.with(|c| c.set(true));
        unsafe {
            let map: unsafe extern "system" fn(*mut c_void, u32, *const c_void, *mut *mut u8) -> i32 = core::mem::transmute(vslot(res as usize, 8));
            if map(res, 0, core::ptr::null(), &mut m) >= 0 && !m.is_null() {
                core::ptr::write_bytes(m, 0, width);
                let unmap: unsafe extern "system" fn(*mut c_void, u32, *const c_void) = core::mem::transmute(vslot(res as usize, 9));
                unmap(res, 0, core::ptr::null());
            }
        }
        INSIDE.with(|c| c.set(false));
    }
    unsafe { hook_release(res as usize) };
    p.live.insert(res as usize, Live { key, heap: hp, off, size });
    *p.live_bytes.entry(key).or_insert(0) += size;
    set().insert(res as usize);
    match class {
        Class::Buffer(_) => bump(S_BUF),
        Class::Texture => {
            if small {
                STAT[S_TEX_SMALL].fetch_add(1, Ordering::Relaxed);
            }
            bump(S_TEX)
        }
    }
    if dirty {
        STAT[S_REUSED].fetch_add(1, Ordering::Relaxed);
    }
    Some(hr)
}

#[cfg(test)]
mod tests {
    #[test]
    fn setting_env_over_cfg() {
        let s = |e: Option<&str>, c: Option<&str>| setting(e.map(String::from), c.map(String::from)).0;
        assert!(!s(None, None));
        assert!(s(None, Some("on")));
        assert!(s(None, Some("1")));
        assert!(!s(None, Some("off")));
        assert!(!s(Some("0"), Some("on")));
        assert!(s(Some("1"), Some("off")));
        assert!(s(Some(""), Some("on")));
        assert!(!s(None, Some("maybe")));
    }

    use super::*;

    fn buf(width: u64) -> [u8; 0x40] {
        let mut d = [0u8; 0x40];
        d[0..4].copy_from_slice(&1u32.to_le_bytes());
        d[0x10..0x18].copy_from_slice(&width.to_le_bytes());
        d
    }

    fn tex(w: u64, h: u32, fmt: u32, flags: u32, samples: u32) -> [u8; 0x40] {
        let mut d = [0u8; 0x40];
        d[0..4].copy_from_slice(&3u32.to_le_bytes());
        d[0x10..0x18].copy_from_slice(&w.to_le_bytes());
        d[0x18..0x1c].copy_from_slice(&h.to_le_bytes());
        d[0x1c..0x1e].copy_from_slice(&1u16.to_le_bytes());
        d[0x1e..0x20].copy_from_slice(&1u16.to_le_bytes());
        d[0x20..0x24].copy_from_slice(&fmt.to_le_bytes());
        d[0x24..0x28].copy_from_slice(&samples.to_le_bytes());
        d[0x30..0x34].copy_from_slice(&flags.to_le_bytes());
        d
    }

    #[test]
    fn arena_allocates_aligned_and_in_order() {
        let mut a = Arena::new(1 << 20);
        assert_eq!(a.alloc(0x10000, 0x10000, false), Some((0, false)));
        assert_eq!(a.alloc(0x20000, 0x10000, false), Some((0x10000, false)));
        assert_eq!(a.live, 2);
        assert_eq!(a.clean_bytes(), (1 << 20) - 0x30000);
    }

    #[test]
    fn freed_blocks_are_dirty_and_only_reused_when_allowed() {
        let mut a = Arena::new(0x40000);
        let (o, _) = a.alloc(0x10000, 0x10000, false).unwrap();
        a.free(o, 0x10000);
        assert_eq!(a.live, 0);
        assert_eq!(a.dirty_bytes(), 0x10000);
        assert_eq!(a.alloc(0x10000, 0x10000, false), Some((0x10000, false)));
        assert_eq!(a.alloc(0x10000, 0x10000, true), Some((0, true)));
    }

    #[test]
    fn exhausted_clean_space_is_not_filled_from_dirty() {
        let mut a = Arena::new(0x20000);
        let (o1, _) = a.alloc(0x10000, 0x10000, false).unwrap();
        a.alloc(0x10000, 0x10000, false).unwrap();
        a.free(o1, 0x10000);
        assert_eq!(a.alloc(0x10000, 0x10000, false), None);
        assert_eq!(a.alloc(0x10000, 0x10000, true), Some((o1, true)));
    }

    #[test]
    fn dirty_neighbours_coalesce() {
        let mut a = Arena::new(0x40000);
        let o: Vec<u64> = (0..4).map(|_| a.alloc(0x10000, 0x10000, false).unwrap().0).collect();
        a.free(o[1], 0x10000);
        a.free(o[3], 0x10000);
        a.free(o[2], 0x10000);
        assert_eq!(a.alloc(0x30000, 0x10000, true), Some((0x10000, true)));
        assert_eq!(a.live, 2);
    }

    #[test]
    fn small_and_large_alignments_share_an_arena() {
        let mut a = Arena::new(0x100000);
        assert_eq!(a.alloc(0x1000, SMALL_ALIGN, false), Some((0, false)));
        assert_eq!(a.alloc(0x10000, BLOCK_ALIGN, false), Some((0x10000, false)));
        assert_eq!(a.alloc(0x3000, SMALL_ALIGN, false), Some((0x1000, false)));
        assert_eq!(a.alloc(0x2000, SMALL_ALIGN, false), Some((0x4000, false)));
    }

    #[test]
    fn classify_buffers() {
        let up = [2u32, 0, 0, 1, 1];
        let def = [1u32, 0, 0, 1, 1];
        assert_eq!(classify(&up, 0, &buf(100)), Ok(Class::Buffer(0x10000)));
        assert_eq!(classify(&def, 0, &buf(0x10001)), Ok(Class::Buffer(0x20000)));
        assert_eq!(classify(&def, HEAP_FLAG_NOT_ZEROED, &buf(64)), Ok(Class::Buffer(0x10000)));
        assert_eq!(classify(&def, 0, &buf(MAX_POOLED + 1)), Err(Skip::Big));
        assert_eq!(classify(&def, 0, &buf(0)), Err(Skip::Big));
        assert_eq!(classify(&[4, 3, 1, 1, 1], 0, &buf(64)), Err(Skip::HeapType));
        assert_eq!(classify(&def, 0x20, &buf(64)), Err(Skip::HeapFlags));
        assert_eq!(classify(&def, 0, &[0u8; 8]), Err(Skip::NoDesc));
    }

    #[test]
    fn classify_textures() {
        let def = [1u32, 0, 0, 1, 1];
        assert_eq!(classify(&def, 0, &tex(256, 256, 28, 0, 1)), Ok(Class::Texture));
        assert_eq!(classify(&def, 0, &tex(256, 256, 28, 4, 1)), Ok(Class::Texture));
        assert_eq!(classify(&def, 0, &tex(256, 256, 28, RES_FLAG_RT, 1)), Err(Skip::RtDs));
        assert_eq!(classify(&def, 0, &tex(256, 256, 45, RES_FLAG_DS, 1)), Err(Skip::RtDs));
        assert_eq!(classify(&def, 0, &tex(256, 256, 28, RES_FLAG_SIMULTANEOUS, 1)), Err(Skip::RtDs));
        assert_eq!(classify(&def, 0, &tex(256, 256, 28, 0, 4)), Err(Skip::Msaa));
        assert_eq!(classify(&[2, 0, 0, 1, 1], 0, &tex(256, 256, 28, 0, 1)), Err(Skip::TexHeap));
    }

    #[test]
    fn heap_desc_layout() {
        let d = heap_desc(&[2, 9, 9, 1, 1], HEAP_FLAG_ALLOW_ONLY_BUFFERS);
        assert_eq!(u64::from_le_bytes(d[0..8].try_into().unwrap()), HEAP_BYTES);
        assert_eq!(u32::from_le_bytes(d[8..12].try_into().unwrap()), 2);
        assert_eq!(u32::from_le_bytes(d[12..16].try_into().unwrap()), 0);
        assert_eq!(u32::from_le_bytes(d[16..20].try_into().unwrap()), 0);
        assert_eq!(u32::from_le_bytes(d[20..24].try_into().unwrap()), 1);
        assert_eq!(u64::from_le_bytes(d[32..40].try_into().unwrap()), BLOCK_ALIGN);
        assert_eq!(u32::from_le_bytes(d[40..44].try_into().unwrap()), HEAP_FLAG_ALLOW_ONLY_BUFFERS);
        assert_eq!(class_heap_flags(Class::Texture), HEAP_FLAG_ALLOW_ONLY_NON_RT_DS_TEXTURES);
        let c = heap_desc(&[4, 3, 1, 1, 1], 0);
        assert_eq!(u32::from_le_bytes(c[12..16].try_into().unwrap()), 3);
    }

    #[test]
    fn slack_budget_tracks_waste_not_total() {
        assert!(within_slack(40, 40 * HEAP_BYTES - (100 << 20)));
        assert!(within_slack(1, 0x10000));
        assert!(!within_slack(20, 0x10000));
        assert!(within_slack(9, HEAP_BYTES));
        assert!(!within_slack(10, HEAP_BYTES));
    }

    #[test]
    fn sized_ok_rules() {
        assert!(sized_ok(0x1000, 0x1000));
        assert!(sized_ok(0x20000, 0x10000));
        assert!(!sized_ok(u64::MAX, 0x10000));
        assert!(!sized_ok(0, 0x10000));
        assert!(!sized_ok(0x1000, 0x400000));
        assert!(!sized_ok(MAX_POOLED + 1, 0x10000));
        assert!(!sized_ok(0x1000, 0x3000));
    }

    #[test]
    fn ptr_set_insert_contains_remove() {
        let s = PtrSet::new(64);
        let ps: Vec<usize> = (0..20).map(|i| 0x7100_0000 + i * 0x990).collect();
        for &p in &ps {
            assert!(s.insert(p));
        }
        for &p in &ps {
            assert!(s.contains(p));
        }
        assert!(!s.contains(0x5555_0000));
        s.remove(ps[3]);
        assert!(!s.contains(ps[3]));
        for (i, &p) in ps.iter().enumerate() {
            assert_eq!(s.contains(p), i != 3);
        }
        assert!(s.insert(ps[3]));
        assert!(s.contains(ps[3]));
    }

    #[test]
    fn ptr_set_full_reports_everything_present() {
        let s = PtrSet::new(8);
        for i in 0..6 {
            s.insert(0x1000 + i * 16);
        }
        assert!(!s.insert(0x9000));
        assert!(s.contains(0xdead_0000));
    }

    fn device() -> Option<*mut c_void> {
        #[repr(C)]
        struct G(u32, u16, u16, [u8; 8]);
        let iid = G(0x189819f1, 0x1db6, 0x4b57, [0xbe, 0x54, 0x18, 0x21, 0x33, 0x9b, 0x85, 0xf7]);
        let p = unsafe { crate::host_proc(b"D3D12CreateDevice\0") };
        if p == 0 {
            return None;
        }
        let create: extern "system" fn(*mut c_void, u32, *const G, *mut *mut c_void) -> i32 = unsafe { core::mem::transmute(p) };
        let mut dev: *mut c_void = core::ptr::null_mut();
        (create(core::ptr::null_mut(), 0xb000, &iid, &mut dev) >= 0 && !dev.is_null()).then_some(dev)
    }

    #[test]
    fn hw_sizes_and_places_textures() {
        let Some(dev) = device() else { return };
        let mut small = tex(64, 64, 28, 0, 1);
        let (s, a, is_small) = unsafe { size_texture(dev, &mut small) }.expect("64x64 rgba8 sized");
        assert!(is_small && a == SMALL_ALIGN && s >= 64 * 64 * 4, "small texture: size {s:#x} align {a:#x}");
        let mut big = tex(1024, 1024, 28, 0, 1);
        let (s2, a2, is_small2) = unsafe { size_texture(dev, &mut big) }.expect("1024^2 rgba8 sized");
        assert!(!is_small2 && a2 == BLOCK_ALIGN && s2 >= 1024 * 1024 * 4, "big texture: size {s2:#x} align {a2:#x}");
        let hd = heap_desc(&[1, 0, 0, 1, 1], HEAP_FLAG_ALLOW_ONLY_NON_RT_DS_TEXTURES);
        let mut hp: *mut c_void = core::ptr::null_mut();
        let ch: unsafe extern "system" fn(*mut c_void, *const u8, *const Guid, *mut *mut c_void) -> i32 = unsafe { core::mem::transmute(vslot(dev as usize, DEVICE_CREATE_HEAP)) };
        assert!(unsafe { ch(dev, hd.as_ptr(), &IID_HEAP, &mut hp) } >= 0 && !hp.is_null());
        let cp: unsafe extern "system" fn(*mut c_void, usize, u64, *const u8, u32, *const c_void, *const Guid, *mut *mut c_void) -> i32 = unsafe { core::mem::transmute(vslot(dev as usize, DEVICE_CREATE_PLACED)) };
        let mut arena = Arena::new(HEAP_BYTES);
        for d in [&small, &big, &small] {
            let (sz, al) = if u64::from_le_bytes(d[8..16].try_into().unwrap()) == SMALL_ALIGN { (s, a) } else { (s2, a2) };
            let (off, _) = arena.alloc(sz, al, false).unwrap();
            let mut r: *mut c_void = core::ptr::null_mut();
            let hr = unsafe { cp(dev, hp as usize, off, d.as_ptr(), 0x400, core::ptr::null(), &IID_RESOURCE, &mut r) };
            assert!(hr >= 0 && !r.is_null(), "CreatePlacedResource off {off:#x} hr={hr:#010x}");
            unsafe {
                let rel: unsafe extern "system" fn(*mut c_void) -> u32 = core::mem::transmute(vslot(r as usize, 2));
                rel(r);
            }
        }
    }
}

#[cfg(test)]
mod removed_gate {
    #[test]
    fn pool_declines_after_device_removal() {
        let heap = [1u32, 0, 0, 1, 1];
        let desc = [0u32; 16];
        let mut out: *mut core::ffi::c_void = core::ptr::null_mut();
        crate::dred::mark_removed();
        let r = unsafe { super::try_create(core::ptr::null_mut(), heap.as_ptr(), 0, desc.as_ptr(), 0, core::ptr::null(), &mut out) };
        crate::dred::clear_removed_for_test();
        assert!(r.is_none());
        assert!(out.is_null());
    }
}
