use core::ffi::c_void;
use core::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::Mutex;
use compat_common::record;

use crate::device_wrap::Guid;

pub const HLSL: &str = r#"#define RS "RootConstants(num32BitConstants=9, b0), UAV(u0)"
cbuffer C : register(b0) { uint4 c0; uint4 c1; uint slot; };
RWByteAddressBuffer s : register(u0);
uint64_t w(uint lo, uint hi) { return ((uint64_t)hi << 32) | (uint64_t)lo; }
[RootSignature(RS)]
[numthreads(1, 1, 1)]
void main() {
    uint o = slot * 8;
    uint2 r = s.Load2(o);
    uint64_t t = w(r.x, r.y);
    uint64_t g = w(c0.x, c0.y);
    uint64_t sys = w(c0.z, c0.w);
    uint64_t num = w(c1.x, c1.y);
    uint64_t den = w(c1.z, c1.w);
    bool neg = t < g;
    uint64_t d = neg ? g - t : t - g;
    uint64_t v = (d / den) * num + ((d % den) * num) / den;
    uint64_t res = neg ? sys - v : sys + v;
    s.Store2(o, uint2((uint)res, (uint)(res >> 32)));
}
"#;

pub const SLOTS: u32 = 4096;
const STATE_COMMON: u32 = 0;
const STATE_UAV: u32 = 0x8;
const STATE_COPY_DEST: u32 = 0x400;
const STATE_COPY_SOURCE: u32 = 0x800;
const STATE_GENERIC_READ: u32 = 0xac3;
const PC_DISPATCH: usize = 14;
const PC_COPY_BUFFER_REGION: usize = 15;
const PC_SET_PSO: usize = 25;
const PC_SET_COMPUTE_ROOT_SIGNATURE: usize = 29;
const PC_SET_COMPUTE_ROOT_CONSTANTS: usize = 35;
const PC_SET_COMPUTE_ROOT_UAV: usize = 41;
const PC_SET_PREDICATION: usize = 55;
const DEV_CREATE_COMPUTE_PSO: usize = 11;
const DEV_CREATE_ROOT_SIGNATURE: usize = 16;
const DEV_CREATE_COMMITTED: usize = 27;
const IID_DEVICE: Guid = Guid { d1: 0x1898_19f1, d2: 0x1db6, d3: 0x4b57, d4: [0xbe, 0x54, 0x18, 0x21, 0x33, 0x9b, 0x85, 0xf7] };
const IID_ROOT_SIGNATURE: Guid = Guid { d1: 0xc54a_6b66, d2: 0x72df, d3: 0x4ee8, d4: [0x8b, 0xe5, 0xa9, 0x46, 0xa1, 0x42, 0x92, 0x14] };
const IID_PSO: Guid = Guid { d1: 0x765a_30f3, d2: 0xf624, d3: 0x4c6f, d4: [0xa8, 0x28, 0xac, 0xe9, 0x48, 0x62, 0x24, 0x45] };
const IID_RESOURCE: Guid = Guid { d1: 0x6964_42be, d2: 0xa72e, d3: 0x4059, d4: [0xbc, 0x79, 0x5b, 0x5c, 0x98, 0x04, 0x0f, 0xad] };

extern "system" {
    fn QueryPerformanceFrequency(v: *mut i64) -> i32;
    fn QueryPerformanceCounter(v: *mut i64) -> i32;
}

#[derive(Clone, Copy)]
struct Pipeline {
    rs: usize,
    pso: usize,
    buf: usize,
    va: u64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Calibration {
    pub gpu: u64,
    pub sys: u64,
    pub sys_freq: u64,
    pub gpu_freq: u64,
}

static QUEUE: Mutex<(usize, usize, usize)> = Mutex::new((0, 0, 0));
static CAL: Mutex<Option<(Calibration, i64)>> = Mutex::new(None);
static PIPE: Mutex<Option<Option<Pipeline>>> = Mutex::new(None);
static NEXT: AtomicU32 = AtomicU32::new(0);
static LOG: AtomicUsize = AtomicUsize::new(0);

fn once(n: usize) -> bool {
    LOG.fetch_add(1, Ordering::Relaxed) < n
}

unsafe fn vslot(obj: *mut c_void, i: usize) -> usize {
    unsafe { *(*(obj as *const *const usize)).add(i) }
}

pub fn convert(t: u64, c: &Calibration) -> u64 {
    let (neg, d) = if t < c.gpu { (true, c.gpu - t) } else { (false, t - c.gpu) };
    let k = gcd(c.sys_freq, c.gpu_freq.max(1));
    let (num, den) = (c.sys_freq / k, c.gpu_freq.max(1) / k);
    let v = (d / den).wrapping_mul(num).wrapping_add(((d % den) * num) / den);
    if neg { c.sys.wrapping_sub(v) } else { c.sys.wrapping_add(v) }
}

fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a.max(1)
}

