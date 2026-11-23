use super::patch::{self, dll_matches, Hook};
use core::ffi::c_void;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Route {
    pub console: &'static str,
    pub func: &'static str,
    pub ordinal: u16,
    pub lib: &'static str,
    pub build: u32,
    pub status: &'static str,
    pub forward: &'static str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ApiSetHost {
    pub build: u32,
    pub apiset: &'static str,
    pub func: &'static str,
    pub host: &'static str,
}

include!(concat!(env!("OUT_DIR"), "/compat_routes.rs"));

pub const DIR_VAR: &str = "TOMBOX_COMPAT_DIR";
pub const ROUTES_VAR: &str = "TOMBOX_COMPAT_ROUTES";

#[link(name = "kernel32")]
extern "system" {
    fn GetModuleHandleW(n: *const u16) -> *mut c_void;
    fn LoadLibraryExW(n: *const u16, f: *mut c_void, flags: u32) -> *mut c_void;
    fn GetProcAddress(h: *mut c_void, name: *const u8) -> *mut c_void;
    fn GetSystemDirectoryW(buf: *mut u16, n: u32) -> u32;
}

const LOAD_WITH_ALTERED_SEARCH_PATH: u32 = 0x8;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

pub fn is_api_set(dll: &str) -> bool {
    let d = dll.to_ascii_lowercase();
    d.starts_with("api-ms-") || d.starts_with("ext-ms-")
}

fn norm(dll: &str) -> String {
    let d = dll.to_ascii_lowercase();
    if d.ends_with(".dll") {
        d
    } else {
        format!("{d}.dll")
    }
}

pub fn host_of(dll: &str, func: &str, build: u32) -> Option<String> {
    let d = norm(dll);
    if !is_api_set(&d) {
        return Some(d);
    }
    let rows = apiset_index().get(&(d, func.to_string()))?;
    let pick = rows
        .iter()
        .filter(|h| h.build <= build)
        .max_by_key(|h| h.build)
        .or_else(|| rows.iter().min_by_key(|h| h.build))?;
    Some(pick.host.to_string())
}

fn known_builds() -> &'static Vec<u32> {
    static K: OnceLock<Vec<u32>> = OnceLock::new();
    K.get_or_init(|| {
        let mut v: Vec<u32> = APISET_HOSTS.iter().map(|h| h.build).chain(ROUTES.iter().map(|r| r.build)).filter(|b| *b != 0).collect();
        v.sort_unstable();
        v.dedup();
        v
    })
}

pub fn variant(console: &str, build: u32) -> u32 {
    let Some(vs) = variants().get(console) else { return 0 };
    for (i, b) in vs.iter().enumerate() {
        let upper = known_builds().iter().copied().find(|k| k > b).unwrap_or(b + 1);
        if build < upper && (build >= *b || i == 0) {
            return *b;
        }
    }
    0
}

pub fn current_build() -> u32 {
    super::k32legacy::console_os_build()
}

pub fn lib_for(console: &str, build: u32) -> Option<&'static str> {
    let v = variant(console, build);
    ROUTES.iter().find(|r| r.console == console && r.build == v).map(|r| r.lib)
}

pub fn console_of(lib: &str) -> Option<&'static str> {
    ROUTES.iter().find(|r| r.lib.eq_ignore_ascii_case(lib)).map(|r| r.console)
}

pub fn loaded(lib: &str) -> usize {
    modules().lock().unwrap_or_else(|e| e.into_inner()).get(&lib.to_ascii_lowercase()).copied().unwrap_or(0)
}

fn ordinal_of(func: &str) -> Option<u16> {
    func.strip_prefix('#').and_then(|n| n.parse().ok())
}

pub fn lookup(dll: &str, func: &str, build: u32) -> Option<&'static Route> {
    let host = host_of(dll, func, build)?;
    let v = variant(&host, build);
    route_index().get(&(host, v, func.to_string())).copied()
}

type RouteKey = (String, u32, String);

fn route_index() -> &'static HashMap<RouteKey, &'static Route> {
    static I: OnceLock<HashMap<RouteKey, &'static Route>> = OnceLock::new();
    I.get_or_init(|| {
        let mut m = HashMap::new();
        for r in ROUTES {
            if ordinal_of(r.func).is_none() {
                m.entry((r.console.to_string(), r.build, r.func.to_string())).or_insert(r);
            }
            m.entry((r.console.to_string(), r.build, format!("#{}", r.ordinal))).or_insert(r);
        }
        m
    })
}

