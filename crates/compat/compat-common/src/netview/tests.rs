use super::sockets::*;
use super::wfp;
use super::*;
use std::os::windows::io::AsRawSocket;

fn rd32(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}

fn rd64(b: &[u8], o: usize) -> u64 {
    u64::from_le_bytes(b[o..o + 8].try_into().unwrap())
}

unsafe fn gaa(family: u32, flags: u32) -> Vec<u8> {
    let mut size = 0u32;
    assert_eq!(get_adapters_addresses(family, flags, core::ptr::null_mut(), &mut size), ERROR_BUFFER_OVERFLOW);
    let mut small = size - 1;
    let mut buf = vec![0u8; size as usize];
    assert_eq!(get_adapters_addresses(family, flags, buf.as_mut_ptr(), &mut small), ERROR_BUFFER_OVERFLOW);
    assert_eq!(small, size);
    assert_eq!(get_adapters_addresses(family, flags, buf.as_mut_ptr(), &mut size), ERROR_SUCCESS);
    buf
}

unsafe fn sockaddr_at(p: u64, len: u32) -> std::net::SocketAddr {
    parse_sockaddr(p as *const u8, len as i32).unwrap()
}

#[test]
fn adapters_addresses_use_the_lh_layout_with_one_nic_and_loopback() {
    unsafe {
        let buf = gaa(AF_UNSPEC, 6);
        let base = buf.as_ptr() as u64;
        let a = &buf[..];
        assert_eq!(rd32(a, 0), 0x1c0);
        assert_eq!(rd32(a, 4), NIC_IF_INDEX);
        let next = rd64(a, 8);
        assert_eq!(next - base, 0x1c0);
        let name = std::ffi::CStr::from_ptr(rd64(a, 0x10) as *const i8).to_str().unwrap().to_string();
        assert!(name.starts_with('{') && name.len() == 38);
        assert_eq!(&a[0x50..0x56], &NIC_MAC);
        assert_eq!(rd32(a, 0x58), 6);
        assert_eq!(rd32(a, 0x64), IF_TYPE_ETHERNET_CSMACD);
        assert_eq!(rd32(a, 0x68), IF_OPER_STATUS_UP);
        assert_eq!(rd64(a, 0xe0), NIC_LUID);
        assert_eq!(rd64(a, 0x28), 0);
        assert_eq!(rd64(a, 0x30), 0);
        assert_eq!(rd64(a, 0xb0), 0);
        let mut u = rd64(a, 0x18);
        let mut seen = Vec::new();
        while u != 0 {
            let e = core::slice::from_raw_parts(u as *const u8, UNICAST_SIZE);
            assert_eq!(rd32(e, 0), UNICAST_SIZE as u32);
            seen.push((sockaddr_at(rd64(e, 0x10), rd32(e, 0x18)).ip(), e[0x38], rd32(e, 0x20), rd32(e, 0x24), rd32(e, 0x28)));
            u = rd64(e, 8);
        }
        assert_eq!(seen, vec![(IpAddr::V6(NIC_IPV6), 64, 2, 4, 4), (IpAddr::V4(NIC_IPV4), 16, 2, 4, 4)]);
        let lo = core::slice::from_raw_parts(next as *const u8, 0x1c0);
        assert_eq!(rd32(lo, 4), LOOPBACK_IF_INDEX);
        assert_eq!(rd64(lo, 8), 0);
        assert_eq!(rd32(lo, 0x64), IF_TYPE_SOFTWARE_LOOPBACK);
        assert_eq!(rd32(lo, 0x58), 0);
        let fname: Vec<u16> = (0..).map(|i| *(rd64(lo, 0x48) as *const u16).add(i)).take_while(|c| *c != 0).collect();
        assert_eq!(String::from_utf16(&fname).unwrap(), "Loopback Pseudo-Interface 1");
    }
}

#[test]
fn adapter_family_and_list_flags_filter_entries() {
    unsafe {
        let buf = gaa(AF_INET, GAA_FLAG_INCLUDE_PREFIX);
        let a = &buf[..];
        let u = core::slice::from_raw_parts(rd64(a, 0x18) as *const u8, UNICAST_SIZE);
        assert_eq!(sockaddr_at(rd64(u, 0x10), rd32(u, 0x18)).ip(), IpAddr::V4(NIC_IPV4));
        assert_eq!(rd64(u, 8), 0);
        let m = core::slice::from_raw_parts(rd64(a, 0x28) as *const u8, MULTICAST_SIZE);
        assert_eq!(sockaddr_at(rd64(m, 0x10), rd32(m, 0x18)).ip(), IpAddr::V4(Ipv4Addr::new(224, 0, 0, 1)));
        let p = core::slice::from_raw_parts(rd64(a, 0xb0) as *const u8, PREFIX_SIZE);
        assert_eq!(sockaddr_at(rd64(p, 0x10), rd32(p, 0x18)).ip(), IpAddr::V4(Ipv4Addr::new(169, 254, 0, 0)));
        assert_eq!(rd32(p, 0x20), 16);
        let mut size = 0u32;
        assert_eq!(get_adapters_addresses(5, 0, core::ptr::null_mut(), &mut size), ERROR_INVALID_PARAMETER);
        assert_eq!(get_adapters_addresses(AF_INET, 0, core::ptr::null_mut(), core::ptr::null_mut()), ERROR_INVALID_PARAMETER);
    }
}

