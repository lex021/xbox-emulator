use super::*;
use core::sync::atomic::AtomicPtr;

#[link(name = "kernel32")]
extern "system" {
    fn SetErrorMode(m: u32) -> u32;
    fn WerSetFlags(f: u32) -> i32;
}

const T_SCHED: u64 = 0x10;
const T_A_START: u64 = 0x20;
const T_A_AFTER_WAIT: u64 = 0x21;
const T_A_AFTER_YIELD: u64 = 0x22;
const T_B_START: u64 = 0x30;
const T_B_SET: u64 = 0x31;
const T_B_AFTER_YIELD: u64 = 0x32;
const T_LISTED: u64 = 0x40;
const T_EXEC_FAIL: u64 = 0x50;
const T_TERMINATED: u64 = 0x60;
const T_BAD: u64 = 0x7f;

static LOG: [AtomicU64; 64] = [const { AtomicU64::new(0) }; 64];
static LOG_N: AtomicUsize = AtomicUsize::new(0);
static LIST: AtomicPtr<CompletionList> = AtomicPtr::new(null_mut());
static CTX_A: AtomicPtr<u8> = AtomicPtr::new(null_mut());
static CTX_B: AtomicPtr<u8> = AtomicPtr::new(null_mut());
static EVENT: AtomicUsize = AtomicUsize::new(0);
static STEP: AtomicU32 = AtomicU32::new(0);
static SEEN: AtomicU32 = AtomicU32::new(0);
static PRIMARY_TID: AtomicU32 = AtomicU32::new(0);

fn log(tag: u64, a: u64, b: u64) {
    let i = LOG_N.fetch_add(1, Ordering::SeqCst);
    if i < LOG.len() {
        LOG[i].store((tag << 56) | ((a & 0xff_ffff) << 32) | (b & 0xffff_ffff), Ordering::SeqCst);
    }
}

fn entries() -> Vec<(u64, u64, u64)> {
    let n = LOG_N.load(Ordering::SeqCst).min(LOG.len());
    LOG[..n].iter().map(|e| e.load(Ordering::SeqCst)).map(|v| (v >> 56, (v >> 32) & 0xff_ffff, v & 0xffff_ffff)).collect()
}

fn who(p: u64) -> u64 {
    if p == CTX_A.load(Ordering::SeqCst) as u64 {
        0xa
    } else if p == CTX_B.load(Ordering::SeqCst) as u64 {
        0xb
    } else {
        p
    }
}

fn bit(ctx: *mut u8) -> u32 {
    if ctx == CTX_A.load(Ordering::SeqCst) {
        1
    } else if ctx == CTX_B.load(Ordering::SeqCst) {
        2
    } else {
        0
    }
}

fn wait_listed(mask: u32) -> bool {
    let list = LIST.load(Ordering::SeqCst);
    for _ in 0..100 {
        if SEEN.load(Ordering::SeqCst) & mask == mask {
            return true;
        }
        let mut head = null_mut();
        if dequeue_completion_list_items(list, 100, &mut head) == STATUS_SUCCESS {
            let mut it = head;
            while !it.is_null() {
                let next = get_next_ums_list_item(it);
                let b = bit(it);
                let full = unsafe { a32(it, OFF_FLAGS).load(Ordering::SeqCst) } & FLAG_FULL_CONTEXT != 0;
                log(T_LISTED, who(it as u64), full as u64);
                SEEN.fetch_or(b, Ordering::SeqCst);
                it = next;
            }
        }
    }
    SEEN.load(Ordering::SeqCst) & mask == mask
}

fn run(ctx: *mut u8) {
    SEEN.fetch_and(!bit(ctx), Ordering::SeqCst);
    let st = execute_ums_thread(ctx);
    log(T_EXEC_FAIL, who(ctx as u64), st as u32 as u64);
}

fn is_terminated(ctx: *mut u8) -> bool {
    let mut b = 0u8;
    let mut ret = 0u32;
    query_ums_thread_information(ctx, 6, &mut b, 1, &mut ret) == STATUS_SUCCESS && ret == 1 && b == 1
}

