use core::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use crate::gcdcom::Guid;

const CLSID_MM_DEVICE_ENUMERATOR: Guid = Guid { data1: 0xBCDE0395, data2: 0xE52F, data3: 0x467C, data4: [0x8E, 0x3D, 0xC4, 0x57, 0x92, 0x91, 0x69, 0x2E] };
const IID_IMM_DEVICE_ENUMERATOR: Guid = Guid { data1: 0xA95664D2, data2: 0x9614, data3: 0x4F35, data4: [0xA7, 0x46, 0xDE, 0x8D, 0xB6, 0x36, 0x17, 0xE6] };
const IID_IAUDIO_CLIENT3: Guid = Guid { data1: 0x7ED4EE07, data2: 0x8E67, data3: 0x4CD4, data4: [0x8C, 0x1A, 0x2B, 0x7A, 0x59, 0x87, 0xAD, 0x42] };
const IID_IAUDIO_CLIENT: Guid = Guid { data1: 0x1CB9AD4C, data2: 0xDBFA, data3: 0x4C32, data4: [0xB1, 0x78, 0xC2, 0xF5, 0x68, 0xA7, 0x03, 0xB2] };
const IID_IAUDIO_RENDER_CLIENT: Guid = Guid { data1: 0xF294ACFC, data2: 0x3146, data3: 0x4483, data4: [0xA7, 0xBF, 0xAD, 0xDC, 0xA7, 0xC2, 0x60, 0xE2] };
const IID_ISIMPLE_AUDIO_VOLUME: Guid = Guid { data1: 0x87CE5498, data2: 0x68D6, data3: 0x44E5, data4: [0x92, 0x15, 0x6D, 0xA4, 0x7E, 0xF8, 0x83, 0xD8] };
const IID_IAUDIO_STREAM_VOLUME: Guid = Guid { data1: 0x93014887, data2: 0x242D, data3: 0x4068, data4: [0x8A, 0x15, 0xCF, 0x5E, 0x93, 0xB9, 0x0F, 0xE3] };
const IID_IAUDIO_ENDPOINT_VOLUME: Guid = Guid { data1: 0x5CDF2C82, data2: 0x841E, data3: 0x4546, data4: [0x97, 0x22, 0x0C, 0xF7, 0x40, 0x78, 0x22, 0x9A] };
const IID_IAUDIO_SESSION_MANAGER: Guid = Guid { data1: 0xBFA971F1, data2: 0x4D5E, data3: 0x40BB, data4: [0x93, 0x5E, 0x96, 0x70, 0x39, 0xBF, 0xBE, 0xE4] };
const IID_IAUDIO_SESSION_CONTROL2: Guid = Guid { data1: 0xBFB7FF88, data2: 0x7239, data3: 0x4FC9, data4: [0x8F, 0xA2, 0x07, 0xC9, 0x50, 0xBE, 0x9C, 0x6D] };
const IID_ISPATIAL_AUDIO_CLIENT: Guid = Guid { data1: 0xBBF8E066, data2: 0xAAAA, data3: 0x49BE, data4: [0x9A, 0x4D, 0xFD, 0x2A, 0x85, 0x8E, 0xA2, 0x7F] };
const IID_ISPATIAL_AUDIO_CLIENT2: Guid = Guid { data1: 0xCAABE452, data2: 0xA66A, data3: 0x4BEE, data4: [0xA9, 0x3E, 0xE3, 0x20, 0x46, 0x3F, 0x6A, 0x53] };
const IID_IAUDIO_CLOCK: Guid = Guid { data1: 0xCD63314F, data2: 0x3FBA, data3: 0x4A1B, data4: [0x81, 0x2C, 0xEF, 0x96, 0x35, 0x87, 0x28, 0xE7] };
const IID_IAUDIO_CAPTURE_CLIENT: Guid = Guid { data1: 0xC8ADBD64, data2: 0xE71E, data3: 0x48A0, data4: [0xA4, 0xDE, 0x18, 0x5C, 0x39, 0x5C, 0xD3, 0x17] };
const IID_IMM_ENDPOINT: Guid = Guid { data1: 0x1BE09788, data2: 0x6894, data3: 0x4089, data4: [0x85, 0x86, 0x9A, 0x2A, 0x6C, 0x26, 0x5A, 0xC5] };
const IID_CLIENT_ITEM_SERVICE: Guid = Guid { data1: 0xD19F8E95, data2: 0xB126, data3: 0x4446, data4: [0x89, 0x0C, 0x5D, 0xCB, 0x7A, 0xD7, 0x14, 0x53] };
const IID_IAUDIO_CLOCK_ADJUSTMENT: Guid = Guid { data1: 0xF6E4C0A0, data2: 0x46D9, data3: 0x4FB8, data4: [0xBE, 0x21, 0x57, 0xA3, 0xEF, 0x2B, 0x62, 0x6C] };
pub const STREAMFLAGS_RATEADJUST: u32 = 0x0010_0000;
pub const AUDIO_CATEGORIES: u32 = 12;
pub const CB_RECORD_INDEX: usize = 0xa0;
pub const CB_FLAGS: usize = 0xa4;
pub const CB_RECORDS: usize = 0x20;
pub const CB_RECORD: usize = 0x28;
pub const CB_BASE_POSITION: usize = 0x70;
pub const CB_BASE_PADDING: usize = 0x80;
pub const CB_BASE_QPC: usize = 0x88;
pub const CB_ANCHOR: usize = 0x98;
pub const FLAG_RECORDS: u32 = 2;
pub const FLAG_POSITION_IS_READ: u32 = 4;
const CLSCTX_ALL: u32 = 0x17;
pub const SHARED: u32 = 0;
const AUTOCONVERT: u32 = 0x8000_0000 | 0x0800_0000;
pub const MAX_FORMAT: usize = 0x48;
pub const ID_CHARS: usize = 0x37;
pub const E_NOINTERFACE: i32 = 0x8000_4002u32 as i32;
pub const E_INVALIDARG: i32 = 0x8007_0057u32 as i32;
pub const E_NOTFOUND: i32 = 0x8007_0490u32 as i32;
pub const AUDCLNT_E_NOT_STOPPED: i32 = 0x8889_0005u32 as i32;
pub const AUDCLNT_E_NOT_INITIALIZED: i32 = 0x8889_0001u32 as i32;

#[link(name = "ole32")]
extern "system" {
    fn CoInitializeEx(r: *mut c_void, f: u32) -> i32;
    fn CoCreateInstance(clsid: *const Guid, outer: *mut c_void, ctx: u32, iid: *const Guid, out: *mut *mut c_void) -> i32;
    fn CoTaskMemFree(p: *mut c_void);
    fn PropVariantClear(p: *mut PropVariant) -> i32;
}

