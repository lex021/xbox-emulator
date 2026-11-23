use core::ffi::c_void;
use std::sync::{Mutex, MutexGuard};

use crate::wstr::{from_ptr, wide};

pub const RPC_S_OK: i32 = 0;
pub const RPC_S_INVALID_ARG: i32 = 87;
pub const RPC_S_OUT_OF_MEMORY: i32 = 14;
pub const RPC_S_PROTSEQ_NOT_SUPPORTED: i32 = 1703;
pub const RPC_S_INVALID_RPC_PROTSEQ: i32 = 1704;
pub const RPC_S_DUPLICATE_ENDPOINT: i32 = 1740;
pub const EPT_S_NOT_REGISTERED: i32 = 1753;
pub const RPC_PROTSEQ_LRPC: u32 = 3;
pub const RPC_BHT_OBJECT_UUID_VALID: u32 = 1;

#[link(name = "rpcrt4")]
extern "system" {
    fn RpcBindingToStringBindingW(b: *mut c_void, out: *mut *mut u16) -> i32;
    fn RpcStringBindingParseW(s: *const u16, obj: *mut *mut u16, protseq: *mut *mut u16, addr: *mut *mut u16, ep: *mut *mut u16, opts: *mut *mut u16) -> i32;
    fn RpcStringFreeW(s: *mut *mut u16) -> i32;
    fn RpcBindingCreateW(template: *const u8, security: *const c_void, options: *const c_void, out: *mut *mut c_void) -> i32;
    fn RpcBindingBind(async_: *mut c_void, b: *mut c_void, ifspec: *const c_void) -> i32;
    fn RpcBindingFree(b: *mut *mut c_void) -> i32;
    fn RpcStringBindingComposeW(obj: *const u16, protseq: *const u16, addr: *const u16, ep: *const u16, opts: *const u16, out: *mut *mut u16) -> i32;
    fn RpcBindingFromStringBindingW(s: *const u16, out: *mut *mut c_void) -> i32;
    fn UuidFromStringW(s: *const u16, out: *mut [u8; 16]) -> i32;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub interface: [u8; 16],
    pub major: u16,
    pub minor: u16,
    pub object: [u8; 16],
    pub protseq: String,
    pub endpoint: String,
    pub annotation: String,
}

#[derive(Default)]
struct Map {
    entries: Vec<Entry>,
    dynamic_bindings: Vec<(usize, [u8; 16])>,
    dynamic_listening: bool,
    calls: Vec<String>,
}

fn map() -> MutexGuard<'static, Map> {
    static M: Mutex<Map> = Mutex::new(Map { entries: Vec::new(), dynamic_bindings: Vec::new(), dynamic_listening: false, calls: Vec::new() });
    M.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn prefix() -> String {
    format!("TomBox.{}.", std::process::id())
}

pub fn namespaced(ep: &str) -> String {
    let p = prefix();
    if ep.starts_with(&p) {
        ep.to_string()
    } else {
        format!("{p}{ep}")
    }
}

pub fn dynamic_endpoint() -> String {
    format!("{}LRPC", prefix())
}

pub fn entries() -> Vec<Entry> {
    map().entries.clone()
}

pub fn log() -> Vec<String> {
    map().calls.clone()
}

fn note(s: String) {
    map().calls.push(s);
}

pub fn resolve(interface: &[u8; 16], major: u16, object: &[u8; 16]) -> Option<Entry> {
    let nil = [0u8; 16];
    let m = map();
    m.entries.iter().find(|e| &e.interface == interface && e.major == major && &e.object == object).or_else(|| if object == &nil { None } else { m.entries.iter().find(|e| &e.interface == interface && e.major == major && e.object == nil) }).cloned()
}

fn is_lrpc(p: &str) -> bool {
    p.eq_ignore_ascii_case("ncalrpc")
}

pub unsafe extern "system" fn rpc_server_use_protseq_w(protseq: *const u16, max: u32, sd: *mut c_void) -> i32 {
    let p = from_ptr(protseq);
    let r = crate::rpcsrv::rpc_server_use_protseq_w(protseq, max, sd);
    note(format!("RpcServerUseProtseqW {p} = {r} (in-process)"));
    if r == RPC_S_OK && is_lrpc(&p) {
        map().dynamic_listening = true;
    }
    r
}

pub unsafe extern "system" fn rpc_server_use_protseq_ep_w(protseq: *const u16, max: u32, ep: *const u16, sd: *mut c_void) -> i32 {
    let r = crate::rpcsrv::rpc_server_use_protseq_ep_w(protseq, max, ep, sd);
    note(format!("RpcServerUseProtseqEpW {} {} = {r} (in-process)", from_ptr(protseq), from_ptr(ep)));
    r
}

