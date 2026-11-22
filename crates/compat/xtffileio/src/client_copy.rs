use crate::client::*;
use crate::com::*;
use crate::file::{self, FileInfo, INVALID_FILE_ATTRIBUTES};
use crate::find::{append_path, find_files, relative_file_name, FindCallback, FoundFile};
use crate::msg::*;
use crate::paths::{full_path_name, path_type, PATH_APPLICATION, PATH_CONSOLE};
use crate::transfer::{optimal_chunks, FileReader, FileWriter, TransferRequest};
use core::ffi::c_void;
use core::ptr::null_mut;

pub const E_NOTIMPL: HRESULT = 0x8000_4001u32 as i32;
pub const E_UNEXPECTED: HRESULT = 0x8000_FFFFu32 as i32;
pub const E_PENDING: HRESULT = 0x8000_000Au32 as i32;
pub const HR_FILE_NOT_FOUND: HRESULT = 0x8007_0002u32 as i32;
pub const HR_PATH_NOT_FOUND: HRESULT = 0x8007_0003u32 as i32;
pub const HR_ACCESS_DENIED: HRESULT = 0x8007_0005u32 as i32;
pub const HR_SHARING_VIOLATION: HRESULT = 0x8007_0020u32 as i32;
pub const HR_IO_DEVICE: HRESULT = 0x8007_045Du32 as i32;
pub const HR_DIRECT_ACCESS_DENIED: HRESULT = 0x8d16_0181u32 as i32;
pub const HR_DIRECT_SHARING_VIOLATION: HRESULT = 0x8d16_0182u32 as i32;
pub const OP_SEND_FILE: u32 = 0xa;
pub const OP_RECV_FILE: u32 = 0xb;
pub const OP_TRANSFER_TO_CONSOLE: u32 = 0x1b;
pub const OP_TRANSFER_FROM_CONSOLE: u32 = 0x1c;
pub const OP_LAST_DIRECT_PROTOCOL_ERROR: u32 = 0x1d;
pub const OP_TRANSFER_TO_CONSOLE_NO_JSON: u32 = 0x1e;
pub const OP_TRANSFER_FROM_CONSOLE_NO_JSON: u32 = 0x1f;
pub const LEGACY_RECEIVE_TIMEOUT_MS: u32 = 120000;
pub const DIRECT_RECEIVE_TIMEOUT_MS: u32 = 30000;
pub const PERF_SIZE: usize = 0xc8;
pub const PERF_READ_PENDING: usize = 8;
pub const PERF_WRITE_PENDING: usize = 0x68;
const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;

type P = *mut c_void;
type W = *const u16;

#[repr(C)]
pub struct CopyCallbackVtbl {
    pub base: UnknownVtbl,
    pub on_start_file_copy: unsafe extern "system" fn(P, W, W, *const XtfFileInfo, W) -> HRESULT,
    pub on_file_copy_progress: unsafe extern "system" fn(P, W, W, u64, u64) -> HRESULT,
    pub on_end_file_copy: unsafe extern "system" fn(P, W, W, *const XtfFileInfo, W, HRESULT) -> HRESULT,
}

unsafe fn units(p: W) -> Vec<u16> {
    if p.is_null() {
        return Vec::new();
    }
    let mut n = 0;
    while *p.add(n) != 0 {
        n += 1;
    }
    core::slice::from_raw_parts(p, n).to_vec()
}

fn xtf_info(name: &[u16], info: &FileInfo) -> XtfFileInfo {
    XtfFileInfo { file_name: name.as_ptr(), attributes: info.attributes, creation: info.creation, access: info.access, write: info.write, size: info.size, reserved: [0; 3] }
}

pub struct Notify {
    pub callback: P,
}

impl Notify {
    pub unsafe fn start(&mut self, root: &str, pattern: &str, name: &str, info: &FileInfo, dest: &str) -> HRESULT {
        if self.callback.is_null() {
            return S_OK;
        }
        let (r, p, n, d) = (wide(root), wide(pattern), wide(name), wide(dest));
        let x = xtf_info(&n, info);
        (vt::<CopyCallbackVtbl>(self.callback).on_start_file_copy)(self.callback, r.as_ptr(), p.as_ptr(), &x, d.as_ptr())
    }

    pub unsafe fn progress(&mut self, src: &str, dst: &str, size: u64, done: u64) -> HRESULT {
        if self.callback.is_null() {
            return S_OK;
        }
        let (s, d) = (wide(src), wide(dst));
        (vt::<CopyCallbackVtbl>(self.callback).on_file_copy_progress)(self.callback, s.as_ptr(), d.as_ptr(), size, done)
    }

    pub unsafe fn end(&mut self, root: &str, pattern: &str, name: &str, info: &FileInfo, dest: &str, hr: HRESULT) -> HRESULT {
        if self.callback.is_null() {
            return S_OK;
        }
        let (r, p, n, d) = (wide(root), wide(pattern), wide(name), wide(dest));
        let x = xtf_info(&n, info);
        (vt::<CopyCallbackVtbl>(self.callback).on_end_file_copy)(self.callback, r.as_ptr(), p.as_ptr(), &x, d.as_ptr(), hr)
    }
}

#[cfg(fileio_b)]
impl crate::fileio_b::CopyNotify for Notify {
    unsafe fn start(&mut self, root: &str, pattern: &str, name: &str, info: &FileInfo, dest: &str) -> HRESULT {
        Notify::start(self, root, pattern, name, info, dest)
    }
    unsafe fn progress(&mut self, src: &str, dst: &str, size: u64, done: u64) -> HRESULT {
        Notify::progress(self, src, dst, size, done)
    }
    unsafe fn end(&mut self, root: &str, pattern: &str, name: &str, info: &FileInfo, dest: &str, hr: HRESULT) -> HRESULT {
        Notify::end(self, root, pattern, name, info, dest, hr)
    }
}

pub struct Entry {
    pub name: String,
    pub info: FileInfo,
}

pub struct Found {
    pub root: String,
    pub search: String,
    pub entries: Vec<Entry>,
}

struct Collect<'a> {
    root: &'a mut Option<String>,
    search: &'a mut Option<String>,
    out: &'a mut Vec<Entry>,
}

impl FindCallback for Collect<'_> {
    fn on_found_file(&mut self, root: &str, pattern: Option<&str>, found: &FoundFile) -> HRESULT {
        if self.root.is_none() {
            *self.root = Some(root.to_string());
            *self.search = pattern.map(|p| p.to_string());
        }
        self.out.push(Entry { name: found.file_name.clone(), info: FileInfo { ..found.info } });
        S_OK
    }
}

