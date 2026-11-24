use compat_common::record;
use core::ffi::c_void;
use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock};

#[repr(C)]
struct Guid(u32, u16, u16, [u8; 8]);
const IID_DEVICE: Guid = Guid(0x189819f1, 0x1db6, 0x4b57, [0xbe, 0x54, 0x18, 0x21, 0x33, 0x9b, 0x85, 0xf7]);
const IID_ALLOCATOR: Guid = Guid(0x6102dee4, 0xaf59, 0x4b09, [0xb9, 0x99, 0xb4, 0x4d, 0x73, 0xf0, 0x9b, 0x24]);
const IID_GFX_LIST: Guid = Guid(0x5b160d0f, 0xac1b, 0x4185, [0x8b, 0xa8, 0xb3, 0xae, 0x42, 0xa5, 0xa4, 0x55]);
const IID_FENCE: Guid = Guid(0x0a753dcf, 0xc4d8, 0x4b91, [0xad, 0xf6, 0xbe, 0x5a, 0x60, 0xd9, 0x5a, 0x76]);
const IID_QUERY_HEAP: Guid = Guid(0x0d9658ae, 0xed45, 0x469e, [0xa6, 0x1d, 0x97, 0x0e, 0xc5, 0x83, 0xca, 0xb4]);
const IID_RESOURCE: Guid = Guid(0x696442be, 0xa72e, 0x4059, [0xbc, 0x79, 0x5b, 0x5c, 0x98, 0x04, 0x0f, 0xad]);

pub const ST_COMMON: u32 = 0;
pub const ST_RENDER_TARGET: u32 = 0x4;
pub const ST_DEPTH_WRITE: u32 = 0x10;
pub const ST_DEPTH_READ: u32 = 0x20;
pub const ST_COPY_DEST: u32 = 0x400;
pub const ST_COPY_SOURCE: u32 = 0x800;
pub const ST_RESOLVE_DEST: u32 = 0x1000;
pub const ST_RESOLVE_SOURCE: u32 = 0x2000;

const PC_CLOSE: usize = 9;
const PC_RESET: usize = 10;
const PC_COPY_TEXTURE_REGION: usize = 16;
const PC_RESOLVE: usize = 19;
const PC_BARRIER: usize = 26;

unsafe fn vt(o: *mut c_void) -> *const usize {
    *(o as *const *const usize)
}
macro_rules! call {
    ($obj:expr, $slot:expr, fn($($at:ty),*) -> $rt:ty $(, $arg:expr)*) => {{
        let o = $obj as *mut c_void;
        let f: unsafe extern "system" fn(*mut c_void $(, $at)*) -> $rt = core::mem::transmute(*vt(o).add($slot));
        f(o $(, $arg)*)
    }};
}

#[derive(Debug, Clone, PartialEq)]
pub struct Sel {
    pub secs: Vec<f64>,
    pub presents: Vec<u64>,
    pub file: bool,
    pub frames: u32,
    pub budget_mb: u64,
    pub max_caps: usize,
    pub chain_every: u64,
    pub all: bool,
    pub raw: bool,
    pub unknown: Vec<String>,
    pub ps_ms: Option<(u64, f64)>,
    pub ps_draw: Option<(u64, f64)>,
}

pub fn parse_ps_draw(v: &str) -> Option<(u64, f64)> {
    let (h, m) = match v.split_once(':') {
        Some((h, m)) => (h, Some(m)),
        None => (v, None),
    };
    let h = h.trim();
    let h = u64::from_str_radix(h.strip_prefix("0x").unwrap_or(h), 16).ok().filter(|x| *x != 0)?;
    let t = match m {
        Some(m) => m.trim().parse::<f64>().ok().filter(|f| f.is_finite() && *f >= 0.0)?,
        None => 0.0,
    };
    Some((h, t))
}

pub fn ps_draw_due(seen: bool, t: f64, target: Option<(u64, f64)>, done: u32, last: f64) -> bool {
    seen && done < SLOW_MAX_SHOTS && target.is_some_and(|(_, tmin)| t >= tmin) && (done == 0 || t - last >= PS_DRAW_GAP_S)
}

pub const PS_DRAW_GAP_S: f64 = 2.0;
static PS_SHOTS: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
static PS_LAST: Mutex<f64> = Mutex::new(0.0);

pub fn ps_draw_target() -> Option<u64> {
    sel().and_then(|s| s.ps_draw).map(|x| x.0)
}

pub fn parse_ps_ms(v: &str) -> Option<(u64, f64)> {
    let (h, m) = v.split_once(':')?;
    let h = h.trim();
    let h = u64::from_str_radix(h.strip_prefix("0x").unwrap_or(h), 16).ok().filter(|x| *x != 0)?;
    let m = m.trim().parse::<f64>().ok().filter(|f| f.is_finite() && *f > 0.0)?;
    Some((h, m))
}

pub fn slow_call(ps: u64, ms: f64, target: Option<(u64, f64)>) -> bool {
    target.is_some_and(|(t, thr)| ps == t && ms > thr)
}

pub fn slow_due(hit: bool, seen: bool, done: u32, max: u32) -> bool {
    hit && seen && done < max
}

pub const SLOW_MAX_SHOTS: u32 = 4;
static SLOW_HIT: AtomicBool = AtomicBool::new(false);
static SLOW_SHOTS: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
static SLOW_LAST: Mutex<Option<(f64, u32)>> = Mutex::new(None);

pub fn ps_ms_target() -> Option<u64> {
    sel().and_then(|s| s.ps_ms).map(|x| x.0)
}

pub fn note_slow_call(ps: u64, ms: f64, frame: u32) -> bool {
    let target = sel().and_then(|s| s.ps_ms);
    if !slow_call(ps, ms, target) || SLOW_SHOTS.load(Ordering::Relaxed) >= SLOW_MAX_SHOTS {
        return false;
    }
    *SLOW_LAST.lock().unwrap_or_else(|e| e.into_inner()) = Some((ms, frame));
    if !SLOW_HIT.swap(true, Ordering::Relaxed) {
        record("d3d12_xs", "passcap", &format!("ps_ms trigger: PS {ps:016x} took {ms:.2} ms in one call (gpuprof window frame {frame}) > {:.2} ms; the next frame that draws this PS arms a shot", target.map_or(0.0, |t| t.1)));
    }
    true
}

pub fn parse(v: &str) -> Option<Sel> {
    let v = v.trim();
    if v.is_empty() || v == "0" {
        return None;
    }
    let mut s = Sel { secs: Vec::new(), presents: Vec::new(), file: false, frames: 1, budget_mb: 1200, max_caps: 400, chain_every: 600, all: false, raw: false, unknown: Vec::new(), ps_ms: None, ps_draw: None };
    for tok in v.split([',', ';', ' ']).map(str::trim).filter(|t| !t.is_empty()) {
        let (k, val) = match tok.find([':', '=']) {
            Some(i) => (tok[..i].to_ascii_lowercase(), Some(tok[i + 1..].trim())),
            None => (tok.to_ascii_lowercase(), None),
        };
        let num = |x: Option<&str>| x.and_then(|x| x.parse::<f64>().ok()).filter(|f| f.is_finite() && *f >= 0.0);
        let int = |x: Option<&str>| x.and_then(|x| x.parse::<u64>().ok());
        let ok = match (k.as_str(), val) {
            ("t" | "s" | "sec", v) => num(v).map(|f| s.secs.push(f)).is_some(),
            ("f" | "frame" | "present", v) => int(v).map(|n| s.presents.push(n)).is_some(),
            ("n" | "frames", v) => int(v).map(|n| s.frames = n.clamp(1, 8) as u32).is_some(),
            ("mb", v) => int(v).map(|n| s.budget_mb = n.max(16)).is_some(),
            ("caps", v) => int(v).map(|n| s.max_caps = (n as usize).clamp(1, 4096)).is_some(),
            ("chain", v) => int(v).map(|n| s.chain_every = n).is_some(),
            ("file" | "1" | "on", None) => {
                s.file = true;
                true
            }
            ("all", None) => {
                s.all = true;
                true
            }
            ("ps_ms" | "psms", Some(v)) => parse_ps_ms(v).map(|x| s.ps_ms = Some(x)).is_some(),
            ("ps", Some(v)) => parse_ps_draw(v).map(|x| s.ps_draw = Some(x)).is_some(),
            ("raw", None) => {
                s.raw = true;
                true
            }
            (_, None) => num(Some(k.as_str())).map(|f| s.secs.push(f)).is_some(),
            _ => false,
        };
        if !ok {
            s.unknown.push(tok.to_string());
        }
    }
    s.secs.sort_by(|a, b| a.partial_cmp(b).unwrap());
    s.presents.sort();
    Some(s)
}

pub fn take_due<T: PartialOrd + Copy>(list: &[T], idx: &mut usize, now: T) -> bool {
    let mut hit = false;
    while *idx < list.len() && list[*idx] <= now {
        *idx += 1;
        hit = true;
    }
    hit
}

pub fn sel() -> Option<&'static Sel> {
    static S: OnceLock<Option<Sel>> = OnceLock::new();
    S.get_or_init(|| std::env::var("TOMBOX_PASSCAP").ok().and_then(|v| parse(&v))).as_ref()
}

pub fn on() -> bool {
    sel().is_some()
}

pub fn tables_on() -> bool {
    on() && !probe_hashes().is_empty()
}

