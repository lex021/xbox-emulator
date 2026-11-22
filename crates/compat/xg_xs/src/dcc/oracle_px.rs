use super::px;
use super::Dcc;
use crate::dcc_oracle::{configs, diff, holes, Con, Rng};
use crate::oracle::{oracle, SERIAL};

type Get = unsafe extern "system" fn(*const u8, *mut u32, u32, u32, u8) -> *mut u32;
type Set = unsafe extern "system" fn(*const u8, *const u32, u32, u8) -> u32;
type Nb = unsafe extern "system" fn(*const u8, u32, u32) -> u32;
type Lay = unsafe extern "system" fn(*mut u8, *mut u8, *const u8, u32);
type Conv = unsafe extern "system" fn(*mut u8, *mut u8, *const u8, *const u32, *const u32);

pub(crate) fn px_states() -> Vec<Dcc> {
    let mut seen = std::collections::HashSet::new();
    let mut v = Vec::new();
    for (f, sw, dim, p0, p1) in configs() {
        let mut d = Dcc::new();
        if d.init(f, sw, dim, p0, p1).is_err() {
            continue;
        }
        let mut bpes = vec![d.r32(4)];
        if d.r32(4) == 0x10 {
            bpes.push(8);
        }
        for b in bpes {
            d.w32(4, b);
            let k = (d.r32(0), d.r32(4), d.r32(8), d.r32(0x14), d.r32(0x48));
            if seen.insert(k) {
                let mut e = Dcc::new();
                e.m.copy_from_slice(&d.m);
                v.push(e);
            }
        }
    }
    v
}

#[test]
fn oracle_dcc_px_helpers() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let Some(o) = oracle() else { return };
    if !crate::dcc_oracle::rvas_22000(&o) {
        println!("oracle_dcc_px_helpers: skipped, its console RVAs are the 22000 build's");
        return;
    }
    type F1 = unsafe extern "system" fn(u32) -> u32;
    type F3 = unsafe extern "system" fn(u32, i32, i32) -> u32;
    type F4 = unsafe extern "system" fn(u32, u32, i32, i32) -> u32;
    type Pl = unsafe extern "system" fn(*const u8, *const u8) -> u32;
    type Sz = unsafe extern "system" fn(*const u8) -> u8;
    let lg: F1 = unsafe { core::mem::transmute(o.h + 0x31aa0) };
    let bt: F3 = unsafe { core::mem::transmute(o.h + 0x23110) };
    let ct: F3 = unsafe { core::mem::transmute(o.h + 0x23150) };
    let ins: F4 = unsafe { core::mem::transmute(o.h + 0x26160) };
    let pl: Pl = unsafe { core::mem::transmute(o.h + 0x313a0) };
    let sz: Sz = unsafe { core::mem::transmute(o.h + 0x370c0) };
    let mut r = Rng(0x1234_5678_9abc);
    let mut cases = 0u64;
    let mut bad = 0u64;
    let mut ns: Vec<u32> = (0..300).collect();
    ns.extend([0x7fff_ffff, 0x8000_0000, 0x8000_0001, 0xffff_fffe, 0xffff_ffff]);
    for &n in &ns {
        cases += 1;
        if unsafe { lg(n) } != px::log2(n) {
            bad += 1;
        }
    }
    for _ in 0..200000 {
        let v = r.u() ^ (r.u() << 16);
        let w = r.u() ^ (r.u() << 16);
        let hi = (r.below(40) as i32) - 4;
        let lo = (r.below(40) as i32) - 4;
        cases += 3;
        if unsafe { bt(v, hi, lo) } != px::bits(v, hi, lo) {
            bad += 1;
        }
        if unsafe { ct(v, hi, lo) } != px::cut(v, hi, lo) {
            bad += 1;
            if bad < 10 {
                println!("cut {:#x} {} {}: {:#x} {:#x}", v, hi, lo, unsafe { ct(v, hi, lo) }, px::cut(v, hi, lo));
            }
        }
        if unsafe { ins(v, w, hi, lo) } != px::insert(v, w, hi, lo) {
            bad += 1;
            if bad < 10 {
                println!("ins {:#x} {:#x} {} {}", v, w, hi, lo);
            }
        }
    }
    let mut d = Dcc::new();
    for v48 in 0..20u32 {
        for c in 0..6u32 {
            for s in [0u32, 0xe, 0xf, 0x10, 0x12, 0x13, 0x14, 0xffff_ffff] {
                d.w32(0x48, v48);
                d.w32(8, c);
                d.w32(0, s);
                cases += 2;
                if unsafe { pl(d.m.as_ptr(), d.m.as_ptr().add(0x20)) } != px::planes(&d) {
                    bad += 1;
                }
                if (unsafe { sz(d.m.as_ptr()) } & 1 != 0) != px::is_swz(&d) {
                    bad += 1;
                }
            }
        }
    }
    println!("dcc px helpers: {} cases, {} mismatches", cases, bad);
    assert_eq!(bad, 0);
}

