use super::runtime::{close_xmon, lock, open_xmon, pin_thread, pmc_busy, record, xbtp_traffic, Handle, CORE_PMC_GUID, E_ACCESSDENIED, E_INVALIDARG, E_UNEXPECTED, S_FALSE};
use super::{tool_alloc, tool_free, wide, xmem_fn, XMemGetWorkingSetStatistics, E_NOTIMPL, E_OUTOFMEMORY, S_OK};
use core::ffi::c_void;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

pub const SESSION_BYTES: usize = 0x220;
pub const XMON_BLOCK_BYTES: usize = 0x810;
pub const SUMMARY_THREAD_BYTES: usize = 0xe8;
pub const TENX_BYTES: usize = 0x30;
pub const UPCD_BYTES: usize = 0xc8;
pub const IPSD_BYTES: usize = 0x248;
pub const SAMPLE_STATE_BYTES: usize = 0x4ec;
pub const SAMPLE_MS: u32 = 100;
pub const FLUSH_TICKS: u32 = 10;
pub const POOL_GROW: usize = 0x10_0000;
pub const POOL_BLOCK: u32 = 0x1_0000;
pub const PACKET_TAG: u32 = 0x8000_0062;
pub const TAG_TENX: u32 = 0x584e_4554;
pub const TAG_UPCD: u32 = 0x4443_5055;
pub const TAG_IPSD: u32 = 0x4453_5049;
pub const F_UPCD: u32 = 0x01;
pub const F_SUMMARY: u32 = 0x02;
pub const F_IPSD: u32 = 0x04;
pub const F_PMC: u32 = 0x08;
pub const F_TSUC: u32 = 0x10;
pub const F_NET: u32 = 0x80;
pub const START_FLAGS: u32 = 0x8e;
pub const IOCTL_XMON_INSTALL: u32 = 0x8fff_204c;
pub const THREAD_NAME: &str = "PIX::GenericThread";
pub const PERCENT: f64 = 100.0;
pub const CPU_SLOTS: usize = 14;
pub const CPU_ENTRY: usize = 0x30;
pub const SYSTEM_BASIC_BYTES: u32 = 0x40;
pub const SYSTEM_PERFORMANCE_BYTES: u32 = 0x158;
pub const SYSTEM_PROCESSOR_BYTES: u32 = 0x2a0;
pub const WORKING_SET_FLAGS: u32 = 0x2000_0003;
pub const WORKING_SET_BYTES: usize = 0x78;
pub const FREE_PAGE_BYTES: usize = 0x38;
pub const WAIT_TIMEOUT: u32 = 0x102;

type ThreadProc = unsafe extern "system" fn(*mut c_void) -> u32;

#[link(name = "kernel32")]
extern "system" {
    fn CreateEventA(sa: *mut c_void, manual: i32, initial: i32, name: *const u8) -> Handle;
    fn CreateThread(sa: *mut c_void, stack: usize, f: ThreadProc, p: *mut c_void, flags: u32, id: *mut u32) -> Handle;
    fn WaitForSingleObject(h: Handle, ms: u32) -> u32;
    fn SetEvent(h: Handle) -> i32;
    fn CloseHandle(h: Handle) -> i32;
    fn GetLastError() -> u32;
    fn GetThreadId(h: Handle) -> u32;
    fn GetCurrentThreadId() -> u32;
    fn QueryPerformanceFrequency(f: *mut i64) -> i32;
    fn Sleep(ms: u32);
    fn GetModuleHandleW(n: *const u16) -> usize;
    fn GetProcAddress(m: usize, n: *const u8) -> usize;
    fn DeviceIoControl(h: Handle, code: u32, i: *mut c_void, il: u32, o: *mut c_void, ol: u32, ret: *mut u32, ov: *mut c_void) -> i32;
    fn OutputDebugStringA(s: *const u8);
}

#[link(name = "ntdll")]
extern "system" {
    fn NtQuerySystemInformation(class: u32, info: *mut c_void, len: u32, ret: *mut u32) -> i32;
}

const MSG_XMON_DEVICE: &[u8] = b"Error installing performance monitor configuration device.\n\0";

fn hr_from_win32(e: u32) -> i32 {
    if e as i32 <= 0 {
        e as i32
    } else {
        ((e & 0xffff) | 0x8007_0000) as i32
    }
}

