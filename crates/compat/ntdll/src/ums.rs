#![allow(dead_code)]

use core::ffi::c_void;
use core::ptr::null_mut;
use core::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};

pub type Handle = *mut c_void;
pub type Ntstatus = i32;
pub type SchedulerProc = extern "system" fn(u32, usize, *mut c_void);
pub type WorkerStart = extern "system" fn(*mut c_void) -> u32;

pub const STATUS_SUCCESS: Ntstatus = 0;
pub const STATUS_TIMEOUT: Ntstatus = 0x102;
pub const STATUS_INFO_LENGTH_MISMATCH: Ntstatus = 0xC000_0004u32 as i32;
pub const STATUS_INVALID_PARAMETER: Ntstatus = 0xC000_000Du32 as i32;
pub const STATUS_NO_MEMORY: Ntstatus = 0xC000_0017u32 as i32;
pub const STATUS_NOT_SUPPORTED: Ntstatus = 0xC000_00BBu32 as i32;
pub const STATUS_INTERNAL_ERROR: Ntstatus = 0xC000_00E5u32 as i32;
pub const STATUS_RETRY: Ntstatus = 0xC000_022Du32 as i32;
pub const STATUS_UMS_THREAD_NOT_READY: Ntstatus = 0xC000_071Cu32 as i32;

pub const REASON_STARTUP: u32 = 0;
pub const REASON_BLOCKED: u32 = 1;
pub const REASON_YIELD: u32 = 2;

pub const CTX_SIZE: usize = 0x1000;
pub const OFF_NEXT: usize = 0x0;
pub const OFF_CONTEXT: usize = 0x10;
pub const OFF_TEB: usize = 0x4e0;
pub const OFF_USER: usize = 0x4e8;
pub const OFF_FLAGS: usize = 0x4f0;
pub const OFF_LOCK: usize = 0x4f8;
pub const OFF_PREV: usize = 0x500;
pub const OFF_SWITCHES: usize = 0x508;
pub const OFF_KYIELDS: usize = 0x50c;
pub const OFF_KYIELD_FAILS: usize = 0x510;
pub const OFF_UYIELDS: usize = 0x514;
pub const OFF_EXT: usize = 0xc00;
pub const OFF_RUN_EVENT: usize = OFF_EXT;
pub const OFF_HANDOFF_EVENT: usize = OFF_EXT + 0x08;
pub const OFF_HANDOFF_REASON: usize = OFF_EXT + 0x10;
pub const OFF_HANDOFF_PAYLOAD: usize = OFF_EXT + 0x18;
pub const OFF_HANDOFF_PARAM: usize = OFF_EXT + 0x20;
pub const OFF_STATE: usize = OFF_EXT + 0x28;
pub const OFF_THREAD: usize = OFF_EXT + 0x30;
pub const OFF_TID: usize = OFF_EXT + 0x38;
pub const OFF_LIST: usize = OFF_EXT + 0x40;
pub const OFF_TRAP: usize = OFF_EXT + 0x48;
pub const OFF_START: usize = OFF_EXT + 0x50;
pub const OFF_START_PARAM: usize = OFF_EXT + 0x58;

pub const FLAG_WORKER: u32 = 0x1;
pub const FLAG_SUSPENDED: u32 = 0x2;
pub const FLAG_FULL_CONTEXT: u32 = 0x4;
pub const FLAG_TERMINATED: u32 = 0x8;
pub const FLAG_YIELDING: u32 = 0x40;

pub const LOCK_IN_KERNEL: u64 = 0x1;

pub const ST_CREATED: u32 = 0;
pub const ST_RUNNING: u32 = 1;
pub const ST_PARKED: u32 = 2;
pub const ST_BLOCKED: u32 = 3;
pub const ST_LISTED: u32 = 4;
pub const ST_TERMINATED: u32 = 5;

pub const CONTEXT_AMD64: u32 = 0x0010_0000;
pub const CONTEXT_CONTROL: u32 = CONTEXT_AMD64 | 1;
pub const CONTEXT_DEBUG_REGISTERS: u32 = CONTEXT_AMD64 | 0x10;
pub const CX_FLAGS: usize = 0x30;
pub const CX_DR0: usize = 0x48;
pub const CX_DR6: usize = 0x68;
pub const CX_DR7: usize = 0x70;
pub const CX_EFLAGS: usize = 0x44;
pub const CX_RSP: usize = 0x98;
pub const CX_RIP: usize = 0xf8;
pub const CONTEXT_BYTES: usize = 0x4d0;

#[repr(C, align(16))]
pub struct Ctx(pub [u8; CONTEXT_BYTES]);

impl Ctx {
    pub fn zeroed() -> Self {
        Ctx([0; CONTEXT_BYTES])
    }
    pub fn u64(&self, off: usize) -> u64 {
        u64::from_le_bytes(self.0[off..off + 8].try_into().unwrap())
    }
    pub fn set_u64(&mut self, off: usize, v: u64) {
        self.0[off..off + 8].copy_from_slice(&v.to_le_bytes());
    }
    pub fn set_u32(&mut self, off: usize, v: u32) {
        self.0[off..off + 4].copy_from_slice(&v.to_le_bytes());
    }
}

#[repr(C)]
pub struct ExceptionPointers {
    pub record: *mut u8,
    pub context: *mut u8,
}

