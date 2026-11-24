use crate::passcap::*;
use core::ffi::c_void;

#[test]
fn passcap_parse_selector() {
    assert!(parse("").is_none());
    assert!(parse("0").is_none());
    let s = parse("t:8, t:38,t=44;n:2,mb:900,caps:50,chain:0,raw,bogus").unwrap();
    assert_eq!(s.secs, vec![8.0, 38.0, 44.0]);
    assert_eq!(s.frames, 2);
    assert_eq!(s.budget_mb, 900);
    assert_eq!(s.max_caps, 50);
    assert_eq!(s.chain_every, 0);
    assert!(s.raw && !s.file && !s.all);
    assert_eq!(s.unknown, vec!["bogus".to_string()]);
    let s = parse("42,f:9000,file").unwrap();
    assert_eq!(s.secs, vec![42.0]);
    assert_eq!(s.presents, vec![9000]);
    assert!(s.file);
    assert!(parse("1").unwrap().file);
    assert_eq!(parse("n:99").unwrap().frames, 8);
}

#[test]
fn passcap_ps_ms_trigger_logic() {
    let s = parse("ps_ms:ba37366c685cb01e:10,n:4").unwrap();
    assert_eq!(s.ps_ms, Some((0xba37366c685cb01e, 10.0)));
    assert_eq!(s.frames, 4);
    assert!(s.secs.is_empty() && !s.file && s.unknown.is_empty());
    assert_eq!(parse("ps_ms:0xba37366c685cb01e:2.5").unwrap().ps_ms, Some((0xba37366c685cb01e, 2.5)));
    for bad in ["ps_ms:zz:10", "ps_ms:ba37366c685cb01e:0", "ps_ms:ba37366c685cb01e", "ps_ms:ba37366c685cb01e:-1", "ps_ms"] {
        let s = parse(bad).unwrap();
        assert_eq!(s.ps_ms, None, "{bad}");
        assert_eq!(s.unknown.len(), 1, "{bad}");
    }
    let t = Some((0xba37366c685cb01e, 10.0));
    assert!(slow_call(0xba37366c685cb01e, 622.0, t));
    assert!(!slow_call(0xba37366c685cb01e, 10.0, t));
    assert!(!slow_call(0xba37366c685cb01e, 3.0, t));
    assert!(!slow_call(0x985ca0d088a1901a, 622.0, t));
    assert!(!slow_call(0xba37366c685cb01e, 622.0, None));
    assert!(slow_due(true, true, 0, SLOW_MAX_SHOTS));
    assert!(!slow_due(true, false, 0, SLOW_MAX_SHOTS));
    assert!(!slow_due(false, true, 0, SLOW_MAX_SHOTS));
    assert!(!slow_due(true, true, SLOW_MAX_SHOTS, SLOW_MAX_SHOTS));
}

#[test]
fn passcap_take_due_fires_once_per_entry() {
    let l = [8.0, 38.0, 44.0];
    let mut i = 0;
    assert!(!take_due(&l, &mut i, 7.9));
    assert!(take_due(&l, &mut i, 8.0));
    assert!(!take_due(&l, &mut i, 9.0));
    assert!(take_due(&l, &mut i, 50.0));
    assert_eq!(i, 3);
    assert!(!take_due(&l, &mut i, 99.0));
}

#[test]
fn passcap_typed_formats() {
    assert_eq!(typed_format(27), 28);
    assert_eq!(typed_format(26), 26);
    assert_eq!(typed_format(60), 61);
    assert_eq!(typed_format(19), 20);
    assert!(crate::rtcap::layout(typed_format(9)).is_some());
}

#[test]
fn passcap_stats_rgba8_black_and_zero_fractions() {
    let (w, h, pitch) = (4usize, 2usize, 256usize);
    let mut d = vec![0u8; pitch * h];
    for x in 0..w {
        d[x * 4 + 3] = 255;
    }
    d[pitch + 4] = 255;
    d[pitch + 4 + 3] = 255;
    let s = stats("rgba8", 4, &d, pitch, w, h, 4);
    assert_eq!(s.pixels, 8);
    assert_eq!(s.max[0], 1.0);
    assert_eq!(s.min[1], 0.0);
    assert!((s.zero_px - 7.0 / 8.0).abs() < 1e-9);
    assert!((s.zero[0] - 7.0 / 8.0).abs() < 1e-9);
    assert!(!s.black());
    let z = vec![0u8; pitch * h];
    let s = stats("rgba8", 4, &z, pitch, w, h, 4);
    assert!(s.black());
    assert!(s.flags().contains("BLACK"));
}

