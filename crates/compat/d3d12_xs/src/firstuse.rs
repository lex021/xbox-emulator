use core::sync::atomic::{AtomicU64, Ordering};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;
use compat_common::record;

pub const CREATE: u8 = 0;
pub const UPLOAD: u8 = 1;
pub const DETILE: u8 = 2;
pub const SRV: u8 = 3;
pub const UAV: u8 = 4;
pub const RTV: u8 = 5;
pub const DSV: u8 = 6;
const KINDS: usize = 7;
const NAMES: [&str; KINDS] = ["create", "upload", "detile", "srv", "uav", "rtv", "dsv"];
const MAX_LINES: u64 = 4000;
const MAX_FRAME_LINES: u64 = 20000;

static PRESENT: AtomicU64 = AtomicU64::new(0);
static START: OnceLock<Instant> = OnceLock::new();

#[derive(Default)]
struct S {
    first: HashMap<usize, [u64; KINDS]>,
    frame: [u64; KINDS],
    bytes: [u64; KINDS],
    top: Vec<(u64, usize, u8)>,
    lines: u64,
    frame_lines: u64,
}

static ST: Mutex<Option<S>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut S) -> R) -> R {
    let mut g = ST.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(S::default))
}

pub fn on() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var("TOMBOX_FIRSTUSE").map(|v| v.trim() == "1").unwrap_or(false))
}

pub fn kind_name(k: u8) -> &'static str {
    NAMES.get(k as usize).copied().unwrap_or("?")
}

fn t_ms() -> f64 {
    START.get().map_or(0.0, |s| s.elapsed().as_secs_f64() * 1000.0)
}

pub fn present() -> u64 {
    PRESENT.load(Ordering::Relaxed)
}

pub fn on_present() {
    if !on() {
        return;
    }
    START.get_or_init(Instant::now);
    let p = PRESENT.fetch_add(1, Ordering::Relaxed);
    let inj = crate::inject::take();
    let line = with(|s| {
        let f = core::mem::take(&mut s.frame);
        let b = core::mem::take(&mut s.bytes);
        let top = core::mem::take(&mut s.top);
        if (f.iter().all(|&n| n == 0) && inj.iter().all(|&n| n == 0)) || s.frame_lines >= MAX_FRAME_LINES {
            return None;
        }
        s.frame_lines += 1;
        Some((f, b, top))
    });
    if let Some((f, b, top)) = line {
        record("d3d12_xs", "firstuse", &format!("frame present #{p} t={:.0} ms: first uses {}{}{} | injected {}", t_ms(), summary(&f), bytes_text(&b), top_text(&top), crate::inject::summary(&inj)));
    }
}

pub fn bytes_text(b: &[u64; KINDS]) -> String {
    let v: Vec<String> = b.iter().enumerate().filter(|(_, n)| **n > 0).map(|(k, n)| format!("{} {:.2} MiB", NAMES[k], *n as f64 / 1048576.0)).collect();
    if v.is_empty() { String::new() } else { format!(" | bytes {}", v.join(", ")) }
}

pub fn top_text(top: &[(u64, usize, u8)]) -> String {
    let v: Vec<String> = top.iter().map(|(b, r, k)| format!("{r:#x} {} {:.2} MiB", kind_name(*k), *b as f64 / 1048576.0)).collect();
    if v.is_empty() { String::new() } else { format!(" | top {}", v.join(", ")) }
}

fn push_top(top: &mut Vec<(u64, usize, u8)>, e: (u64, usize, u8)) {
    top.push(e);
    top.sort_by(|a, b| b.0.cmp(&a.0));
    top.truncate(5);
}

pub fn note_bytes(res: usize, kind: u8, bytes: u64) {
    if !on() || res == 0 || kind as usize >= KINDS {
        return;
    }
    with(|s| {
        s.bytes[kind as usize] += bytes;
        push_top(&mut s.top, (bytes, res, kind));
    });
    note(res, kind);
}

pub fn summary(f: &[u64; KINDS]) -> String {
    let v: Vec<String> = f.iter().enumerate().filter(|(_, n)| **n > 0).map(|(k, n)| format!("{} {n}", NAMES[k])).collect();
    v.join(", ")
}

pub fn note(res: usize, kind: u8) {
    if !on() || res == 0 || kind as usize >= KINDS {
        return;
    }
    let p = present() + 1;
    let log = with(|s| {
        let e = s.first.entry(res).or_insert([0; KINDS]);
        if kind == CREATE {
            *e = [0; KINDS];
        }
        if e[kind as usize] != 0 {
            return false;
        }
        e[kind as usize] = p;
        s.frame[kind as usize] += 1;
        if s.lines < MAX_LINES {
            s.lines += 1;
            return true;
        }
        false
    });
    if log {
        record("d3d12_xs", "firstuse", &format!("res {res:#x} first {} in frame #{p} t={:.0} ms", kind_name(kind), t_ms()));
    }
}

pub fn describe_at(e: &[u64; KINDS], now: u64) -> String {
    let v: Vec<String> = e.iter().enumerate().filter(|(_, p)| **p != 0)
        .map(|(k, p)| format!("{} #{p}{}", NAMES[k], if *p == now { " (THIS FRAME)" } else { "" })).collect();
    if v.is_empty() { " first-use: none recorded".to_string() } else { format!(" first-use: {}", v.join(", ")) }
}

pub fn describe(res: usize) -> String {
    if !on() || res == 0 {
        return String::new();
    }
    let now = present() + 1;
    match with(|s| s.first.get(&res).copied()) {
        Some(e) => describe_at(&e, now),
        None => " first-use: none recorded".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn describe_marks_this_frame_and_summary_skips_zero_kinds() {
        let mut e = [0u64; KINDS];
        e[CREATE as usize] = 3;
        e[UPLOAD as usize] = 9;
        e[SRV as usize] = 4;
        assert_eq!(describe_at(&e, 9), " first-use: create #3, upload #9 (THIS FRAME), srv #4");
        assert_eq!(describe_at(&[0; KINDS], 1), " first-use: none recorded");
        let mut f = [0u64; KINDS];
        f[DETILE as usize] = 2;
        f[RTV as usize] = 1;
        assert_eq!(summary(&f), "detile 2, rtv 1");
        let mut b = [0u64; KINDS];
        b[UPLOAD as usize] = 3 * 1048576;
        assert_eq!(bytes_text(&b), " | bytes upload 3.00 MiB");
        let mut top = Vec::new();
        for i in 0..7u64 {
            push_top(&mut top, (i * 1048576, i as usize + 1, UPLOAD));
        }
        assert_eq!(top.len(), 5);
        assert_eq!(top[0], (6 * 1048576, 7, UPLOAD));
        assert!(top_text(&top[..1]).starts_with(" | top 0x7 upload 6.00 MiB"));
    }
}
