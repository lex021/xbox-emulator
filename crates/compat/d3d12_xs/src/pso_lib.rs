use compat_common::record;
use core::ffi::c_void;
use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering::Relaxed};
use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock, RwLock};
use crate::device_wrap::Guid;

const MAGIC: &[u8; 4] = b"TBPL";
const FILE_VERSION: u32 = 2;
const KEY_VERSION: &[u8] = b"pso-lib-key v2 final-root";
const GFX_DESC: usize = 0x290;
const CS_DESC: usize = 0x38;
const E_INVALIDARG: i32 = 0x8007_0057_u32 as i32;
const ADAPTER_NOT_FOUND: i32 = 0x887e_0001_u32 as i32;
const DRIVER_MISMATCH: i32 = 0x887e_0002_u32 as i32;
const SAVE_SECS: u64 = 30;
const SAVE_FORCE_SECS: u64 = 120;
const SAVE_QUIET_SECS: u64 = 3;
const CACHED_SALT: u128 = 0x7462_706c_6361_6368_6564_7073_6f00_0001;

const IID_DEVICE1: Guid = Guid { d1: 0x77ac_ce80, d2: 0x638e, d3: 0x4e65, d4: [0x88, 0x95, 0xc1, 0xf2, 0x33, 0x86, 0x86, 0x3e] };
const IID_PSO: Guid = Guid { d1: 0x765a_30f3, d2: 0xf624, d3: 0x4c6f, d4: [0xa8, 0x28, 0xac, 0xe9, 0x48, 0x62, 0x24, 0x45] };
const IID_LIBRARY1: Guid = Guid { d1: 0x80ea_bf42, d2: 0x2568, d3: 0x4e5e, d4: [0xbd, 0x82, 0xc3, 0x7f, 0x86, 0x96, 0x1d, 0xc3] };

const DEV1_CREATE_LIBRARY: usize = 44;
const LIB_STORE: usize = 8;
const LIB_LOAD_GFX: usize = 9;
const LIB_LOAD_CS: usize = 10;
const LIB_SIZE: usize = 11;
const LIB_SERIALIZE: usize = 12;
const LIB_LOAD_STREAM: usize = 13;
const PSO_CACHED_BLOB: usize = 8;

static HITS: AtomicU64 = AtomicU64::new(0);
static MISSES: AtomicU64 = AtomicU64::new(0);
static STORED: AtomicU64 = AtomicU64::new(0);
static REJECTED: AtomicU64 = AtomicU64::new(0);
static UNKEYED: AtomicU64 = AtomicU64::new(0);
static CACHED_HITS: AtomicU64 = AtomicU64::new(0);
static CACHED_MISSES: AtomicU64 = AtomicU64::new(0);
static SAVED_AT: AtomicU64 = AtomicU64::new(0);
static SAVING: AtomicBool = AtomicBool::new(false);
static LAST_STORE_AT: AtomicU64 = AtomicU64::new(0);
static LOCK_WAITS: AtomicU64 = AtomicU64::new(0);
static LOCK_WAIT_US: AtomicU64 = AtomicU64::new(0);
static SAVE_HOLD_US: AtomicU64 = AtomicU64::new(0);

thread_local! {
    static WAITED_US: core::cell::Cell<u64> = const { core::cell::Cell::new(0) };
}

pub fn take_lock_wait_us() -> u64 {
    WAITED_US.with(|w| w.replace(0))
}

fn waited(t0: std::time::Instant) {
    let us = t0.elapsed().as_micros() as u64;
    LOCK_WAITS.fetch_add(1, Relaxed);
    LOCK_WAIT_US.fetch_add(us, Relaxed);
    WAITED_US.with(|w| w.set(w.get() + us));
}

fn read_lib(l: &Lib) -> std::sync::RwLockReadGuard<'_, ()> {
    match l.rw.try_read() {
        Ok(g) => g,
        Err(std::sync::TryLockError::Poisoned(e)) => e.into_inner(),
        Err(std::sync::TryLockError::WouldBlock) => {
            let _s = crate::frame_ev::begin(crate::frame_ev::LOCK);
            let t0 = std::time::Instant::now();
            let g = l.rw.read().unwrap_or_else(|e| e.into_inner());
            waited(t0);
            g
        }
    }
}

fn write_lib(l: &Lib) -> std::sync::RwLockWriteGuard<'_, ()> {
    match l.rw.try_write() {
        Ok(g) => g,
        Err(std::sync::TryLockError::Poisoned(e)) => e.into_inner(),
        Err(std::sync::TryLockError::WouldBlock) => {
            let _s = crate::frame_ev::begin(crate::frame_ev::LOCK);
            let t0 = std::time::Instant::now();
            let g = l.rw.write().unwrap_or_else(|e| e.into_inner());
            waited(t0);
            g
        }
    }
}

pub fn save_due(now: u64, saved_at: u64, last_store: u64) -> bool {
    let since = now.saturating_sub(saved_at);
    since >= SAVE_FORCE_SECS || (since >= SAVE_SECS && now.saturating_sub(last_store) >= SAVE_QUIET_SECS)
}
static PREV_RS: AtomicUsize = AtomicUsize::new(0);

pub fn enabled() -> bool {
    if cfg!(test) {
        return false;
    }
    static E: OnceLock<bool> = OnceLock::new();
    *E.get_or_init(|| {
        let on = std::env::var("TOMBOX_PSO_LIBRARY").map(|v| v.trim() != "0").unwrap_or(true);
        if !on {
            record("d3d12_xs", "pso_lib", "TOMBOX_PSO_LIBRARY=0: no persistent driver PSO cache, every PSO is compiled by the driver");
        }
        on
    })
}

unsafe fn vslot(obj: *mut c_void, i: usize) -> usize {
    unsafe { *(*(obj as *const *const usize)).add(i) }
}

unsafe fn release(obj: *mut c_void) {
    let f: unsafe extern "system" fn(*mut c_void) -> u32 = unsafe { core::mem::transmute(vslot(obj, 2)) };
    unsafe { f(obj) };
}