pub fn nt_hresult(status: i32) -> i32 {
    match status as u32 {
        0 => S_OK,
        0xC000_0003 | 0xC000_0004 | 0xC000_000B => E_INVALIDARG,
        0xC000_0005 => E_ACCESSDENIED,
        0xC000_009A => E_UNEXPECTED,
        _ => 0x8000_4005u32 as i32,
    }
}

fn rdtsc() -> u64 {
    unsafe { core::arch::x86_64::_rdtsc() }
}

pub fn packet_header(dwords: u32, tag: u32, payload: u32, flags_or: u32, flags_mask: u32, old: u32) -> [u32; 5] {
    [PACKET_TAG, dwords, tag, payload, (old & flags_mask) | flags_or]
}

pub fn tenx_header() -> [u32; 5] {
    packet_header((TENX_BYTES / 4) as u32, TAG_TENX, 0x20, 1, 0xffff_0001, 0)
}

pub fn upcd_header() -> [u32; 5] {
    packet_header((UPCD_BYTES / 4) as u32, TAG_UPCD, 0xb8, 2, 0xffff_0002, 0)
}

pub fn ipsd_header() -> [u32; 5] {
    packet_header((IPSD_BYTES / 4) as u32, TAG_IPSD, 0x238, 0x402, 0xffff_0402, 0)
}

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct CpuTimes {
    pub idle: i64,
    pub kernel: i64,
    pub user: i64,
    pub dpc: i64,
    pub interrupt: i64,
}

impl CpuTimes {
    pub fn read(b: &[u8], i: usize) -> CpuTimes {
        let q = |o: usize| i64::from_le_bytes(b[i * CPU_ENTRY + o..i * CPU_ENTRY + o + 8].try_into().unwrap());
        CpuTimes { idle: q(0), kernel: q(8), user: q(0x10), dpc: q(0x18), interrupt: q(0x20) }
    }
}

pub fn cvtt32(v: f64) -> i32 {
    if v.is_nan() || v >= 2147483648.0 || v <= -2147483649.0 {
        i32::MIN
    } else {
        v as i32
    }
}

pub fn cpu_shares(cur: CpuTimes, prev: CpuTimes) -> (u16, u16) {
    let busy = cur.user.wrapping_sub(cur.idle).wrapping_sub(prev.user).wrapping_add(cur.kernel).wrapping_add(prev.idle).wrapping_sub(prev.kernel);
    let irq = cur.interrupt.wrapping_sub(prev.interrupt);
    if busy == 0 {
        return (0, 0);
    }
    let dpc = cur.dpc.wrapping_sub(prev.dpc);
    let a = cvtt32(dpc as f64 * PERCENT / busy as f64) as u16;
    let b = cvtt32(irq as f64 * PERCENT / busy as f64) as u16;
    (a, b)
}

#[repr(C, align(8))]
pub struct SampleState(pub [u8; 0x4f0]);

impl SampleState {
    pub fn new() -> Box<SampleState> {
        let mut s = Box::new(SampleState([0; 0x4f0]));
        let h = ipsd_header();
        for (i, v) in h[..4].iter().enumerate() {
            s.put32(i * 4, *v);
        }
        s.put32(0x10, h[4]);
        s
    }
    pub fn get32(&self, o: usize) -> u32 {
        u32::from_le_bytes(self.0[o..o + 4].try_into().unwrap())
    }
    pub fn get64(&self, o: usize) -> u64 {
        u64::from_le_bytes(self.0[o..o + 8].try_into().unwrap())
    }
    pub fn put32(&mut self, o: usize, v: u32) {
        self.0[o..o + 4].copy_from_slice(&v.to_le_bytes());
    }
    pub fn put64(&mut self, o: usize, v: u64) {
        self.0[o..o + 8].copy_from_slice(&v.to_le_bytes());
    }
    pub fn packet(&self) -> &[u8] {
        &self.0[..IPSD_BYTES]
    }
    pub fn samples(&self) -> u32 {
        self.get32(0x4e8)
    }
}

