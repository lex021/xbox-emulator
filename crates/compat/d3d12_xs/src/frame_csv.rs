use crate::frame::{qpc, qpf};
use compat_common::record;
use core::ffi::c_void;
use std::io::Write;
use std::sync::Mutex;

static SW: crate::live_sw::Sw = crate::live_sw::Sw::new("TOMBOX_FRAME_CSV", "frame_csv", false);

#[repr(C)]
struct Guid(u32, u16, u16, [u8; 8]);
const IID_QUERY_HEAP: Guid = Guid(0x0d9658ae, 0xed45, 0x469e, [0xa6, 0x1d, 0x97, 0x0e, 0xc5, 0x83, 0xca, 0xb4]);
const IID_RESOURCE: Guid = Guid(0x696442be, 0xa72e, 0x4059, [0xbc, 0x79, 0x5b, 0x5c, 0x98, 0x04, 0x0f, 0xad]);

unsafe fn vslot(o: usize, i: usize) -> usize {
    unsafe { *(*(o as *const *const usize)).add(i) }
}

macro_rules! vc {
    ($o:expr, $i:expr, fn($($t:ty),*) -> $r:ty $(, $a:expr)*) => {{
        let o = $o as usize;
        let f: unsafe extern "system" fn(usize $(, $t)*) -> $r = core::mem::transmute(vslot(o, $i));
        f(o $(, $a)*)
    }};
}

#[derive(Clone, Copy, Default)]
struct Pending {
    frame: u64,
    t_ms: f64,
    interval_ms: f64,
    cpu_ms: f64,
    wait_ms: f64,
    valid: bool,
    ev: crate::frame_ev::Snap,
    game_dt: Option<f64>,
}

struct Csv {
    out: std::io::BufWriter<std::fs::File>,
    path: String,
    qheap: usize,
    rb: usize,
    map: *const u64,
    freq: u64,
    pending: [Pending; 2],
    last_gpu_start: u64,
    start_qpc: i64,
    last_present: i64,
    rows: u64,
    wait_ticks: i64,
    snap: [(u64, u64); 2],
    qpc_last: i64,
    last_flush: i64,
}
unsafe impl Send for Csv {}

impl Csv {
    fn wrote_row(&mut self, now: i64) {
        self.rows += 1;
        if self.rows % 240 == 0 || self.rows <= 2 || now - self.last_flush >= qpf() / 4 {
            let _ = self.out.flush();
            self.last_flush = now;
        }
    }
}

fn open_file(p: &std::path::Path) -> Result<Csv, String> {
    let f = std::fs::File::create(p).map_err(|e| format!("create {}: {e}", p.display()))?;
    let mut out = std::io::BufWriter::with_capacity(64 * 1024, f);
    let _ = writeln!(out, "frame,t_ms,present_interval_ms,present_cpu_ms,fence_wait_ms,gpu_frame_ms,gpu_present_list_us{}", crate::frame_ev::header());
    let _ = out.flush();
    let now = qpc();
    Ok(Csv { out, path: p.display().to_string(), qheap: 0, rb: 0, map: core::ptr::null(), freq: 0, pending: [Pending::default(); 2], last_gpu_start: 0, start_qpc: now, last_present: 0, rows: 0, wait_ticks: 0, snap: [(0, 0); 2], qpc_last: 0, last_flush: now })
}

static CSV: Mutex<Option<Csv>> = Mutex::new(None);
static FAILED: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

fn path() -> std::path::PathBuf {
    let log = std::env::var("TOMBOX_LOG").map(std::path::PathBuf::from).unwrap_or_else(|_| std::env::temp_dir().join("tombox_compat.log"));
    log.parent().map(|d| d.join("tombox_frames.csv")).unwrap_or_else(|| std::env::temp_dir().join("tombox_frames.csv"))
}

