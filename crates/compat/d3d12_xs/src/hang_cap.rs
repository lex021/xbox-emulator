use compat_common::record;
use core::ffi::c_void;
use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, OnceLock};

pub const DUMP_BYTES: usize = 0x700;
pub const BOUND_BYTES: usize = 0x470;
const QUEUE_RING: usize = 1024;
const CMD_RING: usize = 16384;
const KEEP_DRAWS: usize = 4;
const LOG_DRAWS: usize = 2;
const XBOX_SLOT_COPY_RESOURCE: usize = 17;
const XBOX_SLOT_SET_PSO: usize = 25;
const XBOX_SLOT_RESOURCE_BARRIER: usize = 26;
const PC_QUEUE_SIGNAL: usize = 14;
const PC_QUEUE_WAIT: usize = 15;
const PC_QUEUE_GET_DESC: usize = 18;
const FENCE_GET_COMPLETED: usize = 8;
const DEFAULT_PS: [u64; 3] = [0xf861c56a34758d84, 0x8f215c725a248afa, 0xe4db32f6038a003b];

pub fn on() -> bool {
    static E: OnceLock<bool> = OnceLock::new();
    *E.get_or_init(|| {
        let on = crate::cbv_raw::enabled() && mode_from(std::env::var("TOMBOX_CBRAW_CHECK").ok().as_deref());
        if on {
            record("d3d12_xs", "hang_cap", &format!("TOMBOX_CBRAW_CHECK=2: root CBV bytes ({DUMP_BYTES:#x}) at draws of pixel shaders {:x?}, queue Signal/Wait/ExecuteCommandLists ring and per-list barrier/copy ring, dumped at device removal", targets()));
        }
        on
    })
}

pub fn mode_from(v: Option<&str>) -> bool {
    v.map(|s| s.trim() == "2").unwrap_or(false)
}

pub fn parse_targets(v: Option<&str>) -> Vec<u64> {
    match v {
        Some(s) if !s.trim().is_empty() => s.split(',').filter_map(|h| u64::from_str_radix(h.trim(), 16).ok()).collect(),
        _ => DEFAULT_PS.to_vec(),
    }
}

fn targets() -> &'static Vec<u64> {
    static T: OnceLock<Vec<u64>> = OnceLock::new();
    T.get_or_init(|| parse_targets(std::env::var("TOMBOX_CBRAW_CAP_PS").ok().as_deref()))
}

#[derive(Clone)]
struct DrawSnap {
    seq: u64,
    list: usize,
    ps: u64,
    vs: u64,
    text: String,
    roots: Vec<(usize, u32, u32, u32, bool, u64, Option<Vec<u8>>)>,
}

#[derive(Default)]
struct State {
    pso: HashMap<usize, usize>,
    queues: HashMap<usize, u32>,
    fences: HashMap<usize, u64>,
    qring: VecDeque<String>,
    cring: VecDeque<(u64, usize, String)>,
    draws: HashMap<u64, VecDeque<DrawSnap>>,
    logged: HashMap<(u64, usize), usize>,
}

static ST: Mutex<Option<State>> = Mutex::new(None);
static SEQ: AtomicU64 = AtomicU64::new(0);
static PREV_SET_PSO: AtomicUsize = AtomicUsize::new(0);
static PREV_BARRIER: AtomicUsize = AtomicUsize::new(0);
static PREV_COPY: AtomicUsize = AtomicUsize::new(0);
static REAL_SIGNAL: AtomicUsize = AtomicUsize::new(0);
static REAL_WAIT: AtomicUsize = AtomicUsize::new(0);
static REAL_GET_DESC: AtomicUsize = AtomicUsize::new(0);

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut g = ST.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(State::default))
}

fn seq() -> u64 {
    SEQ.fetch_add(1, Ordering::Relaxed)
}

fn now_us() -> u64 {
    static T0: OnceLock<std::time::Instant> = OnceLock::new();
    T0.get_or_init(std::time::Instant::now).elapsed().as_micros() as u64
}

unsafe fn vslot(o: *mut c_void, i: usize) -> usize {
    unsafe { *(*(o as *const *const usize)).add(i) }
}

fn read_b13(va: u64) -> Option<Vec<u8>> {
    unsafe { crate::bindtrace::read_va(va, DUMP_BYTES) }.or_else(|| read_mem(va, DUMP_BYTES))
}

