use super::*;
use crate::cl_fw::PredicationBufferDesc;

const CL_SET_PREDICATION_BUFFER: usize = 85;
const CL_ADVANCE_PREDICATION: usize = 86;

unsafe fn predicates(g: &Gpu, words: &[u64]) -> Buf {
    let p = unsafe { buffer(g.dev, HEAP_UPLOAD, 256, 0, ST_GENERIC_READ) };
    for (i, w) in words.iter().enumerate() {
        unsafe { core::ptr::write_volatile((p.cpu as *mut u64).add(i), *w) };
    }
    p
}

unsafe fn set_buffer(l: *mut c_void, d: Option<PredicationBufferDesc>) {
    let p = d.as_ref().map(|d| d as *const PredicationBufferDesc).unwrap_or(core::ptr::null());
    call!(l, CL_SET_PREDICATION_BUFFER, (*const PredicationBufferDesc) -> (), p);
}

unsafe fn advance(l: *mut c_void) {
    call!(l, CL_ADVANCE_PREDICATION, () -> ());
}

#[test]
#[ignore = "needs a D3D12 device; run: cargo test -p d3d12_xs --lib hw_ -- --ignored --test-threads=1 (TOMBOX_HW_ADAPTER=warp for WARP)"]
fn hw_write64_lifts_and_restores_a_predicate_set_by_buffer_and_advance() {
    let g = gpu();
    unsafe {
        let pred = predicates(g, &[1, 0]);
        let src = buffer(g.dev, HEAP_UPLOAD, 256, 0, ST_GENERIC_READ);
        core::ptr::write_volatile(src.cpu as *mut u64, 0xeeee_eeee_eeee_eeee);
        let dst = buffer(g.dev, HEAP_DEFAULT, 256, 0, ST_COPY_DEST);
        let out = buffer(g.dev, HEAP_READBACK, 256, 0, ST_COPY_DEST);
        let l = title_list(g.dev);
        set_buffer(l, Some(PredicationBufferDesc { va: pred.va, offset: 0 }));
        call!(l, X_COPY_BUFFER_REGION, (usize, u64, usize, u64, u64) -> (), dst.res, 64, src.res, 0, 8);
        advance(l);
        for i in 0..8u64 {
            call!(l, crate::cl_ext::CL_WRITE64_BOTTOM_OF_PIPE, (u64, u64, u32, u32) -> (), dst.va + 8 * i, 0x0102_0304_0506_0700 + i, 0x800, 0);
        }
        call!(l, X_COPY_BUFFER_REGION, (usize, u64, usize, u64, u64) -> (), dst.res, 72, src.res, 0, 8);
        set_buffer(l, None);
        raw_barrier(l, dst.res, ST_COPY_DEST, ST_COPY_SOURCE);
        call!(l, X_COPY_BUFFER_REGION, (usize, u64, usize, u64, u64) -> (), out.res, 0, dst.res, 0, 80);
        assert!(call!(l, X_CLOSE, () -> i32) >= 0);
        let f = fence(g.dev);
        submit(g.a, l, f, 1);
        assert!(reached(f, 1, Duration::from_secs(5)));
        let q = |o: usize| core::ptr::read_volatile(out.cpu.add(o) as *const u64);
        for i in 0..8 {
            assert_eq!(q(8 * i), 0x0102_0304_0506_0700 + i as u64, "value {i} written under the advanced predicate");
        }
        assert_eq!(q(64), 0xeeee_eeee_eeee_eeee, "the copy under the nonzero predicate at buffer+0 was skipped");
        assert_eq!(q(72), 0, "the advanced predicate (zero at buffer+8) was not restored after the value copies");
    }
}

