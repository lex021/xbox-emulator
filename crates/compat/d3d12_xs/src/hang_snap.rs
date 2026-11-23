use compat_common::record;
use core::ffi::c_void;
use core::sync::atomic::{AtomicUsize, Ordering};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

const SLOTS: usize = 32;
const SLOT_BYTES: u64 = 0x10_0000;
const OFF_MARK: u64 = 0;
const OFF_PS: u64 = 0x100;
const OFF_VS: u64 = 0x800;
const OFF_IB: u64 = 0x1000;
const IB_CAP: u64 = 0x1_0000;
const OFF_VB: u64 = 0x1_1000;
const VB_CAP: u64 = SLOT_BYTES - OFF_VB;
const B13_BYTES: u64 = 0x700;
const PC_LIST_GET_DEVICE: usize = 7;
const PC_LIST_COPY_BUFFER_REGION: usize = 15;
const PC_LIST_RESOURCE_BARRIER: usize = 26;
const PC_LIST_WRITE_BUFFER_IMMEDIATE: usize = 66;
const XBOX_SLOT_ROOT_CONSTANT: usize = 38;
const XBOX_SLOT_ROOT_CONSTANTS: usize = 40;
const COPY_SOURCE: u32 = 0x800;
const WRITE_STATES: u32 = 0x4 | 0x8 | 0x10 | 0x100 | 0x400 | 0x1000;
const MARK_BEFORE: u32 = 0xA000_0000;
const MARK_AFTER_IN: u32 = 0xB000_0000;
const MARK_AFTER_OUT: u32 = 0xC000_0000;

#[repr(C)]
struct Iid(u32, u16, u16, [u8; 8]);
const IID_DEVICE: Iid = Iid(0x189819f1, 0x1db6, 0x4b57, [0xbe, 0x54, 0x18, 0x21, 0x33, 0x9b, 0x85, 0xf7]);
const IID_RESOURCE: Iid = Iid(0x696442be, 0xa72e, 0x4059, [0xbc, 0x79, 0x5b, 0x5c, 0x98, 0x04, 0x0f, 0xad]);
const IID_LIST2: Iid = Iid(0x38c3e585, 0xff17, 0x412c, [0x91, 0x50, 0x4f, 0xc6, 0xf9, 0xd7, 0x2a, 0x28]);

struct Copy {
    name: &'static str,
    off: u64,
    len: u64,
    how: String,
}

struct Snap {
    seq: u64,
    slot: usize,
    list: usize,
    ps: u64,
    pso: usize,
    text: String,
    copies: Vec<Copy>,
    cpu_ps: Option<Vec<u8>>,
    cpu_vs: Option<Vec<u8>>,
    ps_va: u64,
    vs_va: u64,
    consts: String,
    ib_fmt: u32,
    vb_stride: u32,
    after: bool,
}

#[derive(Default)]
struct State {
    rb: usize,
    rb_va: u64,
    mapped: usize,
    next: usize,
    snaps: Vec<Snap>,
    pending: HashMap<usize, usize>,
    consts: HashMap<usize, HashMap<u32, Vec<Option<u32>>>>,
    list2: HashMap<usize, bool>,
    failed: bool,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);
static PREV_CONST: AtomicUsize = AtomicUsize::new(0);
static PREV_CONSTS: AtomicUsize = AtomicUsize::new(0);

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut g = STATE.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(State::default))
}

pub fn mode_from(v: Option<&str>) -> bool {
    v.map(|s| s.trim() == "1").unwrap_or(false)
}

pub fn on() -> bool {
    static E: OnceLock<bool> = OnceLock::new();
    *E.get_or_init(|| {
        let on = crate::hang_cap::on() && mode_from(std::env::var("TOMBOX_HANGCAP_SNAP").ok().as_deref());
        if on {
            record("d3d12_xs", "hang_snap", "TOMBOX_HANGCAP_SNAP=1: GPU-time copies of b13/IB/VB plus MARKER writes are added around every captured draw (diagnostic; nothing is skipped)");
        }
        on
    })
}

pub fn ps_from(v: Option<&str>) -> u64 {
    v.and_then(|s| u64::from_str_radix(s.trim().trim_start_matches("0x"), 16).ok()).unwrap_or(0xf861c56a34758d84)
}

pub(crate) fn snap_ps() -> u64 {
    static P: OnceLock<u64> = OnceLock::new();
    *P.get_or_init(|| ps_from(std::env::var("TOMBOX_HANGCAP_SNAP_PS").ok().as_deref()))
}

