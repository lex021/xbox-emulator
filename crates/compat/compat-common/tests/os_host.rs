use compat_common::os_host::affinity::{self, build_map, Topology};
use compat_common::os_host::coverage::{self, Status};
use compat_common::os_host::dlldir;
use compat_common::os_host::patch::{self, Hook};
use compat_common::os_host::plm::{self, Kind};
use core::ffi::c_void;
use std::path::Path;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

static PLM_LOCK: Mutex<()> = Mutex::new(());

fn addr(f: unsafe extern "system" fn(u8, *mut c_void)) -> usize {
    f as usize
}

extern "system" {
    fn GetCurrentThread() -> *mut c_void;
    fn GetCurrentThreadId() -> u32;
}

fn smt_cores(n: usize) -> Vec<u64> {
    (0..n).map(|i| 0b11u64 << (2 * i)).collect()
}

#[test]
fn map_gives_each_console_core_one_thread_of_its_own_physical_core() {
    let m = build_map(&smt_cores(8), 7);
    assert_eq!(m, vec![1, 4, 16, 64, 256, 1024, 4096]);
    let hybrid = build_map(&[0b11, 0b1100, 0b11_0000, 1 << 8, 1 << 9], 4);
    assert_eq!(hybrid, vec![1, 4, 16, 1 << 8]);
    for i in 0..7 {
        for j in 0..7 {
            if i != j {
                assert_eq!(m[i] & m[j], 0);
            }
        }
    }
}

#[test]
fn map_splits_smt_siblings_when_host_has_fewer_cores() {
    let m = build_map(&smt_cores(4), 7);
    assert_eq!(m, vec![1 | 128, 4, 16, 64, 2, 8, 32]);
    let tiny = build_map(&[1, 2], 7);
    assert_eq!(tiny, vec![1, 2, 1, 2, 1, 2, 1]);
}

#[test]
fn console_and_host_masks_translate_like_console() {
    let t = Topology::new(&smt_cores(8), 0xffff, 7);
    assert_eq!(t.full, 0x7f);
    assert_eq!(t.to_host(0x7f, 0x7f), Some(0x1555));
    assert_eq!(t.to_host(0x3f, 0x7f), Some(0x555));
    assert_eq!(t.to_host(1 << 6, 0x7f), Some(1 << 12));
    assert_eq!(t.to_host(0, 0x7f), None);
    assert_eq!(t.to_host(0x7f, 0x0f), None);
    assert_eq!(t.to_host(u64::MAX, 0x7f), None);
    assert_eq!(t.to_host(0xff00, 0x7f), None);
    assert_eq!(t.to_host(1 << 40, 0x7f), None);
    assert_eq!(t.to_console(0xffff), 0x7f);
    assert_eq!(t.to_console(0b11 << 12), 1 << 6);
    assert_eq!(t.to_console(1 << 14), 0);
    assert_eq!(t.console_index(12), 6);
    assert_eq!(t.console_index(2), 1);
    assert_eq!(t.console_index(15), 15 % 7);
}

#[test]
fn topology_respects_restricted_host_mask() {
    let t = Topology::new(&smt_cores(8), 0x00ff, 7);
    assert_eq!(t.map, vec![1 | 128, 4, 16, 64, 2, 8, 32]);
    assert_eq!(t.to_host(0x7f, 0x7f), Some(0xff));
}

#[test]
fn hybrid_cores_put_performance_cores_first() {
    let order = affinity::order_cores(vec![(0, 1 << 8), (1, 0b11), (0, 1 << 9), (1, 0b1100)]);
    assert_eq!(order, vec![0b11, 0b1100, 1 << 8, 1 << 9]);
}

#[test]
fn live_set_thread_affinity_returns_console_shaped_previous_mask() {
    let t = affinity::topology().clone();
    let (host, _) = unsafe { affinity::real_get_process_affinity_mask() };
    assert_eq!(t.host, host);
    assert!(t.describe().contains("console"));
    std::thread::spawn(move || unsafe {
        let me = GetCurrentThread();
        let prev = affinity::set_thread_affinity_mask(me, 1);
        assert_eq!(prev as u64, affinity::process_console_mask());
        let want = t.to_host(1, t.full).unwrap();
        let host_prev = affinity::real_set_thread_affinity_mask(me, want as usize);
        assert_eq!(host_prev as u64, want);
        let prev2 = affinity::set_thread_affinity_mask(me, if t.cores > 1 { 3 } else { 1 });
        assert_eq!(prev2, 1);
        let zero = affinity::set_thread_affinity_mask(me, 0);
        assert_eq!(zero, 0);
        let full = affinity::set_thread_affinity_mask(me, usize::MAX);
        assert_eq!(full, 0);
        let host_now = affinity::real_set_thread_affinity_mask(me, t.host as usize);
        assert_eq!(host_now as u64, t.to_host(if t.cores > 1 { 3 } else { 1 }, t.full).unwrap());
    })
    .join()
    .unwrap();
}

