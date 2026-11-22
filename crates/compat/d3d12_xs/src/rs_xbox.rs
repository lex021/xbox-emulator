use core::ffi::c_void;

use crate::console_slots_mesh::{rts0_range, RTS0_FLAGS_OFFSET};

pub const XBOX_FORCE_MEMORY_BASED_ABI: u32 = 0x0100_0000;
pub const XBOX_RAYTRACING: u32 = XBOX_FORCE_MEMORY_BASED_ABI;
pub const XBOX_FORCE_COMPUTE_2_SPILL_AREA: u32 = 0x0200_0000;
pub const XBOX_INTERNAL_XDXR: u32 = XBOX_FORCE_MEMORY_BASED_ABI | XBOX_FORCE_COMPUTE_2_SPILL_AREA;
pub const XBOX_DRIVER_DISCARDED: u32 = 0x8000_0000;
pub const XBOX_ROOT_FLAGS: u32 = XBOX_INTERNAL_XDXR | XBOX_DRIVER_DISCARDED;
pub const CONSOLE_UNSUPPORTED: u32 = 0x7cff_f000;

pub const CS_USER_SGPRS: u32 = 32;
pub const GFX_USER_SGPRS: u32 = 16;
pub const CS_SPILL_BASE: u32 = 0x380;
pub const GFX_SPILL_BASE: u32 = 0x1c0;
pub const GFX_SPILL_BASE_COMPUTE_2: u32 = 0x700;

