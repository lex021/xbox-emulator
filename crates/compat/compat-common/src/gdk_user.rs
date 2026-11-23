
use core::ffi::c_void;
use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::OnceLock;

extern "system" {
    fn GetModuleHandleW(n: *const u16) -> *mut u8;
    fn VirtualProtect(addr: *mut c_void, size: usize, new_protect: u32, old_protect: *mut u32) -> i32;
    fn FlushInstructionCache(process: *mut c_void, addr: *const c_void, size: usize) -> i32;
    fn GetCurrentProcess() -> *mut c_void;
}

const S_OK: i32 = 0;
pub(crate) const E_PENDING: i32 = 0x8000_000Au32 as i32;
const E_INVALIDARG: i32 = 0x8007_0057u32 as i32;
const E_NOT_SUFFICIENT_BUFFER: i32 = 0x8007_007Au32 as i32;
const E_NOT_FOUND: i32 = 0x8007_0490u32 as i32;

pub const XASYNC_NAMES: [&str; 6] = ["XAsyncBegin", "XAsyncComplete", "XAsyncGetResult", "XAsyncGetResultSize", "XAsyncGetStatus", "XAsyncSchedule"];
static XASYNC: [AtomicUsize; 6] = [const { AtomicUsize::new(0) }; 6];

static EXE: AtomicUsize = AtomicUsize::new(0);

struct User {
    gamertag: String,
    xuid: u64,
    token: Vec<u8>,
    token16: Vec<u16>,
}
static USER: OnceLock<User> = OnceLock::new();
fn user() -> &'static User {
    USER.get().expect("gdk_user installed")
}

fn handle() -> usize {
    crate::gdk::user::handle()
}
const LOCAL_ID: u64 = 1;

pub(crate) fn log(func: &str, detail: &str) {
    static N: AtomicUsize = AtomicUsize::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    if n < 200 || n % 1000 == 0 {
        crate::record_raw("gdk_user", func, detail);
    }
}


#[repr(C)]
pub(crate) struct ProviderData {
    pub(crate) async_block: *mut c_void,
    buffer_size: usize,
    buffer: *mut u8,
    context: *mut c_void,
}
type Provider = unsafe extern "system" fn(op: u32, data: *const ProviderData) -> i32;

unsafe fn xasync_begin(block: *mut c_void, context: *mut c_void, identity: usize, name: &'static [u8], provider: Provider) -> i32 {
    let f: unsafe extern "system" fn(*mut c_void, *mut c_void, usize, *const u8, Provider) -> i32 =
        core::mem::transmute(XASYNC[0].load(Ordering::Relaxed));
    f(block, context, identity, name.as_ptr(), provider)
}
unsafe fn xasync_schedule(block: *mut c_void) -> i32 {
    let f: unsafe extern "system" fn(*mut c_void, u32) -> i32 = core::mem::transmute(XASYNC[5].load(Ordering::Relaxed));
    f(block, 0)
}
pub(crate) unsafe fn xasync_complete(block: *mut c_void, hr: i32, size: usize) {
    let f: unsafe extern "system" fn(*mut c_void, i32, usize) = core::mem::transmute(XASYNC[1].load(Ordering::Relaxed));
    f(block, hr, size)
}
unsafe fn xasync_get_result(block: *mut c_void, identity: usize, size: usize, buf: *mut u8, used: *mut usize) -> i32 {
    let f: unsafe extern "system" fn(*mut c_void, usize, usize, *mut u8, *mut usize) -> i32 =
        core::mem::transmute(XASYNC[2].load(Ordering::Relaxed));
    f(block, identity, size, buf, used)
}
unsafe fn xasync_get_result_size(block: *mut c_void, size: *mut usize) -> i32 {
    let f: unsafe extern "system" fn(*mut c_void, *mut usize) -> i32 = core::mem::transmute(XASYNC[3].load(Ordering::Relaxed));
    f(block, size)
}
pub(crate) unsafe fn xasync_get_status(block: *mut c_void) -> i32 {
    let f: unsafe extern "system" fn(*mut c_void, bool) -> i32 = core::mem::transmute(XASYNC[4].load(Ordering::Relaxed));
    f(block, false)
}

