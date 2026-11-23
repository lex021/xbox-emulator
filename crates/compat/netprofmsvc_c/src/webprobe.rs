pub const AGENT: &str = "Microsoft NetworkListManager";
pub const WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY: u32 = 4;
pub const WINHTTP_FLAG_ASYNC: u32 = 0x1000_0000;
pub const NOTIFICATION_FLAGS: u32 = 0x017e_0c03;
pub const OPEN_REQUEST_FLAGS: u32 = 0x104;
pub const STATUS_HANDLE_CREATED: u32 = 0x400;
pub const STATUS_HANDLE_CLOSING: u32 = 0x800;
pub const STATUS_SENDREQUEST_COMPLETE: u32 = 0x40_0000;
pub const STATUS_HEADERS_AVAILABLE: u32 = 0x2_0000;
pub const STATUS_REQUEST_ERROR: u32 = 0x20_0000;
pub const API_SEND_REQUEST: usize = 5;
pub const ERROR_WINHTTP_NAME_NOT_RESOLVED: u32 = 12007;

pub fn hresult_from_win32(e: u32) -> i32 {
    if e == 0 { 0 } else { ((e & 0xffff) | 0x8007_0000) as i32 }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    pub host: String,
    pub port: u16,
    pub path: String,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AsyncResult {
    pub api: usize,
    pub error: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    RequestError(AsyncResult),
    HandleClosing,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Outcome {
    pub events: Vec<Event>,
    pub hr: i32,
    pub body: Vec<u8>,
}

pub fn host_is_local(host: &str) -> bool {
    let t = host.trim().trim_end_matches('.');
    t.eq_ignore_ascii_case("localhost") || t.parse::<std::net::IpAddr>().map(|ip| ip.is_loopback()).unwrap_or(false)
}

pub fn run_offline(req: &Request, handles: usize) -> Outcome {
    let error = if host_is_local(&req.host) || req.host.parse::<std::net::IpAddr>().is_ok() { 12029 } else { ERROR_WINHTTP_NAME_NOT_RESOLVED };
    let mut events = vec![Event::RequestError(AsyncResult { api: API_SEND_REQUEST, error })];
    events.extend(std::iter::repeat(Event::HandleClosing).take(handles));
    Outcome { events, hr: hresult_from_win32(error), body: Vec::new() }
}

pub fn has_internet(o: &Outcome) -> bool {
    o.hr >= 0 && !o.events.iter().any(|e| matches!(e, Event::RequestError(_)))
}
