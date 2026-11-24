use core::ffi::c_void;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;
use compat_common::record;

use crate::device_wrap::Guid;

const IID_DEVICE: Guid =
    Guid { d1: 0x1898_19F1, d2: 0x1DB6, d3: 0x4B57, d4: [0xBE, 0x54, 0x18, 0x21, 0x33, 0x9B, 0x85, 0xF7] };
const IID_QUERY_HEAP: Guid =
    Guid { d1: 0x0d96_58ae, d2: 0xed45, d3: 0x469e, d4: [0xa6, 0x1d, 0x97, 0x0e, 0xc5, 0x83, 0xca, 0xb4] };
const IID_RESOURCE: Guid =
    Guid { d1: 0x6964_42be, d2: 0xa72e, d3: 0x4059, d4: [0xbc, 0x79, 0x5b, 0x5c, 0x98, 0x04, 0x0f, 0xad] };
const SLOTS: u32 = 1 << 17;
const FRAMES: u64 = 16;
const SETTLE: u64 = 8;
const PERIOD_SECS: u64 = 20;
const QUERY_TIMESTAMP: u32 = 2;

#[derive(Clone, Copy)]
struct Label {
    a: u64,
    b: u64,
    compute: bool,
}

#[derive(Clone, Copy)]
struct Ev {
    slot: u32,
    label: Label,
    kind: u8,
    count: bool,
    frame: u32,
}

#[derive(Default)]
struct ListRec {
    label: Option<Label>,
    evs: Vec<Ev>,
    last_kind: u8,
}

pub const TAG_BLENDCLAMP: u8 = 1;
pub const TAG_CBV_COPY: u8 = 2;
pub const TAG_RESOLVE: u8 = 3;
pub const TAG_UPLOAD: u8 = 4;
pub const TAG_PRESENT: u8 = 5;
pub const TAG_INJECT0: u8 = 16;
const INJECT_NAMES: [&str; 7] = ["inject:detile", "inject:copy", "inject:clear", "inject:resolve", "inject:barrier", "inject:draw", "inject:query"];
const KIND_PASS: u8 = 16;
const KIND_CLOSE: u8 = 255;

fn tag_name(tag: u64) -> &'static str {
    match tag as u8 {
        TAG_BLENDCLAMP => "blendclamp",
        TAG_CBV_COPY => "cbv_copy",
        TAG_RESOLVE => "resolve",
        TAG_UPLOAD => "upload_pump",
        TAG_PRESENT => "present_copy",
        t if (TAG_INJECT0..TAG_INJECT0 + 7).contains(&t) => INJECT_NAMES[(t - TAG_INJECT0) as usize],
        _ => "emu?",
    }
}

fn stampable(ty: u32) -> bool {
    ty == 0 || ty == 2 || ty == 3
}

#[derive(Clone, Copy)]
struct Span {
    ty: u32,
    s0: u32,
    s1: u32,
}

const IID_ALLOCATOR: Guid = Guid { d1: 0x6102_dee4, d2: 0xaf59, d3: 0x4b09, d4: [0xb9, 0x99, 0xb4, 0x4d, 0x73, 0xf0, 0x9b, 0x24] };
const IID_FENCE: Guid = Guid { d1: 0x0a75_3dcf, d2: 0xc4d8, d3: 0x4b91, d4: [0xad, 0xf6, 0xbe, 0x5a, 0x60, 0xd9, 0x5a, 0x76] };
const IID_LIST: Guid = Guid { d1: 0x5b16_0d0f, d2: 0xac1b, d3: 0x4185, d4: [0x8b, 0xa8, 0xb3, 0xae, 0x42, 0xa5, 0xa4, 0x55] };
type ExecFn = unsafe extern "system" fn(*mut c_void, u32, *const *mut c_void);

struct SubRec {
    ty: u32,
    queue: usize,
    s0: u32,
    s1: u32,
    lists: Vec<(u32, u32)>,
    n_lists: u32,
    missing: u32,
    cpu: [i64; 4],
}

struct State {
    closed: HashMap<usize, (u32, u32, bool)>,
    subs: Vec<SubRec>,
    sub_open: HashMap<usize, SubRec>,
    calib: HashMap<usize, (u32, Vec<crate::gpuprof_tl::Calib>)>,
    heap: usize,
    cheap: usize,
    rb: usize,
    open: HashMap<usize, (u32, u32)>,
    spans: Vec<Span>,
    retire: Vec<(usize, usize, u64, u32)>,
    done_ty: Vec<u32>,
    fence: usize,
    fence_val: u64,
    lists: HashMap<usize, ListRec>,
    labels: HashMap<usize, Label>,
    done: Vec<Vec<Ev>>,
    presents: u64,
    window: Option<u64>,
    win_start: u64,
    finalize_at: Option<u64>,
    last: Option<Instant>,
    reports: u32,
}

static NEXT: [AtomicUsize; 128] = [const { AtomicUsize::new(0) }; 128];
static ARMED: AtomicBool = AtomicBool::new(false);
static SLOT: AtomicU32 = AtomicU32::new(0);
static FRAME_IX: AtomicU32 = AtomicU32::new(0);

fn env_u64(name: &str) -> Option<u64> {
    std::env::var(name).ok().and_then(|v| v.trim().parse().ok())
}

fn frames() -> u64 {
    static F: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    *F.get_or_init(|| env_u64("TOMBOX_GPUPROF_FRAMES").unwrap_or(FRAMES).clamp(1, 240))
}

fn period_ms() -> u64 {
    static P: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    *P.get_or_init(|| env_u64("TOMBOX_GPUPROF_PERIOD_MS").unwrap_or(PERIOD_SECS * 1000).max(250))
}

fn worst_frame(per_frame: &HashMap<u32, u64>) -> Option<(u32, u64, u64)> {
    if per_frame.is_empty() {
        return None;
    }
    let mut v: Vec<u64> = per_frame.values().copied().collect();
    v.sort_unstable();
    let median = v[v.len() / 2];
    let (&f, &w) = per_frame.iter().max_by_key(|(f, t)| (**t, core::cmp::Reverse(**f)))?;
    Some((f, w, median))
}
static STATE: Mutex<Option<State>> = Mutex::new(None);

static LIVE: crate::live_sw::Armed = crate::live_sw::Armed::new("TOMBOX_GPUPROF", "gpuprof", false);

fn cfg_default() -> bool {
    compat_common::cfg("gpuprof").as_deref() == Some("1")
}

pub fn enabled() -> bool {
    LIVE.armed_or(cfg_default)
}

fn windows_on() -> bool {
    LIVE.on_or(cfg_default)
}

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut g = STATE.lock().unwrap_or_else(|e| e.into_inner());
    let s = g.get_or_insert_with(|| State {
        closed: HashMap::new(),
        subs: Vec::new(),
        sub_open: HashMap::new(),
        calib: HashMap::new(),
        heap: 0,
        cheap: 0,
        rb: 0,
        open: HashMap::new(),
        spans: Vec::new(),
        retire: Vec::new(),
        done_ty: Vec::new(),
        fence: 0,
        fence_val: 0,
        lists: HashMap::new(),
        labels: HashMap::new(),
        done: Vec::new(),
        presents: 0,
        window: None,
        win_start: 0,
        finalize_at: None,
        last: None,
        reports: 0,
    });
    f(s)
}

fn next(slot: usize) -> usize {
    NEXT[slot].load(Ordering::Relaxed)
}

unsafe fn real(slot: usize) -> usize {
    unsafe { crate::cmdlist_wrap::real_method(slot) }
}

pub unsafe fn install() {
    if !enabled() {
        return;
    }
    let hooks: [(usize, usize); 7] = [
        (9, h_close as usize), (10, h_reset as usize), (12, h_draw as usize), (13, h_draw_indexed as usize),
        (14, h_dispatch as usize), (25, h_set_pso as usize), (59, h_indirect as usize),
    ];
    for (slot, _) in hooks {
        let prev = crate::cmdlist_wrap::vt_get(slot);
        if prev == 0 {
            record("d3d12_xs", "gpuprof", &format!("command-list slot {slot} empty - profiler not installed"));
            return;
        }
        NEXT[slot].store(prev, Ordering::Relaxed);
    }
    for (slot, f) in hooks {
        crate::cmdlist_wrap::vt_set(slot, f);
    }
    record("d3d12_xs", "gpuprof", &format!("installed: {}-frame GPU timestamp windows every {} ms (TOMBOX_GPUPROF_FRAMES / TOMBOX_GPUPROF_PERIOD_MS); each window also reports its slowest frame against the median", frames(), period_ms()));
}

