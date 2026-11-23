use compat_common::kmd_bridge::{self, KmdApi};
use core::ffi::c_void;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

pub mod gpu_clock;
pub mod ported;

pub const INVALID: isize = -1;
pub const IOCTL_HANDSHAKE: u32 = 0x7900_c190;
pub const IOCTL_RELEASE: u32 = 0x7900_c194;
pub const IOCTL_DRIVER_LEVEL: u32 = 0x7900_c1f8;
pub const IOCTL_GPU_HARDWARE_CONFIGURATION: u32 = 0x7900_c1fc;
pub const IOCTL_PAGE_POOL: u32 = 0x7900_c23b;
pub const GPU_HARDWARE_CONFIGURATION_BYTES: u32 = 0x10;
pub const PAGE_POOL_REQUEST_BYTES: usize = 0x18;
pub const PAGE_POOL_PAGE_BYTES: u64 = 0x1_0000;
pub const DRIVER_LEVEL_MESSAGE: u32 = 0x1a3;
pub const ERROR_INVALID_HANDLE: u32 = 6;
pub const ERROR_NOT_ENOUGH_MEMORY: u32 = 8;
pub const ERROR_NOT_SUPPORTED: u32 = 50;
pub const ERROR_INVALID_PARAMETER: u32 = 87;
pub const ERROR_INSUFFICIENT_BUFFER: u32 = 122;
pub const ERROR_NOACCESS: u32 = 998;
pub const MHZ: u64 = 1_000_000;
pub const QUERY_DRIVER_LEVEL: u32 = 1;
pub const QUERY_PAGE_POOL: u32 = 2;
pub const QUERY_PAGE_POOL_COUNT: u32 = 3;

#[repr(C)]
struct MemoryBasicInformation {
    base: usize,
    alloc_base: usize,
    alloc_protect: u32,
    partition: u16,
    region_size: usize,
    state: u32,
    protect: u32,
    kind: u32,
}

extern "system" {
    fn CreateEventW(sa: *mut c_void, manual: i32, initial: i32, name: *const u16) -> isize;
    fn CloseHandle(h: isize) -> i32;
    fn SetLastError(e: u32);
    fn VirtualQuery(addr: *const c_void, info: *mut MemoryBasicInformation, len: usize) -> usize;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GpuInfo {
    pub frequency_hz: u64,
    pub mode: u32,
    pub compute_units: u32,
}

pub fn gpu_frequency_hz(device_type: u32) -> u64 {
    match device_type {
        2 => 853_333_333,
        3 => 914_285_714,
        4 | 5 => 1_172_000_000,
        6 => 1_563_000_000,
        _ => 1_825_000_000,
    }
}

pub fn gpu_info(device_type: u32) -> GpuInfo {
    let (mode, compute_units) = match device_type {
        2 | 3 => (0, 12),
        4 => (1, 40),
        5 => (1, 44),
        6 => (1, 20),
        _ => (2, 52),
    };
    GpuInfo { frequency_hz: gpu_frequency_hz(device_type), mode, compute_units }
}

pub fn gpu_reply(device_type: u32) -> [u8; 16] {
    let g = gpu_info(device_type);
    let mut r = [0u8; 16];
    r[0..8].copy_from_slice(&g.frequency_hz.to_le_bytes());
    r[8..12].copy_from_slice(&g.mode.to_le_bytes());
    r[12..16].copy_from_slice(&g.compute_units.to_le_bytes());
    r
}

pub fn device_type() -> u32 {
    compat_common::gdk::identity::device_type_value()
}

fn lock<T>(m: &'static Mutex<T>) -> std::sync::MutexGuard<'static, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn handles() -> &'static Mutex<HashSet<isize>> {
    static H: OnceLock<Mutex<HashSet<isize>>> = OnceLock::new();
    H.get_or_init(|| Mutex::new(HashSet::new()))
}

fn unmodeled_seen() -> &'static Mutex<HashSet<u32>> {
    static S: OnceLock<Mutex<HashSet<u32>>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(HashSet::new()))
}

fn pools() -> &'static Mutex<HashMap<u64, (u64, u32)>> {
    static P: OnceLock<Mutex<HashMap<u64, (u64, u32)>>> = OnceLock::new();
    P.get_or_init(|| Mutex::new(HashMap::new()))
}