#[test]
fn live_get_process_affinity_is_console_shaped() {
    let mut p = 0usize;
    let mut s = 0usize;
    let r = unsafe { affinity::get_process_affinity_mask(-1isize as *mut c_void, &mut p, &mut s) };
    assert_eq!(r, 1);
    let t = affinity::topology();
    assert_eq!(s as u64, t.full);
    assert_eq!(p as u64, affinity::process_console_mask());
    assert_eq!(p as u64 & !t.full, 0);
}

static CALLS: AtomicU32 = AtomicU32::new(0);
static LAST: AtomicUsize = AtomicUsize::new(0);
static CB_TID: AtomicU32 = AtomicU32::new(0);

unsafe extern "system" fn record_cb(value: u8, ctx: *mut c_void) {
    CALLS.fetch_add(1, Ordering::SeqCst);
    LAST.store(((value as usize) << 32) | ctx as usize, Ordering::SeqCst);
    CB_TID.store(GetCurrentThreadId(), Ordering::SeqCst);
}

static SLOW_DONE: AtomicU32 = AtomicU32::new(0);

unsafe extern "system" fn slow_cb(_value: u8, _ctx: *mut c_void) {
    std::thread::sleep(Duration::from_millis(300));
    SLOW_DONE.fetch_add(1, Ordering::SeqCst);
}

static SELF_ID: AtomicUsize = AtomicUsize::new(0);

unsafe extern "system" fn self_unregister_cb(_value: u8, _ctx: *mut c_void) {
    let r = plm::unregister(Kind::Constrained, SELF_ID.load(Ordering::SeqCst) as u64);
    LAST.store(r as usize, Ordering::SeqCst);
}

#[test]
fn plm_registration_ids_dispatch_and_unregister_match_console() {
    let _g = PLM_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    std::env::set_var("TOMBOX_PLM_WATCH", "0");
    let mut a = 0u64;
    let mut b = 0u64;
    let ra = unsafe { plm::register_app_state_change_notification(addr(record_cb), 0x1234, &mut a) };
    let rb = unsafe { plm::register_app_constrained_change_notification(addr(record_cb), 0x5678, &mut b) };
    assert_eq!((ra, rb), (0, 0));
    assert!(a >= 1 && b == a + 1);
    assert_eq!(unsafe { plm::register_app_state_change_notification(addr(record_cb), 0, core::ptr::null_mut()) }, plm::ERROR_INVALID_PARAMETER);
    CALLS.store(0, Ordering::SeqCst);
    assert_eq!(plm::suspend(), 1);
    assert_eq!(CALLS.load(Ordering::SeqCst), 1);
    assert_eq!(LAST.load(Ordering::SeqCst), (1usize << 32) | 0x1234);
    assert_ne!(CB_TID.load(Ordering::SeqCst), unsafe { GetCurrentThreadId() });
    assert_eq!(plm::resume(), 1);
    assert_eq!(LAST.load(Ordering::SeqCst), 0x1234);
    assert_eq!(plm::set_constrained(true), 1);
    assert_eq!(LAST.load(Ordering::SeqCst), (1usize << 32) | 0x5678);
    assert_eq!(unsafe { plm::unregister_app_state_change_notification(b) }, plm::ERROR_NOT_FOUND);
    assert_eq!(unsafe { plm::unregister_app_state_change_notification(a) }, 0);
    assert_eq!(unsafe { plm::unregister_app_state_change_notification(a) }, plm::ERROR_NOT_FOUND);
    assert_eq!(unsafe { plm::unregister_app_constrained_change_notification(b) }, 0);
    CALLS.store(0, Ordering::SeqCst);
    assert_eq!(plm::suspend(), 0);
    assert_eq!(plm::set_constrained(false), 0);
    assert_eq!(CALLS.load(Ordering::SeqCst), 0);
}

