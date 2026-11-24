//! The literal-sampler fallback translates against FSSM's physical table layout.
//! Keep the title's root object intact for other PSOs that share it.
use crate::device_wrap::Guid;
use crate::gws_scratch::Resource;
use core::ffi::c_void;
use core::sync::atomic::{AtomicBool, Ordering::Relaxed};
use tombox_xshader::meta::RootParamKind::{Constants, Descriptor, Table};

const IID_ROOT: Guid = Guid {
    d1: 0xc54a_6b66, d2: 0x72df, d3: 0x4ee8, d4: [0x8b, 0xe5, 0xa9, 0x46, 0xa1, 0x42, 0x92, 0x14],
};
const PSO_ROOT: Guid = Guid {
    d1: 0x423b_846a, d2: 0x2bf1, d3: 0x4e73, d4: [0xb3, 0x3d, 0x14, 0x67, 0x8b, 0x61, 0x1f, 0x79],
};
static ACTIVE: AtomicBool = AtomicBool::new(false);

pub(crate) fn binding_retry_error(error: &str) -> bool {
    error.contains("missing root signature for descriptor table")
        || error.contains("image_store to a non-UAV table")
        || error.contains("not a sampler descriptor (Some(Literal(")
}

pub(crate) fn translate_binding_retry(
    shader: &[u8], root: &[u8],
) -> Result<(std::sync::Arc<Vec<u8>>, Option<Vec<u8>>), String> {
    match crate::shader_xlat::translate_with_pipeline_root(shader, root, [0; 8], &crate::shader_xlat::no_link()) {
        Ok(code) => Ok((code, None)),
        Err(external) => {
            translate_variant(shader, root).map(|(code, body)| (code, Some(body)))
                .map_err(|e| format!("title root: {external}; FSSM root: {e}"))
        }
    }
}

pub(crate) fn translate_variant(shader: &[u8], root: &[u8]) -> Result<(std::sync::Arc<Vec<u8>>, Vec<u8>), String> {
    let body = variant(root, shader)?;
    let code = crate::shader_xlat::translate_compute_fssm(shader, &body).map_err(|error| error.to_string())?;
    Ok((code, body))
}

pub(crate) fn cached_literal_sampler_retry(shader: &[u8]) -> bool {
    use tombox_xshader::{Stage, pack_key};

    // The pre-translate index returns DXIL and CBV metadata without running
    // runtime_source_for. Recover the retry decision from the original shader
    // only after the host PSO rejects that DXIL. Neither translation runs DXC.
    std::panic::catch_unwind(|| {
        let Ok(ex) = tombox_xshader::extract(shader) else { return false };
        if ex.stage != Stage::Compute || ex.has_root_sig || ex.root_sig.is_some() {
            return false;
        }
        let cbv = crate::shader_xlat::runtime_mode().cbv_raw;
        let Err(error) = pack_key::translate(&ex, None, cbv) else { return false };
        pack_key::retry_compute_literal_sampler(&ex, cbv, &error).is_some_and(|retry| retry.is_ok())
    }).unwrap_or(false)
}

pub(crate) fn variant(title: &[u8], shader: &[u8]) -> Result<Vec<u8>, String> {
    let (offset, length) = crate::console_slots_mesh::rts0_range(title).ok_or("title root has no RTS0")?;
    let title = title.get(offset..offset + length).ok_or("truncated title root")?;
    let original = tombox_xshader::meta::parse_rts0(title)?;
    let ex = tombox_xshader::extract(shader)?;
    let bindings = tombox_xshader::fssm_bind::parse(&ex.fssm)?.ok_or("no FSSM table")?;
    let inferred = tombox_xshader::hlsl::fssm_root_signature(&ex, &bindings)?;
    if inferred.params.len() > original.params.len() {
        return Err("FSSM declares root parameters missing from the title layout".into());
    }
    // Replace the range array while preserving the root's parameter and static
    // sampler records. Explicit FSSM offsets identify the original heap slots.
    let mut body = title.to_vec();
    body[20..24].copy_from_slice(&(original.flags | inferred.flags).to_le_bytes());
    let params = u32::from_le_bytes(body[8..12].try_into().unwrap()) as usize;
    for (i, (p, q)) in original.params.iter().zip(&inferred.params).enumerate() {
        let param = params + i * 12;
        let at = u32::from_le_bytes(body[param + 8..param + 12].try_into().unwrap()) as usize;
        let new = match (&p.kind, &q.kind) {
            (Table(old), Table(new)) => {
                if new.is_empty() {
                    continue;
                }
                if old.iter().any(|r| r.range_type == 3) != new.iter().any(|r| r.range_type == 3) {
                    return Err(format!("FSSM table {i} changes descriptor heap type"));
                }
                new
            }
            (Constants { num32: a, .. }, Constants { register, space, num32: b }) if a == b => {
                // The title sets constants by parameter index, not by register.
                body[at..at + 4].copy_from_slice(&register.to_le_bytes());
                body[at + 4..at + 8].copy_from_slice(&space.to_le_bytes());
                continue;
            }
            (Descriptor { kind: a, register: b, space: c }, Descriptor { kind: d, register: e, space: f })
                if (a, b, c) == (d, e, f) => continue,
            _ => return Err(format!("FSSM parameter {i} disagrees with the title layout")),
        };
        let table = at;
        let offset = body.len() as u32;
        body[table..table + 4].copy_from_slice(&(new.len() as u32).to_le_bytes());
        body[table + 4..table + 8].copy_from_slice(&offset.to_le_bytes());
        for range in new {
            for word in [range.range_type, range.num, range.base_register, range.space] {
                body.extend_from_slice(&word.to_le_bytes());
            }
            if original.version >= 2 {
                body.extend_from_slice(&1u32.to_le_bytes()); // DESCRIPTORS_VOLATILE
            }
            body.extend_from_slice(&range.offset_in_table.to_le_bytes());
        }
    }
    if original.static_samplers.is_empty() {
        if let Some(sampler) = tombox_xshader::hlsl::single_literal_sampler(&ex) {
            let conflicts = original.params.iter().any(|param| match &param.kind {
                Table(ranges) => ranges.iter().any(|r| r.range_type == 3 && r.space == sampler.space
                    && sampler.register.checked_sub(r.base_register).is_some_and(|i| i < r.num)),
                _ => false,
            });
            if !conflicts {
                let offset = body.len() as u32;
                body[12..16].copy_from_slice(&1u32.to_le_bytes());
                body[16..20].copy_from_slice(&offset.to_le_bytes());
                for word in [sampler.filter, sampler.address_u, sampler.address_v, sampler.address_w, 0,
                    sampler.max_anisotropy, sampler.comparison_func, sampler.border_color,
                    sampler.min_lod_bits, sampler.max_lod_bits, sampler.register, sampler.space, sampler.visibility] {
                    body.extend_from_slice(&word.to_le_bytes());
                }
                if original.version >= 3 {
                    body.extend_from_slice(&0u32.to_le_bytes());
                }
            }
        }
    }
    Ok(body)
}

