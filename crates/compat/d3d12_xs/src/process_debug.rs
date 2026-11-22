use core::ffi::c_void;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use compat_common::record;

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
