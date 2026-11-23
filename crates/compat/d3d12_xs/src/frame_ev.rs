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
pub const PSO_LIB_HIT: usize = 14;
pub const PSO_LIB_MISS: usize = 15;
pub const PREWARM_HIT: usize = 16;
pub const PREWARM_LATE: usize = 17;
pub const PREWARM_UNUSED: usize = 18;
pub const TITLE_WAIT: usize = 19;
pub const TITLE_SLEEP: usize = 20;
pub const N: usize = 21;
pub const NAMES: [&str; N] = ["pso_sync", "pso_inline", "pso_async", "pso_bind_wait", "xlat", "dxc", "committed", "placed", "heap", "upload", "io_read", "io_read_bg", "lock_wait", "frame_wait", "pso_lib_hit", "pso_lib_miss", "prewarm_hit", "prewarm_late", "prewarm_unused", "title_wait", "title_sleep"];

static ACTIVE: AtomicBool = AtomicBool::new(false);
static CNT: [AtomicU64; N] = [const { AtomicU64::new(0) }; N];
static TICKS: [AtomicU64; N] = [const { AtomicU64::new(0) }; N];
static BG: [AtomicU64; N] = [const { AtomicU64::new(0) }; N];
static BG_TICKS: [AtomicU64; N] = [const { AtomicU64::new(0) }; N];
static QPC_LAST: AtomicI64 = AtomicI64::new(0);
static QPC_CALLS: AtomicU64 = AtomicU64::new(0);
static UP_BYTES: AtomicU64 = AtomicU64::new(0);
static UP_COPIES: AtomicU64 = AtomicU64::new(0);
static GTC_CALLS: AtomicU64 = AtomicU64::new(0);
static PRESENT_TID: AtomicU32 = AtomicU32::new(0);
static ORIG_QPC: AtomicUsize = AtomicUsize::new(0);
static ORIG_GTC: AtomicUsize = AtomicUsize::new(0);
static ORIG_GTC64: AtomicUsize = AtomicUsize::new(0);
static ORIG_RF: AtomicUsize = AtomicUsize::new(0);
static ORIG_RFX: AtomicUsize = AtomicUsize::new(0);
static ORIG_WSO: AtomicUsize = AtomicUsize::new(0);
static ORIG_WSOX: AtomicUsize = AtomicUsize::new(0);
static ORIG_WMO: AtomicUsize = AtomicUsize::new(0);
static ORIG_WMOX: AtomicUsize = AtomicUsize::new(0);
static ORIG_SLEEP: AtomicUsize = AtomicUsize::new(0);
static ORIG_SLEEPX: AtomicUsize = AtomicUsize::new(0);
static ORIG_STT: AtomicUsize = AtomicUsize::new(0);
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

#[cfg(test)]
thread_local! {
    static T_ON: Cell<bool> = const { Cell::new(false) };
    static T_ACC: core::cell::RefCell<Snap> = core::cell::RefCell::new(Snap::default());
}