unsafe fn vslot(o: *mut c_void, i: usize) -> usize {
    unsafe { *(*(o as *const *const usize)).add(i) }
}

fn note_consts(list: usize, index: u32, off: u32, vals: &[u32]) {
    with(|s| {
        let v = s.consts.entry(list).or_default().entry(index).or_default();
        for (k, x) in vals.iter().enumerate() {
            let at = off as usize + k;
            if at >= 64 {
                break;
            }
            if v.len() <= at {
                v.resize(at + 1, None);
            }
            v[at] = Some(*x);
        }
    });
}

unsafe extern "system" fn root_constant(list: *mut c_void, index: u32, value: u32, off: u32) {
    note_consts(list as usize, index, off, &[value]);
    let f: unsafe extern "system" fn(*mut c_void, u32, u32, u32) = unsafe { core::mem::transmute(PREV_CONST.load(Ordering::Relaxed)) };
    unsafe { f(list, index, value, off) }
}

unsafe extern "system" fn root_constants(list: *mut c_void, index: u32, n: u32, src: *const u32, off: u32) {
    if !src.is_null() && n <= 64 {
        note_consts(list as usize, index, off, unsafe { core::slice::from_raw_parts(src, n as usize) });
    }
    let f: unsafe extern "system" fn(*mut c_void, u32, u32, *const u32, u32) = unsafe { core::mem::transmute(PREV_CONSTS.load(Ordering::Relaxed)) };
    unsafe { f(list, index, n, src, off) }
}

pub fn install_cmdlist(vt: &[AtomicUsize]) {
    if !on() || vt.len() <= XBOX_SLOT_ROOT_CONSTANTS {
        return;
    }
    PREV_CONST.store(vt[XBOX_SLOT_ROOT_CONSTANT].load(Ordering::Relaxed), Ordering::Relaxed);
    PREV_CONSTS.store(vt[XBOX_SLOT_ROOT_CONSTANTS].load(Ordering::Relaxed), Ordering::Relaxed);
    vt[XBOX_SLOT_ROOT_CONSTANT].store(root_constant as *const () as usize, Ordering::Relaxed);
    vt[XBOX_SLOT_ROOT_CONSTANTS].store(root_constants as *const () as usize, Ordering::Relaxed);
    record("d3d12_xs", "hang_snap", "graphics root 32-bit constant capture installed");
}

pub fn consts_text(map: &HashMap<u32, Vec<Option<u32>>>) -> String {
    let mut keys: Vec<&u32> = map.keys().collect();
    keys.sort();
    keys.iter()
        .map(|k| {
            let v: Vec<String> = map[k].iter().map(|x| x.map_or("--------".to_string(), |x| format!("{x:08x}"))).collect();
            format!("param {k}: [{}]", v.join(" "))
        })
        .collect::<Vec<_>>()
        .join("; ")
}

