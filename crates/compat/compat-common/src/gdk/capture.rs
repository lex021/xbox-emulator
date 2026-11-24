use super::events::Registry;
use super::{cstr, data_root, log, setting, title, Guid, Iface, This, E_INVALIDARG, E_NOT_SUFFICIENT_BUFFER, S_OK};
use core::ffi::c_void;
use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

pub const BROADCAST: Guid = Guid::parse("186d5592-a72d-45fb-9560-11aed0e6647a");
pub const CAPTURE: Guid = Guid::parse("a4f1aee2-4bf1-4485-b008-a7c26d52ac27");
pub const CAPTURE_IID: Guid = Guid::parse("3a949778-772e-4799-bdea-0a6639e96baa");

pub const DIAG_RESULT_SIZE: usize = 8 + 10 * 280;
pub const TAKE_RESULT_SIZE: usize = 252;
pub const CLIP_RESULT_SIZE: usize = 296;
const E_NOT_FOUND: i32 = 0x8007_0490u32 as i32;
const E_HANDLE: i32 = 0x8007_0006u32 as i32;

pub fn capture_dir() -> PathBuf {
    data_root().join("captures").join(super::sanitize(&title().key))
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

static BROADCAST_CHANGED: Registry = Registry::new();
static METADATA_PURGED: Registry = Registry::new();
static METADATA_LOCK: Mutex<()> = Mutex::new(());

unsafe extern "system" fn is_broadcasting(_t: This) -> bool {
    false
}

unsafe extern "system" fn show_broadcast_ui(_t: This, _user: usize) -> i32 {
    let hr = super::win::shell_open("ms-gamebar:", None);
    log("XAppBroadcastShowUI", &format!("-> Xbox Game Bar hr={hr:#010x}"));
    hr
}

unsafe extern "system" fn broadcast_status(_t: This, _user: usize, out: *mut u8) -> i32 {
    if out.is_null() {
        return E_INVALIDARG;
    }
    let mut s = [0u8; 9];
    s[8] = 1;
    core::ptr::copy_nonoverlapping(s.as_ptr(), out, s.len());
    S_OK
}

unsafe extern "system" fn register_broadcast(_t: This, queue: usize, ctx: *mut c_void, cb: usize, token: *mut u64) -> i32 {
    BROADCAST_CHANGED.register(0, queue, ctx, cb, token)
}

unsafe extern "system" fn unregister_broadcast(_t: This, token: u64, wait: bool) -> bool {
    BROADCAST_CHANGED.unregister(None, token, wait)
}

fn metadata_quota() -> u64 {
    setting("capture_metadata_kb").and_then(|v| v.parse::<u64>().ok()).unwrap_or(4096) * 1024
}

fn metadata_path() -> PathBuf {
    capture_dir().join("metadata.jsonl")
}

fn metadata_used() -> u64 {
    std::fs::metadata(metadata_path()).map(|m| m.len()).unwrap_or(0)
}

fn states() -> &'static Mutex<HashMap<String, serde_json::Value>> {
    static S: OnceLock<Mutex<HashMap<String, serde_json::Value>>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(HashMap::new()))
}

fn record(kind: &str, name: &str, value: serde_json::Value, priority: u32) -> i32 {
    if name.is_empty() || name.len() > 32 {
        return E_INVALIDARG;
    }
    let line = serde_json::json!({"t": now_ms(), "kind": kind, "name": name, "value": value, "priority": if priority == 0 { "important" } else { "informational" }}).to_string();
    let _g = METADATA_LOCK.lock();
    let path = metadata_path();
    if metadata_used() + line.len() as u64 + 1 > metadata_quota() {
        let _ = std::fs::remove_file(&path);
        log("XAppCaptureMetadata", "storage quota reached -> metadata purged");
        METADATA_PURGED.fire(None, |r| unsafe {
            let cb: unsafe extern "system" fn(*mut c_void) = core::mem::transmute(r.cb);
            cb(r.ctx as *mut c_void);
        });
    }
    let r = std::fs::create_dir_all(capture_dir()).and_then(|_| std::fs::OpenOptions::new().create(true).append(true).open(&path)).and_then(|mut f| writeln!(f, "{line}"));
    match r {
        Ok(()) => S_OK,
        Err(e) => super::win::hr_from_win32(e.raw_os_error().unwrap_or(29) as u32),
    }
}

