use super::console::reset_security_watermark_in;
use super::store::{self, Store};
use super::tests_console::{env_lock, temp_root};
use super::tlg;
use super::watermark::*;
use core::ffi::c_void;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI32, AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};

pub const FN_WLDP: &str = r"D:\xbox-pc-port\games\Fortnite\extracted\EmbeddedXvd\Windows\System32\wldp.dll";
pub const FN_ERA_VBI: &str = r"D:\xbox-pc-port\games\Fortnite\extracted\era.vbi";

const FN_IAT_LOADLIBRARYEXW: usize = 0x19810;
const FN_IAT_GETVERSIONEXW: usize = 0x199c0;
const FN_IAT_EXPAND: usize = 0x19868;
const FN_IAT_MANUFACTURING: usize = 0x199e0;
const FN_IAT_FILTER_BOOT_OPTION: usize = 0x19c38;
const FN_IAT_NTQSI: usize = 0x19c48;
const FN_IAT_CREATEFILEW: usize = 0x19798;
const FN_IAT_DELETEFILEW: usize = 0x197a0;

#[link(name = "kernel32")]
extern "system" {
    fn LoadLibraryExW(name: *const u16, file: *mut c_void, flags: u32) -> *mut c_void;
    fn GetProcAddress(module: *mut c_void, name: *const u8) -> *mut c_void;
    fn VirtualProtect(addr: *mut c_void, size: usize, prot: u32, old: *mut u32) -> i32;
    fn ExpandEnvironmentStringsW(src: *const u16, dst: *mut u16, n: u32) -> u32;
    fn CreateFileW(name: *const u16, access: u32, share: u32, sa: *mut c_void, disp: u32, flags: u32, tmpl: *mut c_void) -> *mut c_void;
    fn DeleteFileW(name: *const u16) -> i32;
    fn GetVersionExW(info: *mut u8) -> i32;
    fn SetLastError(e: u32);
}

#[link(name = "ntdll")]
extern "system" {
    fn NtQuerySystemInformation(class: u32, buf: *mut c_void, len: u32, ret: *mut u32) -> i32;
}

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

pub unsafe fn wstr(p: *const u16) -> String {
    let mut n = 0;
    while *p.add(n) != 0 {
        n += 1;
    }
    String::from_utf16_lossy(std::slice::from_raw_parts(p, n))
}

pub unsafe fn patch(base: usize, rva: usize, value: usize) {
    let a = (base + rva) as *mut c_void;
    let mut old = 0u32;
    assert!(VirtualProtect(a, 8, 4, &mut old) != 0);
    *(a as *mut usize) = value;
    VirtualProtect(a, 8, old, &mut old);
}

pub fn load(path: &str) -> usize {
    let w = wide(path);
    unsafe { LoadLibraryExW(w.as_ptr(), core::ptr::null_mut(), 8) as usize }
}

pub fn sandbox() -> &'static Mutex<PathBuf> {
    static S: OnceLock<Mutex<PathBuf>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(PathBuf::new()))
}

fn sandbox_store() -> Store {
    Store::at(sandbox().lock().unwrap_or_else(|e| e.into_inner()).clone())
}

pub unsafe extern "system" fn hook_expand(src: *const u16, dst: *mut u16, n: u32) -> u32 {
    let s = wstr(src);
    let lower = s.to_ascii_lowercase();
    if !lower.starts_with("%systemdrive%") && !lower.starts_with("%systemroot%") {
        return ExpandEnvironmentStringsW(src, dst, n);
    }
    let p = sandbox_store().console_path(&s);
    let w = wide(&p.to_string_lossy());
    if (w.len() as u32) <= n && !dst.is_null() {
        core::ptr::copy_nonoverlapping(w.as_ptr(), dst, w.len());
    }
    w.len() as u32
}

unsafe fn title_relative(name: *const u16) -> Option<Vec<u16>> {
    let s = wstr(name);
    s.starts_with('%').then(|| wide(&sandbox_store().title_relative_path(&s).to_string_lossy()))
}