#[link(name = "kernel32")]
extern "system" {
    pub fn CreateEventW(a: *mut c_void, manual: i32, initial: i32, name: *const u16) -> Handle;
    pub fn SetEvent(h: Handle) -> i32;
    pub fn ResetEvent(h: Handle) -> i32;
    pub fn WaitForSingleObject(h: Handle, ms: u32) -> u32;
    pub fn CloseHandle(h: Handle) -> i32;
    pub fn CreateThread(a: *mut c_void, stack: usize, start: extern "system" fn(*mut c_void) -> u32, p: *mut c_void, flags: u32, tid: *mut u32) -> Handle;
    pub fn SuspendThread(h: Handle) -> u32;
    pub fn ResumeThread(h: Handle) -> u32;
    pub fn GetThreadContext(h: Handle, c: *mut Ctx) -> i32;
    pub fn SetThreadContext(h: Handle, c: *const Ctx) -> i32;
    pub fn GetCurrentThreadId() -> u32;
    pub fn GetCurrentThread() -> Handle;
    pub fn GetCurrentProcessId() -> u32;
    pub fn AddVectoredExceptionHandler(first: u32, h: extern "system" fn(*mut ExceptionPointers) -> i32) -> *mut c_void;
    pub fn RemoveVectoredExceptionHandler(h: *mut c_void) -> u32;
    pub fn Sleep(ms: u32);
    pub fn TlsAlloc() -> u32;
    pub fn TlsGetValue(i: u32) -> *mut c_void;
    pub fn TlsSetValue(i: u32, v: *mut c_void) -> i32;
    pub fn VirtualAlloc(a: *mut c_void, size: usize, ty: u32, prot: u32) -> *mut c_void;
    pub fn VirtualFree(a: *mut c_void, size: usize, ty: u32) -> i32;
    pub fn QueryThreadCycleTime(h: Handle, c: *mut u64) -> i32;
    pub fn GetTickCount64() -> u64;
    pub fn AcquireSRWLockExclusive(l: *mut usize);
    pub fn ReleaseSRWLockExclusive(l: *mut usize);
}

#[link(name = "ntdll")]
extern "system" {
    fn NtQuerySystemInformation(class: u32, buf: *mut c_void, len: u32, ret: *mut u32) -> Ntstatus;
}

extern "system" {
    pub fn RtlpUmsThreadYield(param: *mut c_void) -> Ntstatus;
    pub fn RtlpUmsExecuteYieldThreadEnd() -> !;
    fn tbx_ums_park(ctx: *mut u8, param: *mut c_void, then: extern "system" fn(*mut u8, *mut c_void)) -> Ntstatus;
    fn tbx_ums_primary_wrap(proc_: SchedulerProc, param: *mut c_void, primary: *mut u8);
    fn tbx_ums_enter_primary(primary: *mut u8, reason: u64, payload: u64, param: u64) -> !;
}

