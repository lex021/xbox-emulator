//! Fixes the Xbox↔PC return-convention mismatch for the descriptor-handle getters
//! **without wrapping the heap object** (a wrapper would leak into real D3D12 calls
//! like `SetDescriptorHeaps`, crashing the driver).
//!
//! Instead we patch the real `ID3D12DescriptorHeap` vtable in place: slots 9/10
//! (`GetCPU/GPUDescriptorHandleForHeapStart`) are redirected to small bridges.
//! Xbox callers receive the handle in RAX. Native callers use the hidden
//! result pointer and keep the original Windows return convention. The heap
//! remains a real D3D12 object for both callers.

use core::ffi::c_void;
use core::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, RwLock};
use compat_common::record;

extern "system" {
    fn VirtualProtect(addr: *mut c_void, size: usize, new_protect: u32, old_protect: *mut u32) -> i32;
    fn GetModuleHandleW(name: *const u16) -> *mut c_void;
}
const PAGE_READWRITE: u32 = 0x04;

type NativeHandleGetter = unsafe extern "system" fn(*mut c_void, *mut u64) -> *mut u64;

struct NativeHeapVtable {
    address: usize,
    cpu: NativeHandleGetter,
    gpu: NativeHandleGetter,
}

static PATCHED_VTABLES: Mutex<Vec<NativeHeapVtable>> = Mutex::new(Vec::new());
static TITLE_IMAGE: std::sync::OnceLock<(usize, usize)> = std::sync::OnceLock::new();

fn xbox_caller(address: usize) -> bool {
    let (base, size) = *TITLE_IMAGE.get_or_init(|| unsafe {
        let base = GetModuleHandleW(core::ptr::null()) as usize;
        if base == 0 {
            return (0, 0);
        }
        let pe = core::ptr::read_unaligned((base + 0x3c) as *const u32) as usize;
        let size = core::ptr::read_unaligned((base + pe + 0x50) as *const u32) as usize;
        (base, size)
    });
    address.wrapping_sub(base) < size
}

unsafe fn native_getter(heap: *mut c_void, gpu: bool) -> NativeHandleGetter {
    let address = unsafe { *(heap as *const usize) };
    let tables = PATCHED_VTABLES.lock().unwrap_or_else(|e| e.into_inner());
    let table = tables.iter().find(|table| table.address == address).expect("descriptor heap vtable");
    if gpu { table.gpu } else { table.cpu }
}

struct CpuAlias {
    heap: usize,
    start: usize,
    bytes: usize,
    native: usize,
    owned: bool,
}

static SCARLETT_CPU: RwLock<Vec<CpuAlias>> = RwLock::new(Vec::new());
static ALIAS_RELEASE: AtomicUsize = AtomicUsize::new(0);

pub(crate) fn native_cpu_handle(handle: usize) -> usize {
    let aliases = SCARLETT_CPU.read().unwrap_or_else(|e| e.into_inner());
    aliases.iter().find_map(|alias| {
        let offset = handle.checked_sub(alias.start)?;
        (offset < alias.bytes).then(|| alias.native + offset)
    }).unwrap_or(handle)
}

extern "system" {
    fn VirtualAlloc(address: *mut c_void, bytes: usize, alloc_type: u32, protect: u32) -> *mut c_void;
    fn VirtualFree(address: *mut c_void, bytes: usize, free_type: u32) -> i32;
}

// The title reads and copies these entries through the CPU address returned by the heap.
pub(crate) unsafe fn mirror_scarlett_cbv(dest: usize, gpu_va: u64, size: u32) {
    if gpu_va >= 1u64 << 48 || size == 0 || size > 65536 || size % 16 != 0 {
        return;
    }
    let aliases = SCARLETT_CPU.read().unwrap_or_else(|e| e.into_inner());
    if aliases.iter().any(|a| a.owned && dest.checked_sub(a.start).is_some_and(|o| o % 32 == 0 && o <= a.bytes.saturating_sub(32))) {
        let raw = [0x0010_0000_0000_0000 | gpu_va, 0x0002_403c_0000_0000 | u64::from(size / 16), 0, 0];
        unsafe { core::ptr::write_unaligned(dest as *mut [u64; 4], raw) };
    }
}

unsafe extern "system" fn release_aliased_heap(heap: *mut c_void) -> u32 {
    let f: unsafe extern "system" fn(*mut c_void) -> u32 =
        unsafe { core::mem::transmute(ALIAS_RELEASE.load(Ordering::Acquire)) };
    // Keep creation from publishing a recycled heap address before removal.
    let mut aliases = SCARLETT_CPU.write().unwrap_or_else(|e| e.into_inner());
    let refs = unsafe { f(heap) };
    if refs == 0 {
        if let Some(i) = aliases.iter().position(|a| a.heap == heap as usize) {
            let alias = aliases.swap_remove(i);
            if alias.owned {
                unsafe { VirtualFree(alias.start as *mut c_void, 0, 0x8000) };
            }
        }
    }
    refs
}

