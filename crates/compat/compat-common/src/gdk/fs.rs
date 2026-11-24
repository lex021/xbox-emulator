use super::{cstr, log, setting, wstr};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;

extern "system" {
    fn GetLastError() -> u32;
    fn SetLastError(e: u32);
}

#[derive(Clone, Copy, PartialEq)]
enum K {
    W,
    A,
}

pub struct Drive {
    pub letter: char,
    pub root: PathBuf,
}

pub fn game_drive() -> Option<&'static Drive> {
    static D: OnceLock<Option<Drive>> = OnceLock::new();
    D.get_or_init(|| {
        if setting("game_drive").as_deref() == Some("off") {
            return None;
        }
        let letter = setting("game_drive").and_then(|s| s.chars().next()).unwrap_or('G').to_ascii_uppercase();
        let root = super::title().dir.clone()?;
        let r = root.to_string_lossy().to_ascii_uppercase();
        if r.trim_end_matches('\\') == format!("{letter}:") {
            return None;
        }
        Some(Drive { letter, root })
    })
    .as_ref()
}

fn trace_on() -> bool {
    use std::sync::atomic::{AtomicU8, Ordering};
    static T: AtomicU8 = AtomicU8::new(0);
    match T.load(Ordering::Acquire) {
        2 => return false,
        3 => return true,
        1 => return false,
        _ => {}
    }
    if T.compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire).is_err() {
        return T.load(Ordering::Acquire) == 3;
    }
    let on = setting("gdk_file_trace").as_deref() == Some("1");
    T.store(if on { 3 } else { 2 }, Ordering::Release);
    on
}

pub fn translate(p: &str) -> Option<String> {
    if super::xcrd_paths::is_xcrd_path(p) {
        return super::xcrd_paths::resolve(p);
    }
    let d = game_drive()?;
    let (prefix, rest) = if let Some(r) = p.strip_prefix("\\\\?\\") { ("\\\\?\\", r) } else { ("", p) };
    let b = rest.as_bytes();
    if b.len() < 2 || b[1] != b':' || (b[0] as char).to_ascii_uppercase() != d.letter {
        return None;
    }
    let tail = rest[2..].trim_start_matches(['\\', '/']);
    let mut root = d.root.to_string_lossy().into_owned();
    if !root.ends_with('\\') {
        root.push('\\');
    }
    Some(format!("{prefix}{root}{}", tail.replace('/', "\\")))
}

enum Buf {
    W(Vec<u16>),
    A(Vec<u8>),
}

unsafe fn map_arg(k: K, p: usize) -> Option<(Buf, String, String)> {
    if p < 0x10000 {
        return None;
    }
    let s = match k {
        K::W => wstr(p as *const u16)?,
        K::A => cstr(p as *const u8)?,
    };
    match translate(&s) {
        Some(t) => {
            let b = match k {
                K::W => Buf::W(t.encode_utf16().chain(Some(0)).collect()),
                K::A => Buf::A(t.bytes().chain(Some(0)).collect()),
            };
            Some((b, s, t))
        }
        None => {
            if trace_on() {
                Some((Buf::A(Vec::new()), s.clone(), s))
            } else {
                None
            }
        }
    }
}

unsafe fn call(f: usize, a: &[usize]) -> usize {
    match a.len() {
        1 => core::mem::transmute::<usize, unsafe extern "system" fn(usize) -> usize>(f)(a[0]),
        2 => core::mem::transmute::<usize, unsafe extern "system" fn(usize, usize) -> usize>(f)(a[0], a[1]),
        3 => core::mem::transmute::<usize, unsafe extern "system" fn(usize, usize, usize) -> usize>(f)(a[0], a[1], a[2]),
        4 => core::mem::transmute::<usize, unsafe extern "system" fn(usize, usize, usize, usize) -> usize>(f)(a[0], a[1], a[2], a[3]),
        5 => core::mem::transmute::<usize, unsafe extern "system" fn(usize, usize, usize, usize, usize) -> usize>(f)(a[0], a[1], a[2], a[3], a[4]),
        6 => core::mem::transmute::<usize, unsafe extern "system" fn(usize, usize, usize, usize, usize, usize) -> usize>(f)(a[0], a[1], a[2], a[3], a[4], a[5]),
        7 => core::mem::transmute::<usize, unsafe extern "system" fn(usize, usize, usize, usize, usize, usize, usize) -> usize>(f)(a[0], a[1], a[2], a[3], a[4], a[5], a[6]),
        _ => 0,
    }
}

