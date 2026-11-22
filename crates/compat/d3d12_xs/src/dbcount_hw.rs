use super::super::*;

const IID_QUEUE: Guid = Guid { d1: 0x0ec8_70a6, d2: 0x5d7e, d3: 0x4c22, d4: [0x8c, 0xfc, 0x5b, 0xaa, 0xe0, 0x76, 0x16, 0xed] };
const IID_ALLOCATOR: Guid = Guid { d1: 0x6102_dee4, d2: 0xaf59, d3: 0x4b09, d4: [0xb9, 0x99, 0xb4, 0x4d, 0x73, 0xf0, 0x9b, 0x24] };
const IID_LIST: Guid = Guid { d1: 0x5b16_0d0f, d2: 0xac1b, d3: 0x4185, d4: [0x8b, 0xa8, 0xb3, 0xae, 0x42, 0xa5, 0xa4, 0x55] };
const IID_FENCE: Guid = Guid { d1: 0x0a75_3dcf, d2: 0xc4d8, d3: 0x4b91, d4: [0xad, 0xf6, 0xbe, 0x5a, 0x60, 0xd9, 0x5a, 0x76] };
const IID_DESC_HEAP: Guid = Guid { d1: 0x8efb_471d, d2: 0x616c, d3: 0x4f49, d4: [0x90, 0xf7, 0x12, 0x7b, 0xb7, 0x63, 0xfa, 0x51] };
const IID_CMD_SIG: Guid = Guid { d1: 0xc36a_797c, d2: 0xec80, d3: 0x4f0a, d4: [0x89, 0x85, 0xa7, 0xb2, 0x47, 0x50, 0x82, 0xd1] };

const W: u32 = 64;
const H: u32 = 64;
const FMT_RGBA8: u32 = 28;
const FMT_D24S8: u32 = 45;
const LESS: u32 = 2;
const EQUAL: u32 = 3;
const ALWAYS: u32 = 8;
const KEEP: u32 = 1;
const REPLACE: u32 = 3;
const X_TOPOLOGY: usize = 20;
const X_VIEWPORTS: usize = 21;
const X_SCISSORS: usize = 22;
const X_STENCIL_REF: usize = 24;
const X_GROOT: usize = 34;
const X_GCONSTS: usize = 40;
const X_GUAV: usize = 46;
const X_DEPTH_BOUNDS: usize = 68;
const PC_CLOSE: usize = 9;
const PC_TOPOLOGY: usize = 20;
const PC_IB: usize = 43;
const DEV_CREATE_CMD_SIG: usize = 41;

macro_rules! vc {
    ($o:expr, $i:expr, fn($($t:ty),*) -> $r:ty $(, $a:expr)*) => {{
        let o = $o as *mut c_void;
        let f: unsafe extern "system" fn(*mut c_void $(, $t)*) -> $r = core::mem::transmute(vslot(o, $i));
        f(o $(, $a)*)
    }};
}

const RS: &str = "#define RS \"RootFlags(0), RootConstants(num32BitConstants=6, b0), UAV(u1)\"\n";
const VS: &str = r#"cbuffer C : register(b0) { float x0; float y0; float x1; float y1; float z0; float z1; };
[RootSignature(RS)]
float4 main(uint v : SV_VertexID, uint i : SV_InstanceID) : SV_Position {
    uint k = v % 6;
    float u = (k == 1 || k == 2 || k == 4) ? 1.0 : 0.0;
    float w = (k == 2 || k == 4 || k == 5) ? 1.0 : 0.0;
    return float4(lerp(x0, x1, u), lerp(y0, y1, w), i == 0 ? z0 : z1, 1.0);
}
"#;
const MS: &str = r#"cbuffer C : register(b0) { float x0; float y0; float x1; float y1; float z0; float z1; };
struct V { float4 p : SV_Position; };
[RootSignature(RS)]
[outputtopology("triangle")]
[numthreads(1, 1, 1)]
void main(out vertices V v[4], out indices uint3 t[2]) {
    SetMeshOutputCounts(4, 2);
    v[0].p = float4(x0, y0, z0, 1);
    v[1].p = float4(x1, y0, z0, 1);
    v[2].p = float4(x1, y1, z0, 1);
    v[3].p = float4(x0, y1, z0, 1);
    t[0] = uint3(0, 1, 2);
    t[1] = uint3(0, 2, 3);
}
"#;
const PS: &str = "float4 main() : SV_Target { return float4(1, 0, 1, 1); }";
const UAV_PS: &str = "RWByteAddressBuffer u : register(u1);\nfloat4 main() : SV_Target { u.InterlockedAdd(0, 1u); return float4(1, 0, 1, 1); }";

struct Kit {
    dev: usize,
    queue: usize,
    alloc: usize,
    list: usize,
    fence: usize,
    value: u64,
    rs: usize,
    rtv: usize,
    dsv: usize,
    qheap: usize,
    rb: usize,
    vt: Vec<AtomicUsize>,
    _vs: Vec<u8>,
    _ps: Vec<u8>,
}

unsafe fn committed(dev: usize, heap: u32, dim: u32, w: u64, h: u32, fmt: u32, flags: u32, state: u32, clear: Option<[u8; 24]>, samples: u32, array: u16) -> usize {
    let props: [u32; 5] = [heap, 0, 0, 1, 1];
    let mut d = [0u8; 56];
    d[0..4].copy_from_slice(&dim.to_le_bytes());
    d[0x10..0x18].copy_from_slice(&w.to_le_bytes());
    d[0x18..0x1c].copy_from_slice(&h.to_le_bytes());
    d[0x1c..0x1e].copy_from_slice(&array.to_le_bytes());
    d[0x1e..0x20].copy_from_slice(&1u16.to_le_bytes());
    d[0x20..0x24].copy_from_slice(&fmt.to_le_bytes());
    d[0x24..0x28].copy_from_slice(&samples.to_le_bytes());
    d[0x2c..0x30].copy_from_slice(&(if dim == 1 { 1u32 } else { 0 }).to_le_bytes());
    d[0x30..0x34].copy_from_slice(&flags.to_le_bytes());
    let mut r: *mut c_void = core::ptr::null_mut();
    let cv = clear.as_ref().map(|c| c.as_ptr() as *const c_void).unwrap_or(core::ptr::null());
    let hr = unsafe { vc!(dev, DEV_CREATE_COMMITTED, fn(*const u32, u32, *const u8, u32, *const c_void, *const Guid, *mut *mut c_void) -> i32, props.as_ptr(), 0, d.as_ptr(), state, cv, &IID_RESOURCE, &mut r) };
    assert!(hr >= 0 && !r.is_null(), "CreateCommittedResource dim={dim} fmt={fmt} flags={flags:#x} hr={hr:#010x}");
    r as usize
}

