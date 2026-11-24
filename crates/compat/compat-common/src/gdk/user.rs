use super::xasync::{self, Payload};
use super::{log, put_str, put_wstr, Guid, Iface, This, E_INVALIDARG, S_OK};
use core::ffi::c_void;
use super::events::Registry;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::OnceLock;

pub const CLSID: Guid = Guid::parse("01acd177-91f9-4763-a38e-ccbb55ce32e0");
pub const IID_V1: Guid = Guid::parse("eb9bf948-18dc-4d82-bbcc-40e0a809c4c0");
pub const IID_V2: Guid = Guid::parse("1bf2f8c5-d507-4e52-bb05-f726d0e71161");
pub const IID_GAMERTAG: Guid = Guid::parse("cef4fac0-7676-4a94-a119-4c43f9eb5b74");
pub const DEVICE: Guid = Guid::parse("7d824997-10dc-45ab-86b7-2737767c0bf1");
pub const DEVICE_V2: Guid = Guid::parse("0cc6a956-e7e1-4fdf-9341-9d5da649ebc8");

pub const LOCAL_ID: u64 = 1;

pub struct User {
    pub gamertag: String,
    pub xuid: u64,
    pub token: String,
}

pub fn user() -> &'static User {
    static U: OnceLock<User> = OnceLock::new();
    U.get_or_init(|| {
        let gamertag = crate::cfg("gamertag").filter(|g| !g.is_empty() && g.len() <= 32 && g.is_ascii()).unwrap_or_else(|| "XboxEmulator".into());
        let xuid = crate::cfg("xuid").and_then(|x| x.parse::<u64>().ok()).unwrap_or(2533274790395905);
        let token = crate::cfg("xbl_token").filter(|t| !t.is_empty()).unwrap_or_else(|| format!("XBL3.0 x=tombox;local-offline-user-{xuid}"));
        User { gamertag, xuid, token }
    })
}

static HANDLE_ANCHOR: u64 = 0x5842_4F58_5553_4552;

pub const HANDLE_ENV: &str = "TOMBOX_GDK_USER_HANDLE";

#[link(name = "kernel32")]
extern "system" {
    fn CreateMutexW(attr: *mut c_void, owner: i32, name: *const u16) -> *mut c_void;
    fn WaitForSingleObject(h: *mut c_void, ms: u32) -> u32;
    fn ReleaseMutex(h: *mut c_void) -> i32;
    fn CloseHandle(h: *mut c_void) -> i32;
    fn GetModuleHandleExW(flags: u32, name: *const u16, out: *mut *mut c_void) -> i32;
}

fn published(own: usize, force: bool) -> usize {
    let pid = std::process::id();
    let parse = || {
        let v = std::env::var(HANDLE_ENV).ok()?;
        let (p, a) = v.split_once(':')?;
        if p.parse::<u32>().ok()? != pid {
            return None;
        }
        usize::from_str_radix(a.trim_start_matches("0x"), 16).ok().filter(|&v| v != 0)
    };
    if !force {
        if let Some(v) = parse() {
            return v;
        }
    }
    let name: Vec<u16> = format!("Local\\TomBoxGdkUserHandle{pid}").encode_utf16().chain(Some(0)).collect();
    unsafe {
        let m = CreateMutexW(core::ptr::null_mut(), 0, name.as_ptr());
        if !m.is_null() {
            WaitForSingleObject(m, 5000);
        }
        let v = match parse().filter(|_| !force) {
            Some(v) => v,
            None => {
                let mut h: *mut c_void = core::ptr::null_mut();
                GetModuleHandleExW(0x5, &HANDLE_ANCHOR as *const u64 as *const u16, &mut h);
                std::env::set_var(HANDLE_ENV, format!("{pid}:{own:#x}"));
                own
            }
        };
        if !m.is_null() {
            ReleaseMutex(m);
            CloseHandle(m);
        }
        v
    }
}

static H: OnceLock<usize> = OnceLock::new();

pub fn handle() -> usize {
    *H.get_or_init(|| published(&HANDLE_ANCHOR as *const u64 as usize, false))
}

pub fn claim_handle() -> usize {
    *H.get_or_init(|| published(&HANDLE_ANCHOR as *const u64 as usize, true))
}

