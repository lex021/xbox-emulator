use compat_common::os_host::patch::Hook;
use compat_common::os_host::routes::{self, lookup, ROUTES};
use core::ffi::c_void;
use std::path::PathBuf;

#[link(name = "kernel32")]
extern "system" {
    fn GetProcAddress(h: *mut c_void, name: *const u8) -> *mut c_void;
    fn LoadLibraryW(n: *const u16) -> *mut c_void;
}

#[path = "support/built_dlls.rs"]
mod built_dlls;

fn built_dir(file: &str) -> PathBuf {
    let dir = built_dlls::ensure(&[("shcore", "shcore.dll"), ("ntdll", "ntdll.dll"), ("advapi32legacy", "advapi32legacy.dll"), ("kernel32legacy", "kernel32legacy.dll"), ("kernel32legacy_19041", "kernel32legacy_19041.dll")]);
    assert!(dir.join(file).is_file(), "{file} not in {}", dir.display());
    dir
}

static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn setup() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let a = built_dir("shcore.dll");
        let b = built_dir("ntdll.dll");
        std::env::set_var(routes::DIR_VAR, std::env::join_paths([a, b]).unwrap());
    });
}

#[test]
fn table_is_generated_from_every_exports_txt() {
    let shcore: Vec<_> = ROUTES.iter().filter(|r| r.console == "shcore.dll").map(|r| (r.ordinal, r.func, r.lib)).collect();
    assert_eq!(shcore, vec![(37, "RevokeScaleChangeNotifications", "shcore"), (225, "#225", "shcore"), (226, "#226", "shcore"), (227, "#227", "shcore")]);
    for dll in ["ntdll.dll", "kernelbase.dll", "ole32.dll", "advapi32.dll", "kernel32.dll", "kernel32legacy.dll", "xpal.dll", "sc_dll.dll", "gamingtelemetry.dll", "werxboxgc.dll"] {
        assert!(routes::console_dlls().contains(&dll), "{dll} has no routed rows");
    }
    assert!(ROUTES.iter().all(|r| ["implemented", "forwarded", "hooked"].contains(&r.status)));
    assert!(ROUTES.iter().any(|r| r.console == "kernel32legacy.dll" && r.build == 19041 && r.lib == "kernel32legacy_19041"));
}

#[test]
fn direct_imports_resolve_by_name_and_by_console_ordinal() {
    assert_eq!(lookup("SHCORE.dll", "#227", 19041).map(|r| r.lib), Some("shcore"));
    assert_eq!(lookup("shcore.dll", "#37", 19041).map(|r| r.func), Some("RevokeScaleChangeNotifications"));
    assert_eq!(lookup("shcore.dll", "RevokeScaleChangeNotifications", 22000).map(|r| r.ordinal), Some(37));
    assert_eq!(lookup("ntdll.dll", "RtlWnfCompareChangeStamp", 22000).map(|r| r.lib), Some("ntdll"));
    assert_eq!(lookup("KernelBase.dll", "GetPackageInstallTime", 19041).map(|r| r.lib), Some("kernelbase"));
    assert_eq!(lookup("ole32.dll", "CoRevokeDeviceCatalog", 22000).map(|r| r.status), Some("forwarded"));
}

#[test]
fn kernelbase_family_xref_rows_route_to_the_compat_kernelbase() {
    for f in ["AddPackageToFamilyXref", "RemovePackageFromFamilyXref"] {
        let r = lookup("kernelbase.dll", f, 19041).unwrap_or_else(|| panic!("{f} not routed"));
        assert_eq!((r.lib, r.status), ("kernelbase", "implemented"), "{f}");
    }
}

#[test]
fn rows_not_implemented_fall_back_to_the_pc() {
    assert!(lookup("ntdll.dll", "NtClose", 19041).is_none());
    assert_eq!(lookup("ntdll.dll", "RtlpUmsThreadYield", 19041).map(|r| (r.lib, r.status)), Some(("ntdll", "implemented")));
    assert!(lookup("shcore.dll", "GetScaleFactorForDevice", 19041).is_none());
    assert!(lookup("user32.dll", "MessageBoxW", 19041).is_none());
    assert!(lookup("api-ms-win-core-com-l1-1-0.dll", "CoCreateInstance", 19041).is_none());
}

#[test]
fn api_set_imports_follow_the_console_host_and_os_build() {
    let set = "api-ms-win-core-processtopology-obsolete-l1-1-0.dll";
    assert_eq!(routes::host_of(set, "SetThreadAffinityMask", 19041).as_deref(), Some("kernel32legacy.dll"));
    assert_eq!(lookup(set, "SetThreadAffinityMask", 19041).map(|r| r.lib), Some("kernel32legacy_19041"));
    assert_eq!(lookup(set, "SetThreadAffinityMask", 22000).map(|r| r.lib), Some("kernel32legacy"));
    assert_eq!(lookup(set, "SetThreadAffinityMask", 22621).map(|r| r.lib), Some("kernel32legacy"));
    assert_eq!(routes::host_of("api-ms-win-core-com-l1-1-0", "CoCreateInstance", 22000).as_deref(), Some("combase.dll"));
}

struct Image {
    buf: Vec<u8>,
    slots: Vec<(String, String, usize)>,
}

