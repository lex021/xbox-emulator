use super::capture::{self, Record};
use super::guids::guid_text;
use super::wfp::{self as section, G_FORMAT, G_SESSION};
use super::*;
use crate::trace::{self, WFP_GUID};
use compat_common::netview::sockets;
use compat_common::netview::wfp as store;
use std::os::windows::io::AsRawSocket;

fn ids(v: &[Record], g: &GUID) -> Vec<u16> {
    v.iter().filter(|r| r.guid == *g).map(|r| r.id).collect()
}

fn wtext(b: &[u8]) -> String {
    let w: Vec<u16> = b.chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    assert_eq!(w.last(), Some(&0));
    String::from_utf16(&w[..w.len() - 1]).unwrap()
}

fn atext(b: &[u8]) -> String {
    assert_eq!(b.last(), Some(&0));
    String::from_utf8(b[..b.len() - 1].to_vec()).unwrap()
}

fn d32(b: &[u8]) -> u32 {
    assert_eq!(b.len(), 4);
    u32::from_le_bytes(b.try_into().unwrap())
}

fn d64(b: &[u8]) -> u64 {
    assert_eq!(b.len(), 8);
    u64::from_le_bytes(b.try_into().unwrap())
}

fn find<'a>(v: &'a [Record], g: &GUID, id: u16) -> Vec<&'a Record> {
    v.iter().filter(|r| r.guid == *g && r.id == id).collect()
}

static NETSTATE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn lock() -> std::sync::MutexGuard<'static, ()> {
    NETSTATE_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

fn run<T>(f: impl FnOnce() -> T) -> (T, Vec<Record>) {
    capture::start();
    let out = f();
    (out, capture::take())
}

#[test]
fn record_arguments_use_the_wpp_wire_encoding() {
    let _g = lock();
    let (_, v) = run(|| unsafe { rec(Gate::State, &ADAPTERS_GUID, 0x33, &[Z, Arg::W(None), Arg::A(None), w(&ws("ab")), Arg::A(Some(b"x")), r(5), r(0), u(7)]) });
    assert_eq!(v.len(), 1);
    let p = &v[0].parts;
    assert_eq!(p[0], vec![0u8; 8]);
    assert_eq!(p[1], b"N\0U\0L\0L\0\0\0".to_vec());
    assert_eq!(p[2], b"NULL\0".to_vec());
    assert_eq!(p[3], b"a\0b\0\0\0".to_vec());
    assert_eq!(p[4], b"x\0".to_vec());
    assert_eq!(d64(&p[5]), REDACTED);
    assert_eq!(d64(&p[6]), 0);
    assert_eq!(d32(&p[7]), 7);
    assert_eq!(hr_map(0x15), 0x8007_0015);
    assert_eq!(hr_map(0x8032_0005), 0x8032_0005);
    assert_eq!(hr_map(0), 0);
}

#[test]
fn state_records_use_the_c3d66fc7_block_and_stay_quiet_without_it() {
    let _g = lock();
    assert_eq!(trace::STATE_CONTROL_GUID, GUID::new(0xc3d66fc7, 0x5736, 0x46d5, [0xb9, 0x42, 0xc4, 0x04, 0xf5, 0x02, 0xdd, 0xe4]));
    let _t = crate::tests::TRACE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    assert!(!capture::active());
    assert_eq!(trace::WPP_STATE.logger.load(Ordering::Acquire), 0);
    assert!(!enabled(Gate::State));
    unsafe {
        assert!(!trace::wpp_message(&trace::STATE_HEADER_GUID, 0xe, 0));
    }
}

