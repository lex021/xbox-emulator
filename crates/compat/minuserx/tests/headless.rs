use core::ffi::c_void;
use core::ptr::null_mut;

#[link(name = "user32")]
extern "system" {
    fn DestroyWindow(h: *mut c_void) -> i32;
    fn IsWindowVisible(h: *mut c_void) -> i32;
    fn GetWindowLongPtrW(h: *mut c_void, i: i32) -> isize;
    fn GetForegroundWindow() -> *mut c_void;
    fn WindowFromPoint(p: [i32; 2]) -> *mut c_void;
    fn GetAncestor(h: *mut c_void, f: u32) -> *mut c_void;
}

#[link(name = "dwmapi")]
extern "system" {
    fn DwmGetWindowAttribute(h: *mut c_void, attr: u32, v: *mut c_void, n: u32) -> i32;
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

#[test]
fn headless_style_transform() {
    let (s, e, show) = minuserx::headless_styles(0x10CF_0000, 0x0004_0100);
    assert_eq!(s, 0x00CF_0000);
    assert_eq!(e, 0x0800_0180);
    assert!(show);
    assert!(!minuserx::headless_styles(0x00CF_0000, 0).2);
    assert!(minuserx::headless_applies(0x00CF_0000, true));
    assert!(!minuserx::headless_applies(0x5000_0000, true));
    assert!(!minuserx::headless_applies(0x00CF_0000, false));
    assert_eq!(minuserx::headless_show_cmd(1), 4);
    assert_eq!(minuserx::headless_show_cmd(5), 8);
    assert_eq!(minuserx::headless_show_cmd(2), 7);
    assert_eq!(minuserx::headless_show_cmd(0), 0);
}

#[test]
fn headless_window_is_visible_to_the_title_but_cloaked_and_inactive() {
    std::env::set_var("TOMBOX_HEADLESS", "1");
    assert!(minuserx::headless());
    let cls = wide("STATIC");
    let name = wide("tombox headless test");
    let h = unsafe { minuserx::mux_CreateWindowExW(0x0004_0000, cls.as_ptr(), name.as_ptr(), 0x10CF_0000, 40, 40, 320, 200, null_mut(), null_mut(), null_mut(), null_mut()) };
    assert!(!h.is_null());
    unsafe {
        assert_ne!(IsWindowVisible(h), 0, "the title still sees a visible window");
        let mut cloaked = 0u32;
        assert!(DwmGetWindowAttribute(h, 14, &mut cloaked as *mut u32 as *mut c_void, 4) >= 0);
        assert_ne!(cloaked, 0, "DWM never composes it onto the desktop");
        let ex = GetWindowLongPtrW(h, -20) as u32;
        assert_eq!(ex & 0x0800_0080, 0x0800_0080, "tool window + no-activate");
        assert_eq!(ex & 0x0004_0000, 0, "no taskbar button");
        assert_ne!(GetForegroundWindow(), h);
        assert_eq!(minuserx::mux_ShowWindow(h, 5) != 0, true);
        assert_ne!(GetForegroundWindow(), h);
        let hit = WindowFromPoint([40 + 160, 40 + 100]);
        assert_ne!(GetAncestor(hit, 2), h, "a cloaked window is never hit by the user's mouse");
        DestroyWindow(h);
    }
}
