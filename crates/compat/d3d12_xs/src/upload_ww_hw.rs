use super::*;

const D_PLACED_X: usize = 44;
const X_COPY_TEXTURE_REGION: usize = 16;
const FMT_RGBA8: u32 = 28;
const LAYOUT_LINEAR: u32 = 264;
const LAYOUT_DEFAULT_TILED: u32 = 0;
const W: usize = 256;
const H: usize = 64;

extern "system" {
    fn VirtualAlloc(addr: *mut c_void, len: usize, ty: u32, protect: u32) -> *mut c_void;
    fn VirtualFree(addr: *mut c_void, len: usize, ty: u32) -> i32;
}

fn wrapped() -> usize {
    static WD: OnceLock<usize> = OnceLock::new();
    *WD.get_or_init(|| crate::device_wrap::wrap_device(gpu().dev as *mut c_void) as usize)
}

fn tdesc(layout: u32) -> [u8; 0x38] {
    let mut d = [0u8; 0x38];
    d[0..4].copy_from_slice(&3u32.to_le_bytes());
    d[0x10..0x18].copy_from_slice(&(W as u64).to_le_bytes());
    d[0x18..0x1c].copy_from_slice(&(H as u32).to_le_bytes());
    d[0x1c..0x1e].copy_from_slice(&1u16.to_le_bytes());
    d[0x1e..0x20].copy_from_slice(&1u16.to_le_bytes());
    d[0x20..0x24].copy_from_slice(&FMT_RGBA8.to_le_bytes());
    d[0x24..0x28].copy_from_slice(&1u32.to_le_bytes());
    d[0x2c..0x30].copy_from_slice(&layout.to_le_bytes());
    d
}

fn pattern(i: usize, seed: u8) -> u8 {
    (i as u8).wrapping_mul(7) ^ ((i >> 8) as u8).wrapping_mul(13) ^ seed
}

unsafe fn readback(g: &Gpu, tex: *mut c_void) -> Vec<u8> {
    readback_with(g, tex, || {})
}

unsafe fn readback_with(g: &Gpu, tex: *mut c_void, before_submit: impl FnOnce()) -> Vec<u8> {
    let pitch = W * 4;
    let rb = buffer(g.dev, HEAP_READBACK, (pitch * H) as u64, 0, ST_COPY_DEST);
    let mut dst = [0u8; 48];
    dst[0..8].copy_from_slice(&(rb.res as u64).to_le_bytes());
    dst[8..12].copy_from_slice(&1u32.to_le_bytes());
    dst[24..28].copy_from_slice(&FMT_RGBA8.to_le_bytes());
    dst[28..32].copy_from_slice(&(W as u32).to_le_bytes());
    dst[32..36].copy_from_slice(&(H as u32).to_le_bytes());
    dst[36..40].copy_from_slice(&1u32.to_le_bytes());
    dst[40..44].copy_from_slice(&(pitch as u32).to_le_bytes());
    let mut src = [0u8; 48];
    src[0..8].copy_from_slice(&(tex as u64).to_le_bytes());
    let l = title_list(g.dev);
    call!(l, X_COPY_TEXTURE_REGION, (*const u8, u32, u32, u32, *const u8, *const c_void) -> (), dst.as_ptr(), 0, 0, 0, src.as_ptr(), core::ptr::null());
    assert!(call!(l, X_CLOSE, () -> i32) >= 0);
    before_submit();
    let f = fence(g.dev);
    submit(g.a, l, f, 1);
    assert!(reached(f, 1, Duration::from_secs(10)));
    let out = core::slice::from_raw_parts(rb.cpu, pitch * H).to_vec();
    call!(rb.res, 2, () -> u32);
    out
}

fn expected(mem: usize, offs: &[u32], bpe: usize) -> Vec<u8> {
    let mut v = Vec::with_capacity(W * H * bpe);
    for &o in offs {
        for k in 0..bpe {
            v.push(unsafe { *((mem + o as usize + k) as *const u8) });
        }
    }
    v
}

fn first_diff(a: &[u8], b: &[u8]) -> Option<(usize, usize, u8, u8)> {
    a.iter().zip(b).position(|(x, y)| x != y).map(|i| ((i / 4) % W, (i / 4) / W, a[i], b[i]))
}

