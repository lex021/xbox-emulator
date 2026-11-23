use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicI64, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use crate::gcdcom::Guid;
use crate::wasapi;

pub const ENVELOPE: usize = 0x810;
pub const PAYLOAD: usize = 0x10;
pub const CREATE_OBJECT: u32 = 0x01;
pub const GET_MIX_FORMAT: u32 = 0x02;
pub const GET_DEVICE_PERIOD: u32 = 0x03;
pub const INITIALIZE: u32 = 0x04;
pub const IS_FORMAT_SUPPORTED: u32 = 0x05;
pub const GET_STREAM_LATENCY: u32 = 0x06;
pub const START: u32 = 0x07;
pub const STOP: u32 = 0x08;
pub const RESET: u32 = 0x09;
pub const GET_BUFFER_SIZE_LIMITS: u32 = 0x0a;
pub const IS_OFFLOAD_CAPABLE: u32 = 0x0b;
pub const SET_CLIENT_PROPERTIES: u32 = 0x0c;
pub const GET_SHARED_MODE_ENGINE_PERIOD: u32 = 0x0d;
pub const GET_CURRENT_SHARED_MODE_ENGINE_PERIOD: u32 = 0x0e;
pub const INITIALIZE_SHARED_AUDIO_STREAM: u32 = 0x0f;
pub const STREAM_GET_CHANNEL_COUNT: u32 = 0x10;
pub const STREAM_SET_CHANNEL_VOLUME: u32 = 0x11;
pub const STREAM_GET_CHANNEL_VOLUME: u32 = 0x12;
pub const STREAM_SET_ALL_VOLUMES: u32 = 0x13;
pub const STREAM_GET_ALL_VOLUMES: u32 = 0x14;
pub const GET_SIMPLE_VOLUME: u32 = 0x15;
pub const SET_SIMPLE_VOLUME: u32 = 0x16;
pub const GET_SIMPLE_MUTE: u32 = 0x17;
pub const SET_SIMPLE_MUTE: u32 = 0x18;
pub const SESSION_GET_STATE: u32 = 0x19;
pub const SESSION_GET_DISPLAY_NAME: u32 = 0x1a;
pub const SESSION_SET_DISPLAY_NAME: u32 = 0x1b;
pub const SESSION_GET_GROUPING: u32 = 0x1c;
pub const SESSION_SET_GROUPING: u32 = 0x1d;
pub const SESSION_REGISTER_NOTIFICATION: u32 = 0x1e;
pub const ENUMERATOR_SUBSCRIBE: u32 = 0x23;
pub const ENUMERATOR_UNSUBSCRIBE: u32 = 0x24;
pub const DEVICE_GET_ID: u32 = 0x27;
pub const DEVICE_GET_STATE: u32 = 0x28;
pub const ENDPOINT_HOST_STREAM_MODES: u32 = 0x29;
pub const ENDPOINT_OFFLOAD_SERVICE: u32 = 0x2a;
pub const PROPERTY_GET_COUNT: u32 = 0x2b;
pub const PROPERTY_GET_AT: u32 = 0x2c;
pub const PROPERTY_GET_VALUE: u32 = 0x2d;
pub const RELEASE_OBJECT: u32 = 0x2e;
pub const SPATIAL_STATIC_POSITION: u32 = 0x30;
pub const SPATIAL_NATIVE_MASK: u32 = 0x31;
pub const SPATIAL_MAX_DYNAMIC: u32 = 0x32;
pub const SPATIAL_MAX_FRAMES: u32 = 0x33;
pub const SPATIAL_FORMAT_SUPPORTED: u32 = 0x34;
pub const SPATIAL_STREAM_AVAILABLE: u32 = 0x35;
pub const FORMAT_ENUM_COUNT: u32 = 0x3a;
pub const FORMAT_ENUM_GET: u32 = 0x3b;
pub const CLOCK_SET_SAMPLE_RATE: u32 = 0x47;
pub const ITEM_GET: u32 = 0x3c;
pub const ITEM_COUNT: u32 = 0x3d;
pub const ITEM_FLAG: u32 = 0x3e;
pub const ITEM_VALUE: u32 = 0x3f;
pub const ITEM_SERVICE_TYPE: u32 = 0xb;
pub const ENDPOINT_REGISTER: u32 = 0x4f;
pub const ENDPOINT_UNREGISTER: u32 = 0x50;
pub const ENDPOINT_CHANNEL_COUNT: u32 = 0x51;
pub const SET_ENDPOINT_VOLUME_LEVEL: u32 = 0x52;
pub const SET_ENDPOINT_VOLUME_SCALAR: u32 = 0x53;
pub const GET_ENDPOINT_VOLUME_LEVEL: u32 = 0x54;
pub const GET_ENDPOINT_VOLUME_SCALAR: u32 = 0x55;
pub const SET_ENDPOINT_CHANNEL_LEVEL: u32 = 0x56;
pub const SET_ENDPOINT_CHANNEL_SCALAR: u32 = 0x57;
pub const GET_ENDPOINT_CHANNEL_LEVEL: u32 = 0x58;
pub const GET_ENDPOINT_CHANNEL_SCALAR: u32 = 0x59;
pub const SET_ENDPOINT_MUTE: u32 = 0x5a;
pub const GET_ENDPOINT_MUTE: u32 = 0x5b;
pub const ENDPOINT_STEP_INFO: u32 = 0x5c;
pub const ENDPOINT_STEP_UP: u32 = 0x5d;
pub const ENDPOINT_STEP_DOWN: u32 = 0x5e;
pub const ENDPOINT_HARDWARE_SUPPORT: u32 = 0x5f;
pub const ENDPOINT_VOLUME_RANGE: u32 = 0x60;
pub const ENDPOINT_LIST: u32 = 0x70;
pub const DEFAULT_ENDPOINT_TABLE: u32 = 0x71;
pub const SESSION_CHANNEL_COUNT: u32 = 0x72;
pub const SESSION_SET_CHANNEL_VOLUME: u32 = 0x73;
pub const SESSION_GET_CHANNEL_VOLUME: u32 = 0x74;
pub const SESSION_SET_ALL_VOLUMES: u32 = 0x75;
pub const SESSION_GET_ALL_VOLUMES: u32 = 0x76;
pub const SESSION_MANAGER_REGISTER: u32 = 0x79;
pub const SESSION_MANAGER_UNREGISTER: u32 = 0x7a;
pub const SESSION_GET_IDENTIFIER: u32 = 0x7c;
pub const SESSION_GET_INSTANCE_IDENTIFIER: u32 = 0x7d;
pub const SESSION_IS_SYSTEM_SOUNDS: u32 = 0x7e;
pub const SESSION_SET_DUCKING: u32 = 0x7f;
pub const SPATIAL_OFFLOAD_CAPABLE: u32 = 0x85;
pub const SPATIAL_MAX_FRAMES_FOR_CATEGORY: u32 = 0x86;

pub const METHOD_DEFAULT_ENDPOINT_19041: u32 = 0x22;
pub const METHOD_ENDPOINT_19041: u32 = 0x25;
pub const METHOD_DEFAULT_ENDPOINT: u32 = 0x80;
pub const METHOD_ENDPOINT: u32 = 0x81;
pub const METHOD_SESSION_ENUMERATOR: u32 = 0x77;
pub const METHOD_ENUMERATED_SESSION: u32 = 0x78;
pub const METHOD_MEDIA_MANAGER: u32 = 0x61;
pub const METHOD_EXISTING_SESSION: u32 = 0x7b;
pub const MEDIA_MANAGER_TYPE: u32 = 0x13;
pub const EXISTING_SESSION_TYPE: u32 = 0x19;
pub const EXISTING_SESSION_CHARS: usize = 0x200;
pub const MEDIA_PLAY_TO_STREAM_STATE: u32 = 0x64;
pub const MEDIA_CASTING_APP_STATE: u32 = 0x65;
pub const MEDIA_VOIP_CALL_STATE: u32 = 0x66;
pub const METHOD_ARGS: usize = 0x14;
pub const SPATIAL_STREAM_TYPE: u32 = 0xf;
pub const SPATIAL_HRTF_STREAM_TYPE: u32 = 0x10;
pub const SPATIAL_METADATA_STREAM_TYPE: u32 = 0x16;
pub const METHOD_SPATIAL_STREAM: u32 = 0x36;
pub const METHOD_SPATIAL_HRTF_STREAM: u32 = 0x37;
pub const METHOD_SPATIAL_METADATA_STREAM: u32 = 0x38;
pub const METHOD_SPATIAL_STREAM2: u32 = 0x82;
pub const METHOD_SPATIAL_HRTF_STREAM2: u32 = 0x83;
pub const METHOD_SPATIAL_METADATA_STREAM2: u32 = 0x84;
pub const SPATIAL_ARG_MASK: usize = 0x14;
pub const SPATIAL_ARG_MIN: usize = 0x18;
pub const SPATIAL_ARG_MAX: usize = 0x1c;
pub const SPATIAL_ARG_CATEGORY: usize = 0x20;
pub const SPATIAL_ARG_FORMAT: usize = 0x24;
pub const SPATIAL_FORMAT_MAX: usize = 0x48;
pub const SPATIAL_ARG_OPTIONS: usize = 0x70;
pub const SPATIAL_PROPERTY_BYTES: u32 = 0x20;
pub const CLSID_SPATIAL_CLIENT_OUTPUT_ENDPOINT: Guid = Guid { data1: 0x9371E7FF, data2: 0xDF2D, data3: 0x4962, data4: [0x95, 0x85, 0x40, 0x42, 0x4D, 0x05, 0x45, 0x50] };
pub const METHOD_REPLY_COUNT: usize = 0x14;
pub const BLOB_ID: usize = 0x1c;
pub const BLOB_ID_CHARS: usize = 0x37;

pub const DEVICE_ID_CHARS: usize = 0x82;
pub const DEVICE_ID_COUNT: usize = 0x104;
pub const EVENT_KIND_GAMECORE: u32 = 2;
pub const INIT_STRUCT: usize = 0x534;
pub const MAX_OBJECT_TYPE_19041: u32 = 0x17;
pub const MAX_OBJECT_TYPE: u32 = 0x1a;
pub const MAX_VOLUMES: usize = 0x32;
pub const LIST_PAGE: usize = 8;
pub const LIST_ENTRY: usize = 0x74;
pub const TABLE_ENTRY: usize = 0x78;
pub const TABLE_SLOTS: u32 = 9;
pub const PC_ROLES: u32 = 3;
pub const SESSION_STRING_CHARS: usize = 0x200;
pub const DISPLAY_NAME_CHARS: usize = 0x3e8;
pub const PROP_DATA: usize = 0x104;

pub const CB_MAGIC: u32 = 0x4550_4344;
pub const CB_WRITE: usize = 0x10;
pub const CB_READ: usize = 0x18;
pub const CB_FLAGS: usize = 0xa4;
pub const CB_META: usize = 0xb4;
pub const CB_MAGIC_AT: usize = 0xc0;
pub const CB_CONFIG: usize = 0xc8;
pub const FLAG_POSITION_IS_READ: u32 = 4;
pub const CFG_FIXED: u32 = 0xc8;
pub const CFG_ENTRIES: usize = 0x84;
pub const CFG_DURATION: usize = 0x88;
pub const CFG_LOOPBACK: usize = 0x98;
pub const CFG_QUEUE: usize = 0x9c;
pub const CFG_DATA_START: usize = 0xa0;
pub const CFG_DATA_END: usize = 0xa4;
pub const CFG_TOTAL: usize = 0xa8;
pub const CFG_FORMAT: usize = 0xb4;
pub const QUEUE_ENTRY: u32 = 0x40;
pub const MIN_QUEUE_ENTRIES: u32 = 0x19;

pub const INIT_SHARE_MODE: usize = 0;
pub const INIT_FLAGS: usize = 4;
pub const INIT_DURATION: usize = 8;
pub const INIT_FORMAT: usize = 0x18;
pub const INIT_SESSION: usize = 0x60;
pub const INIT_ENDPOINT_CLSID: usize = 0x70;
pub const INIT_EVENT1: usize = 0x94;
pub const INIT_EVENT2: usize = 0x1b8;
pub const INIT_EVENT3: usize = 0x2dc;
pub const INIT_MEMORY: usize = 0x404;
pub const SHARED_INIT_STRUCT: usize = 0x524;
pub const SHARED_INIT_FLAGS: usize = 0x48;
pub const SHARED_INIT_PERIOD: usize = 0x4c;
pub const SHARED_INIT_SESSION: usize = 0x50;
pub const SHARED_INIT_REPLY_SHIFT: usize = 0x10;
pub const SPATIAL_CATEGORY_AT: usize = 0x48;
pub const SPATIAL_FRAMES_AT: usize = 0x4c;
pub const SPATIAL_OFFLOAD_AT: usize = 0x50;
pub const FORMAT_ENUM_AT: usize = 4;
pub const FORMAT_ENUM_TYPE: u32 = 4;
pub const FORMAT_ENUM_CB_AT: usize = 0x14;
pub const FORMAT_ENUM_ROOM: usize = 0x48;
const E_BOUNDS: i32 = 0x8000_000bu32 as i32;
pub const PLAIN_FORMAT: usize = 0x12;

pub struct InitLayout {
    pub size: usize,
    pub mode: Option<usize>,
    pub flags: usize,
    pub duration: Option<usize>,
    pub period: Option<usize>,
    pub format: usize,
    pub session: usize,
    pub clsid: usize,
    pub event1: usize,
    pub event2: usize,
    pub event3: usize,
    pub memory: usize,
}

pub const INIT_LAYOUT: InitLayout = InitLayout { size: INIT_STRUCT, mode: Some(INIT_SHARE_MODE), flags: INIT_FLAGS, duration: Some(INIT_DURATION), period: None, format: INIT_FORMAT, session: INIT_SESSION, clsid: INIT_ENDPOINT_CLSID, event1: INIT_EVENT1, event2: INIT_EVENT2, event3: INIT_EVENT3, memory: INIT_MEMORY };
pub const SHARED_INIT_LAYOUT: InitLayout = InitLayout {
    size: SHARED_INIT_STRUCT,
    mode: None,
    flags: SHARED_INIT_FLAGS,
    duration: None,
    period: Some(SHARED_INIT_PERIOD),
    format: 0,
    session: SHARED_INIT_SESSION,
    clsid: INIT_ENDPOINT_CLSID - SHARED_INIT_REPLY_SHIFT,
    event1: INIT_EVENT1 - SHARED_INIT_REPLY_SHIFT,
    event2: INIT_EVENT2 - SHARED_INIT_REPLY_SHIFT,
    event3: INIT_EVENT3 - SHARED_INIT_REPLY_SHIFT,
    memory: INIT_MEMORY - SHARED_INIT_REPLY_SHIFT,
};
pub const KIND_NONE: u32 = 0;
pub const KIND_CROSS_VM_EVENT: u32 = 2;
pub const KIND_CROSS_VM_MEMORY: u32 = 3;

pub const CLSID_CLIENT_INPUT_ENDPOINT: Guid = Guid { data1: 0xF64A6DA6, data2: 0xE8AF, data3: 0x4B7B, data4: [0xBC, 0xA8, 0x84, 0x7A, 0xE7, 0x65, 0xD5, 0x38] };
pub const CLSID_CLIENT_OUTPUT_ENDPOINT: Guid = Guid { data1: 0xCD773740, data2: 0xB187, data3: 0x4974, data4: [0xA1, 0xD5, 0xE0, 0xFF, 0x91, 0x37, 0x22, 0x77] };
pub const CLSID_XVM_EVENT: Guid = Guid { data1: 0x148EC3BF, data2: 0x349C, data3: 0x4EAC, data4: [0xB3, 0x73, 0x9F, 0xD5, 0x7C, 0xB4, 0x6D, 0x88] };
pub const GUID_XVM_EVENT: Guid = Guid { data1: 0xA963C7E3, data2: 0x8A0A, data3: 0x45FA, data4: [0xBD, 0x23, 0xAA, 0x51, 0xE4, 0xCF, 0x85, 0x1E] };
pub const CLSID_XVM_BUFFER: Guid = Guid { data1: 0xE782CED1, data2: 0x3B5D, data3: 0x43AA, data4: [0xA4, 0x5D, 0x82, 0x85, 0x2B, 0x57, 0xD8, 0x4C] };
pub const GUID_XVM_BUFFER: Guid = Guid { data1: 0x00AA2A8A, data2: 0xACB6, data3: 0x4F3D, data4: [0x9A, 0x7E, 0xBD, 0xE7, 0x12, 0x7B, 0x43, 0x87] };
const GUID_NULL: Guid = Guid { data1: 0, data2: 0, data3: 0, data4: [0; 8] };

