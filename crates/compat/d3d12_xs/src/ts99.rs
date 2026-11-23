use core::ffi::c_void;
use core::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::collections::HashMap;
use std::sync::Mutex;
use compat_common::record;

use crate::device_wrap::Guid;
use crate::sys_ts::Calibration;

pub use xgs_kmd_emu::gpu_clock::CONSOLE_TIMESTAMP_HZ;

pub const HLSL: &str = r#"#define RS "RootConstants(num32BitConstants=10, b0), UAV(u0)"
cbuffer C : register(b0) { uint4 c0; uint4 c1; uint base; uint count; };
RWByteAddressBuffer s : register(u0);
uint64_t w(uint lo, uint hi) { return ((uint64_t)hi << 32) | (uint64_t)lo; }
[RootSignature(RS)]
[numthreads(64, 1, 1)]
void main(uint3 id : SV_DispatchThreadID) {
    if (id.x >= count) return;
    uint o = (base + id.x) * 8;
    uint2 r = s.Load2(o);
    uint64_t t = w(r.x, r.y);
    uint64_t g = w(c0.x, c0.y);
    uint64_t sys = w(c0.z, c0.w);
    uint64_t num = w(c1.x, c1.y);
    uint64_t den = w(c1.z, c1.w);
    bool neg = t < g;
    uint64_t d = neg ? g - t : t - g;
    uint64_t v = (d / den) * num + ((d % den) * num) / den;
    uint64_t res = neg ? sys - v : sys + v;
    s.Store2(o, uint2((uint)res, (uint)(res >> 32)));
}
"#;

pub const RING: u32 = 1 << 16;
pub const GROUP: u32 = 64;
const QUERY_TYPE_TIMESTAMP: u32 = 2;
const STATE_COMMON: u32 = 0;
const STATE_UAV: u32 = 0x8;
const STATE_COPY_DEST: u32 = 0x400;
const STATE_COPY_SOURCE: u32 = 0x800;
const PC_CLOSE: usize = 9;
const PC_RESET: usize = 10;
const PC_DISPATCH: usize = 14;
const PC_COPY_BUFFER_REGION: usize = 15;
const PC_SET_PSO: usize = 25;
const PC_SET_COMPUTE_ROOT_SIGNATURE: usize = 29;
const PC_SET_COMPUTE_ROOT_CONSTANTS: usize = 35;
const PC_SET_COMPUTE_ROOT_UAV: usize = 41;
const PC_RESOLVE_QUERY_DATA: usize = 54;
const PC_SET_PREDICATION: usize = 55;
const DEV_CREATE_QUEUE: usize = 8;
const DEV_CREATE_ALLOCATOR: usize = 9;
const DEV_CREATE_COMPUTE_PSO: usize = 11;
const DEV_CREATE_LIST: usize = 12;
const DEV_CREATE_ROOT_SIGNATURE: usize = 16;
const DEV_CREATE_COMMITTED: usize = 27;
const DEV_CREATE_FENCE: usize = 36;
const Q_EXECUTE: usize = 10;
const Q_SIGNAL: usize = 14;
const Q_WAIT: usize = 15;
const Q_FREQUENCY: usize = 16;
const Q_CALIBRATION: usize = 17;
const FENCE_COMPLETED: usize = 8;
const ALLOC_RESET: usize = 8;
const IID_DEVICE: Guid = Guid { d1: 0x1898_19f1, d2: 0x1db6, d3: 0x4b57, d4: [0xbe, 0x54, 0x18, 0x21, 0x33, 0x9b, 0x85, 0xf7] };
const IID_QUEUE: Guid = Guid { d1: 0x0ec8_70a6, d2: 0x5d7e, d3: 0x4c22, d4: [0x8c, 0xfc, 0x5b, 0xaa, 0xe0, 0x76, 0x16, 0xed] };
const IID_ALLOCATOR: Guid = Guid { d1: 0x6102_dee4, d2: 0xaf59, d3: 0x4b09, d4: [0xb9, 0x99, 0xb4, 0x4d, 0x73, 0xf0, 0x9b, 0x24] };
const IID_LIST: Guid = Guid { d1: 0x5b16_0d0f, d2: 0xac1b, d3: 0x4185, d4: [0x8b, 0xa8, 0xb3, 0xae, 0x42, 0xa5, 0xa4, 0x55] };
const IID_FENCE: Guid = Guid { d1: 0x0a75_3dcf, d2: 0xc4d8, d3: 0x4b91, d4: [0xad, 0xf6, 0xbe, 0x5a, 0x60, 0xd9, 0x5a, 0x76] };
const IID_ROOT_SIGNATURE: Guid = Guid { d1: 0xc54a_6b66, d2: 0x72df, d3: 0x4ee8, d4: [0x8b, 0xe5, 0xa9, 0x46, 0xa1, 0x42, 0x92, 0x14] };
const IID_PSO: Guid = Guid { d1: 0x765a_30f3, d2: 0xf624, d3: 0x4c6f, d4: [0xa8, 0x28, 0xac, 0xe9, 0x48, 0x62, 0x24, 0x45] };
const IID_RESOURCE: Guid = Guid { d1: 0x6964_42be, d2: 0xa72e, d3: 0x4059, d4: [0xbc, 0x79, 0x5b, 0x5c, 0x98, 0x04, 0x0f, 0xad] };

