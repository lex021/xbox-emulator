use compat_common::record;
use core::ffi::c_void;
use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering::Relaxed};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use crate::device_wrap::Guid;

pub const GFX: u8 = 0;
pub const CS: u8 = 1;
const GFX_DESC: usize = 0x290;
const CS_DESC: usize = 0x38;
const PAYLOAD_VERSION: u32 = 2;
pub const MAX_RECORDS: usize = 4096;
pub const MAX_BYTES: usize = 192 << 20;
pub const IDLE_RELEASE_SECS: u64 = 180;
pub const PROMOTE_AHEAD: usize = 4;
const WAIT_MS: u64 = 2000;
const PREWARM_PRIORITY: i32 = -1;
const THREAD_SET_QUERY: u32 = 0x0020 | 0x0040;
const GFX_SHADERS: [usize; 5] = [0x08, 0x18, 0x28, 0x38, 0x48];
const GFX_PTRS: [usize; 11] = [0x00, 0x08, 0x18, 0x28, 0x38, 0x48, 0x58, 0x68, 0x228, 0x278, 0x280];
const CS_PTRS: [usize; 4] = [0x00, 0x08, 0x20, 0x28];

const IID_ROOT_SIGNATURE: Guid = Guid { d1: 0xc54a_6b66, d2: 0x72df, d3: 0x4ee8, d4: [0x8b, 0xe5, 0xa9, 0x46, 0xa1, 0x42, 0x92, 0x14] };

static HIT: AtomicU64 = AtomicU64::new(0);
static LATE: AtomicU64 = AtomicU64::new(0);
static UNUSED: AtomicU64 = AtomicU64::new(0);
static BUILT_LIB: AtomicU64 = AtomicU64::new(0);
static BUILT_DRIVER: AtomicU64 = AtomicU64::new(0);
static BUILD_FAILED: AtomicU64 = AtomicU64::new(0);
static INVALID: AtomicU64 = AtomicU64::new(0);
static REFUSED: AtomicU64 = AtomicU64::new(0);
static SAVING: AtomicBool = AtomicBool::new(false);

extern "system" {
    fn GetCurrentThread() -> *mut c_void;
    fn SetThreadPriority(h: *mut c_void, p: i32) -> i32;
    fn GetThreadPriority(h: *mut c_void) -> i32;
    fn GetCurrentThreadId() -> u32;
    fn OpenThread(access: u32, inherit: i32, id: u32) -> *mut c_void;
    fn CloseHandle(h: *mut c_void) -> i32;
}

