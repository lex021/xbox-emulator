use core::ffi::c_void;
use core::sync::atomic::{AtomicI32, AtomicU32, AtomicU64, Ordering};

use crate::alloc::STATS;
use crate::sys::*;
use compat_common::record;

const M: &str = "xmem";

pub const WS_VERSION_MASK: u32 = 0xf000_0000;
pub const WS_VERSION_INTERNAL: u32 = 0x2000_0000;
pub const WS_VERSION_2: u32 = 0x1000_0000;
pub const WS_TITLE: u32 = 0x1;
pub const WS_TOOLS: u32 = 0x2;

#[repr(C)]
#[derive(Default, Clone, Copy, Debug)]
pub struct WorkingSet {
    pub title_total: u64,
    pub title_used: u64,
    pub tools_total: u64,
    pub tools_available: u64,
    pub flags: u32,
    pub io_pages: u32,
    pub commit_total: u64,
    pub commit_used: u64,
    pub reserved0: [u64; 2],
    pub pool_bytes: u32,
    pub commit_limit: u32,
    pub control_page: u32,
    pub reserved1: u32,
    pub peak_used: u64,
    pub reserved2: [u64; 3],
}

pub const TITLE_LIMIT: u64 = 0x3_6000_0000;

static PEAK_USED: AtomicU64 = AtomicU64::new(0);

pub fn title_used() -> u64 {
    if crate::title_acct::enabled() && crate::sys::env_trimmed_is(b"TOMBOX_XMEM_TITLE_USED_OLD\0", b"1") != Some(true) {
        return crate::title_acct::parts().total().min(TITLE_LIMIT);
    }
    let proc = process_memory();
    (proc.private_usage as u64 + crate::phys::live_bytes()).min(TITLE_LIMIT)
}

pub fn gather(flags: u32) -> WorkingSet {
    let mut ws = WorkingSet::default();
    if flags & WS_TITLE != 0 {
        let used = title_used();
        ws.title_total = TITLE_LIMIT;
        ws.title_used = used;
        ws.commit_total = TITLE_LIMIT;
        ws.commit_used = used;
        ws.peak_used = PEAK_USED.fetch_max(used, Ordering::Relaxed).max(used);
    }
    ws
}

const SYSTEM_PERFORMANCE_INFORMATION: u32 = 2;
const PERFORMANCE_INFO_SIZE: usize = 0x158;
pub const WS_POOL: u32 = 0x8;

pub fn pool_bytes() -> Result<u32, i32> {
    let mut buf = [0u8; PERFORMANCE_INFO_SIZE];
    let mut ret = 0u32;
    let st = unsafe { NtQuerySystemInformation(SYSTEM_PERFORMANCE_INFORMATION, buf.as_mut_ptr(), buf.len() as u32, &mut ret) };
    if st < 0 {
        return Err(st);
    }
    if ret as usize != PERFORMANCE_INFO_SIZE {
        return Err(STATUS_INFO_LENGTH_MISMATCH);
    }
    let paged = u32::from_le_bytes(buf[0x70..0x74].try_into().unwrap());
    let nonpaged = u32::from_le_bytes(buf[0x74..0x78].try_into().unwrap());
    Ok(paged.wrapping_add(nonpaged) << 12)
}

const STATUS_INFO_LENGTH_MISMATCH: i32 = 0xc000_0004u32 as i32;
const HARD_HEADROOM: (u32, u32) = (0x200_0000, 0x280_0000);
const SOFT_HEADROOM: (u32, u32) = (0x1e0_0000, 0x260_0000);
const CONSOLE_MODE_WIDE_HEADROOM: u32 = 0x60;