extern "system" {
    fn QueryPerformanceFrequency(v: *mut i64) -> i32;
}

#[derive(Clone, Copy)]
struct Pipe {
    rs: usize,
    pso: usize,
}

#[derive(Clone, Copy)]
struct Scratch {
    buf: usize,
    va: u64,
}

#[derive(Clone, Copy)]
struct Clock {
    queue: usize,
    cal: Calibration,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pending {
    pub slot: u32,
    pub res: usize,
    pub off: u64,
}

struct Convert {
    alloc: usize,
    list: usize,
    fence: usize,
    done: u64,
}

#[derive(Clone, Copy)]
struct Link {
    to_compute: usize,
    to_copy: usize,
    value: u64,
}

struct Deferred {
    dev: usize,
    queue: usize,
    links: HashMap<usize, Link>,
    pool: Vec<Convert>,
}

static CLOCKS: Mutex<[Option<Clock>; 4]> = Mutex::new([None; 4]);
static PIPE: Mutex<Option<Option<Pipe>>> = Mutex::new(None);
static SCRATCH: Mutex<[Option<Scratch>; 3]> = Mutex::new([None; 3]);
static NEXT: [AtomicU32; 3] = [const { AtomicU32::new(0) }; 3];
static PENDING: Mutex<Option<HashMap<usize, Vec<Pending>>>> = Mutex::new(None);
static DEFERRED: Mutex<Option<Deferred>> = Mutex::new(None);
static LOG: AtomicUsize = AtomicUsize::new(0);
static CONVERTED: AtomicU64 = AtomicU64::new(0);

fn once(n: usize) -> bool {
    LOG.fetch_add(1, Ordering::Relaxed) < n
}

unsafe fn vslot(obj: *mut c_void, i: usize) -> usize {
    unsafe { *(*(obj as *const *const usize)).add(i) }
}

unsafe fn release(obj: *mut c_void) {
    let f: unsafe extern "system" fn(*mut c_void) -> u32 = unsafe { core::mem::transmute(vslot(obj, 2)) };
    unsafe { f(obj) };
}

unsafe fn device_of(obj: *mut c_void) -> *mut c_void {
    let get: unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> i32 = unsafe { core::mem::transmute(vslot(obj, 7)) };
    let mut dev: *mut c_void = core::ptr::null_mut();
    if unsafe { get(obj, &IID_DEVICE, &mut dev) } < 0 {
        return core::ptr::null_mut();
    }
    dev
}

pub fn clock_index(list_type: u32) -> usize {
    match list_type {
        2 => 2,
        3 => 3,
        _ => 0,
    }
}

pub fn scratch_index(list_type: u32) -> usize {
    match list_type {
        2 => 1,
        3 => 2,
        _ => 0,
    }
}

pub fn ring_take(cur: u32, n: u32, ring: u32) -> (u32, u32) {
    let start = if cur.checked_add(n).map_or(true, |e| e > ring) { 0 } else { cur };
    (start, start + n)
}

pub fn chunks(n: u32, max: u32) -> Vec<(u32, u32)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < n {
        let c = (n - i).min(max);
        out.push((i, c));
        i += c;
    }
    out
}

pub fn constants(c: &Calibration, base: u32, count: u32) -> [u32; 10] {
    let k = crate::sys_ts::constants(c, 0);
    [k[0], k[1], k[2], k[3], k[4], k[5], k[6], k[7], base, count]
}

pub fn calibration_from(gpu_ticks: u64, gpu_freq: u64, qpc: u64, qpf: u64) -> Calibration {
    crate::sys_ts::counter_calibration(gpu_ticks, gpu_freq, qpc, qpf, CONSOLE_TIMESTAMP_HZ)
}

pub fn convert(t: u64, c: &Calibration) -> u64 {
    crate::sys_ts::convert(t, c)
}

fn host_fn(queue: *mut c_void, slot: usize) -> usize {
    let r = crate::present::real_queue_slot(slot);
    if r != 0 { r } else { unsafe { vslot(queue, slot) } }
}