unsafe extern "system" fn add_string_event(_t: This, name: *const u8, value: *const u8, priority: u32) -> i32 {
    record("event", &cstr(name).unwrap_or_default(), serde_json::json!(cstr(value).unwrap_or_default()), priority)
}

unsafe extern "system" fn add_int_event(_t: This, name: *const u8, value: i32, priority: u32) -> i32 {
    record("event", &cstr(name).unwrap_or_default(), serde_json::json!(value), priority)
}

unsafe extern "system" fn add_double_event(_t: This, name: *const u8, value: f64, priority: u32) -> i32 {
    record("event", &cstr(name).unwrap_or_default(), serde_json::json!(value), priority)
}

fn start_state(name: String, value: serde_json::Value, priority: u32) -> i32 {
    let hr = record("stateStart", &name, value.clone(), priority);
    if hr >= 0 {
        if let Ok(mut s) = states().lock() {
            s.insert(name, value);
        }
    }
    hr
}

unsafe extern "system" fn start_string_state(_t: This, name: *const u8, value: *const u8, priority: u32) -> i32 {
    start_state(cstr(name).unwrap_or_default(), serde_json::json!(cstr(value).unwrap_or_default()), priority)
}

unsafe extern "system" fn start_int_state(_t: This, name: *const u8, value: i32, priority: u32) -> i32 {
    start_state(cstr(name).unwrap_or_default(), serde_json::json!(value), priority)
}

unsafe extern "system" fn start_double_state(_t: This, name: *const u8, value: f64, priority: u32) -> i32 {
    start_state(cstr(name).unwrap_or_default(), serde_json::json!(value), priority)
}

unsafe extern "system" fn stop_state(_t: This, name: *const u8) -> i32 {
    let n = cstr(name).unwrap_or_default();
    let had = states().lock().map(|mut s| s.remove(&n).is_some()).unwrap_or(false);
    if !had {
        return E_NOT_FOUND;
    }
    record("stateStop", &n, serde_json::Value::Null, 1)
}

unsafe extern "system" fn stop_all_states(_t: This) -> i32 {
    let names: Vec<String> = states().lock().map(|mut s| s.drain().map(|(k, _)| k).collect()).unwrap_or_default();
    for n in names {
        let hr = record("stateStop", &n, serde_json::Value::Null, 1);
        if hr < 0 {
            return hr;
        }
    }
    S_OK
}

unsafe extern "system" fn remaining_storage(_t: This, out: *mut u64) -> i32 {
    if out.is_null() {
        return E_INVALIDARG;
    }
    *out = metadata_quota().saturating_sub(metadata_used());
    S_OK
}

unsafe extern "system" fn register_purged(_t: This, queue: usize, ctx: *mut c_void, cb: usize, token: *mut u64) -> i32 {
    METADATA_PURGED.register(0, queue, ctx, cb, token)
}

unsafe extern "system" fn unregister_purged(_t: This, token: u64, wait: bool) -> bool {
    METADATA_PURGED.unregister(None, token, wait)
}

fn grab() -> Option<super::win::Image> {
    super::win::capture(super::win::main_window())
}

fn write_cstr(dst: *mut u8, cap: usize, s: &str) {
    let b = s.as_bytes();
    let n = b.len().min(cap - 1);
    unsafe {
        core::ptr::write_bytes(dst, 0, cap);
        core::ptr::copy_nonoverlapping(b.as_ptr(), dst, n);
    }
}

fn save_png(path: &PathBuf, img: &super::win::Image) -> Result<u64, i32> {
    let png = super::user::png_rgb(img.w, img.h, &img.rgb);
    if let Some(d) = path.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    std::fs::write(path, &png).map_err(|e| super::win::hr_from_win32(e.raw_os_error().unwrap_or(29) as u32))?;
    Ok(png.len() as u64)
}

