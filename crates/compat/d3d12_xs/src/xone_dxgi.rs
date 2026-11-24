use compat_common::record;
use core::ffi::c_void;
use core::sync::atomic::{AtomicI32, AtomicU32, Ordering};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

const S_OK: i32 = 0;
const E_POINTER: i32 = 0x8000_4003u32 as i32;
const E_INVALIDARG: i32 = 0x8007_0057u32 as i32;
const E_NOINTERFACE: i32 = 0x8000_4002u32 as i32;
const DXGI_ERROR_INVALID_CALL: i32 = 0x887A_0001u32 as i32;
const DXGI_ERROR_NOT_FOUND: i32 = 0x887A_0002u32 as i32;
const DXGI_ERROR_MORE_DATA: i32 = 0x887A_0003u32 as i32;
const DXGI_RESIDENCY_FULLY_RESIDENT: u32 = 1;

type G = [u8; 16];

const fn guid(d1: u32, d2: u16, d3: u16, d4: [u8; 8]) -> G {
    let a = d1.to_le_bytes();
    let b = d2.to_le_bytes();
    let c = d3.to_le_bytes();
    [a[0], a[1], a[2], a[3], b[0], b[1], c[0], c[1], d4[0], d4[1], d4[2], d4[3], d4[4], d4[5], d4[6], d4[7]]
}

const IID_IUNKNOWN: G = guid(0, 0, 0, [0xc0, 0, 0, 0, 0, 0, 0, 0x46]);
const IID_IDXGIOBJECT: G = guid(0xaec2_2fb8, 0x76f3, 0x4639, [0x9b, 0xe0, 0x28, 0xeb, 0x43, 0xa6, 0x7a, 0x2e]);
const IID_IDXGIDEVICE: G = guid(0x54ec_77fa, 0x1377, 0x44e6, [0x8c, 0x32, 0x88, 0xfd, 0x5f, 0x44, 0xc8, 0x4c]);
const IID_IDXGIDEVICE1: G = guid(0x77db_970f, 0x6276, 0x48ba, [0xba, 0x28, 0x07, 0x01, 0x43, 0xb4, 0x39, 0x2c]);
const IID_IDXGIFACTORY4: G = guid(0x1bc6_ea02, 0xef36, 0x464f, [0xbf, 0x0c, 0x21, 0xca, 0x39, 0xe5, 0x16, 0x8a]);
const IID_IDXGIADAPTER: G = guid(0x2411_e7e1, 0x12ac, 0x4ccf, [0xbd, 0x14, 0x97, 0x98, 0xe8, 0x53, 0x4d, 0xc0]);

enum Private {
    Bytes(Vec<u8>),
    Iface(usize),
}

#[repr(C)]
struct XoneDxgiDevice {
    vtable: *const usize,
    refs: AtomicU32,
    device: *mut c_void,
    adapter: usize,
    priority: AtomicI32,
    latency: AtomicU32,
    private: Mutex<HashMap<G, Private>>,
}

unsafe fn slot(o: *mut c_void, i: usize) -> usize {
    unsafe { *(*(o as *const *const usize)).add(i) }
}

unsafe fn com_addref(o: usize) {
    if o != 0 {
        let f: unsafe extern "system" fn(usize) -> u32 = unsafe { core::mem::transmute(slot(o as *mut c_void, 1)) };
        unsafe { f(o) };
    }
}

pub(crate) unsafe fn com_release(o: usize) -> u32 {
    if o == 0 {
        return 0;
    }
    let f: unsafe extern "system" fn(usize) -> u32 = unsafe { core::mem::transmute(slot(o as *mut c_void, 2)) };
    unsafe { f(o) }
}

unsafe fn com_qi(o: usize, riid: *const G, ppv: *mut *mut c_void) -> i32 {
    let f: unsafe extern "system" fn(usize, *const G, *mut *mut c_void) -> i32 = unsafe { core::mem::transmute(slot(o as *mut c_void, 0)) };
    unsafe { f(o, riid, ppv) }
}

fn guid_text(g: &G) -> String {
    let d1 = u32::from_le_bytes([g[0], g[1], g[2], g[3]]);
    let d2 = u16::from_le_bytes([g[4], g[5]]);
    let d3 = u16::from_le_bytes([g[6], g[7]]);
    format!("{{{d1:08x}-{d2:04x}-{d3:04x}-{:02x}{:02x}-{}}}", g[8], g[9], g[10..].iter().map(|b| format!("{b:02x}")).collect::<String>())
}

