use crate::hlsl::{f16_exact, fma16_operands_exact};
use crate::rdna2::Opnd;
use crate::shader_cache::Fma;

fn f16_to_f32(h: u16) -> f32 {
    let s = ((h as u32) & 0x8000) << 16;
    let e = ((h >> 10) & 0x1f) as u32;
    let m = (h & 0x3ff) as u32;
    if e == 0x1f {
        return f32::from_bits(s | 0x7f80_0000 | (m << 13));
    }
    if e == 0 {
        let v = m as f32 * f32::from_bits(0x3380_0000);
        return f32::from_bits(s | v.to_bits());
    }
    f32::from_bits(s | ((e + 112) << 23) | (m << 13))
}

fn f2h_soft(x: f32) -> u16 {
    let b = x.to_bits();
    let s = (b >> 16) & 0x8000;
    let a = b & 0x7fff_ffff;
    if a > 0x7f80_0000 {
        return (s | 0x7e00 | ((a >> 13) & 0x3ff)) as u16;
    }
    if a >= 0x477f_f000 {
        return (s | 0x7c00) as u16;
    }
    if a >= 0x3880_0000 {
        let r = a - 0x3800_0000;
        return (s | ((r + 0xfff + ((r >> 13) & 1)) >> 13)) as u16;
    }
    if a < 0x3300_0000 {
        return s as u16;
    }
    let e = a >> 23;
    let m = (a & 0x7f_ffff) | 0x80_0000;
    let sh = 126 - e;
    let mut q = m >> sh;
    let rem = m & ((1 << sh) - 1);
    let hf = 1 << (sh - 1);
    if rem > hf || (rem == hf && q & 1 != 0) {
        q += 1;
    }
    (s | q) as u16
}

fn fma16(a: f32, b: f32, c: f32) -> f32 {
    let p = a * b;
    let s = p + c;
    if s.to_bits() & 0x7fff_ffff >= 0x7f80_0000 {
        return s;
    }
    let bb = s - p;
    let e = (p - (s - bb)) + (c - bb);
    let mut u = s.to_bits();
    if e != 0.0 {
        if (e > 0.0) != (s > 0.0) {
            u -= 1;
        }
        u |= 1;
    }
    f32::from_bits(u)
}

fn scaled(h: u16) -> i128 {
    let e = ((h >> 10) & 0x1f) as i32;
    let m = (h & 0x3ff) as i128;
    let mag = if e == 0 { m } else { (m | 0x400) << (e - 1) };
    if h & 0x8000 != 0 { -mag } else { mag }
}

fn round_shift(m: i128, sh: u32) -> i128 {
    if sh == 0 {
        return m;
    }
    let q = m >> sh;
    let rem = m & ((1i128 << sh) - 1);
    let half = 1i128 << (sh - 1);
    if rem > half || (rem == half && q & 1 != 0) { q + 1 } else { q }
}

fn exact_f16(a: u16, b: u16, c: u16) -> u16 {
    let x = scaled(a) * scaled(b) + (scaled(c) << 24);
    if x == 0 {
        let pneg = ((a ^ b) & 0x8000) != 0;
        let pzero = scaled(a) == 0 || scaled(b) == 0;
        return if pzero && scaled(c) == 0 && pneg && c & 0x8000 != 0 { 0x8000 } else { 0 };
    }
    let sign = if x < 0 { 0x8000u16 } else { 0 };
    let m = x.abs();
    let top = 127 - m.leading_zeros() as i32;
    let bits = if top < 34 {
        round_shift(m, 24) as u32
    } else {
        let mut q = round_shift(m, (top - 10) as u32);
        let mut k = top - 48;
        if q >= 0x800 {
            q >>= 1;
            k += 1;
        }
        let e = k + 15;
        if e >= 31 { 0x7c00 } else { ((e as u32) << 10) | (q as u32 & 0x3ff) }
    };
    sign | bits as u16
}

fn xs(s: &mut u64) -> u64 {
    *s ^= *s << 13;
    *s ^= *s >> 7;
    *s ^= *s << 17;
    *s
}

fn finite(h: u16) -> bool {
    h & 0x7c00 != 0x7c00
}

#[test]
fn reference_decodes_f16() {
    for h in 0..=u16::MAX {
        if finite(h) {
            let v = f16_to_f32(h) as f64 * (1u64 << 24) as f64;
            assert_eq!(scaled(h) as f64, v, "{h:04x}");
            assert_eq!(f2h_soft(f16_to_f32(h)), h);
        }
    }
}

