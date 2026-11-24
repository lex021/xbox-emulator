use crate::console_slots_dev2::{family, Family};
use crate::mesh_pso::Guid;
use compat_common::record;
use core::ffi::c_void;
use core::sync::atomic::{AtomicUsize, Ordering};
use std::collections::HashMap;
use std::sync::Mutex;

pub const DEV_CREATE_ROOT_SIGNATURE: usize = 16;
pub const DEV_CREATE_PIPELINE_STATE: usize = 86;
pub const CL_DISPATCH_MESH_X: usize = 98;
pub const CL_DISPATCH_MESH: usize = 127;
pub const FORCE_MEMORY_BASED_ABI: u32 = 0x0100_0000;
pub const RTS0_FLAGS_OFFSET: usize = 20;
pub const LIST_TYPE_BUNDLE: u32 = 1;
const E_INVALIDARG: i32 = 0x8007_0057u32 as i32;
const DXGI_ERROR_MORE_DATA: i32 = 0x887a_0003u32 as i32;
const MAX_ROOT_DATA: usize = 4 * 1024 * 1024;
const IID_ROOT_SIGNATURE: Guid = Guid(0xc54a6b66, 0x72df, 0x4ee8, [0x8b, 0xe5, 0xa9, 0x46, 0xa1, 0x42, 0x92, 0x14]);
const CONSOLE_ROOT_DATA: Guid = Guid(0x64f4278c, 0x72c7, 0x4f3d, [0x8e, 0xba, 0x1d, 0x4c, 0xc9, 0x4a, 0x5e, 0x82]);

static PREV_RS: AtomicUsize = AtomicUsize::new(0);
static RS_FLAGS: Mutex<Option<HashMap<usize, u32>>> = Mutex::new(None);
static LOG: AtomicUsize = AtomicUsize::new(0);

fn once(n: usize) -> bool {
    LOG.fetch_add(1, Ordering::Relaxed) < n
}

pub fn device_plan(fam: Family) -> Vec<(usize, usize)> {
    let mut p = vec![(DEV_CREATE_ROOT_SIGNATURE, create_root_signature as usize)];
    if fam == Family::Scarlett {
        p.push((DEV_CREATE_PIPELINE_STATE, create_pipeline_state as usize));
    }
    p
}

pub fn list_plan(fam: Family) -> Vec<(usize, usize)> {
    match fam {
        Family::Scarlett => vec![(CL_DISPATCH_MESH_X, dispatch_mesh_x as usize), (CL_DISPATCH_MESH, dispatch_mesh as usize)],
        Family::XboxOne => Vec::new(),
    }
}

fn device_family() -> Family {
    family(compat_common::gdk::identity::device_type_value())
}

pub fn install_device_for(v: &mut [usize], fam: Family) -> usize {
    let mut n = 0;
    for (s, f) in device_plan(fam) {
        if s >= v.len() {
            continue;
        }
        if s == DEV_CREATE_ROOT_SIGNATURE {
            if v[s] == f {
                continue;
            }
            PREV_RS.store(v[s], Ordering::Relaxed);
        }
        v[s] = f;
        n += 1;
    }
    n
}

pub fn install_device(v: &mut [usize]) -> usize {
    let fam = device_family();
    let n = install_device_for(v, fam);
    record("d3d12_xs", "console_slots_mesh", &format!("{n} device slots installed for the {fam:?} driver layout"));
    n
}

pub fn install_cmdlist_for(vt: &[AtomicUsize], fam: Family) -> usize {
    let mut n = 0;
    for (s, f) in list_plan(fam) {
        if s < vt.len() {
            vt[s].store(f, Ordering::Relaxed);
            n += 1;
        }
    }
    n
}

pub fn install_cmdlist(vt: &[AtomicUsize]) -> usize {
    let fam = device_family();
    let n = install_cmdlist_for(vt, fam);
    record("d3d12_xs", "console_slots_mesh", &format!("{n} command-list slots installed for the {fam:?} driver layout"));
    n
}

