use compat_common::record;
use core::ffi::c_void;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::collections::HashMap;
use std::sync::Mutex;

static STATE: Mutex<Option<HashMap<usize, i32>>> = Mutex::new(None);
static SEEN: AtomicUsize = AtomicUsize::new(0);
static GONE: AtomicBool = AtomicBool::new(false);

pub fn on_close(list: *mut c_void, hr: i32) {
    STATE.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).insert(list as usize, hr);
}

pub fn on_reset(list: *mut c_void) {
    STATE.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).insert(list as usize, 1);
}

fn close_state(list: usize) -> String {
    match STATE.lock().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(|m| m.get(&list).copied()) {
        None => "no title Reset/Close seen".into(),
        Some(1) => "open (Reset, no Close)".into(),
        Some(hr) if hr < 0 => format!("Close failed {hr:#010x}"),
        Some(_) => "closed".into(),
    }
}

unsafe fn vslot(obj: *mut c_void, i: usize) -> usize {
    unsafe { *(*(obj as *const *const usize)).add(i) }
}

pub unsafe fn removed_reason(obj: *mut c_void) -> Option<i32> {
    if obj.is_null() {
        return None;
    }
    let iid = crate::device_wrap::Guid { d1: 0x189819f1, d2: 0x1db6, d3: 0x4b57, d4: [0xbe, 0x54, 0x18, 0x21, 0x33, 0x9b, 0x85, 0xf7] };
    let get_device: unsafe extern "system" fn(*mut c_void, *const crate::device_wrap::Guid, *mut *mut c_void) -> i32 = unsafe { core::mem::transmute(vslot(obj, 7)) };
    let mut dev: *mut c_void = core::ptr::null_mut();
    if unsafe { get_device(obj, &iid, &mut dev) } < 0 || dev.is_null() {
        return None;
    }
    let reason: unsafe extern "system" fn(*mut c_void) -> i32 = unsafe { core::mem::transmute(vslot(dev, 37)) };
    let hr = unsafe { reason(dev) };
    let release: unsafe extern "system" fn(*mut c_void) -> u32 = unsafe { core::mem::transmute(vslot(dev, 2)) };
    unsafe { release(dev) };
    Some(hr)
}

pub unsafe fn queue_desc(queue: *mut c_void, get_desc: usize) -> [u32; 4] {
    let mut d = [0u32; 4];
    if get_desc != 0 {
        let f: unsafe extern "system" fn(*mut c_void, *mut [u32; 4]) -> *mut [u32; 4] = unsafe { core::mem::transmute(get_desc) };
        unsafe { f(queue, &mut d) };
    }
    d
}

pub unsafe fn check(tag: &str, queue: *mut c_void, get_desc: usize, title: &[*mut c_void], real: &[*mut c_void], before: Option<i32>) {
    if GONE.load(Ordering::Relaxed) {
        return;
    }
    let after = unsafe { removed_reason(queue) };
    let n = SEEN.fetch_add(1, Ordering::Relaxed);
    let removed_here = before == Some(0) && after.is_some_and(|h| h != 0);
    if n >= 6 && !removed_here && after == Some(0) {
        return;
    }
    let d = unsafe { queue_desc(queue, get_desc) };
    let lists: Vec<String> = real
        .iter()
        .map(|&l| format!("{l:p} type={} {}", unsafe { crate::fill::list_type(l) }, close_state(l as usize)))
        .collect();
    record("d3d12_xs", "ecl.probe", &format!(
        "{tag} #{n} queue={queue:p} type={} prio={} flags={:#x} node={:#x} title lists={title:?} real lists=[{}] removed before={before:?} after={after:?}",
        d[0], d[1], d[2], d[3], lists.join("; ")
    ));
    if after.is_some_and(|h| h != 0) {
        GONE.store(true, Ordering::Relaxed);
        record("d3d12_xs", "ecl.probe", &format!("device removed {} this submission", if removed_here { "by" } else { "before" }));
    }
}

pub unsafe fn after_device_call(dev: *mut c_void, tag: &str, detail: impl FnOnce() -> String) {
    if GONE.load(Ordering::Relaxed) || dev.is_null() {
        return;
    }
    let reason: unsafe extern "system" fn(*mut c_void) -> i32 = unsafe { core::mem::transmute(vslot(dev, 37)) };
    let hr = unsafe { reason(dev) };
    if hr != 0 && !GONE.swap(true, Ordering::Relaxed) {
        record("d3d12_xs", "ecl.probe", &format!("device removed by {tag} (GetDeviceRemovedReason={hr:#010x}): {}", detail()));
    }
}