core::arch::global_asm!(
    ".globl RtlpUmsThreadYield",
    ".def RtlpUmsThreadYield; .scl 2; .type 32; .endef",
    ".seh_proc RtlpUmsThreadYield",
    "RtlpUmsThreadYield:",
    "sub rsp, 0x28",
    ".seh_stackalloc 0x28",
    ".seh_endprologue",
    "mov qword ptr [rsp + 0x20], rcx",
    "call {current}",
    "mov rdx, qword ptr [rsp + 0x20]",
    "add rsp, 0x28",
    "test rax, rax",
    "jz 2f",
    "test dword ptr [rax + 0x4f0], 1",
    "jz 2f",
    "mov rcx, rax",
    "lea r8, [rip + {yield_handoff}]",
    "jmp tbx_ums_park",
    "2:",
    "mov eax, 0xC000000D",
    "ret",
    ".seh_endproc",
    "",
    ".globl tbx_ums_park",
    ".def tbx_ums_park; .scl 2; .type 32; .endef",
    ".seh_proc tbx_ums_park",
    "tbx_ums_park:",
    "sub rsp, 0x28",
    ".seh_stackalloc 0x28",
    ".seh_endprologue",
    "lock btr dword ptr [rcx + 0x4f0], 2",
    "lea r9, [rcx + 0x55f]",
    "and r9, -64",
    "movdqa xmmword ptr [r9 + 0x100], xmm6",
    "movdqa xmmword ptr [r9 + 0x110], xmm7",
    "movdqa xmmword ptr [r9 + 0x120], xmm8",
    "movdqa xmmword ptr [r9 + 0x130], xmm9",
    "movdqa xmmword ptr [r9 + 0x140], xmm10",
    "movdqa xmmword ptr [r9 + 0x150], xmm11",
    "movdqa xmmword ptr [r9 + 0x160], xmm12",
    "movdqa xmmword ptr [r9 + 0x170], xmm13",
    "movdqa xmmword ptr [r9 + 0x180], xmm14",
    "movdqa xmmword ptr [r9 + 0x190], xmm15",
    "stmxcsr dword ptr [r9 + 0x18]",
    "fnclex",
    "wait",
    "fnstcw word ptr [r9]",
    "mov qword ptr [rcx + 0xa0], rbx",
    "mov qword ptr [rcx + 0xb0], rbp",
    "mov qword ptr [rcx + 0xb8], rsi",
    "mov qword ptr [rcx + 0xc0], rdi",
    "mov qword ptr [rcx + 0xe8], r12",
    "mov qword ptr [rcx + 0xf0], r13",
    "mov qword ptr [rcx + 0xf8], r14",
    "mov qword ptr [rcx + 0x100], r15",
    "mov qword ptr [rcx + 0x88], 0",
    "lea rax, [rsp + 0x30]",
    "mov qword ptr [rcx + 0xa8], rax",
    "mov rax, qword ptr [rsp + 0x28]",
    "mov qword ptr [rcx + 0x108], rax",
    "mov rbx, rcx",
    "call r8",
    "mov rcx, rbx",
    "lea rdx, [rcx + 0x55f]",
    "and rdx, -64",
    "movdqa xmm6, xmmword ptr [rdx + 0x100]",
    "movdqa xmm7, xmmword ptr [rdx + 0x110]",
    "movdqa xmm8, xmmword ptr [rdx + 0x120]",
    "movdqa xmm9, xmmword ptr [rdx + 0x130]",
    "movdqa xmm10, xmmword ptr [rdx + 0x140]",
    "movdqa xmm11, xmmword ptr [rdx + 0x150]",
    "movdqa xmm12, xmmword ptr [rdx + 0x160]",
    "movdqa xmm13, xmmword ptr [rdx + 0x170]",
    "movdqa xmm14, xmmword ptr [rdx + 0x180]",
    "movdqa xmm15, xmmword ptr [rdx + 0x190]",
    "ldmxcsr dword ptr [rdx + 0x18]",
    "fldcw word ptr [rdx]",
    "lea rcx, [rcx + 0x10]",
    "mov rax, qword ptr [rcx + 0x78]",
    "mov rbx, qword ptr [rcx + 0x90]",
    "mov rbp, qword ptr [rcx + 0xa0]",
    "mov rsi, qword ptr [rcx + 0xa8]",
    "mov rdi, qword ptr [rcx + 0xb0]",
    "mov r12, qword ptr [rcx + 0xd8]",
    "mov r13, qword ptr [rcx + 0xe0]",
    "mov r14, qword ptr [rcx + 0xe8]",
    "mov r15, qword ptr [rcx + 0xf0]",
    "mov rsp, qword ptr [rcx + 0x98]",
    "mov rcx, qword ptr [rcx + 0xf8]",
    "jmp rcx",
    ".seh_endproc",
    "",
    ".globl tbx_ums_primary_wrap",
    ".def tbx_ums_primary_wrap; .scl 2; .type 32; .endef",
    ".seh_proc tbx_ums_primary_wrap",
    "tbx_ums_primary_wrap:",
    "sub rsp, 0x148",
    ".seh_stackalloc 0x148",
    ".seh_endprologue",
    "movaps xmmword ptr [rsp + 0x30], xmm6",
    "movaps xmmword ptr [rsp + 0x40], xmm7",
    "movaps xmmword ptr [rsp + 0x50], xmm8",
    "movaps xmmword ptr [rsp + 0x60], xmm9",
    "movaps xmmword ptr [rsp + 0x70], xmm10",
    "movaps xmmword ptr [rsp + 0x80], xmm11",
    "movaps xmmword ptr [rsp + 0x90], xmm12",
    "movaps xmmword ptr [rsp + 0xa0], xmm13",
    "movaps xmmword ptr [rsp + 0xb0], xmm14",
    "movaps xmmword ptr [rsp + 0xc0], xmm15",
    "mov qword ptr [rsp + 0xd0], rbp",
    "mov qword ptr [rsp + 0xd8], rbx",
    "mov qword ptr [rsp + 0xe0], rdi",
    "mov qword ptr [rsp + 0xe8], rsi",
    "mov qword ptr [rsp + 0xf0], r12",
    "mov qword ptr [rsp + 0xf8], r13",
    "mov qword ptr [rsp + 0x100], r14",
    "mov qword ptr [rsp + 0x108], r15",
    "mov qword ptr [rsp + 0x110], r8",
    "mov rbp, rsp",
    "mov qword ptr [r8 + 0xe0], rcx",
    "mov qword ptr [r8 + 0xb0], rbp",
    "mov qword ptr [r8 + 0xa8], rsp",
    "lea rax, [rip + 3f]",
    "mov qword ptr [r8 + 0x108], rax",
    "xor r12d, r12d",
    "xor r13d, r13d",
    "mov r14, rdx",
    "3:",
    "mov rax, qword ptr [rbp + 0x110]",
    "mov rax, qword ptr [rax + 0xe0]",
    "mov rcx, r12",
    "mov rdx, r13",
    "mov r8, r14",
    "call rax",
    "movaps xmm6, xmmword ptr [rsp + 0x30]",
    "movaps xmm7, xmmword ptr [rsp + 0x40]",
    "movaps xmm8, xmmword ptr [rsp + 0x50]",
    "movaps xmm9, xmmword ptr [rsp + 0x60]",
    "movaps xmm10, xmmword ptr [rsp + 0x70]",
    "movaps xmm11, xmmword ptr [rsp + 0x80]",
    "movaps xmm12, xmmword ptr [rsp + 0x90]",
    "movaps xmm13, xmmword ptr [rsp + 0xa0]",
    "movaps xmm14, xmmword ptr [rsp + 0xb0]",
    "movaps xmm15, xmmword ptr [rsp + 0xc0]",
    "mov rbp, qword ptr [rsp + 0xd0]",
    "mov rbx, qword ptr [rsp + 0xd8]",
    "mov rdi, qword ptr [rsp + 0xe0]",
    "mov rsi, qword ptr [rsp + 0xe8]",
    "mov r12, qword ptr [rsp + 0xf0]",
    "mov r13, qword ptr [rsp + 0xf8]",
    "mov r14, qword ptr [rsp + 0x100]",
    "mov r15, qword ptr [rsp + 0x108]",
    "add rsp, 0x148",
    "ret",
    ".seh_endproc",
    "",
    ".globl RtlpUmsExecuteYieldThreadEnd",
    ".def RtlpUmsExecuteYieldThreadEnd; .scl 2; .type 32; .endef",
    "RtlpUmsExecuteYieldThreadEnd:",
    "mov r12, 2",
    "mov r13, rdx",
    "mov r14, rcx",
    "tbx_ums_primary_tail:",
    "lea r11, [r10 + 0x10]",
    "mov rcx, qword ptr [r10 + 0xa8]",
    "mov rdx, qword ptr [r10 + 0x4e0]",
    "test rdx, rdx",
    "jz 5f",
    "test ecx, 7",
    "jnz 5f",
    "cmp rcx, qword ptr [rdx + 0x10]",
    "jb 5f",
    "cmp rcx, qword ptr [rdx + 0x8]",
    "jb 6f",
    "5:",
    "mov ecx, 0xd",
    "int 0x29",
    "6:",
    "mov rbp, qword ptr [r11 + 0xa0]",
    "mov rax, qword ptr [r11 + 0xf8]",
    "mov r11, qword ptr [r11 + 0x98]",
    "mov qword ptr [r11 - 8], rax",
    "lea rsp, [r11 - 8]",
    "ret",
    "",
    ".globl tbx_ums_enter_primary",
    ".def tbx_ums_enter_primary; .scl 2; .type 32; .endef",
    "tbx_ums_enter_primary:",
    "mov r10, rcx",
    "cmp rdx, 2",
    "jne 7f",
    "mov rdx, r8",
    "mov rcx, r9",
    "jmp RtlpUmsExecuteYieldThreadEnd",
    "7:",
    "mov r12, rdx",
    "mov r13, r8",
    "mov r14, r9",
    "jmp tbx_ums_primary_tail",
    current = sym current_context_raw,
    yield_handoff = sym yield_handoff,
);