fn label_of(pso: usize) -> Label {
    let codes = crate::device_x::pso_stage_codes(pso);
    let h = |name: &str| -> u64 {
        codes.iter().find(|(n, _)| *n == name)
            .and_then(|(_, c)| crate::shader_xlat::origin_hash(*c))
            .unwrap_or(0)
    };
    if codes.iter().any(|(n, _)| *n == "cs") {
        Label { a: h("cs"), b: 0, compute: true }
    } else {
        Label { a: h("vs"), b: h("ps"), compute: false }
    }
}

fn on_set_pso(list: usize, pso: usize) {
    with(|s| {
        let l = match s.labels.get(&pso) {
            Some(l) => *l,
            None => {
                let l = label_of(pso);
                s.labels.insert(pso, l);
                l
            }
        };
        s.lists.entry(list).or_default().label = Some(l);
    });
}

unsafe fn ensure_resources(s: &mut State, list: *mut c_void) -> bool {
    if s.heap != 0 && s.rb != 0 {
        return true;
    }
    if s.heap == usize::MAX {
        return false;
    }
    let get: unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> i32 = unsafe { core::mem::transmute(real(7)) };
    let mut dev: *mut c_void = core::ptr::null_mut();
    if unsafe { get(list, &IID_DEVICE, &mut dev) } < 0 || dev.is_null() {
        return false;
    }
    unsafe { ensure_on_device(s, dev) }
}

unsafe fn device_of(obj: *mut c_void) -> *mut c_void {
    let get: unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> i32 =
        unsafe { core::mem::transmute(*(*(obj as *const *const usize)).add(7)) };
    let mut dev: *mut c_void = core::ptr::null_mut();
    if unsafe { get(obj, &IID_DEVICE, &mut dev) } < 0 {
        return core::ptr::null_mut();
    }
    dev
}

unsafe fn ensure_on_device(s: &mut State, dev: *mut c_void) -> bool {
    if s.heap != 0 && s.rb != 0 {
        let release: unsafe extern "system" fn(*mut c_void) -> u32 = unsafe { core::mem::transmute(*(*(dev as *const *const usize)).add(2)) };
        unsafe { release(dev) };
        return true;
    }
    if s.heap == usize::MAX {
        let release: unsafe extern "system" fn(*mut c_void) -> u32 = unsafe { core::mem::transmute(*(*(dev as *const *const usize)).add(2)) };
        unsafe { release(dev) };
        return false;
    }
    let vt = unsafe { *(dev as *const *const usize) };
    let qdesc: [u32; 3] = [1, SLOTS, 0];
    let create_qh: unsafe extern "system" fn(*mut c_void, *const u32, *const Guid, *mut *mut c_void) -> i32 =
        unsafe { core::mem::transmute(*vt.add(39)) };
    let mut heap: *mut c_void = core::ptr::null_mut();
    let hr_h = unsafe { create_qh(dev, qdesc.as_ptr(), &IID_QUERY_HEAP, &mut heap) };
    let cdesc: [u32; 3] = [5, SLOTS, 0];
    let mut cheap: *mut c_void = core::ptr::null_mut();
    let hr_c = unsafe { create_qh(dev, cdesc.as_ptr(), &IID_QUERY_HEAP, &mut cheap) };
    if hr_c < 0 || cheap.is_null() {
        record("d3d12_xs", "gpuprof", &format!("copy-queue timestamp heap unavailable hr={hr_c:#010x}: copy lists and copy-queue submissions stay untimed"));
        s.cheap = usize::MAX;
    } else {
        s.cheap = cheap as usize;
    }
    let props: [u32; 5] = [3, 0, 0, 1, 1];
    let mut bd = [0u8; 0x38];
    bd[0..4].copy_from_slice(&1u32.to_le_bytes());
    bd[0x10..0x18].copy_from_slice(&(SLOTS as u64 * 8).to_le_bytes());
    bd[0x18..0x1c].copy_from_slice(&1u32.to_le_bytes());
    bd[0x1c..0x1e].copy_from_slice(&1u16.to_le_bytes());
    bd[0x1e..0x20].copy_from_slice(&1u16.to_le_bytes());
    bd[0x24..0x28].copy_from_slice(&1u32.to_le_bytes());
    bd[0x2c..0x30].copy_from_slice(&1u32.to_le_bytes());
    let create_cr: unsafe extern "system" fn(*mut c_void, *const u32, u32, *const u8, u32, *const c_void, *const Guid, *mut *mut c_void) -> i32 =
        unsafe { core::mem::transmute(*vt.add(27)) };
    let mut rb: *mut c_void = core::ptr::null_mut();
    let hr_r = unsafe { create_cr(dev, props.as_ptr(), 0, bd.as_ptr(), 0x400, core::ptr::null(), &IID_RESOURCE, &mut rb) };
    let release: unsafe extern "system" fn(*mut c_void) -> u32 = unsafe { core::mem::transmute(*vt.add(2)) };
    unsafe { release(dev) };
    if hr_h < 0 || hr_r < 0 || heap.is_null() || rb.is_null() {
        record("d3d12_xs", "gpuprof", &format!("resource creation FAILED heap hr={hr_h:#010x} readback hr={hr_r:#010x}; profiler disabled"));
        s.heap = usize::MAX;
        return false;
    }
    s.heap = heap as usize;
    s.rb = rb as usize;
    true
}

unsafe fn stamp(list: *mut c_void, kind: u8) -> Option<u32> {
    unsafe { stamp_ev(list, None, kind, true) }
}

fn heap_for(s: &State, ty: u32) -> usize {
    let h = if ty == 3 { s.cheap } else { s.heap };
    if h == usize::MAX { 0 } else { h }
}

static ARGS: Mutex<Option<HashMap<u32, [u32; 5]>>> = Mutex::new(None);

fn note_args(list: *mut c_void, slot: Option<u32>, args: [u32; 5]) {
    let (Some(slot), Some(t)) = (slot, crate::passcap::ps_ms_target()) else { return };
    let hit = with(|s| s.lists.get(&(list as usize)).and_then(|r| r.label).is_some_and(|l| !l.compute && l.b == t));
    if hit {
        let mut g = ARGS.lock().unwrap_or_else(|e| e.into_inner());
        let m = g.get_or_insert_with(HashMap::new);
        if m.len() < 65536 {
            m.insert(slot, args);
        }
    }
}

fn args_text(kind: u8, a: Option<[u32; 5]>) -> String {
    match (kind, a) {
        (_, None) => String::new(),
        (0, Some(a)) => format!(" | draw vertices={} instances={} first_vertex={} first_instance={}", a[0], a[1], a[2], a[3]),
        (1, Some(a)) => format!(" | drawidx indices={} instances={} first_index={} base_vertex={} first_instance={}", a[0], a[1], a[2], a[3] as i32, a[4]),
        (3, Some(a)) => format!(" | indirect max_commands={} arg_offset={:#x} count_buffer={}", a[0], a[1], if a[2] != 0 { "yes" } else { "no" }),
        (_, Some(a)) => format!(" | args {a:?}"),
    }
}

unsafe fn stamp_ev(list: *mut c_void, tag: Option<u8>, kind: u8, count: bool) -> Option<u32> {
    if !ARMED.load(Ordering::Relaxed) {
        return None;
    }
    let ty = unsafe { crate::fill::list_type(list) };
    if !stampable(ty) {
        return None;
    }
    with(|s| {
        if !unsafe { ensure_resources(s, list) } {
            return None;
        }
        let heap = heap_for(s, ty);
        if heap == 0 {
            return None;
        }
        let slot = SLOT.fetch_add(1, Ordering::Relaxed);
        if slot >= SLOTS {
            return None;
        }
        let end: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, u32) = unsafe { core::mem::transmute(real(53)) };
        unsafe { end(list, heap as *mut c_void, QUERY_TIMESTAMP, slot) };
        let r = s.lists.entry(list as usize).or_default();
        let (label, kind) = match tag {
            Some(t) => (Label { a: t as u64, b: 0, compute: false }, KIND_PASS),
            None if count && kind >= KIND_OP0 => (Label { a: 0, b: 0, compute: false }, kind),
            None if count => {
                r.last_kind = kind;
                (r.label.unwrap_or(Label { a: 0, b: 0, compute: kind == 2 }), kind)
            }
            None => (r.label.unwrap_or(Label { a: 0, b: 0, compute: r.last_kind == 2 }), r.last_kind),
        };
        r.evs.push(Ev { slot, label, kind, count, frame: FRAME_IX.load(Ordering::Relaxed) });
        Some(slot)
    })
}

