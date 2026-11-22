mod ums;

#[no_mangle]
pub extern "system" fn RtlWnfCompareChangeStamp(a: u32, b: u32) -> i32 {
    if a == b {
        0
    } else if a < b {
        -1
    } else {
        1
    }
}