pub fn read_mem(addr: u64, n: usize) -> Option<Vec<u8>> {
    extern "system" {
        fn ReadProcessMemory(p: *mut c_void, base: *const c_void, buf: *mut c_void, n: usize, read: *mut usize) -> i32;
        fn GetCurrentProcess() -> *mut c_void;
    }
    if addr < 0x10000 {
        return None;
    }
    let mut v = vec![0u8; n];
    let mut got = 0usize;
    let ok = unsafe { ReadProcessMemory(GetCurrentProcess(), addr as *const c_void, v.as_mut_ptr() as *mut c_void, n, &mut got) };
    if ok != 0 && got == n { Some(v) } else { None }
}

pub fn hex_rows(b: &[u8], bound: usize) -> String {
    let mut out = String::new();
    for (i, row) in b.chunks(16).enumerate() {
        let off = i * 16;
        let words: Vec<String> = row.chunks(4).filter(|w| w.len() == 4).map(|w| format!("{:08x}", u32::from_le_bytes([w[0], w[1], w[2], w[3]]))).collect();
        let floats: Vec<String> = row.chunks(4).filter(|w| w.len() == 4).map(|w| format!("{}", f32::from_le_bytes([w[0], w[1], w[2], w[3]]))).collect();
        let mark = if off == bound { " <-- bound size" } else { "" };
        out.push_str(&format!("      +{off:04x} [{i:3}] {}  ({}){mark}\n", words.join(" "), floats.join(", ")));
    }
    out
}

fn hashes_of(pso: usize) -> (u64, u64) {
    if let Some(h) = crate::dred_marks::hashes(pso) {
        return (h.vs, h.ps);
    }
    crate::device_wrap::pso_summary(pso)
        .map(|(v, p, _)| {
            let h = |c: usize| if c == 0 { 0 } else { crate::shader_xlat::origin_hash(c).unwrap_or(0) };
            (h(v), h(p))
        })
        .unwrap_or((0, 0))
}

pub fn note_evict(old: &crate::gpu_va::Entry, new: &crate::gpu_va::Entry) {
    if !on() || old.len < 0x10000 {
        return;
    }
    static N: AtomicUsize = AtomicUsize::new(0);
    if N.fetch_add(1, Ordering::Relaxed) >= 400 {
        return;
    }
    record("d3d12_xs", "hang_cap", &format!("gpu_va range evicted: res {:#x} [{:#x}, +{:#x}) buffer={} replaced by res {:#x} [{:#x}, +{:#x}) buffer={} t={}us", old.res, old.start, old.len, old.buffer, new.res, new.start, new.len, new.buffer, now_us()));
}

static RESIDENCY: Mutex<Option<HashMap<usize, (bool, u64)>>> = Mutex::new(None);
static RESIDENCY_CALLS: [AtomicU64; 2] = [AtomicU64::new(0), AtomicU64::new(0)];
static RESIDENCY_OBJS: [AtomicU64; 2] = [AtomicU64::new(0), AtomicU64::new(0)];

pub fn residency_counts() -> [u64; 4] {
    [RESIDENCY_CALLS[0].load(Ordering::Relaxed), RESIDENCY_CALLS[1].load(Ordering::Relaxed), RESIDENCY_OBJS[0].load(Ordering::Relaxed), RESIDENCY_OBJS[1].load(Ordering::Relaxed)]
}

pub unsafe fn note_residency(evict: bool, n: u32, objs: *const *mut c_void) {
    RESIDENCY_CALLS[evict as usize].fetch_add(1, Ordering::Relaxed);
    RESIDENCY_OBJS[evict as usize].fetch_add(n as u64, Ordering::Relaxed);
    if !on() || objs.is_null() {
        return;
    }
    let t = now_us();
    let mut g = RESIDENCY.lock().unwrap_or_else(|e| e.into_inner());
    let m = g.get_or_insert_with(HashMap::new);
    for i in 0..n as usize {
        let o = *objs.add(i) as usize;
        if o != 0 {
            m.insert(o, (evict, t));
        }
    }
}

fn residency_of(r: usize) -> Option<(bool, u64)> {
    RESIDENCY.lock().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(|m| m.get(&r).copied())
}

static ORIGIN: Mutex<Option<HashMap<usize, String>>> = Mutex::new(None);

pub fn note_origin(pso: usize, tag: impl FnOnce() -> String) {
    if !on() || pso == 0 {
        return;
    }
    ORIGIN.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).insert(pso, tag());
}

fn origin_of(pso: usize) -> String {
    ORIGIN.lock().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(|m| m.get(&pso).cloned()).unwrap_or_else(|| "no creation record (not created through the graphics or derived path)".into())
}

