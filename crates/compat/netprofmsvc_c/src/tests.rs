use super::aggregate::{self, flush_period_ms, Aggregate, LIST};
use super::failure::{self, failure_fields, failure_metadata, CallContextInfo};
use super::tlg::{provider_enable_callback, EnableCallback};
use super::*;
use std::sync::Mutex;

static LOCK: Mutex<()> = Mutex::new(());

const CONSOLE: &str = r"D:\xbox-pc-port\games\Fortnite\extracted\EmbeddedXvd\Windows\System32\netprofmsvc_c.dll";

#[link(name = "kernel32")]
extern "system" {
    fn LoadLibraryW(name: *const u16) -> *mut c_void;
    fn GetCurrentProcessId() -> u32;
}

#[link(name = "advapi32")]
extern "system" {
    fn EnumerateTraceGuidsEx(class: u32, inb: *const c_void, insz: u32, out: *mut c_void, outsz: u32, ret: *mut u32) -> u32;
}

fn console() -> Option<usize> {
    if !std::path::Path::new(CONSOLE).is_file() {
        eprintln!("{CONSOLE} absent, oracle skipped");
        return None;
    }
    let w: Vec<u16> = CONSOLE.encode_utf16().chain(Some(0)).collect();
    let h = unsafe { LoadLibraryW(w.as_ptr()) };
    assert!(!h.is_null(), "console netprofmsvc_c.dll must load on the PC for the oracle");
    Some(h as usize)
}

unsafe fn rd<T: Copy>(a: usize) -> T {
    core::ptr::read_unaligned(a as *const T)
}

fn instances_in_this_process(guid: &Guid) -> u32 {
    let mut buf = vec![0u8; 0x10000];
    let mut ret = 0u32;
    let st = unsafe { EnumerateTraceGuidsEx(1, guid as *const Guid as *const c_void, 16, buf.as_mut_ptr() as *mut c_void, buf.len() as u32, &mut ret) };
    if st == 4200 {
        return 0;
    }
    assert_eq!(st, 0, "EnumerateTraceGuidsEx");
    let n = u32::from_le_bytes(buf[0..4].try_into().unwrap());
    let pid = unsafe { GetCurrentProcessId() };
    let mut off = 8usize;
    let mut mine = 0;
    for _ in 0..n {
        let next = u32::from_le_bytes(buf[off..off + 4].try_into().unwrap()) as usize;
        let p = u32::from_le_bytes(buf[off + 8..off + 12].try_into().unwrap());
        if p == pid {
            mine += 1;
        }
        if next == 0 {
            break;
        }
        off += next;
    }
    mine
}

#[test]
fn failure_info_layout_is_the_19041_wil_layout() {
    assert_eq!(core::mem::size_of::<FailureInfo>(), 0x90);
    let f: FailureInfo = unsafe { core::mem::zeroed() };
    let b = &f as *const FailureInfo as usize;
    let off = |p: usize| p - b;
    assert_eq!(off(&f.hr as *const _ as usize), 4);
    assert_eq!(off(&f.failure_id as *const _ as usize), 8);
    assert_eq!(off(&f.message as *const _ as usize), 0x10);
    assert_eq!(off(&f.thread_id as *const _ as usize), 0x18);
    assert_eq!(off(&f.function as *const _ as usize), 0x28);
    assert_eq!(off(&f.file as *const _ as usize), 0x30);
    assert_eq!(off(&f.line as *const _ as usize), 0x38);
    assert_eq!(off(&f.failure_count as *const _ as usize), 0x3c);
    assert_eq!(off(&f.call_context as *const _ as usize), 0x40);
    assert_eq!(off(&f.originating.name as *const _ as usize), 0x50);
    assert_eq!(off(&f.current.context_id as *const _ as usize), 0x60);
    assert_eq!(off(&f.current.message as *const _ as usize), 0x70);
    assert_eq!(off(&f.module as *const _ as usize), 0x78);
}