pub fn constants(c: &Calibration, slot: u32) -> [u32; 9] {
    let s = |v: u64| [v as u32, (v >> 32) as u32];
    let den = c.gpu_freq.max(1);
    let k = gcd(c.sys_freq, den);
    let [a, b] = s(c.gpu);
    let [d, e] = s(c.sys);
    let [f, g] = s(c.sys_freq / k);
    let [h, i] = s(den / k);
    [a, b, d, e, f, g, h, i, slot]
}

pub fn system_counter_hz() -> u64 {
    xgs_kmd_emu::gpu_frequency_hz(xgs_kmd_emu::device_type())
}

pub fn counter_calibration(gpu_ticks: u64, gpu_freq: u64, qpc: u64, qpf: u64, counter_hz: u64) -> Calibration {
    let sys = (qpc as u128 * counter_hz as u128 / qpf.max(1) as u128) as u64;
    Calibration { gpu: gpu_ticks, sys, sys_freq: counter_hz, gpu_freq }
}

pub fn note_queue(queue: *mut c_void, freq: usize, calib: usize) {
    let mut g = QUEUE.lock().unwrap_or_else(|e| e.into_inner());
    if g.0 == 0 && freq != 0 && calib != 0 {
        *g = (queue as usize, freq, calib);
    }
}

unsafe fn calibration() -> Option<Calibration> {
    let mut now = 0i64;
    let mut qpf = 0i64;
    unsafe {
        QueryPerformanceCounter(&mut now);
        QueryPerformanceFrequency(&mut qpf);
    }
    let mut g = CAL.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((c, _)) = *g {
        return Some(c);
    }
    let (q, freq, calib) = *QUEUE.lock().unwrap_or_else(|e| e.into_inner());
    if q == 0 {
        return None;
    }
    let f: crate::timestamp::FreqFn = unsafe { core::mem::transmute(freq) };
    let k: crate::timestamp::CalibFn = unsafe { core::mem::transmute(calib) };
    let (mut gf, mut gt, mut ct) = (0u64, 0u64, 0u64);
    if unsafe { f(q as *mut c_void, &mut gf) } < 0 || unsafe { k(q as *mut c_void, &mut gt, &mut ct) } < 0 || gf == 0 || qpf <= 0 {
        return None;
    }
    let hz = system_counter_hz();
    let c = counter_calibration(gt, gf, ct, qpf as u64, hz);
    record("d3d12_xs", "sys_ts", &format!("SYSTEM_TIMESTAMP counter: {hz} Hz console GPU core clock (device slot 68 / IOCTL 0x7900c1fc frequency), epoch = QPC zero, fixed calibration gpu={gt} @ {gf} Hz <-> qpc={ct} @ {qpf} Hz"));
    *g = Some((c, now));
    Some(c)
}

unsafe fn pipeline(list: *mut c_void) -> Option<Pipeline> {
    let mut g = PIPE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(p) = *g {
        return p;
    }
    let p = unsafe { build(list) };
    *g = Some(p);
    p
}

unsafe fn build(list: *mut c_void) -> Option<Pipeline> {
    let code = match tombox_xshader::dxc::compile(HLSL, "cs_6_0") {
        Ok(c) => c,
        Err(e) => {
            record("d3d12_xs", "sys_ts", &format!("system-timestamp conversion shader: {e}"));
            return None;
        }
    };
    let get: unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> i32 = unsafe { core::mem::transmute(vslot(list, 7)) };
    let mut dev: *mut c_void = core::ptr::null_mut();
    if unsafe { get(list, &IID_DEVICE, &mut dev) } < 0 || dev.is_null() {
        return None;
    }
    let out = unsafe { build_on(dev, &code) };
    let rel: unsafe extern "system" fn(*mut c_void) -> u32 = unsafe { core::mem::transmute(vslot(dev, 2)) };
    unsafe { rel(dev) };
    out
}

