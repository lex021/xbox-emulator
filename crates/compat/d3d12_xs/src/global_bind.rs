use compat_common::record;
use core::ffi::c_void;
use core::ptr::null_mut;
use std::sync::OnceLock;
use tombox_xshader::global_abi::{
    window_cb, Window, TB_GLOBAL_CBV, TB_GLOBAL_FAULT_BYTES, TB_GLOBAL_FAULT_UAV,
    TB_GLOBAL_ROOT_DWORDS, TB_GLOBAL_ROOT_VAS, TB_GLOBAL_SPACE, TB_GLOBAL_WINDOWS,
};

use crate::device_wrap::Guid;
use crate::gds::{PARAM_SIZE, PARAM_UAV, ROOT_DWORD_LIMIT, ROOT_FLAG_LOCAL};
use crate::gws_scratch::{self, Resource, UAV};

const ROOT_LAYOUT: Guid = Guid {
    d1: 0xb7e35c2e, d2: 0xcbb6, d3: 0x4ab6, d4: [0x95, 0x87, 0x15, 0x2a, 0xeb, 0x03, 0x4d, 0xb4],
};
const SHADER_ROOT: Guid = Guid { d1: 0xb7e35c2f, ..ROOT_LAYOUT };
const IID_PSO: Guid = Guid {
    d1: 0x765a30f3, d2: 0xf624, d3: 0x4c6f, d4: [0xa8, 0x28, 0xac, 0xe9, 0x48, 0x62, 0x24, 0x45],
};
const IID_ROOT: Guid = Guid {
    d1: 0xc54a6b66, d2: 0x72df, d3: 0x4ee8, d4: [0x8b, 0xe5, 0xa9, 0x46, 0xa1, 0x42, 0x92, 0x14],
};
const E_FAIL: i32 = 0x80004005u32 as i32;
const PARAM_CBV: u32 = 2;
const XBOX_COMPUTE_UAV: usize = 45;
const PC_COMPUTE_CBV: usize = 37;
const PC_COMPUTE_UAV: usize = 41;
type Create = unsafe extern "system" fn(*mut c_void, *const u8, *const Guid, *mut *mut c_void) -> i32;

pub(crate) fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var("TOMBOX_GLOBAL_COMPUTE").as_deref() == Ok("1"))
}

pub(crate) fn refusal(needs_global: bool) -> Option<String> {
    needs_global.then(|| format!("shader needs a runtime table for 64-bit GPU VA access in space {TB_GLOBAL_SPACE}"))
}

unsafe fn slot(object: *mut c_void, index: usize) -> usize {
    unsafe { *(*object.cast::<*const usize>()).add(index) }
}

unsafe fn set_private<const N: usize>(object: *mut c_void, key: &Guid, data: &[u64; N]) -> bool {
    let set: unsafe extern "system" fn(*mut c_void, *const Guid, u32, *const c_void) -> i32 =
        unsafe { core::mem::transmute(slot(object, 4)) };
    unsafe { set(object, key, (N * 8) as u32, data.as_ptr().cast()) >= 0 }
}

unsafe fn get_private<const N: usize>(object: *mut c_void, key: &Guid) -> Option<[u64; N]> {
    if object.is_null() {
        return None;
    }
    let get: unsafe extern "system" fn(*mut c_void, *const Guid, *mut u32, *mut c_void) -> i32 =
        unsafe { core::mem::transmute(slot(object, 3)) };
    let mut data = [0; N];
    let len = (N * 8) as u32;
    let mut size = len;
    (unsafe { get(object, key, &mut size, data.as_mut_ptr().cast()) } >= 0 && size == len).then_some(data)
}

fn descriptor(kind: u32, register: u32) -> [u8; PARAM_SIZE] {
    let mut parameter = [0; PARAM_SIZE];
    parameter[..4].copy_from_slice(&kind.to_le_bytes());
    parameter[8..12].copy_from_slice(&register.to_le_bytes());
    parameter[12..16].copy_from_slice(&TB_GLOBAL_SPACE.to_le_bytes());
    // Each binding is supplied anew when recording a dispatch.
    parameter[16..20].copy_from_slice(&2u32.to_le_bytes()); // DATA_VOLATILE
    parameter
}