pub(crate) unsafe fn start_async(block: *mut c_void, identity: usize, name: &'static [u8], provider: Provider) -> i32 {
    if block.is_null() {
        return E_INVALIDARG;
    }
    let hr = xasync_begin(block, core::ptr::null_mut(), identity, name, provider);
    if hr < 0 {
        return hr;
    }
    xasync_schedule(block)
}


unsafe extern "system" fn add_provider(op: u32, data: *const ProviderData) -> i32 {
    match op {
        1 => {
            xasync_complete((*data).async_block, S_OK, core::mem::size_of::<usize>());
            E_PENDING
        }
        2 => {
            if (*data).buffer_size < core::mem::size_of::<usize>() {
                return E_NOT_SUFFICIENT_BUFFER;
            }
            core::ptr::write_unaligned((*data).buffer as *mut usize, handle());
            S_OK
        }
        _ => S_OK,
    }
}

unsafe extern "system" fn no_result_provider(op: u32, data: *const ProviderData) -> i32 {
    if op == 1 {
        xasync_complete((*data).async_block, S_OK, 0);
        return E_PENDING;
    }
    S_OK
}

fn token_blob_size(wide: bool) -> usize {
    let u = user();
    32 + if wide { (u.token16.len() + 1) * 2 } else { u.token.len() + 1 }
}

unsafe fn write_token_blob(buf: *mut u8, wide: bool) {
    let u = user();
    let tok = buf.add(32);
    let (tok_bytes, count) = if wide {
        (u.token16.len() * 2, u.token16.len())
    } else {
        (u.token.len(), u.token.len())
    };
    if wide {
        core::ptr::copy_nonoverlapping(u.token16.as_ptr() as *const u8, tok, tok_bytes);
    } else {
        core::ptr::copy_nonoverlapping(u.token.as_ptr(), tok, tok_bytes);
    }
    let sig = tok.add(tok_bytes);
    core::ptr::write_bytes(sig, 0, if wide { 2 } else { 1 });
    core::ptr::write_unaligned(buf as *mut usize, count);
    core::ptr::write_unaligned(buf.add(8) as *mut usize, 1);
    core::ptr::write_unaligned(buf.add(16) as *mut usize, tok as usize);
    core::ptr::write_unaligned(buf.add(24) as *mut usize, sig as usize);
}

unsafe extern "system" fn token_provider(op: u32, data: *const ProviderData) -> i32 {
    token_provider_impl(op, data, false)
}
unsafe extern "system" fn token16_provider(op: u32, data: *const ProviderData) -> i32 {
    token_provider_impl(op, data, true)
}
unsafe fn token_provider_impl(op: u32, data: *const ProviderData, wide: bool) -> i32 {
    match op {
        1 => {
            xasync_complete((*data).async_block, S_OK, token_blob_size(wide));
            E_PENDING
        }
        2 => {
            if (*data).buffer_size < token_blob_size(wide) {
                return E_NOT_SUFFICIENT_BUFFER;
            }
            write_token_blob((*data).buffer, wide);
            S_OK
        }
        _ => S_OK,
    }
}


unsafe extern "system" fn x_add_async(options: u32, block: *mut c_void) -> i32 {
    let hr = start_async(block, x_add_async as usize, b"XUserAddAsync\0", add_provider);
    log("XUserAddAsync", &format!("options={options:#x} -> hr={hr:#010x} (local user '{}')", user().gamertag));
    hr
}

unsafe extern "system" fn x_add_result(block: *mut c_void, out: *mut usize) -> i32 {
    if out.is_null() {
        return E_INVALIDARG;
    }
    let hr = xasync_get_result(block, x_add_async as usize, core::mem::size_of::<usize>(), out as *mut u8, core::ptr::null_mut());
    log("XUserAddResult", &format!("-> hr={hr:#010x} user={:#x}", if hr >= 0 { *out } else { 0 }));
    hr
}

