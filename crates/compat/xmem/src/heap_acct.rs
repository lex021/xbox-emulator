use core::sync::atomic::{AtomicI64, AtomicUsize, Ordering};

use crate::sys::*;

const SLOTS: usize = 16;

pub struct Acct {
    heap: AtomicUsize,
    allocated: AtomicI64,
    overhead: AtomicI64,
}

static ACCT: [Acct; SLOTS] = [const { Acct { heap: AtomicUsize::new(0), allocated: AtomicI64::new(0), overhead: AtomicI64::new(0) } }; SLOTS];

#[link(name = "kernel32")]
extern "system" {
    fn HeapSize(heap: Handle, flags: u32, mem: *const core::ffi::c_void) -> usize;
}

pub fn block_overhead(size: usize) -> i64 {
    (((size + 15) & !15) - size + 16) as i64
}

pub fn enabled() -> bool {
    static ON: AtomicUsize = AtomicUsize::new(2);
    match ON.load(Ordering::Relaxed) {
        0 => false,
        1 => true,
        _ => {
            let on = std::env::var("TOMBOX_XMEM_HEAPACCT").map_or(true, |v| v.trim() != "0");
            ON.store(on as usize, Ordering::Relaxed);
            on
        }
    }
}

pub fn slot(heap: usize) -> Option<&'static Acct> {
    if heap == 0 || !enabled() {
        return None;
    }
    ACCT.iter().find(|a| a.heap.load(Ordering::Acquire) == heap)
}

pub fn register(heap: usize, allocated: u64, overhead: u64) -> Option<&'static Acct> {
    if !enabled() {
        return None;
    }
    if let Some(a) = slot(heap) {
        return Some(a);
    }
    for a in ACCT.iter() {
        if a.heap.compare_exchange(0, usize::MAX, Ordering::AcqRel, Ordering::Acquire).is_ok() {
            a.allocated.store(allocated as i64, Ordering::Release);
            a.overhead.store(overhead as i64, Ordering::Release);
            a.heap.store(heap, Ordering::Release);
            return Some(a);
        }
    }
    None
}

impl Acct {
    pub fn add(&self, size: usize) {
        self.allocated.fetch_add(size as i64, Ordering::AcqRel);
        self.overhead.fetch_add(block_overhead(size), Ordering::AcqRel);
    }

    pub fn sub(&self, size: usize) {
        self.allocated.fetch_sub(size as i64, Ordering::AcqRel);
        self.overhead.fetch_sub(block_overhead(size), Ordering::AcqRel);
    }

    pub fn read(&self) -> (u64, u64) {
        (self.allocated.load(Ordering::Acquire).max(0) as u64, self.overhead.load(Ordering::Acquire).max(0) as u64)
    }
}

pub unsafe fn block_size(heap: usize, flags: u32, ptr: usize) -> Option<usize> {
    if ptr == 0 {
        return None;
    }
    let n = HeapSize(heap as Handle, flags & 0x1, ptr as *const core::ffi::c_void);
    if n == usize::MAX {
        None
    } else {
        Some(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overhead_rounds_to_granularity_plus_header() {
        assert_eq!(block_overhead(1), 31);
        assert_eq!(block_overhead(16), 16);
        assert_eq!(block_overhead(17), 31);
    }

    #[test]
    fn counters_are_disjoint_and_never_negative() {
        let a = register(0x5150_0000, 100, 40).unwrap();
        a.add(20);
        assert_eq!(a.read(), (120, 40 + block_overhead(20) as u64));
        a.sub(20);
        a.sub(500);
        a.sub(500);
        a.sub(500);
        assert_eq!(a.read(), (0, 0));
        assert!(core::ptr::eq(register(0x5150_0000, 0, 0).unwrap(), a));
    }
}
