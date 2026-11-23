use compat_common::{record, S_OK};
use core::ffi::c_void;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;

const M: &str = "chad";

pub const E_INVALIDARG: i32 = 0x8007_0057u32 as i32;
pub const E_POINTER: i32 = 0x8000_4003u32 as i32;
pub const XDSP_E_NO_SLOT: i32 = 0x8AC8_10A0u32 as i32;
pub const XDSP_E_BUSY: i32 = 0x8AC8_10A1u32 as i32;
pub const XDSP_E_TOO_MANY_CONTEXTS: i32 = 0x8AC8_10A2u32 as i32;
pub const XDSP_E_CONTEXTS_ACTIVE: i32 = 0x8AC8_10A4u32 as i32;
pub const XDSP_E_TOO_MANY_CLIENTS: i32 = 0x8AC8_10A5u32 as i32;
pub const XDSP_E_TOO_MANY_ACTIVATIONS: i32 = 0x8AC8_10A6u32 as i32;
pub const XDSP_E_IR_SIZE: i32 = 0x8AC8_10A7u32 as i32;

const IR_METADATA_MARKER: u32 = 0xFFFF_ACE1;
pub const KIND_CONVOLUTION: u32 = 1;
pub const KIND_FFT: u32 = 2;
pub const KIND_IFFT: u32 = 3;
pub const OPT_POLAR: u32 = 4;
const MAX_CLIENTS: u32 = 8;
const MAX_ACTIVATIONS: u32 = 0x80;
const MAX_CONTEXTS: u32 = 0x100;

struct Global {
    clients: u32,
    activations: u32,
}

static GLOBAL: Mutex<Global> = Mutex::new(Global { clients: 0, activations: 0 });

fn global() -> std::sync::MutexGuard<'static, Global> {
    GLOBAL.lock().unwrap_or_else(|e| e.into_inner())
}

pub struct Client {
    max_act: u32,
    active: u32,
    pub max_streams: u32,
    pub priority: u32,
    pub mem: usize,
    pub mem_size: usize,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct C32 {
    re: f32,
    im: f32,
}

struct Fft {
    n: usize,
    rev: Vec<usize>,
    tw: Vec<C32>,
}

impl Fft {
    fn new(n: usize) -> Fft {
        let bits = n.trailing_zeros();
        let rev = (0..n).map(|i| i.reverse_bits() >> (usize::BITS - bits)).collect();
        let tw = (0..n / 2)
            .map(|k| {
                let a = -2.0 * core::f64::consts::PI * k as f64 / n as f64;
                C32 { re: a.cos() as f32, im: a.sin() as f32 }
            })
            .collect();
        Fft { n, rev, tw }
    }

    fn run(&self, d: &mut [C32], inverse: bool) {
        let n = self.n;
        for i in 0..n {
            let j = self.rev[i];
            if j > i {
                d.swap(i, j);
            }
        }
        let mut len = 2;
        while len <= n {
            let step = n / len;
            for s in (0..n).step_by(len) {
                for k in 0..len / 2 {
                    let mut w = self.tw[k * step];
                    if inverse {
                        w.im = -w.im;
                    }
                    let a = d[s + k];
                    let b = d[s + k + len / 2];
                    let t = C32 { re: b.re * w.re - b.im * w.im, im: b.re * w.im + b.im * w.re };
                    d[s + k] = C32 { re: a.re + t.re, im: a.im + t.im };
                    d[s + k + len / 2] = C32 { re: a.re - t.re, im: a.im - t.im };
                }
            }
            len <<= 1;
        }
    }
}

fn pack_forward(fft: &Fft, time: &[f32], work: &mut [C32], out: &mut [C32]) {
    let n = fft.n;
    for i in 0..n {
        work[i] = C32 { re: time[i], im: 0.0 };
    }
    fft.run(work, false);
    let b = n / 2;
    out[0] = C32 { re: work[0].re, im: work[b].re };
    out[1..b].copy_from_slice(&work[1..b]);
}

fn unpack_inverse(fft: &Fft, spec: &[C32], work: &mut [C32]) {
    let n = fft.n;
    let b = n / 2;
    work[0] = C32 { re: spec[0].re, im: 0.0 };
    work[b] = C32 { re: spec[0].im, im: 0.0 };
    for k in 1..b {
        work[k] = spec[k];
        work[n - k] = C32 { re: spec[k].re, im: -spec[k].im };
    }
    fft.run(work, true);
    let s = 1.0 / n as f32;
    for v in work.iter_mut() {
        v.re *= s;
    }
}

fn to_polar(spec: &mut [C32]) {
    for c in spec.iter_mut().skip(1) {
        let (m, a) = ((c.re as f64).hypot(c.im as f64), (c.im as f64).atan2(c.re as f64));
        *c = C32 { re: m as f32, im: a as f32 };
    }
}

fn from_polar(spec: &mut [C32]) {
    for c in spec.iter_mut().skip(1) {
        let (m, a) = (c.re as f64, c.im as f64);
        *c = C32 { re: (m * a.cos()) as f32, im: (m * a.sin()) as f32 };
    }
}

fn spectral_mac(acc: &mut [C32], x: &[C32], h: &[C32], g: f32) {
    acc[0].re += x[0].re * h[0].re * g;
    acc[0].im += x[0].im * h[0].im * g;
    for k in 1..acc.len() {
        acc[k].re += (x[k].re * h[k].re - x[k].im * h[k].im) * g;
        acc[k].im += (x[k].re * h[k].im + x[k].im * h[k].re) * g;
    }
}

struct Channel {
    hist: Vec<f32>,
    fdl: Vec<Vec<C32>>,
}

pub struct Context {
    status: [u32; 4],
    client: *mut Client,
    kind: u32,
    polar: bool,
    block: usize,
    partitions: usize,
    irs: Vec<Vec<Vec<C32>>>,
    chans: Vec<Channel>,
    pos: usize,
    fft: Fft,
    work: Vec<C32>,
    acc: Vec<C32>,
    envelope_count: usize,
    envelope: Option<Vec<f32>>,
    group: u32,
}

fn sanitize(v: f32) -> f32 {
    if v.is_finite() {
        v
    } else {
        0.0
    }
}

impl Context {
    fn new(client: *mut Client, block: usize, channels: usize, partitions: usize, ir: &[f32], nir: usize) -> Context {
        let mut irs = Vec::with_capacity(nir);
        for r in 0..nir {
            let base = r * partitions * 2 * block;
            let mut parts = Vec::with_capacity(partitions);
            for p in 0..partitions {
                let s = &ir[base + p * 2 * block..base + (p + 1) * 2 * block];
                let marker = s[0].to_bits() == IR_METADATA_MARKER;
                let part: Vec<C32> = (0..block)
                    .map(|k| if marker { C32 { re: 0.0, im: 0.0 } } else { C32 { re: sanitize(s[2 * k]), im: sanitize(s[2 * k + 1]) } })
                    .collect();
                parts.push(part);
            }
            while parts.last().is_some_and(|p| p.iter().all(|c| c.re == 0.0 && c.im == 0.0)) && parts.len() > 1 {
                parts.pop();
            }
            irs.push(parts);
        }
        let used = irs.iter().map(|p| p.len()).max().unwrap_or(1);
        let chans = (0..channels)
            .map(|_| Channel { hist: vec![0.0; 2 * block], fdl: vec![vec![C32 { re: 0.0, im: 0.0 }; block]; used] })
            .collect();
        Context {
            status: [0; 4],
            client,
            kind: KIND_CONVOLUTION,
            polar: false,
            block,
            partitions: used,
            irs,
            chans,
            pos: 0,
            fft: Fft::new(2 * block),
            work: vec![C32 { re: 0.0, im: 0.0 }; 2 * block],
            acc: vec![C32 { re: 0.0, im: 0.0 }; block],
            envelope_count: partitions,
            envelope: None,
            group: 0,
        }
    }

