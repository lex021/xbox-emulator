use core::ffi::c_void;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use compat_common::record;

static TRACE_NATIVE_AV: AtomicBool = AtomicBool::new(false);
static DESC_GUARD_PAGE: AtomicUsize = AtomicUsize::new(0);
static DESC_GUARD_USED: AtomicBool = AtomicBool::new(false);

extern "system" {
    fn VirtualProtect(address: *mut c_void, size: usize, protect: u32, old: *mut u32) -> i32;
}

/// One diagnostic guard-page access. Windows removes PAGE_GUARD on the first
/// access, so reads can consume it before a write. Never rearm in the handler.
pub(crate) unsafe fn arm_descriptor_guard(cpu: usize) {
    if std::env::var_os("TOMBOX_TEST_DESCRIPTOR_GUARD").is_none() || DESC_GUARD_USED.swap(true, Ordering::AcqRel) { return; }
    static INSTALL: std::sync::Once = std::sync::Once::new();
    INSTALL.call_once(|| unsafe { AddVectoredExceptionHandler(1, descriptor_guard_exception); });
    let page = cpu & !0xfff;
    DESC_GUARD_PAGE.store(page, Ordering::Release);
    let mut old = 0;
    let ok = unsafe { VirtualProtect(page as *mut c_void, 4096, 0x104, &mut old) };
    if ok == 0 { DESC_GUARD_PAGE.store(0, Ordering::Release); }
    record("d3d12_xs", "descriptor_guard", &format!("one-shot page={page:#x} target={cpu:#x} armed={} old_protect={old:#x}; first access may be a read", ok != 0));
}

unsafe extern "system" fn descriptor_guard_exception(info: *const [usize; 2]) -> i32 {
    if info.is_null() { return 0; }
    let [exception, context] = unsafe { *info };
    if exception == 0 || context == 0 { return 0; }
    let fields = exception as *const usize;
    if unsafe { *fields as u32 } != 0x8000_0001 { return 0; }
    let address = unsafe { *fields.add(2) };
    let action = unsafe { *fields.add(4) };
    let target = unsafe { *fields.add(5) };
    let page = DESC_GUARD_PAGE.load(Ordering::Acquire);
    if page == 0 || target & !0xfff != page || DESC_GUARD_PAGE.compare_exchange(page, 0, Ordering::AcqRel, Ordering::Acquire).is_err() { return 0; }
    let msg = format!("DescriptorGuard first-access rip={address:#x} target={target:#x} action={action} (0=read, 1=write, 8=execute); guard consumed\n");
    let mut written = 0;
    unsafe { WriteFile(GetStdHandle((-12i32) as u32), msg.as_ptr(), msg.len() as u32, &mut written, core::ptr::null_mut()) };
    let stack = unsafe { exception_stack(context, address) };
    unsafe { WriteFile(GetStdHandle((-12i32) as u32), stack.as_ptr(), stack.len() as u32, &mut written, core::ptr::null_mut()) };
    -1
}
thread_local! {
    static DESCRIPTOR_COPY: core::cell::Cell<Option<[usize; 6]>> = const { core::cell::Cell::new(None) };
}

pub(crate) fn trace_descriptor_copy(n: u32, dst: usize, src: usize, host_dst: usize, host_src: usize, ty: u32) {
    if TRACE_NATIVE_AV.load(Ordering::Relaxed) {
        DESCRIPTOR_COPY.set(Some([n as usize, dst, src, host_dst, host_src, ty as usize]));
    }
}

pub(crate) fn clear_descriptor_copy() {
    if TRACE_NATIVE_AV.load(Ordering::Relaxed) {
        DESCRIPTOR_COPY.set(None);
    }
}

pub fn install_native_exception_log() {
    if std::env::var_os("TOMBOX_SCARLETT_NATIVE_AV").is_none() {
        return;
    }
    TRACE_NATIVE_AV.store(true, Ordering::Relaxed);
    static INSTALL: std::sync::Once = std::sync::Once::new();
    INSTALL.call_once(|| unsafe {
        AddVectoredExceptionHandler(1, log_native_access_violation);
    });
}

