use core::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, AtomicUsize, Ordering};

use crate::sys::*;

const SLOTS: usize = 16;

pub struct Acct {
    heap: AtomicUsize,
    allocated: AtomicI64,
    overhead: AtomicI64,
    title: AtomicBool,
}

static ACCT: [Acct; SLOTS] = [const { Acct { heap: AtomicUsize::new(0), allocated: AtomicI64::new(0), overhead: AtomicI64::new(0), title: AtomicBool::new(false) } }; SLOTS];

#[link(name = "kernel32")]
extern "system" {
    fn HeapSize(heap: Handle, flags: u32, mem: *const core::ffi::c_void) -> usize;
}

static CLAMPED: AtomicU64 = AtomicU64::new(0);

pub fn clamped_frees() -> u64 {
    CLAMPED.load(Ordering::Relaxed)
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
            let on = env_trimmed_is(b"TOMBOX_XMEM_HEAPACCT\0", b"0") != Some(true);
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

pub fn register_title(heap: usize, allocated: u64, overhead: u64) -> Option<&'static Acct> {
    let a = register(heap, allocated, overhead)?;
    a.title.store(true, Ordering::Release);
    Some(a)
}

pub fn is_title(heap: usize) -> bool {
    slot(heap).is_some_and(|a| a.title.load(Ordering::Acquire))
}

pub fn title_bytes() -> u64 {
    ACCT.iter()
        .filter(|a| a.title.load(Ordering::Acquire) && a.heap.load(Ordering::Acquire) != 0)
        .map(|a| {
            let (x, o) = a.read();
            x + o
        })
        .sum()
}

#[cfg(test)]
thread_local! {
    static LOCAL: [core::cell::Cell<(usize, i64)>; SLOTS] = const { [const { core::cell::Cell::new((0, 0)) }; SLOTS] };
}

#[cfg(test)]
pub fn local_delta(heap: usize) -> i64 {
    LOCAL.with(|m| m.iter().find(|c| c.get().0 == heap).map_or(0, |c| c.get().1))
}

#[cfg(test)]
fn local_add(heap: usize, d: i64) {
    LOCAL.with(|m| {
        if let Some(c) = m.iter().find(|c| c.get().0 == heap).or_else(|| m.iter().find(|c| c.get().0 == 0)) {
            let (_, v) = c.get();
            c.set((heap, v + d));
        }
    });
}

impl Acct {
    pub fn add(&self, size: usize) {
        self.allocated.fetch_add(size as i64, Ordering::AcqRel);
        self.overhead.fetch_add(block_overhead(size), Ordering::AcqRel);
        #[cfg(test)]
        local_add(self.heap.load(Ordering::Acquire), size as i64);
    }

    pub fn sub(&self, size: usize) {
        let mut clamped = false;
        let take = |field: &AtomicI64, n: i64, clamped: &mut bool| {
            let _ = field.fetch_update(Ordering::AcqRel, Ordering::Acquire, |v| {
                *clamped |= v < n;
                Some((v - n).max(0))
            });
        };
        take(&self.allocated, size as i64, &mut clamped);
        take(&self.overhead, block_overhead(size), &mut clamped);
        if clamped {
            CLAMPED.fetch_add(1, Ordering::Relaxed);
        }
        #[cfg(test)]
        local_add(self.heap.load(Ordering::Acquire), -(size as i64));
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

    #[test]
    fn unmatched_frees_clamp_instead_of_hiding_later_allocations() {
        let t = register_title(0x5180_0000, 0, 0).unwrap();
        t.add(0x100);
        let before = clamped_frees();
        t.sub(0x40_0000);
        assert_eq!(t.read(), (0, 0));
        assert!(clamped_frees() > before);
        t.add(0x10_0000);
        assert_eq!(t.read(), (0x10_0000, block_overhead(0x10_0000) as u64));
        t.sub(0x10_0000);
        assert_eq!(t.read(), (0, 0));
    }

    #[test]
    fn only_title_heaps_feed_title_bytes() {
        let other = register(0x5160_0000, 0x1000_0000, 0).unwrap();
        other.add(0x100_0000);
        let t = register_title(0x5170_0000, 0, 0).unwrap();
        t.add(0x30);
        assert!(title_bytes() >= 0x30 + block_overhead(0x30) as u64);
        assert!(!other.title.load(Ordering::Acquire));
        assert_eq!(local_delta(0x5170_0000), 0x30);
        assert_eq!(local_delta(0x5160_0000), 0x100_0000);
        assert_eq!(t.read(), (0x30, block_overhead(0x30) as u64));
    }
}
