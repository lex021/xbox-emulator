use crate::com::*;
use crate::conndiag::chk;
use crate::imports;
use crate::messaging::*;
use crate::monitor::OperationService;
use crate::sys::*;
use core::ffi::c_void;
use core::ptr::{null, null_mut};
use std::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::Mutex;

pub const OP_TRACE_START: u32 = 3;
pub const OP_TRACE_STOP: u32 = 4;
pub const OP_LOG_NETWORK_STATE: u32 = 12;

pub const TRACE_TYPE_ADVANCED_MULTIPLAYER: u32 = 2;
pub const TRACE_TYPE_XBOX_LIVE_SERVICES: u32 = 3;

pub const E_ILLEGAL_STATE_CHANGE: HRESULT = 0x8000_000Du32 as i32;
pub const E_ILLEGAL_METHOD_CALL: HRESULT = 0x8000_000Eu32 as i32;
pub const E_UNEXPECTED: HRESULT = 0x8000_FFFFu32 as i32;
pub const HR_FILE_NOT_FOUND: HRESULT = 0x8007_0002u32 as i32;
pub const HR_SHARING_VIOLATION: HRESULT = 0x8007_0020u32 as i32;

pub const XCRD_TEMP: i32 = 0;
pub const TRACE_XVD_PAGES: u64 = 0x44000;
pub const MOUNT_EXISTING_FLAGS: u32 = 0x40;
pub const PROPS_SIZE: usize = 0x4b8;
pub const LOG_FILE_OFFSET: usize = 0x78;
pub const LOGGER_NAME_OFFSET: usize = 0x280;
pub const ERROR_ALREADY_EXISTS: u32 = 0xb7;
pub const ERROR_BAD_PATHNAME: u32 = 0xa1;
pub const ERROR_WMI_INSTANCE_NOT_FOUND: u32 = 0x1069;
pub const EVENT_TRACE_CONTROL_STOP: u32 = 1;
pub const EVENT_CONTROL_CODE_ENABLE_PROVIDER: u32 = 1;
pub const TRACE_MESSAGE_FLAGS: u32 = 0x2b;
pub const WMI_ENABLE_EVENTS: u32 = 4;
pub const WMI_DISABLE_EVENTS: u32 = 5;
pub const WPP_STATE_FLAG: u32 = 8;

pub const TRACE_SESSION_NAME: &str = "XboxNetApiDiag ETW Trace Session";
pub const STATE_SESSION_NAME: &str = "XboxNetApiDiag ETW State Log Session";
pub const TITLE_TRACE_XVD: &str = "XtfTitleTraceStorage.xvd";
pub const TRACE_SUBFOLDER: &str = "GameCoreTrace";
pub const TRACE_FILE: &str = "xbtitletrace.etl";
pub const PENDING_FILE: &str = "PendingGameCoreTrace";

pub const TRACE_SESSION_GUID: GUID = GUID::new(0xcff86b4c, 0xee62, 0x4ee6, [0x9b, 0xfd, 0x56, 0x27, 0x56, 0x50, 0xaf, 0xcb]);
pub const STATE_SESSION_GUID: GUID = GUID::new(0xdf0aab47, 0x7972, 0x439b, [0xb5, 0xcf, 0x93, 0xda, 0xbb, 0x0c, 0xef, 0x24]);
pub const WPP_CONTROL_GUID: GUID = GUID::new(0xfc70fab8, 0x0cf6, 0x40a1, [0xab, 0xc2, 0xcd, 0x9c, 0x87, 0x9b, 0xeb, 0x9e]);
pub const STATE_HEADER_GUID: GUID = GUID::new(0xdb40d8f2, 0x1a60, 0x3c36, [0x59, 0xaa, 0xf5, 0x23, 0x00, 0xc7, 0x60, 0x82]);
pub const ADAPTERS_GUID: GUID = GUID::new(0x64b142a3, 0x5f89, 0x326e, [0x6d, 0xb4, 0xa3, 0x4c, 0x21, 0x35, 0xc2, 0x5a]);
pub const TEREDO_GUID: GUID = GUID::new(0xf0c88cea, 0x9ce1, 0x3a67, [0x14, 0x02, 0x0f, 0x59, 0x49, 0x94, 0x47, 0x82]);
pub const PROCESSES_GUID: GUID = GUID::new(0x0b7f33c9, 0xe185, 0x3be3, [0x3b, 0xa5, 0x89, 0xa8, 0xa6, 0x06, 0x05, 0x2f]);
pub const PORTS_GUID: GUID = GUID::new(0x8a582f79, 0xfe6d, 0x3ae0, [0x24, 0x35, 0x82, 0xc6, 0xff, 0x7a, 0xb4, 0xd5]);
pub const WFP_GUID: GUID = GUID::new(0xadd518f6, 0x4fb6, 0x3d86, [0x99, 0x7d, 0xe4, 0xdc, 0x67, 0x52, 0xff, 0x62]);

#[derive(Clone, Copy)]
pub struct Provider {
    pub guid: GUID,
    pub level: u8,
    pub any: u64,
}

const fn p(d1: u32, d2: u16, d3: u16, d4: [u8; 8], level: u8, any: u64) -> Provider {
    Provider { guid: GUID::new(d1, d2, d3, d4), level, any }
}

pub const STATE_PROVIDERS: [Provider; 1] = [p(0xc3d66fc7, 0x5736, 0x46d5, [0xb9, 0x42, 0xc4, 0x04, 0xf5, 0x02, 0xdd, 0xe4], 0xff, 0xffff_ffff)];

