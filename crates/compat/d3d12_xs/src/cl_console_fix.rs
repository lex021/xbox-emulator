use core::ffi::c_void;
use core::sync::atomic::{AtomicUsize, Ordering};
use std::collections::HashMap;
use std::sync::Mutex;
use compat_common::record;

use crate::console_slots_dev2::{family, Family};
use crate::device_wrap::Guid;

pub const CL_SET_NAME: usize = 6;
pub const CL_COPY_MEMORY: usize = 60;
pub const CL_FLUSH_PIPELINE_X: usize = 67;
pub const CL_DEBUG_BREAK_SLOTS: [usize; 3] = [104, 105, 109];
pub const SC_BREAK_BATCH: usize = 104;
pub const SC_DEBUG_BREAK_SLOTS: [usize; 3] = [105, 106, 110];
pub const SC_DEPTH_BOUNDS: usize = 107;
pub const SC_SAMPLE_POSITIONS: usize = 108;
pub const SC_RESOLVE_REGION: usize = 109;
const ONE_DEPTH_BOUNDS: usize = 106;
const ONE_SAMPLE_POSITIONS: usize = 107;
const ONE_RESOLVE_REGION: usize = 108;

pub const FLUSH_INVALID_GRAPHICS: u32 = 0x1009_8382;
pub const FLUSH_ANY: u32 = 0x7fff_ffff;
pub const FLUSH_RANGED: u32 = 0xeec4_4041;
pub const FLUSH_RANGE_END: u64 = 0xff_ffff_ff00;

const S_OK: i32 = 0;
const ST_COMMON: u32 = 0;
const ST_COPY_DEST: u32 = 0x400;
const ST_COPY_SOURCE: u32 = 0x800;
const ST_GENERIC_READ: u32 = 0xac3;
const LIST_TYPE_COPY: u32 = 3;
const PC_SLOT_GET_DEVICE: usize = 7;
const PC_SLOT_COPY_BUFFER_REGION: usize = 15;
const PC_SLOT_RESOURCE_BARRIER: usize = 26;
const DEV_CREATE_COMMITTED: usize = 27;
const IID_DEVICE: Guid = Guid { d1: 0x1898_19f1, d2: 0x1db6, d3: 0x4b57, d4: [0xbe, 0x54, 0x18, 0x21, 0x33, 0x9b, 0x85, 0xf7] };
const IID_RESOURCE: Guid = Guid { d1: 0x6964_42be, d2: 0xa72e, d3: 0x4059, d4: [0xbc, 0x79, 0x5b, 0x5c, 0x98, 0x04, 0x0f, 0xad] };

static PREV_COPY: AtomicUsize = AtomicUsize::new(0);
static LOG: AtomicUsize = AtomicUsize::new(0);
static SCRATCH: Mutex<Option<HashMap<usize, (usize, u64)>>> = Mutex::new(None);
static RETIRED: Mutex<Vec<usize>> = Mutex::new(Vec::new());

pub fn scratch_cap(need: u64) -> u64 {
    need.max(1 << 16).next_power_of_two()
}

fn once(n: usize) -> bool {
    LOG.fetch_add(1, Ordering::Relaxed) < n
}

unsafe fn pc_fn(obj: *mut c_void, slot: usize) -> usize {
    unsafe { *(*(obj as *const *const usize)).add(slot) }
}

unsafe extern "system" fn set_name_x(_this: *mut c_void, _name: *const u16) -> i32 {
    S_OK
}

#[unsafe(naked)]
unsafe extern "system" fn debug_break_x() {
    core::arch::naked_asm!("xchg ax, ax", "int3", "ret")
}

pub fn flush_error(flags: u32, start: u64, size: u64, list_type: u32) -> Option<&'static str> {
    if list_type == LIST_TYPE_COPY {
        return Some("FlushPipelineX cannot be performed on a copy command list.");
    }
    if flags & FLUSH_INVALID_GRAPHICS != 0 {
        return Some("Invalid flush flags.");
    }
    if flags & FLUSH_ANY == 0 {
        return Some("Flush flags cannot be 0.");
    }
    if flags & FLUSH_RANGED != 0 && start.wrapping_add(size) > FLUSH_RANGE_END {
        return Some("BaseAddress + SizeInBytes is beyond the graphics address range.");
    }
    None
}

pub fn flush_records(list_type: u32) -> bool {
    list_type != LIST_TYPE_COPY
}