fn apiset_index() -> &'static HashMap<(String, String), Vec<&'static ApiSetHost>> {
    static I: OnceLock<HashMap<(String, String), Vec<&'static ApiSetHost>>> = OnceLock::new();
    I.get_or_init(|| {
        let mut m: HashMap<(String, String), Vec<&'static ApiSetHost>> = HashMap::new();
        for h in APISET_HOSTS {
            m.entry((h.apiset.to_string(), h.func.to_string())).or_default().push(h);
        }
        m
    })
}

fn variants() -> &'static HashMap<&'static str, Vec<u32>> {
    static I: OnceLock<HashMap<&'static str, Vec<u32>>> = OnceLock::new();
    I.get_or_init(|| {
        let mut m: HashMap<&'static str, Vec<u32>> = HashMap::new();
        for r in ROUTES.iter().filter(|r| r.build != 0) {
            m.entry(r.console).or_default().push(r.build);
        }
        for v in m.values_mut() {
            v.sort_unstable();
            v.dedup();
        }
        m
    })
}

pub fn console_dlls() -> Vec<&'static str> {
    let mut v: Vec<&'static str> = ROUTES.iter().map(|r| r.console).collect();
    v.sort_unstable();
    v.dedup();
    v
}

pub fn enabled() -> bool {
    std::env::var(ROUTES_VAR).ok().as_deref() != Some("0")
}

fn system_dir() -> PathBuf {
    let mut buf = [0u16; 520];
    let n = unsafe { GetSystemDirectoryW(buf.as_mut_ptr(), buf.len() as u32) } as usize;
    PathBuf::from(String::from_utf16_lossy(&buf[..n.min(buf.len())]))
}

pub fn shadows_system_dll(file: &str) -> bool {
    system_dir().join(file).is_file()
}

pub fn search_dirs() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Ok(d) = std::env::var(DIR_VAR) {
        for p in std::env::split_paths(&d) {
            if !p.as_os_str().is_empty() {
                v.push(p);
            }
        }
    }
    let own = patch::self_module();
    if own != 0 {
        let p = unsafe { patch::module_path(own as *mut c_void) };
        if let Some(d) = Path::new(&p).parent() {
            v.push(d.to_path_buf());
        }
    }
    if let Some(d) = std::env::current_exe().ok().and_then(|e| e.parent().map(|p| p.to_path_buf())) {
        v.push(d);
    }
    v.dedup();
    v
}

pub fn find_file(lib: &str) -> Option<PathBuf> {
    let file = format!("{lib}.dll");
    let mut vars = vec![format!("TOMBOX_{}", lib.to_ascii_uppercase())];
    if let Some(c) = console_of(lib) {
        let stem = format!("TOMBOX_{}", c.trim_end_matches(".dll").to_ascii_uppercase());
        if !vars.contains(&stem) {
            vars.push(stem);
        }
    }
    for var in vars {
        if let Ok(p) = std::env::var(&var) {
            let p = p.trim();
            if p == "0" {
                return None;
            }
            if !p.is_empty() && Path::new(p).is_file() {
                return Some(PathBuf::from(p));
            }
        }
    }
    search_dirs().into_iter().map(|d| d.join(&file)).find(|p| p.is_file())
}

fn alias_copy(src: &Path, file: &str) -> Option<PathBuf> {
    let dir = std::env::temp_dir().join("tombox-compat-routes");
    std::fs::create_dir_all(&dir).ok()?;
    let len = std::fs::metadata(src).ok()?.len();
    let stem = format!("tbx_{file}");
    let dst = dir.join(&stem);
    if std::fs::metadata(&dst).map(|m| m.len() == len).unwrap_or(false) && std::fs::read(&dst).ok() == std::fs::read(src).ok() {
        return Some(dst);
    }
    let tmp = dir.join(format!("{stem}.{}.tmp", std::process::id()));
    if std::fs::copy(src, &tmp).is_ok() {
        if std::fs::rename(&tmp, &dst).is_ok() {
            return Some(dst);
        }
        let _ = std::fs::remove_file(&tmp);
    }
    let dst = dir.join(format!("tbx_{}_{file}", std::process::id()));
    std::fs::copy(src, &dst).ok().map(|_| dst)
}