extern "system" {
    fn AddVectoredExceptionHandler(first: u32, handler: unsafe extern "system" fn(*const [usize; 2]) -> i32) -> *mut c_void;
    fn GetModuleHandleExW(flags: u32, address: *const u16, output: *mut usize) -> i32;
    fn GetModuleFileNameW(module: usize, name: *mut u16, capacity: u32) -> u32;
    fn GetStdHandle(kind: u32) -> *mut c_void;
    fn WriteFile(handle: *mut c_void, data: *const u8, size: u32, written: *mut u32, overlapped: *mut c_void) -> i32;
}

#[repr(C, align(16))]
struct X64Context([u64; 154]);

extern "system" {
    fn GetCurrentThreadId() -> u32;
    fn GetCurrentThreadStackLimits(low: *mut usize, high: *mut usize);
    fn RtlLookupFunctionEntry(pc: u64, base: *mut u64, history: *mut c_void) -> *const c_void;
    fn RtlVirtualUnwind(kind: u32, base: u64, pc: u64, entry: *const c_void,
        context: *mut X64Context, data: *mut *mut c_void, frame: *mut u64,
        pointers: *mut c_void) -> *mut c_void;
}

unsafe fn exception_stack(original: usize, first_pc: usize) -> String {
    use std::fmt::Write;
    const RSP: usize = 0x98 / 8;
    const RIP: usize = 0xf8 / 8;
    let mut context = X64Context([0; 154]);
    unsafe { core::ptr::copy_nonoverlapping(original as *const u8,
        context.0.as_mut_ptr().cast(), core::mem::size_of::<X64Context>()) };
    let (mut low, mut high) = (0, 0);
    unsafe { GetCurrentThreadStackLimits(&mut low, &mut high) };
    let tid = unsafe { GetCurrentThreadId() };
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis());
    let mut result = format!("ScarlettNativeStack time={now} tid={tid} fault={first_pc:#x}\n");
    for n in 0..24 {
        let pc = context.0[RIP];
        let sp = context.0[RSP];
        if pc == 0 || (sp as usize) < low || (sp as usize) > high.saturating_sub(8) {
            break;
        }
        let mut module = 0usize;
        let mut path = [0u16; 512];
        let len = if unsafe { GetModuleHandleExW(6, pc as *const u16, &mut module) } != 0 {
            unsafe { GetModuleFileNameW(module, path.as_mut_ptr(), path.len() as u32) as usize }
        } else { 0 };
        let name = String::from_utf16_lossy(&path[..len.min(path.len())]);
        let name = name.rsplit([char::from(92), '/']).next().unwrap_or_default();
        let _ = writeln!(result, "  {n:02} {name}+{:#x} pc={pc:#x} sp={sp:#x}", pc.wrapping_sub(module as u64));
        let mut base = 0;
        let lookup_pc = pc - u64::from(n != 0);
        let entry = unsafe { RtlLookupFunctionEntry(lookup_pc, &mut base, core::ptr::null_mut()) };
        if entry.is_null() {
            if module == 0 { break; }
            context.0[RIP] = unsafe { (sp as *const u64).read_unaligned() };
            context.0[RSP] = sp + 8;
        } else {
            let (mut data, mut frame) = (core::ptr::null_mut(), 0);
            unsafe { RtlVirtualUnwind(0, base, lookup_pc, entry, &mut context,
                &mut data, &mut frame, core::ptr::null_mut()) };
        }
        if context.0[RSP] <= sp { break; }
    }
    result
}

