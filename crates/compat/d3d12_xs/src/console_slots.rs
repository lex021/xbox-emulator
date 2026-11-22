use core::ffi::c_void;
use core::sync::atomic::{AtomicUsize, Ordering};
use std::collections::HashMap;
use std::sync::Mutex;
use compat_common::record;

pub const DEVICE_SLOTS: usize = 90;
pub const CMDLIST_SLOTS: usize = 110;

const S_OK: i32 = 0;
const E_NOTIMPL: i32 = 0x8000_4001u32 as i32;

pub const DEV_S_OK: [usize; 1] = [53];
pub const DEV_VOID: [usize; 2] = [54, 65];
pub const DEV_ALL_ONES: [usize; 1] = [64];
pub const DEV_NOTIMPL: [usize; 2] = [69, 89];
pub const DEV_STORE3: usize = 50;
pub const CL_VOID: [usize; 1] = [87];
pub const CL_COPY_MEMORY: usize = 60;
pub const CL_FILL32: usize = 61;
pub const CL_FILL64: usize = 62;
pub const CL_FILL128: usize = 63;
pub const CL_NOP: usize = 82;
const STATE_COPY_DEST: u32 = 0x400;
const STATE_COPY_SOURCE: u32 = 0x800;
const STATE_GENERIC_READ: u32 = 0xac3;
const PC_SLOT_COPY_BUFFER_REGION: usize = 15;
pub const CL_PC_LIST1_FIRST: usize = 104;
pub const PC_LIST1_FIRST: usize = 60;
pub const CL_PC_PASSTHROUGH: [usize; 6] = [104, 105, 106, 107, 108, 109];

pub fn pc_slot_for_list1(xbox_slot: usize) -> Option<usize> {
    (CL_PC_LIST1_FIRST..CL_PC_LIST1_FIRST + 6).contains(&xbox_slot).then(|| xbox_slot - CL_PC_LIST1_FIRST + PC_LIST1_FIRST)
}

static DEVICE_STATE: Mutex<Option<HashMap<usize, [u64; 3]>>> = Mutex::new(None);

pub fn device_state(this: usize) -> Option<[u64; 3]> {
    DEVICE_STATE.lock().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(|m| m.get(&this).copied())
}

unsafe extern "system" fn dev_s_ok(_this: *mut c_void) -> i32 {
    S_OK
}

unsafe extern "system" fn dev_void(_this: *mut c_void) {}

unsafe extern "system" fn dev_all_ones(_this: *mut c_void) -> u32 {
    0xffff_ffff
}

unsafe extern "system" fn dev_notimpl(_this: *mut c_void) -> i32 {
    E_NOTIMPL
}

unsafe extern "system" fn dev_store3(this: *mut c_void, a: u64, b: u64, c: u64) {
    DEVICE_STATE.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).insert(this as usize, [a, b, c]);
}

unsafe extern "system" fn cl_void(_this: *mut c_void) {}

fn once(c: &AtomicUsize, n: usize) -> bool {
    c.fetch_add(1, Ordering::Relaxed) < n
}

pub fn fill_size_ok(size: u64, unit: u64) -> bool {
    size != 0 && size % unit == 0
}

pub fn nop_size_ok(size: u32) -> bool {
    size != 0 && size % 4 == 0 && size < 0x1_0000
}

pub fn fill128_pattern(v: &[u8; 16]) -> [u8; 16] {
    *v
}

static FILL_LOG: AtomicUsize = AtomicUsize::new(0);

unsafe fn fill_checked(list: *mut c_void, va: u64, size: u64, pat: &[u8; 16], unit: u64, what: &str) {
    if !fill_size_ok(size, unit) {
        if once(&FILL_LOG, 8) {
            record("d3d12_xs", "console_slots", &format!("{what}: size {size:#x} is not a non-zero multiple of {unit} (console validation), ignored"));
        }
        return;
    }
    unsafe { crate::fill::fill_pattern(list, false, va, size, pat, 0) }
}

unsafe extern "system" fn cl_fill32(this: *mut c_void, va: u64, size: u64, value: u32) {
    unsafe { fill_checked(this, va, size, &crate::fill::pattern16_32(value), 4, "FillMemoryWith32BitValueX") }
}

