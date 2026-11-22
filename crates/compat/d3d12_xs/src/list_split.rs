use core::ffi::c_void;
use core::sync::atomic::{AtomicUsize, Ordering};
use std::collections::HashMap;
use std::sync::Mutex;
use compat_common::record;

use crate::device_wrap::Guid;
use crate::gpu_wait::{self, Op};

const SLOT_QI: usize = 0;
const SLOT_ADDREF: usize = 1;
const SLOT_RELEASE: usize = 2;
const SLOT_CLOSE: usize = 9;
const SLOT_RESET: usize = 10;
const CHILD_GET_DEVICE: usize = 7;
const DEV_CREATE_ALLOCATOR: usize = 9;
const DEV_CREATE_LIST: usize = 12;
const ALLOC_RESET: usize = 8;
const FENCE_COMPLETED: usize = 8;
const IID_DEVICE: Guid = Guid { d1: 0x1898_19f1, d2: 0x1db6, d3: 0x4b57, d4: [0xbe, 0x54, 0x18, 0x21, 0x33, 0x9b, 0x85, 0xf7] };
const IID_ALLOCATOR: Guid = Guid { d1: 0x6102_dee4, d2: 0xaf59, d3: 0x4b09, d4: [0xb9, 0x99, 0xb4, 0x4d, 0x73, 0xf0, 0x9b, 0x24] };
const IID_GRAPHICS_LIST: Guid = Guid { d1: 0x5b16_0d0f, d2: 0xac1b, d3: 0x4185, d4: [0x8b, 0xa8, 0xb3, 0xae, 0x42, 0xa5, 0xa4, 0x55] };

#[derive(Default)]
pub struct Registry {
    prev: HashMap<usize, usize>,
    origin: HashMap<usize, usize>,
}

impl Registry {
    pub fn add(&mut self, prev: usize, seg: usize) {
        let o = self.origin_of(prev);
        self.prev.insert(seg, prev);
        self.origin.insert(seg, o);
    }

    pub fn origin_of(&self, l: usize) -> usize {
        self.origin.get(&l).copied().unwrap_or(l)
    }

    pub fn chain(&self, l: usize) -> Vec<usize> {
        let mut v = vec![l];
        let mut c = l;
        while let Some(&p) = self.prev.get(&c) {
            v.push(p);
            c = p;
        }
        v.reverse();
        v
    }

    pub fn replace(&mut self, old: usize, new: usize) {
        if let Some(p) = self.prev.remove(&old) {
            self.prev.insert(new, p);
        }
        if let Some(o) = self.origin.remove(&old) {
            self.origin.insert(new, o);
        }
        for v in self.prev.values_mut() {
            if *v == old {
                *v = new;
            }
        }
    }

    pub fn take(&mut self, origin: usize) -> Vec<usize> {
        let segs: Vec<usize> = self.origin.iter().filter(|(_, &o)| o == origin).map(|(&s, _)| s).collect();
        for s in &segs {
            self.origin.remove(s);
            self.prev.remove(s);
        }
        segs
    }
}

struct Pooled {
    list: usize,
    alloc: usize,
    ty: u32,
    busy: bool,
    fence: usize,
    value: u64,
}

static REG: Mutex<Option<Registry>> = Mutex::new(None);
static POOL: Mutex<Vec<Pooled>> = Mutex::new(Vec::new());
static QFENCE: Mutex<Option<HashMap<usize, (usize, u64)>>> = Mutex::new(None);
static PREV: [AtomicUsize; 11] = [const { AtomicUsize::new(0) }; 11];
static LOG: AtomicUsize = AtomicUsize::new(0);

fn reg<R>(f: impl FnOnce(&mut Registry) -> R) -> R {
    f(REG.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(Registry::default))
}

fn once(n: usize) -> bool {
    LOG.fetch_add(1, Ordering::Relaxed) < n
}

unsafe fn vslot(obj: *mut c_void, i: usize) -> usize {
    unsafe { *(*(obj as *const *const usize)).add(i) }
}

