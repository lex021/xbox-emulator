//! Bind the compute PSO compiled for the guest dispatch dimensions.
mod indirect;
mod resume;
mod tables;
pub(crate) use resume::enabled as resume_enabled;

use crate::device_wrap::Guid;
use compat_common::record;
use core::ffi::c_void;
use core::ptr::null_mut;
use tombox_xshader::gds_abi::{tb_gws_group_count, tb_gws_phase_word, tb_gws_scratch_bytes};
use core::sync::atomic::{
    AtomicBool, AtomicUsize,
    Ordering::{Acquire, Relaxed, Release},
};

type Create =
    unsafe extern "system" fn(*mut c_void, *const u8, *const Guid, *mut *mut c_void) -> i32;
type Dispatch = unsafe extern "system" fn(*mut c_void, u32, u32, u32);
type Indirect =
    unsafe extern "system" fn(*mut c_void, *mut c_void, u32, *mut c_void, u64, *mut c_void, u64);
type Bind = unsafe extern "system" fn(*mut c_void, *mut c_void);

const E_FAIL: i32 = 0x8000_4005u32 as i32;
const IID_PSO: Guid = Guid {
    d1: 0x765a30f3,
    d2: 0xf624,
    d3: 0x4c6f,
    d4: [0xa8, 0x28, 0xac, 0xe9, 0x48, 0x62, 0x24, 0x45],
};
static PREV_DISPATCH: AtomicUsize = AtomicUsize::new(0);
static PREV_INDIRECT: AtomicUsize = AtomicUsize::new(0);
static ACTIVE: AtomicBool = AtomicBool::new(false);
static SKIPPED: AtomicUsize = AtomicUsize::new(0);
const PHASED_VARIANT: u32 = 16;
const PHASED_PLAN: u32 = 32;
const PHASED_ROOT: u32 = 48;
const INDIRECT_DISPATCH_STRIDE: u32 = 49;
const NO_GWS: u32 = 50;
const GWS_SIGNAL: u32 = 51;
const TABLE_LAYOUT: u32 = 52;

pub(crate) fn phased_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var("TOMBOX_GWS_PHASED").as_deref() == Ok("1"))
}

const fn key(groups: u32) -> Guid {
    Guid {
        d1: 0x221b821a + groups,
        d2: 0xda43,
        d3: 0x4f26,
        d4: [0xbd, 0xe0, 0x69, 0xe7, 0x90, 0x6a, 0x68, 0x1f],
    }
}

unsafe fn slot(obj: *mut c_void, n: usize) -> usize {
    unsafe { *(*obj.cast::<*const usize>()).add(n) }
}

unsafe fn get_private(pso: *mut c_void, groups: u32, data: *mut c_void, len: u32) -> bool {
    let get: unsafe extern "system" fn(*mut c_void, *const Guid, *mut u32, *mut c_void) -> i32 =
        unsafe { core::mem::transmute(slot(pso, 3)) };
    let mut size = len;
    unsafe { get(pso, &key(groups), &mut size, data) >= 0 && size == len }
}

unsafe fn mask(pso: *mut c_void) -> u32 {
    let mut bits = 0u32;
    if !pso.is_null() && unsafe { get_private(pso, 0, (&mut bits as *mut u32).cast(), 4) } {
        return bits;
    }
    0
}

unsafe fn set_private<const N: usize>(object: *mut c_void, id: u32, data: &[u32; N]) -> i32 {
    let set: unsafe extern "system" fn(*mut c_void, *const Guid, u32, *const c_void) -> i32 =
        unsafe { core::mem::transmute(slot(object, 4)) };
    unsafe { set(object, &key(id), (N * 4) as u32, data.as_ptr().cast()) }
}

pub(crate) unsafe fn note_phased_root(root: *mut c_void, index: u32, bytecode: &[u8]) {
    if root.is_null() {
        return;
    }
    unsafe { set_private(root, PHASED_ROOT, &[index]) };
    if let Ok(layout) = tombox_xshader::parse_root_signature(bytecode) {
        let spans = tables::root_spans(&layout);
        unsafe { set_private(root, TABLE_LAYOUT, &spans) };
    }
}

pub(crate) unsafe fn note_indirect_signature(
    signature: *mut c_void,
    stride: u32,
    args: &[[u32; 4]],
) {
    if let [dispatch] = args {
        if !signature.is_null() && dispatch[0] == 2 && stride >= 12 && stride % 4 == 0 {
            unsafe { set_private(signature, INDIRECT_DISPATCH_STRIDE, &[stride]) };
        }
    }
}