fn image(dlls: &[(&str, &[&str])]) -> Image {
    let mut buf = vec![0u8; 0x4000];
    buf[0] = b'M';
    buf[1] = b'Z';
    buf[0x3c..0x40].copy_from_slice(&0x80u32.to_le_bytes());
    buf[0x80..0x84].copy_from_slice(b"PE\0\0");
    buf[0x98..0x9a].copy_from_slice(&0x20bu16.to_le_bytes());
    let imp_dir = 0x80 + 24 + 112 + 8;
    buf[imp_dir..imp_dir + 4].copy_from_slice(&0x400u32.to_le_bytes());
    let mut desc = 0x400usize;
    let mut arr = 0x800usize;
    let mut names = 0x2000usize;
    let mut slots = Vec::new();
    let put = |buf: &mut Vec<u8>, s: &str, at: &mut usize, hint: bool| -> usize {
        let start = *at;
        if hint {
            *at += 2;
        }
        buf[*at..*at + s.len()].copy_from_slice(s.as_bytes());
        *at += s.len() + 1;
        *at = (*at + 1) & !1;
        start
    };
    for (dll, funcs) in dlls {
        let name_rva = put(&mut buf, dll, &mut names, false);
        let int_rva = arr;
        let iat_rva = arr + (funcs.len() + 1) * 8;
        for (i, f) in funcs.iter().enumerate() {
            let thunk: u64 = match f.strip_prefix('#') {
                Some(o) => (1u64 << 63) | o.parse::<u64>().unwrap(),
                None => put(&mut buf, f, &mut names, true) as u64,
            };
            buf[int_rva + i * 8..int_rva + i * 8 + 8].copy_from_slice(&thunk.to_le_bytes());
            buf[iat_rva + i * 8..iat_rva + i * 8 + 8].copy_from_slice(&0x1111_0000_0000u64.wrapping_add(i as u64).to_le_bytes());
            slots.push((dll.to_ascii_lowercase(), f.to_string(), iat_rva + i * 8));
        }
        arr = iat_rva + (funcs.len() + 1) * 8;
        buf[desc..desc + 4].copy_from_slice(&(int_rva as u32).to_le_bytes());
        buf[desc + 12..desc + 16].copy_from_slice(&(name_rva as u32).to_le_bytes());
        buf[desc + 16..desc + 20].copy_from_slice(&(iat_rva as u32).to_le_bytes());
        desc += 20;
    }
    Image { buf, slots }
}

impl Image {
    fn at(&self, dll: &str, f: &str) -> usize {
        self.slots.iter().find(|s| s.0 == dll && s.1 == f).unwrap().2
    }
    fn slot(&self, dll: &str, f: &str) -> usize {
        let at = self.at(dll, f);
        u64::from_le_bytes(self.buf[at..at + 8].try_into().unwrap()) as usize
    }
    fn set(&mut self, dll: &str, f: &str, v: usize) {
        let at = self.at(dll, f);
        self.buf[at..at + 8].copy_from_slice(&(v as u64).to_le_bytes());
    }
}

#[test]
fn title_iat_is_patched_to_the_built_compat_dlls_with_pc_fallback() {
    setup();
    let set = "api-ms-win-core-processtopology-obsolete-l1-1-0.dll";
    let mut img = image(&[
        ("SHCORE.dll", &["#227", "RevokeScaleChangeNotifications", "GetScaleFactorForDevice", "#225"]),
        ("ntdll.dll", &["RtlWnfCompareChangeStamp", "NtClose"]),
        (set, &["SetThreadAffinityMask"]),
    ]);
    let host_hook: &'static [&'static str] = &["api-ms-win-core-processtopology-"];
    let skip = [Hook { name: "SetThreadAffinityMask", dlls: host_hook, hook: 0x1234 }];
    let before_pc = img.slot("shcore.dll", "GetScaleFactorForDevice");
    let mut hits = Vec::new();
    let n = unsafe { routes::patch_image_for(img.buf.as_mut_ptr(), 19041, &skip, &mut hits) };
    assert_eq!(n, 4, "{hits:?}");
    let sh = routes::module("shcore");
    let nt = routes::module("ntdll");
    assert!(sh != 0 && nt != 0);
    let path = unsafe { compat_common::os_host::patch::module_path(sh as *mut c_void) }.to_ascii_lowercase();
    let file = path.rsplit('\\').next().unwrap_or(&path).to_string();
    let pid_copy = file.strip_prefix("tbx_").and_then(|r| r.strip_suffix("_shcore.dll")).is_some_and(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
    assert!(path.contains("\\tombox-compat-routes\\") && (file == "tbx_shcore.dll" || pid_copy), "{path}");
    let sys = unsafe { LoadLibraryW("shcore.dll\0".encode_utf16().collect::<Vec<u16>>().as_ptr()) } as usize;
    assert_ne!(sys, sh);
    unsafe {
        assert_eq!(img.slot("shcore.dll", "#227"), GetProcAddress(sh as *mut c_void, 227usize as *const u8) as usize);
        assert_eq!(img.slot("shcore.dll", "#225"), GetProcAddress(sh as *mut c_void, 225usize as *const u8) as usize);
        assert_eq!(img.slot("shcore.dll", "RevokeScaleChangeNotifications"), GetProcAddress(sh as *mut c_void, b"RevokeScaleChangeNotifications\0".as_ptr()) as usize);
        assert_eq!(img.slot("ntdll.dll", "RtlWnfCompareChangeStamp"), GetProcAddress(nt as *mut c_void, b"RtlWnfCompareChangeStamp\0".as_ptr()) as usize);
    }
    assert_eq!(img.slot("shcore.dll", "GetScaleFactorForDevice"), before_pc);
    assert_eq!(img.slot("ntdll.dll", "NtClose"), 0x1111_0000_0001);
    assert_eq!(img.slot(set, "SetThreadAffinityMask"), 0x1111_0000_0000);
    let f: extern "system" fn(u32, u32) -> i32 = unsafe { core::mem::transmute(img.slot("ntdll.dll", "RtlWnfCompareChangeStamp")) };
    assert_eq!(f(1, 2), -1);
    let again = unsafe { routes::patch_image_for(img.buf.as_mut_ptr(), 19041, &skip, &mut hits) };
    assert_eq!(again, 0);
}

