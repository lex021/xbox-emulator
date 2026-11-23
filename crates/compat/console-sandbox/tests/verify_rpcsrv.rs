use console_sandbox::rpcsrv::{self, Client};
use console_sandbox::wstr::wide;
use std::sync::Mutex;

static LOCK: Mutex<()> = Mutex::new(());

fn ifspec(uuid: [u8; 16], table: &[usize]) -> (Box<[u8; 0x60]>, Box<Vec<usize>>) {
    let mut b = Box::new([0u8; 0x60]);
    b[0..4].copy_from_slice(&0x60u32.to_le_bytes());
    b[4..20].copy_from_slice(&uuid);
    b[20..22].copy_from_slice(&2u16.to_le_bytes());
    (b, Box::new(table.to_vec()))
}

extern "system" fn op0() {}
extern "system" fn op1() {}

#[test]
fn endpoints_are_in_process_and_offline_protocols_are_refused() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    rpcsrv::reset();
    unsafe {
        let tcp = wide("ncacn_ip_tcp");
        assert_eq!(rpcsrv::rpc_server_use_protseq_w(tcp.as_ptr(), 10, core::ptr::null_mut()), rpcsrv::RPC_S_PROTSEQ_NOT_SUPPORTED);
        let bad = wide("ncabogus");
        assert_eq!(rpcsrv::rpc_server_use_protseq_w(bad.as_ptr(), 10, core::ptr::null_mut()), rpcsrv::RPC_S_INVALID_RPC_PROTSEQ);
        let lrpc = wide("ncalrpc");
        assert_eq!(rpcsrv::rpc_server_use_protseq_w(lrpc.as_ptr(), 10, core::ptr::null_mut()), 0);
        assert_eq!(rpcsrv::rpc_server_use_protseq_w(lrpc.as_ptr(), 10, core::ptr::null_mut()), 0);
        let ep = wide("NetProfOracle");
        assert_eq!(rpcsrv::rpc_server_use_protseq_ep_w(lrpc.as_ptr(), 4, ep.as_ptr(), core::ptr::null_mut()), 0);
        assert_eq!(rpcsrv::rpc_server_use_protseq_ep_w(lrpc.as_ptr(), 4, ep.as_ptr(), core::ptr::null_mut()), rpcsrv::RPC_S_DUPLICATE_ENDPOINT);
        let hv = wide("ncacn_hvsocket");
        assert_eq!(rpcsrv::rpc_server_use_protseq_w(hv.as_ptr(), 10, core::ptr::null_mut()), rpcsrv::RPC_S_INVALID_ENDPOINT_FORMAT);
        assert_eq!(rpcsrv::endpoints().len(), 2);

        let mut v: *mut u8 = core::ptr::null_mut();
        assert_eq!(rpcsrv::rpc_server_inq_bindings(&mut v), 0);
        assert_eq!((v as *const u32).read(), 2);
        let h = (v.add(8) as *const usize).read();
        let mut s: *mut u16 = core::ptr::null_mut();
        assert_eq!(rpcsrv::rpc_binding_to_string_binding_w(h as *mut _, &mut s), 0);
        let text = console_sandbox::wstr::from_ptr(s);
        assert_eq!(text, format!("ncalrpc:[{}]", console_sandbox::rpc::dynamic_endpoint()));
        assert!(rpcsrv::owns_string(s as usize));
        assert_eq!(rpcsrv::rpc_string_free_w(&mut s), 0);
        assert!(s.is_null());
        assert_eq!(rpcsrv::rpc_binding_vector_free(&mut v), 0);
        assert!(v.is_null());
        assert!(!rpcsrv::is_ours(h));

        let mut name: *mut u16 = core::ptr::null_mut();
        assert_eq!(rpcsrv::rpc_server_inq_default_princ_name_w(10, &mut name), 0);
        assert_eq!(console_sandbox::wstr::from_ptr(name), rpcsrv::DEFAULT_PRINCIPAL);
        assert_eq!(rpcsrv::rpc_string_free_w(&mut name), 0);
        assert_eq!(rpcsrv::rpc_server_inq_default_princ_name_w(77, &mut name), rpcsrv::RPC_S_UNKNOWN_AUTHN_SERVICE);
        assert_eq!(rpcsrv::rpc_server_register_auth_info_w(core::ptr::null(), 77, core::ptr::null(), core::ptr::null_mut()), rpcsrv::RPC_S_UNKNOWN_AUTHN_SERVICE);
    }
    rpcsrv::reset();
}

