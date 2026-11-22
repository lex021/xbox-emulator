use core::ffi::c_void;
use core::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Once;

use crate::{panel, set_buttons, PANEL_HEIGHT, PANEL_WIDTH};

const SCALE: i32 = 3;
const LIGHT_BAND: i32 = 24;
const CLIENT_W: i32 = PANEL_WIDTH as i32 * SCALE;
const CLIENT_H: i32 = PANEL_HEIGHT as i32 * SCALE + LIGHT_BAND;

pub const KEYS: [(u32, u32); 10] = [
    (0x70, 0x001),
    (0x71, 0x002),
    (0x72, 0x004),
    (0x73, 0x008),
    (0x74, 0x010),
    (0x25, 0x020),
    (0x27, 0x040),
    (0x26, 0x080),
    (0x28, 0x100),
    (0x0d, 0x200),
];

type HWND = *mut c_void;

#[repr(C)]
struct WndClassW {
    style: u32,
    proc_: unsafe extern "system" fn(HWND, u32, usize, isize) -> isize,
    cls_extra: i32,
    wnd_extra: i32,
    instance: *mut c_void,
    icon: *mut c_void,
    cursor: *mut c_void,
    background: *mut c_void,
    menu: *const u16,
    class: *const u16,
}

#[repr(C)]
struct Msg {
    hwnd: HWND,
    message: u32,
    wparam: usize,
    lparam: isize,
    time: u32,
    pt: [i32; 2],
    private: u32,
}

#[repr(C)]
struct PaintStruct {
    hdc: *mut c_void,
    erase: i32,
    rc: [i32; 4],
    restore: i32,
    inc_update: i32,
    reserved: [u8; 32],
}

#[repr(C)]
struct BitmapInfoHeader {
    size: u32,
    width: i32,
    height: i32,
    planes: u16,
    bit_count: u16,
    compression: u32,
    size_image: u32,
    xppm: i32,
    yppm: i32,
    clr_used: u32,
    clr_important: u32,
}

#[repr(C)]
struct Rect {
    l: i32,
    t: i32,
    r: i32,
    b: i32,
}

#[link(name = "user32")]
extern "system" {
    fn RegisterClassW(c: *const WndClassW) -> u16;
    fn CreateWindowExW(ex: u32, class: *const u16, title: *const u16, style: u32, x: i32, y: i32, w: i32, h: i32, parent: HWND, menu: *mut c_void, inst: *mut c_void, param: *mut c_void) -> HWND;
    fn DefWindowProcW(h: HWND, m: u32, w: usize, l: isize) -> isize;
    fn GetMessageW(m: *mut Msg, h: HWND, a: u32, b: u32) -> i32;
    fn TranslateMessage(m: *const Msg) -> i32;
    fn DispatchMessageW(m: *const Msg) -> isize;
    fn BeginPaint(h: HWND, p: *mut PaintStruct) -> *mut c_void;
    fn EndPaint(h: HWND, p: *const PaintStruct) -> i32;
    fn InvalidateRect(h: HWND, r: *const c_void, erase: i32) -> i32;
    fn AdjustWindowRect(r: *mut Rect, style: u32, menu: i32) -> i32;
    fn SetWindowTextW(h: HWND, t: *const u16) -> i32;
    fn ShowWindow(h: HWND, cmd: i32) -> i32;
}

#[link(name = "gdi32")]
extern "system" {
    fn StretchDIBits(hdc: *mut c_void, xd: i32, yd: i32, wd: i32, hd: i32, xs: i32, ys: i32, ws: i32, hs: i32, bits: *const c_void, bmi: *const BitmapInfoHeader, usage: u32, rop: u32) -> i32;
}

static HWND_CELL: AtomicUsize = AtomicUsize::new(0);
static START: Once = Once::new();
static TITLE: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn frame_bgra() -> Vec<u32> {
    let p = panel();
    let w = PANEL_WIDTH as usize;
    let h = PANEL_HEIGHT as usize + (LIGHT_BAND / SCALE) as usize;
    let mut out = vec![0u32; w * h];
    for (i, &g) in p.pixels.iter().enumerate() {
        let g = g as u32;
        out[i] = (g << 16) | (g << 8) | g;
    }
    let band0 = PANEL_HEIGHT as usize * w;
    for light in 0..5 {
        let on = p.lights & (1 << light) != 0;
        let c = if on { 0x0030_ff30 } else { 0x0020_2020 };
        let x0 = 8 + light * 24;
        for y in 2..(h - PANEL_HEIGHT as usize - 2) {
            for x in x0..x0 + 12 {
                out[band0 + y * w + x] = c;
            }
        }
    }
    out
}

