use std::sync::OnceLock;

extern "system" {
    fn CopyFileW(a: *const u16, b: *const u16, fail: i32) -> i32;
    fn CopyFileExW(a: *const u16, b: *const u16, p: usize, d: usize, c: usize, f: u32) -> i32;
    fn MoveFileWithProgressW(a: *const u16, b: *const u16, p: usize, d: usize, f: u32) -> i32;
    fn MoveFileExW(a: *const u16, b: *const u16, f: u32) -> i32;
    fn ReplaceFileW(a: *const u16, b: *const u16, c: *const u16, f: u32, x: usize, y: usize) -> i32;
    fn CopyFile2(a: *const u16, b: *const u16, p: usize) -> i32;
}

const PREFIX: &str = "\\\\?\\GlobalRoot\\Device\\Harddisk0\\Partition9\\";

fn root() -> &'static std::path::PathBuf {
    static R: OnceLock<std::path::PathBuf> = OnceLock::new();
    R.get_or_init(|| {
        let d = std::env::temp_dir().join(format!("tombox_fs_redirect_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    })
}

unsafe extern "system" fn resolve(p: *const u16, out: *mut u16, cap: u32) -> u32 {
    let mut n = 0;
    while *p.add(n) != 0 {
        n += 1;
    }
    let s = String::from_utf16_lossy(core::slice::from_raw_parts(p, n));
    if s.len() < PREFIX.len() || !s[..PREFIX.len()].eq_ignore_ascii_case(PREFIX) {
        return 0;
    }
    let t: Vec<u16> = root().join(&s[PREFIX.len()..]).to_string_lossy().encode_utf16().collect();
    if t.len() as u32 >= cap {
        return t.len() as u32 + 1;
    }
    core::ptr::copy_nonoverlapping(t.as_ptr(), out, t.len());
    *out.add(t.len()) = 0;
    t.len() as u32
}

fn w(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
}

fn console(name: &str) -> Vec<u16> {
    w(&format!("{PREFIX}{name}"))
}

fn hook(name: &str, real: usize) -> usize {
    compat_common::gdk::xcrd_paths::register(resolve as usize);
    let t = compat_common::gdk::fs::targets();
    let (_, h, orig) = t.iter().find(|r| r.0 == name).unwrap_or_else(|| panic!("{name} is not redirected"));
    orig.store(real, std::sync::atomic::Ordering::Release);
    *h
}

fn put(name: &str, body: &[u8]) {
    std::fs::write(root().join(name), body).unwrap();
}

fn get(name: &str) -> Option<Vec<u8>> {
    std::fs::read(root().join(name)).ok()
}

#[test]
fn copy_move_and_replace_calls_follow_the_console_path_like_create_file() {
    unsafe {
        put("a.bin", b"one");
        let f: unsafe extern "system" fn(*const u16, *const u16, i32) -> i32 = core::mem::transmute(hook("CopyFileW", CopyFileW as usize));
        assert_ne!(f(console("a.bin").as_ptr(), console("b.bin").as_ptr(), 1), 0);
        assert_eq!(get("b.bin").as_deref(), Some(&b"one"[..]));

        let f: unsafe extern "system" fn(*const u16, *const u16, usize, usize, usize, u32) -> i32 = core::mem::transmute(hook("CopyFileExW", CopyFileExW as usize));
        assert_ne!(f(console("a.bin").as_ptr(), console("c.bin").as_ptr(), 0, 0, 0, 0), 0);
        assert_eq!(get("c.bin").as_deref(), Some(&b"one"[..]));

        let f: unsafe extern "system" fn(*const u16, *const u16, usize) -> i32 = core::mem::transmute(hook("CopyFile2", CopyFile2 as usize));
        assert_eq!(f(console("a.bin").as_ptr(), console("d.bin").as_ptr(), 0), 0);
        assert_eq!(get("d.bin").as_deref(), Some(&b"one"[..]));

        let f: unsafe extern "system" fn(*const u16, *const u16, u32) -> i32 = core::mem::transmute(hook("MoveFileExW", MoveFileExW as usize));
        assert_ne!(f(console("b.bin").as_ptr(), console("e.bin").as_ptr(), 0), 0);
        assert!(get("b.bin").is_none());
        assert_eq!(get("e.bin").as_deref(), Some(&b"one"[..]));

        let f: unsafe extern "system" fn(*const u16, *const u16, usize, usize, u32) -> i32 = core::mem::transmute(hook("MoveFileWithProgressW", MoveFileWithProgressW as usize));
        assert_ne!(f(console("c.bin").as_ptr(), console("f.bin").as_ptr(), 0, 0, 0), 0);
        assert!(get("c.bin").is_none());
        assert_eq!(get("f.bin").as_deref(), Some(&b"one"[..]));

        put("new.bin", b"two");
        let f: unsafe extern "system" fn(*const u16, *const u16, *const u16, u32, usize, usize) -> i32 = core::mem::transmute(hook("ReplaceFileW", ReplaceFileW as usize));
        assert_ne!(f(console("f.bin").as_ptr(), console("new.bin").as_ptr(), console("bak.bin").as_ptr(), 0, 0, 0), 0);
        assert_eq!(get("f.bin").as_deref(), Some(&b"two"[..]));
        assert_eq!(get("bak.bin").as_deref(), Some(&b"one"[..]));
        assert!(get("new.bin").is_none());

        put("new2.bin", b"three");
        assert_ne!(f(console("f.bin").as_ptr(), console("new2.bin").as_ptr(), core::ptr::null(), 0, 0, 0), 0);
        assert_eq!(get("f.bin").as_deref(), Some(&b"three"[..]));
    }
}
