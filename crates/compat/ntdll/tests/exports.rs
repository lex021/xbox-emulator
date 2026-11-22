mod common;

use common::*;

#[test]
fn export_table_matches_exports_txt() {
    check_table("ntdll.dll");
}

#[test]
fn ums_entry_points_are_exported_and_refuse_a_non_ums_thread() {
    let h = load_built("ntdll.dll", "tbx_ntdll.dll");
    assert_eq!(check_forwarders(h, &[]), 0);
    let names = ["RtlpExecuteUmsThread", "RtlpUmsExecuteYieldThreadEnd", "RtlpUmsThreadYield"];
    let addrs: Vec<usize> = names.iter().map(|n| proc(h, n) as usize).collect();
    for (n, a) in names.iter().zip(&addrs) {
        assert_ne!(*a, 0, "{n}");
    }
    assert!(addrs[0] != addrs[1] && addrs[1] != addrs[2] && addrs[0] != addrs[2]);
    let exec: extern "system" fn(*mut core::ffi::c_void) -> i32 = unsafe { core::mem::transmute(addrs[0]) };
    let yld: extern "system" fn(*mut core::ffi::c_void) -> i32 = unsafe { core::mem::transmute(addrs[2]) };
    assert_eq!(exec(core::ptr::null_mut()) as u32, 0xC000_00BB);
    assert_eq!(yld(0x55 as *mut core::ffi::c_void) as u32, 0xC000_000D);
}

#[test]
fn rtl_wnf_compare_change_stamp_is_unsigned_three_way() {
    let h = load_built("ntdll.dll", "tbx_ntdll.dll");
    let f: extern "system" fn(u32, u32) -> i32 = unsafe { core::mem::transmute(proc(h, "RtlWnfCompareChangeStamp")) };
    assert_eq!(f(5, 5), 0);
    assert_eq!(f(1, 2), -1);
    assert_eq!(f(2, 1), 1);
    assert_eq!(f(0, 0xffff_ffff), -1);
    assert_eq!(f(0x8000_0000, 1), 1);
}
