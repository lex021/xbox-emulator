use core::ffi::c_void;
use core::sync::atomic::{AtomicUsize, Ordering};
use std::collections::HashMap;
use std::sync::Mutex;
use compat_common::record;

use crate::device_wrap::Guid;
use crate::gpu_va;

const IID_DEVICE: Guid =
    Guid { d1: 0x1898_19F1, d2: 0x1DB6, d3: 0x4B57, d4: [0xBE, 0x54, 0x18, 0x21, 0x33, 0x9B, 0x85, 0xF7] };
const IID_RESOURCE: Guid =
    Guid { d1: 0x6964_42be, d2: 0xa72e, d3: 0x4059, d4: [0xbc, 0x79, 0x5b, 0x5c, 0x98, 0x04, 0x0f, 0xad] };
const STATE_COPY_DEST: u32 = 0x400;
const STATE_COPY_SOURCE: u32 = 0x800;
const STATE_GENERIC_READ: u32 = 0xac3;
const MAX_PATTERN: u64 = 256 << 20;

struct Pattern {
    res: usize,
    cap: u64,
}

static PATTERNS: Mutex<Option<HashMap<[u8; 16], Pattern>>> = Mutex::new(None);
static RETIRED: Mutex<Vec<usize>> = Mutex::new(Vec::new());

unsafe fn vt(obj: *mut c_void) -> *const usize {
    unsafe { *(obj as *const *const usize) }
}

pub fn plane_count(format: u32) -> u32 {
    match format {
        19 | 20 | 44 | 45 | 46 | 47 => 2,
        _ => 1,
    }
}

pub fn subresource_count(desc: &[u8]) -> u32 {
    let dim = u32::from_le_bytes(desc[0..4].try_into().unwrap());
    let depth_or_array = u16::from_le_bytes(desc[0x1c..0x1e].try_into().unwrap()) as u32;
    let mips = (u16::from_le_bytes(desc[0x1e..0x20].try_into().unwrap()) as u32).max(1);
    let format = u32::from_le_bytes(desc[0x20..0x24].try_into().unwrap());
    let layers = if dim == 4 { 1 } else { depth_or_array.max(1) };
    mips * layers * plane_count(format)
}

pub fn pattern_bytes(value: u32, len: usize) -> Vec<u8> {
    wide_pattern_bytes(&pattern16_32(value), len)
}

pub fn pattern16_32(value: u32) -> [u8; 16] {
    let w = value.to_le_bytes();
    core::array::from_fn(|i| w[i & 3])
}

pub fn pattern16_64(value: u64) -> [u8; 16] {
    let w = value.to_le_bytes();
    core::array::from_fn(|i| w[i & 7])
}

pub fn wide_pattern_bytes(pat: &[u8; 16], len: usize) -> Vec<u8> {
    (0..len).map(|i| pat[i & 15]).collect()
}

pub fn copy_states(tracked: u32, copy_list: bool) -> (u32, u32) {
    let before = if copy_list { tracked & (STATE_COPY_DEST | STATE_COPY_SOURCE) } else { tracked };
    (before, STATE_COPY_DEST)
}

fn transition(res: usize, before: u32, after: u32) -> [u8; 32] {
    let mut b = [0u8; 32];
    b[8..16].copy_from_slice(&(res as u64).to_le_bytes());
    b[16..20].copy_from_slice(&u32::MAX.to_le_bytes());
    b[20..24].copy_from_slice(&before.to_le_bytes());
    b[24..28].copy_from_slice(&after.to_le_bytes());
    b
}

unsafe fn device_of(list: *mut c_void) -> *mut c_void {
    let f: unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> i32 =
        unsafe { core::mem::transmute(*vt(list).add(7)) };
    let mut dev: *mut c_void = core::ptr::null_mut();
    if unsafe { f(list, &IID_DEVICE, &mut dev) } < 0 || dev.is_null() {
        return core::ptr::null_mut();
    }
    let release: unsafe extern "system" fn(*mut c_void) -> u32 = unsafe { core::mem::transmute(*vt(dev).add(2)) };
    unsafe { release(dev) };
    dev
}