const S_OK: i32 = 0;
const S_FALSE: i32 = 1;
const E_NOTIMPL: i32 = 0x8000_4001u32 as i32;
const E_INVALIDARG: i32 = 0x8007_0057u32 as i32;
const E_OUTOFMEMORY: i32 = 0x8007_000Eu32 as i32;
const AUDCLNT_E_NOT_INITIALIZED: i32 = 0x8889_0001u32 as i32;
const AUDCLNT_E_ALREADY_INITIALIZED: i32 = 0x8889_0002u32 as i32;
const AUDCLNT_E_NOT_STOPPED: i32 = 0x8889_0005u32 as i32;
const HNS: i64 = 10_000_000;

static HELLO: AtomicU32 = AtomicU32::new(0);

pub fn set_hello(generation: u16) {
    HELLO.store(generation as u32, Ordering::SeqCst);
}

pub fn hello_seen() -> u32 {
    HELLO.load(Ordering::SeqCst)
}

pub fn generation() -> u32 {
    match HELLO.load(Ordering::SeqCst) {
        0 => match compat_common::title_os::title_os_build() {
            Some(b) if b < 22000 => 1,
            Some(_) => 2,
            None => 1,
        },
        g => g,
    }
}

pub fn max_object_type() -> u32 {
    if generation() >= 2 { MAX_OBJECT_TYPE } else { MAX_OBJECT_TYPE_19041 }
}

pub struct Stream {
    pub pool_index: u16,
    pub base: usize,
    pub format: Vec<u8>,
    pub frames: u32,
    pub event: isize,
    pub event_name: String,
    pub renderer: wasapi::Renderer,
}

struct Object {
    kind: u32,
    parent: u32,
    device: Option<String>,
    stream: Option<Stream>,
    volumes: Option<Vec<f32>>,
    properties: Option<[u32; 4]>,
    notify: bool,
    notify_id: u32,
    watch: Option<String>,
    list_cursor: usize,
    pc_sessions: Vec<Arc<wasapi::PcSession>>,
    pc_session: Option<Arc<wasapi::PcSession>>,
    media: Option<(wasapi::MediaManager, i64)>,
    spatial: Option<SpatialHost>,
    formats: Option<wasapi::FormatEnumerator>,
}

impl Object {
    fn new(kind: u32, parent: u32, device: Option<String>) -> Self {
        Object { kind, parent, device, stream: None, volumes: None, properties: None, notify: false, notify_id: 0, watch: None, list_cursor: 0, pc_sessions: Vec::new(), pc_session: None, media: None, spatial: None, formats: None }
    }
}

struct Endpoint {
    scalar: f32,
    mute: bool,
    channels: Vec<f32>,
}

#[derive(Default)]
struct State {
    objects: HashMap<u32, Object>,
    endpoints: HashMap<String, Endpoint>,
    next: u32,
    next_event: u32,
    session: Option<Guid>,
    session_channels: Option<Vec<f32>>,
    session_simple: Option<(f32, bool)>,
}

fn state() -> MutexGuard<'static, State> {
    static S: OnceLock<Mutex<State>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(State { next: 1, ..Default::default() })).lock().unwrap_or_else(|e| e.into_inner())
}

fn guid_bytes(g: &Guid) -> [u8; 16] {
    let mut b = [0u8; 16];
    b[0..4].copy_from_slice(&g.data1.to_le_bytes());
    b[4..6].copy_from_slice(&g.data2.to_le_bytes());
    b[6..8].copy_from_slice(&g.data3.to_le_bytes());
    b[8..16].copy_from_slice(&g.data4);
    b
}

fn guid_from(b: &[u8]) -> Guid {
    Guid { data1: u32::from_le_bytes(b[0..4].try_into().unwrap()), data2: u16::from_le_bytes([b[4], b[5]]), data3: u16::from_le_bytes([b[6], b[7]]), data4: b[8..16].try_into().unwrap() }
}

fn align_up(v: u32, a: u32) -> u32 {
    (v + a - 1) & !(a - 1)
}

pub fn event_name(index: u16) -> String {
    format!("{}{index}", crate::vab::GAMECORE_BASE)
}

pub struct Layout {
    pub frames: u32,
    pub bytes: u32,
    pub entries: u32,
    pub queue: u32,
    pub start: u32,
    pub end: u32,
    pub total: u32,
}

pub fn layout(format: &[u8], frames: u32, period_frames: u32) -> Layout {
    let cb = (wasapi::format_len(format) - 0x12) as u32;
    let bytes = frames * wasapi::block_align(format);
    let entries = (frames / period_frames.max(1) + 3).max(MIN_QUEUE_ENTRIES);
    let size = CFG_FIXED + cb;
    let queue = align_up(CB_CONFIG as u32 + size, 0x80);
    let start = align_up(queue + entries * QUEUE_ENTRY, 0x400);
    let end = start + bytes;
    Layout { frames, bytes, entries, queue, start, end, total: end }
}

pub unsafe fn write_control_block(base: usize, format: &[u8], duration: i64, l: &Layout) {
    let p = base as *mut u8;
    core::ptr::write_bytes(p, 0, l.start as usize);
    let w32 = |o: usize, v: u32| (p.add(o) as *mut u32).write_unaligned(v);
    w32(CB_FLAGS, FLAG_POSITION_IS_READ);
    w32(CB_META, u32::MAX);
    w32(CB_MAGIC_AT, CB_MAGIC);
    let c = CB_CONFIG;
    let flen = wasapi::format_len(format);
    w32(c, CFG_FIXED + (flen as u32 - 0x12));
    w32(c + CFG_ENTRIES, l.entries);
    (p.add(c + CFG_DURATION) as *mut i64).write_unaligned(duration);
    w32(c + CFG_LOOPBACK, u32::MAX);
    w32(c + CFG_QUEUE, l.queue);
    w32(c + CFG_DATA_START, l.start);
    w32(c + CFG_DATA_END, l.end);
    w32(c + CFG_TOTAL, l.total);
    core::ptr::copy_nonoverlapping(format.as_ptr(), p.add(c + CFG_FORMAT), flen);
}

unsafe fn rd<T: Copy>(p: *const u8, o: usize) -> T {
    (p.add(o) as *const T).read_unaligned()
}

unsafe fn wr<T: Copy>(p: *mut u8, o: usize, v: T) {
    (p.add(o) as *mut T).write_unaligned(v)
}

unsafe fn wide_at(p: *const u8, chars: usize) -> Option<String> {
    let s = core::slice::from_raw_parts(p as *const u16, chars);
    let n = s.iter().position(|&c| c == 0)?;
    Some(String::from_utf16_lossy(&s[..n]))
}

unsafe fn put_wide(p: *mut u8, s: &str, chars: usize) -> bool {
    let w: Vec<u16> = s.encode_utf16().collect();
    if w.len() >= chars {
        return false;
    }
    core::ptr::write_bytes(p, 0, chars * 2);
    core::ptr::copy_nonoverlapping(w.as_ptr() as *const u8, p, w.len() * 2);
    true
}

pub unsafe fn call(cb: i32, env: *mut u8) -> i32 {
    if env.is_null() || (cb as usize) < PAYLOAD {
        return E_INVALIDARG;
    }
    let id: u32 = rd(env, 8);
    let object: u32 = rd(env, 0xc);
    let a = env.add(PAYLOAD);
    let room = cb as usize - PAYLOAD;
    let hr = dispatch(id, object, a, room);
    wr(env, 4, hr);
    wr(env, 8, id);
    compat_common::record_raw("crossvm-audio", "call", &format!("id {id:#x} object {object:#x} -> {:#010x}", hr as u32));
    S_OK
}

fn device_of(s: &State, mut object: u32) -> Option<String> {
    for _ in 0..16 {
        let o = s.objects.get(&object)?;
        if o.device.is_some() {
            return o.device.clone();
        }
        if o.parent == 0 || o.parent == object {
            return None;
        }
        object = o.parent;
    }
    None
}

fn stream_owner(s: &State, mut object: u32) -> Option<u32> {
    for _ in 0..16 {
        let o = s.objects.get(&object)?;
        if o.stream.is_some() {
            return Some(object);
        }
        if o.parent == 0 || o.parent == object {
            return None;
        }
        object = o.parent;
    }
    None
}

fn device_for(object: u32) -> Option<String> {
    device_of(&state(), object)
}

fn pc_session_of(s: &State, mut object: u32) -> Option<Arc<wasapi::PcSession>> {
    for _ in 0..16 {
        let o = s.objects.get(&object)?;
        if let Some(p) = &o.pc_session {
            return Some(p.clone());
        }
        if o.parent == 0 || o.parent == object {
            return None;
        }
        object = o.parent;
    }
    None
}

fn session_ref(object: u32) -> wasapi::SessionRef {
    let s = state();
    match pc_session_of(&s, object) {
        Some(p) => wasapi::SessionRef::Pc(p),
        None => wasapi::SessionRef::Title(device_of(&s, object), s.session.unwrap_or(GUID_NULL)),
    }
}

fn endpoint_key(dev: &Option<String>) -> String {
    dev.clone().or_else(|| wasapi::device_id().ok()).unwrap_or_default()
}

fn endpoint_channels(dev: &Option<String>) -> u32 {
    wasapi::endpoint_caps(dev.as_deref()).map(|c| c.channels).unwrap_or(2).max(1)
}

fn with_endpoint<R>(dev: &Option<String>, f: impl FnOnce(&mut Endpoint) -> R) -> R {
    let n = endpoint_channels(dev) as usize;
    let key = endpoint_key(dev);
    let mut s = state();
    let e = s.endpoints.entry(key).or_insert_with(|| Endpoint { scalar: 1.0, mute: false, channels: vec![1.0; n] });
    let r = f(e);
    apply_all(&mut s);
    r
}

fn apply_all(s: &mut State) {
    let keys: Vec<u32> = s.objects.iter().filter(|(_, o)| o.stream.is_some()).map(|(&k, _)| k).collect();
    for k in keys {
        let dev = device_of(s, k);
        let key = endpoint_key(&dev);
        let (es, em, ech) = s.endpoints.get(&key).map(|e| (e.scalar, e.mute, e.channels.clone())).unwrap_or((1.0, false, Vec::new()));
        let o = s.objects.get(&k).unwrap();
        let st = o.stream.as_ref().unwrap();
        let (sg, sm) = s.session_simple.unwrap_or((1.0, false));
        let gain = if sm || em { 0.0 } else { sg * es };
        *st.renderer.shared.gain.lock().unwrap() = Some(gain.clamp(0.0, 1.0));
        let n = wasapi::channels(&st.format) as usize;
        let mut v = o.volumes.clone().unwrap_or_else(|| vec![1.0; n]);
        for (i, c) in v.iter_mut().enumerate() {
            *c *= ech.get(i).copied().unwrap_or(1.0) * s.session_channels.as_ref().and_then(|sc| sc.get(i).copied()).unwrap_or(1.0);
        }
        *st.renderer.shared.channels.lock().unwrap() = Some(v);
    }
}

fn level_to_scalar(db: f32, caps: &wasapi::EndpointCaps) -> f32 {
    if db <= caps.min_db {
        0.0
    } else {
        10f32.powf(db.min(caps.max_db).min(0.0) / 20.0)
    }
}

fn scalar_to_level(v: f32, caps: &wasapi::EndpointCaps) -> f32 {
    if v <= 0.0 {
        caps.min_db
    } else {
        (20.0 * v.log10()).clamp(caps.min_db, caps.max_db)
    }
}