#[test]
#[ignore = "needs a D3D12 device; run: cargo test -p d3d12_xs --lib hw_ -- --ignored --test-threads=1 (TOMBOX_HW_ADAPTER=warp for WARP)"]
fn hw_execute_indirect_bundle_honours_and_restores_a_predicate_set_by_buffer_and_advance() {
    let g = gpu();
    unsafe {
        let (rs, pso) = compute_pso(g.dev, COUNT_HLSL, None);
        let out = buffer(g.dev, HEAP_DEFAULT, 256, FLAG_UAV, ST_UAV);
        let rb = buffer(g.dev, HEAP_READBACK, 256, 0, ST_COPY_DEST);
        let mk = buffer(g.dev, HEAP_READBACK, 256, 0, ST_COPY_DEST);
        let src = buffer(g.dev, HEAP_UPLOAD, 256, 0, ST_GENERIC_READ);
        core::ptr::write_volatile(src.cpu as *mut u64, 0x5a5a_5a5a_5a5a_5a5a);
        let bb = buffer(g.dev, HEAP_UPLOAD, 256, 0, ST_GENERIC_READ);
        core::ptr::write_bytes(bb.cpu, 0, 256);
        crate::placed::register(bb.res as *mut c_void, crate::placed::Placed { va: bb.cpu as u64, desc: [0u8; 0x38], init_state: ST_GENERIC_READ });
        let mut bundles = Vec::new();
        for i in 0..2u32 {
            let b = counting_bundle(g.dev, rs, pso, i, out.va);
            close_bundle_into(b, bb.cpu.add(16 * i as usize));
            bundles.push(b);
        }
        let pred = predicates(g, &[0, 1, 0]);
        let l = title_list(g.dev);
        let eib = || call!(l, crate::cl_ext::CL_EXECUTE_INDIRECT_BUNDLE, (*mut c_void, u64, u32, *mut c_void, u32, u32) -> (), bb.res as *mut c_void, 0, 32, core::ptr::null_mut::<c_void>(), 0, 0);
        let mark = |o: u64| call!(l, X_COPY_BUFFER_REGION, (usize, u64, usize, u64, u64) -> (), mk.res, o, src.res, 0, 8);
        set_buffer(l, Some(PredicationBufferDesc { va: pred.va, offset: 0 }));
        eib();
        mark(0);
        advance(l);
        eib();
        mark(8);
        advance(l);
        mark(16);
        set_buffer(l, None);
        raw_barrier(l, out.res, ST_UAV, ST_COPY_SOURCE);
        call!(l, X_COPY_BUFFER_REGION, (usize, u64, usize, u64, u64) -> (), rb.res, 0, out.res, 0, 8);
        raw_barrier(l, out.res, ST_COPY_SOURCE, ST_UAV);
        assert!(call!(l, X_CLOSE, () -> i32) >= 0);
        let f = fence(g.dev);
        submit(g.a, l, f, 1);
        assert!(reached(f, 1, Duration::from_secs(5)));
        assert_eq!((rd(&rb, 0), rd(&rb, 4)), (1, 1), "only the call under the nonzero predicate at buffer+8 runs its bundles");
        let m = |o: usize| core::ptr::read_volatile(mk.cpu.add(o) as *const u64);
        assert_eq!(m(0), 0, "the zero predicate at buffer+0 was not restored after ExecuteIndirectBundleX");
        assert_eq!(m(8), 0x5a5a_5a5a_5a5a_5a5a, "the nonzero predicate at buffer+8 was not restored after ExecuteIndirectBundleX");
        assert_eq!(m(16), 0, "AdvancePredicationX after the split did not move to buffer+16");
        crate::placed::forget(bb.res as *mut c_void);
        drop(bundles);
    }
}

const X_RESET: usize = 10;
const X_CLEAR_STATE: usize = 11;

unsafe fn title_list_and_allocator(dev: usize) -> (*mut c_void, *mut c_void) {
    let mut a: *mut c_void = core::ptr::null_mut();
    assert!(call!(dev, 9, (u32, *const Guid, *mut *mut c_void) -> i32, 0, &IID_ALLOCATOR, &mut a) >= 0);
    let mut l: *mut c_void = core::ptr::null_mut();
    assert!(call!(dev, 12, (u32, u32, *mut c_void, *mut c_void, *const Guid, *mut *mut c_void) -> i32, 0, 0, a, core::ptr::null_mut(), &IID_LIST, &mut l) >= 0);
    (crate::cmdlist_wrap::wrap(l), a)
}

unsafe fn source(g: &Gpu, v: u64) -> Buf {
    let s = unsafe { buffer(g.dev, HEAP_UPLOAD, 256, 0, ST_GENERIC_READ) };
    unsafe { core::ptr::write_volatile(s.cpu as *mut u64, v) };
    s
}

