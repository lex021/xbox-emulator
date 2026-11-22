use core::ffi::c_void;
use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::collections::HashMap;
use std::sync::Mutex;
use compat_common::record;

use crate::console_slots_cl2::{BundleRecord, BUNDLE_RECORD_MAGIC, BUNDLE_RECORD_SIZE};
use crate::device_wrap::Guid;

pub const HLSL: &str = r#"#define RS "RootConstants(num32BitConstants=10, b0), UAV(u0), UAV(u1)"
cbuffer C : register(b0) { uint n; uint in_base; uint pred_base; uint cand_base; uint max_size; uint has_size; uint title_op; uint magic; uint nlive; uint gx; };
RWByteAddressBuffer inp : register(u0);
RWByteAddressBuffer pred : register(u1);
[RootSignature(RS)]
[numthreads(64, 1, 1)]
void main(uint3 g : SV_GroupID, uint3 t : SV_GroupThreadID) {
    uint i = (g.y * gx + g.x) * 64 + t.x;
    if (i >= n) return;
    uint slot = i / nlive;
    uint2 b = inp.Load2(cand_base + (i % nlive) * 8);
    uint size = max_size;
    if (has_size != 0) size = min(inp.Load(in_base), max_size);
    uint4 r = inp.Load4(in_base + 16 + slot * 16);
    bool run = (slot + 1) * 16 <= size && r.x == magic && r.y == 16 && r.z == b.x && r.w == b.y;
    if (title_op < 2) {
        uint2 p = inp.Load2(in_base + 8);
        bool zero = (p.x | p.y) == 0;
        run = run && (title_op == 0 ? !zero : zero);
    }
    pred.Store2(pred_base + i * 8, uint2(run ? 1u : 0u, 0u));
}
"#;

pub const TITLE_OP_NONE: u32 = 2;
pub const GROUP: u64 = 64;
pub const MAX_GROUPS_X: u64 = 65535;
pub const MAX_PAIRS: u64 = (u32::MAX as u64 - RING_ALIGN) / 8;
pub const MIN_SCRATCH: u64 = 64 << 10;
const RING_ALIGN: u64 = 256;
const N_CONSTANTS: u32 = 10;

const ST_COMMON: u32 = 0;
const ST_UAV: u32 = 0x8;
const ST_PREDICATION: u32 = 0x200;
const ST_COPY_DEST: u32 = 0x400;
const ST_COPY_SOURCE: u32 = 0x800;
const HEAP_DEFAULT: u32 = 1;
const FLAG_UAV: u32 = 4;
const PC_DISPATCH: usize = 14;
const PC_COPY_BUFFER_REGION: usize = 15;
const PC_SET_PSO: usize = 25;
const PC_EXECUTE_BUNDLE: usize = 27;
const PC_SET_COMPUTE_ROOT_SIGNATURE: usize = 29;
const PC_SET_COMPUTE_ROOT_CONSTANTS: usize = 35;
const PC_SET_COMPUTE_ROOT_UAV: usize = 41;
const PC_SET_PREDICATION: usize = 55;
const PC_WRITE_BUFFER_IMMEDIATE: usize = 66;
const DEV_CREATE_COMPUTE_PSO: usize = 11;
const DEV_CREATE_ROOT_SIGNATURE: usize = 16;
const DEV_CREATE_COMMITTED: usize = 27;
const RES_GPU_VA: usize = 11;
const IID_DEVICE: Guid = Guid { d1: 0x1898_19f1, d2: 0x1db6, d3: 0x4b57, d4: [0xbe, 0x54, 0x18, 0x21, 0x33, 0x9b, 0x85, 0xf7] };
const IID_ROOT_SIGNATURE: Guid = Guid { d1: 0xc54a_6b66, d2: 0x72df, d3: 0x4ee8, d4: [0x8b, 0xe5, 0xa9, 0x46, 0xa1, 0x42, 0x92, 0x14] };
const IID_PSO: Guid = Guid { d1: 0x765a_30f3, d2: 0xf624, d3: 0x4c6f, d4: [0xa8, 0x28, 0xac, 0xe9, 0x48, 0x62, 0x24, 0x45] };
const IID_RESOURCE: Guid = Guid { d1: 0x6964_42be, d2: 0xa72e, d3: 0x4059, d4: [0xbc, 0x79, 0x5b, 0x5c, 0x98, 0x04, 0x0f, 0xad] };

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pair {
    pub slot: u32,
    pub bundle: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    pub cand: u64,
    pub in_size: u64,
    pub pred_size: u64,
}

pub fn slot_count(width: u64, offset: u64, max_size: u32) -> u32 {
    (width.saturating_sub(offset).min(max_size as u64) / BUNDLE_RECORD_SIZE as u64) as u32
}

pub fn pair_count(slots: u32, live: usize) -> u64 {
    slots as u64 * live as u64
}

pub fn pair_of(i: u64, live: &[usize]) -> Pair {
    let l = live.len() as u64;
    Pair { slot: (i / l) as u32, bundle: live[(i % l) as usize] }
}

pub fn runs(p: Pair, size_word: Option<u32>, max_size: u32, rec: &BundleRecord, title: Option<(u32, u64)>) -> bool {
    let size = size_word.map_or(max_size, |s| s.min(max_size)) as u64;
    let hit = (p.slot as u64 + 1) * BUNDLE_RECORD_SIZE as u64 <= size && rec.magic == BUNDLE_RECORD_MAGIC && rec.size == BUNDLE_RECORD_SIZE && rec.bundle == p.bundle as u64;
    let pass = match title {
        None => true,
        Some((0, v)) => v != 0,
        Some((_, v)) => v == 0,
    };
    hit && pass
}