fn modules() -> &'static Mutex<HashMap<String, usize>> {
    static M: OnceLock<Mutex<HashMap<String, usize>>> = OnceLock::new();
    M.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn module(lib: &str) -> usize {
    let key = lib.to_ascii_lowercase();
    thread_local! {
        static IN_LOAD: core::cell::Cell<bool> = const { core::cell::Cell::new(false) };
    }
    static LOAD: Mutex<()> = Mutex::new(());
    let cached = || modules().lock().unwrap_or_else(|e| e.into_inner()).get(&key).copied();
    if let Some(h) = cached() {
        return h;
    }
    let nested = IN_LOAD.try_with(|c| c.get()).unwrap_or(true);
    let _g = if nested { None } else { Some(LOAD.lock().unwrap_or_else(|e| e.into_inner())) };
    if let Some(h) = cached() {
        return h;
    }
    let _ = IN_LOAD.try_with(|c| c.set(true));
    let h = unsafe { load(&key) };
    if !nested {
        let _ = IN_LOAD.try_with(|c| c.set(false));
    }
    modules().lock().unwrap_or_else(|e| e.into_inner()).insert(key, h);
    h
}

unsafe fn load(lib: &str) -> usize {
    let file = format!("{lib}.dll");
    let shadows = shadows_system_dll(&file);
    if !shadows {
        let h = GetModuleHandleW(wide(&file).as_ptr());
        if !h.is_null() {
            let p = patch::module_path(h);
            if !p.to_ascii_lowercase().starts_with(patch::windows_dir()) {
                return h as usize;
            }
        }
    }
    let Some(src) = find_file(lib) else { return 0 };
    let path = if shadows { alias_copy(&src, &file).unwrap_or(src) } else { src };
    let h = LoadLibraryExW(wide(&path.display().to_string()).as_ptr(), core::ptr::null_mut(), LOAD_WITH_ALTERED_SEARCH_PATH);
    if !h.is_null() {
        crate::record_raw("os_host", "compat_routes", &format!("loaded {} for {file}", path.display()));
    }
    h as usize
}

pub unsafe fn target(r: &Route) -> usize {
    if !r.forward.is_empty() {
        let f = r.forward.trim_end_matches(".dll");
        if ROUTES.iter().any(|x| x.lib.eq_ignore_ascii_case(f)) {
            module(f);
        }
    }
    let h = module(r.lib);
    if h == 0 {
        return 0;
    }
    let p = if ordinal_of(r.func).is_some() {
        GetProcAddress(h as *mut c_void, r.ordinal as usize as *const u8)
    } else {
        GetProcAddress(h as *mut c_void, cstr0(r.func).as_ptr())
    };
    p as usize
}

fn cstr0(s: &str) -> Vec<u8> {
    s.bytes().chain(Some(0)).collect()
}

pub fn covered(dll: &str, func: &str, hooks: &[Hook]) -> bool {
    hooks.iter().any(|h| h.name == func && dll_matches(dll, h.dlls))
}

pub struct Import {
    pub dll: String,
    pub func: String,
    pub slot: *mut usize,
}

unsafe fn rd32(p: *const u8) -> u32 {
    core::ptr::read_unaligned(p as *const u32)
}

unsafe fn cstr(p: *const u8) -> String {
    let mut n = 0;
    while n < 512 && *p.add(n) != 0 {
        n += 1;
    }
    String::from_utf8_lossy(core::slice::from_raw_parts(p, n)).into_owned()
}

unsafe fn table(base: *mut u8, dll: &str, int_rva: usize, iat_rva: usize, out: &mut Vec<Import>) {
    if iat_rva == 0 {
        return;
    }
    let lookup = if int_rva != 0 { int_rva } else { iat_rva };
    let mut i = 0usize;
    loop {
        let t = core::ptr::read_unaligned(base.add(lookup + i * 8) as *const u64);
        if t == 0 {
            break;
        }
        let func = if t & (1 << 63) != 0 {
            format!("#{}", t & 0xffff)
        } else if (t as usize) < 0x7fff_ffff {
            cstr(base.add(t as usize + 2))
        } else {
            String::new()
        };
        if !func.is_empty() {
            out.push(Import { dll: dll.to_string(), func, slot: base.add(iat_rva + i * 8) as *mut usize });
        }
        i += 1;
    }
}

