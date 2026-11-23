use core::ffi::c_void;
use core::sync::atomic::{AtomicBool, Ordering};
use compat_common::record;

#[repr(C)]
#[derive(Clone, Copy)]
struct G(u32, u16, u16, [u8; 8]);

const IID_FACTORY4: G = G(0x1bc6ea02, 0xef36, 0x464f, [0xbf, 0x0c, 0x21, 0xca, 0x39, 0xe5, 0x16, 0x8a]);
const IID_ADAPTER3: G = G(0x645967a4, 0x1392, 0x4310, [0xa7, 0x98, 0x80, 0x53, 0xce, 0x3e, 0x93, 0xfd]);
const FEATURE_ARCHITECTURE1: u32 = 16;
const DEFAULT_MS: u64 = 2000;
const MIN_MS: u64 = 100;

extern "system" {
    fn GetModuleHandleA(name: *const u8) -> *mut c_void;
    fn LoadLibraryA(name: *const u8) -> *mut c_void;
    fn GetProcAddress(module: *mut c_void, name: *const u8) -> *mut c_void;
}

unsafe fn slot(o: *mut c_void, i: usize) -> usize {
    unsafe { *(*(o as *const *const usize)).add(i) }
}

pub fn parse_ms(v: Option<&str>) -> Option<u64> {
    let s = v?.trim();
    match s.to_ascii_lowercase().as_str() {
        "" | "0" | "off" | "false" | "no" => None,
        "on" | "true" | "yes" => Some(DEFAULT_MS),
        t => t.parse::<u64>().ok().filter(|&n| n > 0).map(|n| n.max(MIN_MS)),
    }
}

#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct MemInfo {
    pub budget: u64,
    pub usage: u64,
    pub avail_reserve: u64,
    pub reserved: u64,
}

impl MemInfo {
    pub fn from_raw(r: [u64; 4]) -> MemInfo {
        MemInfo { budget: r[0], usage: r[1], avail_reserve: r[2], reserved: r[3] }
    }
}

#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct Arch {
    pub uma: bool,
    pub cc_uma: bool,
    pub tile_based: bool,
}

impl Arch {
    pub fn from_raw(r: [u32; 5]) -> Arch {
        Arch { tile_based: r[1] != 0, uma: r[2] != 0, cc_uma: r[3] != 0 }
    }
}

#[derive(Clone, Copy, Default, Debug)]
pub struct Sample {
    pub local: Option<MemInfo>,
    pub non_local: Option<MemInfo>,
    pub make_resident: u64,
    pub make_resident_objs: u64,
    pub evict: u64,
    pub evict_objs: u64,
    pub committed_bytes: u64,
    pub heap_bytes: u64,
    pub title_counted: bool,
    pub placed_tracked: u64,
    pub placed_bytes: u64,
    pub upload_bytes: u64,
    pub upload_copies: u64,
    pub upload_lists: u64,
    pub cross_waits: u64,
}

fn mb(b: u64) -> f64 {
    b as f64 / (1u64 << 20) as f64
}

fn seg(name: &str, m: Option<MemInfo>) -> String {
    match m {
        Some(m) => format!("{name}: usage={:.1}MB budget={:.1}MB ({}) avail-for-reservation={:.1}MB reserved={:.1}MB", mb(m.usage), mb(m.budget), if m.usage > m.budget { "OVER" } else { "ok" }, mb(m.avail_reserve), mb(m.reserved)),
        None => format!("{name}: query failed"),
    }
}

