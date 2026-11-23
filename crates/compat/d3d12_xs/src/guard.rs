use core::cell::Cell;
use core::ffi::c_void;
use core::panic::Location;
use core::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Once;

#[repr(C)]
struct Frame {
    rsp: usize,
    resume: usize,
    lo: usize,
    hi: usize,
    fault_addr: usize,
    fault_rip: usize,
}

thread_local! {
    static ACTIVE: Cell<*mut Frame> = const { Cell::new(core::ptr::null_mut()) };
}

extern "system" {
    fn AddVectoredExceptionHandler(first: u32, handler: unsafe extern "system" fn(*mut [usize; 2]) -> i32) -> *mut c_void;
}

extern "C" {
    fn tb_guard_enter(f: unsafe extern "C" fn(*mut c_void), ctx: *mut c_void, frame: *mut Frame) -> u32;
}

core::arch::global_asm!(
    ".text",
    ".globl tb_guard_enter",
    "tb_guard_enter:",
    "push rbp",
    "push rbx",
    "push rdi",
    "push rsi",
    "push r12",
    "push r13",
    "push r14",
    "push r15",
    "sub rsp, 200",
    "movaps [rsp + 32], xmm6",
    "movaps [rsp + 48], xmm7",
    "movaps [rsp + 64], xmm8",
    "movaps [rsp + 80], xmm9",
    "movaps [rsp + 96], xmm10",
    "movaps [rsp + 112], xmm11",
    "movaps [rsp + 128], xmm12",
    "movaps [rsp + 144], xmm13",
    "movaps [rsp + 160], xmm14",
    "movaps [rsp + 176], xmm15",
    "mov [r8], rsp",
    "lea rax, [rip + tb_guard_resume]",
    "mov [r8 + 8], rax",
    "mov rax, rcx",
    "mov rcx, rdx",
    "call rax",
    "mov eax, 1",
    "tb_guard_done:",
    "movaps xmm6, [rsp + 32]",
    "movaps xmm7, [rsp + 48]",
    "movaps xmm8, [rsp + 64]",
    "movaps xmm9, [rsp + 80]",
    "movaps xmm10, [rsp + 96]",
    "movaps xmm11, [rsp + 112]",
    "movaps xmm12, [rsp + 128]",
    "movaps xmm13, [rsp + 144]",
    "movaps xmm14, [rsp + 160]",
    "movaps xmm15, [rsp + 176]",
    "add rsp, 200",
    "pop r15",
    "pop r14",
    "pop r13",
    "pop r12",
    "pop rsi",
    "pop rdi",
    "pop rbx",
    "pop rbp",
    "ret",
    "tb_guard_resume:",
    "xor eax, eax",
    "jmp tb_guard_done",
);

const STATUS_ACCESS_VIOLATION: u32 = 0xC000_0005;
const STATUS_IN_PAGE_ERROR: u32 = 0xC000_0006;
const RECORD_NUMBER_PARAMETERS: usize = 0x18;
const RECORD_EXCEPTION_ADDRESS: usize = 0x10;
const RECORD_FAULT_ADDRESS: usize = 0x28;
const CONTEXT_RSP: usize = 0x98;
const CONTEXT_RIP: usize = 0xF8;
const EXCEPTION_CONTINUE_EXECUTION: i32 = -1;
const EXCEPTION_CONTINUE_SEARCH: i32 = 0;
const FIRST_LAST: u32 = 0;
const LOG_FIRST: usize = 32;
const LOG_EVERY: usize = 1024;

static RECOVERED: AtomicUsize = AtomicUsize::new(0);

fn catches(lo: usize, hi: usize, code: u32, parameters: u32, fault: usize) -> bool {
    (code == STATUS_ACCESS_VIOLATION || code == STATUS_IN_PAGE_ERROR) && parameters >= 2 && fault >= lo && fault < hi
}

