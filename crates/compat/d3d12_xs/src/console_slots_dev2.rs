use core::ffi::c_void;
use core::sync::atomic::{AtomicUsize, Ordering};
use compat_common::record;

use crate::device_wrap::{real_device, Guid};

const S_OK: i32 = 0;
const E_FAIL: i32 = 0x8000_4005u32 as i32;
const E_INVALIDARG: i32 = 0x8007_0057u32 as i32;
const E_NOINTERFACE: i32 = 0x8000_4002u32 as i32;
const E_NOTIMPL: i32 = 0x8000_4001u32 as i32;

pub const SLOT_CONSOLE_INFO: usize = 68;
pub const SLOT_PLACED_SRV: usize = 70;
pub const SLOT_PLACED_UAV: usize = 71;
pub const SLOT_DRIVER_LEVEL: usize = 72;
pub const SLOT_COMMITTED_X: usize = 76;
pub const SLOT_COMPONENT_PLACED: usize = 77;
pub const SLOT_COMMITTED_OPAQUE: usize = 83;
pub const SLOT_PLACED_OPAQUE: usize = 84;
pub const SLOT_OPAQUE_ALLOC_INFO: usize = 85;
pub const SLOT_GATED_UAV: usize = 86;
pub const SLOT_MULTI_FENCE: usize = 88;

const DEV_CREATE_SRV: usize = 18;
const DEV_CREATE_UAV: usize = 19;
const DEV_COMMITTED: usize = 27;
const DEV_PLACED: usize = 29;
const DEV_PLACED_X: usize = 44;
const FENCE_GET_COMPLETED: usize = 8;
const HEAP_GET_DESC: usize = 8;
const HEAP_FLAG_ALLOW_DISPLAY: u32 = 0x8;
const XBOX_ONE_X_DEVKIT: u32 = 5;
const OPAQUE_LAYOUT: u32 = 0x10d;
const DXGI_R32_UINT: u32 = 0x2a;
const FLAG_ALLOW_UAV: u32 = 0x4;

pub const IID_RESOURCE: Guid = Guid { d1: 0x6964_42be, d2: 0xa72e, d3: 0x4059, d4: [0xbc, 0x79, 0x5b, 0x5c, 0x98, 0x04, 0x0f, 0xad] };

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    XboxOne,
    Scarlett,
}

pub fn family(device_type: u32) -> Family {
    if (2..=5).contains(&device_type) { Family::XboxOne } else { Family::Scarlett }
}

fn device_type() -> u32 {
    compat_common::gdk::identity::device_type_value()
}

fn once(c: &AtomicUsize, n: usize) -> bool {
    c.fetch_add(1, Ordering::Relaxed) < n
}

unsafe fn vt(obj: *mut c_void) -> *const usize {
    unsafe { *(obj as *const *const usize) }
}

unsafe fn slot(obj: *mut c_void, i: usize) -> usize {
    unsafe { *vt(obj).add(i) }
}

pub fn guid_eq(a: &Guid, b: &Guid) -> bool {
    a.d1 == b.d1 && a.d2 == b.d2 && a.d3 == b.d3 && a.d4 == b.d4
}

fn riid_is_resource(riid: *const Guid) -> bool {
    !riid.is_null() && guid_eq(unsafe { &*riid }, &IID_RESOURCE)
}