static HANDSHAKES: AtomicI64 = AtomicI64::new(0);
static DRIVER_LEVEL: AtomicU64 = AtomicU64::new(0);
static NEXT_POOL: AtomicU64 = AtomicU64::new(1);

pub fn accessible(addr: u64, len: u64, write: bool) -> bool {
    if addr == 0 || len == 0 {
        return false;
    }
    let Some(end) = addr.checked_add(len) else { return false };
    let mut at = addr;
    while at < end {
        let mut mbi: MemoryBasicInformation = unsafe { core::mem::zeroed() };
        if unsafe { VirtualQuery(at as usize as *const c_void, &mut mbi, core::mem::size_of::<MemoryBasicInformation>()) } == 0 {
            return false;
        }
        let prot = mbi.protect & 0xff;
        let guard = mbi.protect & 0x101 != 0;
        let ok = mbi.state == 0x1000 && !guard && if write { matches!(prot, 0x04 | 0x08 | 0x40 | 0x80) } else { matches!(prot, 0x02 | 0x04 | 0x08 | 0x20 | 0x40 | 0x80) };
        if !ok {
            return false;
        }
        let next = (mbi.base as u64).saturating_add(mbi.region_size as u64);
        if next <= at {
            return false;
        }
        at = next;
    }
    true
}

fn driver_level_ioctl(input: &[u8]) -> Result<u32, u32> {
    if input.len() < 4 {
        return Err(ERROR_INSUFFICIENT_BUFFER);
    }
    let v = u32::from_le_bytes(input[..4].try_into().unwrap());
    DRIVER_LEVEL.store((1u64 << 32) | v as u64, Ordering::Release);
    Ok(0)
}

pub fn posted_driver_level() -> Option<u32> {
    let v = DRIVER_LEVEL.load(Ordering::Acquire);
    (v >> 32 != 0).then_some(v as u32)
}

fn page_pool_ioctl(input: &[u8]) -> Result<u32, u32> {
    if input.len() < PAGE_POOL_REQUEST_BYTES {
        return Err(ERROR_INVALID_PARAMETER);
    }
    let va = u64::from_le_bytes(input[0..8].try_into().unwrap());
    let pages = u32::from_le_bytes(input[8..12].try_into().unwrap());
    let out = u64::from_le_bytes(input[16..24].try_into().unwrap());
    if !ported::process_entry_exists() {
        return Err(ERROR_NOT_ENOUGH_MEMORY);
    }
    if out & 3 != 0 || !accessible(out, 8, true) {
        return Err(ERROR_NOACCESS);
    }
    if va != 0 && pages != 0 {
        let handle = if accessible(va, (pages as u64) << 16, true) {
            let h = NEXT_POOL.fetch_add(1, Ordering::AcqRel);
            lock(pools()).insert(h, (va, pages));
            h
        } else {
            0
        };
        unsafe { core::ptr::write_unaligned(out as usize as *mut u64, handle) };
        return if handle == 0 { Err(ERROR_NOT_ENOUGH_MEMORY) } else { Ok(0) };
    }
    let handle = unsafe { core::ptr::read_unaligned(out as usize as *const u64) };
    if handle == 0 || lock(pools()).remove(&handle).is_none() {
        return Err(ERROR_INVALID_PARAMETER);
    }
    Ok(0)
}

pub fn page_pool(handle: u64) -> Option<(u64, u32)> {
    lock(pools()).get(&handle).copied()
}

pub fn page_pool_count() -> usize {
    lock(pools()).len()
}

pub fn handshakes() -> i64 {
    HANDSHAKES.load(Ordering::Acquire)
}

pub fn dispatch(code: u32, input: &[u8], output: &mut [u8], device_type: u32) -> Result<u32, u32> {
    dispatch_mut(code, &mut input.to_vec(), output, device_type)
}

