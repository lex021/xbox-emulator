#![allow(non_snake_case)]

use core::ffi::c_void;
use std::sync::OnceLock;

pub type HResult = i32;

pub mod console;
pub mod production;
pub mod store;
pub mod tlg;
pub mod watermark;

pub const E_INVALIDARG_NT: HResult = 0xD000_000Du32 as i32;
pub const SYSTEM_CI_POLICY_INFORMATION: u32 = 0xa4;
pub const SYSTEM_CI_UNLOCK_INFORMATION: u32 = 0xa5;
pub const SYSTEM_CI_DYNAMIC_TRUST_INFORMATION: u32 = 0xc7;
pub const ERROR_NOT_SUPPORTED: u32 = 0x32;

pub const POLICY_SETTINGS: [&str; 6] = [
    "VerifiedAndReputableUI",
    "DisableMshtmlUmci",
    "ShellSmartscreenSuppressed",
    "BrowserSmartscreenSuppressed",
    "WcosOOBELogCollection",
    "LegacyWhqlFlighting",
];
pub const POLICY_SETTING_1000: &str = "VerifiedAndReputablePerfMode";

#[link(name = "ntdll")]
extern "system" {
    fn NtQuerySystemInformation(class: u32, buf: *mut c_void, len: u32, ret: *mut u32) -> i32;
    fn NtSetSystemInformation(class: u32, buf: *mut c_void, len: u32) -> i32;
}

#[link(name = "kernel32")]
extern "system" {
    fn GetSystemDirectoryW(buf: *mut u16, len: u32) -> u32;
    fn LoadLibraryExW(name: *const u16, file: *mut c_void, flags: u32) -> *mut c_void;
    fn GetProcAddress(module: *mut c_void, name: *const u8) -> *mut c_void;
}

type QueryPolicySettingEnabled2 = unsafe extern "system" fn(*const u16, *mut i32) -> HResult;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn hresult_from_win32(e: i32) -> HResult {
    if e > 0 {
        ((e as u32 & 0xffff) | 0x8007_0000) as i32
    } else {
        e
    }
}

fn nt_to_hresult(st: i32) -> HResult {
    if st < 0 {
        (st as u32 | 0x1000_0000) as i32
    } else {
        0
    }
}

fn host_wldp_export(name: &[u8]) -> usize {
    unsafe {
        let mut buf = [0u16; 260];
        let n = GetSystemDirectoryW(buf.as_mut_ptr(), 260) as usize;
        if n == 0 || n > 240 {
            return 0;
        }
        let tail: Vec<u16> = "\\wldp.dll".encode_utf16().chain(Some(0)).collect();
        buf[n..n + tail.len()].copy_from_slice(&tail);
        let h = LoadLibraryExW(buf.as_ptr(), core::ptr::null_mut(), 0);
        if h.is_null() {
            return 0;
        }
        GetProcAddress(h, name.as_ptr()) as usize
    }
}

fn query_policy_setting_enabled2() -> Option<QueryPolicySettingEnabled2> {
    static F: OnceLock<usize> = OnceLock::new();
    let f = *F.get_or_init(|| host_wldp_export(b"WldpQueryPolicySettingEnabled2\0"));
    (f != 0).then(|| unsafe { core::mem::transmute::<usize, QueryPolicySettingEnabled2>(f) })
}

pub fn policy_setting_name(index: i32) -> Option<&'static str> {
    if (0..6).contains(&index) {
        Some(POLICY_SETTINGS[index as usize])
    } else if index == 1000 {
        Some(POLICY_SETTING_1000)
    } else {
        None
    }
}

#[no_mangle]
pub unsafe extern "system" fn WldpCheckRetailConfiguration(_reserved: *mut c_void, is_retail: *mut u32, flags: *mut u32, reason: *mut u32) -> HResult {
    *is_retail = 1;
    *reason = 0;
    *flags = 0;
    0
}

#[no_mangle]
pub unsafe extern "system" fn WldpQueryPolicySettingEnabledInternal(setting: i32, enabled: *mut i32) -> HResult {
    if enabled.is_null() {
        return E_INVALIDARG_NT;
    }
    *enabled = 0;
    let Some(name) = policy_setting_name(setting) else {
        return E_INVALIDARG_NT;
    };
    let Some(f) = query_policy_setting_enabled2() else {
        return E_INVALIDARG_NT;
    };
    let w = wide(name);
    f(w.as_ptr(), enabled)
}

#[no_mangle]
pub unsafe extern "system" fn WldpSetDynamicCodeTrust2(file: *mut c_void) -> HResult {
    let mut info: [usize; 3] = [file as usize, 0, 0];
    nt_to_hresult(NtSetSystemInformation(SYSTEM_CI_DYNAMIC_TRUST_INFORMATION, info.as_mut_ptr() as *mut c_void, 0x18))
}

pub unsafe fn ci_policy_state(out: *mut u32) -> HResult {
    let mut buf = [0u32; 8];
    let st = NtQuerySystemInformation(SYSTEM_CI_POLICY_INFORMATION, buf.as_mut_ptr() as *mut c_void, 0x20, core::ptr::null_mut());
    if st < 0 {
        return (st as u32 | 0x1000_0000) as i32;
    }
    *out = if buf[0] & 0x2400_0000 != 0 { 2 } else { buf[0] >> 31 };
    0
}

#[no_mangle]
pub unsafe extern "system" fn WldpIsVulnerableDriverBlocklistDisabled(disabled: *mut i32) -> HResult {
    console::is_blocklist_disabled_in(&store::Store::open(), &mut *disabled)
}

#[no_mangle]
pub unsafe extern "system" fn WldpIsVulnerableDriverBlocklistEligibleToDisable(eligible: *mut i32) -> HResult {
    let mut info = [0u8; 16];
    let mut ret = 0u32;
    let st = NtQuerySystemInformation(SYSTEM_CI_UNLOCK_INFORMATION, info.as_mut_ptr() as *mut c_void, 0x10, &mut ret);
    let mut result = 0;
    if !(st >= 0 && ret == 0x10 && info[0] & 2 != 0) {
        let mut v = 0u32;
        let hr = ci_policy_state(&mut v);
        if hr < 0 || v == 0 {
            result = 1;
        }
    }
    *eligible = result;
    0
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_console;
#[cfg(test)]
mod tests_production;
#[cfg(test)]
mod tests_watermark;