#[test]
fn passcap_stats_counts_nan_and_inf() {
    let mut d = Vec::new();
    for h in [0x7e00u16, 0x7c00, 0x3c00, 0x0000] {
        d.extend_from_slice(&h.to_le_bytes());
        d.extend_from_slice(&0x3800u16.to_le_bytes());
        d.extend_from_slice(&0u16.to_le_bytes());
        d.extend_from_slice(&0x3c00u16.to_le_bytes());
    }
    let s = stats("rgba16f", 4, &d, 32, 4, 1, 8);
    assert_eq!(s.nan, 1);
    assert_eq!(s.inf, 1);
    assert_eq!(s.max[0], 1.0);
    assert_eq!(s.min[1], 0.5);
    assert!(s.flags().contains("NAN") && s.flags().contains("INF"));
}

#[test]
fn passcap_stats_r11g11b10() {
    let one = 15u32 << 6;
    let v: u32 = one | (one << 11) | ((15u32 << 5) << 22);
    let d = v.to_le_bytes();
    let s = stats("r11g11b10f", 3, &d, 4, 1, 1, 4);
    assert_eq!((s.max[0], s.max[1], s.max[2]), (1.0, 1.0, 1.0));
    assert!(s.uniform());
}

#[repr(C)]
struct G(u32, u16, u16, [u8; 8]);
const IID_DEVICE: G = G(0x189819f1, 0x1db6, 0x4b57, [0xbe, 0x54, 0x18, 0x21, 0x33, 0x9b, 0x85, 0xf7]);
const IID_QUEUE: G = G(0x0ec870a6, 0x5d7e, 0x4c22, [0x8c, 0xfc, 0x5b, 0xaa, 0xe0, 0x76, 0x16, 0xed]);
const IID_ALLOCATOR: G = G(0x6102dee4, 0xaf59, 0x4b09, [0xb9, 0x99, 0xb4, 0x4d, 0x73, 0xf0, 0x9b, 0x24]);
const IID_LIST: G = G(0x5b160d0f, 0xac1b, 0x4185, [0x8b, 0xa8, 0xb3, 0xae, 0x42, 0xa5, 0xa4, 0x55]);
const IID_RESOURCE: G = G(0x696442be, 0xa72e, 0x4059, [0xbc, 0x79, 0x5b, 0x5c, 0x98, 0x04, 0x0f, 0xad]);
const IID_FENCE: G = G(0x0a753dcf, 0xc4d8, 0x4b91, [0xad, 0xf6, 0xbe, 0x5a, 0x60, 0xd9, 0x5a, 0x76]);
const IID_HEAP: G = G(0x8efb471d, 0x616c, 0x4f49, [0x90, 0xf7, 0x12, 0x7b, 0xb7, 0x63, 0xfa, 0x51]);
const IID_FACTORY4: G = G(0x1bc6ea02, 0xef36, 0x464f, [0xbf, 0x0c, 0x21, 0xca, 0x39, 0xe5, 0x16, 0x8a]);
const IID_ADAPTER: G = G(0x2411e7e1, 0x12ac, 0x4ccf, [0xbd, 0x14, 0x97, 0x98, 0xe8, 0x53, 0x4d, 0xc0]);
const IID_DEBUG: G = G(0x344488b7, 0x6846, 0x474b, [0xb9, 0x89, 0xf0, 0x27, 0x44, 0x82, 0x45, 0xe0]);
const IID_INFO_QUEUE: G = G(0x0742a90b, 0xc387, 0x483f, [0xb9, 0x46, 0x30, 0xa7, 0xe4, 0xe6, 0x14, 0x58]);

extern "system" {
    fn LoadLibraryW(p: *const u16) -> *mut c_void;
    fn GetProcAddress(h: *mut c_void, n: *const u8) -> *mut c_void;
}

unsafe fn slot(o: usize, i: usize) -> usize {
    *(*(o as *const *const usize)).add(i)
}
macro_rules! c {
    ($o:expr, $i:expr, ($($t:ty),*) -> $r:ty $(, $a:expr)*) => {{
        let o = $o as usize;
        let f: unsafe extern "system" fn(usize $(, $t)*) -> $r = core::mem::transmute(slot(o, $i));
        f(o $(, $a)*)
    }};
}

struct Dev {
    dev: usize,
    q: usize,
    a: usize,
    l: usize,
    f: usize,
    fv: u64,
    info: usize,
}