#[link(name = "kernel32")]
extern "system" {
    fn SetEvent(h: usize) -> i32;
    fn Sleep(ms: u32);
    fn CreateEventW(a: *mut c_void, manual: i32, initial: i32, name: *const u16) -> usize;
    fn CloseHandle(h: usize) -> i32;
    fn WaitForSingleObject(h: usize, ms: u32) -> u32;
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct PropVariant {
    pub vt: u16,
    pub r1: u16,
    pub r2: u16,
    pub r3: u16,
    pub a: u64,
    pub b: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct PropertyKey {
    pub fmtid: [u8; 16],
    pub pid: u32,
}

unsafe fn slot<T: Copy>(obj: *mut c_void, n: usize) -> T {
    let vt = *(obj as *const *const usize);
    core::mem::transmute_copy(&*vt.add(n))
}

unsafe fn release(obj: *mut c_void) {
    if !obj.is_null() {
        slot::<unsafe extern "system" fn(*mut c_void) -> u32>(obj, 2)(obj);
    }
}

struct Com(*mut c_void);

impl Drop for Com {
    fn drop(&mut self) {
        unsafe { release(self.0) }
    }
}

pub fn format_len(f: &[u8]) -> usize {
    if f.len() < 0x12 {
        return 0;
    }
    let cb = u16::from_le_bytes([f[0x10], f[0x11]]) as usize;
    (0x12 + cb).min(MAX_FORMAT)
}

pub fn block_align(f: &[u8]) -> u32 {
    u16::from_le_bytes([f[0xc], f[0xd]]) as u32
}

pub fn sample_rate(f: &[u8]) -> u32 {
    u32::from_le_bytes(f[4..8].try_into().unwrap())
}

pub fn channels(f: &[u8]) -> u32 {
    u16::from_le_bytes([f[2], f[3]]) as u32
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

unsafe fn take_wide(p: *mut u16) -> String {
    if p.is_null() {
        return String::new();
    }
    let mut n = 0;
    while *p.add(n) != 0 {
        n += 1;
    }
    let s = String::from_utf16_lossy(core::slice::from_raw_parts(p, n));
    CoTaskMemFree(p as *mut c_void);
    s
}

unsafe fn take_format(f: *mut u8) -> Vec<u8> {
    let head = core::slice::from_raw_parts(f, 0x12);
    let v = core::slice::from_raw_parts(f, format_len(head)).to_vec();
    CoTaskMemFree(f as *mut c_void);
    v
}

unsafe fn enumerator() -> Result<Com, i32> {
    CoInitializeEx(core::ptr::null_mut(), 0);
    let mut en = core::ptr::null_mut();
    let hr = CoCreateInstance(&CLSID_MM_DEVICE_ENUMERATOR, core::ptr::null_mut(), CLSCTX_ALL, &IID_IMM_DEVICE_ENUMERATOR, &mut en);
    if hr < 0 { Err(hr) } else { Ok(Com(en)) }
}

unsafe fn device(id: Option<&str>) -> Result<Com, i32> {
    let en = enumerator()?;
    let mut dev = core::ptr::null_mut();
    let hr = match id {
        Some(s) => {
            let w = wide(s);
            slot::<unsafe extern "system" fn(*mut c_void, *const u16, *mut *mut c_void) -> i32>(en.0, 5)(en.0, w.as_ptr(), &mut dev)
        }
        None => slot::<unsafe extern "system" fn(*mut c_void, u32, u32, *mut *mut c_void) -> i32>(en.0, 4)(en.0, 0, 0, &mut dev),
    };
    if hr < 0 { Err(hr) } else { Ok(Com(dev)) }
}

unsafe fn activate(dev: &Com, iid: &Guid) -> Result<Com, i32> {
    let mut o = core::ptr::null_mut();
    let hr = slot::<unsafe extern "system" fn(*mut c_void, *const Guid, u32, *const c_void, *mut *mut c_void) -> i32>(dev.0, 3)(dev.0, iid, CLSCTX_ALL, core::ptr::null(), &mut o);
    if hr < 0 { Err(hr) } else { Ok(Com(o)) }
}

unsafe fn client(id: Option<&str>) -> Result<Com, i32> {
    let dev = device(id)?;
    activate(&dev, &IID_IAUDIO_CLIENT3).or_else(|_| activate(&dev, &IID_IAUDIO_CLIENT))
}

unsafe fn device_id_of(dev: &Com) -> Result<String, i32> {
    let mut id: *mut u16 = core::ptr::null_mut();
    let hr = slot::<unsafe extern "system" fn(*mut c_void, *mut *mut u16) -> i32>(dev.0, 5)(dev.0, &mut id);
    if hr < 0 || id.is_null() {
        return Err(hr);
    }
    Ok(take_wide(id))
}

pub fn device_id() -> Result<String, i32> {
    unsafe { device_id_of(&device(None)?) }
}

pub fn default_id(flow: u32, role: u32) -> Result<String, i32> {
    unsafe {
        let en = enumerator()?;
        let mut dev = core::ptr::null_mut();
        let hr = slot::<unsafe extern "system" fn(*mut c_void, u32, u32, *mut *mut c_void) -> i32>(en.0, 4)(en.0, flow, role, &mut dev);
        if hr < 0 {
            return Err(hr);
        }
        device_id_of(&Com(dev))
    }
}

pub fn device_state(id: Option<&str>) -> Result<u32, i32> {
    unsafe {
        let dev = device(id)?;
        let mut st = 0u32;
        let hr = slot::<unsafe extern "system" fn(*mut c_void, *mut u32) -> i32>(dev.0, 6)(dev.0, &mut st);
        if hr < 0 { Err(hr) } else { Ok(st) }
    }
}

pub fn endpoints(flow: u32, mask: u32) -> Result<Vec<(String, u32)>, i32> {
    unsafe {
        let en = enumerator()?;
        let mut coll = core::ptr::null_mut();
        let hr = slot::<unsafe extern "system" fn(*mut c_void, u32, u32, *mut *mut c_void) -> i32>(en.0, 3)(en.0, flow, mask, &mut coll);
        if hr < 0 {
            return Err(hr);
        }
        let coll = Com(coll);
        let mut n = 0u32;
        let hr = slot::<unsafe extern "system" fn(*mut c_void, *mut u32) -> i32>(coll.0, 3)(coll.0, &mut n);
        if hr < 0 {
            return Err(hr);
        }
        let mut out = Vec::new();
        for i in 0..n {
            let mut d = core::ptr::null_mut();
            if slot::<unsafe extern "system" fn(*mut c_void, u32, *mut *mut c_void) -> i32>(coll.0, 4)(coll.0, i, &mut d) < 0 {
                continue;
            }
            let d = Com(d);
            let mut st = 0u32;
            slot::<unsafe extern "system" fn(*mut c_void, *mut u32) -> i32>(d.0, 6)(d.0, &mut st);
            if let Ok(id) = device_id_of(&d) {
                out.push((id, st));
            }
        }
        Ok(out)
    }
}

pub struct DeviceInfo {
    pub mix: Vec<u8>,
    pub default_period: i64,
    pub min_period: i64,
}

pub fn device_info_for(id: Option<&str>) -> Result<DeviceInfo, i32> {
    unsafe {
        let c = client(id)?;
        let mut f: *mut u8 = core::ptr::null_mut();
        let hr = slot::<unsafe extern "system" fn(*mut c_void, *mut *mut u8) -> i32>(c.0, 8)(c.0, &mut f);
        if hr < 0 {
            return Err(hr);
        }
        let mix = take_format(f);
        let (mut d, mut m) = (0i64, 0i64);
        let hr = slot::<unsafe extern "system" fn(*mut c_void, *mut i64, *mut i64) -> i32>(c.0, 9)(c.0, &mut d, &mut m);
        if hr < 0 {
            return Err(hr);
        }
        Ok(DeviceInfo { mix, default_period: d, min_period: m })
    }
}

pub fn device_info() -> Result<DeviceInfo, i32> {
    device_info_for(None)
}

pub fn is_format_supported_on(id: Option<&str>, mode: u32, fmt: &[u8]) -> (i32, Option<Vec<u8>>) {
    unsafe {
        let c = match client(id) {
            Ok(c) => c,
            Err(hr) => return (hr, None),
        };
        let mut closest: *mut u8 = core::ptr::null_mut();
        let want = if mode == SHARED { &mut closest as *mut *mut u8 } else { core::ptr::null_mut() };
        let hr = slot::<unsafe extern "system" fn(*mut c_void, u32, *const u8, *mut *mut u8) -> i32>(c.0, 7)(c.0, mode, fmt.as_ptr(), want);
        let near = if closest.is_null() { None } else { Some(take_format(closest)) };
        (hr, near)
    }
}

pub fn is_format_supported(fmt: &[u8]) -> i32 {
    is_format_supported_on(None, SHARED, fmt).0
}

pub fn offload_capable(id: Option<&str>, category: u32) -> Result<u32, i32> {
    unsafe {
        let c = client(id)?;
        let mut b = 0i32;
        let hr = slot::<unsafe extern "system" fn(*mut c_void, u32, *mut i32) -> i32>(c.0, 15)(c.0, category, &mut b);
        if hr < 0 { Err(hr) } else { Ok(b as u32) }
    }
}

pub fn buffer_size_limits(id: Option<&str>, fmt: &[u8], event: bool) -> Result<(i64, i64), i32> {
    unsafe {
        let c = client(id)?;
        let (mut lo, mut hi) = (0i64, 0i64);
        let hr = slot::<unsafe extern "system" fn(*mut c_void, *const u8, i32, *mut i64, *mut i64) -> i32>(c.0, 17)(c.0, fmt.as_ptr(), event as i32, &mut lo, &mut hi);
        if hr < 0 { Err(hr) } else { Ok((lo, hi)) }
    }
}

pub fn shared_mode_engine_period(id: Option<&str>, fmt: &[u8]) -> Result<[u32; 4], i32> {
    unsafe {
        let c = client(id)?;
        let mut v = [0u32; 4];
        let hr = slot::<unsafe extern "system" fn(*mut c_void, *const u8, *mut u32, *mut u32, *mut u32, *mut u32) -> i32>(c.0, 18)(c.0, fmt.as_ptr(), &mut v[0], &mut v[1], &mut v[2], &mut v[3]);
        if hr < 0 { Err(hr) } else { Ok(v) }
    }
}

pub fn current_shared_mode_engine_period(id: Option<&str>) -> Result<(Vec<u8>, u32), i32> {
    unsafe {
        let c = client(id)?;
        let mut f: *mut u8 = core::ptr::null_mut();
        let mut p = 0u32;
        let hr = slot::<unsafe extern "system" fn(*mut c_void, *mut *mut u8, *mut u32) -> i32>(c.0, 19)(c.0, &mut f, &mut p);
        if hr < 0 || f.is_null() {
            return Err(hr);
        }
        Ok((take_format(f), p))
    }
}

pub struct EndpointCaps {
    pub channels: u32,
    pub steps: u32,
    pub min_db: f32,
    pub max_db: f32,
    pub inc_db: f32,
}

pub fn endpoint_caps(id: Option<&str>) -> Result<EndpointCaps, i32> {
    unsafe {
        let dev = device(id)?;
        let v = activate(&dev, &IID_IAUDIO_ENDPOINT_VOLUME)?;
        let mut ch = 0u32;
        let hr = slot::<unsafe extern "system" fn(*mut c_void, *mut u32) -> i32>(v.0, 5)(v.0, &mut ch);
        if hr < 0 {
            return Err(hr);
        }
        let (mut step, mut count) = (0u32, 0u32);
        let hr = slot::<unsafe extern "system" fn(*mut c_void, *mut u32, *mut u32) -> i32>(v.0, 16)(v.0, &mut step, &mut count);
        if hr < 0 {
            return Err(hr);
        }
        let (mut lo, mut hi, mut inc) = (0f32, 0f32, 0f32);
        let hr = slot::<unsafe extern "system" fn(*mut c_void, *mut f32, *mut f32, *mut f32) -> i32>(v.0, 20)(v.0, &mut lo, &mut hi, &mut inc);
        if hr < 0 {
            return Err(hr);
        }
        Ok(EndpointCaps { channels: ch, steps: count, min_db: lo, max_db: hi, inc_db: inc })
    }
}

pub enum Prop {
    Empty,
    Text(Vec<u16>),
    Dword(u32),
    Serialized(u16, Vec<u8>),
    Vector(u16, u32, Vec<u8>),
}

unsafe fn property_store(id: Option<&str>) -> Result<Com, i32> {
    let dev = device(id)?;
    let mut ps = core::ptr::null_mut();
    let hr = slot::<unsafe extern "system" fn(*mut c_void, u32, *mut *mut c_void) -> i32>(dev.0, 4)(dev.0, 0, &mut ps);
    if hr < 0 { Err(hr) } else { Ok(Com(ps)) }
}

pub fn property_count(id: Option<&str>) -> Result<u32, i32> {
    unsafe {
        let ps = property_store(id)?;
        let mut n = 0u32;
        let hr = slot::<unsafe extern "system" fn(*mut c_void, *mut u32) -> i32>(ps.0, 3)(ps.0, &mut n);
        if hr < 0 { Err(hr) } else { Ok(n) }
    }
}

pub fn property_at(id: Option<&str>, index: u32) -> Result<PropertyKey, i32> {
    unsafe {
        let ps = property_store(id)?;
        let mut k = PropertyKey::default();
        let hr = slot::<unsafe extern "system" fn(*mut c_void, u32, *mut PropertyKey) -> i32>(ps.0, 4)(ps.0, index, &mut k);
        if hr < 0 { Err(hr) } else { Ok(k) }
    }
}

pub fn scalar_size(vt: u16) -> Option<usize> {
    match vt {
        2 | 18 | 11 => Some(2),
        3 | 19 | 4 | 10 | 22 | 23 => Some(4),
        5 | 6 | 7 | 20 | 21 | 64 => Some(8),
        16 | 17 => Some(1),
        _ => None,
    }
}

pub fn property_value(id: Option<&str>, key: &PropertyKey) -> Result<Option<Prop>, i32> {
    unsafe {
        let ps = property_store(id)?;
        let mut v: PropVariant = core::mem::zeroed();
        let hr = slot::<unsafe extern "system" fn(*mut c_void, *const PropertyKey, *mut PropVariant) -> i32>(ps.0, 5)(ps.0, key, &mut v);
        if hr < 0 {
            return Err(hr);
        }
        let out = match v.vt {
            0 => Some(Prop::Empty),
            31 => {
                let p = v.a as *const u16;
                let mut n = 0;
                while !p.is_null() && *p.add(n) != 0 {
                    n += 1;
                }
                let mut w = if p.is_null() { Vec::new() } else { core::slice::from_raw_parts(p, n).to_vec() };
                w.push(0);
                Some(Prop::Text(w))
            }
            19 => Some(Prop::Dword(v.a as u32)),
            65 => {
                let cb = v.a as u32 as usize;
                let p = v.b as *const u8;
                Some(Prop::Serialized(65, if p.is_null() { Vec::new() } else { core::slice::from_raw_parts(p, cb).to_vec() }))
            }
            72 => {
                let p = v.a as *const u8;
                Some(Prop::Serialized(72, if p.is_null() { vec![0; 16] } else { core::slice::from_raw_parts(p, 16).to_vec() }))
            }
            30 => {
                let p = v.a as *const u8;
                let mut n = 0;
                while !p.is_null() && *p.add(n) != 0 {
                    n += 1;
                }
                let mut b = if p.is_null() { Vec::new() } else { core::slice::from_raw_parts(p, n).to_vec() };
                b.push(0);
                Some(Prop::Serialized(30, b))
            }
            t if t & 0x1000 != 0 => {
                let count = v.a as u32;
                let p = v.b as *const u8;
                match (t & 0xfff, scalar_size(t & 0xfff)) {
                    (_, Some(n)) => Some(Prop::Vector(t, count, if p.is_null() { Vec::new() } else { core::slice::from_raw_parts(p, n * count as usize).to_vec() })),
                    (31, None) => {
                        let mut b = Vec::new();
                        for i in 0..count as usize {
                            let w = *(p as *const *const u16).add(i);
                            let mut n = 0;
                            while !w.is_null() && *w.add(n) != 0 {
                                n += 1;
                            }
                            if !w.is_null() {
                                b.extend(core::slice::from_raw_parts(w, n).iter().flat_map(|c| c.to_le_bytes()));
                            }
                            b.extend_from_slice(&[0, 0]);
                        }
                        Some(Prop::Vector(t, count, b))
                    }
                    _ => None,
                }
            }
            t => scalar_size(t).map(|n| Prop::Serialized(t, v.a.to_le_bytes()[..n].to_vec())),
        };
        PropVariantClear(&mut v);
        Ok(out)
    }
}

const IID_IAUDIO_SESSION_MANAGER2: Guid = Guid { data1: 0x77AA99A0, data2: 0x1BD6, data3: 0x484F, data4: [0x8B, 0xC7, 0x2C, 0x65, 0x4C, 0x9A, 0x9B, 0x6F] };
const IID_ICHANNEL_AUDIO_VOLUME: Guid = Guid { data1: 0x1C158861, data2: 0xB533, data3: 0x4B30, data4: [0xB1, 0xCF, 0xE8, 0x53, 0xE5, 0x1C, 0x59, 0xB8] };

pub struct PcSession(usize);

unsafe impl Send for PcSession {}
unsafe impl Sync for PcSession {}

impl Drop for PcSession {
    fn drop(&mut self) {
        unsafe { release(self.0 as *mut c_void) }
    }
}

#[derive(Clone)]
pub enum SessionRef {
    Title(Option<String>, Guid),
    Pc(Arc<PcSession>),
}

pub fn session_instance(r: &SessionRef) -> Result<String, i32> {
    session_string(r, 13)
}

pub fn session_enumerate(id: Option<&str>) -> Result<Vec<Arc<PcSession>>, i32> {
    unsafe {
        let dev = device(id)?;
        let mgr = activate(&dev, &IID_IAUDIO_SESSION_MANAGER2)?;
        let mut en = core::ptr::null_mut();
        let hr = slot::<unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> i32>(mgr.0, 5)(mgr.0, &mut en);
        if hr < 0 {
            return Err(hr);
        }
        let en = Com(en);
        let mut n = 0i32;
        let hr = slot::<unsafe extern "system" fn(*mut c_void, *mut i32) -> i32>(en.0, 3)(en.0, &mut n);
        if hr < 0 {
            return Err(hr);
        }
        let mut out = Vec::new();
        for i in 0..n.max(0) {
            let mut c = core::ptr::null_mut();
            let hr = slot::<unsafe extern "system" fn(*mut c_void, i32, *mut *mut c_void) -> i32>(en.0, 4)(en.0, i, &mut c);
            if hr < 0 {
                return Err(hr);
            }
            let c = Com(c);
            let mut c2 = core::ptr::null_mut();
            let hr = slot::<unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> i32>(c.0, 0)(c.0, &IID_IAUDIO_SESSION_CONTROL2, &mut c2);
            if hr < 0 {
                return Err(hr);
            }
            let c2 = PcSession(c2 as usize);
            let mut pid = 0u32;
            let hr = slot::<unsafe extern "system" fn(*mut c_void, *mut u32) -> i32>(c2.0 as *mut c_void, 14)(c2.0 as *mut c_void, &mut pid);
            if hr >= 0 && pid == std::process::id() {
                out.push(Arc::new(c2));
            }
        }
        Ok(out)
    }
}

unsafe fn session_iface(r: &SessionRef, iid: &Guid) -> Result<Com, i32> {
    let c = session_control(r)?;
    let mut v = core::ptr::null_mut();
    let hr = slot::<unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> i32>(c.0, 0)(c.0, iid, &mut v);
    if hr < 0 { Err(hr) } else { Ok(Com(v)) }
}

pub fn session_simple_get(r: &SessionRef) -> Result<(f32, bool), i32> {
    unsafe {
        let v = session_iface(r, &IID_ISIMPLE_AUDIO_VOLUME)?;
        let mut g = 0f32;
        let hr = slot::<unsafe extern "system" fn(*mut c_void, *mut f32) -> i32>(v.0, 4)(v.0, &mut g);
        if hr < 0 {
            return Err(hr);
        }
        let mut m = 0i32;
        let hr = slot::<unsafe extern "system" fn(*mut c_void, *mut i32) -> i32>(v.0, 6)(v.0, &mut m);
        if hr < 0 { Err(hr) } else { Ok((g, m != 0)) }
    }
}

pub fn session_simple_set(r: &SessionRef, volume: Option<f32>, mute: Option<bool>, ctx: &Guid) -> i32 {
    unsafe {
        let v = match session_iface(r, &IID_ISIMPLE_AUDIO_VOLUME) {
            Ok(v) => v,
            Err(hr) => return hr,
        };
        let mut hr = 0;
        if let Some(x) = volume {
            hr = slot::<unsafe extern "system" fn(*mut c_void, f32, *const Guid) -> i32>(v.0, 3)(v.0, x, ctx);
        }
        if let (Some(m), true) = (mute, hr >= 0) {
            hr = slot::<unsafe extern "system" fn(*mut c_void, i32, *const Guid) -> i32>(v.0, 5)(v.0, m as i32, ctx);
        }
        hr
    }
}

pub fn session_channels_get(r: &SessionRef) -> Result<Vec<f32>, i32> {
    unsafe {
        let v = session_iface(r, &IID_ICHANNEL_AUDIO_VOLUME)?;
        let mut n = 0u32;
        let hr = slot::<unsafe extern "system" fn(*mut c_void, *mut u32) -> i32>(v.0, 3)(v.0, &mut n);
        if hr < 0 {
            return Err(hr);
        }
        let mut out = vec![0f32; n as usize];
        let hr = slot::<unsafe extern "system" fn(*mut c_void, u32, *mut f32) -> i32>(v.0, 7)(v.0, n, out.as_mut_ptr());
        if hr < 0 { Err(hr) } else { Ok(out) }
    }
}

pub fn session_channels_set(r: &SessionRef, volumes: &[f32], ctx: &Guid) -> i32 {
    unsafe {
        match session_iface(r, &IID_ICHANNEL_AUDIO_VOLUME) {
            Ok(v) => slot::<unsafe extern "system" fn(*mut c_void, u32, *const f32, *const Guid) -> i32>(v.0, 6)(v.0, volumes.len() as u32, volumes.as_ptr(), ctx),
            Err(hr) => hr,
        }
    }
}

unsafe fn session_control(r: &SessionRef) -> Result<Com, i32> {
    let (id, session) = match r {
        SessionRef::Title(id, g) => (id.as_deref(), g),
        SessionRef::Pc(p) => {
            slot::<unsafe extern "system" fn(*mut c_void) -> u32>(p.0 as *mut c_void, 1)(p.0 as *mut c_void);
            return Ok(Com(p.0 as *mut c_void));
        }
    };
    let dev = device(id)?;
    let mgr = activate(&dev, &IID_IAUDIO_SESSION_MANAGER)?;
    let mut ctl = core::ptr::null_mut();
    let hr = slot::<unsafe extern "system" fn(*mut c_void, *const Guid, u32, *mut *mut c_void) -> i32>(mgr.0, 3)(mgr.0, session, 0, &mut ctl);
    if hr < 0 {
        return Err(hr);
    }
    let ctl = Com(ctl);
    let mut c2 = core::ptr::null_mut();
    let hr = slot::<unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> i32>(ctl.0, 0)(ctl.0, &IID_IAUDIO_SESSION_CONTROL2, &mut c2);
    if hr < 0 { Err(hr) } else { Ok(Com(c2)) }
}

pub fn session_state(r: &SessionRef) -> Result<u32, i32> {
    unsafe {
        let c = session_control(r)?;
        let mut st = 0u32;
        let hr = slot::<unsafe extern "system" fn(*mut c_void, *mut u32) -> i32>(c.0, 3)(c.0, &mut st);
        if hr < 0 { Err(hr) } else { Ok(st) }
    }
}

pub fn session_string(r: &SessionRef, n: usize) -> Result<String, i32> {
    unsafe {
        let c = session_control(r)?;
        let mut p: *mut u16 = core::ptr::null_mut();
        let hr = slot::<unsafe extern "system" fn(*mut c_void, *mut *mut u16) -> i32>(c.0, n)(c.0, &mut p);
        if hr < 0 {
            return Err(hr);
        }
        Ok(take_wide(p))
    }
}

pub fn session_set_display_name(r: &SessionRef, name: &[u16], ctx: &Guid) -> i32 {
    unsafe {
        match session_control(r) {
            Ok(c) => slot::<unsafe extern "system" fn(*mut c_void, *const u16, *const Guid) -> i32>(c.0, 5)(c.0, name.as_ptr(), ctx),
            Err(hr) => hr,
        }
    }
}

pub fn session_grouping(r: &SessionRef) -> Result<Guid, i32> {
    unsafe {
        let c = session_control(r)?;
        let mut g = Guid { data1: 0, data2: 0, data3: 0, data4: [0; 8] };
        let hr = slot::<unsafe extern "system" fn(*mut c_void, *mut Guid) -> i32>(c.0, 8)(c.0, &mut g);
        if hr < 0 { Err(hr) } else { Ok(g) }
    }
}

pub fn session_set_grouping(r: &SessionRef, g: &Guid, ctx: &Guid) -> i32 {
    unsafe {
        match session_control(r) {
            Ok(c) => slot::<unsafe extern "system" fn(*mut c_void, *const Guid, *const Guid) -> i32>(c.0, 9)(c.0, g, ctx),
            Err(hr) => hr,
        }
    }
}

pub fn session_is_system_sounds(r: &SessionRef) -> i32 {
    unsafe {
        match session_control(r) {
            Ok(c) => slot::<unsafe extern "system" fn(*mut c_void) -> i32>(c.0, 15)(c.0),
            Err(hr) => hr,
        }
    }
}

pub fn session_set_ducking(r: &SessionRef, opt_out: bool) -> i32 {
    unsafe {
        match session_control(r) {
            Ok(c) => slot::<unsafe extern "system" fn(*mut c_void, i32) -> i32>(c.0, 16)(c.0, opt_out as i32),
            Err(hr) => hr,
        }
    }
}

pub const SESSION_NOT_FOUND: i32 = 0x8007_0002u32 as i32;
pub const SESSION_KEY_INVALID: i32 = 0x8000_4005u32 as i32;

pub fn session_existing(id: Option<&str>, instance: &str) -> Result<Arc<PcSession>, i32> {
    if instance.is_empty() {
        return Err(SESSION_KEY_INVALID);
    }
    for p in session_enumerate(id)? {
        let r = SessionRef::Pc(p.clone());
        if session_instance(&r)?.eq_ignore_ascii_case(instance) {
            return Ok(p);
        }
    }
    Err(SESSION_NOT_FOUND)
}

#[link(name = "runtimeobject")]
extern "system" {
    fn RoGetActivationFactory(id: *mut c_void, iid: *const Guid, out: *mut *mut c_void) -> i32;
    fn WindowsCreateString(s: *const u16, len: u32, out: *mut *mut c_void) -> i32;
    fn WindowsDeleteString(h: *mut c_void) -> i32;
}

pub const MEDIA_MANAGER_CLASS: &str = "Windows.Media.Internal.MediaManager";
pub const IID_MEDIA_MANAGER_STATICS: Guid = Guid { data1: 0xD6F7BE30, data2: 0x77CD, data3: 0x4139, data4: [0x96, 0xD0, 0xC7, 0xA2, 0x69, 0xBD, 0xCE, 0x79] };
pub const IID_EVENT_HANDLER_OBJECT: Guid = Guid { data1: 0xC50898F6, data2: 0xC536, data3: 0x5F47, data4: [0x85, 0x83, 0x8B, 0x2C, 0x24, 0x38, 0xA1, 0x3B] };
pub const MEDIA_ADD_HANDLER: usize = 6;
pub const MEDIA_REMOVE_HANDLER: usize = 7;
pub const MEDIA_PLAY_TO_STREAM_STATE: usize = 8;
pub const MEDIA_CASTING_APP_STATE: usize = 9;
pub const MEDIA_VOIP_CALL_STATE: usize = 10;

#[repr(C)]
struct Handler {
    vtbl: *const HandlerVtbl,
    refs: AtomicU32,
}

#[repr(C)]
struct HandlerVtbl {
    query: unsafe extern "system" fn(*mut Handler, *const Guid, *mut *mut c_void) -> i32,
    add_ref: unsafe extern "system" fn(*mut Handler) -> u32,
    release: unsafe extern "system" fn(*mut Handler) -> u32,
    invoke: unsafe extern "system" fn(*mut Handler, *mut c_void, *mut c_void) -> i32,
}

pub static HANDLER_INVOKES: AtomicU32 = AtomicU32::new(0);
pub static HANDLER_LIVE: AtomicU32 = AtomicU32::new(0);

unsafe extern "system" fn handler_query(h: *mut Handler, iid: *const Guid, out: *mut *mut c_void) -> i32 {
    if out.is_null() {
        return 0x8000_4003u32 as i32;
    }
    if *iid == IID_IUNKNOWN || *iid == IID_EVENT_HANDLER_OBJECT {
        handler_add_ref(h);
        *out = h as *mut c_void;
        0
    } else {
        *out = core::ptr::null_mut();
        E_NOINTERFACE
    }
}

unsafe extern "system" fn handler_add_ref(h: *mut Handler) -> u32 {
    (*h).refs.fetch_add(1, Ordering::SeqCst) + 1
}

unsafe extern "system" fn handler_release(h: *mut Handler) -> u32 {
    let n = (*h).refs.fetch_sub(1, Ordering::SeqCst) - 1;
    if n == 0 {
        drop(Box::from_raw(h));
        HANDLER_LIVE.fetch_sub(1, Ordering::SeqCst);
    }
    n
}

unsafe extern "system" fn handler_invoke(_h: *mut Handler, _sender: *mut c_void, _args: *mut c_void) -> i32 {
    HANDLER_INVOKES.fetch_add(1, Ordering::SeqCst);
    0
}

static HANDLER_VTBL: HandlerVtbl = HandlerVtbl { query: handler_query, add_ref: handler_add_ref, release: handler_release, invoke: handler_invoke };

pub struct MediaManager(usize);

unsafe impl Send for MediaManager {}
unsafe impl Sync for MediaManager {}

impl Drop for MediaManager {
    fn drop(&mut self) {
        unsafe { release(self.0 as *mut c_void) }
    }
}

pub fn media_manager() -> Result<MediaManager, i32> {
    unsafe {
        CoInitializeEx(core::ptr::null_mut(), 0);
        let name: Vec<u16> = MEDIA_MANAGER_CLASS.encode_utf16().collect();
        let mut h = core::ptr::null_mut();
        let hr = WindowsCreateString(name.as_ptr(), name.len() as u32, &mut h);
        if hr < 0 {
            return Err(hr);
        }
        let mut f = core::ptr::null_mut();
        let hr = RoGetActivationFactory(h, &IID_MEDIA_MANAGER_STATICS, &mut f);
        WindowsDeleteString(h);
        if hr < 0 { Err(hr) } else { Ok(MediaManager(f as usize)) }
    }
}

impl MediaManager {
    pub fn state_changed(&self, method: usize, value: u32) -> i32 {
        unsafe { slot::<unsafe extern "system" fn(*mut c_void, u32) -> i32>(self.0 as *mut c_void, method)(self.0 as *mut c_void, value) }
    }

    pub fn add_handler(&self) -> Result<i64, i32> {
        unsafe {
            let h = Box::into_raw(Box::new(Handler { vtbl: &HANDLER_VTBL, refs: AtomicU32::new(1) }));
            HANDLER_LIVE.fetch_add(1, Ordering::SeqCst);
            let mut token = 0i64;
            let hr = slot::<unsafe extern "system" fn(*mut c_void, *mut c_void, *mut i64) -> i32>(self.0 as *mut c_void, MEDIA_ADD_HANDLER)(self.0 as *mut c_void, h as *mut c_void, &mut token);
            handler_release(h);
            if hr < 0 { Err(hr) } else { Ok(token) }
        }
    }

    pub fn remove_handler(&self, token: i64) -> i32 {
        unsafe { slot::<unsafe extern "system" fn(*mut c_void, i64) -> i32>(self.0 as *mut c_void, MEDIA_REMOVE_HANDLER)(self.0 as *mut c_void, token) }
    }
}

pub fn endpoint_offload(id: Option<&str>) -> Result<u32, i32> {
    let mut any = 0u32;
    for cat in 0..AUDIO_CATEGORIES {
        match offload_capable(id, cat) {
            Ok(b) => any |= (b != 0) as u32,
            Err(E_INVALIDARG) => {}
            Err(hr) => return Err(hr),
        }
    }
    Ok(any)
}

const IID_ISPATIAL_AUDIO_OBJECT_RENDER_STREAM: Guid = Guid { data1: 0xBAB5F473, data2: 0xB423, data3: 0x477B, data4: [0x85, 0xF5, 0xB5, 0xA3, 0x32, 0xA0, 0x41, 0x53] };
const VT_BLOB: u16 = 0x41;
pub const SPATIAL_PARAMS2_BYTES: u32 = 0x2c;
const E_OUTOFMEMORY_HR: i32 = 0x8007_000Eu32 as i32;

pub struct SpatialStreamParams {
    pub format: Vec<u8>,
    pub mask: u32,
    pub min: u32,
    pub max: u32,
    pub category: u32,
    pub options: u32,
    pub extra: SpatialExtra,
}

pub enum SpatialExtra {
    Plain,
    Hrtf { decay: Option<[u8; 0x14]>, directivity: Option<[u8; 0x10]>, environment: Option<u32>, orientation: Option<[u8; 0x24]> },
    Metadata { format_id: [u8; 16], max_items: u32 },
}

const IID_ISPATIAL_AUDIO_OBJECT_RENDER_STREAM_FOR_HRTF: Guid = Guid { data1: 0xE08DEEF9, data2: 0x5363, data3: 0x406E, data4: [0x9F, 0xDC, 0x08, 0x0E, 0xE2, 0x47, 0xBB, 0xE0] };
const IID_ISPATIAL_AUDIO_OBJECT_RENDER_STREAM_FOR_METADATA: Guid = Guid { data1: 0xBBC9C907, data2: 0x48D5, data3: 0x4A2E, data4: [0xA0, 0xC7, 0xF7, 0xF0, 0xD6, 0x7C, 0x1F, 0xB1] };
pub const SPATIAL_HRTF_PARAMS2_BYTES: u32 = 0x4c;
pub const SPATIAL_METADATA_PARAMS2_BYTES: u32 = 0x48;

pub fn spatial_params_blob(p: &SpatialStreamParams, event: usize) -> (Vec<u8>, Guid) {
    let put = |b: &mut Vec<u8>, o: usize, v: &[u8]| b[o..o + v.len()].copy_from_slice(v);
    let size = match p.extra {
        SpatialExtra::Plain => SPATIAL_PARAMS2_BYTES,
        SpatialExtra::Hrtf { .. } => SPATIAL_HRTF_PARAMS2_BYTES,
        SpatialExtra::Metadata { .. } => SPATIAL_METADATA_PARAMS2_BYTES,
    } as usize;
    let mut b = vec![0u8; size];
    put(&mut b, 0, &(p.format.as_ptr() as usize as u64).to_le_bytes());
    put(&mut b, 8, &p.mask.to_le_bytes());
    put(&mut b, 0xc, &p.min.to_le_bytes());
    put(&mut b, 0x10, &p.max.to_le_bytes());
    put(&mut b, 0x14, &p.category.to_le_bytes());
    put(&mut b, 0x18, &(event as u64).to_le_bytes());
    let iid = match &p.extra {
        SpatialExtra::Plain => {
            put(&mut b, 0x28, &p.options.to_le_bytes());
            IID_ISPATIAL_AUDIO_OBJECT_RENDER_STREAM
        }
        SpatialExtra::Hrtf { decay, directivity, environment, orientation } => {
            let ptr = |o: Option<*const u8>| o.map(|x| x as usize as u64).unwrap_or(0).to_le_bytes();
            put(&mut b, 0x28, &ptr(decay.as_ref().map(|d| d.as_ptr())));
            put(&mut b, 0x30, &ptr(directivity.as_ref().map(|d| d.as_ptr())));
            put(&mut b, 0x38, &ptr(environment.as_ref().map(|d| d as *const u32 as *const u8)));
            put(&mut b, 0x40, &ptr(orientation.as_ref().map(|d| d.as_ptr())));
            put(&mut b, 0x48, &p.options.to_le_bytes());
            IID_ISPATIAL_AUDIO_OBJECT_RENDER_STREAM_FOR_HRTF
        }
        SpatialExtra::Metadata { format_id, max_items } => {
            put(&mut b, 0x20, format_id);
            put(&mut b, 0x30, &max_items.to_le_bytes());
            put(&mut b, 0x44, &p.options.to_le_bytes());
            IID_ISPATIAL_AUDIO_OBJECT_RENDER_STREAM_FOR_METADATA
        }
    };
    (b, iid)
}

pub struct SpatialStream {
    com: Com,
    event: usize,
}

unsafe impl Send for SpatialStream {}

impl SpatialStream {
    pub fn available_dynamic(&self) -> Result<u32, i32> {
        unsafe {
            let mut n = 0u32;
            let hr = slot::<unsafe extern "system" fn(*mut c_void, *mut u32) -> i32>(self.com.0, 3)(self.com.0, &mut n);
            if hr < 0 { Err(hr) } else { Ok(n) }
        }
    }
}

impl Drop for SpatialStream {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.event) };
    }
}