extern "system" fn scheduler(reason: u32, payload: usize, param: *mut c_void) {
    let step = STEP.fetch_add(1, Ordering::SeqCst);
    log(T_SCHED, (step as u64) << 8 | reason as u64, (who(payload as u64) << 16) | (param as u64 & 0xffff));
    if unsafe { GetCurrentThreadId() } != PRIMARY_TID.load(Ordering::SeqCst) {
        log(T_BAD, 1, step as u64);
        return;
    }
    let a = CTX_A.load(Ordering::SeqCst);
    let b = CTX_B.load(Ordering::SeqCst);
    match step {
        0 => {
            if wait_listed(3) {
                run(a);
            }
        }
        1 => run(b),
        2 => {
            if wait_listed(1) {
                run(a);
            }
        }
        3 => run(a),
        4 => {
            if wait_listed(1) && is_terminated(a) {
                log(T_TERMINATED, 0xa, 0);
                let st = execute_ums_thread(a);
                log(T_EXEC_FAIL, 0xa, st as u32 as u64);
                run(b);
            }
        }
        5 => {
            if wait_listed(2) && is_terminated(b) {
                log(T_TERMINATED, 0xb, 0);
            }
        }
        _ => log(T_BAD, 2, step as u64),
    }
}

extern "system" fn worker_a(_p: *mut c_void) -> u32 {
    let ctx = current_context();
    let own_teb = unsafe { rd(ctx, OFF_TEB) } == teb();
    log(T_A_START, own_teb as u64, (unsafe { GetCurrentThreadId() } != PRIMARY_TID.load(Ordering::SeqCst)) as u64);
    unsafe { WaitForSingleObject(EVENT.load(Ordering::SeqCst) as Handle, u32::MAX) };
    log(T_A_AFTER_WAIT, 0, 0);
    let k = core::hint::black_box(0x5eed_1234u64);
    let f = core::hint::black_box(1.25f64);
    let st = ums_thread_yield(0xa2 as *mut c_void);
    let keep = (core::hint::black_box(k) == 0x5eed_1234) as u64 | (((core::hint::black_box(f) * 4.0) == 5.0) as u64) << 1;
    log(T_A_AFTER_YIELD, keep, st as u32 as u64);
    7
}

extern "system" fn worker_b(_p: *mut c_void) -> u32 {
    log(T_B_START, 0, 0);
    unsafe { SetEvent(EVENT.load(Ordering::SeqCst) as Handle) };
    log(T_B_SET, 0, 0);
    let st = ums_thread_yield(0xb1 as *mut c_void);
    log(T_B_AFTER_YIELD, 0, st as u32 as u64);
    9
}

#[test]
fn scheduler_runs_two_workers_through_yield_block_and_completion_list() {
    let mut list = null_mut();
    assert_eq!(create_completion_list(&mut list), STATUS_SUCCESS);
    assert!(!completion_list_event(list).is_null());
    LIST.store(list, Ordering::SeqCst);
    let (mut a, mut b) = (null_mut(), null_mut());
    assert_eq!(create_ums_thread_context(&mut a), STATUS_SUCCESS);
    assert_eq!(create_ums_thread_context(&mut b), STATUS_SUCCESS);
    CTX_A.store(a, Ordering::SeqCst);
    CTX_B.store(b, Ordering::SeqCst);
    let user = 0x1122_3344_5566_7788u64;
    assert_eq!(set_ums_thread_information(a, 1, &user as *const u64 as *const u8, 8), STATUS_SUCCESS);
    EVENT.store(unsafe { CreateEventW(null_mut(), 1, 0, core::ptr::null()) } as usize, Ordering::SeqCst);
    PRIMARY_TID.store(unsafe { GetCurrentThreadId() }, Ordering::SeqCst);
    assert!(!create_ums_worker_thread(a, list, worker_a, null_mut(), 0).is_null());
    assert!(!create_ums_worker_thread(b, list, worker_b, null_mut(), 0).is_null());
    let info = SchedulerStartupInfo { version: 0x100, list, proc_: Some(scheduler), param: 0x5a as *mut c_void };
    assert_eq!(enter_ums_scheduling_mode(&info), STATUS_SUCCESS);
    assert!(current_context().is_null());
    let e = entries();
    let s = |step: u64, reason: u64, who: u64, param: u64| (T_SCHED, step << 8 | reason, who << 16 | param);
    let want = vec![
        s(0, 0, 0, 0x5a),
        (T_LISTED, 0xa, 0),
        (T_LISTED, 0xb, 0),
        (T_A_START, 1, 1),
        s(1, 1, 1, 0),
        (T_B_START, 0, 0),
        (T_B_SET, 0, 0),
        s(2, 2, 0xb, 0xb1),
        (T_LISTED, 0xa, 1),
        (T_A_AFTER_WAIT, 0, 0),
        s(3, 2, 0xa, 0xa2),
        (T_A_AFTER_YIELD, 3, 0),
        s(4, 1, 1, 0),
        (T_LISTED, 0xa, 0),
        (T_TERMINATED, 0xa, 0),
        (T_EXEC_FAIL, 0xa, STATUS_UMS_THREAD_NOT_READY as u32 as u64),
        (T_B_AFTER_YIELD, 0, 0),
        s(5, 1, 1, 0),
        (T_LISTED, 0xb, 0),
        (T_TERMINATED, 0xb, 0),
    ];
    let mut listed_ab = e.clone();
    if listed_ab.len() > 2 && listed_ab[1] == (T_LISTED, 0xb, 0) && listed_ab[2] == (T_LISTED, 0xa, 0) {
        listed_ab.swap(1, 2);
    }
    assert_eq!(listed_ab, want, "{e:x?}");
    unsafe {
        assert_eq!(rd(a, OFF_USER), user);
        assert_eq!(a32(a, OFF_SWITCHES).load(Ordering::SeqCst), 4);
        assert_eq!(a32(b, OFF_SWITCHES).load(Ordering::SeqCst), 2);
        assert_eq!(a32(a, OFF_UYIELDS).load(Ordering::SeqCst), 1);
        assert_eq!(a32(a, OFF_STATE).load(Ordering::SeqCst), ST_TERMINATED);
        let mut code = 0u32;
        WaitForSingleObject(h(a, OFF_THREAD), 5000);
        GetExitCodeThread(h(a, OFF_THREAD), &mut code);
        assert_eq!(code, 7);
        WaitForSingleObject(h(b, OFF_THREAD), 5000);
        GetExitCodeThread(h(b, OFF_THREAD), &mut code);
        assert_eq!(code, 9);
    }
    let mut t = 0u64;
    let mut ret = 0;
    assert_eq!(query_ums_thread_information(a, 4, &mut t as *mut u64 as *mut u8, 8, &mut ret), STATUS_SUCCESS);
    assert!(t != 0 && t != teb() && ret == 8);
    assert_eq!(delete_ums_thread_context(a), STATUS_SUCCESS);
    assert_eq!(delete_ums_thread_context(b), STATUS_SUCCESS);
    assert_eq!(delete_completion_list(list), STATUS_SUCCESS);
}