pub fn line(n: u64, dt_ms: u64, s: &Sample, prev: &Sample) -> String {
    let d = |a: u64, b: u64| a.saturating_sub(b);
    let title = if s.title_counted {
        format!("committed={:.1}MB heaps={:.1}MB", mb(s.committed_bytes), mb(s.heap_bytes))
    } else {
        "committed/heaps=n/a (xmem title GPU charge not found)".to_string()
    };
    format!(
        "vram #{n} (+{dt_ms}ms): {} | {} | MakeResident +{} calls +{} objs, Evict +{} calls +{} objs | d3d12_xs live {title}, mirrored placed textures={} ({:.1}MB) | uploads +{} lists +{} copies +{:.1}MB, cross-queue waits +{}",
        seg("LOCAL", s.local), seg("NON_LOCAL", s.non_local),
        d(s.make_resident, prev.make_resident), d(s.make_resident_objs, prev.make_resident_objs), d(s.evict, prev.evict), d(s.evict_objs, prev.evict_objs),
        s.placed_tracked, mb(s.placed_bytes),
        d(s.upload_lists, prev.upload_lists), d(s.upload_copies, prev.upload_copies), mb(d(s.upload_bytes, prev.upload_bytes)), d(s.cross_waits, prev.cross_waits))
}

#[derive(Default)]
pub struct OverBudget {
    fired: [bool; 2],
}

impl OverBudget {
    pub fn check(&mut self, n: u64, s: &Sample, arch: Option<Arch>) -> Vec<String> {
        let mut out = Vec::new();
        for (i, (name, m)) in [("LOCAL", s.local), ("NON_LOCAL", s.non_local)].into_iter().enumerate() {
            let Some(m) = m else { continue };
            if m.usage > m.budget && !self.fired[i] {
                self.fired[i] = true;
                out.push(format!("vram OVER BUDGET at sample #{n}: {name} usage {:.1}MB exceeds budget {:.1}MB by {:.1}MB; the OS may page or demote our resources | {}",
                    mb(m.usage), mb(m.budget), mb(m.usage - m.budget), arch_text(arch)));
            }
        }
        out
    }
}

pub fn arch_text(a: Option<Arch>) -> String {
    match a {
        Some(a) => format!("UMA={} CacheCoherentUMA={} TileBasedRenderer={}", a.uma as u32, a.cc_uma as u32, a.tile_based as u32),
        None => "UMA=? CacheCoherentUMA=? (CheckFeatureSupport ARCHITECTURE1 failed)".to_string(),
    }
}

pub(crate) unsafe fn device_luid(device: *mut c_void) -> u64 {
    let f: unsafe extern "system" fn(*mut c_void, *mut u64) -> *mut u64 = unsafe { core::mem::transmute(slot(device, 43)) };
    let mut l = 0u64;
    unsafe { f(device, &mut l) };
    l
}

pub(crate) unsafe fn architecture(device: *mut c_void) -> Option<Arch> {
    let f: unsafe extern "system" fn(*mut c_void, u32, *mut c_void, u32) -> i32 = unsafe { core::mem::transmute(slot(device, 13)) };
    let mut d = [0u32; 5];
    (unsafe { f(device, FEATURE_ARCHITECTURE1, d.as_mut_ptr() as *mut c_void, 20) } >= 0).then(|| Arch::from_raw(d))
}

pub(crate) unsafe fn adapter3(luid: u64) -> Result<*mut c_void, String> {
    let mut m = unsafe { GetModuleHandleA(b"dxgi.dll\0".as_ptr()) };
    if m.is_null() {
        m = unsafe { LoadLibraryA(b"dxgi.dll\0".as_ptr()) };
    }
    if m.is_null() {
        return Err("dxgi.dll not loaded".into());
    }
    let cf = unsafe { GetProcAddress(m, b"CreateDXGIFactory2\0".as_ptr()) };
    if cf.is_null() {
        return Err("CreateDXGIFactory2 missing".into());
    }
    let cf: unsafe extern "system" fn(u32, *const G, *mut *mut c_void) -> i32 = unsafe { core::mem::transmute(cf) };
    let mut fac: *mut c_void = core::ptr::null_mut();
    let hr = unsafe { cf(0, &IID_FACTORY4, &mut fac) };
    if hr < 0 || fac.is_null() {
        return Err(format!("CreateDXGIFactory2(IDXGIFactory4) hr={hr:#010x}"));
    }
    let by_luid: unsafe extern "system" fn(*mut c_void, u64, *const G, *mut *mut c_void) -> i32 = unsafe { core::mem::transmute(slot(fac, 26)) };
    let mut a: *mut c_void = core::ptr::null_mut();
    let hr = unsafe { by_luid(fac, luid, &IID_ADAPTER3, &mut a) };
    let release: unsafe extern "system" fn(*mut c_void) -> u32 = unsafe { core::mem::transmute(slot(fac, 2)) };
    unsafe { release(fac) };
    if hr < 0 || a.is_null() {
        return Err(format!("EnumAdapterByLuid({luid:#x}, IDXGIAdapter3) hr={hr:#010x}"));
    }
    Ok(a)
}

