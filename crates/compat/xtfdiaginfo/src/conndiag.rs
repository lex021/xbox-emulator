use crate::com::*;
use crate::imports;
use crate::offline::{self, HostClass};
use crate::processdelta::IID_XBTP_MESSAGE_IMPL;
use crate::sys::*;
use core::ffi::c_void;
use core::mem::transmute;
use core::ptr::null_mut;
use std::collections::BTreeMap;

pub const MESSAGE_TYPE_TEST_NAME: f64 = 1.0;
pub const MESSAGE_TYPE_TEST_RESULT: f64 = 2.0;
pub const TEST_TCP_CONNECT: u32 = 2;
pub const TEST_PROCESS_RUNNING: u32 = 8;
pub const TEST_PING: u32 = 0x0b;
pub const TEST_XBTP_LINK: u32 = 0x0d;
pub const TEST_VM_PROCESS_LIST: u32 = 0x0f;
pub const TEST_CROSS_VM_PROCESS: u32 = 0x10;
pub const XBTP_PC_PORT: u16 = 4201;
pub const XBTPLINK_PING_PORT_NUMBER: u16 = 0xff03;
pub const XBTPLINK_CONNECT_TIMEOUT_MS: u32 = 5000;
pub const XBTPLINK_HANDLE_QUERY_TRANSPORT_DATA: u32 = 2;
pub const ICMP_TIMEOUT_MS: u32 = 1000;
pub const ICMP_REQUEST_DATA: [u8; 16] = *b"Data Buffer\0\0\0\0\0";
pub const ICMP_REPLY_BYTES: u32 = 0x38;
pub const ICMP6_REPLY_BYTES: u32 = 0x34;
pub const MESSAGE_BUFFER_CHARS: usize = 0x400;
pub const XTFHOST_IMAGE: &str = "xtfhost.exe";
pub const SYSTEM_VM_NAME: &str = "SYSTEM";
pub const UNKNOWN_PROCESS_NAME: &str = "<unknown>";
pub const CONSOLE_EXTENSION_ID: xtfmsg::sys::GUID = xtfmsg::sys::GUID::new(0x5b45d0e0, 0x858b, 0x49fd, [0xa3, 0x32, 0x7b, 0xdf, 0x8d, 0x25, 0x82, 0x44]);
pub const IID_XBTP_CLIENT_SESSION: xtfmsg::sys::GUID = xtfmsg::sys::GUID::new(0xcf84f553, 0xcd9b, 0x4608, [0x96, 0x40, 0x28, 0x71, 0x4e, 0x5e, 0xd4, 0x40]);
pub const CONSOLE_GET_RUNNING_PROCESSES_OPERATION_ID: u32 = 6;
pub const E_HANDLE: HRESULT = 0x8007_0006u32 as i32;
pub const E_UNEXPECTED: HRESULT = 0x8000_FFFFu32 as i32;
pub const HR_FILE_NOT_FOUND: HRESULT = 0x8007_0002u32 as i32;
pub const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
pub const AF_INET: u16 = 2;
pub const AF_INET6: u16 = 0x17;
pub const SOCK_STREAM: i32 = 1;
pub const IPPROTO_TCP: i32 = 6;
pub const IPPROTO_IPV6: i32 = 0x29;
pub const IPV6_V6ONLY: i32 = 0x1b;
pub const SOL_SOCKET: i32 = 0xffff;
pub const SO_LINGER: i32 = 0x80;
pub const SD_BOTH: i32 = 2;
pub const INVALID_SOCKET: usize = usize::MAX;
pub const SOCKET_ERROR: i32 = -1;

const F_HANDLER: &[u8] = b"ConnectionDiagnosticHandler\0";
const F_RUN: &[u8] = b"RunConsoleTests\0";
const F_LINK: &[u8] = b"XtfXbtpLinkTest\0";
const F_PROC: &[u8] = b"IsProcessRunningTest\0";
const F_PING: &[u8] = b"XtfPingAddress\0";
const F_CONNECT: &[u8] = b"XtfConnectTest\0";
const F_RESOLVE: &[u8] = b"SocketAddress::Resolve\0";
const F_NAME: &[u8] = b"SendTestNameMessage\0";
const F_RESULT: &[u8] = b"SendTestResultMessage\0";
const F_BAG: &[u8] = b"AddPropertyBagToBody\0";
const F_VMLIST: &[u8] = b"GetVMProcessList\0";
const F_CROSSVM: &[u8] = b"CrossVMProcessTest\0";

pub type PropertyBag = BTreeMap<Vec<u16>, Vec<u16>>;

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct SocketAddress {
    pub length: u32,
    pub addr: [u8; 28],
}

impl SocketAddress {
    pub fn family(&self) -> u16 {
        u16::from_le_bytes([self.addr[0], self.addr[1]])
    }
}