#[test]
fn compat_dlls_themselves_are_never_rerouted() {
    assert!(routes::skip_module("C:\\games\\x\\kernel32legacy.dll", 1));
    assert!(routes::skip_module("C:\\Temp\\tombox-compat-routes\\tbx_ntdll.dll", 1));
    assert!(routes::skip_module("C:\\Temp\\tombox-compat-routes\\tbx_4711_ntdll.dll", 1));
    assert!(!routes::skip_module("C:\\games\\x\\tbx_4711_FortniteClient.dll", 1));
    assert!(!routes::skip_module("C:\\games\\x\\FortniteClient.exe", 1));
}

#[test]
fn kernel32legacy_variant_rule_matches_the_k32legacy_wiring() {
    let set = "api-ms-win-core-processtopology-obsolete-l1-1-0.dll";
    for b in [19041, 20000, 21999] {
        assert_eq!(lookup(set, "SetThreadAffinityMask", b).map(|r| r.lib), Some("kernel32legacy_19041"), "{b}");
        assert_eq!(routes::lib_for("kernel32legacy.dll", b), Some("kernel32legacy_19041"));
    }
    for b in [22000, 22621, 26100] {
        assert_eq!(routes::lib_for("kernel32legacy.dll", b), Some("kernel32legacy"), "{b}");
    }
    for b in [19041u32, 22000] {
        assert_eq!(compat_common::os_host::k32legacy::dll_name(b), format!("{}.dll", routes::lib_for("kernel32legacy.dll", b).unwrap()));
    }
}

#[test]
fn tombox_kernel32legacy_overrides_both_copies() {
    let _env = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let file = built_dir("ntdll.dll").join("ntdll.dll");
    std::env::set_var("TOMBOX_KERNEL32LEGACY", &file);
    let a = routes::find_file("kernel32legacy");
    let b = routes::find_file("kernel32legacy_19041");
    std::env::set_var("TOMBOX_KERNEL32LEGACY", "0");
    let off = routes::find_file("kernel32legacy_19041");
    std::env::remove_var("TOMBOX_KERNEL32LEGACY");
    assert_eq!(a.as_deref(), Some(file.as_path()));
    assert_eq!(b.as_deref(), Some(file.as_path()));
    assert!(off.is_none());
}

type Gpa = unsafe extern "system" fn(*mut c_void, *const u8) -> *mut c_void;
type LoadW = unsafe extern "system" fn(*const u16) -> *mut c_void;
type LoadA = unsafe extern "system" fn(*const u8) -> *mut c_void;
type LoadExW = unsafe extern "system" fn(*const u16, *mut c_void, u32) -> *mut c_void;
type LoadPkg = unsafe extern "system" fn(*const u16, u32) -> *mut c_void;