unsafe fn host_calibration(queue: *mut c_void) -> Option<(u64, u64)> {
    let f: crate::timestamp::CalibFn = unsafe { core::mem::transmute(host_fn(queue, Q_CALIBRATION)) };
    let (mut g, mut c) = (0u64, 0u64);
    (unsafe { f(queue, &mut g, &mut c) } >= 0).then_some((g, c))
}

unsafe fn create_queue(dev: *mut c_void, ty: u32) -> *mut c_void {
    let desc: [u32; 4] = [ty, 0, 0, 0];
    let cq: unsafe extern "system" fn(*mut c_void, *const u32, *const Guid, *mut *mut c_void) -> i32 = unsafe { core::mem::transmute(vslot(dev, DEV_CREATE_QUEUE)) };
    let mut q: *mut c_void = core::ptr::null_mut();
    let hr = unsafe { cq(dev, desc.as_ptr(), &IID_QUEUE, &mut q) };
    if hr < 0 || q.is_null() {
        record("d3d12_xs", "ts99", &format!("calibration queue type {ty} FAILED hr={hr:#010x}"));
        return core::ptr::null_mut();
    }
    q
}

unsafe fn clock(dev: *mut c_void, list_type: u32) -> Option<Clock> {
    let i = clock_index(list_type);
    let mut g = CLOCKS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(c) = g[i] {
        return Some(c);
    }
    let q = unsafe { create_queue(dev, i as u32) };
    if q.is_null() {
        return None;
    }
    let f: crate::timestamp::FreqFn = unsafe { core::mem::transmute(host_fn(q, Q_FREQUENCY)) };
    let mut gf = 0u64;
    let mut qpf = 0i64;
    unsafe { QueryPerformanceFrequency(&mut qpf) };
    let Some((gt, ct)) = (unsafe { host_calibration(q) }) else {
        unsafe { release(q) };
        return None;
    };
    if unsafe { f(q, &mut gf) } < 0 || gf == 0 || qpf <= 0 {
        unsafe { release(q) };
        return None;
    }
    let cal = calibration_from(gt, gf, ct, qpf as u64);
    record("d3d12_xs", "ts99", &format!(
        "GPU timestamp time base for queue type {i}: {CONSOLE_TIMESTAMP_HZ} Hz (console queue slot 16 constant 0x5f21070), epoch = QPC zero; fixed calibration host gpu={gt} @ {gf} Hz <-> qpc={ct} @ {qpf} Hz -> console {}", cal.sys));
    let c = Clock { queue: q as usize, cal };
    g[i] = Some(c);
    drop(g);
    if i == 0 {
        xgs_kmd_emu::gpu_clock::set_clock_source(clock_now);
    }
    Some(c)
}

unsafe fn clock_now() -> Option<u64> {
    let c = CLOCKS.lock().unwrap_or_else(|e| e.into_inner())[0]?;
    let (g, _) = unsafe { host_calibration(c.queue as *mut c_void) }?;
    Some(convert(g, &c.cal))
}

pub unsafe fn title_frequency(out: *mut u64) -> i32 {
    unsafe { *out = CONSOLE_TIMESTAMP_HZ };
    0
}

pub unsafe fn title_calibration(queue: *mut c_void, gpu: *mut u64, cpu: *mut u64) -> i32 {
    let dev = unsafe { device_of(queue) };
    if !dev.is_null() {
        unsafe { clock(dev, 0) };
        unsafe { release(dev) };
    }
    let mut req = [0u8; 0x18];
    req[8..16].copy_from_slice(&(gpu as u64).to_le_bytes());
    req[16..24].copy_from_slice(&(cpu as u64).to_le_bytes());
    if let Err(e) = xgs_kmd_emu::gpu_clock::calibration_ioctl(&req) {
        if once(16) {
            record("d3d12_xs", "ts99", &format!("GetClockCalibration: IOCTL 0x7900c213 failed with {e} (gpu={gpu:p} cpu={cpu:p}); the console UMD returns S_OK regardless"));
        }
    }
    0
}

unsafe fn pipe(dev: *mut c_void) -> Option<Pipe> {
    let mut g = PIPE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(p) = *g {
        return p;
    }
    let p = match tombox_xshader::dxc::compile(HLSL, "cs_6_0") {
        Ok(code) => unsafe { build_pipe(dev, &code) },
        Err(e) => {
            record("d3d12_xs", "ts99", &format!("timestamp conversion shader: {e}"));
            None
        }
    };
    *g = Some(p);
    p
}

