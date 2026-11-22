use core::ffi::c_void;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::OnceLock;

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Guid {
    pub data1: u32,
    pub data2: u16,
    pub data3: u16,
    pub data4: [u8; 8],
}

impl Guid {
    pub fn from_le(b: &[u8]) -> Guid {
        Guid {
            data1: u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
            data2: u16::from_le_bytes([b[4], b[5]]),
            data3: u16::from_le_bytes([b[6], b[7]]),
            data4: [b[8], b[9], b[10], b[11], b[12], b[13], b[14], b[15]],
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct EventDescriptor {
    pub id: u16,
    pub version: u8,
    pub channel: u8,
    pub level: u8,
    pub opcode: u8,
    pub task: u16,
    pub keyword: u64,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct EventData {
    pub ptr: u64,
    pub size: u32,
    pub reserved: u32,
}

impl EventData {
    pub fn of<T>(v: &T) -> EventData {
        EventData { ptr: v as *const T as u64, size: core::mem::size_of::<T>() as u32, reserved: 0 }
    }

    pub fn bytes(b: &[u8], reserved: u32) -> EventData {
        EventData { ptr: b.as_ptr() as u64, size: b.len() as u32, reserved }
    }

    pub fn wstr(s: Option<&[u16]>) -> EventData {
        static EMPTY: [u16; 1] = [0];
        match s {
            Some(w) => EventData { ptr: w.as_ptr() as u64, size: (w.len() * 2) as u32, reserved: 0 },
            None => EventData { ptr: EMPTY.as_ptr() as u64, size: 2, reserved: 0 },
        }
    }
}

type EnableCallback = unsafe extern "system" fn(*const Guid, u32, u8, u64, u64, *const c_void, *mut c_void);

#[link(name = "advapi32")]
extern "system" {
    fn EventRegister(guid: *const Guid, cb: Option<EnableCallback>, ctx: *mut c_void, handle: *mut u64) -> u32;
    fn EventSetInformation(handle: u64, class: u32, info: *const c_void, len: u32) -> u32;
    fn EventWriteTransfer(handle: u64, desc: *const EventDescriptor, activity: *const Guid, related: *const Guid, count: u32, data: *const EventData) -> u32;
}

pub const ACTIVITY_GUID: &[u8; 16] = include_bytes!("../data/activity_provider_guid.bin");
pub const ACTIVITY_META: &[u8] = include_bytes!("../data/activity_provider_meta.bin");
pub const STATE_GUID: &[u8; 16] = include_bytes!("../data/state_provider_guid.bin");
pub const STATE_META: &[u8] = include_bytes!("../data/state_provider_meta.bin");
pub const WDAC_EVENT_DESC: &[u8; 11] = include_bytes!("../data/wdac_event_desc.bin");
pub const WDAC_EVENT_META: &[u8] = include_bytes!("../data/wdac_event_meta.bin");
pub const BLOCKLIST_EVENT_DESC: &[u8; 11] = include_bytes!("../data/blocklist_event_desc.bin");
pub const BLOCKLIST_EVENT_META: &[u8] = include_bytes!("../data/blocklist_event_meta.bin");

pub const WATERMARK_GUID: &[u8; 16] = include_bytes!("../data/watermark_provider_guid.bin");
pub const WATERMARK_META: &[u8] = include_bytes!("../data/watermark_provider_meta.bin");
pub const RETAIL_UNLOCKED_EVENT_DESC: &[u8; 11] = include_bytes!("../data/retail_unlocked_event_desc.bin");
pub const RETAIL_UNLOCKED_EVENT_META: &[u8] = include_bytes!("../data/retail_unlocked_event_meta.bin");
pub const HSTI_FAILURE_EVENT_DESC: &[u8; 11] = include_bytes!("../data/hsti_failure_event_desc.bin");
pub const HSTI_FAILURE_EVENT_META: &[u8] = include_bytes!("../data/hsti_failure_event_meta.bin");
pub const PRODUCTION_EVENT_DESC: &[u8; 11] = include_bytes!("../data/production_event_desc.bin");
pub const PRODUCTION_EVENT_META: &[u8] = include_bytes!("../data/production_event_meta.bin");

pub fn desc_keyword(desc: &[u8; 11]) -> u64 {
    u64::from_le_bytes([desc[3], desc[4], desc[5], desc[6], desc[7], desc[8], desc[9], desc[10]])
}

pub struct Provider {
    pub level_plus1: AtomicU32,
    pub keyword_any: AtomicU64,
    pub keyword_all: AtomicU64,
    pub handle: AtomicU64,
    pub guid: Guid,
    pub meta: &'static [u8],
    pub register_status: AtomicU32,
}

impl Provider {
    pub fn new(guid: &[u8; 16], meta: &'static [u8]) -> Provider {
        Provider {
            level_plus1: AtomicU32::new(0),
            keyword_any: AtomicU64::new(0),
            keyword_all: AtomicU64::new(0),
            handle: AtomicU64::new(0),
            guid: Guid::from_le(guid),
            meta,
            register_status: AtomicU32::new(u32::MAX),
        }
    }

    pub fn register(&'static self) -> i32 {
        let mut h = 0u64;
        let st = unsafe { EventRegister(&self.guid, Some(enable_callback), self as *const Provider as *mut c_void, &mut h) };
        self.register_status.store(st, Ordering::SeqCst);
        if st != 0 {
            return if st as i32 > 0 { ((st & 0xffff) | 0x8007_0000) as i32 } else { st as i32 };
        }
        self.handle.store(h, Ordering::SeqCst);
        unsafe { EventSetInformation(h, 2, self.meta.as_ptr() as *const c_void, u16::from_le_bytes([self.meta[0], self.meta[1]]) as u32) };
        0
    }

    pub fn enable(&self, is_enabled: u32, level: u8, any: u64, all: u64) {
        match is_enabled {
            0 => self.level_plus1.store(0, Ordering::SeqCst),
            1 => {
                self.level_plus1.store(if level == 0 { 0x100 } else { level as u32 + 1 }, Ordering::SeqCst);
                self.keyword_any.store(any, Ordering::SeqCst);
                self.keyword_all.store(all, Ordering::SeqCst);
            }
            _ => {}
        }
    }

    pub fn keyword_enabled(&self, kw: u64) -> bool {
        if kw == 0 {
            return true;
        }
        let all = self.keyword_all.load(Ordering::SeqCst);
        self.keyword_any.load(Ordering::SeqCst) & kw != 0 && kw & all == all
    }

    pub fn enabled(&self, level: u8, kw: u64) -> bool {
        self.level_plus1.load(Ordering::SeqCst) > level as u32 && self.keyword_enabled(kw)
    }

    pub fn write(&self, desc: &[u8; 11], meta: &[u8], fields: &[EventData]) -> u32 {
        let ed = EventDescriptor {
            id: 0,
            version: 0,
            channel: desc[0],
            level: desc[1],
            opcode: desc[2],
            task: 0,
            keyword: u64::from_le_bytes([desc[3], desc[4], desc[5], desc[6], desc[7], desc[8], desc[9], desc[10]]),
        };
        let mut data = Vec::with_capacity(fields.len() + 2);
        let psize = u16::from_le_bytes([self.meta[0], self.meta[1]]) as usize;
        let esize = u16::from_le_bytes([meta[0], meta[1]]) as usize;
        data.push(EventData::bytes(&self.meta[..psize], 2));
        data.push(EventData::bytes(&meta[..esize], 1));
        data.extend_from_slice(fields);
        unsafe { EventWriteTransfer(self.handle.load(Ordering::SeqCst), &ed, core::ptr::null(), core::ptr::null(), data.len() as u32, data.as_ptr()) }
    }
}

unsafe extern "system" fn enable_callback(_src: *const Guid, is_enabled: u32, level: u8, any: u64, all: u64, _filter: *const c_void, ctx: *mut c_void) {
    if ctx.is_null() {
        return;
    }
    (*(ctx as *const Provider)).enable(is_enabled, level, any, all);
}

pub fn activity() -> &'static Provider {
    static P: OnceLock<Provider> = OnceLock::new();
    static R: OnceLock<i32> = OnceLock::new();
    let p = P.get_or_init(|| Provider::new(ACTIVITY_GUID, ACTIVITY_META));
    R.get_or_init(|| p.register());
    p
}

pub fn state() -> &'static Provider {
    static P: OnceLock<Provider> = OnceLock::new();
    static R: OnceLock<i32> = OnceLock::new();
    let p = P.get_or_init(|| Provider::new(STATE_GUID, STATE_META));
    R.get_or_init(|| p.register());
    p
}

pub fn watermark() -> &'static Provider {
    static P: OnceLock<Provider> = OnceLock::new();
    static R: OnceLock<i32> = OnceLock::new();
    let p = P.get_or_init(|| Provider::new(WATERMARK_GUID, WATERMARK_META));
    R.get_or_init(|| p.register());
    p
}