unsafe fn pump_until(g: &Gpu, what: &str, done: impl Fn() -> bool) {
    let t = Instant::now();
    while !done() {
        assert!(t.elapsed() < Duration::from_secs(10), "upload pump never reached: {what}");
        crate::upload::pump(g.a as *mut c_void);
        std::thread::sleep(Duration::from_millis(2));
    }
    crate::upload::pump(g.a as *mut c_void);
}

unsafe fn cpu_writes_reach_the_gpu(layout: u32) {
    let g = gpu();
    let w = wrapped();
    let desc = tdesc(layout);
    let (total, bpe, offs) = crate::upload::test_offsets(&desc).unwrap_or_else(|e| panic!("XG layout for layout {layout}: {e}"));
    assert_eq!((bpe, offs.len()), (4, W * H));
    if layout == LAYOUT_DEFAULT_TILED {
        assert!(offs[..W].windows(2).any(|p| p[1] != p[0] + 4), "layout {layout} must be tiled for this case, but XG returned a row-linear map");
    } else {
        assert!(offs[..W].windows(2).all(|p| p[1] == p[0] + 4), "layout {layout} must be row-linear");
    }
    let n = ((total as usize) + 0xffff) & !0xffff;
    let mem = VirtualAlloc(core::ptr::null_mut(), n, 0x3000 | 0x20_0000, 0x04) as usize;
    assert_ne!(mem, 0);
    crate::cpu_watch::register_test_region(mem, n);
    for i in 0..total as usize {
        *((mem + i) as *mut u8) = pattern(i, 0x11);
    }
    let mut tex: *mut c_void = core::ptr::null_mut();
    let hr = call!(w, D_PLACED_X, (u64, *const u8, u32, *const c_void, *const Guid, *mut *mut c_void) -> i32, mem as u64, desc.as_ptr(), ST_COMMON, core::ptr::null(), &IID_RESOURCE, &mut tex);
    assert!(hr >= 0 && !tex.is_null(), "CreatePlacedResourceX hr={hr:#010x}");
    pump_until(g, "first upload", || crate::upload::test_ww(tex).sent);
    let s0 = crate::upload::test_ww(tex);
    if crate::cpu_watch::enabled() {
        assert_eq!(s0.watched, Some(true), "a placed texture on emulator write-watched memory must use write watch, not the hash fallback");
    }
    let got = readback(g, tex);
    assert_eq!(first_diff(&got, &expected(mem, &offs, bpe)), None, "layout {layout}: first upload must carry the CPU-written texels (x, y, got, want)");
    let poll = std::env::var("TOMBOX_UPLOAD_POLL").map(|v| v.trim() == "1").unwrap_or(false);
    if !crate::cpu_watch::enabled() && !poll {
        call!(tex, 2, () -> u32);
        return;
    }

    let page = mem + ((total as usize / 2) & !0xfff);
    for k in 16..32 {
        *((page + k) as *mut u8) ^= 0xff;
    }
    pump_until(g, "re-upload after a partial-page CPU write", || {
        let s = crate::upload::test_ww(tex);
        s.uploaded_subs > s0.uploaded_subs
    });
    let s1 = crate::upload::test_ww(tex);
    assert_eq!(s1.lost, s0.lost, "no CPU write may be dropped");
    if crate::cpu_watch::enabled() {
        assert!(s1.dirty_subs > s0.dirty_subs, "the write must be found by GetWriteWatch");
    }
    if crate::cpu_watch::enabled() && layout == LAYOUT_LINEAR {
        assert!(s1.band_uploads > s0.band_uploads, "a one-page write into a linear texture must re-upload only the rows on that page");
    }
    let got = readback(g, tex);
    assert_eq!(first_diff(&got, &expected(mem, &offs, bpe)), None, "layout {layout}: a CPU write after the first upload must reach the GPU (x, y, got, want)");

    for i in 0..total as usize {
        *((mem + i) as *mut u8) = pattern(i, 0x5a);
    }
    pump_until(g, "re-upload after a full CPU rewrite", || {
        let s = crate::upload::test_ww(tex);
        s.uploaded_subs > s1.uploaded_subs
    });
    let got = readback(g, tex);
    assert_eq!(first_diff(&got, &expected(mem, &offs, bpe)), None, "layout {layout}: a full rewrite must reach the GPU (x, y, got, want)");
    let s2 = crate::upload::test_ww(tex);
    for _ in 0..8 {
        crate::upload::pump(g.a as *mut c_void);
        std::thread::sleep(Duration::from_millis(2));
    }
    let s3 = crate::upload::test_ww(tex);
    if crate::cpu_watch::enabled() {
        assert_eq!(s3.uploaded_subs, s2.uploaded_subs, "with no CPU writes, write watch must not re-upload");
    }
    call!(tex, 2, () -> u32);
}

