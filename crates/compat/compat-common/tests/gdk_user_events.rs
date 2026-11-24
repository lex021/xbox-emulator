use compat_common::gdk::{self, events::Registry, user, xasync, Guid};
use core::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering::SeqCst};
use std::sync::Mutex;
use std::time::{Duration, Instant};

const MANUAL: u32 = 0;
const POOL: u32 = 1;
const COMPLETION: u32 = 1;

fn xa() -> usize {
    let mut o: *mut c_void = core::ptr::null_mut();
    assert_eq!(unsafe { gdk::query_api_impl(&xasync::XASYNC, &xasync::XASYNC, &mut o) }, 0);
    o as usize
}

unsafe fn slot(obj: usize, n: usize) -> usize {
    *(*(obj as *const *const usize)).add(n)
}

fn manual_completion_queue() -> usize {
    let a = xa();
    let mut q = 0usize;
    let f: unsafe extern "system" fn(usize, u32, u32, *mut usize) -> i32 = unsafe { core::mem::transmute(slot(a, 12)) };
    assert_eq!(unsafe { f(a, POOL, MANUAL, &mut q) }, 0);
    assert_ne!(q, 0);
    q
}

fn dispatch(q: usize, timeout: u32) -> bool {
    let a = xa();
    let f: unsafe extern "system" fn(usize, usize, u32, u32) -> bool = unsafe { core::mem::transmute(slot(a, 16)) };
    unsafe { f(a, q, COMPLETION, timeout) }
}

fn drain(q: usize) -> usize {
    let mut n = 0;
    while dispatch(q, 0) {
        n += 1;
    }
    n
}

fn iface(clsid: Guid, iid: Guid) -> usize {
    let mut o: *mut c_void = core::ptr::null_mut();
    assert_eq!(unsafe { gdk::query_api_impl(&clsid, &iid, &mut o) }, 0);
    o as usize
}

type Register = unsafe extern "system" fn(usize, usize, *mut c_void, usize, *mut u64) -> i32;
type Unregister = unsafe extern "system" fn(usize, u64, bool) -> bool;

static EVENTS: Mutex<Vec<(usize, u64, u32)>> = Mutex::new(Vec::new());

unsafe extern "system" fn on_change(ctx: *mut c_void, local: u64, event: u32) {
    EVENTS.lock().unwrap().push((ctx as usize, local, event));
}

#[test]
fn user_change_events_reach_the_registered_callback_in_order_and_stop_after_unregister() {
    let q = manual_completion_queue();
    let u = iface(user::CLSID, user::IID_V2);
    unsafe {
        let reg: Register = core::mem::transmute(slot(u, 33));
        let unreg: Unregister = core::mem::transmute(slot(u, 34));
        let state: unsafe extern "system" fn(usize, usize, *mut u32) -> i32 = core::mem::transmute(slot(u, 14));
        let mut token = 0u64;
        assert_eq!(reg(u, q, 0x5150 as *mut c_void, on_change as *const () as usize, &mut token), 0);
        assert_ne!(token, 0);
        let mut st = 9u32;
        assert_eq!(state(u, 1, &mut st), 0);
        assert_eq!(st, user::STATE_SIGNED_IN);

        assert!(user::sign_out());
        assert!(!user::sign_out());
        assert_eq!(state(u, 1, &mut st), 0);
        assert_eq!(st, user::STATE_SIGNED_OUT);
        assert_eq!(drain(q), 2);
        assert!(user::sign_in());
        assert_eq!(drain(q), 1);
        assert_eq!(state(u, 1, &mut st), 0);
        assert_eq!(st, user::STATE_SIGNED_IN);
        let got: Vec<_> = EVENTS.lock().unwrap().drain(..).collect();
        assert_eq!(got, vec![(0x5150, user::LOCAL_ID, user::EVENT_SIGNING_OUT), (0x5150, user::LOCAL_ID, user::EVENT_SIGNED_OUT), (0x5150, user::LOCAL_ID, user::EVENT_SIGNED_IN_AGAIN)]);

        user::fire_change(user::EVENT_GAMERTAG);
        assert!(unreg(u, token, false));
        assert!(!unreg(u, token, false));
        drain(q);
        assert!(EVENTS.lock().unwrap().is_empty(), "a callback queued before unregister must not run after it");
        user::fire_change(user::EVENT_PRIVILEGES);
        assert_eq!(drain(q), 0);
        assert!(EVENTS.lock().unwrap().is_empty());
    }
}

static ASSOC: Mutex<Vec<([u8; 32], u64, u64, usize)>> = Mutex::new(Vec::new());

#[repr(C)]
struct Assoc {
    device: [u8; 32],
    old_user: u64,
    new_user: u64,
}

unsafe extern "system" fn on_assoc(ctx: *mut c_void, c: *const Assoc) {
    ASSOC.lock().unwrap().push(((*c).device, (*c).old_user, (*c).new_user, ctx as usize));
}

static AUDIO: Mutex<Vec<(u64, u32, String, String)>> = Mutex::new(Vec::new());

unsafe fn wide(p: *const u16) -> String {
    let mut n = 0;
    while *p.add(n) != 0 {
        n += 1;
    }
    String::from_utf16_lossy(core::slice::from_raw_parts(p, n))
}

