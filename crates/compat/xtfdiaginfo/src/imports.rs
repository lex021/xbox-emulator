use crate::sys::*;
use core::ffi::c_void;
use std::sync::OnceLock;
use xtfcommon::msvc::WString;

pub type CreateFn = unsafe extern "C" fn(*mut *mut c_void) -> HRESULT;
pub type CreateIidFn = unsafe extern "C" fn(*const GUID, *mut *mut c_void) -> HRESULT;
pub type ParseWideFn = unsafe extern "C" fn(*const u16, *mut *mut c_void) -> HRESULT;
pub type ParseUtf8Fn = unsafe extern "C" fn(*const u8, *mut *mut c_void) -> HRESULT;
pub type LogVFn = unsafe extern "C" fn(*const u8, *mut u8);
pub type InstanceFn = unsafe extern "C" fn() -> *mut c_void;
pub type GetLoggingFn = unsafe extern "C" fn(*mut c_void, *mut u32, *mut WString, *mut u8) -> HRESULT;
pub type SetLoggingFn = unsafe extern "C" fn(*mut c_void, *const u32, *const u16, *const u8) -> HRESULT;
pub type LinkCreateFn = unsafe extern "C" fn() -> *mut u8;
pub type LinkConnectByNameFn = unsafe extern "C" fn(*mut u8, *const u16, u16, u32) -> HRESULT;
pub type LinkQueryFn = unsafe extern "C" fn(*mut u8, u32, u32, *mut u8) -> HRESULT;
pub type LinkCloseFn = unsafe extern "C" fn(*mut u8);

#[derive(Clone, Copy)]
pub struct Imports {
    pub create_json_object: CreateFn,
    pub create_json_array: CreateFn,
    pub get_json_object: ParseWideFn,
    pub get_json_object_from_utf8: ParseUtf8Fn,
    pub create_extension_metadata: CreateIidFn,
    pub create_message: CreateIidFn,
    pub has_system_gone_dark: unsafe extern "C" fn() -> u32,
    pub set_system_go_dark: unsafe extern "C" fn(u8),
    pub log_info_v: LogVFn,
    pub log_error_v: LogVFn,
    pub logging_instance: InstanceFn,
    pub get_logging_properties: GetLoggingFn,
    pub set_logging_properties: SetLoggingFn,
    pub link_create_handle: LinkCreateFn,
    pub link_connect_by_name: LinkConnectByNameFn,
    pub link_query_handle_information: LinkQueryFn,
    pub link_close_handle: LinkCloseFn,
}

static IMPORTS: OnceLock<Result<Imports, HRESULT>> = OnceLock::new();

#[cfg(not(test))]
unsafe fn module(name: &str) -> Result<HANDLE, HRESULT> {
    let w = wz(name);
    let h = GetModuleHandleW(w.as_ptr());
    if !h.is_null() {
        return Ok(h);
    }
    let h = LoadLibraryExW(w.as_ptr(), core::ptr::null_mut(), 0);
    if h.is_null() {
        Err(hr_last_error())
    } else {
        Ok(h)
    }
}

#[cfg(not(test))]
unsafe fn proc_addr<T: Copy>(m: HANDLE, name: &str) -> Result<T, HRESULT> {
    let mut n = name.as_bytes().to_vec();
    n.push(0);
    let p = GetProcAddress(m, n.as_ptr());
    if p.is_null() {
        Err(hr_last_error())
    } else {
        Ok(core::mem::transmute_copy(&p))
    }
}

#[cfg(not(test))]
fn load() -> Result<Imports, HRESULT> {
    unsafe {
        let x = module("xbtp.dll")?;
        let b = module("XtfBaseServices.dll")?;
        Ok(Imports {
            create_json_object: proc_addr(x, "XbtpCreateJsonObject")?,
            create_json_array: proc_addr(x, "XbtpCreateJsonArray")?,
            get_json_object: proc_addr(x, "XbtpGetJsonObject")?,
            get_json_object_from_utf8: proc_addr(x, "XbtpGetJsonObjectFromUtf8")?,
            create_extension_metadata: proc_addr(x, "XbtpCreateExtensionMetadata")?,
            create_message: proc_addr(x, "XbtpCreateMessage")?,
            has_system_gone_dark: proc_addr(x, "XbtpHasSystemGoneDark")?,
            set_system_go_dark: proc_addr(x, "XbtpSetSystemGoDark")?,
            log_info_v: proc_addr(x, "?LogInfoV@@YAXPEBDPEAD@Z")?,
            log_error_v: proc_addr(x, "?LogErrorV@@YAXPEBDPEAD@Z")?,
            logging_instance: proc_addr(b, "?Instance@LoggingManager@xtfbaseservices@@SAAEAV12@XZ")?,
            get_logging_properties: proc_addr(b, "?GetLoggingProperties@LoggingManager@xtfbaseservices@@QEAAJAEAW4eLoggingLevel@log@xtfbase@@AEAV?$basic_string@GU?$char_traits@G@std@@V?$allocator@G@2@@std@@AEA_N@Z")?,
            set_logging_properties: proc_addr(b, "?SetLoggingProperties@LoggingManager@xtfbaseservices@@QEAAJPEAW4eLoggingLevel@log@xtfbase@@PEBGPEA_N@Z")?,
            link_create_handle: proc_addr(x, "XbtpLinkCreateHandle")?,
            link_connect_by_name: proc_addr(x, "XbtpLinkConnectByName")?,
            link_query_handle_information: proc_addr(x, "XbtpLinkQueryHandleInformation")?,
            link_close_handle: proc_addr(x, "XbtpLinkCloseHandle")?,
        })
    }
}

#[cfg(test)]
fn load() -> Result<Imports, HRESULT> {
    crate::tests::test_imports()
}

pub fn get() -> Result<&'static Imports, HRESULT> {
    match IMPORTS.get_or_init(load) {
        Ok(i) => Ok(i),
        Err(e) => Err(*e),
    }
}

pub unsafe fn log_info(fmt: &[u8], args: &[u64]) {
    if let Ok(i) = get() {
        (i.log_info_v)(fmt.as_ptr(), args.as_ptr() as *mut u8);
    }
}

pub unsafe fn log_error(fmt: &[u8], args: &[u64]) {
    if let Ok(i) = get() {
        (i.log_error_v)(fmt.as_ptr(), args.as_ptr() as *mut u8);
    }
}
