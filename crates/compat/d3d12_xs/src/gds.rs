use core::ffi::c_void;
use core::sync::atomic::{AtomicUsize, Ordering};
use std::collections::HashMap;
use std::sync::Mutex;
use compat_common::record;
use tombox_xshader::gds_abi::{
    tb_gds_oa_byte_offset, TB_GDS_BYTES, TB_GDS_OA_COUNTERS, TB_GDS_OA_FIELD_CNTL, TB_GDS_OA_FIELD_COUNTER_DWORD, TB_GDS_OA_FIELD_STAGE,
    TB_GDS_OA_FIELD_WAVE_RELEASE, TB_GDS_REGISTER, TB_GDS_SPACE,
};

use crate::device_wrap::Guid;

pub const PARAM_TABLE: u32 = 0;
pub const PARAM_CONSTANTS: u32 = 1;
pub const PARAM_UAV: u32 = 4;
pub const ROOT_DWORD_LIMIT: u32 = 64;
pub const ROOT_FLAG_LOCAL: u32 = 0x80;
const PARAM_SIZE: usize = 32;
const VERSION_1_1: u32 = 2;
const STATE_COMMON: u32 = 0;
const STATE_UAV: u32 = 0x8;
const STATE_COPY_DEST: u32 = 0x400;
const STATE_COPY_SOURCE: u32 = 0x800;
const STATE_GENERIC_READ: u32 = 0xac3;
const LIST_TYPE_BUNDLE: u32 = 1;
const LIST_TYPE_COPY: u32 = 3;
const PC_WRITE_BUFFER_IMMEDIATE: usize = 66;
const WBI_MODE_DEFAULT: u32 = 0;
const PC_COPY_BUFFER_REGION: usize = 15;
const PC_SET_COMPUTE_ROOT_UAV: usize = 41;
const PC_SET_GRAPHICS_ROOT_UAV: usize = 42;
const XBOX_SET_COMPUTE_ROOT_SIGNATURE: usize = 33;
const XBOX_SET_GRAPHICS_ROOT_SIGNATURE: usize = 34;
const DEV_CREATE_COMMITTED: usize = 27;
const RES_GET_GPU_VA: usize = 11;
const IID_DESERIALIZER: Guid = Guid { d1: 0x7f91_ce67, d2: 0x090c, d3: 0x4bb7, d4: [0xb7, 0x8e, 0xed, 0x8f, 0xf2, 0xe3, 0x1d, 0xa0] };
const IID_RESOURCE: Guid = Guid { d1: 0x6964_42be, d2: 0xa72e, d3: 0x4059, d4: [0xbc, 0x79, 0x5b, 0x5c, 0x98, 0x04, 0x0f, 0xad] };
const IID_DEVICE: Guid = Guid { d1: 0x1898_19f1, d2: 0x1db6, d3: 0x4b57, d4: [0xbe, 0x54, 0x18, 0x21, 0x33, 0x9b, 0x85, 0xf7] };

static ROOTS: Mutex<Option<HashMap<usize, u32>>> = Mutex::new(None);
static STATES: Mutex<Option<HashMap<usize, u32>>> = Mutex::new(None);
static BUFFER: Mutex<Option<(usize, u64)>> = Mutex::new(None);
static PREV_CROOT: AtomicUsize = AtomicUsize::new(0);
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

pub fn root_cost(params: &[u8]) -> u32 {
    params.chunks(PARAM_SIZE).map(|p| match u32_at(p, 0) {
        PARAM_TABLE => 1,
        PARAM_CONSTANTS => u32_at(p, 16),
        _ => 2,
    }).sum()
}

pub fn gds_param() -> [u8; PARAM_SIZE] {
    let mut p = [0u8; PARAM_SIZE];
    p[0..4].copy_from_slice(&PARAM_UAV.to_le_bytes());
    p[8..12].copy_from_slice(&TB_GDS_REGISTER.to_le_bytes());
    p[12..16].copy_from_slice(&TB_GDS_SPACE.to_le_bytes());
    p
}

