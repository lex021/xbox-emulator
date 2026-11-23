//! Minimal shim for the Xbox "DXGIDevice" interface
//! ({77db970f-6276-48ba-ba28-070143b4392c}) that `FXboxCommonD3D12Adapter::
//! CreateRootDevice` obtains via `ID3D12Device::QueryInterface`.
//!
//! On Xbox the D3D12 device also implements an IDXGIDevice-shaped interface. PC
//! D3D12 devices do not, but the RHI only uses `IDXGIDevice::GetAdapter` (vtable
//! slot 7); the returned adapter and its parent factory are **standard PC DXGI**
//! (the factory IID is the real PC `IDXGIFactory2`), so they work natively.
//!
//! We therefore synthesize a tiny IDXGIDevice whose `GetAdapter` returns a real
//! PC `IDXGIAdapter` (from `CreateDXGIFactory2`→`EnumAdapters`). All other slots
//! are stubs (the RHI does not call them here).

use core::ffi::c_void;
use core::sync::atomic::{AtomicU32, Ordering};
use std::sync::OnceLock;
use compat_common::record;
use crate::device_wrap::Guid;

const S_OK: i32 = 0;
const E_NOTIMPL: i32 = 0x8000_4001u32 as i32;
const E_NOINTERFACE: i32 = 0x8000_4002u32 as i32;
const E_FAIL: i32 = 0x8000_4005u32 as i32;

pub const IID_XBOX_DXGIDEVICE: Guid = Guid {
    d1: 0x77db_970f, d2: 0x6276, d3: 0x48ba, d4: [0xba, 0x28, 0x07, 0x01, 0x43, 0xb4, 0x39, 0x2c],
};
const IID_IUNKNOWN: Guid = Guid { d1: 0, d2: 0, d3: 0, d4: [0xC0, 0, 0, 0, 0, 0, 0, 0x46] };
const IID_IDXGIFACTORY2: Guid = Guid {
    d1: 0x50c8_3a1c, d2: 0xe072, d3: 0x4c48, d4: [0x87, 0xb0, 0x36, 0x30, 0xfa, 0x36, 0xa6, 0xd0],
};

type HMODULE = *mut c_void;
extern "system" {
    fn LoadLibraryW(p: *const u16) -> HMODULE;
    fn GetProcAddress(h: HMODULE, n: *const u8) -> *mut c_void;
}

#[repr(C)]
struct DxgiDeviceShim {
    vtable: *const usize,
    adapter: usize, // real IDXGIAdapter (holds a ref)
    refs: AtomicU32,
}

/// Create a real PC IDXGIAdapter (adapter 0 — the default adapter the D3D12
/// device was created against). Returns the adapter pointer (ref owned).
fn make_real_adapter() -> usize {
    let dxgi = unsafe {
        let name: Vec<u16> = r"C:\Windows\System32\dxgi.dll".encode_utf16().chain(std::iter::once(0)).collect();
        LoadLibraryW(name.as_ptr())
    };
    if dxgi.is_null() {
        record("d3d12_xs", "dxgi", "LoadLibrary(dxgi.dll) failed");
        return 0;
    }
    let create_f2 = unsafe { GetProcAddress(dxgi, b"CreateDXGIFactory2\0".as_ptr()) };
    if create_f2.is_null() {
        return 0;
    }
    let create_f2: unsafe extern "system" fn(u32, *const Guid, *mut *mut c_void) -> i32 =
        unsafe { core::mem::transmute(create_f2) };
    let mut factory: *mut c_void = core::ptr::null_mut();
    let hr = unsafe { create_f2(0, &IID_IDXGIFACTORY2, &mut factory) };
    if hr != S_OK || factory.is_null() {
        record("d3d12_xs", "dxgi", &format!("CreateDXGIFactory2 hr={hr:#010x}"));
        return 0;
    }
    // factory->EnumAdapters(0, &adapter)  [IDXGIFactory vtable slot 7]
    let fvt = unsafe { *(factory as *const *const usize) };
    let enum_adapters: unsafe extern "system" fn(*mut c_void, u32, *mut *mut c_void) -> i32 =
        unsafe { core::mem::transmute(*fvt.add(7)) };
    let mut adapter: *mut c_void = core::ptr::null_mut();
    let hr = unsafe { enum_adapters(factory, 0, &mut adapter) };
    // Release our factory ref; the adapter keeps the factory alive (GetParent).
    let frel: unsafe extern "system" fn(*mut c_void) -> u32 = unsafe { core::mem::transmute(*fvt.add(2)) };
    unsafe { frel(factory) };
    record("d3d12_xs", "dxgi", &format!("EnumAdapters(0) hr={hr:#010x} adapter={adapter:p}"));
    adapter as usize
}

