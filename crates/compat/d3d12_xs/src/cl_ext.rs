use core::ffi::c_void;
use core::sync::atomic::{AtomicI32, AtomicUsize, Ordering};
use std::collections::HashMap;
use std::sync::Mutex;
use compat_common::record;

use crate::console_slots::{GRAPHICS_VA_HIGH_MASK, WBI_MODE_MARKER_OUT};
use crate::device_wrap::Guid;
use crate::gpu_wait::{self, Op};

pub const CL_WRITE32_BOTTOM_OF_PIPE: usize = 69;
pub const CL_WRITE64_BOTTOM_OF_PIPE: usize = 70;
pub const CL_WAIT32: usize = 71;
pub const CL_WAIT64: usize = 72;
pub const CL_FORCE_Z_ORDER: usize = 73;
pub const CL_EXECUTE_INDIRECT_BUNDLE: usize = 74;
pub const CL_SETUP_ORDERED_APPEND: usize = 75;
pub const CL_WRITE_GDS: usize = 76;
pub const CL_READ_GDS: usize = 77;
pub const CL_SET_HI_STENCIL: usize = 79;
pub const CL_SET_MSAA_PARAMETERS: usize = 80;
pub const CL_RECORD_D1: usize = 86;
pub const CL_SET_COMPUTE_LIMITS: usize = 96;
pub const CL_SET_COMPRESSION_STATE: usize = 97;

#[cfg(test)]
pub const SLOTS: [usize; 14] = [
    CL_WRITE32_BOTTOM_OF_PIPE, CL_WRITE64_BOTTOM_OF_PIPE,
    CL_WAIT32, CL_WAIT64, CL_FORCE_Z_ORDER, CL_EXECUTE_INDIRECT_BUNDLE, CL_SETUP_ORDERED_APPEND, CL_WRITE_GDS,
    CL_READ_GDS, CL_SET_HI_STENCIL, CL_SET_MSAA_PARAMETERS, CL_RECORD_D1, CL_SET_COMPUTE_LIMITS, CL_SET_COMPRESSION_STATE,
];

const LIST_TYPE_DIRECT: u32 = 0;
const LIST_TYPE_BUNDLE: u32 = 1;
const PC_SLOT_RESOURCE_BARRIER: usize = 26;
const PC_SLOT_SET_SAMPLE_POSITIONS: usize = 63;
const RES_SLOT_GET_DESC: usize = 10;
const RESOURCE_FLAG_ALLOW_RENDER_TARGET: u32 = 0x1;
const RESOURCE_FLAG_ALLOW_DEPTH_STENCIL: u32 = 0x2;
const FEATURE_OPTIONS2: u32 = 18;
const IID_DEVICE: Guid =
    Guid { d1: 0x1898_19F1, d2: 0x1DB6, d3: 0x4B57, d4: [0xBE, 0x54, 0x18, 0x21, 0x33, 0x9B, 0x85, 0xF7] };

pub const BOP_FLAG_GPU_TIMESTAMP: u32 = 0x1;
pub const BOP_FLAG_SYSTEM_TIMESTAMP: u32 = 0x2;
pub const BOP_FLAGS_INVALID: u32 = 0xffff_ff4c;
pub const BOP_FLAG_L2_HINTS: u32 = 0x30;
pub const BOP_FLUSH_VALID: u32 = 0x3800;

pub const MAX_BUNDLE_BYTES: u32 = 0x40_0000;
pub const GDS_LIMIT: u32 = 0x3f00;

pub const COMPRESSION_ENABLE_COLOR_OR_DEPTH: u32 = 0x1;
pub const COMPRESSION_ENABLE_STENCIL: u32 = 0x2;
pub const PRESERVE_VALID: u32 = 0x0fc0_0000;
pub const PRESERVE_COMPRESSED: u32 = 0x40_0000;
pub const PRESERVE_COMPRESSED_STENCIL: u32 = 0x80_0000;
pub const PRESERVE_EXPANDED: u32 = 0x400_0000;
pub const PRESERVE_EXPANDED_STENCIL: u32 = 0x800_0000;

pub const CS_LIMITS_CU_MASK: u32 = 0x3ff;
pub const CS_LIMITS_MAX_WAVES: u32 = 0x640;

fn once(c: &AtomicUsize, n: usize) -> bool {
    c.fetch_add(1, Ordering::Relaxed) < n
}

unsafe fn pc_fn(obj: *mut c_void, slot: usize) -> usize {
    unsafe { *(*(obj as *const *const usize)).add(slot) }
}

unsafe fn list_type(list: *mut c_void) -> u32 {
    unsafe { crate::fill::list_type(list) }
}

unsafe fn res_desc(res: *mut c_void) -> Option<[u8; 56]> {
    let address = res as usize;
    if address < 0x1_0000 || address % core::mem::align_of::<usize>() != 0
        || !crate::title_shadow::is_readable_range(address, core::mem::size_of::<usize>())
    {
        return None;
    }
    let vtable = unsafe { res.cast::<usize>().read() };
    if !crate::title_shadow::is_readable_range(vtable, (RES_SLOT_GET_DESC + 1) * core::mem::size_of::<usize>()) {
        return None;
    }
    let mut d = [0u8; 56];
    let f: unsafe extern "system" fn(*mut c_void, *mut u8) -> *mut u8 = unsafe { core::mem::transmute(pc_fn(res, RES_SLOT_GET_DESC)) };
    unsafe { f(res, d.as_mut_ptr()) };
    Some(d)
}

fn desc_flags(d: &[u8; 56]) -> u32 {
    u32::from_le_bytes(d[0x30..0x34].try_into().unwrap())
}

fn desc_format(d: &[u8; 56]) -> u32 {
    u32::from_le_bytes(d[0x20..0x24].try_into().unwrap())
}

fn desc_width(d: &[u8; 56]) -> u64 {
    u64::from_le_bytes(d[0x10..0x18].try_into().unwrap())
}

fn reject(c: &AtomicUsize, what: &str, why: &str) {
    if once(c, 8) {
        record("d3d12_xs", "cl_ext", &format!("{what}: {why} (console validation); not recorded"));
    }
}