#[test]
fn provider_traits_and_failure_metadata_match_console_bytes() {
    assert_eq!(PROVIDER.traits().len(), 0x3d);
    assert_eq!(AGGREGATE_PROVIDER.traits().len(), 0x3c);
    assert_eq!(failure_metadata().len(), 0x108);
    let Some(base) = console() else { return };
    unsafe {
        for (p, rva) in [(&PROVIDER, 0x6c000usize), (&AGGREGATE_PROVIDER, 0x6caf0)] {
            let meta: usize = rd(base + rva + 8);
            let n = rd::<u16>(meta) as usize;
            assert_eq!(core::slice::from_raw_parts(meta as *const u8, n), p.traits(), "{}", p.name);
            assert_eq!(core::slice::from_raw_parts((meta - 16) as *const u8, 16), &p.guid.bytes()[..]);
        }
        let ev = base + 0x61001;
        assert_eq!(core::slice::from_raw_parts(ev as *const u8, 3), &[0x0b, 0x02, 0x00]);
        assert_eq!(rd::<u64>(ev + 3), 0);
        assert_eq!(core::slice::from_raw_parts((ev + 11) as *const u8, 0x108), failure_metadata());
    }
}

#[test]
fn flush_period_hash_matches_console_0x47410() {
    assert!((600_000..1_200_000).contains(&flush_period_ms(0, &NETWORK_PROFILE_GUID)));
    let Some(base) = console() else { return };
    let f: extern "system" fn(usize) -> u32 = unsafe { core::mem::transmute(base + 0x47410) };
    let mut obj = vec![0u8; 0x400];
    let start = obj.as_mut_ptr() as usize;
    for k in 0..48usize {
        let a = (start + 0x40 + k * 8) & !7;
        unsafe { *((a + 0x148) as *mut usize) = base + 0x6c000 };
        assert_eq!(flush_period_ms(a, &NETWORK_PROFILE_GUID), f(a), "object {a:#x}");
    }
    unsafe { *((start + 0x148) as *mut usize) = base + 0x6caf0 };
    assert_eq!(flush_period_ms(start, &AGGREGATE_GUID), f(start));
}

#[test]
fn enable_callback_matches_console_0x1970() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let Some(base) = console() else { return };
    let theirs: EnableCallback = unsafe { core::mem::transmute(base + 0x1970) };
    static P: Provider = Provider::new(NETWORK_PROFILE_GUID, "Test");
    for (en, lvl, any, all) in [(1u32, 0u8, 0x10u64, 0x2u64), (1, 4, 0x8000_0000_0000, 0), (2, 5, 1, 1), (0, 3, 7, 7), (1, 255, u64::MAX, 3)] {
        let mut raw = [0u64; 8];
        raw[0] = 0x55;
        raw[2] = 0x66;
        raw[3] = 0x77;
        P.level_plus1.store(0x55, Ordering::SeqCst);
        P.any.store(0x66, Ordering::SeqCst);
        P.all.store(0x77, Ordering::SeqCst);
        unsafe {
            theirs(&NETWORK_PROFILE_GUID, en, lvl, any, all, core::ptr::null(), raw.as_mut_ptr() as *mut c_void);
            provider_enable_callback(&NETWORK_PROFILE_GUID, en, lvl, any, all, core::ptr::null(), &P as *const Provider as *mut c_void);
        }
        assert_eq!(P.level_plus1.load(Ordering::SeqCst) as u64, raw[0] & 0xffff_ffff, "level {en} {lvl}");
        assert_eq!(P.any.load(Ordering::SeqCst), raw[2]);
        assert_eq!(P.all.load(Ordering::SeqCst), raw[3]);
    }
}

