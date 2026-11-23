use core::ffi::c_void;
use core::sync::atomic::{AtomicUsize, Ordering};
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use compat_common::record;

use crate::device_wrap::{real_device, Guid};
use crate::sample_pattern::{SAMPLE_PATTERN, SAMPLE_PATTERN_FIRST};

pub const S_OK: i32 = 0;
pub const E_FAIL: i32 = 0x8000_4005u32 as i32;
pub const E_NOTIMPL: i32 = 0x8000_4001u32 as i32;
pub const E_INVALIDARG: i32 = 0x8007_0057u32 as i32;
pub const E_OUTOFMEMORY: i32 = 0x8007_000eu32 as i32;

pub const SLOT_CREATE_PLACED_RESOURCE_X2: usize = 45;
pub const SLOT_PLACED_SRV: usize = 46;
pub const SLOT_PLACED_UAV: usize = 47;
pub const SLOT_SET_DRIVER_OPTION: usize = 49;
pub const SLOT_GPU_CRASH: usize = 51;
pub const SLOT_KERNEL_OPTION_10001: usize = 58;
pub const SLOT_SAMPLE_PATTERN: usize = 59;
pub const SLOT_DERIVED_COMPUTE: usize = 63;
pub const SLOT_COMPONENT_PLACED: usize = 77;

const PC_CREATE_SRV: usize = 18;
const PC_CREATE_UAV: usize = 19;
const PC_CREATE_COMPUTE_PSO: usize = 11;
const RES_GET_DESC: usize = 10;

pub const KERNEL_OPTION_FIXED_ID: u32 = 0x10001;
pub const NO_ADDRESS: u64 = u64::MAX;

const IID_PIPELINE_STATE: Guid = Guid { d1: 0x765a_30f3, d2: 0xf624, d3: 0x4c6f, d4: [0xa8, 0x28, 0xac, 0xe9, 0x48, 0x62, 0x24, 0x45] };

fn once(c: &AtomicUsize, n: usize) -> bool {
    c.fetch_add(1, Ordering::Relaxed) < n
}

fn rd32(d: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(d[o..o + 4].try_into().unwrap())
}

fn rd64(d: &[u8], o: usize) -> u64 {
    u64::from_le_bytes(d[o..o + 8].try_into().unwrap())
}

fn wr32(d: &mut [u8], o: usize, v: u32) {
    d[o..o + 4].copy_from_slice(&v.to_le_bytes());
}

fn wr64(d: &mut [u8], o: usize, v: u64) {
    d[o..o + 8].copy_from_slice(&v.to_le_bytes());
}

unsafe fn vfn(obj: *mut c_void, slot: usize) -> usize {
    unsafe { *(*(obj as *const *const usize)).add(slot) }
}

pub fn component_desc(desc: &[u8; 0x38]) -> [u8; 0x40] {
    let mut d = [0u8; 0x40];
    d[..0x38].copy_from_slice(desc);
    let flags = rd32(desc, 0x30);
    if flags & (1 << 22) != 0 {
        wr32(&mut d, 0x38, 0);
        wr32(&mut d, 0x34, !(flags >> 22) & 2);
    }
    d
}

type ComponentPlacedFn = unsafe extern "system" fn(*mut c_void, *const c_void, *const u8, u32, *const c_void, *const Guid, *mut *mut c_void) -> i32;

pub unsafe extern "system" fn create_placed_resource_x2(
    this: *mut c_void, locations: *const c_void, pdesc: *const u8, state: u32,
    clear: *const c_void, riid: *const Guid, ppv: *mut *mut c_void,
) -> i32 {
    let desc: [u8; 0x38] = unsafe { core::ptr::read_unaligned(pdesc as *const [u8; 0x38]) };
    let d = component_desc(&desc);
    let f: ComponentPlacedFn = unsafe { core::mem::transmute(vfn(this, SLOT_COMPONENT_PLACED)) };
    unsafe { f(this, locations, d.as_ptr(), state, clear, riid, ppv) }
}

pub fn dxgi_format_bytes(f: u32) -> Option<u64> {
    Some(match f {
        1..=4 => 16,
        5..=8 => 12,
        9..=22 => 8,
        23..=47 | 67 | 68 | 87..=93 => 4,
        48..=59 | 85 | 86 | 115 => 2,
        60..=65 => 1,
        _ => return None,
    })
}