    fn new_transform(client: *mut Client, kind: u32, block: usize, polar: bool) -> Context {
        Context {
            status: [0; 4],
            client,
            kind,
            polar,
            block,
            partitions: 0,
            irs: Vec::new(),
            chans: Vec::new(),
            pos: 0,
            fft: Fft::new(block),
            work: vec![C32 { re: 0.0, im: 0.0 }; block],
            acc: vec![C32 { re: 0.0, im: 0.0 }; block / 2],
            envelope_count: 0,
            envelope: None,
            group: 0,
        }
    }

    fn transform(&mut self, input: *const f32, mult: *const f32, output: *mut f32) {
        let n = self.block;
        let rd = |p: *const f32, i: usize| unsafe { core::ptr::read_unaligned(p.add(i)) };
        if self.kind == KIND_FFT {
            let t: Vec<f32> = (0..n).map(|i| sanitize(if mult.is_null() { rd(input, i) } else { rd(input, i) * rd(mult, i) })).collect();
            pack_forward(&self.fft, &t, &mut self.work, &mut self.acc);
            if self.polar {
                to_polar(&mut self.acc);
            }
            for (k, c) in self.acc.iter().enumerate() {
                unsafe {
                    core::ptr::write_unaligned(output.add(2 * k), c.re);
                    core::ptr::write_unaligned(output.add(2 * k + 1), c.im);
                }
            }
        } else {
            for k in 0..n / 2 {
                self.acc[k] = C32 { re: sanitize(rd(input, 2 * k)), im: sanitize(rd(input, 2 * k + 1)) };
            }
            if self.polar {
                from_polar(&mut self.acc);
            }
            unpack_inverse(&self.fft, &self.acc, &mut self.work);
            for i in 0..n {
                unsafe { core::ptr::write_unaligned(output.add(i), sanitize(self.work[i].re)) };
            }
        }
    }

