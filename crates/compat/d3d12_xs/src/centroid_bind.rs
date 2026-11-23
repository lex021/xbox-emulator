use core::ffi::c_void;
use core::sync::atomic::{AtomicUsize, Ordering};
use std::collections::HashMap;
use std::sync::Mutex;
use compat_common::record;
use tombox_xshader::centroid_abi::{root_constants, Constants, TB_CENTROID_MAX_POSITION_SAMPLES, TB_CENTROID_PC_DEFINE, TB_CENTROID_REGISTER, TB_CENTROID_ROOT_DWORDS, TB_CENTROID_SPACE};

use crate::device_wrap::Guid;

pub const RTFMT_PC_CENTROID: u8 = 0x80;
const PARAM_SIZE: usize = 32;
const PARAM_CONSTANTS: u32 = 1;
const VISIBILITY_PIXEL: u32 = 5;
const ROOT_FLAG_DENY_PIXEL: u32 = 0x20;
const ROOT_FLAG_LOCAL: u32 = 0x80;
const VERSION_1_1: u32 = 2;
const PC_SET_GRAPHICS_ROOT_CONSTANTS: usize = 36;
const XBOX_SET_GRAPHICS_ROOT_SIGNATURE: usize = 34;
const IID_DESERIALIZER: Guid = Guid { d1: 0x7f91_ce67, d2: 0x090c, d3: 0x4bb7, d4: [0xb7, 0x8e, 0xed, 0x8f, 0xf2, 0xe3, 0x1d, 0xa0] };

#[derive(Clone, Copy, Default)]
struct ListState {
    rs: usize,
    custom: Option<[u8; 16]>,
    pos: Option<([u8; 128], u32)>,
}

static ROOTS: Mutex<Option<HashMap<usize, u32>>> = Mutex::new(None);
static LISTS: Mutex<Option<HashMap<usize, ListState>>> = Mutex::new(None);
static PREV_GROOT: AtomicUsize = AtomicUsize::new(0);
static LOG: AtomicUsize = AtomicUsize::new(0);

fn once(n: usize) -> bool {
    LOG.fetch_add(1, Ordering::Relaxed) < n
}

unsafe fn vslot(obj: *mut c_void, i: usize) -> usize {
    unsafe { *(*(obj as *const *const usize)).add(i) }
}

fn u32_at(p: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(p[o..o + 4].try_into().unwrap())
}

pub fn param() -> [u8; PARAM_SIZE] {
    let mut p = [0u8; PARAM_SIZE];
    p[0..4].copy_from_slice(&PARAM_CONSTANTS.to_le_bytes());
    p[8..12].copy_from_slice(&TB_CENTROID_REGISTER.to_le_bytes());
    p[12..16].copy_from_slice(&TB_CENTROID_SPACE.to_le_bytes());
    p[16..20].copy_from_slice(&TB_CENTROID_ROOT_DWORDS.to_le_bytes());
    p[24..28].copy_from_slice(&VISIBILITY_PIXEL.to_le_bytes());
    p
}

pub fn append_plan(params: &[u8], flags: u32) -> Result<(Vec<u8>, u32), &'static str> {
    if flags & ROOT_FLAG_LOCAL != 0 {
        return Err("local root signature");
    }
    if flags & ROOT_FLAG_DENY_PIXEL != 0 {
        return Err("root signature denies pixel-shader root access");
    }
    if crate::gds::root_cost(params) + TB_CENTROID_ROOT_DWORDS > crate::gds::ROOT_DWORD_LIMIT {
        return Err("no room for the centroid root constants");
    }
    let idx = (params.len() / PARAM_SIZE) as u32;
    let mut v = params.to_vec();
    v.extend_from_slice(&param());
    Ok((v, idx))
}

unsafe fn release(o: *mut c_void) {
    if !o.is_null() {
        let r: unsafe extern "system" fn(*mut c_void) -> u32 = unsafe { core::mem::transmute(vslot(o, 2)) };
        unsafe { r(o) };
    }
}