fn local_find(pattern: &str, include: u32, exclude: u32, recursion: u32) -> Result<Found, HRESULT> {
    let mut root = None;
    let mut search = None;
    let mut entries = Vec::new();
    let hr = find_files(pattern, include, exclude, recursion, &mut Collect { root: &mut root, search: &mut search, out: &mut entries });
    if hr < 0 {
        return Err(hr);
    }
    Ok(Found { root: root.unwrap_or_default(), search: search.unwrap_or_default(), entries })
}

unsafe fn remote_find(c: &FileIOClient, pattern: &str, include: u32, exclude: u32, recursion: u32) -> Result<Found, HRESULT> {
    let w = wide(pattern);
    let mut list = FileList::default();
    let mut fr = FindResult { root: Bstr::null(), pattern: Bstr::null() };
    let hr = c.list_files(w.as_ptr(), include, exclude, recursion, 0, &mut list, Some(&mut fr));
    if hr < 0 {
        return Err(hr);
    }
    let entries = list
        .entries
        .iter()
        .map(|e| Entry {
            name: String::from_utf16_lossy(bstr_slice(&e.name)),
            info: FileInfo { attributes: e.info.attributes, creation: e.info.creation, access: e.info.access, write: e.info.write, size: e.info.size },
        })
        .collect();
    Ok(Found { root: fr.root.to_string(), search: fr.pattern.to_string(), entries })
}

unsafe fn attributes_of(c: &FileIOClient, remote: bool, path: &str) -> (HRESULT, u32) {
    let (hr, a) = if remote {
        let w = wide(path);
        let mut a = INVALID_FILE_ATTRIBUTES;
        let hr = c.remote_file().get_file_attributes(w.as_ptr(), &mut a);
        (hr, if hr < 0 { INVALID_FILE_ATTRIBUTES } else { a })
    } else {
        match full_path_name(path).and_then(|f| file::get_file_attributes(&f)) {
            Ok(a) => (S_OK, a),
            Err(e) => (e, INVALID_FILE_ATTRIBUTES),
        }
    };
    if hr == HR_FILE_NOT_FOUND || hr == HR_PATH_NOT_FOUND {
        return (S_OK, INVALID_FILE_ATTRIBUTES);
    }
    (hr, a)
}

struct Side {
    kind: u32,
    remote: bool,
    path: String,
}

fn side_of(p: &[u16]) -> Result<Side, HRESULT> {
    let (kind, rel) = path_type(p)?;
    let path = if kind == PATH_CONSOLE { String::from_utf16_lossy(&p[rel..]) } else { String::from_utf16_lossy(p) };
    Ok(Side { kind, remote: kind != 0, path })
}

fn has_wildcard(s: &str) -> bool {
    s.contains('*') || s.contains('?')
}

unsafe fn message() -> Result<Com, HRESULT> {
    let mut m = Com::null();
    let hr = XbtpCreateMessage(&IID_XBTP_MESSAGE_IMPL, m.out());
    if hr < 0 {
        Err(hr)
    } else {
        Ok(m)
    }
}

unsafe fn set_props_json(props: *mut c_void, msg: *mut c_void) -> HRESULT {
    let mut w = Bstr::null();
    let hr = (vt::<JsonObjectVtbl>(props).stringify)(props, w.out());
    if hr < 0 {
        return hr;
    }
    (vt::<MessageImplVtbl>(msg).set_json_body_w)(msg, w.0)
}

unsafe fn csv(s: P) -> &'static ClientSessionVtbl {
    vt::<ClientSessionVtbl>(s)
}

unsafe fn send_cancel(session: P, exchange: i32, offset: u64) {
    let m = match message() {
        Ok(m) => m,
        Err(_) => return,
    };
    let mut props = Com::null();
    if XbtpCreateJsonObject(props.out()) < 0 {
        return;
    }
    set_u64(props.0, "FileOffset", offset);
    set_u32(props.0, "CancelTransfer", 1);
    if set_props_json(props.0, m.0) < 0 {
        return;
    }
    (csv(session).send_more_message)(session, exchange, m.0);
}

pub unsafe fn legacy_send_file(session: P, src: &str, dst: &str, flags: u32, max_size: u64, info: &FileInfo, notify: &mut Notify) -> HRESULT {
    if session.is_null() {
        return E_POINTER;
    }
    let su: Vec<u16> = src.encode_utf16().collect();
    match path_type(&su) {
        Ok((PATH_APPLICATION, _)) => return HR_FILE_NOT_FOUND,
        Ok(_) => {}
        Err(e) => return e,
    }
    let req = TransferRequest {
        src: match Bstr::from_str(src) {
            Ok(b) => b,
            Err(e) => return e,
        },
        dst: match Bstr::from_str(dst) {
            Ok(b) => b,
            Err(e) => return e,
        },
        flags,
        info: FileInfo { ..*info },
    };
    let mut r = FileReader::new(req);
    let mut hr = r.start();
    let mut exchange = 0i32;
    let mut begun = false;
    if hr >= 0 {
        r.chunk = optimal_chunks(max_size).1;
        hr = notify.progress(src, dst, r.req.info.size, 0);
    }
    if hr >= 0 {
        let size = r.req.info.size;
        let mut buf = vec![0u8; r.chunk as usize];
        let mut first = true;
        hr = loop {
            let n = (size - r.offset).min(r.chunk as u64) as u32;
            let at = r.offset;
            let done = match r.stream.read_at(&mut buf[..n as usize], at) {
                Ok(d) => d,
                Err(e) => break e,
            };
            if done != n {
                break HR_IO_DEVICE;
            }
            let m = match message() {
                Ok(m) => m,
                Err(e) => break e,
            };
            let h = (vt::<MessageImplVtbl>(m.0).set_binary_body_copy)(m.0, n, if n == 0 { core::ptr::null() } else { buf.as_ptr() });
            if h < 0 {
                break h;
            }
            let h = r.chunk_json(first, at, m.0);
            if h < 0 {
                break h;
            }
            let h = if first { (csv(session).send_begin_message)(session, OP_SEND_FILE, m.0, &mut exchange) } else { (csv(session).send_more_message)(session, exchange, m.0) };
            if h < 0 {
                break h;
            }
            begun = true;
            first = false;
            r.offset += n as u64;
            if r.offset > size {
                break E_UNEXPECTED;
            }
            let h = notify.progress(src, dst, size, r.offset);
            if h < 0 {
                if r.offset < size {
                    send_cancel(session, exchange, r.offset);
                }
                break h;
            }
            if r.offset >= size {
                break S_OK;
            }
        };
    }
    if begun {
        let mut resp = MsgResponse::default();
        let e = receive_end_message(session, XTF_RECEIVE_RECOMMENDED_TIMEOUT_MS, exchange, &mut resp);
        if hr >= 0 {
            hr = if e < 0 { e } else { resp.result };
        }
    }
    if r.stream.is_open() {
        let c = r.stream.close_with_retry(false);
        if hr >= 0 && c < 0 {
            hr = c;
        }
    }
    hr
}

