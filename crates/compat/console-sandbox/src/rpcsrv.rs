use core::ffi::c_void;
use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::sync::{Condvar, Mutex, MutexGuard, OnceLock};

use crate::wstr::{from_ptr, wide};

pub const RPC_S_OK: i32 = 0;
pub const RPC_S_ACCESS_DENIED: i32 = 5;
pub const RPC_S_OUT_OF_MEMORY: i32 = 14;
pub const RPC_S_INVALID_ARG: i32 = 87;
pub const ERROR_MORE_DATA: i32 = 234;
pub const RPC_S_INVALID_BINDING: i32 = 1702;
pub const RPC_S_PROTSEQ_NOT_SUPPORTED: i32 = 1703;
pub const RPC_S_INVALID_RPC_PROTSEQ: i32 = 1704;
pub const RPC_S_INVALID_ENDPOINT_FORMAT: i32 = 1706;
pub const RPC_S_TYPE_ALREADY_REGISTERED: i32 = 1711;
pub const RPC_S_UNKNOWN_IF: i32 = 1717;
pub const RPC_S_NO_BINDINGS: i32 = 1718;
pub const RPC_S_SERVER_TOO_BUSY: i32 = 1723;
pub const RPC_S_NO_CALL_ACTIVE: i32 = 1725;
pub const RPC_S_DUPLICATE_ENDPOINT: i32 = 1740;
pub const RPC_S_UNKNOWN_AUTHN_SERVICE: i32 = 1747;
pub const RPC_S_PROCNUM_OUT_OF_RANGE: i32 = 1745;
pub const RPC_S_CALL_IN_PROGRESS: i32 = 1791;
pub const RPC_S_INVALID_ASYNC_HANDLE: i32 = 1914;

pub const RPC_IF_AUTOLISTEN: u32 = 0x1;
pub const RPC_IF_ALLOW_SECURE_ONLY: u32 = 0x8;
pub const RPC_IF_ALLOW_CALLBACKS_WITH_NO_AUTH: u32 = 0x10;
pub const RPC_IF_ALLOW_LOCAL_ONLY: u32 = 0x20;

pub const RPC_C_AUTHN_NONE: u32 = 0;
pub const RPC_C_AUTHN_GSS_NEGOTIATE: u32 = 9;
pub const RPC_C_AUTHN_WINNT: u32 = 10;
pub const RPC_C_AUTHN_GSS_SCHANNEL: u32 = 14;
pub const RPC_C_AUTHN_GSS_KERBEROS: u32 = 16;
pub const RPC_C_AUTHN_DEFAULT: u32 = 0xffff_ffff;
pub const RPC_C_AUTHN_LEVEL_NONE: u32 = 1;
pub const RPC_C_AUTHN_LEVEL_PKT_PRIVACY: u32 = 6;

pub const RPC_PROTSEQ_LRPC: u32 = 3;
pub const RPC_PROTSEQ_HVSOCKET: u32 = 5;

pub const RPC_QUERY_SERVER_PRINCIPAL_NAME: u32 = 0x2;
pub const RPC_QUERY_CLIENT_PRINCIPAL_NAME: u32 = 0x4;
pub const RPC_QUERY_CLIENT_PID: u32 = 0x10;
pub const RPC_QUERY_IS_CLIENT_LOCAL: u32 = 0x20;
pub const RPC_QUERY_CLIENT_ID: u32 = 0x80;

pub const RCCL_LOCAL: u32 = 1;
pub const RCCL_REMOTE: u32 = 2;
pub const RPC_CALL_STATUS_IN_PROGRESS: u32 = 1;
pub const RPC_CALL_STATUS_CANCELLED: u32 = 2;
pub const RPC_CALL_STATUS_DISCONNECTED: u32 = 3;
pub const RCT_NORMAL: u32 = 1;

pub const RPC_NOTIFICATION_CLIENT_DISCONNECT: u32 = 1;
pub const RPC_NOTIFICATION_CALL_CANCEL: u32 = 2;
pub const RPC_NOTIFICATION_TYPE_EVENT: u32 = 1;
pub const RPC_NOTIFICATION_TYPE_CALLBACK: u32 = 5;
pub const RPC_ASYNC_EVENT_CLIENT_DISCONNECT: u32 = 3;
pub const RPC_ASYNC_EVENT_CLIENT_CANCEL: u32 = 4;
pub const RPC_ASYNC_STATE_SIZE: u32 = 0x70;
pub const RPC_ASYNC_SIGNATURE: u32 = 0x4352_4153;

pub const DEFAULT_PRINCIPAL: &str = "NT AUTHORITY\\LOCAL SERVICE";

const HANDLE_MAGIC: u64 = 0x4e49_5652_4553_4254;