unsafe fn upload(dev: usize, data: &[u8]) -> (usize, u64) {
    let r = unsafe { committed(dev, 2, 1, data.len().max(256) as u64, 1, 0, 0, 0xac3, None, 1, 1) };
    unsafe {
        let mut m: *mut u8 = core::ptr::null_mut();
        assert!(vc!(r, 8, fn(u32, *const c_void, *mut *mut u8) -> i32, 0, core::ptr::null(), &mut m) >= 0);
        core::ptr::copy_nonoverlapping(data.as_ptr(), m, data.len());
        vc!(r, 9, fn(u32, *const c_void) -> (), 0, core::ptr::null());
        (r, vc!(r, RES_GET_VA, fn() -> u64))
    }
}

unsafe fn heap_start(dev: usize, ty: u32, n: u32) -> usize {
    let d = [ty, n, 0, 0];
    let mut h: *mut c_void = core::ptr::null_mut();
    assert!(unsafe { vc!(dev, 14, fn(*const u32, *const Guid, *mut *mut c_void) -> i32, d.as_ptr(), &IID_DESC_HEAP, &mut h) } >= 0);
    let mut out = 0usize;
    unsafe { vc!(h, 9, fn(*mut usize) -> *mut usize, &mut out) };
    out
}

fn put(d: &mut [u8], o: usize, v: &[u8]) {
    d[o..o + v.len()].copy_from_slice(v);
}

fn ds_block(depth: (u32, u32, u32), stencil: Option<(u32, u32, u8, u8)>, bounds: Option<bool>) -> Vec<u8> {
    let mut d = vec![0u8; if bounds.is_some() { DS1_LEN } else { DS_LEN }];
    put(&mut d, DS_DEPTH_ENABLE, &depth.0.to_le_bytes());
    put(&mut d, DS_DEPTH_WRITE, &depth.1.to_le_bytes());
    put(&mut d, DS_DEPTH_FUNC, &depth.2.to_le_bytes());
    let (en, func, pass, wm) = match stencil { Some((f, p, _, w)) => (1u32, f, p, w), None => (0, ALWAYS, KEEP, 0) };
    put(&mut d, DS_STENCIL_ENABLE, &en.to_le_bytes());
    d[16] = 0xff;
    d[DS_STENCIL_WRITE_MASK] = wm;
    for face in [DS_FRONT, DS_BACK] {
        put(&mut d, face, &KEEP.to_le_bytes());
        put(&mut d, face + 4, &KEEP.to_le_bytes());
        put(&mut d, face + 8, &pass.to_le_bytes());
        put(&mut d, face + 12, &func.to_le_bytes());
    }
    if bounds == Some(true) {
        put(&mut d, DS_BOUNDS, &1u32.to_le_bytes());
    }
    d
}

fn gfx_desc(rs: usize, vs: &[u8], ps: &[u8], depth: (u32, u32, u32), stencil: Option<(u32, u32, u8, u8)>, samples: u32) -> Vec<u8> {
    let mut d = vec![0u8; D_LEN];
    put(&mut d, 0, &(rs as u64).to_le_bytes());
    put(&mut d, 0x08, &(vs.as_ptr() as u64).to_le_bytes());
    put(&mut d, 0x10, &(vs.len() as u64).to_le_bytes());
    put(&mut d, 0x18, &(ps.as_ptr() as u64).to_le_bytes());
    put(&mut d, 0x20, &(ps.len() as u64).to_le_bytes());
    put(&mut d, D_BLEND, &default_blend());
    put(&mut d, 0x1c0, &u32::MAX.to_le_bytes());
    put(&mut d, 0x1c4, &3u32.to_le_bytes());
    put(&mut d, 0x1c8, &1u32.to_le_bytes());
    put(&mut d, 0x1dc, &1u32.to_le_bytes());
    put(&mut d, D_DS, &ds_block(depth, stencil, None));
    put(&mut d, 0x23c, &3u32.to_le_bytes());
    put(&mut d, 0x240, &1u32.to_le_bytes());
    put(&mut d, 0x244, &FMT_RGBA8.to_le_bytes());
    put(&mut d, 0x264, &FMT_D24S8.to_le_bytes());
    put(&mut d, D_SAMPLE_COUNT, &samples.to_le_bytes());
    d
}

unsafe fn make_pso_with(k: &Kit, ps: std::sync::Arc<Vec<u8>>, depth: (u32, u32, u32), stencil: Option<(u32, u32, u8, u8)>, samples: u32) -> usize {
    let d = gfx_desc(k.rs, &k._vs, &ps, depth, stencil, samples);
    let mut p: *mut c_void = core::ptr::null_mut();
    let hr = unsafe { vc!(k.dev, DEV_CREATE_GFX_PSO, fn(*const u8, *const Guid, *mut *mut c_void) -> i32, d.as_ptr(), &IID_PSO, &mut p) };
    assert!(hr >= 0 && !p.is_null(), "CreateGraphicsPipelineState hr={hr:#010x}");
    let shaders = vec![std::sync::Arc::new(k._vs.clone()), ps];
    let mut keep = d.clone();
    keep[0x08..0x10].copy_from_slice(&(shaders[0].as_ptr() as u64).to_le_bytes());
    keep[0x18..0x20].copy_from_slice(&(shaders[1].as_ptr() as u64).to_le_bytes());
    unsafe { crate::device_wrap::register_gfx_pso(p, keep, shaders) };
    p as usize
}

unsafe fn make_pso(k: &Kit, depth: (u32, u32, u32), stencil: Option<(u32, u32, u8, u8)>, samples: u32) -> usize {
    unsafe { make_pso_with(k, std::sync::Arc::new(k._ps.clone()), depth, stencil, samples) }
}

unsafe fn make_stream_pso(k: &Kit, stage: (u32, &[u8]), ps: &[u8], ds: Vec<u8>) -> usize {
    let bc = |b: &[u8]| {
        let mut v = (b.as_ptr() as u64).to_le_bytes().to_vec();
        v.extend_from_slice(&(b.len() as u64).to_le_bytes());
        v
    };
    let mut rast = vec![0u8; 44];
    put(&mut rast, 0, &3u32.to_le_bytes());
    put(&mut rast, 4, &1u32.to_le_bytes());
    put(&mut rast, 0x18, &1u32.to_le_bytes());
    let mut rtf = vec![0u8; 36];
    put(&mut rtf, 0, &FMT_RGBA8.to_le_bytes());
    put(&mut rtf, 32, &1u32.to_le_bytes());
    let ds_ty = if ds.len() == DS1_LEN { SUB_DEPTH_STENCIL1 } else { SUB_DEPTH_STENCIL };
    let mut parts: Vec<(u32, Vec<u8>)> = vec![
        (0, (k.rs as u64).to_le_bytes().to_vec()),
        (stage.0, bc(stage.1)),
        (SUB_PS, bc(ps)),
        (SUB_BLEND, default_blend()),
        (9, u32::MAX.to_le_bytes().to_vec()),
        (10, rast),
        (ds_ty, ds),
        (15, rtf),
        (16, FMT_D24S8.to_le_bytes().to_vec()),
        (SUB_SAMPLE_DESC, vec![1, 0, 0, 0, 0, 0, 0, 0]),
    ];
    if stage.0 == SUB_VS {
        parts.push((14, 3u32.to_le_bytes().to_vec()));
    }
    let mut s = Vec::new();
    let mut subs = Vec::new();
    for (t, b) in &parts {
        append_sub(&mut s, &mut subs, *t, b);
    }
    unsafe {
        let mut d2: *mut c_void = core::ptr::null_mut();
        assert!(vc!(k.dev, 0, fn(*const Guid, *mut *mut c_void) -> i32, &IID_DEVICE2, &mut d2) >= 0);
        let desc = [s.len() as u64, s.as_ptr() as u64];
        let mut p: *mut c_void = core::ptr::null_mut();
        let hr = vc!(d2, DEV2_CREATE_PIPELINE_STATE, fn(*const [u64; 2], *const Guid, *mut *mut c_void) -> i32, &desc, &IID_PSO, &mut p);
        release(d2);
        assert!(hr >= 0 && !p.is_null(), "CreatePipelineState stream hr={hr:#010x}");
        note_stream_pso(p as usize, &s, &[]);
        p as usize
    }
}

