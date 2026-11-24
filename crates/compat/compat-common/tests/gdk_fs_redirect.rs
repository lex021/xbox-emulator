use std::sync::OnceLock;

extern "system" {
    fn LoadLibraryW(n: *const u16) -> usize;
    fn GetProcAddress(m: usize, n: *const u8) -> usize;
}

fn real(name: &str) -> usize {
    real_in("kernel32.dll", name)
}

fn real_in(dll: &str, name: &str) -> usize {
    let k: Vec<u16> = dll.encode_utf16().chain([0]).collect();
    let n: Vec<u8> = name.bytes().chain([0]).collect();
    let f = unsafe { GetProcAddress(LoadLibraryW(k.as_ptr()), n.as_ptr()) };
    assert_ne!(f, 0, "{name}");
    f
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

fn console_a(name: &str) -> Vec<u8> {
    format!("{PREFIX}{name}").bytes().chain([0]).collect()
}

fn hook(name: &str, real: usize) -> usize {
    compat_common::gdk::xcrd_paths::register(resolve as *const () as usize);
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
        let f: unsafe extern "system" fn(*const u16, *const u16, i32) -> i32 = core::mem::transmute(hook("CopyFileW", real("CopyFileW")));
        assert_ne!(f(console("a.bin").as_ptr(), console("b.bin").as_ptr(), 1), 0);
        assert_eq!(get("b.bin").as_deref(), Some(&b"one"[..]));

        let f: unsafe extern "system" fn(*const u16, *const u16, usize, usize, usize, u32) -> i32 = core::mem::transmute(hook("CopyFileExW", real("CopyFileExW")));
        assert_ne!(f(console("a.bin").as_ptr(), console("c.bin").as_ptr(), 0, 0, 0, 0), 0);
        assert_eq!(get("c.bin").as_deref(), Some(&b"one"[..]));

        let f: unsafe extern "system" fn(*const u16, *const u16, usize) -> i32 = core::mem::transmute(hook("CopyFile2", real("CopyFile2")));
        assert_eq!(f(console("a.bin").as_ptr(), console("d.bin").as_ptr(), 0), 0);
        assert_eq!(get("d.bin").as_deref(), Some(&b"one"[..]));

        let f: unsafe extern "system" fn(*const u16, *const u16, u32) -> i32 = core::mem::transmute(hook("MoveFileExW", real("MoveFileExW")));
        assert_ne!(f(console("b.bin").as_ptr(), console("e.bin").as_ptr(), 0), 0);
        assert!(get("b.bin").is_none());
        assert_eq!(get("e.bin").as_deref(), Some(&b"one"[..]));

        let f: unsafe extern "system" fn(*const u16, *const u16, usize, usize, u32) -> i32 = core::mem::transmute(hook("MoveFileWithProgressW", real("MoveFileWithProgressW")));
        assert_ne!(f(console("c.bin").as_ptr(), console("f.bin").as_ptr(), 0, 0, 0), 0);
        assert!(get("c.bin").is_none());
        assert_eq!(get("f.bin").as_deref(), Some(&b"one"[..]));

        put("new.bin", b"two");
        let f: unsafe extern "system" fn(*const u16, *const u16, *const u16, u32, usize, usize) -> i32 = core::mem::transmute(hook("ReplaceFileW", real("ReplaceFileW")));
        assert_ne!(f(console("f.bin").as_ptr(), console("new.bin").as_ptr(), console("bak.bin").as_ptr(), 0, 0, 0), 0);
        assert_eq!(get("f.bin").as_deref(), Some(&b"two"[..]));
        assert_eq!(get("bak.bin").as_deref(), Some(&b"one"[..]));
        assert!(get("new.bin").is_none());

        put("new2.bin", b"three");
        assert_ne!(f(console("f.bin").as_ptr(), console("new2.bin").as_ptr(), core::ptr::null(), 0, 0, 0), 0);
        assert_eq!(get("f.bin").as_deref(), Some(&b"three"[..]));
    }
}