fn fnv(b: &[u8]) -> u64 {
    b.iter().fold(0xcbf29ce484222325u64, |h, &x| (h ^ x as u64).wrapping_mul(0x100000001b3))
}

pub fn dump_dir() -> std::path::PathBuf {
    std::env::var_os("TOMBOX_LOG").map(std::path::PathBuf::from).and_then(|p| p.parent().map(|d| d.to_path_buf())).unwrap_or_else(std::env::temp_dir).join("hang_cap_pso")
}

fn dump_pso(bound: usize, ps: u64) -> String {
    let real = crate::pso_async::real_of(bound);
    let mut out = format!("bound pso {bound:#x}: ");
    let target = if real != 0 && real != bound {
        out.push_str(&format!("async placeholder, real {real:#x} (placeholder has its own registry record: {}); ", crate::device_wrap::pso_record_bytes(bound).is_some()));
        real
    } else {
        out.push_str("not an async placeholder; ");
        bound
    };
    let mut objs = vec![target];
    if target != bound {
        objs.push(bound);
    }
    let dir = dump_dir();
    let _ = std::fs::create_dir_all(&dir);
    for o in objs {
        out.push_str(&format!("\n    pso {o:#x} origin: {}", origin_of(o)));
        let Some((desc, vs, psb, layout, names)) = crate::device_wrap::pso_record_bytes(o) else {
            out.push_str(" | not in the PSO registry");
            continue;
        };
        let stem = format!("ps{ps:016x}_pso{o:x}");
        let mut il = String::new();
        for (i, e) in layout.chunks(32).enumerate() {
            let u = |k: usize| u32::from_le_bytes(e[k..k + 4].try_into().unwrap());
            let nm = names.get(i).map(|n| String::from_utf8_lossy(n.strip_suffix(&[0]).unwrap_or(n)).to_string()).unwrap_or_default();
            il.push_str(&format!("{nm}{} fmt={} slot={} off={} class={} step={}\n", u(8), u(12), u(16), u(20), u(24), u(28)));
        }
        for (tag, b) in [("vs.dxil", &vs), ("ps.dxil", &psb), ("desc.bin", &desc), ("layout.bin", &layout)] {
            let _ = std::fs::write(dir.join(format!("{stem}_{tag}")), b);
        }
        let ext = crate::pso_ext::entry_of(o as *mut c_void).map(|e| e.iter().map(|b| format!("{b:02x}")).collect::<String>()).unwrap_or_else(|| "none".into());
        let _ = std::fs::write(dir.join(format!("{stem}_info.txt")), format!("origin: {}\next entry: {ext}\ndesc: {}\ninput layout:\n{il}", origin_of(o), crate::device_wrap::desc_text(&desc)));
        out.push_str(&format!(" | vs {}B fnv={:016x} ps {}B fnv={:016x} desc {}B fnv={:016x} ext {ext} -> {}", vs.len(), fnv(&vs), psb.len(), fnv(&psb), desc.len(), fnv(&desc), dir.join(&stem).display()));
    }
    out
}

