use compat_common::record;
use core::ffi::c_void;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use crate::cl_ext::Bop64;
use crate::device_wrap::Guid;

const DEVICE: Guid = Guid { d1: 0x189819f1, d2: 0x1db6, d3: 0x4b57, d4: [0xbe, 0x54, 0x18, 0x21, 0x33, 0x9b, 0x85, 0xf7] };
const RESOURCE: Guid = Guid { d1: 0x696442be, d2: 0xa72e, d3: 0x4059, d4: [0xbc, 0x79, 0x5b, 0x5c, 0x98, 0x04, 0x0f, 0xad] };
const COPY_DEST: u32 = 0x400;
const GENERIC_READ: u32 = 0xac3;

extern "system" {
    fn GetCurrentProcess() -> *mut c_void;
    fn WriteProcessMemory(process: *mut c_void, dst: *mut c_void, src: *const u64, bytes: usize, written: *mut usize) -> i32;
}

unsafe fn slot(object: usize, index: usize) -> usize {
    unsafe { *(*(object as *const *const usize)).add(index) }
}

struct Com(usize);
impl Drop for Com {
    fn drop(&mut self) {
        unsafe {
            let release: extern "system" fn(usize) -> u32 = core::mem::transmute(slot(self.0, 2));
            release(self.0);
        }
    }
}

struct Write {
    target: usize,
    bytes: usize,
    kind: Bop64,
    readback: Com,
    mapped: usize,
    _upload: Option<Com>,
}

static LISTS: Mutex<Option<HashMap<usize, Vec<Arc<Write>>>>> = Mutex::new(None);

pub(crate) fn accepts(va: u64) -> bool {
    accepts_width(va, 8)
}

pub(crate) fn accepts_width(va: u64, bytes: usize) -> bool {
    va != 0 && va % bytes as u64 == 0
        && crate::gpu_va::lookup_buffer(va).is_none()
        && crate::gpu_va::lookup_gpu(va).is_none()
        && crate::title_shadow::is_writable_range(va as usize, bytes)
}

pub(crate) fn reset(list: *mut c_void) {
    if let Some(lists) = LISTS.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        lists.remove(&(list as usize));
    }
}

unsafe fn buffer(device: usize, heap: u32, state: u32) -> Option<(Com, usize)> {
    let props = [heap, 0, 0, 1, 1];
    let mut desc = [0u8; 56];
    desc[0] = 1; // BUFFER
    desc[16] = 8; // Width
    desc[24] = 1; // Height
    desc[28] = 1; // DepthOrArraySize
    desc[30] = 1; // MipLevels
    desc[36] = 1; // SampleDesc.Count
    desc[44] = 1; // ROW_MAJOR
    let create: extern "system" fn(usize, *const u32, u32, *const u8, u32, usize, *const Guid, *mut usize) -> i32 =
        unsafe { core::mem::transmute(slot(device, 27)) };
    let mut resource = 0;
    if create(device, props.as_ptr(), 0, desc.as_ptr(), state, 0, &RESOURCE, &mut resource) < 0 || resource == 0 {
        return None;
    }
    let resource = Com(resource);
    let map: extern "system" fn(usize, u32, *const usize, *mut usize) -> i32 =
        unsafe { core::mem::transmute(slot(resource.0, 8)) };
    let mut ptr = 0;
    let read_range = [0, if heap == 3 { 8 } else { 0 }];
    (map(resource.0, 0, read_range.as_ptr(), &mut ptr) >= 0 && ptr != 0).then_some((resource, ptr))
}

pub(crate) unsafe fn record_write(list: *mut c_void, target: u64, kind: Bop64, value: u64) -> bool {
    unsafe { prepare_write(list, target, kind, value, 8) }.is_some()
}

pub(crate) unsafe fn record_write32(list: *mut c_void, target: u64, value: u32) -> bool {
    unsafe { prepare_write(list, target, Bop64::Value, u64::from(value), 4) }.is_some()
}

unsafe fn prepare_write(list: *mut c_void, target: u64, kind: Bop64, value: u64, bytes: usize) -> Option<()> {
    let get: extern "system" fn(*mut c_void, *const Guid, *mut usize) -> i32 =
        unsafe { core::mem::transmute(slot(list as usize, 7)) };
    let mut device = 0;
    if get(list, &DEVICE, &mut device) < 0 || device == 0 {
        return None;
    }
    let device = Com(device);
    let (readback, mapped) = unsafe { buffer(device.0, 3, COPY_DEST) }?;
    let upload = match kind {
        Bop64::Value => {
            let (upload, ptr) = unsafe { buffer(device.0, 2, GENERIC_READ) }?;
            unsafe { (ptr as *mut u64).write(value) };
            let copy: extern "system" fn(*mut c_void, usize, u64, usize, u64, u64) =
                unsafe { core::mem::transmute(crate::cmdlist_wrap::pc_method(list, 15)) };
            copy(list, readback.0, 0, upload.0, 0, bytes as u64);
            Some(upload)
        }
        Bop64::GpuTimestamp => {
            if !unsafe { crate::timestamp::resolve_console_into_readback(list, readback.0) } {
                return None;
            }
            None
        }
        Bop64::SystemTimestamp => {
            if !unsafe { crate::sys_ts::write_to(list, readback.0, 0, COPY_DEST) } {
                return None;
            }
            None
        }
    };
    let write = Arc::new(Write { target: target as usize, bytes, kind, readback, mapped, _upload: upload });
    LISTS.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new)
        .entry(list as usize).or_default().push(write);
    Some(())
}

