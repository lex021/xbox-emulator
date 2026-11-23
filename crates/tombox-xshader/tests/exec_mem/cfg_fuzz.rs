use super::*;

#[derive(Clone, Copy, Debug)]
enum Op {
    Movk(u32, i16),
    Addk(u32, i16),
    Add(u32, u32, u32),
    CmpLt(u32, i32),
    Br(u32, usize),
}

struct Gen {
    s: u64,
    ops: Vec<Op>,
    labels: Vec<usize>,
    fix: Vec<(usize, usize)>,
}

impl Gen {
    fn rnd(&mut self, n: u64) -> u64 {
        self.s ^= self.s << 13;
        self.s ^= self.s >> 7;
        self.s ^= self.s << 17;
        self.s % n
    }
    fn label(&mut self) -> usize {
        self.labels.push(usize::MAX);
        self.labels.len() - 1
    }
    fn place(&mut self, l: usize) {
        self.labels[l] = self.ops.len();
    }
    fn br(&mut self, cond: u32, l: usize) {
        self.fix.push((self.ops.len(), l));
        self.ops.push(Op::Br(cond, 0));
    }
    fn acc(&mut self) -> u32 {
        20 + self.rnd(4) as u32
    }
    fn cond(&mut self) {
        let r = self.acc();
        let k = self.rnd(40) as i32 - 8;
        self.ops.push(Op::CmpLt(r, k));
    }
    fn simple(&mut self) {
        match self.rnd(3) {
            0 => {
                let (r, k) = (self.acc(), self.rnd(17) as i16 - 8);
                self.ops.push(Op::Addk(r, k));
            }
            1 => {
                let (d, a) = (self.acc(), self.acc());
                self.ops.push(Op::Add(d, d, a));
            }
            _ => {
                let r = self.acc();
                let k = self.rnd(9) as i16 - 2;
                self.ops.push(Op::Movk(r, k));
            }
        }
    }
    fn seq(&mut self, depth: u32, exits: &[usize]) {
        let n = 2 + self.rnd(5);
        let mut pend: Vec<usize> = vec![];
        for _ in 0..n {
            if !pend.is_empty() && self.rnd(3) == 0 {
                let k = self.rnd(pend.len() as u64) as usize;
                let l = pend.remove(k);
                self.place(l);
            }
            match self.rnd(10) {
                0 | 1 | 2 => self.simple(),
                3 | 4 => {
                    self.cond();
                    let l = self.label();
                    let c = 4 + self.rnd(2) as u32;
                    self.br(c, l);
                    pend.push(l);
                }
                5 if depth < 3 => {
                    self.cond();
                    let (e, end) = (self.label(), self.label());
                    let c = 4 + self.rnd(2) as u32;
                    self.br(c, e);
                    self.seq(depth + 1, exits);
                    self.br(2, end);
                    self.place(e);
                    self.seq(depth + 1, exits);
                    self.place(end);
                }
                6 | 7 if depth < 3 => {
                    let ctr = 40 + depth;
                    let trips = 1 + self.rnd(3) as i32;
                    self.ops.push(Op::Movk(ctr, 0));
                    let head = self.ops.len();
                    let (out, latch) = (self.label(), self.label());
                    let mut ex = exits.to_vec();
                    ex.push(out);
                    ex.push(latch);
                    self.seq(depth + 1, &ex);
                    if self.rnd(3) == 0 {
                        self.ops.push(Op::Addk(ctr, 1));
                        self.ops.push(Op::CmpLt(ctr, trips));
                        let at = self.ops.len();
                        self.ops.push(Op::Br(5, 0));
                        self.fix_back(at, head);
                        self.seq(depth + 1, &ex);
                    }
                    self.place(latch);
                    self.ops.push(Op::Addk(ctr, 1));
                    self.ops.push(Op::CmpLt(ctr, trips));
                    let at = self.ops.len();
                    self.ops.push(Op::Br(5, 0));
                    self.fix_back(at, head);
                    self.place(out);
                }
                8 if !exits.is_empty() => {
                    self.cond();
                    let l = exits[self.rnd(exits.len() as u64) as usize];
                    let c = 4 + self.rnd(2) as u32;
                    self.br(c, l);
                }
                _ => self.simple(),
            }
        }
        for l in pend {
            self.place(l);
        }
    }
    fn fix_back(&mut self, at: usize, head: usize) {
        if let Op::Br(c, _) = self.ops[at] {
            self.ops[at] = Op::Br(c, head);
        }
    }
    fn finish(&mut self) {
        for &(at, l) in &self.fix {
            if let Op::Br(c, _) = self.ops[at] {
                self.ops[at] = Op::Br(c, self.labels[l]);
            }
        }
    }
}