#[test]
fn plm_runs_several_callbacks_in_parallel_and_unregister_waits_for_in_flight() {
    let _g = PLM_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    std::env::set_var("TOMBOX_PLM_WATCH", "0");
    let mut ids = [0u64; 3];
    for id in ids.iter_mut() {
        assert_eq!(plm::register(Kind::AppState, addr(slow_cb), 0, id), 0);
    }
    SLOW_DONE.store(0, Ordering::SeqCst);
    let start = Instant::now();
    let h = std::thread::spawn(plm::suspend);
    std::thread::sleep(Duration::from_millis(80));
    assert_eq!(plm::unregister(Kind::AppState, ids[0]), 0);
    assert!(SLOW_DONE.load(Ordering::SeqCst) >= 1, "unregister returned while its callback was still running");
    assert_eq!(h.join().unwrap(), 3);
    assert_eq!(SLOW_DONE.load(Ordering::SeqCst), 3);
    assert!(start.elapsed() < Duration::from_millis(800), "callbacks were not run in parallel");
    for id in &ids[1..] {
        assert_eq!(plm::unregister(Kind::AppState, *id), 0);
    }
}

#[test]
fn plm_unregister_from_inside_its_own_callback_does_not_deadlock() {
    let _g = PLM_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    std::env::set_var("TOMBOX_PLM_WATCH", "0");
    let mut id = 0u64;
    assert_eq!(plm::register(Kind::Constrained, addr(self_unregister_cb), 0, &mut id), 0);
    SELF_ID.store(id as usize, Ordering::SeqCst);
    LAST.store(99, Ordering::SeqCst);
    assert_eq!(plm::set_constrained(true), 1);
    assert_eq!(LAST.load(Ordering::SeqCst), 0);
    assert_eq!(plm::registered(Kind::Constrained), 0);
}

#[test]
fn plm_commands_parse() {
    assert_eq!(plm::parse_command(" Suspend "), Some((Kind::AppState, true)));
    assert_eq!(plm::parse_command("resume"), Some((Kind::AppState, false)));
    assert_eq!(plm::parse_command("constrain"), Some((Kind::Constrained, true)));
    assert_eq!(plm::parse_command("unconstrain"), Some((Kind::Constrained, false)));
    assert_eq!(plm::parse_command("bogus"), None);
    assert!(plm::command_path().to_string_lossy().ends_with(".cmd"));
}

#[test]
fn dll_directory_relative_paths_rebase_onto_package_root() {
    let root = Path::new("G:\\");
    assert_eq!(dlldir::resolve(".\\aegir", Path::new("C:\\Windows\\System32"), root).as_deref(), Some("G:\\aegir"));
    assert_eq!(dlldir::resolve(".\\aegir", Path::new("G:\\"), root), None);
    assert_eq!(dlldir::resolve(".\\aegir", Path::new("g:"), root), None);
    assert_eq!(dlldir::resolve("G:\\Data\\Native", Path::new("C:\\x"), root), None);
    assert_eq!(dlldir::resolve("\\\\server\\x", Path::new("C:\\x"), root), None);
    assert_eq!(dlldir::resolve("", Path::new("C:\\x"), root), None);
    assert_eq!(dlldir::resolve("Data/Plugins/../Native", Path::new("C:\\x"), Path::new("D:\\pkg")).as_deref(), Some("D:\\pkg\\Data\\Native"));
    assert!(dlldir::is_relative("aegir"));
    assert!(!dlldir::is_relative("C:\\a"));
}

#[test]
fn package_root_walks_up_to_microsoftgame_config() {
    let dir = std::env::temp_dir().join(format!("tombox_oshost_root_{}", std::process::id()));
    let bin = dir.join("FortniteGame").join("Binaries").join("XSX");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::write(dir.join("MicrosoftGame.Config"), "<Game/>").unwrap();
    assert_eq!(dlldir::package_root_from(&bin), dir);
    let other = std::env::temp_dir().join(format!("tombox_oshost_noroot_{}", std::process::id()));
    std::fs::create_dir_all(&other).unwrap();
    assert_eq!(dlldir::package_root_from(&other), other);
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&other);
}