unsafe extern "system" fn cl_fill64(this: *mut c_void, va: u64, size: u64, value: u64) {
    unsafe { fill_checked(this, va, size, &crate::console_slots_cl2::fill64_pattern(value), 8, "FillMemoryWith64BitValueX") }
}

unsafe extern "system" fn cl_fill128(this: *mut c_void, va: u64, size: u64, value: *const [u8; 16]) {
    if value.is_null() {
        return;
    }
    let pat = fill128_pattern(unsafe { &*value });
    unsafe { fill_checked(this, va, size, &pat, 16, "FillMemoryWith128BitValueX") }
}

static NOP_LOG: AtomicUsize = AtomicUsize::new(0);

unsafe extern "system" fn cl_nop(_this: *mut c_void, size: u32) {
    if !nop_size_ok(size) && once(&NOP_LOG, 8) {
        record("d3d12_xs", "console_slots", &format!("Nop: SizeBytes {size} must be a non-zero multiple of 4 below 64 KiB (console validation)"));
    }
}

static COPY_LOG: AtomicUsize = AtomicUsize::new(0);

pub fn copy_plan(dst_state: u32, src_state: u32, dst_upload: bool) -> Option<(Option<(u32, u32)>, Option<(u32, u32)>)> {
    if dst_upload {
        return None;
    }
    let d = (dst_state != STATE_COPY_DEST).then_some((dst_state, STATE_COPY_DEST));
    let s = (src_state != STATE_COPY_SOURCE && src_state != STATE_GENERIC_READ).then_some((src_state, STATE_COPY_SOURCE));
    Some((d, s))
}

unsafe extern "system" fn cl_copy_memory(this: *mut c_void, dst: u64, src: u64, size: u64, flags: u32) {
    if size == 0 || size % 4 != 0 {
        if once(&COPY_LOG, 8) {
            record("d3d12_xs", "console_slots", &format!("CopyMemoryToMemoryX: size {size:#x} must be a non-zero multiple of 4 (console validation)"));
        }
        return;
    }
    let (Some((de, doff)), Some((se, soff))) = (crate::gpu_va::lookup(dst), crate::gpu_va::lookup(src)) else {
        if once(&COPY_LOG, 8) {
            record("d3d12_xs", "console_slots", &format!("CopyMemoryToMemoryX dst={dst:#x} src={src:#x} size={size:#x}: address not owned by any tracked resource"));
        }
        return;
    };
    if !de.buffer || !se.buffer {
        if once(&COPY_LOG, 8) {
            record("d3d12_xs", "console_slots", &format!("CopyMemoryToMemoryX dst={dst:#x} src={src:#x}: texture memory has console tiling with no PC byte mapping, copy skipped"));
        }
        return;
    }
    let len = size.min(de.len - doff).min(se.len - soff);
    if de.res == se.res {
        if once(&COPY_LOG, 8) {
            record("d3d12_xs", "console_slots", "CopyMemoryToMemoryX within one buffer needs a temporary on PC (one resource cannot be COPY_SOURCE and COPY_DEST at once); copy skipped");
        }
        return;
    }
    let ds = crate::gpu_va::state(de.res);
    let ss = crate::gpu_va::state(se.res);
    let Some((db, sb)) = copy_plan(ds, ss, ds == STATE_GENERIC_READ) else {
        if once(&COPY_LOG, 8) {
            record("d3d12_xs", "console_slots", &format!("CopyMemoryToMemoryX dst={dst:#x} is an upload-heap buffer, which cannot be a GPU copy destination on PC"));
        }
        return;
    };
    let ty = unsafe { crate::fill::list_type(this) };
    for (b, _) in [db, sb].iter().flatten() {
        if !crate::fill::state_legal_on(ty, *b) {
            if once(&COPY_LOG, 8) {
                record("d3d12_xs", "console_slots", &format!("CopyMemoryToMemoryX: a resource is in state {b:#x}, which a type-{ty} list cannot transition; copy skipped"));
            }
            return;
        }
    }
    unsafe {
        if let Some((a, b)) = db { crate::fill::barrier_pub(this, de.res, a, b); }
        if let Some((a, b)) = sb { crate::fill::barrier_pub(this, se.res, a, b); }
        let vt = *(this as *const *const usize);
        let copy: unsafe extern "system" fn(*mut c_void, *mut c_void, u64, *mut c_void, u64, u64) = core::mem::transmute(*vt.add(PC_SLOT_COPY_BUFFER_REGION));
        copy(this, de.res as *mut c_void, doff, se.res as *mut c_void, soff, len);
        if let Some((a, b)) = sb { crate::fill::barrier_pub(this, se.res, b, a); }
        if let Some((a, b)) = db { crate::fill::barrier_pub(this, de.res, b, a); }
    }
    crate::placed::note_gpu_write(de.res as *mut c_void);
    if once(&COPY_LOG, 12) {
        record("d3d12_xs", "console_slots", &format!("CopyMemoryToMemoryX dst={dst:#x} src={src:#x} size={size:#x} flags={flags:#x} -> CopyBufferRegion {len:#x} bytes"));
    }
}

