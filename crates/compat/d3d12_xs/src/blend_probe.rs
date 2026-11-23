use core::ffi::c_void;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;

use compat_common::record;

use crate::device_wrap::Guid;

const IID_QUEUE: Guid = Guid { d1: 0x0ec8_70a6, d2: 0x5d7e, d3: 0x4c22, d4: [0x8c, 0xfc, 0x5b, 0xaa, 0xe0, 0x76, 0x16, 0xed] };
const IID_ALLOCATOR: Guid = Guid { d1: 0x6102_dee4, d2: 0xaf59, d3: 0x4b09, d4: [0xb9, 0x99, 0xb4, 0x4d, 0x73, 0xf0, 0x9b, 0x24] };
const IID_LIST: Guid = Guid { d1: 0x5b16_0d0f, d2: 0xac1b, d3: 0x4185, d4: [0x8b, 0xa8, 0xb3, 0xae, 0x42, 0xa5, 0xa4, 0x55] };
const IID_RESOURCE: Guid = Guid { d1: 0x6964_42be, d2: 0xa72e, d3: 0x4059, d4: [0xbc, 0x79, 0x5b, 0x5c, 0x98, 0x04, 0x0f, 0xad] };
const IID_FENCE: Guid = Guid { d1: 0x0a75_3dcf, d2: 0xc4d8, d3: 0x4b91, d4: [0xad, 0xf6, 0xbe, 0x5a, 0x60, 0xd9, 0x5a, 0x76] };
const IID_ROOT_SIGNATURE: Guid = Guid { d1: 0xc54a_6b66, d2: 0x72df, d3: 0x4ee8, d4: [0x8b, 0xe5, 0xa9, 0x46, 0xa1, 0x42, 0x92, 0x14] };
const IID_PSO: Guid = Guid { d1: 0x765a_30f3, d2: 0xf624, d3: 0x4c6f, d4: [0xa8, 0x28, 0xac, 0xe9, 0x48, 0x62, 0x24, 0x45] };
const IID_DESC_HEAP: Guid = Guid { d1: 0x8efb_471d, d2: 0x616c, d3: 0x4f49, d4: [0x90, 0xf7, 0x12, 0x7b, 0xb7, 0x63, 0xfa, 0x51] };

const VERSION: u32 = 1;
const SIDE: u32 = 8;
const PITCH: u32 = 256;
const ST_RT: u32 = 4;
const ST_COPY_SOURCE: u32 = 0x800;
const ST_COPY_DEST: u32 = 0x400;
const FMT_RGBA16F: u32 = 10;
const FMT_R11G11B10: u32 = 26;

const RS: &str = "#define RS \"RootFlags(0), RootConstants(num32BitConstants=4, b0)\"\n";
const VS: &str = "[RootSignature(RS)]\nfloat4 main(uint id : SV_VertexID) : SV_Position { float2 p = float2((id << 1) & 2, id & 2); return float4(p * float2(2.0, -2.0) + float2(-1.0, 1.0), 0.0, 1.0); }\n";
const PS: &str = "cbuffer C : register(b0) { float4 c; };\nfloat4 main() : SV_Target { return c; }\n";

static DEVICE: AtomicUsize = AtomicUsize::new(0);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Overflow {
    Saturates,
    Inf,
}

pub fn note_device(real: *mut c_void) {
    if !real.is_null() {
        let _ = DEVICE.compare_exchange(0, real as usize, Ordering::AcqRel, Ordering::Relaxed);
    }
}

pub fn cache_line(adapter: &str, o: Overflow) -> String {
    format!("v{VERSION} {} {adapter}", match o { Overflow::Saturates => "saturates", Overflow::Inf => "inf" })
}

pub fn parse_cache(text: &str, adapter: &str) -> Option<Overflow> {
    text.lines().find_map(|l| {
        let mut it = l.splitn(3, ' ');
        let (v, o, a) = (it.next()?, it.next()?, it.next()?);
        if v != format!("v{VERSION}") || a != adapter {
            return None;
        }
        match o {
            "saturates" => Some(Overflow::Saturates),
            "inf" => Some(Overflow::Inf),
            _ => None,
        }
    })
}

