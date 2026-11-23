use compat_common::record;
use core::ffi::c_void;
use core::sync::atomic::{AtomicUsize, Ordering};
use std::cell::Cell;
use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, OnceLock};

const RING: usize = 8000;
const KEEP_SNAPS: usize = 4;
const BEFORE_LINES: usize = 2500;
const AFTER_LINES: usize = 2500;
const XBOX_FIRST: usize = 33;
const XBOX_LAST: usize = 46;
const WRITE_AT_DRAW: u64 = 8;

#[derive(Clone)]
struct Ev {
    seq: u64,
    src: &'static str,
    slot: u16,
    a: u64,
    b: u64,
    c: u64,
    d: u64,
}

struct Snap {
    draw: u64,
    at: u64,
    list: usize,
    ps: u64,
    text: String,
    before: Vec<Ev>,
}

#[derive(Default)]
struct State {
    seq: u64,
    lists: HashMap<usize, VecDeque<Ev>>,
    snaps: VecDeque<Snap>,
    written: u64,
}

static S: Mutex<Option<State>> = Mutex::new(None);
static PREV_X: [AtomicUsize; XBOX_LAST + 1] = [const { AtomicUsize::new(0) }; XBOX_LAST + 1];

thread_local! {
    static SRC: Cell<&'static str> = const { Cell::new("?") };
}

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut g = S.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(State::default))
}

pub fn on() -> bool {
    static E: OnceLock<bool> = OnceLock::new();
    *E.get_or_init(|| {
        let on = crate::hang_cap::on() && std::env::var("TOMBOX_HANGCAP_ROOTLOG").map(|v| v == "1").unwrap_or(false);
        if on {
            record("d3d12_xs", "rootlog", "TOMBOX_HANGCAP_ROOTLOG=1: every root signature / root descriptor / root table / root constant / injected PSO+Dispatch call issued to the PC vtable on title lists is logged with its source and dumped around captured draws at device removal (diagnostic; nothing is skipped)");
        }
        on
    })
}

fn stem(file: &'static str) -> &'static str {
    let f = file.rsplit(['\\', '/']).next().unwrap_or(file);
    f.strip_suffix(".rs").unwrap_or(f)
}

pub fn note(list: usize, src: &'static str, slot: usize, a: u64, b: u64, c: u64, d: u64) {
    if !on() || list == 0 {
        return;
    }
    with(|s| {
        s.seq += 1;
        let seq = s.seq;
        let r = s.lists.entry(list).or_default();
        if r.len() >= RING {
            r.pop_front();
        }
        r.push_back(Ev { seq, src, slot: slot as u16, a, b, c, d });
    });
}

fn src_take() -> &'static str {
    SRC.with(|c| c.replace("?"))
}

unsafe fn real(slot: usize) -> usize {
    unsafe { crate::cmdlist_wrap::real_list_slot(slot) }
}

macro_rules! pc_thunks {
    ($( $name:ident, $slot:expr, ($($a:ident: $t:ty),*), [$($v:expr),*] );* $(;)?) => {
        $(
            unsafe extern "system" fn $name(this: *mut c_void $(, $a: $t)*) {
                let src = src_take();
                let v: [u64; 4] = { let mut x = [0u64; 4]; let y: &[u64] = &[$($v as u64),*]; x[..y.len()].copy_from_slice(y); x };
                note(this as usize, src, $slot, v[0], v[1], v[2], v[3]);
                let f: unsafe extern "system" fn(*mut c_void $(, $t)*) = unsafe { core::mem::transmute(real($slot)) };
                unsafe { f(this $(, $a)*) }
            }
        )*
        fn pc_thunk(slot: usize) -> Option<usize> {
            match slot {
                $( $slot => Some($name as *const () as usize), )*
                _ => None,
            }
        }
    };
}