pub(crate) unsafe fn register_scarlett_cpu(heap: *mut c_void, backing: Option<usize>, bytes: usize) {
    if heap.is_null() || bytes == 0 {
        return;
    }
    let native = unsafe { bridge_cpu(heap) } as usize;
    if !(1..0x10000).contains(&native) {
        return;
    }
    let owned = backing.is_none();
    let start = match backing {
        Some(start) if crate::title_shadow::is_writable_range(start, bytes) => start,
        Some(_) => return,
        None => (unsafe { VirtualAlloc(core::ptr::null_mut(), bytes, 0x3000, PAGE_READWRITE) }) as usize,
    };
    if start == 0 {
        return;
    }
    let published = {
        let mut aliases = SCARLETT_CPU.write().unwrap_or_else(|e| e.into_inner());
        if aliases.iter().any(|a| a.heap == heap as usize) {
            false
        } else if unsafe { install_alias_release(heap) } {
            aliases.push(CpuAlias { heap: heap as usize, start, bytes, native, owned });
            true
        } else {
            false
        }
    };
    if published {
        record("d3d12_xs", "descriptor_cpu", &format!(
            "guest CPU arena {start:#x}+{bytes:#x} routes to native CPU handle {native:#x} owned={owned}"
        ));
    } else if owned {
        unsafe { VirtualFree(start as *mut c_void, 0, 0x8000) };
    }
}

unsafe fn install_alias_release(heap: *mut c_void) -> bool {
    let vt = unsafe { *(heap as *const *mut usize) };
    let slot = unsafe { vt.add(2) };
    if unsafe { *slot } == release_aliased_heap as usize {
        return true;
    }
    let saved = ALIAS_RELEASE.load(Ordering::Acquire);
    let mut old = 0;
    if (saved != 0 && saved != unsafe { *slot })
        || unsafe { VirtualProtect(slot.cast(), core::mem::size_of::<usize>(), PAGE_READWRITE, &mut old) } == 0
    {
        return false;
    }
    unsafe {
        ALIAS_RELEASE.store(*slot, Ordering::Release);
        *slot = release_aliased_heap as usize;
        let mut restored = 0;
        VirtualProtect(slot.cast(), core::mem::size_of::<usize>(), old, &mut restored);
    }
    true
}


pub(crate) unsafe extern "system" fn bridge_cpu(heap: *mut c_void) -> u64 {
    {
        let alias = SCARLETT_CPU.read().unwrap_or_else(|e| e.into_inner());
        if let Some(alias) = alias.iter().find(|a| a.heap == heap as usize) {
            return alias.start as u64;
        }
    }
    let orig = unsafe { native_getter(heap, false) };
    let mut out: u64 = 0;
    unsafe { orig(heap, &mut out) };
    static N: AtomicUsize = AtomicUsize::new(0);
    if N.fetch_add(1, Ordering::Relaxed) < 6 {
        record("d3d12_xs", "heap.GetCPUHandleBase", &format!("heap={heap:p} base={out:#x}"));
    }
    out
}

pub(crate) unsafe extern "system" fn bridge_gpu(heap: *mut c_void) -> u64 {
    let orig = unsafe { native_getter(heap, true) };
    let mut out: u64 = 0;
    unsafe { orig(heap, &mut out) };
    static N: AtomicUsize = AtomicUsize::new(0);
    if N.fetch_add(1, Ordering::Relaxed) < 6 {
        record("d3d12_xs", "heap.GetGPUHandleBase", &format!("heap={heap:p} base={out:#x}"));
    }
    out
}

unsafe extern "system" fn dispatch_handle<const GPU: bool>(
    heap: *mut c_void, out: *mut u64, caller: usize,
) -> u64 {
    if xbox_caller(caller) {
        if GPU { unsafe { bridge_gpu(heap) } } else { unsafe { bridge_cpu(heap) } }
    } else {
        let get = unsafe { native_getter(heap, GPU) };
        unsafe { get(heap, out) as u64 }
    }
}

#[unsafe(naked)]
unsafe extern "system" fn cpu_handle_thunk() {
    core::arch::naked_asm!("mov r8, [rsp]", "jmp {dispatch}", dispatch = sym dispatch_handle::<false>);
}

#[unsafe(naked)]
unsafe extern "system" fn gpu_handle_thunk() {
    core::arch::naked_asm!("mov r8, [rsp]", "jmp {dispatch}", dispatch = sym dispatch_handle::<true>);
}

/// Patch this heap's descriptor-heap vtable (slots 9/10) if not already done.
/// Called for every created heap so each distinct heap-type vtable gets patched.
pub fn ensure_patched(real_heap: *mut c_void) {
    if real_heap.is_null() {
        return;
    }
    let vtbl = unsafe { *(real_heap as *const *mut usize) };
    let vtbl_key = vtbl as usize;
    let mut patched = PATCHED_VTABLES.lock().unwrap_or_else(|e| e.into_inner());
    if patched.iter().any(|table| table.address == vtbl_key) {
        return;
    }
    unsafe {
        let addr = vtbl.add(9) as *mut c_void;
        let mut old: u32 = 0;
        if VirtualProtect(addr, 16, PAGE_READWRITE, &mut old) != 0 {
            patched.push(NativeHeapVtable {
                address: vtbl_key,
                cpu: core::mem::transmute(*vtbl.add(9)),
                gpu: core::mem::transmute(*vtbl.add(10)),
            });
            *vtbl.add(9) = cpu_handle_thunk as *const () as usize;
            *vtbl.add(10) = gpu_handle_thunk as *const () as usize;
            let mut tmp: u32 = 0;
            VirtualProtect(addr, 16, old, &mut tmp);
            record("d3d12_xs", "heap.patch", &format!("vtable {vtbl:p} slots 9/10 bridged ({} total)", patched.len()));
        } else {
            record("d3d12_xs", "heap.patch", "VirtualProtect failed");
        }
    }
}
