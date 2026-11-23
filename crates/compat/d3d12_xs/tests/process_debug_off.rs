mod common_process_debug;
use common_process_debug::*;

#[test]
fn instrumented_and_stock_flags_leave_pc_debug_layer_off() {
    let d = load("off");
    assert_eq!(d.set(0x0010_0000), 0);
    assert_eq!(d.get(), 0x0010_0000);
    let (hr, dev) = d.create(0x0010_0000);
    assert_eq!(hr, 0, "D3D12XboxCreateDevice");
    assert!(!dev.is_null());
    let (hr, _) = d.create(0x8000_0001);
    assert_eq!(hr, 0);
    assert_eq!(d.get(), 0x8000_0001);
    let log = d.log_text();
    assert!(log.contains("ProcessDebugFlags=0x00100000 console UMD=xgs12_i.dll (Instrumented) pc debug layer requested=false"), "{log}");
    assert!(log.contains("ProcessDebugFlags=0x80000001 console UMD=xgs12.dll (StockWithDebugFlag) pc debug layer requested=false"), "{log}");
    assert!(log.contains("console UMD=xgs12_i.dll (Instrumented) pc debug layer requested=false enable_now=false"), "{log}");
    assert!(!log.contains("D3D12GetDebugInterface hr="), "{log}");
    assert!(!log.contains("debug layer ENABLED"), "{log}");
    let info_queue: [u8; 16] = [0x0b, 0xa9, 0x42, 0x07, 0x87, 0xc3, 0x3f, 0x48, 0xb9, 0x46, 0x30, 0xa7, 0xe4, 0xe6, 0x14, 0x58];
    let mut q: *mut core::ffi::c_void = core::ptr::null_mut();
    let hr = unsafe {
        let vt = *(dev as *const *const usize);
        let qi: unsafe extern "system" fn(*mut core::ffi::c_void, *const u8, *mut *mut core::ffi::c_void) -> i32 = core::mem::transmute(*vt);
        qi(dev, info_queue.as_ptr(), &mut q)
    };
    assert!(hr < 0 && q.is_null(), "ID3D12InfoQueue available (hr={hr:#010x}): the PC debug layer is on");
}