#[test]
#[ignore = "needs a D3D12 device; run: cargo test -p d3d12_xs --lib hw_ -- --ignored --test-threads=1 (TOMBOX_HW_ADAPTER=warp for WARP)"]
fn hw_cpu_writes_into_a_linear_placed_texture_reach_the_gpu() {
    unsafe { cpu_writes_reach_the_gpu(LAYOUT_LINEAR) };
}

#[test]
#[ignore = "needs a D3D12 device; run: cargo test -p d3d12_xs --lib hw_ -- --ignored --test-threads=1 (TOMBOX_HW_ADAPTER=warp for WARP)"]
fn hw_cpu_writes_into_a_tiled_placed_texture_reach_the_gpu() {
    unsafe { cpu_writes_reach_the_gpu(LAYOUT_DEFAULT_TILED) };
}

unsafe fn write_between_record_and_submit(layout: u32) {
    let g = gpu();
    let w = wrapped();
    let desc = tdesc(layout);
    let (total, bpe, offs) = crate::upload::test_offsets(&desc).unwrap_or_else(|e| panic!("XG layout for layout {layout}: {e}"));
    let n = ((total as usize) + 0xffff) & !0xffff;
    let mem = VirtualAlloc(core::ptr::null_mut(), n, 0x3000 | 0x20_0000, 0x04) as usize;
    assert_ne!(mem, 0);
    crate::cpu_watch::register_test_region(mem, n);
    for i in 0..total as usize {
        *((mem + i) as *mut u8) = pattern(i, 0x33);
    }
    let mut tex: *mut c_void = core::ptr::null_mut();
    let hr = call!(w, D_PLACED_X, (u64, *const u8, u32, *const c_void, *const Guid, *mut *mut c_void) -> i32, mem as u64, desc.as_ptr(), ST_COMMON, core::ptr::null(), &IID_RESOURCE, &mut tex);
    assert!(hr >= 0 && !tex.is_null(), "CreatePlacedResourceX hr={hr:#010x}");
    pump_until(g, "first upload", || crate::upload::test_ww(tex).sent);
    if !crate::upload::submit_hook_enabled() {
        call!(tex, 2, () -> u32);
        return;
    }
    let before = crate::upload::test_ww(tex).uploaded_subs;
    let page = mem + ((total as usize / 3) & !0xfff);
    let got = readback_with(g, tex, || {
        for k in 0..64 {
            *((page + 100 + k) as *mut u8) ^= 0xa5;
        }
    });
    assert!(crate::upload::test_ww(tex).uploaded_subs > before, "the submit hook must upload the CPU write before the list that reads it");
    assert_eq!(first_diff(&got, &expected(mem, &offs, bpe)), None, "layout {layout}: a CPU write after record and before submit must be visible to that same submit (x, y, got, want)");
    let got = readback_with(g, tex, || {
        for i in 0..total as usize {
            *((mem + i) as *mut u8) = pattern(i, 0x77);
        }
    });
    assert_eq!(first_diff(&got, &expected(mem, &offs, bpe)), None, "layout {layout}: a full rewrite before submit must be visible to that same submit (x, y, got, want)");
    call!(tex, 2, () -> u32);
}

#[test]
#[ignore = "needs a D3D12 device; run: cargo test -p d3d12_xs --lib hw_ -- --ignored --test-threads=1 (TOMBOX_HW_ADAPTER=warp for WARP)"]
fn hw_cpu_write_between_record_and_submit_reaches_that_submit_linear() {
    unsafe { write_between_record_and_submit(LAYOUT_LINEAR) };
}

#[test]
#[ignore = "needs a D3D12 device; run: cargo test -p d3d12_xs --lib hw_ -- --ignored --test-threads=1 (TOMBOX_HW_ADAPTER=warp for WARP)"]
fn hw_cpu_write_between_record_and_submit_reaches_that_submit_tiled() {
    unsafe { write_between_record_and_submit(LAYOUT_DEFAULT_TILED) };
}