static OP_ADD: &[u8] = b"XUserAddAsync\0";
static OP_TOKEN: &[u8] = b"XUserGetTokenAndSignatureAsync\0";
static OP_TOKEN16: &[u8] = b"XUserGetTokenAndSignatureUtf16Async\0";
static OP_RESOLVE_PRIV: &[u8] = b"XUserResolvePrivilegeWithUiAsync\0";
static OP_RESOLVE_ISSUE: &[u8] = b"XUserResolveIssueWithUiAsync\0";
static OP_PICTURE: &[u8] = b"XUserGetGamerPictureAsync\0";

pub const E_GAMEUSER_DEFERRAL_NOT_AVAILABLE: i32 = 0x8924_5103u32 as i32;
pub const E_GAMEUSER_USER_NOT_FOUND: i32 = 0x8924_5104u32 as i32;

unsafe extern "system" fn duplicate_handle(_t: This, u: usize, out: *mut usize) -> i32 {
    if u == 0 || out.is_null() {
        return E_INVALIDARG;
    }
    *out = u;
    S_OK
}

unsafe extern "system" fn close_handle(_t: This, _u: usize) {}

pub unsafe fn local_id_of(u: usize) -> Option<u64> {
    if u == 0 {
        return None;
    }
    if u == handle() || u == &HANDLE_ANCHOR as *const u64 as usize {
        return Some(LOCAL_ID);
    }
    let mut m: *mut c_void = core::ptr::null_mut();
    if u % 8 != 0 || GetModuleHandleExW(0x6, u as *const u16, &mut m) == 0 {
        return None;
    }
    (core::ptr::read(u as *const u64) == HANDLE_ANCHOR).then_some(LOCAL_ID)
}

unsafe extern "system" fn compare(_t: This, a: usize, b: usize) -> i32 {
    let ka = local_id_of(a).map(|i| (0usize, i as usize)).unwrap_or((1, a));
    let kb = local_id_of(b).map(|i| (0usize, i as usize)).unwrap_or((1, b));
    match ka.cmp(&kb) {
        core::cmp::Ordering::Equal => 0,
        core::cmp::Ordering::Less => -1,
        core::cmp::Ordering::Greater => 1,
    }
}

unsafe extern "system" fn get_max_users(_t: This, out: *mut u32) -> i32 {
    if out.is_null() {
        return E_INVALIDARG;
    }
    *out = 1;
    S_OK
}

unsafe extern "system" fn add_async(_t: This, options: u32, block: *mut c_void) -> i32 {
    let hr = xasync::run(block, OP_ADD, Payload::value(handle()));
    log("XUserAddAsync", &format!("options={options:#x} -> local user '{}' hr={hr:#010x}", user().gamertag));
    hr
}

unsafe extern "system" fn add_result(_t: This, block: *mut c_void, out: *mut usize) -> i32 {
    xasync::value(block, OP_ADD, out)
}

unsafe extern "system" fn get_local_id(_t: This, u: usize, out: *mut u64) -> i32 {
    if u == 0 || out.is_null() {
        return E_INVALIDARG;
    }
    *out = LOCAL_ID;
    S_OK
}

unsafe extern "system" fn find_by_local_id(_t: This, local: u64, out: *mut usize) -> i32 {
    if out.is_null() {
        return E_INVALIDARG;
    }
    if local != LOCAL_ID {
        return E_GAMEUSER_USER_NOT_FOUND;
    }
    *out = handle();
    S_OK
}

unsafe extern "system" fn get_id(_t: This, u: usize, out: *mut u64) -> i32 {
    if u == 0 || out.is_null() {
        return E_INVALIDARG;
    }
    *out = user().xuid;
    S_OK
}

unsafe extern "system" fn find_by_id(_t: This, xuid: u64, out: *mut usize) -> i32 {
    if out.is_null() {
        return E_INVALIDARG;
    }
    if xuid != user().xuid {
        return E_GAMEUSER_USER_NOT_FOUND;
    }
    *out = handle();
    S_OK
}

unsafe extern "system" fn get_state(_t: This, u: usize, out: *mut u32) -> i32 {
    if u == 0 || out.is_null() {
        return E_INVALIDARG;
    }
    *out = STATE.load(Ordering::Acquire);
    S_OK
}

unsafe extern "system" fn check_privilege(_t: This, u: usize, _options: u32, privilege: u32, has: *mut bool, reason: *mut u32) -> i32 {
    if u == 0 || has.is_null() {
        return E_INVALIDARG;
    }
    *has = true;
    if !reason.is_null() {
        *reason = 0;
    }
    log("XUserCheckPrivilege", &format!("privilege={privilege} -> granted"));
    S_OK
}