fn dispatch_mut(code: u32, input: &mut [u8], output: &mut [u8], device_type: u32) -> Result<u32, u32> {
    match code {
        IOCTL_HANDSHAKE => {
            ported::create_process_entry();
            HANDSHAKES.fetch_add(1, Ordering::AcqRel);
            Ok(0)
        }
        IOCTL_RELEASE => {
            if !ported::process_entry_exists() {
                return Err(ERROR_INVALID_PARAMETER);
            }
            HANDSHAKES.fetch_sub(1, Ordering::AcqRel);
            Ok(0)
        }
        IOCTL_DRIVER_LEVEL => driver_level_ioctl(input),
        IOCTL_GPU_HARDWARE_CONFIGURATION => {
            if output.len() < GPU_HARDWARE_CONFIGURATION_BYTES as usize {
                return Err(ERROR_INSUFFICIENT_BUFFER);
            }
            output[..16].copy_from_slice(&gpu_reply(device_type));
            Ok(GPU_HARDWARE_CONFIGURATION_BYTES)
        }
        IOCTL_PAGE_POOL => page_pool_ioctl(input),
        gpu_clock::IOCTL_CLOCK_CALIBRATION => gpu_clock::calibration_ioctl(input),
        gpu_clock::IOCTL_TIMESTAMP_PAIR => gpu_clock::timestamp_pair_ioctl(output),
        _ => match ported::dispatch(code, input, output) {
            Some(r) => r,
            None => {
                if lock(unmodeled_seen()).insert(code) {
                    compat_common::record("xgs_kmd_emu", "DeviceIoControl", &format!("IOCTL {code:#010x} is not modeled by the in-process KMD emulation; failing with ERROR_NOT_SUPPORTED"));
                }
                Err(ERROR_NOT_SUPPORTED)
            }
        },
    }
}

fn neither_input_len(code: u32) -> Option<usize> {
    match code {
        IOCTL_PAGE_POOL => Some(PAGE_POOL_REQUEST_BYTES),
        gpu_clock::IOCTL_CLOCK_CALIBRATION => Some(gpu_clock::CLOCK_CALIBRATION_REQUEST_BYTES),
        _ => ported::neither_input_len(code),
    }
}

unsafe extern "C" fn local_open() -> isize {
    let h = CreateEventW(core::ptr::null_mut(), 1, 0, core::ptr::null());
    if h == 0 {
        return INVALID;
    }
    lock(handles()).insert(h);
    h
}

unsafe extern "C" fn local_is_device(h: isize) -> i32 {
    lock(handles()).contains(&h) as i32
}

unsafe extern "C" fn local_close(h: isize) -> i32 {
    let removed = lock(handles()).remove(&h);
    if removed {
        CloseHandle(h);
    } else {
        SetLastError(ERROR_INVALID_HANDLE);
    }
    removed as i32
}

unsafe extern "C" fn local_ioctl(h: isize, code: u32, inb: *const c_void, inl: u32, outb: *mut c_void, outl: u32, ret: *mut u32) -> i32 {
    let fail = |e: u32| {
        if !ret.is_null() {
            *ret = 0;
        }
        SetLastError(e);
        0
    };
    if local_is_device(h) == 0 {
        return fail(ERROR_INVALID_HANDLE);
    }
    let input: &mut [u8] = match neither_input_len(code) {
        Some(n) => {
            if inb.is_null() {
                return fail(ERROR_INVALID_PARAMETER);
            }
            if inb as usize & 3 != 0 || !accessible(inb as u64, n as u64, false) {
                return fail(ERROR_NOACCESS);
            }
            core::slice::from_raw_parts_mut(inb as *mut u8, n)
        }
        None => {
            if inb.is_null() || inl == 0 {
                &mut []
            } else if !accessible(inb as u64, inl as u64, false) {
                return fail(ERROR_NOACCESS);
            } else {
                core::slice::from_raw_parts_mut(inb as *mut u8, inl as usize)
            }
        }
    };
    let output: &mut [u8] = if outb.is_null() || outl == 0 {
        &mut []
    } else if !accessible(outb as u64, outl as u64, true) {
        return fail(ERROR_NOACCESS);
    } else {
        core::slice::from_raw_parts_mut(outb as *mut u8, outl as usize)
    };
    match dispatch_mut(code, input, output, device_type()) {
        Ok(n) => {
            if !ret.is_null() {
                *ret = n;
            }
            1
        }
        Err(e) => fail(e),
    }
}