unsafe extern "system" fn log_native_access_violation(info: *const [usize; 2]) -> i32 {
    let [exception, context] = unsafe { *info };
    let fields = exception as *const usize;
    if exception == 0 || context == 0 || unsafe { *fields as u32 } != 0xc000_0005 {
        return 0;
    }
    static COUNT: AtomicUsize = AtomicUsize::new(0);
    if COUNT.fetch_add(1, Ordering::Relaxed) >= 16 {
        return 0;
    }
    let address = unsafe { *fields.add(2) };
    let action = unsafe { *fields.add(4) };
    let target = unsafe { *fields.add(5) };
    let mut module = 0usize;
    let mut path = [0u16; 512];
    if unsafe { GetModuleHandleExW(6, address as *const u16, &mut module) } != 0 {
        unsafe { GetModuleFileNameW(module, path.as_mut_ptr(), path.len() as u32) };
    }
    let end = path.iter().position(|&word| word == 0).unwrap_or(path.len());
    let name = String::from_utf16_lossy(&path[..end]);
    let registers = [0x78, 0x80, 0x88, 0x90, 0x98, 0xa8, 0xb0].map(|off| {
        unsafe { *((context + off) as *const u64) }
    });
    let message = format!(
        "ScarlettNativeAV {name}+{:#x} at {address:#x} kind={action} target={target:#x} rax rcx rdx rbx rsp rsi rdi={registers:x?}\n",
        address.wrapping_sub(module),
    );
    let mut written = 0;
    unsafe { WriteFile(GetStdHandle((-12i32) as u32), message.as_ptr(), message.len() as u32, &mut written, core::ptr::null_mut()) };
    if let Ok(Some(args)) = DESCRIPTOR_COPY.try_with(|copy| copy.get()) {
        let message = format!("ScarlettDescriptorCopy [count, title dst, title src, host dst, host src, type] = {args:x?}\n");
        unsafe { WriteFile(GetStdHandle((-12i32) as u32), message.as_ptr(), message.len() as u32, &mut written, core::ptr::null_mut()) };
    }
    let stack = unsafe { exception_stack(context, address) };
    unsafe { WriteFile(GetStdHandle((-12i32) as u32), stack.as_ptr(), stack.len() as u32, &mut written, core::ptr::null_mut()) };
    0
}

pub const SET_MASK: u32 = 0x83f0_0001;
pub const SET_MASK_19041: u32 = 0x81f0_0001;
pub const NEW_ABI_BUILD: u32 = 22000;

pub fn console_os_build() -> u32 {
    compat_common::os_host::k32legacy::console_os_build()
}

pub fn set_mask_for(os_build: u32) -> u32 {
    if os_build < NEW_ABI_BUILD { SET_MASK_19041 } else { SET_MASK }
}
pub const FLAG_DEBUG: u32 = 0x0000_0001;
pub const FLAG_INSTRUMENTED: u32 = 0x0010_0000;
pub const FLAG_DEBUG_STOCK: u32 = 0x8000_0000;
const PARAMS_FLAGS_OFFSET: usize = 4;

static FLAGS: AtomicU32 = AtomicU32::new(0);
static DEVICE_CREATED: AtomicBool = AtomicBool::new(false);
static LAYER_ON: AtomicBool = AtomicBool::new(false);
static LOGS: AtomicUsize = AtomicUsize::new(0);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Driver {
    Stock,
    Instrumented,
    Debug,
    StockWithDebugFlag,
}

impl Driver {
    pub fn console_umd(self) -> &'static str {
        match self {
            Driver::Debug => "xgs12_d.dll",
            Driver::Instrumented => "xgs12_i.dll",
            Driver::Stock | Driver::StockWithDebugFlag => "xgs12.dll",
        }
    }

    pub fn pc_debug_layer(self) -> bool {
        self == Driver::Debug
    }
}

pub fn driver_for(flags: u32) -> Driver {
    if flags & FLAG_DEBUG != 0 {
        if flags & FLAG_DEBUG_STOCK != 0 { Driver::StockWithDebugFlag } else { Driver::Debug }
    } else if flags & FLAG_INSTRUMENTED != 0 {
        Driver::Instrumented
    } else {
        Driver::Stock
    }
}

pub fn current() -> u32 {
    FLAGS.load(Ordering::SeqCst)
}

pub fn store_set(flags: u32) -> u32 {
    store_set_for(flags, console_os_build())
}

pub fn store_set_for(flags: u32, os_build: u32) -> u32 {
    let v = flags & set_mask_for(os_build);
    FLAGS.store(v, Ordering::SeqCst);
    v
}

