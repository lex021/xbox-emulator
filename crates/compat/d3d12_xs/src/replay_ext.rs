use core::ffi::c_void;
use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::sync::Mutex;
use compat_common::record;

use crate::replay::{armed, note_resource, push_line};

const N: usize = 128;
static PREV: [AtomicUsize; N] = [const { AtomicUsize::new(0) }; N];

const COPY_SOURCE: u32 = 0x800;
const COPY_DEST: u32 = 0x400;
const SNAP_ONE: u64 = 64 << 20;
const SNAP_TOTAL: u64 = 1536 << 20;
const SNAP_CALLS: usize = 96;
const TABLE_FALLBACK: usize = 32;
const TABLE_CAP: usize = 256;

#[repr(C)]
struct Guid(u32, u16, u16, [u8; 8]);
const IID_DEVICE: Guid = Guid(0x189819f1, 0x1db6, 0x4b57, [0xbe, 0x54, 0x18, 0x21, 0x33, 0x9b, 0x85, 0xf7]);
const IID_FENCE: Guid = Guid(0x0a753dcf, 0xc4d8, 0x4b91, [0xad, 0xf6, 0xbe, 0x5a, 0x60, 0xd9, 0x5a, 0x76]);
const IID_RESOURCE: Guid = Guid(0x696442be, 0xa72e, 0x4059, [0xbc, 0x79, 0x5b, 0x5c, 0x98, 0x04, 0x0f, 0xad]);

extern "system" {
    fn Sleep(ms: u32);
}

unsafe fn vt(obj: *mut c_void) -> *const usize {
    *(obj as *const *const usize)
}

macro_rules! vcall {
    ($obj:expr, $slot:expr, fn($($at:ty),*) -> $rt:ty $(, $arg:expr)*) => {{
        let f: unsafe extern "system" fn(*mut c_void $(, $at)*) -> $rt = core::mem::transmute(*vt($obj).add($slot));
        f($obj $(, $arg)*)
    }};
}

macro_rules! prev {
    ($slot:expr, fn($($t:ty),*) $(-> $r:ty)?) => {{
        let f: unsafe extern "system" fn(*mut c_void $(, $t)*) $(-> $r)? = core::mem::transmute(PREV[$slot].load(Ordering::Relaxed));
        f
    }};
}

fn hex(b: &[u8]) -> String {
    let mut s = String::with_capacity(b.len() * 2);
    for x in b {
        let _ = write!(s, "{x:02x}");
    }
    s
}

unsafe fn bytes<'a>(p: *const u8, n: usize) -> &'a [u8] {
    if p.is_null() || n == 0 {
        &[]
    } else {
        core::slice::from_raw_parts(p, n)
    }
}

unsafe fn loc_res(p: *const u8) -> usize {
    if p.is_null() { 0 } else { core::ptr::read_unaligned(p as *const usize) }
}

unsafe extern "system" fn h_begin_query(this: *mut c_void, heap: *mut c_void, ty: u32, idx: u32) {
    if armed() {
        push_line(this, format!("beginquery heap={:#x} {ty} {idx}", heap as usize));
    }
    prev!(28, fn(*mut c_void, u32, u32))(this, heap, ty, idx)
}
unsafe extern "system" fn h_end_query(this: *mut c_void, heap: *mut c_void, ty: u32, idx: u32) {
    if armed() {
        push_line(this, format!("endquery heap={:#x} {ty} {idx}", heap as usize));
    }
    prev!(29, fn(*mut c_void, u32, u32))(this, heap, ty, idx)
}
unsafe extern "system" fn h_resolve_query(this: *mut c_void, heap: *mut c_void, ty: u32, start: u32, n: u32, dst: *mut c_void, off: u64) {
    if armed() {
        note_resource(dst as usize);
        push_line(this, format!("resolvequery heap={:#x} {ty} {start} {n} dst={:#x} {off:#x}", heap as usize, dst as usize));
    }
    prev!(30, fn(*mut c_void, u32, u32, u32, *mut c_void, u64))(this, heap, ty, start, n, dst, off)
}
unsafe extern "system" fn h_predication(this: *mut c_void, buf: *mut c_void, off: u64, op: u32) {
    if armed() {
        note_resource(buf as usize);
        push_line(this, format!("predication buf={:#x} {off:#x} {op}", buf as usize));
    }
    prev!(31, fn(*mut c_void, u64, u32))(this, buf, off, op)
}
unsafe extern "system" fn h_discard(this: *mut c_void, res: *mut c_void, region: *const u8) {
    if armed() {
        note_resource(res as usize);
        if region.is_null() {
            push_line(this, format!("discard res={:#x} null", res as usize));
        } else {
            let nr = core::ptr::read_unaligned(region as *const u32);
            let rects = core::ptr::read_unaligned(region.add(8) as *const *const u8);
            let first = core::ptr::read_unaligned(region.add(16) as *const u32);
            let subs = core::ptr::read_unaligned(region.add(20) as *const u32);
            push_line(this, format!("discard res={:#x} first={first} subs={subs} {nr} {}", res as usize, hex(bytes(rects, nr as usize * 16))));
        }
    }
    bump(this);
    prev!(55, fn(*mut c_void, *const u8))(this, res, region)
}
unsafe extern "system" fn h_depth_bounds(this: *mut c_void, lo: f32, hi: f32) {
    if armed() {
        push_line(this, format!("depthbounds {:08x} {:08x}", lo.to_bits(), hi.to_bits()));
    }
    prev!(68, fn(f32, f32))(this, lo, hi)
}
unsafe extern "system" fn h_fill(this: *mut c_void, va: u64, size: u32, value: u32, flags: u32) {
    if armed() {
        if let Some((_, res, _, _)) = crate::bindtrace::buffer_at(va) {
            note_resource(res);
        }
        push_line(this, format!("fill {va:#x} {size:#x} {value:08x} {flags:#x}"));
    }
    bump(this);
    prev!(69, fn(u64, u32, u32, u32))(this, va, size, value, flags)
}
unsafe extern "system" fn h_copy_texture_x(this: *mut c_void, dst: *const u8, x: u32, y: u32, z: u32, src: *const u8, bx: *const u8, flags: u32) {
    if armed() {
        note_resource(loc_res(dst));
        note_resource(loc_res(src));
        push_line(this, format!("copytex dst={} {x} {y} {z} src={} box={} flags={flags:#x}", hex(bytes(dst, 48)), hex(bytes(src, 48)), hex(bytes(bx, 24))));
    }
    bump(this);
    prev!(83, fn(*const u8, u32, u32, u32, *const u8, *const u8, u32))(this, dst, x, y, z, src, bx, flags)
}
unsafe extern "system" fn h_copy_resource_x(this: *mut c_void, dst: *mut c_void, src: *mut c_void, flags: u32) {
    if armed() {
        note_resource(dst as usize);
        note_resource(src as usize);
        push_line(this, format!("copyres dst={:#x} src={:#x} flags={flags:#x}", dst as usize, src as usize));
    }
    bump(this);
    prev!(84, fn(*mut c_void, *mut c_void, u32))(this, dst, src, flags)
}

