use core::cell::Cell;
use core::ffi::c_void;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use compat_common::record;

static REAL_SIGNAL: AtomicUsize = AtomicUsize::new(0);
static REAL_WAIT: AtomicUsize = AtomicUsize::new(0);
static ON: AtomicBool = AtomicBool::new(false);
static GEN: AtomicU32 = AtomicU32::new(0);
static POLLER: AtomicBool = AtomicBool::new(false);
const MAX_PENDING: usize = 4096;

thread_local! {
    static OURS: Cell<u32> = const { Cell::new(0) };
}

type FenceOp = unsafe extern "system" fn(*mut c_void, *mut c_void, u64) -> i32;

struct Pending {
    fence: usize,
    value: u64,
    at: Instant,
    ty: u32,
    ours: bool,
    gen: u32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Done {
    pub ty: u32,
    pub fence: usize,
    pub ms: f64,
    pub ours: bool,
    pub gen: u32,
}

#[derive(Default)]
struct Q {
    pending: Vec<Pending>,
    done: Vec<Done>,
    types: HashMap<usize, u32>,
    freqs: HashMap<u32, u64>,
    signals: HashMap<u32, u64>,
    dropped: u64,
    names: HashMap<usize, &'static str>,
}

static S: Mutex<Option<Q>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut Q) -> R) -> R {
    let mut g = S.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(Q::default))
}

unsafe fn vslot(obj: *mut c_void, i: usize) -> usize {
    unsafe { *(*(obj as *const *const usize)).add(i) }
}

unsafe fn completed(fence: *mut c_void) -> u64 {
    let f: unsafe extern "system" fn(*mut c_void) -> u64 = unsafe { core::mem::transmute(vslot(fence, 8)) };
    unsafe { f(fence) }
}

unsafe fn addref(o: *mut c_void) {
    let f: unsafe extern "system" fn(*mut c_void) -> u32 = unsafe { core::mem::transmute(vslot(o, 1)) };
    unsafe { f(o) };
}

unsafe fn release(o: *mut c_void) {
    let f: unsafe extern "system" fn(*mut c_void) -> u32 = unsafe { core::mem::transmute(vslot(o, 2)) };
    unsafe { f(o) };
}

pub fn installed() -> bool {
    REAL_WAIT.load(Ordering::Relaxed) != 0
}

pub fn install(real: &[AtomicUsize]) {
    if !crate::gpuprof::enabled() || real.len() <= 18 || installed() {
        return;
    }
    let (s, w) = (real[14].load(Ordering::Relaxed), real[15].load(Ordering::Relaxed));
    if s == 0 || w == 0 {
        return;
    }
    REAL_SIGNAL.store(s, Ordering::Relaxed);
    REAL_WAIT.store(w, Ordering::Relaxed);
    real[14].store(q_signal as usize, Ordering::Relaxed);
    real[15].store(q_wait as usize, Ordering::Relaxed);
    record("d3d12_xs", "gpuprof", "queue Signal/Wait timing installed on every queue (title and emulator callers); a wait's ms is enqueue-to-signal latency, from the Wait call to the fence reaching its value, so it includes that queue's backlog and is not GPU stall time");
}

pub fn set_real_for_test(signal: usize, wait: usize) {
    REAL_SIGNAL.store(signal, Ordering::Relaxed);
    REAL_WAIT.store(wait, Ordering::Relaxed);
}

pub unsafe fn queue_type(queue: *mut c_void) -> u32 {
    if let Some(t) = with(|q| q.types.get(&(queue as usize)).copied()) {
        return t;
    }
    let desc = crate::present::real_queue_slot(18);
    let desc = if desc != 0 { desc } else { unsafe { vslot(queue, 18) } };
    let t = unsafe { crate::ecl_probe::queue_desc(queue, desc) }[0];
    let fq = crate::present::real_queue_slot(16);
    let fq = if fq != 0 { fq } else { unsafe { vslot(queue, 16) } };
    let mut hz = 0u64;
    let f: unsafe extern "system" fn(*mut c_void, *mut u64) -> i32 = unsafe { core::mem::transmute(fq) };
    if unsafe { f(queue, &mut hz) } < 0 {
        hz = 0;
    }
    with(|q| {
        q.types.insert(queue as usize, t);
        q.freqs.entry(t).or_insert(hz);
    });
    t
}

