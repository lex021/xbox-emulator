use core::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::Duration;

#[link(name = "kernel32")]
extern "system" {
    fn GetCurrentThreadId() -> u32;
    fn GetCurrentProcessId() -> u32;
}

#[link(name = "user32")]
extern "system" {
    fn EnumWindows(cb: unsafe extern "system" fn(*mut c_void, isize) -> i32, l: isize) -> i32;
    fn GetWindowThreadProcessId(h: *mut c_void, pid: *mut u32) -> u32;
    fn IsWindowVisible(h: *mut c_void) -> i32;
    fn IsIconic(h: *mut c_void) -> i32;
    fn GetWindow(h: *mut c_void, cmd: u32) -> *mut c_void;
}

pub const ERROR_SUCCESS: u32 = 0;
pub const ERROR_INVALID_PARAMETER: u32 = 87;
pub const ERROR_NOT_FOUND: u32 = 0x490;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    AppState = 0,
    Constrained = 1,
}

pub type ChangeRoutine = unsafe extern "system" fn(u8, *mut c_void);

struct Entry {
    id: u64,
    kind: Kind,
    routine: usize,
    context: usize,
    active: Mutex<u32>,
    idle: Condvar,
}

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

fn regs() -> &'static Mutex<Vec<Arc<Entry>>> {
    static R: OnceLock<Mutex<Vec<Arc<Entry>>>> = OnceLock::new();
    R.get_or_init(|| Mutex::new(Vec::new()))
}

fn tid() -> u32 {
    unsafe { GetCurrentThreadId() }
}

fn owner_entry(var: &str, own: usize) -> Option<usize> {
    if super::is_owner() {
        return None;
    }
    let p = std::env::var(var).ok().and_then(|s| usize::from_str_radix(s.trim_start_matches("0x"), 16).ok())?;
    (p != 0 && p != own).then_some(p)
}

pub unsafe extern "system" fn register_entry(kind: u32, routine: usize, context: usize, out: *mut u64) -> u32 {
    register_local(if kind == 0 { Kind::AppState } else { Kind::Constrained }, routine, context, out)
}

pub unsafe extern "system" fn unregister_entry(kind: u32, id: u64) -> u32 {
    unregister_local(if kind == 0 { Kind::AppState } else { Kind::Constrained }, id)
}

pub fn register(kind: Kind, routine: usize, context: usize, out: *mut u64) -> u32 {
    if let Some(p) = owner_entry(super::REGISTER_VAR, register_entry as *const () as usize) {
        let f: unsafe extern "system" fn(u32, usize, usize, *mut u64) -> u32 = unsafe { core::mem::transmute(p) };
        return unsafe { f(kind as u32, routine, context, out) };
    }
    register_local(kind, routine, context, out)
}

pub fn unregister(kind: Kind, id: u64) -> u32 {
    if let Some(p) = owner_entry(super::UNREGISTER_VAR, unregister_entry as *const () as usize) {
        let f: unsafe extern "system" fn(u32, u64) -> u32 = unsafe { core::mem::transmute(p) };
        return unsafe { f(kind as u32, id) };
    }
    unregister_local(kind, id)
}

fn register_local(kind: Kind, routine: usize, context: usize, out: *mut u64) -> u32 {
    if out.is_null() {
        return ERROR_INVALID_PARAMETER;
    }
    unsafe { core::ptr::write_unaligned(out, 0) };
    let id = NEXT_ID.fetch_add(1, Ordering::AcqRel) + 1;
    let e = Arc::new(Entry { id, kind, routine, context, active: Mutex::new(0), idle: Condvar::new() });
    regs().lock().unwrap_or_else(|e| e.into_inner()).push(e);
    unsafe { core::ptr::write_unaligned(out, id) };
    crate::record_raw("os_host", if kind == Kind::AppState { "RegisterAppStateChangeNotification" } else { "RegisterAppConstrainedChangeNotification" }, &format!("routine {routine:#x} context {context:#x} -> registration {id}"));
    start_watcher();
    ERROR_SUCCESS
}

fn unregister_local(kind: Kind, id: u64) -> u32 {
    let found = {
        let mut r = regs().lock().unwrap_or_else(|e| e.into_inner());
        match r.iter().position(|e| e.id == id && e.kind == kind) {
            Some(i) => Some(r.remove(i)),
            None => None,
        }
    };
    let Some(e) = found else {
        return ERROR_NOT_FOUND;
    };
    let me = tid();
    let mut a = e.active.lock().unwrap_or_else(|e| e.into_inner());
    while *a != 0 && *a != me {
        a = e.idle.wait(a).unwrap_or_else(|e| e.into_inner());
    }
    ERROR_SUCCESS
}

pub fn registered(kind: Kind) -> usize {
    regs().lock().unwrap_or_else(|e| e.into_inner()).iter().filter(|e| e.kind == kind).count()
}

fn invoke(e: &Entry, value: bool) {
    {
        *e.active.lock().unwrap_or_else(|e| e.into_inner()) = tid();
    }
    let f: ChangeRoutine = unsafe { core::mem::transmute(e.routine) };
    unsafe { f(value as u8, e.context as *mut c_void) };
    *e.active.lock().unwrap_or_else(|e| e.into_inner()) = 0;
    e.idle.notify_all();
}

pub fn dispatch(kind: Kind, value: bool) -> usize {
    let me = tid();
    let snap: Vec<Arc<Entry>> = {
        let r = regs().lock().unwrap_or_else(|e| e.into_inner());
        r.iter()
            .filter(|e| e.kind == kind)
            .map(|e| {
                *e.active.lock().unwrap_or_else(|e| e.into_inner()) = me;
                e.clone()
            })
            .collect()
    };
    match snap.len() {
        0 => {}
        1 => invoke(&snap[0], value),
        _ => std::thread::scope(|s| {
            for e in &snap {
                s.spawn(move || invoke(e, value));
            }
        }),
    }
    snap.len()
}