pub fn promote_depth_view_format(view: u32, resource: u32, disabled: bool) -> u32 {
    if !disabled && (19..=22).contains(&resource) && (44..=47).contains(&view) {
        view - 25
    } else {
        view
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub res: usize,
    pub first_element_delta: u64,
}

pub fn placement_for(buffer: bool, elem: Option<u64>, hit: Option<(crate::gpu_va::Entry, u64)>) -> Result<Placement, &'static str> {
    let Some((e, off)) = hit else { return Err("address is not owned by any tracked resource") };
    if buffer {
        if !e.buffer {
            return Err("address is texture memory but the view is a buffer view");
        }
        let Some(n) = elem.filter(|&n| n != 0) else { return Err("buffer element size is unknown for this format") };
        if off % n != 0 {
            return Err("address is not a whole number of elements into the buffer");
        }
        return Ok(Placement { res: e.res, first_element_delta: off / n });
    }
    if e.buffer {
        return Err("address is buffer memory but the view is a texture view");
    }
    if off != 0 {
        return Err("address is inside a texture, not at its start");
    }
    Ok(Placement { res: e.res, first_element_delta: 0 })
}

pub fn srv_element_size(pc: &[u8]) -> Option<u64> {
    let (fmt, stride, flags) = (rd32(pc, 0), rd32(pc, 0x1c), rd32(pc, 0x20));
    if flags & 1 != 0 { Some(4) } else if stride != 0 { Some(stride as u64) } else { dxgi_format_bytes(fmt) }
}

pub fn uav_element_size(pc: &[u8]) -> Option<u64> {
    let (fmt, stride, flags) = (rd32(pc, 0), rd32(pc, 0x14), rd32(pc, 0x20));
    if flags & 1 != 0 { Some(4) } else if stride != 0 { Some(stride as u64) } else { dxgi_format_bytes(fmt) }
}

const NULL_SRV: [u8; 0x28] = {
    let mut d = [0u8; 0x28];
    d[0] = 28;
    d[4] = 4;
    d[8] = 0x88;
    d[9] = 0x16;
    d[0x14] = 0xff;
    d[0x15] = 0xff;
    d[0x16] = 0xff;
    d[0x17] = 0xff;
    d
};

const NULL_UAV: [u8; 0x28] = {
    let mut d = [0u8; 0x28];
    d[0] = 28;
    d[4] = 4;
    d
};

static VIEW_LOG: AtomicUsize = AtomicUsize::new(0);

unsafe fn resource_desc(res: *mut c_void) -> [u8; 0x38] {
    let mut d = [0u8; 0x38];
    let f: unsafe extern "system" fn(*mut c_void, *mut u8) -> *mut u8 = unsafe { core::mem::transmute(vfn(res, RES_GET_DESC)) };
    unsafe { f(res, d.as_mut_ptr()) };
    d
}

pub fn typed_view_format(f: u32) -> u32 {
    match f {
        1 => 2,
        5 => 6,
        9 => 10,
        15 => 16,
        19 => 21,
        23 => 24,
        27 => 28,
        33 => 34,
        39 => 41,
        44 => 46,
        48 => 49,
        53 => 56,
        60 => 61,
        70 => 71,
        73 => 74,
        76 => 77,
        79 => 80,
        82 => 83,
        90 => 87,
        92 => 88,
        94 => 95,
        97 => 98,
        f => f,
    }
}

pub fn fit_texture_view(pc: &mut [u8], res: &[u8; 0x38], uav: bool) -> bool {
    let (rdim, rfmt, samples) = (rd32(res, 0), rd32(res, 0x20), rd32(res, 0x24));
    if rdim < 2 {
        return false;
    }
    let mut changed = false;
    if rd32(pc, 0) == 0 {
        wr32(pc, 0, typed_view_format(rfmt));
        changed = true;
    }
    if !uav && samples > 1 {
        match rd32(pc, 4) {
            4 => {
                wr32(pc, 4, 6);
                pc[0x10..0x28].fill(0);
                changed = true;
            }
            5 => {
                let (first, size) = (rd32(pc, 0x18), rd32(pc, 0x1c));
                wr32(pc, 4, 7);
                pc[0x10..0x28].fill(0);
                wr32(pc, 0x10, first);
                wr32(pc, 0x14, size);
                changed = true;
            }
            _ => {}
        }
    }
    changed
}

