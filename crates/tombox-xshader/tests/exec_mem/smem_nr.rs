use super::*;

const BYTES: u64 = 4096;

fn data(i: u32) -> u32 {
    0xc0de_0000 | i
}

fn root() -> RootSig {
    let d = |kind, register| RootParam { visibility: 0, kind: RootParamKind::Descriptor { kind, register, space: 0 } };
    RootSig { version: 2, flags: 0, params: vec![d(4, 0), d(2, 0)], num_static_samplers: 0, static_samplers: Vec::new() }
}

fn rfl(sdst: u32, val: u32) -> Vec<u32> {
    vec![vop1(1, 5, LIT), val, vop1(2, sdst, v(5))]
}

struct C {
    name: &'static str,
    setup: Vec<u32>,
    load: [u32; 2],
    n: u32,
    want: Vec<u32>,
}

fn prog(c: &C) -> Vec<u32> {
    let mut w = vec![sop2(0x0e, 3, 1, LIT), 0xff, sop1(3, 2, 0)];
    w.extend_from_slice(&smem(1, 8, 2, 0x00, SNULL));
    w.extend_from_slice(&smem(1, 10, 2, 0x08, SNULL));
    w.push(0xbf8c_c07f);
    w.extend_from_slice(&[sop1(3, 20, 8), sop2(0x10, 21, 9, LIT), 0x4_0000, sop1(3, 22, ic(-1)), sop1(3, 23, 0), sop1(3, 24, 10)]);
    w.extend_from_slice(&c.setup);
    w.extend_from_slice(&c.load);
    w.push(0xbf8c_c07f);
    for k in 0..c.n {
        w.extend_from_slice(&[vop1(1, 2, 40 + k), vop1(1, 1, LIT), 4 * k]);
        w.extend_from_slice(&[0xe000_0000 | (0x1c << 18) | (1 << 12), 1 | (2 << 8) | ((20 / 4) << 16) | (0x80 << 24)]);
    }
    w.push(S_ENDPGM);
    w
}

fn extract(words: &[u32]) -> ShaderExtract {
    let mut ex = shader_root(words, root());
    ex.stage = Stage::Compute;
    ex.isa_preamble = [0, 1, 1, 1];
    ex.output_sig = vec![];
    ex.has_output_sig = false;
    ex.has_input_sig = false;
    ex
}

unsafe fn ser(params: &[[u32; 8]]) -> Result<Vec<u8>, String> {
    let d3d = LoadLibraryA(b"d3d12.dll\0".as_ptr());
    let f: unsafe extern "system" fn(*const u8, u32, *mut *mut c_void, *mut *mut c_void) -> i32 = core::mem::transmute(GetProcAddress(d3d, b"D3D12SerializeRootSignature\0".as_ptr()));
    let ps: Vec<u8> = params.iter().flat_map(|p| p.iter().flat_map(|w| w.to_le_bytes())).collect();
    let mut desc = [0u8; 40];
    desc[0..4].copy_from_slice(&(params.len() as u32).to_le_bytes());
    desc[8..16].copy_from_slice(&(ps.as_ptr() as u64).to_le_bytes());
    let mut blob: *mut c_void = core::ptr::null_mut();
    let mut err: *mut c_void = core::ptr::null_mut();
    if f(desc.as_ptr(), 1, &mut blob, &mut err) < 0 {
        return Err("serialize root signature".into());
    }
    let p = vcall!(blob, 3, fn() -> *const u8);
    let n = vcall!(blob, 4, fn() -> usize);
    Ok(core::slice::from_raw_parts(p, n).to_vec())
}