pub fn candidate_writes(base_va: u64, live: &[usize]) -> Vec<[u64; 2]> {
    let mut v = Vec::with_capacity(live.len() * 2);
    for (i, &b) in live.iter().enumerate() {
        let a = base_va + i as u64 * 8;
        v.push([a, b as u64 & 0xffff_ffff]);
        v.push([a + 4, b as u64 >> 32]);
    }
    v
}

pub fn layout(slots: u32, live: usize, pairs: u64) -> Layout {
    let cand = 16 + slots as u64 * BUNDLE_RECORD_SIZE as u64;
    Layout { cand, in_size: cand + live as u64 * 8, pred_size: pairs * 8 }
}

pub fn dispatch_dims(pairs: u64) -> (u32, u32) {
    let groups = pairs.div_ceil(GROUP);
    if groups == 0 {
        return (0, 0);
    }
    let gx = groups.min(MAX_GROUPS_X);
    (gx as u32, groups.div_ceil(gx) as u32)
}

pub fn constants(n: u64, in_base: u64, pred_base: u64, max_size: u32, has_size: bool, title_op: Option<u32>, l: &Layout, nlive: usize) -> [u32; 10] {
    let gx = dispatch_dims(n).0;
    [n as u32, in_base as u32, pred_base as u32, (in_base + l.cand) as u32, max_size, has_size as u32, title_op.map_or(TITLE_OP_NONE, |o| o.min(1)), BUNDLE_RECORD_MAGIC, nlive as u32, gx]
}

pub fn grown(cap: u64, need: u64) -> u64 {
    if need <= cap { cap } else { need.next_power_of_two().max(MIN_SCRATCH).max(cap.saturating_mul(2)) }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Ring {
    pub cap: u64,
    pub next: u64,
}

impl Ring {
    pub fn alloc(&mut self, need: u64) -> Option<u64> {
        let need = need.div_ceil(RING_ALIGN) * RING_ALIGN;
        if need > self.cap {
            return None;
        }
        if self.next + need > self.cap {
            self.next = 0;
        }
        let o = self.next;
        self.next += need;
        Some(o)
    }
}

pub fn source_barrier(state: u32) -> Option<u32> {
    (state & ST_COPY_SOURCE == 0).then_some(state)
}

pub fn distinct(v: &[usize]) -> Vec<usize> {
    let mut out: Vec<usize> = Vec::new();
    for &x in v {
        if x != 0 && !out.contains(&x) {
            out.push(x);
        }
    }
    out
}

pub fn new_slots(before: &[Vec<u32>], after: &[Vec<u32>]) -> Vec<Option<u32>> {
    after.iter().enumerate().map(|(i, a)| (a.len() > before.get(i).map_or(0, |b| b.len())).then(|| *a.last().unwrap())).collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Call {
    pub bundle: usize,
    pub width: u64,
    pub offset: u64,
    pub max_size: u32,
    pub size_buf: usize,
    pub size_off: u32,
    pub title: Option<(usize, u64, u32)>,
    pub srcs: Vec<(usize, u32)>,
    pub live: Vec<usize>,
    pub gens: Vec<u64>,
    pub qslots: Vec<Option<u32>>,
    pub carried: Option<Vec<usize>>,
}

pub fn executed_in_order(recs: &[BundleRecord], live: &[usize]) -> Vec<usize> {
    recs.iter().filter(|r| r.magic == BUNDLE_RECORD_MAGIC && r.size == BUNDLE_RECORD_SIZE && live.binary_search(&(r.bundle as usize)).is_ok()).map(|r| r.bundle as usize).collect()
}

pub fn cpu_determined(size_buf: usize, title: Option<(usize, u64, u32)>, records_gpu_written: bool) -> bool {
    size_buf == 0 && title.is_none() && !records_gpu_written
}

unsafe fn predict(c: &Call) -> Option<Vec<usize>> {
    if !cpu_determined(c.size_buf, c.title, crate::placed::gpu_written(c.bundle)) {
        return None;
    }
    let p = crate::placed::lookup(c.bundle as *mut c_void)?;
    let base = (p.va + c.offset) as usize;
    let slots = slot_count(c.width, c.offset, c.max_size) as usize;
    let recs: Vec<BundleRecord> = (0..slots).map(|i| unsafe { core::ptr::read_unaligned((base + i * BUNDLE_RECORD_SIZE as usize) as *const BundleRecord) }).collect();
    Some(executed_in_order(&recs, &c.live))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Scratch {
    inp: usize,
    inp_va: u64,
    pred: usize,
    pred_va: u64,
    in_cap: u64,
    pred_cap: u64,
}

struct Where {
    s: Scratch,
    in_off: u64,
    pred_off: u64,
}

struct Shared {
    s: Option<Scratch>,
    rings: [Ring; 2],
    retired: Vec<Scratch>,
}

static PIPE: Mutex<Option<Option<(usize, usize)>>> = Mutex::new(None);
static OWNED: Mutex<Option<HashMap<usize, Scratch>>> = Mutex::new(None);
static SHARED: Mutex<Shared> = Mutex::new(Shared { s: None, rings: [Ring { cap: 0, next: 0 }, Ring { cap: 0, next: 0 }], retired: Vec::new() });
static DEFER: Mutex<Option<HashMap<usize, Call>>> = Mutex::new(None);
static LOG: AtomicUsize = AtomicUsize::new(0);
static GENS: Mutex<Option<HashMap<usize, u64>>> = Mutex::new(None);
static NEXT_GEN: AtomicU64 = AtomicU64::new(1);
static HELD: Mutex<Option<HashMap<usize, Vec<usize>>>> = Mutex::new(None);
static RETIRED: Mutex<Vec<(usize, Vec<usize>)>> = Mutex::new(Vec::new());

fn gens<R>(f: impl FnOnce(&mut HashMap<usize, u64>) -> R) -> R {
    f(GENS.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new))
}

fn held<R>(f: impl FnOnce(&mut HashMap<usize, Vec<usize>>) -> R) -> R {
    f(HELD.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new))
}

pub fn note_close(bundle: usize) {
    let g = NEXT_GEN.fetch_add(1, Ordering::Relaxed);
    gens(|m| m.insert(bundle, g));
}

pub fn held_count(bundle: usize) -> usize {
    let a: usize = held(|m| m.values().map(|v| v.iter().filter(|&&b| b == bundle).count()).sum());
    let r: usize = RETIRED.lock().unwrap_or_else(|e| e.into_inner()).iter().map(|(_, v)| v.iter().filter(|&&b| b == bundle).count()).sum();
    a + r
}

pub fn released_by_title(refs_left: u32, ours: usize) -> bool {
    refs_left as usize <= ours
}

pub fn title_released(obj: usize, refs_left: u32) -> bool {
    if refs_left != 0 && gens(|m| !m.contains_key(&obj)) {
        return false;
    }
    let gone = released_by_title(refs_left, held_count(obj));
    if gone {
        gens(|m| m.remove(&obj));
    }
    gone
}

unsafe fn hold(x: usize, bundles: &[usize]) {
    for &b in bundles {
        let f: unsafe extern "system" fn(*mut c_void) -> u32 = unsafe { core::mem::transmute(vslot(b as *mut c_void, 1)) };
        unsafe { f(b as *mut c_void) };
    }
    if let Some(old) = held(|m| m.insert(x, bundles.to_vec())) {
        RETIRED.lock().unwrap_or_else(|e| e.into_inner()).push((x, old));
    }
}

fn retire(x: usize) {
    if let Some(v) = held(|m| m.remove(&x)) {
        RETIRED.lock().unwrap_or_else(|e| e.into_inner()).push((x, v));
    }
}

unsafe fn sweep() {
    let done: Vec<(usize, Vec<usize>)> = {
        let mut g = RETIRED.lock().unwrap_or_else(|e| e.into_inner());
        let all: Vec<(usize, Vec<usize>)> = g.drain(..).collect();
        let (d, keep): (Vec<_>, Vec<_>) = all.into_iter().partition(|(x, _)| unsafe { crate::list_split::part_idle(*x) });
        *g = keep;
        d
    };
    for (_, v) in done {
        for b in v {
            if unsafe { release(b) } == 0 {
                crate::console_slots_cl2::forget_bundle(b);
                gens(|m| m.remove(&b));
            }
        }
    }
}

fn log(msg: &str) {
    if LOG.fetch_add(1, Ordering::Relaxed) < 24 {
        record("d3d12_xs", "indirect_bundle", msg);
    }
}

fn defer<R>(f: impl FnOnce(&mut HashMap<usize, Call>) -> R) -> R {
    f(DEFER.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new))
}