extern "system" {
    fn LoadLibraryW(p: *const u16) -> *mut c_void;
    fn GetProcAddress(h: *mut c_void, n: *const u8) -> *mut c_void;
}

fn host_factory() -> Result<*mut c_void, i32> {
    let name: Vec<u16> = r"C:\Windows\System32\dxgi.dll".encode_utf16().chain(std::iter::once(0)).collect();
    let dxgi = unsafe { LoadLibraryW(name.as_ptr()) };
    if dxgi.is_null() {
        record("d3d12_xs", "xone.IDXGIDevice", "dxgi.dll not loadable -> no adapter");
        return Err(DXGI_ERROR_NOT_FOUND);
    }
    let cf = unsafe { GetProcAddress(dxgi, b"CreateDXGIFactory2\0".as_ptr()) };
    if cf.is_null() {
        return Err(E_NOINTERFACE);
    }
    let cf: unsafe extern "system" fn(u32, *const G, *mut *mut c_void) -> i32 = unsafe { core::mem::transmute(cf) };
    let mut factory: *mut c_void = core::ptr::null_mut();
    let hr = unsafe { cf(0, &IID_IDXGIFACTORY4, &mut factory) };
    if hr != S_OK || factory.is_null() {
        record("d3d12_xs", "xone.IDXGIDevice", &format!("CreateDXGIFactory2(IDXGIFactory4) hr={hr:#010x}"));
        return Err(if hr < 0 { hr } else { E_POINTER });
    }
    Ok(factory)
}

pub(crate) fn warp_adapter() -> Result<*mut c_void, i32> {
    let factory = host_factory()?;
    let mut adapter = core::ptr::null_mut();
    let enumerate: unsafe extern "system" fn(*mut c_void, *const G, *mut *mut c_void) -> i32 =
        unsafe { core::mem::transmute(slot(factory, 27)) };
    let hr = unsafe { enumerate(factory, &IID_IDXGIADAPTER, &mut adapter) };
    unsafe { com_release(factory as usize) };
    if hr < 0 { Err(hr) } else if adapter.is_null() { Err(E_POINTER) } else { Ok(adapter) }
}

fn host_adapter(real_device: *mut c_void) -> usize {
    let Ok(factory) = host_factory() else { return 0 };
    let mut luid = 0u64;
    if !real_device.is_null() {
        let get_luid: unsafe extern "system" fn(*mut c_void, *mut u64) -> *mut u64 = unsafe { core::mem::transmute(slot(real_device, 43)) };
        unsafe { get_luid(real_device, &mut luid) };
    }
    let mut adapter: *mut c_void = core::ptr::null_mut();
    let by_luid: unsafe extern "system" fn(*mut c_void, u64, *const G, *mut *mut c_void) -> i32 = unsafe { core::mem::transmute(slot(factory, 26)) };
    let mut hr = unsafe { by_luid(factory, luid, &IID_IDXGIADAPTER, &mut adapter) };
    let mut how = "EnumAdapterByLuid";
    if hr != S_OK || adapter.is_null() {
        let enum0: unsafe extern "system" fn(*mut c_void, u32, *mut *mut c_void) -> i32 = unsafe { core::mem::transmute(slot(factory, 7)) };
        hr = unsafe { enum0(factory, 0, &mut adapter) };
        how = "EnumAdapters(0)";
    }
    unsafe { com_release(factory as usize) };
    record("d3d12_xs", "xone.IDXGIDevice", &format!("host adapter via {how} luid={luid:#x} hr={hr:#010x} adapter={adapter:p}"));
    if hr == S_OK { adapter as usize } else { 0 }
}

pub fn query(device: *mut c_void, real_device: *mut c_void, riid: *const c_void, ppv: *mut *mut c_void) -> Option<i32> {
    let g = unsafe { *(riid as *const G) };
    if g != IID_IDXGIDEVICE {
        return None;
    }
    unsafe { com_addref(device as usize) };
    let obj = Box::new(XoneDxgiDevice {
        vtable: vtable(),
        refs: AtomicU32::new(1),
        device,
        adapter: host_adapter(real_device),
        priority: AtomicI32::new(0),
        latency: AtomicU32::new(3),
        private: Mutex::new(HashMap::new()),
    });
    unsafe { *ppv = Box::into_raw(obj) as *mut c_void };
    record("d3d12_xs", "device.QueryInterface", &format!("{} -> Xbox One IDXGIDevice1 on the D3D12 device S_OK", guid_text(&g)));
    Some(S_OK)
}