#[test]
fn forward_table_has_on_link_routes_and_no_default_route() {
    let t = ip_forward_table2(AF_UNSPEC).unwrap();
    let n = rd32(&t, 0) as usize;
    assert_eq!(t.len(), 8 + n * ROUTE_ROW_SIZE);
    assert_eq!(n, routes().len());
    let row = &t[8..8 + ROUTE_ROW_SIZE];
    assert_eq!(rd64(row, 0), LOOPBACK_LUID);
    assert_eq!(rd32(row, 8), LOOPBACK_IF_INDEX);
    assert_eq!(&row[0xc..0x14], &[2, 0, 0, 0, 127, 0, 0, 0]);
    assert_eq!(row[0x28], 8);
    assert_eq!(rd32(row, 0x54), 256);
    assert_eq!(rd32(row, 0x58), ROUTE_PROTOCOL_LOCAL);
    assert_eq!(row[0x5c], 1);
    assert_eq!(rd32(row, 0x64), ROUTE_ORIGIN_WELL_KNOWN);
    assert!(routes().iter().all(|r| r.prefix_len != 0));
    assert!(has_route_to(&"127.5.5.5".parse().unwrap()));
    assert!(has_route_to(&IpAddr::V4(NIC_IPV4)));
    assert!(!has_route_to(&"8.8.8.8".parse().unwrap()));
    assert!(!has_route_to(&"2001:db8::1".parse().unwrap()));
    let v6 = ip_forward_table2(AF_INET6).unwrap();
    assert_eq!(rd32(&v6, 0) as usize, routes().iter().filter(|r| r.dest.is_ipv6()).count());
    assert_eq!(ip_forward_table2(7), Err(ERROR_INVALID_PARAMETER));
}

#[test]
fn teredo_reports_offline_state_and_no_port() {
    unsafe {
        let mut rw = [0xffu8; 12];
        let m = NPI_MS_TEREDO_MODULEID;
        let n = core::ptr::null_mut();
        assert_eq!(nsi_get_all_parameters(1, m.as_ptr(), 1, core::ptr::null(), 0, rw.as_mut_ptr(), 12, n, 0, n, 0), 0);
        assert_eq!(rd32(&rw, 0), TEREDO_STATE_OFFLINE);
        assert_ne!(rd32(&rw, 0), TEREDO_STATE_QUALIFIED);
        assert_eq!(&rw[4..], &[0; 8]);
        let mut port = [0u8; 2];
        assert_eq!(nsi_get_all_parameters(1, m.as_ptr(), 3, core::ptr::null(), 0, port.as_mut_ptr(), 2, n, 0, n, 0), ERROR_NOT_READY);
        assert_eq!(nsi_get_all_parameters(1, m.as_ptr(), 1, core::ptr::null(), 0, rw.as_mut_ptr(), 8, n, 0, n, 0), ERROR_INVALID_PARAMETER);
        let mut other = m;
        other[8] = 0;
        assert_eq!(nsi_get_all_parameters(1, other.as_ptr(), 1, core::ptr::null(), 0, rw.as_mut_ptr(), 12, n, 0, n, 0), ERROR_NOT_FOUND);
    }
}

#[test]
fn socket_registry_reports_live_title_sockets_in_owner_module_tables() {
    let _g = super::sockets::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let c = std::net::TcpStream::connect(l.local_addr().unwrap()).unwrap();
    let u = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let peer = l.local_addr().unwrap();
    let sa = sockaddr_bytes(&peer.ip(), peer.port(), 0);
    unsafe {
        assert!(report_socket(l.as_raw_socket() as usize, core::ptr::null(), 0));
        assert!(report_socket(c.as_raw_socket() as usize, sa.as_ptr(), sa.len() as i32));
        assert!(report_socket(u.as_raw_socket() as usize, core::ptr::null(), 0));
        assert!(!report_socket(0xdead_beef, core::ptr::null(), 0));
        let mut size = 0u32;
        assert_eq!(get_extended_tcp_table(core::ptr::null_mut(), &mut size, true, AF_INET, TCP_TABLE_OWNER_MODULE_ALL, 0), ERROR_INSUFFICIENT_BUFFER);
        let mut buf = vec![0u8; size as usize];
        assert_eq!(get_extended_tcp_table(buf.as_mut_ptr(), &mut size, true, AF_INET, TCP_TABLE_OWNER_MODULE_ALL, 0), ERROR_SUCCESS);
        let n = rd32(&buf, 0) as usize;
        let rows: Vec<&[u8]> = (0..n).map(|i| &buf[8 + i * TCP_ROW_OWNER_MODULE_SIZE..8 + (i + 1) * TCP_ROW_OWNER_MODULE_SIZE]).collect();
        let pid = std::process::id();
        let listen = rows.iter().find(|r| rd32(r, 0) == MIB_TCP_STATE_LISTEN && u16::from_be_bytes([r[8], r[9]]) == peer.port()).expect("listener row");
        assert_eq!(&listen[4..8], &[127, 0, 0, 1]);
        assert_eq!(rd32(listen, 0x14), pid);
        assert_ne!(rd64(listen, 0x18), 0);
        let est = rows.iter().find(|r| rd32(r, 0) == MIB_TCP_STATE_ESTAB && u16::from_be_bytes([r[0x10], r[0x11]]) == peer.port()).expect("connected row");
        assert_eq!(u16::from_be_bytes([est[8], est[9]]), c.local_addr().unwrap().port());
        let t = udp_table(AF_INET, UDP_TABLE_OWNER_MODULE, true).unwrap();
        let un = rd32(&t, 0) as usize;
        assert!((0..un).any(|i| {
            let r = &t[8 + i * UDP_ROW_OWNER_MODULE_SIZE..];
            u16::from_be_bytes([r[4], r[5]]) == u.local_addr().unwrap().port() && rd32(r, 8) == pid
        }));
        let pidt = tcp_table(AF_INET, TCP_TABLE_OWNER_PID_LISTENER, false).unwrap();
        assert_eq!((pidt.len() - 4) % TCP_ROW_OWNER_PID_SIZE, 0);
        assert!(tcp_table(AF_INET, 2, true).is_err());
        let ls = l.as_raw_socket() as u64;
        drop(l);
        assert!(!endpoints().iter().any(|e| e.socket == ls));
        assert!(!snapshot().iter().any(|e| e.socket == ls));
    }
}

