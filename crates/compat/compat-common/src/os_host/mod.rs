pub mod affinity;
pub mod coverage;
pub mod dlldir;
pub mod foreign;
pub mod fortnite_match;
pub mod k32legacy;
pub mod patch;
pub mod plm;
pub mod quiesce;
pub mod routes;
pub mod routes_rt;
pub mod webm_probe;
pub mod tm_race_probe;

use core::ffi::c_void;
use patch::Hook;

#[link(name = "kernel32")]
extern "system" {
    fn GetModuleHandleW(n: *const u16) -> *mut u8;
    fn LoadLibraryW(n: *const u16) -> *mut c_void;
    fn GetProcAddress(h: *mut c_void, name: *const u8) -> *mut c_void;
    fn GetCurrentProcessId() -> u32;
}

pub const OWNER_VAR: &str = "TOMBOX_OSHOST_OWNER";
pub const SIGNAL_VAR: &str = "TOMBOX_OSHOST_SIGNAL";
pub const REGISTER_VAR: &str = "TOMBOX_OSHOST_PLM_REGISTER";
pub const UNREGISTER_VAR: &str = "TOMBOX_OSHOST_PLM_UNREGISTER";

const AFF_DLLS: &[&str] = &["api-ms-win-core-processtopology-", "kernel32.dll", "kernelbase.dll"];
const THREAD_DLLS: &[&str] = &["api-ms-win-core-processthreads-", "kernel32.dll", "kernelbase.dll"];
const DIR_DLLS:&[&str] = &["api-ms-win-core-kernel32-legacy-", "api-ms-win-core-libraryloader-", "kernel32.dll", "kernelbase.dll"];
const PLM_DLLS: &[&str] = &["twinapi.appcore.dll", "kernel.appcore.dll"];

pub unsafe fn resolve(dll: &str, func: &str) -> *mut c_void {
    let w: Vec<u16> = dll.encode_utf16().chain(Some(0)).collect();
    let mut h = GetModuleHandleW(w.as_ptr()) as *mut c_void;
    if h.is_null() {
        h = LoadLibraryW(w.as_ptr());
    }
    let mut p = if h.is_null() { core::ptr::null_mut() } else { GetProcAddress(h, format!("{func}\0").as_ptr()) };
    if p.is_null() {
        let k: Vec<u16> = "kernel32.dll\0".encode_utf16().collect();
        p = GetProcAddress(GetModuleHandleW(k.as_ptr()) as *mut c_void, format!("{func}\0").as_ptr());
    }
    p
}

pub fn hooks() -> Vec<Hook> {
    vec![
        Hook { name: "SetThreadAffinityMask", dlls: AFF_DLLS, hook: affinity::set_thread_affinity_mask as *const () as usize },
        Hook { name: "GetProcessAffinityMask", dlls: AFF_DLLS, hook: affinity::get_process_affinity_mask as *const () as usize },
        Hook { name: "SetProcessAffinityMask", dlls: AFF_DLLS, hook: affinity::set_process_affinity_mask as *const () as usize },
        Hook { name: "CreateThread", dlls: THREAD_DLLS, hook: affinity::create_thread as *const () as usize },
        Hook { name: "GetCurrentProcessorNumber", dlls: THREAD_DLLS, hook: affinity::get_current_processor_number as *const () as usize },
        Hook { name: "SetDllDirectoryW", dlls: DIR_DLLS, hook: dlldir::set_dll_directory_w as *const () as usize },
        Hook { name: "RegisterAppStateChangeNotification", dlls: PLM_DLLS, hook: plm::register_app_state_change_notification as *const () as usize },
        Hook { name: "UnregisterAppStateChangeNotification", dlls: PLM_DLLS, hook: plm::unregister_app_state_change_notification as *const () as usize },
        Hook { name: "RegisterAppConstrainedChangeNotification", dlls: PLM_DLLS, hook: plm::register_app_constrained_change_notification as *const () as usize },
        Hook { name: "UnregisterAppConstrainedChangeNotification", dlls: PLM_DLLS, hook: plm::unregister_app_constrained_change_notification as *const () as usize },
    ]
}

fn pid_marker() -> String {
    format!("{}", unsafe { GetCurrentProcessId() })
}

pub fn is_owner() -> bool {
    static OWNER: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    OWNER.load(std::sync::atomic::Ordering::Acquire) || {
        let mine = std::env::var(SIGNAL_VAR).ok() == Some(format!("{:x}", plm::signal_entry as *const () as usize));
        if mine {
            OWNER.store(true, std::sync::atomic::Ordering::Release);
        }
        mine
    }
}

