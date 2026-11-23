#![cfg(windows)]
#![allow(dead_code)]

#[path = "../../tools/xreplay/src/d3d.rs"]
mod d3d;
#[path = "support/hw_lock.rs"]
mod hw_lock;

use core::ffi::c_void;
use d3d::{Gpu, Guid};
use tombox_xshader::dxc;

extern "system" {
    fn LoadLibraryA(n: *const u8) -> *mut c_void;
    fn GetProcAddress(h: *mut c_void, n: *const u8) -> *mut c_void;
}

const VS: &str = "struct VOut { float4 p : SV_Position; float3 t0 : TEXCOORD0; float t2 : TEXCOORD2; float3 t1 : TEXCOORD1; float4 t3 : TEXCOORD3; float3 t4 : TEXCOORD4; float4 t5 : TEXCOORD5; };\nVOut main(uint id : SV_VertexID, uint inst : SV_InstanceID) { VOut o; float2 uv = float2((id << 1) & 2, id & 2); o.p = float4(uv * float2(2, -2) + float2(-1, 1), 0.5, 1); o.t0 = float3(uv, 1); o.t2 = 0.25 + inst; o.t1 = float3(uv.yx, 0.5); o.t3 = float4(uv, 0.5, 1); o.t4 = float3(0.3, 0.6, 0.9); o.t5 = float4(uv, 1, 1); return o; }";

const IID_QUERY_HEAP: Guid = Guid(0x0d9658ae, 0xed45, 0x469e, [0xa6, 0x1d, 0x97, 0x0e, 0xc5, 0x83, 0xca, 0xb4]);