unsafe fn exec(c: &mut Ctx, code: &[u8], raw: bool) -> Result<Vec<u32>, String> {
    let g = &c.gpu;
    let second = if raw { [3, 0, 0, tombox_xshader::cbv_raw_hlsl::map_space(0)?, 0, 0, 0, 0] } else { [2, 0, 0, 0, 0, 0, 0, 0] };
    let blob = ser(&[[4, 0, 0, 0, 0, 0, 0, 0], second])?;
    let mut rs: *mut c_void = core::ptr::null_mut();
    let hr = vcall!(g.device, 16, fn(u32, *const u8, usize, *const Guid, *mut *mut c_void) -> i32, 0, blob.as_ptr(), blob.len(), &d3d::IID_ROOTSIG, &mut rs);
    if hr < 0 {
        return Err(format!("CreateRootSignature {hr:#x}"));
    }
    let mut d = [0u8; 56];
    d[0..8].copy_from_slice(&(rs as u64).to_le_bytes());
    d[8..16].copy_from_slice(&(code.as_ptr() as u64).to_le_bytes());
    d[16..24].copy_from_slice(&(code.len() as u64).to_le_bytes());
    let mut pso: *mut c_void = core::ptr::null_mut();
    let hr = vcall!(g.device, 11, fn(*const u8, *const Guid, *mut *mut c_void) -> i32, d.as_ptr(), &d3d::IID_PSO, &mut pso);
    if hr < 0 {
        d3d::release(rs);
        return Err(format!("CreateComputePipelineState hr={:#010x}{}", hr as u32, g.messages()));
    }
    let ubuf = g.buffer(BYTES, 1, d3d::COPY_DEST, true)?;
    let cbuf = g.buffer(BYTES, 1, d3d::COPY_DEST, false)?;
    let up = g.buffer(2 * BYTES, 2, d3d::GENERIC_READ, false)?;
    let rb = g.buffer(BYTES, 3, d3d::COPY_DEST, false)?;
    let mut bytes = vec![0xeeu8; BYTES as usize];
    for i in 0..(BYTES / 4) as u32 {
        bytes.extend_from_slice(&data(i).to_le_bytes());
    }
    Gpu::map_write(up, &bytes);
    c.gpu.begin();
    let l = c.gpu.list;
    vcall!(l, 15, fn(*mut c_void, u64, *mut c_void, u64, u64) -> (), ubuf, 0, up, 0, BYTES);
    vcall!(l, 15, fn(*mut c_void, u64, *mut c_void, u64, u64) -> (), cbuf, 0, up, BYTES, BYTES);
    c.gpu.barrier(ubuf, d3d::COPY_DEST, 8);
    c.gpu.barrier(cbuf, d3d::COPY_DEST, if raw { 0x40 } else { 0x1 });
    vcall!(l, 29, fn(*mut c_void) -> (), rs);
    vcall!(l, 41, fn(u32, u64) -> (), 0, Gpu::gpu_va(ubuf));
    vcall!(l, if raw { 39 } else { 37 }, fn(u32, u64) -> (), 1, Gpu::gpu_va(cbuf));
    vcall!(l, 25, fn(*mut c_void) -> (), pso);
    vcall!(l, 14, fn(u32, u32, u32) -> (), 1, 1, 1);
    c.gpu.barrier(ubuf, 8, d3d::COPY_SOURCE);
    vcall!(l, 15, fn(*mut c_void, u64, *mut c_void, u64, u64) -> (), rb, 0, ubuf, 0, BYTES);
    let r = c.gpu.submit();
    let out = r.and_then(|_| Gpu::map_read(rb, BYTES as usize).ok_or_else(|| "map".to_string())).map(|b| b.chunks(4).map(|x| u32::from_le_bytes(x.try_into().unwrap())).collect());
    for k in [pso, rs, ubuf, cbuf, up, rb] {
        d3d::release(k);
    }
    out
}

fn cases() -> Vec<C> {
    let d = |r: std::ops::Range<u32>| r.map(data).collect::<Vec<u32>>();
    let cat = |a: Vec<u32>, z: usize| a.into_iter().chain(std::iter::repeat(0).take(z)).collect::<Vec<u32>>();
    let mut v = Vec::new();
    v.push(C { name: "static NR=0x20 stride 0, dwordx8 at 0x10", setup: vec![sop2(0x10, 25, 11, LIT), 0, sop1(3, 26, LIT), 0x20], load: smem(0x0b, 40, 24, 0x10, SNULL), n: 8, want: cat(d(4..8), 4) });
    v.push(C { name: "static NR=2 stride 16, dwordx8 at 0x10", setup: vec![sop1(3, 25, LIT), 0x10_0000, sop1(3, 26, ic(2))], load: smem(0x0b, 40, 24, 0x10, SNULL), n: 8, want: cat(d(4..8), 4) });
    v.push(C { name: "NR=-1 static reads unaffected", setup: vec![sop2(0x10, 25, 11, LIT), 0x10_0000, sop1(3, 26, ic(-1))], load: smem(0x0a, 40, 24, 0x6f0, SNULL), n: 4, want: d(0x1bc..0x1c0) });
    let mut s = rfl(30, 0x16);
    s.extend_from_slice(&[sop2(0x10, 25, 11, LIT), 0, sop1(3, 26, LIT), 0x20]);
    v.push(C { name: "dynamic soff 0x16 imm 3 rounds to 0x14 and crosses NR=0x20", setup: s, load: smem(0x0a, 40, 24, 3, 30), n: 4, want: cat(d(5..8), 1) });
    let mut s = rfl(26, 0x1c);
    s.extend_from_slice(&[sop1(3, 25, ic(0))]);
    v.push(C { name: "run-time NR=0x1c stride 0", setup: s, load: smem(0x0a, 40, 24, 0x10, SNULL), n: 4, want: cat(d(4..7), 1) });
    let mut s = rfl(26, 1);
    s.extend_from_slice(&[sop1(3, 25, LIT), 0x10_0000]);
    v.push(C { name: "run-time NR=1 stride 16", setup: s, load: smem(0x0a, 40, 24, 0x8, SNULL), n: 4, want: cat(d(2..4), 2) });
    let mut s = rfl(30, 0xffff_fff0);
    s.extend_from_slice(&[sop2(0x10, 25, 11, LIT), 0, sop1(3, 26, ic(-1))]);
    v.push(C { name: "soff 0xfffffff0 + imm 0x20 is past NR=-1 (stride 0), not wrapped to 0x10", setup: s, load: smem(0x09, 40, 24, 0x20, 30), n: 2, want: vec![0, 0] });
    let mut s = rfl(30, 0xffff_fff0);
    s.extend_from_slice(&[sop2(0x10, 25, 11, LIT), 0x10_0000, sop1(3, 26, ic(-1))]);
    v.push(C { name: "soff 0xfffffff0 + imm 0x20 inside NR*16 but past 4 GiB is not read wrapped", setup: s, load: smem(0x09, 40, 24, 0x20, 30), n: 2, want: vec![0, 0] });
    let mut s = rfl(30, 0x30);
    s.extend_from_slice(&[sop2(0x10, 25, 11, LIT), 0x10_0000, sop1(3, 26, ic(-1))]);
    v.push(C { name: "NR=-1 dynamic soff in range", setup: s, load: smem(0x09, 40, 24, 0x4, 30), n: 2, want: d(13..15) });
    let mut s = rfl(30, 0x40);
    s.extend_from_slice(&rfl(26, 0x48));
    s.extend_from_slice(&[sop1(3, 25, ic(0))]);
    v.push(C { name: "run-time NR and dynamic soff", setup: s, load: smem(0x0a, 40, 24, 0, 30), n: 4, want: cat(d(16..18), 2) });
    v
}

