use super::{indirect::RoutedPhases, slot, Bind, Guid, Phase};
use crate::cl2_state::{self, ComputeBindings, ComputeBuffer};
use crate::gws_scratch::{self, Resource, Scratch};
use core::ffi::c_void;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock, Mutex, OnceLock};

const MAX_CONTINUATIONS: u8 = 4;
const MAX_PENDING: usize = 4;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    Semaphore,
    Barrier,
    OrderedSemaphore,
    BarrierAndRelease,
}

impl Kind {
    pub(super) fn from_mode(mode: u32) -> Option<Self> {
        match mode {
            1 => Some(Self::Semaphore),
            2 => Some(Self::Barrier),
            3 => Some(Self::OrderedSemaphore),
            4 => Some(Self::BarrierAndRelease),
            _ => None,
        }
    }

    pub(super) fn is_barrier(self) -> bool {
        matches!(self, Self::Barrier | Self::BarrierAndRelease)
    }
}

#[derive(Clone)]
enum Replay {
    Direct(Phase),
    Indirect(RoutedPhases),
}

#[derive(Clone)]
struct Pending {
    replay: Replay,
    kind: Kind,
    remaining: u8,
}

#[derive(Clone)]
struct Checkpoint {
    bindings: ComputeBindings,
    pending: Vec<Pending>,
}

static WAITERS: LazyLock<Mutex<HashMap<usize, Checkpoint>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

static SIGNALS: LazyLock<Mutex<HashSet<usize>>> = LazyLock::new(|| Mutex::new(HashSet::new()));

pub(crate) fn forget_signals(list: *mut c_void) {
    if enabled() {
        SIGNALS.lock().unwrap().remove(&(list as usize));
    }
}

pub(crate) fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        super::phased_enabled() && std::env::var("TOMBOX_GWS_RESUME").as_deref() == Ok("1")
    })
}

pub(crate) fn forget(list: *mut c_void) {
    if enabled() {
        WAITERS.lock().unwrap().remove(&(list as usize));
    }
}

pub(super) unsafe fn remember(list: *mut c_void, phase: Phase, kind: Kind, bind: Bind) {
    unsafe { remember_replay(list, Replay::Direct(phase), kind, bind) };
}

pub(super) unsafe fn remember_indirect(
    list: *mut c_void,
    phases: RoutedPhases,
    kind: Kind,
    bind: Bind,
) {
    unsafe { remember_replay(list, Replay::Indirect(phases), kind, bind) };
}

unsafe fn remember_replay(list: *mut c_void, replay: Replay, kind: Kind, bind: Bind) {
    if !enabled() {
        return;
    }
    if kind == Kind::BarrierAndRelease {
        SIGNALS.lock().unwrap().insert(list as usize);
    }
    let previous = WAITERS.lock().unwrap().remove(&(list as usize));
    let Some(bindings) = cl2_state::compute_bindings(list) else {
        return;
    };
    let mut pending = previous
        .filter(|old| old.bindings == bindings)
        .map_or_else(Vec::new, |old| old.pending);
    pending.retain_mut(|next| match kind {
        Kind::Semaphore => false,
        Kind::OrderedSemaphore => next.kind == Kind::OrderedSemaphore,
        Kind::Barrier => next.kind.is_barrier() && unsafe { advance(next, list, bind) },
        Kind::BarrierAndRelease => unsafe { advance(next, list, bind) },
    });
    if pending.len() < MAX_PENDING {
        pending.push(Pending {
            replay,
            kind,
            remaining: MAX_CONTINUATIONS,
        });
    }
    WAITERS.lock().unwrap().insert(
        list as usize,
        Checkpoint { bindings, pending },
    );
}

pub(super) unsafe fn before_ordered_wait(list: *mut c_void, bind: Bind) {
    if !enabled() {
        return;
    }
    let Some(checkpoint) = WAITERS.lock().unwrap().remove(&(list as usize)) else {
        return;
    };
    if cl2_state::compute_bindings(list).as_ref() != Some(&checkpoint.bindings) {
        return;
    }
    let mut checkpoint = checkpoint;
    checkpoint.pending.retain(|next| next.kind == Kind::OrderedSemaphore);
    for next in &checkpoint.pending {
        unsafe { replay(next, list, bind) };
    }
    if !checkpoint.pending.is_empty() {
        WAITERS.lock().unwrap().insert(list as usize, checkpoint);
    }
}

