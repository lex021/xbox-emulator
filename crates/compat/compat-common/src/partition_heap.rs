use core::ffi::c_void;

type Handle = *mut c_void;
const HEAP_FLAGS: u32 = 0x102;
pub const ERROR_BAD_ARGUMENTS: u32 = 0xa0;

#[link(name = "ntdll", kind = "raw-dylib")]
extern "system" {
    fn RtlCreateHeap(flags: u32, base: *mut c_void, reserve: usize, commit: usize, lock: *mut c_void, params: *mut c_void) -> Handle;
}

#[link(name = "kernel32")]
extern "system" {
    fn SetLastError(code: u32);
}

#[repr(C)]
#[derive(Default)]
pub struct SegmentHeapParameters {
    pub version_size: u32,
    pub flags: u32,
    pub source_flags: u32,
    pub memory_type_mask: u32,
    pub numa_node: u32,
    pub partition: usize,
    pub reserved: [usize; 2],
    pub tail: [u64; 4],
}

pub fn segment_parameters(partition: usize, memory_type: u32) -> SegmentHeapParameters {
    let mut mask = 1u32.wrapping_shl(memory_type);
    if memory_type == 4 {
        mask |= 8;
    }
    SegmentHeapParameters {
        version_size: 0x50_0003,
        numa_node: 0xffff_ffff,
        memory_type_mask: mask & 0xff,
        partition: if memory_type >= 2 { partition } else { 0 },
        ..Default::default()
    }
}

pub unsafe fn partition_create_heap(partition: usize, memory_type: u32, flags: u32, base: *mut c_void, reserve: usize, commit: usize, lock: *mut c_void, params: *mut c_void) -> Handle {
    let mut built = segment_parameters(partition, memory_type);
    let (flags, reserve, commit, params) = if params.is_null() {
        (flags | HEAP_FLAGS, 0, 0, (&mut built as *mut SegmentHeapParameters).cast())
    } else {
        (flags, reserve, commit, params)
    };
    let mut h = RtlCreateHeap(flags, base, reserve, commit, lock, params);
    if h.is_null() && params == (&mut built as *mut SegmentHeapParameters).cast() && partition == 0 && built.memory_type_mask != 1 {
        built.memory_type_mask = 1;
        h = RtlCreateHeap(flags, base, reserve, commit, lock, params);
    }
    if h.is_null() {
        SetLastError(ERROR_BAD_ARGUMENTS);
    }
    h
}
