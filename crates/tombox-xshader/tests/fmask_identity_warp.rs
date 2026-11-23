#![cfg(windows)]
#![allow(dead_code)]

#[path = "../../tools/xreplay/src/d3d.rs"]
mod d3d;

use core::ffi::c_void;
use d3d::{Gpu, Guid};
use tombox_xshader::{dxc, extract, hlsl};

extern "system" {
    fn LoadLibraryA(n: *const u8) -> *mut c_void;
    fn GetProcAddress(h: *mut c_void, n: *const u8) -> *mut c_void;
}

const W: u32 = 4;
const VS: &str = "float4 main(uint id : SV_VertexID) : SV_Position { float2 uv = float2((id << 1) & 2, id & 2); return float4(uv * float2(2, -2) + float2(-1, 1), 0.5, 1); }";
const FILL: &str = "cbuffer C : register(b0) { float4 a; float4 b; }; float4 main(float4 p : SV_Position, uint s : SV_SampleIndex) : SV_Target { return s == 0 ? a : b; }";

unsafe fn warp() -> Gpu {
    const IID_FACTORY4: Guid = Guid(0x1bc6ea02, 0xef36, 0x464f, [0xbf, 0x0c, 0x21, 0xca, 0x39, 0xe5, 0x16, 0x8a]);
    const IID_ADAPTER: Guid = Guid(0x2411e7e1, 0x12ac, 0x4ccf, [0xbd, 0x14, 0x97, 0x98, 0xe8, 0x53, 0x4d, 0xc0]);
    let dxgi = LoadLibraryA(b"dxgi.dll\0".as_ptr());
    let cf: unsafe extern "system" fn(*const Guid, *mut *mut c_void) -> i32 = core::mem::transmute(GetProcAddress(dxgi, b"CreateDXGIFactory1\0".as_ptr()));
    let mut f: *mut c_void = core::ptr::null_mut();
    assert!(cf(&IID_FACTORY4, &mut f) >= 0);
    let mut ad: *mut c_void = core::ptr::null_mut();
    assert!(vcall!(f, 27, fn(*const Guid, *mut *mut c_void) -> i32, &IID_ADAPTER, &mut ad) >= 0);
    let d3d = LoadLibraryA(b"d3d12.dll\0".as_ptr());
    let create: unsafe extern "system" fn(*mut c_void, u32, *const Guid, *mut *mut c_void) -> i32 = core::mem::transmute(GetProcAddress(d3d, b"D3D12CreateDevice\0".as_ptr()));
    let mut device: *mut c_void = core::ptr::null_mut();
    assert!(create(ad, 0xc100, &d3d::IID_DEVICE, &mut device) >= 0);
    let qd: [u32; 4] = [0, 0, 0, 0];
    let mut queue: *mut c_void = core::ptr::null_mut();
    vcall!(device, 8, fn(*const u32, *const Guid, *mut *mut c_void) -> i32, qd.as_ptr(), &d3d::IID_QUEUE, &mut queue);
    let mut alloc: *mut c_void = core::ptr::null_mut();
    vcall!(device, 9, fn(u32, *const Guid, *mut *mut c_void) -> i32, 0, &d3d::IID_ALLOCATOR, &mut alloc);
    let mut list: *mut c_void = core::ptr::null_mut();
    vcall!(device, 12, fn(u32, u32, *mut c_void, *mut c_void, *const Guid, *mut *mut c_void) -> i32, 0, 0, alloc, core::ptr::null_mut(), &d3d::IID_GFX_LIST, &mut list);
    vcall!(list, 9, fn() -> i32);
    let mut fence: *mut c_void = core::ptr::null_mut();
    vcall!(device, 36, fn(u64, u32, *const Guid, *mut *mut c_void) -> i32, 0, 0, &d3d::IID_FENCE, &mut fence);
    Gpu { device, queue, alloc, list, fence, value: 0, info: core::ptr::null_mut(), open: false }
}

fn tex(w: u32, fmt: u32, samples: u32) -> [u8; 0x38] {
    let mut t = [0u8; 0x38];
    t[0..4].copy_from_slice(&3u32.to_le_bytes());
    t[0x10..0x18].copy_from_slice(&(w as u64).to_le_bytes());
    t[0x18..0x1c].copy_from_slice(&w.to_le_bytes());
    t[0x1c..0x1e].copy_from_slice(&1u16.to_le_bytes());
    t[0x1e..0x20].copy_from_slice(&1u16.to_le_bytes());
    t[0x20..0x24].copy_from_slice(&fmt.to_le_bytes());
    t[0x24..0x28].copy_from_slice(&samples.to_le_bytes());
    t[0x30..0x34].copy_from_slice(&1u32.to_le_bytes());
    t
}

