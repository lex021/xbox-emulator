//! A recording owns its scratch for resubmission. Each submission holds a second
//! reference until its fence completes, independently of command-list Reset.
use compat_common::record;
use core::ffi::c_void;
use std::collections::HashMap;
use std::sync::{Arc, Condvar, LazyLock, Mutex, OnceLock};

pub(crate) const COPY_DEST: u32 = 0x400;
pub(crate) const UAV: u32 = 8;
const GENERIC_READ: u32 = 0xac3;

pub(crate) struct Resource(pub(crate) usize);

impl Drop for Resource {
    fn drop(&mut self) {
        let object = self.0 as *mut c_void;
        let release: unsafe extern "system" fn(*mut c_void) -> u32 =
            unsafe { core::mem::transmute(slot(object, 2)) };
        unsafe { release(object) };
    }
}

pub(crate) type Scratch = Arc<Vec<Resource>>;

struct Submission {
    fence: Option<Resource>,
    _scratch: Vec<Scratch>,
}

static RECORDINGS: LazyLock<Mutex<HashMap<usize, Vec<Scratch>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static SUBMISSIONS: Mutex<Vec<Submission>> = Mutex::new(Vec::new());
static WAKE: Condvar = Condvar::new();

fn wake_reaper() {
    static STARTED: OnceLock<()> = OnceLock::new();
    STARTED.get_or_init(|| {
        if let Err(e) = std::thread::Builder::new()
            .name("tombox-gws-reaper".into())
            .spawn(|| loop {
                let pending = SUBMISSIONS.lock().unwrap();
                let pending = WAKE
                    .wait_while(pending, |entries| entries.iter().all(|s| s.fence.is_none()))
                    .unwrap();
                drop(pending);
                collect();
                std::thread::sleep(std::time::Duration::from_millis(2));
            })
        {
            record(
                "d3d12_xs",
                "gws_scratch",
                &format!("could not start completion worker: {e}"),
            );
        }
    });
    WAKE.notify_one();
}

unsafe fn slot(object: *mut c_void, n: usize) -> usize {
    unsafe { *(*object.cast::<*const usize>()).add(n) }
}

fn collect() {
    let mut pending = SUBMISSIONS.lock().unwrap();
    let mut i = 0;
    let mut complete = Vec::new();
    while i < pending.len() {
        if pending[i]
            .fence
            .as_ref()
            .is_some_and(|f| unsafe { crate::cl2_state::fence_passed(f.0, 1) })
        {
            complete.push(pending.swap_remove(i));
        } else {
            i += 1;
        }
    }
    drop(pending);
    drop(complete);
}

pub(crate) fn forget(list: *mut c_void) {
    if crate::gws_dispatch::phased_enabled() {
        crate::gws_dispatch::forget_checkpoint(list);
        crate::gws_dispatch::forget_signals(list);
    } else if !crate::global_bind::enabled() {
        return;
    }
    let scratch = RECORDINGS.lock().unwrap().remove(&(list as usize));
    if scratch.is_some() {
        collect();
    }
    drop(scratch);
}

pub(crate) unsafe fn submitted(queue: *mut c_void, lists: &[*mut c_void]) {
    if !crate::gws_dispatch::phased_enabled() && !crate::global_bind::enabled() {
        return;
    }
    let recordings = RECORDINGS.lock().unwrap();
    let scratch: Vec<_> = lists
        .iter()
        .filter_map(|&l| recordings.get(&(l as usize)))
        .flatten()
        .cloned()
        .collect();
    drop(recordings);
    unsafe { retain_submission(queue, scratch) };
}