pub const TRACE_PROVIDERS: [Provider; 64] = [
    p(0xfc70fab8, 0x0cf6, 0x40a1, [0xab, 0xc2, 0xcd, 0x9c, 0x87, 0x9b, 0xeb, 0x9e], 0xff, 0xffffffff),
    p(0x6d3f1a5a, 0xe8bf, 0x4c11, [0xa8, 0xd0, 0xe1, 0xdd, 0x68, 0xfc, 0x2a, 0x22], 0x05, 0x0),
    p(0x106b464a, 0x8043, 0x46b1, [0x8c, 0xb8, 0xe9, 0x2a, 0x0c, 0xd7, 0xa5, 0x60], 0x05, 0xffffffff),
    p(0xad33fa19, 0xf2d2, 0x46d1, [0x8f, 0x4c, 0xe3, 0xc3, 0x08, 0x7e, 0x45, 0xad], 0x05, 0xffffffff),
    p(0x5a1600d2, 0x68e5, 0x4de7, [0xbc, 0xf4, 0x1c, 0x2d, 0x21, 0x5f, 0xe0, 0xfe], 0x05, 0xffffffff),
    p(0x106b464d, 0x8043, 0x46b1, [0x8c, 0xb8, 0xe9, 0x2a, 0x0c, 0xd7, 0xa5, 0x60], 0x05, 0xffffffff),
    p(0x0c478c5b, 0x0351, 0x41b1, [0x8c, 0x58, 0x4a, 0x67, 0x37, 0xda, 0x32, 0xe3], 0x05, 0x0),
    p(0x1c95126e, 0x7eea, 0x49a9, [0xa3, 0xfe, 0xa3, 0x78, 0xb0, 0x3d, 0xdb, 0x4d], 0x04, 0x10000000),
    p(0xcdead503, 0x17f5, 0x4a3e, [0xb7, 0xae, 0xdf, 0x8c, 0xc2, 0x90, 0x2e, 0xb9], 0x05, 0xffffffffffffbe),
    p(0x2f07e2ee, 0x15db, 0x40f1, [0x90, 0xef, 0x9d, 0x7b, 0xa2, 0x82, 0x18, 0x8a], 0x05, 0x0),
    p(0xe53c6823, 0x7bb8, 0x44bb, [0x90, 0xdc, 0x3f, 0x86, 0x09, 0x0d, 0x48, 0xa6], 0x05, 0x0),
    p(0xeb004a05, 0x9b1a, 0x11d4, [0x91, 0x23, 0x00, 0x50, 0x04, 0x77, 0x59, 0xbc], 0x05, 0xffffffff),
    p(0x4edbe902, 0x9ed3, 0x4cf0, [0x93, 0xe8, 0xb8, 0xb5, 0xfa, 0x92, 0x02, 0x99], 0xff, 0x0),
    p(0x6600e712, 0xc3b6, 0x44a2, [0x8a, 0x48, 0x93, 0x5c, 0x51, 0x1f, 0x28, 0xc8], 0xff, 0x0),
    p(0x66a5c15c, 0x4f8e, 0x4044, [0xbf, 0x6e, 0x71, 0xd8, 0x96, 0x03, 0x89, 0x77], 0xff, 0x0),
    p(0x3eb875eb, 0x8f4a, 0x4800, [0xa0, 0x0b, 0xe4, 0x84, 0xc9, 0x7d, 0x75, 0x51], 0xff, 0x0),
    p(0x88cd9180, 0x4491, 0x4640, [0xb5, 0x71, 0xe3, 0xbe, 0xe2, 0x52, 0x79, 0x43], 0xff, 0x0),
    p(0x988ce33b, 0xdde5, 0x44eb, [0x98, 0x16, 0xee, 0x15, 0x6b, 0x44, 0x3f, 0xf1], 0x09, 0xffffffff),
    p(0x082f46b6, 0x01f6, 0x4511, [0x90, 0x44, 0xd8, 0x36, 0x75, 0xf4, 0x66, 0x37], 0x09, 0xffffffff),
    p(0x15a7a4f8, 0x0072, 0x4eab, [0xab, 0xad, 0xf9, 0x8a, 0x4d, 0x66, 0x6a, 0xed], 0x04, 0x0),
    p(0x6a1f2b00, 0x6a90, 0x4c38, [0x95, 0xa5, 0x5c, 0xab, 0x3b, 0x05, 0x67, 0x78], 0x04, 0x0),
    p(0xf6da35ce, 0xd312, 0x41c8, [0x98, 0x28, 0x5a, 0x2e, 0x17, 0x3c, 0x91, 0xb6], 0x04, 0x0),
    p(0x43d1a55c, 0x76d6, 0x4f7e, [0x99, 0x5c, 0x64, 0xc7, 0x11, 0xe5, 0xca, 0xfe], 0x05, 0x0),
    p(0x50b3e73c, 0x9370, 0x461d, [0xbb, 0x9f, 0x26, 0xf3, 0x2d, 0x68, 0x88, 0x7d], 0x05, 0x0),
    p(0x08f93b14, 0x1608, 0x4a72, [0x9c, 0xfa, 0x45, 0x7e, 0xec, 0xed, 0xbb, 0xa7], 0x05, 0xffffffff),
    p(0x7d44233d, 0x3055, 0x4b9c, [0xba, 0x64, 0x0d, 0x47, 0xca, 0x40, 0xa2, 0x32], 0x05, 0x0),
    p(0xb3a7698a, 0x0c45, 0x44da, [0xb7, 0x3d, 0xe1, 0x81, 0xc9, 0xb5, 0xc8, 0xe6], 0x05, 0xffffffff),
    p(0xdd85457f, 0x4e2d, 0x44a5, [0xa7, 0xa7, 0x62, 0x53, 0x36, 0x2e, 0x34, 0xdc], 0x05, 0x0),
    p(0xa67075c2, 0x3e39, 0x4109, [0xb6, 0xcd, 0x6d, 0x75, 0x00, 0x58, 0xa7, 0x32], 0x04, 0x0),
    p(0xa6f32731, 0x9a38, 0x4159, [0xa2, 0x20, 0x3d, 0x9b, 0x7f, 0xc5, 0xfe, 0x5d], 0x05, 0x0),
    p(0x2acf2633, 0xed1b, 0x41fd, [0xa1, 0x7e, 0x9f, 0x98, 0x71, 0x9c, 0xdb, 0x36], 0x05, 0xffffffff),
    p(0x014de49f, 0xce63, 0x4779, [0xba, 0x2b, 0xd6, 0x16, 0xf6, 0x96, 0x3a, 0x87], 0x05, 0xffffffff),
    p(0xd9131565, 0xe1dd, 0x4c9e, [0xa7, 0x28, 0x95, 0x19, 0x99, 0xc2, 0xad, 0xb5], 0x05, 0xffffffff),
    p(0xea289c62, 0x8c36, 0x4904, [0x97, 0x26, 0x15, 0xec, 0xd2, 0x82, 0xae, 0xd5], 0x05, 0xffffffff),
    p(0xed795972, 0x60e8, 0x4815, [0x86, 0x34, 0xcf, 0xaa, 0x21, 0xa8, 0x9d, 0xe7], 0x05, 0xffffffff),
    p(0x9580d7dd, 0x0379, 0x4658, [0x98, 0x70, 0xd5, 0xbe, 0x7d, 0x52, 0xd6, 0xde], 0x05, 0x0),
    p(0x3ee18f84, 0x0b52, 0x4411, [0xb7, 0x05, 0xc9, 0x14, 0xcb, 0xd2, 0x29, 0xac], 0x05, 0x0),
    p(0xc30dd967, 0x75d6, 0x4ea4, [0x91, 0x0a, 0x5d, 0x7a, 0xb9, 0xf8, 0x10, 0x8b], 0x05, 0x0),
    p(0x3e7714aa, 0xf717, 0x4c62, [0x8d, 0x4d, 0xb3, 0x38, 0x3e, 0x4d, 0xf0, 0x90], 0x05, 0x0),
    p(0x514775e1, 0xfa95, 0x459f, [0xb6, 0xd4, 0xb8, 0x6a, 0xd1, 0xc5, 0xb4, 0x9e], 0xff, 0x0),
    p(0xe176aa66, 0x5cc8, 0x4321, [0x96, 0x24, 0xf9, 0xc1, 0xd2, 0xb7, 0xbf, 0x06], 0x05, 0xffffffff),
    p(0x71975d00, 0x46bb, 0x4236, [0xbd, 0x4f, 0x41, 0xa8, 0xc7, 0x2f, 0xad, 0xfc], 0x05, 0xffffffff),
    p(0xdb1d0418, 0x105a, 0x4c77, [0x9a, 0x25, 0x8f, 0x96, 0xa1, 0x97, 0x16, 0xa4], 0x05, 0xffffffff),
    p(0x5444519f, 0x2484, 0x45a2, [0x99, 0x1e, 0x95, 0x3e, 0x4b, 0x54, 0xc8, 0xe0], 0x05, 0x0),
    p(0x37945dc2, 0x899b, 0x44d1, [0xb7, 0x9c, 0xdd, 0x4a, 0x9e, 0x57, 0xff, 0x98], 0x05, 0x0),
    p(0xd1bc9aff, 0x2abf, 0x4d71, [0x91, 0x46, 0xec, 0xb2, 0xa9, 0x86, 0xeb, 0x85], 0x05, 0x0),
    p(0xe595f735, 0xb42a, 0x494b, [0xaf, 0xcd, 0xb6, 0x86, 0x66, 0x94, 0x5c, 0xd3], 0x05, 0x0),
    p(0x50bd1bfd, 0x936b, 0x4db3, [0x86, 0xbe, 0xe2, 0x5b, 0x96, 0xc2, 0x58, 0x98], 0x05, 0x0),
    p(0x28c9f48f, 0xd244, 0x45a8, [0x84, 0x2f, 0xdc, 0x9f, 0xbc, 0x9b, 0x6e, 0x92], 0x05, 0xffffffff),
    p(0x5eefebdb, 0xe90c, 0x423a, [0x8a, 0xbf, 0x02, 0x41, 0xe7, 0xc5, 0xb8, 0x7d], 0x05, 0xffffffff),
    p(0x94335eb3, 0x79ea, 0x44d5, [0x8e, 0xa9, 0x30, 0x6f, 0x49, 0xb3, 0xa0, 0x70], 0x05, 0xffffffff),
    p(0xae5f9155, 0xcb33, 0x4a17, [0x83, 0x84, 0xa4, 0x2c, 0x97, 0x1d, 0x51, 0x76], 0xff, 0xfb1beeff),
    p(0xdd5ef90a, 0x6398, 0x47a4, [0xad, 0x34, 0x4d, 0xce, 0xcd, 0xef, 0x79, 0x5f], 0x05, 0x0),
    p(0x20f61733, 0x57f1, 0x4127, [0x9f, 0x48, 0x4a, 0xb7, 0xa9, 0x30, 0x8a, 0xe2], 0x05, 0xffffffff),
    p(0x3bca077a, 0xffa0, 0x48aa, [0xa9, 0x74, 0xb7, 0x5d, 0x6e, 0x0c, 0xb3, 0xa2], 0xff, 0xffffffcf),
    p(0x66eed9b0, 0x5214, 0x4557, [0x9d, 0x74, 0xe0, 0x73, 0xb2, 0xa7, 0xe4, 0x76], 0xff, 0xfb1beeff),
    p(0x48ea4db0, 0x8d7e, 0x419b, [0xb4, 0x65, 0xe5, 0xb5, 0x72, 0xf3, 0x03, 0x05], 0xff, 0xffffffbf),
    p(0x4e8f98a2, 0x3dc0, 0x4dd8, [0xbb, 0x6f, 0x73, 0x35, 0x06, 0x86, 0x50, 0xa5], 0xff, 0xffffffbf),
    p(0x67e5fdfc, 0x8ceb, 0x4064, [0xaa, 0x29, 0x39, 0x1e, 0x6a, 0x0a, 0x93, 0x18], 0x05, 0xffffffff),
    p(0x090bf343, 0x4490, 0x42d3, [0xb2, 0x73, 0x8a, 0xf1, 0x74, 0xd3, 0x14, 0xfb], 0x05, 0xffffffff),
    p(0x3bed4658, 0x7416, 0x4509, [0xb2, 0x02, 0xba, 0x88, 0xa5, 0x62, 0x10, 0x00], 0x05, 0xffffffff),
    p(0xe68f0cea, 0xfaad, 0x5564, [0x5a, 0x6e, 0xe0, 0xcf, 0x98, 0x96, 0x87, 0xed], 0x05, 0xffffffff),
    p(0x4e5392bb, 0xe4e5, 0x46e3, [0xb3, 0x0a, 0x05, 0xb4, 0xfe, 0x13, 0x2c, 0xaf], 0x05, 0xffffffff),
    p(0xb30b7843, 0x1378, 0x4890, [0x82, 0xc1, 0xb2, 0xf8, 0xa7, 0x4b, 0x5b, 0x4e], 0x05, 0xffffffff),
];