pub unsafe fn legacy_receive_file(session: P, src: &str, dst: &str, flags: u32, notify: &mut Notify) -> HRESULT {
    if session.is_null() {
        return E_POINTER;
    }
    let du: Vec<u16> = dst.encode_utf16().collect();
    match path_type(&du) {
        Ok((PATH_APPLICATION, _)) => return HR_FILE_NOT_FOUND,
        Ok(_) => {}
        Err(e) => return e,
    }
    let req = TransferRequest {
        src: match Bstr::from_str(src) {
            Ok(b) => b,
            Err(e) => return e,
        },
        dst: match Bstr::from_str(dst) {
            Ok(b) => b,
            Err(e) => return e,
        },
        flags,
        info: FileInfo::default(),
    };
    let mut w = FileWriter::new(req);
    let mut exchange = 0i32;
    let mut hr = w.start();
    let mut begun = false;
    if hr >= 0 {
        hr = 'send: {
            let m = match message() {
                Ok(m) => m,
                Err(e) => break 'send e,
            };
            let mut props = Com::null();
            let h = XbtpCreateJsonObject(props.out());
            if h < 0 {
                break 'send h;
            }
            let h = set_string(props.0, "SrcFileName", &w.req.src);
            if h < 0 {
                break 'send h;
            }
            let h = set_string(props.0, "DstFileName", &w.req.dst);
            if h < 0 {
                break 'send h;
            }
            let h = set_u32(props.0, "Flags", flags);
            if h < 0 {
                break 'send h;
            }
            let h = set_props_json(props.0, m.0);
            if h < 0 {
                break 'send h;
            }
            let h = (csv(session).send_begin_message)(session, OP_RECV_FILE, m.0, &mut exchange);
            if h >= 0 {
                begun = true;
            }
            h
        };
    }
    if hr >= 0 {
        hr = notify.progress(src, dst, 0, 0);
    }
    let mut ended = false;
    if hr >= 0 {
        let mut first = true;
        hr = loop {
            let mut ty = 0i16;
            let mut status = 0i32;
            let mut m = Com::null();
            let h = (csv(session).receive_message)(session, LEGACY_RECEIVE_TIMEOUT_MS, exchange, &mut ty, &mut status, m.out());
            if h == E_PENDING {
                continue;
            }
            if h < 0 {
                break h;
            }
            if status != 0 {
                break transport_status(status);
            }
            if ty == XBTP_MESSAGE_TYPE_END {
                ended = true;
                let mut resp = MsgResponse::default();
                let d = deserialize(&mut resp, m.0);
                if d < 0 {
                    break d;
                }
                if resp.result < 0 {
                    break resp.result;
                }
                if first || w.offset < w.req.info.size {
                    break E_UNEXPECTED;
                }
                break S_OK;
            }
            if ty != XBTP_MESSAGE_TYPE_MORE {
                break (UNEXPECTED_MESSAGE_BASE | (ty as u16 as u32 & 0xff)) as i32;
            }
            if first {
                let mut props = Com::null();
                let d = deserialize_json_body(m.0, &mut props);
                if d < 0 {
                    break d;
                }
                let d = crate::find::get_file_info_properties(props.0, &mut w.req.info);
                if d < 0 {
                    break d;
                }
            }
            first = false;
            let (off, n) = match w.begin_write(m.0) {
                Ok(v) => v,
                Err(e) => break e,
            };
            if n == u32::MAX {
                break HR_IO_DEVICE;
            }
            let size = w.req.info.size;
            if off + n as u64 > size {
                break E_UNEXPECTED;
            }
            let p = notify.progress(src, dst, size, off + n as u64);
            if p < 0 {
                break p;
            }
            if w.offset >= size {
                break S_OK;
            }
        };
    }
    if begun && !ended && hr >= 0 {
        let mut resp = MsgResponse::default();
        let e = receive_end_message(session, LEGACY_RECEIVE_TIMEOUT_MS, exchange, &mut resp);
        hr = if e < 0 { e } else { resp.result };
    }
    let f = w.finish(hr);
    if hr >= 0 && f < 0 {
        f
    } else {
        hr
    }
}

#[allow(clippy::too_many_arguments)]
unsafe fn copy_legacy(c: &FileIOClient, found: &Found, dst: &str, dst_remote: bool, dst_directory: bool, flags: u32, notify: &mut Notify) -> HRESULT {
    let max_size = found.entries.iter().map(|e| e.info.size).max().unwrap_or(0);
    let session = c.session_a();
    for e in &found.entries {
        if e.info.attributes & FILE_ATTRIBUTE_DIRECTORY != 0 {
            continue;
        }
        let dest = if dst_directory {
            match relative_file_name(&found.root, &e.name) {
                Ok(rel) => append_path(dst, Some(&rel)),
                Err(err) => return err,
            }
        } else {
            dst.to_string()
        };
        let hr = notify.start(&found.root, &found.search, &e.name, &e.info, &dest);
        if hr < 0 {
            return hr;
        }
        let hr = if dst_remote { legacy_send_file(session.0, &e.name, &dest, flags, max_size, &e.info, notify) } else { legacy_receive_file(session.0, &e.name, &dest, flags, notify) };
        if hr < 0 {
            return hr;
        }
        let hr = notify.end(&found.root, &found.search, &e.name, &e.info, &dest, hr);
        if hr < 0 {
            return hr;
        }
    }
    S_OK
}

#[cfg(fileio_b)]
mod direct {
    use super::*;
    use crate::fileio_b::xfer::{files_to_messages, messages_to_files, Outgoing};
    use crate::fileio_b::{set_number, Channel, Timer};

    pub struct ClientChannel {
        pub session: P,
        pub op: u32,
        pub exchange: i32,
        pub begun: bool,
    }