unsafe extern "system" fn flush_pipeline_x(this: *mut c_void, flags: u32, start: u64, size: u64) {
    let _ij = unsafe { crate::inject::scope(this, crate::inject::BARRIER) };
    let ty = unsafe { crate::fill::list_type(this) };
    if let Some(e) = flush_error(flags, start, size, ty) {
        if once(16) {
            record("d3d12_xs", "FlushPipelineX", &format!("flags={flags:#x} start={start:#x} size={size:#x}: {e} (xgs12_d validation)"));
        }
    }
    if !flush_records(ty) {
        return;
    }
    let b: [u64; 4] = [2, 0, 0, 0];
    let f: unsafe extern "system" fn(*mut c_void, u32, *const [u64; 4]) = unsafe { core::mem::transmute(pc_fn(this, PC_SLOT_RESOURCE_BARRIER)) };
    unsafe { f(this, 1, &b) };
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Step {
    Barrier { scratch: bool, before: u32, after: u32 },
    ToScratch,
    FromScratch,
}

pub fn within_plan(state: u32) -> Option<Vec<Step>> {
    if state == ST_GENERIC_READ {
        return None;
    }
    let mut v = Vec::new();
    if state != ST_COPY_SOURCE {
        v.push(Step::Barrier { scratch: false, before: state, after: ST_COPY_SOURCE });
    }
    v.push(Step::Barrier { scratch: true, before: ST_COMMON, after: ST_COPY_DEST });
    v.push(Step::ToScratch);
    v.push(Step::Barrier { scratch: true, before: ST_COPY_DEST, after: ST_COPY_SOURCE });
    v.push(Step::Barrier { scratch: false, before: ST_COPY_SOURCE, after: ST_COPY_DEST });
    v.push(Step::FromScratch);
    if state != ST_COPY_DEST {
        v.push(Step::Barrier { scratch: false, before: ST_COPY_DEST, after: state });
    }
    v.push(Step::Barrier { scratch: true, before: ST_COPY_SOURCE, after: ST_COMMON });
    Some(v)
}

unsafe fn device_of(list: *mut c_void) -> *mut c_void {
    let f: unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> i32 = unsafe { core::mem::transmute(pc_fn(list, PC_SLOT_GET_DEVICE)) };
    let mut dev: *mut c_void = core::ptr::null_mut();
    if unsafe { f(list, &IID_DEVICE, &mut dev) } < 0 || dev.is_null() {
        return core::ptr::null_mut();
    }
    let release: unsafe extern "system" fn(*mut c_void) -> u32 = unsafe { core::mem::transmute(pc_fn(dev, 2)) };
    unsafe { release(dev) };
    dev
}

unsafe fn scratch_for(list: *mut c_void, need: u64) -> Option<usize> {
    let key = crate::list_split::origin_of(list);
    let mut g = SCRATCH.lock().unwrap_or_else(|e| e.into_inner());
    let m = g.get_or_insert_with(HashMap::new);
    if let Some(&(res, cap)) = m.get(&key) {
        if cap >= need {
            return Some(res);
        }
    }
    let dev = unsafe { device_of(list) };
    if dev.is_null() {
        return None;
    }
    let cap = scratch_cap(need);
    let heap: [u32; 5] = [1, 0, 0, 1, 1];
    let mut desc = [0u8; 0x38];
    desc[0..4].copy_from_slice(&1u32.to_le_bytes());
    desc[0x10..0x18].copy_from_slice(&cap.to_le_bytes());
    desc[0x18..0x1c].copy_from_slice(&1u32.to_le_bytes());
    desc[0x1c..0x1e].copy_from_slice(&1u16.to_le_bytes());
    desc[0x1e..0x20].copy_from_slice(&1u16.to_le_bytes());
    desc[0x24..0x28].copy_from_slice(&1u32.to_le_bytes());
    desc[0x2c..0x30].copy_from_slice(&1u32.to_le_bytes());
    let create: unsafe extern "system" fn(*mut c_void, *const u32, u32, *const u8, u32, *const c_void, *const Guid, *mut *mut c_void) -> i32 =
        unsafe { core::mem::transmute(pc_fn(dev, DEV_CREATE_COMMITTED)) };
    let mut res: *mut c_void = core::ptr::null_mut();
    let hr = unsafe { create(dev, heap.as_ptr(), 0, desc.as_ptr(), ST_COMMON, core::ptr::null(), &IID_RESOURCE, &mut res) };
    if hr < 0 || res.is_null() {
        record("d3d12_xs", "CopyMemoryToMemoryX", &format!("scratch buffer of {cap} bytes FAILED hr={hr:#010x}"));
        return None;
    }
    if let Some(old) = m.insert(key, (res as usize, cap)) {
        RETIRED.lock().unwrap_or_else(|e| e.into_inner()).push(old.0);
    }
    Some(res as usize)
}

unsafe fn copy_within(list: *mut c_void, res: usize, dst_off: u64, src_off: u64, len: u64) -> Result<(), &'static str> {
    let _ij = unsafe { crate::inject::scope(list, crate::inject::COPY) };
    let state = crate::gpu_va::state(res);
    let Some(plan) = within_plan(state) else { return Err("the buffer is in an upload heap, which the GPU cannot write on PC") };
    let ty = unsafe { crate::fill::list_type(list) };
    if !crate::fill::state_legal_on(ty, state) {
        return Err("the buffer state cannot be transitioned on this list type");
    }
    let Some(scratch) = (unsafe { scratch_for(list, len) }) else { return Err("no scratch buffer") };
    let copy: unsafe extern "system" fn(*mut c_void, *mut c_void, u64, *mut c_void, u64, u64) = unsafe { core::mem::transmute(pc_fn(list, PC_SLOT_COPY_BUFFER_REGION)) };
    for s in plan {
        unsafe {
            match s {
                Step::Barrier { scratch: on_scratch, before, after } => crate::fill::barrier_pub(list, if on_scratch { scratch } else { res }, before, after),
                Step::ToScratch => copy(list, scratch as *mut c_void, 0, res as *mut c_void, src_off, len),
                Step::FromScratch => copy(list, res as *mut c_void, dst_off, scratch as *mut c_void, 0, len),
            }
        }
    }
    crate::placed::note_gpu_write(res as *mut c_void);
    Ok(())
}