pub fn append_plan(params: &[u8], flags: u32) -> Option<(Vec<u8>, u32)> {
    if flags & ROOT_FLAG_LOCAL != 0 || root_cost(params) + 2 > ROOT_DWORD_LIMIT {
        return None;
    }
    let idx = (params.len() / PARAM_SIZE) as u32;
    let mut v = params.to_vec();
    v.extend_from_slice(&gds_param());
    Some((v, idx))
}

pub unsafe fn patch_root_signature(blob: *const u8, len: usize) -> Option<(Vec<u8>, u32)> {
    unsafe { patch_root_signature_inner(blob, len, false) }
}

pub unsafe fn patch_root_signature_with_gws(blob: *const u8, len: usize) -> Option<(Vec<u8>, u32)> {
    unsafe { patch_root_signature_inner(blob, len, true) }
}

unsafe fn patch_root_signature_inner(blob: *const u8, len: usize, phased: bool) -> Option<(Vec<u8>, u32)> {
    if blob.is_null() || len == 0 {
        return None;
    }
    let mk = crate::host_proc(b"D3D12CreateVersionedRootSignatureDeserializer\0");
    let ser = crate::host_proc(b"D3D12SerializeVersionedRootSignature\0");
    if mk == 0 || ser == 0 {
        return None;
    }
    let mk: unsafe extern "system" fn(*const u8, usize, *const Guid, *mut *mut c_void) -> i32 = unsafe { core::mem::transmute(mk) };
    let mut d: *mut c_void = core::ptr::null_mut();
    if unsafe { mk(blob, len, &IID_DESERIALIZER, &mut d) } < 0 || d.is_null() {
        return None;
    }
    let get: unsafe extern "system" fn(*mut c_void, u32, *mut *const u8) -> i32 = unsafe { core::mem::transmute(vslot(d, 3)) };
    let mut desc: *const u8 = core::ptr::null();
    let mut out = None;
    if unsafe { get(d, VERSION_1_1, &mut desc) } >= 0 && !desc.is_null() {
        let vd = unsafe { core::slice::from_raw_parts(desc, 48) }.to_vec();
        let n = u32_at(&vd, 8) as usize;
        let pp = u64::from_le_bytes(vd[16..24].try_into().unwrap()) as *const u8;
        let params = if n == 0 || pp.is_null() { Vec::new() } else { unsafe { core::slice::from_raw_parts(pp, n * PARAM_SIZE) }.to_vec() };
        match append_plan(&params, u32_at(&vd, 40)).filter(|(np, _)| !phased || root_cost(np) + 3 <= ROOT_DWORD_LIMIT) {
            Some((mut np, idx)) => {
                if phased {
                    let mut scratch = gds_param();
                    scratch[8..12].copy_from_slice(&1u32.to_le_bytes()); // u1 in the GDS space
                    np.extend_from_slice(&scratch);
                    let mut control = [0u8; PARAM_SIZE];
                    control[..4].copy_from_slice(&PARAM_CONSTANTS.to_le_bytes());
                    control[12..16].copy_from_slice(&TB_GDS_SPACE.to_le_bytes());
                    control[16..20].copy_from_slice(&1u32.to_le_bytes()); // b0, one DWORD
                    np.extend_from_slice(&control);
                }
                let mut nd = vd.clone();
                nd[8..12].copy_from_slice(&((np.len() / PARAM_SIZE) as u32).to_le_bytes());
                nd[16..24].copy_from_slice(&(np.as_ptr() as u64).to_le_bytes());
                let ser: unsafe extern "system" fn(*const u8, *mut *mut c_void, *mut *mut c_void) -> i32 = unsafe { core::mem::transmute(ser) };
                let (mut b, mut e): (*mut c_void, *mut c_void) = (core::ptr::null_mut(), core::ptr::null_mut());
                let hr = unsafe { ser(nd.as_ptr(), &mut b, &mut e) };
                if hr >= 0 && !b.is_null() {
                    let ptr: unsafe extern "system" fn(*mut c_void) -> *const u8 = unsafe { core::mem::transmute(vslot(b, 3)) };
                    let size: unsafe extern "system" fn(*mut c_void) -> usize = unsafe { core::mem::transmute(vslot(b, 4)) };
                    out = Some((unsafe { core::slice::from_raw_parts(ptr(b), size(b)) }.to_vec(), idx));
                } else if once(8) {
                    record("d3d12_xs", "gds", &format!("re-serializing a root signature with the GDS parameter FAILED hr={hr:#010x}"));
                }
                for o in [b, e] {
                    if !o.is_null() {
                        let r: unsafe extern "system" fn(*mut c_void) -> u32 = unsafe { core::mem::transmute(vslot(o, 2)) };
                        unsafe { r(o) };
                    }
                }
            }
            None => {
                if !phased && once(8) {
                    record("d3d12_xs", "gds", &format!("root signature uses {} DWORDs (or is local); no room for the 2-DWORD GDS root UAV", root_cost(&params)));
                }
            }
        }
    }
    let r: unsafe extern "system" fn(*mut c_void) -> u32 = unsafe { core::mem::transmute(vslot(d, 2)) };
    unsafe { r(d) };
    out
}