pub fn rts0_range(blob: &[u8]) -> Option<(usize, usize)> {
    if blob.get(0..4) == Some(b"DXBC") {
        let c = tombox_xshader::Dxbc::parse(blob).ok()?;
        let ch = c.chunk(b"RTS0")?;
        Some((ch.offset, ch.size))
    } else {
        Some((0, blob.len()))
    }
}

pub fn console_root_flags(blob: &[u8]) -> Option<u32> {
    let (o, n) = rts0_range(blob)?;
    let body = blob.get(o..o + n)?;
    tombox_xshader::meta::parse_rts0(body).ok().map(|r| r.flags)
}

pub fn pc_root_blob(blob: &[u8]) -> Option<Vec<u8>> {
    crate::rs_xbox::pc_blob(blob)
}

pub fn root_flags(rs: usize) -> Option<u32> {
    RS_FLAGS.lock().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(|m| m.get(&rs).copied())
}

pub fn note_root_flags(rs: usize, flags: u32) {
    RS_FLAGS.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).insert(rs, flags);
}

unsafe fn store_root_data(unknown: *mut c_void, blob: &[u8]) {
    let Some((offset, size)) = rts0_range(blob) else { return };
    if !(RTS0_FLAGS_OFFSET + 4..=MAX_ROOT_DATA).contains(&size) {
        return;
    }
    let body = &blob[offset..offset + size];
    let vt = *(unknown as *const *const usize);
    let query: unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> i32 =
        core::mem::transmute(*vt);
    let mut root = core::ptr::null_mut();
    if query(unknown, &IID_ROOT_SIGNATURE, &mut root) < 0 || root.is_null() {
        return;
    }
    let vt = *(root as *const *const usize);
    let set: unsafe extern "system" fn(*mut c_void, *const Guid, u32, *const u8) -> i32 =
        core::mem::transmute(*vt.add(4));
    let release: unsafe extern "system" fn(*mut c_void) -> u32 = core::mem::transmute(*vt.add(2));
    let hr = set(root, &CONSOLE_ROOT_DATA, size as u32, body.as_ptr());
    release(root);
    if hr < 0 && once(4) {
        record("d3d12_xs", "console_slots_mesh", &format!("could not retain console root signature data: hr={hr:#010x}"));
    }
}

pub unsafe fn root_data(root: *mut c_void) -> Option<Vec<u8>> {
    if root.is_null() {
        return None;
    }
    let vt = *(root as *const *const usize);
    let get: unsafe extern "system" fn(*mut c_void, *const Guid, *mut u32, *mut u8) -> i32 =
        core::mem::transmute(*vt.add(3));
    let mut size = 0;
    let hr = get(root, &CONSOLE_ROOT_DATA, &mut size, core::ptr::null_mut());
    if (hr < 0 && hr != DXGI_ERROR_MORE_DATA) || !(RTS0_FLAGS_OFFSET + 4..=MAX_ROOT_DATA).contains(&(size as usize)) {
        return None;
    }
    let mut body = vec![0; size as usize];
    let hr = get(root, &CONSOLE_ROOT_DATA, &mut size, body.as_mut_ptr());
    if hr < 0 || size as usize != body.len() {
        return None;
    }
    Some(body)
}

type RsFn = unsafe extern "system" fn(*mut c_void, u32, *const u8, usize, *const c_void, *mut *mut c_void) -> i32;

