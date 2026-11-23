use super::*;
use console_sandbox::{comreg, rpc, rpcsrv, wnf};
use std::sync::atomic::{AtomicU32, Ordering as O};
use std::sync::OnceLock;

pub const WLAN_IFSPEC_GLOBAL: usize = 0x6ca78;
pub const WLAN_ENDPOINT_REGISTER: usize = 0x46aec;
pub const WLAN_SECURITY_CALLBACK: usize = 0x46d40;
pub const HVSOCKET_SECURITY_CALLBACK: usize = 0x4cc60;
pub const ENDPOINT_TEARDOWN: usize = 0x312e8;
pub const OPERATOR_NEW: usize = 0x4fa8c;
pub const PUBLISH_NLM_STATE: usize = 0xaf24;
pub const PUBLISH_HOST_STATE: usize = 0x3f324;
pub const REGISTER_CLASS_OBJECTS: usize = 0x7350;
pub const REVOKE_CLASS_OBJECTS: usize = 0x72f0;
pub const REGISTER_FACTORIES: usize = 0x7190;
pub const REVOKE_FACTORIES: usize = 0x7160;
pub const FACTORY_CALLBACK: usize = 0x7570;
pub const FLAG_NAMES: usize = 0x5eeb8;
pub const COST_NAMES: [usize; 7] = [0x5ef58, 0x5ef68, 0x5ef70, 0x5ef80, 0x5ef90, 0x5efa0, 0x5efb0];

fn patched() -> Option<usize> {
    static M: OnceLock<Option<usize>> = OnceLock::new();
    *M.get_or_init(|| {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let m = console()?;
        unsafe { console_sandbox::attach(m) };
        Some(m)
    })
}

unsafe fn ascii(a: usize) -> String {
    let mut v = Vec::new();
    while *((a + v.len()) as *const u8) != 0 {
        v.push(*((a + v.len()) as *const u8));
    }
    String::from_utf8(v).unwrap()
}

#[test]
fn console_wnf_publishers_write_only_the_emulated_store_and_match_the_port() {
    let _g = sandbox();
    let Some(m) = patched() else { return };
    unsafe {
        let st = [connectivity::CONNECTED | connectivity::IPV4_LOCAL, connectivity::COST_UNRESTRICTED];
        let f: unsafe extern "system" fn(usize, *const u32) -> i32 = core::mem::transmute(m + PUBLISH_NLM_STATE);
        assert_eq!(f(0, st.as_ptr()), 0x1000_0000);
        let console_bytes = wnf::state(connectivity::WNF_NLM_4).unwrap();
        assert_eq!(console_bytes.data, [5, 0, 0, 0, 1, 0, 0, 0]);
        let before = console_bytes.stamp;
        assert_eq!(connectivity::publish_nlm_state(st[0], st[1]), 0);
        let port = wnf::state(connectivity::WNF_NLM_4).unwrap();
        assert_eq!((port.data, port.stamp), (console_bytes.data, before + 1));

        let host: [u8; 12] = [0x11, 0x22, 0x33, 0, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb];
        let g: unsafe extern "system" fn(usize, *const u8) = core::mem::transmute(m + PUBLISH_HOST_STATE);
        g(0, host.as_ptr());
        let names = [connectivity::WNF_CNET_9, connectivity::WNF_CNET_13, connectivity::WNF_CNET_14, connectivity::WNF_CNET_3, connectivity::WNF_DUSM_2];
        let from_console: Vec<Vec<u8>> = names.iter().map(|n| wnf::state(*n).unwrap().data).collect();
        assert_eq!(from_console, vec![vec![0x22], vec![0x11], vec![0x33], vec![0x44, 0x55, 0x66, 0x77], vec![0x88, 0x99, 0xaa, 0xbb]]);
        for n in names {
            assert_eq!(wnf::update(n, &[], None, None), 0);
        }
        assert_eq!(connectivity::publish_host_state(&connectivity::HostNetworkState::from_bytes(&host)), [0; 5]);
        let from_port: Vec<Vec<u8>> = names.iter().map(|n| wnf::state(*n).unwrap().data).collect();
        assert_eq!(from_port, from_console);
    }
}