#[test]
fn adapters_section_dumps_the_emulated_nic_loopback_and_on_link_routes() {
    let _g = lock();
    let (hr, v) = run(|| unsafe { log_adapters() });
    assert_eq!(hr, 0);
    assert!(v.iter().all(|r| r.guid == ADAPTERS_GUID && r.gate == Gate::State));
    assert_eq!(v.first().unwrap().id, 0x92);
    let last = v.last().unwrap();
    assert_eq!(last.id, 0x99);
    assert_eq!(d32(&last.parts[1]), 0);
    assert_eq!(find(&v, &ADAPTERS_GUID, 0x96).len(), 2);
    let names: Vec<String> = find(&v, &ADAPTERS_GUID, 0x35).iter().map(|r| wtext(&r.parts[1])).collect();
    assert_eq!(names, vec!["Ethernet".to_string(), "Loopback Pseudo-Interface 1".to_string()]);
    let descs: Vec<String> = find(&v, &ADAPTERS_GUID, 0x34).iter().map(|r| wtext(&r.parts[1])).collect();
    assert_eq!(descs[0], "TomBox Emulated Console Network Adapter");
    assert!(find(&v, &ADAPTERS_GUID, 0x13).iter().all(|r| atext(&r.parts[1]).starts_with('{')));
    let mac = find(&v, &ADAPTERS_GUID, 0x36);
    assert_eq!(mac.len(), 1);
    let bytes: Vec<u8> = mac[0].parts[1..].iter().map(|b| d32(b) as u8).collect();
    assert_eq!(bytes, netview::NIC_MAC.to_vec());
    assert_eq!(find(&v, &ADAPTERS_GUID, 0x38).len(), 1);
    let unicast: Vec<String> = find(&v, &ADAPTERS_GUID, 0x0e).iter().filter(|r| wtext(&r.parts[1]) == SP12).map(|r| wtext(&r.parts[3])).collect();
    assert!(unicast.contains(&"169.254.66.1".to_string()));
    assert!(unicast.contains(&"127.0.0.1".to_string()));
    assert!(unicast.contains(&"::1".to_string()));
    assert!(unicast.iter().any(|s| s.starts_with("fe80::54:42ff:fe00:1")));
    assert!(find(&v, &ADAPTERS_GUID, 0x1a).iter().any(|r| atext(&r.parts[2]) == "WellKnown"));
    assert!(find(&v, &ADAPTERS_GUID, 0x1c).iter().all(|r| atext(&r.parts[2]) == "Preferred"));
    assert!(find(&v, &ADAPTERS_GUID, 0x27).iter().all(|r| d64(&r.parts[1]) == 0));
    let dhcp: Vec<String> = find(&v, &ADAPTERS_GUID, 0x0e).iter().filter(|r| wtext(&r.parts[1]) == SP8).map(|r| wtext(&r.parts[3])).collect();
    assert_eq!(dhcp, vec!["nullptr".to_string(); 4]);
    let routes = netview::routes();
    let summary = find(&v, &ADAPTERS_GUID, 0x98);
    assert_eq!(summary.len(), 1);
    assert_eq!(d32(&summary[0].parts[1]) as usize, routes.len());
    assert_eq!(find(&v, &ADAPTERS_GUID, 0x71).len(), routes.len());
    assert!(find(&v, &ADAPTERS_GUID, 0x61).iter().all(|r| wtext(&r.parts[1]) == SP8));
    let dests: Vec<String> = find(&v, &ADAPTERS_GUID, 0x0c).iter().filter(|r| wtext(&r.parts[1]) == SP16).map(|r| wtext(&r.parts[3])).collect();
    assert_eq!(dests.len(), routes.len());
    assert!(dests.contains(&"169.254.0.0".to_string()));
    assert!(!dests.contains(&"0.0.0.0".to_string()));
    assert!(v.iter().all(|r| r.gate != Gate::Error));
}

#[test]
fn teredo_section_reports_the_offline_state_and_tolerates_the_missing_port() {
    let _g = lock();
    let (hr, v) = run(|| unsafe { log_teredo() });
    assert_eq!(hr, 0);
    assert_eq!(ids(&v, &TEREDO_GUID), vec![0x12, 0x16, 0x18, 0x1b]);
    let s = &v[2].parts;
    assert_eq!(s.len(), 6);
    assert_eq!(d32(&s[1]), netview::TEREDO_STATE_OFFLINE);
    assert!(s[2..].iter().all(|b| d32(b) == 0));
    assert_eq!(d32(&v[3].parts[1]), 0);
    assert!(v.iter().all(|r| r.gate == Gate::State));
}

#[test]
fn processes_section_lists_title_processes_and_their_modules() {
    let _g = lock();
    let (hr, v) = run(|| unsafe { log_processes() });
    assert_eq!(hr, 0);
    assert_eq!(v.first().unwrap().id, 0x1d);
    assert_eq!(v.last().unwrap().id, 0x21);
    let listed = d32(&find(&v, &PROCESSES_GUID, 0x1f)[0].parts[1]) as usize;
    assert!(listed >= 1);
    assert_eq!(find(&v, &PROCESSES_GUID, 0x20).len(), listed);
    let me = std::process::id();
    let row = find(&v, &PROCESSES_GUID, 0x20).into_iter().find(|r| d32(&r.parts[1]) == me).expect("current process row");
    let exe = std::env::current_exe().unwrap();
    let leaf = exe.file_name().unwrap().to_string_lossy().to_lowercase();
    let image = wtext(&row.parts[3]).to_lowercase();
    assert!(image.ends_with(&format!("\\{}", leaf)), "{}", image);
    assert!(std::path::Path::new(&image).is_file());
    let mods: Vec<String> = find(&v, &PROCESSES_GUID, 0x19).iter().map(|r| wtext(&r.parts[2]).to_lowercase()).collect();
    assert!(mods.iter().any(|m| m == "ntdll.dll"));
    assert!(mods.iter().any(|m| m == "kernel32.dll"));
    assert!(find(&v, &PROCESSES_GUID, 0x0b).iter().all(|r| d32(&r.parts[1]) % 0x100 == 0));
    unsafe {
        assert_eq!(wtext(&encode(&w(&process_name(0)))), "<invalid process ID>");
        assert_eq!(wtext(&encode(&w(&process_name(4)))), "<system process ID>");
    }
    let (name, ev) = run(|| unsafe { process_name(0x7fff_fff0) });
    assert_eq!(String::from_utf16(&name).unwrap(), "<couldn't open process, err 87>");
    assert_eq!(ids(&ev, &PROCESSES_GUID), vec![0x1a]);
    assert_eq!(ev[0].gate, Gate::Error);
}