unsafe extern "system" fn x_check_privilege(u: usize, options: u32, privilege: u32, has: *mut bool, reason: *mut u32) -> i32 {
    if u == 0 || has.is_null() {
        return E_INVALIDARG;
    }
    *has = true;
    if !reason.is_null() {
        *reason = 0;
    }
    log("XUserCheckPrivilege", &format!("privilege={privilege} options={options:#x} -> granted (local user policy)"));
    S_OK
}

unsafe extern "system" fn x_close_handle(_u: usize) {}

unsafe extern "system" fn x_compare(a: usize, b: usize) -> i32 {
    if a == b { 0 } else if a < b { -1 } else { 1 }
}

unsafe extern "system" fn x_duplicate_handle(u: usize, out: *mut usize) -> i32 {
    if u == 0 || out.is_null() {
        return E_INVALIDARG;
    }
    *out = u;
    S_OK
}

unsafe extern "system" fn x_find_for_device(_device: *const u8, out: *mut usize) -> i32 {
    if out.is_null() {
        return E_INVALIDARG;
    }
    *out = handle();
    S_OK
}

unsafe extern "system" fn x_find_by_id(xuid: u64, out: *mut usize) -> i32 {
    if out.is_null() {
        return E_INVALIDARG;
    }
    if xuid != user().xuid {
        log("XUserFindUserById", &format!("xuid={xuid} -> not found"));
        return E_NOT_FOUND;
    }
    *out = handle();
    S_OK
}

unsafe extern "system" fn x_find_by_local_id(local: u64, out: *mut usize) -> i32 {
    if out.is_null() {
        return E_INVALIDARG;
    }
    if local != LOCAL_ID {
        log("XUserFindUserByLocalId", &format!("localId={local} -> not found"));
        return E_NOT_FOUND;
    }
    *out = handle();
    S_OK
}

unsafe extern "system" fn x_get_gamertag(u: usize, component: u32, size: usize, buf: *mut u8, used: *mut usize) -> i32 {
    if u == 0 {
        return E_INVALIDARG;
    }
    let s: &str = if component == 2 { "" } else { &user().gamertag };
    let need = s.len() + 1;
    if !used.is_null() {
        *used = need;
    }
    if buf.is_null() || size < need {
        return E_NOT_SUFFICIENT_BUFFER;
    }
    core::ptr::copy_nonoverlapping(s.as_ptr(), buf, s.len());
    *buf.add(s.len()) = 0;
    S_OK
}

unsafe extern "system" fn x_get_id(u: usize, out: *mut u64) -> i32 {
    if u == 0 || out.is_null() {
        return E_INVALIDARG;
    }
    *out = user().xuid;
    S_OK
}

unsafe extern "system" fn x_get_local_id(u: usize, out: *mut u64) -> i32 {
    if u == 0 || out.is_null() {
        return E_INVALIDARG;
    }
    *out = LOCAL_ID;
    S_OK
}

unsafe extern "system" fn x_get_max_users(out: *mut u32) -> i32 {
    if out.is_null() {
        return E_INVALIDARG;
    }
    *out = 1;
    S_OK
}

unsafe extern "system" fn x_get_state(u: usize, out: *mut u32) -> i32 {
    if u == 0 || out.is_null() {
        return E_INVALIDARG;
    }
    *out = 0;
    S_OK
}

unsafe fn describe_request(method: *const u8, url: *const u8, wide: bool) -> String {
    let rd = |p: *const u8| -> String {
        if p.is_null() {
            return "null".into();
        }
        if wide {
            let p = p as *const u16;
            let mut n = 0;
            while n < 512 && *p.add(n) != 0 { n += 1; }
            String::from_utf16_lossy(core::slice::from_raw_parts(p, n))
        } else {
            let mut n = 0;
            while n < 512 && *p.add(n) != 0 { n += 1; }
            String::from_utf8_lossy(core::slice::from_raw_parts(p, n)).into_owned()
        }
    };
    format!("{} {}", rd(method), rd(url))
}