unsafe fn build_on(dev: *mut c_void, code: &[u8]) -> Option<Pipeline> {
    let crs: unsafe extern "system" fn(*mut c_void, u32, *const u8, usize, *const Guid, *mut *mut c_void) -> i32 = unsafe { core::mem::transmute(vslot(dev, DEV_CREATE_ROOT_SIGNATURE)) };
    let mut rs: *mut c_void = core::ptr::null_mut();
    let hr = unsafe { crs(dev, 0, code.as_ptr(), code.len(), &IID_ROOT_SIGNATURE, &mut rs) };
    if hr < 0 || rs.is_null() {
        record("d3d12_xs", "sys_ts", &format!("conversion root signature FAILED hr={hr:#010x}"));
        return None;
    }
    let mut desc = [0u8; 56];
    desc[0..8].copy_from_slice(&(rs as u64).to_le_bytes());
    desc[8..16].copy_from_slice(&(code.as_ptr() as u64).to_le_bytes());
    desc[16..24].copy_from_slice(&(code.len() as u64).to_le_bytes());
    let cps: unsafe extern "system" fn(*mut c_void, *const u8, *const Guid, *mut *mut c_void) -> i32 = unsafe { core::mem::transmute(vslot(dev, DEV_CREATE_COMPUTE_PSO)) };
    let mut pso: *mut c_void = core::ptr::null_mut();
    let hr = unsafe { cps(dev, desc.as_ptr(), &IID_PSO, &mut pso) };
    if hr < 0 || pso.is_null() {
        record("d3d12_xs", "sys_ts", &format!("conversion PSO FAILED hr={hr:#010x}"));
        return None;
    }
    let heap: [u32; 5] = [1, 0, 0, 1, 1];
    let mut rd = [0u8; 56];
    rd[0..4].copy_from_slice(&1u32.to_le_bytes());
    rd[0x10..0x18].copy_from_slice(&(SLOTS as u64 * 8).to_le_bytes());
    rd[0x18..0x1c].copy_from_slice(&1u32.to_le_bytes());
    rd[0x1c..0x1e].copy_from_slice(&1u16.to_le_bytes());
    rd[0x1e..0x20].copy_from_slice(&1u16.to_le_bytes());
    rd[0x24..0x28].copy_from_slice(&1u32.to_le_bytes());
    rd[0x2c..0x30].copy_from_slice(&1u32.to_le_bytes());
    rd[0x30..0x34].copy_from_slice(&4u32.to_le_bytes());
    let cc: unsafe extern "system" fn(*mut c_void, *const u32, u32, *const u8, u32, *const c_void, *const Guid, *mut *mut c_void) -> i32 = unsafe { core::mem::transmute(vslot(dev, DEV_CREATE_COMMITTED)) };
    let mut buf: *mut c_void = core::ptr::null_mut();
    let hr = unsafe { cc(dev, heap.as_ptr(), 0, rd.as_ptr(), STATE_COMMON, core::ptr::null(), &IID_RESOURCE, &mut buf) };
    if hr < 0 || buf.is_null() {
        record("d3d12_xs", "sys_ts", &format!("conversion scratch buffer FAILED hr={hr:#010x}"));
        return None;
    }
    let va: unsafe extern "system" fn(*mut c_void) -> u64 = unsafe { core::mem::transmute(vslot(buf, 11)) };
    Some(Pipeline { rs: rs as usize, pso: pso as usize, buf: buf as usize, va: unsafe { va(buf) } })
}