unsafe fn kit(samples: u32) -> Kit {
    let create = crate::host_proc(b"D3D12CreateDevice\0");
    assert!(create != 0, "d3d12.dll missing");
    let f: extern "system" fn(*mut c_void, u32, *const Guid, *mut *mut c_void) -> i32 = unsafe { core::mem::transmute(create) };
    let mut dev: *mut c_void = core::ptr::null_mut();
    assert!(f(core::ptr::null_mut(), 0xb000, &IID_DEVICE, &mut dev) >= 0 && !dev.is_null());
    let dev = dev as usize;
    unsafe {
        let qd = [0u32; 4];
        let mut queue: *mut c_void = core::ptr::null_mut();
        assert!(vc!(dev, 8, fn(*const u32, *const Guid, *mut *mut c_void) -> i32, qd.as_ptr(), &IID_QUEUE, &mut queue) >= 0);
        let mut alloc: *mut c_void = core::ptr::null_mut();
        assert!(vc!(dev, 9, fn(u32, *const Guid, *mut *mut c_void) -> i32, 0, &IID_ALLOCATOR, &mut alloc) >= 0);
        let mut list: *mut c_void = core::ptr::null_mut();
        assert!(vc!(dev, 12, fn(u32, u32, *mut c_void, *mut c_void, *const Guid, *mut *mut c_void) -> i32, 0, 0, alloc, core::ptr::null_mut(), &IID_LIST, &mut list) >= 0);
        let mut fence: *mut c_void = core::ptr::null_mut();
        assert!(vc!(dev, 36, fn(u64, u32, *const Guid, *mut *mut c_void) -> i32, 0, 0, &IID_FENCE, &mut fence) >= 0);
        let vs = tombox_xshader::dxc::compile(&format!("{RS}{VS}"), "vs_6_0").expect("vs");
        let ps = tombox_xshader::dxc::compile(PS, "ps_6_0").expect("ps");
        let mut rs: *mut c_void = core::ptr::null_mut();
        assert!(vc!(dev, DEV_CREATE_ROOT_SIGNATURE, fn(u32, *const u8, usize, *const Guid, *mut *mut c_void) -> i32, 0, vs.as_ptr(), vs.len(), &IID_ROOT_SIGNATURE, &mut rs) >= 0);
        let mut cc = [0u8; 24];
        cc[0..4].copy_from_slice(&FMT_RGBA8.to_le_bytes());
        let rt = committed(dev, 1, 3, W as u64, H, FMT_RGBA8, 1, 4, Some(cc), samples, 1);
        let mut dc = [0u8; 24];
        dc[0..4].copy_from_slice(&FMT_D24S8.to_le_bytes());
        dc[4..8].copy_from_slice(&1.0f32.to_le_bytes());
        let ds = committed(dev, 1, 3, W as u64, H, FMT_D24S8, 2, 0x10, Some(dc), samples, 1);
        let rtv = heap_start(dev, 2, 1);
        let dsv = heap_start(dev, 3, 1);
        vc!(dev, 20, fn(*mut c_void, *const c_void, usize) -> (), rt as *mut c_void, core::ptr::null(), rtv);
        vc!(dev, 21, fn(*mut c_void, *const c_void, usize) -> (), ds as *mut c_void, core::ptr::null(), dsv);
        note_dsv(dsv, None);
        let qhd = [0u32, 16, 0];
        let mut qheap: *mut c_void = core::ptr::null_mut();
        assert!(vc!(dev, DEV_CREATE_QUERY_HEAP, fn(*const u32, *const Guid, *mut *mut c_void) -> i32, qhd.as_ptr(), &IID_QUERY_HEAP, &mut qheap) >= 0);
        let rb = committed(dev, 3, 1, 256, 1, 0, 0, ST_COPY_DEST, None, 1, 1);
        let lvt = *(list as *const *const usize);
        let vt: Vec<AtomicUsize> = (0..128).map(|_| AtomicUsize::new(0)).collect();
        let map = [(10usize, 10usize), (12, 12), (13, 13), (20, 20), (21, 21), (22, 22), (24, 24), (25, 25), (27, 27), (28, 52), (29, 53), (30, 54), (31, 55), (34, 30), (40, 36), (46, 42), (50, 46), (59, 59), (68, 62)];
        for (x, pc) in map {
            vt[x].store(*lvt.add(pc), Ordering::Relaxed);
        }
        vt[X_DRAW_INDEXED_X].store(t_draw_indexed_x as usize, Ordering::Relaxed);
        vt[X_DISPATCH_MESH].store(t_dispatch_mesh as usize, Ordering::Relaxed);
        Kit { dev, queue: queue as usize, alloc: alloc as usize, list: list as usize, fence: fence as usize, value: 0, rs: rs as usize, rtv, dsv, qheap: qheap as usize, rb, vt, _vs: vs, _ps: ps }
    }
}

unsafe extern "system" fn t_draw_indexed_x(this: *mut c_void, va: u64, fmt: u32, n: u32, inst: u32) {
    let mut view = [0u8; 16];
    view[0..8].copy_from_slice(&va.to_le_bytes());
    view[8..12].copy_from_slice(&(n * 4).to_le_bytes());
    view[12..16].copy_from_slice(&fmt.to_le_bytes());
    unsafe {
        vc!(this, PC_IB, fn(*const u8) -> (), view.as_ptr());
        vc!(this, PC_DRAW_INDEXED, fn(u32, u32, u32, i32, u32) -> (), n, inst, 0, 0, 0);
    }
}

unsafe extern "system" fn t_dispatch_mesh(this: *mut c_void, x: u32, y: u32, z: u32) {
    unsafe { crate::mesh_pso::dispatch_mesh(this, x, y, z) }
}

unsafe fn x<F: Copy>(k: &Kit, slot: usize) -> F {
    let p = k.vt[slot].load(Ordering::Relaxed);
    unsafe { core::mem::transmute_copy(&p) }
}