pub fn bop_error(flush: u32, flags: u32, is64: bool, value: u64) -> Option<&'static str> {
    if flags & BOP_FLAGS_INVALID != 0 {
        return Some("invalid flags");
    }
    let ts = flags & (BOP_FLAG_GPU_TIMESTAMP | BOP_FLAG_SYSTEM_TIMESTAMP);
    if !is64 && ts != 0 {
        return Some("TIMESTAMP flags can only be used with Write64BitValueBottomOfPipeX");
    }
    if ts == BOP_FLAG_GPU_TIMESTAMP | BOP_FLAG_SYSTEM_TIMESTAMP {
        return Some("GPU_TIMESTAMP and SYSTEM_TIMESTAMP are exclusive");
    }
    if ts != 0 && value != 0 {
        return Some("Value must be 0 with a TIMESTAMP flag");
    }
    if flags & BOP_FLAG_L2_HINTS == BOP_FLAG_L2_HINTS {
        return Some("CACHE_L2_LRU and CACHE_L2_STREAM are exclusive");
    }
    if flush & !BOP_FLUSH_VALID != 0 {
        return Some("flush flags can only include BOP_TEXTURE_Ln_INVALIDATE and BOP_TEXTURE_L2_WRITEBACK");
    }
    None
}

static BOP_LOG: AtomicUsize = AtomicUsize::new(0);

unsafe extern "system" fn cl_write32_bottom_of_pipe(this: *mut c_void, va: u64, value: u32, flush: u32, flags: u32) {
    const W: &str = "Write32BitValueBottomOfPipeX";
    if unsafe { list_type(this) } != LIST_TYPE_DIRECT {
        return reject(&BOP_LOG, W, "pCommandList is not of type DIRECT");
    }
    if let Some(e) = bop_error(flush, flags, false, value as u64) {
        return reject(&BOP_LOG, W, e);
    }
    unsafe { crate::console_slots::write_immediate(this, va, &[value], WBI_MODE_MARKER_OUT, W) };
}

unsafe extern "system" fn cl_write64_bottom_of_pipe(this: *mut c_void, va: u64, value: u64, flush: u32, flags: u32) {
    unsafe { crate::bop64::write64(this, va, value, flush, flags) }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Bop64 {
    GpuTimestamp,
    SystemTimestamp,
    Value,
}

pub fn bop64_route(va: u64, flags: u32) -> Result<Bop64, &'static str> {
    if va == 0 || va & GRAPHICS_VA_HIGH_MASK != 0 || va % 8 != 0 {
        return Err("destination must be an 8-byte aligned graphics address");
    }
    Ok(if flags & BOP_FLAG_GPU_TIMESTAMP != 0 {
        Bop64::GpuTimestamp
    } else if flags & BOP_FLAG_SYSTEM_TIMESTAMP != 0 {
        Bop64::SystemTimestamp
    } else {
        Bop64::Value
    })
}

unsafe fn stall_list(list: *mut c_void) {
    let _ij = unsafe { crate::inject::scope(list, crate::inject::BARRIER) };
    let mut b = [0u8; 32];
    b[0..4].copy_from_slice(&2u32.to_le_bytes());
    let f: unsafe extern "system" fn(*mut c_void, u32, *const u8) = unsafe { core::mem::transmute(pc_fn(list, PC_SLOT_RESOURCE_BARRIER)) };
    unsafe { f(list, 1, b.as_ptr()) };
}

static WAIT_LOG: AtomicUsize = AtomicUsize::new(0);

unsafe fn wait_value(list: *mut c_void, va: u64, func: u32, value: u64, mask: u64, flags: u32, align: u64, what: &str) {
    if unsafe { list_type(list) } != LIST_TYPE_DIRECT {
        return reject(&WAIT_LOG, what, "pCommandList is not of type DIRECT");
    }
    if let Some(e) = gpu_wait::wait_args_error(va, func, flags, align) {
        return reject(&WAIT_LOG, what, e);
    }
    let op = Op::Wait { addr: va, func, value, mask };
    if gpu_wait::wait_reg_mem_function(func) == 0 || gpu_wait::locally_satisfied(list, op) || !unsafe { crate::list_split::split_for_wait(list, op) } {
        gpu_wait::note(list, op);
        unsafe { stall_list(list) };
    }
}

unsafe extern "system" fn cl_wait32(this: *mut c_void, va: u64, func: u32, value: u32, mask: u32, flags: u32) {
    unsafe { wait_value(this, va, func, value as u64, mask as u64, flags, 4, "Wait32BitValueX") }
}

unsafe extern "system" fn cl_wait64(this: *mut c_void, va: u64, func: u32, value: u64, flags: u32) {
    unsafe { wait_value(this, va, func, value, u64::MAX, flags, 8, "Wait64BitValueX") }
}

static ZORDER_LOG: AtomicUsize = AtomicUsize::new(0);

pub fn z_order_error(list_type: u32) -> Option<&'static str> {
    if list_type != LIST_TYPE_DIRECT { Some("pCommandList is not of type DIRECT") } else { None }
}

unsafe extern "system" fn cl_force_z_order(this: *mut c_void, enable: u32) {
    if let Some(e) = z_order_error(unsafe { list_type(this) }) {
        return reject(&ZORDER_LOG, "SetPixelShaderDepthForceZOrderX", e);
    }
    if once(&ZORDER_LOG, 2) {
        record("d3d12_xs", "cl_ext", &format!("SetPixelShaderDepthForceZOrderX({enable}): DB_RENDER_OVERRIDE.FORCE_SHADER_Z_ORDER; PC drivers always run depth in the order the DXIL shader declares (late for UAV/discard/depth export, early for [earlydepthstencil])"));
    }
}