unsafe extern "system" fn resolve_privilege_async(_t: This, _u: usize, _options: u32, privilege: u32, block: *mut c_void) -> i32 {
    log("XUserResolvePrivilegeWithUiAsync", &format!("privilege={privilege} -> completed without UI"));
    xasync::run(block, OP_RESOLVE_PRIV, Payload::status(S_OK))
}

unsafe extern "system" fn async_status(_t: This, block: *mut c_void) -> i32 {
    xasync::status(block)
}

fn encode(s: &str, wide: bool) -> Vec<u8> {
    if wide {
        s.encode_utf16().chain(Some(0)).flat_map(|c| c.to_le_bytes()).collect()
    } else {
        s.bytes().chain(Some(0)).collect()
    }
}

pub fn token_payload(wide: bool, method: &str, url: &str, body: &[u8]) -> Payload {
    let t = &user().token;
    let sig = match super::xsign::sign(method, url, t, body) {
        Ok(s) => s,
        Err(hr) => return Payload::status(hr),
    };
    let tok = encode(t, wide);
    let sb = encode(&sig, wide);
    let unit = if wide { 2 } else { 1 };
    let mut b = Vec::with_capacity(32 + tok.len() + sb.len());
    b.extend_from_slice(&((tok.len() / unit) as u64).to_le_bytes());
    b.extend_from_slice(&((sb.len() / unit) as u64).to_le_bytes());
    b.extend_from_slice(&32u64.to_le_bytes());
    b.extend_from_slice(&((32 + tok.len()) as u64).to_le_bytes());
    b.extend_from_slice(&tok);
    b.extend_from_slice(&sb);
    Payload { hr: S_OK, bytes: b, fixups: vec![16, 24] }
}

unsafe fn body_slice<'a>(size: usize, body: *const c_void) -> &'a [u8] {
    if body.is_null() || size == 0 { &[] } else { core::slice::from_raw_parts(body as *const u8, size) }
}

#[allow(clippy::too_many_arguments)]
unsafe extern "system" fn token_async(
    _t: This, u: usize, options: u32, method: *const u8, url: *const u8, _hc: usize, _headers: *const c_void, bs: usize, body: *const c_void, block: *mut c_void,
) -> i32 {
    if u == 0 || url.is_null() {
        return E_INVALIDARG;
    }
    let (m, l) = (super::cstr(method).unwrap_or_default(), super::cstr(url).unwrap_or_default());
    let p = token_payload(false, &m, &l, body_slice(bs, body));
    log("XUserGetTokenAndSignatureAsync", &format!("options={options:#x} {m} {l} body={bs} -> token + signature hr={:#010x}", p.hr));
    xasync::run(block, OP_TOKEN, p)
}

#[allow(clippy::too_many_arguments)]
unsafe extern "system" fn token16_async(
    _t: This, u: usize, options: u32, method: *const u16, url: *const u16, _hc: usize, _headers: *const c_void, bs: usize, body: *const c_void, block: *mut c_void,
) -> i32 {
    if u == 0 || url.is_null() {
        return E_INVALIDARG;
    }
    let (m, l) = (super::wstr(method).unwrap_or_default(), super::wstr(url).unwrap_or_default());
    let p = token_payload(true, &m, &l, body_slice(bs, body));
    log("XUserGetTokenAndSignatureUtf16Async", &format!("options={options:#x} {m} {l} body={bs} -> token + signature hr={:#010x}", p.hr));
    xasync::run(block, OP_TOKEN16, p)
}

unsafe extern "system" fn result_size(_t: This, block: *mut c_void, size: *mut usize) -> i32 {
    xasync::size(block, size)
}

unsafe extern "system" fn token_result(_t: This, block: *mut c_void, size: usize, buf: *mut u8, ptr: *mut *mut u8, used: *mut usize) -> i32 {
    let hr = xasync::bytes(block, OP_TOKEN, size, buf as *mut c_void, used);
    if hr >= 0 && !ptr.is_null() {
        *ptr = buf;
    }
    hr
}

unsafe extern "system" fn token16_result(_t: This, block: *mut c_void, size: usize, buf: *mut u8, ptr: *mut *mut u8, used: *mut usize) -> i32 {
    let hr = xasync::bytes(block, OP_TOKEN16, size, buf as *mut c_void, used);
    if hr >= 0 && !ptr.is_null() {
        *ptr = buf;
    }
    hr
}

unsafe extern "system" fn resolve_issue_async(_t: This, _u: usize, url: *const u8, block: *mut c_void) -> i32 {
    log("XUserResolveIssueWithUiAsync", &format!("{} -> completed without UI", super::cstr(url).unwrap_or_default()));
    xasync::run(block, OP_RESOLVE_ISSUE, Payload::status(S_OK))
}