unsafe fn consts(k: &Kit, c: [f32; 6]) {
    unsafe {
        let f: unsafe extern "system" fn(*mut c_void, u32, u32, *const f32, u32) = x(k, X_GCONSTS);
        f(k.list as *mut c_void, 0, 6, c.as_ptr(), 0);
    }
}

unsafe fn rect(k: &Kit, c: [f32; 6], instances: u32) {
    unsafe {
        consts(k, c);
        let d: unsafe extern "system" fn(*mut c_void, u32, u32, u32, u32) = x(k, X_DRAW);
        d(k.list as *mut c_void, 6, instances, 0, 0);
    }
}

unsafe fn setup(k: &Kit, dsv: usize) {
    let l = k.list as *mut c_void;
    unsafe {
        crate::cl2_state::on_new_list(l);
        let rt: unsafe extern "system" fn(*mut c_void, u32, *const usize, i32, *const usize) = x(k, X_RT);
        rt(l, 1, &k.rtv, 0, &dsv);
        let vp = [0f32, 0.0, W as f32, H as f32, 0.0, 1.0];
        let f: unsafe extern "system" fn(*mut c_void, u32, *const f32) = x(k, X_VIEWPORTS);
        f(l, 1, vp.as_ptr());
        let sc = [0i32, 0, W as i32, H as i32];
        let f: unsafe extern "system" fn(*mut c_void, u32, *const i32) = x(k, X_SCISSORS);
        f(l, 1, sc.as_ptr());
        let f: unsafe extern "system" fn(*mut c_void, u32) = x(k, X_TOPOLOGY);
        f(l, 4);
        let f: unsafe extern "system" fn(*mut c_void, *mut c_void) = x(k, X_GROOT);
        f(l, k.rs as *mut c_void);
        vc!(l, 47, fn(usize, u32, f32, u8, u32, *const c_void) -> (), dsv, 3, 1.0, 0, 0, core::ptr::null());
    }
}

unsafe fn stencil_ref(k: &Kit, r: u32) {
    unsafe {
        let f: unsafe extern "system" fn(*mut c_void, u32) = x(k, X_STENCIL_REF);
        f(k.list as *mut c_void, r);
    }
}

unsafe fn set_pso(k: &Kit, p: usize) {
    unsafe {
        let pso: unsafe extern "system" fn(*mut c_void, *mut c_void) = x(k, X_PSO);
        pso(k.list as *mut c_void, p as *mut c_void);
    }
}

unsafe fn submit(k: &mut Kit) {
    unsafe {
        assert!(vc!(k.list, PC_CLOSE, fn() -> i32) >= 0, "Close");
        let lists = [k.list];
        vc!(k.queue, 10, fn(u32, *const usize) -> (), 1, lists.as_ptr());
        k.value += 1;
        assert!(vc!(k.queue, 14, fn(*mut c_void, u64) -> i32, k.fence as *mut c_void, k.value) >= 0);
        let mut n = 0;
        while vc!(k.fence, 8, fn() -> u64) < k.value {
            std::thread::sleep(std::time::Duration::from_millis(1));
            n += 1;
            assert!(n < 10000, "GPU never finished (removed reason {:#010x})", vc!(k.dev, 37, fn() -> i32));
        }
        assert!(vc!(k.alloc, 8, fn() -> i32) >= 0);
        let reset: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32 = x(k, X_RESET);
        assert!(reset(k.list as *mut c_void, k.alloc as *mut c_void, core::ptr::null_mut()) >= 0);
    }
}

unsafe fn read(k: &Kit, n: usize) -> Vec<u64> {
    unsafe {
        let mut m: *mut u64 = core::ptr::null_mut();
        assert!(vc!(k.rb, 8, fn(u32, *const c_void, *mut *mut u64) -> i32, 0, core::ptr::null(), &mut m) >= 0);
        let v = core::slice::from_raw_parts(m, n).to_vec();
        vc!(k.rb, 9, fn(u32, *const c_void) -> (), 0, core::ptr::null());
        v
    }
}

struct Psos {
    wall: usize,
    mark: usize,
    probe: usize,
    overlap: usize,
}

unsafe fn scene_on(k: &Kit, p: &Psos, dsv: usize) {
    unsafe {
        setup(k, dsv);
        set_pso(k, p.wall);
        rect(k, [-1.0, -1.0, 0.0, 1.0, 0.5, 0.5], 1);
        set_pso(k, p.mark);
        stencil_ref(k, 1);
        rect(k, [-1.0, 0.0, 1.0, 1.0, 0.0, 0.0], 1);
    }
}

unsafe fn measure_with(k: &mut Kit, p: &Psos, dsv: usize, ctrl: Option<u32>, ty: u32, draw: &dyn Fn(&Kit)) -> u64 {
    let l = k.list as *mut c_void;
    unsafe {
        scene_on(k, p, dsv);
        let set: unsafe extern "system" fn(*mut c_void, *const u32) = x(k, X_SET_DB_COUNT_CONTROL);
        let begin: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, u32) = x(k, X_BEGIN_QUERY);
        let end: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, u32) = x(k, X_END_QUERY);
        let resolve: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, u32, u32, *mut c_void, u64) = x(k, X_RESOLVE_QUERY);
        set(l, ctrl.as_ref().map(|c| c as *const u32).unwrap_or(core::ptr::null()));
        begin(l, k.qheap as *mut c_void, ty, 3);
        draw(k);
        end(l, k.qheap as *mut c_void, ty, 3);
        resolve(l, k.qheap as *mut c_void, ty, 3, 1, k.rb as *mut c_void, 0);
        submit(k);
        read(k, 1)[0]
    }
}

unsafe fn measure(k: &mut Kit, p: &Psos, ctrl: Option<u32>, ty: u32, overlap: bool) -> u64 {
    let probe = p.probe;
    let ov = p.overlap;
    let dsv = k.dsv;
    unsafe {
        measure_with(k, p, dsv, ctrl, ty, &|k| {
            if overlap {
                set_pso(k, ov);
                rect(k, [0.0, -1.0, 1.0, 1.0, 0.25, 0.3], 2);
            } else {
                set_pso(k, probe);
                stencil_ref(k, 1);
                rect(k, [-1.0, -1.0, 1.0, 1.0, 0.75, 0.75], 1);
            }
        })
    }
}

fn check(bad: &mut Vec<String>, log: &mut Vec<String>, what: &str, got: u64, want: u64) {
    log.push(format!("{what}: {got}"));
    if got != want {
        bad.push(format!("{what}: got {got}, want {want}"));
    }
}

fn psos(k: &Kit, samples: u32) -> Psos {
    unsafe {
        Psos {
            wall: make_pso(k, (1, 1, LESS), None, samples),
            mark: make_pso(k, (1, 0, ALWAYS), Some((ALWAYS, REPLACE, 0xff, 0xff)), samples),
            probe: make_pso(k, (1, 1, LESS), Some((EQUAL, KEEP, 0xff, 0)), samples),
            overlap: make_pso(k, (1, 1, LESS), None, samples),
        }
    }
}

