use core::ffi::c_void;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use crate::device_wrap::Guid;

const IID_DEVICE: Guid = Guid { d1: 0x1898_19f1, d2: 0x1db6, d3: 0x4b57, d4: [0xbe, 0x54, 0x18, 0x21, 0x33, 0x9b, 0x85, 0xf7] };
const IID_QUEUE: Guid = Guid { d1: 0x0ec8_70a6, d2: 0x5d7e, d3: 0x4c22, d4: [0x8c, 0xfc, 0x5b, 0xaa, 0xe0, 0x76, 0x16, 0xed] };
const IID_ALLOCATOR: Guid = Guid { d1: 0x6102_dee4, d2: 0xaf59, d3: 0x4b09, d4: [0xb9, 0x99, 0xb4, 0x4d, 0x73, 0xf0, 0x9b, 0x24] };
const IID_LIST: Guid = Guid { d1: 0x5b16_0d0f, d2: 0xac1b, d3: 0x4185, d4: [0x8b, 0xa8, 0xb3, 0xae, 0x42, 0xa5, 0xa4, 0x55] };
const IID_RESOURCE: Guid = Guid { d1: 0x6964_42be, d2: 0xa72e, d3: 0x4059, d4: [0xbc, 0x79, 0x5b, 0x5c, 0x98, 0x04, 0x0f, 0xad] };
const IID_FENCE: Guid = Guid { d1: 0x0a75_3dcf, d2: 0xc4d8, d3: 0x4b91, d4: [0xad, 0xf6, 0xbe, 0x5a, 0x60, 0xd9, 0x5a, 0x76] };
const IID_ROOT_SIGNATURE: Guid = Guid { d1: 0xc54a_6b66, d2: 0x72df, d3: 0x4ee8, d4: [0x8b, 0xe5, 0xa9, 0x46, 0xa1, 0x42, 0x92, 0x14] };
const IID_PSO: Guid = Guid { d1: 0x765a_30f3, d2: 0xf624, d3: 0x4c6f, d4: [0xa8, 0x28, 0xac, 0xe9, 0x48, 0x62, 0x24, 0x45] };
const IID_FACTORY4: Guid = Guid { d1: 0x1bc6_ea02, d2: 0xef36, d3: 0x464f, d4: [0xbf, 0x0c, 0x21, 0xca, 0x39, 0xe5, 0x16, 0x8a] };
const IID_ADAPTER: Guid = Guid { d1: 0x2411_e7e1, d2: 0x12ac, d3: 0x4ccf, d4: [0xbd, 0x14, 0x97, 0x98, 0xe8, 0x53, 0x4d, 0xc0] };

const ST_COMMON: u32 = 0;
const ST_COPY_DEST: u32 = 0x400;
const ST_COPY_SOURCE: u32 = 0x800;
const ST_GENERIC_READ: u32 = 0xac3;
const HEAP_DEFAULT: u32 = 1;
const HEAP_UPLOAD: u32 = 2;
const HEAP_READBACK: u32 = 3;
const FLAG_UAV: u32 = 4;
const FUNC_EQUAL: u32 = 3;
const FUNC_GREATER_EQUAL: u32 = 7;

const X_CLOSE: usize = 9;
const X_DISPATCH: usize = 14;
const X_COPY_BUFFER_REGION: usize = 15;
const X_SET_PSO: usize = 25;
const X_BARRIER: usize = 26;
const X_SET_COMPUTE_ROOT_SIGNATURE: usize = 33;
const X_SET_COMPUTE_ROOT_CONSTANT: usize = 37;
const PC_SET_COMPUTE_ROOT_SIGNATURE: usize = 29;
const PC_SET_COMPUTE_ROOT_UAV: usize = 41;

extern "system" {
    fn QueryPerformanceCounter(v: *mut i64) -> i32;
    fn QueryPerformanceFrequency(v: *mut i64) -> i32;
    fn LoadLibraryW(p: *const u16) -> *mut c_void;
    fn GetProcAddress(h: *mut c_void, n: *const u8) -> *mut c_void;
}

unsafe fn vs(o: *mut c_void, i: usize) -> usize {
    unsafe { *(*(o as *const *const usize)).add(i) }
}

macro_rules! call {
    ($o:expr, $i:expr, ($($t:ty),*) -> $r:ty $(, $a:expr)*) => {{
        let o = $o as *mut c_void;
        let f: unsafe extern "system" fn(*mut c_void $(, $t)*) -> $r = core::mem::transmute(vs(o, $i));
        f(o $(, $a)*)
    }};
}

struct Gpu {
    dev: usize,
    a: usize,
    b: usize,
    warp: bool,
}

unsafe impl Send for Gpu {}
unsafe impl Sync for Gpu {}

fn adapter_is_warp() -> bool {
    std::env::var("TOMBOX_HW_ADAPTER").map(|v| v.eq_ignore_ascii_case("warp")).unwrap_or(false)
}

unsafe fn warp_adapter() -> *mut c_void {
    let name: Vec<u16> = "dxgi.dll\0".encode_utf16().collect();
    let h = unsafe { LoadLibraryW(name.as_ptr()) };
    assert!(!h.is_null());
    let p = unsafe { GetProcAddress(h, b"CreateDXGIFactory2\0".as_ptr()) };
    assert!(!p.is_null());
    let create: unsafe extern "system" fn(u32, *const Guid, *mut *mut c_void) -> i32 = unsafe { core::mem::transmute(p) };
    let mut f: *mut c_void = core::ptr::null_mut();
    assert!(unsafe { create(0, &IID_FACTORY4, &mut f) } >= 0);
    let mut a: *mut c_void = core::ptr::null_mut();
    assert!(call!(f, 27, (*const Guid, *mut *mut c_void) -> i32, &IID_ADAPTER, &mut a) >= 0);
    a
}

unsafe fn queue(dev: *mut c_void) -> usize {
    let desc = [0u32; 4];
    let mut q: *mut c_void = core::ptr::null_mut();
    assert!(call!(dev, 8, (*const u32, *const Guid, *mut *mut c_void) -> i32, desc.as_ptr(), &IID_QUEUE, &mut q) >= 0);
    let f16 = unsafe { vs(q, 16) };
    let f17 = unsafe { vs(q, 17) };
    crate::sys_ts::note_queue(q, f16, f17);
    crate::present::apply_queue_vtable(q);
    q as usize
}

fn gpu() -> &'static Gpu {
    static G: OnceLock<Gpu> = OnceLock::new();
    G.get_or_init(|| unsafe {
        crate::enable_experimental_shader_models();
        let warp = adapter_is_warp();
        let adapter = if warp { warp_adapter() } else { core::ptr::null_mut() };
        let p = crate::host_proc(b"D3D12CreateDevice\0");
        let create: extern "system" fn(*mut c_void, u32, *const Guid, *mut *mut c_void) -> i32 = core::mem::transmute(p);
        let mut dev: *mut c_void = core::ptr::null_mut();
        let hr = create(adapter, 0xb000, &IID_DEVICE, &mut dev);
        assert!(hr >= 0 && !dev.is_null(), "D3D12CreateDevice ({}) hr={hr:#010x}", if warp { "WARP" } else { "hardware" });
        Gpu { dev: dev as usize, a: queue(dev), b: queue(dev), warp }
    })
}

struct Buf {
    res: usize,
    va: u64,
    cpu: *mut u8,
}

unsafe fn buffer(dev: usize, heap: u32, size: u64, flags: u32, state: u32) -> Buf {
    let h: [u32; 5] = [heap, 0, 0, 1, 1];
    let mut d = [0u8; 56];
    d[0..4].copy_from_slice(&1u32.to_le_bytes());
    d[0x10..0x18].copy_from_slice(&size.to_le_bytes());
    d[0x18..0x1c].copy_from_slice(&1u32.to_le_bytes());
    d[0x1c..0x1e].copy_from_slice(&1u16.to_le_bytes());
    d[0x1e..0x20].copy_from_slice(&1u16.to_le_bytes());
    d[0x24..0x28].copy_from_slice(&1u32.to_le_bytes());
    d[0x2c..0x30].copy_from_slice(&1u32.to_le_bytes());
    d[0x30..0x34].copy_from_slice(&flags.to_le_bytes());
    let mut r: *mut c_void = core::ptr::null_mut();
    let hr = call!(dev, 27, (*const u32, u32, *const u8, u32, *const c_void, *const Guid, *mut *mut c_void) -> i32,
        h.as_ptr(), 0, d.as_ptr(), state, core::ptr::null(), &IID_RESOURCE, &mut r);
    assert!(hr >= 0, "CreateCommittedResource hr={hr:#010x}");
    let va = call!(r, 11, () -> u64);
    crate::gpu_va::insert(crate::gpu_va::Entry { start: va, len: size, res: r as usize, buffer: true });
    crate::gpu_va::set_state(r as usize, state);
    let mut cpu: *mut u8 = core::ptr::null_mut();
    if heap != HEAP_DEFAULT {
        assert!(call!(r, 8, (u32, *const c_void, *mut *mut u8) -> i32, 0, core::ptr::null(), &mut cpu) >= 0);
    }
    Buf { res: r as usize, va, cpu }
}

unsafe fn raw_list(dev: usize) -> *mut c_void {
    let mut a: *mut c_void = core::ptr::null_mut();
    assert!(call!(dev, 9, (u32, *const Guid, *mut *mut c_void) -> i32, 0, &IID_ALLOCATOR, &mut a) >= 0);
    let mut l: *mut c_void = core::ptr::null_mut();
    assert!(call!(dev, 12, (u32, u32, *mut c_void, *mut c_void, *const Guid, *mut *mut c_void) -> i32, 0, 0, a, core::ptr::null_mut(), &IID_LIST, &mut l) >= 0);
    l
}

#[test]
#[ignore = "requires a D3D12 adapter"]
fn hw_placed_creation_without_output_uses_the_xbox_result() {
    let g = gpu();
    unsafe {
        call!(g.dev, 1, () -> u32);
        let dev = crate::device_wrap::wrap_device(g.dev as *mut c_void);
        let iid_heap = Guid { d1: 0x6b3b_2502, d2: 0x6e51, d3: 0x45b3, d4: [0x90, 0xee, 0x98, 0x84, 0x26, 0x5e, 0x8d, 0xf3] };
        let mut heap = core::ptr::null_mut();
        let mut heap_desc = [0u64; 6];
        heap_desc[0] = 0x2_0000;
        heap_desc[1] = HEAP_DEFAULT as u64;
        heap_desc[2] = 1u64 << 32;
        heap_desc[3] = 1;
        heap_desc[5] = 0xc0; // ALLOW_ONLY_BUFFERS.
        assert_eq!(call!(dev, 28, (*const u64, *const Guid, *mut *mut c_void) -> i32,
            heap_desc.as_ptr(), &iid_heap, &mut heap), 0);

        let mut desc = [0u8; 56];
        desc[..4].copy_from_slice(&1u32.to_le_bytes());
        desc[0x10..0x18].copy_from_slice(&0x100u64.to_le_bytes());
        desc[0x18..0x1c].copy_from_slice(&1u32.to_le_bytes());
        desc[0x1c..0x20].copy_from_slice(&0x0001_0001u32.to_le_bytes());
        desc[0x24..0x28].copy_from_slice(&1u32.to_le_bytes());
        desc[0x2c..0x30].copy_from_slice(&1u32.to_le_bytes());
        let create = |heap, offset, desc: &[u8; 56], out| call!(dev, 29,
            (*mut c_void, u64, *const u8, u32, *const c_void, *const Guid, *mut *mut c_void) -> i32,
            heap, offset, desc.as_ptr(), ST_COMMON, core::ptr::null(), &IID_RESOURCE, out);
        let create_x = |addr, desc: &[u8; 56]| call!(dev, 44,
            (u64, *const u8, u32, *const c_void, *const Guid, *mut *mut c_void) -> i32,
            addr, desc.as_ptr(), ST_COMMON, core::ptr::null(), &IID_RESOURCE, core::ptr::null_mut());
        let no_output = core::ptr::null_mut();
        // Check a plain placed allocation, then the emulated sub-page buffer path.
        assert_eq!(create(heap, 0, &desc, no_output), 0);
        assert_eq!(create(heap, 0x200, &desc, no_output), 0);
        assert!(create(heap, 0x2_0000, &desc, no_output) < 0);

        extern "system" {
            fn VirtualAlloc(address: *mut c_void, size: usize, alloc_type: u32, protect: u32) -> *mut c_void;
            fn VirtualFree(address: *mut c_void, size: usize, free_type: u32) -> i32;
        }
        let base = VirtualAlloc(core::ptr::null_mut(), 0x2_0000, 0x2000, 1);
        assert!(!base.is_null());
        let va = base as u64 + 0x200;
        let placements = crate::placed::count();
        assert_eq!(create_x(va, &desc), 0);
        assert_eq!(create(core::ptr::null_mut(), va, &desc, no_output), 0);
        assert_eq!(crate::placed::count(), placements);
        assert!(crate::gpu_va::lookup(va).is_none());
        assert_ne!(VirtualFree(base, 0, 0x8000), 0);
        call!(heap, 2, () -> u32);
        call!(dev, 2, () -> u32);
    }
}

unsafe fn title_list(dev: usize) -> *mut c_void {
    crate::cmdlist_wrap::wrap(unsafe { raw_list(dev) })
}

unsafe fn fence(dev: usize) -> *mut c_void {
    let mut f: *mut c_void = core::ptr::null_mut();
    assert!(call!(dev, 36, (u64, u32, *const Guid, *mut *mut c_void) -> i32, 0, 0, &IID_FENCE, &mut f) >= 0);
    f
}

unsafe fn submit(q: usize, list: *mut c_void, f: *mut c_void, v: u64) {
    let l = [list];
    call!(q, 10, (u32, *const *mut c_void) -> (), 1, l.as_ptr());
    call!(q, 14, (*mut c_void, u64) -> i32, f, v);
}