unsafe impl Sync for SpatialStream {}

impl SpatialStream {
    pub fn event(&self) -> usize {
        self.event
    }
    unsafe fn call0(&self, n: usize) -> i32 {
        slot::<unsafe extern "system" fn(*mut c_void) -> i32>(self.com.0, n)(self.com.0)
    }
    pub fn start(&self) -> i32 {
        unsafe { self.call0(5) }
    }
    pub fn stop(&self) -> i32 {
        unsafe { self.call0(6) }
    }
    pub fn reset(&self) -> i32 {
        unsafe { self.call0(7) }
    }
    pub fn begin_updating(&self) -> Result<(u32, u32), i32> {
        unsafe {
            let (mut n, mut f) = (0u32, 0u32);
            let hr = slot::<unsafe extern "system" fn(*mut c_void, *mut u32, *mut u32) -> i32>(self.com.0, 8)(self.com.0, &mut n, &mut f);
            if hr < 0 { Err(hr) } else { Ok((n, f)) }
        }
    }
    pub fn end_updating(&self) -> i32 {
        unsafe { self.call0(9) }
    }
    pub fn activate_object(&self, kind: u32) -> Result<SpatialObject, i32> {
        unsafe {
            let mut o = core::ptr::null_mut();
            let hr = slot::<unsafe extern "system" fn(*mut c_void, u32, *mut *mut c_void) -> i32>(self.com.0, 10)(self.com.0, kind, &mut o);
            if hr < 0 || o.is_null() { Err(hr) } else { Ok(SpatialObject(Com(o))) }
        }
    }
}

