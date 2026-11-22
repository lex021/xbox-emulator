use crate::frame::{qpc, qpf};
use core::cell::Cell;
use core::ffi::c_void;
use core::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, AtomicU64, AtomicUsize, Ordering::Relaxed};
use std::sync::{Mutex, MutexGuard, TryLockError};

pub const PSO_SYNC: usize = 0;
pub const PSO_INLINE: usize = 1;
pub const PSO_ASYNC: usize = 2;
pub const PSO_BIND: usize = 3;
pub const XLAT: usize = 4;
pub const DXC: usize = 5;
pub const COMMITTED: usize = 6;
pub const PLACED: usize = 7;
pub const HEAP: usize = 8;
pub const UPLOAD: usize = 9;
pub const IO: usize = 10;
pub const IO_BG: usize = 11;
pub const LOCK: usize = 12;
pub const FRAME_WAIT: usize = 13;
pub const N: usize = 14;
pub const NAMES: [&str; N] = ["pso_sync", "pso_inline", "pso_async", "pso_bind_wait", "xlat", "dxc", "committed", "placed", "heap", "upload", "io_read", "io_read_bg", "lock_wait", "frame_wait"];

static ACTIVE: AtomicBool = AtomicBool::new(false);
static CNT: [AtomicU64; N] = [const { AtomicU64::new(0) }; N];
static TICKS: [AtomicU64; N] = [const { AtomicU64::new(0) }; N];
static BG: [AtomicU64; N] = [const { AtomicU64::new(0) }; N];
static BG_TICKS: [AtomicU64; N] = [const { AtomicU64::new(0) }; N];
static QPC_LAST: AtomicI64 = AtomicI64::new(0);
static QPC_CALLS: AtomicU64 = AtomicU64::new(0);
static GTC_CALLS: AtomicU64 = AtomicU64::new(0);
static PRESENT_TID: AtomicU32 = AtomicU32::new(0);
static ORIG_QPC: AtomicUsize = AtomicUsize::new(0);
static ORIG_GTC: AtomicUsize = AtomicUsize::new(0);
static ORIG_GTC64: AtomicUsize = AtomicUsize::new(0);
static ORIG_RF: AtomicUsize = AtomicUsize::new(0);
static ORIG_RFX: AtomicUsize = AtomicUsize::new(0);
static HOOKED: std::sync::Once = std::sync::Once::new();

thread_local! {
    static CHILD: Cell<i64> = const { Cell::new(0) };
    static KIND: Cell<u8> = const { Cell::new(0) };
    static ASYNC_MARK: Cell<bool> = const { Cell::new(false) };
}

extern "system" {
    fn GetCurrentThreadId() -> u32;
    fn GetModuleHandleW(name: *const u16) -> *mut u8;
}

#[inline]
pub fn active() -> bool {
    ACTIVE.load(Relaxed)
}

pub fn set_active(on: bool) {
    ACTIVE.store(on, Relaxed);
    if on {
        let _ = take();
        install_title_hooks();
    }
}

fn worker() -> bool {
    KIND.with(|k| {
        if k.get() == 0 {
            let ours = std::thread::current().name().is_some_and(|n| n.starts_with("tombox"));
            k.set(if ours { 2 } else { 1 });
        }
        k.get() == 2
    })
}

fn add(k: usize, ticks: i64, bg: bool) {
    let t = ticks.max(0) as u64;
    if bg {
        BG[k].fetch_add(1, Relaxed);
        BG_TICKS[k].fetch_add(t, Relaxed);
    } else {
        CNT[k].fetch_add(1, Relaxed);
        TICKS[k].fetch_add(t, Relaxed);
    }
}

pub struct Scope {
    k: usize,
    t0: i64,
    saved: i64,
    bg: bool,
}

impl Scope {
    pub fn set_kind(&mut self, k: usize) {
        self.k = k;
    }
}

impl Drop for Scope {
    fn drop(&mut self) {
        let t = qpc() - self.t0;
        let child = CHILD.with(|c| c.get());
        add(self.k, t - child, self.bg);
        CHILD.with(|c| c.set(self.saved + t));
    }
}

#[inline]
pub fn begin(k: usize) -> Option<Scope> {
    if !active() {
        return None;
    }
    let bg = worker();
    let saved = CHILD.with(|c| c.replace(0));
    Some(Scope { k, t0: qpc(), saved, bg })
}