unsafe extern "system" fn create_root_signature(this: *mut c_void, mask: u32, blob: *const u8, len: usize, riid: *const c_void, ppv: *mut *mut c_void) -> i32 {
    let prev = PREV_RS.load(Ordering::Relaxed);
    if prev == 0 {
        return E_INVALIDARG;
    }
    let f: RsFn = unsafe { core::mem::transmute(prev) };
    if blob.is_null() || len == 0 {
        return unsafe { f(this, mask, blob, len, riid, ppv) };
    }
    let b = unsafe { core::slice::from_raw_parts(blob, len) };
    let flags = console_root_flags(b);
    let pc = pc_root_blob(b);
    let (ptr, size) = pc.as_ref().map_or((blob, len), |p| (p.as_ptr(), p.len()));
    let extended = flags.filter(|&fl| crate::rs_xbox::XboxRoot(fl).internal_xdxr())
        .and_then(|_| unsafe {
            crate::global_bind::create_root(ptr, size, riid, ppv, |p, n| f(this, mask, p, n, riid, ppv))
        });
    let hr = extended.unwrap_or_else(|| unsafe { f(this, mask, ptr, size, riid, ppv) });
    if hr >= 0 && !ppv.is_null() && !unsafe { *ppv }.is_null() {
        if let Some(fl) = flags {
            unsafe { store_root_data(*ppv, b) };
            note_root_flags(unsafe { *ppv } as usize, fl);
            if pc.is_some() && once(4) {
                record("d3d12_xs", "console_slots_mesh", &format!("root signature {:p}: console-only flag bits {:#x} (XBOX_FORCE_MEMORY_BASED_ABI 0x01000000, XBOX_FORCE_COMPUTE_2_SPILL_AREA 0x02000000, driver-discarded 0x80000000) cleared for the PC runtime, which rejects them; flags {fl:#x} kept for PSO validation", unsafe { *ppv }, crate::rs_xbox::XboxRoot(fl).xbox_bits()));
            }
        }
    }
    hr
}

unsafe extern "system" fn create_pipeline_state(this: *mut c_void, desc: *const [u64; 2], riid: *const c_void, ppv: *mut *mut c_void) -> i32 {
    if this.is_null() {
        record("d3d12_xs", "D3d12Device2_CreatePipelineState", "pDevice2 is NULL.");
        return E_INVALIDARG;
    }
    if desc.is_null() {
        record("d3d12_xs", "D3d12Device2_CreatePipelineState", "pStreamDesc is NULL.");
        return E_INVALIDARG;
    }
    if ppv.is_null() {
        record("d3d12_xs", "D3d12Device2_CreatePipelineState", "ppPipelineState is NULL.");
        return E_INVALIDARG;
    }
    let real = unsafe { crate::device_wrap::real_device(this) };
    unsafe { crate::mesh_pso::create_pipeline_state(real, desc, riid, ppv) }
}

pub const NO_PSO: &str = "No pipeline state object is currently bound to the pipeline.";
pub const NO_AS_PSO: &str = "DispatchMeshX_AS can't be called when the PSO with an amp shader isn't set";

static AS_PSOS: Mutex<Option<std::collections::HashSet<usize>>> = Mutex::new(None);

pub fn note_as_pso(pso: usize) {
    AS_PSOS.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(Default::default).insert(pso);
}

pub fn is_as_pso(pso: usize) -> bool {
    AS_PSOS.lock().unwrap_or_else(|e| e.into_inner()).as_ref().is_some_and(|s| s.contains(&pso))
}

pub fn bound(list: *mut c_void) -> Option<bool> {
    crate::cl2_state::bound_pso(list).filter(|p| *p != 0).map(is_as_pso)
}

pub fn dispatch_mesh_x_errors(flags: u64, args: u64, payload: u64, bundle: bool, pso: Option<bool>) -> Vec<&'static str> {
    let mut e = Vec::new();
    if !bundle && pso.is_none() {
        e.push(NO_PSO);
    }
    if flags > 1 {
        e.push("flags can be only 0 or 1");
    }
    if flags == 1 {
        if args & 0xff != 0 {
            e.push("indirectArgsPointer has to be 256B aligned with AS");
        }
        if args >= 1 << 40 {
            e.push("indirectArgsPointer can't have more than 40 bits set in it");
        }
    } else if args != 0 {
        e.push("indirectArgsPointer has to be 0 when amp shader is not used");
    }
    if flags == 1 {
        if payload & 0xff != 0 {
            e.push("payloadPointer has to be 256B aligned for AS");
        }
    } else if payload & 0xf != 0 {
        e.push("payloadPointer has to be 16B aligned for MS");
    }
    if payload >= 1 << 40 {
        e.push("payloadPointer can't have more than 40 bits set in it");
    }
    if flags == 1 && bundle {
        e.push("amplificationShader call can't be used inside a draw bundle because PSO inheritance is not implemented");
    }
    e
}