pub unsafe extern "system" fn hook_create(name: *const u16, access: u32, share: u32, sa: *mut c_void, disp: u32, flags: u32, tmpl: *mut c_void) -> *mut c_void {
    match title_relative(name) {
        Some(w) => CreateFileW(w.as_ptr(), access, share, sa, disp, flags, tmpl),
        None => CreateFileW(name, access, share, sa, disp, flags, tmpl),
    }
}

pub unsafe extern "system" fn hook_delete(name: *const u16) -> i32 {
    match title_relative(name) {
        Some(w) => DeleteFileW(w.as_ptr()),
        None => DeleteFileW(name),
    }
}

static FILTER_STATUS: AtomicI32 = AtomicI32::new(0);
static FILTER_CALLS: Mutex<Vec<(u32, u32, u32)>> = Mutex::new(Vec::new());
static MFG: AtomicU32 = AtomicU32::new(0);
static MFG_ERROR: AtomicU32 = AtomicU32::new(0);
static NTQSI_CLASSES: Mutex<Vec<u32>> = Mutex::new(Vec::new());

unsafe extern "system" fn hook_filter(kind: u32, store_type: u32, element: u32, data: *const u8, len: u32) -> i32 {
    assert_eq!((kind, len, *data), (1, 1, 1));
    FILTER_CALLS.lock().unwrap().push((kind, store_type, element));
    FILTER_STATUS.load(Ordering::SeqCst)
}

unsafe extern "system" fn hook_ntqsi(class: u32, buf: *mut c_void, len: u32, ret: *mut u32) -> i32 {
    NTQSI_CLASSES.lock().unwrap().push(class);
    match class {
        0xa6 => {
            if !ret.is_null() {
                *ret = 0;
            }
            ERA_HSTI_STATUS
        }
        0x91 | 0x8f => ERA_SECURE_BOOT_STATUS,
        _ => NtQuerySystemInformation(class, buf, len, ret),
    }
}

unsafe extern "system" fn hook_manufacturing(mode: *mut u32) -> i32 {
    let e = MFG_ERROR.load(Ordering::SeqCst);
    if e != 0 {
        SetLastError(e);
        return 0;
    }
    *mode = MFG.load(Ordering::SeqCst);
    1
}

unsafe extern "system" fn hook_version(info: *mut u8) -> i32 {
    let r = GetVersionExW(info);
    *info.add(0x11a) = ERA_PRODUCT_TYPE;
    r
}

unsafe extern "system" fn hook_loadlibrary(name: *const u16, file: *mut c_void, flags: u32) -> *mut c_void {
    if wstr(name).eq_ignore_ascii_case("DMCfgUtils.dll") {
        SetLastError(126);
        return core::ptr::null_mut();
    }
    LoadLibraryExW(name, file, flags)
}

type CheckFn = unsafe extern "system" fn(*mut u32) -> i32;

fn console_check() -> Option<CheckFn> {
    static F: OnceLock<Option<usize>> = OnceLock::new();
    let f = *F.get_or_init(|| {
        if !Path::new(FN_WLDP).is_file() || unsafe { *(0x7ffe_02d4usize as *const u8) } != 0 {
            return None;
        }
        let base = load(FN_WLDP);
        if base == 0 {
            return None;
        }
        unsafe {
            console_sandbox::attach(base);
            patch(base, FN_IAT_LOADLIBRARYEXW, hook_loadlibrary as usize);
            patch(base, FN_IAT_GETVERSIONEXW, hook_version as usize);
            patch(base, FN_IAT_EXPAND, hook_expand as usize);
            patch(base, FN_IAT_MANUFACTURING, hook_manufacturing as usize);
            patch(base, FN_IAT_FILTER_BOOT_OPTION, hook_filter as usize);
            patch(base, FN_IAT_NTQSI, hook_ntqsi as usize);
            patch(base, FN_IAT_CREATEFILEW, hook_create as usize);
            patch(base, FN_IAT_DELETEFILEW, hook_delete as usize);
            let p = GetProcAddress(base as *mut c_void, b"WldpCheckSecurityWatermarkState\0".as_ptr()) as usize;
            assert_eq!(p, base + 0x14ab0);
            Some(p)
        }
    });
    f.map(|p| unsafe { core::mem::transmute::<usize, CheckFn>(p) })
}

