use core::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};

extern "system" {
    fn GetModuleHandleW(name: *const u16) -> *mut c_void;
    fn GetProcAddress(m: *mut c_void, name: *const u8) -> *mut c_void;
    fn GetSystemWindowsDirectoryW(buf: *mut u16, n: u32) -> u32;
}

static WATCHING: AtomicBool = AtomicBool::new(false);

fn windows_dir() -> &'static str {
    static W: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    W.get_or_init(|| {
        let mut buf = [0u16; 512];
        let n = unsafe { GetSystemWindowsDirectoryW(buf.as_mut_ptr(), buf.len() as u32) } as usize;
        let mut s = String::from_utf16_lossy(&buf[..n.min(buf.len())]).to_ascii_lowercase();
        if !s.ends_with('\\') {
            s.push('\\');
        }
        s
    })
}

pub fn is_title_path(path: &str) -> bool {
    !path.to_ascii_lowercase().starts_with(windows_dir())
}

unsafe extern "system" fn loaded(reason: u32, data: *const u8, _ctx: *mut c_void) {
    if reason != 1 || data.is_null() {
        return;
    }
    let full = *(data.add(8) as *const *const u16);
    let base = *(data.add(24) as *const *mut u8);
    if full.is_null() || base.is_null() {
        return;
    }
    let len = *(full as *const u16) as usize / 2;
    let buf = *(full.add(4) as *const *const u16);
    if buf.is_null() {
        return;
    }
    let path = String::from_utf16_lossy(core::slice::from_raw_parts(buf, len));
    if !is_title_path(&path) || crate::os_host::foreign::is_host_injected(&path) {
        return;
    }
    let n = super::hook::patch_late(base);
    if n > 0 {
        let name = path.rsplit('\\').next().unwrap_or(&path).to_string();
        super::log("late_load", &format!("{name}: {n} import slot(s) routed through the emulated runtime hooks"));
    }
}

pub fn watch() -> bool {
    if WATCHING.load(Ordering::Acquire) {
        return true;
    }
    crate::os_host::foreign::prime();
    unsafe {
        let nt: Vec<u16> = "ntdll.dll\0".encode_utf16().collect();
        let h = GetModuleHandleW(nt.as_ptr());
        if h.is_null() {
            return false;
        }
        let p = GetProcAddress(h, b"LdrRegisterDllNotification\0".as_ptr());
        if p.is_null() {
            return false;
        }
        let f: unsafe extern "system" fn(u32, unsafe extern "system" fn(u32, *const u8, *mut c_void), *mut c_void, *mut *mut c_void) -> i32 = core::mem::transmute(p);
        let mut cookie = core::ptr::null_mut();
        let ok = f(0, loaded, core::ptr::null_mut(), &mut cookie) >= 0;
        WATCHING.store(ok, Ordering::Release);
        ok
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_paths_are_not_title_modules() {
        let win = windows_dir().to_string();
        assert!(!is_title_path(&format!("{win}System32\\winhttp.dll")));
        assert!(!is_title_path(&format!("{}SYSTEM32\\XGameRuntime.dll", win.to_ascii_uppercase())));
        assert!(is_title_path(r"D:\games\Fortnite\GameChat2.dll"));
    }
}