unsafe fn create_pattern(dev: *mut c_void, pat: &[u8; 16], cap: u64) -> Option<usize> {
    let heap: [u32; 5] = [2, 0, 0, 1, 1];
    let mut desc = [0u8; 0x38];
    desc[0..4].copy_from_slice(&1u32.to_le_bytes());
    desc[0x10..0x18].copy_from_slice(&cap.to_le_bytes());
    desc[0x18..0x1c].copy_from_slice(&1u32.to_le_bytes());
    desc[0x1c..0x1e].copy_from_slice(&1u16.to_le_bytes());
    desc[0x1e..0x20].copy_from_slice(&1u16.to_le_bytes());
    desc[0x24..0x28].copy_from_slice(&1u32.to_le_bytes());
    desc[0x2c..0x30].copy_from_slice(&1u32.to_le_bytes());
    let create: unsafe extern "system" fn(*mut c_void, *const u32, u32, *const u8, u32, *const c_void, *const Guid, *mut *mut c_void) -> i32 =
        unsafe { core::mem::transmute(*vt(dev).add(27)) };
    let mut res: *mut c_void = core::ptr::null_mut();
    let hr = unsafe { create(dev, heap.as_ptr(), 0, desc.as_ptr(), STATE_GENERIC_READ, core::ptr::null(), &IID_RESOURCE, &mut res) };
    if hr < 0 || res.is_null() {
        record("d3d12_xs", "fill", &format!("pattern buffer {cap} bytes FAILED hr={hr:#010x}"));
        return None;
    }
    let map: unsafe extern "system" fn(*mut c_void, u32, *const c_void, *mut *mut u8) -> i32 =
        unsafe { core::mem::transmute(*vt(res).add(8)) };
    let mut p: *mut u8 = core::ptr::null_mut();
    if unsafe { map(res, 0, core::ptr::null(), &mut p) } < 0 || p.is_null() {
        return None;
    }
    let bytes = wide_pattern_bytes(pat, cap as usize);
    unsafe { core::ptr::copy_nonoverlapping(bytes.as_ptr(), p, bytes.len()) };
    let unmap: unsafe extern "system" fn(*mut c_void, u32, *const c_void) = unsafe { core::mem::transmute(*vt(res).add(9)) };
    unsafe { unmap(res, 0, core::ptr::null()) };
    Some(res as usize)
}

unsafe fn pattern(dev: *mut c_void, pat: &[u8; 16], need: u64) -> Option<usize> {
    let mut g = PATTERNS.lock().unwrap_or_else(|e| e.into_inner());
    let m = g.get_or_insert_with(HashMap::new);
    if let Some(p) = m.get(pat) {
        if p.cap >= need {
            return Some(p.res);
        }
    }
    let cap = need.max(1 << 16).next_power_of_two().min(MAX_PATTERN);
    if cap < need {
        return None;
    }
    let res = unsafe { create_pattern(dev, pat, cap)? };
    if let Some(old) = m.insert(*pat, Pattern { res, cap }) {
        RETIRED.lock().unwrap_or_else(|e| e.into_inner()).push(old.res);
    }
    Some(res)
}

unsafe fn barrier(list: *mut c_void, b: &[u8; 32]) {
    let _ij = unsafe { crate::inject::scope(list, crate::inject::BARRIER) };
    let f: unsafe extern "system" fn(*mut c_void, u32, *const u8) = unsafe { core::mem::transmute(*vt(list).add(26)) };
    unsafe { f(list, 1, b.as_ptr()) };
}

pub const LIST_TYPE_DIRECT: u32 = 0;
pub const LIST_TYPE_COMPUTE: u32 = 2;
pub const LIST_TYPE_COPY: u32 = 3;
const COMPUTE_LEGAL_STATES: u32 = 0x8 | 0x40 | 0x200 | 0x400 | 0x800 | 0x40_0000;
const COPY_LEGAL_STATES: u32 = 0x400 | 0x800;

pub fn state_legal_on(list_type: u32, state: u32) -> bool {
    match list_type {
        LIST_TYPE_COMPUTE => state & !COMPUTE_LEGAL_STATES == 0,
        LIST_TYPE_COPY => state & !COPY_LEGAL_STATES == 0,
        _ => true,
    }
}

pub unsafe fn list_type(list: *mut c_void) -> u32 {
    let f: unsafe extern "system" fn(*mut c_void) -> u32 = unsafe { core::mem::transmute(*vt(list).add(8)) };
    unsafe { f(list) }
}