unsafe extern "C" fn local_query(kind: u32, arg: u64, out: *mut u64) -> i32 {
    if out.is_null() {
        return 0;
    }
    match kind {
        QUERY_DRIVER_LEVEL => match posted_driver_level() {
            Some(v) => {
                *out = v as u64;
                1
            }
            None => 0,
        },
        QUERY_PAGE_POOL => match page_pool(arg) {
            Some((va, pages)) => {
                *out = va;
                *out.add(1) = pages as u64;
                1
            }
            None => 0,
        },
        QUERY_PAGE_POOL_COUNT => {
            *out = page_pool_count() as u64;
            1
        }
        _ => 0,
    }
}

static LOCAL: KmdApi = KmdApi {
    version: kmd_bridge::VERSION,
    open: local_open,
    is_device: local_is_device,
    close: local_close,
    ioctl: local_ioctl,
    query: local_query,
};

pub fn api() -> &'static KmdApi {
    kmd_bridge::publish(&LOCAL)
}

pub fn open() -> isize {
    unsafe { (api().open)() }
}

pub fn is_device(h: isize) -> bool {
    unsafe { (api().is_device)(h) != 0 }
}

pub fn close(h: isize) -> bool {
    unsafe { (api().close)(h) != 0 }
}

pub unsafe fn device_io_control(h: isize, code: u32, inb: *const c_void, inl: u32, outb: *mut c_void, outl: u32, ret: &mut u32) -> bool {
    (api().ioctl)(h, code, inb, inl, outb, outl, ret) != 0
}

pub fn driver_level() -> Option<u32> {
    let mut v = 0u64;
    (unsafe { (api().query)(QUERY_DRIVER_LEVEL, 0, &mut v) } != 0).then_some(v as u32)
}

pub fn registered_page_pool(handle: u64) -> Option<(u64, u32)> {
    let mut v = [0u64; 2];
    (unsafe { (api().query)(QUERY_PAGE_POOL, handle, v.as_mut_ptr()) } != 0).then_some((v[0], v[1] as u32))
}

pub fn registered_page_pool_count() -> usize {
    let mut v = 0u64;
    unsafe { (api().query)(QUERY_PAGE_POOL_COUNT, 0, &mut v) };
    v as usize
}