#[test]
fn fma16_rounds_like_an_exact_fma_then_f16() {
    let mut s = 0x1234_5678_9abc_def1u64;
    let special = [0x0000u16, 0x8000, 0x0001, 0x8001, 0x03ff, 0x0400, 0x3c00, 0xbc00, 0x7bff, 0xfbff, 0x3555, 0x1400, 0x2e66];
    let mut n = 0u64;
    for i in 0..4_000_000u64 {
        let r = xs(&mut s);
        let pick = |k: u64| -> u16 { if (k >> 20) % 5 == 0 { special[(k as usize) % special.len()] } else { k as u16 } };
        let (a, b) = (pick(r), pick(r >> 16));
        let mut c = pick(r >> 32);
        if i % 3 == 0 {
            let p = f16_to_f32(a) * f16_to_f32(b);
            c = f2h_soft(-p) ^ ((r >> 48) as u16 & 3);
        }
        if !finite(a) || !finite(b) || !finite(c) {
            continue;
        }
        let got = f2h_soft(fma16(f16_to_f32(a), f16_to_f32(b), f16_to_f32(c)));
        let want = exact_f16(a, b, c);
        assert_eq!(got, want, "a={a:04x} b={b:04x} c={c:04x}");
        n += 1;
    }
    assert!(n > 2_000_000);
}

#[test]
fn fma16_signed_zero_and_specials() {
    let z = f16_to_f32(0x0000);
    let nz = f16_to_f32(0x8000);
    assert_eq!(fma16(nz, z, nz).to_bits(), 0x8000_0000);
    assert_eq!(fma16(z, z, nz).to_bits(), 0);
    assert_eq!(fma16(1.0, 1.0, -1.0).to_bits(), 0);
    assert_eq!(fma16(f32::INFINITY, 1.0, 1.0), f32::INFINITY);
    assert!(fma16(f32::INFINITY, 0.0, 1.0).is_nan());
    assert!(fma16(f32::INFINITY, 1.0, f32::NEG_INFINITY).is_nan());
    assert!(fma16(f32::NAN, 1.0, 1.0).is_nan());
    assert_eq!(f2h_soft(fma16(65504.0, 1.0, 16.0)), 0x7c00);
    assert_eq!(f2h_soft(fma16(65504.0, 1.0, 15.0)), 0x7bff);
}

#[test]
fn f16_exactness_of_constants() {
    for v in [0.0f32, -0.0, 0.5, 1.0, 2.0, 4.0, -4.0, 65504.0, 6.103515625e-05, 5.960464477539063e-08, f32::INFINITY] {
        assert!(f16_exact(v), "{v}");
    }
    for v in [0.15915494f32, 65520.0, 2.98e-08, 1.0 + f32::EPSILON, f32::NAN, 1e-30] {
        assert!(!f16_exact(v), "{v}");
    }
    for h in 0..=u16::MAX {
        if finite(h) {
            assert!(f16_exact(f16_to_f32(h)));
        }
    }
    assert!(fma16_operands_exact(&[Opnd::ConstF32(0.5), Opnd::ConstF32(2.0)]));
    assert!(!fma16_operands_exact(&[Opnd::ConstF32(0.15915494), Opnd::ConstF32(2.0)]));
}

#[test]
fn defines_follow_the_verified_flags() {
    let f = Fma { preserve: true, any: false, hybrid: true, f2h: true };
    assert_eq!(crate::dxc::fma_defines(f, false), vec!["-D", "TB_FAST_FMA", "-D", "TB_FAST_F2H"]);
    assert_eq!(crate::dxc::fma_defines(f, true), vec!["-D", "TB_HYBRID_FMA", "-D", "TB_FAST_F2H"]);
    let g = Fma { any: true, ..f };
    assert_eq!(crate::dxc::fma_defines(g, true), vec!["-D", "TB_FAST_FMA", "-D", "TB_FAST_F2H"]);
    assert!(crate::dxc::fma_defines(Fma::default(), true).is_empty());
    let k = |f: Fma| crate::shader_cache::key("x", "cs_6_2", f, "");
    assert_ne!(k(Fma::default()), k(Fma { f2h: true, ..Fma::default() }));
    assert_ne!(k(Fma::default()), k(Fma { hybrid: true, ..Fma::default() }));
}

#[test]
fn translator_helpers_have_no_double_in_fast_paths() {
    let h = crate::hlsl::CUBE_HELPERS;
    let line = h.lines().find(|l| l.starts_with("float tb_fma16(")).expect("tb_fma16");
    assert!(!line.contains("double"));
    let src = line.replace("precise ", "");
    assert!(src.contains("p + c") && src.contains("u |= 1u"));
}