pub fn hook_table() -> &'static [Hook] {
    static H: std::sync::OnceLock<Vec<Hook>> = std::sync::OnceLock::new();
    H.get_or_init(|| {
        let mut v = hooks();
        let r = k32legacy::redirects(&v);
        v.extend(r);
        v
    })
}

pub fn sweep_now() -> usize {
    sweep_with(true)
}

pub fn routes_need_polling() -> bool {
    !load_watch_active()
}

pub fn sweep_with(routes_too: bool) -> usize {
    let mut report = Vec::new();
    let n = unsafe { patch::sweep(hook_table(), &mut report) + if routes_too { routes::sweep(hook_table(), &mut report) } else { 0 } };
    for (m, hits) in report {
        crate::record_raw("os_host", "patch", &format!("{m}: {}", hits.join(", ")));
    }
    n
}

pub unsafe extern "system" fn dll_loaded(reason: u32, data: *const u8, _ctx: *mut c_void) {
    if reason != 1 || data.is_null() {
        return;
    }
    let full = *(data.add(8) as *const *const u16);
    let base = *(data.add(24) as *const *mut u8);
    if full.is_null() || base.is_null() {
        return;
    }
    routes_rt::forget_range(base as usize, *(data.add(32) as *const u32) as usize);
    let len = *(full as *const u16) as usize / 2;
    let buf = *(full.add(4) as *const *const u16);
    if buf.is_null() {
        return;
    }
    let path = String::from_utf16_lossy(core::slice::from_raw_parts(buf, len));
    if !patch::is_title_module(&path, base as usize) || foreign::is_host_injected(&path) {
        return;
    }
    let mut hits = Vec::new();
    let routed = if routes::skip_module(&path, base as usize) { 0 } else { routes::patch_image(base, hook_table(), &mut hits) };
    if patch::patch_image(base, hook_table(), &mut hits) + routed > 0 {
        let name = path.rsplit('\\').next().unwrap_or(&path).to_string();
        crate::record_raw("os_host", "patch", &format!("{name} (loaded later): {}", hits.join(", ")));
    }
}

static WATCHING_LOADS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn load_watch_active() -> bool {
    WATCHING_LOADS.load(std::sync::atomic::Ordering::Acquire)
}

unsafe fn watch_loads() -> bool {
    foreign::prime();
    let p = resolve("ntdll.dll", "LdrRegisterDllNotification");
    if p.is_null() {
        return false;
    }
    let f: unsafe extern "system" fn(u32, unsafe extern "system" fn(u32, *const u8, *mut c_void), *mut c_void, *mut *mut c_void) -> i32 = core::mem::transmute(p);
    let mut cookie = core::ptr::null_mut();
    let ok = f(0, dll_loaded, core::ptr::null_mut(), &mut cookie) >= 0;
    WATCHING_LOADS.store(ok, std::sync::atomic::Ordering::Release);
    ok
}

pub fn installed_here() -> bool {
    is_owner()
}

pub fn ensure() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| unsafe { install() });
}

unsafe fn install() {
    if std::env::var("TOMBOX_OSHOST").ok().as_deref() == Some("0") {
        return;
    }
    let marker = pid_marker();
    if std::env::var(OWNER_VAR).ok().as_deref() == Some(marker.as_str()) {
        return;
    }
    std::env::set_var(OWNER_VAR, &marker);
    std::env::set_var(SIGNAL_VAR, format!("{:x}", plm::signal_entry as *const () as usize));
    std::env::set_var(REGISTER_VAR, format!("{:x}", plm::register_entry as *const () as usize));
    std::env::set_var(UNREGISTER_VAR, format!("{:x}", plm::unregister_entry as *const () as usize));
    let topo = affinity::topology();
    crate::record_raw("os_host", "affinity", &topo.describe());
    crate::record_raw("os_host", "dlldir", &dlldir::check());
    k32legacy::publish();
    let n = sweep_now();
    fortnite_match::apply();
    webm_probe::start();
    tm_race_probe::start();
    let watching = watch_loads();
    let n = n + if watching { unsafe { routes::sweep(hook_table(), &mut Vec::new()) } } else { 0 };
    crate::record_raw("os_host", "install", &format!("{n} import slots patched; late-load watch {}; {}", if watching { "on" } else { "UNAVAILABLE" }, coverage::summary()));
    let _ = std::thread::Builder::new().name("tombox-oshost-rescan".into()).spawn(|| {
        for i in 0..400u32 {
            std::thread::sleep(std::time::Duration::from_millis(if i < 120 { 500 } else { 5000 }));
            sweep_with(routes_need_polling());
        }
    });
}