fn w(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

#[test]
fn run_time_loader_calls_resolve_to_routed_exports_with_pc_fallback() {
    setup();
    let dir = built_dir("advapi32legacy.dll");
    let cur = std::env::var_os(routes::DIR_VAR).unwrap();
    let mut all: Vec<PathBuf> = std::env::split_paths(&cur).collect();
    all.push(dir);
    std::env::set_var(routes::DIR_VAR, std::env::join_paths(all).unwrap());
    use compat_common::os_host::routes_rt::{self, pc_binding};
    let ll = "api-ms-win-core-libraryloader-l1-2-0.dll";
    let mut img = image(&[
        (ll, &["GetProcAddress", "LoadLibraryExW", "GetModuleHandleW"]),
        ("KERNEL32.dll", &["LoadLibraryW", "LoadLibraryA", "LoadPackagedLibrary", "GetModuleHandleA"]),
        ("kernelbase.dll", &["LoadLibraryExA"]),
    ]);
    for (d, f) in [(ll, "GetProcAddress"), (ll, "LoadLibraryExW"), (ll, "GetModuleHandleW"), ("kernel32.dll", "LoadLibraryW"), ("kernel32.dll", "LoadLibraryA"), ("kernel32.dll", "LoadPackagedLibrary"), ("kernel32.dll", "GetModuleHandleA")] {
        let v = *pc_binding(f).last().unwrap();
        img.set(d, f, v);
    }
    img.set("kernelbase.dll", "LoadLibraryExA", foreign_load_ex_a as *const () as usize);
    let k32: &'static [&'static str] = &["kernel32.dll"];
    let skip = [Hook { name: "GetModuleHandleA", dlls: k32, hook: 0x4321 }];
    let before_gmha = img.slot("kernel32.dll", "GetModuleHandleA");
    let mut hits = Vec::new();
    let n = unsafe { routes_rt::patch_image(img.buf.as_mut_ptr(), &skip, &mut hits) };
    assert_eq!(n, 7, "{hits:?}");
    let lxa_slot = img.slot("kernelbase.dll", "LoadLibraryExA");
    assert!(routes_rt::is_routing_hook(lxa_slot));
    assert_eq!(routes_rt::chained_to(lxa_slot), Some(foreign_load_ex_a as *const () as usize));
    assert_eq!(img.slot("kernel32.dll", "GetModuleHandleA"), before_gmha);
    assert_eq!(unsafe { routes_rt::patch_image(img.buf.as_mut_ptr(), &skip, &mut hits) }, 0);
    unsafe {
        let pcg: Gpa = core::mem::transmute(pc_binding("GetProcAddress")[0]);
        let gpa: Gpa = core::mem::transmute(img.slot(ll, "GetProcAddress"));
        let lw: LoadW = core::mem::transmute(img.slot("kernel32.dll", "LoadLibraryW"));
        let la: LoadA = core::mem::transmute(img.slot("kernel32.dll", "LoadLibraryA"));
        let lxw: LoadExW = core::mem::transmute(img.slot(ll, "LoadLibraryExW"));
        let lp: LoadPkg = core::mem::transmute(img.slot("kernel32.dll", "LoadPackagedLibrary"));
        let gmw: LoadW = core::mem::transmute(img.slot(ll, "GetModuleHandleW"));

        let sys_sh = lw(w("shcore.dll").as_ptr());
        assert!(!sys_sh.is_null());
        assert!(compat_common::os_host::patch::module_path(sys_sh).to_ascii_lowercase().starts_with(compat_common::os_host::patch::windows_dir()));
        let ours = routes::module("shcore") as *mut c_void;
        assert_eq!(gpa(sys_sh, 227usize as *const u8), pcg(ours, 227usize as *const u8));
        assert!(pcg(sys_sh, 227usize as *const u8).is_null());
        assert_eq!(gpa(sys_sh, b"RevokeScaleChangeNotifications\0".as_ptr()), pcg(ours, b"RevokeScaleChangeNotifications\0".as_ptr()));
        assert_eq!(gpa(sys_sh, b"GetScaleFactorForDevice\0".as_ptr()), pcg(sys_sh, b"GetScaleFactorForDevice\0".as_ptr()));

        let nt = lxw(w("ntdll.dll").as_ptr(), core::ptr::null_mut(), 0);
        let wnf = gpa(nt, b"RtlWnfCompareChangeStamp\0".as_ptr());
        assert!(pcg(nt, b"RtlWnfCompareChangeStamp\0".as_ptr()).is_null());
        assert_eq!(wnf, pcg(routes::module("ntdll") as *mut c_void, b"RtlWnfCompareChangeStamp\0".as_ptr()));
        assert!(pcg(nt, b"RtlpUmsThreadYield\0".as_ptr()).is_null());
        assert!(!gpa(nt, b"RtlpUmsThreadYield\0".as_ptr()).is_null());
        assert_eq!(gpa(nt, b"RtlpUmsThreadYield\0".as_ptr()), pcg(routes::module("ntdll") as *mut c_void, b"RtlpUmsThreadYield\0".as_ptr()));
        assert!(gpa(nt, b"TbxNoSuchNtdllExport\0".as_ptr()).is_null());
        assert_eq!(gpa(nt, b"NtClose\0".as_ptr()), pcg(nt, b"NtClose\0".as_ptr()));

        let a32l = la(b"advapi32legacy.dll\0".as_ptr());
        assert!(!a32l.is_null());
        let p = compat_common::os_host::patch::module_path(a32l).to_ascii_lowercase();
        assert!(p.ends_with("\\advapi32legacy.dll") && !p.starts_with(compat_common::os_host::patch::windows_dir()), "{p}");
        assert_eq!(a32l as usize, routes::loaded("advapi32legacy"));
        assert_eq!(lp(w("advapi32legacy").as_ptr(), 0), a32l);
        assert_eq!(gmw(w("ADVAPI32LEGACY.DLL").as_ptr()), a32l);
        assert!(!gpa(a32l, b"ElfFlushEventLog\0".as_ptr()).is_null());
        assert!(la(b"tombox_no_such_console.dll\0".as_ptr()).is_null());

        let set = "api-ms-win-core-processtopology-obsolete-l1-1-0.dll";
        let hs = lxw(w(set).as_ptr(), core::ptr::null_mut(), 0);
        assert!(!hs.is_null());
        assert!(routes_rt::recorded(hs).contains(&set.to_string()));

        let lxa: LoadExA = core::mem::transmute(lxa_slot);
        let calls = FOREIGN_CALLS.load(Ordering::SeqCst);
        assert_eq!(lxa(b"advapi32legacy.dll\0".as_ptr(), core::ptr::null_mut(), 0), a32l);
        assert_eq!(FOREIGN_CALLS.load(Ordering::SeqCst), calls);
        let ver = lxa(b"version.dll\0".as_ptr(), core::ptr::null_mut(), 0);
        assert!(!ver.is_null());
        assert_eq!(FOREIGN_CALLS.load(Ordering::SeqCst), calls + 1);
        assert_eq!(gpa(ver, b"GetFileVersionInfoW\0".as_ptr()), pcg(ver, b"GetFileVersionInfoW\0".as_ptr()));
    }
}