pub fn bundle_error(bundle_width: Option<u64>, offset: u64, max_size: u32, size_buf: Option<u64>, size_off: u32, list_type: u32, pred_off: u32) -> Option<&'static str> {
    let Some(w) = bundle_width else { return Some("pBundleBuffer cannot be NULL") };
    if offset >= w {
        return Some("BundleBufferOffset is beyond the end of pBundleBuffer");
    }
    if offset % 4 != 0 {
        return Some("BundleBufferOffset must be a multiple of 4");
    }
    if max_size == 0 {
        return Some("MaxSize cannot be 0");
    }
    if max_size >= MAX_BUNDLE_BYTES {
        return Some("MaxSize cannot be 4 MB or larger");
    }
    match size_buf {
        Some(sw) => {
            if size_off as u64 + 4 > sw {
                return Some("SizeBufferOffset plus 4 is beyond the end of pSizeBuffer");
            }
            if size_off % 4 != 0 {
                return Some("SizeBufferOffset must be a multiple of 4");
            }
        }
        None if size_off != 0 => return Some("SizeBufferOffset must be 0 when pSizeBuffer is NULL"),
        None => {}
    }
    if list_type == LIST_TYPE_BUNDLE {
        return Some("can't invoke a bundle from a bundle");
    }
    if pred_off != 0 {
        return Some("PredicationBufferOffset must be zero");
    }
    None
}

static BUNDLE_LOG: AtomicUsize = AtomicUsize::new(0);

pub fn bundle_list_error(list_type: u32) -> Option<&'static str> {
    (list_type != LIST_TYPE_DIRECT).then_some("Cannot be performed on a compute command list.")
}

unsafe extern "system" fn cl_execute_indirect_bundle(this: *mut c_void, bundle: *mut c_void, offset: u64, max_size: u32, size_buf: *mut c_void, size_off: u32, pred_off: u32) {
    const W: &str = "ExecuteIndirectBundleX";
    let offset = offset as u32 as u64;
    let bw = unsafe { res_desc(bundle) }.map(|d| desc_width(&d));
    let sw = unsafe { res_desc(size_buf) }.map(|d| desc_width(&d));
    if (!bundle.is_null() && bw.is_none()) || (!size_buf.is_null() && sw.is_none()) {
        if once(&BUNDLE_LOG, 8) {
            record("d3d12_xs", "cl_ext", &format!("{W}: invalid resource arguments bundle={bundle:p} size_buffer={size_buf:p}"));
        }
        return;
    }
    let ty = unsafe { list_type(this) };
    // The retail UMD always writes zero. The debug UMD rejects a nonzero argument.
    let pred_off = if crate::cmdlist_x::debug_umd(this) { pred_off } else { 0 };
    if let Some(e) = bundle_error(bw, offset, max_size, sw, size_off, ty, pred_off).or_else(|| bundle_list_error(ty)) {
        return reject(&BUNDLE_LOG, W, e);
    }
    unsafe { crate::indirect_bundle::execute(this, bundle, bw.unwrap_or(0), offset, max_size, size_buf, size_off) };
}

pub fn ordered_append_error(index: u32, gds_dword: u32, stage: u32, reserved: u32) -> Option<&'static str> {
    if index >= 8 {
        return Some("CounterIndex should be in the [0..7] range");
    }
    if gds_dword > GDS_LIMIT {
        return Some("GdsAddressOfTheCounterDwords is out of range");
    }
    if stage >= 3 {
        return Some("ShaderStage should be in the [0..2] range");
    }
    if reserved > 3 {
        return Some("Reserved should be in the [0..3] range");
    }
    None
}

pub fn gds_error(offset: u32, count: u32, va: u64, flags: u32, read: bool) -> Option<&'static str> {
    if offset.wrapping_add(count) > GDS_LIMIT {
        return Some("offset and size exceed the GDS range");
    }
    if va == 0 {
        return Some("pCounterValues must be non-null");
    }
    if va % 4 != 0 {
        return Some("pCounterValues should be 4 bytes aligned");
    }
    let mode = flags & 7;
    let ok = mode == 0 || if read { (1..=2).contains(&mode) } else { (3..=4).contains(&mode) };
    if !ok {
        return Some("unsupported Flags value");
    }
    None
}

static GDS_LOG: AtomicUsize = AtomicUsize::new(0);
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OrderedAppend {
    pub cntl: u32,
    pub gds_dword: u32,
    pub stage: u32,
    pub wave_release: u32,
}

unsafe extern "system" fn cl_setup_ordered_append(this: *mut c_void, index: u32, cntl: u32, gds_dword: u32, stage: u32, wave_release: u32, reserved: u32) {
    const W: &str = "SetupOrderedAppendCounterX";
    if let Some(e) = ordered_append_error(index, gds_dword, stage, reserved) {
        return reject(&GDS_LOG, W, e);
    }
    let oa = OrderedAppend { cntl, gds_dword, stage, wave_release };
    if let Err(why) = unsafe { crate::gds::write_ordered_append(this, index, gds_dword, cntl, stage, wave_release) } {
        return reject(&GDS_LOG, W, why);
    }
    if once(&GDS_LOG, 4) {
        record("d3d12_xs", "cl_ext", &format!("{W} counter={index} {oa:x?}: written to the GDS ordered-append table (gds_abi TB_GDS_OA_*); ds_ordered_count lowers to atomics on the counter dwords, so allocations are unique but not handed out in wave-launch order"));
    }
}

unsafe extern "system" fn cl_write_gds(this: *mut c_void, offset: u32, count: u32, va: u64, flags: u32) {
    if let Some(e) = gds_error(offset, count, va, flags, false) {
        return reject(&GDS_LOG, "WriteGDSX", e);
    }
    unsafe { crate::gds::copy(this, true, offset, count, va, "WriteGDSX") };
}

unsafe extern "system" fn cl_read_gds(this: *mut c_void, offset: u32, count: u32, va: u64, flags: u32) {
    if let Some(e) = gds_error(offset, count, va, flags, true) {
        return reject(&GDS_LOG, "ReadGDSX", e);
    }
    unsafe { crate::gds::copy(this, false, offset, count, va, "ReadGDSX") };
}

pub fn hi_stencil_error(res_flags: Option<u32>, has_control: bool) -> Option<&'static str> {
    if !has_control {
        return Some(if res_flags.is_some() { "pResource must be set to NULL when pControl is NULL" } else { "pControl is NULL; the console UMD reads it unconditionally (access violation on console)" });
    }
    match res_flags {
        Some(f) if f & RESOURCE_FLAG_ALLOW_DEPTH_STENCIL == 0 => Some("SetHiStencilStateX can only be called on a depth/stencil resource"),
        _ => None,
    }
}