unsafe fn root(gpu: &Gpu, table: bool) -> *mut c_void {
    let range: [u32; 5] = [0, 3, 0, 0, 0];
    let mut params = [0u8; 64];
    params[0..4].copy_from_slice(&2u32.to_le_bytes());
    params[8..12].copy_from_slice(&(if table { 1u32 } else { 0u32 }).to_le_bytes());
    params[24..28].copy_from_slice(&5u32.to_le_bytes());
    params[32..36].copy_from_slice(&0u32.to_le_bytes());
    params[40..44].copy_from_slice(&1u32.to_le_bytes());
    params[48..56].copy_from_slice(&(range.as_ptr() as u64).to_le_bytes());
    params[56..60].copy_from_slice(&5u32.to_le_bytes());
    let mut samp = [0u32; 13];
    samp[0] = 0x15;
    samp[1] = 3;
    samp[2] = 3;
    samp[3] = 3;
    samp[5] = 1;
    samp[6] = 1;
    samp[9] = f32::MAX.to_bits();
    samp[12] = 5;
    let mut desc = [0u8; 40];
    desc[0..4].copy_from_slice(&(if table { 2u32 } else { 1u32 }).to_le_bytes());
    desc[8..16].copy_from_slice(&(params.as_ptr() as u64).to_le_bytes());
    if table {
        desc[16..20].copy_from_slice(&1u32.to_le_bytes());
        desc[24..32].copy_from_slice(&(samp.as_ptr() as u64).to_le_bytes());
    }
    let d3d = LoadLibraryA(b"d3d12.dll\0".as_ptr());
    let ser: unsafe extern "system" fn(*const u8, u32, *mut *mut c_void, *mut *mut c_void) -> i32 = core::mem::transmute(GetProcAddress(d3d, b"D3D12SerializeRootSignature\0".as_ptr()));
    let mut blob: *mut c_void = core::ptr::null_mut();
    let mut err: *mut c_void = core::ptr::null_mut();
    assert!(ser(desc.as_ptr(), 1, &mut blob, &mut err) >= 0, "serialize root signature");
    let p = vcall!(blob, 3, fn() -> *const u8);
    let n = vcall!(blob, 4, fn() -> usize);
    let mut rs: *mut c_void = core::ptr::null_mut();
    assert!(vcall!(gpu.device, 16, fn(u32, *const u8, usize, *const Guid, *mut *mut c_void) -> i32, 0, p, n, &d3d::IID_ROOTSIG, &mut rs) >= 0);
    rs
}

unsafe fn pso(gpu: &Gpu, rs: *mut c_void, vs: &[u8], ps: &[u8], fmt: u32, samples: u32) -> *mut c_void {
    let mut d = vec![0u8; 656];
    let put32 = |d: &mut Vec<u8>, o: usize, v: u32| d[o..o + 4].copy_from_slice(&v.to_le_bytes());
    let put64 = |d: &mut Vec<u8>, o: usize, v: u64| d[o..o + 8].copy_from_slice(&v.to_le_bytes());
    put64(&mut d, 0, rs as u64);
    put64(&mut d, 8, vs.as_ptr() as u64);
    put64(&mut d, 16, vs.len() as u64);
    put64(&mut d, 24, ps.as_ptr() as u64);
    put64(&mut d, 32, ps.len() as u64);
    for i in 0..8 {
        let o = 0x80 + i * 40;
        put32(&mut d, o + 8, 2);
        put32(&mut d, o + 12, 1);
        put32(&mut d, o + 16, 1);
        put32(&mut d, o + 20, 2);
        put32(&mut d, o + 24, 1);
        put32(&mut d, o + 28, 1);
        put32(&mut d, o + 32, 4);
        d[o + 36] = 0xf;
    }
    put32(&mut d, 0x1c0, 0xffff_ffff);
    put32(&mut d, 0x1c4, 3);
    put32(&mut d, 0x1c8, 1);
    put32(&mut d, 0x1dc, 1);
    put32(&mut d, 0x1f8, 8);
    for f in [0x204usize, 0x214] {
        put32(&mut d, f, 1);
        put32(&mut d, f + 4, 1);
        put32(&mut d, f + 8, 1);
        put32(&mut d, f + 12, 8);
    }
    put32(&mut d, 0x23c, 3);
    put32(&mut d, 0x240, 1);
    put32(&mut d, 0x244, fmt);
    put32(&mut d, 0x268, samples);
    let mut p: *mut c_void = core::ptr::null_mut();
    let hr = vcall!(gpu.device, 10, fn(*const u8, *const Guid, *mut *mut c_void) -> i32, d.as_ptr(), &d3d::IID_PSO, &mut p);
    assert!(hr >= 0, "CreateGraphicsPipelineState {hr:#x} fmt={fmt} samples={samples} ps={}", ps.len());
    p
}