pub fn note_root_signature(rs: *mut c_void, idx: u32) {
    ROOTS.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).insert(rs as usize, idx);
}

pub(crate) fn root_index(rs: *mut c_void) -> Option<u32> {
    ROOTS.lock().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(|m| m.get(&(rs as usize)).copied())
}

unsafe fn buffer(list: *mut c_void) -> Option<(usize, u64)> {
    let mut g = BUFFER.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(b) = *g {
        return Some(b);
    }
    let b = unsafe { create_buffer(list, TB_GDS_BYTES, 1, 4, STATE_COMMON) }?;
    record("d3d12_xs", "gds", &format!("GDS emulated as a {TB_GDS_BYTES:#x}-byte device UAV buffer {:#x} va={:#x} (u{TB_GDS_REGISTER}, space{TB_GDS_SPACE})", b.0, b.1));
    *g = Some(b);
    Some(b)
}

pub(crate) unsafe fn create_buffer(list: *mut c_void, bytes: u64, heap_type: u32, flags: u32, state: u32) -> Option<(usize, u64)> {
    let get: unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> i32 = unsafe { core::mem::transmute(vslot(list, 7)) };
    let mut dev: *mut c_void = core::ptr::null_mut();
    if unsafe { get(list, &IID_DEVICE, &mut dev) } < 0 || dev.is_null() {
        return None;
    }
    let heap: [u32; 5] = [heap_type, 0, 0, 1, 1];
    let mut desc = [0u8; 56];
    desc[0..4].copy_from_slice(&1u32.to_le_bytes());
    desc[0x10..0x18].copy_from_slice(&bytes.to_le_bytes());
    desc[0x18..0x1c].copy_from_slice(&1u32.to_le_bytes());
    desc[0x1c..0x1e].copy_from_slice(&1u16.to_le_bytes());
    desc[0x1e..0x20].copy_from_slice(&1u16.to_le_bytes());
    desc[0x24..0x28].copy_from_slice(&1u32.to_le_bytes());
    desc[0x2c..0x30].copy_from_slice(&1u32.to_le_bytes());
    desc[0x30..0x34].copy_from_slice(&flags.to_le_bytes());
    let create: unsafe extern "system" fn(*mut c_void, *const u32, u32, *const u8, u32, *const c_void, *const Guid, *mut *mut c_void) -> i32 =
        unsafe { core::mem::transmute(vslot(dev, DEV_CREATE_COMMITTED)) };
    let mut res: *mut c_void = core::ptr::null_mut();
    let hr = unsafe { create(dev, heap.as_ptr(), 0, desc.as_ptr(), state, core::ptr::null(), &IID_RESOURCE, &mut res) };
    let rel: unsafe extern "system" fn(*mut c_void) -> u32 = unsafe { core::mem::transmute(vslot(dev, 2)) };
    unsafe { rel(dev) };
    if hr < 0 || res.is_null() {
        record("d3d12_xs", "gds", &format!("buffer ({bytes:#x} bytes) create FAILED hr={hr:#010x}"));
        return None;
    }
    let va: unsafe extern "system" fn(*mut c_void) -> u64 = unsafe { core::mem::transmute(vslot(res, RES_GET_GPU_VA)) };
    Some((res as usize, unsafe { va(res) }))
}

#[cfg(test)]
pub fn set_buffer_for_test(res: usize, va: u64) {
    *BUFFER.lock().unwrap_or_else(|e| e.into_inner()) = Some((res, va));
}

