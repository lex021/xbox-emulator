use core::ffi::c_void;

pub const WS_VISIBLE: u32 = 0x1000_0000;
pub const WS_CHILD: u32 = 0x4000_0000;
pub const WS_EX_TOOLWINDOW: u32 = 0x0000_0080;
pub const WS_EX_APPWINDOW: u32 = 0x0004_0000;
pub const WS_EX_NOACTIVATE: u32 = 0x0800_0000;
pub const SW_SHOWNA: i32 = 8;
const DWMWA_CLOAK: u32 = 13;
const DWMWA_CLOAKED: u32 = 14;

#[link(name = "dwmapi")]
extern "system" {
    fn DwmSetWindowAttribute(h: *mut c_void, attr: u32, v: *const c_void, n: u32) -> i32;
    fn DwmGetWindowAttribute(h: *mut c_void, attr: u32, v: *mut c_void, n: u32) -> i32;
}

pub fn flag(v: Option<&str>) -> bool {
    matches!(v.map(str::trim), Some("1") | Some("on") | Some("true") | Some("yes"))
}

pub fn enabled() -> bool {
    static H: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *H.get_or_init(|| flag(std::env::var("TOMBOX_HEADLESS").ok().as_deref()))
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct Styles {
    pub style: u32,
    pub ex: u32,
    pub show_after: bool,
}

pub fn top_level_styles(style: u32, ex: u32) -> Styles {
    Styles { style: style & !WS_VISIBLE, ex: (ex | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE) & !WS_EX_APPWINDOW, show_after: style & WS_VISIBLE != 0 }
}

pub fn applies(style: u32, parent_is_null: bool) -> bool {
    parent_is_null && style & WS_CHILD == 0
}

pub fn show_cmd(cmd: i32) -> i32 {
    match cmd {
        1 | 9 | 10 => 4,
        3 | 5 => 8,
        2 | 6 | 11 => 7,
        c => c,
    }
}

pub unsafe fn cloak(h: *mut c_void) -> i32 {
    if h.is_null() {
        return -1;
    }
    let on: i32 = 1;
    DwmSetWindowAttribute(h, DWMWA_CLOAK, &on as *const i32 as *const c_void, 4)
}

pub unsafe fn is_cloaked(h: *mut c_void) -> bool {
    let mut v: u32 = 0;
    DwmGetWindowAttribute(h, DWMWA_CLOAKED, &mut v as *mut u32 as *mut c_void, 4) >= 0 && v != 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn top_level_window_loses_visible_and_taskbar_and_activation() {
        let s = top_level_styles(0x10CF_0000, WS_EX_APPWINDOW | 0x100);
        assert_eq!(s.style, 0x00CF_0000);
        assert_eq!(s.ex, 0x100 | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE);
        assert!(s.show_after);
        assert!(!top_level_styles(0x00CF_0000, 0).show_after);
    }

    #[test]
    fn child_and_owned_windows_are_left_alone() {
        assert!(applies(0x00CF_0000, true));
        assert!(!applies(0x5000_0000, true));
        assert!(!applies(0x00CF_0000, false));
    }

    #[test]
    fn show_commands_never_activate() {
        assert_eq!(show_cmd(1), 4);
        assert_eq!(show_cmd(5), 8);
        assert_eq!(show_cmd(3), 8);
        assert_eq!(show_cmd(2), 7);
        assert_eq!(show_cmd(0), 0);
        assert_eq!(show_cmd(8), 8);
    }

    #[test]
    fn flag_values() {
        assert!(flag(Some("1")));
        assert!(flag(Some(" on ")));
        assert!(!flag(Some("0")));
        assert!(!flag(None));
    }

    #[test]
    fn cloak_rejects_null() {
        assert_eq!(unsafe { cloak(core::ptr::null_mut()) }, -1);
    }
}
