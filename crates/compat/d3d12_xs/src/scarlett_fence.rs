//! Scarlett exposes the completed fence value as a CPU-readable address.
use crate::device_wrap::Guid;
use compat_common::record;
use core::ffi::c_void;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::thread::{self, Thread};
use std::time::Duration;

pub const IID_FENCE: Guid = Guid {
    d1: 0x0a753dcf, d2: 0xc4d8, d3: 0x4b91,
    d4: [0xad, 0xf6, 0xbe, 0x5a, 0x60, 0xd9, 0x5a, 0x76],
};
const IID_FENCE1: Guid = Guid {
    d1: 0x433685fe, d2: 0xe22b, d3: 0x4ca0,
    d4: [0xa8, 0xdb, 0xb5, 0xb4, 0xf4, 0xdd, 0x0e, 0x4a],
};
const E_POINTER: i32 = 0x8000_4003u32 as i32;
const E_NOINTERFACE: i32 = 0x8000_4002u32 as i32;

#[derive(Default)]
struct CompletedValue {
    value: AtomicU64,
    stop: AtomicBool,
}

#[repr(C)]
struct Fence {
    table: *const usize,
    real: *mut c_void,
    refs: AtomicU32,
    completed: Arc<CompletedValue>,
    monitor: OnceLock<Option<Thread>>,
}

unsafe fn slot(real: *mut c_void, index: usize) -> usize {
    unsafe { *(*real.cast::<*const usize>()).add(index) }
}

unsafe fn completed_value(real: *mut c_void) -> u64 {
    let f: unsafe extern "system" fn(*mut c_void) -> u64 =
        unsafe { core::mem::transmute(slot(real, 8)) };
    unsafe { f(real) }
}

unsafe fn reference(real: *mut c_void, index: usize) -> u32 {
    let f: unsafe extern "system" fn(*mut c_void) -> u32 =
        unsafe { core::mem::transmute(slot(real, index)) };
    unsafe { f(real) }
}

unsafe extern "system" fn qi(this: *mut Fence, iid: *const Guid, out: *mut *mut c_void) -> i32 {
    if out.is_null() || iid.is_null() {
        return E_POINTER;
    }
    unsafe { *out = core::ptr::null_mut() };
    let fence = unsafe { &*this };
    let query: unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> i32 =
        unsafe { core::mem::transmute(slot(fence.real, 0)) };
    let mut host = core::ptr::null_mut();
    let hr = unsafe { query(fence.real, iid, &mut host) };
    if hr < 0 || host.is_null() {
        return hr;
    }
    unsafe { reference(host, 2) };
    // These interfaces are prefixes of the host fence vtable.
    let prefix = matches!(unsafe { (*iid).d1 }, 0 | 0xc4fec28f | 0x905db94b | 0x63ee58fb);
    if !prefix && unsafe { *iid != IID_FENCE && *iid != IID_FENCE1 } {
        return E_NOINTERFACE;
    }
    unsafe {
        add_ref(this);
        *out = this.cast();
    }
    0
}

unsafe extern "system" fn add_ref(this: *mut Fence) -> u32 {
    unsafe { (*this).refs.fetch_add(1, Ordering::Relaxed) + 1 }
}

unsafe extern "system" fn release(this: *mut Fence) -> u32 {
    let fence = unsafe { &*this };
    let remaining = fence.refs.fetch_sub(1, Ordering::AcqRel) - 1;
    if remaining == 0 {
        let fence = unsafe { Box::from_raw(this) };
        fence.completed.stop.store(true, Ordering::Release);
        if let Some(Some(watcher)) = fence.monitor.get() {
            watcher.unpark();
        }
        unsafe { reference(fence.real, 2) };
    }
    remaining
}

macro_rules! forward {
    ($($name:ident = $index:literal),+ $(,)?) => {$(
        #[unsafe(naked)]
        unsafe extern "system" fn $name() {
            core::arch::naked_asm!(
                "mov rcx, [rcx + 8]",
                "mov rax, [rcx]",
                "jmp qword ptr [rax + {offset}]",
                offset = const $index * 8,
            );
        }
    )+};
}

forward!(
    get_private_data = 3,
    set_private_data = 4,
    set_private_data_interface = 5,
    set_name = 6,
    get_device = 7,
    get_completed_value = 8,
    set_event_on_completion = 9,
    signal = 10,
    get_creation_flags = 11,
);

unsafe extern "system" fn completed_value_address(this: *mut Fence) -> *const u64 {
    let fence = unsafe { &*this };
    fence.monitor.get_or_init(|| {
        let live = Arc::clone(&fence.completed);
        live.value.store(unsafe { completed_value(fence.real) }, Ordering::Release);
        let real = fence.real as usize;
        unsafe { reference(fence.real, 1) };
        match thread::Builder::new().name("scarlett-fence".into()).spawn(move || {
            let real = real as *mut c_void;
            while !live.stop.load(Ordering::Acquire) {
                let value = unsafe { completed_value(real) };
                live.value.store(value, Ordering::Release);
                if value == u64::MAX {
                    break;
                }
                thread::park_timeout(Duration::from_millis(1));
            }
            unsafe { reference(real, 2) };
        }) {
            Ok(handle) => {
                record("d3d12_xs", "ScarlettFence", &format!("slot=12 fence={this:p} host={:p}", fence.real));
                Some(handle.thread().clone())
            }
            Err(error) => {
                unsafe { reference(fence.real, 2) };
                record("d3d12_xs", "ScarlettFence", &format!("failed to monitor fence: {error}"));
                None
            }
        }
    });
    fence.completed.value.as_ptr()
}

fn table() -> *const usize {
    static TABLE: OnceLock<[usize; 13]> = OnceLock::new();
    TABLE.get_or_init(|| [
        qi as *const () as usize, add_ref as *const () as usize, release as *const () as usize,
        get_private_data as *const () as usize, set_private_data as *const () as usize,
        set_private_data_interface as *const () as usize, set_name as *const () as usize,
        get_device as *const () as usize, get_completed_value as *const () as usize,
        set_event_on_completion as *const () as usize, signal as *const () as usize,
        get_creation_flags as *const () as usize, completed_value_address as *const () as usize,
    ]).as_ptr()
}

pub fn wrap(real: *mut c_void, initial: u64) -> *mut c_void {
    let completed = Arc::new(CompletedValue::default());
    completed.value.store(initial, Ordering::Relaxed);
    Box::into_raw(Box::new(Fence {
        table: table(),
        real,
        refs: AtomicU32::new(1),
        completed,
        monitor: OnceLock::new(),
    })).cast()
}

pub unsafe fn unwrap(fence: *mut c_void) -> *mut c_void {
    if !fence.is_null() && unsafe { *fence.cast::<*const usize>() } == table() {
        unsafe { (*fence.cast::<Fence>()).real }
    } else {
        fence
    }
}