#[test]
fn console_view_maps_host_lan_addresses_to_the_emulated_nic() {
    let _g = super::sockets::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let a: std::net::SocketAddr = "192.168.1.20:5000".parse().unwrap();
    assert_eq!(console_local(a), std::net::SocketAddr::new(IpAddr::V4(NIC_IPV4), 5000));
    let l: std::net::SocketAddr = "127.0.0.1:7".parse().unwrap();
    assert_eq!(console_local(l), l);
    let z: std::net::SocketAddr = "0.0.0.0:9".parse().unwrap();
    assert_eq!(console_local(z), z);
}

#[test]
fn wfp_store_is_empty_and_keeps_dynamic_providers_per_session() {
    let _g = wfp::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let e = wfp::engine_open(wfp::FWPM_SESSION_FLAG_DYNAMIC).unwrap();
    for kind in [wfp::Kind::ProviderContext, wfp::Kind::SubLayer, wfp::Kind::Callout, wfp::Kind::Filter, wfp::Kind::IpsecSaContext, wfp::Kind::IpsecSa, wfp::Kind::IkeSa, wfp::Kind::AleEndpoint] {
        let h = wfp::create_enum_handle(e, kind).unwrap();
        assert!(wfp::enum_next(e, h, 0x40).unwrap().is_empty());
        assert_eq!(wfp::destroy_enum_handle(e, h), 0);
        assert_eq!(wfp::destroy_enum_handle(e, h), wfp::ERROR_INVALID_HANDLE);
    }
    let key = [7u8; 16];
    assert_eq!(wfp::provider_get_by_key(e, &key), Err(wfp::FWP_E_PROVIDER_NOT_FOUND));
    let p = wfp::Provider { key, name: Some("p".encode_utf16().collect()), description: None, flags: 0, data: Vec::new(), service_name: None };
    assert_eq!(wfp::provider_add(e, wfp::Provider { flags: wfp::FWPM_PROVIDER_FLAG_PERSISTENT, ..p.clone() }), wfp::FWP_E_DYNAMIC_SESSION_IN_PROGRESS);
    assert_eq!(wfp::provider_add(e, p.clone()), 0);
    assert_eq!(wfp::provider_add(e, p.clone()), wfp::FWP_E_ALREADY_EXISTS);
    assert_eq!(wfp::provider_get_by_key(e, &key), Ok(p.clone()));
    let h = wfp::create_enum_handle(e, wfp::Kind::Provider).unwrap();
    let first = wfp::enum_next(e, h, 0x40).unwrap();
    assert!(first.contains(&wfp::Object::Provider(p.clone())));
    assert!(wfp::enum_next(e, h, 0x40).unwrap().is_empty());
    assert_eq!(wfp::destroy_enum_handle(e, h), 0);
    assert_eq!(wfp::transaction_abort(e), wfp::FWP_E_NO_TXN_IN_PROGRESS);
    assert_eq!(wfp::transaction_begin(e), 0);
    assert_eq!(wfp::transaction_begin(e), wfp::FWP_E_TXN_IN_PROGRESS);
    assert_eq!(wfp::transaction_abort(e), 0);
    assert_eq!(wfp::engine_close(e), 0);
    assert_eq!(wfp::engine_close(e), wfp::ERROR_INVALID_HANDLE);
    let e2 = wfp::engine_open(0).unwrap();
    assert_eq!(wfp::provider_get_by_key(e2, &key), Err(wfp::FWP_E_PROVIDER_NOT_FOUND));
    assert_eq!(wfp::provider_delete_by_key(e2, &key), wfp::FWP_E_PROVIDER_NOT_FOUND);
    assert_eq!(wfp::engine_close(e2), 0);
    assert_eq!(wfp::create_enum_handle(e2, wfp::Kind::Provider), Err(wfp::ERROR_INVALID_HANDLE));
}

#[test]
fn wfp_store_lives_in_the_process_wide_mapping() {
    let _g = wfp::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    extern "system" {
        fn OpenFileMappingW(access: u32, inherit: i32, name: *const u16) -> *mut core::ffi::c_void;
        fn MapViewOfFile(h: *mut core::ffi::c_void, access: u32, high: u32, low: u32, n: usize) -> *mut core::ffi::c_void;
        fn UnmapViewOfFile(p: *const core::ffi::c_void) -> i32;
        fn CloseHandle(h: *mut core::ffi::c_void) -> i32;
    }
    let e = wfp::engine_open(wfp::FWPM_SESSION_FLAG_DYNAMIC).unwrap();
    let e2 = wfp::engine_open(0).unwrap();
    assert!(e2 > e && e >= wfp::FIRST_HANDLE);
    let name: Vec<u16> = "shared-store-probe".encode_utf16().collect();
    let key = [0x5a; 16];
    let p = wfp::Provider { key, name: Some(name.clone()), description: None, flags: 0, data: vec![1, 2, 3], service_name: None };
    assert_eq!(wfp::provider_add(e, p.clone()), 0);
    let n: Vec<u16> = wfp::shared_mapping_name().encode_utf16().chain(Some(0)).collect();
    unsafe {
        let h = OpenFileMappingW(0x0004, 0, n.as_ptr());
        assert!(!h.is_null());
        let v = MapViewOfFile(h, 0x0004, 0, 0, 0) as *const u8;
        assert!(!v.is_null());
        let len = u32::from_le_bytes(core::slice::from_raw_parts(v.add(4), 4).try_into().unwrap()) as usize;
        let next = u64::from_le_bytes(core::slice::from_raw_parts(v.add(8), 8).try_into().unwrap());
        assert!(next >= e2);
        let arena = core::slice::from_raw_parts(v.add(16), len);
        let wide: Vec<u8> = name.iter().flat_map(|c| c.to_le_bytes()).collect();
        assert!(arena.windows(16).any(|w| w == key));
        assert!(arena.windows(wide.len()).any(|w| w == wide.as_slice()));
        UnmapViewOfFile(v as *const _);
        CloseHandle(h);
    }
    assert_eq!(wfp::provider_get_by_key(e2, &key), Ok(p));
    assert_eq!(wfp::engine_close(e), 0);
    assert_eq!(wfp::provider_get_by_key(e2, &key), Err(wfp::FWP_E_PROVIDER_NOT_FOUND));
    assert_eq!(wfp::engine_close(e2), 0);
}

