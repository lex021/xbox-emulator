use core::ffi::c_void;
use core::sync::atomic::Ordering;
use std::collections::HashMap;
use std::sync::Mutex;

use crate::sys::*;
use crate::virt::{XMemBasicInformation, XMemVirtualQuery};
use compat_common::record;

#[path = "heap_19041.rs"]
mod legacy;

const M: &str = "xmem";

const HEAP_NO_SERIALIZE: u32 = 0x1;
const XMEM_HEAP_TOOLS: u64 = 0x1;
const XMEM_HEAP_IN_PLACE: u64 = 0x2;
const XMEM_HEAP_SERIALIZED: u64 = 0x4;

static NAMES: Mutex<Option<HashMap<usize, [u8; 8]>>> = Mutex::new(None);
static TITLE_HEAPS: Mutex<Vec<usize>> = Mutex::new(Vec::new());

#[no_mangle]
pub extern "system" fn XMemGetTitleHeap() -> Handle {
    unsafe { GetProcessHeap() }
}

#[no_mangle]
pub unsafe extern "system" fn XMemCreateWin32Heap(flags: u64, base: *mut c_void) -> Handle {
    let f = flags & !0x8000;
    let legacy = compat_common::os_host::routes::current_build() == 19041;
    let max_flags = if legacy { 2 } else { 7 };
    if f > max_flags {
        SetLastError(ERROR_INVALID_PARAMETER);
        return core::ptr::null_mut();
    }
    if f & XMEM_HEAP_IN_PLACE != 0 {
        if base.is_null() || base as usize & 0xfff != 0 || f != XMEM_HEAP_IN_PLACE {
            SetLastError(ERROR_INVALID_PARAMETER);
            return core::ptr::null_mut();
        }
        if legacy {
            return legacy::create(base);
        }
        let mut info = XMemBasicInformation::default();
        if XMemVirtualQuery(GetCurrentProcess(), base, &mut info) == 0 || info.state & MEM_COMMIT == 0 {
            SetLastError(ERROR_INVALID_PARAMETER);
            return core::ptr::null_mut();
        }
        let h = RtlCreateHeap(0, base, 0, 0, core::ptr::null_mut(), core::ptr::null_mut());
        record(M, "XMemCreateWin32Heap", &format!("in-place base={base:p} size={:#x} -> {h:p}", info.region_size));
        return h;
    }
    if !legacy && f & XMEM_HEAP_TOOLS != 0 {
        record(M, "XMemCreateWin32Heap", "tools partition requested; not available");
        SetLastError(ERROR_INVALID_PARAMETER);
        return core::ptr::null_mut();
    }
    let (h, title) = if legacy {
        legacy::partition_heap(f == XMEM_HEAP_TOOLS, flags & 0x8000 != 0)
    } else {
        let opts = if f & XMEM_HEAP_SERIALIZED != 0 { 0 } else { HEAP_NO_SERIALIZE };
        (HeapCreate(opts, 0, 0), true)
    };
    if title && !h.is_null() {
        TITLE_HEAPS.lock().unwrap_or_else(|e| e.into_inner()).push(h as usize);
    }
    record(M, "XMemCreateWin32Heap", &format!("flags={flags:#x} -> {h:p}"));
    h
}

#[no_mangle]
pub unsafe extern "system" fn XMemSetWin32HeapName(heap: Handle, name: *const u8, len: usize) -> i32 {
    let mut buf = [0u8; 8];
    let n = len.min(8);
    if n != 0 && !name.is_null() {
        core::ptr::copy_nonoverlapping(name, buf.as_mut_ptr(), n);
    }
    NAMES.lock().unwrap().get_or_insert_with(HashMap::new).insert(heap as usize, buf);
    1
}

#[no_mangle]
pub unsafe extern "system" fn XMemGetWin32HeapName(heap: Handle, out: *mut u8, out_len: usize, written: *mut usize) -> i32 {
    if out_len == 0 || out.is_null() {
        SetLastError(ERROR_INVALID_PARAMETER);
        return 0;
    }
    let name = NAMES.lock().unwrap().as_ref().and_then(|m| m.get(&(heap as usize)).copied()).unwrap_or([0u8; 8]);
    let len = name.iter().position(|&c| c == 0).unwrap_or(8);
    let n = len.min(out_len - 1);
    core::ptr::copy_nonoverlapping(name.as_ptr(), out, n);
    *out.add(n) = 0;
    if !written.is_null() {
        *written = n + 1;
    }
    1
}

#[repr(C)]
pub struct HeapStatistics {
    pub allocated: u64,
    pub overhead: u64,
}