unsafe extern "system" fn copy_memory_x(this: *mut c_void, dst: u64, src: u64, size: u64, flags: u32) {
    if size != 0 && size % 4 == 0 {
        if let (Some((de, doff)), Some((se, soff))) = (crate::gpu_va::lookup(dst), crate::gpu_va::lookup(src)) {
            if de.res == se.res && de.buffer {
                let len = size.min(de.len - doff).min(se.len - soff);
                if let Err(e) = unsafe { copy_within(this, de.res, doff, soff, len) } {
                    if once(16) {
                        record("d3d12_xs", "CopyMemoryToMemoryX", &format!("dst={dst:#x} src={src:#x} size={size:#x} in one buffer: {e}; not copied"));
                    }
                }
                return;
            }
        }
    }
    let p = PREV_COPY.load(Ordering::Relaxed);
    if p != 0 {
        let f: unsafe extern "system" fn(*mut c_void, u64, u64, u64, u32) = unsafe { core::mem::transmute(p) };
        unsafe { f(this, dst, src, size, flags) };
    }
}

pub const CL_GET_DEVICE: usize = 7;
const E_POINTER: i32 = 0x8000_4003u32 as i32;

unsafe extern "system" fn get_device_x(this: *mut c_void, riid: *const Guid, ppv: *mut *mut c_void) -> i32 {
    if ppv.is_null() || riid.is_null() {
        return E_POINTER;
    }
    let f: unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> i32 = unsafe { core::mem::transmute(pc_fn(this, PC_SLOT_GET_DEVICE)) };
    let mut dev: *mut c_void = core::ptr::null_mut();
    let hr = unsafe { f(this, &IID_DEVICE, &mut dev) };
    if hr < 0 || dev.is_null() {
        unsafe { *ppv = core::ptr::null_mut() };
        return hr;
    }
    let out = crate::device_wrap::wrapper_qi(dev, riid, ppv);
    let release: unsafe extern "system" fn(*mut c_void) -> u32 = unsafe { core::mem::transmute(pc_fn(dev, 2)) };
    unsafe { release(dev) };
    match out {
        Some(hr) => hr,
        None => {
            if once(16) {
                record("d3d12_xs", "GetDevice", &format!("list {this:p}: device {dev:p} has no Xbox wrapper; returning the host device"));
            }
            unsafe { f(this, riid, ppv) }
        }
    }
}

