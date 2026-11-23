use super::{CMD_SIZE, E_INVALIDARG, OPUS_MAX_FRAME, OPUS_RATE, RESULT_SIZE, S_OK, XAPU_E_BAD_OPUS_PACKET, XAPU_E_BUFFER_TOO_SMALL};
use opusdec::OpusDecoder;
use std::collections::VecDeque;

pub const PT_DECODE_OPUS: u32 = 1;
pub const PT_DECODE_CONVERT_OPUS: u32 = 2;
pub const PT_CONVERT_PCM: u32 = 3;

pub const OPT_DEINTERLEAVED_INPUT: u32 = 2;
pub const OPT_DEINTERLEAVED_OUTPUT: u32 = 4;

pub const CMDOPT_FLUSH: u32 = 2;
pub const CMDOPT_DECODER: u32 = 4;

pub const STATE_OUTPUT_READY: u32 = 1;
pub const STATE_NEED_MORE_INPUT: u32 = 2;
pub const STATE_MORE_OUTPUT: u32 = 4;
pub const STATE_SATURATION: u32 = 8;

pub const MIN_PITCH: f32 = 0.125;
pub const MAX_PITCH: f32 = 8.0;
pub const MIN_RAMP: f32 = 1.0 / 262144.0;
pub const MAX_RAMP: f32 = 8.0;

pub const GROUP_DELAY: usize = 5;
const HALF: usize = GROUP_DELAY;
const KEEP: usize = HALF * 8 + 2;
const PCM_MAX_FRAMES: u32 = 960;

#[derive(Clone, Copy)]
pub struct Config {
    pub codec: u32,
    pub frame: usize,
    pub deint_in: bool,
    pub deint_out: bool,
}

pub struct Start {
    channels: usize,
    pitch: f32,
    decoder: Option<OpusDecoder>,
    pub(crate) error: i32,
}

pub struct Stream {
    pub channels: usize,
    start_pitch: f32,
    pitch: f32,
    decoder: Option<OpusDecoder>,
    error: i32,
    hist: Vec<VecDeque<f32>>,
    pos: f64,
    pad: usize,
    out: Vec<VecDeque<f32>>,
    pcm: Vec<f32>,
}

impl Stream {
    pub fn new() -> Stream {
        Stream { channels: 0, start_pitch: 1.0, pitch: 1.0, decoder: None, error: S_OK, hist: Vec::new(), pos: 0.0, pad: HALF, out: Vec::new(), pcm: Vec::new() }
    }

    pub fn stop(&mut self) {
        self.decoder = None;
        self.hist.clear();
        self.out.clear();
    }

    fn clear_converter(&mut self) {
        self.pitch = self.start_pitch;
        self.pos = 0.0;
        self.pad = HALF;
        self.hist = (0..self.channels).map(|_| VecDeque::from(vec![0.0f32; HALF])).collect();
        self.out = vec![VecDeque::new(); self.channels];
    }

    pub fn apply_start(&mut self, s: Start) {
        self.channels = s.channels;
        self.start_pitch = s.pitch;
        self.decoder = s.decoder;
        self.error = s.error;
        self.clear_converter();
    }

    pub fn reset(&mut self, cfg: &Config, options: u32) {
        let decoder = match cfg.codec {
            PT_DECODE_OPUS => true,
            PT_DECODE_CONVERT_OPUS => options & CMDOPT_DECODER != 0,
            _ => false,
        };
        if decoder {
            if let Some(d) = self.decoder.as_mut() {
                d.reset();
            }
        }
        self.clear_converter();
    }
}

fn rd32(c: &[u8; CMD_SIZE], o: usize) -> u32 {
    u32::from_le_bytes(c[o..o + 4].try_into().unwrap())
}

fn rd64(c: &[u8; CMD_SIZE], o: usize) -> u64 {
    u64::from_le_bytes(c[o..o + 8].try_into().unwrap())
}

