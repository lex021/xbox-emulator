use compat_common::netview::iphlp as model;
use core::ffi::c_void;

pub unsafe extern "system" fn get_if_table2(table: *mut *mut u8) -> u32 {
    model::get_if_table2(table)
}

pub unsafe extern "system" fn get_if_entry2_ex(level: u32, row: *mut u8) -> u32 {
    model::get_if_entry2_ex(level, row)
}

pub unsafe extern "system" fn get_if_entry(row: *mut u8) -> u32 {
    model::get_if_entry(row)
}

pub unsafe extern "system" fn free_mib_table(table: *mut c_void) {
    model::free_mib_table(table)
}

pub unsafe extern "system" fn convert_interface_guid_to_luid(guid: *const [u8; 16], luid: *mut u64) -> u32 {
    model::convert_interface_guid_to_luid(guid, luid)
}

pub unsafe extern "system" fn convert_interface_luid_to_index(luid: *const u64, index: *mut u32) -> u32 {
    model::convert_interface_luid_to_index(luid, index)
}

pub unsafe extern "system" fn get_best_interface_ex(dest: *const u8, index: *mut u32) -> u32 {
    model::get_best_interface_ex(dest, index)
}

#[allow(clippy::too_many_arguments)]
pub unsafe extern "system" fn internal_get_ip_physical_interface_for_destination(dest: *const u8, _flags: u32, _iface: *const c_void, out: *mut [u8; 16], _ctx: *mut c_void, _cb: Option<model::PhysicalInterfaceCallback>, handle: *mut usize) -> u32 {
    model::physical_interface_for_destination(dest, out, handle)
}

pub unsafe extern "system" fn close_get_ip_physical_interface_for_destination(handle: *mut usize) -> u32 {
    model::close_physical_interface_for_destination(handle)
}

#[allow(clippy::too_many_arguments)]
pub unsafe extern "system" fn nsi_set_all_parameters(store: u32, action: u32, module: *const u8, object: u32, key: *const u8, key_len: u32, rw: *const u8, rw_len: u32) -> u32 {
    model::nsi_set_all_parameters(store, action, module, object, key, key_len, rw, rw_len)
}

pub unsafe fn sorted_probes(probes: &[[u8; 28]]) -> Result<Vec<[u8; 28]>, u32> {
    let mut list = vec![0u8; 8 + 16 * probes.len()];
    list[0..4].copy_from_slice(&(probes.len() as i32).to_le_bytes());
    for (i, p) in probes.iter().enumerate() {
        list[8 + 16 * i..16 + 16 * i].copy_from_slice(&(p.as_ptr() as usize).to_le_bytes());
        list[16 + 16 * i..20 + 16 * i].copy_from_slice(&0x1ci32.to_le_bytes());
    }
    let n = list.len() as u32;
    model::address_list_sort(list.as_mut_ptr(), n)?;
    Ok((0..probes.len()).map(|i| core::ptr::read_unaligned(usize::from_le_bytes(list[8 + 16 * i..16 + 16 * i].try_into().unwrap()) as *const [u8; 28])).collect())
}

pub unsafe fn physical_interface_for_probes(probes: &[[u8; 28]]) -> Option<[u8; 16]> {
    let sorted = sorted_probes(probes).ok()?;
    for d in &sorted {
        let mut out = [0u8; 16];
        let mut h = 0usize;
        let r = internal_get_ip_physical_interface_for_destination(d.as_ptr(), 0, core::ptr::null(), &mut out, core::ptr::null_mut(), None, &mut h);
        if h != 0 {
            close_get_ip_physical_interface_for_destination(&mut h);
        }
        if r == 0 {
            return Some(out);
        }
    }
    None
}

pub unsafe fn apply_interface_settings(luid: u64, field_10: u32, field_bc: u32, signature: [u8; 16]) -> [u32; 2] {
    let mut out = [0u32; 2];
    for (i, m) in [model::NPI_MS_IPV4_MODULEID, model::NPI_MS_IPV6_MODULEID].iter().enumerate() {
        let mut rw = [0xffu8; model::NL_INTERFACE_RW_SIZE];
        rw[0x10..0x14].copy_from_slice(&field_10.to_le_bytes());
        rw[0x20..0x28].copy_from_slice(&0u64.to_le_bytes());
        rw[0x28..0x2c].copy_from_slice(&0u32.to_le_bytes());
        rw[0x78..0x7c].copy_from_slice(&0u32.to_le_bytes());
        rw[0xbc..0xc0].copy_from_slice(&field_bc.to_le_bytes());
        rw[0xc0..0xd0].copy_from_slice(&signature);
        rw[0xd8..0xdc].copy_from_slice(&0u32.to_le_bytes());
        let key = luid.to_le_bytes();
        out[i] = nsi_set_all_parameters(model::NSI_STORE_ACTIVE, model::NSI_SET_DEFAULT, m.as_ptr(), model::NL_INTERFACE_OBJECT, key.as_ptr(), 8, rw.as_ptr(), rw.len() as u32);
    }
    out
}