unsafe fn ensure_ring(list: *mut c_void) -> Option<(usize, u64, usize)> {
    if let Some(r) = with(|s| if s.failed { Some(None) } else if s.rb != 0 { Some(Some((s.rb, s.rb_va, s.mapped))) } else { None }) {
        return r;
    }
    let mut dev: *mut c_void = core::ptr::null_mut();
    let hr = unsafe {
        let f: unsafe extern "system" fn(*mut c_void, *const Iid, *mut *mut c_void) -> i32 = core::mem::transmute(vslot(list, PC_LIST_GET_DEVICE));
        f(list, &IID_DEVICE, &mut dev)
    };
    if hr < 0 || dev.is_null() {
        record("d3d12_xs", "hang_snap", &format!("GetDevice on the list failed hr={hr:#x}; snapshots disabled"));
        with(|s| s.failed = true);
        return None;
    }
    let hp: [u32; 5] = [3, 0, 0, 1, 1];
    let mut d = [0u8; 0x38];
    d[0..4].copy_from_slice(&1u32.to_le_bytes());
    d[0x10..0x18].copy_from_slice(&(SLOTS as u64 * SLOT_BYTES).to_le_bytes());
    d[0x18..0x1c].copy_from_slice(&1u32.to_le_bytes());
    d[0x1c..0x1e].copy_from_slice(&1u16.to_le_bytes());
    d[0x1e..0x20].copy_from_slice(&1u16.to_le_bytes());
    d[0x24..0x28].copy_from_slice(&1u32.to_le_bytes());
    d[0x2c..0x30].copy_from_slice(&1u32.to_le_bytes());
    let mut rb: *mut c_void = core::ptr::null_mut();
    let hr = unsafe {
        let f: unsafe extern "system" fn(*mut c_void, *const u32, u32, *const u8, u32, *const c_void, *const Iid, *mut *mut c_void) -> i32 = core::mem::transmute(vslot(dev, 27));
        let r = f(dev, hp.as_ptr(), 0, d.as_ptr(), 0x400, core::ptr::null(), &IID_RESOURCE, &mut rb);
        let rel: unsafe extern "system" fn(*mut c_void) -> u32 = core::mem::transmute(vslot(dev, 2));
        rel(dev);
        r
    };
    if hr < 0 || rb.is_null() {
        record("d3d12_xs", "hang_snap", &format!("readback ring creation failed hr={hr:#x}; snapshots disabled"));
        with(|s| s.failed = true);
        return None;
    }
    let mut p: *mut c_void = core::ptr::null_mut();
    let range = [0usize, 0usize];
    let (hr, va) = unsafe {
        let m: unsafe extern "system" fn(*mut c_void, u32, *const usize, *mut *mut c_void) -> i32 = core::mem::transmute(vslot(rb, 8));
        let g: unsafe extern "system" fn(*mut c_void) -> u64 = core::mem::transmute(vslot(rb, 11));
        let _ = range;
        (m(rb, 0, core::ptr::null(), &mut p), g(rb))
    };
    if hr < 0 || p.is_null() {
        record("d3d12_xs", "hang_snap", &format!("readback ring Map failed hr={hr:#x}; snapshots disabled"));
        with(|s| s.failed = true);
        return None;
    }
    unsafe { core::ptr::write_bytes(p as *mut u8, 0, SLOTS * SLOT_BYTES as usize) };
    record("d3d12_xs", "hang_snap", &format!("readback ring {rb:p} va={va:#x} {SLOTS} slots x {SLOT_BYTES:#x} bytes, persistently mapped at {p:p}"));
    with(|s| {
        s.rb = rb as usize;
        s.rb_va = va;
        s.mapped = p as usize;
    });
    Some((rb as usize, va, p as usize))
}

unsafe fn has_list2(list: *mut c_void) -> bool {
    if let Some(v) = with(|s| s.list2.get(&(list as usize)).copied()) {
        return v;
    }
    let mut l2: *mut c_void = core::ptr::null_mut();
    let ok = unsafe {
        let qi: unsafe extern "system" fn(*mut c_void, *const Iid, *mut *mut c_void) -> i32 = core::mem::transmute(vslot(list, 0));
        let hr = qi(list, &IID_LIST2, &mut l2);
        if hr >= 0 && !l2.is_null() {
            let rel: unsafe extern "system" fn(*mut c_void) -> u32 = core::mem::transmute(vslot(l2, 2));
            rel(l2);
            true
        } else {
            false
        }
    };
    with(|s| s.list2.insert(list as usize, ok));
    ok
}

unsafe fn real_fn(slot: usize) -> usize {
    crate::cmdlist_wrap::real_list_slot(slot)
}

unsafe fn mark(list: *mut c_void, va: u64, value: u32, mode: u32) {
    if !unsafe { has_list2(list) } {
        return;
    }
    let p: [u64; 2] = [va, value as u64];
    let m = [mode];
    unsafe {
        let f: unsafe extern "system" fn(*mut c_void, u32, *const u64, *const u32) = core::mem::transmute(real_fn(PC_LIST_WRITE_BUFFER_IMMEDIATE));
        f(list, 1, p.as_ptr(), m.as_ptr());
    }
}

unsafe fn barrier(list: *mut c_void, res: usize, before: u32, after: u32) {
    let b = [0u64, res as u64, 0xffff_ffffu64 | ((before as u64) << 32), after as u64];
    unsafe {
        let f: unsafe extern "system" fn(*mut c_void, u32, *const u64) = core::mem::transmute(real_fn(PC_LIST_RESOURCE_BARRIER));
        f(list, 1, b.as_ptr());
    }
}

pub fn plan_barrier(heap: u32, state: u32) -> Result<Option<(u32, u32)>, String> {
    if heap == 2 || heap == 3 {
        return Ok(None);
    }
    if state & WRITE_STATES != 0 {
        return Err(format!("tracked state {state:#x} is a write state"));
    }
    if state == 0 || state & COPY_SOURCE != 0 {
        return Ok(None);
    }
    Ok(Some((state, state | COPY_SOURCE)))
}

