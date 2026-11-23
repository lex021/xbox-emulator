use core::ffi::c_void;
use core::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::Mutex;
use compat_common::record;

use crate::device_wrap::Guid;

use crate::cl_ext::{bop64_route, bop_error, Bop64, BOP_FLAG_GPU_TIMESTAMP, BOP_FLAG_SYSTEM_TIMESTAMP, BOP_FLUSH_VALID};
use crate::console_slots::WBI_MODE_MARKER_OUT;

pub const EOP_HEADER: u32 = 0xc004_4700;
pub const EOP_EVENT_BOTTOM_OF_PIPE_TS: u32 = 0x528;
pub const EOP_L2_POLICY_FLAG: u32 = 0x20;
pub const EOP_L2_HINT_FLAGS: u32 = 0x30;
pub const DATA_SEL_VALUE: u32 = 2;
pub const DATA_SEL_GPU_COUNTER: u32 = 3;
pub const DATA_SEL_SYSTEM_COUNTER: u32 = 4;
const LIST_TYPE_DIRECT: u32 = 0;
const PC_SLOT_RESOURCE_BARRIER: usize = 26;
const BARRIER_TYPE_UAV: u32 = 2;
const W: &str = "Write64BitValueBottomOfPipeX";

static LOG: AtomicUsize = AtomicUsize::new(0);

fn once(n: usize) -> bool {
    LOG.fetch_add(1, Ordering::Relaxed) < n
}

pub fn packet(va: u64, value: u64, flush: u32, flags: u32) -> [u32; 6] {
    let cntl = ((((flags & EOP_L2_POLICY_FLAG) << 16) | (flush & BOP_FLUSH_VALID)) << 4) | EOP_EVENT_BOTTOM_OF_PIPE_TS;
    let sel = if flags & BOP_FLAG_GPU_TIMESTAMP != 0 {
        DATA_SEL_GPU_COUNTER
    } else if flags & BOP_FLAG_SYSTEM_TIMESTAMP != 0 {
        DATA_SEL_SYSTEM_COUNTER
    } else {
        DATA_SEL_VALUE
    };
    let hint = if flags & EOP_L2_HINT_FLAGS != 0 { 0x1_0000 } else { 0 };
    [EOP_HEADER, cntl, va as u32, ((va >> 32) as u32) | hint | (sel << 29), value as u32, (value >> 32) as u32]
}

pub const RELEASE_MEM_HEADER: u32 = 0xc006_4900;
pub const RELEASE_MEM_EVENT_CACHE_FLUSH_TS: u32 = 0x514;
const RELEASE_MEM_EVENT_INDEX_EOP_BIT: u32 = 1 << 25;

pub fn gcr_of_flush(flush: u32) -> u32 {
    let mut g = 0;
    if flush & 0x2_0000 != 0 { g |= 0x10_0000; }
    if flush & 0x1000 != 0 { g |= 0x300; }
    if flush & 0x2000 != 0 { g |= 0xc030; }
    if flush & 0x800 != 0 { g |= 0x8010; }
    if flush & 0x20_0000 != 0 { g |= 0x8000_0000; }
    g
}

pub fn release_mem(va: u64, value: u64, flush: u32, flags: u32) -> [u32; 8] {
    let g = gcr_of_flush(flush);
    let gcr = (((g & 0x30) << 2) | (g & 0xdf00)) << 6;
    let ev = if gcr != 0 { gcr | RELEASE_MEM_EVENT_CACHE_FLUSH_TS } else { EOP_EVENT_BOTTOM_OF_PIPE_TS };
    let cntl = if flags & EOP_L2_POLICY_FLAG != 0 { ev | RELEASE_MEM_EVENT_INDEX_EOP_BIT } else { ev };
    let (sel, value) = if flags & (BOP_FLAG_GPU_TIMESTAMP | BOP_FLAG_SYSTEM_TIMESTAMP) == 0 {
        (DATA_SEL_VALUE, value)
    } else if flags & BOP_FLAG_SYSTEM_TIMESTAMP != 0 {
        (DATA_SEL_SYSTEM_COUNTER, 0)
    } else {
        (DATA_SEL_GPU_COUNTER, 0)
    };
    let hint = if flags & EOP_L2_HINT_FLAGS != 0 { 0x1_0000 } else { 0 };
    [RELEASE_MEM_HEADER, cntl, hint | (sel << 29), va as u32, (va >> 32) as u32, value as u32, (value >> 32) as u32, 0]
}