pub const STATE_SIGNED_IN: u32 = 0;
pub const STATE_SIGNING_OUT: u32 = 1;
pub const STATE_SIGNED_OUT: u32 = 2;
pub const EVENT_SIGNED_IN_AGAIN: u32 = 0;
pub const EVENT_SIGNING_OUT: u32 = 1;
pub const EVENT_SIGNED_OUT: u32 = 2;
pub const EVENT_GAMERTAG: u32 = 3;
pub const EVENT_GAMER_PICTURE: u32 = 4;
pub const EVENT_PRIVILEGES: u32 = 5;

static STATE: AtomicU32 = AtomicU32::new(STATE_SIGNED_IN);
pub static CHANGE_EVENTS: Registry = Registry::new();
pub static DEVICE_ASSOCIATION: Registry = Registry::new();
pub static AUDIO_ENDPOINT: Registry = Registry::new();

pub fn state() -> u32 {
    STATE.load(Ordering::Acquire)
}

pub fn fire_change(event: u32) -> usize {
    let n = CHANGE_EVENTS.len();
    CHANGE_EVENTS.fire(None, move |r| unsafe {
        let cb: unsafe extern "system" fn(*mut c_void, u64, u32) = core::mem::transmute(r.cb);
        cb(r.ctx as *mut c_void, LOCAL_ID, event)
    });
    log("XUserChangeEvent", &format!("event {event} queued to {n} registration(s)"));
    n
}

pub fn sign_out() -> bool {
    if STATE.compare_exchange(STATE_SIGNED_IN, STATE_SIGNING_OUT, Ordering::AcqRel, Ordering::Acquire).is_err() {
        return false;
    }
    fire_change(EVENT_SIGNING_OUT);
    STATE.store(STATE_SIGNED_OUT, Ordering::Release);
    fire_change(EVENT_SIGNED_OUT);
    true
}

pub fn sign_in() -> bool {
    if STATE.compare_exchange(STATE_SIGNED_OUT, STATE_SIGNED_IN, Ordering::AcqRel, Ordering::Acquire).is_err() {
        return false;
    }
    fire_change(EVENT_SIGNED_IN_AGAIN);
    true
}

pub fn parse_user_command(line: &str) -> Option<bool> {
    match line.trim().to_ascii_lowercase().as_str() {
        "signout" | "sign-out" => Some(false),
        "signin" | "sign-in" => Some(true),
        _ => None,
    }
}

pub fn fire_device_association(device: [u8; 32], old_user: u64, new_user: u64) -> usize {
    let n = DEVICE_ASSOCIATION.len();
    DEVICE_ASSOCIATION.fire(None, move |r| unsafe {
        #[repr(C)]
        struct Change {
            device: [u8; 32],
            old_user: u64,
            new_user: u64,
        }
        let c = Change { device, old_user, new_user };
        let cb: unsafe extern "system" fn(*mut c_void, *const Change) = core::mem::transmute(r.cb);
        cb(r.ctx as *mut c_void, &c)
    });
    n
}

pub fn fire_audio_endpoint(kind: u32, old_id: &str, new_id: &str) -> usize {
    let n = AUDIO_ENDPOINT.len();
    let (old_w, new_w): (Vec<u16>, Vec<u16>) = (old_id.encode_utf16().chain([0]).collect(), new_id.encode_utf16().chain([0]).collect());
    AUDIO_ENDPOINT.fire(None, move |r| unsafe {
        let cb: unsafe extern "system" fn(*mut c_void, u64, u32, *const u16, *const u16) = core::mem::transmute(r.cb);
        cb(r.ctx as *mut c_void, LOCAL_ID, kind, old_w.as_ptr(), new_w.as_ptr())
    });
    n
}

pub unsafe extern "system" fn register_event(_t: This, queue: usize, ctx: *mut c_void, cb: *const c_void, token: *mut u64) -> i32 {
    CHANGE_EVENTS.register(0, queue, ctx, cb as usize, token)
}

pub unsafe extern "system" fn unregister_event(_t: This, token: u64, wait: bool) -> bool {
    CHANGE_EVENTS.unregister(None, token, wait)
}

unsafe extern "system" fn register_device_association(_t: This, queue: usize, ctx: *mut c_void, cb: *const c_void, token: *mut u64) -> i32 {
    DEVICE_ASSOCIATION.register(0, queue, ctx, cb as usize, token)
}