#[link(name = "kernel32")]
extern "system" {
    fn GetExitCodeThread(h: Handle, code: *mut u32) -> i32;
}

static REJECT: [AtomicU32; 4] = [const { AtomicU32::new(0) }; 4];
static REJECT_CTX: AtomicPtr<u8> = AtomicPtr::new(null_mut());

extern "system" fn reject_scheduler(reason: u32, _payload: usize, _param: *mut c_void) {
    if reason != REASON_STARTUP {
        return;
    }
    let x = REJECT_CTX.load(Ordering::SeqCst);
    unsafe {
        REJECT[0].store(RtlpExecuteUmsThread(null_mut()) as u32, Ordering::SeqCst);
        wr(x, OFF_LOCK, 0x1234_5670);
        REJECT[1].store(RtlpExecuteUmsThread(x) as u32, Ordering::SeqCst);
        wr(x, OFF_LOCK, LOCK_IN_KERNEL);
        REJECT[2].store(RtlpExecuteUmsThread(x) as u32, Ordering::SeqCst);
        REJECT[3].store(RtlpUmsThreadYield(null_mut()) as u32, Ordering::SeqCst);
    }
}

#[test]
fn rtlp_entry_points_reject_misuse_without_switching() {
    unsafe {
        assert_eq!(RtlpExecuteUmsThread(null_mut()), STATUS_NOT_SUPPORTED);
        assert_eq!(RtlpUmsThreadYield(0x77 as *mut c_void), STATUS_INVALID_PARAMETER);
    }
    assert_eq!(ums_thread_yield(null_mut()), STATUS_INVALID_PARAMETER);
    assert_eq!(execute_ums_thread(null_mut()), STATUS_INVALID_PARAMETER);
    let mut list = null_mut();
    assert_eq!(create_completion_list(&mut list), STATUS_SUCCESS);
    let mut x = null_mut();
    assert_eq!(create_ums_thread_context(&mut x), STATUS_SUCCESS);
    REJECT_CTX.store(x, Ordering::SeqCst);
    let info = SchedulerStartupInfo { version: 0x100, list, proc_: Some(reject_scheduler), param: null_mut() };
    assert_eq!(enter_ums_scheduling_mode(&info), STATUS_SUCCESS);
    let got: Vec<u32> = REJECT.iter().map(|r| r.load(Ordering::SeqCst)).collect();
    assert_eq!(got, vec![STATUS_INVALID_PARAMETER as u32, STATUS_UMS_THREAD_NOT_READY as u32, STATUS_UMS_THREAD_NOT_READY as u32, STATUS_INVALID_PARAMETER as u32]);
    let bad = SchedulerStartupInfo { version: 0x101, list, proc_: Some(reject_scheduler), param: null_mut() };
    assert_eq!(enter_ums_scheduling_mode(&bad), STATUS_INVALID_PARAMETER);
    let mut head = null_mut();
    assert_eq!(dequeue_completion_list_items(list, 0, &mut head), STATUS_TIMEOUT);
    assert!(head.is_null());
    assert_eq!(delete_ums_thread_context(x), STATUS_SUCCESS);
    assert_eq!(delete_completion_list(list), STATUS_SUCCESS);
}