pub struct OursScope(u32);

impl Drop for OursScope {
    fn drop(&mut self) {
        OURS.with(|c| c.set(self.0));
    }
}

pub fn ours_scope() -> OursScope {
    OURS.with(|c| {
        let prev = c.get();
        c.set(prev + 1);
        OursScope(prev)
    })
}

pub fn name_fence(fence: *mut c_void, label: &'static str) {
    if fence.is_null() {
        return;
    }
    with(|q| {
        q.names.insert(fence as usize, label);
    });
}

pub fn real_signal() -> usize {
    REAL_SIGNAL.load(Ordering::Relaxed)
}

fn ours() -> bool {
    OURS.with(|c| c.get() != 0)
}

pub unsafe extern "system" fn q_signal(queue: *mut c_void, fence: *mut c_void, value: u64) -> i32 {
    if ON.load(Ordering::Relaxed) {
        let t = unsafe { queue_type(queue) };
        with(|q| *q.signals.entry(t).or_default() += 1);
    }
    let f: FenceOp = unsafe { core::mem::transmute(REAL_SIGNAL.load(Ordering::Relaxed)) };
    unsafe { f(queue, fence, value) }
}

pub unsafe extern "system" fn q_wait(queue: *mut c_void, fence: *mut c_void, value: u64) -> i32 {
    if ON.load(Ordering::Relaxed) && !fence.is_null() {
        let t = unsafe { queue_type(queue) };
        unsafe { note_wait(t, fence, value, ours()) };
    }
    let f: FenceOp = unsafe { core::mem::transmute(REAL_WAIT.load(Ordering::Relaxed)) };
    unsafe { f(queue, fence, value) }
}

pub unsafe fn note_wait(ty: u32, fence: *mut c_void, value: u64, ours: bool) {
    let gen = GEN.load(Ordering::Relaxed);
    if unsafe { completed(fence) } >= value {
        with(|q| q.done.push(Done { ty, fence: fence as usize, ms: 0.0, ours, gen }));
        return;
    }
    let full = with(|q| {
        if q.pending.len() >= MAX_PENDING {
            q.dropped += 1;
            return true;
        }
        false
    });
    if full {
        return;
    }
    unsafe { addref(fence) };
    with(|q| q.pending.push(Pending { fence: fence as usize, value, at: Instant::now(), ty, ours, gen }));
    ensure_poller();
}

fn ensure_poller() {
    if POLLER.swap(true, Ordering::SeqCst) {
        return;
    }
    let _ = std::thread::Builder::new().name("tombox-gpuprof-waits".into()).spawn(|| loop {
        let n = poll();
        std::thread::sleep(if n > 0 { Duration::from_micros(200) } else { Duration::from_millis(5) });
    });
}

pub fn poll() -> usize {
    let mut freed: Vec<usize> = Vec::new();
    let left = with(|q| {
        let now = Instant::now();
        let mut i = 0;
        while i < q.pending.len() {
            let p = &q.pending[i];
            if unsafe { completed(p.fence as *mut c_void) } >= p.value {
                let p = q.pending.swap_remove(i);
                q.done.push(Done { ty: p.ty, fence: p.fence, ms: now.duration_since(p.at).as_secs_f64() * 1000.0, ours: p.ours, gen: p.gen });
                freed.push(p.fence);
            } else {
                i += 1;
            }
        }
        q.pending.len()
    });
    for f in freed {
        unsafe { release(f as *mut c_void) };
    }
    left
}

pub fn arm() {
    GEN.fetch_add(1, Ordering::Relaxed);
    with(|q| {
        q.done.clear();
        q.signals.clear();
        q.dropped = 0;
    });
    ON.store(true, Ordering::Relaxed);
}