#[derive(Default, Clone)]
struct ListState {
    pso: usize,
    groot: usize,
    croot: usize,
    gtables: BTreeMap<u32, u64>,
    ctables: BTreeMap<u32, u64>,
    gvas: BTreeMap<u32, (char, u64)>,
    cvas: BTreeMap<u32, (char, u64)>,
    ib: u64,
    vbs: BTreeMap<u32, u64>,
    rtvs: Vec<usize>,
    dsv: usize,
    epoch: u64,
}

#[derive(Clone, Default)]
struct PsoInfo {
    hashes: Vec<(&'static str, u64)>,
    ds_write: bool,
}

static LISTS: Mutex<Option<HashMap<usize, ListState>>> = Mutex::new(None);
static FILTER: Mutex<(u64, Vec<u64>)> = Mutex::new((u64::MAX, Vec::new()));
static PSO_INFO: Mutex<Option<HashMap<usize, PsoInfo>>> = Mutex::new(None);
static SNAPPED: Mutex<Option<HashMap<(usize, usize), (usize, u64)>>> = Mutex::new(None);
static PER_HASH: Mutex<Option<HashMap<u64, usize>>> = Mutex::new(None);
static QUEUES: Mutex<Vec<usize>> = Mutex::new(Vec::new());
const SNAP_PER_HASH: usize = 16;

pub fn note_queue(queue: *mut c_void) {
    if queue.is_null() {
        return;
    }
    let mut q = QUEUES.lock().unwrap_or_else(|e| e.into_inner());
    if !q.contains(&(queue as usize)) {
        q.push(queue as usize);
    }
}

fn bump(list: *mut c_void) {
    if tracking() {
        with_list(list, |s| s.epoch += 1);
    }
}

macro_rules! bumper {
    ($name:ident, $slot:literal, fn($($a:ident: $t:ty),*)) => {
        unsafe extern "system" fn $name(this: *mut c_void $(, $a: $t)*) {
            bump(this);
            prev!($slot, fn($($t),*))(this $(, $a)*)
        }
    };
}
bumper!(b_copy_buffer, 15, fn(a: *mut c_void, b: u64, c: *mut c_void, d: u64, e: u64));
bumper!(b_copy_texture, 16, fn(a: *const u8, b: u32, c: u32, d: u32, e: *const u8, f: *const u8));
bumper!(b_copy_resource, 17, fn(a: *mut c_void, b: *mut c_void));
bumper!(b_clear_dsv, 51, fn(a: usize, b: u32, c: f32, d: u8, e: u32, f: *const u8));
bumper!(b_clear_rtv, 52, fn(a: usize, b: *const u8, c: u32, d: *const u8));
bumper!(b_clear_uavu, 53, fn(a: u64, b: usize, c: *mut c_void, d: *const u8, e: u32, f: *const u8));
bumper!(b_clear_uavf, 54, fn(a: u64, b: usize, c: *mut c_void, d: *const u8, e: u32, f: *const u8));
bumper!(b_indirect, 59, fn(a: *mut c_void, b: u32, c: *mut c_void, d: u64, e: *mut c_void, f: u64));

fn filter() -> Vec<u64> {
    let gen = compat_common::live::generation();
    let mut g = FILTER.lock().unwrap_or_else(|e| e.into_inner());
    if g.0 != gen {
        let on = compat_common::live::flag("TOMBOX_RECORD_SNAP", "record_snap", true);
        g.1 = if on { compat_common::live::hashes("TOMBOX_RECORD_FILTER", "record_filter") } else { Vec::new() };
        g.0 = gen;
    }
    g.1.clone()
}

fn tracking() -> bool {
    armed() && !filter().is_empty()
}

fn with_list<R>(list: *mut c_void, f: impl FnOnce(&mut ListState) -> R) -> R {
    let mut g = LISTS.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(HashMap::new).entry(list as usize).or_default())
}