#[test]
fn aggregate_flag_and_cost_names_are_the_console_tables() {
    let Some(m) = patched() else { return };
    unsafe {
        for i in 0..10 {
            let s = ascii(m + FLAG_NAMES + i * 0x10);
            assert_eq!(format!("{},", connectivity::flag_names(1 << i)), s, "flag bit {i}");
        }
        let bits = [connectivity::COST_UNRESTRICTED, connectivity::COST_FIXED, connectivity::COST_VARIABLE, connectivity::COST_OVER_DATA_LIMIT, connectivity::COST_CONGESTED, connectivity::COST_ROAMING, connectivity::COST_APPROACHING_DATA_LIMIT];
        for (b, a) in bits.iter().zip(COST_NAMES) {
            assert_eq!(format!("{},", connectivity::cost_names(*b)), ascii(m + a));
        }
    }
}

struct FakeIf {
    buf: Box<[u8; 0x60]>,
    table: Box<[usize; 2]>,
}

impl FakeIf {
    fn ifspec(&self) -> usize {
        self.buf.as_ptr() as usize
    }
    fn epv(&self) -> usize {
        self.table.as_ptr() as usize
    }
    fn routine(&self) -> usize {
        self.table[0]
    }
}

extern "system" fn fake_routine() {}

fn fake_ifspec(uuid: [u8; 16]) -> FakeIf {
    let mut buf = Box::new([0u8; 0x60]);
    buf[0..4].copy_from_slice(&0x60u32.to_le_bytes());
    buf[4..20].copy_from_slice(&uuid);
    buf[20..22].copy_from_slice(&1u16.to_le_bytes());
    FakeIf { buf, table: Box::new([fake_routine as *const () as usize, 0]) }
}