pub unsafe fn patch_root_signature(blob: *const u8, len: usize) -> Result<(Vec<u8>, u32), String> {
    if blob.is_null() || len == 0 {
        return Err("no root signature blob".into());
    }
    let mk = crate::host_proc(b"D3D12CreateVersionedRootSignatureDeserializer\0");
    let ser = crate::host_proc(b"D3D12SerializeVersionedRootSignature\0");
    if mk == 0 || ser == 0 {
        return Err("versioned root signature (de)serializer missing".into());
    }
    let mk: unsafe extern "system" fn(*const u8, usize, *const Guid, *mut *mut c_void) -> i32 = unsafe { core::mem::transmute(mk) };
    let ser: unsafe extern "system" fn(*const u8, *mut *mut c_void, *mut *mut c_void) -> i32 = unsafe { core::mem::transmute(ser) };
    let mut d: *mut c_void = core::ptr::null_mut();
    if unsafe { mk(blob, len, &IID_DESERIALIZER, &mut d) } < 0 || d.is_null() {
        return Err("root signature blob does not deserialize".into());
    }
    let get: unsafe extern "system" fn(*mut c_void, u32, *mut *const u8) -> i32 = unsafe { core::mem::transmute(vslot(d, 3)) };
    let mut desc: *const u8 = core::ptr::null();
    let out = if unsafe { get(d, VERSION_1_1, &mut desc) } < 0 || desc.is_null() {
        Err("root signature has no 1.1 description".to_string())
    } else {
        let vd = unsafe { core::slice::from_raw_parts(desc, 48) }.to_vec();
        let n = u32_at(&vd, 8) as usize;
        let pp = u64::from_le_bytes(vd[16..24].try_into().unwrap()) as *const u8;
        let params = if n == 0 || pp.is_null() { Vec::new() } else { unsafe { core::slice::from_raw_parts(pp, n * PARAM_SIZE) }.to_vec() };
        match append_plan(&params, u32_at(&vd, 40)) {
            Err(e) => Err(e.to_string()),
            Ok((np, idx)) => {
                let mut nd = vd.clone();
                nd[8..12].copy_from_slice(&((n + 1) as u32).to_le_bytes());
                nd[16..24].copy_from_slice(&(np.as_ptr() as u64).to_le_bytes());
                let (mut b, mut e): (*mut c_void, *mut c_void) = (core::ptr::null_mut(), core::ptr::null_mut());
                let hr = unsafe { ser(nd.as_ptr(), &mut b, &mut e) };
                let r = if hr >= 0 && !b.is_null() {
                    let ptr: unsafe extern "system" fn(*mut c_void) -> *const u8 = unsafe { core::mem::transmute(vslot(b, 3)) };
                    let size: unsafe extern "system" fn(*mut c_void) -> usize = unsafe { core::mem::transmute(vslot(b, 4)) };
                    Ok((unsafe { core::slice::from_raw_parts(ptr(b), size(b)) }.to_vec(), idx))
                } else {
                    let msg = if e.is_null() {
                        String::new()
                    } else {
                        let ptr: unsafe extern "system" fn(*mut c_void) -> *const i8 = unsafe { core::mem::transmute(vslot(e, 3)) };
                        unsafe { std::ffi::CStr::from_ptr(ptr(e)) }.to_string_lossy().into_owned()
                    };
                    Err(format!("re-serializing with the centroid root constants failed hr={hr:#010x} {msg}"))
                };
                unsafe {
                    release(b);
                    release(e);
                }
                r
            }
        }
    };
    unsafe { release(d) };
    out
}

pub unsafe fn create(blob: *const u8, len: usize, make: &dyn Fn(*const u8, usize) -> i32, ppv: *mut *mut c_void) -> Option<i32> {
    let (b, idx) = match unsafe { patch_root_signature(blob, len) } {
        Ok(v) => v,
        Err(why) => {
            if once(16) {
                record("d3d12_xs", "centroid", &format!("root signature without centroid priority constants ({why}); its pixel shaders use the PC centroid"));
            }
            return None;
        }
    };
    let hr = make(b.as_ptr(), b.len());
    if hr < 0 || ppv.is_null() || unsafe { (*ppv).is_null() } {
        if once(16) {
            record("d3d12_xs", "centroid", &format!("root signature with the centroid priority constants rejected hr={hr:#010x}; created without them"));
        }
        return None;
    }
    note_root_signature(unsafe { *ppv }, idx);
    Some(hr)
}