unsafe fn open(device: usize, freq: u64) -> Result<Csv, String> {
    let p = path();
    let mut c = open_file(&p)?;
    let qd: [u32; 3] = [1, 4, 0];
    let mut qheap = 0usize;
    let mut rb = 0usize;
    let mut map: *const u64 = core::ptr::null();
    if freq != 0 && unsafe { vc!(device, 39, fn(*const u32, *const Guid, *mut usize) -> i32, qd.as_ptr(), &IID_QUERY_HEAP, &mut qheap) } >= 0 && qheap != 0 {
        let props: [u32; 5] = [3, 0, 0, 1, 1];
        let mut d = [0u8; 0x38];
        d[0..4].copy_from_slice(&1u32.to_le_bytes());
        d[0x10..0x18].copy_from_slice(&256u64.to_le_bytes());
        d[0x18..0x1c].copy_from_slice(&1u32.to_le_bytes());
        d[0x1c..0x1e].copy_from_slice(&1u16.to_le_bytes());
        d[0x1e..0x20].copy_from_slice(&1u16.to_le_bytes());
        d[0x24..0x28].copy_from_slice(&1u32.to_le_bytes());
        d[0x2c..0x30].copy_from_slice(&1u32.to_le_bytes());
        let hr = unsafe { vc!(device, 27, fn(*const u32, u32, *const u8, u32, *const c_void, *const Guid, *mut usize) -> i32, props.as_ptr(), 0, d.as_ptr(), 0x400, core::ptr::null(), &IID_RESOURCE, &mut rb) };
        if hr >= 0 && rb != 0 {
            let mut m: *mut u8 = core::ptr::null_mut();
            if unsafe { vc!(rb, 8, fn(u32, *const c_void, *mut *mut u8) -> i32, 0, core::ptr::null(), &mut m) } >= 0 && !m.is_null() {
                map = m as *const u64;
            }
        }
        if map.is_null() {
            qheap = 0;
        }
    }
    record("d3d12_xs", "[frame_csv]", &format!("writing {} (gpu timestamps {})", p.display(), if qheap != 0 { "on" } else { "off" }));
    crate::frame_ev::set_active(true);
    (c.qheap, c.rb, c.map, c.freq) = (qheap, rb, map, freq);
    Ok(c)
}

pub fn on() -> bool {
    !FAILED.load(core::sync::atomic::Ordering::Relaxed) && SW.on()
}

pub fn close() {
    crate::frame_ev::set_active(false);
    if let Ok(mut g) = CSV.lock() {
        if let Some(c) = g.as_mut() {
            let _ = c.out.flush();
            record("d3d12_xs", "[frame_csv]", &format!("closed {} after {} rows", c.path, c.rows));
        }
        *g = None;
    }
}

pub unsafe fn list_begin(device: *mut c_void, list: *mut c_void, frame: u64, freq: u64) {
    if !on() {
        close();
        return;
    }
    let mut g = CSV.lock().unwrap();
    if g.is_none() {
        match unsafe { open(device as usize, freq) } {
            Ok(c) => *g = Some(c),
            Err(e) => {
                FAILED.store(true, core::sync::atomic::Ordering::Relaxed);
                record("d3d12_xs", "[frame_csv]", &format!("disabled: {e}"));
                return;
            }
        }
    }
    let c = g.as_mut().unwrap();
    if c.qheap != 0 && !crate::dred::removed() {
        let slot = (frame % 2) as u32;
        let i = slot as usize;
        c.snap[i] = unsafe { (core::ptr::read_volatile(c.map.add(i * 2)), core::ptr::read_volatile(c.map.add(i * 2 + 1))) };
        unsafe { vc!(list, 53, fn(usize, u32, u32) -> (), c.qheap, 2, slot * 2) };
    }
}

pub unsafe fn list_end(list: *mut c_void, frame: u64) {
    let g = CSV.lock().unwrap();
    let Some(c) = g.as_ref() else { return };
    if c.qheap != 0 && !crate::dred::removed() {
        let slot = (frame % 2) as u32;
        unsafe {
            vc!(list, 53, fn(usize, u32, u32) -> (), c.qheap, 2, slot * 2 + 1);
            vc!(list, 54, fn(usize, u32, u32, u32, usize, u64) -> (), c.qheap, 2, slot * 2, 2, c.rb, slot as u64 * 16);
        }
    }
}

pub fn add_wait(ticks: i64) {
    if let Ok(mut g) = CSV.try_lock() {
        if let Some(c) = g.as_mut() {
            c.wait_ticks += ticks;
        }
    }
}

