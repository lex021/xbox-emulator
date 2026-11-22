use crate::com::*;
use crate::paths::*;
use core::ffi::c_void;
use std::sync::Mutex;

pub const FILE_ATTRIBUTE_READONLY: u32 = 1;
pub const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
pub const INVALID_FILE_ATTRIBUTES: u32 = u32::MAX;
pub const XTFMOVEFILE_DEFER: u32 = 0x8000_0000;
const ERROR_ALREADY_EXISTS: u16 = 0xb7;
const GENERIC_READ: u32 = 0x8000_0000;
const OPEN_EXISTING: u32 = 3;
const INVALID_HANDLE_VALUE: isize = -1;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Win32FindDataW {
    pub attributes: u32,
    pub creation: [u32; 2],
    pub access: [u32; 2],
    pub write: [u32; 2],
    pub size_high: u32,
    pub size_low: u32,
    pub reserved0: u32,
    pub reserved1: u32,
    pub file_name: [u16; 260],
    pub alternate: [u16; 14],
    pub file_type: u32,
    pub creator_type: u32,
    pub finder_flags: u16,
}

impl Default for Win32FindDataW {
    fn default() -> Self {
        unsafe { core::mem::zeroed() }
    }
}

#[link(name = "kernel32")]
extern "system" {
    fn GetFileAttributesW(name: *const u16) -> u32;
    fn SetFileAttributesW(name: *const u16, a: u32) -> i32;
    fn CreateDirectoryW(name: *const u16, sa: *mut c_void) -> i32;
    fn RemoveDirectoryW(name: *const u16) -> i32;
    fn DeleteFileW(name: *const u16) -> i32;
    fn CreateFileW(name: *const u16, access: u32, share: u32, sa: *mut c_void, disp: u32, flags: u32, tmpl: isize) -> isize;
    fn GetFileSizeEx(h: isize, size: *mut i64) -> i32;
    fn CloseHandle(h: isize) -> i32;
    fn FindFirstFileExW(name: *const u16, level: u32, data: *mut Win32FindDataW, op: u32, filter: *mut c_void, flags: u32) -> isize;
    fn FindClose(h: isize) -> i32;
    pub fn GetLastError() -> u32;
    pub fn SetLastError(e: u32);
}

fn last_hr() -> HRESULT {
    hr_from_win32(unsafe { GetLastError() })
}

fn host_or_set_error(console: &str) -> Option<HostPath> {
    match host_path(console) {
        Ok(h) => Some(h),
        Err(e) => {
            unsafe { SetLastError(e) };
            None
        }
    }
}

pub fn get_attributes_raw(console: &str) -> u32 {
    match host_or_set_error(console) {
        Some(h) => unsafe { GetFileAttributesW(h.wide.as_ptr()) },
        None => INVALID_FILE_ATTRIBUTES,
    }
}

pub fn get_file_attributes(console: &str) -> Result<u32, HRESULT> {
    let a = get_attributes_raw(console);
    if a == INVALID_FILE_ATTRIBUTES {
        Err(last_hr())
    } else {
        Ok(a)
    }
}

pub fn set_file_attributes(console: &str, a: u32) -> HRESULT {
    let ok = match host_or_set_error(console) {
        Some(h) => unsafe { SetFileAttributesW(h.wide.as_ptr(), a) },
        None => 0,
    };
    if ok == 0 {
        last_hr()
    } else {
        S_OK
    }
}

pub fn modify_file_attributes(console: &str, add: u32, remove: u32) -> HRESULT {
    if add & remove != 0 {
        return E_INVALIDARG;
    }
    match get_file_attributes(console) {
        Ok(a) => set_file_attributes(console, (a | add) & !remove),
        Err(e) => e,
    }
}

pub fn get_parent_directory(path: &str) -> Result<String, HRESULT> {
    let mut s: Vec<char> = full_path_name(path)?.chars().collect();
    while let Some('\\') = s.last() {
        s.pop();
    }
    let Some(p) = s.iter().rposition(|&c| c == '\\') else {
        return Err(HR_PATH_NOT_FOUND);
    };
    s.truncate(p);
    if s.len() >= 2 && s[1] == ':' {
        let up = s[0].to_ascii_uppercase();
        if up.is_ascii_uppercase() {
            s[0] = up;
            if s.len() == 2 {
                s.push('\\');
            }
        }
    }
    Ok(s.into_iter().collect())
}