unsafe fn build_pipe(dev: *mut c_void, code: &[u8]) -> Option<Pipe> {
    let crs: unsafe extern "system" fn(*mut c_void, u32, *const u8, usize, *const Guid, *mut *mut c_void) -> i32 = unsafe { core::mem::transmute(vslot(dev, DEV_CREATE_ROOT_SIGNATURE)) };
    let mut rs: *mut c_void = core::ptr::null_mut();
    let hr = unsafe { crs(dev, 0, code.as_ptr(), code.len(), &IID_ROOT_SIGNATURE, &mut rs) };
    if hr < 0 || rs.is_null() {
        record("d3d12_xs", "ts99", &format!("conversion root signature FAILED hr={hr:#010x}"));
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
        record("d3d12_xs", "ts99", &format!("conversion PSO FAILED hr={hr:#010x}"));
        return None;
    }
    Some(Pipe { rs: rs as usize, pso: pso as usize })
}

unsafe fn scratch(dev: *mut c_void, i: usize) -> Option<Scratch> {
    let mut g = SCRATCH.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(s) = g[i] {
        return Some(s);
    }
    let heap: [u32; 5] = [1, 0, 0, 1, 1];
    let mut rd = [0u8; 56];
    rd[0..4].copy_from_slice(&1u32.to_le_bytes());
    rd[0x10..0x18].copy_from_slice(&(RING as u64 * 8).to_le_bytes());
    rd[0x18..0x1c].copy_from_slice(&1u32.to_le_bytes());
    rd[0x1c..0x1e].copy_from_slice(&1u16.to_le_bytes());
    rd[0x1e..0x20].copy_from_slice(&1u16.to_le_bytes());
    rd[0x24..0x28].copy_from_slice(&1u32.to_le_bytes());
    rd[0x2c..0x30].copy_from_slice(&1u32.to_le_bytes());
    rd[0x30..0x34].copy_from_slice(&4u32.to_le_bytes());
    let cc: unsafe extern "system" fn(*mut c_void, *const u32, u32, *const u8, u32, *const c_void, *const Guid, *mut *mut c_void) -> i32 = unsafe { core::mem::transmute(vslot(dev, DEV_CREATE_COMMITTED)) };
    let mut buf: *mut c_void = core::ptr::null_mut();
    let hr = unsafe { cc(dev, heap.as_ptr(), 0, rd.as_ptr(), STATE_COMMON, core::ptr::null(), &IID_RESOURCE, &mut buf) };
    if hr < 0 || buf.is_null() {
        record("d3d12_xs", "ts99", &format!("timestamp scratch {i} FAILED hr={hr:#010x}"));
        return None;
    }
    let va: unsafe extern "system" fn(*mut c_void) -> u64 = unsafe { core::mem::transmute(vslot(buf, 11)) };
    let s = Scratch { buf: buf as usize, va: unsafe { va(buf) } };
    g[i] = Some(s);
    Some(s)
}

fn take(i: usize, n: u32) -> u32 {
    let mut cur = NEXT[i].load(Ordering::Relaxed);
    loop {
        let (start, next) = ring_take(cur, n, RING);
        match NEXT[i].compare_exchange_weak(cur, next, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return start,
            Err(c) => cur = c,
        }
    }
}

unsafe fn resolve(list: *mut c_void, heap: *mut c_void, start: u32, n: u32, dst: usize, off: u64) {
    let f: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, u32, u32, *mut c_void, u64) = unsafe { core::mem::transmute(crate::cmdlist_wrap::pc_method(list, PC_RESOLVE_QUERY_DATA)) };
    unsafe { f(list, heap, QUERY_TYPE_TIMESTAMP, start, n, dst as *mut c_void, off) };
}

unsafe fn copy(list: *mut c_void, dst: usize, doff: u64, src: usize, soff: u64, n: u64) {
    let f: unsafe extern "system" fn(*mut c_void, usize, u64, usize, u64, u64) = unsafe { core::mem::transmute(crate::cmdlist_wrap::pc_method(list, PC_COPY_BUFFER_REGION)) };
    unsafe { f(list, dst, doff, src, soff, n) };
}

unsafe fn bind(list: *mut c_void, p: Pipe, s: Scratch) {
    unsafe {
        let srs: unsafe extern "system" fn(*mut c_void, *mut c_void) = core::mem::transmute(crate::cmdlist_wrap::pc_method(list, PC_SET_COMPUTE_ROOT_SIGNATURE));
        srs(list, p.rs as *mut c_void);
        let spso: unsafe extern "system" fn(*mut c_void, *mut c_void) = core::mem::transmute(crate::cmdlist_wrap::pc_method(list, PC_SET_PSO));
        spso(list, p.pso as *mut c_void);
        let su: unsafe extern "system" fn(*mut c_void, u32, u64) = core::mem::transmute(crate::cmdlist_wrap::pc_method(list, PC_SET_COMPUTE_ROOT_UAV));
        su(list, 1, s.va);
    }
}

