use core::ffi::c_void;
use compat_common::record;

use crate::device_wrap::Guid;

pub const GFX_DESC_SIZE: usize = 0x290;
pub const EXT_STRIDE: usize = 0x40;
pub const EXT_DEPTH_STENCIL: u32 = 1;
const DSS: usize = 0x1f0;
const FEATURE_OPTIONS2: u32 = 18;
const FEATURE_OPTIONS14: u32 = 43;
const DEV_CHECK_FEATURE: usize = 13;
const DEV2_CREATE_PIPELINE_STATE: usize = 47;
const IID_DEVICE2: Guid = Guid { d1: 0x30ba_a41e, d2: 0xb15b, d3: 0x475c, d4: [0xa0, 0xbb, 0x1a, 0xf5, 0xc5, 0xb6, 0x43, 0x28] };

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DsExt {
    pub backface_disable: u32,
    pub depth_bounds_disable: u32,
    pub color_on_depth_fail: u32,
    pub no_color_on_depth_pass: u32,
    pub back_read: u8,
    pub back_write: u8,
    pub front_ref: u8,
    pub back_ref: u8,
    pub front_op: u8,
    pub back_op: u8,
}

fn u32at(d: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(d[o..o + 4].try_into().unwrap())
}

pub fn default_ext(desc: &[u8]) -> DsExt {
    DsExt {
        backface_disable: 0,
        depth_bounds_disable: 1,
        color_on_depth_fail: 0,
        no_color_on_depth_pass: 0,
        back_read: desc[DSS + 0x10],
        back_write: desc[DSS + 0x11],
        front_ref: 0,
        back_ref: 0,
        front_op: 1,
        back_op: 1,
    }
}

pub fn parse_ext(desc: &[u8], entries: &[u8], count: usize) -> (DsExt, Vec<u32>) {
    let mut ext = default_ext(desc);
    let mut other = Vec::new();
    for i in 0..count {
        let e = &entries[i * EXT_STRIDE..(i + 1) * EXT_STRIDE];
        let ty = u32at(e, 0);
        if ty == EXT_DEPTH_STENCIL {
            let p = &e[4..];
            ext = DsExt {
                backface_disable: u32at(p, 0),
                depth_bounds_disable: u32at(p, 4),
                color_on_depth_fail: u32at(p, 8),
                no_color_on_depth_pass: u32at(p, 0xc),
                back_read: p[0x10],
                back_write: p[0x11],
                front_ref: p[0x12],
                back_ref: p[0x13],
                front_op: p[0x14],
                back_op: p[0x15],
            };
        } else {
            other.push(ty);
        }
    }
    (ext, other)
}

pub const EXT_PS_OPTIONS: u32 = 0;
pub const EXT_SHADER_LIMITS: u32 = 2;
pub const EXT_TESSELLATION: u32 = 3;
pub const COMPUTE_EXT_STRIDE: usize = 0x1c;
pub const SHADER_LIMITS_DEFAULT_LATE_ALLOC: u32 = 0x16;

const PS_OPTIONS: [(&str, &str); 8] = [
    ("PerSampleIteration", "PA_SC_MODE_CNTL_1.PS_ITER_SAMPLE; 1 = PS variant with sample-interpolated inputs (sample-rate), 2 = PC default"),
    ("ZOrder", "DB_SHADER_CONTROL.Z_ORDER; 2/4 = PS variant with [earlydepthstencil] unless the PS exports depth or can discard with depth/stencil writes on, 1/3 = PC late-Z default"),
    ("TexKill", "DB_SHADER_CONTROL.KILL_ENABLE; 1 = PC default, 2 = no PC equivalent when the PS can discard (killed lanes export undefined data)"),
    ("ExecOnHierFail", "DB_SHADER_CONTROL.EXEC_ON_HIER_FAIL; no PC equivalent"),
    ("ExecOnNoop", "DB_SHADER_CONTROL.EXEC_ON_NOOP; no PC equivalent"),
    ("DepthBeforeShader", "DB_SHADER_CONTROL.DEPTH_BEFORE_SHADER; no PC equivalent"),
    ("ConservativeZExport", "DB_SHADER_CONTROL.CONSERVATIVE_Z_EXPORT; 2/3 = PS variant with SV_DepthLessEqual/SV_DepthGreaterEqual when the PS exports depth"),
    ("OutOfOrderRasterization", "PA_SC_MODE_CNTL_1.OUT_OF_ORDER_PRIMITIVE_ENABLE+WATER_MARK; no PC equivalent"),
];