pub fn classify(fmt: u32, px: &[u8]) -> Option<Overflow> {
    let r = if fmt == FMT_RGBA16F {
        u16::from_le_bytes([*px.first()?, *px.get(1)?]) as u32
    } else {
        u32::from_le_bytes(px.get(0..4)?.try_into().ok()?) & 0x7ff
    };
    match (fmt, r) {
        (FMT_RGBA16F, 0x7bff) | (FMT_R11G11B10, 0x7bf) => Some(Overflow::Saturates),
        (FMT_RGBA16F, 0x7c00) | (FMT_R11G11B10, 0x7c0) => Some(Overflow::Inf),
        _ => None,
    }
}

pub fn overflow() -> Option<Overflow> {
    static R: OnceLock<Option<Overflow>> = OnceLock::new();
    *R.get_or_init(|| {
        let dev = DEVICE.load(Ordering::Acquire);
        if dev == 0 {
            record("d3d12_xs", "blend_probe", "no device yet when the blend overflow probe was first needed; eager blendclamp kept");
            return None;
        }
        let adapter = tombox_xshader::fma_probe::probe().adapter.clone();
        let file = tombox_xshader::shader_cache::root().join("blend_probe.txt");
        let text = std::fs::read_to_string(&file).unwrap_or_default();
        if let Some(o) = parse_cache(&text, &adapter) {
            record("d3d12_xs", "blend_probe", &format!("{adapter}: blend overflow {o:?} (cached in {})", file.display()));
            return Some(o);
        }
        let t0 = std::time::Instant::now();
        match unsafe { probe(dev as *mut c_void) } {
            Ok(o) => {
                record("d3d12_xs", "blend_probe", &format!("{adapter}: 40000+40000 additive into RGBA16F and R11G11B10 gives {o:?} (probed in {:.1} ms)", t0.elapsed().as_secs_f64() * 1000.0));
                let mut keep: Vec<&str> = text.lines().filter(|l| !l.ends_with(adapter.as_str())).collect();
                let line = cache_line(&adapter, o);
                keep.push(&line);
                let _ = std::fs::create_dir_all(file.parent().unwrap_or(std::path::Path::new(".")));
                let _ = std::fs::write(&file, keep.join("\n") + "\n");
                Some(o)
            }
            Err(e) => {
                record("d3d12_xs", "blend_probe", &format!("{adapter}: blend overflow probe failed ({e}); eager blendclamp kept"));
                None
            }
        }
    })
}

unsafe fn slot(o: usize, i: usize) -> usize {
    unsafe { *(*(o as *const *const usize)).add(i) }
}

macro_rules! vc {
    ($o:expr, $i:expr, fn($($t:ty),*) -> $r:ty $(, $a:expr)*) => {{
        let o = $o as usize;
        let f: unsafe extern "system" fn(*mut c_void $(, $t)*) -> $r = core::mem::transmute(slot(o, $i));
        f(o as *mut c_void $(, $a)*)
    }};
}

struct Bag(Vec<usize>);

impl Bag {
    fn keep(&mut self, o: usize) -> usize {
        self.0.push(o);
        o
    }
}

impl Drop for Bag {
    fn drop(&mut self) {
        for &o in self.0.iter().rev() {
            if o != 0 {
                unsafe { vc!(o, 2, fn() -> u32) };
            }
        }
    }
}

fn check(hr: i32, p: *mut c_void, what: &str) -> Result<usize, String> {
    if hr < 0 || p.is_null() { Err(format!("{what} hr={hr:#010x}")) } else { Ok(p as usize) }
}