#[repr(C)]
pub struct EnableTraceParameters {
    pub version: u32,
    pub enable_property: u32,
    pub control_flags: u32,
    pub source_id: GUID,
    pub enable_filter_desc: *mut c_void,
    pub filter_desc_count: u32,
}

#[repr(C)]
pub struct TraceGuidRegistration {
    pub guid: *const GUID,
    pub handle: HANDLE,
}

#[repr(C)]
pub struct UnicodeString {
    pub length: u16,
    pub maximum_length: u16,
    pub buffer: *const u16,
}

#[repr(C)]
pub struct ObjectAttributes {
    pub length: u32,
    pub root_directory: HANDLE,
    pub object_name: *const UnicodeString,
    pub attributes: u32,
    pub security_descriptor: *mut c_void,
    pub security_quality_of_service: *mut c_void,
}

#[repr(C)]
pub struct SystemTime {
    pub year: u16,
    pub month: u16,
    pub day_of_week: u16,
    pub day: u16,
    pub hour: u16,
    pub minute: u16,
    pub second: u16,
    pub milliseconds: u16,
}

#[repr(C)]
pub struct SystemInfo {
    pub processor_architecture: u16,
    pub reserved: u16,
    pub page_size: u32,
    pub minimum_application_address: *mut c_void,
    pub maximum_application_address: *mut c_void,
    pub active_processor_mask: usize,
    pub number_of_processors: u32,
    pub processor_type: u32,
    pub allocation_granularity: u32,
    pub processor_level: u16,
    pub processor_revision: u16,
}

pub type ControlCallback = unsafe extern "system" fn(u32, *mut c_void, *mut u32, *mut c_void) -> u32;

#[link(name = "advapi32")]
extern "system" {
    pub fn StartTraceW(handle: *mut u64, name: *const u16, props: *mut u8) -> u32;
    pub fn ControlTraceW(handle: u64, name: *const u16, props: *mut u8, code: u32) -> u32;
    pub fn EnableTraceEx2(handle: u64, provider: *const GUID, code: u32, level: u8, any: u64, all: u64, timeout: u32, params: *const EnableTraceParameters) -> u32;
    pub fn RegisterTraceGuidsW(cb: ControlCallback, ctx: *mut c_void, control: *const GUID, count: u32, regs: *mut TraceGuidRegistration, mof_path: *const u16, mof_resource: *const u16, handle: *mut u64) -> u32;
    pub fn UnregisterTraceGuids(handle: u64) -> u32;
    pub fn GetTraceLoggerHandle(buffer: *mut c_void) -> u64;
    pub fn GetTraceEnableLevel(logger: u64) -> u8;
    pub fn GetTraceEnableFlags(logger: u64) -> u32;
}

#[link(name = "advapi32")]
extern "C" {
    pub fn TraceMessage(logger: u64, flags: u32, guid: *const GUID, number: u16, ...) -> u32;
}

#[link(name = "kernel32")]
extern "system" {
    pub fn CreateDirectoryW(path: *const u16, sa: *mut c_void) -> BOOL;
    pub fn DeleteFileW(path: *const u16) -> BOOL;
    pub fn GetLocalTime(t: *mut SystemTime);
    pub fn GetSystemInfo(info: *mut SystemInfo);
}

pub type NtOpenPartitionFn = unsafe extern "system" fn(*mut HANDLE, u32, *const ObjectAttributes) -> i32;

pub type OpenAdapterFn = unsafe extern "system" fn(*mut *mut c_void) -> HRESULT;
pub type CloseAdapterFn = unsafe extern "system" fn(*mut c_void) -> HRESULT;
pub type BuildPathFn = unsafe extern "system" fn(*mut *mut u16, i32, *const u16, u32) -> HRESULT;
pub type DeleteXvdFn = unsafe extern "system" fn(*mut c_void, *const u16, u32) -> HRESULT;
pub type CreateXvdFn = unsafe extern "system" fn(*mut c_void, *const u16, *const u8, u32, u64, u32, u32, *mut u8, *const u8, *const u8, u32, *const u16) -> HRESULT;
pub type MountFn = unsafe extern "system" fn(*mut *mut c_void, *mut c_void, *const u16, u32) -> HRESULT;
pub type UnmountFn = unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT;
pub type QueryDevicePathFn = unsafe extern "system" fn(*mut *mut u16, *const c_void) -> HRESULT;
pub type FreeDevicePathBufferFn = unsafe extern "system" fn(*mut c_void) -> HRESULT;
pub type ResolvePathFn = unsafe extern "system" fn(*const u16, *mut u16, u32) -> u32;