pub fn barriers_allowed(list_type: u32) -> bool {
    list_type != LIST_TYPE_BUNDLE
}

pub fn next_state(current: Option<u32>) -> u32 {
    current.unwrap_or(STATE_COMMON)
}

unsafe fn transition(list: *mut c_void, res: usize, to: u32) {
    let cur = next_state(STATES.lock().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(|m| m.get(&(list as usize)).copied()));
    if cur != to {
        unsafe { crate::fill::barrier_pub(list, res, cur, to) };
    }
    STATES.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).insert(list as usize, to);
}

pub unsafe fn on_close(list: *mut c_void) {
    let cur = STATES.lock().unwrap_or_else(|e| e.into_inner()).as_mut().and_then(|m| m.remove(&(list as usize)));
    if let (Some(c), Some((res, _))) = (cur, *BUFFER.lock().unwrap_or_else(|e| e.into_inner())) {
        if c != STATE_COMMON {
            unsafe { crate::fill::barrier_pub(list, res, c, STATE_COMMON) };
        }
    }
}

unsafe fn bind(list: *mut c_void, rs: *mut c_void, slot: usize) {
    let Some(idx) = root_index(rs) else { return };
    let Some((res, va)) = (unsafe { buffer(list) }) else { return };
    if barriers_allowed(unsafe { crate::fill::list_type(list) }) {
        unsafe { transition(list, res, STATE_UAV) };
    }
    let f: unsafe extern "system" fn(*mut c_void, u32, u64) = unsafe { core::mem::transmute(vslot(list, slot)) };
    unsafe { f(list, idx, va) };
}

unsafe extern "system" fn h_croot(this: *mut c_void, rs: *mut c_void) {
    let f: unsafe extern "system" fn(*mut c_void, *mut c_void) = unsafe { core::mem::transmute(PREV_CROOT.load(Ordering::Relaxed)) };
    unsafe { f(this, rs) };
    unsafe { bind(this, rs, PC_SET_COMPUTE_ROOT_UAV) };
}

unsafe extern "system" fn h_groot(this: *mut c_void, rs: *mut c_void) {
    let f: unsafe extern "system" fn(*mut c_void, *mut c_void) = unsafe { core::mem::transmute(PREV_GROOT.load(Ordering::Relaxed)) };
    unsafe { f(this, rs) };
    unsafe { bind(this, rs, PC_SET_GRAPHICS_ROOT_UAV) };
}

pub fn install(vt: &[AtomicUsize]) {
    PREV_CROOT.store(vt[XBOX_SET_COMPUTE_ROOT_SIGNATURE].load(Ordering::Relaxed), Ordering::Relaxed);
    PREV_GROOT.store(vt[XBOX_SET_GRAPHICS_ROOT_SIGNATURE].load(Ordering::Relaxed), Ordering::Relaxed);
    vt[XBOX_SET_COMPUTE_ROOT_SIGNATURE].store(h_croot as usize, Ordering::Relaxed);
    vt[XBOX_SET_GRAPHICS_ROOT_SIGNATURE].store(h_groot as usize, Ordering::Relaxed);
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CopyPlan {
    pub gds_state: u32,
    pub gds_off: u64,
    pub mem_before: Option<(u32, u32)>,
    pub bytes: u64,
}

pub fn gds_copy_plan(to_gds: bool, dword: u32, count: u32, mem_state: u32, mem_upload: bool, list_type: u32) -> Result<CopyPlan, &'static str> {
    let bytes = count as u64 * 4;
    if !barriers_allowed(list_type) {
        return Err("a bundle cannot copy buffers on PC (CopyBufferRegion is not a bundle command)");
    }
    if dword as u64 * 4 + bytes > TB_GDS_BYTES {
        return Err("GDS range beyond the emulated GDS");
    }
    let mem_before = if to_gds {
        (mem_state != STATE_COPY_SOURCE && mem_state != STATE_GENERIC_READ).then_some((mem_state, STATE_COPY_SOURCE))
    } else {
        if mem_upload {
            return Err("pCounterValues is an upload-heap buffer, which the GPU cannot write on PC");
        }
        (mem_state != STATE_COPY_DEST).then_some((mem_state, STATE_COPY_DEST))
    };
    if let Some((s, _)) = mem_before {
        if !crate::fill::state_legal_on(list_type, s) {
            return Err("pCounterValues buffer state cannot be transitioned on this list type");
        }
    }
    Ok(CopyPlan { gds_state: if to_gds { STATE_COPY_DEST } else { STATE_COPY_SOURCE }, gds_off: dword as u64 * 4, mem_before, bytes })
}

