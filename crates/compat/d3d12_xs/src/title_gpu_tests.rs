use core::ffi::c_void;
use std::sync::Mutex;

use crate::device_wrap::Guid;

static LOG: Mutex<Vec<(u64, u32)>> = Mutex::new(Vec::new());

unsafe extern "system" fn test_charge(bytes: u64, release: u32) -> u64 {
    LOG.lock().unwrap_or_else(|e| e.into_inner()).push((bytes, release));
    0
}

fn install() {
    super::title_gpu::set_charge_for_test(test_charge as *const () as usize);
}

fn net(bytes: u64) -> (usize, usize) {
    let l = LOG.lock().unwrap_or_else(|e| e.into_inner());
    (l.iter().filter(|e| **e == (bytes, 0)).count(), l.iter().filter(|e| **e == (bytes, 1)).count())
}

#[repr(C)]
struct Notifier {
    vt: *const usize,
    cb: usize,
    data: usize,
    registered: u32,
}

#[repr(C)]
struct Obj {
    vt: *const usize,
    notifier: *mut Notifier,
}

const IID_NOTIFIER: [u8; 16] = [0x9a, 0xb3, 0x6e, 0xa0, 0xda, 0x50, 0x5b, 0x42, 0x8c, 0x31, 0x4e, 0xec, 0xd6, 0xc2, 0x70, 0xf3];

unsafe extern "system" fn obj_qi(this: *mut Obj, riid: *const Guid, out: *mut *mut c_void) -> i32 {
    let want = unsafe { core::slice::from_raw_parts(riid as *const u8, 16) };
    let n = unsafe { (*this).notifier };
    if want == IID_NOTIFIER && !n.is_null() {
        unsafe { *out = n as *mut c_void };
        return 0;
    }
    unsafe { *out = core::ptr::null_mut() };
    0x8000_4002u32 as i32
}

unsafe extern "system" fn nop_ref(_: *mut c_void) -> u32 {
    1
}

unsafe extern "system" fn n_register(this: *mut Notifier, cb: usize, data: usize, id: *mut u32) -> i32 {
    unsafe {
        (*this).cb = cb;
        (*this).data = data;
        (*this).registered += 1;
        *id = 7;
    }
    0
}

unsafe extern "system" fn n_unregister(_: *mut Notifier, _: u32) -> i32 {
    0
}

fn obj_vt() -> &'static [usize; 3] {
    static V: std::sync::OnceLock<[usize; 3]> = std::sync::OnceLock::new();
    V.get_or_init(|| [obj_qi as *const () as usize, nop_ref as *const () as usize, nop_ref as *const () as usize])
}

fn notifier_vt() -> &'static [usize; 5] {
    static V: std::sync::OnceLock<[usize; 5]> = std::sync::OnceLock::new();
    V.get_or_init(|| [0, nop_ref as *const () as usize, nop_ref as *const () as usize, n_register as *const () as usize, n_unregister as *const () as usize])
}

unsafe fn destroy(n: &Notifier) {
    let cb: unsafe extern "system" fn(*mut c_void) = unsafe { core::mem::transmute(n.cb) };
    unsafe { cb(n.data as *mut c_void) };
}

#[test]
fn title_gpu_charge_is_released_by_the_destruction_callback_with_the_same_bytes() {
    install();
    let mut n = Notifier { vt: notifier_vt().as_ptr(), cb: 0, data: 0, registered: 0 };
    let mut o = Obj { vt: obj_vt().as_ptr(), notifier: &mut n };
    assert!(unsafe { super::title_gpu::heap(&mut o as *mut Obj as *mut c_void, 0x7_1230) });
    assert_eq!(n.registered, 1);
    assert_eq!(net(0x7_1230), (1, 0));
    unsafe { destroy(&n) };
    assert_eq!(net(0x7_1230), (1, 1));
    let mut bare = Obj { vt: obj_vt().as_ptr(), notifier: core::ptr::null_mut() };
    let before = super::title_gpu::counts().1;
    assert!(!unsafe { super::title_gpu::heap(&mut bare as *mut Obj as *mut c_void, 0x5_5550) });
    assert_eq!(net(0x5_5550), (0, 0));
    assert_eq!(super::title_gpu::counts().1, before + 1);
    assert!(!unsafe { super::title_gpu::heap(core::ptr::null_mut(), 0x1000) });
    assert!(!unsafe { super::title_gpu::heap(&mut o as *mut Obj as *mut c_void, 0) });
    assert_eq!(n.registered, 1);
}

const IID_DEVICE: Guid = Guid { d1: 0x1898_19f1, d2: 0x1db6, d3: 0x4b57, d4: [0xbe, 0x54, 0x18, 0x21, 0x33, 0x9b, 0x85, 0xf7] };
const IID_RESOURCE: Guid = Guid { d1: 0x6964_42be, d2: 0xa72e, d3: 0x4059, d4: [0xbc, 0x79, 0x5b, 0x5c, 0x98, 0x04, 0x0f, 0xad] };
const IID_HEAP: Guid = Guid { d1: 0x6b3b_2502, d2: 0x6e51, d3: 0x45b3, d4: [0x90, 0xee, 0x98, 0x84, 0x26, 0x5e, 0x8d, 0xf3] };

