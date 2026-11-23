use super::{xmem_fn, XMemGetWorkingSetStatistics};
use core::ffi::c_void;

pub const SESSION_NAME: &str = "NewTimingCaptureSession";
pub const PROPERTIES_BYTES: usize = 0x2fc;
pub const NAME_LIMIT: usize = 0x20;
pub const E_INVALIDARG: i32 = 0x8007_0057u32 as i32;
pub const WNODE_FLAG_TRACED_GUID: u32 = 0x2_0000;
pub const CLIENT_CONTEXT_CYCLES: u32 = 3;
pub const LOGGER_NAME_OFFSET: u32 = 0x78;
pub const LOG_FILE_NAME_OFFSET: u32 = 0xb8;
pub const MODE_SYSTEM_CIRCULAR: u32 = 0x200_0002;
pub const MODE_REAL_TIME_EXTRA: u32 = 0x10e;
pub const ENABLE_FLAGS_EXTENSION: u32 = 0x8000_0000;
pub const EXTENSION_OFFSET: u16 = 0x2c0;
pub const EXTENSION_HEADER: u32 = 0x3_000f;
pub const GROUP_MASK_HEADER: u32 = 0x1_0009;
pub const MAX_FILE_MB: u32 = 0x200;
pub const FLUSH_TIMER_MS: u32 = 0xc8;
pub const PARTITION_TOOLS_MIN: u64 = 0x800_0000;
pub const PROFILE_CLOCK: f64 = 10_000_000.0;
pub const TRACE_STACK_TRACING_INFO: u32 = 3;
pub const TRACE_SAMPLED_PROFILE_INTERVAL_INFO: u32 = 5;
pub const TRACE_PROFILE_SOURCE_CONFIG_INFO: u32 = 6;
pub const EVENT_CONTROL_CODE_CAPTURE_STATE: u32 = 2;
pub const EVENT_FILTER_TYPE_STACKWALK_LEVEL_KW: u32 = 0x8000_4000;
pub const ENABLE_PROPERTY_STACK_TRACE: u32 = 4;
pub const MASK_SAMPLED_PROFILE: u32 = 0x100_0000;

pub const SYSTEM_GUID: [u8; 16] = [0xab, 0x62, 0x29, 0x22, 0x80, 0x61, 0x88, 0x4b, 0xa8, 0x25, 0x34, 0x6b, 0x75, 0xf2, 0xa2, 0x4a];
pub const GUID_52258: [u8; 16] = [0xb4, 0xbf, 0x1d, 0xce, 0x7e, 0x13, 0xa6, 0x4d, 0x87, 0xb0, 0x3f, 0x59, 0xaa, 0x10, 0x2c, 0xbc];
pub const GUID_52268: [u8; 16] = [0xb6, 0xa8, 0x68, 0x02, 0xfd, 0x74, 0x02, 0x43, 0x9d, 0xd0, 0x6e, 0x8f, 0x17, 0x95, 0xc0, 0xcf];
pub const GUID_52288: [u8; 16] = [0x39, 0xdc, 0xcb, 0x90, 0x3e, 0x4a, 0xd1, 0x11, 0x84, 0xf4, 0x00, 0x00, 0xf8, 0x04, 0x64, 0xe3];
pub const GUID_522A8: [u8; 16] = [0xd3, 0xa8, 0x6f, 0x3d, 0x05, 0xfe, 0xd0, 0x11, 0x9d, 0xda, 0x00, 0xc0, 0x4f, 0xd7, 0xba, 0x7c];
pub const GUID_522B8: [u8; 16] = [0xd1, 0xa8, 0x6f, 0x3d, 0x05, 0xfe, 0xd0, 0x11, 0x9d, 0xda, 0x00, 0xc0, 0x4f, 0xd7, 0xba, 0x7c];
pub const GUID_527A0: [u8; 16] = [0x18, 0xab, 0xb6, 0x28, 0x07, 0x95, 0xa4, 0x4e, 0x80, 0xd1, 0xd1, 0x0e, 0x9e, 0xf3, 0x6f, 0xc0];
pub const GUID_527B0: [u8; 16] = [0xa6, 0xd9, 0x68, 0x35, 0x09, 0xdc, 0x6c, 0x4a, 0xac, 0x0f, 0x88, 0xc4, 0x70, 0xb7, 0x7f, 0x5e];
pub const GUID_52820: [u8; 16] = [0xef, 0x69, 0x57, 0x60, 0x12, 0x24, 0xee, 0x42, 0xa5, 0x64, 0x72, 0x07, 0x25, 0x39, 0x44, 0x29];
pub const GUID_52848: [u8; 16] = [0xb0, 0x98, 0x29, 0x55, 0xcc, 0x06, 0x28, 0x4c, 0xac, 0xb7, 0x9a, 0xcb, 0xe3, 0x15, 0x53, 0xb0];
pub const GUID_52858: [u8; 16] = [0xf9, 0x32, 0x02, 0x25, 0x83, 0x21, 0x18, 0x4b, 0x87, 0x1d, 0x3c, 0x76, 0x3c, 0x50, 0xcf, 0xd5];
pub const GUID_INLINE_2B537: [u8; 16] = [0xe0, 0x1c, 0x7d, 0xc8, 0x42, 0x1e, 0x8b, 0x48, 0xbf, 0x05, 0x19, 0x46, 0xc9, 0x5b, 0xb9, 0x52];

