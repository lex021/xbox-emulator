use core::ffi::c_void;

pub const NET_SETUP_UNKNOWN_STATUS: u32 = 0;
pub const NET_SETUP_UNJOINED: u32 = 1;
pub const NET_SETUP_WORKGROUP_NAME: u32 = 2;
pub const NET_SETUP_DOMAIN_NAME: u32 = 3;
pub const NERR_SUCCESS: u32 = 0;
pub const ERROR_INVALID_PARAMETER: u32 = 87;
pub const ERROR_NOT_ENOUGH_MEMORY: u32 = 8;
pub const ERROR_NOT_SUPPORTED: u32 = 50;
pub const WORKGROUP: &str = "WORKGROUP";

#[link(name = "netapi32")]
extern "system" {
    fn NetApiBufferAllocate(size: u32, out: *mut *mut c_void) -> u32;
}

pub unsafe extern "system" fn net_get_join_information(server: *const u16, name: *mut *mut u16, status: *mut u32) -> u32 {
    if name.is_null() || status.is_null() {
        return ERROR_INVALID_PARAMETER;
    }
    if !server.is_null() && *server != 0 {
        return ERROR_NOT_SUPPORTED;
    }
    let w: Vec<u16> = WORKGROUP.encode_utf16().chain(Some(0)).collect();
    let mut p: *mut c_void = core::ptr::null_mut();
    if NetApiBufferAllocate((w.len() * 2) as u32, &mut p) != 0 || p.is_null() {
        return ERROR_NOT_ENOUGH_MEMORY;
    }
    core::ptr::copy_nonoverlapping(w.as_ptr(), p as *mut u16, w.len());
    *name = p as *mut u16;
    *status = NET_SETUP_WORKGROUP_NAME;
    NERR_SUCCESS
}

pub fn reads_domain_policy(status: u32) -> bool {
    status == NET_SETUP_DOMAIN_NAME
}