#[derive(Debug, PartialEq)]
pub enum ViewPlan {
    Null,
    Default(usize),
    Desc(usize, Vec<u8>, usize, u64),
}

pub unsafe fn plan_placed_view(res: *mut c_void, pdesc: *const u8, uav: bool, lookup: &dyn Fn(u64) -> Option<(crate::gpu_va::Entry, u64)>) -> ViewPlan {
    if res.is_null() {
        return ViewPlan::Null;
    }
    if pdesc.is_null() {
        return ViewPlan::Default(res as usize);
    }
    let size = if uav { 0x40 } else { 0x48 };
    let x = unsafe { core::slice::from_raw_parts(pdesc, size) }.to_vec();
    let mut pc = x[..0x28].to_vec();
    let rdesc = unsafe { resource_desc(res) };
    let fmt = promote_depth_view_format(rd32(&pc, 0), rd32(&rdesc, 0x20), driver_option(17).unwrap_or(0) != 0);
    wr32(&mut pc, 0, fmt);
    static FIT_LOG: AtomicUsize = AtomicUsize::new(0);
    if fit_texture_view(&mut pc, &rdesc, uav) && once(&FIT_LOG, 16) {
        record("d3d12_xs", "PlacedViewX", &format!("texture view (resource fmt={} samples={}) given view fmt={} dim={}: PC texture views need a typed format and an MSAA dimension for multisampled resources, so they are taken from the resource",rd32(&rdesc, 0x20), rd32(&rdesc, 0x24), rd32(&pc, 0), rd32(&pc, 4)));
    }
    if !uav && rd32(&rdesc, 0x24) > 1 {
        crate::fmask_check::note_view(res as usize, rd32(&pc, 8), rd32(&x, 0x34));
    }
    static MS_LOG: AtomicUsize = AtomicUsize::new(0);
    if !uav && rd32(&rdesc, 0x24) > 1 && (crate::rtcap::enabled() || crate::passcap::on()) && once(&MS_LOG, 64) {
        let hex: Vec<String> = x.chunks(4).map(|c| format!("{:08x}", u32::from_le_bytes([c[0], c[1], c[2], c[3]]))).collect();
        record("d3d12_xs", "PlacedViewX", &format!("MS SRV on res {:#x} (fmt={} samples={}): raw Xbox desc dwords [{}] -> PC fmt={} dim={} map={:#x}", res as usize, rd32(&rdesc, 0x20), rd32(&rdesc, 0x24), hex.join(" "), rd32(&pc, 0), rd32(&pc, 4), rd32(&pc, 8)));
    }
    let extras: Vec<(usize, u32)> = if uav {
        vec![(0x38, rd32(&x, 0x38))]
    } else {
        let mut v = vec![(0x38, rd32(&x, 0x38)), (0x3c, rd32(&x, 0x3c)), (0x40, rd32(&x, 0x40))];
        if fmt == 0 {
            v.push((0x30, rd32(&x, 0x30)));
            v.push((0x34, rd32(&x, 0x34)));
        }
        v
    };
    for (o, v) in extras.into_iter().filter(|e| e.1 != 0) {
        if once(&VIEW_LOG, 16) {
            record("d3d12_xs", "PlacedViewX", &format!("desc+{o:#x} = {v:#x} is a raw hardware-descriptor field with no D3D12 PC view field; view built from the standard part"));
        }
    }
    let buffer = rd32(&pc, 4) == 1;
    let mut target = res as usize;
    let va = rd64(&x, 0x28);
    if va != NO_ADDRESS {
        let elem = if uav { uav_element_size(&pc) } else { srv_element_size(&pc) };
        match placement_for(buffer, elem, lookup(va)) {
            Ok(p) => {
                target = p.res;
                let fe = if uav { 8 } else { 0x10 };
                if buffer {
                    let v = rd64(&pc, fe).wrapping_add(p.first_element_delta);
                    wr64(&mut pc, fe, v);
                }
            }
            Err(why) => {
                if once(&VIEW_LOG, 16) {
                    record("d3d12_xs", "PlacedViewX", &format!("placed address {va:#x}: {why}; null descriptor written"));
                }
                return ViewPlan::Null;
            }
        }
    }
    let mut counter = 0usize;
    if uav && buffer {
        let cva = rd64(&x, 0x30);
        if cva != NO_ADDRESS {
            match lookup(cva) {
                Some((e, off)) if e.buffer => {
                    counter = e.res;
                    wr64(&mut pc, 0x18, off);
                }
                _ => {
                    if once(&VIEW_LOG, 16) {
                        record("d3d12_xs", "PlacedViewX", &format!("UAV counter address {cva:#x} is not inside a tracked buffer; null descriptor written"));
                    }
                    return ViewPlan::Null;
                }
            }
        }
    }
    ViewPlan::Desc(target, pc, counter, va)
}