unsafe fn dispatch(id: u32, object: u32, a: *mut u8, room: usize) -> i32 {
    if id != CREATE_OBJECT && id != RELEASE_OBJECT {
        if let Some(hr) = spatial_call(id, object, a) {
            return hr;
        }
        let enumerator = state().objects.get(&object).map(|o| o.kind == FORMAT_ENUM_TYPE).unwrap_or(false);
        if enumerator || id == FORMAT_ENUM_COUNT || id == FORMAT_ENUM_GET {
            return format_enum(id, object, a);
        }
    }
    match id {
        CREATE_OBJECT => create_object(a, room),
        RELEASE_OBJECT => {
            let o = state().objects.remove(&object);
            if let Some(Object { media: Some((m, token)), .. }) = &o {
                let hr = m.remove_handler(*token);
                compat_common::record_raw("crossvm-audio", "media-manager", &format!("object {object:#x} released, PC MediaManager handler {token:#x} removed -> {:#010x}", hr as u32));
            }
            if let Some(Object { stream: Some(st), .. }) = o {
                drop(st.renderer);
                crate::logan::pool_release(st.pool_index as usize);
            }
            S_OK
        }
        MEDIA_PLAY_TO_STREAM_STATE | MEDIA_CASTING_APP_STATE | MEDIA_VOIP_CALL_STATE => {
            let value: u32 = rd(a, 0);
            let method = match id {
                MEDIA_PLAY_TO_STREAM_STATE => wasapi::MEDIA_PLAY_TO_STREAM_STATE,
                MEDIA_CASTING_APP_STATE => wasapi::MEDIA_CASTING_APP_STATE,
                _ => wasapi::MEDIA_VOIP_CALL_STATE,
            };
            let s = state();
            match s.objects.get(&object).and_then(|o| o.media.as_ref()) {
                Some((m, _)) => m.state_changed(method, value),
                None => E_INVALIDARG,
            }
        }
        GET_MIX_FORMAT => match wasapi::device_info_for(device_for(object).as_deref()) {
            Ok(d) => {
                core::ptr::write_bytes(a, 0, wasapi::MAX_FORMAT.min(room));
                core::ptr::copy_nonoverlapping(d.mix.as_ptr(), a, d.mix.len());
                S_OK
            }
            Err(hr) => hr,
        },
        GET_DEVICE_PERIOD => match wasapi::device_info_for(device_for(object).as_deref()) {
            Ok(d) => {
                wr(a, 2, d.default_period);
                wr(a, 0xa, d.min_period);
                S_OK
            }
            Err(hr) => hr,
        },
        GET_STREAM_LATENCY => {
            let s = state();
            match stream_owner(&s, object).and_then(|k| s.objects.get(&k)).and_then(|o| o.stream.as_ref()) {
                Some(st) => {
                    wr(a, 0, st.renderer.stream_latency());
                    S_OK
                }
                None => AUDCLNT_E_NOT_INITIALIZED,
            }
        }
        IS_FORMAT_SUPPORTED => is_format_supported(object, a),
        INITIALIZE => initialize(object, a, room, &INIT_LAYOUT),
        INITIALIZE_SHARED_AUDIO_STREAM => initialize(object, a, room, &SHARED_INIT_LAYOUT),
        START | STOP | RESET => transport(id, object),
        GET_BUFFER_SIZE_LIMITS => {
            let f = format_at(a);
            let event: u8 = rd(a, wasapi::MAX_FORMAT);
            match wasapi::buffer_size_limits(device_for(object).as_deref(), &f, event != 0) {
                Ok((lo, hi)) => {
                    wr(a, 0x48, hi);
                    wr(a, 0x50, lo);
                    S_OK
                }
                Err(hr) => hr,
            }
        }
        IS_OFFLOAD_CAPABLE => {
            let cat: u32 = rd(a, 0);
            match wasapi::offload_capable(device_for(object).as_deref(), cat) {
                Ok(b) => {
                    wr(a, 0, b);
                    S_OK
                }
                Err(hr) => hr,
            }
        }
        SET_CLIENT_PROPERTIES => {
            let offload: u8 = rd(a, 0);
            let cat: u32 = rd(a, 1);
            let opts: u32 = rd(a, 5);
            let mut s = state();
            match s.objects.get_mut(&object) {
                Some(o) if o.stream.is_some() => AUDCLNT_E_ALREADY_INITIALIZED,
                Some(o) => {
                    o.properties = Some([offload as u32, cat, opts, 0]);
                    S_OK
                }
                None => E_INVALIDARG,
            }
        }
        GET_SHARED_MODE_ENGINE_PERIOD => {
            let f = format_at(a);
            match wasapi::shared_mode_engine_period(device_for(object).as_deref(), &f) {
                Ok(v) => {
                    for (i, x) in v.iter().enumerate() {
                        wr(a, 0x48 + 4 * i, *x);
                    }
                    S_OK
                }
                Err(hr) => hr,
            }
        }
        GET_CURRENT_SHARED_MODE_ENGINE_PERIOD => match wasapi::current_shared_mode_engine_period(device_for(object).as_deref()) {
            Ok((f, p)) => {
                core::ptr::write_bytes(a, 0, wasapi::MAX_FORMAT);
                core::ptr::copy_nonoverlapping(f.as_ptr(), a, f.len());
                wr(a, 0x48, p);
                S_OK
            }
            Err(hr) => hr,
        },
        STREAM_GET_CHANNEL_COUNT | STREAM_SET_CHANNEL_VOLUME | STREAM_GET_CHANNEL_VOLUME | STREAM_SET_ALL_VOLUMES | STREAM_GET_ALL_VOLUMES => stream_volume(id, object, a),
        SET_SIMPLE_VOLUME | SET_SIMPLE_MUTE | GET_SIMPLE_VOLUME | GET_SIMPLE_MUTE if matches!(session_ref(object), wasapi::SessionRef::Pc(_)) => {
            let r = session_ref(object);
            match id {
                SET_SIMPLE_VOLUME => {
                    let v: f32 = rd(a, 0);
                    if !(0.0..=1.0).contains(&v) {
                        return E_INVALIDARG;
                    }
                    wasapi::session_simple_set(&r, Some(v), None, &guid_from(&ctx_at(a, 4)))
                }
                SET_SIMPLE_MUTE => {
                    let m: u8 = rd(a, 0);
                    wasapi::session_simple_set(&r, None, Some(m != 0), &guid_from(&ctx_at(a, 1)))
                }
                _ => match wasapi::session_simple_get(&r) {
                    Ok((g, m)) => {
                        if id == GET_SIMPLE_VOLUME {
                            wr(a, 0, g);
                        } else {
                            wr(a, 0, m as u8);
                        }
                        S_OK
                    }
                    Err(hr) => hr,
                },
            }
        }
        SET_SIMPLE_VOLUME => {
            let v: f32 = rd(a, 0);
            if !(0.0..=1.0).contains(&v) {
                return E_INVALIDARG;
            }
            let ctx = ctx_at(a, 4);
            match simple(object, |g, m| {
                *g = v;
                (*g, *m)
            }) {
                Some((g, m)) => {
                    session_changed(|sid| session_simple_record(sid, &ctx, g, m));
                    S_OK
                }
                None => E_INVALIDARG,
            }
        }
        SET_SIMPLE_MUTE => {
            let m: u8 = rd(a, 0);
            let ctx = ctx_at(a, 1);
            match simple(object, |g, mu| {
                *mu = m != 0;
                (*g, *mu)
            }) {
                Some((g, m)) => {
                    session_changed(|sid| session_simple_record(sid, &ctx, g, m));
                    S_OK
                }
                None => E_INVALIDARG,
            }
        }
        GET_SIMPLE_VOLUME => match simple(object, |g, _| *g) {
            Some(g) => {
                wr(a, 0, g);
                S_OK
            }
            None => E_INVALIDARG,
        },
        GET_SIMPLE_MUTE => match simple(object, |_, m| *m) {
            Some(m) => {
                wr(a, 0, m as u8);
                S_OK
            }
            None => E_INVALIDARG,
        },
        SESSION_GET_STATE => match wasapi::session_state(&session_ref(object)) {
            Ok(st) => {
                wr(a, 0, st);
                S_OK
            }
            Err(hr) => hr,
        },
        SESSION_GET_DISPLAY_NAME | SESSION_GET_IDENTIFIER | SESSION_GET_INSTANCE_IDENTIFIER => {
            let (slot, chars) = match id {
                SESSION_GET_DISPLAY_NAME => (4, DISPLAY_NAME_CHARS),
                SESSION_GET_IDENTIFIER => (12, SESSION_STRING_CHARS),
                _ => (13, SESSION_STRING_CHARS),
            };
            match wasapi::session_string(&session_ref(object), slot) {
                Ok(t) => {
                    if put_wide(a, &t, chars) { S_OK } else { E_OUTOFMEMORY }
                }
                Err(hr) => hr,
            }
        }
        SESSION_SET_DISPLAY_NAME => {
            let Some(name) = wide_at(a, DISPLAY_NAME_CHARS) else { return E_INVALIDARG };
            let ctx = guid_from(core::slice::from_raw_parts(a.add(DISPLAY_NAME_CHARS * 2), 16));
            let w: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
            wasapi::session_set_display_name(&session_ref(object), &w, &ctx)
        }
        SESSION_GET_GROUPING => match wasapi::session_grouping(&session_ref(object)) {
            Ok(g) => {
                core::ptr::copy_nonoverlapping(guid_bytes(&g).as_ptr(), a, 16);
                S_OK
            }
            Err(hr) => hr,
        },
        SESSION_SET_GROUPING => {
            let g = guid_from(core::slice::from_raw_parts(a, 16));
            let ctx = guid_from(core::slice::from_raw_parts(a.add(16), 16));
            wasapi::session_set_grouping(&session_ref(object), &g, &ctx)
        }
        SESSION_IS_SYSTEM_SOUNDS => {
            let hr = wasapi::session_is_system_sounds(&session_ref(object));
            if hr < 0 {
                return hr;
            }
            wr(a, 0, hr as u32);
            S_OK
        }
        SESSION_SET_DUCKING => {
            let b: u32 = rd(a, 0);
            wasapi::session_set_ducking(&session_ref(object), b != 0)
        }
        SESSION_REGISTER_NOTIFICATION | SESSION_MANAGER_REGISTER | ENDPOINT_REGISTER | ENUMERATOR_SUBSCRIBE => subscribe(object, id, true),
        SESSION_MANAGER_UNREGISTER | ENDPOINT_UNREGISTER | ENUMERATOR_UNSUBSCRIBE => subscribe(object, id, false),
        ENDPOINT_HOST_STREAM_MODES => {
            wr(a, 0, 0u32);
            wr(a, 4, 0u32);
            S_OK
        }
        ENDPOINT_OFFLOAD_SERVICE => match wasapi::endpoint_offload(device_for(object).as_deref()) {
            Ok(b) => {
                wr(a, 0, b);
                S_OK
            }
            Err(hr) => hr,
        },
        ITEM_GET | ITEM_COUNT | ITEM_FLAG | ITEM_VALUE => {
            let s = state();
            let Some(st) = stream_owner(&s, object).and_then(|k| s.objects.get(&k)).and_then(|o| o.stream.as_ref()) else {
                return AUDCLNT_E_NOT_INITIALIZED;
            };
            let r = &st.renderer;
            let index: u32 = rd(a, 0);
            match id {
                ITEM_COUNT => r.item_count().map(|n| wr(a, 0, n)),
                ITEM_FLAG => r.item_flag().map(|b| wr(a, 0, b as u8)),
                ITEM_GET => r.item_value(index).map(|_| ()),
                _ => r.item_value(index).map(|v| wr(a, 4, v)),
            }
            .err()
            .unwrap_or(S_OK)
        }
        CLOCK_SET_SAMPLE_RATE => {
            let r: f32 = rd(a, 0);
            let s = state();
            match stream_owner(&s, object).and_then(|k| s.objects.get(&k)).and_then(|o| o.stream.as_ref()) {
                Some(st) => st.renderer.set_sample_rate(r),
                None => AUDCLNT_E_NOT_INITIALIZED,
            }
        }
        DEVICE_GET_ID => match device_for(object).map(Ok).unwrap_or_else(wasapi::device_id) {
            Ok(id) => {
                let w: Vec<u16> = id.encode_utf16().take(DEVICE_ID_CHARS - 1).chain(Some(0)).collect();
                core::ptr::write_bytes(a, 0, DEVICE_ID_COUNT + 8);
                core::ptr::copy_nonoverlapping(w.as_ptr() as *const u8, a, w.len() * 2);
                wr(a, DEVICE_ID_COUNT, (w.len() * 2) as u64);
                S_OK
            }
            Err(hr) => hr,
        },
        DEVICE_GET_STATE => match wasapi::device_state(device_for(object).as_deref()) {
            Ok(st) => {
                wr(a, 0, st);
                S_OK
            }
            Err(hr) => hr,
        },
        PROPERTY_GET_COUNT => match wasapi::property_count(device_for(object).as_deref()) {
            Ok(n) => {
                wr(a, 0, n);
                S_OK
            }
            Err(hr) => hr,
        },
        PROPERTY_GET_AT => {
            let i: u32 = rd(a, 0);
            match wasapi::property_at(device_for(object).as_deref(), i) {
                Ok(k) => {
                    core::ptr::copy_nonoverlapping(k.fmtid.as_ptr(), a.add(4), 16);
                    wr(a, 0x14, k.pid);
                    S_OK
                }
                Err(hr) => hr,
            }
        }
        PROPERTY_GET_VALUE => property_value(object, a, room),
        SPATIAL_STATIC_POSITION | SPATIAL_NATIVE_MASK | SPATIAL_MAX_DYNAMIC | SPATIAL_MAX_FRAMES | SPATIAL_FORMAT_SUPPORTED | SPATIAL_STREAM_AVAILABLE | SPATIAL_OFFLOAD_CAPABLE | SPATIAL_MAX_FRAMES_FOR_CATEGORY => spatial(id, object, a),
        ENDPOINT_CHANNEL_COUNT => {
            wr(a, 0, endpoint_channels(&device_for(object)));
            S_OK
        }
        SET_ENDPOINT_VOLUME_LEVEL | SET_ENDPOINT_VOLUME_SCALAR | GET_ENDPOINT_VOLUME_LEVEL | GET_ENDPOINT_VOLUME_SCALAR | SET_ENDPOINT_CHANNEL_LEVEL | SET_ENDPOINT_CHANNEL_SCALAR | GET_ENDPOINT_CHANNEL_LEVEL | GET_ENDPOINT_CHANNEL_SCALAR | SET_ENDPOINT_MUTE | GET_ENDPOINT_MUTE | ENDPOINT_STEP_INFO | ENDPOINT_STEP_UP | ENDPOINT_STEP_DOWN | ENDPOINT_HARDWARE_SUPPORT | ENDPOINT_VOLUME_RANGE => endpoint_volume(id, object, a),
        ENDPOINT_LIST => endpoint_list(object, a),
        DEFAULT_ENDPOINT_TABLE => default_table(a),
        SESSION_CHANNEL_COUNT | SESSION_SET_CHANNEL_VOLUME | SESSION_GET_CHANNEL_VOLUME | SESSION_SET_ALL_VOLUMES | SESSION_GET_ALL_VOLUMES => session_volume(id, object, a),
        _ => {
            compat_common::record_raw("crossvm-audio", "unserved", &format!("call id {id:#x} has no decoded argument layout; E_NOTIMPL"));
            E_NOTIMPL
        }
    }
}

unsafe fn format_at(a: *const u8) -> Vec<u8> {
    let head = core::slice::from_raw_parts(a, 0x12);
    core::slice::from_raw_parts(a, wasapi::format_len(head).max(0x12)).to_vec()
}

unsafe fn create_object(a: *mut u8, room: usize) -> i32 {
    if room < 0x14 {
        return E_INVALIDARG;
    }
    let kind: u32 = rd(a, 0);
    let flag: u32 = rd(a, 4);
    let parent: u32 = rd(a, 8);
    let method: u32 = rd(a, 0x10);
    if kind >= max_object_type() || flag > 1 {
        return E_INVALIDARG;
    }
    if (kind == MEDIA_MANAGER_TYPE) != (method == METHOD_MEDIA_MANAGER) || (kind == EXISTING_SESSION_TYPE) != (method == METHOD_EXISTING_SESSION) {
        compat_common::record_raw("crossvm-audio", "CreateObject", &format!("type {kind:#x} with method {method:#x}: the host creator for this type checks its method; E_INVALIDARG"));
        return E_INVALIDARG;
    }
    let device = match method {
        METHOD_DEFAULT_ENDPOINT | METHOD_DEFAULT_ENDPOINT_19041 => {
            let flow: u32 = rd(a, 0x14);
            let role: u32 = rd(a, 0x18);
            if flow > 1 || role >= PC_ROLES {
                return E_INVALIDARG;
            }
            match wasapi::default_id(flow, role) {
                Ok(id) => {
                    if method == METHOD_DEFAULT_ENDPOINT && !put_wide(a.add(BLOB_ID), &id, BLOB_ID_CHARS + 1) {
                        compat_common::record_raw("crossvm-audio", "CreateObject", &format!("PC endpoint id {id} is longer than the {BLOB_ID_CHARS}-char guest field"));
                        return E_INVALIDARG;
                    }
                    Some(id)
                }
                Err(hr) => return hr,
            }
        }
        METHOD_ENDPOINT | METHOD_ENDPOINT_19041 => match wide_at(a.add(BLOB_ID), BLOB_ID_CHARS + 1) {
            Some(id) if !id.is_empty() => match wasapi::device_state(Some(&id)) {
                Ok(_) => Some(id),
                Err(hr) => return hr,
            },
            _ if method == METHOD_ENDPOINT => return E_INVALIDARG,
            _ => None,
        },
        _ => None,
    };
    if let Some(hr) = spatial_stream_create(kind, method, parent, a, room) {
        return hr;
    }
    let mut formats = None;
    if kind == FORMAT_ENUM_TYPE {
        let pc = wasapi::spatial(device_for(parent).as_deref()).and_then(|sp| sp.format_enumerator());
        match pc {
            Ok(e) => formats = Some(e),
            Err(hr) => {
                compat_common::record_raw("crossvm-audio", "CreateObject", &format!("type 4 (125980 -> 124574, IAudioFormatEnumerator child of the spatial client) parent {parent:#x}: PC ISpatialAudioClient::GetSupportedAudioObjectFormatEnumerator -> {:#010x}", hr as u32));
                return hr;
            }
        }
    }
    let mut sessions = Vec::new();
    let mut session = None;
    let mut session_parent = None;
    let mut media = None;
    match method {
        METHOD_MEDIA_MANAGER => {
            let m = match wasapi::media_manager() {
                Ok(m) => m,
                Err(hr) => return hr,
            };
            let token = match m.add_handler() {
                Ok(t) => t,
                Err(hr) => return hr,
            };
            media = Some((m, token));
        }
        METHOD_EXISTING_SESSION => {
            if room < METHOD_ARGS + EXISTING_SESSION_CHARS * 2 {
                return E_INVALIDARG;
            }
            let chars = core::slice::from_raw_parts(a.add(METHOD_ARGS) as *const u16, EXISTING_SESSION_CHARS);
            if chars[EXISTING_SESSION_CHARS - 1] != 0 {
                return E_INVALIDARG;
            }
            let Some(key) = wide_at(a.add(METHOD_ARGS), EXISTING_SESSION_CHARS) else { return E_INVALIDARG };
            let p = match wasapi::session_existing(device_for(parent).as_deref(), &key) {
                Ok(p) => p,
                Err(hr) => {
                    compat_common::record_raw("crossvm-audio", "CreateObject", &format!("existing session {key:?} -> {:#010x}", hr as u32));
                    return hr;
                }
            };
            let title = {
                let s = state();
                wasapi::SessionRef::Title(device_of(&s, parent), s.session.unwrap_or(GUID_NULL))
            };
            let same = wasapi::session_instance(&title).map(|t| t.eq_ignore_ascii_case(&key)).unwrap_or(false);
            if !same {
                session = Some(p);
            }
            session_parent = Some(parent);
        }
        METHOD_SESSION_ENUMERATOR => {
            sessions = match wasapi::session_enumerate(device_for(parent).as_deref()) {
                Ok(l) => l,
                Err(hr) => return hr,
            };
            wr(a, METHOD_REPLY_COUNT, sessions.len() as u32);
        }
        METHOD_ENUMERATED_SESSION => {
            let index: u32 = rd(a, METHOD_ARGS);
            let s = state();
            let Some(e) = s.objects.get(&parent) else { return E_INVALIDARG };
            let Some(p) = e.pc_sessions.get(index as usize).cloned() else { return E_INVALIDARG };
            let title = wasapi::SessionRef::Title(device_of(&s, parent), s.session.unwrap_or(GUID_NULL));
            drop(s);
            let same = match (wasapi::session_instance(&wasapi::SessionRef::Pc(p.clone())), wasapi::session_instance(&title)) {
                (Ok(a), Ok(b)) => a.eq_ignore_ascii_case(&b),
                (Err(hr), _) => return hr,
                _ => false,
            };
            if !same {
                session = Some(p);
            }
            session_parent = Some(parent);
        }
        _ => {}
    }
    let mut s = state();
    let n = s.next;
    s.next += 1;
    compat_common::record_raw("crossvm-audio", "CreateObject", &format!("type {kind:#x} flag {flag} parent {parent:#x} method {method:#x} device {device:?} sessions {} -> object {n:#x}", sessions.len()));
    let mut o = Object::new(kind, session_parent.unwrap_or(if flag == 1 { 0 } else { parent }), device);
    o.pc_sessions = sessions;
    o.pc_session = session;
    o.media = media;
    o.formats = formats;
    s.objects.insert(n, o);
    wr(a, 0xc, n);
    S_OK
}