pub fn typed_format(fmt: u32) -> u32 {
    match fmt {
        1 => 2,
        5 => 6,
        9 => 10,
        15 => 16,
        19 => 20,
        23 => 24,
        27 => 28,
        33 => 34,
        39 => 41,
        44 => 45,
        48 => 49,
        53 => 56,
        60 => 61,
        90 => 87,
        92 => 88,
        f => f,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Stats {
    pub ch: usize,
    pub min: [f32; 4],
    pub max: [f32; 4],
    pub mean: [f64; 4],
    pub nan: u64,
    pub inf: u64,
    pub zero: [f64; 4],
    pub zero_px: f64,
    pub pixels: u64,
}

impl Stats {
    pub fn black(&self) -> bool {
        self.pixels > 0 && self.zero_px >= 1.0
    }
    pub fn uniform(&self) -> bool {
        self.pixels > 0 && self.nan == 0 && (0..self.ch).all(|c| self.min[c] == self.max[c])
    }
    pub fn flags(&self) -> String {
        let mut f = Vec::new();
        if self.black() {
            f.push("BLACK");
        }
        if self.nan > 0 {
            f.push("NAN");
        }
        if self.inf > 0 {
            f.push("INF");
        }
        if self.uniform() && !self.black() {
            f.push("UNIFORM");
        }
        f.join(",")
    }
    pub fn line(&self) -> String {
        let names = ["r", "g", "b", "a"];
        let mut s = String::new();
        for c in 0..self.ch {
            let _ = write!(s, "{}[{:.5}..{:.5} mean {:.5} zero {:.3}] ", names[c], self.min[c], self.max[c], self.mean[c], self.zero[c]);
        }
        let _ = write!(s, "nan={} inf={} zero_px={:.4} px={}", self.nan, self.inf, self.zero_px, self.pixels);
        s
    }
}

pub fn stats(kind: &str, ch: usize, data: &[u8], pitch: usize, w: usize, h: usize, bpp: usize) -> Stats {
    let ch = ch.clamp(1, 4);
    let mut s = Stats { ch, min: [f32::MAX; 4], max: [f32::MIN; 4], ..Default::default() };
    let mut sum = [0f64; 4];
    let mut fin = [0u64; 4];
    let mut zero = [0u64; 4];
    let mut zero_px = 0u64;
    let rgb = ch.min(3);
    for y in 0..h {
        let Some(row) = data.get(y * pitch..y * pitch + w * bpp) else { break };
        for x in 0..w {
            let v = crate::rtcap::decode(kind, &row[x * bpp..x * bpp + bpp]);
            let mut all0 = true;
            for c in 0..ch {
                let f = v[c];
                if f.is_nan() {
                    s.nan += 1;
                    all0 = false;
                    continue;
                }
                if f.is_infinite() {
                    s.inf += 1;
                    all0 = false;
                    continue;
                }
                s.min[c] = s.min[c].min(f);
                s.max[c] = s.max[c].max(f);
                sum[c] += f as f64;
                fin[c] += 1;
                if f == 0.0 {
                    zero[c] += 1;
                } else if c < rgb {
                    all0 = false;
                }
            }
            if all0 {
                zero_px += 1;
            }
            s.pixels += 1;
        }
    }
    let n = s.pixels.max(1) as f64;
    for c in 0..ch {
        if fin[c] == 0 {
            s.min[c] = f32::NAN;
            s.max[c] = f32::NAN;
        }
        s.mean[c] = sum[c] / fin[c].max(1) as f64;
        s.zero[c] = zero[c] as f64 / n;
    }
    s.zero_px = zero_px as f64 / n;
    s
}

pub fn preview(kind: &str, ch: usize, is_float: bool, data: &[u8], pitch: usize, w: usize, h: usize, bpp: usize, st: &Stats) -> (usize, usize, Vec<u8>) {
    let step = w.div_ceil(960).max(1);
    let (pw, ph) = (w.div_ceil(step), h.div_ceil(step));
    let mut rgb = vec![0u8; pw * ph * 3];
    let hi = st.max[0];
    let lo = if st.min[0] > 0.0 { st.min[0] } else { hi * 1e-6 };
    for py in 0..ph {
        let y = (py * step).min(h - 1);
        let Some(row) = data.get(y * pitch..y * pitch + w * bpp) else { break };
        for px in 0..pw {
            let x = (px * step).min(w - 1);
            let v = crate::rtcap::decode(kind, &row[x * bpp..x * bpp + bpp]);
            let c3 = if ch == 1 {
                let t = if v[0] > 0.0 && hi > lo && lo > 0.0 { 0.15 + 0.85 * (v[0] / lo).ln() / (hi / lo).ln() } else if v[0] > 0.0 { 1.0 } else { 0.0 };
                [t, t, t]
            } else if ch == 2 {
                [v[0], v[1], 0.0]
            } else {
                [v[0], v[1], v[2]]
            };
            for c in 0..3 {
                let f = c3[c];
                let t = if f.is_nan() {
                    if c == 0 { 1.0 } else { 0.0 }
                } else if is_float && ch > 1 {
                    let m = f.max(0.0);
                    (m / (1.0 + m)).powf(1.0 / 2.2)
                } else {
                    f.clamp(0.0, 1.0)
                };
                rgb[(py * pw + px) * 3 + c] = (t.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
            }
        }
    }
    (pw, ph, rgb)
}

#[derive(Clone, Debug)]
pub struct Cap {
    pub idx: u32,
    pub slot: String,
    pub res: usize,
    pub label: String,
    pub w: usize,
    pub h: usize,
    pub fmt: u32,
    pub read_fmt: u32,
    pub samples: u32,
    pub state: u32,
    pub state_src: &'static str,
    pub rb: usize,
    pub tmp: usize,
    pub pitch: usize,
    pub total: u64,
    pub depth: usize,
}

unsafe impl Send for Cap {}

pub unsafe fn desc_of(res: usize) -> [u8; 0x38] {
    let mut d = [0u8; 0x38];
    call!(res, 10, fn(*mut u8) -> *mut u8, d.as_mut_ptr());
    d
}

fn d32(d: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(d[o..o + 4].try_into().unwrap())
}

pub fn label_of(res: usize, d: &[u8; 0x38]) -> String {
    let w = u64::from_le_bytes(d[0x10..0x18].try_into().unwrap());
    let s = d32(d, 0x24);
    format!("{res:#x} {w}x{} fmt={}{} mips={} arr={} flags={:#x}", d32(d, 0x18), d32(d, 0x20), if s > 1 { format!(" msaa{s}") } else { String::new() },
        u16::from_le_bytes([d[0x1e], d[0x1f]]), u16::from_le_bytes([d[0x1c], d[0x1d]]), d32(d, 0x30))
}

fn barrier_sub0(res: usize, before: u32, after: u32) -> [u64; 4] {
    barrier_sub(res, 0, before, after)
}

fn barrier_sub(res: usize, sub: u32, before: u32, after: u32) -> [u64; 4] {
    [0, res as u64, ((before as u64) << 32) | sub as u64, after as u64]
}

pub fn grid3d_on() -> bool {
    static M: OnceLock<bool> = OnceLock::new();
    *M.get_or_init(|| std::env::var("TOMBOX_PASSCAP_3D").map(|v| v.trim() == "1").unwrap_or(false))
}

pub fn cb_bytes() -> usize {
    static M: OnceLock<usize> = OnceLock::new();
    *M.get_or_init(|| std::env::var("TOMBOX_PASSCAP_CB_BYTES").ok().and_then(|v| v.trim().parse::<usize>().ok()).map(|v| v.clamp(256, 65536) & !63).unwrap_or(1024))
}

pub fn grid3d_text(data: &[u8], pitch: usize, w: usize, h: usize, depth: usize, fmt: u32) -> String {
    if fmt != 17 && fmt != 15 {
        return format!("3D {w}x{h}x{depth} format {fmt}: only R32G32_UINT light grids are decoded");
    }
    let (mut a, mut b, mut off) = (Vec::with_capacity(w * h * depth), Vec::with_capacity(w * h * depth), 0u32);
    for z in 0..depth {
        for y in 0..h {
            let row = (z * h + y) * pitch;
            for x in 0..w {
                let o = row + x * 8;
                let Some(t) = data.get(o..o + 8) else { continue };
                let x0 = u32::from_le_bytes([t[0], t[1], t[2], t[3]]);
                let y0 = u32::from_le_bytes([t[4], t[5], t[6], t[7]]);
                off = off.max(x0);
                a.push(y0 & 0xffff);
                b.push(y0 >> 16);
            }
        }
    }
    let n = a.len().max(1);
    let st = |v: &mut Vec<u32>| -> String {
        let mean = v.iter().map(|&x| x as f64).sum::<f64>() / n as f64;
        let nz = v.iter().filter(|&&x| x != 0).count();
        let big = v.iter().filter(|&&x| x > 256).count();
        v.sort_unstable();
        let p = |q: f64| v.get(((v.len().max(1) - 1) as f64 * q) as usize).copied().unwrap_or(0);
        format!("max {} p99 {} p50 {} mean {mean:.2} nonzero {:.3} over256 {big}", v.last().copied().unwrap_or(0), p(0.99), p(0.5), nz as f64 / n as f64)
    };
    format!("light grid {w}x{h}x{depth} ({} texels): count lo16 {} | count hi16 {} | offset max {off}", a.len(), st(&mut a), st(&mut b))
}

pub fn mips_on() -> bool {
    static M: OnceLock<bool> = OnceLock::new();
    *M.get_or_init(|| std::env::var("TOMBOX_PASSCAP_MIPS").map(|v| v.trim() == "1").unwrap_or(false))
}

unsafe fn committed(dev: usize, heap: u32, desc: &[u8; 0x38], state: u32) -> Result<usize, i32> {
    let hp: [u32; 5] = [heap, 0, 0, 1, 1];
    let mut out: *mut c_void = core::ptr::null_mut();
    let hr = call!(dev, 27, fn(*const u32, u32, *const u8, u32, *const c_void, *const Guid, *mut *mut c_void) -> i32,
        hp.as_ptr(), 0, desc.as_ptr(), state, core::ptr::null(), &IID_RESOURCE, &mut out);
    if hr < 0 || out.is_null() { Err(hr) } else { Ok(out as usize) }
}

fn buffer_desc(size: u64) -> [u8; 0x38] {
    let mut bd = [0u8; 0x38];
    bd[0..4].copy_from_slice(&1u32.to_le_bytes());
    bd[0x10..0x18].copy_from_slice(&size.to_le_bytes());
    bd[0x18..0x1c].copy_from_slice(&1u32.to_le_bytes());
    bd[0x1c..0x1e].copy_from_slice(&1u16.to_le_bytes());
    bd[0x1e..0x20].copy_from_slice(&1u16.to_le_bytes());
    bd[0x24..0x28].copy_from_slice(&1u32.to_le_bytes());
    bd[0x2c..0x30].copy_from_slice(&1u32.to_le_bytes());
    bd
}

unsafe fn list_call_barrier(list: usize, b: &[[u64; 4]]) {
    let f: unsafe extern "system" fn(*mut c_void, u32, *const [u64; 4]) = core::mem::transmute(crate::cmdlist_wrap::pc_method(list as *mut c_void, PC_BARRIER));
    f(list as *mut c_void, b.len() as u32, b.as_ptr());
}

pub unsafe fn list_device(list: usize) -> Option<usize> {
    let mut dev: *mut c_void = core::ptr::null_mut();
    if call!(list, 7, fn(*const Guid, *mut *mut c_void) -> i32, &IID_DEVICE, &mut dev) < 0 || dev.is_null() {
        return None;
    }
    call!(dev, 2, fn() -> u32);
    Some(dev as usize)
}

pub unsafe fn inject(dev: usize, list: usize, res: usize, state: u32, slot: &str, budget_left: u64) -> Result<Cap, String> {
    inject_mip(dev, list, res, state, slot, budget_left, 0)
}

pub unsafe fn inject_mip(dev: usize, list: usize, res: usize, state: u32, slot: &str, budget_left: u64, mip: u32) -> Result<Cap, String> {
    let d = desc_of(res);
    let label = label_of(res, &d);
    let (dim, w, h, fmt, samples, flags) = (d32(&d, 0), u64::from_le_bytes(d[0x10..0x18].try_into().unwrap()) as usize, d32(&d, 0x18) as usize, d32(&d, 0x20), d32(&d, 0x24), d32(&d, 0x30));
    let mips = u16::from_le_bytes([d[0x1e], d[0x1f]]) as u32;
    if mip > 0 && (samples > 1 || mip >= mips.max(1)) {
        return Err(format!("{label}: mip {mip} not capturable (samples {samples}, mips {mips})"));
    }
    if dim != 3 && !(dim == 4 && mip == 0 && samples <= 1 && grid3d_on()) {
        return Err(format!("{label}: dimension {dim} is not a 2D texture"));
    }
    if w == 0 || h == 0 {
        return Err(format!("{label}: empty"));
    }
    let typed = typed_format(fmt);
    let depth = flags & 2 != 0;
    let mut cd = d;
    if samples > 1 {
        if depth {
            return Err(format!("{label}: multisampled depth cannot go through ResolveSubresource; not captured"));
        }
        cd[0x08..0x10].fill(0);
        cd[0x1c..0x1e].copy_from_slice(&1u16.to_le_bytes());
        cd[0x1e..0x20].copy_from_slice(&1u16.to_le_bytes());
        cd[0x20..0x24].copy_from_slice(&typed.to_le_bytes());
        cd[0x24..0x28].copy_from_slice(&1u32.to_le_bytes());
        cd[0x28..0x2c].copy_from_slice(&0u32.to_le_bytes());
        cd[0x2c..0x30].copy_from_slice(&0u32.to_le_bytes());
        cd[0x30..0x34].copy_from_slice(&0u32.to_le_bytes());
    }
    let mut fp = [0u8; 32];
    let (mut rows, mut row_size, mut total) = (0u32, 0u64, 0u64);
    call!(dev, 38, fn(*const u8, u32, u32, u64, *mut [u8; 32], *mut u32, *mut u64, *mut u64) -> (),
        cd.as_ptr(), mip, 1, 0, &mut fp, &mut rows, &mut row_size, &mut total);
    if total == 0 || total == u64::MAX {
        return Err(format!("{label}: no copyable footprint"));
    }
    let (w, h) = if mip == 0 { (w, h) } else { (u32::from_le_bytes(fp[0x0c..0x10].try_into().unwrap()) as usize, u32::from_le_bytes(fp[0x10..0x14].try_into().unwrap()) as usize) };
    fp[0..8].copy_from_slice(&0u64.to_le_bytes());
    let tmp_bytes = if samples > 1 { total } else { 0 };
    if total + tmp_bytes > budget_left {
        return Err(format!("{label}: budget exhausted ({} MB left)", budget_left >> 20));
    }
    let pitch = u32::from_le_bytes(fp[0x18..0x1c].try_into().unwrap()) as usize;
    let fdepth = (u32::from_le_bytes(fp[0x14..0x18].try_into().unwrap()) as usize).max(1);
    let rb = committed(dev, 3, &buffer_desc(total), ST_COPY_DEST).map_err(|hr| format!("{label}: readback buffer hr={hr:#010x}"))?;
    let (src, tmp) = if samples > 1 {
        let tmp = match committed(dev, 1, &cd, ST_RESOLVE_DEST) {
            Ok(t) => t,
            Err(hr) => {
                call!(rb, 2, fn() -> u32);
                return Err(format!("{label}: resolve texture hr={hr:#010x}"));
            }
        };
        if state != ST_RESOLVE_SOURCE {
            list_call_barrier(list, &[barrier_sub0(res, state, ST_RESOLVE_SOURCE)]);
        }
        let f: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, *mut c_void, u32, u32) = core::mem::transmute(crate::cmdlist_wrap::pc_method(list as *mut c_void, PC_RESOLVE));
        f(list as *mut c_void, tmp as *mut c_void, 0, res as *mut c_void, 0, typed);
        let mut back = vec![barrier_sub0(tmp, ST_RESOLVE_DEST, ST_COPY_SOURCE)];
        if state != ST_RESOLVE_SOURCE {
            back.push(barrier_sub0(res, ST_RESOLVE_SOURCE, state));
        }
        list_call_barrier(list, &back);
        (tmp, tmp)
    } else {
        if state != ST_COPY_SOURCE {
            list_call_barrier(list, &[barrier_sub(res, mip, state, ST_COPY_SOURCE)]);
        }
        (res, 0)
    };
    let mut dst = [0u8; 48];
    dst[0..8].copy_from_slice(&(rb as u64).to_le_bytes());
    dst[8..12].copy_from_slice(&1u32.to_le_bytes());
    dst[16..48].copy_from_slice(&fp);
    let mut sl = [0u8; 48];
    sl[0..8].copy_from_slice(&(src as u64).to_le_bytes());
    sl[16..20].copy_from_slice(&mip.to_le_bytes());
    let f: unsafe extern "system" fn(*mut c_void, *const u8, u32, u32, u32, *const u8, *const c_void) = core::mem::transmute(crate::cmdlist_wrap::pc_method(list as *mut c_void, PC_COPY_TEXTURE_REGION));
    f(list as *mut c_void, dst.as_ptr(), 0, 0, 0, sl.as_ptr(), core::ptr::null());
    if samples <= 1 && state != ST_COPY_SOURCE {
        list_call_barrier(list, &[barrier_sub(res, mip, ST_COPY_SOURCE, state)]);
    }
    let read_fmt = u32::from_le_bytes(fp[0x08..0x0c].try_into().unwrap());
    Ok(Cap { idx: 0, slot: slot.to_string(), res, label, w, h, fmt, read_fmt, samples, state, state_src: "caller", rb, tmp, pitch, total: total + tmp_bytes, depth: if dim == 4 { fdepth } else { 1 } })
}

pub fn decode_kind(fmt: u32, read_fmt: u32) -> Option<(usize, bool, usize, &'static str)> {
    crate::rtcap::layout(typed_format(fmt)).or_else(|| crate::rtcap::layout(typed_format(read_fmt)))
}

pub unsafe fn read_cap(c: &Cap) -> Option<Vec<u8>> {
    let mut p: *mut u8 = core::ptr::null_mut();
    if call!(c.rb, 8, fn(u32, *const c_void, *mut *mut u8) -> i32, 0, core::ptr::null(), &mut p) < 0 || p.is_null() {
        return None;
    }
    let v = core::slice::from_raw_parts(p, c.total.min((c.pitch * c.h * c.depth.max(1)) as u64) as usize).to_vec();
    call!(c.rb, 9, fn(u32, *const c_void) -> (), 0, core::ptr::null());
    Some(v)
}

pub unsafe fn release_cap(c: &Cap) {
    if c.rb != 0 {
        call!(c.rb, 2, fn() -> u32);
    }
    if c.tmp != 0 {
        call!(c.tmp, 2, fn() -> u32);
    }
}

#[derive(Default)]
struct LState {
    rts: Vec<usize>,
    dsv: usize,
    draws: u32,
    dispatches: u32,
    indirect: u32,
    clears: Vec<String>,
    pso: usize,
    ps: Vec<u64>,
    states: HashMap<usize, u32>,
    split: Vec<usize>,
    dirty: bool,
    pso_dirty: bool,
    seq: u32,
    cbv: HashMap<u32, u64>,
    groot: usize,
    rc: HashMap<u32, Vec<Option<u32>>>,
    sref: Option<u32>,
    stencil_notes: Vec<String>,
    stencil_seen: Vec<(usize, Option<u32>)>,
    probe_caps: Vec<usize>,
    probe_notes: Vec<String>,
    probe_q: usize,
    probe_oq: usize,
}

struct ProbeBuf {
    rb: usize,
    qh: usize,
    n: usize,
    kind: u8,
    label: String,
}

#[derive(Default)]
struct FrameCount {
    draws: u64,
    dispatches: u64,
    indirect: u64,
    resolves: u64,
    copies: u64,
    into_src: u64,
    unresolved: u64,
    binds: u64,
    per_rt: HashMap<usize, u64>,
    to_src: Vec<String>,
}

struct Pass {
    list: usize,
    key: usize,
    order: u64,
    seq: u32,
    end: String,
    draws: u32,
    dispatches: u32,
    indirect: u32,
    clears: Vec<String>,
    ps: Vec<u64>,
    targets: Vec<String>,
    caps: Vec<usize>,
    notes: Vec<String>,
    writes_src: bool,
}

struct Shot {
    id: u32,
    why: String,
    start_present: u64,
    end_present: u64,
    t: f64,
    frames_left: u32,
    passes: Vec<Pass>,
    caps: Vec<Cap>,
    submit: HashMap<usize, (u64, usize)>,
    bytes: u64,
    waits: u32,
    order: u64,
    src: usize,
    src_label: String,
    chain: String,
    budget_note: Option<String>,
    ps_cache: HashMap<usize, u64>,
    probes: Vec<ProbeBuf>,
    cpu_pending: Vec<(usize, u64, String)>,
    cpu_submit: Vec<String>,
}

#[derive(Default)]
struct G {
    lists: HashMap<usize, LState>,
    fc: FrameCount,
    shot: Option<Shot>,
    submit_seq: u64,
    last_src: usize,
    last_src_desc: Vec<u8>,
    last_into_src_zero: Option<bool>,
    desc_cache: HashMap<usize, [u8; 0x38]>,
    presents: u64,
    t0: Option<std::time::Instant>,
    sec_idx: usize,
    pres_idx: usize,
    next_id: u32,
    device: usize,
    own: Option<(usize, usize, usize, u64)>,
    slow_seen: bool,
    ps_seen: bool,
    slow_cache: HashMap<usize, u64>,
}

static STATE: Mutex<Option<G>> = Mutex::new(None);
static ACTIVE: AtomicBool = AtomicBool::new(false);
static PREV: [AtomicUsize; 128] = [const { AtomicUsize::new(0) }; 128];

fn lock() -> MutexGuard<'static, Option<G>> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

fn with<R>(f: impl FnOnce(&mut G) -> R) -> R {
    let mut g = lock();
    f(g.get_or_insert_with(G::default))
}

fn key(list: *mut c_void) -> usize {
    crate::list_split::origin_of(list)
}

fn prev(slot: usize) -> usize {
    PREV[slot].load(Ordering::Relaxed)
}

pub fn out_root() -> PathBuf {
    if let Some(d) = std::env::var_os("TOMBOX_PASSCAP_DIR").filter(|v| !v.is_empty()) {
        return PathBuf::from(d);
    }
    std::env::var_os("TOMBOX_LOG").map(PathBuf::from).and_then(|p| p.parent().map(|d| d.to_path_buf())).unwrap_or_else(std::env::temp_dir).join("passcap")
}

fn cached_label(g: &mut G, r: usize) -> String {
    match r {
        0 => "-".into(),
        usize::MAX => "?unresolved-view".into(),
        r => {
            if !ACTIVE.load(Ordering::Relaxed) && !g.desc_cache.contains_key(&r) {
                return format!("{r:#x}");
            }
            let d = *g.desc_cache.entry(r).or_insert_with(|| unsafe { desc_of(r) });
            label_of(r, &d)
        }
    }
}

unsafe fn list_type(list: usize) -> u32 {
    call!(list, 8, fn() -> u32)
}

fn ps_hash(shot: &mut Shot, pso: usize) -> u64 {
    if pso == 0 {
        return 0;
    }
    *shot.ps_cache.entry(pso).or_insert_with(|| ps_of(pso))
}

fn ps_of(pso: usize) -> u64 {
    {
        let real = crate::pso_async::real_of(pso);
        let sum = crate::device_wrap::pso_summary(pso).or_else(|| if real != 0 { crate::device_wrap::pso_summary(real) } else { None });
        let missing = if sum.is_none() { 1 } else { 2 };
        sum.and_then(|(_, p, _)| if p == 0 { None } else { crate::shader_xlat::origin_hash(p) }).unwrap_or(missing)
    }
}

fn bound_state(st: &LState, res: usize, depth: bool) -> (u32, &'static str) {
    if let Some(&s) = st.states.get(&res) {
        return (s, "in-list barrier");
    }
    if depth {
        let g = crate::gpu_va::state(res);
        if g & ST_DEPTH_READ != 0 && g & ST_DEPTH_WRITE == 0 {
            return (ST_DEPTH_READ, "gpu_va global state");
        }
        return (ST_DEPTH_WRITE, "default for a bound DSV");
    }
    (ST_RENDER_TARGET, "default for a bound RT")
}

fn budget_left(shot: &Shot) -> u64 {
    let s = sel().map(|s| s.budget_mb).unwrap_or(1200) << 20;
    s.saturating_sub(shot.bytes)
}

unsafe fn capture_into(g: &mut G, list: usize, res: usize, state: u32, state_src: &'static str, slot: &str) -> Result<usize, String> {
    capture_into_mip(g, list, res, state, state_src, slot, 0)
}

unsafe fn capture_into_mip(g: &mut G, list: usize, res: usize, state: u32, state_src: &'static str, slot: &str, mip: u32) -> Result<usize, String> {
    if g.device == 0 {
        g.device = list_device(list).ok_or("GetDevice failed")?;
    }
    let dev = g.device;
    let shot = g.shot.as_mut().ok_or("no shot")?;
    let max = sel().map(|s| s.max_caps).unwrap_or(400);
    if shot.caps.len() >= max {
        let n = format!("capture count limit {max} reached");
        shot.budget_note.get_or_insert(n.clone());
        return Err(n);
    }
    let mut c = inject_mip(dev, list, res, state, slot, budget_left(shot), mip).inspect_err(|e| {
        if e.contains("budget") {
            shot.budget_note.get_or_insert(e.clone());
        }
    })?;
    c.idx = shot.caps.len() as u32;
    c.state_src = state_src;
    shot.bytes += c.total;
    shot.caps.push(c);
    Ok(shot.caps.len() - 1)
}

unsafe fn boundary(g: &mut G, list: *mut c_void, end: &str) {
    if !ACTIVE.load(Ordering::Relaxed) {
        return;
    }
    let k = key(list);
    let Some(st) = g.lists.get(&k) else { return };
    if st.rts.iter().all(|r| *r == 0) && st.dsv == 0 && st.draws == 0 && st.dispatches == 0 && st.indirect == 0 {
        return;
    }
    let all = sel().map(|s| s.all).unwrap_or(false);
    let want = st.dirty || all;
    let (rts, dsv) = (st.rts.clone(), st.dsv);
    let mut pass = Pass { list: list as usize, key: k, order: 0, seq: st.seq, end: end.to_string(), draws: st.draws, dispatches: st.dispatches, indirect: st.indirect,
        clears: st.clears.clone(), ps: st.ps.clone(), targets: Vec::new(), caps: st.probe_caps.clone(), notes: st.probe_notes.iter().chain(st.stencil_notes.iter()).cloned().collect(), writes_src: false };
    if st.pso_dirty && st.draws + st.indirect == 0 {
        pass.notes.push("SetPipelineState with targets bound but no hooked draw: captured anyway; draw count is a lower bound (the title may draw through an unhooked slot)".into());
    }
    let direct = list_type(list as usize) == 0;
    let mut slots: Vec<(String, usize, bool)> = rts.iter().enumerate().map(|(i, r)| (format!("RT{i}"), *r, false)).collect();
    if dsv != 0 {
        slots.push(("DSV".into(), dsv, true));
    }
    for (name, r, depth) in slots {
        let lab = cached_label(g, r);
        let va = if r == 0 || r == usize::MAX { String::new() } else if let Some(p) = crate::placed::lookup(r as *mut c_void) { format!(" placed title va={:#x}", p.va) } else if let Some(v) = crate::gpu_va::texture_va(r) { format!(" texture va={v:#x}") } else { " not placed".to_string() };
        pass.targets.push(format!("{name}={lab}{va}"));
        crate::va_census::touch(r, if depth { crate::va_census::P_DSV } else { crate::va_census::P_RTV });
        if r == g.last_src && r != 0 {
            pass.writes_src = true;
        }
        if r == 0 || r == usize::MAX || !want {
            continue;
        }
        if !direct {
            pass.notes.push(format!("{name}: list type is not DIRECT; not captured"));
            continue;
        }
        let st = g.lists.get(&k).unwrap();
        if st.split.contains(&r) {
            pass.notes.push(format!("{name}: split barrier pending; not captured"));
            continue;
        }
        let (s, why) = bound_state(st, r, depth);
        match capture_into(g, list as usize, r, s, why, &name) {
            Ok(i) => pass.caps.push(i),
            Err(e) => pass.notes.push(format!("{name}: {e}")),
        }
        if mips_on() && !depth {
            let mips = u16::from_le_bytes([desc_of(r)[0x1e], desc_of(r)[0x1f]]) as u32;
            for m in 1..mips.min(16) {
                match capture_into_mip(g, list as usize, r, s, why, &format!("{name}m{m}"), m) {
                    Ok(i) => pass.caps.push(i),
                    Err(e) => pass.notes.push(format!("{name} mip {m}: {e}")),
                }
            }
        }
    }
    let shot = g.shot.as_mut().unwrap();
    shot.order += 1;
    pass.order = shot.order;
    shot.passes.push(pass);
}

fn reset_pass(st: &mut LState) {
    st.draws = 0;
    st.dispatches = 0;
    st.indirect = 0;
    st.clears.clear();
    st.ps.clear();
    st.dirty = false;
    st.pso_dirty = false;
    st.probe_caps.clear();
    st.probe_notes.clear();
    st.stencil_notes.clear();
    st.stencil_seen.clear();
    st.seq += 1;
}

pub fn probe_hashes() -> &'static [u64] {
    static P: OnceLock<Vec<u64>> = OnceLock::new();
    P.get_or_init(|| std::env::var("TOMBOX_PASSCAP_PS").map(|v| v.split(|c: char| c == ',' || c.is_whitespace()).filter_map(|w| u64::from_str_radix(w.trim().trim_start_matches("0x"), 16).ok()).collect()).unwrap_or_default())
}

unsafe fn probe_draw(g: &mut G, list: *mut c_void, k: usize, h: u64) {
    let Some(st) = g.lists.get(&k) else { return };
    let mine = format!("probe PS {h:016x}:");
    let (all, this) = st.probe_notes.iter().fold((0usize, 0usize), |(a, t), x| (a + x.starts_with("probe PS ") as usize, t + x.starts_with(&mine) as usize));
    if this >= 2 || all >= 32 {
        return;
    }
    let mut cbv: Vec<(u32, u64)> = st.cbv.iter().map(|(a, b)| (*a, *b)).collect();
    cbv.sort();
    let pso = st.pso;
    let rts: Vec<usize> = st.rts.clone();
    let dsv = st.dsv;
    let mut notes = vec![format!("probe PS {h:016x}: title root CBVs set since graphics root signature {:#x} (execute-time GPU readback of each source in probes.txt)", st.groot)];
    let mut rcs: Vec<(u32, Vec<Option<u32>>)> = st.rc.iter().map(|(a, b)| (*a, b.clone())).collect();
    rcs.sort_by_key(|x| x.0);
    for (i, w) in &rcs {
        let words: Vec<String> = w.iter().enumerate().map(|(k, v)| match v { Some(u) => format!("[{k}]{u:#010x}/{}", f32::from_bits(*u)), None => format!("[{k}]unset") }).collect();
        notes.push(format!("  root[{i}] constants: {}", words.join(" ")));
    }
    for (i, va) in &cbv {
        let src = crate::cbv_copy::lookup_source(*va).map(|s| format!("PC buffer {:#x}+{:#x} ({:#x} bytes left, state {:#x})", s.res, s.off, s.left, s.state)).unwrap_or_else(|| "not inside any known PC buffer".into());
        notes.push(format!("  root[{i}] title va={va:#x} -> {src}"));
        if let Some(s) = crate::cbv_copy::lookup_source(*va) {
            let r = s.res as *mut c_void;
            let heap = crate::bindtrace::buffer_at(*va).map(|b| b.3);
            let backing = if crate::alias::is_aliased(r) {
                "(a) placed on an existing heap over the title's own pages: the GPU copy reads the title's CPU writes directly, no mirror"
            } else if heap == Some(2) {
                "(a) PC upload heap the title maps and writes itself: the GPU copy reads the title's CPU writes directly, no mirror"
            } else if crate::placed::is_committed_placement(r) {
                "(b) separate committed stand-in for title memory; upload.rs mirrors only Texture2D (unsupported_reason dim != 3), so this buffer has no mirror refresh path, no dirty state and no last-upload pump"
            } else {
                "unclassified: not aliased, not an upload heap bindtrace saw, not a committed placement"
            };
            notes.push(format!("  root[{i}] copy source backing: {backing}; heap type {}", heap.map_or("unknown (buffer not noted)".to_string(), |h| h.to_string())));
        }
        let path = if crate::cbv_raw::copy_on() { "cbv_raw copy mode: ring copy of the source recorded as a GPU CopyBufferRegion in this list, so it reads the source when the GPU runs it" } else if crate::cbv_raw::enabled() { "cbv_raw raw-view mode" } else { "direct root CBV" };
        notes.push(format!("  root[{i}] bind path: {path}; title va alignment {} (va & 0xff = {:#x})", if *va & 0xff == 0 { "256-byte aligned" } else { "NOT 256-byte aligned" }, *va & 0xff));
        if let Some(shot) = g.shot.as_mut() {
            shot.cpu_pending.push((k, *va, format!("probe PS {h:016x} root[{i}] title va={va:#x}")));
        }
        match crate::bindtrace::read_va(*va, cb_bytes()) {
            Some(bytes) => {
                let w: Vec<String> = bytes.chunks_exact(4).map(|x| format!("{:08x}", u32::from_le_bytes(x.try_into().unwrap()))).collect();
                for (r, c) in w.chunks(16).enumerate() {
                    notes.push(format!("  root[{i}] CPU view at record time dw[{:3}..] {}", r * 16, c.join(" ")));
                }
            }
            None => notes.push(format!("  root[{i}] CPU view at record time: not an upload-heap buffer bindtrace can map")),
        }
    }
    let rt_l: Vec<String> = rts.iter().map(|r| if *r == 0 { "-".into() } else { cached_label(g, *r) }).collect();
    let ds_l = if dsv == 0 { "-".to_string() } else { cached_label(g, dsv) };
    notes.push(format!("  bound RTs [{}] DSV {ds_l}", rt_l.join(" | ")));
    notes.push(format!("  PSO {pso:#x}: {}", crate::device_wrap::pso_desc_text(pso).unwrap_or_else(|| "no PSO desc recorded".into())));
    for param in 0..crate::bindtrace::table_count(list as usize, false).min(32) as u32 {
        let mut d3 = Vec::new();
        let mut n = 0usize;
        for slot in 0..256u32 {
            let Some((res, fmt, dim, map)) = crate::bindtrace::table_srv_fmt(list as usize, false, param, slot) else { continue };
            n += 1;
            if dim == 8 {
                let uint = matches!(fmt, 3 | 7 | 13 | 17 | 25 | 30 | 36 | 42 | 50 | 57 | 62);
                d3.push(format!("[{slot}] res={res:#x} pc_fmt={fmt} map={map:#x}{}", if uint { " UINT ok for Texture3D<uint4>" } else { " NOT a UINT format: a Texture3D<uint4> Load of it is undefined (MISMATCH if the shader declares uint)" }));
            }
        }
        if !d3.is_empty() {
            notes.push(format!("  table param {param}: {n} SRVs resolved, Texture3D views: {}", d3.join("; ")));
        }
    }
    notes.push(format!("  translation: {}", crate::fmask_check::translation_text(h)));
    let b = crate::bindtrace::dump(list as usize, false, &|r| format!("res {r:#x}"));
    notes.extend(b.lines().map(|l| format!("  {l}")));
    let mut srvs: Vec<usize> = Vec::new();
    for part in b.split("SRV res 0x").skip(1) {
        let hex: String = part.chars().take_while(|c| c.is_ascii_hexdigit()).collect();
        let is3d = grid3d_on() && part.lines().next().is_some_and(|l| l.contains(" dim=8 "));
        if let Ok(r) = usize::from_str_radix(&hex, 16) {
            if r != 0 && !srvs.contains(&r) && (srvs.len() < 8 || (is3d && srvs.len() < 12)) {
                srvs.push(r);
            }
        }
    }
    let mut caps = Vec::new();
    let direct = list_type(list as usize) == 0;
    for (n, r) in srvs.iter().enumerate() {
        if !direct {
            notes.push(format!("  SRV{n} {r:#x}: list type is not DIRECT; not captured"));
            continue;
        }
        let (state, why) = match g.lists.get(&k).and_then(|s| s.states.get(r).copied()) {
            Some(s) => (s, "in-list barrier"),
            None => match crate::rtcap::state_of(*r).or_else(|| Some(crate::gpu_va::state(*r)).filter(|&x| x != 0)) {
                Some(s) => (s, "tracked global state"),
                None => (0xc0, "default for an SRV"),
            },
        };
        match capture_into(g, list as usize, *r, state, why, &format!("SRV{n}")) {
            Ok(i) => caps.push(i),
            Err(e) => notes.push(format!("  SRV{n} {r:#x}: {e}")),
        }
    }
    let mut qh = 0usize;
    if g.device == 0 {
        g.device = list_device(list as usize).unwrap_or(0);
    }
    if direct && g.device != 0 {
        let qd: [u32; 3] = [2, 1, 0];
        let mut q: *mut c_void = core::ptr::null_mut();
        let hr = call!(g.device, 39, fn(*const u32, *const Guid, *mut *mut c_void) -> i32, qd.as_ptr(), &IID_QUERY_HEAP, &mut q);
        if hr >= 0 && !q.is_null() {
            qh = q as usize;
            let f: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, u32) = core::mem::transmute(crate::cmdlist_wrap::pc_method(list, 52));
            f(list, q, 3, 0);
        } else {
            notes.push(format!("  pipeline-statistics query heap creation failed hr={hr:#x}"));
        }
    }
    let mut oq = 0usize;
    if qh != 0 {
        let qd: [u32; 3] = [0, 1, 0];
        let mut q: *mut c_void = core::ptr::null_mut();
        let hr = call!(g.device, 39, fn(*const u32, *const Guid, *mut *mut c_void) -> i32, qd.as_ptr(), &IID_QUERY_HEAP, &mut q);
        if hr >= 0 && !q.is_null() {
            oq = q as usize;
            let f: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, u32) = core::mem::transmute(crate::cmdlist_wrap::pc_method(list, 52));
            f(list, q, 0, 0);
        } else {
            notes.push(format!("  occlusion query heap creation failed hr={hr:#x}"));
        }
    }
    if let Some(st) = g.lists.get_mut(&k) {
        st.probe_caps.extend(caps);
        st.probe_notes.extend(notes);
        st.probe_q = qh;
        st.probe_oq = oq;
    }
}