#[test]
fn ports_section_lists_sockets_the_title_opened() {
    let _g = lock();
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let c = std::net::TcpStream::connect(l.local_addr().unwrap()).unwrap();
    let ud = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let peer = l.local_addr().unwrap();
    let sa = netview::sockaddr_bytes(&peer.ip(), peer.port(), 0);
    unsafe {
        assert!(sockets::report_socket(c.as_raw_socket() as usize, sa.as_ptr(), sa.len() as i32));
        assert!(sockets::report_socket(ud.as_raw_socket() as usize, core::ptr::null(), 0));
    }
    let (hr, v) = run(|| unsafe { log_ports() });
    assert_eq!(hr, 0);
    assert_eq!(v.first().unwrap().id, 0x3b);
    assert_eq!(v.last().unwrap().id, 0x44);
    for id in [0x3d, 0x3f, 0x41, 0x43] {
        assert_eq!(find(&v, &PORTS_GUID, id).len(), 1);
    }
    let pre = SP8.to_string();
    let states = find(&v, &PORTS_GUID, 0x0a);
    assert!(states.iter().all(|r| wtext(&r.parts[1]) == pre));
    assert!(states.iter().any(|r| d32(&r.parts[2]) == 5 && wtext(&r.parts[3]) == "MIB_TCP_STATE_ESTAB"));
    let lports: Vec<u32> = find(&v, &PORTS_GUID, 0x0c).iter().map(|r| d32(&r.parts[2])).collect();
    assert!(lports.contains(&(c.local_addr().unwrap().port() as u32)));
    let rports: Vec<u32> = find(&v, &PORTS_GUID, 0x0e).iter().map(|r| d32(&r.parts[2])).collect();
    assert!(rports.contains(&(peer.port() as u32)));
    assert!(find(&v, &PORTS_GUID, 0x0b).iter().any(|r| wtext(&r.parts[2]) == "127.0.0.1"));
    let me = std::process::id();
    assert!(find(&v, &PORTS_GUID, 0x0f).iter().all(|r| d32(&r.parts[2]) == me && d32(&r.parts[3]) == me));
    let uports: Vec<u32> = find(&v, &PORTS_GUID, 0x1f).iter().map(|r| d32(&r.parts[2])).collect();
    assert!(uports.contains(&(ud.local_addr().unwrap().port() as u32)));
    assert!(find(&v, &PORTS_GUID, 0x23).iter().all(|r| d32(&r.parts[2]) == 0));
    assert!(find(&v, &PROCESSES_GUID, 0x1a).is_empty());
    assert!(v.iter().all(|r| r.gate == Gate::State));
    drop(c);
    drop(l);
}

#[test]
fn wfp_guid_names_follow_the_console_table() {
    let _g = lock();
    assert_eq!(guid_text(None), "(nullptr)");
    assert_eq!(guid_text(Some(&[0; 16])), "GUID_NULL");
    let inbound = [0xbf, 0xd1, 0x6f, 0xc8, 0xcd, 0x21, 0x7e, 0x49, 0xa0, 0xbb, 0x17, 0x42, 0x5c, 0x88, 0x5c, 0x58];
    assert_eq!(guid_text(Some(&inbound)), "FWPM_LAYER_INBOUND_IPPACKET_V4");
    let mut plumber = [0u8; 16];
    plumber[..14].copy_from_slice(&guids::FW_PLUMBER_PREFIX);
    plumber[14] = 1;
    plumber[15] = 7;
    assert_eq!(guid_text(Some(&plumber)), "FwPlumberCalloutLogging_layer_7");
    plumber[14] = 9;
    assert_eq!(guid_text(Some(&plumber)), "FwPlumberCallout_type_9_layer_7");
    let g = [0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f, 0x10];
    assert_eq!(guid_text(Some(&g)), "{04030201-0605-0807-090A-0B0C0D0E0F10}");
    assert_eq!(guids::NAMES.len(), 324);
}