#[test]
fn oracle_dcc_px_get_set() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let Some(o) = oracle() else { return };
    if !crate::dcc_oracle::rvas_22000(&o) {
        println!("oracle_dcc_px_get_set: skipped, its console RVAs are the 22000 build's");
        return;
    }
    let cget: Get = unsafe { core::mem::transmute(o.h + 0x3fc60) };
    let cset: Set = unsafe { core::mem::transmute(o.h + 0x45860) };
    let cget_swz: Get = unsafe { core::mem::transmute(o.h + 0x370e0) };
    let cset_swz: Set = unsafe { core::mem::transmute(o.h + 0x3bcc0) };
    let cnb: Nb = unsafe { core::mem::transmute(o.h + 0x2d640) };
    let states = px_states();
    let mut r = Rng(0x9e37_79b9_7f4a_7c15);
    let (mut gc, mut gb, mut sc, mut sb, mut nc, mut nb, mut zc, mut swc) = (0u64, 0u64, 0u64, 0u64, 0u64, 0u64, 0u64, 0u64);
    for d in &states {
        for mode in 0..16u32 {
            for addr in 0..256u32 {
                let n = px::get(d, addr, mode);
                let Ok(n) = n else {
                    zc += 1;
                    continue;
                };
                let mut c = [0xdeadu32; 5];
                unsafe { cget(d.m.as_ptr(), c.as_mut_ptr(), addr, mode, 0) };
                gc += 1;
                if c != n {
                    gb += 1;
                    if gb < 10 {
                        println!("get [0]={:#x} bpe={} c={} dim={} v48={} mode={} addr={:#x}: native {:x?} console {:x?}", d.r32(0), d.r32(4), d.r32(8), d.r32(0x14), d.r32(0x48), mode, addr, n, c);
                    }
                }
                if px::is_swz(d) {
                    let mut c2 = [0xdeadu32; 5];
                    unsafe { cget_swz(d.m.as_ptr(), c2.as_mut_ptr(), addr, mode, 0) };
                    swc += 1;
                    if Ok(c2) != px::get_swz(d, addr, mode) {
                        gb += 1;
                    }
                }
                let mut cs = vec![n];
                let mut rc = [r.below(64), r.below(64), r.below(16), r.below(32), 0];
                if r.below(4) == 0 {
                    rc = [r.u(), r.u(), r.u(), r.u(), r.u()];
                }
                cs.push(rc);
                for co in cs {
                    let Ok(a) = px::set(d, &co, mode) else {
                        zc += 1;
                        continue;
                    };
                    sc += 1;
                    let b = unsafe { cset(d.m.as_ptr(), co.as_ptr(), mode, 0) };
                    if a != b {
                        sb += 1;
                        if sb < 10 {
                            println!("set [0]={:#x} bpe={} mode={} c={:x?}: native {:#x} console {:#x}", d.r32(0), d.r32(4), mode, co, a, b);
                        }
                    }
                    if px::is_swz(d) {
                        swc += 1;
                        if Ok(unsafe { cset_swz(d.m.as_ptr(), co.as_ptr(), mode, 0) }) != px::set_swz(d, &co, mode) {
                            sb += 1;
                        }
                    }
                }
                if let Ok(v) = px::neighbor(d, addr, mode) {
                    nc += 1;
                    if v != unsafe { cnb(d.m.as_ptr(), addr, mode) } {
                        nb += 1;
                    }
                }
            }
        }
    }
    println!("dcc px: {} states; get {} cases {} mismatches; set {} cases {} mismatches; swz direct {}; neighbor {} cases {} mismatches; {} div-zero skips", states.len(), gc, gb, sc, sb, swc, nc, nb, zc);
    assert_eq!(gb + sb + nb, 0);
}