unsafe extern "system" fn handler(info: *mut [usize; 2]) -> i32 {
    let frame = ACTIVE.with(|a| a.get());
    if frame.is_null() {
        return EXCEPTION_CONTINUE_SEARCH;
    }
    let record = (*info)[0] as *const u8;
    let code = *(record as *const u32);
    let parameters = *(record.add(RECORD_NUMBER_PARAMETERS) as *const u32);
    let fault = if parameters >= 2 { *(record.add(RECORD_FAULT_ADDRESS) as *const usize) } else { 0 };
    let frame = &mut *frame;
    if !catches(frame.lo, frame.hi, code, parameters, fault) {
        return EXCEPTION_CONTINUE_SEARCH;
    }
    frame.fault_addr = fault;
    frame.fault_rip = *(record.add(RECORD_EXCEPTION_ADDRESS) as *const usize);
    let context = (*info)[1] as *mut u8;
    *(context.add(CONTEXT_RSP) as *mut usize) = frame.rsp;
    *(context.add(CONTEXT_RIP) as *mut usize) = frame.resume;
    EXCEPTION_CONTINUE_EXECUTION
}

fn install() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| unsafe {
        AddVectoredExceptionHandler(FIRST_LAST, handler);
    });
}

fn note_recovered(site: &Location, fault: usize, rip: usize) {
    let n = RECOVERED.fetch_add(1, Ordering::Relaxed);
    if n < LOG_FIRST || n % LOG_EVERY == 0 {
        compat_common::record(
            "d3d12_xs",
            "guard",
            &format!("recovered a fault reading title memory at {fault:#x} (rip {rip:#x}) from {}:{}; {} recovered so far", site.file(), site.line(), n + 1),
        );
    }
}

pub fn recovered() -> usize {
    RECOVERED.load(Ordering::Relaxed)
}

#[track_caller]
pub fn guarded(lo: usize, len: usize, f: unsafe extern "C" fn(*mut c_void), ctx: *mut c_void) -> bool {
    install();
    let site = Location::caller();
    let mut frame = Frame { rsp: 0, resume: 0, lo, hi: lo.saturating_add(len), fault_addr: 0, fault_rip: 0 };
    let prev = ACTIVE.with(|a| a.replace(&mut frame));
    let ok = unsafe { tb_guard_enter(f, ctx, &mut frame) };
    ACTIVE.with(|a| a.set(prev));
    if ok == 0 {
        note_recovered(site, frame.fault_addr, frame.fault_rip);
    }
    ok != 0
}

#[cfg(test)]
mod tests {
    use super::*;

    extern "system" {
        fn VirtualAlloc(addr: *mut c_void, len: usize, ty: u32, protect: u32) -> *mut c_void;
        fn VirtualFree(addr: *mut c_void, len: usize, ty: u32) -> i32;
    }

    const PAGE: usize = 4096;

    struct SumJob {
        addr: usize,
        len: usize,
        sum: u64,
    }

    unsafe extern "C" fn sum_job(ctx: *mut c_void) {
        let job = &mut *(ctx as *mut SumJob);
        job.sum = core::slice::from_raw_parts(job.addr as *const u8, job.len).iter().map(|&b| b as u64).sum();
    }

    const _: () = assert!(!core::mem::needs_drop::<SumJob>());

    fn sum(addr: usize, len: usize) -> Option<u64> {
        let mut job = SumJob { addr, len, sum: 0 };
        guarded(addr, len, sum_job, &mut job as *mut SumJob as *mut c_void).then_some(job.sum)
    }

    unsafe fn region_with_a_hole() -> usize {
        let base = VirtualAlloc(core::ptr::null_mut(), 3 * PAGE, 0x3000, 0x04) as usize;
        assert_ne!(base, 0);
        core::ptr::write_bytes(base as *mut u8, 0xab, 3 * PAGE);
        assert_eq!(VirtualFree((base + PAGE) as *mut c_void, PAGE, 0x4000), 1);
        base
    }

    unsafe fn release(base: usize) {
        VirtualFree(base as *mut c_void, 0, 0x8000);
    }