pub(crate) unsafe fn indirect_stride(signature: *mut c_void) -> Option<u32> {
    let mut stride = 0u32;
    (!signature.is_null()
        && unsafe {
            get_private(signature, INDIRECT_DISPATCH_STRIDE, (&mut stride as *mut u32).cast(), 4)
        })
    .then_some(stride)
}

pub(crate) unsafe fn note_compute(pso: *mut c_void, bytecode: *const u8, len: usize) {
    if !resume::enabled() || pso.is_null() || bytecode.is_null() || !(32..=0x40_0000).contains(&len) {
        return;
    }
    let blob = unsafe { core::slice::from_raw_parts(bytecode, len) };
    let Ok(shader) = tombox_xshader::extract(blob) else {
        return;
    };
    let instructions = tombox_xshader::rdna2::disassemble(&shader.isa);
    let decoded_bytes = instructions.last().map_or(0, |ins| ins.offset + ins.dwords * 4);
    if decoded_bytes != shader.isa.len() || instructions.is_empty() {
        return;
    }
    if !instructions.iter().any(|ins| ins.mnemonic.starts_with("ds_gws_")) {
        unsafe { set_private(pso, NO_GWS, &[1]) };
    } else if instructions.iter().any(|ins| matches!(
        ins.mnemonic.as_str(), "ds_gws_sema_v" | "ds_gws_sema_br" | "ds_gws_sema_release_all"
    )) {
        unsafe { set_private(pso, GWS_SIGNAL, &[1]) };
    }
}

unsafe fn can_signal(pso: *mut c_void) -> bool {
    let mut tag = 0u32;
    !pso.is_null()
        && unsafe { get_private(pso, GWS_SIGNAL, (&mut tag as *mut u32).cast(), 4) }
        && tag == 1
}

unsafe fn cannot_signal(pso: *mut c_void) -> bool {
    let mut tag = 0u32;
    !pso.is_null()
        && unsafe { get_private(pso, NO_GWS, (&mut tag as *mut u32).cast(), 4) }
        && tag == 1
}

unsafe fn phased_root(root: *mut c_void) -> Option<u32> {
    let mut index = 0u32;
    (!root.is_null()
        && unsafe { get_private(root, PHASED_ROOT, (&mut index as *mut u32).cast(), 4) })
    .then_some(index)
}

unsafe fn release(obj: *mut c_void) {
    let f: unsafe extern "system" fn(*mut c_void) -> u32 =
        unsafe { core::mem::transmute(slot(obj, 2)) };
    unsafe { f(obj) };
}

unsafe fn create_with_code(
    create: Create,
    device: *mut c_void,
    desc: &[u8],
    code: &[u8],
    iid: *const Guid,
    out: *mut *mut c_void,
) -> i32 {
    let mut pc = desc.to_vec();
    pc[8..16].copy_from_slice(&(code.as_ptr() as u64).to_le_bytes());
    pc[16..24].copy_from_slice(&(code.len() as u64).to_le_bytes());
    // An Xbox cached blob does not describe any of the host variants.
    pc[0x20..0x30].fill(0);
    unsafe { create(device, pc.as_ptr(), iid, out) }
}

unsafe fn attach_variants(
    base: *mut c_void,
    variants: impl IntoIterator<Item = (u32, *mut c_void)>,
) -> i32 {
    let set_interface: unsafe extern "system" fn(*mut c_void, *const Guid, *mut c_void) -> i32 =
        unsafe { core::mem::transmute(slot(base, 5)) };
    let mut mask = 0u32;
    for (groups, variant) in variants {
        let hr = unsafe { set_interface(base, &key(groups), variant) };
        unsafe { release(variant) };
        if hr >= 0 {
            mask |= 1 << groups;
        }
    }
    if mask == 0 {
        return E_FAIL;
    }
    unsafe { set_private(base, 0, &[mask]) }
}