fn append_parameters(params: &[u8], flags: u32) -> Option<(Vec<u8>, u32)> {
    if flags & ROOT_FLAG_LOCAL != 0 || crate::gds::root_cost(params) + TB_GLOBAL_ROOT_DWORDS > ROOT_DWORD_LIMIT {
        return None;
    }
    let base = (params.len() / PARAM_SIZE) as u32;
    let mut extended = params.to_vec();
    extended.extend_from_slice(&descriptor(PARAM_CBV, TB_GLOBAL_CBV));
    for register in 0..=TB_GLOBAL_FAULT_UAV {
        extended.extend_from_slice(&descriptor(PARAM_UAV, register));
    }
    Some((extended, base))
}

pub(crate) unsafe fn create_root(
    bytes: *const u8, len: usize, riid: *const c_void, out: *mut *mut c_void,
    make: impl FnOnce(*const u8, usize) -> i32,
) -> Option<i32> {
    if !enabled() || out.is_null() || riid.is_null() || unsafe { &*riid.cast::<Guid>() } != &IID_ROOT {
        return None;
    }
    let (b, base) = unsafe { crate::gds::patch_root_parameters(bytes, len, append_parameters) }?;
    let hr = make(b.as_ptr(), b.len());
    if hr < 0 || unsafe { (*out).is_null() } {
        record("d3d12_xs", "global_bind", &format!("extended root signature rejected: {hr:#010x}"));
        return None;
    }
    if unsafe { !set_private(*out, &ROOT_LAYOUT, &[u64::from(base)]) } {
        let proposed = unsafe { out.replace(null_mut()) };
        drop(Resource(proposed as usize));
        return None;
    }
    record("d3d12_xs", "global_bind", &format!("root {:p} global bindings appended at parameter {base}", unsafe { *out }));
    Some(hr)
}

pub(crate) unsafe fn try_create_compute(
    create: Create, device: *mut c_void, desc: &[u8], code: *const u8, len: usize,
    riid: *const Guid, out: *mut *mut c_void,
) -> Option<i32> {
    if !enabled() || code.is_null() || !(32..=0x40_0000).contains(&len)
        || out.is_null() || riid.is_null() || unsafe { &*riid } != &IID_PSO
    {
        return None;
    }
    let root = u64::from_le_bytes(desc[..8].try_into().unwrap()) as *mut c_void;
    let [base] = unsafe { get_private(root, &ROOT_LAYOUT) }?;
    let blob = unsafe { core::slice::from_raw_parts(code, len) };
    let hash = crate::shader_xlat::fnv1a(blob);
    let source_root = unsafe { crate::console_slots_mesh::root_data(root) };
    let translated = match crate::shader_xlat::compile_global_compute(blob, source_root.as_deref()) {
        Ok(code) => code,
        Err(e) => {
            record("d3d12_xs", "global_bind", &format!("compute {hash:016x} translation: {e}"));
            return None;
        }
    };
    let mut pc = desc.to_vec();
    pc[8..16].copy_from_slice(&(translated.as_ptr() as u64).to_le_bytes());
    pc[16..24].copy_from_slice(&(translated.len() as u64).to_le_bytes());
    pc[0x20..0x30].fill(0);
    let hr = unsafe {
        crate::pso_lib::compute(device, &pc, riid, out, &|d| create(device, d, riid, out))
    };
    if hr >= 0 && !unsafe { (*out).is_null() } {
        if !unsafe { set_private(*out, &SHADER_ROOT, &[root as u64, base, hash]) } {
            let proposed = unsafe { out.replace(null_mut()) };
            drop(Resource(proposed as usize));
            return Some(E_FAIL);
        }
        crate::dred_marks::note(unsafe { *out }, unsafe { crate::dred_marks::compute_hashes(desc.as_ptr()) }, false);
    }
    record("d3d12_xs", "global_bind", &format!("compute {hash:016x}: CreateComputePipelineState {hr:#010x}"));
    Some(hr)
}

#[derive(Clone, Copy)]
struct BoundWindow {
    guest: Window,
    host: u64,
    resource: usize,
}