unsafe fn fence_done(fence: usize, value: u64) -> bool {
    if fence == 0 {
        return true;
    }
    let f: unsafe extern "system" fn(*mut c_void) -> u64 = unsafe { core::mem::transmute(vslot(fence as *mut c_void, FENCE_COMPLETED)) };
    (unsafe { f(fence as *mut c_void) }) >= value
}

unsafe fn device_of(list: *mut c_void) -> *mut c_void {
    let get: unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> i32 = unsafe { core::mem::transmute(vslot(list, CHILD_GET_DEVICE)) };
    let mut d: *mut c_void = core::ptr::null_mut();
    if unsafe { get(list, &IID_DEVICE, &mut d) } < 0 || d.is_null() {
        return core::ptr::null_mut();
    }
    let release: unsafe extern "system" fn(*mut c_void) -> u32 = unsafe { core::mem::transmute(vslot(d, 2)) };
    unsafe { release(d) };
    d
}

unsafe fn acquire(like: *mut c_void) -> *mut c_void {
    let ty = unsafe { crate::fill::list_type(like) };
    {
        let mut pool = POOL.lock().unwrap_or_else(|e| e.into_inner());
        for p in pool.iter_mut().filter(|p| !p.busy && p.ty == ty) {
            if !unsafe { fence_done(p.fence, p.value) } {
                continue;
            }
            let ar: unsafe extern "system" fn(*mut c_void) -> i32 = unsafe { core::mem::transmute(vslot(p.alloc as *mut c_void, ALLOC_RESET)) };
            if unsafe { ar(p.alloc as *mut c_void) } < 0 {
                continue;
            }
            let lr: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32 = unsafe { core::mem::transmute(vslot(p.list as *mut c_void, SLOT_RESET)) };
            if unsafe { lr(p.list as *mut c_void, p.alloc as *mut c_void, core::ptr::null_mut()) } < 0 {
                continue;
            }
            p.busy = true;
            p.fence = 0;
            return p.list as *mut c_void;
        }
    }
    let dev = unsafe { device_of(like) };
    if dev.is_null() {
        return core::ptr::null_mut();
    }
    let ca: unsafe extern "system" fn(*mut c_void, u32, *const Guid, *mut *mut c_void) -> i32 = unsafe { core::mem::transmute(vslot(dev, DEV_CREATE_ALLOCATOR)) };
    let mut a: *mut c_void = core::ptr::null_mut();
    let hr = unsafe { ca(dev, ty, &IID_ALLOCATOR, &mut a) };
    if hr < 0 || a.is_null() {
        record("d3d12_xs", "list_split", &format!("segment allocator create FAILED hr={hr:#010x}"));
        return core::ptr::null_mut();
    }
    let cl: unsafe extern "system" fn(*mut c_void, u32, u32, *mut c_void, *mut c_void, *const Guid, *mut *mut c_void) -> i32 = unsafe { core::mem::transmute(vslot(dev, DEV_CREATE_LIST)) };
    let mut l: *mut c_void = core::ptr::null_mut();
    let hr = unsafe { cl(dev, 0, ty, a, core::ptr::null_mut(), &IID_GRAPHICS_LIST, &mut l) };
    if hr < 0 || l.is_null() {
        record("d3d12_xs", "list_split", &format!("segment list create FAILED hr={hr:#010x}"));
        let release: unsafe extern "system" fn(*mut c_void) -> u32 = unsafe { core::mem::transmute(vslot(a, 2)) };
        unsafe { release(a) };
        return core::ptr::null_mut();
    }
    POOL.lock().unwrap_or_else(|e| e.into_inner()).push(Pooled { list: l as usize, alloc: a as usize, ty, busy: true, fence: 0, value: 0 });
    l
}

unsafe fn give_back(seg: usize) {
    crate::indirect_bundle::forget(seg);
    let mut pool = POOL.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(p) = pool.iter_mut().find(|p| p.list == seg) {
        p.busy = false;
    }
}