fn restore(saved: Vec<usize>, saved_mine: Vec<usize>) {
    crate::cl2_state::test_restore(saved);
    for (i, v) in saved_mine.into_iter().enumerate() {
        PREV[i].store(v, Ordering::Relaxed);
    }
}

#[test]
#[ignore = "needs a D3D12 GPU: TOMBOX_EXEC_HW=1 cargo test -p d3d12_xs --lib hw_db_count -- --ignored"]
fn hw_db_count_control_counts_every_mode_exactly() {
    if std::env::var_os("TOMBOX_EXEC_HW").is_none() {
        return;
    }
    let _g = crate::cl2_state::TEST_PREV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let saved_mine: Vec<usize> = PREV.iter().map(|p| p.load(Ordering::Relaxed)).collect();
    unsafe {
        let mut k = kit(1);
        let saved = crate::cl2_state::test_install(&k.vt);
        install(&k.vt);
        let p = psos(&k, 1);
        let cases: &[(Option<u32>, u32, bool, u64, &str)] = &[
            (None, 0, false, 1024, "default zpass"),
            (Some(0x1100_1000), 0, false, 1024, "zfail"),
            (Some(0x1101_0000), 0, false, 2048, "sfail"),
            (Some(0x1100_1100), 0, false, 2048, "zpass+zfail"),
            (Some(0x1101_1100), 0, false, 4096, "zpass+zfail+sfail"),
            (Some(0x1101_1000), 0, false, 3072, "zfail+sfail"),
            (Some(0x1100_0101), 0, false, 0, "ZPASS_INCREMENT_DISABLE"),
            (Some(0x0000_0100), 0, false, 0, "no slice enabled"),
            (Some(0x1100_0200), 0, false, 0, "counter 1 only"),
            (Some(0x1110_0100), 0, false, 1024, "dbfail without depth bounds"),
            (Some(0x1100_0120), 0, false, 4096, "fixed rate 4 samples on 1x"),
            (Some(0x1100_0170), 0, false, 1024, "auto rate"),
            (Some(0x1100_1000), 1, false, 1, "binary zfail"),
            (Some(0x1100_0101), 1, false, 0, "binary disabled"),
            (None, 0, true, 2048, "overlap zpass"),
            (Some(0x1100_1000), 0, true, 2048, "overlap zfail"),
            (Some(0x1100_1100), 0, true, 4096, "overlap zpass+zfail"),
            (Some(0x1101_0000), 0, true, 0, "overlap sfail without stencil"),
            (Some(0x0100_0100), 0, false, 1024, "even slices on slice 0"),
            (Some(0x1000_0100), 0, false, 0, "odd slices on slice 0"),
        ];
        let mut bad = Vec::new();
        let mut log = Vec::new();
        for &(ctrl, ty, ov, want, what) in cases {
            let got = measure(&mut k, &p, ctrl, ty, ov);
            check(&mut bad, &mut log, &format!("{what} control={ctrl:x?} ty={ty}"), got, want);
        }
        eprintln!("{}", log.join("\n"));
        restore(saved, saved_mine);
        assert!(bad.is_empty(), "{}", bad.join("\n"));
    }
}

#[test]
#[ignore = "needs a D3D12 GPU: TOMBOX_EXEC_HW=1 cargo test -p d3d12_xs --lib hw_db_count -- --ignored"]
fn hw_db_count_sample_rate_on_4x_msaa() {
    if std::env::var_os("TOMBOX_EXEC_HW").is_none() {
        return;
    }
    let _g = crate::cl2_state::TEST_PREV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let saved_mine: Vec<usize> = PREV.iter().map(|p| p.load(Ordering::Relaxed)).collect();
    unsafe {
        let mut k = kit(4);
        let saved = crate::cl2_state::test_install(&k.vt);
        install(&k.vt);
        let p = psos(&k, 4);
        let cases: &[(Option<u32>, u64, &str)] = &[
            (Some(0x1100_0170), 4096, "auto rate tracks 4x"),
            (Some(0x1100_0100), 1024, "fixed rate 1 sample"),
            (Some(0x1100_0110), 2048, "fixed rate 2 samples"),
            (Some(0x1100_0130), 8192, "fixed rate 8 samples"),
            (Some(0x1100_1070), 4096, "auto rate zfail"),
            (Some(0x1101_1100), 4096, "fixed rate 1 sample, all categories"),
            (Some(0x1101_1120), 16384, "fixed rate 4 samples, all categories"),
        ];
        let mut bad = Vec::new();
        let mut log = Vec::new();
        for &(ctrl, want, what) in cases {
            let got = measure(&mut k, &p, ctrl, 0, false);
            check(&mut bad, &mut log, &format!("{what} control={ctrl:x?}"), got, want);
        }
        let l = k.list as *mut c_void;
        let set: unsafe extern "system" fn(*mut c_void, *const u32) = x(&k, X_SET_DB_COUNT_CONTROL);
        set(l, core::ptr::null());
        let null_rate = list_count(l);
        scene_on(&k, &p, k.dsv);
        let after_same = list_count(l);
        let one = make_pso(&k, (1, 1, LESS), None, 1);
        set_pso(&k, one);
        set_pso(&k, p.wall);
        let after_change = list_count(l);
        submit(&mut k);
        eprintln!("{}\nnull: {null_rate:x?} same-rate PSO: {after_same:x?} changed: {after_change:x?}", log.join("\n"));
        restore(saved, saved_mine);
        assert!(bad.is_empty(), "{}", bad.join("\n"));
        assert_eq!(null_rate.control, 0x1100_0100);
        assert_eq!(after_same.control, 0x1100_0100);
        assert_eq!(after_change.control, 0x1100_0120);
    }
}

unsafe fn bounds(k: &Kit, lo: f32, hi: f32) {
    unsafe {
        let f: unsafe extern "system" fn(*mut c_void, f32, f32) = x(k, X_DEPTH_BOUNDS);
        f(k.list as *mut c_void, lo, hi);
    }
}

unsafe fn uav_count(k: &mut Kit, uav: usize) -> u64 {
    let l = k.list as *mut c_void;
    unsafe {
        crate::cl2_state::on_new_list(l);
        crate::fill::barrier_pub(l, uav, ST_UAV, ST_COPY_SOURCE);
        vc!(l, PC_COPY_BUFFER, fn(*mut c_void, u64, *mut c_void, u64, u64) -> (), k.rb as *mut c_void, 0, uav as *mut c_void, 0, 8);
        crate::fill::barrier_pub(l, uav, ST_COPY_SOURCE, ST_UAV);
        submit(k);
        read(k, 1)[0] & 0xffff_ffff
    }
}