pub fn disarm() {
    ON.store(false, Ordering::Relaxed);
}

pub fn current() -> Vec<Done> {
    let gen = GEN.load(Ordering::Relaxed);
    poll();
    with(|q| q.done.iter().filter(|d| d.gen == gen).copied().collect())
}

pub fn freqs() -> Vec<(u32, u64)> {
    let mut v: Vec<(u32, u64)> = with(|q| q.freqs.iter().map(|(a, b)| (*a, *b)).collect());
    v.sort_unstable();
    v
}

pub fn freq_note(v: &[(u32, u64)], ours: u64) -> String {
    let mut hz: Vec<u64> = v.iter().map(|x| x.1).chain(core::iter::once(ours)).filter(|h| *h != 0).collect();
    hz.sort_unstable();
    hz.dedup();
    let list: Vec<String> = v.iter().map(|(t, h)| format!("{} {h}", type_name(*t))).collect();
    let mut out = format!("queue timestamp Hz: {}", if list.is_empty() { "-".to_string() } else { list.join(", ") });
    if hz.len() > 1 {
        out.push_str(" | WARNING: queues use different timestamp frequencies, so the cross-queue union and untimed gap mix clocks and are not valid");
    }
    out
}

pub fn type_name(t: u32) -> String {
    match t {
        0 => "direct".to_string(),
        2 => "compute".to_string(),
        3 => "copy".to_string(),
        _ => format!("type{t}"),
    }
}