/// Called when normal translation fails. A PSO handle owns the compiled variants so their COM
/// lifetime follows the handle, even when it was built on the asynchronous PSO worker.
pub(crate) unsafe fn try_create(
    create: Create,
    device: *mut c_void,
    desc: &[u8],
    ptr: *const u8,
    len: usize,
    iid: *const Guid,
    out: *mut *mut c_void,
) -> Option<i32> {
    if ptr.is_null() || !(32..=0x40_0000).contains(&len) {
        return None;
    }
    let blob = unsafe { core::slice::from_raw_parts(ptr, len) };
    let ex = tombox_xshader::extract(blob).ok()?;
    if !matches!(ex.stage, tombox_xshader::Stage::Compute) {
        return None;
    }
    let instructions = tombox_xshader::rdna2::disassemble(&ex.isa);
    let semaphore_wait = instructions.iter().any(|i| i.mnemonic == "ds_gws_sema_p");
    let barriers = instructions.iter().filter(|i| i.mnemonic == "ds_gws_barrier").count();
    let ordered_wait = semaphore_wait && instructions.iter().filter(|i| i.mnemonic.starts_with("ds_gws_")).count() == 1;
    let releases_semaphores = instructions.iter().any(|i| i.mnemonic == "ds_gws_sema_release_all");
    let resume_mode = if ordered_wait {
        3
    } else if semaphore_wait {
        1
    } else if barriers == 1 {
        if releases_semaphores { 4 } else { 2 }
    } else {
        0
    };
    if !semaphore_wait && barriers == 0 {
        return None;
    }
    if !out.is_null() {
        unsafe { *out = null_mut() };
    }
    let root = u64::from_le_bytes(desc[0..8].try_into().unwrap()) as *mut c_void;
    if crate::gds::root_index(root).is_none() {
        record(
            "d3d12_xs",
            "gws_dispatch",
            "compute PSO requires a root signature with the GDS UAV",
        );
        return Some(E_FAIL);
    }
    let carrier = match tombox_xshader::dxc::compile("[numthreads(1,1,1)] void main() {}", "cs_6_0")
    {
        Ok(code) => code,
        Err(e) => {
            record("d3d12_xs", "gws_dispatch", &e);
            return Some(E_FAIL);
        }
    };
    let mut base = null_mut();
    let hr = unsafe { create_with_code(create, device, desc, &carrier, &IID_PSO, &mut base) };
    if hr < 0 {
        return Some(hr);
    }
    let variants = (1..=4).filter_map(|groups| {
        let code = match std::panic::catch_unwind(|| crate::shader_xlat::compile_gws(&ex, groups)) {
            Ok(Ok(code)) => code,
            Ok(Err(e)) => {
                record(
                    "d3d12_xs",
                    "gws_dispatch",
                    &format!("guest X={groups} unavailable: {e}"),
                );
                return None;
            }
            Err(_) => return None,
        };
        let mut variant = null_mut();
        let hr = unsafe { create_with_code(create, device, desc, &code, &IID_PSO, &mut variant) };
        if hr < 0 {
            record(
                "d3d12_xs",
                "gws_dispatch",
                &format!("guest X={groups} PSO creation failed: {hr:#010x}"),
            );
            return None;
        }
        Some((groups, variant))
    });
    let can_phase = phased_enabled() && unsafe { phased_root(root) }.is_some();
    let phased = can_phase.then_some(()).and_then(|()| {
        let compiled =
            std::panic::catch_unwind(|| crate::shader_xlat::compile_phased_gws(&ex));
        let Ok(Ok((code, plan))) = compiled else {
            return None;
        };
        let mut variant = null_mut();
        let hr = unsafe { create_with_code(create, device, desc, &code, &IID_PSO, &mut variant) };
        if hr < 0 {
            return None;
        }
        let address = root as u64;
        let plan = [plan[0], plan[1], address as u32, (address >> 32) as u32, resume_mode];
        if unsafe { set_private(variant, PHASED_PLAN, &plan) } < 0 {
            unsafe { release(variant) };
            return None;
        }
        Some((PHASED_VARIANT, variant))
    });
    let hr = unsafe { attach_variants(base, variants.chain(phased)) };
    let hr = if hr >= 0 && !out.is_null() {
        let qi: unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> i32 =
            unsafe { core::mem::transmute(slot(base, 0)) };
        let hr = unsafe { qi(base, iid, out) };
        if hr >= 0 {
            ACTIVE.store(true, Release);
        }
        hr
    } else if hr >= 0 {
        1 // S_FALSE for a validation-only CreateComputePipelineState.
    } else {
        hr
    };
    unsafe { release(base) };
    Some(hr)
}

unsafe fn get_variant(pso: *mut c_void, id: u32) -> *mut c_void {
    let mut variant: *mut c_void = null_mut();
    if unsafe {
        get_private(
            pso,
            id,
            (&mut variant as *mut *mut c_void).cast(),
            core::mem::size_of::<*mut c_void>() as u32,
        )
    } {
        variant
    } else {
        null_mut()
    }
}

