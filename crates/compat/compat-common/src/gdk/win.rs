use core::ffi::c_void;
use std::cell::RefCell;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

pub type Hwnd = *mut c_void;

#[repr(C)]
#[derive(Default, Clone, Copy)]
pub struct Rect {
    pub l: i32,
    pub t: i32,
    pub r: i32,
    pub b: i32,
}

#[repr(C)]
struct Msg {
    hwnd: Hwnd,
    message: u32,
    wparam: usize,
    lparam: isize,
    time: u32,
    pt: [i32; 2],
    private: u32,
}

#[repr(C)]
struct WndClassExW {
    cb_size: u32,
    style: u32,
    wnd_proc: usize,
    cls_extra: i32,
    wnd_extra: i32,
    instance: *mut c_void,
    icon: *mut c_void,
    cursor: *mut c_void,
    background: *mut c_void,
    menu_name: *const u16,
    class_name: *const u16,
    icon_sm: *mut c_void,
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

#[link(name = "user32")]
extern "system" {
    fn RegisterClassExW(c: *const WndClassExW) -> u16;
    fn CreateWindowExW(ex: u32, class: *const u16, title: *const u16, style: u32, x: i32, y: i32, w: i32, h: i32, parent: Hwnd, menu: usize, inst: *mut c_void, param: *mut c_void) -> Hwnd;
    fn DefWindowProcW(h: Hwnd, m: u32, w: usize, l: isize) -> isize;
    fn DestroyWindow(h: Hwnd) -> i32;
    fn GetMessageW(m: *mut Msg, h: Hwnd, a: u32, b: u32) -> i32;
    fn TranslateMessage(m: *const Msg) -> i32;
    fn DispatchMessageW(m: *const Msg) -> isize;
    fn IsDialogMessageW(h: Hwnd, m: *mut Msg) -> i32;
    fn PostQuitMessage(code: i32);
    pub fn PostMessageW(h: Hwnd, m: u32, w: usize, l: isize) -> i32;
    fn SendMessageW(h: Hwnd, m: u32, w: usize, l: isize) -> isize;
    fn GetWindowTextW(h: Hwnd, buf: *mut u16, n: i32) -> i32;
    fn GetWindowTextLengthW(h: Hwnd) -> i32;
    fn GetDlgItem(h: Hwnd, id: i32) -> Hwnd;
    fn GetSystemMetrics(i: i32) -> i32;
    fn ShowWindow(h: Hwnd, cmd: i32) -> i32;
    fn SetForegroundWindow(h: Hwnd) -> i32;
    fn SetFocus(h: Hwnd) -> Hwnd;
    fn EnableWindow(h: Hwnd, en: i32) -> i32;
    fn EnumWindows(cb: unsafe extern "system" fn(Hwnd, isize) -> i32, l: isize) -> i32;
    fn GetWindowThreadProcessId(h: Hwnd, pid: *mut u32) -> u32;
    fn IsWindowVisible(h: Hwnd) -> i32;
    fn GetWindow(h: Hwnd, cmd: u32) -> Hwnd;
    fn GetWindowRect(h: Hwnd, r: *mut Rect) -> i32;
    fn GetClientRect(h: Hwnd, r: *mut Rect) -> i32;
    fn GetDC(h: Hwnd) -> *mut c_void;
    fn ReleaseDC(h: Hwnd, dc: *mut c_void) -> i32;
    fn PrintWindow(h: Hwnd, dc: *mut c_void, flags: u32) -> i32;
    fn SystemParametersInfoW(action: u32, p: u32, pv: *mut c_void, ini: u32) -> i32;
    fn FindWindowExW(parent: Hwnd, after: Hwnd, class: *const u16, title: *const u16) -> Hwnd;
}

#[link(name = "gdi32")]
extern "system" {
    fn GetStockObject(i: i32) -> *mut c_void;
    fn CreateCompatibleDC(dc: *mut c_void) -> *mut c_void;
    fn CreateDIBSection(dc: *mut c_void, bmi: *const BitmapInfoHeader, usage: u32, bits: *mut *mut u8, sec: *mut c_void, off: u32) -> *mut c_void;
    fn SelectObject(dc: *mut c_void, o: *mut c_void) -> *mut c_void;
    fn DeleteObject(o: *mut c_void) -> i32;
    fn DeleteDC(dc: *mut c_void) -> i32;
    fn BitBlt(dst: *mut c_void, x: i32, y: i32, w: i32, h: i32, src: *mut c_void, sx: i32, sy: i32, rop: u32) -> i32;
}

#[link(name = "shell32")]
extern "system" {
    fn ShellExecuteW(h: Hwnd, op: *const u16, file: *const u16, params: *const u16, dir: *const u16, show: i32) -> isize;
}

#[link(name = "kernel32")]
extern "system" {
    fn GetModuleHandleW(name: *const u16) -> *mut c_void;
    fn GetCurrentProcessId() -> u32;
    fn PowerCreateRequest(ctx: *const ReasonContext) -> *mut c_void;
    fn PowerSetRequest(h: *mut c_void, kind: u32) -> i32;
    fn PowerClearRequest(h: *mut c_void, kind: u32) -> i32;
    fn CloseHandle(h: *mut c_void) -> i32;
    fn GetLastError() -> u32;
}

#[link(name = "advapi32")]
extern "system" {
    fn RegGetValueW(key: usize, sub: *const u16, val: *const u16, flags: u32, kind: *mut u32, data: *mut c_void, cb: *mut u32) -> i32;
}

#[repr(C)]
struct ReasonContext {
    version: u32,
    flags: u32,
    reason: *const u16,
}

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

pub fn hr_from_win32(e: u32) -> i32 {
    if e == 0 { 0 } else { ((e & 0xffff) | 0x8007_0000) as i32 }
}

pub fn last_error_hr() -> i32 {
    let e = unsafe { GetLastError() };
    if e == 0 { super::E_FAIL } else { hr_from_win32(e) }
}

pub const HKCU: usize = 0xffff_ffff_8000_0001;
pub const HKLM: usize = 0xffff_ffff_8000_0002;

pub fn reg_dword(root: usize, sub: &str, val: &str) -> Option<u32> {
    let (s, v) = (wide(sub), wide(val));
    let mut d = 0u32;
    let mut cb = 4u32;
    let r = unsafe { RegGetValueW(root, s.as_ptr(), v.as_ptr(), 0x10, core::ptr::null_mut(), &mut d as *mut u32 as *mut c_void, &mut cb) };
    if r == 0 { Some(d) } else { None }
}

pub fn reg_string(root: usize, sub: &str, val: &str) -> Option<String> {
    let (s, v) = (wide(sub), wide(val));
    let mut buf = vec![0u16; 1024];
    let mut cb = (buf.len() * 2) as u32;
    let r = unsafe { RegGetValueW(root, s.as_ptr(), v.as_ptr(), 0x02, core::ptr::null_mut(), buf.as_mut_ptr() as *mut c_void, &mut cb) };
    if r != 0 {
        return None;
    }
    let n = (cb as usize / 2).saturating_sub(1).min(buf.len());
    Some(String::from_utf16_lossy(&buf[..n]))
}

pub fn shell_open(target: &str, params: Option<&str>) -> i32 {
    let (op, f) = (wide("open"), wide(target));
    let p = params.map(wide);
    let r = unsafe { ShellExecuteW(core::ptr::null_mut(), op.as_ptr(), f.as_ptr(), p.as_ref().map_or(core::ptr::null(), |x| x.as_ptr()), core::ptr::null(), 1) };
    if r > 32 {
        0
    } else {
        match r {
            2 | 3 => hr_from_win32(r as u32),
            5 => hr_from_win32(5),
            8 => hr_from_win32(8),
            31 => hr_from_win32(1155),
            _ => super::E_FAIL,
        }
    }
}

pub struct PowerRequest(usize);

impl PowerRequest {
    pub fn display(reason: &str) -> Result<PowerRequest, i32> {
        let w = wide(reason);
        let ctx = ReasonContext { version: 0, flags: 1, reason: w.as_ptr() };
        let h = unsafe { PowerCreateRequest(&ctx) };
        if h.is_null() || h as isize == -1 {
            return Err(last_error_hr());
        }
        unsafe {
            if PowerSetRequest(h, 0) == 0 || PowerSetRequest(h, 1) == 0 {
                let e = last_error_hr();
                CloseHandle(h);
                return Err(e);
            }
        }
        Ok(PowerRequest(h as usize))
    }
}

impl Drop for PowerRequest {
    fn drop(&mut self) {
        unsafe {
            let h = self.0 as *mut c_void;
            PowerClearRequest(h, 0);
            PowerClearRequest(h, 1);
            CloseHandle(h);
        }
    }
}

#[repr(C)]
struct HighContrastW {
    size: u32,
    flags: u32,
    scheme: *mut u16,
}

pub fn high_contrast() -> (bool, String) {
    let mut hc = HighContrastW { size: core::mem::size_of::<HighContrastW>() as u32, flags: 0, scheme: core::ptr::null_mut() };
    let ok = unsafe { SystemParametersInfoW(0x42, hc.size, &mut hc as *mut HighContrastW as *mut c_void, 0) } != 0;
    if !ok {
        return (false, String::new());
    }
    let scheme = unsafe { super::wstr(hc.scheme) }.unwrap_or_default();
    (hc.flags & 1 != 0, scheme)
}

#[derive(Clone, Default)]
pub struct Edit {
    pub default: String,
    pub max: u32,
    pub password: bool,
    pub numeric: bool,
    pub multiline: bool,
}

#[derive(Clone, Default)]
pub struct List {
    pub items: Vec<String>,
    pub multi: bool,
    pub preselected: Vec<usize>,
    pub min: usize,
    pub max: usize,
}

#[derive(Clone, Default)]
pub struct Dialog {
    pub title: String,
    pub text: String,
    pub edit: Option<Edit>,
    pub list: Option<List>,
    pub buttons: Vec<String>,
    pub default: usize,
    pub cancel: Option<usize>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Outcome {
    pub button: Option<usize>,
    pub text: String,
    pub selected: Vec<usize>,
}

pub const DIALOG_CLASS: &str = "TomBoxGdkDialog";
const ID_EDIT: usize = 90;
const ID_LIST: usize = 91;
const ID_BUTTON: usize = 100;

struct State {
    spec: Dialog,
    out: Outcome,
    done: bool,
    edit: Hwnd,
    list: Hwnd,
    buttons: Vec<Hwnd>,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

unsafe fn window_text(h: Hwnd) -> String {
    if h.is_null() {
        return String::new();
    }
    let n = GetWindowTextLengthW(h).max(0) as usize;
    let mut buf = vec![0u16; n + 1];
    let got = GetWindowTextW(h, buf.as_mut_ptr(), buf.len() as i32).max(0) as usize;
    String::from_utf16_lossy(&buf[..got.min(n)])
}

unsafe fn list_selection(list: Hwnd, multi: bool) -> Vec<usize> {
    if list.is_null() {
        return Vec::new();
    }
    if multi {
        let n = SendMessageW(list, 0x0190, 0, 0).max(0) as usize;
        let mut v = vec![0i32; n];
        let got = SendMessageW(list, 0x0191, n, v.as_mut_ptr() as isize).max(0) as usize;
        v.truncate(got);
        v.into_iter().map(|x| x as usize).collect()
    } else {
        let i = SendMessageW(list, 0x0188, 0, 0);
        if i < 0 { Vec::new() } else { vec![i as usize] }
    }
}

unsafe fn selection_ok(s: &State) -> bool {
    match &s.spec.list {
        Some(l) => {
            let n = list_selection(s.list, l.multi).len();
            n >= l.min && (l.max == 0 || n <= l.max)
        }
        None => true,
    }
}

unsafe fn refresh_buttons(s: &State) {
    let ok = selection_ok(s);
    for (i, b) in s.buttons.iter().enumerate() {
        if Some(i) != s.spec.cancel {
            EnableWindow(*b, ok as i32);
        }
    }
}

unsafe fn finish(h: Hwnd, button: Option<usize>) {
    let close = STATE.with(|st| {
        let mut st = st.borrow_mut();
        let Some(s) = st.as_mut() else { return false };
        if s.done {
            return false;
        }
        if let Some(b) = button {
            if Some(b) != s.spec.cancel && !selection_ok(s) {
                return false;
            }
        }
        s.out.button = button;
        s.out.text = window_text(s.edit);
        let multi = s.spec.list.as_ref().map(|l| l.multi).unwrap_or(false);
        s.out.selected = list_selection(s.list, multi);
        s.done = true;
        true
    });
    if close {
        DestroyWindow(h);
    }
}

unsafe extern "system" fn dialog_proc(h: Hwnd, m: u32, w: usize, l: isize) -> isize {
    match m {
        0x0111 => {
            let id = w & 0xffff;
            let code = (w >> 16) & 0xffff;
            let (def, cancel, count) = STATE.with(|st| st.borrow().as_ref().map(|s| (s.spec.default, s.spec.cancel, s.spec.buttons.len())).unwrap_or((0, None, 0)));
            if id == ID_LIST {
                if code == 1 {
                    STATE.with(|st| {
                        if let Some(s) = st.borrow().as_ref() {
                            refresh_buttons(s);
                        }
                    });
                } else if code == 2 {
                    finish(h, Some(def));
                }
                return 0;
            }
            if id == 1 {
                finish(h, Some(def));
            } else if id == 2 {
                finish(h, cancel);
            } else if id >= ID_BUTTON && id < ID_BUTTON + count {
                finish(h, Some(id - ID_BUTTON));
            }
            0
        }
        0x0010 => {
            let cancel = STATE.with(|st| st.borrow().as_ref().and_then(|s| s.spec.cancel));
            finish(h, cancel);
            0
        }
        0x0002 => {
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(h, m, w, l),
    }
}

fn register_class() -> *const u16 {
    static CLASS: AtomicUsize = AtomicUsize::new(0);
    let c = CLASS.load(Ordering::Acquire);
    if c != 0 {
        return c as *const u16;
    }
    let name: &'static [u16] = Box::leak(wide(DIALOG_CLASS).into_boxed_slice());
    unsafe {
        let wc = WndClassExW {
            cb_size: core::mem::size_of::<WndClassExW>() as u32,
            style: 0,
            wnd_proc: dialog_proc as usize,
            cls_extra: 0,
            wnd_extra: 0,
            instance: GetModuleHandleW(core::ptr::null()),
            icon: core::ptr::null_mut(),
            cursor: core::ptr::null_mut(),
            background: 16usize as *mut c_void,
            menu_name: core::ptr::null(),
            class_name: name.as_ptr(),
            icon_sm: core::ptr::null_mut(),
        };
        RegisterClassExW(&wc);
    }
    CLASS.store(name.as_ptr() as usize, Ordering::Release);
    name.as_ptr()
}

unsafe fn child(parent: Hwnd, class: &str, text: &str, style: u32, ex: u32, r: (i32, i32, i32, i32), id: usize) -> Hwnd {
    let (c, t) = (wide(class), wide(text));
    let h = CreateWindowExW(ex, c.as_ptr(), t.as_ptr(), 0x5000_0000 | style, r.0, r.1, r.2, r.3, parent, id, GetModuleHandleW(core::ptr::null()), core::ptr::null_mut());
    SendMessageW(h, 0x0030, GetStockObject(17) as usize, 1);
    h
}

fn text_lines(s: &str, per_line: usize) -> i32 {
    s.split('\n').map(|l| (l.chars().count() / per_line.max(1)) as i32 + 1).sum::<i32>().clamp(1, 30)
}

pub fn show(spec: Dialog, hwnd_out: Option<Arc<AtomicUsize>>) -> Outcome {
    unsafe {
        let class = register_class();
        let width = 560;
        let pad = 14;
        let text_h = if spec.text.is_empty() { 0 } else { text_lines(&spec.text, 80) * 16 + 8 };
        let edit_h = match &spec.edit {
            Some(e) if e.multiline => 110,
            Some(_) => 26,
            None => 0,
        };
        let list_h = if spec.list.is_some() { 180 } else { 0 };
        let buttons_h = 34;
        let mut y = pad;
        let client_h = pad + text_h + if edit_h > 0 { edit_h + 10 } else { 0 } + if list_h > 0 { list_h + 10 } else { 0 } + buttons_h + pad;
        let total_h = client_h + GetSystemMetrics(4) + 2 * GetSystemMetrics(33);
        let total_w = width + 2 * GetSystemMetrics(32);
        let (sw, sh) = (GetSystemMetrics(0), GetSystemMetrics(1));
        let title = wide(if spec.title.is_empty() { "Xbox" } else { &spec.title });
        let h = CreateWindowExW(0x0000_0008 | 0x0001_0000, class, title.as_ptr(), 0x00C8_0000, (sw - total_w) / 2, (sh - total_h) / 2, total_w, total_h, core::ptr::null_mut(), 0, GetModuleHandleW(core::ptr::null()), core::ptr::null_mut());
        if h.is_null() {
            return Outcome { button: spec.cancel, ..Default::default() };
        }
        let mut state = State { spec: spec.clone(), out: Outcome::default(), done: false, edit: core::ptr::null_mut(), list: core::ptr::null_mut(), buttons: Vec::new() };
        if text_h > 0 {
            child(h, "STATIC", &spec.text.replace('\n', "\r\n"), 0, 0, (pad, y, width - 2 * pad, text_h), 0);
            y += text_h;
        }
        if let Some(e) = &spec.edit {
            let mut style = 0x0001_0000 | 0x0080 | 0x0080_0000;
            if e.password {
                style |= 0x0020;
            }
            if e.numeric {
                style |= 0x2000;
            }
            if e.multiline {
                style |= 0x0004 | 0x1000 | 0x0020_0000 | 0x0040;
            }
            state.edit = child(h, "EDIT", &e.default.replace('\n', "\r\n"), style, 0x200, (pad, y, width - 2 * pad, edit_h), ID_EDIT);
            if e.max > 0 {
                SendMessageW(state.edit, 0x00C5, e.max as usize, 0);
            }
            SendMessageW(state.edit, 0x00B1, 0, -1);
            y += edit_h + 10;
        }
        if let Some(l) = &spec.list {
            let style = 0x0001_0000 | 0x0020_0000 | 0x0001 | if l.multi { 0x0800 } else { 0 };
            state.list = child(h, "LISTBOX", "", style, 0x200, (pad, y, width - 2 * pad, list_h), ID_LIST);
            for it in &l.items {
                let w = wide(it);
                SendMessageW(state.list, 0x0180, 0, w.as_ptr() as isize);
            }
            for &p in &l.preselected {
                if l.multi {
                    SendMessageW(state.list, 0x0185, 1, p as isize);
                } else {
                    SendMessageW(state.list, 0x0186, p, 0);
                }
            }
            y += list_h + 10;
        }
        let bw = 120;
        let n = spec.buttons.len().max(1) as i32;
        let mut bx = width - pad - n * (bw + 8) + 8;
        for (i, b) in spec.buttons.iter().enumerate() {
            let style = 0x0001_0000 | if i == spec.default { 0x0001 } else { 0 };
            state.buttons.push(child(h, "BUTTON", b, style, 0, (bx, y, bw, 28), ID_BUTTON + i));
            bx += bw + 8;
        }
        STATE.with(|st| *st.borrow_mut() = Some(state));
        STATE.with(|st| {
            if let Some(s) = st.borrow().as_ref() {
                refresh_buttons(s);
            }
        });
        if let Some(o) = &hwnd_out {
            o.store(h as usize, Ordering::Release);
        }
        if crate::headless::enabled() {
            crate::headless::cloak(h as *mut c_void);
            ShowWindow(h, crate::headless::SW_SHOWNA);
        } else {
            ShowWindow(h, 5);
            SetForegroundWindow(h);
        }
        let focus = STATE.with(|st| st.borrow().as_ref().map(|s| if !s.edit.is_null() { s.edit } else if !s.list.is_null() { s.list } else { s.buttons.get(s.spec.default).copied().unwrap_or(core::ptr::null_mut()) }));
        if let Some(f) = focus {
            if !f.is_null() {
                SetFocus(f);
            }
        }
        let mut msg: Msg = core::mem::zeroed();
        while GetMessageW(&mut msg, core::ptr::null_mut(), 0, 0) > 0 {
            if IsDialogMessageW(h, &mut msg) == 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        if let Some(o) = &hwnd_out {
            o.store(0, Ordering::Release);
        }
        STATE.with(|st| st.borrow_mut().take()).map(|s| s.out).unwrap_or_default()
    }
}

pub fn close_window(h: usize) {
    if h != 0 {
        unsafe {
            PostMessageW(h as Hwnd, 0x0010, 0, 0);
        }
    }
}

pub fn find_dialog() -> Option<usize> {
    let c = wide(DIALOG_CLASS);
    let h = unsafe { FindWindowExW(core::ptr::null_mut(), core::ptr::null_mut(), c.as_ptr(), core::ptr::null()) };
    if h.is_null() { None } else { Some(h as usize) }
}

pub fn press(h: usize, id: usize) {
    unsafe {
        PostMessageW(h as Hwnd, 0x0111, id, 0);
    }
}

pub fn set_child_text(h: usize, id: usize, text: &str) {
    unsafe {
        let c = GetDlgItem(h as Hwnd, id as i32);
        if !c.is_null() {
            let w = wide(text);
            SendMessageW(c, 0x000C, 0, w.as_ptr() as isize);
        }
    }
}

pub const EDIT_ID: usize = ID_EDIT;
pub const LIST_ID: usize = ID_LIST;
pub const BUTTON_ID: usize = ID_BUTTON;

pub fn select_list(h: usize, idx: usize, multi: bool) {
    unsafe {
        let c = GetDlgItem(h as Hwnd, ID_LIST as i32);
        if !c.is_null() {
            if multi {
                SendMessageW(c, 0x0185, 1, idx as isize);
            } else {
                SendMessageW(c, 0x0186, idx, 0);
            }
            PostMessageW(h as Hwnd, 0x0111, ID_LIST | (1 << 16), c as isize);
        }
    }
}

pub fn spawn_toast(title: String, text: String, ms: u32) {
    std::thread::spawn(move || {
        let spec = Dialog { title, text, buttons: Vec::new(), ..Default::default() };
        let out = Arc::new(AtomicUsize::new(0));
        let o2 = out.clone();
        std::thread::spawn(move || {
            let start = std::time::Instant::now();
            loop {
                std::thread::sleep(std::time::Duration::from_millis(50));
                let h = o2.load(Ordering::Acquire);
                if h != 0 && start.elapsed().as_millis() as u32 >= ms {
                    close_window(h);
                    break;
                }
                if start.elapsed().as_secs() > 60 {
                    break;
                }
            }
        });
        show(spec, Some(out));
    });
}

struct Found {
    pid: u32,
    best: usize,
    area: i64,
}

unsafe extern "system" fn enum_proc(h: Hwnd, l: isize) -> i32 {
    let f = &mut *(l as *mut Found);
    let mut pid = 0u32;
    GetWindowThreadProcessId(h, &mut pid);
    if pid == f.pid && IsWindowVisible(h) != 0 && GetWindow(h, 4).is_null() {
        let mut r = Rect::default();
        GetWindowRect(h, &mut r);
        let a = (r.r - r.l) as i64 * (r.b - r.t) as i64;
        let mut cls = [0u16; 64];
        let n = GetClassNameW(h, cls.as_mut_ptr(), 64).max(0) as usize;
        let is_ours = String::from_utf16_lossy(&cls[..n]) == DIALOG_CLASS;
        if a > f.area && !is_ours {
            f.area = a;
            f.best = h as usize;
        }
    }
    1
}

#[link(name = "user32")]
extern "system" {
    fn GetClassNameW(h: Hwnd, buf: *mut u16, n: i32) -> i32;
}

pub fn main_window() -> Option<usize> {
    let mut f = Found { pid: unsafe { GetCurrentProcessId() }, best: 0, area: 0 };
    unsafe {
        EnumWindows(enum_proc, &mut f as *mut Found as isize);
    }
    if f.best == 0 { None } else { Some(f.best) }
}

pub struct Image {
    pub w: u32,
    pub h: u32,
    pub rgb: Vec<u8>,
}

pub fn capture(window: Option<usize>) -> Option<Image> {
    unsafe {
        let hwnd = window.unwrap_or(0) as Hwnd;
        let (w, h) = if hwnd.is_null() {
            (GetSystemMetrics(0), GetSystemMetrics(1))
        } else {
            let mut r = Rect::default();
            GetClientRect(hwnd, &mut r);
            (r.r - r.l, r.b - r.t)
        };
        if w <= 0 || h <= 0 {
            return None;
        }
        let src = GetDC(hwnd);
        let mem = CreateCompatibleDC(src);
        let bmi = BitmapInfoHeader { size: 40, width: w, height: -h, planes: 1, bit_count: 32, compression: 0, size_image: 0, xppm: 0, yppm: 0, clr_used: 0, clr_important: 0 };
        let mut bits: *mut u8 = core::ptr::null_mut();
        let bmp = CreateDIBSection(src, &bmi, 0, &mut bits, core::ptr::null_mut(), 0);
        if bmp.is_null() || bits.is_null() {
            DeleteDC(mem);
            ReleaseDC(hwnd, src);
            return None;
        }
        let old = SelectObject(mem, bmp);
        let ok = (!hwnd.is_null() && PrintWindow(hwnd, mem, 3) != 0) || BitBlt(mem, 0, 0, w, h, src, 0, 0, 0x00CC_0020) != 0;
        let mut rgb = Vec::with_capacity((w * h * 3) as usize);
        if ok {
            let px = core::slice::from_raw_parts(bits, (w * h * 4) as usize);
            for p in px.chunks(4) {
                rgb.extend_from_slice(&[p[2], p[1], p[0]]);
            }
        }
        SelectObject(mem, old);
        DeleteObject(bmp);
        DeleteDC(mem);
        ReleaseDC(hwnd, src);
        if ok { Some(Image { w: w as u32, h: h as u32, rgb }) } else { None }
    }
}
