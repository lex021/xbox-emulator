use super::store::{self, Store};
use super::tlg::{self, EventData};
use super::HResult;

pub const CLOUD_EDITION_VALUE: &str = "CodeIntegrity-CloudEdition-Enforce-Required";
pub const STATUS_OBJECT_NAME_NOT_FOUND: i32 = 0xC000_0034u32 as i32;
pub const SL_E_VALUE_NOT_FOUND: HResult = 0xC004_F012u32 as i32;
pub const NON_PRODUCTION_ERRORS_CI: &str = "%SystemRoot%\\System32\\CodeIntegrity\\Data\\Non-Production Errors.txt";
pub const NON_PRODUCTION_ERRORS_STATE_SEPARATED: &str = "%SystemDrive%\\data\\systemdata\\etw\\Non-Production Errors.txt";
pub const PART_A_PRIV_TAGS: u64 = 0x800;

const _: () = assert!(ERA_CLOUD_EDITION_STATUS < 0);

pub const ERA_CLOUD_EDITION_STATUS: i32 = STATUS_OBJECT_NAME_NOT_FOUND;

pub fn license_failure_hresult(status: i32) -> HResult {
    let r15 = (status as u32 | 0x1000_0000) as i32;
    match r15 as u32 {
        0xD000_0034 => SL_E_VALUE_NOT_FOUND,
        0xD000_00E5 => 0xC004_F027u32 as i32,
        0xD000_003E => 0xC004_F028u32 as i32,
        _ => r15,
    }
}

pub fn license_query_result(failure_status: i32) -> (HResult, u32) {
    debug_assert!(failure_status < 0);
    let hr = license_failure_hresult(failure_status);
    if hr == SL_E_VALUE_NOT_FOUND {
        (0, 0)
    } else {
        (hr, 0)
    }
}

pub fn non_production_errors_path(state_separation: bool) -> &'static str {
    if state_separation {
        NON_PRODUCTION_ERRORS_STATE_SEPARATED
    } else {
        NON_PRODUCTION_ERRORS_CI
    }
}

pub fn delete_non_production_errors(s: &Store) -> HResult {
    s.delete_file(non_production_errors_path(s.state_separation()))
}

pub fn production_event_enabled() -> bool {
    tlg::state().enabled(tlg::PRODUCTION_EVENT_DESC[1], tlg::desc_keyword(tlg::PRODUCTION_EVENT_DESC))
}

pub fn write_production_event(out: u32, required: u32, failed: u32, hr: HResult) -> Option<u32> {
    if !production_event_enabled() {
        return None;
    }
    let tag = PART_A_PRIV_TAGS;
    let fields = [EventData::of(&tag), EventData::of(&out), EventData::of(&required), EventData::of(&failed), EventData::of(&hr)];
    Some(tlg::state().write(tlg::PRODUCTION_EVENT_DESC, tlg::PRODUCTION_EVENT_META, &fields))
}

pub fn is_production_configuration_in_proc_in(s: &Store, license_failure_status: i32) -> (HResult, u32) {
    let (hr, required) = license_query_result(license_failure_status);
    let failed = 0u32;
    let out = if hr < 0 { 0 } else { (required & failed == 0) as u32 };
    write_production_event(out, required, failed, hr);
    let cleanup = if out != 0 { Some(delete_non_production_errors(s)) } else { None };
    store::log("WldpIsProductionConfigurationInProc", &format!("{CLOUD_EDITION_VALUE} status={license_failure_status:#x} flags={required:#x} out={out} hr={hr:#x} cleanup={cleanup:?}"));
    (hr, out)
}

#[no_mangle]
pub unsafe extern "system" fn WldpIsProductionConfigurationInProc(is_production: *mut u32) -> HResult {
    let (hr, out) = is_production_configuration_in_proc_in(&Store::open(), ERA_CLOUD_EDITION_STATUS);
    *is_production = out;
    hr
}