#[repr(C)]
pub struct AddrInfoW {
    pub ai_flags: i32,
    pub ai_family: i32,
    pub ai_socktype: i32,
    pub ai_protocol: i32,
    pub ai_addrlen: usize,
    pub ai_canonname: *mut u16,
    pub ai_addr: *mut u8,
    pub ai_next: *mut AddrInfoW,
}

impl Default for AddrInfoW {
    fn default() -> Self {
        AddrInfoW { ai_flags: 0, ai_family: 0, ai_socktype: 0, ai_protocol: 0, ai_addrlen: 0, ai_canonname: null_mut(), ai_addr: null_mut(), ai_next: null_mut() }
    }
}

#[link(name = "ws2_32")]
extern "system" {
    fn GetAddrInfoW(node: *const u16, service: *const u16, hints: *const AddrInfoW, result: *mut *mut AddrInfoW) -> i32;
    fn FreeAddrInfoW(info: *mut AddrInfoW);
    fn socket(af: i32, ty: i32, proto: i32) -> usize;
    fn setsockopt(s: usize, level: i32, name: i32, val: *const u8, len: i32) -> i32;
    fn connect(s: usize, name: *const u8, len: i32) -> i32;
    fn shutdown(s: usize, how: i32) -> i32;
    fn closesocket(s: usize) -> i32;
    fn htons(v: u16) -> u16;
}

#[link(name = "iphlpapi")]
extern "system" {
    fn IcmpCreateFile() -> HANDLE;
    fn Icmp6CreateFile() -> HANDLE;
    fn IcmpCloseHandle(h: HANDLE) -> BOOL;
    fn IcmpSendEcho(h: HANDLE, dest: u32, data: *const u8, size: u16, options: *mut c_void, reply: *mut u8, reply_size: u32, timeout: u32) -> u32;
    fn IcmpParseReplies(reply: *mut u8, reply_size: u32) -> u32;
    fn Icmp6SendEcho2(h: HANDLE, event: HANDLE, apc: *mut c_void, ctx: *mut c_void, source: *const u8, dest: *const u8, data: *const u8, size: u16, options: *mut c_void, reply: *mut u8, reply_size: u32, timeout: u32) -> u32;
    fn Icmp6ParseReplies(reply: *mut u8, reply_size: u32) -> u32;
}

#[link(name = "kernel32")]
extern "system" {
    fn K32EnumProcesses(pids: *mut u32, cb: u32, needed: *mut u32) -> BOOL;
    fn QueryFullProcessImageNameW(h: HANDLE, flags: u32, name: *mut u16, size: *mut u32) -> BOOL;
}

pub unsafe fn chk(hr: HRESULT, expr: &[u8], func: &[u8], line: u32) -> HRESULT {
    if hr < 0 {
        imports::log_error(b"XtfCHK: %s: %d: %s 0x%X\n\0", &[func.as_ptr() as u64, line as u64, expr.as_ptr() as u64, hr as u32 as u64]);
    }
    hr
}

fn trim_wide(s: &[u16]) -> &[u16] {
    let sp = |c: &u16| matches!(*c, 0x20 | 0x09..=0x0d | 0xa0 | 0x1680 | 0x2000..=0x200a | 0x2028 | 0x2029 | 0x202f | 0x205f | 0x3000);
    let start = s.iter().position(|c| !sp(c)).unwrap_or(s.len());
    let end = s.iter().rposition(|c| !sp(c)).map(|e| e + 1).unwrap_or(start);
    &s[start..end.max(start)]
}

pub unsafe fn resolve(address: &[u16], out: &mut SocketAddress) -> HRESULT {
    let host = trim_wide(address).to_vec();
    let text = String::from_utf16_lossy(&host);
    if offline::classify_host(&text) == HostClass::RemoteName {
        return chk(hr_from_win32(offline::WSAHOST_NOT_FOUND), b"GetAddrInfoW(pszHost, pszService, &AddressHints, &pAddressInfo)\0", F_RESOLVE, 95);
    }
    let mut z = host;
    z.push(0);
    let hints = AddrInfoW { ai_socktype: SOCK_STREAM, ai_protocol: IPPROTO_TCP, ..Default::default() };
    let mut res: *mut AddrInfoW = null_mut();
    let rc = GetAddrInfoW(z.as_ptr(), core::ptr::null(), &hints, &mut res);
    let hr = chk(hr_from_win32(rc as u32), b"GetAddrInfoW(pszHost, pszService, &AddressHints, &pAddressInfo)\0", F_RESOLVE, 95);
    if rc != 0 {
        if !res.is_null() {
            FreeAddrInfoW(res);
        }
        return hr;
    }
    let mut v4: *mut AddrInfoW = null_mut();
    let mut v6: *mut AddrInfoW = null_mut();
    let mut v6_link: *mut AddrInfoW = null_mut();
    let mut p = res;
    while !p.is_null() && v4.is_null() {
        let a = &*p;
        if a.ai_family == AF_INET as i32 {
            if a.ai_addrlen == 0x10 {
                v4 = p;
            }
        } else if a.ai_family == AF_INET6 as i32 && a.ai_addrlen == 0x1c {
            let b = core::slice::from_raw_parts(a.ai_addr, 0x1c);
            if b[8] == 0xfe && (b[9] & 0xc0) == 0x80 {
                if v6_link.is_null() {
                    v6_link = p;
                }
            } else if v6.is_null() {
                v6 = p;
            }
        }
        p = a.ai_next;
    }
    let pick = if !v4.is_null() {
        v4
    } else if !v6.is_null() {
        v6
    } else {
        v6_link
    };
    let hr = if pick.is_null() {
        chk(E_UNEXPECTED, b"E_UNEXPECTED\0", F_RESOLVE, 155)
    } else {
        let a = &*pick;
        let n = a.ai_addrlen.min(28);
        out.length = a.ai_addrlen as u32;
        core::ptr::copy_nonoverlapping(a.ai_addr, out.addr.as_mut_ptr(), n);
        S_OK
    };
    FreeAddrInfoW(res);
    hr
}

