use crate::com::*;
use crate::imports;
use crate::messaging::*;
use crate::sys::*;
use core::ffi::c_void;
use core::ptr::null_mut;
use std::sync::Mutex;

pub const CONSOLE_PIPE_NAME: &str = "\\\\.\\SEVPIPE\\XTF_CONSOLE_EXTENSION_API_SYSTEM_TO_TITLE_OPERATION_REQUEST";
pub const PIPE_NAME_PREFIX: &str = "\\\\.\\pipe\\XTF_CONSOLE_EXTENSION_API_SYSTEM_TO_TITLE_OPERATION_REQUEST_";
pub const REQUEST_BUFFER_BYTES: u32 = 0x2000;
pub const OPERATION_REQUEST_VERSION: u32 = 1;
pub const PIPE_ACCESS_DUPLEX: u32 = 3;
pub const PIPE_MESSAGE_MODE: u32 = 6;
pub const PIPE_REJECT_REMOTE_CLIENTS: u32 = 8;
pub const THREAD_MODE_BACKGROUND_BEGIN: i32 = 0x10000;

pub trait OperationService: Sync {
    unsafe fn route(&self, op: u32, body: Obj, response: *mut Obj) -> HRESULT;
    unsafe fn check_pending(&self) -> HRESULT;
    unsafe fn thread_start(&self) {}
}

pub fn pipe_name(pid: u32) -> Vec<u16> {
    wz(&format!("{}{}", PIPE_NAME_PREFIX, pid))
}

pub unsafe fn create_operation_response(result: HRESULT, body: Obj, out: *mut BSTR) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    *out = null_mut();
    let i = match imports::get() {
        Ok(i) => i,
        Err(e) => return e,
    };
    let mut o = Ptr::null();
    let hr = (i.create_json_object)(o.out());
    if hr < 0 {
        return hr;
    }
    let hr = set_u32(o.0, &wz("version"), OPERATION_REQUEST_VERSION);
    if hr < 0 {
        return hr;
    }
    let hr = set_i32(o.0, &wz("opresult"), result);
    if hr < 0 {
        return hr;
    }
    if !body.is_null() {
        let hr = obj_set_object(o.0, wz("opbody").as_ptr(), body);
        if hr < 0 {
            return hr;
        }
    }
    obj_stringify(o.0, out)
}

pub unsafe fn process_operation_request(service: &dyn OperationService, request: *const u16, out: *mut BSTR) {
    let mut message = Ptr::null();
    let mut op_body = Ptr::null();
    let mut op_response = Ptr::null();
    let mut hr;
    'route: {
        if request.is_null() || out.is_null() {
            hr = E_POINTER;
            break 'route;
        }
        *out = null_mut();
        hr = match imports::get() {
            Ok(i) => (i.get_json_object)(request, message.out()),
            Err(e) => e,
        };
        if hr < 0 {
            imports::log_error(b"Failed getting operation request message json object\0", &[]);
            break 'route;
        }
        let mut version = 0u32;
        hr = get_u32(message.0, &wz("version"), &mut version);
        if hr < 0 {
            imports::log_error(b"Failed getting operation request message version\0", &[]);
            break 'route;
        }
        if version != OPERATION_REQUEST_VERSION {
            imports::log_error(b"Failed: operation request version mismatch\0", &[]);
            hr = HR_NOT_SUPPORTED;
            break 'route;
        }
        let mut op = 0u32;
        hr = get_u32(message.0, &wz("opid"), &mut op);
        if hr < 0 {
            imports::log_error(b"Failed getting operation request message opId\0", &[]);
            break 'route;
        }
        hr = obj_get_object(message.0, wz("opbody").as_ptr(), op_body.out());
        if hr < 0 && hr != E_JSON_NOT_FOUND {
            imports::log_error(b"Failed getting operation request message opBody\0", &[]);
            break 'route;
        }
        hr = service.route(op, op_body.0, op_response.out());
    }
    let mut response = Bstr::null();
    if create_operation_response(hr, op_response.0, response.out()) >= 0 {
        if !out.is_null() {
            *out = response.0;
            response.0 = null_mut();
        }
    } else {
        imports::log_error(b"Failed CreateOperationResponse\0", &[]);
    }
}