pub unsafe fn emit_srv(real: *mut c_void, plan: ViewPlan, dest: usize) {
    let f: unsafe extern "system" fn(*mut c_void, *mut c_void, *const u8, usize) = unsafe { core::mem::transmute(vfn(real, PC_CREATE_SRV)) };
    match plan {
        ViewPlan::Null => unsafe { f(real, core::ptr::null_mut(), NULL_SRV.as_ptr(), dest) },
        ViewPlan::Default(r) => unsafe { f(real, r as *mut c_void, core::ptr::null(), dest) },
        ViewPlan::Desc(r, d, _, _) => unsafe { f(real, r as *mut c_void, d.as_ptr(), dest) },
    }
}

pub unsafe fn emit_uav(real: *mut c_void, plan: ViewPlan, dest: usize) {
    let f: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void, *const u8, usize) = unsafe { core::mem::transmute(vfn(real, PC_CREATE_UAV)) };
    match plan {
        ViewPlan::Null => unsafe { f(real, core::ptr::null_mut(), core::ptr::null_mut(), NULL_UAV.as_ptr(), dest) },
        ViewPlan::Default(r) => unsafe { f(real, r as *mut c_void, core::ptr::null_mut(), core::ptr::null(), dest) },
        ViewPlan::Desc(r, d, c, _) => unsafe { f(real, r as *mut c_void, c as *mut c_void, d.as_ptr(), dest) },
    }
}

pub unsafe extern "system" fn create_placed_srv_x(this: *mut c_void, res: *mut c_void, pdesc: *const u8, dest: usize) {
    let plan = unsafe { plan_placed_view(res, pdesc, false, &crate::gpu_va::lookup) };
    unsafe { emit_srv(real_device(this), plan, dest) }
}

pub unsafe extern "system" fn create_placed_uav_x(this: *mut c_void, res: *mut c_void, pdesc: *const u8, dest: usize) {
    let plan = unsafe { plan_placed_view(res, pdesc, true, &crate::gpu_va::lookup) };
    unsafe { emit_uav(real_device(this), plan, dest) }
}

#[derive(Default)]
pub struct DriverState {
    pub globals: HashMap<u32, u32>,
    pub kernel: HashMap<u32, u32>,
    pub dev_flags: HashMap<usize, u32>,
    pub fatal_marker: HashSet<usize>,
    pub shader_cache: HashMap<usize, usize>,
    pub reported: HashSet<usize>,
}

pub trait DriverHost {
    fn file_exists(&mut self, path: &str) -> bool;
    fn shader_cache_create(&mut self) -> Result<usize, i32>;
    fn shader_cache_destroy(&mut self, cache: usize);
    fn poke(&mut self, addr: u64) -> bool;
    fn hang_analysis(&mut self, mode: u32);
}

pub const DEV_FLAG_DEADDEAD: u32 = 1;
pub const DEV_FLAG_BE7A: u32 = 4;

pub fn kernel_set_option(st: &mut DriverState, id: u32, v: u32) -> i32 {
    st.kernel.insert(id, v);
    S_OK
}