unsafe fn vt_at(obj: usize, i: usize) -> usize {
    unsafe { *(*(obj as *const *const usize)).add(i) }
}

unsafe fn release_obj(obj: usize) {
    if obj != 0 {
        let f: unsafe extern "system" fn(usize) -> u32 = unsafe { core::mem::transmute(vt_at(obj, 2)) };
        unsafe { f(obj) };
    }
}

const POOL_KEEP: usize = 64;

unsafe fn sweep_retired(s: &mut State) {
    if s.fence == 0 || s.retire.len() <= POOL_KEEP {
        return;
    }
    let get: unsafe extern "system" fn(usize) -> u64 = unsafe { core::mem::transmute(vt_at(s.fence, 8)) };
    let done = unsafe { get(s.fence) };
    let mut keep = Vec::new();
    for e in core::mem::take(&mut s.retire) {
        if e.2 <= done && keep.len() >= POOL_KEEP {
            unsafe { release_obj(e.1) };
            unsafe { release_obj(e.0) };
        } else {
            keep.push(e);
        }
    }
    s.retire = keep;
}

unsafe fn ecl_stamp(queue: *mut c_void, ty: u32) -> Option<(u32, usize, usize, u64)> {
    let dev = unsafe { device_of(queue) };
    if dev.is_null() {
        return None;
    }
    let ok = with(|s| unsafe { ensure_on_device(s, dev) });
    if !ok {
        return None;
    }
    let dev = unsafe { device_of(queue) };
    if dev.is_null() {
        return None;
    }
    let d = dev as usize;
    let out = unsafe { ecl_record(d, ty) };
    unsafe { release_obj(d) };
    out
}

unsafe fn ecl_record(d: usize, ty: u32) -> Option<(u32, usize, usize, u64)> {
    let (heap, rb, mut fence) = with(|s| (heap_for(s, ty), s.rb, s.fence));
    if heap == 0 || rb == 0 {
        return None;
    }
    if fence == 0 {
        let cf: unsafe extern "system" fn(usize, u64, u32, *const Guid, *mut usize) -> i32 = unsafe { core::mem::transmute(vt_at(d, 36)) };
        let mut f = 0usize;
        if unsafe { cf(d, 0, 0, &IID_FENCE, &mut f) } < 0 || f == 0 {
            return None;
        }
        let kept = with(|s| {
            if s.fence == 0 {
                s.fence = f;
            }
            s.fence
        });
        if kept != f {
            unsafe { release_obj(f) };
        }
        fence = kept;
    }
    let get: unsafe extern "system" fn(usize) -> u64 = unsafe { core::mem::transmute(vt_at(fence, 8)) };
    let reached = unsafe { get(fence) };
    let slot = SLOT.fetch_add(1, Ordering::Relaxed);
    if slot >= SLOTS {
        return None;
    }
    let reuse = with(|s| {
        let i = s.retire.iter().position(|e| e.2 <= reached && e.3 == ty)?;
        let e = s.retire.swap_remove(i);
        Some((e.0, e.1))
    });
    let (a, l) = match reuse {
        Some((a, l)) => {
            let ra: unsafe extern "system" fn(usize) -> i32 = unsafe { core::mem::transmute(vt_at(a, 8)) };
            let rl: unsafe extern "system" fn(usize, usize, usize) -> i32 = unsafe { core::mem::transmute(vt_at(l, 10)) };
            if unsafe { ra(a) } < 0 || unsafe { rl(l, a, 0) } < 0 {
                unsafe { release_obj(l) };
                unsafe { release_obj(a) };
                return None;
            }
            (a, l)
        }
        None => {
            let ca: unsafe extern "system" fn(usize, u32, *const Guid, *mut usize) -> i32 = unsafe { core::mem::transmute(vt_at(d, 9)) };
            let mut a = 0usize;
            if unsafe { ca(d, ty, &IID_ALLOCATOR, &mut a) } < 0 || a == 0 {
                return None;
            }
            let cl: unsafe extern "system" fn(usize, u32, u32, usize, usize, *const Guid, *mut usize) -> i32 = unsafe { core::mem::transmute(vt_at(d, 12)) };
            let mut l = 0usize;
            if unsafe { cl(d, 0, ty, a, 0, &IID_LIST, &mut l) } < 0 || l == 0 {
                unsafe { release_obj(a) };
                return None;
            }
            (a, l)
        }
    };
    let end: unsafe extern "system" fn(usize, usize, u32, u32) = unsafe { core::mem::transmute(vt_at(l, 53)) };
    let resolve: unsafe extern "system" fn(usize, usize, u32, u32, u32, usize, u64) = unsafe { core::mem::transmute(vt_at(l, 54)) };
    let close: unsafe extern "system" fn(usize) -> i32 = unsafe { core::mem::transmute(vt_at(l, 9)) };
    unsafe { end(l, heap, QUERY_TIMESTAMP, slot) };
    unsafe { resolve(l, heap, QUERY_TIMESTAMP, slot, 1, rb, slot as u64 * 8) };
    if unsafe { close(l) } < 0 {
        unsafe { release_obj(l) };
        unsafe { release_obj(a) };
        return None;
    }
    let v = with(|s| {
        s.fence_val += 1;
        let v = s.fence_val;
        s.retire.push((a, l, v, ty));
        v
    });
    Some((slot, l, fence, v))
}

unsafe fn submit_stamp(queue: *mut c_void, exec: ExecFn, ty: u32) -> Option<u32> {
    let (slot, l, fence, v) = unsafe { ecl_stamp(queue, ty) }?;
    let lists = [l as *mut c_void];
    unsafe { exec(queue, 1, lists.as_ptr()) };
    let sig = crate::gpuprof_q::real_signal();
    let sig = if sig != 0 { sig } else { crate::present::real_queue_slot(14) };
    if sig != 0 {
        let f: unsafe extern "system" fn(*mut c_void, usize, u64) -> i32 = unsafe { core::mem::transmute(sig) };
        unsafe { f(queue, fence, v) };
    }
    Some(slot)
}

pub unsafe fn ecl_begin(queue: *mut c_void, exec: ExecFn, lists: &[*mut c_void], enter: i64) {
    if !ARMED.load(Ordering::Relaxed) || queue.is_null() || !enabled() {
        return;
    }
    let ty = unsafe { crate::gpuprof_q::queue_type(queue) };
    let need = with(|s| !s.calib.contains_key(&(queue as usize)));
    if need {
        let c = unsafe { crate::gpuprof_tl::calibrate(queue) };
        with(|s| {
            s.calib.entry(queue as usize).or_insert((ty, Vec::new())).1.extend(c);
        });
    }
    let mut rec = with(|s| {
        let mut r = SubRec { ty, queue: queue as usize, s0: 0, s1: 0, lists: Vec::new(), n_lists: lists.len() as u32, missing: 0, cpu: [enter, 0, 0, 0] };
        for l in lists {
            match s.closed.get(&(*l as usize)) {
                Some(&(a, b, true)) => r.lists.push((a, b)),
                _ => r.missing += 1,
            }
        }
        r
    });
    rec.cpu[1] = crate::frame::qpc();
    if let Some(s0) = unsafe { submit_stamp(queue, exec, ty) } {
        rec.s0 = s0;
        with(|s| {
            s.open.insert(queue as usize, (ty, s0));
            s.sub_open.insert(queue as usize, rec);
        });
    }
}

pub fn ecl_exit(queue: *mut c_void) {
    if !enabled() {
        return;
    }
    let t = crate::frame::qpc();
    with(|s| {
        if let Some(r) = s.subs.iter_mut().rev().find(|r| r.queue == queue as usize) {
            if r.cpu[3] == 0 {
                r.cpu[3] = t;
            }
        }
    });
}

pub unsafe fn ecl_end(queue: *mut c_void, exec: ExecFn) {
    if !enabled() {
        return;
    }
    let post = crate::frame::qpc();
    let open = with(|s| (s.open.remove(&(queue as usize)), s.sub_open.remove(&(queue as usize))));
    let (Some((ty, s0)), rec) = open else { return };
    if !ARMED.load(Ordering::Relaxed) {
        return;
    }
    if let Some(s1) = unsafe { submit_stamp(queue, exec, ty) } {
        with(|s| {
            s.spans.push(Span { ty, s0, s1 });
            if let Some(mut r) = rec {
                r.s1 = s1;
                r.cpu[2] = post;
                s.subs.push(r);
            }
        });
    }
}