#[test]
#[ignore = "needs a D3D12 device; run: cargo test -p d3d12_xs --lib hw_ -- --ignored --test-threads=1 (TOMBOX_HW_ADAPTER=warp for WARP)"]
fn hw_a_dirty_page_with_unchanged_texels_is_not_re_uploaded() {
    unsafe {
        let g = gpu();
        let w = wrapped();
        let desc = tdesc(LAYOUT_LINEAR);
        let (total, _, _) = crate::upload::test_offsets(&desc).unwrap();
        let n = ((total as usize) + 0xffff) & !0xffff;
        let mem = VirtualAlloc(core::ptr::null_mut(), n, 0x3000 | 0x20_0000, 0x04) as usize;
        assert_ne!(mem, 0);
        crate::cpu_watch::register_test_region(mem, n);
        for i in 0..total as usize {
            *((mem + i) as *mut u8) = pattern(i, 0x21);
        }
        let mut tex: *mut c_void = core::ptr::null_mut();
        let hr = call!(w, D_PLACED_X, (u64, *const u8, u32, *const c_void, *const Guid, *mut *mut c_void) -> i32, mem as u64, desc.as_ptr(), ST_COMMON, core::ptr::null(), &IID_RESOURCE, &mut tex);
        assert!(hr >= 0 && !tex.is_null());
        pump_until(g, "first upload", || crate::upload::test_ww(tex).sent);
        if !crate::cpu_watch::enabled() {
            call!(tex, 2, () -> u32);
            return;
        }
        let s0 = crate::upload::test_ww(tex);
        for p in (0..total as usize).step_by(0x1000) {
            let a = (mem + p) as *mut u8;
            core::ptr::write_volatile(a, core::ptr::read_volatile(a));
        }
        pump_until(g, "the identical-bytes write to be examined", || crate::upload::test_ww(tex).unchanged > s0.unchanged);
        let s1 = crate::upload::test_ww(tex);
        assert_eq!(s1.uploaded_subs, s0.uploaded_subs, "a page the CPU wrote with identical bytes must not be uploaded again: on Xbox the GPU may have written those texels since (UAV / copy), and a re-upload would overwrite them with the CPU-side bytes");
        call!(tex, 2, () -> u32);
    }
}

#[test]
#[ignore = "needs a D3D12 device; run: cargo test -p d3d12_xs --lib hw_ -- --ignored --test-threads=1 (TOMBOX_HW_ADAPTER=warp for WARP)"]
fn hw_unwatched_textures_get_exactly_the_write_watch_off_treatment() {
    unsafe {
        let g = gpu();
        let w = wrapped();
        let desc = tdesc(LAYOUT_LINEAR);
        let (total, _, _) = crate::upload::test_offsets(&desc).unwrap();
        let n = ((total as usize) + 0xffff) & !0xffff;
        let mem = VirtualAlloc(core::ptr::null_mut(), n, 0x3000, 0x04) as usize;
        assert_ne!(mem, 0);
        for i in 0..total as usize {
            *((mem + i) as *mut u8) = pattern(i, 0x42);
        }
        let mut tex: *mut c_void = core::ptr::null_mut();
        let hr = call!(w, D_PLACED_X, (u64, *const u8, u32, *const c_void, *const Guid, *mut *mut c_void) -> i32, mem as u64, desc.as_ptr(), ST_COMMON, core::ptr::null(), &IID_RESOURCE, &mut tex);
        assert!(hr >= 0 && !tex.is_null());
        pump_until(g, "first upload", || crate::upload::test_ww(tex).sent);
        assert_eq!(crate::upload::test_ww(tex).watched, Some(false), "memory without MEM_WRITE_WATCH must take the hash fallback");
        let poll = std::env::var("TOMBOX_UPLOAD_POLL").map(|v| v.trim() == "1").unwrap_or(false);
        for _ in 0..200 {
            crate::upload::pump(g.a as *mut c_void);
        }
        let s0 = crate::upload::test_ww(tex);
        *((mem + 0x100) as *mut u8) ^= 0xff;
        for _ in 0..200 {
            crate::upload::pump(g.a as *mut c_void);
            std::thread::sleep(Duration::from_millis(1));
        }
        let s1 = crate::upload::test_ww(tex);
        if !poll {
            assert_eq!(s1.uploaded_subs, s0.uploaded_subs, "with polling off, an unwatched texture must behave exactly as with TOMBOX_WRITE_WATCH=0: no hash rescans after the watch window");
        }
        call!(tex, 2, () -> u32);
    }
}