pub unsafe fn write(list: *mut c_void, va: u64) {
    let _ij = unsafe { crate::inject::scope(list, crate::inject::QUERY) };
    let Some((e, off)) = crate::gpu_va::lookup(va) else {
        if once(8) {
            record("d3d12_xs", "sys_ts", &format!("SYSTEM_TIMESTAMP destination {va:#x} is not in a tracked buffer"));
        }
        return;
    };
    let st = crate::gpu_va::state(e.res);
    if !e.buffer || off + 8 > e.len || st == STATE_GENERIC_READ {
        if once(8) {
            record("d3d12_xs", "sys_ts", &format!("SYSTEM_TIMESTAMP destination {va:#x} is not a GPU-writable buffer slot"));
        }
        return;
    }
    let Some(cal) = (unsafe { calibration() }) else {
        if once(8) {
            record("d3d12_xs", "sys_ts", "SYSTEM_TIMESTAMP before any queue submission: no clock calibration yet, value not written");
        }
        return;
    };
    let Some(p) = (unsafe { pipeline(list) }) else { return };
    let slot = NEXT.fetch_add(1, Ordering::Relaxed) % SLOTS;
    let soff = slot as u64 * 8;
    let k = constants(&cal, slot);
    unsafe {
        crate::fill::barrier_pub(list, p.buf, STATE_COMMON, STATE_COPY_DEST);
        if !crate::timestamp::resolve_into(list, p.buf, soff) {
            crate::fill::barrier_pub(list, p.buf, STATE_COPY_DEST, STATE_COMMON);
            return;
        }
        crate::fill::barrier_pub(list, p.buf, STATE_COPY_DEST, STATE_UAV);
        let pred: unsafe extern "system" fn(*mut c_void, *mut c_void, u64, u32) = core::mem::transmute(crate::cmdlist_wrap::pc_method(list, PC_SET_PREDICATION));
        pred(list, core::ptr::null_mut(), 0, 0);
        let srs: unsafe extern "system" fn(*mut c_void, *mut c_void) = core::mem::transmute(crate::cmdlist_wrap::pc_method(list, PC_SET_COMPUTE_ROOT_SIGNATURE));
        srs(list, p.rs as *mut c_void);
        let spso: unsafe extern "system" fn(*mut c_void, *mut c_void) = core::mem::transmute(crate::cmdlist_wrap::pc_method(list, PC_SET_PSO));
        spso(list, p.pso as *mut c_void);
        let sc: unsafe extern "system" fn(*mut c_void, u32, u32, *const u32, u32) = core::mem::transmute(crate::cmdlist_wrap::pc_method(list, PC_SET_COMPUTE_ROOT_CONSTANTS));
        sc(list, 0, 9, k.as_ptr(), 0);
        let su: unsafe extern "system" fn(*mut c_void, u32, u64) = core::mem::transmute(crate::cmdlist_wrap::pc_method(list, PC_SET_COMPUTE_ROOT_UAV));
        su(list, 1, p.va);
        let d: unsafe extern "system" fn(*mut c_void, u32, u32, u32) = core::mem::transmute(crate::cmdlist_wrap::pc_method(list, PC_DISPATCH));
        d(list, 1, 1, 1);
        crate::fill::barrier_pub(list, p.buf, STATE_UAV, STATE_COPY_SOURCE);
        if st != STATE_COPY_DEST { crate::fill::barrier_pub(list, e.res, st, STATE_COPY_DEST); }
        let cp: unsafe extern "system" fn(*mut c_void, usize, u64, usize, u64, u64) = core::mem::transmute(crate::cmdlist_wrap::pc_method(list, PC_COPY_BUFFER_REGION));
        cp(list, e.res, off, p.buf, soff, 8);
        if st != STATE_COPY_DEST { crate::fill::barrier_pub(list, e.res, STATE_COPY_DEST, st); }
        crate::fill::barrier_pub(list, p.buf, STATE_COPY_SOURCE, STATE_COMMON);
        crate::cl2_state::reapply(list);
    }
    crate::placed::note_gpu_write(e.res as *mut c_void);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gpu_ticks_convert_into_the_system_clock() {
        let c = Calibration { gpu: 1_000_000_000, sys: 50_000_000, sys_freq: 10_000_000, gpu_freq: 1_000_000_000 };
        assert_eq!(convert(1_000_000_000, &c), 50_000_000);
        assert_eq!(convert(2_000_000_000, &c), 60_000_000);
        assert_eq!(convert(999_999_900, &c), 49_999_999);
        assert_eq!(convert(500_000_000, &c), 45_000_000);
        let c2 = Calibration { gpu: 0, sys: 0, sys_freq: 10_000_000, gpu_freq: 24_000_000 };
        assert_eq!(convert(24_000_000 * 3600, &c2), 10_000_000 * 3600);
        assert_eq!(convert(u64::MAX / 2, &c2), ((u64::MAX / 2) as u128 * 10_000_000 / 24_000_000) as u64);
        assert_eq!(constants(&c2, 7), [0, 0, 0, 0, 5, 0, 12, 0, 7]);
    }

    fn shader_math(t: u64, k: &[u32; 9]) -> Option<u64> {
        let w = |lo: u32, hi: u32| (hi as u64) << 32 | lo as u64;
        let (g, sys, num, den) = (w(k[0], k[1]), w(k[2], k[3]), w(k[4], k[5]), w(k[6], k[7]));
        let (neg, d) = if t < g { (true, g - t) } else { (false, t - g) };
        let v = (d / den).checked_mul(num)?.checked_add(((d % den).checked_mul(num)?) / den)?;
        Some(if neg { sys.wrapping_sub(v) } else { sys.wrapping_add(v) })
    }

    #[test]
    fn system_counter_runs_at_the_console_gpu_clock_from_the_qpc_epoch() {
        let f = 1_825_000_000u64;
        let c = counter_calibration(5_000_000_000, 1_000_000_000, 30_000_000, 10_000_000, f);
        assert_eq!(c.sys, 3 * f);
        assert_eq!(c.sys_freq, f);
        assert_eq!(convert(5_000_000_000, &c), 3 * f);
        assert_eq!(convert(6_000_000_000, &c), 4 * f);
        assert_eq!(convert(5_000_000_001, &c) - convert(5_000_000_000, &c), 1);
        assert_eq!(convert(5_000_000_040, &c) - convert(5_000_000_000, &c), 73);
        let k = constants(&c, 3);
        assert_eq!((k[4], k[6], k[8]), (73, 40, 3));
        let s = counter_calibration(0, 1_000_000_000, 0, 10_000_000, 1_563_000_000);
        assert_eq!(convert(2_000_000_000, &s), 3_126_000_000);
        let mut last = 0;
        for t in (5_000_000_000u64..5_000_100_000).step_by(997) {
            let v = convert(t, &c);
            assert!(v >= last);
            assert_eq!(shader_math(t, &k), Some(v));
            last = v;
        }
    }

    #[test]
    fn shader_math_does_not_overflow_for_real_host_timestamp_rates() {
        let f = 1_825_000_000u64;
        for gf in [10_000_000u64, 24_000_000, 25_000_000, 99_750_000, 100_000_000, 1_000_000_000, 1_000_000_007, 2_400_000_000] {
            let c = counter_calibration(1 << 40, gf, 1 << 33, 10_000_000, f);
            let k = constants(&c, 0);
            for t in [1u64 << 40, (1 << 40) + gf - 1, (1 << 40) + 3600 * gf, (1 << 40) + 30 * 86_400 * gf] {
                let want = c.sys as i128 + ((t as i128 - c.gpu as i128) * f as i128).div_euclid(gf as i128);
                let got = shader_math(t, &k).unwrap_or_else(|| panic!("overflow gf={gf} t={t}"));
                assert!((got as i128 - want).abs() <= 1, "gf={gf} t={t} got={got} want={want}");
            }
        }
    }

    #[test]
    fn system_counter_frequency_follows_the_emulated_console() {
        let per_type: Vec<u64> = (2..=8).map(xgs_kmd_emu::gpu_frequency_hz).collect();
        assert_eq!(per_type, vec![853_333_333, 914_285_714, 1_172_000_000, 1_172_000_000, 1_563_000_000, 1_825_000_000, 1_825_000_000]);
        assert_eq!(system_counter_hz(), xgs_kmd_emu::gpu_frequency_hz(xgs_kmd_emu::device_type()));
        assert_eq!(system_counter_hz(), crate::console_slots_dev2::kmd_info(xgs_kmd_emu::device_type()).word0);
    }

    #[test]
    fn conversion_shader_compiles_when_dxc_is_present() {
        if !tombox_xshader::dxc::available() {
            return;
        }
        let code = tombox_xshader::dxc::compile(HLSL, "cs_6_0").unwrap();
        assert_eq!(&code[0..4], b"DXBC");
        assert!(code.windows(4).any(|w| w == b"RTS0"));
    }
}
