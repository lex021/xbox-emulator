use core::ffi::c_void;
use core::ptr::null_mut;

use crate::sys::{GetCurrentProcess, Handle, RtlCreateHeap, MEM_COMMIT};
use crate::virt::{XMemBasicInformation, XMemVirtualAlloc, XMemVirtualQuery, XMEM_64K_PAGES};

const STATUS_INVALID_PARAMETER: i32 = 0xc000000du32 as i32;
const STATUS_UNSUCCESSFUL: i32 = 0xc0000001u32 as i32;

#[repr(C)]
struct HeapParameters {
    length: u32,
    segment_and_decommit_parameters: [usize; 6],
    initial_commit: usize,
    initial_reserve: usize,
    commit_routine: unsafe extern "system" fn(Handle, *mut Handle, *mut usize) -> i32,
    reserved: [usize; 2],
}

unsafe fn region(address: Handle) -> Option<XMemBasicInformation> {
    let mut info = XMemBasicInformation::default();
    (XMemVirtualQuery(GetCurrentProcess(), address, &mut info) != 0).then_some(info)
}

unsafe extern "system" fn commit(heap: Handle, address: *mut Handle, bytes: *mut usize) -> i32 {
    if heap.is_null() || address.is_null() || bytes.is_null() || (*address).is_null() || *bytes == 0 {
        return STATUS_INVALID_PARAMETER;
    }
    let Some(base) = region(heap) else {
        return STATUS_UNSUCCESSFUL;
    };
    let Some(target) = region(*address) else {
        return STATUS_UNSUCCESSFUL;
    };
    if base.alloc_base != target.alloc_base {
        return STATUS_UNSUCCESSFUL;
    }
    let (flags, page) = if base.state & XMEM_64K_PAGES != 0 {
        (XMEM_64K_PAGES, 0x10000)
    } else {
        (0, 0x1000)
    };
    let Some(size) = (*bytes - *bytes % page).checked_add(page) else {
        return STATUS_UNSUCCESSFUL;
    };
    if (*address as usize).checked_add(size).and_then(|end| end.checked_add(page - 1)).is_none() {
        return STATUS_UNSUCCESSFUL;
    }
    if XMemVirtualAlloc(*address, size, MEM_COMMIT | flags, base.flags, base.protect).is_null() {
        return STATUS_UNSUCCESSFUL;
    }
    *bytes = size;
    0
}

pub(super) unsafe fn partition_heap(tools: bool, force: bool) -> (Handle, bool) {
    let index = if !tools || core::arch::x86_64::__cpuid_count(0x4000_000d, 0).ecx >> 16 == 0x68 {
        crate::PARTITION_TITLE
    } else if crate::stats::tools_available(force) {
        crate::PARTITION_TOOLS
    } else {
        crate::sys::SetLastError(crate::sys::ERROR_INVALID_PARAMETER);
        return (null_mut(), false);
    };
    let handle = match crate::partition(index as usize) {
        crate::Partition::Real(handle, _) => handle,
        crate::Partition::Process => 0,
        crate::Partition::Missing => {
            crate::sys::SetLastError(crate::ERROR_PARTITION_NOT_FOUND);
            return (null_mut(), false);
        }
    };
    let heap = compat_common::partition_heap::partition_create_heap(handle, 3, 0x100, null_mut(), 0, 0, null_mut(), null_mut());
    (heap, index == crate::PARTITION_TITLE)
}