pub fn prepare_start(cfg: &Config, cmd: &[u8; CMD_SIZE]) -> Result<Start, i32> {
    let ch = rd32(cmd, 0x18);
    let pitch = f32::from_bits(rd32(cmd, 0x28));
    let size = rd32(cmd, 0x2c);
    if cfg.codec != PT_DECODE_OPUS {
        let left = size.wrapping_sub(ch.wrapping_mul(cfg.frame as u32).wrapping_mul(4)) as i32 as i64 as u64;
        let need = (ch as u64 * 0x1e0) << 2;
        if left < need {
            return Err(XAPU_E_BUFFER_TOO_SMALL);
        }
    }
    let mut s = Start { channels: (ch as usize).clamp(1, 2), pitch: 1.0, decoder: None, error: S_OK };
    if !(1..=2).contains(&ch) {
        s.error = E_INVALIDARG;
        return Ok(s);
    }
    if cfg.codec != PT_DECODE_OPUS {
        if !pitch.is_finite() {
            s.error = E_INVALIDARG;
            return Ok(s);
        }
        s.pitch = pitch.clamp(MIN_PITCH, MAX_PITCH);
    }
    if cfg.codec != PT_CONVERT_PCM {
        match OpusDecoder::new(OPUS_RATE, ch as usize) {
            Ok(d) => s.decoder = Some(d),
            Err(_) => s.error = E_INVALIDARG,
        }
    }
    Ok(s)
}

fn kaiser(x: f64) -> f64 {
    fn i0(x: f64) -> f64 {
        let (mut s, mut t, mut k) = (1.0f64, 1.0f64, 1.0f64);
        while t > 1e-12 * s {
            t *= (x / (2.0 * k)) * (x / (2.0 * k));
            s += t;
            k += 1.0;
        }
        s
    }
    const BETA: f64 = 6.0;
    if x.abs() >= 1.0 {
        0.0
    } else {
        i0(BETA * (1.0 - x * x).sqrt()) / i0(BETA)
    }
}

fn sinc(x: f64) -> f64 {
    if x == 0.0 {
        1.0
    } else {
        let p = core::f64::consts::PI * x;
        p.sin() / p
    }
}

fn interp(h: &VecDeque<f32>, pos: f64, pitch: f32) -> f32 {
    let i = pos.floor();
    let frac = pos - i;
    let i = i as isize;
    let at = |k: isize| if k < 0 { 0.0 } else { h.get(k as usize).copied().unwrap_or(0.0) as f64 };
    if pitch <= 1.0 && frac == 0.0 {
        return at(i) as f32;
    }
    let fc = if pitch > 1.0 { 1.0 / pitch as f64 } else { 1.0 };
    let w = (HALF as f64 / fc).ceil() as isize;
    let mut acc = 0.0f64;
    for k in (i - w + 1)..=(i + w) {
        let d = pos - k as f64;
        acc += at(k) * fc * sinc(fc * d) * kaiser(d * fc / HALF as f64);
    }
    acc as f32
}

fn reach(pitch: f32) -> usize {
    let fc = if pitch > 1.0 { 1.0 / pitch as f64 } else { 1.0 };
    (HALF as f64 / fc).ceil() as usize
}

impl Stream {
    fn convert(&mut self, ramp: f32, target: f32, limit: usize) -> usize {
        let ch = self.channels;
        let mut consumed = 0usize;
        while self.out[0].len() < limit {
            let len = self.hist[0].len();
            let need = self.pos.floor() as usize + reach(self.pitch);
            if need >= len {
                break;
            }
            for c in 0..ch {
                let v = interp(&self.hist[c], self.pos, self.pitch);
                self.out[c].push_back(v);
            }
            let before = (self.pos.floor() as usize).max(self.pad);
            self.pos += self.pitch as f64;
            consumed += (self.pos.floor() as usize).max(self.pad) - before;
            if self.pitch != target {
                if self.pitch < target {
                    self.pitch = (self.pitch + ramp).min(target);
                } else {
                    self.pitch = (self.pitch - ramp).max(target);
                }
            }
            let drop = (self.pos.floor() as usize).saturating_sub(KEEP);
            if drop > 0 {
                for c in 0..ch {
                    let n = drop.min(self.hist[c].len());
                    self.hist[c].drain(..n);
                }
                self.pos -= drop as f64;
                self.pad = self.pad.saturating_sub(drop);
            }
        }
        consumed
    }