pub fn forget(list: usize) {
    defer(|m| m.remove(&list));
    retire(list);
}

pub fn deferred(list: *mut c_void) -> Option<Call> {
    defer(|m| m.get(&(list as usize)).cloned())
}

unsafe fn vslot(obj: *mut c_void, i: usize) -> usize {
    unsafe { *(*(obj as *const *const usize)).add(i) }
}

unsafe fn release(obj: usize) -> u32 {
    let f: unsafe extern "system" fn(*mut c_void) -> u32 = unsafe { core::mem::transmute(vslot(obj as *mut c_void, 2)) };
    unsafe { f(obj as *mut c_void) }
}

unsafe fn device_of(list: *mut c_void) -> *mut c_void {
    let get: unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> i32 = unsafe { core::mem::transmute(vslot(list, 7)) };
    let mut dev: *mut c_void = core::ptr::null_mut();
    if unsafe { get(list, &IID_DEVICE, &mut dev) } < 0 || dev.is_null() {
        return core::ptr::null_mut();
    }
    unsafe { release(dev as usize) };
    dev
}

unsafe fn buffer(dev: *mut c_void, size: u64) -> Option<usize> {
    let h: [u32; 5] = [HEAP_DEFAULT, 0, 0, 1, 1];
    let mut d = [0u8; 56];
    d[0..4].copy_from_slice(&1u32.to_le_bytes());
    d[0x10..0x18].copy_from_slice(&size.to_le_bytes());
    d[0x18..0x1c].copy_from_slice(&1u32.to_le_bytes());
    d[0x1c..0x1e].copy_from_slice(&1u16.to_le_bytes());
    d[0x1e..0x20].copy_from_slice(&1u16.to_le_bytes());
    d[0x24..0x28].copy_from_slice(&1u32.to_le_bytes());
    d[0x2c..0x30].copy_from_slice(&1u32.to_le_bytes());
    d[0x30..0x34].copy_from_slice(&FLAG_UAV.to_le_bytes());
    let cc: unsafe extern "system" fn(*mut c_void, *const u32, u32, *const u8, u32, *const c_void, *const Guid, *mut *mut c_void) -> i32 = unsafe { core::mem::transmute(vslot(dev, DEV_CREATE_COMMITTED)) };
    let mut r: *mut c_void = core::ptr::null_mut();
    let hr = unsafe { cc(dev, h.as_ptr(), 0, d.as_ptr(), ST_COMMON, core::ptr::null(), &IID_RESOURCE, &mut r) };
    if hr < 0 || r.is_null() {
        log(&format!("scratch buffer {size:#x} FAILED hr={hr:#010x}"));
        return None;
    }
    Some(r as usize)
}