pub(crate) unsafe fn retain_submission(queue: *mut c_void, scratch: Vec<Scratch>) {
    if scratch.is_empty() {
        return;
    }
    let fence = unsafe { crate::gpu_wait::new_fence(queue) };
    let fence = if fence == 0 {
        None
    } else {
        let fence = Resource(fence);
        let signal: unsafe extern "system" fn(*mut c_void, *mut c_void, u64) -> i32 =
            unsafe { core::mem::transmute(crate::present::host_queue_signal(queue)) };
        (unsafe { signal(queue, fence.0 as *mut c_void, 1) } >= 0).then_some(fence)
    };
    if fence.is_none() {
        record(
            "d3d12_xs",
            "gws_scratch",
            "GPU completion fence unavailable; retaining submitted resources",
        );
    }
    SUBMISSIONS.lock().unwrap().push(Submission {
        fence,
        _scratch: scratch,
    });
    collect();
    wake_reaper();
}

#[cfg(test)]
pub(crate) fn recording_probe(list: *mut c_void) -> impl Fn() -> usize {
    let weak: Vec<_> = RECORDINGS.lock().unwrap()[&(list as usize)].iter().map(Arc::downgrade).collect();
    move || weak.iter().filter(|item| item.strong_count() != 0).count()
}

pub(crate) unsafe fn begin(
    list: *mut c_void,
    pso: *mut c_void,
    bytes: u32,
) -> Option<(usize, u64)> {
    collect();
    let bytes = u64::from(bytes);
    let (gpu, va) = unsafe { crate::gds::create_buffer(list, bytes, 1, 4, COPY_DEST) }?;
    let gpu = Resource(gpu);
    let (upload, _) = unsafe { crate::gds::create_buffer(list, bytes, 2, 0, GENERIC_READ) }?;
    let upload = Resource(upload);
    let src = upload.0 as *mut c_void;
    let map: unsafe extern "system" fn(*mut c_void, u32, *const usize, *mut *mut u8) -> i32 =
        unsafe { core::mem::transmute(slot(src, 8)) };
    let mut data = core::ptr::null_mut();
    if unsafe { map(src, 0, [0usize; 2].as_ptr(), &mut data) } < 0 || data.is_null() {
        return None;
    }
    let unmap: unsafe extern "system" fn(*mut c_void, u32, *const usize) =
        unsafe { core::mem::transmute(slot(src, 9)) };
    unsafe { data.write_bytes(0, bytes as usize) };
    unsafe { unmap(src, 0, core::ptr::null()) };

    let dst = gpu.0;
    let add_ref: unsafe extern "system" fn(*mut c_void) -> u32 =
        unsafe { core::mem::transmute(slot(pso, 1)) };
    unsafe { add_ref(pso) };
    retain(list, vec![gpu, upload, Resource(pso as usize)]);
    let copy: unsafe extern "system" fn(*mut c_void, usize, u64, *mut c_void, u64, u64) =
        unsafe { core::mem::transmute(slot(list, 15)) };
    unsafe { copy(list, dst, 0, src, 0, bytes) };
    Some((dst, va))
}

pub(crate) fn retain(list: *mut c_void, resources: Vec<Resource>) {
    RECORDINGS.lock().unwrap().entry(list as usize).or_default().push(Arc::new(resources));
}

pub(crate) fn recording(source: *mut c_void, needed: impl Fn(usize) -> bool) -> Option<Vec<Scratch>> {
    let recordings = RECORDINGS.lock().unwrap();
    let retained: Vec<_> = recordings.get(&(source as usize))?.iter()
        .filter(|allocation| allocation.iter().any(|resource| needed(resource.0)))
        .cloned()
        .collect();
    (!retained.is_empty()).then_some(retained)
}

pub(crate) fn retain_shared(list: *mut c_void, resources: &[Scratch]) {
    RECORDINGS.lock().unwrap().entry(list as usize).or_default().extend_from_slice(resources);
}

pub(crate) unsafe fn end(list: *mut c_void, resource: usize) {
    // The same closed recording can be submitted again. Begin with a fresh
    // checkpoint and the same state on every execution.
    unsafe { crate::fill::barrier_pub(list, resource, UAV, COPY_DEST) };
}