#[test]
fn dll_main_attach_detach_builds_and_tears_down_the_console_state() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let base = console();
    if let Some(b) = base {
        unsafe {
            assert_ne!(rd::<u64>(b + 0x6c020), 0, "console provider handle");
            assert_eq!(rd::<usize>(b + 0x6c028), b + 0x481c0, "console provider callback is the aggregate one");
            let node: usize = rd(b + 0x6e8b0);
            assert_ne!(node, 0);
            assert_eq!(rd::<usize>(b + 0x6c030), node);
            assert_eq!(rd::<usize>(node + 0x148), b + 0x6c000);
            assert_eq!(rd::<usize>(node + 0x150), 0);
            assert_ne!(rd::<usize>(node + 0x158), 0, "console timer created");
            assert_eq!(rd::<u32>(node + 0x160), flush_period_ms(node, &NETWORK_PROFILE_GUID));
            assert_eq!(rd::<u32>(node + 0x100), 0);
            assert_ne!(rd::<u64>(b + 0x6cb10), 0, "console aggregate provider handle");
            assert_eq!(rd::<usize>(b + 0x6cb18), b + 0x48150);
            assert_eq!(rd::<usize>(b + 0x6e608), b + 0x5910, "console wil logging callback");
        }
    }
    let before = instances_in_this_process(&NETWORK_PROFILE_GUID);
    let before_agg = instances_in_this_process(&AGGREGATE_GUID);
    assert_eq!(DllMain(core::ptr::null_mut(), 1, core::ptr::null_mut()), 1);
    assert_ne!(PROVIDER.handle.load(Ordering::SeqCst), 0);
    assert_ne!(AGGREGATE_PROVIDER.handle.load(Ordering::SeqCst), 0);
    assert!(PROVIDER.callback_is(aggregate::aggregate_enable_callback));
    assert!(AGGREGATE_PROVIDER.callback_is(aggregate::internal_enable_callback));
    assert_eq!(RESULT_LOGGING_CALLBACK.load(Ordering::SeqCst), result_logging_callback_addr());
    let node = {
        let l = LIST.lock().unwrap();
        assert_eq!(l.0.len(), 1);
        l.0[0]
    };
    assert_eq!(PROVIDER.context.load(Ordering::SeqCst), node as *mut c_void);
    unsafe {
        assert!(core::ptr::eq((*node).provider, &PROVIDER));
        assert!(!(*node).timer.load(Ordering::SeqCst).is_null());
        assert_eq!((*node).period_ms.load(Ordering::SeqCst), flush_period_ms(node as usize, &NETWORK_PROFILE_GUID));
    }
    assert_eq!(instances_in_this_process(&NETWORK_PROFILE_GUID), before + 1);
    assert_eq!(instances_in_this_process(&AGGREGATE_GUID), before_agg + 1);
    unsafe {
        aggregate::aggregate_enable_callback(&NETWORK_PROFILE_GUID, 1, 4, 0, 0, core::ptr::null(), node as *mut c_void);
        aggregate::aggregate_enable_callback(&NETWORK_PROFILE_GUID, 2, 4, 0, 0, core::ptr::null(), node as *mut c_void);
        aggregate::internal_enable_callback(&AGGREGATE_GUID, 2, 0, 0x20, 0, core::ptr::null(), core::ptr::null_mut());
    }
    assert_eq!(DllMain(core::ptr::null_mut(), 2, core::ptr::null_mut()), 1);
    assert_eq!(DllMain(core::ptr::null_mut(), 3, core::ptr::null_mut()), 1);
    assert_eq!(LIST.lock().unwrap().0.len(), 1);
    assert_eq!(DllMain(core::ptr::null_mut(), 0, core::ptr::null_mut()), 1);
    assert_eq!(PROVIDER.handle.load(Ordering::SeqCst), 0);
    assert_eq!(AGGREGATE_PROVIDER.handle.load(Ordering::SeqCst), 0);
    assert_eq!(RESULT_LOGGING_CALLBACK.load(Ordering::SeqCst), 0);
    assert!(LIST.lock().unwrap().0.is_empty());
    assert_eq!(instances_in_this_process(&NETWORK_PROFILE_GUID), before);
    assert_eq!(instances_in_this_process(&AGGREGATE_GUID), before_agg);
    assert_eq!(DllMain(core::ptr::null_mut(), 1, core::ptr::null_mut()), 1);
    assert_eq!(LIST.lock().unwrap().0.len(), 1);
    assert_eq!(DllMain(core::ptr::null_mut(), 0, core::ptr::null_mut()), 1);
    assert!(LIST.lock().unwrap().0.is_empty());
}