pub fn spatial_stream_methods(kind: u32) -> Option<(u32, u32)> {
    match kind {
        SPATIAL_STREAM_TYPE => Some((METHOD_SPATIAL_STREAM, METHOD_SPATIAL_STREAM2)),
        SPATIAL_HRTF_STREAM_TYPE => Some((METHOD_SPATIAL_HRTF_STREAM, METHOD_SPATIAL_HRTF_STREAM2)),
        SPATIAL_METADATA_STREAM_TYPE => Some((METHOD_SPATIAL_METADATA_STREAM, METHOD_SPATIAL_METADATA_STREAM2)),
        _ => None,
    }
}

pub struct SpatialLayout {
    pub format: usize,
    pub options: Option<usize>,
    pub reply: SpatialReply,
    pub tail: Option<usize>,
}

pub const SPATIAL_HRTF_FLAGS: usize = 0x24;
pub const SPATIAL_HRTF_DECAY: usize = 0x28;
pub const SPATIAL_HRTF_DIRECTIVITY: usize = 0x3c;
pub const SPATIAL_HRTF_ENVIRONMENT: usize = 0x4c;
pub const SPATIAL_HRTF_ORIENTATION: usize = 0x50;
pub const SPATIAL_METADATA_FORMAT_ID: usize = 0x24;
pub const SPATIAL_METADATA_MAX_ITEMS: usize = 0x34;

pub fn spatial_layout(kind: u32, method: u32) -> Option<SpatialLayout> {
    Some(match (kind, method) {
        (SPATIAL_STREAM_TYPE, METHOD_SPATIAL_STREAM) => SpatialLayout { format: SPATIAL_ARG_FORMAT, options: None, reply: SPATIAL_REPLY, tail: None },
        (SPATIAL_STREAM_TYPE, METHOD_SPATIAL_STREAM2) => SpatialLayout { format: SPATIAL_ARG_FORMAT, options: Some(SPATIAL_ARG_OPTIONS), reply: SPATIAL_REPLY2, tail: None },
        (SPATIAL_HRTF_STREAM_TYPE, METHOD_SPATIAL_HRTF_STREAM) => SpatialLayout { format: 0x74, options: None, reply: SpatialReply { extra: 0xbc, clsid: 0xc0, endpoint_value: 0xd0, token: 0xd4, stream_id: 0xdc, event: 0xe4, stream_value: 0x208, memory: 0x20c }, tail: None },
        (SPATIAL_HRTF_STREAM_TYPE, METHOD_SPATIAL_HRTF_STREAM2) => SpatialLayout { format: 0x74, options: Some(0xc0), reply: SpatialReply { extra: 0xbc, clsid: 0xc4, endpoint_value: 0xd4, token: 0xd8, stream_id: 0xe0, event: 0xe8, stream_value: 0x20c, memory: 0x210 }, tail: None },
        (SPATIAL_METADATA_STREAM_TYPE, METHOD_SPATIAL_METADATA_STREAM) => SpatialLayout { format: 0x36, options: None, reply: SpatialReply { extra: 0x7e, clsid: 0x82, endpoint_value: 0x92, token: 0x96, stream_id: 0x9e, event: 0xa6, stream_value: 0x1ca, memory: 0x1ce }, tail: Some(0x2f2) },
        (SPATIAL_METADATA_STREAM_TYPE, METHOD_SPATIAL_METADATA_STREAM2) => SpatialLayout { format: 0x38, options: Some(0x84), reply: SpatialReply { extra: 0x80, clsid: 0x88, endpoint_value: 0x98, token: 0x9c, stream_id: 0xa4, event: 0xac, stream_value: 0x1d0, memory: 0x1d4 }, tail: Some(0x2f8) },
        _ => return None,
    })
}

pub fn spatial_layout_end(l: &SpatialLayout) -> usize {
    l.tail.map(|t| t + 8).unwrap_or(l.reply.memory + SPATIAL_DESCRIPTOR_BYTES).max(l.options.map(|o| o + 4).unwrap_or(0)).max(l.format + 0x12)
}

unsafe fn spatial_extra(kind: u32, method: u32, a: *const u8) -> wasapi::SpatialExtra {
    let copy = |o: usize, n: usize| core::slice::from_raw_parts(a.add(o), n).to_vec();
    match kind {
        SPATIAL_HRTF_STREAM_TYPE => {
            let has = |i: usize| *a.add(SPATIAL_HRTF_FLAGS + i) != 0;
            wasapi::SpatialExtra::Hrtf {
                decay: has(0).then(|| copy(SPATIAL_HRTF_DECAY, 0x14).try_into().unwrap()),
                directivity: has(1).then(|| copy(SPATIAL_HRTF_DIRECTIVITY, 0x10).try_into().unwrap()),
                environment: has(2).then(|| rd(a, SPATIAL_HRTF_ENVIRONMENT)),
                orientation: has(3).then(|| copy(SPATIAL_HRTF_ORIENTATION, 0x24).try_into().unwrap()),
            }
        }
        SPATIAL_METADATA_STREAM_TYPE => wasapi::SpatialExtra::Metadata {
            format_id: copy(SPATIAL_METADATA_FORMAT_ID, 16).try_into().unwrap(),
            max_items: if method == METHOD_SPATIAL_METADATA_STREAM2 { rd::<u32>(a, SPATIAL_METADATA_MAX_ITEMS) } else { rd::<u16>(a, SPATIAL_METADATA_MAX_ITEMS) as u32 },
        },
        _ => wasapi::SpatialExtra::Plain,
    }
}

unsafe fn spatial_stream_create(kind: u32, method: u32, parent: u32, a: *mut u8, room: usize) -> Option<i32> {
    spatial_stream_methods(kind)?;
    let Some(lay) = spatial_layout(kind, method) else { return Some(E_INVALIDARG) };
    if room < spatial_layout_end(&lay) {
        return Some(E_INVALIDARG);
    }
    let head = core::slice::from_raw_parts(a.add(lay.format), 0x12);
    let flen = u16::from_le_bytes([head[0x10], head[0x11]]) as usize + 0x12;
    if flen > SPATIAL_FORMAT_MAX || lay.format + flen > room {
        return Some(E_INVALIDARG);
    }
    let params = wasapi::SpatialStreamParams {
        format: core::slice::from_raw_parts(a.add(lay.format), flen).to_vec(),
        mask: rd(a, SPATIAL_ARG_MASK),
        min: rd(a, SPATIAL_ARG_MIN),
        max: rd(a, SPATIAL_ARG_MAX),
        category: rd(a, SPATIAL_ARG_CATEGORY),
        options: lay.options.map(|o| rd(a, o)).unwrap_or(0),
        extra: spatial_extra(kind, method, a),
    };
    let dev = device_for(parent);
    let sp = match wasapi::spatial(dev.as_deref()) {
        Ok(sp) => sp,
        Err(hr) => return Some(hr),
    };
    let stream = match sp.activate_stream(&params) {
        Ok(st) => st,
        Err(hr) => {
            compat_common::record_raw("crossvm-audio", "CreateObject", &format!("spatial stream type {kind:#x} method {method:#x} mask {:#x} min {} max {} category {}: PC ActivateSpatialAudioStream -> {:#010x}", params.mask, params.min, params.max, params.category, hr as u32));
            return Some(hr);
        }
    };
    SPATIAL_ACTIVATIONS.fetch_add(1, Ordering::SeqCst);
    if kind == SPATIAL_METADATA_STREAM_TYPE {
        drop(stream);
        compat_common::record_raw("crossvm-audio", "CreateObject", &format!("spatial type {kind:#x} method {method:#x}: PC metadata stream activated, but the DCAS metadata bytes per object and the reply qword at p+0x2e4 come from the audio engine, which is in no dump; no guest object; E_NOTIMPL"));
        return Some(E_NOTIMPL);
    }
    let props = if kind == SPATIAL_HRTF_STREAM_TYPE { SPATIAL_HRTF_PROPERTY_BYTES } else { SPATIAL_PROPERTY_BYTES };
    let reply = &lay.reply;
    let rate = wasapi::sample_rate(&params.format);
    let frames = match sp.max_frames(&params.format) {
        Ok(f) => f,
        Err(hr) => return Some(hr),
    };
    let (packet, mix_rate) = match wasapi::device_info_for(dev.as_deref()) {
        Ok(d) => (((d.default_period * rate as i64 / HNS) as u32).min(frames), wasapi::sample_rate(&d.mix)),
        Err(hr) => return Some(hr),
    };
    let frames = engine_frames(frames, rate, mix_rate);
    let statics = (params.mask & !AUDIO_OBJECT_DYNAMIC).count_ones();
    let dynamic = params.max;
    let total = match dcas::total(frames, statics, dynamic, props, 0) {
        Ok(t) => t,
        Err(hr) => return Some(hr),
    };
    let pages = (total as usize).div_ceil(crate::logan::PAGE);
    let Some(index) = crate::logan::pool_alloc(pages) else {
        return Some(E_OUTOFMEMORY);
    };
    let base = crate::logan::pool() + index as usize * crate::logan::PAGE;
    let name = format!("\\BaseNamedObjects\\AudioEngineDuplicateHandleApiPort-{index}");
    let mem = core::slice::from_raw_parts_mut(base as *mut u8, total as usize);
    let shape = match dcas::write(mem, &name, frames, packet, statics, dynamic, props, 0, &params.format).and_then(|sh| dcas::regions(&sh).map(|r| (sh, r))) {
        Ok(v) => v,
        Err(hr) => {
            crate::logan::pool_release(index as usize);
            return Some(hr);
        }
    };
    let event_id = {
        let mut s = state();
        s.next_event += 1;
        s.next_event
    };
    let event_name = event_name(event_id as u16);
    let event = match crate::vab::create_event(&event_name) {
        Ok(h) => h,
        Err(e) => {
            crate::logan::pool_release(index as usize);
            return Some((0x8007_0000 | (e & 0xffff)) as i32);
        }
    };
    let dynamic_now = stream.available_dynamic().unwrap_or(0);
    core::ptr::write_bytes(a.add(reply.extra), 0, spatial_layout_end(&lay) - reply.extra);
    core::ptr::copy_nonoverlapping(guid_bytes(&CLSID_SPATIAL_CLIENT_OUTPUT_ENDPOINT).as_ptr(), a.add(reply.clsid), 16);
    write_event_descriptor(a.add(reply.event), EVENT_KIND_GAMECORE, event_id as u16);
    wr(a, reply.memory, KIND_CROSS_VM_MEMORY);
    core::ptr::copy_nonoverlapping(guid_bytes(&CLSID_XVM_BUFFER).as_ptr(), a.add(reply.memory + 4), 16);
    core::ptr::copy_nonoverlapping(guid_bytes(&GUID_XVM_BUFFER).as_ptr(), a.add(reply.memory + 0x14), 16);
    wr(a, reply.memory + 0x24, index as u32);
    let pump = Arc::new(SpatialPump {
        stream,
        objects: Mutex::new(Vec::new()),
        control: pingpong::Control::at(base as *mut u8),
        base,
        shape: shape.0,
        regions: shape.1,
        mask: params.mask,
        hrtf: kind == SPATIAL_HRTF_STREAM_TYPE,
        event: event as usize,
        running: AtomicBool::new(false),
        quit: AtomicBool::new(false),
        counters: Arc::new(SpatialCounters::default()),
        status: AtomicI32::new(0),
    });
    let worker = pump.clone();
    let thread = match std::thread::Builder::new().name("tombox-spatial".into()).spawn(move || worker.run()) {
        Ok(t) => t,
        Err(_) => {
            crate::logan::pool_release(index as usize);
            return Some(E_OUTOFMEMORY);
        }
    };
    let host = SpatialHost { pool_index: index, started: false, pump, thread: Some(thread) };
    let mut s = state();
    let n = s.next;
    s.next += 1;
    compat_common::record_raw("crossvm-audio", "CreateObject", &format!("spatial stream type {kind:#x} method {method:#x} mask {:#x} min {} max {} category {}: PC stream activated ({dynamic_now} dynamic available), DCAS {name} at pool page {index}: {frames} frames, {packet} per packet, {statics} static + {dynamic} dynamic, {total:#x} bytes, period event {event_name} -> object {n:#x}", params.mask, params.min, params.max, params.category));
    let mut o = Object::new(kind, parent, dev);
    o.spatial = Some(host);
    s.objects.insert(n, o);
    wr(a, 0xc, n);
    Some(S_OK)
}

pub static SPATIAL_ACTIVATIONS: AtomicU32 = AtomicU32::new(0);

pub struct SpatialReply {
    pub extra: usize,
    pub clsid: usize,
    pub endpoint_value: usize,
    pub token: usize,
    pub stream_id: usize,
    pub event: usize,
    pub stream_value: usize,
    pub memory: usize,
}

pub const SPATIAL_DESCRIPTOR_BYTES: usize = 0x124;
pub const SPATIAL_REPLY: SpatialReply = SpatialReply { extra: 0x6c, clsid: 0x70, endpoint_value: 0x80, token: 0x84, stream_id: 0x8c, event: 0x94, stream_value: 0x1b8, memory: 0x1bc };
pub const SPATIAL_REPLY2: SpatialReply = SpatialReply { extra: 0x6c, clsid: 0x74, endpoint_value: 0x84, token: 0x88, stream_id: 0x90, event: 0x98, stream_value: 0x1bc, memory: 0x1c0 };
pub const SPATIAL_PHASE_THREE: u32 = 0x39;
pub const SPATIAL_PRE_START: u32 = 0x44;
pub const SPATIAL_PRE_STOP: u32 = 0x45;
pub const SPATIAL_RESET: u32 = 0x46;
pub const RECORD_COOKIE: usize = 0;
pub const RECORD_FRAMES: usize = 4;
pub const RECORD_TYPE: usize = 8;
pub const RECORD_FLAGS: usize = 0xc;
pub const RECORD_VOLUME: usize = 0x1c;
pub const RECORD_BUFFER: u32 = 1;
pub const RECORD_ACTIVE: u32 = 2;
pub const RECORD_END_OF_STREAM: u32 = 8;
pub const AUDIO_OBJECT_DYNAMIC: u32 = 1;
pub const SPATIAL_HRTF_PROPERTY_BYTES: u32 = 0xb4;
pub const RECORD_HRTF: u32 = 0x10;
pub const HRTF_DIRTY: usize = 0x20;
pub const HRTF_GAIN: usize = 0x24;
pub const HRTF_ORIENTATION: usize = 0x28;
pub const HRTF_ENVIRONMENT: usize = 0x4c;
pub const HRTF_DECAY: usize = 0x50;
pub const HRTF_DIRECTIVITY: usize = 0x64;
pub const HRTF_UNKNOWN: usize = 0x74;
pub const HRTF_DIRTY_GAIN: u32 = 1;
pub const HRTF_DIRTY_ORIENTATION: u32 = 2;
pub const HRTF_DIRTY_ENVIRONMENT: u32 = 4;
pub const HRTF_DIRTY_DECAY: u32 = 8;
pub const HRTF_DIRTY_DIRECTIVITY: u32 = 0x10;
pub const HRTF_DIRTY_UNKNOWN: u32 = 0x20;
const SPTLAUDCLNT_E_STREAM_NOT_STOPPED: i32 = 0x8889_010au32 as i32;
pub const SPTLAUDCLNT_E_DESTROYED: i32 = 0x8889_0100u32 as i32;
pub const AUDCLNT_E_DEVICE_INVALIDATED: i32 = 0x8889_0004u32 as i32;
pub const AUDCLNT_E_RESOURCES_INVALIDATED: i32 = 0x8889_0026u32 as i32;
pub const ENGINE_HEADROOM: f32 = 1.1;
pub const SRC_PRIMING_FRAMES: u32 = 7;

pub fn engine_frames(max: u32, stream_rate: u32, mix_rate: u32) -> u32 {
    let n = max + if stream_rate != mix_rate { SRC_PRIMING_FRAMES } else { 0 };
    (n as f32 * ENGINE_HEADROOM) as u32
}

pub fn stream_status_error(hr: i32) -> bool {
    matches!(hr, SPTLAUDCLNT_E_DESTROYED | AUDCLNT_E_DEVICE_INVALIDATED | AUDCLNT_E_RESOURCES_INVALIDATED)
}