#[allow(clippy::too_many_arguments)]
unsafe extern "system" fn x_token_async(
    u: usize, options: u32, method: *const u8, url: *const u8, _hc: usize, _headers: *const c_void, _body_size: usize, _body: *const c_void, block: *mut c_void,
) -> i32 {
    if u == 0 {
        return E_INVALIDARG;
    }
    let hr = start_async(block, x_token_async as usize, b"XUserGetTokenAndSignatureAsync\0", token_provider);
    log("XUserGetTokenAndSignatureAsync", &format!("options={options:#x} for '{}' -> hr={hr:#010x} (placeholder token from config)", describe_request(method, url, false)));
    hr
}
unsafe extern "system" fn x_token_result(block: *mut c_void, size: usize, buf: *mut u8, ptr: *mut *mut u8, used: *mut usize) -> i32 {
    let hr = xasync_get_result(block, x_token_async as usize, size, buf, used);
    if hr >= 0 && !ptr.is_null() {
        *ptr = buf;
    }
    hr
}
#[allow(clippy::too_many_arguments)]
unsafe extern "system" fn x_token16_async(
    u: usize, options: u32, method: *const u8, url: *const u8, _hc: usize, _headers: *const c_void, _body_size: usize, _body: *const c_void, block: *mut c_void,
) -> i32 {
    if u == 0 {
        return E_INVALIDARG;
    }
    let hr = start_async(block, x_token16_async as usize, b"XUserGetTokenAndSignatureUtf16Async\0", token16_provider);
    log("XUserGetTokenAndSignatureUtf16Async", &format!("options={options:#x} for '{}' -> hr={hr:#010x} (placeholder token from config)", describe_request(method, url, true)));
    hr
}
unsafe extern "system" fn x_token16_result(block: *mut c_void, size: usize, buf: *mut u8, ptr: *mut *mut u8, used: *mut usize) -> i32 {
    let hr = xasync_get_result(block, x_token16_async as usize, size, buf, used);
    if hr >= 0 && !ptr.is_null() {
        *ptr = buf;
    }
    hr
}
unsafe extern "system" fn x_token_result_size(block: *mut c_void, size: *mut usize) -> i32 {
    xasync_get_result_size(block, size)
}

static NEXT_TOKEN: AtomicU64 = AtomicU64::new(1);
unsafe extern "system" fn x_register(_queue: usize, _ctx: *mut c_void, _cb: *const c_void, token: *mut u64) -> i32 {
    if !token.is_null() {
        *token = NEXT_TOKEN.fetch_add(1, Ordering::Relaxed);
    }
    log("XUserRegisterFor*", "registration accepted (no events are ever delivered)");
    S_OK
}
unsafe extern "system" fn x_unregister(_token: u64, _wait: bool) -> bool {
    true
}

unsafe extern "system" fn x_resolve_issue_async(_u: usize, _url: *const u8, block: *mut c_void) -> i32 {
    log("XUserResolveIssueWithUiAsync", "completed without UI");
    start_async(block, x_resolve_issue_async as usize, b"XUserResolveIssueWithUiAsync\0", no_result_provider)
}
unsafe extern "system" fn x_resolve_privilege_async(_u: usize, _options: u32, privilege: u32, block: *mut c_void) -> i32 {
    log("XUserResolvePrivilegeWithUiAsync", &format!("privilege={privilege} completed without UI"));
    start_async(block, x_resolve_privilege_async as usize, b"XUserResolvePrivilegeWithUiAsync\0", no_result_provider)
}
unsafe extern "system" fn x_resolve_result(block: *mut c_void) -> i32 {
    xasync_get_status(block)
}