unsafe fn viewport(l: *mut c_void) {
    let vp = [0f32, 0.0, W as f32, W as f32, 0.0, 1.0];
    vcall!(l, 21, fn(u32, *const f32) -> (), 1, vp.as_ptr());
    let sc = [0i32, 0, W as i32, W as i32];
    vcall!(l, 22, fn(u32, *const i32) -> (), 1, sc.as_ptr());
    vcall!(l, 20, fn(u32) -> (), 4);
}

struct Rig {
    gpu: Gpu,
    vs: Vec<u8>,
    fill_rs: *mut c_void,
    fill_pso: *mut c_void,
    rs: *mut c_void,
    resolve_pso: *mut c_void,
}

unsafe fn resolve(r: &mut Rig, a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    let g = &mut r.gpu;
    let src = g.committed(&tex(W, 2, 2), d3d::default_heap(), 4).unwrap();
    let mul = g.committed(&tex(2, 2, 1), d3d::default_heap(), 4).unwrap();
    let out = g.committed(&tex(W, 2, 1), d3d::default_heap(), 4).unwrap();
    let (_rh, rtv, _, rinc) = g.heap(2, 3, false).unwrap();
    for (i, res) in [src, mul, out].iter().enumerate() {
        vcall!(g.device, 20, fn(*mut c_void, *const c_void, usize) -> (), *res, core::ptr::null(), rtv + i * rinc);
    }
    let (sh, scpu, sgpu, sinc) = g.heap(0, 3, true).unwrap();
    let srv = |res: *mut c_void, dim: u32, map: u32, slot: usize| {
        let mut v = [0u32; 10];
        v[0] = 2;
        v[1] = dim;
        v[2] = map;
        if dim == 4 {
            v[5] = 1;
        }
        vcall!(g.device, 18, fn(*mut c_void, *const c_void, usize) -> (), res, v.as_ptr() as *const c_void, scpu + slot * sinc);
    };
    srv(src, 6, 0x1688, 0);
    srv(src, 6, 0x1920, 1);
    srv(mul, 4, 0x1688, 2);
    let cb = g.buffer(0x1000, 2, d3d::GENERIC_READ, false).unwrap();
    let mut img = vec![0u8; 0x1000];
    for (i, v) in a.iter().chain(b.iter()).enumerate() {
        img[i * 4..i * 4 + 4].copy_from_slice(&v.to_bits().to_le_bytes());
    }
    img[0x100..0x104].copy_from_slice(&2u32.to_le_bytes());
    let words = [1.0f32, 0.5, 0.5, 0.15, 0.5, 0.1, 0.2, 0.02, 0.3, 1.0, 1.0];
    for (w, v) in words.iter().enumerate() {
        img[0x104 + w * 4..0x108 + w * 4].copy_from_slice(&v.to_bits().to_le_bytes());
    }
    assert!(Gpu::map_write(cb, &img));
    let va = Gpu::gpu_va(cb);
    let rb = g.buffer(0x10000, 3, d3d::COPY_DEST, false).unwrap();
    g.begin();
    let l = g.list;
    let one = [1f32; 4];
    vcall!(l, 48, fn(usize, *const f32, u32, *const c_void) -> (), rtv + rinc, one.as_ptr(), 0, core::ptr::null());
    vcall!(l, 46, fn(u32, *const usize, i32, *const usize) -> (), 1, &rtv, 0, core::ptr::null());
    vcall!(l, 30, fn(*mut c_void) -> (), r.fill_rs);
    vcall!(l, 25, fn(*mut c_void) -> (), r.fill_pso);
    vcall!(l, 38, fn(u32, u64) -> (), 0, va);
    viewport(l);
    vcall!(l, 12, fn(u32, u32, u32, u32) -> (), 3, 1, 0, 0);
    g.barrier(src, 4, 0x80);
    g.barrier(mul, 4, 0x80);
    let orv = rtv + 2 * rinc;
    vcall!(l, 46, fn(u32, *const usize, i32, *const usize) -> (), 1, &orv, 0, core::ptr::null());
    let heaps = [sh];
    vcall!(l, 28, fn(u32, *const *mut c_void) -> (), 1, heaps.as_ptr());
    vcall!(l, 30, fn(*mut c_void) -> (), r.rs);
    vcall!(l, 25, fn(*mut c_void) -> (), r.resolve_pso);
    vcall!(l, 38, fn(u32, u64) -> (), 0, va + 0x100);
    vcall!(l, 32, fn(u32, u64) -> (), 1, sgpu);
    viewport(l);
    vcall!(l, 12, fn(u32, u32, u32, u32) -> (), 3, 1, 0, 0);
    g.barrier(out, 4, 0x800);
    let mut dst = [0u8; 48];
    dst[0..8].copy_from_slice(&(rb as u64).to_le_bytes());
    dst[8..12].copy_from_slice(&1u32.to_le_bytes());
    dst[24..28].copy_from_slice(&2u32.to_le_bytes());
    dst[28..32].copy_from_slice(&W.to_le_bytes());
    dst[32..36].copy_from_slice(&W.to_le_bytes());
    dst[36..40].copy_from_slice(&1u32.to_le_bytes());
    dst[40..44].copy_from_slice(&256u32.to_le_bytes());
    let mut srcl = [0u8; 48];
    srcl[0..8].copy_from_slice(&(out as u64).to_le_bytes());
    vcall!(l, 16, fn(*const u8, u32, u32, u32, *const u8, *const c_void) -> (), dst.as_ptr(), 0, 0, 0, srcl.as_ptr(), core::ptr::null());
    g.submit().unwrap();
    let bytes = Gpu::map_read(rb, 256 * W as usize).unwrap();
    let px = |x: usize, y: usize| -> [f32; 4] { core::array::from_fn(|k| f32::from_le_bytes(bytes[y * 256 + x * 16 + k * 4..][..4].try_into().unwrap())) };
    let p0 = px(1, 1);
    for y in 0..W as usize {
        for x in 0..W as usize {
            assert_eq!(px(x, y).map(f32::to_bits), p0.map(f32::to_bits), "pixel {x},{y} differs from 1,1");
        }
    }
    for o in [src, mul, out, cb, rb, sh] {
        d3d::release(o);
    }
    p0
}