unsafe fn committed(dev: usize, heap: u32, dim: u32, w: u64, fmt: u32, flags: u32, state: u32) -> Result<usize, String> {
    let props: [u32; 5] = [heap, 0, 0, 1, 1];
    let mut d = [0u8; 56];
    let h = if dim == 1 { 1u32 } else { SIDE };
    d[0..4].copy_from_slice(&dim.to_le_bytes());
    d[0x10..0x18].copy_from_slice(&w.to_le_bytes());
    d[0x18..0x1c].copy_from_slice(&h.to_le_bytes());
    d[0x1c..0x1e].copy_from_slice(&1u16.to_le_bytes());
    d[0x1e..0x20].copy_from_slice(&1u16.to_le_bytes());
    d[0x20..0x24].copy_from_slice(&fmt.to_le_bytes());
    d[0x24..0x28].copy_from_slice(&1u32.to_le_bytes());
    d[0x2c..0x30].copy_from_slice(&(if dim == 1 { 1u32 } else { 0 }).to_le_bytes());
    d[0x30..0x34].copy_from_slice(&flags.to_le_bytes());
    let mut clear = [0u8; 24];
    clear[0..4].copy_from_slice(&fmt.to_le_bytes());
    let cv = if dim == 1 { core::ptr::null() } else { clear.as_ptr() };
    let mut r: *mut c_void = core::ptr::null_mut();
    let hr = unsafe { vc!(dev, 27, fn(*const u32, u32, *const u8, u32, *const u8, *const Guid, *mut *mut c_void) -> i32, props.as_ptr(), 0, d.as_ptr(), state, cv, &IID_RESOURCE, &mut r) };
    check(hr, r, "CreateCommittedResource")
}