const SHADER_LIMITS: [(&str, &str); 15] = [
    ("MaxWavesWithLateAllocParameterCache", "GE_PC_ALLOC"),
    ("VSDisableFlags", "SPI_SHADER_PGM_RSRC3_VS.CU_EN"),
    ("field+0x8", "not read by 0xc4a60"),
    ("GSDisableFlags", "SPI_SHADER_PGM_RSRC3_GS.CU_EN"),
    ("PSDisableFlags", "SPI_SHADER_PGM_RSRC3_PS.CU_EN"),
    ("VSMaxWaves", "SPI_SHADER_PGM_RSRC3_VS.WAVE_LIMIT"),
    ("HSMaxWaves", "SPI_SHADER_PGM_RSRC3_HS.WAVE_LIMIT"),
    ("field+0x1c", "not read by 0xc4a60"),
    ("GSMaxWaves", "SPI_SHADER_PGM_RSRC3_GS.WAVE_LIMIT"),
    ("PSMaxWaves", "SPI_SHADER_PGM_RSRC3_PS.WAVE_LIMIT"),
    ("VSMaxWavesForCuLocking", "SPI_SHADER_PGM_RSRC3_VS.LOCK_LOW_THRESHOLD"),
    ("HSMaxWavesForCuLocking", "SPI_SHADER_PGM_RSRC3_HS.LOCK_LOW_THRESHOLD"),
    ("field+0x30", "not read by 0xc4a60"),
    ("GSMaxWavesForCuLocking", "SPI_SHADER_PGM_RSRC3_GS.LOCK_LOW_THRESHOLD"),
    ("PSMaxWavesForCuLocking", "SPI_SHADER_PGM_RSRC3_PS.LOCK_LOW_THRESHOLD"),
];

const TESSELLATION: [(&str, &str); 3] = [
    ("PatchesPerThreadgroup", "HS threadgroup patch count"),
    ("field+0x4", "float, no non-stack reader found on the graphics path"),
    ("DsWaveThreshold", "DS wave threshold, no non-stack reader found on the graphics path"),
];