unsafe fn guid_at(p: *const u8) -> [u8; 16] {
    let mut g = [0u8; 16];
    if !p.is_null() {
        core::ptr::copy_nonoverlapping(p, g.as_mut_ptr(), 16);
    }
    g
}

unsafe fn interface_of(ifspec: *const c_void) -> ([u8; 16], u16, u16) {
    let p = ifspec as *const u8;
    (guid_at(p.add(4)), (p.add(20) as *const u16).read_unaligned(), (p.add(22) as *const u16).read_unaligned())
}

unsafe fn binding_parts(b: *mut c_void) -> Option<(String, String)> {
    if let Some((ps, ep, _)) = crate::rpcsrv::binding_parts(b as usize) {
        return Some((ps, ep));
    }
    let mut s: *mut u16 = core::ptr::null_mut();
    if RpcBindingToStringBindingW(b, &mut s) != RPC_S_OK {
        return None;
    }
    let mut ps: *mut u16 = core::ptr::null_mut();
    let mut ep: *mut u16 = core::ptr::null_mut();
    let r = RpcStringBindingParseW(s, core::ptr::null_mut(), &mut ps, core::ptr::null_mut(), &mut ep, core::ptr::null_mut());
    let out = if r == RPC_S_OK { Some((from_ptr(ps), from_ptr(ep))) } else { None };
    RpcStringFreeW(&mut ps);
    RpcStringFreeW(&mut ep);
    RpcStringFreeW(&mut s);
    out
}

unsafe fn objects(uv: *const u8) -> Vec<[u8; 16]> {
    if uv.is_null() {
        return vec![[0u8; 16]];
    }
    let n = (uv as *const u32).read_unaligned() as usize;
    (0..n).map(|i| guid_at((uv.add(8 + i * 8) as *const *const u8).read_unaligned())).collect()
}

unsafe fn register(ifspec: *const c_void, bv: *const u8, uv: *const u8, annotation: *const u16, replace: bool) -> i32 {
    if ifspec.is_null() || bv.is_null() {
        return RPC_S_INVALID_ARG;
    }
    let (iface, major, minor) = interface_of(ifspec);
    let n = (bv as *const u32).read_unaligned() as usize;
    let mut eps = Vec::new();
    for i in 0..n {
        let b = (bv.add(8 + i * 8) as *const *mut c_void).read_unaligned();
        if let Some(pe) = binding_parts(b) {
            eps.push(pe);
        }
    }
    let ann = from_ptr(annotation);
    let objs = objects(uv);
    let mut m = map();
    for o in &objs {
        if replace {
            m.entries.retain(|e| !(e.interface == iface && e.major == major && &e.object == o));
        }
        for (ps, ep) in &eps {
            let e = Entry { interface: iface, major, minor, object: *o, protseq: ps.clone(), endpoint: ep.clone(), annotation: ann.clone() };
            if !m.entries.contains(&e) {
                m.entries.push(e);
            }
        }
    }
    m.calls.push(format!("RpcEpRegister {} objects on {:?}", objs.len(), eps));
    RPC_S_OK
}

pub unsafe extern "system" fn rpc_ep_register_w(ifspec: *const c_void, bv: *const u8, uv: *const u8, annotation: *const u16) -> i32 {
    register(ifspec, bv, uv, annotation, true)
}

pub unsafe extern "system" fn rpc_ep_register_no_replace_w(ifspec: *const c_void, bv: *const u8, uv: *const u8, annotation: *const u16) -> i32 {
    register(ifspec, bv, uv, annotation, false)
}

pub unsafe extern "system" fn rpc_ep_unregister(ifspec: *const c_void, bv: *const u8, uv: *const u8) -> i32 {
    if ifspec.is_null() || bv.is_null() {
        return RPC_S_INVALID_ARG;
    }
    let (iface, major, _) = interface_of(ifspec);
    let objs = objects(uv);
    let mut m = map();
    let before = m.entries.len();
    m.entries.retain(|e| !(e.interface == iface && e.major == major && objs.contains(&e.object)));
    if m.entries.len() == before {
        return EPT_S_NOT_REGISTERED;
    }
    RPC_S_OK
}