    #[test]
    fn a_read_that_does_not_fault_completes() {
        let v = vec![3u8; 1000];
        assert_eq!(sum(v.as_ptr() as usize, v.len()), Some(3000));
        assert_eq!(sum(v.as_ptr() as usize, 0), Some(0));
    }

    #[test]
    fn a_read_of_decommitted_memory_inside_the_range_is_recovered_and_counted() {
        unsafe {
            let base = region_with_a_hole();
            let before = recovered();
            assert_eq!(sum(base, PAGE), Some(0xab * PAGE as u64));
            assert_eq!(sum(base + PAGE, PAGE), None);
            assert_eq!(sum(base, 3 * PAGE), None);
            assert_eq!(sum(base + 2 * PAGE, PAGE), Some(0xab * PAGE as u64));
            assert!(recovered() >= before + 2);
            release(base);
        }
    }

    unsafe extern "C" fn read_one_byte(ctx: *mut c_void) {
        let addr = *(ctx as *const usize);
        core::ptr::read_volatile(addr as *const u8);
    }

    #[test]
    fn nonvolatile_registers_survive_a_fault() {
        unsafe {
            let base = region_with_a_hole();
            let mut acc = 0u64;
            for i in 0..1000u64 {
                acc = acc.wrapping_add(i);
                let mut hole = base + PAGE;
                assert!(!guarded(base, 3 * PAGE, read_one_byte, &mut hole as *mut usize as *mut c_void));
                let mut ok = base;
                assert!(guarded(base, 3 * PAGE, read_one_byte, &mut ok as *mut usize as *mut c_void));
            }
            assert_eq!(acc, 499500);
            release(base);
        }
    }

    unsafe extern "C" fn nested(ctx: *mut c_void) {
        let hole = *(ctx as *const usize);
        let mut inner = hole;
        assert!(!guarded(hole, 1, read_one_byte, &mut inner as *mut usize as *mut c_void));
        let mut again = hole;
        assert!(!guarded(hole, 1, read_one_byte, &mut again as *mut usize as *mut c_void));
        read_one_byte(ctx);
    }

    #[test]
    fn nested_guards_each_recover_and_the_outer_one_still_catches() {
        unsafe {
            let base = region_with_a_hole();
            let mut hole = base + PAGE;
            assert!(!guarded(hole, 1, nested, &mut hole as *mut usize as *mut c_void));
            assert!(ACTIVE.with(|a| a.get()).is_null());
            assert_eq!(sum(base, PAGE), Some(0xab * PAGE as u64));
            release(base);
        }
    }

    #[test]
    fn only_faults_inside_the_declared_range_are_caught() {
        let (lo, hi) = (0x10000usize, 0x20000usize);
        assert!(catches(lo, hi, STATUS_ACCESS_VIOLATION, 2, lo));
        assert!(catches(lo, hi, STATUS_IN_PAGE_ERROR, 3, hi - 1));
        assert!(!catches(lo, hi, STATUS_ACCESS_VIOLATION, 2, hi));
        assert!(!catches(lo, hi, STATUS_ACCESS_VIOLATION, 2, lo - 1));
        assert!(!catches(lo, hi, STATUS_ACCESS_VIOLATION, 2, 0));
        assert!(!catches(lo, hi, STATUS_ACCESS_VIOLATION, 1, lo));
        assert!(!catches(lo, hi, 0xC000_00FD, 2, lo));
        assert!(!catches(lo, hi, 0x8000_0003, 2, lo));
    }

    fn fabricated(code: u32, parameters: u32, fault: usize, rip: usize) -> ([u8; 0x98], [u8; 0x500]) {
        let mut record = [0u8; 0x98];
        record[0..4].copy_from_slice(&code.to_le_bytes());
        record[RECORD_EXCEPTION_ADDRESS..RECORD_EXCEPTION_ADDRESS + 8].copy_from_slice(&rip.to_le_bytes());
        record[RECORD_NUMBER_PARAMETERS..RECORD_NUMBER_PARAMETERS + 4].copy_from_slice(&parameters.to_le_bytes());
        record[RECORD_FAULT_ADDRESS..RECORD_FAULT_ADDRESS + 8].copy_from_slice(&fault.to_le_bytes());
        (record, [0u8; 0x500])
    }