pub const GROUP_MASK_TABLE: [(u32, u32); 11] = [
    (0x1, 0x1),
    (0x2, 0x2),
    (0x4, 0x4),
    (0x10, 0x2000_1004),
    (0x800, 0x2000_0200),
    (0x100_0000, 0x2000_0002),
    (0x200_0000, 0x200_0000),
    (0x400_0000, 0x400_0000),
    (0x200, 0x200),
    (0x100, 0x300),
    (0x4000, 0x2000_8040),
];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Provider {
    pub enable: u8,
    pub stack: u8,
    pub guid: [u8; 16],
    pub rundown: u8,
    pub timeout: u32,
    pub has_level: u8,
    pub level: u8,
    pub any: u64,
    pub all: u64,
    pub filter: u8,
    pub filter_level: u8,
    pub filter_any: u64,
    pub filter_all: u64,
}

impl Provider {
    fn new(guid: [u8; 16]) -> Provider {
        Provider { enable: 1, guid, timeout: u32::MAX, ..Default::default() }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct StackEvent {
    pub guid: [u8; 16],
    pub kind: u8,
    pub reserved: [u8; 7],
}

impl StackEvent {
    fn new(guid: [u8; 16], kind: u8) -> StackEvent {
        StackEvent { guid, kind, reserved: [0; 7] }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(C)]
pub struct ProfileSource {
    pub source: u32,
    pub interval: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionConfig {
    pub name: String,
    pub log_file: String,
    pub buffer_kb: u32,
    pub min_buffers: u32,
    pub max_buffers: u32,
    pub rate: u32,
    pub mask: u32,
    pub pid: u32,
    pub providers: Vec<Provider>,
    pub stack_events: Vec<StackEvent>,
    pub profile_sources: Vec<ProfileSource>,
}

impl SessionConfig {
    pub fn new(name: &str, buffer_kb: u32, min_buffers: u32, max_buffers: u32) -> SessionConfig {
        SessionConfig {
            name: name.to_string(),
            log_file: String::new(),
            buffer_kb,
            min_buffers,
            max_buffers,
            rate: 0x3e8,
            mask: 0,
            pid: 0,
            providers: Vec::new(),
            stack_events: Vec::new(),
            profile_sources: Vec::new(),
        }
    }

    pub fn timing() -> SessionConfig {
        SessionConfig::new(SESSION_NAME, 0x40, 0x400, 0x50a)
    }
}

pub type TimingConfig = [u32; 18];

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CaptureOptions {
    pub kind: u32,
    pub flag4: u8,
    pub sources: Vec<ProfileSource>,
}

#[repr(C)]
pub struct TimingDesc {
    pub name: *const u16,
    pub cap_mb: u32,
    pub reserved: u32,
    pub f10: u32,
    pub f14: u32,
    pub f18: u32,
    pub f1c: u32,
    pub f20: u32,
    pub f24: u32,
    pub f28: u32,
    pub f2c: u32,
    pub f30: u32,
}

fn on(v: u32, hi: u32) -> u32 {
    if v != 0 {
        hi
    } else {
        0
    }
}

pub fn timing_config(d: &TimingDesc) -> TimingConfig {
    [
        1,
        1 + (d.f14 != 0) as u32,
        1 + (d.f18 != 0) as u32,
        0,
        d.f10,
        1,
        1,
        0,
        if d.f18 != 0 { d.f1c } else { 0xa },
        1,
        on(d.f20, 2),
        0,
        0,
        on(d.f24, 2),
        on(d.f28, 2),
        on(d.f2c, 2),
        on(d.f30, 2),
        0,
    ]
}

pub fn capture_options(d: &TimingDesc) -> CaptureOptions {
    CaptureOptions { kind: 1 + (d.f14 != 0) as u32, ..Default::default() }
}

fn ge(v: u32, n: i32) -> bool {
    v as i32 >= n
}

fn append_sources(o: &CaptureOptions, c: &mut SessionConfig) {
    c.profile_sources.extend(o.sources.iter().copied());
    c.stack_events.push(StackEvent::new(GUID_52258, 0x2f));
}

extern "system" {
    fn GetCurrentProcessId() -> u32;
}

pub fn kernel_config(t: &TimingConfig, o: &CaptureOptions) -> SessionConfig {
    let mut c = SessionConfig::timing();
    c.rate = t[8];
    c.mask |= MASK_SAMPLED_PROFILE;
    if ge(t[14], 1) {
        c.pid = unsafe { GetCurrentProcessId() };
    }
    append_sources(o, &mut c);
    c
}

pub fn user_config(t: &TimingConfig, o: &CaptureOptions) -> SessionConfig {
    let mut c = SessionConfig::timing();
    c.rate = t[8];
    c.mask = 7;
    c.providers.push(Provider::new(GUID_52820));
    if t[3] != 0 {
        c.providers.push(Provider::new(GUID_52858));
    }
    if ge(t[1], 1) {
        c.mask |= 0x10;
        if ge(t[1], 2) {
            c.stack_events.push(StackEvent::new(GUID_522B8, 0x24));
        }
    }
    if ge(o.kind, 1) {
        c.mask |= 1 << 11;
        if ge(o.kind, 2) {
            c.stack_events.push(StackEvent::new(GUID_522B8, 0x32));
        }
    }
    if ge(t[2], 1) {
        c.mask |= 1 << 24;
        if ge(t[2], 2) {
            c.stack_events.push(StackEvent::new(GUID_52258, 0x2e));
        }
    }
    let mut p = Provider { rundown: 1, has_level: 1, level: 3, ..Provider::new(GUID_527A0) };
    if ge(t[10], 1) {
        p.level = 4;
        c.mask |= 0x600_0200;
        if ge(t[10], 2) {
            c.stack_events.push(StackEvent::new(GUID_52288, 0x43));
            c.stack_events.push(StackEvent::new(GUID_52288, 0x44));
        }
    }
    c.providers.push(p);
    let mut q = Provider { has_level: 1, level: 5, ..Provider::new(GUID_INLINE_2B537) };
    let mut push = false;
    if ge(t[13], 1) {
        c.mask |= 1 << 14;
        push = true;
        q.rundown = 1;
        q.any = 6;
        if ge(t[13], 2) {
            c.stack_events.push(StackEvent::new(GUID_522A8, 0x62));
            c.stack_events.push(StackEvent::new(GUID_522A8, 0x63));
            c.stack_events.push(StackEvent::new(GUID_52268, 0x20));
        }
    }
    if ge(t[14], 1) {
        push = true;
        q.any |= 4;
        c.pid = unsafe { GetCurrentProcessId() };
        if ge(t[14], 2) {
            c.stack_events.push(StackEvent::new(SYSTEM_GUID, 0x21));
        }
    }
    if ge(t[15], 1) {
        q.any |= 9;
        push = true;
        if ge(t[15], 2) {
            q.stack = 1;
            q.filter = 1;
            q.filter_level = 5;
            q.filter_any = 1;
        }
    }
    if push {
        c.providers.push(q);
    }
    if ge(t[16], 1) {
        let mut r = Provider::new(GUID_52848);
        r.stack = ge(t[16], 2) as u8;
        c.providers.push(r);
    }
    if o.flag4 != 0 {
        c.providers.push(Provider::new(GUID_527B0));
    }
    append_sources(o, &mut c);
    c
}

pub fn group_mask(c: &SessionConfig) -> [u32; 8] {
    let mut m = [0u32; 8];
    for &(bit, v) in GROUP_MASK_TABLE.iter() {
        if c.mask & bit != 0 {
            m[(v >> 29) as usize] |= v & 0x1fff_ffff;
        }
    }
    if !c.profile_sources.is_empty() {
        m[1] |= 1 << 10;
    }
    m
}

#[repr(C, align(8))]
pub struct Properties(pub [u8; 0x300]);

impl Properties {
    fn zero() -> Properties {
        Properties([0; 0x300])
    }

    fn put<T: Copy>(&mut self, off: usize, v: T) {
        unsafe { core::ptr::write_unaligned(self.0.as_mut_ptr().add(off) as *mut T, v) }
    }

    pub fn get<T: Copy>(&self, off: usize) -> T {
        unsafe { core::ptr::read_unaligned(self.0.as_ptr().add(off) as *const T) }
    }

    fn header() -> Properties {
        let mut p = Properties::zero();
        p.put(0, PROPERTIES_BYTES as u32);
        p.put(0x2c, WNODE_FLAG_TRACED_GUID);
        p.put(0x74, LOGGER_NAME_OFFSET);
        p.put(0x28, CLIENT_CONTEXT_CYCLES);
        p
    }

    fn ptr(&mut self) -> *mut c_void {
        self.0.as_mut_ptr() as *mut c_void
    }
}

pub fn base_properties(mask: &[u32; 8], pid: u32, real_time: bool) -> Properties {
    let mut p = Properties::header();
    p.put(0x48, ENABLE_FLAGS_EXTENSION);
    p.put(0x4a, 0xffu8);
    p.put(0x40, MODE_SYSTEM_CIRCULAR + if real_time { MODE_REAL_TIME_EXTRA } else { 0 });
    p.put(0x48, EXTENSION_OFFSET);
    p.put(0x2c0, EXTENSION_HEADER);
    p.put(0x2c4, GROUP_MASK_HEADER);
    p.put(0x2f4, 2u16);
    p.put(0x2e8, 3u16);
    for (i, v) in mask.iter().enumerate() {
        p.put(0x2c8 + i * 4, *v);
    }
    if pid != 0 {
        p.put(0x18, SYSTEM_GUID);
        p.put(0x2f6, 2u16);
        p.put(0x2f8, pid);
    }
    p
}

pub fn properties(c: &SessionConfig, partition: u64) -> Properties {
    let mut p = base_properties(&group_mask(c), c.pid, c.log_file.is_empty());
    p.put(0x30, c.buffer_kb);
    p.put(0x34, c.min_buffers);
    p.put(0x38, c.max_buffers);
    if c.log_file.is_empty() {
        p.put(0x44, FLUSH_TIMER_MS);
    } else {
        p.put(0x3c, MAX_FILE_MB);
        p.put(0x70, LOG_FILE_NAME_OFFSET);
        for (i, u) in c.log_file.encode_utf16().chain(Some(0)).enumerate() {
            p.put(LOG_FILE_NAME_OFFSET as usize + i * 2, u);
        }
    }
    if partition != 0 {
        p.put(0x2ec, partition);
        p.put(0x2ea, 6u16);
    }
    p
}

#[link(name = "advapi32")]
extern "system" {
    fn StartTraceW(h: *mut u64, name: *const u16, p: *mut c_void) -> u32;
    fn StopTraceW(h: u64, name: *const u16, p: *mut c_void) -> u32;
    fn ControlTraceW(h: u64, name: *const u16, p: *mut c_void, code: u32) -> u32;
    fn EnableTraceEx2(h: u64, guid: *const [u8; 16], code: u32, level: u8, any: u64, all: u64, timeout: u32, params: *const c_void) -> u32;
    fn TraceSetInformation(h: u64, class: u32, info: *const c_void, len: u32) -> u32;
    fn UpdateTraceW(h: u64, name: *const u16, p: *mut c_void) -> u32;
}

pub fn hr(e: u32) -> i32 {
    if e as i32 <= 0 {
        e as i32
    } else {
        ((e & 0xffff) | 0x8007_0000) as i32
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn name_ok(name: &str) -> bool {
    name.encode_utf16().count() < NAME_LIMIT
}

pub fn cvttsd2si(v: f64) -> i64 {
    if v.is_finite() && v > -9.223_372_036_854_776e18 && v < 9.223_372_036_854_776e18 {
        v as i64
    } else {
        i64::MIN
    }
}

pub fn profile_interval(rate: u32) -> u32 {
    cvttsd2si(PROFILE_CLOCK / rate as f64) as u32
}

unsafe fn tools_partition() -> u64 {
    let f = xmem_fn(b"XMemGetPartitionHandleAndId\0");
    if f == 0 {
        return 0;
    }
    let get = core::mem::transmute::<usize, unsafe extern "system" fn(u32, *mut u64, *mut u32) -> u8>(f);
    let mut h = 0u64;
    if get(2, &mut h, core::ptr::null_mut()) == 0 || h == 0 {
        return 0;
    }
    let mut s = [0u8; 0x78];
    XMemGetWorkingSetStatistics(0x2000_8002, s.as_mut_ptr() as *mut c_void);
    let t = u64::from_le_bytes(s[0x10..0x18].try_into().unwrap());
    let a = u64::from_le_bytes(s[0x18..0x20].try_into().unwrap());
    if t.wrapping_sub(a) < PARTITION_TOOLS_MIN {
        return 0;
    }
    h
}

pub unsafe fn start_session(c: &SessionConfig) -> Result<u64, i32> {
    if !name_ok(&c.name) {
        return Err(E_INVALIDARG);
    }
    let part = tools_partition();
    let mut p = properties(c, part);
    if c.mask & MASK_SAMPLED_PROFILE != 0 {
        let info = ProfileSource { source: 0, interval: profile_interval(c.rate) };
        let r = hr(TraceSetInformation(0, TRACE_SAMPLED_PROFILE_INTERVAL_INFO, &info as *const ProfileSource as *const c_void, 8));
        if r < 0 {
            return Err(r);
        }
    }
    for s in &c.profile_sources {
        let r = hr(TraceSetInformation(0, TRACE_SAMPLED_PROFILE_INTERVAL_INFO, s as *const ProfileSource as *const c_void, 8));
        if r < 0 {
            return Err(r);
        }
    }
    if !c.profile_sources.is_empty() {
        let ids: Vec<u32> = c.profile_sources.iter().map(|s| s.source).collect();
        TraceSetInformation(0, TRACE_PROFILE_SOURCE_CONFIG_INFO, ids.as_ptr() as *const c_void, (ids.len() * 4) as u32);
    }
    let n = wide(&c.name);
    let mut h = 0u64;
    let r = hr(StartTraceW(&mut h, n.as_ptr(), p.ptr()));
    if r < 0 {
        return Err(r);
    }
    Ok(h)
}

#[repr(C)]
struct LevelKw {
    any: u64,
    all: u64,
    level: u8,
    filter_in: u8,
    pad: [u8; 6],
}

#[repr(C)]
struct FilterDesc {
    ptr: u64,
    size: u32,
    kind: u32,
}

#[repr(C)]
struct EnableParams {
    version: u32,
    property: u32,
    control: u32,
    source: [u8; 16],
    pad: u32,
    filters: *const FilterDesc,
    count: u32,
    pad2: u32,
}

pub unsafe fn enable_providers(c: &SessionConfig, h: u64) -> i32 {
    for p in &c.providers {
        let kw = LevelKw { any: p.filter_any, all: p.filter_all, level: p.filter_level, filter_in: 1, pad: [0; 6] };
        let fd = FilterDesc { ptr: &kw as *const LevelKw as u64, size: 0x18, kind: EVENT_FILTER_TYPE_STACKWALK_LEVEL_KW };
        let mut ep = EnableParams { version: 0, property: 0, control: 0, source: [0; 16], pad: 0, filters: core::ptr::null(), count: 0, pad2: 0 };
        let params: *const c_void = if p.stack != 0 {
            ep.version = 2;
            ep.property = ENABLE_PROPERTY_STACK_TRACE;
            if p.filter != 0 {
                ep.filters = &fd;
                ep.count = 1;
            }
            &ep as *const EnableParams as *const c_void
        } else {
            core::ptr::null()
        };
        let (level, any, all) = if p.has_level != 0 {
            if p.level > 5 {
                return E_INVALIDARG;
            }
            (p.level, p.any, p.all)
        } else {
            (0, 0, 0)
        };
        let r = hr(EnableTraceEx2(h, &p.guid, (p.enable != 0) as u32, level, any, all, p.timeout, params));
        if r < 0 {
            return r;
        }
    }
    let events = if c.stack_events.is_empty() { core::ptr::null() } else { c.stack_events.as_ptr() as *const c_void };
    let r = hr(TraceSetInformation(h, TRACE_STACK_TRACING_INFO, events, (c.stack_events.len() * 0x18) as u32));
    if r < 0 {
        return r;
    }
    let mut p = base_properties(&group_mask(c), c.pid, c.log_file.is_empty());
    let r = hr(UpdateTraceW(h, core::ptr::null(), p.ptr()));
    if r < 0 {
        return r;
    }
    0
}

pub unsafe fn capture_state(c: &SessionConfig) -> i32 {
    if !name_ok(&c.name) {
        return E_INVALIDARG;
    }
    let mut p = Properties::header();
    let n = wide(&c.name);
    let r = hr(ControlTraceW(0, n.as_ptr(), p.ptr(), 0));
    if r < 0 {
        return r;
    }
    let h: u64 = p.get(8);
    for e in c.providers.iter().filter(|e| e.rundown != 0) {
        let (level, any, all) = if e.has_level != 0 {
            if e.level > 5 {
                return E_INVALIDARG;
            }
            (e.level, e.any, e.all)
        } else {
            (0, 0, 0)
        };
        let r = hr(EnableTraceEx2(h, &e.guid, EVENT_CONTROL_CODE_CAPTURE_STATE, level, any, all, e.timeout, core::ptr::null()));
        if r < 0 {
            return r;
        }
    }
    0
}

pub unsafe fn stop_by_name(name: &str, lost: Option<&mut u64>, rt_lost: Option<&mut u64>) -> i32 {
    if !name_ok(name) {
        return E_INVALIDARG;
    }
    let mut p = Properties::header();
    let n = wide(name);
    if hr(StopTraceW(0, n.as_ptr(), p.ptr())) < 0 {
        return 0;
    }
    if let Some(l) = lost {
        *l = p.get::<u32>(0x58) as u64;
    }
    if let Some(w) = rt_lost {
        *w = p.get::<u32>(0x64) as u64;
    }
    0
}