pub fn create_directory(path: &str) -> HRESULT {
    let mut attempts = 0;
    loop {
        let a = get_attributes_raw(path);
        if a != INVALID_FILE_ATTRIBUTES {
            return if a & FILE_ATTRIBUTE_DIRECTORY != 0 { S_OK } else { E_ACCESSDENIED };
        }
        if unsafe { GetLastError() } == ERROR_PATH_NOT_FOUND && !path.starts_with("\\\\") {
            let first = path.chars().next().unwrap_or('\0');
            let root = wide(&format!("{first}:\\"));
            if unsafe { console_sandbox::fs::get_drive_type_w(root.as_ptr()) } <= 1 {
                return HR_INVALID_DRIVE;
            }
        }
        let hr = match get_parent_directory(path) {
            Err(e) => e,
            Ok(parent) => {
                let mut hr = S_OK;
                if !parent.is_empty() {
                    hr = create_directory(&parent);
                }
                if hr >= 0 {
                    let ok = match host_or_set_error(path) {
                        Some(h) => unsafe { CreateDirectoryW(h.wide.as_ptr(), core::ptr::null_mut()) },
                        None => 0,
                    };
                    if ok != 0 {
                        return hr;
                    }
                    hr = last_hr();
                    if hr >= 0 {
                        return hr;
                    }
                }
                hr
            }
        };
        if hr as u16 != ERROR_ALREADY_EXISTS {
            return hr;
        }
        attempts += 1;
        if attempts >= 2 {
            return hr;
        }
    }
}

static PENDING: Mutex<()> = Mutex::new(());

pub fn pending_moves_file() -> std::path::PathBuf {
    console_sandbox::root().join("pending_file_rename_operations.txt")
}

pub fn move_file_deferred(src: &str, dst: Option<&str>) -> HRESULT {
    let _g = PENDING.lock().unwrap_or_else(|e| e.into_inner());
    let src_full = match full_path_name(src) {
        Ok(s) => s,
        Err(e) => return e,
    };
    if let Err(e) = host_path(&src_full) {
        return hr_from_win32(e);
    }
    let dst_full = match dst {
        Some(d) => match full_path_name(d) {
            Ok(s) => s,
            Err(e) => return e,
        },
        None => String::new(),
    };
    let f = pending_moves_file();
    if let Some(p) = f.parent() {
        let _ = std::fs::create_dir_all(p);
    }
    let mut text = std::fs::read_to_string(&f).unwrap_or_default();
    text.push_str(&format!("{src_full}\t{dst_full}\n"));
    match std::fs::write(&f, text) {
        Ok(_) => S_OK,
        Err(e) => hr_from_last_error_or_fail(e.raw_os_error().unwrap_or(0) as u32),
    }
}

pub fn apply_pending_moves() -> usize {
    let _g = PENDING.lock().unwrap_or_else(|e| e.into_inner());
    let f = pending_moves_file();
    let Ok(text) = std::fs::read_to_string(&f) else { return 0 };
    let _ = std::fs::remove_file(&f);
    let mut n = 0;
    for line in text.lines() {
        let Some((src, dst)) = line.split_once('\t') else { continue };
        let Ok(s) = host_path(src) else { continue };
        let ok = if dst.is_empty() {
            if s.path.is_dir() {
                std::fs::remove_dir(&s.path).is_ok()
            } else {
                std::fs::remove_file(&s.path).is_ok()
            }
        } else {
            match host_path(dst) {
                Ok(d) => {
                    if d.path.is_file() {
                        let _ = std::fs::remove_file(&d.path);
                    }
                    std::fs::rename(&s.path, &d.path).is_ok()
                }
                Err(_) => false,
            }
        };
        if ok {
            n += 1;
        }
    }
    n
}

pub fn move_file(src: &str, dst: Option<&str>, flags: u32) -> HRESULT {
    if flags & XTFMOVEFILE_DEFER != 0 {
        return move_file_deferred(src, dst);
    }
    let Some(s) = host_or_set_error(src) else { return hr_from_last_error_or_fail(unsafe { GetLastError() }) };
    let d = match dst {
        Some(d) => match host_or_set_error(d) {
            Some(h) => Some(h),
            None => return hr_from_last_error_or_fail(unsafe { GetLastError() }),
        },
        None => None,
    };
    let mode = (!flags & 1) | 2;
    #[link(name = "kernel32")]
    extern "system" {
        fn MoveFileExW(a: *const u16, b: *const u16, f: u32) -> i32;
    }
    let ok = unsafe { MoveFileExW(s.wide.as_ptr(), d.as_ref().map_or(core::ptr::null(), |h| h.wide.as_ptr()), mode) };
    if ok != 0 {
        S_OK
    } else {
        hr_from_last_error_or_fail(unsafe { GetLastError() })
    }
}