pub struct SpatialObject(Com);

pub const HRTF_SET_GAIN: usize = 8;
pub const HRTF_SET_ORIENTATION: usize = 9;
pub const HRTF_SET_ENVIRONMENT: usize = 10;
pub const HRTF_SET_DISTANCE_DECAY: usize = 11;
pub const HRTF_SET_DIRECTIVITY: usize = 12;

unsafe impl Send for SpatialObject {}

impl SpatialObject {
    pub fn buffer(&self) -> Result<(*mut u8, u32), i32> {
        unsafe {
            let (mut p, mut n) = (core::ptr::null_mut::<u8>(), 0u32);
            let hr = slot::<unsafe extern "system" fn(*mut c_void, *mut *mut u8, *mut u32) -> i32>(self.0 .0, 3)(self.0 .0, &mut p, &mut n);
            if hr < 0 || p.is_null() { Err(hr) } else { Ok((p, n)) }
        }
    }
    pub fn set_end_of_stream(&self, frames: u32) -> i32 {
        unsafe { slot::<unsafe extern "system" fn(*mut c_void, u32) -> i32>(self.0 .0, 4)(self.0 .0, frames) }
    }
    pub fn set_volume(&self, v: f32) -> i32 {
        unsafe { slot::<unsafe extern "system" fn(*mut c_void, f32) -> i32>(self.0 .0, 8)(self.0 .0, v) }
    }
    pub fn set_hrtf_gain(&self, v: f32) -> i32 {
        unsafe { slot::<unsafe extern "system" fn(*mut c_void, f32) -> i32>(self.0 .0, HRTF_SET_GAIN)(self.0 .0, v) }
    }
    pub fn set_hrtf_environment(&self, v: u32) -> i32 {
        unsafe { slot::<unsafe extern "system" fn(*mut c_void, u32) -> i32>(self.0 .0, HRTF_SET_ENVIRONMENT)(self.0 .0, v) }
    }
    pub unsafe fn set_hrtf_block(&self, n: usize, p: *const u8) -> i32 {
        slot::<unsafe extern "system" fn(*mut c_void, *const u8) -> i32>(self.0 .0, n)(self.0 .0, p)
    }
}