pub(crate) unsafe fn acquire_part(like: *mut c_void) -> *mut c_void {
    unsafe { acquire(like) }
}

pub(crate) unsafe fn abandon_part(seg: *mut c_void) {
    let close: unsafe extern "system" fn(*mut c_void) -> i32 = unsafe { core::mem::transmute(vslot(seg, SLOT_CLOSE)) };
    unsafe { close(seg) };
    unsafe { give_back(seg as usize) };
}

pub(crate) unsafe fn retire_part(seg: *mut c_void) {
    unsafe { give_back(seg as usize) };
}

pub(crate) unsafe fn part_idle(seg: usize) -> bool {
    let pool = POOL.lock().unwrap_or_else(|e| e.into_inner());
    pool.iter().find(|p| p.list == seg).map_or(true, |p| unsafe { fence_done(p.fence, p.value) })
}

pub(crate) fn link(prev: *mut c_void, seg: *mut c_void) {
    reg(|r| r.add(prev as usize, seg as usize));
}

pub(crate) fn relink(old: *mut c_void, new: *mut c_void) {
    reg(|r| r.replace(old as usize, new as usize));
}

pub unsafe fn split_for_wait(list: *mut c_void, op: Op) -> bool {
    let queue = crate::cl2_state::queue_of(list);
    if queue != 0 && gpu_wait::has_queue_fns() {
        unsafe {
            if crate::cl2_state::close_part(list) < 0 {
                return false;
            }
            crate::cl2_state::submit_part(list, queue as *mut c_void);
            gpu_wait::wait_now(queue as *mut c_void, op, crate::fill::list_type(list));
            crate::cl2_state::reopen(list);
        }
        return true;
    }
    let Some(shim) = crate::cmdlist_x::shim_of(list) else { return false };
    let seg = unsafe { acquire(list) };
    if seg.is_null() {
        return false;
    }
    if unsafe { crate::cl2_state::close_part(list) } < 0 {
        let close: unsafe extern "system" fn(*mut c_void) -> i32 = unsafe { core::mem::transmute(vslot(seg, SLOT_CLOSE)) };
        unsafe { close(seg) };
        unsafe { give_back(seg as usize) };
        return false;
    }
    reg(|r| r.add(list as usize, seg as usize));
    crate::cl2_state::clone_state(list, seg);
    unsafe {
        crate::cl2_state::reapply(seg);
        crate::cmdlist_wrap::set_real(shim as *mut c_void, seg);
    }
    crate::cmdlist_x::inherit_shim(list, seg);
    gpu_wait::note(seg, op);
    if once(8) {
        record("d3d12_xs", "list_split", &format!("wait in list {list:p}: recorded part closed, recording continues on segment {seg:p}; the queue waits between the two at submit"));
    }
    true
}

pub fn origin_of(list: *mut c_void) -> usize {
    reg(|r| r.origin_of(list as usize))
}

pub unsafe fn continue_on_segment(list: *mut c_void) -> *mut c_void {
    let Some(shim) = crate::cmdlist_x::shim_of(list) else { return core::ptr::null_mut() };
    let seg = unsafe { acquire(list) };
    if seg.is_null() {
        return seg;
    }
    reg(|r| r.add(list as usize, seg as usize));
    crate::cl2_state::clone_state(list, seg);
    unsafe { crate::cmdlist_wrap::set_real(shim as *mut c_void, seg) };
    crate::cmdlist_x::inherit_shim(list, seg);
    seg
}

pub fn expand(lists: &[*mut c_void]) -> Vec<*mut c_void> {
    let g = REG.lock().unwrap_or_else(|e| e.into_inner());
    let Some(r) = g.as_ref() else { return lists.to_vec() };
    lists.iter().flat_map(|&l| r.chain(l as usize).into_iter().map(|x| x as *mut c_void)).collect()
}