pub fn fnv64(b: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &x in b {
        h ^= x as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

struct Hasher(u64, u64);

impl Hasher {
    fn new() -> Hasher {
        let mut h = Hasher(0xcbf2_9ce4_8422_2325, 0x84222325_cbf29ce4 ^ 0x9e37_79b9_7f4a_7c15);
        h.feed(KEY_VERSION);
        h
    }

    fn feed(&mut self, b: &[u8]) {
        for &x in b {
            self.0 = (self.0 ^ x as u64).wrapping_mul(0x0000_0100_0000_01b3);
            self.1 = (self.1 ^ x as u64).wrapping_mul(0x0000_0100_0000_01b3);
        }
        let n = b.len() as u64;
        self.0 = (self.0 ^ n).wrapping_mul(0x0000_0100_0000_01b3);
        self.1 = (self.1 ^ n).wrapping_mul(0x0000_0100_0000_01b3);
    }

    fn done(self) -> u128 {
        ((self.0 as u128) << 64) | self.1 as u128
    }
}

fn rd64(d: &[u8], o: usize) -> u64 {
    u64::from_le_bytes(d[o..o + 8].try_into().unwrap())
}

fn rd32(d: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(d[o..o + 4].try_into().unwrap())
}

unsafe fn bytes<'a>(p: u64, n: usize) -> &'a [u8] {
    if p == 0 || n == 0 { &[] } else { unsafe { core::slice::from_raw_parts(p as *const u8, n) } }
}

unsafe fn cstr<'a>(p: u64) -> Option<&'a [u8]> {
    if p == 0 {
        return None;
    }
    let s = p as *const u8;
    let mut n = 0;
    while n < 256 {
        if unsafe { *s.add(n) } == 0 {
            return Some(unsafe { core::slice::from_raw_parts(s, n) });
        }
        n += 1;
    }
    None
}

pub unsafe fn gfx_key(desc: &[u8], root: u64, mode: &str) -> Option<u128> {
    if desc.len() < GFX_DESC {
        return None;
    }
    let mut h = Hasher::new();
    h.feed(b"gfx");
    h.feed(mode.as_bytes());
    let mut d = desc[..GFX_DESC].to_vec();
    for o in [0x00, 0x08, 0x18, 0x28, 0x38, 0x48, 0x58, 0x68, 0x228, 0x278, 0x280] {
        d[o..o + 8].fill(0);
    }
    h.feed(&d);
    h.feed(&root.to_le_bytes());
    for o in [0x08, 0x18, 0x28, 0x38, 0x48] {
        let (p, n) = (rd64(desc, o), rd64(desc, o + 8) as usize);
        if n > 64 << 20 {
            return None;
        }
        h.feed(&fnv64(unsafe { bytes(p, n) }).to_le_bytes());
    }
    let (so, so_n) = (rd64(desc, 0x58), rd32(desc, 0x60) as usize);
    if so_n > 64 || (so == 0 && so_n != 0) {
        return None;
    }
    for i in 0..so_n {
        let e = unsafe { bytes(so + (i * 0x18) as u64, 0x18) };
        h.feed(&e[0..4]);
        h.feed(unsafe { cstr(rd64(e, 8)) }?);
        h.feed(&e[16..24]);
    }
    let (st, st_n) = (rd64(desc, 0x68), rd32(desc, 0x70) as usize);
    if st_n > 8 || (st == 0 && st_n != 0) {
        return None;
    }
    h.feed(unsafe { bytes(st, st_n * 4) });
    let (il, il_n) = (rd64(desc, 0x228), rd32(desc, 0x230) as usize);
    if il_n > 64 || (il == 0 && il_n != 0) {
        return None;
    }
    for i in 0..il_n {
        let e = unsafe { bytes(il + (i * 0x20) as u64, 0x20) };
        h.feed(unsafe { cstr(rd64(e, 0)) }?);
        h.feed(&e[8..0x20]);
    }
    Some(h.done())
}

pub unsafe fn compute_key(desc: &[u8], root: u64, mode: &str) -> Option<u128> {
    if desc.len() < CS_DESC {
        return None;
    }
    let mut h = Hasher::new();
    h.feed(b"cs");
    h.feed(mode.as_bytes());
    let mut d = desc[..CS_DESC].to_vec();
    for o in [0x00, 0x08, 0x20, 0x28] {
        d[o..o + 8].fill(0);
    }
    h.feed(&d);
    h.feed(&root.to_le_bytes());
    let (p, n) = (rd64(desc, 0x08), rd64(desc, 0x10) as usize);
    if n > 64 << 20 {
        return None;
    }
    h.feed(&fnv64(unsafe { bytes(p, n) }).to_le_bytes());
    Some(h.done())
}

pub fn stream_key(gfx: u128, extra: &str) -> u128 {
    let mut h = Hasher::new();
    h.feed(b"stream");
    h.feed(&gfx.to_le_bytes());
    h.feed(extra.as_bytes());
    h.done()
}

struct RootInfo {
    hash: u64,
    mask: u32,
    blob: std::sync::Arc<[u8]>,
    via_real: bool,
}

fn roots() -> &'static Mutex<HashMap<usize, RootInfo>> {
    static R: OnceLock<Mutex<HashMap<usize, RootInfo>>> = OnceLock::new();
    R.get_or_init(|| Mutex::new(HashMap::new()))
}

fn real_roots() -> &'static Mutex<HashMap<usize, u64>> {
    static R: OnceLock<Mutex<HashMap<usize, u64>>> = OnceLock::new();
    R.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn combine_root(given: u64, real: u64) -> u64 {
    if real == 0 {
        given
    } else {
        given ^ real.rotate_left(29).wrapping_mul(0x9e37_79b9_7f4a_7c15)
    }
}

pub fn note_real_root(rs: usize, blob: &[u8]) {
    if rs == 0 {
        return;
    }
    real_roots().lock().unwrap_or_else(|e| e.into_inner()).insert(rs, fnv64(blob) | 1);
    roots().lock().unwrap_or_else(|e| e.into_inner()).remove(&rs);
}

pub fn root_real(rs: usize) -> u64 {
    real_roots().lock().unwrap_or_else(|e| e.into_inner()).get(&rs).copied().unwrap_or(0)
}

pub fn note_root(rs: usize, mask: u32, blob: &[u8], via_real: bool) {
    if rs != 0 {
        let hash = combine_root(fnv64(blob) ^ mask as u64, root_real(rs));
        roots().lock().unwrap_or_else(|e| e.into_inner()).insert(rs, RootInfo { hash, mask, blob: std::sync::Arc::from(blob), via_real });
    }
}

pub fn root_info(rs: usize) -> Option<(u32, std::sync::Arc<[u8]>, bool)> {
    roots().lock().unwrap_or_else(|e| e.into_inner()).get(&rs).map(|r| (r.mask, r.blob.clone(), r.via_real))
}

fn root_hash(rs: u64) -> Option<u64> {
    if rs == 0 {
        return Some(0);
    }
    if let Some(h) = roots().lock().unwrap_or_else(|e| e.into_inner()).get(&(rs as usize)).map(|r| r.hash) {
        return Some(h);
    }
    match root_real(rs as usize) {
        0 => None,
        real => Some(combine_root(0, real)),
    }
}

pub fn root_key_hash(rs: usize) -> Option<u64> {
    root_hash(rs as u64)
}

unsafe extern "system" fn create_root_signature(this: *mut c_void, mask: u32, blob: *const u8, len: usize, riid: *const c_void, ppv: *mut *mut c_void) -> i32 {
    let f: unsafe extern "system" fn(*mut c_void, u32, *const u8, usize, *const c_void, *mut *mut c_void) -> i32 = unsafe { core::mem::transmute(PREV_RS.load(Relaxed)) };
    let hr = unsafe { f(this, mask, blob, len, riid, ppv) };
    if hr >= 0 && !ppv.is_null() && !blob.is_null() && len != 0 {
        note_root(unsafe { *ppv } as usize, mask, unsafe { core::slice::from_raw_parts(blob, len) }, false);
    }
    hr
}

pub fn hook_device(v: &mut [usize]) {
    if v.len() <= 16 || v[16] == 0 || v[16] == create_root_signature as *const () as usize {
        return;
    }
    PREV_RS.store(v[16], Relaxed);
    v[16] = create_root_signature as *const () as usize;
}

pub fn cache_file(stem: &str, mode: &str) -> String {
    let tag: String = mode.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
    format!("{stem}.{tag}.bin")
}

pub(crate) fn mode_tag() -> &'static str {
    crate::shader_xlat::runtime_mode().cbv_raw.key_tag()
}