unsafe fn probe_after(this: *mut c_void) {
    if probe_hashes().is_empty() || !ACTIVE.load(Ordering::Relaxed) {
        return;
    }
    with(|g| {
        let k = key(this);
        let Some(st) = g.lists.get_mut(&k) else { return };
        let qh = core::mem::take(&mut st.probe_q);
        if qh == 0 {
            return;
        }
        let mut cbv: Vec<(u32, u64)> = st.cbv.iter().map(|(a, b)| (*a, *b)).collect();
        cbv.sort();
        let seq = st.seq;
        let dev = g.device;
        let list = this as usize;
        let mut out: Vec<ProbeBuf> = Vec::new();
        let oq = core::mem::take(&mut st.probe_oq);
        let end: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, u32) = core::mem::transmute(crate::cmdlist_wrap::pc_method(this, 53));
        end(this, qh as *mut c_void, 3, 0);
        if oq != 0 {
            end(this, oq as *mut c_void, 0, 0);
            match committed(dev, 3, &buffer_desc(256), 0x400) {
                Ok(rb) => {
                    let rs: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, u32, u32, *mut c_void, u64) = core::mem::transmute(crate::cmdlist_wrap::pc_method(this, 54));
                    rs(this, oq as *mut c_void, 0, 0, 1, rb as *mut c_void, 0);
                    out.push(ProbeBuf { rb, qh: oq, n: 8, kind: 3, label: format!("list {list:#x} pass seq {seq} draw occlusion query (samples that survived discard and passed the depth and stencil tests; for a stencil-writing draw with pass=REPLACE this is the number of samples whose stencil was written)") });
                }
                Err(hr) => {
                    call!(oq, 2, fn() -> u32);
                    out.push(ProbeBuf { rb: 0, qh: 0, n: 0, kind: 2, label: format!("list {list:#x} pass seq {seq}: occlusion readback buffer failed hr={hr:#x}") });
                }
            }
        }
        match committed(dev, 3, &buffer_desc(256), 0x400) {
            Ok(rb) => {
                let rs: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, u32, u32, *mut c_void, u64) = core::mem::transmute(crate::cmdlist_wrap::pc_method(this, 54));
                rs(this, qh as *mut c_void, 3, 0, 1, rb as *mut c_void, 0);
                out.push(ProbeBuf { rb, qh, n: 88, kind: 0, label: format!("list {list:#x} pass seq {seq} draw pipeline statistics") });
            }
            Err(hr) => {
                call!(qh, 2, fn() -> u32);
                out.push(ProbeBuf { rb: 0, qh: 0, n: 0, kind: 2, label: format!("list {list:#x} pass seq {seq}: query readback buffer failed hr={hr:#x}") });
            }
        }
        for (i, va) in cbv {
            let Some(src) = crate::cbv_copy::lookup_source(va) else {
                out.push(ProbeBuf { rb: 0, qh: 0, n: 0, kind: 2, label: format!("list {list:#x} pass seq {seq} root[{i}] title va={va:#x}: not inside any known PC buffer, no readback") });
                continue;
            };
            let n = src.left.min(1024);
            let Ok(rb) = committed(dev, 3, &buffer_desc(1024), 0x400) else { continue };
            let bar = crate::cbv_copy::source_barrier(src.state);
            if let Some((before, during)) = bar {
                list_call_barrier(list, &[barrier_sub0(src.res, before, during)]);
            }
            let cp: unsafe extern "system" fn(*mut c_void, *mut c_void, u64, *mut c_void, u64, u64) = core::mem::transmute(crate::cmdlist_wrap::pc_method(this, 15));
            cp(this, rb as *mut c_void, 0, src.res as *mut c_void, src.off, n);
            if let Some((before, during)) = bar {
                list_call_barrier(list, &[barrier_sub0(src.res, during, before)]);
            }
            out.push(ProbeBuf { rb, qh: 0, n: n as usize, kind: 1, label: format!("list {list:#x} pass seq {seq} root[{i}] title va={va:#x} = PC buffer {:#x}+{:#x} (left {:#x}, state {:#x}), copied after the draw on the GPU timeline", src.res, src.off, src.left, src.state) });
        }
        if let Some(shot) = g.shot.as_mut() {
            shot.probes.extend(out);
        } else {
            for p in out {
                if p.rb != 0 {
                    call!(p.rb, 2, fn() -> u32);
                }
                if p.qh != 0 {
                    call!(p.qh, 2, fn() -> u32);
                }
            }
        }
    })
}