fn pso_info(pso: usize) -> PsoInfo {
    let mut g = PSO_INFO.lock().unwrap_or_else(|e| e.into_inner());
    let m = g.get_or_insert_with(HashMap::new);
    if let Some(v) = m.get(&pso) {
        return v.clone();
    }
    let hashes: Vec<(&'static str, u64)> = crate::device_x::pso_stage_codes(pso)
        .into_iter()
        .filter_map(|(name, code)| crate::shader_xlat::origin(code).map(|(h, _)| (name, h)))
        .collect();
    let ds_write = match crate::device_wrap::gfx_pso_desc(pso) {
        Some((d, _)) if d.len() >= 0x204 => {
            let u = |o: usize| u32::from_le_bytes(d[o..o + 4].try_into().unwrap());
            (u(0x1f0) != 0 && u(0x1f4) != 0) || (u(0x1fc) != 0 && d[0x201] != 0)
        }
        _ => true,
    };
    let v = PsoInfo { hashes, ds_write };
    m.insert(pso, v.clone());
    v
}

unsafe extern "system" fn h_reset(this: *mut c_void, alloc: *mut c_void, pso: *mut c_void) -> i32 {
    if tracking() {
        with_list(this, |s| *s = ListState { pso: pso as usize, ..Default::default() });
    }
    prev!(10, fn(*mut c_void, *mut c_void) -> i32)(this, alloc, pso)
}
unsafe extern "system" fn h_set_pso(this: *mut c_void, pso: *mut c_void) {
    if tracking() {
        with_list(this, |s| s.pso = pso as usize);
    }
    prev!(25, fn(*mut c_void))(this, pso)
}
unsafe extern "system" fn h_croot(this: *mut c_void, rs: *mut c_void) {
    if tracking() {
        with_list(this, |s| {
            if s.croot != rs as usize {
                s.ctables.clear();
                s.cvas.clear();
            }
            s.croot = rs as usize;
        });
    }
    prev!(33, fn(*mut c_void))(this, rs)
}
unsafe extern "system" fn h_groot(this: *mut c_void, rs: *mut c_void) {
    if tracking() {
        with_list(this, |s| {
            if s.groot != rs as usize {
                s.gtables.clear();
                s.gvas.clear();
            }
            s.groot = rs as usize;
        });
    }
    prev!(34, fn(*mut c_void))(this, rs)
}
unsafe extern "system" fn h_ctable(this: *mut c_void, i: u32, h: u64) {
    if tracking() {
        with_list(this, |s| {
            s.ctables.insert(i, h);
        });
    }
    prev!(35, fn(u32, u64))(this, i, h)
}
unsafe extern "system" fn h_gtable(this: *mut c_void, i: u32, h: u64) {
    if tracking() {
        with_list(this, |s| {
            s.gtables.insert(i, h);
        });
    }
    prev!(36, fn(u32, u64))(this, i, h)
}
macro_rules! root_va {
    ($name:ident, $slot:literal, $compute:literal, $kind:literal) => {
        unsafe extern "system" fn $name(this: *mut c_void, i: u32, va: u64) {
            if tracking() {
                with_list(this, |s| {
                    if $compute { s.cvas.insert(i, ($kind, va)); } else { s.gvas.insert(i, ($kind, va)); }
                });
            }
            prev!($slot, fn(u32, u64))(this, i, va)
        }
    };
}
root_va!(h_ccbv, 41, true, 'c');
root_va!(h_gcbv, 42, false, 'c');
root_va!(h_csrv, 43, true, 's');
root_va!(h_gsrv, 44, false, 's');
root_va!(h_cuav, 45, true, 'u');
root_va!(h_guav, 46, false, 'u');
unsafe extern "system" fn h_ib(this: *mut c_void, p: *const u8) {
    if tracking() {
        let va = if p.is_null() { 0 } else { core::ptr::read_unaligned(p as *const u64) };
        with_list(this, |s| s.ib = va);
    }
    prev!(47, fn(*const u8))(this, p)
}
unsafe extern "system" fn h_vbs(this: *mut c_void, start: u32, n: u32, p: *const u8) {
    if tracking() {
        with_list(this, |s| {
            for i in 0..n {
                let va = if p.is_null() { 0 } else { core::ptr::read_unaligned(p.add(i as usize * 16) as *const u64) };
                s.vbs.insert(start + i, va);
            }
        });
    }
    prev!(48, fn(u32, u32, *const u8))(this, start, n, p)
}
unsafe extern "system" fn h_targets(this: *mut c_void, n: u32, handles: *const usize, single: i32, dsv: *const usize) {
    if tracking() {
        let mut rtvs = Vec::new();
        for i in 0..(n as usize).min(8) {
            if handles.is_null() || (single != 0 && i > 0) {
                break;
            }
            rtvs.push(*handles.add(if single != 0 { 0 } else { i }));
        }
        let d = if dsv.is_null() { 0 } else { *dsv };
        with_list(this, |s| {
            s.rtvs = rtvs;
            s.dsv = d;
        });
    }
    prev!(50, fn(u32, *const usize, i32, *const usize))(this, n, handles, single, dsv)
}

unsafe fn around(this: *mut c_void, compute: bool, call: impl FnOnce()) {
    if !tracking() {
        call();
        return;
    }
    let snap = snap_call(this, compute, true);
    call();
    if snap {
        snap_call(this, compute, false);
    } else {
        with_list(this, |s| s.epoch += 1);
    }
}
unsafe extern "system" fn h_draw(this: *mut c_void, a: u32, b: u32, c: u32, d: u32) {
    around(this, false, || prev!(12, fn(u32, u32, u32, u32))(this, a, b, c, d));
}
unsafe extern "system" fn h_draw_indexed(this: *mut c_void, a: u32, b: u32, c: u32, d: i32, e: u32) {
    around(this, false, || prev!(13, fn(u32, u32, u32, i32, u32))(this, a, b, c, d, e));
}
unsafe extern "system" fn h_dispatch(this: *mut c_void, a: u32, b: u32, c: u32) {
    around(this, true, || prev!(14, fn(u32, u32, u32))(this, a, b, c));
}

pub fn table_sizes(blob: &[u8]) -> BTreeMap<u32, usize> {
    let mut out = BTreeMap::new();
    let rd = |b: &[u8], o: usize| -> Option<u32> { b.get(o..o + 4).map(|x| u32::from_le_bytes(x.try_into().unwrap())) };
    let mut rts = blob;
    if blob.len() >= 32 && &blob[0..4] == b"DXBC" {
        let n = rd(blob, 28).unwrap_or(0) as usize;
        let mut found = None;
        for i in 0..n.min(64) {
            let Some(off) = rd(blob, 32 + i * 4) else { break };
            let off = off as usize;
            if blob.get(off..off + 4) == Some(b"RTS0") {
                let sz = rd(blob, off + 4).unwrap_or(0) as usize;
                found = blob.get(off + 8..(off + 8 + sz).min(blob.len()));
            }
        }
        match found {
            Some(f) => rts = f,
            None => return out,
        }
    }
    let (Some(ver), Some(np), Some(po)) = (rd(rts, 0), rd(rts, 4), rd(rts, 8)) else { return out };
    let rsize = if ver >= 2 { 24 } else { 20 };
    for p in 0..np.min(64) as usize {
        let base = po as usize + p * 12;
        let (Some(ty), Some(payload)) = (rd(rts, base), rd(rts, base + 8)) else { break };
        if ty != 0 {
            continue;
        }
        let (Some(nr), Some(ro)) = (rd(rts, payload as usize), rd(rts, payload as usize + 4)) else { continue };
        let mut end = 0usize;
        let mut run = 0usize;
        for r in 0..nr.min(64) as usize {
            let rb = ro as usize + r * rsize;
            let Some(count) = rd(rts, rb + 4) else { break };
            let Some(offset) = rd(rts, rb + rsize - 4) else { break };
            let count = if count == u32::MAX { TABLE_FALLBACK } else { (count as usize).min(TABLE_CAP) };
            let start = if offset == u32::MAX { run } else { offset as usize };
            run = start + count;
            end = end.max(run);
        }
        out.insert(p as u32, end.min(TABLE_CAP));
    }
    out
}

#[derive(Clone, Copy, PartialEq)]
enum Use {
    Read,
    Write,
}

fn collect(s: &ListState, compute: bool, ds_write: bool) -> Vec<(usize, Option<(u64, u64)>, Use, String)> {
    let mut out: Vec<(usize, Option<(u64, u64)>, Use, String)> = Vec::new();
    let mut add = |res: usize, win: Option<(u64, u64)>, u: Use, why: String| {
        if res == 0 {
            return;
        }
        if let Some(e) = out.iter_mut().find(|e| e.0 == res) {
            if u == Use::Write {
                e.2 = Use::Write;
            }
            return;
        }
        out.push((res, win, u, why));
    };
    let add_va = |va: u64, u: Use, why: String, add: &mut dyn FnMut(usize, Option<(u64, u64)>, Use, String)| {
        if va == 0 {
            return;
        }
        if let Some((base, res, _size, _)) = crate::bindtrace::buffer_at(va) {
            add(res, Some((base, va)), u, why);
        }
    };
    let (root, tables, vas) = if compute { (s.croot, &s.ctables, &s.cvas) } else { (s.groot, &s.gtables, &s.gvas) };
    let sizes = crate::replay::root_blob(root).map(|b| table_sizes(&b)).unwrap_or_default();
    let heaps = crate::bindtrace::heaps();
    for (&idx, &gpu) in tables {
        let n = sizes.get(&idx).copied().unwrap_or(TABLE_FALLBACK);
        let Some(&(hcpu, hgpu, _, inc)) = heaps.iter().find(|h| h.3 != 0 && gpu >= h.1 && gpu < h.1 + (h.2 * h.3) as u64) else { continue };
        let cpu = hcpu + (gpu - hgpu) as usize;
        for (h, d) in crate::bindtrace::descs_in(cpu, n * inc) {
            let slot = (h - cpu) / inc;
            match d {
                crate::bindtrace::Desc::Srv { res, .. } => add(res, None, Use::Read, format!("table{idx}[{slot}] srv")),
                crate::bindtrace::Desc::Uav { res, .. } => add(res, None, Use::Write, format!("table{idx}[{slot}] uav")),
                crate::bindtrace::Desc::Cbv { va, .. } => add_va(va, Use::Read, format!("table{idx}[{slot}] cbv"), &mut add),
                crate::bindtrace::Desc::Sampler { .. } => {}
            }
        }
    }
    for (&idx, &(k, va)) in vas {
        add_va(va, if k == 'u' { Use::Write } else { Use::Read }, format!("root{idx} {k}"), &mut add);
    }
    if !compute {
        for (&slot, &va) in &s.vbs {
            add_va(va, Use::Read, format!("vb{slot}"), &mut add);
        }
        add_va(s.ib, Use::Read, "ib".into(), &mut add);
        for (i, &h) in s.rtvs.iter().enumerate() {
            if let Some(r) = crate::replay::view_res(h) {
                add(r, None, Use::Write, format!("rt{i}"));
            }
        }
        if s.dsv != 0 {
            if let Some(r) = crate::replay::view_res(s.dsv) {
                add(r, None, if ds_write { Use::Write } else { Use::Read }, "dsv".into());
            }
        }
    }
    out
}

enum Pending {
    Gpu { id: usize, buf: usize, size: u64, fps: Option<Vec<[u8; 32]>> },
    Cpu { id: usize, data: Vec<u8> },
}

static PENDING: Mutex<Vec<Pending>> = Mutex::new(Vec::new());
static NEXT_ID: AtomicUsize = AtomicUsize::new(0);
static CALLS: AtomicUsize = AtomicUsize::new(0);
static BYTES: AtomicU64 = AtomicU64::new(0);
static DEVICE: AtomicUsize = AtomicUsize::new(0);

unsafe fn device_of(list: *mut c_void) -> *mut c_void {
    let d = DEVICE.load(Ordering::Relaxed);
    if d != 0 {
        return d as *mut c_void;
    }
    let mut dev: *mut c_void = core::ptr::null_mut();
    if vcall!(list, 7, fn(*const Guid, *mut *mut c_void) -> i32, &IID_DEVICE, &mut dev) < 0 || dev.is_null() {
        return core::ptr::null_mut();
    }
    DEVICE.store(dev as usize, Ordering::Relaxed);
    dev
}

unsafe fn readback_buffer(dev: *mut c_void, size: u64) -> *mut c_void {
    let heap: [u32; 5] = [3, 0, 0, 1, 1];
    let mut bd = [0u8; 0x38];
    bd[0..4].copy_from_slice(&1u32.to_le_bytes());
    bd[0x10..0x18].copy_from_slice(&size.to_le_bytes());
    bd[0x18..0x1c].copy_from_slice(&1u32.to_le_bytes());
    bd[0x1c..0x1e].copy_from_slice(&1u16.to_le_bytes());
    bd[0x1e..0x20].copy_from_slice(&1u16.to_le_bytes());
    bd[0x24..0x28].copy_from_slice(&1u32.to_le_bytes());
    bd[0x2c..0x30].copy_from_slice(&1u32.to_le_bytes());
    let mut buf: *mut c_void = core::ptr::null_mut();
    let hr = vcall!(dev, 27, fn(*const u32, u32, *const u8, u32, *const c_void, *const Guid, *mut *mut c_void) -> i32,
        heap.as_ptr(), 0, bd.as_ptr(), COPY_DEST, core::ptr::null(), &IID_RESOURCE, &mut buf);
    if hr < 0 { core::ptr::null_mut() } else { buf }
}

unsafe fn res_desc(res: usize) -> [u8; 0x38] {
    let mut d = [0u8; 0x38];
    vcall!(res as *mut c_void, 10, fn(*mut u8) -> *mut u8, d.as_mut_ptr());
    d
}

unsafe fn heap_type(res: usize) -> u32 {
    let mut props = [0u32; 5];
    let mut flags = 0u32;
    if vcall!(res as *mut c_void, 14, fn(*mut u32, *mut u32) -> i32, props.as_mut_ptr(), &mut flags) < 0 {
        return 0;
    }
    props[0]
}

unsafe fn barrier(list: *mut c_void, res: usize, before: u32, after: u32) {
    let b = [0u64, res as u64, 0xffff_ffffu64 | ((before as u64) << 32), after as u64];
    vcall!(list, 26, fn(u32, *const u64) -> (), 1, b.as_ptr());
}

unsafe fn snap_one(list: *mut c_void, res: usize, win: Option<(u64, u64)>) -> Result<(usize, String), String> {
    let desc = res_desc(res);
    let dim = u32::from_le_bytes(desc[0..4].try_into().unwrap());
    let ht = heap_type(res);
    if ht == 3 {
        return Err("readback heap".into());
    }
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let file = format!("data/snap_{id}.bin");
    if dim == 1 {
        let size = u64::from_le_bytes(desc[0x10..0x18].try_into().unwrap());
        let (off, n) = match win {
            Some((base, va)) if size > SNAP_ONE => {
                let o = va - base;
                (o, (size - o).min(1 << 20))
            }
            _ => (0, size),
        };
        if n == 0 || n > SNAP_ONE {
            return Err(format!("buffer too large ({size} bytes)"));
        }
        if BYTES.fetch_add(n, Ordering::Relaxed) + n > SNAP_TOTAL {
            return Err("snapshot budget used".into());
        }
        if ht == 2 {
            let mut p: *mut u8 = core::ptr::null_mut();
            if vcall!(res as *mut c_void, 8, fn(u32, *const c_void, *mut *mut u8) -> i32, 0, core::ptr::null(), &mut p) < 0 || p.is_null() {
                return Err("upload map failed".into());
            }
            let data = core::slice::from_raw_parts(p.add(off as usize), n as usize).to_vec();
            vcall!(res as *mut c_void, 9, fn(u32, *const c_void) -> (), 0, core::ptr::null());
            PENDING.lock().unwrap_or_else(|e| e.into_inner()).push(Pending::Cpu { id, data });
            return Ok((id, format!("kind=buf off={off:#x} size={n} src=cpu file={file}")));
        }
        let dev = device_of(list);
        if dev.is_null() {
            return Err("no device".into());
        }
        let buf = readback_buffer(dev, n);
        if buf.is_null() {
            return Err("readback alloc failed".into());
        }
        let st = crate::replay::tracked_state(res);
        let needs = matches!(st, Some(s) if s & COPY_SOURCE == 0 || s == 0);
        if needs {
            barrier(list, res, st.unwrap(), COPY_SOURCE);
        }
        vcall!(list, 15, fn(*mut c_void, u64, *mut c_void, u64, u64) -> (), buf, 0, res as *mut c_void, off, n);
        if needs {
            barrier(list, res, COPY_SOURCE, st.unwrap());
        }
        PENDING.lock().unwrap_or_else(|e| e.into_inner()).push(Pending::Gpu { id, buf: buf as usize, size: n, fps: None });
        return Ok((id, format!("kind=buf off={off:#x} size={n} state={} file={file}", st.map(|s| format!("{s:#x}")).unwrap_or_else(|| "unknown".into()))));
    }
    if !(2..=4).contains(&dim) {
        return Err(format!("dimension {dim}"));
    }
    if u32::from_le_bytes(desc[0x24..0x28].try_into().unwrap()) != 1 {
        return Err("multisampled".into());
    }
    let Some(st) = crate::replay::tracked_state(res) else { return Err("texture state unknown".into()) };
    let dev = device_of(list);
    if dev.is_null() {
        return Err("no device".into());
    }
    let mips = u16::from_le_bytes([desc[0x1e], desc[0x1f]]).max(1) as u32;
    let arr = if dim == 4 { 1 } else { u16::from_le_bytes([desc[0x1c], desc[0x1d]]).max(1) as u32 };
    let subs = mips * arr;
    if subs > 1 && crate::replay::mixed_state(res) {
        return Err("per_subresource_state".into());
    }
    let mut fps = vec![[0u8; 32]; subs as usize];
    let mut total = 0u64;
    vcall!(dev, 38, fn(*const u8, u32, u32, u64, *mut [u8; 32], *mut u32, *mut u64, *mut u64) -> (),
        desc.as_ptr(), 0, subs, 0, fps.as_mut_ptr(), core::ptr::null_mut(), core::ptr::null_mut(), &mut total);
    if total == 0 || total == u64::MAX || total > SNAP_ONE {
        return Err(format!("texture too large ({total} bytes)"));
    }
    if BYTES.fetch_add(total, Ordering::Relaxed) + total > SNAP_TOTAL {
        return Err("snapshot budget used".into());
    }
    let buf = readback_buffer(dev, total);
    if buf.is_null() {
        return Err("readback alloc failed".into());
    }
    let needs = st & COPY_SOURCE == 0 || st == 0;
    if needs {
        barrier(list, res, st, COPY_SOURCE);
    }
    for (i, fp) in fps.iter().enumerate() {
        let mut dst = [0u8; 48];
        dst[0..8].copy_from_slice(&(buf as u64).to_le_bytes());
        dst[8..12].copy_from_slice(&1u32.to_le_bytes());
        dst[16..48].copy_from_slice(fp);
        let mut src = [0u8; 48];
        src[0..8].copy_from_slice(&(res as u64).to_le_bytes());
        src[16..20].copy_from_slice(&(i as u32).to_le_bytes());
        vcall!(list, 16, fn(*const u8, u32, u32, u32, *const u8, *const c_void) -> (), dst.as_ptr(), 0, 0, 0, src.as_ptr(), core::ptr::null());
    }
    if needs {
        barrier(list, res, COPY_SOURCE, st);
    }
    PENDING.lock().unwrap_or_else(|e| e.into_inner()).push(Pending::Gpu { id, buf: buf as usize, size: total, fps: Some(fps) });
    Ok((id, format!("kind=tex subs={subs} size={total} state={st:#x} file={file}")))
}

unsafe fn snap_call(list: *mut c_void, compute: bool, pre: bool) -> bool {
    let s = with_list(list, |s| s.clone());
    if s.pso == 0 {
        return false;
    }
    let info = pso_info(s.pso);
    let f = filter();
    let Some(&(_, key)) = info.hashes.iter().find(|(_, h)| f.contains(h)) else { return false };
    if pre {
        if CALLS.fetch_add(1, Ordering::Relaxed) >= SNAP_CALLS {
            return false;
        }
        let mut g = PER_HASH.lock().unwrap_or_else(|e| e.into_inner());
        let c = g.get_or_insert_with(HashMap::new).entry(key).or_insert(0);
        if *c >= SNAP_PER_HASH {
            return false;
        }
        *c += 1;
    }
    let tag = if pre { "pre" } else { "post" };
    let hs: Vec<String> = info.hashes.iter().map(|(n, h)| format!("{n}={h:016x}")).collect();
    push_line(list, format!("snapcall {tag} pso={:#x} {}", s.pso, hs.join(" ")));
    for (res, win, u, why) in collect(&s, compute, info.ds_write) {
        if !pre && u != Use::Write {
            continue;
        }
        note_resource(res);
        let k = (list as usize, res);
        if pre {
            let hit = SNAPPED.lock().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(|m| m.get(&k).copied());
            if let Some((id, ep)) = hit {
                if ep == s.epoch {
                    push_line(list, format!("snap {tag} res={res:#x} use={why} ref={id}"));
                    continue;
                }
            }
        }
        match snap_one(list, res, win) {
            Ok((id, info)) => {
                SNAPPED.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).insert(k, (id, s.epoch));
                push_line(list, format!("snap {tag} res={res:#x} use={why} id={id} {info}"))
            }
            Err(e) => {
                if !pre {
                    if let Some(m) = SNAPPED.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
                        m.remove(&k);
                    }
                }
                push_line(list, format!("snapskip {tag} res={res:#x} use={why} reason={}", e.replace(' ', "_")))
            }
        }
    }
    true
}