pub unsafe fn query(adapter: *mut c_void, group: u32) -> Option<MemInfo> {
    let q: unsafe extern "system" fn(*mut c_void, u32, u32, *mut [u64; 4]) -> i32 = unsafe { core::mem::transmute(slot(adapter, 14)) };
    let mut r = [0u64; 4];
    (unsafe { q(adapter, 0, group, &mut r) } >= 0).then(|| MemInfo::from_raw(r))
}

pub fn sample(adapter: *mut c_void) -> Sample {
    let r = crate::hang_cap::residency_counts();
    let (committed_bytes, heap_bytes, title_counted) = crate::title_gpu::live_bytes();
    let (placed_tracked, placed_bytes) = crate::upload::tracked_bytes();
    let u = crate::upload::upload_totals();
    Sample {
        local: unsafe { query(adapter, 0) },
        non_local: unsafe { query(adapter, 1) },
        make_resident: r[0],
        make_resident_objs: r[2],
        evict: r[1],
        evict_objs: r[3],
        committed_bytes,
        heap_bytes,
        title_counted,
        placed_tracked,
        placed_bytes,
        upload_bytes: u[0],
        upload_copies: u[1],
        upload_lists: u[2],
        cross_waits: u[3],
    }
}

static STARTED: AtomicBool = AtomicBool::new(false);

