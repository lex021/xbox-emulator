use core::ffi::c_void;

#[link(name = "kernel32")]
extern "system" {
    fn VirtualProtect(addr: *mut c_void, size: usize, new_protect: u32, old_protect: *mut u32) -> i32;
    fn VirtualQuery(addr: *const c_void, info: *mut [u8; 48], len: usize) -> usize;
    fn K32EnumProcessModules(p: *mut c_void, mods: *mut *mut c_void, cb: u32, needed: *mut u32) -> i32;
    fn GetCurrentProcess() -> *mut c_void;
    fn GetModuleFileNameW(m: *mut c_void, buf: *mut u16, n: u32) -> u32;
    fn GetSystemWindowsDirectoryW(buf: *mut u16, n: u32) -> u32;
    fn GetModuleHandleExW(flags: u32, addr: *const u16, out: *mut *mut c_void) -> i32;
    fn FreeLibrary(m: *mut c_void) -> i32;
}

#[derive(Clone, Copy)]
pub struct Hook {
    pub name: &'static str,
    pub dlls: &'static [&'static str],
    pub hook: usize,
}

unsafe fn rd32(p: *const u8) -> u32 {
    core::ptr::read_unaligned(p as *const u32)
}

unsafe fn cstr_lower(p: *const u8) -> String {
    let mut v = Vec::new();
    let mut i = 0;
    while i < 512 {
        let c = *p.add(i);
        if c == 0 {
            break;
        }
        v.push(c.to_ascii_lowercase());
        i += 1;
    }
    String::from_utf8_lossy(&v).into_owned()
}

unsafe fn cstr_eq(p: *const u8, s: &str) -> bool {
    let b = s.as_bytes();
    for (i, c) in b.iter().enumerate() {
        if *p.add(i) != *c {
            return false;
        }
    }
    *p.add(b.len()) == 0
}

pub fn dll_matches(dll: &str, prefixes: &[&str]) -> bool {
    let d = dll.to_ascii_lowercase();
    prefixes.iter().any(|p| d.starts_with(p))
}

pub unsafe fn write_slot(slot: *mut usize, value: usize) -> bool {
    let mut info = [0u8; 48];
    let mut prot = 0x04u32;
    if VirtualQuery(slot as *const c_void, &mut info, 48) != 0 {
        let cur = u32::from_le_bytes([info[36], info[37], info[38], info[39]]);
        if cur & 0xf0 != 0 {
            prot = 0x40;
        }
    }
    let mut old = 0u32;
    if VirtualProtect(slot as *mut c_void, 8, prot, &mut old) == 0 {
        return false;
    }
    core::ptr::write_volatile(slot, value);
    let mut tmp = 0u32;
    VirtualProtect(slot as *mut c_void, 8, old, &mut tmp);
    true
}

unsafe fn patch_table(base: *mut u8, dll: &str, int_rva: usize, iat_rva: usize, hooks: &[Hook], hits: &mut Vec<String>) -> usize {
    let wanted: Vec<&Hook> = hooks.iter().filter(|h| dll_matches(dll, h.dlls)).collect();
    if wanted.is_empty() || iat_rva == 0 {
        return 0;
    }
    let lookup = if int_rva != 0 { int_rva } else { iat_rva };
    let mut n = 0;
    let mut i = 0usize;
    loop {
        let thunk = core::ptr::read_unaligned(base.add(lookup + i * 8) as *const u64);
        if thunk == 0 {
            break;
        }
        if thunk & (1 << 63) == 0 && (thunk as usize) < 0x7fff_ffff {
            let fname = base.add(thunk as usize + 2);
            for h in &wanted {
                if cstr_eq(fname, h.name) {
                    let slot = base.add(iat_rva + i * 8) as *mut usize;
                    if core::ptr::read_volatile(slot) != h.hook && write_slot(slot, h.hook) {
                        n += 1;
                        hits.push(format!("{dll}!{}", h.name));
                    }
                }
            }
        }
        i += 1;
    }
    n
}

pub unsafe fn patch_image(base: *mut u8, hooks: &[Hook], hits: &mut Vec<String>) -> usize {
    if base.is_null() || core::ptr::read_unaligned(base as *const u16) != 0x5a4d {
        return 0;
    }
    let pe = base.add(rd32(base.add(0x3c)) as usize);
    if rd32(pe) != 0x4550 || core::ptr::read_unaligned(pe.add(24) as *const u16) != 0x20b {
        return 0;
    }
    let dirs = pe.add(24 + 112);
    let mut n = 0;
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
                let dll = cstr_lower(base.add(name_rva));
                n += patch_table(base, &dll, int_rva, iat_rva, hooks, hits);
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
            let dll = cstr_lower(base.add(name_rva));
            n += patch_table(base, &dll, int_rva, iat_rva, hooks, hits);
            d = d.add(32);
        }
    }
    n
}