pc_thunks!(
    pc_dispatch, 14, (x: u32, y: u32, z: u32), [x, y, z];
    pc_pso, 25, (p: *mut c_void), [p as usize];
    pc_heaps, 28, (n: u32, h: *const *mut c_void), [n, heap(h, 0), heap(h, 1)];
    pc_crs, 29, (p: *mut c_void), [p as usize];
    pc_grs, 30, (p: *mut c_void), [p as usize];
    pc_ctab, 31, (i: u32, h: u64), [i, h];
    pc_gtab, 32, (i: u32, h: u64), [i, h];
    pc_cc1, 33, (i: u32, v: u32, o: u32), [i, v, o];
    pc_gc1, 34, (i: u32, v: u32, o: u32), [i, v, o];
    pc_ccn, 35, (i: u32, n: u32, p: *const u32, o: u32), [i, n, o, first(p, n)];
    pc_gcn, 36, (i: u32, n: u32, p: *const u32, o: u32), [i, n, o, first(p, n)];
    pc_ccbv, 37, (i: u32, va: u64), [i, va];
    pc_gcbv, 38, (i: u32, va: u64), [i, va];
    pc_csrv, 39, (i: u32, va: u64), [i, va];
    pc_gsrv, 40, (i: u32, va: u64), [i, va];
    pc_cuav, 41, (i: u32, va: u64), [i, va];
    pc_guav, 42, (i: u32, va: u64), [i, va];
);

fn heap(h: *const *mut c_void, i: u32) -> usize {
    if h.is_null() { 0 } else { unsafe { *h.add(i as usize) as usize } }
}

fn first(p: *const u32, n: u32) -> u64 {
    if p.is_null() || n == 0 { 0 } else { unsafe { *p as u64 } }
}

pub fn route(slot: usize, file: &'static str) -> Option<usize> {
    if !on() {
        return None;
    }
    let t = pc_thunk(slot)?;
    SRC.with(|c| c.set(stem(file)));
    Some(t)
}

macro_rules! x_thunks {
    ($( $name:ident, $slot:expr, ($($a:ident: $t:ty),*), [$($v:expr),*] );* $(;)?) => {
        $(
            unsafe extern "system" fn $name(this: *mut c_void $(, $a: $t)*) {
                let v: [u64; 4] = { let mut x = [0u64; 4]; let y: &[u64] = &[$($v as u64),*]; x[..y.len()].copy_from_slice(y); x };
                note(this as usize, "title", $slot - 4, v[0], v[1], v[2], v[3]);
                let f: unsafe extern "system" fn(*mut c_void $(, $t)*) = unsafe { core::mem::transmute(PREV_X[$slot].load(Ordering::Relaxed)) };
                unsafe { f(this $(, $a)*) }
            }
        )*
        fn x_thunk(slot: usize) -> Option<usize> {
            match slot {
                $( $slot => Some($name as *const () as usize), )*
                _ => None,
            }
        }
    };
}

x_thunks!(
    x_crs, 33, (p: *mut c_void), [p as usize];
    x_grs, 34, (p: *mut c_void), [p as usize];
    x_ctab, 35, (i: u32, h: u64), [i, h];
    x_gtab, 36, (i: u32, h: u64), [i, h];
    x_cc1, 37, (i: u32, v: u32, o: u32), [i, v, o];
    x_gc1, 38, (i: u32, v: u32, o: u32), [i, v, o];
    x_ccn, 39, (i: u32, n: u32, p: *const u32, o: u32), [i, n, o, first(p, n)];
    x_gcn, 40, (i: u32, n: u32, p: *const u32, o: u32), [i, n, o, first(p, n)];
    x_ccbv, 41, (i: u32, va: u64), [i, va];
    x_gcbv, 42, (i: u32, va: u64), [i, va];
    x_csrv, 43, (i: u32, va: u64), [i, va];
    x_gsrv, 44, (i: u32, va: u64), [i, va];
    x_cuav, 45, (i: u32, va: u64), [i, va];
    x_guav, 46, (i: u32, va: u64), [i, va];
);

pub fn install(vt: &[AtomicUsize]) {
    if !on() || vt.len() <= XBOX_LAST {
        return;
    }
    for slot in XBOX_FIRST..=XBOX_LAST {
        let Some(t) = x_thunk(slot) else { continue };
        let prev = vt[slot].load(Ordering::Relaxed);
        if prev == 0 || prev == t {
            continue;
        }
        PREV_X[slot].store(prev, Ordering::Relaxed);
        vt[slot].store(t, Ordering::Relaxed);
    }
    record("d3d12_xs", "rootlog", "title root-signature/table/constant/CBV/SRV/UAV calls (Xbox slots 33..46) wrapped for the root log");
}