unsafe fn slot(obj: *mut c_void, index: usize) -> usize {
    unsafe { *(*obj.cast::<*const usize>()).add(index) }
}

pub(crate) unsafe fn create_root(dev: *mut c_void, body: &[u8]) -> Result<Resource, String> {
    let (offset, length) = crate::console_slots_mesh::rts0_range(body).ok_or("root has no RTS0")?;
    let body = body.get(offset..offset + length).ok_or("truncated root")?;
    let blob = crate::device_wrap::shader_root_container(body, crate::cbv_raw::enabled());
    let mut root = core::ptr::null_mut();
    let orig = crate::device_wrap::real_device_orig(16);
    let create: unsafe extern "system" fn(*mut c_void, u32, *const u8, usize, *const Guid, *mut *mut c_void) -> i32 =
        unsafe { core::mem::transmute(if orig == 0 { slot(dev, 16) } else { orig }) };
    let hr = unsafe { create(dev, 0, blob.as_ptr(), blob.len(), &IID_ROOT, &mut root) };
    if hr < 0 || root.is_null() {
        return Err(format!("FSSM CreateRootSignature hr={hr:#010x}"));
    }
    crate::pso_lib::note_real_root(root as usize, &blob);
    Ok(Resource(root as usize))
}

pub(crate) unsafe fn attach(pso: *mut c_void, root: &Resource) -> bool {
    let set: unsafe extern "system" fn(*mut c_void, *const Guid, *mut c_void) -> i32 =
        unsafe { core::mem::transmute(slot(pso, 5)) };
    if unsafe { set(pso, &PSO_ROOT, root.0 as *mut c_void) } < 0 {
        return false;
    }
    ACTIVE.store(true, Relaxed);
    true
}

pub(crate) unsafe fn dispatch(
    list: *mut c_void,
    run: impl FnOnce(),
) -> bool {
    if !ACTIVE.load(Relaxed) {
        return false;
    }
    let Some(pso) = crate::cl2_state::bound_pso(list).filter(|&pso| pso != 0) else { return false };
    let pso = pso as *mut c_void;
    let get: unsafe extern "system" fn(*mut c_void, *const Guid, *mut u32, *mut usize) -> i32 =
        unsafe { core::mem::transmute(slot(pso, 3)) };
    let mut root = 0;
    let mut size = core::mem::size_of::<usize>() as u32;
    if unsafe { get(pso, &PSO_ROOT, &mut size, &mut root) } < 0 || root == 0 {
        return false;
    }
    let root = Resource(root);
    let Some(bindings) = crate::cl2_state::compute_bindings(list) else { return false };
    let tables: Vec<_> = bindings.table_handles().map(|(index, handle)| {
        (index, crate::title_heap::compute_variant_table(list as usize, root.0, index, handle))
    }).collect();
    let set_root: unsafe extern "system" fn(*mut c_void, *mut c_void) =
        unsafe { core::mem::transmute(crate::cmdlist_wrap::pc_method(list, 29)) };
    let set_table: unsafe extern "system" fn(*mut c_void, u32, u64) =
        unsafe { core::mem::transmute(crate::cmdlist_wrap::pc_method(list, 31)) };
    unsafe {
        set_root(list, root.0 as *mut c_void);
        for &(index, handle) in &tables {
            set_table(list, index, handle);
        }
        bindings.apply_compute_args(list);
        run();
        set_root(list, bindings.root());
        for &(index, handle) in &tables {
            set_table(list, index, handle);
        }
        bindings.apply_compute_args(list);
    }
    true
}