pub unsafe fn flush(queue: *mut c_void, dir: &std::path::Path, log: &mut String) {
    let pending: Vec<Pending> = core::mem::take(&mut *PENDING.lock().unwrap_or_else(|e| e.into_inner()));
    CALLS.store(0, Ordering::Relaxed);
    BYTES.store(0, Ordering::Relaxed);
    *LISTS.lock().unwrap_or_else(|e| e.into_inner()) = None;
    *PSO_INFO.lock().unwrap_or_else(|e| e.into_inner()) = None;
    *SNAPPED.lock().unwrap_or_else(|e| e.into_inner()) = None;
    *PER_HASH.lock().unwrap_or_else(|e| e.into_inner()) = None;
    let mut queues: Vec<usize> = core::mem::take(&mut *QUEUES.lock().unwrap_or_else(|e| e.into_inner()));
    if !queue.is_null() && !queues.contains(&(queue as usize)) {
        queues.push(queue as usize);
    }
    if pending.is_empty() {
        return;
    }
    let mut waited = !queues.is_empty();
    for q in queues {
        let q = q as *mut c_void;
        let mut ok = false;
        let mut dev: *mut c_void = core::ptr::null_mut();
        if vcall!(q, 7, fn(*const Guid, *mut *mut c_void) -> i32, &IID_DEVICE, &mut dev) >= 0 && !dev.is_null() {
            let mut fence: *mut c_void = core::ptr::null_mut();
            if vcall!(dev, 36, fn(u64, u32, *const Guid, *mut *mut c_void) -> i32, 0, 0, &IID_FENCE, &mut fence) >= 0 && !fence.is_null() {
                vcall!(q, 14, fn(*mut c_void, u64) -> i32, fence, 1);
                let mut spins = 0;
                while vcall!(fence, 8, fn() -> u64) < 1 && spins < 10_000 {
                    Sleep(1);
                    spins += 1;
                }
                ok = spins < 10_000;
                vcall!(fence, 2, fn() -> u32);
            }
            vcall!(dev, 2, fn() -> u32);
        }
        waited &= ok;
    }
    let _ = std::fs::create_dir_all(dir.join("data"));
    let (mut ok, mut bytes_written) = (0usize, 0u64);
    for p in pending {
        match p {
            Pending::Cpu { id, data } => {
                if std::fs::write(dir.join(format!("data/snap_{id}.bin")), &data).is_ok() {
                    ok += 1;
                    bytes_written += data.len() as u64;
                }
            }
            Pending::Gpu { id, buf, size, fps } => {
                let b = buf as *mut c_void;
                let mut data: *mut u8 = core::ptr::null_mut();
                if waited && vcall!(b, 8, fn(u32, *const c_void, *mut *mut u8) -> i32, 0, core::ptr::null(), &mut data) >= 0 && !data.is_null() {
                    let raw = core::slice::from_raw_parts(data, size as usize);
                    let mut out = Vec::with_capacity(raw.len() + 64);
                    if let Some(fps) = &fps {
                        out.extend_from_slice(b"TBTX");
                        out.extend_from_slice(&(fps.len() as u32).to_le_bytes());
                        for fp in fps {
                            out.extend_from_slice(fp);
                        }
                    }
                    out.extend_from_slice(raw);
                    vcall!(b, 9, fn(u32, *const c_void) -> (), 0, core::ptr::null());
                    if std::fs::write(dir.join(format!("data/snap_{id}.bin")), &out).is_ok() {
                        ok += 1;
                        bytes_written += size;
                    }
                }
                if waited {
                    vcall!(b, 2, fn() -> u32);
                }
            }
        }
    }
    let _ = writeln!(log, "snapshots written={ok} bytes={bytes_written} gpu_wait={waited}");
    record("d3d12_xs", "replay", &format!("draw-time snapshots: {ok} files, {bytes_written} bytes"));
}