pub(super) unsafe fn between_release_rows(list: *mut c_void, bind: Bind) {
    let Some(checkpoint) = WAITERS.lock().unwrap().remove(&(list as usize)) else {
        return;
    };
    if cl2_state::compute_bindings(list).as_ref() != Some(&checkpoint.bindings) {
        return;
    }
    // Charge the batch's continuation budget once, in remember_indirect.
    for next in &checkpoint.pending {
        unsafe { replay(next, list, bind) };
    }
    WAITERS.lock().unwrap().insert(list as usize, checkpoint);
}

unsafe fn replay(next: &Pending, list: *mut c_void, bind: Bind) {
    unsafe {
        match &next.replay {
            Replay::Direct(phase) => phase.run(list, bind),
            Replay::Indirect(phases) => phases.resume(list, bind),
        }
    }
}

unsafe fn advance(next: &mut Pending, list: *mut c_void, bind: Bind) -> bool {
    unsafe { replay(next, list, bind) };
    next.remaining -= 1;
    next.remaining != 0
}

pub(super) unsafe fn after_dispatch(list: *mut c_void, bound: *mut c_void, bind: Bind) {
    if !enabled() {
        return;
    }
    if unsafe { super::can_signal(bound) } && cl2_state::predication_of(list).is_none() {
        SIGNALS.lock().unwrap().insert(list as usize);
    }
    let Some(mut checkpoint) = WAITERS.lock().unwrap().remove(&(list as usize)) else {
        return;
    };
    // A compute shader without GWS instructions cannot unblock a checkpoint.
    if unsafe { super::cannot_signal(bound) } {
        WAITERS.lock().unwrap().insert(list as usize, checkpoint);
        return;
    }
    if cl2_state::predication_of(list).is_some()
        || cl2_state::compute_bindings(list).as_ref() != Some(&checkpoint.bindings)
    {
        return;
    }
    if checkpoint.pending.iter().any(|next| !next.kind.is_barrier()) {
        checkpoint.pending.retain_mut(|next| {
            next.kind.is_barrier() || unsafe { advance(next, list, bind) }
        });
        unsafe { bind(list, bound) };
    }
    if !checkpoint.pending.is_empty() {
        WAITERS.lock().unwrap().insert(list as usize, checkpoint);
    }
}

const MAX_SAVED_LISTS: usize = 16;

struct Source {
    checkpoint: Checkpoint,
    scratch: Vec<Scratch>,
    root: usize,
    buffers: Vec<ComputeBuffer>,
    bindings: Scratch,
    table: Option<super::tables::Table>,
    list_type: u32,
    remaining: u8,
}

#[derive(Default)]
struct Timeline {
    device: usize,
    sources: Vec<Source>,
    export: Option<Export>,
}

struct Export {
    id: u64,
    fence: Scratch,
}

type QueueState = Arc<Mutex<Timeline>>;
type FenceOp = unsafe extern "system" fn(*mut c_void, *mut c_void, u64) -> i32;
static QUEUE_CALLS: Mutex<()> = Mutex::new(());
static EXPORT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
const EXPORT_KEY: Guid = Guid {
    d1: 0xcf190524, d2: 0x78b6, d3: 0x4814,
    d4: [0xb6, 0x73, 0xce, 0x62, 0x19, 0x5a, 0x9a, 0x07],
};
const IID_DEVICE: Guid = Guid {
    d1: 0x189819f1, d2: 0x1db6, d3: 0x4b57,
    d4: [0xbe, 0x54, 0x18, 0x21, 0x33, 0x9b, 0x85, 0xf7],
};
static QUEUES: LazyLock<Mutex<HashMap<usize, QueueState>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn retired_queues() -> Option<&'static std::sync::mpsc::Sender<QueueState>> {
    static RETIRE: OnceLock<Option<std::sync::mpsc::Sender<QueueState>>> = OnceLock::new();
    RETIRE.get_or_init(|| {
        let (send, receive) = std::sync::mpsc::channel::<QueueState>();
        std::thread::Builder::new().name("tombox-gws-queue-reaper".into())
            .spawn(move || for state in receive { drop(state); })
            .ok()?;
        Some(send)
    }).as_ref()
}