#[derive(Default)]
pub struct SpatialCounters {
    pub passes: AtomicI64,
    pub reads: AtomicI64,
    pub starved: AtomicI64,
    pub faults: AtomicI64,
    pub buffers: AtomicI64,
    pub silent: AtomicI64,
    pub end_of_stream: AtomicI64,
    pub volumes: AtomicI64,
    pub dynamic_skipped: AtomicI64,
    pub hrtf_properties: AtomicI64,
    pub hrtf_unknown: AtomicI64,
    pub errors: AtomicI64,
    pub last_error: AtomicU32,
}

struct SpatialPump {
    stream: wasapi::SpatialStream,
    objects: Mutex<Vec<(u32, wasapi::SpatialObject, f32)>>,
    control: pingpong::Control,
    base: usize,
    shape: dcas::Shape,
    regions: dcas::Regions,
    mask: u32,
    hrtf: bool,
    event: usize,
    running: AtomicBool,
    quit: AtomicBool,
    counters: Arc<SpatialCounters>,
    status: AtomicI32,
}

unsafe impl Send for SpatialPump {}
unsafe impl Sync for SpatialPump {}

struct SpatialHost {
    pool_index: u16,
    started: bool,
    pump: Arc<SpatialPump>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Drop for SpatialHost {
    fn drop(&mut self) {
        self.pump.running.store(false, Ordering::SeqCst);
        self.pump.quit.store(true, Ordering::SeqCst);
        wasapi::signal_event(self.pump.stream.event());
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        let _ = self.pump.stream.stop();
        self.pump.objects.lock().unwrap_or_else(|e| e.into_inner()).clear();
        crate::logan::pool_release(self.pool_index as usize);
    }
}

impl SpatialPump {
    fn object(&self, list: &mut Vec<(u32, wasapi::SpatialObject, f32)>, kind: u32) -> Option<usize> {
        if let Some(i) = list.iter().position(|o| o.0 == kind) {
            return Some(i);
        }
        if kind == AUDIO_OBJECT_DYNAMIC || kind & self.mask != kind || kind.count_ones() != 1 {
            self.counters.dynamic_skipped.fetch_add(1, Ordering::SeqCst);
            return None;
        }
        match self.stream.activate_object(kind) {
            Ok(o) => {
                list.push((kind, o, 1.0));
                Some(list.len() - 1)
            }
            Err(hr) => {
                self.fail(hr);
                None
            }
        }
    }

    fn fail(&self, hr: i32) {
        self.counters.errors.fetch_add(1, Ordering::SeqCst);
        self.counters.last_error.store(hr as u32, Ordering::SeqCst);
        if stream_status_error(hr) {
            let _ = self.status.compare_exchange(0, hr, Ordering::SeqCst, Ordering::SeqCst);
        }
    }

    fn status(&self) -> i32 {
        self.status.load(Ordering::SeqCst)
    }

    unsafe fn pass(&self) {
        let (_, frames) = match self.stream.begin_updating() {
            Ok(v) => v,
            Err(hr) => {
                self.fail(hr);
                return;
            }
        };
        self.counters.passes.fetch_add(1, Ordering::SeqCst);
        self.control.set_frames_per_packet(frames);
        match self.control.read_acquire() {
            Ok(index) => {
                self.counters.reads.fetch_add(1, Ordering::SeqCst);
                self.read(index, frames);
                if self.control.read_commit(index) < 0 {
                    self.counters.faults.fetch_add(1, Ordering::SeqCst);
                }
            }
            Err(pingpong::E_READ_STARVED) => {
                self.counters.starved.fetch_add(1, Ordering::SeqCst);
            }
            Err(_) => {
                self.counters.faults.fetch_add(1, Ordering::SeqCst);
            }
        }
        let hr = self.stream.end_updating();
        if hr < 0 {
            self.fail(hr);
        }
        wasapi::signal_event(self.event);
    }

    unsafe fn read(&self, index: u32, frames: u32) {
        let data = self.base + dcas::DATA;
        let props = self.shape.properties as usize;
        let frame_bytes = self.shape.frame_bytes as usize;
        let mut list = self.objects.lock().unwrap_or_else(|e| e.into_inner());
        for k in 0..self.shape.objects as usize {
            let r = (data + self.regions.properties[index as usize] + k * props) as *const u8;
            let kind: u32 = rd(r, RECORD_TYPE);
            let flags: u32 = rd(r, RECORD_FLAGS);
            if kind == 0 || flags & (RECORD_BUFFER | RECORD_ACTIVE | RECORD_END_OF_STREAM | RECORD_HRTF) == 0 {
                continue;
            }
            let Some(i) = self.object(&mut list, kind) else { continue };
            let count: u32 = rd(r, RECORD_FRAMES);
            let volume: f32 = rd(r, RECORD_VOLUME);
            if self.hrtf && flags & RECORD_HRTF != 0 {
                self.hrtf_properties(&list[i].1, r);
            }
            if !self.hrtf && volume != list[i].2 {
                let hr = list[i].1.set_volume(volume);
                if hr < 0 {
                    self.fail(hr);
                } else {
                    list[i].2 = volume;
                    self.counters.volumes.fetch_add(1, Ordering::SeqCst);
                }
            }
            if flags & RECORD_BUFFER != 0 {
                match list[i].1.buffer() {
                    Ok((dst, len)) => {
                        let n = (len as usize).min(count.min(frames) as usize * 4).min(frame_bytes);
                        if wasapi::muted() {
                            core::ptr::write_bytes(dst, 0, len as usize);
                            self.counters.silent.fetch_add(1, Ordering::SeqCst);
                        } else {
                            let src = (data + self.regions.samples[index as usize] + k * frame_bytes) as *const u8;
                            core::ptr::copy_nonoverlapping(src, dst, n);
                            core::ptr::write_bytes(dst.add(n), 0, len as usize - n);
                        }
                        self.counters.buffers.fetch_add(1, Ordering::SeqCst);
                    }
                    Err(hr) => self.fail(hr),
                }
            }
            if flags & RECORD_END_OF_STREAM != 0 {
                let hr = list[i].1.set_end_of_stream(if flags & RECORD_BUFFER != 0 { count.min(frames) } else { 0 });
                if hr < 0 {
                    self.fail(hr);
                }
                self.counters.end_of_stream.fetch_add(1, Ordering::SeqCst);
                list.remove(i);
            }
        }
    }

    unsafe fn hrtf_properties(&self, o: &wasapi::SpatialObject, r: *const u8) {
        let dirty: u32 = rd(r, HRTF_DIRTY);
        let mut hrs = Vec::new();
        if dirty & HRTF_DIRTY_GAIN != 0 {
            hrs.push(o.set_hrtf_gain(rd(r, HRTF_GAIN)));
        }
        if dirty & HRTF_DIRTY_ORIENTATION != 0 {
            hrs.push(o.set_hrtf_block(wasapi::HRTF_SET_ORIENTATION, r.add(HRTF_ORIENTATION)));
        }
        if dirty & HRTF_DIRTY_ENVIRONMENT != 0 {
            hrs.push(o.set_hrtf_environment(rd(r, HRTF_ENVIRONMENT)));
        }
        if dirty & HRTF_DIRTY_DECAY != 0 {
            hrs.push(o.set_hrtf_block(wasapi::HRTF_SET_DISTANCE_DECAY, r.add(HRTF_DECAY)));
        }
        if dirty & HRTF_DIRTY_DIRECTIVITY != 0 {
            hrs.push(o.set_hrtf_block(wasapi::HRTF_SET_DIRECTIVITY, r.add(HRTF_DIRECTIVITY)));
        }
        if dirty & HRTF_DIRTY_UNKNOWN != 0 {
            self.counters.hrtf_unknown.fetch_add(1, Ordering::SeqCst);
        }
        for hr in hrs {
            if hr < 0 {
                self.fail(hr);
            } else {
                self.counters.hrtf_properties.fetch_add(1, Ordering::SeqCst);
            }
        }
    }

    fn run(self: Arc<Self>) {
        unsafe { wasapi::com_init() };
        while !self.quit.load(Ordering::SeqCst) {
            if !self.running.load(Ordering::SeqCst) {
                wasapi::wait_event(self.stream.event(), 20);
                continue;
            }
            if wasapi::wait_event(self.stream.event(), 50) && self.running.load(Ordering::SeqCst) && !self.quit.load(Ordering::SeqCst) {
                unsafe { self.pass() };
            }
        }
    }