unsafe fn run(slot: usize, mut a: Vec<usize>) -> usize {
    let (name, k, paths, _) = TABLE[slot];
    if let Some(h) = crate::os_host::routes_rt::fs_load_override(name, &a) {
        return h;
    }
    let f = ORIG[slot].load(Ordering::Acquire);
    if f == 0 {
        SetLastError(127);
        return 0;
    }
    let orig_args = a.clone();
    let mut keep = Vec::new();
    let mut notes = Vec::new();
    for &i in paths {
        if let Some((b, from, to)) = map_arg(k, a[i]) {
            match &b {
                Buf::W(v) => a[i] = v.as_ptr() as usize,
                Buf::A(v) if !v.is_empty() => a[i] = v.as_ptr() as usize,
                _ => {}
            }
            notes.push(if from == to { format!("{from:?}") } else { format!("{from:?} => {to:?}") });
            keep.push(b);
        }
    }
    let r = call(f, &a);
    crate::os_host::routes_rt::fs_load_record(name, &orig_args, r);
    if !notes.is_empty() && (trace_on() || first_redirect()) {
        let e = GetLastError();
        log("game-drive", &format!("{name}({}) -> {r:#x} err={e}", notes.join(", ")));
        SetLastError(e);
    }
    drop(keep);
    r
}

fn first_redirect() -> bool {
    static N: AtomicUsize = AtomicUsize::new(0);
    N.fetch_add(1, Ordering::Relaxed) < 40
}

type Row = (&'static str, K, &'static [usize], usize);

const TABLE: &[Row] = &[
    ("CreateFileW", K::W, &[0], 7),
    ("FindFirstFileExW", K::W, &[0], 6),
    ("FindFirstFileW", K::W, &[0], 2),
    ("GetFileAttributesW", K::W, &[0], 1),
    ("GetFileAttributesExW", K::W, &[0], 3),
    ("CreateDirectoryW", K::W, &[0], 2),
    ("SetFileAttributesW", K::W, &[0], 2),
    ("RemoveDirectoryW", K::W, &[0], 1),
    ("DeleteFileW", K::W, &[0], 1),
    ("GetDiskFreeSpaceExW", K::W, &[0], 4),
    ("LoadLibraryW", K::W, &[0], 1),
    ("LoadLibraryExW", K::W, &[0], 3),
    ("_wfopen", K::W, &[0], 2),
    ("CopyFileW", K::W, &[0, 1], 3),
    ("MoveFileW", K::W, &[0, 1], 2),
    ("MoveFileExW", K::W, &[0, 1], 3),
    ("GetFileAttributesA", K::A, &[0], 1),
    ("GetFileAttributesExA", K::A, &[0], 3),
    ("LoadLibraryA", K::A, &[0], 1),
    ("LoadLibraryExA", K::A, &[0], 3),
    ("fopen", K::A, &[0], 2),
    ("fopen_s", K::A, &[1], 3),
    ("_unlink", K::A, &[0], 1),
    ("remove", K::A, &[0], 1),
    ("CreateFile2", K::W, &[0], 5),
    ("CopyFileExW", K::W, &[0, 1], 6),
    ("MoveFileWithProgressW", K::W, &[0, 1], 5),
    ("ReplaceFileW", K::W, &[0, 1, 2], 6),
    ("CopyFile2", K::W, &[0, 1], 3),
    ("CreateFileA", K::A, &[0], 7),
    ("DeleteFileA", K::A, &[0], 1),
    ("CreateDirectoryA", K::A, &[0], 2),
    ("FindFirstFileExA", K::A, &[0], 6),
    ("SetFileAttributesA", K::A, &[0], 2),
    ("_mkdir", K::A, &[0], 1),
    ("_open", K::A, &[0], 3),
    ("_access", K::A, &[0], 2),
    ("_stat64", K::A, &[0], 2),
    ("rename", K::A, &[0, 1], 2),
    ("_wopen", K::W, &[0], 3),
    ("_wstat64", K::W, &[0], 2),
    ("_wfopen_s", K::W, &[1], 3),
    ("_wremove", K::W, &[0], 1),
    ("_wrename", K::W, &[0, 1], 2),
];