unsafe extern "system" fn diagnostic_screenshot(_t: This, _game_only: bool, formats: u32, prefix: *const u8, out: *mut u8) -> i32 {
    if out.is_null() || formats & 1 == 0 {
        return E_INVALIDARG;
    }
    let Some(img) = grab() else { return super::win::last_error_hr() };
    let name = format!("{}_{}.png", cstr(prefix).filter(|p| !p.is_empty()).unwrap_or_else(|| "diag".into()), now_ms());
    let path = capture_dir().join("diagnostic").join(name);
    let size = match save_png(&path, &img) {
        Ok(s) => s,
        Err(hr) => return hr,
    };
    core::ptr::write_bytes(out, 0, DIAG_RESULT_SIZE);
    core::ptr::write_unaligned(out as *mut usize, 1);
    let f = out.add(8);
    write_cstr(f, 260, &path.to_string_lossy());
    core::ptr::write_unaligned(f.add(264) as *mut usize, size as usize);
    core::ptr::write_unaligned(f.add(272) as *mut u32, img.w);
    core::ptr::write_unaligned(f.add(276) as *mut u32, img.h);
    log("XAppCaptureTakeDiagnosticScreenshot", &format!("{}x{} -> {}", img.w, img.h, path.display()));
    S_OK
}

fn scale(img: &super::win::Image, max_w: u32) -> (u32, u32, Vec<u8>) {
    if img.w <= max_w {
        return (img.w, img.h, img.rgb.clone());
    }
    let w = max_w;
    let h = ((img.h as u64 * w as u64) / img.w as u64).max(1) as u32;
    let mut out = vec![0u8; (w * h * 3) as usize];
    for y in 0..h {
        let sy = (y as u64 * img.h as u64 / h as u64) as usize;
        for x in 0..w {
            let sx = (x as u64 * img.w as u64 / w as u64) as usize;
            let s = (sy * img.w as usize + sx) * 3;
            let d = ((y * w + x) * 3) as usize;
            out[d..d + 3].copy_from_slice(&img.rgb[s..s + 3]);
        }
    }
    (w, h, out)
}

struct Frame {
    t_ms: u64,
    w: u32,
    h: u32,
    jpeg: Vec<u8>,
}

struct History {
    frames: std::collections::VecDeque<Frame>,
    bytes: usize,
}

fn history() -> &'static Mutex<History> {
    static H: OnceLock<Mutex<History>> = OnceLock::new();
    H.get_or_init(|| Mutex::new(History { frames: std::collections::VecDeque::new(), bytes: 0 }))
}

pub fn history_seconds() -> u64 {
    setting("capture_history_s").and_then(|v| v.parse().ok()).unwrap_or(15).clamp(1, 120)
}

pub const HISTORY_FPS: u64 = 10;
const HISTORY_WIDTH: u32 = 480;

pub fn push_history_frame(t_ms: u64, w: u32, h: u32, rgb: &[u8]) -> Result<(), i32> {
    let jpeg = super::media::jpeg_encode(w, h, rgb)?;
    let mut hs = history().lock().map_err(|_| super::E_FAIL)?;
    let keep = history_seconds() * 1000;
    hs.bytes += jpeg.len();
    hs.frames.push_back(Frame { t_ms, w, h, jpeg });
    while hs.frames.front().map(|x| t_ms.saturating_sub(x.t_ms) > keep).unwrap_or(false) {
        if let Some(f) = hs.frames.pop_front() {
            hs.bytes -= f.jpeg.len();
        }
    }
    Ok(())
}

pub fn start_history() {
    static STARTED: AtomicBool = AtomicBool::new(false);
    if STARTED.swap(true, Ordering::AcqRel) || setting("capture_history").as_deref() == Some("0") {
        return;
    }
    log("XAppCapture", &format!("diagnostic clip history on: last {} s at {HISTORY_FPS} fps, up to {HISTORY_WIDTH} px wide, JPEG frames", history_seconds()));
    std::thread::spawn(|| {
        super::media::init_com_here();
        let mut failed = false;
        loop {
            let t0 = std::time::Instant::now();
            if let Some(img) = grab() {
                let (w, h, px) = scale(&img, HISTORY_WIDTH);
                if let Err(hr) = push_history_frame(now_ms(), w, h, &px) {
                    if !failed {
                        log("XAppCapture", &format!("history frame encode failed hr={:#010x}", hr as u32));
                        failed = true;
                    }
                }
            }
            let spent = t0.elapsed().as_millis() as u64;
            std::thread::sleep(std::time::Duration::from_millis((1000 / HISTORY_FPS).saturating_sub(spent)));
        }
    });
}

pub fn history_len() -> usize {
    history().lock().map(|h| h.frames.len()).unwrap_or(0)
}

pub fn history_bytes() -> usize {
    history().lock().map(|h| h.bytes).unwrap_or(0)
}

pub struct Clip {
    pub path: PathBuf,
    pub size: u64,
    pub start_s: i64,
    pub duration_ms: u32,
    pub w: u32,
    pub h: u32,
}