    fn unconverted(&self) -> usize {
        self.hist.first().map_or(0, |h| h.len().saturating_sub((self.pos.ceil() as usize).max(self.pad)))
    }
}

pub struct Outcome {
    pub status: i32,
    pub state: u32,
    pub state_data: [u16; 4],
    pub frames: usize,
    pub bytes: usize,
    pub pitch: f32,
}

fn decode(st: &mut Stream, cmd: &[u8; CMD_SIZE]) -> Result<Vec<Vec<f32>>, i32> {
    let ch = st.channels;
    let first = rd32(cmd, 0x20) as usize;
    let count = rd32(cmd, 0x24);
    let in_len = rd32(cmd, 0x28) as usize;
    let in_ptr = rd64(cmd, 0x2c) as usize;
    if in_ptr == 0 || in_len == 0 {
        return Ok(vec![Vec::new(); ch]);
    }
    let pkt = unsafe { core::slice::from_raw_parts(in_ptr as *const u8, in_len.min(0x10000)) };
    if st.pcm.len() != OPUS_MAX_FRAME * ch {
        st.pcm = vec![0f32; OPUS_MAX_FRAME * ch];
    }
    let n = match st.decoder.as_mut() {
        Some(d) => d.decode_float(pkt, &mut st.pcm, OPUS_MAX_FRAME as i32, false),
        None => return Err(E_INVALIDARG),
    };
    if n < 0 {
        return Err(XAPU_E_BAD_OPUS_PACKET);
    }
    let n = n as usize;
    let start = first.min(n);
    let end = if count == u32::MAX { n } else { (start + count as usize).min(n) };
    Ok((0..ch).map(|c| (start..end).map(|i| st.pcm[i * ch + c]).collect()).collect())
}

fn pcm_input(cfg: &Config, st: &Stream, cmd: &[u8; CMD_SIZE]) -> Result<Vec<Vec<f32>>, i32> {
    let ch = st.channels;
    let first = rd32(cmd, 0x20) as usize;
    let frames = rd32(cmd, 0x24);
    let in_len = rd32(cmd, 0x28) as usize;
    let in_ptr = rd64(cmd, 0x2c) as usize;
    if in_ptr == 0 {
        return Ok(vec![Vec::new(); ch]);
    }
    let step = if ch == 2 && !cfg.deint_in { 8 } else { 4 };
    if !(step..=PCM_MAX_FRAMES).contains(&frames) || frames % step != 0 || in_len != frames as usize * ch * 4 || first > frames as usize {
        return Err(E_INVALIDARG);
    }
    let frames = frames as usize;
    let src = in_ptr as *const f32;
    let at = |c: usize, i: usize| unsafe { core::ptr::read_unaligned(if cfg.deint_in { src.add(c * frames + i) } else { src.add(i * ch + c) }) };
    Ok((0..ch).map(|c| (first..frames).map(|i| at(c, i)).collect()).collect())
}

fn put(out: *mut f32, idx: usize, v: f32) {
    unsafe { core::ptr::write_unaligned(out.add(idx), v) };
}