pub unsafe fn ipsd_sample(k: &mut SampleState) -> i32 {
    let mut perf = [0u8; SYSTEM_PERFORMANCE_BYTES as usize];
    let mut cpus = [0u8; SYSTEM_PROCESSOR_BYTES as usize];
    let mut ws = [0u64; WORKING_SET_BYTES / 8];
    let mut ret = 0u32;
    k.put32(0x4e8, k.get32(0x4e8).wrapping_add(1));
    if k.get32(0x20) == 0 {
        let mut basic = [0u8; SYSTEM_BASIC_BYTES as usize];
        let hr = nt_hresult(NtQuerySystemInformation(0, basic.as_mut_ptr().cast(), SYSTEM_BASIC_BYTES, &mut ret));
        if hr < 0 {
            return hr;
        }
        k.put32(0x20, u32::from_le_bytes(basic[8..12].try_into().unwrap()));
    }
    k.put64(0x18, rdtsc());
    let hr = nt_hresult(NtQuerySystemInformation(2, perf.as_mut_ptr().cast(), SYSTEM_PERFORMANCE_BYTES, &mut ret));
    if hr < 0 {
        return hr;
    }
    k.0[0x28..0x28 + SYSTEM_PERFORMANCE_BYTES as usize].copy_from_slice(&perf);
    let h = nt_hresult(NtQuerySystemInformation(8, cpus.as_mut_ptr().cast(), SYSTEM_PROCESSOR_BYTES, &mut ret));
    if h < 0 {
        return h;
    }
    if k.samples() > 1 {
        for i in 0..CPU_SLOTS {
            let prev = CpuTimes::read(&k.0[0x248..0x248 + SYSTEM_PROCESSOR_BYTES as usize], i);
            let cur = CpuTimes::read(&cpus, i);
            let (a, b) = cpu_shares(cur, prev);
            k.0[0x180 + 2 * i..0x182 + 2 * i].copy_from_slice(&a.to_le_bytes());
            k.0[0x19c + 2 * i..0x19e + 2 * i].copy_from_slice(&b.to_le_bytes());
        }
        k.0[0x248..0x248 + SYSTEM_PROCESSOR_BYTES as usize].copy_from_slice(&cpus);
    }
    if XMemGetWorkingSetStatistics(WORKING_SET_FLAGS, ws.as_mut_ptr().cast()) == 0 {
        return E_UNEXPECTED;
    }
    for (dst, src) in [(0x1b8, 0), (0x1c0, 1), (0x1c8, 2), (0x1d0, 3), (0x1d8, 5), (0x1e0, 6), (0x1e8, 7), (0x1f0, 8)] {
        k.put64(dst, ws[src]);
    }
    k.put64(0x1f8, (ws[4] >> 32) as u32 as u64);
    let mut fp = [0u64; FREE_PAGE_BYTES / 8];
    if let f @ 1.. = xmem_fn(b"XMemGetFreePageStatistics\0") {
        core::mem::transmute::<usize, unsafe extern "system" fn(*mut c_void) -> i32>(f)(fp.as_mut_ptr().cast());
    }
    for i in 0..6 {
        k.put64(0x200 + i * 8, fp[1 + i]);
    }
    k.put64(0x240, fp[0]);
    k.put64(0x230, ws[0xb]);
    k.put64(0x238, ws[0xa] as u32 as u64);
    h
}

#[derive(Default)]
struct PoolState {
    cur: usize,
    used: u32,
    block: u32,
    empty: bool,
    base: usize,
    size: usize,
    committed: usize,
}

struct SessionPool {
    state: Mutex<PoolState>,
    event: Handle,
}

impl SessionPool {
    unsafe fn new() -> SessionPool {
        SessionPool { state: Mutex::new(PoolState::default()), event: CreateEventA(core::ptr::null_mut(), 0, 0, core::ptr::null()) }
    }

    fn acquire(st: &mut PoolState) -> usize {
        if st.committed + POOL_GROW > st.size {
            return 0;
        }
        record("session pool", "carving a 0x100000 grow step out of title-supplied tooling memory (0x26000 with rt+0xd40 != 0) is not ported; rt+0xd40 is only written by the runtime destructor 0x18710");
        0
    }

    fn setup(&self, base: usize, size: usize) -> i32 {
        let mut st = lock(&self.state);
        if !(st.base == base && st.size == size) {
            st.base = base;
            st.size = size;
        }
        st.cur = Self::acquire(&mut st);
        st.block = POOL_BLOCK;
        st.empty = false;
        st.used = 0;
        S_OK
    }

    fn write(&self, src: &[u8]) -> i32 {
        let mut st = lock(&self.state);
        if st.cur == 0 || src.len() as u64 > (st.block as u64).saturating_sub(st.used as u64) {
            return E_OUTOFMEMORY;
        }
        unsafe { core::ptr::copy_nonoverlapping(src.as_ptr(), (st.cur + st.used as usize) as *mut u8, src.len()) };
        st.used += src.len() as u32;
        S_OK
    }