type LoadExA = unsafe extern "system" fn(*const u8, *mut c_void, u32) -> *mut c_void;
type HandleExW = unsafe extern "system" fn(u32, *const u16, *mut *mut c_void) -> i32;
type HandleExA = unsafe extern "system" fn(u32, *const u8, *mut *mut c_void) -> i32;
type GpaCaller = unsafe extern "system" fn(*mut c_void, *const u8, *mut c_void) -> *mut c_void;
type LdrGpa = unsafe extern "system" fn(*mut c_void, *const Counted<u8>, u32, *mut *mut c_void) -> i32;
type LdrGpaEx = unsafe extern "system" fn(*mut c_void, *const Counted<u8>, u32, *mut *mut c_void, u32) -> i32;
type LdrDll = unsafe extern "system" fn(*const u16, *const u32, *const Counted<u16>, *mut *mut c_void) -> i32;
use compat_common::os_host::routes_rt::Counted;
use std::sync::atomic::{AtomicUsize, Ordering};

static FOREIGN_CALLS: AtomicUsize = AtomicUsize::new(0);
static LOOP_NEXT: AtomicUsize = AtomicUsize::new(0);
static LOOP_CALLS: AtomicUsize = AtomicUsize::new(0);

unsafe extern "system" fn foreign_load_ex_a(name: *const u8, file: *mut c_void, flags: u32) -> *mut c_void {
    FOREIGN_CALLS.fetch_add(1, Ordering::SeqCst);
    let pc: LoadExA = core::mem::transmute(compat_common::os_host::routes_rt::pc_binding("LoadLibraryExA")[0]);
    pc(name, file, flags)
}

unsafe extern "system" fn looping_get_module_handle_w(name: *const u16) -> *mut c_void {
    LOOP_CALLS.fetch_add(1, Ordering::SeqCst);
    let back: LoadW = core::mem::transmute(LOOP_NEXT.load(Ordering::SeqCst));
    back(name)
}

fn a_str(s: &[u8]) -> Counted<u8> {
    Counted { len: s.len() as u16, max: s.len() as u16, buf: s.as_ptr() }
}

fn u_str(s: &[u16]) -> Counted<u16> {
    Counted { len: (s.len() * 2) as u16, max: (s.len() * 2) as u16, buf: s.as_ptr() }
}