#[derive(Clone, Copy)]
pub struct XCrd {
    pub open_adapter: OpenAdapterFn,
    pub close_adapter: CloseAdapterFn,
    pub build_path: BuildPathFn,
    pub delete_xvd: DeleteXvdFn,
    pub create_xvd: CreateXvdFn,
    pub mount: MountFn,
    pub unmount: UnmountFn,
    pub query_device_path: QueryDevicePathFn,
    pub free_device_path_buffer: FreeDevicePathBufferFn,
    pub resolve_path: Option<ResolvePathFn>,
}

static XCRD: Mutex<Option<XCrd>> = Mutex::new(None);

pub fn bind_xcrd(x: Option<XCrd>) {
    *XCRD.lock().unwrap_or_else(|e| e.into_inner()) = x;
}

unsafe fn xcrd_proc(m: HANDLE, name: &[u8]) -> Result<usize, HRESULT> {
    let f = GetProcAddress(m, name.as_ptr());
    if f.is_null() {
        return Err(hr_last_error());
    }
    Ok(f as usize)
}

unsafe fn load_xcrd() -> Result<XCrd, HRESULT> {
    let name = wz("XCRDAPI.dll");
    let mut m = GetModuleHandleW(name.as_ptr());
    if m.is_null() {
        m = LoadLibraryExW(name.as_ptr(), null_mut(), 0);
    }
    if m.is_null() {
        return Err(hr_last_error());
    }
    let resolve = GetProcAddress(m, b"TomBoxXcrdResolvePath\0".as_ptr());
    Ok(XCrd {
        open_adapter: core::mem::transmute::<usize, OpenAdapterFn>(xcrd_proc(m, b"XCrdOpenAdapter\0")?),
        close_adapter: core::mem::transmute::<usize, CloseAdapterFn>(xcrd_proc(m, b"XCrdCloseAdapter\0")?),
        build_path: core::mem::transmute::<usize, BuildPathFn>(xcrd_proc(m, b"XCrdBuildPath\0")?),
        delete_xvd: core::mem::transmute::<usize, DeleteXvdFn>(xcrd_proc(m, b"XCrdDeleteXVD\0")?),
        create_xvd: core::mem::transmute::<usize, CreateXvdFn>(xcrd_proc(m, b"XCrdCreateXVD\0")?),
        mount: core::mem::transmute::<usize, MountFn>(xcrd_proc(m, b"XCrdMount\0")?),
        unmount: core::mem::transmute::<usize, UnmountFn>(xcrd_proc(m, b"XCrdUnmount\0")?),
        query_device_path: core::mem::transmute::<usize, QueryDevicePathFn>(xcrd_proc(m, b"XCrdQueryDevicePath\0")?),
        free_device_path_buffer: core::mem::transmute::<usize, FreeDevicePathBufferFn>(xcrd_proc(m, b"XCrdFreeDevicePathBuffer\0")?),
        resolve_path: if resolve.is_null() { None } else { Some(core::mem::transmute::<*mut c_void, ResolvePathFn>(resolve)) },
    })
}

pub unsafe fn xcrd() -> Result<XCrd, HRESULT> {
    let mut g = XCRD.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(x) = *g {
        return Ok(x);
    }
    let x = load_xcrd()?;
    *g = Some(x);
    Ok(x)
}

pub struct XvdState {
    pub handle: usize,
    pub directory: Option<Vec<u16>>,
}

pub static XVD: Mutex<XvdState> = Mutex::new(XvdState { handle: usize::MAX, directory: None });

pub struct ServerState {
    pub trace_type: u32,
    pub start_tick: u64,
    pub pending_restart: bool,
}

pub static SERVER: Mutex<ServerState> = Mutex::new(ServerState { trace_type: 0, start_tick: 0, pending_restart: false });

pub fn trace_type() -> u32 {
    SERVER.lock().unwrap_or_else(|e| e.into_inner()).trace_type
}

pub fn trace_directory() -> Option<Vec<u16>> {
    XVD.lock().unwrap_or_else(|e| e.into_inner()).directory.clone()
}

pub fn xvd_handle() -> usize {
    XVD.lock().unwrap_or_else(|e| e.into_inner()).handle
}

unsafe fn wcopy(p: *const u16) -> Option<Vec<u16>> {
    if p.is_null() || *p == 0 {
        return None;
    }
    Some(wslice(p).to_vec())
}

fn join(dir: Option<&[u16]>, leaf: &str) -> Vec<u16> {
    let mut v: Vec<u16> = match dir {
        Some(d) => d.to_vec(),
        None => "(null)".encode_utf16().collect(),
    };
    v.push(b'\\' as u16);
    v.extend(leaf.encode_utf16());
    v
}

pub unsafe fn host_path(path: &[u16]) -> Vec<u16> {
    let mut z = path.to_vec();
    z.push(0);
    if let Ok(x) = xcrd() {
        if let Some(f) = x.resolve_path {
            let mut cap = 520u32;
            for _ in 0..4 {
                let mut out = vec![0u16; cap as usize];
                let r = f(z.as_ptr(), out.as_mut_ptr(), cap);
                if r == 0 {
                    break;
                }
                if r < cap {
                    out.truncate(r as usize);
                    out.push(0);
                    return out;
                }
                cap = r + 1;
            }
        }
    }
    z
}

pub unsafe fn create_and_mount_trace_xvd() -> HRESULT {
    let mut st = XVD.lock().unwrap_or_else(|e| e.into_inner());
    imports::log_info(b"CreateAndMountTraceXVD called\0", &[]);
    if st.handle != usize::MAX {
        return chk(HR_SHARING_VIOLATION, b"ERROR_SHARING_VIOLATION\0", b"CreateAndMountTraceXVD\0", 117);
    }
    st.directory = None;
    let x = match xcrd() {
        Ok(x) => x,
        Err(e) => return chk(e, b"XCrdOpenAdapter(&crdAdapter)\0", b"CreateAndMountTraceXVD\0", 121),
    };
    let mut adapter: *mut c_void = null_mut();
    let mut path: *mut u16 = null_mut();
    let mut handle: *mut c_void = usize::MAX as *mut c_void;
    let mut device: *mut u16 = null_mut();
    let mut hr;
    'body: {
        hr = chk((x.open_adapter)(&mut adapter), b"XCrdOpenAdapter(&crdAdapter)\0", b"CreateAndMountTraceXVD\0", 121);
        if hr < 0 {
            break 'body;
        }
        hr = chk((x.build_path)(&mut path, XCRD_TEMP, wz(TITLE_TRACE_XVD).as_ptr(), 0), b"XCrdBuildPath(&crdPath, XCrdTemp, TitleTraceXvdName, 0)\0", b"CreateAndMountTraceXVD\0", 122);
        if hr < 0 {
            break 'body;
        }
        hr = (x.delete_xvd)(adapter, path, 0);
        if hr == HR_FILE_NOT_FOUND {
            hr = S_OK;
        }
        hr = chk(hr, b"XCrdDeleteXVD(crdAdapter, crdPath, 0)\0", b"CreateAndMountTraceXVD\0", 130);
        if hr < 0 {
            break 'body;
        }
        hr = chk((x.create_xvd)(adapter, path, null(), 0, TRACE_XVD_PAGES, 0, 0, null_mut(), null(), null(), 0, null()), b"XCrdCreateXVD( crdAdapter, crdPath, nullptr, xvdCreateFlags, driveSize, ...)\0", b"CreateAndMountTraceXVD\0", 144);
        if hr < 0 {
            break 'body;
        }
        hr = chk((x.mount)(&mut handle, adapter, path, 0), b"XCrdMount(&traceXvdHandle, crdAdapter, crdPath, 0)\0", b"CreateAndMountTraceXVD\0", 146);
        if hr < 0 {
            break 'body;
        }
        hr = chk((x.query_device_path)(&mut device, handle), b"XCrdQueryDevicePath(&devicePath, traceXvdHandle)\0", b"CreateAndMountTraceXVD\0", 147);
        if hr < 0 {
            break 'body;
        }
        st.directory = wcopy(device);
        st.handle = handle as usize;
    }
    if !device.is_null() {
        (x.free_device_path_buffer)(device as *mut c_void);
    }
    if hr < 0 && handle as usize != usize::MAX {
        (x.unmount)(adapter, handle);
    }
    if !path.is_null() {
        (x.free_device_path_buffer)(path as *mut c_void);
    }
    if !adapter.is_null() {
        (x.close_adapter)(adapter);
    }
    hr
}