unsafe fn probe_text(p: &ProbeBuf) -> String {
    if p.kind == 2 || p.rb == 0 {
        return format!("{}\n", p.label);
    }
    let mut ptr: *mut u8 = core::ptr::null_mut();
    let range = [0usize, p.n];
    let mut t = format!("{}\n", p.label);
    if call!(p.rb, 8, fn(u32, *const usize, *mut *mut u8) -> i32, 0, range.as_ptr(), &mut ptr) < 0 || ptr.is_null() {
        t.push_str("  readback Map failed\n");
        return t;
    }
    let bytes = core::slice::from_raw_parts(ptr, p.n).to_vec();
    let empty = [0usize, 0usize];
    call!(p.rb, 9, fn(u32, *const usize) -> (), 0, empty.as_ptr());
    if p.kind == 3 {
        t.push_str(&format!("  samples passed = {}
", u64::from_le_bytes(bytes[0..8].try_into().unwrap())));
        return t;
    }
    if p.kind == 0 {
        let q = |i: usize| u64::from_le_bytes(bytes[i * 8..i * 8 + 8].try_into().unwrap());
        t.push_str(&format!("  IAVertices={} IAPrimitives={} VSInvocations={} GSInvocations={} GSPrimitives={} CInvocations={} CPrimitives={} PSInvocations={} HS={} DS={} CS={}\n", q(0), q(1), q(2), q(3), q(4), q(5), q(6), q(7), q(8), q(9), q(10)));
    } else {
        for (row, c) in bytes.chunks(16).enumerate() {
            let w: Vec<String> = c.chunks(4).filter(|x| x.len() == 4).map(|x| { let u = u32::from_le_bytes(x.try_into().unwrap()); format!("{u:#010x}/{}", f32::from_bits(u)) }).collect();
            t.push_str(&format!("  [{:3}] {}\n", row * 4, w.join(" ")));
        }
    }
    t
}

fn note_rc(this: *mut c_void, i: u32, off: u32, vals: &[u32]) {
    with(|g| {
        let st = g.lists.entry(key(this)).or_default();
        let e = st.rc.entry(i).or_default();
        for (k, v) in vals.iter().enumerate() {
            let at = off as usize + k;
            if at >= 64 {
                break;
            }
            if e.len() <= at {
                e.resize(at + 1, None);
            }
            e[at] = Some(*v);
        }
    });
}

unsafe extern "system" fn set_stencil_ref(this: *mut c_void, r: u32) {
    with(|g| g.lists.entry(key(this)).or_default().sref = Some(r));
    let f: unsafe extern "system" fn(*mut c_void, u32) = core::mem::transmute(prev(24));
    f(this, r)
}

unsafe extern "system" fn set_graphics_root_c1(this: *mut c_void, i: u32, v: u32, off: u32) {
    note_rc(this, i, off, &[v]);
    let f: unsafe extern "system" fn(*mut c_void, u32, u32, u32) = core::mem::transmute(prev(38));
    f(this, i, v, off)
}

unsafe extern "system" fn set_graphics_root_cn(this: *mut c_void, i: u32, n: u32, p: *const c_void, off: u32) {
    if !p.is_null() && n > 0 {
        let vals = std::slice::from_raw_parts(p as *const u32, (n as usize).min(64));
        note_rc(this, i, off, vals);
    }
    let f: unsafe extern "system" fn(*mut c_void, u32, u32, *const c_void, u32) = core::mem::transmute(prev(40));
    f(this, i, n, p, off)
}

unsafe extern "system" fn set_graphics_root_sig(this: *mut c_void, rs: *mut c_void) {
    with(|g| {
        let st = g.lists.entry(key(this)).or_default();
        if st.groot != rs as usize {
            st.groot = rs as usize;
            st.cbv.clear();
            st.rc.clear();
        }
    });
    let f: unsafe extern "system" fn(*mut c_void, *mut c_void) = core::mem::transmute(prev(34));
    f(this, rs)
}

unsafe extern "system" fn set_compute_table(this: *mut c_void, i: u32, h: u64) {
    if !crate::rtcap::enabled() && tables_on() {
        crate::bindtrace::set_table(this as usize, true, i, h);
    }
    let f: unsafe extern "system" fn(*mut c_void, u32, u64) = core::mem::transmute(prev(35));
    f(this, i, h)
}

fn cs_of(pso: usize) -> u64 {
    let real = crate::pso_async::real_of(pso);
    for p in [pso, real] {
        if p == 0 {
            continue;
        }
        if let Some((_, c)) = crate::device_x::pso_stage_codes(p).into_iter().find(|(n, _)| *n == "cs") {
            return crate::shader_xlat::origin_hash(c).unwrap_or(1);
        }
    }
    0
}

fn note_dispatch(this: *mut c_void, x: u32, y: u32, z: u32) {
    with(|g| {
        if g.shot.is_none() {
            return;
        }
        let k = key(this);
        let Some(st) = g.lists.get(&k) else { return };
        if st.probe_notes.iter().filter(|n| n.starts_with("dispatch ")).count() >= 24 {
            return;
        }
        let (pso, n) = (st.pso, st.dispatches);
        let cs = cs_of(pso);
        let mut uavs = Vec::new();
        for param in 0..crate::bindtrace::table_count(this as usize, true).min(16) as u32 {
            for slot in 0..16u32 {
                if let Some((res, d)) = crate::bindtrace::table_uav(this as usize, true, param, slot) {
                    let lab = if res == 0 { "null".to_string() } else { cached_label(g, res) };
                    uavs.push(format!("param[{param}]+{slot} UAV {lab} | {}", crate::bindtrace::uav_view_text(d)));
                }
            }
        }
        let st = g.lists.get_mut(&k).unwrap();
        st.probe_notes.push(format!("dispatch {n} cs={cs:016x} pso={pso:#x} groups={x}x{y}x{z}: {} compute-table UAVs (TOMBOX_PASSCAP: wave size and LDS come from translating cs offline)", uavs.len()));
        for u in uavs.into_iter().take(32) {
            st.probe_notes.push(format!("  {u}"));
        }
    });
}

unsafe extern "system" fn set_graphics_table(this: *mut c_void, i: u32, h: u64) {
    if !crate::rtcap::enabled() && tables_on() {
        crate::bindtrace::set_table(this as usize, false, i, h);
    }
    let f: unsafe extern "system" fn(*mut c_void, u32, u64) = core::mem::transmute(prev(36));
    f(this, i, h)
}

unsafe extern "system" fn set_graphics_root_cbv(this: *mut c_void, i: u32, va: u64) {
    with(|g| {
        g.lists.entry(key(this)).or_default().cbv.insert(i, va);
    });
    let f: unsafe extern "system" fn(*mut c_void, u32, u64) = core::mem::transmute(prev(42));
    f(this, i, va)
}

unsafe fn views(n: u32, handles: *const usize, single: bool) -> Vec<usize> {
    let inc = crate::rtcap::rtv_increment();
    (0..(n as usize).min(8)).map(|i| {
        if handles.is_null() {
            return 0;
        }
        let h = if single { (*handles).wrapping_add(i * inc) } else { *handles.add(i) };
        crate::rtcap::view_of(h).unwrap_or(usize::MAX)
    }).collect()
}

unsafe extern "system" fn om_set_targets(this: *mut c_void, n: u32, handles: *const usize, single: i32, dsv: *const usize) {
    let rts = views(n, handles, single != 0);
    let d = if dsv.is_null() { 0 } else { crate::rtcap::view_of(*dsv).unwrap_or(usize::MAX) };
    with(|g| {
        g.fc.binds += 1;
        g.fc.unresolved += rts.iter().filter(|r| **r == usize::MAX).count() as u64 + (d == usize::MAX) as u64;
        for r in rts.iter().chain(core::iter::once(&d)).filter(|r| **r != 0 && **r != usize::MAX && ACTIVE.load(Ordering::Relaxed)) {
            if !g.desc_cache.contains_key(r) {
                let dd = desc_of(*r);
                g.desc_cache.insert(*r, dd);
            }
        }
        let k = key(this);
        let changed = g.lists.get(&k).map(|s| s.rts != rts || s.dsv != d).unwrap_or(true);
        if changed {
            boundary(g, this, "OMSetRenderTargets");
            let st = g.lists.entry(k).or_default();
            reset_pass(st);
            st.rts = rts;
            st.dsv = d;
        }
    });
    let f: unsafe extern "system" fn(*mut c_void, u32, *const usize, i32, *const usize) = core::mem::transmute(prev(50));
    f(this, n, handles, single, dsv)
}

fn note_draw(this: *mut c_void, kind: u8) {
    with(|g| {
        let k = key(this);
        let active = ACTIVE.load(Ordering::Relaxed);
        let st = g.lists.entry(k).or_default();
        match kind {
            2 => st.dispatches += 1,
            3 => st.indirect += 1,
            _ => st.draws += 1,
        }
        if kind != 2 {
            st.dirty = true;
        }
        let rts = if kind == 2 { Vec::new() } else { st.rts.clone() };
        let pso = st.pso;
        match kind {
            2 => g.fc.dispatches += 1,
            3 => g.fc.indirect += 1,
            _ => g.fc.draws += 1,
        }
        if kind != 2 {
            if rts.contains(&g.last_src) && g.last_src != 0 {
                g.fc.into_src += 1;
            }
            for r in rts.iter().filter(|r| **r != 0) {
                *g.fc.per_rt.entry(*r).or_default() += 1;
            }
        }
        if !active && kind != 2 && pso != 0 {
            if let Some(t) = ps_draw_target() {
                let h = *g.slow_cache.entry(pso).or_insert_with(|| ps_of(pso));
                if h == t {
                    g.ps_seen = true;
                }
            }
        }
        if !active && kind != 2 && pso != 0 && SLOW_HIT.load(Ordering::Relaxed) {
            if let Some(t) = ps_ms_target() {
                let h = *g.slow_cache.entry(pso).or_insert_with(|| ps_of(pso));
                if h == t {
                    g.slow_seen = true;
                }
            }
        }
        if active && kind != 2 {
            if let Some(shot) = g.shot.as_mut() {
                let h = ps_hash(shot, pso);
                let st = g.lists.get_mut(&k).unwrap();
                if !st.ps.contains(&h) && st.ps.len() < 160 {
                    st.ps.push(h);
                }
                if st.dsv != 0 {
                    let sref = st.sref;
                    if !st.stencil_seen.contains(&(pso, sref)) && st.stencil_notes.len() < 32 {
                        st.stencil_seen.push((pso, sref));
                        if let Some((d, _)) = crate::device_wrap::gfx_pso_desc(crate::pso_async::real_of(pso)).or_else(|| crate::device_wrap::gfx_pso_desc(pso)) {
                            let u = |o: usize| d.get(o..o + 4).map_or(0, |b| u32::from_le_bytes(b.try_into().unwrap()));
                            if u(0x1fc) != 0 {
                                let r = sref.map_or("never set in this list".to_string(), |v| format!("{v:#x}"));
                                st.stencil_notes.push(format!("stencil draw ps={h:016x} pso={pso:#x} OMSetStencilRef={r} rm={:#x} wm={:#x} front fail/dfail/pass/func={}/{}/{}/{} back={}/{}/{}/{} depth(en={} write={} func={})",
                                    d.get(0x200).copied().unwrap_or(0), d.get(0x201).copied().unwrap_or(0), u(0x204), u(0x208), u(0x20c), u(0x210), u(0x214), u(0x218), u(0x21c), u(0x220), u(0x1f0), u(0x1f4), u(0x1f8)));
                            }
                        } else {
                            st.stencil_notes.push(format!("stencil state UNKNOWN for draw ps={h:016x} pso={pso:#x}: PSO desc not in the registry (stream or derived PSO)"));
                        }
                    }
                }
                if probe_hashes().contains(&h) {
                    unsafe { probe_draw(g, this, k, h) };
                }
            }
        }
    })
}

unsafe extern "system" fn draw(this: *mut c_void, a: u32, b: u32, c: u32, d: u32) {
    note_draw(this, 0);
    let f: unsafe extern "system" fn(*mut c_void, u32, u32, u32, u32) = core::mem::transmute(prev(12));
    f(this, a, b, c, d);
    probe_after(this)
}
unsafe extern "system" fn draw_indexed(this: *mut c_void, a: u32, b: u32, c: u32, d: i32, e: u32) {
    note_draw(this, 1);
    let f: unsafe extern "system" fn(*mut c_void, u32, u32, u32, i32, u32) = core::mem::transmute(prev(13));
    f(this, a, b, c, d, e);
    probe_after(this)
}
unsafe extern "system" fn dispatch(this: *mut c_void, a: u32, b: u32, c: u32) {
    note_draw(this, 2);
    if ACTIVE.load(Ordering::Relaxed) {
        note_dispatch(this, a, b, c);
    }
    let f: unsafe extern "system" fn(*mut c_void, u32, u32, u32) = core::mem::transmute(prev(14));
    f(this, a, b, c)
}
unsafe extern "system" fn draw_indexed_x(this: *mut c_void, va: u64, fmt: u32, n: u32, inst: u32) {
    note_draw(this, 1);
    let f: unsafe extern "system" fn(*mut c_void, u64, u32, u32, u32) = core::mem::transmute(prev(81));
    f(this, va, fmt, n, inst)
}
unsafe extern "system" fn dispatch_mesh_x(this: *mut c_void, n: u32, a: u64, b: u64, c: u64) {
    note_draw(this, 0);
    let f: unsafe extern "system" fn(*mut c_void, u32, u64, u64, u64) = core::mem::transmute(prev(98));
    f(this, n, a, b, c)
}
unsafe extern "system" fn dispatch_mesh(this: *mut c_void, x: u32, y: u32, z: u32) {
    note_draw(this, 0);
    let f: unsafe extern "system" fn(*mut c_void, u32, u32, u32) = core::mem::transmute(prev(127));
    f(this, x, y, z)
}
unsafe extern "system" fn indirect(this: *mut c_void, sig: *mut c_void, n: u32, buf: *mut c_void, off: u64, cbuf: *mut c_void, coff: u64) {
    note_draw(this, 3);
    let f: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, *mut c_void, u64, *mut c_void, u64) = core::mem::transmute(prev(59));
    f(this, sig, n, buf, off, cbuf, coff)
}

unsafe extern "system" fn set_pso(this: *mut c_void, pso: *mut c_void) {
    with(|g| {
        let st = g.lists.entry(key(this)).or_default();
        st.pso = pso as usize;
        if st.rts.iter().any(|r| *r != 0) || st.dsv != 0 {
            st.pso_dirty = true;
            st.dirty = true;
        }
    });
    let f: unsafe extern "system" fn(*mut c_void, *mut c_void) = core::mem::transmute(prev(25));
    f(this, pso)
}

unsafe extern "system" fn barrier(this: *mut c_void, n: u32, p: *const u8) {
    if n != 0 && !p.is_null() {
        let fixed = crate::xone_barrier::sanitize(n, p);
        let bytes: Vec<u8> = match fixed {
            Some(v) => v,
            None => core::slice::from_raw_parts(p, n as usize * 32).to_vec(),
        };
        with(|g| {
            let st = g.lists.entry(key(this)).or_default();
            for b in bytes.chunks_exact(32) {
                if d32(b, 0) != 0 {
                    continue;
                }
                let flags = d32(b, 4);
                let res = u64::from_le_bytes(b[8..16].try_into().unwrap()) as usize;
                let sub = d32(b, 16);
                if flags & 1 != 0 {
                    if !st.split.contains(&res) {
                        st.split.push(res);
                    }
                    continue;
                }
                if flags & 2 != 0 {
                    st.split.retain(|r| *r != res);
                }
                if sub == 0 || sub == u32::MAX {
                    st.states.insert(res, d32(b, 24));
                }
            }
        });
    }
    let f: unsafe extern "system" fn(*mut c_void, u32, *const u8) = core::mem::transmute(prev(26));
    f(this, n, p)
}

unsafe extern "system" fn clear_rtv(this: *mut c_void, h: usize, color: *const f32, n: u32, rects: *const c_void) {
    let res = crate::rtcap::view_of(h).unwrap_or(usize::MAX);
    let c = if color.is_null() { [0.0; 4] } else { [*color, *color.add(1), *color.add(2), *color.add(3)] };
    with(|g| {
        let lab = cached_label(g, res);
        let st = g.lists.entry(key(this)).or_default();
        st.dirty = true;
        if st.clears.len() < 8 {
            st.clears.push(format!("clearRTV {lab} ({:.4},{:.4},{:.4},{:.4}) rects={n}", c[0], c[1], c[2], c[3]));
        }
    });
    let f: unsafe extern "system" fn(*mut c_void, usize, *const f32, u32, *const c_void) = core::mem::transmute(prev(52));
    f(this, h, color, n, rects)
}

unsafe extern "system" fn clear_dsv(this: *mut c_void, h: usize, flags: u32, depth: f32, stencil: u8, n: u32, rects: *const c_void) {
    let res = crate::rtcap::view_of(h).unwrap_or(usize::MAX);
    with(|g| {
        let lab = cached_label(g, res);
        let st = g.lists.entry(key(this)).or_default();
        st.dirty = true;
        if st.clears.len() < 8 {
            st.clears.push(format!("clearDSV {lab} flags={flags} depth={depth} stencil={stencil} rects={n}"));
        }
    });
    let f: unsafe extern "system" fn(*mut c_void, usize, u32, f32, u8, u32, *const c_void) = core::mem::transmute(prev(51));
    f(this, h, flags, depth, stencil, n, rects)
}

unsafe fn after_write(this: *mut c_void, what: &str, dst: usize, src: usize, default_state: u32, resolve: bool) {
    with(|g| {
        if resolve {
            g.fc.resolves += 1;
        } else {
            g.fc.copies += 1;
        }
        let into_src = dst == g.last_src && dst != 0;
        let from_src = src == g.last_src && src != 0;
        if (into_src || from_src) && g.fc.to_src.len() < 8 {
            let t = format!("{what} {}{} <- {}{}", if into_src { "*SRC* " } else { "" }, cached_label(g, dst), if from_src { "*SRC* " } else { "" }, cached_label(g, src));
            g.fc.to_src.push(t);
        }
        if !ACTIVE.load(Ordering::Relaxed) || dst == 0 {
            return;
        }
        let d = *g.desc_cache.entry(dst).or_insert_with(|| desc_of(dst));
        if !(into_src || d32(&d, 0x30) & 3 != 0 || sel().map(|s| s.all).unwrap_or(false)) || d32(&d, 0) != 3 {
            return;
        }
        let k = key(this);
        let st = g.lists.entry(k).or_default();
        let (s, why) = match st.states.get(&dst) {
            Some(&s) => (s, "in-list barrier"),
            None => (default_state, "default for the copy/resolve destination"),
        };
        let seq = st.seq;
        let mut pass = Pass { list: this as usize, key: k, order: 0, seq, end: format!("{what} dst={} src={}", cached_label(g, dst), cached_label(g, src)), draws: 0, dispatches: 0, indirect: 0,
            clears: Vec::new(), ps: Vec::new(), targets: vec![format!("dst={}", cached_label(g, dst))], caps: Vec::new(), notes: Vec::new(), writes_src: into_src };
        if list_type(this as usize) != 0 {
            pass.notes.push("list type is not DIRECT; not captured".into());
        } else {
            match capture_into(g, this as usize, dst, s, why, "dst") {
                Ok(i) => pass.caps.push(i),
                Err(e) => pass.notes.push(e),
            }
        }
        let shot = g.shot.as_mut().unwrap();
        shot.order += 1;
        pass.order = shot.order;
        shot.passes.push(pass);
    })
}

unsafe extern "system" fn resolve(this: *mut c_void, dst: *mut c_void, dsub: u32, src: *mut c_void, ssub: u32, fmt: u32) {
    let f: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, *mut c_void, u32, u32) = core::mem::transmute(prev(19));
    f(this, dst, dsub, src, ssub, fmt);
    after_write(this, &format!("ResolveSubresource(sub {dsub}<-{ssub} fmt={fmt})"), if dsub == 0 { dst as usize } else { 0 }, src as usize, ST_RESOLVE_DEST, true);
}