pub fn console_type(fam: Family, device_type: u32) -> u32 {
    match fam {
        Family::XboxOne => match device_type {
            3 => 1,
            4 | 7 => 2,
            5 | 8 => 3,
            _ => 0,
        },
        Family::Scarlett => match device_type {
            3 => 1,
            4 => 2,
            5 => 3,
            6 => 4,
            8 => 6,
            _ => 5,
        },
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KmdInfo {
    pub word0: u64,
    pub mode: u32,
    pub compute_units: u32,
}

pub fn kmd_info(device_type: u32) -> KmdInfo {
    let g = xgs_kmd_emu::gpu_info(device_type);
    KmdInfo { word0: g.frequency_hz, mode: g.mode, compute_units: g.compute_units }
}

pub fn kmd_reply(device_type: u32) -> [u8; 16] {
    xgs_kmd_emu::gpu_reply(device_type)
}

pub fn console_info(fam: Family, device_type: u32) -> [u8; 16] {
    let reply = kmd_reply(device_type);
    let mut out = [0u8; 16];
    out[0..8].copy_from_slice(&reply[0..8]);
    out[8..12].copy_from_slice(&console_type(fam, device_type).to_le_bytes());
    if fam == Family::XboxOne {
        out[12..16].copy_from_slice(&reply[12..16]);
    }
    out
}

unsafe extern "system" fn console_info_x(_this: *mut c_void, out: *mut [u8; 16]) {
    static N: AtomicUsize = AtomicUsize::new(0);
    let dt = device_type();
    let info = console_info(family(dt), dt);
    if !out.is_null() {
        unsafe { core::ptr::write_unaligned(out, info) };
    }
    if once(&N, 4) {
        record("d3d12_xs", "device68", &format!("device type {dt} -> console type {} compute units {} (emulated IOCTL 0x7900c1fc reply {:?})", u32::from_le_bytes(info[8..12].try_into().unwrap()), u32::from_le_bytes(info[12..16].try_into().unwrap()), kmd_info(dt)));
    }
}

pub fn set_driver_level(_dev: usize, v: u32) -> i32 {
    crate::kmd_slots::set_driver_level(v)
}

#[allow(dead_code)]
pub fn driver_level(_dev: usize) -> Option<u32> {
    crate::kmd_slots::driver_level()
}

unsafe extern "system" fn driver_level_x(this: *mut c_void, v: u32) -> i32 {
    static N: AtomicUsize = AtomicUsize::new(0);
    let hr = set_driver_level(this as usize, v);
    if hr != S_OK || once(&N, 8) {
        record("d3d12_xs", "device72", &format!("value {v} -> {hr:#010x} (IOCTL 0x7900c1f8 on the emulated KMD; 5..10 accepted)"));
    }
    hr
}

pub fn format_bytes(f: u32) -> Option<u32> {
    match f {
        2..=4 => Some(16),
        6..=8 => Some(12),
        10..=14 | 16..=18 => Some(8),
        23..=26 | 27..=32 | 34..=38 | 39..=43 | 67 | 87 | 88 | 91 => Some(4),
        48..=52 | 54..=59 => Some(2),
        60..=65 => Some(1),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BufferView {
    pub first: usize,
    pub stride: usize,
    pub flags: usize,
    pub format: usize,
}

pub const SRV_BUFFER: BufferView = BufferView { first: 0x10, stride: 0x1c, flags: 0x20, format: 0 };
pub const UAV_BUFFER: BufferView = BufferView { first: 0x08, stride: 0x14, flags: 0x20, format: 0 };

fn rd32(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}

fn rd64(b: &[u8], o: usize) -> u64 {
    u64::from_le_bytes(b[o..o + 8].try_into().unwrap())
}

pub fn element_bytes(view: &[u8], at: BufferView) -> Option<u32> {
    let stride = rd32(view, at.stride);
    if stride != 0 {
        return Some(stride);
    }
    if rd32(view, at.flags) & 1 != 0 {
        return Some(4);
    }
    format_bytes(rd32(view, at.format))
}

pub fn rebase_buffer_view(view: &mut [u8], at: BufferView, offset: u64) -> bool {
    if offset == 0 {
        return true;
    }
    match element_bytes(view, at) {
        Some(e) if offset % e as u64 == 0 => {
            let first = rd64(view, at.first) + offset / e as u64;
            view[at.first..at.first + 8].copy_from_slice(&first.to_le_bytes());
            true
        }
        _ => false,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Existing(usize, u64),
    Place,
}

pub fn choose_target(found: Option<(crate::gpu_va::Entry, u64)>, resource_is_buffer: bool) -> Target {
    match found {
        Some((e, off)) if e.buffer && resource_is_buffer => Target::Existing(e.res, off),
        Some((e, 0)) if !e.buffer && !resource_is_buffer => Target::Existing(e.res, 0),
        _ => Target::Place,
    }
}

unsafe fn place_at(this: *mut c_void, va: u64, rdesc: *const u8, state: u32) -> Option<usize> {
    let f: unsafe extern "system" fn(*mut c_void, u64, *const u8, u32, *const c_void, *const Guid, *mut *mut c_void) -> i32 =
        unsafe { core::mem::transmute(slot(this, DEV_PLACED_X)) };
    let mut out: *mut c_void = core::ptr::null_mut();
    let hr = unsafe { f(this, va, rdesc, state, core::ptr::null(), &IID_RESOURCE, &mut out) };
    (hr >= 0 && !out.is_null()).then_some(out as usize)
}

fn same_view_texture(expected: &[u8; 0x38], physical: &[u8; 0x38]) -> bool {
    let format = rd32(expected, 0x20);
    let physical_format = rd32(physical, 0x20);
    expected[..4] == physical[..4]
        && expected[0x10..0x20] == physical[0x10..0x20]
        && expected[0x24..0x2c] == physical[0x24..0x2c]
        && crate::title_heap::group(format) == crate::title_heap::group(physical_format)
}

unsafe fn resolve(this: *mut c_void, va: u64, rdesc: *const u8, view: &mut [u8], at: BufferView, state: u32, what: &str) -> Option<usize> {
    let rd: [u8; 0x38] = unsafe { core::ptr::read_unaligned(rdesc as *const [u8; 0x38]) };
    if va == 0 {
        return None;
    }
    let is_buffer = rd32(&rd, 0) == 1;
    if let Target::Existing(res, off) = choose_target(crate::gpu_va::lookup_gpu(va), is_buffer) {
        if is_buffer && rebase_buffer_view(view, at, off) {
            return Some(res);
        }
        if !is_buffer && crate::placed::lookup(res as *mut c_void)
            .is_some_and(|placement| same_view_texture(&rd, &placement.desc))
        {
            return Some(res);
        }
    }
    let made = unsafe { place_at(this, va, rdesc, state) };
    static N: AtomicUsize = AtomicUsize::new(0);
    if made.is_none() || once(&N, 8) {
        record("d3d12_xs", what, &format!("va={va:#x} dim={} format={} -> placement at the view address {:?}", rd32(&rd, 0), rd32(&rd, 0x20), made.map(|r| r as *const c_void)));
    }
    made
}

fn null_view_dimension(resource_dim: u32, srv: bool) -> u32 {
    match (resource_dim, srv) {
        (1, _) => 1,
        (2, _) => 2,
        (4, true) => 7,
        (4, false) => 8,
        _ => 4,
    }
}

unsafe fn null_view(this: *mut c_void, rdesc: *const u8, dest: usize, srv: bool) {
    let real = unsafe { real_device(this) };
    let (dim, fmt) = if rdesc.is_null() { (3, 0) } else { unsafe { (core::ptr::read_unaligned(rdesc as *const u32), core::ptr::read_unaligned(rdesc.add(0x20) as *const u32)) } };
    let mut d = [0u8; 0x28];
    let format = if fmt == 0 && dim == 1 {
        // D3D12 requires an element interpretation even for a null buffer view.
        // R32_TYPELESS with the RAW flag is valid for both descriptor classes.
        d[32..36].copy_from_slice(&1u32.to_le_bytes());
        39
    } else if fmt == 0 {
        28
    } else {
        crate::device_ext_a::typed_view_format(fmt)
    };
    d[0..4].copy_from_slice(&format.to_le_bytes());
    d[4..8].copy_from_slice(&null_view_dimension(dim, srv).to_le_bytes());
    if srv {
        d[8..12].copy_from_slice(&0x1688u32.to_le_bytes());
        let f: unsafe extern "system" fn(*mut c_void, *mut c_void, *const u8, usize) = unsafe { core::mem::transmute(slot(real, DEV_CREATE_SRV)) };
        unsafe { f(real, core::ptr::null_mut(), d.as_ptr(), dest) };
    } else {
        let f: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void, *const u8, usize) = unsafe { core::mem::transmute(slot(real, DEV_CREATE_UAV)) };
        unsafe { f(real, core::ptr::null_mut(), core::ptr::null_mut(), d.as_ptr(), dest) };
    }
}

pub fn placed_view_rule_breaks(flags: u32, srv: bool) -> Vec<&'static str> {
    let mut v = Vec::new();
    if flags & 1 != 0 { v.push("ALLOW_RENDER_TARGET"); }
    if flags & 2 != 0 { v.push("ALLOW_DEPTH_STENCIL"); }
    if srv && flags & 8 != 0 { v.push("DENY_SHADER_RESOURCE"); }
    if !srv && flags & FLAG_ALLOW_UAV == 0 { v.push("missing ALLOW_UNORDERED_ACCESS"); }
    v
}

fn note_rules(rdesc: *const u8, srv: bool, what: &str) {
    static N: AtomicUsize = AtomicUsize::new(0);
    let flags = unsafe { core::ptr::read_unaligned(rdesc.add(0x30) as *const u32) };
    let bad = placed_view_rule_breaks(flags, srv);
    if !bad.is_empty() && once(&N, 8) {
        record("d3d12_xs", what, &format!("resource flags {flags:#x} break the debug-driver rules ({}); the release driver builds the view anyway", bad.join(", ")));
    }
}

unsafe extern "system" fn placed_srv_x(this: *mut c_void, rdesc: *const u8, desc: *const u8, dest: usize) {
    if desc.is_null() || rdesc.is_null() {
        unsafe { null_view(this, rdesc, dest, true) };
        return;
    }
    note_rules(rdesc, true, "CreatePlacedRawShaderResourceViewX");
    let x: [u8; 0x48] = unsafe { core::ptr::read_unaligned(desc as *const [u8; 0x48]) };
    let mut view: [u8; 0x28] = x[..0x28].try_into().unwrap();
    let va = rd64(&x, 0x28);
    let Some(res) = (unsafe { resolve(this, va, rdesc, &mut view, SRV_BUFFER, 0, "CreatePlacedRawShaderResourceViewX") }) else {
        unsafe { null_view(this, rdesc, dest, true) };
        return;
    };
    let real = unsafe { real_device(this) };
    let f: unsafe extern "system" fn(*mut c_void, *mut c_void, *const u8, usize) = unsafe { core::mem::transmute(slot(real, DEV_CREATE_SRV)) };
    unsafe { f(real, res as *mut c_void, view.as_ptr(), dest) };
}

unsafe extern "system" fn placed_uav_x(this: *mut c_void, rdesc: *const u8, desc: *const u8, dest: usize) {
    if desc.is_null() || rdesc.is_null() {
        unsafe { null_view(this, rdesc, dest, false) };
        return;
    }
    note_rules(rdesc, false, "CreatePlacedUnorderedAccessViewX");
    let x: [u8; 0x40] = unsafe { core::ptr::read_unaligned(desc as *const [u8; 0x40]) };
    let mut view: [u8; 0x28] = x[..0x28].try_into().unwrap();
    let va = rd64(&x, 0x28);
    let counter_va = rd64(&x, 0x30);
    let Some(res) = (unsafe { resolve(this, va, rdesc, &mut view, UAV_BUFFER, 0, "CreatePlacedUnorderedAccessViewX") }) else {
        unsafe { null_view(this, rdesc, dest, false) };
        return;
    };
    let counter = match counter_va {
        0 | u64::MAX => core::ptr::null_mut(),
        cva => match crate::gpu_va::lookup_gpu(cva) {
            Some((e, off)) if e.buffer => {
                view[0x18..0x20].copy_from_slice(&off.to_le_bytes());
                e.res as *mut c_void
            }
            _ => {
                record("d3d12_xs", "CreatePlacedUnorderedAccessViewX", &format!("counter va {cva:#x} is not inside a known buffer; view created without a counter"));
                core::ptr::null_mut()
            }
        },
    };
    let real = unsafe { real_device(this) };
    let f: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void, *const u8, usize) = unsafe { core::mem::transmute(slot(real, DEV_CREATE_UAV)) };
    unsafe { f(real, res as *mut c_void, counter, view.as_ptr(), dest) };
}

unsafe extern "system" fn committed_x(
    this: *mut c_void, heap: *const u32, heap_flags: u32, desc: *const u8,
    state: u32, clear: *const c_void, riid: *const Guid, ppv: *mut *mut c_void,
) -> i32 {
    if !riid_is_resource(riid) {
        return E_NOINTERFACE;
    }
    if desc.is_null() || heap.is_null() {
        return E_INVALIDARG;
    }
    let f: unsafe extern "system" fn(*mut c_void, *const u32, u32, *const u8, u32, *const c_void, *const Guid, *mut *mut c_void) -> i32 =
        unsafe { core::mem::transmute(slot(this, DEV_COMMITTED)) };
    unsafe { f(this, heap, heap_flags, desc, state, clear, riid, ppv) }
}

unsafe extern "system" fn component_placed_x(
    this: *mut c_void, locations: *const u64, desc: *const u8, state: u32,
    clear: *const c_void, riid: *const Guid, ppv: *mut *mut c_void,
) -> i32 {
    if !riid_is_resource(riid) {
        return E_NOINTERFACE;
    }
    if locations.is_null() || desc.is_null() {
        return E_INVALIDARG;
    }
    let va = unsafe { core::ptr::read_unaligned(locations) };
    static N: AtomicUsize = AtomicUsize::new(0);
    if once(&N, 8) {
        let meta = unsafe { (core::ptr::read_unaligned(locations.add(1)), core::ptr::read_unaligned(locations.add(2))) };
        record("d3d12_xs", "CreateComponentPlacedResourceX", &format!("surface va={va:#x} metadata components {:#x}/{:#x} (compression metadata is managed by the PC driver)", meta.0, meta.1));
    }
    let f: unsafe extern "system" fn(*mut c_void, u64, *const u8, u32, *const c_void, *const Guid, *mut *mut c_void) -> i32 =
        unsafe { core::mem::transmute(slot(this, DEV_PLACED_X)) };
    unsafe { f(this, va, desc, state, clear, riid, ppv) }
}

pub fn preview_gate(device_type: u32) -> bool {
    device_type == XBOX_ONE_X_DEVKIT
}

fn gate(what: &str) -> bool {
    static ERR: AtomicUsize = AtomicUsize::new(0);
    static WARN: AtomicUsize = AtomicUsize::new(0);
    if preview_gate(device_type()) {
        if once(&WARN, 1) {
            record("d3d12_xs", what, "WARNING: This preview API is only functional on Xbox One X development kit hardware");
        }
        true
    } else {
        if once(&ERR, 8) {
            record("d3d12_xs", what, "ERROR: This preview API is only available on Xbox One X development kit hardware.");
        }
        false
    }
}

pub fn opaque_desc(desc: &[u8; 0x38], opaque_type: i32) -> [u8; 0x38] {
    let mut d = *desc;
    if opaque_type == 1 {
        d[0x28..0x2c].copy_from_slice(&0u32.to_le_bytes());
        d[0x2c..0x30].copy_from_slice(&OPAQUE_LAYOUT.to_le_bytes());
        d[0x20..0x24].copy_from_slice(&DXGI_R32_UINT.to_le_bytes());
        d[0x1e..0x20].copy_from_slice(&1u16.to_le_bytes());
        d[0x24..0x28].copy_from_slice(&1u32.to_le_bytes());
        d[0x30..0x34].copy_from_slice(&FLAG_ALLOW_UAV.to_le_bytes());
    }
    d
}

unsafe extern "system" fn committed_opaque_x(
    this: *mut c_void, heap: *const u32, heap_flags: u32, opaque_type: i32,
    desc: *const u8, state: u32, riid: *const Guid, ppv: *mut *mut c_void,
) -> i32 {
    if !gate("CreateCommittedOpaqueResourceX") {
        return E_FAIL;
    }
    if desc.is_null() {
        return E_INVALIDARG;
    }
    let d = opaque_desc(&unsafe { core::ptr::read_unaligned(desc as *const [u8; 0x38]) }, opaque_type);
    if !riid_is_resource(riid) {
        return E_NOINTERFACE;
    }
    let pc = crate::device_x::pc_resource_desc(&d);
    let f: unsafe extern "system" fn(*mut c_void, *const u32, u32, *const u8, u32, *const c_void, *const Guid, *mut *mut c_void) -> i32 =
        unsafe { core::mem::transmute(slot(this, DEV_COMMITTED)) };
    unsafe { f(this, heap, heap_flags, pc.as_ptr(), state, core::ptr::null(), riid, ppv) }
}

unsafe fn heap_flags(heap: *mut c_void) -> u32 {
    let mut desc = [0u8; 0x30];
    let f: unsafe extern "system" fn(*mut c_void, *mut u8) -> *mut u8 = unsafe { core::mem::transmute(slot(heap, HEAP_GET_DESC)) };
    unsafe { f(heap, desc.as_mut_ptr()) };
    rd32(&desc, 0x28)
}

unsafe extern "system" fn placed_opaque_x(
    this: *mut c_void, heap: *mut c_void, offset: u64, opaque_type: i32,
    desc: *const u8, state: u32, riid: *const Guid, ppv: *mut *mut c_void,
) -> i32 {
    if !riid_is_resource(riid) {
        return E_NOINTERFACE;
    }
    if !gate("CreatePlacedOpaqueResourceX") {
        return E_FAIL;
    }
    if desc.is_null() {
        return E_INVALIDARG;
    }
    if !heap.is_null() && unsafe { heap_flags(heap) } & HEAP_FLAG_ALLOW_DISPLAY != 0 {
        return E_INVALIDARG;
    }
    let d = opaque_desc(&unsafe { core::ptr::read_unaligned(desc as *const [u8; 0x38]) }, opaque_type);
    if heap.is_null() {
        let f: unsafe extern "system" fn(*mut c_void, u64, *const u8, u32, *const c_void, *const Guid, *mut *mut c_void) -> i32 =
            unsafe { core::mem::transmute(slot(this, DEV_PLACED_X)) };
        return unsafe { f(this, offset, d.as_ptr(), state, core::ptr::null(), riid, ppv) };
    }
    let pc = crate::device_x::pc_resource_desc(&d);
    let f: unsafe extern "system" fn(*mut c_void, *mut c_void, u64, *const u8, u32, *const c_void, *const Guid, *mut *mut c_void) -> i32 =
        unsafe { core::mem::transmute(slot(this, DEV_PLACED)) };
    unsafe { f(this, heap, offset, pc.as_ptr(), state, core::ptr::null(), riid, ppv) }
}

pub fn opaque_allocation_info(descs: &[[u8; 0x38]], types: &[i32], mut texture: impl FnMut(&[u8; 0x38]) -> Option<(u64, u64)>) -> [u64; 2] {
    let (mut size, mut align) = (0u64, 1u64);
    for (i, src) in descs.iter().enumerate() {
        let d = opaque_desc(src, types[i]);
        let (s, a) = if rd32(&d, 0) == 1 {
            (rd64(src, 0x10), rd64(&d, 8).max(4))
        } else {
            match texture(&d) {
                Some(v) => v,
                None => return [0, 0],
            }
        };
        align = align.max(a);
        size = ((size + a - 1) & !(a - 1)) + s;
    }
    [size, align]
}

fn xg_create() -> usize {
    static P: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *P.get_or_init(|| {
        extern "system" {
            fn GetModuleHandleW(n: *const u16) -> *mut c_void;
            fn LoadLibraryW(n: *const u16) -> *mut c_void;
            fn GetProcAddress(h: *mut c_void, n: *const u8) -> *mut c_void;
        }
        let name: Vec<u16> = "xg_x.dll".encode_utf16().chain(core::iter::once(0)).collect();
        let mut h = unsafe { GetModuleHandleW(name.as_ptr()) };
        if h.is_null() {
            h = unsafe { LoadLibraryW(name.as_ptr()) };
        }
        if h.is_null() { 0 } else { unsafe { GetProcAddress(h, b"XGCreateTextureComputer\0".as_ptr()) as usize } }
    })
}

pub unsafe fn xg_texture_size(create: usize, d: &[u8; 0x38]) -> Option<(u64, u64)> {
    if create == 0 {
        return None;
    }
    let f: unsafe extern "system" fn(*const u8, *mut *mut c_void) -> i32 = unsafe { core::mem::transmute(create) };
    let mut c: *mut c_void = core::ptr::null_mut();
    if unsafe { f(d.as_ptr(), &mut c) } < 0 || c.is_null() {
        return None;
    }
    let mut layout = vec![0u8; 0x2000];
    let get: unsafe extern "system" fn(*mut c_void, *mut u8) -> i32 = unsafe { core::mem::transmute(slot(c, 2)) };
    unsafe { get(c, layout.as_mut_ptr()) };
    let rel: unsafe extern "system" fn(*mut c_void) -> u32 = unsafe { core::mem::transmute(slot(c, 1)) };
    unsafe { rel(c) };
    Some((rd64(&layout, 0), rd64(&layout, 8)))
}

unsafe extern "system" fn opaque_alloc_info_x(
    out: *mut [u64; 2], _this: *mut c_void, _mask: u32, n: u32, types: *const i32, descs: *const [u8; 0x38],
) -> *mut [u64; 2] {
    let r = if !gate("GetOpaqueResourceAllocationInfoX") {
        [0, 0]
    } else if n == 0 {
        [0, 1]
    } else if types.is_null() || descs.is_null() {
        [0, 0]
    } else {
        let d: Vec<[u8; 0x38]> = (0..n as usize).map(|i| unsafe { core::ptr::read_unaligned(descs.add(i)) }).collect();
        let t = unsafe { core::slice::from_raw_parts(types, n as usize) };
        let create = xg_create();
        opaque_allocation_info(&d, t, |x| unsafe { xg_texture_size(create, x) })
    };
    unsafe { core::ptr::write_unaligned(out, r) };
    out
}

unsafe extern "system" fn gated_uav_x(this: *mut c_void, res: *mut c_void, _counter: *mut c_void, desc: *const u8, dest: usize) {
    if !gate("CreateUnorderedAccessView (opaque preview)") {
        return;
    }
    let f: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void, *const u8, usize) =
        unsafe { core::mem::transmute(slot(this, DEV_CREATE_UAV)) };
    unsafe { f(this, res, core::ptr::null_mut(), desc, dest) }
}

pub fn fences_done(n: usize, any: bool, mask: &mut [u64], mut complete: impl FnMut(usize) -> bool) -> bool {
    for i in 0..n {
        if mask[i >> 6] >> (i & 63) & 1 != 0 {
            continue;
        }
        if complete(i) {
            mask[i >> 6] |= 1 << (i & 63);
            if any {
                return true;
            }
        }
    }
    if any {
        return false;
    }
    let words = (n.wrapping_sub(1) >> 6) + 1;
    let last = (n.wrapping_sub(1) & 63) + 1;
    (0..words.min(mask.len())).all(|w| {
        let full = if w + 1 == words { if last == 64 { u64::MAX } else { (1u64 << last) - 1 } } else { u64::MAX };
        mask[w] == full
    }) && n != 0
}

pub fn wait_fences(n: usize, any: bool, mut complete: impl FnMut(usize) -> bool, mut idle: impl FnMut()) {
    let mut mask = vec![0u64; ((n.max(1) - 1) >> 6) + 1];
    while !fences_done(n, any, &mut mask, &mut complete) {
        idle();
    }
}

unsafe extern "system" fn multi_fence_x(_this: *mut c_void, fences: *const *mut c_void, values: *const u64, n: u32, flags: u32, event: *mut c_void) -> i32 {
    extern "system" {
        fn Sleep(ms: u32);
        fn SetEvent(h: isize) -> i32;
    }
    let complete = |i: usize| {
        let f = unsafe { *fences.add(i) };
        let get: unsafe extern "system" fn(*mut c_void) -> u64 = unsafe { core::mem::transmute(slot(f, FENCE_GET_COMPLETED)) };
        unsafe { get(f) >= *values.add(i) }
    };
    wait_fences(n as usize, flags & 1 != 0, complete, || unsafe { Sleep(0) });
    unsafe { SetEvent(event as isize) };
    S_OK
}

unsafe extern "system" fn scarlett_notimpl(_this: *mut c_void) -> i32 {
    E_NOTIMPL
}

pub fn plan(fam: Family) -> Vec<(usize, usize)> {
    let mut p: Vec<(usize, usize)> = vec![
        (SLOT_CONSOLE_INFO, console_info_x as usize),
        (SLOT_PLACED_SRV, placed_srv_x as usize),
        (SLOT_PLACED_UAV, placed_uav_x as usize),
        (SLOT_DRIVER_LEVEL, driver_level_x as usize),
        (SLOT_COMMITTED_X, committed_x as usize),
        (SLOT_COMPONENT_PLACED, component_placed_x as usize),
    ];
    match fam {
        Family::XboxOne => {
            p.push((SLOT_GATED_UAV, gated_uav_x as usize));
            p.push((SLOT_MULTI_FENCE, multi_fence_x as usize));
            p.push((SLOT_COMMITTED_OPAQUE, committed_opaque_x as usize));
            p.push((SLOT_PLACED_OPAQUE, placed_opaque_x as usize));
            p.push((SLOT_OPAQUE_ALLOC_INFO, opaque_alloc_info_x as usize));
        }
        Family::Scarlett => {
            p.push((SLOT_COMMITTED_OPAQUE, scarlett_notimpl as usize));
            p.push((SLOT_PLACED_OPAQUE, multi_fence_x as usize));
            p.push((SLOT_OPAQUE_ALLOC_INFO, scarlett_notimpl as usize));
            p.push((SLOT_MULTI_FENCE, scarlett_notimpl as usize));
        }
    }
    p
}

pub fn install(v: &mut [usize]) -> usize {
    let fam = family(device_type());
    let mut n = 0;
    for (s, f) in plan(fam) {
        if s < v.len() {
            v[s] = f;
            n += 1;
        }
    }
    record("d3d12_xs", "console_slots_dev2", &format!("{n} device slots installed for the {fam:?} driver layout"));
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn family_follows_device_type() {
        for t in 2..=5 { assert_eq!(family(t), Family::XboxOne); }
        for t in [0, 1, 6, 7, 8] { assert_eq!(family(t), Family::Scarlett); }
    }

    #[test]
    fn console_type_matches_release_drivers() {
        let one: Vec<u32> = (0..=9).map(|t| console_type(Family::XboxOne, t)).collect();
        assert_eq!(one, vec![0, 0, 0, 1, 2, 3, 0, 2, 3, 0]);
        let sc: Vec<u32> = (0..=9).map(|t| console_type(Family::Scarlett, t)).collect();
        assert_eq!(sc, vec![5, 5, 5, 1, 2, 3, 4, 5, 6, 5]);
    }

    #[test]
    fn console_info_layout() {
        let i = console_info(Family::Scarlett, 7);
        assert_eq!(&i[0..8], &1_825_000_000u64.to_le_bytes());
        assert_eq!(u32::from_le_bytes(i[8..12].try_into().unwrap()), 5);
        assert_eq!(&i[12..16], &[0; 4]);
        let x = console_info(Family::XboxOne, 4);
        assert_eq!(u32::from_le_bytes(x[8..12].try_into().unwrap()), 2);
        assert_eq!(u32::from_le_bytes(x[12..16].try_into().unwrap()), 40);
    }

    #[test]
    fn kmd_reply_per_console() {
        let cu = |t: u32| u32::from_le_bytes(kmd_reply(t)[12..16].try_into().unwrap());
        let mode = |t: u32| u32::from_le_bytes(kmd_reply(t)[8..12].try_into().unwrap());
        assert_eq!([cu(2), cu(3), cu(4), cu(5), cu(6), cu(7), cu(8)], [12, 12, 40, 44, 20, 52, 52]);
        assert_eq!([mode(2), mode(3), mode(4), mode(5)], [0, 0, 1, 1]);
        assert_eq!([mode(6), mode(7), mode(8)], [1, 2, 2]);
        assert_eq!(&kmd_reply(7)[0..8], &1_825_000_000u64.to_le_bytes());
    }

    #[test]
    fn driver_level_range() {
        let _g = crate::kmd_slots::test_serial();
        for v in [0, 4, 11, 0xffff_ffff] {
            assert_eq!(set_driver_level(0x1234, v), E_INVALIDARG);
        }
        for v in 5..=10 {
            assert_eq!(set_driver_level(0x1234, v), S_OK);
            assert_eq!(driver_level(0x1234), Some(v));
        }
        assert_eq!(set_driver_level(0x1234, 3), E_INVALIDARG);
        assert_eq!(driver_level(0x1234), Some(10));
    }

    #[test]
    fn buffer_views_rebase_by_element() {
        let mut v = [0u8; 0x28];
        v[0x1c..0x20].copy_from_slice(&16u32.to_le_bytes());
        v[0x10..0x18].copy_from_slice(&3u64.to_le_bytes());
        assert!(rebase_buffer_view(&mut v, SRV_BUFFER, 64));
        assert_eq!(rd64(&v, 0x10), 7);
        assert!(!rebase_buffer_view(&mut v, SRV_BUFFER, 8));
        let mut u = [0u8; 0x28];
        u[0x20..0x24].copy_from_slice(&1u32.to_le_bytes());
        assert!(rebase_buffer_view(&mut u, UAV_BUFFER, 12));
        assert_eq!(rd64(&u, 0x08), 3);
        let mut f = [0u8; 0x28];
        f[0..4].copy_from_slice(&2u32.to_le_bytes());
        assert!(rebase_buffer_view(&mut f, SRV_BUFFER, 32));
        assert_eq!(rd64(&f, 0x10), 2);
        let mut unk = [0u8; 0x28];
        assert!(!rebase_buffer_view(&mut unk, SRV_BUFFER, 4));
        assert!(rebase_buffer_view(&mut unk, SRV_BUFFER, 0));
    }

    #[test]
    fn view_targets() {
        let buf = crate::gpu_va::Entry { start: 0x1000, len: 0x1000, res: 7, buffer: true };
        let tex = crate::gpu_va::Entry { start: 0x8000, len: 0x1000, res: 9, buffer: false };
        assert_eq!(choose_target(Some((buf, 0x40)), true), Target::Existing(7, 0x40));
        assert_eq!(choose_target(Some((buf, 0x40)), false), Target::Place);
        assert_eq!(choose_target(Some((tex, 0)), false), Target::Existing(9, 0));
        assert_eq!(choose_target(Some((tex, 0x100)), false), Target::Place);
        assert_eq!(choose_target(None, true), Target::Place);
    }

    #[test]
    fn placed_view_rules() {
        assert!(placed_view_rule_breaks(0, true).is_empty());
        assert_eq!(placed_view_rule_breaks(0xb, true), vec!["ALLOW_RENDER_TARGET", "ALLOW_DEPTH_STENCIL", "DENY_SHADER_RESOURCE"]);
        assert_eq!(placed_view_rule_breaks(0, false), vec!["missing ALLOW_UNORDERED_ACCESS"]);
        assert!(placed_view_rule_breaks(4, false).is_empty());
        assert_eq!(null_view_dimension(1, true), 1);
        assert_eq!(null_view_dimension(4, true), 7);
        assert_eq!(null_view_dimension(4, false), 8);
        assert_eq!(null_view_dimension(3, false), 4);
    }

    #[test]
    fn riid_check() {
        assert!(riid_is_resource(&IID_RESOURCE));
        assert!(!riid_is_resource(core::ptr::null()));
        let other = Guid { d1: 1, ..IID_RESOURCE };
        assert!(!riid_is_resource(&other));
    }

    #[test]
    fn preview_api_needs_one_x_devkit() {
        assert!(preview_gate(5));
        for t in [0, 1, 2, 3, 4, 6, 7, 8] { assert!(!preview_gate(t)); }
    }

    #[test]
    fn opaque_type_one_rewrites_desc() {
        let mut d = [0u8; 0x38];
        d[0] = 3;
        d[0x10..0x18].copy_from_slice(&256u64.to_le_bytes());
        d[0x1e] = 9;
        d[0x20] = 28;
        let o = opaque_desc(&d, 1);
        assert_eq!(rd32(&o, 0x20), 0x2a);
        assert_eq!(u16::from_le_bytes([o[0x1e], o[0x1f]]), 1);
        assert_eq!(rd32(&o, 0x24), 1);
        assert_eq!(rd32(&o, 0x28), 0);
        assert_eq!(rd32(&o, 0x2c), 0x10d);
        assert_eq!(rd32(&o, 0x30), 4);
        assert_eq!(rd64(&o, 0x10), 256);
        assert_eq!(opaque_desc(&d, 0), d);
        assert_eq!(opaque_desc(&d, 2), d);
    }

    #[test]
    fn opaque_allocation_accumulates_like_release() {
        let mut b = [0u8; 0x38];
        b[0] = 1;
        b[0x10..0x18].copy_from_slice(&100u64.to_le_bytes());
        let mut t = [0u8; 0x38];
        t[0] = 3;
        let r = opaque_allocation_info(&[b, t, b], &[0, 0, 0], |_| Some((0x10000, 0x10000)));
        assert_eq!(r, [0x20000 + 100, 0x10000]);
        let mut b2 = b;
        b2[8..16].copy_from_slice(&256u64.to_le_bytes());
        assert_eq!(opaque_allocation_info(&[b, b2], &[0, 0], |_| None), [256 + 100, 256]);
        assert_eq!(opaque_allocation_info(&[b, t], &[0, 0], |_| None), [0, 0]);
        assert_eq!(opaque_allocation_info(&[], &[], |_| None), [0, 1]);
    }

    #[test]
    fn fence_wait_all_and_any() {
        let done = [true, false, true];
        let mut m = vec![0u64; 1];
        assert!(!fences_done(3, false, &mut m, |i| done[i]));
        assert_eq!(m[0], 0b101);
        assert!(fences_done(3, true, &mut vec![0u64; 1], |i| done[i]));
        assert!(fences_done(2, false, &mut vec![0u64; 1], |_| true));
        assert!(!fences_done(3, true, &mut vec![0u64; 1], |_| false));
        let mut big = vec![0u64; 2];
        assert!(fences_done(64, false, &mut big[..1], |_| true));
        let mut m2 = vec![0u64; 2];
        assert!(fences_done(100, false, &mut m2, |_| true));
        assert_eq!(m2, vec![u64::MAX, (1u64 << 36) - 1]);
    }

    #[test]
    fn fence_wait_polls_until_done() {
        let mut ticks = 0u32;
        let polls = core::cell::Cell::new(0u32);
        wait_fences(70, false, |i| { polls.set(polls.get() + 1); i % 2 == 0 || polls.get() > 200 }, || ticks += 1);
        assert!(ticks >= 1);
        let mut idle = 0;
        wait_fences(5, true, |i| i == 4, || idle += 1);
        assert_eq!(idle, 0);
    }

    #[test]
    fn plans_follow_driver_family() {
        let one = plan(Family::XboxOne);
        let sc = plan(Family::Scarlett);
        let get = |p: &Vec<(usize, usize)>, s: usize| p.iter().find(|x| x.0 == s).map(|x| x.1);
        assert_eq!(get(&one, 83), Some(committed_opaque_x as usize));
        assert_eq!(get(&sc, 83), Some(scarlett_notimpl as usize));
        assert_eq!(get(&sc, 84), Some(multi_fence_x as usize));
        assert_eq!(get(&one, 88), Some(multi_fence_x as usize));
        assert_eq!(get(&sc, 85), Some(scarlett_notimpl as usize));
        assert_eq!(get(&one, 86), Some(gated_uav_x as usize));
        assert_eq!(get(&sc, 86), None);
        assert_eq!(get(&sc, 88), Some(scarlett_notimpl as usize));
        assert_eq!(unsafe { scarlett_notimpl(core::ptr::null_mut()) }, E_NOTIMPL);
        for s in [68, 70, 71, 72, 76, 77, 83, 84, 85, 88] {
            assert!(get(&one, s).is_some() && get(&sc, s).is_some(), "slot {s}");
        }
        assert_eq!((one.len(), sc.len()), (11, 10));
        let mut v = vec![0usize; 160];
        assert_eq!(install(&mut v), plan(family(device_type())).len());
        assert!(v[82] == 0);
    }

    #[test]
    fn device68_writes_struct() {
        let mut out = [0xffu8; 16];
        unsafe { console_info_x(core::ptr::null_mut(), &mut out) };
        let dt = device_type();
        assert_eq!(out, console_info(family(dt), dt));
    }

    #[test]
    fn device72_hook_validates() {
        let _g = crate::kmd_slots::test_serial();
        let dev = 0x5555usize as *mut c_void;
        assert_eq!(unsafe { driver_level_x(dev, 11) }, E_INVALIDARG);
        assert_eq!(unsafe { driver_level_x(dev, 6) }, S_OK);
        assert_eq!(driver_level(0x5555), Some(6));
    }

    #[test]
    fn opaque_entry_points_fail_off_devkit() {
        if preview_gate(device_type()) {
            return;
        }
        assert_eq!(unsafe { committed_opaque_x(core::ptr::null_mut(), core::ptr::null(), 0, 1, core::ptr::null(), 0, &IID_RESOURCE, core::ptr::null_mut()) }, E_FAIL);
        assert_eq!(unsafe { placed_opaque_x(core::ptr::null_mut(), core::ptr::null_mut(), 0, 1, core::ptr::null(), 0, &IID_RESOURCE, core::ptr::null_mut()) }, E_FAIL);
        let bad = Guid { d1: 2, ..IID_RESOURCE };
        assert_eq!(unsafe { placed_opaque_x(core::ptr::null_mut(), core::ptr::null_mut(), 0, 1, core::ptr::null(), 0, &bad, core::ptr::null_mut()) }, E_NOINTERFACE);
        let mut out = [9u64; 2];
        unsafe { opaque_alloc_info_x(&mut out, core::ptr::null_mut(), 0, 1, core::ptr::null(), core::ptr::null()) };
        assert_eq!(out, [0, 0]);
        unsafe { gated_uav_x(core::ptr::null_mut(), core::ptr::null_mut(), core::ptr::null_mut(), core::ptr::null(), 0) };
    }

    #[test]
    fn resource_creators_check_riid_first() {
        let bad = Guid { d1: 3, ..IID_RESOURCE };
        assert_eq!(unsafe { committed_x(core::ptr::null_mut(), core::ptr::null(), 0, core::ptr::null(), 0, core::ptr::null(), &bad, core::ptr::null_mut()) }, E_NOINTERFACE);
        assert_eq!(unsafe { component_placed_x(core::ptr::null_mut(), core::ptr::null(), core::ptr::null(), 0, core::ptr::null(), &bad, core::ptr::null_mut()) }, E_NOINTERFACE);
        assert_eq!(unsafe { committed_x(core::ptr::null_mut(), core::ptr::null(), 0, core::ptr::null(), 0, core::ptr::null(), &IID_RESOURCE, core::ptr::null_mut()) }, E_INVALIDARG);
        assert_eq!(unsafe { component_placed_x(core::ptr::null_mut(), core::ptr::null(), core::ptr::null(), 0, core::ptr::null(), &IID_RESOURCE, core::ptr::null_mut()) }, E_INVALIDARG);
    }

    #[test]
    fn multi_fence_signals_after_real_fences() {
        #[repr(C)]
        struct F { vt: *const usize, v: u64 }
        unsafe extern "system" fn get(f: *mut c_void) -> u64 { unsafe { (*(f as *mut F)).v } }
        let mut table = [0usize; 12];
        table[FENCE_GET_COMPLETED] = get as usize;
        let mut a = F { vt: table.as_ptr(), v: 5 };
        let mut b = F { vt: table.as_ptr(), v: 9 };
        let fences = [&mut a as *mut F as *mut c_void, &mut b as *mut F as *mut c_void];
        let values = [5u64, 9];
        extern "system" {
            fn CreateEventW(a: *mut c_void, manual: i32, init: i32, name: *const u16) -> isize;
            fn WaitForSingleObject(h: isize, ms: u32) -> u32;
            fn CloseHandle(h: isize) -> i32;
        }
        let ev = unsafe { CreateEventW(core::ptr::null_mut(), 1, 0, core::ptr::null()) };
        assert_eq!(unsafe { multi_fence_x(core::ptr::null_mut(), fences.as_ptr(), values.as_ptr(), 2, 0, ev as *mut c_void) }, S_OK);
        assert_eq!(unsafe { WaitForSingleObject(ev, 0) }, 0);
        unsafe { CloseHandle(ev) };
    }
}