unsafe fn warp_gpu() -> Gpu {
    const IID_FACTORY4: Guid = Guid(0x1bc6ea02, 0xef36, 0x464f, [0xbf, 0x0c, 0x21, 0xca, 0x39, 0xe5, 0x16, 0x8a]);
    const IID_ADAPTER: Guid = Guid(0x2411e7e1, 0x12ac, 0x4ccf, [0xbd, 0x14, 0x97, 0x98, 0xe8, 0x53, 0x4d, 0xc0]);
    let dxgi = LoadLibraryA(b"dxgi.dll\0".as_ptr());
    let cf: unsafe extern "system" fn(*const Guid, *mut *mut c_void) -> i32 = core::mem::transmute(GetProcAddress(dxgi, b"CreateDXGIFactory1\0".as_ptr()));
    let mut f: *mut c_void = core::ptr::null_mut();
    assert!(cf(&IID_FACTORY4, &mut f) >= 0, "CreateDXGIFactory1");
    let mut ad: *mut c_void = core::ptr::null_mut();
    assert!(vcall!(f, 27, fn(*const Guid, *mut *mut c_void) -> i32, &IID_ADAPTER, &mut ad) >= 0, "EnumWarpAdapter");
    let d3d = LoadLibraryA(b"d3d12.dll\0".as_ptr());
    let create: unsafe extern "system" fn(*mut c_void, u32, *const Guid, *mut *mut c_void) -> i32 = core::mem::transmute(GetProcAddress(d3d, b"D3D12CreateDevice\0".as_ptr()));
    let mut device: *mut c_void = core::ptr::null_mut();
    assert!(create(ad, 0xc100, &d3d::IID_DEVICE, &mut device) >= 0, "D3D12CreateDevice(WARP)");
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

unsafe fn root_sig(gpu: &Gpu, cbv: bool) -> Result<*mut c_void, String> {
    let mut params = [0u8; 64];
    params[0..4].copy_from_slice(&1u32.to_le_bytes());
    params[8..12].copy_from_slice(&1u32.to_le_bytes());
    params[12..16].copy_from_slice(&0u32.to_le_bytes());
    params[16..20].copy_from_slice(&5u32.to_le_bytes());
    params[24..28].copy_from_slice(&5u32.to_le_bytes());
    params[32..36].copy_from_slice(&(if cbv { 2u32 } else { 3u32 }).to_le_bytes());
    params[40..44].copy_from_slice(&13u32.to_le_bytes());
    params[44..48].copy_from_slice(&(if cbv { 0u32 } else { tombox_xshader::cbv_raw_hlsl::map_space(0)? }).to_le_bytes());
    params[56..60].copy_from_slice(&5u32.to_le_bytes());
    let mut desc = [0u8; 40];
    desc[0..4].copy_from_slice(&2u32.to_le_bytes());
    desc[8..16].copy_from_slice(&(params.as_ptr() as u64).to_le_bytes());
    desc[32..36].copy_from_slice(&0u32.to_le_bytes());
    let d3d = LoadLibraryA(b"d3d12.dll\0".as_ptr());
    let ser: unsafe extern "system" fn(*const u8, u32, *mut *mut c_void, *mut *mut c_void) -> i32 = core::mem::transmute(GetProcAddress(d3d, b"D3D12SerializeRootSignature\0".as_ptr()));
    let mut blob: *mut c_void = core::ptr::null_mut();
    let mut err: *mut c_void = core::ptr::null_mut();
    if ser(desc.as_ptr(), 1, &mut blob, &mut err) < 0 {
        return Err("serialize root signature".into());
    }
    let p = vcall!(blob, 3, fn() -> *const u8);
    let n = vcall!(blob, 4, fn() -> usize);
    let mut rs: *mut c_void = core::ptr::null_mut();
    let hr = vcall!(gpu.device, 16, fn(u32, *const u8, usize, *const Guid, *mut *mut c_void) -> i32, 0, p, n, &d3d::IID_ROOTSIG, &mut rs);
    if hr < 0 {
        return Err(format!("CreateRootSignature {hr:#x}{}", gpu.messages()));
    }
    Ok(rs)
}

fn rt_size() -> u32 {
    std::env::var("TOMBOX_F861_SIZE").ok().and_then(|v| v.parse().ok()).filter(|&n: &u32| (16..=4096).contains(&n)).unwrap_or(64)
}

fn tex(fmt: u32, samples: u32, flags: u32) -> [u8; 0x38] {
    let mut t = [0u8; 0x38];
    t[0..4].copy_from_slice(&3u32.to_le_bytes());
    t[0x10..0x18].copy_from_slice(&(rt_size() as u64).to_le_bytes());
    t[0x18..0x1c].copy_from_slice(&rt_size().to_le_bytes());
    t[0x1c..0x1e].copy_from_slice(&1u16.to_le_bytes());
    t[0x1e..0x20].copy_from_slice(&1u16.to_le_bytes());
    t[0x20..0x24].copy_from_slice(&fmt.to_le_bytes());
    t[0x24..0x28].copy_from_slice(&samples.to_le_bytes());
    t[0x30..0x34].copy_from_slice(&flags.to_le_bytes());
    t
}

pub fn ps_hlsl(src: &str, raw: bool) -> Result<String, String> {
    let h = if raw { tombox_xshader::cbv_raw_hlsl::rewrite_hlsl(src)? } else { src.to_string() };
    Ok(tombox_xshader::runtime_hlsl::finalize(&h, &[2, 0, 0, 0, 0, 0, 0, 0], true))
}

unsafe fn rs_from_blob(gpu: &Gpu, blob: &[u8]) -> Result<*mut c_void, String> {
    let mut rs: *mut c_void = core::ptr::null_mut();
    let hr = vcall!(gpu.device, 16, fn(u32, *const u8, usize, *const Guid, *mut *mut c_void) -> i32, 0, blob.as_ptr(), blob.len(), &d3d::IID_ROOTSIG, &mut rs);
    if hr < 0 {
        return Err(format!("CreateRootSignature(blob) {hr:#x}{}", gpu.messages()));
    }
    Ok(rs)
}

unsafe fn run_once(gpu: &mut Gpu, ps: &[u8], cbv: bool, b13: &[u8], samples: u32) -> Result<(), String> {
    let full = std::env::var("TOMBOX_F861_RS").ok().map(|p| std::fs::read(p).expect("rs blob"));
    let rs = match &full {
        Some(b) => rs_from_blob(gpu, b)?,
        None => root_sig(gpu, cbv)?,
    };
    let rc_vals: Vec<u32> = std::env::var("TOMBOX_F861_RC").ok().map(|v| v.split(',').filter_map(|x| u32::from_str_radix(x.trim().trim_start_matches("0x"), 16).ok()).collect()).unwrap_or_default();
    let real_vs = std::env::var("TOMBOX_F861_VS").ok().filter(|_| full.is_some());
    let vs = match &real_vs {
        Some(p) => {
            let src = std::fs::read_to_string(p).map_err(|e| format!("vs hlsl {e}"))?;
            let h = tombox_xshader::runtime_hlsl::finalize(&tombox_xshader::cbv_raw_hlsl::rewrite_hlsl(&src)?, &[0; 8], true);
            dxc::compile(&h, "vs_6_6")?
        }
        None => dxc::compile(VS, "vs_6_0")?,
    };
    let names: [&[u8]; 3] = [b"POSITION\0", b"NORMAL\0", b"TEXCOORD\0"];
    let mut il = [0u8; 96];
    for (i, (fmt, off)) in [(2u32, 0u32), (6, 16), (2, 28)].iter().enumerate() {
        let e = &mut il[i * 32..i * 32 + 32];
        e[0..8].copy_from_slice(&(names[i].as_ptr() as u64).to_le_bytes());
        e[12..16].copy_from_slice(&fmt.to_le_bytes());
        e[20..24].copy_from_slice(&off.to_le_bytes());
    }
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
    put32(&mut d, 0x1f0, 1);
    put32(&mut d, 0x1f4, 1);
    put32(&mut d, 0x1f8, 7);
    for f in [0x204usize, 0x214] {
        put32(&mut d, f, 1);
        put32(&mut d, f + 4, 1);
        put32(&mut d, f + 8, 1);
        put32(&mut d, f + 12, 8);
    }
    if real_vs.is_some() {
        put64(&mut d, 0x228, il.as_ptr() as u64);
        put32(&mut d, 0x230, 3);
    }
    put32(&mut d, 0x23c, 3);
    put32(&mut d, 0x240, 1);
    put32(&mut d, 0x244, 26);
    put32(&mut d, 0x264, 20);
    put32(&mut d, 0x268, samples);
    let mut pso: *mut c_void = core::ptr::null_mut();
    let exact = std::env::var("TOMBOX_F861_EXACT").ok().filter(|_| full.is_some());
    let mut exact_keep: Vec<Vec<u8>> = Vec::new();
    let mut stride = 44u32;
    if let Some(dir) = &exact {
        let rdf = |n: &str| std::fs::read(format!("{dir}_{n}")).map_err(|e| format!("exact {n}: {e}"));
        let ed = rdf("desc.bin")?;
        let evs = rdf("vs.dxil")?;
        let eps = rdf("ps.dxil")?;
        let mut lay = rdf("layout.bin")?;
        let names: [&[u8]; 3] = [b"POSITION\0", b"NORMAL\0", b"TEXCOORD\0"];
        let fsize = |f: u32| match f { 2 | 3 | 4 => 16, 6 | 7 | 8 => 12, 16 | 17 | 18 => 8, _ => 4 };
        stride = 0;
        for (i, e) in lay.chunks_mut(32).enumerate() {
            e[0..8].copy_from_slice(&(names[i.min(2)].as_ptr() as u64).to_le_bytes());
            let f = u32::from_le_bytes(e[12..16].try_into().unwrap());
            let off = u32::from_le_bytes(e[20..24].try_into().unwrap());
            stride = stride.max(off + fsize(f));
        }
        d.copy_from_slice(&ed[..656]);
        put64(&mut d, 0, rs as u64);
        put64(&mut d, 8, evs.as_ptr() as u64);
        put64(&mut d, 16, evs.len() as u64);
        put64(&mut d, 24, eps.as_ptr() as u64);
        put64(&mut d, 32, eps.len() as u64);
        put64(&mut d, 0x228, lay.as_ptr() as u64);
        d[0x278..0x288].fill(0);
        if std::env::var_os("TOMBOX_F861_CULLNONE").is_some() {
            d[0x1c8..0x1cc].copy_from_slice(&1u32.to_le_bytes());
            println!("diagnostic: cull forced to NONE (desc no longer byte-exact)");
        }
        if std::env::var_os("TOMBOX_F861_DEPTHOFF").is_some() {
            d[0x1f0..0x1f4].copy_from_slice(&0u32.to_le_bytes());
            println!("diagnostic: depth test off (desc no longer byte-exact)");
        }
        println!("exact: desc/vs/ps/layout from {dir}_* (vs {}B ps {}B stride {stride} samples {})", evs.len(), eps.len(), u32::from_le_bytes(d[0x268..0x26c].try_into().unwrap()));
        exact_keep.extend([evs, eps, lay]);
    }
    let hr = vcall!(gpu.device, 10, fn(*const u8, *const Guid, *mut *mut c_void) -> i32, d.as_ptr(), &d3d::IID_PSO, &mut pso);
    if hr < 0 {
        return Err(format!("CreateGraphicsPipelineState hr={:#010x}{}", hr as u32, gpu.messages()));
    }
    let rt = gpu.committed(&tex(26, samples, 1), d3d::default_heap(), 4)?;
    let srvrt = std::env::var_os("TOMBOX_F861_SRVRT").is_some() && full.is_some();
    let ds = gpu.committed(&tex(if srvrt { 19 } else { 20 }, samples, 2), d3d::default_heap(), 0x10)?;
    let (_rh, rtv, _, _) = gpu.heap(2, 1, false)?;
    let (_dh, dsv, _, _) = gpu.heap(3, 1, false)?;
    vcall!(gpu.device, 20, fn(*mut c_void, *const c_void, usize) -> (), rt, core::ptr::null(), rtv);
    let mut dv = [0u32; 6];
    dv[0] = 20;
    dv[1] = if samples > 1 { 5 } else { 3 };
    vcall!(gpu.device, 21, fn(*mut c_void, *const c_void, usize) -> (), ds, dv.as_ptr() as *const c_void, dsv);
    let ub = gpu.buffer(0x10000, 2, d3d::GENERIC_READ, false)?;
    let off = if cbv { 0x1000usize } else { 0xfd0usize };
    let mut img = vec![0u8; 0x10000];
    img[off..off + b13.len()].copy_from_slice(b13);
    if !Gpu::map_write(ub, &img) {
        return Err("map upload".into());
    }
    let va = Gpu::gpu_va(ub) + off as u64;
    gpu.begin();
    let l = gpu.list;
    let clear = [0f32; 4];
    vcall!(l, 48, fn(usize, *const f32, u32, *const c_void) -> (), rtv, clear.as_ptr(), 0, core::ptr::null());
    vcall!(l, 47, fn(usize, u32, f32, u8, u32, *const c_void) -> (), dsv, 3, 0.0, 0, 0, core::ptr::null());
    vcall!(l, 46, fn(u32, *const usize, i32, *const usize) -> (), 1, &rtv, 0, &dsv);
    vcall!(l, 30, fn(*mut c_void) -> (), rs);
    vcall!(l, 25, fn(*mut c_void) -> (), pso);
    if let Ok(s) = std::env::var("TOMBOX_F861_BIAS") {
        let v: Vec<f32> = s.split(',').map(|t| t.trim().parse::<f32>().unwrap_or(f32::NAN)).collect();
        let (b, c, sl) = (v.first().copied().unwrap_or(0.0), v.get(1).copied().unwrap_or(0.0), v.get(2).copied().unwrap_or(0.0));
        vcall!(l, 82, fn(f32, f32, f32) -> (), b, c, sl);
        println!("RSSetDepthBias({b}, {c}, {sl})");
    }
    let mut rc = [0u32; 5];
    for (i, v) in rc_vals.iter().take(5).enumerate() {
        rc[i] = *v;
    }
    let mut keep: Vec<*mut c_void> = Vec::new();
    let mut indexed = false;
    if full.is_some() {
        let (sh, scpu, sg, sinc) = gpu.heap(0, 256, true)?;
        if srvrt {
            let view = |res: *mut c_void, fmt: u32, slot: usize| {
                let mut v = [0u32; 10];
                v[0] = fmt;
                v[1] = if samples > 1 { 6 } else { 4 };
                v[2] = 0x1688;
                if samples == 1 {
                    v[5] = 1;
                }
                vcall!(gpu.device, 18, fn(*mut c_void, *const c_void, usize) -> (), res, v.as_ptr() as *const c_void, scpu + slot * sinc);
            };
            for slot in 0..62usize {
                if slot == 5 {
                    view(ds, 21, slot);
                } else {
                    view(rt, 26, slot);
                }
            }
        }
        let reserved = std::env::var_os("TOMBOX_F861_RESERVED").is_some();
        let (mh, mcpu, mg, minc) = gpu.heap(1, 16, true)?;
        if reserved {
            let mut sd = [0u32; 13];
            sd[0] = 0x15;
            sd[1] = 1;
            sd[2] = 1;
            sd[3] = 1;
            sd[5] = 16;
            sd[6] = 1;
            sd[12] = f32::MAX.to_bits();
            for k in 0..16usize {
                vcall!(gpu.device, 22, fn(*const c_void, usize) -> (), sd.as_ptr() as *const c_void, mcpu + k * minc);
            }
            let mk = |dim: u32, w: u64, h: u32, depth: u16, mips: u16, layout: u32| {
                let mut t = [0u8; 0x38];
                t[0..4].copy_from_slice(&dim.to_le_bytes());
                t[0x10..0x18].copy_from_slice(&w.to_le_bytes());
                t[0x18..0x1c].copy_from_slice(&h.to_le_bytes());
                t[0x1c..0x1e].copy_from_slice(&depth.to_le_bytes());
                t[0x1e..0x20].copy_from_slice(&mips.to_le_bytes());
                t[0x20..0x24].copy_from_slice(&28u32.to_le_bytes());
                t[0x24..0x28].copy_from_slice(&1u32.to_le_bytes());
                t[0x2c..0x30].copy_from_slice(&layout.to_le_bytes());
                t
            };
            let reserve = |t: &[u8; 0x38]| -> Result<*mut c_void, String> {
                let mut r: *mut c_void = core::ptr::null_mut();
                let hr = vcall!(gpu.device, 30, fn(*const u8, u32, *const c_void, *const Guid, *mut *mut c_void) -> i32, t.as_ptr(), 0xc0, core::ptr::null(), &d3d::IID_RESOURCE, &mut r);
                if hr < 0 || r.is_null() {
                    return Err(format!("CreateReservedResource hr={:#010x}{}", hr as u32, gpu.messages()));
                }
                Ok(r)
            };
            let r2d = reserve(&mk(3, 1024, 1024, 1, 11, 2))?;
            let r2a = reserve(&mk(3, 512, 512, 8, 10, 2))?;
            let r3d = reserve(&mk(4, 64, 64, 64, 7, 2))?;
            let c1a = gpu.committed(&mk(2, 256, 1, 4, 1, 0), d3d::default_heap(), 0xc0)?;
            let srv = |res: *mut c_void, dim: u32, slot: usize| {
                let mut v = [0u32; 10];
                v[0] = 28;
                v[1] = dim;
                v[2] = 0x1688;
                v[5] = u32::MAX;
                match dim {
                    5 => v[7] = 8,
                    3 => v[7] = 4,
                    _ => {}
                }
                if dim == 8 {
                    v[0] = 30;
                }
                vcall!(gpu.device, 18, fn(*mut c_void, *const c_void, usize) -> (), res, v.as_ptr() as *const c_void, scpu + slot * sinc);
            };
            for slot in 0..62usize {
                srv(r2d, 4, slot);
            }
            srv(r2a, 5, 0);
            srv(c1a, 3, 3);
            let _ = r3d;
            for slot in 0..6usize {
                srv(r2d, 4, 64 + slot);
            }
            keep.extend([r2d, r2a, r3d, c1a]);
            println!("reserved: 2D 1024x1024x11 / 2DArray 512x512x8 / 3D 64^3 created with no tiles mapped, bound in table 8 slots 0..61 and table 4 slots 0..5");
        }
        let heaps = [sh, mh];
        vcall!(l, 28, fn(u32, *const *mut c_void) -> (), 2, heaps.as_ptr());
        let hexes = |k: &str, n: usize| -> Vec<u32> {
            let mut v: Vec<u32> = std::env::var(k).ok().map(|t| t.split(|c: char| c == ',' || c == ' ').filter_map(|x| u32::from_str_radix(x.trim().trim_start_matches("0x"), 16).ok()).collect()).unwrap_or_default();
            v.resize(n, 0);
            v
        };
        let r0 = hexes("TOMBOX_F861_RC0", 21);
        vcall!(l, 36, fn(u32, u32, *const u32, u32) -> (), 0, 21, r0.as_ptr(), 0);
        let r1 = hexes("TOMBOX_F861_RC1", 16);
        vcall!(l, 36, fn(u32, u32, *const u32, u32) -> (), 1, 16, r1.as_ptr(), 0);
        vcall!(l, 36, fn(u32, u32, *const u32, u32) -> (), 2, 5, rc.as_ptr(), 0);
        vcall!(l, 32, fn(u32, u64) -> (), 3, mg);
        vcall!(l, 32, fn(u32, u64) -> (), 4, if reserved { sg + 64 * sinc as u64 } else { sg });
        let vs_va = match std::env::var("TOMBOX_F861_VSB13").ok().filter(|_| real_vs.is_some() || exact.is_some()) {
            Some(p) => {
                let vb13 = std::fs::read(p).map_err(|e| format!("vs b13 {e}"))?;
                let vub = gpu.buffer(0x10000, 2, d3d::GENERIC_READ, false)?;
                let mut vimg = vec![0u8; 0x10000];
                vimg[0xce0..0xce0 + vb13.len()].copy_from_slice(&vb13);
                if !Gpu::map_write(vub, &vimg) {
                    return Err("map vs upload".into());
                }
                keep.push(vub);
                Gpu::gpu_va(vub) + 0xce0
            }
            None => va,
        };
        if real_vs.is_some() || exact.is_some() {
            let n = 4096u32;
            let mut shift = [0f32; 3];
            if let Some(p) = std::env::var("TOMBOX_F861_VB").ok().filter(|_| exact.is_some() && std::env::var_os("TOMBOX_F861_DIST").is_some()) {
                let raw = std::fs::read(&p).map_err(|e| format!("real vb {e}"))?;
                let cnt = raw.len() / stride.max(1) as usize;
                let f = |o: usize| f32::from_le_bytes(raw[o..o + 4].try_into().unwrap());
                let mut c = [0f32; 3];
                for v in 0..cnt {
                    for k in 0..3 {
                        c[k] += f(v * stride as usize + k * 4) / cnt.max(1) as f32;
                    }
                }
                let dist: f32 = std::env::var("TOMBOX_F861_DIST").ok().and_then(|v| v.parse().ok()).unwrap_or(30.0);
                let fwd = [-0.53999406f32, -0.1305262, -0.8314861];
                let cam = [914.01276f32, 18.913355, 883.2228];
                for k in 0..3 {
                    shift[k] = cam[k] + fwd[k] * dist - c[k];
                }
                println!("instance stand-in: mesh centroid {c:?} moved {dist} units in front of the captured camera (translation {shift:?})");
            }
            let idx = gpu.buffer(n as u64 * 16, 2, d3d::GENERIC_READ, false)?;
            let mut rows = vec![0u8; n as usize * 16 * 3];
            for k in 0..n as usize {
                for r in 0..3usize {
                    let o = (k * 3 + r) * 16 + r * 4;
                    rows[o..o + 4].copy_from_slice(&1f32.to_le_bytes());
                    let w = (k * 3 + r) * 16 + 12;
                    rows[w..w + 4].copy_from_slice(&shift[r].to_le_bytes());
                }
            }
            let mat = gpu.buffer(rows.len() as u64, 2, d3d::GENERIC_READ, false)?;
            if !Gpu::map_write(idx, &vec![0u8; n as usize * 16]) || !Gpu::map_write(mat, &rows) {
                return Err("map vs buffers".into());
            }
            let bview = |res: *mut c_void, fmt: u32, count: u32, slot: usize| {
                let mut v = [0u32; 10];
                v[0] = fmt;
                v[1] = 1;
                v[2] = 0x1688;
                v[6] = count;
                vcall!(gpu.device, 18, fn(*mut c_void, *const c_void, usize) -> (), res, v.as_ptr() as *const c_void, scpu + slot * sinc);
            };
            bview(idx, 3, n, 128 + 9);
            bview(mat, 2, n * 3, 128 + 10);
            let fwd = [-0.53999406f32, -0.1305262, -0.8314861];
            let cam = [914.01276f32, 18.913355, 883.2228];
            let c: Vec<f32> = (0..3).map(|k| cam[k] + fwd[k] * 50.0).collect();
            let a = [1.2961135f32, 0.0, -0.8417382];
            let b = [-0.19532247f32, 2.723972, -0.30075872];
            let mut vb = Vec::new();
            let tri = if exact.is_some() { [(-200f32, -200f32), (-200., 600.), (600., -200.)] } else { [(-200f32, -200f32), (600., -200.), (-200., 600.)] };
            for (sa, sb) in tri {
                for k in 0..3 {
                    vb.extend_from_slice(&(c[k] + a[k] * sa / 10.0 + b[k] * sb / 10.0).to_le_bytes());
                }
                if exact.is_some() {
                    vb.extend_from_slice(&((0x1ffu32 << 10) | (3 << 30)).to_le_bytes());
                    vb.extend_from_slice(&0.5f32.to_le_bytes());
                    vb.extend_from_slice(&0.5f32.to_le_bytes());
                    while vb.len() % stride as usize != 0 {
                        vb.push(0);
                    }
                } else {
                    vb.extend_from_slice(&1f32.to_le_bytes());
                    for f in [0f32, 1., 0., 0.5, 0.5, 0.5, 1.] {
                        vb.extend_from_slice(&f.to_le_bytes());
                    }
                }
            }
            if let Some(p) = std::env::var("TOMBOX_F861_VB").ok().filter(|_| exact.is_some()) {
                vb = std::fs::read(&p).map_err(|e| format!("real vb {e}"))?;
                println!("real vb: {} bytes ({} vertices at stride {stride})", vb.len(), vb.len() / stride.max(1) as usize);
            }
            if let Some(p) = std::env::var("TOMBOX_F861_IB").ok().filter(|_| exact.is_some()) {
                let ibd = std::fs::read(&p).map_err(|e| format!("real ib {e}"))?;
                let fmt: u32 = std::env::var("TOMBOX_F861_IBFMT").ok().and_then(|v| v.parse().ok()).unwrap_or(57);
                let ibuf = gpu.buffer(ibd.len() as u64, 2, d3d::GENERIC_READ, false)?;
                if !Gpu::map_write(ibuf, &ibd) {
                    return Err("map ib".into());
                }
                let iv = [Gpu::gpu_va(ibuf), (ibd.len() as u64) | ((fmt as u64) << 32)];
                vcall!(l, 43, fn(*const u64) -> (), iv.as_ptr());
                indexed = true;
                println!("real ib: {} bytes fmt {fmt}", ibd.len());
                keep.push(ibuf);
            }
            let vbuf = gpu.buffer(vb.len() as u64, 2, d3d::GENERIC_READ, false)?;
            if !Gpu::map_write(vbuf, &vb) {
                return Err("map vb".into());
            }
            let view = [Gpu::gpu_va(vbuf), (vb.len() as u64) | ((stride as u64) << 32)];
            vcall!(l, 44, fn(u32, u32, *const u64) -> (), 0, 1, view.as_ptr());
            keep.extend([idx, mat, vbuf]);
        }
        vcall!(l, 40, fn(u32, u64) -> (), 5, vs_va);
        vcall!(l, 40, fn(u32, u64) -> (), 6, va);
        vcall!(l, 32, fn(u32, u64) -> (), 7, if real_vs.is_some() || exact.is_some() { sg + 128 * sinc as u64 } else { sg });
        vcall!(l, 32, fn(u32, u64) -> (), 8, sg);
        let ua = gpu.buffer(0x10000, 1, 8, true)?;
        vcall!(l, 42, fn(u32, u64) -> (), 9, Gpu::gpu_va(ua));
        keep.extend([sh, mh, ua]);
    } else {
        vcall!(l, 36, fn(u32, u32, *const u32, u32) -> (), 0, 5, rc.as_ptr(), 0);
        vcall!(l, if cbv { 38 } else { 40 }, fn(u32, u64) -> (), 1, va);
    }
    let sz = rt_size();
    let vp = [0f32, 0., sz as f32, sz as f32, 0., 1.];
    vcall!(l, 21, fn(u32, *const f32) -> (), 1, vp.as_ptr());
    let sc = [0i32, 0, sz as i32, sz as i32];
    vcall!(l, 22, fn(u32, *const i32) -> (), 1, sc.as_ptr());
    vcall!(l, 20, fn(u32) -> (), 4);
    let qd = [2u32, 1, 0];
    let mut qh: *mut c_void = core::ptr::null_mut();
    let qhr = vcall!(gpu.device, 39, fn(*const u32, *const Guid, *mut *mut c_void) -> i32, qd.as_ptr(), &IID_QUERY_HEAP, &mut qh);
    let qrb = gpu.buffer(256, 3, d3d::COPY_DEST, false)?;
    if qhr >= 0 {
        vcall!(l, 52, fn(*mut c_void, u32, u32) -> (), qh, 3, 0);
    }
    if indexed {
        let a: Vec<i64> = std::env::var("TOMBOX_F861_ARGS").unwrap_or_else(|_| "312,2,0,0,0".into()).split(',').filter_map(|x| x.trim().parse().ok()).collect();
        let g = |i: usize, d: i64| a.get(i).copied().unwrap_or(d);
        println!("DrawIndexedInstanced(idx={} inst={} startIdx={} baseVtx={} startInst={})", g(0, 312), g(1, 2), g(2, 0), g(3, 0), g(4, 0));
        vcall!(l, 13, fn(u32, u32, u32, i32, u32) -> (), g(0, 312) as u32, g(1, 2) as u32, g(2, 0) as u32, g(3, 0) as i32, g(4, 0) as u32);
    } else {
        vcall!(l, 12, fn(u32, u32, u32, u32) -> (), 3, 2, 0, 0);
    }
    if qhr >= 0 {
        vcall!(l, 53, fn(*mut c_void, u32, u32) -> (), qh, 3, 0);
        vcall!(l, 54, fn(*mut c_void, u32, u32, u32, *mut c_void, u64) -> (), qh, 3, 0, 1, qrb, 0);
    }
    let r = gpu.submit();
    if r.is_ok() && qhr >= 0 {
        if let Some(q) = Gpu::map_read(qrb, 88) {
            let st: Vec<u64> = q.chunks(8).map(|c| u64::from_le_bytes(c.try_into().unwrap())).collect();
            println!("stats vs={} cinv={} cprim={} ps={}", st[2], st[5], st[6], st[7]);
        }
        d3d::release(qh);
    }
    d3d::release(pso);
    drop(keep);
    r
}

#[test]
#[ignore]
fn f861_one_draw_repro() {
    hw_lock::require_free_gpu();
    let src = std::fs::read_to_string(std::env::var("TOMBOX_F861_HLSL").expect("TOMBOX_F861_HLSL")).expect("hlsl");
    let b13 = std::fs::read(std::env::var("TOMBOX_F861_B13").expect("TOMBOX_F861_B13")).expect("b13");
    let modes: Vec<String> = std::env::var("TOMBOX_F861_MODES").unwrap_or_else(|_| "raw,cbv".into()).split(',').map(|s| s.trim().to_string()).collect();
    let samples: u32 = std::env::var("TOMBOX_F861_SAMPLES").ok().and_then(|s| s.parse().ok()).unwrap_or(2);
    unsafe {
        let mut gpu = if std::env::var_os("TOMBOX_EXEC_HW").is_some() { Gpu::new(std::env::var_os("TOMBOX_EXEC_DEBUG").is_some()).expect("d3d12 device") } else { warp_gpu() };
        gpu.open = false;
        for m in modes {
            let raw = m == "raw";
            let h = ps_hlsl(&src, raw).expect("ps hlsl");
            let ps = dxc::compile(&h, "ps_6_6").expect("ps compile");
            let t = std::time::Instant::now();
            let r = run_once(&mut gpu, &ps, !raw, &b13, samples);
            println!("mode {m} samples {samples} hw {}: {:?} in {:.1} ms", std::env::var_os("TOMBOX_EXEC_HW").is_some(), r, t.elapsed().as_secs_f64() * 1000.0);
            if r.is_err() {
                println!("removed reason {:#x}", vcall!(gpu.device, 37, fn() -> i32) as u32);
                break;
            }
        }
    }
}