pub fn enabled() -> bool {
    static E: OnceLock<bool> = OnceLock::new();
    *E.get_or_init(|| {
        let on = std::env::var("TOMBOX_PSO_PREWARM_LIB").map(|v| v.trim() != "0").unwrap_or(true);
        if !on {
            record("d3d12_xs", "pso_prewarm", "TOMBOX_PSO_PREWARM_LIB=0: PSOs are not recorded or prewarmed from the pipeline library");
        }
        on && crate::pso_lib::enabled()
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Root {
    pub mask: u32,
    pub blob: u64,
    pub via_real: bool,
    pub real: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rec {
    pub kind: u8,
    pub key: u128,
    pub seq: u32,
    pub desc: Vec<u8>,
    pub shaders: Vec<(u16, u64)>,
    pub so: Vec<(Vec<u8>, Vec<u8>)>,
    pub strides: Vec<u32>,
    pub il: Vec<(Vec<u8>, Vec<u8>)>,
    pub root: Option<Root>,
}

pub type Blobs = HashMap<u64, Arc<[u8]>>;

pub fn blob_id(b: &[u8]) -> u64 {
    crate::pso_lib::fnv64(b) ^ (b.len() as u64).rotate_left(29)
}

fn rd64(d: &[u8], o: usize) -> u64 {
    u64::from_le_bytes(d[o..o + 8].try_into().unwrap())
}

fn rd32(d: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(d[o..o + 4].try_into().unwrap())
}

fn wr64(d: &mut [u8], o: usize, v: u64) {
    d[o..o + 8].copy_from_slice(&v.to_le_bytes());
}

unsafe fn raw<'a>(p: u64, n: usize) -> &'a [u8] {
    if p == 0 || n == 0 { &[] } else { unsafe { core::slice::from_raw_parts(p as *const u8, n) } }
}

unsafe fn cstr<'a>(p: u64) -> Option<&'a [u8]> {
    if p == 0 {
        return None;
    }
    let s = p as *const u8;
    for n in 0..256 {
        if unsafe { *s.add(n) } == 0 {
            return Some(unsafe { core::slice::from_raw_parts(s, n) });
        }
    }
    None
}

pub unsafe fn capture(kind: u8, desc: &[u8], key: u128, root: Option<(u32, &[u8], bool, u64)>) -> Option<(Rec, Vec<(u64, Arc<[u8]>)>)> {
    let mut blobs: Vec<(u64, Arc<[u8]>)> = Vec::new();
    let mut keep = |b: &[u8]| -> u64 {
        let id = blob_id(b);
        if !blobs.iter().any(|(i, _)| *i == id) {
            blobs.push((id, Arc::from(b)));
        }
        id
    };
    let root = match (rd64(desc, 0), root) {
        (0, _) => None,
        (_, None) => return None,
        (_, Some((mask, b, via_real, real))) => Some(Root { mask, blob: keep(b), via_real, real }),
    };
    let mut r = Rec { kind, key, seq: 0, desc: Vec::new(), shaders: Vec::new(), so: Vec::new(), strides: Vec::new(), il: Vec::new(), root };
    if kind == GFX {
        if desc.len() < GFX_DESC {
            return None;
        }
        let mut d = desc[..GFX_DESC].to_vec();
        for o in GFX_SHADERS {
            let (p, n) = (rd64(desc, o), rd64(desc, o + 8) as usize);
            if n > 64 << 20 {
                return None;
            }
            if p != 0 && n != 0 {
                r.shaders.push((o as u16, keep(unsafe { raw(p, n) })));
            }
        }
        let (so, so_n) = (rd64(desc, 0x58), rd32(desc, 0x60) as usize);
        if so_n > 64 || (so == 0 && so_n != 0) {
            return None;
        }
        for i in 0..so_n {
            let e = unsafe { raw(so + (i * 0x18) as u64, 0x18) };
            r.so.push((e.to_vec(), unsafe { cstr(rd64(e, 8)) }?.to_vec()));
        }
        let (st, st_n) = (rd64(desc, 0x68), rd32(desc, 0x70) as usize);
        if st_n > 8 || (st == 0 && st_n != 0) {
            return None;
        }
        r.strides = unsafe { raw(st, st_n * 4) }.chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect();
        let (il, il_n) = (rd64(desc, 0x228), rd32(desc, 0x230) as usize);
        if il_n > 64 || (il == 0 && il_n != 0) {
            return None;
        }
        for i in 0..il_n {
            let e = unsafe { raw(il + (i * 0x20) as u64, 0x20) };
            r.il.push((e.to_vec(), unsafe { cstr(rd64(e, 0)) }?.to_vec()));
        }
        for o in GFX_PTRS {
            d[o..o + 8].fill(0);
        }
        r.desc = d;
    } else {
        if desc.len() < CS_DESC {
            return None;
        }
        let mut d = desc[..CS_DESC].to_vec();
        let (p, n) = (rd64(desc, 0x08), rd64(desc, 0x10) as usize);
        if n > 64 << 20 {
            return None;
        }
        if p != 0 && n != 0 {
            r.shaders.push((0x08, keep(unsafe { raw(p, n) })));
        }
        for o in CS_PTRS {
            d[o..o + 8].fill(0);
        }
        r.desc = d;
    }
    Some((r, blobs))
}

pub struct Built {
    pub desc: Vec<u8>,
    _keep: Vec<Arc<[u8]>>,
    _names: Vec<Vec<u8>>,
    _so: Vec<u8>,
    _strides: Vec<u32>,
    _il: Vec<u8>,
}

unsafe impl Send for Built {}

pub fn build(r: &Rec, blobs: &Blobs) -> Option<Built> {
    let want = if r.kind == GFX { GFX_DESC } else { CS_DESC };
    if r.desc.len() != want {
        return None;
    }
    let mut desc = r.desc.clone();
    let mut keep = Vec::new();
    for &(o, id) in &r.shaders {
        let o = o as usize;
        let b = blobs.get(&id)?.clone();
        if o + 16 > desc.len() || rd64(&desc, o + 8) as usize != b.len() {
            return None;
        }
        wr64(&mut desc, o, b.as_ptr() as u64);
        keep.push(b);
    }
    let mut names = Vec::new();
    let mut so = Vec::new();
    let mut il = Vec::new();
    let strides = r.strides.clone();
    if r.kind == GFX {
        for (e, n) in &r.so {
            let mut z = n.clone();
            z.push(0);
            let mut e = e.clone();
            wr64(&mut e, 8, z.as_ptr() as u64);
            names.push(z);
            so.extend_from_slice(&e);
        }
        for (e, n) in &r.il {
            let mut z = n.clone();
            z.push(0);
            let mut e = e.clone();
            wr64(&mut e, 0, z.as_ptr() as u64);
            names.push(z);
            il.extend_from_slice(&e);
        }
        if rd32(&desc, 0x60) as usize != r.so.len() || rd32(&desc, 0x70) as usize != strides.len() || rd32(&desc, 0x230) as usize != r.il.len() {
            return None;
        }
        if !so.is_empty() {
            wr64(&mut desc, 0x58, so.as_ptr() as u64);
        }
        if !strides.is_empty() {
            wr64(&mut desc, 0x68, strides.as_ptr() as u64);
        }
        if !il.is_empty() {
            wr64(&mut desc, 0x228, il.as_ptr() as u64);
        }
    }
    Some(Built { desc, _keep: keep, _names: names, _so: so, _strides: strides, _il: il })
}

pub fn root_hash(r: &Root, blobs: &Blobs) -> Option<u64> {
    blobs.get(&r.blob).map(|b| crate::pso_lib::combine_root(crate::pso_lib::fnv64(b) ^ r.mask as u64, r.real))
}

pub fn key_of(r: &Rec, b: &Built, blobs: &Blobs, mode: &str) -> Option<u128> {
    let root = match &r.root {
        None => 0,
        Some(rt) => root_hash(rt, blobs)?,
    };
    if r.kind == GFX {
        unsafe { crate::pso_lib::gfx_key(&b.desc, root, mode) }
    } else {
        unsafe { crate::pso_lib::compute_key(&b.desc, root, mode) }
    }
}

struct W(Vec<u8>);

impl W {
    fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    fn u16(&mut self, v: u16) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn u128(&mut self, v: u128) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn bytes(&mut self, b: &[u8]) {
        self.u32(b.len() as u32);
        self.0.extend_from_slice(b);
    }
}

struct Rd<'a> {
    b: &'a [u8],
    o: usize,
}

impl<'a> Rd<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let s = self.b.get(self.o..self.o.checked_add(n)?)?;
        self.o += n;
        Some(s)
    }
    fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }
    fn u16(&mut self) -> Option<u16> {
        Some(u16::from_le_bytes(self.take(2)?.try_into().ok()?))
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    fn u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }
    fn u128(&mut self) -> Option<u128> {
        Some(u128::from_le_bytes(self.take(16)?.try_into().ok()?))
    }
    fn bytes(&mut self) -> Option<&'a [u8]> {
        let n = self.u32()? as usize;
        self.take(n)
    }
    fn count(&mut self, max: usize) -> Option<usize> {
        let n = self.u32()? as usize;
        if n > max { None } else { Some(n) }
    }
}

pub struct Journal {
    pub recs: Vec<Rec>,
    pub keys: HashSet<u128>,
    pub blobs: Blobs,
    pub bytes: usize,
    pub max_n: usize,
    pub max_bytes: usize,
    pub dirty: bool,
    next_seq: u32,
}

impl Journal {
    pub fn new(max_n: usize, max_bytes: usize) -> Self {
        Journal { recs: Vec::new(), keys: HashSet::new(), blobs: HashMap::new(), bytes: 0, max_n, max_bytes, dirty: false, next_seq: 0 }
    }

    pub fn has(&self, k: u128) -> bool {
        self.keys.contains(&k)
    }

    pub fn add(&mut self, mut r: Rec, blobs: Vec<(u64, Arc<[u8]>)>) -> bool {
        if self.keys.contains(&r.key) {
            return false;
        }
        let fresh: usize = blobs.iter().filter(|(i, _)| !self.blobs.contains_key(i)).map(|(_, b)| b.len()).sum();
        let own = r.desc.len() + r.so.iter().chain(r.il.iter()).map(|(e, n)| e.len() + n.len()).sum::<usize>();
        if self.recs.len() >= self.max_n || self.bytes + fresh + own > self.max_bytes {
            return false;
        }
        for (i, b) in blobs {
            self.blobs.entry(i).or_insert(b);
        }
        self.bytes += fresh + own;
        r.seq = self.next_seq;
        self.next_seq += 1;
        self.keys.insert(r.key);
        self.recs.push(r);
        self.dirty = true;
        true
    }