#[link(name = "rpcrt4")]
extern "system" {
    fn RpcBindingToStringBindingW(b: *mut c_void, out: *mut *mut u16) -> i32;
    fn RpcBindingVectorFree(v: *mut *mut c_void) -> i32;
    fn RpcStringFreeW(s: *mut *mut u16) -> i32;
    fn RpcAsyncCompleteCall(a: *mut c_void, reply: *mut c_void) -> i32;
    fn RpcAsyncAbortCall(a: *mut c_void, code: u32) -> i32;
    fn UuidToStringW(u: *const [u8; 16], out: *mut *mut u16) -> i32;
}

#[link(name = "kernel32")]
extern "system" {
    fn GetProcessHeap() -> *mut c_void;
    fn HeapAlloc(h: *mut c_void, flags: u32, n: usize) -> *mut c_void;
    fn HeapFree(h: *mut c_void, flags: u32, p: *mut c_void) -> i32;
    fn SetEvent(h: usize) -> i32;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Endpoint {
    pub protseq: String,
    pub endpoint: String,
    pub max_calls: u32,
    pub dynamic: bool,
    pub security: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthInfo {
    pub principal: String,
    pub service: u32,
    pub get_key: usize,
    pub arg: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Interface {
    pub ifspec: usize,
    pub uuid: [u8; 16],
    pub major: u16,
    pub minor: u16,
    pub mgr_type: [u8; 16],
    pub epv: usize,
    pub flags: u32,
    pub max_calls: u32,
    pub max_rpc_size: u32,
    pub callback: usize,
    pub security: usize,
    pub active: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Client {
    pub protseq: String,
    pub endpoint: String,
    pub local: bool,
    pub pid: u32,
    pub principal: String,
    pub authn_service: u32,
    pub authn_level: u32,
    pub client_id: Vec<u8>,
    pub object: [u8; 16],
}

impl Client {
    pub fn lrpc(endpoint: &str) -> Self {
        Client { protseq: "ncalrpc".into(), endpoint: endpoint.into(), local: true, pid: std::process::id(), principal: DEFAULT_PRINCIPAL.into(), authn_service: RPC_C_AUTHN_WINNT, authn_level: RPC_C_AUTHN_LEVEL_PKT_PRIVACY, client_id: Vec::new(), object: [0; 16] }
    }

    pub fn hvsocket(endpoint: &str, vm_id: [u8; 16]) -> Self {
        Client { protseq: "ncacn_hvsocket".into(), endpoint: endpoint.into(), local: false, pid: 0, principal: String::new(), authn_service: RPC_C_AUTHN_NONE, authn_level: RPC_C_AUTHN_LEVEL_NONE, client_id: vm_id.to_vec(), object: [0; 16] }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Server(usize),
    Call(u64),
}

struct Subscription {
    kind: u32,
    info: [usize; 4],
}

struct Call {
    client: Client,
    iface: [u8; 16],
    opnum: u16,
    status: u32,
    async_state: usize,
    reply: Option<Result<u32, u32>>,
    subs: HashMap<u32, Subscription>,
    queued: u32,
}

#[derive(Default)]
struct State {
    endpoints: Vec<Endpoint>,
    auth: Vec<AuthInfo>,
    interfaces: Vec<Interface>,
    handles: HashMap<usize, Kind>,
    calls: HashMap<u64, Call>,
    vectors: HashSet<usize>,
    strings: HashSet<usize>,
    next_call: u64,
    next_dynamic: u32,
}

struct Shared {
    state: Mutex<State>,
    idle: Condvar,
}

fn shared() -> &'static Shared {
    static S: OnceLock<Shared> = OnceLock::new();
    S.get_or_init(|| Shared { state: Mutex::new(State::default()), idle: Condvar::new() })
}

fn lock() -> MutexGuard<'static, State> {
    shared().state.lock().unwrap_or_else(|e| e.into_inner())
}

thread_local! {
    static CURRENT: Cell<usize> = const { Cell::new(0) };
}

pub fn endpoints() -> Vec<Endpoint> {
    lock().endpoints.clone()
}

pub fn auth_info() -> Vec<AuthInfo> {
    lock().auth.clone()
}

pub fn interfaces() -> Vec<Interface> {
    lock().interfaces.clone()
}

pub fn reset() {
    let mut s = lock();
    for (&h, _) in s.handles.iter() {
        unsafe { drop(Box::from_raw(h as *mut [u64; 2])) };
    }
    *s = State::default();
}

fn known_protseq(p: &str) -> bool {
    ["ncalrpc", "ncacn_ip_tcp", "ncacn_np", "ncacn_http", "ncadg_ip_udp", "ncacn_hvsocket"].iter().any(|k| k.eq_ignore_ascii_case(p))
}

fn in_process(p: &str) -> bool {
    p.eq_ignore_ascii_case("ncalrpc") || p.eq_ignore_ascii_case("ncacn_hvsocket")
}

fn protseq_id(p: &str) -> u32 {
    if p.eq_ignore_ascii_case("ncalrpc") {
        RPC_PROTSEQ_LRPC
    } else {
        RPC_PROTSEQ_HVSOCKET
    }
}

fn new_handle(s: &mut State, k: Kind) -> usize {
    let h = Box::into_raw(Box::new([HANDLE_MAGIC, 0u64])) as usize;
    s.handles.insert(h, k);
    h
}

pub fn kind_of(h: usize) -> Option<Kind> {
    if h == 0 {
        return None;
    }
    lock().handles.get(&h).copied()
}

pub fn is_ours(h: usize) -> bool {
    kind_of(h).is_some()
}

pub fn listen(protseq: &str, max_calls: u32, endpoint: Option<&str>, security: usize) -> i32 {
    if !known_protseq(protseq) {
        return RPC_S_INVALID_RPC_PROTSEQ;
    }
    if !in_process(protseq) {
        return RPC_S_PROTSEQ_NOT_SUPPORTED;
    }
    let mut s = lock();
    match endpoint {
        None => {
            if s.endpoints.iter().any(|e| e.dynamic && e.protseq.eq_ignore_ascii_case(protseq)) {
                return RPC_S_OK;
            }
            if protseq.eq_ignore_ascii_case("ncacn_hvsocket") {
                return RPC_S_INVALID_ENDPOINT_FORMAT;
            }
            let ep = if s.next_dynamic == 0 { crate::rpc::dynamic_endpoint() } else { format!("{}{}", crate::rpc::dynamic_endpoint(), s.next_dynamic) };
            s.next_dynamic += 1;
            s.endpoints.push(Endpoint { protseq: protseq.to_ascii_lowercase(), endpoint: ep, max_calls, dynamic: true, security });
            RPC_S_OK
        }
        Some(ep) => {
            if ep.is_empty() {
                return RPC_S_INVALID_ENDPOINT_FORMAT;
            }
            if s.endpoints.iter().any(|e| e.protseq.eq_ignore_ascii_case(protseq) && e.endpoint.eq_ignore_ascii_case(ep)) {
                return RPC_S_DUPLICATE_ENDPOINT;
            }
            s.endpoints.push(Endpoint { protseq: protseq.to_ascii_lowercase(), endpoint: ep.to_string(), max_calls, dynamic: false, security });
            RPC_S_OK
        }
    }
}

pub unsafe extern "system" fn rpc_server_use_protseq_w(protseq: *const u16, max: u32, sd: *mut c_void) -> i32 {
    if protseq.is_null() {
        return RPC_S_INVALID_RPC_PROTSEQ;
    }
    listen(&from_ptr(protseq), max, None, sd as usize)
}

pub unsafe extern "system" fn rpc_server_use_protseq_ep_w(protseq: *const u16, max: u32, ep: *const u16, sd: *mut c_void) -> i32 {
    if protseq.is_null() {
        return RPC_S_INVALID_RPC_PROTSEQ;
    }
    if ep.is_null() {
        return RPC_S_INVALID_ENDPOINT_FORMAT;
    }
    listen(&from_ptr(protseq), max, Some(&from_ptr(ep)), sd as usize)
}

fn known_authn(svc: u32) -> bool {
    matches!(svc, RPC_C_AUTHN_GSS_NEGOTIATE | RPC_C_AUTHN_WINNT | RPC_C_AUTHN_GSS_SCHANNEL | RPC_C_AUTHN_GSS_KERBEROS | RPC_C_AUTHN_DEFAULT)
}

unsafe fn alloc_string(s: &str) -> *mut u16 {
    let w = wide(s);
    let p = HeapAlloc(GetProcessHeap(), 0, w.len() * 2) as *mut u16;
    if p.is_null() {
        return p;
    }
    core::ptr::copy_nonoverlapping(w.as_ptr(), p, w.len());
    lock().strings.insert(p as usize);
    p
}

pub fn owns_string(p: usize) -> bool {
    lock().strings.contains(&p)
}

pub unsafe extern "system" fn rpc_server_inq_default_princ_name_w(svc: u32, out: *mut *mut u16) -> i32 {
    if out.is_null() {
        return RPC_S_INVALID_ARG;
    }
    if !known_authn(svc) || svc == RPC_C_AUTHN_DEFAULT {
        return RPC_S_UNKNOWN_AUTHN_SERVICE;
    }
    let p = alloc_string(DEFAULT_PRINCIPAL);
    if p.is_null() {
        return RPC_S_OUT_OF_MEMORY;
    }
    *out = p;
    RPC_S_OK
}

pub unsafe extern "system" fn rpc_server_register_auth_info_w(principal: *const u16, svc: u32, get_key: *const c_void, arg: *mut c_void) -> i32 {
    if !known_authn(svc) {
        return RPC_S_UNKNOWN_AUTHN_SERVICE;
    }
    let mut s = lock();
    let a = AuthInfo { principal: from_ptr(principal), service: svc, get_key: get_key as usize, arg: arg as usize };
    s.auth.retain(|x| x.service != svc);
    s.auth.push(a);
    RPC_S_OK
}

unsafe fn guid_at(p: *const u8) -> [u8; 16] {
    let mut g = [0u8; 16];
    if !p.is_null() {
        core::ptr::copy_nonoverlapping(p, g.as_mut_ptr(), 16);
    }
    g
}

pub unsafe fn interface_id(ifspec: usize) -> ([u8; 16], u16, u16) {
    let p = ifspec as *const u8;
    (guid_at(p.add(4)), (p.add(20) as *const u16).read_unaligned(), (p.add(22) as *const u16).read_unaligned())
}

unsafe fn default_epv(ifspec: usize) -> usize {
    ((ifspec + 72) as *const usize).read_unaligned()
}

unsafe fn interpreter_info(ifspec: usize) -> usize {
    ((ifspec + 80) as *const usize).read_unaligned()
}

#[allow(clippy::too_many_arguments)]
pub unsafe fn register_interface(ifspec: usize, mgr_type: *const u8, mgr_epv: usize, flags: u32, max_calls: u32, max_rpc_size: u32, callback: usize, security: usize) -> i32 {
    if ifspec == 0 {
        return RPC_S_INVALID_ARG;
    }
    let (uuid, major, minor) = interface_id(ifspec);
    let mt = guid_at(mgr_type);
    let epv = if mgr_epv != 0 { mgr_epv } else { default_epv(ifspec) };
    let mut s = lock();
    if s.interfaces.iter().any(|i| i.uuid == uuid && i.major == major && i.mgr_type == mt) {
        return RPC_S_TYPE_ALREADY_REGISTERED;
    }
    s.interfaces.push(Interface { ifspec, uuid, major, minor, mgr_type: mt, epv, flags, max_calls, max_rpc_size, callback, security, active: 0 });
    RPC_S_OK
}

pub unsafe extern "system" fn rpc_server_register_if(ifspec: *const c_void, mgr_type: *const u8, epv: *const c_void) -> i32 {
    register_interface(ifspec as usize, mgr_type, epv as usize, 0, 1234, u32::MAX, 0, 0)
}

pub unsafe extern "system" fn rpc_server_register_if_ex(ifspec: *const c_void, mgr_type: *const u8, epv: *const c_void, flags: u32, max_calls: u32, callback: *const c_void) -> i32 {
    register_interface(ifspec as usize, mgr_type, epv as usize, flags, max_calls, u32::MAX, callback as usize, 0)
}

pub unsafe extern "system" fn rpc_server_register_if2(ifspec: *const c_void, mgr_type: *const u8, epv: *const c_void, flags: u32, max_calls: u32, max_size: u32, callback: *const c_void) -> i32 {
    register_interface(ifspec as usize, mgr_type, epv as usize, flags, max_calls, max_size, callback as usize, 0)
}

#[allow(clippy::too_many_arguments)]
pub unsafe extern "system" fn rpc_server_register_if3(ifspec: *const c_void, mgr_type: *const u8, epv: *const c_void, flags: u32, max_calls: u32, max_size: u32, callback: *const c_void, sd: *mut c_void) -> i32 {
    register_interface(ifspec as usize, mgr_type, epv as usize, flags, max_calls, max_size, callback as usize, sd as usize)
}

pub unsafe extern "system" fn rpc_server_unregister_if(ifspec: *const c_void, mgr_type: *const u8, wait: u32) -> i32 {
    let target = if ifspec.is_null() { None } else { Some(interface_id(ifspec as usize)) };
    let mt = if mgr_type.is_null() { None } else { Some(guid_at(mgr_type)) };
    let hit = |i: &Interface| target.map(|(u, ma, _)| i.uuid == u && i.major == ma).unwrap_or(true) && mt.map(|m| i.mgr_type == m).unwrap_or(true);
    let mut s = lock();
    if target.is_some() && !s.interfaces.iter().any(|i| hit(i)) {
        return RPC_S_UNKNOWN_IF;
    }
    if wait != 0 {
        while s.interfaces.iter().any(|i| hit(i) && i.active > 0) {
            s = shared().idle.wait(s).unwrap_or_else(|e| e.into_inner());
        }
    }
    s.interfaces.retain(|i| !hit(i));
    RPC_S_OK
}

pub unsafe extern "system" fn rpc_server_inq_bindings(out: *mut *mut u8) -> i32 {
    if out.is_null() {
        return RPC_S_INVALID_ARG;
    }
    let mut s = lock();
    let n = s.endpoints.len();
    if n == 0 {
        *out = core::ptr::null_mut();
        return RPC_S_NO_BINDINGS;
    }
    let v = HeapAlloc(GetProcessHeap(), 8, 8 + n * 8) as *mut u8;
    if v.is_null() {
        return RPC_S_OUT_OF_MEMORY;
    }
    (v as *mut u32).write_unaligned(n as u32);
    for i in 0..n {
        let h = new_handle(&mut s, Kind::Server(i));
        (v.add(8 + i * 8) as *mut usize).write_unaligned(h);
    }
    s.vectors.insert(v as usize);
    *out = v;
    RPC_S_OK
}

pub unsafe extern "system" fn rpc_binding_vector_free(v: *mut *mut u8) -> i32 {
    if v.is_null() || (*v).is_null() {
        return RPC_S_INVALID_ARG;
    }
    let p = *v;
    let mut s = lock();
    if !s.vectors.remove(&(p as usize)) {
        drop(s);
        return RpcBindingVectorFree(v as *mut *mut c_void);
    }
    let n = (p as *const u32).read_unaligned() as usize;
    for i in 0..n {
        let h = (p.add(8 + i * 8) as *const usize).read_unaligned();
        if s.handles.remove(&h).is_some() {
            drop(Box::from_raw(h as *mut [u64; 2]));
        }
    }
    HeapFree(GetProcessHeap(), 0, p as *mut c_void);
    *v = core::ptr::null_mut();
    RPC_S_OK
}

pub fn binding_parts(h: usize) -> Option<(String, String, [u8; 16])> {
    let s = lock();
    match s.handles.get(&h)? {
        Kind::Server(i) => s.endpoints.get(*i).map(|e| (e.protseq.clone(), e.endpoint.clone(), [0u8; 16])),
        Kind::Call(c) => s.calls.get(c).map(|c| (c.client.protseq.clone(), c.client.endpoint.clone(), c.client.object)),
    }
}

unsafe fn uuid_text(u: &[u8; 16]) -> String {
    let mut p: *mut u16 = core::ptr::null_mut();
    if UuidToStringW(u, &mut p) != RPC_S_OK {
        return String::new();
    }
    let t = from_ptr(p);
    RpcStringFreeW(&mut p);
    t
}

pub unsafe fn string_binding(h: usize) -> Option<String> {
    let (ps, ep, obj) = binding_parts(h)?;
    let o = if obj == [0u8; 16] { String::new() } else { format!("{}@", uuid_text(&obj)) };
    Some(format!("{o}{ps}:[{ep}]"))
}

pub unsafe extern "system" fn rpc_binding_to_string_binding_w(b: *mut c_void, out: *mut *mut u16) -> i32 {
    if !is_ours(b as usize) {
        return RpcBindingToStringBindingW(b, out);
    }
    if out.is_null() {
        return RPC_S_INVALID_ARG;
    }
    match string_binding(b as usize) {
        Some(t) => {
            let p = alloc_string(&t);
            if p.is_null() {
                return RPC_S_OUT_OF_MEMORY;
            }
            *out = p;
            RPC_S_OK
        }
        None => RPC_S_INVALID_BINDING,
    }
}

pub unsafe extern "system" fn rpc_string_free_w(s: *mut *mut u16) -> i32 {
    if s.is_null() {
        return RPC_S_INVALID_ARG;
    }
    let p = *s as usize;
    if p != 0 && lock().strings.remove(&p) {
        HeapFree(GetProcessHeap(), 0, p as *mut c_void);
        *s = core::ptr::null_mut();
        return RPC_S_OK;
    }
    RpcStringFreeW(s)
}

fn call_of(s: &State, b: usize) -> Result<u64, i32> {
    let h = if b == 0 { CURRENT.with(|c| c.get()) } else { b };
    if h == 0 {
        return Err(RPC_S_NO_CALL_ACTIVE);
    }
    match s.handles.get(&h) {
        Some(Kind::Call(c)) if s.calls.contains_key(c) => Ok(*c),
        _ if b == 0 => Err(RPC_S_NO_CALL_ACTIVE),
        _ => Err(RPC_S_INVALID_BINDING),
    }
}

pub fn current_call() -> usize {
    CURRENT.with(|c| c.get())
}

unsafe fn put_name(base: *mut u8, len_off: usize, ptr_off: usize, name: &str) -> bool {
    let w = wide(name);
    let need = (w.len() * 2) as u32;
    let lp = base.add(len_off) as *mut u32;
    let have = lp.read_unaligned();
    let dst = (base.add(ptr_off) as *const *mut u16).read_unaligned();
    lp.write_unaligned(need);
    if dst.is_null() || have < need {
        return dst.is_null() && have == 0;
    }
    core::ptr::copy_nonoverlapping(w.as_ptr(), dst, w.len());
    true
}

pub unsafe extern "system" fn rpc_server_inq_call_attributes_w(b: *mut c_void, attrs: *mut u8) -> i32 {
    if attrs.is_null() {
        return RPC_S_INVALID_ARG;
    }
    let version = (attrs as *const u32).read_unaligned();
    if !(1..=3).contains(&version) {
        return RPC_S_INVALID_ARG;
    }
    let s = lock();
    let id = match call_of(&s, b as usize) {
        Ok(c) => c,
        Err(e) => return e,
    };
    let c = &s.calls[&id];
    let flags = (attrs.add(4) as *const u32).read_unaligned();
    let server_principal = s.auth.iter().find(|a| a.service == c.client.authn_service).map(|a| a.principal.clone()).unwrap_or_default();
    let mut more = false;
    if flags & RPC_QUERY_SERVER_PRINCIPAL_NAME != 0 && !put_name(attrs, 8, 0x10, &server_principal) {
        more = true;
    }
    if flags & RPC_QUERY_CLIENT_PRINCIPAL_NAME != 0 && !put_name(attrs, 0x18, 0x20, &c.client.principal) {
        more = true;
    }
    let wr = |o: usize, v: u32| (attrs.add(o) as *mut u32).write_unaligned(v);
    wr(0x28, c.client.authn_level);
    wr(0x2c, c.client.authn_service);
    wr(0x30, (c.client.authn_level == RPC_C_AUTHN_LEVEL_NONE) as u32);
    if version >= 2 {
        wr(0x34, 0);
        wr(0x38, protseq_id(&c.client.protseq));
        if flags & RPC_QUERY_IS_CLIENT_LOCAL != 0 {
            wr(0x3c, if c.client.local { RCCL_LOCAL } else { RCCL_REMOTE });
        }
        if flags & RPC_QUERY_CLIENT_PID != 0 {
            (attrs.add(0x40) as *mut usize).write_unaligned(c.client.pid as usize);
        }
        wr(0x48, c.status);
        wr(0x4c, RCT_NORMAL);
        (attrs.add(0x58) as *mut u16).write_unaligned(c.opnum);
        core::ptr::copy_nonoverlapping(c.iface.as_ptr(), attrs.add(0x5c), 16);
    }
    if version >= 3 && flags & RPC_QUERY_CLIENT_ID != 0 {
        let lp = attrs.add(0x6c) as *mut u32;
        let have = lp.read_unaligned() as usize;
        let dst = (attrs.add(0x70) as *const *mut u8).read_unaligned();
        lp.write_unaligned(c.client.client_id.len() as u32);
        if have < c.client.client_id.len() || (dst.is_null() && !c.client.client_id.is_empty()) {
            more = true;
        } else if !c.client.client_id.is_empty() {
            core::ptr::copy_nonoverlapping(c.client.client_id.as_ptr(), dst, c.client.client_id.len());
        }
    }
    if more {
        ERROR_MORE_DATA
    } else {
        RPC_S_OK
    }
}

pub unsafe extern "system" fn rpc_server_test_cancel(b: *mut c_void) -> i32 {
    let s = lock();
    match call_of(&s, b as usize) {
        Err(e) => e,
        Ok(c) if s.calls[&c].status == RPC_CALL_STATUS_CANCELLED => RPC_S_OK,
        Ok(_) => RPC_S_CALL_IN_PROGRESS,
    }
}

pub unsafe extern "system" fn rpc_server_subscribe_for_notification(b: *mut c_void, notification: u32, kind: u32, info: *const usize) -> i32 {
    if !(RPC_NOTIFICATION_CLIENT_DISCONNECT..=RPC_NOTIFICATION_CALL_CANCEL).contains(&notification) || !(1..=5).contains(&kind) || info.is_null() {
        return RPC_S_INVALID_ARG;
    }
    let mut s = lock();
    let id = match call_of(&s, b as usize) {
        Ok(c) => c,
        Err(e) => return e,
    };
    let mut i = [0usize; 4];
    core::ptr::copy_nonoverlapping(info, i.as_mut_ptr(), 4);
    s.calls.get_mut(&id).unwrap().subs.insert(notification, Subscription { kind, info: i });
    RPC_S_OK
}

pub unsafe extern "system" fn rpc_server_unsubscribe_for_notification(b: *mut c_void, notification: u32, queued: *mut u32) -> i32 {
    let mut s = lock();
    let id = match call_of(&s, b as usize) {
        Ok(c) => c,
        Err(e) => return e,
    };
    let c = s.calls.get_mut(&id).unwrap();
    if c.subs.remove(&notification).is_none() {
        return RPC_S_INVALID_ARG;
    }
    if !queued.is_null() {
        *queued = c.queued;
    }
    RPC_S_OK
}

fn async_call(s: &State, a: usize) -> Option<u64> {
    if a == 0 {
        return None;
    }
    let rt = unsafe { ((a + 0x20) as *const usize).read_unaligned() };
    match s.handles.get(&rt) {
        Some(Kind::Call(c)) if s.calls.get(c).map(|x| x.async_state == a).unwrap_or(false) => Some(*c),
        _ => None,
    }
}

pub unsafe extern "system" fn rpc_async_complete_call(a: *mut c_void, reply: *mut c_void) -> i32 {
    let mut s = lock();
    let Some(id) = async_call(&s, a as usize) else {
        drop(s);
        return RpcAsyncCompleteCall(a, reply);
    };
    let c = s.calls.get_mut(&id).unwrap();
    if c.reply.is_some() {
        return RPC_S_INVALID_ASYNC_HANDLE;
    }
    let v = if reply.is_null() { 0 } else { (reply as *const u32).read_unaligned() };
    c.reply = Some(Ok(v));
    shared().idle.notify_all();
    RPC_S_OK
}

pub unsafe extern "system" fn rpc_async_abort_call(a: *mut c_void, code: u32) -> i32 {
    let mut s = lock();
    let Some(id) = async_call(&s, a as usize) else {
        drop(s);
        return RpcAsyncAbortCall(a, code);
    };
    let c = s.calls.get_mut(&id).unwrap();
    if c.reply.is_some() {
        return RPC_S_INVALID_ASYNC_HANDLE;
    }
    c.reply = Some(Err(code));
    shared().idle.notify_all();
    RPC_S_OK
}

pub struct Dispatch {
    pub routine: usize,
    pub call: usize,
    pub ifspec: usize,
}

fn begin(iface: &[u8; 16], major: u16, opnum: u16, client: &Client, is_async: bool) -> Result<(Dispatch, u64, usize), i32> {
    let mut s = lock();
    if !s.endpoints.iter().any(|e| e.protseq.eq_ignore_ascii_case(&client.protseq) && e.endpoint.eq_ignore_ascii_case(&client.endpoint)) {
        return Err(RPC_S_INVALID_BINDING);
    }
    let Some(ix) = s.interfaces.iter().position(|i| &i.uuid == iface && i.major == major && (i.mgr_type == client.object || i.mgr_type == [0u8; 16])) else {
        return Err(RPC_S_UNKNOWN_IF);
    };
    let i = s.interfaces[ix].clone();
    if i.flags & RPC_IF_ALLOW_LOCAL_ONLY != 0 && !client.local {
        return Err(RPC_S_ACCESS_DENIED);
    }
    if i.flags & RPC_IF_ALLOW_SECURE_ONLY != 0 && client.authn_level <= RPC_C_AUTHN_LEVEL_NONE {
        return Err(RPC_S_ACCESS_DENIED);
    }
    if i.active >= i.max_calls.max(1) {
        return Err(RPC_S_SERVER_TOO_BUSY);
    }
    let routine = unsafe {
        if i.epv != 0 {
            ((i.epv + opnum as usize * 8) as *const usize).read_unaligned()
        } else {
            let info = interpreter_info(i.ifspec);
            if info == 0 {
                return Err(RPC_S_PROCNUM_OUT_OF_RANGE);
            }
            let table = ((info + 8) as *const usize).read_unaligned();
            ((table + opnum as usize * 8) as *const usize).read_unaligned()
        }
    };
    if routine == 0 {
        return Err(RPC_S_PROCNUM_OUT_OF_RANGE);
    }
    s.next_call += 1;
    let id = s.next_call;
    let h = new_handle(&mut s, Kind::Call(id));
    let a = if is_async {
        let st = Box::into_raw(Box::new([0u8; RPC_ASYNC_STATE_SIZE as usize])) as *mut u8;
        unsafe {
            (st as *mut u32).write_unaligned(RPC_ASYNC_STATE_SIZE);
            (st.add(4) as *mut u32).write_unaligned(RPC_ASYNC_SIGNATURE);
            (st.add(0x20) as *mut usize).write_unaligned(h);
        }
        st as usize
    } else {
        0
    };
    s.calls.insert(id, Call { client: client.clone(), iface: *iface, opnum, status: RPC_CALL_STATUS_IN_PROGRESS, async_state: a, reply: None, subs: HashMap::new(), queued: 0 });
    s.interfaces[ix].active += 1;
    Ok((Dispatch { routine, call: h, ifspec: i.ifspec }, id, i.callback))
}

fn finish(id: u64, h: usize, iface: &[u8; 16], major: u16) {
    let mut s = lock();
    if let Some(c) = s.calls.remove(&id) {
        if c.async_state != 0 {
            unsafe { drop(Box::from_raw(c.async_state as *mut [u8; RPC_ASYNC_STATE_SIZE as usize])) };
        }
    }
    if s.handles.remove(&h).is_some() {
        unsafe { drop(Box::from_raw(h as *mut [u64; 2])) };
    }
    if let Some(i) = s.interfaces.iter_mut().find(|i| &i.uuid == iface && i.major == major && i.active > 0) {
        i.active -= 1;
    }
    drop(s);
    shared().idle.notify_all();
}

unsafe fn security_check(callback: usize, ifspec: usize, call: usize) -> i32 {
    if callback == 0 {
        return RPC_S_OK;
    }
    let f: unsafe extern "system" fn(usize, usize) -> i32 = core::mem::transmute(callback);
    let prev = CURRENT.with(|c| c.replace(call));
    let r = f(ifspec, call);
    CURRENT.with(|c| c.set(prev));
    if r != RPC_S_OK {
        RPC_S_ACCESS_DENIED
    } else {
        RPC_S_OK
    }
}

pub unsafe fn invoke<R>(iface: &[u8; 16], major: u16, opnum: u16, client: &Client, f: impl FnOnce(&Dispatch) -> R) -> Result<R, i32> {
    let (d, id, cb) = begin(iface, major, opnum, client, false)?;
    let sec = security_check(cb, d.ifspec, d.call);
    if sec != RPC_S_OK {
        finish(id, d.call, iface, major);
        return Err(sec);
    }
    let prev = CURRENT.with(|c| c.replace(d.call));
    let r = f(&d);
    CURRENT.with(|c| c.set(prev));
    finish(id, d.call, iface, major);
    Ok(r)
}

pub struct AsyncCall {
    pub dispatch: Dispatch,
    pub state: usize,
    id: u64,
    iface: [u8; 16],
    major: u16,
}

pub unsafe fn invoke_async(iface: &[u8; 16], major: u16, opnum: u16, client: &Client, f: impl FnOnce(&Dispatch, usize)) -> Result<AsyncCall, i32> {
    let (d, id, cb) = begin(iface, major, opnum, client, true)?;
    let sec = security_check(cb, d.ifspec, d.call);
    if sec != RPC_S_OK {
        finish(id, d.call, iface, major);
        return Err(sec);
    }
    let state = lock().calls[&id].async_state;
    let prev = CURRENT.with(|c| c.replace(d.call));
    f(&d, state);
    CURRENT.with(|c| c.set(prev));
    Ok(AsyncCall { dispatch: d, state, id, iface: *iface, major })
}

impl AsyncCall {
    pub fn reply(&self) -> Option<Result<u32, u32>> {
        lock().calls.get(&self.id).and_then(|c| c.reply)
    }

    pub fn wait(self, timeout: std::time::Duration) -> Option<Result<u32, u32>> {
        let end = std::time::Instant::now() + timeout;
        let mut s = lock();
        loop {
            if let Some(r) = s.calls.get(&self.id).and_then(|c| c.reply) {
                drop(s);
                finish(self.id, self.dispatch.call, &self.iface, self.major);
                return Some(r);
            }
            let now = std::time::Instant::now();
            if now >= end {
                return None;
            }
            s = shared().idle.wait_timeout(s, end - now).unwrap_or_else(|e| e.into_inner()).0;
        }
    }

    pub unsafe fn notify(&self, notification: u32) -> bool {
        let mut s = lock();
        let Some(c) = s.calls.get_mut(&self.id) else { return false };
        c.status = if notification == RPC_NOTIFICATION_CALL_CANCEL { RPC_CALL_STATUS_CANCELLED } else { RPC_CALL_STATUS_DISCONNECTED };
        let Some(sub) = c.subs.get(&notification) else { return false };
        let (kind, info, state) = (sub.kind, sub.info, c.async_state);
        c.queued += 1;
        drop(s);
        match kind {
            RPC_NOTIFICATION_TYPE_EVENT => SetEvent(info[0]) != 0,
            RPC_NOTIFICATION_TYPE_CALLBACK => {
                let f: unsafe extern "system" fn(usize, usize, u32) = core::mem::transmute(info[0]);
                f(state, 0, if notification == RPC_NOTIFICATION_CALL_CANCEL { RPC_ASYNC_EVENT_CLIENT_CANCEL } else { RPC_ASYNC_EVENT_CLIENT_DISCONNECT });
                true
            }
            _ => false,
        }
    }
}

pub fn cancel(call: usize) -> bool {
    let mut s = lock();
    let Some(Kind::Call(id)) = s.handles.get(&call).copied() else { return false };
    match s.calls.get_mut(&id) {
        Some(c) => {
            c.status = RPC_CALL_STATUS_CANCELLED;
            true
        }
        None => false,
    }
}

pub fn active_calls(iface: &[u8; 16]) -> u32 {
    lock().interfaces.iter().filter(|i| &i.uuid == iface).map(|i| i.active).sum()
}