pub fn at_draw(list: usize, ps: u64, draw: u64, text: &str) {
    if !on() || list == 0 || ps != crate::hang_snap::snap_ps() {
        return;
    }
    let (before, n) = with(|s| {
        let before: Vec<Ev> = s.lists.get(&list).map(|r| r.iter().cloned().collect()).unwrap_or_default();
        let at = s.seq;
        if s.snaps.len() >= KEEP_SNAPS {
            s.snaps.pop_front();
        }
        s.snaps.push_back(Snap { draw, at, list, ps, text: text.to_string(), before: before.clone() });
        s.written += 1;
        (before, s.written)
    });
    if n <= WRITE_AT_DRAW {
        write(draw, list, ps, text, &before, None);
    }
}

fn write(draw: u64, list: usize, ps: u64, text: &str, before: &[Ev], after: Option<&[Ev]>) {
    let mut by_src: HashMap<&'static str, usize> = HashMap::new();
    for e in before.iter().chain(after.unwrap_or(&[]).iter()) {
        *by_src.entry(e.src).or_default() += 1;
    }
    let mut srcs: Vec<_> = by_src.into_iter().collect();
    srcs.sort_by(|a, b| b.1.cmp(&a.1));
    let state = state_text(before);
    let when = if after.is_some() { "at device removal" } else { "at the draw (no calls after it yet)" };
    let mut out = format!("draw #{draw} list {list:#x} ps={ps:016x} {text}\nwritten {when}\nsources: {srcs:?}\n\nroot state issued to the PC list at the draw:\n{state}\ninjected compute passes before the draw:\n{}\n", passes_text(before));
    if let Some(a) = after {
        out.push_str(&format!("injected compute passes after the draw:\n{}\n", passes_text(a)));
    }
    out.push_str(&format!("calls before the draw (last {BEFORE_LINES}):\n"));
    for e in before.iter().rev().take(BEFORE_LINES).collect::<Vec<_>>().into_iter().rev() {
        out.push_str(&line(e));
        out.push('\n');
    }
    if let Some(a) = after {
        out.push_str(&format!("---- draw #{draw} ----\ncalls after the draw (first {AFTER_LINES}):\n"));
        for e in a.iter().take(AFTER_LINES) {
            out.push_str(&line(e));
            out.push('\n');
        }
    }
    let dir = crate::hang_cap::dump_dir();
    let _ = std::fs::create_dir_all(&dir);
    let p = dir.join(format!("rootlog_draw{draw}.txt"));
    let _ = std::fs::write(&p, &out);
    record("d3d12_xs", "rootlog", &format!("draw #{draw} list {list:#x}: {} calls before, {} after -> {}\n{state}", before.len(), after.map_or(0, |a| a.len()), p.display()));
}

fn name(slot: u16) -> &'static str {
    match slot {
        14 => "Dispatch",
        25 => "SetPipelineState",
        28 => "SetDescriptorHeaps",
        29 => "SetComputeRootSignature",
        30 => "SetGraphicsRootSignature",
        31 => "SetComputeRootDescriptorTable",
        32 => "SetGraphicsRootDescriptorTable",
        33 => "SetComputeRoot32BitConstant",
        34 => "SetGraphicsRoot32BitConstant",
        35 => "SetComputeRoot32BitConstants",
        36 => "SetGraphicsRoot32BitConstants",
        37 => "SetComputeRootConstantBufferView",
        38 => "SetGraphicsRootConstantBufferView",
        39 => "SetComputeRootShaderResourceView",
        40 => "SetGraphicsRootShaderResourceView",
        41 => "SetComputeRootUnorderedAccessView",
        42 => "SetGraphicsRootUnorderedAccessView",
        _ => "?",
    }
}

fn line(e: &Ev) -> String {
    let args = match e.slot {
        14 => format!("({}, {}, {})", e.a, e.b, e.c),
        25 | 29 | 30 => format!("({:#x})", e.a),
        28 => format!("(n={}, {:#x}, {:#x})", e.a, e.b, e.c),
        31 | 32 => format!("(param {}, gpu handle {:#x})", e.a, e.b),
        33 | 34 => format!("(param {}, value {:#010x}, offset {})", e.a, e.b, e.c),
        35 | 36 => format!("(param {}, n {}, offset {}, first {:#010x})", e.a, e.b, e.c, e.d),
        _ => format!("(param {}, va {:#x})", e.a, e.b),
    };
    let res = match e.slot {
        37..=42 if e.b != 0 => match crate::bindtrace::buffer_at(e.b) {
            Some((base, res, size, heap)) => format!(" -> res {res:#x} +{:#x} of {size:#x} heap {heap}", e.b - base),
            None => match crate::gpu_va::lookup_gpu(e.b) {
                Some((g, off)) => format!(" -> res {:#x} +{off:#x} of {:#x} (gpu_va)", g.res, g.len),
                None => " -> VA NOT IN ANY TRACKED BUFFER".to_string(),
            },
        },
        _ => String::new(),
    };
    format!("#{:<9} {:<18} PC{:<3} {}{}{}", e.seq, e.src, e.slot, name(e.slot), args, res)
}

