use std::sync::OnceLock;

pub const KIND_KBM: u32 = 0x0000_0030;

static PAD: OnceLock<bool> = OnceLock::new();
static KBM: OnceLock<bool> = OnceLock::new();

pub fn hide_choice(v: Option<&str>) -> bool {
    matches!(v.map(str::trim), Some("0") | Some("off") | Some("false"))
}

fn read(cell: &OnceLock<bool>, var: &str, what: &str) -> bool {
    *cell.get_or_init(|| {
        let h = hide_choice(std::env::var(var).ok().as_deref());
        if h {
            crate::record_raw("host input", var, &format!("{var}=0: {what}"));
        }
        h
    })
}

pub fn pad_hidden() -> bool {
    read(&PAD, "TOMBOX_HOST_PAD", "host gamepads are hidden from the title (no host gamepad device callbacks, host gamepad readings dropped); only the virtual pad is visible")
}

pub fn kbm_hidden() -> bool {
    read(&KBM, "TOMBOX_HOST_KBM", "host keyboard and mouse are hidden from the title (GameInput keyboard/mouse readings and device callbacks dropped, Win32 keyboard/mouse window messages dropped, overlay hotkey ignored)")
}

pub fn kbm_message(msg: u32) -> bool {
    msg == 0x00FF || (0x0100..=0x0109).contains(&msg) || (0x0200..=0x020E).contains(&msg) || (0x02A0..=0x02A3).contains(&msg)
}

pub fn hide_reading(kind: u32, pad_hidden: bool, kbm_hidden: bool) -> bool {
    (pad_hidden && kind & crate::gamepad::KIND_GAMEPAD != 0) || (kbm_hidden && kind & KIND_KBM != 0)
}

pub fn hide_device(supported: u32, pad_hidden: bool, kbm_hidden: bool) -> bool {
    hide_reading(supported, pad_hidden, kbm_hidden)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_off_false_hide() {
        assert!(hide_choice(Some("0")));
        assert!(hide_choice(Some(" off ")));
        assert!(hide_choice(Some("false")));
        assert!(!hide_choice(Some("1")));
        assert!(!hide_choice(None));
    }

    #[test]
    fn keyboard_and_mouse_messages_are_input() {
        for m in [0x00FF, 0x0100, 0x0101, 0x0102, 0x0104, 0x0105, 0x0109, 0x0200, 0x0201, 0x0202, 0x020A, 0x020E, 0x02A1, 0x02A3] {
            assert!(kbm_message(m), "{m:#x}");
        }
        for m in [0x0012, 0x000F, 0x0005, 0x0006, 0x0007, 0x0008, 0x001C, 0x00A0, 0x00A1, 0x0112, 0x0113, 0x0020, 0x8000] {
            assert!(!kbm_message(m), "{m:#x}");
        }
    }

    #[test]
    fn readings_hidden_by_switch() {
        let pad = crate::gamepad::KIND_GAMEPAD;
        assert!(hide_reading(pad, true, false));
        assert!(!hide_reading(pad, false, true));
        assert!(hide_reading(0x10, false, true));
        assert!(hide_reading(0x20, false, true));
        assert!(hide_reading(0x30, false, true));
        assert!(!hide_reading(0x30, true, false));
        assert!(!hide_reading(0x10, false, false));
        assert!(hide_device(0x30, false, true));
        assert!(!hide_device(pad | 0xE, false, true));
    }
}