#[derive(Debug, PartialEq, Eq)]
pub enum FileCheck<'a> {
    Empty,
    Blob(&'a [u8]),
    Rebuild(String),
}

pub fn check_file<'a>(file: Option<&'a [u8]>, ident: &str) -> FileCheck<'a> {
    let Some(b) = file else { return FileCheck::Empty };
    if b.len() < 12 || &b[0..4] != MAGIC {
        return FileCheck::Rebuild("not a TomBox pipeline library file".into());
    }
    let ver = u32::from_le_bytes(b[4..8].try_into().unwrap());
    if ver != FILE_VERSION {
        return FileCheck::Rebuild(format!("file version {ver}, this build writes {FILE_VERSION}"));
    }
    let n = u32::from_le_bytes(b[8..12].try_into().unwrap()) as usize;
    let Some(id) = b.get(12..12 + n) else { return FileCheck::Rebuild("truncated header".into()) };
    if id != ident.as_bytes() {
        return FileCheck::Rebuild(format!("built for [{}], this run is [{ident}]", String::from_utf8_lossy(id)));
    }
    let blob = &b[12 + n..];
    if blob.is_empty() { FileCheck::Empty } else { FileCheck::Blob(blob) }
}

pub fn file_bytes(ident: &str, blob: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(12 + ident.len() + blob.len());
    v.extend_from_slice(MAGIC);
    v.extend_from_slice(&FILE_VERSION.to_le_bytes());
    v.extend_from_slice(&(ident.len() as u32).to_le_bytes());
    v.extend_from_slice(ident.as_bytes());
    v.extend_from_slice(blob);
    v
}

#[derive(Debug, PartialEq, Eq)]
pub enum OnCreate {
    Use,
    RebuildEmpty,
    Unavailable,
}

pub fn on_create_hr(hr: i32, had_blob: bool) -> OnCreate {
    match (hr >= 0, had_blob) {
        (true, _) => OnCreate::Use,
        (false, true) => OnCreate::RebuildEmpty,
        (false, false) => OnCreate::Unavailable,
    }
}

struct Lib {
    lib: usize,
    rw: RwLock<()>,
    path: std::path::PathBuf,
    ident: String,
    rejected: Mutex<HashSet<u128>>,
    stored: Mutex<HashSet<u128>>,
}

unsafe impl Send for Lib {}
unsafe impl Sync for Lib {}

static LIB: OnceLock<Option<Lib>> = OnceLock::new();

fn ident() -> String {
    let p = tombox_xshader::fma_probe::probe();
    format!("{} | {}", p.adapter, mode_tag())
}

unsafe fn luid(real: *mut c_void) -> u64 {
    let f: unsafe extern "system" fn(*mut c_void, *mut u64) -> *mut u64 = unsafe { core::mem::transmute(vslot(real, 43)) };
    let mut l = 0u64;
    unsafe { f(real, &mut l) };
    l
}

unsafe fn create_library(dev1: *mut c_void, blob: &[u8]) -> (i32, usize) {
    let f: unsafe extern "system" fn(*mut c_void, *const u8, usize, *const Guid, *mut *mut c_void) -> i32 = unsafe { core::mem::transmute(vslot(dev1, DEV1_CREATE_LIBRARY)) };
    let mut out: *mut c_void = core::ptr::null_mut();
    let hr = unsafe { f(dev1, if blob.is_empty() { core::ptr::null() } else { blob.as_ptr() }, blob.len(), &IID_LIBRARY1, &mut out) };
    (hr, out as usize)
}