pub fn res_text(r: usize) -> String {
    extern "system" {
        fn IsBadReadPtr(p: *const c_void, n: usize) -> i32;
    }
    let mut flags: Vec<&str> = Vec::new();
    let mut out = format!("res {r:#x}");
    if let Some(p) = crate::placed::lookup(r as *mut c_void) {
        out.push_str(&format!(" placed {}x{}x{} mips={} fmt={} layout={} flags={:#x}", p.width(), p.height(), p.depth_or_array(), p.mips(), p.format(), p.layout(), p.flags()));
    }
    let readable = unsafe { r >= 0x10000 && IsBadReadPtr(r as *const c_void, 8) == 0 && IsBadReadPtr(*(r as *const *const c_void), 15 * 8) == 0 };
    if !readable {
        flags.push("UNREADABLE-OBJECT");
    } else {
        let mut d = [0u8; 0x38];
        unsafe {
            let f: unsafe extern "system" fn(*mut c_void, *mut [u8; 0x38]) -> *mut [u8; 0x38] = core::mem::transmute(vslot(r as *mut c_void, 10));
            f(r as *mut c_void, &mut d);
        }
        let u = |o: usize| u32::from_le_bytes(d[o..o + 4].try_into().unwrap());
        let layout = u(0x2c);
        out.push_str(&format!(" dim={} {}x{}x{} mips={} fmt={} samples={} layout={layout} rflags={:#x}", u(0), u64::from_le_bytes(d[0x10..0x18].try_into().unwrap()), u(0x18), u16::from_le_bytes(d[0x1c..0x1e].try_into().unwrap()), u16::from_le_bytes(d[0x1e..0x20].try_into().unwrap()), u(0x20), u(0x24), u(0x30)));
        if layout == 2 || layout == 3 {
            flags.push("TILED-LAYOUT");
        }
        let mut hp = [0u32; 5];
        let mut hf = 0u32;
        let hr = unsafe {
            let f: unsafe extern "system" fn(*mut c_void, *mut [u32; 5], *mut u32) -> i32 = core::mem::transmute(vslot(r as *mut c_void, 14));
            f(r as *mut c_void, &mut hp, &mut hf)
        };
        if hr < 0 {
            out.push_str(&format!(" heap=none(hr={:#x})", hr as u32));
            flags.push("RESERVED");
        } else {
            out.push_str(&format!(" heapType={} cpuPage={} pool={} heapFlags={:#x}", hp[0], hp[1], hp[2], hf));
            if hp[0] == 4 && hp[1] == 1 {
                flags.push("CPU-NOT-AVAILABLE-CUSTOM");
            }
        }
    }
    match residency_of(r) {
        Some((true, t)) => {
            out.push_str(&format!(" evicted at {t}us"));
            flags.push("EVICTED");
        }
        Some((false, t)) => out.push_str(&format!(" made resident at {t}us")),
        None => {}
    }
    let va = crate::gpu_va::texture_va(r).map(|v| format!(" titleVA={v:#x}")).unwrap_or_default();
    out.push_str(&va);
    if !flags.is_empty() {
        out.push_str(&format!(" FLAG[{}]", flags.join(",")));
    }
    out
}

fn va_near(va: u64) -> String {
    let near = crate::gpu_va::near(va, 2).iter().map(|e| format!("res {:#x} [{:#x}, +{:#x})", e.res, e.start, e.len)).collect::<Vec<_>>().join(", ");
    let bt = crate::bindtrace::buffer_at(va).map_or("none".to_string(), |(base, res, size, heap)| format!("res {res:#x} base {base:#x} size {size:#x} heap {heap} off {:#x}", va - base));
    format!("bindtrace buffer: {bt}; gpu_va neighbours: {near}")
}

fn va_where(va: u64) -> String {
    if va == 0 {
        return "null".into();
    }
    match (crate::gpu_va::lookup_gpu(va), crate::gpu_va::lookup(va)) {
        (Some((e, off)), _) => format!("PC GPU VA, res {:#x} +{off:#x} of {:#x} ({:#x} left, buffer={})", e.res, e.len, e.len.saturating_sub(off), e.buffer),
        (None, Some((e, off))) => format!("title VA, res {:#x} +{off:#x} of {:#x} ({:#x} left, buffer={})", e.res, e.len, e.len.saturating_sub(off), e.buffer),
        (None, None) => "not inside any registered range".into(),
    }
}

fn sampler_words(b: &[u8], off: usize) -> String {
    b.get(off..off + 16).map(|s| s.chunks(4).map(|w| format!("{:08x}", u32::from_le_bytes([w[0], w[1], w[2], w[3]]))).collect::<Vec<_>>().join(" ")).unwrap_or_else(|| "n/a".into())
}