#[test]
fn oracle_dcc_px_relayout() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let Some(o) = oracle() else { return };
    if !crate::dcc_oracle::rvas_22000(&o) {
        println!("oracle_dcc_px_relayout: skipped, its console RVAs are the 22000 build's");
        return;
    }
    let lay: Lay = unsafe { core::mem::transmute(o.h + 0x4ac90) };
    let conv: Conv = unsafe { core::mem::transmute(o.h + 0x4adc0) };
    let hl = holes(&o);
    let mut r = Rng(77);
    let (mut cases, mut bad, mut nonbij) = (0u64, 0u64, 0u64);
    let modes = [0u32, 1, 2, 3, 4, 8, 9, 10];
    let mut cfgs = configs();
    cfgs.retain(|_| r.below(6) == 0);
    for (f, sw, dim, p0, p1) in cfgs {
        let mut n = Dcc::new();
        if n.init(f, sw, dim, p0, p1).is_err() {
            continue;
        }
        let Some(c) = Con::new(&o, f, sw, dim, p0, p1) else { continue };
        if n.r32(4) == 0x10 && r.below(2) == 0 {
            n.w32(4, 8);
        }
        for k in 0..3 {
            for j in 0..256 {
                n.b[k][j] = r.u() as u8;
            }
        }
        n.w32(0x5c, r.below(16));
        let from = modes[r.below(8) as usize];
        let to = modes[r.below(8) as usize];
        n.w32(0xf0, from);
        for (dst, src) in [(2usize, 2usize), (0, 2)] {
            let mut ok = true;
            for k in 0..4u32 {
                if (n.r32(0x5c) >> k) & 1 == 0 {
                    continue;
                }
                let mut seen = [false; 64];
                for a in k * 64..k * 64 + 64 {
                    let Ok(cc) = px::get(&n, a, from) else { ok = false; break };
                    let Ok(b) = px::set(&n, &cc, to) else { ok = false; break };
                    seen[(b & 63) as usize] = true;
                }
                if seen.iter().any(|&x| !x) {
                    ok = false;
                }
            }
            if !ok {
                nonbij += 1;
                continue;
            }
            c.push(&n);
            let mut nn = c.pull();
            px::relayout(&mut nn, dst, src, to).unwrap();
            unsafe { lay(c.p(), c.buf(dst).as_mut_ptr(), c.buf(src).as_ptr(), to) };
            cases += 1;
            if let Some(m) = diff(&nn, &c.pull(), &hl, true) {
                bad += 1;
                if bad < 10 {
                    println!("relayout f={} sw={} from={} to={}: {}", f, sw, from, to, m);
                }
            }
        }
        let gm = [modes[r.below(8) as usize], modes[r.below(8) as usize], 3, modes[r.below(8) as usize]];
        let sm = [3u32, modes[r.below(8) as usize], 3, 3];
        let mut ok = true;
        for k in 0..4u32 {
            if (n.r32(0x5c) >> k) & 1 == 0 || gm[k as usize] == sm[k as usize] {
                continue;
            }
            let mut seen = [false; 64];
            for a in k * 64..k * 64 + 64 {
                let Ok(cc) = px::get(&n, a, gm[k as usize]) else { ok = false; break };
                let Ok(b) = px::set(&n, &cc, sm[k as usize]) else { ok = false; break };
                seen[(b & 63) as usize] = true;
            }
            if seen.iter().any(|&x| !x) {
                ok = false;
            }
        }
        if !ok {
            nonbij += 1;
            continue;
        }
        c.push(&n);
        let mut nn = c.pull();
        px::convert(&mut nn, 2, 1, &gm, &sm).unwrap();
        unsafe { conv(c.p(), c.buf(2).as_mut_ptr(), c.buf(1).as_ptr(), gm.as_ptr(), sm.as_ptr()) };
        cases += 1;
        if let Some(m) = diff(&nn, &c.pull(), &hl, true) {
            bad += 1;
            if bad < 10 {
                println!("convert f={} sw={} gm={:?} sm={:?}: {}", f, sw, gm, sm, m);
            }
        }
    }
    println!("dcc px relayout/convert: {} cases, {} mismatches, {} non-bijective skips", cases, bad, nonbij);
    assert_eq!(bad, 0);
}