unsafe fn vs(o: *mut c_void, i: usize) -> usize {
    unsafe { *(*(o as *const *const usize)).add(i) }
}

unsafe fn release(o: *mut c_void) -> u32 {
    let f: unsafe extern "system" fn(*mut c_void) -> u32 = unsafe { core::mem::transmute(vs(o, 2)) };
    unsafe { f(o) }
}

fn buffer_desc(size: u64) -> [u8; 0x38] {
    let mut d = [0u8; 0x38];
    d[0..4].copy_from_slice(&1u32.to_le_bytes());
    d[0x10..0x18].copy_from_slice(&size.to_le_bytes());
    d[0x18..0x1c].copy_from_slice(&1u32.to_le_bytes());
    d[0x1c..0x1e].copy_from_slice(&1u16.to_le_bytes());
    d[0x1e..0x20].copy_from_slice(&1u16.to_le_bytes());
    d[0x24..0x28].copy_from_slice(&1u32.to_le_bytes());
    d[0x2c..0x30].copy_from_slice(&1u32.to_le_bytes());
    d
}

unsafe fn committed(dev: *mut c_void, heap_type: u32, size: u64, state: u32) -> *mut c_void {
    let h: [u32; 5] = [heap_type, 0, 0, 1, 1];
    let d = buffer_desc(size);
    let mut r: *mut c_void = core::ptr::null_mut();
    let f: unsafe extern "system" fn(*mut c_void, *const u32, u32, *const u8, u32, *const c_void, *const Guid, *mut *mut c_void) -> i32 = unsafe { core::mem::transmute(vs(dev, 27)) };
    let hr = unsafe { f(dev, h.as_ptr(), 0, d.as_ptr(), state, core::ptr::null(), &IID_RESOURCE, &mut r) };
    assert!(hr >= 0 && !r.is_null(), "CreateCommittedResource hr={hr:#010x}");
    r
}

#[test]
#[ignore = "needs a D3D12 device; run: cargo test -p d3d12_xs --lib hw_ -- --ignored --test-threads=1 (TOMBOX_HW_ADAPTER=warp for WARP)"]
fn hw_title_committed_and_heap_charge_title_memory_but_internal_upload_buffers_do_not() {
    install();
    unsafe {
        let create: extern "system" fn(*mut c_void, u32, *const Guid, *mut *mut c_void) -> i32 = core::mem::transmute(crate::host_proc(b"D3D12CreateDevice\0"));
        let mut real: *mut c_void = core::ptr::null_mut();
        let hr = create(core::ptr::null_mut(), 0xb000, &IID_DEVICE, &mut real);
        assert!(hr >= 0 && !real.is_null(), "D3D12CreateDevice hr={hr:#010x}");
        let wrapped = crate::device_wrap::wrap_device(real);
        let title_size = 0x13_1000u64;
        let want = crate::device_wrap::title_layout_size(&buffer_desc(title_size));
        assert!(want >= title_size, "console layout size {want:#x}");
        let t = committed(wrapped, 1, title_size, 0);
        assert_eq!(net(want), (1, 0), "title CreateCommittedResource must charge its console size");
        let internal_size = 0x24_7000u64;
        let ring = committed(real, 2, internal_size, 0xac3);
        assert_eq!(net(crate::device_wrap::title_layout_size(&buffer_desc(internal_size))), (0, 0), "internal upload ring made on the real device must not be charged");
        let heap_size = 0x40_0000u64;
        let mut hd = [0u8; crate::device_console_fix::HEAP_DESC_SIZE];
        hd[0..8].copy_from_slice(&heap_size.to_le_bytes());
        hd[8..12].copy_from_slice(&1u32.to_le_bytes());
        hd[0x14..0x18].copy_from_slice(&1u32.to_le_bytes());
        hd[0x18..0x1c].copy_from_slice(&1u32.to_le_bytes());
        hd[0x28..0x2c].copy_from_slice(&0x4u32.to_le_bytes());
        let ch: unsafe extern "system" fn(*mut c_void, *const u8, *const Guid, *mut *mut c_void) -> i32 = core::mem::transmute(vs(wrapped, 28));
        let mut heap: *mut c_void = core::ptr::null_mut();
        let hr = ch(wrapped, hd.as_ptr(), &IID_HEAP, &mut heap);
        assert!(hr >= 0 && !heap.is_null(), "CreateHeap hr={hr:#010x}");
        assert_eq!(net(heap_size), (1, 0), "title CreateHeap must charge its size");
        let placed: unsafe extern "system" fn(*mut c_void, *mut c_void, u64, *const u8, u32, *const c_void, *const Guid, *mut *mut c_void) -> i32 = core::mem::transmute(vs(wrapped, 29));
        let pd = buffer_desc(0x1_0000);
        let mut pr: *mut c_void = core::ptr::null_mut();
        let hr = placed(wrapped, heap, 0, pd.as_ptr(), 0x400, core::ptr::null(), &IID_RESOURCE, &mut pr);
        assert!(hr >= 0 && !pr.is_null(), "CreatePlacedResource hr={hr:#010x}");
        assert_eq!(net(0x1_0000), (0, 0), "a placed resource on a title heap counts once, through the heap");
        release(ring);
        release(pr);
        release(heap);
        assert_eq!(net(heap_size), (1, 1));
        release(t);
        assert_eq!(net(want), (1, 1));
    }
}