unsafe fn finish(g: &Gpu, l: *mut c_void, dst: &Buf, out: &Buf, n: u64) {
    raw_barrier(l, dst.res, ST_COPY_DEST, ST_COPY_SOURCE);
    call!(l, X_COPY_BUFFER_REGION, (usize, u64, usize, u64, u64) -> (), out.res, 0, dst.res, 0, n);
    assert!(call!(l, X_CLOSE, () -> i32) >= 0);
    let f = fence(g.dev);
    submit(g.a, l, f, 1);
    assert!(reached(f, 1, Duration::from_secs(5)));
}

const MARK: u64 = 0x7777_6666_5555_4444;

#[test]
#[ignore = "needs a D3D12 device; run: cargo test -p d3d12_xs --lib hw_ -- --ignored --test-threads=1 (TOMBOX_HW_ADAPTER=warp for WARP)"]
fn hw_advance_steps_one_64bit_predicate_and_skips_work_on_zero() {
    let g = gpu();
    unsafe {
        let pred = predicates(g, &[0, 1u64 << 32, 0]);
        let src = source(g, MARK);
        let dst = buffer(g.dev, HEAP_DEFAULT, 256, 0, ST_COPY_DEST);
        let out = buffer(g.dev, HEAP_READBACK, 256, 0, ST_COPY_DEST);
        let l = title_list(g.dev);
        set_buffer(l, Some(PredicationBufferDesc { va: pred.va, offset: 0 }));
        call!(l, X_COPY_BUFFER_REGION, (usize, u64, usize, u64, u64) -> (), dst.res, 0, src.res, 0, 8);
        advance(l);
        call!(l, X_COPY_BUFFER_REGION, (usize, u64, usize, u64, u64) -> (), dst.res, 8, src.res, 0, 8);
        advance(l);
        call!(l, X_COPY_BUFFER_REGION, (usize, u64, usize, u64, u64) -> (), dst.res, 16, src.res, 0, 8);
        set_buffer(l, None);
        finish(g, l, &dst, &out, 24);
        let q = |o: usize| core::ptr::read_volatile(out.cpu.add(o) as *const u64);
        assert_eq!(q(0), 0, "a zero predicate must skip the work (EQUAL_ZERO polarity)");
        assert_eq!(q(8), MARK, "after one AdvancePredicationX the predicate is the 64-bit word at +8 (1<<32: zero low dword, nonzero high dword); a 4-byte step, a 16-byte step or a 32-bit read all skip this copy");
        assert_eq!(q(16), 0, "after the second AdvancePredicationX the zero word at +16 must skip the work");
    }
}

#[test]
#[ignore = "needs a D3D12 device; run: cargo test -p d3d12_xs --lib hw_ -- --ignored --test-threads=1 (TOMBOX_HW_ADAPTER=warp for WARP)"]
fn hw_reset_drops_the_predication_buffer_cursor() {
    let g = gpu();
    unsafe {
        let pred = predicates(g, &[1, 0]);
        let src = source(g, MARK);
        let dst = buffer(g.dev, HEAP_DEFAULT, 256, 0, ST_COPY_DEST);
        let out = buffer(g.dev, HEAP_READBACK, 256, 0, ST_COPY_DEST);
        let (l, a) = title_list_and_allocator(g.dev);
        set_buffer(l, Some(PredicationBufferDesc { va: pred.va, offset: 0 }));
        assert!(crate::cl_fw::cursor(crate::list_split::origin_of(crate::cmdlist_wrap::unwrap(l))).is_some());
        assert!(call!(l, X_CLOSE, () -> i32) >= 0);
        assert!(call!(l, X_RESET, (*mut c_void, *mut c_void) -> i32, a, core::ptr::null_mut()) >= 0);
        assert_eq!(crate::cl_fw::cursor(crate::list_split::origin_of(crate::cmdlist_wrap::unwrap(l))), None, "Reset must drop the cursor: the console list preamble (xgs12 0x3416c) starts every recording with predication off");
        advance(l);
        call!(l, X_COPY_BUFFER_REGION, (usize, u64, usize, u64, u64) -> (), dst.res, 0, src.res, 0, 8);
        finish(g, l, &dst, &out, 8);
        assert_eq!(core::ptr::read_volatile(out.cpu as *const u64), MARK, "an AdvancePredicationX after Reset re-armed the previous recording's predicate (buffer+8 = 0) and skipped the copy");
    }
}