#[no_mangle]
pub unsafe extern "system" fn XMemGetWin32HeapStatistics(heap: Handle, out: *mut HeapStatistics) -> i32 {
    if out.is_null() {
        SetLastError(ERROR_INVALID_PARAMETER);
        return 0;
    }
    crate::title_acct::announce();
    if let Some(a) = crate::heap_acct::slot(heap as usize) {
        let (allocated, overhead) = a.read();
        (*out).allocated = allocated;
        (*out).overhead = overhead;
        trace_heap(heap, allocated, overhead);
        return 1;
    }
    let patched = crate::hooks::install_heap_accounting();
    let mut s = HeapSummaryInfo { cb: core::mem::size_of::<HeapSummaryInfo>() as u32, ..Default::default() };
    if HeapSummary(heap, 0, &mut s) == 0 {
        return 0;
    }
    let committed = s.committed as u64;
    let allocated = (s.allocated as u64).min(committed);
    (*out).allocated = allocated;
    (*out).overhead = committed - allocated;
    let title_created = crate::title_acct::enabled() && TITLE_HEAPS.lock().unwrap_or_else(|e| e.into_inner()).contains(&(heap as usize));
    let reg = if title_created { crate::heap_acct::register_title(heap as usize, allocated, committed - allocated) } else { crate::heap_acct::register(heap as usize, allocated, committed - allocated) };
    if reg.is_some() {
        record(M, "XMemGetWin32HeapStatistics", &format!("heap {heap:p}: one baseline HeapSummary (allocated {allocated:#x}, overhead {:#x}), then running totals from the heap import wrappers ({patched} import slots); no further heap walks", committed - allocated));
    }
    trace_heap(heap, allocated, committed - allocated);
    1
}

fn trace_heap(heap: Handle, allocated: u64, overhead: u64) {
    if !crate::title_acct::log_on() {
        return;
    }
    crate::title_acct::trace("heap", &format!("queried heap {heap:p} reported allocated {allocated:#x} overhead {overhead:#x} (sum {:.1}MB)", (allocated + overhead) as f64 / (1u64 << 20) as f64));
}