static TLS_INDEX: AtomicU32 = AtomicU32::new(u32::MAX);
static VEH: AtomicUsize = AtomicUsize::new(0);

fn tls() -> u32 {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| TLS_INDEX.store(unsafe { TlsAlloc() }, Ordering::SeqCst));
    TLS_INDEX.load(Ordering::SeqCst)
}

fn ensure_veh() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| VEH.store(unsafe { AddVectoredExceptionHandler(1, blocked_return_trap) } as usize, Ordering::SeqCst));
}

unsafe fn a64(ctx: *mut u8, off: usize) -> &'static AtomicU64 {
    &*(ctx.add(off) as *const AtomicU64)
}

unsafe fn a32(ctx: *mut u8, off: usize) -> &'static AtomicU32 {
    &*(ctx.add(off) as *const AtomicU32)
}

unsafe fn rd(ctx: *mut u8, off: usize) -> u64 {
    a64(ctx, off).load(Ordering::SeqCst)
}

unsafe fn wr(ctx: *mut u8, off: usize, v: u64) {
    a64(ctx, off).store(v, Ordering::SeqCst)
}

unsafe fn h(ctx: *mut u8, off: usize) -> Handle {
    rd(ctx, off) as Handle
}

fn teb() -> u64 {
    let t: u64;
    unsafe { core::arch::asm!("mov {}, gs:[0x30]", out(reg) t, options(nostack, readonly, preserves_flags)) };
    t
}

extern "system" fn current_context_raw() -> *mut u8 {
    let i = TLS_INDEX.load(Ordering::SeqCst);
    if i == u32::MAX {
        return null_mut();
    }
    unsafe { TlsGetValue(i) as *mut u8 }
}

pub fn current_context() -> *mut u8 {
    current_context_raw()
}

fn set_current(ctx: *mut u8) {
    unsafe { TlsSetValue(tls(), ctx as *mut c_void) };
}

#[repr(C)]
pub struct CompletionList {
    lock: usize,
    event: Handle,
    head: *mut u8,
    tail: *mut u8,
}

pub fn create_completion_list(out: *mut *mut CompletionList) -> Ntstatus {
    if out.is_null() {
        return STATUS_INVALID_PARAMETER;
    }
    let event = unsafe { CreateEventW(null_mut(), 0, 0, core::ptr::null()) };
    if event.is_null() {
        return STATUS_NO_MEMORY;
    }
    let l = Box::into_raw(Box::new(CompletionList { lock: 0, event, head: null_mut(), tail: null_mut() }));
    unsafe { *out = l };
    STATUS_SUCCESS
}