#[test]
#[ignore = "needs a D3D12 device; run: cargo test -p d3d12_xs --lib hw_ -- --ignored --test-threads=1 (TOMBOX_HW_ADAPTER=warp for WARP)"]
fn hw_clear_state_drops_the_predication_buffer_cursor() {
    let g = gpu();
    unsafe {
        let pred = predicates(g, &[1, 0]);
        let src = source(g, MARK);
        let dst = buffer(g.dev, HEAP_DEFAULT, 256, 0, ST_COPY_DEST);
        let out = buffer(g.dev, HEAP_READBACK, 256, 0, ST_COPY_DEST);
        let l = title_list(g.dev);
        set_buffer(l, Some(PredicationBufferDesc { va: pred.va, offset: 0 }));
        assert!(crate::cl_fw::cursor(crate::list_split::origin_of(crate::cmdlist_wrap::unwrap(l))).is_some());
        call!(l, X_CLEAR_STATE, (*mut c_void) -> (), core::ptr::null_mut());
        assert_eq!(crate::cl_fw::cursor(crate::list_split::origin_of(crate::cmdlist_wrap::unwrap(l))), None, "ClearState must drop the cursor");
        advance(l);
        call!(l, X_COPY_BUFFER_REGION, (usize, u64, usize, u64, u64) -> (), dst.res, 0, src.res, 0, 8);
        finish(g, l, &dst, &out, 8);
        assert_eq!(core::ptr::read_volatile(out.cpu as *const u64), MARK, "an AdvancePredicationX after ClearState re-armed the previous recording's predicate (buffer+8 = 0) and skipped the copy");
    }
}

#[test]
#[ignore = "needs a D3D12 device; run: cargo test -p d3d12_xs --lib hw_ -- --ignored --test-threads=1 (TOMBOX_HW_ADAPTER=warp for WARP)"]
fn hw_plain_set_predication_interplay_with_the_buffer_cursor() {
    let g = gpu();
    unsafe {
        let pred = predicates(g, &[0, 0]);
        let plain = predicates(g, &[1]);
        let src = source(g, MARK);
        let dst = buffer(g.dev, HEAP_DEFAULT, 256, 0, ST_COPY_DEST);
        let out = buffer(g.dev, HEAP_READBACK, 256, 0, ST_COPY_DEST);
        let l = title_list(g.dev);
        let copy = |o: u64| call!(l, X_COPY_BUFFER_REGION, (usize, u64, usize, u64, u64) -> (), dst.res, o, src.res, 0, 8);
        let null = core::ptr::null_mut::<c_void>();
        set_buffer(l, Some(PredicationBufferDesc { va: pred.va, offset: 0 }));
        copy(0);
        call!(l, X_PREDICATION, (*mut c_void, u64, u32) -> (), null, 0, 0);
        copy(8);
        call!(l, X_PREDICATION, (*mut c_void, u64, u32) -> (), plain.res as *mut c_void, 0, 0);
        copy(16);
        call!(l, X_PREDICATION, (*mut c_void, u64, u32) -> (), null, 0, 0);
        copy(24);
        advance(l);
        copy(32);
        set_buffer(l, None);
        copy(40);
        finish(g, l, &dst, &out, 48);
        let q = |o: usize| core::ptr::read_volatile(out.cpu.add(o) as *const u64);
        assert_eq!(q(0), 0, "zero buffer predicate skips");
        assert_eq!(q(8), 0, "SetPredication(NULL) with no resource predicate active emits nothing on console (xgs12 0x56fd1), so the buffer predicate still skips");
        assert_eq!(q(16), MARK, "a resource SetPredication replaces the buffer predicate");
        assert_eq!(q(24), MARK, "SetPredication(NULL) after a resource predicate disables predication");
        assert_eq!(q(32), 0, "AdvancePredicationX after plain SetPredication moves the kept buffer cursor to +8 (zero, skips)");
        assert_eq!(q(40), MARK, "SetPredicationBufferX(NULL) disables predication");
    }
}
