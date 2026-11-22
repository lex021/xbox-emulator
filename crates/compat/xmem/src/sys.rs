use core::ffi::c_void;

pub type Handle = *mut c_void;

pub const MEM_COMMIT: u32 = 0x0000_1000;
pub const MEM_RESERVE: u32 = 0x0000_2000;
pub const MEM_RELEASE: u32 = 0x0000_8000;
pub const MEM_RESET: u32 = 0x0008_0000;
pub const MEM_TOP_DOWN: u32 = 0x0010_0000;
pub const MEM_WRITE_WATCH: u32 = 0x0020_0000;
pub const MEM_RESET_UNDO: u32 = 0x0100_0000;
pub const WIN_ALLOC_TYPE_MASK: u32 =
    MEM_COMMIT | MEM_RESERVE | MEM_RESET | MEM_TOP_DOWN | MEM_WRITE_WATCH | MEM_RESET_UNDO;

pub const PAGE_NOACCESS: u32 = 0x01;
pub const PAGE_READWRITE: u32 = 0x04;
pub const PAGE_GUARD: u32 = 0x100;

pub const ERROR_INVALID_PARAMETER: u32 = 87;
pub const ERROR_CAN_NOT_COMPLETE: u32 = 1003;
pub const ERROR_BAD_LENGTH: u32 = 24;

#[repr(C)]
#[derive(Default, Clone, Copy)]
pub struct MemoryBasicInformation {
    pub base: usize,
    pub alloc_base: usize,
    pub alloc_protect: u32,
    pub partition_id: u16,
    pub pad0: u16,
    pub region_size: usize,
    pub state: u32,
    pub protect: u32,
    pub kind: u32,
    pub pad1: u32,
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
pub struct MemoryStatusEx {
    pub length: u32,
    pub load: u32,
    pub total_phys: u64,
    pub avail_phys: u64,
    pub total_page_file: u64,
    pub avail_page_file: u64,
    pub total_virtual: u64,
    pub avail_virtual: u64,
    pub avail_ext_virtual: u64,
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
pub struct ProcessMemoryCountersEx {
    pub cb: u32,
    pub page_fault_count: u32,
    pub peak_working_set: usize,
    pub working_set: usize,
    pub quota_peak_paged_pool: usize,
    pub quota_paged_pool: usize,
    pub quota_peak_non_paged_pool: usize,
    pub quota_non_paged_pool: usize,
    pub pagefile_usage: usize,
    pub peak_pagefile_usage: usize,
    pub private_usage: usize,
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
pub struct HeapSummaryInfo {
    pub cb: u32,
    pub allocated: usize,
    pub committed: usize,
    pub reserved: usize,
    pub max_reserve: usize,
}

extern "system" {
    pub fn GetProcessHeap() -> Handle;
    pub fn HeapAlloc(heap: Handle, flags: u32, bytes: usize) -> *mut c_void;
    pub fn HeapFree(heap: Handle, flags: u32, mem: *mut c_void) -> i32;
    pub fn HeapValidate(heap: Handle, flags: u32, mem: *const c_void) -> i32;
    pub fn HeapCreate(options: u32, initial: usize, maximum: usize) -> Handle;
    pub fn HeapSummary(heap: Handle, flags: u32, summary: *mut HeapSummaryInfo) -> i32;
    pub fn VirtualAlloc(addr: *mut c_void, size: usize, alloc_type: u32, protect: u32) -> *mut c_void;
    pub fn VirtualFree(addr: *mut c_void, size: usize, free_type: u32) -> i32;
    pub fn VirtualQueryEx(process: Handle, addr: *const c_void, mbi: *mut MemoryBasicInformation, len: usize) -> usize;
    pub fn GetWriteWatch(
        flags: u32,
        base: *mut c_void,
        size: usize,
        addresses: *mut *mut c_void,
        count: *mut usize,
        granularity: *mut u32,
    ) -> u32;
    pub fn ResetWriteWatch(base: *mut c_void, size: usize) -> u32;
    pub fn GlobalMemoryStatusEx(status: *mut MemoryStatusEx) -> i32;
    pub fn K32GetProcessMemoryInfo(process: Handle, counters: *mut ProcessMemoryCountersEx, cb: u32) -> i32;
    pub fn GetCurrentProcess() -> Handle;
    pub fn GetLastError() -> u32;
    pub fn SetLastError(code: u32);
    pub fn OutputDebugStringA(s: *const u8);
}

#[link(name = "ntdll", kind = "raw-dylib")]
extern "system" {
    pub fn RtlCreateHeap(
        flags: u32,
        base: *mut c_void,
        reserve: usize,
        commit: usize,
        lock: *mut c_void,
        params: *mut c_void,
    ) -> Handle;
}

pub fn round_up(v: usize, a: usize) -> usize {
    (v + a - 1) & !(a - 1)
}

pub fn system_memory() -> MemoryStatusEx {
    let mut s = MemoryStatusEx { length: core::mem::size_of::<MemoryStatusEx>() as u32, ..Default::default() };
    unsafe { GlobalMemoryStatusEx(&mut s) };
    s
}

pub fn process_memory() -> ProcessMemoryCountersEx {
    let mut c = ProcessMemoryCountersEx { cb: core::mem::size_of::<ProcessMemoryCountersEx>() as u32, ..Default::default() };
    unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut c, c.cb) };
    c
}

pub fn reserve_aligned(size: usize, align: usize, alloc_type: u32, protect: u32) -> *mut c_void {
    if align <= 0x1_0000 {
        return unsafe { VirtualAlloc(core::ptr::null_mut(), size, alloc_type, protect) };
    }
    for _ in 0..32 {
        let probe = unsafe { VirtualAlloc(core::ptr::null_mut(), size + align, MEM_RESERVE, PAGE_NOACCESS) };
        if probe.is_null() {
            return probe;
        }
        unsafe { VirtualFree(probe, 0, MEM_RELEASE) };
        let want = round_up(probe as usize, align) as *mut c_void;
        let p = unsafe { VirtualAlloc(want, size, alloc_type | MEM_RESERVE, protect) };
        if !p.is_null() {
            return p;
        }
    }
    core::ptr::null_mut()
}