#[test]
#[ignore = "needs a D3D12 device; run: cargo test -p d3d12_xs --lib hw_ -- --ignored --test-threads=1 (TOMBOX_HW_ADAPTER=warp for WARP)"]
fn hw_a_texture_whose_backing_is_released_stops_being_polled_without_a_failure() {
    unsafe {
        let g = gpu();
        let w = wrapped();
        let desc = tdesc(LAYOUT_LINEAR);
        let (total, _, _) = crate::upload::test_offsets(&desc).unwrap();
        let n = ((total as usize) + 0xffff) & !0xffff;
        let mem = VirtualAlloc(core::ptr::null_mut(), n, 0x3000 | 0x20_0000, 0x04) as usize;
        assert_ne!(mem, 0);
        crate::cpu_watch::register_test_region(mem, n);
        for i in 0..total as usize {
            *((mem + i) as *mut u8) = pattern(i, 0x19);
        }
        let mut tex: *mut c_void = core::ptr::null_mut();
        let hr = call!(w, D_PLACED_X, (u64, *const u8, u32, *const c_void, *const Guid, *mut *mut c_void) -> i32, mem as u64, desc.as_ptr(), ST_COMMON, core::ptr::null(), &IID_RESOURCE, &mut tex);
        assert!(hr >= 0 && !tex.is_null());
        pump_until(g, "first upload", || crate::upload::test_ww(tex).sent);
        if !crate::cpu_watch::enabled() {
            call!(tex, 2, () -> u32);
            return;
        }
        let s0 = crate::upload::test_ww(tex);
        assert_eq!(VirtualFree(mem as *mut c_void, 0, 0x8000), 1);
        pump_until(g, "the released backing to be noticed", || crate::upload::test_ww(tex).freed);
        for _ in 0..50 {
            crate::upload::pump(g.a as *mut c_void);
        }
        let s1 = crate::upload::test_ww(tex);
        assert_eq!(s1.fallbacks, s0.fallbacks, "released backing is not a write-watch failure");
        assert_eq!(s1.lost, s0.lost, "released backing is not a lost CPU write");
        assert_eq!(s1.uploaded_subs, s0.uploaded_subs, "nothing is uploaded from released memory");
        assert_eq!(s1.watched, Some(false));
        call!(tex, 2, () -> u32);
    }
}

#[test]
#[ignore = "needs a D3D12 device; run: cargo test -p d3d12_xs --lib hw_ -- --ignored --test-threads=1 (TOMBOX_HW_ADAPTER=warp for WARP)"]
fn hw_a_freed_texture_whose_range_is_reallocated_uploads_the_new_bytes() {
    unsafe {
        let g = gpu();
        let w = wrapped();
        let desc = tdesc(LAYOUT_LINEAR);
        let (total, bpe, offs) = crate::upload::test_offsets(&desc).unwrap();
        let n = ((total as usize) + 0xffff) & !0xffff;
        let mem = VirtualAlloc(core::ptr::null_mut(), n, 0x3000 | 0x20_0000, 0x04) as usize;
        assert_ne!(mem, 0);
        crate::cpu_watch::register_test_region(mem, n);
        for i in 0..total as usize {
            *((mem + i) as *mut u8) = pattern(i, 0x0d);
        }
        let mut tex: *mut c_void = core::ptr::null_mut();
        let hr = call!(w, D_PLACED_X, (u64, *const u8, u32, *const c_void, *const Guid, *mut *mut c_void) -> i32, mem as u64, desc.as_ptr(), ST_COMMON, core::ptr::null(), &IID_RESOURCE, &mut tex);
        assert!(hr >= 0 && !tex.is_null());
        pump_until(g, "first upload", || crate::upload::test_ww(tex).sent);
        if !crate::cpu_watch::enabled() {
            call!(tex, 2, () -> u32);
            return;
        }
        assert_eq!(VirtualFree(mem as *mut c_void, 0, 0x8000), 1);
        pump_until(g, "the released backing to be noticed", || crate::upload::test_ww(tex).freed);
        let mut base = VirtualAlloc((mem - 0x1_0000) as *mut c_void, n + 0x2_0000, 0x3000 | 0x20_0000, 0x04) as usize;
        let mut len = n + 0x2_0000;
        if base == 0 {
            base = VirtualAlloc(mem as *mut c_void, n + 0x1_0000, 0x3000 | 0x20_0000, 0x04) as usize;
            len = n + 0x1_0000;
        }
        assert!(base != 0 && base <= mem && mem + n <= base + len, "could not allocate a different-sized block over the freed range");
        assert_ne!((base, len), (mem, n));
        crate::cpu_watch::register_test_region(base, len);
        crate::upload::TomboxNoteTitleMemory(base, len, 1);
        for i in 0..total as usize {
            *((mem + i) as *mut u8) = pattern(i, 0x6b);
        }
        let s0 = crate::upload::test_ww(tex);
        crate::upload::pump(g.a as *mut c_void);
        let s1 = crate::upload::test_ww(tex);
        assert!(!s1.freed, "a new allocation over the range must bring the resource back at the next pump, not on a timer");
        assert_eq!(s1.watched, Some(true), "the reborn resource must be write-watched on the new allocation");
        pump_until(g, "the reborn upload", || crate::upload::test_ww(tex).uploaded_subs > s0.uploaded_subs);
        let got = readback(g, tex);
        assert_eq!(first_diff(&got, &expected(mem, &offs, bpe)), None, "the old resource must show the new allocation's bytes (x, y, got, want)");
        for k in 0..32 {
            *((mem + 0x2000 + k) as *mut u8) ^= 0x3c;
        }
        let s2 = crate::upload::test_ww(tex);
        pump_until(g, "a later write into the new allocation", || crate::upload::test_ww(tex).uploaded_subs > s2.uploaded_subs);
        let got = readback(g, tex);
        assert_eq!(first_diff(&got, &expected(mem, &offs, bpe)), None, "writes into the new allocation keep reaching the GPU (x, y, got, want)");
        call!(tex, 2, () -> u32);
        VirtualFree(base as *mut c_void, 0, 0x8000);
    }
}

