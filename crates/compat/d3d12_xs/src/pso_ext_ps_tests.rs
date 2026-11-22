use super::*;
use crate::device_wrap::Guid;

const IID_DEV: Guid = Guid { d1: 0x1898_19f1, d2: 0x1db6, d3: 0x4b57, d4: [0xbe, 0x54, 0x18, 0x21, 0x33, 0x9b, 0x85, 0xf7] };
const IID_QUEUE: Guid = Guid { d1: 0x0ec8_70a6, d2: 0x5d7e, d3: 0x4c22, d4: [0x8c, 0xfc, 0x5b, 0xaa, 0xe0, 0x76, 0x16, 0xed] };
const IID_ALLOCATOR: Guid = Guid { d1: 0x6102_dee4, d2: 0xaf59, d3: 0x4b09, d4: [0xb9, 0x99, 0xb4, 0x4d, 0x73, 0xf0, 0x9b, 0x24] };
const IID_LIST: Guid = Guid { d1: 0x5b16_0d0f, d2: 0xac1b, d3: 0x4185, d4: [0x8b, 0xa8, 0xb3, 0xae, 0x42, 0xa5, 0xa4, 0x55] };
const IID_RESOURCE: Guid = Guid { d1: 0x6964_42be, d2: 0xa72e, d3: 0x4059, d4: [0xbc, 0x79, 0x5b, 0x5c, 0x98, 0x04, 0x0f, 0xad] };
const IID_FENCE: Guid = Guid { d1: 0x0a75_3dcf, d2: 0xc4d8, d3: 0x4b91, d4: [0xad, 0xf6, 0xbe, 0x5a, 0x60, 0xd9, 0x5a, 0x76] };
const IID_RS: Guid = Guid { d1: 0xc54a_6b66, d2: 0x72df, d3: 0x4ee8, d4: [0x8b, 0xe5, 0xa9, 0x46, 0xa1, 0x42, 0x92, 0x14] };
const IID_PSO: Guid = Guid { d1: 0x765a_30f3, d2: 0xf624, d3: 0x4c6f, d4: [0xa8, 0x28, 0xac, 0xe9, 0x48, 0x62, 0x24, 0x45] };
const IID_HEAP: Guid = Guid { d1: 0x8efb_471d, d2: 0x616c, d3: 0x4f49, d4: [0x90, 0xf7, 0x12, 0x7b, 0xb7, 0x63, 0xfa, 0x51] };

const SIZE: u32 = 64;
const PIXELS: u32 = SIZE * SIZE;

fn ps(depth: bool, discard: bool) -> String {
    let mut h = String::from("RWByteAddressBuffer tb_u : register(u0);\nstruct SIn {\n    float4 i0 : SV_Position;\n    float4 i1 : TEXCOORD0;\n    nointerpolation uint4 i2 : TEXCOORD1;\n};\n");
    if depth {
        h.push_str("struct SOut {\n    float od : SV_Depth;\n};\nSOut main(SIn i) {\n    SOut o = (SOut)0;\n");
    } else {
        h.push_str("void main(SIn i) {\n");
    }
    h.push_str("    uint exec_lo = 1u;\n    uint tb_n = 0u;\n    tb_u.InterlockedAdd(0, i.i1.x >= 0.0 ? 1u : 0u, tb_n);\n");
    if discard {
        h.push_str("    if (i.i1.y < -1.0) exec_lo = 0u;\n    if ((exec_lo & 1u) == 0u) discard;\n");
    }
    if depth {
        h.push_str("    if (exec_lo & 1u) o.od = i.i0.z + 0.25;\n    return o;\n");
    }
    h.push_str("}\n");
    h
}

fn opts(per_sample: u32, z_order: u32, tex_kill: u32, cons: u32) -> PsOptions {
    let mut o = [0u32; 8];
    o[PER_SAMPLE] = per_sample;
    o[Z_ORDER] = z_order;
    o[TEX_KILL] = tex_kill;
    o[CONSERVATIVE_Z] = cons;
    o
}

#[test]
fn ps_options_select_only_the_edits_with_a_pc_equivalent() {
    assert!(PsEdit::from_options(&[0; 8], true).is_empty());
    assert!(PsEdit::from_options(&opts(2, 1, 2, 1), true).is_empty());
    assert!(PsEdit::from_options(&opts(0, 3, 0, 0), true).is_empty());
    let e = PsEdit::from_options(&opts(1, 2, 0, 3), true);
    assert_eq!((e.per_sample, e.early_z, e.ds_write, e.cons_z), (true, true, true, 3));
    assert!(!PsEdit::from_options(&opts(0, 4, 0, 0), false).ds_write);
    let m = merge(&opts(1, 2, 0, 0), &opts(0, 4, 2, 0));
    assert_eq!(m, opts(1, 4, 2, 0));
    let mut ents = vec![0u8; EXT_STRIDE * 2];
    ents[0x40..0x44].copy_from_slice(&1u32.to_le_bytes());
    ents[4..8].copy_from_slice(&1u32.to_le_bytes());
    assert_eq!(options_in_entries(&ents, 2), Some(opts(1, 0, 0, 0)));
    assert_eq!(options_in_entries(&ents[0x40..], 1), None);
}