#[inline]
pub fn time<R>(k: usize, f: impl FnOnce() -> R) -> R {
    let _s = begin(k);
    f()
}

pub fn note_nested(k: usize, ticks: i64) {
    if !active() {
        return;
    }
    add(k, ticks, worker());
    CHILD.with(|c| c.set(c.get() + ticks.max(0)));
}

fn on_dxc(us: u64) {
    note_nested(DXC, (us as i128 * qpf() as i128 / 1_000_000) as i64);
}

pub fn note_frame_wait(ticks: i64) {
    if active() {
        add(FRAME_WAIT, ticks, worker());
    }
}

pub fn mark_async() {
    if active() {
        ASYNC_MARK.with(|m| m.set(true));
    }
}

pub fn take_async_mark() -> bool {
    ASYNC_MARK.with(|m| m.replace(false))
}

pub fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    if !active() {
        return m.lock().unwrap_or_else(|e| e.into_inner());
    }
    match m.try_lock() {
        Ok(g) => g,
        Err(TryLockError::Poisoned(e)) => e.into_inner(),
        Err(TryLockError::WouldBlock) => {
            let _s = begin(LOCK);
            m.lock().unwrap_or_else(|e| e.into_inner())
        }
    }
}

#[derive(Clone, Copy, Default)]
pub struct Snap {
    pub n: [u64; N],
    pub ticks: [u64; N],
    pub bg_n: [u64; N],
    pub bg_ticks: [u64; N],
    pub qpc_last: i64,
    pub qpc_calls: u64,
    pub gtc_calls: u64,
}

pub fn take() -> Snap {
    let mut s = Snap::default();
    for k in 0..N {
        s.n[k] = CNT[k].swap(0, Relaxed);
        s.ticks[k] = TICKS[k].swap(0, Relaxed);
        s.bg_n[k] = BG[k].swap(0, Relaxed);
        s.bg_ticks[k] = BG_TICKS[k].swap(0, Relaxed);
    }
    s.qpc_last = QPC_LAST.load(Relaxed);
    s.qpc_calls = QPC_CALLS.swap(0, Relaxed);
    s.gtc_calls = GTC_CALLS.swap(0, Relaxed);
    s
}

pub fn note_present_thread() {
    PRESENT_TID.store(unsafe { GetCurrentThreadId() }, Relaxed);
}

pub fn header() -> String {
    let mut h = String::new();
    for name in NAMES {
        h.push_str(&format!(",{name}_n,{name}_ms"));
    }
    h.push_str(",bg_pso_n,bg_xlat_n,bg_xlat_ms,bg_dxc_n,bg_dxc_ms,qpc_calls,gtc_calls,game_dt_ms");
    h
}

pub fn row(s: &Snap, game_dt_ms: Option<f64>) -> String {
    let f = qpf() as f64;
    let ms = |t: u64| t as f64 * 1000.0 / f;
    let mut r = String::with_capacity(256);
    for k in 0..N {
        if s.n[k] == 0 {
            r.push_str(",0,0");
        } else {
            r.push_str(&format!(",{},{:.3}", s.n[k], ms(s.ticks[k])));
        }
    }
    let bg_pso = s.bg_n[PSO_SYNC] + s.bg_n[PSO_INLINE] + s.bg_n[PSO_ASYNC];
    r.push_str(&format!(",{bg_pso},{},{:.3},{},{:.3},{},{},", s.bg_n[XLAT], ms(s.bg_ticks[XLAT]), s.bg_n[DXC], ms(s.bg_ticks[DXC]), s.qpc_calls, s.gtc_calls));
    if let Some(g) = game_dt_ms {
        r.push_str(&format!("{g:.3}"));
    }
    r
}

pub fn game_dt_ms(prev_last: i64, s: &Snap) -> Option<f64> {
    if prev_last == 0 || s.qpc_last == 0 {
        return None;
    }
    Some((s.qpc_last - prev_last) as f64 * 1000.0 / qpf() as f64)
}

unsafe extern "system" fn qpc_hook(p: *mut i64) -> i32 {
    let f: unsafe extern "system" fn(*mut i64) -> i32 = unsafe { core::mem::transmute(ORIG_QPC.load(Relaxed)) };
    let r = unsafe { f(p) };
    if r != 0 && !p.is_null() && active() {
        QPC_LAST.fetch_max(unsafe { *p }, Relaxed);
        QPC_CALLS.fetch_add(1, Relaxed);
    }
    r
}