fn named(n: &str) -> [u8; 16] {
    super::wfp_names::NAMES.iter().find(|(_, x)| *x == n).map(|(k, _)| *k).unwrap()
}

fn wide(s: &str) -> Option<Vec<u16>> {
    Some(s.encode_utf16().collect())
}

fn test_filter(key: u8, layer: [u8; 16], sublayer: [u8; 16], weight: wfp::Value) -> wfp::Filter {
    wfp::Filter {
        key: [key; 16],
        name: wide("f"),
        description: None,
        flags: 0,
        provider_key: None,
        data: Vec::new(),
        layer,
        sublayer,
        weight,
        conditions: vec![wfp::Condition { field: named("FWPM_CONDITION_IP_REMOTE_ADDRESS"), match_type: 0, value: wfp::Value::U32(0x0a00_0001) }],
        action_type: 0x1001,
        action_key: [0; 16],
        context: [0; 16],
        reserved: None,
        id: 0,
        effective_weight: wfp::Value::Empty,
    }
}

#[test]
fn wfp_store_adds_sublayers_callouts_contexts_and_filters_with_console_checks() {
    let _g = wfp::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let e = wfp::engine_open(wfp::FWPM_SESSION_FLAG_DYNAMIC).unwrap();
    let connect = named("FWPM_LAYER_ALE_AUTH_CONNECT_V4");
    let transport = named("FWPM_LAYER_INBOUND_TRANSPORT_V4");
    let pkey = [0x71u8; 16];
    let sl = wfp::SubLayer { key: [0x72; 16], name: wide("s"), description: None, flags: 0, provider_key: Some(pkey), data: vec![1, 2, 3], weight: 0x10 };
    assert_eq!(wfp::sublayer_add(e, sl.clone()), wfp::FWP_E_PROVIDER_NOT_FOUND);
    assert_eq!(wfp::provider_add(e, wfp::Provider { key: pkey, name: wide("p"), description: None, flags: 0, data: Vec::new(), service_name: None }), 0);
    assert_eq!(wfp::sublayer_add(e, wfp::SubLayer { name: None, ..sl.clone() }), wfp::ERROR_INVALID_PARAMETER);
    assert_eq!(wfp::sublayer_add(e, wfp::SubLayer { key: named("FWPM_SUBLAYER_UNIVERSAL"), ..sl.clone() }), wfp::FWP_E_ALREADY_EXISTS);
    assert_eq!(wfp::sublayer_add(e, wfp::SubLayer { flags: wfp::FWPM_SUBLAYER_FLAG_PERSISTENT, ..sl.clone() }), wfp::FWP_E_DYNAMIC_SESSION_IN_PROGRESS);
    assert_eq!(wfp::sublayer_add(e, sl.clone()), 0);
    assert_eq!(wfp::sublayer_get_by_key(e, &sl.key), Ok(sl.clone()));
    let co = wfp::Callout { key: [0x73; 16], name: wide("c"), description: None, flags: 0, provider_key: None, data: Vec::new(), layer: [0x99; 16], id: 0 };
    assert_eq!(wfp::callout_add(e, co.clone()), Err(wfp::FWP_E_LAYER_NOT_FOUND));
    let id1 = wfp::callout_add(e, wfp::Callout { layer: connect, ..co.clone() }).unwrap();
    assert_eq!(wfp::callout_get_by_key(e, &co.key).unwrap().id, id1);
    let ctx = wfp::ProviderContext { key: [0x74; 16], name: wide("x"), description: None, flags: 0, provider_key: None, data: Vec::new(), kind: wfp::FWPM_GENERAL_CONTEXT, body: wfp::ContextBody::General(vec![9]), id: 0 };
    assert_eq!(wfp::provider_context_add(e, wfp::ProviderContext { body: wfp::ContextBody::Null, ..ctx.clone() }), Err(wfp::ERROR_INVALID_PARAMETER));
    assert_eq!(wfp::provider_context_add(e, wfp::ProviderContext { kind: 13, ..ctx.clone() }), Err(wfp::ERROR_INVALID_PARAMETER));
    let cid = wfp::provider_context_add(e, ctx.clone()).unwrap();
    assert_eq!(wfp::provider_context_get_by_key(e, &ctx.key).unwrap().id, cid);
    assert_eq!(wfp::filter_add(e, test_filter(1, [0x99; 16], [0; 16], wfp::Value::Empty)), Err(wfp::FWP_E_LAYER_NOT_FOUND));
    assert_eq!(wfp::filter_add(e, test_filter(1, connect, [0x98; 16], wfp::Value::Empty)), Err(wfp::FWP_E_SUBLAYER_NOT_FOUND));
    assert_eq!(wfp::filter_add(e, test_filter(1, connect, [0; 16], wfp::Value::U32(1))), Err(wfp::ERROR_INVALID_PARAMETER));
    assert_eq!(wfp::filter_add(e, wfp::Filter { action_type: 0x5003, action_key: [0x97; 16], ..test_filter(1, connect, [0; 16], wfp::Value::Empty) }), Err(wfp::FWP_E_CALLOUT_NOT_FOUND));
    assert_eq!(wfp::filter_add(e, wfp::Filter { flags: wfp::FWPM_FILTER_FLAG_HAS_PROVIDER_CONTEXT, context: [0x96; 16], ..test_filter(1, connect, [0; 16], wfp::Value::Empty) }), Err(wfp::FWP_E_PROVIDER_CONTEXT_NOT_FOUND));
    let a = wfp::filter_add(e, wfp::Filter { action_type: 0x5003, action_key: co.key, ..test_filter(1, connect, sl.key, wfp::Value::U64(7)) }).unwrap();
    let b = wfp::filter_add(e, wfp::Filter { flags: wfp::FWPM_FILTER_FLAG_HAS_PROVIDER_CONTEXT, context: ctx.key, ..test_filter(2, transport, named("FWPM_SUBLAYER_UNIVERSAL"), wfp::Value::U8(3)) }).unwrap();
    let c = wfp::filter_add(e, test_filter(3, transport, [0; 16], wfp::Value::Empty)).unwrap();
    assert_eq!((b, c), (a + 1, a + 2));
    assert_eq!(wfp::filter_get_by_key(e, &[1; 16]).unwrap().effective_weight, wfp::Value::U64(7));
    match wfp::filter_get_by_key(e, &[2; 16]).unwrap().effective_weight {
        wfp::Value::U64(w) => assert_eq!(w >> 60, 3),
        other => panic!("{:?}", other),
    }
    assert!(matches!(wfp::filter_get_by_key(e, &[3; 16]).unwrap().effective_weight, wfp::Value::U64(_)));
    assert_eq!(wfp::filter_add(e, test_filter(3, transport, [0; 16], wfp::Value::Empty)), Err(wfp::FWP_E_ALREADY_EXISTS));
    let by_layer = |l: Option<[u8; 16]>| {
        let h = wfp::create_filter_enum_handle(e, l).unwrap();
        let v: Vec<u64> = wfp::enum_next(e, h, 0x40).unwrap().iter().map(|o| match o { wfp::Object::Filter(f) => f.id, _ => panic!() }).collect();
        assert_eq!(wfp::destroy_enum_handle(e, h), 0);
        v
    };
    assert_eq!(by_layer(Some(connect)), vec![a]);
    assert_eq!(by_layer(Some(transport)), vec![b, c]);
    assert_eq!(by_layer(None), vec![a, b, c]);
    assert_eq!(wfp::create_filter_enum_handle(e, Some([0x99; 16])), Err(wfp::FWP_E_LAYER_NOT_FOUND));
    assert_eq!(wfp::sublayer_delete_by_key(e, &sl.key), wfp::FWP_E_IN_USE);
    assert_eq!(wfp::callout_delete_by_key(e, &co.key), wfp::FWP_E_IN_USE);
    assert_eq!(wfp::provider_context_delete_by_key(e, &ctx.key), wfp::FWP_E_IN_USE);
    assert_eq!(wfp::provider_delete_by_key(e, &pkey), wfp::FWP_E_IN_USE);
    assert_eq!(wfp::filter_delete_by_key(e, &[1; 16]), 0);
    assert_eq!(wfp::filter_delete_by_id(e, b), 0);
    assert_eq!(wfp::filter_delete_by_id(e, b), wfp::FWP_E_FILTER_NOT_FOUND);
    assert_eq!(wfp::sublayer_delete_by_key(e, &sl.key), 0);
    assert_eq!(wfp::callout_delete_by_key(e, &co.key), 0);
    assert_eq!(wfp::provider_context_delete_by_key(e, &ctx.key), 0);
    assert_eq!(wfp::object_count(wfp::Kind::Filter), 1);
    assert_eq!(wfp::engine_close(e), 0);
    for k in [wfp::Kind::Provider, wfp::Kind::SubLayer, wfp::Kind::Callout, wfp::Kind::Filter, wfp::Kind::ProviderContext] {
        assert_eq!(wfp::object_count(k), 0);
    }
}

