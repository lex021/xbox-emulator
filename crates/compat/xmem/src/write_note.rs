use core::ffi::c_void;
use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

extern "system" {
    fn GetModuleHandleA(name: *const u8) -> *mut c_void;
    fn GetProcAddress(module: *mut c_void, name: *const u8) -> *mut c_void;
    fn GetTickCount64() -> u64;
}

type NoteFn = unsafe extern "system" fn(usize, usize, u32);

const ABSENT: usize = 1;
const NOTE_COMMITTED: u32 = 1;
static NOTE_FN: AtomicUsize = AtomicUsize::new(0);
static TRIES: AtomicUsize = AtomicUsize::new(0);
static LAST_TRY_MS: AtomicU64 = AtomicU64::new(0);
const RETRY_MS: u64 = 250;

fn note_fn() -> Option<NoteFn> {
    let f = NOTE_FN.load(Ordering::Acquire);
    if f > ABSENT {
        return Some(unsafe { core::mem::transmute::<usize, NoteFn>(f) });
    }
    let now = unsafe { GetTickCount64() };
    let last = LAST_TRY_MS.load(Ordering::Relaxed);
    if last != 0 && now.wrapping_sub(last) < RETRY_MS {
        return None;
    }
    LAST_TRY_MS.store(now.max(1), Ordering::Relaxed);
    let p = unsafe {
        let m = GetModuleHandleA(b"d3d12_xs.dll\0".as_ptr());
        if m.is_null() { 0 } else { GetProcAddress(m, b"TomboxNoteTitleMemory\0".as_ptr()) as usize }
    };
    if p > ABSENT {
        NOTE_FN.store(p, Ordering::Release);
        return Some(unsafe { core::mem::transmute::<usize, NoteFn>(p) });
    }
    TRIES.fetch_add(1, Ordering::Relaxed);
    None
}

pub fn committed(addr: usize, len: usize) {
    if addr == 0 || len == 0 {
        return;
    }
    if let Some(f) = note_fn() {
        unsafe { f(addr, len, NOTE_COMMITTED) };
    }
}
