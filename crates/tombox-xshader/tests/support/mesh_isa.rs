#![allow(dead_code)]

pub const S_ENDPGM: u32 = 0xbf81_0000;
pub const LIT: u32 = 0xff;
pub const EXEC: u32 = 126;
pub const M0: u32 = 124;
pub const F_NEG1: u32 = 243;
pub const F_ONE: u32 = 242;

pub fn v(n: u32) -> u32 {
    256 + n
}

pub fn ic(n: i32) -> u32 {
    if n >= 0 { 128 + n as u32 } else { 192 + (-n) as u32 }
}

pub fn vop1(op: u32, vdst: u32, src0: u32) -> u32 {
    0x7e00_0000 | (vdst << 17) | (op << 9) | src0
}

pub fn vop2(op: u32, vdst: u32, src0: u32, vsrc1: u32) -> u32 {
    (op << 25) | (vdst << 17) | (vsrc1 << 9) | src0
}

pub fn vopc(op: u32, src0: u32, vsrc1: u32) -> u32 {
    0x7c00_0000 | (op << 17) | (vsrc1 << 9) | src0
}

pub fn vop3(op: u32, vdst: u32, srcs: [u32; 3]) -> [u32; 2] {
    [0xd400_0000 | (op << 16) | vdst, srcs[0] | (srcs[1] << 9) | (srcs[2] << 18)]
}

pub fn sop1(op: u32, sdst: u32, s0: u32) -> u32 {
    0xbe80_0000 | (sdst << 16) | (op << 8) | s0
}

pub fn sop2(op: u32, sdst: u32, s0: u32, s1: u32) -> u32 {
    0x8000_0000 | (op << 23) | (sdst << 16) | (s1 << 8) | s0
}

pub fn sopp(op: u32, imm: u16) -> u32 {
    0xbf80_0000 | (op << 16) | imm as u32
}

pub fn exp(target: u32, en: u32, done: bool, s: [u32; 4]) -> [u32; 2] {
    [0xf800_0000 | ((done as u32) << 11) | (target << 4) | en, s[0] | (s[1] << 8) | (s[2] << 16) | (s[3] << 24)]
}

pub fn console_ms(nv: u32, np: u32, prim: u32) -> Vec<u32> {
    console_ms_b(nv, np, prim, false)
}

pub fn console_ms_b(nv: u32, np: u32, prim: u32, div_barrier: bool) -> Vec<u32> {
    let mut w = vec![];
    w.push(sop1(4, EXEC, ic(-1)));
    w.extend(vop3(0x365, 0, [ic(-1), ic(0), 0]));
    w.extend(vop3(0x366, 0, [ic(-1), v(0), 0]));
    w.extend([sop2(0x27, 3, 3, LIT), 0x40018]);
    w.extend(vop3(0x346, 0, [3, ic(6), v(0)]));
    w.push(vop1(2, 2, v(5)));
    w.push(sop1(4, 10, EXEC));
    w.push(vopc(0xd2, ic(0), 0));
    w.push(sopp(8, 3 + div_barrier as u16));
    w.extend([sop1(3, M0, LIT), (np << 12) | nv]);
    w.push(sopp(0x10, 9));
    if div_barrier {
        w.push(sopp(0x0a, 0));
    }
    w.push(sop1(4, EXEC, 10));
    w.push(sopp(0x0a, 0));
    w.extend([vop1(1, 2, LIT), 4.0f32.to_bits()]);
    w.extend([vopc(0xc2, LIT, 0), 65]);
    w.push(vop2(0x01, 1, F_NEG1, 2));
    w.extend([vopc(0xc2, LIT, 0), 66]);
    w.push(vop2(0x01, 3, F_NEG1, 2));
    w.push(vop1(1, 4, ic(0)));
    w.push(vop1(1, 6, F_ONE));
    w.push(vop1(1, 7, 2));
    w.push(vop1(1, 8, 3));
    w.extend([vop1(1, 9, LIT), prim]);
    w.extend(exp(12, 0xf, false, [1, 3, 4, 6]));
    w.extend(exp(20, 0x1, false, [9, 0, 0, 0]));
    w.extend(exp(32, 0x3, true, [7, 8, 0, 0]));
    w.push(S_ENDPGM);
    w
}

pub const TRI: u32 = 64 | (65 << 10) | (66 << 20);
