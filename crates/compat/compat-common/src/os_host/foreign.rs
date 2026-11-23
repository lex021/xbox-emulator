use core::ffi::c_void;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

#[link(name = "kernel32")]
extern "system" {
    fn GetLongPathNameW(short: *const u16, long: *mut u16, n: u32) -> u32;
}

pub fn normalize(path: &str) -> String {
    let mut s = path.replace('/', "\\");
    for p in ["\\\\?\\UNC\\", "\\??\\UNC\\"] {
        if s.len() >= p.len() && s[..p.len()].eq_ignore_ascii_case(p) {
            s = format!("\\\\{}", &s[p.len()..]);
        }
    }
    for p in ["\\\\?\\", "\\??\\", "\\\\.\\"] {
        if s.starts_with(p) {
            s = s[p.len()..].to_string();
        }
    }
    if s.contains('~') {
        let w: Vec<u16> = s.encode_utf16().chain(Some(0)).collect();
        let mut buf = vec![0u16; 32768];
        let n = unsafe { GetLongPathNameW(w.as_ptr(), buf.as_mut_ptr(), buf.len() as u32) } as usize;
        if n > 0 && n < buf.len() {
            s = String::from_utf16_lossy(&buf[..n]);
        }
    }
    while s.contains("\\\\") && !s.starts_with("\\\\") {
        s = s.replace("\\\\", "\\");
    }
    let mut s = s.to_ascii_lowercase();
    if !s.ends_with('\\') && Path::new(&s).extension().is_none() {
        s.push('\\');
    }
    s
}

fn root_form(p: &Path) -> Option<String> {
    let s = p.to_string_lossy();
    if s.is_empty() {
        return None;
    }
    let mut n = normalize(&s);
    if !n.ends_with('\\') {
        n.push('\\');
    }
    Some(n)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    System,
    Game,
    Emulator,
    HostInjected,
}

pub fn classify_with(path: &str, windows: &str, game: &[String], emulator: &[String]) -> Origin {
    let p = normalize(path);
    let under = |r: &String| !r.is_empty() && p.starts_with(r.as_str());
    if under(&normalize(windows)) {
        return Origin::System;
    }
    if game.iter().any(under) {
        return Origin::Game;
    }
    if emulator.iter().any(under) {
        return Origin::Emulator;
    }
    Origin::HostInjected
}

fn game_roots() -> &'static Vec<String> {
    static G: OnceLock<Vec<String>> = OnceLock::new();
    G.get_or_init(|| {
        let mut v: Vec<PathBuf> = Vec::new();
        if let Some(d) = std::env::current_exe().ok().and_then(|e| e.parent().map(|p| p.to_path_buf())) {
            v.push(d);
        }
        v.push(super::dlldir::package_root().clone());
        if let Some(d) = crate::gdk::title().dir.clone() {
            v.push(d);
        }
        for k in ["TOMBOX_PACKAGE_ROOT", "TOMBOX_GAME_DIR", "TOMBOX_GAME_ROOT"] {
            if let Ok(s) = std::env::var(k) {
                v.push(PathBuf::from(s));
            }
        }
        let mut out: Vec<String> = v.iter().filter_map(|p| root_form(p)).collect();
        out.sort();
        out.dedup();
        out
    })
}

fn emulator_roots() -> &'static Vec<String> {
    static E: OnceLock<Vec<String>> = OnceLock::new();
    E.get_or_init(|| {
        let mut v: Vec<PathBuf> = Vec::new();
        let own = super::patch::self_module();
        if own != 0 {
            let p = unsafe { super::patch::module_path(own as *mut c_void) };
            if let Some(d) = Path::new(&p).parent() {
                v.push(d.to_path_buf());
            }
        }
        for k in ["TOMBOX_DLL_DIR", "TOMBOX_ROOT", "TOMBOX_EMULATOR_DIR", "TOMBOX_GDK_ROOT"] {
            if let Ok(s) = std::env::var(k) {
                v.push(PathBuf::from(s));
            }
        }
        let mut out: Vec<String> = v.iter().filter_map(|p| root_form(p)).collect();
        out.sort();
        out.dedup();
        out
    })
}

pub fn prime() {
    let _ = (game_roots(), emulator_roots());
}

pub fn foreign_allowed() -> bool {
    std::env::var("TOMBOX_LATE_LOAD_FOREIGN").map(|v| v == "1").unwrap_or(false)
}

pub fn origin(path: &str) -> Origin {
    classify_with(path, super::patch::windows_dir(), game_roots(), emulator_roots())
}

