use super::bc1;
use super::rans::{self, ByteSource, DecodeError, Model, L};
use std::collections::VecDeque;

#[derive(Clone, Debug)]
pub struct Fragment {
    pub symbols: Vec<u16>,
    pub q: u8,
    pub z: u8,
}

pub fn header_model(alphabet_bits: u8, z: u8) -> Option<u8> {
    let e = (alphabet_bits as i32 - z as i32) & 15;
    let eab = if e < 10 { e as u8 } else { 9 };
    if eab > 8 {
        None
    } else {
        Some(eab)
    }
}

fn header(z: u8, flush: bool, q: u8, len: usize) -> Vec<u8> {
    assert!((1..=65535 + 257).contains(&len), "fragment length {len}");
    let hdr3 = len > 256;
    let b0 = (z & 3) << 6 | (hdr3 as u8) << 5 | (flush as u8) << 4 | (q & 15);
    if hdr3 {
        let v = len - 257;
        vec![b0, v as u8, (v >> 8) as u8]
    } else {
        vec![b0, (len - 1) as u8]
    }
}

pub fn encode_fragment(alphabet_bits: u8, frag: &Fragment) -> Result<Vec<u8>, String> {
    let eab = header_model(alphabet_bits, frag.z).ok_or("z selects an undefined alphabet")?;
    let m: &Model = rans::model(eab, frag.q);
    if frag.symbols.is_empty() {
        return Err("empty fragment".into());
    }
    let mut x: u64 = L as u64;
    let mut rev: Vec<u8> = Vec::new();
    let mut emitted_before_last = 0usize;
    for (k, &s) in frag.symbols.iter().enumerate().rev() {
        if s as u32 >> (eab + 1) != 0 {
            return Err(format!("symbol {s} does not fit {} bits", eab + 1));
        }
        let (f, cf) = m.forward(s);
        if f == 0 {
            return Err(format!(
                "symbol {s} has zero frequency in model eab {eab} q {}",
                frag.q
            ));
        }
        let x_max = (f as u64) << 16;
        while x >= x_max {
            rev.push(x as u8);
            x >>= 8;
        }
        if k == frag.symbols.len() - 1 {
            emitted_before_last = rev.len();
        }
        x = ((x / f as u64) << 16) + (x % f as u64) + cf as u64;
    }
    let mut ntrim = 0;
    while ntrim < 2 && ntrim < rev.len() && rev[ntrim] == 0 && emitted_before_last > ntrim {
        ntrim += 1;
    }
    let mut payload = (x as u32).to_be_bytes().to_vec();
    payload.extend(rev[ntrim..].iter().rev());
    let mut out = header(frag.z, true, frag.q, payload.len());
    out.extend(payload);
    Ok(out)
}

struct PerLane {
    queues: Vec<VecDeque<u8>>,
    order: Vec<usize>,
}

impl ByteSource for PerLane {
    fn next(&mut self, lane: usize) -> Option<u8> {
        let b = self.queues[lane].pop_front();
        if b.is_some() {
            self.order.push(lane);
        }
        b
    }
}

pub fn interleave(
    alphabet_bits: &[u8],
    target: usize,
    lane_bytes: &[Vec<u8>],
) -> Result<Vec<u8>, DecodeError> {
    let total: usize = lane_bytes.iter().map(|v| v.len()).sum();
    let mut src = PerLane {
        queues: lane_bytes
            .iter()
            .map(|v| v.iter().copied().collect())
            .collect(),
        order: vec![],
    };
    rans::decode_lanes(&mut src, alphabet_bits, target, total)?;
    if src.queues.iter().any(|q| !q.is_empty()) {
        return Err(DecodeError::Invalid("a lane left bytes unread".into()));
    }
    let mut cursors = vec![0usize; lane_bytes.len()];
    Ok(src
        .order
        .iter()
        .map(|&l| {
            let b = lane_bytes[l][cursors[l]];
            cursors[l] += 1;
            b
        })
        .collect())
}

pub fn encode_lanes(alphabet_bits: &[u8], lanes: &[Vec<Fragment>]) -> Result<Vec<u8>, String> {
    let target = lanes[0].iter().map(|f| f.symbols.len()).sum::<usize>();
    let mut bytes = Vec::new();
    for (i, frags) in lanes.iter().enumerate() {
        if frags.iter().map(|f| f.symbols.len()).sum::<usize>() != target {
            return Err(format!("lane {i} has a different symbol count"));
        }
        let mut v = Vec::new();
        for f in frags {
            v.extend(encode_fragment(alphabet_bits[i], f)?);
        }
        bytes.push(v);
    }
    interleave(alphabet_bits, target, &bytes).map_err(|e| e.to_string())
}

pub struct Rng(pub u64);

impl Rng {
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    pub fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n
    }
}

pub fn sample_symbol(rng: &mut Rng, eab: u8, q: u8) -> u16 {
    let m = rans::model(eab, q);
    loop {
        let cf = 1 + rng.below(0xFFFF) as u16;
        let s = m.inverse_exact(cf);
        if s as u32 >> (eab + 1) == 0 && m.forward(s).0 > 0 {
            return s;
        }
    }
}

pub fn synthetic_bc1(seed: u64, blocks: usize, max_frag: usize) -> (Vec<u8>, Vec<Vec<u16>>) {
    let mut rng = Rng(seed | 1);
    let ab = bc1::BC1_ALPHABET_BITS;
    let mut lanes = Vec::new();
    let mut syms = Vec::new();
    for &a in ab.iter() {
        let mut frags = Vec::new();
        let mut all = Vec::new();
        let mut left = blocks;
        while left > 0 {
            let n = left.min(1 + rng.below(max_frag as u64) as usize);
            let z = if rng.below(4) == 0 {
                rng.below(a as u64 + 1).min(3) as u8
            } else {
                0
            };
            let eab = header_model(a, z).unwrap();
            let q = rng.below(16) as u8;
            let s: Vec<u16> = (0..n).map(|_| sample_symbol(&mut rng, eab, q)).collect();
            all.extend(&s);
            frags.push(Fragment { symbols: s, q, z });
            left -= n;
        }
        lanes.push(frags);
        syms.push(all);
    }
    let stream = encode_lanes(&ab, &lanes).expect("synthetic encode");
    (stream, syms)
}