pub unsafe fn start(device: *mut c_void) {
    let Some(ms) = parse_ms(std::env::var("TOMBOX_VRAM_LOG").ok().as_deref()) else { return };
    if device.is_null() || STARTED.swap(true, Ordering::AcqRel) {
        return;
    }
    let luid = unsafe { device_luid(device) };
    let arch = unsafe { architecture(device) };
    let adapter = match unsafe { adapter3(luid) } {
        Ok(a) => a as usize,
        Err(e) => {
            record("d3d12_xs", "vram", &format!("TOMBOX_VRAM_LOG={ms}: no IDXGIAdapter3 for the device adapter (luid {luid:#x}): {e}; VRAM log off"));
            return;
        }
    };
    record("d3d12_xs", "vram", &format!("TOMBOX_VRAM_LOG={ms}ms: logging IDXGIAdapter3::QueryVideoMemoryInfo LOCAL/NON_LOCAL for adapter luid {luid:#x} | {}", arch_text(arch)));
    let spawned = std::thread::Builder::new().name("tombox-vram".into()).spawn(move || {
        let a = adapter as *mut c_void;
        let mut prev = sample(a);
        let mut over = OverBudget::default();
        let mut n = 0u64;
        record("d3d12_xs", "vram", &line(n, 0, &prev, &Sample::default()));
        for l in over.check(n, &prev, arch) {
            record("d3d12_xs", "vram", &l);
        }
        let mut t = std::time::Instant::now();
        loop {
            std::thread::sleep(std::time::Duration::from_millis(ms));
            n += 1;
            let s = sample(a);
            let dt = t.elapsed().as_millis() as u64;
            t = std::time::Instant::now();
            record("d3d12_xs", "vram", &line(n, dt, &s, &prev));
            for l in over.check(n, &s, arch) {
                record("d3d12_xs", "vram", &l);
            }
            prev = s;
        }
    });
    if let Err(e) = spawned {
        record("d3d12_xs", "vram", &format!("VRAM log thread failed to start: {e}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_value_parses_to_an_interval_or_off() {
        assert_eq!(parse_ms(None), None);
        assert_eq!(parse_ms(Some("")), None);
        assert_eq!(parse_ms(Some("0")), None);
        assert_eq!(parse_ms(Some("off")), None);
        assert_eq!(parse_ms(Some("bogus")), None);
        assert_eq!(parse_ms(Some("2000")), Some(2000));
        assert_eq!(parse_ms(Some(" 500 ")), Some(500));
        assert_eq!(parse_ms(Some("5")), Some(MIN_MS));
        assert_eq!(parse_ms(Some("on")), Some(DEFAULT_MS));
    }

    #[test]
    fn raw_structs_map_to_the_dxgi_and_d3d12_field_order() {
        let m = MemInfo::from_raw([10 << 20, 12 << 20, 3 << 20, 1 << 20]);
        assert_eq!(m, MemInfo { budget: 10 << 20, usage: 12 << 20, avail_reserve: 3 << 20, reserved: 1 << 20 });
        assert_eq!(Arch::from_raw([0, 0, 1, 1, 0]), Arch { uma: true, cc_uma: true, tile_based: false });
        assert_eq!(Arch::from_raw([0, 1, 0, 0, 0]), Arch { uma: false, cc_uma: false, tile_based: true });
    }

    #[test]
    fn line_reports_segments_deltas_and_over_budget_once() {
        let prev = Sample { make_resident: 4, evict: 1, upload_bytes: 1 << 20, upload_copies: 10, upload_lists: 2, ..Sample::default() };
        let s = Sample {
            local: Some(MemInfo { budget: 100 << 20, usage: 150 << 20, avail_reserve: 50 << 20, reserved: 0 }),
            non_local: Some(MemInfo { budget: 200 << 20, usage: 20 << 20, avail_reserve: 100 << 20, reserved: 0 }),
            make_resident: 7, make_resident_objs: 9, evict: 3, evict_objs: 5,
            committed_bytes: 64 << 20, heap_bytes: 32 << 20, title_counted: true,
            placed_tracked: 3, placed_bytes: 8 << 20,
            upload_bytes: 5 << 20, upload_copies: 25, upload_lists: 5, cross_waits: 2,
        };
        let l = line(7, 2001, &s, &prev);
        assert!(l.starts_with("vram #7 (+2001ms): LOCAL: usage=150.0MB budget=100.0MB (OVER)"), "{l}");
        assert!(l.contains("NON_LOCAL: usage=20.0MB budget=200.0MB (ok)"), "{l}");
        assert!(l.contains("MakeResident +3 calls +9 objs, Evict +2 calls +5 objs"), "{l}");
        assert!(l.contains("committed=64.0MB heaps=32.0MB, mirrored placed textures=3 (8.0MB)"), "{l}");
        assert!(l.contains("uploads +3 lists +15 copies +4.0MB, cross-queue waits +2"), "{l}");
        let mut o = OverBudget::default();
        let first = o.check(7, &s, Some(Arch { uma: false, cc_uma: false, tile_based: false }));
        assert_eq!(first.len(), 1);
        assert!(first[0].contains("LOCAL usage 150.0MB exceeds budget 100.0MB by 50.0MB") && first[0].contains("UMA=0 CacheCoherentUMA=0"), "{}", first[0]);
        assert!(o.check(8, &s, None).is_empty());
        let q = Sample { local: None, ..Sample::default() };
        assert!(line(1, 1, &q, &q).contains("LOCAL: query failed"));
        assert!(line(1, 1, &q, &q).contains("committed/heaps=n/a"));
    }
}