pub fn delete_completion_list(l: *mut CompletionList) -> Ntstatus {
    if l.is_null() {
        return STATUS_INVALID_PARAMETER;
    }
    unsafe {
        if !(*l).head.is_null() {
            return STATUS_INVALID_PARAMETER;
        }
        CloseHandle((*l).event);
        drop(Box::from_raw(l));
    }
    STATUS_SUCCESS
}

pub fn completion_list_event(l: *mut CompletionList) -> Handle {
    if l.is_null() {
        return null_mut();
    }
    unsafe { (*l).event }
}

unsafe fn list_push(l: *mut CompletionList, ctx: *mut u8) {
    wr(ctx, OFF_NEXT, 0);
    AcquireSRWLockExclusive(&mut (*l).lock);
    if (*l).tail.is_null() {
        (*l).head = ctx;
    } else {
        wr((*l).tail, OFF_NEXT, ctx as u64);
    }
    (*l).tail = ctx;
    ReleaseSRWLockExclusive(&mut (*l).lock);
    SetEvent((*l).event);
}

pub fn dequeue_completion_list_items(l: *mut CompletionList, timeout_ms: u32, out: *mut *mut u8) -> Ntstatus {
    if l.is_null() || out.is_null() {
        return STATUS_INVALID_PARAMETER;
    }
    let deadline = unsafe { GetTickCount64() }.saturating_add(timeout_ms as u64);
    loop {
        let head = unsafe {
            AcquireSRWLockExclusive(&mut (*l).lock);
            let head = (*l).head;
            (*l).head = null_mut();
            (*l).tail = null_mut();
            ReleaseSRWLockExclusive(&mut (*l).lock);
            head
        };
        if !head.is_null() {
            unsafe { *out = head };
            return STATUS_SUCCESS;
        }
        let wait = if timeout_ms == u32::MAX {
            u32::MAX
        } else {
            let now = unsafe { GetTickCount64() };
            if now >= deadline {
                unsafe { *out = null_mut() };
                return STATUS_TIMEOUT;
            }
            (deadline - now) as u32
        };
        if unsafe { WaitForSingleObject((*l).event, wait) } != 0 && timeout_ms != u32::MAX {
            unsafe { *out = null_mut() };
            return STATUS_TIMEOUT;
        }
    }
}

pub fn get_next_ums_list_item(ctx: *mut u8) -> *mut u8 {
    if ctx.is_null() {
        return null_mut();
    }
    unsafe { a64(ctx, OFF_NEXT).swap(0, Ordering::SeqCst) as *mut u8 }
}

pub fn create_ums_thread_context(out: *mut *mut u8) -> Ntstatus {
    if out.is_null() {
        return STATUS_INVALID_PARAMETER;
    }
    unsafe {
        let ctx = VirtualAlloc(null_mut(), CTX_SIZE, 0x3000, 0x04) as *mut u8;
        if ctx.is_null() {
            return STATUS_NO_MEMORY;
        }
        let run = CreateEventW(null_mut(), 0, 0, core::ptr::null());
        let handoff = CreateEventW(null_mut(), 0, 0, core::ptr::null());
        if run.is_null() || handoff.is_null() {
            if !run.is_null() {
                CloseHandle(run);
            }
            if !handoff.is_null() {
                CloseHandle(handoff);
            }
            VirtualFree(ctx as *mut c_void, 0, 0x8000);
            return STATUS_NO_MEMORY;
        }
        wr(ctx, OFF_RUN_EVENT, run as u64);
        wr(ctx, OFF_HANDOFF_EVENT, handoff as u64);
        *out = ctx;
    }
    STATUS_SUCCESS
}

pub fn delete_ums_thread_context(ctx: *mut u8) -> Ntstatus {
    if ctx.is_null() {
        return STATUS_INVALID_PARAMETER;
    }
    unsafe {
        let flags = a32(ctx, OFF_FLAGS).load(Ordering::SeqCst);
        let thread = h(ctx, OFF_THREAD);
        if flags & FLAG_WORKER != 0 && !thread.is_null() {
            if flags & FLAG_TERMINATED == 0 {
                return STATUS_INVALID_PARAMETER;
            }
            WaitForSingleObject(thread, u32::MAX);
            CloseHandle(thread);
        }
        CloseHandle(h(ctx, OFF_RUN_EVENT));
        CloseHandle(h(ctx, OFF_HANDOFF_EVENT));
        VirtualFree(ctx as *mut c_void, 0, 0x8000);
    }
    STATUS_SUCCESS
}

pub fn query_ums_thread_information(ctx: *mut u8, class: u32, out: *mut u8, len: u32, ret: *mut u32) -> Ntstatus {
    if !ret.is_null() {
        unsafe { *ret = 0 };
    }
    if ctx.is_null() || out.is_null() {
        return STATUS_INVALID_PARAMETER;
    }
    unsafe {
        let flags = a32(ctx, OFF_FLAGS).load(Ordering::SeqCst);
        let (v, size): (u64, u32) = match class {
            1 => (rd(ctx, OFF_USER), 8),
            4 => (rd(ctx, OFF_TEB), 8),
            5 => (((flags >> 1) & 1) as u64, 1),
            6 => (((flags >> 3) & 1) as u64, 1),
            _ => return STATUS_NOT_SUPPORTED,
        };
        if len != size {
            return STATUS_INFO_LENGTH_MISMATCH;
        }
        if size == 8 {
            (out as *mut u64).write_unaligned(v);
        } else {
            *out = v as u8;
        }
        if !ret.is_null() {
            *ret = size;
        }
    }
    STATUS_SUCCESS
}