#[derive(Debug, PartialEq, Eq)]
struct Snapshot {
    hr: i32,
    out: u32,
    hsti: Option<Vec<u8>>,
    sb: Option<Vec<u8>>,
    log: Option<Vec<u8>>,
}

struct Scenario {
    filter: i32,
    manufacturing: Result<bool, u32>,
    hsti: Option<Vec<u8>>,
    sb: Option<Vec<u8>>,
    log_dir: bool,
    stale_log: bool,
}

impl Scenario {
    fn retail() -> Scenario {
        Scenario { filter: ERA_FILTER_BOOT_OPTION_STATUS, manufacturing: Ok(false), hsti: None, sb: None, log_dir: false, stale_log: false }
    }

    fn prepare(&self, tag: &str) -> Store {
        let s = Store::at(temp_root(tag));
        if let Some(b) = &self.hsti {
            std::fs::write(s.console_path(store::DPP_HSTI), b).unwrap();
        }
        if let Some(b) = &self.sb {
            std::fs::write(s.console_path(store::DPP_SB), b).unwrap();
        }
        let log = s.title_relative_path(store::NON_PRODUCTION_ERRORS);
        if self.log_dir || self.stale_log {
            std::fs::create_dir_all(log.parent().unwrap()).unwrap();
        }
        if self.stale_log {
            std::fs::write(&log, b"stale").unwrap();
        }
        s
    }

    fn snapshot(s: &Store, hr: i32, out: u32) -> Snapshot {
        Snapshot {
            hr,
            out,
            hsti: std::fs::read(s.console_path(store::DPP_HSTI)).ok(),
            sb: std::fs::read(s.console_path(store::DPP_SB)).ok(),
            log: std::fs::read(s.title_relative_path(store::NON_PRODUCTION_ERRORS)).ok(),
        }
    }

    fn port(&self, tag: &str) -> Snapshot {
        let s = self.prepare(&format!("{tag}_port"));
        let o = check_security_watermark_state_in(&s, &Era { filter_boot_option: self.filter, manufacturing: self.manufacturing });
        Scenario::snapshot(&s, o.hr, o.out)
    }

    fn console(&self, f: CheckFn, tag: &str) -> Snapshot {
        let s = self.prepare(&format!("{tag}_console"));
        *sandbox().lock().unwrap() = s.root().to_path_buf();
        FILTER_STATUS.store(self.filter, Ordering::SeqCst);
        match self.manufacturing {
            Ok(m) => {
                MFG.store(m as u32, Ordering::SeqCst);
                MFG_ERROR.store(0, Ordering::SeqCst);
            }
            Err(e) => MFG_ERROR.store(e, Ordering::SeqCst),
        }
        let mut out = 0xdead_beefu32;
        let hr = unsafe { f(&mut out) };
        Scenario::snapshot(&s, hr, out)
    }
}

fn zero_hash() -> Option<Vec<u8>> {
    Some(vec![0u8; HASH_SIZE])
}

#[test]
fn era_retail_fresh_state_reports_not_watermarked() {
    let _g = env_lock();
    let s = Store::at(temp_root("wm_fresh"));
    std::fs::write(s.console_path(store::DPP_HSTI), b"old").unwrap();
    std::fs::write(s.console_path(store::DPP_SB), b"old").unwrap();
    let o = check_security_watermark_state_in(&s, &Era::retail(&s));
    assert_eq!(o.hr, 0x9043_0006u32 as i32);
    assert_eq!(o.out, 1);
    assert!(o.bcd_banned && !o.manufacturing && !o.hsti_verified && !o.secure_boot);
    assert!(o.lines.is_empty());
    assert!(!s.file_exists(store::DPP_HSTI));
    assert!(!s.file_exists(store::DPP_SB));
    assert!(s.title_cwd().is_dir());
    assert!(!s.title_relative_path(store::NON_PRODUCTION_ERRORS).exists());
}

#[test]
fn export_uses_store_state() {
    let _g = env_lock();
    let d = temp_root("wm_export");
    std::env::set_var("TOMBOX_GDK_ROOT", &d);
    let mut out = 7u32;
    let hr = unsafe { WldpCheckSecurityWatermarkState(&mut out) };
    std::env::remove_var("TOMBOX_GDK_ROOT");
    assert_eq!((hr, out), (0x9043_0006u32 as i32, 1));
    assert!(d.join("wldp").join("TitleCwd").is_dir());
}

