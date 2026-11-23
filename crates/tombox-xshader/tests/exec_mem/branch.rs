use super::ds::cs_prog;
use super::*;

const LANES: u32 = 64;

fn jump_over(padding: usize) -> CsCase {
    let skipped = 1 + padding;
    let k = (8 + 4 + 4 + 4 * skipped) as u32;
    let mut body = vec![vop1(1, 10, ic(7))];
    body.extend_from_slice(&[sop1(0x1f, 20, 0), sop2(0, 20, 20, LIT), k, sop2(4, 21, 21, ic(0)), sop1(0x20, 0, 20)]);
    body.push(vop1(1, 10, ic(63)));
    body.extend(std::iter::repeat(sopp(0, 0)).take(padding));
    body.push(vop1(1, 11, ic(5)));
    let mem = (0..LANES).flat_map(|t| [(t as usize, 7), (LANES as usize + t as usize, 5)]).collect();
    CsCase { m: "s_setpc_b64", op: vec![sop1(0x20, 0, 20)], words: cs_prog(&body, &[10, 11]), threads: LANES, mem, init: vec![] }
}

#[test]
fn pc_relative_jumps_are_direct_branches_near_and_beyond_the_16_bit_range() {
    let cases: Vec<CsCase> = [0usize, 3, 40, 40_000].into_iter().map(jump_over).collect();
    run_cs_cases("far_jump", &cases);
}

#[test]
fn a_jump_through_a_computed_register_is_still_refused() {
    let body = vec![sop1(0x1f, 20, 0), sop2(0, 20, 20, ic(8)), sop1(0x20, 0, 20)];
    let w = cs_prog(&body, &[10]);
    assert!(translate(&cs_extract(&w, LANES)).is_err(), "s_setpc_b64 without the getpc/add/addc pattern must not translate");
}