fn source_of(va: u64) -> Option<(usize, u64, u64, u32)> {
    if let Some((base, res, size, heap)) = crate::bindtrace::buffer_at(va) {
        return Some((res, va - base, size.saturating_sub(va - base), heap));
    }
    if let Some((e, off)) = crate::gpu_va::lookup_gpu(va) {
        return Some((e.res, off, e.len.saturating_sub(off), 0));
    }
    None
}

unsafe fn copy_va(list: *mut c_void, rb: usize, dst: u64, va: u64, want: u64, name: &'static str) -> Copy {
    let Some((res, off, left, heap)) = source_of(va) else {
        return Copy { name, off: dst, len: 0, how: format!("va {va:#x} not in any tracked buffer; not copied") };
    };
    let n = want.min(left);
    if n == 0 {
        return Copy { name, off: dst, len: 0, how: format!("va {va:#x} res {res:#x} +{off:#x}: nothing left in the buffer") };
    }
    let state = crate::rtcap::state_of(res).unwrap_or_else(|| crate::gpu_va::state(res));
    let plan = match plan_barrier(heap, state) {
        Ok(p) => p,
        Err(why) => return Copy { name, off: dst, len: 0, how: format!("va {va:#x} res {res:#x}: {why}; not copied") },
    };
    unsafe {
        if let Some((b, a)) = plan {
            barrier(list, res, b, a);
        }
        let f: unsafe extern "system" fn(*mut c_void, *mut c_void, u64, *mut c_void, u64, u64) = core::mem::transmute(real_fn(PC_LIST_COPY_BUFFER_REGION));
        f(list, rb as *mut c_void, dst, res as *mut c_void, off, n);
        if let Some((b, a)) = plan {
            barrier(list, res, a, b);
        }
    }
    Copy { name, off: dst, len: n, how: format!("va {va:#x} res {res:#x} +{off:#x} heap {heap} state {state:#x} barrier {}", plan.map_or("none".to_string(), |(b, a)| format!("{b:#x}->{a:#x}->{b:#x}"))) }
}

pub fn draw_args(text: &str) -> Option<(u32, u32, u32, i32, u32)> {
    let t = text.split("DrawIndexedInstanced(").nth(1)?;
    let get = |k: &str| -> Option<i64> { t.split(&format!("{k}=")).nth(1)?.split(|c: char| c == ' ' || c == ')').next()?.parse().ok() };
    Some((get("idx")? as u32, get("inst")? as u32, get("startIdx")? as u32, get("baseVtx")? as i32, get("startInst")? as u32))
}

pub unsafe fn before_draw(list: usize, ps: u64, pso: usize, text: &str, roots: &[(usize, u32, u32, u32, bool, u64, Option<Vec<u8>>)], seq: u64) {
    if !on() || list == 0 || ps != snap_ps() {
        return;
    }
    let l = list as *mut c_void;
    let Some((rb, rb_va, _)) = (unsafe { ensure_ring(l) }) else { return };
    let slot = with(|s| {
        let k = s.next % SLOTS;
        s.next += 1;
        k
    });
    let base = slot as u64 * SLOT_BYTES;
    unsafe { mark(l, rb_va + base + OFF_MARK, MARK_BEFORE | (seq as u32 & 0x0fff_ffff), 2) };
    let ps_root = roots.iter().find(|r| r.1 == 13 && r.3 == 5 && r.5 != 0);
    let vs_root = roots.iter().find(|r| r.1 == 13 && r.3 == 1 && r.5 != 0);
    let mut copies = Vec::new();
    if let Some(r) = ps_root {
        copies.push(unsafe { copy_va(l, rb, base + OFF_PS, r.5, B13_BYTES, "ps_b13") });
    }
    if let Some(r) = vs_root {
        copies.push(unsafe { copy_va(l, rb, base + OFF_VS, r.5, B13_BYTES, "vs_b13") });
    }
    let args = draw_args(text);
    let ib = crate::console_slots::bound_ib(list);
    let mut ib_fmt = 0;
    if let (Some(ib), Some((idx, _, start, _, _))) = (ib, args) {
        ib_fmt = ib.format;
        let isz = if ib.format == 57 { 2u64 } else { 4 };
        copies.push(unsafe { copy_va(l, rb, base + OFF_IB, ib.va + start as u64 * isz, (idx as u64 * isz).min(IB_CAP), "ib") });
    }
    let vbs = crate::rtcap::bound_vbs(list);
    let mut vb_stride = 0;
    if let Some(&(_, va, size, stride)) = vbs.iter().find(|v| v.0 == 0) {
        vb_stride = stride;
        copies.push(unsafe { copy_va(l, rb, base + OFF_VB, va, (size as u64).min(VB_CAP), "vb0") });
    }
    let consts = with(|s| s.consts.get(&list).map(consts_text).unwrap_or_else(|| "none recorded".into()));
    let cpu = |r: Option<&(usize, u32, u32, u32, bool, u64, Option<Vec<u8>>)>| r.and_then(|r| r.6.clone());
    let snap = Snap {
        seq,
        slot,
        list,
        ps,
        pso,
        text: text.to_string(),
        copies,
        cpu_ps: cpu(ps_root),
        cpu_vs: cpu(vs_root),
        ps_va: ps_root.map_or(0, |r| r.5),
        vs_va: vs_root.map_or(0, |r| r.5),
        consts,
        ib_fmt,
        vb_stride,
        after: false,
    };
    with(|s| {
        s.snaps.retain(|x| x.slot != slot);
        s.snaps.push(snap);
        s.pending.insert(list, slot);
    });
}