fn words(ops: &[Op]) -> Vec<u32> {
    ops.iter()
        .enumerate()
        .map(|(i, o)| match *o {
            Op::Movk(r, k) => sopk(0x00, r, k as u16),
            Op::Addk(r, k) => sopk(0x0f, r, k as u16),
            Op::Add(d, a, b) => sop2(0x02, d, a, b),
            Op::CmpLt(r, k) => sopc(0x04, r, ic(k)),
            Op::Br(c, t) => sopp(c, (t as i64 - i as i64 - 1) as i16 as u16),
        })
        .collect()
}

fn oracle(ops: &[Op], init: [u32; 4]) -> Option<[u32; 4]> {
    let mut s = [0i32; 64];
    for (k, v) in init.iter().enumerate() {
        s[20 + k] = *v as i32;
    }
    let mut scc = false;
    let mut pc = 0usize;
    let mut steps = 0;
    while pc < ops.len() {
        steps += 1;
        if steps > 20000 {
            return None;
        }
        match ops[pc] {
            Op::Movk(r, k) => s[r as usize] = k as i32,
            Op::Addk(r, k) => {
                let (v, o) = s[r as usize].overflowing_add(k as i32);
                s[r as usize] = v;
                scc = o;
            }
            Op::Add(d, a, b) => {
                let (v, o) = s[a as usize].overflowing_add(s[b as usize]);
                s[d as usize] = v;
                scc = o;
            }
            Op::CmpLt(r, k) => scc = s[r as usize] < k,
            Op::Br(c, t) => {
                let taken = match c {
                    2 => true,
                    4 => !scc,
                    _ => scc,
                };
                if taken {
                    pc = t;
                    continue;
                }
            }
        }
        pc += 1;
    }
    Some([s[20] as u32, s[21] as u32, s[22] as u32, s[23] as u32])
}

#[test]
fn random_control_flow_matches_a_reference_interpreter() {
    let n: u64 = std::env::var("TOMBOX_CFG_FUZZ_N").ok().and_then(|v| v.parse().ok()).unwrap_or(160);
    let mut fails = Vec::new();
    let mut ran = 0;
    let mut seen = [0usize; 4];
    for seed in 1..=n {
        let mut g = Gen { s: seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1, ops: vec![], labels: vec![], fix: vec![] };
        g.seq(0, &[]);
        g.finish();
        let init = [g.rnd(7) as u32, g.rnd(5) as u32, 3, g.rnd(11) as u32];
        let Some(want) = oracle(&g.ops, init) else { continue };
        let mut p = Prog::new();
        p.sv(20, init[0]).sv(21, init[1]).sv(22, init[2]).sv(23, init[3]).raw(&words(&g.ops)).s2v(0, 20).s2v(1, 21).s2v(2, 22).s2v(3, 23);
        ran += 1;
        let h = hlsl_of(&p.words()).unwrap_or_default();
        for (k, pat) in ["break;", "} else {", "= true;", "tb_pc"].iter().enumerate() {
            seen[k] += h.contains(pat) as usize;
        }
        match run(&p.words()) {
            Ok(got) if got == want => {}
            Ok(got) => {
                fails.push(format!("seed {seed}: got {got:08x?} want {want:08x?}\nops {:?}\n{h}", g.ops));
            }
            Err(e) => fails.push(format!("seed {seed}: {}", e.lines().take(4).collect::<Vec<_>>().join(" | "))),
        }
        if fails.len() >= 3 {
            break;
        }
    }
    println!("cfg_fuzz: {ran} programs run, {} failing; with break {}, else {}, flags {}, dispatcher {}", fails.len(), seen[0], seen[1], seen[2], seen[3]);
    assert!(fails.is_empty(), "{}", fails.join("\n\n"));
}