pub fn process(cfg: &Config, st: &mut Stream, cmd: &[u8; CMD_SIZE]) -> Outcome {
    let ch = st.channels.max(1);
    let options = rd32(cmd, 0xc);
    let ramp = f32::from_bits(rd32(cmd, 0x18));
    let target = f32::from_bits(rd32(cmd, 0x1c));
    let out_cap = rd32(cmd, 0x34) as usize;
    let out_ptr = rd64(cmd, 0x38) as usize as *mut f32;
    let mut o = Outcome { status: S_OK, state: 0, state_data: [0; 4], frames: 0, bytes: 0, pitch: st.pitch };
    if st.error != S_OK {
        o.status = st.error;
        return o;
    }
    if cfg.codec == PT_DECODE_OPUS {
        let pcm = match decode(st, cmd) {
            Ok(p) => p,
            Err(e) => {
                o.status = e;
                return o;
            }
        };
        let n = pcm[0].len();
        if n * ch * 4 > out_cap || (n > 0 && out_ptr.is_null()) {
            o.status = XAPU_E_BUFFER_TOO_SMALL;
            return o;
        }
        let mut sat = false;
        for (c, p) in pcm.iter().enumerate() {
            for (i, &v) in p.iter().enumerate() {
                sat |= v.abs() > 1.0;
                put(out_ptr, if cfg.deint_out { c * n + i } else { i * ch + c }, v);
            }
        }
        o.state = if sat { STATE_SATURATION } else { 0 };
        o.frames = n;
        o.bytes = n * ch * 4;
        return o;
    }
    let ramp = if ramp.is_finite() { ramp.abs().clamp(MIN_RAMP, MAX_RAMP) } else { MIN_RAMP };
    if !target.is_finite() {
        o.status = E_INVALIDARG;
        return o;
    }
    let target = target.clamp(MIN_PITCH, MAX_PITCH);
    let input = if cfg.codec == PT_CONVERT_PCM { pcm_input(cfg, st, cmd) } else { decode(st, cmd) };
    let input = match input {
        Ok(p) => p,
        Err(e) => {
            o.status = e;
            o.state = STATE_NEED_MORE_INPUT;
            return o;
        }
    };
    for (c, p) in input.into_iter().enumerate() {
        st.hist[c].extend(p);
    }
    let frame = cfg.frame;
    let consumed = st.convert(ramp, target, frame);
    let have = st.out[0].len();
    let flush = options & CMDOPT_FLUSH != 0;
    let emit = if have >= frame { frame } else if flush { have } else { 0 };
    if emit > 0 {
        let need = if cfg.deint_out { ((ch - 1) * frame + emit) * 4 } else { emit * ch * 4 };
        if need > out_cap || out_ptr.is_null() {
            o.status = XAPU_E_BUFFER_TOO_SMALL;
        } else {
            let mut sat = false;
            for c in 0..ch {
                for (i, v) in st.out[c].drain(..emit).enumerate() {
                    sat |= v.abs() > 1.0;
                    put(out_ptr, if cfg.deint_out { c * frame + i } else { i * ch + c }, v);
                }
            }
            o.frames = emit;
            o.bytes = emit * ch * 4;
            o.state |= STATE_OUTPUT_READY;
            if sat {
                o.state |= STATE_SATURATION;
            }
        }
    }
    let more = st.convert(ramp, target, frame);
    let left = st.out[0].len();
    let consumed = consumed + more;
    if left >= frame {
        o.state |= STATE_MORE_OUTPUT;
    } else {
        o.state |= STATE_NEED_MORE_INPUT;
    }
    o.state_data = [left.min(0xffff) as u16, consumed.min(0xffff) as u16, 0, st.unconverted().min(0xffff) as u16];
    o.pitch = st.pitch;
    o
}

pub fn fill_result(r: &mut [u8; RESULT_SIZE], o: &Outcome, cmd: &[u8; CMD_SIZE]) {
    r[0x18..0x1c].copy_from_slice(&o.state.to_le_bytes());
    for (k, v) in o.state_data.iter().enumerate() {
        r[0x1c + 2 * k..0x1e + 2 * k].copy_from_slice(&v.to_le_bytes());
    }
    r[0x2c..0x30].copy_from_slice(&(o.status as u32).to_le_bytes());
    r[0x30..0x34].copy_from_slice(&(o.frames as u32).to_le_bytes());
    r[0x34..0x38].copy_from_slice(&(o.bytes as u32).to_le_bytes());
    r[0x38..0x40].copy_from_slice(&cmd[0x38..0x40]);
    r[0x40..0x44].copy_from_slice(&o.pitch.to_bits().to_le_bytes());
    r[0x44..0x48].copy_from_slice(&cmd[0x28..0x2c]);
    r[0x48..0x50].copy_from_slice(&cmd[0x2c..0x34]);
}