unsafe extern "system" fn queue_destroyed(queue: *mut c_void) {
    let state = QUEUES.lock().unwrap().remove(&(queue as usize));
    if let Some(state) = state {
        // Avoid calling D3D12 Release from inside its destruction callback.
        let _ = retired_queues().unwrap().send(state);
    }
}

unsafe fn queue_state(queue: *mut c_void) -> Option<QueueState> {
    let mut queues = QUEUES.lock().unwrap();
    if let Some(state) = queues.get(&(queue as usize)) {
        return Some(Arc::clone(state));
    }
    let device = unsafe { queue_device(queue) }?;
    retired_queues()?;
    if !unsafe { crate::title_gpu::register_destruction(queue, queue_destroyed, queue) } {
        return None;
    }
    let state = Arc::new(Mutex::new(Timeline { device: device.0, ..Timeline::default() }));
    queues.insert(queue as usize, Arc::clone(&state));
    Some(state)
}

fn existing_queue(queue: usize) -> Option<QueueState> {
    QUEUES.lock().unwrap().get(&queue).cloned()
}

unsafe fn queue_device(queue: *mut c_void) -> Option<Resource> {
    let get: unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> i32 =
        unsafe { core::mem::transmute(slot(queue, 7)) };
    let mut dev = core::ptr::null_mut();
    if unsafe { get(queue, &IID_DEVICE, &mut dev) } < 0 || dev.is_null() {
        return None;
    }
    Some(Resource(dev as usize))
}

pub(crate) unsafe fn queue_signal(
    queue: *mut c_void, fence: *mut c_void, value: u64, forward: FenceOp,
) -> i32 {
    if !enabled() || fence.is_null() {
        return unsafe { forward(queue, fence, value) };
    }
    let _submission = QUEUE_CALLS.lock().unwrap();
    let hr = unsafe { forward(queue, fence, value) };
    if hr < 0 {
        return hr;
    }
    let mut stamp = [0, 0, value];
    if let Some(state) = existing_queue(queue as usize) {
        let mut timeline = state.lock().unwrap();
        if !timeline.sources.is_empty() {
            let private = unsafe { crate::gpu_wait::new_fence(queue) };
            if private != 0 {
                let private = Arc::new(vec![Resource(private)]);
                let signal: FenceOp = unsafe { core::mem::transmute(crate::present::host_queue_signal(queue)) };
                if unsafe { signal(queue, private[0].0 as *mut c_void, 1) } >= 0 {
                    let id = EXPORT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    stamp = [queue as u64, id, value];
                    unsafe { gws_scratch::retain_submission(queue, vec![Arc::clone(&private)]) };
                    timeline.export = Some(Export { id, fence: private });
                }
            }
        }
    }
    let set: unsafe extern "system" fn(*mut c_void, *const Guid, u32, *const c_void) -> i32 =
        unsafe { core::mem::transmute(slot(fence, 4)) };
    unsafe { set(fence, &EXPORT_KEY, 24, stamp.as_ptr().cast()) };
    hr
}

pub(crate) unsafe fn queue_wait(
    queue: *mut c_void, fence: *mut c_void, value: u64, forward: FenceOp,
) -> i32 {
    if !enabled() || fence.is_null() {
        return unsafe { forward(queue, fence, value) };
    }
    let _submission = QUEUE_CALLS.lock().unwrap();
    let hr = unsafe { forward(queue, fence, value) };
    if hr >= 0 {
        unsafe { transfer_waiters(queue, fence, value, forward) };
    }
    hr
}