unsafe fn dispatch(list: *mut c_void, cal: &Calibration, base: u32, count: u32) {
    let _ij = unsafe { crate::inject::scope(list, crate::inject::QUERY) };
    let k = constants(cal, base, count);
    unsafe {
        let sc: unsafe extern "system" fn(*mut c_void, u32, u32, *const u32, u32) = core::mem::transmute(crate::cmdlist_wrap::pc_method(list, PC_SET_COMPUTE_ROOT_CONSTANTS));
        sc(list, 0, 10, k.as_ptr(), 0);
        let d: unsafe extern "system" fn(*mut c_void, u32, u32, u32) = core::mem::transmute(crate::cmdlist_wrap::pc_method(list, PC_DISPATCH));
        d(list, count.div_ceil(GROUP), 1, 1);
    }
}

pub unsafe fn emit(list: *mut c_void, heap: *mut c_void, start: u32, n: u32, dst: usize, off: u64, dst_state: Option<u32>) -> bool {
    let _ij = unsafe { crate::inject::scope(list, crate::inject::QUERY) };
    if n == 0 {
        return true;
    }
    let ty = unsafe { crate::fill::list_type(list) };
    if ty == crate::fill::LIST_TYPE_COPY {
        return false;
    }
    let dev = unsafe { device_of(list) };
    if dev.is_null() {
        return false;
    }
    let parts = unsafe { (clock(dev, ty), pipe(dev), scratch(dev, scratch_index(ty))) };
    unsafe { release(dev) };
    let (Some(c), Some(p), Some(s)) = parts else {
        if once(16) {
            record("d3d12_xs", "ts99", "GPU timestamp conversion unavailable (calibration, shader or scratch failed); timestamp not written");
        }
        return false;
    };
    unsafe {
        let paused = crate::cl2_state::pause_queries(list, false);
        let pred: unsafe extern "system" fn(*mut c_void, *mut c_void, u64, u32) = core::mem::transmute(crate::cmdlist_wrap::pc_method(list, PC_SET_PREDICATION));
        pred(list, core::ptr::null_mut(), 0, 0);
        for (i, cnt) in chunks(n, RING) {
            let base = take(scratch_index(ty), cnt);
            let soff = base as u64 * 8;
            crate::fill::barrier_pub(list, s.buf, STATE_COMMON, STATE_COPY_DEST);
            resolve(list, heap, start + i, cnt, s.buf, soff);
            crate::fill::barrier_pub(list, s.buf, STATE_COPY_DEST, STATE_UAV);
            bind(list, p, s);
            dispatch(list, &c.cal, base, cnt);
            crate::fill::barrier_pub(list, s.buf, STATE_UAV, STATE_COPY_SOURCE);
            let tr = dst_state.filter(|&st| st != STATE_COPY_DEST);
            if let Some(st) = tr { crate::fill::barrier_pub(list, dst, st, STATE_COPY_DEST); }
            copy(list, dst, off + i as u64 * 8, s.buf, soff, cnt as u64 * 8);
            if let Some(st) = tr { crate::fill::barrier_pub(list, dst, STATE_COPY_DEST, st); }
            crate::fill::barrier_pub(list, s.buf, STATE_COPY_SOURCE, STATE_COMMON);
        }
        crate::cl2_state::reapply(list);
        crate::cl2_state::unpause_queries(list, &paused);
    }
    crate::placed::note_gpu_write(dst as *mut c_void);
    CONVERTED.fetch_add(n as u64, Ordering::Relaxed);
    true
}

pub unsafe fn after_resolve(list: *mut c_void, heap: *mut c_void, ty: u32, start: u32, count: u32, dest: *mut c_void, offset: u64) {
    if ty != QUERY_TYPE_TIMESTAMP || dest.is_null() || heap.is_null() || count == 0 {
        return;
    }
    if !unsafe { emit(list, heap, start, count, dest as usize, offset, None) } && once(16) {
        record("d3d12_xs", "ts99", &format!("ResolveQueryData TIMESTAMP start={start} count={count} dest={dest:p}+{offset:#x}: not converted to the {CONSOLE_TIMESTAMP_HZ} Hz time base"));
    }
}

static REAL_RESOLVE: AtomicUsize = AtomicUsize::new(0);

unsafe extern "system" fn resolve_x(this: *mut c_void, heap: *mut c_void, ty: u32, start: u32, count: u32, dest: *mut c_void, offset: u64) {
    let f: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, u32, u32, *mut c_void, u64) = unsafe { core::mem::transmute(REAL_RESOLVE.load(Ordering::Relaxed)) };
    unsafe { f(this, heap, ty, start, count, dest, offset) };
    unsafe { after_resolve(this, heap, ty, start, count, dest, offset) };
}

pub fn install_plain_resolve(xbox_vt: &[AtomicUsize], slot: usize, real: usize) {
    REAL_RESOLVE.store(real, Ordering::Relaxed);
    xbox_vt[slot].store(resolve_x as usize, Ordering::Relaxed);
}