#[inline]
pub fn active() -> bool {
    #[cfg(test)]
    if T_ON.with(|c| c.get()) {
        return true;
    }
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

pub fn counts_on_title_frame(k: usize) -> bool {
    matches!(k, PSO_LIB_HIT | PSO_LIB_MISS | PREWARM_HIT | PREWARM_LATE | PREWARM_UNUSED)
}

pub fn note(k: usize, ticks: i64) {
    if active() {
        add(k, ticks, worker());
    }
}

fn add(k: usize, ticks: i64, bg: bool) {
    let t = ticks.max(0) as u64;
    let bg = bg && !counts_on_title_frame(k);
    #[cfg(test)]
    if T_ON.with(|c| c.get()) {
        T_ACC.with(|a| {
            let mut a = a.borrow_mut();
            if bg {
                a.bg_n[k] += 1;
                a.bg_ticks[k] += t;
            } else {
                a.n[k] += 1;
                a.ticks[k] += t;
            }
        });
        return;
    }
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

pub fn note_upload(bytes: u64, copies: u64) {
    #[cfg(test)]
    if T_ON.with(|c| c.get()) {
        T_ACC.with(|a| {
            let mut a = a.borrow_mut();
            a.upload_bytes += bytes;
            a.upload_copies += copies;
        });
        return;
    }
    if active() {
        UP_BYTES.fetch_add(bytes, Relaxed);
        UP_COPIES.fetch_add(copies, Relaxed);
    }
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
    pub upload_bytes: u64,
    pub upload_copies: u64,
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
    s.upload_bytes = UP_BYTES.swap(0, Relaxed);
    s.upload_copies = UP_COPIES.swap(0, Relaxed);
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
    h.push_str(",bg_pso_n,bg_xlat_n,bg_xlat_ms,bg_dxc_n,bg_dxc_ms,qpc_calls,gtc_calls,game_dt_ms,upload_bytes,upload_copies");
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
    r.push_str(&format!(",{},{}", s.upload_bytes, s.upload_copies));
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

fn on_present_thread() -> bool {
    let t = PRESENT_TID.load(Relaxed);
    t != 0 && unsafe { GetCurrentThreadId() } == t
}

fn timed<R>(k: usize, f: impl FnOnce() -> R) -> R {
    if !active() || !on_present_thread() {
        return f();
    }
    let t0 = qpc();
    let r = f();
    add(k, qpc() - t0, false);
    r
}

unsafe extern "system" fn wso_hook(h: *mut c_void, ms: u32) -> u32 {
    let f: unsafe extern "system" fn(*mut c_void, u32) -> u32 = unsafe { core::mem::transmute(ORIG_WSO.load(Relaxed)) };
    timed(TITLE_WAIT, || unsafe { f(h, ms) })
}

unsafe extern "system" fn wsox_hook(h: *mut c_void, ms: u32, alert: i32) -> u32 {
    let f: unsafe extern "system" fn(*mut c_void, u32, i32) -> u32 = unsafe { core::mem::transmute(ORIG_WSOX.load(Relaxed)) };
    timed(TITLE_WAIT, || unsafe { f(h, ms, alert) })
}

unsafe extern "system" fn wmo_hook(n: u32, hs: *const *mut c_void, all: i32, ms: u32) -> u32 {
    let f: unsafe extern "system" fn(u32, *const *mut c_void, i32, u32) -> u32 = unsafe { core::mem::transmute(ORIG_WMO.load(Relaxed)) };
    timed(TITLE_WAIT, || unsafe { f(n, hs, all, ms) })
}

unsafe extern "system" fn wmox_hook(n: u32, hs: *const *mut c_void, all: i32, ms: u32, alert: i32) -> u32 {
    let f: unsafe extern "system" fn(u32, *const *mut c_void, i32, u32, i32) -> u32 = unsafe { core::mem::transmute(ORIG_WMOX.load(Relaxed)) };
    timed(TITLE_WAIT, || unsafe { f(n, hs, all, ms, alert) })
}

unsafe extern "system" fn sleep_hook(ms: u32) {
    let f: unsafe extern "system" fn(u32) = unsafe { core::mem::transmute(ORIG_SLEEP.load(Relaxed)) };
    timed(TITLE_SLEEP, || unsafe { f(ms) })
}

unsafe extern "system" fn sleepx_hook(ms: u32, alert: i32) -> u32 {
    let f: unsafe extern "system" fn(u32, i32) -> u32 = unsafe { core::mem::transmute(ORIG_SLEEPX.load(Relaxed)) };
    timed(TITLE_SLEEP, || unsafe { f(ms, alert) })
}

unsafe extern "system" fn stt_hook() -> i32 {
    let f: unsafe extern "system" fn() -> i32 = unsafe { core::mem::transmute(ORIG_STT.load(Relaxed)) };
    timed(TITLE_SLEEP, || unsafe { f() })
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
        let targets: [(&str, usize, &AtomicUsize); 12] = [
            ("QueryPerformanceCounter", qpc_hook as usize, &ORIG_QPC),
            ("GetTickCount", gtc_hook as usize, &ORIG_GTC),
            ("GetTickCount64", gtc64_hook as usize, &ORIG_GTC64),
            ("ReadFile", read_file_hook as usize, &ORIG_RF),
            ("ReadFileEx", read_file_ex_hook as usize, &ORIG_RFX),
            ("WaitForSingleObject", wso_hook as usize, &ORIG_WSO),
            ("WaitForSingleObjectEx", wsox_hook as usize, &ORIG_WSOX),
            ("WaitForMultipleObjects", wmo_hook as usize, &ORIG_WMO),
            ("WaitForMultipleObjectsEx", wmox_hook as usize, &ORIG_WMOX),
            ("Sleep", sleep_hook as usize, &ORIG_SLEEP),
            ("SleepEx", sleepx_hook as usize, &ORIG_SLEEPX),
            ("SwitchToThread", stt_hook as usize, &ORIG_STT),
        ];
        let n = unsafe { compat_common::gdk::hook::patch_module(GetModuleHandleW(core::ptr::null()), &targets) };
        compat_common::record("d3d12_xs", "[frame_csv]", &format!("frame event counters on; title clock/file/wait probes patched {n} import slots in the title exe (QueryPerformanceCounter, GetTickCount[64], ReadFile[Ex], WaitForSingle/MultipleObjects[Ex] -> title_wait, Sleep[Ex]/SwitchToThread -> title_sleep; waits count only on the title's present thread)"));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn local(on: bool) -> Snap {
        T_ON.with(|c| c.set(on));
        T_ACC.with(|a| core::mem::take(&mut *a.borrow_mut()))
    }

    #[test]
    fn title_wait_and_sleep_columns_follow_the_present_thread_only() {
        let h = header();
        assert!(h.contains(",title_wait_n,title_wait_ms,title_sleep_n,title_sleep_ms,bg_pso_n"));
        let _ = local(true);
        PRESENT_TID.store(0, Relaxed);
        assert_eq!(timed(TITLE_WAIT, || 7), 7);
        assert_eq!(local(true).n[TITLE_WAIT], 0);
        note_present_thread();
        timed(TITLE_WAIT, || ());
        timed(TITLE_SLEEP, || ());
        timed(TITLE_SLEEP, || ());
        let s = local(false);
        assert_eq!((s.n[TITLE_WAIT], s.n[TITLE_SLEEP]), (1, 2));
        assert_eq!(s.bg_n[TITLE_WAIT] + s.bg_n[TITLE_SLEEP], 0);
        let other = std::thread::spawn(|| {
            T_ON.with(|c| c.set(true));
            timed(TITLE_WAIT, || ());
            T_ACC.with(|a| a.borrow().n[TITLE_WAIT])
        }).join().unwrap();
        assert_eq!(other, 0);
        PRESENT_TID.store(0, Relaxed);
    }

    #[test]
    fn pipeline_library_events_from_pso_workers_land_in_the_frame_columns() {
        let _ = local(true);
        add(PSO_LIB_HIT, 10, true);
        add(PSO_LIB_MISS, 20, true);
        add(PSO_SYNC, 30, true);
        let s = local(false);
        assert_eq!((s.n[PSO_LIB_HIT], s.ticks[PSO_LIB_HIT]), (1, 10));
        assert_eq!((s.n[PSO_LIB_MISS], s.ticks[PSO_LIB_MISS]), (1, 20));
        assert_eq!(s.bg_n[PSO_LIB_HIT] + s.bg_n[PSO_LIB_MISS], 0);
        assert_eq!((s.n[PSO_SYNC], s.bg_n[PSO_SYNC], s.bg_ticks[PSO_SYNC]), (0, 1, 30));
        let r = row(&s, None);
        let cols: Vec<&str> = r.split(',').collect();
        assert_eq!(cols[1 + 2 * PSO_LIB_HIT], "1");
        assert_eq!(cols[1 + 2 * PSO_LIB_MISS], "1");
    }

    #[test]
    fn scopes_attribute_counts_and_exclusive_time_to_the_right_class() {
        let _ = local(true);
        {
            let _outer = begin(XLAT);
            note_nested(DXC, 1000);
            let mut inner = begin(PSO_SYNC).unwrap();
            inner.set_kind(PSO_INLINE);
            mark_async();
        }
        let s = local(false);
        assert_eq!(s.n[XLAT], 1);
        assert_eq!((s.n[DXC], s.ticks[DXC]), (1, 1000));
        assert_eq!(s.n[PSO_INLINE], 1);
        assert_eq!(s.n[PSO_SYNC], 0);
        assert_eq!(s.n.iter().sum::<u64>(), 3);
        assert_eq!(s.bg_n, [0; N]);
        assert_eq!(local(false).n, [0; N]);
        let m = Mutex::new(5);
        assert_eq!(*lock(&m), 5);
        assert!(take_async_mark());
        assert!(!take_async_mark());
        let row = row(&s, Some(1.5));
        assert_eq!(row.split(',').count(), header().split(',').count());
        assert!(row.ends_with("1.500,0,0"));
    }

    #[test]
    fn upload_bytes_and_copies_are_the_last_two_columns() {
        let _ = local(true);
        note_upload(0x4000, 3);
        note_upload(0x1000, 2);
        let s = local(false);
        assert_eq!((s.upload_bytes, s.upload_copies), (0x5000, 5));
        let h = header();
        assert!(h.ends_with(",game_dt_ms,upload_bytes,upload_copies"));
        let r = row(&s, None);
        assert_eq!(r.split(',').count(), h.split(',').count());
        assert!(r.ends_with(",20480,5"), "{r}");
    }

    #[test]
    fn nested_time_is_never_charged_to_the_outer_scope() {
        let _ = local(true);
        {
            let _outer = begin(XLAT);
            note_nested(DXC, i64::MAX / 4);
        }
        let s = local(false);
        assert_eq!(s.n[XLAT], 1);
        assert_eq!(s.ticks[XLAT], 0);
        assert_eq!(s.ticks[DXC], (i64::MAX / 4) as u64);
    }

    #[test]
    fn inactive_threads_record_nothing_locally() {
        let _ = local(false);
        if !ACTIVE.load(Relaxed) {
            assert!(begin(XLAT).is_none());
        }
        T_ACC.with(|a| assert_eq!(a.borrow().n, [0; N]));
    }

    #[test]
    fn game_dt_needs_two_clock_reads() {
        let s = Snap { qpc_last: qpf(), ..Default::default() };
        assert!(game_dt_ms(0, &s).is_none());
        assert!((game_dt_ms(qpf() / 2, &s).unwrap() - 500.0).abs() < 1e-6);
    }
}