pub fn delete_files(path: &str, flags: u32) -> HRESULT {
    let orig = match get_file_attributes(path) {
        Ok(a) => a,
        Err(e) => return e,
    };
    let mut hr;
    'work: {
        if orig & FILE_ATTRIBUTE_READONLY != 0 {
            hr = set_file_attributes(path, orig & !FILE_ATTRIBUTE_READONLY);
            if hr < 0 {
                break 'work;
            }
        }
        if (flags as i32) < 0 {
            hr = move_file(path, None, XTFMOVEFILE_DEFER);
        } else {
            let ok = match host_or_set_error(path) {
                Some(h) => unsafe { DeleteFileW(h.wide.as_ptr()) },
                None => 0,
            };
            if ok != 0 {
                return S_OK;
            }
            hr = last_hr();
        }
        if hr >= 0 {
            return hr;
        }
    }
    if orig != INVALID_FILE_ATTRIBUTES && orig & FILE_ATTRIBUTE_READONLY != 0 {
        set_file_attributes(path, orig);
    }
    hr
}

pub fn remove_directory(path: &str, flags: u32) -> HRESULT {
    if (flags as i32) < 0 {
        return move_file(path, None, XTFMOVEFILE_DEFER);
    }
    let ok = match host_or_set_error(path) {
        Some(h) => unsafe { RemoveDirectoryW(h.wide.as_ptr()) },
        None => 0,
    };
    if ok != 0 {
        S_OK
    } else {
        hr_from_last_error_or_fail(unsafe { GetLastError() })
    }
}

pub fn get_file_size(path: &str) -> Result<u64, HRESULT> {
    let h = match host_or_set_error(path) {
        Some(p) => unsafe { CreateFileW(p.wide.as_ptr(), GENERIC_READ, 3, core::ptr::null_mut(), OPEN_EXISTING, 0, 0) },
        None => INVALID_HANDLE_VALUE,
    };
    if h == INVALID_HANDLE_VALUE {
        return Err(last_hr());
    }
    let mut size = 0i64;
    let ok = unsafe { GetFileSizeEx(h, &mut size) };
    let r = if ok == 0 { Err(last_hr()) } else { Ok(size as u64) };
    unsafe { CloseHandle(h) };
    r
}

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct FileInfo {
    pub attributes: u32,
    pub creation: u64,
    pub access: u64,
    pub write: u64,
    pub size: u64,
}

pub fn find_first(path: &str, data: &mut Win32FindDataW) -> isize {
    match host_or_set_error(path) {
        Some(h) => unsafe { FindFirstFileExW(h.wide.as_ptr(), 1, data, 0, core::ptr::null_mut(), 0) },
        None => INVALID_HANDLE_VALUE,
    }
}

pub fn find_first_search(pattern: &str, data: &mut Win32FindDataW) -> isize {
    match crate::paths::host_search_path(pattern) {
        Ok(h) => unsafe { FindFirstFileExW(h.wide.as_ptr(), 1, data, 0, core::ptr::null_mut(), 0) },
        Err(e) => {
            unsafe { SetLastError(e) };
            INVALID_HANDLE_VALUE
        }
    }
}

pub fn find_close(h: isize) {
    unsafe { FindClose(h) };
}

pub fn get_file_info(path: &str) -> Result<FileInfo, HRESULT> {
    let mut d = Win32FindDataW::default();
    let h = find_first(path, &mut d);
    if h == INVALID_HANDLE_VALUE {
        return Err(last_hr());
    }
    let ft = |v: [u32; 2]| ((v[1] as u64) << 32) | v[0] as u64;
    let info = FileInfo {
        attributes: d.attributes,
        creation: ft(d.creation),
        access: ft(d.access),
        write: ft(d.write),
        size: ((d.size_high as u64) << 32) | d.size_low as u64,
    };
    find_close(h);
    Ok(info)
}