struct Batch {
    fence: Com,
    writes: Vec<Arc<Write>>,
    published: bool,
    retire_after_wait: bool,
}

impl Batch {
    fn complete(&mut self) -> bool {
        let completed: extern "system" fn(usize) -> u64 =
            unsafe { core::mem::transmute(slot(self.fence.0, 8)) };
        let value = completed(self.fence.0);
        if value == 0 {
            return false;
        }
        if value == u64::MAX {
            record("d3d12_xs", "bop64_cpu", "device removed before CPU writeback");
            return true;
        }
        if self.published {
            return value >= 3;
        }
        for write in &self.writes {
            let value = unsafe {
                if write.bytes == 4 {
                    u64::from((write.mapped as *const u32).read_volatile())
                } else {
                    (write.mapped as *const u64).read_volatile()
                }
            };
            let mut written = 0;
            let ok = crate::title_shadow::is_writable_range(write.target, write.bytes) && unsafe {
                WriteProcessMemory(GetCurrentProcess(), write.target as *mut c_void, &value, write.bytes, &mut written) != 0
            } && written == write.bytes;
            record("d3d12_xs", "bop64_cpu", &format!(
                "after fence dst={:#x} kind={:?} readback={:#x} value={value:#018x} written={ok} bytes={}",
                write.target, write.kind, write.readback.0, write.bytes,
            ));
        }
        let signal: extern "system" fn(usize, u64) -> i32 =
            unsafe { core::mem::transmute(slot(self.fence.0, 10)) };
        if signal(self.fence.0, 2) < 0 {
            record("d3d12_xs", "bop64_cpu", "CPU fence signal failed");
        }
        self.writes.clear();
        self.published = true;
        !self.retire_after_wait
    }
}

fn worker() -> Option<&'static std::sync::mpsc::Sender<Batch>> {
    static WORKER: OnceLock<Option<std::sync::mpsc::Sender<Batch>>> = OnceLock::new();
    WORKER.get_or_init(|| {
        let (send, receive) = std::sync::mpsc::channel::<Batch>();
        std::thread::Builder::new().name("tombox-bop64-readback".into()).spawn(move || {
            let interval = std::time::Duration::from_micros(250);
            let mut pending = Vec::new();
            loop {
                if pending.is_empty() {
                    let Ok(batch) = receive.recv() else { break };
                    pending.push(batch);
                }
                pending.extend(receive.try_iter());
                pending.retain_mut(|batch| !batch.complete());
                if !pending.is_empty() {
                    std::thread::sleep(interval);
                }
            }
        }).ok().map(|_| send)
    }).as_ref()
}

pub(crate) unsafe fn after_submit(queue: *mut c_void, lists: &[*mut c_void], signal: usize, wait: usize) {
    let writes: Vec<_> = {
        let state = LISTS.lock().unwrap_or_else(|e| e.into_inner());
        let Some(state) = state.as_ref() else { return };
        lists.iter().filter_map(|list| state.get(&(*list as usize))).flatten().cloned().collect()
    };
    if writes.is_empty() {
        return;
    }
    let Some(worker) = worker().filter(|_| signal != 0 && wait != 0) else {
        record("d3d12_xs", "bop64_cpu", "writeback worker unavailable");
        std::mem::forget(writes);
        return;
    };
    let fence = unsafe { crate::gpu_wait::new_fence(queue) };
    if fence == 0 {
        record("d3d12_xs", "bop64_cpu", "writeback fence unavailable");
        std::mem::forget(writes);
        return;
    }
    let queue_signal: extern "system" fn(*mut c_void, usize, u64) -> i32 = unsafe { core::mem::transmute(signal) };
    let queue_wait: extern "system" fn(*mut c_void, usize, u64) -> i32 = unsafe { core::mem::transmute(wait) };
    let mut batch = Batch { fence: Com(fence), writes, published: false, retire_after_wait: false };
    if queue_signal(queue, fence, 1) < 0 {
        record("d3d12_xs", "bop64_cpu", "queue signal failed");
        std::mem::forget(batch);
        return;
    }
    // Keep subsequent submissions and their visible fences behind CPU publication.
    if queue_wait(queue, fence, 2) < 0 {
        record("d3d12_xs", "bop64_cpu", "queue wait failed");
    } else {
        batch.retire_after_wait = queue_signal(queue, fence, 3) >= 0;
    }
    if let Err(error) = worker.send(batch) {
        record("d3d12_xs", "bop64_cpu", "writeback worker stopped");
        // Release the queue and retain resources whose execution status is unknown.
        let signal: extern "system" fn(usize, u64) -> i32 =
            unsafe { core::mem::transmute(slot(fence, 10)) };
        signal(fence, 2);
        std::mem::forget(error.0);
    }
}