#[derive(Debug, PartialEq, Eq)]
pub enum MeshXPlan {
    Rejected(Vec<&'static str>),
    Forward(u32),
    MsMemoryPayload,
    AsMemoryRings,
}

pub fn plan_dispatch_mesh_x(count: u32, flags: u64, args: u64, payload: u64, bundle: bool, pso: Option<bool>) -> MeshXPlan {
    let e = dispatch_mesh_x_errors(flags, args, payload, bundle, pso);
    if !e.is_empty() {
        return MeshXPlan::Rejected(e);
    }
    if flags == 1 && pso != Some(true) {
        return MeshXPlan::Rejected(vec![NO_AS_PSO]);
    }
    match (flags, payload) {
        (0, 0) => MeshXPlan::Forward(count),
        (0, _) => MeshXPlan::MsMemoryPayload,
        _ => MeshXPlan::AsMemoryRings,
    }
}

unsafe extern "system" fn dispatch_mesh_x(this: *mut c_void, count: u32, flags: u64, args: u64, payload: u64) {
    let bundle = !this.is_null() && unsafe { crate::fill::list_type(this) } == LIST_TYPE_BUNDLE;
    let pso = if this.is_null() { None } else { bound(this) };
    match plan_dispatch_mesh_x(count, flags, args, payload, bundle, pso) {
        MeshXPlan::Rejected(e) => {
            for m in e {
                record("d3d12_xs", "DispatchMeshX", &format!("{m}: not recorded, as the console debug layer does"));
            }
        }
        MeshXPlan::Forward(n) => unsafe { crate::mesh_pso::dispatch_mesh(this, n, 1, 1) },
        MeshXPlan::MsMemoryPayload => record("d3d12_xs", "DispatchMeshX", &format!("mesh-only dispatch of {count} groups with payloadPointer {payload:#x}: the console mesh shader payload register is not decoded, dispatch not executed")),
        MeshXPlan::AsMemoryRings => record("d3d12_xs", "DispatchMeshX", &format!("amplification dispatch of {count} groups into app rings (args {args:#x}, payload {payload:#x}): the console AS writes its rings without launching mesh groups and PC D3D12 has no such dispatch, not executed")),
    }
}

pub fn dispatch_mesh_errors(bundle: bool, pso: Option<bool>) -> Option<&'static str> {
    (!bundle && pso.is_none()).then_some(NO_PSO)
}