#[test]
fn wfp_section_enumerates_its_dynamic_provider_and_empty_object_stores() {
    let _g = lock();
    let before = store::provider_count();
    let (hr, v) = run(|| unsafe { section::log_wfp() });
    assert_eq!(hr, 0);
    assert_eq!(store::provider_count(), before);
    assert!(v.iter().all(|r| r.gate != Gate::Error));
    let s = ids(&v, &WFP_GUID);
    assert_eq!(s.first(), Some(&0x9c));
    assert_eq!(s.last(), Some(&0xa7));
    let fin = find(&v, &WFP_GUID, 0xa7);
    assert_eq!(d32(&fin[0].parts[1]), 0);
    let provider_names: Vec<String> = find(&v, &G_FORMAT, 0xb).iter().map(|r| wtext(&r.parts[1])).collect();
    assert_eq!(provider_names, vec![section::PROVIDER_NAME.to_string()]);
    assert_eq!(wtext(&find(&v, &G_FORMAT, 0x1c)[0].parts[1]), "0x00000000 ()");
    assert_eq!(ids(&v, &G_FORMAT), vec![0x1a, 0x1b, 0xb, 0xe, 0x1c, 0x1d, 0x17, 0x19, 0x1f]);
    assert_eq!(find(&v, &WFP_GUID, 0xe).len(), 1);
    for (end, id) in [(0xf, "providers"), (0x15, "contexts"), (0x1b, "sublayers"), (0x21, "callouts"), (0x7f, "filters"), (0x95, "ipsec"), (0x9b, "ale")] {
        let r = find(&v, &WFP_GUID, end);
        assert_eq!(r.len(), 1, "{}", id);
        assert_eq!(d32(&r[0].parts[1]), 0, "{}", id);
    }
    for (pre, _) in section::FILTER_LAYERS.iter() {
        assert_eq!(find(&v, &WFP_GUID, *pre).len(), 1);
    }
    let counts = find(&v, &WFP_GUID, 0x7e);
    assert_eq!(counts.len(), 17);
    assert!(counts.iter().all(|r| d32(&r.parts[1]) == 0));
    assert!(find(&v, &WFP_GUID, 0x7b).is_empty());
    for id in [0x8a, 0x8d, 0x94, 0x99] {
        assert!(find(&v, &WFP_GUID, id).is_empty());
    }
    assert_eq!(find(&v, &G_SESSION, 0x1b).len(), 1);
    assert_eq!(find(&v, &G_SESSION, 0x19).len(), 1);
    assert_eq!(find(&v, &G_SESSION, 0x2d).len(), 1);
    assert_eq!(find(&v, &G_SESSION, 0x35).len(), 1);
    assert_eq!(find(&v, &G_SESSION, 0x36).len(), 2);
    let created: usize = [0x5c, 0x7f, 0x9a, 0xa3, 0xc5, 0xf9, 0x10b, 0x12a, 0x13e].iter().map(|id| find(&v, &G_SESSION, *id).len()).sum();
    assert_eq!(created, 4 + 17 + 4);
    let destroyed: usize = [0x62, 0x87, 0xa0, 0xa9, 0xcb, 0xff, 0x111, 0x132, 0x144].iter().map(|id| find(&v, &G_SESSION, *id).len()).sum();
    assert_eq!(destroyed, created);
    assert!(find(&v, &G_SESSION, 0x5e).is_empty());
}

#[test]
fn log_state_sections_runs_all_five_sections_without_errors() {
    let _g = lock();
    let (out, v) = run(|| unsafe { log_state_sections() });
    assert_eq!(out, [0; 5], "{:?}", v.iter().filter(|r| r.gate == Gate::Error).map(|r| (r.guid, r.id)).collect::<Vec<_>>());
    for g in [ADAPTERS_GUID, TEREDO_GUID, PROCESSES_GUID, PORTS_GUID, WFP_GUID] {
        assert!(v.iter().any(|r| r.guid == g));
    }
    let me = std::process::id();
    let per_process = [0x1a, 0x1c, 0x14, 0x11, 0x12, 0x17];
    for r in v.iter().filter(|r| r.gate == Gate::Error) {
        assert!(r.guid == PROCESSES_GUID && per_process.contains(&r.id), "unexpected error record {:x}", r.id);
        if matches!(r.id, 0x1a | 0x1c | 0x14) {
            assert_ne!(d32(&r.parts[1]), me, "own process failed in record {:x}", r.id);
        }
    }
}

fn gname(n: &str) -> [u8; 16] {
    guids::NAMES.iter().find(|(_, x)| *x == n).map(|(k, _)| *k).unwrap()
}

fn wv(s: &str) -> Option<Vec<u16>> {
    Some(s.encode_utf16().collect())
}

fn gbytes(g: &GUID) -> [u8; 16] {
    let mut b = [0u8; 16];
    b[0..4].copy_from_slice(&g.d1.to_le_bytes());
    b[4..6].copy_from_slice(&g.d2.to_le_bytes());
    b[6..8].copy_from_slice(&g.d3.to_le_bytes());
    b[8..].copy_from_slice(&g.d4);
    b
}

