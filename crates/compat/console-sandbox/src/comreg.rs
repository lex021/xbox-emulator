use core::ffi::c_void;
use std::sync::{Mutex, MutexGuard};

pub const S_OK: i32 = 0;
pub const E_INVALIDARG: i32 = 0x8007_0057u32 as i32;
pub const E_POINTER: i32 = 0x8000_4003u32 as i32;
pub const E_OUTOFMEMORY: i32 = 0x8007_000Eu32 as i32;
pub const E_NOINTERFACE: i32 = 0x8000_4002u32 as i32;
pub const CLASS_E_NOAGGREGATION: i32 = 0x8004_0110u32 as i32;
pub const CO_E_OBJISREG: i32 = 0x8004_01FBu32 as i32;
pub const REGDB_E_CLASSNOTREG: i32 = 0x8004_0154u32 as i32;
pub const CO_E_ERRORINAPP: i32 = 0x8000_4019u32 as i32;

pub const CLSCTX_INPROC_SERVER: u32 = 0x1;
pub const CLSCTX_LOCAL_SERVER: u32 = 0x4;
pub const REGCLS_SINGLEUSE: u32 = 0;
pub const REGCLS_MULTIPLEUSE: u32 = 1;
pub const REGCLS_MULTI_SEPARATE: u32 = 2;
pub const REGCLS_SUSPENDED: u32 = 4;

pub const IID_IUNKNOWN: [u8; 16] = [0, 0, 0, 0, 0, 0, 0, 0, 0xC0, 0, 0, 0, 0, 0, 0, 0x46];
pub const IID_ICLASSFACTORY: [u8; 16] = [1, 0, 0, 0, 0, 0, 0, 0, 0xC0, 0, 0, 0, 0, 0, 0, 0x46];

#[link(name = "ole32")]
extern "system" {
    fn CoCreateInstance(clsid: *const [u8; 16], outer: *mut c_void, ctx: u32, iid: *const [u8; 16], out: *mut *mut c_void) -> i32;
    fn CoGetClassObject(clsid: *const [u8; 16], ctx: u32, info: *mut c_void, iid: *const [u8; 16], out: *mut *mut c_void) -> i32;
}