unsafe extern "system" fn unregister_device_association(_t: This, token: u64, wait: bool) -> bool {
    DEVICE_ASSOCIATION.unregister(None, token, wait)
}

unsafe extern "system" fn register_audio_endpoint(_t: This, queue: usize, ctx: *mut c_void, cb: *const c_void, token: *mut u64) -> i32 {
    AUDIO_ENDPOINT.register(0, queue, ctx, cb as usize, token)
}

unsafe extern "system" fn unregister_audio_endpoint(_t: This, token: u64, wait: bool) -> bool {
    AUDIO_ENDPOINT.unregister(None, token, wait)
}

unsafe extern "system" fn gamer_picture_async(_t: This, u: usize, size: u32, block: *mut c_void) -> i32 {
    if u == 0 {
        return E_INVALIDARG;
    }
    let px = match size { 0 => 64, 1 => 208, 2 => 424, _ => 1080 };
    xasync::run(block, OP_PICTURE, Payload::data(png_solid(px, px, [0x10, 0x7c, 0x10])))
}

unsafe extern "system" fn gamer_picture_result(_t: This, block: *mut c_void, size: usize, buf: *mut u8, used: *mut usize) -> i32 {
    xasync::bytes(block, OP_PICTURE, size, buf as *mut c_void, used)
}

unsafe extern "system" fn get_gamertag(_t: This, u: usize, component: u32, size: usize, buf: *mut u8, used: *mut usize) -> i32 {
    if u == 0 {
        return E_INVALIDARG;
    }
    let s: &str = if component == 2 { "" } else { &user().gamertag };
    put_str(s, size, buf, used)
}

unsafe extern "system" fn find_for_device(_t: This, _device: *const u8, out: *mut usize) -> i32 {
    if out.is_null() {
        return E_INVALIDARG;
    }
    *out = handle();
    S_OK
}

unsafe extern "system" fn default_audio_endpoint(_t: This, _local: u64, _kind: u32, count: usize, buf: *mut u16, used: *mut usize) -> i32 {
    put_wstr("", count, buf, used)
}

fn crc32(data: &[u8]) -> u32 {
    let mut c = 0xffff_ffffu32;
    for &b in data {
        c ^= b as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xedb8_8320 ^ (c >> 1) } else { c >> 1 };
        }
    }
    !c
}

fn png_chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let mut c = kind.to_vec();
    c.extend_from_slice(data);
    out.extend_from_slice(&c);
    out.extend_from_slice(&crc32(&c).to_be_bytes());
}

pub fn png_solid(w: u32, h: u32, rgb: [u8; 3]) -> Vec<u8> {
    let px: Vec<u8> = (0..w * h).flat_map(|_| rgb).collect();
    png_rgb(w, h, &px)
}

pub fn png_rgb(w: u32, h: u32, px: &[u8]) -> Vec<u8> {
    let mut raw = Vec::with_capacity(((w * 3 + 1) * h) as usize);
    for row in px.chunks((w * 3) as usize).take(h as usize) {
        raw.push(0);
        raw.extend_from_slice(row);
    }
    let mut z = vec![0x78, 0x01];
    let chunks: Vec<&[u8]> = raw.chunks(65535).collect();
    for (i, c) in chunks.iter().enumerate() {
        z.push(if i + 1 == chunks.len() { 1 } else { 0 });
        z.extend_from_slice(&(c.len() as u16).to_le_bytes());
        z.extend_from_slice(&(!(c.len() as u16)).to_le_bytes());
        z.extend_from_slice(c);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for &x in &raw {
        a = (a + x as u32) % 65521;
        b = (b + a) % 65521;
    }
    z.extend_from_slice(&((b << 16) | a).to_be_bytes());
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&w.to_be_bytes());
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    png_chunk(&mut out, b"IHDR", &ihdr);
    png_chunk(&mut out, b"IDAT", &z);
    png_chunk(&mut out, b"IEND", &[]);
    out
}

unsafe extern "system" fn get_is_guest(_t: This, u: usize, out: *mut bool) -> i32 {
    if u == 0 || out.is_null() {
        return E_INVALIDARG;
    }
    *out = false;
    S_OK
}

unsafe extern "system" fn get_age_group(_t: This, u: usize, out: *mut u32) -> i32 {
    if u == 0 || out.is_null() {
        return E_INVALIDARG;
    }
    *out = 3;
    S_OK
}