pub unsafe fn probe(dev: *mut c_void) -> Result<Overflow, String> {
    if !tombox_xshader::dxc::available() {
        return Err("DXC not available".into());
    }
    let vs = tombox_xshader::dxc::compile(&format!("{RS}{VS}"), "vs_6_0").map_err(|e| format!("vs: {e}"))?;
    let ps = tombox_xshader::dxc::compile(PS, "ps_6_0").map_err(|e| format!("ps: {e}"))?;
    let dev = dev as usize;
    let mut bag = Bag(Vec::new());
    unsafe {
        let qd = [0u32; 4];
        let mut queue: *mut c_void = core::ptr::null_mut();
        let queue = bag.keep(check(vc!(dev, 8, fn(*const u32, *const Guid, *mut *mut c_void) -> i32, qd.as_ptr(), &IID_QUEUE, &mut queue), queue, "CreateCommandQueue")?);
        let mut alloc: *mut c_void = core::ptr::null_mut();
        let alloc = bag.keep(check(vc!(dev, 9, fn(u32, *const Guid, *mut *mut c_void) -> i32, 0, &IID_ALLOCATOR, &mut alloc), alloc, "CreateCommandAllocator")?);
        let mut list: *mut c_void = core::ptr::null_mut();
        let list = bag.keep(check(vc!(dev, 12, fn(u32, u32, *mut c_void, *mut c_void, *const Guid, *mut *mut c_void) -> i32, 0, 0, alloc as *mut c_void, core::ptr::null_mut(), &IID_LIST, &mut list), list, "CreateCommandList")?);
        let mut fence: *mut c_void = core::ptr::null_mut();
        let fence = bag.keep(check(vc!(dev, 36, fn(u64, u32, *const Guid, *mut *mut c_void) -> i32, 0, 0, &IID_FENCE, &mut fence), fence, "CreateFence")?);
        let mut rs: *mut c_void = core::ptr::null_mut();
        let rs = bag.keep(check(vc!(dev, 16, fn(u32, *const u8, usize, *const Guid, *mut *mut c_void) -> i32, 0, vs.as_ptr(), vs.len(), &IID_ROOT_SIGNATURE, &mut rs), rs, "CreateRootSignature")?);
        let hd = [2u32, 2, 0, 0];
        let mut heap: *mut c_void = core::ptr::null_mut();
        let heap = bag.keep(check(vc!(dev, 14, fn(*const u32, *const Guid, *mut *mut c_void) -> i32, hd.as_ptr(), &IID_DESC_HEAP, &mut heap), heap, "CreateDescriptorHeap")?);
        let mut base = 0usize;
        vc!(heap, 9, fn(*mut usize) -> *mut usize, &mut base);
        let inc = vc!(dev, 15, fn(u32) -> u32, 2) as usize;
        let fmts = [FMT_RGBA16F, FMT_R11G11B10];
        let mut targets = Vec::new();
        for (i, &fmt) in fmts.iter().enumerate() {
            let rt = bag.keep(committed(dev, 1, 3, SIDE as u64, fmt, 1, ST_RT)?);
            let rb = bag.keep(committed(dev, 3, 1, (PITCH * SIDE) as u64, 0, 0, ST_COPY_DEST)?);
            let mut d = vec![0u8; 0x290];
            d[0..8].copy_from_slice(&(rs as u64).to_le_bytes());
            d[0x08..0x10].copy_from_slice(&(vs.as_ptr() as u64).to_le_bytes());
            d[0x10..0x18].copy_from_slice(&(vs.len() as u64).to_le_bytes());
            d[0x18..0x20].copy_from_slice(&(ps.as_ptr() as u64).to_le_bytes());
            d[0x20..0x28].copy_from_slice(&(ps.len() as u64).to_le_bytes());
            for (o, v) in [(0x80usize, 1u32), (0x88, 2), (0x8c, 2), (0x90, 1), (0x94, 2), (0x98, 2), (0x9c, 1), (0x1c0, u32::MAX), (0x1c4, 3), (0x1c8, 1), (0x1dc, 1), (0x23c, 3), (0x240, 1), (0x244, fmt), (0x268, 1)] {
                d[o..o + 4].copy_from_slice(&v.to_le_bytes());
            }
            d[0xa4] = 0xf;
            let mut pso: *mut c_void = core::ptr::null_mut();
            let pso = bag.keep(check(vc!(dev, 10, fn(*const u8, *const Guid, *mut *mut c_void) -> i32, d.as_ptr(), &IID_PSO, &mut pso), pso, "CreateGraphicsPipelineState")?);
            let rtv = base + i * inc;
            vc!(dev, 20, fn(*mut c_void, *const c_void, usize) -> (), rt as *mut c_void, core::ptr::null(), rtv);
            targets.push((fmt, rt, rb, pso, rtv));
        }
        let l = list;
        let barrier = |res: usize, a: u32, b: u32| {
            let mut e = [0u8; 32];
            e[8..16].copy_from_slice(&(res as u64).to_le_bytes());
            e[16..20].copy_from_slice(&u32::MAX.to_le_bytes());
            e[20..24].copy_from_slice(&a.to_le_bytes());
            e[24..28].copy_from_slice(&b.to_le_bytes());
            vc!(l, 26, fn(u32, *const u8) -> (), 1, e.as_ptr());
        };
        let vp = [0f32, 0.0, SIDE as f32, SIDE as f32, 0.0, 1.0];
        let sc = [0i32, 0, SIDE as i32, SIDE as i32];
        let c = [40000.0f32, 40000.0, 40000.0, 1.0];
        for &(fmt, rt, rb, pso, rtv) in &targets {
            vc!(l, 48, fn(usize, *const f32, u32, *const c_void) -> (), rtv, [0f32; 4].as_ptr(), 0, core::ptr::null());
            vc!(l, 46, fn(u32, *const usize, i32, *const usize) -> (), 1, &rtv, 0, core::ptr::null());
            vc!(l, 21, fn(u32, *const f32) -> (), 1, vp.as_ptr());
            vc!(l, 22, fn(u32, *const i32) -> (), 1, sc.as_ptr());
            vc!(l, 30, fn(*mut c_void) -> (), rs as *mut c_void);
            vc!(l, 25, fn(*mut c_void) -> (), pso as *mut c_void);
            vc!(l, 20, fn(u32) -> (), 4);
            vc!(l, 36, fn(u32, u32, *const f32, u32) -> (), 0, 4, c.as_ptr(), 0);
            vc!(l, 12, fn(u32, u32, u32, u32) -> (), 3, 1, 0, 0);
            vc!(l, 12, fn(u32, u32, u32, u32) -> (), 3, 1, 0, 0);
            barrier(rt, ST_RT, ST_COPY_SOURCE);
            let mut dst = [0u8; 48];
            dst[0..8].copy_from_slice(&(rb as u64).to_le_bytes());
            dst[8..12].copy_from_slice(&1u32.to_le_bytes());
            for (o, v) in [(24usize, fmt), (28, SIDE), (32, SIDE), (36, 1), (40, PITCH)] {
                dst[o..o + 4].copy_from_slice(&v.to_le_bytes());
            }
            let mut src = [0u8; 48];
            src[0..8].copy_from_slice(&(rt as u64).to_le_bytes());
            vc!(l, 16, fn(*const u8, u32, u32, u32, *const u8, *const c_void) -> (), dst.as_ptr(), 0, 0, 0, src.as_ptr(), core::ptr::null());
            barrier(rt, ST_COPY_SOURCE, ST_RT);
        }
        let hr = vc!(l, 9, fn() -> i32);
        if hr < 0 {
            return Err(format!("Close hr={hr:#010x}"));
        }
        let lists = [list];
        vc!(queue, 10, fn(u32, *const usize) -> (), 1, lists.as_ptr());
        if vc!(queue, 14, fn(*mut c_void, u64) -> i32, fence as *mut c_void, 1) < 0 {
            return Err("Signal failed".into());
        }
        let t0 = std::time::Instant::now();
        while vc!(fence, 8, fn() -> u64) < 1 {
            if t0.elapsed().as_millis() > 2000 {
                core::mem::forget(bag);
                return Err("GPU did not finish within 2 s (objects leaked rather than freed in flight)".into());
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        let mut verdicts = Vec::new();
        for &(fmt, _, rb, _, _) in &targets {
            let mut m: *mut u8 = core::ptr::null_mut();
            if vc!(rb, 8, fn(u32, *const c_void, *mut *mut u8) -> i32, 0, core::ptr::null(), &mut m) < 0 || m.is_null() {
                return Err("readback Map failed".into());
            }
            let bpp = if fmt == FMT_RGBA16F { 8 } else { 4 };
            let off = (SIDE / 2 * PITCH + SIDE / 2 * bpp) as usize;
            let px = core::slice::from_raw_parts(m.add(off), bpp as usize).to_vec();
            vc!(rb, 9, fn(u32, *const c_void) -> (), 0, core::ptr::null());
            match classify(fmt, &px) {
                Some(o) => verdicts.push(o),
                None => return Err(format!("fmt {fmt}: unexpected result {px:02x?}")),
            }
        }
        Ok(if verdicts.iter().all(|&o| o == Overflow::Saturates) { Overflow::Saturates } else { Overflow::Inf })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_lines_round_trip_per_adapter_and_version() {
        let a = "NVIDIA X [10de:2d05] driver 32.0.16.1742";
        let text = format!("{}\n{}\n", cache_line("other", Overflow::Inf), cache_line(a, Overflow::Saturates));
        assert_eq!(parse_cache(&text, a), Some(Overflow::Saturates));
        assert_eq!(parse_cache(&text, "other"), Some(Overflow::Inf));
        assert_eq!(parse_cache(&text, "NVIDIA X [10de:2d05] driver 32.0.16.1800"), None);
        assert_eq!(parse_cache(&format!("v0 saturates {a}"), a), None);
    }

    #[test]
    fn results_classify_by_format() {
        assert_eq!(classify(10, &[0xff, 0x7b, 0, 0, 0, 0, 0, 0]), Some(Overflow::Saturates));
        assert_eq!(classify(10, &[0x00, 0x7c, 0, 0, 0, 0, 0, 0]), Some(Overflow::Inf));
        assert_eq!(classify(26, &0x7bfu32.to_le_bytes()), Some(Overflow::Saturates));
        assert_eq!(classify(26, &0x7c0u32.to_le_bytes()), Some(Overflow::Inf));
        assert_eq!(classify(26, &0x3c0u32.to_le_bytes()), None);
    }
}