#[test]
fn smem_bounds_follow_num_records_on_warp() {
    let m = CTX.get_or_init(|| Mutex::new(unsafe { ctx_new() }));
    let mut fails = Vec::new();
    let mut runs = 0;
    for c in cases() {
        let t = translate(&extract(&prog(&c))).unwrap_or_else(|e| panic!("{}: translate: {e}", c.name));
        for raw in [false, true] {
            let h = if raw { tombox_xshader::cbv_raw_hlsl::rewrite_hlsl(&t.hlsl).unwrap_or_else(|e| panic!("{}: rewrite: {e}", c.name)) } else { t.hlsl.clone() };
            if raw {
                assert!(h.contains("ByteAddressBuffer tb_cbr"), "{}: no raw rewrite\n{h}", c.name);
            }
            let code = dxc::compile(&h, &dxc::profile_for(t.profile)).unwrap_or_else(|e| panic!("{} raw={raw}: dxc: {e}\n{h}", c.name));
            let mut g = m.lock().unwrap_or_else(|e| e.into_inner());
            let out = unsafe { exec(&mut g, &code, raw) }.unwrap_or_else(|e| panic!("{} raw={raw}: {e}\n{h}", c.name));
            drop(g);
            runs += 1;
            if out[..c.want.len()] != c.want[..] {
                fails.push(format!("{} raw={raw}: got {:08x?} want {:08x?}\n{}", c.name, &out[..c.want.len()], c.want, h.lines().filter(|l| l.contains("cb") || l.contains("tb_smem")).collect::<Vec<_>>().join("\n")));
            }
        }
    }
    println!("smem NR bounds: {runs} WARP runs, {} failing", fails.len());
    assert!(fails.is_empty(), "{}", fails.join("\n\n"));
}

#[test]
fn smem_bounds_emission() {
    let cs = cases();
    let hl = |i: usize| translate(&extract(&prog(&cs[i]))).unwrap_or_else(|e| panic!("{e}")).hlsl;
    let h0 = hl(0);
    assert!(h0.contains("s44 = 0u;") && h0.contains("s47 = 0u;") && h0.contains("s43 = cb0[1].w;"), "static OOB dwords fold to 0u\n{h0}");
    assert!(!h0.contains("tb_smem_in"), "{h0}");
    let h2 = hl(2);
    assert!(h2.contains("s40 = cb0[111].x;") && !h2.contains("tb_smem"), "NR=-1 static reads stay plain\n{h2}");
    let h3 = hl(3);
    assert!(h3.contains("& 0xfffffffcu) + 0x0u") && h3.contains("tb_smem_le(") && !h3.contains("4095u"), "{h3}");
    let h4 = hl(4);
    assert!(h4.contains("tb_smem_in(s25, s26, 0x10u, 0u)"), "{h4}");
    let lims = |i: usize| translate(&extract(&prog(&cs[i]))).unwrap().cb_limits;
    assert_eq!(lims(0).get(&0), Some(&0x20));
    assert_eq!(lims(1).get(&0), Some(&32));
    assert_eq!(lims(2).get(&0), Some(&(u32::MAX as u64 * 16)));
    assert!(lims(4).is_empty(), "run-time NR has no static limit");
    let raw = tombox_xshader::cbv_raw_hlsl::rewrite_hlsl(&h3).unwrap();
    assert!(raw.contains(tombox_xshader::hlsl::CBFIT_RAW) && !raw.contains(tombox_xshader::hlsl::CBFIT_CBUFFER), "{raw}");
}
