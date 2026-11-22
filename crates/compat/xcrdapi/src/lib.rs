pub mod abi;
pub mod objects;
pub mod request;
pub mod bcpack;
pub mod stats;
pub mod access_cache;
pub mod pool;
pub mod write_note;
pub mod xcrd_core;
pub mod xcrd_a;
pub mod xcrd_storage;
pub mod xcrd2_storage;
pub mod xcrd2_stream;
pub mod xcrd2_xvd;
pub mod xcrd2_xfer;
pub mod xcrd2_hsv;
pub mod xcrd_driver;
pub mod xcrd_xvdfile;
#[cfg(test)]
mod xcrd_driver_tests;
#[cfg(test)]
mod xcrd2_tests;
#[cfg(test)]
pub mod xcrd_testkit;
#[cfg(test)]
mod xcrd_a_tests;

#[cfg(test)]
mod tests;

use abi::*;
use core::ffi::c_void;
use std::sync::atomic::{AtomicUsize, Ordering};

const M: &str = "xcrdapi";

pub(crate) fn log(func: &str, detail: &str) {
    #[cfg(not(test))]
    compat_common::record(M, func, detail);
    #[cfg(test)]
    compat_common::record_raw(M, func, detail);
}

pub(crate) static TRAPS: AtomicUsize = AtomicUsize::new(0);

pub(crate) fn log_fatal(iface: &str, slot: usize, this: usize) {
    TRAPS.fetch_add(1, Ordering::SeqCst);
    let msg = format!("FATAL: title called unimplemented {iface} vtable slot {slot} on {this:#x}; the console ABI for this slot is not known, refusing to continue");
    log("trap", &msg);
    #[cfg(not(test))]
    {
        if std::env::var_os("TOMBOX_XCRD_TRAP_CONTINUE").is_none() {
            eprintln!("[xcrdapi] {msg}");
            std::process::abort();
        }
    }
}

#[no_mangle]
pub unsafe extern "system" fn DStorageGetFactory(riid: *const c_void, ppv: *mut *mut c_void) -> i32 {
    if ppv.is_null() {
        log("DStorageGetFactory", "E_POINTER");
        return E_POINTER;
    }
    let init = xcrd_driver::dstorage_init();
    if init < 0 {
        unsafe { *ppv = core::ptr::null_mut() };
        log("DStorageGetFactory", &format!("XCRD control-page init failed hr={init:#010x}"));
        return init;
    }
    let hr = unsafe { (objects::FACTORY_VTBL.query_interface)(objects::factory_ptr(), riid, ppv) };
    let iid = unsafe { read_iid(riid) }.map(|i| iid_text(&i)).unwrap_or_default();
    log("DStorageGetFactory", &format!("riid={iid} -> hr={hr:#010x} factory={:p} (console-layout IDStorageFactoryX, CPU file/zlib backend)", unsafe { *ppv }));
    hr
}
