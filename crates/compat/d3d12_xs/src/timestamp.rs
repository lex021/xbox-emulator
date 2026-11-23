use core::ffi::c_void;
use core::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::Mutex;
use compat_common::record;

use crate::device_wrap::Guid;

const IID_DEVICE: Guid =
    Guid { d1: 0x1898_19F1, d2: 0x1DB6, d3: 0x4B57, d4: [0xBE, 0x54, 0x18, 0x21, 0x33, 0x9B, 0x85, 0xF7] };
const IID_QUERY_HEAP: Guid =
    Guid { d1: 0x0d96_58ae, d2: 0xed45, d3: 0x469e, d4: [0xa6, 0x1d, 0x97, 0x0e, 0xc5, 0x83, 0xca, 0xb4] };
const HEAP_TYPE_TIMESTAMP: u32 = 1;
const HEAP_TYPE_COPY_QUEUE_TIMESTAMP: u32 = 5;
const QUERY_TYPE_TIMESTAMP: u32 = 2;
const FEATURE_OPTIONS3: u32 = 21;
pub const HEAP_SLOTS: u32 = 1 << 16;

struct Heaps {
    direct: usize,
    copy: usize,
    copy_supported: bool,
}

static HEAPS: Mutex<Option<Heaps>> = Mutex::new(None);
static NEXT_DIRECT: AtomicU32 = AtomicU32::new(0);
static NEXT_COPY: AtomicU32 = AtomicU32::new(0);

unsafe fn vt(obj: *mut c_void) -> *const usize {
    unsafe { *(obj as *const *const usize) }
}

pub fn next_index(counter: &AtomicU32) -> u32 {
    counter.fetch_add(1, Ordering::Relaxed) % HEAP_SLOTS
}

unsafe fn create_heap(dev: *mut c_void, ty: u32) -> usize {
    let desc: [u32; 3] = [ty, HEAP_SLOTS, 0];
    let f: unsafe extern "system" fn(*mut c_void, *const u32, *const Guid, *mut *mut c_void) -> i32 =
        unsafe { core::mem::transmute(*vt(dev).add(39)) };
    let mut h: *mut c_void = core::ptr::null_mut();
    let hr = unsafe { f(dev, desc.as_ptr(), &IID_QUERY_HEAP, &mut h) };
    if hr < 0 {
        record("d3d12_xs", "timestamp", &format!("CreateQueryHeap type={ty} FAILED hr={hr:#010x}"));
        return 0;
    }
    h as usize
}

unsafe fn heaps(list: *mut c_void) -> Option<(usize, usize, bool)> {
    let mut g = HEAPS.lock().unwrap_or_else(|e| e.into_inner());
    if g.is_none() {
        let get: unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> i32 =
            unsafe { core::mem::transmute(*vt(list).add(7)) };
        let mut dev: *mut c_void = core::ptr::null_mut();
        if unsafe { get(list, &IID_DEVICE, &mut dev) } < 0 || dev.is_null() {
            return None;
        }
        let check: unsafe extern "system" fn(*mut c_void, u32, *mut c_void, u32) -> i32 =
            unsafe { core::mem::transmute(*vt(dev).add(13)) };
        let mut opt3 = [0u32; 5];
        let copy_supported = unsafe { check(dev, FEATURE_OPTIONS3, opt3.as_mut_ptr() as *mut c_void, 20) } >= 0 && opt3[0] != 0;
        let direct = unsafe { create_heap(dev, HEAP_TYPE_TIMESTAMP) };
        let copy = if copy_supported { unsafe { create_heap(dev, HEAP_TYPE_COPY_QUEUE_TIMESTAMP) } } else { 0 };
        let release: unsafe extern "system" fn(*mut c_void) -> u32 = unsafe { core::mem::transmute(*vt(dev).add(2)) };
        unsafe { release(dev) };
        record("d3d12_xs", "timestamp", &format!("query heaps direct={direct:#x} copy={copy:#x} copyQueueTimestamps={copy_supported}"));
        *g = Some(Heaps { direct, copy, copy_supported });
    }
    g.as_ref().map(|h| (h.direct, h.copy, h.copy_supported))
}