pub unsafe fn copy(list: *mut c_void, to_gds: bool, dword: u32, count: u32, va: u64, what: &str) {
    if count == 0 {
        return;
    }
    let Some((e, off)) = crate::gpu_va::lookup(va) else {
        if once(16) {
            record("d3d12_xs", "gds", &format!("{what}: pCounterValues {va:#x} is not in a tracked buffer"));
        }
        return;
    };
    if !e.buffer || off + count as u64 * 4 > e.len {
        if once(16) {
            record("d3d12_xs", "gds", &format!("{what}: pCounterValues {va:#x} + {} bytes is not inside one buffer", count * 4));
        }
        return;
    }
    let st = crate::gpu_va::state(e.res);
    let ty = unsafe { crate::fill::list_type(list) };
    let plan = match gds_copy_plan(to_gds, dword, count, st, st == STATE_GENERIC_READ, ty) {
        Ok(p) => p,
        Err(why) => {
            if once(16) {
                record("d3d12_xs", "gds", &format!("{what}: {why}"));
            }
            return;
        }
    };
    let Some((gres, _)) = (unsafe { buffer(list) }) else { return };
    let back = STATES.lock().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(|m| m.get(&(list as usize)).copied());
    unsafe {
        transition(list, gres, plan.gds_state);
        if let Some((a, b)) = plan.mem_before { crate::fill::barrier_pub(list, e.res, a, b); }
        let f: unsafe extern "system" fn(*mut c_void, usize, u64, usize, u64, u64) = core::mem::transmute(vslot(list, PC_COPY_BUFFER_REGION));
        if to_gds {
            f(list, gres, plan.gds_off, e.res, off, plan.bytes);
        } else {
            f(list, e.res, off, gres, plan.gds_off, plan.bytes);
        }
        if let Some((a, b)) = plan.mem_before { crate::fill::barrier_pub(list, e.res, b, a); }
        if let Some(s) = back.filter(|&s| s == STATE_UAV && ty != LIST_TYPE_COPY) {
            transition(list, gres, s);
        }
    }
    if !to_gds {
        crate::placed::note_gpu_write(e.res as *mut c_void);
    }
}

pub fn ordered_append_words(counter: u32, gds_dword: u32, cntl: u32, stage: u32, wave_release: u32) -> [(u32, u32); 4] {
    let mut w = [(0u32, 0u32); 4];
    w[TB_GDS_OA_FIELD_COUNTER_DWORD as usize] = (tb_gds_oa_byte_offset(counter, TB_GDS_OA_FIELD_COUNTER_DWORD), gds_dword);
    w[TB_GDS_OA_FIELD_CNTL as usize] = (tb_gds_oa_byte_offset(counter, TB_GDS_OA_FIELD_CNTL), cntl);
    w[TB_GDS_OA_FIELD_STAGE as usize] = (tb_gds_oa_byte_offset(counter, TB_GDS_OA_FIELD_STAGE), stage);
    w[TB_GDS_OA_FIELD_WAVE_RELEASE as usize] = (tb_gds_oa_byte_offset(counter, TB_GDS_OA_FIELD_WAVE_RELEASE), wave_release);
    w
}

#[repr(C)]
struct WbiParam {
    dest: u64,
    value: u32,
}