pub unsafe fn ping_address(address: &[u16]) -> HRESULT {
    let mut addr = SocketAddress::default();
    let hr = chk(resolve(address, &mut addr), b"SocketAddress::Resolve(address, nullptr, &addr)\0", F_PING, 25);
    if hr < 0 {
        return hr;
    }
    let loopback = offline::sockaddr_is_loopback(&addr.addr);
    let mut data = ICMP_REQUEST_DATA;
    match addr.family() {
        AF_INET => {
            let h = IcmpCreateFile();
            if h == INVALID_HANDLE_VALUE {
                imports::log_error(b"IcmpCreateFile failed\0", &[]);
                let hr = chk(hr_last_error(), b"HRESULT_FROM_WIN32(GetLastError())\0", F_PING, 37);
                if hr < 0 {
                    return hr;
                }
            }
            let mut reply = [0u8; ICMP_REPLY_BYTES as usize];
            let dest = u32::from_le_bytes([addr.addr[4], addr.addr[5], addr.addr[6], addr.addr[7]]);
            let hr = if loopback {
                let n = IcmpSendEcho(h, dest, data.as_mut_ptr(), 0x10, null_mut(), reply.as_mut_ptr(), ICMP_REPLY_BYTES, ICMP_TIMEOUT_MS);
                if n == 0 {
                    chk(hr_last_error(), b"HRESULT_FROM_WIN32(GetLastError())\0", F_PING, 67)
                } else if IcmpParseReplies(reply.as_mut_ptr(), ICMP_REPLY_BYTES) == 0 {
                    chk(hr_last_error(), b"HRESULT_FROM_WIN32(GetLastError())\0", F_PING, 56)
                } else {
                    let status = u32::from_le_bytes([reply[4], reply[5], reply[6], reply[7]]);
                    chk(hr_from_win32(status), b"HRESULT_FROM_WIN32(pEchoReply->Status)\0", F_PING, 62)
                }
            } else {
                chk(hr_from_win32(offline::IP_DEST_HOST_UNREACHABLE), b"HRESULT_FROM_WIN32(pEchoReply->Status)\0", F_PING, 62)
            };
            if h != INVALID_HANDLE_VALUE {
                IcmpCloseHandle(h);
            }
            hr
        }
        AF_INET6 => {
            let mut source = [0u8; 28];
            source[0..2].copy_from_slice(&AF_INET6.to_le_bytes());
            let h = Icmp6CreateFile();
            if h == INVALID_HANDLE_VALUE {
                imports::log_error(b"Icmp6CreateFile failed\0", &[]);
                let hr = chk(hr_last_error(), b"HRESULT_FROM_WIN32(GetLastError())\0", F_PING, 85);
                if hr < 0 {
                    return hr;
                }
            }
            let mut reply = [0u8; ICMP6_REPLY_BYTES as usize];
            let hr = if loopback {
                let n = Icmp6SendEcho2(h, null_mut(), null_mut(), null_mut(), source.as_ptr(), addr.addr.as_ptr(), data.as_mut_ptr(), 0x10, null_mut(), reply.as_mut_ptr(), ICMP6_REPLY_BYTES, ICMP_TIMEOUT_MS);
                if n == 0 {
                    chk(hr_last_error(), b"HRESULT_FROM_WIN32(GetLastError())\0", F_PING, 119)
                } else if Icmp6ParseReplies(reply.as_mut_ptr(), ICMP6_REPLY_BYTES) == 0 {
                    chk(hr_last_error(), b"HRESULT_FROM_WIN32(GetLastError())\0", F_PING, 108)
                } else {
                    let status = u32::from_le_bytes([reply[0x1c], reply[0x1d], reply[0x1e], reply[0x1f]]);
                    chk(hr_from_win32(status), b"HRESULT_FROM_WIN32(pEchoReply->Status)\0", F_PING, 114)
                }
            } else {
                chk(hr_from_win32(offline::IP_DEST_HOST_UNREACHABLE), b"HRESULT_FROM_WIN32(pEchoReply->Status)\0", F_PING, 114)
            };
            if h != INVALID_HANDLE_VALUE {
                IcmpCloseHandle(h);
            }
            hr
        }
        _ => chk(E_FAIL, b"E_FAIL\0", F_PING, 124),
    }
}