unsafe fn allocate_phase(list: *mut c_void, pso: *mut c_void, groups: [u32; 3], routed: bool) -> Option<(Phase, u32)> {
    if !matches!(unsafe { crate::fill::list_type(list) }, 0 | 2)
        || crate::cl2_state::predication_of(list).is_some()
    {
        return None;
    }
    let root = crate::cl2_state::compute_root(list)?;
    let index = unsafe { phased_root(root as *mut c_void) }?;
    let count = tb_gws_group_count(groups)?;
    let mut plan = [0u32; 5];
    if !unsafe { get_private(pso, PHASED_PLAN, plan.as_mut_ptr().cast(), 20) } {
        return None;
    }
    let [steps, waves_per_group, root_lo, root_hi, resume_mode] = plan;
    if root as u64 != (u64::from(root_lo) | (u64::from(root_hi) << 32)) {
        return None;
    }
    let waves = count * waves_per_group;
    let rounds = steps * waves + 1;
    let bytes = tb_gws_scratch_bytes(waves);
    let (scratch, va) = unsafe { crate::gws_scratch::begin(list, pso, bytes) }?;
    Some((Phase { scratch, va, pso: pso as usize, groups, waves, rounds, index, routed }, resume_mode))
}

unsafe fn phased_dispatch(list: *mut c_void, pso: *mut c_void, groups: [u32; 3], bind: Bind) -> bool {
    let Some((phase, resume_mode)) = (unsafe { allocate_phase(list, pso, groups, false) }) else {
        return false;
    };
    let kind = resume::Kind::from_mode(resume_mode);
    unsafe {
        if kind == Some(resume::Kind::OrderedSemaphore) {
            resume::before_ordered_wait(list, bind);
        }
        phase.run(list, bind);
        if let Some(kind) = kind {
            resume::remember(list, phase, kind, bind);
        } else {
            resume::forget(list);
        }
    }
    true
}

#[derive(Clone, Copy)]
struct Phase {
    scratch: usize,
    va: u64,
    pso: usize,
    groups: [u32; 3],
    waves: u32,
    rounds: u32,
    index: u32,
    routed: bool,
}

impl Phase {
    unsafe fn run(self, list: *mut c_void, bind: Bind) {
        let dispatch: Dispatch = unsafe { core::mem::transmute(slot(list, 14)) };
        let Self { scratch, va, pso, groups, waves, rounds, index, routed } = self;
        let bind_uav: unsafe extern "system" fn(*mut c_void, u32, u64) =
            unsafe { core::mem::transmute(slot(list, 41)) };
        let constant: unsafe extern "system" fn(*mut c_void, u32, u32, u32) =
            unsafe { core::mem::transmute(slot(list, 33)) };
        let barrier: unsafe extern "system" fn(*mut c_void, u32, *const u64) =
            unsafe { core::mem::transmute(slot(list, 26)) };
        let uav_barrier = [2u64, 0, 0, 0];
        unsafe {
            crate::fill::barrier_pub(list, scratch, crate::gws_scratch::COPY_DEST, crate::gws_scratch::UAV);
            bind_uav(list, index + 1, va);
            bind(list, pso as *mut c_void);
            barrier(list, 1, uav_barrier.as_ptr());
            for _ in 0..rounds {
                for selected in 0..waves {
                    let control = if routed { selected } else { tb_gws_phase_word(groups, waves, selected) };
                    constant(list, index + 2, control, 0);
                    dispatch(list, 1, 1, 1);
                    barrier(list, 1, uav_barrier.as_ptr());
                }
            }
            crate::gws_scratch::end(list, scratch);
        }
    }
}