unsafe extern "system" fn break_batch_x(this: *mut c_void) {
    let n = BREAK_BATCHES.fetch_add(1, Ordering::Relaxed);
    if n == 0 {
        record("d3d12_xs", "BreakBatchX", &format!("list {this:p}: EVENT_WRITE BREAK_BATCH (xgs12_10 0x6c3c0) only closes the primitive binner batch; D3D12 PC has no binner batch state, so the draw stream is unchanged"));
    }
}

static BREAK_BATCHES: AtomicUsize = AtomicUsize::new(0);

pub fn break_batch_count() -> usize {
    BREAK_BATCHES.load(Ordering::Relaxed)
}

pub fn plan_for(fam: Family) -> Vec<(usize, usize)> {
    let mut v = vec![
        (CL_SET_NAME, set_name_x as *const () as usize),
        (CL_GET_DEVICE, get_device_x as *const () as usize),
        (CL_FLUSH_PIPELINE_X, flush_pipeline_x as *const () as usize),
    ];
    if fam == Family::XboxOne {
        for s in CL_DEBUG_BREAK_SLOTS {
            v.push((s, debug_break_x as *const () as usize));
        }
    }
    v
}

pub fn plan() -> Vec<(usize, usize)> {
    plan_for(current_family())
}

fn current_family() -> Family {
    family(compat_common::gdk::identity::device_type_value())
}

pub fn install_console_bodies_for(vt: &[AtomicUsize], fam: Family) -> usize {
    let mut n = 0;
    for (s, f) in plan_for(fam) {
        if s < vt.len() {
            vt[s].store(f, Ordering::Relaxed);
            n += 1;
        }
    }
    n
}

pub fn install_console_bodies(vt: &[AtomicUsize]) -> usize {
    install_console_bodies_for(vt, current_family())
}

pub fn scarlett_tail_plan(cur: &[usize]) -> Vec<(usize, usize)> {
    let dbg = debug_break_x as *const () as usize;
    let mut v = vec![(SC_BREAK_BATCH, break_batch_x as *const () as usize)];
    for s in SC_DEBUG_BREAK_SLOTS {
        v.push((s, dbg));
    }
    for (sc, one) in [(SC_DEPTH_BOUNDS, ONE_DEPTH_BOUNDS), (SC_SAMPLE_POSITIONS, ONE_SAMPLE_POSITIONS), (SC_RESOLVE_REGION, ONE_RESOLVE_REGION)] {
        if one < cur.len() {
            v.push((sc, cur[one]));
        }
    }
    v
}

pub fn install_family_tail_for(vt: &[AtomicUsize], fam: Family) -> usize {
    if fam != Family::Scarlett {
        return 0;
    }
    let cur: Vec<usize> = vt.iter().map(|a| a.load(Ordering::Relaxed)).collect();
    let mut n = 0;
    for (s, f) in scarlett_tail_plan(&cur) {
        if s < vt.len() {
            vt[s].store(f, Ordering::Relaxed);
            n += 1;
        }
    }
    n
}

pub fn install_family_tail(vt: &[AtomicUsize]) -> usize {
    let fam = current_family();
    let n = install_family_tail_for(vt, fam);
    record("d3d12_xs", "cl_console_fix", &format!("{fam:?} list tail layout: {n} slots of 104..110 moved to the release driver order"));
    n
}