unsafe fn loc_res(l: *const u8) -> (usize, u32, u32) {
    if l.is_null() {
        return (0, 9, 0);
    }
    (core::ptr::read_unaligned(l as *const u64) as usize, core::ptr::read_unaligned(l.add(8) as *const u32), core::ptr::read_unaligned(l.add(16) as *const u32))
}

unsafe extern "system" fn copy_region(this: *mut c_void, dst: *const u8, x: u32, y: u32, z: u32, src: *const u8, bx: *const c_void) {
    let f: unsafe extern "system" fn(*mut c_void, *const u8, u32, u32, u32, *const u8, *const c_void) = core::mem::transmute(prev(16));
    f(this, dst, x, y, z, src, bx);
    let (d, dt, ds) = loc_res(dst);
    let (s, _, _) = loc_res(src);
    after_write(this, "CopyTextureRegion", if dt == 0 && ds == 0 { d } else { 0 }, s, ST_COPY_DEST, false);
}
unsafe extern "system" fn copy_region_x(this: *mut c_void, dst: *const u8, x: u32, y: u32, z: u32, src: *const u8, bx: *const c_void, flags: u32) {
    let f: unsafe extern "system" fn(*mut c_void, *const u8, u32, u32, u32, *const u8, *const c_void, u32) = core::mem::transmute(prev(83));
    f(this, dst, x, y, z, src, bx, flags);
    let (d, dt, ds) = loc_res(dst);
    let (s, _, _) = loc_res(src);
    after_write(this, "CopyTextureRegionX", if dt == 0 && ds == 0 { d } else { 0 }, s, ST_COPY_DEST, false);
}
unsafe extern "system" fn copy_resource(this: *mut c_void, dst: *mut c_void, src: *mut c_void) {
    let f: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) = core::mem::transmute(prev(17));
    f(this, dst, src);
    after_write(this, "CopyResource", dst as usize, src as usize, ST_COPY_DEST, false);
}
unsafe extern "system" fn copy_resource_x(this: *mut c_void, dst: *mut c_void, src: *mut c_void, flags: u32) {
    let f: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void, u32) = core::mem::transmute(prev(84));
    f(this, dst, src, flags);
    after_write(this, "CopyResourceX", dst as usize, src as usize, ST_COPY_DEST, false);
}