    impl Channel for ClientChannel {
        unsafe fn send(&mut self, msg: *mut c_void) -> HRESULT {
            if self.session.is_null() {
                return crate::fileio_b::HR_NO_SESSION;
            }
            if !self.begun {
                let hr = (csv(self.session).send_begin_message)(self.session, self.op, msg, &mut self.exchange);
                if hr >= 0 {
                    self.begun = true;
                }
                return hr;
            }
            (csv(self.session).send_more_message)(self.session, self.exchange, msg)
        }
        unsafe fn receive_more(&mut self, timeout: u32) -> Result<Com, HRESULT> {
            if self.session.is_null() {
                return Err(crate::fileio_b::HR_NO_SESSION);
            }
            let mut m = Com::null();
            let hr = (csv(self.session).receive_more_message)(self.session, timeout, self.exchange, m.out());
            if hr < 0 {
                Err(hr)
            } else {
                Ok(m)
            }
        }
    }

    unsafe fn receive_end(ch: &ClientChannel, props: &mut Com) -> HRESULT {
        let mut status = 0i32;
        let mut m = Com::null();
        let hr = (csv(ch.session).receive_end_message)(ch.session, DIRECT_RECEIVE_TIMEOUT_MS, ch.exchange, &mut status, m.out());
        if hr < 0 {
            return hr;
        }
        if status != 0 {
            return transport_status(status);
        }
        let hr = deserialize_json_body(m.0, props);
        if hr < 0 {
            return hr;
        }
        let mut result = 0i32;
        let hr = get_i32(props.0, "Result", &mut result);
        if hr < 0 {
            return hr;
        }
        result
    }

    const PERF_SLOTS: [(&str, usize); 12] = [
        ("ReadCpu", 0x18),
        ("WriteCpu", 0x78),
        ("ReadFileScan", 0x10),
        ("WriteFileScan", 0x70),
        ("ReadNetwork", 0x88),
        ("WriteNetwork", 0x28),
        ("ReadFileMeta", 0x38),
        ("WriteFileMeta", 0x98),
        ("ReadFile", 0x48),
        ("WriteFile", 0xa8),
        ("ReadDiskWait", 0x58),
        ("WriteDiskWait", 0xb8),
    ];

    unsafe fn extract_json_body(timer: &mut Timer, props: *mut c_void) {
        if props.is_null() {
            return;
        }
        for (name, _) in PERF_SLOTS {
            let mut v = 0f64;
            if crate::fileio_b::get_number(props, name, &mut v) >= 0 {
                timer.mark_plus(name, v);
            }
        }
    }

    unsafe fn put(perf: *mut u8, off: usize, v: f64) {
        core::ptr::write_unaligned(perf.add(off) as *mut f64, v);
    }

    unsafe fn get(perf: *mut u8, off: usize) -> f64 {
        core::ptr::read_unaligned(perf.add(off) as *const f64)
    }

    unsafe fn deserialize_perf(timer: &Timer, perf: *mut u8) {
        for (name, off) in PERF_SLOTS {
            if let Some(v) = timer.get(name) {
                put(perf, off, v);
            }
        }
        for base in [0x18usize, 0x78] {
            let total: f64 = (0..5).map(|i| get(perf, base + i * 0x10)).sum::<f64>() / 100.0;
            if total > 0.0 {
                for i in 0..5 {
                    let v = get(perf, base + i * 0x10) / total;
                    put(perf, base + 8 + i * 0x10, v);
                }
            }
        }
    }

    pub unsafe fn reset_perf(perf: P) {
        if perf.is_null() {
            return;
        }
        let p = perf as *mut u8;
        core::ptr::write_bytes(p, 0, PERF_SIZE);
        *p.add(PERF_READ_PENDING) = 1;
        *p.add(PERF_WRITE_PENDING) = 1;
    }

    enum TokenQuery {
        OpenFailed,
        LocalFailed(HRESULT),
        TransportFailed,
        Result(HRESULT),
    }

    unsafe fn last_error_token(c: &FileIOClient, token: u64) -> TokenQuery {
        let (addr, port, redirect) = c.connection();
        let mut s = Com::null();
        if open_vm_session(addr.0, port, &mut s, redirect) < 0 {
            return TokenQuery::OpenFailed;
        }
        let mut props = Com::null();
        let hr = XbtpCreateJsonObject(props.out());
        if hr < 0 {
            return TokenQuery::LocalFailed(hr);
        }
        let hr = set_u64(props.0, "LastErrorToken", token);
        if hr < 0 {
            return TokenQuery::LocalFailed(hr);
        }
        let m = match message() {
            Ok(m) => m,
            Err(e) => return TokenQuery::LocalFailed(e),
        };
        let hr = set_props_json(props.0, m.0);
        if hr < 0 {
            return TokenQuery::LocalFailed(hr);
        }
        let mut exchange = 0i32;
        if (csv(s.0).send_begin_message)(s.0, OP_LAST_DIRECT_PROTOCOL_ERROR, m.0, &mut exchange) < 0 {
            return TokenQuery::TransportFailed;
        }
        let mut resp = MsgResponse::default();
        if receive_end_message(s.0, DIRECT_RECEIVE_TIMEOUT_MS, exchange, &mut resp) < 0 {
            return TokenQuery::TransportFailed;
        }
        TokenQuery::Result(resp.result)
    }