fn limits() {
    static INIT: std::sync::Once = std::sync::Once::new();
    INIT.call_once(|| {
        let base = pool_bytes().unwrap_or(0);
        let wide = console_mode() >= CONSOLE_MODE_WIDE_HEADROOM;
        let (h, s) = if wide { (HARD_HEADROOM.1, SOFT_HEADROOM.1) } else { (HARD_HEADROOM.0, SOFT_HEADROOM.0) };
        TRANSFER_HARD_LIMIT.fetch_add(base.wrapping_add(h), Ordering::AcqRel);
        TRANSFER_SOFT_LIMIT.fetch_add(base.wrapping_add(s), Ordering::AcqRel);
        record(M, "XMemTransferMemory", &format!("pool commit limits: base={base:#x} hard={:#x} soft={:#x}", TRANSFER_HARD_LIMIT.load(Ordering::Acquire), TRANSFER_SOFT_LIMIT.load(Ordering::Acquire)));
    });
}

pub fn hard_limit() -> u32 {
    limits();
    TRANSFER_HARD_LIMIT.load(Ordering::Acquire)
}

#[cfg(test)]
pub fn soft_limit() -> u32 {
    limits();
    TRANSFER_SOFT_LIMIT.load(Ordering::Acquire)
}

#[link(name = "ntdll", kind = "raw-dylib")]
extern "system" {
    fn RtlNtStatusToDosError(status: i32) -> u32;
}

unsafe fn wr<T: Copy>(base: *mut u8, off: usize, v: T) {
    core::ptr::write_unaligned(base.add(off) as *mut T, v);
}