#[test]
fn wfp_text_helpers_follow_the_console_formatters() {
    use super::wfp_format::*;
    use super::wfp_tables as t;
    use store::Value;
    assert_eq!(flags(0, true, &t::FILTER_FLAGS), "0x00000000 (NONE)");
    assert_eq!(flags(0x21, true, &t::FILTER_FLAGS), "0x00000021 (PERSISTENT DISABLED)");
    assert_eq!(flags(0x8000_0001, true, &t::CONDITION_FLAGS), "0x80000001 (IS_LOOPBACK unrecognized_0x80000000)");
    assert_eq!(flags(0xff00_0000_0000_0001, false, &t::INBOUND_TRANSPORT_FLAGS), "0xff00000000000001 (PASSTHRU RESERVED)");
    assert_eq!(flags(0, true, &t::PROVIDER_FLAGS), "0x00000000 ()");
    assert_eq!(lookup(0xb, &t::MATCH_TYPE), "??? 11/0x0000000b");
    assert_eq!(lookup(8, &t::CLASSIFY_OPTION), "FWPM_GENERAL_CONTEXT");
    assert_eq!(t::IF_TYPE.len(), 199);
    assert_eq!(ipv4(0xc0a8_0001), "192.168.0.1");
    let mut v6 = [0u8; 16];
    v6[0] = 0xfe;
    v6[1] = 0x80;
    v6[15] = 1;
    assert_eq!(ipv6(&v6), "FE80:0000:0000:0000:0000:0000:0000:0001");
    let remote = gname("FWPM_CONDITION_IP_REMOTE_ADDRESS");
    let local = gname("FWPM_CONDITION_IP_LOCAL_ADDRESS");
    let cases: Vec<(Value, Option<[u8; 16]>, &str)> = vec![
        (Value::Empty, None, "FWP_EMPTY"),
        (Value::U32(0x0a00_0001), Some(remote), "FWP_UINT32 IPv4 address 10.0.0.1"),
        (Value::U32(5), None, "FWP_UINT32 uint32 0x00000005/5"),
        (Value::U8(1), Some(gname("FWPM_CONDITION_IP_LOCAL_ADDRESS_TYPE")), "FWP_UINT8 local address type NlatUnicast"),
        (Value::U32(0x8000_0001), Some(gname("FWPM_CONDITION_FLAGS")), "FWP_UINT32 condition flags 0x80000001 (IS_LOOPBACK unrecognized_0x80000000)"),
        (Value::U32(6), Some(gname("FWPM_CONDITION_ARRIVAL_INTERFACE_TYPE")), "FWP_UINT32 arrival interface type IF_TYPE_ETHERNET_CSMACD"),
        (Value::I8(-1), None, "FWP_INT8 int8 0xffffffff/-1"),
        (Value::U64(7), None, "FWP_UINT64 uint64 0x0000000000000007/7"),
        (Value::F32(1.5), None, "FWP_FLOAT float32 1.500000"),
        (Value::Bytes16(v6), Some(local), "FWP_BYTE_ARRAY16_TYPE IPv6 address FE80:0000:0000:0000:0000:0000:0000:0001"),
        (Value::Blob(Vec::new()), None, "FWP_BYTE_BLOB_TYPE byteBlob 0x1111111111111111 { size 0, ptr 0x0000000000000000 }"),
        (Value::Sid(vec![1, 1, 0, 0, 0, 0, 0, 5, 18, 0, 0, 0]), None, "FWP_SID sid \"S-1-5-18\""),
        (Value::Sid(vec![1]), None, "FWP_SID sid 0x1111111111111111"),
        (Value::Unicode(None), None, "FWP_UNICODE_STRING_TYPE unicodeString \"(null)\""),
        (Value::V4AddrMask(0x0a00_0000, 0xff00_0000), None, "FWP_V4_ADDR_MASK v4AddrMask 0x1111111111111111 {addr 10.0.0.0, mask 255.0.0.0)"),
        (Value::Range(Box::new(Value::U16(1)), Box::new(Value::U16(2))), None, "FWP_RANGE_TYPE rangeValue 0x1111111111111111 { valueLow FWP_UINT16 uint16 0x0001/1, valueHigh FWP_UINT16 uint16 0x0002/2}"),
        (Value::Other(0x55), None, "??? type 85 ???"),
    ];
    for (v, k, want) in cases {
        assert_eq!(value_text(&v, k.as_ref(), BUF), want);
    }
    assert_eq!(value_text(&Value::Unicode(wv(&"x".repeat(2000))), None, BUF).encode_utf16().count(), BUF - 1);
    let mut data = b"ABCDEFGHIJKLMNOP".to_vec();
    data.extend_from_slice(&[0, 0x7f]);
    let lines = hex_lines(&data);
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0].0, 0);
    assert_eq!(lines[0].1, "41 42 43 44 45 46 47 48 49 4A 4B 4C 4D 4E 4F 50 ");
    assert_eq!(lines[0].2, "ABCDEFGHIJKLMNOP");
    assert_eq!(lines[1].0, 16);
    assert_eq!(lines[1].1.len(), 48);
    assert!(lines[1].1.starts_with("00 7F "));
    assert_eq!(lines[1].2, "..");
}