mod iphlp_model {
    use super::super::iphlp::*;
    use super::super::*;
    use super::{rd32, rd64};
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

    fn nic() -> Adapter {
        adapters().into_iter().find(|a| a.if_index == NIC_IF_INDEX).unwrap()
    }

    fn v6sock(ip: Ipv6Addr) -> [u8; 28] {
        let mut b = [0u8; 28];
        b[0] = 0x17;
        b[8..24].copy_from_slice(&ip.octets());
        b
    }

    #[test]
    fn if_table2_rows_follow_mib_if_row2_layout() {
        let t = if_table2();
        assert_eq!(rd32(&t, 0), 2);
        assert_eq!(t.len(), 8 + 2 * IF_ROW2_SIZE);
        let r = &t[8..8 + IF_ROW2_SIZE];
        let a = nic();
        assert_eq!(rd64(r, 0), NIC_LUID);
        assert_eq!(rd32(r, 8), NIC_IF_INDEX);
        assert_eq!(&r[0xc..0x1c], &interface_guid(&a));
        assert_eq!(interface_guid(&a), parse_guid("{5A6F8E2C-1B3D-4E59-9C70-546F6D426F78}").unwrap());
        assert_eq!(&r[0xc..0x10], &[0x2c, 0x8e, 0x6f, 0x5a]);
        let alias: Vec<u16> = r[0x1c..0x1c + 16].chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        assert_eq!(String::from_utf16_lossy(&alias), "Ethernet");
        assert_eq!(rd32(r, 0x420), 6);
        assert_eq!(&r[0x424..0x42a], &NIC_MAC);
        assert_eq!(&r[0x444..0x44a], &NIC_MAC);
        assert_eq!(rd32(r, 0x464), 1500);
        assert_eq!(rd32(r, 0x468), IF_TYPE_ETHERNET_CSMACD);
        assert_eq!(rd32(r, 0x474), 14);
        assert_eq!(rd32(r, 0x478), 2);
        assert_eq!(r[0x480], 5);
        assert_eq!(rd32(r, 0x484), 1);
        assert_eq!(rd32(r, 0x488), 1);
        assert_eq!(rd32(r, 0x48c), 1);
        assert_eq!(&r[0x490..0x4a0], &a.network_guid);
        assert_eq!(rd64(r, 0x4a8), 1_000_000_000);
        assert!(r[0x4b8..].iter().all(|b| *b == 0));
        let l = &t[8 + IF_ROW2_SIZE..];
        assert_eq!(rd64(l, 0), LOOPBACK_LUID);
        assert_eq!(rd32(l, 0x478), 1);
        assert_eq!(rd32(l, 0x420), 0);
    }

