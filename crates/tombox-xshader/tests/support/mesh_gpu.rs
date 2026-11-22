#![allow(dead_code)]

use crate::d3d::{self, Gpu, Guid};
use crate::vcall;
use core::ffi::c_void;

extern "system" {
    fn LoadLibraryA(n: *const u8) -> *mut c_void;
    fn GetProcAddress(h: *mut c_void, n: *const u8) -> *mut c_void;
}

pub unsafe fn warp_device() -> *mut c_void {
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
    d3d::release(ad);
    d3d::release(f);
    device
}

unsafe fn gpu_on(device: *mut c_void) -> Gpu {
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

unsafe fn mesh_tier(device: *mut c_void) -> u32 {
    let mut o7 = [0u32; 2];
    let hr = vcall!(device, 13, fn(u32, *mut c_void, u32) -> i32, 32, o7.as_mut_ptr() as *mut c_void, 8);
    if hr < 0 { 0 } else { o7[0] }
}

pub unsafe fn mesh_gpu() -> (Gpu, &'static str) {
    if std::env::var_os("TOMBOX_EXEC_HW").is_none() {
        let d = warp_device();
        assert!(mesh_tier(d) != 0, "WARP reports MeshShaderTier 0: rerun with TOMBOX_EXEC_HW=1 to use the hardware adapter");
        return (gpu_on(d), "WARP");
    }
    let mut g = Gpu::new(false).expect("hardware d3d12 device");
    g.open = false;
    assert!(mesh_tier(g.device) != 0, "neither WARP nor the hardware adapter supports mesh shaders");
    (g, "hardware")
}

pub const IID_DEVICE2: Guid = Guid(0x30baa41e, 0xb15b, 0x475c, [0xa0, 0xbb, 0x1a, 0xf5, 0xc5, 0xb6, 0x43, 0x28]);
const IID_LIST6: Guid = Guid(0xc3827890, 0xe548, 0x4cfa, [0x96, 0xcf, 0x56, 0x89, 0xa9, 0x37, 0x0f, 0x80]);

fn sub(stream: &mut Vec<u8>, ty: u32, body: &[u8]) {
    stream.extend_from_slice(&ty.to_le_bytes());
    let align = if matches!(ty, 0..=6 | 19 | 24 | 25) { 8 } else { 4 };
    while stream.len() % align != 0 {
        stream.push(0);
    }
    stream.extend_from_slice(body);
    while stream.len() % 8 != 0 {
        stream.push(0);
    }
}

fn bytecode(b: &[u8]) -> Vec<u8> {
    let mut v = (b.as_ptr() as u64).to_le_bytes().to_vec();
    v.extend_from_slice(&(b.len() as u64).to_le_bytes());
    v
}

pub fn mesh_stream(rs: *mut c_void, ms: &[u8], ps: &[u8]) -> Vec<u8> {
    let mut s = Vec::new();
    sub(&mut s, 25, &bytecode(ms));
    sub(&mut s, 2, &bytecode(ps));
    let mut rast = [0u32; 11];
    rast[0] = 3;
    rast[1] = 1;
    rast[6] = 1;
    sub(&mut s, 10, &rast.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>());
    let mut ds = [0u32; 13];
    ds[2] = 8;
    ds[4] = 0xffff;
    for k in [5usize, 9] {
        ds[k] = 1;
        ds[k + 1] = 1;
        ds[k + 2] = 1;
        ds[k + 3] = 8;
    }
    sub(&mut s, 11, &ds.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>());
    let mut rtf = [0u32; 9];
    rtf[0] = 3;
    rtf[8] = 1;
    sub(&mut s, 15, &rtf.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>());
    sub(&mut s, 17, &[1, 0, 0, 0, 0, 0, 0, 0]);
    sub(&mut s, 9, &0xffff_ffffu32.to_le_bytes());
    if !rs.is_null() {
        sub(&mut s, 0, &(rs as u64).to_le_bytes());
    }
    s

}

pub unsafe fn render_pixel(gpu: &mut Gpu, pso: *mut c_void, rs: *mut c_void, cbv: Option<u64>, groups: u32) -> Result<[u32; 4], String> {
    let mut td = [0u8; 0x38];
    td[0..4].copy_from_slice(&3u32.to_le_bytes());
    td[0x10..0x18].copy_from_slice(&1u64.to_le_bytes());
    td[0x18..0x1c].copy_from_slice(&1u32.to_le_bytes());
    td[0x1c..0x1e].copy_from_slice(&1u16.to_le_bytes());
    td[0x1e..0x20].copy_from_slice(&1u16.to_le_bytes());
    td[0x20..0x24].copy_from_slice(&3u32.to_le_bytes());
    td[0x24..0x28].copy_from_slice(&1u32.to_le_bytes());
    td[0x30..0x34].copy_from_slice(&1u32.to_le_bytes());
    let rt = gpu.committed(&td, d3d::default_heap(), 4)?;
    let rb = gpu.buffer(256, 3, d3d::COPY_DEST, false)?;
    let (heap, rtv, _, _) = gpu.heap(2, 1, false)?;
    vcall!(gpu.device, 20, fn(*mut c_void, *const c_void, usize) -> (), rt, core::ptr::null(), rtv);
    gpu.begin();
    let l = gpu.list;
    let clear = [5.0f32, 6.0, 7.0, 8.0];
    vcall!(l, 48, fn(usize, *const f32, u32, *const c_void) -> (), rtv, clear.as_ptr(), 0, core::ptr::null());
    vcall!(l, 46, fn(u32, *const usize, i32, *const usize) -> (), 1, &rtv, 0, core::ptr::null());
    vcall!(l, 30, fn(*mut c_void) -> (), rs);
    if let Some(address) = cbv {
        vcall!(l, 38, fn(u32, u64) -> (), 0, address);
    }
    vcall!(l, 25, fn(*mut c_void) -> (), pso);
    let vp = [0f32, 0., 1., 1., 0., 1.];
    vcall!(l, 21, fn(u32, *const f32) -> (), 1, vp.as_ptr());
    let sc = [0i32, 0, 1, 1];
    vcall!(l, 22, fn(u32, *const i32) -> (), 1, sc.as_ptr());
    let mut l6: *mut c_void = core::ptr::null_mut();
    assert!(vcall!(l, 0, fn(*const Guid, *mut *mut c_void) -> i32, &IID_LIST6, &mut l6) >= 0, "ID3D12GraphicsCommandList6");
    vcall!(l6, 79, fn(u32, u32, u32) -> (), groups, 1, 1);
    d3d::release(l6);
    gpu.barrier(rt, 4, d3d::COPY_SOURCE);
    let mut dst2 = [0u8; 48];
    dst2[0..8].copy_from_slice(&(rb as u64).to_le_bytes());
    dst2[8..12].copy_from_slice(&1u32.to_le_bytes());
    dst2[24..28].copy_from_slice(&3u32.to_le_bytes());
    dst2[28..32].copy_from_slice(&1u32.to_le_bytes());
    dst2[32..36].copy_from_slice(&1u32.to_le_bytes());
    dst2[36..40].copy_from_slice(&1u32.to_le_bytes());
    dst2[40..44].copy_from_slice(&256u32.to_le_bytes());
    let mut src = [0u8; 48];
    src[0..8].copy_from_slice(&(rt as u64).to_le_bytes());
    vcall!(l, 16, fn(*const u8, u32, u32, u32, *const u8, *const c_void) -> (), dst2.as_ptr(), 0, 0, 0, src.as_ptr(), core::ptr::null());
    gpu.submit()?;
    let b = Gpu::map_read(rb, 16).ok_or("map readback")?;
    d3d::release(rt);
    d3d::release(rb);
    d3d::release(heap);
    let mut o = [0u32; 4];
    for i in 0..4 {
        o[i] = u32::from_le_bytes(b[i * 4..i * 4 + 4].try_into().unwrap());
    }
    Ok(o)
}