static ORIG: [AtomicUsize; 44] = [const { AtomicUsize::new(0) }; 44];

macro_rules! hooks {
    ($($i:tt $n:tt),*) => {
        fn hook_table() -> [usize; 44] {
            [$(hook_fn!($i $n)),*]
        }
    };
}

macro_rules! hook_fn {
    ($i:tt 1) => {{ unsafe extern "system" fn h(a: usize) -> usize { run($i, vec![a]) } h as usize }};
    ($i:tt 2) => {{ unsafe extern "system" fn h(a: usize, b: usize) -> usize { run($i, vec![a, b]) } h as usize }};
    ($i:tt 3) => {{ unsafe extern "system" fn h(a: usize, b: usize, c: usize) -> usize { run($i, vec![a, b, c]) } h as usize }};
    ($i:tt 4) => {{ unsafe extern "system" fn h(a: usize, b: usize, c: usize, d: usize) -> usize { run($i, vec![a, b, c, d]) } h as usize }};
    ($i:tt 5) => {{ unsafe extern "system" fn h(a: usize, b: usize, c: usize, d: usize, e: usize) -> usize { run($i, vec![a, b, c, d, e]) } h as usize }};
    ($i:tt 6) => {{ unsafe extern "system" fn h(a: usize, b: usize, c: usize, d: usize, e: usize, f: usize) -> usize { run($i, vec![a, b, c, d, e, f]) } h as usize }};
    ($i:tt 7) => {{ unsafe extern "system" fn h(a: usize, b: usize, c: usize, d: usize, e: usize, f: usize, g: usize) -> usize { run($i, vec![a, b, c, d, e, f, g]) } h as usize }};
}

hooks!(0 7, 1 6, 2 2, 3 1, 4 3, 5 2, 6 2, 7 1, 8 1, 9 4, 10 1, 11 3, 12 2, 13 3, 14 2, 15 3, 16 1, 17 3, 18 1, 19 3, 20 2, 21 3, 22 1, 23 1, 24 5, 25 6, 26 5, 27 6, 28 3, 29 7, 30 1, 31 2, 32 6, 33 2, 34 1, 35 3, 36 2, 37 2, 38 2, 39 3, 40 2, 41 3, 42 1, 43 2);

pub fn targets() -> Vec<(&'static str, usize, &'static AtomicUsize)> {
    if game_drive().is_none() && !trace_on() && !super::xcrd_paths::enabled() {
        return Vec::new();
    }
    let h = hook_table();
    TABLE.iter().enumerate().map(|(i, r)| (r.0, h[i], &ORIG[i])).collect()
}

pub fn loader_hooks() -> Vec<usize> {
    let h = hook_table();
    TABLE.iter().enumerate().filter(|(_, r)| r.0.starts_with("LoadLibrary")).map(|(i, _)| h[i]).collect()
}

pub fn describe() -> String {
    match game_drive() {
        Some(d) => format!("game drive {}: -> {}", d.letter, d.root.display()),
        None => "game drive mapping off".into(),
    }
}