unsafe extern "system" fn proc_(h: HWND, m: u32, w: usize, l: isize) -> isize {
    match m {
        0x000f => {
            let mut ps: PaintStruct = core::mem::zeroed();
            let dc = BeginPaint(h, &mut ps);
            let px = frame_bgra();
            let rows = (px.len() / PANEL_WIDTH as usize) as i32;
            let bmi = BitmapInfoHeader {
                size: core::mem::size_of::<BitmapInfoHeader>() as u32,
                width: PANEL_WIDTH as i32,
                height: -rows,
                planes: 1,
                bit_count: 32,
                compression: 0,
                size_image: 0,
                xppm: 0,
                yppm: 0,
                clr_used: 0,
                clr_important: 0,
            };
            StretchDIBits(dc, 0, 0, CLIENT_W, CLIENT_H, 0, 0, PANEL_WIDTH as i32, rows, px.as_ptr() as *const c_void, &bmi, 0, 0x00CC_0020);
            EndPaint(h, &ps);
            0
        }
        0x0100 | 0x0101 => {
            if let Some(&(_, bit)) = KEYS.iter().find(|k| k.0 == w as u32) {
                let cur = panel().buttons;
                set_buttons(if m == 0x0100 { cur | bit } else { cur & !bit });
                return 0;
            }
            DefWindowProcW(h, m, w, l)
        }
        0x0008 => {
            set_buttons(0);
            DefWindowProcW(h, m, w, l)
        }
        0x0010 => {
            ShowWindow(h, 0);
            0
        }
        _ => DefWindowProcW(h, m, w, l),
    }
}

pub fn show() {
    START.call_once(|| {
        std::thread::Builder::new()
            .name("tombox-frontpanel".into())
            .spawn(|| unsafe {
                let class = wide("TomBoxFrontPanel");
                let wc = WndClassW {
                    style: 0,
                    proc_,
                    cls_extra: 0,
                    wnd_extra: 0,
                    instance: core::ptr::null_mut(),
                    icon: core::ptr::null_mut(),
                    cursor: core::ptr::null_mut(),
                    background: core::ptr::null_mut(),
                    menu: core::ptr::null(),
                    class: class.as_ptr(),
                };
                RegisterClassW(&wc);
                let style = 0x00C0_0000 | 0x0008_0000;
                let mut r = Rect { l: 0, t: 0, r: CLIENT_W, b: CLIENT_H };
                AdjustWindowRect(&mut r, style, 0);
                let t = TITLE.lock().map(|s| s.clone()).unwrap_or_default();
                let title = wide(&format!("Front panel{}", if t.is_empty() { String::new() } else { format!(" - {t}") }));
                let h = CreateWindowExW(0x0000_0008 | 0x0000_0080, class.as_ptr(), title.as_ptr(), style, 40, 40, r.r - r.l, r.b - r.t, core::ptr::null_mut(), core::ptr::null_mut(), core::ptr::null_mut(), core::ptr::null_mut());
                HWND_CELL.store(h as usize, Ordering::Release);
                if compat_common::headless::enabled() {
                    compat_common::headless::cloak(h as *mut c_void);
                }
                ShowWindow(h, 4);
                compat_common::record("XFrontPanelDisplay", "window", &format!("front panel window {h:p} (F1-F5 buttons 1-5, arrows, Enter = select)"));
                let mut msg: Msg = core::mem::zeroed();
                while GetMessageW(&mut msg, core::ptr::null_mut(), 0, 0) > 0 {
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            })
            .ok();
    });
}

pub fn invalidate() {
    let h = HWND_CELL.load(Ordering::Acquire);
    if h != 0 {
        unsafe { InvalidateRect(h as HWND, core::ptr::null(), 0) };
    }
}

pub fn set_title(t: &str) {
    if let Ok(mut s) = TITLE.lock() {
        *s = t.to_string();
    }
    let h = HWND_CELL.load(Ordering::Acquire);
    if h != 0 {
        let w = wide(&format!("Front panel - {t}"));
        unsafe { SetWindowTextW(h as HWND, w.as_ptr()) };
    }
}