fn class(d: &Done, names: &HashMap<usize, &'static str>) -> &'static str {
    match names.get(&d.fence) {
        Some(n) => n,
        None if d.ours => "emulator",
        None => "title",
    }
}

pub fn summarize(done: &[Done], unresolved: &[(u32, f64)], signals: &HashMap<u32, u64>, dropped: u64, names: &HashMap<usize, &'static str>) -> String {
    let mut by: Vec<((u32, &'static str), (u64, u64, f64, f64))> = Vec::new();
    for d in done {
        let k = (d.ty, class(d, names));
        let e = match by.iter_mut().find(|e| e.0 == k) {
            Some(e) => e,
            None => {
                by.push((k, (0, 0, 0.0, 0.0)));
                by.last_mut().unwrap()
            }
        };
        e.1 .0 += 1;
        if d.ms == 0.0 {
            e.1 .1 += 1;
        }
        e.1 .2 += d.ms;
        e.1 .3 = e.1 .3.max(d.ms);
    }
    by.sort_by_key(|e| (e.0 .0, e.0 .1 != "title", e.0 .1));
    let mut out = String::from("queue waits (enqueue-to-signal latency ms, not GPU stall):");
    if by.is_empty() {
        out.push_str(" none");
    }
    for ((t, o), (n, z, tot, mx)) in &by {
        out.push_str(&format!(" | {} waits on {o} fences n={n} already-reached={z} total {tot:.3} ms max {mx:.3} ms", type_name(*t)));
    }
    if !unresolved.is_empty() {
        let oldest = unresolved.iter().map(|u| u.1).fold(0.0f64, f64::max);
        out.push_str(&format!(" | unresolved {} (oldest {oldest:.1} ms)", unresolved.len()));
    }
    if dropped > 0 {
        out.push_str(&format!(" | dropped {dropped}"));
    }
    let mut fences: Vec<(usize, u32, u64, f64, f64, &'static str)> = Vec::new();
    for d in done {
        match fences.iter_mut().find(|f| f.0 == d.fence) {
            Some(f) => {
                f.2 += 1;
                f.3 += d.ms;
                f.4 = f.4.max(d.ms);
            }
            None => fences.push((d.fence, d.ty, 1, d.ms, d.ms, class(d, names))),
        }
    }
    fences.sort_by(|a, b| b.3.partial_cmp(&a.3).unwrap_or(core::cmp::Ordering::Equal));
    let top: Vec<String> = fences.iter().take(3).filter(|f| f.3 > 0.0)
        .map(|f| format!("{} fence {:#x} waited on {} n={} total {:.3} ms max {:.3} ms", f.5, f.0, type_name(f.1), f.2, f.3, f.4)).collect();
    out.push_str(&format!("\n[gpuprof]   top wait fences: {}", if top.is_empty() { "-".to_string() } else { top.join(" | ") }));
    let mut sg: Vec<_> = signals.iter().collect();
    sg.sort();
    let sg: Vec<String> = sg.iter().map(|(t, n)| format!("{} {n}", type_name(**t))).collect();
    out.push_str(&format!("\n[gpuprof]   queue signals: {}", if sg.is_empty() { "-".to_string() } else { sg.join(", ") }));
    out
}

pub fn report_line() -> String {
    if !installed() {
        return "queue waits: Signal/Wait hooks not installed (TOMBOX_GPUPROF was off when the first queue was created)".to_string();
    }
    let gen = GEN.load(Ordering::Relaxed);
    let done = current();
    let (unresolved, signals, dropped, names) = with(|q| {
        let now = Instant::now();
        let u: Vec<(u32, f64)> = q.pending.iter().filter(|p| p.gen == gen).map(|p| (p.ty, now.duration_since(p.at).as_secs_f64() * 1000.0)).collect();
        (u, q.signals.clone(), q.dropped, q.names.clone())
    });
    summarize(&done, &unresolved, &signals, dropped, &names)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frequency_note_flags_mixed_clocks() {
        assert_eq!(freq_note(&[(0, 100), (2, 100)], 100), "queue timestamp Hz: direct 100, compute 100");
        assert!(freq_note(&[(0, 100), (3, 50)], 100).ends_with("mix clocks and are not valid"));
        assert_eq!(freq_note(&[], 0), "queue timestamp Hz: -");
    }

    #[test]
    fn ours_scope_restores_the_previous_value() {
        assert!(!ours());
        {
            let _a = ours_scope();
            assert!(ours());
            {
                let _b = ours_scope();
                assert!(ours());
            }
            assert!(ours());
        }
        assert!(!ours());
    }

    #[test]
    fn summary_groups_by_queue_and_caller_and_ranks_fences() {
        let d = |ty, fence, ms, ours| Done { ty, fence, ms, ours, gen: 1 };
        let done = vec![d(2, 0x10, 20.0, false), d(2, 0x10, 5.0, false), d(0, 0x20, 0.0, false), d(0, 0x30, 1.5, true), d(0, 0x40, 3.0, false)];
        let sig: HashMap<u32, u64> = [(0, 4), (2, 1)].into_iter().collect();
        let names: HashMap<usize, &'static str> = [(0x40usize, "upload")].into_iter().collect();
        let s = summarize(&done, &[(3, 12.0)], &sig, 0, &names);
        assert!(s.starts_with("queue waits (enqueue-to-signal latency ms, not GPU stall): | direct waits on title fences n=1 already-reached=1 total 0.000 ms max 0.000 ms | direct waits on emulator fences n=1 already-reached=0 total 1.500 ms max 1.500 ms | direct waits on upload fences n=1 already-reached=0 total 3.000 ms max 3.000 ms | compute waits on title fences n=2 already-reached=0 total 25.000 ms max 20.000 ms | unresolved 1 (oldest 12.0 ms)"), "{s}");
        assert!(s.contains("top wait fences: title fence 0x10 waited on compute n=2 total 25.000 ms max 20.000 ms | upload fence 0x40 waited on direct n=1 total 3.000 ms max 3.000 ms | emulator fence 0x30 waited on direct n=1 total 1.500 ms max 1.500 ms\n"), "{s}");
        assert!(s.ends_with("queue signals: direct 4, compute 1"), "{s}");
        assert_eq!(summarize(&[], &[], &HashMap::new(), 2, &HashMap::new()), "queue waits (enqueue-to-signal latency ms, not GPU stall): none | dropped 2\n[gpuprof]   top wait fences: -\n[gpuprof]   queue signals: -");
    }
}