const COMPUTE_LIMITS: [(&str, &str); 6] = [
    ("DisableFlags", "COMPUTE_STATIC_THREAD_MGMT_SE*.SH*_CU_EN"),
    ("MaxWaves", "COMPUTE_RESOURCE_LIMITS.WAVES_PER_SH"),
    ("MaxThreadgroupsPerCu", "field named by the debug validator; register not traced"),
    ("MaxWavesForCuLocking", "field named by the debug validator; register not traced"),
    ("NumThreadgroupsWalkedPerCu", "field named by the debug validator; register not traced"),
    ("SimdWalkAlgorithm", "field named by the debug validator; register not traced"),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Block {
    PsOptions,
    ShaderLimits,
    Tessellation,
    ComputeLimits,
}

impl Block {
    pub fn of_ext(ty: u32) -> Option<Block> {
        match ty {
            EXT_PS_OPTIONS => Some(Block::PsOptions),
            EXT_SHADER_LIMITS => Some(Block::ShaderLimits),
            EXT_TESSELLATION => Some(Block::Tessellation),
            _ => None,
        }
    }

    pub fn of_derived_tag(tag: u32) -> Option<Block> {
        match tag {
            4 => Some(Block::PsOptions),
            6 => Some(Block::ShaderLimits),
            7 => Some(Block::Tessellation),
            _ => None,
        }
    }

    pub fn size(self) -> usize {
        self.fields().len() * 4
    }

    fn fields(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Block::PsOptions => &PS_OPTIONS,
            Block::ShaderLimits => &SHADER_LIMITS,
            Block::Tessellation => &TESSELLATION,
            Block::ComputeLimits => &COMPUTE_LIMITS,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Block::PsOptions => "D3D12XBOX_PIXEL_SHADER_OPTIONS_DESC",
            Block::ShaderLimits => "D3D12XBOX_GRAPHICS_SHADER_LIMITS_DESC",
            Block::Tessellation => "D3D12XBOX_PIPELINE_TESSELLATION_PARAMETERS_DESC",
            Block::ComputeLimits => "D3D12XBOX_COMPUTE_SHADER_LIMITS_DESC",
        }
    }

    pub fn changes_output(self) -> bool {
        self == Block::PsOptions
    }

    pub fn non_default(self, payload: &[u8]) -> Vec<String> {
        let mut v = Vec::new();
        for (i, (name, reg)) in self.fields().iter().enumerate() {
            if payload.len() < i * 4 + 4 {
                break;
            }
            let x = u32at(payload, i * 4);
            let dflt = if self == Block::ShaderLimits && i == 0 { SHADER_LIMITS_DEFAULT_LATE_ALLOC } else { 0 };
            if x != dflt {
                v.push(format!("{name}={x:#x} ({reg})"));
            }
        }
        v
    }

    pub fn describe(self, payload: &[u8]) -> Option<String> {
        let f = self.non_default(payload);
        if f.is_empty() {
            return None;
        }
        let why = if self.changes_output() {
            "pixel-shader execution controls; per-field PC handling above"
        } else {
            "CU/wave scheduling limits with no D3D12 PC API; rendered results are unaffected"
        };
        Some(format!("{}: {} - {why}", self.name(), f.join(", ")))
    }
}

pub fn describe_entries(entries: &[u8], count: usize) -> Vec<String> {
    let mut v = Vec::new();
    for e in entries.chunks_exact(EXT_STRIDE).take(count) {
        let ty = u32at(e, 0);
        match Block::of_ext(ty) {
            Some(b) => v.extend(b.describe(&e[4..4 + b.size()])),
            None if ty == EXT_DEPTH_STENCIL => {}
            None => v.push(format!("ext type {ty} is not one the console driver accepts (0x45410 handles 0-3)")),
        }
    }
    v
}

pub fn describe_compute(entries: &[u8], count: usize) -> Vec<String> {
    let mut v = Vec::new();
    for e in entries.chunks_exact(COMPUTE_EXT_STRIDE).take(count) {
        let ty = u32at(e, 0);
        if ty == 0 {
            v.extend(Block::ComputeLimits.describe(&e[4..4 + Block::ComputeLimits.size()]));
        } else {
            v.push(format!("compute ext type {ty} is skipped by the console driver (0x4a520 copies type 0 only)"));
        }
    }
    v
}

fn record_once(api: &str, msg: String) {
    static SEEN: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
    let mut g = SEEN.lock().unwrap_or_else(|p| p.into_inner());
    if g.len() < 64 && !g.contains(&msg) {
        g.push(msg.clone());
        drop(g);
        record("d3d12_xs", api, &msg);
    }
}

pub unsafe fn note_compute(num_ext: u32, pext: *const u8) {
    if pext.is_null() || num_ext == 0 {
        return;
    }
    let entries = unsafe { core::slice::from_raw_parts(pext, num_ext as usize * COMPUTE_EXT_STRIDE) };
    for m in describe_compute(entries, num_ext as usize) {
        record_once("CreateComputePSO_X", m);
    }
}

