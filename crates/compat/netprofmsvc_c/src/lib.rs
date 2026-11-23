use core::ffi::c_void;
use std::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};

pub mod aggregate;
pub mod failure;
#[cfg(any(test, feature = "service"))]
pub mod iphlp;
#[cfg(any(test, feature = "service"))]
pub mod probes;
#[cfg(any(test, feature = "service"))]
pub mod webprobe;
#[cfg(any(test, feature = "service"))]
pub mod join;
#[cfg(any(test, feature = "service"))]
pub mod connectivity;
pub mod tlg;

pub use failure::{result_logging_callback, FailureInfo};
pub use tlg::{Guid, Provider, AGGREGATE_GUID, NETWORK_PROFILE_GUID};

pub static SVCHOST_GLOBALS: AtomicPtr<c_void> = AtomicPtr::new(core::ptr::null_mut());

pub static PROVIDER: Provider = Provider::new(NETWORK_PROFILE_GUID, "Microsoft.Windows.NetworkProfileService");
pub static AGGREGATE_PROVIDER: Provider = Provider::new(AGGREGATE_GUID, "Microsoft.Windows.TlgAggregateInternal");

pub static RESULT_LOGGING_CALLBACK: AtomicUsize = AtomicUsize::new(0);

#[repr(C)]
struct ExceptionRecord {
    code: u32,
    flags: u32,
    record: *mut c_void,
    address: *mut c_void,
    params: u32,
    info: [usize; 15],
}

#[link(name = "kernel32")]
extern "system" {
    fn RaiseFailFastException(rec: *const ExceptionRecord, ctx: *const c_void, flags: u32);
}

#[no_mangle]
pub extern "system" fn SvchostPushServiceGlobalsEx(globals: *mut c_void) {
    SVCHOST_GLOBALS.store(globals, Ordering::Release);
}

pub fn svchost_globals() -> *mut c_void {
    SVCHOST_GLOBALS.load(Ordering::Acquire)
}

fn fail_fast() -> ! {
    let rec = ExceptionRecord { code: 0xC000_0409, flags: 1, record: core::ptr::null_mut(), address: core::ptr::null_mut(), params: 1, info: { let mut i = [0usize; 15]; i[0] = 7; i } };
    unsafe { RaiseFailFastException(&rec, core::ptr::null(), 0) };
    std::process::abort()
}

pub fn result_logging_callback_addr() -> usize {
    result_logging_callback as *const () as usize
}

#[no_mangle]
pub extern "system" fn DllMain(_module: *mut c_void, reason: u32, _reserved: *mut c_void) -> i32 {
    match reason {
        1 => {
            aggregate::attach();
            let ours = result_logging_callback_addr();
            let cur = RESULT_LOGGING_CALLBACK.load(Ordering::SeqCst);
            if cur != 0 && cur != ours {
                fail_fast();
            }
            RESULT_LOGGING_CALLBACK.store(ours, Ordering::SeqCst);
        }
        0 => {
            RESULT_LOGGING_CALLBACK.store(0, Ordering::SeqCst);
            aggregate::detach();
        }
        _ => {}
    }
    1
}

pub unsafe fn report_failure(info: &FailureInfo) {
    let cb = RESULT_LOGGING_CALLBACK.load(Ordering::SeqCst);
    if cb != 0 {
        let f: unsafe extern "system" fn(*const FailureInfo, *mut u16, usize) = core::mem::transmute(cb);
        f(info, core::ptr::null_mut(), 0);
    }
}

#[cfg(feature = "service")]
#[used]
#[link_section = ".CRT$XCT"]
pub static SERVICE_EARLY_OPT_OUT: extern "C" fn() = service_early_opt_out;

#[cfg(feature = "service")]
extern "C" fn service_early_opt_out() {
    compat_common::gdk::EARLY_INIT_OPT_OUT.store(true, Ordering::SeqCst);
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
