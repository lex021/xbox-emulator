//! Recreate tracked buffer descriptors in a private heap for a suspended shader.
use super::{get_private, slot, Guid, TABLE_LAYOUT};
use tombox_xshader::meta::{RootParamKind, RootSig};
use crate::bindtrace::{self, Desc};
use crate::cl2_state::{ComputeBindings, ComputeBuffer};
use crate::gws_scratch::Resource;
use core::ffi::c_void;
use core::ptr::null_mut;

const IID_HEAP: Guid = Guid {
    d1: 0x8efb471d, d2: 0x616c, d3: 0x4f49,
    d4: [0x90, 0xf7, 0x12, 0x7b, 0xb7, 0x63, 0xfa, 0x51],
};
const IID_DEVICE: Guid = Guid {
    d1: 0x189819f1, d2: 0x1db6, d3: 0x4b57,
    d4: [0xbe, 0x54, 0x18, 0x21, 0x33, 0x9b, 0x85, 0xf7],
};
const MAX_DESCRIPTORS: usize = 64;
const DIRECT_HEAP_INDEXING: u32 = 0x400 | 0x800;

pub(super) struct Table {
    heap: usize,
    handles: Vec<(u32, u64)>,
}

impl Table {
    pub(super) unsafe fn bind(&self, list: *mut c_void) {
        let heaps: unsafe extern "system" fn(*mut c_void, u32, *const usize) =
            unsafe { core::mem::transmute(slot(list, 28)) };
        let set: unsafe extern "system" fn(*mut c_void, u32, u64) =
            unsafe { core::mem::transmute(slot(list, 31)) };
        unsafe { heaps(list, 1, &self.heap) };
        for &(index, handle) in &self.handles {
            unsafe { set(list, index, handle) };
        }
    }
}

pub(super) fn root_spans(root: &RootSig) -> [u32; 64] {
    let mut spans = [0; 64];
    if root.flags & DIRECT_HEAP_INDEXING != 0 {
        return spans;
    }
    for (span, parameter) in spans.iter_mut().zip(&root.params) {
        let RootParamKind::Table(ranges) = &parameter.kind else { continue };
        let mut end = 0u32;
        let mut next = 0u32;
        for range in ranges {
            let start = if range.offset_in_table == u32::MAX { next } else { range.offset_in_table };
            let Some(last) = start.checked_add(range.num) else {
                end = 0;
                break;
            };
            if range.range_type > 2 || range.num == 0 || last as usize > MAX_DESCRIPTORS {
                end = 0;
                break;
            }
            next = last;
            end = end.max(last);
        }
        *span = end;
    }
    spans
}

unsafe fn heap_handle(heap: *mut c_void, index: usize) -> u64 {
    let get: unsafe extern "system" fn(*mut c_void, *mut u64) -> u64 =
        unsafe { core::mem::transmute(slot(heap, index)) };
    let mut handle = 0;
    let out = &mut handle as *mut u64;
    let rax = unsafe { get(heap, out) };
    if rax == out as u64 { handle } else { rax }
}