    fn reset_used(&self) {
        lock(&self.state).used = 0;
    }

    fn rotate(&self) -> (usize, u32) {
        let mut st = lock(&self.state);
        let (cur, used) = (st.cur, st.used);
        st.cur = Self::acquire(&mut st);
        if st.cur == 0 {
            if !st.empty {
                st.empty = true;
            }
        } else {
            st.empty = false;
        }
        st.used = 0;
        (cur, used)
    }
}

impl Drop for SessionPool {
    fn drop(&mut self) {
        if self.event != 0 {
            unsafe { CloseHandle(self.event) };
        }
    }
}

#[derive(Default)]
struct ThreadSlot {
    stop: Handle,
    thread: Handle,
}

pub struct Session {
    blocks: [AtomicUsize; 4],
    flags: AtomicU32,
    ticks: AtomicU32,
    lock: Mutex<()>,
    thread: Mutex<ThreadSlot>,
    pool: SessionPool,
    flush_bound: AtomicU32,
    half_qpf: i64,
}

static SESSION: Mutex<Option<&'static Session>> = Mutex::new(None);

pub fn session() -> Option<&'static Session> {
    *lock(&SESSION)
}

fn set_description() -> Option<unsafe extern "system" fn(Handle, *const u16) -> i32> {
    static F: OnceLock<usize> = OnceLock::new();
    let f = *F.get_or_init(|| unsafe {
        let m = GetModuleHandleW(wide("kernel32.dll").as_ptr());
        let s = GetProcAddress(m, b"SetThreadDescription\0".as_ptr());
        let g = GetProcAddress(m, b"GetThreadDescription\0".as_ptr());
        if s != 0 && g != 0 {
            s
        } else {
            0
        }
    });
    (f != 0).then(|| unsafe { core::mem::transmute::<usize, unsafe extern "system" fn(Handle, *const u16) -> i32>(f) })
}

unsafe extern "system" fn session_thread(p: *mut c_void) -> u32 {
    (*(p as *const Session)).run()
}

unsafe fn xmon_install(guid: [u32; 4]) -> Result<u64, i32> {
    if pmc_busy() {
        return Err(E_ACCESSDENIED);
    }
    let h = open_xmon();
    if h == 0 {
        OutputDebugStringA(MSG_XMON_DEVICE.as_ptr());
        return Err(E_ACCESSDENIED);
    }
    let mut inb = [0u32; 9];
    inb[0] = 1;
    inb[1..5].copy_from_slice(&guid);
    let mut out = 0u64;
    let mut ret = 0u32;
    let ok = DeviceIoControl(h, IOCTL_XMON_INSTALL, inb.as_mut_ptr().cast(), 0x24, (&mut out as *mut u64).cast(), 8, &mut ret, core::ptr::null_mut()) != 0;
    let got = if ret != 0 { out } else { 0 };
    close_xmon(h);
    if ok {
        Ok(got)
    } else {
        Err(E_ACCESSDENIED)
    }
}