pub fn set_ums_thread_information(ctx: *mut u8, class: u32, data: *const u8, len: u32) -> Ntstatus {
    if ctx.is_null() || data.is_null() {
        return STATUS_INVALID_PARAMETER;
    }
    if class != 1 {
        return STATUS_NOT_SUPPORTED;
    }
    if len != 8 {
        return STATUS_INFO_LENGTH_MISMATCH;
    }
    unsafe { wr(ctx, OFF_USER, (data as *const u64).read_unaligned()) };
    STATUS_SUCCESS
}

pub fn create_ums_worker_thread(ctx: *mut u8, list: *mut CompletionList, start: WorkerStart, param: *mut c_void, stack: usize) -> Handle {
    if ctx.is_null() || list.is_null() {
        return null_mut();
    }
    tls();
    ensure_veh();
    unsafe {
        if a32(ctx, OFF_FLAGS).load(Ordering::SeqCst) & FLAG_WORKER != 0 || !h(ctx, OFF_THREAD).is_null() {
            return null_mut();
        }
        wr(ctx, OFF_LIST, list as u64);
        wr(ctx, OFF_START, start as usize as u64);
        wr(ctx, OFF_START_PARAM, param as u64);
        wr(ctx, OFF_LOCK, LOCK_IN_KERNEL);
        a32(ctx, OFF_STATE).store(ST_CREATED, Ordering::SeqCst);
        a32(ctx, OFF_FLAGS).fetch_or(FLAG_WORKER, Ordering::SeqCst);
        let mut tid = 0u32;
        let t = CreateThread(null_mut(), stack, worker_main, ctx as *mut c_void, 4, &mut tid);
        if t.is_null() {
            a32(ctx, OFF_FLAGS).fetch_and(!FLAG_WORKER, Ordering::SeqCst);
            return null_mut();
        }
        wr(ctx, OFF_THREAD, t as u64);
        wr(ctx, OFF_TID, tid as u64);
        ResumeThread(t);
        t
    }
}

extern "system" fn worker_main(p: *mut c_void) -> u32 {
    let ctx = p as *mut u8;
    set_current(ctx);
    unsafe {
        wr(ctx, OFF_TEB, teb());
        tbx_ums_park(ctx, null_mut(), initial_handoff);
        let start: WorkerStart = core::mem::transmute(rd(ctx, OFF_START) as usize);
        let code = start(rd(ctx, OFF_START_PARAM) as *mut c_void);
        terminate(ctx);
        code
    }
}

extern "system" fn initial_handoff(ctx: *mut u8, _param: *mut c_void) {
    unsafe {
        let list = rd(ctx, OFF_LIST) as *mut CompletionList;
        a32(ctx, OFF_STATE).store(ST_LISTED, Ordering::SeqCst);
        wr(ctx, OFF_LOCK, 0);
        list_push(list, ctx);
        WaitForSingleObject(h(ctx, OFF_RUN_EVENT), u32::MAX);
    }
}

unsafe fn hand_to_primary(worker: *mut u8, reason: u32, payload: u64, param: u64) {
    let prim = rd(worker, OFF_PREV) as *mut u8;
    if prim.is_null() {
        return;
    }
    wr(prim, OFF_HANDOFF_REASON, reason as u64);
    wr(prim, OFF_HANDOFF_PAYLOAD, payload);
    wr(prim, OFF_HANDOFF_PARAM, param);
    SetEvent(h(prim, OFF_HANDOFF_EVENT));
}

extern "system" fn yield_handoff(ctx: *mut u8, param: *mut c_void) {
    unsafe {
        a64(ctx, OFF_LOCK).fetch_and(3, Ordering::SeqCst);
        a32(ctx, OFF_STATE).store(ST_PARKED, Ordering::SeqCst);
        hand_to_primary(ctx, REASON_YIELD, ctx as u64, param as u64);
        WaitForSingleObject(h(ctx, OFF_RUN_EVENT), u32::MAX);
    }
}

unsafe fn terminate(ctx: *mut u8) {
    a32(ctx, OFF_FLAGS).fetch_or(FLAG_TERMINATED, Ordering::SeqCst);
    wr(ctx, OFF_LOCK, LOCK_IN_KERNEL);
    a32(ctx, OFF_STATE).store(ST_TERMINATED, Ordering::SeqCst);
    hand_to_primary(ctx, REASON_BLOCKED, 1, 0);
    list_push(rd(ctx, OFF_LIST) as *mut CompletionList, ctx);
}

