use super::production::*;
use super::store::Store;
use super::tests_console::{env_lock, temp_root};
use super::tests_watermark::{hook_delete, hook_expand, load, patch, sandbox};
use super::tlg;
use core::ffi::c_void;
use std::path::Path;
use std::sync::atomic::{AtomicI32, AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};

const TM_WLDP: &str = r"D:\xbox-pc-port\games\Trackmania\extracted\Ubisoft.TrackmaniaNext_1.6.2.0_neutral__ngz4m417e0mpw\EmbeddedXvd\Windows\System32\wldp.dll";
const ERA_VBIS: [&str; 3] = [
    r"D:\xbox-pc-port\games\Fortnite\extracted\era.vbi",
    r"D:\xbox-pc-port\games\Trackmania\extracted\Ubisoft.TrackmaniaNext_1.6.2.0_neutral__ngz4m417e0mpw\era.vbi",
    r"D:\xbox-pc-port\games\XDefiant\extracted\era.vbi",
];
const TM_IAT_GETPROCADDRESS: usize = 0x2ed28;
const TM_IAT_EXPAND: usize = 0x2ed80;
const TM_IAT_DELETEFILEW: usize = 0x2ec70;

#[link(name = "kernel32")]
extern "system" {
    fn GetProcAddress(module: *mut c_void, name: *const u8) -> *mut c_void;
}

type Ntqsi = unsafe extern "system" fn(u32, *mut c_void, u32, *mut u32) -> i32;

static LICENSE_STATUS: AtomicI32 = AtomicI32::new(STATUS_OBJECT_NAME_NOT_FOUND);
static LICENSE_CALLS: AtomicU32 = AtomicU32::new(0);
static REAL_NTQSI: OnceLock<usize> = OnceLock::new();
static RESOLVED: Mutex<Vec<String>> = Mutex::new(Vec::new());

unsafe extern "system" fn hook_ntqsi(class: u32, buf: *mut c_void, len: u32, ret: *mut u32) -> i32 {
    if class == 0x86 {
        LICENSE_CALLS.fetch_add(1, Ordering::SeqCst);
        return LICENSE_STATUS.load(Ordering::SeqCst);
    }
    let real: Ntqsi = core::mem::transmute(*REAL_NTQSI.get().unwrap());
    real(class, buf, len, ret)
}

unsafe extern "system" fn hook_gpa(module: *mut c_void, name: *const u8) -> *mut c_void {
    let r = GetProcAddress(module, name);
    if name as usize > 0xffff {
        let n = std::ffi::CStr::from_ptr(name as *const i8).to_string_lossy().into_owned();
        if n == "NtQuerySystemInformation" && !r.is_null() {
            let _ = REAL_NTQSI.set(r as usize);
            RESOLVED.lock().unwrap().push(n);
            return hook_ntqsi as *mut c_void;
        }
    }
    r
}

type LicenseFn = unsafe extern "system" fn(*mut u32) -> i32;

fn console() -> Option<(LicenseFn, LicenseFn)> {
    static F: OnceLock<Option<usize>> = OnceLock::new();
    let base = (*F.get_or_init(|| {
        if !Path::new(TM_WLDP).is_file() {
            return None;
        }
        let base = load(TM_WLDP);
        if base == 0 {
            return None;
        }
        unsafe {
            console_sandbox::attach(base);
            patch(base, TM_IAT_GETPROCADDRESS, hook_gpa as usize);
            patch(base, TM_IAT_EXPAND, hook_expand as usize);
            patch(base, TM_IAT_DELETEFILEW, hook_delete as usize);
            assert_eq!(GetProcAddress(base as *mut c_void, b"WldpIsProductionConfigurationInProc\0".as_ptr()) as usize, base + 0x14f10);
        }
        Some(base)
    }))?;
    unsafe { Some((core::mem::transmute::<usize, LicenseFn>(base + 0x7c08), core::mem::transmute::<usize, LicenseFn>(base + 0x14f10))) }
}

fn ci_log(s: &Store) -> std::path::PathBuf {
    s.console_path(NON_PRODUCTION_ERRORS_CI)
}

#[test]
fn era_license_lookup_is_value_not_found() {
    let _g = env_lock();
    let s = Store::at(temp_root("prod_era"));
    std::fs::write(ci_log(&s), b"x").unwrap();
    assert_eq!(is_production_configuration_in_proc_in(&s, ERA_CLOUD_EDITION_STATUS), (0, 1));
    assert!(!ci_log(&s).exists());
}

#[test]
fn license_status_mapping_follows_console_epilogue() {
    assert_eq!(license_query_result(STATUS_OBJECT_NAME_NOT_FOUND), (0, 0));
    assert_eq!(license_query_result(0xC000_00E5u32 as i32), (0xC004_F027u32 as i32, 0));
    assert_eq!(license_query_result(0xC000_003Eu32 as i32), (0xC004_F028u32 as i32, 0));
    assert_eq!(license_query_result(0x8007_007Au32 as i32), (0x9007_007Au32 as i32, 0));
    assert_eq!(license_query_result(0xC000_0001u32 as i32), (0xD000_0001u32 as i32, 0));
}