pub fn note_derived(tag: u32, payload: &[u8]) -> bool {
    let Some(b) = Block::of_derived_tag(tag) else { return false };
    if let Some(m) = b.describe(&payload[..b.size()]) {
        record_once("CreateDerivedPSO_X", format!("tag {tag} {m}"));
    }
    true
}

pub fn unmodelled(desc: &[u8], ext: &DsExt) -> Vec<&'static str> {
    let mut v = Vec::new();
    let stencil = u32at(desc, DSS + 0xc) != 0;
    if ext.color_on_depth_fail != 0 {
        v.push("ENABLE_COLOR_WRITES_ON_DEPTH_FAIL");
    }
    if ext.no_color_on_depth_pass != 0 {
        v.push("DISABLE_COLOR_WRITES_ON_DEPTH_PASS");
    }
    if stencil && (ext.front_op != 1 || ext.back_op != 1) {
        v.push("STENCILOPVAL other than 1");
    }
    v
}

pub struct Plan {
    pub depth_bounds: bool,
    pub per_face: bool,
}

pub fn plan(desc: &[u8], ext: &DsExt, bounds_ok: bool, per_face_ok: bool) -> Plan {
    let front = (desc[DSS + 0x10], desc[DSS + 0x11]);
    let differs = ext.backface_disable == 0 && u32at(desc, DSS + 0xc) != 0 && (ext.back_read, ext.back_write) != front;
    Plan { depth_bounds: ext.depth_bounds_disable == 0 && bounds_ok, per_face: differs && per_face_ok }
}

fn push(s: &mut Vec<u8>, ty: u32, inner: &[u8], align8: bool) {
    let start = s.len();
    s.extend_from_slice(&ty.to_le_bytes());
    if align8 {
        s.extend_from_slice(&[0u8; 4]);
    }
    s.extend_from_slice(inner);
    while (s.len() - start) % 8 != 0 {
        s.push(0);
    }
}

pub fn depth_stencil1(desc: &[u8], bounds: bool) -> Vec<u8> {
    let mut v = desc[DSS..DSS + 0x34].to_vec();
    v.extend_from_slice(&(bounds as u32).to_le_bytes());
    v
}

pub fn depth_stencil2(desc: &[u8], ext: &DsExt, bounds: bool) -> Vec<u8> {
    let d = &desc[DSS..DSS + 0x34];
    let mut v = d[0..0x10].to_vec();
    let face = |v: &mut Vec<u8>, ops: &[u8], read: u8, write: u8| {
        v.extend_from_slice(ops);
        v.extend_from_slice(&[read, write, 0, 0]);
    };
    face(&mut v, &d[0x14..0x24], d[0x10], d[0x11]);
    let back_ops = if ext.backface_disable != 0 { &d[0x14..0x24] } else { &d[0x24..0x34] };
    let (br, bw) = if ext.backface_disable != 0 { (d[0x10], d[0x11]) } else { (ext.back_read, ext.back_write) };
    face(&mut v, back_ops, br, bw);
    v.extend_from_slice(&(bounds as u32).to_le_bytes());
    v
}

pub fn stream(desc: &[u8], ext: &DsExt, p: &Plan) -> Vec<u8> {
    let mut s = Vec::new();
    push(&mut s, 0, &desc[0..8], true);
    for (k, ty) in [1u32, 2, 3, 4, 5].iter().enumerate() {
        push(&mut s, *ty, &desc[0x08 + 0x10 * k..0x18 + 0x10 * k], true);
    }
    push(&mut s, 7, &desc[0x58..0x78], true);
    push(&mut s, 8, &desc[0x78..0x1c0], false);
    push(&mut s, 9, &desc[0x1c0..0x1c4], false);
    push(&mut s, 10, &desc[0x1c4..0x1f0], false);
    if p.per_face {
        push(&mut s, 26, &depth_stencil2(desc, ext, p.depth_bounds), false);
    } else {
        push(&mut s, 21, &depth_stencil1(desc, p.depth_bounds), false);
    }
    push(&mut s, 12, &desc[0x228..0x238], true);
    push(&mut s, 13, &desc[0x238..0x23c], false);
    push(&mut s, 14, &desc[0x23c..0x240], false);
    let mut rt = desc[0x244..0x264].to_vec();
    rt.extend_from_slice(&desc[0x240..0x244]);
    push(&mut s, 15, &rt, false);
    push(&mut s, 16, &desc[0x264..0x268], false);
    push(&mut s, 17, &desc[0x268..0x270], false);
    push(&mut s, 18, &desc[0x270..0x274], false);
    push(&mut s, 19, &desc[0x278..0x288], true);
    push(&mut s, 20, &desc[0x288..0x28c], false);
    s
}