pub const KIND_OP0: u8 = 20;
const OPS: [(usize, &str); 12] = [
    (15, "CopyBufferRegion"), (16, "CopyTextureRegion"), (17, "CopyResource"), (18, "CopyTiles"), (19, "ResolveSubresource"),
    (26, "ResourceBarrier"), (52, "ClearRenderTargetView"), (53, "ClearUnorderedAccessViewUint"), (54, "ClearUnorderedAccessViewFloat"),
    (55, "DiscardResource"), (83, "CopyTextureRegionX"), (51, "ClearDepthStencilView"),
];
const KIND_EMU_BARRIER: u8 = KIND_OP0 + 12;
static OP_NEXT: [AtomicUsize; 12] = [const { AtomicUsize::new(0) }; 12];
static REAL_BARRIER: AtomicUsize = AtomicUsize::new(0);
static EMU_BARRIER_CALLS: AtomicU32 = AtomicU32::new(0);
static EMU_BARRIER_COUNT: AtomicU32 = AtomicU32::new(0);
static INJ: Mutex<([u64; crate::inject::KINDS], [u64; crate::inject::KINDS])> = Mutex::new(([0; crate::inject::KINDS], [0; crate::inject::KINDS]));

pub fn inject_text(a: &[u64; crate::inject::KINDS], b: &[u64; crate::inject::KINDS], nf: u64) -> String {
    let v: Vec<String> = (0..crate::inject::KINDS).filter(|&k| b[k] > a[k]).map(|k| format!("{} {:.1}/f", crate::inject::NAMES[k], (b[k] - a[k]) as f64 / nf.max(1) as f64)).collect();
    format!("emulator-injected commands in the window: {}", if v.is_empty() { "none".to_string() } else { v.join(", ") })
}

thread_local! {
    static IN_TITLE_BARRIER: core::cell::Cell<bool> = const { core::cell::Cell::new(false) };
}

pub fn op_name(kind: u8) -> &'static str {
    if kind == KIND_EMU_BARRIER {
        return "emulator ResourceBarrier";
    }
    OPS.get(kind.wrapping_sub(KIND_OP0) as usize).map_or("op?", |o| o.1)
}

fn ops_level() -> u64 {
    static L: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    *L.get_or_init(|| env_u64("TOMBOX_GPUPROF_OPS").unwrap_or(1))
}

unsafe fn op_stamp(list: *mut c_void, i: usize) {
    unsafe { stamp_ev(list, None, KIND_OP0 + i as u8, true) };
}

macro_rules! op_hooks {
    ($(($name:ident, $i:literal)),* $(,)?) => {
        $(
            unsafe extern "system" fn $name(this: *mut c_void, a: usize, b: usize, c: usize, d: usize, e: usize, f: usize, g: usize) -> usize {
                unsafe { op_stamp(this, $i) };
                let n: unsafe extern "system" fn(*mut c_void, usize, usize, usize, usize, usize, usize, usize) -> usize =
                    unsafe { core::mem::transmute(OP_NEXT[$i].load(Ordering::Relaxed)) };
                unsafe { n(this, a, b, c, d, e, f, g) }
            }
        )*
    };
}
op_hooks!((op0, 0), (op1, 1), (op2, 2), (op3, 3), (op4, 4), (op6, 6), (op7, 7), (op8, 8), (op9, 9), (op10, 10));

unsafe extern "system" fn op_barrier(this: *mut c_void, n: u32, b: *const u8) {
    unsafe { op_stamp(this, 5) };
    let f: unsafe extern "system" fn(*mut c_void, u32, *const u8) = unsafe { core::mem::transmute(OP_NEXT[5].load(Ordering::Relaxed)) };
    IN_TITLE_BARRIER.with(|c| c.set(true));
    unsafe { f(this, n, b) };
    IN_TITLE_BARRIER.with(|c| c.set(false));
}

unsafe extern "system" fn op_clear_dsv(this: *mut c_void, h: usize, flags: u32, depth: f32, stencil: u8, n: u32, rects: *const c_void) {
    unsafe { op_stamp(this, 11) };
    let f: unsafe extern "system" fn(*mut c_void, usize, u32, f32, u8, u32, *const c_void) = unsafe { core::mem::transmute(OP_NEXT[11].load(Ordering::Relaxed)) };
    unsafe { f(this, h, flags, depth, stencil, n, rects) }
}

static COMMON: Mutex<[u64; 4]> = Mutex::new([0; 4]);

pub fn common_count(b: &[u8], title: bool, c: &mut [u64; 4]) {
    for e in b.chunks_exact(32) {
        let w = |o: usize| u32::from_le_bytes([e[o], e[o + 1], e[o + 2], e[o + 3]]);
        if w(0) != 0 {
            continue;
        }
        let k = if title { 0 } else { 2 };
        c[k] += (w(20) == 0) as u64;
        c[k + 1] += (w(24) == 0) as u64;
    }
}

fn common_text(c: &[u64; 4], nf: u64) -> String {
    let f = |v: u64| v as f64 / nf.max(1) as f64;
    format!("COMMON-state transitions per frame (implicit promotion and decay are not visible to the API): title from COMMON {:.1}, to COMMON {:.1} | emulator from COMMON {:.1}, to COMMON {:.1}", f(c[0]), f(c[1]), f(c[2]), f(c[3]))
}

unsafe extern "system" fn real_barrier(this: *mut c_void, n: u32, b: *const u8) {
    if ARMED.load(Ordering::Relaxed) && !b.is_null() && n > 0 && n < 65536 {
        let title = IN_TITLE_BARRIER.with(|c| c.get());
        let sl = unsafe { core::slice::from_raw_parts(b, n as usize * 32) };
        common_count(sl, title, &mut COMMON.lock().unwrap_or_else(|e| e.into_inner()));
    }
    if ARMED.load(Ordering::Relaxed) && !IN_TITLE_BARRIER.with(|c| c.get()) {
        EMU_BARRIER_CALLS.fetch_add(1, Ordering::Relaxed);
        EMU_BARRIER_COUNT.fetch_add(n, Ordering::Relaxed);
        if ops_level() >= 3 {
            unsafe { stamp_ev(this, None, KIND_EMU_BARRIER, true) };
        }
    }
    let f: unsafe extern "system" fn(*mut c_void, u32, *const u8) = unsafe { core::mem::transmute(REAL_BARRIER.load(Ordering::Relaxed)) };
    unsafe { f(this, n, b) }
}

pub unsafe fn install_ops(vt: &[AtomicUsize]) {
    if !enabled() || ops_level() == 0 {
        return;
    }
    let hooks: [usize; 12] = [op0 as usize, op1 as usize, op2 as usize, op3 as usize, op4 as usize, op_barrier as usize, op6 as usize, op7 as usize, op8 as usize, op9 as usize, op10 as usize, op_clear_dsv as usize];
    let mut done = Vec::new();
    for (i, (slot, name)) in OPS.iter().enumerate() {
        let prev = vt.get(*slot).map_or(0, |v| v.load(Ordering::Relaxed));
        if prev == 0 {
            continue;
        }
        OP_NEXT[i].store(prev, Ordering::Relaxed);
        vt[*slot].store(hooks[i], Ordering::Relaxed);
        done.push(*name);
    }
    let mut emu = "emulator barriers not counted (TOMBOX_GPUPROF_OPS=2 counts them, 3 also times each)".to_string();
    if ops_level() >= 2 {
        let p = unsafe { crate::cmdlist_wrap::real_slot_ptr(26) };
        if !p.is_null() {
            extern "system" {
                fn VirtualProtect(p: *mut c_void, n: usize, prot: u32, old: *mut u32) -> i32;
            }
            let mut old = 0u32;
            if unsafe { VirtualProtect(p as *mut c_void, 8, 0x04, &mut old) } != 0 {
                REAL_BARRIER.store(unsafe { *p }, Ordering::Relaxed);
                unsafe { *p = real_barrier as usize };
                let mut tmp = 0u32;
                unsafe { VirtualProtect(p as *mut c_void, 8, old, &mut tmp) };
                emu = "emulator barriers counted at the real ResourceBarrier".to_string();
            }
        }
    }
    record("d3d12_xs", "gpuprof", &format!("title command timing installed on {}; {emu}", done.join(", ")));
}