pub fn encode_clip(begin: u64, end: u64, prefix: &str) -> Result<Clip, i32> {
    let (frames, w, h, first) = {
        let hs = history().lock().map_err(|_| super::E_FAIL)?;
        let sel: Vec<&Frame> = hs.frames.iter().filter(|f| f.t_ms >= begin && f.t_ms <= end).collect();
        let Some(last) = sel.last() else { return Err(super::E_NOT_FOUND) };
        let (w, h) = (last.w, last.h);
        let same: Vec<Vec<u8>> = sel.iter().filter(|f| f.w == w && f.h == h).map(|f| f.jpeg.clone()).collect();
        let first = sel.iter().find(|f| f.w == w && f.h == h).map(|f| f.t_ms).unwrap_or(begin);
        (same, w, h, first)
    };
    let name = format!("{}_{}.mp4", if prefix.is_empty() { "clip" } else { prefix }, now_ms());
    let path = capture_dir().join("diagnostic").join(super::sanitize(&name));
    let _ = std::fs::create_dir_all(path.parent().unwrap());
    let p = path.clone();
    let (size, n) = super::media::com_thread(move || {
        let mut it = frames.into_iter();
        super::media::h264_mp4(&p, w, h, HISTORY_FPS as u32, || it.next().and_then(|j| super::media::jpeg_decode(&j).ok()).map(|(_, _, rgb)| rgb))
    })
    .unwrap_or(Err(super::E_FAIL))?;
    let (ew, eh) = super::media::even_dims(w, h);
    Ok(Clip { path, size, start_s: (first / 1000) as i64, duration_ms: (n as u64 * 1000 / HISTORY_FPS) as u32, w: ew, h: eh })
}

unsafe extern "system" fn diagnostic_clip(_t: This, start: i64, duration_ms: u32, prefix: *const u8, out: *mut u8) -> i32 {
    if out.is_null() || duration_ms == 0 {
        return E_INVALIDARG;
    }
    start_history();
    let max_ms = history_seconds() * 1000;
    let duration = (duration_ms as u64).min(max_ms);
    let now = now_ms();
    let mut begin = start.max(0) as u64 * 1000;
    if begin + duration > now + max_ms {
        return E_INVALIDARG;
    }
    if begin < now.saturating_sub(max_ms) {
        begin = now.saturating_sub(max_ms);
    }
    let end = begin + duration;
    while now_ms() < end {
        std::thread::sleep(std::time::Duration::from_millis((end - now_ms()).min(100)));
    }
    std::thread::sleep(std::time::Duration::from_millis(1000 / HISTORY_FPS + 20));
    let clip = match encode_clip(begin, end, &cstr(prefix).unwrap_or_default()) {
        Ok(c) => c,
        Err(hr) => return hr,
    };
    core::ptr::write_bytes(out, 0, CLIP_RESULT_SIZE);
    write_cstr(out, 260, &clip.path.to_string_lossy());
    core::ptr::write_unaligned(out.add(264) as *mut usize, clip.size as usize);
    core::ptr::write_unaligned(out.add(272) as *mut i64, clip.start_s);
    core::ptr::write_unaligned(out.add(280) as *mut u32, clip.duration_ms);
    core::ptr::write_unaligned(out.add(284) as *mut u32, clip.w);
    core::ptr::write_unaligned(out.add(288) as *mut u32, clip.h);
    *out.add(292) = 0;
    log("XAppCaptureRecordDiagnosticClip", &format!("H.264 {}x{} {} ms from history [{begin}..{end}] -> {}", clip.w, clip.h, clip.duration_ms, clip.path.display()));
    S_OK
}

static RECORD_ENABLED: AtomicBool = AtomicBool::new(true);

pub fn screenshot_path(local_id: &str) -> PathBuf {
    capture_dir().join("screenshots").join(format!("{}.png", super::sanitize(local_id)))
}

unsafe extern "system" fn take_screenshot(_t: This, _user: usize, out: *mut u8) -> i32 {
    if out.is_null() {
        return E_INVALIDARG;
    }
    let Some(img) = grab() else { return super::win::last_error_hr() };
    let id = format!("{}-{}", title().key, now_ms());
    if let Err(hr) = save_png(&screenshot_path(&id), &img) {
        return hr;
    }
    core::ptr::write_bytes(out, 0, TAKE_RESULT_SIZE);
    write_cstr(out, 250, &id);
    core::ptr::write_unaligned(out.add(250) as *mut u16, 1);
    log("XAppCaptureTakeScreenshot", &format!("{}x{} -> {id}", img.w, img.h));
    S_OK
}