fn reached(f: *mut c_void, v: u64, within: Duration) -> bool {
    let t = Instant::now();
    loop {
        if unsafe { call!(f, 8, () -> u64) } >= v {
            return true;
        }
        if t.elapsed() > within {
            return false;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

unsafe fn rd(b: &Buf, off: usize) -> u32 {
    unsafe { core::ptr::read_volatile(b.cpu.add(off) as *const u32) }
}

unsafe fn compute_pso(dev: usize, hlsl: &str, rs: Option<*mut c_void>) -> (*mut c_void, *mut c_void) {
    let code = tombox_xshader::dxc::compile(hlsl, "cs_6_0").expect("dxc");
    let rs = rs.unwrap_or_else(|| {
        let mut r: *mut c_void = core::ptr::null_mut();
        assert!(call!(dev, 16, (u32, *const u8, usize, *const Guid, *mut *mut c_void) -> i32, 0, code.as_ptr(), code.len(), &IID_ROOT_SIGNATURE, &mut r) >= 0);
        r
    });
    let mut d = [0u8; 56];
    d[0..8].copy_from_slice(&(rs as u64).to_le_bytes());
    d[8..16].copy_from_slice(&(code.as_ptr() as u64).to_le_bytes());
    d[16..24].copy_from_slice(&(code.len() as u64).to_le_bytes());
    let mut pso: *mut c_void = core::ptr::null_mut();
    let hr = call!(dev, 11, (*const u8, *const Guid, *mut *mut c_void) -> i32, d.as_ptr(), &IID_PSO, &mut pso);
    assert!(hr >= 0, "CreateComputePipelineState hr={hr:#010x}");
    (rs, pso)
}

fn chain_len(shim: *mut c_void) -> usize {
    crate::list_split::expand(&[crate::cmdlist_wrap::unwrap(shim)]).len()
}

unsafe fn waiter(g: &Gpu, flag: &Buf, func: u32, value: u32, out: &Buf) -> *mut c_void {
    let w = unsafe { title_list(g.dev) };
    call!(w, crate::cl_ext::CL_WRITE32_BOTTOM_OF_PIPE, (u64, u32, u32, u32) -> (), out.va, 1, 0, 0);
    call!(w, crate::cl_ext::CL_WAIT32, (u64, u32, u32, u32, u32) -> (), flag.va, func, value, 0xffff_ffff, 0);
    call!(w, X_COPY_BUFFER_REGION, (usize, u64, usize, u64, u64) -> (), out.res, 8, flag.res, 0, 4);
    call!(w, crate::cl_ext::CL_WRITE32_BOTTOM_OF_PIPE, (u64, u32, u32, u32) -> (), out.va + 16, 2, 0, 0);
    assert!(call!(w, X_CLOSE, () -> i32) >= 0);
    w
}

#[test]
#[ignore = "needs a D3D12 device; run: cargo test -p d3d12_xs --lib hw_ -- --ignored --test-threads=1 (TOMBOX_HW_ADAPTER=warp for WARP)"]
fn hw_mid_list_wait_orders_after_a_value_written_by_another_list() {
    let g = gpu();
    unsafe {
        let flag = buffer(g.dev, HEAP_DEFAULT, 256, FLAG_UAV, ST_COMMON);
        let out = buffer(g.dev, HEAP_READBACK, 256, 0, ST_COPY_DEST);
        let w = waiter(g, &flag, FUNC_GREATER_EQUAL, 42, &out);
        assert_eq!(chain_len(w), 2, "the wait did not split the list");
        let fa = fence(g.dev);
        submit(g.a, w, fa, 1);
        assert!(!reached(fa, 1, Duration::from_millis(300)), "the waiter ran past the wait before the value was written");
        assert_eq!(rd(&out, 0), 1, "the part before the wait did not execute");
        assert_eq!(rd(&out, 16), 0);
        let p = title_list(g.dev);
        call!(p, crate::cl_ext::CL_WRITE32_BOTTOM_OF_PIPE, (u64, u32, u32, u32) -> (), flag.va, 42, 0, 0);
        assert!(call!(p, X_CLOSE, () -> i32) >= 0);
        let fb = fence(g.dev);
        submit(g.b, p, fb, 1);
        assert!(reached(fb, 1, Duration::from_secs(5)));
        assert!(reached(fa, 1, Duration::from_secs(5)), "the waiter never resumed after the producer wrote the value");
        assert_eq!((rd(&out, 0), rd(&out, 8), rd(&out, 16)), (1, 42, 2));
    }
}

#[test]
#[ignore = "needs a D3D12 device; run: cargo test -p d3d12_xs --lib hw_ -- --ignored --test-threads=1 (TOMBOX_HW_ADAPTER=warp for WARP)"]
fn hw_mid_list_wait_after_its_producer_was_submitted_uses_the_address_fence() {
    let g = gpu();
    unsafe {
        let flag = buffer(g.dev, HEAP_DEFAULT, 256, FLAG_UAV, ST_COMMON);
        let out = buffer(g.dev, HEAP_READBACK, 256, 0, ST_COPY_DEST);
        let p = title_list(g.dev);
        call!(p, crate::cl_ext::CL_WRITE32_BOTTOM_OF_PIPE, (u64, u32, u32, u32) -> (), flag.va, 42, 0, 0);
        assert!(call!(p, X_CLOSE, () -> i32) >= 0);
        let fb = fence(g.dev);
        submit(g.b, p, fb, 1);
        let w = waiter(g, &flag, FUNC_GREATER_EQUAL, 42, &out);
        assert_eq!(chain_len(w), 2, "the wait did not split the list");
        let before = crate::wait_probe::armed_total();
        let fa = fence(g.dev);
        submit(g.a, w, fa, 1);
        assert_eq!(crate::wait_probe::armed_total(), before, "a probe was armed although the producing write was already submitted");
        assert!(reached(fb, 1, Duration::from_secs(5)));
        assert!(reached(fa, 1, Duration::from_secs(5)), "the waiter never resumed");
        assert_eq!((rd(&out, 0), rd(&out, 8), rd(&out, 16)), (1, 42, 2));
    }
}

#[test]
#[ignore = "needs a D3D12 device; run: cargo test -p d3d12_xs --lib hw_ -- --ignored --test-threads=1 (TOMBOX_HW_ADAPTER=warp for WARP)"]
fn hw_mid_list_wait_on_a_shader_written_default_heap_value() {
    let g = gpu();
    unsafe {
        let flag = buffer(g.dev, HEAP_DEFAULT, 256, FLAG_UAV, ST_COMMON);
        let out = buffer(g.dev, HEAP_READBACK, 256, 0, ST_COPY_DEST);
        let w = waiter(g, &flag, FUNC_EQUAL, 77, &out);
        assert_eq!(chain_len(w), 2);
        let fa = fence(g.dev);
        submit(g.a, w, fa, 1);
        assert!(!reached(fa, 1, Duration::from_millis(300)), "the waiter ran past a wait on a value no shader had written yet");
        assert_eq!((rd(&out, 0), rd(&out, 16)), (1, 0));
        let (rs, pso) = compute_pso(g.dev, "#define RS \"UAV(u0)\"\nRWByteAddressBuffer o : register(u0);\n[RootSignature(RS)]\n[numthreads(1, 1, 1)]\nvoid main() { o.Store(0, 77u); }\n", None);
        let p = raw_list(g.dev);
        call!(p, PC_SET_COMPUTE_ROOT_SIGNATURE, (*mut c_void) -> (), rs);
        call!(p, X_SET_PSO, (*mut c_void) -> (), pso);
        call!(p, PC_SET_COMPUTE_ROOT_UAV, (u32, u64) -> (), 0, flag.va);
        call!(p, X_DISPATCH, (u32, u32, u32) -> (), 1, 1, 1);
        assert!(call!(p, X_CLOSE, () -> i32) >= 0);
        let fb = fence(g.dev);
        submit(g.b, p, fb, 1);
        assert!(reached(fb, 1, Duration::from_secs(5)));
        assert!(reached(fa, 1, Duration::from_secs(5)), "the waiter never resumed after the shader wrote the value");
        assert_eq!((rd(&out, 0), rd(&out, 8), rd(&out, 16)), (1, 77, 2));
    }
}

#[test]
#[ignore = "needs a D3D12 device; run: cargo test -p d3d12_xs --lib hw_ -- --ignored --test-threads=1 (TOMBOX_HW_ADAPTER=warp for WARP)"]
fn hw_gds_round_trip_through_the_shader_binding() {
    let g = gpu();
    unsafe {
        let mut param = [0u8; 32];
        param[0..4].copy_from_slice(&1u32.to_le_bytes());
        param[16..20].copy_from_slice(&1u32.to_le_bytes());
        let mut desc = [0u8; 40];
        desc[0..4].copy_from_slice(&1u32.to_le_bytes());
        desc[8..16].copy_from_slice(&(param.as_ptr() as u64).to_le_bytes());
        let (mut blob, mut err): (*mut c_void, *mut c_void) = (core::ptr::null_mut(), core::ptr::null_mut());
        assert!(crate::D3D12SerializeRootSignature(desc.as_ptr() as *const c_void, 1, &mut blob, &mut err) >= 0);
        let bp = call!(blob, 3, () -> *const u8);
        let bl = call!(blob, 4, () -> usize);
        let (patched, idx) = crate::gds::patch_root_signature(bp, bl).expect("GDS root parameter not appended");
        assert_eq!(idx, 1);
        let mut rs: *mut c_void = core::ptr::null_mut();
        assert!(call!(g.dev, 16, (u32, *const u8, usize, *const Guid, *mut *mut c_void) -> i32, 0, patched.as_ptr(), patched.len(), &IID_ROOT_SIGNATURE, &mut rs) >= 0);
        crate::gds::note_root_signature(rs, idx);
        let hlsl = format!("{}\ncbuffer C : register(b0) {{ uint add; }};\n[numthreads(64, 1, 1)]\nvoid main() {{ uint o; tb_gds.InterlockedAdd(16, add, o); }}\n", tombox_xshader::gds_abi::TB_GDS_HLSL_DECL);
        let (_, pso) = compute_pso(g.dev, &hlsl, Some(rs));
        let src = buffer(g.dev, HEAP_UPLOAD, 256, 0, ST_GENERIC_READ);
        core::ptr::write_volatile(src.cpu as *mut u32, 1000);
        core::ptr::write_volatile(src.cpu.add(4) as *mut u32, 5);
        let dst = buffer(g.dev, HEAP_DEFAULT, 256, 0, ST_COPY_DEST);
        let out = buffer(g.dev, HEAP_READBACK, 256, 0, ST_COPY_DEST);
        let l = title_list(g.dev);
        call!(l, crate::cl_ext::CL_WRITE_GDS, (u32, u32, u64, u32) -> (), 4, 2, src.va, 0);
        call!(l, X_SET_COMPUTE_ROOT_SIGNATURE, (*mut c_void) -> (), rs);
        call!(l, X_SET_PSO, (*mut c_void) -> (), pso);
        call!(l, X_SET_COMPUTE_ROOT_CONSTANT, (u32, u32, u32) -> (), 0, 3, 0);
        call!(l, X_DISPATCH, (u32, u32, u32) -> (), 4, 1, 1);
        call!(l, crate::cl_ext::CL_READ_GDS, (u32, u32, u64, u32) -> (), 4, 2, dst.va, 0);
        let mut b = [0u8; 32];
        b[8..16].copy_from_slice(&(dst.res as u64).to_le_bytes());
        b[16..20].copy_from_slice(&0xffff_ffffu32.to_le_bytes());
        b[20..24].copy_from_slice(&ST_COPY_DEST.to_le_bytes());
        b[24..28].copy_from_slice(&ST_COPY_SOURCE.to_le_bytes());
        call!(l, X_BARRIER, (u32, *const u8) -> (), 1, b.as_ptr());
        call!(l, X_COPY_BUFFER_REGION, (usize, u64, usize, u64, u64) -> (), out.res, 0, dst.res, 0, 8);
        assert!(call!(l, X_CLOSE, () -> i32) >= 0);
        let f = fence(g.dev);
        submit(g.a, l, f, 1);
        assert!(reached(f, 1, Duration::from_secs(5)));
        assert_eq!((rd(&out, 0), rd(&out, 4)), (1000 + 256 * 3, 5));
    }
}

fn gws_shader_fixture(opcode: u32, resource: u32) -> Vec<u8> {
    gws_shader_code(&[
        0x7e00_0289, // v_mov_b32 v0, 9 (next barrier count)
        0xd800_0000 | (opcode << 18) | (1 << 17) | resource, 0,
        0xbf81_0000,
    ])
}

fn gws_shader_code(isa: &[u32]) -> Vec<u8> {
    gws_shader_code_with_shape(isa, [32, 1, 1])
}

fn gws_shader_code_with_shape(isa: &[u32], [x, y, z]: [u32; 3]) -> Vec<u8> {
    let preamble = [16, x, y, z, 0xd580_0000, 0x03fe_0000, 0];
    let section = preamble.into_iter().chain(isa.iter().copied()).flat_map(u32::to_le_bytes).collect::<Vec<_>>();
    let mut fssm = vec![0u8; 0x100];
    fssm[..4].copy_from_slice(b"FSSM");
    fssm[0xc0..0xc4].copy_from_slice(&0x100u32.to_le_bytes());
    fssm[0xc8..0xcc].copy_from_slice(&(section.len() as u32).to_le_bytes());
    fssm.extend_from_slice(&section);
    let mut dxil = vec![0u8; 24];
    dxil[..4].copy_from_slice(&0x0005_0060u32.to_le_bytes());
    dxil.extend_from_slice(b"XDAT");
    dxil.extend_from_slice(&1u32.to_le_bytes());
    dxil.extend_from_slice(&miniz_oxide::deflate::compress_to_vec_zlib(&fssm, 6));
    let mut dxbc = vec![0u8; 44];
    dxbc[..4].copy_from_slice(b"DXBC");
    dxbc[20..24].copy_from_slice(&1u32.to_le_bytes());
    dxbc[24..28].copy_from_slice(&(44 + dxil.len() as u32).to_le_bytes());
    dxbc[28..32].copy_from_slice(&1u32.to_le_bytes());
    dxbc[32..36].copy_from_slice(&36u32.to_le_bytes());
    dxbc[36..40].copy_from_slice(b"DXIL");
    dxbc[40..44].copy_from_slice(&(dxil.len() as u32).to_le_bytes());
    dxbc.extend_from_slice(&dxil);
    dxbc
}

unsafe fn gws_test_signature(dev: *mut c_void, opcode: u32, stride: u32) -> *mut c_void {
    const IID_SIGNATURE: Guid = Guid {
        d1: 0xc36a797c, d2: 0xec80, d3: 0x4f0a,
        d4: [0x89, 0x85, 0xa7, 0xb2, 0x47, 0x50, 0x82, 0xd1],
    };
    let args = [opcode, 0, 0, 0];
    let desc = [u64::from(stride) | (1 << 32), args.as_ptr() as u64, 0];
    let mut signature = core::ptr::null_mut();
    let hr = call!(dev, 41, (*const u64, *mut c_void, *const Guid, *mut *mut c_void) -> i32,
        desc.as_ptr(), core::ptr::null_mut(), &IID_SIGNATURE, &mut signature);
    assert!(hr >= 0 && !signature.is_null(), "command signature hr={hr:#010x}");
    signature
}

#[test]
#[ignore = "needs D3D12 and DXC; set TOMBOX_HW_ADAPTER=warp for WARP"]
fn hw_gws_indirect_signature_preserves_gpu_execution_time_arguments() {
    let g = gpu();
    unsafe {
        let dev = crate::device_wrap::wrap_device(g.dev as *mut c_void);
        let mut signature = core::ptr::null_mut();
        for (opcode, stride, expected) in [(0, 16, None), (2, 12, Some(12)), (105, 16, Some(16))] {
            signature = gws_test_signature(dev, opcode, stride);
            assert_eq!(crate::gws_dispatch::indirect_stride(signature), expected);
            if opcode != 105 {
                call!(signature, 2, () -> u32);
            }
        }

        let hlsl = r#"
            RWByteAddressBuffer output : register(u0);
            [RootSignature("UAV(u0)")]
            [numthreads(1,1,1)]
            void main(uint3 gid : SV_GroupID) {
                uint old;
                output.InterlockedMax(0, gid.x + 1, old);
                output.InterlockedMax(4, gid.y + 1, old);
                output.InterlockedMax(8, gid.z + 1, old);
            }
        "#;
        let (root, pso) = compute_pso(g.dev, hlsl, None);
        let args = buffer(g.dev, HEAP_UPLOAD, 256, 0, ST_GENERIC_READ);
        let indirect_args = buffer(g.dev, HEAP_DEFAULT, 256, 0, ST_COPY_DEST);
        let output = buffer(g.dev, HEAP_DEFAULT, 256, FLAG_UAV, ST_COPY_DEST);
        let readback = buffer(g.dev, HEAP_READBACK, 256, 0, ST_COPY_DEST);
        args.cpu.add(64).write_bytes(0, 12);

        let list = title_list(g.dev);
        let raw = crate::cmdlist_wrap::unwrap(list);
        call!(list, X_COPY_BUFFER_REGION, (usize, u64, usize, u64, u64) -> (), indirect_args.res, 16, args.res, 16, 20);
        const INDIRECT_ARGUMENT: u32 = 0x200;
        crate::fill::barrier_pub(raw, indirect_args.res, ST_COPY_DEST, INDIRECT_ARGUMENT);
        call!(list, X_COPY_BUFFER_REGION, (usize, u64, usize, u64, u64) -> (), output.res, 0, args.res, 64, 12);
        crate::fill::barrier_pub(raw, output.res, ST_COPY_DEST, 8);
        call!(list, X_SET_COMPUTE_ROOT_SIGNATURE, (*mut c_void) -> (), root);
        call!(list, X_SET_PSO, (*mut c_void) -> (), pso);
        call!(list, X_SET_COMPUTE_ROOT_UAV, (u32, u64) -> (), 0, output.va);
        call!(list, 59, (*mut c_void, u32, usize, u64, usize, u64) -> (), signature, 1, indirect_args.res, 16, indirect_args.res, 32);
        crate::fill::barrier_pub(raw, indirect_args.res, INDIRECT_ARGUMENT, ST_COPY_DEST);
        crate::fill::barrier_pub(raw, output.res, 8, ST_COPY_SOURCE);
        call!(list, X_COPY_BUFFER_REGION, (usize, u64, usize, u64, u64) -> (), readback.res, 0, output.res, 0, 12);
        crate::fill::barrier_pub(raw, output.res, ST_COPY_SOURCE, ST_COPY_DEST);
        assert!(call!(list, X_CLOSE, () -> i32) >= 0);

        let f = fence(g.dev);
        for (i, (groups, count, expected)) in [
            ([3u32, 2, 1], 1u32, [3, 2, 1]),
            ([4, 1, 1], 1, [4, 1, 1]),
            ([2, 1, 1], 0, [0, 0, 0]),
            ([0, 1, 1], 1, [0, 0, 0]),
        ].into_iter().enumerate() {
            args.cpu.add(16).cast::<[u32; 3]>().write(groups);
            args.cpu.add(32).cast::<u32>().write(count);
            let value = i as u64 + 1;
            submit(g.a, list, f, value);
            assert!(reached(f, value, Duration::from_secs(5)));
            assert_eq!([rd(&readback, 0), rd(&readback, 4), rd(&readback, 8)], expected);
        }
        for obj in [signature, list, f, pso, root] {
            call!(obj, 2, () -> u32);
        }
    }
}


#[test]
#[ignore = "needs D3D12 and DXC with TOMBOX_GWS_PHASED=1 and TOMBOX_GWS_RESUME=1"]
fn hw_gws_resume_between_closed_lists() {
    if !crate::gws_dispatch::phased_enabled()
        || std::env::var("TOMBOX_GWS_RESUME").as_deref() != Ok("1") {
        return;
    }
    let g = gpu();
    unsafe {
        let dev = crate::device_wrap::wrap_device(g.dev as *mut c_void);
        let code = tombox_xshader::dxc::compile(r#"[RootSignature("UAV(u0), RootConstants(num32BitConstants=2, b0)")] [numthreads(1,1,1)] void main() {}"#, "cs_6_0").unwrap();
        let mut root = core::ptr::null_mut();
        assert!(call!(dev, 16, (u32, *const u8, usize, *const Guid, *mut *mut c_void) -> i32,
            0, code.as_ptr(), code.len(), &IID_ROOT_SIGNATURE, &mut root) >= 0);
        let create_with_root = |root: *mut c_void, shader: Vec<u8>| {
            let mut desc = [0u8; 56];
            desc[..8].copy_from_slice(&(root as u64).to_le_bytes());
            desc[8..16].copy_from_slice(&(shader.as_ptr() as u64).to_le_bytes());
            desc[16..24].copy_from_slice(&(shader.len() as u64).to_le_bytes());
            let mut pso = core::ptr::null_mut();
            assert!(call!(dev, 11, (*const u8, *const Guid, *mut *mut c_void) -> i32,
                desc.as_ptr(), &IID_PSO, &mut pso) >= 0);
            pso
        };
        let create = |shader| create_with_root(root, shader);
        use tombox_xshader::gds_abi::TB_GWS_TABLE_DWORD;
        const GWS_INDEX: u32 = 31;
        let ds = |op: u32| 0xd802_0000u32 | (op << 18) | GWS_INDEX;
        let make_waiter = |marker: u32| create(gws_shader_code(&[
            0xbefc_03ff, 0x400,
            0x7e28_0281, 0x7e2a_0280,
            ds(0x1c), 0,
            0xd802_0000 | marker, 21 | (20 << 8),
            0xbf81_0000,
        ]));
        let waiter = make_waiter(0x90);
        let younger = make_waiter(0x94);
        let unrelated = create(gws_shader_code(&[0xbf81_0000]));
        let poster = create(gws_shader_fixture(0x1a, GWS_INDEX));
        let releaser = create(gws_shader_fixture(0x18, GWS_INDEX));
        let participant = create(gws_shader_code(&[
            0xbefc_03ff, 0x400,
            0x7e28_0281, 0x7e2a_0280,
            ds(0x1d), 20,
            0xd802_0090, 21 | (20 << 8),
            0xbf81_0000,
        ]));
        let src = buffer(g.dev, HEAP_UPLOAD, 256, 0, ST_GENERIC_READ);
        src.cpu.cast::<[u32; 2]>().write([0, 1]);
        let output = buffer(g.dev, HEAP_DEFAULT, 256, 0, ST_COPY_DEST);
        let readback = buffer(g.dev, HEAP_READBACK, 256, 0, ST_COPY_DEST);
        let make_reader = |offset| {
            let list = title_list(g.dev);
            let raw = crate::cmdlist_wrap::unwrap(list);
            crate::gds::copy(raw, false, 0x90 / 4, 2, output.va + offset, "read cross-list markers");
            crate::fill::barrier_pub(raw, output.res, ST_COPY_DEST, ST_COPY_SOURCE);
            call!(list, X_COPY_BUFFER_REGION, (usize, u64, usize, u64, u64) -> (),
                readback.res, offset, output.res, offset, 8);
            crate::fill::barrier_pub(raw, output.res, ST_COPY_SOURCE, ST_COPY_DEST);
            assert!(call!(list, X_CLOSE, () -> i32) >= 0);
            raw
        };
        let record_with_target = |pso, groups, seed, target: Option<u64>| {
            let list = title_list(g.dev);
            let raw = crate::cmdlist_wrap::unwrap(list);
            if seed {
                let initial_count = if pso == participant { src.va + 4 } else { src.va };
                crate::gds::copy(raw, true, TB_GWS_TABLE_DWORD + GWS_INDEX + 1, 1, src.va, "clear producer barrier");
                crate::gds::copy(raw, true, TB_GWS_TABLE_DWORD + GWS_INDEX, 1, initial_count, "seed cross-list counter");
                crate::gds::copy(raw, true, 0x90 / 4, 1, src.va, "clear older cross-list marker");
                crate::gds::copy(raw, true, 0x94 / 4, 1, src.va, "clear younger cross-list marker");
            }
            call!(list, X_SET_COMPUTE_ROOT_SIGNATURE, (*mut c_void) -> (), root);
            call!(list, X_SET_COMPUTE_ROOT_CONSTANT, (u32, u32, u32) -> (), 1, if seed { 19 } else { 43 }, 0);
            if let Some(va) = target {
                call!(list, 45, (u32, u64) -> (), 0, va);
            }
            call!(list, X_SET_PSO, (*mut c_void) -> (), pso);
            call!(list, X_DISPATCH, (u32, u32, u32) -> (), groups, 1, 1);
            assert!(call!(list, X_CLOSE, () -> i32) >= 0);
            list
        };
        let record = |pso, groups, seed| record_with_target(pso, groups, seed, None);
        let submit_batch = |queue: *mut c_void, lists: &[*mut c_void]| {
            call!(queue, 10, (u32, *const *mut c_void) -> (), lists.len() as u32, lists.as_ptr());
        };
        let completed = fence(g.dev);
        let mut value = 0;
        for (first, second, groups, expected) in [
            (waiter, unrelated, 2, 0),
            (waiter, poster, 2, 32),
            (waiter, releaser, 2, 64),
            (participant, participant, 1, 64),
        ] {
            let queue = queue(g.dev as *mut c_void) as *mut c_void;
            let lists = [
                crate::cmdlist_wrap::unwrap(record(first, groups, true)),
                crate::cmdlist_wrap::unwrap(record(unrelated, 1, false)),
                crate::cmdlist_wrap::unwrap(record(second, 1, false)),
                make_reader(0),
            ];
            for _ in 0..2 {
                value += 1;
                submit_batch(queue, &lists);
                call!(queue, 14, (*mut c_void, u64) -> i32, completed, value);
                assert!(reached(completed, value, Duration::from_secs(10)));
                assert_eq!(rd(&readback, 0), expected,
                    "the reader must see exactly the waves released by the preceding list");
            }
            call!(queue, 2, () -> u32);
        }
        let timeline_queue = queue(g.dev as *mut c_void) as *mut c_void;
        let lists = [
            crate::cmdlist_wrap::unwrap(record(waiter, 1, true)),
            crate::cmdlist_wrap::unwrap(record(younger, 1, false)),
            crate::cmdlist_wrap::unwrap(record(unrelated, 1, false)),
            crate::cmdlist_wrap::unwrap(record(poster, 1, false)),
            make_reader(0),
            crate::cmdlist_wrap::unwrap(record(releaser, 1, false)),
            make_reader(8),
        ];
        submit_batch(timeline_queue, &lists);
        let queue = timeline_queue;
        call!(queue, 14, (*mut c_void, u64) -> i32, completed, value + 1);
        assert!(reached(completed, value + 1, Duration::from_secs(10)));
        assert_eq!([rd(&readback, 0), rd(&readback, 4)], [32, 0],
            "the first credit must resume only the older list");
        assert_eq!([rd(&readback, 8), rd(&readback, 12)], [32, 32],
            "release-all must resume the younger list without repeating the older wave");
        call!(queue, 2, () -> u32);

        let cooperative_release = create(gws_shader_code(&[
            0x7e28_0280,                   // v20 = 0 for the one-wave barrier
            ds(0x1d) + 1, 20,
            ds(0x18), 0,
            0xbf81_0000,
        ]));
        let separate = self::queue(g.dev as *mut c_void) as *mut c_void;
        let waiting_list = record(waiter, 2, true);
        submit_batch(separate, &[crate::cmdlist_wrap::unwrap(waiting_list)]);
        value += 2;
        call!(separate, 14, (*mut c_void, u64) -> i32, completed, value);
        assert!(reached(completed, value, Duration::from_secs(10)));

        let mut allocator = core::ptr::null_mut();
        assert!(call!(g.dev, 9, (u32, *const Guid, *mut *mut c_void) -> i32,
            0, &IID_ALLOCATOR, &mut allocator) >= 0);
        assert!(call!(waiting_list, 10, (*mut c_void, *mut c_void) -> i32,
            allocator, core::ptr::null_mut()) >= 0);
        assert!(call!(waiting_list, X_CLOSE, () -> i32) >= 0);

        let post = crate::cmdlist_wrap::unwrap(record(poster, 1, false));
        let release = record(cooperative_release, 1, false);
        for list in [post, make_reader(0), crate::cmdlist_wrap::unwrap(release), make_reader(8)] {
            submit_batch(separate, &[list]);
        }
        value += 1;
        call!(separate, 14, (*mut c_void, u64) -> i32, completed, value);
        assert!(reached(completed, value, Duration::from_secs(10)));
        assert_eq!(rd(&readback, 0), 32, "a separate post submission must wake one saved wave");
        assert_eq!(rd(&readback, 8), 64, "cooperative release-all must wake the other wave after Reset");

        let first = record(participant, 1, true);
        let second = record(participant, 1, false);
        for list in [crate::cmdlist_wrap::unwrap(first), make_reader(0),
            crate::cmdlist_wrap::unwrap(second), make_reader(8)] {
            submit_batch(separate, &[list]);
        }
        value += 1;
        call!(separate, 14, (*mut c_void, u64) -> i32, completed, value);
        assert!(reached(completed, value, Duration::from_secs(10)));
        assert_eq!(rd(&readback, 0), 0, "the first barrier dispatch must remain blocked");
        assert_eq!(rd(&readback, 8), 64, "the second submission must finish the earlier barrier");
        call!(separate, 2, () -> u32);

        let origin_queue = self::queue(g.dev as *mut c_void) as *mut c_void;
        let signal_queue = self::queue(g.dev as *mut c_void) as *mut c_void;
        let handoff = fence(g.dev);
        let waiting = record(waiter, 2, true);
        submit_batch(origin_queue, &[waiting]);
        assert!(call!(origin_queue, 14, (*mut c_void, u64) -> i32, handoff, 2) >= 0);

        // Waiting for an earlier value on the same title fence must not import
        // the pending waves. CPU observation avoids racing the device GDS.
        assert!(reached(handoff, 2, Duration::from_secs(10)));
        assert!(call!(signal_queue, 15, (*mut c_void, u64) -> i32, handoff, 1) >= 0);
        submit_batch(signal_queue, &[record(poster, 1, false), make_reader(0)]);
        value += 1;
        call!(signal_queue, 14, (*mut c_void, u64) -> i32, completed, value);
        assert!(reached(completed, value, Duration::from_secs(10)));
        assert_eq!(rd(&readback, 0), 0, "an earlier fence value must not transfer the waiter");
        submit_batch(signal_queue, &[record(unrelated, 1, true)]);

        assert!(call!(signal_queue, 15, (*mut c_void, u64) -> i32, handoff, 2) >= 0);
        submit_batch(signal_queue, &[record(poster, 1, false), make_reader(0)]);
        assert!(call!(signal_queue, 23, (*mut c_void, u64, u32) -> i32, handoff, 3, 0) >= 0);
        assert!(call!(origin_queue, 15, (*mut c_void, u64) -> i32, handoff, 3) >= 0);
        submit_batch(origin_queue, &[record(releaser, 1, false), make_reader(8)]);
        value += 1;
        call!(origin_queue, 14, (*mut c_void, u64) -> i32, completed, value);
        assert!(reached(completed, value, Duration::from_secs(10)));
        assert_eq!(rd(&readback, 0), 32, "the destination queue must resume exactly one wave");
        assert_eq!(rd(&readback, 8), 64, "the return fence must transfer the remaining waiter back");
        call!(handoff, 2, () -> u32);
        call!(origin_queue, 2, () -> u32);
        call!(signal_queue, 2, () -> u32);

        const ROOT_MARKER: u32 = 0x51ab_1e7e;
        let add_root = |mut shader: Vec<u8>, code: &[u8]| {
            let container = tombox_xshader::Dxbc::parse(code).unwrap();
            let rts = container.chunk(b"RTS0").unwrap();
            shader.splice(36..36, [0; 4]);
            let offset = shader.len() as u32;
            shader.extend_from_slice(&code[rts.offset - 8..rts.offset + rts.size]);
            let size = shader.len() as u32;
            shader[24..28].copy_from_slice(&size.to_le_bytes());
            shader[28..32].copy_from_slice(&2u32.to_le_bytes());
            shader[32..36].copy_from_slice(&40u32.to_le_bytes());
            shader[36..40].copy_from_slice(&offset.to_le_bytes());
            shader
        };
        let shader = gws_shader_code(&[
            ds(0x1c), 0,
            0xf408_0200, 0xfa00_0000, // s_load_dwordx4 s[8:11], s[0:1], 0
            0x3402_0082,              // v1 = v0 * 4
            0x7e14_02ff, ROOT_MARKER,
            0xe070_1000, 0x8002_0a01, // buffer_store_dword via the root UAV
            0xbf81_0000,
        ]);
        let root_waiter = create(add_root(shader, &code));

        let original = buffer(g.dev, HEAP_DEFAULT, 256, FLAG_UAV, 0x8);
        let other = buffer(g.dev, HEAP_DEFAULT, 256, FLAG_UAV, 0x8);
        let buffer_queue = self::queue(g.dev as *mut c_void) as *mut c_void;
        let seed = buffer(g.dev, HEAP_UPLOAD, 256, 0, ST_GENERIC_READ);
        seed.cpu.cast::<u32>().write(0x1234);
        let init = title_list(g.dev);
        let init_raw = crate::cmdlist_wrap::unwrap(init);
        for target in [&original, &other] {
            crate::fill::barrier_pub(init_raw, target.res, 0x8, ST_COPY_DEST);
            call!(init, X_COPY_BUFFER_REGION, (usize, u64, usize, u64, u64) -> (),
                target.res, 0, seed.res, 0, 4);
            crate::fill::barrier_pub(init_raw, target.res, ST_COPY_DEST, 0x8);
        }
        assert!(call!(init, X_CLOSE, () -> i32) >= 0);
        let waiting = record_with_target(root_waiter, 1, true, Some(original.va));
        submit_batch(buffer_queue, &[init, waiting]);
        value += 1;
        call!(buffer_queue, 14, (*mut c_void, u64) -> i32, completed, value);
        assert!(reached(completed, value, Duration::from_secs(10)));
        assert!(call!(waiting, 10, (*mut c_void, *mut c_void) -> i32, allocator, core::ptr::null_mut()) >= 0);
        assert!(call!(waiting, X_CLOSE, () -> i32) >= 0);
        submit_batch(buffer_queue, &[record_with_target(poster, 1, false, Some(other.va))]);

        let read = title_list(g.dev);
        let read_raw = crate::cmdlist_wrap::unwrap(read);
        for (target, offset) in [(&original, 0), (&other, 4)] {
            crate::fill::barrier_pub(read_raw, target.res, 0x8, ST_COPY_SOURCE);
            call!(read, X_COPY_BUFFER_REGION, (usize, u64, usize, u64, u64) -> (),
                readback.res, offset, target.res, 0, 4);
            crate::fill::barrier_pub(read_raw, target.res, ST_COPY_SOURCE, 0x8);
        }
        assert!(call!(read, X_CLOSE, () -> i32) >= 0);
        submit_batch(buffer_queue, &[read]);
        value += 1;
        call!(buffer_queue, 14, (*mut c_void, u64) -> i32, completed, value);
        assert!(reached(completed, value, Duration::from_secs(10)));
        assert_eq!([rd(&readback, 0), rd(&readback, 4)], [ROOT_MARKER, 0x1234],
            "the resumed wave must write through its own root UAV");

        let waiting = record_with_target(root_waiter, 1, true, Some(original.va));
        submit_batch(buffer_queue, &[init, waiting]);
        let transition = |before: u32, after: u32| {
            let list = title_list(g.dev);
            let barrier = [0, original.res as u64, u64::from(u32::MAX) | (u64::from(before) << 32), u64::from(after)];
            call!(list, X_BARRIER, (u32, *const u64) -> (), 1, barrier.as_ptr());
            assert!(call!(list, X_CLOSE, () -> i32) >= 0);
            submit_batch(buffer_queue, &[list]);
        };
        transition(0x8, ST_COPY_SOURCE);
        submit_batch(buffer_queue, &[record_with_target(poster, 1, false, Some(other.va))]);
        let blocked_read = title_list(g.dev);
        call!(blocked_read, X_COPY_BUFFER_REGION, (usize, u64, usize, u64, u64) -> (),
            readback.res, 8, original.res, 0, 4);
        assert!(call!(blocked_read, X_CLOSE, () -> i32) >= 0);
        submit_batch(buffer_queue, &[blocked_read]);

        transition(ST_COPY_SOURCE, 0x8);
        submit_batch(buffer_queue, &[record_with_target(releaser, 1, false, Some(other.va)), read]);
        value += 1;
        call!(buffer_queue, 14, (*mut c_void, u64) -> i32, completed, value);
        assert!(reached(completed, value, Duration::from_secs(10)));
        assert_eq!(rd(&readback, 8), 0x1234, "the wave must remain blocked while the UAV is unavailable");
        assert_eq!([rd(&readback, 0), rd(&readback, 4)], [ROOT_MARKER, 0x1234],
            "restoring the UAV state must let the next signal resume the original wave");
        let table_code = tombox_xshader::dxc::compile(
            r#"[RootSignature("DescriptorTable(UAV(u0))")] [numthreads(1,1,1)] void main() {}"#,
            "cs_6_0",
        ).unwrap();
        let mut table_root = core::ptr::null_mut();
        assert!(call!(dev, 16, (u32, *const u8, usize, *const Guid, *mut *mut c_void) -> i32,
            0, table_code.as_ptr(), table_code.len(), &IID_ROOT_SIGNATURE, &mut table_root) >= 0);
        let table_shader = gws_shader_code(&[
            ds(0x1c), 0,
            0xf408_0202, 0xfa00_0000, // s_load_dwordx4 s[8:11], s[4:5], 0
            0x3402_0082,
            0x7e14_02ff, ROOT_MARKER,
            0xe070_1000, 0x8002_0a01,
            0xbf81_0000,
        ]);
        let table_waiter = create_with_root(table_root, add_root(table_shader, &table_code));
        const IID_HEAP: Guid = Guid {
            d1: 0x8efb471d, d2: 0x616c, d3: 0x4f49,
            d4: [0x90, 0xf7, 0x12, 0x7b, 0xb7, 0x63, 0xfa, 0x51],
        };
        let mut heap = core::ptr::null_mut();
        assert!(call!(dev, 14, (*const u32, *const Guid, *mut *mut c_void) -> i32,
            [0, 128, 1, 0].as_ptr(), &IID_HEAP, &mut heap) >= 0);
        let inc = u64::from(call!(dev, 15, (u32) -> u32, 0));
        let cpu_handle = call!(heap, 9, () -> usize) + (96 * inc) as usize;
        let gpu_handle = call!(heap, 10, () -> u64) + 96 * inc;
        let uav = [39u32, 1, 0, 0, 64, 0, 0, 0, 1, 0];
        let write_descriptor = |resource: usize| {
            call!(dev, 19, (usize, usize, *const u32, usize) -> (),
                resource, 0, uav.as_ptr(), cpu_handle);
        };
        write_descriptor(original.res);
        let table_list = title_list(g.dev);
        let raw = crate::cmdlist_wrap::unwrap(table_list);
        crate::gds::copy(raw, true, TB_GWS_TABLE_DWORD + GWS_INDEX, 1, src.va,
            "clear descriptor waiter semaphore");
        call!(table_list, X_SET_COMPUTE_ROOT_SIGNATURE, (*mut c_void) -> (), table_root);
        call!(table_list, 32, (u32, *const *mut c_void) -> (), 1, &heap);
        call!(table_list, 35, (u32, u64) -> (), 0, gpu_handle);
        call!(table_list, X_SET_PSO, (*mut c_void) -> (), table_waiter);
        call!(table_list, X_DISPATCH, (u32, u32, u32) -> (), 1, 1, 1);
        assert!(call!(table_list, X_CLOSE, () -> i32) >= 0);
        submit_batch(buffer_queue, &[init, table_list]);
        value += 1;
        call!(buffer_queue, 14, (*mut c_void, u64) -> i32, completed, value);
        assert!(reached(completed, value, Duration::from_secs(10)));
        write_descriptor(other.res);
        assert!(call!(table_list, 10, (*mut c_void, *mut c_void) -> i32, allocator, core::ptr::null_mut()) >= 0);
        assert!(call!(table_list, X_CLOSE, () -> i32) >= 0);
        submit_batch(buffer_queue, &[record(poster, 1, false), read]);
        value += 1;
        call!(buffer_queue, 14, (*mut c_void, u64) -> i32, completed, value);
        assert!(reached(completed, value, Duration::from_secs(10)));
        assert_eq!([rd(&readback, 0), rd(&readback, 4)], [ROOT_MARKER, 0x1234],
            "the resumed wave must use the descriptor saved before the heap was overwritten");
        call!(heap, 2, () -> u32);
        call!(buffer_queue, 2, () -> u32);
    }
}

#[test]
#[ignore = "needs D3D12 and DXC; set TOMBOX_HW_ADAPTER=warp for WARP"]
fn hw_gws_compute_pso_selects_dispatch_variant() {
    const RESOURCE: u32 = 7;
    let dxbc = gws_shader_fixture(0x1c, RESOURCE);
    let post_shader = gws_shader_fixture(0x1a, RESOURCE);
    let barrier_shader = gws_shader_fixture(0x1d, RESOURCE + 1);
    let ex = tombox_xshader::extract(&dxbc).expect("Xbox fixture");
    assert_eq!(ex.isa_preamble[1..], [32, 1, 1]);
    assert!(tombox_xshader::hlsl::translate(&ex).is_err());
    crate::shader_xlat::compile_gws(&ex, 2).expect("GWS variant must compile");

    let g = gpu();
    unsafe {
        let no_op = "[numthreads(1, 1, 1)] void main() {}";
        let root_hlsl = format!("[RootSignature(\"UAV(u0, space={})\")] {no_op}", tombox_xshader::gds_abi::TB_GDS_SPACE);
        let dev = crate::device_wrap::wrap_device(g.dev as *mut c_void);
        let phased = crate::gws_dispatch::phased_enabled();
        let (rs, normal) = if phased {
            let code = tombox_xshader::dxc::compile(&format!("[RootSignature(\"\")] {no_op}"), "cs_6_0").unwrap();
            let mut rs = core::ptr::null_mut();
            let hr = call!(dev, 16, (u32, *const u8, usize, *const Guid, *mut *mut c_void) -> i32, 0, code.as_ptr(), code.len(), &IID_ROOT_SIGNATURE, &mut rs);
            assert!(hr >= 0 && !rs.is_null(), "wrapped root signature hr={hr:#010x}");
            compute_pso(g.dev, no_op, Some(rs))
        } else {
            let (rs, normal) = compute_pso(g.dev, &root_hlsl, None);
            crate::gds::note_root_signature(rs, 0);
            (rs, normal)
        };
        let resume = phased && std::env::var("TOMBOX_GWS_RESUME").as_deref() == Ok("1");
        let command_queue = if resume { queue(g.dev as *mut c_void) } else { g.a };
        let list = title_list(g.dev);
        let mut desc = [0u8; 56];
        desc[0..8].copy_from_slice(&(rs as u64).to_le_bytes());
        desc[8..16].copy_from_slice(&(dxbc.as_ptr() as u64).to_le_bytes());
        desc[16..24].copy_from_slice(&(dxbc.len() as u64).to_le_bytes());
        let mut pso: *mut c_void = core::ptr::null_mut();
        let hr = call!(dev, 11, (*const u8, *const Guid, *mut *mut c_void) -> i32, desc.as_ptr(), &IID_PSO, &mut pso);
        assert!(hr >= 0 && !pso.is_null(), "GWS PSO creation hr={hr:#010x}");

        desc[8..16].copy_from_slice(&(post_shader.as_ptr() as u64).to_le_bytes());
        desc[16..24].copy_from_slice(&(post_shader.len() as u64).to_le_bytes());
        let mut producer = core::ptr::null_mut();
        let hr = call!(dev, 11, (*const u8, *const Guid, *mut *mut c_void) -> i32, desc.as_ptr(), &IID_PSO, &mut producer);
        assert!(hr >= 0 && !producer.is_null(), "GWS producer PSO creation hr={hr:#010x}");

        desc[8..16].copy_from_slice(&(barrier_shader.as_ptr() as u64).to_le_bytes());
        desc[16..24].copy_from_slice(&(barrier_shader.len() as u64).to_le_bytes());
        let mut barrier_pso = core::ptr::null_mut();
        let hr = call!(dev, 11, (*const u8, *const Guid, *mut *mut c_void) -> i32, desc.as_ptr(), &IID_PSO, &mut barrier_pso);
        assert!(hr >= 0 && !barrier_pso.is_null(), "GWS barrier PSO creation hr={hr:#010x}");

        let src = buffer(g.dev, HEAP_UPLOAD, 256, 0, ST_GENERIC_READ);
        core::ptr::write_volatile(src.cpu as *mut u32, 0);
        let barrier_groups = if phased { 5 } else { 2 };
        core::ptr::write_volatile(src.cpu.add(4) as *mut u32, barrier_groups - 1);
        let dst = buffer(g.dev, HEAP_DEFAULT, 256, 0, ST_COPY_DEST);
        let out = buffer(g.dev, HEAP_READBACK, 256, 0, ST_COPY_DEST);
        let raw = crate::cmdlist_wrap::unwrap(list);
        let counter = tombox_xshader::gds_abi::TB_GWS_TABLE_DWORD + RESOURCE;
        // The emulated semaphore table is outside the title-visible GDS range.
        crate::gds::copy(raw, true, counter, 2, src.va, "seed GWS test");
        call!(list, X_SET_COMPUTE_ROOT_SIGNATURE, (*mut c_void) -> (), rs);
        // Credits must survive a guest dispatch boundary and a compute PSO switch.
        call!(list, X_SET_PSO, (*mut c_void) -> (), producer);
        call!(list, X_DISPATCH, (u32, u32, u32) -> (), 64, 1, 1);
        let uav_barrier = [2u64, 0, 0, 0];
        call!(list, X_BARRIER, (u32, *const u64) -> (), 1, uav_barrier.as_ptr());
        call!(list, X_SET_PSO, (*mut c_void) -> (), pso);
        for groups in [[2, 1, 1], [3, 1, 1], [1, 1, 1], [2, 2, 1], [5, 1, 1], [0, 1, 1]] {
            let [x, y, z] = groups;
            if x == 3 {
                call!(list, crate::console_slots::CL_DISPATCH_X, (u32, u32, u32, u32) -> (), x, y, z, 0);
            } else {
                call!(list, X_DISPATCH, (u32, u32, u32) -> (), x, y, z);
            }
            // No resource transition occurs between the two valid launches.
            let mut barrier = [0u8; 32];
            barrier[..4].copy_from_slice(&2u32.to_le_bytes());
            call!(list, X_BARRIER, (u32, *const u8) -> (), 1, barrier.as_ptr());
        }
        let mut indirect_inputs = None;
        if phased {
            let upload = buffer(g.dev, HEAP_UPLOAD, 256, 0, ST_GENERIC_READ);
            let commands = buffer(g.dev, HEAP_DEFAULT, 256, 0, ST_COPY_DEST);
            let signature = gws_test_signature(dev, 105, 16);
            let data = [
                2u32, 1, 1, 0, // valid DispatchX
                4, 1, 1, 0, // disabled by the count buffer
                3, 2, 1, 0, // multidimensional phased dispatch
                0, 1, 1, 0, // zero groups
            ];
            core::ptr::copy_nonoverlapping(data.as_ptr(), upload.cpu.cast(), data.len());
            upload.cpu.add(64).cast::<[u32; 4]>().write([4, 3, 2, 0]);
            core::ptr::write_volatile(upload.cpu.add(80).cast::<u32>(), 0);
            let batch = [3u32, 1, 1, 0, 5, 1, 1, 0, 2, 1, 1, 0];
            core::ptr::copy_nonoverlapping(batch.as_ptr(), upload.cpu.add(96).cast(), batch.len());
            upload.cpu.add(144).cast::<[u32; 4]>().write([1, 1, 1, 0]);
            core::ptr::write_volatile(upload.cpu.add(160).cast::<u32>(), 2);
            upload.cpu.add(176).cast::<[u32; 4]>().write([2, 1, 1, 0]);
            upload.cpu.add(192).cast::<[u32; 4]>().write([1, 1, 1, 0]);
            upload.cpu.add(208).cast::<[u32; 2]>().write([0, 1]);
            upload.cpu.add(224).cast::<[u32; 4]>().write([5, 1, 1, 0]);
            upload.cpu.add(240).cast::<[u32; 4]>().write([4, 2, 2, 0]);
            call!(list, X_COPY_BUFFER_REGION, (usize, u64, usize, u64, u64) -> (), commands.res, 0, upload.res, 0, 256);
            let mut transition = [0u64; 4];
            transition[1] = commands.res as u64;
            transition[2] = u64::from(u32::MAX) | (u64::from(ST_COPY_DEST) << 32);
            transition[3] = 0x200;
            call!(list, X_BARRIER, (u32, *const u64) -> (), 1, transition.as_ptr());
            for (offset, counted) in [(0, false), (16, true), (32, false), (48, false), (64, false), (224, false)] {
                let count = if counted { commands.res } else { 0 };
                call!(list, 59, (*mut c_void, u32, usize, u64, usize, u64) -> (),
                    signature, 1, commands.res, offset, count, 80);
                call!(list, X_BARRIER, (u32, *const u64) -> (), 1, uav_barrier.as_ptr());
            }
            call!(list, 59, (*mut c_void, u32, usize, u64, usize, u64) -> (),
                signature, 3, commands.res, 96, commands.res, 160);
            call!(list, X_BARRIER, (u32, *const u64) -> (), 1, uav_barrier.as_ptr());
            indirect_inputs = Some((upload, commands, signature));
        }
        call!(list, X_SET_PSO, (*mut c_void) -> (), barrier_pso);
        call!(list, X_DISPATCH, (u32, u32, u32) -> (), barrier_groups, 1, 1);
        if let Some((upload, commands, signature)) = &indirect_inputs {
            crate::gds::copy(raw, true, counter + 1, 1, upload.va + 192, "seed indirect GWS barrier");
            call!(list, 59, (*mut c_void, u32, usize, u64, usize, u64) -> (),
                *signature, 1, commands.res, 176, 0, 0);
        }
        if resume {
            const MARKER: u32 = 0x5ab;
            let ds = |op: u32, res: u32| 0xd802_0000 | (op << 18) | res;
            let waiter = gws_shader_code(&[
                0x7e28_02ff, MARKER,                   // v_mov_b32 v20, literal
                ds(0x1a, RESOURCE + 4), 0,            // side effect before the wait
                ds(0x1c, RESOURCE + 2), 0,
                ds(0x1b, RESOURCE + 3), 20 << 8,      // add the saved v20 after waking
                0xbf81_0000,
            ]);
            let poster = gws_shader_fixture(0x1a, RESOURCE + 2);
            let release = gws_shader_fixture(0x18, RESOURCE + 2);
            let mut create = |shader: &[u8]| {
                desc[8..16].copy_from_slice(&(shader.as_ptr() as u64).to_le_bytes());
                desc[16..24].copy_from_slice(&(shader.len() as u64).to_le_bytes());
                let mut pso = core::ptr::null_mut();
                let hr = call!(dev, 11, (*const u8, *const Guid, *mut *mut c_void) -> i32,
                    desc.as_ptr(), &IID_PSO, &mut pso);
                assert!(hr >= 0 && !pso.is_null(), "cross-dispatch fixture PSO hr={hr:#010x}");
                pso
            };
            let waiter_pso = create(&waiter);
            let poster_pso = create(&poster);
            let release_pso = create(&release);
            src.cpu.add(32).write_bytes(0, 12);
            crate::gds::copy(raw, true, counter + 2, 3, src.va + 32, "seed cross-dispatch test");
            call!(list, X_SET_PSO, (*mut c_void) -> (), waiter_pso);
            call!(list, X_DISPATCH, (u32, u32, u32) -> (), 2, 1, 1);
            let (_, commands, signature) = indirect_inputs.as_ref().unwrap();
            let unrelated = create(&gws_shader_code(&[0xbf81_0000]));
            call!(list, X_SET_PSO, (*mut c_void) -> (), unrelated);
            for _ in 0..3 {
                call!(list, X_DISPATCH, (u32, u32, u32) -> (), 1, 1, 1);
                call!(list, 59, (*mut c_void, u32, usize, u64, usize, u64) -> (),
                    *signature, 1, commands.res, 192, 0, 0);
            }
            crate::gds::copy(raw, false, counter + 3, 1, dst.va + 132, "read marker before late post");
            call!(list, X_SET_PSO, (*mut c_void) -> (), poster_pso);
            for (count_offset, readback_offset) in [(208, 20), (212, 24)] {
                call!(list, 59, (*mut c_void, u32, usize, u64, usize, u64) -> (),
                    *signature, 1, commands.res, 192, commands.res, count_offset);
                crate::gds::copy(raw, false, counter + 3, 1, dst.va + readback_offset, "read marker after indirect producer");
            }
            crate::gds::copy(raw, true, counter + 3, 1, src.va + 32, "clear indirect marker before direct producer");
            call!(list, X_DISPATCH, (u32, u32, u32) -> (), 1, 1, 1);
            crate::gds::copy(raw, false, counter + 2, 3, dst.va + 8, "read post results");

            crate::gds::copy(raw, true, counter + 2, 3, src.va + 32, "seed release-all test");
            call!(list, X_SET_PSO, (*mut c_void) -> (), release_pso);
            call!(list, X_DISPATCH, (u32, u32, u32) -> (), 1, 1, 1);
            call!(list, X_BARRIER, (u32, *const u64) -> (), 1, uav_barrier.as_ptr());
            call!(list, X_SET_PSO, (*mut c_void) -> (), waiter_pso);
            call!(list, X_DISPATCH, (u32, u32, u32) -> (), 2, 1, 1);
            crate::gds::copy(raw, false, counter + 3, 1, dst.va + 28, "read marker after early release");
            call!(list, X_SET_PSO, (*mut c_void) -> (), release_pso);
            call!(list, X_DISPATCH, (u32, u32, u32) -> (), 1, 1, 1);
            crate::gds::copy(raw, false, counter + 2, 2, dst.va + 32, "read release-all results");

            let barrier_shader = |marker, resource| gws_shader_code(&[
                0x7e28_02ff, marker,
                0x7e00_0283, // v_mov_b32 v0, 3 (four waves in the next generation)
                ds(0x1d, RESOURCE + 5), 0,
                ds(0x19, resource), 20 << 8,
                0xbf81_0000,
            ]);
            let first = create(&barrier_shader(0x321, RESOURCE + 6));
            let second = create(&barrier_shader(0x654, RESOURCE + 7));
            src.cpu.add(48).cast::<[u32; 3]>().write([1, 0, 0]);
            crate::gds::copy(raw, true, counter + 5, 3, src.va + 48, "seed two-wave cross-dispatch barrier");
            call!(list, X_SET_PSO, (*mut c_void) -> (), first);
            call!(list, X_DISPATCH, (u32, u32, u32) -> (), 1, 1, 1);
            crate::gds::copy(raw, false, counter + 6, 1, dst.va + 40, "read marker before last barrier arrival");
            call!(list, X_SET_PSO, (*mut c_void) -> (), second);
            call!(list, 59, (*mut c_void, u32, usize, u64, usize, u64) -> (),
                *signature, 1, commands.res, 192, 0, 0);
            crate::gds::copy(raw, false, counter + 6, 2, dst.va + 44, "read direct and indirect barrier participants");

            let batch_barrier = create(&gws_shader_code(&[
                0xbefc_03ff, 0x400,                 // s_mov_b32 m0, 0x400 (GDS size)
                0x7e28_0281, 0x7e2a_0280,           // v20 = 1, v21 = 0
                ds(0x1d, RESOURCE + 5), 0,
                ds(0x00, 0x80), 21 | (20 << 8),     // ds_add_u32 v21, v20 offset:0x80 gds
                0xbf81_0000,
            ]));
            src.cpu.add(64).cast::<u32>().write(7);
            crate::gds::copy(raw, true, counter + 5, 1, src.va + 64, "seed eight-wave indirect barrier");
            crate::gds::copy(raw, true, 0x80 / 4, 1, src.va + 32, "clear indirect barrier lane counter");
            call!(list, X_SET_PSO, (*mut c_void) -> (), batch_barrier);
            call!(list, 59, (*mut c_void, u32, usize, u64, usize, u64) -> (),
                *signature, 2, commands.res, 96, 0, 0);
            crate::gds::copy(raw, false, 0x80 / 4, 1, dst.va + 76, "read indirect barrier lane counter");

            for (count_offset, readback_offset) in [(208, 52), (212, 64)] {
                crate::gds::copy(raw, true, counter + 2, 3, src.va + 32, "seed indirect waiter");
                call!(list, X_SET_PSO, (*mut c_void) -> (), waiter_pso);
                call!(list, 59, (*mut c_void, u32, usize, u64, usize, u64) -> (),
                    *signature, 1, commands.res, 224, commands.res, count_offset);
                call!(list, X_SET_PSO, (*mut c_void) -> (), poster_pso);
                call!(list, X_DISPATCH, (u32, u32, u32) -> (), 1, 1, 1);
                crate::gds::copy(raw, false, counter + 2, 3, dst.va + readback_offset, "read indirect waiter");
            }

            let fifo_waiter = |output_byte| gws_shader_code(&[
                0xbefc_03ff, 0x400,
                0x7e28_0281, 0x7e2a_0280,
                ds(0x1c, RESOURCE + 2), 0,
                ds(0x00, output_byte), 21 | (20 << 8),
                0xbf81_0000,
            ]);
            let older = create(&fifo_waiter(0x84));
            let younger = create(&fifo_waiter(0x88));
            let queue_both = |direct_groups, indirect_offset| {
                crate::gds::copy(raw, true, counter + 2, 1, src.va + 32, "clear FIFO semaphore");
                crate::gds::copy(raw, true, 0x84 / 4, 2, src.va + 32, "clear FIFO lane counters");
                call!(list, X_SET_PSO, (*mut c_void) -> (), older);
                call!(list, X_DISPATCH, (u32, u32, u32) -> (), direct_groups, 1, 1);
                call!(list, X_SET_PSO, (*mut c_void) -> (), younger);
                call!(list, 59, (*mut c_void, u32, usize, u64, usize, u64) -> (),
                    *signature, 1, commands.res, indirect_offset, commands.res, 212);
            };
            queue_both(2, 224);
            call!(list, X_SET_PSO, (*mut c_void) -> (), poster_pso);
            for offset in [80, 88, 96] {
                call!(list, X_DISPATCH, (u32, u32, u32) -> (), 1, 1, 1);
                crate::gds::copy(raw, false, 0x84 / 4, 2, dst.va + offset, "read FIFO lane counters");
            }
            call!(list, X_SET_PSO, (*mut c_void) -> (), release_pso);
            call!(list, X_DISPATCH, (u32, u32, u32) -> (), 1, 1, 1);
            queue_both(2, 224);
            call!(list, X_SET_PSO, (*mut c_void) -> (), release_pso);
            call!(list, X_DISPATCH, (u32, u32, u32) -> (), 1, 1, 1);
            crate::gds::copy(raw, false, 0x84 / 4, 2, dst.va + 104, "read release-all FIFO counters");
            crate::gds::copy(raw, false, counter + 2, 1, dst.va + 112, "read credits after release-all");

            queue_both(1, 192);
            call!(list, X_SET_PSO, (*mut c_void) -> (), poster_pso);
            call!(list, X_DISPATCH, (u32, u32, u32) -> (), 1, 1, 1);
            crate::gds::copy(raw, false, 0x84 / 4, 2, dst.va + 116, "read one-group waiters after post");
            call!(list, X_SET_PSO, (*mut c_void) -> (), release_pso);
            call!(list, X_DISPATCH, (u32, u32, u32) -> (), 1, 1, 1);
            crate::gds::copy(raw, false, 0x84 / 4, 2, dst.va + 124, "read one-group waiters after release-all");

            let mixed = create(&gws_shader_code(&[
                0x7e28_0281,                  // v20 = 1
                ds(0x1d, RESOURCE + 5), 20,    // two-wave barrier
                ds(0x1c, RESOURCE + 2), 0,     // one semaphore credit for two waves
                ds(0x1b, RESOURCE + 6), 20 << 8,
                0xbf81_0000,
            ]));
            src.cpu.add(72).cast::<u32>().write(1);
            crate::gds::copy(raw, true, counter + 2, 1, src.va + 72, "seed mixed semaphore");
            crate::gds::copy(raw, true, counter + 5, 1, src.va + 72, "seed mixed barrier");
            crate::gds::copy(raw, true, counter + 6, 1, src.va + 32, "clear mixed marker");
            call!(list, X_SET_PSO, (*mut c_void) -> (), mixed);
            call!(list, X_DISPATCH, (u32, u32, u32) -> (), 2, 1, 1);
            crate::gds::copy(raw, false, counter + 6, 1, dst.va + 136, "read marker after mixed dispatch");
            call!(list, X_SET_PSO, (*mut c_void) -> (), release_pso);
            call!(list, X_DISPATCH, (u32, u32, u32) -> (), 1, 1, 1);
            crate::gds::copy(raw, false, counter + 6, 1, dst.va + 140, "read marker after mixed release");

            let cooperative_release = create(&gws_shader_code(&[
                0x7e28_0280,                  // v20 = 0
                ds(0x1d, RESOURCE + 5), 20,    // single-wave barrier
                ds(0x18, RESOURCE + 2), 0,
                0xbf81_0000,
            ]));
            for (indirect_release, snapshot) in [(false, 144), (true, 152)] {
                crate::gds::copy(raw, true, counter + 5, 1, src.va + 32, "seed cooperative release barrier");
                crate::gds::copy(raw, true, counter + 2, 1, src.va + 32, "clear cooperative release semaphore");
                crate::gds::copy(raw, true, 0x84 / 4, 1, src.va + 32, "clear cooperative release marker");
                call!(list, X_SET_PSO, (*mut c_void) -> (), older);
                call!(list, X_DISPATCH, (u32, u32, u32) -> (), 1, 1, 1);
                crate::gds::copy(raw, false, 0x84 / 4, 1, dst.va + snapshot, "read before cooperative release");
                call!(list, X_SET_PSO, (*mut c_void) -> (), cooperative_release);
                if indirect_release {
                    call!(list, 59, (*mut c_void, u32, usize, u64, usize, u64) -> (),
                        *signature, 1, commands.res, 192, 0, 0);
                } else {
                    call!(list, X_DISPATCH, (u32, u32, u32) -> (), 1, 1, 1);
                }
                crate::gds::copy(raw, false, 0x84 / 4, 1, dst.va + snapshot + 4, "read after cooperative release");
            }

            let between_rows_waiter = create(&gws_shader_code(&[
                ds(0x1c, RESOURCE + 2), 0,
                ds(0x1a, RESOURCE + 7), 0,
                ds(0x1a, RESOURCE + 8), 0,
                0xbf81_0000,
            ]));
            let resetting_release = create(&gws_shader_code(&[
                0x7e28_0280,                  // v20 = 0
                ds(0x1d, RESOURCE + 5), 20,
                ds(0x19, RESOURCE + 7), 20 << 8,
                ds(0x1a, RESOURCE + 9), 0,
                ds(0x18, RESOURCE + 2), 0,
                0xbf81_0000,
            ]));
            src.cpu.add(76).cast::<u32>().write(6);
            for (records, args_offset, count_buffer, count_offset, snapshot) in [
                (2, 176, commands.res, 208, 160),
                (2, 176, commands.res, 212, 172),
                (2, 176, 0, 0, 184),
                (4, 96, 0, 0, 196),
            ] {
                crate::gds::copy(raw, true, counter + 2, 1, src.va + 32, "clear batch semaphore");
                crate::gds::copy(raw, true, counter + 5, 1, src.va + 32, "seed batch barrier");
                crate::gds::copy(raw, true, counter + 7, 1, src.va + 76, "seed batch observation");
                crate::gds::copy(raw, true, counter + 8, 2, src.va + 32, "clear batch wake and producer counts");
                call!(list, X_SET_PSO, (*mut c_void) -> (), between_rows_waiter);
                if records == 4 {
                    call!(list, 59, (*mut c_void, u32, usize, u64, usize, u64) -> (),
                        *signature, 1, commands.res, 192, 0, 0);
                } else {
                    call!(list, X_DISPATCH, (u32, u32, u32) -> (), 1, 1, 1);
                }
                call!(list, X_SET_PSO, (*mut c_void) -> (), resetting_release);
                call!(list, 59, (*mut c_void, u32, usize, u64, usize, u64) -> (),
                    *signature, records, commands.res, args_offset, count_buffer, count_offset);
                crate::gds::copy(raw, false, counter + 7, 3, dst.va + snapshot, "read batch observation and counts");
            }

            let two_wave_waiter = create(&gws_shader_code_with_shape(&[
                0xbefc_03ff, 0x400,                 // s_mov_b32 m0, 0x400 (GDS size)
                0x7e28_0281, 0x7e2a_0280,           // v20 = 1, v21 = 0
                ds(0x1c, RESOURCE + 2), 0,
                ds(0x00, 0x8c), 21 | (20 << 8),
                0xbf81_0000,
            ], [8, 4, 2]));
            for (routed, snapshot) in [(false, 208), (true, 220)] {
                crate::gds::copy(raw, true, counter + 2, 1, src.va + 32, "clear two-wave semaphore");
                crate::gds::copy(raw, true, 0x8c / 4, 1, src.va + 32, "clear two-wave marker");
                call!(list, X_SET_PSO, (*mut c_void) -> (), two_wave_waiter);
                if routed {
                    call!(list, 59, (*mut c_void, u32, usize, u64, usize, u64) -> (),
                        *signature, 1, commands.res, 192, 0, 0);
                } else {
                    call!(list, crate::console_slots::CL_DISPATCH_X, (u32, u32, u32, u32) -> (), 1, 1, 1, 0);
                }
                crate::gds::copy(raw, false, 0x8c / 4, 1, dst.va + snapshot, "read before waking either wave");
                call!(list, X_SET_PSO, (*mut c_void) -> (), poster_pso);
                call!(list, X_DISPATCH, (u32, u32, u32) -> (), 1, 1, 1);
                crate::gds::copy(raw, false, 0x8c / 4, 1, dst.va + snapshot + 4, "read after waking one wave");
                call!(list, X_SET_PSO, (*mut c_void) -> (), release_pso);
                call!(list, X_DISPATCH, (u32, u32, u32) -> (), 1, 1, 1);
                crate::gds::copy(raw, false, 0x8c / 4, 1, dst.va + snapshot + 8, "read after waking both waves");
            }

            crate::gds::copy(raw, true, counter + 2, 1, src.va + 32, "clear 16-group semaphore");
            crate::gds::copy(raw, true, 0x84 / 4, 1, src.va + 32, "clear 16-group marker");
            call!(list, X_SET_PSO, (*mut c_void) -> (), older);
            call!(list, X_DISPATCH, (u32, u32, u32) -> (), 4, 2, 2);
            crate::gds::copy(raw, false, 0x84 / 4, 1, dst.va + 232, "read 16-group waiters before post");
            call!(list, X_SET_PSO, (*mut c_void) -> (), poster_pso);
            call!(list, X_DISPATCH, (u32, u32, u32) -> (), 1, 1, 1);
            crate::gds::copy(raw, false, 0x84 / 4, 1, dst.va + 236, "read 16-group waiters after post");
            call!(list, X_SET_PSO, (*mut c_void) -> (), release_pso);
            call!(list, X_DISPATCH, (u32, u32, u32) -> (), 1, 1, 1);
            crate::gds::copy(raw, false, 0x84 / 4, 1, dst.va + 240, "read 16-group waiters after release-all");

            crate::gds::copy(raw, true, counter + 2, 1, src.va + 32, "clear routed large semaphore");
            crate::gds::copy(raw, true, 0x84 / 4, 1, src.va + 32, "clear routed large marker");
            call!(list, X_SET_PSO, (*mut c_void) -> (), older);
            call!(list, 59, (*mut c_void, u32, usize, u64, usize, u64) -> (),
                *signature, 1, commands.res, 240, 0, 0);
            crate::gds::copy(raw, false, 0x84 / 4, 1, dst.va + 244, "read routed large waiters before post");
            call!(list, X_SET_PSO, (*mut c_void) -> (), poster_pso);
            call!(list, X_DISPATCH, (u32, u32, u32) -> (), 1, 1, 1);
            crate::gds::copy(raw, false, 0x84 / 4, 1, dst.va + 248, "read routed large waiters after post");
            call!(list, X_SET_PSO, (*mut c_void) -> (), release_pso);
            call!(list, X_DISPATCH, (u32, u32, u32) -> (), 1, 1, 1);
            crate::gds::copy(raw, false, 0x84 / 4, 1, dst.va + 252, "read routed large waiters after release-all");
        }
        if let Some((_, commands, _)) = &indirect_inputs {
            crate::fill::barrier_pub(raw, commands.res, 0x200, ST_COPY_DEST);
        }
        call!(list, X_SET_PSO, (*mut c_void) -> (), normal);
        call!(list, X_DISPATCH, (u32, u32, u32) -> (), 2, 1, 1);
        crate::gds::copy(raw, false, counter, 2, dst.va, "read GWS test");
        let mut barrier = [0u8; 32];
        barrier[8..16].copy_from_slice(&(dst.res as u64).to_le_bytes());
        barrier[16..20].copy_from_slice(&u32::MAX.to_le_bytes());
        barrier[20..24].copy_from_slice(&ST_COPY_DEST.to_le_bytes());
        barrier[24..28].copy_from_slice(&ST_COPY_SOURCE.to_le_bytes());
        call!(list, X_BARRIER, (u32, *const u8) -> (), 1, barrier.as_ptr());
        call!(list, X_COPY_BUFFER_REGION, (usize, u64, usize, u64, u64) -> (), out.res, 0, dst.res, 0, if resume { 256 } else { 8 });
        crate::fill::barrier_pub(raw, dst.res, ST_COPY_SOURCE, ST_COPY_DEST);
        assert!(call!(list, X_CLOSE, () -> i32) >= 0);
        let f = fence(g.dev);
        submit(command_queue, list, f, 1);
        assert!(reached(f, 1, Duration::from_secs(5)));
        // The phased path also executes the direct 2x2x1 grid.
        let expected = 64 - 2 - 3 - u32::from(resume) - if phased { 4 + 5 + 2 + 6 + 8 + 5 } else { 0 };
        assert_eq!(rd(&out, 0), expected);
        assert_eq!(rd(&out, 4), 9, "the last wave reloads the barrier counter");
        let check_resume = |groups, routed_groups: u32| {
            if resume {
                assert_eq!(rd(&out, 132), 0, "unrelated dispatches must leave both waves waiting");
                assert_eq!(rd(&out, 20), 0, "a zero indirect count must leave both waves waiting");
                assert_eq!(rd(&out, 24), 0x5ab, "the indirect producer must resume a waiting wave");
                assert_eq!(rd(&out, 12), 0x5ab, "the saved VGPR must survive the PSO switch");
                assert_eq!(rd(&out, 8), 0, "exactly one credit wakes one wave");
                assert_eq!(rd(&out, 16), 2, "the resumed wave must not repeat its pre-wait instruction");
                assert_eq!(rd(&out, 28), 0, "release-all must not wake waves that arrived later");
                assert_eq!(rd(&out, 32), 0, "release-all must not leave semaphore credit");
                assert_eq!(rd(&out, 36), 2 * 0x5ab, "release-all must resume both queued waves");
                assert_eq!(rd(&out, 40), 0, "the first barrier dispatch must wait for the second wave");
                assert_eq!([rd(&out, 44), rd(&out, 48)], [0x321, 0x654],
                    "the last arrival must resume both dispatches with their saved registers");
                assert_eq!([rd(&out, 52), rd(&out, 56), rd(&out, 60)], [1, 0, 0],
                    "a zero-count indirect waiter must neither execute initially nor resume");
                assert_eq!([rd(&out, 64), rd(&out, 68), rd(&out, 72)], [0, 0x5ab, groups],
                    "only the selected indirect variant may execute and consume a credit");
                assert_eq!(rd(&out, 76), 8 * 32,
                    "both indirect barrier records must resume, with every lane executing once");
                assert_eq!([rd(&out, 80), rd(&out, 84)], [32, 0], "the first credit wakes the oldest wave");
                assert_eq!([rd(&out, 88), rd(&out, 92)], [64, 0], "the older launch drains first");
                assert_eq!([rd(&out, 96), rd(&out, 100)], [64, 32], "the younger launch only wakes after the older one");
                assert_eq!([rd(&out, 104), rd(&out, 108), rd(&out, 112)], [64, groups * 32, 0],
                    "release-all must wake every lane in both launches without leaving credits");
                assert_eq!([rd(&out, 116), rd(&out, 120)], [32, 0],
                    "one post must resume the one-group direct dispatch first");
                assert_eq!([rd(&out, 124), rd(&out, 128)], [32, 32],
                    "release-all must resume the one-group indirect dispatch exactly once");
                assert_eq!(rd(&out, 136), 1, "both waves pass the barrier but only one has a semaphore credit");
                assert_eq!(rd(&out, 140), 2, "release-all resumes the wave still waiting in the mixed shader");
                assert_eq!([rd(&out, 144), rd(&out, 148)], [0, 32],
                    "direct cooperative release-all wakes the previous dispatch");
                assert_eq!([rd(&out, 152), rd(&out, 156)], [0, 32],
                    "indirect cooperative release-all wakes the previous dispatch");
                let batch = |offset| [rd(&out, offset), rd(&out, offset + 4), rd(&out, offset + 8)];
                assert_eq!(batch(160), [6, 0, 0], "a zero-count batch must leave the observation intact");
                assert_eq!(batch(172), [1, 1, 2], "one active row must wake the older wave exactly once");
                assert_eq!(batch(184), [0, 1, 3], "the second row must see the earlier wake and clear its increment");
                assert_eq!(batch(196), [0, 1, 11], "all four rows must run and wake the indirect waiter exactly once");
                assert_eq!(batch(208), [0, 32, 64], "DispatchX must resume the two waves independently");
                assert_eq!(batch(220), [0, 32, 64], "ExecuteIndirect must resume the two waves independently");
                assert_eq!(batch(232), [0, 32, 16 * 32],
                    "one PSO must resume all 16 waves in a three-dimensional dispatch");
                assert_eq!(batch(244), [0, 32, routed_groups * 32],
                    "the indirect waiter must resume with the GPU-provided grid");
            }
        };
        check_resume(5, 16);
        if phased {
            // A second submission must start with zeroed checkpoints even though
            // the command list and its scratch allocation are the same.
            let scratch_alive = crate::gws_scratch::recording_probe(raw);
            let scratch_before_submit = scratch_alive();
            assert!(scratch_before_submit > 1);
            let (upload, _, _) = indirect_inputs.as_ref().unwrap();
            core::ptr::write_volatile(upload.cpu.cast::<u32>(), 4);
            core::ptr::write_volatile(upload.cpu.add(80).cast::<u32>(), 7);
            core::ptr::write_volatile(upload.cpu.add(160).cast::<u32>(), 1);
            core::ptr::write_volatile(upload.cpu.add(224).cast::<u32>(), 8);
            core::ptr::write_volatile(upload.cpu.add(240).cast::<[u32; 4]>(), [2, 3, 2, 0]);
            let gate = fence(g.dev);
            assert!(call!(command_queue, 15, (*mut c_void, u64) -> i32, gate, 1) >= 0);
            submit(command_queue, list, f, 2);
            let mut alloc = core::ptr::null_mut();
            assert!(call!(g.dev, 9, (u32, *const Guid, *mut *mut c_void) -> i32, 0, &IID_ALLOCATOR, &mut alloc) >= 0);
            let reset_hr = call!(list, 10, (*mut c_void, *mut c_void) -> i32, alloc, core::ptr::null_mut());
            let held_while_in_flight = scratch_alive();
            assert!(call!(gate, 10, (u64) -> i32, 1) >= 0);
            assert!(reset_hr >= 0);
            assert_eq!(held_while_in_flight, scratch_before_submit);
            assert!(reached(f, 2, Duration::from_secs(15)), "GWS stress resubmission did not complete");
            assert_eq!(rd(&out, 0), expected - 4, "resubmission must route the updated dimensions and counts");
            check_resume(8, 12);
            if resume {
                call!(command_queue, 2, () -> u32);
            }
            let start = Instant::now();
            // Queue-owned checkpoints can outlive Reset. Temporary allocations
            // unrelated to pending waves must still retire after the fence.
            let allowed = if resume { 1 } else { 0 };
            while scratch_alive() > allowed {
                assert!(start.elapsed() < Duration::from_secs(5), "temporary scratch survives completion and reset");
                std::thread::sleep(Duration::from_millis(2));
            }
        }
    }
}

fn counter_window(t0: i64, t1: i64, qpf: i64) -> (u64, u64) {
    let hz = crate::sys_ts::system_counter_hz() as u128;
    let tol = hz / 500;
    let lo = (t0 as u128 * hz / qpf as u128).saturating_sub(tol);
    let hi = t1 as u128 * hz / qpf as u128 + tol;
    (lo as u64, hi as u64)
}

#[test]
#[ignore = "needs a D3D12 device; run: cargo test -p d3d12_xs --lib hw_ -- --ignored --test-threads=1 (TOMBOX_HW_ADAPTER=warp for WARP)"]
fn hw_system_timestamp_counts_the_console_gpu_clock_from_the_qpc_epoch() {
    let g = gpu();
    unsafe {
        let out = buffer(g.dev, HEAP_READBACK, 64, 0, ST_COPY_DEST);
        let w = |l: *mut c_void, va: u64, flags: u32| call!(l, crate::cl_ext::CL_WRITE64_BOTTOM_OF_PIPE, (u64, u64, u32, u32) -> (), va, 0, 0, flags);
        let l = title_list(g.dev);
        w(l, out.va, crate::cl_ext::BOP_FLAG_SYSTEM_TIMESTAMP);
        w(l, out.va + 8, crate::cl_ext::BOP_FLAG_SYSTEM_TIMESTAMP);
        w(l, out.va + 16, crate::cl_ext::BOP_FLAG_GPU_TIMESTAMP);
        assert!(call!(l, X_CLOSE, () -> i32) >= 0);
        let l2 = title_list(g.dev);
        w(l2, out.va + 24, crate::cl_ext::BOP_FLAG_GPU_TIMESTAMP);
        w(l2, out.va + 32, crate::cl_ext::BOP_FLAG_SYSTEM_TIMESTAMP);
        assert!(call!(l2, X_CLOSE, () -> i32) >= 0);
        let (mut t0, mut t1, mut t2, mut t3, mut qpf) = (0i64, 0i64, 0i64, 0i64, 0i64);
        QueryPerformanceFrequency(&mut qpf);
        let f = fence(g.dev);
        QueryPerformanceCounter(&mut t0);
        submit(g.a, l, f, 1);
        assert!(reached(f, 1, Duration::from_secs(5)));
        QueryPerformanceCounter(&mut t1);
        std::thread::sleep(Duration::from_millis(200));
        QueryPerformanceCounter(&mut t2);
        submit(g.a, l2, f, 2);
        assert!(reached(f, 2, Duration::from_secs(5)));
        QueryPerformanceCounter(&mut t3);
        let q = |o: usize| core::ptr::read_volatile(out.cpu.add(o) as *const u64);
        let (v0, v1, a0, a1, v2) = (q(0), q(8), q(16), q(24), q(32));
        let who = if g.warp { "WARP" } else { "hardware" };
        assert!(v0 != 0 && v1 != 0 && v2 != 0, "SYSTEM_TIMESTAMP not written");
        assert!(v0 <= v1 && v1 < v2, "not monotonic: {v0} {v1} {v2}");
        let (lo, hi) = counter_window(t0, t1, qpf);
        assert!(v0 >= lo && v1 <= hi, "{who}: counter window [{lo}, {hi}] but SYSTEM_TIMESTAMP {v0}, {v1}");
        let (lo2, hi2) = counter_window(t2, t3, qpf);
        assert!(v2 >= lo2 && v2 <= hi2, "{who}: counter window [{lo2}, {hi2}] but SYSTEM_TIMESTAMP {v2}");
        let mut gf = 0u64;
        assert!(call!(g.a, 16, (*mut u64) -> i32, &mut gf) >= 0 && gf != 0);
        let hz = crate::sys_ts::system_counter_hz() as f64;
        let want = (a1 - a0) as f64 * hz / gf as f64;
        let got = (v2 - v1) as f64;
        assert!((got - want).abs() <= want * 0.001 + hz * 0.0002, "{who}: SYSTEM delta {got} vs GPU delta scaled to {hz} Hz {want}");
        let wall = (t3 - t0) as f64 / qpf as f64;
        assert!(got / hz >= 0.19 && got / hz <= wall + 0.01, "{who}: SYSTEM delta {} s outside [0.19, {wall}] s", got / hz);
    }
}

#[test]
#[ignore = "needs a D3D12 device; run: cargo test -p d3d12_xs --lib hw_ -- --ignored --test-threads=1 (TOMBOX_HW_ADAPTER=warp for WARP)"]
fn hw_write64_bottom_of_pipe_follows_the_release_umd_for_every_flag_and_flush_combination() {
    let g = gpu();
    unsafe {
        let out = buffer(g.dev, HEAP_READBACK, 256, 0, ST_COPY_DEST);
        let l = title_list(g.dev);
        let w = |va: u64, value: u64, flush: u32, flags: u32| call!(l, crate::cl_ext::CL_WRITE64_BOTTOM_OF_PIPE, (u64, u64, u32, u32) -> (), va, value, flush, flags);
        w(out.va, 0x1122_3344_5566_7788, 0, 0x80);
        w(out.va + 8, 0xab, 0x4800, 0);
        w(out.va + 16, 0, 0x3800, 3);
        w(out.va + 24, 5, 0x800, 1);
        w(out.va + 32, 9, 0x1000, 0x30);
        w(out.va + 40, 0, 0x3800, 2);
        w(out.va + 48, 0xdead, 0x2000, 0x10);
        w(out.va + 60, 1, 0, 0);
        assert!(call!(l, X_CLOSE, () -> i32) >= 0);
        w(out.va + 64, 1, 0, 0);
        let (mut t0, mut t1, mut qpf) = (0i64, 0i64, 0i64);
        QueryPerformanceFrequency(&mut qpf);
        let f = fence(g.dev);
        QueryPerformanceCounter(&mut t0);
        submit(g.a, l, f, 1);
        assert!(reached(f, 1, Duration::from_secs(5)));
        QueryPerformanceCounter(&mut t1);
        let q = |o: usize| core::ptr::read_volatile(out.cpu.add(o) as *const u64);
        assert_eq!(q(0), 0x1122_3344_5566_7788, "an undefined flag bit dropped the value write");
        assert_eq!(q(8), 0xab, "an undefined flush bit dropped the value write");
        let (lo, hi) = counter_window(t0, t1, qpf);
        let (gt0, gt1) = (q(16), q(24));
        if crate::bop64::scarlett() {
            assert!(gt0 >= lo && gt0 <= hi, "Scarlett: both TIMESTAMP flags must give the SYSTEM counter, got {gt0} outside [{lo}, {hi}]");
            assert!(gt1 != 0 && gt1 != 5, "GPU timestamp {gt1} must ignore Value");
        } else {
            assert!(gt0 != 0 && gt1 >= gt0 && gt1 != 5, "GPU timestamps {gt0} {gt1}: GPU_TIMESTAMP must win over SYSTEM_TIMESTAMP and ignore Value");
            let mut gf = 0u64;
            assert!(call!(g.a, 16, (*mut u64) -> i32, &mut gf) >= 0 && gf != 0);
            let span = (t1 - t0) as u128 * gf as u128 / qpf as u128;
            assert!(((gt1 - gt0) as u128) <= span + 1, "two GPU timestamps of one list {gt0}..{gt1} further apart than the submit window");
        }
        assert_eq!(q(32), 9, "LRU|STREAM together dropped the value write");
        let st = q(40);
        assert!(st >= lo && st <= hi, "SYSTEM_TIMESTAMP {st} outside the console-clock window [{lo}, {hi}]");
        assert_eq!(q(48), 0xdead);
        assert_eq!(q(56), 0, "a write to a destination off 8-byte alignment was recorded");
        assert_eq!(q(64), 0, "a write recorded into a closed list reached the GPU");
    }
}

#[test]
#[ignore = "needs a D3D12 device; run: cargo test -p d3d12_xs --lib hw_ -- --ignored --test-threads=1 (TOMBOX_HW_ADAPTER=warp for WARP)"]
fn hw_write64_value_lands_as_one_copy_ignores_and_keeps_title_predication() {
    let g = gpu();
    unsafe {
        let pred = buffer(g.dev, HEAP_UPLOAD, 256, 0, ST_GENERIC_READ);
        core::ptr::write_volatile(pred.cpu as *mut u64, 0);
        let src = buffer(g.dev, HEAP_UPLOAD, 256, 0, ST_GENERIC_READ);
        core::ptr::write_volatile(src.cpu as *mut u64, 0xeeee_eeee_eeee_eeee);
        let dst = buffer(g.dev, HEAP_DEFAULT, 256, 0, ST_COPY_DEST);
        let out = buffer(g.dev, HEAP_READBACK, 256, 0, ST_COPY_DEST);
        let l = title_list(g.dev);
        call!(l, 31, (*mut c_void, u64, u32) -> (), pred.res as *mut c_void, 0, 0);
        for i in 0..8u64 {
            call!(l, crate::cl_ext::CL_WRITE64_BOTTOM_OF_PIPE, (u64, u64, u32, u32) -> (), dst.va + 8 * i, 0x0102_0304_0506_0700 + i, 0x800, 0);
        }
        call!(l, X_COPY_BUFFER_REGION, (usize, u64, usize, u64, u64) -> (), dst.res, 64, src.res, 0, 8);
        call!(l, crate::cl_ext::CL_WRITE64_BOTTOM_OF_PIPE, (u64, u64, u32, u32) -> (), out.va + 128, u64::MAX - 1, 0, 0);
        call!(l, 31, (*mut c_void, u64, u32) -> (), core::ptr::null_mut::<c_void>(), 0, 0);
        let mut b = [0u8; 32];
        b[8..16].copy_from_slice(&(dst.res as u64).to_le_bytes());
        b[16..20].copy_from_slice(&0xffff_ffffu32.to_le_bytes());
        b[20..24].copy_from_slice(&ST_COPY_DEST.to_le_bytes());
        b[24..28].copy_from_slice(&ST_COPY_SOURCE.to_le_bytes());
        call!(l, X_BARRIER, (u32, *const u8) -> (), 1, b.as_ptr());
        call!(l, X_COPY_BUFFER_REGION, (usize, u64, usize, u64, u64) -> (), out.res, 0, dst.res, 0, 72);
        assert!(call!(l, X_CLOSE, () -> i32) >= 0);
        let f = fence(g.dev);
        submit(g.a, l, f, 1);
        assert!(reached(f, 1, Duration::from_secs(5)));
        let q = |o: usize| core::ptr::read_volatile(out.cpu.add(o) as *const u64);
        for i in 0..8 {
            assert_eq!(q(8 * i), 0x0102_0304_0506_0700 + i as u64, "value {i} under title predication");
        }
        assert_eq!(q(64), 0, "the title's predication was not restored after the value copy");
        assert_eq!(q(128), u64::MAX - 1);
    }
}

#[test]
#[ignore = "needs a D3D12 device; run: cargo test -p d3d12_xs --lib hw_ -- --ignored --test-threads=1 (TOMBOX_HW_ADAPTER=warp for WARP)"]
fn hw_write64_into_a_closed_list_is_not_recorded() {
    let g = gpu();
    unsafe {
        let out = buffer(g.dev, HEAP_READBACK, 256, 0, ST_COPY_DEST);
        let l = title_list(g.dev);
        assert!(call!(l, X_CLOSE, () -> i32) >= 0);
        call!(l, crate::cl_ext::CL_WRITE64_BOTTOM_OF_PIPE, (u64, u64, u32, u32) -> (), out.va + 64, 1, 0, 0);
        let f = fence(g.dev);
        submit(g.a, l, f, 1);
        assert!(reached(f, 1, Duration::from_secs(5)));
        let q = |o: usize| core::ptr::read_volatile(out.cpu.add(o) as *const u64);
        assert_eq!(q(64), 0, "a write recorded into a closed list reached the GPU");
    }
}

const X_PREDICATION: usize = 31;
const X_SET_COMPUTE_ROOT_UAV: usize = 45;
const X_CLOSE_BUNDLE: usize = 92;
const ST_UAV: u32 = 0x8;
const ST_PREDICATION: u32 = 0x200;

unsafe fn bundle_list(dev: usize) -> *mut c_void {
    let mut a: *mut c_void = core::ptr::null_mut();
    assert!(call!(dev, 9, (u32, *const Guid, *mut *mut c_void) -> i32, 1, &IID_ALLOCATOR, &mut a) >= 0);
    let mut l: *mut c_void = core::ptr::null_mut();
    assert!(call!(dev, 12, (u32, u32, *mut c_void, *mut c_void, *const Guid, *mut *mut c_void) -> i32, 0, 1, a, core::ptr::null_mut(), &IID_LIST, &mut l) >= 0);
    crate::cmdlist_wrap::wrap(l)
}

unsafe fn raw_barrier(l: *mut c_void, res: usize, before: u32, after: u32) {
    let mut b = [0u8; 32];
    b[8..16].copy_from_slice(&(res as u64).to_le_bytes());
    b[16..20].copy_from_slice(&0xffff_ffffu32.to_le_bytes());
    b[20..24].copy_from_slice(&before.to_le_bytes());
    b[24..28].copy_from_slice(&after.to_le_bytes());
    call!(l, X_BARRIER, (u32, *const u8) -> (), 1, b.as_ptr());
}

#[test]
#[ignore = "needs a D3D12 device; run: cargo test -p d3d12_xs --lib hw_ -- --ignored --test-threads=1 (TOMBOX_HW_ADAPTER=warp for WARP)"]
fn hw_execute_indirect_bundle_honours_gpu_size_max_records_and_predication() {
    let g = gpu();
    unsafe {
        let (rs, pso) = compute_pso(g.dev, "#define RS \"RootConstants(num32BitConstants=1, b0), UAV(u0)\"\ncbuffer C : register(b0) { uint idx; };\nRWByteAddressBuffer o : register(u0);\n[RootSignature(RS)]\n[numthreads(1, 1, 1)]\nvoid main() { uint p; o.InterlockedAdd(idx * 4, 1u, p); }\n", None);
        let out = buffer(g.dev, HEAP_DEFAULT, 256, FLAG_UAV, ST_UAV);
        let rb = buffer(g.dev, HEAP_READBACK, 256, 0, ST_COPY_DEST);
        let bb = buffer(g.dev, HEAP_UPLOAD, 4096, 0, ST_GENERIC_READ);
        core::ptr::write_bytes(bb.cpu, 0, 4096);
        crate::placed::register(bb.res as *mut c_void, crate::placed::Placed { va: bb.cpu as u64, desc: [0u8; 0x38], init_state: ST_GENERIC_READ });
        let mut bundles = Vec::new();
        for i in 0..2u32 {
            let b = bundle_list(g.dev);
            call!(b, X_SET_COMPUTE_ROOT_SIGNATURE, (*mut c_void) -> (), rs);
            call!(b, X_SET_PSO, (*mut c_void) -> (), pso);
            call!(b, X_SET_COMPUTE_ROOT_CONSTANT, (u32, u32, u32) -> (), 0, i, 0);
            call!(b, X_SET_COMPUTE_ROOT_UAV, (u32, u64) -> (), 1, out.va);
            call!(b, X_DISPATCH, (u32, u32, u32) -> (), 1, 1, 1);
            let mut dst = bb.cpu.add(0x100 + 16 * i as usize) as *mut c_void;
            assert_eq!(call!(b, X_CLOSE_BUNDLE, (*mut *mut c_void, u32) -> u32, &mut dst, 16), 16);
            bundles.push(b);
        }
        let szb = buffer(g.dev, HEAP_DEFAULT, 256, FLAG_UAV, ST_COMMON);
        let pb = buffer(g.dev, HEAP_DEFAULT, 256, FLAG_UAV, ST_COMMON);
        let bb2 = buffer(g.dev, HEAP_DEFAULT, 256, 0, ST_COMMON);
        let (wrs, wpso) = compute_pso(g.dev, "#define RS \"UAV(u0), UAV(u1)\"\nRWByteAddressBuffer s : register(u0);\nRWByteAddressBuffer q : register(u1);\n[RootSignature(RS)]\n[numthreads(1, 1, 1)]\nvoid main() { s.Store4(0, uint4(16, 0, 48, 32)); q.Store2(0, uint2(0, 0)); }\n", None);
        let p = raw_list(g.dev);
        call!(p, PC_SET_COMPUTE_ROOT_SIGNATURE, (*mut c_void) -> (), wrs);
        call!(p, X_SET_PSO, (*mut c_void) -> (), wpso);
        call!(p, PC_SET_COMPUTE_ROOT_UAV, (u32, u64) -> (), 0, szb.va);
        call!(p, PC_SET_COMPUTE_ROOT_UAV, (u32, u64) -> (), 1, pb.va);
        call!(p, X_DISPATCH, (u32, u32, u32) -> (), 1, 1, 1);
        raw_barrier(p, bb2.res, ST_COMMON, ST_COPY_DEST);
        call!(p, X_COPY_BUFFER_REGION, (usize, u64, usize, u64, u64) -> (), bb2.res, 0, bb.res, 0x100, 32);
        raw_barrier(p, bb2.res, ST_COPY_DEST, ST_COMMON);
        assert!(call!(p, X_CLOSE, () -> i32) >= 0);
        let fp = fence(g.dev);
        submit(g.a, p, fp, 1);
        assert!(reached(fp, 1, Duration::from_secs(5)));
        let l = title_list(g.dev);
        let null = core::ptr::null_mut::<c_void>();
        let eib_with_pred_offset = |res: usize, off: u64, max: u32, sz: *mut c_void, so: u32, po: u32| call!(l, crate::cl_ext::CL_EXECUTE_INDIRECT_BUNDLE, (*mut c_void, u64, u32, *mut c_void, u32, u32) -> (), res as *mut c_void, off, max, sz, so, po);
        let eib = |res, off, max, sz, so| eib_with_pred_offset(res, off, max, sz, so, 0);
        let mut n = 0u64;
        let mut snap = || {
            raw_barrier(l, out.res, ST_UAV, ST_COPY_SOURCE);
            call!(l, X_COPY_BUFFER_REGION, (usize, u64, usize, u64, u64) -> (), rb.res, n * 8, out.res, 0, 8);
            raw_barrier(l, out.res, ST_COPY_SOURCE, ST_UAV);
            n += 1;
        };
        let szp = szb.res as *mut c_void;
        // The retail UMD always writes zero into this packet field.
        eib_with_pred_offset(bb.res, 0x100, 32, null, 0, 4);
        snap();
        eib(bb.res, 0x100, 32, szp, 0);
        snap();
        eib(bb.res, 0x100, 32, szp, 4);
        snap();
        eib(bb.res, 0x100, 16, szp, 8);
        snap();
        eib(bb.res, 0x100, 0x40, szp, 12);
        snap();
        raw_barrier(l, pb.res, ST_COMMON, ST_PREDICATION);
        call!(l, X_PREDICATION, (*mut c_void, u64, u32) -> (), pb.res as *mut c_void, 0, 0);
        eib(bb.res, 0x100, 32, null, 0);
        call!(l, X_PREDICATION, (*mut c_void, u64, u32) -> (), null, 0, 0);
        snap();
        call!(l, X_PREDICATION, (*mut c_void, u64, u32) -> (), pb.res as *mut c_void, 0, 1);
        eib(bb.res, 0x100, 32, null, 0);
        call!(l, X_PREDICATION, (*mut c_void, u64, u32) -> (), null, 0, 0);
        snap();
        raw_barrier(l, pb.res, ST_PREDICATION, ST_COMMON);
        eib(bb2.res, 0, 32, null, 0);
        snap();
        assert!(call!(l, X_CLOSE, () -> i32) >= 0);
        let f = fence(g.dev);
        submit(g.a, l, f, 1);
        assert!(reached(f, 1, Duration::from_secs(5)));
        let got: Vec<(u32, u32)> = (0..8).map(|i| (rd(&rb, i * 8), rd(&rb, i * 8 + 4))).collect();
        assert_eq!(got, vec![(1, 1), (2, 1), (2, 1), (3, 1), (4, 2), (4, 2), (5, 3), (6, 4)], "per-call bundle execution counts (A, B)");

        call!(g.dev, 1, () -> u32);
        let debug_device = crate::device_wrap::wrap_device_for_driver(g.dev as *mut c_void, crate::process_debug::Driver::Debug);
        let mut allocator = core::ptr::null_mut();
        assert_eq!(call!(g.dev, 9, (u32, *const Guid, *mut *mut c_void) -> i32, 0, &IID_ALLOCATOR, &mut allocator), 0);
        let params = [0u32, 0, 1, 0];
        let mut debug = core::ptr::null_mut();
        assert_eq!(call!(debug_device, 73, (*const u32, *mut c_void, *mut c_void, *const Guid, *mut *mut c_void) -> i32,
            params.as_ptr(), allocator, null, &IID_LIST, &mut debug), 0);
        call!(debug, crate::cl_ext::CL_EXECUTE_INDIRECT_BUNDLE, (*mut c_void, u64, u32, *mut c_void, u32, u32) -> (),
            bb.res as *mut c_void, 0x100, 32, null, 0, 4);
        raw_barrier(debug, out.res, ST_UAV, ST_COPY_SOURCE);
        call!(debug, X_COPY_BUFFER_REGION, (usize, u64, usize, u64, u64) -> (), rb.res, 64, out.res, 0, 8);
        assert!(call!(debug, X_CLOSE, () -> i32) >= 0);
        submit(g.a, debug, f, 2);
        assert!(reached(f, 2, Duration::from_secs(5)));
        assert_eq!((rd(&rb, 64), rd(&rb, 68)), (6, 4), "debug UMD rejects a nonzero offset");
        call!(debug, 2, () -> u32);
        call!(allocator, 2, () -> u32);
        call!(debug_device, 2, () -> u32);
        crate::placed::forget(bb.res as *mut c_void);
        drop(bundles);
    }
}

#[test]
#[ignore = "needs a D3D12 device; run: cargo test -p d3d12_xs --lib hw_ -- --ignored --test-threads=1 (TOMBOX_HW_ADAPTER=warp for WARP)"]
fn hw_execute_indirect_bundle_reads_the_size_at_each_execution_of_a_closed_list() {
    let g = gpu();
    unsafe {
        let (rs, pso) = compute_pso(g.dev, "#define RS \"RootConstants(num32BitConstants=1, b0), UAV(u0)\"\ncbuffer C : register(b0) { uint idx; };\nRWByteAddressBuffer o : register(u0);\n[RootSignature(RS)]\n[numthreads(1, 1, 1)]\nvoid main() { uint p; o.InterlockedAdd(idx * 4, 1u, p); }\n", None);
        let (srs, spso) = compute_pso(g.dev, "#define RS \"RootConstants(num32BitConstants=1, b0), UAV(u0)\"\ncbuffer C : register(b0) { uint v; };\nRWByteAddressBuffer o : register(u0);\n[RootSignature(RS)]\n[numthreads(1, 1, 1)]\nvoid main() { o.Store(0, v); }\n", None);
        let out = buffer(g.dev, HEAP_DEFAULT, 256, FLAG_UAV, ST_UAV);
        let rb = buffer(g.dev, HEAP_READBACK, 256, 0, ST_COPY_DEST);
        let bb = buffer(g.dev, HEAP_UPLOAD, 256, 0, ST_GENERIC_READ);
        core::ptr::write_bytes(bb.cpu, 0, 256);
        crate::placed::register(bb.res as *mut c_void, crate::placed::Placed { va: bb.cpu as u64, desc: [0u8; 0x38], init_state: ST_GENERIC_READ });
        for i in 0..2u32 {
            let b = bundle_list(g.dev);
            call!(b, X_SET_COMPUTE_ROOT_SIGNATURE, (*mut c_void) -> (), rs);
            call!(b, X_SET_PSO, (*mut c_void) -> (), pso);
            call!(b, X_SET_COMPUTE_ROOT_CONSTANT, (u32, u32, u32) -> (), 0, i, 0);
            call!(b, X_SET_COMPUTE_ROOT_UAV, (u32, u64) -> (), 1, out.va);
            call!(b, X_DISPATCH, (u32, u32, u32) -> (), 1, 1, 1);
            let mut dst = bb.cpu.add(16 * i as usize) as *mut c_void;
            assert_eq!(call!(b, X_CLOSE_BUNDLE, (*mut *mut c_void, u32) -> u32, &mut dst, 16), 16);
        }
        let szb = buffer(g.dev, HEAP_DEFAULT, 256, FLAG_UAV, ST_COMMON);
        let l = title_list(g.dev);
        call!(l, crate::cl_ext::CL_EXECUTE_INDIRECT_BUNDLE, (*mut c_void, u64, u32, *mut c_void, u32, u32) -> (), bb.res as *mut c_void, 0, 32, szb.res as *mut c_void, 0, 0);
        raw_barrier(l, out.res, ST_UAV, ST_COPY_SOURCE);
        call!(l, X_COPY_BUFFER_REGION, (usize, u64, usize, u64, u64) -> (), rb.res, 0, out.res, 0, 8);
        raw_barrier(l, out.res, ST_COPY_SOURCE, ST_UAV);
        assert!(call!(l, X_CLOSE, () -> i32) >= 0);
        let mut got = Vec::new();
        for (n, size) in [16u32, 32, 0, 48].into_iter().enumerate() {
            let s = raw_list(g.dev);
            call!(s, PC_SET_COMPUTE_ROOT_SIGNATURE, (*mut c_void) -> (), srs);
            call!(s, X_SET_PSO, (*mut c_void) -> (), spso);
            call!(s, 33, (u32, u32, u32) -> (), 0, size, 0);
            call!(s, PC_SET_COMPUTE_ROOT_UAV, (u32, u64) -> (), 1, szb.va);
            call!(s, X_DISPATCH, (u32, u32, u32) -> (), 1, 1, 1);
            assert!(call!(s, X_CLOSE, () -> i32) >= 0);
            let fs = fence(g.dev);
            submit(g.a, s, fs, 1);
            assert!(reached(fs, 1, Duration::from_secs(5)));
            let f = fence(g.dev);
            submit(g.a, l, f, 1);
            assert!(reached(f, 1, Duration::from_secs(5)), "execution {n} did not complete");
            got.push((rd(&rb, 0), rd(&rb, 4)));
        }
        assert_eq!(got, vec![(1, 0), (2, 1), (2, 1), (3, 2)], "each execution must follow the size the GPU holds at that execution");
        crate::placed::forget(bb.res as *mut c_void);
    }
}

const COUNT_HLSL: &str = "#define RS \"RootConstants(num32BitConstants=1, b0), UAV(u0)\"\ncbuffer C : register(b0) { uint idx; };\nRWByteAddressBuffer o : register(u0);\n[RootSignature(RS)]\n[numthreads(1, 1, 1)]\nvoid main() { uint p; o.InterlockedAdd(idx * 4, 1u, p); }\n";

unsafe fn counting_bundle(dev: usize, rs: *mut c_void, pso: *mut c_void, idx: u32, out_va: u64) -> *mut c_void {
    let b = unsafe { bundle_list(dev) };
    call!(b, X_SET_COMPUTE_ROOT_SIGNATURE, (*mut c_void) -> (), rs);
    call!(b, X_SET_PSO, (*mut c_void) -> (), pso);
    call!(b, X_SET_COMPUTE_ROOT_CONSTANT, (u32, u32, u32) -> (), 0, idx, 0);
    call!(b, X_SET_COMPUTE_ROOT_UAV, (u32, u64) -> (), 1, out_va);
    call!(b, X_DISPATCH, (u32, u32, u32) -> (), 1, 1, 1);
    b
}

unsafe fn close_bundle_into(b: *mut c_void, at: *mut u8) {
    let mut dst = at as *mut c_void;
    assert_eq!(call!(b, X_CLOSE_BUNDLE, (*mut *mut c_void, u32) -> u32, &mut dst, 16), 16);
}

#[test]
#[ignore = "needs a D3D12 device; run: cargo test -p d3d12_xs --lib hw_ -- --ignored --test-threads=1 (TOMBOX_HW_ADAPTER=warp for WARP)"]
fn hw_execute_indirect_bundle_runs_bundles_closed_after_the_call_at_each_submit() {
    let g = gpu();
    unsafe {
        let (rs, pso) = compute_pso(g.dev, COUNT_HLSL, None);
        let out = buffer(g.dev, HEAP_DEFAULT, 256, FLAG_UAV, ST_UAV);
        let rb = buffer(g.dev, HEAP_READBACK, 256, 0, ST_COPY_DEST);
        let bb = buffer(g.dev, HEAP_UPLOAD, 256, 0, ST_GENERIC_READ);
        core::ptr::write_bytes(bb.cpu, 0, 256);
        crate::placed::register(bb.res as *mut c_void, crate::placed::Placed { va: bb.cpu as u64, desc: [0u8; 0x38], init_state: ST_GENERIC_READ });
        let a = counting_bundle(g.dev, rs, pso, 0, out.va);
        let l = title_list(g.dev);
        call!(l, crate::cl_ext::CL_EXECUTE_INDIRECT_BUNDLE, (*mut c_void, u64, u32, *mut c_void, u32, u32) -> (), bb.res as *mut c_void, 0, 32, core::ptr::null_mut::<c_void>(), 0, 0);
        raw_barrier(l, out.res, ST_UAV, ST_COPY_SOURCE);
        call!(l, X_COPY_BUFFER_REGION, (usize, u64, usize, u64, u64) -> (), rb.res, 0, out.res, 0, 8);
        raw_barrier(l, out.res, ST_COPY_SOURCE, ST_UAV);
        assert!(call!(l, X_CLOSE, () -> i32) >= 0);
        close_bundle_into(a, bb.cpu);
        let f = fence(g.dev);
        submit(g.a, l, f, 1);
        assert!(reached(f, 1, Duration::from_secs(5)));
        let first = (rd(&rb, 0), rd(&rb, 4));
        let b = counting_bundle(g.dev, rs, pso, 1, out.va);
        close_bundle_into(b, bb.cpu.add(16));
        submit(g.a, l, f, 2);
        assert!(reached(f, 2, Duration::from_secs(5)));
        let second = (rd(&rb, 0), rd(&rb, 4));
        assert_eq!((first, second), ((1, 0), (2, 1)), "bundles closed after the call, before each ExecuteCommandLists, must be reachable");
        crate::placed::forget(bb.res as *mut c_void);
    }
}

#[test]
#[ignore = "needs a D3D12 device; run: cargo test -p d3d12_xs --lib hw_ -- --ignored --test-threads=1 (TOMBOX_HW_ADAPTER=warp for WARP)"]
fn hw_execute_indirect_bundle_survives_a_bundle_released_before_submit() {
    let g = gpu();
    unsafe {
        let (rs, pso) = compute_pso(g.dev, COUNT_HLSL, None);
        let out = buffer(g.dev, HEAP_DEFAULT, 256, FLAG_UAV, ST_UAV);
        let rb = buffer(g.dev, HEAP_READBACK, 256, 0, ST_COPY_DEST);
        let bb = buffer(g.dev, HEAP_UPLOAD, 256, 0, ST_GENERIC_READ);
        core::ptr::write_bytes(bb.cpu, 0, 256);
        crate::placed::register(bb.res as *mut c_void, crate::placed::Placed { va: bb.cpu as u64, desc: [0u8; 0x38], init_state: ST_GENERIC_READ });
        let k = counting_bundle(g.dev, rs, pso, 5, out.va);
        close_bundle_into(k, bb.cpu);
        let kr = crate::cmdlist_wrap::unwrap(k) as usize;
        let l = title_list(g.dev);
        call!(l, crate::cl_ext::CL_EXECUTE_INDIRECT_BUNDLE, (*mut c_void, u64, u32, *mut c_void, u32, u32) -> (), bb.res as *mut c_void, 0, 16, core::ptr::null_mut::<c_void>(), 0, 0);
        raw_barrier(l, out.res, ST_UAV, ST_COPY_SOURCE);
        call!(l, X_COPY_BUFFER_REGION, (usize, u64, usize, u64, u64) -> (), rb.res, 0, out.res, 20, 4);
        raw_barrier(l, out.res, ST_COPY_SOURCE, ST_UAV);
        assert!(call!(l, X_CLOSE, () -> i32) >= 0);
        assert!(crate::console_slots_cl2::live_bundles().contains(&kr));
        assert!(call!(k, 2, () -> u32) >= 1, "the recorded part must keep the PC bundle alive");
        assert!(!crate::console_slots_cl2::live_bundles().contains(&kr), "a bundle the title released is no longer a candidate");
        let f = fence(g.dev);
        submit(g.a, l, f, 1);
        assert!(reached(f, 1, Duration::from_secs(5)));
        assert_eq!(rd(&rb, 0), 0, "a released bundle must not run");
        submit(g.a, l, f, 2);
        assert!(reached(f, 2, Duration::from_secs(5)));
        assert_eq!(crate::indirect_bundle::held_count(kr), 0, "references of the replaced part are dropped once the GPU is done with it");
        crate::placed::forget(bb.res as *mut c_void);
    }
}

#[test]
#[ignore = "needs a D3D12 device; run: cargo test -p d3d12_xs --lib hw_ -- --ignored --test-threads=1 (TOMBOX_HW_ADAPTER=warp for WARP)"]
fn hw_execute_bundle_state_carries_into_the_calling_list() {
    let g = gpu();
    unsafe {
        let (rs, pso) = compute_pso(g.dev, COUNT_HLSL, None);
        let (_, store) = compute_pso(g.dev, "#define RS \"RootConstants(num32BitConstants=1, b0), UAV(u0)\"\ncbuffer C : register(b0) { uint v; };\nRWByteAddressBuffer o : register(u0);\n[RootSignature(RS)]\n[numthreads(1, 1, 1)]\nvoid main() { o.Store(0, v); }\n", Some(rs));
        let out = buffer(g.dev, HEAP_DEFAULT, 256, FLAG_UAV, ST_UAV);
        let pout = buffer(g.dev, HEAP_DEFAULT, 256, FLAG_UAV, ST_UAV);
        let rb = buffer(g.dev, HEAP_READBACK, 256, 0, ST_COPY_DEST);
        let bb = buffer(g.dev, HEAP_UPLOAD, 256, 0, ST_GENERIC_READ);
        let k = counting_bundle(g.dev, rs, pso, 3, out.va);
        close_bundle_into(k, bb.cpu);
        let l = title_list(g.dev);
        call!(l, X_SET_COMPUTE_ROOT_SIGNATURE, (*mut c_void) -> (), rs);
        call!(l, X_SET_PSO, (*mut c_void) -> (), store);
        call!(l, X_SET_COMPUTE_ROOT_CONSTANT, (u32, u32, u32) -> (), 0, 7, 0);
        call!(l, X_SET_COMPUTE_ROOT_UAV, (u32, u64) -> (), 1, pout.va);
        call!(l, 27, (*mut c_void) -> (), k);
        call!(l, X_DISPATCH, (u32, u32, u32) -> (), 1, 1, 1);
        raw_barrier(l, pout.res, ST_UAV, ST_COPY_SOURCE);
        call!(l, X_COPY_BUFFER_REGION, (usize, u64, usize, u64, u64) -> (), rb.res, 0, pout.res, 0, 4);
        raw_barrier(l, out.res, ST_UAV, ST_COPY_SOURCE);
        call!(l, X_COPY_BUFFER_REGION, (usize, u64, usize, u64, u64) -> (), rb.res, 4, out.res, 12, 4);
        assert!(call!(l, X_CLOSE, () -> i32) >= 0);
        let f = fence(g.dev);
        submit(g.a, l, f, 1);
        assert!(reached(f, 1, Duration::from_secs(5)));
        assert_eq!((rd(&rb, 0), rd(&rb, 4)), (0, 2), "after ExecuteBundle the list must draw with the PSO, root signature, constants and UAV the bundle set");
    }
}

#[test]
#[ignore = "needs a D3D12 device; run: cargo test -p d3d12_xs --lib hw_ -- --ignored --test-threads=1 (TOMBOX_HW_ADAPTER=warp for WARP)"]
fn hw_execute_indirect_bundle_has_no_candidate_budget() {
    let g = gpu();
    unsafe {
        const N: u32 = 96;
        let (rs, pso) = compute_pso(g.dev, COUNT_HLSL, None);
        let out = buffer(g.dev, HEAP_DEFAULT, 512, FLAG_UAV, ST_UAV);
        let rb = buffer(g.dev, HEAP_READBACK, 512, 0, ST_COPY_DEST);
        let bb = buffer(g.dev, HEAP_UPLOAD, 4096, 0, ST_GENERIC_READ);
        core::ptr::write_bytes(bb.cpu, 0, 4096);
        crate::placed::register(bb.res as *mut c_void, crate::placed::Placed { va: bb.cpu as u64, desc: [0u8; 0x38], init_state: ST_GENERIC_READ });
        for i in 0..N {
            let b = counting_bundle(g.dev, rs, pso, i, out.va);
            close_bundle_into(b, bb.cpu.add(16 * i as usize));
        }
        let live = crate::console_slots_cl2::live_bundles().len() as u64;
        assert!(live * N as u64 > 4096, "{live} live bundles x {N} slots must exceed the old 4096-candidate budget");
        let l = title_list(g.dev);
        call!(l, crate::cl_ext::CL_EXECUTE_INDIRECT_BUNDLE, (*mut c_void, u64, u32, *mut c_void, u32, u32) -> (), bb.res as *mut c_void, 0, 16 * N, core::ptr::null_mut::<c_void>(), 0, 0);
        raw_barrier(l, out.res, ST_UAV, ST_COPY_SOURCE);
        call!(l, X_COPY_BUFFER_REGION, (usize, u64, usize, u64, u64) -> (), rb.res, 0, out.res, 0, 4 * N as u64);
        raw_barrier(l, out.res, ST_COPY_SOURCE, ST_UAV);
        assert!(call!(l, X_CLOSE, () -> i32) >= 0);
        let f = fence(g.dev);
        submit(g.a, l, f, 1);
        assert!(reached(f, 1, Duration::from_secs(20)));
        let got: Vec<u32> = (0..N as usize).map(|i| rd(&rb, 4 * i)).collect();
        assert_eq!(got, vec![1; N as usize], "every record slot must run its bundle exactly once");
        crate::placed::forget(bb.res as *mut c_void);
    }
}

#[test]
#[ignore = "needs a D3D12 device; run: cargo test -p d3d12_xs --lib hw_ -- --ignored --test-threads=1 (TOMBOX_HW_ADAPTER=warp for WARP)"]
fn hw_execute_indirect_bundle_carries_executed_bundle_state_and_nothing_from_skipped_ones() {
    let g = gpu();
    unsafe {
        let (rs, pso) = compute_pso(g.dev, COUNT_HLSL, None);
        let (_, store) = compute_pso(g.dev, "#define RS \"RootConstants(num32BitConstants=1, b0), UAV(u0)\"\ncbuffer C : register(b0) { uint v; };\nRWByteAddressBuffer o : register(u0);\n[RootSignature(RS)]\n[numthreads(1, 1, 1)]\nvoid main() { o.Store(0, v); }\n", Some(rs));
        let out = buffer(g.dev, HEAP_DEFAULT, 256, FLAG_UAV, ST_UAV);
        let pout = buffer(g.dev, HEAP_DEFAULT, 256, FLAG_UAV, ST_UAV);
        let rb = buffer(g.dev, HEAP_READBACK, 256, 0, ST_COPY_DEST);
        let bb = buffer(g.dev, HEAP_UPLOAD, 256, 0, ST_GENERIC_READ);
        core::ptr::write_bytes(bb.cpu, 0, 256);
        crate::placed::register(bb.res as *mut c_void, crate::placed::Placed { va: bb.cpu as u64, desc: [0u8; 0x38], init_state: ST_GENERIC_READ });
        let k = counting_bundle(g.dev, rs, pso, 3, out.va);
        close_bundle_into(k, bb.cpu);
        let l = title_list(g.dev);
        let eib = |off: u64| call!(l, crate::cl_ext::CL_EXECUTE_INDIRECT_BUNDLE, (*mut c_void, u64, u32, *mut c_void, u32, u32) -> (), bb.res as *mut c_void, off, 16, core::ptr::null_mut::<c_void>(), 0, 0);
        let grab = |dst: u64| {
            raw_barrier(l, pout.res, ST_UAV, ST_COPY_SOURCE);
            call!(l, X_COPY_BUFFER_REGION, (usize, u64, usize, u64, u64) -> (), rb.res, dst, pout.res, 0, 4);
            raw_barrier(l, pout.res, ST_COPY_SOURCE, ST_UAV);
            raw_barrier(l, out.res, ST_UAV, ST_COPY_SOURCE);
            call!(l, X_COPY_BUFFER_REGION, (usize, u64, usize, u64, u64) -> (), rb.res, dst + 4, out.res, 12, 4);
            raw_barrier(l, out.res, ST_COPY_SOURCE, ST_UAV);
        };
        call!(l, X_SET_COMPUTE_ROOT_SIGNATURE, (*mut c_void) -> (), rs);
        call!(l, X_SET_PSO, (*mut c_void) -> (), store);
        call!(l, X_SET_COMPUTE_ROOT_CONSTANT, (u32, u32, u32) -> (), 0, 7, 0);
        call!(l, X_SET_COMPUTE_ROOT_UAV, (u32, u64) -> (), 1, pout.va);
        eib(0x80);
        call!(l, X_DISPATCH, (u32, u32, u32) -> (), 1, 1, 1);
        grab(0);
        call!(l, X_SET_COMPUTE_ROOT_CONSTANT, (u32, u32, u32) -> (), 0, 9, 0);
        eib(0);
        call!(l, X_DISPATCH, (u32, u32, u32) -> (), 1, 1, 1);
        grab(8);
        assert!(call!(l, X_CLOSE, () -> i32) >= 0);
        let f = fence(g.dev);
        submit(g.a, l, f, 1);
        assert!(reached(f, 1, Duration::from_secs(5)));
        let got = (rd(&rb, 0), rd(&rb, 4), rd(&rb, 8), rd(&rb, 12));
        assert_eq!(got, (7, 0, 7, 2), "skipped bundle: the title state and no bundle work; executed bundle: its work, then the list draws with the PSO, root signature, constants and UAV the bundle set");
        crate::placed::forget(bb.res as *mut c_void);
    }
}

const IID_QUERY_HEAP: Guid = Guid { d1: 0x0d96_58ae, d2: 0xed45, d3: 0x469e, d4: [0xa6, 0x1d, 0x97, 0x0e, 0xc5, 0x83, 0xca, 0xb4] };
const X_END_QUERY: usize = 29;
const X_RESOLVE_QUERY_DATA: usize = 30;
const CONSOLE_HZ: u64 = 99_750_000;

unsafe fn title_clock(q: usize) -> (u64, u64) {
    let (mut g, mut c) = (0u64, 0u64);
    assert_eq!(call!(q, 17, (*mut u64, *mut u64) -> i32, &mut g, &mut c), 0);
    (g, c)
}

unsafe fn timestamp_heap(dev: usize, n: u32) -> *mut c_void {
    let desc: [u32; 3] = [1, n, 0];
    let mut h: *mut c_void = core::ptr::null_mut();
    assert!(call!(dev, 39, (*const u32, *const Guid, *mut *mut c_void) -> i32, desc.as_ptr(), &IID_QUERY_HEAP, &mut h) >= 0);
    h
}

fn qpf() -> u64 {
    let mut f = 0i64;
    unsafe { QueryPerformanceFrequency(&mut f) };
    f as u64
}

fn on_qpc_base(v: u64, qpc: u64, tol_ms: u64) -> bool {
    let want = (qpc as u128 * CONSOLE_HZ as u128 / qpf() as u128) as i128;
    (v as i128 - want).unsigned_abs() <= (CONSOLE_HZ / 1000 * tol_ms) as u128
}

#[test]
#[ignore = "needs a D3D12 device; run: cargo test -p d3d12_xs --lib hw_ -- --ignored --test-threads=1 (TOMBOX_HW_ADAPTER=warp for WARP)"]
fn hw_queue_timestamp_frequency_and_calibration_follow_the_console_99_75_mhz_clock() {
    let g = gpu();
    unsafe {
        let mut f = 0u64;
        assert_eq!(call!(g.a, 16, (*mut u64) -> i32, &mut f), 0);
        assert_eq!(f, CONSOLE_HZ);
        let host: crate::timestamp::FreqFn = core::mem::transmute(crate::present::real_queue_slot(16));
        let mut hf = 0u64;
        assert!(host(g.a as *mut c_void, &mut hf) >= 0 && hf != 0);
        let (g0, c0) = title_clock(g.a);
        assert!(on_qpc_base(g0, c0, 5), "{}: calibration gpu {g0} is not QPC {c0} on the 99.75 MHz base", if g.warp { "WARP" } else { "hardware" });
        let h = xgs_kmd_emu::open();
        let (mut ig, mut ic) = (0u64, 0u64);
        let mut req = [0u8; 0x18];
        req[8..16].copy_from_slice(&(&mut ig as *mut u64 as u64).to_le_bytes());
        req[16..24].copy_from_slice(&(&mut ic as *mut u64 as u64).to_le_bytes());
        let mut ret = 0u32;
        assert!(xgs_kmd_emu::device_io_control(h, 0x7900_c213, req.as_ptr() as *const c_void, 0x18, core::ptr::null_mut(), 0, &mut ret));
        xgs_kmd_emu::close(h);
        std::thread::sleep(Duration::from_millis(200));
        let (g1, c1) = title_clock(g.a);
        assert!(g0 <= ig && ig <= g1 && c0 <= ic && ic <= c1, "IOCTL 0x7900c213 ({ig}, {ic}) not between slot 17 samples ({g0}, {c0}) and ({g1}, {c1})");
        let dg = (g1 - g0) as f64 / CONSOLE_HZ as f64;
        let dc = (c1 - c0) as f64 / qpf() as f64;
        assert!((dg - dc).abs() < 0.002, "GPU clock advanced {dg} s while QPC advanced {dc} s");
        let (mut tg, mut tc) = (0u64, 0u64);
        assert_eq!(call!(g.b, 17, (*mut u64, *mut u64) -> i32, &mut tg, &mut tc), 0);
        assert!(on_qpc_base(tg, tc, 5));
    }
}

#[test]
#[ignore = "needs a D3D12 device; run: cargo test -p d3d12_xs --lib hw_ -- --ignored --test-threads=1 (TOMBOX_HW_ADAPTER=warp for WARP)"]
fn hw_end_query_timestamp_resolve_is_on_the_console_base_bracketed_by_calibration_and_keeps_predication() {
    let g = gpu();
    unsafe {
        let heap = timestamp_heap(g.dev, 128);
        let out = buffer(g.dev, HEAP_READBACK, 4096, 0, ST_COPY_DEST);
        let pred = buffer(g.dev, HEAP_UPLOAD, 256, 0, ST_GENERIC_READ);
        core::ptr::write_volatile(pred.cpu as *mut u64, 0);
        let src = buffer(g.dev, HEAP_UPLOAD, 256, 0, ST_GENERIC_READ);
        core::ptr::write_volatile(src.cpu as *mut u64, 0x5555_aaaa);
        let before = crate::ts99::converted();
        let l = title_list(g.dev);
        for i in 0..100u32 {
            call!(l, X_END_QUERY, (*mut c_void, u32, u32) -> (), heap, 2, i);
        }
        call!(l, X_PREDICATION, (*mut c_void, u64, u32) -> (), pred.res as *mut c_void, 0, 0);
        call!(l, X_RESOLVE_QUERY_DATA, (*mut c_void, u32, u32, u32, *mut c_void, u64) -> (), heap, 2, 3, 90, out.res as *mut c_void, 16);
        call!(l, X_COPY_BUFFER_REGION, (usize, u64, usize, u64, u64) -> (), out.res, 2048, src.res, 0, 8);
        call!(l, X_PREDICATION, (*mut c_void, u64, u32) -> (), core::ptr::null_mut::<c_void>(), 0, 0);
        call!(l, X_COPY_BUFFER_REGION, (usize, u64, usize, u64, u64) -> (), out.res, 2056, src.res, 0, 8);
        assert!(call!(l, X_CLOSE, () -> i32) >= 0);
        let f = fence(g.dev);
        let (gb, _) = title_clock(g.a);
        submit(g.a, l, f, 1);
        assert!(reached(f, 1, Duration::from_secs(5)));
        let (ga, _) = title_clock(g.a);
        assert!(crate::ts99::converted() >= before + 90);
        let q = |o: usize| core::ptr::read_volatile(out.cpu.add(o) as *const u64);
        let who = if g.warp { "WARP" } else { "hardware" };
        assert_eq!((q(0), q(8)), (0, 0), "resolve wrote before its destination offset");
        let mut last = 0;
        for i in 0..90 {
            let v = q(16 + 8 * i);
            assert!(v + 2 >= gb && v <= ga + 2, "{who}: timestamp {i} = {v} outside the GetClockCalibration bracket [{gb}, {ga}]");
            assert!(v >= last, "{who}: timestamps not monotonic at {i}");
            last = v;
        }
        assert_eq!(q(16 + 8 * 90), 0, "resolve wrote past its count");
        assert_eq!(q(2048), 0, "the title predication was not restored after the conversion");
        assert_eq!(q(2056), 0x5555_aaaa);
    }
}

#[test]
#[ignore = "needs a D3D12 device; run: cargo test -p d3d12_xs --lib hw_ -- --ignored --test-threads=1 (TOMBOX_HW_ADAPTER=warp for WARP)"]
fn hw_gpu_timestamp_writes_are_on_the_console_base_and_advance_with_wall_time() {
    let g = gpu();
    unsafe {
        let out = buffer(g.dev, HEAP_READBACK, 256, 0, ST_COPY_DEST);
        let bop = |l: *mut c_void, va: u64| call!(l, crate::cl_ext::CL_WRITE64_BOTTOM_OF_PIPE, (u64, u64, u32, u32) -> (), va, 0, 0, crate::cl_ext::BOP_FLAG_GPU_TIMESTAMP);
        let top = |l: *mut c_void, va: u64| call!(l, crate::console_slots::CL_WRITE64_TOP, (u64, u64, u32) -> (), va, 0, crate::console_slots::WRITE_TOP_FLAG_GPU_TIMESTAMP);
        let l = title_list(g.dev);
        top(l, out.va);
        bop(l, out.va + 8);
        assert!(call!(l, X_CLOSE, () -> i32) >= 0);
        let l2 = title_list(g.dev);
        bop(l2, out.va + 16);
        top(l2, out.va + 24);
        assert!(call!(l2, X_CLOSE, () -> i32) >= 0);
        let f = fence(g.dev);
        let (g0, c0) = title_clock(g.a);
        submit(g.a, l, f, 1);
        assert!(reached(f, 1, Duration::from_secs(5)));
        let (g1, _) = title_clock(g.a);
        std::thread::sleep(Duration::from_millis(200));
        let (g2, _) = title_clock(g.a);
        submit(g.a, l2, f, 2);
        assert!(reached(f, 2, Duration::from_secs(5)));
        let (g3, c3) = title_clock(g.a);
        let q = |o: usize| core::ptr::read_volatile(out.cpu.add(o) as *const u64);
        let v = [q(0), q(8), q(16), q(24)];
        let who = if g.warp { "WARP" } else { "hardware" };
        assert!(v[0] + 2 >= g0 && v[1] <= g1 + 2 && v[0] <= v[1], "{who}: list 1 {v:?} outside [{g0}, {g1}]");
        assert!(v[2] + 2 >= g2 && v[3] <= g3 + 2 && v[2] <= v[3], "{who}: list 2 {v:?} outside [{g2}, {g3}]");
        let secs = (v[2] - v[1]) as f64 / CONSOLE_HZ as f64;
        let wall = (c3 - c0) as f64 / qpf() as f64;
        assert!(secs >= 0.19 && secs <= wall + 0.002, "{who}: timestamps advanced {secs} s across a 200 ms sleep (wall {wall} s)");
    }
}

#[test]
#[ignore = "needs a D3D12 device; run: cargo test -p d3d12_xs --lib hw_ -- --ignored --test-threads=1 (TOMBOX_HW_ADAPTER=warp for WARP)"]
fn hw_dma_list_timestamp_is_converted_after_the_copy_queue_submission() {
    let g = gpu();
    unsafe {
        let desc: [u32; 4] = [3, 0, 0, 0];
        let mut cq: *mut c_void = core::ptr::null_mut();
        assert!(call!(g.dev, 8, (*const u32, *const Guid, *mut *mut c_void) -> i32, desc.as_ptr(), &IID_QUEUE, &mut cq) >= 0);
        crate::present::apply_queue_vtable(cq);
        let mut a: *mut c_void = core::ptr::null_mut();
        assert!(call!(g.dev, 9, (u32, *const Guid, *mut *mut c_void) -> i32, 3, &IID_ALLOCATOR, &mut a) >= 0);
        let mut rl: *mut c_void = core::ptr::null_mut();
        assert!(call!(g.dev, 12, (u32, u32, *mut c_void, *mut c_void, *const Guid, *mut *mut c_void) -> i32, 0, 3, a, core::ptr::null_mut(), &IID_LIST, &mut rl) >= 0);
        let l = crate::dma_list::wrap(rl);
        let out = buffer(g.dev, HEAP_READBACK, 256, 0, ST_COPY_DEST);
        call!(l, crate::dma_list::DMA_SLOT_TIMESTAMP, (u64, u32, u32, u32) -> (), out.va + 8, 0, 0, 0);
        call!(l, crate::dma_list::DMA_SLOT_TIMESTAMP, (u64, u32, u32, u32) -> (), out.va + 16, 0, 0, 0);
        assert!(call!(rl, 9, () -> i32) >= 0);
        let f = fence(g.dev);
        let (g0, _) = title_clock(g.a);
        submit(cq as usize, l, f, 1);
        assert!(reached(f, 1, Duration::from_secs(5)));
        let (g1, _) = title_clock(g.a);
        let q = |o: usize| core::ptr::read_volatile(out.cpu.add(o) as *const u64);
        let who = if g.warp { "WARP" } else { "hardware" };
        let (v0, v1) = (q(8), q(16));
        assert!(v0 != 0, "{who}: DMA timestamp not written");
        assert!(v0 + 2 >= g0 && v1 <= g1 + 2 && v0 <= v1, "{who}: DMA timestamps {v0} {v1} outside [{g0}, {g1}]");
        assert_eq!((q(0), q(24)), (0, 0));
        assert_eq!(crate::ts99::pending_of(&[rl]).len(), 2);
        call!(l, 10, (*mut c_void, *mut c_void) -> i32, a, core::ptr::null_mut());
        assert!(crate::ts99::pending_of(&[rl]).is_empty(), "Reset through the DMA list did not drop its pending timestamps");
    }
}

#[path = "cl_fw_hw.rs"]
mod cl_fw_hw;

#[path = "device_place_hw.rs"]
mod device_place_hw;

#[path = "upload_ww_hw.rs"]
mod upload_ww_hw;

#[test]
#[ignore = "needs a D3D12 device; run: cargo test -p d3d12_xs --lib hw_ -- --ignored --test-threads=1 (TOMBOX_HW_ADAPTER=warp for WARP)"]
fn hw_gpuprof_times_a_cross_queue_wait_and_the_submissions_around_it() {
    let g = gpu();
    unsafe {
        let mk = |ty: u32| -> *mut c_void {
            let desc: [u32; 4] = [ty, 0, 0, 0];
            let mut q: *mut c_void = core::ptr::null_mut();
            assert!(call!(g.dev, 8, (*const u32, *const Guid, *mut *mut c_void) -> i32, desc.as_ptr(), &IID_QUEUE, &mut q) >= 0 && !q.is_null());
            q
        };
        let (dq, cq, kq) = (mk(0), mk(2), mk(3));
        type Exec = unsafe extern "system" fn(*mut c_void, u32, *const *mut c_void);
        crate::gpuprof_q::set_real_for_test(vs(cq, 14), vs(cq, 15));
        let exec: Exec = core::mem::transmute(vs(cq, 10));
        let settle = |n: usize| {
            let t = Instant::now();
            while crate::gpuprof_q::current().len() < n && t.elapsed() < Duration::from_secs(5) {
                std::thread::sleep(Duration::from_millis(1));
            }
        };
        let who = if g.warp { "WARP" } else { "hardware" };
        let f = fence(g.dev);
        crate::gpuprof_q::arm();
        let s0 = crate::gpuprof::test_stamp(cq, exec, 2).expect("compute begin stamp");
        crate::gpuprof_q::q_wait(cq, f, 1);
        let s1 = crate::gpuprof::test_stamp(cq, exec, 2).expect("compute end stamp");
        std::thread::sleep(Duration::from_millis(60));
        crate::gpuprof_q::q_signal(dq, f, 1);
        let (pf, pv) = crate::gpuprof::test_fence();
        assert!(reached(pf as *mut c_void, pv, Duration::from_secs(5)), "{who}: stamp fence never reached {pv}");
        settle(1);
        let d = crate::gpuprof_q::current();
        assert_eq!(d.len(), 1, "{who}: {d:?}");
        assert_eq!(d[0].ty, 2);
        assert!(d[0].ms >= 50.0 && d[0].ms < 3000.0, "{who}: cross-queue wait measured {} ms across a 60 ms sleep", d[0].ms);
        crate::gpuprof_q::q_wait(cq, f, 1);
        let mut freq = 0u64;
        assert!(call!(cq, 16, (*mut u64) -> i32, &mut freq) >= 0 && freq != 0);
        let tk = crate::gpuprof::test_ticks(&[s0, s1]);
        assert!(tk.len() == 2 && tk[0] != 0 && tk[1] >= tk[0], "{who}: compute stamps {tk:?}");
        let span = (tk[1] - tk[0]) as f64 * 1000.0 / freq as f64;
        assert!(span >= 50.0, "{who}: compute submission span {span} ms does not cover the 60 ms cross-queue wait");
        let f2 = fence(g.dev);
        crate::gpuprof_q::name_fence(f2, "upload");
        crate::gpuprof_q::q_wait(dq, f2, 1);
        std::thread::sleep(Duration::from_millis(20));
        assert!(call!(f2, 10, (u64) -> i32, 1) >= 0);
        settle(3);
        let line = crate::gpuprof_q::report_line();
        assert!(line.contains("direct waits on upload fences n=1 already-reached=0"), "{who}: {line}");
        assert!(line.contains("compute waits on title fences n=2 already-reached=1"), "{who}: {line}");
        assert!(line.contains("queue signals: direct 1"), "{who}: {line}");
        let (pf, pv) = crate::gpuprof::test_fence();
        assert!(reached(pf as *mut c_void, pv, Duration::from_secs(5)));
        let before = crate::gpuprof::test_pool();
        for _ in 0..40 {
            assert!(crate::gpuprof::test_stamp(cq, exec, 2).is_some());
            let (pf, pv) = crate::gpuprof::test_fence();
            assert!(reached(pf as *mut c_void, pv, Duration::from_secs(5)));
        }
        assert!(crate::gpuprof::test_pool() <= before + 1, "{who}: stamp lists were not reused ({} -> {})", before, crate::gpuprof::test_pool());
        if crate::gpuprof::test_has_copy_heap() {
            let kexec: Exec = core::mem::transmute(vs(kq, 10));
            let a = crate::gpuprof::test_stamp(kq, kexec, 3).expect("copy begin stamp");
            let b = crate::gpuprof::test_stamp(kq, kexec, 3).expect("copy end stamp");
            let (pf, pv) = crate::gpuprof::test_fence();
            assert!(reached(pf as *mut c_void, pv, Duration::from_secs(5)));
            let tk = crate::gpuprof::test_ticks(&[a, b]);
            assert!(tk.len() == 2 && tk[0] != 0 && tk[1] >= tk[0], "{who}: copy-queue stamps {tk:?}");
        } else {
            eprintln!("{who}: no copy-queue timestamp heap; copy spans untested");
        }
        crate::gpuprof_q::disarm();
        for o in [f, f2, dq, cq, kq] {
            call!(o, 2, () -> u32);
        }
    }
}