fn ops_summary(rows: &[((u64, u64, bool, u8), (u64, u64))], per: &dyn Fn(u64) -> f64, nf: u64, emu_calls: u32, emu_n: u32) -> String {
    let mut v: Vec<(u8, u64, u64)> = Vec::new();
    for ((_, _, _, kind), (t, c)) in rows {
        if *kind >= KIND_OP0 && *kind != KIND_PASS {
            match v.iter_mut().find(|e| e.0 == *kind) {
                Some(e) => {
                    e.1 += t;
                    e.2 += c;
                }
                None => v.push((*kind, *t, *c)),
            }
        }
    }
    v.sort_by(|a, b| b.1.cmp(&a.1));
    let parts: Vec<String> = v.iter().map(|(k, t, c)| format!("{} {:.3} ms/f {:.1}/f", op_name(*k), per(*t), *c as f64 / nf as f64)).collect();
    let mut out = format!("title non-draw commands (each timed to the next stamp, so emulation done inside the call is included): {}", if parts.is_empty() { "-".to_string() } else { parts.join(" | ") });
    if emu_calls > 0 {
        out.push_str(&format!(" | emulator ResourceBarrier calls {:.1}/f ({:.1} barriers/f)", emu_calls as f64 / nf as f64, emu_n as f64 / nf as f64));
    }
    out
}

pub unsafe fn pass_begin(list: *mut c_void, tag: u8) {
    unsafe { stamp_ev(list, Some(tag), KIND_PASS, true) };
}

pub unsafe fn pass_end(list: *mut c_void) {
    unsafe { stamp_ev(list, None, 0, false) };
}

pub unsafe fn own_begin(list: *mut c_void, tag: u8) {
    if !ARMED.load(Ordering::Relaxed) {
        return;
    }
    with(|s| {
        let r = s.lists.entry(list as usize).or_default();
        r.evs.clear();
        r.label = None;
        r.last_kind = 4;
    });
    unsafe { pass_begin(list, tag) };
}

pub unsafe fn own_close(list: *mut c_void) {
    unsafe { close_list(list) };
}

unsafe fn close_list(list: *mut c_void) {
    let ty = unsafe { crate::fill::list_type(list) };
    with(|s| {
        let heap = heap_for(s, ty);
        let Some(r) = s.lists.get_mut(&(list as usize)) else { return };
        if r.evs.is_empty() || heap == 0 {
            r.evs.clear();
            return;
        }
        let mut evs = core::mem::take(&mut r.evs);
        let slot = SLOT.fetch_add(1, Ordering::Relaxed);
        if slot >= SLOTS {
            return;
        }
        let end: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, u32) = unsafe { core::mem::transmute(real(53)) };
        let resolve: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, u32, u32, *mut c_void, u64) =
            unsafe { core::mem::transmute(real(54)) };
        unsafe { end(list, heap as *mut c_void, QUERY_TIMESTAMP, slot) };
        evs.push(Ev { slot, label: Label { a: 0, b: 0, compute: false }, kind: KIND_CLOSE, count: false, frame: FRAME_IX.load(Ordering::Relaxed) });
        s.closed.insert(list as usize, (evs[0].slot, slot, evs.len() >= 2));
        let mut slots: Vec<u32> = evs.iter().map(|e| e.slot).collect();
        slots.sort_unstable();
        let mut i = 0;
        while i < slots.len() {
            let mut j = i + 1;
            while j < slots.len() && slots[j] == slots[j - 1] + 1 {
                j += 1;
            }
            unsafe { resolve(list, heap as *mut c_void, QUERY_TIMESTAMP, slots[i], (j - i) as u32, s.rb as *mut c_void, slots[i] as u64 * 8) };
            i = j;
        }
        s.done.push(evs);
        s.done_ty.push(ty);
    });
}

fn name(l: &Label) -> String {
    if l.compute {
        format!("cs={:016x}", l.a)
    } else {
        format!("vs={:016x} ps={}", l.a, if l.b == 0 { "-".to_string() } else { format!("{:016x}", l.b) })
    }
}