pub unsafe fn com_init() {
    CoInitializeEx(core::ptr::null_mut(), 0);
}

pub fn wait_event(h: usize, ms: u32) -> bool {
    unsafe { WaitForSingleObject(h, ms) == 0 }
}

pub fn signal_event(h: usize) {
    unsafe { SetEvent(h) };
}

pub struct Spatial(Com);

pub fn spatial(id: Option<&str>) -> Result<Spatial, i32> {
    unsafe {
        let dev = device(id)?;
        Ok(Spatial(activate(&dev, &IID_ISPATIAL_AUDIO_CLIENT)?))
    }
}

impl Spatial {
    pub fn native_mask(&self) -> Result<u32, i32> {
        unsafe {
            let mut m = 0u32;
            let hr = slot::<unsafe extern "system" fn(*mut c_void, *mut u32) -> i32>(self.0 .0, 4)(self.0 .0, &mut m);
            if hr < 0 { Err(hr) } else { Ok(m) }
        }
    }
    pub fn max_dynamic(&self) -> Result<u32, i32> {
        unsafe {
            let mut m = 0u32;
            let hr = slot::<unsafe extern "system" fn(*mut c_void, *mut u32) -> i32>(self.0 .0, 5)(self.0 .0, &mut m);
            if hr < 0 { Err(hr) } else { Ok(m) }
        }
    }
    pub fn max_frames(&self, fmt: &[u8]) -> Result<u32, i32> {
        unsafe {
            let mut m = 0u32;
            let hr = slot::<unsafe extern "system" fn(*mut c_void, *const u8, *mut u32) -> i32>(self.0 .0, 7)(self.0 .0, fmt.as_ptr(), &mut m);
            if hr < 0 { Err(hr) } else { Ok(m) }
        }
    }
    pub fn format_supported(&self, fmt: &[u8]) -> i32 {
        unsafe { slot::<unsafe extern "system" fn(*mut c_void, *const u8) -> i32>(self.0 .0, 8)(self.0 .0, fmt.as_ptr()) }
    }
    pub fn stream_available(&self, iid: &Guid) -> i32 {
        unsafe { slot::<unsafe extern "system" fn(*mut c_void, *const Guid, *const c_void) -> i32>(self.0 .0, 9)(self.0 .0, iid, core::ptr::null()) }
    }
    unsafe fn client2(&self) -> Result<Com, i32> {
        let mut c2 = core::ptr::null_mut();
        let hr = slot::<unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> i32>(self.0 .0, 0)(self.0 .0, &IID_ISPATIAL_AUDIO_CLIENT2, &mut c2);
        if hr < 0 { Err(hr) } else { Ok(Com(c2)) }
    }
    pub fn offload_capable(&self, category: u32) -> Result<u32, i32> {
        unsafe {
            let c2 = self.client2()?;
            let mut b = 0i32;
            let hr = slot::<unsafe extern "system" fn(*mut c_void, u32, *mut i32) -> i32>(c2.0, 11)(c2.0, category, &mut b);
            if hr < 0 { Err(hr) } else { Ok(b as u32) }
        }
    }
    pub fn max_frames_for_category(&self, category: u32, offload: bool, fmt: &[u8]) -> Result<u32, i32> {
        unsafe {
            let c2 = self.client2()?;
            let mut n = 0u32;
            let hr = slot::<unsafe extern "system" fn(*mut c_void, u32, i32, *const u8, *mut u32) -> i32>(c2.0, 12)(c2.0, category, offload as i32, fmt.as_ptr(), &mut n);
            if hr < 0 { Err(hr) } else { Ok(n) }
        }
    }
    pub fn activate_stream(&self, p: &SpatialStreamParams) -> Result<SpatialStream, i32> {
        unsafe {
            let event = CreateEventW(core::ptr::null_mut(), 0, 0, core::ptr::null());
            if event == 0 {
                return Err(E_OUTOFMEMORY_HR);
            }
            let (blob, iid) = spatial_params_blob(p, event);
            let mut pv: PropVariant = core::mem::zeroed();
            pv.vt = VT_BLOB;
            let raw = &mut pv as *mut PropVariant as *mut u8;
            core::ptr::write_unaligned(raw.add(8) as *mut u32, blob.len() as u32);
            core::ptr::write_unaligned(raw.add(16) as *mut *const u8, blob.as_ptr());
            let mut st = core::ptr::null_mut();
            let hr = slot::<unsafe extern "system" fn(*mut c_void, *const PropVariant, *const Guid, *mut *mut c_void) -> i32>(self.0 .0, 10)(self.0 .0, &pv, &iid, &mut st);
            if hr < 0 {
                CloseHandle(event);
                return Err(hr);
            }
            Ok(SpatialStream { com: Com(st), event })
        }
    }
    pub fn static_position(&self, kind: u32) -> Result<[f32; 3], i32> {
        unsafe {
            let mut p = [0f32; 3];
            let hr = slot::<unsafe extern "system" fn(*mut c_void, u32, *mut f32, *mut f32, *mut f32) -> i32>(self.0 .0, 3)(self.0 .0, kind, &mut p[0], &mut p[1], &mut p[2]);
            if hr < 0 { Err(hr) } else { Ok(p) }
        }
    }
    unsafe fn formats(&self) -> Result<Com, i32> {
        let mut e = core::ptr::null_mut();
        let hr = slot::<unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> i32>(self.0 .0, 6)(self.0 .0, &mut e);
        if hr < 0 { Err(hr) } else { Ok(Com(e)) }
    }
    pub fn format_enumerator(&self) -> Result<FormatEnumerator, i32> {
        unsafe { self.formats().map(FormatEnumerator) }
    }
    pub fn format_count(&self) -> Result<u32, i32> {
        self.format_enumerator()?.count()
    }
    pub fn format_at(&self, index: u32) -> Result<Vec<u8>, i32> {
        self.format_enumerator()?.get(index)
    }
}

pub struct FormatEnumerator(Com);

unsafe impl Send for FormatEnumerator {}

impl FormatEnumerator {
    pub fn count(&self) -> Result<u32, i32> {
        unsafe {
            let mut n = 0u32;
            let hr = slot::<unsafe extern "system" fn(*mut c_void, *mut u32) -> i32>(self.0 .0, 3)(self.0 .0, &mut n);
            if hr < 0 { Err(hr) } else { Ok(n) }
        }
    }
    pub fn get(&self, index: u32) -> Result<Vec<u8>, i32> {
        unsafe {
            let mut f: *mut u8 = core::ptr::null_mut();
            let hr = slot::<unsafe extern "system" fn(*mut c_void, u32, *mut *mut u8) -> i32>(self.0 .0, 4)(self.0 .0, index, &mut f);
            if hr < 0 || f.is_null() {
                return Err(if hr < 0 { hr } else { 0x8000_4003u32 as i32 });
            }
            let head = core::slice::from_raw_parts(f, 0x12);
            let out = core::slice::from_raw_parts(f, format_len(head)).to_vec();
            CoTaskMemFree(f as *mut c_void);
            Ok(out)
        }
    }
}

pub struct Ring {
    pub base: usize,
    pub start: u32,
    pub end: u32,
    pub align: u32,
}

impl Ring {
    pub fn write_pos(&self) -> &AtomicI64 {
        unsafe { &*((self.base + 0x10) as *const AtomicI64) }
    }
    pub fn read_pos(&self) -> &AtomicI64 {
        unsafe { &*((self.base + 0x18) as *const AtomicI64) }
    }
    pub fn bytes(&self) -> u32 {
        self.end - self.start
    }
    fn word(&self, o: usize) -> &AtomicU32 {
        unsafe { &*((self.base + o) as *const AtomicU32) }
    }
    pub unsafe fn publish_position(&self, frames: u64, qpc: u64, rate: f32, base: u64, anchor: u64) {
        let p = self.base as *mut u8;
        let w64 = |o: usize, v: u64| (p.add(o) as *mut u64).write_volatile(v);
        let cur = self.word(CB_RECORD_INDEX).load(Ordering::Acquire);
        let next = (cur == 0) as u32;
        let r = CB_RECORDS + next as usize * CB_RECORD;
        w64(r, frames);
        w64(r + 8, frames);
        w64(r + 0x10, 0);
        w64(r + 0x18, qpc);
        (p.add(r + 0x20) as *mut f32).write_volatile(rate);
        (p.add(r + 0x24) as *mut u32).write_volatile(1);
        self.word(CB_RECORD_INDEX).swap(next, Ordering::AcqRel);
        if self.word(CB_FLAGS).load(Ordering::Acquire) & FLAG_RECORDS == 0 {
            w64(CB_BASE_POSITION, base);
            w64(CB_BASE_PADDING, 0);
            w64(CB_BASE_QPC, qpc);
            (*((self.base + CB_ANCHOR) as *const AtomicU64)).swap(anchor, Ordering::AcqRel);
            self.word(CB_FLAGS).fetch_or(FLAG_RECORDS | FLAG_POSITION_IS_READ, Ordering::AcqRel);
        }
    }
    pub fn freeze_position(&self, bytes: u64) {
        unsafe { (*((self.base + CB_ANCHOR) as *const AtomicU64)).swap(bytes, Ordering::AcqRel) };
        self.word(CB_FLAGS).fetch_and(!(FLAG_RECORDS | FLAG_POSITION_IS_READ), Ordering::AcqRel);
    }
}

#[derive(Default)]
pub struct Shared {
    pub running: AtomicBool,
    pub quit: AtomicBool,
    pub reset: AtomicBool,
    pub gain: Mutex<Option<f32>>,
    pub channels: Mutex<Option<Vec<f32>>>,
    pub capture: Mutex<Option<Vec<u8>>>,
    pub rendered: AtomicI64,
    pub signals: AtomicI64,
    pub signal: Mutex<Vec<isize>>,
    pub client: AtomicUsize,
    pub latency: AtomicI64,
    pub period: AtomicI64,
    pub rate: AtomicU32,
    pub records: AtomicI64,
    pub rebases: AtomicI64,
}