#[test]
fn ps_edits_follow_the_generated_shader_shape_and_refuse_unknown_shapes() {
    let h = ps(false, false);
    let all = PsEdit { per_sample: true, early_z: true, ds_write: false, cons_z: 0 };
    let v = apply(&h, all).unwrap();
    assert!(v.contains("    sample float4 i1 : TEXCOORD0;"), "{v}");
    assert!(v.contains("    float4 i0 : SV_Position;") && v.contains("    nointerpolation uint4 i2 : TEXCOORD1;"));
    assert!(v.contains("[earlydepthstencil]\nvoid main(SIn i) {"));
    let d = apply(&ps(true, false), PsEdit { cons_z: 3, ..Default::default() }).unwrap();
    assert!(d.contains("float od : SV_DepthGreaterEqual;") && d.contains("    linear noperspective centroid float4 i0 : SV_Position;"), "{d}");
    let d = apply(&ps(true, false), PsEdit { cons_z: 2, ..Default::default() }).unwrap();
    assert!(d.contains("float od : SV_DepthLessEqual;"));
    let only_sys = "struct SIn {\n    float4 i0 : SV_Position;\n};\nvoid main(SIn i) {\n}\n";
    let v = apply(only_sys, PsEdit { per_sample: true, ..Default::default() }).unwrap();
    assert!(v.contains("    uint tb_sample_rate : SV_SampleIndex;\n};"), "{v}");
    assert!(apply("float4 main() : SV_Target { return 0; }", all).is_err());
    let (e, gaps) = plan(&ps(true, false), PsEdit { early_z: true, ..Default::default() });
    assert!(!e.early_z && gaps.len() == 1);
    let (e, gaps) = plan(&ps(false, true), PsEdit { early_z: true, ds_write: true, ..Default::default() });
    assert!(!e.early_z && gaps.len() == 1);
    let (e, gaps) = plan(&ps(false, true), PsEdit { early_z: true, ds_write: false, ..Default::default() });
    assert!(e.early_z && gaps.is_empty());
    let (e, _) = plan(&ps(false, false), PsEdit { cons_z: 2, ..Default::default() });
    assert_eq!(e.cons_z, 0);
    for (hl, e) in [(ps(false, false), all), (ps(true, true), PsEdit { per_sample: true, cons_z: 2, ..Default::default() })] {
        let src = apply(&hl, e).unwrap();
        assert!(tombox_xshader::dxc::compile(&src, "ps_6_0").is_ok(), "{src}");
    }
}

#[test]
fn variant_cache_keys_on_the_edit_and_falls_back_on_failure() {
    let o = PsOrigin { blob: Arc::new(vec![1, 2, 3]), hash: 0x7e57_0001, rtfmt: [0; 8], options: opts(1, 0, 0, 0) };
    let calls = Mutex::new(0);
    let ok = |_: &[u8], _: u64, _: &[u8; 8], e: &dyn Fn(&str) -> Result<String, String>| -> Result<Vec<u8>, String> {
        *calls.lock().unwrap() += 1;
        e(&ps(false, false)).map(|s| s.into_bytes())
    };
    let a = variant_with(&o, false, &ok).unwrap();
    let b = variant_with(&o, true, &ok).unwrap();
    assert!(Arc::ptr_eq(&a, &b));
    assert_eq!(*calls.lock().unwrap(), 1);
    assert!(String::from_utf8_lossy(&a).contains("sample float4 i1"));
    let none = PsOrigin { options: opts(2, 1, 1, 1), hash: 0x7e57_0002, ..o.clone() };
    assert!(variant_with(&none, true, &ok).is_none());
    let bad = PsOrigin { options: opts(0, 2, 0, 0), hash: 0x7e57_0003, ..o.clone() };
    let fail = |_: &[u8], _: u64, _: &[u8; 8], _: &dyn Fn(&str) -> Result<String, String>| -> Result<Vec<u8>, String> { Err("dxc".into()) };
    assert!(variant_with(&bad, false, &fail).is_none());
    let kill = PsOrigin { options: opts(0, 2, 0, 0), hash: 0x7e57_0004, ..o.clone() };
    let killer = |_: &[u8], _: u64, _: &[u8; 8], e: &dyn Fn(&str) -> Result<String, String>| -> Result<Vec<u8>, String> { e(&ps(false, true)).map(|s| s.into_bytes()) };
    assert!(variant_with(&kill, true, &killer).is_none());
    let p = 0x7e57_0010 as *mut c_void;
    note_pso(p, Some(o.clone()));
    assert_eq!(origin_of(p).map(|x| x.hash), Some(o.hash));
    note_pso(p, None);
    assert!(origin_of(p).is_none());
}

unsafe fn vslot(o: *mut c_void, i: usize) -> usize {
    unsafe { *(*(o as *const *const usize)).add(i) }
}

macro_rules! call {
    ($o:expr, $i:expr, ($($t:ty),*) -> $r:ty $(, $a:expr)*) => {{
        let o = $o as *mut c_void;
        let f: unsafe extern "system" fn(*mut c_void $(, $t)*) -> $r = core::mem::transmute(vslot(o, $i));
        f(o $(, $a)*)
    }};
}

