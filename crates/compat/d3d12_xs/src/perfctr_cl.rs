use core::ffi::c_void;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use compat_common::record;

use crate::console_slots_dev2::{family, Family};
use crate::device_wrap::Guid;
use crate::perfctr::{CounterSet, PcSource};

pub const CL_BEGIN_COUNTERS_X: usize = 100;
pub const CL_SAMPLE_COUNTERS_X: usize = 101;
pub const CL_END_COUNTERS_X: usize = 102;

pub const ERR_CLOSED: &str = "Cannot record commands into a command list that has already been closed.";
pub const ERR_ALIGN: &str = "AlignedDestinationBufferOffset must be a multiple of 8 bytes.";
pub const ERR_NOT_BUFFER: &str = "pDestinationBuffer is not a buffer resource.";
pub const ERR_BUNDLE: &str = "This API cannot be called on bundle command lists.";

pub const HLSL: &str = r#"#define RS "RootConstants(num32BitConstants=8, b0), UAV(u0)"
cbuffer C : register(b0) { uint4 c0; uint4 c1; };
RWByteAddressBuffer s : register(u0);
uint64_t ld(uint i) { uint2 r = s.Load2(i * 8); return ((uint64_t)r.y << 32) | (uint64_t)r.x; }
[RootSignature(RS)]
[numthreads(1, 1, 1)]
void main() {
    uint64_t d = ld(c0.y) - ld(c0.x);
    uint64_t hz = ((uint64_t)c1.y << 32) | (uint64_t)c1.x;
    uint64_t f = ((uint64_t)c1.w << 32) | (uint64_t)c1.z;
    uint64_t v = (d / f) * hz + ((d % f) * hz) / f;
    s.Store2(c0.z * 8, uint2((uint)v, (uint)(v >> 32)));
    s.Store2(c0.z * 8 + 8, uint2(c0.w, 0));
}
"#;

pub const BEGIN_SLOTS: u32 = 256;
pub const SCRATCH_SLOTS: u32 = 4096;
pub const MIRROR_SLOTS: u32 = 4096;

const STATE_COMMON: u32 = 0;
const STATE_UAV: u32 = 0x8;
const STATE_COPY_DEST: u32 = 0x400;
const STATE_COPY_SOURCE: u32 = 0x800;
const STATE_GENERIC_READ: u32 = 0xac3;
const HEAP_DEFAULT: u32 = 1;
const HEAP_READBACK: u32 = 3;
const FLAG_UAV: u32 = 4;
const DIM_BUFFER: u32 = 1;
const LIST_TYPE_BUNDLE: u32 = 1;

const PC_GET_DEVICE: usize = 7;
const PC_DISPATCH: usize = 14;
const PC_COPY_BUFFER_REGION: usize = 15;
const PC_SET_PSO: usize = 25;
const PC_SET_COMPUTE_ROOT_SIGNATURE: usize = 29;
const PC_SET_COMPUTE_ROOT_CONSTANTS: usize = 35;
const PC_SET_COMPUTE_ROOT_UAV: usize = 41;
const PC_SET_PREDICATION: usize = 55;
const RES_MAP: usize = 8;
const RES_GET_DESC: usize = 10;
const RES_GET_VA: usize = 11;
const DEV_CREATE_QUEUE: usize = 8;
const DEV_CREATE_COMPUTE_PSO: usize = 11;
const DEV_CREATE_ROOT_SIGNATURE: usize = 16;
const DEV_CREATE_COMMITTED: usize = 27;
const QUEUE_GET_TIMESTAMP_FREQUENCY: usize = 16;

const IID_DEVICE: Guid = Guid { d1: 0x1898_19f1, d2: 0x1db6, d3: 0x4b57, d4: [0xbe, 0x54, 0x18, 0x21, 0x33, 0x9b, 0x85, 0xf7] };
const IID_QUEUE: Guid = Guid { d1: 0x0ec8_70a6, d2: 0x5d7e, d3: 0x4c22, d4: [0x8c, 0xfc, 0x5b, 0xaa, 0xe0, 0x76, 0x16, 0xed] };
const IID_ROOT_SIGNATURE: Guid = Guid { d1: 0xc54a_6b66, d2: 0x72df, d3: 0x4ee8, d4: [0x8b, 0xe5, 0xa9, 0x46, 0xa1, 0x42, 0x92, 0x14] };
const IID_PSO: Guid = Guid { d1: 0x765a_30f3, d2: 0xf624, d3: 0x4c6f, d4: [0xa8, 0x28, 0xac, 0xe9, 0x48, 0x62, 0x24, 0x45] };
const IID_RESOURCE: Guid = Guid { d1: 0x6964_42be, d2: 0xa72e, d3: 0x4059, d4: [0xbc, 0x79, 0x5b, 0x5c, 0x98, 0x04, 0x0f, 0xad] };

#[derive(Clone, Copy)]
struct Pipeline {
    rs: usize,
    pso: usize,
    scratch: usize,
    scratch_va: u64,
    mirror: usize,
    mirror_cpu: usize,
}

struct Active {
    set_ptr: usize,
    xs: bool,
    cells: Arc<Vec<Cell>>,
    begin_slot: Option<u32>,
}

