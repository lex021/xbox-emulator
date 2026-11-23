use crate::sys::*;
use std::sync::Mutex;

static TITLE_PIDS: Mutex<Vec<u32>> = Mutex::new(Vec::new());

pub fn register_title_process(pid: u32) {
    let mut v = TITLE_PIDS.lock().unwrap_or_else(|e| e.into_inner());
    if !v.contains(&pid) {
        v.push(pid);
    }
}

pub fn unregister_title_process(pid: u32) {
    TITLE_PIDS.lock().unwrap_or_else(|e| e.into_inner()).retain(|p| *p != pid);
}

pub fn is_title_process(pid: u32) -> bool {
    pid == unsafe { GetCurrentProcessId() } || TITLE_PIDS.lock().unwrap_or_else(|e| e.into_inner()).contains(&pid)
}

pub unsafe fn open_process(access: u32, pid: u32) -> HANDLE {
    if !is_title_process(pid) {
        SetLastError(ERROR_INVALID_PARAMETER);
        return core::ptr::null_mut();
    }
    OpenProcess(access, 0, pid)
}

pub unsafe fn debug_active_process(pid: u32) -> BOOL {
    if !is_title_process(pid) {
        SetLastError(ERROR_INVALID_PARAMETER);
        return 0;
    }
    DebugActiveProcess(pid)
}

pub unsafe fn debug_active_process_stop(pid: u32) -> BOOL {
    if !is_title_process(pid) {
        SetLastError(ERROR_INVALID_PARAMETER);
        return 0;
    }
    DebugActiveProcessStop(pid)
}

pub fn title_pids() -> Vec<u32> {
    let mut v = vec![unsafe { GetCurrentProcessId() }];
    for p in TITLE_PIDS.lock().unwrap_or_else(|e| e.into_inner()).iter() {
        if !v.contains(p) {
            v.push(*p);
        }
    }
    v
}