struct Rig {
    dev: *mut c_void,
    queue: *mut c_void,
    rs: *mut c_void,
    vs_hi: Vec<u8>,
    vs_lo: Vec<u8>,
}

fn null() -> *mut c_void {
    core::ptr::null_mut()
}

const VS: &str = "#define RS \"UAV(u0)\"\nstruct VOut { float4 p : SV_Position; float4 t : TEXCOORD0; nointerpolation uint4 k : TEXCOORD1; };\n[RootSignature(RS)]\nVOut main(uint id : SV_VertexID) { VOut o; float2 uv = float2((id << 1) & 2, id & 2); o.p = float4(uv * float2(2, -2) + float2(-1, 1), TB_Z, 1); o.t = float4(uv * 64.0, 0, 1); o.k = uint4(id, 0, 0, 0); return o; }\n";

unsafe fn rig() -> Rig {
    unsafe {
        crate::enable_experimental_shader_models();
        let mk: extern "system" fn(*mut c_void, u32, *const Guid, *mut *mut c_void) -> i32 = core::mem::transmute(crate::host_proc(b"D3D12CreateDevice\0"));
        let mut dev = null();
        assert!(mk(null(), 0xb000, &IID_DEV, &mut dev) >= 0 && !dev.is_null());
        let qd = [0u32; 4];
        let mut queue = null();
        assert!(call!(dev, 8, (*const u32, *const Guid, *mut *mut c_void) -> i32, qd.as_ptr(), &IID_QUEUE, &mut queue) >= 0);
        let vs_hi = tombox_xshader::dxc::compile(&VS.replace("TB_Z", "0.5"), "vs_6_0").expect("vs");
        let vs_lo = tombox_xshader::dxc::compile(&VS.replace("TB_Z", "0.6"), "vs_6_0").expect("vs");
        let mut rs = null();
        assert!(call!(dev, 16, (u32, *const u8, usize, *const Guid, *mut *mut c_void) -> i32, 0, vs_hi.as_ptr(), vs_hi.len(), &IID_RS, &mut rs) >= 0);
        Rig { dev, queue, rs, vs_hi, vs_lo }
    }
}

fn put(d: &mut [u8], o: usize, v: u32) {
    d[o..o + 4].copy_from_slice(&v.to_le_bytes());
}

struct Draw<'a> {
    vs: &'a [u8],
    ps: &'a [u8],
    depth: bool,
    write: bool,
    func: u32,
}

unsafe fn pso(r: &Rig, d: &Draw, samples: u32) -> *mut c_void {
    let mut p = vec![0u8; 0x290];
    p[0..8].copy_from_slice(&(r.rs as u64).to_le_bytes());
    p[8..0x10].copy_from_slice(&(d.vs.as_ptr() as u64).to_le_bytes());
    p[0x10..0x18].copy_from_slice(&(d.vs.len() as u64).to_le_bytes());
    p[0x18..0x20].copy_from_slice(&(d.ps.as_ptr() as u64).to_le_bytes());
    p[0x20..0x28].copy_from_slice(&(d.ps.len() as u64).to_le_bytes());
    for i in 0..8 {
        let o = 0x80 + i * 40;
        for (k, v) in [1u32, 2, 1, 1, 2, 1, 4].iter().enumerate() {
            put(&mut p, o + 8 + 4 * k, *v);
        }
        p[o + 36] = 15;
    }
    put(&mut p, 0x1c0, 0xffff_ffff);
    put(&mut p, 0x1c4, 3);
    put(&mut p, 0x1c8, 1);
    put(&mut p, 0x1dc, 1);
    put(&mut p, 0x1f0, d.depth as u32);
    put(&mut p, 0x1f4, d.write as u32);
    put(&mut p, 0x1f8, d.func);
    for f in [0x204usize, 0x214] {
        for (k, v) in [1u32, 1, 1, 8].iter().enumerate() {
            put(&mut p, f + 4 * k, *v);
        }
    }
    put(&mut p, 0x23c, 3);
    put(&mut p, 0x264, 40);
    put(&mut p, 0x268, samples);
    let mut out = null();
    let hr = unsafe { call!(r.dev, 10, (*const u8, *const Guid, *mut *mut c_void) -> i32, p.as_ptr(), &IID_PSO, &mut out) };
    assert!(hr >= 0, "pso hr={hr:#010x}");
    out
}

unsafe fn committed(dev: *mut c_void, heap: u32, desc: &[u8], state: u32, clear: *const u8) -> *mut c_void {
    let h: [u32; 5] = [heap, 0, 0, 1, 1];
    let mut r = null();
    let hr = unsafe { call!(dev, 27, (*const u32, u32, *const u8, u32, *const u8, *const Guid, *mut *mut c_void) -> i32, h.as_ptr(), 0, desc.as_ptr(), state, clear, &IID_RESOURCE, &mut r) };
    assert!(hr >= 0, "resource hr={hr:#010x}");
    r
}

fn buffer_desc(size: u64, flags: u32) -> Vec<u8> {
    let mut d = vec![0u8; 56];
    put(&mut d, 0, 1);
    d[0x10..0x18].copy_from_slice(&size.to_le_bytes());
    put(&mut d, 0x18, 1);
    d[0x1c..0x1e].copy_from_slice(&1u16.to_le_bytes());
    d[0x1e..0x20].copy_from_slice(&1u16.to_le_bytes());
    put(&mut d, 0x24, 1);
    put(&mut d, 0x2c, 1);
    put(&mut d, 0x30, flags);
    d
}