#[test]
fn failed_query_reports_not_production_and_keeps_log() {
    let _g = env_lock();
    let s = Store::at(temp_root("prod_fail"));
    std::fs::write(ci_log(&s), b"x").unwrap();
    assert_eq!(is_production_configuration_in_proc_in(&s, 0xC000_0001u32 as i32), (0xD000_0001u32 as i32, 0));
    assert!(ci_log(&s).exists());
}

#[test]
fn state_separation_selects_etw_log() {
    let _g = env_lock();
    let s = Store::at(temp_root("prod_sep"));
    s.set("state_separation", Some("1")).unwrap();
    let etw = s.console_path(NON_PRODUCTION_ERRORS_STATE_SEPARATED);
    std::fs::create_dir_all(etw.parent().unwrap()).unwrap();
    std::fs::write(&etw, b"x").unwrap();
    std::fs::write(ci_log(&s), b"x").unwrap();
    assert_eq!(is_production_configuration_in_proc_in(&s, ERA_CLOUD_EDITION_STATUS), (0, 1));
    assert!(!etw.exists());
    assert!(ci_log(&s).exists());
}

#[test]
fn export_writes_out_param() {
    let _g = env_lock();
    let d = temp_root("prod_export");
    std::env::set_var("TOMBOX_GDK_ROOT", &d);
    let mut out = 7u32;
    let hr = unsafe { WldpIsProductionConfigurationInProc(&mut out) };
    std::env::remove_var("TOMBOX_GDK_ROOT");
    assert_eq!((hr, out), (0, 1));
}

#[test]
fn production_event_blob_matches_console_image() {
    assert_eq!(&tlg::PRODUCTION_EVENT_DESC[..3], &[0x0b, 5, 0]);
    assert_eq!(tlg::desc_keyword(tlg::PRODUCTION_EVENT_DESC), 0x8000_0000_0000);
    let Ok(dll) = std::fs::read(TM_WLDP) else { return };
    let mut blob = tlg::PRODUCTION_EVENT_DESC.to_vec();
    blob.extend_from_slice(tlg::PRODUCTION_EVENT_META);
    assert!(dll.windows(blob.len()).any(|w| w == blob.as_slice()));
    for name in ["IsProductionConfiguration", "PartA_PrivTags", "WldpIsWcosProductionConfiguration", "RequiredConfigurationChecks", "FailedConfigurationChecks", "ErrorCode"] {
        assert!(tlg::PRODUCTION_EVENT_META.windows(name.len()).any(|w| w == name.as_bytes()), "{name}");
    }
}

#[test]
fn era_product_policy_has_no_cloud_edition_value() {
    let name: Vec<u8> = CLOUD_EDITION_VALUE.encode_utf16().flat_map(|c| c.to_le_bytes()).collect();
    let kernel_info: Vec<u8> = "Kernel-ProductInfo".encode_utf16().flat_map(|c| c.to_le_bytes()).collect();
    for p in ERA_VBIS {
        let Ok(vbi) = std::fs::read(p) else { continue };
        assert!(vbi.windows(kernel_info.len()).any(|w| w == kernel_info.as_slice()), "{p}");
        assert!(!vbi.windows(name.len()).any(|w| w == name.as_slice()), "{p}");
    }
}

#[test]
fn oracle_console_license_query_matches_port() {
    let _g = env_lock();
    let Some((query, export)) = console() else {
        assert!(std::env::var_os("TOMBOX_REQUIRE_ORACLE").is_none());
        return;
    };
    for st in [STATUS_OBJECT_NAME_NOT_FOUND, 0xC000_00E5u32 as i32, 0xC000_003Eu32 as i32, 0xC000_0001u32 as i32, 0xC000_0022u32 as i32, 0x8007_007Au32 as i32, 0x8000_0005u32 as i32] {
        LICENSE_STATUS.store(st, Ordering::SeqCst);
        let before = LICENSE_CALLS.load(Ordering::SeqCst);
        let mut flags = 0x1234u32;
        let hr = unsafe { query(&mut flags) };
        assert!(LICENSE_CALLS.load(Ordering::SeqCst) > before, "{st:#x}");
        assert_eq!((hr, flags), license_query_result(st), "{st:#x}");
    }
    assert!(!RESOLVED.lock().unwrap().is_empty());
    for (tag, st) in [("prod_o_era", STATUS_OBJECT_NAME_NOT_FOUND), ("prod_o_fail", 0xC000_0001u32 as i32)] {
        LICENSE_STATUS.store(st, Ordering::SeqCst);
        let c = Store::at(temp_root(&format!("{tag}_console")));
        std::fs::write(ci_log(&c), b"x").unwrap();
        *sandbox().lock().unwrap() = c.root().to_path_buf();
        let mut out = 0xdead_beefu32;
        let hr = unsafe { export(&mut out) };
        let p = Store::at(temp_root(&format!("{tag}_port")));
        std::fs::write(ci_log(&p), b"x").unwrap();
        assert_eq!(is_production_configuration_in_proc_in(&p, st), (hr, out), "{tag}");
        assert_eq!(ci_log(&p).exists(), ci_log(&c).exists(), "{tag}");
    }
}