#[test]
fn wfp_object_formatters_emit_the_console_record_sequences() {
    use super::wfp_format::*;
    let _g = lock();
    let sl = store::SubLayer { key: [0x72; 16], name: wv("s"), description: None, flags: 1, provider_key: None, data: vec![0x41; 17], weight: 0x10 };
    let ((), v) = run(|| unsafe { format_sublayer(&sl) });
    assert_eq!(ids(&v, &G_FORMAT), vec![0x39, 0x3a, 0xb, 0xe, 0x3b, 0x3c, 0x3d, 0x17, 0x18, 0xa, 0xa, 0x3e]);
    assert_eq!(wtext(&find(&v, &G_FORMAT, 0x3b)[0].parts[1]), "0x00000001 (PERSISTENT)");
    assert_eq!(d64(&find(&v, &G_FORMAT, 0x3c)[0].parts[1]), 0);
    assert_eq!(wtext(&find(&v, &G_FORMAT, 0x3c)[0].parts[2]), "(nullptr)");
    let dump = find(&v, &G_FORMAT, 0xa);
    assert_eq!(wtext(&dump[0].parts[1]), SP4);
    assert_eq!(wtext(&dump[0].parts[2]), SP8);
    assert_eq!(d32(&dump[1].parts[3]), 16);
    assert_eq!(atext(&dump[1].parts[5]), "A");
    assert_eq!(d32(&find(&v, &G_FORMAT, 0x3e)[0].parts[1]), 0x10);
    let co = store::Callout { key: [0x73; 16], name: wv("c"), description: wv("d"), flags: 0x50000, provider_key: None, data: Vec::new(), layer: gname("FWPM_LAYER_ALE_AUTH_CONNECT_V4"), id: 9 };
    let ((), v) = run(|| unsafe { format_callout(&co) });
    assert_eq!(ids(&v, &G_FORMAT), vec![0x3f, 0x40, 0xb, 0xd, 0x41, 0x42, 0x43, 0x17, 0x19, 0x44, 0x45]);
    assert_eq!(wtext(&find(&v, &G_FORMAT, 0x41)[0].parts[2]), "0x00050000 (PERSISTENT REGISTERED)");
    assert_eq!(wtext(&find(&v, &G_FORMAT, 0x44)[0].parts[2]), "FWPM_LAYER_ALE_AUTH_CONNECT_V4");
    assert_eq!(wtext(&find(&v, &G_FORMAT, 0x17)[0].parts[1]), SP8);
    let mut context = [0u8; 16];
    context[..8].copy_from_slice(&3u64.to_le_bytes());
    let fl = store::Filter {
        key: [1; 16],
        name: wv("f"),
        description: None,
        flags: 0,
        provider_key: None,
        data: Vec::new(),
        layer: gname("FWPM_LAYER_INBOUND_TRANSPORT_V4"),
        sublayer: [0; 16],
        weight: store::Value::Empty,
        conditions: vec![
            store::Condition { field: gname("FWPM_CONDITION_IP_REMOTE_ADDRESS"), match_type: 0, value: store::Value::U32(0x0a00_0001) },
            store::Condition { field: [5; 16], match_type: 0xb, value: store::Value::Blob(vec![1, 2]) },
        ],
        action_type: 0x5003,
        action_key: gname("FWPM_CALLOUT_IPSEC_INBOUND_TRANSPORT_V4"),
        context,
        reserved: None,
        id: 42,
        effective_weight: store::Value::U64(1),
    };
    let ((), v) = run(|| unsafe { format_filter("", &fl) });
    assert_eq!(ids(&v, &G_FORMAT), vec![0x46, 0x47, 0xb, 0xe, 0x48, 0x49, 0x4a, 0x17, 0x19, 0x4b, 0x4c, 0x4d, 0x4e, 0x4f, 0x50, 0x51, 0x52, 0x53, 0x50, 0x51, 0x52, 0x53, 0xa, 0x54, 0x55, 0x56, 0x5a, 0x5e, 0x5f, 0x60]);
    assert_eq!(wtext(&find(&v, &G_FORMAT, 0x17)[0].parts[1]), SP4);
    assert_eq!(wtext(&find(&v, &G_FORMAT, 0x4c)[0].parts[2]), "GUID_NULL");
    assert_eq!(wtext(&find(&v, &G_FORMAT, 0x4d)[0].parts[2]), "FWP_EMPTY");
    assert_eq!(wtext(&find(&v, &G_FORMAT, 0x53)[0].parts[2]), "FWP_UINT32 IPv4 address 10.0.0.1");
    assert_eq!(wtext(&find(&v, &G_FORMAT, 0x52)[1].parts[3]), "??? 11/0x0000000b");
    assert_eq!(wtext(&find(&v, &G_FORMAT, 0xa)[0].parts[2]), SP21);
    assert_eq!(wtext(&find(&v, &G_FORMAT, 0x55)[0].parts[3]), "FWP_ACTION_CALLOUT_TERMINATING");
    assert_eq!(wtext(&find(&v, &G_FORMAT, 0x5a)[0].parts[2]), "0x0000000000000003 (PASSTHRU PERSIST_CONNECTION_SECURITY)");
    let id = find(&v, &G_FORMAT, 0x5f);
    assert_eq!((d64(&id[0].parts[2]), d64(&id[0].parts[3])), (42, 42));
    let pc = store::ProviderContext {
        key: [0x74; 16],
        name: wv("x"),
        description: None,
        flags: 0,
        provider_key: None,
        data: Vec::new(),
        kind: store::FWPM_CLASSIFY_OPTIONS_CONTEXT,
        body: store::ContextBody::ClassifyOptions(vec![store::ClassifyOption { kind: 0, value: store::Value::U32(1) }, store::ClassifyOption { kind: 2, value: store::Value::U32(30) }]),
        id: 5,
    };
    let ((), v) = run(|| unsafe { format_provider_context(&pc) });
    assert_eq!(ids(&v, &G_FORMAT), vec![0x20, 0x21, 0xb, 0xe, 0x22, 0x23, 0x24, 0x17, 0x19, 0x25, 0x31, 0x0f, 0x10, 0x11, 0x12, 0x13, 0x11, 0x12, 0x16, 0x38]);
    assert_eq!(wtext(&find(&v, &G_FORMAT, 0x13)[0].parts[3]), "FWP_OPTION_VALUE_DENY_MULTICAST_STATE");
    assert_eq!(wtext(&find(&v, &G_FORMAT, 0x16)[0].parts[2]), "FWP_UINT32 uint32 0x0000001e/30");
    let keying = store::ProviderContext { kind: 0, body: store::ContextBody::Keying { key_mods: vec![[0; 16]], flags: 2 }, ..pc.clone() };
    let ((), v) = run(|| unsafe { format_provider_context(&keying) });
    assert_eq!(&ids(&v, &G_FORMAT)[10..], &[0x26, 0x27, 0x28, 0x29, 0x2a, 0x38]);
    assert_eq!(wtext(&find(&v, &G_FORMAT, 0x29)[0].parts[2]), "GUID_NULL");
}