pub unsafe fn mount_trace_xvd() -> HRESULT {
    let mut st = XVD.lock().unwrap_or_else(|e| e.into_inner());
    imports::log_info(b"MountTraceXVD called\0", &[]);
    if st.handle != usize::MAX {
        return chk(HR_SHARING_VIOLATION, b"ERROR_SHARING_VIOLATION\0", b"MountTraceXVD\0", 196);
    }
    st.directory = None;
    let x = match xcrd() {
        Ok(x) => x,
        Err(e) => return chk(e, b"XCrdOpenAdapter(&crdAdapter)\0", b"MountTraceXVD\0", 200),
    };
    let mut adapter: *mut c_void = null_mut();
    let mut path: *mut u16 = null_mut();
    let mut handle: *mut c_void = usize::MAX as *mut c_void;
    let mut device: *mut u16 = null_mut();
    let mut hr;
    'body: {
        hr = chk((x.open_adapter)(&mut adapter), b"XCrdOpenAdapter(&crdAdapter)\0", b"MountTraceXVD\0", 200);
        if hr < 0 {
            break 'body;
        }
        hr = chk((x.build_path)(&mut path, XCRD_TEMP, wz(TITLE_TRACE_XVD).as_ptr(), 0), b"XCrdBuildPath(&crdPath, XCrdTemp, TitleTraceXvdName, 0)\0", b"MountTraceXVD\0", 201);
        if hr < 0 {
            break 'body;
        }
        hr = chk((x.mount)(&mut handle, adapter, path, MOUNT_EXISTING_FLAGS), b"XCrdMount(&traceXvdHandle, crdAdapter, crdPath, mountFlags)\0", b"MountTraceXVD\0", 202);
        if hr < 0 {
            break 'body;
        }
        hr = chk((x.query_device_path)(&mut device, handle), b"XCrdQueryDevicePath(&devicePath, traceXvdHandle)\0", b"MountTraceXVD\0", 203);
        if hr < 0 {
            break 'body;
        }
        st.directory = wcopy(device);
        st.handle = handle as usize;
    }
    if !device.is_null() {
        (x.free_device_path_buffer)(device as *mut c_void);
    }
    if hr < 0 && handle as usize != usize::MAX {
        (x.unmount)(adapter, handle);
    }
    if !path.is_null() {
        (x.free_device_path_buffer)(path as *mut c_void);
    }
    if !adapter.is_null() {
        (x.close_adapter)(adapter);
    }
    hr
}

pub unsafe fn unmount_trace_xvd() {
    imports::log_info(b"UnmountTitleTraceXVD called\0", &[]);
    let mut st = XVD.lock().unwrap_or_else(|e| e.into_inner());
    if st.handle == usize::MAX {
        return;
    }
    let x = match xcrd() {
        Ok(x) => x,
        Err(e) => {
            chk(e, b"XCrdOpenAdapter(&crdAdapter)\0", b"UnmountTraceXVD\0", 243);
            return;
        }
    };
    let mut adapter: *mut c_void = null_mut();
    if chk((x.open_adapter)(&mut adapter), b"XCrdOpenAdapter(&crdAdapter)\0", b"UnmountTraceXVD\0", 243) < 0 {
        return;
    }
    if chk((x.unmount)(adapter, st.handle as *mut c_void), b"XCrdUnmount(crdAdapter, s_traceXvdHandle)\0", b"UnmountTraceXVD\0", 244) >= 0 {
        st.handle = usize::MAX;
        st.directory = None;
    }
    (x.close_adapter)(adapter);
}

pub unsafe fn get_trace_directory(out: &mut Option<Vec<u16>>) -> HRESULT {
    *out = None;
    let dir = trace_directory();
    let Some(d) = dir else {
        return S_OK;
    };
    let path = join(Some(&d), TRACE_SUBFOLDER);
    let host = host_path(&path);
    if CreateDirectoryW(host.as_ptr(), null_mut()) == 0 {
        let e = GetLastError();
        if e != ERROR_ALREADY_EXISTS {
            return chk(hr_from_win32(e), b"lastError\0", b"_GetTraceDirectory\0", 47);
        }
    }
    *out = Some(path);
    S_OK
}

pub unsafe fn get_trace_file_path(out: &mut Vec<u16>) -> HRESULT {
    let mut dir = None;
    let hr = chk(get_trace_directory(&mut dir), b"_GetTraceDirectory(&strTraceDirectory)\0", b"GetTraceFilePath\0", 80);
    if hr < 0 {
        return hr;
    }
    *out = join(dir.as_deref(), TRACE_FILE);
    S_OK
}

pub struct Wpp {
    pub registration: AtomicU64,
    pub logger: AtomicU64,
    pub flags: AtomicU32,
    pub level: AtomicU32,
    pub registrations: AtomicUsize,
}

pub static WPP: Wpp = Wpp { registration: AtomicU64::new(0), logger: AtomicU64::new(0), flags: AtomicU32::new(0), level: AtomicU32::new(0), registrations: AtomicUsize::new(0) };

pub static WPP_STATE: Wpp = Wpp { registration: AtomicU64::new(0), logger: AtomicU64::new(0), flags: AtomicU32::new(0), level: AtomicU32::new(0), registrations: AtomicUsize::new(0) };

pub const STATE_CONTROL_GUID: GUID = STATE_PROVIDERS[0].guid;

unsafe extern "system" fn wpp_control(code: u32, ctx: *mut c_void, _size: *mut u32, buffer: *mut c_void) -> u32 {
    let block: &Wpp = if ctx.is_null() { &WPP } else { &*(ctx as *const Wpp) };
    match code {
        WMI_ENABLE_EVENTS => {
            let logger = GetTraceLoggerHandle(buffer);
            let level = GetTraceEnableLevel(logger);
            let flags = GetTraceEnableFlags(logger);
            block.level.store(level as u32, Ordering::Release);
            block.flags.store(flags, Ordering::Release);
            block.logger.store(logger, Ordering::Release);
        }
        WMI_DISABLE_EVENTS => {
            block.logger.store(0, Ordering::Release);
            block.flags.store(0, Ordering::Release);
            block.level.store(0, Ordering::Release);
        }
        _ => {}
    }
    0
}