pub unsafe fn resolve_into(list: *mut c_void, res: usize, off: u64) -> bool {
    let Some((direct, _, _)) = (unsafe { heaps(list) }) else { return false };
    if direct == 0 {
        return false;
    }
    let idx = next_index(&NEXT_DIRECT);
    let end: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, u32) = unsafe { core::mem::transmute(*vt(list).add(53)) };
    let resolve: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, u32, u32, *mut c_void, u64) = unsafe { core::mem::transmute(*vt(list).add(54)) };
    unsafe {
        end(list, direct as *mut c_void, QUERY_TYPE_TIMESTAMP, idx);
        resolve(list, direct as *mut c_void, QUERY_TYPE_TIMESTAMP, idx, 1, res as *mut c_void, off);
    }
    true
}

pub unsafe fn write(list: *mut c_void, copy_list: bool, addr: u64, a: u32, b: u32, c: u32) {
    let _ij = unsafe { crate::inject::scope(list, crate::inject::QUERY) };
    static MISS: AtomicUsize = AtomicUsize::new(0);
    static LOG: AtomicUsize = AtomicUsize::new(0);
    let Some((e, off)) = crate::gpu_va::lookup(addr) else {
        if MISS.fetch_add(1, Ordering::Relaxed) < 8 {
            record("d3d12_xs", "timestamp", &format!("addr={addr:#x} not owned by any tracked buffer"));
        }
        return;
    };
    if !e.buffer || off % 8 != 0 || off + 8 > e.len {
        if MISS.fetch_add(1, Ordering::Relaxed) < 8 {
            record("d3d12_xs", "timestamp", &format!("addr={addr:#x} buffer={} off={off:#x} len={:#x}: not an 8-byte-aligned buffer slot", e.buffer, e.len));
        }
        return;
    }
    let Some((direct, copy, copy_supported)) = (unsafe { heaps(list) }) else { return };
    let (heap, idx) = if copy_list {
        if !copy_supported || copy == 0 {
            if MISS.fetch_add(1, Ordering::Relaxed) < 8 {
                record("d3d12_xs", "timestamp", "host GPU has no copy-queue timestamps; DMA timestamp not written");
            }
            return;
        }
        (copy, next_index(&NEXT_COPY))
    } else {
        if direct == 0 {
            return;
        }
        (direct, next_index(&NEXT_DIRECT))
    };
    let end: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, u32) =
        unsafe { core::mem::transmute(*vt(list).add(53)) };
    let tracked = crate::gpu_va::state(e.res);
    let (before, during) = crate::fill::copy_states(tracked, copy_list);
    let need = before != during && tracked != 0x400;
    let ty = unsafe { crate::fill::list_type(list) };
    if need && !crate::fill::state_legal_on(ty, before) {
        if MISS.fetch_add(1, Ordering::Relaxed) < 8 {
            record("d3d12_xs", "timestamp", &format!("addr={addr:#x}: buffer in state {before:#x}, not transitionable on a type-{ty} list; timestamp not written"));
        }
        return;
    }
    if copy_list != (ty == crate::fill::LIST_TYPE_COPY) {
        if MISS.fetch_add(1, Ordering::Relaxed) < 8 {
            record("d3d12_xs", "timestamp", &format!("addr={addr:#x}: list type {ty} does not match its interface (copyList={copy_list}); timestamp not written"));
        }
        return;
    }
    unsafe { end(list, heap as *mut c_void, QUERY_TYPE_TIMESTAMP, idx) };
    let ok = if copy_list {
        unsafe { crate::ts99::defer_dma(list, heap as *mut c_void, idx, e.res, off) }
    } else {
        unsafe { crate::ts99::emit(list, heap as *mut c_void, idx, 1, e.res, off, need.then_some(before)) }
    };
    if !ok {
        if MISS.fetch_add(1, Ordering::Relaxed) < 8 {
            record("d3d12_xs", "timestamp", &format!("addr={addr:#x}: GPU timestamp could not be converted to the console 99.75 MHz time base; not written"));
        }
        return;
    }
    if LOG.fetch_add(1, Ordering::Relaxed) < 4 {
        record("d3d12_xs", "timestamp", &format!("addr={addr:#x} -> res={:#x}+{off:#x} slot={idx} copyList={copy_list} args=({a},{b},{c})", e.res));
    }
}

extern "system" {
    fn QueryPerformanceFrequency(v: *mut i64) -> i32;
}

pub type FreqFn = unsafe extern "system" fn(*mut c_void, *mut u64) -> i32;
pub type CalibFn = unsafe extern "system" fn(*mut c_void, *mut u64, *mut u64) -> i32;

struct Clock {
    freq: u64,
    first: Option<(u64, u64)>,
    checks: u32,
}

