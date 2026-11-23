use core::ffi::c_void;
use core::sync::atomic::{AtomicUsize, Ordering};

use compat_common::record;

use crate::device_wrap::Guid;

const IID_DESTRUCTION_NOTIFIER: Guid = Guid { d1: 0xa06e_b39a, d2: 0x50da, d3: 0x425b, d4: [0x8c, 0x31, 0x4e, 0xec, 0xd6, 0xc2, 0x70, 0xf3] };

#[link(name = "kernel32")]
extern "system" {
    fn GetModuleHandleA(name: *const u8) -> *mut c_void;
    fn GetProcAddress(module: *mut c_void, name: *const u8) -> *mut c_void;
}

type ChargeFn = unsafe extern "system" fn(u64, u32) -> u64;
type DestroyedFn = unsafe extern "system" fn(*mut c_void);

static CHARGE: AtomicUsize = AtomicUsize::new(0);
static TRACKED: AtomicUsize = AtomicUsize::new(0);
static UNTRACKED: AtomicUsize = AtomicUsize::new(0);
static LIVE: [core::sync::atomic::AtomicU64; 2] = [const { core::sync::atomic::AtomicU64::new(0) }; 2];
const MISSING: usize = 1;
const HEAP_BIT: u64 = 1 << 63;

pub fn live_bytes() -> (u64, u64, bool) {
    let f = CHARGE.load(Ordering::Acquire);
    (LIVE[0].load(Ordering::Relaxed), LIVE[1].load(Ordering::Relaxed), f != 0 && f != MISSING)
}

fn charge_fn() -> Option<ChargeFn> {
    let mut f = CHARGE.load(Ordering::Acquire);
    if f == 0 {
        let h = unsafe { GetModuleHandleA(b"xmem.dll\0".as_ptr()) };
        let p = if h.is_null() { core::ptr::null_mut() } else { unsafe { GetProcAddress(h, b"TomboxTitleGpuCharge\0".as_ptr()) } };
        f = if p.is_null() { MISSING } else { p as usize };
        let _ = CHARGE.compare_exchange(0, f, Ordering::AcqRel, Ordering::Acquire);
        f = CHARGE.load(Ordering::Acquire);
        if f == MISSING {
            record("d3d12_xs", "title_gpu", "xmem.dll TomboxTitleGpuCharge not found: title GPU memory is not counted in title_used");
        }
    }
    if f == MISSING { None } else { Some(unsafe { core::mem::transmute::<usize, ChargeFn>(f) }) }
}

#[cfg(test)]
pub fn set_charge_for_test(f: usize) {
    CHARGE.store(f, Ordering::Release);
}

pub fn counts() -> (usize, usize) {
    (TRACKED.load(Ordering::Relaxed), UNTRACKED.load(Ordering::Relaxed))
}

unsafe fn slot(obj: *mut c_void, i: usize) -> usize {
    unsafe { *(*(obj as *const *const usize)).add(i) }
}

unsafe extern "system" fn on_destroyed(data: *mut c_void) {
    let d = data as u64;
    let bytes = d & !HEAP_BIT;
    LIVE[(d & HEAP_BIT != 0) as usize].fetch_sub(bytes, Ordering::Relaxed);
    if let Some(f) = charge_fn() {
        unsafe { f(bytes, 1) };
    }
}

pub unsafe fn track(obj: *mut c_void, bytes: u64, what: &str) -> bool {
    unsafe { track_kind(obj, bytes, what, false) }
}

unsafe fn track_kind(obj: *mut c_void, bytes: u64, what: &str, is_heap: bool) -> bool {
    if bytes & HEAP_BIT != 0 {
        return false;
    }
    if obj.is_null() || bytes == 0 {
        return false;
    }
    let Some(charge) = charge_fn() else { return false };
    let qi: unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> i32 = unsafe { core::mem::transmute(slot(obj, 0)) };
    let mut n: *mut c_void = core::ptr::null_mut();
    if unsafe { qi(obj, &IID_DESTRUCTION_NOTIFIER, &mut n) } < 0 || n.is_null() {
        if UNTRACKED.fetch_add(1, Ordering::Relaxed) < 4 {
            record("d3d12_xs", "title_gpu", &format!("{what} {obj:p} has no ID3DDestructionNotifier: {bytes:#x} bytes not counted"));
        }
        return false;
    }
    let register: unsafe extern "system" fn(*mut c_void, DestroyedFn, *mut c_void, *mut u32) -> i32 = unsafe { core::mem::transmute(slot(n, 3)) };
    let release: unsafe extern "system" fn(*mut c_void) -> u32 = unsafe { core::mem::transmute(slot(n, 2)) };
    let mut id = 0u32;
    let hr = unsafe { register(n, on_destroyed, (bytes | if is_heap { HEAP_BIT } else { 0 }) as usize as *mut c_void, &mut id) };
    unsafe { release(n) };
    if hr < 0 {
        if UNTRACKED.fetch_add(1, Ordering::Relaxed) < 4 {
            record("d3d12_xs", "title_gpu", &format!("{what} {obj:p}: RegisterDestructionCallback hr={hr:#010x}, {bytes:#x} bytes not counted"));
        }
        return false;
    }
    LIVE[is_heap as usize].fetch_add(bytes, Ordering::Relaxed);
    let total = unsafe { charge(bytes, 0) };
    if TRACKED.fetch_add(1, Ordering::Relaxed) < 6 {
        record("d3d12_xs", "title_gpu", &format!("{what} {obj:p} charged {bytes:#x} bytes to title memory (title GPU total {:.1} MB), released when the object is destroyed", total as f64 / (1u64 << 20) as f64));
    }
    true
}

pub unsafe fn committed(res: *mut c_void, xdesc: &[u8; 0x38]) -> bool {
    unsafe { track(res, crate::device_wrap::title_layout_size(xdesc), "committed resource") }
}

pub unsafe fn heap(heap: *mut c_void, size: u64) -> bool {
    unsafe { track_kind(heap, size, "heap", true) }
}