    pub fn retain(&mut self, keep: &dyn Fn(&Rec, &Blobs) -> bool) -> usize {
        let before = self.recs.len();
        let blobs = &self.blobs;
        let recs: Vec<Rec> = core::mem::take(&mut self.recs).into_iter().filter(|r| keep(r, blobs)).collect();
        let dropped = before - recs.len();
        let used: HashSet<u64> = recs.iter().flat_map(|r| r.shaders.iter().map(|s| s.1).chain(r.root.iter().map(|rt| rt.blob))).collect();
        self.blobs.retain(|i, _| used.contains(i));
        self.keys = recs.iter().map(|r| r.key).collect();
        self.bytes = self.blobs.values().map(|b| b.len()).sum::<usize>() + recs.iter().map(|r| r.desc.len() + r.so.iter().chain(r.il.iter()).map(|(e, n)| e.len() + n.len()).sum::<usize>()).sum::<usize>();
        self.recs = recs;
        if dropped != 0 {
            self.dirty = true;
        }
        dropped
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut w = W(Vec::with_capacity(self.bytes + 1024));
        w.u32(PAYLOAD_VERSION);
        let mut ids: Vec<&u64> = self.blobs.keys().collect();
        ids.sort();
        w.u32(ids.len() as u32);
        for i in ids {
            w.u64(*i);
            w.bytes(&self.blobs[i]);
        }
        w.u32(self.recs.len() as u32);
        for r in &self.recs {
            w.u8(r.kind);
            w.u128(r.key);
            w.u32(r.seq);
            w.bytes(&r.desc);
            w.u32(r.shaders.len() as u32);
            for &(o, i) in &r.shaders {
                w.u16(o);
                w.u64(i);
            }
            for list in [&r.so, &r.il] {
                w.u32(list.len() as u32);
                for (e, n) in list {
                    w.bytes(e);
                    w.bytes(n);
                }
            }
            w.u32(r.strides.len() as u32);
            for &s in &r.strides {
                w.u32(s);
            }
            match &r.root {
                None => w.u8(0),
                Some(rt) => {
                    w.u8(1 + rt.via_real as u8);
                    w.u32(rt.mask);
                    w.u64(rt.blob);
                    w.u64(rt.real);
                }
            }
        }
        w.0
    }