#[test]
fn result_logging_callback_writes_the_failure_event_only_when_enabled() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let file = b"netprofmsvc.cpp\0";
    let func = b"Fn\0";
    let msg: Vec<u16> = "oops".encode_utf16().chain(Some(0)).collect();
    let f = FailureInfo {
        failure_type: 1,
        hr: 0x8007_0005u32 as i32,
        failure_id: 9,
        message: msg.as_ptr(),
        thread_id: 42,
        code: core::ptr::null(),
        function: func.as_ptr(),
        file: file.as_ptr(),
        line: 77,
        failure_count: 3,
        call_context: core::ptr::null(),
        originating: CallContextInfo { context_id: 5, name: core::ptr::null(), message: core::ptr::null() },
        current: CallContextInfo { context_id: 6, name: core::ptr::null(), message: core::ptr::null() },
        module: core::ptr::null(),
        return_address: core::ptr::null_mut(),
        caller_return_address: core::ptr::null_mut(),
    };
    let d = unsafe { failure_fields(&f) };
    let sizes: Vec<u32> = d.iter().map(|e| e.size).collect();
    assert_eq!(sizes, vec![4, 16, 4, 1, 4, 10, 4, 1, 4, 1, 2, 4, 1, 2, 4, 4, 3]);
    unsafe {
        assert_eq!(*(d[0].ptr as *const i32), f.hr);
        assert_eq!(d[1].ptr, file.as_ptr() as u64);
        assert_eq!(*(d[2].ptr as *const u32), 77);
        assert_eq!(*(d[8].ptr as *const i32), 5);
        assert_eq!(*(d[11].ptr as *const i32), 6);
        assert_eq!(*(d[14].ptr as *const i32), 9);
        assert_eq!(*(d[15].ptr as *const i32), 3);
    }
    assert_eq!(DllMain(core::ptr::null_mut(), 1, core::ptr::null_mut()), 1);
    PROVIDER.level_plus1.store(0, Ordering::SeqCst);
    assert_eq!(unsafe { failure::write_failure(&f) }, None);
    PROVIDER.level_plus1.store(3, Ordering::SeqCst);
    assert_eq!(unsafe { failure::write_failure(&f) }, Some(0));
    PROVIDER.level_plus1.store(2, Ordering::SeqCst);
    assert_eq!(unsafe { failure::write_failure(&f) }, None);
    PROVIDER.level_plus1.store(0x100, Ordering::SeqCst);
    unsafe { report_failure(&f) };
    assert_eq!(DllMain(core::ptr::null_mut(), 0, core::ptr::null_mut()), 1);
    assert_eq!(PROVIDER.level_plus1.load(Ordering::SeqCst), 0);
}

#[test]
fn push_stores_and_replaces_the_globals_pointer() {
    let mut a = [0u8; 0xe0];
    let mut b = [0u8; 0xe0];
    SvchostPushServiceGlobalsEx(a.as_mut_ptr() as *mut c_void);
    assert_eq!(svchost_globals(), a.as_mut_ptr() as *mut c_void);
    SvchostPushServiceGlobalsEx(b.as_mut_ptr() as *mut c_void);
    assert_eq!(svchost_globals(), b.as_mut_ptr() as *mut c_void);
    SvchostPushServiceGlobalsEx(core::ptr::null_mut());
    assert!(svchost_globals().is_null());
    assert_eq!(a[0], 0);
}

#[test]
fn aggregate_create_destroy_closes_its_timer() {
    let a = Aggregate::create();
    assert!(!a.is_null());
    unsafe {
        assert!(!(*a).timer.load(Ordering::SeqCst).is_null());
        (*a).stop_timer();
        assert!((*a).timer.load(Ordering::SeqCst).is_null());
        aggregate::destroy(a);
    }
}