    fn process(&mut self, gain_in: f32, gain_out: f32, input: *const f32, output: *mut f32) {
        let b = self.block;
        let p = self.partitions;
        self.pos = (self.pos + 1) % p;
        let nir = self.irs.len();
        for c in 0..self.chans.len() {
            let ch = &mut self.chans[c];
            ch.hist.copy_within(b.., 0);
            for i in 0..b {
                let v = if input.is_null() { 0.0 } else { unsafe { *input.add(c * b + i) } };
                ch.hist[b + i] = sanitize(v * gain_in);
            }
            let slot = self.pos;
            pack_forward(&self.fft, &ch.hist, &mut self.work, &mut ch.fdl[slot]);
            for a in self.acc.iter_mut() {
                *a = C32 { re: 0.0, im: 0.0 };
            }
            let r = c.min(nir - 1);
            let ir = &self.irs[r];
            let stride = (self.envelope_count + 3) & !3;
            for (k, h) in ir.iter().enumerate() {
                let x = &ch.fdl[(slot + p - k) % p];
                let g = self.envelope.as_ref().map_or(1.0, |e| sanitize(e[r * stride + k]));
                spectral_mac(&mut self.acc, x, h, g);
            }
            unpack_inverse(&self.fft, &self.acc, &mut self.work);
            if !output.is_null() {
                for i in 0..b {
                    unsafe { *output.add(c * b + i) = sanitize(self.work[b + i].re * gain_out) };
                }
            }
        }
    }
}

#[no_mangle]
pub extern "system" fn XDspConnect(mem: *mut c_void, mem_size: u32, max_activations: u32, out: *mut *mut Client) -> i32 {
    XDspConnectWithMaximumStreamLimit(mem, mem_size, max_activations, MAX_STREAMS, MAX_PRIORITY, out)
}

pub const MAX_STREAMS: u32 = 0x100;
pub const MAX_PRIORITY: u32 = 2;

#[no_mangle]
pub extern "system" fn XDspConnectWithMaximumStreamLimit(mem: *mut c_void, mem_size: u32, max_activations: u32, max_streams: u32, priority: u32, out: *mut *mut Client) -> i32 {
    if mem.is_null() || !(0x10000..=0x1F40_0000).contains(&mem_size) || max_activations > MAX_ACTIVATIONS || max_streams > MAX_STREAMS || priority > MAX_PRIORITY {
        return E_INVALIDARG;
    }
    if out.is_null() {
        return E_POINTER;
    }
    unsafe { *out = core::ptr::null_mut() };
    let mut g = global();
    if g.activations + max_activations > MAX_ACTIVATIONS {
        return XDSP_E_TOO_MANY_ACTIVATIONS;
    }
    if g.clients >= MAX_CLIENTS {
        return XDSP_E_TOO_MANY_CLIENTS;
    }
    g.clients += 1;
    g.activations += max_activations;
    let c = Box::into_raw(Box::new(Client { max_act: max_activations, active: 0, max_streams, priority, mem: mem as usize, mem_size: mem_size as usize }));
    unsafe { *out = c };
    record(M, "XDspConnect", &format!("mem={mem:p} size={mem_size:#x} maxActivations={max_activations} maxStreams={max_streams} priority={priority} -> client {c:p} (CPU convolution)"));
    S_OK
}

#[no_mangle]
pub extern "system" fn XDspDisconnect(client: *mut Client) -> i32 {
    if client.is_null() {
        return E_INVALIDARG;
    }
    let c = unsafe { &*client };
    if c.active != 0 {
        return XDSP_E_CONTEXTS_ACTIVE;
    }
    let mut g = global();
    g.clients -= 1;
    g.activations -= c.max_act;
    drop(unsafe { Box::from_raw(client) });
    record(M, "XDspDisconnect", "-> S_OK");
    S_OK
}

#[no_mangle]
pub extern "system" fn XDspActivate(client: *mut Client, desc: *const u8, status_out: *mut *mut u32, ctx_out: *mut *mut Context) -> i32 {
    if status_out.is_null() || ctx_out.is_null() {
        return E_POINTER;
    }
    unsafe {
        *status_out = core::ptr::null_mut();
        *ctx_out = core::ptr::null_mut();
    }
    if client.is_null() || desc.is_null() {
        return E_INVALIDARG;
    }
    let rd = |o: usize| unsafe { core::ptr::read_unaligned(desc.add(o) as *const u32) };
    let (kind, flags, block, channels, len) = (rd(0), rd(4), rd(8), rd(0xc), rd(0x10));
    let ir = unsafe { core::ptr::read_unaligned(desc.add(0x14) as *const u64) } as usize as *const f32;
    if !(1..=3).contains(&kind) || !(1..=2).contains(&channels) {
        return E_INVALIDARG;
    }
    if flags & 0xA != 0 && channels != 2 {
        return E_INVALIDARG;
    }
    if flags & 4 != 0 && kind == 1 {
        return E_INVALIDARG;
    }
    let block_ok = if kind == 1 { matches!(block, 0x200 | 0x400) } else { matches!(block, 0x200 | 0x400 | 0x800) && channels == 1 };
    if !block_ok {
        return E_INVALIDARG;
    }
    if kind != KIND_CONVOLUTION {
        let cl = unsafe { &mut *client };
        if cl.active >= cl.max_act || cl.active >= MAX_CONTEXTS {
            return XDSP_E_TOO_MANY_CONTEXTS;
        }
        let ctx = Box::into_raw(Box::new(Context::new_transform(client, kind, block as usize, flags & OPT_POLAR != 0)));
        cl.active += 1;
        unsafe {
            *ctx_out = ctx;
            *status_out = (*ctx).status.as_mut_ptr();
        }
        record(M, "XDspActivate", &format!("{} block={block} polar={} -> {ctx:p}", if kind == KIND_FFT { "fft" } else { "ifft" }, flags & OPT_POLAR != 0));
        return S_OK;
    }
    let partitions = (len / 2 / block) as usize;
    let fdl = (partitions + 7) & !7;
    if partitions < 9 || fdl > 0x800 {
        return XDSP_E_IR_SIZE;
    }
    if ir.is_null() {
        return E_INVALIDARG;
    }
    let cl = unsafe { &mut *client };
    if cl.active >= cl.max_act || cl.active >= MAX_CONTEXTS {
        return XDSP_E_TOO_MANY_CONTEXTS;
    }
    let nir = if flags & 8 != 0 { 2 } else { 1 };
    let total = nir * partitions * 2 * block as usize;
    let irs = unsafe { core::slice::from_raw_parts(ir, total) };
    let ctx = Box::into_raw(Box::new(Context::new(client, block as usize, channels as usize, partitions, irs, nir)));
    cl.active += 1;
    unsafe {
        *ctx_out = ctx;
        *status_out = (*ctx).status.as_mut_ptr();
    }
    record(M, "XDspActivate", &format!("conv flags={flags:#x} block={block} ch={channels} partitions={partitions} (used {}) -> {ctx:p}", unsafe { (*ctx).partitions }));
    S_OK
}

#[no_mangle]
pub extern "system" fn XDspSubmitCommand(ctx: *mut Context, cmd: *const u8, out_seq: *mut u32) -> i32 {
    submit(ctx, cmd, core::ptr::null(), out_seq, "XDspSubmitCommand")
}

#[no_mangle]
pub extern "system" fn XDspSubmitCommandWithEnvelope(ctx: *mut Context, cmd: *const u8, envelope: *const f32, out_seq: *mut u32) -> i32 {
    submit(ctx, cmd, envelope, out_seq, "XDspSubmitCommandWithEnvelope")
}

fn submit(ctx: *mut Context, cmd: *const u8, envelope: *const f32, out_seq: *mut u32, name: &str) -> i32 {
    static LOG: AtomicU32 = AtomicU32::new(0);
    if ctx.is_null() || cmd.is_null() {
        return E_INVALIDARG;
    }
    let c = unsafe { &mut *ctx };
    if (c.status[3] as i32) < 0 {
        return c.status[3] as i32;
    }
    if c.status[0].wrapping_sub(c.status[1]) >= 8 {
        return XDSP_E_NO_SLOT;
    }
    let g0 = unsafe { core::ptr::read_unaligned(cmd as *const f32) };
    let g1 = unsafe { core::ptr::read_unaligned(cmd.add(4) as *const f32) };
    let input = unsafe { core::ptr::read_unaligned(cmd.add(8) as *const usize) } as *const f32;
    let output = unsafe { core::ptr::read_unaligned(cmd.add(0x10) as *const usize) } as *mut f32;
    let mult = unsafe { core::ptr::read_unaligned(cmd.add(0x18) as *const usize) } as *const f32;
    let (mem, size) = unsafe { ((*c.client).mem, (*c.client).mem_size) };
    let inside = |p: usize| p >= mem && p - mem < size;
    if c.kind != KIND_CONVOLUTION {
        if !inside(input as usize) || !inside(output as usize) || (c.kind == KIND_FFT && !mult.is_null() && !inside(mult as usize)) {
            return E_INVALIDARG;
        }
        c.status[0] = c.status[0].wrapping_add(1);
        c.transform(input, if c.kind == KIND_FFT { mult } else { core::ptr::null() }, output);
        c.status[1] = c.status[0];
        if !out_seq.is_null() {
            unsafe { *out_seq = c.status[0] };
        }
        if LOG.fetch_add(1, Ordering::Relaxed) < 4 {
            record(M, name, &format!("seq={} {} -> S_OK", c.status[0], if c.kind == KIND_FFT { "fft" } else { "ifft" }));
        }
        return S_OK;
    }
    if !inside(input as usize) || !inside(output as usize) {
        return E_INVALIDARG;
    }
    if c.group == 0 {
        if envelope.is_null() {
            c.envelope = None;
        } else {
            if !inside(envelope as usize) {
                return E_INVALIDARG;
            }
            let n = ((c.envelope_count + 3) & !3) * (c.irs.len() - 1) + c.envelope_count;
            c.envelope = Some(unsafe { core::slice::from_raw_parts(envelope, n) }.to_vec());
        }
    }
    c.group = (c.group + 1) & 7;
    c.status[0] = c.status[0].wrapping_add(1);
    c.process(g0, g1, input, output);
    c.status[1] = c.status[0];
    if !out_seq.is_null() {
        unsafe { *out_seq = c.status[0] };
    }
    if LOG.fetch_add(1, Ordering::Relaxed) < 4 {
        record(M, name, &format!("seq={} gains=({g0},{g1}) envelope={} -> S_OK", c.status[0], c.envelope.is_some()));
    }
    S_OK
}

#[no_mangle]
pub extern "system" fn XDspDeactivate(ctx: *mut Context) -> i32 {
    if ctx.is_null() {
        return E_INVALIDARG;
    }
    let c = unsafe { &*ctx };
    if c.status[0] != c.status[1] {
        return XDSP_E_BUSY;
    }
    unsafe { (*c.client).active -= 1 };
    drop(unsafe { Box::from_raw(ctx) });
    S_OK
}

#[cfg(test)]
mod tests {
    use super::*;