static FAKE_SEQ: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(1);
static FAKE_LOG: std::sync::Mutex<Vec<(u64, usize, usize)>> = std::sync::Mutex::new(Vec::new());

unsafe extern "system" fn fake_epoch() -> u64 {
    FAKE_SEQ.load(core::sync::atomic::Ordering::Acquire)
}

unsafe extern "system" fn fake_changed(seq: u64, lo: usize, hi: usize) -> i32 {
    FAKE_LOG.lock().unwrap_or_else(|e| e.into_inner()).iter().any(|&(s, a, b)| s >= seq && a < hi && lo < b) as i32
}

fn fake_bump(lo: usize, hi: usize) {
    let mut g = FAKE_LOG.lock().unwrap_or_else(|e| e.into_inner());
    let s = FAKE_SEQ.fetch_add(1, core::sync::atomic::Ordering::AcqRel);
    g.push((s, lo, hi));
}

#[test]
#[ignore = "needs a D3D12 device; run: cargo test -p d3d12_xs --lib hw_ -- --ignored --test-threads=1 (TOMBOX_HW_ADAPTER=warp for WARP)"]
fn hw_a_freed_then_reallocated_region_is_rescanned_by_the_residency_cache() {
    unsafe {
        let g = gpu();
        let w = wrapped();
        struct Off;
        impl Drop for Off {
            fn drop(&mut self) {
                crate::upload::test_epoch_fns(None);
            }
        }
        crate::upload::test_epoch_fns(Some((fake_epoch as *const () as usize, fake_changed as *const () as usize)));
        let _off = Off;
        let desc = tdesc(LAYOUT_LINEAR);
        let (total, bpe, offs) = crate::upload::test_offsets(&desc).unwrap();
        let n = ((total as usize) + 0xffff) & !0xffff;
        let mem = VirtualAlloc(core::ptr::null_mut(), n, 0x3000 | 0x20_0000, 0x04) as usize;
        assert_ne!(mem, 0);
        crate::cpu_watch::register_test_region(mem, n);
        for i in 0..total as usize {
            *((mem + i) as *mut u8) = pattern(i, 0x21);
        }
        let mut tex: *mut c_void = core::ptr::null_mut();
        let hr = call!(w, D_PLACED_X, (u64, *const u8, u32, *const c_void, *const Guid, *mut *mut c_void) -> i32, mem as u64, desc.as_ptr(), ST_COMMON, core::ptr::null(), &IID_RESOURCE, &mut tex);
        assert!(hr >= 0 && !tex.is_null());
        pump_until(g, "first upload", || crate::upload::test_ww(tex).sent);
        let s0 = crate::upload::test_ww(tex);
        if !s0.rcached {
            call!(tex, 2, () -> u32);
            VirtualFree(mem as *mut c_void, 0, 0x8000);
            return;
        }
        fake_bump(mem + n + 0x10_0000, mem + n + 0x11_0000);
        for k in 0..64 {
            *((mem + 0x1000 + k) as *mut u8) ^= 0x5a;
        }
        crate::upload::TomboxNoteTitleMemory(mem + 0x1000, 0x1000, crate::upload::NOTE_WRITTEN);
        pump_until(g, "an upload served from the cache", || crate::upload::test_ww(tex).uploaded_subs > s0.uploaded_subs);
        let s1 = crate::upload::test_ww(tex);
        assert!(s1.rcached, "a write and an unrelated address-space change keep the cached residency");
        assert!(s1.rs[1] > s0.rs[1], "the scan after the write is a cache hit");
        let got = readback(g, tex);
        assert_eq!(first_diff(&got, &expected(mem, &offs, bpe)), None, "the cached residency still uploads the written bytes (x, y, got, want)");
        assert_eq!(VirtualFree(mem as *mut c_void, 0, 0x8000), 1);
        fake_bump(mem, mem + n);
        let again = VirtualAlloc(mem as *mut c_void, n, 0x3000 | 0x20_0000, 0x04) as usize;
        assert_eq!(again, mem, "could not reallocate the freed range at the same address");
        crate::cpu_watch::register_test_region(mem, n);
        for i in 0..total as usize {
            *((mem + i) as *mut u8) = pattern(i, 0x5e);
        }
        if crate::cpu_watch::enabled() {
            crate::upload::TomboxNoteTitleMemory(mem, n, crate::upload::NOTE_COMMITTED);
        } else {
            crate::upload::TomboxNoteTitleMemory(mem, n, crate::upload::NOTE_WRITTEN);
        }
        pump_until(g, "the upload from the reallocated memory", || crate::upload::test_ww(tex).uploaded_subs > s1.uploaded_subs);
        let s2 = crate::upload::test_ww(tex);
        assert!(s2.rs[3] > s1.rs[3], "the free must invalidate the cached residency");
        assert!(s2.rs[2] > s1.rs[2] && s2.rs[0] > s1.rs[0], "the reallocated range must be queried again, not served from the old cache");
        assert!(s2.rcached, "the rescanned residency is cached again");
        let got = readback(g, tex);
        assert_eq!(first_diff(&got, &expected(mem, &offs, bpe)), None, "the texture shows the reallocated memory's bytes (x, y, got, want)");
        call!(tex, 2, () -> u32);
        VirtualFree(mem as *mut c_void, 0, 0x8000);
    }
}