#[test]
fn physical_interface_probe_flow_finds_nothing_offline_and_the_nic_on_link() {
    use compat_common::netview::{self, iphlp as model};
    let _g = sandbox();
    let [p4, p6] = probes::probe_addresses().unwrap();
    let root4 = probes::as_sort_entry(&p4);
    let root6 = probes::as_sort_entry(&p6);
    assert_eq!(&root4[20..24], &[192, 228, 79, 201]);
    unsafe {
        let sorted = iphlp::sorted_probes(&[root6, root4]).unwrap();
        assert_eq!(sorted, vec![root6, root4]);
        assert_eq!(iphlp::physical_interface_for_probes(&[root4, root6]), None);
        let link = probes::as_sort_entry(&probes::probe_from_text("169.254.1.2").unwrap());
        let nic = netview::adapters().into_iter().find(|a| a.if_index == netview::NIC_IF_INDEX).unwrap();
        assert_eq!(iphlp::physical_interface_for_probes(&[root4, link]), Some(model::interface_guid(&nic)));
        let mut luid = 0u64;
        assert_eq!(iphlp::convert_interface_guid_to_luid(&model::interface_guid(&nic), &mut luid), 0);
        let mut idx = 0u32;
        assert_eq!(iphlp::convert_interface_luid_to_index(&luid, &mut idx), 0);
        assert_eq!(idx, netview::NIC_IF_INDEX);
        let mut t: *mut u8 = core::ptr::null_mut();
        assert_eq!(iphlp::get_if_table2(&mut t), 0);
        assert_eq!(core::ptr::read_unaligned(t as *const u32), 2);
        iphlp::free_mib_table(t as *mut c_void);
    }
}

#[test]
fn interface_settings_write_both_ip_modules_like_the_event_manager() {
    use compat_common::netview::{self, iphlp as model};
    model::nsi_reset();
    unsafe {
        assert_eq!(iphlp::apply_interface_settings(netview::NIC_LUID, 1, 1, [3u8; 16]), [0, 0]);
        assert_eq!(iphlp::apply_interface_settings(0x99, 1, 1, [3u8; 16]), [netview::ERROR_NOT_FOUND, netview::ERROR_NOT_FOUND]);
    }
    for m in [model::NPI_MS_IPV4_MODULEID, model::NPI_MS_IPV6_MODULEID] {
        let rw = model::nsi_interface_rw(&m, netview::NIC_LUID).unwrap();
        assert_eq!(u32::from_le_bytes(rw[0x10..0x14].try_into().unwrap()), 1);
        assert_eq!(&rw[0xc0..0xd0], &[3u8; 16]);
        assert_eq!(&rw[0x14..0x18], &[0xff; 4]);
    }
    model::nsi_reset();
}