unsafe extern "system" fn qi(this: *mut DxgiDeviceShim, riid: *const Guid, ppv: *mut *mut c_void) -> i32 {
    if ppv.is_null() || riid.is_null() {
        return E_FAIL;
    }
    unsafe { *ppv = core::ptr::null_mut() };
    let g = unsafe { &*riid };
    if *g == IID_XBOX_DXGIDEVICE || *g == IID_IUNKNOWN {
        unsafe {
            *ppv = this as *mut c_void;
            (*this).refs.fetch_add(1, Ordering::SeqCst);
        }
        return S_OK;
    }
    E_NOINTERFACE
}

unsafe extern "system" fn addref(this: *mut DxgiDeviceShim) -> u32 {
    unsafe { (*this).refs.fetch_add(1, Ordering::SeqCst) + 1 }
}

unsafe extern "system" fn release(this: *mut DxgiDeviceShim) -> u32 {
    let n = unsafe { (*this).refs.fetch_sub(1, Ordering::SeqCst) } - 1;
    if n == 0 {
        unsafe {
            let a = (*this).adapter;
            if a != 0 {
                let avt = *(a as *const *const usize);
                let arel: unsafe extern "system" fn(*mut c_void) -> u32 = core::mem::transmute(*avt.add(2));
                arel(a as *mut c_void);
            }
            drop(Box::from_raw(this));
        }
    }
    n
}

/// IDXGIDevice::GetAdapter(this, IDXGIAdapter** ppAdapter)  [slot 7]
unsafe extern "system" fn get_adapter(this: *mut DxgiDeviceShim, pp_adapter: *mut *mut c_void) -> i32 {
    let a = unsafe { (*this).adapter };
    if a == 0 || pp_adapter.is_null() {
        record("d3d12_xs", "DXGIDevice::GetAdapter", "no adapter -> E_FAIL");
        return E_FAIL;
    }
    unsafe {
        // AddRef the adapter we hand out.
        let avt = *(a as *const *const usize);
        let aaddref: unsafe extern "system" fn(*mut c_void) -> u32 = core::mem::transmute(*avt.add(1));
        aaddref(a as *mut c_void);
        *pp_adapter = a as *mut c_void;
    }
    record("d3d12_xs", "DXGIDevice::GetAdapter", &format!("-> {a:#x} S_OK"));
    S_OK
}

/// Generic HRESULT stub for the IDXGIDevice slots the RHI does not call.
unsafe extern "system" fn stub() -> i32 {
    E_NOTIMPL
}

fn vtable() -> *const usize {
    static VT: OnceLock<usize> = OnceLock::new();
    *VT.get_or_init(|| {
        // IUnknown(0-2), IDXGIObject(3-6), IDXGIDevice: GetAdapter(7), CreateSurface(8),
        // QueryResourceResidency(9), SetGPUThreadPriority(10), GetGPUThreadPriority(11)
        let v: Vec<usize> = vec![
            qi as usize, addref as usize, release as usize,
            stub as usize, stub as usize, stub as usize, stub as usize, // IDXGIObject
            get_adapter as usize, // 7
            stub as usize, stub as usize, stub as usize, stub as usize, // 8-11
        ];
        Box::leak(v.into_boxed_slice()).as_ptr() as usize
    }) as *const usize
}

/// Create the DXGIDevice shim (returned to the game from device QueryInterface).
pub fn create() -> *mut c_void {
    let adapter = make_real_adapter();
    let shim = Box::new(DxgiDeviceShim { vtable: vtable(), adapter, refs: AtomicU32::new(1) });
    Box::into_raw(shim) as *mut c_void
}