unsafe fn wsa_hr() -> (u32, HRESULT) {
    let e = GetLastError();
    (e, hr_from_win32(e))
}

pub unsafe fn connect_test(address: &[u16]) -> HRESULT {
    let mut addr = SocketAddress::default();
    let hr = chk(resolve(address, &mut addr), b"SocketAddress::Resolve(address, nullptr, &addr)\0", F_CONNECT, 204);
    if hr < 0 {
        return hr;
    }
    let (af, len, lines, exprs): (u16, i32, [u32; 4], [&[u8]; 3]) = match addr.family() {
        AF_INET => (AF_INET, 0x10, [210, 211, 0, 212], [b"s = socket(AF_INET, SOCK_STREAM, IPPROTO_TCP)\0", b"connect(s, (sockaddr*)&addr.SockAddrINet.Ipv4, sizeof(addr.SockAddrINet.Ipv4))\0", b"\0"]),
        AF_INET6 => (AF_INET6, 0x1c, [218, 219, 220, 221], [b"s = socket(AF_INET6, SOCK_STREAM, IPPROTO_TCP)\0", b"connect(s, (sockaddr*)&addr.SockAddrINet.Ipv6, sizeof(addr.SockAddrINet.Ipv6))\0", b"setsockopt(s, IPPROTO_IPV6, IPV6_V6ONLY, (const char*)&ipv6, sizeof(ipv6))\0"]),
        _ => return chk(E_INVALIDARG, b"E_INVALIDARG\0", F_CONNECT, 225),
    };
    let port = htons(XBTP_PC_PORT).to_ne_bytes();
    addr.addr[2] = port[0];
    addr.addr[3] = port[1];
    let s = socket(af as i32, SOCK_STREAM, IPPROTO_TCP);
    if s == INVALID_SOCKET {
        let (e, hr) = wsa_hr();
        chk(hr, exprs[0], F_CONNECT, lines[0]);
        if e != 0 {
            return hr;
        }
    }
    let mut result = S_OK;
    'run: {
        let linger = [0u8; 4];
        if setsockopt(s, SOL_SOCKET, SO_LINGER, linger.as_ptr(), 4) == SOCKET_ERROR {
            let (e, hr) = wsa_hr();
            chk(hr, b"setsockopt(s, SOL_SOCKET, SO_LINGER, (const char*)&linger, sizeof(linger))\0", F_CONNECT, lines[1]);
            if e != 0 {
                result = hr;
                break 'run;
            }
        }
        if af == AF_INET6 {
            let v6only = 0u32.to_le_bytes();
            if setsockopt(s, IPPROTO_IPV6, IPV6_V6ONLY, v6only.as_ptr(), 4) == SOCKET_ERROR {
                let (e, hr) = wsa_hr();
                chk(hr, exprs[2], F_CONNECT, lines[2]);
                if e != 0 {
                    result = hr;
                    break 'run;
                }
            }
        }
        if offline::sockaddr_is_loopback(&addr.addr) {
            if connect(s, addr.addr.as_ptr(), len) == SOCKET_ERROR {
                let (_, hr) = wsa_hr();
                result = chk(hr, exprs[1], F_CONNECT, lines[3]);
            }
        } else {
            result = chk(hr_from_win32(offline::WSAEHOSTUNREACH), exprs[1], F_CONNECT, lines[3]);
        }
    }
    if s != INVALID_SOCKET {
        shutdown(s, SD_BOTH);
        closesocket(s);
    }
    result
}

pub struct LinkResult {
    pub hr: HRESULT,
    pub tcp_connections: u32,
    pub xtd_connections: u32,
}

pub unsafe fn xbtp_link_test(address: &[u16]) -> LinkResult {
    let mut r = LinkResult { hr: S_OK, tcp_connections: 0, xtd_connections: 0 };
    let i = match imports::get() {
        Ok(i) => i,
        Err(e) => {
            r.hr = e;
            return r;
        }
    };
    let h = (i.link_create_handle)();
    if h.is_null() {
        r.hr = chk(E_HANDLE, b"E_HANDLE\0", F_LINK, 148);
        return r;
    }
    let mut z = address.to_vec();
    z.push(0);
    r.hr = chk((i.link_connect_by_name)(h, z.as_ptr(), XBTPLINK_PING_PORT_NUMBER, XBTPLINK_CONNECT_TIMEOUT_MS), b"XbtpLinkConnectByName(connectHandle, address, XBTPLINK_PING_PORT_NUMBER, 5000)\0", F_LINK, 151);
    if r.hr >= 0 {
        let mut data = [0u8; 8];
        r.hr = chk((i.link_query_handle_information)(h, XBTPLINK_HANDLE_QUERY_TRANSPORT_DATA, 8, data.as_mut_ptr()), b"XbtpLinkQueryHandleInformation(connectHandle, XBTPLINK_HANDLE_QUERY_TRANSPORT_DATA, sizeof(transportData), &transportData)\0", F_LINK, 152);
        if r.hr >= 0 {
            r.tcp_connections = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
            r.xtd_connections = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
        }
    }
    (i.link_close_handle)(h);
    r
}