    fn run_handler(frame: &mut Frame, active: bool, code: u32, parameters: u32, fault: usize) -> (i32, usize, usize) {
        let (mut record, mut context) = fabricated(code, parameters, fault, 0x7777);
        let mut info = [record.as_mut_ptr() as usize, context.as_mut_ptr() as usize];
        let prev = ACTIVE.with(|a| a.replace(if active { frame as *mut Frame } else { core::ptr::null_mut() }));
        let r = unsafe { handler(&mut info) };
        ACTIVE.with(|a| a.set(prev));
        let rsp = usize::from_le_bytes(context[CONTEXT_RSP..CONTEXT_RSP + 8].try_into().unwrap());
        let rip = usize::from_le_bytes(context[CONTEXT_RIP..CONTEXT_RIP + 8].try_into().unwrap());
        (r, rsp, rip)
    }

    #[test]
    fn the_handler_lets_every_other_exception_continue_its_search() {
        let mut frame = Frame { rsp: 0x1111, resume: 0x2222, lo: 0x10000, hi: 0x20000, fault_addr: 0, fault_rip: 0 };
        assert_eq!(run_handler(&mut frame, true, STATUS_ACCESS_VIOLATION, 2, 0x30000), (EXCEPTION_CONTINUE_SEARCH, 0, 0));
        assert_eq!(run_handler(&mut frame, true, STATUS_ACCESS_VIOLATION, 2, 0), (EXCEPTION_CONTINUE_SEARCH, 0, 0));
        assert_eq!(run_handler(&mut frame, true, 0xC000_0094, 2, 0x10000), (EXCEPTION_CONTINUE_SEARCH, 0, 0));
        assert_eq!(run_handler(&mut frame, false, STATUS_ACCESS_VIOLATION, 2, 0x10000), (EXCEPTION_CONTINUE_SEARCH, 0, 0));
        assert_eq!(frame.fault_addr, 0);
        assert_eq!(run_handler(&mut frame, true, STATUS_ACCESS_VIOLATION, 2, 0x10008), (EXCEPTION_CONTINUE_EXECUTION, 0x1111, 0x2222));
        assert_eq!((frame.fault_addr, frame.fault_rip), (0x10008, 0x7777));
    }

    #[test]
    fn a_thread_decommitting_concurrently_never_crashes_the_reader() {
        unsafe {
            let base = VirtualAlloc(core::ptr::null_mut(), 64 * PAGE, 0x3000, 0x04) as usize;
            assert_ne!(base, 0);
            let stop = std::sync::atomic::AtomicBool::new(false);
            let (mut ok, mut failed) = (0u32, 0u32);
            std::thread::scope(|s| {
                s.spawn(|| {
                    let mut i = 0usize;
                    while !stop.load(Ordering::Relaxed) {
                        let page = (base + (i % 64) * PAGE) as *mut c_void;
                        VirtualFree(page, PAGE, 0x4000);
                        VirtualAlloc(page, PAGE, 0x1000, 0x04);
                        i += 1;
                    }
                });
                for _ in 0..20000 {
                    match sum(base, 64 * PAGE) {
                        Some(_) => ok += 1,
                        None => failed += 1,
                    }
                }
                stop.store(true, Ordering::Relaxed);
            });
            eprintln!("concurrent decommit: {ok} reads completed, {failed} faulted and were skipped");
            assert_eq!(ok + failed, 20000);
            release(base);
        }
    }

    #[test]
    fn the_guard_is_cleared_after_every_call() {
        assert!(ACTIVE.with(|a| a.get()).is_null());
        let v = [1u8];
        sum(v.as_ptr() as usize, 1);
        assert!(ACTIVE.with(|a| a.get()).is_null());
    }
}
