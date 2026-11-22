use core::ffi::c_void;

use crate::rs_strip::{rts0_flags_at, XBOX_ROOT_FLAGS};

const XBOX_MEMORY_ABI: u32 = 0x0100_0000;
const ALLOW_IA: u32 = 0x1;

#[repr(C)]
struct Desc {
    num_params: u32,
    params: *const c_void,
    num_samplers: u32,
    samplers: *const c_void,
    flags: u32,
}

#[repr(C)]
struct Versioned {
    version: u32,
    desc: Desc,
}

unsafe fn vslot(o: *mut c_void, i: usize) -> usize {
    unsafe { *(*(o as *const *const usize)).add(i) }
}

unsafe fn blob_bytes(b: *mut c_void) -> Vec<u8> {
    unsafe {
        let ptr: unsafe extern "system" fn(*mut c_void) -> *const u8 = core::mem::transmute(vslot(b, 3));
        let size: unsafe extern "system" fn(*mut c_void) -> usize = core::mem::transmute(vslot(b, 4));
        core::slice::from_raw_parts(ptr(b), size(b)).to_vec()
    }
}

unsafe fn release(o: *mut c_void) {
    if !o.is_null() {
        let f: unsafe extern "system" fn(*mut c_void) -> u32 = unsafe { core::mem::transmute(vslot(o, 2)) };
        unsafe { f(o) };
    }
}

fn flags_of(blob: &[u8]) -> u32 {
    let at = rts0_flags_at(blob).expect("RTS0 flags");
    u32::from_le_bytes(blob[at..at + 4].try_into().unwrap())
}

#[test]
fn desc_layout_matches_d3d12() {
    assert_eq!(core::mem::offset_of!(Desc, flags), crate::rs_strip::DESC_FLAGS);
    assert_eq!(core::mem::offset_of!(Versioned, desc) + core::mem::offset_of!(Desc, flags), crate::rs_strip::VERSIONED_DESC_FLAGS);
}

#[test]
fn serialize_strips_xbox_flags_for_host_and_keeps_them_in_blob() {
    let d = Desc { num_params: 0, params: core::ptr::null(), num_samplers: 0, samplers: core::ptr::null(), flags: ALLOW_IA | XBOX_MEMORY_ABI };
    let (mut blob, mut err): (*mut c_void, *mut c_void) = (core::ptr::null_mut(), core::ptr::null_mut());
    let hr = crate::D3D12SerializeRootSignature(&d as *const Desc as *const c_void, 1, &mut blob, &mut err);
    assert!(hr >= 0, "hr={hr:#010x}");
    assert_eq!(d.flags, ALLOW_IA | XBOX_MEMORY_ABI);
    let bytes = unsafe { blob_bytes(blob) };
    assert_eq!(flags_of(&bytes), ALLOW_IA | XBOX_MEMORY_ABI);
    let mut de: *mut c_void = core::ptr::null_mut();
    let hr = crate::D3D12CreateRootSignatureDeserializer(bytes.as_ptr() as *const c_void, bytes.len(), crate::iids::IID_ID3D12RootSignatureDeserializer.as_ptr() as *const c_void, &mut de);
    assert!(hr >= 0, "deserializer hr={hr:#010x}");
    let get: unsafe extern "system" fn(*mut c_void) -> *const Desc = unsafe { core::mem::transmute(vslot(de, 3)) };
    assert_eq!(unsafe { (*get(de)).flags }, ALLOW_IA | XBOX_MEMORY_ABI);
    unsafe {
        release(de);
        release(blob);
        release(err);
    }
}

#[test]
fn serialize_versioned_strips_xbox_flags_for_host() {
    let v = Versioned { version: 1, desc: Desc { num_params: 0, params: core::ptr::null(), num_samplers: 0, samplers: core::ptr::null(), flags: ALLOW_IA | XBOX_ROOT_FLAGS } };
    let (mut blob, mut err): (*mut c_void, *mut c_void) = (core::ptr::null_mut(), core::ptr::null_mut());
    let hr = crate::D3D12SerializeVersionedRootSignature(&v as *const Versioned as *const c_void, &mut blob, &mut err);
    assert!(hr >= 0, "hr={hr:#010x}");
    assert_eq!(flags_of(&unsafe { blob_bytes(blob) }), ALLOW_IA | XBOX_ROOT_FLAGS);
    unsafe {
        release(blob);
        release(err);
    }
}

#[test]
fn xbox_one_params_translate_to_scarlett_layout() {
    let x = crate::default_xone_params(0x0010_0001);
    assert_eq!(core::mem::size_of_val(&x), 0x28);
    assert_eq!((x[0], x[1], x[2], x[6], x[7]), (0x1000a, 0x0010_0001, u32::MAX, u32::MAX, u32::MAX));
    let mut src = [0u8; 0x28];
    for (i, w) in x.iter().enumerate() {
        src[i * 4..i * 4 + 4].copy_from_slice(&w.to_le_bytes());
    }
    src[0x20] = 0x5a;
    let s = crate::scarlett_params(&src);
    assert_eq!(core::mem::size_of_val(&s), 0x48);
    assert_eq!(s[0], 0x20004);
    assert_eq!(&s[1..10], &[0x0010_0001, u32::MAX, 0, 0, 0, u32::MAX, u32::MAX, 0x5a, 0]);
    assert!(s[10..].iter().all(|&w| w == 0));
}

#[test]
fn versioned_deserializer_restores_xbox_flags_at_every_version() {
    let v = Versioned { version: 2, desc: Desc { num_params: 0, params: core::ptr::null(), num_samplers: 0, samplers: core::ptr::null(), flags: ALLOW_IA | XBOX_ROOT_FLAGS } };
    let (mut blob, mut err): (*mut c_void, *mut c_void) = (core::ptr::null_mut(), core::ptr::null_mut());
    let hr = crate::D3D12SerializeVersionedRootSignature(&v as *const Versioned as *const c_void, &mut blob, &mut err);
    assert!(hr >= 0, "hr={hr:#010x}");
    let bytes = unsafe { blob_bytes(blob) };
    let iid: [u32; 4] = [0x7f91_ce67, 0x4bb7_090c, 0x8fed_8eb7, 0xa01d_e3f2];
    let mut de: *mut c_void = core::ptr::null_mut();
    let hr = crate::D3D12CreateVersionedRootSignatureDeserializer(bytes.as_ptr() as *const c_void, bytes.len(), iid.as_ptr() as *const c_void, &mut de);
    assert!(hr >= 0, "deserializer hr={hr:#010x}");
    let at_version: unsafe extern "system" fn(*mut c_void, u32, *mut *const Versioned) -> i32 = unsafe { core::mem::transmute(vslot(de, 3)) };
    let mut checked = 0;
    for ver in [1u32, 2, 3] {
        let mut d: *const Versioned = core::ptr::null();
        let hr = unsafe { at_version(de, ver, &mut d) };
        assert!(hr >= 0 && !d.is_null(), "version {ver} hr={hr:#010x}");
        assert_eq!(unsafe { (*d).version }, ver);
        assert_eq!(unsafe { (*d).desc.flags }, ALLOW_IA | XBOX_ROOT_FLAGS, "version {ver}");
        checked += 1;
    }
    let unconverted: unsafe extern "system" fn(*mut c_void) -> *const Versioned = unsafe { core::mem::transmute(vslot(de, 4)) };
    assert_eq!(unsafe { (*unconverted(de)).desc.flags }, ALLOW_IA | XBOX_ROOT_FLAGS);
    assert_eq!(checked, 3);
    unsafe {
        release(de);
        release(blob);
        release(err);
    }
}