pub(crate) unsafe fn write_jump(p: *mut u8, target: usize) -> bool {
    let mut code = [0u8; 12];
    code[0] = 0x48;
    code[1] = 0xB8;
    code[2..10].copy_from_slice(&(target as u64).to_le_bytes());
    code[10] = 0xFF;
    code[11] = 0xE0;
    let mut old = 0u32;
    if VirtualProtect(p as *mut c_void, 12, 0x40, &mut old) == 0 {
        return false;
    }
    core::ptr::copy_nonoverlapping(code.as_ptr(), p, 12);
    let mut tmp = 0u32;
    VirtualProtect(p as *mut c_void, 12, old, &mut tmp);
    FlushInstructionCache(GetCurrentProcess(), p as *const c_void, 12);
    true
}

pub fn install() {
    crate::gdk::install();
    unsafe {
        let base = GetModuleHandleW(core::ptr::null());
        if base.is_null() {
            return;
        }
        let o = match layout(base) {
            Ok(o) => o,
            Err(e) => {
                crate::record_raw("gdk_user", "install", &format!("{e} - local XUser layer NOT installed"));
                return;
            }
        };
        if crate::cfg("local_user").as_deref() == Some("0") {
            crate::record_raw("gdk_user", "install", "local_user=0 in tombox.cfg - local XUser layer NOT installed");
            return;
        }
        let gamertag = crate::cfg("gamertag").filter(|g| !g.is_empty() && g.len() <= 32 && g.is_ascii()).unwrap_or_else(|| "XboxEmulator".into());
        let xuid = crate::cfg("xuid").and_then(|x| x.parse::<u64>().ok()).unwrap_or(2533274790395905);
        let token = crate::cfg("xbl_token").filter(|t| !t.is_empty()).unwrap_or_else(|| format!("XBL3.0 x=tombox;local-offline-user-{xuid}"));
        let mut t8 = token.clone().into_bytes();
        t8.push(0);
        let t16: Vec<u16> = token.encode_utf16().chain(Some(0)).collect();
        let _ = USER.set(User { gamertag: gamertag.clone(), xuid, token: t8, token16: t16 });
        EXE.store(base as usize, Ordering::SeqCst);
        for (slot, rva) in XASYNC.iter().zip(&o.xasync) {
            slot.store(base as usize + rva, Ordering::SeqCst);
        }

        let table: [(&str, usize); 28] = [
            ("XUserAddAsync", x_add_async as usize),
            ("XUserAddResult", x_add_result as usize),
            ("XUserCheckPrivilege", x_check_privilege as usize),
            ("XUserCloseHandle", x_close_handle as usize),
            ("XUserCompare", x_compare as usize),
            ("XUserDuplicateHandle", x_duplicate_handle as usize),
            ("XUserFindForDevice", x_find_for_device as usize),
            ("XUserFindUserById", x_find_by_id as usize),
            ("XUserFindUserByLocalId", x_find_by_local_id as usize),
            ("XUserGetGamertag", x_get_gamertag as usize),
            ("XUserGetId", x_get_id as usize),
            ("XUserGetLocalId", x_get_local_id as usize),
            ("XUserGetMaxUsers", x_get_max_users as usize),
            ("XUserGetState", x_get_state as usize),
            ("XUserGetTokenAndSignatureAsync", x_token_async as usize),
            ("XUserGetTokenAndSignatureResult", x_token_result as usize),
            ("XUserGetTokenAndSignatureResultSize", x_token_result_size as usize),
            ("XUserGetTokenAndSignatureUtf16Async", x_token16_async as usize),
            ("XUserGetTokenAndSignatureUtf16Result", x_token16_result as usize),
            ("XUserGetTokenAndSignatureUtf16ResultSize", x_token_result_size as usize),
            ("XUserRegisterForChangeEvent", x_register as usize),
            ("XUserRegisterForDeviceAssociationChanged", x_register as usize),
            ("XUserResolveIssueWithUiAsync", x_resolve_issue_async as usize),
            ("XUserResolveIssueWithUiResult", x_resolve_result as usize),
            ("XUserResolvePrivilegeWithUiAsync", x_resolve_privilege_async as usize),
            ("XUserResolvePrivilegeWithUiResult", x_resolve_result as usize),
            ("XUserUnregisterForChangeEvent", x_unregister as usize),
            ("XUserUnregisterForDeviceAssociationChanged", x_unregister as usize),
        ];
        let mut ok = 0;
        let mut failed = Vec::new();
        for ((name, target), rva) in table.iter().zip(&o.xuser) {
            if write_jump(base.add(*rva), *target) { ok += 1 } else { failed.push(*name) }
        }
        crate::gdk_store::install(base, &o.xstore);
        crate::record_raw("gdk_user", "install", &format!(
            "local offline user installed from {}: gamertag='{gamertag}' xuid={xuid} ({ok}/28 XUser wrappers redirected{})", o.source,
            if failed.is_empty() { String::new() } else { format!("; FAILED: {}", failed.join(", ")) }));
    }
}