pub unsafe fn module_path(m: *mut c_void) -> String {
    let mut buf = [0u16; 1024];
    let n = GetModuleFileNameW(m, buf.as_mut_ptr(), buf.len() as u32) as usize;
    String::from_utf16_lossy(&buf[..n.min(buf.len())])
}

pub fn windows_dir() -> &'static str {
    static W: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    W.get_or_init(|| unsafe {
        let mut buf = [0u16; 512];
        let n = GetSystemWindowsDirectoryW(buf.as_mut_ptr(), buf.len() as u32) as usize;
        let mut s = String::from_utf16_lossy(&buf[..n.min(buf.len())]).to_ascii_lowercase();
        if !s.ends_with('\\') {
            s.push('\\');
        }
        s
    })
}

pub fn self_module() -> usize {
    static M: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *M.get_or_init(|| unsafe {
        let mut h = core::ptr::null_mut();
        GetModuleHandleExW(0x4 | 0x2, self_module as *const () as *const u16, &mut h);
        h as usize
    })
}

pub fn is_title_module(path: &str, base: usize) -> bool {
    base != self_module() && !path.to_ascii_lowercase().starts_with(windows_dir())
}

pub unsafe fn loaded_modules() -> Vec<*mut c_void> {
    let mut v: Vec<*mut c_void> = vec![core::ptr::null_mut(); 2048];
    let mut needed = 0u32;
    if K32EnumProcessModules(GetCurrentProcess(), v.as_mut_ptr(), (v.len() * 8) as u32, &mut needed) == 0 {
        return Vec::new();
    }
    v.truncate((needed as usize / 8).min(2048));
    v
}

pub unsafe fn pin_module(m: *mut c_void) -> Option<*mut c_void> {
    let mut h = core::ptr::null_mut();
    if m.is_null() || GetModuleHandleExW(0x4, m as *const u16, &mut h) == 0 || h.is_null() {
        return None;
    }
    if h != m {
        FreeLibrary(h);
        return None;
    }
    Some(h)
}

pub unsafe fn unpin_module(m: *mut c_void) {
    FreeLibrary(m);
}

pub unsafe fn sweep_modules(mods: &[*mut c_void], hooks: &[Hook], report: &mut Vec<(String, Vec<String>)>) -> usize {
    let mut total = 0;
    for &m in mods {
        let Some(pinned) = pin_module(m) else { continue };
        let path = module_path(pinned);
        if !path.is_empty() && is_title_module(&path, pinned as usize) && !super::foreign::is_host_injected(&path) {
            let mut hits = Vec::new();
            let n = patch_image(pinned as *mut u8, hooks, &mut hits);
            if n > 0 {
                total += n;
                let name = path.rsplit('\\').next().unwrap_or(&path).to_string();
                report.push((name, hits));
            }
        }
        FreeLibrary(pinned);
    }
    total
}

pub unsafe fn sweep(hooks: &[Hook], report: &mut Vec<(String, Vec<String>)>) -> usize {
    sweep_modules(&loaded_modules(), hooks, report)
}

#[cfg(test)]
mod sweep_race_tests {
    use super::*;

    #[link(name = "kernel32")]
    extern "system" {
        fn LoadLibraryW(name: *const u16) -> *mut c_void;
        fn GetModuleHandleW(name: *const u16) -> *mut c_void;
    }

    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }

    static MSIMG: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn sweep_skips_a_module_unloaded_after_enumeration() {
        let _g = MSIMG.lock().unwrap_or_else(|e| e.into_inner());
        unsafe {
            let name = w("msimg32.dll");
            if !GetModuleHandleW(name.as_ptr()).is_null() {
                return;
            }
            let base = LoadLibraryW(name.as_ptr());
            assert!(!base.is_null());
            let listed = vec![base];
            while !GetModuleHandleW(name.as_ptr()).is_null() {
                FreeLibrary(base);
            }
            assert!(pin_module(base).is_none(), "an unloaded base cannot be pinned");
            let mut report = Vec::new();
            assert_eq!(sweep_modules(&listed, &[], &mut report), 0, "the stale base is skipped, never read");
            assert!(report.is_empty());
        }
    }

    #[test]
    fn pinning_a_live_module_keeps_it_loaded_and_releases_the_reference() {
        let _g = MSIMG.lock().unwrap_or_else(|e| e.into_inner());
        unsafe {
            let name = w("msimg32.dll");
            if !GetModuleHandleW(name.as_ptr()).is_null() {
                return;
            }
            let base = LoadLibraryW(name.as_ptr());
            let pinned = pin_module(base).expect("a loaded module pins");
            assert_eq!(pinned, base);
            FreeLibrary(base);
            assert!(!GetModuleHandleW(name.as_ptr()).is_null(), "the pin holds the module while a sweep patches it");
            unpin_module(pinned);
            assert!(GetModuleHandleW(name.as_ptr()).is_null(), "unpin drops the only reference");
        }
    }
}