pub unsafe fn after_draw(list: usize) {
    if !on() {
        return;
    }
    let Some((slot, va, seq)) = with(|s| {
        let slot = s.pending.remove(&list)?;
        let seq = s.snaps.iter_mut().find(|x| x.slot == slot).map(|x| {
            x.after = true;
            x.seq
        })?;
        Some((slot, s.rb_va, seq))
    }) else {
        return;
    };
    let base = va + slot as u64 * SLOT_BYTES;
    let l = list as *mut c_void;
    unsafe {
        mark(l, base + OFF_MARK + 4, MARK_AFTER_IN | (seq as u32 & 0x0fff_ffff), 1);
        mark(l, base + OFF_MARK + 8, MARK_AFTER_OUT | (seq as u32 & 0x0fff_ffff), 2);
    }
}

pub fn diff_text(a: &[u8], b: &[u8]) -> String {
    let n = a.len().min(b.len());
    let rows: Vec<usize> = (0..n / 16).filter(|r| a[r * 16..r * 16 + 16] != b[r * 16..r * 16 + 16]).collect();
    if rows.is_empty() {
        return format!("identical ({n} bytes)");
    }
    let show: Vec<String> = rows.iter().take(12).map(|r| format!("row {r}")).collect();
    format!("{} of {} rows differ: {}{}", rows.len(), n / 16, show.join(", "), if rows.len() > 12 { " ..." } else { "" })
}

pub fn marker_text(words: [u32; 3], seq: u64) -> String {
    let s = seq as u32 & 0x0fff_ffff;
    let w = |v: u32, tag: u32, name: &str| if v == tag | s { format!("{name}=SET") } else if v == 0 { format!("{name}=unset") } else { format!("{name}=other({v:#x})") };
    format!("{} {} {}", w(words[0], MARK_BEFORE, "before(all prior done)"), w(words[1], MARK_AFTER_IN, "draw_started"), w(words[2], MARK_AFTER_OUT, "draw_done"))
}