unsafe fn transfer_waiters(queue: *mut c_void, fence: *mut c_void, value: u64, wait: FenceOp) {
    let get: unsafe extern "system" fn(*mut c_void, *const Guid, *mut u32, *mut c_void) -> i32 =
        unsafe { core::mem::transmute(slot(fence, 3)) };
    let mut stamp = [0u64; 3];
    let mut size = 24;
    if unsafe { get(fence, &EXPORT_KEY, &mut size, stamp.as_mut_ptr().cast()) } < 0 || size != 24 {
        return;
    }
    let [source, id, signaled] = stamp;
    if source == 0 || source == queue as u64 || value != signaled {
        return;
    }
    let Some(source_state) = existing_queue(source as usize) else { return };
    // The stamp can outlive its queue. Dereference only the destination and
    // compare against the device identity recorded by the source timeline.
    let Some(device) = (unsafe { queue_device(queue) }) else { return };
    let desc = unsafe { crate::ecl_probe::queue_desc(queue, slot(queue, 18)) };
    let mut source = source_state.lock().unwrap();
    let Some(export) = source.export.as_ref().filter(|export| export.id == id) else { return };
    if source.device != device.0 || source.sources.is_empty()
        || source.sources.iter().any(|saved| saved.list_type != desc[0])
    {
        return;
    }
    let Some(destination) = (unsafe { queue_state(queue) }) else { return };
    let mut destination = destination.lock().unwrap();
    if source.sources.len() + destination.sources.len() > MAX_SAVED_LISTS {
        return;
    }
    let private = Arc::clone(&export.fence);
    // A CPU signal on the title fence can satisfy the public wait early.
    // Order the replay behind a private signal that only the source queue can complete.
    if unsafe { wait(queue, private[0].0 as *mut c_void, 1) } < 0 {
        return;
    }
    unsafe { gws_scratch::retain_submission(queue, vec![private]) };
    destination.sources.append(&mut source.sources);
    destination.export = None;
    source.export = None;
}

unsafe fn snapshot(list: *mut c_void, checkpoint: Checkpoint) -> Option<Source> {
    let root = checkpoint.bindings.replayable_root()?;
    let mut buffers = checkpoint.bindings.replayable_buffers()?;
    let table = match checkpoint.bindings.descriptor_heap() {
        Some(heap) => Some(unsafe { super::tables::snapshot(list, root, heap, &checkpoint.bindings, &mut buffers) }?),
        None if checkpoint.bindings.table_handles().next().is_some() => return None,
        None => None,
    };
    let scratch = gws_scratch::recording(list, |resource| {
        checkpoint.pending.iter().any(|pending| match &pending.replay {
            Replay::Direct(phase) => phase.scratch == resource,
            Replay::Indirect(phases) => phases.owns_scratch(resource),
        })
    })?;
    let list_type = unsafe { crate::fill::list_type(list) };
    let mut bindings: Vec<_> = std::iter::once(root).chain(buffers.iter().map(ComputeBuffer::resource))
        .map(|object| {
            let add_ref: unsafe extern "system" fn(*mut c_void) -> u32 =
                unsafe { core::mem::transmute(super::slot(object, 1)) };
            unsafe { add_ref(object) };
            Resource(object as usize)
        }).collect();
    let table = table.map(|(table, heap)| {
        bindings.push(heap);
        table
    });
    Some(Source {
        checkpoint, scratch, root: root as usize, buffers, bindings: Arc::new(bindings), table,
        list_type, remaining: MAX_CONTINUATIONS,
    })
}