unsafe fn open(real: *mut c_void) -> Option<Lib> {
    let t0 = std::time::Instant::now();
    let qi: unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> i32 = unsafe { core::mem::transmute(vslot(real, 0)) };
    let mut dev1: *mut c_void = core::ptr::null_mut();
    if unsafe { qi(real, &IID_DEVICE1, &mut dev1) } < 0 || dev1.is_null() {
        record("d3d12_xs", "pso_lib", "device has no ID3D12Device1; PSOs fall back to CachedPSO blobs");
        return None;
    }
    let ident = ident();
    let path = tombox_xshader::shader_cache::title_dir(&crate::shader_xlat::title_name_pub()).join(cache_file("pso_library", mode_tag()));
    let file = std::fs::read(&path).ok();
    let check = check_file(file.as_deref(), &ident);
    let (blob, why): (&'static [u8], Option<String>) = match check {
        FileCheck::Empty => (&[], None),
        FileCheck::Rebuild(w) => (&[], Some(w)),
        FileCheck::Blob(b) => (Box::leak(b.to_vec().into_boxed_slice()), None),
    };
    drop(file);
    let (mut hr, mut lib) = unsafe { create_library(dev1, blob) };
    let mut note = why.map(|w| format!("dropped the stored library ({w}); rebuilding"));
    if on_create_hr(hr, !blob.is_empty()) == OnCreate::RebuildEmpty {
        let what = match hr {
            DRIVER_MISMATCH => "driver version changed",
            ADAPTER_NOT_FOUND => "adapter changed",
            E_INVALIDARG => "blob rejected",
            _ => "load failed",
        };
        note = Some(format!("driver rejected the stored library hr={hr:#010x} ({what}); rebuilding"));
        (hr, lib) = unsafe { create_library(dev1, &[]) };
    }
    unsafe { release(dev1) };
    if hr < 0 || lib == 0 {
        record("d3d12_xs", "pso_lib", &format!("CreatePipelineLibrary failed hr={hr:#010x}; PSOs fall back to CachedPSO blobs"));
        return None;
    }
    record("d3d12_xs", "pso_lib", &format!(
        "pipeline library {} ({} bytes loaded in {:.0} ms, adapter luid {:016x}, {}){}; TOMBOX_PSO_LIBRARY=0 disables",
        path.display(), blob.len(), t0.elapsed().as_secs_f64() * 1000.0, unsafe { luid(real) }, ident, note.map(|n| format!("; {n}")).unwrap_or_default()));
    SAVED_AT.store(now_s(), Relaxed);
    register_exit_save();
    Some(Lib { lib, rw: RwLock::new(()), path, ident, rejected: Mutex::new(HashSet::new()), stored: Mutex::new(HashSet::new()) })
}

fn now_s() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn lib(real: *mut c_void) -> Option<&'static Lib> {
    LIB.get_or_init(|| unsafe { open(real) }).as_ref()
}

pub fn warm(real: *mut c_void, wrapper: *mut c_void) {
    if !enabled() || real.is_null() || LIB.get().is_some() {
        return;
    }
    let (r, w) = (real as usize, wrapper as usize);
    let addref: unsafe extern "system" fn(*mut c_void) -> u32 = unsafe { core::mem::transmute(vslot(real, 1)) };
    unsafe { addref(real) };
    let spawned = std::thread::Builder::new().name("tombox-pso-lib".into()).spawn(move || {
        match lib(r as *mut c_void).and_then(|l| l.path.parent().map(|d| (l, d))) {
            Some((l, dir)) => {
                crate::pso_prewarm::start(r as *mut c_void, w as *mut c_void, &l.ident, dir);
                crate::pso_xrec::start(w as *mut c_void, dir);
            }
            None => unsafe { release(r as *mut c_void) },
        }
    });
    if spawned.is_err() {
        unsafe { release(real) };
    }
}

fn wide(k: u128) -> [u16; 33] {
    let mut w = [0u16; 33];
    for (i, c) in format!("{k:032x}").bytes().enumerate() {
        w[i] = c as u16;
    }
    w
}

fn note_counts() {
    let n = HITS.load(Relaxed) + MISSES.load(Relaxed);
    if n == 1 || n % 200 == 0 {
        record("d3d12_xs", "pso_lib", &summary());
    }
}

pub fn summary() -> String {
    format!("pipeline library: {} hits, {} misses (compiled by the driver), {} stored, {} rejected loads, {} unkeyed; CachedPSO fallback {} hits {} misses; {} library lock waits {:.1} ms total",
        HITS.load(Relaxed), MISSES.load(Relaxed), STORED.load(Relaxed), REJECTED.load(Relaxed), UNKEYED.load(Relaxed), CACHED_HITS.load(Relaxed), CACHED_MISSES.load(Relaxed), LOCK_WAITS.load(Relaxed), LOCK_WAIT_US.load(Relaxed) as f64 / 1000.0)
}

unsafe fn try_load(l: &Lib, k: u128, load: &dyn Fn(usize, *const u16) -> i32) -> bool {
    if l.rejected.lock().unwrap_or_else(|e| e.into_inner()).contains(&k) {
        return false;
    }
    let w = wide(k);
    let _r = read_lib(l);
    let mut ev = crate::frame_ev::begin(crate::frame_ev::PSO_LIB_HIT);
    let hr = load(l.lib, w.as_ptr());
    if hr < 0 {
        if let Some(e) = ev.as_mut() {
            e.set_kind(crate::frame_ev::PSO_LIB_MISS);
        }
    }
    hr >= 0
}

unsafe fn store(l: &Lib, k: u128, pso: *mut c_void) {
    if !l.stored.lock().unwrap_or_else(|e| e.into_inner()).insert(k) {
        return;
    }
    let w = wide(k);
    let hr = {
        let _w = write_lib(l);
        let f: unsafe extern "system" fn(*mut c_void, *const u16, *mut c_void) -> i32 = unsafe { core::mem::transmute(vslot(l.lib as *mut c_void, LIB_STORE)) };
        unsafe { f(l.lib as *mut c_void, w.as_ptr(), pso) }
    };
    if hr >= 0 {
        let n = STORED.fetch_add(1, Relaxed) + 1;
        let _ = n;
        LAST_STORE_AT.store(now_s(), Relaxed);
    } else {
        l.rejected.lock().unwrap_or_else(|e| e.into_inner()).insert(k);
        let n = REJECTED.fetch_add(1, Relaxed);
        if n < 8 {
            record("d3d12_xs", "pso_lib", &format!("StorePipeline {k:032x} hr={hr:#010x}: the stored entry under this key does not match this PSO; it uses CachedPSO blobs from now on"));
        }
    }
}

unsafe fn cached_blob(pso: *mut c_void) -> Option<Vec<u8>> {
    let f: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> i32 = unsafe { core::mem::transmute(vslot(pso, PSO_CACHED_BLOB)) };
    let mut b: *mut c_void = core::ptr::null_mut();
    if unsafe { f(pso, &mut b) } < 0 || b.is_null() {
        return None;
    }
    let ptr: unsafe extern "system" fn(*mut c_void) -> *const u8 = unsafe { core::mem::transmute(vslot(b, 3)) };
    let len: unsafe extern "system" fn(*mut c_void) -> usize = unsafe { core::mem::transmute(vslot(b, 4)) };
    let (p, n) = unsafe { (ptr(b), len(b)) };
    let v = if p.is_null() || n == 0 { None } else { Some(unsafe { core::slice::from_raw_parts(p, n) }.to_vec()) };
    unsafe { release(b) };
    v
}