    static SERIAL: Mutex<()> = Mutex::new(());

    fn ir_from_time(h: &[f32], block: usize, partitions: usize) -> Vec<f32> {
        let fft = Fft::new(2 * block);
        let mut work = vec![C32 { re: 0.0, im: 0.0 }; 2 * block];
        let mut out = Vec::new();
        for p in 0..partitions {
            let mut t = vec![0.0f32; 2 * block];
            for i in 0..block {
                t[i] = *h.get(p * block + i).unwrap_or(&0.0);
            }
            let mut spec = vec![C32 { re: 0.0, im: 0.0 }; block];
            pack_forward(&fft, &t, &mut work, &mut spec);
            for c in spec {
                out.push(c.re);
                out.push(c.im);
            }
        }
        out
    }

    fn desc(flags: u32, block: u32, ch: u32, len: u32, ir: *const f32) -> [u8; 0x20] {
        let mut d = [0u8; 0x20];
        d[0..4].copy_from_slice(&1u32.to_le_bytes());
        d[4..8].copy_from_slice(&flags.to_le_bytes());
        d[8..12].copy_from_slice(&block.to_le_bytes());
        d[12..16].copy_from_slice(&ch.to_le_bytes());
        d[16..20].copy_from_slice(&len.to_le_bytes());
        d[20..28].copy_from_slice(&(ir as u64).to_le_bytes());
        d
    }

    fn cmd(inp: *const f32, out: *mut f32) -> [u8; 32] {
        let mut c = [0u8; 32];
        c[0..4].copy_from_slice(&1.0f32.to_le_bytes());
        c[4..8].copy_from_slice(&1.0f32.to_le_bytes());
        c[8..16].copy_from_slice(&(inp as u64).to_le_bytes());
        c[16..24].copy_from_slice(&(out as u64).to_le_bytes());
        c
    }

    fn tdesc(kind: u32, flags: u32, block: u32, ch: u32) -> [u8; 0x20] {
        let mut d = desc(flags, block, ch, 0, core::ptr::null());
        d[0..4].copy_from_slice(&kind.to_le_bytes());
        d
    }

    fn fcmd(inp: *const f32, out: *mut f32, mult: *const f32) -> [u8; 32] {
        let mut c = cmd(inp, out);
        c[24..32].copy_from_slice(&(mult as u64).to_le_bytes());
        c
    }

    fn dft(x: &[f32]) -> Vec<(f64, f64)> {
        let n = x.len();
        (0..=n / 2)
            .map(|k| {
                let (mut re, mut im) = (0.0f64, 0.0f64);
                for (t, &v) in x.iter().enumerate() {
                    let a = -2.0 * core::f64::consts::PI * (k * t % n) as f64 / n as f64;
                    re += v as f64 * a.cos();
                    im += v as f64 * a.sin();
                }
                (re, im)
            })
            .collect()
    }

    #[test]
    fn fft_and_ifft_streams_activate_with_console_checks() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let mut mem = vec![0.0f32; 0x4000];
        let mut cl: *mut Client = core::ptr::null_mut();
        assert_eq!(XDspConnect(mem.as_mut_ptr() as *mut c_void, 0x10000, 3, &mut cl), S_OK);
        let junk = [1.0f32; 4];
        let mut ctxs = Vec::new();
        for kind in [KIND_FFT, KIND_IFFT] {
            let (mut st, mut ctx) = (core::ptr::null_mut(), core::ptr::null_mut());
            for (flags, block, ch) in [(0u32, 0x400u32, 2u32), (0, 0x300, 1), (2, 0x400, 1), (8, 0x400, 1), (0, 0x1000, 1)] {
                assert_eq!(XDspActivate(cl, tdesc(kind, flags, block, ch).as_ptr(), &mut st, &mut ctx), E_INVALIDARG, "kind {kind} flags {flags} block {block} ch {ch}");
                assert!(ctx.is_null());
            }
            let mut d = tdesc(kind, OPT_POLAR, 0x800, 1);
            d[16..20].copy_from_slice(&7u32.to_le_bytes());
            d[20..28].copy_from_slice(&(junk.as_ptr() as u64).to_le_bytes());
            assert_eq!(XDspActivate(cl, d.as_ptr(), &mut st, &mut ctx), S_OK, "kind {kind}");
            assert!(!ctx.is_null() && !st.is_null());
            ctxs.push(ctx);
        }
        let mut x: *mut Context = core::ptr::null_mut();
        let mut st: *mut u32 = core::ptr::null_mut();
        assert_eq!(XDspActivate(cl, tdesc(KIND_FFT, 0, 0x200, 1).as_ptr(), &mut st, &mut x), S_OK);
        ctxs.push(x);
        assert_eq!(XDspActivate(cl, tdesc(KIND_FFT, 0, 0x200, 1).as_ptr(), &mut st, &mut x), XDSP_E_TOO_MANY_CONTEXTS);
        assert_eq!(XDspDisconnect(cl), XDSP_E_CONTEXTS_ACTIVE);
        for c in ctxs {
            assert_eq!(XDspDeactivate(c), S_OK);
        }
        assert_eq!(XDspDisconnect(cl), S_OK);
    }