pub fn set_driver_option(st: &mut DriverState, host: &mut dyn DriverHost, dev: usize, id: u32, v: u32) -> i32 {
    let nz = (v != 0) as u32;
    match id {
        3 | 9 | 10 => return S_OK,
        4 => return E_NOTIMPL,
        15 | 18 => {
            st.globals.insert(id, nz);
            return S_OK;
        }
        16 => {
            if v != 0 {
                if st.shader_cache.contains_key(&dev) {
                    return S_OK;
                }
                if host.file_exists("g:\\D3D_DisableShaderCache") {
                    return E_INVALIDARG;
                }
                return match host.shader_cache_create() {
                    Ok(c) => {
                        st.shader_cache.insert(dev, c);
                        S_OK
                    }
                    Err(hr) => hr,
                };
            }
            if let Some(c) = st.shader_cache.remove(&dev) {
                host.shader_cache_destroy(c);
            }
            return S_OK;
        }
        0xbe7a => {
            *st.dev_flags.entry(dev).or_default() |= DEV_FLAG_BE7A;
            return S_OK;
        }
        0xbaaad411 => {
            host.hang_analysis(2);
            return S_OK;
        }
        0x14 | 17 | 19 | 0x100 => {
            st.globals.insert(id, nz);
        }
        0x16 => {
            st.globals.insert(id, (v == 0) as u32);
        }
        0x101..=0x103 => {
            st.globals.insert(id, v);
        }
        0x104 => {
            if v != 0 && !host.poke((v as u64) << 12) {
                record("d3d12_xs", "SetDriverOptionX", &format!("option 0x104: page {:#x} is not writable; the console would fault on this store", (v as u64) << 12));
            }
        }
        0xdeaddead => {
            *st.dev_flags.entry(dev).or_default() |= DEV_FLAG_DEADDEAD;
            st.globals.insert(id, 1);
            st.fatal_marker.insert(dev);
        }
        _ => {}
    }
    kernel_set_option(st, id, v)
}

static DRIVER: Mutex<Option<DriverState>> = Mutex::new(None);

fn with_driver<R>(f: impl FnOnce(&mut DriverState) -> R) -> R {
    let mut g = DRIVER.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(DriverState::default))
}

pub fn driver_option(id: u32) -> Option<u32> {
    with_driver(|s| s.globals.get(&id).copied())
}

pub fn kernel_option(id: u32) -> Option<u32> {
    with_driver(|s| s.kernel.get(&id).copied())
}

