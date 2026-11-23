use core::ffi::c_void;
use core::sync::atomic::{AtomicUsize, Ordering};
use std::collections::HashMap;
use std::sync::Mutex;
use compat_common::record;

pub const PREDICATE_BYTES: u32 = 8;
pub const PREDICATION_OP_EQUAL_ZERO: u32 = 0;
const PC_SLOT_SET_PREDICATION: usize = 55;
const LIST_TYPE_DIRECT: u32 = 0;
const LIST_TYPE_BUNDLE: u32 = 1;

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct PredicationBufferDesc {
    pub va: u64,
    pub offset: u32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cursor {
    pub va: u64,
    pub offset: u32,
}

impl Cursor {
    pub fn address(self) -> u64 {
        self.va.wrapping_add(self.offset as u64)
    }

    pub fn advanced(self) -> Cursor {
        Cursor { va: self.va, offset: self.offset.wrapping_add(PREDICATE_BYTES) }
    }
}

pub fn set_error(va: u64) -> Option<&'static str> {
    if va % 8 != 0 {
        return Some("BufferLocation must be a multiple of 8");
    }
    if va == 0 {
        return Some("BufferLocation cannot be null");
    }
    None
}

pub fn list_error(list_type: u32, what: &str) -> Option<String> {
    match list_type {
        LIST_TYPE_DIRECT => None,
        LIST_TYPE_BUNDLE => Some(format!("{what}: PC SetPredication cannot be recorded into a bundle")),
        2 => Some(format!("{what} cannot be performed on a compute command list (console validation)")),
        _ => Some(format!("{what} cannot be performed on a copy command list (console validation)")),
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Target {
    Buffer { res: usize, offset: u64 },
    Untracked,
    Misaligned,
}

pub fn resolve(addr: u64) -> Target {
    if addr % PREDICATE_BYTES as u64 != 0 {
        return Target::Misaligned;
    }
    match crate::gpu_va::lookup(addr) {
        Some((e, off)) if e.buffer && off + PREDICATE_BYTES as u64 <= e.len => Target::Buffer { res: e.res, offset: off },
        _ => Target::Untracked,
    }
}

static CURSORS: Mutex<Option<HashMap<usize, Cursor>>> = Mutex::new(None);
static LOG: AtomicUsize = AtomicUsize::new(0);

fn once() -> bool {
    LOG.fetch_add(1, Ordering::Relaxed) < 16
}

fn store(list: usize, c: Option<Cursor>) {
    let mut g = CURSORS.lock().unwrap_or_else(|e| e.into_inner());
    let m = g.get_or_insert_with(HashMap::new);
    match c {
        Some(c) => { m.insert(list, c); }
        None => { m.remove(&list); }
    }
}

pub fn forget(list: usize) {
    store(list, None);
}

pub fn cursor(list: usize) -> Option<Cursor> {
    CURSORS.lock().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(|m| m.get(&list).copied())
}

unsafe fn pc_set_predication(list: *mut c_void, res: usize, offset: u64, op: u32) {
    crate::cl2_state::note_predication(list, res, offset, op);
    let f: unsafe extern "system" fn(*mut c_void, *mut c_void, u64, u32) = unsafe { core::mem::transmute(*(*(list as *const *const usize)).add(PC_SLOT_SET_PREDICATION)) };
    unsafe { f(list, res as *mut c_void, offset, op) };
}

static FIRST_SET: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);
static FIRST_ADVANCE: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

unsafe fn apply(list: *mut c_void, c: Cursor, what: &str) {
    let addr = c.address();
    let first = if what == "AdvancePredicationX" { &FIRST_ADVANCE } else { &FIRST_SET };
    match resolve(addr) {
        Target::Buffer { res, offset } => {
            unsafe { pc_set_predication(list, res, offset, PREDICATION_OP_EQUAL_ZERO) };
            if !first.swap(true, Ordering::Relaxed) {
                record("d3d12_xs", "cl_fw", &format!("first call {what} on list {list:p}: predicate {addr:#x} (buffer {:#x} + offset {:#x}) -> PC SetPredication(res {res:#x}, offset {offset:#x}, EQUAL_ZERO)", c.va, c.offset));
            }
        }
        t => {
            unsafe { pc_set_predication(list, 0, 0, 0) };
            if once() {
                let why = if t == Target::Misaligned { "is not 8-byte aligned, so PC SetPredication cannot read it" } else { "is not inside a tracked buffer" };
                record("d3d12_xs", "cl_fw", &format!("{what}: predicate {addr:#x} (buffer {:#x} + offset {:#x}) {why}; predication disabled", c.va, c.offset));
            }
        }
    }
}

pub unsafe fn set_predication_buffer(list: *mut c_void, desc: *const PredicationBufferDesc) {
    const W: &str = "SetPredicationBufferX";
    if let Some(e) = list_error(unsafe { crate::fill::list_type(list) }, W) {
        if once() {
            record("d3d12_xs", "cl_fw", &format!("{e}; not recorded"));
        }
        return;
    }
    let key = crate::list_split::origin_of(list);
    if desc.is_null() {
        store(key, None);
        unsafe { pc_set_predication(list, 0, 0, 0) };
        if !FIRST_SET.swap(true, Ordering::Relaxed) {
            record("d3d12_xs", "cl_fw", &format!("first call {W}(NULL) on list {list:p}: predication cleared"));
        }
        return;
    }
    let d = unsafe { *desc };
    if let Some(e) = set_error(d.va) {
        if once() {
            record("d3d12_xs", "cl_fw", &format!("{W}: {e} (console validation, BufferLocation {:#x}); not recorded", d.va));
        }
        return;
    }
    let c = Cursor { va: d.va, offset: d.offset };
    store(key, Some(c));
    unsafe { apply(list, c, W) };
}

pub unsafe extern "system" fn advance_predication(list: *mut c_void) {
    const W: &str = "AdvancePredicationX";
    if let Some(e) = list_error(unsafe { crate::fill::list_type(list) }, W) {
        if once() {
            record("d3d12_xs", "cl_fw", &format!("{e}; not recorded"));
        }
        return;
    }
    let key = crate::list_split::origin_of(list);
    let Some(c) = cursor(key) else {
        if once() {
            record("d3d12_xs", "cl_fw", &format!("{W}: no SetPredicationBufferX is active on list {list:p}; there is no predicate position to move, so predication is left as it is"));
        }
        return;
    };
    let n = c.advanced();
    store(key, Some(n));
    unsafe { apply(list, n, W) };
}

pub const CL_CONSOLE_NULL_SLOTS: [usize; 1] = [66];

pub fn install_null_slots(xbox_vt: &[AtomicUsize]) {
    for s in CL_CONSOLE_NULL_SLOTS {
        if s < xbox_vt.len() {
            xbox_vt[s].store(0, Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn console_null_slots_stay_null_like_the_umd_vtable() {
        let vt: Vec<AtomicUsize> = (0..128).map(|i| AtomicUsize::new(1000 + i)).collect();
        install_null_slots(&vt);
        assert_eq!(vt[66].load(Ordering::Relaxed), 0);
        assert_eq!(vt[65].load(Ordering::Relaxed), 1065);
        assert_eq!(vt[67].load(Ordering::Relaxed), 1067);
    }

    #[repr(C)]
    struct FakeList {
        vt: *const usize,
        ty: u32,
    }

    static CALLS: Mutex<Vec<(usize, usize, u64, u32)>> = Mutex::new(Vec::new());

    unsafe extern "system" fn fake_get_type(this: *mut c_void) -> u32 {
        unsafe { (*(this as *const FakeList)).ty }
    }

    unsafe extern "system" fn fake_set_predication(this: *mut c_void, res: *mut c_void, off: u64, op: u32) {
        CALLS.lock().unwrap().push((this as usize, res as usize, off, op));
    }

    fn fake_vt() -> Vec<usize> {
        let mut v = vec![0usize; 64];
        v[8] = fake_get_type as usize;
        v[PC_SLOT_SET_PREDICATION] = fake_set_predication as usize;
        v
    }

    fn calls_for(l: usize) -> Vec<(usize, u64, u32)> {
        CALLS.lock().unwrap().iter().filter(|c| c.0 == l).map(|c| (c.1, c.2, c.3)).collect()
    }

    #[test]
    fn cursor_math_follows_the_firmware_record() {
        assert_eq!(core::mem::size_of::<PredicationBufferDesc>(), 16);
        let c = Cursor { va: 0x1000, offset: 0x10 };
        assert_eq!(c.address(), 0x1010);
        assert_eq!(c.advanced(), Cursor { va: 0x1000, offset: 0x18 });
        assert_eq!(c.advanced().advanced().address(), 0x1020);
        assert_eq!(set_error(0x1000), None);
        assert!(set_error(0).is_some());
        assert!(set_error(0x1004).is_some());
        assert_eq!(list_error(0, "x"), None);
        assert!(list_error(1, "x").is_some());
        assert!(list_error(2, "x").is_some());
        assert!(list_error(3, "x").is_some());
    }

    #[test]
    fn advance_moves_the_predicate_one_qword_per_call() {
        crate::gpu_va::insert(crate::gpu_va::Entry { start: 0x7a_0000_0000, len: 0x100, res: 0xabc0, buffer: true });
        let vt: &Vec<usize> = Box::leak(Box::new(fake_vt()));
        let l: &FakeList = Box::leak(Box::new(FakeList { vt: vt.as_ptr(), ty: LIST_TYPE_DIRECT }));
        let p = l as *const FakeList as *mut c_void;
        let d = PredicationBufferDesc { va: 0x7a_0000_0040, offset: 8 };
        unsafe {
            set_predication_buffer(p, &d);
            advance_predication(p);
            advance_predication(p);
        }
        assert_eq!(calls_for(p as usize), vec![(0xabc0, 0x48, 0), (0xabc0, 0x50, 0), (0xabc0, 0x58, 0)]);
        assert_eq!(cursor(p as usize), Some(Cursor { va: 0x7a_0000_0040, offset: 0x18 }));
        unsafe {
            set_predication_buffer(p, core::ptr::null());
            advance_predication(p);
        }
        assert_eq!(calls_for(p as usize).len(), 4);
        assert_eq!(calls_for(p as usize)[3], (0, 0, 0));
        assert_eq!(cursor(p as usize), None);
    }

    #[test]
    fn advance_past_tracked_memory_disables_predication() {
        crate::gpu_va::insert(crate::gpu_va::Entry { start: 0x7b_0000_0000, len: 0x10, res: 0xdef0, buffer: true });
        let vt: &Vec<usize> = Box::leak(Box::new(fake_vt()));
        let l: &FakeList = Box::leak(Box::new(FakeList { vt: vt.as_ptr(), ty: LIST_TYPE_DIRECT }));
        let p = l as *const FakeList as *mut c_void;
        let d = PredicationBufferDesc { va: 0x7b_0000_0000, offset: 8 };
        unsafe {
            set_predication_buffer(p, &d);
            advance_predication(p);
        }
        assert_eq!(calls_for(p as usize), vec![(0xdef0, 8, 0), (0, 0, 0)]);
    }

    #[test]
    fn rejected_lists_make_no_pc_call() {
        crate::gpu_va::insert(crate::gpu_va::Entry { start: 0x7c_0000_0000, len: 0x100, res: 0x1230, buffer: true });
        let vt: &Vec<usize> = Box::leak(Box::new(fake_vt()));
        for ty in [1u32, 2, 3] {
            let l: &FakeList = Box::leak(Box::new(FakeList { vt: vt.as_ptr(), ty }));
            let p = l as *const FakeList as *mut c_void;
            let d = PredicationBufferDesc { va: 0x7c_0000_0000, offset: 0 };
            unsafe {
                set_predication_buffer(p, &d);
                advance_predication(p);
            }
            assert!(calls_for(p as usize).is_empty());
            assert_eq!(cursor(p as usize), None);
        }
        let l: &FakeList = Box::leak(Box::new(FakeList { vt: vt.as_ptr(), ty: 0 }));
        let p = l as *const FakeList as *mut c_void;
        unsafe {
            advance_predication(p);
            set_predication_buffer(p, &PredicationBufferDesc { va: 0x7c_0000_0004, offset: 0 });
        }
        assert!(calls_for(p as usize).is_empty());
    }

    #[test]
    fn forget_drops_the_cursor_so_advance_after_reset_does_nothing() {
        crate::gpu_va::insert(crate::gpu_va::Entry { start: 0x7e_0000_0000, len: 0x100, res: 0x7890, buffer: true });
        let vt: &Vec<usize> = Box::leak(Box::new(fake_vt()));
        let l: &FakeList = Box::leak(Box::new(FakeList { vt: vt.as_ptr(), ty: LIST_TYPE_DIRECT }));
        let p = l as *const FakeList as *mut c_void;
        unsafe { set_predication_buffer(p, &PredicationBufferDesc { va: 0x7e_0000_0000, offset: 0 }) };
        assert!(cursor(p as usize).is_some());
        forget(crate::list_split::origin_of(p));
        assert_eq!(cursor(p as usize), None);
        unsafe { advance_predication(p) };
        assert_eq!(calls_for(p as usize), vec![(0x7890, 0, 0)], "an AdvancePredicationX after Reset must not re-arm the previous recording's predicate");
    }

    #[test]
    fn plain_null_set_predication_is_a_noop_unless_a_resource_predicate_is_active() {
        assert!(crate::cl2_state::plain_null_is_noop(false, 0));
        assert!(!crate::cl2_state::plain_null_is_noop(true, 0));
        assert!(!crate::cl2_state::plain_null_is_noop(false, 0x1234));
        assert!(!crate::cl2_state::plain_null_is_noop(true, 0x1234));
    }

    #[test]
    fn predicate_set_by_buffer_and_advance_is_the_one_the_list_state_restores() {
        crate::gpu_va::insert(crate::gpu_va::Entry { start: 0x7d_0000_0000, len: 0x100, res: 0x4560, buffer: true });
        let vt: &Vec<usize> = Box::leak(Box::new(fake_vt()));
        let l: &FakeList = Box::leak(Box::new(FakeList { vt: vt.as_ptr(), ty: LIST_TYPE_DIRECT }));
        let p = l as *const FakeList as *mut c_void;
        unsafe {
            set_predication_buffer(p, &PredicationBufferDesc { va: 0x7d_0000_0000, offset: 0x10 });
            advance_predication(p);
        }
        assert_eq!(calls_for(p as usize).last().copied(), Some((0x4560, 0x18, 0)));
        assert_eq!(crate::cl2_state::predication_of(p), Some((0x4560, 0x18, 0)), "Write64BitValueBottomOfPipeX and list splits lift and restore the predicate from cl2_state, so it must match the one the PC list now has");
    }

    #[test]
    fn clear_state_drops_the_cursor_so_advance_after_clear_state_does_nothing() {
        crate::gpu_va::insert(crate::gpu_va::Entry { start: 0x7f_0000_0000, len: 0x100, res: 0x3344, buffer: true });
        let vt: &Vec<usize> = Box::leak(Box::new(fake_vt()));
        let l: &FakeList = Box::leak(Box::new(FakeList { vt: vt.as_ptr(), ty: LIST_TYPE_DIRECT }));
        let p = l as *const FakeList as *mut c_void;
        unsafe { set_predication_buffer(p, &PredicationBufferDesc { va: 0x7f_0000_0000, offset: 0 }) };
        assert!(cursor(p as usize).is_some());
        assert_eq!(crate::cl2_state::predication_of(p), Some((0x3344, 0, 0)));
        unsafe { crate::cl2_state::w_clear_state(p, core::ptr::null_mut()) };
        assert_eq!(cursor(p as usize), None, "ClearState must drop the predication cursor");
        assert_eq!(crate::cl2_state::predication_of(p), None, "ClearState must clear predication in list state");
        unsafe { advance_predication(p) };
        assert_eq!(calls_for(p as usize), vec![(0x3344, 0, 0)], "an AdvancePredicationX after ClearState must not re-arm the previous recording's predicate");
    }

    #[test]
    fn buffer_predication_clears_plain_predicate_flag_so_subsequent_null_predication_is_noop() {
        crate::gpu_va::insert(crate::gpu_va::Entry { start: 0x8a_0000_0000, len: 0x100, res: 0x5566, buffer: true });
        let vt: &Vec<usize> = Box::leak(Box::new(fake_vt()));
        let l: &FakeList = Box::leak(Box::new(FakeList { vt: vt.as_ptr(), ty: LIST_TYPE_DIRECT }));
        let p = l as *const FakeList as *mut c_void;
        unsafe { crate::cl2_state::w_predication(p, 0x1234 as *mut c_void, 0, 0) };
        assert_eq!(crate::cl2_state::predication_of(p), Some((0x1234, 0, 0)));
        unsafe { set_predication_buffer(p, &PredicationBufferDesc { va: 0x8a_0000_0000, offset: 0 }) };
        assert_eq!(crate::cl2_state::predication_of(p), Some((0x5566, 0, 0)));
        let calls_before = calls_for(p as usize).len();
        unsafe { crate::cl2_state::w_predication(p, core::ptr::null_mut(), 0, 0) };
        assert_eq!(crate::cl2_state::predication_of(p), Some((0x5566, 0, 0)), "SetPredication(NULL) must not disable buffer predication");
        assert_eq!(calls_for(p as usize).len(), calls_before, "SetPredication(NULL) with buffer predication must emit no PC packet");
    }
}