fn synthetic_pe() -> Vec<u8> {
    let mut img = vec![0u8; 0x2000];
    img[0] = b'M';
    img[1] = b'Z';
    let put32 = |img: &mut Vec<u8>, at: usize, v: u32| img[at..at + 4].copy_from_slice(&v.to_le_bytes());
    let put64 = |img: &mut Vec<u8>, at: usize, v: u64| img[at..at + 8].copy_from_slice(&v.to_le_bytes());
    put32(&mut img, 0x3c, 0x80);
    put32(&mut img, 0x80, 0x4550);
    let opt = 0x80 + 24;
    img[opt..opt + 2].copy_from_slice(&0x20bu16.to_le_bytes());
    put32(&mut img, opt + 112 + 8, 0x200);
    put32(&mut img, opt + 112 + 13 * 8, 0x280);
    put32(&mut img, 0x200, 0x300);
    put32(&mut img, 0x20c, 0x400);
    put32(&mut img, 0x210, 0x500);
    put32(&mut img, 0x214, 0x320);
    put32(&mut img, 0x220, 0x440);
    put32(&mut img, 0x224, 0x540);
    put64(&mut img, 0x300, 0x600);
    put64(&mut img, 0x308, 0x620);
    put64(&mut img, 0x320, 0x600);
    let a = b"api-ms-win-core-processtopology-obsolete-l1-1-0.dll";
    img[0x400..0x400 + a.len()].copy_from_slice(a);
    let b = b"USER32.dll";
    img[0x440..0x440 + b.len()].copy_from_slice(b);
    let n1 = b"SetThreadAffinityMask";
    img[0x602..0x602 + n1.len()].copy_from_slice(n1);
    let n2 = b"GetProcessAffinityMask";
    img[0x622..0x622 + n2.len()].copy_from_slice(n2);
    put32(&mut img, 0x280, 1);
    put32(&mut img, 0x284, 0x700);
    put32(&mut img, 0x28c, 0x800);
    put32(&mut img, 0x290, 0x900);
    let c = b"API-MS-WIN-CORE-PSM-APPNOTIFY-L1-1-0.dll";
    img[0x700..0x700 + c.len()].copy_from_slice(c);
    put64(&mut img, 0x900, 0xa00);
    put64(&mut img, 0x800, 0x1234);
    let n3 = b"RegisterAppStateChangeNotification";
    img[0xa02..0xa02 + n3.len()].copy_from_slice(n3);
    img
}

#[test]
fn patcher_hits_api_set_imports_and_delay_imports_only_for_matching_dlls() {
    let mut img = synthetic_pe();
    const T: &[&str] = &["api-ms-win-core-processtopology-", "kernel32.dll"];
    const P: &[&str] = &["api-ms-win-core-psm-appnotify-"];
    let hooks = [
        Hook { name: "SetThreadAffinityMask", dlls: T, hook: 0x1111 },
        Hook { name: "GetProcessAffinityMask", dlls: T, hook: 0x2222 },
        Hook { name: "RegisterAppStateChangeNotification", dlls: P, hook: 0x3333 },
    ];
    let base = img.as_mut_ptr();
    let mut hits = Vec::new();
    let n = unsafe { patch::patch_image(base, &hooks, &mut hits) };
    assert_eq!(n, 3, "{hits:?}");
    let rd = |img: &Vec<u8>, at: usize| u64::from_le_bytes(img[at..at + 8].try_into().unwrap());
    assert_eq!(rd(&img, 0x500), 0x1111);
    assert_eq!(rd(&img, 0x508), 0x2222);
    assert_eq!(rd(&img, 0x540), 0);
    assert_eq!(rd(&img, 0x800), 0x3333);
    let mut again = Vec::new();
    assert_eq!(unsafe { patch::patch_image(img.as_mut_ptr(), &hooks, &mut again) }, 0);
    assert!(patch::dll_matches("API-MS-WIN-CORE-PROCESSTOPOLOGY-OBSOLETE-L1-1-0.DLL", T));
    assert!(!patch::dll_matches("user32.dll", T));
}

#[test]
fn hook_table_covers_the_hooked_rows() {
    let names: Vec<&str> = compat_common::os_host::hooks().iter().map(|h| h.name).collect();
    for (_, f, s) in coverage::ROWS {
        if let Status::Hooked(_) = s {
            assert!(names.contains(f), "{f} marked hooked but not in the hook table");
        }
    }
    assert_eq!(coverage::ROWS.len(), 37);
    assert!(coverage::summary().starts_with("37 rows: 6 hooked here, 1 hooked by another owner, 7 emulated by a compat crate, 23 name-match pc"));
}

#[test]
fn windows_modules_are_never_patched() {
    let w = patch::windows_dir().to_string();
    assert!(!patch::is_title_module(&format!("{w}System32\\kernelbase.dll"), 1));
    assert!(patch::is_title_module("G:\\GbxGame.exe", 1));
}