unsafe extern "system" fn close(this: *mut c_void) -> i32 {
    with(|g| {
        boundary(g, this, "Close");
        if let Some(st) = g.lists.get_mut(&key(this)) {
            reset_pass(st);
        }
    });
    let f: unsafe extern "system" fn(*mut c_void) -> i32 = core::mem::transmute(prev(PC_CLOSE));
    f(this)
}

unsafe extern "system" fn reset(this: *mut c_void, alloc: *mut c_void, pso: *mut c_void) -> i32 {
    with(|g| {
        let k = key(this);
        g.lists.remove(&k);
        g.lists.remove(&(this as usize));
        if let Some(s) = g.shot.as_mut() {
            if !s.passes.iter().any(|p| p.list == this as usize) {
                s.submit.remove(&(this as usize));
            }
        }
    });
    let f: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32 = core::mem::transmute(prev(PC_RESET));
    let hr = f(this, alloc, pso);
    if !pso.is_null() {
        with(|g| g.lists.entry(key(this)).or_default().pso = pso as usize);
    }
    hr
}

pub fn install(vt: &[AtomicUsize]) {
    if !on() {
        return;
    }
    let hooks: [(usize, usize); 26] = [
        (36, set_graphics_table as *const () as usize), (35, set_compute_table as *const () as usize), (24, set_stencil_ref as *const () as usize), (42, set_graphics_root_cbv as *const () as usize), (34, set_graphics_root_sig as *const () as usize), (38, set_graphics_root_c1 as *const () as usize), (40, set_graphics_root_cn as *const () as usize),
        (9, close as *const () as usize), (10, reset as *const () as usize), (12, draw as *const () as usize), (13, draw_indexed as *const () as usize), (14, dispatch as *const () as usize),
        (16, copy_region as *const () as usize), (17, copy_resource as *const () as usize), (19, resolve as *const () as usize), (25, set_pso as *const () as usize), (26, barrier as *const () as usize),
        (50, om_set_targets as *const () as usize), (51, clear_dsv as *const () as usize), (52, clear_rtv as *const () as usize), (59, indirect as *const () as usize), (83, copy_region_x as *const () as usize),
        (84, copy_resource_x as *const () as usize), (81, draw_indexed_x as *const () as usize), (98, dispatch_mesh_x as *const () as usize),
        (127, dispatch_mesh as *const () as usize),
    ];
    let mut missing = Vec::new();
    for (slot, f) in hooks {
        let p = vt[slot].load(Ordering::Relaxed);
        if p == 0 {
            missing.push(slot);
            continue;
        }
        PREV[slot].store(p, Ordering::Relaxed);
        vt[slot].store(f, Ordering::Relaxed);
    }
    let s = sel().unwrap();
    record("d3d12_xs", "passcap", &format!(
        "TOMBOX_PASSCAP armed: shots at t={:?} s after first present, presents={:?}, file trigger={} ({}\\now or %TEMP%\\tombox_passcap_now), frames/shot={}, budget {} MB / {} captures, chain log every {} presents, output {}; hooks chained on Xbox slots 9,10,12,13,14,16,17,19,24,25,26,34,35,36,38,40,42,50,51,52,59,81,83,84,98,127; probe PS {:x?}{}{} (draws are never skipped or changed; captures add barrier+copy commands only on capture frames)",
        s.secs, s.presents, s.file, out_root().display(), s.frames, s.budget_mb, s.max_caps, s.chain_every, out_root().display(), probe_hashes(),
        if missing.is_empty() { String::new() } else { format!("; empty slots left alone: {missing:?}") },
        if s.unknown.is_empty() { String::new() } else { format!("; ignored tokens {:?}", s.unknown) }));
}