extern "system" fn blocked_return_trap(p: *mut ExceptionPointers) -> i32 {
    unsafe {
        let rec = (*p).record;
        if *(rec as *const u32) != 0x8000_0004 {
            return 0;
        }
        let ctx = current_context_raw();
        if ctx.is_null() || a32(ctx, OFF_FLAGS).load(Ordering::SeqCst) & FLAG_WORKER == 0 {
            return 0;
        }
        let addr = *(rec.add(0x10) as *const u64);
        if a32(ctx, OFF_STATE).load(Ordering::SeqCst) != ST_BLOCKED || addr == 0 || rd(ctx, OFF_TRAP) != addr {
            return 0;
        }
        let c = (*p).context;
        *(c.add(CX_DR0) as *mut u64) = 0;
        *(c.add(CX_DR6) as *mut u64) = 0;
        let dr7 = *(c.add(CX_DR7) as *const u64) & !0x000f_0003u64;
        *(c.add(CX_DR7) as *mut u64) = if dr7 & 0xff == 0 { 0 } else { dr7 };
        wr(ctx, OFF_TRAP, 0);
        let mut d = Ctx::zeroed();
        d.set_u32(CX_FLAGS, CONTEXT_DEBUG_REGISTERS);
        d.set_u64(CX_DR7, *(c.add(CX_DR7) as *const u64));
        SetThreadContext(GetCurrentThread(), &d);
        core::ptr::copy_nonoverlapping(c, ctx.add(OFF_CONTEXT), CONTEXT_BYTES);
        a32(ctx, OFF_FLAGS).fetch_or(FLAG_FULL_CONTEXT, Ordering::SeqCst);
        a32(ctx, OFF_STATE).store(ST_LISTED, Ordering::SeqCst);
        wr(ctx, OFF_LOCK, 0);
        list_push(rd(ctx, OFF_LIST) as *mut CompletionList, ctx);
        WaitForSingleObject(h(ctx, OFF_RUN_EVENT), u32::MAX);
        core::ptr::copy_nonoverlapping(ctx.add(OFF_CONTEXT), c, CONTEXT_BYTES);
        *(c.add(CX_EFLAGS) as *mut u32) |= 0x1_0000;
        a32(ctx, OFF_FLAGS).fetch_and(!FLAG_FULL_CONTEXT, Ordering::SeqCst);
        -1
    }
}

fn fast_fail_invalid_set_of_context() -> ! {
    unsafe { core::arch::asm!("int 0x29", in("ecx") 0xdu32, options(noreturn, nostack)) }
}

unsafe fn check_stack(ctx: *mut u8) {
    let rsp = rd(ctx, OFF_CONTEXT + CX_RSP);
    let t = rd(ctx, OFF_TEB) as *const u64;
    if t.is_null() || rsp & 7 != 0 || rsp < *t.add(2) || rsp >= *t.add(1) {
        fast_fail_invalid_set_of_context();
    }
}

fn thread_state(buf: &mut Vec<u64>, tid: u64) -> Option<(u32, u32)> {
    if buf.is_empty() {
        buf.resize(64 * 1024, 0);
    }
    loop {
        let mut ret = 0u32;
        let st = unsafe { NtQuerySystemInformation(5, buf.as_mut_ptr() as *mut c_void, (buf.len() * 8) as u32, &mut ret) };
        if st == STATUS_INFO_LENGTH_MISMATCH {
            let want = (ret as usize / 8 + 4096).max(buf.len() * 2);
            buf.resize(want, 0);
            continue;
        }
        if st < 0 {
            return None;
        }
        break;
    }
    let base = buf.as_ptr() as *const u8;
    let pid = unsafe { GetCurrentProcessId() } as u64;
    let mut off = 0usize;
    unsafe {
        loop {
            let p = base.add(off);
            let next = *(p as *const u32) as usize;
            let n = *(p.add(4) as *const u32) as usize;
            if *(p.add(0x50) as *const u64) == pid {
                for i in 0..n {
                    let t = p.add(0x100 + i * 0x50);
                    if *(t.add(0x30) as *const u64) == tid {
                        return Some((*(t.add(0x44) as *const u32), *(t.add(0x48) as *const u32)));
                    }
                }
                return None;
            }
            if next == 0 {
                return None;
            }
            off += next;
        }
    }
}

unsafe fn try_park_blocked(target: *mut u8, buf: &mut Vec<u64>, last_cycles: &mut u64) -> bool {
    let thread = h(target, OFF_THREAD);
    if thread.is_null() || a32(target, OFF_STATE).load(Ordering::SeqCst) != ST_RUNNING {
        return false;
    }
    let mut cycles = 0u64;
    QueryThreadCycleTime(thread, &mut cycles);
    if cycles != *last_cycles {
        *last_cycles = cycles;
        return false;
    }
    match thread_state(buf, rd(target, OFF_TID)) {
        Some((5, _)) => {}
        _ => return false,
    }
    if SuspendThread(thread) == u32::MAX {
        return false;
    }
    let mut c = Ctx::zeroed();
    c.set_u32(CX_FLAGS, CONTEXT_CONTROL | CONTEXT_DEBUG_REGISTERS);
    let mut parked = false;
    if GetThreadContext(thread, &mut c) != 0 {
        let rip = c.u64(CX_RIP);
        if rip > 2 && *((rip - 2) as *const [u8; 2]) == [0x0f, 0x05] && a32(target, OFF_STATE).compare_exchange(ST_RUNNING, ST_BLOCKED, Ordering::SeqCst, Ordering::SeqCst).is_ok() {
            wr(target, OFF_TRAP, rip);
            let dr7 = (c.u64(CX_DR7) & !0x000f_0003u64) | 1;
            c.set_u32(CX_FLAGS, CONTEXT_DEBUG_REGISTERS);
            c.set_u64(CX_DR0, rip);
            c.set_u64(CX_DR7, dr7);
            if SetThreadContext(thread, &c) != 0 {
                wr(target, OFF_LOCK, (rd(target, OFF_LOCK) & 3) | LOCK_IN_KERNEL);
                parked = true;
            } else {
                wr(target, OFF_TRAP, 0);
                a32(target, OFF_STATE).store(ST_RUNNING, Ordering::SeqCst);
            }
        }
    }
    ResumeThread(thread);
    parked
}