#[no_mangle]
pub unsafe extern "system" fn XMemGetWorkingSetStatistics(flags: u32, out: *mut c_void) -> i32 {
    if out.is_null() {
        SetLastError(ERROR_INVALID_PARAMETER);
        return 0;
    }
    crate::title_acct::announce();
    let mut ws = gather(flags);
    if flags & WS_TITLE != 0 {
        crate::title_acct::trace("ws", &format!("flags={flags:#x}"));
    }
    if flags & WS_POOL != 0 {
        match pool_bytes() {
            Ok(b) => {
                ws.pool_bytes = b;
                ws.commit_limit = hard_limit();
            }
            Err(st) => {
                let n = match flags & WS_VERSION_MASK {
                    WS_VERSION_INTERNAL => core::mem::size_of::<WorkingSet>(),
                    WS_VERSION_2 => 0x50,
                    _ => 0x48,
                };
                core::ptr::write_bytes(out as *mut u8, 0, n);
                SetLastError(RtlNtStatusToDosError(st));
                return 0;
            }
        }
    }
    let o = out as *mut u8;
    match flags & WS_VERSION_MASK {
        WS_VERSION_INTERNAL => {
            core::ptr::write_unaligned(out as *mut WorkingSet, ws);
        }
        WS_VERSION_2 => {
            core::ptr::write_bytes(o, 0, 0x50);
            wr(o, 0x00, ws.title_total);
            wr(o, 0x08, ws.title_used);
            wr(o, 0x10, ws.tools_total);
            wr(o, 0x18, ws.tools_available);
            wr(o, 0x20, ws.flags);
            wr(o, 0x28, ws.commit_total);
            wr(o, 0x30, ws.commit_used);
            wr(o, 0x48, ws.pool_bytes);
            wr(o, 0x4c, ws.commit_limit);
        }
        _ => {
            core::ptr::write_bytes(o, 0, 0x48);
            wr(o, 0x00, ws.title_total);
            wr(o, 0x08, ws.title_used);
            wr(o, 0x10, ws.tools_total);
            wr(o, 0x18, ws.tools_available);
            wr(o, 0x20, ws.flags);
            wr(o, 0x24, ws.pool_bytes);
            wr(o, 0x28, ws.commit_total);
            wr(o, 0x30, ws.commit_used);
        }
    }
    1
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
pub struct FreePageStatistics {
    pub zeroed_bytes: u64,
    pub free_bytes: u64,
    pub standby_bytes: u64,
    pub total_bytes: u64,
    pub tools_zeroed_bytes: u64,
    pub tools_free_bytes: u64,
    pub tools_total_bytes: u64,
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct PartitionNodePages {
    total: u64,
    counts: [[u64; 2]; 4],
}

impl PartitionNodePages {
    fn free_pages(self) -> [u64; 3] {
        let [small, medium, large, huge] = self.counts.map(|[a, b]| a.wrapping_add(b));
        [small, medium, large.wrapping_add(huge << 9)]
    }
}

const CURRENT_MEMORY_PARTITION: Handle = usize::MAX as Handle;
const PARTITION_NODE_INFORMATION: u32 = 7;

#[repr(C)]
struct PartitionNodes {
    count: u32,
    reserved: u32,
    nodes: *mut PartitionNodePages,
}

unsafe fn query_partition_pages(handle: Handle, two_nodes: bool, out: *mut [[u64; 3]; 2]) -> i32 {
    let mut rows = [PartitionNodePages::default(); 2];
    let mut query = PartitionNodes {
        count: 1 + u32::from(two_nodes),
        reserved: 0,
        nodes: rows.as_mut_ptr(),
    };
    let status = crate::NtManagePartition(handle, core::ptr::null_mut(), PARTITION_NODE_INFORMATION, (&raw mut query).cast(), size_of::<PartitionNodes>() as u32);
    if status < 0 {
        SetLastError(RtlNtStatusToDosError(status));
        return 0;
    }
    let first = rows[0].free_pages();
    let second = if two_nodes { rows[1].free_pages() } else { first };
    out.write_unaligned([first, second]);
    1
}

#[no_mangle]
pub unsafe extern "system" fn XMemGetFreePageStatistics(out: *mut FreePageStatistics) -> i32 {
    if out.is_null() {
        SetLastError(ERROR_INVALID_PARAMETER);
        return 0;
    }
    if compat_common::os_host::routes::current_build() == 19041 {
        let handle = match crate::partition(crate::PARTITION_TITLE as usize) {
            crate::Partition::Real(handle, _) => handle as Handle,
            crate::Partition::Process => CURRENT_MEMORY_PARTITION,
            crate::Partition::Missing => {
                SetLastError(crate::ERROR_PARTITION_NOT_FOUND);
                return 0;
            }
        };
        return query_partition_pages(handle, multi_node(), out.cast());
    }
    let sys = system_memory();
    *out = FreePageStatistics {
        zeroed_bytes: 0,
        free_bytes: sys.avail_phys,
        standby_bytes: 0,
        total_bytes: sys.total_phys,
        ..Default::default()
    };
    1
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
pub struct PhysicalMemoryLayout {
    pub base: u64,
    pub size: u64,
}

#[no_mangle]
pub unsafe extern "system" fn XMemGetPhysicalMemoryLayout(out: *mut PhysicalMemoryLayout) -> i32 {
    if out.is_null() {
        SetLastError(ERROR_INVALID_PARAMETER);
        return 0;
    }
    if compat_common::os_host::routes::current_build() == 19041 {
        out.cast::<[u64; 2]>().write_unaligned(compat_common::physical_pages::TITLE_LAYOUT_19041);
    } else {
        *out = PhysicalMemoryLayout { base: 0, size: system_memory().total_phys };
    }
    1
}

#[link(name = "ntdll", kind = "raw-dylib")]
extern "system" {
    fn NtQuerySystemInformation(class: u32, info: *mut u8, len: u32, ret: *mut u32) -> i32;
}

#[link(name = "kernel32")]
extern "system" {
    fn GetModuleHandleA(name: *const u8) -> usize;
    fn GetProcAddress(h: usize, name: *const u8) -> usize;
}

pub const ERROR_NOT_SUPPORTED: u32 = 50;
pub const ERROR_NO_SYSTEM_RESOURCES: u32 = 1450;
const SYSTEM_NUMA_AVAILABLE_MEMORY: u32 = 0x3c;
const CONSOLE_MODE_TOOLS_ALWAYS: u32 = 0x68;
const SYSTEM_TYPE_NO_TOOLS: u16 = 0x60;

static HIGHEST_NODE: AtomicI32 = AtomicI32::new(-1);
pub static TRANSFER_SOFT_LIMIT: AtomicU32 = AtomicU32::new(0);
pub static TRANSFER_HARD_LIMIT: AtomicU32 = AtomicU32::new(0);

fn xpal_u16(name: &[u8]) -> Option<u16> {
    let h = unsafe { GetModuleHandleA(b"xpal.dll\0".as_ptr()) };
    if h == 0 {
        return None;
    }
    let f = unsafe { GetProcAddress(h, name.as_ptr()) };
    if f == 0 {
        return None;
    }
    Some(unsafe { core::mem::transmute::<usize, unsafe extern "system" fn() -> u16>(f)() })
}

pub fn console_mode() -> u32 {
    xpal_u16(b"XGetConsoleMode\0").unwrap_or(0) as u32
}

pub fn multi_node() -> bool {
    if HIGHEST_NODE.load(Ordering::Acquire) < 0 {
        let mut buf = [0u8; 0x408];
        let mut ret = 0u32;
        if unsafe { NtQuerySystemInformation(SYSTEM_NUMA_AVAILABLE_MEMORY, buf.as_mut_ptr(), buf.len() as u32, &mut ret) } >= 0 {
            HIGHEST_NODE.store(i32::from_le_bytes(buf[0..4].try_into().unwrap()), Ordering::Release);
        }
    }
    HIGHEST_NODE.load(Ordering::Acquire) > 0
}

pub fn tools_available(force: bool) -> bool {
    if console_mode() == CONSOLE_MODE_TOOLS_ALWAYS {
        return true;
    }
    if unsafe { crate::XMemGetPartitionHandleAndId(crate::PARTITION_TOOLS, core::ptr::null_mut(), core::ptr::null_mut()) } == 0 {
        return false;
    }
    if force {
        return true;
    }
    if xpal_u16(b"XGetSystemType\0") == Some(SYSTEM_TYPE_NO_TOOLS) {
        return false;
    }
    tools_device_flag()
}

fn tools_device_flag() -> bool {
    static FLAG: AtomicI32 = AtomicI32::new(-1);
    if FLAG.load(Ordering::Acquire) < 0 {
        record(M, "XMemIsToolsMemAvailable", "console asks the xmem device (IOCTL 0x1500e8) for the tools flag; PC has no such device, so the flag stays unset and tools memory is unavailable");
        FLAG.store(0, Ordering::Release);
    }
    FLAG.load(Ordering::Acquire) == 1
}

#[no_mangle]
pub extern "system" fn XMemIsToolsMemAvailable() -> i32 {
    tools_available(false) as i32
}

pub fn transfer_pages(size: usize) -> u64 {
    ((size as u64).wrapping_add(0x1f_ffff) >> 12) & 0xf_ffff_ffff_fe00
}

fn partition_transfer(source: u32, target: u32, any_node: bool, size: usize) -> bool {
    let bytes = transfer_pages(size) << 12;
    let avail = system_memory().avail_phys;
    let ok = bytes <= avail;
    record(M, "XMemTransferMemory", &format!("partition {source} -> {target} pages={:#x} any_node={any_node} avail={avail:#x} -> {ok}", bytes >> 12));
    if !ok {
        unsafe { SetLastError(ERROR_NO_SYSTEM_RESOURCES) };
    }
    ok
}

#[no_mangle]
pub extern "system" fn XMemTransferMemory(direction: u32, size: usize) -> i32 {
    if !tools_available(false) && (direction == 1 || direction == 2) {
        unsafe { SetLastError(ERROR_NOT_SUPPORTED) };
        return 0;
    }
    if size == 0 {
        return 1;
    }
    let (source, target, any_node) = match direction {
        0 => (1, 0, false),
        1 => (2, 0, false),
        2 => (2, 1, multi_node()),
        _ => {
            unsafe { SetLastError(ERROR_INVALID_PARAMETER) };
            return 0;
        }
    };
    let mut ok = partition_transfer(source, target, any_node, size);
    if !ok && !any_node && multi_node() {
        ok = partition_transfer(source, target, true, size);
    }
    if ok && direction <= 1 {
        limits();
        TRANSFER_SOFT_LIMIT.fetch_add(size as u32, Ordering::AcqRel);
        TRANSFER_HARD_LIMIT.fetch_add(size as u32, Ordering::AcqRel);
    }
    ok as i32
}

pub fn summary() -> String {
    let ws = gather(WS_TITLE);
    let mut s = format!(
        "XMem: title total={:#x} used={:#x} committed={:#x} peak={:#x}\n",
        ws.title_total, ws.title_used, ws.commit_used, ws.peak_used
    );
    for (i, t) in STATS.iter().enumerate() {
        let n = t.count.load(Ordering::Relaxed);
        if n != 0 || t.peak.load(Ordering::Relaxed) != 0 {
            s.push_str(&format!(
                "XMem: type {i:2}: {} allocations, {:#x} bytes, peak {:#x}\n",
                n,
                t.bytes.load(Ordering::Relaxed),
                t.peak.load(Ordering::Relaxed)
            ));
        }
    }
    s
}

#[no_mangle]
pub extern "system" fn XMemDumpMemoryStats(a: *mut c_void, b: *mut c_void) {
    if a.is_null() || b.is_null() {
        return;
    }
    let s = summary();
    record(M, "XMemDumpMemoryStats", &s);
    let mut z = s.into_bytes();
    z.push(0);
    unsafe { OutputDebugStringA(z.as_ptr()) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn working_set_versions_fill_their_sizes() {
        let mut big = [0xccu8; 0x80];
        assert_eq!(unsafe { XMemGetWorkingSetStatistics(WS_VERSION_2 | WS_TITLE, big.as_mut_ptr() as *mut c_void) }, 1);
        let total = u64::from_le_bytes(big[0..8].try_into().unwrap());
        let used = u64::from_le_bytes(big[8..16].try_into().unwrap());
        assert_eq!(total, TITLE_LIMIT);
        assert!(used <= total);
        assert!(big[0x38..0x48].iter().all(|&b| b == 0));
        assert!(big[0x50..].iter().all(|&b| b == 0xcc));
        let mut old = [0xccu8; 0x80];
        assert_eq!(unsafe { XMemGetWorkingSetStatistics(WS_TITLE, old.as_mut_ptr() as *mut c_void) }, 1);
        assert!(old[0x48..].iter().all(|&b| b == 0xcc));
        assert_eq!(core::mem::size_of::<WorkingSet>(), 0x78);
        let mut tools = [0u8; 0x50];
        unsafe { XMemGetWorkingSetStatistics(WS_VERSION_2 | WS_TOOLS, tools.as_mut_ptr() as *mut c_void) };
        assert!(tools.iter().all(|&b| b == 0));
    }

    #[test]
    fn series_x_title_partition_size() {
        assert_eq!(TITLE_LIMIT, 13 * (1 << 30) + (1 << 29));
        assert_eq!(TITLE_LIMIT >> 30, 13);
        assert_eq!((TITLE_LIMIT + (1 << 30) - 1) >> 30, 14);
        let mut v2 = [0u8; 0x50];
        assert_eq!(unsafe { XMemGetWorkingSetStatistics(0x1000_0001, v2.as_mut_ptr() as *mut c_void) }, 1);
        let total = u64::from_le_bytes(v2[0..8].try_into().unwrap());
        let used = u64::from_le_bytes(v2[8..16].try_into().unwrap());
        assert_eq!(total, 14_495_514_624);
        assert_eq!(u64::from_le_bytes(v2[0x28..0x30].try_into().unwrap()), total);
        assert_eq!(u64::from_le_bytes(v2[0x30..0x38].try_into().unwrap()), used);
        assert!(used <= total);
    }

    #[test]
    fn partition_page_counts_follow_the_19041_groups() {
        let row = PartitionNodePages { total: 42, counts: [[1, 2], [3, 4], [5, 6], [7, 8]] };
        assert_eq!(row.free_pages(), [3, 7, 11 + 15 * 512]);
    }

    #[test]
    fn failed_partition_query_preserves_the_19041_output() {
        let mut out = [[u64::MAX; 3]; 2];
        assert_eq!(unsafe { query_partition_pages(core::ptr::null_mut(), false, &mut out) }, 0);
        assert_eq!(unsafe { GetLastError() }, 6);
        assert_eq!(out, [[u64::MAX; 3]; 2]);
    }

    #[test]
    fn page_statistics_are_consistent() {
        let mut f = FreePageStatistics::default();
        assert_eq!(unsafe { XMemGetFreePageStatistics(&mut f) }, 1);
        assert!(f.free_bytes <= f.total_bytes && f.total_bytes > 0);
        let mut l = PhysicalMemoryLayout::default();
        assert_eq!(unsafe { XMemGetPhysicalMemoryLayout(&mut l) }, 1);
        if compat_common::os_host::routes::current_build() == 19041 {
            assert_eq!([l.base, l.size], compat_common::physical_pages::TITLE_LAYOUT_19041);
        } else {
            assert_eq!(l.size, f.total_bytes);
        }
    }

    #[test]
    fn transfer_memory_follows_console_flow() {
        assert_eq!(XMemIsToolsMemAvailable(), 0);
        assert_eq!(XMemTransferMemory(1, 0x10000), 0);
        assert_eq!(unsafe { GetLastError() }, ERROR_NOT_SUPPORTED);
        assert_eq!(XMemTransferMemory(2, 0), 0);
        assert_eq!(unsafe { GetLastError() }, ERROR_NOT_SUPPORTED);
        assert_eq!(XMemTransferMemory(3, 0), 1);
        assert_eq!(XMemTransferMemory(0, 0), 1);
        unsafe { SetLastError(0) };
        assert_eq!(XMemTransferMemory(3, 0x1000), 0);
        assert_eq!(unsafe { GetLastError() }, ERROR_INVALID_PARAMETER);
        let (before, soft) = (hard_limit(), soft_limit());
        assert_eq!(before.wrapping_sub(soft), 0x20_0000);
        let mut ws = [0u8; 0x78];
        assert_eq!(unsafe { XMemGetWorkingSetStatistics(WS_VERSION_INTERNAL | WS_POOL, ws.as_mut_ptr() as *mut c_void) }, 1);
        let pool = u32::from_le_bytes(ws[0x48..0x4c].try_into().unwrap());
        assert!(pool > 0);
        assert_eq!(u32::from_le_bytes(ws[0x4c..0x50].try_into().unwrap()), before);
        assert_eq!(XMemTransferMemory(0, 0x30_0000), 1);
        assert_eq!(hard_limit(), before.wrapping_add(0x30_0000));
        assert_eq!(soft_limit(), soft.wrapping_add(0x30_0000));
        assert_eq!(unsafe { XMemGetWorkingSetStatistics(WS_VERSION_INTERNAL | WS_POOL, ws.as_mut_ptr() as *mut c_void) }, 1);
        assert_eq!(u32::from_le_bytes(ws[0x4c..0x50].try_into().unwrap()), before.wrapping_add(0x30_0000));
        let mut v2 = [0xccu8; 0x50];
        assert_eq!(unsafe { XMemGetWorkingSetStatistics(WS_VERSION_2 | WS_POOL, v2.as_mut_ptr() as *mut c_void) }, 1);
        assert_ne!(u32::from_le_bytes(v2[0x48..0x4c].try_into().unwrap()), 0xcccc_cccc);
        assert_ne!(u32::from_le_bytes(v2[0x48..0x4c].try_into().unwrap()), 0);
        assert_eq!(u32::from_le_bytes(v2[0x4c..0x50].try_into().unwrap()), before.wrapping_add(0x30_0000));
        assert!(v2[..0x48].iter().all(|&b| b == 0));
        let mut old = [0xccu8; 0x48];
        assert_eq!(unsafe { XMemGetWorkingSetStatistics(WS_POOL, old.as_mut_ptr() as *mut c_void) }, 1);
        assert_ne!(u32::from_le_bytes(old[0x24..0x28].try_into().unwrap()), 0);
        assert_ne!(u32::from_le_bytes(old[0x24..0x28].try_into().unwrap()), 0xcccc_cccc);
        assert_eq!(transfer_pages(0x30_0000), 0x400);
        assert_eq!(transfer_pages(1), 0x200);
        assert_eq!(transfer_pages(0x20_0000), 0x200);
        assert_eq!(XMemTransferMemory(0, usize::MAX >> 4), 0);
        assert_eq!(unsafe { GetLastError() }, ERROR_NO_SYSTEM_RESOURCES);
    }
}