#[derive(Default)]
struct Mirror {
    by_key: HashMap<(usize, u64), Vec<(u32, u32)>>,
}

pub const MIRROR_CANDIDATES: usize = 8;

static PIPE: Mutex<Option<Option<Pipeline>>> = Mutex::new(None);
static ACTIVE: Mutex<Option<HashMap<usize, Active>>> = Mutex::new(None);
static MIRROR: Mutex<Option<Mirror>> = Mutex::new(None);
static NEXT_BEGIN: AtomicU32 = AtomicU32::new(0);
static NEXT_SCRATCH: AtomicU32 = AtomicU32::new(0);
static NEXT_MIRROR: AtomicU32 = AtomicU32::new(0);
static NEXT_SEQ: AtomicU32 = AtomicU32::new(0);
static HOST_FREQ: AtomicU64 = AtomicU64::new(0);
static SCARLETT: AtomicBool = AtomicBool::new(false);
static LOG: AtomicUsize = AtomicUsize::new(0);

fn once(n: usize) -> bool {
    LOG.fetch_add(1, Ordering::Relaxed) < n
}

unsafe fn vslot(obj: *mut c_void, i: usize) -> usize {
    unsafe { *(*(obj as *const *const usize)).add(i) }
}

unsafe fn release(obj: *mut c_void) {
    if !obj.is_null() {
        let f: unsafe extern "system" fn(*mut c_void) -> u32 = unsafe { core::mem::transmute(vslot(obj, 2)) };
        unsafe { f(obj) };
    }
}