#[test]
fn ex_caller_and_ldr_loader_imports_are_routed_too() {
    setup();
    let dir = built_dir("advapi32legacy.dll");
    let cur = std::env::var_os(routes::DIR_VAR).unwrap();
    let mut all: Vec<PathBuf> = std::env::split_paths(&cur).collect();
    all.push(dir);
    std::env::set_var(routes::DIR_VAR, std::env::join_paths(all).unwrap());
    use compat_common::os_host::routes_rt::{self, pc_binding};
    let funcs: &[(&str, &[&str])] = &[
        ("ntdll.dll", &["LdrGetProcedureAddress", "LdrGetProcedureAddressEx", "LdrLoadDll", "LdrGetDllHandle"]),
        ("KERNELBASE.dll", &["GetProcAddressForCaller", "GetModuleHandleExW"]),
        ("api-ms-win-core-libraryloader-l1-2-0.dll", &["GetModuleHandleExA"]),
    ];
    let mut img = image(funcs);
    for (d, fs) in funcs {
        for f in fs.iter() {
            img.set(&d.to_ascii_lowercase(), f, *pc_binding(f).first().unwrap_or_else(|| panic!("{f} has no PC binding")));
        }
    }
    let mut hits = Vec::new();
    assert_eq!(unsafe { routes_rt::patch_image(img.buf.as_mut_ptr(), &[], &mut hits) }, 7, "{hits:?}");
    assert_eq!(unsafe { routes_rt::patch_image(img.buf.as_mut_ptr(), &[], &mut hits) }, 0);
    unsafe {
        let pcg: Gpa = core::mem::transmute(pc_binding("GetProcAddress")[0]);
        let ldr_gpa: LdrGpa = core::mem::transmute(img.slot("ntdll.dll", "LdrGetProcedureAddress"));
        let ldr_gpax: LdrGpaEx = core::mem::transmute(img.slot("ntdll.dll", "LdrGetProcedureAddressEx"));
        let ldr_load: LdrDll = core::mem::transmute(img.slot("ntdll.dll", "LdrLoadDll"));
        let ldr_handle: LdrDll = core::mem::transmute(img.slot("ntdll.dll", "LdrGetDllHandle"));
        let gpac: GpaCaller = core::mem::transmute(img.slot("kernelbase.dll", "GetProcAddressForCaller"));
        let gmxw: HandleExW = core::mem::transmute(img.slot("kernelbase.dll", "GetModuleHandleExW"));
        let gmxa: HandleExA = core::mem::transmute(img.slot("api-ms-win-core-libraryloader-l1-2-0.dll", "GetModuleHandleExA"));

        let sys_sh = LoadLibraryW(w("shcore.dll").as_ptr());
        let ours = routes::module("shcore") as *mut c_void;
        let mut out: *mut c_void = core::ptr::null_mut();
        assert_eq!(ldr_gpa(sys_sh, core::ptr::null(), 227, &mut out), 0);
        assert_eq!(out, pcg(ours, 227usize as *const u8));
        let revoke = a_str(b"RevokeScaleChangeNotifications");
        out = core::ptr::null_mut();
        assert_eq!(ldr_gpax(sys_sh, &revoke, 0, &mut out, 0), 0);
        assert_eq!(out, pcg(ours, b"RevokeScaleChangeNotifications\0".as_ptr()));
        let pcname = a_str(b"GetScaleFactorForDevice");
        out = core::ptr::null_mut();
        assert_eq!(ldr_gpa(sys_sh, &pcname, 0, &mut out), 0);
        assert_eq!(out, pcg(sys_sh, b"GetScaleFactorForDevice\0".as_ptr()));
        let missing = a_str(b"TbxNoSuchNtdllExport");
        let nt = LoadLibraryW(w("ntdll.dll").as_ptr());
        assert_eq!(ldr_gpa(nt, &missing, 0, &mut out) as u32, 0xC000_0139);
        assert_eq!(gpac(sys_sh, 227usize as *const u8, core::ptr::null_mut()), pcg(ours, 227usize as *const u8));
        assert_eq!(gpac(sys_sh, b"GetScaleFactorForDevice\0".as_ptr(), core::ptr::null_mut()), pcg(sys_sh, b"GetScaleFactorForDevice\0".as_ptr()));

        let name = w("advapi32legacy.dll");
        let us = u_str(&name[..name.len() - 1]);
        let mut h: *mut c_void = core::ptr::null_mut();
        assert_eq!(ldr_load(core::ptr::null(), core::ptr::null(), &us, &mut h), 0);
        assert_eq!(h as usize, routes::loaded("advapi32legacy"));
        assert!(!compat_common::os_host::patch::module_path(h).to_ascii_lowercase().starts_with(compat_common::os_host::patch::windows_dir()));
        let mut h2: *mut c_void = core::ptr::null_mut();
        assert_eq!(ldr_handle(core::ptr::null(), core::ptr::null(), &us, &mut h2), 0);
        assert_eq!(h2, h);
        let nope = w("tombox_no_such_console.dll");
        let un = u_str(&nope[..nope.len() - 1]);
        assert_eq!(ldr_load(core::ptr::null(), core::ptr::null(), &un, &mut h2) as u32, 0xC000_0135);
        assert_eq!(ldr_handle(core::ptr::null(), core::ptr::null(), &un, &mut h2) as u32, 0xC000_0135);
        out = core::ptr::null_mut();
        assert_eq!(ldr_gpa(h, &a_str(b"ElfFlushEventLog"), 0, &mut out), 0);
        assert!(!out.is_null());

        let mut m: *mut c_void = core::ptr::null_mut();
        assert_eq!(gmxw(0x2, w("advapi32legacy").as_ptr(), &mut m), 1);
        assert_eq!(m, h);
        m = core::ptr::null_mut();
        assert_eq!(gmxa(0x2, b"ADVAPI32LEGACY.DLL\0".as_ptr(), &mut m), 1);
        assert_eq!(m, h);
        assert_eq!(gmxw(0x2, nope.as_ptr(), &mut m), 0);
        assert!(m.is_null());
    }
}

#[test]
fn a_foreign_hook_that_calls_back_into_routing_does_not_loop() {
    use compat_common::os_host::routes_rt::{self, pc_binding};
    let mut img = image(&[("kernel32.dll", &["GetModuleHandleW"])]);
    img.set("kernel32.dll", "GetModuleHandleW", looping_get_module_handle_w as *const () as usize);
    let mut hits = Vec::new();
    assert_eq!(unsafe { routes_rt::patch_image(img.buf.as_mut_ptr(), &[], &mut hits) }, 1, "{hits:?}");
    let tramp = img.slot("kernel32.dll", "GetModuleHandleW");
    assert!(routes_rt::is_routing_hook(tramp));
    LOOP_NEXT.store(tramp, Ordering::SeqCst);
    let before = LOOP_CALLS.load(Ordering::SeqCst);
    let pc: LoadW = unsafe { core::mem::transmute(pc_binding("GetModuleHandleW")[0]) };
    let f: LoadW = unsafe { core::mem::transmute(tramp) };
    let k = w("kernel32.dll");
    assert_eq!(unsafe { f(k.as_ptr()) }, unsafe { pc(k.as_ptr()) });
    assert_eq!(LOOP_CALLS.load(Ordering::SeqCst), before + 1);
    img.set("kernel32.dll", "GetModuleHandleW", looping_get_module_handle_w as *const () as usize);
    assert_eq!(unsafe { routes_rt::patch_image(img.buf.as_mut_ptr(), &[], &mut hits) }, 0);
}
static MANY_CALLS: [AtomicUsize; 24] = [const { AtomicUsize::new(0) }; 24];

unsafe extern "system" fn many_foreign<const N: usize>(name: *const u16) -> *mut c_void {
    MANY_CALLS[N].fetch_add(1, Ordering::SeqCst);
    let pc: LoadW = core::mem::transmute(compat_common::os_host::routes_rt::pc_binding("GetModuleHandleW")[0]);
    pc(name)
}

macro_rules! many {
    ($($n:literal),*) => { [$(many_foreign::<$n> as *const () as usize),*] };
}