unsafe extern "system" fn gtc_hook() -> u32 {
    let f: unsafe extern "system" fn() -> u32 = unsafe { core::mem::transmute(ORIG_GTC.load(Relaxed)) };
    if active() {
        GTC_CALLS.fetch_add(1, Relaxed);
    }
    unsafe { f() }
}

unsafe extern "system" fn gtc64_hook() -> u64 {
    let f: unsafe extern "system" fn() -> u64 = unsafe { core::mem::transmute(ORIG_GTC64.load(Relaxed)) };
    if active() {
        GTC_CALLS.fetch_add(1, Relaxed);
    }
    unsafe { f() }
}

fn io_kind() -> usize {
    if unsafe { GetCurrentThreadId() } == PRESENT_TID.load(Relaxed) { IO } else { IO_BG }
}

unsafe extern "system" fn read_file_hook(h: *mut c_void, buf: *mut c_void, n: u32, got: *mut u32, ov: *mut c_void) -> i32 {
    let f: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, *mut u32, *mut c_void) -> i32 = unsafe { core::mem::transmute(ORIG_RF.load(Relaxed)) };
    if !active() {
        return unsafe { f(h, buf, n, got, ov) };
    }
    let t0 = qpc();
    let r = unsafe { f(h, buf, n, got, ov) };
    add(io_kind(), qpc() - t0, false);
    r
}

unsafe extern "system" fn read_file_ex_hook(h: *mut c_void, buf: *mut c_void, n: u32, ov: *mut c_void, done: *mut c_void) -> i32 {
    let f: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, *mut c_void, *mut c_void) -> i32 = unsafe { core::mem::transmute(ORIG_RFX.load(Relaxed)) };
    if !active() {
        return unsafe { f(h, buf, n, ov, done) };
    }
    let t0 = qpc();
    let r = unsafe { f(h, buf, n, ov, done) };
    add(io_kind(), qpc() - t0, false);
    r
}

fn install_title_hooks() {
    HOOKED.call_once(|| {
        tombox_xshader::shader_cache::COMPILE_HOOK.store(on_dxc as fn(u64) as usize, Relaxed);
        let targets: [(&str, usize, &AtomicUsize); 5] = [
            ("QueryPerformanceCounter", qpc_hook as usize, &ORIG_QPC),
            ("GetTickCount", gtc_hook as usize, &ORIG_GTC),
            ("GetTickCount64", gtc64_hook as usize, &ORIG_GTC64),
            ("ReadFile", read_file_hook as usize, &ORIG_RF),
            ("ReadFileEx", read_file_ex_hook as usize, &ORIG_RFX),
        ];
        let n = unsafe { compat_common::gdk::hook::patch_module(GetModuleHandleW(core::ptr::null()), &targets) };
        compat_common::record("d3d12_xs", "[frame_csv]", &format!("frame event counters on; title clock/file probes patched {n} import slots in the title exe (QueryPerformanceCounter, GetTickCount[64], ReadFile[Ex])"));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scopes_record_exclusive_time_and_reset_per_take() {
        ACTIVE.store(true, Relaxed);
        KIND.with(|k| k.set(1));
        let _ = take();
        {
            let _outer = begin(XLAT);
            note_nested(DXC, 1000);
            let mut inner = begin(PSO_SYNC).unwrap();
            inner.set_kind(PSO_INLINE);
        }
        let s = take();
        assert_eq!(s.n[XLAT], 1);
        assert_eq!(s.n[DXC], 1);
        assert_eq!(s.ticks[DXC], 1000);
        assert_eq!(s.n[PSO_INLINE], 1);
        assert_eq!(s.n[PSO_SYNC], 0);
        let s2 = take();
        assert_eq!(s2.n, [0; N]);
        let m = Mutex::new(5);
        assert_eq!(*lock(&m), 5);
        mark_async();
        assert!(take_async_mark());
        assert!(!take_async_mark());
        ACTIVE.store(false, Relaxed);
        assert!(begin(XLAT).is_none());
        let row = row(&s, Some(1.5));
        assert_eq!(row.split(',').count(), header().split(',').count());
        assert!(row.ends_with("1.500"));
    }

    #[test]
    fn game_dt_needs_two_clock_reads() {
        let s = Snap { qpc_last: qpf(), ..Default::default() };
        assert!(game_dt_ms(0, &s).is_none());
        assert!((game_dt_ms(qpf() / 2, &s).unwrap() - 500.0).abs() < 1e-6);
    }
}