    fn map_direct_error(hr: HRESULT) -> HRESULT {
        if hr == HR_ACCESS_DENIED {
            HR_DIRECT_ACCESS_DENIED
        } else if hr == HR_SHARING_VIOLATION {
            HR_DIRECT_SHARING_VIOLATION
        } else {
            hr
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub unsafe fn copy_direct(c: &FileIOClient, level: u32, found: &Found, pattern: &str, include: u32, exclude: u32, recursion: u32, dst: &str, dst_remote: bool, dst_directory: bool, flags: u32, perf: P, notify: &mut Notify) -> HRESULT {
        reset_perf(perf);
        let session = c.session_a();
        let mut timer = Timer::new();
        let mut count = 0u32;
        let no_json = level == 2;
        let mut remote = Com::null();
        if dst_remote {
            let mut ch = ClientChannel { session: session.0, op: if no_json { OP_TRANSFER_TO_CONSOLE_NO_JSON } else { OP_TRANSFER_TO_CONSOLE }, exchange: 0, begun: false };
            let token = &ch as *const ClientChannel as u64;
            let files: Vec<FoundFile> = found.entries.iter().map(|e| FoundFile { file_name: e.name.clone(), info: FileInfo { ..e.info } }).collect();
            let out = Outgoing { flags, no_json, token, root: &found.root, pattern: &found.search };
            let dst_u: Vec<u16> = dst.encode_utf16().collect();
            let mut hr = files_to_messages(&mut ch, &found.root, &files, &dst_u, dst_directory, &out, &mut timer, &mut count, Some(notify));
            if ch.begun && hr >= 0 {
                hr = receive_end(&ch, &mut remote);
            }
            if hr < 0 {
                return match last_error_token(c, token) {
                    TokenQuery::OpenFailed => hr,
                    TokenQuery::LocalFailed(e) => e,
                    TokenQuery::Result(r) if r < 0 => r,
                    TokenQuery::TransportFailed | TokenQuery::Result(_) => map_direct_error(hr),
                };
            }
            if !perf.is_null() {
                *(perf as *mut u8).add(PERF_WRITE_PENDING) = 0;
                extract_json_body(&mut timer, remote.0);
                deserialize_perf(&timer, perf as *mut u8);
            }
            return hr;
        }
        let mut ch = ClientChannel { session: session.0, op: if no_json { OP_TRANSFER_FROM_CONSOLE_NO_JSON } else { OP_TRANSFER_FROM_CONSOLE }, exchange: 0, begun: false };
        let hr = 'req: {
            let m = match message() {
                Ok(m) => m,
                Err(e) => break 'req e,
            };
            let mut props = Com::null();
            let h = XbtpCreateJsonObject(props.out());
            if h < 0 {
                break 'req h;
            }
            let steps: [&dyn Fn() -> HRESULT; 7] = [
                &|| set_string(props.0, "SearchPattern", &Bstr::from_str(pattern).unwrap_or_else(|_| Bstr::null())),
                &|| set_u32(props.0, "IncludeFileAttributes", include),
                &|| set_u32(props.0, "ExcludeFileAttributes", exclude),
                &|| set_u32(props.0, "RecursionLevels", recursion),
                &|| set_u32(props.0, "Flags", flags),
                &|| set_string(props.0, "DstFileName", &Bstr::from_str(dst).unwrap_or_else(|_| Bstr::null())),
                &|| set_number(props.0, "IsDestinationDirectory", if dst_directory { 1.0 } else { 0.0 }),
            ];
            for s in steps {
                let h = s();
                if h < 0 {
                    break 'req h;
                }
            }
            let h = set_props_json(props.0, m.0);
            if h < 0 {
                break 'req h;
            }
            ch.send(m.0)
        };
        if hr < 0 {
            return hr;
        }
        let mut hr = messages_to_files(&mut ch, null_mut(), flags, &mut timer, &mut count, no_json, Some(notify));
        let r = receive_end(&ch, &mut remote);
        if r < 0 && (hr >= 0 || !remote.0.is_null()) {
            hr = r;
        }
        if hr >= 0 && !perf.is_null() {
            *(perf as *mut u8).add(PERF_READ_PENDING) = 0;
            extract_json_body(&mut timer, remote.0);
            deserialize_perf(&timer, perf as *mut u8);
        }
        hr
    }
}

#[repr(C)]
pub struct FileListVtbl {
    pub base: UnknownVtbl,
    pub slot3: usize,
    pub add: unsafe extern "system" fn(P, *const XtfFileInfo, P) -> HRESULT,
    pub slot5: usize,
    pub slot6: usize,
    pub max_file_size: unsafe extern "system" fn(P) -> u64,
    pub slots8_11: [usize; 4],
}

#[repr(C)]
pub struct FileListInnerVtbl {
    pub slots0_5: [usize; 6],
    pub count: unsafe extern "system" fn(P) -> u32,
    pub slots7_25: [usize; 19],
    pub next: unsafe extern "system" fn(P, P) -> P,
    pub slots27_29: [usize; 3],
}

pub const FILE_LIST_INNER_OFFSET: usize = 0x10;
pub const FILE_LIST_ENTRY_INFO_OFFSET: usize = 0x60;

pub unsafe fn read_file_list(list: P, pattern: &str) -> Result<Found, HRESULT> {
    let inner = (list as *mut u8).add(FILE_LIST_INNER_OFFSET) as P;
    let iv = vt::<FileListInnerVtbl>(inner);
    if (iv.count)(inner) == 0 {
        return Err(HR_FILE_NOT_FOUND);
    }
    let mut entries = Vec::new();
    let mut e = (iv.next)(inner, null_mut());
    while !e.is_null() {
        let info = &*((e as *mut u8).add(FILE_LIST_ENTRY_INFO_OFFSET) as *const XtfFileInfo);
        entries.push(Entry {
            name: String::from_utf16_lossy(&units(info.file_name)),
            info: FileInfo { attributes: info.attributes, creation: info.creation, access: info.access, write: info.write, size: info.size },
        });
        e = (iv.next)(inner, e);
    }
    Ok(Found { root: pattern.to_string(), search: pattern.to_string(), entries })
}

#[allow(clippy::too_many_arguments)]
pub unsafe fn copy_files_internal(c: &FileIOClient, pattern: W, include: u32, exclude: u32, recursion: u32, dst: W, flags: u32, perf: P, callback: P) -> HRESULT {
    copy_files_with_list(c, null_mut(), pattern, include, exclude, recursion, dst, flags, perf, callback)
}

#[allow(clippy::too_many_arguments)]
pub unsafe fn copy_files_with_list(c: &FileIOClient, list: P, pattern: W, include: u32, exclude: u32, recursion: u32, dst: W, flags: u32, perf: P, callback: P) -> HRESULT {
    if pattern.is_null() || dst.is_null() {
        return E_INVALIDARG;
    }
    let listed = if list.is_null() {
        None
    } else {
        match read_file_list(list, &String::from_utf16_lossy(&units(pattern))) {
            Ok(f) => Some(f),
            Err(e) => return e,
        }
    };
    let pu = units(pattern);
    let du = units(dst);
    let src = match side_of(&pu) {
        Ok(s) => s,
        Err(e) => return e,
    };
    let dsts = match side_of(&du) {
        Ok(s) => s,
        Err(e) => return e,
    };
    if src.remote == dsts.remote {
        return E_INVALIDARG;
    }
    let app: Option<W> = if src.kind == PATH_APPLICATION {
        Some(pattern)
    } else if dsts.kind == PATH_APPLICATION {
        Some(dst)
    } else {
        None
    };
    if let Some(a) = app {
        let hr = c.start_application_operations(a);
        if hr < 0 {
            return hr;
        }
    }
    let hr = copy_files_body(c, listed, &src, &dsts, include, exclude, recursion, flags, perf, callback);
    if let Some(a) = app {
        c.end_application_operations(a);
    }
    hr
}

#[allow(clippy::too_many_arguments)]
unsafe fn copy_files_body(c: &FileIOClient, listed: Option<Found>, src: &Side, dsts: &Side, include: u32, exclude: u32, recursion: u32, flags: u32, perf: P, callback: P) -> HRESULT {
    let level = direct_level(c);
    let has_list = listed.is_some();
    let skip_find = has_list || (src.remote && level != 0);
    let found = if let Some(f) = listed {
        f
    } else if skip_find {
        Found { root: String::new(), search: String::new(), entries: Vec::new() }
    } else {
        let r = if src.remote { remote_find(c, &src.path, include & !FILE_ATTRIBUTE_DIRECTORY, exclude | FILE_ATTRIBUTE_DIRECTORY, recursion) } else { local_find(&src.path, include & !FILE_ATTRIBUTE_DIRECTORY, exclude | FILE_ATTRIBUTE_DIRECTORY, recursion) };
        match r {
            Ok(f) => f,
            Err(e) => return e,
        }
    };
    if (!skip_find || has_list) && found.entries.is_empty() {
        return HR_FILE_NOT_FOUND;
    }
    let count = found.entries.len();
    let dst_directory = if dsts.path.ends_with('\\') {
        true
    } else {
        let (hr, attrs) = attributes_of(c, dsts.remote, &dsts.path);
        if hr < 0 {
            return hr;
        }
        let multi = if src.remote && !has_list { has_wildcard(&src.path) } else { count != 1 };
        if multi {
            attrs == INVALID_FILE_ATTRIBUTES || attrs & FILE_ATTRIBUTE_DIRECTORY != 0
        } else if attrs != INVALID_FILE_ATTRIBUTES {
            attrs & FILE_ATTRIBUTE_DIRECTORY != 0
        } else {
            let (shr, sattrs) = attributes_of(c, src.remote, &src.path);
            if shr < 0 {
                return shr;
            }
            sattrs != INVALID_FILE_ATTRIBUTES && sattrs & FILE_ATTRIBUTE_DIRECTORY != 0
        }
    };
    if !dst_directory && count > 1 {
        return E_INVALIDARG;
    }
    let mut notify = Notify { callback };
    if level != 0 {
        #[cfg(fileio_b)]
        {
            let pat = if found.search.is_empty() { src.path.clone() } else { found.search.clone() };
            return direct::copy_direct(c, level, &found, &pat, include, exclude, recursion, &dsts.path, dsts.remote, dst_directory, flags, perf, &mut notify);
        }
    }
    let _ = perf;
    copy_legacy(c, &found, &dsts.path, dsts.remote, dst_directory, flags, &mut notify)
}

unsafe fn direct_level(c: &FileIOClient) -> u32 {
    #[cfg(fileio_b)]
    {
        let l = c.direct_protocol_level();
        if l >= 2 {
            return 2;
        }
        l
    }
    #[cfg(not(fileio_b))]
    {
        let _ = c;
        0
    }
}

pub unsafe extern "system" fn c_copy_files(this: P, pattern: W, include: u32, exclude: u32, recursion: u32, dst: W, flags: u32, callback: P) -> HRESULT {
    copy_files_internal(client_from_main(this), pattern, include, exclude, recursion, dst, flags, null_mut(), callback)
}

pub unsafe extern "system" fn i2_copy_files(this: P, root: W, files: *const W, count: u32, dst: W, flags: u32, callback: P) -> HRESULT {
    let c = client_from_i2(this);
    if root.is_null() || dst.is_null() || (files.is_null() && count != 0) {
        return E_INVALIDARG;
    }
    let ru = units(root);
    let du = units(dst);
    let rk = match path_type(&ru) {
        Ok((k, _)) => k,
        Err(e) => return e,
    };
    let dk = match path_type(&du) {
        Ok((k, _)) => k,
        Err(e) => return e,
    };
    let app: Option<W> = if rk == PATH_APPLICATION {
        Some(root)
    } else if dk == PATH_APPLICATION {
        Some(dst)
    } else {
        None
    };
    if let Some(a) = app {
        let hr = c.start_application_operations(a);
        if hr < 0 {
            return hr;
        }
    }
    let rs = String::from_utf16_lossy(&ru);
    let mut hr = S_OK;
    for i in 0..count as usize {
        let f = *files.add(i);
        if f.is_null() {
            hr = E_INVALIDARG;
            break;
        }
        let part = String::from_utf16_lossy(&units(f));
        let pattern = wide(&append_path(&rs, Some(&part)));
        hr = (CLIENT_VTBL.copy_files)(FileIOClient::as_unknown(c as *const FileIOClient as *mut FileIOClient), pattern.as_ptr(), 0, 0, 0, dst, flags, callback);
        if hr < 0 {
            break;
        }
    }
    if let Some(a) = app {
        c.end_application_operations(a);
    }
    hr
}

pub unsafe extern "system" fn i3_copy_files_direct_list(this: P, list: P, reserved: u32, pattern: W, dst: W, flags: u32, perf: P, callback: P) -> HRESULT {
    if list.is_null() || reserved != 0 {
        return E_INVALIDARG;
    }
    copy_files_with_list(client_from_i3(this), list, pattern, 0, 0, 0, dst, flags, perf, callback)
}

pub unsafe extern "system" fn i3_copy_files_perf(this: P, pattern: W, include: u32, exclude: u32, recursion: u32, dst: W, flags: u32, perf: P, callback: P) -> HRESULT {
    copy_files_internal(client_from_i3(this), pattern, include, exclude, recursion, dst, flags, perf, callback)
}


pub const E_DIRECT_PROTOCOL_REQUIRED: HRESULT = 0x8c11_01fbu32 as i32;
pub const OP_QUERY_DIRTY_FILE_BLOCK: u32 = 0x1a;
pub const INCREMENTAL_BLOCK: usize = 0x80000;
pub const INCREMENTAL_STATS_SIZE: usize = 0xa8;

#[cfg(fileio_b)]
pub mod incremental {
    use super::*;
    use crate::file::Win32FindDataW;
    use crate::fileio_b::diff::{probe_symlink_target, DeltaTable, DELTA_ENTRY_SIZE, FILE_ATTRIBUTE_REPARSE_POINT};
    use crate::fileio_b::{get_number, set_number, Timer};
    use std::collections::VecDeque;