#[test]
fn ansi_and_crt_path_calls_follow_the_console_path_too() {
    unsafe {
        type A1 = unsafe extern "system" fn(*const u8) -> i32;
        type A2 = unsafe extern "system" fn(*const u8, usize) -> i32;
        let f: unsafe extern "system" fn(*const u8, usize) -> i32 = core::mem::transmute(hook("CreateDirectoryA", real("CreateDirectoryA")));
        assert_ne!(f(console_a("da").as_ptr(), 0), 0);
        assert!(root().join("da").is_dir());

        let f: unsafe extern "system" fn(*const u8, u32, u32, usize, u32, u32, usize) -> isize = core::mem::transmute(hook("CreateFileA", real("CreateFileA")));
        let h = f(console_a("da\\fa.bin").as_ptr(), 0x4000_0000, 0, 0, 2, 0x80, 0);
        assert_ne!(h, -1);
        let close: unsafe extern "system" fn(isize) -> i32 = core::mem::transmute(real("CloseHandle"));
        close(h);
        assert!(root().join("da").join("fa.bin").is_file());

        let f: unsafe extern "system" fn(*const u8, u32) -> i32 = core::mem::transmute(hook("SetFileAttributesA", real("SetFileAttributesA")));
        assert_ne!(f(console_a("da\\fa.bin").as_ptr(), 0x80), 0);

        let mut data = [0u8; 600];
        let f: unsafe extern "system" fn(*const u8, u32, *mut u8, u32, usize, u32) -> isize = core::mem::transmute(hook("FindFirstFileExA", real("FindFirstFileExA")));
        let fh = f(console_a("da\\*.bin").as_ptr(), 0, data.as_mut_ptr(), 0, 0, 0);
        assert_ne!(fh, -1);
        let fclose: unsafe extern "system" fn(isize) -> i32 = core::mem::transmute(real("FindClose"));
        fclose(fh);
        assert_eq!(&data[44..50], b"fa.bin");

        let f: A1 = core::mem::transmute(hook("DeleteFileA", real("DeleteFileA")));
        assert_ne!(f(console_a("da\\fa.bin").as_ptr()), 0);
        assert!(!root().join("da").join("fa.bin").exists());

        let crt = |n: &str| real_in("ucrtbase.dll", n);
        let f: A1 = core::mem::transmute(hook("_mkdir", crt("_mkdir")));
        assert_eq!(f(console_a("db").as_ptr()), 0);
        assert!(root().join("db").is_dir());

        let f: unsafe extern "system" fn(*const u8, i32, i32) -> i32 = core::mem::transmute(hook("_open", crt("_open")));
        let fd = f(console_a("db\\o.bin").as_ptr(), 0x0100 | 0x0001 | 0x8000, 0x0180);
        assert!(fd >= 0);
        let cl: unsafe extern "system" fn(i32) -> i32 = core::mem::transmute(crt("_close"));
        cl(fd);
        assert!(root().join("db").join("o.bin").is_file());

        let f: A2 = core::mem::transmute(hook("_access", crt("_access")));
        assert_eq!(f(console_a("db\\o.bin").as_ptr(), 0), 0);

        let mut st = [0u8; 64];
        let f: A2 = core::mem::transmute(hook("_stat64", crt("_stat64")));
        assert_eq!(f(console_a("db\\o.bin").as_ptr(), st.as_mut_ptr() as usize), 0);

        let f: unsafe extern "system" fn(*const u8, *const u8) -> i32 = core::mem::transmute(hook("rename", crt("rename")));
        assert_eq!(f(console_a("db\\o.bin").as_ptr(), console_a("db\\r.bin").as_ptr()), 0);
        assert!(root().join("db").join("r.bin").is_file());

        let f: unsafe extern "system" fn(*const u16, *const u16) -> i32 = core::mem::transmute(hook("_wrename", crt("_wrename")));
        assert_eq!(f(console("db\\r.bin").as_ptr(), console("db\\s.bin").as_ptr()), 0);
        assert!(root().join("db").join("s.bin").is_file());

        let f: unsafe extern "system" fn(*const u16, usize) -> i32 = core::mem::transmute(hook("_wstat64", crt("_wstat64")));
        assert_eq!(f(console("db\\s.bin").as_ptr(), st.as_mut_ptr() as usize), 0);

        let mut fp = 0usize;
        let mode = w("rb");
        let f: unsafe extern "system" fn(*mut usize, *const u16, *const u16) -> i32 = core::mem::transmute(hook("_wfopen_s", crt("_wfopen_s")));
        assert_eq!(f(&mut fp, console("db\\s.bin").as_ptr(), mode.as_ptr()), 0);
        let fc: unsafe extern "system" fn(usize) -> i32 = core::mem::transmute(crt("fclose"));
        fc(fp);

        let f: unsafe extern "system" fn(*const u16) -> i32 = core::mem::transmute(hook("_wremove", crt("_wremove")));
        assert_eq!(f(console("db\\s.bin").as_ptr()), 0);
        assert!(!root().join("db").join("s.bin").exists());

        let f: unsafe extern "system" fn(*const u16, i32, i32) -> i32 = core::mem::transmute(hook("_wopen", crt("_wopen")));
        let fd = f(console("db\\wo.bin").as_ptr(), 0x0100 | 0x0001 | 0x8000, 0x0180);
        assert!(fd >= 0);
        cl(fd);
        assert!(root().join("db").join("wo.bin").is_file());

        let errno: unsafe extern "system" fn() -> *mut i32 = core::mem::transmute(crt("_errno"));
        let gle: unsafe extern "system" fn() -> u32 = core::mem::transmute(real("GetLastError"));
        let sle: unsafe extern "system" fn(u32) = core::mem::transmute(real("SetLastError"));
        let pc = root().join("db").join("wo.bin").to_string_lossy().into_owned();
        let pc_a: Vec<u8> = pc.bytes().chain([0]).collect();
        let missing_a: Vec<u8> = root().join("nope").join("x.bin").to_string_lossy().bytes().chain([0]).collect();
        let rel_a = b"tombox_no_such_relative_file.bin\0";
        let empty_a = b"\0";

        let f: A2 = core::mem::transmute(hook("_access", crt("_access")));
        assert_eq!(f(pc_a.as_ptr(), 0), 0);
        for p in [&missing_a[..], &rel_a[..], &empty_a[..]] {
            *errno() = 0;
            assert_eq!(f(p.as_ptr(), 0), -1, "{p:?}");
            assert_eq!(*errno(), 2, "{p:?}");
        }
        let f: A2 = core::mem::transmute(hook("_stat64", crt("_stat64")));
        assert_eq!(f(pc_a.as_ptr(), st.as_mut_ptr() as usize), 0);
        for p in [&missing_a[..], &rel_a[..], &empty_a[..]] {
            *errno() = 0;
            assert_eq!(f(p.as_ptr(), st.as_mut_ptr() as usize), -1, "{p:?}");
            assert_eq!(*errno(), 2, "{p:?}");
        }
        let f: unsafe extern "system" fn(*const u8, u32, u32, usize, u32, u32, usize) -> isize = core::mem::transmute(hook("CreateFileA", real("CreateFileA")));
        let h = f(pc_a.as_ptr(), 0x8000_0000, 1, 0, 3, 0x80, 0);
        assert_ne!(h, -1);
        close(h);
        for (p, want) in [(&missing_a[..], 3u32), (&rel_a[..], 2), (&empty_a[..], 3)] {
            sle(0);
            assert_eq!(f(p.as_ptr(), 0x8000_0000, 1, 0, 3, 0x80, 0), -1, "{p:?}");
            assert_eq!(gle(), want, "{p:?}");
        }

        static SEEN: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        unsafe extern "system" fn spy(p: *const u8, _m: usize) -> i32 {
            SEEN.store(p as usize, std::sync::atomic::Ordering::SeqCst);
            -7
        }
        let f: A2 = core::mem::transmute(hook("_access", spy as *const () as usize));
        for p in [&pc_a[..], &missing_a[..], &rel_a[..], &empty_a[..]] {
            assert_eq!(f(p.as_ptr(), 0), -7);
            assert_eq!(SEEN.load(std::sync::atomic::Ordering::SeqCst), p.as_ptr() as usize, "{p:?}");
        }
        hook("_access", crt("_access"));
    }
}