    pub fn decode(b: &[u8], max_n: usize, max_bytes: usize) -> Option<Journal> {
        let mut rd = Rd { b, o: 0 };
        if rd.u32()? != PAYLOAD_VERSION {
            return None;
        }
        let mut j = Journal::new(max_n, max_bytes);
        let nb = rd.count(1 << 20)?;
        let mut blobs: Blobs = HashMap::with_capacity(nb);
        for _ in 0..nb {
            let id = rd.u64()?;
            let data = rd.bytes()?;
            if blob_id(data) != id {
                return None;
            }
            blobs.insert(id, Arc::from(data));
        }
        let nr = rd.count(1 << 20)?;
        let mut recs = Vec::with_capacity(nr.min(max_n));
        for _ in 0..nr {
            let kind = rd.u8()?;
            let key = rd.u128()?;
            let seq = rd.u32()?;
            let desc = rd.bytes()?.to_vec();
            let mut shaders = Vec::new();
            for _ in 0..rd.count(8)? {
                shaders.push((rd.u16()?, rd.u64()?));
            }
            let mut lists: [Vec<(Vec<u8>, Vec<u8>)>; 2] = [Vec::new(), Vec::new()];
            for l in lists.iter_mut() {
                for _ in 0..rd.count(64)? {
                    let e = rd.bytes()?.to_vec();
                    let n = rd.bytes()?.to_vec();
                    l.push((e, n));
                }
            }
            let mut strides = Vec::new();
            for _ in 0..rd.count(8)? {
                strides.push(rd.u32()?);
            }
            let root = match rd.u8()? {
                0 => None,
                t @ (1 | 2) => Some(Root { via_real: t == 2, mask: rd.u32()?, blob: rd.u64()?, real: rd.u64()? }),
                _ => return None,
            };
            let [so, il] = lists;
            recs.push(Rec { kind, key, seq, desc, shaders, so, strides, il, root });
        }
        if rd.o != b.len() {
            return None;
        }
        recs.sort_by_key(|r| r.seq);
        for r in recs {
            let ids: Vec<u64> = r.shaders.iter().map(|s| s.1).chain(r.root.iter().map(|rt| rt.blob)).collect();
            let mut refs = Vec::with_capacity(ids.len());
            for i in ids {
                refs.push((i, blobs.get(&i)?.clone()));
            }
            j.add(r, refs);
        }
        j.dirty = false;
        Some(j)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    Queued,
    InFlight,
    Ready(usize),
    Taken,
    Failed,
    Released,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Claim {
    Absent,
    Inline,
    Wait,
    Ready(usize),
}

pub struct Queue {
    order: VecDeque<u128>,
    slot: HashMap<u128, Slot>,
    by_seq: Vec<u128>,
    pos: HashMap<u128, usize>,
}

impl Queue {
    pub fn new(keys: Vec<u128>) -> Self {
        let pos = keys.iter().enumerate().map(|(i, k)| (*k, i)).collect();
        let slot = keys.iter().map(|k| (*k, Slot::Queued)).collect();
        Queue { order: keys.iter().copied().collect(), slot, by_seq: keys, pos }
    }

    pub fn state(&self, k: u128) -> Option<Slot> {
        self.slot.get(&k).copied()
    }

    pub fn next(&mut self) -> Option<u128> {
        while let Some(k) = self.order.pop_front() {
            if let Some(s) = self.slot.get_mut(&k) {
                if *s == Slot::Queued {
                    *s = Slot::InFlight;
                    return Some(k);
                }
            }
        }
        None
    }

    pub fn promote(&mut self, k: u128) {
        let Some(&p) = self.pos.get(&k) else { return };
        let ahead: Vec<u128> = self.by_seq[p + 1..].iter().copied().filter(|x| self.slot.get(x) == Some(&Slot::Queued)).take(PROMOTE_AHEAD).collect();
        if ahead.is_empty() {
            return;
        }
        self.order.retain(|x| !ahead.contains(x));
        for x in ahead.into_iter().rev() {
            self.order.push_front(x);
        }
    }

    pub fn claim(&mut self, k: u128) -> Claim {
        let Some(s) = self.slot.get_mut(&k) else { return Claim::Absent };
        let c = match *s {
            Slot::Queued => {
                *s = Slot::Taken;
                Claim::Inline
            }
            Slot::InFlight => Claim::Wait,
            Slot::Ready(p) => {
                *s = Slot::Taken;
                Claim::Ready(p)
            }
            Slot::Taken | Slot::Failed | Slot::Released => return Claim::Absent,
        };
        if c != Claim::Wait {
            self.promote(k);
        }
        c
    }

    pub fn finish(&mut self, k: u128, pso: Option<usize>) -> Option<usize> {
        match self.slot.get_mut(&k) {
            Some(s) if *s == Slot::InFlight => {
                *s = match pso {
                    Some(p) => Slot::Ready(p),
                    None => Slot::Failed,
                };
                None
            }
            _ => pso,
        }
    }

    pub fn drained(&self) -> bool {
        !self.slot.values().any(|s| matches!(s, Slot::Queued | Slot::InFlight))
    }

    pub fn expire(&mut self) -> Vec<usize> {
        let mut out = Vec::new();
        for s in self.slot.values_mut() {
            match *s {
                Slot::Ready(p) => {
                    out.push(p);
                    *s = Slot::Released;
                }
                Slot::Queued => *s = Slot::Released,
                _ => {}
            }
        }
        self.order.clear();
        out
    }
}

struct Pw {
    q: Mutex<Queue>,
    cv: Condvar,
    recs: HashMap<u128, Rec>,
    blobs: Blobs,
    busy: Mutex<HashMap<u128, usize>>,
    rs: Mutex<HashMap<(u64, u32, bool), usize>>,
    real: usize,
    wrapper: usize,
    last_use: AtomicU64,
    expired: AtomicBool,
    workers_left: AtomicUsize,
}

unsafe impl Send for Pw {}
unsafe impl Sync for Pw {}

static PW: OnceLock<Pw> = OnceLock::new();
static JOURNAL: OnceLock<Mutex<Journal>> = OnceLock::new();
static FILE: OnceLock<(std::path::PathBuf, String)> = OnceLock::new();

fn journal() -> &'static Mutex<Journal> {
    JOURNAL.get_or_init(|| Mutex::new(Journal::new(MAX_RECORDS, MAX_BYTES)))
}

fn now_s() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

unsafe fn vslot(obj: *mut c_void, i: usize) -> usize {
    unsafe { *(*(obj as *const *const usize)).add(i) }
}

unsafe fn release(obj: usize) {
    if obj != 0 {
        let f: unsafe extern "system" fn(*mut c_void) -> u32 = unsafe { core::mem::transmute(vslot(obj as *mut c_void, 2)) };
        unsafe { f(obj as *mut c_void) };
    }
}

pub fn ident(lib_ident: &str) -> String {
    format!("{lib_ident} | xlat {}", tombox_xshader::pack_key::TRANSLATOR_HASH)
}

pub fn summary() -> String {
    format!("pso prewarm: {} handed to the title (prewarm_hit), {} asked before ready (prewarm_late), {} released unused; built {} from the library + {} by the driver, {} failed, {} invalid records, {} not recorded (cap)",
        HIT.load(Relaxed), LATE.load(Relaxed), UNUSED.load(Relaxed), BUILT_LIB.load(Relaxed), BUILT_DRIVER.load(Relaxed), BUILD_FAILED.load(Relaxed), INVALID.load(Relaxed), REFUSED.load(Relaxed))
}

pub fn start(real: *mut c_void, wrapper: *mut c_void, lib_ident: &str, dir: &std::path::Path) {
    if !enabled() || PW.get().is_some() {
        return;
    }
    let t0 = std::time::Instant::now();
    let id = ident(lib_ident);
    let path = dir.join(crate::pso_lib::cache_file("pso_prewarm", crate::pso_lib::mode_tag()));
    let _ = FILE.set((path.clone(), id.clone()));
    let file = std::fs::read(&path).ok();
    let mut why = None;
    let mut j = match crate::pso_lib::check_file(file.as_deref(), &id) {
        crate::pso_lib::FileCheck::Empty => Journal::new(MAX_RECORDS, MAX_BYTES),
        crate::pso_lib::FileCheck::Rebuild(w) => {
            why = Some(w);
            Journal::new(MAX_RECORDS, MAX_BYTES)
        }
        crate::pso_lib::FileCheck::Blob(b) => Journal::decode(b, MAX_RECORDS, MAX_BYTES).unwrap_or_else(|| {
            why = Some("unreadable record payload".into());
            Journal::new(MAX_RECORDS, MAX_BYTES)
        }),
    };
    drop(file);
    let mode = crate::pso_lib::mode_tag();
    let invalid = j.retain(&|r, b| build(r, b).and_then(|x| key_of(r, &x, b, mode)) == Some(r.key));
    INVALID.store(invalid as u64, Relaxed);
    if why.is_some() {
        j.dirty = true;
    }
    let mut recs: Vec<Rec> = j.recs.clone();
    recs.sort_by_key(|r| r.seq);
    let blobs = j.blobs.clone();
    let keys: Vec<u128> = recs.iter().map(|r| r.key).collect();
    let n = keys.len();
    {
        let mut g = journal().lock().unwrap_or_else(|e| e.into_inner());
        let mut fresh = core::mem::replace(&mut *g, j);
        for r in core::mem::take(&mut fresh.recs) {
            let ids: Vec<(u64, Arc<[u8]>)> = r.shaders.iter().map(|s| s.1).chain(r.root.iter().map(|rt| rt.blob)).filter_map(|i| fresh.blobs.get(&i).map(|b| (i, b.clone()))).collect();
            g.add(r, ids);
        }
    }
    let pw = Pw {
        q: Mutex::new(Queue::new(keys)),
        cv: Condvar::new(),
        recs: recs.into_iter().map(|r| (r.key, r)).collect(),
        blobs,
        busy: Mutex::new(HashMap::new()),
        rs: Mutex::new(HashMap::new()),
        real: real as usize,
        wrapper: wrapper as usize,
        last_use: AtomicU64::new(now_s()),
        expired: AtomicBool::new(false),
        workers_left: AtomicUsize::new(0),
    };
    if PW.set(pw).is_err() {
        return;
    }
    let workers = crate::pso_xrec::workers_for(n, std::thread::available_parallelism().map(|c| c.get()).unwrap_or(4));
    record("d3d12_xs", "pso_prewarm", &format!(
        "{} with {n} PSO records ({invalid} dropped: key no longer matches) read in {:.0} ms{}; {workers} BELOW_NORMAL workers build them in first-use order; unclaimed ones are released {IDLE_RELEASE_SECS} s after the last claim; TOMBOX_PSO_PREWARM_LIB=0 disables",
        path.display(), t0.elapsed().as_secs_f64() * 1000.0, why.map(|w| format!(" (old records dropped: {w})")).unwrap_or_default()));
    let pw = PW.get().unwrap();
    pw.workers_left.store(workers, Relaxed);
    for _ in 0..workers {
        let _ = std::thread::Builder::new().name("tombox-pso-prewarm".into()).spawn(worker);
    }
}

fn worker() {
    let Some(pw) = PW.get() else { return };
    unsafe { SetThreadPriority(GetCurrentThread(), PREWARM_PRIORITY) };
    let me = unsafe { OpenThread(THREAD_SET_QUERY, 0, GetCurrentThreadId()) } as usize;
    let t0 = std::time::Instant::now();
    let mut done = 0usize;
    loop {
        if pw.expired.load(Relaxed) {
            break;
        }
        let Some(k) = pw.q.lock().unwrap_or_else(|e| e.into_inner()).next() else { break };
        if me != 0 {
            pw.busy.lock().unwrap_or_else(|e| e.into_inner()).insert(k, me);
        }
        let pso = pw.recs.get(&k).and_then(|r| unsafe { make(pw, r) });
        if me != 0 {
            pw.busy.lock().unwrap_or_else(|e| e.into_inner()).remove(&k);
            unsafe { SetThreadPriority(GetCurrentThread(), PREWARM_PRIORITY) };
        }
        if pso.is_none() {
            BUILD_FAILED.fetch_add(1, Relaxed);
        }
        let stray = {
            let mut q = pw.q.lock().unwrap_or_else(|e| e.into_inner());
            q.finish(k, pso)
        };
        pw.cv.notify_all();
        if let Some(p) = stray {
            unsafe { release(p) };
        }
        done += 1;
    }
    if me != 0 {
        unsafe { CloseHandle(me as *mut c_void) };
    }
    if pw.workers_left.fetch_sub(1, Relaxed) == 1 {
        pw.last_use.fetch_max(now_s(), Relaxed);
        record("d3d12_xs", "pso_prewarm", &format!("workers finished: last one built {done} PSOs in {:.1} s; {}", t0.elapsed().as_secs_f64(), summary()));
    }
}

unsafe fn root_sig(pw: &Pw, rt: &Root) -> Option<usize> {
    let tag = (rt.blob, rt.mask, rt.via_real);
    if let Some(&r) = pw.rs.lock().unwrap_or_else(|e| e.into_inner()).get(&tag) {
        return Some(r);
    }
    let b = pw.blobs.get(&rt.blob)?;
    let mut rs: *mut c_void = core::ptr::null_mut();
    let hr = if rt.via_real || pw.wrapper == 0 {
        let f: unsafe extern "system" fn(*mut c_void, u32, *const u8, usize, *const Guid, *mut *mut c_void) -> i32 = unsafe { core::mem::transmute(vslot(pw.real as *mut c_void, 16)) };
        let hr = unsafe { f(pw.real as *mut c_void, rt.mask, b.as_ptr(), b.len(), &IID_ROOT_SIGNATURE, &mut rs) };
        if hr >= 0 && !rs.is_null() {
            crate::pso_lib::note_root(rs as usize, rt.mask, b, rt.via_real);
        }
        hr
    } else {
        let f: unsafe extern "system" fn(*mut c_void, u32, *const u8, usize, *const Guid, *mut *mut c_void) -> i32 = unsafe { core::mem::transmute(vslot(pw.wrapper as *mut c_void, 16)) };
        unsafe { f(pw.wrapper as *mut c_void, rt.mask, b.as_ptr(), b.len(), &IID_ROOT_SIGNATURE, &mut rs) }
    };
    if hr < 0 || rs.is_null() {
        return None;
    }
    pw.rs.lock().unwrap_or_else(|e| e.into_inner()).insert(tag, rs as usize);
    Some(rs as usize)
}

static STALE_ROOT: AtomicU64 = AtomicU64::new(0);

unsafe fn make(pw: &Pw, r: &Rec) -> Option<usize> {
    let mut b = build(r, &pw.blobs)?;
    if let Some(rt) = &r.root {
        let rs = unsafe { root_sig(pw, rt) }?;
        if crate::pso_lib::root_key_hash(rs) != root_hash(rt, &pw.blobs) {
            let n = STALE_ROOT.fetch_add(1, Relaxed);
            if n < 4 {
                record("d3d12_xs", "pso_prewarm", &format!("record {:032x}: its root signature now reaches the driver as a different blob than when it was recorded; skipped so the library never stores it under the old key (#{})", r.key, n + 1));
            }
            return None;
        }
        wr64(&mut b.desc, 0, rs as u64);
    }
    let (pso, from_lib) = unsafe { crate::pso_lib::prebuild(pw.real as *mut c_void, r.kind, r.key, &b.desc) };
    if pso.is_some() {
        if from_lib { &BUILT_LIB } else { &BUILT_DRIVER }.fetch_add(1, Relaxed);
    }
    pso
}

fn boost(pw: &Pw, k: u128) {
    let Some(h) = pw.busy.lock().unwrap_or_else(|e| e.into_inner()).get(&k).copied() else { return };
    let want = unsafe { GetThreadPriority(GetCurrentThread()) }.clamp(0, 2);
    unsafe { SetThreadPriority(h as *mut c_void, want) };
}

unsafe fn hand(p: usize, riid: *const Guid, ppv: *mut *mut c_void) -> Option<i32> {
    let qi: unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> i32 = unsafe { core::mem::transmute(vslot(p as *mut c_void, 0)) };
    let mut out: *mut c_void = core::ptr::null_mut();
    let hr = unsafe { qi(p as *mut c_void, riid, &mut out) };
    unsafe { release(p) };
    if hr < 0 || out.is_null() {
        return None;
    }
    unsafe { *ppv = out };
    Some(hr)
}

pub unsafe fn claim(k: u128, riid: *const Guid, ppv: *mut *mut c_void) -> Option<i32> {
    let pw = PW.get()?;
    if ppv.is_null() || riid.is_null() || pw.expired.load(Relaxed) {
        return None;
    }
    let mut q = pw.q.lock().unwrap_or_else(|e| e.into_inner());
    match q.claim(k) {
        Claim::Absent => None,
        Claim::Inline => {
            drop(q);
            LATE.fetch_add(1, Relaxed);
            crate::frame_ev::note(crate::frame_ev::PREWARM_LATE, 0);
            pw.last_use.fetch_max(now_s(), Relaxed);
            None
        }
        Claim::Ready(p) => {
            drop(q);
            HIT.fetch_add(1, Relaxed);
            crate::frame_ev::note(crate::frame_ev::PREWARM_HIT, 0);
            pw.last_use.fetch_max(now_s(), Relaxed);
            unsafe { hand(p, riid, ppv) }
        }
        Claim::Wait => {
            LATE.fetch_add(1, Relaxed);
            let _ev = crate::frame_ev::begin(crate::frame_ev::PREWARM_LATE);
            boost(pw, k);
            let until = std::time::Instant::now() + std::time::Duration::from_millis(WAIT_MS);
            while q.state(k) == Some(Slot::InFlight) {
                let left = until.saturating_duration_since(std::time::Instant::now());
                if left.is_zero() {
                    break;
                }
                q = pw.cv.wait_timeout(q, left).unwrap_or_else(|e| e.into_inner()).0;
            }
            let c = q.claim(k);
            drop(q);
            pw.last_use.fetch_max(now_s(), Relaxed);
            match c {
                Claim::Ready(p) => unsafe { hand(p, riid, ppv) },
                _ => None,
            }
        }
    }
}

pub unsafe fn note(kind: u8, k: u128, desc: &[u8]) {
    if !enabled() || FILE.get().is_none() {
        return;
    }
    if journal().lock().unwrap_or_else(|e| e.into_inner()).has(k) {
        return;
    }
    let rs = if desc.len() >= 8 { rd64(desc, 0) } else { 0 };
    let info = if rs == 0 { None } else { crate::pso_lib::root_info(rs as usize) };
    if rs != 0 && info.is_none() {
        return;
    }
    let Some((r, blobs)) = (unsafe { capture(kind, desc, k, info.as_ref().map(|(m, b, v)| (*m, &b[..], *v, crate::pso_lib::root_real(rs as usize)))) }) else { return };
    if !journal().lock().unwrap_or_else(|e| e.into_inner()).add(r, blobs) {
        let n = REFUSED.fetch_add(1, Relaxed);
        if n == 0 {
            record("d3d12_xs", "pso_prewarm", &format!("record cap reached ({MAX_RECORDS} PSOs / {} MB); later PSOs are not prewarmed", MAX_BYTES >> 20));
        }
    }
}

pub fn on_present() {
    let Some(pw) = PW.get() else { return };
    if pw.expired.load(Relaxed) || pw.workers_left.load(Relaxed) != 0 {
        return;
    }
    if now_s().saturating_sub(pw.last_use.load(Relaxed)) < IDLE_RELEASE_SECS {
        return;
    }
    if pw.expired.swap(true, Relaxed) {
        return;
    }
    let left = pw.q.lock().unwrap_or_else(|e| e.into_inner()).expire();
    for &p in &left {
        unsafe { release(p) };
        crate::frame_ev::note(crate::frame_ev::PREWARM_UNUSED, 0);
    }
    UNUSED.fetch_add(left.len() as u64, Relaxed);
    let roots: Vec<usize> = pw.rs.lock().unwrap_or_else(|e| e.into_inner()).drain().map(|(_, r)| r).collect();
    for r in roots {
        unsafe { release(r) };
    }
    record("d3d12_xs", "pso_prewarm", &format!("released {} unclaimed prewarmed PSOs ({IDLE_RELEASE_SECS} s without a claim); {}", left.len(), summary()));
}

pub fn save(exiting: bool) -> Option<String> {
    let (path, id) = FILE.get()?;
    if SAVING.swap(true, Relaxed) {
        return None;
    }
    let payload = {
        let g = if exiting {
            match journal().try_lock() {
                Ok(g) => g,
                Err(std::sync::TryLockError::Poisoned(e)) => e.into_inner(),
                Err(std::sync::TryLockError::WouldBlock) => {
                    SAVING.store(false, Relaxed);
                    return None;
                }
            }
        } else {
            journal().lock().unwrap_or_else(|e| e.into_inner())
        };
        if !g.dirty {
            SAVING.store(false, Relaxed);
            return None;
        }
        (g.encode(), g.recs.len())
    };
    let bytes = crate::pso_lib::file_bytes(id, &payload.0);
    let tmp = path.with_extension(format!("bin.{}", std::process::id()));
    let r = std::fs::write(&tmp, &bytes).and_then(|_| std::fs::rename(&tmp, path));
    let msg = match r {
        Ok(()) => {
            journal().lock().unwrap_or_else(|e| e.into_inner()).dirty = false;
            format!("saved {} PSO records ({} bytes) to {}", payload.1, bytes.len(), path.display())
        }
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            format!("save to {} failed: {e}", path.display())
        }
    };
    SAVING.store(false, Relaxed);
    record("d3d12_xs", "pso_prewarm", &msg);
    Some(msg)
}