pub unsafe fn after_submit(queue: *mut c_void, lists: &[*mut c_void]) {
    let segs: Vec<usize> = {
        let pool = POOL.lock().unwrap_or_else(|e| e.into_inner());
        lists.iter().map(|&l| l as usize).filter(|l| pool.iter().any(|p| p.list == *l)).collect()
    };
    if segs.is_empty() {
        return;
    }
    let signal = gpu_wait::signal_fn();
    if signal == 0 {
        return;
    }
    let (fence, value) = {
        let mut g = QFENCE.lock().unwrap_or_else(|e| e.into_inner());
        let m = g.get_or_insert_with(HashMap::new);
        if !m.contains_key(&(queue as usize)) {
            let f = unsafe { gpu_wait::new_fence(queue) };
            if f == 0 {
                return;
            }
            m.insert(queue as usize, (f, 0));
        }
        let e = m.get_mut(&(queue as usize)).unwrap();
        e.1 += 1;
        *e
    };
    let s: unsafe extern "system" fn(*mut c_void, *mut c_void, u64) -> i32 = unsafe { core::mem::transmute(signal) };
    unsafe { s(queue, fence as *mut c_void, value) };
    for p in POOL.lock().unwrap_or_else(|e| e.into_inner()).iter_mut().filter(|p| segs.contains(&p.list)) {
        p.fence = fence;
        p.value = value;
    }
}

fn prev(slot: usize) -> usize {
    PREV[slot].load(Ordering::Relaxed)
}

fn origin(this: *mut c_void) -> *mut c_void {
    reg(|r| r.origin_of(this as usize)) as *mut c_void
}

unsafe extern "system" fn h_qi(this: *mut c_void, riid: *const Guid, out: *mut *mut c_void) -> i32 {
    let f: unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> i32 = unsafe { core::mem::transmute(prev(SLOT_QI)) };
    unsafe { f(origin(this), riid, out) }
}

unsafe extern "system" fn h_addref(this: *mut c_void) -> u32 {
    let f: unsafe extern "system" fn(*mut c_void) -> u32 = unsafe { core::mem::transmute(prev(SLOT_ADDREF)) };
    unsafe { f(origin(this)) }
}

unsafe extern "system" fn h_release(this: *mut c_void) -> u32 {
    let f: unsafe extern "system" fn(*mut c_void) -> u32 = unsafe { core::mem::transmute(prev(SLOT_RELEASE)) };
    let o = origin(this);
    let n = unsafe { f(o) };
    if crate::indirect_bundle::title_released(o as usize, n) {
        crate::console_slots_cl2::forget_bundle(o as usize);
    }
    n
}

unsafe extern "system" fn h_reset(this: *mut c_void, alloc: *mut c_void, pso: *mut c_void) -> i32 {
    let o = origin(this);
    let segs = reg(|r| r.take(o as usize));
    if !segs.is_empty() {
        let shim = crate::cmdlist_x::shim_of(this);
        for s in segs {
            if s == this as usize && crate::cl2_state::is_open(this) {
                let close: unsafe extern "system" fn(*mut c_void) -> i32 = unsafe { core::mem::transmute(vslot(this, SLOT_CLOSE)) };
                unsafe { crate::gds::on_close(this) };
                unsafe { close(this) };
                crate::cl2_state::mark_closed(this);
            }
            gpu_wait::on_close(s as *mut c_void);
            unsafe { give_back(s) };
        }
        if let Some(shim) = shim {
            unsafe { crate::cmdlist_wrap::set_real(shim as *mut c_void, o) };
        }
    }
    let f: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32 = unsafe { core::mem::transmute(prev(SLOT_RESET)) };
    unsafe { f(o, alloc, pso) }
}