fn snap_text(d: &DrawSnap, full: bool) -> String {
    let mut s = format!("draw #{} list {:#x} vs={:016x} ps={:016x} {}\n", d.seq, d.list, d.vs, d.ps, d.text);
    for (param, reg, space, vis, set, va, bytes) in &d.roots {
        s.push_str(&format!("    root param {param} b{reg} space{space} vis={vis} {} va={va:#x} {}\n", if *set { "set-since-RS" } else { "NOT SET since SetGraphicsRootSignature" }, va_where(*va)));
        if *va != 0 {
            s.push_str(&format!("      {}
", va_near(*va)));
        }
        if *reg != 13 {
            continue;
        }
        match bytes {
            Some(b) => {
                let fnv = crate::shader_xlat::fnv1a(b);
                s.push_str(&format!("      b13 bytes fnv={fnv:016x}; S# at +0x530: {} | +0x540: {} | +0x550: {}\n", sampler_words(b, 0x530), sampler_words(b, 0x540), sampler_words(b, 0x550)));
                if full {
                    s.push_str(&hex_rows(b, BOUND_BYTES));
                }
            }
            None => s.push_str("      b13 bytes not CPU-readable at that VA\n"),
        }
    }
    s
}

pub fn draw(list: usize, text: &str, params: &[(usize, u32, u32, u32)], mask: u64, vas: &HashMap<u32, u64>) {
    if !on() {
        return;
    }
    let pso = with(|s| s.pso.get(&list).copied().unwrap_or(0));
    if pso == 0 {
        return;
    }
    let (vs, ps) = hashes_of(pso);
    if !targets().contains(&ps) {
        return;
    }
    let roots = params
        .iter()
        .map(|&(i, r, sp, vis)| {
            let set = i < 64 && mask & (1u64 << i) != 0;
            let va = vas.get(&(i as u32)).copied().unwrap_or(0);
            let bytes = if r == 13 && va != 0 { read_b13(va) } else { None };
            (i, r, sp, vis, set, va, bytes)
        })
        .collect();
    let d = DrawSnap { seq: seq(), list, ps, vs, text: format!("pso {pso:#x} {text} t={}us", now_us()), roots };
    let log = with(|s| {
        let q = s.draws.entry(ps).or_default();
        q.push_back(d.clone());
        while q.len() > KEEP_DRAWS {
            q.pop_front();
        }
        let n = s.logged.entry((ps, pso)).or_default();
        *n += 1;
        (*n <= LOG_DRAWS, *n == 1)
    });
    if log.1 {
        record("d3d12_xs", "hang_cap", &format!("pso {pso:#x} ps={ps:016x} desc {}", crate::device_wrap::pso_desc_text(pso).unwrap_or_else(|| "not in the PSO registry".into())));
    }
    if log.0 {
        record("d3d12_xs", "hang_cap", &snap_text(&d, true));
    }
    crate::rootlog::at_draw(list, ps, d.seq, text);
    unsafe { crate::hang_snap::before_draw(list, ps, pso, text, &d.roots, d.seq) };
    if log.1 {
        record("d3d12_xs", "hang_cap", &format!("pso bytes at draw of ps={ps:016x}: {}", dump_pso(pso, ps)));
        let b = unsafe { crate::bindtrace::dump(list, false, &res_text) };
        let flagged = b.lines().filter(|l| l.contains("FLAG[")).count();
        record("d3d12_xs", "hang_cap", &format!("descriptors at draw of ps={ps:016x} pso {pso:#x}: {flagged} flagged, Evict calls {} MakeResident calls {}\n{b}", RESIDENCY_CALLS[1].load(Ordering::Relaxed), RESIDENCY_CALLS[0].load(Ordering::Relaxed)));
    }
}

unsafe extern "system" fn set_pso(list: *mut c_void, pso: *mut c_void) {
    with(|s| {
        s.pso.insert(list as usize, pso as usize);
    });
    let f: unsafe extern "system" fn(*mut c_void, *mut c_void) = unsafe { core::mem::transmute(PREV_SET_PSO.load(Ordering::Relaxed)) };
    unsafe { f(list, pso) }
}

fn push_cmd(list: usize, text: String) {
    let sq = seq();
    with(|s| {
        s.cring.push_back((sq, list, text));
        while s.cring.len() > CMD_RING {
            s.cring.pop_front();
        }
    });
}

pub fn barrier_text(b: &[u8]) -> String {
    let u = |o: usize| u32::from_le_bytes(b[o..o + 4].try_into().unwrap());
    let q = |o: usize| u64::from_le_bytes(b[o..o + 8].try_into().unwrap());
    match u(0) {
        0 => format!("transition res {:#x} sub {:#x} {:#x}->{:#x} flags {:#x}", q(8), u(16), u(20), u(24), u(4)),
        1 => format!("aliasing {:#x}->{:#x} flags {:#x}", q(8), q(16), u(4)),
        2 => format!("uav res {:#x} flags {:#x}", q(8), u(4)),
        t => format!("type {t} flags {:#x} {:#x} {:#x}", u(4), q(8), q(16)),
    }
}

unsafe extern "system" fn resource_barrier(list: *mut c_void, n: u32, p: *const u8) {
    if !p.is_null() {
        for i in 0..(n as usize).min(64) {
            let b = unsafe { core::slice::from_raw_parts(p.add(i * 32), 32) };
            push_cmd(list as usize, format!("barrier {}", barrier_text(b)));
        }
    }
    let f: unsafe extern "system" fn(*mut c_void, u32, *const u8) = unsafe { core::mem::transmute(PREV_BARRIER.load(Ordering::Relaxed)) };
    unsafe { f(list, n, p) }
}

unsafe extern "system" fn copy_resource(list: *mut c_void, dst: *mut c_void, src: *mut c_void) {
    push_cmd(list as usize, format!("CopyResource dst {dst:p} src {src:p}"));
    let f: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) = unsafe { core::mem::transmute(PREV_COPY.load(Ordering::Relaxed)) };
    unsafe { f(list, dst, src) }
}

pub fn install_cmdlist(vt: &[AtomicUsize]) {
    if !on() || vt.len() <= XBOX_SLOT_RESOURCE_BARRIER {
        return;
    }
    PREV_SET_PSO.store(vt[XBOX_SLOT_SET_PSO].load(Ordering::Relaxed), Ordering::Relaxed);
    PREV_BARRIER.store(vt[XBOX_SLOT_RESOURCE_BARRIER].load(Ordering::Relaxed), Ordering::Relaxed);
    PREV_COPY.store(vt[XBOX_SLOT_COPY_RESOURCE].load(Ordering::Relaxed), Ordering::Relaxed);
    vt[XBOX_SLOT_SET_PSO].store(set_pso as usize, Ordering::Relaxed);
    vt[XBOX_SLOT_RESOURCE_BARRIER].store(resource_barrier as usize, Ordering::Relaxed);
    vt[XBOX_SLOT_COPY_RESOURCE].store(copy_resource as usize, Ordering::Relaxed);
    record("d3d12_xs", "hang_cap", "command-list SetPipelineState/ResourceBarrier/CopyResource capture installed");
    crate::hang_snap::install_cmdlist(vt);
    crate::rootlog::install(vt);
}

unsafe fn queue_type(queue: *mut c_void) -> u32 {
    if let Some(t) = with(|s| s.queues.get(&(queue as usize)).copied()) {
        return t;
    }
    let t = unsafe { crate::ecl_probe::queue_desc(queue, REAL_GET_DESC.load(Ordering::Relaxed)) }[0];
    with(|s| {
        s.queues.insert(queue as usize, t);
    });
    t
}

fn push_q(text: String) {
    let line = format!("#{} t={}us {text}", seq(), now_us());
    with(|s| {
        s.qring.push_back(line);
        while s.qring.len() > QUEUE_RING {
            s.qring.pop_front();
        }
    });
}

pub unsafe fn note_signal(queue: *mut c_void, fence: *mut c_void, value: u64, who: &str) {
    if !on() {
        return;
    }
    let t = unsafe { queue_type(queue) };
    with(|s| {
        s.fences.insert(fence as usize, value);
    });
    push_q(format!("{who} Signal queue {queue:p} type {t} fence {fence:p} value {value}"));
}

pub unsafe fn note_wait(queue: *mut c_void, fence: *mut c_void, value: u64, who: &str) {
    if !on() {
        return;
    }
    let t = unsafe { queue_type(queue) };
    with(|s| {
        s.fences.entry(fence as usize).or_insert(0);
    });
    push_q(format!("{who} Wait queue {queue:p} type {t} fence {fence:p} value {value}"));
}

pub unsafe fn note_exec(queue: *mut c_void, title: &[*mut c_void], real: &[*mut c_void]) {
    if !on() {
        return;
    }
    let t = unsafe { queue_type(queue) };
    push_q(format!("ExecuteCommandLists queue {queue:p} type {t} title {title:?} real {real:?}"));
}

unsafe extern "system" fn q_signal(queue: *mut c_void, fence: *mut c_void, value: u64) -> i32 {
    unsafe { note_signal(queue, fence, value, "pc slot 14") };
    let f: unsafe extern "system" fn(*mut c_void, *mut c_void, u64) -> i32 = unsafe { core::mem::transmute(REAL_SIGNAL.load(Ordering::Relaxed)) };
    unsafe { f(queue, fence, value) }
}

unsafe extern "system" fn q_wait(queue: *mut c_void, fence: *mut c_void, value: u64) -> i32 {
    unsafe { note_wait(queue, fence, value, "pc slot 15") };
    let f: unsafe extern "system" fn(*mut c_void, *mut c_void, u64) -> i32 = unsafe { core::mem::transmute(REAL_WAIT.load(Ordering::Relaxed)) };
    unsafe { f(queue, fence, value) }
}

pub fn install_queue(queue_vt: &[AtomicUsize], real: &dyn Fn(usize) -> usize) {
    if !on() || queue_vt.len() <= PC_QUEUE_GET_DESC {
        return;
    }
    REAL_SIGNAL.store(real(PC_QUEUE_SIGNAL), Ordering::Relaxed);
    REAL_WAIT.store(real(PC_QUEUE_WAIT), Ordering::Relaxed);
    REAL_GET_DESC.store(real(PC_QUEUE_GET_DESC), Ordering::Relaxed);
    queue_vt[PC_QUEUE_SIGNAL].store(q_signal as usize, Ordering::Relaxed);
    queue_vt[PC_QUEUE_WAIT].store(q_wait as usize, Ordering::Relaxed);
    record("d3d12_xs", "hang_cap", "queue Signal/Wait capture installed");
}

pub fn signal_fn(real: usize) -> usize {
    if on() && REAL_SIGNAL.load(Ordering::Relaxed) == real && real != 0 { q_signal as usize } else { real }
}

pub fn wait_fn(real: usize) -> usize {
    if on() && REAL_WAIT.load(Ordering::Relaxed) == real && real != 0 { q_wait as usize } else { real }
}

pub fn dump_on_removal() {
    if !on() {
        return;
    }
    crate::hang_snap::dump();
    crate::rootlog::dump();
    let (q, fences, draws, cmds) = with(|s| {
        let draws: Vec<DrawSnap> = s.draws.values().flat_map(|d| d.iter().cloned()).collect();
        (s.qring.iter().cloned().collect::<Vec<_>>(), s.fences.iter().map(|(k, v)| (*k, *v)).collect::<Vec<_>>(), draws, s.cring.iter().cloned().collect::<Vec<_>>())
    });
    let mut draws = draws;
    draws.sort_by_key(|d| d.seq);
    for d in &draws {
        let now: Vec<Option<Vec<u8>>> = d.roots.iter().map(|r| if r.1 == 13 && r.5 != 0 { read_b13(r.5) } else { None }).collect();
        let changed: Vec<String> = d.roots.iter().zip(&now).filter(|(r, n)| r.1 == 13 && r.6.is_some() && r.6 != **n).map(|(r, _)| format!("param {}", r.0)).collect();
        let note = if changed.is_empty() { "b13 bytes unchanged since recording".to_string() } else { format!("b13 bytes CHANGED since recording at {}", changed.join(", ")) };
        record("d3d12_xs", "hang_cap.removal", &format!("kept {}{note}", snap_text(d, true)));
    }
    let lists: Vec<usize> = draws.iter().map(|d| d.list).collect();
    for l in lists.iter().copied().collect::<std::collections::BTreeSet<_>>() {
        let last_draw = draws.iter().filter(|d| d.list == l).map(|d| d.seq).max().unwrap_or(0);
        let mine: Vec<String> = cmds.iter().filter(|c| c.1 == l && c.0 < last_draw).rev().take(48).map(|c| format!("#{} {}", c.0, c.2)).collect();
        record("d3d12_xs", "hang_cap.removal", &format!("list {l:#x}: last {} barrier/copy ops before its last captured draw #{last_draw} (newest first):\n    {}", mine.len(), mine.join("\n    ")));
    }
    let tail: Vec<String> = cmds.iter().rev().take(96).map(|c| format!("#{} list {:#x} {}", c.0, c.1, c.2)).collect();
    record("d3d12_xs", "hang_cap.removal", &format!("last {} barrier/copy ops on any list (newest first):\n    {}", tail.len(), tail.join("\n    ")));
    let qt: Vec<String> = q.iter().rev().take(256).cloned().collect();
    record("d3d12_xs", "hang_cap.removal", &format!("last {} queue events (newest first):\n    {}", qt.len(), qt.join("\n    ")));
    let mut fl = Vec::new();
    for (f, last) in fences {
        let done = unsafe {
            let g: unsafe extern "system" fn(*mut c_void) -> u64 = core::mem::transmute(vslot(f as *mut c_void, FENCE_GET_COMPLETED));
            g(f as *mut c_void)
        };
        fl.push(format!("fence {f:#x} last signal {last} completed {done}{}", if done < last { " PENDING" } else { "" }));
    }
    record("d3d12_xs", "hang_cap.removal", &format!("{} fences:\n    {}", fl.len(), fl.join("\n    ")));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore]
    fn dump_f861_pc_root_signature() {
        let path = "D:/xbox-pc-port/emulator/input/shader_cache/titles/GbxGame/src/f861c56a34758d84_0200000000000000.xbox";
        let raw = std::fs::read(path).unwrap();
        let show = |tag: &str, b: &[u8]| {
            let Some((o, n)) = crate::console_slots_mesh::rts0_range(b) else { println!("{tag}: no RTS0"); return };
            let r = &b[o..o + n];
            let u = |x: usize| u32::from_le_bytes(r[x..x + 4].try_into().unwrap());
            let (ver, np, po) = (u(0), u(4) as usize, u(8) as usize);
            println!("{tag}: ver={ver} params={np} flags={:#x} len={}", u(20), b.len());
            for i in 0..np {
                let (ty, vis, pl) = (u(po + i * 12), u(po + i * 12 + 4), u(po + i * 12 + 8) as usize);
                let extra = if (2..=4).contains(&ty) { format!("reg={} space={}{}", u(pl), u(pl + 4), if ver >= 2 { format!(" flags={:#x}", u(pl + 8)) } else { String::new() }) } else if ty == 1 { format!("reg={} space={} n={}", u(pl), u(pl + 4), u(pl + 8)) } else { format!("ranges={}", u(pl)) };
                println!("  param {i} type={ty} vis={vis} {extra}");
            }
        };
        let start = raw.windows(4).position(|w| w == b"DXBC").unwrap_or(0);
        let blob = raw[start..].to_vec();
        show("embedded", &blob);
        let patched = crate::cbv_raw::patch_blob(&blob).unwrap().map(|(b, _)| b).unwrap_or(blob.clone());
        show("after cbv_raw", &patched);
        let pc = unsafe { crate::rs_xbox::pc_blob_raw(patched.as_ptr(), patched.len()) }.unwrap_or(patched.clone());
        show("after rs_xbox", &pc);
        let g = unsafe { crate::gds::patch_root_signature(pc.as_ptr(), pc.len()) }.map(|(b, _)| b).unwrap_or(pc.clone());
        show("after gds", &g);
        if let Ok(out) = std::env::var("TOMBOX_RS_OUT") {
            std::fs::write(out, &g).unwrap();
        }
    }

    #[test]
    fn pso_desc_text_reads_the_graphics_desc_fields() {
        let mut d = vec![0u8; 0x290];
        d[0x240..0x244].copy_from_slice(&1u32.to_le_bytes());
        d[0x244..0x248].copy_from_slice(&26u32.to_le_bytes());
        d[0x264..0x268].copy_from_slice(&45u32.to_le_bytes());
        d[0x268..0x26c].copy_from_slice(&4u32.to_le_bytes());
        d[0x1e8..0x1ec].copy_from_slice(&8u32.to_le_bytes());
        d[0x80 + 36] = 0xf;
        let t = crate::device_wrap::desc_text(&d);
        assert!(t.contains("rt0(fmt=26 "), "{t}");
        assert!(t.contains("mask=0xf)"), "{t}");
        assert!(t.contains("dsv=45 sample=4/0"), "{t}");
        assert!(t.contains("forced=8 "), "{t}");
    }

    #[test]
    fn res_text_flags_an_unreadable_object() {
        let t = res_text(0);
        assert!(t.contains("FLAG[UNREADABLE-OBJECT]"), "{t}");
        assert!(residency_of(0x1234).is_none());
    }

    #[test]
    fn mode_two_only_and_default_targets() {
        assert!(mode_from(Some("2")));
        assert!(!mode_from(Some("1")));
        assert!(!mode_from(None));
        assert_eq!(parse_targets(None), DEFAULT_PS.to_vec());
        assert_eq!(parse_targets(Some("f861c56a34758d84, 1")), vec![0xf861c56a34758d84, 1]);
    }

    #[test]
    fn hex_rows_marks_the_bound_and_barriers_decode() {
        let b: Vec<u8> = (0..0x480u32).flat_map(|i| (i as u8).to_le_bytes()).collect();
        let t = hex_rows(&b, BOUND_BYTES);
        assert!(t.lines().nth(BOUND_BYTES / 16).unwrap().ends_with("<-- bound size"));
        assert_eq!(t.lines().count(), 0x48);
        let mut br = [0u8; 32];
        br[8..16].copy_from_slice(&0x1234u64.to_le_bytes());
        br[16..20].copy_from_slice(&0xffff_ffffu32.to_le_bytes());
        br[20..24].copy_from_slice(&4u32.to_le_bytes());
        br[24..28].copy_from_slice(&0x80u32.to_le_bytes());
        assert_eq!(barrier_text(&br), "transition res 0x1234 sub 0xffffffff 0x4->0x80 flags 0x0");
    }

    #[test]
    fn read_mem_reads_own_memory_and_refuses_null_page() {
        let v = vec![1u8, 2, 3, 4];
        assert_eq!(read_mem(v.as_ptr() as u64, 4), Some(v.clone()));
        assert_eq!(read_mem(0x10, 4), None);
    }
}