    #[allow(clashing_extern_declarations)]
    #[link(name = "kernel32")]
    extern "system" {
        fn FindNextFileW(h: isize, data: *mut Win32FindDataW) -> i32;
    }

    pub struct LocalFile {
        pub name: String,
        pub info: FileInfo,
        pub index: usize,
    }

    fn relative_units(full: &str, root_len: usize) -> Vec<u16> {
        let u: Vec<u16> = full.encode_utf16().collect();
        u[root_len.min(u.len())..].to_vec()
    }

    fn name_of(d: &Win32FindDataW) -> String {
        let n = d.file_name.iter().position(|&c| c == 0).unwrap_or(d.file_name.len());
        String::from_utf16_lossy(&d.file_name[..n])
    }

    fn join64(v: [u32; 2]) -> u64 {
        ((v[1] as u64) << 32) | v[0] as u64
    }

    #[repr(C)]
    pub struct StatusCallbackVtbl {
        pub base: UnknownVtbl,
        pub on_update_status: unsafe extern "system" fn(P, HRESULT, *const u16) -> HRESULT,
    }

    pub const E_XTFAPP_INVALID_SYMLINK: HRESULT = 0x8d16_0194u32 as i32;
    pub const E_XTFAPP_PATH_TOO_LONG: HRESULT = 0x8d16_0199u32 as i32;
    const SYMLINK_MESSAGE_CAPACITY: usize = 0x208;
    const LONG_PATH_LIMIT: usize = 0x104;
    const LONG_PATH_PREFIX: &str = "Source file path detected which is longer than 260 characters, which could cause issues while testing or building a package from this content: ";