fn lower(s: &[u16]) -> Vec<u16> {
    s.iter().map(|&c| if (b'A' as u16..=b'Z' as u16).contains(&c) { c + 0x20 } else { c }).collect()
}

fn contains(hay: &[u16], needle: &[u16]) -> bool {
    needle.is_empty() || hay.windows(needle.len()).any(|w| w == needle)
}

pub unsafe fn get_process_count(image: &str, count: &mut u32) -> HRESULT {
    let mut name: Vec<u16> = image.encode_utf16().take(0x103).collect();
    name = lower(&name);
    let mut cb = 0x80u32;
    let pids: Vec<u32> = loop {
        let mut buf = vec![0u32; (cb / 4) as usize];
        let mut needed = 0u32;
        if K32EnumProcesses(buf.as_mut_ptr(), cb, &mut needed) == 0 {
            let e = GetLastError();
            return if e != 0 { hr_from_win32(e) } else { E_FAIL };
        }
        if needed < cb {
            buf.truncate((needed / 4) as usize);
            break buf;
        }
        cb *= 2;
    };
    let mut n = 0u32;
    for pid in pids {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            continue;
        }
        let mut path = [0u16; 0x104];
        let mut size = 0x104u32;
        if QueryFullProcessImageNameW(h, 0, path.as_mut_ptr(), &mut size) != 0 {
            let p = lower(&path[..size.min(0x104) as usize]);
            if contains(&p, &name) {
                n += 1;
            }
        }
        CloseHandle(h);
    }
    *count = n;
    if n == 0 {
        HR_FILE_NOT_FOUND
    } else {
        S_OK
    }
}

fn bag_add(bag: &mut PropertyBag, name: &str, value: &[u16]) -> HRESULT {
    bag.insert(name.encode_utf16().collect(), value.to_vec());
    S_OK
}