pub unsafe fn store_create_params(params: *const c_void) -> u32 {
    if params.is_null() {
        return current();
    }
    let v = unsafe { core::ptr::read_unaligned((params as *const u8).add(PARAMS_FLAGS_OFFSET) as *const u32) };
    FLAGS.store(v, Ordering::SeqCst);
    v
}

fn log(func: &str, msg: &str) {
    if LOGS.fetch_add(1, Ordering::Relaxed) < 32 {
        record("d3d12_xs", func, msg);
    }
}

pub fn wants_debug_layer(flags: u32, env_forced: bool) -> bool {
    env_forced || driver_for(flags).pc_debug_layer()
}

pub fn claim_debug_layer() -> bool {
    if DEVICE_CREATED.load(Ordering::SeqCst) {
        return false;
    }
    !LAYER_ON.swap(true, Ordering::SeqCst)
}

pub fn debug_layer_enabled() -> bool {
    LAYER_ON.load(Ordering::SeqCst)
}

pub fn mark_device_created() {
    DEVICE_CREATED.store(true, Ordering::SeqCst);
}

pub unsafe fn on_create_device(params: *const c_void, env_forced: bool) -> (Driver, bool) {
    let flags = unsafe { store_create_params(params) };
    let d = driver_for(flags);
    let want = wants_debug_layer(flags, env_forced);
    let claimed = want && claim_debug_layer();
    let already = DEVICE_CREATED.load(Ordering::SeqCst);
    log(
        "D3D12XboxCreateDevice",
        &format!(
            "params={:#x} ProcessDebugFlags={flags:#010x} console UMD={} ({d:?}) pc debug layer requested={want} enable_now={claimed} device_already_created={already}",
            params as usize,
            d.console_umd()
        ),
    );
    (d, claimed)
}

/// Route high-severity debug-layer messages to the compatibility log. Suppress
/// repeated startup shader diagnostics in this callback to keep GPU traces short.
pub(crate) unsafe fn capture_messages(device: *mut c_void) {
    use crate::device_wrap::Guid;

    if std::env::var_os("TOMBOX_D3D12_DEBUG_CAPTURE").is_none() {
        return;
    }
    const IID_INFO_QUEUE1: Guid = Guid {
        d1: 0x2852_dd88, d2: 0xb484, d3: 0x4c0c,
        d4: [0xb6, 0xb1, 0x67, 0x16, 0x85, 0x00, 0xe6, 0x00],
    };
    type MessageCallback = unsafe extern "system" fn(u32, u32, u32, *const u8, *mut c_void);
    unsafe extern "system" fn message(
        _category: u32, severity: u32, id: u32, desc: *const u8, _context: *mut c_void,
    ) {
        static SHADER_MISMATCHES: AtomicUsize = AtomicUsize::new(0);
        if severity > 1 || desc.is_null() || (id == 688 && SHADER_MISMATCHES.fetch_add(1, Ordering::Relaxed) >= 4) {
            return;
        }
        let message = unsafe { std::ffi::CStr::from_ptr(desc.cast()) }.to_string_lossy();
        record("d3d12_xs", "PcD3D12Validation", &format!("#{id}: {message}"));
    }

    let qi: unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> i32 =
        unsafe { core::mem::transmute(**device.cast::<*const usize>()) };
    let mut info = core::ptr::null_mut();
    let hr = unsafe { qi(device, &IID_INFO_QUEUE1, &mut info) };
    if hr < 0 || info.is_null() {
        record("d3d12_xs", "PcD3D12Validation", &format!("InfoQueue1 unavailable hr={hr:#010x}"));
        return;
    }
    let vt = unsafe { *info.cast::<*const usize>() };
    let register: unsafe extern "system" fn(*mut c_void, MessageCallback, u32, *mut c_void, *mut u32) -> i32 =
        unsafe { core::mem::transmute(*vt.add(38)) };
    let mute: unsafe extern "system" fn(*mut c_void, i32) =
        unsafe { core::mem::transmute(*vt.add(36)) };
    let release: unsafe extern "system" fn(*mut c_void) -> u32 =
        unsafe { core::mem::transmute(*vt.add(2)) };
    let mut cookie = 0;
    const IGNORE_DEBUG_FILTERS: u32 = 1;
    let hr = unsafe { register(info, message, IGNORE_DEBUG_FILTERS, core::ptr::null_mut(), &mut cookie) };
    if hr >= 0 {
        unsafe { mute(info, 1) };
    }
    unsafe { release(info) };
    record("d3d12_xs", "PcD3D12Validation", &format!("logging callback hr={hr:#010x}"));
}