    unsafe fn update_status(status: P, code: HRESULT, message: &[u16]) -> HRESULT {
        let mut m = message.to_vec();
        m.push(0);
        (vt::<StatusCallbackVtbl>(status).on_update_status)(status, code, m.as_ptr())
    }

    fn symlink_message(child: &str, attributes: u32) -> Vec<u16> {
        let kind = if attributes & FILE_ATTRIBUTE_DIRECTORY != 0 { "directory" } else { "file" };
        let m: Vec<u16> = format!("Invalid symlink to {kind} at \"{child}\"").encode_utf16().collect();
        if m.len() >= SYMLINK_MESSAGE_CAPACITY {
            Vec::new()
        } else {
            m
        }
    }

    enum Entry {
        Next,
        Fail(HRESULT),
        Keep,
    }

    unsafe fn resolve_reparse(child: &str, d: &mut Win32FindDataW, status: P) -> Entry {
        if d.attributes & FILE_ATTRIBUTE_REPARSE_POINT == 0 {
            return Entry::Keep;
        }
        match probe_symlink_target(child) {
            Ok(p) => {
                d.attributes = p.attributes;
                d.creation = p.creation;
                d.access = p.access;
                d.write = p.write;
                d.size_high = p.size_high;
                d.size_low = p.size_low;
                Entry::Keep
            }
            Err(hr) if hr == HR_FILE_NOT_FOUND || hr == HR_ACCESS_DENIED => {
                let a = file::get_attributes_raw(child);
                if a == INVALID_FILE_ATTRIBUTES || a & FILE_ATTRIBUTE_REPARSE_POINT == 0 || status.is_null() {
                    return Entry::Fail(hr);
                }
                let r = update_status(status, E_XTFAPP_INVALID_SYMLINK, &symlink_message(child, a));
                if r < 0 {
                    Entry::Fail(r)
                } else {
                    Entry::Next
                }
            }
            Err(hr) => Entry::Fail(hr),
        }
    }

    unsafe fn scan_entry(child: String, d: &mut Win32FindDataW, root_len: usize, status: P, table: &mut DeltaTable, files: &mut Vec<LocalFile>, queue: &mut VecDeque<String>, timer: &mut Timer) -> HRESULT {
        match resolve_reparse(&child, d, status) {
            Entry::Next => return S_OK,
            Entry::Fail(hr) => return hr,
            Entry::Keep => {}
        }
        if d.attributes & FILE_ATTRIBUTE_DIRECTORY != 0 {
            queue.push_back(child);
            return S_OK;
        }
        let units: Vec<u16> = child.encode_utf16().collect();
        if units.len() > LONG_PATH_LIMIT && !status.is_null() {
            let mut m: Vec<u16> = LONG_PATH_PREFIX.encode_utf16().collect();
            m.extend_from_slice(&units);
            let r = update_status(status, E_XTFAPP_PATH_TOO_LONG, &m);
            if r < 0 {
                return r;
            }
        }
        timer.mark("Cpu");
        let rel = relative_units(&child, root_len);
        timer.mark("HashPrep");
        let index = table.add(&rel);
        timer.mark("Hash");
        let write = join64(d.write);
        table.entries[index].time = write;
        let info = FileInfo { attributes: d.attributes, creation: join64(d.creation), access: join64(d.access), write, size: ((d.size_high as u64) << 32) | d.size_low as u64 };
        files.push(LocalFile { name: child, info, index });
        timer.mark("HashStore");
        S_OK
    }

    #[allow(clippy::too_many_arguments)]
    pub unsafe fn enumerate_local_delta_root(root: &str, table: &mut DeltaTable, files: &mut Vec<LocalFile>, timer: &mut Timer, status: P) -> HRESULT {
        let full = match full_path_name(root) {
            Ok(f) => f,
            Err(e) => return e,
        };
        if let Err(e) = file::get_file_attributes(&full) {
            return e;
        }
        let full = full.trim_end_matches('\\').to_string();
        let root_len = full.encode_utf16().count();
        let mut queue: VecDeque<String> = VecDeque::new();
        queue.push_back(full);
        while let Some(dir) = queue.pop_front() {
            timer.mark("Cpu");
            table.add(&relative_units(&dir, root_len));
            timer.mark("HashStore");
            let mut d: Win32FindDataW = unsafe { core::mem::zeroed() };
            let h = file::find_first_search(&format!("{dir}\\*"), &mut d);
            if h == -1 {
                continue;
            }
            timer.mark("FileScan");
            let hr = loop {
                let name = name_of(&d);
                if name != "." && name != ".." {
                    let hr = scan_entry(format!("{dir}\\{name}"), &mut d, root_len, status, table, files, &mut queue, timer);
                    if hr < 0 {
                        break hr;
                    }
                }
                if FindNextFileW(h, &mut d) == 0 {
                    break S_OK;
                }
            };
            file::find_close(h);
            if hr < 0 {
                return hr;
            }
        }
        S_OK
    }

    unsafe fn put(stats: *mut u8, off: usize, v: f64) {
        core::ptr::write_unaligned(stats.add(off) as *mut f64, v);
    }