extern "system" {
    fn GetFileAttributesA(name: *const u8) -> u32;
    fn LoadLibraryW(name: *const u16) -> *mut c_void;
    fn GetProcAddress(m: *mut c_void, name: *const u8) -> *mut c_void;
    fn FreeLibrary(m: *mut c_void) -> i32;
    fn VirtualQuery(addr: *const c_void, info: *mut [u64; 6], len: usize) -> usize;
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

static SCACHE_API: Mutex<Option<Option<(usize, usize)>>> = Mutex::new(None);

fn scache_api() -> Option<(usize, usize)> {
    let mut g = SCACHE_API.lock().unwrap_or_else(|e| e.into_inner());
    *g.get_or_insert_with(|| unsafe {
        let mut m = LoadLibraryW(wide("scache.dll").as_ptr());
        if m.is_null() {
            m = LoadLibraryW(wide("scache_lnm.dll").as_ptr());
        }
        if m.is_null() {
            return None;
        }
        let names: [&[u8]; 9] = [b"ShaderCache_Create\0", b"ShaderCache_Destroy\0", b"ShaderCache_ComputeLocalHashKey\0", b"ShaderCache_FindLocalValue\0", b"ShaderCache_AddLocalValue\0", b"ShaderCache_FreeValue\0", b"ShaderCache_FindRemoteValue\0", b"ShaderCache_AddRemoteValue\0", b"ShaderCache_Flush\0"];
        let p: Vec<usize> = names.iter().map(|n| GetProcAddress(m, n.as_ptr()) as usize).collect();
        if p.iter().any(|&a| a == 0) {
            FreeLibrary(m);
            return None;
        }
        Some((p[0], p[1]))
    })
}

static POKE_TARGET: u64 = 0;

struct Win32Host;

impl DriverHost for Win32Host {
    fn file_exists(&mut self, path: &str) -> bool {
        let mut c = path.as_bytes().to_vec();
        c.push(0);
        unsafe { GetFileAttributesA(c.as_ptr()) != u32::MAX }
    }
    fn shader_cache_create(&mut self) -> Result<usize, i32> {
        let Some((create, _)) = scache_api() else { return Err(E_FAIL) };
        static NAME: [u16; 8] = [b'S' as u16, b'C' as u16, b'a' as u16, b'c' as u16, b'h' as u16, b'e' as u16, b'E' as u16, 0];
        let mut p = [0u8; 0x40];
        wr32(&mut p, 0x10, 4);
        wr32(&mut p, 0x14, 0x10017);
        wr64(&mut p, 0x18, NAME.as_ptr() as u64);
        wr32(&mut p, 0x20, 4);
        wr32(&mut p, 0x24, 1);
        wr32(&mut p, 0x28, 0x10_0000);
        wr32(&mut p, 0x38, 0x4000_0000);
        let mut out = 0usize;
        let f: unsafe extern "system" fn(*const u8, *mut usize) -> i32 = unsafe { core::mem::transmute(create) };
        let hr = unsafe { f(p.as_ptr(), &mut out) };
        if hr < 0 { Err(hr) } else { Ok(out) }
    }
    fn shader_cache_destroy(&mut self, cache: usize) {
        if let Some((_, destroy)) = scache_api() {
            let f: unsafe extern "system" fn(usize) = unsafe { core::mem::transmute(destroy) };
            unsafe { f(cache) };
        }
    }
    fn poke(&mut self, addr: u64) -> bool {
        let mut info = [0u64; 6];
        if unsafe { VirtualQuery(addr as *const c_void, &mut info, 48) } == 0 {
            return false;
        }
        let state = info[4] as u32;
        let protect = (info[4] >> 32) as u32;
        if state != 0x1000 || protect & 0xcc == 0 {
            return false;
        }
        unsafe { core::ptr::write_volatile(addr as *mut u64, &POKE_TARGET as *const u64 as u64) };
        true
    }
    fn hang_analysis(&mut self, mode: u32) {
        record("d3d12_xs", "XHIT", &format!("hang analysis mode {mode}: D3D ERROR: Failed to load XHIT GPU hang analysis DLL. (xhit.dll drives RDNA2 hardware registers and has no PC host)"));
    }
}

pub unsafe extern "system" fn set_driver_option_x(this: *mut c_void, id: u32, value: u32) -> i32 {
    static N: AtomicUsize = AtomicUsize::new(0);
    let hr = with_driver(|s| set_driver_option(s, &mut Win32Host, this as usize, id, value));
    if hr < 0 || once(&N, 16) {
        record("d3d12_xs", "SetDriverOptionX", &format!("id={id:#x} value={value:#x} -> {hr:#010x}"));
    }
    hr
}

pub unsafe extern "system" fn kernel_option_10001_x(_this: *mut c_void, value: u32) -> i32 {
    static N: AtomicUsize = AtomicUsize::new(0);
    let hr = with_driver(|s| kernel_set_option(s, KERNEL_OPTION_FIXED_ID, value));
    if once(&N, 8) {
        record("d3d12_xs", "KernelOptionX", &format!("kernel option {KERNEL_OPTION_FIXED_ID:#x} = {value:#x} -> {hr:#010x}"));
    }
    hr
}

#[derive(Debug, PartialEq)]
pub enum CrashAction {
    HangAnalysisOnly,
    Ignored,
    Fatal,
}

pub fn gpu_crash(st: &mut DriverState, host: &mut dyn DriverHost, dev: usize, flags: u32) -> CrashAction {
    if flags & 1 != 0 {
        host.hang_analysis(2);
        return CrashAction::HangAnalysisOnly;
    }
    if !st.reported.insert(dev) {
        return CrashAction::Ignored;
    }
    record("d3d12_xs", "GpuCrashX", "!!! XBOX UMD FATAL ERROR: GPU has crashed.");
    host.hang_analysis(2);
    kernel_set_option(st, 0xbaaad911, 2);
    CrashAction::Fatal
}

pub unsafe extern "system" fn gpu_crash_x(this: *mut c_void, flags: u32) {
    let a = with_driver(|s| gpu_crash(s, &mut Win32Host, this as usize, flags));
    if a == CrashAction::Fatal {
        unsafe { core::arch::asm!("int3") };
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct SamplePattern {
    pub positions: [[i8; 32]; 4],
    pub centroid: [u8; 16],
    pub regs: [u32; 2],
}

fn nib_signed(v: u32) -> i8 {
    let n = (v & 0xf) as u8;
    (if n & 8 != 0 { n | 0xf0 } else { n }) as i8
}

pub fn sample_pattern(count: u32, quality: u32) -> Result<SamplePattern, i32> {
    let li = if count == 0 { 0 } else { 31 - count.leading_zeros() };
    if quality.wrapping_sub(5) <= 0xffff_fff8 || li > 3 {
        return Err(E_INVALIDARG);
    }
    let q = match quality {
        0xffff_fffe => 0,
        0xffff_ffff => 1,
        q => q + 2,
    };
    let e = &SAMPLE_PATTERN[(SAMPLE_PATTERN_FIRST[li as usize] + q) as usize];
    let a = e[7];
    let r0 = (a & 7) | ((a >> 10) & 0x78) | ((a >> 13) & 0x380) | ((a >> 14) & 0xc00) | ((a & 0x10) << 8);
    let b = e[4];
    let r1 = (b & 7) | ((b >> 1) & 0x38) | ((b >> 2) & 0x1c0) | ((b >> 3) & 0xe00) | ((b >> 4) & 0x1000)
        | ((b >> 4) & 0x4000) | ((b >> 4) & 0x8000) | ((b >> 4) & 0x10000) | ((b >> 6) & 0x1c_0000) | ((b >> 6) & 0x20_0000);
    let mut p = SamplePattern { regs: [r0, r1], ..Default::default() };
    for i in 0..8 {
        p.centroid[i] = ((e[5] >> (4 * i)) & 0xf) as u8;
        p.centroid[8 + i] = ((e[6] >> (4 * i)) & 0xf) as u8;
    }
    for s in 0..16 {
        let (dw, sh) = (s >> 2, (s & 3) * 8);
        for px in 0..4 {
            let w = e[8 + 4 * px + dw];
            p.positions[px][2 * s] = nib_signed(w >> sh);
            p.positions[px][2 * s + 1] = nib_signed(w >> (sh + 4));
        }
    }
    Ok(p)
}

pub unsafe extern "system" fn get_sample_pattern_x(_this: *mut c_void, desc: *const [u32; 2], positions: *mut [[i8; 32]; 4], centroid: *mut [u8; 16], regs: *mut [u32; 2]) -> i32 {
    if desc.is_null() {
        return E_INVALIDARG;
    }
    let [count, quality] = unsafe { core::ptr::read_unaligned(desc) };
    let p = match sample_pattern(count, quality) {
        Ok(p) => p,
        Err(hr) => return hr,
    };
    unsafe {
        if !regs.is_null() { *regs = p.regs; }
        if !centroid.is_null() { *centroid = p.centroid; }
        if !positions.is_null() { *positions = p.positions; }
    }
    S_OK
}

pub const SCARLETT: bool = true;

pub fn compute_resource_limits(p: &[u32; 6], scarlett: bool) -> [u32; 5] {
    let mut o = [0u32; 5];
    o[1] = 0xffff;
    o[2] = 0xffff;
    if scarlett {
        o[3] = 0xffff;
        o[4] = 0xffff;
    }
    let mask = !p[0] | 0xffff_0000;
    for v in &mut o[1..5] {
        *v &= mask;
    }
    let n = if scarlett { 4 } else { 2 };
    for v in &mut o[1..1 + n] {
        if *v & 0xffff == 0 {
            *v = (*v & 0xffff_0001) | 1;
        }
    }
    let cap = if scarlett { 0x640 } else { 0x1e0 };
    if p[1] != 0 {
        let d = if scarlett { 64 } else { 32 };
        let v = p[1].min(cap);
        o[0] = (o[0] & !0x3ff) | ((((v + d - 1) / d) << 4) & 0x3ff);
    }
    let mut c = o[0];
    if p[3] != 0 {
        let d = if scarlett { 16 } else { 8 };
        let v = p[3].min(cap);
        c = (o[0] & !0x3f_0000) | ((((v + d - 1) / d) << 16) & 0x3f_0000);
    }
    let mut a = c;
    if p[2] != 0 {
        a = (c & !0xf000) | ((p[2].min(15) << 12) & 0xf000);
    }
    let mut r = if p[4] != 0 {
        (a & !0x700_0000) | (((p[4].min(8) - 1) << 24) & 0x700_0000)
    } else {
        (a & 0xf9ff_ffff) | (1 << 24)
    };
    r &= !(1 << 22);
    o[0] = if p[5] != 0 {
        (((p[5] - 1) & 3) << 22) | (r & !(1 << 23))
    } else {
        r & !(1 << 23)
    };
    o
}

static DERIVED_LIMITS: Mutex<Option<HashMap<usize, [u32; 5]>>> = Mutex::new(None);

pub fn derived_limits(pso: usize) -> Option<[u32; 5]> {
    DERIVED_LIMITS.lock().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(|m| m.get(&pso).copied())
}

pub fn last_patch(num: u32, patches: *const u8) -> Option<[u32; 6]> {
    if num == 0 || patches.is_null() {
        return None;
    }
    let p = unsafe { patches.add((num as usize - 1) * 0x1c + 4) };
    Some(unsafe { core::ptr::read_unaligned(p as *const [u32; 6]) })
}

pub unsafe fn derive_compute_on(real: *mut c_void, base: *mut c_void, num: u32, patches: *const u8, ppv: *mut *mut c_void) -> i32 {
    static N: AtomicUsize = AtomicUsize::new(0);
    if ppv.is_null() {
        return E_INVALIDARG;
    }
    unsafe { *ppv = core::ptr::null_mut() };
    let (Some(desc), Some(code)) = (crate::device_x::compute_desc(base as usize), crate::device_x::compute_code(base as usize)) else {
        record("d3d12_xs", "CreateDerivedComputePipelineStateX", &format!("base {base:p} is not a compute pipeline created through this layer -> E_INVALIDARG"));
        return E_INVALIDARG;
    };
    let limits = last_patch(num, patches).map(|p| compute_resource_limits(&p, SCARLETT));
    let f: unsafe extern "system" fn(*mut c_void, *const u8, *const Guid, *mut *mut c_void) -> i32 = unsafe { core::mem::transmute(vfn(real, PC_CREATE_COMPUTE_PSO)) };
    let mut out: *mut c_void = core::ptr::null_mut();
    let hr = unsafe { f(real, desc.as_ptr(), &IID_PIPELINE_STATE, &mut out) };
    if hr < 0 || out.is_null() {
        record("d3d12_xs", "CreateDerivedComputePipelineStateX", &format!("PC CreateComputePipelineState -> {hr:#010x}"));
        return if hr < 0 { hr } else { E_OUTOFMEMORY };
    }
    crate::device_x::register_compute(out, &desc, code);
    if let Some(l) = limits {
        DERIVED_LIMITS.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).insert(out as usize, l);
    }
    if once(&N, 6) {
        record("d3d12_xs", "CreateDerivedComputePipelineStateX", &format!("base={base:p} patches={num} -> {out:p}, COMPUTE_RESOURCE_LIMITS/STATIC_THREAD_MGMT {limits:x?} (scheduling registers only; the PC program is the base program)"));
    }
    unsafe { *ppv = out };
    S_OK
}

pub unsafe extern "system" fn create_derived_compute_pso_x(this: *mut c_void, base: *mut c_void, num: u32, patches: *const u8, ppv: *mut *mut c_void) -> i32 {
    unsafe { derive_compute_on(real_device(this), base, num, patches, ppv) }
}

pub fn install(v: &mut [usize]) {
    v[SLOT_CREATE_PLACED_RESOURCE_X2] = create_placed_resource_x2 as usize;
    v[SLOT_PLACED_SRV] = create_placed_srv_x as usize;
    v[SLOT_PLACED_UAV] = create_placed_uav_x as usize;
    v[SLOT_SET_DRIVER_OPTION] = set_driver_option_x as usize;
    v[SLOT_GPU_CRASH] = gpu_crash_x as usize;
    v[SLOT_KERNEL_OPTION_10001] = kernel_option_10001_x as usize;
    v[SLOT_SAMPLE_PATTERN] = get_sample_pattern_x as usize;
    v[SLOT_DERIVED_COMPUTE] = create_derived_compute_pso_x as usize;
}

#[cfg(test)]
#[path = "device_ext_a_tests.rs"]
mod tests;