fn w(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

unsafe fn arr_add_string(a: Obj, s: *const u16) -> HRESULT {
    let f: unsafe extern "system" fn(Obj, *const u16) -> HRESULT = transmute(slot(a, 4));
    f(a, s)
}

pub unsafe fn add_property_bag_to_body(body: Obj, properties: Option<&PropertyBag>) -> HRESULT {
    let i = match imports::get() {
        Ok(i) => i,
        Err(e) => return e,
    };
    let mut bag = Ptr::null();
    let mut names = Ptr::null();
    let mut values = Ptr::null();
    let hr = chk((i.create_json_object)(bag.out()), b"XbtpCreateJsonObject(&bag)\0", F_BAG, 47);
    if hr < 0 {
        return hr;
    }
    let hr = chk((i.create_json_array)(names.out()), b"XbtpCreateJsonArray(&names)\0", F_BAG, 48);
    if hr < 0 {
        return hr;
    }
    let hr = chk((i.create_json_array)(values.out()), b"XbtpCreateJsonArray(&values)\0", F_BAG, 49);
    if hr < 0 {
        return hr;
    }
    if let Some(props) = properties {
        for (k, v) in props.iter() {
            let mut kz = k.clone();
            kz.push(0);
            let mut vz = v.clone();
            vz.push(0);
            arr_add_string(names.0, kz.as_ptr());
            arr_add_string(values.0, vz.as_ptr());
        }
        obj_set_array(bag.0, wz("propertyBagNames").as_ptr(), names.0);
        obj_set_array(bag.0, wz("propertyBagValues").as_ptr(), values.0);
        obj_set_object(body, wz("propertyBag").as_ptr(), bag.0);
    }
    hr
}

unsafe fn serialize_into_message(body: Obj, message: Obj) -> HRESULT {
    if body.is_null() || message.is_null() {
        return E_INVALIDARG;
    }
    let mut s = Bstr::null();
    let hr = obj_stringify(body, s.out());
    if hr < 0 {
        return hr;
    }
    let f: unsafe extern "system" fn(Obj, *const u16) -> HRESULT = transmute(slot(message, 7));
    let hr = f(message, s.0);
    if hr < 0 {
        hr
    } else {
        S_OK
    }
}

unsafe fn send_more(session: Obj, body: Obj, func: &[u8], lines: [u32; 3]) -> HRESULT {
    let i = match imports::get() {
        Ok(i) => i,
        Err(e) => return e,
    };
    let mut more = Ptr::null();
    let hr = chk((i.create_message)(&IID_XBTP_MESSAGE_IMPL, more.out()), b"XbtpCreateMessage(IID_PPV_ARGS(&moreMessage))\0", func, lines[0]);
    if hr < 0 {
        return hr;
    }
    let hr = chk(serialize_into_message(body, more.0), b"XtfSerializeJsonBody(body, moreMessage)\0", func, lines[1]);
    if hr < 0 {
        return hr;
    }
    let f: unsafe extern "system" fn(Obj, Obj) -> HRESULT = transmute(slot(session, 3));
    chk(f(session, more.0), b"session->SendMoreMessage(moreMessage)\0", func, lines[2])
}

pub fn format_test_message(text: &str) -> Vec<u16> {
    let mut v: Vec<u16> = text.encode_utf16().take(MESSAGE_BUFFER_CHARS - 1).collect();
    v.push(0);
    v
}

pub unsafe fn send_test_name_message(session: Obj, test_type: u32, properties: Option<&PropertyBag>, message: &str) -> HRESULT {
    let i = match imports::get() {
        Ok(i) => i,
        Err(e) => return e,
    };
    let buff = format_test_message(message);
    let mut body = Ptr::null();
    let mut bag = Ptr::null();
    let hr = chk((i.create_json_object)(body.out()), b"XbtpCreateJsonObject(&body)\0", F_NAME, 103);
    if hr < 0 {
        return hr;
    }
    let hr = chk((i.create_json_object)(bag.out()), b"XbtpCreateJsonObject(&bag)\0", F_NAME, 104);
    if hr < 0 {
        return hr;
    }
    let hr = chk(obj_set_number(body.0, wz("diagMessageType").as_ptr(), MESSAGE_TYPE_TEST_NAME), b"body->SetNumber(MessageTypeProperty, MessageType_TestName)\0", F_NAME, 105);
    if hr < 0 {
        return hr;
    }
    let hr = chk(obj_set_number(body.0, wz("diagTestType").as_ptr(), test_type as i32 as f64), b"body->SetNumber(TestTypeProperty, testType)\0", F_NAME, 106);
    if hr < 0 {
        return hr;
    }
    let hr = chk(obj_set_string(body.0, wz("diagTestMessage").as_ptr(), buff.as_ptr()), b"body->SetString(TestMessageProperty, buff)\0", F_NAME, 107);
    if hr < 0 {
        return hr;
    }
    let hr = chk(add_property_bag_to_body(body.0, properties), b"AddPropertyBagToBody(body, properties)\0", F_NAME, 110);
    if hr < 0 {
        return hr;
    }
    send_more(session, body.0, F_NAME, [113, 114, 117])
}

pub unsafe fn send_test_result_message(session: Obj, test_type: u32, test_hr: HRESULT, properties: Option<&PropertyBag>) -> HRESULT {
    let i = match imports::get() {
        Ok(i) => i,
        Err(e) => return e,
    };
    let mut body = Ptr::null();
    let mut bag = Ptr::null();
    let hr = chk((i.create_json_object)(body.out()), b"XbtpCreateJsonObject(&body)\0", F_RESULT, 138);
    if hr < 0 {
        return hr;
    }
    let hr = chk((i.create_json_object)(bag.out()), b"XbtpCreateJsonObject(&bag)\0", F_RESULT, 139);
    if hr < 0 {
        return hr;
    }
    let hr = chk(obj_set_number(body.0, wz("diagMessageType").as_ptr(), MESSAGE_TYPE_TEST_RESULT), b"body->SetNumber(MessageTypeProperty, MessageType_TestResult)\0", F_RESULT, 140);
    if hr < 0 {
        return hr;
    }
    let hr = chk(obj_set_number(body.0, wz("diagTestType").as_ptr(), test_type as i32 as f64), b"body->SetNumber(TestTypeProperty, testType)\0", F_RESULT, 141);
    if hr < 0 {
        return hr;
    }
    let hr = chk(obj_set_number(body.0, wz("diagTestResult").as_ptr(), test_hr as f64), b"body->SetNumber(TestHResultProperty, testhr)\0", F_RESULT, 142);
    if hr < 0 {
        return hr;
    }
    let hr = chk(add_property_bag_to_body(body.0, properties), b"AddPropertyBagToBody(body, properties)\0", F_RESULT, 145);
    if hr < 0 {
        return hr;
    }
    send_more(session, body.0, F_RESULT, [148, 149, 152])
}

pub unsafe fn is_process_running_test(session: Obj) -> HRESULT {
    let mut bag = PropertyBag::new();
    send_test_name_message(session, TEST_PROCESS_RUNNING, None, &format!("Process running [{}]", XTFHOST_IMAGE));
    let mut hr = chk(bag_add(&mut bag, "ProcessName", &w(XTFHOST_IMAGE)), b"SafePropertyBagAdd(&bag, Property_ProcessName, processImageName)\0", F_PROC, 268);
    if hr >= 0 {
        let mut count = 0u32;
        hr = chk(get_process_count(XTFHOST_IMAGE, &mut count), b"GetProcessCount(processImageName, &procCount, true)\0", F_PROC, 270);
        if hr >= 0 && count != 1 {
            hr = chk(HR_FILE_NOT_FOUND, b"HRESULT_FROM_WIN32(ERROR_FILE_NOT_FOUND)\0", F_PROC, 277);
        }
    }
    send_test_result_message(session, TEST_PROCESS_RUNNING, hr, Some(&bag));
    hr
}

pub unsafe fn get_vm_process_list(session: Obj, list: &mut Ptr) -> HRESULT {
    let mut bag = PropertyBag::new();
    let mut resp = Ptr::null();
    let mut cross = Ptr::null();
    send_test_name_message(session, TEST_VM_PROCESS_LIST, None, &format!("Get process list for [{}]", SYSTEM_VM_NAME));
    let hr = 'run: {
        let hr = chk(bag_add(&mut bag, "TargetVM", &w(SYSTEM_VM_NAME)), b"SafePropertyBagAdd(&bag, Property_TargetVM, PortToName[targetVm])\0", F_VMLIST, 172);
        if hr < 0 {
            break 'run hr;
        }
        let i = match imports::get() {
            Ok(i) => i,
            Err(e) => break 'run e,
        };
        let hr = chk((i.create_json_object)(resp.out()), b"XbtpCreateJsonObject(&respBody)\0", F_VMLIST, 174);
        if hr < 0 {
            break 'run hr;
        }
        let hr = chk(xtfmsg::api::open_pipe_to_system(&CONSOLE_EXTENSION_ID, &IID_XBTP_CLIENT_SESSION, cross.out()), b"XtfOpenPipeToSystem(ConsoleExtensionId, IID_PPV_ARGS(&crossVmSession))\0", F_VMLIST, 182);
        if hr < 0 {
            break 'run hr;
        }
        let hr = chk(xtfmsg::api::send_json_message(cross.0, null_mut(), CONSOLE_GET_RUNNING_PROCESSES_OPERATION_ID, resp.out()), b"XtfSendJsonMessage(crossVmSession, NULL, ConsoleGetRunningProcessesOperationId, &respBody)\0", F_VMLIST, 192);
        if hr < 0 {
            break 'run hr;
        }
        chk(obj_get_array(resp.0, wz("Processes").as_ptr(), list.out()), b"respBody->GetArray(ProcessesArrayProperty, processList)\0", F_VMLIST, 193)
    };
    send_test_result_message(session, TEST_VM_PROCESS_LIST, hr, Some(&bag));
    hr
}