    unsafe fn send_request(s: P, table: &DeltaTable, remote: W, delete_orphans: u32, correlation: W, timer: &mut Timer, exchange: &mut i32) -> HRESULT {
        let bytes = table.table_bytes();
        let m = match message() {
            Ok(m) => m,
            Err(e) => return e,
        };
        let mut body = Com::null();
        let h = XbtpCreateJsonObject(body.out());
        if h < 0 {
            return h;
        }
        let remote_b = match Bstr::from_str(&String::from_utf16_lossy(&units(remote))) {
            Ok(b) => b,
            Err(e) => return e,
        };
        let mut steps: Vec<HRESULT> = vec![
            set_u32(body.0, "FileCount", table.entries.len() as u32),
            set_u32(body.0, "CollisionTableSize", table.collisions.len() as u32),
            set_string(body.0, "DstFileName", &remote_b),
            set_number(body.0, "RemoveOrphans", if delete_orphans != 0 { 1.0 } else { 0.0 }),
        ];
        if !correlation.is_null() {
            match Bstr::from_str(&String::from_utf16_lossy(&units(correlation))) {
                Ok(g) => steps.push(set_string(body.0, "CorrelationGuid", &g)),
                Err(e) => steps.push(e),
            }
        }
        if let Some(&e) = steps.iter().find(|&&h| h < 0) {
            return e;
        }
        let first = bytes.len().min(INCREMENTAL_BLOCK);
        let h = (vt::<MessageImplVtbl>(m.0).set_binary_body_copy)(m.0, first as u32, bytes.as_ptr());
        if h < 0 {
            return h;
        }
        let h = set_props_json(body.0, m.0);
        if h < 0 {
            return h;
        }
        timer.mark("Cpu");
        let h = (csv(s).send_begin_message)(s, OP_QUERY_DIRTY_FILE_BLOCK, m.0, exchange);
        if h < 0 {
            return h;
        }
        timer.mark("Network");
        for chunk in [&bytes[first..], &table.collisions[..]] {
            for part in chunk.chunks(INCREMENTAL_BLOCK) {
                let mm = match message() {
                    Ok(m) => m,
                    Err(e) => return e,
                };
                timer.mark("Cpu");
                let h = (vt::<MessageImplVtbl>(mm.0).set_binary_body_copy)(mm.0, part.len() as u32, part.as_ptr());
                if h < 0 {
                    return h;
                }
                let h = (csv(s).send_more_message)(s, *exchange, mm.0);
                if h < 0 {
                    return h;
                }
                timer.mark("Network");
            }
        }
        S_OK
    }

    #[allow(clippy::too_many_arguments)]
    pub unsafe fn calculate_incremental_list(c: &FileIOClient, local: W, remote: W, delete_orphans: u32, stats: P, list: P, correlation: W, status: P) -> HRESULT {
        if local.is_null() || remote.is_null() || list.is_null() {
            return E_INVALIDARG;
        }
        if direct_level(c) == 0 {
            return E_DIRECT_PROTOCOL_REQUIRED;
        }
        if !stats.is_null() {
            *(stats as *mut u64) = 0;
        }
        let mut timer = Timer::new();
        let mut table = DeltaTable::default();
        let mut files: Vec<LocalFile> = Vec::new();
        let hr = enumerate_local_delta_root(&String::from_utf16_lossy(&units(local)), &mut table, &mut files, &mut timer, status);
        if hr < 0 {
            return hr;
        }
        let session = c.session_a();
        let s = session.0;
        if s.is_null() {
            return E_POINTER;
        }
        let mut exchange = 0i32;
        let hr = send_request(s, &table, remote, delete_orphans, correlation, &mut timer, &mut exchange);
        if hr < 0 {
            return hr;
        }
        let mut status = 0i32;
        let mut end = Com::null();
        let hr = (csv(s).receive_end_message)(s, u32::MAX, exchange, &mut status, end.out());
        if hr < 0 {
            return hr;
        }
        if status != 0 {
            return transport_status(status);
        }
        timer.mark("Network");
        let mut props = Com::null();
        let hr = deserialize_json_body(end.0, &mut props);
        if hr < 0 {
            return hr;
        }
        let mut result = 0i32;
        let hr = get_i32(props.0, "Result", &mut result);
        if hr < 0 {
            return hr;
        }
        if result < 0 {
            return result;
        }
        let mut n = 0u32;
        let mut p: *mut u8 = null_mut();
        let hr = (vt::<MessageVtbl>(end.0).get_binary_body)(end.0, &mut n, &mut p);
        if hr < 0 {
            return hr;
        }
        let raw: &[u8] = if p.is_null() { &[] } else { core::slice::from_raw_parts(p, n as usize) };
        let words: Vec<u64> = raw.chunks_exact(8).map(|w| u64::from_le_bytes(w.try_into().unwrap())).collect();
        let lv = vt::<FileListVtbl>(list);
        for f in &files {
            if table.entries[f.index].time == 0 {
                continue;
            }
            let dirty = words.get(f.index >> 6).is_some_and(|w| w & (1u64 << (f.index & 63)) != 0);
            if !dirty {
                continue;
            }
            let name = wide(&f.name);
            let info = xtf_info(&name, &f.info);
            (lv.add)(list, &info, null_mut());
        }
        timer.mark("Cpu");
        if !stats.is_null() {
            let st = stats as *mut u8;
            let remote_names = ["Cpu", "Network", "FileScan", "HashPrep", "Hash", "HashFind", "DeleteWait", "CollisionResolve"];
            for (i, name) in remote_names.iter().enumerate() {
                let mut v = 0f64;
                if get_number(props.0, name, &mut v) >= 0 {
                    put(st, 0x38 + i * 8, v);
                }
            }
            for (off, name) in [(0x98usize, "FilesDeleted"), (0xa0, "DirectoriesDeleted")] {
                let mut v = 0f64;
                if get_number(props.0, name, &mut v) >= 0 {
                    put(st, off, v);
                }
            }
            let local_names = ["Cpu", "Network", "FileScan", "HashPrep", "Hash", "HashStore", "CollisionStore"];
            for (i, name) in local_names.iter().enumerate() {
                put(st, i * 8, timer.get(name).unwrap_or(0.0));
            }
            put(st, 0x78, (table.entries.len() * DELTA_ENTRY_SIZE) as f64);
            put(st, 0x80, table.collision_count as f64);
            put(st, 0x88, table.collisions.len() as f64);
            put(st, 0x90, (words.len() * 8) as f64);
        }
        S_OK
    }
}

#[allow(clippy::too_many_arguments)]
pub unsafe extern "system" fn i3_calculate_incremental_list(this: P, local: W, remote: W, delete_orphans: u32, stats: P, list: P, correlation: W, status: P) -> HRESULT {
    let c = client_from_i3(this);
    #[cfg(fileio_b)]
    {
        incremental::calculate_incremental_list(c, local, remote, delete_orphans, stats, list, correlation, status)
    }
    #[cfg(not(fileio_b))]
    {
        let _ = (c, local, remote, delete_orphans, stats, list, correlation, status);
        E_DIRECT_PROTOCOL_REQUIRED
    }
}