unsafe fn bound_window(va: u64) -> Result<BoundWindow, String> {
    let (entry, _) = crate::gpu_va::lookup_buffer(va).ok_or_else(|| format!("no buffer for root UAV {va:#x}"))?;
    if crate::placed_offset::bytes(entry.res) != 0 {
        return Err(format!("root UAV {va:#x} uses an adjusted placed resource"));
    }
    if !matches!(crate::gpu_va::state(entry.res), 0 | UAV) {
        return Err(format!("root UAV {va:#x} buffer state is {:#x}", crate::gpu_va::state(entry.res)));
    }
    let res = entry.res as *mut c_void;
    let desc = unsafe { crate::device_ext_a::resource_desc(res) };
    let flags = u32::from_le_bytes(desc[0x30..0x34].try_into().unwrap());
    let width = u64::from_le_bytes(desc[0x10..0x18].try_into().unwrap());
    if flags & 4 == 0 || width < entry.len {
        return Err(format!("root UAV {va:#x} buffer has incompatible flags or size"));
    }
    let size = u32::try_from(entry.len).map_err(|_| format!("root UAV {va:#x} buffer exceeds the 32-bit window size"))?;
    let get_va: unsafe extern "system" fn(*mut c_void) -> u64 = unsafe { core::mem::transmute(slot(res, 11)) };
    let host = unsafe { get_va(res) };
    if host == 0 || host & 3 != 0 || entry.start & 3 != 0 || size < 4 {
        return Err(format!("root UAV {va:#x} has no aligned host buffer"));
    }
    Ok(BoundWindow { guest: Window { base: entry.start, size }, host, resource: entry.res })
}

unsafe fn table_upload(
    list: *mut c_void, windows: &[BoundWindow], bindings: &crate::cl2_state::ComputeBindings,
) -> Option<(Resource, u64)> {
    let guest: Vec<_> = windows.iter().map(|w| w.guest).collect();
    let mut table = window_cb(&guest, [0; 2]).to_vec();
    table.resize(table.len() + TB_GLOBAL_ROOT_VAS * 4, 0);
    let base = table.len() - TB_GLOBAL_ROOT_VAS * 4;
    for (index, _, va) in bindings.views() {
        if (index as usize) < TB_GLOBAL_ROOT_VAS {
            table[base + index as usize * 4] = va as u32;
            table[base + index as usize * 4 + 1] = (va >> 32) as u32;
        }
    }
    let table_bytes = table.len() * 4;
    let (res, va) = unsafe { crate::gds::create_buffer(list, table_bytes.next_multiple_of(256) as u64, 2, 0, 0xac3) }?;
    let upload = Resource(res);
    let res = res as *mut c_void;
    let map: unsafe extern "system" fn(*mut c_void, u32, *const usize, *mut *mut u32) -> i32 =
        unsafe { core::mem::transmute(slot(res, 8)) };
    let unmap: unsafe extern "system" fn(*mut c_void, u32, *const usize) =
        unsafe { core::mem::transmute(slot(res, 9)) };
    let mut data = null_mut();
    if unsafe { map(res, 0, [0usize; 2].as_ptr(), &mut data) } < 0 || data.is_null() {
        return None;
    }
    unsafe {
        data.copy_from_nonoverlapping(table.as_ptr(), table.len());
        unmap(res, 0, [0usize, table_bytes].as_ptr());
    }
    Some((upload, va))
}

unsafe fn active_shader(list: *mut c_void) -> Option<(*mut c_void, [u64; 3])> {
    if !enabled() {
        return None;
    }
    let pso = crate::cl2_state::bound_pso(list)? as *mut c_void;
    Some((pso, unsafe { get_private(pso, &SHADER_ROOT) }?))
}

fn fail(list: *mut c_void, hash: u64, error: impl std::fmt::Display) -> bool {
    record("d3d12_xs", "global_bind", &format!("compute {hash:016x} on list {list:p}: {error}"));
    crate::cl2_state::note_global_error(list);
    false
}

pub(crate) unsafe fn before_indirect(list: *mut c_void) -> bool {
    match unsafe { active_shader(list) } {
        None => true,
        Some((_, [_, _, hash])) => fail(list, hash, "global windows for ExecuteIndirect are unsupported"),
    }
}