#[no_mangle]
pub extern "system" fn D3D12XboxSetProcessDebugFlags(flags: u32) -> i32 {
    let v = store_set(flags);
    let d = driver_for(v);
    log(
        "D3D12XboxSetProcessDebugFlags",
        &format!("flags={flags:#010x} stored={v:#010x} (mask {:#x}) next device: console UMD={} pc debug layer={} -> S_OK", set_mask_for(console_os_build()), d.console_umd(), d.pc_debug_layer()),
    );
    0
}

#[no_mangle]
pub extern "system" fn D3D12XboxGetProcessDebugFlags() -> u32 {
    current()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static T: Mutex<()> = Mutex::new(());

    #[test]
    fn set_masks_and_get_returns_stored_value() {
        let _g = T.lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(D3D12XboxSetProcessDebugFlags(0xffff_ffff), 0);
        assert_eq!(D3D12XboxGetProcessDebugFlags(), 0x83f0_0001);
        assert_eq!(D3D12XboxSetProcessDebugFlags(0x0000_0201), 0);
        assert_eq!(D3D12XboxGetProcessDebugFlags(), 0x0000_0001);
        assert_eq!(D3D12XboxSetProcessDebugFlags(0x0010_0000), 0);
        assert_eq!(D3D12XboxGetProcessDebugFlags(), 0x0010_0000);
        assert_eq!(D3D12XboxSetProcessDebugFlags(0), 0);
        assert_eq!(D3D12XboxGetProcessDebugFlags(), 0);
    }

    #[test]
    fn create_params_overwrite_global_unmasked() {
        let _g = T.lock().unwrap_or_else(|e| e.into_inner());
        let mut params = [0u32; 16];
        params[1] = 0x0000_0e00;
        store_set(0x0010_0001);
        assert_eq!(unsafe { store_create_params(params.as_ptr() as *const c_void) }, 0x0000_0e00);
        assert_eq!(current(), 0x0000_0e00);
        store_set(0x0010_0000);
        assert_eq!(unsafe { store_create_params(core::ptr::null()) }, 0x0010_0000);
    }

    #[test]
    fn driver_selection_matches_console_loader() {
        assert_eq!(driver_for(0), Driver::Stock);
        assert_eq!(driver_for(FLAG_INSTRUMENTED), Driver::Instrumented);
        assert_eq!(driver_for(FLAG_DEBUG), Driver::Debug);
        assert_eq!(driver_for(FLAG_DEBUG | FLAG_INSTRUMENTED), Driver::Debug);
        assert_eq!(driver_for(FLAG_DEBUG | FLAG_DEBUG_STOCK), Driver::StockWithDebugFlag);
        assert_eq!(driver_for(FLAG_DEBUG_STOCK), Driver::Stock);
        assert_eq!(Driver::Debug.console_umd(), "xgs12_d.dll");
        assert_eq!(Driver::Instrumented.console_umd(), "xgs12_i.dll");
        assert_eq!(Driver::StockWithDebugFlag.console_umd(), "xgs12.dll");
        assert!(Driver::Debug.pc_debug_layer());
        assert!(!Driver::Instrumented.pc_debug_layer());
        assert!(!Driver::StockWithDebugFlag.pc_debug_layer());
        assert!(!Driver::Stock.pc_debug_layer());
    }

    #[test]
    fn set_mask_keeps_debug_and_instrumented_bits() {
        let _g = T.lock().unwrap_or_else(|e| e.into_inner());
        for (flags, bit) in [(0u32, false), (FLAG_DEBUG, true), (FLAG_INSTRUMENTED, true), (0x0000_0200, false)] {
            assert_eq!(store_set(flags) & 0x0010_0001 != 0, bit, "flags {flags:#x}");
        }
    }
}