#[test]
#[ignore = "needs a D3D12 GPU: TOMBOX_EXEC_HW=1 cargo test -p d3d12_xs --lib hw_db_count -- --ignored"]
fn hw_db_count_replays_every_draw_path() {
    if std::env::var_os("TOMBOX_EXEC_HW").is_none() {
        return;
    }
    let _g = crate::cl2_state::TEST_PREV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let saved_mine: Vec<usize> = PREV.iter().map(|p| p.load(Ordering::Relaxed)).collect();
    unsafe {
        let mut k = kit(1);
        let saved = crate::cl2_state::test_install(&k.vt);
        install(&k.vt);
        assert!(chain(&k.vt, X_DISPATCH_MESH, w_dispatch_mesh as usize));
        let p = psos(&k, 1);
        let mut bad = Vec::new();
        let mut log = Vec::new();
        let probe_ds = |b: Option<bool>| ds_block((1, 1, LESS), Some((EQUAL, KEEP, 0xff, 0)), b);
        let vs = k._vs.clone();
        let ps = k._ps.clone();
        let bpso = make_stream_pso(&k, (SUB_VS, &vs), &ps, probe_ds(Some(true)));
        for (ctrl, want, what) in [
            (None, 0u64, "bounds default"),
            (Some(0x1110_0000), 2048, "bounds dbfail"),
            (Some(0x1101_0000), 1024, "bounds sfail"),
            (Some(0x1100_1000), 1024, "bounds zfail"),
            (Some(0x1111_1100), 4096, "bounds all categories"),
            (Some(0x1110_0120), 8192, "bounds dbfail at rate 4"),
        ] {
            let dsv = k.dsv;
            let got = measure_with(&mut k, &p, dsv, ctrl, 0, &|k| {
                set_pso(k, bpso);
                stencil_ref(k, 1);
                bounds(k, 0.0, 0.75);
                rect(k, [-1.0, -1.0, 1.0, 1.0, 0.75, 0.75], 1);
            });
            check(&mut bad, &mut log, what, got, want);
        }
        let spso = make_stream_pso(&k, (SUB_VS, &vs), &ps, probe_ds(None));
        for (ctrl, want, what) in [(Some(0x1100_1000), 1024u64, "stream DS zfail"), (Some(0x1101_0000), 2048, "stream DS sfail"), (Some(0x1110_0000), 0, "stream DS dbfail")] {
            let dsv = k.dsv;
            let got = measure_with(&mut k, &p, dsv, ctrl, 0, &|k| {
                set_pso(k, spso);
                stencil_ref(k, 1);
                rect(k, [-1.0, -1.0, 1.0, 1.0, 0.75, 0.75], 1);
            });
            check(&mut bad, &mut log, what, got, want);
        }
        let ms = tombox_xshader::dxc::compile(&format!("{RS}{MS}"), "ms_6_5").expect("ms");
        let mpso = make_stream_pso(&k, (SUB_MS, &ms), &ps, probe_ds(Some(false)));
        for (ctrl, want, what) in [(None, 1024u64, "mesh zpass"), (Some(0x1100_1000), 1024, "mesh zfail"), (Some(0x1101_0000), 2048, "mesh sfail"), (Some(0x1101_1100), 4096, "mesh all")] {
            let dsv = k.dsv;
            let got = measure_with(&mut k, &p, dsv, ctrl, 0, &|k| {
                set_pso(k, mpso);
                stencil_ref(k, 1);
                consts(k, [-1.0, -1.0, 1.0, 1.0, 0.75, 0.75]);
                let f: unsafe extern "system" fn(*mut c_void, u32, u32, u32) = x(k, X_DISPATCH_MESH);
                f(k.list as *mut c_void, 1, 1, 1);
            });
            check(&mut bad, &mut log, what, got, want);
        }
        let idx: Vec<u8> = (0u32..6).flat_map(|i| i.to_le_bytes()).collect();
        let (_ib, ib_va) = upload(k.dev, &idx);
        for (ctrl, want, what) in [(Some(0x1100_1000), 1024u64, "DrawIndexedX zfail"), (Some(0x1101_1000), 3072, "DrawIndexedX zfail+sfail")] {
            let dsv = k.dsv;
            let probe = p.probe;
            let got = measure_with(&mut k, &p, dsv, ctrl, 0, &|k| {
                set_pso(k, probe);
                stencil_ref(k, 1);
                consts(k, [-1.0, -1.0, 1.0, 1.0, 0.75, 0.75]);
                let f: unsafe extern "system" fn(*mut c_void, u64, u32, u32, u32) = x(k, X_DRAW_INDEXED_X);
                f(k.list as *mut c_void, ib_va, 42, 6, 1);
            });
            check(&mut bad, &mut log, what, got, want);
        }
        let arg = [0u32, 0, 0, 0];
        let sd: [u64; 3] = [16 | (1u64 << 32), arg.as_ptr() as u64, 0];
        let mut sig: *mut c_void = core::ptr::null_mut();
        assert!(vc!(k.dev, DEV_CREATE_CMD_SIG, fn(*const u64, *mut c_void, *const Guid, *mut *mut c_void) -> i32, sd.as_ptr(), core::ptr::null_mut(), &IID_CMD_SIG, &mut sig) >= 0 && !sig.is_null());
        let args: Vec<u8> = [6u32, 1, 0, 0].iter().flat_map(|v| v.to_le_bytes()).collect();
        let (argbuf, _) = upload(k.dev, &args);
        for (ctrl, want, what) in [(Some(0x1100_1000), 1024u64, "ExecuteIndirect zfail"), (Some(0x1101_0000), 2048, "ExecuteIndirect sfail"), (Some(0x1100_0120), 4096, "ExecuteIndirect rate 4")] {
            let dsv = k.dsv;
            let probe = p.probe;
            let sig = sig as usize;
            let got = measure_with(&mut k, &p, dsv, ctrl, 0, &|k| {
                set_pso(k, probe);
                stencil_ref(k, 1);
                consts(k, [-1.0, -1.0, 1.0, 1.0, 0.75, 0.75]);
                let f: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, *mut c_void, u64, *mut c_void, u64) = x(k, X_EXEC_INDIRECT);
                f(k.list as *mut c_void, sig as *mut c_void, 1, argbuf as *mut c_void, 0, core::ptr::null_mut(), 0);
            });
            check(&mut bad, &mut log, what, got, want);
        }
        let mut balloc: *mut c_void = core::ptr::null_mut();
        assert!(vc!(k.dev, 9, fn(u32, *const Guid, *mut *mut c_void) -> i32, 1, &IID_ALLOCATOR, &mut balloc) >= 0);
        let mut bundle: *mut c_void = core::ptr::null_mut();
        assert!(vc!(k.dev, 12, fn(u32, u32, *mut c_void, *mut c_void, *const Guid, *mut *mut c_void) -> i32, 0, 1, balloc, p.probe as *mut c_void, &IID_LIST, &mut bundle) >= 0);
        vc!(bundle, PC_TOPOLOGY, fn(u32) -> (), 4);
        vc!(bundle, PC_DRAW, fn(u32, u32, u32, u32) -> (), 6, 1, 0, 0);
        assert!(vc!(bundle, PC_CLOSE, fn() -> i32) >= 0);
        for (ctrl, want, what) in [(None, 1024u64, "bundle zpass"), (Some(0x1100_0120), 4096, "bundle rate 4"), (Some(0x0100_0100), 1024, "bundle even slice")] {
            let dsv = k.dsv;
            let probe = p.probe;
            let b = bundle as usize;
            let got = measure_with(&mut k, &p, dsv, ctrl, 0, &|k| {
                set_pso(k, probe);
                stencil_ref(k, 1);
                consts(k, [-1.0, -1.0, 1.0, 1.0, 0.75, 0.75]);
                let f: unsafe extern "system" fn(*mut c_void, *mut c_void) = x(k, X_EXEC_BUNDLE);
                f(k.list as *mut c_void, b as *mut c_void);
            });
            check(&mut bad, &mut log, what, got, want);
        }
        let uav_src = format!("{RS}{UAV_PS}").replace("float4 main()", "[RootSignature(RS)] float4 main()");
        let ups = std::sync::Arc::new(tombox_xshader::dxc::compile(&uav_src, "ps_6_0").expect("uav ps"));
        note_ps_hlsl(&ups, &uav_src, "ps_6_0");
        let upso = make_pso_with(&k, ups.clone(), (1, 1, LESS), Some((EQUAL, KEEP, 0xff, 0)), 1);
        let uav = committed(k.dev, 1, 1, 256, 1, 0, 4, ST_UAV, None, 1, 1);
        let uav_va = vc!(uav, RES_GET_VA, fn() -> u64);
        let mut runs = Vec::new();
        for (ctrl, want, what) in [(None, 1024u64, "UAV PS zpass"), (Some(0x1100_1000), 1024, "UAV PS zfail"), (Some(0x1101_1100), 4096, "UAV PS all categories")] {
            let before = uav_count(&mut k, uav);
            let dsv = k.dsv;
            let got = measure_with(&mut k, &p, dsv, ctrl, 0, &|k| {
                set_pso(k, upso);
                stencil_ref(k, 1);
                let f: unsafe extern "system" fn(*mut c_void, u32, u64) = x(k, X_GUAV);
                f(k.list as *mut c_void, 1, uav_va);
                rect(k, [-1.0, -1.0, 1.0, 1.0, 0.75, 0.75], 1);
            });
            let after = uav_count(&mut k, uav);
            runs.push(after - before);
            check(&mut bad, &mut log, what, got, want);
        }
        log.push(format!("UAV PS invocations per measurement: {runs:?}"));
        if runs.iter().any(|&r| r != runs[0]) || runs[0] == 0 {
            bad.push(format!("UAV stores repeated by counting replays: {runs:?}"));
        }
        let mut dc = [0u8; 24];
        dc[0..4].copy_from_slice(&FMT_D24S8.to_le_bytes());
        dc[4..8].copy_from_slice(&1.0f32.to_le_bytes());
        let arr = committed(k.dev, 1, 3, W as u64, H, FMT_D24S8, 2, 0x10, Some(dc), 1, 2);
        let heap = heap_start(k.dev, 3, 2);
        let inc = vc!(k.dev, DEV_INCREMENT_SIZE, fn(u32) -> u32, 3) as usize;
        for s in 0..2u32 {
            let mut d = [0u8; 24];
            d[0..4].copy_from_slice(&FMT_D24S8.to_le_bytes());
            d[4..8].copy_from_slice(&4u32.to_le_bytes());
            d[16..20].copy_from_slice(&s.to_le_bytes());
            d[20..24].copy_from_slice(&1u32.to_le_bytes());
            let h = heap + s as usize * inc;
            vc!(k.dev, DEV_CREATE_DSV, fn(*mut c_void, *const u8, usize) -> (), arr as *mut c_void, d.as_ptr(), h);
            note_dsv(h, Some(&d));
        }
        for (slice, ctrl, want, what) in [
            (0usize, Some(0x0100_0100u32), 1024u64, "array slice 0 even"),
            (0, Some(0x1000_0100), 0, "array slice 0 odd"),
            (1, Some(0x0100_0100), 0, "array slice 1 even"),
            (1, Some(0x1000_0100), 1024, "array slice 1 odd"),
            (1, Some(0x1100_1000), 1024, "array slice 1 both zfail"),
            (1, Some(0x1001_0000), 2048, "array slice 1 odd sfail"),
        ] {
            let dsv = heap + slice * inc;
            let probe = p.probe;
            let got = measure_with(&mut k, &p, dsv, ctrl, 0, &|k| {
                set_pso(k, probe);
                stencil_ref(k, 1);
                rect(k, [-1.0, -1.0, 1.0, 1.0, 0.75, 0.75], 1);
            });
            check(&mut bad, &mut log, what, got, want);
        }
        eprintln!("{}", log.join("\n"));
        restore(saved, saved_mine);
        assert!(bad.is_empty(), "{}", bad.join("\n"));
    }
}