    #[test]
    fn if_entry_lookups_use_luid_then_index_and_report_missing_rows() {
        unsafe {
            let mut row = vec![0u8; IF_ROW2_SIZE];
            assert_eq!(get_if_entry2(row.as_mut_ptr()), ERROR_INVALID_PARAMETER);
            row[8..12].copy_from_slice(&LOOPBACK_IF_INDEX.to_le_bytes());
            assert_eq!(get_if_entry2(row.as_mut_ptr()), ERROR_SUCCESS);
            assert_eq!(rd64(&row, 0), LOOPBACK_LUID);
            let mut row = vec![0u8; IF_ROW2_SIZE];
            row[0..8].copy_from_slice(&NIC_LUID.to_le_bytes());
            row[8..12].copy_from_slice(&LOOPBACK_IF_INDEX.to_le_bytes());
            assert_eq!(get_if_entry2_ex(MIB_IF_ENTRY_NORMAL_WITHOUT_STATISTICS, row.as_mut_ptr()), ERROR_SUCCESS);
            assert_eq!(rd32(&row, 8), NIC_IF_INDEX);
            assert_eq!(get_if_entry2_ex(1, row.as_mut_ptr()), ERROR_INVALID_PARAMETER);
            row[0..8].copy_from_slice(&0x1234u64.to_le_bytes());
            assert_eq!(get_if_entry2(row.as_mut_ptr()), ERROR_FILE_NOT_FOUND);
            let mut old = vec![0u8; IF_ROW_SIZE];
            old[0x200..0x204].copy_from_slice(&NIC_IF_INDEX.to_le_bytes());
            assert_eq!(get_if_entry(old.as_mut_ptr()), ERROR_SUCCESS);
            let name: Vec<u16> = old[0..0x200].chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|c| *c != 0).collect();
            assert_eq!(String::from_utf16_lossy(&name), "\\DEVICE\\TCPIP_{5A6F8E2C-1B3D-4E59-9C70-546F6D426F78}");
            assert_eq!(rd32(&old, 0x210), 6);
            assert_eq!(rd32(&old, 0x220), MIB_IF_OPER_STATUS_OPERATIONAL);
            assert_eq!(&old[0x25c..0x25c + 6], b"TomBox");
            old[0x200..0x204].copy_from_slice(&99u32.to_le_bytes());
            assert_eq!(get_if_entry(old.as_mut_ptr()), ERROR_INVALID_PARAMETER);
            let mut tp: *mut u8 = core::ptr::null_mut();
            assert_eq!(get_if_table2(&mut tp), ERROR_SUCCESS);
            let t = core::slice::from_raw_parts(tp, 8 + 2 * IF_ROW2_SIZE);
            assert_eq!(t, &if_table2()[..]);
            free_mib_table(tp as *mut core::ffi::c_void);
            free_mib_table(core::ptr::null_mut());
        }
    }

    #[test]
    fn guid_luid_index_conversions_and_best_interface() {
        unsafe {
            let g = interface_guid(&nic());
            let mut luid = 0u64;
            assert_eq!(convert_interface_guid_to_luid(&g, &mut luid), ERROR_SUCCESS);
            assert_eq!(luid, NIC_LUID);
            assert_eq!(convert_interface_guid_to_luid(&[9u8; 16], &mut luid), ERROR_NOT_FOUND);
            let mut idx = 0u32;
            assert_eq!(convert_interface_luid_to_index(&LOOPBACK_LUID, &mut idx), ERROR_SUCCESS);
            assert_eq!(idx, LOOPBACK_IF_INDEX);
            assert_eq!(convert_interface_luid_to_index(&7u64, &mut idx), ERROR_NOT_FOUND);
            assert_eq!(convert_interface_luid_to_index(core::ptr::null(), &mut idx), ERROR_INVALID_PARAMETER);
            let s = sockaddr_bytes(&IpAddr::V4(Ipv4Addr::new(169, 254, 3, 4)), 0, 0);
            assert_eq!(get_best_interface_ex(s.as_ptr(), &mut idx), ERROR_SUCCESS);
            assert_eq!(idx, NIC_IF_INDEX);
            let s = sockaddr_bytes(&IpAddr::V4(Ipv4Addr::LOCALHOST), 0, 0);
            assert_eq!(get_best_interface_ex(s.as_ptr(), &mut idx), ERROR_SUCCESS);
            assert_eq!(idx, LOOPBACK_IF_INDEX);
            let s = sockaddr_bytes(&IpAddr::V4(Ipv4Addr::new(198, 41, 0, 4)), 0, 0);
            assert_eq!(get_best_interface_ex(s.as_ptr(), &mut idx), ERROR_NETWORK_UNREACHABLE);
            let s = v6sock(Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 9));
            assert_eq!(get_best_interface_ex(s.as_ptr(), &mut idx), ERROR_SUCCESS);
            assert_eq!(idx, NIC_IF_INDEX);
            let s = v6sock(Ipv4Addr::new(169, 254, 66, 1).to_ipv6_mapped());
            assert_eq!(get_best_interface_ex(s.as_ptr(), &mut idx), ERROR_SUCCESS);
            assert_eq!(idx, NIC_IF_INDEX);
        }
    }

    #[test]
    fn physical_interface_for_destination_is_unreachable_offline_and_names_on_link_interfaces() {
        unsafe {
            let mut out = [0u8; 16];
            let mut h = 5usize;
            let root4 = v6sock(Ipv4Addr::new(198, 41, 0, 4).to_ipv6_mapped());
            assert_eq!(physical_interface_for_destination(root4.as_ptr(), &mut out, &mut h), ERROR_NETWORK_UNREACHABLE);
            assert_eq!(h, 0);
            let root6 = v6sock(Ipv6Addr::new(0x2001, 0x503, 0xba3e, 0, 0, 0, 2, 0x30));
            assert_eq!(physical_interface_for_destination(root6.as_ptr(), &mut out, &mut h), ERROR_NETWORK_UNREACHABLE);
            let link = sockaddr_bytes(&IpAddr::V4(Ipv4Addr::new(169, 254, 9, 9)), 0, 0);
            assert_eq!(physical_interface_for_destination(link.as_ptr(), &mut out, &mut h), ERROR_SUCCESS);
            assert_eq!(out, interface_guid(&nic()));
            assert_eq!(close_physical_interface_for_destination(&mut h), ERROR_SUCCESS);
            let mut bogus = 0x44usize;
            assert_eq!(close_physical_interface_for_destination(&mut bogus), ERROR_INVALID_HANDLE);
        }
    }

    fn sort_list(addrs: &[&[u8]]) -> Vec<u8> {
        let mut list = vec![0u8; 8 + 16 * addrs.len()];
        list[0..4].copy_from_slice(&(addrs.len() as i32).to_le_bytes());
        for (i, a) in addrs.iter().enumerate() {
            list[8 + 16 * i..16 + 16 * i].copy_from_slice(&(a.as_ptr() as usize).to_le_bytes());
            list[16 + 16 * i..20 + 16 * i].copy_from_slice(&(a.len() as i32).to_le_bytes());
        }
        list
    }

    fn order(list: &[u8]) -> Vec<usize> {
        let n = rd32(list, 0) as usize;
        (0..n).map(|i| usize::from_le_bytes(list[8 + 16 * i..16 + 16 * i].try_into().unwrap())).collect()
    }

    #[test]
    fn address_list_sort_follows_rfc6724_against_the_emulated_view() {
        let root4 = v6sock(Ipv4Addr::new(198, 41, 0, 4).to_ipv6_mapped());
        let link4 = v6sock(Ipv4Addr::new(169, 254, 7, 7).to_ipv6_mapped());
        let ll6 = v6sock(Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 7));
        let lo6 = v6sock(Ipv6Addr::LOCALHOST);
        let mut list = sort_list(&[&root4, &link4, &ll6, &lo6]);
        let len = list.len() as u32;
        assert_eq!(unsafe { address_list_sort(list.as_mut_ptr(), len) }, Ok(len));
        assert_eq!(order(&list), vec![lo6.as_ptr() as usize, ll6.as_ptr() as usize, link4.as_ptr() as usize, root4.as_ptr() as usize]);
        let root6 = v6sock(Ipv6Addr::new(0x2001, 0x503, 0xba3e, 0, 0, 0, 2, 0x30));
        let mut both = sort_list(&[&root4, &root6]);
        assert_eq!(unsafe { address_list_sort(both.as_mut_ptr(), both.len() as u32) }, Ok(both.len() as u32));
        assert_eq!(order(&both), vec![root4.as_ptr() as usize, root6.as_ptr() as usize]);
        assert_eq!(policy(&IpAddr::V6(Ipv6Addr::LOCALHOST)), Policy { precedence: 50, label: 0 });
        assert_eq!(policy(&IpAddr::V4(Ipv4Addr::new(1, 2, 3, 4))), Policy { precedence: 35, label: 4 });
        assert_eq!(scope(&IpAddr::V4(Ipv4Addr::new(169, 254, 1, 1))), 2);
        assert_eq!(scope(&IpAddr::V6(Ipv6Addr::new(0xff05, 0, 0, 0, 0, 0, 0, 1))), 5);
        let v4 = sockaddr_bytes(&IpAddr::V4(Ipv4Addr::LOCALHOST), 0, 0);
        list[8..16].copy_from_slice(&(v4.as_ptr() as usize).to_le_bytes());
        assert_eq!(unsafe { address_list_sort(list.as_mut_ptr(), len) }, Err(ERROR_INVALID_PARAMETER));
        assert_eq!(unsafe { address_list_sort(list.as_mut_ptr(), 20) }, Err(ERROR_INVALID_PARAMETER));
    }

    #[test]
    fn nsi_interface_rw_writes_merge_unchanged_groups() {
        let _g = super::super::sockets::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        nsi_reset();
        let mut rw = vec![0xffu8; NL_INTERFACE_RW_SIZE];
        rw[0x10..0x14].copy_from_slice(&1u32.to_le_bytes());
        rw[0xc0..0xd0].copy_from_slice(&[7u8; 16]);
        let key = NIC_LUID.to_le_bytes();
        let n = NL_INTERFACE_RW_SIZE as u32;
        unsafe {
            assert_eq!(nsi_set_all_parameters(NSI_STORE_ACTIVE, NSI_SET_DEFAULT, NPI_MS_IPV4_MODULEID.as_ptr(), NL_INTERFACE_OBJECT, key.as_ptr(), 8, rw.as_ptr(), n), ERROR_SUCCESS);
            let mut rw2 = vec![0xffu8; NL_INTERFACE_RW_SIZE];
            rw2[0x10..0x14].copy_from_slice(&2u32.to_le_bytes());
            assert_eq!(nsi_set_all_parameters(NSI_STORE_ACTIVE, NSI_SET_DEFAULT, NPI_MS_IPV4_MODULEID.as_ptr(), NL_INTERFACE_OBJECT, key.as_ptr(), 8, rw2.as_ptr(), n), ERROR_SUCCESS);
            let bad = 5u64.to_le_bytes();
            assert_eq!(nsi_set_all_parameters(NSI_STORE_ACTIVE, NSI_SET_DEFAULT, NPI_MS_IPV4_MODULEID.as_ptr(), NL_INTERFACE_OBJECT, bad.as_ptr(), 8, rw.as_ptr(), n), ERROR_NOT_FOUND);
            assert_eq!(nsi_set_all_parameters(NSI_STORE_ACTIVE, NSI_SET_DEFAULT, NPI_MS_TEREDO_MODULEID.as_ptr(), NL_INTERFACE_OBJECT, key.as_ptr(), 8, rw.as_ptr(), n), ERROR_NOT_FOUND);
            assert_eq!(nsi_set_all_parameters(NSI_STORE_ACTIVE, NSI_SET_DEFAULT, NPI_MS_IPV4_MODULEID.as_ptr(), NL_INTERFACE_OBJECT, key.as_ptr(), 8, rw.as_ptr(), 0x10), ERROR_INVALID_PARAMETER);
        }
        let got = nsi_interface_rw(&NPI_MS_IPV4_MODULEID, NIC_LUID).unwrap();
        assert_eq!(rd32(&got, 0x10), 2);
        assert_eq!(&got[0xc0..0xd0], &[7u8; 16]);
        assert!(got[0x20..0x30].iter().all(|b| *b == 0xff));
        assert!(nsi_interface_rw(&NPI_MS_IPV6_MODULEID, NIC_LUID).is_none());
        nsi_reset();
    }
}