pub(crate) unsafe fn before_dispatch(list: *mut c_void) -> bool {
    let Some((pso, [expected_root, base, hash])) = (unsafe { active_shader(list) }) else { return true };
    let Some(bindings) = crate::cl2_state::compute_bindings(list) else {
        return fail(list, hash, "no compute root signature");
    };
    if bindings.root() as u64 != expected_root || !matches!(unsafe { crate::fill::list_type(list) }, 0 | 2) {
        return fail(list, hash, "unexpected root signature or command-list type");
    }
    let mut windows: Vec<BoundWindow> = Vec::new();
    for (_, kind, va) in bindings.views() {
        if kind != XBOX_COMPUTE_UAV || va == 0 {
            continue;
        }
        match unsafe { bound_window(va) } {
            Ok(w) if !windows.iter().any(|old| old.guest == w.guest) => windows.push(w),
            Ok(_) => {}
            Err(e) => return fail(list, hash, e),
        }
    }
    if windows.is_empty() || windows.len() > TB_GLOBAL_WINDOWS {
        let views: Vec<_> = bindings.views().collect();
        return fail(list, hash, format!("{} bindable root UAV windows, views={views:x?}", windows.len()));
    }
    let Some((table, table_va)) = (unsafe { table_upload(list, &windows, &bindings) }) else {
        return fail(list, hash, "could not allocate the global window table");
    };
    let Some((fault, fault_va)) = (unsafe { gws_scratch::begin(list, pso, TB_GLOBAL_FAULT_BYTES as u32) }) else {
        return fail(list, hash, "could not allocate the fault UAV");
    };
    let mut retained = vec![table];
    for window in &windows {
        let add_ref: unsafe extern "system" fn(*mut c_void) -> u32 =
            unsafe { core::mem::transmute(slot(window.resource as *mut c_void, 1)) };
        unsafe { add_ref(window.resource as *mut c_void) };
        retained.push(Resource(window.resource));
    }
    gws_scratch::retain(list, retained);
    let cbv: unsafe extern "system" fn(*mut c_void, u32, u64) =
        unsafe { core::mem::transmute(slot(list, PC_COMPUTE_CBV)) };
    let uav: unsafe extern "system" fn(*mut c_void, u32, u64) =
        unsafe { core::mem::transmute(slot(list, PC_COMPUTE_UAV)) };
    unsafe {
        crate::fill::barrier_pub(list, fault, gws_scratch::COPY_DEST, UAV);
        cbv(list, base as u32, table_va);
        for i in 0..TB_GLOBAL_WINDOWS {
            uav(list, base as u32 + 1 + i as u32, windows.get(i).map_or(fault_va, |w| w.host));
        }
        uav(list, base as u32 + 1 + TB_GLOBAL_FAULT_UAV, fault_va);
    }
    record("d3d12_xs", "global_bind", &format!("compute {hash:016x}: bound {} GPU VA windows", windows.len()));
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn global_memory_shader_is_refused_with_reason() {
        assert_eq!(refusal(false), None);
        let reason = refusal(true).unwrap();
        assert!(reason.contains("space 4000003") && reason.contains("GPU VA"), "{reason}");
    }

    #[test]
    fn appended_parameters_preserve_title_bindings_and_enforce_root_budget() {
        let existing = crate::gds::gds_param();
        let (params, base) = append_parameters(&existing, 0).unwrap();
        assert_eq!(base, 1);
        assert_eq!(&params[..PARAM_SIZE], &existing);
        assert_eq!(&params[PARAM_SIZE..2 * PARAM_SIZE], &descriptor(PARAM_CBV, TB_GLOBAL_CBV));
        for (reg, parameter) in params[2 * PARAM_SIZE..].chunks_exact(PARAM_SIZE).enumerate() {
            assert_eq!(parameter, &descriptor(PARAM_UAV, reg as u32));
        }
        assert_eq!(crate::gds::root_cost(&params), 2 + TB_GLOBAL_ROOT_DWORDS);
        assert!(append_parameters(&existing.repeat(29), 0).is_none());
        assert!(append_parameters(&existing, ROOT_FLAG_LOCAL).is_none());
    }
}