#[test]
#[ignore = "needs a D3D12 GPU: TOMBOX_EXEC_HW=1 cargo test -p d3d12_xs --lib hw_db_count -- --ignored"]
fn hw_db_count_pause_lifts_and_restores_a_predicate_set_by_buffer_and_advance() {
    if std::env::var_os("TOMBOX_EXEC_HW").is_none() {
        return;
    }
    let _g = crate::cl2_state::TEST_PREV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let saved_mine: Vec<usize> = PREV.iter().map(|p| p.load(Ordering::Relaxed)).collect();
    unsafe {
        let mut k = kit(1);
        let saved = crate::cl2_state::test_install(&k.vt);
        install(&k.vt);
        let p = psos(&k, 1);
        let mut words = [0u8; 16];
        words[0..8].copy_from_slice(&1u64.to_le_bytes());
        let (pr, pva) = upload(k.dev, &words);
        crate::gpu_va::insert(crate::gpu_va::Entry { start: pva, len: 256, res: pr, buffer: true });
        let probe = p.probe;
        let dsv = k.dsv;
        let seen = std::cell::Cell::new(None);
        let got = measure_with(&mut k, &p, dsv, Some(0x1100_1000), 0, &|k| {
            let l = k.list as *mut c_void;
            crate::cl_fw::set_predication_buffer(l, &crate::cl_fw::PredicationBufferDesc { va: pva, offset: 0 });
            set_pso(k, probe);
            stencil_ref(k, 1);
            rect(k, [-1.0, -1.0, 1.0, 1.0, 0.75, 0.75], 1);
            crate::cl_fw::advance_predication(l);
            set_pso(k, probe);
            stencil_ref(k, 1);
            rect(k, [-1.0, -1.0, 1.0, 1.0, 0.75, 0.75], 1);
            seen.set(crate::cl2_state::predication_of(l));
            crate::cl_fw::set_predication_buffer(l, core::ptr::null());
        });
        restore(saved, saved_mine);
        assert_eq!(seen.get(), Some((pr, 8, 0)), "the list state does not hold the advanced predicate");
        assert_eq!(got, 1024, "zfail samples of the draw under the nonzero predicate only: 2048 means the zero predicate at buffer+8 was not restored after the counting pause, less means the partial result copy was not lifted out of predication");
    }
}