    #[test]
    fn forward_fft_packs_like_convolution_ir_and_matches_dft() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        for n in [0x200usize, 0x400, 0x800] {
            let mut mem = vec![0.0f32; 0x4000];
            let mut cl: *mut Client = core::ptr::null_mut();
            assert_eq!(XDspConnect(mem.as_mut_ptr() as *mut c_void, 0x10000, 2, &mut cl), S_OK);
            let (mut st, mut fx) = (core::ptr::null_mut(), core::ptr::null_mut());
            assert_eq!(XDspActivate(cl, tdesc(KIND_FFT, 0, n as u32, 1).as_ptr(), &mut st, &mut fx), S_OK);
            let x: Vec<f32> = (0..n).map(|i| ((i as f32) * 0.31).sin() + 0.25 * ((i * 37 % 11) as f32 - 5.0) / 5.0).collect();
            mem[..n].copy_from_slice(&x);
            let base = mem.as_mut_ptr();
            let mut seq = 0;
            assert_eq!(XDspSubmitCommand(fx, fcmd(base, unsafe { base.add(0x1000) }, core::ptr::null()).as_ptr(), &mut seq), S_OK);
            assert_eq!(seq, 1);
            unsafe { assert_eq!((*st, *st.add(1)), (1, 1)) };
            let out = &mem[0x1000..0x1000 + n];
            let want = dft(&x);
            let tol = 2e-3 * n as f64;
            assert!((out[0] as f64 - want[0].0).abs() < tol);
            assert!((out[1] as f64 - want[n / 2].0).abs() < tol);
            for k in 1..n / 2 {
                assert!((out[2 * k] as f64 - want[k].0).abs() < tol && (out[2 * k + 1] as f64 - want[k].1).abs() < tol, "n {n} bin {k}");
            }
            let spec = out.to_vec();
            let (mut st2, mut ix) = (core::ptr::null_mut(), core::ptr::null_mut());
            assert_eq!(XDspActivate(cl, tdesc(KIND_IFFT, 0, n as u32, 1).as_ptr(), &mut st2, &mut ix), S_OK);
            mem[0x2000..0x2000 + n].copy_from_slice(&spec);
            let base = mem.as_mut_ptr();
            let junk = vec![9.0f32; n];
            assert_eq!(XDspSubmitCommand(ix, fcmd(unsafe { base.add(0x2000) }, base, junk.as_ptr()).as_ptr(), &mut seq), S_OK);
            for i in 0..n {
                assert!((mem[i] - x[i]).abs() < 1e-4, "n {n} sample {i}");
            }
            assert_eq!(XDspDeactivate(fx), S_OK);
            assert_eq!(XDspDeactivate(ix), S_OK);
            assert_eq!(XDspDisconnect(cl), S_OK);
        }
    }

    #[test]
    fn polar_format_roundtrips_and_holds_magnitude_phase() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let n = 0x400usize;
        let mut mem = vec![0.0f32; 0x4000];
        let mut cl: *mut Client = core::ptr::null_mut();
        assert_eq!(XDspConnect(mem.as_mut_ptr() as *mut c_void, 0x10000, 2, &mut cl), S_OK);
        let (mut st, mut fx, mut ix) = (core::ptr::null_mut(), core::ptr::null_mut(), core::ptr::null_mut());
        assert_eq!(XDspActivate(cl, tdesc(KIND_FFT, OPT_POLAR, n as u32, 1).as_ptr(), &mut st, &mut fx), S_OK);
        assert_eq!(XDspActivate(cl, tdesc(KIND_IFFT, OPT_POLAR, n as u32, 1).as_ptr(), &mut st, &mut ix), S_OK);
        let x: Vec<f32> = (0..n).map(|i| ((i as f32) * 0.05).cos() * 0.7 - ((i as f32) * 0.9).sin() * 0.2).collect();
        mem[..n].copy_from_slice(&x);
        let base = mem.as_mut_ptr();
        assert_eq!(XDspSubmitCommand(fx, fcmd(base, unsafe { base.add(0x1000) }, core::ptr::null()).as_ptr(), core::ptr::null_mut()), S_OK);
        let want = dft(&x);
        let tol = 2e-3 * n as f64;
        assert!((mem[0x1000] as f64 - want[0].0).abs() < tol);
        assert!((mem[0x1001] as f64 - want[n / 2].0).abs() < tol);
        for k in 1..n / 2 {
            let (m, a) = (mem[0x1000 + 2 * k] as f64, mem[0x1000 + 2 * k + 1] as f64);
            assert!((m - want[k].0.hypot(want[k].1)).abs() < tol, "bin {k}");
            assert!(m >= 0.0 && a.abs() <= core::f64::consts::PI + 1e-6);
            if m > 1.0 {
                assert!(((a - want[k].1.atan2(want[k].0) + 3.0 * core::f64::consts::PI) % (2.0 * core::f64::consts::PI) - core::f64::consts::PI).abs() < 1e-3, "bin {k}");
            }
        }
        let base = mem.as_mut_ptr();
        assert_eq!(XDspSubmitCommand(ix, fcmd(unsafe { base.add(0x1000) }, unsafe { base.add(0x2000) }, core::ptr::null()).as_ptr(), core::ptr::null_mut()), S_OK);
        for i in 0..n {
            assert!((mem[0x2000 + i] - x[i]).abs() < 1e-4, "sample {i}");
        }
        assert_eq!(XDspDeactivate(fx), S_OK);
        assert_eq!(XDspDeactivate(ix), S_OK);
        assert_eq!(XDspDisconnect(cl), S_OK);
    }

    #[test]
    fn forward_multiplier_and_buffer_translation_follow_console() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let n = 0x200usize;
        let mut mem = vec![0.0f32; 0x4000];
        let mut cl: *mut Client = core::ptr::null_mut();
        assert_eq!(XDspConnect(mem.as_mut_ptr() as *mut c_void, 0x10000, 2, &mut cl), S_OK);
        let (mut st, mut fx, mut ix) = (core::ptr::null_mut(), core::ptr::null_mut(), core::ptr::null_mut());
        assert_eq!(XDspActivate(cl, tdesc(KIND_FFT, 0, n as u32, 1).as_ptr(), &mut st, &mut fx), S_OK);
        assert_eq!(XDspActivate(cl, tdesc(KIND_IFFT, 0, n as u32, 1).as_ptr(), &mut st, &mut ix), S_OK);
        let x: Vec<f32> = (0..n).map(|i| ((i * 13 % 7) as f32) - 3.0).collect();
        let w: Vec<f32> = (0..n).map(|i| 0.5 - 0.5 * (2.0 * core::f32::consts::PI * i as f32 / n as f32).cos()).collect();
        mem[..n].copy_from_slice(&x);
        mem[0x400..0x400 + n].copy_from_slice(&w);
        let base = mem.as_mut_ptr();
        let outside = vec![0.0f32; n];
        assert_eq!(XDspSubmitCommand(fx, fcmd(base, unsafe { base.add(0x1000) }, outside.as_ptr()).as_ptr(), core::ptr::null_mut()), E_INVALIDARG);
        assert_eq!(XDspSubmitCommand(fx, fcmd(outside.as_ptr(), unsafe { base.add(0x1000) }, core::ptr::null()).as_ptr(), core::ptr::null_mut()), E_INVALIDARG);
        assert_eq!(XDspSubmitCommand(fx, fcmd(base, outside.as_ptr() as *mut f32, core::ptr::null()).as_ptr(), core::ptr::null_mut()), E_INVALIDARG);
        assert_eq!(XDspSubmitCommand(fx, fcmd(core::ptr::null(), unsafe { base.add(0x1000) }, core::ptr::null()).as_ptr(), core::ptr::null_mut()), E_INVALIDARG);
        assert_eq!(XDspSubmitCommandWithEnvelope(fx, fcmd(base, unsafe { base.add(0x1000) }, unsafe { base.add(0x400) }).as_ptr(), outside.as_ptr(), core::ptr::null_mut()), S_OK);
        let xw: Vec<f32> = x.iter().zip(&w).map(|(a, b)| a * b).collect();
        let want = dft(&xw);
        let tol = 1e-3 * n as f64;
        for k in 1..n / 2 {
            assert!((mem[0x1000 + 2 * k] as f64 - want[k].0).abs() < tol && (mem[0x1000 + 2 * k + 1] as f64 - want[k].1).abs() < tol, "bin {k}");
        }
        let base = mem.as_mut_ptr();
        assert_eq!(XDspSubmitCommand(ix, fcmd(unsafe { base.add(0x1000) }, unsafe { base.add(0x2000) }, outside.as_ptr()).as_ptr(), core::ptr::null_mut()), S_OK);
        for i in 0..n {
            assert!((mem[0x2000 + i] - xw[i]).abs() < 1e-4, "sample {i}");
        }
        for _ in 0..20 {
            let base = mem.as_mut_ptr();
            assert_eq!(XDspSubmitCommand(fx, fcmd(base, unsafe { base.add(0x1000) }, core::ptr::null()).as_ptr(), core::ptr::null_mut()), S_OK);
        }
        unsafe { assert_eq!((*st, *st.add(1)), (1, 1)) };
        assert_eq!(XDspDeactivate(fx), S_OK);
        assert_eq!(XDspDeactivate(ix), S_OK);
        assert_eq!(XDspDisconnect(cl), S_OK);
    }

    #[test]
    fn fft_stream_output_is_a_valid_convolution_impulse_response() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let b = 0x200usize;
        let parts = 9usize;
        let mut mem = vec![0.0f32; 0x10000];
        let mut cl: *mut Client = core::ptr::null_mut();
        assert_eq!(XDspConnect(mem.as_mut_ptr() as *mut c_void, 0x40000, 2, &mut cl), S_OK);
        let (mut st, mut fx) = (core::ptr::null_mut(), core::ptr::null_mut());
        assert_eq!(XDspActivate(cl, tdesc(KIND_FFT, 0, (2 * b) as u32, 1).as_ptr(), &mut st, &mut fx), S_OK);
        let h: Vec<f32> = (0..b * 4 + 33).map(|i| ((i as f32) * 0.21).sin() * (-(i as f32) / 500.0).exp()).collect();
        let mut ir = Vec::new();
        for p in 0..parts {
            for i in 0..2 * b {
                mem[i] = if i < b { *h.get(p * b + i).unwrap_or(&0.0) } else { 0.0 };
            }
            let base = mem.as_mut_ptr();
            assert_eq!(XDspSubmitCommand(fx, fcmd(base, unsafe { base.add(0x1000) }, core::ptr::null()).as_ptr(), core::ptr::null_mut()), S_OK);
            ir.extend_from_slice(&mem[0x1000..0x1000 + 2 * b]);
        }
        assert_eq!(ir, ir_from_time(&h, b, parts));
        let mut cx: *mut Context = core::ptr::null_mut();
        assert_eq!(XDspActivate(cl, desc(0, b as u32, 1, (2 * b * parts) as u32, ir.as_ptr()).as_ptr(), &mut st, &mut cx), S_OK);
        let blocks = 8;
        let input: Vec<f32> = (0..b * blocks).map(|i| ((i * 104729 % 997) as f32 / 498.5) - 1.0).collect();
        mem[0x4000..0x4000 + b * blocks].copy_from_slice(&input);
        let base = mem.as_mut_ptr();
        for k in 0..blocks {
            assert_eq!(XDspSubmitCommand(cx, cmd(unsafe { base.add(0x4000 + k * b) }, unsafe { base.add(0x8000 + k * b) }).as_ptr(), core::ptr::null_mut()), S_OK);
        }
        let want = reference(&input, &h);
        let err = mem[0x8000..0x8000 + b * blocks].iter().zip(&want).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
        assert!(err < 1e-3, "max err {err}");
        assert_eq!(XDspDeactivate(fx), S_OK);
        assert_eq!(XDspDeactivate(cx), S_OK);
        assert_eq!(XDspDisconnect(cl), S_OK);
    }

    #[test]
    fn fft_roundtrip() {
        let fft = Fft::new(16);
        let t: Vec<f32> = (0..16).map(|i| (i as f32 * 0.7).sin()).collect();
        let mut w = vec![C32 { re: 0.0, im: 0.0 }; 16];
        let mut s = vec![C32 { re: 0.0, im: 0.0 }; 8];
        pack_forward(&fft, &t, &mut w, &mut s);
        unpack_inverse(&fft, &s, &mut w);
        for i in 0..16 {
            assert!((w[i].re - t[i]).abs() < 1e-5);
        }
    }

    #[test]
    fn connect_limits() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let mut buf = vec![0u8; 0x10000];
        let mut c: *mut Client = core::ptr::null_mut();
        assert_eq!(XDspConnect(buf.as_mut_ptr() as _, 0x10000, 0x80, core::ptr::null_mut()), E_POINTER);
        assert_eq!(XDspConnect(core::ptr::null_mut(), 0x10000, 0x80, &mut c), E_INVALIDARG);
        assert_eq!(XDspConnect(buf.as_mut_ptr() as _, 0xFFFF, 0x80, &mut c), E_INVALIDARG);
        assert_eq!(XDspConnect(buf.as_mut_ptr() as _, 0x10000, 0x81, &mut c), E_INVALIDARG);
        assert_eq!(XDspConnect(buf.as_mut_ptr() as _, 0x10000, 0x80, &mut c), S_OK);
        let mut c2: *mut Client = core::ptr::null_mut();
        assert_eq!(XDspConnect(buf.as_mut_ptr() as _, 0x10000, 1, &mut c2), XDSP_E_TOO_MANY_ACTIVATIONS);
        assert_eq!(XDspDisconnect(c), S_OK);
        assert_eq!(XDspDisconnect(core::ptr::null_mut()), E_INVALIDARG);
    }

    #[test]
    fn connect_with_maximum_stream_limit_validates_like_console() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let mut buf = vec![0u8; 0x10000];
        let mut c: *mut Client = core::ptr::null_mut();
        let m = buf.as_mut_ptr() as *mut c_void;
        assert_eq!(XDspConnectWithMaximumStreamLimit(m, 0x10000, 4, 0x101, 2, &mut c), E_INVALIDARG);
        assert_eq!(XDspConnectWithMaximumStreamLimit(m, 0x10000, 4, 0x100, 3, &mut c), E_INVALIDARG);
        assert_eq!(XDspConnectWithMaximumStreamLimit(m, 0x10000, 4, 0x101, 2, core::ptr::null_mut()), E_INVALIDARG);
        assert_eq!(XDspConnectWithMaximumStreamLimit(m, 0x10000, 4, 8, 0, core::ptr::null_mut()), E_POINTER);
        assert_eq!(XDspConnectWithMaximumStreamLimit(m, 0x10000, 4, 8, 0, &mut c), S_OK);
        assert_eq!(unsafe { ((*c).max_streams, (*c).priority) }, (8, 0));
        assert_eq!(XDspDisconnect(c), S_OK);
        assert_eq!(XDspConnect(m, 0x10000, 4, &mut c), S_OK);
        assert_eq!(unsafe { ((*c).max_streams, (*c).priority) }, (MAX_STREAMS, MAX_PRIORITY));
        assert_eq!(XDspDisconnect(c), S_OK);
    }

    #[test]
    fn activate_validation() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let mut buf = vec![0u8; 0x10000];
        let mut c: *mut Client = core::ptr::null_mut();
        assert_eq!(XDspConnect(buf.as_mut_ptr() as _, 0x10000, 2, &mut c), S_OK);
        let ir = vec![0.0f32; 2 * 0x200 * 16 * 2];
        let mut st: *mut u32 = core::ptr::null_mut();
        let mut x: *mut Context = core::ptr::null_mut();
        assert_eq!(XDspActivate(c, desc(0, 0x200, 1, 2 * 0x200 * 16, ir.as_ptr()).as_ptr(), core::ptr::null_mut(), &mut x), E_POINTER);
        assert_eq!(XDspActivate(c, desc(0, 0x300, 1, 2 * 0x200 * 16, ir.as_ptr()).as_ptr(), &mut st, &mut x), E_INVALIDARG);
        assert_eq!(XDspActivate(c, desc(0xA, 0x200, 1, 2 * 0x200 * 16, ir.as_ptr()).as_ptr(), &mut st, &mut x), E_INVALIDARG);
        assert_eq!(XDspActivate(c, desc(4, 0x200, 1, 2 * 0x200 * 16, ir.as_ptr()).as_ptr(), &mut st, &mut x), E_INVALIDARG);
        assert_eq!(XDspActivate(c, desc(0, 0x200, 1, 2 * 0x200 * 8, ir.as_ptr()).as_ptr(), &mut st, &mut x), XDSP_E_IR_SIZE);
        assert_eq!(XDspActivate(c, desc(0xA, 0x200, 2, 2 * 0x200 * 16, ir.as_ptr()).as_ptr(), &mut st, &mut x), S_OK);
        assert!(!st.is_null());
        let mut y: *mut Context = core::ptr::null_mut();
        assert_eq!(XDspActivate(c, desc(0, 0x200, 1, 2 * 0x200 * 16, ir.as_ptr()).as_ptr(), &mut st, &mut y), S_OK);
        let mut z: *mut Context = core::ptr::null_mut();
        assert_eq!(XDspActivate(c, desc(0, 0x200, 1, 2 * 0x200 * 16, ir.as_ptr()).as_ptr(), &mut st, &mut z), XDSP_E_TOO_MANY_CONTEXTS);
        assert_eq!(XDspDisconnect(c), XDSP_E_CONTEXTS_ACTIVE);
        assert_eq!(XDspDeactivate(x), S_OK);
        assert_eq!(XDspDeactivate(y), S_OK);
        assert_eq!(XDspDisconnect(c), S_OK);
    }

    fn reference(x: &[f32], h: &[f32]) -> Vec<f32> {
        (0..x.len()).map(|n| (0..h.len()).filter(|&k| k <= n).map(|k| h[k] * x[n - k]).sum()).collect()
    }

    #[test]
    fn convolution_matches_direct_and_drops_metadata() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let b = 0x200usize;
        let parts = 10usize;
        let h: Vec<f32> = (0..b * 3 + 17).map(|i| ((i as f32) * 0.37).sin() * (-(i as f32) / 600.0).exp()).collect();
        let mut ir = ir_from_time(&h, b, parts - 1);
        let mut meta = vec![0.0f32; 2 * b];
        meta[0] = f32::from_bits(IR_METADATA_MARKER);
        meta[1] = f32::NAN;
        ir.extend_from_slice(&meta);
        let mut mem = vec![0.0f32; 0x10000];
        let mut c: *mut Client = core::ptr::null_mut();
        assert_eq!(XDspConnect(mem.as_mut_ptr() as _, 0x40000, 1, &mut c), S_OK);
        let mut st: *mut u32 = core::ptr::null_mut();
        let mut x: *mut Context = core::ptr::null_mut();
        assert_eq!(XDspActivate(c, desc(0, b as u32, 1, (2 * b * parts) as u32, ir.as_ptr()).as_ptr(), &mut st, &mut x), S_OK);
        let blocks = 6;
        let input: Vec<f32> = (0..b * blocks).map(|i| ((i * 7919 % 1000) as f32 / 500.0) - 1.0).collect();
        mem[..b * blocks].copy_from_slice(&input);
        let outside = vec![0.0f32; b];
        assert_eq!(XDspSubmitCommand(x, cmd(input.as_ptr(), mem[0x8000..].as_mut_ptr()).as_ptr(), core::ptr::null_mut()), E_INVALIDARG);
        assert_eq!(XDspSubmitCommand(x, cmd(mem.as_ptr(), outside.as_ptr() as *mut f32).as_ptr(), core::ptr::null_mut()), E_INVALIDARG);
        assert_eq!(unsafe { *st }, 0);
        let base = mem.as_mut_ptr();
        for k in 0..blocks {
            let mut seq = 0;
            assert_eq!(XDspSubmitCommand(x, cmd(unsafe { base.add(k * b) }, unsafe { base.add(0x8000 + k * b) }).as_ptr(), &mut seq), S_OK);
            assert_eq!(seq, k as u32 + 1);
            unsafe {
                assert_eq!(*st, k as u32 + 1);
                assert_eq!(*st.add(1), k as u32 + 1);
            }
        }
        let output = mem[0x8000..0x8000 + b * blocks].to_vec();
        let want = reference(&input, &h);
        let err = output.iter().zip(want.iter()).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
        assert!(err < 1e-3, "max err {err}");
        assert!(output.iter().all(|v| v.is_finite()));
        assert_eq!(XDspDeactivate(x), S_OK);
        assert_eq!(XDspDisconnect(c), S_OK);
    }

    #[test]
    fn stereo_uses_two_irs() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let b = 0x200usize;
        let parts = 9usize;
        let mut h0 = vec![0.0f32; b];
        h0[0] = 1.0;
        let mut h1 = vec![0.0f32; b];
        h1[3] = 0.5;
        let mut ir = ir_from_time(&h0, b, parts);
        ir.extend(ir_from_time(&h1, b, parts));
        let mut mem = vec![0.0f32; 0x4000];
        let mut c: *mut Client = core::ptr::null_mut();
        assert_eq!(XDspConnect(mem.as_mut_ptr() as _, 0x10000, 1, &mut c), S_OK);
        let mut st: *mut u32 = core::ptr::null_mut();
        let mut x: *mut Context = core::ptr::null_mut();
        assert_eq!(XDspActivate(c, desc(0xA, b as u32, 2, (2 * b * parts) as u32, ir.as_ptr()).as_ptr(), &mut st, &mut x), S_OK);
        mem[10] = 1.0;
        mem[b + 10] = 1.0;
        let base = mem.as_mut_ptr();
        assert_eq!(XDspSubmitCommand(x, cmd(base, unsafe { base.add(0x2000) }).as_ptr(), core::ptr::null_mut()), S_OK);
        let out = mem[0x2000..0x2000 + 2 * b].to_vec();
        assert!((out[10] - 1.0).abs() < 1e-4);
        assert!((out[b + 13] - 0.5).abs() < 1e-4);
        assert!(out[b + 10].abs() < 1e-4);
        assert_eq!(XDspDeactivate(x), S_OK);
        assert_eq!(XDspDisconnect(c), S_OK);
    }

    #[test]
    fn submit_with_envelope_scales_each_ir_partition() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let b = 0x200usize;
        let parts = 9usize;
        let mut h = vec![0.0f32; b * 3];
        h[0] = 1.0;
        h[b + 5] = 0.5;
        h[2 * b + 7] = 0.25;
        let ir = ir_from_time(&h, b, parts);
        let mut mem = vec![0.0f32; 0x4000];
        let mut c: *mut Client = core::ptr::null_mut();
        assert_eq!(XDspConnect(mem.as_mut_ptr() as _, 0x10000, 1, &mut c), S_OK);
        let mut st: *mut u32 = core::ptr::null_mut();
        let mut x: *mut Context = core::ptr::null_mut();
        assert_eq!(XDspActivate(c, desc(0, b as u32, 1, (2 * b * parts) as u32, ir.as_ptr()).as_ptr(), &mut st, &mut x), S_OK);
        let base = mem.as_mut_ptr();
        let (inp, out, env) = (0x1000usize, 0x2000usize, 0usize);
        let outside = vec![1.0f32; 16];
        assert_eq!(XDspSubmitCommandWithEnvelope(x, cmd(unsafe { base.add(inp) }, unsafe { base.add(out) }).as_ptr(), outside.as_ptr(), core::ptr::null_mut()), E_INVALIDARG);
        assert_eq!(unsafe { *st }, 0);
        for (k, g) in [2.0f32, 0.0, 4.0].iter().enumerate() {
            mem[env + k] = *g;
        }
        mem[inp + 1] = 1.0;
        let base = mem.as_mut_ptr();
        for k in 0..3 {
            let mut seq = 0;
            assert_eq!(XDspSubmitCommandWithEnvelope(x, cmd(unsafe { base.add(inp + k * b) }, unsafe { base.add(out + k * b) }).as_ptr(), unsafe { base.add(env) }, &mut seq), S_OK);
            assert_eq!(seq, k as u32 + 1);
        }
        let output = mem[out..out + 3 * b].to_vec();
        assert!((output[1] - 2.0).abs() < 1e-4, "{}", output[1]);
        assert!(output[b + 6].abs() < 1e-4, "{}", output[b + 6]);
        assert!((output[2 * b + 8] - 1.0).abs() < 1e-4, "{}", output[2 * b + 8]);
        let base = mem.as_mut_ptr();
        for _ in 3..8 {
            assert_eq!(XDspSubmitCommand(x, cmd(unsafe { base.add(inp) }, unsafe { base.add(out) }).as_ptr(), core::ptr::null_mut()), S_OK);
        }
        mem[inp..inp + b].iter_mut().for_each(|v| *v = 0.0);
        mem[inp + 1] = 1.0;
        let base = mem.as_mut_ptr();
        assert_eq!(XDspSubmitCommand(x, cmd(unsafe { base.add(inp) }, unsafe { base.add(out) }).as_ptr(), core::ptr::null_mut()), S_OK);
        assert!((mem[out + 1] - 1.0).abs() < 1e-4, "{}", mem[out + 1]);
        assert_eq!(XDspDeactivate(x), S_OK);
        assert_eq!(XDspDisconnect(c), S_OK);
    }
}