unsafe fn wpp_register(block: &'static Wpp, guid: &'static GUID) -> u32 {
    if block.registration.load(Ordering::Acquire) != 0 {
        return 0;
    }
    let mut reg = TraceGuidRegistration { guid, handle: null_mut() };
    let mut h = 0u64;
    let r = RegisterTraceGuidsW(wpp_control, block as *const Wpp as *mut c_void, guid, 1, &mut reg, null(), null(), &mut h);
    if r == 0 {
        block.registration.store(h, Ordering::Release);
        block.registrations.fetch_add(1, Ordering::AcqRel);
    }
    r
}

unsafe fn wpp_unregister(block: &Wpp) {
    let h = block.registration.swap(0, Ordering::AcqRel);
    if h != 0 {
        UnregisterTraceGuids(h);
    }
    block.logger.store(0, Ordering::Release);
    block.flags.store(0, Ordering::Release);
}

pub unsafe fn wpp_init() -> u32 {
    let r = wpp_register(&WPP, &WPP_CONTROL_GUID);
    if r != 0 {
        return r;
    }
    wpp_register(&WPP_STATE, &STATE_CONTROL_GUID)
}

pub unsafe fn wpp_cleanup() {
    wpp_unregister(&WPP);
    wpp_unregister(&WPP_STATE);
}

pub unsafe fn wpp_message(guid: &GUID, number: u16, value: u64) -> bool {
    let logger = WPP_STATE.logger.load(Ordering::Acquire);
    if logger == 0 || WPP_STATE.flags.load(Ordering::Acquire) & WPP_STATE_FLAG == 0 {
        return false;
    }
    let v = value;
    TraceMessage(logger, TRACE_MESSAGE_FLAGS, guid, number, &v as *const u64, 8usize, null::<c_void>()) == 0
}

unsafe fn open_partition(name: &str) -> Option<HANDLE> {
    let ntdll = GetModuleHandleW(wz("ntdll.dll").as_ptr());
    if ntdll.is_null() {
        return None;
    }
    let f = GetProcAddress(ntdll, b"NtOpenPartition\0".as_ptr());
    if f.is_null() {
        return None;
    }
    let f: NtOpenPartitionFn = core::mem::transmute(f);
    let w: Vec<u16> = name.encode_utf16().collect();
    let us = UnicodeString { length: (w.len() * 2) as u16, maximum_length: (w.len() * 2) as u16, buffer: w.as_ptr() };
    let oa = ObjectAttributes { length: 0x30, root_directory: null_mut(), object_name: &us, attributes: 0x40, security_descriptor: null_mut(), security_quality_of_service: null_mut() };
    let mut h: HANDLE = null_mut();
    if f(&mut h, 2, &oa) < 0 {
        return None;
    }
    Some(h)
}

fn put32(b: &mut [u8], o: usize, v: u32) {
    b[o..o + 4].copy_from_slice(&v.to_le_bytes());
}

fn put16(b: &mut [u8], o: usize, v: u16) {
    b[o..o + 2].copy_from_slice(&v.to_le_bytes());
}

fn guid_bytes(g: &GUID) -> [u8; 16] {
    let mut b = [0u8; 16];
    b[0..4].copy_from_slice(&g.d1.to_le_bytes());
    b[4..6].copy_from_slice(&g.d2.to_le_bytes());
    b[6..8].copy_from_slice(&g.d3.to_le_bytes());
    b[8..16].copy_from_slice(&g.d4);
    b
}

fn guid_from(b: &[u8]) -> GUID {
    GUID::new(u32::from_le_bytes(b[0..4].try_into().unwrap()), u16::from_le_bytes(b[4..6].try_into().unwrap()), u16::from_le_bytes(b[6..8].try_into().unwrap()), b[8..16].try_into().unwrap())
}

pub struct Properties {
    pub bytes: Vec<u8>,
    pub partition: HANDLE,
}

impl Drop for Properties {
    fn drop(&mut self) {
        if !self.partition.is_null() {
            unsafe { CloseHandle(self.partition) };
        }
    }
}

pub unsafe fn build_properties(file: &[u16], buffer_kb: u32, min_buffers: u32, max_file_mb: u32, session: &GUID) -> Properties {
    let mut b = vec![0u8; PROPS_SIZE];
    put32(&mut b, 0, PROPS_SIZE as u32);
    b[0x18..0x28].copy_from_slice(&guid_bytes(session));
    put32(&mut b, 0x28, 2);
    put32(&mut b, 0x2c, 0x20000);
    put32(&mut b, 0x30, buffer_kb);
    put32(&mut b, 0x34, min_buffers);
    if min_buffers != 0 {
        put32(&mut b, 0x38, min_buffers + 0x14);
    }
    put32(&mut b, 0x3c, max_file_mb);
    put32(&mut b, 0x40, if max_file_mb != 0 { 2 } else { 0 });
    put32(&mut b, 0x488, 0x50007);
    put32(&mut b, 0x48, 0x8000_0000);
    put16(&mut b, 0x48, 0x480);
    put16(&mut b, 0x4a8, 3);
    b[0x4a] = 0xff;
    put32(&mut b, 0x480, 0x2000d);
    put32(&mut b, 0x484, 0x10009);
    let mut partition = open_partition("\\GLOBAL??\\MemoryPartitionXboxTools");
    if partition.is_none() {
        partition = open_partition("\\GLOBAL??\\MemoryPartitionXboxTitle");
    }
    if let Some(h) = partition {
        put16(&mut b, 0x4aa, 6);
        b[0x4ac..0x4b4].copy_from_slice(&(h as u64).to_le_bytes());
    }
    put32(&mut b, 0x70, LOG_FILE_OFFSET as u32);
    put32(&mut b, 0x74, LOGGER_NAME_OFFSET as u32);
    let n = file.len().min(0x103);
    for (i, c) in file[..n].iter().enumerate() {
        put16(&mut b, LOG_FILE_OFFSET + i * 2, *c);
    }
    put16(&mut b, 0x27e, 0);
    Properties { bytes: b, partition: partition.unwrap_or(null_mut()) }
}

fn hr_win32(e: u32) -> HRESULT {
    if (e as i32) <= 0 {
        e as i32
    } else {
        hr_from_win32(e)
    }
}

pub unsafe fn enable_providers(handle: u64, providers: &[Provider], params: &EnableTraceParameters) -> HRESULT {
    for (i, pr) in providers.iter().enumerate() {
        let r = EnableTraceEx2(handle, &pr.guid, EVENT_CONTROL_CODE_ENABLE_PROVIDER, pr.level, pr.any, 0, 0, params);
        if r != 0 {
            imports::log_error(b"Couldn't enable provider index %u (err %u)\0", &[i as u64, r as u64]);
            return hr_win32(r);
        }
    }
    S_OK
}