pub(super) unsafe fn create(base: Handle) -> Handle {
    let Some(first) = region(base) else {
        return null_mut();
    };
    if first.state & MEM_COMMIT == 0 {
        return null_mut();
    }
    let next = base.wrapping_byte_add(first.region_size);
    let extra = region(next).filter(|r| r.alloc_base == first.alloc_base).map_or(0, |r| r.region_size);
    let mut params = HeapParameters {
        length: size_of::<HeapParameters>() as u32,
        segment_and_decommit_parameters: [0; 6],
        initial_commit: first.region_size,
        initial_reserve: first.region_size + extra,
        commit_routine: commit,
        reserved: [0; 2],
    };
    RtlCreateHeap(0, base, 0, 0, null_mut(), (&raw mut params).cast::<c_void>())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::heap::XMemCreateWin32Heap;
    use crate::sys::{GetLastError, HeapAlloc, VirtualAlloc, VirtualFree, MEM_RELEASE, MEM_RESERVE, PAGE_READWRITE};

    #[link(name = "ntdll")]
    extern "system" {
        fn RtlDestroyHeap(heap: Handle) -> Handle;
    }

    #[test]
    fn callback_checks_its_inputs_and_reports_the_actual_commit() {
        unsafe {
            let base = VirtualAlloc(null_mut(), 0x10000, MEM_RESERVE, PAGE_READWRITE);
            let other = VirtualAlloc(null_mut(), 0x10000, MEM_RESERVE, PAGE_READWRITE);
            assert!(!base.is_null() && !other.is_null());
            assert_eq!(VirtualAlloc(base, 0x1000, MEM_COMMIT, PAGE_READWRITE), base);
            let mut address = other;
            let mut size = 0x1000;
            assert_eq!(commit(base, &mut address, &mut size), STATUS_UNSUCCESSFUL);
            assert_eq!(region(other).unwrap().state, MEM_RESERVE);
            assert_eq!(size, 0x1000);
            assert_eq!(commit(base, null_mut(), &mut size), STATUS_INVALID_PARAMETER);
            address = base.wrapping_byte_add(0x1000);
            size = 0;
            assert_eq!(commit(base, &mut address, &mut size), STATUS_INVALID_PARAMETER);
            for (requested, expected) in [(1, 4096), (4095, 4096), (4096, 8192), (4097, 8192)] {
                size = requested;
                assert_eq!(commit(base, &mut address, &mut size), 0);
                assert_eq!(size, expected);
                assert_eq!(region(address.wrapping_byte_add(size - 1)).unwrap().state, MEM_COMMIT);
                address = address.wrapping_byte_add(size);
                assert_eq!(region(address).unwrap().state, MEM_RESERVE);
            }
            size = usize::MAX;
            assert_eq!(commit(base, &mut address, &mut size), STATUS_UNSUCCESSFUL);
            assert_eq!(size, usize::MAX);
            assert_ne!(VirtualFree(other, 0, MEM_RELEASE), 0);
            assert_ne!(VirtualFree(base, 0, MEM_RELEASE), 0);
        }
    }

    #[test]
    fn legacy_creation_uses_partition_and_in_place_heaps() {
        if std::env::var_os("XMEM_HEAP_TEST_CHILD").is_none() {
            let result = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "heap::legacy::tests::legacy_creation_uses_partition_and_in_place_heaps"])
                .env("XMEM_HEAP_TEST_CHILD", "1")
                .env("TOMBOX_CONSOLE_OSBUILD", "19041")
                .status().unwrap();
            assert!(result.success());
            return;
        }
        unsafe {
            for flags in [3, 4, 7, 0x8004, u64::MAX] {
                assert!(XMemCreateWin32Heap(flags, null_mut()).is_null());
                assert_eq!(GetLastError(), 87);
            }
            for flags in [0, 0x8000] {
                let h = XMemCreateWin32Heap(flags, null_mut());
                assert!(!h.is_null());
                assert!(!HeapAlloc(h, 0, 512).is_null());
                assert!(RtlDestroyHeap(h).is_null());
            }
            let base = VirtualAlloc(null_mut(), 0x100000, MEM_RESERVE, PAGE_READWRITE);
            assert!(!base.is_null());
            assert!(XMemCreateWin32Heap(2, base).is_null());
            assert_eq!(VirtualAlloc(base, 0x10000, MEM_COMMIT, PAGE_READWRITE), base);
            let h = XMemCreateWin32Heap(0x8002, base);
            assert_eq!(h, base);
            let p = HeapAlloc(h, 0, 0x20000);
            assert!(!p.is_null());
            assert!(p as usize >= base as usize && p as usize + 0x20000 <= base as usize + 0x100000);
            core::ptr::write_bytes(p, 0x5a, 0x20000);
            assert!(HeapAlloc(h, 0, 0x100000).is_null());
            assert!(RtlDestroyHeap(h).is_null());
            assert_ne!(VirtualFree(base, 0, MEM_RELEASE), 0);
        }
    }
}