fn sandbox() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    static INIT: std::sync::Once = std::sync::Once::new();
    INIT.call_once(|| {
        let d = std::env::temp_dir().join(format!("tombox-netprof-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::env::set_var("TOMBOX_GDK_ROOT", &d);
    });
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

#[test]
fn probe_values_live_in_the_console_network_list_key() {
    let mut seen = 0;
    for p in ["Fortnite", "XDefiant"] {
        let path = std::path::PathBuf::from(format!(r"D:\xbox-pc-port\games\{p}\extracted\EmbeddedXvd\Windows\System32\config\SOFTWARE"));
        let Some(h) = console_sandbox::hive::Hive::open(&path) else { continue };
        let k = h.key(h.find(probes::NETWORK_LIST_SUBTREE).unwrap()).unwrap();
        let text = |name: &str| {
            let v = k.values.iter().find(|v| v.name.eq_ignore_ascii_case(name)).unwrap();
            assert_eq!(v.kind, 1);
            let w: Vec<u16> = v.data.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|c| *c != 0).collect();
            String::from_utf16_lossy(&w)
        };
        assert_eq!(text(""), "192.228.79.201", "{p}");
        assert_eq!(text(probes::ROOT_DNS_IPV6_VALUE), "2001:478:65::53", "{p}");
        seen += 1;
    }
    assert!(seen > 0);
}

#[test]
fn probe_reader_matches_the_console_sockaddr_layout() {
    let _g = sandbox();
    let [v4, v6] = probes::probe_addresses().unwrap();
    assert_eq!(u16::from_le_bytes([v4[0], v4[1]]), 2);
    assert_eq!(&v4[2..4], &[0, 0]);
    assert_eq!(&v4[4..8], &[192, 228, 79, 201]);
    assert!(v4[8..].iter().all(|b| *b == 0));
    assert_eq!(u16::from_le_bytes([v6[0], v6[1]]), 0x17);
    assert_eq!(&v6[8..24], &"2001:478:65::53".parse::<std::net::Ipv6Addr>().unwrap().octets());
    assert!(v6[0x1c..].iter().all(|b| *b == 0));
    let p = probes::probe_from_text("[fe80::1%7]:53").unwrap();
    assert_eq!((&p[2..4], u32::from_le_bytes(p[0x18..0x1c].try_into().unwrap())), (&53u16.to_be_bytes()[..], 7));
    assert_eq!(&probes::probe_from_text("10.1.2.3:80").unwrap()[2..8], &[0, 80, 10, 1, 2, 3]);
    assert_eq!(probes::probe_from_text("1.2.3"), Err(probes::HR_NT_INVALID_PARAMETER));
    assert_eq!(probes::read_probe("NoSuchProbeValue"), Err(probes::HR_FILE_NOT_FOUND));
    let e = probes::as_sort_entry(&v4);
    assert_eq!(u16::from_le_bytes([e[0], e[1]]), 0x17);
    assert_eq!(&e[18..24], &[0xff, 0xff, 192, 228, 79, 201]);
    assert_eq!(probes::as_sort_entry(&v6)[..], v6[..28]);
}

#[test]
fn paired_web_probe_reports_no_internet_offline_without_sockets() {
    let req = webprobe::Request { host: "www.msftconnecttest.com".into(), port: 80, path: "connecttest.txt".into() };
    let o = webprobe::run_offline(&req, 3);
    assert_eq!(o.events[0], webprobe::Event::RequestError(webprobe::AsyncResult { api: webprobe::API_SEND_REQUEST, error: webprobe::ERROR_WINHTTP_NAME_NOT_RESOLVED }));
    assert_eq!(o.events.iter().filter(|e| **e == webprobe::Event::HandleClosing).count(), 3);
    assert_eq!(o.hr, 0x8007_2ee7u32 as i32);
    assert!(o.body.is_empty());
    assert!(!webprobe::has_internet(&o));
    let lo = webprobe::run_offline(&webprobe::Request { host: "127.0.0.1".into(), port: 80, path: "/".into() }, 0);
    assert_eq!(lo.hr, webprobe::hresult_from_win32(12029));
    assert!(!webprobe::has_internet(&lo));
}

#[link(name = "netapi32")]
extern "system" {
    fn NetApiBufferFree(p: *mut c_void) -> u32;
}

#[test]
fn join_information_is_an_unjoined_workgroup() {
    unsafe {
        let mut name: *mut u16 = core::ptr::null_mut();
        let mut status = 0u32;
        assert_eq!(join::net_get_join_information(core::ptr::null(), &mut name, &mut status), join::NERR_SUCCESS);
        assert_eq!(status, join::NET_SETUP_WORKGROUP_NAME);
        assert!(!join::reads_domain_policy(status));
        let s: Vec<u16> = (0..).map(|i| *name.add(i)).take_while(|c| *c != 0).collect();
        assert_eq!(String::from_utf16_lossy(&s), join::WORKGROUP);
        assert_eq!(NetApiBufferFree(name as *mut c_void), 0);
        let srv: Vec<u16> = "OTHER\0".encode_utf16().collect();
        assert_eq!(join::net_get_join_information(srv.as_ptr(), &mut name, &mut status), join::ERROR_NOT_SUPPORTED);
        assert_eq!(join::net_get_join_information(core::ptr::null(), core::ptr::null_mut(), &mut status), join::ERROR_INVALID_PARAMETER);
    }
}

#[path = "tests_service.rs"]
mod service;