pub const CL_DISPATCH_X: usize = 78;
pub const CL_DRAW_INDEXED_X: usize = 81;
pub const CL_SET_PREDICATION_BUFFER: usize = 85;
pub const CL_WRITE32_TOP: usize = 88;
pub const CL_WRITE64_TOP: usize = 89;
pub const CL_WRITE32_BOTTOM_OF_SHADER: usize = 90;
pub const CL_PREFETCH_MEMORY: usize = 98;
const XBOX_SLOT_IA_SET_INDEX_BUFFER: usize = 47;
const PC_SLOT_DRAW_INDEXED: usize = 13;
const PC_SLOT_DISPATCH: usize = 14;
const PC_SLOT_IA_SET_INDEX_BUFFER: usize = 43;
const PC_SLOT_WRITE_BUFFER_IMMEDIATE: usize = 66;
pub const WBI_MODE_MARKER_IN: u32 = 1;
pub const WBI_MODE_MARKER_OUT: u32 = 2;
pub const DISPATCH_X_INVALID_FLAGS: u32 = 0xffff_f103;
pub const WRITE_TOP_FLAG_GPU_TIMESTAMP: u32 = 0x4;
pub const WRITE_TOP_FLAG_SYSTEM_TIMESTAMP: u32 = 0x8;
pub const DXGI_FORMAT_R32_UINT: u32 = 42;
pub const DXGI_FORMAT_R16_UINT: u32 = 57;
pub const PREFETCH_MAX_BYTES: u32 = 0x20_0000;
pub const GRAPHICS_VA_HIGH_MASK: u64 = 0xffff_ff00_0000_0000;

unsafe fn pc_fn(list: *mut c_void, slot: usize) -> usize {
    unsafe { *(*(list as *const *const usize)).add(slot) }
}

static DISPATCH_LOG: AtomicUsize = AtomicUsize::new(0);

unsafe extern "system" fn cl_dispatch_x(this: *mut c_void, x: u32, y: u32, z: u32, flags: u32) {
    if flags & DISPATCH_X_INVALID_FLAGS != 0 && once(&DISPATCH_LOG, 8) {
        record("d3d12_xs", "console_slots", &format!("DispatchX: Flags {flags:#x} contains invalid bits (console debug validation); dispatched as the release UMD does"));
    }
    let f: unsafe extern "system" fn(*mut c_void, u32, u32, u32) = unsafe { core::mem::transmute(pc_fn(this, PC_SLOT_DISPATCH)) };
    unsafe { crate::gws_dispatch::dispatch_with(this, x, y, z, f) }
}

#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Debug)]
pub struct IbView {
    pub va: u64,
    pub size: u32,
    pub format: u32,
}

static IB_STATE: Mutex<Option<HashMap<usize, Option<IbView>>>> = Mutex::new(None);
static REAL_SET_IB: AtomicUsize = AtomicUsize::new(0);

fn note_ib(list: usize, v: Option<IbView>) {
    IB_STATE.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).insert(list, v);
}

pub fn bound_ib(list: usize) -> Option<IbView> {
    IB_STATE.lock().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(|m| m.get(&list).copied()).flatten()
}