unsafe extern "system" fn qi(this: *mut XoneDxgiDevice, riid: *const G, ppv: *mut *mut c_void) -> i32 {
    if ppv.is_null() || riid.is_null() {
        return E_POINTER;
    }
    unsafe { *ppv = core::ptr::null_mut() };
    let g = unsafe { *riid };
    if g == IID_IUNKNOWN || g == IID_IDXGIOBJECT || g == IID_IDXGIDEVICE || g == IID_IDXGIDEVICE1 {
        unsafe {
            (*this).refs.fetch_add(1, Ordering::SeqCst);
            *ppv = this as *mut c_void;
        }
        return S_OK;
    }
    let hr = unsafe { com_qi((*this).device as usize, riid, ppv) };
    if hr != S_OK {
        record("d3d12_xs", "xone.IDXGIDevice.QueryInterface", &format!("{} -> {hr:#010x}", guid_text(&g)));
        return E_NOINTERFACE;
    }
    hr
}

unsafe extern "system" fn addref(this: *mut XoneDxgiDevice) -> u32 {
    unsafe { (*this).refs.fetch_add(1, Ordering::SeqCst) + 1 }
}

unsafe extern "system" fn release(this: *mut XoneDxgiDevice) -> u32 {
    let n = unsafe { (*this).refs.fetch_sub(1, Ordering::SeqCst) } - 1;
    if n == 0 {
        unsafe {
            let b = Box::from_raw(this);
            for (_, v) in b.private.lock().unwrap().drain() {
                if let Private::Iface(p) = v {
                    com_release(p);
                }
            }
            com_release(b.adapter);
            com_release(b.device as usize);
        }
    }
    n
}

unsafe extern "system" fn set_private_data(this: *mut XoneDxgiDevice, name: *const G, size: u32, data: *const u8) -> i32 {
    if name.is_null() {
        return E_INVALIDARG;
    }
    let key = unsafe { *name };
    let mut m = unsafe { (*this).private.lock().unwrap() };
    let old = if size == 0 || data.is_null() {
        m.remove(&key)
    } else {
        m.insert(key, Private::Bytes(unsafe { core::slice::from_raw_parts(data, size as usize) }.to_vec()))
    };
    if let Some(Private::Iface(p)) = old {
        unsafe { com_release(p) };
    }
    S_OK
}

unsafe extern "system" fn set_private_data_interface(this: *mut XoneDxgiDevice, name: *const G, unk: *mut c_void) -> i32 {
    if name.is_null() {
        return E_INVALIDARG;
    }
    let key = unsafe { *name };
    let mut m = unsafe { (*this).private.lock().unwrap() };
    let old = if unk.is_null() {
        m.remove(&key)
    } else {
        unsafe { com_addref(unk as usize) };
        m.insert(key, Private::Iface(unk as usize))
    };
    if let Some(Private::Iface(p)) = old {
        unsafe { com_release(p) };
    }
    S_OK
}

unsafe extern "system" fn get_private_data(this: *mut XoneDxgiDevice, name: *const G, size: *mut u32, data: *mut u8) -> i32 {
    if name.is_null() || size.is_null() {
        return E_INVALIDARG;
    }
    let m = unsafe { (*this).private.lock().unwrap() };
    let Some(v) = m.get(unsafe { &*name }) else {
        unsafe { *size = 0 };
        return DXGI_ERROR_NOT_FOUND;
    };
    let bytes: Vec<u8> = match v {
        Private::Bytes(b) => b.clone(),
        Private::Iface(p) => p.to_le_bytes().to_vec(),
    };
    let have = unsafe { *size } as usize;
    unsafe { *size = bytes.len() as u32 };
    if data.is_null() {
        return S_OK;
    }
    if have < bytes.len() {
        return DXGI_ERROR_MORE_DATA;
    }
    if let Private::Iface(p) = v {
        unsafe { com_addref(*p) };
    }
    unsafe { core::ptr::copy_nonoverlapping(bytes.as_ptr(), data, bytes.len()) };
    S_OK
}