#[repr(C)]
pub struct OperationRequestMonitor {
    pub pipe: HANDLE,
    pub thread: HANDLE,
    pub pipe_name: Vec<u16>,
    pub service: &'static dyn OperationService,
}

unsafe impl Send for OperationRequestMonitor {}
unsafe impl Sync for OperationRequestMonitor {}

struct MonitorSlot(*mut OperationRequestMonitor);

unsafe impl Send for MonitorSlot {}

static MONITOR: Mutex<MonitorSlot> = Mutex::new(MonitorSlot(null_mut()));

impl OperationRequestMonitor {
    pub unsafe fn create_pipe(&mut self) {
        self.pipe = CreateNamedPipeW(self.pipe_name.as_ptr(), PIPE_ACCESS_DUPLEX, PIPE_MESSAGE_MODE | PIPE_REJECT_REMOTE_CLIENTS, 1, 4, 4, 0, null_mut());
        if self.pipe == INVALID_HANDLE_VALUE {
            imports::log_error(b"Failed %d to create pipe\n\0", &[GetLastError() as u64]);
        }
    }

    pub unsafe fn serve_one(&mut self) {
        let mut buf = vec![0u16; (REQUEST_BUFFER_BYTES / 2) as usize];
        let mut read = 0u32;
        if ReadFile(self.pipe, buf.as_mut_ptr() as *mut c_void, REQUEST_BUFFER_BYTES, &mut read, null_mut()) == 0 {
            imports::log_error(b"Failed reading operation request message %d\0", &[GetLastError() as u64]);
            return;
        }
        let last = buf.len() - 1;
        buf[last] = 0;
        let mut response = Bstr::null();
        process_operation_request(self.service, buf.as_ptr(), response.out());
        let chars = if response.0.is_null() { 0 } else { SysStringLen(response.0) + 1 };
        let bytes = chars * 2;
        let mut written = 0u32;
        if WriteFile(self.pipe, response.0 as *const c_void, bytes, &mut written, null_mut()) == 0 || written != bytes {
            imports::log_error(b"Failed to write operation response\0", &[]);
        }
        FlushFileBuffers(self.pipe);
    }

    pub unsafe fn run(&mut self) -> u32 {
        self.service.thread_start();
        SetThreadPriority(self.thread, THREAD_MODE_BACKGROUND_BEGIN);
        self.create_pipe();
        let hr = self.service.check_pending();
        if hr < 0 {
            imports::log_error(b"Failed when checking for incoming traces - %d code\0", &[hr as u32 as u64]);
        }
        loop {
            if ConnectNamedPipe(self.pipe, null_mut()) == 0 {
                let e = GetLastError();
                if e == ERROR_PIPE_CONNECTED {
                    imports::log_info(b"ConnectNamedPipe returned ERROR_PIPE_CONNECTED\0", &[]);
                } else {
                    imports::log_error(b"Failed %d to connect\0", &[e as u64]);
                }
            } else {
                imports::log_info(b"OperationRequestMonitor::AwaitIncomingConnection, got a connection\0", &[]);
            }
            self.serve_one();
            CloseHandle(self.pipe);
            self.create_pipe();
        }
    }
}

unsafe extern "system" fn monitor_thread(param: *mut c_void) -> u32 {
    (*(param as *mut OperationRequestMonitor)).run()
}

pub unsafe fn start_operation_request_monitor(service: &'static dyn OperationService) -> HRESULT {
    let mut slot = MONITOR.lock().unwrap_or_else(|e| e.into_inner());
    if !slot.0.is_null() {
        return S_OK;
    }
    let m = Box::into_raw(Box::new(OperationRequestMonitor { pipe: INVALID_HANDLE_VALUE, thread: INVALID_HANDLE_VALUE, pipe_name: pipe_name(GetCurrentProcessId()), service }));
    slot.0 = m;
    let t = CreateThread(null_mut(), 0x4000, monitor_thread, m as *mut c_void, 0, null_mut());
    (*m).thread = t;
    if t == INVALID_HANDLE_VALUE {
        let e = GetLastError();
        imports::log_error(b"Failed %d to start Operation Request Monitor thread\0", &[e as u64]);
        return hr_from_win32(e);
    }
    imports::log_info(b"Started Operation Request Monitor thread\0", &[]);
    S_OK
}