pub fn note_root_signature(rs: *mut c_void, idx: u32) {
    ROOTS.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).insert(rs as usize, idx);
}

pub fn forget(rs: *mut c_void) {
    if let Some(m) = ROOTS.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        m.remove(&(rs as usize));
    }
}

pub fn root_index(rs: usize) -> Option<u32> {
    ROOTS.lock().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(|m| m.get(&rs).copied())
}

pub fn tag_rtfmt(rs: u64, mut rtfmt: [u8; 8]) -> [u8; 8] {
    if rs == 0 || root_index(rs as usize).is_none() {
        rtfmt[0] |= RTFMT_PC_CENTROID;
    }
    rtfmt
}

pub fn pc_define(rtfmt: &[u8; 8]) -> String {
    if rtfmt[0] & RTFMT_PC_CENTROID != 0 { format!("#define {TB_CENTROID_PC_DEFINE}\n") } else { String::new() }
}

fn constants_of(s: &ListState) -> Constants {
    root_constants(s.custom, s.pos.as_ref().map(|(p, n)| (p, *n)))
}

unsafe fn push(list: *mut c_void, s: &ListState) {
    let Some(idx) = root_index(s.rs) else { return };
    let c = constants_of(s);
    let f: unsafe extern "system" fn(*mut c_void, u32, u32, *const u32, u32) = unsafe { core::mem::transmute(crate::cmdlist_wrap::pc_method(list, PC_SET_GRAPHICS_ROOT_CONSTANTS)) };
    unsafe { f(list, idx, TB_CENTROID_ROOT_DWORDS, c.as_ptr(), 0) };
}

fn update(list: *mut c_void, f: impl FnOnce(&mut ListState)) -> ListState {
    let mut g = LISTS.lock().unwrap_or_else(|e| e.into_inner());
    let s = g.get_or_insert_with(HashMap::new).entry(list as usize).or_default();
    f(s);
    *s
}

pub unsafe fn on_msaa(list: *mut c_void, centroid: Option<[u8; 16]>, applied: Option<([u8; 128], u32)>) {
    if let Some((_, n)) = applied.filter(|&(_, n)| n > TB_CENTROID_MAX_POSITION_SAMPLES) {
        if once(16) {
            record("d3d12_xs", "centroid", &format!("{n}x programmed sample positions do not fit the centroid root constants; centroid evaluates at the standard {n}x sample positions"));
        }
    }
    let s = update(list, |s| {
        if centroid.is_some() {
            s.custom = centroid;
        }
        if applied.is_some() {
            s.pos = applied;
        }
    });
    unsafe { push(list, &s) };
}

pub fn on_close(list: *mut c_void) {
    if let Some(m) = LISTS.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        m.remove(&(list as usize));
    }
}

unsafe extern "system" fn h_groot(this: *mut c_void, rs: *mut c_void) {
    let f: unsafe extern "system" fn(*mut c_void, *mut c_void) = unsafe { core::mem::transmute(PREV_GROOT.load(Ordering::Relaxed)) };
    unsafe { f(this, rs) };
    let s = update(this, |s| s.rs = rs as usize);
    unsafe { push(this, &s) };
}

pub fn install(vt: &[AtomicUsize]) {
    PREV_GROOT.store(vt[XBOX_SET_GRAPHICS_ROOT_SIGNATURE].load(Ordering::Relaxed), Ordering::Relaxed);
    vt[XBOX_SET_GRAPHICS_ROOT_SIGNATURE].store(h_groot as usize, Ordering::Relaxed);
}

#[cfg(test)]
#[path = "centroid_bind_tests.rs"]
mod tests;