unsafe fn report(s: &mut State) {
    let map: unsafe extern "system" fn(*mut c_void, u32, *const c_void, *mut *mut u8) -> i32 =
        unsafe { core::mem::transmute(*(*(s.rb as *const *const usize)).add(8)) };
    let unmap: unsafe extern "system" fn(*mut c_void, u32, *const c_void) =
        unsafe { core::mem::transmute(*(*(s.rb as *const *const usize)).add(9)) };
    let mut p: *mut u8 = core::ptr::null_mut();
    if unsafe { map(s.rb as *mut c_void, 0, core::ptr::null(), &mut p) } < 0 || p.is_null() {
        record("d3d12_xs", "gpuprof", "readback Map FAILED");
        return;
    }
    let ts = |slot: u32| unsafe { core::ptr::read_unaligned((p as *const u64).add(slot as usize)) };
    let freq = crate::timestamp::freq();
    let tick_ms = if freq != 0 { 1000.0 / freq as f64 } else { 0.0 };
    let max_ticks = if freq != 0 { freq.saturating_mul(2) } else { u64::MAX };
    let args = ARGS.lock().unwrap_or_else(|e| e.into_inner()).take().unwrap_or_default();
    let mut longest: Vec<(u64, u8, Label, u32, u32)> = Vec::new();
    let mut rejected: HashMap<String, u64> = HashMap::new();
    let mut fast_samples = 0u32;
    let mut agg: HashMap<(u64, u64, bool, u8), (u64, u64)> = HashMap::new();
    let mut per_frame: HashMap<u32, u64> = HashMap::new();
    let mut by_frame: HashMap<(u32, u64, u64, bool, u8), u64> = HashMap::new();
    let (mut lo, mut hi, mut bad, mut n) = (u64::MAX, 0u64, 0u64, 0u64);
    let mut iv: Vec<(u64, u64)> = Vec::new();
    let mut list_q: HashMap<u32, (u64, u64)> = HashMap::new();
    for (si, seq) in s.done.iter().enumerate() {
        let lty = s.done_ty.get(si).copied().unwrap_or(0);
        for w in seq.windows(2) {
            let (t0, t1) = (ts(w[0].slot), ts(w[1].slot));
            if t0 != 0 && t1 >= t0 && w[0].kind <= 1 && !w[0].label.compute && crate::passcap::ps_ms_target() == Some(w[0].label.b) {
                let ms = (t1 - t0) as f64 * tick_ms;
                let at = args_text(w[0].kind, args.get(&w[0].slot).copied());
                if crate::passcap::note_slow_call(w[0].label.b, ms, w[0].frame) {
                    record("d3d12_xs", "gpuprof", &format!("ps_ms call: PS {:016x} {ms:.2} ms at present #{} (firstuse frame numbering){at}", w[0].label.b, s.win_start + w[0].frame as u64 + 1));
                } else if ms < 5.0 && fast_samples < 4 {
                    fast_samples += 1;
                    record("d3d12_xs", "gpuprof", &format!("ps_ms fast call sample: PS {:016x} {ms:.2} ms at present #{}{at}", w[0].label.b, s.win_start + w[0].frame as u64 + 1));
                }
            }
            if t0 == 0 || t1 < t0 || t1 - t0 > max_ticks {
                bad += 1;
                let who = if w[0].kind == KIND_PASS { tag_name(w[0].label.a).to_string() } else if w[0].kind >= KIND_OP0 { op_name(w[0].kind).to_string() } else { name(&w[0].label) };
                let why = if t0 == 0 || t1 == 0 { "unresolved" } else if t1 < t0 { "negative" } else { "over 2 s" };
                *rejected.entry(format!("{why} {who}")).or_default() += 1;
                continue;
            }
            if (t1 - t0) as f64 * tick_ms > 20.0 {
                longest.push((t1 - t0, w[0].kind, w[0].label, w[0].frame, w[0].slot));
                longest.sort_by(|a, b| b.0.cmp(&a.0));
                longest.truncate(8);
            }
            lo = lo.min(t0);
            hi = hi.max(t1);
            n += 1;
            iv.push((t0, t1));
            let q = list_q.entry(lty).or_default();
            q.0 += t1 - t0;
            q.1 += (w[0].kind == 4) as u64;
            let e = agg.entry((w[0].label.a, w[0].label.b, w[0].label.compute, w[0].kind)).or_default();
            e.0 += t1 - t0;
            e.1 += w[0].count as u64;
            *per_frame.entry(w[0].frame).or_default() += t1 - t0;
            *by_frame.entry((w[0].frame, w[0].label.a, w[0].label.b, w[0].label.compute, w[0].kind)).or_default() += t1 - t0;
        }
    }
    let mut ecl_q: HashMap<u32, (u64, u64)> = HashMap::new();
    let mut ecl_bad = 0u64;
    for sp in &s.spans {
        let (t0, t1) = (ts(sp.s0), ts(sp.s1));
        if t0 == 0 || t1 < t0 || t1 - t0 > max_ticks.saturating_mul(10) {
            ecl_bad += 1;
            continue;
        }
        lo = lo.min(t0);
        hi = hi.max(t1);
        iv.push((t0, t1));
        let q = ecl_q.entry(sp.ty).or_default();
        q.0 += t1 - t0;
        q.1 += 1;
    }
    let qfreq = crate::gpuprof_q::freqs();
    let mut tl: Vec<crate::gpuprof_tl::SubT> = Vec::new();
    for r in &s.subs {
        let (h, t) = (ts(r.s0), ts(r.s1));
        if h == 0 || t < h {
            continue;
        }
        let mut st = crate::gpuprof_tl::SubT { ty: r.ty, queue: r.queue, head: h, tail: t, lists: Vec::new(), n_lists: r.n_lists, missing: r.missing, cpu: r.cpu, head_qpc: None };
        for &(a, b) in &r.lists {
            let (ta, tb) = (ts(a), ts(b));
            if ta == 0 || tb < ta {
                st.missing += 1;
            } else {
                st.lists.push((ta, tb));
            }
        }
        let gf = qfreq.iter().find(|e| e.0 == r.ty).map_or(freq, |e| e.1);
        st.head_qpc = s.calib.get(&r.queue).and_then(|c| crate::gpuprof_tl::gpu_to_qpc(&c.1, gf, crate::frame::qpf(), h));
        tl.push(st);
    }
    unsafe { unmap(s.rb as *mut c_void, 0, core::ptr::null()) };
    let covered = union_ticks(&mut iv);
    let mut rows: Vec<_> = agg.into_iter().collect();
    rows.sort_by(|a, b| b.1 .0.cmp(&a.1 .0));
    let total: u64 = rows.iter().map(|r| r.1 .0).sum();
    let nf = frames();
    let per = |t: u64| t as f64 * tick_ms / nf as f64;
    let span = if hi > lo { per(hi - lo) } else { 0.0 };
    let mut out = format!(
        "[gpuprof] window {}: {nf} frames, {n} timed events ({bad} rejected), freq={freq} Hz | summed GPU {:.3} ms/frame, wall span {:.3} ms/frame\n",
        s.reports, per(total), span);
    out.push_str(&format!("[gpuprof]   passes: {}\n", pass_summary(&rows, &per, nf)));
    out.push_str(&format!("[gpuprof]   {}\n", longest_text(&longest, &rejected, tick_ms, &args)));
    let (i0, i1) = *INJ.lock().unwrap_or_else(|e| e.into_inner());
    out.push_str(&format!("[gpuprof]   {}\n", inject_text(&i0, &i1, nf)));
    out.push_str(&format!("[gpuprof]   {}\n", ops_summary(&rows, &per, nf, EMU_BARRIER_CALLS.load(Ordering::Relaxed), EMU_BARRIER_COUNT.load(Ordering::Relaxed))));
    out.push_str(&format!("[gpuprof]   {}\n", queue_summary(&ecl_q, &list_q, ecl_bad, &per, nf, if hi > lo { hi - lo } else { 0 }, covered)));
    for l in crate::gpuprof_tl::analyse(&mut tl, tick_ms, crate::frame::qpf(), nf).lines() {
        out.push_str(&format!("[gpuprof]   {l}\n"));
    }
    out.push_str(&format!("[gpuprof]   {}\n", common_text(&COMMON.lock().unwrap_or_else(|e| e.into_inner()), nf)));
    out.push_str(&format!("[gpuprof]   {}\n", crate::gpuprof_q::freq_note(&crate::gpuprof_q::freqs(), freq)));
    out.push_str(&format!("[gpuprof]   {}\n", crate::gpuprof_q::report_line()));
    if let Some((wf, wt, med)) = worst_frame(&per_frame) {
        let ms = |t: u64| t as f64 * tick_ms;
        let mut d: Vec<(f64, String)> = rows.iter().map(|((a, b, compute, kind), (t, _))| {
            let in_worst = by_frame.get(&(wf, *a, *b, *compute, *kind)).copied().unwrap_or(0);
            let who = if *kind == KIND_PASS { tag_name(*a).to_string() } else if *kind >= KIND_OP0 { op_name(*kind).to_string() } else { name(&Label { a: *a, b: *b, compute: *compute }) };
            (ms(in_worst) - per(*t), who)
        }).collect();
        d.sort_by(|x, y| y.0.partial_cmp(&x.0).unwrap_or(core::cmp::Ordering::Equal));
        let top: Vec<String> = d.iter().take(8).filter(|x| x.0 > 0.05).map(|(x, w)| format!("+{x:.2} {w}")).collect();
        out.push_str(&format!("[gpuprof]   slowest frame {wf}: {:.3} ms vs median {:.3} ms over {} frames | over its per-frame average: {}\n",
            ms(wt), ms(med), per_frame.len(), if top.is_empty() { "-".to_string() } else { top.join(", ") }));
    }
    for ((a, b, compute, kind), (t, c)) in rows.iter().take(40) {
        let k = match *kind { 0 => "draw", 1 => "drawidx", 2 => "dispatch", 3 => "indirect", 4 => "listhead", KIND_PASS => "emupass", k if k >= KIND_OP0 => "titleop", _ => "?" };
        let who = if *kind == KIND_PASS { tag_name(*a).to_string() } else if *kind >= KIND_OP0 { op_name(*kind).to_string() } else { name(&Label { a: *a, b: *b, compute: *compute }) };
        out.push_str(&format!("[gpuprof]   {:7.3} ms/f {:5.1}% {:6.1} calls/f {k:8} {who}\n",
            per(*t), *t as f64 * 100.0 / total.max(1) as f64, *c as f64 / nf as f64));
    }
    record("d3d12_xs", "gpuprof", &out);
}

fn longest_text(longest: &[(u64, u8, Label, u32, u32)], rejected: &HashMap<String, u64>, tick_ms: f64, args: &HashMap<u32, [u32; 5]>) -> String {
    let l: Vec<String> = longest.iter().map(|(t, kind, label, frame, slot)| {
        let who = if *kind == KIND_PASS { tag_name(label.a).to_string() } else if *kind >= KIND_OP0 { op_name(*kind).to_string() } else { name(label) };
        format!("{:.2} ms frame {frame} {who}{}", *t as f64 * tick_ms, args_text(*kind, args.get(slot).copied()))
    }).collect();
    let mut r: Vec<(&String, &u64)> = rejected.iter().collect();
    r.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    let r: Vec<String> = r.iter().take(8).map(|(k, n)| format!("{k} x{n}")).collect();
    format!("longest single events (>20 ms): {} | rejected intervals: {}", if l.is_empty() { "none".to_string() } else { l.join("; ") }, if r.is_empty() { "none".to_string() } else { r.join(", ") })
}

fn union_ticks(iv: &mut Vec<(u64, u64)>) -> u64 {
    iv.sort_unstable();
    let mut total = 0u64;
    let mut cur: Option<(u64, u64)> = None;
    for &(a, b) in iv.iter() {
        match cur {
            Some((c0, c1)) if a <= c1 => cur = Some((c0, c1.max(b))),
            Some((c0, c1)) => {
                total += c1 - c0;
                cur = Some((a, b));
            }
            None => cur = Some((a, b)),
        }
    }
    if let Some((c0, c1)) = cur {
        total += c1 - c0;
    }
    total
}

fn queue_summary(ecl: &HashMap<u32, (u64, u64)>, lists: &HashMap<u32, (u64, u64)>, ecl_bad: u64, per: &dyn Fn(u64) -> f64, nf: u64, span: u64, covered: u64) -> String {
    let mut tys: Vec<u32> = ecl.keys().chain(lists.keys()).copied().collect();
    tys.sort_unstable();
    tys.dedup();
    let mut parts: Vec<String> = Vec::new();
    for t in tys {
        let (et, en) = ecl.get(&t).copied().unwrap_or((0, 0));
        let (lt, ln) = lists.get(&t).copied().unwrap_or((0, 0));
        parts.push(format!("{} submissions {:.3} ms/f ({:.1}/f) lists {:.3} ms/f ({:.1}/f)", crate::gpuprof_q::type_name(t), per(et), en as f64 / nf as f64, per(lt), ln as f64 / nf as f64));
    }
    let sum: u64 = ecl.values().map(|v| v.0).sum();
    let mut out = format!("queues: {}", if parts.is_empty() { "-".to_string() } else { parts.join(" | ") });
    out.push_str(&format!(" | summed submissions {:.3} ms/f, timed union {:.3} ms/f, untimed gap {:.3} ms/f of wall span {:.3} ms/f", per(sum), per(covered), per(span.saturating_sub(covered)), per(span)));
    if ecl_bad > 0 {
        out.push_str(&format!(", {ecl_bad} submission spans rejected"));
    }
    out
}