fn leaf_name(path: &[u16]) -> Vec<u16> {
    let mut out = Vec::new();
    for &c in path.iter().take(0x103) {
        if c == b'\\' as u16 {
            out.clear();
        } else {
            out.push(c);
        }
    }
    out
}

fn wide_ieq(a: &[u16], b: &[u16]) -> bool {
    a.len() == b.len() && lower(a) == lower(b)
}

pub unsafe fn cross_vm_process_test(session: Obj, list: Obj) -> HRESULT {
    let mut bag = PropertyBag::new();
    send_test_name_message(session, TEST_CROSS_VM_PROCESS, None, &format!("Process running [{}] on [{}]", XTFHOST_IMAGE, SYSTEM_VM_NAME));
    let hr = 'run: {
        let hr = chk(bag_add(&mut bag, "ProcessName", &w(XTFHOST_IMAGE)), b"SafePropertyBagAdd(&bag, Property_ProcessName, processImageName)\0", F_CROSSVM, 217);
        if hr < 0 {
            break 'run hr;
        }
        let hr = chk(bag_add(&mut bag, "TargetVM", &w(SYSTEM_VM_NAME)), b"SafePropertyBagAdd(&bag, Property_TargetVM, targetVm)\0", F_CROSSVM, 218);
        if hr < 0 {
            break 'run hr;
        }
        let mut n = 0u32;
        let hr = chk(arr_get_count(list, &mut n), b"processList->GetSize(&cProcesses)\0", F_CROSSVM, 220);
        if hr < 0 {
            break 'run hr;
        }
        let target = w(XTFHOST_IMAGE);
        for idx in 0..n {
            let mut process = Ptr::null();
            let hr = chk(arr_get_object(list, idx, process.out()), b"processList->GetObjectAt(i, &process)\0", F_CROSSVM, 224);
            if hr < 0 {
                break 'run hr;
            }
            let mut name = Bstr::null();
            let r = obj_get_string(process.0, wz("ProcessName").as_ptr(), name.out());
            let mut found_hr = r;
            if r < 0 || name.0.is_null() || *name.0 == 0 {
                let u = wz(UNKNOWN_PROCESS_NAME);
                let fresh = SysAllocString(u.as_ptr());
                if !fresh.is_null() {
                    name.reset();
                    name.0 = fresh;
                }
                found_hr = S_OK;
            }
            if wide_ieq(&leaf_name(name.as_slice()), &target) {
                break 'run found_hr;
            }
        }
        chk(HR_FILE_NOT_FOUND, b"HRESULT_FROM_WIN32(ERROR_FILE_NOT_FOUND)\0", F_CROSSVM, 245)
    };
    send_test_result_message(session, TEST_CROSS_VM_PROCESS, hr, Some(&bag));
    hr
}