unsafe fn gpu_va(res: usize) -> u64 {
    let f: unsafe extern "system" fn(*mut c_void) -> u64 = unsafe { core::mem::transmute(vslot(res as *mut c_void, RES_GPU_VA)) };
    unsafe { f(res as *mut c_void) }
}

unsafe fn new_scratch(list: *mut c_void, in_cap: u64, pred_cap: u64) -> Option<Scratch> {
    let dev = unsafe { device_of(list) };
    if dev.is_null() {
        return None;
    }
    let inp = unsafe { buffer(dev, in_cap)? };
    let Some(pred) = (unsafe { buffer(dev, pred_cap) }) else {
        unsafe { release(inp) };
        return None;
    };
    Some(Scratch { inp, inp_va: unsafe { gpu_va(inp) }, pred, pred_va: unsafe { gpu_va(pred) }, in_cap, pred_cap })
}

unsafe fn owned(list: *mut c_void, l: &Layout) -> Option<Where> {
    let mut g = OWNED.lock().unwrap_or_else(|e| e.into_inner());
    let m = g.get_or_insert_with(HashMap::new);
    let cur = m.get(&(list as usize)).copied();
    let s = match cur {
        Some(s) if s.in_cap >= l.in_size && s.pred_cap >= l.pred_size => s,
        _ => {
            let (ic, pc) = cur.map_or((0, 0), |s| (s.in_cap, s.pred_cap));
            if let Some(old) = cur {
                m.remove(&(list as usize));
                unsafe {
                    release(old.inp);
                    release(old.pred);
                }
            }
            let s = unsafe { new_scratch(list, grown(ic, l.in_size), grown(pc, l.pred_size))? };
            m.insert(list as usize, s);
            s
        }
    };
    Some(Where { s, in_off: 0, pred_off: 0 })
}

unsafe fn shared(list: *mut c_void, l: &Layout) -> Option<Where> {
    let mut g = SHARED.lock().unwrap_or_else(|e| e.into_inner());
    let in_need = l.in_size.div_ceil(RING_ALIGN) * RING_ALIGN;
    let pred_need = l.pred_size.div_ceil(RING_ALIGN) * RING_ALIGN;
    if !g.s.is_some_and(|s| s.in_cap >= in_need && s.pred_cap >= pred_need) {
        let (ic, pc) = g.s.map_or((0, 0), |s| (s.in_cap, s.pred_cap));
        let s = unsafe { new_scratch(list, grown(ic, in_need).max(8 << 20), grown(pc, pred_need).max(1 << 20))? };
        if let Some(old) = g.s.replace(s) {
            g.retired.push(old);
        }
        g.rings = [Ring { cap: s.in_cap, next: 0 }, Ring { cap: s.pred_cap, next: 0 }];
    }
    let s = g.s?;
    let in_off = g.rings[0].alloc(in_need)?;
    let pred_off = g.rings[1].alloc(pred_need)?;
    Some(Where { s, in_off, pred_off })
}

unsafe fn build_pipe(list: *mut c_void) -> Option<(usize, usize)> {
    let code = match tombox_xshader::dxc::compile(HLSL, "cs_6_0") {
        Ok(c) => c,
        Err(e) => {
            log(&format!("bundle predicate shader: {e}"));
            return None;
        }
    };
    let dev = unsafe { device_of(list) };
    if dev.is_null() {
        return None;
    }
    let crs: unsafe extern "system" fn(*mut c_void, u32, *const u8, usize, *const Guid, *mut *mut c_void) -> i32 = unsafe { core::mem::transmute(vslot(dev, DEV_CREATE_ROOT_SIGNATURE)) };
    let mut rs: *mut c_void = core::ptr::null_mut();
    let hr = unsafe { crs(dev, 0, code.as_ptr(), code.len(), &IID_ROOT_SIGNATURE, &mut rs) };
    if hr < 0 || rs.is_null() {
        log(&format!("predicate root signature FAILED hr={hr:#010x}"));
        return None;
    }
    let mut desc = [0u8; 56];
    desc[0..8].copy_from_slice(&(rs as u64).to_le_bytes());
    desc[8..16].copy_from_slice(&(code.as_ptr() as u64).to_le_bytes());
    desc[16..24].copy_from_slice(&(code.len() as u64).to_le_bytes());
    let cps: unsafe extern "system" fn(*mut c_void, *const u8, *const Guid, *mut *mut c_void) -> i32 = unsafe { core::mem::transmute(vslot(dev, DEV_CREATE_COMPUTE_PSO)) };
    let mut pso: *mut c_void = core::ptr::null_mut();
    let hr = unsafe { cps(dev, desc.as_ptr(), &IID_PSO, &mut pso) };
    if hr < 0 || pso.is_null() {
        log(&format!("predicate PSO FAILED hr={hr:#010x}"));
        return None;
    }
    Some((rs as usize, pso as usize))
}