#[test]
#[ignore = "needs a D3D12 GPU: TOMBOX_EXEC_HW=1 cargo test -p d3d12_xs --lib hw_db_count -- --ignored"]
fn hw_query_x_depth_stencil_and_nested_occlusion() {
    if std::env::var_os("TOMBOX_EXEC_HW").is_none() {
        return;
    }
    let _g = crate::cl2_state::TEST_PREV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let saved_mine: Vec<usize> = PREV.iter().map(|p| p.load(Ordering::Relaxed)).collect();
    unsafe {
        let mut k = kit(1);
        let lvt = *(k.list as *const *const usize);
        let saved = crate::cl2_state::test_install(&k.vt);
        crate::query_x::install(&k.vt, lvt);
        let p = psos(&k, 1);
        let l = k.list as *mut c_void;

        let begin: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, u32) = x(&k, X_BEGIN_QUERY);
        let end: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, u32) = x(&k, X_END_QUERY);
        let resolve: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, u32, u32, *mut c_void, u64) = x(&k, X_RESOLVE_QUERY);

        // Control point scene:
        // 64x64 buffer (4096 pixels).
        // Wall: X [-1, 0], Y [-1, 1], depth 0.5 (2048 px)
        // Mark: X [-1, 1], Y [0, 1], stencil 1 (2048 px)
        // Probe: X [-1, 1], Y [-1, 1], depth 0.75, depth func LESS, stencil func EQUAL 1.
        // Exactly 1024 pixels pass (Top-Right quadrant: depth 0.75 < 1.0, stencil 1 == 1).
        // 1024 pixels fail depth test only (Top-Left quadrant: depth 0.75 >= 0.5, stencil 1 == 1).
        // 1024 pixels fail stencil test only (Bottom-Right quadrant: depth 0.75 < 1.0, stencil 0 != 1).
        // 1024 pixels fail both (Bottom-Left quadrant: depth 0.75 >= 0.5, stencil 0 != 1).

        // Test 1: Single un-nested query with depth/stencil control points
        scene_on(&k, &p, k.dsv);
        begin(l, k.qheap as *mut c_void, 0, 0);
        set_pso(&k, p.probe);
        stencil_ref(&k, 1);
        rect(&k, [-1.0, -1.0, 1.0, 1.0, 0.75, 0.75], 1);
        end(l, k.qheap as *mut c_void, 0, 0);
        resolve(l, k.qheap as *mut c_void, 0, 0, 1, k.rb as *mut c_void, 0);
        submit(&mut k);
        let single_res = read(&k, 1)[0];
        eprintln!("[SYNTHETIC_TEST1] Single un-nested query samples_passed = {single_res} (control point: 1024 passing, 3072 failing)");
        assert_eq!(single_res, 1024, "depth/stencil control points failed: expected 1024 passing samples");

        // Test 2: Nested occlusion queries: outer query (slot 0) and inner query (slot 1)
        // Outer query begins -> Inner query begins -> Draw probe -> Inner ends -> Outer ends
        scene_on(&k, &p, k.dsv);
        begin(l, k.qheap as *mut c_void, 0, 0); // Begin outer
        begin(l, k.qheap as *mut c_void, 0, 1); // Begin inner (unnest_enabled: ends outer on PC command list!)
        set_pso(&k, p.probe);
        stencil_ref(&k, 1);
        rect(&k, [-1.0, -1.0, 1.0, 1.0, 0.75, 0.75], 1); // Draw inside inner
        end(l, k.qheap as *mut c_void, 0, 1);   // End inner
        end(l, k.qheap as *mut c_void, 0, 0);   // End outer (swallowed!)
        resolve(l, k.qheap as *mut c_void, 0, 0, 2, k.rb as *mut c_void, 0);
        submit(&mut k);
        let nested_res = read(&k, 2);
        let outer_res = nested_res[0];
        let inner_res = nested_res[1];
        eprintln!("[SYNTHETIC_TEST2] Nested queries: outer samples_passed = {outer_res}, inner samples_passed = {inner_res}");
        assert_eq!(inner_res, 1024, "inner query failed to record geometry inside it");
        if std::env::var("TOMBOX_QUERY_UNNEST").map(|v| v != "0").unwrap_or(true) {
            // When un-nesting is enabled in query_x, outer query was ended early with 0 draws, so it returns 0!
            assert_eq!(outer_res, 0, "outer query was expected to be 0 due to early un-nest termination");
        } else {
            // When un-nesting is disabled, GPU native driver counts both queries: outer gets 1024!
            assert_eq!(outer_res, 1024, "native GPU driver counts outer query as 1024");
        }

        // Test 3: Outer query has draw BEFORE inner query begins
        scene_on(&k, &p, k.dsv);
        begin(l, k.qheap as *mut c_void, 0, 0); // Begin outer
        set_pso(&k, p.wall);
        rect(&k, [-1.0, -1.0, 0.0, 1.0, 0.2, 0.2], 1); // Draw 1 before inner: 2048 px pass
        begin(l, k.qheap as *mut c_void, 0, 1); // Begin inner (outer ended here!)
        set_pso(&k, p.probe);
        stencil_ref(&k, 1);
        rect(&k, [-1.0, -1.0, 1.0, 1.0, 0.75, 0.75], 1); // Draw 2 inside inner: 1024 px pass
        end(l, k.qheap as *mut c_void, 0, 1);   // End inner
        end(l, k.qheap as *mut c_void, 0, 0);   // End outer
        resolve(l, k.qheap as *mut c_void, 0, 0, 2, k.rb as *mut c_void, 0);
        submit(&mut k);
        let predraw_res = read(&k, 2);
        eprintln!("[SYNTHETIC_TEST3] Pre-draw nested: outer = {}, inner = {}", predraw_res[0], predraw_res[1]);

        // Test 4: Draw AFTER inner query ends, but before outer query ends
        scene_on(&k, &p, k.dsv);
        begin(l, k.qheap as *mut c_void, 0, 0); // Begin outer
        begin(l, k.qheap as *mut c_void, 0, 1); // Begin inner (outer ended here!)
        set_pso(&k, p.probe);
        stencil_ref(&k, 1);
        rect(&k, [-1.0, -1.0, 1.0, 1.0, 0.75, 0.75], 1); // Draw 1 inside inner: 1024 px pass
        end(l, k.qheap as *mut c_void, 0, 1);   // End inner
        set_pso(&k, p.wall);
        rect(&k, [-1.0, -1.0, 0.0, 1.0, 0.2, 0.2], 1); // Draw 2 after inner: 2048 px pass
        end(l, k.qheap as *mut c_void, 0, 0);   // End outer
        resolve(l, k.qheap as *mut c_void, 0, 0, 2, k.rb as *mut c_void, 0);
        submit(&mut k);
        let postdraw_res = read(&k, 2);
        eprintln!("[SYNTHETIC_TEST4] Post-draw nested: outer = {}, inner = {}", postdraw_res[0], postdraw_res[1]);

        restore(saved, saved_mine);
    }
}