fn pass_summary(rows: &[((u64, u64, bool, u8), (u64, u64))], per: &dyn Fn(u64) -> f64, nf: u64) -> String {
    let mut title = (0u64, 0u64);
    let mut tags: Vec<(u64, u64, u64)> = Vec::new();
    for ((a, _, _, kind), (t, c)) in rows {
        if *kind == KIND_PASS {
            match tags.iter_mut().find(|e| e.0 == *a) {
                Some(e) => {
                    e.1 += t;
                    e.2 += c;
                }
                None => tags.push((*a, *t, *c)),
            }
        } else {
            title.0 += t;
            title.1 += c;
        }
    }
    tags.sort_by_key(|e| e.0);
    let mut out = format!("title {:.3} ms/f {:.1} calls/f", per(title.0), title.1 as f64 / nf as f64);
    for (a, t, c) in tags {
        out.push_str(&format!(" | {} {:.3} ms/f {:.1} calls/f", tag_name(a), per(t), c as f64 / nf as f64));
    }
    out
}

pub fn on_present() {
    if !enabled() {
        return;
    }
    with(|s| {
        s.presents += 1;
        if let Some(at) = s.finalize_at {
            if s.presents >= at {
                if s.heap != 0 && s.heap != usize::MAX {
                    unsafe { report(s) };
                }
                s.reports += 1;
                s.finalize_at = None;
                s.done.clear();
                s.done_ty.clear();
                s.spans.clear();
                unsafe { sweep_retired(s) };
                s.last = Some(Instant::now());
            }
            return;
        }
        if let Some(start) = s.window {
            FRAME_IX.store((s.presents - start) as u32, Ordering::Relaxed);
            if s.presents >= start + frames() {
                ARMED.store(false, Ordering::Relaxed);
                crate::gpuprof_q::disarm();
                let qs: Vec<usize> = s.calib.keys().copied().collect();
                for q in qs {
                    if let Some(c) = unsafe { crate::gpuprof_tl::calibrate(q as *mut c_void) } {
                        if let Some(e) = s.calib.get_mut(&q) {
                            e.1.push(c);
                        }
                    }
                }
                INJ.lock().unwrap_or_else(|e| e.into_inner()).1 = crate::inject::totals();
                s.window = None;
                s.finalize_at = Some(s.presents + SETTLE);
            }
            return;
        }
        let due = match s.last {
            None => {
                s.last = Some(Instant::now());
                false
            }
            Some(t) => t.elapsed().as_millis() as u64 >= period_ms(),
        };
        if due && windows_on() {
            for r in s.lists.values_mut() {
                r.evs.clear();
            }
            s.done.clear();
            s.done_ty.clear();
            s.spans.clear();
            s.open.clear();
            s.closed.clear();
            s.subs.clear();
            s.sub_open.clear();
            s.calib.clear();
            *COMMON.lock().unwrap_or_else(|e| e.into_inner()) = [0; 4];
            unsafe { sweep_retired(s) };
            SLOT.store(0, Ordering::Relaxed);
            FRAME_IX.store(0, Ordering::Relaxed);
            s.window = Some(s.presents);
            s.win_start = s.presents;
            crate::gpuprof_q::arm();
            *INJ.lock().unwrap_or_else(|e| e.into_inner()) = (crate::inject::totals(), crate::inject::totals());
            EMU_BARRIER_CALLS.store(0, Ordering::Relaxed);
            EMU_BARRIER_COUNT.store(0, Ordering::Relaxed);
            ARMED.store(true, Ordering::Relaxed);
        }
    });
}

unsafe extern "system" fn h_close(this: *mut c_void) -> i32 {
    unsafe { crate::blendclamp::flush(this) };
    unsafe { close_list(this) };
    let f: unsafe extern "system" fn(*mut c_void) -> i32 = unsafe { core::mem::transmute(next(9)) };
    unsafe { f(this) }
}

unsafe extern "system" fn h_reset(this: *mut c_void, alloc: *mut c_void, pso: *mut c_void) -> i32 {
    with(|s| {
        let r = s.lists.entry(this as usize).or_default();
        r.evs.clear();
        r.label = None;
    });
    if !pso.is_null() {
        on_set_pso(this as usize, pso as usize);
    }
    let f: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32 = unsafe { core::mem::transmute(next(10)) };
    let hr = unsafe { f(this, alloc, pso) };
    if hr >= 0 && ARMED.load(Ordering::Relaxed) && unsafe { crate::fill::list_type(this) } != 1 {
        unsafe { stamp(this, 4) };
    }
    hr
}

unsafe extern "system" fn h_set_pso(this: *mut c_void, pso: *mut c_void) {
    on_set_pso(this as usize, pso as usize);
    let f: unsafe extern "system" fn(*mut c_void, *mut c_void) = unsafe { core::mem::transmute(next(25)) };
    unsafe { f(this, pso) }
}

unsafe extern "system" fn h_draw(this: *mut c_void, a: u32, b: u32, c: u32, d: u32) {
    let slot = unsafe { stamp(this, 0) };
    note_args(this, slot, [a, b, c, d, 0]);
    let f: unsafe extern "system" fn(*mut c_void, u32, u32, u32, u32) = unsafe { core::mem::transmute(next(12)) };
    unsafe { f(this, a, b, c, d) }
}

unsafe extern "system" fn h_draw_indexed(this: *mut c_void, a: u32, b: u32, c: u32, d: i32, e: u32) {
    let slot = unsafe { stamp(this, 1) };
    note_args(this, slot, [a, b, c, d as u32, e]);
    let f: unsafe extern "system" fn(*mut c_void, u32, u32, u32, i32, u32) = unsafe { core::mem::transmute(next(13)) };
    unsafe { f(this, a, b, c, d, e) }
}

unsafe extern "system" fn h_dispatch(this: *mut c_void, a: u32, b: u32, c: u32) {
    unsafe { stamp(this, 2) };
    let f: unsafe extern "system" fn(*mut c_void, u32, u32, u32) = unsafe { core::mem::transmute(next(14)) };
    unsafe { f(this, a, b, c) }
}

unsafe extern "system" fn h_indirect(this: *mut c_void, sig: *mut c_void, n: u32, buf: *mut c_void, off: u64, cbuf: *mut c_void, coff: u64) {
    let slot = unsafe { stamp(this, 3) };
    note_args(this, slot, [n, off as u32, (!cbuf.is_null()) as u32, 0, 0]);
    let f: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, *mut c_void, u64, *mut c_void, u64) = unsafe { core::mem::transmute(next(59)) };
    unsafe { f(this, sig, n, buf, off, cbuf, coff) }
}

#[cfg(test)]
pub(crate) unsafe fn test_stamp(queue: *mut c_void, exec: ExecFn, ty: u32) -> Option<u32> {
    unsafe { submit_stamp(queue, exec, ty) }
}

#[cfg(test)]
pub(crate) fn test_pool() -> usize {
    with(|s| s.retire.len())
}

#[cfg(test)]
pub(crate) fn test_fence() -> (usize, u64) {
    with(|s| (s.fence, s.fence_val))
}

#[cfg(test)]
pub(crate) fn test_has_copy_heap() -> bool {
    with(|s| s.cheap != 0 && s.cheap != usize::MAX)
}