static HIS_LOG: AtomicUsize = AtomicUsize::new(0);

unsafe extern "system" fn cl_set_hi_stencil(_this: *mut c_void, res: *mut c_void, ctrl: *const [u32; 2]) {
    let flags = unsafe { res_desc(res) }.map(|d| desc_flags(&d));
    if !res.is_null() && flags.is_none() {
        return reject(&HIS_LOG, "SetHiStencilStateX", "invalid resource address");
    }
    let has_control = crate::title_shadow::is_readable_range(ctrl as usize, 8);
    if let Some(e) = hi_stencil_error(flags, has_control) {
        return reject(&HIS_LOG, "SetHiStencilStateX", e);
    }
    let c = unsafe { ctrl.read_unaligned() };
    if once(&HIS_LOG, 2) {
        record("d3d12_xs", "cl_ext", &format!("SetHiStencilStateX res={res:p} state0={:?} state1={:?}: HTILE stencil-result compares only let the hardware cull tiles that would fail anyway (conservative); PC drivers keep their own HiS",
            crate::cmdlist_x::decode_hi_stencil(c[0]), crate::cmdlist_x::decode_hi_stencil(c[1])));
    }
}

pub fn sample_positions_error(pos: &[u8; 128], config: Option<[u32; 2]>) -> Option<&'static str> {
    if pos.iter().any(|&b| !(-8..=7).contains(&(b as i8))) {
        return Some("Invalid value encountered in D3D11X_MSAA_SAMPLE_POSITIONS. Values should be signed integers in [-8..7] range.");
    }
    if let Some(c) = config {
        if c[1] & 0x22000 != 0 {
            return Some("Reserved fields must be zero");
        }
        if c[0] & 0xc00 == 0xc00 {
            return Some("DetailToExposedMode can't take the value of 3");
        }
    }
    None
}

pub fn config_samples(config: [u32; 2]) -> u32 {
    1 << (config[0] & 7)
}

pub fn pc_sample_positions(pos: &[u8; 128], samples: u32, pixels: u32) -> Vec<[i8; 2]> {
    let mut v = Vec::with_capacity((samples * pixels) as usize);
    for p in 0..pixels as usize {
        for s in 0..samples as usize {
            let o = 32 * p + 2 * s;
            v.push([pos[o] as i8, pos[o + 1] as i8]);
        }
    }
    v
}

pub fn masked_sample_positions(pos: &[u8; 128]) -> [u8; 128] {
    core::array::from_fn(|i| (((pos[i] << 4) as i8) >> 4) as u8)
}

pub fn quad_is_uniform(pos: &[u8; 128], samples: u32) -> bool {
    let n = 2 * samples as usize;
    (1..4).all(|p| pos[32 * p..32 * p + n] == pos[..n])
}

static SAMPLES: Mutex<Option<HashMap<usize, u32>>> = Mutex::new(None);
static SPS_TIER: AtomicI32 = AtomicI32::new(-1);
static MSAA_LOG: AtomicUsize = AtomicUsize::new(0);

unsafe fn sample_positions_tier(list: *mut c_void) -> i32 {
    let t = SPS_TIER.load(Ordering::Relaxed);
    if t >= 0 {
        return t;
    }
    let get: unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> i32 = unsafe { core::mem::transmute(pc_fn(list, 7)) };
    let mut dev: *mut c_void = core::ptr::null_mut();
    let mut tier = 0;
    if unsafe { get(list, &IID_DEVICE, &mut dev) } >= 0 && !dev.is_null() {
        let check: unsafe extern "system" fn(*mut c_void, u32, *mut c_void, u32) -> i32 = unsafe { core::mem::transmute(pc_fn(dev, 13)) };
        let mut o = [0u32; 2];
        if unsafe { check(dev, FEATURE_OPTIONS2, o.as_mut_ptr() as *mut c_void, 8) } >= 0 {
            tier = o[1] as i32;
        }
        let release: unsafe extern "system" fn(*mut c_void) -> u32 = unsafe { core::mem::transmute(pc_fn(dev, 2)) };
        unsafe { release(dev) };
    }
    SPS_TIER.store(tier, Ordering::Relaxed);
    tier
}

unsafe extern "system" fn cl_set_msaa_parameters(this: *mut c_void, pos: *const [u8; 128], centroid: *const [u8; 16], config: *const [u32; 2]) {
    const W: &str = "RSSetMSAAParametersX";
    let cfg = if config.is_null() { None } else { Some(unsafe { *config }) };
    let zero = [0u8; 128];
    let raw = if pos.is_null() { &zero } else { unsafe { &*pos } };
    if let Some(e) = sample_positions_error(raw, cfg) {
        if once(&MSAA_LOG, 8) {
            record("d3d12_xs", "cl_ext", &format!("{W}: {e} (xgs12_d message); applied with the register nibble masking as xgs12 does"));
        }
    }
    let masked = masked_sample_positions(raw);
    let p = &masked;
    if unsafe { list_type(this) } == LIST_TYPE_BUNDLE {
        return reject(&MSAA_LOG, W, "This API can't be used on a draw bundle list; the host forbids SetSamplePositions in bundles");
    }
    let key = crate::list_split::origin_of(this);
    let samples = {
        let mut g = SAMPLES.lock().unwrap_or_else(|e| e.into_inner());
        let m = g.get_or_insert_with(HashMap::new);
        if let Some(c) = cfg {
            m.insert(key, config_samples(c));
        }
        m.get(&key).copied()
    };
    if !centroid.is_null() {
        unsafe { crate::centroid_bind::on_msaa(this, Some(*centroid), None) };
    }
    if pos.is_null() {
        return;
    }
    let Some(n) = samples.filter(|&n| (2..=16).contains(&n)) else {
        if once(&MSAA_LOG, 8) {
            record("d3d12_xs", "cl_ext", &format!("{W}: sample positions without a known MSAA sample count for this list; positions not applied"));
        }
        return;
    };
    let tier = unsafe { sample_positions_tier(this) };
    let pixels = if tier >= 2 { 4 } else if tier == 1 && quad_is_uniform(p, n) { 1 } else { 0 };
    if pixels == 0 {
        if once(&MSAA_LOG, 8) {
            record("d3d12_xs", "cl_ext", &format!("{W}: host programmable sample positions tier {tier} cannot express these positions; not applied"));
        }
        return;
    }
    let v = pc_sample_positions(p, n, pixels);
    let f: unsafe extern "system" fn(*mut c_void, u32, u32, *const [i8; 2]) = unsafe { core::mem::transmute(pc_fn(this, PC_SLOT_SET_SAMPLE_POSITIONS)) };
    unsafe { f(this, n, pixels, v.as_ptr()) };
    unsafe { crate::centroid_bind::on_msaa(this, None, Some((*p, n))) };
}