mod nsi_read {
    use super::super::iphlp::*;
    use super::super::*;
    use super::rd32;

    #[test]
    fn nsi_get_all_parameters_returns_written_interface_rows() {
        let _g = super::super::sockets::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        nsi_reset();
        let key = NIC_LUID.to_le_bytes();
        let mut out = vec![0u8; NL_INTERFACE_RW_SIZE];
        let n = NL_INTERFACE_RW_SIZE as u32;
        let null = core::ptr::null_mut();
        unsafe {
            assert_eq!(nsi_get_all_parameters(1, NPI_MS_IPV6_MODULEID.as_ptr(), NL_INTERFACE_OBJECT, key.as_ptr(), 8, out.as_mut_ptr(), n, null, 0, null, 0), ERROR_NOT_FOUND);
            let mut rw = vec![0xffu8; NL_INTERFACE_RW_SIZE];
            rw[0xbc..0xc0].copy_from_slice(&9u32.to_le_bytes());
            assert_eq!(nsi_set_all_parameters(NSI_STORE_ACTIVE, NSI_SET_DEFAULT, NPI_MS_IPV6_MODULEID.as_ptr(), NL_INTERFACE_OBJECT, key.as_ptr(), 8, rw.as_ptr(), n), ERROR_SUCCESS);
            assert_eq!(nsi_get_all_parameters(1, NPI_MS_IPV6_MODULEID.as_ptr(), NL_INTERFACE_OBJECT, key.as_ptr(), 8, out.as_mut_ptr(), n, null, 0, null, 0), ERROR_SUCCESS);
            assert_eq!(rd32(&out, 0xbc), 9);
            assert_eq!(nsi_get_all_parameters(1, NPI_MS_IPV4_MODULEID.as_ptr(), NL_INTERFACE_OBJECT, key.as_ptr(), 8, out.as_mut_ptr(), n, null, 0, null, 0), ERROR_NOT_FOUND);
            assert_eq!(nsi_get_all_parameters(1, NPI_MS_IPV6_MODULEID.as_ptr(), NL_INTERFACE_OBJECT, key.as_ptr(), 8, out.as_mut_ptr(), 4, null, 0, null, 0), ERROR_INVALID_PARAMETER);
            let mut rod = [0u8; 16];
            assert_eq!(nsi_get_all_parameters(1, NPI_MS_IPV6_MODULEID.as_ptr(), NL_INTERFACE_OBJECT, key.as_ptr(), 8, out.as_mut_ptr(), n, rod.as_mut_ptr(), 16, null, 0), ERROR_NOT_SUPPORTED);
            assert_eq!(nsi_get_all_parameters(1, NPI_MS_TEREDO_MODULEID.as_ptr(), NSI_TEREDO_OBJECT_STATE, core::ptr::null(), 0, out.as_mut_ptr(), 12, null, 0, null, 0), ERROR_SUCCESS);
        }
        nsi_reset();
    }
}