#[test]
#[ignore = "needs a D3D12 device; run: cargo test -p d3d12_xs --lib hw_ -- --ignored --test-threads=1 (TOMBOX_HW_ADAPTER=warp for WARP)"]
fn hw_vram_log_reads_the_device_adapter_budget_and_architecture() {
    unsafe {
        let g = gpu();
        let dev = g.dev as *mut c_void;
        let luid = crate::vram_log::device_luid(dev);
        assert_ne!(luid, 0);
        let arch = crate::vram_log::architecture(dev);
        assert!(arch.is_some(), "CheckFeatureSupport(ARCHITECTURE1) must succeed");
        let a = crate::vram_log::adapter3(luid).expect("IDXGIAdapter3 for the device LUID");
        let local = crate::vram_log::query(a, 0).expect("LOCAL QueryVideoMemoryInfo");
        assert!(local.budget > 0, "{local:?}");
        assert!(crate::vram_log::query(a, 1).is_some(), "NON_LOCAL QueryVideoMemoryInfo");
        let s0 = crate::vram_log::sample(a);
        let s1 = crate::vram_log::sample(a);
        let l = crate::vram_log::line(1, 2000, &s1, &s0);
        assert!(l.starts_with("vram #1 (+2000ms): LOCAL: usage="), "{l}");
        assert!(l.contains("NON_LOCAL: usage=") && l.contains("MakeResident +"), "{l}");
        let t = crate::vram_log::arch_text(arch);
        assert!(t.starts_with("UMA=") && !t.contains('?'), "{t}");
        if g.warp {
            assert!(arch.unwrap().uma, "WARP reports UMA");
        }
        let r: unsafe extern "system" fn(*mut c_void) -> u32 = core::mem::transmute(*(*(a as *const *const usize)).add(2));
        r(a);
    }
}