fn barrier(res: *mut c_void, before: u32, after: u32) -> [u8; 0x20] {
    let mut b = [0u8; 0x20];
    b[8..0x10].copy_from_slice(&(res as u64).to_le_bytes());
    b[0x10..0x14].copy_from_slice(&0xffff_ffffu32.to_le_bytes());
    b[0x14..0x18].copy_from_slice(&before.to_le_bytes());
    b[0x18..0x1c].copy_from_slice(&after.to_le_bytes());
    b
}

unsafe fn run(r: &Rig, draws: &[Draw], samples: u32, clear_depth: f32) -> u32 {
    unsafe {
        let dev = r.dev;
        let mut dd = vec![0u8; 56];
        put(&mut dd, 0, 3);
        dd[0x10..0x18].copy_from_slice(&(SIZE as u64).to_le_bytes());
        put(&mut dd, 0x18, SIZE);
        dd[0x1c..0x1e].copy_from_slice(&1u16.to_le_bytes());
        dd[0x1e..0x20].copy_from_slice(&1u16.to_le_bytes());
        put(&mut dd, 0x20, 40);
        put(&mut dd, 0x24, samples);
        put(&mut dd, 0x30, 2);
        let mut cv = [0u8; 0x14];
        cv[0..4].copy_from_slice(&40u32.to_le_bytes());
        cv[4..8].copy_from_slice(&clear_depth.to_le_bytes());
        let depth = committed(dev, 1, &dd, 0x10, cv.as_ptr());
        let uav = committed(dev, 1, &buffer_desc(256, 4), 8, core::ptr::null());
        let back = committed(dev, 3, &buffer_desc(256, 0), 0x400, core::ptr::null());
        let hd = [3u32, 1, 0, 0];
        let mut heap = null();
        assert!(call!(dev, 14, (*const u32, *const Guid, *mut *mut c_void) -> i32, hd.as_ptr(), &IID_HEAP, &mut heap) >= 0);
        let mut dsv = 0usize;
        call!(heap, 9, (*mut usize) -> *mut usize, &mut dsv);
        call!(dev, 21, (*mut c_void, *const c_void, usize) -> (), depth, core::ptr::null(), dsv);
        let mut alloc = null();
        assert!(call!(dev, 9, (u32, *const Guid, *mut *mut c_void) -> i32, 0, &IID_ALLOCATOR, &mut alloc) >= 0);
        let mut l = null();
        assert!(call!(dev, 12, (u32, u32, *mut c_void, *mut c_void, *const Guid, *mut *mut c_void) -> i32, 0, 0, alloc, null(), &IID_LIST, &mut l) >= 0);
        let psos: Vec<*mut c_void> = draws.iter().map(|d| pso(r, d, samples)).collect();
        call!(l, 46, (u32, *const usize, i32, *const usize) -> (), 0, core::ptr::null(), 0, &dsv);
        call!(l, 47, (usize, u32, f32, u8, u32, *const c_void) -> (), dsv, 1, clear_depth, 0, 0, core::ptr::null());
        call!(l, 30, (*mut c_void) -> (), r.rs);
        let va = call!(uav, 11, () -> u64);
        call!(l, 42, (u32, u64) -> (), 0, va);
        let vp = [0f32, 0.0, SIZE as f32, SIZE as f32, 0.0, 1.0];
        call!(l, 21, (u32, *const f32) -> (), 1, vp.as_ptr());
        let sc = [0i32, 0, SIZE as i32, SIZE as i32];
        call!(l, 22, (u32, *const i32) -> (), 1, sc.as_ptr());
        call!(l, 20, (u32) -> (), 4);
        for p in &psos {
            call!(l, 25, (*mut c_void) -> (), *p);
            call!(l, 12, (u32, u32, u32, u32) -> (), 3, 1, 0, 0);
        }
        let b = barrier(uav, 8, 0x800);
        call!(l, 26, (u32, *const u8) -> (), 1, b.as_ptr());
        call!(l, 15, (*mut c_void, u64, *mut c_void, u64, u64) -> (), back, 0, uav, 0, 4);
        assert!(call!(l, 9, () -> i32) >= 0);
        let mut fence = null();
        assert!(call!(dev, 36, (u64, u32, *const Guid, *mut *mut c_void) -> i32, 0, 0, &IID_FENCE, &mut fence) >= 0);
        let ls = [l];
        call!(r.queue, 10, (u32, *const *mut c_void) -> (), 1, ls.as_ptr());
        call!(r.queue, 14, (*mut c_void, u64) -> i32, fence, 1);
        let t = std::time::Instant::now();
        while call!(fence, 8, () -> u64) < 1 {
            assert!(t.elapsed().as_secs() < 10, "GPU timeout");
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        let mut cpu: *mut u8 = core::ptr::null_mut();
        assert!(call!(back, 8, (u32, *const c_void, *mut *mut u8) -> i32, 0, core::ptr::null(), &mut cpu) >= 0);
        let n = core::ptr::read_volatile(cpu as *const u32);
        for o in psos.into_iter().chain([fence, l, alloc, heap, back, uav, depth]) {
            call!(o, 2, () -> u32);
        }
        n
    }
}

fn compile_ps(h: &str) -> Vec<u8> {
    tombox_xshader::dxc::compile(h, "ps_6_0").unwrap_or_else(|e| panic!("{e}\n{h}"))
}

fn edited(h: &str, o: &PsOptions, ds_write: bool) -> Vec<u8> {
    let (e, gaps) = plan(h, PsEdit::from_options(o, ds_write));
    assert!(gaps.is_empty(), "{gaps:?}");
    compile_ps(&apply(h, e).unwrap())
}

#[test]
fn per_sample_iteration_runs_the_translated_ps_once_per_sample_like_ps_iter_sample() {
    unsafe {
        let r = rig();
        let h = ps(false, false);
        let base = compile_ps(&h);
        let var = edited(&h, &opts(1, 0, 0, 0), false);
        let d = |p: &[u8]| run(&r, &[Draw { vs: &r.vs_hi, ps: p, depth: false, write: false, func: 8 }], 4, 1.0);
        let (nb, nv) = (d(&base), d(&var));
        assert_eq!((nb, nv), (PIXELS, PIXELS * 4), "base={nb} variant={nv}");
        let off = edited(&h, &opts(2, 0, 0, 0), false);
        assert_eq!(off, base);
        call!(r.rs, 2, () -> u32);
        call!(r.queue, 2, () -> u32);
        call!(r.dev, 2, () -> u32);
    }
}

#[test]
fn early_z_order_skips_occluded_pixels_before_the_translated_ps_runs() {
    unsafe {
        let r = rig();
        let h = ps(false, false);
        let base = compile_ps(&h);
        let var = edited(&h, &opts(0, 2, 0, 0), true);
        let var4 = edited(&h, &opts(0, 4, 0, 0), true);
        let late = edited(&h, &opts(0, 1, 0, 0), true);
        assert_eq!(late, base);
        let d = |p: &[u8]| run(&r, &[Draw { vs: &r.vs_hi, ps: p, depth: true, write: true, func: 2 }], 1, 0.0);
        let (nb, nv, n4) = (d(&base), d(&var), d(&var4));
        assert_eq!((nb, nv, n4), (PIXELS, 0, 0), "late-Z base={nb} EARLY_Z_THEN_LATE_Z={nv} EARLY_Z_THEN_RE_Z={n4}");
        let killer = ps(false, true);
        let (e, gaps) = plan(&killer, PsEdit::from_options(&opts(0, 2, 0, 0), true));
        assert!(!e.early_z && gaps.len() == 1);
        let kill_ro = edited(&killer, &opts(0, 2, 0, 0), false);
        let nk = run(&r, &[Draw { vs: &r.vs_hi, ps: &kill_ro, depth: true, write: false, func: 2 }], 1, 0.0);
        assert_eq!(nk, 0);
        call!(r.rs, 2, () -> u32);
        call!(r.queue, 2, () -> u32);
        call!(r.dev, 2, () -> u32);
    }
}

#[test]
fn conservative_z_export_variant_keeps_the_exported_depth() {
    unsafe {
        let r = rig();
        let h = ps(true, false);
        let base = compile_ps(&h);
        let probe = edited(&ps(false, false), &opts(0, 2, 0, 0), false);
        for cons in [3u32, 0] {
            let writer = if cons == 0 { base.clone() } else { edited(&h, &opts(0, 0, 0, cons), true) };
            let n = run(&r, &[
                Draw { vs: &r.vs_hi, ps: &writer, depth: true, write: true, func: 2 },
                Draw { vs: &r.vs_lo, ps: &probe, depth: true, write: false, func: 2 },
            ], 1, 1.0);
            assert_eq!(n, PIXELS * 2, "conservative={cons}: exported depth 0.75 must hold the probe at 0.6 in front");
        }
        call!(r.rs, 2, () -> u32);
        call!(r.queue, 2, () -> u32);
        call!(r.dev, 2, () -> u32);
    }
}

extern "system" {
    fn LoadLibraryA(name: *const u8) -> *mut c_void;
    fn GetProcAddress(h: *mut c_void, name: *const u8) -> *mut c_void;
}

const IID_FACTORY4: Guid = Guid { d1: 0x1bc6_ea02, d2: 0xef36, d3: 0x464f, d4: [0xbf, 0x0c, 0x21, 0xca, 0x39, 0xe5, 0x16, 0x8a] };
const IID_ADAPTER: Guid = Guid { d1: 0x2411_e7e1, d2: 0x12ac, d3: 0x4ccf, d4: [0xbd, 0x14, 0x97, 0x98, 0xe8, 0x53, 0x4d, 0xc0] };
const IID_QUERY_HEAP: Guid = Guid { d1: 0x0d96_58ae, d2: 0xed45, d3: 0x469e, d4: [0xa6, 0x1d, 0x97, 0x0e, 0xc5, 0x83, 0xca, 0xb4] };

unsafe fn warp_adapter() -> *mut c_void {
    let h = LoadLibraryA(b"dxgi.dll\0".as_ptr());
    assert!(!h.is_null());
    let p = GetProcAddress(h, b"CreateDXGIFactory2\0".as_ptr());
    assert!(!p.is_null());
    let create: unsafe extern "system" fn(u32, *const Guid, *mut *mut c_void) -> i32 = core::mem::transmute(p);
    let mut f: *mut c_void = core::ptr::null_mut();
    assert!(create(0, &IID_FACTORY4, &mut f) >= 0);
    let mut a: *mut c_void = core::ptr::null_mut();
    assert!(call!(f, 27, (*const Guid, *mut *mut c_void) -> i32, &IID_ADAPTER, &mut a) >= 0);
    call!(f, 2, () -> u32);
    a
}

unsafe fn rig_warp() -> Rig {
    unsafe {
        crate::enable_experimental_shader_models();
        let adapter = warp_adapter();
        let mk: extern "system" fn(*mut c_void, u32, *const Guid, *mut *mut c_void) -> i32 = core::mem::transmute(crate::host_proc(b"D3D12CreateDevice\0"));
        let mut dev = null();
        let hr = mk(adapter, 0xb000, &IID_DEV, &mut dev);
        call!(adapter, 2, () -> u32);
        assert!(hr >= 0 && !dev.is_null(), "D3D12CreateDevice(WARP) hr={hr:#010x}");
        let qd = [0u32; 4];
        let mut queue = null();
        assert!(call!(dev, 8, (*const u32, *const Guid, *mut *mut c_void) -> i32, qd.as_ptr(), &IID_QUEUE, &mut queue) >= 0);
        let vs_hi = tombox_xshader::dxc::compile(&VS.replace("TB_Z", "0.5"), "vs_6_0").expect("vs");
        let vs_lo = tombox_xshader::dxc::compile(&VS.replace("TB_Z", "0.6"), "vs_6_0").expect("vs");
        let mut rs = null();
        assert!(call!(dev, 16, (u32, *const u8, usize, *const Guid, *mut *mut c_void) -> i32, 0, vs_hi.as_ptr(), vs_hi.len(), &IID_RS, &mut rs) >= 0);
        Rig { dev, queue, rs, vs_hi, vs_lo }
    }
}

unsafe fn pso_mask(r: &Rig, d: &Draw, samples: u32, mask: u32) -> *mut c_void {
    let mut p = vec![0u8; 0x290];
    p[0..8].copy_from_slice(&(r.rs as u64).to_le_bytes());
    p[8..0x10].copy_from_slice(&(d.vs.as_ptr() as u64).to_le_bytes());
    p[0x10..0x18].copy_from_slice(&(d.vs.len() as u64).to_le_bytes());
    if !d.ps.is_empty() {
        p[0x18..0x20].copy_from_slice(&(d.ps.as_ptr() as u64).to_le_bytes());
        p[0x20..0x28].copy_from_slice(&(d.ps.len() as u64).to_le_bytes());
    }
    for i in 0..8 {
        let o = 0x80 + i * 40;
        for (k, v) in [1u32, 2, 1, 1, 2, 1, 4].iter().enumerate() {
            put(&mut p, o + 8 + 4 * k, *v);
        }
        p[o + 36] = 15;
    }
    put(&mut p, 0x1c0, mask);
    put(&mut p, 0x1c4, 3);
    put(&mut p, 0x1c8, 1);
    put(&mut p, 0x1dc, 1);
    put(&mut p, 0x1f0, d.depth as u32);
    put(&mut p, 0x1f4, d.write as u32);
    put(&mut p, 0x1f8, d.func);
    for f in [0x204usize, 0x214] {
        for (k, v) in [1u32, 1, 1, 8].iter().enumerate() {
            put(&mut p, f + 4 * k, *v);
        }
    }
    put(&mut p, 0x23c, 3);
    put(&mut p, 0x264, 40);
    put(&mut p, 0x268, samples);
    let mut out = null();
    let hr = unsafe { call!(r.dev, 10, (*const u8, *const Guid, *mut *mut c_void) -> i32, p.as_ptr(), &IID_PSO, &mut out) };
    assert!(hr >= 0, "pso hr={hr:#010x}");
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MaskMetrics {
    ps_invocations: u32,
    occlusion_samples: u64,
}

unsafe fn run_with_sample_mask(r: &Rig, d: &Draw, samples: u32, mask: u32) -> MaskMetrics {
    unsafe {
        let dev = r.dev;
        let mut dd = vec![0u8; 56];
        put(&mut dd, 0, 3);
        dd[0x10..0x18].copy_from_slice(&(SIZE as u64).to_le_bytes());
        put(&mut dd, 0x18, SIZE);
        dd[0x1c..0x1e].copy_from_slice(&1u16.to_le_bytes());
        dd[0x1e..0x20].copy_from_slice(&1u16.to_le_bytes());
        put(&mut dd, 0x20, 40);
        put(&mut dd, 0x24, samples);
        put(&mut dd, 0x30, 2);
        let mut cv = [0u8; 0x14];
        cv[0..4].copy_from_slice(&40u32.to_le_bytes());
        cv[4..8].copy_from_slice(&1.0f32.to_le_bytes());
        let depth = committed(dev, 1, &dd, 0x10, cv.as_ptr());
        let uav = committed(dev, 1, &buffer_desc(256, 4), 8, core::ptr::null());
        let back = committed(dev, 3, &buffer_desc(256, 0), 0x400, core::ptr::null());
        let qback = committed(dev, 3, &buffer_desc(256, 0), 0x400, core::ptr::null());
        let hd = [3u32, 1, 0, 0];
        let mut heap = null();
        assert!(call!(dev, 14, (*const u32, *const Guid, *mut *mut c_void) -> i32, hd.as_ptr(), &IID_HEAP, &mut heap) >= 0);
        let mut dsv = 0usize;
        call!(heap, 9, (*mut usize) -> *mut usize, &mut dsv);
        call!(dev, 21, (*mut c_void, *const c_void, usize) -> (), depth, core::ptr::null(), dsv);
        let qdesc: [u32; 3] = [0, 4, 0];
        let mut qheap: *mut c_void = null();
        assert!(call!(dev, 39, (*const u32, *const Guid, *mut *mut c_void) -> i32, qdesc.as_ptr(), &IID_QUERY_HEAP, &mut qheap) >= 0);
        let mut alloc = null();
        assert!(call!(dev, 9, (u32, *const Guid, *mut *mut c_void) -> i32, 0, &IID_ALLOCATOR, &mut alloc) >= 0);
        let mut l = null();
        assert!(call!(dev, 12, (u32, u32, *mut c_void, *mut c_void, *const Guid, *mut *mut c_void) -> i32, 0, 0, alloc, null(), &IID_LIST, &mut l) >= 0);
        let p = pso_mask(r, d, samples, mask);
        call!(l, 46, (u32, *const usize, i32, *const usize) -> (), 0, core::ptr::null(), 0, &dsv);
        call!(l, 47, (usize, u32, f32, u8, u32, *const c_void) -> (), dsv, 1, 1.0f32, 0, 0, core::ptr::null());
        call!(l, 30, (*mut c_void) -> (), r.rs);
        let va = call!(uav, 11, () -> u64);
        call!(l, 42, (u32, u64) -> (), 0, va);
        let vp = [0f32, 0.0, SIZE as f32, SIZE as f32, 0.0, 1.0];
        call!(l, 21, (u32, *const f32) -> (), 1, vp.as_ptr());
        let sc = [0i32, 0, SIZE as i32, SIZE as i32];
        call!(l, 22, (u32, *const i32) -> (), 1, sc.as_ptr());
        call!(l, 20, (u32) -> (), 4);
        call!(l, 25, (*mut c_void) -> (), p);
        call!(l, 52, (*mut c_void, u32, u32) -> (), qheap, 0, 0);
        call!(l, 12, (u32, u32, u32, u32) -> (), 3, 1, 0, 0);
        call!(l, 53, (*mut c_void, u32, u32) -> (), qheap, 0, 0);
        call!(l, 54, (*mut c_void, u32, u32, u32, *mut c_void, u64) -> (), qheap, 0, 0, 1, qback, 0);
        let b = barrier(uav, 8, 0x800);
        call!(l, 26, (u32, *const u8) -> (), 1, b.as_ptr());
        call!(l, 15, (*mut c_void, u64, *mut c_void, u64, u64) -> (), back, 0, uav, 0, 4);
        assert!(call!(l, 9, () -> i32) >= 0);
        let mut fence = null();
        assert!(call!(dev, 36, (u64, u32, *const Guid, *mut *mut c_void) -> i32, 0, 0, &IID_FENCE, &mut fence) >= 0);
        let ls = [l];
        call!(r.queue, 10, (u32, *const *mut c_void) -> (), 1, ls.as_ptr());
        call!(r.queue, 14, (*mut c_void, u64) -> i32, fence, 1);
        let t = std::time::Instant::now();
        while call!(fence, 8, () -> u64) < 1 {
            assert!(t.elapsed().as_secs() < 10, "GPU timeout");
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        let mut cpu: *mut u8 = core::ptr::null_mut();
        assert!(call!(back, 8, (u32, *const c_void, *mut *mut u8) -> i32, 0, core::ptr::null(), &mut cpu) >= 0);
        let ps_invocations = core::ptr::read_volatile(cpu as *const u32);
        let mut qcpu: *mut u8 = core::ptr::null_mut();
        assert!(call!(qback, 8, (u32, *const c_void, *mut *mut u8) -> i32, 0, core::ptr::null(), &mut qcpu) >= 0);
        let occlusion_samples = core::ptr::read_volatile(qcpu as *const u64);
        for o in [p, fence, l, alloc, qheap, heap, qback, back, uav, depth] {
            call!(o, 2, () -> u32);
        }
        MaskMetrics { ps_invocations, occlusion_samples }
    }
}

#[test]
fn sample_mask_ab_test_on_warp() {
    unsafe {
        let r = rig_warp();
        let h = ps(false, false);
        let shader = compile_ps(&h);

        // Test 1: 1x MSAA with Pixel Shader (full-screen quad/triangle covering 64x64 = 4096 pixels)
        let d_ps = Draw { vs: &r.vs_hi, ps: &shader, depth: true, write: true, func: 2 };
        let m1_0 = run_with_sample_mask(&r, &d_ps, 1, 0x0);
        let m1_all = run_with_sample_mask(&r, &d_ps, 1, 0xffff_ffff);
        let m1_1 = run_with_sample_mask(&r, &d_ps, 1, 0x1);

        // Test 2: 4x MSAA with Pixel Shader (64x64 = 4096 pixels, 4x MSAA = 16384 samples)
        let m4_0 = run_with_sample_mask(&r, &d_ps, 4, 0x0);
        let m4_all = run_with_sample_mask(&r, &d_ps, 4, 0xffff_ffff);
        let m4_1 = run_with_sample_mask(&r, &d_ps, 4, 0x1);

        // Test 3: Depth-only pass without Pixel Shader (UE / Fortnite occlusion query box geometry)
        let d_nops = Draw { vs: &r.vs_hi, ps: &[], depth: true, write: true, func: 2 };
        let occ1_0 = run_with_sample_mask(&r, &d_nops, 1, 0x0);
        let occ1_all = run_with_sample_mask(&r, &d_nops, 1, 0xffff_ffff);
        let occ1_1 = run_with_sample_mask(&r, &d_nops, 1, 0x1);

        let occ4_0 = run_with_sample_mask(&r, &d_nops, 4, 0x0);
        let occ4_all = run_with_sample_mask(&r, &d_nops, 4, 0xffff_ffff);
        let occ4_1 = run_with_sample_mask(&r, &d_nops, 4, 0x1);

        eprintln!("=== WARP SAMPLE MASK SYNTHETIC A/B TEST RESULTS ===");
        eprintln!("1x MSAA with PS: mask=0x0 -> ps_invocations={}, occlusion_samples={}", m1_0.ps_invocations, m1_0.occlusion_samples);
        eprintln!("1x MSAA with PS: mask=0xFFFFFFFF -> ps_invocations={}, occlusion_samples={}", m1_all.ps_invocations, m1_all.occlusion_samples);
        eprintln!("1x MSAA with PS: mask=0x1 -> ps_invocations={}, occlusion_samples={}", m1_1.ps_invocations, m1_1.occlusion_samples);

        eprintln!("4x MSAA with PS: mask=0x0 -> ps_invocations={}, occlusion_samples={}", m4_0.ps_invocations, m4_0.occlusion_samples);
        eprintln!("4x MSAA with PS: mask=0xFFFFFFFF -> ps_invocations={}, occlusion_samples={}", m4_all.ps_invocations, m4_all.occlusion_samples);
        eprintln!("4x MSAA with PS: mask=0x1 -> ps_invocations={}, occlusion_samples={}", m4_1.ps_invocations, m4_1.occlusion_samples);

        eprintln!("1x MSAA Depth-only (no PS): mask=0x0 -> ps_invocations={}, occlusion_samples={}", occ1_0.ps_invocations, occ1_0.occlusion_samples);
        eprintln!("1x MSAA Depth-only (no PS): mask=0xFFFFFFFF -> ps_invocations={}, occlusion_samples={}", occ1_all.ps_invocations, occ1_all.occlusion_samples);
        eprintln!("1x MSAA Depth-only (no PS): mask=0x1 -> ps_invocations={}, occlusion_samples={}", occ1_1.ps_invocations, occ1_1.occlusion_samples);

        eprintln!("4x MSAA Depth-only (no PS): mask=0x0 -> ps_invocations={}, occlusion_samples={}", occ4_0.ps_invocations, occ4_0.occlusion_samples);
        eprintln!("4x MSAA Depth-only (no PS): mask=0xFFFFFFFF -> ps_invocations={}, occlusion_samples={}", occ4_all.ps_invocations, occ4_all.occlusion_samples);
        eprintln!("4x MSAA Depth-only (no PS): mask=0x1 -> ps_invocations={}, occlusion_samples={}", occ4_1.ps_invocations, occ4_1.occlusion_samples);

        // Core assertions confirming SampleMask behavior:
        // When SampleMask = 0: exactly 0 samples pass the occlusion test!
        assert_eq!(m1_0.occlusion_samples, 0);
        assert_eq!(m4_0.occlusion_samples, 0);
        assert_eq!(occ1_0.occlusion_samples, 0);
        assert_eq!(occ4_0.occlusion_samples, 0);

        // When SampleMask = 0xFFFFFFFF: all covered samples pass!
        assert_eq!(m1_all.occlusion_samples, PIXELS as u64);
        assert_eq!(m4_all.occlusion_samples, (PIXELS * 4) as u64);
        assert_eq!(occ1_all.occlusion_samples, PIXELS as u64);
        assert_eq!(occ4_all.occlusion_samples, (PIXELS * 4) as u64);

        // When SampleMask = 0x1:
        // On 1x MSAA, sample 0 is the only sample, so mask 0x1 == 0xFFFFFFFF.
        assert_eq!(m1_1.occlusion_samples, PIXELS as u64);
        assert_eq!(occ1_1.occlusion_samples, PIXELS as u64);

        // On 4x MSAA, only sample 0 is active (1 of 4 samples per pixel), so exactly 1/4 of samples pass!
        assert_eq!(m4_1.occlusion_samples, PIXELS as u64);
        assert_eq!(occ4_1.occlusion_samples, PIXELS as u64);

        call!(r.rs, 2, () -> u32);
        call!(r.queue, 2, () -> u32);
        call!(r.dev, 2, () -> u32);
    }
}