pub fn dirty() -> bool {
    FILE.get().is_some() && journal().try_lock().map(|g| g.dirty).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Src {
        vs: Vec<u8>,
        ps: Vec<u8>,
        names: Vec<Vec<u8>>,
        il: Vec<u8>,
        so: Vec<u8>,
        strides: Vec<u32>,
        desc: Vec<u8>,
    }

    fn src(vs: &[u8]) -> Src {
        let mut s = Src { vs: vs.to_vec(), ps: vec![9, 8, 7, 6, 5], names: vec![b"POSITION\0".to_vec(), b"TEXCOORD\0".to_vec(), b"SV_Position\0".to_vec()], il: vec![0; 0x40], so: vec![0; 0x18], strides: vec![16, 32], desc: vec![0; GFX_DESC] };
        for i in 0..2 {
            wr64(&mut s.il, i * 0x20, s.names[i].as_ptr() as u64);
            s.il[i * 0x20 + 0xc] = 6 + i as u8;
            s.il[i * 0x20 + 0x10] = i as u8;
        }
        wr64(&mut s.so, 8, s.names[2].as_ptr() as u64);
        s.so[0x14] = 4;
        let d = &mut s.desc;
        wr64(d, 0x00, 0xdead_0000);
        wr64(d, 0x08, s.vs.as_ptr() as u64);
        wr64(d, 0x10, s.vs.len() as u64);
        wr64(d, 0x18, s.ps.as_ptr() as u64);
        wr64(d, 0x20, s.ps.len() as u64);
        wr64(d, 0x58, s.so.as_ptr() as u64);
        d[0x60] = 1;
        wr64(d, 0x68, s.strides.as_ptr() as u64);
        d[0x70] = 2;
        wr64(d, 0x228, s.il.as_ptr() as u64);
        d[0x230] = 2;
        d[0x240] = 1;
        d[0x244] = 28;
        wr64(d, 0x278, 0x1234);
        wr64(d, 0x280, 77);
        s
    }

    fn root_blob() -> Vec<u8> {
        b"DXBC-root-signature-bytes".to_vec()
    }

    fn rec_of(s: &Src, mode: &str) -> (Rec, Vec<(u64, Arc<[u8]>)>, u128) {
        let rb = root_blob();
        let root = crate::pso_lib::fnv64(&rb) ^ 3;
        let k = unsafe { crate::pso_lib::gfx_key(&s.desc, root, mode) }.unwrap();
        let (r, b) = unsafe { capture(GFX, &s.desc, k, Some((3, &rb, false, 0))) }.unwrap();
        (r, b, k)
    }

    #[test]
    fn prewarm_rebuilds_the_exact_key_the_title_create_looks_up() {
        let s = src(&[1, 2, 3, 4]);
        let (r, b, k) = rec_of(&s, "cbraw=1");
        let mut j = Journal::new(MAX_RECORDS, MAX_BYTES);
        assert!(j.add(r, b));
        let bytes = crate::pso_lib::file_bytes("id", &j.encode());
        let crate::pso_lib::FileCheck::Blob(payload) = crate::pso_lib::check_file(Some(&bytes), "id") else { panic!("header") };
        let back = Journal::decode(payload, MAX_RECORDS, MAX_BYTES).unwrap();
        drop(s);
        let _noise: Vec<Vec<u8>> = (0..64).map(|i| vec![i as u8; 64]).collect();
        let r2 = &back.recs[0];
        assert_eq!(r2, &j.recs[0]);
        let built = build(r2, &back.blobs).unwrap();
        assert_eq!(key_of(r2, &built, &back.blobs, "cbraw=1"), Some(k));
        assert_ne!(key_of(r2, &built, &back.blobs, "cbraw=0"), Some(k));
        assert_eq!(rd64(&built.desc, 0x278), 0);
        assert_eq!(rd64(&built.desc, 0x280), 0);
        let title = src(&[1, 2, 3, 4]);
        let root = crate::pso_lib::fnv64(&root_blob()) ^ 3;
        assert_eq!(unsafe { crate::pso_lib::gfx_key(&title.desc, root, "cbraw=1") }, Some(k));
        assert_eq!(crate::pso_lib::stream_key(k, "e"), crate::pso_lib::stream_key(key_of(r2, &built, &back.blobs, "cbraw=1").unwrap(), "e"));
        let other = src(&[1, 2, 3, 5]);
        assert_ne!(unsafe { crate::pso_lib::gfx_key(&other.desc, root, "cbraw=1") }, Some(k));
        let mut cs = vec![0u8; CS_DESC];
        let code = vec![5u8; 40];
        wr64(&mut cs, 0x08, code.as_ptr() as u64);
        wr64(&mut cs, 0x10, 40);
        wr64(&mut cs, 0x20, 0x99);
        let ck = unsafe { crate::pso_lib::compute_key(&cs, 0, "cbraw=1") }.unwrap();
        let (cr, cb) = unsafe { capture(CS, &cs, ck, None) }.unwrap();
        let mut blobs: Blobs = HashMap::new();
        blobs.extend(cb);
        let cbuilt = build(&cr, &blobs).unwrap();
        assert_eq!(key_of(&cr, &cbuilt, &blobs, "cbraw=1"), Some(ck));
        let mut tamper = blobs.clone();
        let id = cr.shaders[0].1;
        tamper.insert(id, Arc::from(vec![6u8; 40]));
        let tb = build(&cr, &tamper).unwrap();
        assert_ne!(key_of(&cr, &tb, &tamper, "cbraw=1"), Some(ck));
    }

    #[test]
    fn a_root_signature_without_recorded_bytes_is_not_recorded() {
        let s = src(&[1, 2]);
        assert!(unsafe { capture(GFX, &s.desc, 1, None) }.is_none());
        let mut d = s.desc.clone();
        wr64(&mut d, 0, 0);
        let (r, _) = unsafe { capture(GFX, &d, 1, None) }.unwrap();
        assert!(r.root.is_none());
    }

    #[test]
    fn records_dedupe_shared_shaders_and_respect_the_caps() {
        let (r1, b1, _) = rec_of(&src(&[1, 2, 3]), "m");
        let (r2, b2, _) = rec_of(&src(&[1, 2, 4]), "m");
        let (r3, b3, _) = rec_of(&src(&[1, 2, 5]), "m");
        let mut j = Journal::new(2, MAX_BYTES);
        assert!(j.add(r1.clone(), b1.clone()));
        assert!(!j.add(r1.clone(), b1.clone()));
        assert!(j.add(r2, b2));
        assert_eq!(j.blobs.len(), 4);
        assert!(!j.add(r3.clone(), b3.clone()));
        assert_eq!(j.recs.iter().map(|r| r.seq).collect::<Vec<_>>(), vec![0, 1]);
        let one = j.bytes;
        let mut small = Journal::new(MAX_RECORDS, one - 1);
        let (r4, b4, _) = rec_of(&src(&[1, 2, 3]), "m");
        assert!(small.add(r4, b4));
        assert!(!small.add(r3, b3));
        assert_eq!(small.recs.len(), 1);
    }

    #[test]
    fn invalid_records_are_dropped_with_their_blobs() {
        let (r1, b1, _) = rec_of(&src(&[1, 2, 3]), "m");
        let (r2, b2, _) = rec_of(&src(&[7, 7, 7, 7, 7, 7]), "m");
        let gone = r2.key;
        let mut j = Journal::new(MAX_RECORDS, MAX_BYTES);
        j.add(r1, b1);
        j.add(r2, b2);
        j.dirty = false;
        assert_eq!(j.retain(&|r, _| r.key != gone), 1);
        assert!(j.dirty && !j.has(gone) && j.blobs.len() == 3);
        assert!(Journal::decode(&j.encode()[..10], 9, 9).is_none());
    }

    #[test]
    fn workers_follow_first_use_order_and_a_claim_promotes_what_comes_next() {
        let mut q = Queue::new((1..=10).collect());
        assert_eq!(q.next(), Some(1));
        assert_eq!(q.next(), Some(2));
        assert_eq!(q.claim(6), Claim::Inline);
        assert_eq!((q.next(), q.next(), q.next(), q.next()), (Some(7), Some(8), Some(9), Some(10)));
        assert_eq!(q.next(), Some(3));
        assert_eq!(q.next(), Some(4));
        assert_eq!(q.next(), Some(5));
        assert_eq!(q.next(), None);
        assert_eq!(q.state(6), Some(Slot::Taken));
    }

    #[test]
    fn each_prewarmed_pso_is_handed_out_once_and_taken_keys_are_never_built() {
        let mut q = Queue::new(vec![1, 2, 3]);
        assert_eq!(q.claim(2), Claim::Inline);
        assert_eq!(q.next(), Some(3));
        assert_eq!(q.next(), Some(1));
        assert_eq!(q.next(), None);
        assert_eq!(q.claim(1), Claim::Wait);
        assert_eq!(q.finish(1, Some(0x100)), None);
        assert_eq!(q.claim(1), Claim::Ready(0x100));
        assert_eq!(q.claim(1), Claim::Absent);
        assert_eq!(q.claim(2), Claim::Absent);
        assert_eq!(q.claim(99), Claim::Absent);
        assert_eq!(q.finish(3, None), None);
        assert_eq!(q.claim(3), Claim::Absent);
        assert!(q.drained());
        assert_eq!(q.finish(1, Some(0x200)), Some(0x200));
    }

    #[test]
    fn expiry_releases_only_unclaimed_ready_psos_and_stops_queued_work() {
        let mut q = Queue::new(vec![1, 2, 3, 4]);
        let (a, b) = (q.next().unwrap(), q.next().unwrap());
        q.finish(a, Some(0xa0));
        q.finish(b, Some(0xb0));
        assert_eq!(q.claim(a), Claim::Ready(0xa0));
        assert!(!q.drained());
        assert_eq!(q.expire(), vec![0xb0]);
        assert_eq!(q.next(), None);
        assert_eq!(q.claim(b), Claim::Absent);
        assert_eq!(q.claim(3), Claim::Absent);
        assert!(q.drained());
    }

    #[test]
    fn identity_includes_the_translator_hash() {
        let a = ident("NVIDIA X driver 1 | cbraw=1");
        assert!(a.starts_with("NVIDIA X driver 1 | cbraw=1 | xlat "));
        assert!(a.ends_with(tombox_xshader::pack_key::TRANSLATOR_HASH));
        let f = crate::pso_lib::file_bytes(&a, b"x");
        assert!(matches!(crate::pso_lib::check_file(Some(&f), &ident("NVIDIA X driver 2 | cbraw=1")), crate::pso_lib::FileCheck::Rebuild(_)));
    }

    #[test]
    #[ignore = "needs a D3D12 device; run: cargo test -p d3d12_xs --lib hw_pso_prewarm -- --ignored"]
    fn hw_pso_prewarm_rebuilt_desc_creates_and_loads_under_the_title_key() {
        const IID_DEVICE: Guid = Guid { d1: 0x1898_19f1, d2: 0x1db6, d3: 0x4b57, d4: [0xbe, 0x54, 0x18, 0x21, 0x33, 0x9b, 0x85, 0xf7] };
        const IID_DEVICE1: Guid = Guid { d1: 0x77ac_ce80, d2: 0x638e, d3: 0x4e65, d4: [0x88, 0x95, 0xc1, 0xf2, 0x33, 0x86, 0x86, 0x3e] };
        const IID_LIBRARY1: Guid = Guid { d1: 0x80ea_bf42, d2: 0x2568, d3: 0x4e5e, d4: [0xbd, 0x82, 0xc3, 0x7f, 0x86, 0x96, 0x1d, 0xc3] };
        const IID_PSO: Guid = Guid { d1: 0x765a_30f3, d2: 0xf624, d3: 0x4c6f, d4: [0xa8, 0x28, 0xac, 0xe9, 0x48, 0x62, 0x24, 0x45] };
        unsafe {
            let p = crate::host_proc(b"D3D12CreateDevice ");
            let create: extern "system" fn(*mut c_void, u32, *const Guid, *mut *mut c_void) -> i32 = core::mem::transmute(p);
            let mut dev: *mut c_void = core::ptr::null_mut();
            assert!(create(core::ptr::null_mut(), 0xb000, &IID_DEVICE, &mut dev) >= 0 && !dev.is_null());
            let code = tombox_xshader::dxc::compile("RWByteAddressBuffer o : register(u0);
[numthreads(1, 1, 1)]
void main() { o.Store(0, 5u); }
", "cs_6_0").expect("dxc");
            let rs_src = tombox_xshader::dxc::compile("#define RS \"UAV(u0)\"
RWByteAddressBuffer o : register(u0);
[RootSignature(RS)]
[numthreads(1, 1, 1)]
void main() { o.Store(0, 5u); }
", "cs_6_0").expect("dxc rs");
            let mk_rs: unsafe extern "system" fn(*mut c_void, u32, *const u8, usize, *const Guid, *mut *mut c_void) -> i32 = core::mem::transmute(vslot(dev, 16));
            let (mut rs_a, mut rs_b): (*mut c_void, *mut c_void) = (core::ptr::null_mut(), core::ptr::null_mut());
            assert!(mk_rs(dev, 0, rs_src.as_ptr(), rs_src.len(), &IID_ROOT_SIGNATURE, &mut rs_a) >= 0);
            let rs_copy = rs_src.clone();
            assert!(mk_rs(dev, 0, rs_copy.as_ptr(), rs_copy.len(), &IID_ROOT_SIGNATURE, &mut rs_b) >= 0);
            eprintln!("identical root signature blobs give the same object: {}", rs_a == rs_b);
            let mut desc = [0u8; CS_DESC];
            wr64(&mut desc, 0, rs_a as u64);
            wr64(&mut desc, 8, code.as_ptr() as u64);
            wr64(&mut desc, 16, code.len() as u64);
            let root = crate::pso_lib::fnv64(&rs_src);
            let k = crate::pso_lib::compute_key(&desc, root, "cbraw=0").unwrap();
            let (r, b) = capture(CS, &desc, k, Some((0, &rs_src, true, 0))).unwrap();
            let mut j = Journal::new(MAX_RECORDS, MAX_BYTES);
            assert!(j.add(r, b));
            let back = Journal::decode(&j.encode(), MAX_RECORDS, MAX_BYTES).unwrap();
            let mut built = build(&back.recs[0], &back.blobs).unwrap();
            assert_eq!(key_of(&back.recs[0], &built, &back.blobs, "cbraw=0"), Some(k));
            wr64(&mut built.desc, 0, rs_b as u64);
            let mk: unsafe extern "system" fn(*mut c_void, *const u8, *const Guid, *mut *mut c_void) -> i32 = core::mem::transmute(vslot(dev, 11));
            let mut pso: *mut c_void = core::ptr::null_mut();
            assert!(mk(dev, built.desc.as_ptr(), &IID_PSO, &mut pso) >= 0 && !pso.is_null(), "rebuilt desc creates");
            let qi: unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> i32 = core::mem::transmute(vslot(dev, 0));
            let mut dev1: *mut c_void = core::ptr::null_mut();
            assert!(qi(dev, &IID_DEVICE1, &mut dev1) >= 0);
            let mk_lib: unsafe extern "system" fn(*mut c_void, *const u8, usize, *const Guid, *mut *mut c_void) -> i32 = core::mem::transmute(vslot(dev1, 44));
            let mut lib: *mut c_void = core::ptr::null_mut();
            assert!(mk_lib(dev1, core::ptr::null(), 0, &IID_LIBRARY1, &mut lib) >= 0);
            let mut w = [0u16; 33];
            for (i, c) in format!("{k:032x}").bytes().enumerate() {
                w[i] = c as u16;
            }
            let st: unsafe extern "system" fn(*mut c_void, *const u16, *mut c_void) -> i32 = core::mem::transmute(vslot(lib, 8));
            assert!(st(lib, w.as_ptr(), pso) >= 0);
            let ld: unsafe extern "system" fn(*mut c_void, *const u16, *const u8, *const Guid, *mut *mut c_void) -> i32 = core::mem::transmute(vslot(lib, 10));
            let mut title: *mut c_void = core::ptr::null_mut();
            assert!(ld(lib, w.as_ptr(), desc.as_ptr(), &IID_PSO, &mut title) >= 0 && !title.is_null(), "the title's own desc (its own root signature object) loads the prewarm-stored entry");
            let mut q = Queue::new(vec![k]);
            assert_eq!(q.next(), Some(k));
            q.finish(k, Some(pso as usize));
            let Claim::Ready(got) = q.claim(k) else { panic!("ready") };
            let mut out: *mut c_void = core::ptr::null_mut();
            assert_eq!(hand(got, &IID_PSO, &mut out), Some(0));
            assert_eq!(out, pso, "the title receives the prewarmed object itself");
            for o in [title, out, lib, dev1, rs_b, rs_a, dev] {
                release(o as usize);
            }
        }
    }
}