pub(crate) unsafe fn submit_lists(
    queue: *mut c_void,
    lists: &[*mut c_void],
    exec: crate::gpu_wait::ExecFn,
    signal: usize,
    wait: usize,
) {
    if !enabled() || lists.is_empty() {
        unsafe { crate::gpu_wait::submit(queue, lists, exec, signal, wait) };
        return;
    }
    let _submission = QUEUE_CALLS.lock().unwrap();
    let queue_has_work = lists.iter().any(|&list| is_gws_recording(list));
    let existing = QUEUES.lock().unwrap().get(&(queue as usize)).cloned();
    let queue_state = if existing.is_some() || !queue_has_work {
        existing
    } else {
        unsafe { queue_state(queue) }
    };
    let mut saved = queue_state.as_ref().map(|state| state.lock().unwrap());
    let mut sources = saved.as_mut().map_or_else(Vec::new, |timeline| {
        timeline.export = None;
        std::mem::take(&mut timeline.sources)
    });
    let mut next = 0;
    for (i, &list) in lists.iter().enumerate() {
        if !cl2_state::title_closed(list) {
            continue;
        }
        let signals = SIGNALS.lock().unwrap().contains(&(list as usize));
        let checkpoint = WAITERS.lock().unwrap().get(&(list as usize)).cloned();
        let arrives = checkpoint.as_ref().is_some_and(|entry| {
            entry.pending.iter().any(|p| p.kind.is_barrier())
        });
        let current = checkpoint.and_then(|checkpoint| unsafe { snapshot(list, checkpoint) });
        if let Some(source) = &current {
            // Submitting the same recording reinitializes its scratch on the GPU.
            sources.retain(|older| !older.scratch.iter().any(|old| {
                source.scratch.iter().any(|current| Arc::ptr_eq(old, current))
            }));
        }
        if !sources.is_empty() && (signals || arrives) {
            let tails: Vec<_> = sources.iter_mut().filter_map(|older| {
                let tail = unsafe { continuation(older, list, signals, arrives) }?;
                older.remaining -= 1;
                Some(tail)
            }).collect();
            sources.retain(|source| source.remaining != 0);
            if !tails.is_empty() {
                if next < i {
                    unsafe { crate::gpu_wait::submit(queue, &lists[next..i], exec, signal, wait) };
                }
                unsafe { crate::gpu_wait::submit(queue, &[list], exec, signal, wait) };
                for tail in &tails {
                    unsafe { exec(queue, 1, tail) };
                }
                unsafe { gws_scratch::submitted(queue, &tails) };
                for tail in tails {
                    gws_scratch::forget(tail);
                }
                next = i + 1;
            }
        }
        if let Some(source) = current {
            sources.push(source);
        }
    }
    if next < lists.len() {
        unsafe { crate::gpu_wait::submit(queue, &lists[next..], exec, signal, wait) };
    }
    if let Some(saved) = &mut saved {
        // Without a GPU completion status, keep the newest recordings in the
        // bounded experimental queue cache.
        let excess = sources.len().saturating_sub(MAX_SAVED_LISTS);
        sources.drain(..excess);
        saved.sources = sources;
    }
}

fn is_gws_recording(list: *mut c_void) -> bool {
    SIGNALS.lock().unwrap().contains(&(list as usize))
        || WAITERS.lock().unwrap().contains_key(&(list as usize))
}

unsafe fn continuation(
    source: &Source,
    producer: *mut c_void,
    signals: bool,
    arrives: bool,
) -> Option<*mut c_void> {
    let ty = unsafe { crate::fill::list_type(producer) };
    if !matches!(ty, 0 | 2) || source.list_type != ty || !source.buffers.iter().all(ComputeBuffer::is_replayable) {
        return None;
    }
    let root = source.root as *mut c_void;
    let checkpoint = &source.checkpoint;
    let relevant = |pending: &&Pending| {
        if pending.kind.is_barrier() { arrives } else { signals }
    };
    if !checkpoint.pending.iter().any(|pending| relevant(&pending)) {
        return None;
    }
    let (tail, allocator) = unsafe { crate::list_split::new_part(producer, ty) }?;
    let resources = vec![Resource(tail as usize), Resource(allocator as usize)];
    let root_fn: Bind = unsafe { core::mem::transmute(super::slot(tail, 29)) };
    let bind: Bind = unsafe { core::mem::transmute(super::slot(tail, 25)) };
    let close: unsafe extern "system" fn(*mut c_void) -> i32 =
        unsafe { core::mem::transmute(super::slot(tail, 9)) };
    unsafe {
        root_fn(tail, root);
        crate::gds::bind_compute(tail, root);
        checkpoint.bindings.apply_compute_args(tail);
        if let Some(table) = &source.table {
            table.bind(tail);
        }
        for pending in checkpoint.pending.iter().filter(relevant) {
            replay(pending, tail, bind);
        }
        crate::gds::on_close(tail);
        if close(tail) < 0 {
            return None;
        }
    }
    gws_scratch::retain_shared(tail, &source.scratch);
    gws_scratch::retain_shared(tail, std::slice::from_ref(&source.bindings));
    gws_scratch::retain(tail, resources);
    Some(tail)
}