unsafe extern "system" fn resolve_issue16_async(_t: This, _u: usize, url: *const u16, block: *mut c_void) -> i32 {
    log("XUserResolveIssueWithUiUtf16Async", &format!("{} -> completed without UI", super::wstr(url).unwrap_or_default()));
    xasync::run(block, OP_RESOLVE_ISSUE, Payload::status(S_OK))
}

unsafe extern "system" fn get_sign_out_deferral(_t: This, out: *mut usize) -> i32 {
    if !out.is_null() {
        *out = 0;
    }
    E_GAMEUSER_DEFERRAL_NOT_AVAILABLE
}

unsafe extern "system" fn add_by_id_async(_t: This, xuid: u64, block: *mut c_void) -> i32 {
    log("XUserAddByIdWithUiAsync", &format!("xuid={xuid} -> local user"));
    xasync::run(block, OP_ADD, Payload::value(handle()))
}

static OP_FIND_CONTROLLER: &[u8] = b"XUserFindControllerForUserWithUiAsync\0";

unsafe extern "system" fn find_controller_async(_t: This, _u: usize, block: *mut c_void) -> i32 {
    xasync::run(block, OP_FIND_CONTROLLER, Payload::value([0u8; 32]))
}

unsafe extern "system" fn find_controller_result(_t: This, block: *mut c_void, out: *mut [u8; 32]) -> i32 {
    xasync::value(block, OP_FIND_CONTROLLER, out)
}

pub fn user_layout() -> Vec<(usize, usize)> {
    vec![
        (3, duplicate_handle as usize),
        (4, close_handle as usize),
        (5, compare as usize),
        (6, get_max_users as usize),
        (7, add_async as usize),
        (8, add_result as usize),
        (9, get_local_id as usize),
        (10, find_by_local_id as usize),
        (11, get_id as usize),
        (12, find_by_id as usize),
        (13, get_is_guest as usize),
        (14, get_state as usize),
        (16, gamer_picture_async as usize),
        (17, result_size as usize),
        (18, gamer_picture_result as usize),
        (19, get_age_group as usize),
        (20, check_privilege as usize),
        (21, resolve_privilege_async as usize),
        (22, async_status as usize),
        (23, token_async as usize),
        (24, result_size as usize),
        (25, token_result as usize),
        (26, token16_async as usize),
        (27, result_size as usize),
        (28, token16_result as usize),
        (29, resolve_issue_async as usize),
        (30, async_status as usize),
        (31, resolve_issue16_async as usize),
        (32, async_status as usize),
        (33, register_event as usize),
        (34, unregister_event as usize),
        (35, get_sign_out_deferral as usize),
        (36, close_handle as usize),
        (37, add_by_id_async as usize),
        (38, add_result as usize),
    ]
}

pub fn device_layout() -> Vec<(usize, usize)> {
    vec![
        (3, find_for_device as usize),
        (4, register_device_association as usize),
        (5, unregister_device_association as usize),
        (6, default_audio_endpoint as usize),
        (7, register_audio_endpoint as usize),
        (8, unregister_audio_endpoint as usize),
        (9, find_controller_async as usize),
        (10, find_controller_result as usize),
    ]
}

pub fn pick(layout: &[(usize, usize)], confirmed: &[usize]) -> Vec<(usize, usize)> {
    layout.iter().copied().filter(|(s, _)| confirmed.contains(s)).collect()
}

pub fn interfaces() -> Vec<Iface> {
    let u = user_layout();
    let d = device_layout();
    vec![
        Iface { clsid: CLSID, iid: CLSID, name: "XUser(01acd177)", slots: pick(&u, &[3, 4, 5, 7, 8, 11, 29, 30, 34]) },
        Iface {
            clsid: CLSID,
            iid: IID_V1,
            name: "XUser(eb9bf948)",
            slots: pick(&u, &[3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 33, 34]),
        },
        Iface {
            clsid: CLSID,
            iid: IID_V2,
            name: "XUser(1bf2f8c5)",
            slots: pick(&u, &[3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38]),
        },
        Iface { clsid: CLSID, iid: IID_GAMERTAG, name: "XUserGamertag(cef4fac0)", slots: vec![(3, get_gamertag as usize)] },
        Iface { clsid: DEVICE, iid: DEVICE, name: "XUserDevice(7d824997)", slots: pick(&d, &[3, 4, 5, 6, 7, 8]) },
        Iface { clsid: DEVICE, iid: DEVICE_V2, name: "XUserDevice(0cc6a956)", slots: pick(&d, &[3, 4, 5, 6, 7, 8, 9, 10]) },
    ]
}