unsafe fn with_cached(k: u128, desc: &[u8], at: usize, ppv: *mut *mut c_void, create: &dyn Fn(*const u8) -> i32) -> i32 {
    let ck = k ^ CACHED_SALT;
    if let Some(b) = tombox_xshader::shader_cache::get(ck).filter(|b| !b.is_empty()) {
        let mut d = desc.to_vec();
        d[at..at + 8].copy_from_slice(&(b.as_ptr() as u64).to_le_bytes());
        d[at + 8..at + 16].copy_from_slice(&(b.len() as u64).to_le_bytes());
        let hr = create(d.as_ptr());
        if hr >= 0 {
            CACHED_HITS.fetch_add(1, Relaxed);
            return hr;
        }
    }
    CACHED_MISSES.fetch_add(1, Relaxed);
    let hr = create(desc.as_ptr());
    if hr >= 0 && !ppv.is_null() && !unsafe { *ppv }.is_null() && rd64(desc, at) == 0 {
        if let Some(b) = unsafe { cached_blob(*ppv) } {
            let _ = tombox_xshader::shader_cache::put(ck, &b);
        }
    }
    hr
}

static FLIGHTS: crate::pso_xrec::Flights = crate::pso_xrec::Flights::new();
static FLIGHT_WAITS: AtomicU64 = AtomicU64::new(0);

pub fn flight_waits() -> u64 {
    FLIGHT_WAITS.load(Relaxed)
}

unsafe fn run(key: Option<u128>, kind: Option<u8>, real: *mut c_void, desc: &[u8], cached_at: Option<usize>, riid: *const Guid, ppv: *mut *mut c_void, load: &dyn Fn(usize, *const u16) -> i32, create: &dyn Fn(*const u8) -> i32) -> i32 {
    if !enabled() {
        return create(desc.as_ptr());
    }
    let Some(k) = key else {
        UNKEYED.fetch_add(1, Relaxed);
        return create(desc.as_ptr());
    };
    if kind.is_some() && !crate::pso_xrec::replaying() {
        if let Some(hr) = unsafe { crate::pso_prewarm::claim(k, riid, ppv) } {
            return hr;
        }
    }
    let (_flight, waited) = FLIGHTS.enter(k);
    if waited {
        FLIGHT_WAITS.fetch_add(1, Relaxed);
    }
    let l = lib(real);
    if let Some(l) = l {
        if unsafe { try_load(l, k, load) } {
            HITS.fetch_add(1, Relaxed);
            note_counts();
            if let Some(kd) = kind {
                unsafe { crate::pso_prewarm::note(kd, k, desc) };
            }
            return 0;
        }
    }
    MISSES.fetch_add(1, Relaxed);
    note_counts();
    let rejected = l.is_some_and(|l| l.rejected.lock().unwrap_or_else(|e| e.into_inner()).contains(&k));
    let hr = match cached_at {
        Some(at) if l.is_none() || rejected => unsafe { with_cached(k, desc, at, ppv, create) },
        _ => create(desc.as_ptr()),
    };
    if let (Some(l), false) = (l, rejected) {
        if hr >= 0 && !ppv.is_null() && !unsafe { *ppv }.is_null() {
            unsafe { store(l, k, *ppv) };
        }
    }
    if let (Some(kd), true) = (kind, hr >= 0) {
        unsafe { crate::pso_prewarm::note(kd, k, desc) };
    }
    hr
}

pub(crate) unsafe fn prebuild(real: *mut c_void, kind: u8, k: u128, desc: &[u8]) -> (Option<usize>, bool) {
    let gfx = kind == crate::pso_prewarm::GFX;
    let mut pso: *mut c_void = core::ptr::null_mut();
    let l = match LIB.get() {
        Some(Some(l)) => Some(l),
        _ => None,
    };
    let rejected = l.is_some_and(|l| l.rejected.lock().unwrap_or_else(|e| e.into_inner()).contains(&k));
    if let (Some(l), false) = (l, rejected) {
        let w = wide(k);
        let f: unsafe extern "system" fn(*mut c_void, *const u16, *const u8, *const Guid, *mut *mut c_void) -> i32 = unsafe { core::mem::transmute(vslot(l.lib as *mut c_void, if gfx { LIB_LOAD_GFX } else { LIB_LOAD_CS })) };
        let hr = {
            let _r = read_lib(l);
            unsafe { f(l.lib as *mut c_void, w.as_ptr(), desc.as_ptr(), &IID_PSO, &mut pso) }
        };
        if hr >= 0 && !pso.is_null() {
            return (Some(pso as usize), true);
        }
        pso = core::ptr::null_mut();
    }
    let f: unsafe extern "system" fn(*mut c_void, *const u8, *const Guid, *mut *mut c_void) -> i32 = unsafe { core::mem::transmute(vslot(real, if gfx { 10 } else { 11 })) };
    let hr = unsafe { f(real, desc.as_ptr(), &IID_PSO, &mut pso) };
    if hr < 0 || pso.is_null() {
        return (None, false);
    }
    if let (Some(l), false) = (l, rejected) {
        unsafe { store(l, k, pso) };
    }
    (Some(pso as usize), false)
}

pub unsafe fn gfx(real: *mut c_void, desc: &[u8], riid: *const Guid, ppv: *mut *mut c_void, create: &dyn Fn(*const u8) -> i32) -> i32 {
    if !enabled() {
        return create(desc.as_ptr());
    }
    let key = root_hash(rd64(desc, 0)).and_then(|r| unsafe { gfx_key(desc, r, mode_tag()) });
    let load = |lib: usize, name: *const u16| -> i32 {
        let f: unsafe extern "system" fn(*mut c_void, *const u16, *const u8, *const Guid, *mut *mut c_void) -> i32 = unsafe { core::mem::transmute(vslot(lib as *mut c_void, LIB_LOAD_GFX)) };
        unsafe { f(lib as *mut c_void, name, desc.as_ptr(), riid, ppv) }
    };
    unsafe { run(key, Some(crate::pso_prewarm::GFX), real, desc, Some(0x278), riid, ppv, &load, create) }
}

pub unsafe fn compute(real: *mut c_void, desc: &[u8], riid: *const Guid, ppv: *mut *mut c_void, create: &dyn Fn(*const u8) -> i32) -> i32 {
    if !enabled() {
        return create(desc.as_ptr());
    }
    let key = root_hash(rd64(desc, 0)).and_then(|r| unsafe { compute_key(desc, r, mode_tag()) });
    let load = |lib: usize, name: *const u16| -> i32 {
        let f: unsafe extern "system" fn(*mut c_void, *const u16, *const u8, *const Guid, *mut *mut c_void) -> i32 = unsafe { core::mem::transmute(vslot(lib as *mut c_void, LIB_LOAD_CS)) };
        unsafe { f(lib as *mut c_void, name, desc.as_ptr(), riid, ppv) }
    };
    unsafe { run(key, Some(crate::pso_prewarm::CS), real, desc, Some(0x20), riid, ppv, &load, create) }
}

