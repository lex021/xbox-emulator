use core::sync::atomic::{AtomicBool, Ordering};
use std::collections::HashSet;
use std::sync::Mutex;

pub const MSAA_ALIGN: u64 = 0x40_0000;

static CAPABLE: Mutex<Option<HashSet<usize>>> = Mutex::new(None);

pub fn heap_layout(size: u64, msaa: bool) -> (u64, u64) {
    if msaa || size >= MSAA_ALIGN {
        (size.div_ceil(MSAA_ALIGN) * MSAA_ALIGN, MSAA_ALIGN)
    } else {
        (size, 0)
    }
}

pub fn note_heap(heap: usize, align: u64) {
    if align >= MSAA_ALIGN {
        CAPABLE.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashSet::new).insert(heap);
    }
}

pub fn forget_heap(heap: usize) {
    if let Some(s) = CAPABLE.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        s.remove(&heap);
    }
}

pub fn capable(heap: usize) -> bool {
    CAPABLE.lock().unwrap_or_else(|e| e.into_inner()).as_ref().is_some_and(|s| s.contains(&heap))
}

pub fn max_align(samples: u32) -> u64 {
    if samples > 1 { MSAA_ALIGN } else { 0x1_0000 }
}

pub fn span_start(samples: u32, va: u64, lo: u64, align: u64) -> u64 {
    if samples > 1 {
        lo + (va - lo) % align
    } else {
        lo
    }
}

pub const TILE: u64 = 0x1_0000;

static TILES_REFUSED: AtomicBool = AtomicBool::new(false);

pub fn tiles_usable() -> bool {
    !TILES_REFUSED.load(Ordering::Acquire)
}

pub fn note_tiles_refused(hr: i32) {
    if !TILES_REFUSED.swap(true, Ordering::AcqRel) {
        compat_common::record("d3d12_xs", "msaa_place", &format!("reserved MSAA texture refused hr={hr:#010x}: MSAA placements off a 4 MB step fall back to committed resources from now on"));
    }
}

pub fn needs_tiles(samples: u32, offset: u64, align: u64, heap_capable: bool) -> bool {
    samples > 1 && align > TILE && (offset % align != 0 || !heap_capable)
}

pub fn span_align(samples: u32, align: u64) -> u64 {
    if samples > 1 && tiles_usable() { align.min(TILE) } else { align }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn msaa_heap_layout_and_span_rules() {
        assert_eq!(heap_layout(0x1_0000, false), (0x1_0000, 0));
        assert_eq!(heap_layout(0x1_0000, true), (MSAA_ALIGN, MSAA_ALIGN));
        assert_eq!(heap_layout(0x41_0000, false), (0x80_0000, MSAA_ALIGN));
        assert_eq!(max_align(1), 0x1_0000);
        assert_eq!(max_align(4), MSAA_ALIGN);
        assert_eq!(span_start(1, 0x1_2345_0000, 0x1_0000_0000, 0x1_0000), 0x1_0000_0000);
        assert_eq!(span_start(4, 0x1_0080_0000, 0x1_0000_0000, MSAA_ALIGN), 0x1_0000_0000);
        assert_eq!(span_start(4, 0x1_0081_0000, 0x1_0000_0000, MSAA_ALIGN), 0x1_0001_0000);
        note_heap(0x1234, MSAA_ALIGN);
        note_heap(0x5678, 0);
        assert!(capable(0x1234) && !capable(0x5678));
        forget_heap(0x1234);
        assert!(!capable(0x1234));
        assert!(needs_tiles(4, 0x31_0000, MSAA_ALIGN, true));
        assert!(!needs_tiles(4, 0x40_0000, MSAA_ALIGN, true));
        assert!(needs_tiles(4, 0x40_0000, MSAA_ALIGN, false), "a heap that is not 4 MB aligned cannot place MSAA directly at any offset");
        assert!(!needs_tiles(1, 0x31_0000, 0x1_0000, false));
        assert!(!needs_tiles(4, 0x31_0000, 0x1_0000, false), "small MSAA with 64 KB alignment places directly");
        assert_eq!(span_align(1, 0x1_0000), 0x1_0000);
        assert_eq!(span_align(4, MSAA_ALIGN), if tiles_usable() { TILE } else { MSAA_ALIGN });
    }
}