unsafe fn host(dll: &str, name: &[u8]) -> usize {
    let w: Vec<u16> = dll.encode_utf16().chain(Some(0)).collect();
    let h = LoadLibraryW(w.as_ptr());
    assert!(!h.is_null(), "{dll}");
    GetProcAddress(h, name.as_ptr()) as usize
}

unsafe fn open() -> Dev {
    let warp = std::env::var("TOMBOX_HW_ADAPTER").map(|v| v.eq_ignore_ascii_case("warp")).unwrap_or(false);
    let dbg = host("d3d12.dll", b"D3D12GetDebugInterface\0");
    if dbg != 0 && std::env::var("TOMBOX_PASSCAP_TEST_DEBUG").map(|v| v != "0").unwrap_or(true) {
        let f: unsafe extern "system" fn(*const G, *mut usize) -> i32 = core::mem::transmute(dbg);
        let mut d = 0usize;
        if f(&IID_DEBUG, &mut d) >= 0 && d != 0 {
            c!(d, 3, () -> ());
        }
    }
    let mut adapter = 0usize;
    if warp {
        let cf: unsafe extern "system" fn(u32, *const G, *mut usize) -> i32 = core::mem::transmute(host("dxgi.dll", b"CreateDXGIFactory2\0"));
        let mut fac = 0usize;
        assert!(cf(0, &IID_FACTORY4, &mut fac) >= 0);
        assert!(c!(fac, 27, (*const G, *mut usize) -> i32, &IID_ADAPTER, &mut adapter) >= 0);
    }
    let create: unsafe extern "system" fn(usize, u32, *const G, *mut usize) -> i32 = core::mem::transmute(host("d3d12.dll", b"D3D12CreateDevice\0"));
    let mut dev = 0usize;
    let hr = create(adapter, 0xb000, &IID_DEVICE, &mut dev);
    assert!(hr >= 0 && dev != 0, "D3D12CreateDevice hr={hr:#010x}");
    let mut info = 0usize;
    let _ = c!(dev, 0, (*const G, *mut usize) -> i32, &IID_INFO_QUEUE, &mut info);
    let qd = [0u32; 4];
    let mut q = 0usize;
    assert!(c!(dev, 8, (*const u32, *const G, *mut usize) -> i32, qd.as_ptr(), &IID_QUEUE, &mut q) >= 0);
    let mut a = 0usize;
    assert!(c!(dev, 9, (u32, *const G, *mut usize) -> i32, 0, &IID_ALLOCATOR, &mut a) >= 0);
    let mut l = 0usize;
    assert!(c!(dev, 12, (u32, u32, usize, usize, *const G, *mut usize) -> i32, 0, 0, a, 0, &IID_LIST, &mut l) >= 0);
    let mut f = 0usize;
    assert!(c!(dev, 36, (u64, u32, *const G, *mut usize) -> i32, 0, 0, &IID_FENCE, &mut f) >= 0);
    Dev { dev, q, a, l, f, fv: 0, info }
}

