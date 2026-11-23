use console_sandbox::comreg;
use core::ffi::c_void;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;

static LOCK: Mutex<()> = Mutex::new(());
static REFS: AtomicU32 = AtomicU32::new(1);
static CREATED: AtomicU32 = AtomicU32::new(0);
static FACTORY_CALLS: AtomicU32 = AtomicU32::new(0);

unsafe extern "system" fn qi(this: usize, iid: *const [u8; 16], out: *mut usize) -> i32 {
    if *iid == comreg::IID_IUNKNOWN || *iid == comreg::IID_ICLASSFACTORY {
        REFS.fetch_add(1, Ordering::SeqCst);
        *out = this;
        0
    } else {
        *out = 0;
        comreg::E_NOINTERFACE
    }
}

unsafe extern "system" fn add(_: usize) -> u32 {
    REFS.fetch_add(1, Ordering::SeqCst) + 1
}

unsafe extern "system" fn rel(_: usize) -> u32 {
    REFS.fetch_sub(1, Ordering::SeqCst) - 1
}

unsafe extern "system" fn create(this: usize, outer: usize, iid: *const [u8; 16], out: *mut usize) -> i32 {
    if outer != 0 {
        return comreg::CLASS_E_NOAGGREGATION;
    }
    CREATED.fetch_add(1, Ordering::SeqCst);
    qi(this, iid, out)
}

fn object() -> usize {
    let vt: &'static [usize; 5] = Box::leak(Box::new([qi as *const () as usize, add as *const () as usize, rel as *const () as usize, create as *const () as usize, 0]));
    Box::leak(Box::new([vt.as_ptr() as usize])).as_ptr() as usize
}

unsafe extern "system" fn get_factory(_id: *mut c_void, out: *mut usize) -> i32 {
    FACTORY_CALLS.fetch_add(1, Ordering::SeqCst);
    static F: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    let f = *F.get_or_init(object);
    REFS.fetch_add(1, Ordering::SeqCst);
    *out = f;
    0
}

#[link(name = "runtimeobject")]
extern "system" {
    fn WindowsCreateString(s: *const u16, n: u32, out: *mut *mut c_void) -> i32;
    fn WindowsDeleteString(s: *mut c_void) -> i32;
}

#[test]
fn class_objects_serve_in_process_with_single_use_suspension_and_duplicates() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    unsafe {
        let obj = object();
        let multi = [0x61u8; 16];
        let mut c1 = 0u32;
        assert_eq!(comreg::co_register_class_object(&multi, obj as *mut _, comreg::CLSCTX_LOCAL_SERVER, comreg::REGCLS_MULTIPLEUSE | comreg::REGCLS_SUSPENDED, &mut c1), 0);
        let mut out: *mut c_void = core::ptr::null_mut();
        assert!(comreg::create_from_table(&multi, core::ptr::null_mut(), comreg::CLSCTX_LOCAL_SERVER, &comreg::IID_IUNKNOWN, &mut out).is_none());
        assert_eq!(comreg::co_resume_class_objects(), 0);
        assert_eq!(comreg::co_create_instance(&multi, core::ptr::null_mut(), comreg::CLSCTX_LOCAL_SERVER, &comreg::IID_IUNKNOWN, &mut out), 0);
        assert_eq!(out as usize, obj);
        comreg::release(obj);
        let mut cf: *mut c_void = core::ptr::null_mut();
        assert_eq!(comreg::co_get_class_object(&multi, comreg::CLSCTX_LOCAL_SERVER, core::ptr::null_mut(), &comreg::IID_ICLASSFACTORY, &mut cf), 0);
        assert_eq!(cf as usize, obj);
        comreg::release(obj);
        assert_eq!(comreg::co_create_instance(&multi, 1usize as *mut _, comreg::CLSCTX_LOCAL_SERVER, &comreg::IID_IUNKNOWN, &mut out), comreg::CLASS_E_NOAGGREGATION);
        let mut c2 = 0u32;
        assert_eq!(comreg::co_register_class_object(&multi, obj as *mut _, comreg::CLSCTX_LOCAL_SERVER, comreg::REGCLS_MULTIPLEUSE, &mut c2), 0);
        assert_ne!(c1, c2);

        let single = [0x62u8; 16];
        let mut c3 = 0u32;
        assert_eq!(comreg::co_register_class_object(&single, obj as *mut _, comreg::CLSCTX_INPROC_SERVER, comreg::REGCLS_SINGLEUSE, &mut c3), 0);
        let mut c4 = 0u32;
        assert_eq!(comreg::co_register_class_object(&single, obj as *mut _, comreg::CLSCTX_INPROC_SERVER, comreg::REGCLS_SINGLEUSE, &mut c4), comreg::CO_E_OBJISREG);
        assert_eq!(comreg::co_create_instance(&single, core::ptr::null_mut(), comreg::CLSCTX_INPROC_SERVER, &comreg::IID_IUNKNOWN, &mut out), 0);
        comreg::release(obj);
        assert!(comreg::create_from_table(&single, core::ptr::null_mut(), comreg::CLSCTX_INPROC_SERVER, &comreg::IID_IUNKNOWN, &mut out).is_none());
        assert_eq!(CREATED.load(Ordering::SeqCst), 2);

        for c in [c1, c2, c3] {
            assert_eq!(comreg::co_revoke_class_object(c), 0);
        }
        assert_eq!(comreg::co_revoke_class_object(c1), comreg::E_INVALIDARG);
        assert_eq!(REFS.load(Ordering::SeqCst), 1);
        assert_eq!(comreg::co_register_class_object(core::ptr::null(), obj as *mut _, 4, 1, &mut c1), comreg::E_INVALIDARG);
    }
}

#[test]
fn activation_factories_resolve_through_the_registered_callback() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    unsafe {
        let name: Vec<u16> = "TomBox.Test.ActivatableClass".encode_utf16().collect();
        let mut h: *mut c_void = core::ptr::null_mut();
        assert_eq!(WindowsCreateString(name.as_ptr(), name.len() as u32, &mut h), 0);
        let ids = [h];
        let cbs = [get_factory as *const () as usize];
        let mut cookie = 0usize;
        assert_eq!(comreg::ro_register_activation_factories(ids.as_ptr(), cbs.as_ptr(), 1, &mut cookie), 0);
        let mut again = 0usize;
        assert_eq!(comreg::ro_register_activation_factories(ids.as_ptr(), cbs.as_ptr(), 1, &mut again), comreg::CO_E_OBJISREG);
        let before = REFS.load(Ordering::SeqCst);
        let mut out: *mut c_void = core::ptr::null_mut();
        assert_eq!(comreg::ro_get_activation_factory(h, &comreg::IID_IUNKNOWN, &mut out), 0);
        assert!(!out.is_null());
        assert_eq!(FACTORY_CALLS.load(Ordering::SeqCst), 1);
        assert_eq!(REFS.load(Ordering::SeqCst), before + 1);
        comreg::release(out as usize);
        let bad = [7u8; 16];
        assert_eq!(comreg::ro_get_activation_factory(h, &bad, &mut out), comreg::E_NOINTERFACE);
        comreg::ro_revoke_activation_factories(cookie);
        assert!(comreg::factories().iter().all(|f| f.cookie != cookie));
        assert!(comreg::activation_factory("TomBox.Test.ActivatableClass", h, &comreg::IID_IUNKNOWN, &mut out).is_none());
        assert_eq!(comreg::ro_register_activation_factories(ids.as_ptr(), cbs.as_ptr(), 0, &mut cookie), comreg::E_INVALIDARG);
        WindowsDeleteString(h);
    }
}