#[test]
fn trackmania_msaa_resolve_averages_both_samples_through_identity_fmask_on_warp() {
    std::env::set_var("TOMBOX_XS_MSAA_DIM", "1");
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../input/shader_cache/dump/GbxGame/0ff9ad9a34cd4e65_0200000000000000.xbox");
    let Ok(blob) = std::fs::read(&p) else {
        eprintln!("skipped: {} missing", p.display());
        return;
    };
    let t = hlsl::translate(&extract(&blob).unwrap()).unwrap();
    assert!(t.hlsl.contains("Texture2DMS<float4> tex0") && t.hlsl.contains("0x88888810u"));
    unsafe {
        let gpu = warp();
        let vs = dxc::compile(VS, "vs_6_0").unwrap();
        let fill = dxc::compile(FILL, "ps_6_0").unwrap();
        let src = match std::env::var("TOMBOX_FMASK_WARP_PROBE").ok() {
            Some(m) => t.hlsl.replacen(&m, &format!("{{ o.o0 = float4((float)v6, asfloat(v8), asfloat(v7), asfloat(v1)); return o; }}
{m}"), 1),
            None => t.hlsl.clone(),
        };
        let ps = dxc::compile(&src, &dxc::profile_for(&t.profile)).unwrap();
        let fill_rs = root(&gpu, false);
        let rs = root(&gpu, true);
        let fill_pso = pso(&gpu, fill_rs, &vs, &fill, 2, 2);
        let resolve_pso = pso(&gpu, rs, &vs, &ps, 2, 1);
        let mut r = Rig { gpu, vs, fill_rs, fill_pso, rs, resolve_pso };
        let a = [0.8f32, 0.1, 0.3, 1.0];
        let b = [0.2f32, 0.6, 0.05, 1.0];
        let ab = resolve(&mut r, a, b);
        let ba = resolve(&mut r, b, a);
        let aa = resolve(&mut r, a, a);
        let bb = resolve(&mut r, b, b);
        eprintln!("ab={ab:?} ba={ba:?} aa={aa:?} bb={bb:?}");
        for c in 0..3 {
            assert!(ab[c].is_finite() && ab[c] > 0.0, "channel {c} of the resolve is {}", ab[c]);
            assert!((ab[c] - ba[c]).abs() <= 1e-5 * ab[c].abs().max(1.0), "channel {c}: sample order changes the result ({} vs {})", ab[c], ba[c]);
            let (lo, hi) = if aa[c] < bb[c] { (aa[c], bb[c]) } else { (bb[c], aa[c]) };
            assert!(hi - lo > 1e-4, "channel {c}: a-only and b-only resolves are indistinguishable");
            assert!(ab[c] > lo && ab[c] < hi, "channel {c}: {} is not between the one-colour resolves {lo} and {hi}, so both samples were not averaged", ab[c]);
        }
    }
}