fn sa_details(dir: u32) -> store::SaDetails {
    let mut local = [0u8; 16];
    local[..4].copy_from_slice(&0xa9fe_4201u32.to_le_bytes());
    store::SaDetails {
        ip_version: 0,
        direction: dir,
        traffic: store::Traffic { ip_version: 0, local, remote: [0; 16], traffic_type: 0, filter_or_policy_id: 7, remote_port: 3074, local_port: 3075, protocol: 17, local_if_luid: 0x6000, real_if_profile_id: 0 },
        bundle: store::SaBundle {
            flags: 0,
            lifetime: [3600, 100000, 0],
            idle_timeout: 300,
            nd_allow_clear_timeout: 0,
            ipsec_id: None,
            nap_context: 0,
            qm_sa_id: 1,
            sas: vec![store::Sa { spi: 0x1234, transform: 4, auth: store::AuthTransform { auth_type: 2, auth_config: 2, crypto_module: None }, auth_key: vec![1; 4], cipher: store::CipherTransform { cipher_type: 3, cipher_config: 3, crypto_module: None }, cipher_key: Vec::new() }],
            key_module: None,
            ip_version: 0,
            peer_v4_private: 0,
            mm_sa_id: 2,
            pfs_group: 0,
            sa_lookup_context: [0; 16],
            qm_filter_id: 3,
        },
        udp_encap: None,
        transport_filter: None,
    }
}

#[test]
fn wfp_ipsec_and_ike_formatters_walk_the_nested_structures() {
    use super::wfp_format::*;
    let _g = lock();
    let ctx = store::SaContext { id: 11, inbound: sa_details(1), outbound: sa_details(0) };
    let ((), v) = run(|| unsafe { format_sa_context(&ctx) });
    let s = ids(&v, &G_FORMAT);
    assert_eq!(&s[..3], &[0x77, 0x78, 0x7a]);
    assert_eq!(find(&v, &G_FORMAT, 0x7a).len(), 2);
    assert_eq!(wtext(&find(&v, &G_FORMAT, 0x7a)[0].parts[1]), SP12);
    assert_eq!(wtext(&find(&v, &G_FORMAT, 0x7b)[0].parts[3]), "FWP_DIRECTION_INBOUND");
    assert_eq!(wtext(&find(&v, &G_FORMAT, 0x7e)[0].parts[2]), "169.254.66.1");
    assert_eq!(wtext(&find(&v, &G_FORMAT, 0x61)[0].parts[1]), format!("{}{}", SP12, SP16));
    let start = s.iter().position(|x| *x == 0x61).unwrap();
    assert_eq!(&s[start..start + 22], &[0x61, 0x62, 0x6c, 0x6d, 0x6e, 0xb5, 0xb6, 0xb7, 0xb8, 0x6f, 0x17, 0x18, 0xa, 0x70, 0x71, 0xb9, 0xba, 0xbb, 0xbc, 0x72, 0x17, 0x19]);
    assert_eq!(wtext(&find(&v, &G_FORMAT, 0xb6)[0].parts[3]), "IPSEC_AUTH_SHA_256");
    assert_eq!(wtext(&find(&v, &G_FORMAT, 0xb8)[0].parts[2]), "(nullptr)");
    assert_eq!(find(&v, &G_FORMAT, 0xb2).len(), 2);
    assert_eq!(find(&v, &G_FORMAT, 0xb4).len(), 2);
    assert_eq!(*s.last().unwrap(), 0xb4);
    let cred = store::Credential { kind: 0, impersonation: 0, body: store::CredentialBody::PresharedKey { key: vec![7], flags: 1 } };
    let ike = store::IkeSa {
        id: 3,
        key_module: 2,
        ip_version: 0,
        udp_encap: Some((4500, 4500)),
        traffic_ip_version: 1,
        local: [0; 16],
        remote: [0; 16],
        auth_ip_filter_id: 0,
        proposal: store::IkeProposal { cipher: 4, key_len: 256, rounds: 0, integrity: 2, max_lifetime: 28800, dh_group: 3, qm_limit: 0 },
        cookie_initiator: 1,
        cookie_responder: 2,
        credentials: vec![(cred.clone(), store::Credential { kind: 2, impersonation: 1, body: store::CredentialBody::Name(wv("n")) })],
        policy_key: [0; 16],
        virtual_if_tunnel_id: 0,
        correlation_key: Vec::new(),
    };
    let ((), v) = run(|| unsafe { format_ike_sa(&ike) });
    assert_eq!(ids(&v, &G_FORMAT), vec![
        0x13f, 0x140, 0x141, 0x142, 0x143, 0x144, 0x146, 0x147, 0x14a, 0x14b, 0x14c, 0x14d, 0x14e, 0xea, 0xeb, 0xec, 0x14f, 0xed, 0x150, 0x151, 0x152, 0x153, 0x154, 0x155, 0x156, 0x157, 0x158,
        0x159, 0x15a, 0xde, 0xdf, 0xe0, 0xe1, 0x17, 0x18, 0xa, 0xe2, 0x15b, 0xde, 0xdf, 0xe8, 0xe9, 0x15c, 0x15d, 0x15e, 0x17, 0x19,
    ]);
    assert_eq!(wtext(&find(&v, &G_FORMAT, 0x140)[0].parts[2]), "IKEEXT_KEY_MODULE_IKEV2");
    assert_eq!(wtext(&find(&v, &G_FORMAT, 0x151)[0].parts[2]), "IKEEXT_DH_GROUP_14/2048");
    assert_eq!(wtext(&find(&v, &G_FORMAT, 0xea)[0].parts[1]), SP12);
    assert_eq!(wtext(&find(&v, &G_FORMAT, 0xe9)[0].parts[2]), "n");
    assert_eq!(wtext(&find(&v, &G_FORMAT, 0x17)[0].parts[1]), format!("{}{}", SP16, SP8));
    let ale = store::AleEndpoint { id: 1, ip_version: 0, local: [1, 0, 0, 0x7f, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0], remote: [0; 16], protocol: 6, local_port: 80, remote_port: 0, token_id: 0, mm_sa: 0, qm_sa: 0, ipsec_status: 0, flags: 0, app_id: Vec::new() };
    let ((), v) = run(|| unsafe { format_ale(&ale) });
    assert_eq!(ids(&v, &G_FORMAT), vec![0x16f, 0x170, 0x171, 0x172, 0x175, 0x176, 0x177, 0x178, 0x179, 0x17a, 0x17b, 0x17c, 0x17d, 0x17, 0x19]);
    assert_eq!(wtext(&find(&v, &G_FORMAT, 0x171)[0].parts[1]), "127.0.0.1");
}