pub const DESC_FLAGS: usize = 32;
pub const VERSIONED_DESC_FLAGS: usize = 8 + DESC_FLAGS;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct XboxRoot(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UserData {
    Sgpr(u32),
    Spill(u32),
}

impl XboxRoot {
    pub fn memory_based_abi(self) -> bool {
        self.0 & XBOX_FORCE_MEMORY_BASED_ABI != 0
    }

    pub fn compute_2_spill_area(self) -> bool {
        self.0 & XBOX_FORCE_COMPUTE_2_SPILL_AREA != 0
    }

    pub fn internal_xdxr(self) -> bool {
        self.0 & XBOX_INTERNAL_XDXR == XBOX_INTERNAL_XDXR
    }

    pub fn console_supported(self) -> bool {
        self.0 & CONSOLE_UNSUPPORTED == 0
    }

    pub fn pc_flags(self) -> u32 {
        self.0 & !XBOX_ROOT_FLAGS
    }

    pub fn xbox_bits(self) -> u32 {
        self.0 & XBOX_ROOT_FLAGS
    }

    pub fn driver_flags(self) -> u32 {
        self.0 & !XBOX_DRIVER_DISCARDED
    }

    pub fn gfx_spill_base(self) -> u32 {
        if self.compute_2_spill_area() {
            GFX_SPILL_BASE_COMPUTE_2
        } else {
            GFX_SPILL_BASE
        }
    }

    pub fn user_data(self, compute: bool, slot: u32) -> UserData {
        if compute {
            if slot < CS_USER_SGPRS { UserData::Sgpr(slot) } else { UserData::Spill(CS_SPILL_BASE + slot * 4) }
        } else if slot < GFX_USER_SGPRS {
            UserData::Sgpr(slot)
        } else {
            UserData::Spill(self.gfx_spill_base() + slot * 4)
        }
    }
}

pub fn blob_flags(blob: &[u8]) -> Option<(usize, u32)> {
    let (o, n) = rts0_range(blob)?;
    if n < RTS0_FLAGS_OFFSET + 4 {
        return None;
    }
    let at = o + RTS0_FLAGS_OFFSET;
    Some((at, u32::from_le_bytes(blob.get(at..at + 4)?.try_into().ok()?)))
}

pub fn set_blob_flags(blob: &mut [u8], flags: u32) -> bool {
    let Some((at, _)) = blob_flags(blob) else { return false };
    blob[at..at + 4].copy_from_slice(&flags.to_le_bytes());
    if blob.get(0..4) == Some(b"DXBC") {
        blob[4..20].fill(0);
    }
    true
}

pub fn pc_blob(blob: &[u8]) -> Option<Vec<u8>> {
    let (_, f) = blob_flags(blob)?;
    let x = XboxRoot(f);
    if x.xbox_bits() == 0 {
        return None;
    }
    let mut b = blob.to_vec();
    set_blob_flags(&mut b, x.pc_flags());
    Some(b)
}

pub unsafe fn pc_blob_raw(blob: *const u8, len: usize) -> Option<Vec<u8>> {
    if blob.is_null() || len == 0 {
        return None;
    }
    pc_blob(unsafe { core::slice::from_raw_parts(blob, len) })
}

unsafe fn vslot(o: *mut c_void, i: usize) -> usize {
    unsafe { *(*(o as *const *const usize)).add(i) }
}

pub unsafe fn restore_blob(blob: *mut c_void, xbox: u32) -> bool {
    if blob.is_null() || xbox == 0 {
        return false;
    }
    let ptr: unsafe extern "system" fn(*mut c_void) -> *mut u8 = unsafe { core::mem::transmute(vslot(blob, 3)) };
    let size: unsafe extern "system" fn(*mut c_void) -> usize = unsafe { core::mem::transmute(vslot(blob, 4)) };
    let (p, n) = unsafe { (ptr(blob), size(blob)) };
    if p.is_null() || n == 0 {
        return false;
    }
    let b = unsafe { core::slice::from_raw_parts_mut(p, n) };
    let Some((_, f)) = blob_flags(b) else { return false };
    set_blob_flags(b, f | xbox)
}

unsafe fn read_flags(desc: *const u8, at: usize) -> u32 {
    unsafe { core::ptr::read_unaligned(desc.add(at) as *const u32) }
}

unsafe fn or_flags(desc: *const u8, at: usize, bits: u32) {
    if !desc.is_null() && bits != 0 {
        let p = unsafe { desc.add(at) } as *mut u32;
        unsafe { core::ptr::write_unaligned(p, core::ptr::read_unaligned(p) | bits) };
    }
}

pub type SerializeFn = unsafe extern "system" fn(*const c_void, u32, *mut *mut c_void, *mut *mut c_void) -> i32;
pub type SerializeVersionedFn = unsafe extern "system" fn(*const c_void, *mut *mut c_void, *mut *mut c_void) -> i32;
pub type DeserializerFn = unsafe extern "system" fn(*const c_void, usize, *const c_void, *mut *mut c_void) -> i32;

pub unsafe fn serialize(f: SerializeFn, desc: *const c_void, version: u32, ppblob: *mut *mut c_void, pperror: *mut *mut c_void) -> i32 {
    if desc.is_null() {
        return unsafe { f(desc, version, ppblob, pperror) };
    }
    let x = XboxRoot(unsafe { read_flags(desc as *const u8, DESC_FLAGS) });
    if x.xbox_bits() == 0 {
        return unsafe { f(desc, version, ppblob, pperror) };
    }
    let mut d = unsafe { core::slice::from_raw_parts(desc as *const u8, DESC_FLAGS + 4) }.to_vec();
    d[DESC_FLAGS..DESC_FLAGS + 4].copy_from_slice(&x.pc_flags().to_le_bytes());
    let hr = unsafe { f(d.as_ptr() as *const c_void, version, ppblob, pperror) };
    if hr >= 0 && !ppblob.is_null() {
        unsafe { restore_blob(*ppblob, x.xbox_bits()) };
    }
    hr
}

pub unsafe fn serialize_versioned(f: SerializeVersionedFn, desc: *const c_void, ppblob: *mut *mut c_void, pperror: *mut *mut c_void) -> i32 {
    if desc.is_null() {
        return unsafe { f(desc, ppblob, pperror) };
    }
    let x = XboxRoot(unsafe { read_flags(desc as *const u8, VERSIONED_DESC_FLAGS) });
    if x.xbox_bits() == 0 {
        return unsafe { f(desc, ppblob, pperror) };
    }
    let mut d = unsafe { core::slice::from_raw_parts(desc as *const u8, VERSIONED_DESC_FLAGS + 4) }.to_vec();
    d[VERSIONED_DESC_FLAGS..VERSIONED_DESC_FLAGS + 4].copy_from_slice(&x.pc_flags().to_le_bytes());
    let hr = unsafe { f(d.as_ptr() as *const c_void, ppblob, pperror) };
    if hr >= 0 && !ppblob.is_null() {
        unsafe { restore_blob(*ppblob, x.xbox_bits()) };
    }
    hr
}

pub unsafe fn deserializer(f: DeserializerFn, versioned: bool, data: *const c_void, size: usize, riid: *const c_void, pp: *mut *mut c_void) -> i32 {
    let Some(pc) = (unsafe { pc_blob_raw(data as *const u8, size) }) else {
        return unsafe { f(data, size, riid, pp) };
    };
    let xbox = blob_flags(unsafe { core::slice::from_raw_parts(data as *const u8, size) }).map(|(_, fl)| XboxRoot(fl).xbox_bits()).unwrap_or(0);
    let hr = unsafe { f(pc.as_ptr() as *const c_void, pc.len(), riid, pp) };
    if hr >= 0 && !pp.is_null() && !unsafe { *pp }.is_null() {
        let d = unsafe { *pp };
        if versioned {
            let get: unsafe extern "system" fn(*mut c_void) -> *const u8 = unsafe { core::mem::transmute(vslot(d, 4)) };
            unsafe { or_flags(get(d), VERSIONED_DESC_FLAGS, xbox) };
            unsafe { restore_versions(d, xbox) };
        } else {
            let get: unsafe extern "system" fn(*mut c_void) -> *const u8 = unsafe { core::mem::transmute(vslot(d, 3)) };
            unsafe { or_flags(get(d), DESC_FLAGS, xbox) };
        }
    }
    hr
}

pub const ROOT_SIGNATURE_VERSIONS: [u32; 3] = [1, 2, 3];

unsafe fn restore_versions(o: *mut c_void, xbox: u32) {
    let at_version: unsafe extern "system" fn(*mut c_void, u32, *mut *const u8) -> i32 = unsafe { core::mem::transmute(vslot(o, 3)) };
    for v in ROOT_SIGNATURE_VERSIONS {
        let mut desc: *const u8 = core::ptr::null();
        if unsafe { at_version(o, v, &mut desc) } >= 0 && !desc.is_null() {
            let p = unsafe { desc.add(VERSIONED_DESC_FLAGS) } as *mut u32;
            unsafe { core::ptr::write_unaligned(p, core::ptr::read_unaligned(p) | xbox) };
        }
    }
}