pub fn compute_limits_error(d: &[u32; 6]) -> Option<&'static str> {
    if d[0] & !CS_LIMITS_CU_MASK != 0 {
        return Some("DisableFlags is invalid");
    }
    if d[0] == CS_LIMITS_CU_MASK {
        return Some("DisableFlags can't disable all CUs");
    }
    if d[1] > CS_LIMITS_MAX_WAVES {
        return Some("MaxWaves is invalid");
    }
    if d[3] > CS_LIMITS_MAX_WAVES {
        return Some("MaxWavesForCuLocking is invalid");
    }
    if d[2] >= 0x10 {
        return Some("MaxThreadgroupsPerCu is invalid");
    }
    if d[4] > 8 {
        return Some("NumThreadgroupsWalkedPerCu is invalid");
    }
    if d[5] > 3 {
        return Some("SimdWalkAlgorithm is invalid");
    }
    None
}

static CSL_LOG: AtomicUsize = AtomicUsize::new(0);

unsafe extern "system" fn cl_set_compute_limits(this: *mut c_void, desc: *const [u32; 6]) {
    const W: &str = "SetComputeShaderLimitsX";
    if unsafe { list_type(this) } == LIST_TYPE_BUNDLE {
        return reject(&CSL_LOG, W, "cannot be used in a bundle");
    }
    let d = if desc.is_null() { [0u32; 6] } else { unsafe { *desc } };
    if let Some(e) = compute_limits_error(&d) {
        return reject(&CSL_LOG, W, e);
    }
    if once(&CSL_LOG, 2) {
        record("d3d12_xs", "cl_ext", &format!("{W} {d:x?}: COMPUTE_STATIC_THREAD_MGMT/RESOURCE_LIMITS only restrict which CUs and how many waves run; results are unchanged and the PC scheduler is used"));
    }
}

pub fn has_stencil(format: u32) -> bool {
    matches!(format, 19 | 20 | 44 | 45 | 46 | 47)
}

pub fn compression_error(cflags: u32, pflags: u32, res: Option<(u32, u32)>, list_type: u32) -> Option<&'static str> {
    if list_type == LIST_TYPE_BUNDLE {
        return Some("cannot be recorded into a draw bundle");
    }
    if cflags & !3 != 0 {
        return Some("invalid CompressionStateFlags");
    }
    if pflags & !PRESERVE_VALID != 0 {
        return Some("invalid PreserveFlags");
    }
    let Some((rflags, format)) = res else { return Some("the given resource does not support compression") };
    let depth = rflags & RESOURCE_FLAG_ALLOW_DEPTH_STENCIL != 0;
    let color = rflags & RESOURCE_FLAG_ALLOW_RENDER_TARGET != 0;
    if !depth && !color {
        return Some("the given resource does not support compression");
    }
    let enable = cflags & COMPRESSION_ENABLE_COLOR_OR_DEPTH != 0;
    if pflags & PRESERVE_EXPANDED != 0 && enable {
        return Some("PRESERVE_EXPANDED cannot be specified with ENABLE");
    }
    if pflags & PRESERVE_COMPRESSED != 0 && !enable {
        return Some("PRESERVE_COMPRESSED cannot be specified without ENABLE");
    }
    if color && !depth {
        if pflags & (PRESERVE_COMPRESSED_STENCIL | PRESERVE_EXPANDED_STENCIL) != 0 {
            return Some("PRESERVE_*_STENCIL cannot be specified with a render target");
        }
        return None;
    }
    let st = cflags & COMPRESSION_ENABLE_STENCIL != 0;
    if pflags & PRESERVE_EXPANDED_STENCIL != 0 && st {
        return Some("PRESERVE_EXPANDED_STENCIL cannot be specified with ENABLE_STENCIL");
    }
    if pflags & PRESERVE_COMPRESSED_STENCIL != 0 && !st {
        return Some("PRESERVE_COMPRESSED_STENCIL cannot be specified without ENABLE_STENCIL");
    }
    if !has_stencil(format) && (st || pflags & (PRESERVE_COMPRESSED_STENCIL | PRESERVE_EXPANDED_STENCIL) != 0) {
        return Some("stencil flags on a resource that does not support stencil compression");
    }
    None
}

static COMP_LOG: AtomicUsize = AtomicUsize::new(0);

unsafe extern "system" fn cl_set_compression_state(this: *mut c_void, res: *mut c_void, cflags: u32, pflags: u32) {
    const W: &str = "SetResourceCompressionStateX";
    let r = unsafe { res_desc(res) }.filter(|d| u32::from_le_bytes(d[0..4].try_into().unwrap()) != 1).map(|d| (desc_flags(&d), desc_format(&d)));
    if let Some(e) = compression_error(cflags, pflags, r, unsafe { list_type(this) }) {
        return reject(&COMP_LOG, W, e);
    }
    if once(&COMP_LOG, 4) {
        record("d3d12_xs", "cl_ext", &format!("{W} res={res:p} compression={cflags:#x} preserve={pflags:#x}: PC keeps DCC/HTILE internal and always preserves texel contents, which every preserve mode allows"));
    }
}