#[test]
fn console_wlan_endpoint_registers_in_process_and_its_callbacks_gate_calls() {
    let _g = sandbox();
    let Some(m) = patched() else { return };
    unsafe {
        rpcsrv::reset();
        let ifspec: usize = rd(m + WLAN_IFSPEC_GLOBAL);
        let (uuid, major, _) = rpcsrv::interface_id(ifspec);
        let new: unsafe extern "system" fn(usize) -> *mut usize = core::mem::transmute(m + OPERATOR_NEW);
        let holder = new(8);
        let reg: unsafe extern "system" fn(*mut usize, usize) -> *mut usize = core::mem::transmute(m + WLAN_ENDPOINT_REGISTER);
        assert_eq!(reg(holder, ifspec), holder);
        assert_eq!(*holder, ifspec);
        let eps = rpcsrv::endpoints();
        assert_eq!(eps.len(), 1);
        assert_eq!((eps[0].protseq.as_str(), eps[0].dynamic, eps[0].max_calls), ("ncalrpc", true, 10));
        assert_eq!(eps[0].endpoint, rpc::dynamic_endpoint());
        let auth = rpcsrv::auth_info();
        assert_eq!(auth.len(), 1);
        assert_eq!((auth[0].service, auth[0].principal.as_str()), (rpcsrv::RPC_C_AUTHN_WINNT, rpcsrv::DEFAULT_PRINCIPAL));
        let i = rpcsrv::interfaces().into_iter().find(|i| i.uuid == uuid).unwrap();
        assert_eq!((i.flags, i.max_calls, i.max_rpc_size, i.callback, i.security), (0x29, 1234, 0, m + WLAN_SECURITY_CALLBACK, 0));
        let map: Vec<_> = rpc::entries().into_iter().filter(|e| e.interface == uuid).collect();
        assert_eq!(map.len(), 1);
        assert_eq!((map[0].protseq.as_str(), map[0].endpoint.as_str(), map[0].annotation.as_str()), ("ncalrpc", eps[0].endpoint.as_str(), "Wlan Service"));

        let lrpc = rpcsrv::Client::lrpc(&eps[0].endpoint);
        let r = rpcsrv::invoke(&uuid, major, 0, &lrpc, |d| (d.routine, rpcsrv::current_call() == d.call)).unwrap();
        assert_ne!(r.0, 0);
        assert!(r.1);
        let hv_ep = "00000000-0000-0000-0000-000000000000:1";
        assert_eq!(rpcsrv::listen("ncacn_hvsocket", 10, Some(hv_ep), 0), 0);
        let mut hv = rpcsrv::Client::hvsocket(hv_ep, [7; 16]);
        hv.local = true;
        hv.authn_level = rpcsrv::RPC_C_AUTHN_LEVEL_PKT_PRIVACY;
        assert_eq!(rpcsrv::invoke(&uuid, major, 0, &hv, |_| ()).err(), Some(rpcsrv::RPC_S_ACCESS_DENIED));

        let fake = fake_ifspec([0x42; 16]);
        assert_eq!(rpcsrv::register_interface(fake.ifspec(), core::ptr::null(), fake.epv(), 0x11, 1234, 0, m + HVSOCKET_SECURITY_CALLBACK, 0), 0);
        let remote = rpcsrv::Client::hvsocket(hv_ep, [7; 16]);
        assert_eq!(rpcsrv::invoke(&[0x42; 16], 1, 0, &remote, |d| d.routine).unwrap(), fake.routine());
        assert_eq!(rpcsrv::invoke(&[0x42; 16], 1, 0, &hv, |_| ()).err(), Some(rpcsrv::RPC_S_ACCESS_DENIED));
        assert_eq!(rpcsrv::invoke(&[0x42; 16], 1, 0, &lrpc, |_| ()).err(), Some(rpcsrv::RPC_S_ACCESS_DENIED));

        let down: unsafe extern "system" fn(usize, *mut usize) = core::mem::transmute(m + ENDPOINT_TEARDOWN);
        down(0, holder);
        assert!(rpcsrv::interfaces().iter().all(|i| i.uuid != uuid));
        assert!(rpc::entries().iter().all(|e| e.interface != uuid));
        rpcsrv::reset();
    }
}

static OBJ_REFS: AtomicU32 = AtomicU32::new(1);

unsafe extern "system" fn obj_qi(this: usize, iid: *const [u8; 16], out: *mut usize) -> i32 {
    if *iid == comreg::IID_IUNKNOWN || *iid == comreg::IID_ICLASSFACTORY {
        OBJ_REFS.fetch_add(1, O::SeqCst);
        *out = this;
        0
    } else {
        *out = 0;
        comreg::E_NOINTERFACE
    }
}

unsafe extern "system" fn obj_add(_: usize) -> u32 {
    OBJ_REFS.fetch_add(1, O::SeqCst) + 1
}

unsafe extern "system" fn obj_rel(_: usize) -> u32 {
    OBJ_REFS.fetch_sub(1, O::SeqCst) - 1
}

unsafe extern "system" fn obj_create(this: usize, outer: usize, iid: *const [u8; 16], out: *mut usize) -> i32 {
    if outer != 0 {
        return comreg::CLASS_E_NOAGGREGATION;
    }
    obj_qi(this, iid, out)
}

fn object() -> usize {
    static O: OnceLock<usize> = OnceLock::new();
    *O.get_or_init(|| {
        let vt: &'static [usize; 5] = Box::leak(Box::new([obj_qi as *const () as usize, obj_add as *const () as usize, obj_rel as *const () as usize, obj_create as *const () as usize, 0]));
        Box::leak(Box::new([vt.as_ptr() as usize])).as_ptr() as usize
    })
}