pub fn dump() {
    if !on() {
        return;
    }
    let (mapped, snaps) = with(|s| (s.mapped, s.snaps.iter().map(|x| x.seq).collect::<Vec<_>>()));
    if mapped == 0 {
        record("d3d12_xs", "hang_snap", "no readback ring; nothing to dump");
        return;
    }
    let dir = crate::hang_cap::dump_dir().join("snap");
    let _ = std::fs::create_dir_all(&dir);
    let mut order = snaps;
    order.sort();
    for seq in order {
        let text = with(|s| {
            let x = s.snaps.iter().find(|x| x.seq == seq)?;
            let base = x.slot as u64 * SLOT_BYTES;
            let read = |off: u64, n: u64| unsafe { core::slice::from_raw_parts((mapped as u64 + base + off) as *const u8, n as usize).to_vec() };
            let m = read(OFF_MARK, 12);
            let words = [0, 4, 8].map(|o| u32::from_le_bytes(m[o..o + 4].try_into().unwrap()));
            let mut out = format!("draw #{} list {:#x} ps={:016x} pso {:#x} {} | {} | b1/root consts: {} | ib fmt {} vb stride {} | after-markers recorded {}", x.seq, x.list, x.ps, x.pso, x.text, marker_text(words, x.seq), x.consts, x.ib_fmt, x.vb_stride, x.after);
            for c in &x.copies {
                let bytes = read(c.off, c.len);
                let _ = std::fs::write(dir.join(format!("draw{}_{}.bin", x.seq, c.name)), &bytes);
                out.push_str(&format!("\n    {} {} bytes from {}", c.name, c.len, c.how));
                let cpu = match c.name {
                    "ps_b13" => x.cpu_ps.as_ref(),
                    "vs_b13" => x.cpu_vs.as_ref(),
                    _ => None,
                };
                if let Some(cpu) = cpu {
                    out.push_str(&format!(" | GPU-time vs CPU-at-record: {}", diff_text(&bytes, cpu)));
                    let _ = std::fs::write(dir.join(format!("draw{}_{}_cpu.bin", x.seq, c.name)), cpu);
                }
                if c.name == "ib" && c.len > 0 {
                    let isz = if x.ib_fmt == 57 { 2 } else { 4 };
                    let idx: Vec<u32> = bytes.chunks_exact(isz).map(|b| if isz == 2 { u16::from_le_bytes([b[0], b[1]]) as u32 } else { u32::from_le_bytes(b.try_into().unwrap()) }).collect();
                    out.push_str(&format!(" | indices min {} max {}", idx.iter().min().unwrap_or(&0), idx.iter().max().unwrap_or(&0)));
                }
            }
            let now = |va: u64| if va == 0 { None } else { unsafe { crate::bindtrace::read_va(va, B13_BYTES as usize) } };
            if let (Some(n), Some(c)) = (now(x.ps_va), x.cpu_ps.as_ref()) {
                out.push_str(&format!("\n    ps_b13 CPU now vs CPU-at-record: {}", diff_text(&n, c)));
            }
            if let (Some(n), Some(c)) = (now(x.vs_va), x.cpu_vs.as_ref()) {
                out.push_str(&format!("\n    vs_b13 CPU now vs CPU-at-record: {}", diff_text(&n, c)));
            }
            Some(out)
        });
        if let Some(t) = text {
            record("d3d12_xs", "hang_snap.removal", &t);
        }
    }
    record("d3d12_xs", "hang_snap.removal", &format!("snapshot files in {}", dir.display()));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snap_knob_and_draw_args() {
        assert!(mode_from(Some("1")));
        assert!(!mode_from(Some("0")));
        assert!(!mode_from(None));
        assert_eq!(draw_args("rs 0x1 DrawIndexedInstanced(idx=312 inst=2 startIdx=48 baseVtx=-3 startInst=1)"), Some((312, 2, 48, -3, 1)));
        assert_eq!(ps_from(None), 0xf861c56a34758d84);
        assert_eq!(ps_from(Some("8f215c725a248afa")), 0x8f215c725a248afa);
        assert_eq!(draw_args("DrawInstanced(vtx=3 inst=1 startVtx=0 startInst=0)"), None);
    }

    #[test]
    fn barrier_plan_never_touches_write_states() {
        assert_eq!(plan_barrier(2, 0xac3), Ok(None));
        assert_eq!(plan_barrier(1, 0), Ok(None));
        assert_eq!(plan_barrier(1, 0x2), Ok(Some((0x2, 0x802))));
        assert_eq!(plan_barrier(0, 0x800), Ok(None));
        assert!(plan_barrier(1, 0x8).is_err());
        assert!(plan_barrier(1, 0x400).is_err());
    }

    #[test]
    fn markers_and_diffs_read_back() {
        let seq = 0x1234u64;
        assert_eq!(marker_text([MARK_BEFORE | 0x1234, 0, 0], seq), "before(all prior done)=SET draw_started=unset draw_done=unset");
        assert!(marker_text([7, 0, 0], seq).starts_with("before(all prior done)=other(0x7)"));
        let a = vec![0u8; 64];
        let mut b = a.clone();
        assert_eq!(diff_text(&a, &b), "identical (64 bytes)");
        b[33] = 1;
        assert_eq!(diff_text(&a, &b), "1 of 4 rows differ: row 2");
        let mut m = HashMap::new();
        m.insert(2u32, vec![Some(1u32), None]);
        assert_eq!(consts_text(&m), "param 2: [00000001 --------]");
    }
}