fn state_text(evs: &[Ev]) -> String {
    let mut grs = None;
    let mut since: Vec<&Ev> = Vec::new();
    for e in evs {
        if e.slot == 30 {
            grs = Some(e);
            since.clear();
        } else if matches!(e.slot, 32 | 34 | 36 | 38 | 40 | 42) {
            since.push(e);
        }
    }
    let mut last: HashMap<u64, &Ev> = HashMap::new();
    for e in &since {
        last.insert(e.a, e);
    }
    let mut keys: Vec<&u64> = last.keys().collect();
    keys.sort();
    let mut out = match grs {
        Some(g) => format!("graphics root signature {:#x} set by {} at #{}\n", g.a, g.src, g.seq),
        None => "graphics root signature: not set on this list within the log window\n".to_string(),
    };
    for k in keys {
        out.push_str(&format!("    param {k}: {}\n", line(last[k])));
    }
    out
}

fn passes_text(evs: &[Ev]) -> String {
    let mut out = String::new();
    for (i, e) in evs.iter().enumerate() {
        if e.slot != 14 || e.src == "title" {
            continue;
        }
        out.push_str(&format!("  injected {}\n", line(e)));
        let mut k = i;
        while k > 0 && evs[k - 1].src == e.src && i - k < 16 {
            k -= 1;
        }
        for p in &evs[k..i] {
            out.push_str(&format!("      before: {}\n", line(p)));
        }
        for n in evs.iter().skip(i + 1).take_while(|n| n.src != "title").take(32) {
            out.push_str(&format!("      after:  {}\n", line(n)));
        }
    }
    if out.is_empty() {
        out.push_str("  none in the log window\n");
    }
    out
}

pub fn dump() {
    if !on() {
        return;
    }
    let snaps = with(|s| {
        s.snaps.iter().map(|sn| {
            let after: Vec<Ev> = s.lists.get(&sn.list).map(|r| r.iter().filter(|e| e.seq > sn.at).cloned().collect()).unwrap_or_default();
            (sn.draw, sn.list, sn.ps, sn.text.clone(), sn.before.clone(), after)
        }).collect::<Vec<_>>()
    });
    if snaps.is_empty() {
        record("d3d12_xs", "rootlog", "no captured draw; nothing to dump");
        return;
    }
    for (draw, list, ps, text, before, after) in snaps.iter().rev() {
        write(*draw, *list, *ps, text, before, Some(after));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stem_takes_the_module_name() {
        assert_eq!(stem("crates\\compat\\d3d12_xs\\src\\ts99.rs"), "ts99");
        assert_eq!(stem("src/cl2_state.rs"), "cl2_state");
    }

    #[test]
    fn state_keeps_the_last_value_per_param_after_the_last_root_signature() {
        let e = |seq, slot, a, b| Ev { seq, src: "title", slot, a, b, c: 0, d: 0 };
        let evs = vec![e(1, 40, 13, 0x10), e(2, 30, 0xaaa, 0), e(3, 40, 13, 0x20), e(4, 40, 13, 0x30), e(5, 38, 2, 0x40)];
        let t = state_text(&evs);
        assert!(t.contains("0xaaa"));
        assert!(t.contains("va 0x30"));
        assert!(!t.contains("va 0x20"));
        assert!(!t.contains("va 0x10"));
        assert!(t.contains("param 2"));
    }

    #[test]
    fn injected_dispatch_lists_what_follows_it() {
        let e = |seq, src, slot, a| Ev { seq, src, slot, a, b: 0, c: 0, d: 0 };
        let evs = vec![e(1, "title", 40, 13), e(2, "ts99", 29, 0x1), e(3, "ts99", 14, 1), e(4, "ts99", 30, 0x2), e(5, "title", 40, 13)];
        let t = passes_text(&evs);
        assert!(t.contains("injected"));
        assert!(t.contains("before: #2"));
        assert!(t.contains("after:  #4"));
        assert!(!t.contains("#5"));
    }
}