pub fn signal_local(kind: Kind, value: bool) -> usize {
    let (tx, rx) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        let _ = tx.send(dispatch(kind, value));
    });
    let n = if kind == Kind::AppState && value { super::quiesce::watch(&rx) } else { rx.recv().unwrap_or(0) };
    let _ = worker.join();
    let what = match (kind, value) {
        (Kind::AppState, true) => "suspend (quiesced=TRUE)",
        (Kind::AppState, false) => "resume (quiesced=FALSE)",
        (Kind::Constrained, true) => "constrained=TRUE",
        (Kind::Constrained, false) => "constrained=FALSE",
    };
    crate::record_raw("os_host", "plm", &format!("{what} delivered to {n} registration(s)"));
    n
}

pub unsafe extern "system" fn signal_entry(kind: u32, value: u32) -> u32 {
    let k = if kind == 0 { Kind::AppState } else { Kind::Constrained };
    signal_local(k, value != 0) as u32
}

pub fn signal(kind: Kind, value: bool) -> usize {
    if !super::is_owner() {
        if let Some(p) = std::env::var(super::SIGNAL_VAR).ok().and_then(|s| usize::from_str_radix(s.trim_start_matches("0x"), 16).ok()) {
            if p != 0 && p != signal_entry as *const () as usize {
                let f: unsafe extern "system" fn(u32, u32) -> u32 = unsafe { core::mem::transmute(p) };
                return unsafe { f(kind as u32, value as u32) } as usize;
            }
        }
    }
    signal_local(kind, value)
}

pub fn suspend() -> usize {
    signal(Kind::AppState, true)
}

pub fn resume() -> usize {
    signal(Kind::AppState, false)
}

pub fn set_constrained(on: bool) -> usize {
    signal(Kind::Constrained, on)
}

pub unsafe extern "system" fn register_app_state_change_notification(routine: usize, context: usize, out: *mut u64) -> u32 {
    register(Kind::AppState, routine, context, out)
}

pub unsafe extern "system" fn unregister_app_state_change_notification(id: u64) -> u32 {
    unregister(Kind::AppState, id)
}

pub unsafe extern "system" fn register_app_constrained_change_notification(routine: usize, context: usize, out: *mut u64) -> u32 {
    register(Kind::Constrained, routine, context, out)
}

pub unsafe extern "system" fn unregister_app_constrained_change_notification(id: u64) -> u32 {
    unregister(Kind::Constrained, id)
}

pub fn command_path() -> std::path::PathBuf {
    let dir = std::env::var("TOMBOX_PLM_DIR").map(std::path::PathBuf::from).unwrap_or_else(|_| std::env::temp_dir().join("tombox_plm"));
    dir.join(format!("{}.cmd", unsafe { GetCurrentProcessId() }))
}

pub fn parse_command(line: &str) -> Option<(Kind, bool)> {
    match line.trim().to_ascii_lowercase().as_str() {
        "suspend" | "quiesce" => Some((Kind::AppState, true)),
        "resume" => Some((Kind::AppState, false)),
        "constrain" | "constrained" => Some((Kind::Constrained, true)),
        "unconstrain" | "unconstrained" | "full" => Some((Kind::Constrained, false)),
        _ => None,
    }
}

unsafe fn title_minimized() -> Option<bool> {
    unsafe extern "system" fn cb(h: *mut c_void, l: isize) -> i32 {
        let st = &mut *(l as *mut (u32, u32, u32));
        let mut pid = 0u32;
        GetWindowThreadProcessId(h, &mut pid);
        if pid == st.0 && IsWindowVisible(h) != 0 && GetWindow(h, 4).is_null() {
            st.1 += 1;
            if IsIconic(h) != 0 {
                st.2 += 1;
            }
        }
        1
    }
    let mut st = (GetCurrentProcessId(), 0u32, 0u32);
    EnumWindows(cb, &mut st as *mut _ as isize);
    if st.1 == 0 {
        None
    } else {
        Some(st.2 == st.1)
    }
}

static WATCHING: AtomicBool = AtomicBool::new(false);

fn start_watcher() {
    if std::env::var("TOMBOX_PLM_WATCH").ok().as_deref() == Some("0") {
        return;
    }
    if WATCHING.swap(true, Ordering::AcqRel) {
        return;
    }
    let _ = std::thread::Builder::new().name("tombox-plm".into()).spawn(|| {
        let path = command_path();
        crate::record_raw("os_host", "plm", &format!("PLM watcher running: window minimise/restore -> constrained TRUE/FALSE; commands (suspend|resume|constrain|unconstrain) via {}", path.display()));
        let mut constrained = false;
        loop {
            std::thread::sleep(Duration::from_millis(100));
            if let Ok(text) = std::fs::read_to_string(&path) {
                let _ = std::fs::remove_file(&path);
                for line in text.lines() {
                    if let Some((k, v)) = parse_command(line) {
                        if k == Kind::Constrained {
                            constrained = v;
                        }
                        signal_local(k, v);
                    } else if let Some(signed_in) = crate::gdk::user::parse_user_command(line) {
                        if signed_in {
                            crate::gdk::user::sign_in();
                        } else {
                            crate::gdk::user::sign_out();
                        }
                    }
                }
            }
            if registered(Kind::Constrained) == 0 {
                continue;
            }
            if let Some(min) = unsafe { title_minimized() } {
                if min != constrained {
                    constrained = min;
                    signal_local(Kind::Constrained, min);
                }
            }
        }
    });
}