pub unsafe fn defer_dma(list: *mut c_void, heap: *mut c_void, idx: u32, res: usize, off: u64) -> bool {
    let dev = unsafe { device_of(list) };
    if dev.is_null() {
        return false;
    }
    let parts = unsafe { (clock(dev, 3), clock(dev, 2), pipe(dev), scratch(dev, 2)) };
    unsafe { release(dev) };
    let (Some(_), Some(_), Some(_), Some(s)) = parts else { return false };
    let slot = take(2, 1);
    unsafe {
        crate::fill::barrier_pub(list, s.buf, STATE_COMMON, STATE_COPY_DEST);
        resolve(list, heap, idx, 1, s.buf, slot as u64 * 8);
        crate::fill::barrier_pub(list, s.buf, STATE_COPY_DEST, STATE_COMMON);
    }
    let mut g = PENDING.lock().unwrap_or_else(|e| e.into_inner());
    g.get_or_insert_with(HashMap::new).entry(list as usize).or_default().push(Pending { slot, res, off });
    true
}

pub fn forget_list(list: *mut c_void) {
    if let Some(m) = PENDING.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        m.remove(&(list as usize));
    }
}

pub fn pending_of(lists: &[*mut c_void]) -> Vec<Pending> {
    let g = PENDING.lock().unwrap_or_else(|e| e.into_inner());
    let Some(m) = g.as_ref() else { return Vec::new() };
    lists.iter().filter_map(|l| m.get(&(*l as usize))).flatten().copied().collect()
}

pub fn runs(p: &[Pending]) -> Vec<(u32, u32)> {
    let mut s: Vec<u32> = p.iter().map(|e| e.slot).collect();
    s.sort_unstable();
    s.dedup();
    let mut out: Vec<(u32, u32)> = Vec::new();
    for v in s {
        match out.last_mut() {
            Some((b, n)) if *b + *n == v => *n += 1,
            _ => out.push((v, 1)),
        }
    }
    out
}

unsafe fn deferred_init(dev: *mut c_void) -> Option<Deferred> {
    let cq = CLOCKS.lock().unwrap_or_else(|e| e.into_inner())[2]?.queue;
    Some(Deferred { dev: dev as usize, queue: cq, links: HashMap::new(), pool: Vec::new() })
}

unsafe fn link(d: &mut Deferred, queue: *mut c_void) -> Option<Link> {
    if let Some(l) = d.links.get(&(queue as usize)) {
        return Some(*l);
    }
    let dev = d.dev as *mut c_void;
    let cf: unsafe extern "system" fn(*mut c_void, u64, u32, *const Guid, *mut *mut c_void) -> i32 = unsafe { core::mem::transmute(vslot(dev, DEV_CREATE_FENCE)) };
    let (mut a, mut b): (*mut c_void, *mut c_void) = (core::ptr::null_mut(), core::ptr::null_mut());
    if unsafe { cf(dev, 0, 0, &IID_FENCE, &mut a) } < 0 || unsafe { cf(dev, 0, 0, &IID_FENCE, &mut b) } < 0 {
        return None;
    }
    let l = Link { to_compute: a as usize, to_copy: b as usize, value: 0 };
    d.links.insert(queue as usize, l);
    Some(l)
}

unsafe fn completed(fence: usize) -> u64 {
    unsafe {
        let f: unsafe extern "system" fn(*mut c_void) -> u64 = core::mem::transmute(vslot(fence as *mut c_void, FENCE_COMPLETED));
        f(fence as *mut c_void)
    }
}

unsafe fn convert_list(d: &mut Deferred) -> Option<usize> {
    let dev = d.dev as *mut c_void;
    let idx = match d.pool.iter().position(|c| c.fence == 0 || unsafe { completed(c.fence) } >= c.done) {
        Some(i) => {
            let c = &d.pool[i];
            unsafe {
                let ar: unsafe extern "system" fn(*mut c_void) -> i32 = core::mem::transmute(vslot(c.alloc as *mut c_void, ALLOC_RESET));
                if ar(c.alloc as *mut c_void) < 0 {
                    return None;
                }
                let lr: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32 = core::mem::transmute(vslot(c.list as *mut c_void, PC_RESET));
                if lr(c.list as *mut c_void, c.alloc as *mut c_void, core::ptr::null_mut()) < 0 {
                    return None;
                }
            }
            i
        }
        None => {
            let ca: unsafe extern "system" fn(*mut c_void, u32, *const Guid, *mut *mut c_void) -> i32 = unsafe { core::mem::transmute(vslot(dev, DEV_CREATE_ALLOCATOR)) };
            let mut a: *mut c_void = core::ptr::null_mut();
            if unsafe { ca(dev, 2, &IID_ALLOCATOR, &mut a) } < 0 {
                return None;
            }
            let cl: unsafe extern "system" fn(*mut c_void, u32, u32, *mut c_void, *mut c_void, *const Guid, *mut *mut c_void) -> i32 = unsafe { core::mem::transmute(vslot(dev, DEV_CREATE_LIST)) };
            let mut l: *mut c_void = core::ptr::null_mut();
            if unsafe { cl(dev, 0, 2, a, core::ptr::null_mut(), &IID_LIST, &mut l) } < 0 {
                unsafe { release(a) };
                return None;
            }
            d.pool.push(Convert { alloc: a as usize, list: l as usize, fence: 0, done: 0 });
            d.pool.len() - 1
        }
    };
    Some(idx)
}