impl Session {
    unsafe fn create() -> Option<&'static Session> {
        let b = tool_alloc(SESSION_BYTES) as usize;
        if b == 0 {
            return None;
        }
        core::ptr::write_bytes(b as *mut u8, 0, SESSION_BYTES);
        let mut f = 0i64;
        QueryPerformanceFrequency(&mut f);
        let s = Box::leak(Box::new(Session {
            blocks: [AtomicUsize::new(b), AtomicUsize::new(0), AtomicUsize::new(0), AtomicUsize::new(0)],
            flags: AtomicU32::new(0),
            ticks: AtomicU32::new(0),
            lock: Mutex::new(()),
            thread: Mutex::new(ThreadSlot::default()),
            pool: SessionPool::new(),
            flush_bound: AtomicU32::new(0),
            half_qpf: f / 2,
        }));
        Some(s)
    }

    unsafe fn init(&self, base: usize, size: usize) -> i32 {
        for (i, n) in [(1, XMON_BLOCK_BYTES), (2, SUMMARY_THREAD_BYTES), (3, TENX_BYTES)] {
            let p = tool_alloc(n) as usize;
            if p != 0 {
                core::ptr::write_bytes(p as *mut u8, 0, n);
                if i == 3 {
                    let h = tenx_header();
                    core::ptr::copy_nonoverlapping(h.as_ptr() as *const u8, p as *mut u8, 0x14);
                }
            }
            self.blocks[i].store(p, Ordering::SeqCst);
            if p == 0 {
                return E_OUTOFMEMORY;
            }
        }
        self.pool.setup(base, size)
    }

    pub fn flags(&self) -> u32 {
        self.flags.load(Ordering::SeqCst)
    }

    pub fn ticks(&self) -> u32 {
        self.ticks.load(Ordering::SeqCst)
    }

    pub fn half_qpf(&self) -> i64 {
        self.half_qpf
    }

    pub fn thread_handle(&self) -> Handle {
        lock(&self.thread).thread
    }

    pub fn block(&self, i: usize) -> usize {
        self.blocks[i].load(Ordering::SeqCst)
    }

    pub fn tenx(&self) -> [u8; TENX_BYTES] {
        let mut t = [0u8; TENX_BYTES];
        let p = self.block(3);
        if p != 0 {
            unsafe { core::ptr::copy_nonoverlapping(p as *const u8, t.as_mut_ptr(), TENX_BYTES) };
        }
        t
    }

    unsafe fn pmc_enable(&self) -> i32 {
        let p = (self.block(1) + 0x20) as *mut u32;
        *p = (*p & 0xffff_0102) | 0x102;
        match xmon_install(CORE_PMC_GUID) {
            Err(hr) => hr,
            Ok(_) => {
                record("session PMC", "xmondrv accepted IOCTL 0x8fff204c; the counter-table read 0x44f3c (IOCTL 0x8fff2048), 0x44c30 (0x8fff2020), 0x4560c, 0x4550c and the D:\\dfcounter0..3 probes after it are not ported");
                E_NOTIMPL
            }
        }
    }

    unsafe fn thread_start(&self) -> i32 {
        let mut t = lock(&self.thread);
        if t.thread != 0 && WaitForSingleObject(t.thread, 0) == WAIT_TIMEOUT {
            return E_UNEXPECTED;
        }
        let e = CreateEventA(core::ptr::null_mut(), 1, 0, core::ptr::null());
        if e != t.stop {
            if t.stop != 0 {
                CloseHandle(t.stop);
            }
            t.stop = e;
        }
        if t.stop == 0 {
            let hr = hr_from_win32(GetLastError());
            if hr < 0 {
                return hr;
            }
        }
        let mut tid = 0u32;
        let h = CreateThread(core::ptr::null_mut(), 0, session_thread, self as *const Session as *mut c_void, 0, &mut tid);
        if h != t.thread {
            if t.thread != 0 {
                CloseHandle(t.thread);
            }
            t.thread = h;
        }
        if let Some(f) = set_description() {
            f(t.thread, wide(THREAD_NAME).as_ptr());
        }
        S_OK
    }

    pub unsafe fn thread_stop(&self) {
        let t = lock(&self.thread);
        if t.thread != 0 && WaitForSingleObject(t.thread, 0) == WAIT_TIMEOUT {
            let own = GetThreadId(t.thread) == GetCurrentThreadId();
            SetEvent(t.stop);
            let h = t.thread;
            drop(t);
            if !own {
                WaitForSingleObject(h, u32::MAX);
            }
        }
    }

    pub unsafe fn set_flags(&self, want: u32, out: Option<&mut u32>) -> i32 {
        let _g = lock(&self.lock);
        let old = self.flags.load(Ordering::SeqCst);
        if old == want {
            return S_FALSE;
        }
        if old == 0 && want != 0 {
            self.pool.reset_used();
        }
        let mut v = old;
        v = if want & F_SUMMARY != 0 { v | F_SUMMARY } else { v & !F_SUMMARY };
        v = if want & F_IPSD != 0 { v | F_IPSD } else { v & !F_IPSD };
        self.flags.store(v, Ordering::SeqCst);
        if old & F_PMC == 0 && want & F_PMC != 0 && self.pmc_enable() >= 0 {
            self.flags.fetch_or(F_PMC, Ordering::SeqCst);
        }
        if old & F_PMC != 0 && want & F_PMC == 0 {
            self.flags.fetch_and(!F_PMC, Ordering::SeqCst);
        }
        if old & F_TSUC == 0 && want & F_TSUC != 0 {
            record("session flags", "bit 0x10 (the TSUC/TADE/WTE summary thread 0x33f40 on the 0xe8 block) is not ported");
        }
        if want & F_NET != 0 {
            self.flags.fetch_or(F_NET, Ordering::SeqCst);
        } else {
            self.flags.fetch_and(!F_NET, Ordering::SeqCst);
        }
        let mut r = S_OK;
        if self.flags.load(Ordering::SeqCst) != 0 {
            let th = lock(&self.thread).thread;
            if th == 0 || WaitForSingleObject(th, 0) != WAIT_TIMEOUT {
                r = self.thread_start();
            }
        }
        if let Some(o) = out {
            *o = self.flags.load(Ordering::SeqCst);
        }
        r
    }

    fn send(&self, pkt: &[u8]) -> i32 {
        let mut hr = self.pool.write(pkt);
        if hr == E_OUTOFMEMORY && self.flush(true) >= 0 {
            hr = self.pool.write(pkt);
        }
        hr
    }

    fn flush(&self, force: bool) -> i32 {
        let t = self.ticks.fetch_add(1, Ordering::SeqCst) + 1;
        if t < FLUSH_TICKS && !force {
            return S_OK;
        }
        let (_cur, used) = self.pool.rotate();
        if used != 0 && self.flush_bound.load(Ordering::SeqCst) != 0 {
            record("session flush", "delivering a filled session block through 0x28cb0 to the capture backend vtbl[2] is not ported");
            self.ticks.store(0, Ordering::SeqCst);
        }
        S_OK
    }

    unsafe fn run(&self) -> u32 {
        pin_thread();
        let mut upcd = [0u8; UPCD_BYTES];
        for (i, v) in upcd_header().iter().enumerate() {
            upcd[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
        }
        let mut k = SampleState::new();
        let mut prev = u32::MAX;
        let mut r = 0u32;
        loop {
            Sleep(SAMPLE_MS);
            let f = self.flags.load(Ordering::SeqCst);
            if f != prev && f == 0 {
                break;
            }
            let keep = |hr: i32| if hr < 0 { 0 } else { hr as u32 };
            if f & F_UPCD != 0 {
                r = keep(self.send(&upcd));
            }
            if f & F_IPSD != 0 && ipsd_sample(&mut k) >= 0 {
                r = keep(self.send(k.packet()));
            }
            if f & F_NET != 0 {
                let p = self.block(3);
                if p != 0 {
                    *((p + 0x18) as *mut u64) = rdtsc();
                    let ok = match xbtp_traffic() {
                        Some(g) => g((p + 0x20) as *mut u64, (p + 0x28) as *mut u64) >= 0,
                        None => false,
                    };
                    if ok {
                        let pkt = core::slice::from_raw_parts(p as *const u8, TENX_BYTES);
                        r = keep(self.send(pkt));
                    }
                }
            }
            self.flush(false);
            prev = f;
        }
        r
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        unsafe {
            self.thread_stop();
            for b in &self.blocks {
                let p = b.swap(0, Ordering::SeqCst);
                if p != 0 {
                    tool_free(p as *mut c_void);
                }
            }
            let t = lock(&self.thread);
            if t.stop != 0 {
                CloseHandle(t.stop);
            }
            if t.thread != 0 {
                CloseHandle(t.thread);
            }
        }
    }
}

pub fn tool_memory() -> (usize, usize) {
    (0, 0)
}

pub unsafe fn ensure(want: u32) -> i32 {
    let s = {
        let mut g = lock(&SESSION);
        match *g {
            Some(s) => s,
            None => {
                let (base, size) = tool_memory();
                let Some(s) = Session::create() else { return E_OUTOFMEMORY };
                *g = Some(s);
                let hr = s.init(base, size);
                if hr < 0 {
                    return hr;
                }
                let _l = lock(&s.lock);
                s.flush_bound.store(1, Ordering::SeqCst);
                s
            }
        }
    };
    let mut out = 0u32;
    s.set_flags(want, Some(&mut out))
}

pub unsafe fn end() -> i32 {
    if let Some(s) = session() {
        let mut tmp = 0u32;
        s.set_flags(0, Some(&mut tmp));
        s.thread_stop();
    }
    S_OK
}

pub unsafe fn destroy() {
    end();
    let s = lock(&SESSION).take();
    if let Some(s) = s {
        drop(Box::from_raw(s as *const Session as *mut Session));
    }
}