unsafe extern "system" fn ia_set_index_buffer_tracked(this: *mut c_void, view: *const IbView) {
    note_ib(this as usize, if view.is_null() { None } else { Some(unsafe { *view }) });
    let f: unsafe extern "system" fn(*mut c_void, *const IbView) = unsafe { core::mem::transmute(REAL_SET_IB.load(Ordering::Relaxed)) };
    unsafe { f(this, view) }
}

pub fn index_stride(format: u32) -> Option<u32> {
    match format {
        DXGI_FORMAT_R16_UINT => Some(2),
        DXGI_FORMAT_R32_UINT => Some(4),
        _ => None,
    }
}

pub fn draw_indexed_x_view(va: u64, format: u32, count: u32) -> Option<IbView> {
    let stride = index_stride(format)?;
    if count == 0 || va == 0 || va % stride as u64 != 0 {
        return None;
    }
    Some(IbView { va, size: count.checked_mul(stride)?, format })
}

static DRAWX_LOG: AtomicUsize = AtomicUsize::new(0);

pub fn draw_indexed_x_call(count: u32, instance_count: u32) -> (u32, u32, u32, i32, u32) {
    (count, instance_count, 0, 0, 0)
}

unsafe extern "system" fn cl_draw_indexed_x(this: *mut c_void, va: u64, format: u32, count: u32, instance_count: u32) {
    let Some(view) = draw_indexed_x_view(va, format, count) else {
        if once(&DRAWX_LOG, 8) {
            record("d3d12_xs", "console_slots", &format!("DrawIndexedX va={va:#x} format={format} count={count}: fails console validation (R16/R32 index, non-zero count, aligned address); not drawn"));
        }
        return;
    };
    let real = REAL_SET_IB.load(Ordering::Relaxed);
    let set_ib: unsafe extern "system" fn(*mut c_void, *const IbView) = unsafe { core::mem::transmute(if real != 0 { real } else { pc_fn(this, PC_SLOT_IA_SET_INDEX_BUFFER) }) };
    let draw: unsafe extern "system" fn(*mut c_void, u32, u32, u32, i32, u32) = unsafe { core::mem::transmute(pc_fn(this, PC_SLOT_DRAW_INDEXED)) };
    let prev = bound_ib(this as usize);
    unsafe {
        set_ib(this, &view);
        let (a, b, c, d, e) = draw_indexed_x_call(count, instance_count);
        draw(this, a, b, c, d, e);
        match prev {
            Some(p) => set_ib(this, &p),
            None => set_ib(this, core::ptr::null()),
        }
    }
}