unsafe extern "system" fn on_audio(_ctx: *mut c_void, local: u64, kind: u32, old: *const u16, new: *const u16) {
    AUDIO.lock().unwrap().push((local, kind, wide(old), wide(new)));
}

#[test]
fn device_association_and_audio_endpoint_registrations_deliver_their_payloads() {
    let q = manual_completion_queue();
    let d = iface(user::DEVICE, user::DEVICE_V2);
    unsafe {
        let reg_a: Register = core::mem::transmute(slot(d, 4));
        let unreg_a: Unregister = core::mem::transmute(slot(d, 5));
        let reg_e: Register = core::mem::transmute(slot(d, 7));
        let unreg_e: Unregister = core::mem::transmute(slot(d, 8));
        let (mut ta, mut te) = (0u64, 0u64);
        assert_eq!(reg_a(d, q, 7 as *mut c_void, on_assoc as *const () as usize, &mut ta), 0);
        assert_eq!(reg_e(d, q, core::ptr::null_mut(), on_audio as *const () as usize, &mut te), 0);
        assert_eq!(reg_a(d, q, core::ptr::null_mut(), 0, &mut ta), gdk::E_INVALIDARG);
        let mut dev = [0u8; 32];
        dev[0] = 0xab;
        dev[31] = 0xcd;
        assert_eq!(user::fire_device_association(dev, 0, user::LOCAL_ID), 1);
        assert_eq!(user::fire_audio_endpoint(1, "old-ep", "new-ep"), 1);
        assert_eq!(drain(q), 2);
        assert_eq!(*ASSOC.lock().unwrap(), vec![(dev, 0, user::LOCAL_ID, 7)]);
        assert_eq!(*AUDIO.lock().unwrap(), vec![(user::LOCAL_ID, 1, "old-ep".to_string(), "new-ep".to_string())]);
        assert!(unreg_a(d, ta, true));
        assert!(unreg_e(d, te, true));
        assert_eq!(user::fire_device_association(dev, user::LOCAL_ID, 0), 0);
        assert_eq!(drain(q), 0);
    }
}

static WAITED: Registry = Registry::new();

#[test]
fn unregister_with_wait_blocks_until_a_running_callback_returns() {
    let started = Box::leak(Box::new(AtomicBool::new(false)));
    let finished = Box::leak(Box::new(AtomicBool::new(false)));
    let mut token = 0u64;
    unsafe {
        assert_eq!(WAITED.register(0, 0, core::ptr::null_mut(), 1, &mut token), 0);
    }
    let (s, f) = (&*started, &*finished);
    assert_eq!(
        WAITED.fire_token(token, move |_| {
            s.store(true, SeqCst);
            std::thread::sleep(Duration::from_millis(300));
            f.store(true, SeqCst);
        }),
        0
    );
    let t0 = Instant::now();
    while !started.load(SeqCst) {
        assert!(t0.elapsed() < Duration::from_secs(5), "callback never started");
        std::thread::yield_now();
    }
    assert_eq!(WAITED.in_flight(token), 1);
    assert!(WAITED.unregister(None, token, true));
    assert!(finished.load(SeqCst), "unregister(wait=true) returned while the callback was still running");
    assert_eq!(WAITED.in_flight(token), 0);
}

static NOWAIT: Registry = Registry::new();

#[test]
fn unregister_without_wait_returns_while_the_callback_runs_and_a_callback_may_unregister_itself() {
    let finished = Box::leak(Box::new(AtomicBool::new(false)));
    let started = Box::leak(Box::new(AtomicBool::new(false)));
    let mut token = 0u64;
    unsafe {
        assert_eq!(NOWAIT.register(0, 0, core::ptr::null_mut(), 1, &mut token), 0);
    }
    let (s, f) = (&*started, &*finished);
    NOWAIT.fire_token(token, move |_| {
        s.store(true, SeqCst);
        std::thread::sleep(Duration::from_millis(300));
        f.store(true, SeqCst);
    });
    let t0 = Instant::now();
    while !started.load(SeqCst) {
        assert!(t0.elapsed() < Duration::from_secs(5));
        std::thread::yield_now();
    }
    assert!(NOWAIT.unregister(None, token, false));
    assert!(!finished.load(SeqCst));
    while !finished.load(SeqCst) {
        assert!(t0.elapsed() < Duration::from_secs(5));
        std::thread::yield_now();
    }

    static SELF_DONE: AtomicU32 = AtomicU32::new(0);
    let mut t2 = 0u64;
    unsafe {
        assert_eq!(NOWAIT.register(0, 0, core::ptr::null_mut(), 1, &mut t2), 0);
    }
    NOWAIT.fire_token(t2, move |r| {
        let removed = NOWAIT.unregister(None, r.token, true);
        SELF_DONE.store(if removed { 1 } else { 2 }, SeqCst);
    });
    while SELF_DONE.load(SeqCst) == 0 {
        assert!(t0.elapsed() < Duration::from_secs(5), "a callback unregistering itself with wait=true deadlocked");
        std::thread::yield_now();
    }
    assert_eq!(SELF_DONE.load(SeqCst), 1);
}