#[test]
fn filter_hit_matches_console_compare() {
    assert!(filter_hit(0));
    assert!(filter_hit(0x1000_0000));
    assert!(filter_hit(0x8043_0006u32 as i32));
    assert!(filter_hit(0x9043_0006u32 as i32));
    assert!(!filter_hit(0xC000_0034u32 as i32));
    assert!(!filter_hit(0xC000_000Du32 as i32));
    assert!(!filter_hit(1));
}

#[test]
fn zero_hashes_skip_checks_and_clear_log() {
    let _g = env_lock();
    let sc = Scenario { filter: 0xC000_0034u32 as i32, hsti: zero_hash(), sb: zero_hash(), stale_log: true, ..Scenario::retail() };
    let p = sc.port("wm_zero");
    assert_eq!((p.hr, p.out), (0, 0));
    assert_eq!(p.hsti, zero_hash());
    assert_eq!(p.sb, zero_hash());
    assert_eq!(p.log, None);
}

#[test]
fn manufacturing_failure_returns_win32_hresult() {
    let _g = env_lock();
    let sc = Scenario { manufacturing: Err(50), ..Scenario::retail() };
    let p = sc.port("wm_mfgerr");
    assert_eq!((p.hr, p.out), (0x8007_0032u32 as i32, 1));
}

#[test]
fn vsnprintf_formats_console_conversions() {
    assert_eq!(vsnprintf(b"a 0x%08lx b\n", &[0x1600_0049], LOG_LINE_MAX).unwrap(), b"a 0x16000049 b\n");
    assert_eq!(vsnprintf(b"%x %4u %d %%", &[0xab, 7, u32::MAX], LOG_LINE_MAX).unwrap(), b"ab    7 -1 %");
    assert_eq!(vsnprintf(b"abcd", &[], 3), None);
}

#[test]
fn reset_matches_console_export_order() {
    let _g = env_lock();
    let s = Store::at(temp_root("wm_reset"));
    std::fs::write(s.console_path(store::DPP_SB), b"x").unwrap();
    assert_eq!(reset_security_watermark_in(&s), 0);
    assert!(!s.file_exists(store::DPP_SB));
}

#[test]
fn watermark_tlg_blobs_match_console_image() {
    assert_eq!(tlg::Guid::from_le(tlg::WATERMARK_GUID), tlg::Guid { data1: 0x9a2edb8f, data2: 0x5883, data3: 0x499f, data4: [0xac, 0xed, 0x6e, 0x4b, 0x69, 0xd4, 0x3d, 0xdf] });
    assert_eq!(&tlg::RETAIL_UNLOCKED_EVENT_DESC[..3], &[0x0b, 5, 0]);
    assert_eq!(tlg::desc_keyword(tlg::RETAIL_UNLOCKED_EVENT_DESC), 0x8000_0000_0000);
    assert_eq!(tlg::desc_keyword(tlg::HSTI_FAILURE_EVENT_DESC), 0x4000_0000_0000);
    let Ok(dll) = std::fs::read(FN_WLDP) else { return };
    let img = Image { path: FN_WLDP.into(), bytes: dll };
    let meta = img.slice(0x1c5aa - 16, 16 + tlg::WATERMARK_META.len()).unwrap();
    assert_eq!(&meta[..16], tlg::WATERMARK_GUID);
    assert_eq!(&meta[16..], tlg::WATERMARK_META);
    for (rva, desc, m) in [(0x1c56c, tlg::RETAIL_UNLOCKED_EVENT_DESC, tlg::RETAIL_UNLOCKED_EVENT_META), (0x1c521, tlg::HSTI_FAILURE_EVENT_DESC, tlg::HSTI_FAILURE_EVENT_META)] {
        assert_eq!(img.slice(rva, 11).unwrap(), desc);
        assert_eq!(img.slice(rva + 11, m.len()).unwrap(), m);
    }
}