#[test]
fn any_number_of_foreign_hooks_get_their_own_routing_stub() {
    setup();
    let dir = built_dir("advapi32legacy.dll");
    let cur = std::env::var_os(routes::DIR_VAR).unwrap();
    let mut all: Vec<PathBuf> = std::env::split_paths(&cur).collect();
    all.push(dir);
    std::env::set_var(routes::DIR_VAR, std::env::join_paths(all).unwrap());
    use compat_common::os_host::routes_rt::{self, pc_binding};
    let real = many!(0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23);
    let pc: LoadW = unsafe { core::mem::transmute(pc_binding("GetModuleHandleW")[0]) };
    let a32l = routes::module("advapi32legacy") as *mut c_void;
    assert!(!a32l.is_null());
    let k = w("kernel32.dll");
    let nope = w("tombox_no_such_console.dll");
    for (i, foreign) in real.iter().enumerate() {
        let mut img = image(&[("kernel32.dll", &["GetModuleHandleW"])]);
        img.set("kernel32.dll", "GetModuleHandleW", *foreign);
        routes_rt::forget_range(img.buf.as_ptr() as usize, img.buf.len());
        let mut hits = Vec::new();
        assert_eq!(unsafe { routes_rt::patch_image(img.buf.as_mut_ptr(), &[], &mut hits) }, 1, "{i}: {hits:?}");
        let stub = img.slot("kernel32.dll", "GetModuleHandleW");
        assert!(routes_rt::is_routing_hook(stub));
        assert_eq!(routes_rt::chained_to(stub), Some(*foreign));
        let f: LoadW = unsafe { core::mem::transmute(stub) };
        let before = MANY_CALLS[i].load(Ordering::SeqCst);
        assert_eq!(unsafe { f(k.as_ptr()) }, unsafe { pc(k.as_ptr()) });
        assert!(unsafe { f(nope.as_ptr()) }.is_null());
        assert_eq!(MANY_CALLS[i].load(Ordering::SeqCst), before + 2);
        assert_eq!(unsafe { f(w("advapi32legacy.dll").as_ptr()) }, a32l);
    }
    let mut stubs = std::collections::HashSet::new();
    for i in 0..5000usize {
        let fake = 0x7777_0000_0000 + i * 16;
        let s = unsafe { routes_rt::chain_stub("LoadLibraryW", fake) }.expect("stub memory");
        assert_eq!(unsafe { routes_rt::chain_stub("LoadLibraryW", fake) }, Some(s));
        assert_eq!(routes_rt::chained_to(s), Some(fake));
        assert!(stubs.insert(s));
    }
    assert!(routes_rt::chain_count() >= 5000 + real.len());
}
#[link(name = "kernel32")]
extern "system" {
    fn RtlLookupFunctionEntry(pc: u64, base: *mut u64, history: *mut c_void) -> *const u32;
}

#[test]
fn chain_shim_has_unwind_data_and_routing_sweeps_are_event_driven() {
    let shim = compat_common::os_host::routes_rt::shim_address() as u64;
    let mut base = 0u64;
    let e = unsafe { RtlLookupFunctionEntry(shim + 4, &mut base, core::ptr::null_mut()) };
    assert!(!e.is_null());
    assert_eq!(base + unsafe { *e } as u64, shim);
    compat_common::os_host::ensure();
    assert!(compat_common::os_host::load_watch_active());
    assert!(!compat_common::os_host::routes_need_polling());
}
fn add_dir(file: &str) {
    setup();
    let dir = built_dir(file);
    let cur = std::env::var_os(routes::DIR_VAR).unwrap();
    let mut all: Vec<PathBuf> = std::env::split_paths(&cur).collect();
    if !all.contains(&dir) {
        all.push(dir);
        std::env::set_var(routes::DIR_VAR, std::env::join_paths(all).unwrap());
    }
}

#[test]
fn gdk_get_proc_address_hook_resolves_through_routing() {
    add_dir("shcore.dll");
    let pcg: Gpa = unsafe { core::mem::transmute(compat_common::os_host::routes_rt::pc_binding("GetProcAddress")[0]) };
    unsafe {
        let sys_sh = LoadLibraryW(w("shcore.dll").as_ptr());
        let ours = routes::module("shcore") as *mut c_void;
        assert_eq!(compat_common::gdk::hook::get_proc_address_hook(sys_sh, 227usize as *const u8), pcg(ours, 227usize as *const u8));
        assert_eq!(compat_common::gdk::hook::get_proc_address_hook(sys_sh, b"RevokeScaleChangeNotifications\0".as_ptr()), pcg(ours, b"RevokeScaleChangeNotifications\0".as_ptr()));
        assert!(!pcg(ours, 227usize as *const u8).is_null());
    }
}