unsafe extern "system" fn get_parent(this: *mut XoneDxgiDevice, riid: *const G, ppv: *mut *mut c_void) -> i32 {
    if ppv.is_null() || riid.is_null() {
        return E_INVALIDARG;
    }
    unsafe { *ppv = core::ptr::null_mut() };
    let a = unsafe { (*this).adapter };
    if a == 0 {
        record("d3d12_xs", "xone.IDXGIDevice.GetParent", "no host adapter -> DXGI_ERROR_NOT_FOUND");
        return DXGI_ERROR_NOT_FOUND;
    }
    let hr = unsafe { com_qi(a, riid, ppv) };
    record("d3d12_xs", "xone.IDXGIDevice.GetParent", &format!("{} -> host adapter hr={hr:#010x}", guid_text(unsafe { &*riid })));
    hr
}

unsafe extern "system" fn get_adapter(this: *mut XoneDxgiDevice, pp: *mut *mut c_void) -> i32 {
    if pp.is_null() {
        return E_INVALIDARG;
    }
    let a = unsafe { (*this).adapter };
    unsafe { *pp = a as *mut c_void };
    if a == 0 {
        return DXGI_ERROR_NOT_FOUND;
    }
    unsafe { com_addref(a) };
    S_OK
}

unsafe extern "system" fn create_surface(_this: *mut XoneDxgiDevice, _desc: *const c_void, _n: u32, _usage: u32, _shared: *const c_void, pp: *mut *mut c_void) -> i32 {
    if !pp.is_null() {
        unsafe { *pp = core::ptr::null_mut() };
    }
    record("d3d12_xs", "xone.IDXGIDevice.CreateSurface", "D3D12 devices have no DXGI surfaces -> DXGI_ERROR_INVALID_CALL");
    DXGI_ERROR_INVALID_CALL
}

unsafe extern "system" fn query_resource_residency(_this: *mut XoneDxgiDevice, res: *const *mut c_void, status: *mut u32, n: u32) -> i32 {
    if (res.is_null() || status.is_null()) && n != 0 {
        return E_INVALIDARG;
    }
    for i in 0..n as usize {
        if unsafe { *res.add(i) }.is_null() {
            return E_INVALIDARG;
        }
        unsafe { *status.add(i) = DXGI_RESIDENCY_FULLY_RESIDENT };
    }
    S_OK
}

unsafe extern "system" fn set_gpu_thread_priority(this: *mut XoneDxgiDevice, p: i32) -> i32 {
    if !(-7..=7).contains(&p) {
        return E_INVALIDARG;
    }
    unsafe { (*this).priority.store(p, Ordering::Relaxed) };
    record("d3d12_xs", "xone.IDXGIDevice.SetGPUThreadPriority", &format!("{p}"));
    S_OK
}

unsafe extern "system" fn get_gpu_thread_priority(this: *mut XoneDxgiDevice, p: *mut i32) -> i32 {
    if p.is_null() {
        return E_POINTER;
    }
    unsafe { *p = (*this).priority.load(Ordering::Relaxed) };
    S_OK
}

unsafe extern "system" fn set_maximum_frame_latency(this: *mut XoneDxgiDevice, n: u32) -> i32 {
    if n > 16 {
        return DXGI_ERROR_INVALID_CALL;
    }
    let n = if n == 0 { 3 } else { n };
    unsafe { (*this).latency.store(n, Ordering::Relaxed) };
    record("d3d12_xs", "xone.IDXGIDevice.SetMaximumFrameLatency", &format!("{n}"));
    S_OK
}

unsafe extern "system" fn get_maximum_frame_latency(this: *mut XoneDxgiDevice, p: *mut u32) -> i32 {
    if p.is_null() {
        return DXGI_ERROR_INVALID_CALL;
    }
    unsafe { *p = (*this).latency.load(Ordering::Relaxed) };
    S_OK
}

fn vtable() -> *const usize {
    static VT: OnceLock<usize> = OnceLock::new();
    *VT.get_or_init(|| {
        let v: Vec<usize> = vec![
            qi as usize,
            addref as usize,
            release as usize,
            set_private_data as usize,
            set_private_data_interface as usize,
            get_private_data as usize,
            get_parent as usize,
            get_adapter as usize,
            create_surface as usize,
            query_resource_residency as usize,
            set_gpu_thread_priority as usize,
            get_gpu_thread_priority as usize,
            set_maximum_frame_latency as usize,
            get_maximum_frame_latency as usize,
        ];
        Box::leak(v.into_boxed_slice()).as_ptr() as usize
    }) as *const usize
}