#[no_mangle]
pub extern "system" fn XMemSetWin32HeapTrackingHooks(alloc: usize, realloc: usize, free: usize) {
    use crate::hooks::TRACKING_HOOKS;
    TRACKING_HOOKS[0].store(alloc, Ordering::Release);
    TRACKING_HOOKS[1].store(realloc, Ordering::Release);
    TRACKING_HOOKS[2].store(free, Ordering::Release);
    if alloc != 0 || realloc != 0 || free != 0 {
        crate::hooks::install_heap_hooks();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_heap_allocates_and_reports() {
        let h = unsafe { XMemCreateWin32Heap(0x4, core::ptr::null_mut()) };
        assert!(!h.is_null());
        let p = unsafe { HeapAlloc(h, 0, 0x1000) };
        assert!(!p.is_null());
        let mut st = HeapStatistics { allocated: 0, overhead: 0 };
        assert_eq!(unsafe { XMemGetWin32HeapStatistics(h, &mut st) }, 1);
        assert!(st.allocated >= 0x1000);
        let mut s = HeapSummaryInfo { cb: core::mem::size_of::<HeapSummaryInfo>() as u32, ..Default::default() };
        assert_ne!(unsafe { HeapSummary(h, 0, &mut s) }, 0);
        assert_eq!(st.allocated + st.overhead, s.committed as u64);
        let exe = unsafe { GetModuleHandleW(core::ptr::null()) };
        unsafe { crate::iat::patch_module(exe, &crate::hooks::heap_rule) };
        let q = unsafe { HeapAlloc(h, 0, 0x2000) };
        let mut st2 = HeapStatistics { allocated: 0, overhead: 0 };
        assert_eq!(unsafe { XMemGetWin32HeapStatistics(h, &mut st2) }, 1);
        assert_eq!(st2.allocated, st.allocated + 0x2000);
        assert_eq!(unsafe { HeapFree(h, 0, q) }, 1);
        let mut st3 = HeapStatistics { allocated: 0, overhead: 0 };
        assert_eq!(unsafe { XMemGetWin32HeapStatistics(h, &mut st3) }, 1);
        assert_eq!(st3.allocated, st.allocated);
        assert_eq!(st3.overhead, st.overhead);
        assert!(unsafe { XMemCreateWin32Heap(0x9, core::ptr::null_mut()) }.is_null());
        assert!(unsafe { XMemCreateWin32Heap(0x1, core::ptr::null_mut()) }.is_null());
    }

    #[test]
    fn in_place_heap_uses_caller_memory() {
        let mem = unsafe { VirtualAlloc(core::ptr::null_mut(), 0x10_0000, MEM_RESERVE | MEM_COMMIT, PAGE_READWRITE) };
        let h = unsafe { XMemCreateWin32Heap(0x2, mem) };
        assert!(!h.is_null());
        let p = unsafe { HeapAlloc(h, 0, 0x100) };
        assert!(p as usize >= mem as usize && (p as usize) < mem as usize + 0x10_0000);
        assert!(unsafe { XMemCreateWin32Heap(0x2, (mem as usize + 8) as *mut c_void) }.is_null());
    }

    #[test]
    fn heap_names_round_trip_and_truncate() {
        let h = unsafe { GetProcessHeap() };
        assert_eq!(unsafe { XMemSetWin32HeapName(h, b"TitleHeapX".as_ptr(), 10) }, 1);
        let mut buf = [0xffu8; 16];
        let mut n = 0usize;
        assert_eq!(unsafe { XMemGetWin32HeapName(h, buf.as_mut_ptr(), 16, &mut n) }, 1);
        assert_eq!(&buf[..9], b"TitleHea\0");
        assert_eq!(n, 9);
        assert_eq!(unsafe { XMemGetWin32HeapName(h, buf.as_mut_ptr(), 4, &mut n) }, 1);
        assert_eq!(&buf[..4], b"Tit\0");
        assert_eq!(unsafe { XMemGetWin32HeapName(h, buf.as_mut_ptr(), 0, &mut n) }, 0);
    }

    static EVENTS: std::sync::Mutex<Vec<(u8, usize, usize, usize)>> = std::sync::Mutex::new(Vec::new());
    const MAGIC: usize = 0x7357;

    unsafe extern "system" fn on_alloc(_heap: usize, _flags: u32, size: usize, result: usize) {
        if size == MAGIC {
            let _ = HeapAlloc(GetProcessHeap(), 0, MAGIC);
            EVENTS.lock().unwrap().push((1, size, result, 0));
        }
    }

    unsafe extern "system" fn on_realloc(_heap: usize, _flags: u32, size: usize, old: usize, new: usize) {
        if size == MAGIC * 2 {
            EVENTS.lock().unwrap().push((2, size, old, new));
        }
    }

    unsafe extern "system" fn on_free(_heap: usize, _flags: u32, ptr: usize) {
        if ptr != 0 && EVENTS.lock().unwrap().iter().any(|e| e.2 == ptr || e.3 == ptr) {
            EVENTS.lock().unwrap().push((3, 0, ptr, 0));
        }
    }

    extern "system" {
        fn GetModuleHandleW(name: *const u16) -> usize;
        fn HeapReAlloc(heap: Handle, flags: u32, mem: *mut c_void, bytes: usize) -> *mut c_void;
    }

    #[test]
    fn tracking_hooks_see_heap_operations() {
        use crate::hooks::{heap_rule, TRACKING_HOOKS, REALLOC_REPORTS_FREE};
        let exe = unsafe { GetModuleHandleW(core::ptr::null()) };
        TRACKING_HOOKS[0].store(on_alloc as *const () as usize, Ordering::Release);
        TRACKING_HOOKS[1].store(on_realloc as *const () as usize, Ordering::Release);
        TRACKING_HOOKS[2].store(on_free as *const () as usize, Ordering::Release);
        let patched = unsafe { crate::iat::patch_module(exe, &heap_rule) };
        let wrapped = std::cell::Cell::new(0usize);
        let count = |_dll: &[u8], name: Option<&[u8]>, value: usize| -> Option<usize> {
            if matches!(name, Some(b"HeapAlloc" | b"HeapReAlloc" | b"HeapFree")) && crate::hooks::is_ours(value) {
                wrapped.set(wrapped.get() + 1);
            }
            None
        };
        unsafe { crate::iat::patch_module(exe, &count) };
        assert!(wrapped.get() >= 2, "heap imports routed through wrappers: {} (newly patched {patched})", wrapped.get());
        let heap = unsafe { GetProcessHeap() };
        let p = unsafe { HeapAlloc(heap, 0, MAGIC) } as usize;
        assert!(p != 0);
        REALLOC_REPORTS_FREE.store(1, Ordering::Release);
        let q = unsafe { HeapReAlloc(heap, 0, p as *mut c_void, MAGIC * 2) } as usize;
        REALLOC_REPORTS_FREE.store(0, Ordering::Release);
        assert!(q != 0);
        assert_eq!(unsafe { HeapFree(heap, 0, q as *mut c_void) }, 1);
        TRACKING_HOOKS.iter().for_each(|h| h.store(0, Ordering::Release));
        let ev = EVENTS.lock().unwrap().clone();
        assert_eq!(ev.iter().filter(|e| e.0 == 1).count(), 1, "{ev:x?}");
        assert!(ev.contains(&(1, MAGIC, p, 0)));
        assert!(ev.contains(&(3, 0, p, 0)), "{ev:x?}");
        assert!(ev.contains(&(2, MAGIC * 2, p, q)));
        assert!(ev.contains(&(3, 0, q, 0)));
        let pos = |e: (u8, usize, usize, usize)| ev.iter().position(|x| *x == e).unwrap();
        assert!(pos((3, 0, p, 0)) < pos((2, MAGIC * 2, p, q)));
    }
}