pub unsafe fn barrier_pub(list: *mut c_void, res: usize, before: u32, after: u32) {
    unsafe { barrier(list, &transition(res, before, after)) };
}

fn once(c: &AtomicUsize, n: usize) -> bool {
    c.fetch_add(1, Ordering::Relaxed) < n
}

pub unsafe fn fill(list: *mut c_void, copy_list: bool, va: u64, size: u32, value: u32, flags: u32) {
    unsafe { fill_pattern(list, copy_list, va, size as u64, &pattern16_32(value), flags) }
}

pub unsafe fn fill_pattern(list: *mut c_void, copy_list: bool, va: u64, size: u64, pat: &[u8; 16], flags: u32) {
    let _ij = unsafe { crate::inject::scope(list, crate::inject::COPY) };
    let value = u32::from_le_bytes([pat[0], pat[1], pat[2], pat[3]]);
    static LOG: AtomicUsize = AtomicUsize::new(0);
    static MISS: AtomicUsize = AtomicUsize::new(0);
    if size == 0 {
        return;
    }
    let Some((e, off)) = gpu_va::lookup(va) else {
        if once(&MISS, 8) {
            record("d3d12_xs", "fill", &format!("va={va:#x} size={size:#x} value={value:#x} flags={flags:#x}: address not owned by any tracked resource"));
        }
        return;
    };
    let dev = unsafe { device_of(list) };
    if dev.is_null() {
        return;
    }
    let res = e.res as *mut c_void;
    let tracked = gpu_va::state(e.res);
    if e.buffer && tracked == STATE_GENERIC_READ {
        if once(&MISS, 8) {
            record("d3d12_xs", "fill", &format!("va={va:#x} targets an upload-heap buffer, which cannot be a GPU copy destination on PC"));
        }
        return;
    }
    let (before, during) = copy_states(tracked, copy_list);
    let need_barrier = before != during;
    let ty = unsafe { list_type(list) };
    if need_barrier && !state_legal_on(ty, before) {
        if once(&MISS, 8) {
            record("d3d12_xs", "fill", &format!("va={va:#x}: resource is in state {before:#x}, which a type-{ty} list cannot transition; the title must move it first, fill skipped"));
        }
        return;
    }
    if e.buffer {
        let len = size.min(e.len - off);
        let Some(src) = (unsafe { pattern(dev, pat, len) }) else { return };
        if need_barrier { unsafe { barrier(list, &transition(e.res, before, during)) }; }
        let f: unsafe extern "system" fn(*mut c_void, *mut c_void, u64, *mut c_void, u64, u64) =
            unsafe { core::mem::transmute(*vt(list).add(15)) };
        unsafe { f(list, res, off, src as *mut c_void, 0, len) };
        if need_barrier { unsafe { barrier(list, &transition(e.res, during, before)) }; }
    } else {
        if off != 0 {
            if once(&MISS, 8) {
                record("d3d12_xs", "fill", &format!("va={va:#x} starts {off:#x} bytes into a texture; console tiling has no PC byte mapping, fill skipped"));
            }
            return;
        }
        let mut desc = [0u8; 0x38];
        let get_desc: unsafe extern "system" fn(*mut c_void, *mut u8) -> *mut u8 = unsafe { core::mem::transmute(*vt(res).add(10)) };
        unsafe { get_desc(res, desc.as_mut_ptr()) };
        if u32::from_le_bytes(desc[0x24..0x28].try_into().unwrap()) > 1 {
            if once(&MISS, 8) {
                record("d3d12_xs", "fill", "multisampled texture: PC has no copy into MSAA surfaces; fill skipped");
            }
            return;
        }
        let subs = subresource_count(&desc);
        let footprints: unsafe extern "system" fn(*mut c_void, *const u8, u32, u32, u64, *mut u8, *mut u32, *mut u64, *mut u64) =
            unsafe { core::mem::transmute(*vt(dev).add(38)) };
        let mut layouts = vec![[0u8; 32]; subs as usize];
        let mut max_total = 0u64;
        for i in 0..subs {
            let mut total = 0u64;
            unsafe { footprints(dev, desc.as_ptr(), i, 1, 0, layouts[i as usize].as_mut_ptr(), core::ptr::null_mut(), core::ptr::null_mut(), &mut total) };
            max_total = max_total.max(total);
        }
        let Some(src) = (unsafe { pattern(dev, pat, max_total) }) else { return };
        if need_barrier { unsafe { barrier(list, &transition(e.res, before, during)) }; }
        let copy: unsafe extern "system" fn(*mut c_void, *const u8, u32, u32, u32, *const u8, *const c_void) =
            unsafe { core::mem::transmute(*vt(list).add(16)) };
        for i in 0..subs {
            let mut dst = [0u8; 48];
            dst[0..8].copy_from_slice(&(e.res as u64).to_le_bytes());
            dst[16..20].copy_from_slice(&i.to_le_bytes());
            let mut s = [0u8; 48];
            s[0..8].copy_from_slice(&(src as u64).to_le_bytes());
            s[8..12].copy_from_slice(&1u32.to_le_bytes());
            s[16..48].copy_from_slice(&layouts[i as usize]);
            s[16..24].copy_from_slice(&0u64.to_le_bytes());
            unsafe { copy(list, dst.as_ptr(), 0, 0, 0, s.as_ptr(), core::ptr::null()) };
        }
        if need_barrier { unsafe { barrier(list, &transition(e.res, during, before)) }; }
    }
    crate::placed::note_gpu_write(res);
    if once(&LOG, 6) {
        record("d3d12_xs", "fill", &format!(
            "va={va:#x} size={size:#x} value={value:#x} flags={flags:#x} -> {} res={res:p} off={off:#x} copyList={copy_list}",
            if e.buffer { "buffer" } else { "texture" }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn barrier_states_respect_list_type() {
        assert!(state_legal_on(LIST_TYPE_DIRECT, 0x80));
        assert!(!state_legal_on(LIST_TYPE_COMPUTE, 0x80));
        assert!(!state_legal_on(LIST_TYPE_COMPUTE, 0xac3));
        assert!(state_legal_on(LIST_TYPE_COMPUTE, 0x48));
        assert!(state_legal_on(LIST_TYPE_COMPUTE, 0));
        assert!(state_legal_on(LIST_TYPE_COPY, 0x400));
        assert!(!state_legal_on(LIST_TYPE_COPY, 0x8));
    }

    #[test]
    fn wide_patterns_repeat_their_full_width() {
        let p = pattern16_64(0x1122334455667788);
        assert_eq!(wide_pattern_bytes(&p, 10), vec![0x88, 0x77, 0x66, 0x55, 0x44, 0x33, 0x22, 0x11, 0x88, 0x77]);
        let mut q = [0u8; 16];
        for i in 0..16 { q[i] = i as u8; }
        assert_eq!(wide_pattern_bytes(&q, 18)[15..], [15, 0, 1]);
        assert_eq!(pattern16_32(0xaabbccdd), pattern16_64(0xaabbccdd_aabbccdd));
    }

    #[test]
    fn pattern_repeats_little_endian_word() {
        assert_eq!(pattern_bytes(0x11223344, 6), vec![0x44, 0x33, 0x22, 0x11, 0x44, 0x33]);
        assert!(pattern_bytes(0, 16).iter().all(|&b| b == 0));
    }

    #[test]
    fn subresources_include_planes_and_mips() {
        let mut d = [0u8; 0x38];
        d[0..4].copy_from_slice(&3u32.to_le_bytes());
        d[0x1c..0x1e].copy_from_slice(&6u16.to_le_bytes());
        d[0x1e..0x20].copy_from_slice(&4u16.to_le_bytes());
        d[0x20..0x24].copy_from_slice(&28u32.to_le_bytes());
        assert_eq!(subresource_count(&d), 24);
        d[0x20..0x24].copy_from_slice(&45u32.to_le_bytes());
        assert_eq!(subresource_count(&d), 48);
        d[0..4].copy_from_slice(&4u32.to_le_bytes());
        assert_eq!(subresource_count(&d), 8);
    }

    #[test]
    fn copy_list_states_are_copy_legal() {
        assert_eq!(copy_states(0x880, true), (0x800, 0x400));
        assert_eq!(copy_states(0x80, true), (0, 0x400));
        assert_eq!(copy_states(0x4, false), (0x4, 0x400));
        assert_eq!(copy_states(0x400, false), (0x400, 0x400));
    }
}