pub fn install_copy_memory(vt: &[AtomicUsize]) -> usize {
    let mut n = 0;
    if CL_COPY_MEMORY < vt.len() {
        let p = vt[CL_COPY_MEMORY].load(Ordering::Relaxed);
        if p != copy_memory_x as *const () as usize {
            PREV_COPY.store(p, Ordering::Relaxed);
            vt[CL_COPY_MEMORY].store(copy_memory_x as *const () as usize, Ordering::Relaxed);
            n += 1;
        }
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_break_slots_run_the_release_body_bytes() {
        let mut p = debug_break_x as *const () as usize as *const u8;
        if unsafe { *p } == 0xe9 {
            let rel = unsafe { core::ptr::read_unaligned(p.add(1) as *const i32) };
            p = unsafe { p.add(5).offset(rel as isize) };
        }
        let b = unsafe { core::slice::from_raw_parts(p, 4) };
        assert_eq!(b, &[0x66, 0x90, 0xcc, 0xc3]);
    }

    #[test]
    fn set_name_returns_s_ok_and_keeps_nothing() {
        let name: Vec<u16> = "x\0".encode_utf16().collect();
        assert_eq!(unsafe { set_name_x(8 as *mut c_void, name.as_ptr()) }, S_OK);
    }

    #[test]
    fn flush_validation_matches_xgs12_d() {
        assert_eq!(flush_error(0x4, 0, 0, 0), None);
        assert!(flush_error(0, 0, 0, 0).is_some());
        assert!(flush_error(0x8000_0000, 0, 0, 0).is_some());
        assert!(flush_error(0x2, 0, 0, 0).is_some());
        assert!(flush_error(0x1000_0000, 0, 0, 0).is_some());
        assert_eq!(flush_error(0x1, 0, FLUSH_RANGE_END, 0), None);
        assert!(flush_error(0x1, 0x100, FLUSH_RANGE_END, 0).is_some());
        assert_eq!(flush_error(0x4, 0x100, FLUSH_RANGE_END, 2), None);
        assert!(flush_error(0x4, 0, 0, LIST_TYPE_COPY).is_some());
        assert!(flush_records(0) && flush_records(2) && !flush_records(LIST_TYPE_COPY));
    }

    #[test]
    fn copy_within_one_buffer_goes_through_scratch_and_restores_state() {
        assert_eq!(within_plan(ST_GENERIC_READ), None);
        let p = within_plan(0x8).unwrap();
        assert_eq!(p, vec![
            Step::Barrier { scratch: false, before: 0x8, after: ST_COPY_SOURCE },
            Step::Barrier { scratch: true, before: ST_COMMON, after: ST_COPY_DEST },
            Step::ToScratch,
            Step::Barrier { scratch: true, before: ST_COPY_DEST, after: ST_COPY_SOURCE },
            Step::Barrier { scratch: false, before: ST_COPY_SOURCE, after: ST_COPY_DEST },
            Step::FromScratch,
            Step::Barrier { scratch: false, before: ST_COPY_DEST, after: 0x8 },
            Step::Barrier { scratch: true, before: ST_COPY_SOURCE, after: ST_COMMON },
        ]);
        let d = within_plan(ST_COPY_DEST).unwrap();
        assert!(!d.contains(&Step::Barrier { scratch: false, before: ST_COPY_DEST, after: ST_COPY_DEST }));
        assert_eq!(d.iter().filter(|s| matches!(s, Step::Barrier { scratch: false, .. })).count(), 2);
        let s = within_plan(ST_COPY_SOURCE).unwrap();
        assert_eq!(s.iter().filter(|s| matches!(s, Step::Barrier { scratch: false, .. })).count(), 2);
    }

    #[test]
    fn install_fills_the_audited_slots_and_chains_copy_memory() {
        let vt: Vec<AtomicUsize> = (0..128).map(|i| AtomicUsize::new(500 + i)).collect();
        assert_eq!(install_console_bodies_for(&vt, Family::XboxOne), 6);
        assert_eq!(install_copy_memory(&vt), 1);
        assert_eq!(install_copy_memory(&vt), 0);
        assert_eq!(scratch_cap(1), 1 << 16);
        assert_eq!(scratch_cap((1 << 16) + 4), 1 << 17);
        assert_eq!(vt[6].load(Ordering::Relaxed), set_name_x as *const () as usize);
        assert_eq!(vt[67].load(Ordering::Relaxed), flush_pipeline_x as *const () as usize);
        for s in CL_DEBUG_BREAK_SLOTS {
            assert_eq!(vt[s].load(Ordering::Relaxed), debug_break_x as *const () as usize);
        }
        assert_eq!(vt[60].load(Ordering::Relaxed), copy_memory_x as *const () as usize);
        assert_eq!(PREV_COPY.load(Ordering::Relaxed), 560);
        assert_eq!(vt[106].load(Ordering::Relaxed), 606);
        assert_eq!(install_family_tail_for(&vt, Family::XboxOne), 0);
        assert_eq!(vt[109].load(Ordering::Relaxed), debug_break_x as *const () as usize);
    }

    static FAKE_DEV_REFS: AtomicUsize = AtomicUsize::new(1);

    unsafe extern "system" fn fake_dev_qi(_t: *mut c_void, _g: *const Guid, _p: *mut *mut c_void) -> i32 { 0x8000_4002u32 as i32 }
    unsafe extern "system" fn fake_dev_addref(_t: *mut c_void) -> u32 { (FAKE_DEV_REFS.fetch_add(1, Ordering::SeqCst) + 1) as u32 }
    unsafe extern "system" fn fake_dev_release(_t: *mut c_void) -> u32 { (FAKE_DEV_REFS.fetch_sub(1, Ordering::SeqCst) - 1) as u32 }

    struct FakeDev { vt: *const usize }
    struct FakeList { vt: *const usize, dev: *mut c_void }

    unsafe extern "system" fn fake_get_device(this: *mut c_void, _g: *const Guid, ppv: *mut *mut c_void) -> i32 {
        let l = unsafe { &*(this as *const FakeList) };
        FAKE_DEV_REFS.fetch_add(1, Ordering::SeqCst);
        unsafe { *ppv = l.dev };
        S_OK
    }

    #[test]
    fn get_device_hands_back_the_xbox_device_wrapper() {
        let dvt: [usize; 3] = [fake_dev_qi as usize, fake_dev_addref as usize, fake_dev_release as usize];
        let dev = Box::into_raw(Box::new(FakeDev { vt: dvt.as_ptr() })) as *mut c_void;
        let mut lvt = [0usize; 8];
        lvt[PC_SLOT_GET_DEVICE] = fake_get_device as usize;
        let list = Box::into_raw(Box::new(FakeList { vt: lvt.as_ptr(), dev })) as *mut c_void;
        let w = crate::device_wrap::wrap_device(dev);
        let mut out: *mut c_void = core::ptr::null_mut();
        assert_eq!(unsafe { get_device_x(list, &IID_DEVICE, &mut out) }, S_OK);
        assert_eq!(out, w);
        assert_eq!(FAKE_DEV_REFS.load(Ordering::SeqCst), 1);
        assert_eq!(unsafe { get_device_x(list, &IID_DEVICE, core::ptr::null_mut()) }, E_POINTER);
        let wvt = unsafe { *(w as *const *const usize) };
        let wrel: unsafe extern "system" fn(*mut c_void) -> u32 = unsafe { core::mem::transmute(*wvt.add(2)) };
        assert_eq!(unsafe { wrel(w) }, 1);
        assert_eq!(unsafe { wrel(w) }, 0);
        assert_eq!(FAKE_DEV_REFS.load(Ordering::SeqCst), 0);
        FAKE_DEV_REFS.store(1, Ordering::SeqCst);
        let mut raw: *mut c_void = core::ptr::null_mut();
        assert_eq!(unsafe { get_device_x(list, &IID_DEVICE, &mut raw) }, S_OK);
        assert_eq!(raw, dev);
        let _ = unsafe { (Box::from_raw(list as *mut FakeList), Box::from_raw(dev as *mut FakeDev)) };
    }

    #[test]
    fn scarlett_tail_follows_xgs12_10_release_vtable() {
        let vt: Vec<AtomicUsize> = (0..128).map(|i| AtomicUsize::new(500 + i)).collect();
        assert_eq!(install_console_bodies_for(&vt, Family::Scarlett), 3);
        assert_eq!(vt[7].load(Ordering::Relaxed), get_device_x as *const () as usize);
        for s in 104..=110 {
            assert_eq!(vt[s].load(Ordering::Relaxed), 500 + s);
        }
        assert_eq!(install_family_tail_for(&vt, Family::Scarlett), 7);
        let dbg = debug_break_x as *const () as usize;
        assert_eq!(vt[104].load(Ordering::Relaxed), break_batch_x as *const () as usize);
        assert_eq!(vt[105].load(Ordering::Relaxed), dbg);
        assert_eq!(vt[106].load(Ordering::Relaxed), dbg);
        assert_eq!(vt[107].load(Ordering::Relaxed), 606);
        assert_eq!(vt[108].load(Ordering::Relaxed), 607);
        assert_eq!(vt[109].load(Ordering::Relaxed), 608);
        assert_eq!(vt[110].load(Ordering::Relaxed), dbg);
        assert_eq!(vt[111].load(Ordering::Relaxed), 611);
        assert_eq!(vt[6].load(Ordering::Relaxed), set_name_x as *const () as usize);
        assert_eq!(vt[67].load(Ordering::Relaxed), flush_pipeline_x as *const () as usize);
        let before = break_batch_count();
        unsafe { break_batch_x(8 as *mut c_void) };
        assert!(break_batch_count() > before);
    }
}