static CLOCK: Mutex<Clock> = Mutex::new(Clock { freq: 0, first: None, checks: 0 });

pub fn freq() -> u64 {
    CLOCK.lock().unwrap_or_else(|e| e.into_inner()).freq
}

pub fn measured_rate(a: (u64, u64), b: (u64, u64), qpf: u64) -> Option<f64> {
    let dg = b.0.checked_sub(a.0)? as f64;
    let dc = b.1.checked_sub(a.1)? as f64;
    if dc <= 0.0 || qpf == 0 {
        return None;
    }
    Some(dg * qpf as f64 / dc)
}

pub unsafe fn on_frequency(queue: *mut c_void, out: *mut u64, hr: i32) {
    if hr < 0 || out.is_null() {
        record("d3d12_xs", "timestamp.clock", &format!("GetTimestampFrequency queue={queue:p} FAILED hr={hr:#010x}"));
        return;
    }
    let f = unsafe { *out };
    let mut g = CLOCK.lock().unwrap_or_else(|e| e.into_inner());
    if g.freq != f {
        g.freq = f;
        let mut qpf = 0i64;
        unsafe { QueryPerformanceFrequency(&mut qpf) };
        record("d3d12_xs", "timestamp.clock", &format!("GetTimestampFrequency queue={queue:p} -> {f} Hz (host GPU clock; title converts its timestamp-query ticks with this) QPF={qpf}"));
    }
}

pub unsafe fn on_calibration(queue: *mut c_void, gpu: *mut u64, cpu: *mut u64, hr: i32) {
    static N: AtomicUsize = AtomicUsize::new(0);
    if N.fetch_add(1, Ordering::Relaxed) < 4 {
        let (g, c) = if hr >= 0 && !gpu.is_null() && !cpu.is_null() { unsafe { (*gpu, *cpu) } } else { (0, 0) };
        record("d3d12_xs", "timestamp.clock", &format!("GetClockCalibration queue={queue:p} hr={hr:#010x} gpu={g} cpu={c}"));
    }
}

pub unsafe fn check_clock(queue: *mut c_void, freq_fn: usize, calib_fn: usize) {
    if freq_fn == 0 || calib_fn == 0 {
        return;
    }
    let calib: CalibFn = unsafe { core::mem::transmute(calib_fn) };
    let (mut gt, mut ct) = (0u64, 0u64);
    if unsafe { calib(queue, &mut gt, &mut ct) } < 0 {
        return;
    }
    let mut g = CLOCK.lock().unwrap_or_else(|e| e.into_inner());
    if g.freq == 0 {
        let f: FreqFn = unsafe { core::mem::transmute(freq_fn) };
        let mut v = 0u64;
        if unsafe { f(queue, &mut v) } >= 0 {
            g.freq = v;
        }
    }
    let Some(first) = g.first else {
        g.first = Some((gt, ct));
        return;
    };
    let mut qpf = 0i64;
    unsafe { QueryPerformanceFrequency(&mut qpf) };
    let Some(rate) = measured_rate(first, (gt, ct), qpf as u64) else { return };
    let secs = (ct.saturating_sub(first.1)) as f64 / qpf.max(1) as f64;
    if secs < 1.0 {
        return;
    }
    g.checks += 1;
    let err = if g.freq != 0 { (rate - g.freq as f64) / g.freq as f64 } else { f64::NAN };
    if g.checks <= 3 || err.abs() > 0.001 || g.checks % 20 == 0 {
        record("d3d12_xs", "timestamp.clock", &format!(
            "clock check over {secs:.1}s: GPU timestamps advance at {rate:.0} Hz, reported frequency {} Hz (error {:+.4}%){}",
            g.freq, err * 100.0, if err.abs() > 0.001 { " MISMATCH: title GPU times are scaled wrong" } else { "" }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn measured_rate_matches_clock() {
        let qpf = 10_000_000u64;
        let a = (1_000_000u64, 50_000_000u64);
        let b = (a.0 + 24_000_000, a.1 + qpf);
        assert_eq!(measured_rate(a, b, qpf), Some(24_000_000.0));
        assert_eq!(measured_rate(b, a, qpf), None);
    }

    #[test]
    fn ring_index_wraps_within_heap() {
        let c = AtomicU32::new(HEAP_SLOTS - 1);
        assert_eq!(next_index(&c), HEAP_SLOTS - 1);
        assert_eq!(next_index(&c), 0);
        assert_eq!(next_index(&c), 1);
    }
}