pub unsafe fn install(xbox_vt: &[AtomicUsize]) {
    if !crate::rtcap::enabled() {
        return;
    }
    let hooks: [(usize, usize); 36] = [
        (28, h_begin_query as usize), (29, h_end_query as usize), (30, h_resolve_query as usize), (31, h_predication as usize),
        (55, h_discard as usize), (68, h_depth_bounds as usize), (69, h_fill as usize), (83, h_copy_texture_x as usize),
        (84, h_copy_resource_x as usize), (10, h_reset as usize), (25, h_set_pso as usize), (33, h_croot as usize),
        (34, h_groot as usize), (35, h_ctable as usize), (36, h_gtable as usize), (41, h_ccbv as usize),
        (42, h_gcbv as usize), (43, h_csrv as usize), (44, h_gsrv as usize), (45, h_cuav as usize),
        (46, h_guav as usize), (47, h_ib as usize), (48, h_vbs as usize), (50, h_targets as usize),
        (12, h_draw as usize), (13, h_draw_indexed as usize), (14, h_dispatch as usize), (15, b_copy_buffer as usize),
        (16, b_copy_texture as usize), (17, b_copy_resource as usize), (51, b_clear_dsv as usize), (52, b_clear_rtv as usize),
        (53, b_clear_uavu as usize), (54, b_clear_uavf as usize), (59, b_indirect as usize), (0, 0),
    ];
    let mut n = 0;
    for (slot, f) in hooks {
        if f == 0 {
            continue;
        }
        PREV[slot].store(xbox_vt[slot].load(Ordering::Relaxed), Ordering::Relaxed);
        xbox_vt[slot].store(f, Ordering::Relaxed);
        n += 1;
    }
    record("d3d12_xs", "replay", &format!("recorder extension on {n} methods (queries, depth bounds, fill, Xbox copies, draw-time snapshots for record_filter)"));
}

#[cfg(test)]
mod tests {
    #[test]
    fn root_table_sizes_from_rts0() {
        let mut r = Vec::new();
        let w = |v: &mut Vec<u8>, x: u32| v.extend_from_slice(&x.to_le_bytes());
        w(&mut r, 2);
        w(&mut r, 2);
        w(&mut r, 24);
        w(&mut r, 0);
        w(&mut r, 0);
        w(&mut r, 0);
        w(&mut r, 0);
        w(&mut r, 0);
        w(&mut r, 48);
        w(&mut r, 2);
        w(&mut r, 0);
        w(&mut r, 0);
        w(&mut r, 2);
        w(&mut r, 56);
        for (ty, n, off) in [(0u32, 4u32, 0u32), (1, 3, u32::MAX)] {
            w(&mut r, ty);
            w(&mut r, n);
            w(&mut r, 0);
            w(&mut r, 0);
            w(&mut r, 0);
            w(&mut r, off);
        }
        let s = super::table_sizes(&r);
        assert_eq!(s.get(&0), Some(&7));
        assert_eq!(s.get(&1), None);
    }
}