unsafe extern "system" fn dispatch_mesh(this: *mut c_void, x: u32, y: u32, z: u32) {
    if this.is_null() {
        return;
    }
    let bundle = unsafe { crate::fill::list_type(this) } == LIST_TYPE_BUNDLE;
    if let Some(m) = dispatch_mesh_errors(bundle, bound(this)) {
        record("d3d12_xs", "DispatchMesh", &format!("{m}: not recorded, as the console debug layer does"));
        return;
    }
    unsafe { crate::mesh_pso::dispatch_mesh(this, x, y, z) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rts0(flags: u32) -> Vec<u8> {
        let body: Vec<u8> = [1u32, 0, 24, 0, 24, flags].iter().flat_map(|x| x.to_le_bytes()).collect();
        crate::device_wrap::wrap_root_signature(&body)
    }

    #[test]
    fn scarlett_plans_wire_mesh_slots_and_xbox_one_does_not() {
        let d = device_plan(Family::Scarlett);
        assert_eq!(d, vec![(16, create_root_signature as usize), (86, create_pipeline_state as usize)]);
        assert_eq!(device_plan(Family::XboxOne), vec![(16, create_root_signature as usize)]);
        assert_eq!(list_plan(Family::Scarlett), vec![(98, dispatch_mesh_x as usize), (127, dispatch_mesh as usize)]);
        assert!(list_plan(Family::XboxOne).is_empty());
        let vt: Vec<AtomicUsize> = (0..128).map(|_| AtomicUsize::new(7)).collect();
        assert_eq!(install_cmdlist_for(&vt, Family::XboxOne), 0);
        assert_eq!(vt[98].load(Ordering::Relaxed), 7);
        assert_eq!(install_cmdlist_for(&vt, Family::Scarlett), 2);
        assert_eq!(vt[98].load(Ordering::Relaxed), dispatch_mesh_x as usize);
        assert_eq!(vt[127].load(Ordering::Relaxed), dispatch_mesh as usize);
    }

    #[test]
    fn device_install_chains_root_signature_once() {
        let mut v = vec![5usize; 160];
        assert_eq!(install_device_for(&mut v, Family::Scarlett), 2);
        assert_eq!(PREV_RS.load(Ordering::Relaxed), 5);
        assert_eq!(v[86], create_pipeline_state as usize);
        assert_eq!(install_device_for(&mut v, Family::XboxOne), 0);
        assert_eq!(PREV_RS.load(Ordering::Relaxed), 5);
    }

    #[test]
    fn force_memory_based_abi_is_cleared_for_pc_and_kept_for_checks() {
        let plain = rts0(0x1);
        assert_eq!(console_root_flags(&plain), Some(1));
        assert!(pc_root_blob(&plain).is_none());
        let forced = rts0(FORCE_MEMORY_BASED_ABI | 0x1);
        assert_eq!(console_root_flags(&forced), Some(0x0100_0001));
        let pc = pc_root_blob(&forced).unwrap();
        assert_eq!(console_root_flags(&pc), Some(1));
        assert_eq!(pc.len(), forced.len());
        let body: Vec<u8> = [1u32, 0, 24, 0, 24, FORCE_MEMORY_BASED_ABI].iter().flat_map(|x| x.to_le_bytes()).collect();
        assert_eq!(console_root_flags(&pc_root_blob(&body).unwrap()), Some(0));
        note_root_flags(0xabc0, 0x0100_0000);
        assert_eq!(root_flags(0xabc0), Some(0x0100_0000));
        assert_eq!(root_flags(0xabc8), None);
    }

    #[test]
    fn original_root_data_is_stored_on_the_queried_interface() {
        #[repr(C)]
        struct Root {
            vtable: *const usize,
            body: Vec<u8>,
            releases: u32,
        }
        #[repr(C)]
        struct Unknown {
            vtable: *const usize,
            root: *mut Root,
        }
        fn same_guid(a: &Guid, b: &Guid) -> bool {
            (a.0, a.1, a.2, a.3) == (b.0, b.1, b.2, b.3)
        }
        unsafe extern "system" fn query(this: *mut c_void, iid: *const Guid, out: *mut *mut c_void) -> i32 {
            if !same_guid(&*iid, &IID_ROOT_SIGNATURE) {
                return E_INVALIDARG;
            }
            *out = (*(this as *mut Unknown)).root.cast();
            0
        }
        unsafe extern "system" fn release(this: *mut c_void) -> u32 {
            (*(this as *mut Root)).releases += 1;
            1
        }
        unsafe extern "system" fn get(this: *mut c_void, tag: *const Guid, size: *mut u32, data: *mut u8) -> i32 {
            let body = &(*(this as *mut Root)).body;
            if !same_guid(&*tag, &CONSOLE_ROOT_DATA) || body.is_empty() {
                return E_INVALIDARG;
            }
            let capacity = *size as usize;
            *size = body.len() as u32;
            if data.is_null() || capacity < body.len() {
                return DXGI_ERROR_MORE_DATA;
            }
            core::ptr::copy_nonoverlapping(body.as_ptr(), data, body.len());
            0
        }
        unsafe extern "system" fn set(this: *mut c_void, tag: *const Guid, size: u32, data: *const u8) -> i32 {
            if !same_guid(&*tag, &CONSOLE_ROOT_DATA) {
                return E_INVALIDARG;
            }
            (*(this as *mut Root)).body = core::slice::from_raw_parts(data, size as usize).to_vec();
            0
        }
        let root_vtable = [0, 0, release as usize, get as usize, set as usize];
        let mut root = Root { vtable: root_vtable.as_ptr(), body: Vec::new(), releases: 0 };
        let root_ptr = &raw mut root;
        let unknown_vtable = [query as usize, 0, 0];
        let mut unknown = Unknown { vtable: unknown_vtable.as_ptr(), root: root_ptr };
        assert_eq!(unsafe { root_data(root_ptr.cast()) }, None);
        let blob = rts0(FORCE_MEMORY_BASED_ABI);
        let (offset, size) = rts0_range(&blob).unwrap();
        unsafe { store_root_data((&mut unknown as *mut Unknown).cast(), &blob) };
        assert_eq!(unsafe { root_data(root_ptr.cast()) }.as_deref(), Some(&blob[offset..offset + size]));
        assert_eq!(root.releases, 1);
    }

    #[test]
    fn dispatch_mesh_x_follows_the_debug_umd_checks() {
        let ms = Some(false);
        let asp = Some(true);
        assert_eq!(plan_dispatch_mesh_x(3, 0, 0, 0, false, ms), MeshXPlan::Forward(3));
        assert_eq!(plan_dispatch_mesh_x(3, 0, 0, 0x1000, false, ms), MeshXPlan::MsMemoryPayload);
        assert_eq!(plan_dispatch_mesh_x(3, 1, 0x100, 0x200, false, asp), MeshXPlan::AsMemoryRings);
        assert_eq!(plan_dispatch_mesh_x(3, 1, 0x100, 0x200, false, ms), MeshXPlan::Rejected(vec![NO_AS_PSO]));
        assert_eq!(plan_dispatch_mesh_x(3, 0, 0, 0, false, None), MeshXPlan::Rejected(vec![NO_PSO]));
        assert_eq!(plan_dispatch_mesh_x(3, 0, 0, 0, true, None), MeshXPlan::Forward(3));
        assert_eq!(dispatch_mesh_x_errors(2, 0, 0, false, ms), vec!["flags can be only 0 or 1"]);
        assert_eq!(dispatch_mesh_x_errors(0, 0x100, 0, false, ms), vec!["indirectArgsPointer has to be 0 when amp shader is not used"]);
        assert_eq!(dispatch_mesh_x_errors(1, 0x180, 0x100, false, asp), vec!["indirectArgsPointer has to be 256B aligned with AS"]);
        assert_eq!(dispatch_mesh_x_errors(1, 1 << 40, 0x100, false, asp), vec!["indirectArgsPointer can't have more than 40 bits set in it"]);
        assert_eq!(dispatch_mesh_x_errors(1, 0, 0x110, false, asp), vec!["payloadPointer has to be 256B aligned for AS"]);
        assert_eq!(dispatch_mesh_x_errors(0, 0, 0x18, false, ms), vec!["payloadPointer has to be 16B aligned for MS"]);
        assert!(dispatch_mesh_x_errors(0, 0, 0x10, false, ms).is_empty());
        assert_eq!(dispatch_mesh_x_errors(0, 0, 1 << 40, false, ms), vec!["payloadPointer can't have more than 40 bits set in it"]);
        assert_eq!(dispatch_mesh_x_errors(1, 0, 0, true, None), vec!["amplificationShader call can't be used inside a draw bundle because PSO inheritance is not implemented"]);
        assert!(dispatch_mesh_x_errors(0, 0, 0, true, None).is_empty());
        assert!(matches!(plan_dispatch_mesh_x(1, 3, 1, 1, false, None), MeshXPlan::Rejected(e) if e.len() == 4));
        assert_eq!(dispatch_mesh_errors(false, None), Some(NO_PSO));
        assert_eq!(dispatch_mesh_errors(true, None), None);
        assert_eq!(dispatch_mesh_errors(false, ms), None);
    }

    #[test]
    fn amplification_psos_are_remembered() {
        assert!(!is_as_pso(0x7770));
        note_as_pso(0x7770);
        assert!(is_as_pso(0x7770));
        assert!(!is_as_pso(0x7778));
    }

    #[test]
    fn create_pipeline_state_null_arguments() {
        let d = [0u64; 2];
        let mut p: *mut c_void = core::ptr::null_mut();
        unsafe {
            assert_eq!(create_pipeline_state(core::ptr::null_mut(), &d, core::ptr::null(), &mut p), E_INVALIDARG);
            assert_eq!(create_pipeline_state(8 as *mut c_void, core::ptr::null(), core::ptr::null(), &mut p), E_INVALIDARG);
            assert_eq!(create_pipeline_state(8 as *mut c_void, &d, core::ptr::null(), core::ptr::null_mut()), E_INVALIDARG);
        }
    }
}