extern "system" fn bad_target_scheduler(reason: u32, _payload: usize, _param: *mut c_void) {
    if reason != REASON_STARTUP {
        return;
    }
    let x = REJECT_CTX.load(Ordering::SeqCst);
    unsafe {
        wr(x, OFF_TEB, teb());
        wr(x, OFF_CONTEXT + CX_RSP, 0x1000);
        wr(x, OFF_LOCK, 0);
        RtlpExecuteUmsThread(x);
    }
}

fn child_mode() -> Option<String> {
    std::env::var("TBX_UMS_FASTFAIL_CHILD").ok()
}

#[test]
fn fast_fail_child() {
    let Some(mode) = child_mode() else { return };
    unsafe {
        SetErrorMode(0x8003);
        WerSetFlags(0x20);
    }
    let mut x = null_mut();
    create_ums_thread_context(&mut x);
    if mode == "yield_end" {
        unsafe {
            wr(x, OFF_TEB, teb());
            wr(x, OFF_CONTEXT + CX_RSP, 0x1000);
            core::arch::asm!("call {f}", f = sym RtlpUmsExecuteYieldThreadEnd, in("r10") x, in("rdx") 0u64, in("rcx") 0u64, options(noreturn));
        }
    }
    let mut list = null_mut();
    create_completion_list(&mut list);
    REJECT_CTX.store(x, Ordering::SeqCst);
    let info = SchedulerStartupInfo { version: 0x100, list, proc_: Some(bad_target_scheduler), param: null_mut() };
    enter_ums_scheduling_mode(&info);
}

fn run_child(mode: &str) -> Option<i32> {
    std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "ums::tests::fast_fail_child", "--test-threads=1", "--nocapture"])
        .env("TBX_UMS_FASTFAIL_CHILD", mode)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap()
        .code()
}

#[test]
fn yield_end_fails_fast_on_a_primary_stack_outside_the_teb_limits() {
    if child_mode().is_some() {
        return;
    }
    assert_eq!(run_child("yield_end"), Some(0xC000_0409u32 as i32));
}

#[test]
fn execute_fails_fast_on_a_target_stack_outside_the_teb_limits() {
    if child_mode().is_some() {
        return;
    }
    assert_eq!(run_child("execute"), Some(0xC000_0409u32 as i32));
}

#[test]
fn completion_list_links_items_fifo_through_the_context_head() {
    let mut list = null_mut();
    assert_eq!(create_completion_list(&mut list), STATUS_SUCCESS);
    let (mut x, mut y) = (null_mut(), null_mut());
    create_ums_thread_context(&mut x);
    create_ums_thread_context(&mut y);
    unsafe {
        list_push(list, x);
        list_push(list, y);
    }
    let mut head = null_mut();
    assert_eq!(dequeue_completion_list_items(list, u32::MAX, &mut head), STATUS_SUCCESS);
    assert_eq!(head, x);
    assert_eq!(get_next_ums_list_item(x), y);
    assert!(get_next_ums_list_item(y).is_null());
    assert!(get_next_ums_list_item(x).is_null());
    let mut b = 9u8;
    assert_eq!(query_ums_thread_information(x, 6, &mut b, 1, null_mut()), STATUS_SUCCESS);
    assert_eq!(b, 0);
    assert_eq!(query_ums_thread_information(x, 4, &mut b, 1, null_mut()), STATUS_INFO_LENGTH_MISMATCH);
    assert_eq!(query_ums_thread_information(x, 2, &mut b, 1, null_mut()), STATUS_NOT_SUPPORTED);
    assert_eq!(delete_ums_thread_context(x), STATUS_SUCCESS);
    assert_eq!(delete_ums_thread_context(y), STATUS_SUCCESS);
    assert_eq!(delete_completion_list(list), STATUS_SUCCESS);
}
