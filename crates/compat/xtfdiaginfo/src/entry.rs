use crate::com::*;
use crate::imports;
use crate::sys::*;
use crate::OperationFn;
use core::ffi::c_void;

pub const DIAGINFO_EXTENSION_ID: GUID = GUID::new(0xfd21ac23, 0x3a3a, 0x45ce, [0xa7, 0x85, 0xc9, 0x73, 0x37, 0x2d, 0xd8, 0x81]);
pub const IID_XBTP_EXTENSION_METADATA_IMPL: GUID = GUID::new(0xe2b6c063, 0x7d82, 0x414a, [0xbe, 0x86, 0x67, 0xed, 0x1b, 0x44, 0x77, 0x97]);

pub const OP_SYSTEM_UP_TIME: u32 = 0x0c;
pub const OP_SET_SYSTEM_GO_DARK: u32 = 0x10;
pub const OP_FETCH_SYSTEM_GO_DARK: u32 = 0x11;
pub const OP_SET_LOGGING_PROPERTIES: u32 = 0x18;
pub const OP_GET_LOGGING_PROPERTIES: u32 = 0x19;
pub const OP_PROCESS_INFORMATION_DELTA_UPDATE: u32 = 0x20;
pub const OP_TERMINATE_PROCESS: u32 = 0x21;
pub const OP_DEBUG_BREAK: u32 = 0x24;
pub const OP_RESUME_ALL_THREADS: u32 = 0x25;
pub const OP_REPORT_TITLE_MEMORY: u32 = 0x2d;
pub const OP_QUERY_PROFILING_DIRECTORY: u32 = 0x2e;
pub const OP_RESUME_DEFERRED_EXECUTION: u32 = 0x31;
pub const OP_CONNECTION_DIAGNOSTIC: u32 = 0x32;
pub const OP_DEBUG_ACTION: u32 = 0x36;

pub const CONSOLE_REGISTRATION_ORDER: [u32; 14] = [
    OP_SYSTEM_UP_TIME,
    OP_SET_SYSTEM_GO_DARK,
    OP_FETCH_SYSTEM_GO_DARK,
    OP_SET_LOGGING_PROPERTIES,
    OP_GET_LOGGING_PROPERTIES,
    OP_PROCESS_INFORMATION_DELTA_UPDATE,
    OP_TERMINATE_PROCESS,
    OP_DEBUG_BREAK,
    OP_RESUME_ALL_THREADS,
    OP_REPORT_TITLE_MEMORY,
    OP_QUERY_PROFILING_DIRECTORY,
    OP_RESUME_DEFERRED_EXECUTION,
    OP_CONNECTION_DIAGNOSTIC,
    OP_DEBUG_ACTION,
];

pub const CONSOLE_HANDLERS: [(u32, OperationFn); 14] = [
    (OP_SYSTEM_UP_TIME, crate::system::system_up_time_handler),
    (OP_SET_SYSTEM_GO_DARK, crate::system::set_system_go_dark_handler),
    (OP_FETCH_SYSTEM_GO_DARK, crate::system::fetch_system_go_dark_handler),
    (OP_SET_LOGGING_PROPERTIES, crate::system::set_logging_properties_handler),
    (OP_GET_LOGGING_PROPERTIES, crate::system::get_logging_properties_handler),
    (OP_PROCESS_INFORMATION_DELTA_UPDATE, crate::processdelta::process_information_delta_update_handler),
    (OP_TERMINATE_PROCESS, crate::process::terminate_process_handler),
    (OP_DEBUG_BREAK, crate::process::debug_break_handler),
    (OP_RESUME_ALL_THREADS, crate::process::resume_all_threads_handler),
    (OP_REPORT_TITLE_MEMORY, crate::titlememory::title_memory_report_handler),
    (OP_QUERY_PROFILING_DIRECTORY, crate::system::query_profiling_directory_handler),
    (OP_RESUME_DEFERRED_EXECUTION, crate::process::resume_deferred_execution_handler),
    (OP_CONNECTION_DIAGNOSTIC, crate::conndiag::connection_diagnostic_handler),
    (OP_DEBUG_ACTION, crate::debugaction::debug_action_handler),
];

pub unsafe fn extension_entry_point(extension: *const GUID, iid: *const GUID, out: *mut *mut c_void, handlers: &[(u32, OperationFn)]) -> HRESULT {
    if *extension != DIAGINFO_EXTENSION_ID || *iid != IID_XBTP_EXTENSION_METADATA_IMPL {
        return E_INVALIDARG;
    }
    let i = match imports::get() {
        Ok(i) => i,
        Err(e) => return e,
    };
    let mut md = Ptr::null();
    let hr = (i.create_extension_metadata)(&IID_XBTP_EXTENSION_METADATA_IMPL, md.out());
    if hr < 0 {
        return hr;
    }
    for &(id, f) in handlers {
        let hr = md_add_function(md.0, id, f);
        if hr < 0 {
            return hr;
        }
    }
    *out = md.detach();
    S_OK
}

#[no_mangle]
pub unsafe extern "C" fn XtfOrdinal_4(extension: *const GUID, iid: *const GUID, out: *mut *mut c_void) -> HRESULT {
    extension_entry_point(extension, iid, out, &CONSOLE_HANDLERS)
}