pub unsafe fn start_etw_trace(file: &[u16], providers: &[Provider], buffer_kb: u32, min_buffers: u32, max_file_mb: u32, session: &GUID, name: &str) -> HRESULT {
    imports::log_info(b"Starting ETW capture, buffer size %u KB, min num buffers %u, max file size %u MB.\0", &[buffer_kb as u64, min_buffers as u64, max_file_mb as u64]);
    let host = host_path(file);
    let mut props = build_properties(&host[..host.len() - 1], buffer_kb, min_buffers, max_file_mb, session);
    let wname = wz(name);
    let mut handle = 0u64;
    let mut r = StartTraceW(&mut handle, wname.as_ptr(), props.bytes.as_mut_ptr());
    if r == ERROR_ALREADY_EXISTS {
        imports::log_info(b"ETW trace already running!  Restarting.\0", &[]);
        r = ControlTraceW(0, wname.as_ptr(), props.bytes.as_mut_ptr(), EVENT_TRACE_CONTROL_STOP);
        if r != 0 {
            imports::log_error(b"Couldn't stop pre-existing trace (err %u)!\0", &[r as u64]);
            return hr_win32(r);
        }
        r = StartTraceW(&mut handle, wname.as_ptr(), props.bytes.as_mut_ptr());
        if r != 0 {
            imports::log_error(b"Couldn't start new trace (err %u)!\0", &[r as u64]);
            return hr_win32(r);
        }
    } else if r == ERROR_BAD_PATHNAME {
        imports::log_error(b"Couldn't start trace (bad pathname)!\0", &[]);
        return hr_win32(r);
    } else if r != 0 {
        imports::log_error(b"Couldn't start trace (err %u)!\0", &[r as u64]);
        return hr_win32(r);
    }
    imports::log_info(b"Started ETW capture\0", &[]);
    let params = EnableTraceParameters { version: 1, enable_property: 0, control_flags: 0, source_id: guid_from(&props.bytes[0x18..0x28]), enable_filter_desc: null_mut(), filter_desc_count: 0 };
    let hr = enable_providers(handle, providers, &params);
    if hr < 0 {
        imports::log_error(b"Couldn't enable regular provider set: %08lX\0", &[hr as u32 as u64]);
        return hr;
    }
    S_OK
}

pub unsafe fn stop_etw_trace(file: &[u16], session: &GUID, name: &str, out: Option<&mut [u8; 0x78]>) {
    imports::log_info(b"Stopping ETW capture file.\0", &[]);
    let mut props = build_properties(file, 0, 0, 0, session);
    let wname = wz(name);
    let r = ControlTraceW(0, wname.as_ptr(), props.bytes.as_mut_ptr(), EVENT_TRACE_CONTROL_STOP);
    if r == ERROR_WMI_INSTANCE_NOT_FOUND {
        imports::log_error(b"No pre-existing trace is running!  Continuing.\0", &[]);
    } else if r != 0 {
        imports::log_error(b"Couldn't stop pre-existing trace (err %u)!  Continuing.\0", &[r as u64]);
    }
    if let Some(o) = out {
        o.copy_from_slice(&props.bytes[..0x78]);
        o[0x70..0x78].fill(0);
    }
}

pub unsafe fn stop_trace_session(out: Option<&mut [u8; 0x78]>) {
    stop_etw_trace(&[], &TRACE_SESSION_GUID, TRACE_SESSION_NAME, out);
}

pub unsafe fn xbox_net_api_diag_log_state(file: &[u16]) -> HRESULT {
    let hr = start_etw_trace(file, &STATE_PROVIDERS, 0x400, 4, 0, &STATE_SESSION_GUID, STATE_SESSION_NAME);
    if hr != 0 {
        imports::log_error(b"Couldn't open output log!\0", &[]);
        imports::log_error(b"Failed, error 0x%08x!\0", &[hr as u32 as u64]);
        return S_OK;
    }
    Sleep(0x10);
    wpp_message(&STATE_HEADER_GUID, 0xe, 0);
    crate::netstate::log_state_sections();
    stop_etw_trace(file, &STATE_SESSION_GUID, STATE_SESSION_NAME, None);
    S_OK
}

pub fn state_file_name(t: &SystemTime) -> String {
    format!("xbtitlestate_{:04}{:02}{:02}_{:02}{:02}{:02}.etl", t.year, t.month, t.day, t.hour, t.minute, t.second)
}

pub unsafe fn log_network_state() -> HRESULT {
    imports::log_info(b"LogNetworkState called\0", &[]);
    let mut dir = None;
    let hr = chk(get_trace_directory(&mut dir), b"_GetTraceDirectory(&strTraceDirectory)\0", b"LogNetworkState\0", 172);
    if hr < 0 {
        return hr;
    }
    let mut t: SystemTime = core::mem::zeroed();
    GetLocalTime(&mut t);
    let file = join(dir.as_deref(), &state_file_name(&t));
    chk(xbox_net_api_diag_log_state(&file), b"XboxNetApiDiagLogState(...)\0", b"LogNetworkState\0", 194)
}

pub unsafe fn start_advanced_multiplayer_trace() -> HRESULT {
    imports::log_info(b"StartAdvancedMultiplayerTrace called\0", &[]);
    let mut file = Vec::new();
    let hr = chk(get_trace_file_path(&mut file), b"GetTraceFilePath(&traceLogFilePath)\0", b"StartAdvancedMultiplayerTrace\0", 218);
    if hr < 0 {
        return hr;
    }
    let mut si: SystemInfo = core::mem::zeroed();
    GetSystemInfo(&mut si);
    let hr = start_etw_trace(&file, &TRACE_PROVIDERS, 0x200, si.number_of_processors * 4, 0x400, &TRACE_SESSION_GUID, TRACE_SESSION_NAME);
    if hr != 0 {
        imports::log_error(b"XboxNetApiDiagStartEtwTrace Failed, error 0x%08x!\0", &[hr as u32 as u64]);
    }
    let hr = chk(hr, b"XboxNetApiDiagStartEtwTrace( traceLogFilePath.Ptr(), ...)\0", b"StartAdvancedMultiplayerTrace\0", 276);
    if hr < 0 {
        return hr;
    }
    let hr = chk(log_network_state(), b"LogNetworkState()\0", b"StartAdvancedMultiplayerTrace\0", 279);
    if hr < 0 {
        stop_trace_session(None);
    }
    hr
}

pub unsafe fn stop_advanced_multiplayer_trace(events_lost: &mut u32) -> HRESULT {
    imports::log_info(b"StopAdvancedMultiplayerTrace called\0", &[]);
    *events_lost = 0;
    let mut file = Vec::new();
    let hr = chk(get_trace_file_path(&mut file), b"GetTraceFilePath(&traceLogFilePath)\0", b"StopAdvancedMultiplayerTrace\0", 307);
    if hr < 0 {
        return hr;
    }
    let mut props = [0u8; 0x78];
    stop_trace_session(Some(&mut props));
    *events_lost = u32::from_le_bytes(props[0x58..0x5c].try_into().unwrap());
    let hr = log_network_state();
    unmount_trace_xvd();
    chk(hr, b"LogNetworkState()\0", b"StopAdvancedMultiplayerTrace\0", 320)
}

pub unsafe fn start_trace_for_type(st: &mut ServerState, t: u32) -> HRESULT {
    let hr = match t {
        TRACE_TYPE_ADVANCED_MULTIPLAYER => start_advanced_multiplayer_trace(),
        TRACE_TYPE_XBOX_LIVE_SERVICES => E_ILLEGAL_METHOD_CALL,
        _ => {
            imports::log_error(b"Error: unsupported trace type\0", &[]);
            E_UNEXPECTED
        }
    };
    let hr = chk(hr, b"StartTraceForType\0", b"StartTraceForType\0", 610);
    if hr >= 0 {
        st.trace_type = t;
        st.start_tick = GetTickCount64();
    }
    hr
}

