pub const TB_GLOBAL_SPACE: u32 = 4_000_003;
pub const TB_GLOBAL_WINDOWS: usize = 2;
pub const TB_GLOBAL_CBV: u32 = 0;
pub const TB_GLOBAL_FAULT_UAV: u32 = TB_GLOBAL_WINDOWS as u32;
pub const TB_GLOBAL_CB_DWORDS: usize = 4 * TB_GLOBAL_WINDOWS + 4;
pub const TB_GLOBAL_FAULT_BYTES: usize = 16;
pub const TB_GLOBAL_ROOT_VAS: usize = 32;
pub const TB_GLOBAL_ROOT_DWORDS: u32 = 2 * (TB_GLOBAL_WINDOWS as u32 + 2);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Window {
    pub base: u64,
    pub size: u32,
}

pub fn window_cb(win: &[Window], ms_in: [u32; 2]) -> [u32; TB_GLOBAL_CB_DWORDS] {
    let mut c = [0u32; TB_GLOBAL_CB_DWORDS];
    for (i, w) in win.iter().take(TB_GLOBAL_WINDOWS).enumerate() {
        c[4 * i] = w.base as u32;
        c[4 * i + 1] = (w.base >> 32) as u32;
        c[4 * i + 2] = w.size;
    }
    c[4 * TB_GLOBAL_WINDOWS] = ms_in[0];
    c[4 * TB_GLOBAL_WINDOWS + 1] = ms_in[1];
    c
}

pub fn resolve(win: &[Window], va: u64) -> Option<(usize, u32)> {
    win.iter().take(TB_GLOBAL_WINDOWS).enumerate().find_map(|(i, w)| {
        let off = va.wrapping_sub(w.base);
        (w.size != 0 && va >= w.base && off < w.size as u64).then_some((i, off as u32))
    })
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Fault {
    pub count: u32,
    pub first_va: u64,
}

pub fn fault_of(rec: &[u32; 4]) -> Fault {
    Fault { count: rec[0], first_va: rec[1] as u64 | (rec[2] as u64) << 32 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_table_and_resolution() {
        let w = [Window { base: 0x10_0000_0000, size: 0x100 }, Window { base: 0x7f10_0000_0000, size: 64 }];
        let c = window_cb(&w, [3, 0x1234]);
        assert_eq!(&c[..8], &[0, 0x10, 0x100, 0, 0, 0x7f10, 64, 0]);
        assert_eq!(&c[8..10], &[3, 0x1234]);
        assert_eq!(resolve(&w, 0x10_0000_00fc), Some((0, 0xfc)));
        assert_eq!(resolve(&w, 0x10_0000_0100), None);
        assert_eq!(resolve(&w, 0x7f10_0000_0010), Some((1, 0x10)));
        assert_eq!(resolve(&w, 0x0f_ffff_fffc), None);
        assert_eq!(fault_of(&[2, 0x40, 0x7f, 0]), Fault { count: 2, first_va: 0x7f_0000_0040 });
        assert_eq!(TB_GLOBAL_ROOT_DWORDS, 8);
    }
}