#[test]
fn calls_carry_attributes_cancel_and_async_completion_and_unregister_waits() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    rpcsrv::reset();
    unsafe {
        let ep = console_sandbox::rpc::dynamic_endpoint();
        assert_eq!(rpcsrv::listen("ncalrpc", 10, None, 0), 0);
        let uuid = [0x33u8; 16];
        let (spec, table) = ifspec(uuid, &[op0 as *const () as usize, op1 as *const () as usize]);
        let sp = spec.as_ptr() as *const core::ffi::c_void;
        assert_eq!(rpcsrv::rpc_server_register_if3(sp, core::ptr::null(), table.as_ptr() as *const _, rpcsrv::RPC_IF_AUTOLISTEN | rpcsrv::RPC_IF_ALLOW_LOCAL_ONLY, 1, 0, core::ptr::null(), core::ptr::null_mut()), 0);
        assert_eq!(rpcsrv::rpc_server_register_if3(sp, core::ptr::null(), table.as_ptr() as *const _, 1, 1, 0, core::ptr::null(), core::ptr::null_mut()), rpcsrv::RPC_S_TYPE_ALREADY_REGISTERED);

        assert_eq!(rpcsrv::rpc_server_test_cancel(core::ptr::null_mut()), rpcsrv::RPC_S_NO_CALL_ACTIVE);
        let mut attrs = [0u8; 0x78];
        attrs[0] = 2;
        assert_eq!(rpcsrv::rpc_server_inq_call_attributes_w(core::ptr::null_mut(), attrs.as_mut_ptr()), rpcsrv::RPC_S_NO_CALL_ACTIVE);

        let client = Client::lrpc(&ep);
        let seen = rpcsrv::invoke(&uuid, 2, 1, &client, |d| {
            let mut a = [0u8; 0x78];
            a[0] = 2;
            a[4] = (rpcsrv::RPC_QUERY_CLIENT_PID | rpcsrv::RPC_QUERY_IS_CLIENT_LOCAL) as u8;
            let r = rpcsrv::rpc_server_inq_call_attributes_w(core::ptr::null_mut(), a.as_mut_ptr());
            let before = rpcsrv::rpc_server_test_cancel(core::ptr::null_mut());
            rpcsrv::cancel(d.call);
            let after = rpcsrv::rpc_server_test_cancel(d.call as *mut _);
            let busy = rpcsrv::invoke(&uuid, 2, 0, &client, |_| ()).err();
            (d.routine, r, a, before, after, busy)
        })
        .unwrap();
        assert_eq!(seen.0, op1 as *const () as usize);
        assert_eq!(seen.1, 0);
        let rd = |o: usize| u32::from_le_bytes(seen.2[o..o + 4].try_into().unwrap());
        assert_eq!((rd(0x38), rd(0x3c), rd(0x40), rd(0x48), rd(0x4c)), (rpcsrv::RPC_PROTSEQ_LRPC, rpcsrv::RCCL_LOCAL, std::process::id(), rpcsrv::RPC_CALL_STATUS_IN_PROGRESS, rpcsrv::RCT_NORMAL));
        assert_eq!(u16::from_le_bytes([seen.2[0x58], seen.2[0x59]]), 1);
        assert_eq!(&seen.2[0x5c..0x6c], &uuid);
        assert_eq!((seen.3, seen.4, seen.5), (rpcsrv::RPC_S_CALL_IN_PROGRESS, 0, Some(rpcsrv::RPC_S_SERVER_TOO_BUSY)));
        assert_eq!(rpcsrv::invoke(&uuid, 2, 0, &Client::hvsocket(&ep, [1; 16]), |_| ()).err(), Some(rpcsrv::RPC_S_INVALID_BINDING));

        static HITS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        unsafe extern "system" fn on_disconnect(_a: usize, _c: usize, ev: u32) {
            if ev == rpcsrv::RPC_ASYNC_EVENT_CLIENT_DISCONNECT {
                HITS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
        }
        let call = rpcsrv::invoke_async(&uuid, 2, 0, &client, |d, state| {
            assert_eq!((state as *const u32).read(), rpcsrv::RPC_ASYNC_STATE_SIZE);
            assert_eq!(((state + 0x20) as *const usize).read(), d.call);
            let info = [on_disconnect as *const () as usize, 0, 0, 0];
            assert_eq!(rpcsrv::rpc_server_subscribe_for_notification(d.call as *mut _, rpcsrv::RPC_NOTIFICATION_CLIENT_DISCONNECT, rpcsrv::RPC_NOTIFICATION_TYPE_CALLBACK, info.as_ptr()), 0);
        })
        .unwrap();
        assert!(call.notify(rpcsrv::RPC_NOTIFICATION_CLIENT_DISCONNECT));
        assert_eq!(HITS.load(std::sync::atomic::Ordering::SeqCst), 1);
        let mut queued = 0u32;
        assert_eq!(rpcsrv::rpc_server_unsubscribe_for_notification(call.dispatch.call as *mut _, rpcsrv::RPC_NOTIFICATION_CLIENT_DISCONNECT, &mut queued), 0);
        assert_eq!(queued, 1);
        assert_eq!(rpcsrv::active_calls(&uuid), 1);
        let spu = sp as usize;
        let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let d2 = done.clone();
        let unreg = std::thread::spawn(move || {
            let r = rpcsrv::rpc_server_unregister_if(spu as *const core::ffi::c_void, core::ptr::null(), 1);
            d2.store(true, std::sync::atomic::Ordering::SeqCst);
            r
        });
        std::thread::sleep(std::time::Duration::from_millis(50));
        assert!(!done.load(std::sync::atomic::Ordering::SeqCst));
        let reply = 0x1234u32;
        assert_eq!(rpcsrv::rpc_async_complete_call(call.state as *mut _, &reply as *const u32 as *mut _), 0);
        assert_eq!(rpcsrv::rpc_async_complete_call(call.state as *mut _, &reply as *const u32 as *mut _), rpcsrv::RPC_S_INVALID_ASYNC_HANDLE);
        assert_eq!(call.wait(std::time::Duration::from_secs(5)), Some(Ok(0x1234)));
        assert_eq!(unreg.join().unwrap(), 0);
        assert_eq!(rpcsrv::active_calls(&uuid), 0);
        assert_eq!(rpcsrv::rpc_server_unregister_if(sp, core::ptr::null(), 1), rpcsrv::RPC_S_UNKNOWN_IF);
        drop(table);
    }
    rpcsrv::reset();
}