unsafe fn pipe(list: *mut c_void) -> Option<(usize, usize)> {
    let mut g = PIPE.lock().unwrap_or_else(|e| e.into_inner());
    if g.is_none() {
        *g = Some(unsafe { build_pipe(list) });
    }
    (*g)?
}

unsafe fn set_predication(list: *mut c_void, res: usize, off: u64, op: u32) {
    let f: unsafe extern "system" fn(*mut c_void, *mut c_void, u64, u32) = unsafe { core::mem::transmute(vslot(list, PC_SET_PREDICATION)) };
    unsafe { f(list, res as *mut c_void, off, op) };
}

unsafe fn restore_title_predication(list: *mut c_void, title: Option<(usize, u64, u32)>) {
    match title {
        Some((res, off, op)) => unsafe { set_predication(list, res, off, op) },
        None => unsafe { set_predication(list, 0, 0, 0) },
    }
}

unsafe fn copy(list: *mut c_void, dst: usize, doff: u64, src: usize, soff: u64, n: u64) {
    let f: unsafe extern "system" fn(*mut c_void, usize, u64, usize, u64, u64) = unsafe { core::mem::transmute(vslot(list, PC_COPY_BUFFER_REGION)) };
    unsafe { f(list, dst, doff, src, soff, n) };
}

unsafe fn sources(list: *mut c_void, srcs: &[(usize, u32)], to_copy: bool) {
    for &(r, st) in srcs {
        if source_barrier(st).is_some() {
            let (a, b) = if to_copy { (st, ST_COPY_SOURCE) } else { (ST_COPY_SOURCE, st) };
            unsafe { crate::fill::barrier_pub(list, r, a, b) };
        }
    }
}

unsafe fn predicate_pass(list: *mut c_void, c: &Call, slots: u32, n: u64, l: &Layout, w: &Where, (rs, pso): (usize, usize)) {
    let s = w.s;
    let k = constants(n, w.in_off, w.pred_off, c.max_size, c.size_buf != 0, c.title.map(|t| t.2), l, c.live.len());
    let (gx, gy) = dispatch_dims(n);
    unsafe {
        crate::fill::barrier_pub(list, s.inp, ST_COMMON, ST_COPY_DEST);
        sources(list, &c.srcs, true);
        copy(list, s.inp, w.in_off + 16, c.bundle, c.offset, slots as u64 * BUNDLE_RECORD_SIZE as u64);
        if c.size_buf != 0 {
            copy(list, s.inp, w.in_off, c.size_buf, c.size_off as u64, 4);
        }
        if let Some((pres, poff, _)) = c.title {
            copy(list, s.inp, w.in_off + 8, pres, poff, 8);
        }
        sources(list, &c.srcs, false);
        let wr = candidate_writes(s.inp_va + w.in_off + l.cand, &c.live);
        let wbi: unsafe extern "system" fn(*mut c_void, u32, *const [u64; 2], *const u32) = core::mem::transmute(vslot(list, PC_WRITE_BUFFER_IMMEDIATE));
        wbi(list, wr.len() as u32, wr.as_ptr(), core::ptr::null());
        crate::fill::barrier_pub(list, s.inp, ST_COPY_DEST, ST_UAV);
        crate::fill::barrier_pub(list, s.pred, ST_COMMON, ST_UAV);
        let srs: unsafe extern "system" fn(*mut c_void, *mut c_void) = core::mem::transmute(vslot(list, PC_SET_COMPUTE_ROOT_SIGNATURE));
        srs(list, rs as *mut c_void);
        let spso: unsafe extern "system" fn(*mut c_void, *mut c_void) = core::mem::transmute(vslot(list, PC_SET_PSO));
        spso(list, pso as *mut c_void);
        let sc: unsafe extern "system" fn(*mut c_void, u32, u32, *const u32, u32) = core::mem::transmute(vslot(list, PC_SET_COMPUTE_ROOT_CONSTANTS));
        sc(list, 0, N_CONSTANTS, k.as_ptr(), 0);
        let su: unsafe extern "system" fn(*mut c_void, u32, u64) = core::mem::transmute(vslot(list, PC_SET_COMPUTE_ROOT_UAV));
        su(list, 1, s.inp_va);
        su(list, 2, s.pred_va);
        let d: unsafe extern "system" fn(*mut c_void, u32, u32, u32) = core::mem::transmute(vslot(list, PC_DISPATCH));
        d(list, gx, gy, 1);
        crate::fill::barrier_pub(list, s.pred, ST_UAV, ST_PREDICATION);
        crate::fill::barrier_pub(list, s.inp, ST_UAV, ST_COMMON);
    }
}

unsafe fn candidates(list: *mut c_void, c: &Call, n: u64, w: &Where) {
    let eb: unsafe extern "system" fn(*mut c_void, *mut c_void) = unsafe { core::mem::transmute(vslot(list, PC_EXECUTE_BUNDLE)) };
    let rebind: Vec<bool> = c.live.iter().map(|&b| crate::cl2_state::sets_inherited_state(b as *mut c_void)).collect();
    let nl = c.live.len() as u64;
    for i in 0..n {
        let p = pair_of(i, &c.live);
        unsafe {
            set_predication(list, w.s.pred, w.pred_off + i * 8, 0);
            eb(list, p.bundle as *mut c_void);
            if i + 1 < n && rebind[(i % nl) as usize] {
                crate::cl2_state::reapply_bindings(list);
            }
        }
    }
    unsafe {
        restore_title_predication(list, c.title);
        crate::fill::barrier_pub(list, w.s.pred, ST_PREDICATION, ST_COMMON);
    }
}