pub const XUSER_NAMES: [&str; 28] = [
    "XUserAddAsync", "XUserAddResult", "XUserCheckPrivilege", "XUserCloseHandle", "XUserCompare", "XUserDuplicateHandle", "XUserFindForDevice",
    "XUserFindUserById", "XUserFindUserByLocalId", "XUserGetGamertag", "XUserGetId", "XUserGetLocalId", "XUserGetMaxUsers", "XUserGetState",
    "XUserGetTokenAndSignatureAsync", "XUserGetTokenAndSignatureResult", "XUserGetTokenAndSignatureResultSize", "XUserGetTokenAndSignatureUtf16Async",
    "XUserGetTokenAndSignatureUtf16Result", "XUserGetTokenAndSignatureUtf16ResultSize", "XUserRegisterForChangeEvent", "XUserRegisterForDeviceAssociationChanged",
    "XUserResolveIssueWithUiAsync", "XUserResolveIssueWithUiResult", "XUserResolvePrivilegeWithUiAsync", "XUserResolvePrivilegeWithUiResult",
    "XUserUnregisterForChangeEvent", "XUserUnregisterForDeviceAssociationChanged",
];

#[derive(Debug, PartialEq)]
pub struct Layout {
    pub source: String,
    pub xasync: Vec<usize>,
    pub xuser: Vec<usize>,
    pub xstore: Vec<usize>,
}

pub fn layout_from(o: &crate::title_offsets::Offsets, stamp: u32) -> Result<Layout, String> {
    if o.pe_timestamp.is_none() {
        return Err(format!("{} has no pe_timestamp", o.source));
    }
    if !o.matches_image(stamp) {
        return Err(format!("host EXE timestamp {stamp:#x} does not match {}", o.source));
    }
    Ok(Layout { source: o.source.clone(), xasync: o.require(&XASYNC_NAMES)?, xuser: o.require(&XUSER_NAMES)?, xstore: o.require(&crate::gdk_store::XSTORE_NAMES)? })
}

pub const JUMP_BYTES: usize = 12;

pub fn layout_from_symbols(t: &crate::title_symbols::Table) -> Result<Layout, String> {
    let rvas = |names: &[&str], patched: bool| -> Result<Vec<usize>, String> {
        let syms = t.require(names)?;
        if patched {
            if let Some((n, s)) = names.iter().zip(&syms).find(|(_, s)| s.size < JUMP_BYTES) {
                return Err(format!("{n} is only {} bytes, too short for a {JUMP_BYTES}-byte jump", s.size));
            }
        }
        Ok(syms.iter().map(|s| s.rva).collect())
    };
    Ok(Layout { source: t.source.clone(), xasync: rvas(&XASYNC_NAMES, false)?, xuser: rvas(&XUSER_NAMES, true)?, xstore: rvas(&crate::gdk_store::XSTORE_NAMES, true)? })
}