pub unsafe fn stream(real: *mut c_void, desc: &[u8], extra: &str, sd: *const [usize; 2], riid: *const Guid, ppv: *mut *mut c_void, create: &dyn Fn() -> i32) -> i32 {
    if !enabled() {
        return create();
    }
    let key = root_hash(rd64(desc, 0)).and_then(|r| unsafe { gfx_key(desc, r, mode_tag()) }).map(|g| stream_key(g, extra));
    let load = |lib: usize, name: *const u16| -> i32 {
        let f: unsafe extern "system" fn(*mut c_void, *const u16, *const [usize; 2], *const Guid, *mut *mut c_void) -> i32 = unsafe { core::mem::transmute(vslot(lib as *mut c_void, LIB_LOAD_STREAM)) };
        unsafe { f(lib as *mut c_void, name, sd, riid, ppv) }
    };
    unsafe { run(key, None, real, desc, None, riid, ppv, &load, &|_| create()) }
}

fn save(exiting: bool) -> Result<String, String> {
    if crate::dred::removed() {
        return Err("device removed; the pipeline library is not serialized or written".into());
    }
    let Some(Some(l)) = LIB.get() else { return Err("no library".into()) };
    let blob = {
        let _w = if exiting {
            match l.rw.try_write() {
                Ok(g) => g,
                Err(std::sync::TryLockError::Poisoned(e)) => e.into_inner(),
                Err(std::sync::TryLockError::WouldBlock) => return Err("a PSO thread held the library at exit; keeping the last periodic save".into()),
            }
        } else {
            write_lib(l)
        };
        let held = std::time::Instant::now();
        let size: unsafe extern "system" fn(*mut c_void) -> usize = unsafe { core::mem::transmute(vslot(l.lib as *mut c_void, LIB_SIZE)) };
        let n = unsafe { size(l.lib as *mut c_void) };
        if n == 0 {
            return Err("empty library".into());
        }
        let mut v = vec![0u8; n];
        let ser: unsafe extern "system" fn(*mut c_void, *mut u8, usize) -> i32 = unsafe { core::mem::transmute(vslot(l.lib as *mut c_void, LIB_SERIALIZE)) };
        let hr = unsafe { ser(l.lib as *mut c_void, v.as_mut_ptr(), n) };
        if hr < 0 {
            return Err(format!("Serialize hr={hr:#010x}"));
        }
        SAVE_HOLD_US.store(held.elapsed().as_micros() as u64, Relaxed);
        v
    };
    let bytes = file_bytes(&l.ident, &blob);
    if let Some(d) = l.path.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let tmp = l.path.with_extension(format!("bin.{}", std::process::id()));
    std::fs::write(&tmp, &bytes).map_err(|e| format!("write {}: {e}", tmp.display()))?;
    if let Err(e) = std::fs::rename(&tmp, &l.path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("rename to {}: {e}", l.path.display()));
    }
    SAVED_AT.store(now_s(), Relaxed);
    Ok(format!("saved {} bytes to {}", bytes.len(), l.path.display()))
}

fn save_logged(why: &str) {
    if SAVING.swap(true, Relaxed) {
        return;
    }
    let stored = STORED.load(Relaxed);
    let r = save(why == "process exit");
    SAVING.store(false, Relaxed);
    match r {
        Ok(m) => record("d3d12_xs", "pso_lib", &format!("{m} ({why}; {stored} PSOs stored this run; the library was locked {:.1} ms for Serialize); {}", SAVE_HOLD_US.load(Relaxed) as f64 / 1000.0, summary())),
        Err(e) => record("d3d12_xs", "pso_lib", &format!("save ({why}) failed: {e}")),
    }
}

fn save_async() {
    if SAVING.load(Relaxed) {
        return;
    }
    let _ = std::thread::Builder::new().name("tombox-pso-lib-save".into()).spawn(|| save_logged("periodic"));
}

static LAST_SAVED_STORED: AtomicU64 = AtomicU64::new(0);
static LAST_SUMMARY: AtomicU64 = AtomicU64::new(0);
pub const SUMMARY_SECS: u64 = 10;

pub fn summary_due(now: u64, last: u64) -> bool {
    last == 0 || now.saturating_sub(last) >= SUMMARY_SECS
}

pub fn on_present() {
    if !enabled() || LIB.get().is_none() {
        return;
    }
    let now = now_s();
    let last = LAST_SUMMARY.load(Relaxed);
    if summary_due(now, last) && LAST_SUMMARY.compare_exchange(last, now, Relaxed, Relaxed).is_ok() {
        record("d3d12_xs", "pso_lib", &format!("totals: {}", summary()));
        if crate::pso_prewarm::enabled() {
            record("d3d12_xs", "pso_prewarm", &format!("totals: {}", crate::pso_prewarm::summary()));
        }
    }
    crate::pso_prewarm::on_present();
    static PW_SAVED: AtomicU64 = AtomicU64::new(0);
    let ps = PW_SAVED.load(Relaxed);
    if now.saturating_sub(ps) >= SAVE_SECS && crate::pso_prewarm::dirty() && PW_SAVED.compare_exchange(ps, now, Relaxed, Relaxed).is_ok() {
        let _ = std::thread::Builder::new().name("tombox-pso-prewarm-save".into()).spawn(|| crate::pso_prewarm::save(false));
    }
    if !matches!(LIB.get(), Some(Some(_))) {
        return;
    }
    let s = STORED.load(Relaxed);
    if s != LAST_SAVED_STORED.load(Relaxed) && save_due(now, SAVED_AT.load(Relaxed), LAST_STORE_AT.load(Relaxed)) {
        LAST_SAVED_STORED.store(s, Relaxed);
        save_async();
    }
}

extern "C" fn exit_save() {
    if STORED.load(Relaxed) != 0 {
        save_logged("process exit");
    }
    let _ = crate::pso_prewarm::save(true);
    crate::pso_xrec::save_at_exit();
    if crate::pso_xrec::enabled() {
        record("d3d12_xs", "pso_xrec", &format!("final totals at process exit: {}; same-PSO creates that waited for another thread's build: {}", crate::pso_xrec::summary(), flight_waits()));
    }
    if crate::pso_prewarm::enabled() {
        record("d3d12_xs", "pso_prewarm", &format!("final totals at process exit: {}", crate::pso_prewarm::summary()));
    }
    record("d3d12_xs", "pso_lib", &format!("final totals at process exit: {}", summary()));
}