unsafe fn slot(o: *mut c_void, i: usize) -> usize {
    unsafe { *(*(o as *const *const usize)).add(i) }
}

unsafe fn feature_u32(real: *mut c_void, feature: u32, words: usize, idx: usize) -> bool {
    let f: unsafe extern "system" fn(*mut c_void, u32, *mut u32, u32) -> i32 = unsafe { core::mem::transmute(slot(real, DEV_CHECK_FEATURE)) };
    let mut buf = [0u32; 8];
    let hr = unsafe { f(real, feature, buf.as_mut_ptr(), (words * 4) as u32) };
    hr >= 0 && buf[idx] != 0
}

pub unsafe fn caps(real: *mut c_void) -> (bool, bool) {
    static CAPS: std::sync::Mutex<Vec<(usize, (bool, bool))>> = std::sync::Mutex::new(Vec::new());
    if let Some(c) = CAPS.lock().unwrap_or_else(|p| p.into_inner()).iter().find(|e| e.0 == real as usize) {
        return c.1;
    }
    let c = unsafe { (feature_u32(real, FEATURE_OPTIONS2, 2, 0), feature_u32(real, FEATURE_OPTIONS14, 3, 2)) };
    let mut g = CAPS.lock().unwrap_or_else(|p| p.into_inner());
    g.retain(|e| e.0 != real as usize);
    g.push((real as usize, c));
    if g.len() > 8 {
        g.remove(0);
    }
    c
}

pub unsafe fn create(real: *mut c_void, desc: &[u8], num_ext: u32, pext: *const u8, riid: *const Guid, ppv: *mut *mut c_void) -> Option<i32> {
    if pext.is_null() || num_ext == 0 || desc.len() < GFX_DESC_SIZE {
        return None;
    }
    let entries = unsafe { core::slice::from_raw_parts(pext, num_ext as usize * EXT_STRIDE) };
    let (ext, _) = parse_ext(desc, entries, num_ext as usize);
    for m in describe_entries(entries, num_ext as usize) {
        record_once("CreateGraphicsPSO_X", m);
    }
    let (bounds_ok, per_face_ok) = unsafe { caps(real) };
    let p = plan(desc, &ext, bounds_ok, per_face_ok);
    let gaps = unmodelled(desc, &ext);
    static N: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);
    let note = N.fetch_add(1, core::sync::atomic::Ordering::Relaxed) < 16;
    if note && (!gaps.is_empty() || (ext.depth_bounds_disable == 0 && !bounds_ok)) {
        record("d3d12_xs", "CreateGraphicsPSO_X", &format!("ext {ext:?}: no PC equivalent for {gaps:?}; depth bounds supported={bounds_ok} per-face stencil masks supported={per_face_ok}"));
    }
    if !p.depth_bounds && !p.per_face {
        return None;
    }
    let qi: unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> i32 = unsafe { core::mem::transmute(slot(real, 0)) };
    let mut dev2: *mut c_void = core::ptr::null_mut();
    if unsafe { qi(real, &IID_DEVICE2, &mut dev2) } < 0 || dev2.is_null() {
        return None;
    }
    let s = stream(desc, &ext, &p);
    let sd: [usize; 2] = [s.len(), s.as_ptr() as usize];
    let f: unsafe extern "system" fn(*mut c_void, *const [usize; 2], *const Guid, *mut *mut c_void) -> i32 = unsafe { core::mem::transmute(slot(dev2, DEV2_CREATE_PIPELINE_STATE)) };
    let extra = format!("{ext:?} depth_bounds={} per_face={}", p.depth_bounds, p.per_face);
    let hr = unsafe { crate::pso_lib::stream(real, desc, &extra, &sd, riid, ppv, &|| f(dev2, &sd, riid, ppv)) };
    let rel: unsafe extern "system" fn(*mut c_void) -> u32 = unsafe { core::mem::transmute(slot(dev2, 2)) };
    unsafe { rel(dev2) };
    if note || hr < 0 {
        record("d3d12_xs", "CreateGraphicsPSO_X", &format!("pipeline stream with depthBounds={} perFaceMasks={} back=({:#x},{:#x}) -> hr={hr:#010x}{}", p.depth_bounds, p.per_face, ext.back_read, ext.back_write, if hr < 0 { "; creating without the extension state" } else { "" }));
    }
    if hr < 0 { None } else { Some(hr) }
}