pub fn decode_release_mem(p: &[u32; 8], flush: u32) -> Option<Eop> {
    let kind = match p[2] >> 29 {
        DATA_SEL_VALUE => Bop64::Value,
        DATA_SEL_GPU_COUNTER => Bop64::GpuTimestamp,
        DATA_SEL_SYSTEM_COUNTER => Bop64::SystemTimestamp,
        _ => return None,
    };
    let caches = p[1] & 0xff != EOP_EVENT_BOTTOM_OF_PIPE_TS & 0xff;
    Some(Eop { kind, flush: if caches { flush & BOP_FLUSH_VALID } else { 0 }, value: p[5] as u64 | (p[6] as u64) << 32 })
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Eop {
    pub kind: Bop64,
    pub flush: u32,
    pub value: u64,
}

pub fn decode(p: &[u32; 6]) -> Option<Eop> {
    let kind = match p[3] >> 29 {
        DATA_SEL_VALUE => Bop64::Value,
        DATA_SEL_GPU_COUNTER => Bop64::GpuTimestamp,
        DATA_SEL_SYSTEM_COUNTER => Bop64::SystemTimestamp,
        _ => return None,
    };
    Some(Eop { kind, flush: (p[1] >> 4) & BOP_FLUSH_VALID, value: p[4] as u64 | (p[5] as u64) << 32 })
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Plan {
    Write { eop: Eop, diagnostic: Option<&'static str> },
    Reject(&'static str),
}

pub fn plan(scarlett: bool, list_type: u32, closed: bool, va: u64, value: u64, flush: u32, flags: u32) -> Plan {
    if list_type != LIST_TYPE_DIRECT {
        return Plan::Reject("pCommandList is not of type DIRECT.");
    }
    if closed {
        return Plan::Reject("64 bit value writes cannot be inserted into a closed command list.");
    }
    if let Err(e) = bop64_route(va, flags) {
        return Plan::Reject(e);
    }
    let eop = if scarlett { decode_release_mem(&release_mem(va, value, flush, flags), flush) } else { decode(&packet(va, value, flush, flags)) };
    match eop {
        Some(eop) => Plan::Write { eop, diagnostic: bop_error(flush, flags, true, value) },
        None => Plan::Reject("unencodable EVENT_WRITE_EOP data select"),
    }
}

pub fn scarlett() -> bool {
    crate::console_slots_dev2::family(xgs_kmd_emu::device_type()) == crate::console_slots_dev2::Family::Scarlett
}

unsafe fn vslot(obj: *mut c_void, i: usize) -> usize {
    unsafe { *(*(obj as *const *const usize)).add(i) }
}

unsafe fn flush_barrier(list: *mut c_void) {
    let mut b = [0u8; 32];
    b[0..4].copy_from_slice(&BARRIER_TYPE_UAV.to_le_bytes());
    let f: unsafe extern "system" fn(*mut c_void, u32, *const u8) = unsafe { core::mem::transmute(crate::cmdlist_wrap::pc_method(list, PC_SLOT_RESOURCE_BARRIER)) };
    unsafe { f(list, 1, b.as_ptr()) };
}

pub unsafe extern "system" fn write64(this: *mut c_void, va: u64, value: u64, flush: u32, flags: u32) {
    let ty = unsafe { crate::fill::list_type(this) };
    let closed = ty == LIST_TYPE_DIRECT && crate::cl2_state::title_closed(this);
    let eop = match plan(scarlett(), ty, closed, va, value, flush, flags) {
        Plan::Reject(why) => {
            if once(16) {
                record("d3d12_xs", "bop64", &format!("{W}(va={va:#x}, flush={flush:#x}, flags={flags:#x}): {why} (console validation); not recorded"));
            }
            return;
        }
        Plan::Write { eop, diagnostic } => {
            static FIRST: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);
            if !FIRST.swap(true, Ordering::Relaxed) {
                record("d3d12_xs", "bop64", &format!("first call {W}(va={va:#x}, value={value:#x}, flush={flush:#x}, flags={flags:#x}) list type {ty}: recorded as {:?}", eop.kind));
            }
            if let Some(d) = diagnostic {
                if once(16) {
                    record("d3d12_xs", "bop64", &format!("{W}(va={va:#x}, value={value:#x}, flush={flush:#x}, flags={flags:#x}): debug UMD reports \"{d}\"; the console UMD still emits the end-of-pipe packet it encodes ({eop:?}), emulated the same"));
                }
            }
            eop
        }
    };
    if eop.flush != 0 {
        unsafe { flush_barrier(this) };
    }
    match eop.kind {
        Bop64::GpuTimestamp => unsafe { crate::timestamp::write(this, false, va, flags, flush, 0) },
        Bop64::SystemTimestamp => unsafe { crate::sys_ts::write(this, va) },
        Bop64::Value => unsafe { write_value(this, va, eop.value) },
    }
}

pub const SCRATCH_SLOTS: u32 = 4096;
const STATE_COMMON: u32 = 0;
const STATE_COPY_DEST: u32 = 0x400;
const STATE_COPY_SOURCE: u32 = 0x800;
const STATE_GENERIC_READ: u32 = 0xac3;
const PC_SLOT_COPY_BUFFER_REGION: usize = 15;
const PC_SLOT_SET_PREDICATION: usize = 55;
const PC_SLOT_WRITE_BUFFER_IMMEDIATE: usize = 66;
const DEV_CREATE_COMMITTED: usize = 27;
const IID_DEVICE: Guid = Guid { d1: 0x1898_19f1, d2: 0x1db6, d3: 0x4b57, d4: [0xbe, 0x54, 0x18, 0x21, 0x33, 0x9b, 0x85, 0xf7] };
const IID_RESOURCE: Guid = Guid { d1: 0x6964_42be, d2: 0xa72e, d3: 0x4059, d4: [0xbc, 0x79, 0x5b, 0x5c, 0x98, 0x04, 0x0f, 0xad] };

#[derive(Clone, Copy)]
struct Scratch {
    res: usize,
    va: u64,
}

static SCRATCH: Mutex<Option<Option<Scratch>>> = Mutex::new(None);
static NEXT: AtomicU32 = AtomicU32::new(0);

pub fn scratch_offset(counter: &AtomicU32) -> u64 {
    (counter.fetch_add(1, Ordering::Relaxed) % SCRATCH_SLOTS) as u64 * 8
}

unsafe fn scratch(list: *mut c_void) -> Option<Scratch> {
    let mut g = SCRATCH.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(s) = *g {
        return s;
    }
    let get: unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> i32 = unsafe { core::mem::transmute(vslot(list, 7)) };
    let mut dev: *mut c_void = core::ptr::null_mut();
    if unsafe { get(list, &IID_DEVICE, &mut dev) } < 0 || dev.is_null() {
        return None;
    }
    let heap: [u32; 5] = [1, 0, 0, 1, 1];
    let mut rd = [0u8; 56];
    rd[0..4].copy_from_slice(&1u32.to_le_bytes());
    rd[0x10..0x18].copy_from_slice(&(SCRATCH_SLOTS as u64 * 8).to_le_bytes());
    rd[0x18..0x1c].copy_from_slice(&1u32.to_le_bytes());
    rd[0x1c..0x1e].copy_from_slice(&1u16.to_le_bytes());
    rd[0x1e..0x20].copy_from_slice(&1u16.to_le_bytes());
    rd[0x24..0x28].copy_from_slice(&1u32.to_le_bytes());
    rd[0x2c..0x30].copy_from_slice(&1u32.to_le_bytes());
    let cc: unsafe extern "system" fn(*mut c_void, *const u32, u32, *const u8, u32, *const c_void, *const Guid, *mut *mut c_void) -> i32 = unsafe { core::mem::transmute(vslot(dev, DEV_CREATE_COMMITTED)) };
    let mut buf: *mut c_void = core::ptr::null_mut();
    let hr = unsafe { cc(dev, heap.as_ptr(), 0, rd.as_ptr(), STATE_COMMON, core::ptr::null(), &IID_RESOURCE, &mut buf) };
    let rel: unsafe extern "system" fn(*mut c_void) -> u32 = unsafe { core::mem::transmute(vslot(dev, 2)) };
    unsafe { rel(dev) };
    let s = if hr < 0 || buf.is_null() {
        record("d3d12_xs", "bop64", &format!("64-bit value scratch buffer FAILED hr={hr:#010x}; values fall back to two 32-bit immediate writes"));
        None
    } else {
        let va: unsafe extern "system" fn(*mut c_void) -> u64 = unsafe { core::mem::transmute(vslot(buf, 11)) };
        Some(Scratch { res: buf as usize, va: unsafe { va(buf) } })
    };
    *g = Some(s);
    s
}

unsafe fn write_value(list: *mut c_void, va: u64, value: u64) {
    let _ij = unsafe { crate::inject::scope(list, crate::inject::COPY) };
    let words = [value as u32, (value >> 32) as u32];
    let fallback = |why: &str| {
        if once(16) {
            record("d3d12_xs", "bop64", &format!("{W} {va:#x}: {why}"));
        }
    };
    let Some((e, off)) = crate::gpu_va::lookup(va) else {
        return fallback("destination is not in graphics memory owned by a tracked resource");
    };
    if !e.buffer || off + 8 > e.len {
        return fallback("destination is not inside a buffer; texture memory has no PC byte address");
    }
    let st = crate::gpu_va::state(e.res);
    let Some((db, _)) = crate::console_slots::copy_plan(st, STATE_COPY_SOURCE, st == STATE_GENERIC_READ) else {
        return fallback("destination is an upload-heap buffer, which the GPU cannot write on PC");
    };
    if let Some((a, _)) = db {
        if !crate::fill::state_legal_on(unsafe { crate::fill::list_type(list) }, a) {
            return fallback("destination state cannot be transitioned on this list");
        }
    }
    let Some(s) = (unsafe { scratch(list) }) else {
        return unsafe { crate::console_slots::write_immediate(list, va, &words, WBI_MODE_MARKER_OUT, W) };
    };
    let soff = scratch_offset(&NEXT);
    let params = crate::console_slots::wbi_params(s.va + soff, &words);
    let modes = [WBI_MODE_MARKER_OUT; 2];
    let pred = crate::cl2_state::predication_of(list);
    unsafe {
        let sp: unsafe extern "system" fn(*mut c_void, *mut c_void, u64, u32) = core::mem::transmute(crate::cmdlist_wrap::pc_method(list, PC_SLOT_SET_PREDICATION));
        if pred.is_some() {
            sp(list, core::ptr::null_mut(), 0, 0);
        }
        crate::fill::barrier_pub(list, s.res, STATE_COMMON, STATE_COPY_DEST);
        let wbi: unsafe extern "system" fn(*mut c_void, u32, *const crate::console_slots::WbiParam, *const u32) = core::mem::transmute(crate::cmdlist_wrap::pc_method(list, PC_SLOT_WRITE_BUFFER_IMMEDIATE));
        wbi(list, 2, params.as_ptr(), modes.as_ptr());
        crate::fill::barrier_pub(list, s.res, STATE_COPY_DEST, STATE_COPY_SOURCE);
        if let Some((a, b)) = db { crate::fill::barrier_pub(list, e.res, a, b); }
        let cp: unsafe extern "system" fn(*mut c_void, usize, u64, usize, u64, u64) = core::mem::transmute(crate::cmdlist_wrap::pc_method(list, PC_SLOT_COPY_BUFFER_REGION));
        cp(list, e.res, off, s.res, soff, 8);
        if let Some((a, b)) = db { crate::fill::barrier_pub(list, e.res, b, a); }
        crate::fill::barrier_pub(list, s.res, STATE_COPY_SOURCE, STATE_COMMON);
        if let Some((r, o, op)) = pred {
            sp(list, r as *mut c_void, o, op);
        }
    }
    crate::placed::note_gpu_write(e.res as *mut c_void);
    crate::gpu_wait::note(list, crate::gpu_wait::Op::Write { addr: va, value });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kind_on(scarlett: bool, flags: u32) -> Bop64 {
        match plan(scarlett, 0, false, 0x1000, 0, 0, flags) {
            Plan::Write { eop, .. } => eop.kind,
            Plan::Reject(e) => panic!("{e}"),
        }
    }

    fn kind(flags: u32) -> Bop64 {
        kind_on(false, flags)
    }

    #[test]
    fn release_mem_matches_the_scarlett_release_umd_encoding() {
        assert_eq!(release_mem(0x12_3456_7890, 0x1122_3344_5566_7788, 0, 0), [0xc006_4900, 0x528, 0x4000_0000, 0x3456_7890, 0x12, 0x5566_7788, 0x1122_3344, 0]);
        assert_eq!(release_mem(0x1000, 9, 0, 1), [0xc006_4900, 0x528, 0x6000_0000, 0x1000, 0, 0, 0, 0]);
        assert_eq!(release_mem(0x1000, 0, 0, 2)[2], 0x8000_0000);
        assert_eq!(release_mem(0x1000, 0, 0, 3)[2], 0x8000_0000);
        assert_eq!(release_mem(0x1000, 0, 0x800, 0)[1], 0x20_1514);
        assert_eq!(release_mem(0x1000, 0, 0x1000, 0)[1], 0xc514);
        assert_eq!(release_mem(0x1000, 0, 0x2000, 0)[1], 0x30_3514);
        assert_eq!(release_mem(0x1000, 0, 0x3800, 0)[1], 0x30_f514);
        assert_eq!(release_mem(0x1000, 0, 0, 0x20)[1], 0x200_0528);
        assert_eq!(release_mem(0x1000, 0, 0, 0x10)[2], 0x4001_0000);
        assert_eq!(release_mem(0x1000, 0, 0x4_0000, 0)[1], 0x528);
        assert_eq!(gcr_of_flush(0x3800), 0xc330);
        assert_eq!(gcr_of_flush(0x22_0000), 0x8010_0000);
    }

    #[test]
    fn scarlett_lets_system_timestamp_win_and_keeps_the_flush() {
        assert_eq!(kind_on(true, 0), Bop64::Value);
        assert_eq!(kind_on(true, BOP_FLAG_GPU_TIMESTAMP), Bop64::GpuTimestamp);
        assert_eq!(kind_on(true, BOP_FLAG_SYSTEM_TIMESTAMP), Bop64::SystemTimestamp);
        assert_eq!(kind_on(true, BOP_FLAG_GPU_TIMESTAMP | BOP_FLAG_SYSTEM_TIMESTAMP), Bop64::SystemTimestamp);
        assert_eq!(kind_on(true, 0x80), Bop64::Value);
        match plan(true, 0, false, 0x2000, 7, 0x4800, 0x10) {
            Plan::Write { eop, diagnostic } => {
                assert_eq!(eop, Eop { kind: Bop64::Value, flush: 0x800, value: 7 });
                assert_eq!(diagnostic, Some("flush flags can only include BOP_TEXTURE_Ln_INVALIDATE and BOP_TEXTURE_L2_WRITEBACK"));
            }
            p => panic!("{p:?}"),
        }
        match plan(true, 0, false, 0x2000, 0, 0, 2) {
            Plan::Write { eop, .. } => assert_eq!(eop, Eop { kind: Bop64::SystemTimestamp, flush: 0, value: 0 }),
            p => panic!("{p:?}"),
        }
        assert_eq!(plan(true, 1, false, 0x1000, 0, 0, 0), Plan::Reject("pCommandList is not of type DIRECT."));
        assert_eq!(plan(true, 0, true, 0x1000, 0, 0, 0), Plan::Reject("64 bit value writes cannot be inserted into a closed command list."));
    }

    #[test]
    fn packet_matches_the_release_umd_encoding() {
        assert_eq!(packet(0x12_3456_7890, 0x1122_3344_5566_7788, 0, 0), [0xc004_4700, 0x528, 0x3456_7890, 0x4000_0012, 0x5566_7788, 0x1122_3344]);
        assert_eq!(packet(0x1000, 0, 0x3800, 1), [0xc004_4700, 0x3_8528, 0x1000, 0x6000_0000, 0, 0]);
        assert_eq!(packet(0x1000, 0, 0x800, 2), [0xc004_4700, 0x8528, 0x1000, 0x8000_0000, 0, 0]);
        assert_eq!(packet(0x1000, 0, 0, 0x10)[3], 0x4001_0000);
        assert_eq!(packet(0x1000, 0, 0, 0x20), [0xc004_4700, 0x200_0528, 0x1000, 0x4001_0000, 0, 0]);
        assert_eq!(packet(0x1000, 0, 0xffff_ffff, 0)[1], 0x3_8528);
    }

    #[test]
    fn data_select_follows_the_release_bit_order() {
        assert_eq!(kind(0), Bop64::Value);
        assert_eq!(kind(BOP_FLAG_GPU_TIMESTAMP), Bop64::GpuTimestamp);
        assert_eq!(kind(BOP_FLAG_SYSTEM_TIMESTAMP), Bop64::SystemTimestamp);
        assert_eq!(kind(BOP_FLAG_GPU_TIMESTAMP | BOP_FLAG_SYSTEM_TIMESTAMP), Bop64::GpuTimestamp);
        assert_eq!(kind(0x30 | BOP_FLAG_SYSTEM_TIMESTAMP), Bop64::SystemTimestamp);
        assert_eq!(kind(0x80), Bop64::Value);
    }

    #[test]
    fn debug_errors_are_reported_but_the_write_is_kept() {
        let cases: [(u64, u32, u32, &str); 5] = [
            (0, 0, 0x40, "invalid flags"),
            (0, 0, 3, "GPU_TIMESTAMP and SYSTEM_TIMESTAMP are exclusive"),
            (5, 0, 1, "Value must be 0 with a TIMESTAMP flag"),
            (0, 0, 0x30, "CACHE_L2_LRU and CACHE_L2_STREAM are exclusive"),
            (0, 0x4000, 0, "flush flags can only include BOP_TEXTURE_Ln_INVALIDATE and BOP_TEXTURE_L2_WRITEBACK"),
        ];
        for (value, flush, flags, msg) in cases {
            match plan(false, 0, false, 0x2000, value, flush, flags) {
                Plan::Write { diagnostic, .. } => assert_eq!(diagnostic, Some(msg)),
                p => panic!("{p:?}"),
            }
        }
        match plan(false, 0, false, 0x2000, 5, 0x4800, 1) {
            Plan::Write { eop, .. } => assert_eq!(eop, Eop { kind: Bop64::GpuTimestamp, flush: 0x800, value: 5 }),
            p => panic!("{p:?}"),
        }
        match plan(false, 0, false, 0x2000, 7, 0x1000, 0x10) {
            Plan::Write { eop, diagnostic } => {
                assert_eq!(diagnostic, None);
                assert_eq!(eop, Eop { kind: Bop64::Value, flush: 0x1000, value: 7 });
            }
            p => panic!("{p:?}"),
        }
    }

    #[test]
    fn scratch_slots_are_8_byte_aligned_and_wrap() {
        let c = AtomicU32::new(SCRATCH_SLOTS - 1);
        assert_eq!(scratch_offset(&c), (SCRATCH_SLOTS as u64 - 1) * 8);
        assert_eq!(scratch_offset(&c), 0);
        assert_eq!(scratch_offset(&c), 8);
    }

    #[test]
    fn list_and_address_errors_are_not_recorded() {
        assert_eq!(plan(false, 2, false, 0x1000, 0, 0, 0), Plan::Reject("pCommandList is not of type DIRECT."));
        assert_eq!(plan(false, 1, false, 0x1000, 0, 0, 0), Plan::Reject("pCommandList is not of type DIRECT."));
        assert_eq!(plan(false, 0, true, 0x1000, 0, 0, 0), Plan::Reject("64 bit value writes cannot be inserted into a closed command list."));
        assert!(matches!(plan(false, 0, false, 0x1004, 0, 0, 0), Plan::Reject(_)));
        assert!(matches!(plan(false, 0, false, 0x100_0000_0000, 0, 0, 0), Plan::Reject(_)));
        assert!(matches!(plan(false, 0, false, 0, 0, 0, 0), Plan::Reject(_)));
    }
}