fn register_exit_save() {
    extern "C" {
        fn atexit(f: extern "C" fn()) -> i32;
    }
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| unsafe {
        atexit(exit_save);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wr64(d: &mut [u8], o: usize, v: u64) {
        d[o..o + 8].copy_from_slice(&v.to_le_bytes());
    }

    #[test]
    fn key_follows_content_not_pointers() {
        let vs = vec![1u8, 2, 3, 4];
        let vs2 = vs.clone();
        let name = b"POSITION\0".to_vec();
        let name2 = name.clone();
        let mut el = vec![0u8; 0x20];
        wr64(&mut el, 0, name.as_ptr() as u64);
        el[0xc] = 6;
        let mut el2 = el.clone();
        wr64(&mut el2, 0, name2.as_ptr() as u64);
        let mk = |code: &[u8], il: &[u8]| {
            let mut d = vec![0u8; GFX_DESC];
            wr64(&mut d, 0x08, code.as_ptr() as u64);
            wr64(&mut d, 0x10, code.len() as u64);
            wr64(&mut d, 0x228, il.as_ptr() as u64);
            d[0x230] = 1;
            d[0x240] = 1;
            d
        };
        let (a, b) = (mk(&vs, &el), mk(&vs2, &el2));
        let ka = unsafe { gfx_key(&a, 7, "cbraw=0") }.unwrap();
        assert_eq!(Some(ka), unsafe { gfx_key(&b, 7, "cbraw=0") });
        assert_ne!(Some(ka), unsafe { gfx_key(&a, 8, "cbraw=0") });
        assert_ne!(Some(ka), unsafe { gfx_key(&a, 7, "cbraw=1") });
        let mut c = a.clone();
        c[0x244] = 28;
        assert_ne!(Some(ka), unsafe { gfx_key(&c, 7, "cbraw=0") });
        let mut cached = a.clone();
        wr64(&mut cached, 0x278, 0x1234);
        wr64(&mut cached, 0x280, 99);
        assert_eq!(Some(ka), unsafe { gfx_key(&cached, 7, "cbraw=0") });
        let vs3 = vec![1u8, 2, 3, 5];
        assert_ne!(Some(ka), unsafe { gfx_key(&mk(&vs3, &el), 7, "cbraw=0") });
        let mut bad = a.clone();
        bad[0x230] = 1;
        wr64(&mut bad, 0x228, 0);
        assert_eq!(unsafe { gfx_key(&bad, 7, "cbraw=0") }, None);
        assert_ne!(stream_key(ka, "x"), stream_key(ka, "y"));
        let mut cs = vec![0u8; CS_DESC];
        wr64(&mut cs, 0x08, vs.as_ptr() as u64);
        wr64(&mut cs, 0x10, 4);
        let k1 = unsafe { compute_key(&cs, 1, "cbraw=0") }.unwrap();
        wr64(&mut cs, 0x08, vs2.as_ptr() as u64);
        assert_eq!(Some(k1), unsafe { compute_key(&cs, 1, "cbraw=0") });
        assert_ne!(k1, ka);
    }

    #[test]
    fn stored_library_is_dropped_on_any_identity_or_version_change() {
        let ident = "NVIDIA X [10de:2d05] driver 32.0.16.1742 | cbraw=0";
        let f = file_bytes(ident, b"blob");
        assert_eq!(check_file(Some(&f), ident), FileCheck::Blob(b"blob"));
        assert_eq!(check_file(None, ident), FileCheck::Empty);
        assert!(matches!(check_file(Some(&f), "NVIDIA X [10de:2d05] driver 32.0.16.1800 | cbraw=0"), FileCheck::Rebuild(_)));
        assert!(matches!(check_file(Some(&f), "NVIDIA X [10de:2d05] driver 32.0.16.1742 | cbraw=1"), FileCheck::Rebuild(_)));
        let mut old = f.clone();
        old[4] = 9;
        assert!(matches!(check_file(Some(&old), ident), FileCheck::Rebuild(_)));
        assert!(matches!(check_file(Some(&f[..14]), ident), FileCheck::Rebuild(_)));
        assert!(matches!(check_file(Some(b"junkjunkjunkjunk"), ident), FileCheck::Rebuild(_)));
        assert_eq!(check_file(Some(&file_bytes(ident, b"")), ident), FileCheck::Empty);
    }

    #[test]
    fn driver_rejection_rebuilds_and_missing_support_disables() {
        assert_eq!(on_create_hr(0, true), OnCreate::Use);
        assert_eq!(on_create_hr(DRIVER_MISMATCH, true), OnCreate::RebuildEmpty);
        assert_eq!(on_create_hr(ADAPTER_NOT_FOUND, true), OnCreate::RebuildEmpty);
        assert_eq!(on_create_hr(E_INVALIDARG, true), OnCreate::RebuildEmpty);
        assert_eq!(on_create_hr(E_INVALIDARG, false), OnCreate::Unavailable);
    }

    #[test]
    #[ignore = "needs a D3D12 device; run: cargo test -p d3d12_xs --lib hw_pso_lib -- --ignored"]
    fn hw_pso_lib_store_serialize_reload_and_rebuild() {
        const IID_DEVICE: Guid = Guid { d1: 0x1898_19f1, d2: 0x1db6, d3: 0x4b57, d4: [0xbe, 0x54, 0x18, 0x21, 0x33, 0x9b, 0x85, 0xf7] };
        const IID_PSO: Guid = Guid { d1: 0x765a_30f3, d2: 0xf624, d3: 0x4c6f, d4: [0xa8, 0x28, 0xac, 0xe9, 0x48, 0x62, 0x24, 0x45] };
        unsafe {
            let p = crate::host_proc(b"D3D12CreateDevice ");
            let create: extern "system" fn(*mut c_void, u32, *const Guid, *mut *mut c_void) -> i32 = core::mem::transmute(p);
            let mut dev: *mut c_void = core::ptr::null_mut();
            assert!(create(core::ptr::null_mut(), 0xb000, &IID_DEVICE, &mut dev) >= 0 && !dev.is_null());
            let qi: unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> i32 = core::mem::transmute(vslot(dev, 0));
            let mut dev1: *mut c_void = core::ptr::null_mut();
            assert!(qi(dev, &IID_DEVICE1, &mut dev1) >= 0);
            let code = tombox_xshader::dxc::compile("#define RS \"UAV(u0)\"
RWByteAddressBuffer o : register(u0);
[RootSignature(RS)]
[numthreads(1, 1, 1)]
void main() { o.Store(0, 5u); }
", "cs_6_0").expect("dxc");
            let mut desc = [0u8; CS_DESC];
            desc[8..16].copy_from_slice(&(code.as_ptr() as u64).to_le_bytes());
            desc[16..24].copy_from_slice(&(code.len() as u64).to_le_bytes());
            let mk: unsafe extern "system" fn(*mut c_void, *const u8, *const Guid, *mut *mut c_void) -> i32 = core::mem::transmute(vslot(dev, 11));
            let mut pso: *mut c_void = core::ptr::null_mut();
            assert!(mk(dev, desc.as_ptr(), &IID_PSO, &mut pso) >= 0);
            let (hr, lib) = create_library(dev1, &[]);
            assert!(hr >= 0 && lib != 0, "empty library hr={hr:#010x}");
            let k = compute_key(&desc, 0, "cbraw=0").unwrap();
            let w = wide(k);
            let st: unsafe extern "system" fn(*mut c_void, *const u16, *mut c_void) -> i32 = core::mem::transmute(vslot(lib as *mut c_void, LIB_STORE));
            assert!(st(lib as *mut c_void, w.as_ptr(), pso) >= 0);
            assert!(st(lib as *mut c_void, w.as_ptr(), pso) < 0, "a second store under the same name must be refused");
            let size: unsafe extern "system" fn(*mut c_void) -> usize = core::mem::transmute(vslot(lib as *mut c_void, LIB_SIZE));
            let n = size(lib as *mut c_void);
            let mut blob = vec![0u8; n];
            let ser: unsafe extern "system" fn(*mut c_void, *mut u8, usize) -> i32 = core::mem::transmute(vslot(lib as *mut c_void, LIB_SERIALIZE));
            assert!(ser(lib as *mut c_void, blob.as_mut_ptr(), n) >= 0);
            let file = file_bytes("test ident", &blob);
            let FileCheck::Blob(b) = check_file(Some(&file), "test ident") else { panic!("round trip") };
            let (hr2, lib2) = create_library(dev1, b);
            assert!(hr2 >= 0 && lib2 != 0, "reload hr={hr2:#010x}");
            let ld: unsafe extern "system" fn(*mut c_void, *const u16, *const u8, *const Guid, *mut *mut c_void) -> i32 = core::mem::transmute(vslot(lib2 as *mut c_void, LIB_LOAD_CS));
            let mut got: *mut c_void = core::ptr::null_mut();
            assert!(ld(lib2 as *mut c_void, w.as_ptr(), desc.as_ptr(), &IID_PSO, &mut got) >= 0 && !got.is_null(), "load from the reloaded library");
            let other = wide(k ^ 1);
            let mut none: *mut c_void = core::ptr::null_mut();
            assert!(ld(lib2 as *mut c_void, other.as_ptr(), desc.as_ptr(), &IID_PSO, &mut none) < 0, "unknown names miss");
            let mut junk = b.to_vec();
            for x in junk.iter_mut().take(64) {
                *x ^= 0x5a;
            }
            let (hr3, _) = create_library(dev1, &junk);
            assert_eq!(on_create_hr(hr3, true), OnCreate::RebuildEmpty, "corrupt blob hr={hr3:#010x}");
            for o in [got, pso, lib2 as *mut c_void, lib as *mut c_void, dev1, dev] {
                release(o);
            }
        }
    }

    #[test]
    fn saves_wait_for_a_quiet_moment_but_never_longer_than_the_force_interval() {
        assert!(!save_due(100, 90, 0));
        assert!(save_due(130, 100, 120));
        assert!(!save_due(130, 100, 129));
        assert!(!save_due(219, 100, 219));
        assert!(save_due(220, 100, 220));
    }

    #[test]
    fn each_cbv_mode_keeps_its_own_cache_files() {
        assert_eq!(cache_file("pso_library", "cbraw=1"), "pso_library.cbraw_1.bin");
        assert_eq!(cache_file("pso_prewarm", "cbraw=copy"), "pso_prewarm.cbraw_copy.bin");
        assert_ne!(cache_file("pso_library", "cbraw=0"), cache_file("pso_library", "cbraw=1"));
    }

    #[test]
    fn totals_are_logged_every_ten_seconds_even_without_new_entries() {
        assert!(summary_due(100, 0));
        assert!(!summary_due(105, 100));
        assert!(summary_due(110, 100));
        assert!(summary_due(500, 100));
        assert!(summary().contains("hits") && summary().contains("stored"));
    }

    #[test]
    fn names_are_32_hex_digits_and_nul_terminated() {
        let w = wide(0xabc);
        assert_eq!(w[32], 0);
        assert_eq!(String::from_utf16_lossy(&w[..32]), format!("{:032x}", 0xabcu128));
    }
}

#[cfg(test)]
mod removed_gate {
    #[test]
    fn no_library_blob_is_written_after_device_removal() {
        crate::dred::mark_removed();
        let e = super::save(false).unwrap_err();
        crate::dred::clear_removed_for_test();
        assert!(e.contains("device removed"), "{e}");
    }
}

#[cfg(test)]
mod final_root_key {
    #[test]
    fn the_key_follows_the_root_signature_the_driver_receives() {
        let title = b"title-root-signature".to_vec();
        let (a, b, c) = (0x7a11_0000usize, 0x7a12_0000usize, 0x7a13_0000usize);
        super::note_real_root(a, b"final-root-with-aliases");
        super::note_root(a, 0, &title, false);
        super::note_real_root(b, b"final-root-without-aliases");
        super::note_root(b, 0, &title, false);
        let (ha, hb) = (super::root_key_hash(a).unwrap(), super::root_key_hash(b).unwrap());
        assert_ne!(ha, hb, "one title blob patched two ways must give two keys");
        super::note_real_root(c, b"final-root-with-aliases");
        super::note_root(c, 0, &title, false);
        assert_eq!(super::root_key_hash(c), Some(ha), "the same final blob keys the same");
        super::note_root(0x7a14_0000, 0, &title, false);
        assert_ne!(super::root_key_hash(0x7a14_0000), Some(ha));
        super::note_real_root(a, b"a-new-object-at-a-reused-address");
        assert_ne!(super::root_key_hash(a), Some(ha), "a stale title entry never outlives a new root signature at the same address");
        assert!(super::root_info(a).is_none());
    }
}