pub struct Renderer {
    pub shared: Arc<Shared>,
    pub thread: Option<std::thread::JoinHandle<()>>,
}

impl Drop for Renderer {
    fn drop(&mut self) {
        self.shared.quit.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

pub fn muted() -> bool {
    std::env::var("TOMBOX_AUDIO_MUTE").map(|v| v == "1").unwrap_or(false)
}

pub fn capturing() -> bool {
    std::env::var("TOMBOX_AUDIO_CAPTURE").map(|v| v == "1").unwrap_or(false)
}

pub struct Open {
    pub device: Option<String>,
    pub format: Vec<u8>,
    pub duration: i64,
    pub session: Guid,
    pub properties: Option<[u32; 4]>,
    pub flags: u32,
    pub period_frames: Option<u32>,
}

pub fn open(o: Open, ring: Ring) -> Result<(Renderer, u32), i32> {
    let shared = Arc::new(Shared::default());
    if capturing() {
        *shared.capture.lock().unwrap() = Some(Vec::new());
    }
    let (tx, rx) = std::sync::mpsc::channel::<Result<u32, i32>>();
    let s2 = shared.clone();
    let t = std::thread::spawn(move || unsafe { run(o, ring, s2, tx) });
    match rx.recv() {
        Ok(Ok(frames)) => Ok((Renderer { shared, thread: Some(t) }, frames)),
        Ok(Err(hr)) => {
            let _ = t.join();
            Err(hr)
        }
        Err(_) => Err(-2147467259),
    }
}

impl Renderer {
    pub fn stream_latency(&self) -> i64 {
        self.shared.latency.load(Ordering::SeqCst)
    }
    pub fn set_sample_rate(&self, rate: f32) -> i32 {
        unsafe {
            let c = self.shared.client.load(Ordering::SeqCst) as *mut c_void;
            if c.is_null() {
                return AUDCLNT_E_NOT_INITIALIZED;
            }
            let mut adj = core::ptr::null_mut();
            let hr = slot::<unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> i32>(c, 14)(c, &IID_IAUDIO_CLOCK_ADJUSTMENT, &mut adj);
            if hr < 0 {
                return hr;
            }
            let adj = Com(adj);
            let hr = slot::<unsafe extern "system" fn(*mut c_void, f32) -> i32>(adj.0, 3)(adj.0, rate);
            if hr >= 0 {
                self.shared.rate.store(rate.to_bits(), Ordering::SeqCst);
            }
            hr
        }
    }
    unsafe fn item_service(&self) -> Result<Com, i32> {
        let c = self.shared.client.load(Ordering::SeqCst) as *mut c_void;
        if c.is_null() {
            return Err(AUDCLNT_E_NOT_INITIALIZED);
        }
        let mut svc = core::ptr::null_mut();
        let hr = slot::<unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> i32>(c, 14)(c, &IID_CLIENT_ITEM_SERVICE, &mut svc);
        if hr < 0 { Err(hr) } else { Ok(Com(svc)) }
    }
    pub fn item_count(&self) -> Result<u32, i32> {
        unsafe {
            let svc = self.item_service()?;
            let mut n = 0u32;
            let hr = slot::<unsafe extern "system" fn(*mut c_void, *mut u32) -> i32>(svc.0, 3)(svc.0, &mut n);
            if hr < 0 { Err(hr) } else { Ok(n) }
        }
    }
    pub fn item_flag(&self) -> Result<bool, i32> {
        unsafe {
            let svc = self.item_service()?;
            let mut b = 0i32;
            let hr = slot::<unsafe extern "system" fn(*mut c_void, *mut i32) -> i32>(svc.0, 5)(svc.0, &mut b);
            if hr < 0 { Err(hr) } else { Ok(b != 0) }
        }
    }
    pub fn item_value(&self, index: u32) -> Result<u32, i32> {
        unsafe {
            let svc = self.item_service()?;
            let mut ch = core::ptr::null_mut();
            let hr = slot::<unsafe extern "system" fn(*mut c_void, u32, *mut *mut c_void) -> i32>(svc.0, 4)(svc.0, index, &mut ch);
            if hr < 0 {
                return Err(hr);
            }
            let ch = Com(ch);
            let mut v = 0u32;
            let hr = slot::<unsafe extern "system" fn(*mut c_void, *mut u32) -> i32>(ch.0, 3)(ch.0, &mut v);
            if hr < 0 { Err(hr) } else { Ok(v) }
        }
    }
    pub fn current_rate(&self) -> f32 {
        f32::from_bits(self.shared.rate.load(Ordering::SeqCst))
    }
    pub fn records(&self) -> i64 {
        self.shared.records.load(Ordering::SeqCst)
    }
    pub fn rebases(&self) -> i64 {
        self.shared.rebases.load(Ordering::SeqCst)
    }
}

unsafe fn run(o: Open, ring: Ring, s: Arc<Shared>, tx: std::sync::mpsc::Sender<Result<u32, i32>>) {
    let c = match client(o.device.as_deref()) {
        Ok(c) => c,
        Err(hr) => {
            let _ = tx.send(Err(hr));
            return;
        }
    };
    if let Some(p) = o.properties {
        let props = [16u32, p[0], p[1], p[2]];
        slot::<unsafe extern "system" fn(*mut c_void, *const u32) -> i32>(c.0, 16)(c.0, props.as_ptr());
    }
    let flags = o.flags & STREAMFLAGS_RATEADJUST;
    let hr = match o.period_frames {
        Some(p) => {
            let mut c3 = core::ptr::null_mut();
            let hr = slot::<unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> i32>(c.0, 0)(c.0, &IID_IAUDIO_CLIENT3, &mut c3);
            if hr < 0 {
                let _ = tx.send(Err(hr));
                return;
            }
            let c3 = Com(c3);
            slot::<unsafe extern "system" fn(*mut c_void, u32, u32, *const u8, *const Guid) -> i32>(c3.0, 20)(c3.0, flags, p, o.format.as_ptr(), &o.session)
        }
        None => slot::<unsafe extern "system" fn(*mut c_void, u32, u32, i64, i64, *const u8, *const Guid) -> i32>(c.0, 3)(c.0, SHARED, AUTOCONVERT | flags, o.duration, 0, o.format.as_ptr(), &o.session),
    };
    if hr < 0 {
        let _ = tx.send(Err(hr));
        return;
    }
    let mut frames = 0u32;
    slot::<unsafe extern "system" fn(*mut c_void, *mut u32) -> i32>(c.0, 4)(c.0, &mut frames);
    let mut lat = 0i64;
    slot::<unsafe extern "system" fn(*mut c_void, *mut i64) -> i32>(c.0, 5)(c.0, &mut lat);
    s.latency.store(lat, Ordering::SeqCst);
    let (mut dp, mut mp) = (0i64, 0i64);
    slot::<unsafe extern "system" fn(*mut c_void, *mut i64, *mut i64) -> i32>(c.0, 9)(c.0, &mut dp, &mut mp);
    s.period.store(dp.max(10_000), Ordering::SeqCst);
    let mut rc = core::ptr::null_mut();
    let hr = slot::<unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> i32>(c.0, 14)(c.0, &IID_IAUDIO_RENDER_CLIENT, &mut rc);
    if hr < 0 {
        let _ = tx.send(Err(hr));
        return;
    }
    let rc = Com(rc);
    let mut clock = core::ptr::null_mut();
    slot::<unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> i32>(c.0, 14)(c.0, &IID_IAUDIO_CLOCK, &mut clock);
    let clock = Com(clock);
    let mut clock_freq = 0u64;
    if !clock.0.is_null() {
        slot::<unsafe extern "system" fn(*mut c_void, *mut u64) -> i32>(clock.0, 3)(clock.0, &mut clock_freq);
    }
    s.rate.store((sample_rate(&o.format) as f32).to_bits(), Ordering::SeqCst);
    let mut vol = core::ptr::null_mut();
    slot::<unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> i32>(c.0, 14)(c.0, &IID_ISIMPLE_AUDIO_VOLUME, &mut vol);
    let vol = Com(vol);
    let mut sv = core::ptr::null_mut();
    slot::<unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> i32>(c.0, 14)(c.0, &IID_IAUDIO_STREAM_VOLUME, &mut sv);
    let sv = Com(sv);
    if !vol.0.is_null() && muted() {
        slot::<unsafe extern "system" fn(*mut c_void, i32, *const Guid) -> i32>(vol.0, 5)(vol.0, 1, core::ptr::null());
    }
    s.client.store(c.0 as usize, Ordering::SeqCst);
    let _ = tx.send(Ok(frames));
    let align = ring.align.max(1) as i64;
    let rate = sample_rate(&o.format).max(1) as i64;
    let tick = ((frames as i64 * 1000 / rate) / 4).clamp(1, 10) as u32;
    let period = std::time::Duration::from_nanos(s.period.load(Ordering::SeqCst) as u64 * 100);
    let mut next_signal = std::time::Instant::now();
    let mut playing = false;
    let mut last_clock = 0u64;
    let mut base_frames: Option<u64> = None;
    let mut frozen: Option<u64> = None;
    let mut last_read = 0i64;
    let nominal = sample_rate(&o.format) as f32;
    let mut applied_gain: Option<f32> = None;
    let mut applied_channels: Option<Vec<f32>> = None;
    while !s.quit.load(Ordering::SeqCst) {
        let r = ring.read_pos().load(Ordering::Acquire);
        let guest_reset = r < last_read;
        last_read = r;
        if s.reset.swap(false, Ordering::SeqCst) || guest_reset {
            if playing {
                slot::<unsafe extern "system" fn(*mut c_void) -> i32>(c.0, 11)(c.0);
                playing = false;
            }
            slot::<unsafe extern "system" fn(*mut c_void) -> i32>(c.0, 12)(c.0);
            base_frames = None;
            frozen = None;
            ring.freeze_position(0);
            last_clock = 0;
            s.rebases.fetch_add(1, Ordering::SeqCst);
        }
        let want = s.running.load(Ordering::SeqCst);
        if want != playing {
            slot::<unsafe extern "system" fn(*mut c_void) -> i32>(c.0, if want { 10 } else { 11 })(c.0);
            playing = want;
            next_signal = std::time::Instant::now();
            if !playing {
                if let Some(b) = base_frames {
                    let (mut pos, mut qpc) = (0u64, 0u64);
                    slot::<unsafe extern "system" fn(*mut c_void, *mut u64, *mut u64) -> i32>(clock.0, 4)(clock.0, &mut pos, &mut qpc);
                    let f = (pos as u128 * rate as u128 / clock_freq.max(1) as u128) as u64;
                    let read = (ring.read_pos().load(Ordering::Acquire).max(0) as u64) / align as u64;
                    last_clock = pos;
                    let held = f.saturating_sub(b).min(read);
                    frozen = Some(held);
                    ring.freeze_position(held * align as u64);
                }
            }
        }
        let g = *s.gain.lock().unwrap();
        if g != applied_gain && !vol.0.is_null() {
            if let Some(v) = g {
                slot::<unsafe extern "system" fn(*mut c_void, f32, *const Guid) -> i32>(vol.0, 3)(vol.0, v.clamp(0.0, 1.0), core::ptr::null());
            }
            applied_gain = g;
        }
        let ch = s.channels.lock().unwrap().clone();
        if ch != applied_channels && !sv.0.is_null() {
            if let Some(v) = &ch {
                slot::<unsafe extern "system" fn(*mut c_void, u32, *const f32) -> i32>(sv.0, 6)(sv.0, v.len() as u32, v.as_ptr());
            }
            applied_channels = ch;
        }
        if playing {
            pump(c.0, rc.0, frames, &ring, align, &s);
            if clock_freq != 0 {
                let (mut pos, mut qpc) = (0u64, 0u64);
                if slot::<unsafe extern "system" fn(*mut c_void, *mut u64, *mut u64) -> i32>(clock.0, 4)(clock.0, &mut pos, &mut qpc) >= 0 && qpc != 0 && pos != last_clock {
                    last_clock = pos;
                    let f = (pos as u128 * rate as u128 / clock_freq as u128) as u64;
                    let b = *base_frames.get_or_insert(f);
                    let held = frozen.unwrap_or(f.saturating_sub(b));
                    ring.publish_position(f, qpc, nominal, b + held, held * align as u64);
                    s.records.fetch_add(1, Ordering::SeqCst);
                }
            }
            let now = std::time::Instant::now();
            if now >= next_signal {
                for &e in s.signal.lock().unwrap().iter() {
                    SetEvent(e as usize);
                }
                s.signals.fetch_add(1, Ordering::SeqCst);
                next_signal = (next_signal + period).max(now);
            }
        }
        Sleep(tick);
    }
    s.client.store(0, Ordering::SeqCst);
    if playing {
        slot::<unsafe extern "system" fn(*mut c_void) -> i32>(c.0, 11)(c.0);
    }
}

unsafe fn pump(c: *mut c_void, rc: *mut c_void, frames: u32, ring: &Ring, align: i64, s: &Shared) {
    let mut pad = 0u32;
    if slot::<unsafe extern "system" fn(*mut c_void, *mut u32) -> i32>(c, 6)(c, &mut pad) < 0 {
        return;
    }
    let w = ring.write_pos().load(Ordering::Acquire);
    let r = ring.read_pos().load(Ordering::Acquire);
    if w < r || r < 0 {
        return;
    }
    let avail = (w - r) / align;
    let n = avail.min(frames.saturating_sub(pad) as i64);
    if n <= 0 {
        return;
    }
    let mut dst: *mut u8 = core::ptr::null_mut();
    if slot::<unsafe extern "system" fn(*mut c_void, u32, *mut *mut u8) -> i32>(rc, 3)(rc, n as u32, &mut dst) < 0 || dst.is_null() {
        return;
    }
    let total = (n * align) as usize;
    let size = ring.bytes() as usize;
    let mut done = 0usize;
    while done < total {
        let off = (r as usize + done) % size;
        let chunk = (size - off).min(total - done);
        core::ptr::copy_nonoverlapping((ring.base + ring.start as usize + off) as *const u8, dst.add(done), chunk);
        done += chunk;
    }
    if let Some(cap) = s.capture.lock().unwrap().as_mut() {
        cap.extend_from_slice(core::slice::from_raw_parts(dst, total));
    }
    slot::<unsafe extern "system" fn(*mut c_void, u32, u32) -> i32>(rc, 4)(rc, n as u32, 0);
    let _ = ring.read_pos().compare_exchange(r, r + total as i64, Ordering::AcqRel, Ordering::Acquire);
    s.rendered.fetch_add(total as i64, Ordering::SeqCst);
}

pub const CAPTURE_QUEUE_READ: usize = 0;
pub const CAPTURE_QUEUE_WRITE: usize = 4;
pub const CAPTURE_ENTRY: usize = 0x40;
pub const CAPTURE_KIND_DATA: u32 = 1;
pub const CAPTURE_KIND_SILENT: u32 = 2;
pub const CAPTURE_DISCONTINUITY: u32 = 1;
pub const CAPTURE_TIMESTAMP_ERROR: u32 = 4;
const BUFFERFLAGS_SILENT: u32 = 2;

pub fn device_flow(id: Option<&str>) -> Result<u32, i32> {
    unsafe {
        let dev = device(id)?;
        let mut ep = core::ptr::null_mut();
        let hr = slot::<unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> i32>(dev.0, 0)(dev.0, &IID_IMM_ENDPOINT, &mut ep);
        if hr < 0 {
            return Err(hr);
        }
        let ep = Com(ep);
        let mut flow = 0u32;
        let hr = slot::<unsafe extern "system" fn(*mut c_void, *mut u32) -> i32>(ep.0, 3)(ep.0, &mut flow);
        if hr < 0 { Err(hr) } else { Ok(flow) }
    }
}

pub struct CaptureQueue {
    pub base: usize,
    pub queue: usize,
    pub entries: u32,
}

impl CaptureQueue {
    fn index(&self, o: usize) -> &AtomicU32 {
        unsafe { &*((self.base + o) as *const AtomicU32) }
    }
    fn next(&self, i: u32) -> u32 {
        if i + 1 >= self.entries { 0 } else { i + 1 }
    }
    fn entry(&self, i: u32) -> *mut u8 {
        (self.base + self.queue + i as usize * CAPTURE_ENTRY) as *mut u8
    }
    pub fn drop_unread(&self) {
        let r = self.index(CAPTURE_QUEUE_READ).load(Ordering::Acquire);
        self.index(CAPTURE_QUEUE_WRITE).swap(r, Ordering::AcqRel);
    }
    fn oldest_offset(&self) -> Option<u32> {
        let mut r = self.index(CAPTURE_QUEUE_READ).load(Ordering::Acquire);
        let w = self.index(CAPTURE_QUEUE_WRITE).load(Ordering::Acquire);
        while r != w {
            let e = self.entry(r);
            unsafe {
                if (e.add(8) as *const u32).read_volatile() != CAPTURE_KIND_SILENT {
                    return Some((e.add(4) as *const u32).read_volatile());
                }
            }
            r = self.next(r);
        }
        None
    }
    fn place(&self, ring: &Ring, at: u32, bytes: u32) -> Option<u32> {
        let at = if at + bytes > ring.end { ring.start } else { at };
        if at + bytes > ring.end {
            return None;
        }
        match self.oldest_offset() {
            None => Some(at),
            Some(old) if at <= old && at + bytes <= old => Some(at),
            Some(old) if at > old && at + bytes <= ring.end => Some(at),
            _ => None,
        }
    }
    pub unsafe fn post(&self, ring: &Ring, at: &mut u32, data: Option<&[u8]>, frames: u32, devpos: u64, qpc: u64, rate: f32, flags: u32) -> bool {
        let w = self.index(CAPTURE_QUEUE_WRITE).load(Ordering::Acquire);
        let n = self.next(w);
        if n == self.index(CAPTURE_QUEUE_READ).load(Ordering::Acquire) {
            return false;
        }
        let (kind, off) = match data {
            Some(d) => {
                let Some(off) = self.place(ring, *at, d.len() as u32) else {
                    return false;
                };
                core::ptr::copy_nonoverlapping(d.as_ptr(), (ring.base + off as usize) as *mut u8, d.len());
                *at = off + d.len() as u32;
                (CAPTURE_KIND_DATA, off)
            }
            None => (CAPTURE_KIND_SILENT, *at),
        };
        let e = self.entry(w);
        core::ptr::write_bytes(e, 0, CAPTURE_ENTRY);
        (e as *mut u32).write_volatile(1);
        (e.add(4) as *mut u32).write_volatile(off);
        (e.add(8) as *mut u32).write_volatile(kind);
        (e.add(0x10) as *mut u64).write_volatile(devpos);
        (e.add(0x18) as *mut u64).write_volatile(devpos);
        (e.add(0x28) as *mut u64).write_volatile(qpc);
        (e.add(0x30) as *mut f32).write_volatile(rate);
        (e.add(0x34) as *mut u32).write_volatile(flags);
        (e.add(0x38) as *mut u32).write_volatile(frames);
        self.index(CAPTURE_QUEUE_WRITE).swap(n, Ordering::AcqRel);
        ring.write_pos().fetch_add(frames as i64 * ring.align as i64, Ordering::AcqRel);
        true
    }
}

pub fn open_capture(o: Open, ring: Ring, queue: CaptureQueue) -> Result<(Renderer, u32), i32> {
    let shared = Arc::new(Shared::default());
    let (tx, rx) = std::sync::mpsc::channel::<Result<u32, i32>>();
    let s2 = shared.clone();
    let t = std::thread::spawn(move || unsafe { run_capture(o, ring, queue, s2, tx) });
    match rx.recv() {
        Ok(Ok(frames)) => Ok((Renderer { shared, thread: Some(t) }, frames)),
        Ok(Err(hr)) => {
            let _ = t.join();
            Err(hr)
        }
        Err(_) => Err(-2147467259),
    }
}

unsafe fn run_capture(o: Open, ring: Ring, q: CaptureQueue, s: Arc<Shared>, tx: std::sync::mpsc::Sender<Result<u32, i32>>) {
    let c = match client(o.device.as_deref()) {
        Ok(c) => c,
        Err(hr) => {
            let _ = tx.send(Err(hr));
            return;
        }
    };
    if let Some(p) = o.properties {
        let props = [16u32, p[0], p[1], p[2]];
        slot::<unsafe extern "system" fn(*mut c_void, *const u32) -> i32>(c.0, 16)(c.0, props.as_ptr());
    }
    let flags = o.flags & STREAMFLAGS_RATEADJUST;
    let hr = match o.period_frames {
        Some(p) => {
            let mut c3 = core::ptr::null_mut();
            let hr = slot::<unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> i32>(c.0, 0)(c.0, &IID_IAUDIO_CLIENT3, &mut c3);
            if hr < 0 {
                let _ = tx.send(Err(hr));
                return;
            }
            let c3 = Com(c3);
            slot::<unsafe extern "system" fn(*mut c_void, u32, u32, *const u8, *const Guid) -> i32>(c3.0, 20)(c3.0, flags, p, o.format.as_ptr(), &o.session)
        }
        None => slot::<unsafe extern "system" fn(*mut c_void, u32, u32, i64, i64, *const u8, *const Guid) -> i32>(c.0, 3)(c.0, SHARED, AUTOCONVERT | flags, o.duration, 0, o.format.as_ptr(), &o.session),
    };
    if hr < 0 {
        let _ = tx.send(Err(hr));
        return;
    }
    let mut frames = 0u32;
    slot::<unsafe extern "system" fn(*mut c_void, *mut u32) -> i32>(c.0, 4)(c.0, &mut frames);
    let mut lat = 0i64;
    slot::<unsafe extern "system" fn(*mut c_void, *mut i64) -> i32>(c.0, 5)(c.0, &mut lat);
    s.latency.store(lat, Ordering::SeqCst);
    let (mut dp, mut mp) = (0i64, 0i64);
    slot::<unsafe extern "system" fn(*mut c_void, *mut i64, *mut i64) -> i32>(c.0, 9)(c.0, &mut dp, &mut mp);
    s.period.store(dp.max(10_000), Ordering::SeqCst);
    let mut cc = core::ptr::null_mut();
    let hr = slot::<unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> i32>(c.0, 14)(c.0, &IID_IAUDIO_CAPTURE_CLIENT, &mut cc);
    if hr < 0 {
        let _ = tx.send(Err(hr));
        return;
    }
    let cc = Com(cc);
    let nominal = sample_rate(&o.format) as f32;
    s.rate.store(nominal.to_bits(), Ordering::SeqCst);
    s.client.store(c.0 as usize, Ordering::SeqCst);
    let _ = tx.send(Ok(frames));
    let align = ring.align.max(1) as usize;
    let rate = sample_rate(&o.format).max(1) as i64;
    let tick = ((frames as i64 * 1000 / rate) / 4).clamp(1, 10) as u32;
    let mut playing = false;
    let mut at = ring.start;
    let mut lost = false;
    while !s.quit.load(Ordering::SeqCst) {
        let want = s.running.load(Ordering::SeqCst);
        if s.reset.swap(false, Ordering::SeqCst) {
            if playing {
                slot::<unsafe extern "system" fn(*mut c_void) -> i32>(c.0, 11)(c.0);
                playing = false;
            }
            slot::<unsafe extern "system" fn(*mut c_void) -> i32>(c.0, 12)(c.0);
            q.drop_unread();
            at = ring.start;
            lost = false;
            s.rebases.fetch_add(1, Ordering::SeqCst);
        }
        if want != playing {
            slot::<unsafe extern "system" fn(*mut c_void) -> i32>(c.0, if want { 10 } else { 11 })(c.0);
            playing = want;
        }
        let mut posted = false;
        while playing {
            let mut next = 0u32;
            if slot::<unsafe extern "system" fn(*mut c_void, *mut u32) -> i32>(cc.0, 5)(cc.0, &mut next) < 0 || next == 0 {
                break;
            }
            let (mut data, mut n, mut f, mut devpos, mut qpc) = (core::ptr::null_mut::<u8>(), 0u32, 0u32, 0u64, 0u64);
            if slot::<unsafe extern "system" fn(*mut c_void, *mut *mut u8, *mut u32, *mut u32, *mut u64, *mut u64) -> i32>(cc.0, 3)(cc.0, &mut data, &mut n, &mut f, &mut devpos, &mut qpc) != 0 {
                break;
            }
            let bytes = n as usize * align;
            let payload = if f & BUFFERFLAGS_SILENT != 0 || data.is_null() { None } else { Some(core::slice::from_raw_parts(data, bytes)) };
            let pf = (f & (CAPTURE_DISCONTINUITY | CAPTURE_TIMESTAMP_ERROR)) | if lost { CAPTURE_DISCONTINUITY } else { 0 };
            if n != 0 && q.post(&ring, &mut at, payload, n, devpos, qpc, nominal, pf) {
                lost = false;
                posted = true;
                s.records.fetch_add(1, Ordering::SeqCst);
                s.rendered.fetch_add(bytes as i64, Ordering::SeqCst);
            } else if n != 0 {
                lost = true;
            }
            slot::<unsafe extern "system" fn(*mut c_void, u32) -> i32>(cc.0, 4)(cc.0, n);
        }
        if posted {
            for &e in s.signal.lock().unwrap().iter() {
                SetEvent(e as usize);
            }
            s.signals.fetch_add(1, Ordering::SeqCst);
        }
        std::thread::sleep(std::time::Duration::from_millis(tick as u64));
    }
    slot::<unsafe extern "system" fn(*mut c_void) -> i32>(c.0, 11)(c.0);
    s.client.store(0, Ordering::SeqCst);
}

const IID_IUNKNOWN: Guid = Guid { data1: 0, data2: 0, data3: 0, data4: [0xC0, 0, 0, 0, 0, 0, 0, 0x46] };
const IID_IAUDIO_SESSION_EVENTS: Guid = Guid { data1: 0x24918ACC, data2: 0x64B3, data3: 0x37C1, data4: [0x8C, 0xA9, 0x74, 0xA6, 0x6E, 0x99, 0x57, 0xA8] };
const IID_IMM_NOTIFICATION_CLIENT: Guid = Guid { data1: 0x7991EEC9, data2: 0x7E89, data3: 0x4D85, data4: [0x83, 0x90, 0x6C, 0x70, 0x3C, 0xEC, 0x60, 0xC0] };

pub enum PcEvent {
    Device { kind: u32, id: String, a: u32, b: u32, key: Option<PropertyKey> },
    Session { sid: String, kind: u32, ctx: [u8; 16], text: String, guid: [u8; 16], value: u32, volume: f32, volumes: Vec<f32> },
}

static EVENT_SINK: std::sync::OnceLock<fn(PcEvent)> = std::sync::OnceLock::new();

pub fn set_event_sink(f: fn(PcEvent)) {
    let _ = EVENT_SINK.set(f);
}

fn emit(e: PcEvent) {
    if let Some(f) = EVENT_SINK.get() {
        f(e);
    }
}

unsafe fn wide_str(p: *const u16) -> String {
    if p.is_null() {
        return String::new();
    }
    let mut n = 0;
    while *p.add(n) != 0 {
        n += 1;
    }
    String::from_utf16_lossy(core::slice::from_raw_parts(p, n))
}

fn same_guid(a: &Guid, b: &Guid) -> bool {
    a.data1 == b.data1 && a.data2 == b.data2 && a.data3 == b.data3 && a.data4 == b.data4
}

#[repr(C)]
struct Callback {
    vtbl: *const *const (),
    iid: Guid,
    sid: String,
}

unsafe extern "system" fn cb_qi(this: *mut Callback, riid: *const Guid, out: *mut *mut c_void) -> i32 {
    if same_guid(&*riid, &IID_IUNKNOWN) || same_guid(&*riid, &(*this).iid) {
        *out = this as *mut c_void;
        0
    } else {
        *out = core::ptr::null_mut();
        -2147467262
    }
}

unsafe extern "system" fn cb_ref(_this: *mut Callback) -> u32 {
    1
}

unsafe extern "system" fn dev_state(_this: *mut Callback, id: *const u16, state: u32) -> i32 {
    emit(PcEvent::Device { kind: 1, id: wide_str(id), a: state, b: 0, key: None });
    0
}

unsafe extern "system" fn dev_added(_this: *mut Callback, id: *const u16) -> i32 {
    emit(PcEvent::Device { kind: 2, id: wide_str(id), a: 0, b: 0, key: None });
    0
}

unsafe extern "system" fn dev_removed(_this: *mut Callback, id: *const u16) -> i32 {
    emit(PcEvent::Device { kind: 3, id: wide_str(id), a: 0, b: 0, key: None });
    0
}

unsafe extern "system" fn dev_default(_this: *mut Callback, flow: u32, role: u32, id: *const u16) -> i32 {
    emit(PcEvent::Device { kind: 4, id: wide_str(id), a: flow, b: role, key: None });
    0
}

unsafe extern "system" fn dev_property(_this: *mut Callback, id: *const u16, key: *const PropertyKey) -> i32 {
    let k = PropertyKey { fmtid: (*key).fmtid, pid: (*key).pid };
    emit(PcEvent::Device { kind: 5, id: wide_str(id), a: 0, b: 0, key: Some(k) });
    0
}

unsafe fn guid16(p: *const Guid) -> [u8; 16] {
    let mut g = [0u8; 16];
    if !p.is_null() {
        core::ptr::copy_nonoverlapping(p as *const u8, g.as_mut_ptr(), 16);
    }
    g
}

unsafe fn session_emit(this: *mut Callback, kind: u32, ctx: *const Guid, text: String, guid: [u8; 16], value: u32) -> i32 {
    emit(PcEvent::Session { sid: (*this).sid.clone(), kind, ctx: guid16(ctx), text, guid, value, volume: 0.0, volumes: Vec::new() });
    0
}

unsafe extern "system" fn sess_name(this: *mut Callback, name: *const u16, ctx: *const Guid) -> i32 {
    session_emit(this, 0x10, ctx, wide_str(name), [0; 16], 0)
}

unsafe extern "system" fn sess_icon(this: *mut Callback, path: *const u16, ctx: *const Guid) -> i32 {
    session_emit(this, 0x100, ctx, wide_str(path), [0; 16], 0)
}

unsafe extern "system" fn sess_simple(this: *mut Callback, v: f32, m: i32, ctx: *const Guid) -> i32 {
    emit(PcEvent::Session { sid: (*this).sid.clone(), kind: 1, ctx: guid16(ctx), text: String::new(), guid: [0; 16], value: (m != 0) as u32, volume: v, volumes: Vec::new() });
    0
}

unsafe extern "system" fn sess_channel(this: *mut Callback, n: u32, v: *const f32, changed: u32, ctx: *const Guid) -> i32 {
    let volumes = if v.is_null() { Vec::new() } else { core::slice::from_raw_parts(v, n as usize).to_vec() };
    emit(PcEvent::Session { sid: (*this).sid.clone(), kind: 2, ctx: guid16(ctx), text: String::new(), guid: [0; 16], value: changed, volume: 0.0, volumes });
    0
}

unsafe extern "system" fn sess_grouping(this: *mut Callback, g: *const Guid, ctx: *const Guid) -> i32 {
    session_emit(this, 4, ctx, String::new(), guid16(g), 0)
}

unsafe extern "system" fn sess_state(this: *mut Callback, st: u32) -> i32 {
    session_emit(this, 8, core::ptr::null(), String::new(), [0; 16], st)
}

unsafe extern "system" fn sess_disconnected(this: *mut Callback, reason: u32) -> i32 {
    session_emit(this, 0x20, core::ptr::null(), String::new(), [0; 16], reason)
}

struct Vtbl([*const (); 10]);
unsafe impl Sync for Vtbl {}

static DEVICE_VTBL: Vtbl = Vtbl([cb_qi as *const (), cb_ref as *const (), cb_ref as *const (), dev_state as *const (), dev_added as *const (), dev_removed as *const (), dev_default as *const (), dev_property as *const (), core::ptr::null(), core::ptr::null()]);
static SESSION_VTBL: Vtbl = Vtbl([cb_qi as *const (), cb_ref as *const (), cb_ref as *const (), sess_name as *const (), sess_icon as *const (), sess_simple as *const (), sess_channel as *const (), sess_grouping as *const (), sess_state as *const (), sess_disconnected as *const ()]);

fn watch<F: FnOnce() -> Result<(), i32> + Send + 'static>(f: F) -> i32 {
    let (tx, rx) = std::sync::mpsc::channel::<i32>();
    std::thread::spawn(move || unsafe {
        CoInitializeEx(core::ptr::null_mut(), 0);
        let hr = match f() {
            Ok(()) => 0,
            Err(hr) => hr,
        };
        let _ = tx.send(hr);
        if hr >= 0 {
            loop {
                std::thread::park();
            }
        }
    });
    rx.recv().unwrap_or(-2147467259)
}

pub fn watch_devices() -> i32 {
    static DONE: std::sync::OnceLock<i32> = std::sync::OnceLock::new();
    *DONE.get_or_init(|| {
        watch(|| unsafe {
            let en = enumerator()?;
            let cb = Box::leak(Box::new(Callback { vtbl: DEVICE_VTBL.0.as_ptr(), iid: IID_IMM_NOTIFICATION_CLIENT, sid: String::new() }));
            let hr = slot::<unsafe extern "system" fn(*mut c_void, *mut Callback) -> i32>(en.0, 6)(en.0, cb);
            if hr < 0 {
                return Err(hr);
            }
            core::mem::forget(en);
            Ok(())
        })
    })
}

pub fn watch_session(r: &SessionRef) -> Result<String, i32> {
    static DONE: std::sync::OnceLock<Mutex<std::collections::HashMap<String, Result<String, i32>>>> = std::sync::OnceLock::new();
    let map = DONE.get_or_init(|| Mutex::new(std::collections::HashMap::new()));
    let sid = session_string(r, 13)?;
    let key = sid.to_lowercase();
    if let Some(res) = map.lock().unwrap().get(&key) {
        return res.clone();
    }
    let res = Ok(sid).and_then(|sid: String| {
        let r2 = r.clone();
        let reg_sid = sid.clone();
        let hr = watch(move || unsafe {
            let c = session_control(&r2)?;
            let cb = Box::leak(Box::new(Callback { vtbl: SESSION_VTBL.0.as_ptr(), iid: IID_IAUDIO_SESSION_EVENTS, sid: reg_sid }));
            let hr = slot::<unsafe extern "system" fn(*mut c_void, *mut Callback) -> i32>(c.0, 10)(c.0, cb);
            if hr < 0 {
                return Err(hr);
            }
            core::mem::forget(c);
            Ok(())
        });
        if hr < 0 { Err(hr) } else { Ok(sid) }
    });
    map.lock().unwrap().insert(key, res.clone());
    res
}