#[test]
fn console_class_and_factory_registration_stays_in_process() {
    let _g = sandbox();
    let Some(m) = patched() else { return };
    unsafe {
        let clsid: [[u8; 16]; 1] = [[0x5a; 16]];
        let objs: [usize; 1] = [object()];
        let mut cookies = [0u32; 1];
        let reg: unsafe extern "system" fn(usize, usize, *const [u8; 16], *const usize, *mut u32, u32) -> i32 = core::mem::transmute(m + REGISTER_CLASS_OBJECTS);
        let refs0 = OBJ_REFS.load(O::SeqCst);
        assert_eq!(reg(0, 0, clsid.as_ptr(), objs.as_ptr(), cookies.as_mut_ptr(), 1), 0);
        let e = comreg::classes().into_iter().find(|e| e.cookie == cookies[0]).unwrap();
        assert_eq!((e.clsid, e.context, e.flags, e.suspended), (clsid[0], comreg::CLSCTX_LOCAL_SERVER, comreg::REGCLS_MULTIPLEUSE | comreg::REGCLS_SUSPENDED, false));
        assert_eq!(OBJ_REFS.load(O::SeqCst), refs0 + 1);
        let mut out: *mut c_void = core::ptr::null_mut();
        assert_eq!(comreg::co_create_instance(&clsid[0], core::ptr::null_mut(), comreg::CLSCTX_LOCAL_SERVER, &comreg::IID_IUNKNOWN, &mut out), 0);
        assert_eq!(out as usize, objs[0]);
        comreg::release(out as usize);
        assert!(comreg::create_from_table(&clsid[0], core::ptr::null_mut(), comreg::CLSCTX_INPROC_SERVER, &comreg::IID_IUNKNOWN, &mut out).is_none());
        let rev: unsafe extern "system" fn(usize, usize, *mut u32, u32) = core::mem::transmute(m + REVOKE_CLASS_OBJECTS);
        rev(0, 0, cookies.as_mut_ptr(), 1);
        assert_eq!(cookies[0], 0);
        assert!(comreg::classes().iter().all(|x| x.clsid != clsid[0]));
        assert_eq!(OBJ_REFS.load(O::SeqCst), refs0);

        let name: Vec<u16> = "TomBox.NetworkProfile.Oracle\0".encode_utf16().collect();
        let names = [name.as_ptr()];
        let mut cookie = 0usize;
        let regf: unsafe extern "system" fn(usize, usize, *const *const u16, *mut usize, u32) -> i32 = core::mem::transmute(m + REGISTER_FACTORIES);
        assert_eq!(regf(0, 0, names.as_ptr(), &mut cookie, 1), 0);
        let f = comreg::factories().into_iter().find(|f| f.cookie == cookie).unwrap();
        assert_eq!((f.class.as_str(), f.callback), ("TomBox.NetworkProfile.Oracle", m + FACTORY_CALLBACK));
        let revf: unsafe extern "system" fn(usize, usize, usize) -> i32 = core::mem::transmute(m + REVOKE_FACTORIES);
        assert_eq!(revf(0, 0, cookie), 0);
        assert!(comreg::factories().iter().all(|x| x.cookie != cookie));
    }
}