unsafe fn layout(base: *mut u8) -> Result<Layout, String> {
    let se = match crate::title_symbols::for_this_title().and_then(layout_from_symbols) {
        Ok(l) => return Ok(l),
        Err(e) => e,
    };
    let o = crate::title_offsets::for_this_title().ok_or_else(|| format!("symbols: {se}; offsets file: {}", crate::title_offsets::status_for_this_title()))?;
    layout_from(o, crate::title_offsets::image_timestamp(base)).map_err(|je| format!("symbols: {se}; offsets file: {je}"))
}

#[cfg(test)]
mod layout_tests {
    use super::*;

    fn file(stamp: Option<&str>, skip: Option<&str>) -> String {
        let names = XASYNC_NAMES.iter().chain(XUSER_NAMES.iter()).chain(crate::gdk_store::XSTORE_NAMES.iter());
        let vals: Vec<String> = names.enumerate().filter(|(_, n)| Some(**n) != skip).map(|(i, n)| format!("\"{n}\":{}", 0x100 + i * 0x10)).collect();
        let ts = stamp.map(|t| format!(",\"pe_timestamp\":\"{t}\"")).unwrap_or_default();
        format!("{{\"exe\":\"T.exe\"{ts},\"values\":{{{}}}}}", vals.join(","))
    }

    #[test]
    fn every_patched_function_comes_from_the_offsets_file() {
        let o = crate::title_offsets::parse(&file(Some("0x1234"), None), "t").unwrap();
        let l = layout_from(&o, 0x1234).unwrap();
        assert_eq!((l.xasync.len(), l.xuser.len(), l.xstore.len()), (6, 28, 22));
        assert_eq!(l.xasync[0], 0x100);
        assert_eq!(l.xuser[0], 0x100 + 6 * 0x10);
        assert_eq!(l.xstore[21], 0x100 + 55 * 0x10);
    }

    #[test]
    fn the_title_symbols_supply_every_address_when_present() {
        let names: Vec<&str> = XASYNC_NAMES.iter().chain(XUSER_NAMES.iter()).chain(crate::gdk_store::XSTORE_NAMES.iter()).copied().collect();
        let syms: Vec<(&str, u32, u32)> = names.iter().enumerate().map(|(i, n)| (*n, 0x1000 + i as u32 * 0x80, 0x40)).collect();
        let f = crate::title_symbols::build_for_test("T", &syms);
        let t = crate::title_symbols::parse_bytes(&f, &names, "t").unwrap();
        let l = layout_from_symbols(&t).unwrap();
        assert_eq!((l.xasync[0], l.xuser[0], l.xstore[21]), (0x1000, 0x1000 + 6 * 0x80, 0x1000 + 55 * 0x80));
        let mut short = syms.clone();
        short[10].2 = 8;
        let t = crate::title_symbols::parse_bytes(&crate::title_symbols::build_for_test("T", &short), &names, "t").unwrap();
        assert!(layout_from_symbols(&t).unwrap_err().contains("too short"));
        let t = crate::title_symbols::parse_bytes(&crate::title_symbols::build_for_test("T", &syms[1..]), &names, "t").unwrap();
        assert!(layout_from_symbols(&t).unwrap_err().contains("XAsyncBegin"));
    }

    #[test]
    fn a_wrong_build_a_missing_stamp_or_a_missing_name_leaves_the_layer_off() {
        let o = crate::title_offsets::parse(&file(Some("0x1234"), None), "t").unwrap();
        assert!(layout_from(&o, 0x9999).unwrap_err().contains("does not match"));
        let o = crate::title_offsets::parse(&file(None, None), "t").unwrap();
        assert!(layout_from(&o, 0x1234).unwrap_err().contains("pe_timestamp"));
        let o = crate::title_offsets::parse(&file(Some("0x1234"), Some("XUserGetId")), "t").unwrap();
        assert!(layout_from(&o, 0x1234).unwrap_err().contains("XUserGetId"));
    }
}