#[test]
fn late_load_notification_patches_a_title_module() {
    let mut img = synthetic_pe();
    let game = std::env::current_exe().unwrap().parent().unwrap().to_string_lossy().into_owned();
    let path: Vec<u16> = format!("{game}\\Data\\Plugins\\Fake.dll").encode_utf16().collect();
    let mut us = [0u8; 16];
    us[0..2].copy_from_slice(&((path.len() * 2) as u16).to_le_bytes());
    us[2..4].copy_from_slice(&((path.len() * 2) as u16).to_le_bytes());
    us[8..16].copy_from_slice(&(path.as_ptr() as u64).to_le_bytes());
    let mut data = [0u8; 40];
    data[8..16].copy_from_slice(&(us.as_ptr() as u64).to_le_bytes());
    data[16..24].copy_from_slice(&(us.as_ptr() as u64).to_le_bytes());
    data[24..32].copy_from_slice(&(img.as_mut_ptr() as u64).to_le_bytes());
    unsafe { compat_common::os_host::dll_loaded(1, data.as_ptr(), core::ptr::null_mut()) };
    let rd = |img: &Vec<u8>, at: usize| u64::from_le_bytes(img[at..at + 8].try_into().unwrap()) as usize;
    let hooks = compat_common::os_host::hooks();
    let find = |n: &str| hooks.iter().find(|h| h.name == n).unwrap().hook;
    assert_eq!(rd(&img, 0x500), find("SetThreadAffinityMask"));
    assert_eq!(rd(&img, 0x508), find("GetProcessAffinityMask"));
    assert_eq!(rd(&img, 0x540), 0);
    assert!(!compat_common::os_host::routes::covered("api-ms-win-core-psm-appnotify-l1-1-0.dll", "RegisterAppStateChangeNotification", &hooks));
    let routed = compat_common::os_host::routes::lookup("api-ms-win-core-psm-appnotify-l1-1-0.dll", "RegisterAppStateChangeNotification", compat_common::os_host::routes::current_build()).map(|r| unsafe { compat_common::os_host::routes::target(r) }).filter(|t| *t != 0);
    assert_eq!(rd(&img, 0x800), routed.unwrap_or(0x1234));
    let mut sys = synthetic_pe();
    let wpath: Vec<u16> = format!("{}System32\\fake.dll", patch::windows_dir()).encode_utf16().collect();
    us[0..2].copy_from_slice(&((wpath.len() * 2) as u16).to_le_bytes());
    us[8..16].copy_from_slice(&(wpath.as_ptr() as u64).to_le_bytes());
    data[24..32].copy_from_slice(&(sys.as_mut_ptr() as u64).to_le_bytes());
    unsafe { compat_common::os_host::dll_loaded(1, data.as_ptr(), core::ptr::null_mut()) };
    assert_eq!(rd(&sys, 0x500), 0);
    assert_eq!(rd(&sys, 0x800), 0x1234);
    let mut overlay = synthetic_pe();
    let opath: Vec<u16> = "C:\\Users\\someone\\AppData\\Local\\Discord\\app-1.0.9\\modules\\discord_hook\\DiscordHook64.dll".encode_utf16().collect();
    us[0..2].copy_from_slice(&((opath.len() * 2) as u16).to_le_bytes());
    us[2..4].copy_from_slice(&((opath.len() * 2) as u16).to_le_bytes());
    us[8..16].copy_from_slice(&(opath.as_ptr() as u64).to_le_bytes());
    data[24..32].copy_from_slice(&(overlay.as_mut_ptr() as u64).to_le_bytes());
    unsafe { compat_common::os_host::dll_loaded(1, data.as_ptr(), core::ptr::null_mut()) };
    assert_eq!(rd(&overlay, 0x500), 0, "a host-injected overlay keeps its native imports");
    assert_eq!(rd(&overlay, 0x800), 0x1234);
}

#[test]
fn ensure_installs_once_with_owner_marker_and_load_watch() {
    std::env::set_var("TOMBOX_PLM_WATCH", "0");
    compat_common::os_host::ensure();
    compat_common::os_host::ensure();
    assert_eq!(std::env::var(compat_common::os_host::OWNER_VAR).unwrap(), std::process::id().to_string());
    assert!(compat_common::os_host::installed_here());
    assert!(compat_common::os_host::load_watch_active());
}