impl Dev {
    unsafe fn run(&mut self) {
        assert!(c!(self.l, 9, () -> i32) >= 0, "Close failed");
        let ls = [self.l];
        c!(self.q, 10, (u32, *const usize) -> (), 1, ls.as_ptr());
        self.fv += 1;
        c!(self.q, 14, (usize, u64) -> i32, self.f, self.fv);
        let t = std::time::Instant::now();
        while c!(self.f, 8, () -> u64) < self.fv {
            assert!(t.elapsed().as_secs() < 20, "GPU timeout");
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert!(c!(self.a, 8, () -> i32) >= 0);
        assert!(c!(self.l, 10, (usize, usize) -> i32, self.a, 0) >= 0);
    }
    unsafe fn errors(&self) -> Vec<String> {
        if self.info == 0 {
            return Vec::new();
        }
        let n = c!(self.info, 8, () -> u64);
        let mut out = Vec::new();
        for i in 0..n {
            let mut len = 0usize;
            if c!(self.info, 5, (u64, *mut u8, *mut usize) -> i32, i, core::ptr::null_mut(), &mut len) < 0 || len < 32 {
                continue;
            }
            let mut b = vec![0u8; len];
            if c!(self.info, 5, (u64, *mut u8, *mut usize) -> i32, i, b.as_mut_ptr(), &mut len) < 0 {
                continue;
            }
            let sev = u32::from_le_bytes(b[4..8].try_into().unwrap());
            let p = u64::from_le_bytes(b[16..24].try_into().unwrap()) as *const u8;
            let dl = u64::from_le_bytes(b[24..32].try_into().unwrap()) as usize;
            if sev <= 1 && !p.is_null() {
                out.push(String::from_utf8_lossy(core::slice::from_raw_parts(p, dl.saturating_sub(1))).into_owned());
            }
        }
        out
    }
    unsafe fn texture(&self, w: u32, h: u32, fmt: u32, samples: u32, flags: u32, state: u32, clear: Option<&[u8]>) -> usize {
        let mut d = [0u8; 0x38];
        d[0..4].copy_from_slice(&3u32.to_le_bytes());
        d[0x10..0x18].copy_from_slice(&(w as u64).to_le_bytes());
        d[0x18..0x1c].copy_from_slice(&h.to_le_bytes());
        d[0x1c..0x1e].copy_from_slice(&1u16.to_le_bytes());
        d[0x1e..0x20].copy_from_slice(&1u16.to_le_bytes());
        d[0x20..0x24].copy_from_slice(&fmt.to_le_bytes());
        d[0x24..0x28].copy_from_slice(&samples.to_le_bytes());
        d[0x30..0x34].copy_from_slice(&flags.to_le_bytes());
        let hp: [u32; 5] = [1, 0, 0, 1, 1];
        let mut r = 0usize;
        let cv = clear.map(|c| {
            let mut v = [0u8; 20];
            v[0..4].copy_from_slice(&fmt.to_le_bytes());
            v[4..4 + c.len()].copy_from_slice(c);
            v
        });
        let hr = c!(self.dev, 27, (*const u32, u32, *const u8, u32, *const u8, *const G, *mut usize) -> i32,
            hp.as_ptr(), 0, d.as_ptr(), state, cv.as_ref().map_or(core::ptr::null(), |v| v.as_ptr()), &IID_RESOURCE, &mut r);
        assert!(hr >= 0, "texture fmt={fmt} samples={samples} hr={hr:#010x}");
        r
    }
    unsafe fn upload(&mut self, tex: usize, w: usize, h: usize, fmt: u32, bpp: usize, pattern: &[u8]) {
        let pitch = (w * bpp).div_ceil(256) * 256;
        let size = (pitch * h) as u64;
        let mut bd = [0u8; 0x38];
        bd[0..4].copy_from_slice(&1u32.to_le_bytes());
        bd[0x10..0x18].copy_from_slice(&size.to_le_bytes());
        bd[0x18..0x1c].copy_from_slice(&1u32.to_le_bytes());
        bd[0x1c..0x1e].copy_from_slice(&1u16.to_le_bytes());
        bd[0x1e..0x20].copy_from_slice(&1u16.to_le_bytes());
        bd[0x24..0x28].copy_from_slice(&1u32.to_le_bytes());
        bd[0x2c..0x30].copy_from_slice(&1u32.to_le_bytes());
        let hp: [u32; 5] = [2, 0, 0, 1, 1];
        let mut b = 0usize;
        assert!(c!(self.dev, 27, (*const u32, u32, *const u8, u32, *const u8, *const G, *mut usize) -> i32, hp.as_ptr(), 0, bd.as_ptr(), 0xac3, core::ptr::null(), &IID_RESOURCE, &mut b) >= 0);
        let mut p: *mut u8 = core::ptr::null_mut();
        assert!(c!(b, 8, (u32, *const u8, *mut *mut u8) -> i32, 0, core::ptr::null(), &mut p) >= 0);
        for y in 0..h {
            core::ptr::copy_nonoverlapping(pattern.as_ptr().add(y * w * bpp), p.add(y * pitch), w * bpp);
        }
        let mut src = [0u8; 48];
        src[0..8].copy_from_slice(&(b as u64).to_le_bytes());
        src[8..12].copy_from_slice(&1u32.to_le_bytes());
        src[24..28].copy_from_slice(&fmt.to_le_bytes());
        src[28..32].copy_from_slice(&(w as u32).to_le_bytes());
        src[32..36].copy_from_slice(&(h as u32).to_le_bytes());
        src[36..40].copy_from_slice(&1u32.to_le_bytes());
        src[40..44].copy_from_slice(&(pitch as u32).to_le_bytes());
        let mut dst = [0u8; 48];
        dst[0..8].copy_from_slice(&(tex as u64).to_le_bytes());
        c!(self.l, 16, (*const u8, u32, u32, u32, *const u8, *const u8) -> (), dst.as_ptr(), 0, 0, 0, src.as_ptr(), core::ptr::null());
        self.run();
        let _keep = b;
    }
    unsafe fn barrier(&self, res: usize, before: u32, after: u32) {
        let b: [u64; 4] = [0, res as u64, 0xffff_ffffu64 | ((before as u64) << 32), after as u64];
        c!(self.l, 26, (u32, *const [u64; 4]) -> (), 1, &b);
    }
    unsafe fn heap_start(&self, kind: u32) -> usize {
        let hd: [u32; 4] = [kind, 1, 0, 0];
        let mut h = 0usize;
        assert!(c!(self.dev, 14, (*const u32, *const G, *mut usize) -> i32, hd.as_ptr(), &IID_HEAP, &mut h) >= 0);
        let mut out = 0usize;
        c!(h, 9, (*mut usize) -> *mut usize, &mut out);
        out
    }
}

unsafe fn capture_exact(d: &mut Dev, tex: usize, state: u32) -> (Cap, Vec<u8>) {
    let cap = inject(d.dev, d.l, tex, state, "RT0", 1 << 30).expect("inject");
    d.run();
    let data = read_cap(&cap).expect("map");
    (cap, data)
}

fn hw_on() -> bool {
    std::env::var("TOMBOX_HW_ADAPTER").is_ok() || std::env::var("TOMBOX_PASSCAP_HW").is_ok()
}

#[test]
#[ignore = "needs a D3D12 device; run: TOMBOX_HW_ADAPTER=warp cargo test -p d3d12_xs --lib passcap_hw -- --ignored --test-threads=1"]
fn passcap_hw_rgba8_and_r8_patterns_read_back_exactly() {
    if !hw_on() {
        return;
    }
    unsafe {
        let mut d = open();
        for (fmt, bpp, w, h) in [(28u32, 4usize, 64usize, 32usize), (61, 1, 37, 19)] {
            let tex = d.texture(w as u32, h as u32, fmt, 1, 1, 0x400, None);
            let pattern: Vec<u8> = (0..w * h * bpp).map(|i| (i * 31 + i / 7) as u8).collect();
            d.upload(tex, w, h, fmt, bpp, &pattern);
            d.barrier(tex, 0x400, ST_RENDER_TARGET);
            let (cap, data) = capture_exact(&mut d, tex, ST_RENDER_TARGET);
            assert_eq!((cap.w, cap.h), (w, h));
            for y in 0..h {
                assert_eq!(&data[y * cap.pitch..y * cap.pitch + w * bpp], &pattern[y * w * bpp..(y + 1) * w * bpp], "fmt {fmt} row {y}");
            }
            let (ch, _, b, kind) = decode_kind(cap.fmt, cap.read_fmt).unwrap();
            let s = stats(kind, ch, &data, cap.pitch, w, h, b);
            assert_eq!(s.pixels as usize, w * h);
            d.barrier(tex, ST_RENDER_TARGET, 0x800);
            let (_, again) = capture_exact(&mut d, tex, 0x800);
            assert_eq!(again, data, "state restored and re-capturable");
            eprintln!("passcap hw: fmt {fmt} {w}x{h} pitch {} exact, {}", cap.pitch, s.line());
            release_cap(&cap);
        }
        eprintln!("passcap hw: debug info queue present = {}", d.info != 0);
        let e = d.errors();
        assert!(e.is_empty(), "debug layer errors: {e:#?}");
    }
}

#[test]
#[ignore = "needs a D3D12 device; run: TOMBOX_HW_ADAPTER=warp cargo test -p d3d12_xs --lib passcap_hw -- --ignored --test-threads=1"]
fn passcap_hw_msaa_r11g11b10_resolves_to_clear_color() {
    if !hw_on() {
        return;
    }
    unsafe {
        let mut d = open();
        let color = [0.5f32, 0.25, 1.0, 1.0];
        let cb: Vec<u8> = color.iter().flat_map(|f| f.to_le_bytes()).collect();
        let tex = d.texture(48, 24, 26, 2, 1, ST_RENDER_TARGET, Some(&cb));
        let h = d.heap_start(2);
        c!(d.dev, 20, (usize, *const u8, usize) -> (), tex, core::ptr::null(), h);
        c!(d.l, 48, (usize, *const f32, u32, *const u8) -> (), h, color.as_ptr(), 0, core::ptr::null());
        let (cap, data) = capture_exact(&mut d, tex, ST_RENDER_TARGET);
        assert_eq!(cap.samples, 2);
        let (ch, _, b, kind) = decode_kind(cap.fmt, cap.read_fmt).unwrap();
        let s = stats(kind, ch, &data, cap.pitch, cap.w, cap.h, b);
        assert_eq!((s.min[0], s.max[0], s.min[1], s.max[1], s.min[2], s.max[2]), (0.5, 0.5, 0.25, 0.25, 1.0, 1.0), "{}", s.line());
        release_cap(&cap);
        let tex2 = d.texture(16, 16, 40, 1, 2, ST_DEPTH_WRITE, Some(&0.75f32.to_le_bytes()));
        let hd = d.heap_start(3);
        c!(d.dev, 21, (usize, *const u8, usize) -> (), tex2, core::ptr::null(), hd);
        c!(d.l, 47, (usize, u32, f32, u8, u32, *const u8) -> (), hd, 1, 0.75, 0, 0, core::ptr::null());
        let (cap, data) = capture_exact(&mut d, tex2, ST_DEPTH_WRITE);
        let (ch, _, b, kind) = decode_kind(cap.fmt, cap.read_fmt).unwrap();
        let s = stats(kind, ch, &data, cap.pitch, cap.w, cap.h, b);
        assert_eq!((s.min[0], s.max[0]), (0.75, 0.75), "{}", s.line());
        release_cap(&cap);
        let ms_depth = d.texture(16, 16, 40, 2, 2, ST_DEPTH_WRITE, Some(&0.5f32.to_le_bytes()));
        assert!(inject(d.dev, d.l, ms_depth, ST_DEPTH_WRITE, "DSV", 1 << 30).unwrap_err().contains("multisampled depth"));
        let e = d.errors();
        assert!(e.is_empty(), "debug layer errors: {e:#?}");
    }
}

#[test]
fn passcap_ps_draw_trigger_logic() {
    use crate::passcap::{parse, parse_ps_draw, ps_draw_due, PS_DRAW_GAP_S, SLOW_MAX_SHOTS};
    assert_eq!(parse_ps_draw("a946cde5f859340b:60"), Some((0xa946cde5f859340b, 60.0)));
    assert_eq!(parse_ps_draw("0xa946cde5f859340b"), Some((0xa946cde5f859340b, 0.0)));
    assert_eq!(parse_ps_draw("zz:1"), None);
    assert_eq!(parse_ps_draw("a946:-1"), None);
    let s = parse("ps:a946cde5f859340b:60").unwrap();
    assert_eq!(s.ps_draw, Some((0xa946cde5f859340b, 60.0)));
    assert!(s.unknown.is_empty());
    let tg = Some((1u64, 60.0));
    assert!(!ps_draw_due(true, 59.9, tg, 0, 0.0));
    assert!(ps_draw_due(true, 60.0, tg, 0, 0.0));
    assert!(!ps_draw_due(false, 61.0, tg, 0, 0.0));
    assert!(!ps_draw_due(true, 61.0, tg, 1, 60.0));
    assert!(ps_draw_due(true, 60.0 + PS_DRAW_GAP_S, tg, 1, 60.0));
    assert!(!ps_draw_due(true, 99.0, tg, SLOW_MAX_SHOTS, 60.0));
    assert!(!ps_draw_due(true, 99.0, None, 0, 0.0));
}

#[test]
fn grid3d_text_reports_light_counts() {
    let (w, h, d, pitch) = (2usize, 1usize, 2usize, 256usize);
    let mut data = vec![0u8; pitch * h * d];
    let put = |data: &mut Vec<u8>, z: usize, x: usize, off: u32, y: u32| {
        let o = z * h * pitch + x * 8;
        data[o..o + 4].copy_from_slice(&off.to_le_bytes());
        data[o + 4..o + 8].copy_from_slice(&y.to_le_bytes());
    };
    put(&mut data, 0, 0, 5, 3 | (1 << 16));
    put(&mut data, 1, 1, 9, 300);
    let s = grid3d_text(&data, pitch, w, h, d, 17);
    assert!(s.starts_with("light grid 2x1x2 (4 texels): count lo16 max 300 "), "{s}");
    assert!(s.contains("over256 1 | count hi16 max 1 "), "{s}");
    assert!(s.ends_with("offset max 9"), "{s}");
    assert!(grid3d_text(&data, pitch, w, h, d, 28).contains("only R32G32_UINT"));
}