fn plan_of(c: &Call) -> Option<(u32, u64, Layout)> {
    let slots = slot_count(c.width, c.offset, c.max_size);
    let n = pair_count(slots, c.live.len());
    if n == 0 {
        return None;
    }
    if n > MAX_PAIRS {
        log(&format!("ExecuteIndirectBundleX: {slots} record slots x {} live bundles = {n} candidates exceed the 32-bit byte addressing of the predicate buffer; not executed", c.live.len()));
        return None;
    }
    Some((slots, n, layout(slots, c.live.len(), n)))
}

unsafe fn record_part(x: *mut c_void, c: &Call, reuse: Option<&[Option<u32>]>) -> Option<Vec<Option<u32>>> {
    let job = plan_of(c).and_then(|(slots, n, l)| {
        let w = unsafe { owned(x, &l) };
        let p = unsafe { pipe(x) };
        match (w, p) {
            (Some(w), Some(p)) => Some((slots, n, l, w, p)),
            _ => {
                log("ExecuteIndirectBundleX: predicate pipeline or scratch buffers unavailable; not executed");
                None
            }
        }
    });
    unsafe {
        if let Some((slots, n, l, w, p)) = &job {
            predicate_pass(x, c, *slots, *n, l, w, *p);
        }
        crate::cl2_state::reapply(x);
        if let Some((_, n, _, w, _)) = &job {
            candidates(x, c, *n, w);
        }
    }
    let before = crate::cl2_state::query_parts(x);
    let hr = match reuse {
        Some(s) => unsafe { crate::cl2_state::close_part_into(x, s) },
        None => unsafe { crate::cl2_state::close_part(x) },
    };
    if hr < 0 {
        log(&format!("ExecuteIndirectBundleX part {x:p}: Close FAILED hr={hr:#010x}"));
        return None;
    }
    unsafe { hold(x as usize, &c.live) };
    Some(match reuse {
        Some(s) => s.to_vec(),
        None => new_slots(&before, &crate::cl2_state::query_parts(x)),
    })
}

fn live_now() -> (Vec<usize>, Vec<u64>) {
    let live: Vec<usize> = crate::console_slots_cl2::live_bundles().into_iter().filter(|&b| !crate::cl2_state::is_open(b as *mut c_void)).collect();
    let g = gens(|m| live.iter().map(|b| m.get(b).copied().unwrap_or(0)).collect());
    (live, g)
}

unsafe fn split(list: *mut c_void, mut c: Call) -> bool {
    let Some(shim) = crate::cmdlist_x::shim_of(list) else { return false };
    let queue = crate::cl2_state::queue_of(list);
    let x = unsafe { crate::list_split::acquire_part(list) };
    if x.is_null() {
        return false;
    }
    let b = unsafe { crate::list_split::acquire_part(list) };
    if b.is_null() {
        unsafe { crate::list_split::abandon_part(x) };
        return false;
    }
    if unsafe { crate::cl2_state::close_part(list) } < 0 {
        unsafe {
            crate::list_split::abandon_part(x);
            crate::list_split::abandon_part(b);
        }
        return false;
    }
    crate::list_split::link(list, x);
    crate::list_split::link(x, b);
    crate::cl2_state::clone_state(list, x);
    c.qslots = unsafe { record_part(x, &c, None) }.unwrap_or_default();
    if queue != 0 {
        unsafe {
            crate::cl2_state::submit_part(list, queue as *mut c_void);
            crate::cl2_state::submit_part(x, queue as *mut c_void);
        }
    }
    crate::cl2_state::clone_state(x, b);
    crate::cl2_state::set_queue(b, queue);
    c.carried = unsafe { predict(&c) };
    for &k in c.carried.iter().flatten() {
        unsafe { crate::cl2_state::after_bundle(b, k as *mut c_void) };
    }
    unsafe {
        crate::cl2_state::reapply(b);
        crate::cmdlist_wrap::set_real(shim as *mut c_void, b);
    }
    crate::cmdlist_x::inherit_shim(list, b);
    log(&format!("ExecuteIndirectBundleX in list {list:p}: candidates go in part {x:p} ({}); recording continues on {b:p} with the pre-call state", if queue != 0 { "list already executing on its queue: part submitted now, re-resolved if the closed list is executed again" } else { "re-resolved against the closed bundles at each ExecuteCommandLists" }));
    defer(|m| m.insert(x as usize, c));
    true
}

pub unsafe fn resolve(lists: &mut [*mut c_void]) {
    unsafe { sweep() };
    if defer(|m| m.is_empty()) {
        return;
    }
    let mut now: Option<(Vec<usize>, Vec<u64>)> = None;
    for e in lists.iter_mut() {
        let Some(c) = deferred(*e) else { continue };
        let (live, g) = now.get_or_insert_with(live_now);
        if c.live == *live && c.gens == *g {
            continue;
        }
        let old = *e;
        let nx = unsafe { crate::list_split::acquire_part(old) };
        if nx.is_null() {
            log(&format!("ExecuteIndirectBundleX part {old:p}: no list to re-resolve the candidates against the bundles closed since; the earlier candidates execute"));
            continue;
        }
        crate::list_split::relink(old, nx);
        crate::cl2_state::clone_state(old, nx);
        let c2 = Call { live: live.clone(), gens: g.clone(), ..c.clone() };
        let now_carried = unsafe { predict(&c2) };
        if now_carried != c.carried {
            log(&format!("ExecuteIndirectBundleX part {old:p}: the bundles that run changed after the call was recorded; the state carried into the rest of the list follows the record-time set"));
        }
        if unsafe { record_part(nx, &c2, Some(&c.qslots)) }.is_none() {
            crate::list_split::relink(nx, old);
            unsafe { crate::list_split::abandon_part(nx) };
            continue;
        }
        defer(|m| m.insert(nx as usize, c2));
        *e = nx;
        unsafe { crate::list_split::retire_part(old) };
    }
}