pub unsafe extern "system" fn rpc_binding_create_w(template: *const u8, security: *const c_void, options: *const c_void, out: *mut *mut c_void) -> i32 {
    if template.is_null() || out.is_null() {
        return RPC_S_INVALID_ARG;
    }
    let protseq = (template.add(8) as *const u32).read_unaligned();
    if protseq != RPC_PROTSEQ_LRPC {
        note(format!("RpcBindingCreateW protseq {protseq} refused: the emulator is offline"));
        return RPC_S_PROTSEQ_NOT_SUPPORTED;
    }
    let mut t = [0u8; 56];
    core::ptr::copy_nonoverlapping(template, t.as_mut_ptr(), 56);
    let ep_ptr = (template.add(24) as *const *const u16).read_unaligned();
    let dynamic = ep_ptr.is_null();
    let ep = wide(&if dynamic { dynamic_endpoint() } else { namespaced(&from_ptr(ep_ptr)) });
    (t.as_mut_ptr().add(24) as *mut *const u16).write_unaligned(ep.as_ptr());
    let r = RpcBindingCreateW(t.as_ptr(), security, options, out);
    if r == RPC_S_OK && dynamic {
        let flags = (template.add(4) as *const u32).read_unaligned();
        let obj = if flags & RPC_BHT_OBJECT_UUID_VALID != 0 { guid_at(template.add(40)) } else { [0u8; 16] };
        map().dynamic_bindings.push((*out as usize, obj));
    }
    r
}

pub unsafe extern "system" fn rpc_binding_bind(async_: *mut c_void, b: *mut c_void, ifspec: *const c_void) -> i32 {
    let dyn_obj = map().dynamic_bindings.iter().find(|(h, _)| *h == b as usize).map(|(_, o)| *o);
    if let (Some(obj), false) = (dyn_obj, ifspec.is_null()) {
        let (iface, major, _) = interface_of(ifspec);
        if resolve(&iface, major, &obj).is_none() {
            note("RpcBindingBind: interface/object not in the emulated endpoint map".into());
            return EPT_S_NOT_REGISTERED;
        }
    }
    RpcBindingBind(async_, b, ifspec)
}

pub unsafe extern "system" fn rpc_binding_free(b: *mut *mut c_void) -> i32 {
    if !b.is_null() {
        let h = *b as usize;
        map().dynamic_bindings.retain(|(x, _)| *x != h);
    }
    RpcBindingFree(b)
}

pub unsafe extern "system" fn rpc_string_binding_compose_w(obj: *const u16, protseq: *const u16, addr: *const u16, ep: *const u16, opts: *const u16, out: *mut *mut u16) -> i32 {
    let p = from_ptr(protseq);
    if is_lrpc(&p) && !ep.is_null() && *ep != 0 {
        let n = wide(&namespaced(&from_ptr(ep)));
        return RpcStringBindingComposeW(obj, protseq, addr, n.as_ptr(), opts, out);
    }
    RpcStringBindingComposeW(obj, protseq, addr, ep, opts, out)
}

pub unsafe extern "system" fn rpc_binding_from_string_binding_w(s: *const u16, out: *mut *mut c_void) -> i32 {
    if s.is_null() || out.is_null() {
        return RPC_S_INVALID_ARG;
    }
    let mut o: *mut u16 = core::ptr::null_mut();
    let mut ps: *mut u16 = core::ptr::null_mut();
    let mut a: *mut u16 = core::ptr::null_mut();
    let mut ep: *mut u16 = core::ptr::null_mut();
    let mut op: *mut u16 = core::ptr::null_mut();
    let r = RpcStringBindingParseW(s, &mut o, &mut ps, &mut a, &mut ep, &mut op);
    if r != RPC_S_OK {
        return r;
    }
    let p = from_ptr(ps);
    let e = from_ptr(ep);
    let obj = from_ptr(o);
    let res = if !is_lrpc(&p) {
        note(format!("RpcBindingFromStringBindingW {p} refused: the emulator is offline"));
        RPC_S_PROTSEQ_NOT_SUPPORTED
    } else {
        let dynamic = e.is_empty();
        let target = wide(&if dynamic { dynamic_endpoint() } else { namespaced(&e) });
        let mut sb: *mut u16 = core::ptr::null_mut();
        let opts = if op.is_null() || *op == 0 { core::ptr::null() } else { op as *const u16 };
        let objp = if o.is_null() || *o == 0 { core::ptr::null() } else { o as *const u16 };
        let r = RpcStringBindingComposeW(objp, ps, core::ptr::null(), target.as_ptr(), opts, &mut sb);
        let r = if r == RPC_S_OK { RpcBindingFromStringBindingW(sb, out) } else { r };
        RpcStringFreeW(&mut sb);
        if r == RPC_S_OK && dynamic {
            let mut g = [0u8; 16];
            if !obj.is_empty() {
                let w = wide(&obj);
                UuidFromStringW(w.as_ptr(), &mut g);
            }
            map().dynamic_bindings.push((*out as usize, g));
        }
        r
    };
    for x in [&mut o, &mut ps, &mut a, &mut ep, &mut op] {
        RpcStringFreeW(x);
    }
    res
}

pub fn is_dynamic_binding(b: usize) -> bool {
    map().dynamic_bindings.iter().any(|(h, _)| *h == b)
}