pub fn present_skipped(entry: i64) {
    let mut g = CSV.lock().unwrap_or_else(|e| e.into_inner());
    let Some(c) = g.as_mut() else { return };
    let now = qpc();
    let f = qpf() as f64;
    let ms = |t: i64| t as f64 * 1000.0 / f;
    if c.last_present == 0 {
        c.start_qpc = entry;
    }
    let interval = if c.last_present != 0 { ms(entry - c.last_present) } else { 0.0 };
    c.last_present = entry;
    crate::frame_ev::note_present_thread();
    let ev = crate::frame_ev::take();
    let game_dt = crate::frame_ev::game_dt_ms(c.qpc_last, &ev);
    if ev.qpc_last != 0 {
        c.qpc_last = ev.qpc_last;
    }
    let _ = writeln!(c.out, "0,{:.3},{interval:.3},{:.3},{:.3},,{}", ms(entry - c.start_qpc), ms(now - entry), ms(c.wait_ticks), crate::frame_ev::row(&ev, game_dt));
    c.wait_ticks = 0;
    c.wrote_row(now);
}

pub fn present_done(frame: u64, entry: i64) {
    let mut g = CSV.lock().unwrap();
    let Some(c) = g.as_mut() else { return };
    let now = qpc();
    let f = qpf() as f64;
    let ms = |t: i64| t as f64 * 1000.0 / f;
    let slot = (frame % 2) as usize;
    let old = c.pending[slot];
    let (mut gpu_frame, mut gpu_list) = (String::new(), String::new());
    if old.valid && c.qheap != 0 && !crate::dred::removed() {
        let (a, z) = c.snap[slot];
        if z >= a && a != 0 {
            gpu_list = format!("{:.1}", (z - a) as f64 * 1e6 / c.freq as f64);
            if c.last_gpu_start != 0 && a > c.last_gpu_start {
                gpu_frame = format!("{:.3}", (a - c.last_gpu_start) as f64 * 1000.0 / c.freq as f64);
            }
            c.last_gpu_start = a;
        }
    }
    if old.valid {
        let _ = writeln!(c.out, "{},{:.3},{:.3},{:.3},{:.3},{gpu_frame},{gpu_list}{}", old.frame, old.t_ms, old.interval_ms, old.cpu_ms, old.wait_ms, crate::frame_ev::row(&old.ev, old.game_dt));
        c.wrote_row(now);
    }
    if c.last_present == 0 {
        c.start_qpc = entry;
    }
    let interval = if c.last_present != 0 { ms(entry - c.last_present) } else { 0.0 };
    c.last_present = entry;
    crate::frame_ev::note_present_thread();
    let ev = crate::frame_ev::take();
    let game_dt = crate::frame_ev::game_dt_ms(c.qpc_last, &ev);
    if ev.qpc_last != 0 {
        c.qpc_last = ev.qpc_last;
    }
    c.pending[slot] = Pending { frame, t_ms: ms(entry - c.start_qpc), interval_ms: interval, cpu_ms: ms(now - entry), wait_ms: ms(c.wait_ticks), valid: true, ev, game_dt };
    c.wait_ticks = 0;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_title_present_reaches_the_file_without_a_clean_exit() {
        let p = std::env::temp_dir().join(format!("tombox_frames_test_{}.csv", std::process::id()));
        *CSV.lock().unwrap() = Some(open_file(&p).unwrap());
        let header = std::fs::read_to_string(&p).unwrap();
        assert!(header.starts_with("frame,t_ms,present_interval_ms") && header.ends_with('\n'));
        let t = qpc();
        present_done(1, t);
        present_done(2, t + 10);
        assert_eq!(CSV.lock().unwrap().as_ref().unwrap().rows, 0);
        present_done(3, t + 20);
        present_skipped(t + 30);
        let rows = CSV.lock().unwrap().as_ref().unwrap().rows;
        let body = std::fs::read_to_string(&p).unwrap();
        *CSV.lock().unwrap() = None;
        let _ = std::fs::remove_file(&p);
        assert_eq!(rows, 2);
        let lines: Vec<&str> = body.lines().collect();
        assert_eq!(lines.len(), 3, "{body}");
        assert!(lines[1].starts_with("1,"));
        assert!(lines[2].starts_with("0,"));
        assert_eq!(lines[1].split(',').count(), lines[0].split(',').count());
        assert_eq!(lines[2].split(',').count(), lines[0].split(',').count());
    }
}