#[link(name = "runtimeobject")]
extern "system" {
    fn RoGetActivationFactory(id: *mut c_void, iid: *const [u8; 16], out: *mut *mut c_void) -> i32;
    fn WindowsGetStringRawBuffer(h: *mut c_void, len: *mut u32) -> *const u16;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClassEntry {
    pub cookie: u32,
    pub clsid: [u8; 16],
    pub object: usize,
    pub context: u32,
    pub flags: u32,
    pub suspended: bool,
    pub used: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FactoryEntry {
    pub cookie: usize,
    pub class: String,
    pub callback: usize,
}

#[derive(Default)]
struct Table {
    classes: Vec<ClassEntry>,
    factories: Vec<FactoryEntry>,
    next_cookie: u32,
    next_factory: usize,
}

fn table() -> MutexGuard<'static, Table> {
    static T: Mutex<Table> = Mutex::new(Table { classes: Vec::new(), factories: Vec::new(), next_cookie: 0, next_factory: 0 });
    T.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn classes() -> Vec<ClassEntry> {
    table().classes.clone()
}

pub fn factories() -> Vec<FactoryEntry> {
    table().factories.clone()
}

unsafe fn vcall<F>(obj: usize, slot: usize) -> F {
    let vt = (obj as *const usize).read();
    core::mem::transmute_copy(&((vt + slot * 8) as *const usize).read())
}

pub unsafe fn add_ref(obj: usize) -> u32 {
    let f: unsafe extern "system" fn(usize) -> u32 = vcall(obj, 1);
    f(obj)
}

pub unsafe fn release(obj: usize) -> u32 {
    let f: unsafe extern "system" fn(usize) -> u32 = vcall(obj, 2);
    f(obj)
}

pub unsafe fn query_interface(obj: usize, iid: &[u8; 16], out: *mut *mut c_void) -> i32 {
    let f: unsafe extern "system" fn(usize, *const [u8; 16], *mut *mut c_void) -> i32 = vcall(obj, 0);
    f(obj, iid, out)
}

unsafe fn guid(p: *const [u8; 16]) -> [u8; 16] {
    p.read_unaligned()
}

pub unsafe extern "system" fn co_register_class_object(clsid: *const [u8; 16], unk: *mut c_void, ctx: u32, flags: u32, cookie: *mut u32) -> i32 {
    if clsid.is_null() || unk.is_null() || cookie.is_null() {
        return E_INVALIDARG;
    }
    if flags & !(REGCLS_MULTIPLEUSE | REGCLS_MULTI_SEPARATE | REGCLS_SUSPENDED | 8 | 0x10 | 0x20) != 0 {
        return E_INVALIDARG;
    }
    let c = guid(clsid);
    let registered = table().classes.iter().any(|e| e.clsid == c && e.context & ctx != 0 && !(e.flags & 3 == REGCLS_SINGLEUSE && e.used));
    if registered && flags & REGCLS_MULTIPLEUSE == 0 {
        return CO_E_OBJISREG;
    }
    add_ref(unk as usize);
    let mut t = table();
    t.next_cookie += 1;
    let k = t.next_cookie;
    t.classes.push(ClassEntry { cookie: k, clsid: c, object: unk as usize, context: ctx, flags, suspended: flags & REGCLS_SUSPENDED != 0, used: false });
    *cookie = k;
    S_OK
}

pub unsafe extern "system" fn co_revoke_class_object(cookie: u32) -> i32 {
    let e = {
        let mut t = table();
        match t.classes.iter().position(|e| e.cookie == cookie) {
            Some(i) => t.classes.remove(i),
            None => return E_INVALIDARG,
        }
    };
    release(e.object);
    S_OK
}

pub unsafe extern "system" fn co_resume_class_objects() -> i32 {
    for e in table().classes.iter_mut() {
        e.suspended = false;
    }
    S_OK
}

fn take_class(clsid: &[u8; 16], ctx: u32, local: bool) -> Option<usize> {
    let mut t = table();
    let e = t.classes.iter_mut().find(|e| &e.clsid == clsid && e.context & ctx != 0 && !(local && e.suspended) && !(e.flags & 3 == REGCLS_SINGLEUSE && e.used))?;
    if e.flags & 3 == REGCLS_SINGLEUSE {
        e.used = true;
    }
    Some(e.object)
}

pub unsafe fn class_object(clsid: &[u8; 16], ctx: u32, iid: &[u8; 16], out: *mut *mut c_void) -> Option<i32> {
    let obj = take_class(clsid, ctx, ctx & CLSCTX_INPROC_SERVER == 0)?;
    Some(query_interface(obj, iid, out))
}

pub unsafe extern "system" fn co_get_class_object(clsid: *const [u8; 16], ctx: u32, info: *mut c_void, iid: *const [u8; 16], out: *mut *mut c_void) -> i32 {
    if clsid.is_null() || iid.is_null() || out.is_null() {
        return E_INVALIDARG;
    }
    *out = core::ptr::null_mut();
    match class_object(&guid(clsid), ctx, &guid(iid), out) {
        Some(r) => r,
        None => CoGetClassObject(clsid, ctx, info, iid, out),
    }
}

pub unsafe fn create_from_table(clsid: &[u8; 16], outer: *mut c_void, ctx: u32, iid: &[u8; 16], out: *mut *mut c_void) -> Option<i32> {
    let mut cf: *mut c_void = core::ptr::null_mut();
    let r = class_object(clsid, ctx, &IID_ICLASSFACTORY, &mut cf)?;
    if r < 0 {
        return Some(r);
    }
    let f: unsafe extern "system" fn(usize, *mut c_void, *const [u8; 16], *mut *mut c_void) -> i32 = vcall(cf as usize, 3);
    let hr = f(cf as usize, outer, iid, out);
    release(cf as usize);
    Some(hr)
}

pub unsafe extern "system" fn co_create_instance(clsid: *const [u8; 16], outer: *mut c_void, ctx: u32, iid: *const [u8; 16], out: *mut *mut c_void) -> i32 {
    if out.is_null() {
        return E_POINTER;
    }
    *out = core::ptr::null_mut();
    if clsid.is_null() || iid.is_null() {
        return E_INVALIDARG;
    }
    match create_from_table(&guid(clsid), outer, ctx, &guid(iid), out) {
        Some(r) => r,
        None => CoCreateInstance(clsid, outer, ctx, iid, out),
    }
}

unsafe fn hstring_text(h: *mut c_void) -> String {
    if h.is_null() {
        return String::new();
    }
    let mut n = 0u32;
    let p = WindowsGetStringRawBuffer(h, &mut n);
    if p.is_null() {
        return String::new();
    }
    String::from_utf16_lossy(std::slice::from_raw_parts(p, n as usize))
}

pub unsafe extern "system" fn ro_register_activation_factories(ids: *const *mut c_void, callbacks: *const usize, count: u32, cookie: *mut usize) -> i32 {
    if ids.is_null() || callbacks.is_null() || cookie.is_null() || count == 0 {
        return E_INVALIDARG;
    }
    let names: Vec<String> = (0..count as usize).map(|i| hstring_text(*ids.add(i))).collect();
    if names.iter().any(|n| n.is_empty()) || (0..count as usize).any(|i| *callbacks.add(i) == 0) {
        return E_INVALIDARG;
    }
    let mut t = table();
    if names.iter().any(|n| t.factories.iter().any(|f| &f.class == n)) {
        return CO_E_OBJISREG;
    }
    t.next_factory += 1;
    let k = t.next_factory;
    for (i, n) in names.into_iter().enumerate() {
        t.factories.push(FactoryEntry { cookie: k, class: n, callback: *callbacks.add(i) });
    }
    *cookie = k;
    S_OK
}

pub unsafe extern "system" fn ro_revoke_activation_factories(cookie: usize) {
    table().factories.retain(|f| f.cookie != cookie);
}

pub unsafe fn activation_factory(class: &str, id: *mut c_void, iid: &[u8; 16], out: *mut *mut c_void) -> Option<i32> {
    let cb = table().factories.iter().find(|f| f.class == class).map(|f| f.callback)?;
    let f: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> i32 = core::mem::transmute(cb);
    let mut fac: *mut c_void = core::ptr::null_mut();
    let hr = f(id, &mut fac);
    if hr < 0 {
        return Some(hr);
    }
    if fac.is_null() {
        return Some(CO_E_ERRORINAPP);
    }
    let r = query_interface(fac as usize, iid, out);
    release(fac as usize);
    Some(r)
}

pub unsafe extern "system" fn ro_get_activation_factory(id: *mut c_void, iid: *const [u8; 16], out: *mut *mut c_void) -> i32 {
    if out.is_null() || iid.is_null() {
        return E_INVALIDARG;
    }
    *out = core::ptr::null_mut();
    match activation_factory(&hstring_text(id), id, &guid(iid), out) {
        Some(r) => r,
        None => RoGetActivationFactory(id, iid, out),
    }
}