pub unsafe fn after_execute(queue: *mut c_void, lists: &[*mut c_void]) {
    let work = pending_of(lists);
    if work.is_empty() {
        return;
    }
    let (copy_cal, compute) = {
        let g = CLOCKS.lock().unwrap_or_else(|e| e.into_inner());
        (g[3].map(|c| c.cal), g[2].is_some())
    };
    let sc = SCRATCH.lock().unwrap_or_else(|e| e.into_inner())[2];
    let p = PIPE.lock().unwrap_or_else(|e| e.into_inner()).flatten();
    let (Some(cal), true, Some(s), Some(p)) = (copy_cal, compute, sc, p) else { return };
    let mut g = DEFERRED.lock().unwrap_or_else(|e| e.into_inner());
    if g.is_none() {
        let dev = unsafe { device_of(queue) };
        if dev.is_null() {
            return;
        }
        *g = unsafe { deferred_init(dev) };
        unsafe { release(dev) };
    }
    let Some(d) = g.as_mut() else {
        if once(16) {
            record("d3d12_xs", "ts99", "DMA timestamps: deferred conversion queue/fences unavailable; values stay on the host GPU clock");
        }
        return;
    };
    let Some(mut ln) = (unsafe { link(d, queue) }) else { return };
    let Some(i) = (unsafe { convert_list(d) }) else { return };
    let l = d.pool[i].list as *mut c_void;
    unsafe {
        crate::fill::barrier_pub(l, s.buf, STATE_COMMON, STATE_UAV);
        bind(l, p, s);
        for (base, n) in runs(&work) {
            dispatch(l, &cal, base, n);
        }
        crate::fill::barrier_pub(l, s.buf, STATE_UAV, STATE_COPY_SOURCE);
        for e in &work {
            copy(l, e.res, e.off, s.buf, e.slot as u64 * 8, 8);
        }
        crate::fill::barrier_pub(l, s.buf, STATE_COPY_SOURCE, STATE_COMMON);
        let close: unsafe extern "system" fn(*mut c_void) -> i32 = core::mem::transmute(vslot(l, PC_CLOSE));
        if close(l) < 0 {
            return;
        }
        ln.value += 1;
        let v = ln.value;
        let cq = d.queue as *mut c_void;
        let sig: unsafe extern "system" fn(*mut c_void, *mut c_void, u64) -> i32 = core::mem::transmute(host_fn(queue, Q_SIGNAL));
        let wait: unsafe extern "system" fn(*mut c_void, *mut c_void, u64) -> i32 = core::mem::transmute(host_fn(queue, Q_WAIT));
        sig(queue, ln.to_compute as *mut c_void, v);
        let cwait: unsafe extern "system" fn(*mut c_void, *mut c_void, u64) -> i32 = core::mem::transmute(vslot(cq, Q_WAIT));
        cwait(cq, ln.to_compute as *mut c_void, v);
        let exec: unsafe extern "system" fn(*mut c_void, u32, *const *mut c_void) = core::mem::transmute(vslot(cq, Q_EXECUTE));
        let one = [l];
        exec(cq, 1, one.as_ptr());
        let csig: unsafe extern "system" fn(*mut c_void, *mut c_void, u64) -> i32 = core::mem::transmute(vslot(cq, Q_SIGNAL));
        csig(cq, ln.to_copy as *mut c_void, v);
        wait(queue, ln.to_copy as *mut c_void, v);
        d.pool[i].fence = ln.to_copy;
        d.pool[i].done = v;
        d.links.insert(queue as usize, ln);
    }
    for e in &work {
        crate::placed::note_gpu_write(e.res as *mut c_void);
    }
    CONVERTED.fetch_add(work.len() as u64, Ordering::Relaxed);
    if once(16) {
        record("d3d12_xs", "ts99", &format!("DMA timestamps: {} values converted to {CONSOLE_TIMESTAMP_HZ} Hz on the compute queue after copy queue {queue:p}", work.len()));
    }
}