#[test]
fn bcd_tables_and_log_strings_match_console_image() {
    let Ok(dll) = std::fs::read(FN_WLDP) else { return };
    assert_eq!(compat_common::title_os::file_version(Path::new(FN_WLDP)), Some(LOG_IMAGE_VERSION));
    let img = Image { path: FN_WLDP.into(), bytes: dll };
    let words = |rva: u32, n: usize| -> Vec<u32> { img.slice(rva, n * 4).unwrap().chunks(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect() };
    assert_eq!(words(0x1f450, 4), OS_LOADER_ELEMENTS);
    assert_eq!(words(0x1f160, 2), BOOT_MANAGER_ELEMENTS);
    for rva in [FMT_BCD_OS_LOADER, FMT_BCD_BOOT_MANAGER] {
        let f = img.cstr(rva).unwrap();
        assert!(f.windows(5).any(|w| w == b"%08lx"));
        assert_eq!(*f.last().unwrap(), b'\n');
    }
    for rva in [FMT_MANUFACTURING, FMT_HSTI] {
        let f = img.cstr(rva).unwrap();
        assert!(!f.contains(&b'%') && *f.last().unwrap() == b'\n');
    }
    assert_eq!(img.cstr(LOG_HEADER_RVA).unwrap().len(), LOG_HEADER_LEN);
    assert_eq!(log_image().map(|i| i.slice(LOG_HEADER_RVA, LOG_HEADER_LEN).unwrap().to_vec()), img.slice(LOG_HEADER_RVA, LOG_HEADER_LEN).map(|h| h.to_vec()));
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

#[test]
fn era_kernel_answers_ground_the_model() {
    let Ok(vbi) = std::fs::read(FN_ERA_VBI) else { return };
    let stub = [0xb8u8, 0x06, 0x00, 0x43, 0x80, 0xc3];
    assert!(find(&vbi, &stub).is_some());
}

#[test]
fn oracle_console_dll_matches_port() {
    let _g = env_lock();
    let Some(f) = console_check() else {
        assert!(std::env::var_os("TOMBOX_REQUIRE_ORACLE").is_none());
        return;
    };
    let scenarios = [
        ("o_retail", Scenario::retail()),
        ("o_retail_stale", Scenario { hsti: Some(b"old".to_vec()), sb: Some(vec![1u8; HASH_SIZE]), stale_log: true, ..Scenario::retail() }),
        ("o_miss", Scenario { filter: 0xC000_0034u32 as i32, ..Scenario::retail() }),
        ("o_mfg", Scenario { filter: 0xC000_0034u32 as i32, manufacturing: Ok(true), hsti: zero_hash(), ..Scenario::retail() }),
        ("o_mfgerr", Scenario { manufacturing: Err(50), stale_log: true, ..Scenario::retail() }),
        ("o_zero", Scenario { filter: 0xC000_0034u32 as i32, hsti: zero_hash(), sb: zero_hash(), stale_log: true, ..Scenario::retail() }),
        ("o_short", Scenario { filter: 0xC000_0034u32 as i32, hsti: Some(vec![0u8; 8]), sb: Some(vec![0u8; 4]), ..Scenario::retail() }),
        ("o_hsti_ok_sb_bad", Scenario { filter: 0xC000_0034u32 as i32, hsti: zero_hash(), sb: Some(vec![2u8; HASH_SIZE]), ..Scenario::retail() }),
        ("o_log", Scenario { log_dir: true, ..Scenario::retail() }),
        ("o_log_mfg", Scenario { manufacturing: Ok(true), log_dir: true, ..Scenario::retail() }),
    ];
    for (tag, sc) in &scenarios {
        FILTER_CALLS.lock().unwrap().clear();
        let c = sc.console(f, tag);
        let calls = FILTER_CALLS.lock().unwrap().clone();
        let want: Vec<(u32, u32, u32)> = OS_LOADER_ELEMENTS.iter().map(|&e| (1, BCD_OS_LOADER, e)).chain(BOOT_MANAGER_ELEMENTS.iter().map(|&e| (1, BCD_BOOT_MANAGER, e))).collect();
        assert_eq!(calls, want, "{tag}");
        let p = sc.port(tag);
        assert_eq!(p, c, "{tag}");
    }
    let classes = NTQSI_CLASSES.lock().unwrap().clone();
    assert!(classes.contains(&0xa6) && classes.contains(&0x91));
    assert_eq!(classes.iter().filter(|&&c| c == 0x91).count(), 1);
}