fn streams() -> &'static Mutex<HashMap<usize, std::fs::File>> {
    static S: OnceLock<Mutex<HashMap<usize, std::fs::File>>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(HashMap::new()))
}

unsafe extern "system" fn open_stream(_t: This, local_id: *const u8, format: u32, handle: *mut usize, total: *mut u64) -> i32 {
    static NEXT: AtomicUsize = AtomicUsize::new(0x5353_0001);
    let Some(id) = cstr(local_id) else { return E_INVALIDARG };
    if handle.is_null() || format != 1 {
        return E_INVALIDARG;
    }
    let f = match std::fs::File::open(screenshot_path(&id)) {
        Ok(f) => f,
        Err(_) => return E_NOT_FOUND,
    };
    if !total.is_null() {
        *total = f.metadata().map(|m| m.len()).unwrap_or(0);
    }
    let h = NEXT.fetch_add(1, Ordering::Relaxed);
    if let Ok(mut s) = streams().lock() {
        s.insert(h, f);
    }
    *handle = h;
    S_OK
}

unsafe extern "system" fn read_stream(_t: This, handle: usize, start: u64, bytes: u32, buf: *mut u8, written: *mut u32) -> i32 {
    if buf.is_null() && bytes > 0 {
        return E_NOT_SUFFICIENT_BUFFER;
    }
    let Ok(mut s) = streams().lock() else { return E_HANDLE };
    let Some(f) = s.get_mut(&handle) else { return E_HANDLE };
    let mut tmp = vec![0u8; bytes as usize];
    let n = match f.seek(SeekFrom::Start(start)).and_then(|_| f.read(&mut tmp)) {
        Ok(n) => n,
        Err(e) => return super::win::hr_from_win32(e.raw_os_error().unwrap_or(30) as u32),
    };
    core::ptr::copy_nonoverlapping(tmp.as_ptr(), buf, n);
    if !written.is_null() {
        *written = n as u32;
    }
    S_OK
}

unsafe extern "system" fn close_stream(_t: This, handle: usize) -> i32 {
    match streams().lock().map(|mut s| s.remove(&handle).is_some()) {
        Ok(true) => S_OK,
        _ => E_HANDLE,
    }
}

unsafe extern "system" fn enable_record(_t: This) -> i32 {
    RECORD_ENABLED.store(true, Ordering::Release);
    log("XAppCaptureEnableRecord", "user captures enabled");
    S_OK
}

unsafe extern "system" fn disable_record(_t: This) -> i32 {
    RECORD_ENABLED.store(false, Ordering::Release);
    log("XAppCaptureDisableRecord", "user captures disabled");
    S_OK
}

pub fn record_enabled() -> bool {
    RECORD_ENABLED.load(Ordering::Acquire)
}

pub fn interfaces() -> Vec<Iface> {
    vec![
        Iface {
            clsid: BROADCAST,
            iid: BROADCAST,
            name: "XAppBroadcast+XAppCaptureMetadata(186d5592)",
            slots: vec![
                (3, is_broadcasting as usize),
                (4, show_broadcast_ui as usize),
                (5, broadcast_status as usize),
                (6, register_broadcast as usize),
                (7, unregister_broadcast as usize),
                (8, add_string_event as usize),
                (9, add_int_event as usize),
                (10, add_double_event as usize),
                (11, start_string_state as usize),
                (12, start_int_state as usize),
                (13, start_double_state as usize),
                (14, stop_state as usize),
                (15, stop_all_states as usize),
                (16, remaining_storage as usize),
                (17, register_purged as usize),
                (18, unregister_purged as usize),
            ],
        },
        Iface {
            clsid: CAPTURE,
            iid: CAPTURE_IID,
            name: "XAppCapture(3a949778)",
            slots: vec![
                (3, diagnostic_screenshot as usize),
                (4, diagnostic_clip as usize),
                (5, take_screenshot as usize),
                (6, open_stream as usize),
                (7, read_stream as usize),
                (8, close_stream as usize),
                (9, enable_record as usize),
                (10, disable_record as usize),
            ],
        },
    ]
}