unsafe fn inline(list: *mut c_void, c: &Call) {
    let Some((slots, n, l)) = plan_of(c) else { return };
    let (Some(w), Some(p)) = (unsafe { shared(list, &l) }, unsafe { pipe(list) }) else {
        log("ExecuteIndirectBundleX: predicate pipeline or scratch ring unavailable; not executed");
        return;
    };
    unsafe {
        set_predication(list, 0, 0, 0);
        predicate_pass(list, c, slots, n, &l, &w, p);
        crate::cl2_state::reapply(list);
        candidates(list, c, n, &w);
        crate::cl2_state::reapply(list);
    }
}

pub unsafe fn execute(list: *mut c_void, bundle: *mut c_void, width: u64, offset: u64, max_size: u32, size_buf: *mut c_void, size_off: u32) {
    let title = crate::cl2_state::predication_of(list);
    let srcs = distinct(&[bundle as usize, size_buf as usize, title.map_or(0, |t| t.0)]).into_iter().map(|r| (r, crate::gpu_va::state(r))).collect();
    let (live, g) = live_now();
    let c = Call { bundle: bundle as usize, width, offset, max_size, size_buf: size_buf as usize, size_off, title, srcs, live, gens: g, qslots: Vec::new(), carried: None };
    if unsafe { split(list, c.clone()) } {
        return;
    }
    log(&format!("ExecuteIndirectBundleX in list {list:p}: no shim or pooled list to split on; candidates recorded inline against the bundles closed now, and state a skipped bundle sets is not undone"));
    unsafe { inline(list, &c) };
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(b: usize) -> BundleRecord {
        BundleRecord { magic: BUNDLE_RECORD_MAGIC, size: BUNDLE_RECORD_SIZE, bundle: b as u64 }
    }

    #[test]
    fn slot_count_is_bounded_by_max_size_and_the_buffer() {
        assert_eq!(slot_count(0x100, 0, 0x40), 4);
        assert_eq!(slot_count(0x100, 0xf0, 0x40), 1);
        assert_eq!(slot_count(0x100, 0, 0x1f), 1);
        assert_eq!(slot_count(0x100, 0, 0xf), 0);
        assert_eq!(slot_count(0x100, 0x100, 0x40), 0);
        assert_eq!(slot_count(4 << 20, 0, (4 << 20) - 1), 262_143);
    }

    #[test]
    fn every_slot_pairs_with_every_live_bundle_without_a_budget() {
        let live = [0x1000, 0x2000, 0x3000];
        assert_eq!(pair_count(262_143, live.len()), 786_429);
        assert_eq!(pair_of(0, &live), Pair { slot: 0, bundle: 0x1000 });
        assert_eq!(pair_of(4, &live), Pair { slot: 1, bundle: 0x2000 });
        assert_eq!(pair_of(786_428, &live), Pair { slot: 262_142, bundle: 0x3000 });
        assert_eq!(pair_count(5, 0), 0);
        assert!(pair_count(262_143, 2048) <= MAX_PAIRS);
    }

    #[test]
    fn dispatch_covers_every_pair_past_the_group_limit() {
        assert_eq!(dispatch_dims(0), (0, 0));
        assert_eq!(dispatch_dims(1), (1, 1));
        assert_eq!(dispatch_dims(64), (1, 1));
        assert_eq!(dispatch_dims(65), (2, 1));
        assert_eq!(dispatch_dims(65535 * 64), (65535, 1));
        assert_eq!(dispatch_dims(65535 * 64 + 1), (65535, 2));
        let n = 786_429u64 * 64;
        let (gx, gy) = dispatch_dims(n);
        assert!(gx as u64 * gy as u64 * 64 >= n && (gx as u64 * (gy as u64 - 1)) * 64 < n);
    }

    #[test]
    fn predicate_follows_min_of_size_and_max_and_the_title_predicate() {
        let a = Pair { slot: 0, bundle: 0x1000 };
        let b = Pair { slot: 1, bundle: 0x2000 };
        assert!(runs(a, None, 32, &rec(0x1000), None));
        assert!(runs(b, None, 32, &rec(0x2000), None));
        assert!(!runs(b, None, 31, &rec(0x2000), None));
        assert!(runs(a, Some(16), 32, &rec(0x1000), None));
        assert!(!runs(b, Some(16), 32, &rec(0x2000), None));
        assert!(!runs(a, Some(0), 32, &rec(0x1000), None));
        assert!(!runs(b, Some(48), 16, &rec(0x2000), None));
        assert!(!runs(a, None, 32, &rec(0x2000), None));
        assert!(!runs(a, None, 32, &BundleRecord { magic: 0, ..rec(0x1000) }, None));
        assert!(!runs(a, None, 32, &BundleRecord { size: 8, ..rec(0x1000) }, None));
        assert!(!runs(a, None, 32, &rec(0x1000), Some((0, 0))));
        assert!(runs(a, None, 32, &rec(0x1000), Some((0, 5))));
        assert!(runs(a, None, 32, &rec(0x1000), Some((1, 0))));
        assert!(!runs(a, None, 32, &rec(0x1000), Some((1, 5))));
    }

    #[test]
    fn candidate_table_layout_and_constants_match_the_shader() {
        let w = candidate_writes(0x1000, &[0x1_2345_6789, 0x10]);
        assert_eq!(w, vec![[0x1000, 0x2345_6789], [0x1004, 1], [0x1008, 0x10], [0x100c, 0]]);
        let l = layout(2, 3, 6);
        assert_eq!(l, Layout { cand: 48, in_size: 72, pred_size: 48 });
        let k = constants(6, 0x100, 0x200, 0x40, true, None, &l, 3);
        assert_eq!(k, [6, 0x100, 0x200, 0x130, 0x40, 1, TITLE_OP_NONE, BUNDLE_RECORD_MAGIC, 3, 1]);
        assert_eq!(constants(1, 0, 0, 16, false, Some(1), &l, 1)[6], 1);
        assert_eq!(constants(1, 0, 0, 16, false, Some(0), &l, 1)[6], 0);
        assert_eq!(constants(65535 * 64 * 3, 0, 0, 16, false, None, &l, 1)[9], 65535);
    }

    #[test]
    fn scratch_grows_to_the_need_and_rings_align_and_wrap() {
        assert_eq!(grown(0, 10), MIN_SCRATCH);
        assert_eq!(grown(MIN_SCRATCH, 100), MIN_SCRATCH);
        assert_eq!(grown(MIN_SCRATCH, MIN_SCRATCH + 1), MIN_SCRATCH * 2);
        assert_eq!(grown(1 << 20, 5 << 20), 8 << 20);
        let mut r = Ring { cap: 1024, next: 0 };
        assert_eq!(r.alloc(10), Some(0));
        assert_eq!(r.alloc(300), Some(256));
        assert_eq!(r.alloc(512), Some(0));
        assert_eq!(r.next, 512);
        assert_eq!(r.alloc(2048), None);
    }

    #[test]
    fn source_states_without_copy_source_are_transitioned_once() {
        assert_eq!(source_barrier(0xac3), None);
        assert_eq!(source_barrier(ST_COPY_SOURCE), None);
        assert_eq!(source_barrier(ST_COMMON), Some(ST_COMMON));
        assert_eq!(source_barrier(ST_PREDICATION), Some(ST_PREDICATION));
        assert_eq!(source_barrier(ST_UAV), Some(ST_UAV));
        assert_eq!(distinct(&[5, 0, 5, 7]), vec![5, 7]);
    }

    #[test]
    fn a_re_resolved_part_reuses_the_query_slots_of_the_part_it_replaces() {
        let before = vec![vec![1], vec![2, 3], vec![]];
        let after = vec![vec![1, 9], vec![2, 3], vec![4]];
        assert_eq!(new_slots(&before, &after), vec![Some(9), None, Some(4)]);
        assert_eq!(new_slots(&[], &[]), vec![]);
    }

    #[test]
    fn carried_state_follows_the_valid_records_in_slot_order_when_the_cpu_can_see_them() {
        let live = [0x1000, 0x2000];
        let recs = [rec(0x2000), BundleRecord { magic: 0, ..rec(0x1000) }, rec(0x3000), rec(0x1000), rec(0x2000)];
        assert_eq!(executed_in_order(&recs, &live), vec![0x2000, 0x1000, 0x2000]);
        assert!(cpu_determined(0, None, false));
        assert!(!cpu_determined(0x10, None, false));
        assert!(!cpu_determined(0, Some((1, 0, 0)), false));
        assert!(!cpu_determined(0, None, true));
    }

    #[test]
    fn a_bundle_is_gone_for_the_title_when_only_our_references_remain() {
        assert!(released_by_title(0, 0));
        assert!(released_by_title(2, 2));
        assert!(!released_by_title(3, 2));
        let b = 0x6_1000;
        assert!(!title_released(b, 1));
        note_close(b);
        let g1 = gens(|m| m[&b]);
        note_close(b);
        assert!(gens(|m| m[&b]) > g1);
        assert!(title_released(b, 0));
        assert!(gens(|m| !m.contains_key(&b)));
        note_close(b);
        held(|m| m.insert(0x6_2000, vec![b]));
        assert_eq!(held_count(b), 1);
        assert!(!title_released(b, 2));
        assert!(title_released(b, 1));
        held(|m| m.remove(&0x6_2000));
    }

    #[test]
    fn deferred_parts_are_forgotten_when_their_list_is_given_back() {
        let c = Call { bundle: 1, width: 16, offset: 0, max_size: 16, size_buf: 0, size_off: 0, title: None, srcs: vec![], live: vec![7], gens: vec![1], qslots: vec![], carried: None };
        defer(|m| m.insert(0x5_1000, c.clone()));
        assert_eq!(deferred(0x5_1000 as *mut c_void), Some(c));
        forget(0x5_1000);
        assert_eq!(deferred(0x5_1000 as *mut c_void), None);
    }

    #[test]
    fn predicate_shader_compiles_when_dxc_is_present() {
        if !tombox_xshader::dxc::available() {
            return;
        }
        let code = tombox_xshader::dxc::compile(HLSL, "cs_6_0").unwrap();
        assert_eq!(&code[0..4], b"DXBC");
        assert!(code.windows(4).any(|w| w == b"RTS0"));
    }
}