pub unsafe fn write_ordered_append(list: *mut c_void, counter: u32, gds_dword: u32, cntl: u32, stage: u32, wave_release: u32) -> Result<(), &'static str> {
    if counter >= TB_GDS_OA_COUNTERS {
        return Err("counter index beyond the ordered-append table");
    }
    if !barriers_allowed(unsafe { crate::fill::list_type(list) }) {
        return Err("a bundle cannot write the GDS buffer on PC (no resource barriers in bundles)");
    }
    let Some((gres, gva)) = (unsafe { buffer(list) }) else { return Err("GDS buffer unavailable") };
    let words = ordered_append_words(counter, gds_dword, cntl, stage, wave_release);
    let params: Vec<WbiParam> = words.iter().map(|&(o, v)| WbiParam { dest: gva + o as u64, value: v }).collect();
    let modes = [WBI_MODE_DEFAULT; 4];
    let back = STATES.lock().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(|m| m.get(&(list as usize)).copied());
    unsafe {
        transition(list, gres, STATE_COPY_DEST);
        let f: unsafe extern "system" fn(*mut c_void, u32, *const WbiParam, *const u32) = core::mem::transmute(vslot(list, PC_WRITE_BUFFER_IMMEDIATE));
        f(list, params.len() as u32, params.as_ptr(), modes.as_ptr());
        if let Some(s) = back.filter(|&s| s == STATE_UAV) {
            transition(list, gres, s);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn param(ty: u32, n: u32) -> [u8; 32] {
        let mut p = [0u8; 32];
        p[0..4].copy_from_slice(&ty.to_le_bytes());
        p[16..20].copy_from_slice(&n.to_le_bytes());
        p
    }

    #[test]
    fn gds_root_uav_is_appended_when_it_fits() {
        let mut ps = Vec::new();
        ps.extend_from_slice(&param(PARAM_TABLE, 3));
        ps.extend_from_slice(&param(PARAM_CONSTANTS, 8));
        ps.extend_from_slice(&param(2, 0));
        assert_eq!(root_cost(&ps), 1 + 8 + 2);
        let (v, idx) = append_plan(&ps, 0).unwrap();
        assert_eq!(idx, 3);
        assert_eq!(&v[96..128], &gds_param());
        assert_eq!(u32_at(&v, 96), PARAM_UAV);
        assert_eq!(u32_at(&v, 104), TB_GDS_REGISTER);
        assert_eq!(u32_at(&v, 108), TB_GDS_SPACE);
        assert_eq!(u32_at(&v, 120), 0);
        assert!(append_plan(&ps, ROOT_FLAG_LOCAL).is_none());
        let full = param(PARAM_CONSTANTS, 63);
        assert!(append_plan(&full, 0).is_none());
        assert!(append_plan(&param(PARAM_CONSTANTS, 62), 0).is_some());
        assert_eq!(append_plan(&[], 0).unwrap().1, 0);
    }

    #[test]
    fn gds_copies_plan_states_and_ranges() {
        let w = gds_copy_plan(true, 4, 2, 0x8, false, 0).unwrap();
        assert_eq!(w, CopyPlan { gds_state: STATE_COPY_DEST, gds_off: 16, mem_before: Some((0x8, STATE_COPY_SOURCE)), bytes: 8 });
        assert_eq!(gds_copy_plan(true, 0, 1, STATE_GENERIC_READ, true, 0).unwrap().mem_before, None);
        let r = gds_copy_plan(false, 0, 1, STATE_COPY_DEST, false, 0).unwrap();
        assert_eq!((r.gds_state, r.mem_before), (STATE_COPY_SOURCE, None));
        assert!(gds_copy_plan(false, 0, 1, STATE_GENERIC_READ, true, 0).is_err());
        assert!(gds_copy_plan(true, 0x3fff, 2, 0x800, false, 0).is_err());
        assert!(gds_copy_plan(true, 0, 1, STATE_UAV, false, LIST_TYPE_COPY).is_err());
        assert_eq!(next_state(None), STATE_COMMON);
        assert!(gds_copy_plan(true, 0, 1, 0x800, false, LIST_TYPE_BUNDLE).is_err());
        assert!(!barriers_allowed(LIST_TYPE_BUNDLE) && barriers_allowed(0) && barriers_allowed(2));
    }

    #[test]
    fn ordered_append_config_lands_in_the_shared_table() {
        let w = ordered_append_words(2, 0x40, 0x11, 1, 3);
        let base = tombox_xshader::gds_abi::TB_GDS_TITLE_DWORDS * 4 + 2 * 16;
        assert_eq!(w, [(base, 0x40), (base + 4, 0x11), (base + 8, 1), (base + 12, 3)]);
        assert!(w[3].0 as u64 + 4 <= TB_GDS_BYTES);
    }
}