pub fn install(vt: &[AtomicUsize]) -> usize {
    let mut n = 0;
    for (slot, f) in [
        (SLOT_QI, h_qi as usize),
        (SLOT_ADDREF, h_addref as usize),
        (SLOT_RELEASE, h_release as usize),
        (SLOT_RESET, h_reset as usize),
    ] {
        let p = vt[slot].load(Ordering::Relaxed);
        if p == 0 || p == f {
            continue;
        }
        PREV[slot].store(p, Ordering::Relaxed);
        vt[slot].store(f, Ordering::Relaxed);
        n += 1;
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segments_chain_back_to_the_title_list_and_are_returned_on_reset() {
        let mut r = Registry::default();
        assert_eq!(r.chain(0x10), vec![0x10]);
        r.add(0x10, 0x20);
        r.add(0x20, 0x30);
        assert_eq!(r.chain(0x30), vec![0x10, 0x20, 0x30]);
        assert_eq!(r.origin_of(0x30), 0x10);
        assert_eq!(r.origin_of(0x20), 0x10);
        assert_eq!(r.origin_of(0x10), 0x10);
        r.add(0x50, 0x60);
        let mut t = r.take(0x10);
        t.sort();
        assert_eq!(t, vec![0x20, 0x30]);
        assert_eq!(r.chain(0x30), vec![0x30]);
        assert_eq!(r.chain(0x60), vec![0x50, 0x60]);
    }

    #[test]
    fn a_middle_part_is_replaced_in_place_in_the_chain() {
        let mut r = Registry::default();
        r.add(0x10, 0x20);
        r.add(0x20, 0x30);
        r.replace(0x20, 0x40);
        assert_eq!(r.chain(0x30), vec![0x10, 0x40, 0x30]);
        assert_eq!(r.origin_of(0x40), 0x10);
        assert_eq!(r.origin_of(0x20), 0x20);
        let mut t = r.take(0x10);
        t.sort();
        assert_eq!(t, vec![0x30, 0x40]);
    }

    #[test]
    fn install_chains_identity_and_reset() {
        let vt: Vec<AtomicUsize> = (0..32).map(|i| AtomicUsize::new(100 + i)).collect();
        assert_eq!(install(&vt), 4);
        assert_eq!(vt[0].load(Ordering::Relaxed), h_qi as usize);
        assert_eq!(vt[1].load(Ordering::Relaxed), h_addref as usize);
        assert_eq!(vt[10].load(Ordering::Relaxed), h_reset as usize);
        assert_eq!(prev(SLOT_RESET), 110);
        assert_eq!(vt[9].load(Ordering::Relaxed), 109);

        static SEEN: Mutex<Vec<(&'static str, usize)>> = Mutex::new(Vec::new());
        unsafe extern "system" fn qi(this: *mut c_void, _r: *const Guid, out: *mut *mut c_void) -> i32 {
            SEEN.lock().unwrap().push(("qi", this as usize));
            unsafe { *out = this };
            0
        }
        unsafe extern "system" fn addref(this: *mut c_void) -> u32 {
            SEEN.lock().unwrap().push(("addref", this as usize));
            2
        }
        unsafe extern "system" fn release(this: *mut c_void) -> u32 {
            SEEN.lock().unwrap().push(("release", this as usize));
            1
        }
        unsafe extern "system" fn reset(this: *mut c_void, _a: *mut c_void, _p: *mut c_void) -> i32 {
            SEEN.lock().unwrap().push(("reset", this as usize));
            0
        }
        for (s, f) in [(SLOT_QI, qi as usize), (SLOT_ADDREF, addref as usize), (SLOT_RELEASE, release as usize), (SLOT_RESET, reset as usize)] {
            PREV[s].store(f, Ordering::Relaxed);
        }
        let (r, n) = (0x7_1000usize, 0x7_2000usize);
        reg(|g| g.add(r, n));
        let np = n as *mut c_void;
        let mut out: *mut c_void = core::ptr::null_mut();
        unsafe {
            assert_eq!(h_qi(np, core::ptr::null(), &mut out), 0);
            assert_eq!(out as usize, r);
            h_addref(np);
            h_release(np);
            assert_eq!(h_reset(np, core::ptr::null_mut(), core::ptr::null_mut()), 0);
        }
        assert_eq!(origin_of(np), n);
        let seen = SEEN.lock().unwrap().clone();
        assert_eq!(seen, vec![("qi", r), ("addref", r), ("release", r), ("reset", r)]);
    }
}