pub fn is_host_injected(path: &str) -> bool {
    if path.is_empty() || foreign_allowed() {
        return false;
    }
    if origin(path) != Origin::HostInjected {
        return false;
    }
    static SEEN: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    let key = normalize(path);
    if SEEN.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner()).insert(key) {
        crate::record_raw("os_host", "late_load", &format!("late_load: {path} is host-injected (outside game/emulator), imports left native"));
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roots() -> (String, Vec<String>, Vec<String>) {
        ("C:\\Windows\\".into(), vec![normalize("Y:\\"), normalize("D:\\xbox-pc-port\\games\\Terraria\\extracted\\pkg\\")], vec![normalize("D:\\TomBox\\target\\release\\")])
    }

    #[test]
    fn game_mount_and_package_paths_are_game() {
        let (w, g, e) = roots();
        assert_eq!(classify_with("Y:\\Terraria.exe", &w, &g, &e), Origin::Game);
        assert_eq!(classify_with("y:\\Bin\\XCurl.dll", &w, &g, &e), Origin::Game);
        assert_eq!(classify_with("D:\\XBOX-PC-PORT\\games\\Terraria\\extracted\\pkg\\x.dll", &w, &g, &e), Origin::Game);
        assert_eq!(classify_with("\\\\?\\Y:\\Bin\\a.dll", &w, &g, &e), Origin::Game);
        assert_eq!(classify_with("Y:/Bin/a.dll", &w, &g, &e), Origin::Game);
    }

    #[test]
    fn emulator_directory_is_emulator() {
        let (w, g, e) = roots();
        assert_eq!(classify_with("D:\\TomBox\\target\\release\\xmem.dll", &w, &g, &e), Origin::Emulator);
        assert_eq!(classify_with("d:\\tombox\\TARGET\\Release\\d3d12_xs.dll", &w, &g, &e), Origin::Emulator);
    }

    #[test]
    fn overlays_and_program_files_are_host_injected() {
        let (w, g, e) = roots();
        assert_eq!(classify_with("C:\\Users\\someone\\AppData\\Local\\Discord\\app-1.0.9\\modules\\discord_hook\\DiscordHook64.dll", &w, &g, &e), Origin::HostInjected);
        assert_eq!(classify_with("C:\\Program Files (x86)\\Steam\\GameOverlayRenderer64.dll", &w, &g, &e), Origin::HostInjected);
        assert_eq!(classify_with("C:\\Program Files\\NVIDIA Corporation\\NvContainer\\nvspcap64.dll", &w, &g, &e), Origin::HostInjected);
        assert_eq!(classify_with("\\\\server\\share\\hook.dll", &w, &g, &e), Origin::HostInjected);
        assert_eq!(classify_with("\\\\?\\UNC\\server\\share\\hook.dll", &w, &g, &e), Origin::HostInjected);
        assert_eq!(classify_with("Y:\\..hidden", &w, &g, &e), Origin::Game);
    }

    #[test]
    fn system_directory_is_system_in_any_case() {
        let (w, g, e) = roots();
        assert_eq!(classify_with("C:\\WINDOWS\\System32\\d3d12.dll", &w, &g, &e), Origin::System);
        assert_eq!(classify_with("\\??\\c:\\windows\\system32\\nvwgf2umx.dll", &w, &g, &e), Origin::System);
    }

    #[test]
    fn prefix_match_is_on_directory_boundaries() {
        let w = "C:\\Windows\\".to_string();
        let g = vec![normalize("D:\\games\\Terraria\\")];
        let e = vec![normalize("D:\\TomBox\\target\\release\\")];
        assert_eq!(classify_with("D:\\games\\TerrariaMods\\hook.dll", &w, &g, &e), Origin::HostInjected);
        assert_eq!(classify_with("D:\\TomBox\\target\\release2\\x.dll", &w, &g, &e), Origin::HostInjected);
    }

    #[test]
    fn short_names_expand_before_matching() {
        let dir = std::env::temp_dir().join("tombox foreign long name test");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("hook.dll");
        std::fs::write(&file, b"x").unwrap();
        let long = file.to_string_lossy().into_owned();
        let w: Vec<u16> = long.encode_utf16().chain(Some(0)).collect();
        let mut buf = vec![0u16; 1024];
        #[link(name = "kernel32")]
        extern "system" {
            fn GetShortPathNameW(l: *const u16, s: *mut u16, n: u32) -> u32;
        }
        let n = unsafe { GetShortPathNameW(w.as_ptr(), buf.as_mut_ptr(), buf.len() as u32) } as usize;
        let short = String::from_utf16_lossy(&buf[..n]);
        let g = vec![normalize(&dir.to_string_lossy())];
        if short.contains('~') {
            assert_eq!(classify_with(&short, "C:\\Windows\\", &g, &[]), Origin::Game, "{short}");
        }
        assert_eq!(classify_with(&long, "C:\\Windows\\", &g, &[]), Origin::Game);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