pub fn note_submit(queue: usize, real: &[*mut c_void]) {
    if !on() {
        return;
    }
    with(|g| {
        let Some(shot) = g.shot.as_mut() else { return };
        let keys: Vec<usize> = real.iter().map(|l| *l as usize).collect();
        let mut keep = Vec::new();
        for (lk, va, label) in core::mem::take(&mut shot.cpu_pending) {
            if keys.iter().any(|&x| x == lk || key(x as *mut c_void) == lk) {
                let text = match unsafe { crate::bindtrace::read_va(va, cb_bytes()) } {
                    Some(bytes) => bytes.chunks_exact(4).map(|x| format!("{:08x}", u32::from_le_bytes(x.try_into().unwrap()))).collect::<Vec<_>>().chunks(16).enumerate().map(|(r, c)| format!("  dw[{:3}..] {}", r * 16, c.join(" "))).collect::<Vec<_>>().join("\n"),
                    None => "  not an upload-heap buffer bindtrace can map".to_string(),
                };
                shot.cpu_submit.push(format!("{label}: CPU view at ExecuteCommandLists (queue {queue:#x})\n{text}"));
            } else {
                keep.push((lk, va, label));
            }
        }
        shot.cpu_pending = keep;
        for l in real {
            if !shot.submit.contains_key(&(*l as usize)) {
                g.submit_seq += 1;
                shot.submit.insert(*l as usize, (g.submit_seq, queue));
            }
        }
    })
}

unsafe fn plane_text(plane_count: u32, planes: *const u8) -> String {
    if planes.is_null() {
        return format!("planes={plane_count} (null)");
    }
    let words: Vec<String> = (0..(plane_count as usize * 4).clamp(4, 16)).map(|i| format!("{:x}", core::ptr::read_unaligned((planes as *const u64).add(i)))).collect();
    let n = core::ptr::read_unaligned(planes.add(8) as *const u32);
    let pp = core::ptr::read_unaligned(planes.add(0x10) as *const *const usize);
    let res: Vec<String> = if pp.is_null() { Vec::new() } else { (0..(n as usize).min(4)).map(|i| format!("{:#x}", *pp.add(i))).collect() };
    format!("planes={plane_count} plane0 res_count={n} res=[{}] raw=[{}]", res.join(" "), words.join(" "))
}

fn chain_line(g: &mut G, src: usize, planes: &str) -> String {
    let fc = core::mem::take(&mut g.fc);
    let mut per: Vec<(usize, u64)> = fc.per_rt.iter().map(|(a, b)| (*a, *b)).collect();
    per.sort_by(|a, b| b.1.cmp(&a.1));
    let known = |r: usize| g.desc_cache.get(&r).map(|d| label_of(r, d)).unwrap_or_else(|| format!("{r:#x}"));
    let top: Vec<String> = per.iter().take(6).map(|(r, c)| format!("{}{} x{c}", if *r == src { "*SRC* " } else { "" }, known(*r))).collect();
    let src_label = label_of(src, &unsafe { desc_of(src) });
    format!("src={} | {planes} | frame: draws={} dispatches={} indirect={} resolves={} copies={} binds={} unresolved_views={} draws_into_src={} | to/from src: [{}] | top rts: [{}]",
        src_label, fc.draws, fc.dispatches, fc.indirect, fc.resolves, fc.copies, fc.binds, fc.unresolved, fc.into_src, fc.to_src.join("; "), top.join("; "))
}

unsafe fn own_objects(g: &mut G, queue: *mut c_void) -> Option<(usize, usize, usize)> {
    if let Some((a, l, f, _)) = g.own {
        return Some((a, l, f));
    }
    let mut dev: *mut c_void = core::ptr::null_mut();
    if call!(queue, 7, fn(*const Guid, *mut *mut c_void) -> i32, &IID_DEVICE, &mut dev) < 0 || dev.is_null() {
        return None;
    }
    let mut a: *mut c_void = core::ptr::null_mut();
    let mut l: *mut c_void = core::ptr::null_mut();
    let mut f: *mut c_void = core::ptr::null_mut();
    let ok = call!(dev, 9, fn(u32, *const Guid, *mut *mut c_void) -> i32, 0, &IID_ALLOCATOR, &mut a) >= 0
        && call!(dev, 12, fn(u32, u32, *mut c_void, *mut c_void, *const Guid, *mut *mut c_void) -> i32, 0, 0, a, core::ptr::null_mut(), &IID_GFX_LIST, &mut l) >= 0
        && call!(dev, 36, fn(u64, u32, *const Guid, *mut *mut c_void) -> i32, 0, 0, &IID_FENCE, &mut f) >= 0;
    if g.device == 0 {
        g.device = dev as usize;
    }
    call!(dev, 2, fn() -> u32);
    if !ok {
        return None;
    }
    call!(l, PC_CLOSE, fn() -> i32);
    g.own = Some((a as usize, l as usize, f as usize, 0));
    Some((a as usize, l as usize, f as usize))
}

unsafe fn wait_fence(f: usize, v: u64, ms: u32) -> bool {
    extern "system" {
        fn Sleep(ms: u32);
    }
    let mut spins = 0;
    while call!(f, 8, fn() -> u64) < v {
        if spins >= ms {
            return false;
        }
        Sleep(1);
        spins += 1;
    }
    true
}

pub unsafe fn on_present(queue: *mut c_void, src: *mut c_void, plane_count: u32, planes: *const u8) {
    let Some(s) = sel() else { return };
    let ptext = plane_text(plane_count, planes);
    let mut run_list: Option<usize> = None;
    let mut finalize: Option<Shot> = None;
    {
        let mut guard = lock();
        let g = guard.get_or_insert_with(G::default);
        g.presents += 1;
        let t0 = *g.t0.get_or_insert_with(std::time::Instant::now);
        let t = t0.elapsed().as_secs_f64();
        let src_u = src as usize;
        let d = desc_of(src_u);
        let changed = src_u != g.last_src || g.last_src_desc != d[..];
        let into_zero = g.fc.into_src == 0;
        let flip = g.last_into_src_zero.map(|z| z != into_zero).unwrap_or(true);
        let periodic = s.chain_every != 0 && g.presents % s.chain_every == 0;
        let recording_end = ACTIVE.load(Ordering::Relaxed) && g.shot.as_ref().map(|sh| sh.frames_left <= 1).unwrap_or(false);
        static LINES: AtomicUsize = AtomicUsize::new(0);
        let quota = changed || periodic || recording_end || LINES.load(Ordering::Relaxed) < 400;
        if (changed || flip || periodic || recording_end) && quota && LINES.fetch_add(1, Ordering::Relaxed) < 4000 {
            let line = chain_line(g, src_u, &ptext);
            if periodic {
                g.desc_cache.clear();
            }
            record("d3d12_xs", "passcap.chain", &format!("present #{} t={t:.2}s{}{} {line}", g.presents,
                if changed { " SRC-CHANGED" } else { "" }, if flip { if into_zero { " DRAWS-INTO-SRC-STOPPED" } else { " DRAWS-INTO-SRC-STARTED" } } else { "" }));
            if recording_end {
                if let Some(sh) = g.shot.as_mut() {
                    sh.chain = line;
                }
            }
        } else {
            g.fc = FrameCount::default();
        }
        g.last_into_src_zero = Some(into_zero);
        g.last_src = src_u;
        g.last_src_desc = d.to_vec();
        if ACTIVE.load(Ordering::Relaxed) {
            let done = g.shot.as_mut().map(|sh| {
                sh.frames_left = sh.frames_left.saturating_sub(1);
                sh.frames_left == 0
            }).unwrap_or(true);
            if done {
                ACTIVE.store(false, Ordering::Relaxed);
                if let Some((a, l, _)) = own_objects(g, queue) {
                    let lab = label_of(src_u, &d);
                    call!(a, 8, fn() -> i32);
                    call!(l, PC_RESET, fn(*mut c_void, *mut c_void) -> i32, a as *mut c_void, core::ptr::null_mut());
                    let dev = g.device;
                    let sh = g.shot.as_mut().unwrap();
                    sh.end_present = g.presents;
                    sh.src = src_u;
                    sh.src_label = lab.clone();
                    let mut pass = Pass { list: l, key: l, order: 0, seq: 0, end: "PresentX (title src, state COMMON) -> present.rs CopyResource into the swapchain buffer".into(), draws: 0, dispatches: 0, indirect: 0,
                        clears: Vec::new(), ps: Vec::new(), targets: vec![format!("src={lab}")], caps: Vec::new(), notes: Vec::new(), writes_src: true };
                    let left = budget_left(sh) + (256 << 20);
                    match inject(dev, l, src_u, ST_COMMON, "PRESENT", left) {
                        Ok(mut c) => {
                            c.idx = sh.caps.len() as u32;
                            sh.bytes += c.total;
                            sh.caps.push(c);
                            pass.caps.push(sh.caps.len() - 1);
                        }
                        Err(e) => pass.notes.push(e),
                    }
                    sh.order += 1;
                    pass.order = sh.order;
                    sh.passes.push(pass);
                    call!(l, PC_CLOSE, fn() -> i32);
                    run_list = Some(l);
                }
            }
        }
        if !ACTIVE.load(Ordering::Relaxed) {
            if let Some(sh) = g.shot.as_mut() {
                if sh.end_present != 0 && run_list.is_none() {
                    let lists: Vec<usize> = sh.passes.iter().filter(|p| !p.caps.is_empty()).map(|p| p.list).collect();
                    let pending = lists.iter().filter(|l| !sh.submit.contains_key(l)).count();
                    sh.waits += 1;
                    if pending == 0 || sh.waits > 8 {
                        finalize = g.shot.take();
                    }
                }
            }
        }
        let seen = core::mem::take(&mut g.slow_seen);
        let ps_seen = core::mem::take(&mut g.ps_seen);
        if g.shot.is_none() && !ACTIVE.load(Ordering::Relaxed) {
            let mut why = None;
            if slow_due(SLOW_HIT.load(Ordering::Relaxed), seen, SLOW_SHOTS.load(Ordering::Relaxed), SLOW_MAX_SHOTS) {
                SLOW_HIT.store(false, Ordering::Relaxed);
                let n = SLOW_SHOTS.fetch_add(1, Ordering::Relaxed) + 1;
                let last = *SLOW_LAST.lock().unwrap_or_else(|e| e.into_inner());
                why = Some(format!("ps_ms trigger {n}/{SLOW_MAX_SHOTS}: PS {:016x} drawn in the frame just presented after a {} call", ps_ms_target().unwrap_or(0), last.map_or("?".to_string(), |(ms, f)| format!("{ms:.2} ms (gpuprof frame {f})"))));
            }
            let ps_last = *PS_LAST.lock().unwrap_or_else(|e| e.into_inner());
            if ps_draw_due(ps_seen, t, s.ps_draw, PS_SHOTS.load(Ordering::Relaxed), ps_last) {
                let n = PS_SHOTS.fetch_add(1, Ordering::Relaxed) + 1;
                *PS_LAST.lock().unwrap_or_else(|e| e.into_inner()) = t;
                why = Some(format!("ps trigger {n}/{SLOW_MAX_SHOTS}: PS {:016x} drawn in the frame just presented at t={t:.1}s (>= {:.1}s)", s.ps_draw.map_or(0, |x| x.0), s.ps_draw.map_or(0.0, |x| x.1)));
            }
            if take_due(&s.secs, &mut g.sec_idx, t) {
                why = Some(format!("t>={:.1}s after first present", s.secs[g.sec_idx - 1]));
            }
            if take_due(&s.presents, &mut g.pres_idx, g.presents) {
                why = Some(format!("present #{}", s.presents[g.pres_idx - 1]));
            }
            if s.file {
                for p in [out_root().join("now"), std::env::temp_dir().join("tombox_passcap_now")] {
                    if p.exists() {
                        let _ = std::fs::remove_file(&p);
                        why = Some(format!("trigger file {}", p.display()));
                    }
                }
            }
            if let Some(w) = why {
                g.next_id += 1;
                g.shot = Some(Shot { id: g.next_id, why: w.clone(), start_present: g.presents, end_present: 0, t, frames_left: s.frames, passes: Vec::new(), caps: Vec::new(), submit: HashMap::new(), cpu_pending: Vec::new(), cpu_submit: Vec::new(),
                    bytes: 0, waits: 0, order: 0, src: 0, src_label: String::new(), chain: String::new(), budget_note: None, ps_cache: HashMap::new(), probes: Vec::new() });
                ACTIVE.store(true, Ordering::Relaxed);
                record("d3d12_xs", "passcap", &format!("shot {} armed ({w}) at present #{} t={t:.2}s: recording the next {} frame(s)", g.next_id, g.presents, s.frames));
            }
        }
    }
    if let Some(l) = run_list {
        let lists = [l as *mut c_void];
        call!(queue, 10, fn(u32, *const *mut c_void) -> (), 1, lists.as_ptr());
    }
    if let Some(sh) = finalize {
        finish(queue as usize, sh);
    }
}