#[cfg(test)]
pub fn test_serial() -> std::sync::MutexGuard<'static, ()> {
    static S: Mutex<()> = Mutex::new(());
    S.lock().unwrap_or_else(|e| e.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    extern "system" {
        fn GetLastError() -> u32;
    }

    #[test]
    fn gpu_config_per_console() {
        let f: Vec<u64> = (2..=8).map(gpu_frequency_hz).collect();
        assert_eq!(f, vec![853_333_333, 914_285_714, 1_172_000_000, 1_172_000_000, 1_563_000_000, 1_825_000_000, 1_825_000_000]);
        let m: Vec<(u32, u32)> = (2..=8).map(|t| (gpu_info(t).mode, gpu_info(t).compute_units)).collect();
        assert_eq!(m, vec![(0, 12), (0, 12), (1, 40), (1, 44), (1, 20), (2, 52), (2, 52)]);
        let r = gpu_reply(4);
        assert_eq!(u64::from_le_bytes(r[0..8].try_into().unwrap()), 1_172_000_000);
        assert_eq!(u32::from_le_bytes(r[8..12].try_into().unwrap()), 1);
        assert_eq!(u32::from_le_bytes(r[12..16].try_into().unwrap()), 40);
    }

    #[test]
    fn gpu_config_ioctl_checks_length_like_kernel() {
        let mut small = [0u8; 15];
        assert_eq!(dispatch(IOCTL_GPU_HARDWARE_CONFIGURATION, &[], &mut small, 7), Err(ERROR_INSUFFICIENT_BUFFER));
        let mut big = [0xccu8; 0x18];
        assert_eq!(dispatch(IOCTL_GPU_HARDWARE_CONFIGURATION, &[], &mut big, 7), Ok(0x10));
        assert_eq!(&big[..16], &gpu_reply(7));
        assert_eq!(&big[16..], &[0xcc; 8]);
    }

    #[test]
    fn unknown_ioctls_fail() {
        let mut out = [0u8; 0x10];
        for code in [0x7900_c000u32, 0x7900_c28b, 0x1234_5678] {
            assert_eq!(dispatch(code, &[], &mut out, 7), Err(ERROR_NOT_SUPPORTED));
        }
    }

    #[test]
    fn driver_level_requires_four_bytes_and_posts_value() {
        let _g = test_serial();
        let mut out = [0u8; 4];
        assert_eq!(dispatch(IOCTL_DRIVER_LEVEL, &[1, 2, 3], &mut out, 7), Err(ERROR_INSUFFICIENT_BUFFER));
        assert_eq!(dispatch(IOCTL_DRIVER_LEVEL, &7u32.to_le_bytes(), &mut out, 7), Ok(0));
        assert_eq!(driver_level(), Some(7));
        assert_eq!(dispatch(IOCTL_DRIVER_LEVEL, &[9, 0, 0, 0, 0xff], &mut [], 7), Ok(0));
        assert_eq!(posted_driver_level(), Some(9));
    }

    fn pool_req(va: u64, pages: u32, out: *mut u64) -> [u8; 0x18] {
        let mut r = [0u8; 0x18];
        r[0..8].copy_from_slice(&va.to_le_bytes());
        r[8..12].copy_from_slice(&pages.to_le_bytes());
        r[16..24].copy_from_slice(&(out as u64).to_le_bytes());
        r
    }

    #[test]
    fn page_pool_register_and_unregister_like_kernel() {
        let _g = test_serial();
        let mem = vec![0u8; 0x3_0000];
        let va = ((mem.as_ptr() as u64) + 0xffff) & !0xffff;
        ported::create_process_entry();
        let mut out = 0u64;
        let before = page_pool_count();
        assert_eq!(dispatch(IOCTL_PAGE_POOL, &pool_req(va, 1, &mut out), &mut [], 7), Ok(0));
        let a = out;
        assert_ne!(a, 0);
        assert_eq!(registered_page_pool(a), Some((va, 1)));
        assert_eq!(dispatch(IOCTL_PAGE_POOL, &pool_req(va, 1, &mut out), &mut [], 7), Ok(0));
        let b = out;
        assert_ne!(a, b);
        assert_eq!(registered_page_pool_count(), before + 2);
        out = a;
        assert_eq!(dispatch(IOCTL_PAGE_POOL, &pool_req(0, 0, &mut out), &mut [], 7), Ok(0));
        assert_eq!(page_pool(a), None);
        assert_eq!(dispatch(IOCTL_PAGE_POOL, &pool_req(0, 0, &mut out), &mut [], 7), Err(ERROR_INVALID_PARAMETER));
        out = 0;
        assert_eq!(dispatch(IOCTL_PAGE_POOL, &pool_req(0, 0, &mut out), &mut [], 7), Err(ERROR_INVALID_PARAMETER));
        out = b;
        assert_eq!(dispatch(IOCTL_PAGE_POOL, &pool_req(va, 0, &mut out), &mut [], 7), Ok(0));
        assert_eq!(page_pool_count(), before);
        out = 0x55;
        assert_eq!(dispatch(IOCTL_PAGE_POOL, &pool_req(0x10000, 4, &mut out), &mut [], 7), Err(ERROR_NOT_ENOUGH_MEMORY));
        assert_eq!(out, 0);
        assert_eq!(dispatch(IOCTL_PAGE_POOL, &pool_req(va, 1, core::ptr::null_mut()), &mut [], 7), Err(ERROR_NOACCESS));
        assert_eq!(dispatch(IOCTL_PAGE_POOL, &[0u8; 8], &mut [], 7), Err(ERROR_INVALID_PARAMETER));
        drop(mem);
    }

    #[test]
    fn page_pool_ioctl_is_method_neither() {
        let _g = test_serial();
        ported::create_process_entry();
        let h = open();
        let mut ret = 7u32;
        assert!(!unsafe { device_io_control(h, IOCTL_PAGE_POOL, core::ptr::null(), 0x18, core::ptr::null_mut(), 0, &mut ret) });
        assert_eq!(unsafe { GetLastError() }, ERROR_INVALID_PARAMETER);
        let mem = vec![0u8; 0x2_0000];
        let va = ((mem.as_ptr() as u64) + 0xffff) & !0xffff;
        let mut out = 0u64;
        let req = pool_req(va, 1, &mut out);
        assert!(unsafe { device_io_control(h, IOCTL_PAGE_POOL, req.as_ptr().cast(), 0, core::ptr::null_mut(), 0, &mut ret) });
        assert_eq!(ret, 0);
        assert_eq!(registered_page_pool(out), Some((va, 1)));
        let req = pool_req(0, 0, &mut out);
        assert!(unsafe { device_io_control(h, IOCTL_PAGE_POOL, req.as_ptr().cast(), 0x18, core::ptr::null_mut(), 0, &mut ret) });
        assert_eq!(registered_page_pool(out), None);
        assert!(close(h));
    }

    #[test]
    fn device_handle_routes_until_closed() {
        let _g = test_serial();
        let h = open();
        assert_ne!(h, INVALID);
        assert!(is_device(h));
        let mut out = [0u8; 16];
        let mut ret = 0u32;
        assert!(unsafe { device_io_control(h, IOCTL_GPU_HARDWARE_CONFIGURATION, core::ptr::null(), 0, out.as_mut_ptr().cast(), 16, &mut ret) });
        assert_eq!(ret, 16);
        assert_eq!(out, gpu_reply(device_type()));
        assert!(!unsafe { device_io_control(h, IOCTL_GPU_HARDWARE_CONFIGURATION, core::ptr::null(), 0, out.as_mut_ptr().cast(), 8, &mut ret) });
        assert_eq!(unsafe { GetLastError() }, ERROR_INSUFFICIENT_BUFFER);
        assert!(!unsafe { device_io_control(h, 0x7900_c000, core::ptr::null(), 0, out.as_mut_ptr().cast(), 4, &mut ret) });
        assert_eq!(unsafe { GetLastError() }, ERROR_NOT_SUPPORTED);
        let before = handshakes();
        assert!(unsafe { device_io_control(h, IOCTL_HANDSHAKE, core::ptr::null(), 0, core::ptr::null_mut(), 0, &mut ret) });
        assert_eq!(ret, 0);
        assert!(unsafe { device_io_control(h, IOCTL_RELEASE, core::ptr::null(), 0, core::ptr::null_mut(), 0, &mut ret) });
        assert_eq!(handshakes(), before);
        assert!(close(h));
        assert!(!is_device(h));
        assert!(!close(h));
        assert!(!unsafe { device_io_control(h, IOCTL_GPU_HARDWARE_CONFIGURATION, core::ptr::null(), 0, out.as_mut_ptr().cast(), 16, &mut ret) });
        assert_eq!(unsafe { GetLastError() }, ERROR_INVALID_HANDLE);
    }

    #[test]
    fn owner_is_published_once_per_process() {
        let a = api() as *const KmdApi;
        assert_eq!(a, api() as *const KmdApi);
        assert_eq!(kmd_bridge::owner().map(|o| o as *const KmdApi), Some(a));
        let v = std::env::var(kmd_bridge::VAR).unwrap();
        assert!(v.ends_with(&format!(":{:x}", a as usize)));
    }

    #[test]
    fn title_iat_hooks_route_the_kmd_device_path() {
        let _g = test_serial();
        let _ = api();
        let name: Vec<u16> = "\\\\.\\XgsKmd10".encode_utf16().chain(Some(0)).collect();
        let h = unsafe { kmd_bridge::create_file_w(name.as_ptr(), 0xc000_0000, 3, core::ptr::null_mut(), 3, 0, 0) };
        assert!(is_device(h));
        let mut out = [0u8; 16];
        let mut ret = 0u32;
        assert_eq!(unsafe { kmd_bridge::device_io_control(h, IOCTL_GPU_HARDWARE_CONFIGURATION, core::ptr::null(), 0, out.as_mut_ptr().cast(), 16, &mut ret, core::ptr::null_mut()) }, 1);
        assert_eq!((ret, out), (16, gpu_reply(device_type())));
        assert_eq!(unsafe { kmd_bridge::close_handle(h) }, 1);
        assert!(!is_device(h));
    }
}