unsafe extern "system" fn cl_set_predication_buffer(this: *mut c_void, desc: *const crate::cl_fw::PredicationBufferDesc) {
    unsafe { crate::cl_fw::set_predication_buffer(this, desc) }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WbiParam {
    pub dest: u64,
    pub value: u32,
}

pub fn wbi_params(va: u64, words: &[u32]) -> Vec<WbiParam> {
    words.iter().enumerate().map(|(i, &w)| WbiParam { dest: va + 4 * i as u64, value: w }).collect()
}

static WBI_LOG: AtomicUsize = AtomicUsize::new(0);

pub(crate) unsafe fn write_immediate(list: *mut c_void, va: u64, words: &[u32], mode: u32, what: &str) {
    let align = 4 * words.len() as u64;
    if va == 0 || va & GRAPHICS_VA_HIGH_MASK != 0 || va % align != 0 {
        if once(&WBI_LOG, 8) {
            record("d3d12_xs", "console_slots", &format!("{what}: destination {va:#x} must be {align}-byte aligned (console validation)"));
        }
        return;
    }
    let Some((e, off)) = crate::gpu_va::lookup(va) else {
        if once(&WBI_LOG, 8) {
            record("d3d12_xs", "console_slots", &format!("{what}: destination {va:#x} is not in graphics memory owned by a tracked resource"));
        }
        return;
    };
    if !e.buffer || off + align > e.len {
        if once(&WBI_LOG, 8) {
            record("d3d12_xs", "console_slots", &format!("{what}: destination {va:#x} is not inside a buffer; texture memory has no PC byte address"));
        }
        return;
    }
    let st = crate::gpu_va::state(e.res);
    let Some((db, _)) = copy_plan(st, STATE_COPY_SOURCE, st == STATE_GENERIC_READ) else {
        if once(&WBI_LOG, 8) {
            record("d3d12_xs", "console_slots", &format!("{what}: destination {va:#x} is an upload-heap buffer, which the GPU cannot write on PC"));
        }
        return;
    };
    let ty = unsafe { crate::fill::list_type(list) };
    if let Some((a, _)) = db {
        if !crate::fill::state_legal_on(ty, a) {
            if once(&WBI_LOG, 8) {
                record("d3d12_xs", "console_slots", &format!("{what}: buffer state {a:#x} cannot be transitioned on a type-{ty} list"));
            }
            return;
        }
    }
    let params = wbi_params(va, words);
    let modes = vec![mode; params.len()];
    unsafe {
        if let Some((a, b)) = db { crate::fill::barrier_pub(list, e.res, a, b); }
        let f: unsafe extern "system" fn(*mut c_void, u32, *const WbiParam, *const u32) = core::mem::transmute(pc_fn(list, PC_SLOT_WRITE_BUFFER_IMMEDIATE));
        f(list, params.len() as u32, params.as_ptr(), modes.as_ptr());
        if let Some((a, b)) = db { crate::fill::barrier_pub(list, e.res, b, a); }
    }
    crate::placed::note_gpu_write(e.res as *mut c_void);
    let value = words.iter().rev().fold(0u64, |acc, &w| (acc << 32) | w as u64);
    crate::gpu_wait::note(list, crate::gpu_wait::Op::Write { addr: va, value });
}

unsafe extern "system" fn cl_write32_top(this: *mut c_void, va: u64, value: u32, _flags: u32) {
    unsafe { write_immediate(this, va, &[value], WBI_MODE_MARKER_IN, "Write32BitValueTopOfPipeX") }
}

pub fn write64_top_is_timestamp(flags: u32) -> bool {
    flags & (WRITE_TOP_FLAG_GPU_TIMESTAMP | WRITE_TOP_FLAG_SYSTEM_TIMESTAMP) != 0
}

unsafe extern "system" fn cl_write64_top(this: *mut c_void, va: u64, value: u64, flags: u32) {
    if write64_top_is_timestamp(flags) {
        unsafe { crate::timestamp::write(this, false, va, flags, 0, 0) };
        return;
    }
    unsafe { write_immediate(this, va, &[value as u32, (value >> 32) as u32], WBI_MODE_MARKER_IN, "Write64BitValueTopOfPipeX") }
}

pub const SHADER_STAGE_PIXEL: u32 = 4;
pub const SHADER_STAGE_COMPUTE: u32 = 5;

unsafe extern "system" fn cl_write32_bottom_of_shader(this: *mut c_void, va: u64, value: u32, stage: u32) {
    if stage != SHADER_STAGE_PIXEL && stage != SHADER_STAGE_COMPUTE {
        if once(&WBI_LOG, 8) {
            record("d3d12_xs", "console_slots", &format!("Write32BitValueBottomOfShaderX: stage {stage} is not pixel (4) or compute (5) (console validation)"));
        }
        return;
    }
    unsafe { write_immediate(this, va, &[value], WBI_MODE_MARKER_OUT, "Write32BitValueBottomOfShaderX") }
}

pub const PREFETCH_VALID_FLAGS: u32 = 0;

pub fn prefetch_ok(size: u64, flags: u32, valid_flags: u32) -> bool {
    size != 0 && size < PREFETCH_MAX_BYTES as u64 && size % 4 == 0 && flags & !valid_flags == 0
}

static PREFETCH_LOG: AtomicUsize = AtomicUsize::new(0);

unsafe extern "system" fn cl_prefetch_memory(_this: *mut c_void, va: u64, size: u64, flags: u32) {
    if !prefetch_ok(size, flags, PREFETCH_VALID_FLAGS) && once(&PREFETCH_LOG, 8) {
        record("d3d12_xs", "console_slots", &format!("PrefetchMemoryX va={va:#x} size={size:#x} flags={flags:#x}: fails console validation"));
    }
}

pub fn snapshot(vt: &[AtomicUsize]) -> Vec<usize> {
    vt.iter().map(|s| s.load(Ordering::Relaxed)).collect()
}

pub fn snapshot_slice(vt: &[usize]) -> Vec<usize> {
    vt.to_vec()
}

fn device_plan() -> Vec<(usize, usize)> {
    let mut p = Vec::new();
    for s in DEV_S_OK { p.push((s, dev_s_ok as usize)); }
    for s in DEV_VOID { p.push((s, dev_void as usize)); }
    for s in DEV_ALL_ONES { p.push((s, dev_all_ones as usize)); }
    for s in DEV_NOTIMPL { p.push((s, dev_notimpl as usize)); }
    p.push((DEV_STORE3, dev_store3 as usize));
    p.extend(crate::device_console_fix::plan());
    p
}

pub fn install_device(v: &mut [usize], defaults: &[usize]) -> usize {
    let mut n = 0;
    for (slot, f) in device_plan() {
        if slot < v.len() && slot < defaults.len() && v[slot] == defaults[slot] {
            v[slot] = f;
            n += 1;
        }
    }
    n
}

pub unsafe fn install_cmdlist(vt: &[AtomicUsize], real_vt: *const usize, defaults: &[usize]) -> usize {
    let mut plan: Vec<(usize, usize)> = Vec::new();
    for s in CL_VOID { plan.push((s, cl_void as usize)); }
    plan.push((CL_COPY_MEMORY, cl_copy_memory as usize));
    plan.push((CL_FILL32, cl_fill32 as usize));
    plan.push((CL_FILL64, cl_fill64 as usize));
    plan.push((CL_FILL128, cl_fill128 as usize));
    plan.push((CL_NOP, cl_nop as usize));
    plan.push((CL_DISPATCH_X, cl_dispatch_x as usize));
    plan.push((CL_DRAW_INDEXED_X, cl_draw_indexed_x as usize));
    plan.push((CL_SET_PREDICATION_BUFFER, cl_set_predication_buffer as usize));
    plan.push((CL_WRITE32_TOP, cl_write32_top as usize));
    plan.push((CL_WRITE64_TOP, cl_write64_top as usize));
    plan.push((CL_WRITE32_BOTTOM_OF_SHADER, cl_write32_bottom_of_shader as usize));
    plan.push((CL_PREFETCH_MEMORY, cl_prefetch_memory as usize));
    if XBOX_SLOT_IA_SET_INDEX_BUFFER < vt.len() {
        let prev = vt[XBOX_SLOT_IA_SET_INDEX_BUFFER].load(Ordering::Relaxed);
        if prev != 0 && prev != ia_set_index_buffer_tracked as usize {
            REAL_SET_IB.store(prev, Ordering::Relaxed);
            vt[XBOX_SLOT_IA_SET_INDEX_BUFFER].store(ia_set_index_buffer_tracked as usize, Ordering::Relaxed);
        }
    }
    if !real_vt.is_null() {
        for s in CL_PC_PASSTHROUGH {
            if let Some(pc) = pc_slot_for_list1(s) {
                plan.push((s, *real_vt.add(pc)));
            }
        }
    }
    let mut n = 0;
    for (slot, f) in plan {
        if slot < vt.len() && slot < defaults.len() && vt[slot].load(Ordering::Relaxed) == defaults[slot] {
            vt[slot].store(f, Ordering::Relaxed);
            n += 1;
        }
    }
    record("d3d12_xs", "console_slots", &format!("command-list slots filled from the console UMD map: {n}"));
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draw_indexed_x_validates_like_the_debug_umd() {
        assert_eq!(draw_indexed_x_view(0x1000, DXGI_FORMAT_R16_UINT, 6), Some(IbView { va: 0x1000, size: 12, format: 57 }));
        assert_eq!(draw_indexed_x_view(0x1000, DXGI_FORMAT_R32_UINT, 3), Some(IbView { va: 0x1000, size: 12, format: 42 }));
        assert_eq!(draw_indexed_x_view(0x1001, DXGI_FORMAT_R16_UINT, 6), None);
        assert_eq!(draw_indexed_x_view(0x1002, DXGI_FORMAT_R32_UINT, 6), None);
        assert_eq!(draw_indexed_x_view(0x1000, DXGI_FORMAT_R16_UINT, 0), None);
        assert_eq!(draw_indexed_x_view(0, DXGI_FORMAT_R16_UINT, 3), None);
        assert_eq!(draw_indexed_x_view(0x1000, 28, 3), None);
        assert_eq!(core::mem::size_of::<IbView>(), 16);
        assert_eq!(draw_indexed_x_call(36, 4), (36, 4, 0, 0, 0));
        assert_eq!(draw_indexed_x_call(6, 0), (6, 0, 0, 0, 0));
    }

    #[test]
    fn index_buffer_tracking_is_per_list() {
        note_ib(0x77, Some(IbView { va: 8, size: 4, format: 42 }));
        note_ib(0x78, None);
        assert_eq!(bound_ib(0x77), Some(IbView { va: 8, size: 4, format: 42 }));
        assert_eq!(bound_ib(0x78), None);
        assert_eq!(bound_ib(0x79), None);
    }

    #[test]
    fn value_writes_split_into_dwords_in_memory_order() {
        assert_eq!(wbi_params(0x100, &[0xdead_beef]), vec![WbiParam { dest: 0x100, value: 0xdead_beef }]);
        let v: u64 = 0x1122_3344_5566_7788;
        assert_eq!(wbi_params(0x200, &[v as u32, (v >> 32) as u32]), vec![WbiParam { dest: 0x200, value: 0x5566_7788 }, WbiParam { dest: 0x204, value: 0x1122_3344 }]);
        assert_eq!(core::mem::size_of::<WbiParam>(), 16);
        assert!(write64_top_is_timestamp(WRITE_TOP_FLAG_GPU_TIMESTAMP));
        assert!(write64_top_is_timestamp(WRITE_TOP_FLAG_SYSTEM_TIMESTAMP));
        assert!(!write64_top_is_timestamp(0x30));
    }

    #[test]
    fn prefetch_and_dispatch_flag_rules_match_console() {
        assert!(prefetch_ok(4, 0, PREFETCH_VALID_FLAGS));
        assert!(prefetch_ok(0x1f_fffc, 0, PREFETCH_VALID_FLAGS));
        assert!(!prefetch_ok(0, 0, PREFETCH_VALID_FLAGS));
        assert!(!prefetch_ok(0x20_0000, 0, PREFETCH_VALID_FLAGS));
        assert!(!prefetch_ok(6, 0, PREFETCH_VALID_FLAGS));
        assert!(!prefetch_ok(4, 1, PREFETCH_VALID_FLAGS));
        assert_eq!(0x0efc & DISPATCH_X_INVALID_FLAGS, 0);
        assert_ne!(0x1 & DISPATCH_X_INVALID_FLAGS, 0);
        assert_ne!(0x100 & DISPATCH_X_INVALID_FLAGS, 0);
    }

    #[test]
    fn fill_and_nop_validation_matches_console() {
        assert!(fill_size_ok(4, 4));
        assert!(!fill_size_ok(0, 4));
        assert!(!fill_size_ok(6, 4));
        assert!(fill_size_ok(16, 8));
        assert!(!fill_size_ok(12, 8));
        assert!(!fill_size_ok(24, 16));
        assert!(nop_size_ok(4));
        assert!(nop_size_ok(0xfffc));
        assert!(!nop_size_ok(0));
        assert!(!nop_size_ok(6));
        assert!(!nop_size_ok(0x1_0000));
    }

    #[test]
    fn fill128_repeats_the_value_bytes_in_memory_order() {
        let v: [u8; 16] = core::array::from_fn(|i| i as u8 + 1);
        assert_eq!(fill128_pattern(&v), v);
    }

    #[test]
    fn copy_plan_transitions_only_what_it_must() {
        assert_eq!(copy_plan(0x400, 0x800, false), Some((None, None)));
        assert_eq!(copy_plan(0x8, 0x8, false), Some((Some((0x8, 0x400)), Some((0x8, 0x800)))));
        assert_eq!(copy_plan(0x400, 0xac3, false), Some((None, None)));
        assert_eq!(copy_plan(0xac3, 0x800, true), None);
    }

    #[test]
    fn list1_block_maps_to_pc_list1() {
        assert_eq!(pc_slot_for_list1(104), Some(60));
        assert_eq!(pc_slot_for_list1(106), Some(62));
        assert_eq!(pc_slot_for_list1(107), Some(63));
        assert_eq!(pc_slot_for_list1(108), Some(64));
        assert_eq!(pc_slot_for_list1(109), Some(65));
        assert_eq!(pc_slot_for_list1(103), None);
        assert_eq!(pc_slot_for_list1(110), None);
    }

    #[test]
    fn device_trivial_bodies_match_release_umd() {
        unsafe {
            let t = 0x1234usize as *mut c_void;
            assert_eq!(dev_s_ok(t), 0);
            assert_eq!(dev_all_ones(t), 0xffff_ffff);
            assert_eq!(dev_notimpl(t), 0x8000_4001u32 as i32);
            dev_void(t);
        }
    }

    #[test]
    fn device_slot_50_stores_three_values_per_device() {
        unsafe {
            dev_store3(0x10 as *mut c_void, 1, 2, 3);
            dev_store3(0x20 as *mut c_void, 4, 5, 6);
            dev_store3(0x10 as *mut c_void, 7, 8, 9);
        }
        assert_eq!(device_state(0x10), Some([7, 8, 9]));
        assert_eq!(device_state(0x20), Some([4, 5, 6]));
        assert_eq!(device_state(0x30), None);
    }

    #[test]
    fn install_device_only_replaces_default_slots() {
        let defaults: Vec<usize> = (0..DEVICE_SLOTS).map(|i| 1000 + i).collect();
        let mut v = defaults.clone();
        v[64] = 7;
        let n = install_device(&mut v, &defaults);
        assert_eq!(v[64], 7);
        assert_eq!(n, device_plan().len() - 1);
        assert_eq!(v[53], dev_s_ok as usize);
        assert_eq!(v[69], dev_notimpl as usize);
        assert_eq!(v[89], dev_notimpl as usize);
        assert_eq!(v[50], dev_store3 as usize);
        assert_eq!(v[52], 1052);
    }

    #[test]
    fn install_cmdlist_keeps_real_functions_and_passes_list1_through() {
        let real: Vec<usize> = (0..80).map(|i| 5000 + i).collect();
        let vt: Vec<AtomicUsize> = (0..128).map(|i| AtomicUsize::new(9000 + i)).collect();
        let defaults = snapshot(&vt);
        vt[107].store(1, Ordering::Relaxed);
        let n = unsafe { install_cmdlist(&vt, real.as_ptr(), &defaults) };
        let passthrough = CL_PC_PASSTHROUGH.iter().filter(|&&s| pc_slot_for_list1(s).is_some()).count();
        assert_eq!(n, CL_VOID.len() + 12 + passthrough - 1);
        assert_eq!(vt[47].load(Ordering::Relaxed), ia_set_index_buffer_tracked as usize);
        assert_eq!(REAL_SET_IB.load(Ordering::Relaxed), 9047);
        assert_eq!(vt[81].load(Ordering::Relaxed), cl_draw_indexed_x as usize);
        assert_eq!(vt[89].load(Ordering::Relaxed), cl_write64_top as usize);
        assert_eq!(vt[61].load(Ordering::Relaxed), cl_fill32 as usize);
        assert_eq!(vt[60].load(Ordering::Relaxed), cl_copy_memory as usize);
        assert_eq!(vt[106].load(Ordering::Relaxed), 5062);
        assert_eq!(vt[107].load(Ordering::Relaxed), 1);
        assert_eq!(vt[108].load(Ordering::Relaxed), 5064);
        assert_eq!(vt[87].load(Ordering::Relaxed), cl_void as usize);
        assert_eq!(vt[104].load(Ordering::Relaxed), 5060);
        assert_eq!(vt[105].load(Ordering::Relaxed), 5061);
        assert_eq!(vt[109].load(Ordering::Relaxed), 5065);
        assert_eq!(vt[110].load(Ordering::Relaxed), 9110);
    }
}
