//! Bind the compute PSO compiled for the guest dispatch dimensions.
use crate::device_wrap::Guid;
use compat_common::record;
use core::ffi::c_void;
use core::ptr::null_mut;
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

pub(crate) unsafe fn note_phased_root(root: *mut c_void, index: u32) {
    if !root.is_null() {
        unsafe { set_private(root, PHASED_ROOT, &[index]) };
    }
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
    if !matches!(ex.stage, tombox_xshader::Stage::Compute)
        || !tombox_xshader::rdna2::disassemble(&ex.isa)
            .iter()
            .any(|i| i.mnemonic == "ds_gws_sema_p")
    {
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
    let phased = (2..=8).filter(|_| can_phase).filter_map(|groups| {
        let compiled =
            std::panic::catch_unwind(|| crate::shader_xlat::compile_phased_gws(&ex, groups));
        let Ok(Ok((code, plan))) = compiled else {
            return None;
        };
        let mut variant = null_mut();
        let hr = unsafe { create_with_code(create, device, desc, &code, &IID_PSO, &mut variant) };
        if hr < 0 {
            return None;
        }
        let address = root as u64;
        let plan = [plan[0], plan[1], address as u32, (address >> 32) as u32];
        if unsafe { set_private(variant, PHASED_PLAN, &plan) } < 0 {
            unsafe { release(variant) };
            return None;
        }
        Some((PHASED_VARIANT + groups, variant))
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

unsafe fn phased_dispatch(list: *mut c_void, pso: *mut c_void, x: u32, bind: Bind) -> bool {
    if !matches!(unsafe { crate::fill::list_type(list) }, 0 | 2)
        || crate::cl2_state::predication_of(list).is_some()
    {
        return false;
    }
    let root = crate::cl2_state::compute_root(list).unwrap_or(0);
    let Some(index) = (unsafe { phased_root(root as *mut c_void) }) else {
        return false;
    };
    let mut plan = [0u32; 4];
    if !unsafe { get_private(pso, PHASED_PLAN, plan.as_mut_ptr().cast(), 16) } {
        return false;
    }
    let [bytes, rounds, root_lo, root_hi] = plan;
    if root as u64 != (u64::from(root_lo) | (u64::from(root_hi) << 32)) {
        return false;
    }
    let Some(scratch) = (unsafe { crate::gws_scratch::begin(list, pso, bytes, index + 1) }) else {
        return false;
    };
    let constant: unsafe extern "system" fn(*mut c_void, u32, u32, u32) =
        unsafe { core::mem::transmute(slot(list, 33)) };
    let dispatch: Dispatch = unsafe { core::mem::transmute(slot(list, 14)) };
    let barrier: unsafe extern "system" fn(*mut c_void, u32, *const u64) =
        unsafe { core::mem::transmute(slot(list, 26)) };
    let uav_barrier = [2u64, 0, 0, 0];
    unsafe {
        bind(list, pso);
        barrier(list, 1, uav_barrier.as_ptr());
        for _ in 0..rounds {
            for selected in 0..x {
                constant(list, index + 2, selected, 0);
                dispatch(list, x, 1, 1);
                barrier(list, 1, uav_barrier.as_ptr());
            }
        }
        crate::gws_scratch::end(list, scratch);
    }
    true
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
        return;
    }
    if groups.contains(&0) {
        return;
    }
    if let [x @ 2..=8, 1, 1] = groups {
        let id = PHASED_VARIANT + x;
        if variants & (1 << id) != 0 {
            let variant = unsafe { get_variant(pso, id) };
            if !variant.is_null() {
                let ran = unsafe { phased_dispatch(list, variant, x, bind) };
                if ran {
                    unsafe { bind(list, pso) };
                }
                unsafe { release(variant) };
                if ran {
                    return;
                }
            }
        }
    }
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
    if !ACTIVE.load(Acquire) {
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
    if ACTIVE.load(Acquire) {
        let pso = crate::cl2_state::bound_pso(list).unwrap_or(0) as *mut c_void;
        if unsafe { mask(pso) } != 0 {
            if SKIPPED.fetch_add(1, Relaxed) < 16 {
                record(
                    "d3d12_xs",
                    "gws_dispatch",
                    "cooperative GWS ExecuteIndirect is unsupported; skipped",
                );
            }
            return;
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
}

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