pub fn converted() -> u64 {
    CONVERTED.load(Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shader(t: u64, k: &[u32; 10]) -> u64 {
        let w = |lo: u32, hi: u32| (hi as u64) << 32 | lo as u64;
        let (g, sys, num, den) = (w(k[0], k[1]), w(k[2], k[3]), w(k[4], k[5]), w(k[6], k[7]));
        let (neg, d) = if t < g { (true, g - t) } else { (false, t - g) };
        let v = (d / den) * num + ((d % den) * num) / den;
        if neg { sys.wrapping_sub(v) } else { sys.wrapping_add(v) }
    }

    #[test]
    fn host_ticks_land_on_the_99_75_mhz_qpc_epoch_base() {
        assert_eq!(CONSOLE_TIMESTAMP_HZ, 0x5f2_1070);
        let c = calibration_from(7_000_000_000, 1_000_000_000, 50_000_000, 10_000_000);
        assert_eq!(c.sys, 5 * CONSOLE_TIMESTAMP_HZ);
        assert_eq!(c.sys_freq, CONSOLE_TIMESTAMP_HZ);
        assert_eq!(convert(7_000_000_000, &c), 5 * CONSOLE_TIMESTAMP_HZ);
        assert_eq!(convert(8_000_000_000, &c), 6 * CONSOLE_TIMESTAMP_HZ);
        assert_eq!(convert(6_000_000_000, &c), 4 * CONSOLE_TIMESTAMP_HZ);
        let k = constants(&c, 12, 70);
        assert_eq!((k[4], k[6], k[8], k[9]), (399, 4000, 12, 70));
        let mut last = 0;
        for t in (6_999_000_000u64..7_001_000_000).step_by(999_983) {
            let v = convert(t, &c);
            assert!(v >= last);
            assert_eq!(shader(t, &k), v);
            last = v;
        }
    }

    #[test]
    fn conversion_matches_exact_math_for_host_rates() {
        for gf in [10_000_000u64, 24_000_000, 1_000_000_000, 1_000_000_007, 2_400_000_000] {
            let c = calibration_from(1 << 40, gf, 1 << 34, 10_000_000);
            let k = constants(&c, 0, 1);
            for t in [(1u64 << 40) - gf, 1 << 40, (1 << 40) + 17, (1 << 40) + 3600 * gf, (1 << 40) + 30 * 86_400 * gf] {
                let want = c.sys as i128 + ((t as i128 - c.gpu as i128) * CONSOLE_TIMESTAMP_HZ as i128).div_euclid(gf as i128);
                assert!((shader(t, &k) as i128 - want).abs() <= 1, "gf={gf} t={t}");
            }
        }
    }

    #[test]
    fn scratch_ring_and_chunks_keep_entries_contiguous() {
        assert_eq!(ring_take(0, 5, 16), (0, 5));
        assert_eq!(ring_take(10, 6, 16), (10, 16));
        assert_eq!(ring_take(12, 6, 16), (0, 6));
        assert_eq!(ring_take(u32::MAX, 1, 16), (0, 1));
        assert_eq!(chunks(5, 2), vec![(0, 2), (2, 2), (4, 1)]);
        assert_eq!(chunks(0, 2), vec![]);
        assert_eq!(chunks(RING + 3, RING), vec![(0, RING), (RING, 3)]);
        assert_eq!((clock_index(0), clock_index(1), clock_index(2), clock_index(3)), (0, 0, 2, 3));
        assert_eq!((scratch_index(0), scratch_index(1), scratch_index(2), scratch_index(3)), (0, 0, 1, 2));
    }

    #[test]
    fn dma_pending_groups_into_runs_and_is_dropped_on_reset() {
        let p = |slot| Pending { slot, res: 1, off: 0 };
        assert_eq!(runs(&[p(4), p(5), p(9), p(6), p(9)]), vec![(4, 3), (9, 1)]);
        let l = 0x5150usize as *mut c_void;
        PENDING.lock().unwrap().get_or_insert_with(HashMap::new).insert(l as usize, vec![p(1), p(2)]);
        assert_eq!(pending_of(&[l]).len(), 2);
        forget_list(l);
        assert!(pending_of(&[l]).is_empty());
    }

    #[test]
    fn conversion_shader_compiles_when_dxc_is_present() {
        if !tombox_xshader::dxc::available() {
            return;
        }
        let code = tombox_xshader::dxc::compile(HLSL, "cs_6_0").unwrap();
        assert_eq!(&code[0..4], b"DXBC");
    }

    #[test]
    fn title_frequency_is_the_console_constant() {
        let mut v = 0u64;
        assert_eq!(unsafe { title_frequency(&mut v) }, 0);
        assert_eq!(v, 99_750_000);
    }
}