unsafe fn itow(v: u32) -> Vec<u16> {
    w(&v.to_string())
}

pub unsafe fn run_console_tests(session: Obj, addresses: Obj) -> HRESULT {
    let mut size = 0u32;
    let hr = chk(arr_get_count(addresses, &mut size), b"pcAddressArray->GetSize(&arraySize)\0", F_RUN, 333);
    if hr < 0 {
        return hr;
    }
    let mut bag = PropertyBag::new();
    for idx in 0..size {
        let mut ip = Bstr::null();
        let hr = chk(arr_get_string(addresses, idx, ip.out()), b"pcAddressArray->GetStringAt(i, &ipAddress)\0", F_RUN, 341);
        if hr < 0 {
            return hr;
        }
        let address = ip.as_slice().to_vec();
        let shown = String::from_utf16_lossy(&address);
        send_test_name_message(session, TEST_PING, None, &format!("Ping address [{}]", shown));
        let ping = ping_address(&address);
        send_test_result_message(session, TEST_PING, ping, None);
        send_test_name_message(session, TEST_XBTP_LINK, None, &format!("XBTP Link [{}]", shown));
        let link = xbtp_link_test(&address);
        let hr = chk(bag_add(&mut bag, "TCPConnections", &itow(link.tcp_connections)), b"SafePropertyBagAdd(&bag, Property_TcpConnections, strVal)\0", F_RUN, 358);
        if hr < 0 {
            return hr;
        }
        let hr = chk(bag_add(&mut bag, "XTDConnections", &itow(link.xtd_connections)), b"SafePropertyBagAdd(&bag, Property_XtdConnections, strVal)\0", F_RUN, 364);
        if hr < 0 {
            return hr;
        }
        send_test_result_message(session, TEST_XBTP_LINK, link.hr, Some(&bag));
        bag.clear();
        send_test_name_message(session, TEST_TCP_CONNECT, None, &format!("Connect to XBTP on PC [{}, Port {}]", shown, XBTP_PC_PORT));
        let tcp = connect_test(&address);
        bag.insert(w("Port"), itow(XBTP_PC_PORT as u32));
        bag.insert(w("Service"), w("XBTP"));
        send_test_result_message(session, TEST_TCP_CONNECT, tcp, Some(&bag));
        bag.clear();
    }
    is_process_running_test(session);
    let mut list = Ptr::null();
    let hr = get_vm_process_list(session, &mut list);
    if hr >= 0 {
        cross_vm_process_test(session, list.0);
    }
    hr
}

pub unsafe extern "system" fn connection_diagnostic_handler(session: usize, received: usize, _reply: usize) -> HRESULT {
    let session = session as Obj;
    let received = received as Obj;
    let hr = 'run: {
        let i = match imports::get() {
            Ok(i) => i,
            Err(e) => break 'run e,
        };
        let mut len = 0u32;
        let mut body: *mut u8 = null_mut();
        let hr = chk(msg_get_json_body(received, &mut len, &mut body), b"received->GetJsonBody(&jsonBodyLength, &jsonBody)\0", F_HANDLER, 517);
        if hr < 0 {
            break 'run hr;
        }
        if body.is_null() {
            break 'run chk(E_UNEXPECTED, b"E_UNEXPECTED\0", F_HANDLER, 520);
        }
        let mut obj = Ptr::null();
        let hr = chk((i.get_json_object_from_utf8)(body, obj.out()), b"XbtpGetJsonObjectFromUtf8(jsonBody, &jsonBodyObject)\0", F_HANDLER, 524);
        if hr < 0 {
            break 'run hr;
        }
        let mut addresses = Ptr::null();
        let hr = chk(obj_get_array(obj.0, wz("diagAddressArray").as_ptr(), addresses.out()), b"jsonBodyObject->GetArray(AddressArrayProperty, &pcAddressArray)\0", F_HANDLER, 527);
        if hr < 0 {
            break 'run hr;
        }
        chk(run_console_tests(session, addresses.0), b"RunConsoleTests(session, pcAddressArray)\0", F_HANDLER, 530)
    };
    if hr < 0 {
        imports::log_error(b"XtfDiagInfo: ConnectionDiagnosticHandler failed 0x%X\n\0", &[hr as u32 as u64]);
    }
    hr
}