pub unsafe fn imports(base: *mut u8) -> Vec<Import> {
    let mut out = Vec::new();
    if base.is_null() || core::ptr::read_unaligned(base as *const u16) != 0x5a4d {
        return out;
    }
    let pe = base.add(rd32(base.add(0x3c)) as usize);
    if rd32(pe) != 0x4550 || core::ptr::read_unaligned(pe.add(24) as *const u16) != 0x20b {
        return out;
    }
    let dirs = pe.add(24 + 112);
    let imp = rd32(dirs.add(8)) as usize;
    if imp != 0 {
        let mut d = base.add(imp);
        loop {
            let int_rva = rd32(d) as usize;
            let name_rva = rd32(d.add(12)) as usize;
            let iat_rva = rd32(d.add(16)) as usize;
            if name_rva == 0 && iat_rva == 0 {
                break;
            }
            if name_rva != 0 {
                table(base, &cstr(base.add(name_rva)).to_ascii_lowercase(), int_rva, iat_rva, &mut out);
            }
            d = d.add(20);
        }
    }
    let delay = rd32(dirs.add(13 * 8)) as usize;
    if delay != 0 {
        let mut d = base.add(delay);
        loop {
            let name_rva = rd32(d.add(4)) as usize;
            if name_rva == 0 {
                break;
            }
            let iat_rva = rd32(d.add(12)) as usize;
            let int_rva = rd32(d.add(16)) as usize;
            table(base, &cstr(base.add(name_rva)).to_ascii_lowercase(), int_rva, iat_rva, &mut out);
            d = d.add(32);
        }
    }
    out
}

pub unsafe fn patch_image_for(base: *mut u8, build: u32, skip: &[Hook], hits: &mut Vec<String>) -> usize {
    let mut n = 0;
    for imp in imports(base) {
        if covered(&imp.dll, &imp.func, skip) {
            continue;
        }
        let Some(r) = lookup(&imp.dll, &imp.func, build) else { continue };
        let t = target(r);
        if t == 0 {
            continue;
        }
        let cur = core::ptr::read_volatile(imp.slot);
        if super::routes_rt::routes_to(cur, &imp.func, t) {
            continue;
        }
        if cur != t && patch::write_slot(imp.slot, t) {
            n += 1;
            hits.push(format!("{}!{} -> {}", imp.dll, imp.func, r.lib));
        }
    }
    n
}

pub unsafe fn patch_image(base: *mut u8, skip: &[Hook], hits: &mut Vec<String>) -> usize {
    if !enabled() {
        return 0;
    }
    patch_image_for(base, current_build(), skip, hits) + super::routes_rt::patch_image(base, skip, hits)
}

pub unsafe fn sweep(skip: &[Hook], report: &mut Vec<(String, Vec<String>)>) -> usize {
    let mut total = 0;
    if !enabled() {
        return 0;
    }
    for m in patch::loaded_modules() {
        let Some(pinned) = patch::pin_module(m) else { continue };
        let path = patch::module_path(pinned);
        if !path.is_empty() && patch::is_title_module(&path, pinned as usize) && !skip_module(&path, pinned as usize) && !super::foreign::is_host_injected(&path) {
            let mut hits = Vec::new();
            let n = patch_image(pinned as *mut u8, skip, &mut hits);
            if n > 0 {
                total += n;
                report.push((path.rsplit('\\').next().unwrap_or(&path).to_string(), hits));
            }
        }
        patch::unpin_module(pinned);
    }
    total
}

pub fn is_compat_module(base: usize) -> bool {
    modules().lock().unwrap_or_else(|e| e.into_inner()).values().any(|h| *h == base)
}

pub fn skip_module(path: &str, base: usize) -> bool {
    if is_compat_module(base) {
        return true;
    }
    if path.to_ascii_lowercase().starts_with(patch::windows_dir()) {
        return false;
    }
    let file = path.rsplit(['\\', '/']).next().unwrap_or(path).to_ascii_lowercase();
    let stem = file.strip_suffix(".dll").unwrap_or(&file);
    let stem = stem.strip_prefix("tbx_").unwrap_or(stem);
    let stem = match stem.split_once('_') {
        Some((pid, rest)) if !pid.is_empty() && pid.bytes().all(|b| b.is_ascii_digit()) && file.starts_with("tbx_") => rest,
        _ => stem,
    };
    ROUTES.iter().any(|r| r.lib.eq_ignore_ascii_case(stem))
}