pub unsafe fn trace_start(body: Obj) -> HRESULT {
    let mut st = SERVER.lock().unwrap_or_else(|e| e.into_inner());
    imports::log_info(b"TraceStart called\0", &[]);
    let mut t = 0u32;
    let mut hr = if body.is_null() { chk(E_INVALIDARG, b"E_INVALIDARG\0", b"DeserializeTraceType\0", 43) } else { chk(get_u32(body, &wz("TraceType"), &mut t), b"XtfGetJsonProperty(traceData, TraceTypeProperty, &parsedTraceType)\0", b"DeserializeTraceType\0", 47) };
    if hr >= 0 && t.wrapping_sub(1) > 2 {
        hr = chk(E_INVALIDARG, b"E_INVALIDARG\0", b"DeserializeTraceType\0", 53);
    }
    'body: {
        if hr < 0 {
            imports::log_error(b"Error: trace data deserialization failed\0", &[]);
            break 'body;
        }
        if st.trace_type != 0 || st.pending_restart {
            imports::log_error(b"Error: trace is already running\0", &[]);
            hr = chk(E_ILLEGAL_STATE_CHANGE, b"E_ILLEGAL_STATE_CHANGE\0", b"TraceStart\0", 666);
            break 'body;
        }
        hr = chk(create_and_mount_trace_xvd(), b"CreateAndMountTraceXVD()\0", b"TraceStart\0", 671);
        if hr < 0 {
            break 'body;
        }
        hr = start_trace_for_type(&mut st, t);
        if hr < 0 {
            unmount_trace_xvd();
        }
        hr = chk(hr, b"StartTraceForType(traceType)\0", b"TraceStart\0", 683);
    }
    imports::log_info(if hr < 0 { b"TraceStart failed\0" } else { b"TraceStart succeeded\0" }, &[]);
    hr
}

pub unsafe fn trace_stop(response: *mut Obj) -> HRESULT {
    let mut st = SERVER.lock().unwrap_or_else(|e| e.into_inner());
    imports::log_info(b"TraceStop called\0", &[]);
    if response.is_null() {
        return chk(E_POINTER, b"E_POINTER\0", b"TraceStop\0", 719);
    }
    *response = null_mut();
    let mut reply = Ptr::null();
    let mut events_lost = 0u32;
    let mut hr;
    'body: {
        if st.trace_type == 0 {
            hr = chk(E_ILLEGAL_STATE_CHANGE, b"E_ILLEGAL_STATE_CHANGE\0", b"TraceStop\0", 729);
            break 'body;
        }
        hr = match st.trace_type {
            TRACE_TYPE_ADVANCED_MULTIPLAYER => stop_advanced_multiplayer_trace(&mut events_lost),
            TRACE_TYPE_XBOX_LIVE_SERVICES => E_ILLEGAL_METHOD_CALL,
            _ => E_UNEXPECTED,
        };
        hr = chk(hr, b"StopTraceForType\0", b"TraceStop\0", 798);
        if hr < 0 {
            break 'body;
        }
        let duration = GetTickCount64().wrapping_sub(st.start_tick);
        st.start_tick = 0;
        hr = match imports::get() {
            Ok(i) => (i.create_json_object)(reply.out()),
            Err(e) => e,
        };
        hr = chk(hr, b"XbtpCreateJsonObject(&replyBodyObject)\0", b"TraceStop\0", 804);
        if hr < 0 {
            break 'body;
        }
        hr = chk(obj_set_number(reply.0, wz("TraceDurationMs").as_ptr(), duration as i64 as f64), b"XtfSetJsonProperty(replyBodyObject, TraceDurationMsProperty, totalTraceTime)\0", b"TraceStop\0", 805);
        if hr < 0 {
            break 'body;
        }
        hr = chk(set_u32(reply.0, &wz("TraceEventsLost"), events_lost), b"XtfSetJsonProperty(replyBodyObject, TraceEventsLostProperty, eventsLost)\0", b"TraceStop\0", 806);
        if hr < 0 {
            break 'body;
        }
        hr = chk(set_u32(reply.0, &wz("TraceType"), st.trace_type), b"XtfSetJsonProperty(replyBodyObject, TraceTypeProperty, s_traceType)\0", b"TraceStop\0", 807);
        if hr < 0 {
            break 'body;
        }
        *response = reply.detach();
    }
    st.trace_type = 0;
    hr
}

pub unsafe fn mount_pending_trace_xvd(trace_type: &mut u32) -> HRESULT {
    let mut hr = chk(mount_trace_xvd(), b"MountTraceXVD()\0", b"MountPendingTraceXVD\0", 272);
    if hr < 0 {
        return hr;
    }
    let file = join(trace_directory().as_deref(), PENDING_FILE);
    let host = host_path(&file);
    let h = CreateFileW(host.as_ptr(), 0x8000_0000, 0, null_mut(), 3, 0x80, null_mut());
    if h == INVALID_HANDLE_VALUE {
        let e = GetLastError();
        hr = chk(hr_win32(e), b"GetLastError()\0", b"MountPendingTraceXVD\0", 281);
        if e != 0 {
            unmount_trace_xvd();
            return hr;
        }
    } else {
        let mut read = 0u32;
        if ReadFile(h, trace_type as *mut u32 as *mut c_void, 4, &mut read, null_mut()) == 0 {
            hr = chk(hr_win32(GetLastError()), b"GetLastError()\0", b"MountPendingTraceXVD\0", 286);
        } else if read != 4 {
            hr = chk(E_UNEXPECTED, b"E_UNEXPECTED\0", b"MountPendingTraceXVD\0", 290);
        }
        if CloseHandle(h) == 0 || DeleteFileW(host.as_ptr()) == 0 {
            if hr >= 0 {
                hr = hr_win32(GetLastError());
            }
        }
    }
    if hr < 0 {
        unmount_trace_xvd();
    }
    hr
}

pub unsafe fn check_for_pending_trace_xvd() -> HRESULT {
    let mut st = SERVER.lock().unwrap_or_else(|e| e.into_inner());
    let mut t = 0u32;
    let mut hr = mount_pending_trace_xvd(&mut t);
    if hr == HR_PATH_NOT_FOUND {
        return hr;
    }
    hr = chk(hr, b"MountPendingTraceXVD(&traceType)\0", b"CheckForPendingTraceXVD\0", 840);
    if hr < 0 {
        return hr;
    }
    hr = chk(start_trace_for_type(&mut st, t), b"StartTraceForType(traceType)\0", b"CheckForPendingTraceXVD\0", 844);
    if hr < 0 {
        unmount_trace_xvd();
    }
    hr
}

pub struct TraceServer;

pub static TRACE_SERVER: TraceServer = TraceServer;

impl OperationService for TraceServer {
    unsafe fn route(&self, op: u32, body: Obj, response: *mut Obj) -> HRESULT {
        if response.is_null() {
            return chk(E_POINTER, b"E_POINTER\0", b"RouteOperationRequest\0", 291);
        }
        match op {
            OP_TRACE_START => {
                let hr = chk(trace_start(body), b"TraceStart(opBody)\0", b"RouteOperationRequest\0", 389);
                if hr >= 0 {
                    imports::log_info(b"Started Trace\0", &[]);
                }
                hr
            }
            OP_TRACE_STOP => chk(trace_stop(response), b"TraceStop(opResponse)\0", b"RouteOperationRequest\0", 394),
            OP_LOG_NETWORK_STATE => chk(log_network_state(), b"LogNetworkState()\0", b"RouteOperationRequest\0", 400),
            _ => {
                imports::log_error(b"Failed: operation unknown (%u)\0", &[op as u64]);
                S_OK
            }
        }
    }

    unsafe fn check_pending(&self) -> HRESULT {
        check_for_pending_trace_xvd()
    }

    unsafe fn thread_start(&self) {
        wpp_init();
    }
}

#[no_mangle]
pub unsafe extern "C" fn XtfOrdinal_10() -> HRESULT {
    crate::monitor::start_operation_request_monitor(&TRACE_SERVER)
}