unsafe fn run_until_handoff(cur: *mut u8, target: *mut u8) -> (u64, u64, u64) {
    let mut buf: Vec<u64> = Vec::new();
    let mut last_cycles = u64::MAX;
    a32(target, OFF_STATE).store(ST_RUNNING, Ordering::SeqCst);
    SetEvent(h(target, OFF_RUN_EVENT));
    loop {
        if WaitForSingleObject(h(cur, OFF_HANDOFF_EVENT), 1) == 0 {
            return (rd(cur, OFF_HANDOFF_REASON), rd(cur, OFF_HANDOFF_PAYLOAD), rd(cur, OFF_HANDOFF_PARAM));
        }
        if try_park_blocked(target, &mut buf, &mut last_cycles) {
            return (REASON_BLOCKED as u64, 1, 0);
        }
    }
}

#[no_mangle]
pub unsafe extern "system" fn RtlpExecuteUmsThread(target: *mut u8) -> Ntstatus {
    let cur = current_context_raw();
    if cur.is_null() {
        return STATUS_NOT_SUPPORTED;
    }
    if target.is_null() || h(target, OFF_RUN_EVENT).is_null() {
        return STATUS_INVALID_PARAMETER;
    }
    let owner = (GetCurrentThreadId() as u64) & !3;
    if a64(target, OFF_LOCK).compare_exchange(0, owner, Ordering::SeqCst, Ordering::SeqCst).is_err() {
        return STATUS_UMS_THREAD_NOT_READY;
    }
    wr(target, OFF_PREV, cur as u64);
    check_stack(target);
    let (reason, payload, param) = run_until_handoff(cur, target);
    tbx_ums_enter_primary(cur, reason, payload, param)
}

pub fn execute_ums_thread(target: *mut u8) -> Ntstatus {
    if target.is_null() {
        return STATUS_INVALID_PARAMETER;
    }
    let cur = current_context();
    if cur.is_null() {
        return STATUS_NOT_SUPPORTED;
    }
    unsafe {
        if a32(cur, OFF_FLAGS).load(Ordering::SeqCst) & FLAG_WORKER != 0 {
            return STATUS_NOT_SUPPORTED;
        }
        if a32(target, OFF_FLAGS).load(Ordering::SeqCst) & FLAG_WORKER == 0 {
            return STATUS_INVALID_PARAMETER;
        }
        a32(target, OFF_SWITCHES).fetch_add(1, Ordering::SeqCst);
        RtlpExecuteUmsThread(target);
        let flags = a32(target, OFF_FLAGS).load(Ordering::SeqCst);
        if rd(target, OFF_LOCK) & !3 != 0 {
            STATUS_INTERNAL_ERROR
        } else if flags & FLAG_TERMINATED != 0 || flags & FLAG_SUSPENDED != 0 {
            STATUS_UMS_THREAD_NOT_READY
        } else {
            STATUS_RETRY
        }
    }
}

pub fn ums_thread_yield(param: *mut c_void) -> Ntstatus {
    let cur = current_context();
    if cur.is_null() {
        return STATUS_INVALID_PARAMETER;
    }
    unsafe {
        if a32(cur, OFF_FLAGS).load(Ordering::SeqCst) & FLAG_WORKER == 0 {
            return STATUS_INVALID_PARAMETER;
        }
        a32(cur, OFF_FLAGS).fetch_or(FLAG_YIELDING, Ordering::SeqCst);
        a32(cur, OFF_UYIELDS).fetch_add(1, Ordering::SeqCst);
        RtlpUmsThreadYield(param);
        a32(cur, OFF_FLAGS).fetch_and(!FLAG_YIELDING, Ordering::SeqCst);
    }
    STATUS_SUCCESS
}

#[repr(C)]
pub struct SchedulerStartupInfo {
    pub version: u32,
    pub list: *mut CompletionList,
    pub proc_: Option<SchedulerProc>,
    pub param: *mut c_void,
}

pub fn enter_ums_scheduling_mode(info: *const SchedulerStartupInfo) -> Ntstatus {
    if info.is_null() {
        return STATUS_INVALID_PARAMETER;
    }
    let info = unsafe { &*info };
    let Some(proc_) = info.proc_ else { return STATUS_INVALID_PARAMETER };
    if info.version != 0x100 || info.list.is_null() {
        return STATUS_INVALID_PARAMETER;
    }
    tls();
    if !current_context().is_null() {
        return STATUS_NOT_SUPPORTED;
    }
    let mut prim = null_mut();
    let st = create_ums_thread_context(&mut prim);
    if st < 0 {
        return st;
    }
    unsafe {
        wr(prim, OFF_TEB, teb());
        wr(prim, OFF_TID, GetCurrentThreadId() as u64);
        wr(prim, OFF_LIST, info.list as u64);
        set_current(prim);
        tbx_ums_primary_wrap(proc_, info.param, prim);
        set_current(null_mut());
    }
    delete_ums_thread_context(prim)
}

#[cfg(test)]
mod tests;