#[test]
fn gdk_game_drive_loadlibrary_hooks_resolve_through_routing() {
    add_dir("advapi32legacy.dll");
    let a32l = routes::module("advapi32legacy") as *mut c_void;
    assert!(!a32l.is_null());
    let hooks = compat_common::gdk::fs::loader_hooks();
    assert_eq!(hooks.len(), 4);
    unsafe {
        let lw: LoadW = core::mem::transmute(hooks[0]);
        let lxw: LoadExW = core::mem::transmute(hooks[1]);
        let la: LoadA = core::mem::transmute(hooks[2]);
        let lxa: LoadExA = core::mem::transmute(hooks[3]);
        assert_eq!(lw(w("advapi32legacy.dll").as_ptr()), a32l);
        assert_eq!(lxw(w("ADVAPI32LEGACY").as_ptr(), core::ptr::null_mut(), 0), a32l);
        assert_eq!(la(b"advapi32legacy.dll\0".as_ptr()), a32l);
        assert_eq!(lxa(b"advapi32legacy.dll\0".as_ptr(), core::ptr::null_mut(), 0), a32l);
        assert!(compat_common::os_host::routes_rt::recorded(a32l).contains(&"advapi32legacy.dll".to_string()));
    }
}

#[test]
fn kernel32legacy_redirects_leave_loader_slots_to_routing_without_a_patch_fight() {
    let _env = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    add_dir("advapi32legacy.dll");
    add_dir("kernel32legacy.dll");
    use compat_common::os_host::routes_rt::{self, pc_binding};
    let k32l = routes::module("kernel32legacy");
    assert_ne!(k32l, 0);
    let redirects = unsafe { compat_common::os_host::k32legacy::redirects_from(k32l, &[]) };
    assert!(!redirects.is_empty());
    for f in routes_rt::LOADER_FUNCS {
        assert!(!redirects.iter().any(|h| h.name == *f), "{f} is still redirected by k32legacy");
    }
    let set = "api-ms-win-core-kernel32-legacy-l1-1-0.dll";
    let direct = "kernel32legacy.dll";
    assert!(lookup(direct, "LoadLibraryW", routes::current_build()).is_some());
    let mut img = image(&[(set, &["LoadLibraryW", "LoadLibraryA"]), (direct, &["LoadLibraryW", "LoadLibraryA"])]);
    img.set(set, "LoadLibraryW", pc_binding("LoadLibraryW")[0]);
    img.set(set, "LoadLibraryA", pc_binding("LoadLibraryA")[0]);
    routes_rt::forget_range(img.buf.as_ptr() as usize, img.buf.len());
    let mut hits = Vec::new();
    let first = unsafe { routes::patch_image(img.buf.as_mut_ptr(), &redirects, &mut hits) };
    assert!(first >= 4, "{hits:?}");
    for d in [set, direct] {
        for f in ["LoadLibraryW", "LoadLibraryA"] {
            assert!(routes_rt::is_routing_hook(img.slot(d, f)), "{d}!{f}: {hits:?}");
        }
    }
    for _ in 0..3 {
        let mut h2 = Vec::new();
        assert_eq!(unsafe { routes::patch_image(img.buf.as_mut_ptr(), &redirects, &mut h2) }, 0, "{h2:?}");
    }
    let a32l = routes::module("advapi32legacy") as *mut c_void;
    unsafe {
        for d in [set, direct] {
            let lw: LoadW = core::mem::transmute(img.slot(d, "LoadLibraryW"));
            let la: LoadA = core::mem::transmute(img.slot(d, "LoadLibraryA"));
            assert_eq!(lw(w("advapi32legacy.dll").as_ptr()), a32l);
            assert_eq!(la(b"advapi32legacy.dll\0".as_ptr()), a32l);
            let v = la(b"version.dll\0".as_ptr());
            assert!(!v.is_null());
            assert!(compat_common::os_host::patch::module_path(v).to_ascii_lowercase().starts_with(compat_common::os_host::patch::windows_dir()));
        }
    }
}

unsafe fn loader_count(h: *mut c_void) -> u32 {
    let nt = LoadLibraryW(w("ntdll.dll").as_ptr());
    let find: unsafe extern "system" fn(*mut c_void, *mut usize) -> i32 = core::mem::transmute(GetProcAddress(nt, b"LdrFindEntryForAddress\0".as_ptr()));
    let mut e = 0usize;
    assert_eq!(find(h, &mut e), 0);
    *((*((e + 0x98) as *const usize) + 0x18) as *const u32)
}

#[test]
fn a_routed_load_takes_a_loader_reference_that_free_library_balances() {
    setup();
    let dir = built_dlls::ensure(&[("pixevt", "pixevt.dll")]);
    let cur = std::env::var_os(routes::DIR_VAR).unwrap();
    let mut all: Vec<PathBuf> = std::env::split_paths(&cur).collect();
    if !all.contains(&dir) {
        all.push(dir);
    }
    std::env::set_var(routes::DIR_VAR, std::env::join_paths(all).unwrap());
    use compat_common::os_host::routes_rt;
    extern "system" {
        fn FreeLibrary(h: *mut c_void) -> i32;
    }
    unsafe {
        let h = routes_rt::resolve_load("pixevt.dll", 0);
        assert!(!h.is_null());
        assert_eq!(h as usize, routes::loaded("pixevt"));
        let base = loader_count(h);
        assert!(base >= 2, "{base}");
        let again = routes_rt::resolve_load("PIXEVT", 0);
        assert_eq!(again, h);
        assert_eq!(loader_count(h), base + 1);
        assert!(FreeLibrary(again) != 0);
        assert_eq!(loader_count(h), base);
        assert!(FreeLibrary(h) != 0);
        assert_eq!(loader_count(h), base - 1);
        assert!(!GetProcAddress(h, b"PIXBeginCapture1\0".as_ptr()).is_null());
    }
}