    fn prepare(&self) -> i32 {
        let mut list = self.objects.lock().unwrap_or_else(|e| e.into_inner());
        let mut bit = 2u32;
        while bit != 0 {
            if self.mask & bit != 0 && !list.iter().any(|o| o.0 == bit) {
                match self.stream.activate_object(bit) {
                    Ok(o) => list.push((bit, o, 1.0)),
                    Err(hr) => return hr,
                }
            }
            bit = bit.wrapping_shl(1);
        }
        S_OK
    }
}

pub fn spatial_counters(object: u32) -> Option<Arc<SpatialCounters>> {
    state().objects.get(&object)?.spatial.as_ref().map(|h| h.pump.counters.clone())
}

pub fn spatial_control(object: u32) -> Option<(usize, u16, dcas::Shape, dcas::Regions)> {
    let s = state();
    let h = s.objects.get(&object)?.spatial.as_ref()?;
    Some((h.pump.base, h.pool_index, h.pump.shape, h.pump.regions))
}

pub fn spatial_static_objects(object: u32) -> Option<Vec<u32>> {
    let s = state();
    let h = s.objects.get(&object)?.spatial.as_ref()?;
    let list = h.pump.objects.lock().unwrap_or_else(|e| e.into_inner());
    Some(list.iter().map(|o| o.0).collect())
}

unsafe fn spatial_call(id: u32, object: u32, a: *mut u8) -> Option<i32> {
    let mut s = state();
    let h = s.objects.get_mut(&object)?.spatial.as_mut()?;
    let pump = h.pump.clone();
    Some(match id {
        SPATIAL_PHASE_THREE => {
            drop(s);
            pump.prepare()
        }
        SPATIAL_PRE_START => {
            wr(a, 0, 0u8);
            if pump.status() < 0 {
                return Some(pump.status());
            }
            if h.started {
                return Some(SPTLAUDCLNT_E_STREAM_NOT_STOPPED);
            }
            h.started = true;
            if !pump.running.load(Ordering::SeqCst) {
                wr(a, 0, 1u8);
            }
            S_OK
        }
        START => {
            drop(s);
            let hr = pump.stream.start();
            if hr < 0 {
                pump.fail(hr);
                return Some(hr);
            }
            pump.running.store(true, Ordering::SeqCst);
            wasapi::signal_event(pump.event);
            S_OK
        }
        SPATIAL_PRE_STOP => {
            if !h.started {
                return Some(S_FALSE);
            }
            h.started = false;
            S_OK
        }
        STOP => {
            drop(s);
            let hr = pump.stream.stop();
            if hr < 0 {
                pump.fail(hr);
                return Some(hr);
            }
            if hr != S_FALSE {
                pump.running.store(false, Ordering::SeqCst);
            }
            hr
        }
        SPATIAL_RESET => {
            if pump.status() < 0 {
                pump.status()
            } else if h.started {
                SPTLAUDCLNT_E_STREAM_NOT_STOPPED
            } else {
                S_OK
            }
        }

        _ => return None,
    })
}


pub mod dcas {
    pub const HEADER: u32 = 0x326;
    pub const MAGIC: u32 = 0x5341_4344;
    pub const DATA: usize = 0x340;
    pub const DATA_HEADER: u32 = 0x1c;
    pub const DATA_MAGIC: u32 = 0x5341_4244;
    pub const NAME: usize = 0x184;
    pub const NAME_CHARS: usize = 0x40;
    pub const FRAMES: usize = 0x204;
    pub const STATIC_OBJECTS: usize = 0x208;
    pub const DYNAMIC_OBJECTS: usize = 0x20c;
    pub const TOTAL: usize = 0x210;
    pub const FORMAT: usize = 0x214;
    pub const FORMAT_EXTRA_MAX: usize = 0x100;
    pub const STATE: usize = 0x30;
    pub const STATE_INITIAL: u32 = 6;
    pub const E_INVALID_CONTROL: i32 = 0x887c_0045u32 as i32;
    pub const E_MISALIGNED: i32 = 0x887c_0054u32 as i32;
    pub const E_OVERFLOW: i32 = 0x8007_0216u32 as i32;
    const E_INVALIDARG: i32 = 0x8007_0057u32 as i32;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct Shape {
        pub objects: u32,
        pub properties: u32,
        pub metadata: u32,
        pub frame_bytes: u32,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct Regions {
        pub properties: [usize; 2],
        pub metadata: [usize; 2],
        pub samples: [usize; 2],
    }

    pub fn align32(v: u32) -> Result<u32, i32> {
        if v & 0x1f == 0 {
            return Ok(v);
        }
        let r = (v - (v & 0x1f)).wrapping_add(0x20);
        if r < v { Err(E_OVERFLOW) } else { Ok(r) }
    }

    pub fn data_size(sample_bytes: u32, properties: u32, metadata: u32, objects: u32) -> Result<u32, i32> {
        let frame = align32(sample_bytes)? as u64;
        let n = objects as u64;
        let p = properties as u64 * n * 2;
        let m = metadata as u64 * n * 2;
        let s = frame * n * 2;
        if p > u32::MAX as u64 || m > u32::MAX as u64 || s > u32::MAX as u64 {
            return Err(E_OVERFLOW);
        }
        let head = align32(u32::try_from(DATA_HEADER as u64 + p + m).map_err(|_| E_OVERFLOW)?)? as u64;
        u32::try_from(head + s).map_err(|_| E_OVERFLOW)
    }

    pub fn regions(shape: &Shape) -> Result<Regions, i32> {
        let n = shape.objects as usize;
        let p0 = DATA_HEADER as usize;
        let p1 = p0 + shape.properties as usize * n;
        let m0 = p1 + shape.properties as usize * n;
        let m1 = m0 + shape.metadata as usize * n;
        let end = m1 + shape.metadata as usize * n;
        let s0 = align32(u32::try_from(end).map_err(|_| E_OVERFLOW)?)? as usize;
        let s1 = s0 + shape.frame_bytes as usize * n;
        Ok(Regions { properties: [p0, p1], metadata: [m0, m1], samples: [s0, s1] })
    }

    pub fn total(frames: u32, static_objects: u32, dynamic_objects: u32, properties: u32, metadata: u32) -> Result<u32, i32> {
        let objects = static_objects.checked_add(dynamic_objects).ok_or(E_OVERFLOW)?;
        let bytes = frames.checked_mul(4).ok_or(E_OVERFLOW)?;
        data_size(bytes, properties, metadata, objects)?.checked_add(DATA as u32).ok_or(E_OVERFLOW)
    }

    fn put(b: &mut [u8], o: usize, v: u32) {
        b[o..o + 4].copy_from_slice(&v.to_le_bytes());
    }

    fn get(b: &[u8], o: usize) -> u32 {
        u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn write(b: &mut [u8], name: &str, frames: u32, packet: u32, static_objects: u32, dynamic_objects: u32, properties: u32, metadata: u32, format: &[u8]) -> Result<Shape, i32> {
        let total = total(frames, static_objects, dynamic_objects, properties, metadata)?;
        if format.len() < 0x12 {
            return Err(E_INVALIDARG);
        }
        let extra = u16::from_le_bytes([format[0x10], format[0x11]]) as usize;
        if extra > FORMAT_EXTRA_MAX || format.len() < 0x12 + extra {
            return Err(E_INVALIDARG);
        }
        if b.len() < total as usize {
            return Err(0x8007_000Eu32 as i32);
        }
        if (b.as_ptr() as usize + DATA) & 0x1f != 0 {
            return Err(E_MISALIGNED);
        }
        b[..DATA].fill(0);
        put(b, 0, HEADER);
        put(b, 4, MAGIC);
        put(b, STATE, STATE_INITIAL);
        put(b, super::pingpong::FRAMES_PER_PACKET, packet);
        put(b, super::pingpong::FLAGS, super::pingpong::FLAG_VALID);
        let w: Vec<u16> = name.encode_utf16().take(NAME_CHARS - 1).collect();
        for (i, c) in w.iter().enumerate() {
            b[NAME + i * 2..NAME + i * 2 + 2].copy_from_slice(&c.to_le_bytes());
        }
        put(b, FRAMES, frames);
        put(b, STATIC_OBJECTS, static_objects);
        put(b, DYNAMIC_OBJECTS, dynamic_objects);
        put(b, TOTAL, total);
        b[FORMAT..FORMAT + 0x12 + extra].copy_from_slice(&format[..0x12 + extra]);
        let objects = static_objects + dynamic_objects;
        let shape = Shape { objects, properties, metadata, frame_bytes: align32(frames * 4)? };
        let d = &mut b[DATA..total as usize];
        d.fill(0);
        put(d, 0, DATA_HEADER);
        put(d, 4, total - DATA as u32);
        put(d, 8, objects);
        put(d, 0xc, properties);
        put(d, 0x10, metadata);
        put(d, 0x14, shape.frame_bytes);
        put(d, 0x18, DATA_MAGIC);
        Ok(shape)
    }

    pub fn validate(b: &[u8]) -> i32 {
        if b.len() < DATA {
            return E_INVALID_CONTROL;
        }
        if get(b, 0) != HEADER || get(b, 4) != MAGIC || get(b, STATIC_OBJECTS).wrapping_add(get(b, DYNAMIC_OBJECTS)) == 0 || (get(b, TOTAL) as usize) < DATA {
            return E_INVALID_CONTROL;
        }
        0
    }

    pub fn parse(b: &[u8]) -> Result<(Shape, Regions), i32> {
        let hr = validate(b);
        if hr < 0 {
            return Err(hr);
        }
        let total = get(b, TOTAL) as usize;
        if b.len() < total {
            return Err(E_INVALID_CONTROL);
        }
        let d = &b[DATA..total];
        let size = d.len() as u32;
        if size <= DATA_HEADER {
            return Err(E_INVALID_CONTROL);
        }
        let shape = Shape { objects: get(d, 8), properties: get(d, 0xc), metadata: get(d, 0x10), frame_bytes: get(d, 0x14) };
        if data_size(shape.frame_bytes, shape.properties, shape.metadata, shape.objects)? != size {
            return Err(E_INVALID_CONTROL);
        }
        Ok((shape, regions(&shape)?))
    }
}

pub mod pingpong {
    use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

    pub const STATES: usize = 17;
    pub const FAULTED: u32 = 16;
    pub const WRITE_ACQUIRE: [u32; STATES] = [1, 16, 2, 4, 16, 5, 7, 16, 9, 16, 10, 12, 16, 13, 15, 16, 16];
    pub const WRITE_COMMIT: [u32; STATES] = [16, 2, 16, 16, 5, 16, 16, 8, 16, 10, 16, 16, 13, 16, 16, 0, 16];
    pub const WRITE_CANCEL: [u32; STATES] = [0, 0, 2, 3, 3, 5, 6, 6, 8, 8, 10, 11, 11, 13, 14, 14, 16];
    pub const READ_ACQUIRE: [u32; STATES] = [3, 4, 5, 16, 16, 16, 6, 7, 11, 12, 13, 16, 16, 16, 14, 15, 16];
    pub const READ_COMMIT: [u32; STATES] = [16, 16, 16, 6, 7, 8, 16, 16, 16, 16, 16, 14, 15, 0, 16, 16, 16];
    pub const FLUSH: [u32; STATES] = [6, 7, 6, 3, 4, 3, 6, 7, 14, 15, 14, 11, 12, 11, 14, 15, 6];
    pub const READER_INDEX: [u32; STATES] = [1, 1, 1, 1, 1, 1, 2, 2, 0, 0, 0, 0, 0, 0, 2, 2, 3];
    pub const WRITER_INDEX: [u32; STATES] = [0, 0, 2, 0, 0, 2, 0, 0, 1, 1, 2, 1, 1, 2, 1, 1, 3];
    pub const INDEX_STARVED: u32 = 2;
    pub const INDEX_FAULT: u32 = 3;

    pub const POSITION: usize = 0x08;
    pub const GRANT_POSITION: usize = 0x10;
    pub const DYNAMIC_REQUESTED: usize = 0x18;
    pub const DYNAMIC_GRANTED: usize = 0x1c;
    pub const BUFFER_OBJECTS: usize = 0x20;
    pub const FRAMES_COMMITTED: usize = 0x28;
    pub const STATE: usize = 0x30;
    pub const FRAMES_PER_PACKET: usize = 0x34;
    pub const BUFFER_FRAMES: usize = 0x38;
    pub const DYNAMIC_IN_USE: usize = 0x40;
    pub const FLAGS: usize = 0x48;
    pub const READ_STARVED: usize = 0x4c;
    pub const WRITE_STARVED: usize = 0x50;

    pub const FLAG_VALID: u32 = 1;
    pub const FLAG_STARTED: u32 = 2;
    pub const FLAG_POSITION: u32 = 4;
    pub const FLAG_POSITION_LATCHED: u32 = 8;
    pub const FLAG_DISCONTINUITY: u32 = 0x10;
    pub const FLAG_EVENT_SET: u32 = 0x20;
    pub const FLAG_GRANT_PENDING: u32 = 0x40;
    pub const FLAG_CLOSED: u32 = 0x80;

    pub const E_INVALID_CONTROL: i32 = 0x887c_0045u32 as i32;
    pub const E_WRITE_STARVED: i32 = 0x887c_0061u32 as i32;
    pub const E_READ_STARVED: i32 = 0x887c_0062u32 as i32;
    pub const E_BAD_WRITE_INDEX: i32 = 0x887c_006fu32 as i32;
    pub const E_NOT_STARTED: i32 = 0x887c_0013u32 as i32;
    const E_INVALIDARG: i32 = 0x8007_0057u32 as i32;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct Control {
        base: usize,
    }

    impl Control {
        pub unsafe fn at(base: *mut u8) -> Self {
            Control { base: base as usize }
        }

        fn u32(&self, o: usize) -> &AtomicU32 {
            unsafe { &*((self.base + o) as *const AtomicU32) }
        }

        fn u64(&self, o: usize) -> &AtomicU64 {
            unsafe { &*((self.base + o) as *const AtomicU64) }
        }

        pub fn state(&self) -> u32 {
            let s = self.u32(STATE).load(Ordering::SeqCst);
            if s > FAULTED {
                self.u32(STATE).swap(FAULTED, Ordering::SeqCst);
                return FAULTED;
            }
            s
        }

        fn step(&self, table: &[u32; STATES]) -> u32 {
            let mut s = self.state();
            loop {
                let next = table[s as usize];
                match self.u32(STATE).compare_exchange(s, next, Ordering::SeqCst, Ordering::SeqCst) {
                    Ok(_) => return next,
                    Err(cur) if cur > FAULTED => return FAULTED,
                    Err(cur) => s = cur,
                }
            }
        }

        pub fn flush(&self) {
            self.u32(FLAGS).fetch_or(FLAG_DISCONTINUITY, Ordering::SeqCst);
            self.u64(FRAMES_COMMITTED).store(0, Ordering::SeqCst);
            self.step(&FLUSH);
        }

        pub fn write_acquire(&self) -> Result<u32, i32> {
            let next = self.step(&WRITE_ACQUIRE);
            match WRITER_INDEX[next as usize] {
                INDEX_FAULT => {
                    self.step(&FLUSH);
                    Err(E_INVALID_CONTROL)
                }
                INDEX_STARVED => {
                    self.u32(WRITE_STARVED).fetch_add(1, Ordering::SeqCst);
                    Err(E_WRITE_STARVED)
                }
                i => {
                    let packet = self.u32(FRAMES_PER_PACKET).load(Ordering::SeqCst);
                    self.u32(BUFFER_FRAMES + i as usize * 4).store(packet, Ordering::SeqCst);
                    Ok(i)
                }
            }
        }

        pub fn write_commit(&self, index: u32, objects: u32, packet_frames: u32) -> i32 {
            if index > 1 {
                return E_BAD_WRITE_INDEX;
            }
            self.u32(BUFFER_OBJECTS + index as usize * 4).swap(objects, Ordering::SeqCst);
            let next = self.step(&WRITE_COMMIT);
            if WRITER_INDEX[next as usize] == INDEX_FAULT {
                return E_INVALID_CONTROL;
            }
            self.u64(FRAMES_COMMITTED).fetch_add(if objects != 0 { packet_frames as u64 } else { 0 }, Ordering::SeqCst);
            0
        }

        pub fn write_cancel(&self) {
            self.step(&WRITE_CANCEL);
        }

        pub fn read_acquire(&self) -> Result<u32, i32> {
            let next = self.step(&READ_ACQUIRE);
            match READER_INDEX[next as usize] {
                INDEX_FAULT => {
                    self.step(&FLUSH);
                    Err(E_INVALID_CONTROL)
                }
                INDEX_STARVED => {
                    self.u32(READ_STARVED).fetch_add(1, Ordering::SeqCst);
                    Err(E_READ_STARVED)
                }
                i => Ok(i),
            }
        }

        pub fn read_commit(&self, index: u32) -> i32 {
            if index > 1 {
                return E_INVALIDARG;
            }
            let next = self.step(&READ_COMMIT);
            if READER_INDEX[next as usize] == INDEX_FAULT {
                return E_INVALID_CONTROL;
            }
            0
        }

        pub fn frames_per_packet(&self, max: u32) -> u32 {
            let v = self.u32(FRAMES_PER_PACKET).load(Ordering::SeqCst);
            let m = v.min(max);
            if m == 0 { max } else { m }
        }

        pub fn set_frames_per_packet(&self, frames: u32) {
            self.u32(FRAMES_PER_PACKET).store(frames, Ordering::SeqCst);
        }

        pub fn buffer_objects(&self, index: u32) -> u32 {
            self.u32(BUFFER_OBJECTS + index as usize * 4).load(Ordering::SeqCst)
        }

        pub fn buffer_frames(&self, index: u32) -> u32 {
            self.u32(BUFFER_FRAMES + index as usize * 4).load(Ordering::SeqCst)
        }

        pub fn frames_committed(&self) -> u64 {
            self.u64(FRAMES_COMMITTED).load(Ordering::SeqCst)
        }

        pub fn flags(&self) -> u32 {
            self.u32(FLAGS).load(Ordering::SeqCst)
        }

        pub fn set_flags(&self, set: u32, clear: u32) {
            self.u32(FLAGS).fetch_and(!clear, Ordering::SeqCst);
            self.u32(FLAGS).fetch_or(set, Ordering::SeqCst);
        }

        pub fn counters(&self) -> (u32, u32) {
            (self.u32(READ_STARVED).load(Ordering::SeqCst), self.u32(WRITE_STARVED).load(Ordering::SeqCst))
        }

        pub fn take_counters(&self) -> (u32, u32) {
            (self.u32(READ_STARVED).swap(0, Ordering::SeqCst), self.u32(WRITE_STARVED).swap(0, Ordering::SeqCst))
        }

        pub fn transport(&self, start: bool) -> i32 {
            if self.flags() & (FLAG_VALID | FLAG_CLOSED) != FLAG_VALID {
                return E_INVALID_CONTROL;
            }
            if start {
                self.set_flags(FLAG_STARTED, 0);
            } else {
                self.set_flags(0, FLAG_STARTED | FLAG_POSITION_LATCHED);
            }
            0
        }

        pub fn reset(&self) -> i32 {
            if self.flags() & (FLAG_VALID | FLAG_CLOSED) != FLAG_VALID {
                return E_INVALID_CONTROL;
            }
            self.flush();
            0
        }
    }
}

unsafe fn is_format_supported(object: u32, a: *mut u8) -> i32 {
    let want: u8 = rd(a, 0);
    let mode: u32 = rd(a, 2);
    let f = format_at(a.add(6));
    if f.len() < 0x12 || mode > 1 {
        return E_INVALIDARG;
    }
    let (hr, near) = wasapi::is_format_supported_on(device_for(object).as_deref(), mode, &f);
    wr(a, 1, 0u8);
    if let (Some(c), true) = (near, want != 0) {
        core::ptr::write_bytes(a.add(0x4e), 0, wasapi::MAX_FORMAT);
        core::ptr::copy_nonoverlapping(c.as_ptr(), a.add(0x4e), c.len());
        wr(a, 1, 1u8);
    }
    hr
}

unsafe fn transport(id: u32, object: u32) -> i32 {
    let s = state();
    let Some(st) = stream_owner(&s, object).and_then(|k| s.objects.get(&k)).and_then(|o| o.stream.as_ref()) else {
        return AUDCLNT_E_NOT_INITIALIZED;
    };
    let sh = &st.renderer.shared;
    let running = sh.running.load(Ordering::SeqCst);
    match id {
        START => {
            if running {
                return AUDCLNT_E_NOT_STOPPED;
            }
            let mut sig = sh.signal.lock().unwrap();
            sig.clear();
            sig.push(st.event);
            sig.extend(crate::vab::sra_events());
            drop(sig);
            sh.running.store(true, Ordering::SeqCst);
            S_OK
        }
        STOP => {
            if !running {
                return S_FALSE;
            }
            sh.running.store(false, Ordering::SeqCst);
            S_OK
        }
        _ => {
            if running {
                return AUDCLNT_E_NOT_STOPPED;
            }
            sh.reset.store(true, Ordering::SeqCst);
            S_OK
        }
    }
}

fn simple<R>(object: u32, f: impl FnOnce(&mut f32, &mut bool) -> R) -> Option<R> {
    let mut s = state();
    if !s.objects.contains_key(&object) {
        return None;
    }
    let (mut g, mut m) = s.session_simple.unwrap_or((1.0, false));
    let r = f(&mut g, &mut m);
    s.session_simple = Some((g, m));
    apply_all(&mut s);
    Some(r)
}

unsafe fn stream_volume(id: u32, object: u32, a: *mut u8) -> i32 {
    let mut s = state();
    let Some(k) = stream_owner(&s, object) else { return AUDCLNT_E_NOT_INITIALIZED };
    let o = s.objects.get_mut(&k).unwrap();
    let n = wasapi::channels(&o.stream.as_ref().unwrap().format) as usize;
    let v = o.volumes.get_or_insert_with(|| vec![1.0; n]);
    let r = channel_op(id - STREAM_GET_CHANNEL_COUNT, v, a);
    apply_all(&mut s);
    r
}

unsafe fn channel_op(op: u32, v: &mut Vec<f32>, a: *mut u8) -> i32 {
    let n = v.len();
    match op {
        0 => {
            wr(a, 0, n as u32);
            S_OK
        }
        1 => {
            let i: u32 = rd(a, 0);
            let x: f32 = rd(a, 4);
            if i as usize >= n || !(0.0..=1.0).contains(&x) {
                return E_INVALIDARG;
            }
            v[i as usize] = x;
            S_OK
        }
        2 => {
            let i: u32 = rd(a, 0);
            if i as usize >= n {
                return E_INVALIDARG;
            }
            wr(a, 0, v[i as usize]);
            S_OK
        }
        3 => {
            let c: u32 = rd(a, 0);
            if c as usize != n || n > MAX_VOLUMES {
                return E_INVALIDARG;
            }
            let mut nv = Vec::with_capacity(n);
            for i in 0..n {
                let x: f32 = rd(a, 4 + 4 * i);
                if !(0.0..=1.0).contains(&x) {
                    return E_INVALIDARG;
                }
                nv.push(x);
            }
            *v = nv;
            S_OK
        }
        _ => {
            let c: u32 = rd(a, 0);
            if c as usize != n || n > MAX_VOLUMES {
                return E_INVALIDARG;
            }
            wr(a, 0, n as u32);
            for (i, x) in v.iter().enumerate() {
                wr(a, 4 + 4 * i, *x);
            }
            S_OK
        }
    }
}

unsafe fn session_volume(id: u32, object: u32, a: *mut u8) -> i32 {
    if let r @ wasapi::SessionRef::Pc(_) = session_ref(object) {
        let mut v = match wasapi::session_channels_get(&r) {
            Ok(v) => v,
            Err(hr) => return hr,
        };
        let hr = channel_op(id - SESSION_CHANNEL_COUNT, &mut v, a);
        return match id {
            SESSION_SET_CHANNEL_VOLUME if hr >= 0 => wasapi::session_channels_set(&r, &v, &guid_from(&ctx_at(a, 8))),
            SESSION_SET_ALL_VOLUMES if hr >= 0 => wasapi::session_channels_set(&r, &v, &guid_from(&ctx_at(a, SESSION_ALL_VOLUMES_CTX))),
            _ => hr,
        };
    }
    let dev = device_for(object);
    let n = wasapi::device_info_for(dev.as_deref()).map(|d| wasapi::channels(&d.mix) as usize).unwrap_or(2);
    let mut s = state();
    let mut v = s.session_channels.clone().unwrap_or_else(|| vec![1.0; n]);
    let r = channel_op(id - SESSION_CHANNEL_COUNT, &mut v, a);
    if r >= 0 && (id == SESSION_SET_CHANNEL_VOLUME || id == SESSION_SET_ALL_VOLUMES) {
        s.session_channels = Some(v.clone());
        apply_all(&mut s);
        drop(s);
        let (changed, ctx) = if id == SESSION_SET_CHANNEL_VOLUME { (rd::<u32>(a, 0), ctx_at(a, 8)) } else { (u32::MAX, ctx_at(a, SESSION_ALL_VOLUMES_CTX)) };
        session_changed(|sid| session_channel_record(sid, &ctx, changed, &v));
    }
    r
}
fn subscribe(object: u32, id: u32, on: bool) -> i32 {
    if !state().objects.contains_key(&object) {
        return E_INVALIDARG;
    }
    let mut watch = None;
    if on {
        wasapi::set_event_sink(pc_event);
        let hr = match id {
            ENUMERATOR_SUBSCRIBE => wasapi::watch_devices(),
            ENDPOINT_REGISTER => {
                watch = Some(endpoint_key(&device_for(object)));
                S_OK
            }
            SESSION_REGISTER_NOTIFICATION | SESSION_MANAGER_REGISTER => match wasapi::watch_session(&session_ref(object)) {
                Ok(sid) => {
                    watch = Some(sid);
                    S_OK
                }
                Err(hr) => hr,
            },
            _ => S_OK,
        };
        if hr < 0 {
            return hr;
        }
    }
    let mut s = state();
    match s.objects.get_mut(&object) {
        Some(o) => {
            o.notify = on;
            o.notify_id = if on { id } else { 0 };
            o.watch = watch;
            compat_common::record_raw("crossvm-audio", "notify", &format!("object {object:#x} notifications {} by {id:#x}", if on { "registered" } else { "unregistered" }));
            S_OK
        }
        None => E_INVALIDARG,
    }
}

pub const NOTIFY_RECORD: usize = 0xc;
pub const DEVICE_RECORD_ID: usize = 0x34;
pub const DEVICE_RECORD_MAX_CHARS: usize = (gchost_frame() - NOTIFY_RECORD - DEVICE_RECORD_ID) / 2;
pub const VOLUME_RECORD_CHANNELS: usize = 0x38;

const fn gchost_frame() -> usize {
    crate::gchost::AUDIO_FRAME
}

pub fn device_record(kind: u32, id: &str, a: u32, b: u32, key: Option<&wasapi::PropertyKey>) -> Option<Vec<u8>> {
    let w: Vec<u16> = id.encode_utf16().chain(Some(0)).collect();
    if w.len() > DEVICE_RECORD_MAX_CHARS {
        return None;
    }
    let mut r = vec![0u8; DEVICE_RECORD_ID + w.len() * 2];
    let n = r.len() as u32;
    r[0..4].copy_from_slice(&n.to_le_bytes());
    r[0x18..0x1c].copy_from_slice(&kind.to_le_bytes());
    match kind {
        1 => r[0x1c..0x20].copy_from_slice(&a.to_le_bytes()),
        4 => {
            r[0x1c..0x20].copy_from_slice(&a.to_le_bytes());
            r[0x20..0x24].copy_from_slice(&b.to_le_bytes());
        }
        5 => {
            let k = key?;
            r[0x1c..0x2c].copy_from_slice(&k.fmtid);
            r[0x2c..0x30].copy_from_slice(&k.pid.to_le_bytes());
        }
        _ => {}
    }
    for (i, c) in w.iter().enumerate() {
        r[DEVICE_RECORD_ID + 2 * i..DEVICE_RECORD_ID + 2 * i + 2].copy_from_slice(&c.to_le_bytes());
    }
    Some(r)
}

pub fn volume_record(ctx: &[u8; 16], muted: bool, master: f32, channels: &[f32]) -> Option<Vec<u8>> {
    let mut r = vec![0u8; VOLUME_RECORD_CHANNELS + 4 * channels.len()];
    if NOTIFY_RECORD + r.len() > gchost_frame() {
        return None;
    }
    r[0x18..0x28].copy_from_slice(ctx);
    r[0x28..0x2c].copy_from_slice(&(muted as u32).to_le_bytes());
    r[0x2c..0x30].copy_from_slice(&master.to_le_bytes());
    r[0x30..0x34].copy_from_slice(&(channels.len() as u32).to_le_bytes());
    for (i, c) in channels.iter().enumerate() {
        r[VOLUME_RECORD_CHANNELS + 4 * i..VOLUME_RECORD_CHANNELS + 4 * i + 4].copy_from_slice(&c.to_le_bytes());
    }
    Some(r)
}

pub fn notify_frame(listener: u32, record: &[u8]) -> Vec<u8> {
    let mut f = vec![0u8; gchost_frame()];
    f[4..8].copy_from_slice(&listener.to_le_bytes());
    f[NOTIFY_RECORD..NOTIFY_RECORD + record.len()].copy_from_slice(record);
    f
}

fn sent() -> std::sync::MutexGuard<'static, Vec<(i32, Vec<u8>)>> {
    static S: std::sync::OnceLock<Mutex<Vec<(i32, Vec<u8>)>>> = std::sync::OnceLock::new();
    S.get_or_init(|| Mutex::new(Vec::new())).lock().unwrap_or_else(|e| e.into_inner())
}

pub fn sent_notifications() -> Vec<(i32, Vec<u8>)> {
    sent().clone()
}

fn repeated(e: &wasapi::PcEvent) -> bool {
    static LAST: Mutex<Option<(String, std::time::Instant)>> = Mutex::new(None);
    let wasapi::PcEvent::Session { sid, kind, ctx, text, guid, value, volume, volumes } = e else { return false };
    let key = format!("{sid}|{kind}|{ctx:?}|{text}|{guid:?}|{value}|{}|{:?}", volume.to_bits(), volumes.iter().map(|v| v.to_bits()).collect::<Vec<_>>());
    let mut l = LAST.lock().unwrap_or_else(|p| p.into_inner());
    let now = std::time::Instant::now();
    let dup = matches!(&*l, Some((k, t)) if *k == key && now.duration_since(*t) < std::time::Duration::from_millis(PC_REPEAT_MS));
    *l = Some((key, now));
    dup
}

pub const PC_REPEAT_MS: u64 = 100;

fn pc_event(e: wasapi::PcEvent) {
    if repeated(&e) {
        return;
    }
    let mut frames = Vec::new();
    {
        let s = state();
        for (&k, o) in s.objects.iter().filter(|(_, o)| o.notify) {
            let rec = match (&e, o.notify_id) {
                (wasapi::PcEvent::Device { kind, id, a, b, key }, ENUMERATOR_SUBSCRIBE) => device_record(*kind, id, *a, *b, key.as_ref()),
                (wasapi::PcEvent::Session { sid, kind, ctx, text, guid, value, volume, volumes }, SESSION_REGISTER_NOTIFICATION | SESSION_MANAGER_REGISTER) if o.watch.as_deref() == Some(sid.as_str()) => match *kind {
                    1 if pc_session_of(&s, k).is_some() => session_simple_record(sid, ctx, *volume, *value != 0),
                    2 if pc_session_of(&s, k).is_some() => session_channel_record(sid, ctx, *value, volumes),
                    0x10 | 0x100 => session_text_record(*kind, sid, ctx, text),
                    4 => session_record(4, sid, ctx, guid, &[]),
                    8 | 0x20 => session_value_record(*kind, sid, *value),
                    _ => None,
                },
                _ => None,
            };
            if let Some(r) = rec {
                frames.push(notify_frame(k, &r));
            }
        }
    }
    deliver(frames);
}

fn endpoint_changed(object: u32, ctx: &[u8; 16]) {
    let key = endpoint_key(&device_for(object));
    let mut frames = Vec::new();
    {
        let s = state();
        let Some(e) = s.endpoints.get(&key) else { return };
        let Some(r) = volume_record(ctx, e.mute, e.scalar, &e.channels) else { return };
        for (&k, _) in s.objects.iter().filter(|(_, o)| o.notify && o.notify_id == ENDPOINT_REGISTER && o.watch.as_deref() == Some(key.as_str())) {
            frames.push(notify_frame(k, &r));
        }
    }
    deliver(frames);
}

pub const SESSION_RECORD_SID: usize = 0x48;
pub const SESSION_ALL_VOLUMES_CTX: usize = 0xcc;
pub const SESSION_RECORD_ARGS: usize = 0x30;

pub fn session_record(kind: u32, sid: &str, ctx: &[u8; 16], args: &[u8], tail: &[u8]) -> Option<Vec<u8>> {
    let w: Vec<u8> = sid.encode_utf16().chain(Some(0)).flat_map(|c| c.to_le_bytes()).collect();
    let base = SESSION_RECORD_SID + w.len();
    if args.len() > SESSION_RECORD_SID - SESSION_RECORD_ARGS {
        return None;
    }
    let mut r = vec![0u8; base];
    r[4..8].copy_from_slice(&kind.to_le_bytes());
    r[0x18..0x1c].copy_from_slice(&(w.len() as u32).to_le_bytes());
    r[0x1c..0x2c].copy_from_slice(ctx);
    r[SESSION_RECORD_ARGS..SESSION_RECORD_ARGS + args.len()].copy_from_slice(args);
    r[SESSION_RECORD_SID..base].copy_from_slice(&w);
    r.extend_from_slice(tail);
    if r.len() < 0x50 {
        return None;
    }
    if NOTIFY_RECORD + r.len() > gchost_frame() {
        return None;
    }
    let n = r.len() as u32;
    r[0..4].copy_from_slice(&n.to_le_bytes());
    Some(r)
}

fn session_sid_len(sid: &str) -> usize {
    SESSION_RECORD_SID + 2 * (sid.encode_utf16().count() + 1)
}

pub fn session_text_record(kind: u32, sid: &str, ctx: &[u8; 16], text: &str) -> Option<Vec<u8>> {
    let t: Vec<u8> = text.encode_utf16().chain(Some(0)).flat_map(|c| c.to_le_bytes()).collect();
    let mut a = (t.len() as u32).to_le_bytes().to_vec();
    a.extend_from_slice(&(session_sid_len(sid) as u32).to_le_bytes());
    session_record(kind, sid, ctx, &a, &t)
}

pub fn session_value_record(kind: u32, sid: &str, value: u32) -> Option<Vec<u8>> {
    if kind == 8 && value > 2 || kind == 0x20 && !matches!(value, 0 | 1 | 2 | 4 | 5) {
        return None;
    }
    session_record(kind, sid, &[0; 16], &value.to_le_bytes(), &[])
}

pub fn session_simple_record(sid: &str, ctx: &[u8; 16], volume: f32, mute: bool) -> Option<Vec<u8>> {
    let mut a = volume.to_le_bytes().to_vec();
    a.push(mute as u8);
    session_record(1, sid, ctx, &a, &[])
}

pub fn session_channel_record(sid: &str, ctx: &[u8; 16], changed: u32, volumes: &[f32]) -> Option<Vec<u8>> {
    let base = session_sid_len(sid);
    let off = (base + 3) & !3;
    let mut a = (off as u32).to_le_bytes().to_vec();
    a.extend_from_slice(&changed.to_le_bytes());
    a.extend_from_slice(&(volumes.len() as u32).to_le_bytes());
    let mut t = vec![0u8; off - base];
    for v in volumes {
        t.extend_from_slice(&v.to_le_bytes());
    }
    let r = session_record(2, sid, ctx, &a, &t)?;
    if r.len() != off + 4 * volumes.len() {
        return None;
    }
    Some(r)
}

fn session_changed(build: impl Fn(&str) -> Option<Vec<u8>>) {
    let mut frames = Vec::new();
    {
        let s = state();
        for (&k, o) in s.objects.iter().filter(|(_, o)| o.notify && matches!(o.notify_id, SESSION_REGISTER_NOTIFICATION | SESSION_MANAGER_REGISTER)) {
            if pc_session_of(&s, k).is_some() {
                continue;
            }
            if let Some(r) = o.watch.as_deref().and_then(&build) {
                frames.push(notify_frame(k, &r));
            }
        }
    }
    deliver(frames);
}

unsafe fn ctx_at(a: *const u8, o: usize) -> [u8; 16] {
    let mut g = [0u8; 16];
    core::ptr::copy_nonoverlapping(a.add(o), g.as_mut_ptr(), 16);
    g
}

fn deliver(frames: Vec<Vec<u8>>) {
    for f in frames {
        let hr = crate::gchost::push_audio(&f);
        compat_common::record_raw("crossvm-audio", "notify", &format!("host->guest notification to listener {:#x}, record kind {:#x} -> {hr:#010x}", u32::from_le_bytes(f[4..8].try_into().unwrap()), u32::from_le_bytes(f[0x24..0x28].try_into().unwrap())));
        let mut l = sent();
        if l.len() >= 256 {
            l.remove(0);
        }
        l.push((hr, f));
    }
}

pub fn notifications_registered(object: u32) -> bool {
    state().objects.get(&object).map(|o| o.notify).unwrap_or(false)
}

pub fn serialized_prop(vt: u16, count: Option<u32>, bytes: &[u8]) -> Vec<u8> {
    let mut d = vec![0u8; 8];
    d[0..2].copy_from_slice(&vt.to_le_bytes());
    if let Some(n) = count {
        d[4..8].copy_from_slice(&n.to_le_bytes());
    }
    d.extend_from_slice(bytes);
    d
}

pub const PROP_TAG: usize = 0x14;
pub const PROP_BYTES: usize = 0x18;
pub const PROP_LEN: usize = PROP_BYTES + PROP_DATA;

unsafe fn property_value(object: u32, a: *mut u8, room: usize) -> i32 {
    let mut key = wasapi::PropertyKey::default();
    core::ptr::copy_nonoverlapping(a, key.fmtid.as_mut_ptr(), 16);
    key.pid = rd(a, 0x10);
    let v = match wasapi::property_value(device_for(object).as_deref(), &key) {
        Ok(v) => v,
        Err(hr) => return hr,
    };
    core::ptr::write_bytes(a.add(0x14), 0, 4 + PROP_DATA + 8);
    let (tag, data): (u32, Vec<u8>) = match v {
        Some(wasapi::Prop::Empty) => (0, Vec::new()),
        Some(wasapi::Prop::Text(w)) => (1, w.iter().flat_map(|c| c.to_le_bytes()).collect()),
        Some(wasapi::Prop::Dword(d)) => (4, d.to_le_bytes().to_vec()),
        Some(wasapi::Prop::Serialized(vt, bytes)) => (3, serialized_prop(vt, None, &bytes)),
        Some(wasapi::Prop::Vector(vt, count, bytes)) => (3, serialized_prop(vt, Some(count), &bytes)),
        None => {
            compat_common::record_raw("crossvm-audio", "property", "PC property type has no decoded guest serialisation; E_NOTIMPL");
            return E_NOTIMPL;
        }
    };
    put_property(a, room, tag, &data)
}

pub unsafe fn put_property(a: *mut u8, room: usize, tag: u32, data: &[u8]) -> i32 {
    if PROP_BYTES + data.len() > room {
        compat_common::record_raw("crossvm-audio", "property", &format!("serialised value of {} bytes passes the {room:#x}-byte envelope payload; the console memcpy would write past its 0x810 allocation, host answers E_OUTOFMEMORY", data.len()));
        return E_OUTOFMEMORY;
    }
    core::ptr::copy_nonoverlapping(data.as_ptr(), a.add(PROP_BYTES), data.len());
    wr(a, PROP_TAG, tag);
    wr(a, PROP_LEN, data.len() as u64);
    S_OK
}

unsafe fn spatial(id: u32, object: u32, a: *mut u8) -> i32 {
    let sp = match wasapi::spatial(device_for(object).as_deref()) {
        Ok(s) => s,
        Err(hr) => return hr,
    };
    let put = |r: Result<u32, i32>, at: usize| match r {
        Ok(v) => {
            wr(a, at, v);
            S_OK
        }
        Err(hr) => hr,
    };
    match id {
        SPATIAL_NATIVE_MASK => put(sp.native_mask(), 0),
        SPATIAL_MAX_DYNAMIC => put(sp.max_dynamic(), 0),
        SPATIAL_MAX_FRAMES => {
            let f = format_at(a);
            put(sp.max_frames(&f), 0x48)
        }
        SPATIAL_FORMAT_SUPPORTED => sp.format_supported(&format_at(a)),
        SPATIAL_STREAM_AVAILABLE => sp.stream_available(&guid_from(core::slice::from_raw_parts(a, 16))),
        SPATIAL_STATIC_POSITION => {
            let kind: u32 = rd(a, 0);
            match sp.static_position(kind) {
                Ok(p) => {
                    for (i, v) in p.iter().enumerate() {
                        wr(a, 4 + 4 * i, *v);
                    }
                    S_OK
                }
                Err(hr) => hr,
            }
        }
        SPATIAL_MAX_FRAMES_FOR_CATEGORY => {
            let f = format_at(a);
            let cat: u32 = rd(a, SPATIAL_CATEGORY_AT);
            let offload: u8 = rd(a, SPATIAL_OFFLOAD_AT);
            put(sp.max_frames_for_category(cat, offload != 0, &f), SPATIAL_FRAMES_AT)
        }
        _ => {
            let cat: u32 = rd(a, 0);
            put(sp.offload_capable(cat), 4)
        }
    }
}

unsafe fn format_enum(id: u32, object: u32, a: *mut u8) -> i32 {
    let s = state();
    let Some(e) = s.objects.get(&object).filter(|o| o.kind == FORMAT_ENUM_TYPE).and_then(|o| o.formats.as_ref()) else {
        compat_common::record_raw("crossvm-audio", "format-enum", &format!("call id {id:#x} on object {object:#x}: only the type 4 object (map 1ac4d0: 0x3a -> 125a10, 0x3b -> 125a90) serves 0x3a / 0x3b and it serves nothing else; 125e00 answers E_NOTIMPL"));
        return E_NOTIMPL;
    };
    match id {
        FORMAT_ENUM_COUNT => match e.count() {
            Ok(n) => {
                wr(a, 0, n);
                S_OK
            }
            Err(hr) => hr,
        },
        FORMAT_ENUM_GET => {
            let cb: u16 = rd(a, FORMAT_ENUM_CB_AT);
            if cb as usize + PLAIN_FORMAT > FORMAT_ENUM_ROOM {
                return E_INVALIDARG;
            }
            let index: u32 = rd(a, 0);
            match e.count() {
                Ok(n) if index >= n => return E_BOUNDS,
                Err(hr) => return hr,
                _ => {}
            }
            match e.get(index) {
                Ok(f) => {
                    let plain = plain_format(&f);
                    core::ptr::copy_nonoverlapping(plain.as_ptr(), a.add(FORMAT_ENUM_AT), PLAIN_FORMAT);
                    S_OK
                }
                Err(hr) => hr,
            }
        }
        _ => E_NOTIMPL,
    }
}

pub fn plain_format(f: &[u8]) -> [u8; PLAIN_FORMAT] {
    let mut out = [0u8; PLAIN_FORMAT];
    out.copy_from_slice(&f[..PLAIN_FORMAT]);
    if u16::from_le_bytes([f[0], f[1]]) == 0xfffe && f.len() >= 0x28 {
        out[0..2].copy_from_slice(&f[0x18..0x1a]);
    }
    out[0x10] = 0;
    out[0x11] = 0;
    out
}

unsafe fn endpoint_volume(id: u32, object: u32, a: *mut u8) -> i32 {
    let ctx_at = match id {
        SET_ENDPOINT_VOLUME_LEVEL | SET_ENDPOINT_VOLUME_SCALAR => Some(4),
        SET_ENDPOINT_CHANNEL_LEVEL | SET_ENDPOINT_CHANNEL_SCALAR => Some(8),
        SET_ENDPOINT_MUTE => Some(1),
        ENDPOINT_STEP_UP | ENDPOINT_STEP_DOWN => Some(0),
        _ => None,
    };
    let ctx = ctx_at.map(|o| {
        let mut g = [0u8; 16];
        core::ptr::copy_nonoverlapping(a.add(o), g.as_mut_ptr(), 16);
        g
    });
    let hr = endpoint_volume_call(id, object, a);
    if let (Some(g), S_OK) = (ctx, hr) {
        endpoint_changed(object, &g);
    }
    hr
}

unsafe fn endpoint_volume_call(id: u32, object: u32, a: *mut u8) -> i32 {
    let dev = device_for(object);
    let caps = match wasapi::endpoint_caps(dev.as_deref()) {
        Ok(c) => c,
        Err(hr) => return hr,
    };
    let steps = caps.steps.max(2);
    match id {
        SET_ENDPOINT_VOLUME_LEVEL | SET_ENDPOINT_VOLUME_SCALAR => {
            let x: f32 = rd(a, 0);
            let v = if id == SET_ENDPOINT_VOLUME_LEVEL {
                if x < caps.min_db || x > caps.max_db {
                    return E_INVALIDARG;
                }
                level_to_scalar(x, &caps)
            } else {
                if !(0.0..=1.0).contains(&x) {
                    return E_INVALIDARG;
                }
                x
            };
            with_endpoint(&dev, |e| e.scalar = v);
            S_OK
        }
        GET_ENDPOINT_VOLUME_LEVEL | GET_ENDPOINT_VOLUME_SCALAR => {
            let v = with_endpoint(&dev, |e| e.scalar);
            wr(a, 0, if id == GET_ENDPOINT_VOLUME_LEVEL { scalar_to_level(v, &caps) } else { v });
            S_OK
        }
        SET_ENDPOINT_CHANNEL_LEVEL | SET_ENDPOINT_CHANNEL_SCALAR => {
            let ch: u32 = rd(a, 0);
            let x: f32 = rd(a, 4);
            let v = if id == SET_ENDPOINT_CHANNEL_LEVEL {
                if x < caps.min_db || x > caps.max_db {
                    return E_INVALIDARG;
                }
                level_to_scalar(x, &caps)
            } else {
                if !(0.0..=1.0).contains(&x) {
                    return E_INVALIDARG;
                }
                x
            };
            with_endpoint(&dev, |e| match e.channels.get_mut(ch as usize) {
                Some(c) => {
                    *c = v;
                    S_OK
                }
                None => E_INVALIDARG,
            })
        }
        GET_ENDPOINT_CHANNEL_LEVEL | GET_ENDPOINT_CHANNEL_SCALAR => {
            let ch: u32 = rd(a, 0);
            match with_endpoint(&dev, |e| e.channels.get(ch as usize).copied()) {
                Some(v) => {
                    wr(a, 0, if id == GET_ENDPOINT_CHANNEL_LEVEL { scalar_to_level(v, &caps) } else { v });
                    S_OK
                }
                None => E_INVALIDARG,
            }
        }
        SET_ENDPOINT_MUTE => {
            let m: u8 = rd(a, 0);
            let was = with_endpoint(&dev, |e| core::mem::replace(&mut e.mute, m != 0));
            if was == (m != 0) { S_FALSE } else { S_OK }
        }
        GET_ENDPOINT_MUTE => {
            let m = with_endpoint(&dev, |e| e.mute);
            wr(a, 0, m as u8);
            S_OK
        }
        ENDPOINT_STEP_INFO => {
            let v = with_endpoint(&dev, |e| e.scalar);
            wr(a, 0, (v * (steps - 1) as f32).round() as u32);
            wr(a, 4, steps);
            S_OK
        }
        ENDPOINT_STEP_UP | ENDPOINT_STEP_DOWN => {
            let d = 1.0 / (steps - 1) as f32;
            with_endpoint(&dev, |e| {
                let cur = (e.scalar * (steps - 1) as f32).round();
                let next = if id == ENDPOINT_STEP_UP { cur + 1.0 } else { cur - 1.0 };
                e.scalar = (next * d).clamp(0.0, 1.0);
            });
            S_OK
        }
        ENDPOINT_HARDWARE_SUPPORT => {
            wr(a, 0, 0u32);
            S_OK
        }
        _ => {
            wr(a, 0, caps.min_db);
            wr(a, 4, caps.max_db);
            wr(a, 8, caps.inc_db);
            S_OK
        }
    }
}

unsafe fn endpoint_list(object: u32, a: *mut u8) -> i32 {
    let flow: u32 = rd(a, 0);
    let mask: u32 = rd(a, 4);
    if flow > 2 || mask > 0xf {
        return E_INVALIDARG;
    }
    let list = match wasapi::endpoints(flow, mask) {
        Ok(l) => l,
        Err(hr) => return hr,
    };
    let mut s = state();
    let Some(o) = s.objects.get_mut(&object) else { return E_INVALIDARG };
    let at = o.list_cursor.min(list.len());
    let page: Vec<&(String, u32)> = list[at..].iter().take(LIST_PAGE).collect();
    let last = at + page.len() >= list.len();
    o.list_cursor = if last { 0 } else { at + page.len() };
    drop(s);
    core::ptr::write_bytes(a.add(8), 0, 8 + LIST_PAGE * LIST_ENTRY);
    wr(a, 8, page.len() as u32);
    wr(a, 0xc, last as u32);
    for (i, (id, st)) in page.iter().enumerate() {
        let e = a.add(0x10 + i * LIST_ENTRY);
        if !put_wide(e, id, BLOB_ID_CHARS + 1) {
            return E_INVALIDARG;
        }
        wr(e, 0x70, *st);
    }
    S_OK
}

unsafe fn default_table(a: *mut u8) -> i32 {
    let mut entries = Vec::new();
    for flow in 0..2u32 {
        for role in 0..PC_ROLES {
            match wasapi::default_id(flow, role) {
                Ok(id) => entries.push((flow, role, id)),
                Err(wasapi::E_NOTFOUND) => {}
                Err(hr) => return hr,
            }
        }
    }
    if entries.len() > LIST_PAGE {
        entries.truncate(LIST_PAGE);
    }
    core::ptr::write_bytes(a, 0, 8 + LIST_PAGE * TABLE_ENTRY);
    wr(a, 0, entries.len() as u32);
    wr(a, 4, 1u32);
    for (i, (flow, role, id)) in entries.iter().enumerate() {
        let e = a.add(8 + i * TABLE_ENTRY);
        wr(e, 0, *flow);
        wr(e, 4, *role);
        if !put_wide(e.add(8), id, BLOB_ID_CHARS + 1) {
            return E_INVALIDARG;
        }
    }
    S_OK
}

unsafe fn initialize(object: u32, a: *mut u8, room: usize, lay: &InitLayout) -> i32 {
    if room < lay.size {
        return E_INVALIDARG;
    }
    let (existing, props) = {
        let s = state();
        match s.objects.get(&object) {
            Some(o) => (o.stream.is_some(), o.properties),
            None => return E_INVALIDARG,
        }
    };
    if existing {
        return AUDCLNT_E_ALREADY_INITIALIZED;
    }
    let mode: u32 = lay.mode.map(|o| rd(a, o)).unwrap_or(wasapi::SHARED);
    if mode > 1 {
        return E_INVALIDARG;
    }
    let flags: u32 = rd(a, lay.flags);
    let duration: i64 = lay.duration.map(|o| rd(a, o)).unwrap_or(0);
    let period: Option<u32> = lay.period.map(|o| rd(a, o));
    if period == Some(0) {
        return E_INVALIDARG;
    }
    let format = format_at(a.add(lay.format));
    let flen = format.len();
    let align = wasapi::block_align(&format);
    let rate = wasapi::sample_rate(&format);
    if align == 0 || rate == 0 {
        return E_INVALIDARG;
    }
    let session = guid_from(core::slice::from_raw_parts(a.add(lay.session), 16));
    let dev = device_for(object);
    let info = match wasapi::device_info_for(dev.as_deref()) {
        Ok(d) => d,
        Err(hr) => return hr,
    };
    let period_frames = period.unwrap_or((info.default_period * rate as i64 / HNS).max(1) as u32);
    let floor = period.map(|p| p as i64 * 2 * HNS / rate as i64).unwrap_or(0).max(info.default_period * 2);
    let frames = ((duration.max(floor) * rate as i64 + HNS - 1) / HNS) as u32;
    let l = layout(&format, frames, period_frames);
    let pages = (l.total as usize).div_ceil(crate::logan::PAGE);
    let Some(index) = crate::logan::pool_alloc(pages) else {
        return E_OUTOFMEMORY;
    };
    let base = crate::logan::pool() + index as usize * crate::logan::PAGE;
    let hns = frames as i64 * HNS / rate as i64;
    write_control_block(base, &format, hns, &l);
    let ring = wasapi::Ring { base, start: l.start, end: l.end, align };
    let open = wasapi::Open { device: dev.clone(), format: format.clone(), duration: hns, session, properties: props, flags, period_frames: period };
    let capture = dev.is_some() && wasapi::device_flow(dev.as_deref()) == Ok(1);
    let opened = if capture {
        wasapi::open_capture(open, ring, wasapi::CaptureQueue { base, queue: l.queue as usize, entries: l.entries })
    } else {
        wasapi::open(open, ring)
    };
    let (renderer, device_frames) = match opened {
        Ok(r) => r,
        Err(hr) => {
            crate::logan::pool_release(index as usize);
            return hr;
        }
    };
    let event_id = {
        let mut s = state();
        s.next_event += 1;
        s.next_event
    };
    let name = event_name(event_id as u16);
    let event = match crate::vab::create_event(&name) {
        Ok(h) => h,
        Err(e) => {
            crate::logan::pool_release(index as usize);
            return (0x8007_0000 | (e & 0xffff)) as i32;
        }
    };
    core::ptr::write_bytes(a.add(lay.clsid), 0, lay.size - lay.clsid);
    core::ptr::copy_nonoverlapping(format.as_ptr(), a.add(lay.format), flen);
    core::ptr::copy_nonoverlapping(guid_bytes(if capture { &CLSID_CLIENT_INPUT_ENDPOINT } else { &CLSID_CLIENT_OUTPUT_ENDPOINT }).as_ptr(), a.add(lay.clsid), 16);
    wr(a, lay.event1, KIND_NONE);
    wr(a, lay.event2, KIND_NONE);
    write_event_descriptor(a.add(lay.event3), EVENT_KIND_GAMECORE, event_id as u16);
    wr(a, lay.memory, KIND_CROSS_VM_MEMORY);
    core::ptr::copy_nonoverlapping(guid_bytes(&CLSID_XVM_BUFFER).as_ptr(), a.add(lay.memory + 4), 16);
    core::ptr::copy_nonoverlapping(guid_bytes(&GUID_XVM_BUFFER).as_ptr(), a.add(lay.memory + 0x14), 16);
    wr(a, lay.memory + 0x24, index as u32);
    compat_common::record_raw("crossvm-audio", "Initialize", &format!("object {object:#x}: {frames} frames ({} bytes) at pool page {index}, WASAPI buffer {device_frames} frames, period {period:?}, flags {flags:#x}, period event {name}, {}", l.bytes, if capture { "capture" } else { "render" }));
    let mut s = state();
    s.session.get_or_insert(session);
    if let Some(o) = s.objects.get_mut(&object) {
        o.stream = Some(Stream { pool_index: index, base, format, frames, event, event_name: name, renderer });
    }
    apply_all(&mut s);
    S_OK
}

pub unsafe fn write_event_descriptor(p: *mut u8, kind: u32, index: u16) {
    wr(p, 0, KIND_CROSS_VM_EVENT);
    core::ptr::copy_nonoverlapping(guid_bytes(&CLSID_XVM_EVENT).as_ptr(), p.add(4), 16);
    core::ptr::copy_nonoverlapping(guid_bytes(&GUID_XVM_EVENT).as_ptr(), p.add(0x14), 16);
    wr(p, 0x24, kind);
    wr(p, 0x28, index);
}

pub fn stream_info(object: u32) -> Option<(usize, u16, u32, String, i64)> {
    let s = state();
    let st = s.objects.get(&object)?.stream.as_ref()?;
    Some((st.base, st.pool_index, st.frames, st.event_name.clone(), st.renderer.shared.rendered.load(Ordering::SeqCst)))
}

pub fn stream_records(object: u32) -> Option<(i64, f32, i64)> {
    let s = state();
    let r = &s.objects.get(&object)?.stream.as_ref()?.renderer;
    Some((r.records(), r.current_rate(), r.rebases()))
}

pub fn stream_signals(object: u32) -> Option<i64> {
    let s = state();
    Some(s.objects.get(&object)?.stream.as_ref()?.renderer.shared.signals.load(Ordering::SeqCst))
}

pub fn stream_gain(object: u32) -> Option<(Option<f32>, Option<Vec<f32>>)> {
    let s = state();
    let sh = &s.objects.get(&object)?.stream.as_ref()?.renderer.shared;
    let g = *sh.gain.lock().unwrap();
    let c = sh.channels.lock().unwrap().clone();
    Some((g, c))
}

pub fn object_kind(object: u32) -> Option<u32> {
    state().objects.get(&object).map(|o| o.kind)
}

pub fn object_device(object: u32) -> Option<String> {
    device_for(object)
}

pub fn take_capture(object: u32) -> Option<Vec<u8>> {
    let s = state();
    let st = s.objects.get(&object)?.stream.as_ref()?;
    let mut c = st.renderer.shared.capture.lock().unwrap();
    c.as_mut().map(std::mem::take)
}

pub fn objects() -> usize {
    state().objects.len()
}