static EXT_OF: std::sync::Mutex<Option<std::collections::HashMap<usize, [u8; EXT_STRIDE]>>> = std::sync::Mutex::new(None);

pub fn entry_from_payload(payload: &[u8]) -> [u8; EXT_STRIDE] {
    let mut e = [0u8; EXT_STRIDE];
    e[0..4].copy_from_slice(&EXT_DEPTH_STENCIL.to_le_bytes());
    e[4..4 + 0x18].copy_from_slice(&payload[..0x18]);
    e
}

pub const DERIVED_PATCH_STRIDE: usize = 0x14c;

pub fn apply_derived_patch(tag: u32, payload: &[u8], desc: &mut [u8], ds_ext: &mut Option<[u8; EXT_STRIDE]>) -> bool {
    match tag {
        0 => {
            desc[DSS..DSS + 0x34].copy_from_slice(&payload[..0x34]);
            if let Some(e) = ds_ext.as_mut() {
                e[4 + 0x10] = payload[0x10];
                e[4 + 0x11] = payload[0x11];
            }
        }
        1 => desc[0x1c4..0x1c4 + 0x2c].copy_from_slice(&payload[..0x2c]),
        2 => desc[0x78..0x78 + 0x148].copy_from_slice(&payload[..0x148]),
        3 => desc[0x1c0..0x1c4].copy_from_slice(&payload[..4]),
        5 => *ds_ext = Some(entry_from_payload(payload)),
        _ => return false,
    }
    true
}

pub unsafe fn note(pso: *mut c_void, num_ext: u32, pext: *const u8) {
    if pso.is_null() {
        return;
    }
    let entries: &[u8] = if pext.is_null() || num_ext == 0 { &[] } else { unsafe { core::slice::from_raw_parts(pext, num_ext as usize * EXT_STRIDE) } };
    let last = entries.chunks_exact(EXT_STRIDE).rev().find(|e| u32at(e, 0) == EXT_DEPTH_STENCIL);
    let mut g = EXT_OF.lock().unwrap_or_else(|p| p.into_inner());
    let m = g.get_or_insert_with(Default::default);
    match last {
        Some(e) => {
            m.insert(pso as usize, e.try_into().unwrap());
        }
        None => {
            m.remove(&(pso as usize));
        }
    }
}

pub fn entry_of(pso: *mut c_void) -> Option<[u8; EXT_STRIDE]> {
    EXT_OF.lock().unwrap_or_else(|p| p.into_inner()).as_ref().and_then(|m| m.get(&(pso as usize)).copied())
}

#[path = "pso_ext_ps.rs"]
pub mod ps;

#[cfg(test)]
#[path = "pso_ext_tests.rs"]
mod tests;