#[test]
fn wfp_section_lists_store_objects_under_their_layer_cases() {
    let _g = lock();
    let e = store::engine_open(store::FWPM_SESSION_FLAG_DYNAMIC).unwrap();
    let transport = gname("FWPM_LAYER_INBOUND_TRANSPORT_V4");
    let sl = store::SubLayer { key: [0x52; 16], name: wv("emu sublayer"), description: None, flags: 0, provider_key: None, data: Vec::new(), weight: 1 };
    assert_eq!(store::sublayer_add(e, sl), 0);
    let flt = store::Filter {
        key: [0x53; 16],
        name: wv("emu filter"),
        description: None,
        flags: 0,
        provider_key: None,
        data: Vec::new(),
        layer: transport,
        sublayer: [0x52; 16],
        weight: store::Value::Empty,
        conditions: Vec::new(),
        action_type: 0x1002,
        action_key: [0; 16],
        context: [0; 16],
        reserved: None,
        id: 0,
        effective_weight: store::Value::Empty,
    };
    let fid = store::filter_add(e, flt).unwrap();
    let (hr, v) = run(|| unsafe { section::log_wfp() });
    assert_eq!(store::engine_close(e), 0);
    assert_eq!(hr, 0);
    assert!(v.iter().all(|r| r.gate != Gate::Error));
    let case = section::FILTER_LAYERS.iter().position(|(_, g)| g.map(|g| gbytes(&g)) == Some(transport)).unwrap();
    let counts: Vec<u32> = find(&v, &WFP_GUID, 0x7e).iter().map(|r| d32(&r.parts[1])).collect();
    assert_eq!(counts.len(), 17);
    for (i, c) in counts.iter().enumerate() {
        assert_eq!(*c, if i == 0 || i == case { 1 } else { 0 }, "case {}", i);
    }
    assert_eq!(find(&v, &WFP_GUID, 0x7b).len(), 2);
    let ids5f: Vec<u64> = find(&v, &G_FORMAT, 0x5f).iter().map(|r| d64(&r.parts[2])).collect();
    assert_eq!(ids5f, vec![fid, fid]);
    let names: Vec<String> = find(&v, &G_FORMAT, 0xb).iter().map(|r| wtext(&r.parts[1])).collect();
    assert!(names.contains(&"emu sublayer".to_string()));
    assert_eq!(names.iter().filter(|n| *n == "emu filter").count(), 2);
    assert_eq!(store::object_count(store::Kind::Filter), 0);
    assert_eq!(store::object_count(store::Kind::SubLayer), 0);
}