unsafe fn finish(present_queue: usize, mut sh: Shot) {
    let fence = with(|g| g.own.map(|o| o.2));
    let mut queues: Vec<usize> = sh.submit.values().map(|v| v.1).collect();
    queues.push(present_queue);
    queues.sort();
    queues.dedup();
    let mut waited = Vec::new();
    if let Some(f) = fence {
        for q in &queues {
            let v = with(|g| {
                let o = g.own.as_mut().unwrap();
                o.3 += 1;
                o.3
            });
            call!(*q, 14, fn(*mut c_void, u64) -> i32, f as *mut c_void, v);
            waited.push((*q, wait_fence(f, v, 10_000)));
        }
    }
    let ok = !waited.is_empty() && waited.iter().all(|w| w.1);
    let raw = sel().map(|s| s.raw).unwrap_or(false);
    for p in sh.passes.iter_mut() {
        if !sh.submit.contains_key(&p.list) && !p.caps.is_empty() && p.list != p.key {
            if let Some(v) = sh.submit.get(&p.key) {
                sh.submit.insert(p.list, *v);
            }
        }
    }
    let _ = std::thread::Builder::new().name("tombox-passcap".into()).spawn(move || write_shot(sh, ok, waited, raw));
}

fn write_shot(sh: Shot, gpu_done: bool, waited: Vec<(usize, bool)>, raw: bool) {
    let dir = out_root().join(format!("shot{:02}_p{}", sh.id, sh.start_present));
    let _ = std::fs::create_dir_all(&dir);
    let mut order: Vec<usize> = (0..sh.passes.len()).collect();
    let present_list = sh.passes.last().map(|p| p.list).unwrap_or(0);
    let exec = |p: &Pass| -> u64 {
        if p.list == present_list && p.end.starts_with("PresentX") {
            return u64::MAX - 1;
        }
        sh.submit.get(&p.list).map(|v| v.0).unwrap_or(u64::MAX)
    };
    order.sort_by_key(|&i| (exec(&sh.passes[i]), sh.passes[i].order));
    let mut results: Vec<Option<(Stats, String)>> = vec![None; sh.caps.len()];
    for c in &sh.caps {
        let slotname: String = c.slot.chars().filter(|ch| ch.is_ascii_alphanumeric()).collect();
        let stem = format!("{:04}_{}_{}x{}_fmt{}", c.idx, slotname, c.w, c.h, c.fmt);
        let res = if !gpu_done {
            None
        } else {
            unsafe { read_cap(c) }
        };
        let text = match (res, decode_kind(c.fmt, c.read_fmt)) {
            (Some(data), _) if c.depth > 1 => (Stats::default(), grid3d_text(&data, c.pitch, c.w, c.h, c.depth, if c.fmt == 0 { c.read_fmt } else { c.fmt })),
            (None, _) => (Stats::default(), if gpu_done { "readback Map failed".to_string() } else { "GPU fence wait timed out; not read".to_string() }),
            (Some(data), None) => {
                if raw {
                    let _ = std::fs::write(dir.join(format!("{stem}.raw")), &data);
                }
                (Stats::default(), format!("format {} (copy fmt {}) not decoded; pitch {}", c.fmt, c.read_fmt, c.pitch))
            }
            (Some(data), Some((ch, is_float, bpp, kind))) => {
                let st = stats(kind, ch, &data, c.pitch, c.w, c.h, bpp);
                let (pw, ph, rgb) = preview(kind, ch, is_float, &data, c.pitch, c.w, c.h, bpp, &st);
                let png = compat_common::gdk::user::png_rgb(pw as u32, ph as u32, &rgb);
                let _ = std::fs::write(dir.join(format!("{stem}.png")), png);
                if raw {
                    let mut packed = Vec::with_capacity(c.w * c.h * bpp);
                    for y in 0..c.h {
                        if let Some(r) = data.get(y * c.pitch..y * c.pitch + c.w * bpp) {
                            packed.extend_from_slice(r);
                        }
                    }
                    let _ = std::fs::write(dir.join(format!("{stem}.raw")), packed);
                }
                (st, format!("{kind} {} -> {stem}.png", st.line()))
            }
        };
        results[c.idx as usize] = Some(text);
        unsafe { release_cap(c) };
    }
    let mut out = String::new();
    let _ = writeln!(out, "passcap shot {} ({}) t={:.2}s: recorded presents #{}..#{} | GPU complete={gpu_done} queues waited={:x?}", sh.id, sh.why, sh.t, sh.start_present, sh.end_present, waited);
    let _ = writeln!(out, "presented src: {} -> present.rs present_frame: CopyResource(swapchain buffer, src), then overlay; the swapchain only ever shows src", sh.src_label);
    let _ = writeln!(out, "last frame chain: {}", sh.chain);
    if let Some(b) = &sh.budget_note {
        let _ = writeln!(out, "BUDGET: {b} (later passes logged without captures)");
    }
    let _ = writeln!(out, "{} passes, {} captures, {} MB; order = GPU execution order (submission index, then recording order); exec ? = list never submitted", sh.passes.len(), sh.caps.len(), sh.bytes >> 20);
    let mut first_black = None;
    let mut first_nan = None;
    let mut lit: HashMap<usize, bool> = HashMap::new();
    let mut first_dark_after_lit = None;
    for &i in &order {
        let p = &sh.passes[i];
        for &ci in &p.caps {
            let c = &sh.caps[ci];
            if let Some((st, _)) = &results[ci] {
                if st.pixels == 0 {
                    continue;
                }
                let tag = format!("cap {:04} {} {} (pass order {})", ci, c.slot, c.label, p.order);
                if st.black() && first_black.is_none() && p.draws + p.indirect > 0 {
                    first_black = Some(tag.clone());
                }
                if st.nan > 0 && first_nan.is_none() {
                    first_nan = Some(tag.clone());
                }
                let was = lit.get(&c.res).copied().unwrap_or(false);
                if was && st.black() && first_dark_after_lit.is_none() {
                    first_dark_after_lit = Some(tag.clone());
                }
                if !st.black() {
                    lit.insert(c.res, true);
                }
            }
        }
    }
    let _ = writeln!(out, "first BLACK target after a pass with draws: {}", first_black.unwrap_or_else(|| "none".into()));
    let _ = writeln!(out, "first NaN target: {}", first_nan.unwrap_or_else(|| "none".into()));
    let _ = writeln!(out, "first target that had content earlier in the frame and is now BLACK: {}", first_dark_after_lit.unwrap_or_else(|| "none".into()));
    let _ = writeln!(out);
    for &i in &order {
        let p = &sh.passes[i];
        let e = exec(p);
        let es = if e == u64::MAX { "?".to_string() } else if e == u64::MAX - 1 { "present".to_string() } else { e.to_string() };
        let ps: Vec<String> = p.ps.iter().map(|h| format!("{h:016x}")).collect();
        let _ = writeln!(out, "[exec {es} order {:04}] list={:#x}{} pass#{} end={} draws={} indirect={} dispatches={}{} ps=[{}]", p.order, p.list,
            if p.key != p.list { format!(" (origin {:#x})", p.key) } else { String::new() }, p.seq, p.end, p.draws, p.indirect, p.dispatches,
            if p.writes_src { " WRITES-PRESENTED-SRC" } else { "" }, ps.join(","));
        for t in &p.targets {
            let _ = writeln!(out, "    target {t}");
        }
        for c in &p.clears {
            let _ = writeln!(out, "    {c}");
        }
        for &ci in &p.caps {
            let c = &sh.caps[ci];
            let (st, txt) = results[ci].clone().unwrap_or_default();
            let _ = writeln!(out, "    cap {:04} {} state={:#x} ({}){} {} {}", ci, c.slot, c.state, c.state_src, if c.samples > 1 { format!(" msaa{}x->resolved(sample avg)", c.samples) } else { String::new() }, st.flags(), txt);
        }
        for n in &p.notes {
            let _ = writeln!(out, "    note {n}");
        }
    }
    let _ = std::fs::write(dir.join("passes.txt"), &out);
    if crate::va_census::on() {
        let views: Vec<(usize, u8)> = crate::bindtrace::all_views().into_iter().map(|(r, u)| (r, if u { crate::va_census::P_UAV } else { crate::va_census::P_SRV })).collect();
        let c = crate::va_census::census(&views);
        let _ = std::fs::write(dir.join("va_census.txt"), &c);
        record("d3d12_xs", "passcap", &format!("TOMBOX_VA_ALIAS_CENSUS: {}", c.lines().next().unwrap_or("")));
    }
    if !sh.probes.is_empty() {
        let mut pt = format!("passcap shot {} probe readbacks (GPU complete={gpu_done}); pass seq matches the pass#/seq of passes.txt lists
", sh.id);
        for p in &sh.probes {
            if gpu_done {
                pt.push_str(&unsafe { probe_text(p) });
            } else {
                pt.push_str(&format!("{} (GPU not complete; not read)
", p.label));
            }
            if p.rb != 0 {
                unsafe { call!(p.rb, 2, fn() -> u32) };
            }
            if p.qh != 0 {
                unsafe { call!(p.qh, 2, fn() -> u32) };
            }
        }
        for s in &sh.cpu_submit {
            pt.push_str(s);
            pt.push_str("\n");
        }
        let _ = std::fs::write(dir.join("probes.txt"), pt);
    }
    record("d3d12_xs", "passcap", &format!("shot {} written: {} passes, {} captures -> {}", sh.id, sh.passes.len(), sh.caps.len(), dir.join("passes.txt").display()));
}