unsafe fn dispatch_for_pso(
    list: *mut c_void,
    pso: *mut c_void,
    groups: [u32; 3],
    bind: Bind,
    dispatch: Dispatch,
) {
    let variants = unsafe { mask(pso) };
    if variants == 0 {
        unsafe { dispatch(list, groups[0], groups[1], groups[2]) };
        if !pso.is_null() && !groups.contains(&0) {
            unsafe { resume::after_dispatch(list, pso, bind) };
        }
        return;
    }
    if groups.contains(&0) {
        return;
    }
    if variants & (1 << PHASED_VARIANT) != 0 && (groups != [1, 1, 1] || resume::enabled()) {
        let variant = unsafe { get_variant(pso, PHASED_VARIANT) };
        if !variant.is_null() {
            let ran = unsafe { phased_dispatch(list, variant, groups, bind) };
            if ran {
                unsafe { bind(list, pso) };
            }
            unsafe { release(variant) };
            if ran {
                return;
            }
        }
    }
    resume::forget(list);
    if let [x @ 1..=4, 1, 1] = groups {
        let mut variant: *mut c_void = null_mut();
        if variants & (1 << x) != 0
            && unsafe {
                get_private(
                    pso,
                    x,
                    (&mut variant as *mut *mut c_void).cast(),
                    core::mem::size_of::<*mut c_void>() as u32,
                )
            }
            && !variant.is_null()
        {
            unsafe {
                bind(list, variant);
                dispatch(list, 1, 1, 1);
                bind(list, pso);
                // GetPrivateData added a temporary reference. The carrier still owns its reference.
                release(variant);
            }
            return;
        }
    }
    if SKIPPED.fetch_add(1, Relaxed) < 16 {
        record(
            "d3d12_xs",
            "gws_dispatch",
            &format!("unsupported cooperative GWS Dispatch{groups:?}; skipped"),
        );
    }
}

unsafe extern "system" fn dispatch(list: *mut c_void, x: u32, y: u32, z: u32) {
    let prev: Dispatch = unsafe { core::mem::transmute(PREV_DISPATCH.load(Relaxed)) };
    unsafe { dispatch_with(list, x, y, z, prev) };
}

pub(crate) unsafe fn dispatch_with(list: *mut c_void, x: u32, y: u32, z: u32, prev: Dispatch) {
    if ![x, y, z].contains(&0) && !unsafe { crate::global_bind::before_dispatch(list) } {
        return;
    }
    if unsafe { crate::fssm_compute::dispatch(list, || prev(list, x, y, z)) } {
        return;
    }
    if !ACTIVE.load(Acquire) && !resume::enabled() {
        unsafe { prev(list, x, y, z) };
        return;
    }
    let bind: Bind = unsafe { core::mem::transmute(crate::cmdlist_wrap::real_method(25)) };
    let pso = crate::cl2_state::bound_pso(list).unwrap_or(0) as *mut c_void;
    unsafe { dispatch_for_pso(list, pso, [x, y, z], bind, prev) };
}

unsafe extern "system" fn indirect(
    list: *mut c_void,
    signature: *mut c_void,
    count: u32,
    args: *mut c_void,
    offset: u64,
    count_buf: *mut c_void,
    count_offset: u64,
) {
    if count != 0 && !unsafe { crate::global_bind::before_indirect(list) } {
        return;
    }
    let mut resume_after = None;
    if ACTIVE.load(Acquire) || resume::enabled() {
        let pso = crate::cl2_state::bound_pso(list).unwrap_or(0) as *mut c_void;
        if unsafe { mask(pso) } != 0 {
            if count == 0 {
                return;
            }
            let result = unsafe { indirect::run(list, pso, signature, count, args, offset, count_buf, count_offset) };
            if result.is_ok() {
                return;
            }
            resume::forget(list);
            if SKIPPED.fetch_add(1, Relaxed) < 16 {
                let reason = result.unwrap_err();
                record(
                    "d3d12_xs",
                    "gws_dispatch",
                    &format!("cooperative GWS ExecuteIndirect skipped: {reason}"),
                );
            }
            return;
        }
        if count != 0 {
            if !pso.is_null() && resume::enabled() && unsafe { indirect_stride(signature) }.is_some() {
                resume_after = Some(pso);
            } else {
                resume::forget(list);
            }
        }
    }
    let prev: Indirect = unsafe { core::mem::transmute(PREV_INDIRECT.load(Relaxed)) };
    unsafe {
        prev(
            list,
            signature,
            count,
            args,
            offset,
            count_buf,
            count_offset,
        )
    };
    if let Some(pso) = resume_after {
        let bind: Bind = unsafe { core::mem::transmute(crate::cmdlist_wrap::real_method(25)) };
        unsafe { resume::after_dispatch(list, pso, bind) };
    }
}

pub(crate) use resume::{forget as forget_checkpoint, forget_signals, submit_lists, queue_signal, queue_wait};

pub(crate) fn install(vt: &[AtomicUsize]) {
    PREV_DISPATCH.store(
        vt[14].swap(dispatch as *const () as usize, Relaxed),
        Relaxed,
    );
    PREV_INDIRECT.store(
        vt[59].swap(indirect as *const () as usize, Relaxed),
        Relaxed,
    );
}