pub fn plan() -> Vec<(usize, usize)> {
    vec![
        (CL_WRITE32_BOTTOM_OF_PIPE, cl_write32_bottom_of_pipe as usize),
        (CL_WRITE64_BOTTOM_OF_PIPE, cl_write64_bottom_of_pipe as usize),
        (CL_WAIT32, cl_wait32 as usize),
        (CL_WAIT64, cl_wait64 as usize),
        (CL_FORCE_Z_ORDER, cl_force_z_order as usize),
        (CL_EXECUTE_INDIRECT_BUNDLE, cl_execute_indirect_bundle as usize),
        (CL_SETUP_ORDERED_APPEND, cl_setup_ordered_append as usize),
        (CL_WRITE_GDS, cl_write_gds as usize),
        (CL_READ_GDS, cl_read_gds as usize),
        (CL_SET_HI_STENCIL, cl_set_hi_stencil as usize),
        (CL_SET_MSAA_PARAMETERS, cl_set_msaa_parameters as usize),
        (CL_RECORD_D1, crate::cl_fw::advance_predication as usize),
        (CL_SET_COMPUTE_LIMITS, cl_set_compute_limits as usize),
        (CL_SET_COMPRESSION_STATE, cl_set_compression_state as usize),
    ]
}

pub fn install(xbox_vt: &[AtomicUsize]) -> usize {
    let mut n = 0;
    for (slot, f) in plan() {
        if slot < xbox_vt.len() {
            xbox_vt[slot].store(f, Ordering::Relaxed);
            n += 1;
        }
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_covers_every_slot_once() {
        let vt: Vec<AtomicUsize> = (0..128).map(|_| AtomicUsize::new(0)).collect();
        assert_eq!(install(&vt), SLOTS.len());
        for s in SLOTS {
            assert_ne!(vt[s].load(Ordering::Relaxed), 0);
        }
        assert_eq!(vt[CL_WRITE32_BOTTOM_OF_PIPE].load(Ordering::Relaxed), cl_write32_bottom_of_pipe as usize);
        assert_eq!(vt[CL_WAIT64].load(Ordering::Relaxed), cl_wait64 as usize);
        assert_eq!(vt[81].load(Ordering::Relaxed), 0);
        assert_eq!(vt[78].load(Ordering::Relaxed), 0);
    }

    #[test]
    fn bottom_of_pipe_validation_matches_debug_umd() {
        assert_eq!(bop_error(0, 0, false, 5), None);
        assert_eq!(bop_error(0x3800, 0x10, false, 5), None);
        assert_eq!(bop_error(0, 0x80, false, 5), None);
        assert!(bop_error(0, 0x4, false, 0).is_some());
        assert!(bop_error(0, 0x1, false, 0).is_some());
        assert_eq!(bop_error(0, 0x1, true, 0), None);
        assert_eq!(bop_error(0, 0x2, true, 0), None);
        assert!(bop_error(0, 0x3, true, 0).is_some());
        assert!(bop_error(0, 0x1, true, 7).is_some());
        assert!(bop_error(0, 0x30, true, 7).is_some());
        assert!(bop_error(0x400, 0, true, 7).is_some());
    }

    #[test]
    fn bundle_validation_matches_debug_umd() {
        assert_eq!(bundle_error(Some(0x100), 0, 0x40, None, 0, 0, 0), None);
        assert!(bundle_error(None, 0, 0x40, None, 0, 0, 0).is_some());
        assert!(bundle_error(Some(0x100), 0x100, 0x40, None, 0, 0, 0).is_some());
        assert!(bundle_error(Some(0x100), 2, 0x40, None, 0, 0, 0).is_some());
        assert!(bundle_error(Some(0x100), 0, 0, None, 0, 0, 0).is_some());
        assert!(bundle_error(Some(0x100), 0, 0x40_0000, None, 0, 0, 0).is_some());
        assert!(bundle_error(Some(0x100), 0, 0x40, Some(8), 8, 0, 0).is_some());
        assert_eq!(bundle_error(Some(0x100), 0, 0x40, Some(8), 4, 0, 0), None);
        assert!(bundle_error(Some(0x100), 0, 0x40, None, 4, 0, 0).is_some());
        assert!(bundle_error(Some(0x100), 0, 0x40, None, 0, 1, 0).is_some());
        assert!(bundle_error(Some(0x100), 0, 0x40, None, 0, 0, 4).is_some());
    }

    #[test]
    fn gds_validation_matches_debug_umd() {
        assert_eq!(ordered_append_error(7, 0x3f00, 2, 3), None);
        assert!(ordered_append_error(8, 0, 0, 0).is_some());
        assert!(ordered_append_error(0, 0x3f01, 0, 0).is_some());
        assert!(ordered_append_error(0, 0, 3, 0).is_some());
        assert!(ordered_append_error(0, 0, 0, 4).is_some());
        assert_eq!(gds_error(0, 4, 0x1000, 0, false), None);
        assert_eq!(gds_error(0, 4, 0x1000, 3, false), None);
        assert_eq!(gds_error(0, 4, 0x1000, 4, false), None);
        assert!(gds_error(0, 4, 0x1000, 1, false).is_some());
        assert_eq!(gds_error(0, 4, 0x1000, 2, true), None);
        assert!(gds_error(0, 4, 0x1000, 3, true).is_some());
        assert!(gds_error(0x3f00, 1, 0x1000, 0, true).is_some());
        assert!(gds_error(0, 1, 0, 0, true).is_some());
        assert!(gds_error(0, 1, 0x1002, 0, true).is_some());
    }

    #[test]
    fn hi_stencil_requires_control_and_depth_resource() {
        assert_eq!(hi_stencil_error(None, true), None);
        assert_eq!(hi_stencil_error(Some(RESOURCE_FLAG_ALLOW_DEPTH_STENCIL), true), None);
        assert!(hi_stencil_error(Some(RESOURCE_FLAG_ALLOW_RENDER_TARGET), true).is_some());
        assert!(hi_stencil_error(Some(RESOURCE_FLAG_ALLOW_DEPTH_STENCIL), false).is_some());
        assert!(hi_stencil_error(None, false).is_some());
    }

    #[test]
    fn msaa_sample_positions_pack_like_pa_sc_aa_sample_locs() {
        let mut pos = [0u8; 128];
        for p in 0..4 {
            for s in 0..16 {
                pos[32 * p + 2 * s] = ((s as i8) - 8) as u8;
                pos[32 * p + 2 * s + 1] = ((p as i8) * 2 - 4) as u8;
            }
        }
        assert_eq!(sample_positions_error(&pos, Some([2, 0])), None);
        let v = pc_sample_positions(&pos, 4, 4);
        assert_eq!(v.len(), 16);
        assert_eq!(v[0], [-8, -4]);
        assert_eq!(v[3], [-5, -4]);
        assert_eq!(v[4], [-8, -2]);
        assert_eq!(v[15], [-5, 2]);
        assert!(!quad_is_uniform(&pos, 4));
        let mut u = [0u8; 128];
        for p in 0..4 { u[32 * p] = 3; u[32 * p + 1] = 0xfd; }
        assert!(quad_is_uniform(&u, 1));
        assert_eq!(pc_sample_positions(&u, 1, 1), vec![[3, -3]]);
        assert_eq!(config_samples([2, 0]), 4);
        assert_eq!(config_samples([3, 0]), 8);
        let mut bad = pos;
        bad[5] = 8;
        assert!(sample_positions_error(&bad, None).is_some());
        assert_eq!(masked_sample_positions(&pos), pos);
        let mut wide = pos;
        wide[0] = 9;
        wide[1] = 0x17;
        wide[2] = 0xf0;
        assert!(sample_positions_error(&wide, None).is_some());
        let m = masked_sample_positions(&wide);
        assert_eq!(sample_positions_error(&m, None), None);
        assert_eq!(pc_sample_positions(&m, 2, 1), vec![[-7, 7], [0, -4]]);
        assert!(sample_positions_error(&pos, Some([0xc00, 0])).is_some());
        assert!(sample_positions_error(&pos, Some([0, 0x2000])).is_some());
    }

    #[test]
    fn compute_limits_validation_matches_debug_umd() {
        assert_eq!(compute_limits_error(&[0; 6]), None);
        assert_eq!(compute_limits_error(&[0x3fe, 0x640, 15, 0x640, 8, 3]), None);
        assert!(compute_limits_error(&[0x400, 0, 0, 0, 0, 0]).is_some());
        assert!(compute_limits_error(&[0x3ff, 0, 0, 0, 0, 0]).is_some());
        assert!(compute_limits_error(&[0, 0x641, 0, 0, 0, 0]).is_some());
        assert!(compute_limits_error(&[0, 0, 16, 0, 0, 0]).is_some());
        assert!(compute_limits_error(&[0, 0, 0, 0x641, 0, 0]).is_some());
        assert!(compute_limits_error(&[0, 0, 0, 0, 9, 0]).is_some());
        assert!(compute_limits_error(&[0, 0, 0, 0, 0, 4]).is_some());
    }

    #[test]
    fn compression_state_validation_matches_debug_umd() {
        let rt = Some((RESOURCE_FLAG_ALLOW_RENDER_TARGET, 28));
        let ds = Some((RESOURCE_FLAG_ALLOW_DEPTH_STENCIL, 45));
        let d32 = Some((RESOURCE_FLAG_ALLOW_DEPTH_STENCIL, 40));
        assert_eq!(compression_error(1, PRESERVE_COMPRESSED, rt, 0), None);
        assert_eq!(compression_error(0, PRESERVE_EXPANDED, rt, 0), None);
        assert!(compression_error(1, PRESERVE_EXPANDED, rt, 0).is_some());
        assert!(compression_error(0, PRESERVE_COMPRESSED, rt, 0).is_some());
        assert!(compression_error(1, PRESERVE_COMPRESSED_STENCIL, rt, 0).is_some());
        assert!(compression_error(4, 0, rt, 0).is_some());
        assert!(compression_error(0, 0x1000, rt, 0).is_some());
        assert!(compression_error(0, 0, rt, 1).is_some());
        assert!(compression_error(0, 0, None, 0).is_some());
        assert!(compression_error(0, 0, Some((0, 28)), 0).is_some());
        assert_eq!(compression_error(3, PRESERVE_COMPRESSED | PRESERVE_COMPRESSED_STENCIL, ds, 0), None);
        assert!(compression_error(3, PRESERVE_EXPANDED_STENCIL, ds, 0).is_some());
        assert!(compression_error(1, PRESERVE_COMPRESSED_STENCIL, ds, 0).is_some());
        assert!(compression_error(2, 0, d32, 0).is_some());
        assert_eq!(compression_error(1, PRESERVE_COMPRESSED, d32, 0), None);
    }

    #[repr(C)]
    struct FakeList {
        vt: *const usize,
        ty: u32,
        barriers: AtomicUsize,
        barrier_type: AtomicUsize,
    }

    unsafe extern "system" fn fake_get_type(this: *mut c_void) -> u32 {
        unsafe { (*(this as *const FakeList)).ty }
    }

    unsafe extern "system" fn fake_barrier(this: *mut c_void, n: u32, b: *const u8) {
        let l = unsafe { &*(this as *const FakeList) };
        l.barriers.fetch_add(n as usize, Ordering::Relaxed);
        l.barrier_type.store(unsafe { *(b as *const u32) } as usize, Ordering::Relaxed);
    }

    fn fake_vt() -> Vec<usize> {
        let mut v = vec![0usize; 80];
        v[8] = fake_get_type as usize;
        v[26] = fake_barrier as usize;
        v
    }

    fn fake(vt: &[usize], ty: u32) -> Box<FakeList> {
        Box::new(FakeList { vt: vt.as_ptr(), ty, barriers: AtomicUsize::new(0), barrier_type: AtomicUsize::new(0) })
    }

    #[test]
    fn waits_record_ops_in_argument_order_and_stall_the_list() {
        let vt = fake_vt();
        let l = fake(&vt, LIST_TYPE_DIRECT);
        let p = &*l as *const FakeList as *mut c_void;
        unsafe {
            cl_wait32(p, 0x1000, gpu_wait::FUNC_EQUAL, 7, 0xff, 0);
            cl_wait64(p, 0x2000, gpu_wait::FUNC_GREATER_EQUAL, 0x1_0000_0002, 0);
        }
        gpu_wait::on_close(p);
        assert_eq!(gpu_wait::closed_ops(p as usize), vec![
            Op::Wait { addr: 0x1000, func: gpu_wait::FUNC_EQUAL, value: 7, mask: 0xff },
            Op::Wait { addr: 0x2000, func: gpu_wait::FUNC_GREATER_EQUAL, value: 0x1_0000_0002, mask: u64::MAX },
        ]);
        assert_eq!(l.barriers.load(Ordering::Relaxed), 2);
        assert_eq!(l.barrier_type.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn invalid_waits_record_nothing() {
        let vt = fake_vt();
        for (ty, va, func, flags) in [(3, 0x1000, gpu_wait::FUNC_EQUAL, 0), (2, 0x1000, gpu_wait::FUNC_EQUAL, 0), (0, 0x1002, gpu_wait::FUNC_EQUAL, 0), (0, 0x1000, gpu_wait::FUNC_NEVER, 0), (0, 0x1000, gpu_wait::FUNC_EQUAL, 1)] {
            let l = fake(&vt, ty);
            let p = &*l as *const FakeList as *mut c_void;
            unsafe { cl_wait32(p, va, func, 1, !0, flags) };
            unsafe { cl_wait64(p, 0x1004, gpu_wait::FUNC_EQUAL, 1, 0) };
            gpu_wait::on_close(p);
            assert!(gpu_wait::closed_ops(p as usize).is_empty());
            assert_eq!(l.barriers.load(Ordering::Relaxed), 0);
        }
    }

    #[test]
    fn rejected_slots_touch_only_get_type() {
        let vt = fake_vt();
        let l = fake(&vt, 2);
        let p = &*l as *const FakeList as *mut c_void;
        unsafe {
            cl_force_z_order(p, 1);
            cl_write32_bottom_of_pipe(p, 0x1000, 1, 0, 0);
            cl_write64_bottom_of_pipe(p, 0x1000, 1, 0, 0);
            crate::cl_fw::advance_predication(p);
            cl_set_hi_stencil(p, core::ptr::null_mut(), core::ptr::null());
        }
        gpu_wait::on_close(p);
        assert!(gpu_wait::closed_ops(p as usize).is_empty());
        assert_eq!(z_order_error(LIST_TYPE_DIRECT), None);
        assert!(z_order_error(2).is_some());
        assert!(z_order_error(LIST_TYPE_BUNDLE).is_some());
    }

    #[test]
    fn ordered_append_setup_is_kept_per_list_and_counter() {
        static WBI: Mutex<Vec<(u64, u32)>> = Mutex::new(Vec::new());
        unsafe extern "system" fn fake_wbi(_this: *mut c_void, n: u32, p: *const [u64; 2], _m: *const u32) {
            for i in 0..n as usize {
                let e = unsafe { *p.add(i) };
                WBI.lock().unwrap().push((e[0], e[1] as u32));
            }
        }
        crate::gds::set_buffer_for_test(0x5150, 0x1_0000_0000);
        let mut vt = fake_vt();
        vt[66] = fake_wbi as usize;
        let l = fake(&vt, LIST_TYPE_DIRECT);
        let p = &*l as *const FakeList as *mut c_void;
        let b = fake(&vt, LIST_TYPE_BUNDLE);
        let bp = &*b as *const FakeList as *mut c_void;
        unsafe {
            cl_setup_ordered_append(p, 3, 0x11, 0x100, 1, 2, 0);
            cl_setup_ordered_append(p, 8, 0x11, 0x100, 1, 2, 0);
            cl_setup_ordered_append(bp, 3, 0x11, 0x100, 1, 2, 0);
        }
        let base = 0x1_0000_0000u64 + tombox_xshader::gds_abi::tb_gds_oa_byte_offset(3, 0) as u64;
        assert_eq!(*WBI.lock().unwrap(), vec![(base, 0x100), (base + 4, 0x11), (base + 8, 1), (base + 12, 2)]);
        assert_eq!(l.barriers.load(Ordering::Relaxed), 1);
        assert_eq!(b.barriers.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn write64_bottom_of_pipe_routes_by_flag() {
        assert_eq!(bop64_route(0x1000, 0), Ok(Bop64::Value));
        assert_eq!(bop64_route(0x1000, BOP_FLAG_GPU_TIMESTAMP), Ok(Bop64::GpuTimestamp));
        assert_eq!(bop64_route(0x1000, BOP_FLAG_SYSTEM_TIMESTAMP), Ok(Bop64::SystemTimestamp));
        assert!(bop64_route(0x1004, 0).is_err());
        assert!(bop64_route(0, 0).is_err());
        assert!(bop64_route(0x100_0000_0000, 0).is_err());
    }

    #[test]
    fn execute_indirect_bundle_runs_only_on_direct_lists() {
        assert_eq!(bundle_list_error(LIST_TYPE_DIRECT), None);
        assert!(bundle_list_error(2).is_some());
        assert!(bundle_list_error(3).is_some());
        assert!(bundle_error(Some(0x100), 0, 0x40, None, 0, LIST_TYPE_BUNDLE, 0).is_some());
    }

    #[test]
    fn resource_desc_offsets_match_d3d12_layout() {
        #[repr(C)]
        struct Desc { dim: u32, align: u64, width: u64, height: u32, depth: u16, mips: u16, format: u32, count: u32, quality: u32, layout: u32, flags: u32 }
        assert_eq!(core::mem::offset_of!(Desc, width), 0x10);
        assert_eq!(core::mem::offset_of!(Desc, format), 0x20);
        assert_eq!(core::mem::offset_of!(Desc, flags), 0x30);
        assert_eq!(core::mem::size_of::<Desc>(), 0x38);
        let mut d = [0u8; 56];
        d[0x30..0x34].copy_from_slice(&RESOURCE_FLAG_ALLOW_DEPTH_STENCIL.to_le_bytes());
        d[0x20..0x24].copy_from_slice(&45u32.to_le_bytes());
        d[0x10..0x18].copy_from_slice(&0x400u64.to_le_bytes());
        assert_eq!(desc_flags(&d), RESOURCE_FLAG_ALLOW_DEPTH_STENCIL);
        assert_eq!(desc_format(&d), 45);
        assert_eq!(desc_width(&d), 0x400);
    }
}