pub fn sample_errors(fam: Family, closed: bool, bundle: bool, offset: u64, is_buffer: bool, entries: u32, width: u64) -> Vec<String> {
    if closed {
        return vec![ERR_CLOSED.to_string()];
    }
    let mut e = Vec::new();
    if fam == Family::Scarlett && bundle {
        e.push(ERR_BUNDLE.to_string());
    }
    if offset & 7 != 0 {
        e.push(ERR_ALIGN.to_string());
    }
    if !is_buffer {
        e.push(ERR_NOT_BUFFER.to_string());
    }
    let need = (offset as u32).wrapping_add(entries.wrapping_mul(8));
    let size = width as u32;
    if need > size {
        e.push(format!("Resolve destination extent of {} exceeds pDestinationBuffer size of {} bytes.", need as i32, size as i32));
    }
    e
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Write {
    pub dst_off: u64,
    pub hz: u64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cell {
    pub block: u8,
    pub select: u32,
    pub sample_offset: u32,
    pub values: u32,
    pub source: PcSource,
}

pub fn cells_of(set: &CounterSet) -> Vec<Cell> {
    set.entries.iter().map(|e| Cell { block: e.block, select: e.select, sample_offset: e.sample_offset, values: e.values(), source: e.source }).collect()
}

pub fn cells_of_xs(set: &crate::perfctr_xs::XsCounterSet) -> Vec<Cell> {
    set.entries.iter().map(|e| Cell { block: e.block, select: e.select, sample_offset: e.sample_offset, values: e.values(), source: e.source }).collect()
}

pub fn plan_cells(cells: &[Cell], offset: u64) -> (Vec<Write>, Vec<usize>) {
    let mut w = Vec::new();
    let mut missing = Vec::new();
    for (n, e) in cells.iter().enumerate() {
        match e.source {
            PcSource::ConsoleClock { hz } => {
                for i in 0..e.values as u64 {
                    w.push(Write { dst_off: offset + e.sample_offset as u64 + 8 * i, hz });
                }
            }
            PcSource::None => missing.push(n),
        }
    }
    (w, missing)
}

pub fn plan_sample(set: &CounterSet, offset: u64) -> (Vec<Write>, Vec<usize>) {
    plan_cells(&cells_of(set), offset)
}

fn set_live(ptr: usize, xs: bool) -> bool {
    if xs { crate::perfctr_xs::xs_counter_set_for(ptr).is_some() } else { crate::perfctr::is_live(ptr) }
}

fn cells_for(ptr: usize) -> Option<(bool, Vec<Cell>)> {
    if let Some(cs) = crate::perfctr::counter_set_for(ptr) {
        return Some((false, cells_of(&cs)));
    }
    crate::perfctr_xs::xs_counter_set_for(ptr).map(|x| (true, cells_of_xs(&x)))
}

pub fn clock_value(begin: u64, now: u64, hz: u64, freq: u64) -> u64 {
    let d = now.wrapping_sub(begin);
    let f = freq.max(1);
    (d / f).wrapping_mul(hz).wrapping_add(((d % f).wrapping_mul(hz)) / f)
}

pub fn constants(begin_slot: u32, now_slot: u32, out_slot: u32, seq: u32, hz: u64, freq: u64) -> [u32; 8] {
    [begin_slot, now_slot, out_slot, seq, hz as u32, (hz >> 32) as u32, freq.max(1) as u32, (freq.max(1) >> 32) as u32]
}

pub fn begin_slot_index(n: u32) -> u32 {
    n % BEGIN_SLOTS
}

pub fn scratch_slot_index(n: u32) -> u32 {
    BEGIN_SLOTS + 2 * (n % ((SCRATCH_SLOTS - BEGIN_SLOTS) / 2))
}

pub fn next_seq() -> u32 {
    loop {
        let s = NEXT_SEQ.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
        if s != 0 {
            return s;
        }
    }
}

pub fn distinct_hz(w: &[Write]) -> Vec<u64> {
    let mut v: Vec<u64> = Vec::new();
    for x in w {
        if !v.contains(&x.hz) {
            v.push(x.hz);
        }
    }
    v
}

unsafe fn device_of(list: *mut c_void) -> *mut c_void {
    let get: unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> i32 = unsafe { core::mem::transmute(crate::cmdlist_wrap::pc_method(list, PC_GET_DEVICE)) };
    let mut dev: *mut c_void = core::ptr::null_mut();
    if unsafe { get(list, &IID_DEVICE, &mut dev) } < 0 {
        return core::ptr::null_mut();
    }
    dev
}

unsafe fn device_key(list: *mut c_void) -> usize {
    let dev = unsafe { device_of(list) };
    let k = dev as usize;
    unsafe { release(dev) };
    k
}

unsafe fn host_freq(dev: *mut c_void) -> u64 {
    let f = crate::timestamp::freq();
    if f != 0 {
        return f;
    }
    let c = HOST_FREQ.load(Ordering::Relaxed);
    if c != 0 {
        return c;
    }
    let desc = [0u32; 4];
    let cq: unsafe extern "system" fn(*mut c_void, *const u32, *const Guid, *mut *mut c_void) -> i32 = unsafe { core::mem::transmute(vslot(dev, DEV_CREATE_QUEUE)) };
    let mut q: *mut c_void = core::ptr::null_mut();
    if unsafe { cq(dev, desc.as_ptr(), &IID_QUEUE, &mut q) } < 0 || q.is_null() {
        return 0;
    }
    let gf: unsafe extern "system" fn(*mut c_void, *mut u64) -> i32 = unsafe { core::mem::transmute(vslot(q, QUEUE_GET_TIMESTAMP_FREQUENCY)) };
    let mut v = 0u64;
    let hr = unsafe { gf(q, &mut v) };
    unsafe { release(q) };
    if hr < 0 {
        return 0;
    }
    HOST_FREQ.store(v, Ordering::Relaxed);
    v
}

unsafe fn committed_buffer(dev: *mut c_void, heap: u32, size: u64, flags: u32, state: u32) -> *mut c_void {
    let h: [u32; 5] = [heap, 0, 0, 1, 1];
    let mut rd = [0u8; 56];
    rd[0..4].copy_from_slice(&DIM_BUFFER.to_le_bytes());
    rd[0x10..0x18].copy_from_slice(&size.to_le_bytes());
    rd[0x18..0x1c].copy_from_slice(&1u32.to_le_bytes());
    rd[0x1c..0x1e].copy_from_slice(&1u16.to_le_bytes());
    rd[0x1e..0x20].copy_from_slice(&1u16.to_le_bytes());
    rd[0x24..0x28].copy_from_slice(&1u32.to_le_bytes());
    rd[0x2c..0x30].copy_from_slice(&1u32.to_le_bytes());
    rd[0x30..0x34].copy_from_slice(&flags.to_le_bytes());
    let cc: unsafe extern "system" fn(*mut c_void, *const u32, u32, *const u8, u32, *const c_void, *const Guid, *mut *mut c_void) -> i32 = unsafe { core::mem::transmute(vslot(dev, DEV_CREATE_COMMITTED)) };
    let mut buf: *mut c_void = core::ptr::null_mut();
    let hr = unsafe { cc(dev, h.as_ptr(), 0, rd.as_ptr(), state, core::ptr::null(), &IID_RESOURCE, &mut buf) };
    if hr < 0 {
        record("d3d12_xs", "perfctr_cl", &format!("buffer heap={heap} size={size:#x} FAILED hr={hr:#010x}"));
        return core::ptr::null_mut();
    }
    buf
}

unsafe fn build_on(dev: *mut c_void, code: &[u8]) -> Option<Pipeline> {
    let crs: unsafe extern "system" fn(*mut c_void, u32, *const u8, usize, *const Guid, *mut *mut c_void) -> i32 = unsafe { core::mem::transmute(vslot(dev, DEV_CREATE_ROOT_SIGNATURE)) };
    let mut rs: *mut c_void = core::ptr::null_mut();
    let hr = unsafe { crs(dev, 0, code.as_ptr(), code.len(), &IID_ROOT_SIGNATURE, &mut rs) };
    if hr < 0 || rs.is_null() {
        record("d3d12_xs", "perfctr_cl", &format!("counter conversion root signature FAILED hr={hr:#010x}"));
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
        record("d3d12_xs", "perfctr_cl", &format!("counter conversion PSO FAILED hr={hr:#010x}"));
        return None;
    }
    let scratch = unsafe { committed_buffer(dev, HEAP_DEFAULT, SCRATCH_SLOTS as u64 * 8, FLAG_UAV, STATE_COMMON) };
    let mirror = unsafe { committed_buffer(dev, HEAP_READBACK, MIRROR_SLOTS as u64 * 16, 0, STATE_COPY_DEST) };
    if scratch.is_null() || mirror.is_null() {
        return None;
    }
    let va: unsafe extern "system" fn(*mut c_void) -> u64 = unsafe { core::mem::transmute(vslot(scratch, RES_GET_VA)) };
    let map: unsafe extern "system" fn(*mut c_void, u32, *const c_void, *mut *mut u8) -> i32 = unsafe { core::mem::transmute(vslot(mirror, RES_MAP)) };
    let mut cpu: *mut u8 = core::ptr::null_mut();
    if unsafe { map(mirror, 0, core::ptr::null(), &mut cpu) } < 0 || cpu.is_null() {
        record("d3d12_xs", "perfctr_cl", "counter readback mirror Map FAILED");
        return None;
    }
    Some(Pipeline { rs: rs as usize, pso: pso as usize, scratch: scratch as usize, scratch_va: unsafe { va(scratch) }, mirror: mirror as usize, mirror_cpu: cpu as usize })
}

unsafe fn pipeline(list: *mut c_void) -> Option<Pipeline> {
    let mut g = PIPE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(p) = *g {
        return p;
    }
    let p = match tombox_xshader::dxc::compile(HLSL, "cs_6_0") {
        Ok(code) => {
            let dev = unsafe { device_of(list) };
            if dev.is_null() {
                None
            } else {
                let p = unsafe { build_on(dev, &code) };
                unsafe { release(dev) };
                p
            }
        }
        Err(e) => {
            record("d3d12_xs", "perfctr_cl", &format!("counter conversion shader: {e}"));
            None
        }
    };
    *g = Some(p);
    p
}

fn note_mirror(res: usize, off: u64, slot: u32, seq: u32) {
    let mut g = MIRROR.lock().unwrap_or_else(|e| e.into_inner());
    let m = g.get_or_insert_with(Mirror::default);
    let c = m.by_key.entry((res, off)).or_default();
    c.push((slot, seq));
    if c.len() > MIRROR_CANDIDATES {
        c.remove(0);
    }
}

pub fn executed_value(cell: &[u64; 2], seq: u32) -> Option<u64> {
    if cell[1] == seq as u64 { Some(cell[0]) } else { None }
}

pub fn read_value(resource: usize, offset: u64) -> Option<u64> {
    let cands = {
        let g = MIRROR.lock().unwrap_or_else(|e| e.into_inner());
        g.as_ref()?.by_key.get(&(resource, offset))?.clone()
    };
    let p = (*PIPE.lock().unwrap_or_else(|e| e.into_inner()))??;
    for &(slot, seq) in cands.iter().rev() {
        let cell = unsafe { (p.mirror_cpu as *const u64).add(slot as usize * 2) };
        let s1 = unsafe { core::ptr::read_volatile(cell.add(1)) };
        let v = unsafe { core::ptr::read_volatile(cell) };
        let s2 = unsafe { core::ptr::read_volatile(cell.add(1)) };
        if s1 == s2 {
            if let Some(v) = executed_value(&[v, s1], seq) {
                return Some(v);
            }
        }
    }
    None
}

pub fn read_sample(resource: usize, offset: u64, len: usize) -> Option<Vec<u8>> {
    if offset % 8 != 0 || len % 8 != 0 {
        return None;
    }
    let mut out = Vec::with_capacity(len);
    for i in (0..len as u64).step_by(8) {
        out.extend_from_slice(&read_value(resource, offset + i)?.to_le_bytes());
    }
    Some(out)
}

pub unsafe fn record_begin(list: *mut c_void) -> Option<u32> {
    let _ij = unsafe { crate::inject::scope(list, crate::inject::QUERY) };
    let p = unsafe { pipeline(list) }?;
    let slot = begin_slot_index(NEXT_BEGIN.fetch_add(1, Ordering::Relaxed));
    unsafe {
        crate::fill::barrier_pub(list, p.scratch, STATE_COMMON, STATE_COPY_DEST);
        let ok = crate::timestamp::resolve_into(list, p.scratch, slot as u64 * 8);
        crate::fill::barrier_pub(list, p.scratch, STATE_COPY_DEST, STATE_COMMON);
        if ok { Some(slot) } else { None }
    }
}

pub unsafe fn record_sample(list: *mut c_void, res: *mut c_void, offset: u64, begin_slot: u32, writes: &[Write]) -> usize {
    let _ij = unsafe { crate::inject::scope(list, crate::inject::QUERY) };
    if writes.is_empty() {
        return 0;
    }
    let Some(p) = (unsafe { pipeline(list) }) else { return 0 };
    let dev = unsafe { device_of(list) };
    if dev.is_null() {
        return 0;
    }
    let freq = unsafe { host_freq(dev) };
    unsafe { release(dev) };
    if freq == 0 {
        if once(16) {
            record("d3d12_xs", "perfctr_cl", "SampleCountersX: host GPU timestamp frequency unavailable; counters not written");
        }
        return 0;
    }
    let va: unsafe extern "system" fn(*mut c_void) -> u64 = unsafe { core::mem::transmute(vslot(res, RES_GET_VA)) };
    let base = unsafe { va(res) };
    let Some((e, base_off)) = crate::gpu_va::lookup(base) else {
        if once(16) {
            record("d3d12_xs", "perfctr_cl", &format!("SampleCountersX: destination {res:p} (va {base:#x}) is not a tracked buffer; counters not written"));
        }
        return 0;
    };
    let st = crate::gpu_va::state(e.res);
    if !e.buffer || st == STATE_GENERIC_READ {
        if once(16) {
            record("d3d12_xs", "perfctr_cl", &format!("SampleCountersX: destination {res:p} is not GPU-writable (state {st:#x}); counters not written"));
        }
        return 0;
    }
    let fits: Vec<Write> = writes.iter().copied().filter(|w| base_off + w.dst_off + 8 <= e.len).collect();
    if fits.len() != writes.len() && once(16) {
        record("d3d12_xs", "perfctr_cl", &format!("SampleCountersX: {} counter values fall past the end of {res:p} (offset {offset:#x}); those are not written", writes.len() - fits.len()));
    }
    if fits.is_empty() {
        return 0;
    }
    let now = scratch_slot_index(NEXT_SCRATCH.fetch_add(1, Ordering::Relaxed));
    let hzs = distinct_hz(&fits);
    let seq = next_seq();
    let outs: Vec<(u64, u32)> = hzs.iter().map(|&h| (h, scratch_slot_index(NEXT_SCRATCH.fetch_add(1, Ordering::Relaxed)))).collect();
    unsafe {
        crate::fill::barrier_pub(list, p.scratch, STATE_COMMON, STATE_COPY_DEST);
        if !crate::timestamp::resolve_into(list, p.scratch, now as u64 * 8) {
            crate::fill::barrier_pub(list, p.scratch, STATE_COPY_DEST, STATE_COMMON);
            return 0;
        }
        crate::fill::barrier_pub(list, p.scratch, STATE_COPY_DEST, STATE_UAV);
        let pred: unsafe extern "system" fn(*mut c_void, *mut c_void, u64, u32) = core::mem::transmute(crate::cmdlist_wrap::pc_method(list, PC_SET_PREDICATION));
        pred(list, core::ptr::null_mut(), 0, 0);
        let srs: unsafe extern "system" fn(*mut c_void, *mut c_void) = core::mem::transmute(crate::cmdlist_wrap::pc_method(list, PC_SET_COMPUTE_ROOT_SIGNATURE));
        srs(list, p.rs as *mut c_void);
        let spso: unsafe extern "system" fn(*mut c_void, *mut c_void) = core::mem::transmute(crate::cmdlist_wrap::pc_method(list, PC_SET_PSO));
        spso(list, p.pso as *mut c_void);
        let su: unsafe extern "system" fn(*mut c_void, u32, u64) = core::mem::transmute(crate::cmdlist_wrap::pc_method(list, PC_SET_COMPUTE_ROOT_UAV));
        su(list, 1, p.scratch_va);
        let sc: unsafe extern "system" fn(*mut c_void, u32, u32, *const u32, u32) = core::mem::transmute(crate::cmdlist_wrap::pc_method(list, PC_SET_COMPUTE_ROOT_CONSTANTS));
        let d: unsafe extern "system" fn(*mut c_void, u32, u32, u32) = core::mem::transmute(crate::cmdlist_wrap::pc_method(list, PC_DISPATCH));
        for &(hz, out) in &outs {
            let k = constants(begin_slot, now, out, seq, hz, freq);
            sc(list, 0, 8, k.as_ptr(), 0);
            d(list, 1, 1, 1);
        }
        crate::fill::barrier_pub(list, p.scratch, STATE_UAV, STATE_COPY_SOURCE);
        if st != STATE_COPY_DEST {
            crate::fill::barrier_pub(list, e.res, st, STATE_COPY_DEST);
        }
        let cp: unsafe extern "system" fn(*mut c_void, usize, u64, usize, u64, u64) = core::mem::transmute(crate::cmdlist_wrap::pc_method(list, PC_COPY_BUFFER_REGION));
        for w in &fits {
            let out = outs.iter().find(|o| o.0 == w.hz).map(|o| o.1).unwrap_or(now);
            cp(list, e.res, base_off + w.dst_off, p.scratch, out as u64 * 8, 8);
            let m = NEXT_MIRROR.fetch_add(1, Ordering::Relaxed) % MIRROR_SLOTS;
            cp(list, p.mirror, m as u64 * 16, p.scratch, out as u64 * 8, 16);
            note_mirror(res as usize, w.dst_off, m, seq);
        }
        if st != STATE_COPY_DEST {
            crate::fill::barrier_pub(list, e.res, STATE_COPY_DEST, st);
        }
        crate::fill::barrier_pub(list, p.scratch, STATE_COPY_SOURCE, STATE_COMMON);
        crate::cl2_state::reapply(list);
    }
    crate::placed::note_gpu_write(e.res as *mut c_void);
    fits.len()
}

unsafe fn res_desc(res: *mut c_void) -> [u8; 56] {
    let get: unsafe extern "system" fn(*mut c_void, *mut u8) -> *mut u8 = unsafe { core::mem::transmute(vslot(res, RES_GET_DESC)) };
    let mut d = [0u8; 56];
    unsafe { get(res, d.as_mut_ptr()) };
    d
}

fn gpu_recordable(list_type: u32) -> bool {
    list_type == crate::fill::LIST_TYPE_DIRECT || list_type == crate::fill::LIST_TYPE_COMPUTE
}

pub unsafe extern "system" fn begin_counters_x(this: *mut c_void, set: *mut c_void) {
    if set.is_null() {
        if once(16) {
            record("d3d12_xs", "perfctr_cl", "BeginCountersX(NULL): the console UMD dereferences the counter set unconditionally; nothing recorded");
        }
        return;
    }
    let Some((xs, cells)) = cells_for(set as usize) else {
        if once(16) {
            record("d3d12_xs", "perfctr_cl", &format!("BeginCountersX({set:p}): not a live counter set created by CreateCounterSetX; nothing recorded"));
        }
        return;
    };
    if cells.is_empty() {
        return;
    }
    let key = unsafe { device_key(this) };
    let needs_clock = cells.iter().any(|e| matches!(e.source, PcSource::ConsoleClock { .. }));
    let ty = unsafe { crate::fill::list_type(this) };
    let closed = crate::cl2_state::title_closed(this);
    let begin_slot = if needs_clock && gpu_recordable(ty) && !closed { unsafe { record_begin(this) } } else { None };
    static FIRST: AtomicBool = AtomicBool::new(false);
    if !FIRST.swap(true, Ordering::Relaxed) {
        record("d3d12_xs", "perfctr_cl", &format!("first call BeginCountersX({set:p}) list type {ty}: {} counters, clock begin {}", cells.len(), if begin_slot.is_some() { "recorded" } else { "not needed or not recordable" }));
    }
    if needs_clock && begin_slot.is_none() && once(16) {
        record("d3d12_xs", "perfctr_cl", &format!("BeginCountersX({set:p}): no begin timestamp recorded (list type {ty}, closed {closed}); clock counters of this set will not be written"));
    }
    let mut g = ACTIVE.lock().unwrap_or_else(|e| e.into_inner());
    g.get_or_insert_with(HashMap::new).insert(key, Active { set_ptr: set as usize, xs, cells: Arc::new(cells), begin_slot });
}

pub unsafe extern "system" fn sample_counters_x(this: *mut c_void, dst: *mut c_void, offset: u64) {
    let key = unsafe { device_key(this) };
    let active = {
        let g = ACTIVE.lock().unwrap_or_else(|e| e.into_inner());
        g.as_ref().and_then(|m| m.get(&key)).map(|a| (a.set_ptr, a.xs, a.cells.clone(), a.begin_slot))
    };
    let closed = crate::cl2_state::title_closed(this);
    let (is_buffer, width) = if dst.is_null() {
        (false, 0)
    } else {
        let d = unsafe { res_desc(dst) };
        (u32::from_le_bytes(d[0..4].try_into().unwrap()) == DIM_BUFFER, u64::from_le_bytes(d[16..24].try_into().unwrap()))
    };
    let Some((set_ptr, _, cells, begin_slot)) = active.filter(|a| set_live(a.0, a.1)) else {
        if once(16) {
            record("d3d12_xs", "perfctr_cl", &format!("SampleCountersX({dst:p}, {offset:#x}) with no active counter set: the console UMD dereferences the null set; nothing recorded"));
        }
        return;
    };
    let ty = unsafe { crate::fill::list_type(this) };
    let fam = if SCARLETT.load(Ordering::Relaxed) { Family::Scarlett } else { Family::XboxOne };
    let errs = sample_errors(fam, closed, ty == LIST_TYPE_BUNDLE, offset, is_buffer, cells.len() as u32, width);
    if !errs.is_empty() || dst.is_null() {
        if once(16) {
            record("d3d12_xs", "perfctr_cl", &format!("SampleCountersX({dst:p}, {offset:#x}): {} (console debug validation); not recorded", errs.join(" ")));
        }
        return;
    }
    if !gpu_recordable(ty) {
        if once(16) {
            record("d3d12_xs", "perfctr_cl", &format!("SampleCountersX on a type-{ty} list: PC {} cannot hold timestamp queries and dispatches; not recorded", if ty == LIST_TYPE_BUNDLE { "bundles" } else { "copy lists" }));
        }
        return;
    }
    let (writes, missing) = plan_cells(&cells, offset);
    if !missing.is_empty() && once(32) {
        let names: Vec<String> = missing.iter().map(|&n| {
            let e = &cells[n];
            format!("block {:#x} select {:#x}", e.block, e.select)
        }).collect();
        record("d3d12_xs", "perfctr_cl", &format!("SampleCountersX(set {set_ptr:#x}): no PC source for {}; those sample bytes are not written", names.join(", ")));
    }
    if writes.is_empty() {
        return;
    }
    let Some(b) = begin_slot else {
        if once(16) {
            record("d3d12_xs", "perfctr_cl", "SampleCountersX: the active set has no recorded begin timestamp; clock counters not written");
        }
        return;
    };
    static FIRST: AtomicBool = AtomicBool::new(false);
    if !FIRST.swap(true, Ordering::Relaxed) {
        record("d3d12_xs", "perfctr_cl", &format!("first call SampleCountersX({dst:p}, {offset:#x}) list type {ty}: {} counter writes recorded, {} without a PC source", writes.len(), missing.len()));
    }
    unsafe { record_sample(this, dst, offset, b, &writes) };
}

pub unsafe extern "system" fn end_counters_x(this: *mut c_void) {
    let key = unsafe { device_key(this) };
    let mut g = ACTIVE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(m) = g.as_mut() {
        m.remove(&key);
    }
}

pub fn active_set(device: usize) -> Option<usize> {
    let g = ACTIVE.lock().unwrap_or_else(|e| e.into_inner());
    g.as_ref().and_then(|m| m.get(&device)).map(|a| a.set_ptr)
}

pub fn plan() -> Vec<(usize, usize)> {
    vec![
        (CL_BEGIN_COUNTERS_X, begin_counters_x as *const () as usize),
        (CL_SAMPLE_COUNTERS_X, sample_counters_x as *const () as usize),
        (CL_END_COUNTERS_X, end_counters_x as *const () as usize),
    ]
}

pub fn install_for(vt: &[AtomicUsize], fam: Family) -> usize {
    let mut n = 0;
    SCARLETT.store(fam == Family::Scarlett, Ordering::Relaxed);
    for (s, f) in plan() {
        if s < vt.len() {
            vt[s].store(f, Ordering::Relaxed);
            n += 1;
        }
    }
    n
}

pub fn install(vt: &[AtomicUsize]) -> usize {
    let fam = family(compat_common::gdk::identity::device_type_value());
    let n = install_for(vt, fam);
    crate::perfctr::set_value_reader(read_value);
    record("d3d12_xs", "perfctr_cl", &format!("{n} perf-counter command-list slots installed for the {fam:?} driver layout"));
    n
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::perfctr::CounterEntry;

    fn entry(block: u8, select: u32, instances: u16, engines: u16, wide: bool, sample_offset: u32, source: PcSource) -> CounterEntry {
        CounterEntry { block, slot: 0, select, instances, engines, wide, sample_offset, result_offset: 0, source }
    }

    fn set(entries: Vec<CounterEntry>) -> CounterSet {
        let sample_bytes = entries.iter().map(|e| 8 * e.values()).sum();
        CounterSet { scorpio: false, sq_ctrl: 0xffff, se_mask: 0xf, entries, sample_bytes }
    }

    #[test]
    fn debug_validation_matches_xgs12_d() {
        assert!(sample_errors(Family::XboxOne, false, false, 0, true, 2, 16).is_empty());
        assert_eq!(sample_errors(Family::Scarlett, true, true, 3, false, 99, 0), vec![ERR_CLOSED.to_string()]);
        assert_eq!(sample_errors(Family::XboxOne, false, false, 4, true, 1, 64), vec![ERR_ALIGN.to_string()]);
        assert_eq!(sample_errors(Family::XboxOne, false, false, 0, false, 1, 64), vec![ERR_NOT_BUFFER.to_string()]);
        assert_eq!(sample_errors(Family::XboxOne, false, false, 8, true, 2, 16), vec!["Resolve destination extent of 24 exceeds pDestinationBuffer size of 16 bytes.".to_string()]);
        assert_eq!(sample_errors(Family::XboxOne, false, false, 12, true, 3, 16).len(), 2);
        assert!(sample_errors(Family::XboxOne, false, true, 0, true, 1, 8).is_empty());
        assert_eq!(sample_errors(Family::Scarlett, false, true, 0, true, 1, 8), vec![ERR_BUNDLE.to_string()]);
        assert_eq!(sample_errors(Family::Scarlett, false, true, 4, true, 1, 16), vec![ERR_BUNDLE.to_string(), ERR_ALIGN.to_string()]);
        assert!(sample_errors(Family::XboxOne, false, false, 0x1_0000_0000, true, 1, 8).is_empty());
    }

    #[test]
    fn sample_layout_follows_entry_offsets_and_only_sourced_entries_are_written() {
        let s = set(vec![
            entry(0, 0, 1, 1, false, 0, PcSource::ConsoleClock { hz: 853_000_000 }),
            entry(0xb, 4, 1, 2, false, 8, PcSource::None),
            entry(2, 0, 1, 1, false, 24, PcSource::ConsoleClock { hz: 853_000_000 }),
            entry(0x15, 1, 3, 1, true, 32, PcSource::None),
        ]);
        assert_eq!(s.sample_bytes, 56);
        let (w, missing) = plan_sample(&s, 0x100);
        assert_eq!(w, vec![Write { dst_off: 0x100, hz: 853_000_000 }, Write { dst_off: 0x118, hz: 853_000_000 }]);
        assert_eq!(missing, vec![1, 3]);
        assert_eq!(distinct_hz(&w), vec![853_000_000]);
        assert_eq!(s.entries[1].values(), 2);
        assert_eq!(s.entries[3].values(), 3);
    }

    #[test]
    fn scarlett_sets_use_their_harvest_adjusted_value_counts() {
        let xe = |block: u8, inst: u16, eng: u16, arr: u16, harv: u16, off: u32, source: PcSource| crate::perfctr_xs::XsEntry { block, slot: 0, select: 0, instances: inst, engines: eng, arrays: arr, harvested: harv, sample_offset: off, result_offset: off, source };
        let s = crate::perfctr_xs::XsCounterSet { lockhart: false, sq_ctrl: 0xffff, entries: vec![
            xe(0xe, 1, 1, 1, 0, 0, PcSource::ConsoleClock { hz: 1_825_000_000 }),
            xe(0xb, 2, 2, 2, 1, 8, PcSource::None),
            xe(3, 1, 1, 1, 0, 56, PcSource::ConsoleClock { hz: 1_825_000_000 }),
        ], sample_bytes: 64 };
        let c = cells_of_xs(&s);
        assert_eq!(c.iter().map(|x| x.values).collect::<Vec<_>>(), vec![1, 6, 1]);
        let (w, missing) = plan_cells(&c, 0x40);
        assert_eq!(w.iter().map(|x| x.dst_off).collect::<Vec<_>>(), vec![0x40, 0x40 + 56]);
        assert_eq!(missing, vec![1]);
    }

    #[test]
    fn multi_instance_clock_entries_write_every_value() {
        let s = set(vec![entry(3, 0, 2, 2, false, 16, PcSource::ConsoleClock { hz: 1_172_000_000 })]);
        let (w, _) = plan_sample(&s, 8);
        assert_eq!(w.iter().map(|x| x.dst_off).collect::<Vec<_>>(), vec![24, 32, 40, 48]);
    }

    #[test]
    fn clock_conversion_is_exact_against_wide_math() {
        let cases = [(0u64, 1_000_000_000u64, 853_000_000u64, 1_000_000_000u64), (5, 123_456_789_012, 914_000_000, 1_000_000_000), (77, 3_600_000_000_000, 1_172_000_000, 1_000_000_000), (1, 10_000_000_000, 853_000_000, 10_000_000_000)];
        for (b, n, hz, f) in cases {
            let want = ((n - b) as u128 * hz as u128 / f as u128) as u64;
            assert_eq!(clock_value(b, n, hz, f), want);
        }
        assert_eq!(clock_value(10, 10, 853_000_000, 1_000_000_000), 0);
        assert_eq!(constants(3, 300, 302, 9, 0x1_0000_0002, 1_000_000_000), [3, 300, 302, 9, 2, 1, 1_000_000_000, 0]);
    }

    #[test]
    fn scratch_rings_keep_begin_slots_apart_from_sample_slots() {
        assert_eq!(begin_slot_index(BEGIN_SLOTS + 5), 5);
        assert_eq!(scratch_slot_index(0), BEGIN_SLOTS);
        assert_eq!(scratch_slot_index(1), BEGIN_SLOTS + 2);
        assert_eq!(scratch_slot_index((SCRATCH_SLOTS - BEGIN_SLOTS) / 2), BEGIN_SLOTS);
        assert!((0..10_000).all(|i| (BEGIN_SLOTS..SCRATCH_SLOTS - 1).contains(&scratch_slot_index(i))));
        let a = next_seq();
        assert!(a != 0 && next_seq() != a);
    }

    #[test]
    fn mirror_keeps_recent_candidates_and_only_executed_cells_count() {
        for i in 0..(MIRROR_CANDIDATES as u32 + 3) {
            note_mirror(0xdead_0000, 0, i, 100 + i);
        }
        note_mirror(0xdead_0000, 8, 9, 50);
        {
            let g = MIRROR.lock().unwrap();
            let m = g.as_ref().unwrap();
            let c = &m.by_key[&(0xdead_0000, 0)];
            assert_eq!(c.len(), MIRROR_CANDIDATES);
            assert_eq!(c.last(), Some(&(MIRROR_CANDIDATES as u32 + 2, 100 + MIRROR_CANDIDATES as u32 + 2)));
            assert_eq!(c[0], (3, 103));
            assert_eq!(m.by_key[&(0xdead_0000, 8)], vec![(9, 50)]);
        }
        assert_eq!(executed_value(&[1234, 7], 7), Some(1234));
        assert_eq!(executed_value(&[1234, 6], 7), None);
        assert_eq!(executed_value(&[0, 0], 7), None);
    }

    #[test]
    fn read_sample_needs_every_cell_written() {
        assert_eq!(read_sample(0x1234_5000, 4, 8), None);
        assert_eq!(read_sample(0x1234_5000, 0, 8), None);
    }

    #[test]
    fn slots_install_at_100_to_102_on_both_layouts() {
        for fam in [Family::Scarlett, Family::XboxOne] {
            let vt: Vec<AtomicUsize> = (0..128).map(|i| AtomicUsize::new(7000 + i)).collect();
            assert_eq!(install_for(&vt, fam), 3);
            assert_eq!(vt[99].load(Ordering::Relaxed), 7099);
            assert_eq!(vt[100].load(Ordering::Relaxed), begin_counters_x as *const () as usize);
            assert_eq!(vt[101].load(Ordering::Relaxed), sample_counters_x as *const () as usize);
            assert_eq!(vt[102].load(Ordering::Relaxed), end_counters_x as *const () as usize);
            assert_eq!(vt[103].load(Ordering::Relaxed), 7103);
        }
    }

    #[test]
    fn conversion_shader_compiles_when_dxc_is_present() {
        if !tombox_xshader::dxc::available() {
            return;
        }
        let code = tombox_xshader::dxc::compile(HLSL, "cs_6_0").unwrap();
        assert_eq!(&code[0..4], b"DXBC");
        assert!(code.windows(4).any(|w| w == b"RTS0"));
    }
}