pub(super) unsafe fn snapshot(
    list: *mut c_void,
    root: *mut c_void,
    original: usize,
    bindings: &ComputeBindings,
    buffers: &mut Vec<ComputeBuffer>,
) -> Option<(Table, Resource)> {
    let original = original as *mut c_void;
    if original.is_null() || bindings.table_handles().next().is_none() {
        return None;
    }
    let mut spans = [0u32; 64];
    if !unsafe { get_private(root, TABLE_LAYOUT, spans.as_mut_ptr().cast(), 256) } {
        return None;
    }
    let mut count = 0usize;
    let mut ranges = Vec::new();
    let mut handles = Vec::new();
    for (index, handle) in bindings.table_handles() {
        let span = *spans.get(index as usize)? as usize;
        if span == 0 || count + span > MAX_DESCRIPTORS {
            return None;
        }
        handles.push((index, count as u64));
        ranges.push((handle, span));
        count += span;
    }
    let (cpu, gpu) = unsafe { (heap_handle(original, 9) as usize, heap_handle(original, 10)) };
    let (inc, descriptors) = bindtrace::descriptor_snapshot(cpu, gpu, &ranges)?;
    for descriptor in &descriptors {
        let (address, state) = match descriptor {
            Desc::Cbv { va, .. } => (*va, 0x1),
            Desc::Srv { res, desc: Some(desc) } if *res != 0 && desc[4..8] == 1u32.to_le_bytes() =>
                (crate::gpu_va::buffer_base(*res)?, 0x40),
            // Raw and typed buffer UAVs have no structured-buffer counter.
            Desc::Uav { res, desc: Some(desc) } if *res != 0
                && desc[4..8] == 1u32.to_le_bytes() && desc[..4] != [0; 4] =>
                (crate::gpu_va::buffer_base(*res)?, 0x8),
            _ => return None,
        };
        buffers.push(ComputeBuffer::lookup(address, state)?);
    }

    let get: unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> i32 =
        unsafe { core::mem::transmute(slot(list, 7)) };
    let mut device = null_mut();
    if unsafe { get(list, &IID_DEVICE, &mut device) } < 0 || device.is_null() {
        return None;
    }
    let _device = Resource(device as usize);
    let create: unsafe extern "system" fn(*mut c_void, *const u32, *const Guid, *mut *mut c_void) -> i32 =
        unsafe { core::mem::transmute(slot(device, 14)) };
    let mut heap = null_mut();
    if unsafe { create(device, [0, count as u32, 1, 0].as_ptr(), &IID_HEAP, &mut heap) } < 0 || heap.is_null() {
        return None;
    }
    let owned = Resource(heap as usize);
    let (saved_cpu, saved_gpu) = unsafe { (heap_handle(heap, 9) as usize, heap_handle(heap, 10)) };
    if saved_cpu == 0 || saved_gpu == 0 {
        return None;
    }
    for (index, descriptor) in descriptors.into_iter().enumerate() {
        let dest = saved_cpu + index * inc;
        unsafe { recreate(device, descriptor, dest) };
    }
    for (_, handle) in &mut handles {
        *handle = saved_gpu + *handle * inc as u64;
    }
    Some((Table { heap: heap as usize, handles }, owned))
}

unsafe fn recreate(device: *mut c_void, descriptor: Desc, dest: usize) {
    match descriptor {
        Desc::Cbv { va, size } => {
            #[repr(C)]
            struct Cbv { address: u64, size: u32, padding: u32 }
            let create: unsafe extern "system" fn(*mut c_void, *const Cbv, usize) =
                unsafe { core::mem::transmute(slot(device, 17)) };
            unsafe { create(device, &Cbv { address: va, size, padding: 0 }, dest) };
        }
        Desc::Srv { res, desc: Some(desc) } => {
            let create: unsafe extern "system" fn(*mut c_void, usize, *const u8, usize) =
                unsafe { core::mem::transmute(slot(device, 18)) };
            unsafe { create(device, res, desc.as_ptr(), dest) };
        }
        Desc::Uav { res, desc: Some(desc) } => {
            let create: unsafe extern "system" fn(*mut c_void, usize, *mut c_void, *const u8, usize) =
                unsafe { core::mem::transmute(slot(device, 19)) };
            unsafe { create(device, res, null_mut(), desc.as_ptr(), dest) };
        }
        _ => unreachable!("descriptor was validated during collection"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tombox_xshader::meta::{RootParam, RootRange};

    #[test]
    fn gws_table_spans_reject_unbounded_and_direct_indexing() {
        let range = |offset_in_table, num| RootRange {
            range_type: 1, num, base_register: 0, space: 0, offset_in_table,
        };
        let mut root = RootSig {
            version: 2,
            flags: 0,
            params: vec![
                RootParam { visibility: 0, kind: RootParamKind::Table(vec![range(4, 2), range(u32::MAX, 3)]) },
                RootParam { visibility: 0, kind: RootParamKind::Table(vec![range(0, u32::MAX)]) },
            ],
            num_static_samplers: 0,
            static_samplers: Vec::new(),
        };
        assert_eq!(&root_spans(&root)[..2], &[9, 0]);
        root.flags = 0x400;
        assert_eq!(root_spans(&root), [0; 64]);
    }
}