#[test]
fn aggregate_connectivity_follows_the_console_mapping() {
    use connectivity::*;
    let fam = |level, detail| Family { level, timestamp: 0, detail };
    assert_eq!(flags(&accumulate(&[])), DISCONNECTED);
    let nic = Interface { connected: true, identified: true, hidden: false, v4: fam(LEVEL_LOCAL, 0), v6: fam(LEVEL_NONE, 0), cost: Some(COST_UNRESTRICTED) };
    let a = accumulate(&[nic]);
    assert_eq!(flags(&a), CONNECTED | IPV4_LOCAL);
    assert_eq!(cost(&a), COST_UNRESTRICTED);
    let hidden = Interface { hidden: true, v4: fam(LEVEL_INTERNET, DETAIL_WEB_HIJACK), ..nic };
    let unidentified = Interface { identified: false, v6: fam(LEVEL_INTERNET, 0), ..nic };
    assert_eq!(flags(&accumulate(&[nic, hidden, unidentified])), CONNECTED | IPV4_LOCAL);
    let roam = Interface { v4: fam(LEVEL_INTERNET, DETAIL_AUTH_PROXY), v6: fam(LEVEL_LOCAL, DETAIL_WEB_HIJACK), cost: Some(COST_VARIABLE | COST_ROAMING), ..nic };
    let b = accumulate(&[nic, roam]);
    assert_eq!(flags(&b), CONNECTED | IPV4_INTERNET | IPV4_AUTH_PROXY | IPV6_LOCAL | IPV6_WEB_HIJACK);
    assert_eq!(cost(&b), COST_VARIABLE | COST_ROAMING);
    assert_eq!(flag_names(flags(&b)), "Connected,IPv4Internet,IPv6Local,IPv6WebHijack,IPv4AuthProx");
    assert_eq!(cost_names(0), "Unknown");
    let v4 = Family { level: LEVEL_LOCAL, timestamp: 50, detail: 0 };
    let v6 = Family { level: LEVEL_LOCAL, timestamp: 20, detail: 0 };
    assert_eq!(connectivity_timestamp(&v4, &v6), 20);
    assert_eq!(connectivity_timestamp(&v4, &Family::default()), 50);
    let rec = |key, v4: Family, v6: Family| Record { key, identified: true, hidden: false, v4, v6 };
    let low4 = Family { level: LEVEL_LOCAL, timestamp: 1, detail: 0 };
    let high4 = Family { level: LEVEL_INTERNET, timestamp: 2, detail: 1 };
    let order = [(7, AF_INET), (9, AF_INET), (9, AF_INET6)];
    assert_eq!(select_families(&[rec(9, high4, v6), rec(7, low4, Family::default())], &order), (low4, v6));
    assert_eq!(select_families(&[rec(1, low4, v6), rec(2, high4, Family::default())], &order), (high4, Family::default()));
    assert_eq!(select_families(&[rec(5, high4, v6), rec(6, low4, v4)], &[]), (low4, v4));
    let mut hid = rec(7, high4, v6);
    hid.hidden = true;
    let mut unid = rec(9, high4, v6);
    unid.identified = false;
    assert_eq!(select_families(&[hid, unid], &order), (Family::default(), Family::default()));
    assert_eq!(query(None), (DISCONNECTED, 0));
}

#[test]
fn aggregator_publishes_only_while_enabled_and_zeroes_when_disabled() {
    use connectivity::*;
    let _g = sandbox();
    let mut ag = Aggregator::default();
    assert_eq!(ag.update(CONNECTED | IPV4_LOCAL, COST_UNRESTRICTED, [0; 0x38]), None);
    let stamp = wnf::state(WNF_NLM_4).map(|s| s.stamp).unwrap_or(0);
    assert_eq!(ag.set_publishing(true), Some(0));
    assert_eq!(wnf::state(WNF_NLM_4).unwrap().data, [5, 0, 0, 0, 1, 0, 0, 0]);
    assert_eq!(ag.update(CONNECTED | IPV4_LOCAL, COST_UNRESTRICTED, [1; 0x38]), None);
    assert_eq!(ag.update(DISCONNECTED, 0, [0; 0x38]), Some(0));
    assert_eq!(wnf::state(WNF_NLM_4).unwrap().data, [2, 0, 0, 0, 0, 0, 0, 0]);
    assert_eq!(ag.set_publishing(false), Some(0));
    let s = wnf::state(WNF_NLM_4).unwrap();
    assert_eq!((s.data, s.stamp), (vec![0u8; 8], stamp + 3));
    assert_eq!(ag.read(), (DISCONNECTED, 0));
    assert_eq!(signal_network_event(), 0);
    let p = wnf::state(WNF_SEB_4).unwrap().data;
    assert_eq!(p.len(), 0x1c);
    assert_eq!(&p[0..8], &[0x50, 0, 0, 0, 0xff, 0xff, 0xff, 0xff]);
    assert_eq!(&p[8..24], &SEB_NETWORK_EVENT);
}