#[cfg(test)]
pub(crate) unsafe fn test_ticks(slots: &[u32]) -> Vec<u64> {
    with(|s| {
        let map: unsafe extern "system" fn(usize, u32, *const c_void, *mut *mut u8) -> i32 = unsafe { core::mem::transmute(vt_at(s.rb, 8)) };
        let unmap: unsafe extern "system" fn(usize, u32, *const c_void) = unsafe { core::mem::transmute(vt_at(s.rb, 9)) };
        let mut p: *mut u8 = core::ptr::null_mut();
        if unsafe { map(s.rb, 0, core::ptr::null(), &mut p) } < 0 || p.is_null() {
            return Vec::new();
        }
        let v = slots.iter().map(|&i| unsafe { core::ptr::read_unaligned((p as *const u64).add(i as usize)) }).collect();
        unsafe { unmap(s.rb, 0, core::ptr::null()) };
        v
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_print_console_hashes() {
        assert_eq!(name(&Label { a: 0x1e8bbd9bbe7ab9b7, b: 0, compute: true }), "cs=1e8bbd9bbe7ab9b7");
        assert_eq!(name(&Label { a: 1, b: 0, compute: false }), "vs=0000000000000001 ps=-");
    }

    #[test]
    fn pass_summary_splits_title_time_from_each_emulator_pass() {
        let rows = vec![
            ((7, 8, false, 0), (1600, 32)),
            ((TAG_BLENDCLAMP as u64, 0, false, KIND_PASS), (320, 48)),
            ((9, 0, true, 2), (160, 16)),
            ((TAG_CBV_COPY as u64, 0, false, KIND_PASS), (16, 160)),
        ];
        let per = |t: u64| t as f64 / FRAMES as f64;
        assert_eq!(pass_summary(&rows, &per, FRAMES), "title 110.000 ms/f 3.0 calls/f | blendclamp 20.000 ms/f 3.0 calls/f | cbv_copy 1.000 ms/f 10.0 calls/f");
    }

    #[test]
    fn worst_frame_picks_the_slowest_and_reports_the_median() {
        let m: HashMap<u32, u64> = [(0, 10), (1, 12), (2, 40), (3, 11), (4, 13)].into_iter().collect();
        assert_eq!(worst_frame(&m), Some((2, 40, 12)));
        let tie: HashMap<u32, u64> = [(5, 9), (3, 9)].into_iter().collect();
        assert_eq!(worst_frame(&tie), Some((3, 9, 9)));
        assert_eq!(worst_frame(&HashMap::new()), None);
    }

    #[test]
    fn direct_compute_and_copy_lists_take_timestamps_but_bundles_do_not() {
        assert!(stampable(0));
        assert!(stampable(2));
        assert!(stampable(3));
        assert!(!stampable(1));
    }

    #[test]
    fn union_merges_overlapping_intervals_across_queues() {
        let mut iv = vec![(10, 20), (15, 30), (40, 50), (0, 5), (45, 48)];
        assert_eq!(union_ticks(&mut iv), 5 + 20 + 10);
        assert_eq!(union_ticks(&mut Vec::new()), 0);
        let ecl: HashMap<u32, (u64, u64)> = [(0, (32, 16)), (2, (16, 16))].into_iter().collect();
        let lists: HashMap<u32, (u64, u64)> = [(0, (16, 32))].into_iter().collect();
        let per = |t: u64| t as f64 / 16.0;
        assert_eq!(queue_summary(&ecl, &lists, 0, &per, 16, 160, 40), "queues: direct submissions 2.000 ms/f (1.0/f) lists 1.000 ms/f (2.0/f) | compute submissions 1.000 ms/f (1.0/f) lists 0.000 ms/f (0.0/f) | summed submissions 3.000 ms/f, timed union 2.500 ms/f, untimed gap 7.500 ms/f of wall span 10.000 ms/f");
    }

    #[test]
    fn every_tag_has_a_name() {
        for t in [TAG_BLENDCLAMP, TAG_CBV_COPY, TAG_RESOLVE, TAG_UPLOAD, TAG_PRESENT, TAG_INJECT0, TAG_INJECT0 + 6] {
            assert_ne!(tag_name(t as u64), "emu?");
        }
        assert_eq!(tag_name((TAG_INJECT0 + 1) as u64), "inject:copy");
        assert_eq!(tag_name((TAG_INJECT0 + 7) as u64), "emu?");
    }

    static SEEN: Mutex<Vec<u64>> = Mutex::new(Vec::new());

    unsafe extern "system" fn rec7(_t: *mut c_void, a: usize, b: usize, c: usize, d: usize, e: usize, f: usize, g: usize) -> usize {
        SEEN.lock().unwrap().extend([a, b, c, d, e, f, g].map(|x| x as u64));
        99
    }

    unsafe extern "system" fn rec_dsv(_t: *mut c_void, h: usize, flags: u32, depth: f32, stencil: u8, n: u32, rects: *const c_void) {
        SEEN.lock().unwrap().extend([h as u64, flags as u64, depth.to_bits() as u64, stencil as u64, n as u64, rects as u64]);
    }

    #[test]
    fn common_count_splits_title_and_emulator_transitions() {
        let mut b = vec![0u8; 96];
        b[24..28].copy_from_slice(&4u32.to_le_bytes());
        b[32 + 20..32 + 24].copy_from_slice(&0x80u32.to_le_bytes());
        b[64..68].copy_from_slice(&1u32.to_le_bytes());
        let mut c = [0u64; 4];
        common_count(&b, true, &mut c);
        common_count(&b[..32], false, &mut c);
        assert_eq!(c, [1, 1, 1, 0]);
        assert!(common_text(&c, 2).contains("title from COMMON 0.5, to COMMON 0.5 | emulator from COMMON 0.5, to COMMON 0.0"));
    }

    #[test]
    fn longest_events_and_rejections_are_named() {
        let l = Label { a: 1, b: 0xb014f6a0dce8314d, compute: false };
        let mut args = HashMap::new();
        args.insert(7u32, [36u32, 1, 0, 0, 0]);
        let mut rej = HashMap::new();
        rej.insert("over 2 s vs=0000000000000001 ps=b014f6a0dce8314d".to_string(), 2u64);
        let s = longest_text(&[(250_000_000, 1, l, 3, 7)], &rej, 1e-6, &args);
        assert_eq!(s, "longest single events (>20 ms): 250.00 ms frame 3 vs=0000000000000001 ps=b014f6a0dce8314d | drawidx indices=36 instances=1 first_index=0 base_vertex=0 first_instance=0 | rejected intervals: over 2 s vs=0000000000000001 ps=b014f6a0dce8314d x2");
        assert_eq!(longest_text(&[], &HashMap::new(), 1.0, &HashMap::new()), "longest single events (>20 ms): none | rejected intervals: none");
        assert_eq!(args_text(3, Some([4, 0x40, 1, 0, 0])), " | indirect max_commands=4 arg_offset=0x40 count_buffer=yes");
    }

    #[test]
    fn inject_text_reports_per_frame_deltas() {
        let a = [0u64; crate::inject::KINDS];
        let mut b = a;
        b[crate::inject::BARRIER as usize] = 32;
        b[crate::inject::COPY as usize] = 8;
        assert_eq!(inject_text(&a, &b, 16), "emulator-injected commands in the window: copy 0.5/f, barrier 2.0/f");
        assert_eq!(inject_text(&a, &a, 16), "emulator-injected commands in the window: none");
    }

    #[test]
    fn op_hooks_forward_every_argument_unchanged() {
        OP_NEXT[0].store(rec7 as usize, Ordering::Relaxed);
        OP_NEXT[11].store(rec_dsv as usize, Ordering::Relaxed);
        SEEN.lock().unwrap().clear();
        assert_eq!(unsafe { op0(core::ptr::null_mut(), 1, 2, 3, 4, 5, 6, 7) }, 99);
        unsafe { op_clear_dsv(core::ptr::null_mut(), 0x1234, 3, 0.25, 7, 2, 0x40 as *const c_void) };
        assert_eq!(*SEEN.lock().unwrap(), vec![1, 2, 3, 4, 5, 6, 7, 0x1234, 3, 0.25f32.to_bits() as u64, 7, 2, 0x40]);
    }

    #[test]
    fn op_kinds_are_named_and_summarised_apart_from_draws() {
        assert_eq!(op_name(KIND_OP0 + 5), "ResourceBarrier");
        assert_eq!(op_name(KIND_OP0 + 11), "ClearDepthStencilView");
        assert_eq!(op_name(KIND_EMU_BARRIER), "emulator ResourceBarrier");
        assert!(KIND_OP0 > KIND_PASS && KIND_EMU_BARRIER < KIND_CLOSE);
        let rows = vec![
            ((7, 8, false, 0), (1600, 32)),
            ((0, 0, false, KIND_OP0 + 1), (320, 16)),
            ((0, 0, false, KIND_OP0 + 5), (32, 160)),
            ((TAG_CBV_COPY as u64, 0, false, KIND_PASS), (16, 160)),
        ];
        let per = |t: u64| t as f64 / 16.0;
        assert_eq!(ops_summary(&rows, &per, 16, 32, 64), "title non-draw commands (each timed to the next stamp, so emulation done inside the call is included): CopyTextureRegion 20.000 ms/f 1.0/f | ResourceBarrier 2.000 ms/f 10.0/f | emulator ResourceBarrier calls 2.0/f (4.0 barriers/f)");
    }
}
